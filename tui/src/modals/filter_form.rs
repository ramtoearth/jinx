use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use domain::{Priority, TaskStatus};

use crate::state::*;

pub(crate) fn open_filter_modal(state: &mut RuntimeState) {
    crate::panels::refresh_groups_cache(state);
    let (date_idx, date_from, date_to) = if state.tareas_filter.no_deadline {
        (7, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
    } else {
        match (&state.tareas_filter.from_date, &state.tareas_filter.to_date) {
            (None, None) => (0, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled()),
            (Some(f), Some(t)) => {
                let today = today_str();
                let yesterday = yesterday_str();
                let (wk_m, wk_s) = week_bounds();
                let (lw_m, lw_s) = last_week_bounds();
                let (mo_f, mo_l) = month_bounds();
                if f == &today && t == &today {
                    (1, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else if f == &yesterday && t == &yesterday {
                    (2, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else if f == &wk_m && t == &wk_s {
                    (3, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else if f == &lw_m && t == &lw_s {
                    (4, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else if f == &mo_f && t == &mo_l {
                    (5, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else {
                    (6, DateTimeInput::from_iso(f, false), DateTimeInput::from_iso(t, false))
                }
            }
            (Some(f), None) => (6, DateTimeInput::from_iso(f, false), DateTimeInput::date_only_disabled()),
            (None, Some(t)) => (6, DateTimeInput::date_only_disabled(), DateTimeInput::from_iso(t, false)),
        }
    };
    let priority_sel = [
        state.tareas_filter.priorities.contains(&Priority::Alta),
        state.tareas_filter.priorities.contains(&Priority::Media),
        state.tareas_filter.priorities.contains(&Priority::Baja),
    ];
    state.filter_form = FilterFormState {
        status_idx: match state.tareas_filter.status {
            Some(TaskStatus::Pendiente) => 0,
            None => 1,
            Some(TaskStatus::Completada) => 2,
            Some(TaskStatus::Cancelada) => 3,
        },
        priority_sel,
        priority_cursor: 0,
        group_idx: match state.tareas_filter.group_id {
            None => 0,
            Some(Some(gid)) => {
                state.groups_cache.iter().position(|g| g.id == gid).map(|i| i + 1).unwrap_or(0)
            }
            Some(None) => state.groups_cache.len() + 1,
        },
        date_idx,
        date_from,
        date_to,
        field: 0,
    };
    state.app.modal = Some(jinx::app::Modal::FilterTasks);
}

pub(crate) fn handle_filter_form_key(state: &mut RuntimeState, key: KeyEvent) {
    let n_groups = state.groups_cache.len();
    let is_custom = state.filter_form.date_idx == 6;
    let n_fields: usize = if is_custom { 6 } else { 4 };

    if state.filter_form.field == 4 && is_custom {
        match state.filter_form.date_from.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => {
                state.filter_form.field = 5;
                return;
            }
            DateInputResult::PrevField => {
                state.filter_form.field = 3;
                return;
            }
            DateInputResult::Submit => {
                apply_filter(state);
                return;
            }
            DateInputResult::Cancel => {
                state.app.modal = None;
                return;
            }
        }
    }
    if state.filter_form.field == 5 && is_custom {
        match state.filter_form.date_to.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => {
                state.filter_form.field = 0;
                return;
            }
            DateInputResult::PrevField => {
                state.filter_form.field = 4;
                return;
            }
            DateInputResult::Submit => {
                apply_filter(state);
                return;
            }
            DateInputResult::Cancel => {
                state.app.modal = None;
                return;
            }
        }
    }

    match key.code {
        KeyCode::Tab => {
            let next = (state.filter_form.field + 1) % n_fields;
            state.filter_form.field = if !is_custom && next > 3 { 0 } else { next };
        }
        KeyCode::BackTab => {
            let prev = if state.filter_form.field == 0 { n_fields - 1 } else { state.filter_form.field - 1 };
            state.filter_form.field = if !is_custom && prev > 3 { 3 } else { prev };
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let next = (state.filter_form.field + 1) % n_fields;
            state.filter_form.field = if !is_custom && next > 3 { 0 } else { next };
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let prev = if state.filter_form.field == 0 { n_fields - 1 } else { state.filter_form.field - 1 };
            state.filter_form.field = if !is_custom && prev > 3 { 3 } else { prev };
        }
        KeyCode::Left | KeyCode::Char('h') => match state.filter_form.field {
            0 => state.filter_form.status_idx = (state.filter_form.status_idx + 3) % 4,
            1 => state.filter_form.priority_cursor = (state.filter_form.priority_cursor + 2) % 3,
            2 => {
                let n = n_groups + 2;
                state.filter_form.group_idx = (state.filter_form.group_idx + n - 1) % n;
            }
            3 => state.filter_form.date_idx = (state.filter_form.date_idx + 7) % 8,
            _ => {}
        },
        KeyCode::Right | KeyCode::Char('l') => match state.filter_form.field {
            0 => state.filter_form.status_idx = (state.filter_form.status_idx + 1) % 4,
            1 => state.filter_form.priority_cursor = (state.filter_form.priority_cursor + 1) % 3,
            2 => {
                let n = n_groups + 2;
                state.filter_form.group_idx = (state.filter_form.group_idx + 1) % n;
            }
            3 => state.filter_form.date_idx = (state.filter_form.date_idx + 1) % 8,
            _ => {}
        },
        KeyCode::Char(' ') if state.filter_form.field == 1 => {
            let c = state.filter_form.priority_cursor;
            state.filter_form.priority_sel[c] = !state.filter_form.priority_sel[c];
        }
        KeyCode::Char('r') => {
            state.filter_form = FilterFormState::default();
        }
        KeyCode::Enter => {
            if state.filter_form.field == 1 {
                let c = state.filter_form.priority_cursor;
                state.filter_form.priority_sel[c] = !state.filter_form.priority_sel[c];
            } else {
                apply_filter(state);
            }
        }
        KeyCode::Esc => {
            state.app.modal = None;
        }
        _ => {}
    }

    if state.filter_form.date_idx == 6 {
        state.filter_form.date_from.enabled = true;
        state.filter_form.date_to.enabled = true;
    }
}

pub(crate) fn today_str() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub(crate) fn yesterday_str() -> String {
    (chrono::Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

pub(crate) fn week_bounds() -> (String, String) {
    let now = chrono::Local::now();
    let weekday = now.format("%u").to_string().parse::<i64>().unwrap_or(1);
    let monday = now - chrono::Duration::days(weekday - 1);
    let sunday = monday + chrono::Duration::days(6);
    (
        monday.format("%Y-%m-%d").to_string(),
        sunday.format("%Y-%m-%d").to_string(),
    )
}

pub(crate) fn last_week_bounds() -> (String, String) {
    let now = chrono::Local::now();
    let weekday = now.format("%u").to_string().parse::<i64>().unwrap_or(1);
    let this_monday = now - chrono::Duration::days(weekday - 1);
    let last_monday = this_monday - chrono::Duration::days(7);
    let last_sunday = last_monday + chrono::Duration::days(6);
    (
        last_monday.format("%Y-%m-%d").to_string(),
        last_sunday.format("%Y-%m-%d").to_string(),
    )
}

pub(crate) fn month_bounds() -> (String, String) {
    let now = chrono::Local::now();
    let year: u32 = now.format("%Y").to_string().parse().unwrap_or(2026);
    let month: u32 = now.format("%m").to_string().parse().unwrap_or(1);
    let last_day = jinx::proximos::days_in_month(year, month);
    (
        format!("{:04}-{:02}-01", year, month),
        format!("{:04}-{:02}-{:02}", year, month, last_day),
    )
}

pub(crate) fn apply_filter(state: &mut RuntimeState) {
    let form = &state.filter_form;
    state.tareas_filter.status = match form.status_idx {
        0 => Some(TaskStatus::Pendiente),
        1 => None,
        2 => Some(TaskStatus::Completada),
        3 => Some(TaskStatus::Cancelada),
        _ => Some(TaskStatus::Pendiente),
    };
    let mut priorities = Vec::new();
    if form.priority_sel[0] { priorities.push(Priority::Alta); }
    if form.priority_sel[1] { priorities.push(Priority::Media); }
    if form.priority_sel[2] { priorities.push(Priority::Baja); }
    state.tareas_filter.priorities = priorities;
    let n_groups = state.groups_cache.len();
    state.tareas_filter.group_id = match form.group_idx {
        0 => None,
        i if i <= n_groups => Some(Some(state.groups_cache[i - 1].id)),
        _ => Some(None),
    };
    state.tareas_filter.no_deadline = form.date_idx == 7;
    let (from_date, to_date) = match form.date_idx {
        0 | 7 => (None, None),
        1 => { let t = today_str(); (Some(t.clone()), Some(t)) }
        2 => { let y = yesterday_str(); (Some(y.clone()), Some(y)) }
        3 => { let (m, s) = week_bounds(); (Some(m), Some(s)) }
        4 => { let (m, s) = last_week_bounds(); (Some(m), Some(s)) }
        5 => { let (f, l) = month_bounds(); (Some(f), Some(l)) }
        6 => (form.date_from.to_date_string(), form.date_to.to_date_string()),
        _ => (None, None),
    };
    state.tareas_filter.from_date = from_date;
    state.tareas_filter.to_date = to_date;
    state.task_cursor = 0;
    state.tareas_scroll = 0;
    state.tareas_search_active = false;
    state.tareas_search_query.clear();
    state.app.modal = None;
}

pub(crate) fn render_filter_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.filter_form;
    let block = Block::default()
        .title(state.locale.modals.filter_tasks.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let status_labels = [
        state.locale.filters.pending.as_str(),
        state.locale.filters.all.as_str(),
        state.locale.filters.completed.as_str(),
        state.locale.filters.cancelled.as_str(),
    ];
    let priority_labels = [
        state.locale.filters.high.as_str(),
        state.locale.filters.medium.as_str(),
        state.locale.filters.low.as_str(),
    ];

    let group_label = match form.group_idx {
        0 => state.locale.filters.all_groups.clone(),
        i if i <= state.groups_cache.len() => state.groups_cache[i - 1].name.clone(),
        _ => state.locale.filters.no_group.clone(),
    };

    let date_labels = [
        state.locale.filters.all.as_str(),
        state.locale.filters.today.as_str(),
        state.locale.filters.yesterday.as_str(),
        state.locale.filters.this_week.as_str(),
        state.locale.filters.last_week.as_str(),
        state.locale.filters.this_month.as_str(),
        state.locale.filters.custom.as_str(),
        state.locale.filters.no_date_filter.as_str(),
    ];

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(super::form_line(
        state.locale.form_labels.status.as_str(),
        format!("← {} →", status_labels[form.status_idx]),
        form.field == 0,
    ));

    // Priority multi-select: show checkboxes with cursor highlight
    {
        let field_active = form.field == 1;
        let label_style = if field_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut spans: Vec<Span<'static>> = vec![
            Span::styled(format!("  {:16}", state.locale.form_labels.priority.as_str()), label_style),
        ];
        for (i, plabel) in priority_labels.iter().enumerate() {
            let checked = if form.priority_sel[i] { "[x]" } else { "[ ]" };
            let is_cursor = field_active && form.priority_cursor == i;
            let style = if is_cursor {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else if form.priority_sel[i] {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!("{}{}", checked, plabel), style));
            if i < 2 { spans.push(Span::raw(" ")); }
        }
        if !form.priority_sel.iter().any(|&s| s) {
            spans.push(Span::styled(
                format!(" ({})", state.locale.filters.all.as_str()),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));
    }

    lines.push(super::form_line(
        state.locale.form_labels.group.as_str(),
        format!("← {} →", group_label),
        form.field == 2,
    ));
    lines.push(super::form_line(
        state.locale.form_labels.date.as_str(),
        format!("← {} →", date_labels[form.date_idx]),
        form.field == 3,
    ));
    if form.date_idx == 6 {
        lines.push(date_input_line(
            state.locale.form_labels.from_date.as_str(), &form.date_from, form.field == 4,
            &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
        ));
        lines.push(date_input_line(
            state.locale.form_labels.to_date.as_str(), &form.date_to, form.field == 5,
            &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        state.locale.hints.filter_form.clone(),
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}
