//! A pattern from a request in plain language (#319): the filter editor's
//! request line, answered by a language model on a worker thread.
//!
//! The model is behind the `Model` trait, so the editor does not know which
//! model it talks to. `system` gives Apple's Foundation Models when recon is
//! built with the `foundation-models` feature on an Apple-silicon Mac, and
//! nothing on every other build: the editor then has no request line, and
//! everything else in it works as before.
//!
//! What the model receives is the fixed `INSTRUCTIONS` and the text
//! `request_text` builds: the filter's prompt, its marked lines, a few other
//! lines of the file, and the request. The filter's description is for
//! people only and is never an argument to it.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

/// What the model is told before every request: the rules of the `regex`
/// crate's dialect, and the shape of the answer.
pub const INSTRUCTIONS: &str = "\
You write one regular expression for a log viewer. The viewer tests the \
expression against each line of a file separately, and a line matches when \
the expression is found anywhere in it.

The expression is for the Rust `regex` crate. Its rules:
- There is no lookahead or lookbehind: (?=, (?!, (?<= and (?<! are errors.
- There are no backreferences: \\1 is an error.
- \\d, \\w, \\s, \\b, character classes, alternation, repetition and \
  non-capturing groups (?:...) are available.
- (?i) at the start makes the whole expression case-insensitive.
- Escape . * + ? ( ) [ ] { } | ^ $ and \\ with a backslash to match them \
  literally.

To leave out lines, match only the lines to keep: there is no way to say \
\"not\" inside the expression. Prefer a short expression that matches the \
important text over one that copies a whole line.

Answer with exactly two lines and nothing else:
pattern: the expression, as it is, with no quotes and no backticks
explanation: one short sentence about what the expression matches";

/// The most marked lines the model receives of each kind.
const MARKS_MAX: usize = 20;

/// How many of the file's other lines the model receives, spread over the
/// file, so it sees the lines the pattern must not catch by accident.
pub(crate) const SAMPLE_LINES: usize = 12;

/// The most characters of one line the model receives. The model's context
/// is small, and a log line's point is near its start.
const LINE_MAX: usize = 200;

/// A pattern the model proposes, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub pattern: String,
    pub explanation: String,
}

/// A language model that writes a pattern.
pub trait Model: Send + Sync {
    /// Whether the model can take a request now. Asked each time the filter
    /// editor opens: a model can finish its download while recon runs.
    fn available(&self) -> bool;

    /// Answer `text` under `INSTRUCTIONS`. Runs on a worker thread and may
    /// take seconds; it stops early, with any error, once `cancel` fires.
    fn generate(&self, text: &str, cancel: &Cancel) -> Result<Candidate, String>;
}

/// The model this build has, or `None` when it has none.
#[must_use]
pub fn system() -> Option<Arc<dyn Model>> {
    #[cfg(all(
        feature = "foundation-models",
        target_os = "macos",
        target_arch = "aarch64"
    ))]
    {
        foundation::FoundationModel::new().map(|model| Arc::new(model) as Arc<dyn Model>)
    }
    #[cfg(not(all(
        feature = "foundation-models",
        target_os = "macos",
        target_arch = "aarch64"
    )))]
    {
        None
    }
}

/// A switch the editor turns to stop a request, shared with the thread that
/// runs it.
#[derive(Clone, Default)]
pub struct Cancel(Arc<Mutex<CancelState>>);

#[derive(Default)]
struct CancelState {
    cancelled: bool,
    /// What stops the model, once the model has a way to be stopped.
    hook: Option<Box<dyn FnOnce() + Send>>,
}

impl std::fmt::Debug for Cancel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Cancel").field(&self.is_cancelled()).finish()
    }
}

impl Cancel {
    /// Stop the request: run the hook, if the model gave one.
    pub fn cancel(&self) {
        let hook = {
            let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            state.cancelled = true;
            state.hook.take()
        };
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Run `hook` when the request is cancelled. `false`, and `hook` is not
    /// kept, when it is cancelled already: the model must not start.
    pub fn on_cancel(&self, hook: impl FnOnce() + Send + 'static) -> bool {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.cancelled {
            return false;
        }
        state.hook = Some(Box::new(hook));
        true
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cancelled
    }
}

/// A request on its worker thread. Dropping it cancels the request, and its
/// reply then has nowhere to go: a reply that comes after a cancel never
/// reaches the editor.
#[derive(Debug)]
pub(crate) struct Running {
    reply: Receiver<Result<Candidate, String>>,
    cancel: Cancel,
    pub(crate) started: Instant,
}

impl Running {
    /// Send `text` to `model` on a new thread.
    pub(crate) fn start(model: Arc<dyn Model>, text: String) -> Self {
        let (tx, reply) = mpsc::channel();
        let cancel = Cancel::default();
        let theirs = cancel.clone();
        std::thread::spawn(move || {
            let result = model.generate(&text, &theirs);
            // The editor dropped its end if the request was cancelled.
            let _ = tx.send(result);
        });
        Self {
            reply,
            cancel,
            started: Instant::now(),
        }
    }

    /// The reply, once it is here. Never blocks.
    pub(crate) fn poll(&self) -> Option<Result<Candidate, String>> {
        match self.reply.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the model stopped".to_string())),
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// The text the model receives with `INSTRUCTIONS`: the filter's `prompt`,
/// the lines it must match and must not match, `sample` lines of the file
/// with no mark, and the user's `request`. Nothing else goes to the model,
/// and there is no argument for the filter's description.
pub(crate) fn request_text(
    prompt: Option<&str>,
    must_match: &[&str],
    must_not_match: &[&str],
    sample: &[&str],
    request: &str,
) -> String {
    let mut text = String::new();
    if let Some(prompt) = prompt {
        text.push_str("What the lines to match look like: ");
        text.push_str(prompt);
        text.push_str("\n\n");
    }
    section(&mut text, "Lines the pattern must match:", must_match);
    section(
        &mut text,
        "Lines the pattern must not match:",
        must_not_match,
    );
    section(&mut text, "Other lines of the file:", sample);
    text.push_str("Request: ");
    text.push_str(request);
    text.push('\n');
    text
}

/// The pattern and the explanation in the model's answer, from its
/// `pattern:` and `explanation:` lines.
///
/// Plain lines, not JSON: a pattern is full of backslashes, and the model
/// does not escape them in a JSON string, so its JSON did not parse. The
/// model also often leaves out the two labels; an answer with no `pattern:`
/// line has its pattern on its first line and its explanation after it. A
/// pattern the model put in backticks or double quotes all the same is
/// taken out of them.
pub fn parse_answer(answer: &str) -> Result<Candidate, String> {
    let label = |line: &str, name: &str| {
        let line = line.trim().trim_start_matches(['-', '*', ' ']);
        let (key, value) = line.split_once(':')?;
        key.trim()
            .trim_matches('*')
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    };
    let labelled = |name: &str| answer.lines().find_map(|line| label(line, name));
    let (pattern, explanation) = if let Some(pattern) = labelled("pattern") {
        (pattern, labelled("explanation").unwrap_or_default())
    } else {
        let mut lines = answer
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("```"));
        let pattern = lines.next().unwrap_or_default().to_string();
        let rest = lines.collect::<Vec<_>>().join(" ");
        let explanation = label(&rest, "explanation").unwrap_or(rest);
        (pattern, explanation)
    };
    let pattern = unquote(&pattern).to_string();
    if pattern.is_empty() {
        return Err(format!("no pattern in the answer: {:?}", answer.trim()));
    }
    Ok(Candidate {
        pattern,
        explanation,
    })
}

/// `text` without one pair of backticks or double quotes around it.
fn unquote(text: &str) -> &str {
    for quote in ['`', '"'] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    text
}

/// A heading and up to `MARKS_MAX` lines under it, each cut to `LINE_MAX`
/// characters, and a blank line. Nothing when there are no lines.
fn section(text: &mut String, heading: &str, lines: &[&str]) {
    if lines.is_empty() {
        return;
    }
    text.push_str(heading);
    text.push('\n');
    for line in lines.iter().take(MARKS_MAX) {
        let line: String = line.chars().take(LINE_MAX).collect();
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text.push('\n');
}

#[cfg(all(
    feature = "foundation-models",
    target_os = "macos",
    target_arch = "aarch64"
))]
mod foundation {
    use super::{Cancel, Candidate, INSTRUCTIONS, Model};
    use fm_rs::{GenerationOptions, Session, SystemLanguageModel};

    /// Apple's on-device model, through `fm-rs`.
    pub(super) struct FoundationModel(SystemLanguageModel);

    impl FoundationModel {
        /// `None` on a macOS too old to have the framework.
        pub(super) fn new() -> Option<Self> {
            SystemLanguageModel::new().ok().map(Self)
        }
    }

    impl Model for FoundationModel {
        fn available(&self) -> bool {
            self.0.is_available()
        }

        fn generate(&self, text: &str, cancel: &Cancel) -> Result<Candidate, String> {
            // A new session for each request: the request text carries all
            // the context, and a session's cancel handle stops whatever the
            // session runs, so one per request cannot stop a later one.
            let session =
                Session::with_instructions(&self.0, INSTRUCTIONS).map_err(|e| e.to_string())?;
            let handle = session.cancellation_handle();
            if !cancel.on_cancel(move || handle.cancel()) {
                return Err("cancelled".to_string());
            }
            let answer = session
                .respond(text, &GenerationOptions::default())
                .map_err(|e| e.to_string())?;
            super::parse_answer(answer.content())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_text_has_each_part_under_its_heading() {
        let text = request_text(
            Some("timeouts"),
            &["ERROR timeout"],
            &["ERROR timeout DEMO"],
            &["INFO ok"],
            "also exclude DEMO",
        );
        assert_eq!(
            text,
            "What the lines to match look like: timeouts\n\n\
             Lines the pattern must match:\nERROR timeout\n\n\
             Lines the pattern must not match:\nERROR timeout DEMO\n\n\
             Other lines of the file:\nINFO ok\n\n\
             Request: also exclude DEMO\n"
        );
    }

    #[test]
    fn an_empty_part_has_no_heading() {
        assert_eq!(request_text(None, &[], &[], &[], "x"), "Request: x\n");
    }

    #[test]
    fn a_long_line_is_cut() {
        let long = "a".repeat(LINE_MAX + 50);
        let text = request_text(None, &[&long], &[], &[], "x");
        assert!(text.contains(&"a".repeat(LINE_MAX)));
        assert!(!text.contains(&"a".repeat(LINE_MAX + 1)));
    }

    #[test]
    fn an_answer_gives_its_pattern_and_explanation() {
        let answer = "pattern: ERROR \\d+: timeout\nexplanation: Timeouts.\n";
        assert_eq!(
            parse_answer(answer),
            Ok(Candidate {
                pattern: r"ERROR \d+: timeout".to_string(),
                explanation: "Timeouts.".to_string(),
            })
        );
    }

    #[test]
    fn a_quoted_or_marked_up_answer_is_read_too() {
        let answer = "Here it is:\n- **Pattern**: `a:b`\n- **Explanation**: \"x\"";
        let candidate = parse_answer(answer).expect("a pattern");
        assert_eq!(candidate.pattern, "a:b");
        assert_eq!(candidate.explanation, "\"x\"");
        assert_eq!(
            parse_answer("pattern: \"a\"").expect("a pattern").pattern,
            "a"
        );
    }

    #[test]
    fn an_answer_with_no_labels_has_the_pattern_first() {
        let candidate = parse_answer("```\nERROR \\d\\d\n```\nError lines\nwith a number.\n")
            .expect("a pattern");
        assert_eq!(candidate.pattern, r"ERROR \d\d");
        assert_eq!(candidate.explanation, "Error lines with a number.");
        let candidate = parse_answer("a+\n").expect("a pattern");
        assert_eq!(
            (candidate.pattern.as_str(), candidate.explanation.as_str()),
            ("a+", "")
        );
    }

    #[test]
    fn an_answer_with_no_pattern_is_an_error() {
        assert!(parse_answer("pattern: ``").is_err());
        assert!(parse_answer("  \n").is_err());
    }

    #[test]
    fn a_cancel_before_the_hook_refuses_it() {
        let cancel = Cancel::default();
        cancel.cancel();
        assert!(!cancel.on_cancel(|| {}));
    }

    #[test]
    fn a_cancel_runs_the_hook() {
        let cancel = Cancel::default();
        let (tx, rx) = mpsc::channel();
        assert!(cancel.on_cancel(move || tx.send(()).expect("receiver")));
        cancel.cancel();
        assert!(rx.try_recv().is_ok());
        assert!(cancel.is_cancelled());
    }
}
