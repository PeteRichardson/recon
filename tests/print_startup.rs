//! `--print-startup` (#628): which files recon read at startup, which it
//! skipped and why, and the file each alias, command, row and key comes
//! from, with what it hides or what hides it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A config home with `config` as its `config.toml`, and a filter path
/// directory with each of `packs` as `(file name, text)`, under `target/`,
/// named after the test so parallel tests never share one.
fn fixture(name: &str, config: &str, packs: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-fixtures-print-startup")
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
    recon_with(home, packs, args, &[])
}

/// [`recon`], with `env` added.
fn recon_with(home: &Path, packs: &Path, args: &[&str], env: &[(&str, &Path)]) -> Output {
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
        .envs(env.iter().copied())
        .output()
        .expect("run recon")
}

/// `path` as the report shows it: the home directory as `~`.
fn shown(path: &Path) -> String {
    let home = std::env::var("HOME").expect("HOME");
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The one line of `section` in `out` that starts with `start`, its
/// columns split on runs of two or more spaces. `section` is a `[table]`
/// line, or `None` for the lines above the first one.
fn row<'a>(out: &'a str, section: Option<&str>, start: &str) -> Vec<&'a str> {
    let mut lines = out.lines();
    if let Some(section) = section {
        lines.by_ref().find(|line| *line == section);
    }
    let lines: Vec<&str> = lines
        .take_while(|line| !line.starts_with('['))
        .filter(|line| line.starts_with(&format!("{start} ")))
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "one `{start}` row in {section:?} of:\n{out}"
    );
    lines[0]
        .split("  ")
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .collect()
}

const CONFIG: &str = "\
[aliases]\n\
dlogs = \"unlist code\"\n\
[commands]\n\
jq = [\"jq\", \".\"]\n\
[[filetype]]\n\
glob = \"*.json\"\n\
views = [\"jq\", \"raw\"]\n\
[keymap]\n\
dlogs = 'F6'\n";

const DEPLOY: &str = "\
[aliases]\n\
dlogs = \"list code\"\n\
dtail = \"unlist code\"\n\
[commands]\n\
show = [\"cat\", \"{file}\"]\n\
[[filetype]]\n\
glob = \"*.log\"\n\
views = [\"show\", \"raw\"]\n";

#[test]
fn the_report_names_the_source_of_each_definition_and_what_it_hides() {
    let (home, packs) = fixture(
        "sources",
        CONFIG,
        &[
            ("deploylogs.filters.toml", DEPLOY),
            ("old.filters.toml", "[aliases\n"),
            (".ignore", "old.filters.toml\n"),
        ],
    );
    let config = &shown(&home.join("recon/config.toml"));
    let deploy = &shown(&packs.join("deploylogs.filters.toml"));
    let out = stdout(&recon(&home, &packs, &["--print-startup"]));

    assert_eq!(row(&out, None, "config file"), ["config file", config]);
    assert_eq!(
        row(&out, None, "RECON_FILTER_PATH"),
        ["RECON_FILTER_PATH", packs.to_str().expect("utf-8")]
    );
    assert!(out.contains(&format!(" {deploy}\n")), "{out}");
    let old = packs.join("old.filters.toml");
    assert_eq!(
        row(&out, None, "skipped"),
        [
            "skipped",
            &shown(&old),
            "not selected by .include or .ignore"
        ]
    );

    assert_eq!(
        row(&out, Some("[aliases]"), "dlogs"),
        ["dlogs", config, "hides `deploylogs:dlogs`"]
    );
    assert_eq!(
        row(&out, Some("[aliases]"), "deploylogs:dlogs"),
        [
            "deploylogs:dlogs",
            deploy,
            &format!("loses `dlogs` to {config}")
        ]
    );
    assert_eq!(row(&out, Some("[aliases]"), "dtail"), ["dtail", deploy]);
    assert_eq!(row(&out, Some("[commands]"), "jq"), ["jq", config]);
    assert_eq!(row(&out, Some("[commands]"), "show"), ["show", deploy]);
    assert_eq!(
        row(&out, Some("[[filetype]]"), "*.json"),
        ["*.json", config, "jq, raw"]
    );
    assert_eq!(
        row(&out, Some("[[filetype]]"), "*.log"),
        ["*.log", deploy, "show, raw"]
    );
    assert_eq!(
        row(&out, Some("[keymap]"), "dlogs"),
        ["dlogs", "F6", config]
    );
}

/// Without `all`, the report lists only what the files add or change; with
/// it, the built-in views and every key too, each marked `recon`.
#[test]
fn all_adds_the_built_in_views_and_keys() {
    let (home, packs) = fixture("all", "", &[]);
    let changes = stdout(&recon(&home, &packs, &["--print-startup"]));
    assert!(!changes.contains("global.quit"), "{changes}");
    assert!(!changes.contains("built-in view"), "{changes}");

    let all = stdout(&recon(&home, &packs, &["--print-startup", "all"]));
    assert_eq!(
        row(&all, Some("[commands]"), "raw"),
        ["raw", "recon", "built-in view"]
    );
    assert_eq!(
        row(&all, Some("[keymap]"), "global.quit"),
        ["global.quit", "q", "recon"]
    );
}

/// Every record of the load has the one target `recon::config` (#628), so
/// `RUST_LOG=recon::config=debug` logs how recon built its configuration
/// and nothing else.
#[test]
fn one_log_target_selects_the_load_records() {
    let (home, packs) = fixture("log_target", CONFIG, &[("deploylogs.filters.toml", DEPLOY)]);
    let log = home.join("recon.log");
    stdout(&recon_with(
        &home,
        &packs,
        &["--print-startup"],
        &[
            ("RECON_LOG", &log),
            ("RUST_LOG", Path::new("recon::config=debug")),
        ],
    ));
    let written = fs::read_to_string(&log).expect("the log file");
    for needle in [
        "config read in",
        "reading filter sets from",
        "filter sets read in",
        "no filter sets file at",
    ] {
        assert!(written.contains(needle), "no {needle:?} in:\n{written}");
    }
    for line in written.lines().filter(|line| line.starts_with("[20")) {
        assert!(line.contains(" recon::config]"), "another target: {line}");
    }
}
