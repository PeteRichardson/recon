//! Every load error in one run (#630): an error in one set file, one pack
//! alias or one `[keymap]` line does not hide the next one. recon still
//! refuses to run while any of them is there.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A config home with `config` as its `config.toml`, and a filter path
/// directory with each of `packs` as `(file name, text)`, under `target/`,
/// named after the test so parallel tests never share one.
fn fixture(name: &str, config: &str, packs: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures-load-errors")
        .join(name);
    fs::remove_dir_all(&dir).ok();
    let home = dir.join("home");
    fs::create_dir_all(home.join("recon")).expect("create config dir");
    fs::write(home.join("recon/config.toml"), config).expect("write config.toml");
    let dir = dir.join("packs");
    fs::create_dir_all(&dir).expect("create filter path dir");
    for (file, text) in packs {
        fs::write(dir.join(file), text).expect("write pack");
    }
    (home, dir)
}

/// A clean environment, for the reason `tests/print_commands.rs` gives.
fn recon(home: &Path, packs: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_recon"))
        .args(args)
        .env_clear()
        .envs(
            ["PATH", "HOME"]
                .into_iter()
                .filter_map(|name| std::env::var_os(name).map(|value| (name, value))),
        )
        .env("XDG_CONFIG_HOME", home)
        .env("RECON_FILTER_PATH", packs)
        .output()
        .expect("run recon")
}

/// `out` refused to run, and its stderr.
fn refused(out: &Output) -> String {
    assert!(
        !out.status.success(),
        "recon ran: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn two_broken_set_files_show_both_errors() {
    let (home, packs) = fixture(
        "two_set_files",
        "",
        &[
            ("a.filters.toml", "[sets.a\n"),
            ("b.filters.toml", "unknown = 1\n"),
        ],
    );
    let err = refused(&recon(&home, &packs, &["--batch", "."]));
    assert!(err.contains("a.filters.toml"), "{err}");
    assert!(err.contains("b.filters.toml"), "{err}");
}

#[test]
fn a_bad_keymap_line_and_a_bad_pack_alias_show_together() {
    let (home, packs) = fixture(
        "keymap_and_alias",
        "[keymap]\n'global.hepl' = 'F1'\n",
        &[("team.filters.toml", "[aliases]\n'a:b' = \"unlist code\"\n")],
    );
    let err = refused(&recon(&home, &packs, &["--batch", "."]));
    assert!(err.contains("global.hepl"), "{err}");
    assert!(err.contains("`a:b`"), "{err}");
}

#[test]
fn print_startup_shows_every_error_and_what_loaded() {
    let (home, packs) = fixture(
        "print_startup",
        "[keymap]\n'global.hepl' = 'F1'\n",
        &[
            ("a.filters.toml", "[sets.a\n"),
            ("team.filters.toml", "[aliases]\ndlogs = \"unlist code\"\n"),
        ],
    );
    let out = recon(&home, &packs, &["--print-startup"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let errors = stdout
        .split_once("\n[errors]\n")
        .unwrap_or_else(|| panic!("no [errors] section: {stdout}"))
        .1;
    assert!(errors.contains("a.filters.toml"), "{stdout}");
    assert!(errors.contains("global.hepl"), "{stdout}");
    let aliases = stdout.split_once("[aliases]\n").expect("aliases section").1;
    assert!(
        aliases.lines().any(|line| line.starts_with("dlogs ")),
        "{stdout}"
    );
}
