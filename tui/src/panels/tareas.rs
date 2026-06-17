use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem},
};
use domain::{Priority, TaskPatch, TaskStatus};
use domain::task::TaskRepository;
use domain::group::GroupRepository;

use crate::modals::{open_new_task_modal, open_edit_task_modal, open_new_group_modal, open_edit_group_modal, open_filter_modal};
use crate::state::*;
use jinx::app::Modal;
use jinx::color::resolve_style;

use super::panel_block;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub(crate) fn handle_tareas_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    if state.tareas_search_active {
        handle_tareas_search_key(state, key);
        return;
    }

    if key.code == KeyCode::Char('s') {
        state.pending_g = false;
        state.tareas_section = match state.tareas_section {
            TareasSection::Tasks => TareasSection::Groups,
            TareasSection::Groups => TareasSection::Tasks,
        };
        return;
    }

    match state.tareas_section {
        TareasSection::Tasks => handle_tareas_tasks_key(state, key),
        TareasSection::Groups => handle_tareas_groups_key(state, key),
    }
}

pub(crate) fn handle_tareas_tasks_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let tasks = if !state.tareas_search_query.is_empty() {
        get_search_filtered_tasks(state)
    } else {
        get_filtered_tasks(state)
    };
    if state.pending_g {
        state.pending_g = false;
        if key.code == KeyCode::Char('g') {
            state.task_cursor = 0;
            return;
        }
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') if state.task_cursor > 0 => {
            state.task_cursor -= 1;
        }
        KeyCode::Down | KeyCode::Char('j') if state.task_cursor + 1 < tasks.len() => {
            state.task_cursor += 1;
        }
        KeyCode::Char('n') => open_new_task_modal(state),
        KeyCode::Char('e') => {
            if let Some(t) = tasks.get(state.task_cursor) {
                open_edit_task_modal(state, t.id);
            }
        }
        KeyCode::Char('c') => {
            if let Some(t) = tasks.get(state.task_cursor) {
                let task_id = t.id;
                let new_status = if t.status == TaskStatus::Completada {
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
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);
            state.task_cursor = state.task_cursor.saturating_sub(half);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);
            state.task_cursor = (state.task_cursor + half).min(tasks.len().saturating_sub(1));
        }
        KeyCode::Char('d') => {
            if let Some(t) = tasks.get(state.task_cursor) {
                state.delete_confirm_name = t.title.clone();
                state.app.modal = Some(Modal::DeleteTask { id: t.id });
            }
        }
        KeyCode::Char('g') => { state.pending_g = true; }
        KeyCode::Char('G') => { state.task_cursor = tasks.len().saturating_sub(1); }
        KeyCode::Char('f') => open_filter_modal(state),
        KeyCode::Char('/') => {
            state.tareas_search_active = true;
            state.tareas_search_query.clear();
            state.task_cursor = 0;
            state.tareas_scroll = 0;
        }
        _ => {}
    }
}

pub(crate) fn handle_tareas_search_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            state.tareas_search_active = false;
            state.tareas_search_query.clear();
            state.task_cursor = 0;
            state.tareas_scroll = 0;
        }
        KeyCode::Enter => {
            state.tareas_search_active = false;
        }
        KeyCode::Backspace => {
            state.tareas_search_query.pop();
            state.task_cursor = 0;
            state.tareas_scroll = 0;
        }
        KeyCode::Up if state.task_cursor > 0 => {
            state.task_cursor -= 1;
        }
        KeyCode::Down => {
            let count = get_search_filtered_tasks(state).len();
            if state.task_cursor + 1 < count {
                state.task_cursor += 1;
            }
        }
        KeyCode::Char(c) => {
            state.tareas_search_query.push(c);
            state.task_cursor = 0;
            state.tareas_scroll = 0;
        }
        _ => {}
    }
}

pub(crate) fn get_search_filtered_tasks(state: &RuntimeState) -> Vec<domain::Task> {
    let mut tasks = get_filtered_tasks(state);
    if !state.tareas_search_query.is_empty() {
        let query = state.tareas_search_query.to_lowercase();
        tasks.retain(|t| t.title.to_lowercase().contains(&query));
    }
    tasks
}

pub(crate) fn handle_tareas_groups_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let groups = state.storage.list_groups().unwrap_or_default();
    if state.pending_g {
        state.pending_g = false;
        if key.code == KeyCode::Char('g') {
            state.group_cursor = 0;
            return;
        }
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') if state.group_cursor > 0 => {
            state.group_cursor -= 1;
        }
        KeyCode::Down | KeyCode::Char('j') if state.group_cursor + 1 < groups.len() => {
            state.group_cursor += 1;
        }
        KeyCode::Char('n') => open_new_group_modal(state),
        KeyCode::Char('g') => { state.pending_g = true; }
        KeyCode::Char('G') => { state.group_cursor = groups.len().saturating_sub(1); }
        KeyCode::Char('e') => {
            if let Some(g) = groups.get(state.group_cursor) {
                open_edit_group_modal(state, g.id);
            }
        }
        KeyCode::Char('d') => {
            if let Some(g) = groups.get(state.group_cursor) {
                state.delete_confirm_name = g.name.clone();
                state.app.modal = Some(Modal::DeleteGroup { id: g.id });
            }
        }
        _ => {}
    }
}

pub(crate) fn get_filtered_tasks(state: &RuntimeState) -> Vec<domain::Task> {
    let mut tasks = state
        .storage
        .list_tasks(state.tareas_filter.to_storage_filter())
        .unwrap_or_default();
    if !state.tareas_filter.priorities.is_empty() {
        tasks.retain(|t| state.tareas_filter.priorities.contains(&t.priority));
    }
    tasks
}

pub(crate) fn refresh_groups_cache(state: &mut RuntimeState) {
    state.groups_cache = state.storage.list_groups().unwrap_or_default();
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

pub(crate) fn render_tareas(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let block = panel_block(state.locale.panels.tasks.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let groups = state.storage.list_groups().unwrap_or_default();
    let groups_height = (groups.len() + 3).min(8) as u16;

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(groups_height)])
        .split(inner);

    // --- Tasks section ---
    let tasks = if !state.tareas_search_query.is_empty() || state.tareas_search_active {
        get_search_filtered_tasks(state)
    } else {
        get_filtered_tasks(state)
    };
    let mut task_items: Vec<ListItem> = Vec::new();

    if state.tareas_search_active {
        let search_line = format!(" /{}\u{2502}", state.tareas_search_query);
        task_items.push(ListItem::new(Line::from(Span::styled(
            search_line,
            Style::default().fg(Color::Green),
        ))));
    }

    if !state.tareas_filter.is_default() {
        let status_label = match state.tareas_filter.status {
            Some(TaskStatus::Pendiente) => state.locale.filters.pending.as_str(),
            Some(TaskStatus::Completada) => state.locale.filters.completed.as_str(),
            Some(TaskStatus::Cancelada) => state.locale.filters.cancelled.as_str(),
            None => state.locale.filters.all.as_str(),
        };
        let priority_label: String = if state.tareas_filter.priorities.is_empty() {
            state.locale.filters.all.to_string()
        } else {
            state.tareas_filter.priorities.iter().map(|p| match p {
                Priority::Alta => state.locale.filters.high.as_str(),
                Priority::Media => state.locale.filters.medium.as_str(),
                Priority::Baja => state.locale.filters.low.as_str(),
            }).collect::<Vec<_>>().join("+")
        };
        let group_label = match &state.tareas_filter.group_id {
            None => state.locale.filters.all_groups.clone(),
            Some(None) => state.locale.filters.no_group.clone(),
            Some(Some(gid)) => groups
                .iter()
                .find(|g| g.id == *gid)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| "?".to_string()),
        };
        let date_label = if state.tareas_filter.no_deadline {
            format!("  {}:{}", state.locale.form_labels.date.trim(), state.locale.filters.no_date_filter.as_str())
        } else {
            match (&state.tareas_filter.from_date, &state.tareas_filter.to_date) {
                (None, None) => String::new(),
                (Some(f), Some(t)) if f == t => format!("  {}:{}", state.locale.form_labels.date.trim(), f),
                (Some(f), Some(t)) => format!("  {}:{}/{}", state.locale.form_labels.date.trim(), f, t),
                (Some(f), None) => format!("  {}:{}", state.locale.form_labels.from_date.trim(), f),
                (None, Some(t)) => format!("  {}:{}", state.locale.form_labels.to_date.trim(), t),
            }
        };
        let filter_line = state.locale.filters.filters_prefix
            .replace("{status}", status_label)
            .replace("{priority}", &priority_label)
            .replace("{group}", &group_label)
            .replace("{date}", &date_label);
        task_items.push(ListItem::new(Line::from(Span::styled(
            filter_line,
            Style::default().fg(Color::Yellow),
        ))));
    }

    for (i, t) in tasks.iter().enumerate() {
        let cursor = if state.tareas_section == TareasSection::Tasks && i == state.task_cursor {
            "▶"
        } else {
            " "
        };

        let group_indicator = if let Some(gid) = t.group_id {
            if let Some(g) = groups.iter().find(|g| g.id == gid) {
                let styled = resolve_style(Some(&g.color), Some(&g.name), state.color_mode);
                if let Some(prefix) = &styled.prefix {
                    format!("{} ", prefix)
                } else {
                    "██ ".to_string()
                }
            } else {
                "   ".to_string()
            }
        } else {
            "   ".to_string()
        };

        let deadline_str: String = t.deadline.as_deref().map(|d| {
            if let Some(pos) = d.find('T') {
                let date = &d[..pos];
                let time_part = &d[pos + 1..];
                let hm: &str = if time_part.len() >= 5 { &time_part[..5] } else { time_part };
                if hm == "00:00" {
                    date.to_string()
                } else {
                    format!("{} {}", date, hm)
                }
            } else {
                d.to_string()
            }
        }).map(|d| format!(" ({})", d)).unwrap_or_default();

        let label = format!(
            " {} {}[{}] {}{}",
            cursor, group_indicator, t.priority.as_str(), t.title, deadline_str
        );

        let base_style = if state.tareas_section == TareasSection::Tasks && i == state.task_cursor {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else if let Some(gid) = t.group_id {
            if let Some(g) = groups.iter().find(|g| g.id == gid) {
                resolve_style(Some(&g.color), Some(&g.name), state.color_mode).style
            } else {
                Style::default()
            }
        } else {
            Style::default()
        };
        task_items.push(ListItem::new(label).style(base_style));
    }

    if tasks.is_empty() {
        task_items.push(ListItem::new(Line::from(Span::styled(
            state.locale.misc.no_tasks.clone(),
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let task_hint = if state.tareas_search_active {
        state.locale.hints.tasks_search.as_str()
    } else if state.tareas_section == TareasSection::Tasks {
        state.locale.hints.tasks_nav.as_str()
    } else {
        state.locale.hints.tasks_switch_to_groups.as_str()
    };
    task_items.push(ListItem::new(Line::from(Span::styled(
        task_hint.to_string(),
        Style::default().fg(Color::DarkGray),
    ))));

    // Scroll: keep cursor visible within available height
    let visible_height = sections[0].height as usize;
    let filter_offset = if !state.tareas_filter.is_default() { 1 } else { 0 };
    let cursor_line = if state.tareas_section == TareasSection::Tasks {
        state.task_cursor + filter_offset
    } else {
        0
    };
    if visible_height > 0 && !task_items.is_empty() {
        if cursor_line < state.tareas_scroll {
            state.tareas_scroll = cursor_line;
        } else if cursor_line >= state.tareas_scroll + visible_height {
            state.tareas_scroll = cursor_line - visible_height + 1;
        }
        let max_scroll = task_items.len().saturating_sub(visible_height);
        state.tareas_scroll = state.tareas_scroll.min(max_scroll);
    }
    let end = (state.tareas_scroll + visible_height).min(task_items.len());
    let visible_items: Vec<ListItem> = task_items.drain(state.tareas_scroll..end).collect();
    frame.render_widget(List::new(visible_items), sections[0]);

    // --- Groups section ---
    let mut group_items: Vec<ListItem> = Vec::new();
    group_items.push(ListItem::new(Line::from(Span::styled(
        state.locale.misc.groups_separator.clone(),
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    ))));

    if groups.is_empty() {
        group_items.push(ListItem::new(state.locale.misc.no_groups.clone()));
    } else {
        for (i, g) in groups.iter().enumerate() {
            let selected = state.tareas_section == TareasSection::Groups && i == state.group_cursor;
            let cursor = if selected { "▶" } else { " " };
            let label = format!(" {} {} ({})", cursor, g.name, g.color);
            let style = if selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                resolve_style(Some(&g.color), Some(&g.name), state.color_mode).style
            };
            group_items.push(ListItem::new(label).style(style));
        }
    }

    let group_hint = if state.tareas_section == TareasSection::Groups {
        state.locale.hints.groups_nav.as_str()
    } else {
        state.locale.hints.groups_switch_to_tasks.as_str()
    };
    group_items.push(ListItem::new(Line::from(Span::styled(
        group_hint.to_string(),
        Style::default().fg(Color::DarkGray),
    ))));

    frame.render_widget(List::new(group_items), sections[1]);
}
