use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use domain::{
    EventPatch, NewEvent,
    calendar::EventRepository,
};

use crate::state::*;
use jinx::text_editor::TextEditor;

pub(crate) fn open_new_event_modal(state: &mut RuntimeState) {
    crate::panels::refresh_groups_cache(state);
    state.event_form = EventFormState::default();
    state.app.modal = Some(jinx::app::Modal::NewEvent);
}

pub(crate) fn open_edit_event_modal(state: &mut RuntimeState, id: i64) {
    crate::panels::refresh_groups_cache(state);
    let events = state.storage.list_events(None, None).unwrap_or_default();
    if let Some(ev) = events.iter().find(|e| e.id == id) {
        let group_idx = ev.group_id
            .and_then(|gid| state.groups_cache.iter().position(|g| g.id == gid).map(|p| p + 1))
            .unwrap_or(0);
        let dur_str = ev.duration_minutes.map(|d| d.to_string()).unwrap_or_default();
        state.event_form = EventFormState {
            title: TextEditor::from_string(&ev.title),
            datetime: DateTimeInput::from_date_time_strings(&ev.start_date, &ev.start_time),
            duration: TextEditor::from_string(&dur_str),
            group_idx,
            field: 0,
            edit_id: Some(id),
            error: None,
        };
        state.app.modal = Some(jinx::app::Modal::EditEvent { id });
    }
}

pub(crate) fn handle_event_form_key(state: &mut RuntimeState, key: KeyEvent) {
    let n_fields = 4; // title, datetime, duration, group

    if state.event_form.field == 1 {
        match state.event_form.datetime.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => {
                state.event_form.field = (state.event_form.field + 1) % n_fields;
                return;
            }
            DateInputResult::PrevField => {
                state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields;
                return;
            }
            DateInputResult::Submit => {
                save_event(state);
                return;
            }
            DateInputResult::Cancel => {
                state.app.modal = None;
                state.event_form.error = None;
                return;
            }
        }
    }

    match key.code {
        KeyCode::Tab => state.event_form.field = (state.event_form.field + 1) % n_fields,
        KeyCode::BackTab => state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields,
        KeyCode::Down if state.event_form.field != 0 && state.event_form.field != 2 => {
            state.event_form.field = (state.event_form.field + 1) % n_fields;
        }
        KeyCode::Up if state.event_form.field != 0 && state.event_form.field != 2 => {
            state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left | KeyCode::Char('h') if state.event_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.event_form.group_idx = (state.event_form.group_idx + n - 1) % n;
        }
        KeyCode::Right | KeyCode::Char('l') if state.event_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.event_form.group_idx = (state.event_form.group_idx + 1) % n;
        }
        KeyCode::Char('j') if state.event_form.field == 3 => {
            state.event_form.field = (state.event_form.field + 1) % n_fields;
        }
        KeyCode::Char('k') if state.event_form.field == 3 => {
            state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left => match state.event_form.field {
            0 => { state.event_form.title.move_left(); }
            2 => { state.event_form.duration.move_left(); }
            _ => {}
        },
        KeyCode::Right => match state.event_form.field {
            0 => { state.event_form.title.move_right(); }
            2 => { state.event_form.duration.move_right(); }
            _ => {}
        },
        KeyCode::Char(c) => match state.event_form.field {
            0 => { state.event_form.title.insert_char(c); }
            2 => { state.event_form.duration.insert_char(c); }
            _ => {}
        },
        KeyCode::Backspace => match state.event_form.field {
            0 => { state.event_form.title.backspace(); }
            2 => { state.event_form.duration.backspace(); }
            _ => {}
        },
        KeyCode::Delete => match state.event_form.field {
            0 => { state.event_form.title.delete(); }
            2 => { state.event_form.duration.delete(); }
            _ => {}
        },
        KeyCode::Enter => save_event(state),
        KeyCode::Esc => { state.app.modal = None; state.event_form.error = None; }
        _ => {}
    }
}

pub(crate) fn save_event(state: &mut RuntimeState) {
    let form = state.event_form.clone();
    let title_text = form.title.to_string();
    let duration_text = form.duration.to_string();
    if title_text.trim().is_empty() {
        state.event_form.error = Some(state.locale.errors.title_empty.clone());
        return;
    }
    let start_date = form.datetime.to_date_string().unwrap_or_default();
    let start_time = form.datetime.to_time_string();
    if start_date.is_empty() {
        state.event_form.error = Some(state.locale.errors.start_date_required.clone());
        return;
    }
    let duration_minutes: Option<u32> = if duration_text.trim().is_empty() {
        None
    } else {
        match duration_text.trim().parse::<u32>() {
            Ok(d) => Some(d),
            Err(_) => { state.event_form.error = Some(state.locale.errors.duration_integer.clone()); return; }
        }
    };
    let group_id = if form.group_idx == 0 { None } else {
        state.groups_cache.get(form.group_idx - 1).map(|g| g.id)
    };

    let result = if let Some(id) = form.edit_id {
        state.storage.update_event(id, EventPatch {
            title: Some(title_text.trim().to_string()),
            start_date: Some(start_date.clone()),
            start_time: Some(start_time.clone()),
            duration_minutes: Some(duration_minutes),
            group_id: Some(group_id),
        }).map(|e| e.id)
    } else {
        state.storage.create_event(NewEvent {
            title: title_text.trim().to_string(),
            start_date,
            start_time,
            duration_minutes,
            group_id,
        }).map(|e| e.id)
    };

    match result {
        Ok(_event_id) => {
            state.app.modal = None;
            state.event_form = EventFormState::default();
            state.app.status_bar = state.locale.status.event_saved.clone();
        }
        Err(e) => state.event_form.error = Some(e.message()),
    }
}

pub(crate) fn render_event_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.event_form;
    let is_edit = form.edit_id.is_some();
    let title_str = if is_edit { &state.locale.modals.edit_event } else { &state.locale.modals.new_event };
    let block = Block::default().title(title_str.as_str()).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let groups: Vec<String> = std::iter::once(state.locale.filters.none.clone())
        .chain(state.groups_cache.iter().map(|g| g.name.clone()))
        .collect();

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(super::form_line_editor(state.locale.form_labels.title.as_str(), &form.title, form.field == 0));
    lines.push(date_input_line(
        state.locale.form_labels.datetime.as_str(), &form.datetime, form.field == 1,
        &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
    ));
    let dur_text = form.duration.to_string();
    lines.push(super::form_line_editor(state.locale.form_labels.duration_min.as_str(), &form.duration, form.field == 2));
    if form.field != 2 && dur_text.is_empty() {
        lines.pop();
        lines.push(super::form_line(state.locale.form_labels.duration_min.as_str(), state.locale.misc.empty_duration.clone(), false));
    }
    lines.push(super::form_line(
        state.locale.form_labels.group.as_str(),
        format!("← {} →", groups.get(form.group_idx).map(String::as_str).unwrap_or(state.locale.filters.none.as_str())),
        form.field == 3,
    ));
    lines.push(Line::from(""));
    if let Some(ref err) = form.error {
        lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        state.locale.hints.event_form.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}
