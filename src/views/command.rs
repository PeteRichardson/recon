//! Command views (#457): a program the user names in `[commands]`, whose
//! output recon shows as a view of the file.
//!
//! The argv is never given to a shell, as the editor templates are not.
//! `{file}` is the path; with no `{file}`, the file goes to stdin. `{out}` is
//! a new temporary path that recon reads after the command stops, then
//! deletes. `{px}` is the view pane's width in pixels.

use super::spool::Spool;
pub(crate) use super::spool::Spooling;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long a command runs before it is killed, when the config says nothing.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

/// How long the built-in `quicklook` runs (#506). The first `qlmanage`
/// after a cold start must start `quicklookd` and find its generators,
/// which can take more than [`DEFAULT_TIMEOUT`].
const QUICK_LOOK_TIMEOUT: Duration = Duration::from_secs(10);

/// One `[commands]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The program and its arguments, with the placeholders still in them.
    /// Never empty: the config refuses `[]`.
    pub argv: Vec<String>,
    /// The MIME type of the output. `None` is `text/plain`, highlighted by
    /// the file's name as raw is.
    pub output: Option<String>,
    pub timeout: Duration,
    /// Whether the command runs for the files near the selection before the
    /// user reaches them. Off for a command that is slow or has side effects.
    pub prefetch: bool,
    /// Whether the output is read as CSV and shown with its columns
    /// aligned (#575). For a command whose `output` is `text/csv` or
    /// `text/tab-separated-values`; a picture ignores it.
    pub align: bool,
    /// Whether all the output goes to a spool file (#573), not only what
    /// comes past `MAX_OUTPUT`. A spooled command never runs as a prefetch.
    pub spool: bool,
}

#[cfg(test)]
impl Command {
    /// A command for a test: `argv`, plain text, a 5 s timeout, prefetched.
    pub(crate) fn of(argv: &[&str]) -> Self {
        Self {
            argv: argv.iter().map(ToString::to_string).collect(),
            output: None,
            timeout: Duration::from_secs(5),
            prefetch: true,
            align: false,
            spool: false,
        }
    }

    pub(crate) fn with_output(mut self, output: &str) -> Self {
        self.output = Some(output.to_string());
        self
    }

    pub(crate) fn with_prefetch(mut self, prefetch: bool) -> Self {
        self.prefetch = prefetch;
        self
    }

    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub(crate) fn with_align(mut self, align: bool) -> Self {
        self.align = align;
        self
    }

    pub(crate) fn with_spool(mut self, spool: bool) -> Self {
        self.spool = spool;
        self
    }
}

#[cfg(test)]
impl Commands {
    /// `[commands]` for a test.
    pub(crate) fn of(entries: impl IntoIterator<Item = (&'static str, Command)>) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(name, command)| (name.to_string(), command))
                .collect(),
        )
    }
}

impl Command {
    /// Whether the output is an image, for the image view (#458).
    pub(crate) fn is_image(&self) -> bool {
        self.output
            .as_deref()
            .is_some_and(|mime| mime.trim().to_ascii_lowercase().starts_with("image/"))
    }

    /// The built-in `quicklook` command (#458): Quick Look's thumbnail of
    /// the file, `{px}` on its longest side. macOS only, and `None` where
    /// `qlmanage` is not on `PATH`.
    ///
    /// Through `sh`, because `qlmanage -o` writes into a directory that
    /// must already be there, and `{out}` is a path that is not. It writes
    /// nothing, and says that it made a thumbnail, when the directory is
    /// not there.
    pub(crate) fn quick_look() -> Option<Self> {
        let program = "qlmanage";
        (cfg!(target_os = "macos") && on_path(program)).then(|| Self {
            argv: [
                "sh",
                "-c",
                "mkdir \"$1\" && exec qlmanage -t -s \"$2\" -o \"$1\" \"$3\" >/dev/null",
                "sh",
                "{out}",
                "{px}",
                "{file}",
            ]
            .map(String::from)
            .to_vec(),
            output: Some("image/png".to_string()),
            timeout: QUICK_LOOK_TIMEOUT,
            prefetch: true,
            align: false,
            spool: false,
        })
    }
}

/// The table form of a command, with every key spelt out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    argv: Vec<String>,
    output: Option<String>,
    timeout: Option<f64>,
    prefetch: Option<bool>,
    align: Option<bool>,
    spool: Option<bool>,
}

impl<'de> Deserialize<'de> for Command {
    /// An argv array, or a table with `argv`, `output`, `timeout`,
    /// `prefetch`, `align` and `spool`. A hand-written visitor rather than
    /// `untagged`, so that a typo in a table key is named in the error,
    /// not lost in "did not match any variant".
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, Error, MapAccess, SeqAccess, Visitor};

        struct CommandVisitor;

        impl<'de> Visitor<'de> for CommandVisitor {
            type Value = Table;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an argv array or a table with `argv`")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Table, A::Error> {
                let argv = Vec::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
                Ok(Table {
                    argv,
                    output: None,
                    timeout: None,
                    prefetch: None,
                    align: None,
                    spool: None,
                })
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Table, A::Error> {
                Table::deserialize(de::value::MapAccessDeserializer::new(map))
            }
        }

        let table = deserializer.deserialize_any(CommandVisitor)?;
        if table.argv.is_empty() {
            return Err(D::Error::custom("`argv` names no program"));
        }
        let timeout = match table.timeout {
            None => DEFAULT_TIMEOUT,
            Some(seconds) if seconds.is_finite() && seconds > 0.0 => {
                Duration::from_secs_f64(seconds)
            }
            Some(seconds) => {
                return Err(D::Error::custom(format!(
                    "`timeout = {seconds}` must be a number of seconds above 0"
                )));
            }
        };
        Ok(Self {
            argv: table.argv,
            output: table.output,
            timeout,
            prefetch: table.prefetch.unwrap_or(true),
            align: table.align.unwrap_or(false),
            spool: table.spool.unwrap_or(false),
        })
    }
}

/// `[commands]`, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commands(pub BTreeMap<String, Command>);

impl<'de> Deserialize<'de> for Commands {
    /// A command named for a built-in view is refused: a row that names it
    /// could mean either one.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let commands = BTreeMap::<String, Command>::deserialize(deserializer)?;
        if let Some(name) = commands
            .keys()
            .find(|name| super::BUILT_IN.contains(&name.as_str()))
        {
            return Err(D::Error::custom(format!(
                "`{name}` is the name of a built-in view; give the command another name"
            )));
        }
        Ok(Self(commands))
    }
}

/// The most output a view keeps in memory. Past it the output goes to a
/// spool (#573); for a picture or `{out}`, which are read whole, the rest
/// is read and dropped, so that the command is not stopped on a full pipe.
pub(crate) const MAX_OUTPUT: u64 = crate::views::read::MAX_PREVIEW_BYTES;

/// The most stderr kept: only its first line is shown.
const MAX_STDERR: u64 = 64 << 10;

/// How often a running command is checked for its exit. A poll, because
/// only the thread that holds the `Child` can kill it at the timeout.
const POLL: Duration = Duration::from_millis(5);

/// A new, empty directory for one run's `{out}`, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "recon-view-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a command gave.
#[derive(Debug)]
pub(crate) enum Output {
    /// All its output, below the spool threshold.
    Bytes(Vec<u8>),
    /// Its output in a file (#573). The command can still be running.
    Spooled(Spool),
}

/// The start of stdout, and the pipe when more can come after it.
#[derive(Default)]
struct Head {
    bytes: Vec<u8>,
    rest: Option<ChildStdout>,
}

/// Run `command` on `file`, for a pane `px` pixels wide, and give back
/// its output: its bytes, or a spool when its stdout goes past
/// `spooling.bytes` or the command has `spool = true` (#573). A spool is
/// read again as an index from `index_bytes` (#51).
///
/// From the spool on, `timeout` is the most time with no output, not the
/// most time the command runs.
///
/// Each run is in the log (#490): the argv, how long it ran and its exit
/// status at `Debug`, and at `Warn` when it could not start, was killed at
/// its timeout or failed, with the first line of its stderr.
///
/// # Errors
///
/// Why the view failed, for the status line: the program could not start,
/// it ran past its timeout, it exited with an error (the first line of its
/// stderr), it wrote nothing to `{out}`, or the spool could not be made.
pub(crate) fn run(
    command: &Command,
    file: &Path,
    px: u32,
    spooling: &Spooling,
    index_bytes: u64,
) -> Result<Output, String> {
    let uses = |placeholder: &str| command.argv.iter().any(|arg| arg.contains(placeholder));
    let (to_stdin, to_out) = (!uses("{file}"), uses("{out}"));
    let scratch = to_out
        .then(Scratch::new)
        .transpose()
        .map_err(|err| format!("cannot make a temporary directory: {err}"))?;
    let out = scratch.as_ref().map(|scratch| scratch.0.join("out"));
    let px = px.to_string();
    // The editor's one substitution pass (#182): a path that is not UTF-8
    // goes to the command as it is on disk.
    let argv = crate::editor::fill(
        &command.argv,
        &[
            ("file", file.as_os_str()),
            ("out", out.as_deref().unwrap_or(Path::new("")).as_os_str()),
            ("px", px.as_ref()),
        ],
    );

    let program = &command.argv[0];
    let started = Instant::now();
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        // The leader of a group of its own, so that a kill reaches what it
        // started too: `sh -c 'decode | grep'` must not leave `decode`
        // writing (#573).
        .process_group(0)
        .stdin(if to_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            log::warn!("command view {argv:?}: cannot start: {err}");
            match err.kind() {
                io::ErrorKind::NotFound => format!("`{program}` not found"),
                _ => format!("cannot start `{program}`: {err}"),
            }
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        let file = file.to_path_buf();
        // A command that stops reading early closes the pipe; that is its
        // business, not a failure of the view.
        thread::spawn(move || {
            if let Ok(mut source) = std::fs::File::open(file) {
                let _ = io::copy(&mut source, &mut stdin);
                let _ = stdin.flush();
            }
        });
    }
    // A picture and `{out}` are read whole after the command: only text on
    // stdout spools.
    let spools = !to_out && !command.is_image();
    let threshold = match (spools, command.spool) {
        (false, _) => MAX_OUTPUT,
        (true, true) => 0,
        (true, false) => spooling.bytes,
    };
    let stdout = head(child.stdout.take(), threshold, spools);
    let stderr = drain(child.stderr.take(), MAX_STDERR);
    let spool = |child, bytes: Vec<u8>, rest, stderr| {
        log::debug!(
            "command view {argv:?}: spooled after {:?}",
            started.elapsed()
        );
        let timeout = command.timeout;
        Spool::start(
            &spooling.dir,
            index_bytes,
            child,
            rest,
            &bytes,
            timeout,
            stderr,
        )
        .map(Output::Spooled)
        .map_err(|err| {
            log::warn!("command view {argv:?}: cannot spool: {err}");
            format!("cannot spool: {err}")
        })
    };

    let deadline = Instant::now() + command.timeout;
    let mut stdout_bytes = None;
    let status = loop {
        match stdout.try_recv() {
            Ok(Head {
                bytes,
                rest: Some(rest),
            }) => return spool(child, bytes, rest, stderr),
            Ok(Head { bytes, rest: None }) => stdout_bytes = Some(bytes),
            Err(_) => {}
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            // Past the deadline, or the wait itself failed.
            _ => {
                kill(&mut child);
                log::warn!(
                    "command view {argv:?}: killed after {:?}, its timeout",
                    command.timeout
                );
                return Err(format!(
                    "timed out after {}s",
                    command.timeout.as_secs_f64()
                ));
            }
        }
    };
    let ran = started.elapsed();
    // A grandchild can hold a pipe open after the command exits, so the
    // readers get what is left of the timeout and no more.
    let left = || deadline.saturating_duration_since(Instant::now());
    let stdout = match stdout_bytes {
        Some(bytes) => bytes,
        None => match stdout.recv_timeout(left()).unwrap_or_default() {
            // A command can write past the threshold and exit before it is
            // seen.
            Head {
                bytes,
                rest: Some(rest),
            } => return spool(child, bytes, rest, stderr),
            Head { bytes, rest: None } => bytes,
        },
    };
    let stderr = stderr.recv_timeout(left()).unwrap_or_default();
    if !status.success() {
        let first = first_line(&stderr);
        log::warn!(
            "command view {argv:?}: {status} after {ran:?}, stderr: {}",
            first.as_deref().unwrap_or("")
        );
        return Err(first.unwrap_or_else(|| status.to_string()));
    }
    log::debug!("command view {argv:?}: {status} after {ran:?}");
    match out {
        Some(out) => read_out(&out).map(Output::Bytes),
        None => Ok(Output::Bytes(stdout)),
    }
}

/// Kill `child` and all it started: it leads a process group of its own.
pub(crate) fn kill(child: &mut Child) {
    if let Some(pid) = i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The first line of `stderr` with text on it: the reason a command
/// failed.
pub(crate) fn first_line(stderr: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Read `pipe` on its own thread until it ends or `cap` bytes came. At the
/// cap, the pipe comes back with the bytes when `keep_rest`, for a spool.
fn head(pipe: Option<ChildStdout>, cap: u64, keep_rest: bool) -> mpsc::Receiver<Head> {
    let (sender, receiver) = mpsc::channel();
    if let Some(pipe) = pipe {
        thread::spawn(move || {
            let (bytes, rest) = read_up_to(pipe, cap, keep_rest);
            let _ = sender.send(Head { bytes, rest });
        });
    }
    receiver
}

/// Read `pipe` on its own thread, keeping at most `cap` bytes.
fn drain(pipe: Option<impl Read + Send + 'static>, cap: u64) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(pipe) = pipe {
        thread::spawn(move || {
            let _ = sender.send(read_up_to(pipe, cap, false).0);
        });
    }
    receiver
}

/// The first `cap` bytes of `pipe`, and, with `keep_rest`, the pipe when
/// more can come. Otherwise the rest is read and dropped, so that the
/// command is not stopped on a full pipe.
fn read_up_to<R: Read>(mut pipe: R, cap: u64, keep_rest: bool) -> (Vec<u8>, Option<R>) {
    let mut bytes = Vec::new();
    let read = (&mut pipe).take(cap).read_to_end(&mut bytes);
    if keep_rest && read.is_ok() && bytes.len() as u64 == cap {
        return (bytes, Some(pipe));
    }
    let _ = io::copy(&mut pipe, &mut io::sink());
    (bytes, None)
}

/// What the command wrote to `{out}`: a file, or a directory with one file
/// in it, as `qlmanage -o` makes.
fn read_out(out: &Path) -> Result<Vec<u8>, String> {
    let file = if out.is_dir() {
        let mut entries = std::fs::read_dir(out)
            .map_err(|err| format!("cannot read {{out}}: {err}"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file());
        match (entries.next(), entries.next()) {
            (Some(file), None) => file,
            _ => return Err("{out} is a directory without exactly one file".to_string()),
        }
    } else {
        out.to_path_buf()
    };
    let mut kept = Vec::new();
    std::fs::File::open(&file)
        .and_then(|source| source.take(MAX_OUTPUT).read_to_end(&mut kept))
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => "wrote nothing to {out}".to_string(),
            _ => format!("cannot read {{out}}: {err}"),
        })?;
    Ok(kept)
}

/// Whether `program` can be started: a path to a file, or a name found on
/// `PATH`.
pub(crate) fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return is_executable(Path::new(program));
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| is_executable(&dir.join(program)))
    })
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fixture_file;
    use crate::views::spool::{Spool, State};

    fn command(argv: &[&str]) -> Command {
        Command::of(argv)
    }

    /// `run` with the app's spool threshold, which no test output reaches.
    fn run(command: &Command, file: &Path, px: u32) -> Result<Output, String> {
        super::run(
            command,
            file,
            px,
            &Spooling::default(),
            crate::document::INDEX_BYTES,
        )
    }

    fn text(output: Result<Output, String>) -> String {
        match output.expect("the command ran") {
            Output::Bytes(bytes) => String::from_utf8(bytes).unwrap(),
            Output::Spooled(spool) => panic!("spooled to {}", spool.path().display()),
        }
    }

    #[test]
    fn with_no_file_placeholder_the_file_goes_to_stdin() {
        let file = fixture_file("command_stdin.txt", b"one\ntwo\n");
        assert_eq!(text(run(&command(&["cat"]), &file, 0)), "one\ntwo\n");
    }

    #[test]
    fn file_is_the_path_and_px_the_pane_width() {
        let file = fixture_file("command_file.txt", b"x\n");
        let output = text(run(&command(&["echo", "{file}", "{px}"]), &file, 640));
        assert_eq!(output, format!("{} 640\n", file.display()));
    }

    #[test]
    fn out_is_read_and_then_deleted() {
        let file = fixture_file("command_out.txt", b"copied\n");
        // The command writes `{out}`'s own path after the copy, so the test
        // can look for it afterwards.
        let script = "cp \"$1\" \"$2\" && echo \"$2\" >> \"$2\"";
        let output = text(run(
            &command(&["sh", "-c", script, "sh", "{file}", "{out}"]),
            &file,
            0,
        ));
        let (copied, out) = output.split_once('\n').unwrap();
        assert_eq!(copied, "copied");
        let out = Path::new(out.trim_end());
        assert!(!out.exists(), "{} was left", out.display());
        assert!(!out.parent().unwrap().exists(), "its directory was left");
    }

    #[test]
    fn out_as_a_directory_gives_its_one_file() {
        let file = fixture_file("command_out_dir.txt", b"inside\n");
        let output = run(
            &command(&[
                "sh",
                "-c",
                "mkdir \"$1\" && cp \"$2\" \"$1/x.txt\"",
                "sh",
                "{out}",
                "{file}",
            ]),
            &file,
            0,
        );
        assert_eq!(text(output), "inside\n");
    }

    #[test]
    fn a_non_zero_exit_fails_with_the_first_line_of_stderr() {
        let file = fixture_file("command_exit.txt", b"x\n");
        let err = run(
            &command(&[
                "sh",
                "-c",
                "echo; echo 'bad input' >&2; echo more >&2; exit 3",
            ]),
            &file,
            0,
        )
        .expect_err("a failure");
        assert_eq!(err, "bad input");
    }

    #[test]
    fn a_command_past_its_timeout_is_killed() {
        let file = fixture_file("command_timeout.txt", b"x\n");
        let mut slow = command(&["sleep", "10"]);
        slow.timeout = Duration::from_millis(100);
        let started = std::time::Instant::now();
        let err = run(&slow, &file, 0).expect_err("a timeout");
        assert_eq!(err, "timed out after 0.1s");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "it was not killed"
        );
    }

    /// A spool threshold of `bytes`, in a directory of the test's own.
    fn spooling(name: &str, bytes: u64) -> Spooling {
        Spooling {
            bytes,
            dir: crate::fixtures::fixture_dir(name),
        }
    }

    /// The spool `output` is, and its lines once the command is over.
    fn spooled(output: Result<Output, String>) -> (Spool, crate::lines::Lines) {
        let Output::Spooled(spool) = output.expect("the command ran") else {
            panic!("the output was not spooled");
        };
        let started = Instant::now();
        loop {
            if let Some(lines) = spool.take_lines() {
                return (spool, lines);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "it never ended"
            );
            thread::sleep(POLL);
        }
    }

    /// Past the threshold, the output goes to a file: all of it, not only
    /// what came before the threshold (#573).
    #[test]
    fn output_past_the_threshold_goes_to_a_spool() {
        let file = fixture_file("command_spool.txt", b"x\n");
        let output = super::run(
            &command(&["printf", "one\\ntwo\\nthree\\n"]),
            &file,
            0,
            &spooling("command_spool", 5),
            crate::document::INDEX_BYTES,
        );
        let (spool, lines) = spooled(output);
        assert_eq!(lines, ["one", "two", "three"]);
        assert_eq!(spool.state(), State::Ended);
    }

    /// For a spool, `timeout` is the most time with no output: a command
    /// that writes all the time runs past it, and one that stops writing
    /// is killed. Its lines stay.
    #[test]
    fn a_spooled_command_is_killed_only_when_it_is_quiet() {
        let file = fixture_file("command_spool_quiet.txt", b"x\n");
        let script = "for i in 1 2 3 4 5 6; do echo $i; sleep 0.1; done; sleep 10";
        let started = Instant::now();
        let output = super::run(
            &command(&["sh", "-c", script]).with_timeout(Duration::from_millis(400)),
            &file,
            0,
            &spooling("command_spool_quiet", 1),
            crate::document::INDEX_BYTES,
        );
        let (spool, lines) = spooled(output);
        assert_eq!(lines, ["1", "2", "3", "4", "5", "6"]);
        assert_eq!(spool.state(), State::Stopped("no output for 0.4s".into()));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it was not killed"
        );
    }

    /// `stop` kills the command, and the lines it wrote stay.
    #[test]
    fn a_stopped_spool_keeps_its_lines() {
        let file = fixture_file("command_spool_stop.txt", b"x\n");
        let output = super::run(
            &command(&["sh", "-c", "echo one; echo two; sleep 10"]),
            &file,
            0,
            &spooling("command_spool_stop", 1),
            crate::document::INDEX_BYTES,
        );
        let Ok(Output::Spooled(spool)) = &output else {
            panic!("not spooled: {output:?}");
        };
        let started = Instant::now();
        while std::fs::read(spool.path()).unwrap() != b"one\ntwo\n" {
            assert!(started.elapsed() < Duration::from_secs(10), "no lines came");
            thread::sleep(POLL);
        }
        spool.stop();
        let (spool, lines) = spooled(output);
        assert_eq!(lines, ["one", "two"]);
        assert_eq!(spool.state(), State::Stopped("by the user".into()));
    }

    /// A command that fails after its output went to the spool says why, as
    /// one that fails before does: the first line of its stderr.
    #[test]
    fn a_spooled_command_that_fails_gives_its_stderr() {
        let file = fixture_file("command_spool_fail.txt", b"x\n");
        let output = super::run(
            &command(&["sh", "-c", "echo out; echo 'bad input' >&2; exit 3"]),
            &file,
            0,
            &spooling("command_spool_fail", 1),
            crate::document::INDEX_BYTES,
        );
        let (spool, lines) = spooled(output);
        assert_eq!(lines, ["out"]);
        assert_eq!(spool.state(), State::Stopped("bad input".into()));
    }

    /// Dropping the spool kills the command, and what it started, and
    /// deletes the file: the user left the file, or recon exits.
    #[test]
    fn dropping_a_spool_kills_its_command_and_deletes_its_file() {
        let dir = crate::fixtures::fixture_dir("command_spool_drop");
        let file = fixture_file("command_spool_drop.txt", b"x\n");
        let mark = dir.join("still-running");
        // The mark is the work of a child of the command: the kill must
        // reach it too.
        let script = format!("echo one; (sleep 1; touch '{}') & wait", mark.display());
        let output = super::run(
            &command(&["sh", "-c", &script]),
            &file,
            0,
            &Spooling {
                dir: dir.clone(),
                ..spooling("command_spool_drop_unused", 1)
            },
            crate::document::INDEX_BYTES,
        );
        let Ok(Output::Spooled(spool)) = output else {
            panic!("not spooled: {output:?}");
        };
        let path = spool.path().to_path_buf();
        assert!(
            path.starts_with(&dir),
            "{} is not in [spool] dir",
            path.display()
        );
        drop(spool);
        assert!(!path.exists(), "{} was left", path.display());
        thread::sleep(Duration::from_millis(1500));
        assert!(!mark.exists(), "the command was not killed");
    }

    /// `spool = true` sends all the output to the spool, however small.
    #[test]
    fn spool_true_spools_from_the_first_byte() {
        let file = fixture_file("command_spool_true.txt", b"x\n");
        let output = super::run(
            &command(&["echo", "small"]).with_spool(true),
            &file,
            0,
            &spooling("command_spool_true", MAX_OUTPUT),
            crate::document::INDEX_BYTES,
        );
        let (_, lines) = spooled(output);
        assert_eq!(lines, ["small"]);
    }

    /// A picture is decoded whole, so it never spools: past the threshold
    /// it comes back as bytes, cut only at `MAX_OUTPUT`, as before.
    #[test]
    fn a_picture_never_spools() {
        let file = fixture_file("command_spool_image.txt", b"x\n");
        let output = super::run(
            &command(&["printf", "0123456789"]).with_output("image/png"),
            &file,
            0,
            &spooling("command_spool_image", 4),
            crate::document::INDEX_BYTES,
        );
        assert_eq!(text(output), "0123456789");
    }

    #[test]
    fn a_missing_program_says_so() {
        let file = fixture_file("command_missing.txt", b"x\n");
        let err = run(&command(&["recon-no-such-program"]), &file, 0).expect_err("a failure");
        assert_eq!(err, "`recon-no-such-program` not found");
    }

    #[test]
    fn on_path_finds_a_program_by_name_or_path() {
        assert!(on_path("sh"));
        assert!(on_path("/bin/sh"));
        assert!(!on_path("recon-no-such-program"));
        assert!(!on_path("/no/such/program"));
    }

    /// A cold `qlmanage` can be slow, so `quicklook` waits longer than
    /// other commands before it fails (#506).
    #[cfg(target_os = "macos")]
    #[test]
    fn quicklook_waits_longer_than_other_commands() {
        let quick_look = Command::quick_look().expect("qlmanage on PATH");
        assert!(
            quick_look.timeout > DEFAULT_TIMEOUT,
            "{:?}",
            quick_look.timeout
        );
    }

    fn commands(toml: &str) -> Result<Commands, toml::de::Error> {
        #[derive(Deserialize)]
        struct File {
            commands: Commands,
        }
        toml::from_str::<File>(toml).map(|file| file.commands)
    }

    #[test]
    fn a_command_is_an_argv_or_a_table() {
        let parsed = commands(
            "[commands]\njq = [\"jq\", \".\"]\n\
             summary = { argv = [\"s\", \"{file}\"], output = \"text/markdown\", timeout = 30, prefetch = false }\n\
             csv = { argv = [\"transmcap\", \"{file}\"], output = \"text/csv\", align = true }\n\
             big = { argv = [\"decode\", \"{file}\"], spool = true }\n",
        )
        .unwrap();
        assert_eq!(
            parsed.0["jq"],
            command(&["jq", "."]).with_timeout(DEFAULT_TIMEOUT)
        );
        let summary = &parsed.0["summary"];
        assert_eq!(summary.output.as_deref(), Some("text/markdown"));
        assert_eq!(summary.timeout, Duration::from_secs(30));
        assert!(!summary.prefetch);
        assert!(!summary.align);
        let csv = &parsed.0["csv"];
        assert!(csv.align, "`align = true` was not read");
        assert!(!parsed.0["jq"].align, "an argv command does not align");
        assert!(parsed.0["big"].spool, "`spool = true` was not read");
        assert!(!parsed.0["jq"].spool, "an argv command does not spool");
    }

    #[test]
    fn a_bad_command_is_refused_with_its_reason() {
        let reserved = commands("[commands]\nhex = [\"xxd\"]\n").unwrap_err();
        assert!(reserved.to_string().contains("built-in view"), "{reserved}");
        let table = commands("[commands]\ntable = [\"xsv\", \"table\"]\n").unwrap_err();
        assert!(table.to_string().contains("built-in view"), "{table}");
        let empty = commands("[commands]\njq = []\n").unwrap_err();
        assert!(empty.to_string().contains("names no program"), "{empty}");
        let typo = commands("[commands]\njq = { argv = [\"jq\"], timout = 3 }\n").unwrap_err();
        assert!(typo.to_string().contains("timout"), "{typo}");
        let zero = commands("[commands]\njq = { argv = [\"jq\"], timeout = 0 }\n").unwrap_err();
        assert!(zero.to_string().contains("above 0"), "{zero}");
    }

    /// `align = true` reads the output as CSV and lines the columns up
    /// (#575), one line for each record even when a quoted field holds a
    /// comma or a new line.
    #[test]
    fn an_aligned_command_output_lands_in_columns() {
        let csv = "name,qty\nwidget,3\n\"abacus,x\",120\n";
        let aligned =
            crate::views::read::command_output(csv.as_bytes().to_vec(), Some("text/csv"), true);
        assert_eq!(
            *aligned.lines,
            ["name      qty", "widget    3", "abacus,x  120"]
        );
        assert!(aligned.text);
        assert_eq!(aligned.mime.as_deref(), Some("text/csv"));

        let raw =
            crate::views::read::command_output(csv.as_bytes().to_vec(), Some("text/csv"), false);
        assert_eq!(*raw.lines, ["name,qty", "widget,3", "\"abacus,x\",120"]);
    }

    /// `text/tab-separated-values` aligns on tabs.
    #[test]
    fn an_aligned_tsv_output_lands_in_columns() {
        let tsv = "a\tb\nlong\tx\n";
        let aligned = crate::views::read::command_output(
            tsv.as_bytes().to_vec(),
            Some("text/tab-separated-values"),
            true,
        );
        assert_eq!(*aligned.lines, ["a     b", "long  x"]);
    }

    /// Every run is in the log (#490): the argv, how long it ran and how it
    /// ended at `Debug`, and a failure at `Warn` with the reason.
    #[test]
    fn each_run_is_logged_and_a_failure_is_a_warning() {
        use crate::logging::capture::{here_at, install};
        use log::Level::{Debug, Warn};
        install();
        let file = fixture_file("command_logged.txt", b"x\n");

        text(run(&command(&["echo", "ok490"]), &file, 0));
        let ok = here_at(
            Debug,
            "command view [\"echo\", \"ok490\"]: exit status: 0 after ",
        );
        assert_eq!(ok.len(), 1, "{ok:?}");

        let _ = run(&command(&["sh", "-c", "echo bad490 >&2; exit 3"]), &file, 0);
        let bad = here_at(Warn, "bad490");
        assert!(
            bad.iter()
                .any(|text| text.contains("exit status: 3") && text.ends_with("stderr: bad490")),
            "{bad:?}"
        );

        let _ = run(
            &command(&["sh", "-c", "sleep 5", "slow490"]).with_timeout(Duration::from_millis(50)),
            &file,
            0,
        );
        let slow = here_at(Warn, "slow490");
        assert!(
            slow.iter().any(|text| text.contains("killed after")),
            "{slow:?}"
        );

        let _ = run(&command(&["recon-no-such-program-490"]), &file, 0);
        let missing = here_at(Warn, "recon-no-such-program-490");
        assert!(
            missing.iter().any(|text| text.contains("cannot start")),
            "{missing:?}"
        );
    }
}
