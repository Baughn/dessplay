/// A simple single-line text input widget with cursor support.
pub struct TextInput {
    text: String,
    /// Cursor position as byte offset into `text`.
    cursor: usize,
}

impl TextInput {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
        }
    }

    pub fn insert(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            // Find previous char boundary
            let prev = self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.text.drain(prev..self.cursor);
            self.cursor = prev;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            let next = self.cursor
                + self.text[self.cursor..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0);
            self.text.drain(self.cursor..next);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor = self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.text.len() {
            self.cursor += self.text[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
        }
    }

    pub fn move_word_left(&mut self) {
        // Skip whitespace to the left, then skip non-whitespace to the left
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        // Find the start of the current word
        self.cursor = trimmed
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + trimmed[i..].chars().next().unwrap().len_utf8())
            .unwrap_or(0);
    }

    pub fn move_word_right(&mut self) {
        let after = &self.text[self.cursor..];
        // Skip non-whitespace, then skip whitespace
        let skip_word = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let rest = &after[skip_word..];
        let skip_space = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        self.cursor += skip_word + skip_space;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Take the current text and reset the input.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Cursor position as character count (for display purposes).
    pub fn cursor_pos(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_take() {
        let mut input = TextInput::new();
        input.insert('h');
        input.insert('i');
        assert_eq!(input.text(), "hi");
        assert_eq!(input.cursor_pos(), 2);
        let text = input.take();
        assert_eq!(text, "hi");
        assert!(input.is_empty());
        assert_eq!(input.cursor_pos(), 0);
    }

    #[test]
    fn backspace() {
        let mut input = TextInput::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.backspace();
        assert_eq!(input.text(), "ab");
        assert_eq!(input.cursor_pos(), 2);
    }

    #[test]
    fn backspace_at_start() {
        let mut input = TextInput::new();
        input.backspace(); // should be no-op
        assert!(input.is_empty());
    }

    #[test]
    fn delete() {
        let mut input = TextInput::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.home();
        input.delete();
        assert_eq!(input.text(), "bc");
        assert_eq!(input.cursor_pos(), 0);
    }

    #[test]
    fn delete_at_end() {
        let mut input = TextInput::new();
        input.insert('a');
        input.delete(); // should be no-op
        assert_eq!(input.text(), "a");
    }

    #[test]
    fn cursor_movement() {
        let mut input = TextInput::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        assert_eq!(input.cursor_pos(), 3);

        input.move_left();
        assert_eq!(input.cursor_pos(), 2);

        input.move_left();
        assert_eq!(input.cursor_pos(), 1);

        input.move_right();
        assert_eq!(input.cursor_pos(), 2);

        input.home();
        assert_eq!(input.cursor_pos(), 0);

        input.end();
        assert_eq!(input.cursor_pos(), 3);
    }

    #[test]
    fn insert_in_middle() {
        let mut input = TextInput::new();
        input.insert('a');
        input.insert('c');
        input.move_left();
        input.insert('b');
        assert_eq!(input.text(), "abc");
        assert_eq!(input.cursor_pos(), 2);
    }

    #[test]
    fn utf8_handling() {
        let mut input = TextInput::new();
        input.insert('å');
        input.insert('ä');
        input.insert('ö');
        assert_eq!(input.text(), "åäö");
        assert_eq!(input.cursor_pos(), 3);

        input.backspace();
        assert_eq!(input.text(), "åä");
        assert_eq!(input.cursor_pos(), 2);

        input.move_left();
        input.delete();
        assert_eq!(input.text(), "å");
    }

    #[test]
    fn move_word_left() {
        let mut input = TextInput::new();
        for c in "hello world foo".chars() {
            input.insert(c);
        }
        // Cursor at end: "hello world foo|"
        input.move_word_left();
        assert_eq!(input.cursor_pos(), 12); // "hello world |foo"
        input.move_word_left();
        assert_eq!(input.cursor_pos(), 6); // "hello |world foo"
        input.move_word_left();
        assert_eq!(input.cursor_pos(), 0); // "|hello world foo"
        input.move_word_left();
        assert_eq!(input.cursor_pos(), 0); // stays at start
    }

    #[test]
    fn move_word_right() {
        let mut input = TextInput::new();
        for c in "hello world foo".chars() {
            input.insert(c);
        }
        input.home();
        // Cursor at start: "|hello world foo"
        input.move_word_right();
        assert_eq!(input.cursor_pos(), 6); // "hello |world foo"
        input.move_word_right();
        assert_eq!(input.cursor_pos(), 12); // "hello world |foo"
        input.move_word_right();
        assert_eq!(input.cursor_pos(), 15); // "hello world foo|"
        input.move_word_right();
        assert_eq!(input.cursor_pos(), 15); // stays at end
    }

    #[test]
    fn move_word_multiple_spaces() {
        let mut input = TextInput::new();
        for c in "a   b".chars() {
            input.insert(c);
        }
        input.home();
        input.move_word_right();
        assert_eq!(input.cursor_pos(), 4); // "a   |b"
        input.move_word_left();
        assert_eq!(input.cursor_pos(), 0); // "|a   b"
    }

    #[test]
    fn clear() {
        let mut input = TextInput::new();
        input.insert('x');
        input.insert('y');
        input.clear();
        assert!(input.is_empty());
        assert_eq!(input.cursor_pos(), 0);
    }
}
