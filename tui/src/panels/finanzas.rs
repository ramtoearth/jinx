use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph},
};

use crate::state::*;
use super::panel_block;

pub(crate) fn handle_finanzas_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Left => {
            state.finance_month = prev_month(&state.finance_month);
        }
        KeyCode::Right => {
            state.finance_month = next_month(&state.finance_month);
        }
        KeyCode::Char('n') => {
            crate::modals::open_transaction_modal(state);
        }
        KeyCode::Char('d') => {
            crate::modals::open_debt_modal(state);
        }
        KeyCode::Char('g') => {
            crate::modals::open_goal_modal(state);
        }
        KeyCode::Char('b') => {
            crate::modals::open_budget_modal(state);
        }
        _ => {}
    }
}

pub(crate) fn render_finanzas(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let block = panel_block("Finanzas");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let month = &state.finance_month;

    let summary = state.services.finance.monthly_summary(month).ok();
    let budget_items = state.services.finance.budget_status(month).unwrap_or_default();
    let goals = state.services.finance.list_goals().unwrap_or_default();
    let debts = state.services.finance.list_debts().unwrap_or_default();
    let categories = state.services.finance.list_categories(None).unwrap_or_default();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // summary
            Constraint::Min(4),    // budget + goals + debts
        ])
        .split(inner);

    // --- Monthly summary ---
    let (income, expenses, balance, rate) = match &summary {
        Some(s) => (s.total_income, s.total_expenses, s.balance, s.savings_rate),
        None => (0, 0, 0, 0.0),
    };

    let summary_lines = vec![
        Line::from(vec![
            Span::styled(format!("  ← {} →", month), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  Ingresos: "),
            Span::styled(format_money(income), Style::default().fg(Color::Green)),
            Span::raw("    Gastos: "),
            Span::styled(format_money(expenses), Style::default().fg(Color::Red)),
            Span::raw("    Balance: "),
            Span::styled(format_money(balance), if balance >= 0 { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }),
            Span::raw(format!("    Ahorro: {:.1}%", rate)),
        ]),
    ];
    frame.render_widget(Paragraph::new(summary_lines), chunks[0]);

    // --- Budget + Goals + Debts in remaining space ---
    let bottom_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40), // budget
            Constraint::Percentage(35), // goals
            Constraint::Percentage(25), // debts
        ])
        .split(chunks[1]);

    // Budget bars
    let mut budget_lines: Vec<ListItem> = vec![
        ListItem::new(Line::from(Span::styled(
            "  Presupuesto", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ))),
    ];
    if budget_items.is_empty() {
        budget_lines.push(ListItem::new("    (sin presupuesto configurado)"));
    } else {
        for (b, spent) in &budget_items {
            let pct = if b.monthly_limit > 0 { (*spent as f64 / b.monthly_limit as f64 * 100.0).min(100.0) } else { 0.0 };
            let bar = progress_bar(pct, 10);
            let color = if pct > 90.0 { Color::Red } else if pct > 70.0 { Color::Yellow } else { Color::Green };
            let cat_name = categories.iter().find(|c| c.id == b.category_id).map(|c| c.name.as_str()).unwrap_or("?");
            budget_lines.push(ListItem::new(Line::from(vec![
                Span::raw(format!("    {:<14}", cat_name)),
                Span::styled(bar, Style::default().fg(color)),
                Span::raw(format!("  {} / {}", format_money(*spent), format_money(b.monthly_limit))),
            ])));
        }
    }
    frame.render_widget(List::new(budget_lines), bottom_chunks[0]);

    // Goals
    let mut goal_lines: Vec<ListItem> = vec![
        ListItem::new(Line::from(Span::styled(
            "  Metas", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        ))),
    ];
    if goals.is_empty() {
        goal_lines.push(ListItem::new("    (sin metas)"));
    } else {
        for g in &goals {
            let pct = if g.target_amount > 0 { (g.current_amount as f64 / g.target_amount as f64 * 100.0).min(100.0) } else { 0.0 };
            let bar = progress_bar(pct, 10);
            goal_lines.push(ListItem::new(Line::from(vec![
                Span::raw(format!("    {:<14}", g.name)),
                Span::styled(bar, Style::default().fg(Color::Cyan)),
                Span::raw(format!("  {} / {}", format_money(g.current_amount), format_money(g.target_amount))),
            ])));
        }
    }
    frame.render_widget(List::new(goal_lines), bottom_chunks[1]);

    // Debts
    let mut debt_lines: Vec<ListItem> = vec![
        ListItem::new(Line::from(Span::styled(
            "  Deudas", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ))),
    ];
    if debts.is_empty() {
        debt_lines.push(ListItem::new("    (sin deudas)"));
    } else {
        for d in &debts {
            let rate_str = d.interest_rate.map(|r| format!(" ({:.1}%)", r)).unwrap_or_default();
            debt_lines.push(ListItem::new(Line::from(vec![
                Span::raw(format!("    {:<14}", d.creditor)),
                Span::raw(format!("{} restante", format_money(d.remaining_amount))),
                Span::styled(rate_str, Style::default().fg(Color::DarkGray)),
            ])));
        }
    }
    frame.render_widget(List::new(debt_lines), bottom_chunks[2]);
}

fn format_money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let whole = abs / 100;
    let frac = abs % 100;
    format!("{sign}${whole}.{frac:02}")
}

fn progress_bar(pct: f64, width: usize) -> String {
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

fn prev_month(m: &str) -> String {
    let parts: Vec<&str> = m.split('-').collect();
    if parts.len() < 2 { return m.to_string(); }
    let y: i32 = parts[0].parse().unwrap_or(2026);
    let mo: i32 = parts[1].parse().unwrap_or(1);
    let (ny, nm) = if mo == 1 { (y - 1, 12) } else { (y, mo - 1) };
    format!("{ny:04}-{nm:02}")
}

fn next_month(m: &str) -> String {
    let parts: Vec<&str> = m.split('-').collect();
    if parts.len() < 2 { return m.to_string(); }
    let y: i32 = parts[0].parse().unwrap_or(2026);
    let mo: i32 = parts[1].parse().unwrap_or(1);
    let (ny, nm) = if mo == 12 { (y + 1, 1) } else { (y, mo + 1) };
    format!("{ny:04}-{nm:02}")
}
