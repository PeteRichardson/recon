//! The view cache (#455): results kept so that `,` and `.` back to a file,
//! and a prefetched file, show at once.
//!
//! One entry for each path, stamped with the mtime and size it was read at,
//! then one result for each view and extent. A file that changes on disk
//! has a new stamp, and its old results go at its next read. The least
//! recently used file goes first once the results pass the byte limit.

use super::read::Contents;
use super::{Extent, View};
use crate::scan::{self, Stamp};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// The most bytes of lines the cache holds.
pub(crate) const LIMIT: usize = 256 << 20;

/// Which version of a file a result is for: the stamp the explorer's scan
/// cache also compares, so the two agree on when a file changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Key {
    path: PathBuf,
    stamp: Stamp,
}

impl Key {
    /// The key for `path` as it is on disk now, or `None` when it cannot be
    /// stat'd: a file with no stamp is not cached.
    pub(crate) fn of(path: &Path) -> Option<Self> {
        Some(Self {
            path: path.to_path_buf(),
            stamp: scan::stamp(path).ok()?,
        })
    }
}

#[derive(Debug)]
struct Entry {
    stamp: Stamp,
    binary: Option<bool>,
    views: Vec<(View, Extent, Contents)>,
    bytes: usize,
    used: u64,
}

impl Entry {
    fn new(stamp: Stamp) -> Self {
        Self {
            stamp,
            binary: None,
            views: Vec::new(),
            bytes: 0,
            used: 0,
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    files: HashMap<PathBuf, Entry>,
    bytes: usize,
    clock: u64,
}

impl Inner {
    /// The entry for `key`, emptied first when it was for another version
    /// of the file.
    fn entry(&mut self, key: &Key) -> &mut Entry {
        self.clock += 1;
        let entry = self
            .files
            .entry(key.path.clone())
            .or_insert_with(|| Entry::new(key.stamp));
        if entry.stamp != key.stamp {
            self.bytes -= entry.bytes;
            *entry = Entry::new(key.stamp);
        }
        entry.used = self.clock;
        entry
    }

    /// The entry for `key`, if it is for this version of the file.
    fn current(&mut self, key: &Key) -> Option<&mut Entry> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.files.get_mut(&key.path)?;
        if entry.stamp != key.stamp {
            return None;
        }
        entry.used = clock;
        Some(entry)
    }
}

/// Shared by the workers.
#[derive(Debug)]
pub(crate) struct Cache {
    inner: Mutex<Inner>,
    limit: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self::with_limit(LIMIT)
    }
}

/// What `contents` cost to keep: each line's text and its `String`.
fn size_of(contents: &Contents) -> usize {
    contents
        .lines
        .iter()
        .map(|line| line.len() + std::mem::size_of::<String>())
        .sum()
}

impl Cache {
    pub(crate) fn with_limit(limit: usize) -> Self {
        Self {
            inner: Mutex::default(),
            limit,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the file is binary, if a worker already looked.
    pub(crate) fn binary(&self, key: &Key) -> Option<bool> {
        self.lock().current(key)?.binary
    }

    pub(crate) fn set_binary(&self, key: &Key, binary: bool) {
        self.lock().entry(key).binary = Some(binary);
    }

    /// The file in `view`, read to `extent`. A whole read answers for a
    /// preview too: it is everything the preview would have shown.
    pub(crate) fn get(&self, key: &Key, view: View, extent: Extent) -> Option<Contents> {
        let mut inner = self.lock();
        inner
            .current(key)?
            .views
            .iter()
            .find(|(v, e, _)| *v == view && (*e == extent || *e == Extent::Full))
            .map(|(_, _, contents)| contents.clone())
    }

    /// Keep `contents`. A message (a read that failed) is not kept: the
    /// next read may succeed. A whole read replaces the preview it answers
    /// for.
    pub(crate) fn put(&self, key: Key, view: View, extent: Extent, contents: &Contents) {
        if !contents.text && !contents.hex {
            return;
        }
        let bytes = size_of(contents);
        if bytes > self.limit {
            return;
        }
        let mut inner = self.lock();
        let entry = inner.entry(&key);
        if entry
            .views
            .iter()
            .any(|(v, e, _)| *v == view && *e == extent)
        {
            return;
        }
        let mut freed = 0;
        if extent == Extent::Full {
            entry.views.retain(|(v, _, contents)| {
                let preview = *v == view;
                if preview {
                    freed += size_of(contents);
                }
                !preview
            });
        }
        entry.views.push((view, extent, contents.clone()));
        entry.bytes = entry.bytes + bytes - freed;
        inner.bytes = inner.bytes + bytes - freed;
        while inner.bytes > self.limit {
            let Some(oldest) = inner
                .files
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            if let Some(entry) = inner.files.remove(&oldest) {
                inner.bytes -= entry.bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fixture_file;
    use std::sync::Arc;
    use std::time::SystemTime;

    fn text(lines: &[&str]) -> Contents {
        Contents {
            lines: Arc::new(lines.iter().map(ToString::to_string).collect()),
            text: true,
            ..Contents::default()
        }
    }

    #[test]
    fn a_change_on_disk_is_a_new_key() {
        let path = fixture_file("cache_key_mtime.txt", b"a\n");
        let before = Key::of(&path).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH).unwrap();
        assert_ne!(
            Key::of(&path).unwrap(),
            before,
            "same key after a new mtime"
        );
    }

    #[test]
    fn a_whole_read_answers_for_a_preview_but_not_the_other_way() {
        let path = fixture_file("cache_extent.txt", b"a\n");
        let key = Key::of(&path).unwrap();
        let cache = Cache::default();

        cache.put(key.clone(), View::Raw, Extent::Preview, &text(&["a"]));
        assert!(cache.get(&key, View::Raw, Extent::Full).is_none());
        assert!(cache.get(&key, View::Hex, Extent::Preview).is_none());

        cache.put(key.clone(), View::Raw, Extent::Full, &text(&["a", "b"]));
        assert!(cache.get(&key, View::Raw, Extent::Full).is_some());
    }

    #[test]
    fn a_whole_read_replaces_the_preview_it_answers_for() {
        let key = Key::of(&fixture_file("cache_replace.txt", b"a\n")).unwrap();
        let one = text(&["x"]);
        let cache = Cache::with_limit(size_of(&one) * 2);

        cache.put(key.clone(), View::Raw, Extent::Preview, &one);
        cache.put(key.clone(), View::Raw, Extent::Full, &one);
        cache.put(key.clone(), View::Hex, Extent::Full, &one);

        assert!(
            cache.get(&key, View::Raw, Extent::Full).is_some(),
            "the preview was still charged, and the file was pushed out"
        );
    }

    #[test]
    fn the_least_recently_used_file_goes_first() {
        let a = Key::of(&fixture_file("cache_lru_a.txt", b"a\n")).unwrap();
        let b = Key::of(&fixture_file("cache_lru_b.txt", b"b\n")).unwrap();
        let c = Key::of(&fixture_file("cache_lru_c.txt", b"c\n")).unwrap();
        let one = text(&["x"]);
        let cache = Cache::with_limit(size_of(&one) * 2);

        cache.put(a.clone(), View::Raw, Extent::Full, &one);
        cache.put(b.clone(), View::Raw, Extent::Full, &one);
        cache.get(&a, View::Raw, Extent::Full);
        cache.put(c.clone(), View::Raw, Extent::Full, &one);

        assert!(cache.get(&a, View::Raw, Extent::Full).is_some());
        assert!(
            cache.get(&b, View::Raw, Extent::Full).is_none(),
            "b was older"
        );
        assert!(cache.get(&c, View::Raw, Extent::Full).is_some());
    }

    #[test]
    fn a_failed_read_is_not_kept() {
        let key = Key::of(&fixture_file("cache_message.txt", b"a\n")).unwrap();
        let cache = Cache::default();
        cache.put(
            key.clone(),
            View::Raw,
            Extent::Full,
            &Contents::message("<denied>".into()),
        );
        assert!(cache.get(&key, View::Raw, Extent::Full).is_none());
    }
}
