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
//! in the pattern. The marks live only while the editor is open.
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
//! Above the pattern are three more fields (#317): the filter's name, its
//! description and its prompt. Tab and Shift-Tab move the keys round the
//! ring name, description, prompt, pattern, lines. Enter gives the filter
//! what the fields hold; an empty field is a key the filter does not have.

use super::App;
use super::prompt::SearchPrompt;
use crate::filter::Details;
use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::prelude::Style;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the typing stops before the pattern as it stands is kept as a
/// version (#316).
pub(super) const VERSION_PAUSE: Duration = Duration::from_secs(1);

/// Shown in the panel when Enter finds no pattern to add.
pub(super) const NO_PATTERN: &str = "type a pattern first";

/// Where the filter editor's keys go (#314), in the order Tab moves
/// through them (#317). Each field takes the characters typed, and Up/Down
/// scroll the lines under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum EditorFocus {
    /// The filter's name.
    Name,
    /// Why the filter exists, for people.
    Description,
    /// What the filter's lines look like, for a model.
    Prompt,
    /// The pattern. Where the editor opens.
    #[default]
    Pattern,
    /// Up/Down move the cursor line, and the mark keys mark it.
    Lines,
}

impl EditorFocus {
    const RING: [Self; 5] = [
        Self::Name,
        Self::Description,
        Self::Prompt,
        Self::Pattern,
        Self::Lines,
    ];

    /// Tab: the next in the ring, the lines back to the name.
    fn next(self) -> Self {
        let at = Self::RING
            .iter()
            .position(|&focus| focus == self)
            .unwrap_or(0);
        Self::RING[(at + 1) % Self::RING.len()]
    }

    /// Shift-Tab: the previous in the ring.
    fn prev(self) -> Self {
        let at = Self::RING
            .iter()
            .position(|&focus| focus == self)
            .unwrap_or(0);
        Self::RING[(at + Self::RING.len() - 1) % Self::RING.len()]
    }
}

/// What a marked line says about the pattern (#314).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    /// The pattern must match this line: `+`.
    MustMatch,
    /// The pattern must not match this line: `-`.
    MustNotMatch,
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
    /// The file's lines, shared with the file view that read them.
    pub(super) lines: Arc<Vec<String>>,
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
            lines,
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
        }
    }

    /// An editor over `lines` on the pattern of the filter at `index`, cursor
    /// at its end as `c` puts it, with the highlight and the count already
    /// showing, and the filter's name, description and prompt in their
    /// fields.
    pub(super) fn editing(
        lines: Arc<Vec<String>>,
        style: Style,
        index: usize,
        pattern: String,
        details: Details,
    ) -> Self {
        let field = |text: Option<String>| {
            SearchPrompt::editing(
                text.unwrap_or_default(),
                super::prompt::PromptKind::default(),
            )
        };
        let mut editor = Self {
            field: field(Some(pattern)),
            name: field(details.name),
            description: field(details.description),
            prompt: field(details.prompt),
            target: Some(index),
            ..Self::new(lines, style)
        };
        editor.recompile();
        // The pattern it came with is the first version: it did not come
        // from the keyboard.
        editor.record();
        editor
    }

    /// What the name, description and prompt fields hold, trimmed. An empty
    /// field is `None`: the filter does not have that key.
    pub(super) fn details(&self) -> Details {
        let text = |field: &SearchPrompt| {
            let text = field.pattern.trim();
            (!text.is_empty()).then(|| text.to_string())
        };
        Details {
            name: text(&self.name),
            description: text(&self.description),
            prompt: text(&self.prompt),
        }
    }

    /// The field the keys go to when it is not the pattern, which is the
    /// only one with versions.
    fn detail_field(&mut self) -> Option<&mut SearchPrompt> {
        match self.focus {
            EditorFocus::Name => Some(&mut self.name),
            EditorFocus::Description => Some(&mut self.description),
            EditorFocus::Prompt => Some(&mut self.prompt),
            EditorFocus::Pattern | EditorFocus::Lines => None,
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
            .zip(self.lines.get(index))
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
            (0..self.lines.len())
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
        self.shown.as_ref().map_or(self.lines.len(), Vec::len)
    }

    /// The line drawn at `row`, or `None` past the last one.
    pub(super) fn line_at(&self, row: usize) -> Option<usize> {
        match &self.shown {
            None => (row < self.lines.len()).then_some(row),
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
            match mark {
                Some(mark) => self.marks.insert(index, mark),
                None => self.marks.remove(&index),
            };
        }
        self.anchor = None;
        self.refresh();
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
        let (cursor, len) = (self.cursor, self.lines.len());
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
        let pattern = &self.field.pattern;
        if pattern.is_empty()
            || self.error.is_some()
            || self
                .version
                .is_some_and(|version| self.versions[version] == *pattern)
        {
            return;
        }
        self.versions.truncate(self.version.map_or(0, |at| at + 1));
        self.versions.push(pattern.clone());
        self.version = Some(self.versions.len() - 1);
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
            | EditorFocus::Pattern => self.scroll(delta),
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
            self.focus.next()
        } else {
            self.focus.prev()
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
        let status = self.counts();
        if self.matches_only {
            format!("{status} · matches only")
        } else {
            status
        }
    }

    fn counts(&self) -> String {
        let total = grouped(self.lines.len());
        let count = if self.regex.is_none() {
            format!("{total} lines")
        } else {
            format!("{} of {total} lines match", grouped(self.matches))
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

impl App<'_> {
    /// `f I`: open the filter editor on the open file, with an empty
    /// pattern.
    ///
    /// A truncated preview is loaded in full first, as `n` and `/` do: the
    /// count is a claim about the whole file.
    pub(super) fn open_filter_editor(&mut self) {
        self.promote_truncated_preview();
        let lines = self.view.source().clone();
        let mut editor = FilterEditor::new(lines, self.filters.next_style());
        self.mark_origin_line(&mut editor);
        self.filter_editor = Some(editor);
    }

    /// Opened from the file view — `f` pressed there — the file view's
    /// cursor line is what the user was looking at, so it is the first
    /// must-match line (#314). Opened from the filter pane, nothing is
    /// marked.
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
        editor.marks.insert(line, Mark::MustMatch);
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
        let (pattern, style) = (regex.as_str().to_string(), filter.style);
        let details = Details {
            name: filter.name.clone(),
            description: filter.description.clone(),
            prompt: filter.prompt.clone(),
        };
        self.promote_truncated_preview();
        let lines = self.view.source().clone();
        let mut editor = FilterEditor::editing(lines, style, index, pattern, details);
        self.mark_origin_line(&mut editor);
        self.filter_editor = Some(editor);
    }

    /// Feed a key to the open filter editor. It takes every key: a key
    /// `Scope::FilterEditor` does not bind is tried as a prompt editing key, and a
    /// character that is neither is typed into the pattern.
    ///
    /// The mark keys (#314) act only on the lines. With the focus on a
    /// field they are typed, since `+` and `-` are pattern characters too;
    /// and with the focus on the lines, a key that would edit a field does
    /// nothing. `Ctrl-z` and `Ctrl-y` are the pattern's versions, so they
    /// do nothing in the name, the description and the prompt (#317).
    pub(super) fn handle_filter_editor_key(&mut self, key: event::KeyEvent) {
        use crate::keymap::ActionId as A;
        let pressed = crate::keymap::normalise(key);
        let focus = self
            .filter_editor
            .as_ref()
            .map_or(EditorFocus::Pattern, |editor| editor.focus);
        let action = self
            .keymap
            .resolve(crate::keymap::Scope::FilterEditor, pressed)
            .filter(|action| {
                matches!(focus, EditorFocus::Pattern | EditorFocus::Lines)
                    || !matches!(action, A::FilterEditorUndo | A::FilterEditorRedo)
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
                A::FilterEditorCommit => self.commit_filter_editor(),
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
                A::FilterEditorUndo => self.step_filter_editor_version(FilterEditor::undo),
                A::FilterEditorRedo => self.step_filter_editor_version(FilterEditor::redo),
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
    /// description and prompt in the fields (#317), and close. A pattern
    /// that is empty or does not compile, or a name another filter in the
    /// set has, keeps the editor open with the reason in the panel.
    fn commit_filter_editor(&mut self) {
        let Some(editor) = self.filter_editor.as_mut() else {
            return;
        };
        if editor.field.pattern.is_empty() {
            editor.error = Some(NO_PATTERN.to_string());
            return;
        }
        if editor.error.is_some() {
            return;
        }
        let (pattern, target, details) = (
            editor.field.pattern.clone(),
            editor.target,
            editor.details(),
        );
        if let Some(name) = &details.name
            && self.filters.name_taken(target, name)
        {
            editor.error = Some(format!("another filter in this set is named {name:?}"));
            return;
        }
        // The details go on before a changed pattern: `set_details` renames
        // the filter in its set's profiles from the name they know it by,
        // which for a filter with no name is its pattern as it was.
        let outcome = match target {
            None => self.add_filter(&pattern).map(|()| {
                if let Some((index, _)) = self.filters.filters_in(0).last() {
                    self.filters.set_details(index, details);
                }
            }),
            Some(index) => {
                self.filters.set_details(index, details);
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
