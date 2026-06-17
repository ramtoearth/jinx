use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use domain::note::NoteRepository;

use crate::state::*;
use jinx::config as app_config;

pub(crate) fn handle_export_note_key(state: &mut RuntimeState, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => { state.app.modal = None; }
        KeyCode::Up if state.dir_browser.cursor > 0 => {
            state.dir_browser.cursor -= 1;
        }
        KeyCode::Down
            if state.dir_browser.cursor + 1 < state.dir_browser.entries.len() => {
                state.dir_browser.cursor += 1;
            }
        KeyCode::Right => {
            if let Some(entry) = state.dir_browser.entries.get(state.dir_browser.cursor) {
                if entry.is_dir {
                    state.dir_browser.current_dir = state.dir_browser.current_dir.join(&entry.name);
                    state.dir_browser.refresh_entries();
                }
            }
        }
        KeyCode::Left => {
            if let Some(parent) = state.dir_browser.current_dir.parent().map(|p| p.to_path_buf()) {
                state.dir_browser.current_dir = parent;
                state.dir_browser.refresh_entries();
            }
        }
        KeyCode::Enter => {
            let is_dir_selected = state.dir_browser.entries
                .get(state.dir_browser.cursor)
                .map(|e| e.is_dir)
                .unwrap_or(false);
            if is_dir_selected {
                let name = state.dir_browser.entries[state.dir_browser.cursor].name.clone();
                state.dir_browser.current_dir = state.dir_browser.current_dir.join(&name);
                state.dir_browser.refresh_entries();
            } else {
                // Confirm export
                let path = state.dir_browser.current_dir.join(&state.dir_browser.filename);
                match state.storage.export_note(state.dir_browser.note_id, &path) {
                    Ok(written) => {
                        let msg = state.locale.status.note_exported
                            .replace("{path}", &written.to_string_lossy());
                        state.app.status_bar = msg;
                        let mut cfg = app_config::load();
                        cfg.last_export_dir = Some(state.dir_browser.current_dir.to_string_lossy().to_string());
                        let _ = app_config::save(&cfg);
                    }
                    Err(e) => {
                        state.app.status_bar = format!("Error: {}", e.message());
                    }
                }
                state.app.modal = None;
            }
        }
        KeyCode::Backspace => { state.dir_browser.filename.pop(); }
        KeyCode::Char(c) => { state.dir_browser.filename.push(c); }
        _ => {}
    }
}

pub(crate) fn render_export_note_modal(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let block = Block::default()
        .title(state.locale.modals.export_note.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 5 || inner.width < 10 {
        return;
    }

    let browser = &state.dir_browser;

    // Layout: path line (1) + entries list (variable) + separator (1) + filename (1) + hint (1)
    let path_area = Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 };
    let filename_area = Rect { x: inner.x, y: inner.y + inner.height - 2, width: inner.width, height: 1 };
    let hint_area = Rect { x: inner.x, y: inner.y + inner.height - 1, width: inner.width, height: 1 };
    let list_height = inner.height.saturating_sub(4);
    let list_area = Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: list_height };

    // Path breadcrumb
    let path_str = browser.current_dir.to_string_lossy();
    let path_display = if path_str.len() > inner.width as usize - 2 {
        format!("...{}", &path_str[path_str.len() - (inner.width as usize - 5)..])
    } else {
        path_str.to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(path_display, Style::default().fg(Color::Yellow)),
        ])),
        path_area,
    );

    // Entry list
    let visible_start = if browser.cursor >= list_height as usize {
        browser.cursor - list_height as usize + 1
    } else {
        0
    };
    let mut lines: Vec<Line> = Vec::new();
    for (i, entry) in browser.entries.iter().enumerate().skip(visible_start).take(list_height as usize) {
        let is_selected = i == browser.cursor;
        let icon = if entry.is_dir { " " } else { "  " };
        let style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else if entry.is_dir {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(format!(" {}{}", icon, entry.name), style)));
    }
    if browser.entries.is_empty() {
        lines.push(Line::from(Span::styled("  (empty)", Style::default().fg(Color::DarkGray))));
    }
    frame.render_widget(Paragraph::new(lines), list_area);

    // Filename field (always editable)
    let fn_style = Style::default().fg(Color::Cyan);
    let cursor_indicator = "│";
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" File: ", Style::default().fg(Color::DarkGray)),
            Span::styled(browser.filename.clone(), fn_style),
            Span::styled(cursor_indicator.to_string(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ])),
        filename_area,
    );

    // Hint
    frame.render_widget(
        Paragraph::new(Span::styled(
            state.locale.hints.export_note.clone(),
            Style::default().fg(Color::DarkGray),
        )),
        hint_area,
    );
}
