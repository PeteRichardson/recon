//! The filter editor (#38, #312): a full-screen place to write a filter's
//! pattern against the open file, with every line it matches highlighted
//! while it is typed.
//!
//! `f I` opens it on a new, empty pattern, and `f C` on the selected filter's
//! pattern (#313). It covers the whole window, as the set picker does, and
//! takes every key while it is open. Enter adds the pattern to the scratch
//! set exactly as `f i` would, or replaces the selected filter's pattern
//! exactly as `f c` would; Esc changes nothing.
//!
//! Tab moves the focus from the pattern field to the file's lines, where
//! `+` and `-` mark a line that the pattern must match or must not match
//! (#314). Each marked line is a check that passes or fails on each key typed
//! in the pattern.
//!
//! On the lines, `f` and `F` go to the next and previous failed check, `n`
//! and `N` to the next and previous unmarked match, each wrapping at the end
//! of the file, and `u` shows only the
//! matched and the marked lines (#315). The editor draws its own lines, so
//! `u` here is hide mode for the editor alone: the main window's hide mode
//! does not change.
//!
//! `Ctrl-z` and `Ctrl-y` step back and forward through the pattern's
//! versions (#316). See `FilterEditor::record` for when a version is kept.
//!
//! Above the pattern are four more fields (#317): the filter's name, its
//! description, its prompt and its sense. Tab and Shift-Tab move the keys
//! round the ring name, description, prompt, sense, pattern, lines. Enter
//! gives the filter what the fields hold; an empty text field is a key the
//! filter does not have.
//!
//! Enter also keeps the marks on the filter as its examples (#318), and `f C`
//! shows a filter's examples as marks again. An example that is not a line
//! of the open file is drawn after the file's last line, so it is checked,
//! counted and reached by `f` as any mark is. A pattern that fails a check
//! does not go on the filter: Enter names the first failed check instead.
//!
//! With a language model (#319), the panel has a request line under the
//! pattern: `Ctrl-g` moves the keys to it, and Enter there sends the request
//! to the model on a worker thread. The reply's pattern goes in the pattern
//! field as a new version, and its explanation under it, only when it
//! compiles and passes every mark (#320); one that does not goes back to
//! the model with what was wrong, up to `generate::ATTEMPTS` tries. Each
//! request the model answered goes to it again with every later one, with
//! the pattern as it stands, so the pattern improves step by step. Esc
//! cancels a request at any try. Without a model the request line is not
//! there, and nothing else changes. See `crate::generate` for what the
//! model receives.
//!
//! A filter is generated when the model wrote its pattern from its prompt
//! (#321): Enter records the hash of the two as the filter's
//! `generated_from`, and the prompt row says `generated`. A change to the
//! pattern or the prompt makes it an ordinary filter again, and the prompt
//! row says what to do about it; see `Generated`. `Ctrl-r` regenerates the
//! pattern from the prompt and the marks alone, through the same verify
//! loop. Enter refuses a generated filter without a must-match and a
//! must-not-match mark. Nothing here, or anywhere, asks the model for a
//! pattern except `Ctrl-r` and a request.
//!
//! Enter after one or more requests (#322) first asks the model for one
//! prompt with all of their intent: the consolidation step. The prompt
//! field shows the model's prompt, and the keys edit only it. Enter saves
//! the filter with the prompt as it is then, and Esc saves it with the
//! prompt from before the step, at any time, also while the model writes.
//! Either way the session's requests are spent: the prompt holds what they
//! asked for. A session with no request, or one that `Ctrl-r` started
//! again, saves at once.
//!
//! A mouse drag along a line of the file marks a phrase (#322): the part of
//! the line that is important. Phrase marks go to the model with each
//! request, `Ctrl-r` and the consolidation step, as hints. recon never
//! checks them, so they do not change the failed-check count. `=` removes
//! them with the line's mark. They last only while the editor is open.

use super::App;
use super::prompt::SearchPrompt;
use crate::filter::{Details, Example, Sense, generated_hash};
use crate::generate::{self, ATTEMPTS, Candidate, Rejection, Running};
use crossterm::event::{self, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::{Rect, Style};
use regex::Regex;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the typing stops before the pattern as it stands is kept as a
/// version (#316).
pub(super) const VERSION_PAUSE: Duration = Duration::from_secs(1);

/// Shown in the panel when Enter finds no pattern to add.
pub(super) const NO_PATTERN: &str = "type a pattern first";

/// Shown in the panel when Enter on the request line finds no request.
pub(super) const NO_REQUEST: &str = "type a request first";

/// Shown in the panel when `Ctrl-r` finds no prompt to regenerate from.
pub(super) const NO_PROMPT: &str = "type a prompt first";

/// Shown on the status row when `Ctrl-r` finds no model (#321).
pub(super) const NO_MODEL: &str = "no model here to regenerate the pattern; edit it by hand";

/// Shown in the panel when Enter finds a generated filter without an
/// example of each kind (#321, rule 7).
pub(super) const NEEDS_EXAMPLES: &str = "a generated filter needs a must-match and a must-not-match line: mark one of each with + and -";

/// Where the filter editor's keys go (#314), in the order Tab moves
/// through them (#317). Each text field takes the characters typed, and
/// Up/Down scroll the lines under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum EditorFocus {
    /// The filter's name.
    Name,
    /// Why the filter exists, for people.
    Description,
    /// What the filter's lines look like, for a model.
    Prompt,
    /// Include, context or exclude: a choice, not text.
    Sense,
    /// The pattern. Where the editor opens.
    #[default]
    Pattern,
    /// A request to the model (#319). In the ring only with a model.
    Request,
    /// Up/Down move the cursor line, and the mark keys mark it.
    Lines,
}

impl EditorFocus {
    const RING: [Self; 7] = [
        Self::Name,
        Self::Description,
        Self::Prompt,
        Self::Sense,
        Self::Pattern,
        Self::Request,
        Self::Lines,
    ];

    /// Tab: the next in the ring, the lines back to the name. The request
    /// line is passed over without a model.
    fn next(self, model: bool) -> Self {
        self.step(1, model)
    }

    /// Shift-Tab: the previous in the ring.
    fn prev(self, model: bool) -> Self {
        self.step(Self::RING.len() - 1, model)
    }

    fn step(self, by: usize, model: bool) -> Self {
        let at = Self::RING
            .iter()
            .position(|&focus| focus == self)
            .unwrap_or(0);
        let next = Self::RING[(at + by) % Self::RING.len()];
        if next == Self::Request && !model {
            next.step(by, model)
        } else {
            next
        }
    }
}

/// What the status row says while the model writes the prompt (#322).
pub(super) const CONSOLIDATING: &str = "the model writes one prompt from your requests";

/// What the status row says once the model's prompt is in the field.
pub(super) const CONSOLIDATED: &str =
    "the model's prompt for the filter: edit it · Enter saves · Esc keeps the prompt as it was";

/// A part of a line the user selected with the mouse (#322): a hint to the
/// model, never a check. `start` and `end` are byte offsets into the line's
/// text, on character boundaries, `start` before `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Phrase {
    pub(super) line: usize,
    pub(super) start: usize,
    pub(super) end: usize,
}

/// The consolidation step (#322): after Enter, before the save.
#[derive(Debug)]
pub(super) struct Consolidation {
    /// The prompt field before the step: what Esc saves.
    earlier: String,
    /// The model writing the prompt, until its answer is here.
    pub(super) running: Option<Running<String>>,
    /// When the model started.
    started: Instant,
}

/// What a marked line says about the pattern (#314).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    /// The pattern must match this line: `+`.
    MustMatch,
    /// The pattern must not match this line: `-`.
    MustNotMatch,
}

/// The prompt row's word for a filter the model wrote from its prompt
/// (#321). The filter pane shows `GENERATED_MARK` for the same state.
pub(super) const GENERATED: &str = "generated from the prompt";

/// What the prompt row says after a hand edit of a generated pattern
/// (#321, rule 4).
pub(super) const PATTERN_CHANGED: &str =
    "pattern changed by hand: change the prompt to agree, or delete it";

/// What the prompt row says when the prompt of a generated pattern
/// changed (#321, rule 5), with a model and without one.
///
/// With a model it names the key that regenerates, as the keymap in force
/// binds it (#386), and names none when a rebind left it without one.
pub(super) fn prompt_changed(regenerate: Option<&str>) -> String {
    match regenerate {
        Some(key) => format!("not generated from this prompt: {key} regenerates the pattern"),
        None => "not generated from this prompt".to_string(),
    }
}
pub(super) const PROMPT_CHANGED_NO_MODEL: &str =
    "not generated from this prompt: an ordinary filter";

/// Whether the model wrote the pattern from the prompt (#321), as the
/// prompt row shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Generated {
    /// The model wrote this pattern from this prompt.
    Yes,
    /// The model wrote a pattern from this prompt, and the pattern was
    /// changed by hand since (rule 4).
    PatternChanged,
    /// The model wrote this pattern, but not from this prompt (rule 5).
    PromptChanged,
    /// No pattern from the model, or no prompt.
    No,
}

/// A marked line's state under the current pattern: the mark, and whether
/// the pattern does what the mark says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Check {
    pub(super) mark: Mark,
    pub(super) passes: bool,
}

/// What the filter editor holds while it is open.
#[derive(Debug)]
pub(super) struct FilterEditor {
    /// The pattern being typed. A `SearchPrompt` for its line editing only,
    /// so the editor's field and every other prompt move and delete the same
    /// way under the same keys; its `kind`, `origin` and `error` are unused.
    pub(super) field: SearchPrompt,
    /// The filter's name, description and prompt (#317), edited as `field`
    /// is. They have no versions: `Ctrl-z` is the pattern's.
    pub(super) name: SearchPrompt,
    pub(super) description: SearchPrompt,
    pub(super) prompt: SearchPrompt,
    /// What a match does (#317): `Include` for a new filter, as `f i` gives.
    pub(super) sense: Sense,
    /// The file's lines, shared with the file view that read them.
    pub(super) lines: Arc<Vec<String>>,
    /// The filter's examples that are not lines of the file (#318), drawn
    /// after its last line. Line `lines.len() + k` is `extra[k]`: every index
    /// into the lines, in `cursor`, `marks` and the rows, counts them.
    pub(super) extra: Vec<String>,
    /// The regex the highlight uses: the last pattern that compiled, so a
    /// half-typed `(` does not blank the screen. `None` while the pattern is
    /// empty — an empty regex matches every line, and a highlight on every
    /// line says nothing.
    pub(super) regex: Option<Regex>,
    /// Why the pattern as typed does not compile, or why Enter refused it.
    pub(super) error: Option<String>,
    /// How many of `lines` `regex` matches.
    pub(super) matches: usize,
    /// The first line drawn.
    pub(super) top: usize,
    /// How many lines the last render drew, for a page key.
    pub(super) page: usize,
    /// The colour the filter takes, so the highlight shows the line as the
    /// file view will show it after Enter: the next palette colour for a new
    /// filter, the filter's own for one being changed.
    pub(super) style: Style,
    /// The filter Enter changes, by index, or `None` for a new filter. The
    /// editor takes every key while it is open, so nothing can remove the
    /// filter under it; `replace_filter` checks the index anyway.
    pub(super) target: Option<usize>,
    /// Where the keys go: the pattern field or the lines.
    pub(super) focus: EditorFocus,
    /// The cursor line, as an index into `lines`. Drawn and moved only while
    /// `focus` is `Lines`.
    pub(super) cursor: usize,
    /// The other end of a visual-line range, from `V`, or `None` when no
    /// range is open.
    pub(super) anchor: Option<usize>,
    /// The marked lines, by index into `lines`.
    pub(super) marks: BTreeMap<usize, Mark>,
    /// How many marks the pattern fails.
    pub(super) failures: usize,
    /// Set when the cursor moved and the next render must scroll it into
    /// view. The render knows the page height; a key does not.
    pub(super) reveal: bool,
    /// `u` (#315): show only the lines the pattern matches and the marked
    /// lines.
    pub(super) matches_only: bool,
    /// The lines drawn, by index into `lines`, in file order: `None` when
    /// every line is drawn. `top` and the page keys count in these rows;
    /// `cursor` and `marks` stay line indexes.
    shown: Option<Vec<usize>>,
    /// The pattern's versions, oldest first (#316). Only valid patterns,
    /// and never two the same side by side.
    pub(super) versions: Vec<String>,
    /// Which of `versions` the pattern was last, or `None` before the
    /// first.
    pub(super) version: Option<usize>,
    /// When the pattern was last typed into, while that edit is not yet
    /// kept as a version.
    pub(super) edited_at: Option<Instant>,
    /// Whether a model was available when the editor opened (#319). Without
    /// one there is no request line.
    pub(super) model: bool,
    /// The request line: what the user asks the model for next.
    pub(super) request: SearchPrompt,
    /// The request the model is answering, if one is running.
    pub(super) running: Option<Asking>,
    /// The requests of this session the model answered, oldest first. Each
    /// goes to the model with every later request, so the pattern improves
    /// step by step. They last only while the editor is open.
    pub(super) requests: Vec<String>,
    /// The whole seconds the running request has taken, as the status row
    /// last showed them.
    pub(super) waited: u64,
    /// What the model said about the last pattern it gave.
    pub(super) explanation: Option<String>,
    /// Each prompt, trimmed, and the pattern the model wrote from it
    /// (#321): the filter's own when it opened as generated, and each
    /// pattern the model gave since. See `generated`.
    pub(super) generated: Vec<(String, String)>,
    /// The phrase marks (#322), in the order they were made.
    pub(super) phrases: Vec<Phrase>,
    /// Where the left button went down on a line's text: the line, and the
    /// byte of the character under the pointer. A drag from it selects.
    press: Option<(usize, usize)>,
    /// The phrase a drag has selected so far. The release marks it.
    pub(super) selection: Option<Phrase>,
    /// Where the last render drew the lines, for a mouse event.
    pub(super) lines_area: Rect,
    /// The consolidation step, while it is open (#322).
    pub(super) consolidation: Option<Consolidation>,
}

/// A request the model is answering (#319), through each attempt of its
/// verify loop (#320). Dropped, it cancels the attempt that runs.
#[derive(Debug)]
pub(super) struct Asking {
    /// The attempt that runs now.
    running: Running,
    /// The request, as the request line had it, or `None` for `Ctrl-r`,
    /// which regenerates from the prompt alone (#321).
    request: Option<String>,
    /// The prompt field, trimmed, when the request was sent: what the
    /// pattern the model gives is generated from.
    prompt: String,
    /// What the first attempt sent. A later attempt sends it again, with
    /// what was wrong with each pattern before.
    text: String,
    /// Which attempt runs now, from 1 to `ATTEMPTS`.
    pub(super) attempt: usize,
    /// Each pattern the model gave that failed, oldest first, and why.
    rejected: Vec<(String, Rejection)>,
    /// When the first attempt started.
    started: Instant,
}

/// What a key on the sense field asks for (#317).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SenseKey {
    Next,
    Prev,
    Choose(Sense),
}

/// Which way a jump key looks from the cursor line (#315).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Down,
    Up,
}

/// What a jump key looks for (#315).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// A marked line the pattern gets wrong.
    Failure,
    /// A line the pattern matches that has no mark.
    Unmarked,
}

impl FilterEditor {
    /// An editor over `lines`, with an empty pattern.
    pub(super) fn new(lines: Arc<Vec<String>>, style: Style) -> Self {
        Self {
            field: SearchPrompt::default(),
            name: SearchPrompt::default(),
            description: SearchPrompt::default(),
            prompt: SearchPrompt::default(),
            sense: Sense::Include,
            lines,
            extra: Vec::new(),
            regex: None,
            error: None,
            matches: 0,
            top: 0,
            page: 1,
            style,
            target: None,
            focus: EditorFocus::default(),
            cursor: 0,
            anchor: None,
            marks: BTreeMap::new(),
            failures: 0,
            reveal: false,
            matches_only: false,
            shown: None,
            versions: Vec::new(),
            version: None,
            edited_at: None,
            model: false,
            request: SearchPrompt::default(),
            running: None,
            requests: Vec::new(),
            waited: 0,
            explanation: None,
            generated: Vec::new(),
            phrases: Vec::new(),
            press: None,
            selection: None,
            lines_area: Rect::default(),
            consolidation: None,
        }
    }

    /// An editor over `lines` on the pattern of the filter at `index`, cursor
    /// at its end as `c` puts it, with the highlight and the count already
    /// showing, and the filter's name, description and prompt in their
    /// fields, and its sense. Its examples are marks (#318): on each line of
    /// the file that has an example's text, or on a line of `extra` for an
    /// example the file does not have.
    pub(super) fn editing(
        lines: Arc<Vec<String>>,
        style: Style,
        index: usize,
        pattern: String,
        details: Details,
        sense: Sense,
    ) -> Self {
        let field = |text: Option<String>| {
            SearchPrompt::editing(
                text.unwrap_or_default(),
                super::prompt::PromptKind::default(),
            )
        };
        // Generated when the hash agrees with the prompt and the pattern
        // as they are (#321), the same test the filter pane makes.
        let generated = match (&details.generated_from, &details.prompt) {
            (Some(hash), Some(prompt)) if *hash == generated_hash(prompt, &pattern) => {
                vec![(prompt.trim().to_string(), pattern.clone())]
            }
            _ => Vec::new(),
        };
        let mut editor = Self {
            field: field(Some(pattern)),
            name: field(details.name),
            description: field(details.description),
            prompt: field(details.prompt),
            sense,
            target: Some(index),
            generated,
            ..Self::new(lines, style)
        };
        editor.mark_examples(details.examples);
        editor.recompile();
        // The pattern it came with is the first version: it did not come
        // from the keyboard.
        editor.record();
        editor
    }

    /// Put a mark on each line with an example's text, and draw each
    /// example the file does not have after its last line.
    fn mark_examples(&mut self, examples: Vec<Example>) {
        if examples.is_empty() {
            return;
        }
        let mut found = vec![false; examples.len()];
        for (index, line) in self.lines.iter().enumerate() {
            for (at, example) in examples.iter().enumerate() {
                if example.line == *line {
                    found[at] = true;
                    self.marks.insert(index, mark_of(example));
                }
            }
        }
        for (example, found) in examples.into_iter().zip(found) {
            if !found {
                let index = self.lines.len() + self.extra.len();
                self.marks.insert(index, mark_of(&example));
                self.extra.push(example.line);
            }
        }
    }

    /// The text of line `index`: a line of the file, or of `extra` past
    /// its end.
    pub(super) fn text(&self, index: usize) -> Option<&str> {
        match index.checked_sub(self.lines.len()) {
            None => self.lines.get(index).map(String::as_str),
            Some(at) => self.extra.get(at).map(String::as_str),
        }
    }

    /// How many lines there are: the file's and `extra`.
    fn total(&self) -> usize {
        self.lines.len() + self.extra.len()
    }

    /// What the name, description and prompt fields hold, trimmed, the
    /// marks as examples (#318), and the hash of the prompt and the pattern
    /// when the model wrote the one from the other (#321). An empty field
    /// is `None`: the filter does not have that key. A line the file has
    /// twice is one example.
    pub(super) fn details(&self) -> Details {
        let text = |field: &SearchPrompt| {
            let text = field.pattern.trim();
            (!text.is_empty()).then(|| text.to_string())
        };
        let mut examples: Vec<Example> = Vec::new();
        for (&index, &mark) in &self.marks {
            let Some(line) = self.text(index) else {
                continue;
            };
            if !examples.iter().any(|example| example.line == line) {
                examples.push(Example {
                    line: line.to_string(),
                    must_match: mark == Mark::MustMatch,
                });
            }
        }
        let prompt = text(&self.prompt);
        let generated_from = (self.generated() == Generated::Yes)
            .then(|| {
                prompt
                    .as_deref()
                    .map(|p| generated_hash(p, &self.field.pattern))
            })
            .flatten();
        Details {
            name: text(&self.name),
            description: text(&self.description),
            prompt,
            examples,
            generated_from,
        }
    }

    /// Whether the model wrote the pattern from the prompt (#321). A
    /// pattern with no prompt is never generated: there is nothing to
    /// regenerate it from.
    pub(super) fn generated(&self) -> Generated {
        let prompt = self.prompt.pattern.trim();
        let pattern = self.field.pattern.as_str();
        if prompt.is_empty() {
            return Generated::No;
        }
        if self
            .generated
            .iter()
            .any(|(p, q)| p == prompt && q == pattern)
        {
            Generated::Yes
        } else if self.generated.iter().any(|(p, _)| p == prompt) {
            Generated::PatternChanged
        } else if self.generated.iter().any(|(_, q)| q == pattern) {
            Generated::PromptChanged
        } else {
            Generated::No
        }
    }

    /// Why Enter refuses a generated filter (#321, rule 7): it has no
    /// must-match mark or no must-not-match mark. The model needs both to
    /// regenerate the pattern, and they are the test of what it gives.
    fn missing_examples(&self) -> Option<&'static str> {
        if self.generated() != Generated::Yes {
            return None;
        }
        let examples = self.details().examples;
        let has = |must_match| examples.iter().any(|e| e.must_match == must_match);
        (!has(true) || !has(false)).then_some(NEEDS_EXAMPLES)
    }

    /// Why Enter refuses the pattern while a check fails (#318): how many
    /// fail, and the text of the first. `None` when every check passes.
    fn failed_checks(&self) -> Option<String> {
        let first = self
            .marks
            .keys()
            .find(|&&index| self.check(index).is_some_and(|check| !check.passes))?;
        let line = self.text(*first).unwrap_or_default();
        Some(failure_message(self.failures, line))
    }

    /// The field the keys go to when it is not the pattern, which is the
    /// only one with versions.
    fn detail_field(&mut self) -> Option<&mut SearchPrompt> {
        match self.focus {
            EditorFocus::Name => Some(&mut self.name),
            EditorFocus::Description => Some(&mut self.description),
            EditorFocus::Prompt => Some(&mut self.prompt),
            EditorFocus::Request => Some(&mut self.request),
            EditorFocus::Sense | EditorFocus::Pattern | EditorFocus::Lines => None,
        }
    }

    /// Compile the pattern again after an edit, and count what it matches.
    ///
    /// A pattern that does not compile keeps the last highlight and count:
    /// the user is in the middle of typing, and the screen should not flash
    /// empty between `(` and `)`.
    fn recompile(&mut self) {
        let pattern = self.field.pattern.as_str();
        if pattern.is_empty() {
            self.regex = None;
            self.error = None;
            self.matches = 0;
            self.refresh();
            return;
        }
        match Regex::new(pattern) {
            Ok(regex) => {
                self.matches = self
                    .lines
                    .iter()
                    .filter(|line| regex.is_match(line))
                    .count();
                self.regex = Some(regex);
                self.error = None;
                self.refresh();
            }
            Err(error) => self.error = Some(error_line(&error)),
        }
    }

    /// Whether the highlight's pattern matches line `index`. No pattern
    /// matches nothing, as no line is highlighted.
    fn matches_line(&self, index: usize) -> bool {
        self.regex
            .as_ref()
            .zip(self.text(index))
            .is_some_and(|(regex, line)| regex.is_match(line))
    }

    /// Line `index`'s check, or `None` when it has no mark.
    pub(super) fn check(&self, index: usize) -> Option<Check> {
        let mark = *self.marks.get(&index)?;
        let matched = self.matches_line(index);
        let passes = match mark {
            Mark::MustMatch => matched,
            Mark::MustNotMatch => !matched,
        };
        Some(Check { mark, passes })
    }

    /// Count the checks the pattern fails, and work out again which lines
    /// are drawn, after the pattern, a mark or `matches_only` changed.
    fn refresh(&mut self) {
        self.failures = self
            .marks
            .keys()
            .filter(|&&index| self.check(index).is_some_and(|check| !check.passes))
            .count();
        self.refresh_shown();
    }

    /// Work out which lines `matches_only` leaves. With no pattern every
    /// line is drawn, as no line is highlighted to keep.
    ///
    /// The first line drawn stays first where it is still drawn, and the
    /// cursor line, once hidden, moves to the next line drawn.
    fn refresh_shown(&mut self) {
        let first = self.line_at(self.top);
        self.shown = (self.matches_only && self.regex.is_some()).then(|| {
            (0..self.total())
                .filter(|&index| self.marks.contains_key(&index) || self.matches_line(index))
                .collect()
        });
        self.top = first.map_or(0, |line| self.row_of(line));
        if !self.is_shown(self.cursor)
            && let Some(line) = self.line_at(self.row_of(self.cursor))
        {
            self.cursor = line;
        }
    }

    /// How many lines are drawn.
    pub(super) fn rows(&self) -> usize {
        self.shown.as_ref().map_or(self.total(), Vec::len)
    }

    /// The line drawn at `row`, or `None` past the last one.
    pub(super) fn line_at(&self, row: usize) -> Option<usize> {
        match &self.shown {
            None => (row < self.total()).then_some(row),
            Some(shown) => shown.get(row).copied(),
        }
    }

    /// The row of `line`, or of the first line drawn after it when it is
    /// hidden: the last row when none is.
    fn row_of(&self, line: usize) -> usize {
        match &self.shown {
            None => line,
            Some(shown) => shown
                .partition_point(|&drawn| drawn < line)
                .min(shown.len().saturating_sub(1)),
        }
    }

    /// Whether line `index` is drawn.
    fn is_shown(&self, index: usize) -> bool {
        self.shown
            .as_ref()
            .is_none_or(|shown| shown.binary_search(&index).is_ok())
    }

    /// Put `mark` on the cursor line, or on every line drawn in the visual
    /// range and close the range. `None` removes the mark.
    fn set_mark(&mut self, mark: Option<Mark>) {
        let (first, last) = self.range();
        let drawn: Vec<usize> = (first..=last)
            .filter(|&index| self.is_shown(index))
            .collect();
        for index in drawn {
            if let Some(mark) = mark {
                self.marks.insert(index, mark);
            } else {
                self.marks.remove(&index);
                self.phrases.retain(|phrase| phrase.line != index);
            }
        }
        self.anchor = None;
        self.refresh();
        // A refusal of Enter may name the mark just changed (#318).
        self.error = pattern_error(&self.field.pattern);
    }

    /// `u`: show only the matched and the marked lines, or every line again.
    fn toggle_matches_only(&mut self) {
        self.matches_only = !self.matches_only;
        self.refresh_shown();
        self.reveal = true;
    }

    /// `f`, `F`, `n` and `N`: move the cursor line to the nearest `target`
    /// line in `direction`, and wrap once past the end of the file as `n`
    /// and `N` do in the file view. The text is what the status row says: a
    /// wrap, or that the file has no `target` line.
    fn jump(&mut self, target: Target, direction: Direction) -> Option<&'static str> {
        let is_target = |index: usize| match target {
            Target::Failure => self.check(index).is_some_and(|check| !check.passes),
            Target::Unmarked => !self.marks.contains_key(&index) && self.matches_line(index),
        };
        let (cursor, len) = (self.cursor, self.total());
        // The cursor line is looked at last, after the wrap: the only
        // target, it is where the jump lands.
        let (before, after): (Vec<usize>, Vec<usize>) = match direction {
            Direction::Down => (
                (cursor + 1..len).collect(),
                (0..(cursor + 1).min(len)).collect(),
            ),
            Direction::Up => ((0..cursor).rev().collect(), (cursor..len).rev().collect()),
        };
        let (line, wrapped) = match before.into_iter().find(|&index| is_target(index)) {
            Some(line) => (line, false),
            None => match after.into_iter().find(|&index| is_target(index)) {
                Some(line) => (line, true),
                None => {
                    return Some(match target {
                        Target::Failure => "no failed check",
                        Target::Unmarked => "no unmarked match",
                    });
                }
            },
        };
        // A failed check is marked and an unmarked match matches, so the
        // line is drawn in either mode.
        self.cursor = line;
        self.reveal = true;
        wrapped.then_some(match direction {
            Direction::Down => super::search::WRAPPED_TO_TOP,
            Direction::Up => super::search::WRAPPED_TO_BOTTOM,
        })
    }

    /// The first and last line of the visual range, or the cursor line twice
    /// when no range is open.
    pub(super) fn range(&self) -> (usize, usize) {
        let anchor = self.anchor.unwrap_or(self.cursor);
        (anchor.min(self.cursor), anchor.max(self.cursor))
    }

    /// Keep the pattern as it stands as a version, if it is valid and not
    /// the version it already is.
    ///
    /// A version is kept when the typing stops, not on each key: when a
    /// key edits the pattern `VERSION_PAUSE` or more after the last edit,
    /// when `Tab` leaves the pattern, and before an undo or a redo. The
    /// pattern must compile and not be empty; a pattern that never compiled
    /// is not a version to go back to.
    fn record(&mut self) {
        self.edited_at = None;
        if self.field.pattern.is_empty() || self.error.is_some() {
            return;
        }
        self.keep_version();
    }

    /// Keep the pattern as it stands as a version, unless it is the version
    /// it already is, and drop the versions an undo left ahead of it.
    fn keep_version(&mut self) {
        let pattern = &self.field.pattern;
        if self
            .version
            .is_some_and(|version| self.versions[version] == *pattern)
        {
            return;
        }
        self.versions.truncate(self.version.map_or(0, |at| at + 1));
        self.versions.push(pattern.clone());
        self.version = Some(self.versions.len() - 1);
    }

    /// The model's reply (#319): its pattern in the field as a new version,
    /// and its explanation under it. The pattern as it was is kept first,
    /// so `Ctrl-z` goes back to it — an empty pattern too, as `Ctrl-z`
    /// must undo what the model did. A pattern that does not compile is not
    /// a version; it shows its error as a typed one does. The request joins
    /// the session's requests, and the request line is cleared for the
    /// next one. A regenerated pattern (#321), with no request, starts the
    /// session's requests again: it was written from the prompt alone.
    /// The pattern is generated from `prompt`.
    fn take_candidate(&mut self, request: Option<String>, prompt: String, candidate: Candidate) {
        self.error = pattern_error(&self.field.pattern);
        if self.field.pattern.is_empty() {
            self.edited_at = None;
            self.keep_version();
        } else {
            self.record();
        }
        self.field = SearchPrompt::editing(candidate.pattern, super::prompt::PromptKind::default());
        self.recompile();
        self.record();
        self.explanation = Some(candidate.explanation);
        self.generated.push((prompt, self.field.pattern.clone()));
        match request {
            Some(request) => {
                self.requests.push(request);
                self.request = SearchPrompt::default();
            }
            None => self.requests.clear(),
        }
    }

    /// The text the model receives for `request` (#319): the prompt field,
    /// the pattern when it compiles, the marks, a sample of the unmarked
    /// lines and the session's earlier requests. Never the description.
    pub(super) fn request_text(&self, request: &str) -> String {
        let examples = self.details().examples;
        let lines = |must_match| marked(&examples, must_match);
        let prompt = self.prompt.pattern.trim();
        let pattern = &self.field.pattern;
        generate::request_text(&generate::Parts {
            prompt: (!prompt.is_empty()).then_some(prompt),
            pattern: (!pattern.is_empty() && pattern_error(pattern).is_none())
                .then_some(pattern.as_str()),
            must_match: &lines(true),
            must_not_match: &lines(false),
            sample: &self.sample(),
            phrases: &self.phrase_texts(),
            earlier: &self.requests,
            request,
        })
    }

    /// The text the model receives for `Ctrl-r` (#321): the prompt, the
    /// marks and a sample of the unmarked lines. Not the pattern and not
    /// the session's requests: the pattern is written again from the
    /// prompt.
    pub(super) fn regenerate_text(&self) -> String {
        let examples = self.details().examples;
        let lines = |must_match| marked(&examples, must_match);
        let prompt = self.prompt.pattern.trim();
        generate::request_text(&generate::Parts {
            prompt: (!prompt.is_empty()).then_some(prompt),
            must_match: &lines(true),
            must_not_match: &lines(false),
            sample: &self.sample(),
            phrases: &self.phrase_texts(),
            request: generate::REGENERATE,
            ..generate::Parts::default()
        })
    }

    /// The text the model receives in the consolidation step (#322).
    pub(super) fn consolidate_text(&self) -> String {
        let prompt = self.prompt.pattern.trim();
        let pattern = &self.field.pattern;
        generate::consolidate_text(
            (!prompt.is_empty()).then_some(prompt),
            &self.requests,
            if pattern_error(pattern).is_none() {
                pattern
            } else {
                ""
            },
            &self.phrase_texts(),
        )
    }

    /// Each phrase mark's text, and the line it is part of (#322).
    fn phrase_texts(&self) -> Vec<(&str, &str)> {
        self.phrases
            .iter()
            .filter_map(|phrase| {
                let line = self.text(phrase.line)?;
                Some((line.get(phrase.start..phrase.end)?, line))
            })
            .collect()
    }

    /// The left button went down at `column`, `row` on the screen. On a
    /// line's text, a drag from here selects a phrase (#322).
    fn press_at(&mut self, column: u16, row: u16) {
        self.selection = None;
        self.press = self.position_at(column, row).and_then(|(line, column)| {
            let at = self.byte_at(line, column)?;
            Some((line, at))
        });
    }

    /// The pointer moved with the button down: select from the press to the
    /// character under the pointer, both in. The phrase stays on the line
    /// of the press: a pointer on another row, or past the text's end,
    /// selects to the column it is at.
    fn drag_to(&mut self, column: u16) {
        let Some((line, pressed)) = self.press else {
            return;
        };
        let Some(text) = self.text(line) else {
            return;
        };
        let column = self.text_column(line, column).unwrap_or(0);
        let at = self.byte_at(line, Some(column)).unwrap_or(text.len());
        let (first, last) = (pressed.min(at), pressed.max(at));
        let end = text[last..]
            .chars()
            .next()
            .map_or(text.len(), |c| last + c.len_utf8());
        self.selection = (first < end).then_some(Phrase {
            line,
            start: first,
            end,
        });
    }

    /// The button came up: a drag's selection is a phrase mark. One that
    /// overlaps a phrase mark of the line takes its place.
    fn release(&mut self) {
        self.press = None;
        let Some(new) = self.selection.take() else {
            return;
        };
        self.phrases.retain(|phrase| {
            phrase.line != new.line || phrase.end <= new.start || new.end <= phrase.start
        });
        self.phrases.push(new);
    }

    /// The line and the column of its text under `column`, `row` on the
    /// screen: `None` off the lines, and no column over the gutter.
    fn position_at(&self, column: u16, row: u16) -> Option<(usize, Option<usize>)> {
        let area = self.lines_area;
        if !area.contains(ratatui::layout::Position::new(column, row)) {
            return None;
        }
        let line = self.line_at(self.top + usize::from(row - area.y))?;
        Some((line, self.text_column(line, column)))
    }

    /// The byte of the character at display column `column` of line
    /// `line`'s text, or `None` past its end.
    fn byte_at(&self, line: usize, column: Option<usize>) -> Option<usize> {
        let column = column?;
        let mut at = 0;
        for (byte, c) in self.text(line)?.char_indices() {
            let width = if c == '\t' {
                TAB_WIDTH
            } else {
                unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
            };
            if column < at + width {
                return Some(byte);
            }
            at += width;
        }
        None
    }

    /// Whether `pattern` compiles and gets every mark right (#320).
    fn verify(&self, pattern: &str) -> Result<(), Rejection> {
        let examples = self.details().examples;
        let lines = |must_match| marked(&examples, must_match);
        generate::verify(pattern, &lines(true), &lines(false))
    }

    /// Up to `SAMPLE_LINES` lines of the file with no mark and some text,
    /// spread from its start to its end.
    fn sample(&self) -> Vec<&str> {
        let step = (self.lines.len() / generate::SAMPLE_LINES).max(1);
        (0..self.lines.len())
            .step_by(step)
            .filter(|index| !self.marks.contains_key(index))
            .map(|index| self.lines[index].as_str())
            .filter(|line| !line.trim().is_empty())
            .take(generate::SAMPLE_LINES)
            .collect()
    }

    /// A key that edits the pattern, at `now`. The pattern before it is a
    /// version if the typing had stopped; an edit that changes the pattern
    /// removes the versions an undo left ahead of it.
    pub(super) fn edit_pattern(&mut self, now: Instant, edit: impl FnOnce(&mut SearchPrompt)) {
        if self
            .edited_at
            .is_some_and(|at| now.saturating_duration_since(at) >= VERSION_PAUSE)
        {
            self.record();
        }
        let before = self.field.pattern.clone();
        edit(&mut self.field);
        if self.field.pattern == before {
            return;
        }
        self.versions.truncate(self.version.map_or(0, |at| at + 1));
        self.edited_at = Some(now);
        self.recompile();
    }

    /// `Ctrl-z`: go back to the previous version. The pattern as typed is
    /// kept first, so `Ctrl-y` comes back to it; a pattern that does not
    /// compile goes back to the last version.
    fn undo(&mut self) -> Option<&'static str> {
        self.record();
        let Some(at) = self.version else {
            return Some("no older version of the pattern");
        };
        let target = if self.versions[at] == self.field.pattern {
            match at.checked_sub(1) {
                Some(target) => target,
                None => return Some("no older version of the pattern"),
            }
        } else {
            at
        };
        self.go_to_version(target);
        None
    }

    /// `Ctrl-y`: go forward to the version an undo left.
    fn redo(&mut self) -> Option<&'static str> {
        self.record();
        match self.version.map(|at| at + 1) {
            Some(target) if target < self.versions.len() => {
                self.go_to_version(target);
                None
            }
            _ => Some("no newer version of the pattern"),
        }
    }

    /// Put version `index` in the field, cursor at its end, with its
    /// highlight and counts.
    fn go_to_version(&mut self, index: usize) {
        self.version = Some(index);
        self.field = SearchPrompt::editing(
            self.versions[index].clone(),
            super::prompt::PromptKind::default(),
        );
        self.edited_at = None;
        self.recompile();
    }

    /// Move the first row drawn by `delta` rows, kept inside the file.
    fn scroll(&mut self, delta: isize) {
        let last = self.rows().saturating_sub(1);
        self.top = self.top.saturating_add_signed(delta).min(last);
    }

    /// Up/Down and the page keys: scroll in a field, and move the cursor
    /// line in the lines.
    fn step(&mut self, delta: isize) {
        match self.focus {
            EditorFocus::Name
            | EditorFocus::Description
            | EditorFocus::Prompt
            | EditorFocus::Sense
            | EditorFocus::Pattern
            | EditorFocus::Request => self.scroll(delta),
            EditorFocus::Lines => {
                let last = self.rows().saturating_sub(1);
                let row = self
                    .row_of(self.cursor)
                    .saturating_add_signed(delta)
                    .min(last);
                if let Some(line) = self.line_at(row) {
                    self.cursor = line;
                }
                self.reveal = true;
            }
        }
    }

    /// Tab (`forward`) or Shift-Tab: move the focus round the ring. A
    /// cursor line off the screen comes back to the first line drawn when
    /// the lines take the keys, so the first mark lands where the user is
    /// looking. Leaving the pattern keeps it as a version; leaving the lines
    /// closes a visual range.
    fn move_focus(&mut self, forward: bool) {
        let next = if forward {
            self.focus.next(self.model)
        } else {
            self.focus.prev(self.model)
        };
        match self.focus {
            EditorFocus::Pattern => self.record(),
            EditorFocus::Lines => self.anchor = None,
            _ => {}
        }
        if next == EditorFocus::Lines {
            let row = self.row_of(self.cursor);
            if (row < self.top || row >= self.top + self.page)
                && let Some(line) = self.line_at(self.top)
            {
                self.cursor = line;
            }
        }
        self.focus = next;
    }

    /// A key on the sense field: Space and Right choose the next sense,
    /// Left the previous, in the order include, context, exclude, and `i`,
    /// `c` and `x` choose include, context and exclude. Any other key does
    /// nothing.
    fn sense_key(&mut self, key: SenseKey) {
        const ORDER: [Sense; 3] = [Sense::Include, Sense::Context, Sense::Exclude];
        let at = ORDER
            .iter()
            .position(|&sense| sense == self.sense)
            .unwrap_or(0);
        self.sense = match key {
            SenseKey::Next => ORDER[(at + 1) % ORDER.len()],
            SenseKey::Prev => ORDER[(at + ORDER.len() - 1) % ORDER.len()],
            SenseKey::Choose(sense) => sense,
        };
    }

    /// Scroll so the cursor line is on the screen, if a key moved it.
    pub(super) fn reveal_cursor(&mut self) {
        if !std::mem::take(&mut self.reveal) {
            return;
        }
        let row = self.row_of(self.cursor);
        if row < self.top {
            self.top = row;
        } else if row >= self.top + self.page {
            self.top = row + 1 - self.page;
        }
    }

    /// What the status row shows: how many lines the pattern matches, how
    /// many checks it fails once a line is marked, and whether only the
    /// matches are drawn.
    pub(super) fn status(&self) -> String {
        let mut status = self.counts();
        if self.matches_only {
            status.push_str(" · matches only");
        }
        if let Some(asking) = &self.running {
            let _ = write!(
                status,
                " · asking the model, try {} of {ATTEMPTS}, {} s · Esc cancels",
                asking.attempt, self.waited
            );
        }
        match &self.consolidation {
            Some(Consolidation {
                running: Some(_), ..
            }) => {
                let _ = write!(
                    status,
                    " · {CONSOLIDATING}, {} s · Esc keeps the prompt as it was",
                    self.waited
                );
            }
            Some(_) => {
                let _ = write!(status, " · {CONSOLIDATED}");
            }
            None => {}
        }
        status
    }

    fn counts(&self) -> String {
        let total = grouped(self.lines.len());
        let count = if self.regex.is_none() {
            format!("{total} lines")
        } else {
            format!("{} of {total} lines match", grouped(self.matches))
        };
        let count = if self.extra.is_empty() {
            count
        } else {
            let noun = if self.extra.len() == 1 {
                "example"
            } else {
                "examples"
            };
            format!(
                "{count} · {} {noun} not in the file",
                grouped(self.extra.len())
            )
        };
        if self.marks.is_empty() {
            return count;
        }
        let verb = if self.failures == 1 { "fails" } else { "fail" };
        let noun = if self.failures == 1 {
            "check"
        } else {
            "checks"
        };
        format!("{count} · {} {noun} {verb}", grouped(self.failures))
    }
}

/// The one line of a `regex::Error` worth showing in a single row.
///
/// A syntax error's text is several lines: the pattern, a caret under the
/// fault, and `error: <reason>` last. The panel shows the pattern already,
/// so the reason is what is left to say.
fn error_line(error: &regex::Error) -> String {
    let text = error.to_string();
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("invalid pattern")
        .trim()
        .to_string()
}

/// Why `pattern` does not compile, or `None` when it does or is empty.
fn pattern_error(pattern: &str) -> Option<String> {
    if pattern.is_empty() {
        return None;
    }
    Regex::new(pattern).err().map(|error| error_line(&error))
}

/// The lines of `examples` with the mark `must_match` gives.
fn marked(examples: &[Example], must_match: bool) -> Vec<&str> {
    examples
        .iter()
        .filter(|example| example.must_match == must_match)
        .map(|example| example.line.as_str())
        .collect()
}

/// The mark an example is shown with.
fn mark_of(example: &Example) -> Mark {
    if example.must_match {
        Mark::MustMatch
    } else {
        Mark::MustNotMatch
    }
}

/// What Enter says when `failures` checks fail, `line` the first (#318).
fn failure_message(failures: usize, line: &str) -> String {
    let line = line.trim();
    if failures == 1 {
        format!("a check fails; fix the pattern or clear the mark: {line:?}")
    } else {
        format!(
            "{} checks fail; fix the pattern or clear the marks. The first: {line:?}",
            grouped(failures)
        )
    }
}

/// What `f c` says when its pattern fails `failed`, a filter's stored
/// examples (#318).
///
/// `open` is the keys that open the filter in the editor, where the
/// examples are, as the keymap in force binds them (#386); `None` leaves
/// the pointer out rather than naming a key that does nothing.
pub(super) fn failed_examples_message(failed: &[&Example], open: Option<&str>) -> String {
    let line = failed.first().map_or("", |example| example.line.trim());
    let one = open.map_or_else(String::new, |keys| format!("; {keys} shows it"));
    let many = open.map_or_else(String::new, |keys| format!("; {keys} shows them"));
    if failed.len() == 1 {
        format!("fails an example of the filter{one}: {line:?}")
    } else {
        format!(
            "fails {} examples of the filter{many}. The first: {line:?}",
            grouped(failed.len())
        )
    }
}

/// `n` with a comma between each group of three digits: `50,000`.
pub(super) fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// How many columns a tab takes, as the editor draws it.
pub(super) const TAB_WIDTH: usize = 4;

impl FilterEditor {
    /// Start the consolidation step (#322): the keys go to the prompt, and
    /// the model writes one prompt from the session. A request still
    /// running is dropped, as the save would drop it.
    fn consolidate(&mut self, model: Arc<dyn generate::Model>) {
        let text = self.consolidate_text();
        self.running = None;
        self.consolidation = Some(Consolidation {
            earlier: self.prompt.pattern.clone(),
            running: Some(Running::consolidate(model, text)),
            started: Instant::now(),
        });
        self.waited = 0;
        self.error = None;
        self.anchor = None;
        self.press = None;
        self.selection = None;
        if self.focus == EditorFocus::Pattern {
            self.record();
        }
        self.focus = EditorFocus::Prompt;
    }

    /// Close the consolidation step (#322): keep the prompt in the field
    /// with `accept`, or put back the one from before the step. The
    /// session's requests are spent either way. A pattern the model gave
    /// and nobody changed since is generated from the prompt it keeps: the
    /// prompt says what the requests that wrote it asked for.
    fn end_consolidation(&mut self, accept: bool) {
        let Some(step) = self.consolidation.take() else {
            return;
        };
        if accept {
            let prompt = self.prompt.pattern.trim().to_string();
            let pattern = self.field.pattern.clone();
            if !prompt.is_empty() && self.generated.last().is_some_and(|(_, q)| *q == pattern) {
                self.generated.push((prompt, pattern));
            }
        } else {
            self.prompt = SearchPrompt::editing(step.earlier, super::prompt::PromptKind::default());
        }
        self.requests.clear();
    }

    /// Send `text` to `model` on a worker thread for `request`, or for
    /// `Ctrl-r` with `None`, in place of a request still running.
    fn ask(&mut self, model: Arc<dyn generate::Model>, request: Option<String>, text: String) {
        self.running = Some(Asking {
            running: Running::start(model, text.clone()),
            request,
            prompt: self.prompt.pattern.trim().to_string(),
            text,
            attempt: 1,
            rejected: Vec::new(),
            started: Instant::now(),
        });
        self.waited = 0;
        self.explanation = None;
        self.error = pattern_error(&self.field.pattern);
    }
}

impl App<'_> {
    /// `f I`: open the filter editor on the open file, with an empty
    /// pattern.
    ///
    /// A truncated preview is loaded in full first, as `n` and `/` do: the
    /// count is a claim about the whole file.
    pub(super) fn open_filter_editor(&mut self) {
        self.promote_truncated_preview();
        let lines = self.view.source().clone();
        let editor = FilterEditor::new(lines, self.filters.next_style());
        self.show_filter_editor(editor);
    }

    /// Open `editor`: mark the origin line, and ask the model, if this build
    /// has one, whether it can take a request now (#319).
    fn show_filter_editor(&mut self, mut editor: FilterEditor) {
        self.mark_origin_line(&mut editor);
        editor.model = self.model.as_ref().is_some_and(|model| model.available());
        self.filter_editor = Some(editor);
    }

    /// Opened from the file view — `f` pressed there — the file view's
    /// cursor line is what the user was looking at, so it is the first
    /// must-match line (#314). Opened from the filter pane, nothing is
    /// marked. A line an example already marks keeps that mark (#318).
    fn mark_origin_line(&self, editor: &mut FilterEditor) {
        if self.chain_origin != Some(super::Focus::View) {
            return;
        }
        let row = self.view.cursor_visible_row();
        let line = self.document.source_at(row).unwrap_or(row);
        if line >= editor.lines.len() {
            return;
        }
        editor.cursor = line;
        editor.reveal = true;
        editor.marks.entry(line).or_insert(Mark::MustMatch);
        editor.refresh();
    }

    /// `f C`: open the filter editor on the filter at `index` (#313), with
    /// its name, description and prompt (#317). A definition filter has no
    /// pattern to show, and says so instead. A typed filter has no name,
    /// so its name field is empty; a file filter without a `name` has its
    /// pattern as its name, and keeps it when the pattern changes, as `c`
    /// keeps it, so its set's profiles still find it.
    pub(super) fn open_filter_editor_on(&mut self, index: usize) {
        let Some(filter) = self.filters.filters().get(index) else {
            return;
        };
        let Some(regex) = filter.predicate.as_regex() else {
            self.report(
                "a definition filter has no pattern to edit; c turns it into one",
                false,
            );
            return;
        };
        let (pattern, style, sense) = (regex.as_str().to_string(), filter.style, filter.sense);
        let details = Details {
            name: filter.name.clone(),
            description: filter.description.clone(),
            prompt: filter.prompt.clone(),
            examples: filter.examples.clone(),
            generated_from: filter.generated_from.clone(),
        };
        self.promote_truncated_preview();
        let lines = self.view.source().clone();
        let editor = FilterEditor::editing(lines, style, index, pattern, details, sense);
        self.show_filter_editor(editor);
    }

    /// Feed a key to the open filter editor. It takes every key: a key
    /// `Scope::FilterEditor` does not bind is tried as a prompt editing key, and a
    /// character that is neither is typed into the pattern.
    ///
    /// The mark keys (#314) act only on the lines. With the focus on a
    /// field they are typed, since `+` and `-` are pattern characters too;
    /// and with the focus on the lines, a key that would edit a field does
    /// nothing. `Ctrl-z` and `Ctrl-y` are the pattern's versions, so they
    /// do nothing in the name, the description and the prompt (#317); on
    /// the request line they undo what the model gave (#319).
    pub(super) fn handle_filter_editor_key(&mut self, key: event::KeyEvent) {
        use crate::keymap::ActionId as A;
        let pressed = crate::keymap::normalise(key);
        // The consolidation step (#322) has two keys of its own; the rest
        // edit the prompt once the model's prompt is in it.
        let writing = self.filter_editor.as_ref().and_then(|editor| {
            editor
                .consolidation
                .as_ref()
                .map(|step| step.running.is_some())
        });
        if let Some(writing) = writing {
            match self
                .keymap
                .resolve(crate::keymap::Scope::FilterEditor, pressed)
            {
                Some(A::FilterEditorCommit) if !writing => self.finish_consolidation(true),
                Some(A::FilterEditorCommit) => {}
                Some(A::FilterEditorCancel) => self.finish_consolidation(false),
                _ if !writing => self.type_in_filter_editor(key),
                _ => {}
            }
            return;
        }
        let focus = self
            .filter_editor
            .as_ref()
            .map_or(EditorFocus::Pattern, |editor| editor.focus);
        let action = self
            .keymap
            .resolve(crate::keymap::Scope::FilterEditor, pressed)
            .filter(|action| {
                matches!(
                    focus,
                    EditorFocus::Pattern | EditorFocus::Request | EditorFocus::Lines
                ) || !matches!(action, A::FilterEditorUndo | A::FilterEditorRedo)
            })
            .filter(|action| {
                focus == EditorFocus::Lines
                    || !matches!(
                        action,
                        A::FilterEditorMarkMatch
                            | A::FilterEditorMarkNoMatch
                            | A::FilterEditorMarkClear
                            | A::FilterEditorVisualLine
                            | A::FilterEditorFailureNext
                            | A::FilterEditorFailurePrev
                            | A::FilterEditorUnmarkedNext
                            | A::FilterEditorUnmarkedPrev
                            | A::FilterEditorToggleMatchesOnly
                    )
            });
        if let Some(action) = action {
            let page = |editor: &FilterEditor| isize::try_from(editor.page).unwrap_or(isize::MAX);
            match action {
                A::FilterEditorCommit if focus == EditorFocus::Request => self.send_request(),
                A::FilterEditorCommit => self.commit_filter_editor(),
                // Esc stops a running request first (#319); a second Esc
                // does what it does without one.
                A::FilterEditorCancel
                    if self
                        .filter_editor
                        .as_ref()
                        .is_some_and(|editor| editor.running.is_some()) =>
                {
                    // Dropped, it cancels the request, and its reply has
                    // nowhere to go.
                    self.edit_filter_editor(|editor| editor.running = None);
                    self.report("request cancelled", false);
                }
                // Esc closes a visual range first, as it does in the file
                // view; a second Esc closes the editor.
                A::FilterEditorCancel
                    if self
                        .filter_editor
                        .as_ref()
                        .is_some_and(|editor| editor.anchor.is_some()) =>
                {
                    self.edit_filter_editor(|editor| editor.anchor = None);
                }
                A::FilterEditorCancel => {
                    self.filter_editor = None;
                    // As a cancelled `f i` prompt: the chain ends where it is.
                    self.chain_origin = None;
                }
                A::FilterEditorScrollUp => self.edit_filter_editor(|editor| editor.step(-1)),
                A::FilterEditorScrollDown => self.edit_filter_editor(|editor| editor.step(1)),
                A::FilterEditorPageUp => {
                    self.edit_filter_editor(|editor| editor.step(-page(editor)));
                }
                A::FilterEditorPageDown => {
                    self.edit_filter_editor(|editor| editor.step(page(editor)));
                }
                A::FilterEditorFocus => self.edit_filter_editor(|editor| editor.move_focus(true)),
                A::FilterEditorFocusPrev => {
                    self.edit_filter_editor(|editor| editor.move_focus(false));
                }
                A::FilterEditorMarkMatch => {
                    self.edit_filter_editor(|editor| editor.set_mark(Some(Mark::MustMatch)));
                }
                A::FilterEditorMarkNoMatch => {
                    self.edit_filter_editor(|editor| editor.set_mark(Some(Mark::MustNotMatch)));
                }
                A::FilterEditorMarkClear => self.edit_filter_editor(|editor| editor.set_mark(None)),
                A::FilterEditorVisualLine => self.edit_filter_editor(|editor| {
                    editor.anchor = match editor.anchor {
                        Some(_) => None,
                        None => Some(editor.cursor),
                    };
                }),
                A::FilterEditorFailureNext => {
                    self.jump_in_filter_editor(Target::Failure, Direction::Down);
                }
                A::FilterEditorFailurePrev => {
                    self.jump_in_filter_editor(Target::Failure, Direction::Up);
                }
                A::FilterEditorUnmarkedNext => {
                    self.jump_in_filter_editor(Target::Unmarked, Direction::Down);
                }
                A::FilterEditorUnmarkedPrev => {
                    self.jump_in_filter_editor(Target::Unmarked, Direction::Up);
                }
                A::FilterEditorToggleMatchesOnly => {
                    self.edit_filter_editor(FilterEditor::toggle_matches_only);
                }
                A::FilterEditorRegenerate => self.regenerate(),
                A::FilterEditorUndo => self.step_filter_editor_version(FilterEditor::undo),
                A::FilterEditorRedo => self.step_filter_editor_version(FilterEditor::redo),
                // Without a model there is no request line to go to.
                A::FilterEditorRequest => self.edit_filter_editor(|editor| {
                    if editor.model && editor.focus != EditorFocus::Request {
                        if editor.focus == EditorFocus::Pattern {
                            editor.record();
                        }
                        editor.anchor = None;
                        editor.focus = EditorFocus::Request;
                    }
                }),
                // `resolve(Scope::FilterEditor, ..)` answers only with the arms
                // above; matched rather than left to a panic, as in
                // `handle_search_key`.
                _ => {}
            }
            return;
        }
        if focus == EditorFocus::Lines {
            return;
        }
        if focus == EditorFocus::Sense {
            self.handle_sense_key(key);
            return;
        }
        self.type_in_filter_editor(key);
    }

    /// A key no binding of the editor took, in a text field: a prompt
    /// editing key, or a character to type.
    fn type_in_filter_editor(&mut self, key: event::KeyEvent) {
        use crate::keymap::ActionId as A;
        let pressed = crate::keymap::normalise(key);
        if let Some(action) = self.keymap.resolve(crate::keymap::Scope::Prompt, pressed) {
            let edit: fn(&mut SearchPrompt) = match action {
                A::PromptLeft => SearchPrompt::move_left,
                A::PromptRight => SearchPrompt::move_right,
                A::PromptStart => SearchPrompt::move_to_start,
                A::PromptEnd => SearchPrompt::move_to_end,
                // Unlike a prompt, Backspace at the start never closes the
                // editor: the file view it covers is still worth reading.
                A::PromptDeleteBack => |field| {
                    field.delete_before();
                },
                A::PromptDeleteForward => SearchPrompt::delete_at,
                A::PromptDeleteWord => SearchPrompt::delete_word_before,
                A::PromptDeleteStart => SearchPrompt::delete_to_start,
                // Commit and cancel are the editor's own keys, above, and
                // the editor keeps no history.
                _ => return,
            };
            self.edit_filter_editor_field(edit);
            return;
        }
        match key.code {
            // The same three refusals as `handle_search_key`: no modified
            // character is typed, and a pasted newline is dropped.
            KeyCode::Char(_)
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {}
            KeyCode::Char(c) if c == '\n' || c == '\r' => {}
            KeyCode::Char(c) => self.edit_filter_editor_field(|field| field.insert(c)),
            _ => {}
        }
    }

    /// A mouse event in the open editor (#322): a drag along a line of the
    /// file marks a phrase. Any other mouse event does nothing, and nothing
    /// does in the consolidation step.
    pub(super) fn handle_filter_editor_mouse(&mut self, mouse: MouseEvent) {
        let Some(editor) = self.filter_editor.as_mut() else {
            return;
        };
        if editor.consolidation.is_some() {
            return;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => editor.press_at(mouse.column, mouse.row),
            MouseEventKind::Drag(MouseButton::Left) => editor.drag_to(mouse.column),
            MouseEventKind::Up(MouseButton::Left) => editor.release(),
            _ => {}
        }
    }

    /// A key on the sense field. Left and Right are the prompt's keys for
    /// the cursor, so a rebinding of them moves the choice too.
    fn handle_sense_key(&mut self, key: event::KeyEvent) {
        use crate::keymap::ActionId as A;
        let pressed = crate::keymap::normalise(key);
        let choice = match self.keymap.resolve(crate::keymap::Scope::Prompt, pressed) {
            Some(A::PromptRight) => Some(SenseKey::Next),
            Some(A::PromptLeft) => Some(SenseKey::Prev),
            _ if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                None
            }
            _ => match key.code {
                KeyCode::Char(' ') => Some(SenseKey::Next),
                KeyCode::Char('i') => Some(SenseKey::Choose(Sense::Include)),
                KeyCode::Char('c') => Some(SenseKey::Choose(Sense::Context)),
                KeyCode::Char('x') => Some(SenseKey::Choose(Sense::Exclude)),
                _ => None,
            },
        };
        if let Some(choice) = choice {
            self.edit_filter_editor(|editor| editor.sense_key(choice));
        }
    }

    /// An editing key in the field that has the focus: the pattern, with
    /// its versions and highlight, or the name, description or prompt.
    fn edit_filter_editor_field(&mut self, edit: impl FnOnce(&mut SearchPrompt)) {
        let now = Instant::now();
        self.edit_filter_editor(|editor| match editor.detail_field() {
            Some(field) => {
                edit(field);
                // A refused name is fixed here, so its reason goes; a
                // pattern that does not compile still says why.
                editor.error = pattern_error(&editor.field.pattern);
            }
            None => editor.edit_pattern(now, edit),
        });
    }

    fn edit_filter_editor(&mut self, edit: impl FnOnce(&mut FilterEditor)) {
        if let Some(editor) = self.filter_editor.as_mut() {
            edit(editor);
        }
    }

    /// `Ctrl-z` or `Ctrl-y`: step through the versions, and say on the
    /// status row when there is none to step to.
    fn step_filter_editor_version(&mut self, step: fn(&mut FilterEditor) -> Option<&'static str>) {
        if let Some(text) = self.filter_editor.as_mut().and_then(step) {
            self.report(text, false);
        }
    }

    /// Enter on the request line (#319): send the request to the model on a
    /// worker thread, in place of one still running.
    fn send_request(&mut self) {
        let Some(model) = self.model.clone() else {
            return;
        };
        let Some(editor) = self.filter_editor.as_mut() else {
            return;
        };
        let request = editor.request.pattern.trim().to_string();
        if request.is_empty() {
            editor.error = Some(NO_REQUEST.to_string());
            return;
        }
        let text = editor.request_text(&request);
        editor.ask(model, Some(request), text);
    }

    /// `Ctrl-r` (#321): write the pattern again from the prompt and the
    /// marks, through the verify loop, in place of a request still running.
    /// The one way besides a request that the pattern changes by the model,
    /// and only when the user presses it.
    fn regenerate(&mut self) {
        let model = self.model.clone().filter(|_| {
            self.filter_editor
                .as_ref()
                .is_some_and(|editor| editor.model)
        });
        let Some(model) = model else {
            self.report(NO_MODEL, false);
            return;
        };
        let Some(editor) = self.filter_editor.as_mut() else {
            return;
        };
        if editor.prompt.pattern.trim().is_empty() {
            editor.error = Some(NO_PROMPT.to_string());
            return;
        }
        let text = editor.regenerate_text();
        editor.ask(model, None, text);
    }

    /// Take the model's reply, if it is here, and say whether the screen
    /// changed: the reply, or one more second of waiting on the status
    /// row. Runs on the render loop, and never blocks.
    ///
    /// The verify loop (#320): a pattern goes in the field only when it
    /// compiles and gets every marked line right. One that does not goes
    /// back to the model with what was wrong, until `ATTEMPTS` attempts
    /// have failed; then the pattern stays as it is and the error row says
    /// why the last one failed. An error from the model itself ends the
    /// loop at once: the model gave no pattern to correct.
    pub(super) fn drain_request(&mut self) -> bool {
        let model = self.model.clone();
        let Some(editor) = self.filter_editor.as_mut() else {
            return false;
        };
        if let Some(step) = editor.consolidation.as_mut() {
            let Some(running) = step.running.as_ref() else {
                return false;
            };
            let Some(reply) = running.poll() else {
                let waited = step.started.elapsed().as_secs();
                return std::mem::replace(&mut editor.waited, waited) != waited;
            };
            step.running = None;
            match reply {
                Ok(prompt) => {
                    editor.prompt =
                        SearchPrompt::editing(prompt, super::prompt::PromptKind::default());
                }
                // The prompt as it was stays in the field, to edit or save.
                Err(error) => {
                    editor.error = Some(format!(
                        "the model gave no prompt: {error}; Enter saves the prompt as it is"
                    ));
                }
            }
            return true;
        }
        let Some(asking) = editor.running.as_ref() else {
            return false;
        };
        let Some(reply) = asking.running.poll() else {
            let waited = asking.started.elapsed().as_secs();
            return std::mem::replace(&mut editor.waited, waited) != waited;
        };
        let Some(mut asking) = editor.running.take() else {
            return false;
        };
        let candidate = match reply {
            Ok(candidate) => candidate,
            // A request that failed is not kept: the model did not act on it.
            Err(error) => {
                editor.error = Some(format!("the model gave no pattern: {error}"));
                return true;
            }
        };
        let Err(rejection) = editor.verify(&candidate.pattern) else {
            editor.take_candidate(asking.request, asking.prompt, candidate);
            return true;
        };
        let reason = rejection.reason();
        asking.rejected.push((candidate.pattern, rejection));
        match model {
            Some(model) if asking.attempt < ATTEMPTS => {
                asking.attempt += 1;
                let text = generate::retry_text(&asking.text, &asking.rejected);
                asking.running = Running::start(model, text);
                editor.running = Some(asking);
            }
            _ => {
                let pattern = asking.rejected.last().map_or("", |(pattern, _)| pattern);
                editor.error = Some(format!(
                    "no pattern from the model passed in {} tries; the last, {pattern}, {reason}",
                    asking.attempt
                ));
            }
        }
        true
    }

    /// A jump key: move the cursor line, and say on the status row when it
    /// wrapped or found nothing.
    fn jump_in_filter_editor(&mut self, target: Target, direction: Direction) {
        let text = self
            .filter_editor
            .as_mut()
            .and_then(|editor| editor.jump(target, direction));
        if let Some(text) = text {
            self.report(text, false);
        }
    }

    /// Enter: add the pattern as `f i … Enter` would, or change the target
    /// filter's as `f c … Enter` would, give the filter the name,
    /// description, prompt and sense in the fields (#317), the marks as
    /// its examples (#318) and its generated state (#321), and close. A new
    /// excluding filter is added as `f x` adds one. A pattern that is
    /// empty, does not compile or fails a check, a generated filter without
    /// a mark of each kind, or a name another filter in the set has, keeps
    /// the editor open with the reason in the panel.
    ///
    /// After one or more requests, Enter opens the consolidation step
    /// (#322) instead, and the save waits for its Enter or Esc.
    fn commit_filter_editor(&mut self) {
        if !self.filter_editor_ready() {
            return;
        }
        let model = self.model.clone();
        let Some(editor) = self.filter_editor.as_mut() else {
            return;
        };
        if let Some(model) = model
            && editor.model
            && !editor.requests.is_empty()
        {
            editor.consolidate(model);
            return;
        }
        self.save_filter_editor();
    }

    /// Enter or Esc in the consolidation step (#322): keep the model's
    /// prompt, as edited, with `accept`, or the prompt from before, and
    /// save. The refusals are made again: the prompt kept can make the
    /// filter generated, which needs a mark of each kind.
    fn finish_consolidation(&mut self, accept: bool) {
        self.edit_filter_editor(|editor| editor.end_consolidation(accept));
        if self.filter_editor_ready() {
            self.save_filter_editor();
        }
    }

    /// Whether Enter can save: `false`, with the reason in the panel, for a
    /// pattern that is empty, does not compile or fails a check, a
    /// generated filter without a mark of each kind, or a name another
    /// filter in the set has.
    fn filter_editor_ready(&mut self) -> bool {
        let Some(editor) = self.filter_editor.as_mut() else {
            return false;
        };
        if editor.field.pattern.is_empty() {
            editor.error = Some(NO_PATTERN.to_string());
            return false;
        }
        // Worked out again, not read: a refusal from an earlier Enter may
        // still be showing after the mark it named was cleared.
        editor.error = pattern_error(&editor.field.pattern);
        if editor.error.is_some() {
            return false;
        }
        if let Some(message) = editor.failed_checks() {
            editor.error = Some(message);
            return false;
        }
        if let Some(message) = editor.missing_examples() {
            editor.error = Some(message.to_string());
            return false;
        }
        if let Some(name) = editor.details().name
            && self.filters.name_taken(editor.target, &name)
        {
            editor.error = Some(format!("another filter in this set is named {name:?}"));
            return false;
        }
        true
    }

    /// Save what the editor holds, as `commit_filter_editor` says, and
    /// close. `filter_editor_ready` said yes.
    fn save_filter_editor(&mut self) {
        let Some(editor) = self.filter_editor.as_ref() else {
            return;
        };
        let (pattern, target, details, sense) = (
            editor.field.pattern.clone(),
            editor.target,
            editor.details(),
            editor.sense,
        );
        // The details go on before a changed pattern: `set_details` renames
        // the filter in its set's profiles from the name they know it by,
        // which for a filter with no name is its pattern as it was.
        // The sense goes on before the add or the replace, whose re-evaluate
        // is what shows it.
        let outcome = match target {
            None => {
                let added = if sense == Sense::Exclude {
                    self.add_excluding_filter(&pattern)
                } else {
                    self.add_filter(&pattern)
                };
                added.map(|()| {
                    if let Some((index, _)) = self.filters.filters_in(0).last() {
                        self.filters.set_details(index, details);
                        if self.filters.set_sense(index, sense) {
                            self.refresh_view();
                        }
                    }
                })
            }
            Some(index) => {
                self.filters.set_details(index, details);
                self.filters.set_sense(index, sense);
                self.replace_filter(index, &pattern)
            }
        };
        if let Err(error) = outcome {
            if let Some(editor) = self.filter_editor.as_mut() {
                editor.error = Some(error_line(&error));
            }
            return;
        }
        self.filter_editor = None;
        // The same close as a filter prompt's commit: arm the bounce guard
        // (#48), and end the chain where it started.
        self.swallow_next_enter = true;
        self.return_to_chain_origin();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_puts_a_comma_between_each_three_digits() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(50_000), "50,000");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn an_error_shows_its_reason_line() {
        // Through a `String`, so clippy does not refuse the bad literal.
        let pattern = String::from("foo(");
        let error = Regex::new(&pattern).expect_err("unclosed group");
        assert_eq!(error_line(&error), "error: unclosed group");
    }
}
