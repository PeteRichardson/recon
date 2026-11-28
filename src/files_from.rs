//! `--files-from` in the TUI (#519): the listed files, as a directory of
//! links.

use crate::config::Config;
use crate::stdin::{State, lock};
use color_eyre::eyre::eyre;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Between the folders of a path-like name: `a/log.txt` is `a__log.txt`.
const SEPARATOR: &str = "__";

/// Read the `--files-from` list — stdin for `-`, else the file it names —
/// into a directory of links in `$TMPDIR`, and point `config` at that
/// directory. `main` calls it after [`Config::load`], for the TUI: batch
/// mode reads the list itself.
///
/// # Errors
/// The list cannot be opened, the directory cannot be made, or the OS
/// refuses the read thread.
pub fn list_into(config: &mut Config) -> color_eyre::Result<Option<LinkDir>> {
    let Some(list) = config.files_from.clone() else {
        return Ok(None);
    };
    let delete = config.delete_stdin_temp_file();
    let reader: Box<dyn Read + Send> = if list == crate::config::STDIN_PATH {
        Box::new(io::stdin())
    } else {
        Box::new(
            std::fs::File::open(&list)
                .map_err(|err| eyre!("--files-from {list}: {}", crate::stdin::reason(&err)))?,
        )
    };
    let links = LinkDir::start(&std::env::temp_dir(), io::BufReader::new(reader), delete)?;
    log::debug!("files-from: links in {}", links.path().display());
    config.list_files_into(links.path());
    Ok(Some(links))
}

/// The directory of links, and how the read of the list stands.
#[derive(Debug)]
pub struct LinkDir {
    path: PathBuf,
    shared: Arc<Mutex<Shared>>,
    /// `[stdin] delete_temp_file`: remove the directory when this is
    /// dropped.
    delete: bool,
}

/// What the read thread tells the app.
#[derive(Debug, Default)]
struct Shared {
    names: Names,
    /// Whether anything happened since the last take.
    changed: bool,
    renamed: Vec<(OsString, OsString)>,
    /// Kept until the list ends, for the one warning then.
    skipped: Vec<PathBuf>,
    state: State,
}

/// What the list did since the last [`LinkDir::take`].
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Changes {
    /// Every link name, in the order of the list.
    pub(crate) order: Vec<OsString>,
    /// Links renamed, as `(old, new)`, in the order it was done.
    pub(crate) renamed: Vec<(OsString, OsString)>,
    /// Listed paths that do not exist, so have no link: all of them, once
    /// the list ends, and none before.
    pub(crate) skipped: Vec<PathBuf>,
    /// Whether more of the list can come.
    pub(crate) state: State,
}

impl LinkDir {
    /// Make the directory in `parent`, and read `list` into it on a thread
    /// of its own. With `delete`, the directory is removed when this is
    /// dropped.
    ///
    /// # Errors
    /// The directory cannot be made, or the OS refuses the thread.
    pub(crate) fn start(
        parent: &Path,
        list: impl BufRead + Send + 'static,
        delete: bool,
    ) -> io::Result<Self> {
        let path = create(parent)?;
        let shared = Arc::<Mutex<Shared>>::default();
        let reading = Arc::clone(&shared);
        let dir = path.clone();
        std::thread::Builder::new()
            .name("recon-files-from".to_string())
            .spawn(move || read(list, &dir, &reading))?;
        Ok(Self {
            path,
            shared,
            delete,
        })
    }

    /// The directory of links.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// What changed since the last call, or `None` for nothing.
    pub(crate) fn take(&self) -> Option<Changes> {
        let mut shared = lock(&self.shared);
        if !std::mem::take(&mut shared.changed) {
            return None;
        }
        Some(Changes {
            order: shared.names.given.clone(),
            renamed: std::mem::take(&mut shared.renamed),
            skipped: if shared.state == State::Open {
                Vec::new()
            } else {
                std::mem::take(&mut shared.skipped)
            },
            state: shared.state.clone(),
        })
    }
}

impl Drop for LinkDir {
    fn drop(&mut self) {
        // The links only: `remove_dir_all` does not follow a link.
        if self.delete
            && let Err(err) = std::fs::remove_dir_all(&self.path)
        {
            log::warn!("files-from: cannot delete {}: {err}", self.path.display());
        }
    }
}

/// Make `recon-files-<YYYYMMDD-HHMMSS>-<pid>` in `parent`, for its owner
/// only, as the file stdin is copied into is.
fn create(parent: &Path) -> io::Result<PathBuf> {
    // Absolute, as the explorer holds the folder it lists.
    let path = crate::path::lexical_absolute(parent).join(crate::stdin::temp_name("files"));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&path)?;
    Ok(path)
}

/// Read `list` until it ends, and link each path it names into `dir` as it
/// comes.
fn read(mut list: impl BufRead, dir: &Path, shared: &Mutex<Shared>) {
    let mut line = Vec::new();
    let state = loop {
        line.clear();
        match list.read_until(b'\n', &mut line) {
            Ok(0) => break State::Closed,
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => break State::Stopped(err.to_string()),
        }
        let Some(path) = crate::batch::listed_path(&line) else {
            continue;
        };
        add(&path, dir, shared);
    };
    let mut shared = lock(shared);
    shared.state = state;
    shared.changed = true;
}

/// Link `path` into `dir`, renaming the links its name collides with. A
/// path that does not exist is skipped.
fn add(path: &Path, dir: &Path, shared: &Mutex<Shared>) {
    if std::fs::symlink_metadata(path).is_err() {
        log::warn!("files-from: no such file: {}", path.display());
        let mut shared = lock(shared);
        shared.skipped.push(path.to_path_buf());
        shared.changed = true;
        return;
    }
    let Some(named) = lock(shared).names.add(path) else {
        return;
    };
    // All the old names go before any new one is made: a new name can be
    // another link's old one.
    let targets: Vec<_> = named
        .renamed
        .iter()
        .map(|(old, new)| {
            let old = dir.join(old);
            let target = std::fs::read_link(&old);
            if let Err(err) = std::fs::remove_file(&old) {
                log::warn!("files-from: cannot rename {}: {err}", old.display());
            }
            (target, new)
        })
        .collect();
    for (target, new) in targets {
        match target {
            Ok(target) => link(&target, &dir.join(new)),
            Err(err) => log::warn!(
                "files-from: cannot read the link for {}: {err}",
                Path::new(new).display()
            ),
        }
    }
    link(path, &dir.join(&named.added));
    let mut shared = lock(shared);
    shared.renamed.extend(named.renamed);
    shared.changed = true;
}

/// Make the link `at`, to `target`. A failure is logged: the file is then
/// not listed, as a path that does not exist is not.
fn link(target: &Path, at: &Path) {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, at);
    #[cfg(not(unix))]
    let made: io::Result<()> = Err(io::ErrorKind::Unsupported.into());
    if let Err(err) = made {
        log::warn!("files-from: cannot link {}: {err}", at.display());
    }
}

/// A name change that one new path makes.
#[derive(Debug, PartialEq, Eq)]
struct Named {
    /// Links whose names change, as `(old, new)`.
    renamed: Vec<(OsString, OsString)>,
    /// The name of the new link.
    added: OsString,
}

/// The link name of each listed path.
///
/// A name that only one path has is the file's own name. The paths that
/// share a name each get the fewest of their last folders that make them
/// different: `a/log.txt` and `b/log.txt` are `a__log.txt` and
/// `b__log.txt`. A name that is still taken, by another path's name, gets
/// `~2`, `~3`, … after it.
#[derive(Debug, Default)]
struct Names {
    /// Every path added, in the order it came.
    paths: Vec<PathBuf>,
    /// The name of each path, by index into `paths`.
    given: Vec<OsString>,
    /// The paths that share each file name, by index into `paths`.
    by_base: HashMap<OsString, Vec<usize>>,
    /// Which path has each name.
    taken: HashMap<OsString, usize>,
}

impl Names {
    /// Give `path` a name, and say which names change. `None` for a path
    /// that was listed before.
    fn add(&mut self, path: &Path) -> Option<Named> {
        let base = path.file_name().unwrap_or(path.as_os_str()).to_owned();
        let group = self.by_base.entry(base).or_default();
        // A path listed before is in the group of its own file name.
        if group.iter().any(|&member| self.paths[member] == path) {
            return None;
        }
        let index = self.paths.len();
        group.push(index);
        self.paths.push(path.to_path_buf());
        self.given.push(OsString::new());
        let group = group.clone();
        for &member in &group {
            self.taken.remove(&self.given[member]);
        }
        let mut renamed = Vec::new();
        for (member, name) in group.iter().zip(self.group_names(&group)) {
            let name = self.free(name);
            self.taken.insert(name.clone(), *member);
            let old = std::mem::replace(&mut self.given[*member], name.clone());
            if *member != index && old != name {
                renamed.push((old, name));
            }
        }
        Some(Named {
            renamed,
            added: self.given[index].clone(),
        })
    }

    /// The names of `group`, paths with one file name: the fewest last
    /// folders that tell them apart, the same number for each.
    fn group_names(&self, group: &[usize]) -> Vec<OsString> {
        let parts: Vec<Vec<&OsStr>> = group
            .iter()
            .map(|&member| normal_parts(&self.paths[member]))
            .collect();
        let most = parts.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let names = |count: usize| -> Vec<OsString> {
            parts.iter().map(|parts| join_last(parts, count)).collect()
        };
        (1..most)
            .map(names)
            .find(|names| names.iter().collect::<HashSet<_>>().len() == names.len())
            .unwrap_or_else(|| names(most))
    }

    /// `name`, or the first of `name~2`, `name~3`, … that no path has.
    fn free(&self, name: OsString) -> OsString {
        if !self.taken.contains_key(&name) {
            return name;
        }
        let mut n = 2;
        loop {
            let mut candidate = name.clone();
            candidate.push(format!("~{n}"));
            if !self.taken.contains_key(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }
}

/// The folder and file names of `path`, without its root.
fn normal_parts(path: &Path) -> Vec<&OsStr> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect()
}

/// The last `count` of `parts`, joined by [`SEPARATOR`].
fn join_last(parts: &[&OsStr], count: usize) -> OsString {
    let mut name = OsString::new();
    for (at, part) in parts[parts.len().saturating_sub(count)..]
        .iter()
        .enumerate()
    {
        if at > 0 {
            name.push(SEPARATOR);
        }
        name.push(part);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fixture_dir;

    fn named(renamed: &[(&str, &str)], added: &str) -> Named {
        Named {
            renamed: renamed
                .iter()
                .map(|&(old, new)| (old.into(), new.into()))
                .collect(),
            added: added.into(),
        }
    }

    /// Take changes until the list ends, or fail after 10 s.
    fn until_closed(links: &LinkDir) -> Vec<Changes> {
        let start = std::time::Instant::now();
        let mut all = Vec::new();
        loop {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "timed out"
            );
            if let Some(changes) = links.take() {
                let closed = changes.state != State::Open;
                all.push(changes);
                if closed {
                    return all;
                }
            }
            std::thread::yield_now();
        }
    }

    fn link_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("read the links")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// Each listed file gets a link to it; a path that does not exist gets
    /// none, and is reported as skipped. The order is the list's.
    #[cfg(unix)]
    #[test]
    fn each_listed_file_gets_a_link_and_a_missing_one_is_skipped() {
        let files = crate::path::lexical_absolute(&fixture_dir("files_from_links"));
        for name in ["a", "b"] {
            std::fs::create_dir(files.join(name)).expect("mkdir");
            std::fs::write(files.join(name).join("log.txt"), name).expect("write");
        }
        std::fs::write(files.join("notes.md"), "notes").expect("write");
        let missing = files.join("gone.txt");
        let list = format!(
            "{0}/notes.md\n{0}/a/log.txt\n{1}\n{0}/b/log.txt\n",
            files.display(),
            missing.display()
        );

        let links = LinkDir::start(
            &fixture_dir("files_from_links_tmp"),
            io::Cursor::new(list),
            false,
        )
        .expect("started");
        let all = until_closed(&links);

        let last = all.last().expect("a change");
        assert_eq!(last.state, State::Closed);
        assert_eq!(last.order, ["notes.md", "a__log.txt", "b__log.txt"]);
        let skipped: Vec<_> = all.iter().flat_map(|c| c.skipped.clone()).collect();
        assert_eq!(skipped, [missing]);
        assert_eq!(
            link_names(links.path()),
            ["a__log.txt", "b__log.txt", "notes.md"]
        );
        let target = std::fs::read_link(links.path().join("a__log.txt")).expect("a link");
        assert_eq!(target, files.join("a/log.txt"));
    }

    #[test]
    fn delete_removes_the_directory_on_drop_and_the_default_keeps_it() {
        let kept = LinkDir::start(&fixture_dir("files_from_keep"), io::Cursor::new(""), false)
            .expect("started");
        let kept_path = kept.path().to_path_buf();
        drop(kept);
        assert!(kept_path.is_dir(), "left for the OS by default");

        let deleted = LinkDir::start(&fixture_dir("files_from_delete"), io::Cursor::new(""), true)
            .expect("started");
        let deleted_path = deleted.path().to_path_buf();
        drop(deleted);
        assert!(!deleted_path.exists());
    }

    /// A unique name stays short. When a later path has the same name,
    /// both get path-like names, and the earlier link is renamed.
    #[test]
    fn a_name_that_collides_becomes_path_like_and_renames_the_earlier_link() {
        let mut names = Names::default();

        assert_eq!(
            names.add(Path::new("/x/a/log.txt")),
            Some(named(&[], "log.txt"))
        );
        assert_eq!(
            names.add(Path::new("/x/notes.md")),
            Some(named(&[], "notes.md"))
        );
        assert_eq!(
            names.add(Path::new("/x/b/log.txt")),
            Some(named(&[("log.txt", "a__log.txt")], "b__log.txt"))
        );
    }
}
