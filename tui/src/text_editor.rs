//! Pure text editor model with cursor for the chat input field.
//!
//! The buffer is stored as `Vec<String>` (one entry per logical line).
//! The cursor is `(row, col)` where `col` is always a valid char boundary.
//! No I/O or Ratatui dependency — purely testable.

/// A simple multi-line text editor with cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEditor {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
}

impl std::fmt::Display for TextEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.lines.join("\n"))
    }
}

impl Default for TextEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEditor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    pub fn from_string(s: &str) -> Self {
        let lines: Vec<String> = if s.is_empty() {
            vec![String::new()]
        } else {
            s.split('\n').map(|l| l.to_string()).collect()
        };
        let cursor_row = lines.len() - 1;
        let cursor_col = lines[cursor_row].len();
        Self { lines, cursor_row, cursor_col }
    }

    pub fn from_lines(lines: Vec<String>, cursor_row: usize, cursor_col: usize) -> Self {
        Self { lines, cursor_row, cursor_col }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.trim().is_empty())
    }

    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    // -- Accessors --

    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn cursor_row(&self) -> usize {
        self.cursor_row
    }

    pub fn cursor_col(&self) -> usize {
        self.cursor_col
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn current_line(&self) -> &str {
        &self.lines[self.cursor_row]
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    // -- Basic editing --

    pub fn insert_char(&mut self, c: char) {
        let line = &mut self.lines[self.cursor_row];
        line.insert(self.cursor_col, c);
        self.cursor_col += c.len_utf8();
    }

    pub fn insert_newline(&mut self) {
        let tail = self.lines[self.cursor_row][self.cursor_col..].to_string();
        self.lines[self.cursor_row].truncate(self.cursor_col);
        self.cursor_row += 1;
        self.lines.insert(self.cursor_row, tail);
        self.cursor_col = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let line = &self.lines[self.cursor_row];
            let prev_char_start = prev_char_boundary(line, self.cursor_col);
            self.lines[self.cursor_row].drain(prev_char_start..self.cursor_col);
            self.cursor_col = prev_char_start;
        } else if self.cursor_row > 0 {
            let removed = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&removed);
        }
    }

    pub fn delete(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col < line_len {
            let next = next_char_boundary(&self.lines[self.cursor_row], self.cursor_col);
            self.lines[self.cursor_row].drain(self.cursor_col..next);
        } else if self.cursor_row + 1 < self.lines.len() {
            let next_line = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].push_str(&next_line);
        }
    }

    // -- Navigation --

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col = prev_char_boundary(&self.lines[self.cursor_row], self.cursor_col);
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
        }
    }

    pub fn move_right(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col < line_len {
            self.cursor_col = next_char_boundary(&self.lines[self.cursor_row], self.cursor_col);
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = clamp_col(&self.lines[self.cursor_row], self.cursor_col);
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = clamp_col(&self.lines[self.cursor_row], self.cursor_col);
        }
    }

    pub fn move_home(&mut self) {
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_row = self.lines.len() - 1;
        self.cursor_col = self.lines[self.cursor_row].len();
    }

    // -- Readline shortcuts --

    pub fn kill_to_start(&mut self) {
        self.lines[self.cursor_row].drain(..self.cursor_col);
        self.cursor_col = 0;
    }

    pub fn kill_to_end(&mut self) {
        self.lines[self.cursor_row].truncate(self.cursor_col);
    }

    pub fn kill_word_back(&mut self) {
        if self.cursor_col == 0 {
            return;
        }
        let line = &self.lines[self.cursor_row];
        let start = word_start_before(line, self.cursor_col);
        self.lines[self.cursor_row].drain(start..self.cursor_col);
        self.cursor_col = start;
    }

    pub fn move_word_back(&mut self) {
        if self.cursor_col == 0 {
            return;
        }
        let line = &self.lines[self.cursor_row];
        self.cursor_col = word_start_before(line, self.cursor_col);
    }

    pub fn move_word_forward(&mut self) {
        let line = &self.lines[self.cursor_row];
        if self.cursor_col >= line.len() {
            return;
        }
        self.cursor_col = word_end_after(line, self.cursor_col);
    }
}

// -- Helper functions --

/// Find the byte offset of the previous char boundary before `pos`.
fn prev_char_boundary(s: &str, pos: usize) -> usize {
    let mut i = pos;
    loop {
        i -= 1;
        if s.is_char_boundary(i) {
            return i;
        }
        if i == 0 {
            return 0;
        }
    }
}

/// Find the byte offset of the next char boundary after `pos`.
fn next_char_boundary(s: &str, pos: usize) -> usize {
    let mut i = pos + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

/// Clamp a column to the nearest valid char boundary <= line.len().
fn clamp_col(line: &str, desired_col: usize) -> usize {
    let max = line.len();
    if desired_col >= max {
        return max;
    }
    // Walk back to a valid char boundary
    let mut col = desired_col;
    while col > 0 && !line.is_char_boundary(col) {
        col -= 1;
    }
    col
}

/// Find the start of the word before `pos` in the line.
/// Skips whitespace backwards, then skips non-whitespace backwards.
fn word_start_before(line: &str, pos: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = pos;

    // Skip whitespace backwards
    while i > 0 {
        let prev = prev_char_boundary(line, i);
        if !bytes[prev..i].iter().all(|&b| (b as char).is_whitespace()) {
            break;
        }
        i = prev;
    }

    // Skip non-whitespace backwards
    while i > 0 {
        let prev = prev_char_boundary(line, i);
        if bytes[prev..i].iter().any(|&b| (b as char).is_whitespace()) {
            break;
        }
        i = prev;
    }

    i
}

/// Find the end of the word after `pos` in the line.
/// Skips whitespace forward, then skips non-whitespace forward.
fn word_end_after(line: &str, pos: usize) -> usize {
    let len = line.len();
    let mut i = pos;

    // Skip non-whitespace forward (current word)
    while i < len {
        let next = next_char_boundary(line, i);
        let chunk = &line[i..next];
        if chunk.chars().next().map(|c| c.is_whitespace()).unwrap_or(false) {
            break;
        }
        i = next;
    }

    // Skip whitespace forward
    while i < len {
        let next = next_char_boundary(line, i);
        let chunk = &line[i..next];
        if chunk.chars().next().map(|c| !c.is_whitespace()).unwrap_or(false) {
            break;
        }
        i = next;
    }

    i
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Construction and basic properties --

    #[test]
    fn new_editor_is_empty() {
        let ed = TextEditor::new();
        assert!(ed.is_empty());
        assert_eq!(ed.cursor(), (0, 0));
        assert_eq!(ed.line_count(), 1);
        assert_eq!(ed.to_string(), "");
    }

    #[test]
    fn from_string_round_trip() {
        let text = "hello\nworld\nfoo";
        let ed = TextEditor::from_string(text);
        assert_eq!(ed.to_string(), text);
        assert_eq!(ed.line_count(), 3);
        // Cursor at end
        assert_eq!(ed.cursor(), (2, 3));
    }

    #[test]
    fn is_empty_whitespace_only() {
        let ed = TextEditor::from_string("   \n  \n\t");
        assert!(ed.is_empty());
    }

    #[test]
    fn is_empty_with_content() {
        let ed = TextEditor::from_string("  hello  ");
        assert!(!ed.is_empty());
    }

    // -- Insert char --

    #[test]
    fn insert_char_at_end() {
        let mut ed = TextEditor::new();
        ed.insert_char('h');
        ed.insert_char('i');
        assert_eq!(ed.to_string(), "hi");
        assert_eq!(ed.cursor(), (0, 2));
    }

    #[test]
    fn insert_char_at_middle() {
        let mut ed = TextEditor::from_string("ac");
        ed.move_home();
        ed.move_right(); // after 'a'
        ed.insert_char('b');
        assert_eq!(ed.to_string(), "abc");
        assert_eq!(ed.cursor_col(), 2);
    }

    #[test]
    fn insert_multibyte_char() {
        let mut ed = TextEditor::new();
        ed.insert_char('é');
        ed.insert_char('ñ');
        assert_eq!(ed.to_string(), "éñ");
        assert_eq!(ed.cursor_col(), "éñ".len());
    }

    // -- Backspace --

    #[test]
    fn backspace_in_middle() {
        let mut ed = TextEditor::from_string("abc");
        ed.move_home();
        ed.move_right();
        ed.move_right(); // after 'b'
        ed.backspace();
        assert_eq!(ed.to_string(), "ac");
        assert_eq!(ed.cursor_col(), 1);
    }

    #[test]
    fn backspace_at_start_no_op() {
        let mut ed = TextEditor::from_string("hello");
        ed.move_home();
        ed.backspace();
        assert_eq!(ed.to_string(), "hello");
        assert_eq!(ed.cursor(), (0, 0));
    }

    #[test]
    fn backspace_merges_lines() {
        let mut ed = TextEditor::from_string("ab\ncd");
        ed.cursor_row = 1;
        ed.cursor_col = 0;
        ed.backspace();
        assert_eq!(ed.to_string(), "abcd");
        assert_eq!(ed.cursor(), (0, 2));
    }

    // -- Delete --

    #[test]
    fn delete_in_middle() {
        let mut ed = TextEditor::from_string("abc");
        ed.move_home();
        ed.move_right(); // after 'a'
        ed.delete();
        assert_eq!(ed.to_string(), "ac");
        assert_eq!(ed.cursor_col(), 1);
    }

    #[test]
    fn delete_at_end_no_op_single_line() {
        let mut ed = TextEditor::from_string("abc");
        // cursor already at end
        ed.delete();
        assert_eq!(ed.to_string(), "abc");
    }

    #[test]
    fn delete_merges_next_line() {
        let mut ed = TextEditor::from_string("ab\ncd");
        ed.cursor_row = 0;
        ed.cursor_col = 2; // end of first line
        ed.delete();
        assert_eq!(ed.to_string(), "abcd");
        assert_eq!(ed.cursor(), (0, 2));
    }

    // -- Insert newline --

    #[test]
    fn insert_newline_splits_line() {
        let mut ed = TextEditor::from_string("abcd");
        ed.move_home();
        ed.move_right();
        ed.move_right(); // after 'b'
        ed.insert_newline();
        assert_eq!(ed.to_string(), "ab\ncd");
        assert_eq!(ed.cursor(), (1, 0));
    }

    // -- Navigation --

    #[test]
    fn move_left_wraps_to_prev_line() {
        let mut ed = TextEditor::from_string("ab\ncd");
        ed.cursor_row = 1;
        ed.cursor_col = 0;
        ed.move_left();
        assert_eq!(ed.cursor(), (0, 2));
    }

    #[test]
    fn move_right_wraps_to_next_line() {
        let mut ed = TextEditor::from_string("ab\ncd");
        ed.cursor_row = 0;
        ed.cursor_col = 2;
        ed.move_right();
        assert_eq!(ed.cursor(), (1, 0));
    }

    #[test]
    fn move_up_clamps_col() {
        let mut ed = TextEditor::from_string("hi\nworld");
        // cursor at end of "world" (row=1, col=5)
        ed.move_up();
        // "hi" has len 2, so col clamped to 2
        assert_eq!(ed.cursor(), (0, 2));
    }

    #[test]
    fn move_down_clamps_col() {
        let mut ed = TextEditor::from_string("world\nhi");
        ed.cursor_row = 0;
        ed.cursor_col = 5; // end of "world"
        ed.move_down();
        assert_eq!(ed.cursor(), (1, 2)); // "hi" len=2
    }

    #[test]
    fn move_home_and_end() {
        let mut ed = TextEditor::from_string("abc\ndef\nghi");
        ed.cursor_row = 1;
        ed.cursor_col = 1;
        ed.move_home();
        assert_eq!(ed.cursor(), (0, 0));
        ed.move_end();
        assert_eq!(ed.cursor(), (2, 3));
    }

    // -- Readline: kill --

    #[test]
    fn kill_to_start() {
        let mut ed = TextEditor::from_string("hello world");
        ed.cursor_row = 0;
        ed.cursor_col = 5;
        ed.kill_to_start();
        assert_eq!(ed.to_string(), " world");
        assert_eq!(ed.cursor_col(), 0);
    }

    #[test]
    fn kill_to_end() {
        let mut ed = TextEditor::from_string("hello world");
        ed.cursor_row = 0;
        ed.cursor_col = 5;
        ed.kill_to_end();
        assert_eq!(ed.to_string(), "hello");
        assert_eq!(ed.cursor_col(), 5);
    }

    #[test]
    fn kill_word_back() {
        let mut ed = TextEditor::from_string("hello world");
        // cursor at end
        ed.kill_word_back();
        assert_eq!(ed.to_string(), "hello ");
        assert_eq!(ed.cursor_col(), 6);
    }

    #[test]
    fn kill_word_back_at_start() {
        let mut ed = TextEditor::from_string("hello");
        ed.cursor_row = 0;
        ed.cursor_col = 0;
        ed.kill_word_back();
        assert_eq!(ed.to_string(), "hello");
    }

    // -- Readline: word movement --

    #[test]
    fn move_word_back() {
        let mut ed = TextEditor::from_string("hello world foo");
        // cursor at end (col=15)
        ed.move_word_back();
        assert_eq!(ed.cursor_col(), 12); // start of "foo"... actually after "world "
        // Let's trace: pos=15, skip non-ws backwards from 15: "foo" -> i=12
        // then skip ws: ' ' at 11 -> i=11... wait the algorithm:
        // Actually re-checking: word_start_before skips whitespace first, then non-ws.
        // At pos=15 (end), bytes[14]='o' not ws, so first loop doesn't skip.
        // Then skip non-ws: 'o','o','f' -> stops at 12 (space). Result: 12.
        // Hmm, that leaves cursor at col 12 which is the start of "foo".
        // Wait: "hello world foo" indices: h0 e1 l2 l3 o4 ' '5 w6 o7 r8 l9 d10 ' '11 f12 o13 o14
        // len=15. cursor at 15. word_start_before: skip ws backwards from 15: nothing (14='o').
        // skip non-ws backwards: 14='o',13='o',12='f',11=' ' -> stop at 12. Yes, start of "foo".
    }

    #[test]
    fn move_word_forward() {
        let mut ed = TextEditor::from_string("hello world foo");
        ed.move_home();
        ed.move_word_forward();
        // From 0: skip non-ws "hello" -> col=5, then skip ws ' ' -> col=6
        assert_eq!(ed.cursor_col(), 6);
    }

    #[test]
    fn move_word_forward_at_end() {
        let mut ed = TextEditor::from_string("hello");
        // cursor at end
        ed.move_word_forward();
        assert_eq!(ed.cursor_col(), 5); // no change
    }

    #[test]
    fn move_word_back_at_start() {
        let mut ed = TextEditor::from_string("hello");
        ed.move_home();
        ed.move_word_back();
        assert_eq!(ed.cursor_col(), 0); // no change
    }

    // -- Clear --

    #[test]
    fn clear_resets_everything() {
        let mut ed = TextEditor::from_string("some\ntext");
        ed.clear();
        assert_eq!(ed.to_string(), "");
        assert_eq!(ed.cursor(), (0, 0));
        assert_eq!(ed.line_count(), 1);
        assert!(ed.is_empty());
    }
}
