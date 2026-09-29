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
//! `request_text` builds: the filter's prompt, the current pattern, its
//! marked lines, a few other lines of the file, the session's earlier
//! requests and the latest request. The filter's description is for people
//! only and is never a part of it.
//!
//! The editor checks each pattern the model gives before it shows it (#320):
//! `verify` compiles it and tests it against the marked lines, and a pattern
//! that fails goes back to the model with `retry_text`, up to `ATTEMPTS`
//! times in all.
//!
//! When the user saves a filter after one or more requests, the model
//! writes one prompt from all of them (#322), under `PROMPT_INSTRUCTIONS`
//! and with the text `consolidate_text` builds. Phrase marks, the parts of
//! lines the user selected with the mouse, go to the model as hints in each
//! text; they are never a check.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use regex::Regex;

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

There can be a current pattern and earlier requests. Then change the \
current pattern to do what the latest request asks, and keep what the \
earlier requests asked for.

To leave out lines, match only the lines to keep: there is no way to say \
\"not\" inside the expression. Prefer a short expression that matches the \
important text over one that copies a whole line.

Answer with exactly two lines and nothing else:
pattern: the expression, as it is, with no quotes and no backticks
explanation: one short sentence about what the expression matches";

/// What the model is told before it writes one prompt from a session's
/// requests (#322).
pub const PROMPT_INSTRUCTIONS: &str = "\
You write the description of a filter for a log viewer. A filter keeps the \
lines of a file that its regular expression matches. The user asked for the \
filter in steps, and each step changed what the filter keeps.

Write one sentence in plain language that says what the lines the filter \
keeps look like, with everything that the steps asked for. Later steps win \
over earlier ones. Do not describe the regular expression and do not write \
one. Do not mention the steps.

Answer with exactly one line and nothing else:
prompt: the sentence";

/// The request `Ctrl-r` sends in the filter editor (#321), with the
/// prompt and the marks and no current pattern: write the pattern again
/// from the prompt.
pub(crate) const REGENERATE: &str =
    "Write a new pattern for the lines that the prompt describes. Use the marked lines to test it.";

/// The most times one request goes to the model (#320): the first time,
/// and again each time its pattern does not compile or fails a mark. The
/// `?` help and the README say this number.
pub const ATTEMPTS: usize = 3;

/// The most marked lines the model receives of each kind.
const MARKS_MAX: usize = 20;

/// The most earlier requests the model receives: the latest of them.
const REQUESTS_MAX: usize = 10;

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

/// A language model that writes a pattern, and a filter's prompt.
pub trait Model: Send + Sync {
    /// Whether the model can take a request now. Asked each time the filter
    /// editor opens: a model can finish its download while recon runs.
    fn available(&self) -> bool;

    /// Answer `text` under `INSTRUCTIONS`. Runs on a worker thread and may
    /// take seconds; it stops early, with any error, once `cancel` fires.
    fn generate(&self, text: &str, cancel: &Cancel) -> Result<Candidate, String>;

    /// Answer `text` under `PROMPT_INSTRUCTIONS` with one prompt (#322), on
    /// a worker thread as `generate` is.
    fn consolidate(&self, text: &str, cancel: &Cancel) -> Result<String, String>;
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

/// A request on its worker thread: a pattern, or with `T` a `String` a
/// prompt (#322). Dropping it cancels the request, and its reply then has
/// nowhere to go: a reply that comes after a cancel never reaches the
/// editor.
#[derive(Debug)]
pub(crate) struct Running<T = Candidate> {
    reply: Receiver<Result<T, String>>,
    cancel: Cancel,
}

impl Running {
    /// Send `text` to `model` on a new thread, for a pattern.
    pub(crate) fn start(model: Arc<dyn Model>, text: String) -> Self {
        Running::spawn(move |cancel| model.generate(&text, cancel))
    }
}

impl Running<String> {
    /// Send `text` to `model` on a new thread, for one prompt (#322).
    pub(crate) fn consolidate(model: Arc<dyn Model>, text: String) -> Self {
        Running::spawn(move |cancel| model.consolidate(&text, cancel))
    }
}

impl<T: Send + 'static> Running<T> {
    fn spawn(ask: impl FnOnce(&Cancel) -> Result<T, String> + Send + 'static) -> Self {
        let (tx, reply) = mpsc::channel();
        let cancel = Cancel::default();
        let theirs = cancel.clone();
        std::thread::spawn(move || {
            let result = ask(&theirs);
            // The editor dropped its end if the request was cancelled.
            let _ = tx.send(result);
        });
        Self { reply, cancel }
    }

    /// The reply, once it is here. Never blocks.
    pub(crate) fn poll(&self) -> Option<Result<T, String>> {
        match self.reply.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the model stopped".to_string())),
        }
    }
}

impl<T> Drop for Running<T> {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// What goes to the model for one request (#319). Nothing else goes to
/// it, and there is no part for the filter's description.
#[derive(Debug, Default)]
pub(crate) struct Parts<'a> {
    /// The filter's prompt.
    pub(crate) prompt: Option<&'a str>,
    /// The pattern in the field, when it compiles.
    pub(crate) pattern: Option<&'a str>,
    pub(crate) must_match: &'a [&'a str],
    pub(crate) must_not_match: &'a [&'a str],
    /// Lines of the file with no mark.
    pub(crate) sample: &'a [&'a str],
    /// The phrase marks (#322): each phrase, and the line it is part of.
    pub(crate) phrases: &'a [(&'a str, &'a str)],
    /// The requests of this session the model answered, oldest first.
    pub(crate) earlier: &'a [String],
    /// The request to answer.
    pub(crate) request: &'a str,
}

/// The text the model receives with `INSTRUCTIONS`, each part under its
/// heading. An empty part has no heading.
pub(crate) fn request_text(parts: &Parts) -> String {
    let mut text = String::new();
    if let Some(prompt) = parts.prompt {
        text.push_str("What the lines to match look like: ");
        text.push_str(prompt);
        text.push_str("\n\n");
    }
    if let Some(pattern) = parts.pattern {
        text.push_str("Current pattern: ");
        text.push_str(pattern);
        text.push_str("\n\n");
    }
    section(&mut text, "Lines the pattern must match:", parts.must_match);
    section(
        &mut text,
        "Lines the pattern must not match:",
        parts.must_not_match,
    );
    section(&mut text, "Other lines of the file:", parts.sample);
    phrase_section(&mut text, parts.phrases);
    let earlier: Vec<&str> = parts.earlier[parts.earlier.len().saturating_sub(REQUESTS_MAX)..]
        .iter()
        .map(String::as_str)
        .collect();
    section(&mut text, "Earlier requests, oldest first:", &earlier);
    text.push_str("Request: ");
    text.push_str(parts.request);
    text.push('\n');
    text
}

/// The phrase marks (#322) under their heading, each as the phrase and the
/// line it is part of. Nothing when there are none.
fn phrase_section(text: &mut String, phrases: &[(&str, &str)]) {
    let phrases: Vec<String> = phrases
        .iter()
        .map(|(phrase, line)| {
            let line: String = line.chars().take(LINE_MAX).collect();
            format!("{phrase:?} in the line: {}", line.trim_end())
        })
        .collect();
    let phrases: Vec<&str> = phrases.iter().map(String::as_str).collect();
    section(
        text,
        "Important parts of lines, as hints (they are not tests):",
        &phrases,
    );
}

/// The text the model receives with `PROMPT_INSTRUCTIONS` (#322): the
/// filter's prompt before the session, the session's requests oldest
/// first, the pattern they gave and the phrase marks.
pub(crate) fn consolidate_text(
    prompt: Option<&str>,
    requests: &[String],
    pattern: &str,
    phrases: &[(&str, &str)],
) -> String {
    let mut text = String::new();
    if let Some(prompt) = prompt {
        text.push_str("The filter's description before the steps: ");
        text.push_str(prompt);
        text.push_str("\n\n");
    }
    let requests: Vec<&str> = requests.iter().map(String::as_str).collect();
    section(&mut text, "The steps, oldest first:", &requests);
    if !pattern.is_empty() {
        text.push_str("The regular expression the steps gave: ");
        text.push_str(pattern);
        text.push_str("\n\n");
    }
    phrase_section(&mut text, phrases);
    text.push_str("Write the one sentence now.\n");
    text
}

/// The prompt in the model's answer (#322): its `prompt:` line, or its
/// first line when it has none, out of the quotes the model put round it.
pub fn parse_prompt(answer: &str) -> Result<String, String> {
    let lines = || {
        answer
            .lines()
            .map(|line| line.trim().trim_start_matches(['-', '*', ' ']))
            .filter(|line| !line.is_empty() && !line.starts_with("```"))
    };
    let labelled = lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .trim_matches('*')
            .eq_ignore_ascii_case("prompt")
            .then_some(value)
    });
    let prompt = labelled.or_else(|| lines().next()).unwrap_or_default();
    let prompt = unquote(prompt.trim()).trim().to_string();
    if prompt.is_empty() {
        return Err(format!("no prompt in the answer: {:?}", answer.trim()));
    }
    Ok(prompt)
}

/// Why the editor did not take a pattern the model gave (#320).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rejection {
    /// It does not compile, and why.
    Invalid(String),
    /// It gets these marked lines wrong.
    Fails {
        must_match: Vec<String>,
        must_not_match: Vec<String>,
    },
}

impl Rejection {
    /// What the model is told about it, under the pattern.
    fn feedback(&self) -> String {
        match self {
            Self::Invalid(error) => format!("It does not compile: {error}\n\n"),
            Self::Fails {
                must_match,
                must_not_match,
            } => {
                let mut text = String::new();
                let lines = |text: &mut String, heading: &str, lines: &[String]| {
                    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
                    section(text, heading, &lines);
                };
                lines(
                    &mut text,
                    "It does not match these lines, which it must match:",
                    must_match,
                );
                lines(
                    &mut text,
                    "It matches these lines, which it must not match:",
                    must_not_match,
                );
                text
            }
        }
    }

    /// The reason in one line, for the editor's error row.
    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Invalid(error) => format!("does not compile: {error}"),
            Self::Fails {
                must_match,
                must_not_match,
            } => {
                let count = must_match.len() + must_not_match.len();
                let first = must_match.first().or(must_not_match.first());
                let first = first.map_or("", |line| line.trim());
                if count == 1 {
                    format!("fails a mark: {first:?}")
                } else {
                    format!("fails {count} marks, the first: {first:?}")
                }
            }
        }
    }
}

/// Whether `pattern` compiles and gets every marked line right (#320).
pub(crate) fn verify(
    pattern: &str,
    must_match: &[&str],
    must_not_match: &[&str],
) -> Result<(), Rejection> {
    let regex = Regex::new(pattern).map_err(|error| {
        let text = error.to_string();
        let reason = text
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("invalid pattern")
            .trim();
        let reason = reason.strip_prefix("error:").unwrap_or(reason).trim();
        Rejection::Invalid(reason.to_string())
    })?;
    let wrong = |lines: &[&str], matched: bool| -> Vec<String> {
        lines
            .iter()
            .filter(|line| regex.is_match(line) != matched)
            .map(ToString::to_string)
            .collect()
    };
    let must_match = wrong(must_match, true);
    let must_not_match = wrong(must_not_match, false);
    if must_match.is_empty() && must_not_match.is_empty() {
        Ok(())
    } else {
        Err(Rejection::Fails {
            must_match,
            must_not_match,
        })
    }
}

/// The text of a later attempt (#320): the first attempt's `text`, and
/// each pattern the model gave for it, oldest first, with what was wrong.
pub(crate) fn retry_text(text: &str, rejected: &[(String, Rejection)]) -> String {
    let mut out = text.to_string();
    out.push_str("\nYour earlier answers to this request were wrong.\n\n");
    for (pattern, rejection) in rejected {
        out.push_str("Pattern: ");
        out.push_str(pattern);
        out.push('\n');
        out.push_str(&rejection.feedback());
    }
    out.push_str("Write a pattern that does not have these problems.\n");
    out
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
    use super::{Cancel, Candidate, INSTRUCTIONS, Model, PROMPT_INSTRUCTIONS};
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
            super::parse_answer(&self.respond(INSTRUCTIONS, text, cancel)?)
        }

        fn consolidate(&self, text: &str, cancel: &Cancel) -> Result<String, String> {
            super::parse_prompt(&self.respond(PROMPT_INSTRUCTIONS, text, cancel)?)
        }
    }

    impl FoundationModel {
        /// The model's answer to `text` under `instructions`.
        fn respond(
            &self,
            instructions: &str,
            text: &str,
            cancel: &Cancel,
        ) -> Result<String, String> {
            // A new session for each request: the request text carries all
            // the context, and a session's cancel handle stops whatever the
            // session runs, so one per request cannot stop a later one.
            let session =
                Session::with_instructions(&self.0, instructions).map_err(|e| e.to_string())?;
            let handle = session.cancellation_handle();
            if !cancel.on_cancel(move || handle.cancel()) {
                return Err("cancelled".to_string());
            }
            let answer = session
                .respond(text, &GenerationOptions::default())
                .map_err(|e| e.to_string())?;
            Ok(answer.content().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_text_has_each_part_under_its_heading() {
        let earlier = ["the timeouts".to_string()];
        let text = request_text(&Parts {
            prompt: Some("timeouts"),
            pattern: Some("timeout"),
            must_match: &["ERROR timeout"],
            must_not_match: &["ERROR timeout DEMO"],
            sample: &["INFO ok"],
            phrases: &[],
            earlier: &earlier,
            request: "also exclude DEMO",
        });
        assert_eq!(
            text,
            "What the lines to match look like: timeouts\n\n\
             Current pattern: timeout\n\n\
             Lines the pattern must match:\nERROR timeout\n\n\
             Lines the pattern must not match:\nERROR timeout DEMO\n\n\
             Other lines of the file:\nINFO ok\n\n\
             Earlier requests, oldest first:\nthe timeouts\n\n\
             Request: also exclude DEMO\n"
        );
    }

    #[test]
    fn an_empty_part_has_no_heading() {
        let text = request_text(&Parts {
            request: "x",
            ..Parts::default()
        });
        assert_eq!(text, "Request: x\n");
    }

    #[test]
    fn only_the_latest_earlier_requests_are_sent() {
        let earlier: Vec<String> = (0..REQUESTS_MAX + 2).map(|n| format!("r{n}")).collect();
        let text = request_text(&Parts {
            earlier: &earlier,
            request: "x",
            ..Parts::default()
        });
        assert!(!text.contains("r1\n"), "{text}");
        assert!(text.contains("r2\n") && text.contains(&format!("r{}\n", REQUESTS_MAX + 1)));
    }

    #[test]
    fn a_long_line_is_cut() {
        let long = "a".repeat(LINE_MAX + 50);
        let text = request_text(&Parts {
            must_match: &[&long],
            request: "x",
            ..Parts::default()
        });
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
    fn a_pattern_that_passes_every_mark_is_verified() {
        assert_eq!(verify("ERROR", &["ERROR x"], &["INFO x"]), Ok(()));
        assert_eq!(verify("x", &[], &[]), Ok(()));
    }

    #[test]
    fn a_pattern_that_does_not_compile_is_rejected_with_the_error() {
        let Err(Rejection::Invalid(error)) = verify("ERROR(", &[], &[]) else {
            panic!("compiled");
        };
        assert_eq!(error, "unclosed group");
        assert!(!error.contains('\n'), "{error}");
    }

    #[test]
    fn a_pattern_that_fails_marks_is_rejected_with_the_lines() {
        let rejection = verify("x", &["a x", "b"], &["c x", "d"]).expect_err("passed");
        assert_eq!(
            rejection,
            Rejection::Fails {
                must_match: vec!["b".to_string()],
                must_not_match: vec!["c x".to_string()],
            }
        );
        assert_eq!(rejection.reason(), "fails 2 marks, the first: \"b\"");
        let one = verify("x", &["b"], &[]).expect_err("passed");
        assert_eq!(one.reason(), "fails a mark: \"b\"");
    }

    #[test]
    fn a_retry_has_the_first_text_and_each_rejected_pattern() {
        let rejected = [
            (
                "a(".to_string(),
                Rejection::Invalid("unclosed group".to_string()),
            ),
            (
                "a".to_string(),
                Rejection::Fails {
                    must_match: vec!["b".to_string()],
                    must_not_match: vec!["a DEMO".to_string()],
                },
            ),
        ];
        assert_eq!(
            retry_text("Request: x\n", &rejected),
            "Request: x\n\n\
             Your earlier answers to this request were wrong.\n\n\
             Pattern: a(\nIt does not compile: unclosed group\n\n\
             Pattern: a\n\
             It does not match these lines, which it must match:\nb\n\n\
             It matches these lines, which it must not match:\na DEMO\n\n\
             Write a pattern that does not have these problems.\n"
        );
    }

    #[test]
    fn a_phrase_mark_goes_to_the_model_with_its_line() {
        let text = request_text(&Parts {
            phrases: &[("timeout", "ERROR timeout")],
            request: "x",
            ..Parts::default()
        });
        assert_eq!(
            text,
            "Important parts of lines, as hints (they are not tests):\n\
             \"timeout\" in the line: ERROR timeout\n\n\
             Request: x\n"
        );
    }

    #[test]
    fn the_consolidate_text_has_the_prompt_the_steps_and_the_pattern() {
        let requests = ["the timeouts".to_string(), "also exclude DEMO".to_string()];
        let text = consolidate_text(
            Some("errors"),
            &requests,
            "timeout",
            &[("DEMO", "ERROR timeout DEMO")],
        );
        assert_eq!(
            text,
            "The filter's description before the steps: errors\n\n\
             The steps, oldest first:\nthe timeouts\nalso exclude DEMO\n\n\
             The regular expression the steps gave: timeout\n\n\
             Important parts of lines, as hints (they are not tests):\n\
             \"DEMO\" in the line: ERROR timeout DEMO\n\n\
             Write the one sentence now.\n"
        );
        let bare = consolidate_text(None, &requests, "", &[]);
        assert!(bare.starts_with("The steps, oldest first:"), "{bare}");
        assert!(!bare.contains("regular expression the steps"), "{bare}");
    }

    #[test]
    fn a_prompt_answer_is_read_with_or_without_its_label() {
        assert_eq!(
            parse_prompt("prompt: Timeouts, except in DEMO runs\n").as_deref(),
            Ok("Timeouts, except in DEMO runs")
        );
        assert_eq!(
            parse_prompt("- **Prompt**: \"Timeouts\"").as_deref(),
            Ok("Timeouts")
        );
        assert_eq!(
            parse_prompt("```\nTimeout lines\n```\n").as_deref(),
            Ok("Timeout lines")
        );
        assert!(parse_prompt("prompt: \"\"").is_err());
        assert!(parse_prompt("  \n").is_err());
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
