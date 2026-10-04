//! The `:` command line: its input plus the history.

use crate::text_input::TextInput;

const HISTORY_LIMIT: usize = 100;

#[derive(Debug, Default)]
pub struct CommandLine {
    pub input: TextInput,
    history: Vec<String>,
    /// Entry currently recalled with Up/Down; `None` while typing a new line.
    history_idx: Option<usize>,
}

impl CommandLine {
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
        let line = self.input.take();
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
        self.input.take();
        self.history_idx = None;
    }

    fn recall(&mut self, idx: Option<usize>) {
        self.history_idx = idx;
        let text = idx.map_or("", |i| self.history[i].as_str()).to_string();
        self.input.set(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_and_submit(line: &mut CommandLine, text: &str) {
        text.chars().for_each(|c| line.input.insert(c));
        line.submit();
    }

    #[test]
    fn history_walks_both_ways() {
        let mut line = CommandLine::default();
        type_and_submit(&mut line, "launch a");
        type_and_submit(&mut line, "launch b");

        line.history_prev();
        assert_eq!(line.input.text(), "launch b");
        line.history_prev();
        line.history_prev(); // stays on the oldest
        assert_eq!(line.input.text(), "launch a");
        line.history_next();
        assert_eq!(line.input.text(), "launch b");
        line.history_next();
        assert_eq!(line.input.text(), "");
    }

    #[test]
    fn history_skips_blanks_and_repeats() {
        let mut line = CommandLine::default();
        type_and_submit(&mut line, "  ");
        type_and_submit(&mut line, "help");
        type_and_submit(&mut line, "help");
        line.history_prev();
        line.history_prev();
        assert_eq!(line.input.text(), "help");
        assert_eq!(line.history.len(), 1);
    }
}
