use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use jinx_core::{
    HexColor, NewGroup,
    group::GroupRepository,
};

use crate::state::*;
use jinx::text_editor::TextEditor;

pub(crate) fn open_new_group_modal(state: &mut RuntimeState) {
    state.group_form = GroupFormState::default();
    state.app.modal = Some(jinx::app::Modal::NewGroup);
}

pub(crate) fn open_edit_group_modal(state: &mut RuntimeState, id: i64) {
    let groups = state.storage.list_groups().unwrap_or_default();
    if let Some(g) = groups.iter().find(|g| g.id == id) {
        let color_str = g.color.to_string();
        let color_idx = COLOR_PRESETS.iter().position(|&p| p == color_str).unwrap_or(0);
        let color_custom = if COLOR_PRESETS.contains(&color_str.as_str()) {
            String::new()
        } else {
            color_str
        };
        state.group_form = GroupFormState {
            name: TextEditor::from_string(&g.name),
            color_idx,
            color_custom,
            edit_id: Some(id),
            ..Default::default()
        };
        state.app.modal = Some(jinx::app::Modal::EditGroup { id });
    }
}

pub(crate) fn handle_group_form_key(state: &mut RuntimeState, key: KeyEvent) {
    match key.code {
        KeyCode::Tab => state.group_form.field = (state.group_form.field + 1) % 2,
        KeyCode::BackTab => state.group_form.field = (state.group_form.field + 1) % 2,
        KeyCode::Down | KeyCode::Up => {
            state.group_form.field = (state.group_form.field + 1) % 2;
        }
        KeyCode::Left | KeyCode::Char('h') if state.group_form.field == 1 && state.group_form.color_custom.is_empty() => {
            state.group_form.color_idx = (state.group_form.color_idx + COLOR_PRESETS.len() - 1) % COLOR_PRESETS.len();
        }
        KeyCode::Right | KeyCode::Char('l') if state.group_form.field == 1 && state.group_form.color_custom.is_empty() => {
            state.group_form.color_idx = (state.group_form.color_idx + 1) % COLOR_PRESETS.len();
        }
        KeyCode::Char('j') | KeyCode::Char('k') if state.group_form.field == 1 && state.group_form.color_custom.is_empty() => {
            state.group_form.field = (state.group_form.field + 1) % 2;
        }
        KeyCode::Left if state.group_form.field == 0 => {
            state.group_form.name.move_left();
        }
        KeyCode::Right if state.group_form.field == 0 => {
            state.group_form.name.move_right();
        }
        KeyCode::Char(c) => match state.group_form.field {
            0 => { state.group_form.name.insert_char(c); }
            1 => { state.group_form.color_custom.push(c); }
            _ => {}
        },
        KeyCode::Backspace => match state.group_form.field {
            0 => { state.group_form.name.backspace(); }
            1 => { state.group_form.color_custom.pop(); }
            _ => {}
        },
        KeyCode::Delete if state.group_form.field == 0 => { state.group_form.name.delete(); }
        KeyCode::Enter => save_group(state),
        KeyCode::Esc => { state.app.modal = None; state.group_form.error = None; }
        _ => {}
    }
}

pub(crate) fn save_group(state: &mut RuntimeState) {
    let form = state.group_form.clone();
    let name_text = form.name.to_string();
    if name_text.trim().is_empty() {
        state.group_form.error = Some(state.locale.errors.name_empty.clone());
        return;
    }
    let color_str = form.effective_color().to_string();
    let color = match HexColor::new(&color_str) {
        Ok(c) => c,
        Err(_) => { state.group_form.error = Some(state.locale.errors.color_invalid.clone()); return; }
    };

    let result: Result<_, _> = if let Some(id) = form.edit_id {
        state.storage.rename_group(id, name_text.trim().to_string())
            .and_then(|_| state.storage.recolor_group(id, color))
            .map(|_| ())
    } else {
        state.storage.create_group(NewGroup { name: name_text.trim().to_string(), color }).map(|_| ())
    };

    match result {
        Ok(()) => {
            state.app.modal = None;
            state.group_form = GroupFormState::default();
            state.app.status_bar = state.locale.status.group_saved.clone();
        }
        Err(e) => state.group_form.error = Some(e.message()),
    }
}

pub(crate) fn render_group_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.group_form;
    let is_edit = form.edit_id.is_some();
    let title_str = if is_edit { &state.locale.modals.edit_group } else { &state.locale.modals.new_group };
    let block = Block::default().title(title_str.as_str()).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let color_display = if form.color_custom.is_empty() {
        state.locale.misc.preset_color
            .replace("{color}", COLOR_PRESETS[form.color_idx % COLOR_PRESETS.len()])
            .replace("{idx}", &(form.color_idx + 1).to_string())
            .replace("{total}", &COLOR_PRESETS.len().to_string())
    } else {
        state.locale.misc.custom_color.replace("{color}", &form.color_custom)
    };

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(super::form_line_editor(state.locale.form_labels.name.as_str(), &form.name, form.field == 0));
    lines.push(super::form_line(state.locale.form_labels.color.as_str(), color_display, form.field == 1));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        state.locale.hints.color_hint.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    if let Some(ref err) = form.error {
        lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        state.locale.hints.group_form.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}
