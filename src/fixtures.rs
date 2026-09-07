//! One registry of fixture names for every test module (#164).
//!
//! Three test modules each grew their own way of making scratch files under
//! `target/`: `lib.rs` claimed directory names through a guarded registry,
//! `fileview.rs` claimed file names through a second registry that could not
//! see the first, and `filenav.rs` — plus a dozen `lib.rs` tests that built
//! their path by hand — claimed nothing and went straight to
//! `remove_dir_all`/`create_dir_all`. The collision guard #69 added to close
//! a release-only flake covered exactly one of the three. A new fixture whose
//! name differed from an existing one only in case, in any of the unguarded
//! places, raced it on a case-insensitive filesystem with no assertion to
//! name the pair.
//!
//! Everything now goes through here: one root, one registry, one guard.
//! `#[cfg(test)]` at the module declaration, so nothing in the shipped
//! binary knows this exists.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Where every fixture lives. One root rather than the three there used to
/// be (`test-appdirs`, `test-navdirs`, `test-fixtures`) so that the registry
/// below describes the filesystem exactly: two names that would collide on
/// disk collide here.
const ROOT: &str = "target/test-fixtures";

/// Every fixture name claimed so far in this process, whichever module
/// claimed it.
static CLAIMED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Panic loudly if `name` has already been used for a fixture in this
/// process, instead of letting two tests race to create and delete the same
/// path. That race is exactly what caused a real, release-only flake: both
/// tests "succeeded" and just clobbered each other's files depending on
/// interleaving.
///
/// **Compared case-insensitively, because the filesystem is** (#69). This
/// guard was `used == name` and so had a hole exactly the shape of the bug
/// it exists to prevent: macOS ships case-insensitive APFS, so `o_ctrl` and
/// `O_ctrl` name one directory, and five `o_*`/`O_*` fixture pairs sat on
/// top of each other undetected. The failure was a `NotFound` on
/// `fs::write` roughly one run in five — one test's `remove_dir_all`
/// landing between the other's `create_dir_all` and its `fs::write`.
///
/// Deliberately not conditioned on the host filesystem. A guard that only
/// fired on macOS would let a colliding pair be added on Linux and
/// rediscovered by whoever next ran the suite on a Mac; refusing the pair
/// everywhere costs nothing but a fixture rename.
///
/// `eq_ignore_ascii_case` rather than a full Unicode case fold: fixture
/// names here are hand-written ASCII identifiers, and the ASCII form needs
/// no allocation. A non-ASCII fixture name would slip through, which is a
/// smaller hole than the one this closes and not one this suite can reach.
fn claim(name: &str) {
    let mut names = CLAIMED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        !names.iter().any(|used| used.eq_ignore_ascii_case(name)),
        "fixture name {name:?} is already in use by another test (compared \
         case-insensitively — macOS treats {name:?} and its other-case \
         spellings as one path) — pick a unique name"
    );
    names.push(name.to_string());
}

/// Whether `name` has been claimed, by any spelling of its case.
fn is_claimed(name: &str) -> bool {
    CLAIMED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|used| used.eq_ignore_ascii_case(name))
}

/// A fresh, empty `target/test-fixtures/<name>/`, claimed for this test and
/// recreated. The caller populates it.
pub(crate) fn fixture_dir(name: &str) -> PathBuf {
    claim(name);
    let dir = Path::new(ROOT).join(name);
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

/// A file `target/test-fixtures/<name>` holding `bytes`, claimed the same
/// way — a file and a directory cannot share a name, so they share the
/// registry.
pub(crate) fn fixture_file(name: &str, bytes: &[u8]) -> PathBuf {
    claim(name);
    fs::create_dir_all(ROOT).expect("create fixture root");
    let path = Path::new(ROOT).join(name);
    fs::write(&path, bytes).expect("write fixture");
    path
}

/// The directory `fixture_dir(name)` made earlier in this test, for the
/// tests that add a second file to it once the `App` is up.
///
/// Panics when nothing has claimed `name`. Before this module those tests
/// spelt the path out by hand, and a hand-typed path can quietly name a
/// directory no test owns — or one another test is deleting.
pub(crate) fn fixture_path(name: &str) -> PathBuf {
    assert!(
        is_claimed(name),
        "fixture {name:?} was never claimed — make it with fixture_dir first"
    );
    Path::new(ROOT).join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two fixture names differing only in case are a collision, not two
    /// names — see `claim`. The probe names are deliberately not any real
    /// fixture's: claiming a name here consumes it for the rest of the
    /// process.
    #[test]
    #[should_panic(expected = "already in use")]
    fn a_name_differing_only_in_case_is_a_collision() {
        fixture_dir("zz_case_probe");
        fixture_dir("ZZ_CASE_PROBE");
    }

    /// The registry is one registry: a file claimed by one module and a
    /// directory claimed by another cannot share a name, because on disk
    /// they cannot share a path.
    #[test]
    #[should_panic(expected = "already in use")]
    fn a_file_and_a_directory_cannot_share_a_name() {
        fixture_file("zz_shared_probe", b"x");
        fixture_dir("zz_shared_probe");
    }

    #[test]
    #[should_panic(expected = "never claimed")]
    fn a_path_is_only_handed_out_for_a_claimed_name() {
        fixture_path("zz_unclaimed_probe");
    }

    #[test]
    fn a_directory_is_recreated_empty() {
        let dir = fixture_dir("zz_recreate_probe");
        fs::write(dir.join("stale.txt"), "x").expect("write");
        // A second claim would panic, so the recreation is exercised through
        // the same call the guard sits in front of: clear the registry entry
        // and go again.
        CLAIMED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|used| used != "zz_recreate_probe");

        let again = fixture_dir("zz_recreate_probe");

        assert_eq!(again, dir);
        assert!(
            !again.join("stale.txt").exists(),
            "the old contents survived"
        );
        assert_eq!(fixture_path("zz_recreate_probe"), dir);
    }
}
