//! Packs: the command views, `[[filetype]]` rows and palette aliases of a
//! set file (#602, #627).
//!
//! A set file on the filter path can carry what recon must know about one
//! tool: its `[sets]`, its `[commands]`, its `[[filetype]]` rows and its
//! `[aliases]`. The file is the unit of control, so a command, a row or an
//! alias has no listed state: reading the file brings them into effect.

use super::FileType;
use super::command::Commands;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The commands and rows of one set file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    /// The file's stem: `mpegpack.filters.toml` gives `mpegpack`. A view
    /// name `mpegpack:beautiful` always means this pack's `beautiful`.
    pub name: String,
    /// The file, for messages.
    pub path: PathBuf,
    pub commands: Commands,
    /// In file order.
    pub filetypes: Vec<FileType>,
    /// The `[aliases]` of the file (#627): each name, and the chain it runs.
    pub aliases: BTreeMap<String, String>,
}

/// The rows and commands the view table is built from and the palette's
/// aliases, once the packs are in, and a warning for each definition a
/// collision renamed or dropped.
#[derive(Debug, Default)]
pub struct Merged {
    pub rows: Vec<FileType>,
    pub commands: Commands,
    pub aliases: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

/// `config.toml`'s rows, commands and aliases with `packs` added, packs in
/// precedence order.
///
/// A command or an alias keeps its bare name unless an earlier definition
/// holds it: `config.toml` first, then the packs in order. One that loses
/// is kept as `pack:name`, with a warning. Every view name in a row is then
/// rewritten to the key it resolves to, so the table needs no pack logic.
///
/// # Errors
///
/// A row that names a view that is not a built-in view or a command, and a
/// pack alias the palette cannot run.
pub fn merge(
    rows: &[FileType],
    commands: &Commands,
    aliases: &BTreeMap<String, String>,
    packs: &[Pack],
) -> Result<Merged, Error> {
    let mut merged = Merged {
        rows: Vec::with_capacity(rows.len()),
        commands: commands.clone(),
        aliases: aliases.clone(),
        warnings: Vec::new(),
    };
    // `pack:view` must name one pack, so of two packs with one name (two
    // directories that each hold `filters.toml`) the first wins.
    let mut kept: Vec<&Pack> = Vec::with_capacity(packs.len());
    for pack in packs {
        if kept.iter().any(|earlier| earlier.name == pack.name) {
            merged.warnings.push(format!(
                "{} is ignored: an earlier file is the pack `{}`",
                pack.path.display(),
                pack.name
            ));
        } else {
            kept.push(pack);
        }
    }
    let packs = kept;
    for pack in &packs {
        for (name, command) in &pack.commands.0 {
            let key = claim(
                &merged.commands.0,
                "[commands]",
                pack,
                name,
                &mut merged.warnings,
            );
            merged.commands.0.insert(key, command.clone());
        }
    }
    merge_aliases(&mut merged, &packs)?;

    for row in rows {
        let row = resolve(row, None, &packs, &merged.commands)?;
        merged.rows.push(row);
    }
    let mut seen: Vec<FileType> = Vec::new();
    for pack in &packs {
        for row in &pack.filetypes {
            let row = resolve(row, Some(pack), &packs, &merged.commands)?;
            if seen.iter().any(|earlier| same_criteria(earlier, &row)) {
                merged.warnings.push(format!(
                    "the [[filetype]] row for `{}` in {} is ignored: an earlier file has a row for the same files",
                    row.label(),
                    pack.path.display()
                ));
                continue;
            }
            seen.push(row.clone());
            match merged.rows[..rows.len()]
                .iter_mut()
                .find(|user| same_criteria(user, &row))
            {
                Some(user) => add_to(user, &row),
                None => merged.rows.push(row),
            }
        }
    }
    Ok(merged)
}

/// The key `pack`'s `name` gets in `taken`, the `[commands]` or `[aliases]`
/// of the earlier files: its bare name, or `pack:name` with a warning when
/// an earlier file holds the bare name.
fn claim<T>(
    taken: &BTreeMap<String, T>,
    table: &str,
    pack: &Pack,
    name: &str,
    warnings: &mut Vec<String>,
) -> String {
    if !taken.contains_key(name) {
        return name.to_string();
    }
    let key = format!("{}:{name}", pack.name);
    warnings.push(format!(
        "{table} `{name}` in {} is named `{key}`: an earlier file defines `{name}`",
        pack.path.display()
    ));
    key
}

/// Add the aliases of `packs` to `merged`, which holds `config.toml`'s.
/// Each pack alias is checked as `config.toml` checks its own (#557), by
/// its bare name: a chain cannot run another alias, from any file.
fn merge_aliases(merged: &mut Merged, packs: &[&Pack]) -> Result<(), Error> {
    let mut every = merged.aliases.clone();
    for pack in packs {
        for (name, chain) in &pack.aliases {
            every.entry(name.clone()).or_insert_with(|| chain.clone());
        }
    }
    for pack in packs {
        for (name, chain) in &pack.aliases {
            crate::commands::check_alias(name, chain, &every).map_err(|problem| {
                Error::BadAlias {
                    path: pack.path.clone(),
                    name: name.clone(),
                    problem,
                }
            })?;
            let key = claim(
                &merged.aliases,
                "[aliases]",
                pack,
                name,
                &mut merged.warnings,
            );
            merged.aliases.insert(key, chain.clone());
        }
    }
    Ok(())
}

/// `row`, from `pack` or from `config.toml`, with each view name rewritten
/// to the key of the command it means.
fn resolve(
    row: &FileType,
    pack: Option<&Pack>,
    packs: &[&Pack],
    commands: &Commands,
) -> Result<FileType, Error> {
    let views = row
        .views
        .iter()
        .flatten()
        .map(|name| {
            view(name, pack, packs, commands).ok_or_else(|| Error::UnknownView {
                path: pack.map(|pack| pack.path.clone()),
                row: row.label().to_string(),
                name: name.clone(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FileType {
        views: row.views.as_ref().map(|_| views),
        ..row.clone()
    })
}

/// The key a view name means: `pack:name` that pack's command, a bare name
/// the row's own pack's command first, then the first definition. A pack's
/// command has the key `pack:name` when it lost its bare name to an earlier
/// file, and its bare name when not.
fn view(name: &str, pack: Option<&Pack>, packs: &[&Pack], commands: &Commands) -> Option<String> {
    let key_of = |pack: &Pack, command: &str| {
        pack.commands.0.contains_key(command).then(|| {
            let key = format!("{}:{command}", pack.name);
            if commands.0.contains_key(&key) {
                key
            } else {
                command.to_string()
            }
        })
    };
    if let Some((pack_name, command)) = name.split_once(':') {
        let pack = packs.iter().find(|pack| pack.name == pack_name)?;
        return key_of(pack, command);
    }
    if let Some(key) = pack.and_then(|pack| key_of(pack, name)) {
        return Some(key);
    }
    (super::BUILT_IN.contains(&name) || commands.0.contains_key(name)).then(|| name.to_string())
}

/// Whether two rows match the same files: equal `glob` and equal `mime`,
/// either of which may be absent.
fn same_criteria(a: &FileType, b: &FileType) -> bool {
    a.glob == b.glob && a.mime == b.mime
}

/// Add a pack row to the user's row of the same criteria. The pack's views
/// go after the user's own views and before the built-in views that end
/// the chain, which stay last; a view already in the chain stays where it
/// is. The pack's hand-off fills a row that has none, and never replaces
/// one.
fn add_to(user: &mut FileType, row: &FileType) {
    let is_built_in = |name: &String| super::BUILT_IN.contains(&name.as_str());
    if let Some(pack_views) = &row.views {
        let chain = user.views.get_or_insert_with(Vec::new);
        let mut at = chain.len()
            - chain
                .iter()
                .rev()
                .take_while(|name| is_built_in(name))
                .count();
        for name in pack_views {
            if chain.contains(name) {
                continue;
            }
            if is_built_in(name) {
                chain.push(name.clone());
            } else {
                chain.insert(at, name.clone());
                at += 1;
            }
        }
    }
    if user.handoff.is_none() {
        user.handoff.clone_from(&row.handoff);
    }
}

/// Why the packs cannot be merged.
#[derive(Debug)]
pub enum Error {
    /// A row names a view that no file defines.
    UnknownView {
        /// The pack file, or `None` for `config.toml`.
        path: Option<PathBuf>,
        /// The row's label.
        row: String,
        name: String,
    },
    /// A pack's `[aliases]` entry the palette cannot run (#627).
    BadAlias {
        path: PathBuf,
        name: String,
        problem: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownView { path, row, name } => {
                write!(f, "the [[filetype]] row for `{row}` ")?;
                if let Some(path) = path {
                    write!(f, "in {} ", path.display())?;
                }
                write!(
                    f,
                    "names view `{name}`, which is not a built-in view or a name in [commands]"
                )
            }
            Self::BadAlias {
                path,
                name,
                problem,
            } => write!(f, "[aliases] `{name}` in {}: {problem}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::command::Command;

    fn pack(name: &str, commands: &[&'static str], rows: Vec<FileType>) -> Pack {
        Pack {
            name: name.to_string(),
            path: PathBuf::from(format!("team/{name}.filters.toml")),
            commands: Commands::of(commands.iter().map(|name| (*name, Command::of(&[name])))),
            filetypes: rows,
            aliases: BTreeMap::new(),
        }
    }

    fn views(row: &FileType) -> Vec<&str> {
        row.views.iter().flatten().map(String::as_str).collect()
    }

    #[test]
    fn a_pack_row_of_new_criteria_goes_after_the_users_rows() {
        let user = [FileType::with_view_names("*.json", &["raw"])];
        let mpeg = pack(
            "mpegpack",
            &["beautiful"],
            vec![FileType::with_view_names("*.mpg", &["beautiful", "hex"])],
        );
        let merged = merge(&user, &Commands::default(), &BTreeMap::new(), &[mpeg]).expect("merges");
        let labels: Vec<&str> = merged.rows.iter().map(FileType::label).collect();
        assert_eq!(labels, ["*.json", "*.mpg"]);
        assert_eq!(views(&merged.rows[1]), ["beautiful", "hex"]);
        assert_eq!(merged.commands.0["beautiful"].argv, ["beautiful"]);
        assert!(merged.warnings.is_empty(), "{:?}", merged.warnings);
    }

    #[test]
    fn a_pack_row_of_equal_criteria_adds_views_before_the_built_in_ones() {
        let user = [
            FileType::with_view_names("*.mpg", &["jq", "raw", "hex"]),
            FileType::with_view_names("*", &["raw"]),
        ];
        let commands = Commands::of([("jq", Command::of(&["jq"]))]);
        let mut row = FileType::with_view_names("*.mpg", &["beautiful", "hex"]);
        row.handoff = Some(vec!["mpv".to_string(), "{file}".to_string()]);
        let mpeg = pack("mpegpack", &["beautiful"], vec![row]);
        let merged = merge(&user, &commands, &BTreeMap::new(), &[mpeg]).expect("merges");
        assert_eq!(merged.rows.len(), 2, "the row is merged, not added");
        assert_eq!(views(&merged.rows[0]), ["jq", "beautiful", "raw", "hex"]);
        assert_eq!(
            merged.rows[0].handoff.as_deref(),
            Some(&["mpv".to_string(), "{file}".to_string()][..]),
            "an empty hand-off takes the pack's"
        );
    }

    #[test]
    fn a_pack_never_replaces_the_users_hand_off() {
        let mut user = FileType::with_view_names("*.mpg", &["raw"]);
        user.handoff = Some(vec!["vlc".to_string()]);
        let mut row = FileType::with_view_names("*.mpg", &["beautiful"]);
        row.handoff = Some(vec!["mpv".to_string()]);
        let mpeg = pack("mpegpack", &["beautiful"], vec![row]);
        let merged =
            merge(&[user], &Commands::default(), &BTreeMap::new(), &[mpeg]).expect("merges");
        assert_eq!(
            merged.rows[0].handoff.as_deref(),
            Some(&["vlc".to_string()][..])
        );
    }

    #[test]
    fn a_losing_command_is_kept_under_its_pack_name_and_its_rows_follow_it() {
        let commands = Commands::of([("dump", Command::of(&["user-dump"]))]);
        let first = pack(
            "first",
            &["dump", "show"],
            vec![FileType::with_view_names("*.a", &["dump", "show"])],
        );
        let second = pack(
            "second",
            &["show"],
            vec![FileType::with_view_names("*.b", &["show"])],
        );
        let merged = merge(&[], &commands, &BTreeMap::new(), &[first, second]).expect("merges");

        assert_eq!(
            merged.commands.0["dump"].argv,
            ["user-dump"],
            "config.toml wins"
        );
        assert_eq!(merged.commands.0["first:dump"].argv, ["dump"]);
        assert_eq!(
            merged.commands.0["show"].argv,
            ["show"],
            "the first pack wins"
        );
        assert_eq!(merged.commands.0["second:show"].argv, ["show"]);
        // A pack's own row means its own command.
        assert_eq!(views(&merged.rows[0]), ["first:dump", "show"]);
        assert_eq!(views(&merged.rows[1]), ["second:show"]);

        assert_eq!(merged.warnings.len(), 2, "{:?}", merged.warnings);
        assert!(
            merged.warnings[0].contains("first:dump"),
            "{:?}",
            merged.warnings
        );
        assert!(
            merged.warnings[0].contains("team/first.filters.toml"),
            "{:?}",
            merged.warnings
        );
        assert!(
            merged.warnings[1].contains("second:show"),
            "{:?}",
            merged.warnings
        );
    }

    #[test]
    fn a_pack_view_name_means_that_packs_view() {
        let first = pack("first", &["show"], vec![]);
        let second = pack(
            "second",
            &["show"],
            vec![FileType::with_view_names("*.b", &["first:show"])],
        );
        let user = [FileType::with_view_names("*.c", &["second:show", "show"])];
        let merged = merge(
            &user,
            &Commands::default(),
            &BTreeMap::new(),
            &[first, second],
        )
        .expect("merges");
        assert_eq!(views(&merged.rows[0]), ["second:show", "show"]);
        assert_eq!(views(&merged.rows[1]), ["show"], "first won the bare name");
    }

    #[test]
    fn an_unknown_view_is_refused_with_its_file_and_row() {
        let mpeg = pack(
            "mpegpack",
            &[],
            vec![FileType::with_view_names("*.mpg", &["nope"])],
        );
        let err = merge(&[], &Commands::default(), &BTreeMap::new(), &[mpeg]).expect_err("refused");
        let err = err.to_string();
        assert!(err.contains("team/mpegpack.filters.toml"), "{err}");
        assert!(err.contains("*.mpg"), "{err}");
        assert!(err.contains("`nope`"), "{err}");
    }

    #[test]
    fn a_users_row_can_name_a_pack_view_only_by_its_pack() {
        let user = [FileType::with_view_names("*.c", &["nope:show"])];
        let err = merge(&user, &Commands::default(), &BTreeMap::new(), &[]).expect_err("refused");
        assert!(err.to_string().contains("`nope:show`"), "{err}");
    }

    #[test]
    fn of_two_pack_rows_with_equal_criteria_the_first_wins() {
        let first = pack(
            "first",
            &["a"],
            vec![FileType::with_view_names("*.x", &["a"])],
        );
        let second = pack(
            "second",
            &["b"],
            vec![FileType::with_view_names("*.x", &["b"])],
        );
        let merged = merge(
            &[],
            &Commands::default(),
            &BTreeMap::new(),
            &[first, second],
        )
        .expect("merges");
        assert_eq!(merged.rows.len(), 1);
        assert_eq!(views(&merged.rows[0]), ["a"]);
        assert_eq!(merged.warnings.len(), 1, "{:?}", merged.warnings);
        assert!(
            merged.warnings[0].contains("team/second.filters.toml"),
            "{:?}",
            merged.warnings
        );
        assert!(merged.warnings[0].contains("*.x"), "{:?}", merged.warnings);
    }

    #[test]
    fn a_pack_view_the_user_names_stays_where_they_wrote_it() {
        let user = [FileType::with_view_names(
            "*.mpg",
            &["mpegpack:info", "jq", "raw"],
        )];
        let commands = Commands::of([("jq", Command::of(&["jq"]))]);
        let mpeg = pack(
            "mpegpack",
            &["beautiful", "info"],
            vec![FileType::with_view_names("*.mpg", &["beautiful", "info"])],
        );
        let merged = merge(&user, &commands, &BTreeMap::new(), &[mpeg]).expect("merges");
        assert_eq!(views(&merged.rows[0]), ["info", "jq", "beautiful", "raw"]);
    }

    /// Two directories can each hold `filters.toml`: `pack:view` could not
    /// tell them apart, so the first one wins.
    #[test]
    fn a_later_pack_of_the_same_name_is_ignored() {
        let first = pack(
            "filters",
            &["show"],
            vec![FileType::with_view_names("*.a", &["show"])],
        );
        let mut second = pack(
            "filters",
            &["show", "more"],
            vec![FileType::with_view_names("*.b", &["more"])],
        );
        second.path = PathBuf::from("other/filters.toml");
        let merged = merge(
            &[],
            &Commands::default(),
            &BTreeMap::new(),
            &[first, second],
        )
        .expect("merges");
        assert_eq!(merged.commands.0.keys().collect::<Vec<_>>(), ["show"]);
        let labels: Vec<&str> = merged.rows.iter().map(FileType::label).collect();
        assert_eq!(labels, ["*.a"]);
        assert_eq!(merged.warnings.len(), 1, "{:?}", merged.warnings);
        assert!(
            merged.warnings[0].contains("other/filters.toml"),
            "{:?}",
            merged.warnings
        );
    }

    /// A pack's `[aliases]` (#627), with the given entries.
    fn with_aliases(mut pack: Pack, aliases: &[(&str, &str)]) -> Pack {
        pack.aliases = aliases
            .iter()
            .map(|(name, chain)| ((*name).to_string(), (*chain).to_string()))
            .collect();
        pack
    }

    #[test]
    fn a_pack_alias_that_config_toml_defines_first_is_kept_as_pack_name() {
        let user = BTreeMap::from([("dlogs".to_string(), "e ~/mine".to_string())]);
        let deploy = with_aliases(
            pack("deploylogs", &[], vec![]),
            &[("dlogs", "e $1"), ("dtail", "unlist code")],
        );
        let merged = merge(&[], &Commands::default(), &user, &[deploy]).expect("merges");
        assert_eq!(merged.aliases["dlogs"], "e ~/mine");
        assert_eq!(merged.aliases["deploylogs:dlogs"], "e $1");
        assert_eq!(merged.aliases["dtail"], "unlist code");
        assert_eq!(
            merged.warnings,
            [
                "[aliases] `dlogs` in team/deploylogs.filters.toml is named `deploylogs:dlogs`: an earlier file defines `dlogs`"
            ]
        );
    }

    #[test]
    fn a_pack_alias_the_palette_cannot_run_is_refused_with_its_file() {
        let deploy = with_aliases(pack("deploylogs", &[], vec![]), &[("dlogs", "frobnicate")]);
        let err = merge(&[], &Commands::default(), &BTreeMap::new(), &[deploy])
            .expect_err("refused")
            .to_string();
        assert_eq!(
            err,
            "[aliases] `dlogs` in team/deploylogs.filters.toml: `frobnicate` is not a command"
        );
    }
}
