use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem},
};
use domain::{TaskFilter, TaskPatch, TaskStatus};
use domain::task::TaskRepository;
use domain::calendar::EventRepository;
use domain::group::GroupRepository;

use crate::modals::{open_new_event_modal, open_edit_event_modal, open_edit_task_modal, today_str, week_bounds, month_bounds};
use crate::state::*;
use jinx::app::Modal;
use jinx::calendario::{entry_count, flat_entries, nth_entry, FlatCalEntry};
use jinx::color::resolve_style;

use super::panel_block;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub(crate) fn handle_calendario_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = state.storage.list_events(None, None).unwrap_or_default();
    let mut view = jinx::calendario::calendar_layout(&tasks, &events);
    if let Some((from, to)) = calendar_date_range(state.calendar_filter_idx) {
        view.retain(|date, _| date.as_str() >= from.as_str() && date.as_str() <= to.as_str());
    }
    let flat = flat_entries(&view);
    let count = entry_count(&flat);

    if state.pending_g {
        state.pending_g = false;
        if key.code == KeyCode::Char('g') {
            state.calendar_cursor = 0;
            return;
        }
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') if state.calendar_cursor > 0 => {
            state.calendar_cursor -= 1;
        }
        KeyCode::Down | KeyCode::Char('j') if count > 0 && state.calendar_cursor + 1 < count => {
            state.calendar_cursor += 1;
        }
        KeyCode::Char('n') => open_new_event_modal(state),
        KeyCode::Char('e') => {
            if let Some(entry) = nth_entry(&flat, state.calendar_cursor) {
                if entry.is_task {
                    open_edit_task_modal(state, entry.entity_id);
                } else {
                    open_edit_event_modal(state, entry.entity_id);
                }
            }
        }
        KeyCode::Char('c') => {
            if let Some(entry) = nth_entry(&flat, state.calendar_cursor) {
                if entry.is_task {
                    let task_id = entry.entity_id;
                    let task = tasks.iter().find(|t| t.id == task_id);
                    let new_status = if task.map(|t| t.status) == Some(TaskStatus::Completada) {
                        TaskStatus::Pendiente
                    } else {
                        TaskStatus::Completada
                    };
                    match state.storage.update_task(
                        task_id,
                        TaskPatch { status: Some(new_status), ..Default::default() },
                    ) {
                        Ok(_) => {
                            state.app.status_bar = if new_status == TaskStatus::Completada {
                                state.locale.status.task_completed.clone()
                            } else {
                                state.locale.status.task_pending.clone()
                            };
                        }
                        Err(e) => state.app.status_bar = format!("Error: {}", e.message()),
                    }
                }
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);
            state.calendar_cursor = state.calendar_cursor.saturating_sub(half);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);
            state.calendar_cursor = (state.calendar_cursor + half).min(count.saturating_sub(1));
        }
        KeyCode::Char('d') => {
            if let Some(entry) = nth_entry(&flat, state.calendar_cursor) {
                state.delete_confirm_name = entry.text.clone();
                if entry.is_task {
                    state.app.modal = Some(Modal::DeleteTask { id: entry.entity_id });
                } else {
                    state.app.modal = Some(Modal::DeleteEvent { id: entry.entity_id });
                }
            }
        }
        KeyCode::Char('g') => { state.pending_g = true; }
        KeyCode::Char('G') => { state.calendar_cursor = count.saturating_sub(1); }
        KeyCode::Char('f') => {
            state.calendar_filter_idx = (state.calendar_filter_idx + 1) % 4;
            state.calendar_cursor = 0;
            state.calendar_scroll = 0;
            state.calendar_scroll_initialized = false;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn calendar_date_range(filter_idx: usize) -> Option<(String, String)> {
    match filter_idx {
        1 => { let t = today_str(); Some((t.clone(), t)) }
        2 => Some(week_bounds()),
        3 => Some(month_bounds()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

pub(crate) fn render_calendario(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let block = panel_block(state.locale.panels.calendar.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = state.storage.list_events(None, None).unwrap_or_default();
    let mut view = jinx::calendario::calendar_layout(&tasks, &events);
    if let Some((from, to)) = calendar_date_range(state.calendar_filter_idx) {
        view.retain(|date, _| date.as_str() >= from.as_str() && date.as_str() <= to.as_str());
    }
    let flat = flat_entries(&view);

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let groups = state.storage.list_groups().unwrap_or_default();
    let mut lines: Vec<ListItem> = vec![];

    if state.calendar_filter_idx > 0 {
        let filter_labels = [
            "",
            state.locale.filters.today.as_str(),
            state.locale.filters.this_week.as_str(),
            state.locale.filters.this_month.as_str(),
        ];
        let label = format!("  [{}]", filter_labels[state.calendar_filter_idx]);
        lines.push(ListItem::new(Line::from(Span::styled(
            label, Style::default().fg(Color::Magenta),
        ))));
    }
    let mut entry_idx = 0usize;
    let mut today_line_idx: Option<usize> = None;
    let mut today_entry_idx: Option<usize> = None;
    let mut current_date_is_today_or_later = false;
    let mut cursor_line_idx: usize = 0;

    for item in &flat {
        match item {
            FlatCalEntry::DateHeader(date) => {
                if today_line_idx.is_none() && date.as_str() >= today.as_str() {
                    today_line_idx = Some(lines.len());
                    current_date_is_today_or_later = true;
                }
                if date == &today {
                    let label = state.locale.misc.today_marker.replace("{date}", date);
                    lines.push(
                        ListItem::new(label)
                            .style(Style::default().fg(Color::Rgb(255, 20, 147)).add_modifier(Modifier::BOLD)),
                    );
                } else {
                    lines.push(
                        ListItem::new(date.as_str())
                            .style(Style::default().add_modifier(Modifier::BOLD)),
                    );
                }
            }
            FlatCalEntry::Entry(entry) => {
                if today_entry_idx.is_none() && current_date_is_today_or_later {
                    today_entry_idx = Some(entry_idx);
                }
                let selected = entry_idx == state.calendar_cursor;
                if selected {
                    cursor_line_idx = lines.len();
                }
                let cursor = if selected { "▶" } else { " " };

                let entry_style = if selected {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else if let Some(gid) = entry.group_id {
                    if let Some(g) = groups.iter().find(|g| g.id == gid) {
                        resolve_style(Some(&g.color), Some(&g.name), state.color_mode).style
                    } else {
                        Style::default()
                    }
                } else {
                    Style::default()
                };

                let label = format!("  {} {}", cursor, entry.text);
                lines.push(ListItem::new(label).style(entry_style));
                entry_idx += 1;
            }
        }
    }

    if flat.is_empty() {
        lines.push(ListItem::new(Line::from(Span::styled(
            state.locale.misc.no_calendar_entries.clone(),
            Style::default().fg(Color::DarkGray),
        ))));
    }

    lines.push(ListItem::new(Line::from(Span::styled(
        state.locale.hints.calendar_nav.clone(),
        Style::default().fg(Color::DarkGray),
    ))));

    let visible_height = inner.height as usize;

    if !state.calendar_scroll_initialized {
        if let Some(idx) = today_entry_idx {
            state.calendar_cursor = idx;
        }
        if let Some(line_idx) = today_line_idx {
            state.calendar_scroll = line_idx;
        }
        let max_scroll = lines.len().saturating_sub(visible_height);
        state.calendar_scroll = state.calendar_scroll.min(max_scroll);
        state.calendar_scroll_initialized = true;
    } else if !lines.is_empty() && visible_height > 0 {
        if cursor_line_idx < state.calendar_scroll {
            state.calendar_scroll = cursor_line_idx;
        } else if cursor_line_idx >= state.calendar_scroll + visible_height {
            state.calendar_scroll = cursor_line_idx - visible_height + 1;
        }
        let max_scroll = lines.len().saturating_sub(visible_height);
        state.calendar_scroll = state.calendar_scroll.min(max_scroll);
    }

    let end = (state.calendar_scroll + visible_height).min(lines.len());
    let visible_lines: Vec<ListItem> = lines.drain(state.calendar_scroll..end).collect();
    frame.render_widget(List::new(visible_lines), inner);
}
