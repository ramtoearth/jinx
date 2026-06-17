mod task_form;
mod event_form;
mod group_form;
mod filter_form;
mod settings_form;
mod export_note;
mod delete_confirm;

pub(crate) use task_form::*;
pub(crate) use event_form::*;
pub(crate) use group_form::*;
pub(crate) use filter_form::*;
pub(crate) use settings_form::*;
pub(crate) use export_note::*;
pub(crate) use delete_confirm::*;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Clear,
};
use jinx_core::{
    task::TaskRepository, calendar::EventRepository, group::GroupRepository,
    note::NoteRepository,
};

use crate::state::*;
use jinx::app::Modal;
use jinx::text_editor::TextEditor;

pub(crate) fn handle_modal_key(state: &mut RuntimeState, key: KeyEvent) {
    let modal = state.app.modal.clone();
    match modal {
        Some(Modal::NewTask) | Some(Modal::EditTask { .. }) => handle_task_form_key(state, key),
        Some(Modal::NewEvent) | Some(Modal::EditEvent { .. }) => handle_event_form_key(state, key),
        Some(Modal::NewGroup) | Some(Modal::EditGroup { .. }) => handle_group_form_key(state, key),
        Some(Modal::DeleteTask { id }) => handle_delete_key(state, key, |s| {
            match s.storage.delete_task(id) {
                Ok(_) => { s.app.modal = None; s.app.status_bar = s.locale.status.task_deleted.clone(); if s.task_cursor > 0 { s.task_cursor -= 1; } }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        Some(Modal::DeleteEvent { id }) => handle_delete_key(state, key, |s| {
            match s.storage.delete_event(id) {
                Ok(_) => { s.app.modal = None; s.app.status_bar = s.locale.status.event_deleted.clone(); if s.calendar_cursor > 0 { s.calendar_cursor -= 1; } }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        Some(Modal::DeleteGroup { id }) => handle_delete_key(state, key, |s| {
            match s.storage.delete_group(id) {
                Ok(_) => { s.app.modal = None; s.app.status_bar = s.locale.status.group_deleted.clone(); if s.group_cursor > 0 { s.group_cursor -= 1; } }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        Some(Modal::DeleteNote { id }) => handle_delete_key(state, key, |s| {
            match s.storage.delete_note(id) {
                Ok(_) => {
                    s.app.modal = None;
                    s.app.status_bar = s.locale.status.note_deleted.clone();
                    s.notes_view = NotesView::List;
                    s.notes_current_id = None;
                    crate::panels::refresh_notes_cache(s);
                    if s.notes_cursor > 0 { s.notes_cursor -= 1; }
                }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        Some(Modal::Settings) => handle_settings_form_key(state, key),
        Some(Modal::FilterTasks) => handle_filter_form_key(state, key),
        Some(Modal::ExportNote { .. }) => handle_export_note_key(state, key),
        _ => { if key.code == KeyCode::Esc { state.app.modal = None; } }
    }
}

pub(crate) fn handle_modal_paste(state: &mut RuntimeState, data: &str) {
    let clean: String = data.chars().filter(|&c| c != '\n' && c != '\r').collect();
    match &state.app.modal {
        Some(Modal::NewTask) | Some(Modal::EditTask { .. })
            if state.task_form.field == 0 => { for c in clean.chars() { state.task_form.title.insert_char(c); } }
        Some(Modal::NewEvent) | Some(Modal::EditEvent { .. }) => match state.event_form.field {
            0 => { for c in clean.chars() { state.event_form.title.insert_char(c); } }
            2 => { for c in clean.chars() { state.event_form.duration.insert_char(c); } }
            _ => {}
        },
        Some(Modal::NewGroup) | Some(Modal::EditGroup { .. }) => match state.group_form.field {
            0 => { for c in clean.chars() { state.group_form.name.insert_char(c); } }
            1 => state.group_form.color_custom.push_str(&clean),
            _ => {}
        },
        Some(Modal::Settings) => {
            if let Some(ed) = settings_active_editor(state) {
                for c in clean.chars() { ed.insert_char(c); }
            }
        }
        _ => {}
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1])[1]
}

pub(crate) fn render_modal(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let popup = centered_rect(70, 70, area);
    frame.render_widget(Clear, popup);
    match &state.app.modal {
        Some(Modal::NewTask) | Some(Modal::EditTask { .. }) => render_task_form(frame, state, popup),
        Some(Modal::NewEvent) | Some(Modal::EditEvent { .. }) => render_event_form(frame, state, popup),
        Some(Modal::NewGroup) | Some(Modal::EditGroup { .. }) => render_group_form(frame, state, popup),
        Some(Modal::DeleteTask { .. }) | Some(Modal::DeleteEvent { .. }) | Some(Modal::DeleteGroup { .. }) | Some(Modal::DeleteNote { .. }) => {
            render_delete_confirm(frame, state, popup);
        }
        Some(Modal::Settings) => render_settings_form(frame, state, popup),
        Some(Modal::FilterTasks) => render_filter_form(frame, state, popup),
        Some(Modal::ExportNote { .. }) => render_export_note_modal(frame, state, popup),
        _ => {}
    }
}

pub(crate) fn form_line(label: &str, value: String, active: bool) -> Line<'static> {
    let (ls, vs) = if active {
        (Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
         Style::default().fg(Color::Cyan))
    } else {
        (Style::default().fg(Color::DarkGray), Style::default())
    };
    Line::from(vec![
        Span::styled(format!("  {:16}", label), ls),
        Span::styled(value, vs),
    ])
}

pub(crate) fn form_line_editor(label: &str, editor: &TextEditor, active: bool) -> Line<'static> {
    let (ls, vs) = if active {
        (Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
         Style::default().fg(Color::Cyan))
    } else {
        (Style::default().fg(Color::DarkGray), Style::default())
    };
    let text = editor.to_string();
    if active {
        let col = editor.cursor_col();
        let (before, after) = text.split_at(col.min(text.len()));
        Line::from(vec![
            Span::styled(format!("  {:16}", label), ls),
            Span::styled(before.to_string(), vs),
            Span::styled("│".to_string(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(after.to_string(), vs),
        ])
    } else {
        Line::from(vec![
            Span::styled(format!("  {:16}", label), ls),
            Span::styled(text, vs),
        ])
    }
}
