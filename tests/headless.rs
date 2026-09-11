//! Headless mode (#143) end to end: the built binary, a piped stdin, real
//! files and a real `filters.toml`. `src/headless.rs` tests the pieces;
//! this is the one place `main`'s headless decision, the flags, the
//! summary on stderr and the exit code are exercised together — with no
//! tty, so it runs in CI.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A fresh directory for one test under `target/`, named after the test so
/// parallel tests never share one.
fn fixture(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures-headless")
        .join(name);
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

/// A config home whose `filters.toml` defines one set, `Bugs`, with the
/// include filters `hit` and `other` and a profile `only_hit`. No `default`
/// profile, so `--set Bugs` alone would enable nothing — the tests always
/// name the profile.
fn config_home(dir: &Path) -> PathBuf {
    let home = dir.join("config");
    fs::create_dir_all(home.join("recon")).expect("create config dir");
    fs::write(
        home.join("recon/filters.toml"),
        "[sets.Bugs]\n\n\
         [sets.Bugs.profiles]\n\
         only_hit = [\"hit\"]\n\n\
         [[sets.Bugs.filters]]\n\
         name = \"hit\"\n\
         pattern = \"hit\"\n\n\
         [[sets.Bugs.filters]]\n\
         name = \"other\"\n\
         pattern = \"other\"\n",
    )
    .expect("write filters.toml");
    home
}

/// Run recon with `args`, `stdin` piped in and closed, and the config home
/// at `home`. Stdin is a pipe even when empty, which is what makes the run
/// headless.
fn recon(home: &Path, args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_recon"))
        .args(args)
        .env("XDG_CONFIG_HOME", home)
        .env_remove("RECON_LOG")
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn recon");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    child.wait_with_output().expect("wait for recon")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("utf-8 output")
}

#[test]
fn files_hide_lists_the_matching_inputs_from_stdin() {
    let dir = fixture("files_hide");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    let b = dir.join("b.log");
    fs::write(&a, "hit\n").expect("write");
    fs::write(&b, "miss\n").expect("write");
    let list = format!("{}\n{}\n", a.display(), b.display());

    let out = recon(
        &home,
        &["--emit", "files", "--set", "Bugs:only_hit", "--hide"],
        list.as_bytes(),
    );

    assert_eq!(text(&out.stdout), format!("{}\n", a.display()));
    assert_eq!(
        text(&out.stderr),
        "recon: emitted 1 files of 2 inputs, hide mode\n"
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn lines_n_over_two_files_prefixes_path_and_line_number() {
    let dir = fixture("lines_two_files");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    let b = dir.join("b.log");
    fs::write(&a, "hit\n").expect("write");
    fs::write(&b, "miss\nhit\n").expect("write");
    let list = format!("{}\n{}\n", b.display(), a.display());

    let out = recon(
        &home,
        &["--emit", "lines", "-n", "--set", "Bugs:only_hit", "--hide"],
        list.as_bytes(),
    );

    assert_eq!(
        text(&out.stdout),
        format!("{}\t2\thit\n{}\t1\thit\n", b.display(), a.display()),
        "b before a: input order"
    );
    assert_eq!(
        text(&out.stderr),
        "recon: emitted 2 lines of 2 files, hide mode\n"
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn a_path_file_with_empty_stdin_is_read_headless_and_quiet_drops_the_summary() {
    let dir = fixture("quiet");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "hit\nmiss\n").expect("write");

    let out = recon(
        &home,
        &["--emit", "lines", "-q", a.to_str().expect("utf-8 path")],
        b"",
    );

    assert_eq!(
        text(&out.stdout),
        "hit\nmiss\n",
        "dim mode, no filter: the whole file"
    );
    assert!(out.stderr.is_empty(), "stderr: {}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn an_unreadable_input_warns_skips_and_exits_2() {
    let dir = fixture("unreadable");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "hit\n").expect("write");
    let missing = dir.join("missing.log");
    let list = format!("{}\n{}\n", missing.display(), a.display());

    let out = recon(
        &home,
        &["--emit", "lines", "--set", "Bugs:only_hit", "--hide"],
        list.as_bytes(),
    );

    assert_eq!(text(&out.stdout), format!("{}\thit\n", a.display()));
    assert_eq!(
        text(&out.stderr),
        format!(
            "recon: cannot read {}: no such file\nrecon: emitted 1 lines of 1 file, hide mode\n",
            missing.display()
        )
    );
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn cwd_over_a_path_directory_prints_it_without_reading_anything() {
    let dir = fixture("cwd");
    let home = config_home(&dir);
    fs::write(dir.join("core.bin"), b"\0\0\0").expect("write");

    let out = recon(
        &home,
        &["--emit", "cwd", dir.to_str().expect("utf-8 path")],
        b"",
    );

    assert_eq!(text(&out.stdout), format!("{}\n", dir.display()));
    assert_eq!(
        text(&out.stderr),
        format!("recon: emitted {}\n", dir.display())
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn an_unknown_set_is_refused_before_anything_is_read() {
    let dir = fixture("unknown_set");
    let home = config_home(&dir);

    let out = recon(&home, &["--emit", "files", "--set", "Nope"], b"");

    assert!(out.stdout.is_empty());
    assert!(
        text(&out.stderr).contains("unknown set \"Nope\"; filters.toml defines: Bugs"),
        "stderr: {}",
        text(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1));
}

/// A refusal that happens before any terminal setup must not write terminal
/// control codes. A script that keeps stderr in a file holds them verbatim,
/// and the message is then not the first thing in the file (#222).
#[test]
fn a_refusal_before_the_terminal_writes_no_escape_sequences() {
    let dir = fixture("no_escapes");
    let home = config_home(&dir);

    let out = recon(&home, &["--emit", "files", "--set", "Nope"], b"");

    assert_ne!(
        out.stderr.first(),
        Some(&0x1b),
        "stderr starts with an escape sequence: {:?}",
        text(&out.stderr)
    );
    assert!(
        text(&out.stderr).contains("unknown set"),
        "stderr: {}",
        text(&out.stderr)
    );
}

/// `--print-editor-config` prints a stanza and exits. It must not need
/// `filters.toml`, and a syntax error in that file must not stop it (#191).
#[test]
fn print_editor_config_survives_a_malformed_filters_toml() {
    let dir = fixture("bad_filters_print");
    let home = dir.join("config");
    std::fs::create_dir_all(home.join("recon")).expect("create config dir");
    std::fs::write(home.join("recon/filters.toml"), "this is not toml =\n")
        .expect("write filters.toml");

    let out = recon(&home, &["--print-editor-config", "vscode"], b"");

    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    assert!(!out.stdout.is_empty(), "nothing was printed");
}
