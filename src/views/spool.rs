//! A command view's output in a file (#573): what a command writes past
//! the spool threshold, or all it writes with `spool = true`.
//!
//! As stdin is (#517), the output is copied into a file as it comes, and
//! the app follows that file, so a command that writes for minutes, or
//! writes gigabytes, shows its lines as they come. One thread copies, and
//! one holds the command: it kills the command when it is quiet for its
//! `timeout`, or when the user stops it. When the command is over, that
//! thread reads the file again whole, as an index when it is large (#51).
//!
//! Dropping the spool kills the command and deletes the file.

use crate::lines::Lines;
use crate::stdin::{create_private, reason};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// How often the command is checked for its exit, its silence and a stop.
/// A timeout is in seconds, and a command can run for minutes.
const POLL: Duration = Duration::from_millis(50);

/// How the command stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum State {
    /// The command runs: more may come.
    #[default]
    Running,
    /// The command exited with success, and all it wrote is in the file.
    Ended,
    /// The command was stopped, or failed, for the reason given: the user
    /// stopped it, it was quiet for its timeout, a write to the spool
    /// failed, or it exited with an error. The lines it wrote stay.
    Stopped(String),
}

/// What the threads tell the app.
#[derive(Debug, Default)]
struct Shared {
    state: State,
    /// The file read again whole once the command is over, for the app to
    /// take once.
    lines: Option<Lines>,
    /// Why a write to the spool failed, for the waiting thread to give as
    /// the reason.
    failed: Option<String>,
}

/// When the command last wrote, in milliseconds from `started`: its
/// timeout counts from there. An atomic, so the copy does not lock for
/// each read of the pipe.
#[derive(Debug)]
struct LastOutput {
    started: Instant,
    millis: AtomicU64,
}

impl LastOutput {
    fn now() -> Self {
        Self {
            started: Instant::now(),
            millis: AtomicU64::new(0),
        }
    }

    fn touch(&self) {
        let millis = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.millis.store(millis, Ordering::Relaxed);
    }

    fn quiet_for(&self) -> Duration {
        let last = Duration::from_millis(self.millis.load(Ordering::Relaxed));
        self.started.elapsed().saturating_sub(last)
    }
}

/// Where a spool goes, and from what size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Spooling {
    /// Output past this many bytes goes to a spool. `MAX_OUTPUT` in the
    /// app; a test sets a few bytes.
    pub(crate) bytes: u64,
    /// `[spool] dir`: the directory the file is made in.
    pub(crate) dir: PathBuf,
}

impl Default for Spooling {
    /// Past `MAX_OUTPUT`, in the system's temporary directory.
    fn default() -> Self {
        Self {
            bytes: super::command::MAX_OUTPUT,
            dir: std::env::temp_dir(),
        }
    }
}

/// A command's output copied into a file, and the command that writes it.
#[derive(Debug)]
pub(crate) struct Spool {
    path: PathBuf,
    shared: Arc<Mutex<Shared>>,
    /// Set to kill the command: by `stop`, by a failed write, and on drop.
    stop: Arc<AtomicBool>,
    /// Set on drop: the file is gone, and is not read again.
    dropped: Arc<AtomicBool>,
}

impl Spool {
    /// Make the file in `dir`, write `head` to it, and copy the rest of
    /// `pipe` after it on a thread of its own, while another thread waits
    /// for `child`. The child is killed after `timeout` with no output;
    /// `stderr` gives the reason when it fails. The file is read again
    /// as an index from `index_bytes` (#51).
    ///
    /// # Errors
    /// The file cannot be made or written, or the OS refuses a thread.
    pub(crate) fn start(
        dir: &Path,
        index_bytes: u64,
        child: Child,
        pipe: impl Read + Send + 'static,
        head: &[u8],
        timeout: Duration,
        stderr: mpsc::Receiver<Vec<u8>>,
    ) -> io::Result<Self> {
        let path = dir.join(format!(
            "recon-spool-{}-{}.txt",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = create_private(&path)?;
        file.write_all(head)?;
        let shared = Arc::<Mutex<Shared>>::default();
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let last_output = Arc::new(LastOutput::now());
        let (copied, done) = mpsc::channel();
        let copying = Copying {
            shared: Arc::clone(&shared),
            stop: Arc::clone(&stop),
            last_output: Arc::clone(&last_output),
        };
        thread::Builder::new()
            .name("recon-spool-copy".to_string())
            .spawn(move || {
                copying.copy(pipe, file);
                let _ = copied.send(());
            })?;
        let holding = Holding {
            path: path.clone(),
            index_bytes,
            timeout,
            shared: Arc::clone(&shared),
            stop: Arc::clone(&stop),
            dropped: Arc::clone(&dropped),
            last_output,
        };
        thread::Builder::new()
            .name("recon-spool-wait".to_string())
            .spawn(move || holding.hold(child, &done, &stderr))?;
        log::debug!("spool: {}", path.display());
        Ok(Self {
            path,
            shared,
            stop,
            dropped,
        })
    }

    /// The file the output is copied into.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// How the command stands now.
    pub(crate) fn state(&self) -> State {
        lock(&self.shared).state.clone()
    }

    /// The file read again whole, once the command is over. `Some` once.
    pub(crate) fn take_lines(&self) -> Option<Lines> {
        lock(&self.shared).lines.take()
    }

    /// Kill the command. The lines it wrote stay.
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Relaxed);
        self.stop.store(true, Ordering::Relaxed);
        if let Err(err) = std::fs::remove_file(&self.path) {
            log::warn!("spool: cannot delete {}: {err}", self.path.display());
        }
    }
}

/// A panic on a spool thread leaves what it last wrote, which is as good as
/// any to show.
fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The number of the next spool, for `recon-spool-<pid>-<n>.txt`.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The copy thread's share of the spool.
struct Copying {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    last_output: Arc<LastOutput>,
}

impl Copying {
    /// Copy `pipe` into `file` until it ends or a write fails. Each read is
    /// written straight through, as stdin's copy is, so the tail sees a
    /// line as soon as the command writes it.
    fn copy(&self, mut pipe: impl Read, mut file: impl Write) {
        let mut buf = vec![0; 64 * 1024];
        loop {
            let read = match pipe.read(&mut buf) {
                Ok(0) => return,
                Ok(read) => read,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            };
            if let Err(err) = file.write_all(&buf[..read]) {
                lock(&self.shared).failed = Some(reason(&err));
                self.stop.store(true, Ordering::Relaxed);
                return;
            }
            self.last_output.touch();
        }
    }
}

/// The waiting thread's share of the spool.
struct Holding {
    path: PathBuf,
    index_bytes: u64,
    timeout: Duration,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    last_output: Arc<LastOutput>,
}

impl Holding {
    /// Wait for `child` to exit, killing it at a stop or after its timeout
    /// with no output. Then wait for the copy, read the file again whole,
    /// and say how it ended.
    fn hold(self, mut child: Child, copied: &mpsc::Receiver<()>, stderr: &mpsc::Receiver<Vec<u8>>) {
        let end = self.wait(&mut child);
        // A grandchild can hold the pipe open after the command exits: the
        // copy gets one more timeout, and no more.
        let _ = copied.recv_timeout(self.timeout);
        if self.dropped.load(Ordering::Relaxed) {
            return;
        }
        let failed = lock(&self.shared).failed.take();
        let state = match (failed, end) {
            (Some(reason), _) => State::Stopped(reason),
            (None, End::Exited(status)) if status.success() => State::Ended,
            (None, End::Exited(status)) => State::Stopped(
                super::command::first_line(&stderr.recv_timeout(POLL).unwrap_or_default())
                    .unwrap_or_else(|| status.to_string()),
            ),
            (None, End::Stopped) => State::Stopped("by the user".to_string()),
            (None, End::Quiet) => {
                State::Stopped(format!("no output for {}s", self.timeout.as_secs_f64()))
            }
        };
        log::debug!("spool: {}: {state:?}", self.path.display());
        let lines = crate::document::read_lines_with_threshold(&self.path, self.index_bytes)
            .inspect_err(|err| log::warn!("spool: cannot read {}: {err}", self.path.display()))
            .ok();
        let mut shared = lock(&self.shared);
        shared.state = state;
        shared.lines = lines;
    }

    /// How the command ended.
    fn wait(&self, child: &mut Child) -> End {
        loop {
            let end = match child.try_wait() {
                Ok(Some(status)) => return End::Exited(status),
                // The wait itself failed: the command cannot be watched.
                Err(_) => End::Stopped,
                Ok(None) if self.stop.load(Ordering::Relaxed) => End::Stopped,
                Ok(None) if self.last_output.quiet_for() >= self.timeout => End::Quiet,
                Ok(None) => {
                    thread::sleep(POLL);
                    continue;
                }
            };
            super::command::kill(child);
            return end;
        }
    }
}

/// How a spooled command ended.
enum End {
    Exited(ExitStatus),
    /// By `stop`, a failed write, or the drop.
    Stopped,
    /// After its timeout with no output.
    Quiet,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disk that is full: every write fails.
    struct Full;

    impl Write for Full {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::StorageFull))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A write that fails stops the spool: the command is killed, and the
    /// reason is the OS error. There is no limit on the spool's size other
    /// than the disk's.
    #[test]
    fn a_failed_write_stops_the_command_with_the_reason() {
        let copying = Copying {
            shared: Arc::default(),
            stop: Arc::default(),
            last_output: Arc::new(LastOutput::now()),
        };
        copying.copy(&b"more\n"[..], Full);
        assert!(copying.stop.load(Ordering::Relaxed), "the command runs on");
        assert_eq!(lock(&copying.shared).failed.as_deref(), Some("disk full"));
    }
}
