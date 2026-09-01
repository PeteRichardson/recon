//! One rule for turning a user-supplied path into an absolute one.
//!
//! recon resolves paths in three places — the navigator's `set_dir`, the
//! editor's `project_root`, and `open_in_editor` — and they used to disagree
//! (#78). Two called `std::path::absolute` and documented at length why
//! *not* to resolve symlinks; the third called `fs::canonicalize`, which
//! resolves them. The disagreement was reachable: navigate into a symlinked
//! directory with `l` and the navigator had already resolved it, so `o` opened
//! the link target — a path the navigator never showed.
//!
//! The rule is now this function, at all three sites.

use std::path::{Component, Path, PathBuf};

/// Absolutise `path` and collapse `.` and `..` **without touching the
/// filesystem**.
///
/// Two jobs, and it is worth being clear that they are separate.
///
/// **Absolutising** is what the navigator actually needed from
/// `fs::canonicalize`. `FileNav::go_to_parent` climbs with `Path::parent`,
/// which on a bare `.` returns `None` — so without this, launching recon with
/// no argument left the user unable to climb out of the directory they started
/// in.
///
/// **Collapsing** is what has to be added back, because `std::path::absolute`
/// deliberately keeps `..` — resolving it lexically is wrong in general, since
/// `a/link/..` is `a` only when `link` is a real directory. That generality is
/// exactly what recon does not want: the navigator's contract is that it shows
/// the path you walked, so `recon ..` should title itself with the parent
/// directory rather than `/where/you/were/..`, and climbing out of *that*
/// should not land you back where you started.
///
/// So the collapse is lexical on purpose. On the one input where lexical and
/// filesystem truth differ — a `..` immediately after a symlinked directory —
/// recon answers with the path the user typed or walked, which is the same
/// answer `editor::project_root` and `App::open_in_editor` already gave.
///
/// `/..` is `/`, matching the kernel: the root's parent is the root.
///
/// Falls back to the path as given if `absolute` fails, which it can only do
/// when the current directory is unreadable.
pub(crate) fn lexical_absolute(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Only a `Normal` segment may be popped. Popping a `RootDir`
                // or a Windows `Prefix` would turn an absolute path into a
                // relative one, and popping a `..` that survived the fallback
                // above would climb somewhere the caller never named.
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !absolute.has_root() {
                    // Relative only, i.e. the `absolute` fallback fired. A
                    // leading `..` is meaningful there and must be kept.
                    out.push(component.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_a_parent_segment() {
        assert_eq!(lexical_absolute(Path::new("/a/b/../c")), Path::new("/a/c"));
    }

    #[test]
    fn collapses_repeated_parent_segments() {
        assert_eq!(lexical_absolute(Path::new("/a/b/c/../..")), Path::new("/a"));
    }

    #[test]
    fn drops_current_dir_segments() {
        assert_eq!(
            lexical_absolute(Path::new("/a/./b/./c")),
            Path::new("/a/b/c")
        );
    }

    /// The root's parent is the root, which is what `cd /..` does.
    #[test]
    fn parent_of_root_is_root() {
        assert_eq!(lexical_absolute(Path::new("/..")), Path::new("/"));
        assert_eq!(lexical_absolute(Path::new("/../../..")), Path::new("/"));
    }

    #[test]
    fn root_survives_unchanged() {
        assert_eq!(lexical_absolute(Path::new("/")), Path::new("/"));
    }

    /// The job `fs::canonicalize` was really doing for the navigator: without
    /// an absolute path, `Path::parent` on `.` is `None` and `go_to_parent`
    /// cannot climb at all.
    #[test]
    fn a_relative_path_becomes_absolute_and_climbable() {
        let cwd = std::env::current_dir().expect("cwd");
        assert_eq!(lexical_absolute(Path::new(".")), cwd);
        assert!(
            lexical_absolute(Path::new(".")).parent().is_some(),
            "an absolutised `.` must have a parent to climb to"
        );
    }

    #[test]
    fn a_relative_parent_resolves_against_the_cwd() {
        let cwd = std::env::current_dir().expect("cwd");
        let expected = cwd.parent().expect("cwd has a parent");
        assert_eq!(lexical_absolute(Path::new("..")), expected);
    }

    #[test]
    fn is_idempotent() {
        let once = lexical_absolute(Path::new("/a/b/../c/./d"));
        assert_eq!(lexical_absolute(&once), once);
    }

    /// The whole point of #78: this must **not** resolve symlinks, so the
    /// navigator keeps showing the path the user walked and `open_in_editor`
    /// opens that same path.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_is_left_alone() {
        use std::fs;

        let root = Path::new("target/test-paths/symlink_left_alone");
        let _ = fs::remove_dir_all(root);
        fs::create_dir_all(root.join("real")).expect("create fixture");
        let link = root.join("link");
        std::os::unix::fs::symlink("real", &link).expect("create symlink");

        let resolved = lexical_absolute(&link);

        assert!(
            resolved.ends_with("symlink_left_alone/link"),
            "the symlink was resolved away: {resolved:?}"
        );
        assert_eq!(
            fs::canonicalize(&link).expect("canonicalize").file_name(),
            Some(std::ffi::OsStr::new("real")),
            "sanity: canonicalize really would have resolved it"
        );
    }

    /// The one input where lexical collapsing and filesystem truth disagree,
    /// pinned deliberately rather than left to be discovered. recon answers
    /// with the walked path; `canonicalize` would answer with the link's
    /// parent.
    #[cfg(unix)]
    #[test]
    fn a_parent_after_a_symlink_collapses_lexically() {
        use std::fs;

        let root = Path::new("target/test-paths/parent_after_symlink");
        let _ = fs::remove_dir_all(root);
        fs::create_dir_all(root.join("elsewhere/deep")).expect("create fixture");
        fs::create_dir_all(root.join("here")).expect("create fixture");
        let link = root.join("here/link");
        std::os::unix::fs::symlink("../elsewhere/deep", &link).expect("create symlink");

        let resolved = lexical_absolute(&link.join(".."));

        assert!(
            resolved.ends_with("parent_after_symlink/here"),
            "expected the walked path's parent, got {resolved:?}"
        );
    }
}
