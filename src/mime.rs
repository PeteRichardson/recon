//! A file's MIME type: the one the file view's title shows and the one a
//! `[[filetype]] mime =` row matches (#467).
//!
//! One function gives both, so the title never shows a type that a row
//! cannot match: the point of the title's type is that it can be copied
//! into the config.

use std::path::Path;

/// The type of a directory.
pub(crate) const DIRECTORY: &str = "inode/directory";

/// The type of a text file that nothing else names.
const TEXT: &str = "text/plain";

/// The type of a binary file that nothing else names.
const OCTET_STREAM: &str = "application/octet-stream";

/// The type of the file at `path` whose first bytes are `head`: the type
/// its magic bytes name, else the type its extension names, else plain
/// text or an octet stream, as the binary check found it.
///
/// Magic bytes first, because they describe the content and the name only
/// claims to. The fallbacks are the two standard ones, so a readable file
/// always has a type.
pub(crate) fn of(path: &Path, head: &[u8], binary: bool) -> &'static str {
    infer::get(head)
        .map(|kind| kind.mime_type())
        .or_else(|| mime_guess::from_path(path).first_raw())
        .unwrap_or(if binary { OCTET_STREAM } else { TEXT })
}

/// Whether `of` can give `mime` for some file: the check a `mime =` row
/// passes when the config loads, so a row that can never match is an
/// error rather than a silent no-op.
///
/// Exact, as matching is. An extension's type counts only if it is the
/// one `of` gives for that extension: `.md` is `text/markdown`, so
/// `text/x-markdown`, its second name, matches no file. `mime_guess` would
/// also take a wildcard or another case, which no file's type is.
pub(crate) fn is_known(mime: &str) -> bool {
    if mime.contains('*') {
        return false;
    }
    infer::is_mime_supported(mime)
        || [TEXT, OCTET_STREAM].contains(&mime)
        || mime_guess::get_mime_extensions_str(mime).is_some_and(|extensions| {
            extensions
                .iter()
                .any(|extension| mime_guess::from_ext(extension).first_raw() == Some(mime))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACH_O: &[u8] = b"\xcf\xfa\xed\xfe\x07\x00\x00\x01";

    #[test]
    fn magic_bytes_name_the_type_first() {
        // A Mach-O file named as Rust: the content wins over the name.
        assert_eq!(
            of(Path::new("tool.rs"), MACH_O, true),
            "application/x-mach-binary"
        );
    }

    #[test]
    fn a_file_with_no_magic_bytes_is_typed_by_its_extension() {
        assert_eq!(
            of(Path::new("src/main.rs"), b"fn main() {}\n", false),
            "text/x-rust"
        );
        assert_eq!(
            of(Path::new("Cargo.TOML"), b"[package]\n", false),
            "text/x-toml"
        );
    }

    #[test]
    fn text_with_no_known_type_is_plain_text() {
        assert_eq!(of(Path::new("README"), b"hello\n", false), "text/plain");
    }

    #[test]
    fn binary_with_no_known_type_is_an_octet_stream() {
        assert_eq!(
            of(Path::new("blob"), b"\x01\x00\x02", true),
            "application/octet-stream"
        );
    }
}
