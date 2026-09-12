//! Quoting values for a TOML stanza recon prints.
//!
//! recon never writes `config.toml` — `Cargo.toml` drops `toml`'s serializer
//! so that it cannot — so every stanza it emits is built by hand, and two
//! commands now build one. This is the quoting both use.

/// Quote a template as a TOML string.
///
/// A literal string (`'…'`) wherever the template has no single quote in it,
/// which keeps the backslashes in the `osascript` forms readable — TOML's basic
/// strings would double every one of them. Falls back to a basic string with
/// the two escapes TOML requires when the template contains a `'`.
pub(crate) fn toml_string(value: &str) -> String {
    if value.contains('\'') {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("'{value}'")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_value_is_a_literal_string() {
        assert_eq!(toml_string("zed {file}:{line}"), "'zed {file}:{line}'");
    }

    #[test]
    fn a_value_holding_a_quote_falls_back_to_a_basic_string() {
        assert_eq!(toml_string("it's"), "\"it's\"");
    }

    #[test]
    fn a_basic_string_escapes_backslash_and_quote() {
        assert_eq!(
            toml_string("a\\b\"c'"),
            "\"a\\\\b\\\"c'\"",
            "both escapes TOML requires"
        );
    }
}
