//! The format of the build stamp.
//!
//! Pure functions only, so that `cargo test` can test them. `build.rs`
//! includes this file with `#[path]` and supplies the facts from git; the
//! library compiles it only for its tests.

/// The facts from git that the stamp shows.
pub(crate) struct Git {
    /// Commits after the last `vX.Y.Z` tag, or `None` if there is no tag.
    pub(crate) since_tag: Option<u32>,
    /// The abbreviated commit hash, without the `g` prefix.
    pub(crate) hash: String,
    /// The tracked files have changes that are not committed.
    pub(crate) dirty: bool,
    /// The branch name, or `None` when `HEAD` is detached.
    pub(crate) branch: Option<String>,
}

/// The `-V` line, without the program name:
/// `0.2.3+5.g1a2b3c4.dirty (branch)`.
///
/// The `+` part is semver build metadata in the `git describe` format. A
/// release build (clean, exactly on a tag) and a build with no git are the
/// release version only: a release build contains exactly the release code,
/// so the branch says nothing about it.
pub(crate) fn short(version: &str, git: Option<&Git>) -> String {
    let Some(git) = git.filter(|git| git.dirty || git.since_tag != Some(0)) else {
        return version.to_string();
    };
    let mut metadata = match git.since_tag {
        Some(n) => format!("+{n}.g{}", git.hash),
        None => format!("+g{}", git.hash),
    };
    if git.dirty {
        metadata.push_str(".dirty");
    }
    let branch = git.branch.as_deref().unwrap_or("detached");
    format!("{version}{metadata} ({branch})")
}

/// The `--version` text, without the program name: the `-V` line, then
/// `checkout:` (only when there is git) and `built:`.
pub(crate) fn long(short: &str, checkout: Option<&str>, built: &str) -> String {
    match checkout {
        Some(checkout) => format!("{short}\ncheckout: {checkout}\nbuilt: {built}"),
        None => format!("{short}\nbuilt: {built}"),
    }
}

/// The number of commits after the tag, from
/// `git describe --tags --long` output such as `v0.1.0-5-g1a2b3c4`.
/// A tag can contain `-`, so the line is split from the right.
pub(crate) fn parse_describe(output: &str) -> Option<u32> {
    let mut parts = output.trim().rsplitn(3, '-');
    let hash = parts.next()?;
    let count = parts.next()?;
    parts.next()?;
    if !hash.starts_with('g') {
        return None;
    }
    count.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(since_tag: Option<u32>, dirty: bool, branch: Option<&str>) -> Git {
        Git {
            since_tag,
            hash: "1a2b3c4".to_string(),
            dirty,
            branch: branch.map(str::to_string),
        }
    }

    #[test]
    fn after_a_tag_shows_the_count_and_hash() {
        let git = git(Some(5), false, Some("main"));
        assert_eq!(short("0.2.3", Some(&git)), "0.2.3+5.g1a2b3c4 (main)");
    }

    #[test]
    fn dirty_adds_the_flag() {
        let git = git(Some(5), true, Some("feature"));
        assert_eq!(
            short("0.2.3", Some(&git)),
            "0.2.3+5.g1a2b3c4.dirty (feature)"
        );
    }

    #[test]
    fn no_tag_shows_only_the_hash() {
        let git = git(None, false, Some("main"));
        assert_eq!(short("0.1.0", Some(&git)), "0.1.0+g1a2b3c4 (main)");
    }

    #[test]
    fn no_tag_and_dirty() {
        let git = git(None, true, Some("main"));
        assert_eq!(short("0.1.0", Some(&git)), "0.1.0+g1a2b3c4.dirty (main)");
    }

    #[test]
    fn clean_on_a_tag_is_the_version_only() {
        let git = git(Some(0), false, Some("main"));
        assert_eq!(short("0.2.3", Some(&git)), "0.2.3");
    }

    #[test]
    fn clean_on_a_tag_with_detached_head_is_the_version_only() {
        let git = git(Some(0), false, None);
        assert_eq!(short("0.2.3", Some(&git)), "0.2.3");
    }

    #[test]
    fn dirty_on_a_tag_keeps_the_metadata() {
        let git = git(Some(0), true, Some("main"));
        assert_eq!(short("0.2.3", Some(&git)), "0.2.3+0.g1a2b3c4.dirty (main)");
    }

    #[test]
    fn detached_head() {
        let git = git(Some(2), false, None);
        assert_eq!(short("0.2.3", Some(&git)), "0.2.3+2.g1a2b3c4 (detached)");
    }

    #[test]
    fn no_git_is_the_version_only() {
        assert_eq!(short("0.2.3", None), "0.2.3");
    }

    #[test]
    fn long_with_git() {
        assert_eq!(
            long(
                "0.2.3 (main)",
                Some("/src/vrsn"),
                "2026-09-29T11:32:07-07:00"
            ),
            "0.2.3 (main)\ncheckout: /src/vrsn\nbuilt: 2026-09-29T11:32:07-07:00"
        );
    }

    #[test]
    fn long_without_git_has_no_checkout() {
        assert_eq!(
            long("0.2.3", None, "2026-09-29T11:32:07-07:00"),
            "0.2.3\nbuilt: 2026-09-29T11:32:07-07:00"
        );
    }

    #[test]
    fn parse_describe_reads_the_count() {
        assert_eq!(parse_describe("v0.1.0-5-g1a2b3c4\n"), Some(5));
        assert_eq!(parse_describe("v0.1.0-0-g1a2b3c4"), Some(0));
    }

    #[test]
    fn parse_describe_allows_a_dash_in_the_tag() {
        assert_eq!(parse_describe("v1.0.0-rc.1-3-g1a2b3c4"), Some(3));
    }

    #[test]
    fn parse_describe_rejects_other_output() {
        assert_eq!(parse_describe(""), None);
        assert_eq!(parse_describe("1a2b3c4"), None);
        assert_eq!(parse_describe("v0.1.0-x-g1a2b3c4"), None);
    }
}
