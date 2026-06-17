use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::agent::{restart_agent, send_user_message};
use crate::state::*;
use jinx::app::Panel;
use jinx::text_editor::TextEditor;

use super::{panel_block, strip_md, word_wrap};

// ---------------------------------------------------------------------------
// Slash-command registry
// ---------------------------------------------------------------------------

struct SlashCommand {
    name: &'static str,
    description: &'static str,
}

const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand { name: "clear", description: "Borrar chat y reiniciar agente" },
];

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub(crate) fn update_cmd_picker(state: &mut RuntimeState) {
    let text = state.chat_editor.to_string();
    if text.starts_with('/') && state.chat_editor.line_count() == 1 {
        let query = &text[1..];
        let filtered: Vec<usize> = SLASH_COMMANDS.iter().enumerate()
            .filter(|(_, cmd)| cmd.name.starts_with(query))
            .map(|(i, _)| i)
            .collect();
        if !filtered.is_empty() {
            state.cmd_picker_active = true;
            state.cmd_picker_filtered = filtered;
            state.cmd_picker_cursor = state.cmd_picker_cursor.min(
                state.cmd_picker_filtered.len().saturating_sub(1)
            );
            return;
        }
    }
    state.cmd_picker_active = false;
    state.cmd_picker_filtered.clear();
}

pub(crate) fn handle_chat_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    // Note picker intercepts keys when active
    if state.note_picker_active {
        if let Some(msg_idx) = state.note_picker_msg_idx {
            let count = state.chat_history.get(msg_idx)
                .and_then(|m| m.note_results.as_ref())
                .map(|v| v.len())
                .unwrap_or(0);
            if count > 0 {
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        if state.note_picker_cursor + 1 < count {
                            state.note_picker_cursor += 1;
                        }
                        return;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if state.note_picker_cursor > 0 {
                            state.note_picker_cursor -= 1;
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        if let Some(entry) = state.chat_history.get(msg_idx)
                            .and_then(|m| m.note_results.as_ref())
                            .and_then(|v| v.get(state.note_picker_cursor))
                        {
                            let note_id = entry.id;
                            state.notes_current_id = Some(note_id);
                            state.notes_view = NotesView::Preview;
                            state.notes_preview_scroll = 0;
                            state.app.focused_panel = Panel::Notas;
                            super::refresh_notes_cache(state);
                        }
                        state.note_picker_active = false;
                        return;
                    }
                    KeyCode::Esc => {
                        state.note_picker_active = false;
                        return;
                    }
                    _ => {}
                }
            }
        }
        state.note_picker_active = false;
    }

    // Slash-command picker intercepts keys when active
    if state.cmd_picker_active {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let max = state.cmd_picker_filtered.len();
                if state.cmd_picker_cursor + 1 < max {
                    state.cmd_picker_cursor += 1;
                }
                return;
            }
            KeyCode::Up | KeyCode::Char('k') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if state.cmd_picker_cursor > 0 {
                    state.cmd_picker_cursor -= 1;
                }
                return;
            }
            KeyCode::Tab => {
                if let Some(&idx) = state.cmd_picker_filtered.get(state.cmd_picker_cursor) {
                    let full = format!("/{}", SLASH_COMMANDS[idx].name);
                    state.chat_editor = TextEditor::from_string(&full);
                }
                update_cmd_picker(state);
                return;
            }
            KeyCode::Enter => {
                if let Some(&idx) = state.cmd_picker_filtered.get(state.cmd_picker_cursor) {
                    let full = format!("/{}", SLASH_COMMANDS[idx].name);
                    state.chat_editor = TextEditor::from_string(&full);
                    state.cmd_picker_active = false;
                }
                // Fall through to the Enter handler below to execute
            }
            KeyCode::Esc => {
                state.cmd_picker_active = false;
                return;
            }
            _ => {
                // Fall through to normal key handling; update_cmd_picker runs at the end
            }
        }
    }

    match key.code {
        KeyCode::Enter => {
            let text = state.chat_editor.to_string();
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                state.app.status_bar = state.locale.errors.empty_message.clone();
                return;
            }
            if trimmed == "/clear" {
                state.chat_editor.clear();
                state.chat_history.clear();
                state.chat_scroll = 0;
                state.note_picker_active = false;
                state.note_picker_msg_idx = None;
                state.pending_request = None;
                restart_agent(state);
                return;
            }
            state.prompt_history.push(trimmed.clone());
            state.prompt_history_idx = None;
            state.prompt_stash.clear();
            state.note_picker_active = false;
            state.chat_history.push(ChatMsg { role: ChatRole::User, text: trimmed.clone(), note_results: None });
            state.chat_editor.clear();
            state.chat_scroll = 0;
            send_user_message(state, trimmed);
        }
        // Ctrl+J inserts newline (does NOT send)
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.insert_newline();
        }
        // Chat history scroll (Shift+arrows)
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.chat_scroll = state.chat_scroll.saturating_add(3);
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.chat_scroll = state.chat_scroll.saturating_sub(3);
        }
        // Navigation — Up/Down: history recall when single-line, else cursor movement
        KeyCode::Left => state.chat_editor.move_left(),
        KeyCode::Right => state.chat_editor.move_right(),
        KeyCode::Up if state.chat_editor.line_count() == 1 => {
            if state.prompt_history.is_empty() { return; }
            let idx = match state.prompt_history_idx {
                None => {
                    state.prompt_stash = state.chat_editor.to_string();
                    state.prompt_history.len() - 1
                }
                Some(0) => return,
                Some(i) => i - 1,
            };
            state.prompt_history_idx = Some(idx);
            state.chat_editor = TextEditor::from_string(&state.prompt_history[idx]);
        }
        KeyCode::Down if state.chat_editor.line_count() == 1 && state.prompt_history_idx.is_some() => {
            let idx = state.prompt_history_idx.unwrap();
            if idx + 1 >= state.prompt_history.len() {
                state.prompt_history_idx = None;
                state.chat_editor = TextEditor::from_string(&state.prompt_stash);
            } else {
                state.prompt_history_idx = Some(idx + 1);
                state.chat_editor = TextEditor::from_string(&state.prompt_history[idx + 1]);
            }
        }
        KeyCode::Up => state.chat_editor.move_up(),
        KeyCode::Down => state.chat_editor.move_down(),
        // Readline shortcuts
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.move_home();
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.move_end();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.kill_to_start();
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.kill_to_end();
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.kill_word_back();
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.clear();
        }
        // Alt+B / Alt+F — word movement (Option on macOS emits ALT)
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
            state.chat_editor.move_word_back();
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
            state.chat_editor.move_word_forward();
        }
        KeyCode::Backspace => state.chat_editor.backspace(),
        KeyCode::Delete => state.chat_editor.delete(),
        // Chat history scroll
        KeyCode::PageUp if key.modifiers.contains(KeyModifiers::SHIFT) => {
            // Jump to top
            state.chat_scroll = usize::MAX; // clamped during render
        }
        KeyCode::PageDown if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.chat_scroll = 0;
        }
        KeyCode::PageUp => {
            state.chat_scroll = state.chat_scroll.saturating_add(10);
        }
        KeyCode::PageDown => {
            state.chat_scroll = state.chat_scroll.saturating_sub(10);
        }
        // Regular character input
        KeyCode::Char(c) => {
            state.chat_editor.insert_char(c);
        }
        _ => {}
    }

    update_cmd_picker(state);
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

pub(crate) fn render_chat(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let block = panel_block(state.locale.panels.chat.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Calculate dynamic input height based on editor content
    let input_width = inner.width.saturating_sub(2) as usize; // account for borders
    let visual_lines = count_visual_lines(&state.chat_editor, input_width);
    let min_input_height: u16 = 3;
    let max_input_height: u16 = 8.min((inner.height * 40 / 100).max(3));
    let input_height = (visual_lines as u16 + 2) // +2 for border
        .max(min_input_height)
        .min(max_input_height);

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(input_height)])
        .split(inner);

    let hist_area = parts[0];
    state.history_area = Some(hist_area);
    let wrap_width = (hist_area.width as usize).saturating_sub(2);
    let avail_height = hist_area.height as usize;

    // Build display lines for all messages
    let mut all_lines: Vec<Line<'static>> = Vec::new();
    for (msg_idx, msg) in state.chat_history.iter().enumerate() {
        let (label, color): (&str, Color) = match msg.role {
            ChatRole::Agent => (state.locale.chat.agent.as_str(), Color::Green),
            ChatRole::System => (state.locale.chat.system.as_str(), Color::Yellow),
            ChatRole::User => (state.locale.chat.you.as_str(), Color::Cyan),
        };
        let style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        let body_style = Style::default().fg(color);

        let clean = strip_md(&msg.text);
        let wrapped = word_wrap(&clean, wrap_width.saturating_sub(label.len() + 3));

        let header = format!("[{label}]");
        if wrapped.is_empty() {
            all_lines.push(Line::from(Span::styled(header, style)));
        } else {
            all_lines.push(Line::from(vec![
                Span::styled(format!("{header} "), style),
                Span::styled(wrapped[0].clone(), body_style),
            ]));
            let indent = " ".repeat(header.chars().count() + 1);
            for line in &wrapped[1..] {
                all_lines.push(Line::from(Span::styled(
                    format!("{indent}{line}"),
                    body_style,
                )));
            }
        }

        // Render note picker entries if present
        if let Some(entries) = &msg.note_results {
            let is_active = state.note_picker_active
                && state.note_picker_msg_idx == Some(msg_idx);
            all_lines.push(Line::from(Span::styled(
                "  ┌─────────────────────────────────────┐".to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            for (i, entry) in entries.iter().enumerate() {
                let cursor = if is_active && i == state.note_picker_cursor { "▶" } else { " " };
                let title = if entry.title.is_empty() {
                    &state.locale.misc.untitled_note
                } else {
                    &entry.title
                };
                let date = &entry.updated_at[..10.min(entry.updated_at.len())];
                let label_text = format!("  │ {cursor} {title}  {date}");
                let entry_style = if is_active && i == state.note_picker_cursor {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                all_lines.push(Line::from(Span::styled(label_text, entry_style)));
            }
            all_lines.push(Line::from(Span::styled(
                "  └─────────────────────────────────────┘".to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            if is_active {
                all_lines.push(Line::from(Span::styled(
                    "  ↑↓:select  Enter:open  Esc:close".to_string(),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                )));
            }
        }

        all_lines.push(Line::from(""));
    }

    // Typing indicator when agent is working
    if let Some((_, started)) = &state.pending_request {
        let elapsed = started.elapsed();
        let dot_count = (elapsed.as_millis() / 500) as usize % 3 + 1;
        let dots = ".".repeat(dot_count);
        let secs = elapsed.as_secs();
        let indicator = state.locale.chat.thinking
            .replace("{dots}", &dots)
            .replace("{secs}", &secs.to_string());
        all_lines.push(Line::from(Span::styled(
            indicator,
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )));
        all_lines.push(Line::from(""));
    }

    // Scroll support: offset from bottom
    let total = all_lines.len();
    let scroll_offset = state.chat_scroll.min(total.saturating_sub(avail_height));
    let end = total.saturating_sub(scroll_offset);
    let start = end.saturating_sub(avail_height);
    let mut visible: Vec<Line<'static>> = all_lines[start..end].to_vec();

    // Show scroll indicator if not at bottom
    if scroll_offset > 0 && !visible.is_empty() {
        visible[0] = Line::from(Span::styled(
            state.locale.chat.older_messages.clone(),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        ));
    }

    frame.render_widget(Paragraph::new(visible), hist_area);

    // Input field with cursor
    let input_block = Block::default().title(state.locale.chat.input_title.as_str()).borders(Borders::ALL);
    let input_inner = input_block.inner(parts[1]);
    state.input_area = Some(input_inner);
    frame.render_widget(input_block, parts[1]);

    // Render editor content with character-level wrapping (matches cursor calculation)
    let input_w = input_inner.width as usize;
    let mut input_lines: Vec<Line<'_>> = Vec::new();
    for logical_line in state.chat_editor.lines() {
        if logical_line.is_empty() || input_w == 0 {
            input_lines.push(Line::from(""));
        } else {
            let chars: Vec<char> = logical_line.chars().collect();
            for chunk in chars.chunks(input_w) {
                input_lines.push(Line::from(chunk.iter().collect::<String>()));
            }
        }
    }
    let input_para = Paragraph::new(input_lines);
    frame.render_widget(input_para, input_inner);

    // Slash-command picker overlay
    if state.cmd_picker_active && !state.cmd_picker_filtered.is_empty() {
        let picker_height = (state.cmd_picker_filtered.len() as u16 + 2).min(6);
        let picker_area = Rect {
            x: parts[1].x,
            y: parts[1].y.saturating_sub(picker_height),
            width: parts[1].width.min(40),
            height: picker_height,
        };
        let picker_block = Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let picker_inner = picker_block.inner(picker_area);
        frame.render_widget(Clear, picker_area);
        frame.render_widget(picker_block, picker_area);

        let mut cmd_lines: Vec<Line<'static>> = Vec::new();
        for (i, &cmd_idx) in state.cmd_picker_filtered.iter().enumerate() {
            let cmd = &SLASH_COMMANDS[cmd_idx];
            let cursor_mark = if i == state.cmd_picker_cursor { "▶" } else { " " };
            let style = if i == state.cmd_picker_cursor {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            cmd_lines.push(Line::from(Span::styled(
                format!("{} /{:<12} {}", cursor_mark, cmd.name, cmd.description),
                style,
            )));
        }
        frame.render_widget(Paragraph::new(cmd_lines), picker_inner);
    }

    // Set cursor position (only when Chat panel is focused and no modal)
    if state.app.focused_panel == Panel::Chat && state.app.modal.is_none() {
        let (cursor_x, cursor_y) = calculate_cursor_position(&state.chat_editor, input_inner);
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

/// Count the total visual lines the editor content would occupy given a width.
pub(crate) fn count_visual_lines(editor: &TextEditor, width: usize) -> usize {
    if width == 0 {
        return editor.line_count();
    }
    editor.lines().iter().map(|line| {
        let char_count = line.chars().count();
        if char_count == 0 {
            1
        } else {
            char_count.div_ceil(width)
        }
    }).sum()
}

/// Calculate the absolute (x, y) position of the cursor within the input area.
pub(crate) fn calculate_cursor_position(editor: &TextEditor, area: Rect) -> (u16, u16) {
    let width = area.width as usize;
    if width == 0 {
        return (area.x, area.y);
    }

    let mut visual_row: usize = 0;
    for row in 0..editor.cursor_row() {
        let line_chars = editor.lines()[row].chars().count();
        visual_row += if line_chars == 0 { 1 } else { line_chars.div_ceil(width) };
    }

    // For the cursor's line, find which visual row/col the cursor falls on
    let current_line = editor.current_line();
    let cursor_char_offset = current_line[..editor.cursor_col()].chars().count();
    let extra_rows = cursor_char_offset / width;
    let col_in_row = cursor_char_offset % width;

    visual_row += extra_rows;

    let x = area.x + col_in_row as u16;
    let y = area.y + (visual_row as u16).min(area.height.saturating_sub(1));
    (x, y)
}
