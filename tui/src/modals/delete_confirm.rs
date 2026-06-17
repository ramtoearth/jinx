use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::state::*;
use jinx::app::Modal;

pub(crate) fn handle_delete_key<F: FnOnce(&mut RuntimeState)>(
    state: &mut RuntimeState,
    key: KeyEvent,
    confirm_action: F,
) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => confirm_action(state),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            state.app.modal = None;
        }
        _ => {}
    }
}

pub(crate) fn render_delete_confirm(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let kind = match &state.app.modal {
        Some(Modal::DeleteTask { .. }) => state.locale.misc.task_kind.as_str(),
        Some(Modal::DeleteEvent { .. }) => state.locale.misc.event_kind.as_str(),
        Some(Modal::DeleteGroup { .. }) => state.locale.misc.group_kind.as_str(),
        _ => state.locale.misc.item_kind.as_str(),
    };
    let block = Block::default().title(state.locale.modals.confirm_delete.as_str()).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let confirm_text = state.locale.misc.delete_confirm
        .replace("{kind}", kind)
        .replace("{name}", &state.delete_confirm_name);

    let lines: Vec<Line<'static>> = vec![
        Line::from(""),
        Line::from(Span::styled(
            confirm_text,
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            state.locale.hints.delete_prompt.clone(),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
