//! `-c` / `--config` (#537): read a config file other than `config.toml`.
//! The file replaces `config.toml`; it is not a layer on top of it. Each
//! test's config home holds a `config.toml` that is not valid TOML, so a run
//! that reads it fails and a run that reads the named file instead does not.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A config home with a broken `config.toml` and, beside the home, a
/// `try.toml` holding `text`, under `target/`, named after the test so
/// parallel tests never share one.
fn fixture(name: &str, text: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures-config-flag")
        .join(name);
    fs::remove_dir_all(&dir).ok();
    let home = dir.join("home");
    fs::create_dir_all(home.join("recon")).expect("create config dir");
    fs::write(home.join("recon/config.toml"), "[keymap\n").expect("write config.toml");
    let file = dir.join("try.toml");
    fs::write(&file, text).expect("write try.toml");
    (home, file)
}

/// A clean environment, for the reason `tests/print_commands.rs` gives.
fn recon(home: &Path, envs: &[(&str, &Path)], args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_recon"))
        .args(args)
        .env_clear()
        .envs(
            ["PATH", "HOME"]
                .into_iter()
                .filter_map(|name| std::env::var_os(name).map(|value| (name, value))),
        )
        .env("XDG_CONFIG_HOME", home)
        .envs(envs.iter().copied())
        .output()
        .expect("run recon")
}

const HELP_ON_F1: &str = "[keymap]\n'global.help' = 'F1'\n";

fn stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn the_named_file_replaces_config_toml() {
    let (home, file) = fixture("replaces", HELP_ON_F1);
    for flag in ["-c", "--config"] {
        let out = recon(
            &home,
            &[],
            &[flag, file.to_str().unwrap(), "--print-keymap"],
        );
        assert!(stdout(&out).contains("'global.help' = 'F1'"), "{flag}");
    }
}

/// The user named the file, so a file that is not there is a mistake, not
/// the "no config yet" that a missing `config.toml` is.
#[test]
fn a_missing_named_file_refuses_to_start() {
    let (home, file) = fixture("missing", HELP_ON_F1);
    let missing = file.with_file_name("nope.toml");
    let out = recon(
        &home,
        &[],
        &["-c", missing.to_str().unwrap(), "--print-keymap"],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("nope.toml"), "{stderr}");
}

#[test]
fn recon_config_names_the_file_too() {
    let (home, file) = fixture("env", HELP_ON_F1);
    let out = recon(&home, &[("RECON_CONFIG", &file)], &["--print-keymap"]);
    assert!(stdout(&out).contains("'global.help' = 'F1'"));
}

/// One file replaces `config.toml`; two would need the merge rules that a
/// replacement does without.
#[test]
fn a_second_config_flag_is_refused() {
    let (home, file) = fixture("twice", HELP_ON_F1);
    let path = file.to_str().unwrap();
    let out = recon(&home, &[], &["-c", path, "-c", path, "--print-keymap"]);
    assert!(!out.status.success());
}

/// An error in the named file names that file, not `config.toml`.
#[test]
fn a_keymap_error_names_the_named_file() {
    let (home, file) = fixture("keymap_error", "[keymap]\n'global.hepl' = 'F1'\n");
    let out = recon(
        &home,
        &[],
        &["-c", file.to_str().unwrap(), "--print-keymap"],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("try.toml"), "{stderr}");
}

/// A shell expands `~` in `-c ~/try.toml`, but not in a variable or a quoted
/// argument; recon reads `~` as each path setting does (#505).
#[test]
fn a_leading_tilde_is_home() {
    let (home, file) = fixture("tilde", HELP_ON_F1);
    let dir = file.parent().unwrap();
    let out = recon(
        &home,
        &[("HOME", dir), ("RECON_CONFIG", Path::new("~/try.toml"))],
        &["--print-keymap"],
    );
    assert!(stdout(&out).contains("'global.help' = 'F1'"));
}

/// The log's startup snapshot names the file this run read (#490), so a
/// bug report shows the `-c` file and not a `config.toml` that was skipped.
#[test]
fn the_log_names_the_named_file() {
    let (home, file) = fixture("log", HELP_ON_F1);
    let log = file.with_file_name("recon.log");
    let out = recon(
        &home,
        &[("RECON_LOG", &log), ("RUST_LOG", Path::new("recon=debug"))],
        &["-c", file.to_str().unwrap(), "-b", file.to_str().unwrap()],
    );
    stdout(&out);
    let text = fs::read_to_string(&log).expect("read the log");
    let expected = format!("config file: {}", file.display());
    assert!(text.contains(&expected), "{text}");
}

/// An `[aliases]` entry the palette cannot run stops recon at startup, not
/// when the alias runs (#557), and the message names the alias.
#[test]
fn an_alias_with_an_unknown_command_refuses_to_start() {
    let (home, file) = fixture("alias", "[aliases]\nproj = \"e ~ ; frobnicate\"\n");
    let out = recon(
        &home,
        &[],
        &["-c", file.to_str().unwrap(), "--print-keymap"],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("[aliases] `proj`"), "{stderr}");
    assert!(stderr.contains("`frobnicate`"), "{stderr}");
}

/// A `[keymap]` line can give a user alias a key (#559): `--print-keymap`
/// prints it, and a line for an alias that does not exist stops recon at
/// startup with its name.
#[test]
fn a_keymap_line_binds_an_alias_and_an_unknown_one_refuses_to_start() {
    let (home, file) = fixture(
        "alias_key",
        "[aliases]\nproj = \"e ~ ; explorer.focus\"\n[keymap]\nproj = 'F6'\n",
    );
    let out = recon(
        &home,
        &[],
        &["-c", file.to_str().unwrap(), "--print-keymap"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\n'proj' = 'F6'\n"), "{stdout}");

    let (home, file) = fixture(
        "alias_key_unknown",
        "[aliases]\nproj = \"e ~\"\n[keymap]\nprji = 'F6'\n",
    );
    let out = recon(
        &home,
        &[],
        &["-c", file.to_str().unwrap(), "--print-keymap"],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("\"prji\""), "{stderr}");
    assert!(stderr.contains("did you mean \"proj\""), "{stderr}");
}
