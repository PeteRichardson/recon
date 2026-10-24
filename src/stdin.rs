//! `-` in the TUI (#517): the lines on stdin, copied into a file.
//!
//! Everything in the TUI that shows a file reads it from a path — the view,
//! follow mode, `r`, hex, `o` and `O`. So rather than a second, path-free
//! kind of view, recon copies stdin into a file in `$TMPDIR` as it comes
//! and opens that file with follow mode on. A thread does the copy, so a
//! slow or endless writer never blocks a frame.

use crate::config::Config;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// What the TUI's title and batch mode's summary call stdin.
pub(crate) const LABEL: &str = "<stdin>";

/// The first size the log and the status line are told of; each later one
/// is ten times the one before: 1 MB, 10 MB, 100 MB, 1 GB, …
const FIRST_MILESTONE: u64 = 1_000_000;

/// How the copy stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum State {
    /// Stdin is still open: more may come.
    #[default]
    Open,
    /// Stdin ended, and every byte is in the file.
    Closed,
    /// The copy stopped before stdin ended, for the reason given — a full
    /// disk, say. Stdin is closed, so the writer gets SIGPIPE.
    Stopped(String),
}

/// What the copy thread tells the app.
#[derive(Debug, Default)]
struct Shared {
    state: State,
    /// Sizes passed since the app last asked, in bytes.
    milestones: Vec<u64>,
}

/// The file stdin is copied into, and how the copy stands.
#[derive(Debug)]
pub struct StdinFile {
    path: PathBuf,
    shared: Arc<Mutex<Shared>>,
    /// `[stdin] delete_temp_file`: remove the file when this is dropped.
    delete: bool,
}

/// Copy the process's stdin into a file in `$TMPDIR`, and point `config`
/// at that file in place of `-`. `main` calls it after [`Config::load`],
/// for the TUI: batch mode reads stdin itself.
///
/// # Errors
/// The file cannot be created, or the OS refuses the copy thread.
pub fn copy_into(config: &mut Config) -> io::Result<StdinFile> {
    let file = StdinFile::start(
        &std::env::temp_dir(),
        io::stdin(),
        close_stdin,
        config.delete_stdin_temp_file(),
    )?;
    log::debug!("stdin: copied into {}", file.path().display());
    config.read_stdin_into(file.path());
    Ok(file)
}

/// Close stdin, for a copy that stopped before it ended: `/dev/null` takes
/// fd 0, so the pipe has no reader and the writer gets SIGPIPE.
fn close_stdin() {
    #[cfg(unix)]
    match File::open("/dev/null") {
        Ok(null) => {
            if let Err(err) = rustix::stdio::dup2_stdin(&null) {
                log::warn!("stdin: cannot close: {err}");
            }
        }
        Err(err) => log::warn!("stdin: cannot open /dev/null to close stdin: {err}"),
    }
}

impl StdinFile {
    /// Create the file in `dir` and copy `stdin` into it on a thread of its
    /// own. `close` runs if the copy stops before stdin ends, and must
    /// close stdin, so that the writer is not left blocked on a full pipe.
    /// With `delete`, the file is removed when this is dropped.
    pub(crate) fn start(
        dir: &Path,
        stdin: impl Read + Send + 'static,
        close: impl FnOnce() + Send + 'static,
        delete: bool,
    ) -> io::Result<Self> {
        let (path, file) = create(dir)?;
        let shared = Arc::default();
        let copying = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("recon-stdin".to_string())
            .spawn(move || copy(stdin, file, &copying, close))?;
        Ok(Self {
            path,
            shared,
            delete,
        })
    }

    /// The file stdin is copied into.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// How the copy stands now.
    pub(crate) fn state(&self) -> State {
        lock(&self.shared).state.clone()
    }

    /// The sizes the copy passed since the last call, in bytes.
    pub(crate) fn take_milestones(&self) -> Vec<u64> {
        std::mem::take(&mut lock(&self.shared).milestones)
    }
}

impl Drop for StdinFile {
    fn drop(&mut self) {
        if self.delete
            && let Err(err) = std::fs::remove_file(&self.path)
        {
            log::warn!("stdin: cannot delete {}: {err}", self.path.display());
        }
    }
}

/// A panic on the copy thread leaves what it last wrote, which is as good
/// as any to show.
fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Create `recon-stdin-<YYYYMMDD-HHMMSS>-<pid>.txt` in `dir`, readable by
/// its owner only: what comes down a pipe can be private (`sudo …`).
fn create(dir: &Path) -> io::Result<(PathBuf, File)> {
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S");
    let path = dir.join(format!("recon-stdin-{stamp}-{}.txt", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(&path)?;
    Ok((path, file))
}

/// Copy `stdin` into `file` until it ends or a write fails, telling `shared`
/// how it stands. A write failure closes stdin with `close`.
///
/// Each read is written straight through, with no buffer of its own, so
/// follow mode sees a line as soon as the writer sends it.
fn copy(mut stdin: impl Read, mut file: impl Write, shared: &Mutex<Shared>, close: impl FnOnce()) {
    let set = |state| lock(shared).state = state;
    let mut buf = vec![0; 64 * 1024];
    let mut copied: u64 = 0;
    let mut next = FIRST_MILESTONE;
    loop {
        let read = match stdin.read(&mut buf) {
            Ok(0) => return set(State::Closed),
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return set(State::Stopped(err.to_string())),
        };
        if let Err(err) = file.write_all(&buf[..read]) {
            drop(stdin);
            close();
            return set(State::Stopped(reason(&err)));
        }
        copied += read as u64;
        while copied >= next {
            lock(shared).milestones.push(next);
            next = next.saturating_mul(10);
        }
    }
}

/// Why a write failed, as the title says it.
fn reason(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::StorageFull => "disk full".to_string(),
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fixture_dir;

    fn shared() -> Mutex<Shared> {
        Mutex::default()
    }

    fn state(shared: &Mutex<Shared>) -> State {
        lock(shared).state.clone()
    }

    /// A writer whose every write fails, as on a full disk.
    struct Full;

    impl Write for Full {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::StorageFull.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn every_byte_is_copied_and_the_end_of_stdin_closes_it() {
        let shared = shared();
        let mut file = Vec::new();
        let mut closed = false;

        copy(&b"one\ntwo\n"[..], &mut file, &shared, || closed = true);

        assert_eq!(file, b"one\ntwo\n");
        assert_eq!(state(&shared), State::Closed);
        assert!(!closed, "stdin ended by itself: nothing to close");
    }

    /// The log and the status line hear of 1 MB, 10 MB, … once each.
    #[test]
    fn each_tenfold_size_is_a_milestone_once() {
        let shared = shared();

        copy(
            io::repeat(b'x').take(12_000_000),
            io::sink(),
            &shared,
            || {},
        );

        let milestones = std::mem::take(&mut shared.lock().expect("lock").milestones);
        assert_eq!(milestones, [1_000_000, 10_000_000]);
    }

    /// A full disk stops the copy, says why, and closes stdin so the writer
    /// is not left blocked.
    #[test]
    fn a_failed_write_stops_the_copy_and_closes_stdin() {
        let shared = shared();
        let mut closed = false;

        copy(&b"one\n"[..], Full, &shared, || closed = true);

        assert_eq!(state(&shared), State::Stopped("disk full".to_string()));
        assert!(closed);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_named_for_its_run_and_only_its_owner_can_read_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fixture_dir("stdin_create");

        let (path, _) = create(&dir).expect("created");

        let name = path.file_name().expect("a name").to_string_lossy();
        let pid = std::process::id();
        assert!(name.starts_with("recon-stdin-"), "{name}");
        assert!(name.ends_with(&format!("-{pid}.txt")), "{name}");
        // `recon-stdin-` + `YYYYMMDD-HHMMSS` + `-<pid>.txt`.
        assert_eq!(
            name.len(),
            "recon-stdin-".len() + 15 + format!("-{pid}.txt").len()
        );
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// The thread reports what it did: here, that the empty stdin ended.
    #[test]
    fn a_started_copy_reports_the_end_of_stdin() {
        let file =
            StdinFile::start(&fixture_dir("stdin_state"), &b""[..], || {}, false).expect("started");

        let start = std::time::Instant::now();
        while file.state() == State::Open {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "timed out"
            );
            std::thread::yield_now();
        }
        assert_eq!(file.state(), State::Closed);
    }

    #[test]
    fn delete_removes_the_file_on_drop_and_the_default_keeps_it() {
        // Two folders: one run makes one file, so two in a second clash.
        let kept =
            StdinFile::start(&fixture_dir("stdin_keep"), &b""[..], || {}, false).expect("started");
        let kept_path = kept.path().to_path_buf();
        drop(kept);
        assert!(kept_path.exists(), "left for the OS by default");

        let deleted =
            StdinFile::start(&fixture_dir("stdin_delete"), &b""[..], || {}, true).expect("started");
        let deleted_path = deleted.path().to_path_buf();
        drop(deleted);
        assert!(!deleted_path.exists());
    }
}
