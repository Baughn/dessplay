//! The one line editor. Every editable text in the UI — the chat input,
//! modal field editors, the series filter — is a [`LineBuffer`], so the
//! full editing vocabulary (word motion, word kill, line jumps, the
//! horizontal-scroll discipline) exists exactly once and cannot differ
//! between fields.
//!
//! [`LineBuffer`] is pure state (text, cursor, scroll offset) with no
//! rendering; [`TextField`] wraps one in the standard bordered input box.

use tuirealm::event::{Event, Key, KeyModifiers, NoUserEvent};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders, Paragraph};

use super::keys::{plain, typed, word_mod};
use crate::ui::theme;

/// First index at or left of `cursor` that starts the word being left:
/// skip any whitespace to the left, then skip the word to the left.
fn word_boundary_left(chars: &[char], cursor: usize) -> usize {
    let mut i = cursor.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// First index at or right of `cursor` past the next word: skip any
/// whitespace to the right, then skip the word to the right.
fn word_boundary_right(chars: &[char], cursor: usize) -> usize {
    let n = chars.len();
    let mut i = cursor.min(n);
    while i < n && chars[i].is_whitespace() {
        i += 1;
    }
    while i < n && !chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// A single editable line: text, cursor, and horizontal scroll offset.
///
/// Invariants: `cursor <= len` after every operation; `offset <= cursor`
/// and `cursor` visible within the window after [`LineBuffer::scroll`]
/// (which render paths call every draw). Setting or clearing the text
/// resets the scroll — the bug class where a previously-scrolled field
/// rendered the next value from a stale column is unrepresentable here.
#[derive(Clone, Debug, Default)]
pub struct LineBuffer {
    chars: Vec<char>,
    cursor: usize,
    offset: usize,
}

impl LineBuffer {
    /// An empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// A buffer holding `text`, cursor parked at the end.
    pub fn from_text(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let cursor = chars.len();
        Self {
            chars,
            cursor,
            offset: 0,
        }
    }

    /// The buffer contents.
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// Is the buffer empty?
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Length in chars.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Cursor position, in chars (0..=len).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Current horizontal scroll offset (first visible column).
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The buffer as inline spans with a REVERSED cell at the cursor —
    /// for surfacing a filter inside a pane or modal title (where a full
    /// [`TextField`] box doesn't fit). No horizontal scrolling: titles
    /// hold short filter strings.
    pub fn cursor_spans(&self) -> Vec<Span<'static>> {
        let cursor = self.cursor;
        let pre: String = self.chars.iter().take(cursor).collect();
        let at: String = self
            .chars
            .get(cursor)
            .map(|c| c.to_string())
            .unwrap_or_else(|| " ".into());
        let post: String = self.chars.iter().skip(cursor + 1).collect();
        vec![
            Span::raw(pre),
            Span::styled(at, Style::default().add_modifier(Modifier::REVERSED)),
            Span::raw(post),
        ]
    }

    /// Replace the contents; cursor to the end, scroll reset.
    pub fn set_text(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
        self.offset = 0;
    }

    /// Empty the buffer; cursor and scroll reset.
    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
        self.offset = 0;
    }

    /// Insert a character at the cursor.
    pub fn insert(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Delete the character before the cursor. Returns whether anything
    /// was deleted (false on an empty buffer / cursor at start).
    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.chars.remove(self.cursor);
        true
    }

    /// Delete the character after the cursor (the `Delete` key).
    pub fn delete_forward(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    /// Move the cursor one char left.
    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the cursor one char right.
    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.chars.len());
    }

    /// Jump the cursor to the start of the line.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Jump the cursor to the end of the line.
    pub fn end(&mut self) {
        self.cursor = self.chars.len();
    }

    /// Move the cursor left by one word.
    pub fn move_word_left(&mut self) {
        self.cursor = word_boundary_left(&self.chars, self.cursor);
    }

    /// Move the cursor right by one word.
    pub fn move_word_right(&mut self) {
        self.cursor = word_boundary_right(&self.chars, self.cursor);
    }

    /// Delete the word before the cursor (Ctrl-W / Ctrl-Backspace).
    pub fn kill_word_left(&mut self) {
        let target = word_boundary_left(&self.chars, self.cursor);
        self.chars.drain(target..self.cursor);
        self.cursor = target;
    }

    /// Transpose the two characters around the cursor (Ctrl-T), with
    /// readline's semantics: at the end of the line swap the last two
    /// characters; elsewhere swap the character before the cursor with the
    /// one under it and advance, so repeated presses drag a character
    /// rightward. No-op at the start of the line or with fewer than two
    /// characters.
    pub fn transpose_chars(&mut self) {
        if self.cursor == 0 || self.chars.len() < 2 {
            return;
        }
        if self.cursor == self.chars.len() {
            self.chars.swap(self.cursor - 2, self.cursor - 1);
        } else {
            self.chars.swap(self.cursor - 1, self.cursor);
            self.cursor += 1;
        }
    }

    /// Feed one key event through the standard editing vocabulary.
    /// Returns whether the event was consumed. Callers keep their own
    /// semantics for Enter/Esc/Up/Down (submit, cancel, history, list
    /// navigation) by matching those *before* delegating here.
    ///
    /// The vocabulary (see [`super::keys`] for the terminal policy):
    /// typed characters; Backspace/Delete; Left/Right; Home/End and
    /// Ctrl-A/Ctrl-E; word motion via Ctrl/Alt-arrows and Alt-b/Alt-f
    /// (ghostty emits readline bytes for Option-arrow); word kill via
    /// Ctrl-W (Ctrl-only — Alt-W is a typed character on macOS) and
    /// Ctrl/Alt-Backspace; character transpose via Ctrl-T (Ctrl-only,
    /// same macOS reasoning).
    pub fn edit(&mut self, ev: &Event<NoUserEvent>) -> bool {
        if let Some(c) = typed(ev) {
            self.insert(c);
            return true;
        }
        if let Some((key, mods)) = word_mod(ev) {
            match key {
                Key::Left => self.move_word_left(),
                Key::Right => self.move_word_right(),
                Key::Char('b') if mods.contains(KeyModifiers::ALT) => self.move_word_left(),
                Key::Char('f') if mods.contains(KeyModifiers::ALT) => self.move_word_right(),
                Key::Backspace => self.kill_word_left(),
                Key::Char('w') if mods.contains(KeyModifiers::CONTROL) => self.kill_word_left(),
                Key::Char('t') if mods.contains(KeyModifiers::CONTROL) => self.transpose_chars(),
                Key::Char('a') if mods.contains(KeyModifiers::CONTROL) => self.home(),
                Key::Char('e') if mods.contains(KeyModifiers::CONTROL) => self.end(),
                _ => return false,
            }
            return true;
        }
        match plain(ev) {
            Some(Key::Backspace) => {
                self.backspace();
                true
            }
            Some(Key::Delete) => {
                self.delete_forward();
                true
            }
            Some(Key::Left) => {
                self.move_left();
                true
            }
            Some(Key::Right) => {
                self.move_right();
                true
            }
            Some(Key::Home) => {
                self.home();
                true
            }
            Some(Key::End) => {
                self.end();
                true
            }
            _ => false,
        }
    }

    /// Reconcile the scroll offset for a viewport `width` chars wide and
    /// return it. Called by render paths every draw, so the offset can
    /// never be stale: the cursor is always within the window, and the
    /// window never scrolls further than needed to show the end of text.
    pub fn scroll(&mut self, width: usize) -> usize {
        let width = width.max(1);
        // Don't leave the window dangling past the content (text shrank).
        let max_start = (self.chars.len() + 1).saturating_sub(width);
        self.offset = self.offset.min(max_start);
        // Keep the cursor visible: scroll left …
        if self.cursor < self.offset {
            self.offset = self.cursor;
        }
        // … or right.
        if self.cursor >= self.offset + width {
            self.offset = self.cursor + 1 - width;
        }
        self.offset
    }

    /// The visible slice of text for a window reconciled by
    /// [`LineBuffer::scroll`], plus the cursor's column within it.
    fn visible(&self, width: usize) -> (&[char], usize) {
        let width = width.max(1);
        let end = (self.offset + width).min(self.chars.len());
        (&self.chars[self.offset..end], self.cursor - self.offset)
    }
}

/// A [`LineBuffer`] rendered as the standard one-line bordered input box,
/// with a placeholder when empty and a reversed-cell cursor when focused.
#[derive(Clone, Debug, Default)]
pub struct TextField {
    buf: LineBuffer,
    placeholder: &'static str,
}

impl TextField {
    /// An empty field with a placeholder shown while empty.
    pub fn new(placeholder: &'static str) -> Self {
        Self {
            buf: LineBuffer::new(),
            placeholder,
        }
    }

    /// A field prefilled with `text`, cursor at the end.
    pub fn with_text(text: &str) -> Self {
        Self {
            buf: LineBuffer::from_text(text),
            placeholder: "",
        }
    }

    /// The field contents.
    pub fn text(&self) -> String {
        self.buf.text()
    }

    /// Is the field empty?
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Replace the contents; cursor to the end, scroll reset.
    pub fn set_text(&mut self, text: &str) {
        self.buf.set_text(text);
    }

    /// Empty the field; cursor and scroll reset.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Insert a character at the cursor.
    pub fn insert(&mut self, c: char) {
        self.buf.insert(c);
    }

    /// Feed one key event through the editing vocabulary
    /// ([`LineBuffer::edit`]). Returns whether it was consumed.
    pub fn edit(&mut self, ev: &Event<NoUserEvent>) -> bool {
        self.buf.edit(ev)
    }

    /// The underlying buffer (tests, cursor inspection).
    pub fn buffer(&self) -> &LineBuffer {
        &self.buf
    }

    /// Mutable access to the underlying buffer (tests drive
    /// [`LineBuffer::scroll`] directly to simulate a rendered window).
    pub fn buffer_mut(&mut self) -> &mut LineBuffer {
        &mut self.buf
    }

    /// Render as a bordered one-line input. `focused` drives the border
    /// color and whether the cursor cell is shown. `masked` replaces every
    /// character with `*` (password entry).
    pub fn render(&mut self, frame: &mut Frame, area: Rect, focused: bool, masked: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border_style(focused));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let width = inner.width as usize;
        let line = if self.buf.is_empty() && !self.placeholder.is_empty() {
            // Placeholder, dim; the cursor cell (reversed) sits on its
            // first character when focused — where typing would land.
            let text: String = self.placeholder.chars().take(width).collect();
            if focused {
                let mut it = text.chars();
                let head = it.next().map(String::from).unwrap_or_else(|| " ".into());
                let rest: String = it.collect();
                Line::from(vec![
                    Span::styled(head, theme::dim().add_modifier(Modifier::REVERSED)),
                    Span::styled(rest, theme::dim()),
                ])
            } else {
                Line::from(Span::styled(text, theme::dim()))
            }
        } else {
            self.buf.scroll(width);
            let (chars, cursor_col) = self.buf.visible(width);
            let render_char = |c: &char| if masked { '*' } else { *c };
            let pre: String = chars[..cursor_col.min(chars.len())]
                .iter()
                .map(render_char)
                .collect();
            let (at, post): (String, String) = if cursor_col < chars.len() {
                (
                    render_char(&chars[cursor_col]).to_string(),
                    chars[cursor_col + 1..].iter().map(render_char).collect(),
                )
            } else {
                (" ".to_string(), String::new())
            };
            if focused {
                Line::from(vec![
                    Span::raw(pre),
                    Span::styled(at, Style::default().add_modifier(Modifier::REVERSED)),
                    Span::raw(post),
                ])
            } else {
                Line::from(vec![Span::raw(pre), Span::raw(at), Span::raw(post)])
            }
        };
        frame.render_widget(Paragraph::new(line), inner);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use tuirealm::event::KeyEvent;

    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn ctrl(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
        })
    }

    fn alt(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::ALT,
        })
    }

    fn buf(text: &str) -> LineBuffer {
        LineBuffer::from_text(text)
    }

    #[test]
    fn word_boundary_left_cases() {
        let chars: Vec<char> = "the quick".chars().collect();
        assert_eq!(word_boundary_left(&chars, 9), 4); // from end → start of "quick"
        assert_eq!(word_boundary_left(&chars, 4), 0); // from start of "quick" → "the"
        assert_eq!(word_boundary_left(&chars, 0), 0); // clamped
        // Mid-word.
        assert_eq!(word_boundary_left(&chars, 6), 4);
        // Leading spaces: word-left from the end stops at the word start.
        let spaced: Vec<char> = "   hi".chars().collect();
        assert_eq!(word_boundary_left(&spaced, 5), 3);
        // From the word start, skipping the leading spaces reaches 0.
        assert_eq!(word_boundary_left(&spaced, 3), 0);
        // Empty.
        assert_eq!(word_boundary_left(&[], 0), 0);
    }

    #[test]
    fn word_boundary_right_cases() {
        let chars: Vec<char> = "the quick".chars().collect();
        assert_eq!(word_boundary_right(&chars, 0), 3); // → end of "the"
        assert_eq!(word_boundary_right(&chars, 3), 9); // → end of "quick"
        assert_eq!(word_boundary_right(&chars, 9), 9); // clamped
        // Mid-word.
        assert_eq!(word_boundary_right(&chars, 1), 3);
        // Trailing spaces.
        let spaced: Vec<char> = "hi   ".chars().collect();
        assert_eq!(word_boundary_right(&spaced, 0), 2);
        // Empty.
        assert_eq!(word_boundary_right(&[], 0), 0);
    }

    #[test]
    fn insert_and_delete_at_cursor() {
        let mut b = buf("helo");
        b.move_left();
        b.insert('l');
        assert_eq!(b.text(), "hello");
        assert_eq!(b.cursor(), 4);
        b.delete_forward();
        assert_eq!(b.text(), "hell");
        assert!(b.backspace());
        assert_eq!(b.text(), "hel");
        assert_eq!(b.cursor(), 3);
    }

    #[test]
    fn backspace_at_start_is_a_noop() {
        let mut b = buf("hi");
        b.home();
        assert!(!b.backspace());
        assert_eq!(b.text(), "hi");
    }

    #[test]
    fn kill_word_mid_line_keeps_tail() {
        let mut b = buf("the quick brown");
        b.move_word_left(); // cursor at start of "brown"
        b.kill_word_left(); // removes "quick "
        assert_eq!(b.text(), "the brown");
        assert_eq!(b.cursor(), 4);
    }

    #[test]
    fn transpose_matches_readline() {
        // At end of line: swap the last two, cursor stays at the end.
        let mut b = buf("teh");
        b.transpose_chars();
        assert_eq!(b.text(), "the");
        assert_eq!(b.cursor(), 3);
        // Mid-line: swap around the cursor and advance, so repeated
        // presses drag the character rightward.
        let mut b = buf("abcd");
        b.home();
        b.move_right(); // cursor between a and b
        b.transpose_chars();
        assert_eq!(b.text(), "bacd");
        assert_eq!(b.cursor(), 2);
        b.transpose_chars();
        assert_eq!(b.text(), "bcad");
        assert_eq!(b.cursor(), 3);
        // No-ops: cursor at start, or fewer than two characters.
        let mut b = buf("ab");
        b.home();
        b.transpose_chars();
        assert_eq!(b.text(), "ab");
        assert_eq!(b.cursor(), 0);
        let mut b = buf("a");
        b.transpose_chars();
        assert_eq!(b.text(), "a");
    }

    #[test]
    fn edit_handles_the_full_vocabulary() {
        let mut b = buf("the quick brown");
        // Word motion via Ctrl-arrow and Alt-b/f.
        assert!(b.edit(&ctrl(Key::Left)));
        assert_eq!(b.cursor(), 10);
        assert!(b.edit(&alt(Key::Char('b'))));
        assert_eq!(b.cursor(), 4);
        assert!(b.edit(&alt(Key::Char('f'))));
        assert_eq!(b.cursor(), 9);
        // Ctrl-A / Ctrl-E jump to the ends.
        assert!(b.edit(&ctrl(Key::Char('a'))));
        assert_eq!(b.cursor(), 0);
        assert!(b.edit(&ctrl(Key::Char('e'))));
        assert_eq!(b.cursor(), 15);
        // Ctrl-W kills a word; Alt-Backspace too.
        assert!(b.edit(&ctrl(Key::Char('w'))));
        assert_eq!(b.text(), "the quick ");
        assert!(b.edit(&alt(Key::Backspace)));
        assert_eq!(b.text(), "the ");
        // Ctrl-T transposes the two characters before the cursor.
        assert!(b.edit(&ctrl(Key::Char('t'))));
        assert_eq!(b.text(), "th e");
        // Ctrl-B/F are readline char motion, not ours: unconsumed, untyped.
        assert!(!b.edit(&ctrl(Key::Char('b'))));
        assert!(!b.edit(&ctrl(Key::Char('f'))));
        // Alt-W is a typed character on macOS, not a kill: unconsumed here.
        assert!(!b.edit(&alt(Key::Char('w'))));
        // Alt-T likewise: transpose is Ctrl-only.
        assert!(!b.edit(&alt(Key::Char('t'))));
        assert_eq!(b.text(), "th e");
        // Enter / Esc / Up are the caller's business: unconsumed.
        assert!(!b.edit(&key(Key::Enter)));
        assert!(!b.edit(&key(Key::Esc)));
        assert!(!b.edit(&key(Key::Up)));
    }

    #[test]
    fn kitty_extra_modifier_bits_still_move_by_word() {
        let mut b = buf("the quick brown");
        let ev = Event::Keyboard(KeyEvent {
            code: Key::Left,
            modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        });
        assert!(b.edit(&ev));
        assert_eq!(b.cursor(), 10);
    }

    #[test]
    fn scroll_follows_the_cursor_both_ways() {
        let mut b = buf("abcdefghij"); // len 10, cursor 10
        // Narrow window: cursor at end scrolls right.
        assert_eq!(b.scroll(5), 6); // window shows "ghij" + cursor cell
        // Jump home: window snaps back.
        b.home();
        assert_eq!(b.scroll(5), 0);
        // Mid-cursor stays put when already visible.
        b.move_right();
        assert_eq!(b.scroll(5), 0);
    }

    #[test]
    fn set_and_clear_reset_the_scroll() {
        let mut b = buf("a fairly long line that scrolls");
        b.scroll(10);
        assert!(b.offset() > 0);
        b.set_text("short");
        assert_eq!(b.offset(), 0);
        let mut b = buf("a fairly long line that scrolls");
        b.scroll(10);
        assert!(b.offset() > 0);
        b.clear();
        assert_eq!(b.offset(), 0);
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn scroll_never_dangles_after_shrink() {
        let mut b = buf("abcdefghijklmno");
        b.scroll(5); // offset 11
        // Kill everything from a Home-ward word walk.
        b.kill_word_left();
        assert_eq!(b.text(), "");
        // Reconcile: window must snap back to the (now empty) content.
        assert_eq!(b.scroll(5), 0);
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        /// One arbitrary editing operation.
        #[derive(Debug, Clone)]
        enum Op {
            Insert(char),
            Backspace,
            DeleteForward,
            Left,
            Right,
            Home,
            End,
            WordLeft,
            WordRight,
            KillWord,
            Transpose,
            SetText(String),
            Clear,
        }

        fn op() -> impl Strategy<Value = Op> {
            prop_oneof![
                proptest::char::range(' ', '~').prop_map(Op::Insert),
                Just(Op::Backspace),
                Just(Op::DeleteForward),
                Just(Op::Left),
                Just(Op::Right),
                Just(Op::Home),
                Just(Op::End),
                Just(Op::WordLeft),
                Just(Op::WordRight),
                Just(Op::KillWord),
                Just(Op::Transpose),
                "[ -~]{0,40}".prop_map(Op::SetText),
                Just(Op::Clear),
            ]
        }

        fn apply(b: &mut LineBuffer, op: &Op) {
            match op {
                Op::Insert(c) => b.insert(*c),
                Op::Backspace => {
                    b.backspace();
                }
                Op::DeleteForward => b.delete_forward(),
                Op::Left => b.move_left(),
                Op::Right => b.move_right(),
                Op::Home => b.home(),
                Op::End => b.end(),
                Op::WordLeft => b.move_word_left(),
                Op::WordRight => b.move_word_right(),
                Op::KillWord => b.kill_word_left(),
                Op::Transpose => b.transpose_chars(),
                Op::SetText(t) => b.set_text(t),
                Op::Clear => b.clear(),
            }
        }

        proptest! {
            /// Whatever the editing history, the invariants hold after
            /// every operation: cursor within the text, and once scrolled,
            /// the cursor sits inside the window and the window inside the
            /// content. This is the property whose violation was the
            /// "input renders from a stale column" bug class.
            #[test]
            fn cursor_and_scroll_invariants(
                ops in proptest::collection::vec(op(), 0..60),
                width in 1usize..40,
            ) {
                let mut b = LineBuffer::new();
                for op in &ops {
                    apply(&mut b, op);
                    prop_assert!(b.cursor() <= b.len());
                    let offset = b.scroll(width);
                    prop_assert!(offset <= b.cursor());
                    prop_assert!(b.cursor() < offset + width);
                    prop_assert!(offset <= (b.len() + 1).saturating_sub(width));
                }
            }

            /// The visible slice is exactly the window the offset promises.
            #[test]
            fn visible_matches_scroll(
                text in "[ -~]{0,60}",
                lefts in 0usize..60,
                width in 1usize..30,
            ) {
                let mut b = LineBuffer::from_text(&text);
                for _ in 0..lefts {
                    b.move_left();
                }
                b.scroll(width);
                let (slice, cursor_col) = b.visible(width);
                prop_assert!(slice.len() <= width);
                prop_assert!(cursor_col <= slice.len());
                let full: Vec<char> = text.chars().collect();
                prop_assert_eq!(slice, &full[b.offset()..b.offset() + slice.len()]);
            }
        }
    }
}
