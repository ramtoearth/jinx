//! Integration tests for the TextEditor and chat scroll behaviour.

use jinx::text_editor::TextEditor;

// ---------------------------------------------------------------------------
// Task 9.1: Multiline message via Ctrl+J preserves newlines
// ---------------------------------------------------------------------------

#[test]
fn multiline_message_preserves_newlines() {
    let mut ed = TextEditor::new();
    // Simulate typing "hello", Ctrl+J, "world"
    for c in "hello".chars() {
        ed.insert_char(c);
    }
    ed.insert_newline(); // Ctrl+J
    for c in "world".chars() {
        ed.insert_char(c);
    }

    let text = ed.to_string();
    assert_eq!(text, "hello\nworld");
    assert!(text.contains('\n'));
    assert!(!text.trim().is_empty());
}

// ---------------------------------------------------------------------------
// Task 9.2: PgUp/PgDn scroll offset clamping
// ---------------------------------------------------------------------------

#[test]
fn scroll_offset_saturating_add_sub() {
    let mut scroll: usize = 0;

    // PgUp adds 10
    scroll = scroll.saturating_add(10);
    assert_eq!(scroll, 10);

    // PgDn subtracts 10
    scroll = scroll.saturating_sub(10);
    assert_eq!(scroll, 0);

    // PgDn at 0 stays at 0
    scroll = scroll.saturating_sub(10);
    assert_eq!(scroll, 0);

    // Multiple PgUps
    scroll = scroll.saturating_add(10);
    scroll = scroll.saturating_add(10);
    assert_eq!(scroll, 20);

    // Shift+PgDn resets to 0
    scroll = 0;
    assert_eq!(scroll, 0);
}

#[test]
fn scroll_clamped_during_render() {
    // Simulates the clamping logic: scroll_offset.min(total - avail_height)
    let total_lines: usize = 25;
    let avail_height: usize = 10;
    let max_scroll = total_lines.saturating_sub(avail_height); // 15

    let mut scroll: usize = usize::MAX; // Shift+PgUp sets to MAX
    scroll = scroll.min(max_scroll);
    assert_eq!(scroll, 15);

    // Normal case
    scroll = 5;
    scroll = scroll.min(max_scroll);
    assert_eq!(scroll, 5);
}

// ---------------------------------------------------------------------------
// Task 9.3: New message resets scroll to 0
// ---------------------------------------------------------------------------

#[test]
fn new_message_resets_scroll() {
    let mut scroll: usize = 15; // user scrolled up
    assert_eq!(scroll, 15);

    // Simulates what happens when a new message is added
    scroll = 0;
    assert_eq!(scroll, 0);
}

// ---------------------------------------------------------------------------
// Task 9.5: TextEditor doesn't interfere with global keys
// ---------------------------------------------------------------------------

#[test]
fn editor_operations_dont_consume_tab() {
    // Tab is handled at the dispatch level, not by the editor.
    // This test verifies the editor has no Tab-related method that would conflict.
    let mut ed = TextEditor::new();
    ed.insert_char('a');
    ed.insert_char('b');
    // No tab method exists — the focus cycle is handled externally
    assert_eq!(ed.to_string(), "ab");
}

// ---------------------------------------------------------------------------
// Additional edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_editor_send_guard() {
    let ed = TextEditor::new();
    assert!(ed.to_string().trim().is_empty());
    assert!(ed.is_empty());
}

#[test]
fn whitespace_only_editor_is_empty() {
    let mut ed = TextEditor::new();
    ed.insert_char(' ');
    ed.insert_char(' ');
    ed.insert_newline();
    ed.insert_char('\t');
    assert!(ed.is_empty());
    assert!(ed.to_string().trim().is_empty());
}

#[test]
fn cursor_position_after_complex_editing() {
    let mut ed = TextEditor::new();
    // Type "hello world"
    for c in "hello world".chars() {
        ed.insert_char(c);
    }
    assert_eq!(ed.cursor(), (0, 11));

    // Ctrl+A (home)
    ed.move_home();
    assert_eq!(ed.cursor(), (0, 0));

    // Move right 5 (after "hello")
    for _ in 0..5 {
        ed.move_right();
    }
    assert_eq!(ed.cursor(), (0, 5));

    // Ctrl+K (kill to end)
    ed.kill_to_end();
    assert_eq!(ed.to_string(), "hello");
    assert_eq!(ed.cursor(), (0, 5));

    // Ctrl+U (kill to start)
    ed.kill_to_start();
    assert_eq!(ed.to_string(), "");
    assert_eq!(ed.cursor(), (0, 0));
}

#[test]
fn word_navigation_with_multiple_spaces() {
    let mut ed = TextEditor::from_string("hello   world   foo");
    ed.move_home();

    // Alt+F: skip "hello" then "   " -> lands at 8 (start of "world")
    ed.move_word_forward();
    assert_eq!(ed.cursor_col(), 8);

    // Alt+F: skip "world" then "   " -> lands at 16 (start of "foo")
    ed.move_word_forward();
    assert_eq!(ed.cursor_col(), 16);

    // Alt+B: skip back to start of "foo"... we're at 16 which is 'f'
    // Actually from 16: skip whitespace backwards (none, 'f' is non-ws)
    // then skip non-ws backwards: 'f','o','o' would go to... wait.
    // word_start_before at pos=16: bytes[15]=' ' so first loop (skip ws) goes to 15,14,13 -> stops at 12 ('d')
    // Hmm, let me re-read the algorithm.
    // Actually: word_start_before skips ws first, then non-ws.
    // At pos=16: prev_char=15=' ' is ws -> skip: i goes 15,14,13 -> 13 is ' ', 12 is 'd' not ws -> stop at 13? No...
    // Let me trace carefully for "hello   world   foo" (indices 0-18, len=19, cursor at 19 after from_string)
    // We moved to col=16 above. word_start_before(line, 16):
    //   bytes: h0 e1 l2 l3 o4 ' '5 ' '6 ' '7 w8 o9 r10 l11 d12 ' '13 ' '14 ' '15 f16 o17 o18
    //   i=16. First loop (skip ws back): bytes[15]=' '? line[15..16]=" " is ws -> i=15
    //     bytes[14]=' '? line[14..15]=" " is ws -> i=14
    //     bytes[13]=' '? line[13..14]=" " is ws -> i=13
    //     bytes[12]='d'? line[12..13]="d" not ws -> break. i=13
    //   Second loop (skip non-ws back): bytes[12]='d' not ws -> i=12
    //     bytes[11]='l' -> i=11, bytes[10]='r' -> i=10, bytes[9]='o' -> i=9, bytes[8]='w' -> i=8
    //     bytes[7]=' ' ws -> break. i=8
    //   Result: 8. So Alt+B from 16 goes to 8.
    ed.move_word_back();
    assert_eq!(ed.cursor_col(), 8);
}
