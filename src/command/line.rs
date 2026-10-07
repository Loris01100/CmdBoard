//! The `:` command line: its input, the history and Tab completion.

use super::complete::Completion;
use crate::text_input::TextInput;

const HISTORY_LIMIT: usize = 100;

#[derive(Debug, Default)]
pub struct CommandLine {
    pub input: TextInput,
    history: Vec<String>,
    /// Entry currently recalled with Up/Down; `None` while typing a new line.
    history_idx: Option<usize>,
    /// Candidates of the last Tab, and the one shown. Cleared by any other key.
    pub completion: Option<(Completion, usize)>,
}

impl CommandLine {
    /// Tab: completes the line with the first candidate. Pressed again right after,
    /// moves to the next candidate (`forward`) or the previous one.
    pub fn complete(&mut self, compute: impl FnOnce(&str) -> Option<Completion>, forward: bool) {
        if let Some((completion, index)) = &mut self.completion
            && self.input.text() == completion.line(*index)
        {
            let len = completion.candidates.len();
            *index = if forward {
                (*index + 1) % len
            } else {
                (*index + len - 1) % len
            };
            let line = completion.line(*index);
            self.input.set(&line);
            return;
        }
        self.completion = compute(self.input.text()).map(|completion| (completion, 0));
        if let Some((completion, _)) = &self.completion {
            let line = completion.line(0);
            self.input.set(&line);
        }
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
        self.completion = None;
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

    #[test]
    fn history_is_bounded_and_empty_history_does_nothing() {
        let mut line = CommandLine::default();
        line.history_prev();
        line.history_next();
        assert!(line.input.is_empty());
        for i in 0..=HISTORY_LIMIT {
            type_and_submit(&mut line, &format!("help {i}"));
        }
        assert_eq!(line.history.len(), HISTORY_LIMIT);
        assert_eq!(line.history[0], "help 1"); // the oldest went
    }

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
    fn tab_cycles_through_candidates() {
        let mut line = CommandLine::default();
        "launch s".chars().for_each(|c| line.input.insert(c));
        let candidates = || Completion {
            base: "launch ".into(),
            candidates: vec!["Steam".into(), "Stellaris".into()],
        };
        line.complete(|_| Some(candidates()), true);
        assert_eq!(line.input.text(), "launch Steam");
        line.complete(|_| panic!("cycles without recomputing"), true);
        assert_eq!(line.input.text(), "launch Stellaris");
        line.complete(|_| None, true); // wraps
        assert_eq!(line.input.text(), "launch Steam");
        line.complete(|_| None, false);
        assert_eq!(line.input.text(), "launch Stellaris");

        line.input.insert(' '); // edited: the next Tab starts over
        line.complete(|_| None, true);
        assert_eq!(line.completion, None);
        assert_eq!(line.input.text(), "launch Stellaris ");
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
