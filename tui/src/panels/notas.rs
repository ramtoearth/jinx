use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use jinx_core::note::NoteRepository;

use crate::state::*;
use jinx::app::Modal;
use jinx::config::{self as app_config};
use jinx::text_editor::TextEditor;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn refresh_notes_cache(state: &mut RuntimeState) {
    state.notes_cache = if state.notes_search_active && !state.notes_search_query.is_empty() {
        state.storage.search_notes(&state.notes_search_query).unwrap_or_default()
    } else {
        state.storage.list_notes().unwrap_or_default()
    };
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub(crate) fn handle_notas_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match state.notes_view {
        NotesView::List => handle_notes_list_key(state, key),
        NotesView::Preview => handle_notes_preview_key(state, key),
        NotesView::Edit => handle_notes_edit_key(state, key),
    }
}

pub(crate) fn handle_notes_list_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    if state.notes_search_active {
        match key.code {
            KeyCode::Esc => {
                state.notes_search_active = false;
                state.notes_search_query.clear();
                refresh_notes_cache(state);
            }
            KeyCode::Enter => {
                state.notes_search_active = false;
            }
            KeyCode::Backspace => {
                state.notes_search_query.pop();
                refresh_notes_cache(state);
                state.notes_cursor = 0;
            }
            KeyCode::Char(c) => {
                state.notes_search_query.push(c);
                refresh_notes_cache(state);
                state.notes_cursor = 0;
            }
            _ => {}
        }
        return;
    }

    let count = state.notes_cache.len();
    let page = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);

    if state.notes_pending_g {
        state.notes_pending_g = false;
        if key.code == KeyCode::Char('g') && count > 0 {
            state.notes_cursor = 0;
        }
        return;
    }

    match key.code {
        KeyCode::Down | KeyCode::Char('j')
            if count > 0 && state.notes_cursor + 1 < count =>
        {
            state.notes_cursor += 1;
        }
        KeyCode::Up | KeyCode::Char('k') if state.notes_cursor > 0 => {
            state.notes_cursor -= 1;
        }
        KeyCode::Char('G') if count > 0 => {
            state.notes_cursor = count - 1;
        }
        KeyCode::Char('g') => {
            state.notes_pending_g = true;
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) && count > 0 => {
            state.notes_cursor = (state.notes_cursor + page).min(count - 1);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.notes_cursor = state.notes_cursor.saturating_sub(page);
        }
        KeyCode::Enter => {
            if let Some(note) = state.notes_cache.get(state.notes_cursor) {
                state.notes_current_id = Some(note.id);
                state.notes_preview_scroll = 0;
                state.notes_view = NotesView::Preview;
            }
        }
        KeyCode::Char('n') => {
            match state.storage.create_note(jinx_core::NewNote {
                title: String::new(),
                body: String::new(),
            }) {
                Ok(note) => {
                    state.notes_current_id = Some(note.id);
                    state.notes_title_editor = TextEditor::new();
                    state.notes_editor = TextEditor::new();
                    state.notes_title_focused = true;
                    state.notes_editor_scroll = 0;
                    state.notes_undo_stack.clear();
                    state.notes_redo_stack.clear();
                    state.notes_view = NotesView::Edit;
                    refresh_notes_cache(state);
                    state.notes_cursor = 0;
                }
                Err(e) => state.app.status_bar = format!("Error: {}", e.message()),
            }
        }
        KeyCode::Char('d') => {
            if let Some(note) = state.notes_cache.get(state.notes_cursor) {
                state.delete_confirm_name = note.title.clone();
                state.app.modal = Some(Modal::DeleteNote { id: note.id });
            }
        }
        KeyCode::Char('/') => {
            state.notes_search_active = true;
            state.notes_search_query.clear();
        }
        _ => {}
    }
}

pub(crate) fn handle_notes_preview_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char('e') => {
            if let Some(note) = state.notes_cache.iter().find(|n| Some(n.id) == state.notes_current_id) {
                state.notes_title_editor = TextEditor::from_string(&note.title);
                state.notes_editor = TextEditor::from_string(&note.body);
                state.notes_title_focused = true;
                state.notes_editor_scroll = 0;
                state.notes_undo_stack.clear();
                state.notes_redo_stack.clear();
                state.notes_view = NotesView::Edit;
            }
        }
        KeyCode::Esc => {
            state.notes_view = NotesView::List;
            state.notes_current_id = None;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.notes_preview_scroll += 1;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.notes_preview_scroll = state.notes_preview_scroll.saturating_sub(1);
        }
        KeyCode::Char('d') => {
            if let Some(id) = state.notes_current_id {
                if let Some(note) = state.notes_cache.iter().find(|n| n.id == id) {
                    state.delete_confirm_name = note.title.clone();
                }
                state.app.modal = Some(Modal::DeleteNote { id });
            }
        }
        KeyCode::Char('s') => {
            if let Some(id) = state.notes_current_id {
                if let Some(note) = state.notes_cache.iter().find(|n| n.id == id) {
                    let cfg = app_config::load();
                    let initial_dir = cfg.last_export_dir
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| std::env::var("HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from("/")));
                    state.dir_browser.current_dir = initial_dir;
                    state.dir_browser.note_id = id;
                    let safe_title: String = note.title.chars()
                        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
                        .collect();
                    state.dir_browser.filename = format!("{}.md", safe_title.trim());
                    state.dir_browser.field = 0;
                    state.dir_browser.refresh_entries();
                    state.app.modal = Some(Modal::ExportNote { id });
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn notes_push_undo(state: &mut RuntimeState) {
    let snapshot = (
        state.notes_editor.lines().to_vec(),
        state.notes_editor.cursor_row(),
        state.notes_editor.cursor_col(),
    );
    state.notes_undo_stack.push(snapshot);
    state.notes_redo_stack.clear();
}

pub(crate) fn handle_notes_edit_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            save_current_note(state);
            state.notes_view = NotesView::Preview;
        }
        KeyCode::Esc => {
            save_current_note(state);
            state.notes_view = NotesView::Preview;
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.notes_title_focused = !state.notes_title_focused;
        }
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) && !state.notes_title_focused => {
            if let Some((lines, row, col)) = state.notes_undo_stack.pop() {
                let current = (
                    state.notes_editor.lines().to_vec(),
                    state.notes_editor.cursor_row(),
                    state.notes_editor.cursor_col(),
                );
                state.notes_redo_stack.push(current);
                state.notes_editor = TextEditor::from_lines(lines, row, col);
            }
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) && !state.notes_title_focused => {
            if let Some((lines, row, col)) = state.notes_redo_stack.pop() {
                let current = (
                    state.notes_editor.lines().to_vec(),
                    state.notes_editor.cursor_row(),
                    state.notes_editor.cursor_col(),
                );
                state.notes_undo_stack.push(current);
                state.notes_editor = TextEditor::from_lines(lines, row, col);
            }
        }
        _ => {
            if state.notes_title_focused {
                match key.code {
                    KeyCode::Enter => {
                        state.notes_title_focused = false;
                    }
                    KeyCode::Char(c) => { state.notes_title_editor.insert_char(c); }
                    KeyCode::Backspace => { state.notes_title_editor.backspace(); }
                    KeyCode::Left => { state.notes_title_editor.move_left(); }
                    KeyCode::Right => { state.notes_title_editor.move_right(); }
                    KeyCode::Home => { state.notes_title_editor.move_home(); }
                    KeyCode::End => { state.notes_title_editor.move_end(); }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char(c) => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            match c {
                                'u' => { notes_push_undo(state); state.notes_editor.kill_to_start(); }
                                'k' => { notes_push_undo(state); state.notes_editor.kill_to_end(); }
                                'a' => { state.notes_editor.move_home(); }
                                'e' => { state.notes_editor.move_end(); }
                                _ => {}
                            }
                        } else {
                            notes_push_undo(state);
                            state.notes_editor.insert_char(c);
                        }
                    }
                    KeyCode::Enter => { notes_push_undo(state); state.notes_editor.insert_newline(); }
                    KeyCode::Backspace => { notes_push_undo(state); state.notes_editor.backspace(); }
                    KeyCode::Delete => { notes_push_undo(state); state.notes_editor.delete(); }
                    KeyCode::Left => { state.notes_editor.move_left(); }
                    KeyCode::Right => { state.notes_editor.move_right(); }
                    KeyCode::Up => { state.notes_editor.move_up(); }
                    KeyCode::Down => { state.notes_editor.move_down(); }
                    KeyCode::Home => { state.notes_editor.move_home(); }
                    KeyCode::End => { state.notes_editor.move_end(); }
                    _ => {}
                }
            }
        }
    }
}

pub(crate) fn save_current_note(state: &mut RuntimeState) {
    if let Some(id) = state.notes_current_id {
        let title = state.notes_title_editor.to_string();
        let body = state.notes_editor.to_string();
        let title_val = if title.trim().is_empty() {
            state.locale.misc.untitled_note.clone()
        } else {
            title
        };
        match state.storage.update_note(id, jinx_core::NotePatch {
            title: Some(title_val),
            body: Some(body),
        }) {
            Ok(_) => {
                state.app.status_bar = state.locale.status.note_saved.clone();
                refresh_notes_cache(state);
            }
            Err(e) => state.app.status_bar = format!("Error: {}", e.message()),
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

pub(crate) fn render_notas(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    if state.notes_cache.is_empty() && state.notes_view == NotesView::List && !state.notes_search_active {
        refresh_notes_cache(state);
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", state.locale.panels.notes));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match state.notes_view {
        NotesView::List => {
            render_notes_list(frame, state, inner);
        }
        NotesView::Preview | NotesView::Edit => {
            if inner.width >= 60 {
                let cols = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .split(inner);
                render_notes_list(frame, state, cols[0]);
                if state.notes_view == NotesView::Preview {
                    render_note_preview(frame, state, cols[1]);
                } else {
                    render_note_edit(frame, state, cols[1]);
                }
            } else if state.notes_view == NotesView::Preview {
                render_note_preview(frame, state, inner);
            } else {
                render_note_edit(frame, state, inner);
            }
        }
    }
}

pub(crate) fn render_notes_list(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    if area.height < 2 {
        return;
    }

    let available = area.height as usize;
    let count = state.notes_cache.len();

    if state.notes_search_active {
        let query_line = format!(" /{}", state.notes_search_query);
        let search_para = Paragraph::new(query_line).style(Style::default().fg(Color::Yellow));
        let search_area = Rect { height: 1, ..area };
        frame.render_widget(search_para, search_area);
        let list_area = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };
        render_notes_list_items(frame, state, list_area);
        return;
    }

    if count == 0 {
        let empty = Paragraph::new(state.locale.misc.no_notes.as_str())
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, area);
        return;
    }

    render_notes_list_items(frame, state, area);
    let _ = available;
}

pub(crate) fn render_notes_list_items(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let count = state.notes_cache.len();
    let available = area.height as usize;
    let scroll = if state.notes_cursor >= state.notes_scroll + available {
        state.notes_cursor - available + 1
    } else if state.notes_cursor < state.notes_scroll {
        state.notes_cursor
    } else {
        state.notes_scroll
    };

    let items: Vec<ListItem> = state.notes_cache.iter().enumerate()
        .skip(scroll)
        .take(available)
        .map(|(i, note)| {
            let cursor = if i == state.notes_cursor { "▶" } else { " " };
            let title = if note.title.is_empty() {
                &state.locale.misc.untitled_note
            } else {
                &note.title
            };
            let date = &note.updated_at[..10.min(note.updated_at.len())];
            let label = format!("{cursor} {title}  {date}");
            let style = if i == state.notes_cursor {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else if Some(note.id) == state.notes_current_id {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(label).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
    let _ = count;
}

pub(crate) fn render_note_preview(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let note = match state.notes_cache.iter().find(|n| Some(n.id) == state.notes_current_id) {
        Some(n) => n,
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let title_display = if note.title.is_empty() {
        &state.locale.misc.untitled_note
    } else {
        &note.title
    };
    let title_para = Paragraph::new(format!(" {title_display}"))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    frame.render_widget(title_para, chunks[0]);

    let width = chunks[1].width.saturating_sub(1) as usize;
    let rendered = jinx::markdown::render_markdown(&note.body, width.max(10));
    let total = rendered.len();
    let avail = chunks[1].height as usize;
    let scroll = state.notes_preview_scroll.min(total.saturating_sub(avail));
    let visible: Vec<ratatui::text::Line<'_>> = rendered.into_iter().skip(scroll).take(avail).collect();
    let body_para = Paragraph::new(visible);
    frame.render_widget(body_para, chunks[1]);

    let hint_para = Paragraph::new(state.locale.hints.notes_preview.as_str())
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hint_para, chunks[2]);
}

pub(crate) fn render_note_edit(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(area);

    // Title line
    let title_text = state.notes_title_editor.to_string();
    let title_style = if state.notes_title_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let title_label = if state.notes_title_focused {
        format!(" > {title_text}│")
    } else {
        format!("   {title_text}")
    };
    let title_para = Paragraph::new(title_label).style(title_style);
    frame.render_widget(title_para, chunks[0]);

    // Body
    let body_style = if !state.notes_title_focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let input_w = chunks[1].width.saturating_sub(1) as usize;
    let avail = chunks[1].height as usize;

    // Build all visual lines from logical lines
    let mut body_lines: Vec<ratatui::text::Line<'static>> = Vec::new();
    for logical_line in state.notes_editor.lines() {
        if input_w == 0 {
            body_lines.push(ratatui::text::Line::from(logical_line.to_string()));
        } else {
            let chars: Vec<char> = logical_line.chars().collect();
            if chars.is_empty() {
                body_lines.push(ratatui::text::Line::from(String::new()));
            } else {
                for chunk in chars.chunks(input_w) {
                    body_lines.push(ratatui::text::Line::from(chunk.iter().collect::<String>()));
                }
            }
        }
    }

    // Calculate cursor's visual row
    let mut cursor_visual_row: usize = 0;
    if !state.notes_title_focused && input_w > 0 {
        let (row, col) = (state.notes_editor.cursor_row(), state.notes_editor.cursor_col());
        for (i, logical_line) in state.notes_editor.lines().iter().enumerate() {
            let char_count = logical_line.chars().count();
            let n_visual = if char_count == 0 { 1 } else { (char_count.saturating_sub(1) / input_w) + 1 };
            if i == row {
                cursor_visual_row += col / input_w;
                break;
            }
            cursor_visual_row += n_visual;
        }
    }

    // Auto-scroll: ensure cursor is visible
    if cursor_visual_row < state.notes_editor_scroll {
        state.notes_editor_scroll = cursor_visual_row;
    } else if cursor_visual_row >= state.notes_editor_scroll + avail {
        state.notes_editor_scroll = cursor_visual_row - avail + 1;
    }

    let scroll = state.notes_editor_scroll;
    let visible: Vec<ratatui::text::Line<'static>> = body_lines.into_iter()
        .skip(scroll)
        .take(avail)
        .collect();
    let body_para = Paragraph::new(visible).style(body_style);
    frame.render_widget(body_para, chunks[1]);

    // Set cursor position
    if !state.notes_title_focused && input_w > 0 {
        let col = state.notes_editor.cursor_col();
        let visual_col = (col % input_w) as u16;
        let screen_row = (cursor_visual_row - scroll) as u16;
        let x = chunks[1].x + visual_col;
        let y = chunks[1].y + screen_row;
        if x < chunks[1].x + chunks[1].width && y < chunks[1].y + chunks[1].height {
            frame.set_cursor_position((x, y));
        }
    }
}
