//! The log file (#500): where it is, and how it stays small.
//!
//! `main` decides whether there is a file; this module opens it, appends to
//! it, and rotates it at a size limit, so that a log that is always on keeps
//! many sessions and still does not fill the disk.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// A log file that appends, and rotates at `max_bytes`: `recon.log` becomes
/// `recon.log.1`, `recon.log.1` becomes `recon.log.2`, and so on, until there
/// are `files` files with the live one.
pub struct Rotating {
    path: PathBuf,
    max_bytes: u64,
    files: u32,
    file: File,
    /// The size of the live file: its size when it was opened, and what this
    /// process wrote to it since. A second recon on the same file is not
    /// seen, which is acceptable for a log.
    len: u64,
}

impl Rotating {
    /// Open `path` to append to it.
    ///
    /// # Errors
    ///
    /// The file cannot be opened, or a rotation that it needs fails.
    pub fn open(path: &Path, max_bytes: u64, files: u32) -> io::Result<Self> {
        let file = append(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            max_bytes,
            files,
            file,
            len,
        })
    }

    /// Move each old file one step, the live file to `.1`, and start a new
    /// live file. The file past the count goes. With one file there is no
    /// old file to keep, so the live one is emptied.
    fn rotate(&mut self) -> io::Result<()> {
        if self.files <= 1 {
            self.file.set_len(0)?;
        } else {
            let old = |n: u32| {
                let mut name = self.path.clone().into_os_string();
                name.push(format!(".{n}"));
                PathBuf::from(name)
            };
            // Not there is normal: the old files are made one at a time.
            let _ = std::fs::remove_file(old(self.files - 1));
            for n in (1..self.files - 1).rev() {
                let _ = std::fs::rename(old(n), old(n + 1));
            }
            std::fs::rename(&self.path, old(1))?;
            self.file = append(&self.path)?;
        }
        self.len = 0;
        Ok(())
    }
}

/// `path`, opened to append, and made if it is not there.
fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

impl Write for Rotating {
    /// One record is one write, so a record never starts in one file and
    /// ends in the next. A record larger than the limit gets a file to
    /// itself.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.len > 0 && self.len + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        self.file.write_all(buf)?;
        self.len += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Where the log goes when nothing names a file: `recon/recon.log` under
/// `$XDG_STATE_HOME`, else under `~/.local/state`. `None` when neither names
/// a home.
///
/// The environment as arguments, for the reason `config::config_home_from`
/// takes it so: a test must not set a process-wide variable.
#[must_use]
pub fn default_path(xdg_state_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    Some(crate::config::recon_dir_from(xdg_state_home, home, ".local/state")?.join("recon.log"))
}

/// Choose the log file, or `None` for stderr.
///
/// A file that a setting names wins everywhere, batch mode too. Else, the
/// default file only in the TUI and only when `RUST_LOG` asks for a log:
/// the TUI draws on stderr and drops what is logged there (#246), but a
/// batch run's stderr is where a script reads its log. Without `RUST_LOG`
/// nothing is written to a file that nobody asked for.
#[must_use]
pub fn choose(
    named: Option<&Path>,
    tui: bool,
    rust_log: bool,
    default: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    match named {
        Some(path) => Some(path.to_path_buf()),
        None if tui && rust_log => default(),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fixture_dir;
    use std::fs;

    #[test]
    fn a_second_session_appends_to_the_first() {
        let path = fixture_dir("logfile_appends").join("recon.log");

        Rotating::open(&path, 1000, 2)
            .unwrap()
            .write_all(b"one\n")
            .unwrap();
        Rotating::open(&path, 1000, 2)
            .unwrap()
            .write_all(b"two\n")
            .unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
    }

    /// The names in `dir`, sorted, and what each file holds.
    fn contents(dir: &Path) -> Vec<(String, String)> {
        let mut found: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    fs::read_to_string(&path).unwrap(),
                )
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn a_full_file_rotates_and_the_oldest_goes() {
        let dir = fixture_dir("logfile_rotates");
        let mut log = Rotating::open(&dir.join("recon.log"), 8, 3).unwrap();

        for line in ["aaaaaaa\n", "bbbbbbb\n", "ccccccc\n", "ddddddd\n"] {
            log.write_all(line.as_bytes()).unwrap();
        }

        assert_eq!(
            contents(&dir),
            [
                ("recon.log".to_string(), "ddddddd\n".to_string()),
                ("recon.log.1".to_string(), "ccccccc\n".to_string()),
                ("recon.log.2".to_string(), "bbbbbbb\n".to_string()),
            ]
        );
    }

    #[test]
    fn a_file_full_at_open_rotates_before_the_first_record() {
        let dir = fixture_dir("logfile_full_at_open");
        let path = dir.join("recon.log");
        fs::write(&path, "old session\n").unwrap();

        Rotating::open(&path, 5, 2)
            .unwrap()
            .write_all(b"new\n")
            .unwrap();

        assert_eq!(
            contents(&dir),
            [
                ("recon.log".to_string(), "new\n".to_string()),
                ("recon.log.1".to_string(), "old session\n".to_string()),
            ]
        );
    }

    /// With one file there is no old file: the live one is emptied.
    #[test]
    fn one_file_is_emptied_at_the_limit() {
        let dir = fixture_dir("logfile_one_file");
        let mut log = Rotating::open(&dir.join("recon.log"), 8, 1).unwrap();

        log.write_all(b"aaaaaaa\n").unwrap();
        log.write_all(b"bbbbbbb\n").unwrap();

        assert_eq!(
            contents(&dir),
            [("recon.log".to_string(), "bbbbbbb\n".to_string())]
        );
    }

    #[test]
    fn the_default_file_is_under_the_state_home() {
        assert_eq!(
            default_path(Some("/state"), Some("/home/me")),
            Some(PathBuf::from("/state/recon/recon.log"))
        );
    }

    /// The XDG specification says that a relative or empty path is not
    /// valid, and must be ignored.
    #[test]
    fn without_a_good_state_home_the_default_is_under_home() {
        for state in [None, Some(""), Some("relative/state")] {
            assert_eq!(
                default_path(state, Some("/home/me")),
                Some(PathBuf::from("/home/me/.local/state/recon/recon.log")),
                "{state:?}"
            );
        }
        assert_eq!(default_path(None, None), None);
        assert_eq!(default_path(None, Some("")), None);
    }

    #[test]
    fn a_named_file_wins_in_each_mode() {
        for (tui, rust_log) in [(true, true), (true, false), (false, true), (false, false)] {
            assert_eq!(
                choose(Some(Path::new("/tmp/x.log")), tui, rust_log, || Some(
                    "/d".into()
                )),
                Some("/tmp/x.log".into()),
            );
        }
    }

    #[test]
    fn the_default_file_is_only_for_the_tui_with_rust_log() {
        let default = || Some(PathBuf::from("/d/recon.log"));
        assert_eq!(
            choose(None, true, true, default),
            Some("/d/recon.log".into())
        );
        assert_eq!(choose(None, false, true, default), None, "batch");
        assert_eq!(choose(None, true, false, default), None, "no RUST_LOG");
        assert_eq!(choose(None, true, true, || None), None, "no home");
    }
}
