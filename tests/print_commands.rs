//! The two print commands against a `config.toml` recon cannot read (#366).
//! Neither reads anything from that file, so neither may be stopped by it:
//! these are the commands a user reaches for when recon refuses the file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A config home whose `config.toml` is not valid TOML, under `target/`,
/// named after the test so parallel tests never share one.
fn broken_config_home(name: &str) -> PathBuf {
    let home = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures-print")
        .join(name);
    fs::remove_dir_all(&home).ok();
    fs::create_dir_all(home.join("recon")).expect("create config dir");
    fs::write(home.join("recon/config.toml"), "[keymap\n").expect("write config.toml");
    home
}

fn recon(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_recon"))
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
        .output()
        .expect("run recon")
}

#[test]
fn print_keymap_defaults_survives_a_broken_config_file() {
    let home = broken_config_home("keymap_defaults");
    let out = recon(&home, &["--print-keymap", "defaults"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("[keymap]"));
}

#[test]
fn print_editor_config_survives_a_broken_config_file() {
    let home = broken_config_home("editor_config");
    let out = recon(&home, &["--print-editor-config", "vscode"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("[editor]"));
}

/// The map in force still needs the file, so a broken one still refuses it.
#[test]
fn print_keymap_in_force_still_refuses_a_broken_config_file() {
    let home = broken_config_home("keymap_in_force");
    let out = recon(&home, &["--print-keymap"]);
    assert!(!out.status.success());
}
