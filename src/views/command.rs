//! Command views (#457): a program the user names in `[commands]`, whose
//! output recon shows as a view of the file.
//!
//! The argv is never given to a shell, as the editor templates are not.
//! `{file}` is the path; with no `{file}`, the file goes to stdin. `{out}` is
//! a new temporary path that recon reads after the command stops, then
//! deletes. `{px}` is the view pane's width in pixels.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long a command runs before it is killed, when the config says nothing.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// Whether the output is an image, which needs the image view (stage 4).
    pub(crate) fn is_image(&self) -> bool {
        self.output
            .as_deref()
            .is_some_and(|mime| mime.trim().to_ascii_lowercase().starts_with("image/"))
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
}

impl<'de> Deserialize<'de> for Command {
    /// An argv array, or a table with `argv`, `output`, `timeout` and
    /// `prefetch`. A hand-written visitor rather than `untagged`, so that a
    /// typo in a table key is named in the error, not lost in "did not match
    /// any variant".
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

/// The most output a view keeps. Past it the rest is read and dropped, so
/// that the command is not stopped on a full pipe.
const MAX_OUTPUT: u64 = crate::views::read::MAX_PREVIEW_BYTES;

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

/// Run `command` on `file`, for a pane `px` pixels wide, and give back the
/// bytes of its output.
///
/// # Errors
///
/// Why the view failed, for the status line: the program could not start,
/// it ran past its timeout, it exited with an error (the first line of its
/// stderr), or it wrote nothing to `{out}`.
pub(crate) fn run(command: &Command, file: &Path, px: u32) -> Result<Vec<u8>, String> {
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
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(if to_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => format!("`{program}` not found"),
            _ => format!("cannot start `{program}`: {err}"),
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
    let stdout = drain(child.stdout.take(), MAX_OUTPUT);
    let stderr = drain(child.stderr.take(), MAX_STDERR);

    let deadline = Instant::now() + command.timeout;
    let status = wait(&mut child, deadline)
        .ok_or_else(|| format!("timed out after {}s", command.timeout.as_secs_f64()))?;
    // A grandchild can hold a pipe open after the command exits, so the
    // readers get what is left of the timeout and no more.
    let left = || deadline.saturating_duration_since(Instant::now());
    let stdout = stdout.recv_timeout(left()).unwrap_or_default();
    let stderr = stderr.recv_timeout(left()).unwrap_or_default();
    if !status.success() {
        let first = String::from_utf8_lossy(&stderr)
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string);
        return Err(first.unwrap_or_else(|| status.to_string()));
    }
    match out {
        Some(out) => read_out(&out),
        None => Ok(stdout),
    }
}

/// Read `pipe` on its own thread, keeping at most `cap` bytes.
fn drain(pipe: Option<impl Read + Send + 'static>, cap: u64) -> mpsc::Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        thread::spawn(move || {
            let mut kept = Vec::new();
            let _ = (&mut pipe).take(cap).read_to_end(&mut kept);
            let _ = io::copy(&mut pipe, &mut io::sink());
            let _ = sender.send(kept);
        });
    }
    receiver
}

/// The child's exit, or `None` after it was killed at `deadline`.
fn wait(child: &mut Child, deadline: Instant) -> Option<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            // Past the deadline, or the wait itself failed.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
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

    fn command(argv: &[&str]) -> Command {
        Command::of(argv)
    }

    fn text(output: Result<Vec<u8>, String>) -> String {
        String::from_utf8(output.expect("the command ran")).unwrap()
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
             summary = { argv = [\"s\", \"{file}\"], output = \"text/markdown\", timeout = 30, prefetch = false }\n",
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
    }

    #[test]
    fn a_bad_command_is_refused_with_its_reason() {
        let reserved = commands("[commands]\nhex = [\"xxd\"]\n").unwrap_err();
        assert!(reserved.to_string().contains("built-in view"), "{reserved}");
        let empty = commands("[commands]\njq = []\n").unwrap_err();
        assert!(empty.to_string().contains("names no program"), "{empty}");
        let typo = commands("[commands]\njq = { argv = [\"jq\"], timout = 3 }\n").unwrap_err();
        assert!(typo.to_string().contains("timout"), "{typo}");
        let zero = commands("[commands]\njq = { argv = [\"jq\"], timeout = 0 }\n").unwrap_err();
        assert!(zero.to_string().contains("above 0"), "{zero}");
    }
}
