//! Single-line editable text, used by the command line and form fields.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    text: String,
    /// Cursor position, in characters.
    cursor: usize,
}

impl TextInput {
    /// Input holding `text`, cursor at the end.
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Replaces the text, cursor at the end.
    pub fn set(&mut self, text: &str) {
        *self = Self::new(text);
    }

    /// Returns the text and empties the input.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte_index();
        self.text.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte_index();
            self.text.remove(at);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte_index();
            self.text.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    /// Applies an editing key. Returns false when the key is not an editing key.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => self.insert(c),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.home(),
            KeyCode::End => self.end(),
            _ => return false,
        }
        true
    }

    /// The part of the text that fits in `width` columns, scrolled so the cursor stays
    /// visible, and the cursor column within it.
    pub fn view(&self, width: usize) -> (String, usize) {
        if width == 0 {
            return (String::new(), 0);
        }
        let offset = self.cursor.saturating_sub(width - 1);
        let visible = self.text.chars().skip(offset).take(width).collect();
        (visible, self.cursor - offset)
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte_index(&self) -> usize {
        self.text
            .char_indices()
            .nth(self.cursor)
            .map_or(self.text.len(), |(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_around_the_cursor_with_accents() {
        let mut input = TextInput::new("catégrie");
        input.left();
        input.left();
        input.left();
        input.insert('o');
        assert_eq!(input.text(), "catégorie");
        input.home();
        input.delete();
        input.end();
        input.backspace();
        assert_eq!(input.text(), "atégori");
        assert_eq!(input.cursor, 7);
    }

    #[test]
    fn ctrl_keys_are_not_typed() {
        let mut input = TextInput::default();
        assert!(!input.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)));
        assert!(input.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)));
        assert_eq!(input.text(), "S");
    }

    #[test]
    fn view_scrolls_to_cursor() {
        let input = TextInput::new("abcdefghij");
        assert_eq!(input.view(4), ("hij".to_string(), 3));
        assert_eq!(TextInput::new("ab").view(4), ("ab".to_string(), 2));
        assert_eq!(input.view(0), (String::new(), 0));
    }
}
