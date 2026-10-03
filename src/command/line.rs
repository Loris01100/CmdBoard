//! Editable text of the `:` command line, with its history.

const HISTORY_LIMIT: usize = 100;

#[derive(Debug, Default)]
pub struct CommandLine {
    pub input: String,
    /// Cursor position, in characters.
    pub cursor: usize,
    history: Vec<String>,
    /// Entry currently recalled with Up/Down; `None` while typing a new line.
    history_idx: Option<usize>,
}

impl CommandLine {
    pub fn insert(&mut self, c: char) {
        let at = self.byte_index();
        self.input.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte_index();
            self.input.remove(at);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte_index();
            self.input.remove(at);
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

    /// Recalls the previous (older) history entry.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_idx {
            None => self.history.len() - 1,
            Some(i) => i.saturating_sub(1),
        };
        self.recall(Some(idx));
    }

    /// Recalls the next (newer) entry, or an empty line past the newest.
    pub fn history_next(&mut self) {
        match self.history_idx {
            Some(i) if i + 1 < self.history.len() => self.recall(Some(i + 1)),
            Some(_) => self.recall(None),
            None => {}
        }
    }

    /// Takes the typed line, records it in the history and clears the input.
    pub fn submit(&mut self) -> String {
        let line = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.history_idx = None;
        let trimmed = line.trim();
        if !trimmed.is_empty() && self.history.last().map(String::as_str) != Some(trimmed) {
            self.history.push(trimmed.to_string());
            if self.history.len() > HISTORY_LIMIT {
                self.history.remove(0);
            }
        }
        line
    }

    /// Abandons the line being typed; the history is kept.
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.history_idx = None;
    }

    fn recall(&mut self, idx: Option<usize>) {
        self.history_idx = idx;
        self.input = idx.map(|i| self.history[i].clone()).unwrap_or_default();
        self.cursor = self.len();
    }

    fn len(&self) -> usize {
        self.input.chars().count()
    }

    fn byte_index(&self) -> usize {
        self.input
            .char_indices()
            .nth(self.cursor)
            .map_or(self.input.len(), |(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> CommandLine {
        let mut line = CommandLine::default();
        text.chars().for_each(|c| line.insert(c));
        line
    }

    #[test]
    fn edits_around_the_cursor_with_accents() {
        let mut line = typed("catégrie");
        line.left();
        line.left();
        line.left();
        line.insert('o');
        assert_eq!(line.input, "catégorie");
        line.home();
        line.delete();
        line.end();
        line.backspace();
        assert_eq!(line.input, "atégori");
        assert_eq!(line.cursor, 7);
    }

    #[test]
    fn history_walks_both_ways() {
        let mut line = typed("launch a");
        line.submit();
        "launch b".chars().for_each(|c| line.insert(c));
        line.submit();

        line.history_prev();
        assert_eq!(line.input, "launch b");
        line.history_prev();
        line.history_prev(); // stays on the oldest
        assert_eq!(line.input, "launch a");
        line.history_next();
        assert_eq!(line.input, "launch b");
        line.history_next();
        assert_eq!(line.input, "");
    }

    #[test]
    fn history_skips_blanks_and_repeats() {
        let mut line = typed("  ");
        line.submit();
        for _ in 0..2 {
            "help".chars().for_each(|c| line.insert(c));
            line.submit();
        }
        line.history_prev();
        line.history_prev();
        assert_eq!(line.input, "help");
        assert_eq!(line.history.len(), 1);
    }
}
