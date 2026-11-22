//! A set file's `[aliases]` (#627): the aliases of a pack join the ones of
//! `config.toml`, and a `[keymap]` line can give one a key.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A config home with `config` as its `config.toml`, and a filter path
/// directory with `pack` as its `deploylogs.filters.toml`, under `target/`,
/// named after the test so parallel tests never share one.
fn fixture(name: &str, config: &str, pack: &str) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures-pack-aliases")
        .join(name);
    fs::remove_dir_all(&dir).ok();
    let home = dir.join("home");
    fs::create_dir_all(home.join("recon")).expect("create config dir");
    fs::write(home.join("recon/config.toml"), config).expect("write config.toml");
    let packs = dir.join("packs");
    fs::create_dir_all(&packs).expect("create filter path dir");
    fs::write(packs.join("deploylogs.filters.toml"), pack).expect("write pack");
    (home, packs)
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

fn stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const DLOGS: &str = "[aliases]\ndlogs = \"e $1 ; unlist code\"\n";

#[test]
fn a_keymap_line_binds_an_alias_of_a_set_file() {
    let (home, packs) = fixture("keymap_binds", "[keymap]\ndlogs = 'F6'\n", DLOGS);
    let out = stdout(&recon(&home, &packs, &["--print-keymap"]));
    assert!(out.contains("\n'dlogs' = 'F6'\n"), "{out}");
}

/// The map in force depends on the set files now (#627), so a set file
/// recon cannot read stops plain `--print-keymap`. The two print commands
/// that read no file still run (#191, #366).
#[test]
fn a_broken_set_file_stops_only_the_print_command_that_reads_it() {
    let (home, packs) = fixture("broken_pack", "", "[aliases\n");
    let in_force = recon(&home, &packs, &["--print-keymap"]);
    assert!(!in_force.status.success());
    let stderr = String::from_utf8_lossy(&in_force.stderr);
    assert!(stderr.contains("deploylogs.filters.toml"), "{stderr}");

    assert!(stdout(&recon(&home, &packs, &["--print-keymap", "defaults"])).contains("[keymap]"));
    assert!(
        stdout(&recon(&home, &packs, &["--print-editor-config", "vscode"])).contains("[editor]")
    );
}

/// `pack:name` names a pack alias that lost its bare name, so a set file
/// that gives an alias a name with a `:` refuses to start, and the message
/// names the file.
#[test]
fn a_pack_alias_with_a_colon_in_its_name_refuses_to_start() {
    let (home, packs) = fixture(
        "colon_name",
        "",
        "[aliases]\n'team:dlogs' = \"unlist code\"\n",
    );
    let out = recon(&home, &packs, &["--print-keymap"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("`team:dlogs`"), "{stderr}");
    assert!(stderr.contains("deploylogs.filters.toml"), "{stderr}");
}
