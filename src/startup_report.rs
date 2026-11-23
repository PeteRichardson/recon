//! `--print-startup` (#628): how recon built its configuration.
//!
//! A pack collision (ADR 0018) keeps the definition that lost as
//! `pack:name`, and the warning says so once, in a panel. This report shows
//! the whole result in one place: the files read and skipped, the file each
//! definition comes from, and what it hides or what hides it.

use crate::filtersets::Skipped;
use crate::keymap::Keymap;
use crate::views::pack::Merged;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthStr;

/// What the report is made from: start-up's answers, up to the keymap.
pub(crate) struct Report<'a> {
    /// `config.toml`: its path, or the bare name when recon has none.
    pub config_file: &'a Path,
    /// The directories of the filter path, in order.
    pub filter_dirs: &'a [PathBuf],
    /// The `RECON_*` variables, by name.
    pub env: BTreeMap<String, String>,
    /// The set files read, and those skipped.
    pub read: &'a [PathBuf],
    pub skipped: &'a [Skipped],
    pub merged: &'a Merged,
    pub bindings: &'a Keymap,
    /// Every start-up warning, in the order the panel shows them.
    pub warnings: &'a [String],
    /// The load errors (#630). The section is there only when there are
    /// some.
    pub errors: &'a [String],
    /// `--print-startup all`: the built-in views and every key too.
    pub all: bool,
    /// `HOME`, which the report shows as `~`.
    pub home: Option<&'a str>,
}

impl Report<'_> {
    /// A path as the report shows it: the home directory as `~`, so a row
    /// stays short enough to read.
    fn shown(&self, path: &Path) -> String {
        match self.home.and_then(|home| path.strip_prefix(home).ok()) {
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        }
    }
}

/// The report's text.
#[must_use]
pub(crate) fn print(report: &Report<'_>) -> String {
    let mut out = String::from("# recon startup: where each definition comes from\n");
    let config = report.shown(report.config_file);
    let mut head = vec![vec!["config file".to_string(), config.clone()]];
    if report.env.is_empty() {
        head.push(vec!["RECON_*".into(), "(none set)".into()]);
    }
    for (name, value) in &report.env {
        head.push(vec![name.clone(), value.clone()]);
    }
    list(
        &mut head,
        "filter path",
        report.filter_dirs.iter().map(|d| vec![report.shown(d)]),
    );
    list(
        &mut head,
        "read",
        report.read.iter().map(|f| vec![report.shown(f)]),
    );
    list(
        &mut head,
        "skipped",
        report
            .skipped
            .iter()
            .map(|skip| vec![report.shown(&skip.path), skip.reason.to_string()]),
    );
    table(&mut out, &head);

    let sources = &report.merged.sources;
    section(&mut out, "[aliases]", &named(report, &sources.aliases));
    let mut commands = named(report, &sources.commands);
    if report.all {
        commands.extend(
            crate::views::BUILT_IN
                .iter()
                .map(|view| vec![(*view).to_string(), "recon".into(), "built-in view".into()]),
        );
    }
    section(&mut out, "[commands]", &commands);

    let rows: Vec<Vec<String>> = report
        .merged
        .rows
        .iter()
        .zip(&sources.rows)
        .map(|(row, from)| {
            vec![
                row.label().to_string(),
                from.iter()
                    .map(|f| report.shown(f))
                    .collect::<Vec<_>>()
                    .join(", "),
                row.views
                    .as_ref()
                    .map_or_else(|| "(no views)".to_string(), |views| views.join(", ")),
            ]
        })
        .collect();
    section(&mut out, "[[filetype]]", &rows);

    let keys: Vec<Vec<String>> =
        crate::keymap::key_rows(report.bindings, &Keymap::default(), report.all)
            .into_iter()
            .map(|row| {
                let mut columns = vec![
                    row.name,
                    if row.keys.is_empty() {
                        "(no key)".into()
                    } else {
                        row.keys.join(", ")
                    },
                    if row.yours {
                        config.clone()
                    } else {
                        "recon".into()
                    },
                ];
                if !row.lost.is_empty() {
                    columns.push(row.lost.join(", "));
                }
                columns
            })
            .collect();
    section(&mut out, "[keymap]", &keys);

    if !report.errors.is_empty() {
        out.push_str("\n[errors]\n");
        for error in report.errors {
            bullet(&mut out, error);
        }
    }
    out.push_str("\n[warnings]\n");
    if report.warnings.is_empty() {
        out.push_str("(none)\n");
    }
    for warning in report.warnings {
        bullet(&mut out, warning);
    }
    out
}

/// `text` as a `- ` item, each line after its first indented under it: a
/// TOML error is a snippet of several lines.
fn bullet(out: &mut String, text: &str) {
    let mut lines = text.lines();
    let _ = writeln!(out, "- {}", lines.next().unwrap_or_default());
    for line in lines {
        let _ = writeln!(out, "  {line}");
    }
}

/// `items` as rows of a two-column group under `label`: the label on the
/// first row only, or one `(none)` row when there are no items.
fn list(head: &mut Vec<Vec<String>>, label: &str, items: impl Iterator<Item = Vec<String>>) {
    let start = head.len();
    for item in items {
        let label = if head.len() == start { label } else { "" };
        head.push(std::iter::once(label.to_string()).chain(item).collect());
    }
    if head.len() == start {
        head.push(vec![label.to_string(), "(none)".into()]);
    }
}

/// One row per key of `sources`, a `[commands]` or `[aliases]` table: its
/// file, then what it hides or what hides it.
///
/// A `pack:name` key lost `name` to the file that holds the bare name, and
/// that file's row names each `pack:name` it hides.
fn named(report: &Report<'_>, sources: &BTreeMap<String, PathBuf>) -> Vec<Vec<String>> {
    sources
        .iter()
        .map(|(key, from)| {
            let mut columns = vec![key.clone(), report.shown(from)];
            if let Some((_, bare)) = key.split_once(':') {
                if let Some(winner) = sources.get(bare) {
                    columns.push(format!("loses `{bare}` to {}", report.shown(winner)));
                }
            } else {
                let hidden: Vec<String> = sources
                    .keys()
                    .filter(|other| other.split_once(':').is_some_and(|(_, b)| b == key))
                    .map(|other| format!("`{other}`"))
                    .collect();
                if !hidden.is_empty() {
                    columns.push(format!("hides {}", hidden.join(", ")));
                }
            }
            columns
        })
        .collect()
}

/// `title`, then `rows` as a table, or `(none)`.
fn section(out: &mut String, title: &str, rows: &[Vec<String>]) {
    let _ = write!(out, "\n{title}\n");
    if rows.is_empty() {
        out.push_str("(none)\n");
    }
    table(out, rows);
}

/// `rows` with each column but the last padded to its widest cell, and two
/// spaces between columns, so a reader or a script can split on them.
fn table(out: &mut String, rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if index + 1 == row.len() {
                continue;
            }
            if widths.len() <= index {
                widths.resize(index + 1, 0);
            }
            widths[index] = widths[index].max(cell.width());
        }
    }
    for row in rows {
        let mut line = String::new();
        for (index, cell) in row.iter().enumerate() {
            if index + 1 == row.len() {
                line.push_str(cell);
            } else {
                // By display width, not `char`s: `{:<w$}` would pad a
                // wide character as one column.
                line.push_str(cell);
                line.push_str(&" ".repeat(widths[index] - cell.width() + 2));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
}
