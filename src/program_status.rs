//! recon's status, as Rex's Program Status Protocol reports it (#638).
//!
//! OSC 7501: `ESC ] 7501 ; state=…:app=recon:msg=<base64> ESC \`. Each
//! report replaces the whole record. See ADR 0022.

use base64::Engine as _;

/// `[terminal] status` in `config.toml`: whether recon sends reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Setting {
    /// Send them when the terminal is Rex.
    #[default]
    Auto,
    /// Always send them. For a Rex pane that does not say it is Rex, as
    /// inside tmux.
    On,
    /// Never send them.
    Off,
}

impl Setting {
    /// Whether to send reports, in the terminal that `TERM_PROGRAM` names.
    #[must_use]
    pub fn reports(self, term_program: Option<&str>) -> bool {
        match self {
            Self::Auto => term_program == Some("rex"),
            Self::On => true,
            Self::Off => false,
        }
    }
}

/// The longest `msg` Rex takes, in bytes before the base64.
const MSG_MAX: usize = 2048;

/// What recon is doing, as one report says it. `done` comes with #640.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Status {
    state: State,
    msg: String,
}

/// The `state` a report gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    /// Busy, with the percentage done when the work knows it.
    Working(Option<u8>),
}

impl Status {
    /// At rest, showing what `msg` says.
    pub(crate) fn idle(msg: impl Into<String>) -> Self {
        Self {
            state: State::Idle,
            msg: msg.into(),
        }
    }

    /// Busy with what `msg` says, `progress` percent of the way.
    pub(crate) fn working(msg: impl Into<String>, progress: Option<u8>) -> Self {
        Self {
            state: State::Working(progress),
            msg: msg.into(),
        }
    }
}

/// The report that removes recon's record: sent as recon gives the
/// terminal back. No `id`, so it removes every record of the pane: recon
/// sends only the one, and a program run from recon has its own pane.
pub const CLEAR: &[u8] = b"\x1b]7501;state=clear\x1b\\";

/// `count` in at most four characters: exact below 10,000, then whole
/// thousands, then millions. Rounded down.
pub(crate) fn short_count(count: usize) -> String {
    match count {
        0..10_000 => count.to_string(),
        10_000..1_000_000 => format!("{}k", count / 1_000),
        1_000_000..10_000_000 => {
            format!("{}.{}M", count / 1_000_000, count / 100_000 % 10)
        }
        _ => format!("{}M", count / 1_000_000),
    }
}

/// The bytes of one report.
pub(crate) fn encode(status: &Status) -> Vec<u8> {
    // `char::is_control` is the spec's set: U+0000–U+001F, U+007F and
    // U+0080–U+009F.
    let mut text: String = status
        .msg
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    // `floor_char_boundary` by hand: it is newer than the MSRV.
    let mut end = text.len().min(MSG_MAX);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    let msg = base64::engine::general_purpose::STANDARD.encode(text);
    let state = match status.state {
        State::Idle => "idle".to_string(),
        State::Working(None) => "working".to_string(),
        State::Working(Some(percent)) => format!("working:progress={}", percent.min(100)),
    };
    format!("\x1b]7501;state={state}:app=recon:msg={msg}\x1b\\").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_report_names_recon_and_carries_the_message_in_base64() {
        let report = encode(&Status::idle("app.log"));
        // "app.log" in standard base64 is "YXBwLmxvZw==".
        assert_eq!(
            report,
            b"\x1b]7501;state=idle:app=recon:msg=YXBwLmxvZw==\x1b\\"
        );
    }

    /// Busy work names its state, and a percentage when it has one. With
    /// no `progress`, Rex shows the work as busy with no end in sight.
    #[test]
    fn a_working_report_carries_progress_only_when_it_has_some() {
        // "load" and "pass" in standard base64.
        assert_eq!(
            encode(&Status::working("load", None)),
            b"\x1b]7501;state=working:app=recon:msg=bG9hZA==\x1b\\"
        );
        assert_eq!(
            encode(&Status::working("pass", Some(37))),
            b"\x1b]7501;state=working:progress=37:app=recon:msg=cGFzcw==\x1b\\"
        );
    }

    /// A decoded `msg` with a control character makes Rex drop the whole
    /// report. A file name can hold a newline or a tab.
    #[test]
    fn control_characters_in_the_message_become_spaces() {
        let report = encode(&Status::idle("a\nb\tc\u{7f}d\u{85}e"));
        // "a b c d e" in standard base64.
        assert_eq!(
            report,
            b"\x1b]7501;state=idle:app=recon:msg=YSBiIGMgZCBl\x1b\\"
        );
    }

    /// Rex drops a report whose `msg` decodes to more than 2048 bytes.
    /// `é` is two bytes, so 1025 of them cut at 1024: 2048 bytes, and never
    /// half a character.
    #[test]
    fn a_long_message_is_cut_to_2048_bytes_on_a_character_boundary() {
        let report = String::from_utf8(encode(&Status::idle("é".repeat(1025)))).unwrap();
        let msg = report
            .strip_prefix("\x1b]7501;state=idle:app=recon:msg=")
            .and_then(|rest| rest.strip_suffix("\x1b\\"))
            .expect("one idle report");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(msg)
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "é".repeat(1024));
    }

    /// With no `id`, `clear` removes every record on the terminal: recon
    /// writes only the one root record, so that is the record it leaves.
    /// Rex sets `TERM_PROGRAM=rex`. Another terminal could show the bytes,
    /// or do something else with an OSC it does not know, so `auto` stays
    /// quiet there.
    #[test]
    fn auto_reports_only_in_rex_and_on_and_off_ignore_the_terminal() {
        for (setting, term_program, expected) in [
            (Setting::Auto, Some("rex"), true),
            (Setting::Auto, Some("iTerm.app"), false),
            (Setting::Auto, None, false),
            (Setting::On, None, true),
            (Setting::Off, Some("rex"), false),
        ] {
            assert_eq!(
                setting.reports(term_program),
                expected,
                "{setting:?} in {term_program:?}"
            );
        }
    }

    /// A count must fit beside a name in the Activity popup. Rounded down,
    /// so a count never says more lines than there are.
    #[test]
    fn a_short_count_keeps_four_digits_and_rounds_down_past_them() {
        for (count, text) in [
            (0, "0"),
            (1204, "1204"),
            (9999, "9999"),
            (10_000, "10k"),
            (88_765, "88k"),
            (999_999, "999k"),
            (1_000_000, "1.0M"),
            (1_299_999, "1.2M"),
            (12_345_678, "12M"),
        ] {
            assert_eq!(short_count(count), text, "{count}");
        }
    }
}
