use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use domain::{
    NewTask, Priority, TaskFilter, TaskPatch, TaskStatus,
    task::TaskRepository,
};

use crate::state::*;
use jinx::text_editor::TextEditor;

pub(crate) fn open_new_task_modal(state: &mut RuntimeState) {
    super::super::refresh_groups_cache(state);
    state.task_form = TaskFormState { priority_idx: 1, ..Default::default() };
    state.app.modal = Some(jinx::app::Modal::NewTask);
}

pub(crate) fn open_edit_task_modal(state: &mut RuntimeState, id: i64) {
    super::super::refresh_groups_cache(state);
    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    if let Some(t) = tasks.iter().find(|t| t.id == id) {
        let priority_idx = match t.priority {
            Priority::Alta => 0,
            Priority::Media => 1,
            Priority::Baja => 2,
        };
        let status_idx = match t.status {
            TaskStatus::Pendiente => 0,
            TaskStatus::Completada => 1,
            TaskStatus::Cancelada => 2,
        };
        let group_idx = t.group_id
            .and_then(|gid| state.groups_cache.iter().position(|g| g.id == gid).map(|p| p + 1))
            .unwrap_or(0);
        let deadline = match &t.deadline {
            Some(s) => DateTimeInput::from_iso(s, true),
            None => DateTimeInput::date_time_disabled(),
        };
        state.task_form = TaskFormState {
            title: TextEditor::from_string(&t.title),
            priority_idx,
            deadline,
            group_idx,
            status_idx,
            field: 0,
            edit_id: Some(id),
            error: None,
        };
        state.app.modal = Some(jinx::app::Modal::EditTask { id });
    }
}

pub(crate) fn handle_task_form_key(state: &mut RuntimeState, key: KeyEvent) {
    let is_edit = state.task_form.edit_id.is_some();
    let n_fields = if is_edit { 5 } else { 4 };

    if state.task_form.field == 2 {
        match state.task_form.deadline.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => {
                state.task_form.field = (state.task_form.field + 1) % n_fields;
                return;
            }
            DateInputResult::PrevField => {
                state.task_form.field = (state.task_form.field + n_fields - 1) % n_fields;
                return;
            }
            DateInputResult::Submit => {
                save_task(state);
                return;
            }
            DateInputResult::Cancel => {
                state.app.modal = None;
                state.task_form.error = None;
                return;
            }
        }
    }

    match key.code {
        KeyCode::Tab => state.task_form.field = (state.task_form.field + 1) % n_fields,
        KeyCode::BackTab => state.task_form.field = (state.task_form.field + n_fields - 1) % n_fields,
        KeyCode::Down if state.task_form.field != 0 => {
            state.task_form.field = (state.task_form.field + 1) % n_fields;
        }
        KeyCode::Up if state.task_form.field != 0 => {
            state.task_form.field = (state.task_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left | KeyCode::Char('h') if state.task_form.field == 1 => {
            state.task_form.priority_idx = (state.task_form.priority_idx + 2) % 3;
        }
        KeyCode::Left | KeyCode::Char('h') if state.task_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.task_form.group_idx = (state.task_form.group_idx + n - 1) % n;
        }
        KeyCode::Left | KeyCode::Char('h') if state.task_form.field == 4 => {
            state.task_form.status_idx = (state.task_form.status_idx + 2) % 3;
        }
        KeyCode::Right | KeyCode::Char('l') if state.task_form.field == 1 => {
            state.task_form.priority_idx = (state.task_form.priority_idx + 1) % 3;
        }
        KeyCode::Right | KeyCode::Char('l') if state.task_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.task_form.group_idx = (state.task_form.group_idx + 1) % n;
        }
        KeyCode::Right | KeyCode::Char('l') if state.task_form.field == 4 => {
            state.task_form.status_idx = (state.task_form.status_idx + 1) % 3;
        }
        KeyCode::Char('j') if state.task_form.field != 0 => {
            state.task_form.field = (state.task_form.field + 1) % n_fields;
        }
        KeyCode::Char('k') if state.task_form.field != 0 => {
            state.task_form.field = (state.task_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left if state.task_form.field == 0 => { state.task_form.title.move_left(); }
        KeyCode::Right if state.task_form.field == 0 => { state.task_form.title.move_right(); }
        KeyCode::Char(c) if state.task_form.field == 0 => { state.task_form.title.insert_char(c); }
        KeyCode::Backspace if state.task_form.field == 0 => { state.task_form.title.backspace(); }
        KeyCode::Delete if state.task_form.field == 0 => { state.task_form.title.delete(); }
        KeyCode::Enter => save_task(state),
        KeyCode::Esc => { state.app.modal = None; state.task_form.error = None; }
        _ => {}
    }
}

pub(crate) fn save_task(state: &mut RuntimeState) {
    let form = state.task_form.clone();
    let title_text = form.title.to_string();
    if title_text.trim().is_empty() {
        state.task_form.error = Some(state.locale.errors.title_empty.clone());
        return;
    }
    let priorities = [Priority::Alta, Priority::Media, Priority::Baja];
    let statuses = [TaskStatus::Pendiente, TaskStatus::Completada, TaskStatus::Cancelada];
    let priority = priorities[form.priority_idx];
    let deadline = form.deadline.to_iso_string();
    let group_id = if form.group_idx == 0 { None } else {
        state.groups_cache.get(form.group_idx - 1).map(|g| g.id)
    };

    let result = if let Some(id) = form.edit_id {
        state.storage.update_task(id, TaskPatch {
            title: Some(title_text.trim().to_string()),
            priority: Some(priority),
            deadline: Some(deadline),
            group_id: Some(group_id),
            status: Some(statuses[form.status_idx]),
        }).map(|t| t.id)
    } else {
        state.storage.create_task(NewTask {
            title: title_text.trim().to_string(),
            priority: Some(priority),
            deadline,
            group_id,
        }).map(|t| t.id)
    };

    match result {
        Ok(_task_id) => {
            state.app.modal = None;
            state.task_form = TaskFormState::default();
            state.app.status_bar = state.locale.status.task_saved.clone();
        }
        Err(e) => state.task_form.error = Some(e.message()),
    }
}

pub(crate) fn render_task_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.task_form;
    let is_edit = form.edit_id.is_some();
    let title_str = if is_edit { &state.locale.modals.edit_task } else { &state.locale.modals.new_task };
    let block = Block::default().title(title_str.as_str()).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let priorities = [
        state.locale.filters.high.as_str(),
        state.locale.filters.medium.as_str(),
        state.locale.filters.low.as_str(),
    ];
    let statuses = [
        state.locale.filters.pending.as_str(),
        state.locale.filters.completed.as_str(),
        state.locale.filters.cancelled.as_str(),
    ];
    let groups: Vec<String> = std::iter::once(state.locale.filters.none.clone())
        .chain(state.groups_cache.iter().map(|g| g.name.clone()))
        .collect();

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(super::form_line_editor(state.locale.form_labels.title.as_str(), &form.title, form.field == 0));
    lines.push(super::form_line(state.locale.form_labels.priority.as_str(), format!("← {} →", priorities[form.priority_idx]), form.field == 1));
    lines.push(date_input_line(
        state.locale.form_labels.deadline.as_str(), &form.deadline, form.field == 2,
        &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
    ));
    lines.push(super::form_line(
        state.locale.form_labels.group.as_str(),
        format!("← {} →", groups.get(form.group_idx).map(String::as_str).unwrap_or(state.locale.filters.none.as_str())),
        form.field == 3,
    ));
    if is_edit {
        lines.push(super::form_line(state.locale.form_labels.status.as_str(), format!("← {} →", statuses[form.status_idx]), form.field == 4));
    }
    lines.push(Line::from(""));
    if let Some(ref err) = form.error {
        lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        state.locale.hints.task_form.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}
