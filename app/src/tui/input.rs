//! Single-line text editing for the TUI's fields (pairing code, IP, log
//! filter, the file browser's path).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Default, Clone, Debug, PartialEq)]
pub struct LineEdit {
    pub text: String,
    /// in chars, 0..=len
    pub cursor: usize,
}

impl LineEdit {
    pub fn set(&mut self, s: impl Into<String>) {
        self.text = s.into();
        self.cursor = self.len();
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, i: usize) -> usize {
        self.text.char_indices().nth(i).map_or(self.text.len(), |(b, _)| b)
    }

    /// Type or paste `s` at the cursor (control characters and newlines dropped).
    pub fn insert(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            let b = self.byte(self.cursor);
            self.text.insert(b, c);
            self.cursor += 1;
        }
    }

    fn remove(&mut self, from: usize, to: usize) {
        let (a, b) = (self.byte(from), self.byte(to));
        self.text.replace_range(a..b, "");
        self.cursor = from;
    }

    /// Apply an editing key. Returns false for keys that aren't editing
    /// (Tab, Enter, arrows up/down…), so the caller can handle them.
    pub fn key(&mut self, k: &KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('u') if ctrl => self.remove(0, self.cursor),
            KeyCode::Char('w') if ctrl => {
                let chars: Vec<char> = self.text.chars().collect();
                let mut i = self.cursor;
                while i > 0 && chars[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && chars[i - 1] != ' ' && chars[i - 1] != '/' {
                    i -= 1;
                }
                // a trailing '/' goes with the word before it
                if i == self.cursor && i > 0 {
                    i -= 1;
                }
                self.remove(i, self.cursor);
            }
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.len(),
            KeyCode::Char(_) if ctrl || k.modifiers.contains(KeyModifiers::ALT) => return false,
            KeyCode::Char(c) => self.insert(c.encode_utf8(&mut [0; 4])),
            KeyCode::Backspace if self.cursor > 0 => self.remove(self.cursor - 1, self.cursor),
            KeyCode::Backspace => {}
            KeyCode::Delete if self.cursor < self.len() => {
                let c = self.cursor;
                self.remove(c, c + 1);
            }
            KeyCode::Delete => {}
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn edits() {
        let mut e = LineEdit::default();
        for c in "héllo".chars() {
            assert!(e.key(&k(KeyCode::Char(c))));
        }
        assert_eq!((e.text.as_str(), e.cursor), ("héllo", 5));
        e.key(&k(KeyCode::Left));
        e.key(&k(KeyCode::Left));
        e.key(&k(KeyCode::Backspace));
        assert_eq!((e.text.as_str(), e.cursor), ("hélo", 2));
        e.key(&k(KeyCode::Delete));
        assert_eq!(e.text, "héo");
        e.key(&k(KeyCode::Home));
        e.insert("a\nb");
        assert_eq!((e.text.as_str(), e.cursor), ("abhéo", 2));
        e.key(&ctrl('u'));
        assert_eq!((e.text.as_str(), e.cursor), ("héo", 0));
        assert!(!e.key(&k(KeyCode::Tab)));
        assert!(!e.key(&k(KeyCode::Enter)));
        assert!(!e.key(&ctrl('l')));
    }

    #[test]
    fn delete_word_stops_at_slashes() {
        let mut e = LineEdit::default();
        e.set("/home/user/Docs");
        e.key(&ctrl('w'));
        assert_eq!(e.text, "/home/user/");
        e.key(&ctrl('w'));
        assert_eq!(e.text, "/home/user");
        e.key(&ctrl('w'));
        assert_eq!(e.text, "/home/");
    }
}
