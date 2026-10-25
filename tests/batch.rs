//! Batch mode (#143) end to end: the built binary, a piped stdin, real
//! files and a real `filters.toml`. `src/batch.rs` tests the pieces;
//! this is the one place `main`'s batch decision, the flags, the
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
        .join("target/test-fixtures-batch")
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
/// at `home`. Stdin is a pipe even when empty, which is what puts the run
/// in batch mode.
fn recon(home: &Path, args: &[&str], stdin: &[u8]) -> Output {
    recon_with(home, args, stdin, &[])
}

/// `recon`, with `env` set as well.
fn recon_with(home: &Path, args: &[&str], stdin: &[u8], env: &[(&str, &Path)]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_recon"))
        .args(args)
        // A clean environment, not two variables removed (#367): an exported
        // `RECON_WARNINGS` or `RECON_FILTER_PATH` changed what these runs
        // print. `PATH` and `HOME` are all a run needs.
        .env_clear()
        .envs(
            ["PATH", "HOME"]
                .into_iter()
                .filter_map(|name| std::env::var_os(name).map(|value| (name, value))),
        )
        .env("XDG_CONFIG_HOME", home)
        .envs(env.iter().copied())
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

/// #511: batch mode always hides, so `--emit files` lists only the matching
/// inputs, not every readable one — and `--hide` changes nothing. The list
/// comes from `--files-from -` (#517).
#[test]
fn files_lists_the_matching_inputs_from_stdin_with_or_without_hide() {
    let dir = fixture("files_from_stdin");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    let b = dir.join("b.log");
    fs::write(&a, "hit\n").expect("write");
    fs::write(&b, "miss\n").expect("write");
    let list = format!("{}\n{}\n", a.display(), b.display());

    for args in [
        &[
            "--emit",
            "files",
            "--files-from",
            "-",
            "--set",
            "Bugs:only_hit",
        ][..],
        &[
            "--emit",
            "files",
            "--files-from",
            "-",
            "--set",
            "Bugs:only_hit",
            "--hide",
        ][..],
    ] {
        let out = recon(&home, args, list.as_bytes());

        assert_eq!(text(&out.stdout), format!("{}\n", a.display()), "{args:?}");
        assert_eq!(
            text(&out.stderr),
            "recon: emitted 1 files of 2 inputs, hide mode\n",
            "{args:?}"
        );
        assert_eq!(out.status.code(), Some(0));
    }
}

/// #511: batch mode always hides, so `--emit lines` without `--hide` prints
/// only the matches — and `--hide`, still accepted, changes nothing.
#[test]
fn lines_print_only_the_matches_with_or_without_hide() {
    let dir = fixture("lines_no_hide");
    let home = config_home(&dir);
    let log = dir.join("server.log");
    fs::write(&log, "hit\nmiss\nhit again\n").expect("write");
    let path = log.to_str().expect("utf-8 path");

    for args in [
        &["--emit", "lines", "--set", "Bugs:only_hit", path][..],
        &["--emit", "lines", "--set", "Bugs:only_hit", "--hide", path][..],
    ] {
        let out = recon(&home, args, b"");

        assert_eq!(text(&out.stdout), "hit\nhit again\n", "{args:?}");
        assert_eq!(
            text(&out.stderr),
            "recon: emitted 2 lines of server.log, hide mode\n",
            "{args:?}"
        );
        assert_eq!(out.status.code(), Some(0));
    }
}

/// #517: `-` is the input itself, as for `grep`: its lines are filtered,
/// not read as a list of files.
#[test]
fn a_dash_path_filters_the_lines_on_stdin() {
    let dir = fixture("dash_lines");
    let home = config_home(&dir);

    let out = recon(
        &home,
        &["-b", "--set", "Bugs:only_hit", "-"],
        b"hit\nmiss\nhit again\n",
    );

    assert_eq!(text(&out.stdout), "hit\nhit again\n");
    assert_eq!(
        text(&out.stderr),
        "recon: emitted 2 lines of <stdin>, hide mode\n"
    );
    assert_eq!(out.status.code(), Some(0));
}

/// #517: a matching line on stdin is written as soon as it is read, while
/// the pipe is still open, so `log stream | recon -b -` is filtered as it
/// comes and not at an end that never comes.
#[test]
fn a_dash_path_writes_each_match_before_stdin_closes() {
    use std::io::{BufRead as _, BufReader};
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = fixture("dash_streams");
    let home = config_home(&dir);
    let mut child = Command::new(env!("CARGO_BIN_EXE_recon"))
        .args(["-b", "--set", "Bugs:only_hit", "-"])
        .env_clear()
        .envs(
            ["PATH", "HOME"]
                .into_iter()
                .filter_map(|name| std::env::var_os(name).map(|value| (name, value))),
        )
        .env("XDG_CONFIG_HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn recon");
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let (lines, got) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            lines.send(line.expect("utf-8 line")).ok();
        }
    });

    stdin.write_all(b"miss\nhit\n").expect("write stdin");
    stdin.flush().expect("flush stdin");
    // Generous: a pass is milliseconds, and only a buffered run waits it out.
    let first = got.recv_timeout(Duration::from_secs(10));
    drop(stdin);
    let status = child.wait().expect("wait for recon");

    assert_eq!(first.as_deref(), Ok("hit"), "written before stdin closed");
    assert_eq!(status.code(), Some(0));
}

/// #517: `--emit files -` was the way to give a list before `-` meant
/// lines. Refused, with the new way in the message, so a script that used
/// it fails loudly instead of printing nothing.
#[test]
fn emit_files_over_a_dash_path_points_to_files_from() {
    let dir = fixture("dash_files");
    let home = config_home(&dir);

    let out = recon(&home, &["--emit", "files", "-"], b"/some/file.log\n");

    assert!(out.stdout.is_empty(), "stdout: {}", text(&out.stdout));
    assert!(
        text(&out.stderr).contains("for a list of files, use --files-from -"),
        "stderr: {}",
        text(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1));
}

/// #224: a definition filter is answered by parsing each file, so
/// `--emit files` lists only the files that define the kinds asked for,
/// and the summary does not say `no filter`.
#[test]
fn files_hide_with_a_definitions_profile_lists_the_files_that_define_it() {
    let dir = fixture("files_definitions");
    let home = dir.join("config");
    fs::create_dir_all(home.join("recon")).expect("create config dir");
    fs::write(
        home.join("recon/filters.toml"),
        "[sets.definitions]\n\n\
         [sets.definitions.profiles]\n\
         typedefs = [\"structs\"]\n",
    )
    .expect("write filters.toml");
    let a = dir.join("a.rs");
    let b = dir.join("b.rs");
    let c = dir.join("c.log");
    fs::write(&a, "struct Point;\n").expect("write");
    fs::write(&b, "fn main() {}\n").expect("write");
    fs::write(&c, "struct Point;\n").expect("write");
    let list = format!("{}\n{}\n{}\n", a.display(), b.display(), c.display());

    let out = recon(
        &home,
        &[
            "--emit",
            "files",
            "--files-from",
            "-",
            "--set",
            "definitions:typedefs",
        ],
        list.as_bytes(),
    );

    assert_eq!(text(&out.stdout), format!("{}\n", a.display()));
    assert_eq!(
        text(&out.stderr),
        "recon: emitted 1 files of 3 inputs, hide mode\n"
    );
    assert_eq!(out.status.code(), Some(0));
}

/// #441: `--set NAME:LIST` turns on the union of a profile and a named
/// filter, with no profile for the combination.
#[test]
fn set_with_a_list_emits_the_union_of_a_profile_and_a_filter() {
    let dir = fixture("set_list_union");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "hit\nother\nmiss\n").expect("write");

    let out = recon(
        &home,
        &[
            "--emit",
            "lines",
            "--files-from",
            "-",
            "-q",
            "--set",
            "Bugs:only_hit,other",
        ],
        format!("{}\n", a.display()).as_bytes(),
    );

    assert_eq!(text(&out.stdout), "hit\nother\n");
    assert_eq!(text(&out.stderr), "");
    assert_eq!(out.status.code(), Some(0));
}

/// #538: `--set +NAME:ITEMS` changes what an earlier `--set` gave the
/// set; without the `+`, the last `--set` replaces it.
#[test]
fn set_with_plus_changes_what_an_earlier_set_gave() {
    let dir = fixture("set_plus_keeps");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "hit\nother\nmiss\n").expect("write");
    let stdin = format!("{}\n", a.display());

    for (second, expected) in [("+Bugs:other", "hit\nother\n"), ("Bugs:other", "other\n")] {
        let out = recon(
            &home,
            &[
                "--emit",
                "lines",
                "--files-from",
                "-",
                "-q",
                "--set",
                "Bugs:only_hit",
                "--set",
                second,
            ],
            stdin.as_bytes(),
        );

        assert_eq!(text(&out.stdout), expected, "{second}");
        assert_eq!(text(&out.stderr), "", "{second}");
        assert_eq!(out.status.code(), Some(0), "{second}");
    }
}

/// #441: a kind of the built-in set is named alone, with no profile in
/// any file.
#[test]
fn set_names_a_definition_kind_without_a_profile() {
    let dir = fixture("set_list_kind");
    let home = dir.join("config");
    fs::create_dir_all(&home).expect("create config dir");
    let a = dir.join("a.rs");
    let b = dir.join("b.rs");
    fs::write(&a, "struct Point;\n").expect("write");
    fs::write(&b, "fn main() {}\n").expect("write");

    let out = recon(
        &home,
        &[
            "--emit",
            "files",
            "--files-from",
            "-",
            "-q",
            "--set",
            "definitions:structs",
        ],
        format!("{}\n{}\n", a.display(), b.display()).as_bytes(),
    );

    assert_eq!(text(&out.stdout), format!("{}\n", a.display()));
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
        &[
            "--emit",
            "lines",
            "--files-from",
            "-",
            "-n",
            "--set",
            "Bugs:only_hit",
        ],
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
fn a_path_file_with_empty_stdin_is_read_in_batch_mode_and_quiet_drops_the_summary() {
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
        "no filter: the whole file"
    );
    assert!(out.stderr.is_empty(), "stderr: {}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0));
}

/// #353: an explicit `PATH` wins over a pipe on stdin, so the list a shell
/// loop reads from is left for the loop: `while read f; do recon --emit
/// lines "$f"; done < list`.
#[test]
fn a_path_argument_leaves_the_list_on_stdin_unread() {
    let dir = fixture("path_wins");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    let b = dir.join("b.log");
    fs::write(&a, "from a\n").expect("write");
    fs::write(&b, "from b\n").expect("write");
    let list = format!("{}\n", b.display());

    let out = recon(
        &home,
        &["--emit", "lines", "-q", a.to_str().expect("utf-8 path")],
        list.as_bytes(),
    );

    assert_eq!(text(&out.stdout), "from a\n");
    assert_eq!(out.status.code(), Some(0));
}

/// #353: `--emit cwd` needs no list, so a list on stdin is not read: with
/// no `PATH`, the answer is recon's own directory, not the list's.
#[test]
fn emit_cwd_leaves_the_list_on_stdin_unread() {
    let dir = fixture("cwd_ignores_stdin");
    let home = config_home(&dir);
    let list = format!("{}\n", dir.join("a.log").display());

    let out = recon(&home, &["--emit", "cwd", "-q"], list.as_bytes());

    let cwd = std::env::current_dir().expect("cwd");
    assert_eq!(text(&out.stdout), format!("{}\n", cwd.display()));
    assert_eq!(out.status.code(), Some(0));
}

/// #439: `-b` with no `--emit` is a batch run that emits lines. Before it,
/// this run had no `--emit`, so it tried to open the TUI.
#[test]
fn batch_without_emit_emits_the_matching_lines() {
    let dir = fixture("batch_flag");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "hit\nmiss\nhit again\n").expect("write");

    let out = recon(
        &home,
        &[
            "-b",
            "--set",
            "Bugs:only_hit",
            a.to_str().expect("utf-8 path"),
        ],
        b"",
    );

    assert_eq!(text(&out.stdout), "hit\nhit again\n");
    assert_eq!(
        text(&out.stderr),
        "recon: emitted 2 lines of a.log, hide mode\n"
    );
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
        &[
            "--emit",
            "lines",
            "--files-from",
            "-",
            "--set",
            "Bugs:only_hit",
        ],
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

    assert_eq!(out.stdout, b"");
    assert!(
        text(&out.stderr).contains("unknown set \"Nope\"; the set files define: Bugs"),
        "stderr: {}",
        text(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1));
}

/// `--unlist` takes a set out of the filters that decide the output (#283).
/// `Bugs` here autoloads with `hit` on, so without the flag only `hit` is
/// emitted; with it, no filter is on and every line is emitted.
#[test]
fn unlist_removes_an_autoload_set_from_the_output() {
    let dir = fixture("unlist");
    let home = config_home(&dir);
    fs::write(
        home.join("recon/filters.toml"),
        "[sets.Bugs]\nautoload = true\n\n\
         [sets.Bugs.profiles]\n\
         default = [\"hit\"]\n\n\
         [[sets.Bugs.filters]]\n\
         pattern = \"hit\"\n",
    )
    .expect("write filters.toml");
    let log = dir.join("a.log");
    fs::write(&log, "hit\nmiss\n").expect("write");
    let path = log.display().to_string();

    let out = recon(&home, &["--emit", "lines", &path], b"");
    assert_eq!(text(&out.stdout), "hit\n");

    let out = recon(&home, &["--emit", "lines", "--unlist", "Bugs", &path], b"");
    assert_eq!(text(&out.stdout), "hit\nmiss\n");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn set_and_unlist_of_one_set_is_refused_before_anything_is_read() {
    let dir = fixture("set_and_unlist");
    let home = config_home(&dir);

    let out = recon(
        &home,
        &[
            "--emit",
            "files",
            "--set",
            "Bugs:only_hit",
            "--unlist",
            "Bugs",
        ],
        b"",
    );

    assert_eq!(out.stdout, b"");
    assert!(
        text(&out.stderr).contains("--set and --unlist both name the set \"Bugs\""),
        "stderr: {}",
        text(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1));
}

/// A `[keymap]` line naming an action that does not exist refuses to start,
/// before any terminal setup, exactly as an unknown `--set` does (#61).
///
/// The one place the whole path runs together: the file is found, the stanza
/// parsed, the table resolved, and the refusal reaches stderr instead of a
/// screen that is about to be replaced by the alternate one. `App::new`
/// cannot do any of that — it returns `Self`.
#[test]
fn an_unknown_action_in_the_keymap_is_refused_before_anything_is_read() {
    let dir = fixture("unknown_action");
    let home = config_home(&dir);
    fs::write(
        home.join("recon/config.toml"),
        "[keymap]\n'global.qiut' = 'q'\n",
    )
    .expect("write config.toml");

    let out = recon(&home, &["--emit", "files"], b"");

    assert_eq!(out.stdout, b"");
    assert!(
        text(&out.stderr).contains("unknown action \"global.qiut\" in [keymap]"),
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

/// `--print-keymap defaults` prints recon's built-in table, which no
/// `config.toml` can change — so no `config.toml` may stop it printing. A
/// `[keymap]` recon refuses is precisely when a user needs this command, and
/// being refused by the file it diagnoses is #191's shape moved onto
/// `config.toml`.
///
/// Plain `--print-keymap` is asserted to still fail on the same file, which is
/// the boundary: it prints the map **in force**, and this file leaves none.
#[test]
fn print_keymap_defaults_survives_a_keymap_recon_refuses() {
    let dir = fixture("bad_keymap_print_defaults");
    let home = config_home(&dir);
    fs::write(
        home.join("recon/config.toml"),
        "[keymap]\n'global.quit' = 'x'\n'global.reload' = 'x'\n",
    )
    .expect("write config.toml");

    let out = recon(&home, &["--print-keymap", "defaults"], b"");

    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    let printed = text(&out.stdout);
    assert!(printed.contains("[keymap]"), "{printed}");
    assert!(
        printed.contains("'global.quit' = 'q'"),
        "the built-in table is what must be printed: {printed}"
    );

    let in_force = recon(&home, &["--print-keymap"], b"");
    assert_eq!(
        in_force.status.code(),
        Some(1),
        "there is no map in force to print, so this one must still refuse"
    );
}

/// Batch mode has no warning panel, so the keymap warnings that would have
/// filled one must reach stderr instead.
///
/// They did not: `Displaced` warnings were collected onto the `Config` and
/// handed to `App::new`, which the batch path never reaches, while the
/// reserved-key warning was logged inside `keymap::config::build` and so came through.
/// Two classes of keymap warning, one switch governing both, and one of them
/// vanishing is what made it a defect rather than a choice.
#[test]
fn batch_puts_keymap_warnings_on_stderr() {
    let dir = fixture("batch_keymap_warnings");
    let home = config_home(&dir);
    // 'j' is explorer.down, view.down and filters.down by default, so one line
    // costs three panes their key and raises three warnings.
    fs::write(
        home.join("recon/config.toml"),
        "[keymap]\n'global.quit' = 'j'\n",
    )
    .expect("write config.toml");
    let log = dir.join("a.log");
    fs::write(&log, "hit\n").expect("write");
    let list = format!("{}\n", log.display());

    let out = recon(
        &home,
        &["--emit", "files", "--files-from", "-"],
        list.as_bytes(),
    );

    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    let stderr = text(&out.stderr);
    for loser in ["explorer.down", "view.down", "filters.down"] {
        assert!(
            stderr.contains(loser),
            "{loser}'s warning never reached stderr: {stderr}"
        );
    }
    assert!(
        stderr.contains("takes 'j' from"),
        "the warning text itself must be there: {stderr}"
    );

    // The summary still goes out, after the warnings rather than instead of
    // them.
    assert!(
        stderr.contains("recon: emitted"),
        "the summary must survive: {stderr}"
    );
}

/// The same switch that hides the panel hides these, since they are the same
/// warnings by another route.
#[test]
fn batch_keymap_warnings_obey_no_warnings() {
    let dir = fixture("batch_keymap_warnings_off");
    let home = config_home(&dir);
    fs::write(
        home.join("recon/config.toml"),
        "[keymap]\n'global.quit' = 'j'\n",
    )
    .expect("write config.toml");
    let log = dir.join("a.log");
    fs::write(&log, "hit\n").expect("write");
    let list = format!("{}\n", log.display());

    let out = recon(
        &home,
        &["--emit", "files", "--files-from", "-", "--no-warnings"],
        list.as_bytes(),
    );

    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    let stderr = text(&out.stderr);
    assert!(
        !stderr.contains("takes 'j' from"),
        "--no-warnings must silence these too: {stderr}"
    );
    assert!(
        stderr.contains("recon: emitted"),
        "but not the summary, which is a different switch: {stderr}"
    );
}

/// `--print-keymap` prints a pasteable `[keymap]` stanza to stdout and exits,
/// with nothing on stderr.
#[test]
fn print_keymap_emits_a_pasteable_stanza() {
    let dir = fixture("print-keymap");
    let home = config_home(&dir);

    let out = recon(&home, &["--print-keymap"], b"");

    assert!(out.status.success());
    assert!(
        out.stderr.is_empty(),
        "nothing belongs on stderr: {}",
        text(&out.stderr)
    );
    let printed = String::from_utf8(out.stdout).expect("utf-8");
    assert!(printed.contains("[keymap]"), "{printed}");
    assert!(printed.contains("global.quit"), "{printed}");
}

/// `--print-keymap` writes the keymap warnings to stderr, the same warnings a
/// normal start shows in its panel (#259), and leaves the stanza on stdout
/// exactly as it was.
///
/// It did not: the command returned before the code that logs them, so the
/// panel's "recon --print-keymap prints them all" sent the user to a command
/// that printed none of them.
#[test]
fn print_keymap_puts_keymap_warnings_on_stderr() {
    let dir = fixture("print_keymap_warnings");
    let home = config_home(&dir);
    fs::write(
        home.join("recon/config.toml"),
        "[keymap]\n'global.quit' = 'j'\n",
    )
    .expect("write config.toml");

    let out = recon(&home, &["--print-keymap"], b"");

    assert_eq!(out.status.code(), Some(0), "stderr: {}", text(&out.stderr));
    let stderr = text(&out.stderr);
    for loser in ["explorer.down", "view.down", "filters.down"] {
        assert!(
            stderr.contains(loser),
            "{loser}'s warning never reached stderr: {stderr}"
        );
    }
    let printed = text(&out.stdout);
    assert!(printed.contains("[keymap]"), "{printed}");
    assert!(
        !printed.contains("takes 'j' from"),
        "a warning leaked onto stdout: {printed}"
    );

    let quiet = recon(&home, &["--print-keymap", "--no-warnings"], b"");
    assert_eq!(quiet.status.code(), Some(0));
    assert!(
        quiet.stderr.is_empty(),
        "--no-warnings must silence them: {}",
        text(&quiet.stderr)
    );
    assert_eq!(
        quiet.stdout, out.stdout,
        "the warnings must not change what stdout carries"
    );
}

/// A config home for pattern references (#473): `org:Summary`, and a
/// `team` set that refers to it — once correctly and twice not. The set
/// `spare` has a broken reference too, and is never turned on.
fn reference_home(dir: &Path) -> PathBuf {
    let home = dir.join("config");
    fs::create_dir_all(home.join("recon")).expect("create config dir");
    fs::write(
        home.join("recon/filters.toml"),
        "[sets.org]\n\
         [[sets.org.filters]]\n\
         name = 'Summary'\n\
         pattern = '^SUMMARY'\n\n\
         [sets.team]\n\
         [[sets.team.filters]]\n\
         name = 'u'\n\
         pattern = '${org:Summary}.*Updates'\n\
         [[sets.team.filters]]\n\
         name = 'bad'\n\
         pattern = '${org:Nope}'\n\
         [[sets.team.filters]]\n\
         name = 'worse'\n\
         pattern = '${gone:X}'\n\n\
         [sets.spare]\n\
         [[sets.spare.filters]]\n\
         name = 's'\n\
         pattern = '${org:Nope}'\n",
    )
    .expect("write filters.toml");
    home
}

#[test]
fn a_reference_uses_the_target_s_pattern_and_a_disabled_broken_one_is_ignored() {
    let dir = fixture("reference_expands");
    let home = reference_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "SUMMARY: Updates\nSUMMARY only\nUpdates\n").expect("write");

    let out = recon(
        &home,
        &[
            "--emit",
            "lines",
            "--files-from",
            "-",
            "-q",
            "--set",
            "team:u",
        ],
        format!("{}\n", a.display()).as_bytes(),
    );

    assert_eq!(text(&out.stdout), "SUMMARY: Updates\n");
    assert_eq!(text(&out.stderr), "");
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn a_broken_reference_that_takes_effect_stops_the_run_before_any_output() {
    let dir = fixture("reference_broken");
    let home = reference_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "SUMMARY: Updates\n").expect("write");

    let out = recon(
        &home,
        &["--emit", "lines", "--set", "team:u,bad,worse"],
        format!("{}\n", a.display()).as_bytes(),
    );

    assert_eq!(out.stdout, b"");
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("filter team:bad has a broken reference: org:Nope not found"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("filter team:worse has a broken reference: gone:X not found"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("spare"), "stderr: {stderr}");
    assert_eq!(out.status.code(), Some(1));
}

/// #490: with `RECON_LOG` and `RUST_LOG=recon=debug`, the startup snapshot
/// goes to the file, each record with a timestamp and its module, and
/// stderr has what it has without them.
#[test]
fn recon_log_gets_the_startup_snapshot_with_timestamps_and_modules() {
    let dir = fixture("recon_log_snapshot");
    let home = config_home(&dir);
    let a = dir.join("a.log");
    fs::write(&a, "hit\n").expect("write");
    let log = dir.join("recon.log");

    let out = recon_with(
        &home,
        &["--emit", "lines", "--files-from", "-", "-q"],
        format!("{}\n", a.display()).as_bytes(),
        &[("RECON_LOG", &log), ("RUST_LOG", Path::new("recon=debug"))],
    );

    assert_eq!(text(&out.stderr), "");
    assert_eq!(out.status.code(), Some(0));
    let written = fs::read_to_string(&log).expect("the log file");
    for needle in [
        "mode: batch: --emit with no terminal on stdin",
        "filter set \"Bugs\": listed",
        "config read in",
        "filter sets read in",
        "environment: RECON_LOG=",
    ] {
        assert!(written.contains(needle), "no {needle:?} in:\n{written}");
    }
    let snapshot = written
        .lines()
        .find(|line| line.contains("mode: batch"))
        .expect("the mode record");
    assert!(
        snapshot.starts_with("[20") && snapshot.contains("DEBUG recon"),
        "no timestamp or module: {snapshot}"
    );
}

/// #493: `--follow` prints each new matching line at once, through a pipe,
/// and SIGINT ends the run with the summary and exit code 0.
#[cfg(unix)]
#[test]
fn follow_prints_new_matching_lines_until_sigint() {
    use std::io::BufRead as _;
    let dir = fixture("follow");
    let home = config_home(&dir);
    let log = dir.join("app.log");
    fs::write(&log, "hit one\nmiss\n").expect("write");
    let mut child = Command::new(env!("CARGO_BIN_EXE_recon"))
        .args(["--batch", "--follow", "--set", "Bugs:only_hit", "-n"])
        .arg(&log)
        .env_clear()
        .envs(
            ["PATH", "HOME"]
                .into_iter()
                .filter_map(|name| std::env::var_os(name).map(|value| (name, value))),
        )
        .env("XDG_CONFIG_HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn recon");
    // A run that never ends fails here, not by hanging the suite.
    let pid = child.id().to_string();
    let watchdog = {
        let pid = pid.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(20));
            let _ = Command::new("kill").args(["-KILL", &pid]).status();
        })
    };
    let mut stdout = std::io::BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut line = String::new();
    stdout.read_line(&mut line).expect("first line");
    assert_eq!(line, "1\thit one\n");

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .expect("open");
    file.write_all(b"miss again\nhit two\n").expect("append");
    line.clear();
    stdout.read_line(&mut line).expect("the new line");
    assert_eq!(line, "4\thit two\n");

    Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("send SIGINT");
    let out = child.wait_with_output().expect("wait for recon");
    drop(watchdog);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        text(&out.stderr),
        "recon: emitted 2 lines of app.log, hide mode\n"
    );
}

#[test]
fn follow_refuses_a_directory() {
    let dir = fixture("follow_dir");
    let home = config_home(&dir);
    fs::write(dir.join("a.log"), "hit\n").expect("write");
    let out = recon(
        &home,
        &["--batch", "--follow", dir.to_str().expect("utf-8")],
        b"",
    );
    assert_ne!(out.status.code(), Some(0));
    assert!(
        text(&out.stderr).contains("--follow needs one file, not a directory"),
        "{}",
        text(&out.stderr)
    );
}
