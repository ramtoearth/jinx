use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};
use jinx_core::{GoalHorizon, NewBudget, NewCategory, NewDebt, NewGoal, NewTransaction, TransactionType};

use crate::state::*;

const TX_FIELDS: usize = 5;
const DEBT_FIELDS: usize = 6;
const GOAL_FIELDS: usize = 5;
const BUDGET_FIELDS: usize = 2;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn refresh_categories(state: &mut RuntimeState) {
    let tx_type = if state.transaction_form.tx_type_idx == 0 {
        Some(TransactionType::Gasto)
    } else {
        Some(TransactionType::Ingreso)
    };
    state.finance_categories = state.services.finance.list_categories(tx_type).unwrap_or_default();
}

// ---------------------------------------------------------------------------
// Open helpers
// ---------------------------------------------------------------------------

pub(crate) fn open_transaction_modal(state: &mut RuntimeState) {
    state.transaction_form = TransactionFormState::default();
    refresh_categories(state);
    state.app.modal = Some(jinx::app::Modal::NewTransaction);
}

pub(crate) fn open_debt_modal(state: &mut RuntimeState) {
    state.debt_form = DebtFormState::default();
    state.app.modal = Some(jinx::app::Modal::NewDebt);
}

pub(crate) fn open_goal_modal(state: &mut RuntimeState) {
    state.goal_form = GoalFormState::default();
    state.app.modal = Some(jinx::app::Modal::NewGoal);
}

pub(crate) fn open_budget_modal(state: &mut RuntimeState) {
    state.budget_form = BudgetFormState::default();
    state.finance_categories = state.services.finance.list_categories(Some(TransactionType::Gasto)).unwrap_or_default();
    state.app.modal = Some(jinx::app::Modal::EditBudget);
}

// ---------------------------------------------------------------------------
// Transaction form
// ---------------------------------------------------------------------------

pub(crate) fn handle_transaction_form_key(state: &mut RuntimeState, key: KeyEvent) {
    // Adding new category mode
    if state.transaction_form.category_adding {
        match key.code {
            KeyCode::Esc => { state.transaction_form.category_adding = false; }
            KeyCode::Enter => {
                let name = state.transaction_form.category_new_name.trim().to_string();
                if !name.is_empty() {
                    let tx_type = if state.transaction_form.tx_type_idx == 0 {
                        Some(TransactionType::Gasto)
                    } else {
                        Some(TransactionType::Ingreso)
                    };
                    if let Ok(cat) = state.services.finance.create_category(NewCategory { name, tx_type }) {
                        refresh_categories(state);
                        // Select the new one
                        if let Some(pos) = state.finance_categories.iter().position(|c| c.id == cat.id) {
                            state.transaction_form.category_idx = pos;
                        }
                    }
                }
                state.transaction_form.category_adding = false;
                state.transaction_form.category_new_name.clear();
            }
            KeyCode::Char(c) => { state.transaction_form.category_new_name.push(c); }
            KeyCode::Backspace => { state.transaction_form.category_new_name.pop(); }
            _ => {}
        }
        return;
    }

    // Date field
    if state.transaction_form.field == 4 {
        match state.transaction_form.date.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => { state.transaction_form.field = 0; return; }
            DateInputResult::PrevField => { state.transaction_form.field = 3; return; }
            DateInputResult::Submit => { save_transaction(state); return; }
            DateInputResult::Cancel => { state.app.modal = None; return; }
        }
    }

    match key.code {
        KeyCode::Esc => { state.app.modal = None; }
        KeyCode::Enter => save_transaction(state),
        KeyCode::Tab | KeyCode::Down => {
            state.transaction_form.field = (state.transaction_form.field + 1) % TX_FIELDS;
            if state.transaction_form.field == 2 { refresh_categories(state); }
        }
        KeyCode::BackTab | KeyCode::Up => {
            state.transaction_form.field = (state.transaction_form.field + TX_FIELDS - 1) % TX_FIELDS;
            if state.transaction_form.field == 2 { refresh_categories(state); }
        }
        // Type selector
        KeyCode::Left | KeyCode::Right if state.transaction_form.field == 1 => {
            state.transaction_form.tx_type_idx = 1 - state.transaction_form.tx_type_idx;
            refresh_categories(state);
            state.transaction_form.category_idx = 0;
        }
        // Category selector
        KeyCode::Left if state.transaction_form.field == 2 => {
            let len = state.finance_categories.len();
            if len > 0 {
                state.transaction_form.category_idx = (state.transaction_form.category_idx + len - 1) % len;
            }
        }
        KeyCode::Right if state.transaction_form.field == 2 => {
            let len = state.finance_categories.len();
            if len > 0 {
                state.transaction_form.category_idx = (state.transaction_form.category_idx + 1) % len;
            }
        }
        KeyCode::Char('a') if state.transaction_form.field == 2 => {
            state.transaction_form.category_adding = true;
            state.transaction_form.category_new_name.clear();
        }
        KeyCode::Char('d') if state.transaction_form.field == 2 => {
            if let Some(cat) = state.finance_categories.get(state.transaction_form.category_idx) {
                let _ = state.services.finance.delete_category(cat.id);
                refresh_categories(state);
                if state.transaction_form.category_idx > 0 && state.finance_categories.is_empty() {
                    state.transaction_form.category_idx = 0;
                } else if state.transaction_form.category_idx >= state.finance_categories.len() && !state.finance_categories.is_empty() {
                    state.transaction_form.category_idx = state.finance_categories.len() - 1;
                }
            }
        }
        // Text fields
        KeyCode::Char(c) => {
            match state.transaction_form.field {
                0 => state.transaction_form.amount.push(c),
                3 => state.transaction_form.description.push(c),
                _ => {}
            }
        }
        KeyCode::Backspace => {
            match state.transaction_form.field {
                0 => { state.transaction_form.amount.pop(); }
                3 => { state.transaction_form.description.pop(); }
                _ => {}
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Debt form
// ---------------------------------------------------------------------------

pub(crate) fn handle_debt_form_key(state: &mut RuntimeState, key: KeyEvent) {
    if state.debt_form.field == 5 {
        match state.debt_form.start_date.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => { state.debt_form.field = 0; return; }
            DateInputResult::PrevField => { state.debt_form.field = 4; return; }
            DateInputResult::Submit => { save_debt(state); return; }
            DateInputResult::Cancel => { state.app.modal = None; return; }
        }
    }

    match key.code {
        KeyCode::Esc => { state.app.modal = None; }
        KeyCode::Enter => save_debt(state),
        KeyCode::Tab | KeyCode::Down => {
            state.debt_form.field = (state.debt_form.field + 1) % DEBT_FIELDS;
        }
        KeyCode::BackTab | KeyCode::Up => {
            state.debt_form.field = (state.debt_form.field + DEBT_FIELDS - 1) % DEBT_FIELDS;
        }
        KeyCode::Char(c) => {
            match state.debt_form.field {
                0 => state.debt_form.creditor.push(c),
                1 => state.debt_form.total_amount.push(c),
                2 => state.debt_form.interest_rate.push(c),
                3 => state.debt_form.monthly_payment.push(c),
                4 => state.debt_form.due_day.push(c),
                _ => {}
            }
        }
        KeyCode::Backspace => {
            match state.debt_form.field {
                0 => { state.debt_form.creditor.pop(); }
                1 => { state.debt_form.total_amount.pop(); }
                2 => { state.debt_form.interest_rate.pop(); }
                3 => { state.debt_form.monthly_payment.pop(); }
                4 => { state.debt_form.due_day.pop(); }
                _ => {}
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Goal form
// ---------------------------------------------------------------------------

pub(crate) fn handle_goal_form_key(state: &mut RuntimeState, key: KeyEvent) {
    if state.goal_form.field == 3 {
        match state.goal_form.deadline.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => { state.goal_form.field = 4; return; }
            DateInputResult::PrevField => { state.goal_form.field = 2; return; }
            DateInputResult::Submit => { save_goal(state); return; }
            DateInputResult::Cancel => { state.app.modal = None; return; }
        }
    }

    match key.code {
        KeyCode::Esc => { state.app.modal = None; }
        KeyCode::Enter => save_goal(state),
        KeyCode::Tab | KeyCode::Down => {
            state.goal_form.field = (state.goal_form.field + 1) % GOAL_FIELDS;
        }
        KeyCode::BackTab | KeyCode::Up => {
            state.goal_form.field = (state.goal_form.field + GOAL_FIELDS - 1) % GOAL_FIELDS;
        }
        KeyCode::Left | KeyCode::Right if state.goal_form.field == 4 => {
            state.goal_form.horizon_idx = (state.goal_form.horizon_idx + 1) % 3;
        }
        KeyCode::Char(c) => {
            match state.goal_form.field {
                0 => state.goal_form.name.push(c),
                1 => state.goal_form.target_amount.push(c),
                2 => state.goal_form.current_amount.push(c),
                _ => {}
            }
        }
        KeyCode::Backspace => {
            match state.goal_form.field {
                0 => { state.goal_form.name.pop(); }
                1 => { state.goal_form.target_amount.pop(); }
                2 => { state.goal_form.current_amount.pop(); }
                _ => {}
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Budget form
// ---------------------------------------------------------------------------

pub(crate) fn handle_budget_form_key(state: &mut RuntimeState, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => { state.app.modal = None; }
        KeyCode::Enter => save_budget(state),
        KeyCode::Tab | KeyCode::Down => {
            state.budget_form.field = (state.budget_form.field + 1) % BUDGET_FIELDS;
        }
        KeyCode::BackTab | KeyCode::Up => {
            state.budget_form.field = (state.budget_form.field + BUDGET_FIELDS - 1) % BUDGET_FIELDS;
        }
        KeyCode::Left if state.budget_form.field == 0 => {
            let len = state.finance_categories.len();
            if len > 0 { state.budget_form.category_idx = (state.budget_form.category_idx + len - 1) % len; }
        }
        KeyCode::Right if state.budget_form.field == 0 => {
            let len = state.finance_categories.len();
            if len > 0 { state.budget_form.category_idx = (state.budget_form.category_idx + 1) % len; }
        }
        KeyCode::Char(c) if state.budget_form.field == 1 => {
            state.budget_form.monthly_limit.push(c);
        }
        KeyCode::Backspace if state.budget_form.field == 1 => {
            state.budget_form.monthly_limit.pop();
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Save logic
// ---------------------------------------------------------------------------

fn save_transaction(state: &mut RuntimeState) {
    let form = &state.transaction_form;
    let amount_cents: i64 = match parse_money(&form.amount) {
        Some(v) => v,
        None => { state.transaction_form.error = Some("Monto inválido".into()); return; }
    };
    let category_id = match state.finance_categories.get(form.category_idx) {
        Some(cat) => cat.id,
        None => { state.transaction_form.error = Some("Selecciona una categoría".into()); return; }
    };
    let tx_type = if form.tx_type_idx == 0 { TransactionType::Gasto } else { TransactionType::Ingreso };
    let date = form.date.to_date_string().unwrap_or_else(|| {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    });

    match state.services.finance.create_transaction(NewTransaction {
        amount: amount_cents, tx_type, category_id,
        description: form.description.clone(), date, recurring_id: None,
    }) {
        Ok(_) => {
            state.app.modal = None;
            state.app.status_bar = "Transacción registrada".into();
        }
        Err(e) => { state.transaction_form.error = Some(e.message()); }
    }
}

fn save_debt(state: &mut RuntimeState) {
    let form = &state.debt_form;
    let total = match parse_money(&form.total_amount) {
        Some(v) => v,
        None => { state.debt_form.error = Some("Monto total inválido".into()); return; }
    };
    let payment = match parse_money(&form.monthly_payment) {
        Some(v) => v,
        None => { state.debt_form.error = Some("Pago mensual inválido".into()); return; }
    };
    if form.creditor.trim().is_empty() {
        state.debt_form.error = Some("Acreedor requerido".into());
        return;
    }
    let rate: Option<f64> = if form.interest_rate.is_empty() { None } else {
        match form.interest_rate.parse() {
            Ok(r) => Some(r),
            Err(_) => { state.debt_form.error = Some("Tasa inválida".into()); return; }
        }
    };
    let due_day: Option<u32> = if form.due_day.is_empty() { None } else { form.due_day.parse().ok() };
    let start_date = form.start_date.to_date_string().unwrap_or_else(|| {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    });

    match state.services.finance.create_debt(NewDebt {
        creditor: form.creditor.trim().to_string(), total_amount: total,
        remaining_amount: total, interest_rate: rate,
        monthly_payment: payment, due_day, start_date,
    }) {
        Ok(_) => { state.app.modal = None; state.app.status_bar = "Deuda registrada".into(); }
        Err(e) => { state.debt_form.error = Some(e.message()); }
    }
}

fn save_goal(state: &mut RuntimeState) {
    let form = &state.goal_form;
    let target = match parse_money(&form.target_amount) {
        Some(v) => v,
        None => { state.goal_form.error = Some("Monto objetivo inválido".into()); return; }
    };
    let current = if form.current_amount.is_empty() { 0 } else {
        match parse_money(&form.current_amount) {
            Some(v) => v,
            None => { state.goal_form.error = Some("Monto actual inválido".into()); return; }
        }
    };
    if form.name.trim().is_empty() {
        state.goal_form.error = Some("Nombre requerido".into());
        return;
    }
    let horizons = [GoalHorizon::Corto, GoalHorizon::Mediano, GoalHorizon::Largo];
    let deadline = form.deadline.to_date_string();

    match state.services.finance.create_goal(NewGoal {
        name: form.name.trim().to_string(), target_amount: target,
        current_amount: current, deadline, horizon: horizons[form.horizon_idx],
    }) {
        Ok(_) => { state.app.modal = None; state.app.status_bar = "Meta creada".into(); }
        Err(e) => { state.goal_form.error = Some(e.message()); }
    }
}

fn save_budget(state: &mut RuntimeState) {
    let form = &state.budget_form;
    let limit = match parse_money(&form.monthly_limit) {
        Some(v) => v,
        None => { state.budget_form.error = Some("Límite inválido".into()); return; }
    };
    let category_id = match state.finance_categories.get(form.category_idx) {
        Some(cat) => cat.id,
        None => { state.budget_form.error = Some("Selecciona una categoría".into()); return; }
    };

    match state.services.finance.set_budget(NewBudget {
        category_id, monthly_limit: limit, month: state.finance_month.clone(),
    }) {
        Ok(_) => { state.app.modal = None; state.app.status_bar = "Presupuesto guardado".into(); }
        Err(e) => { state.budget_form.error = Some(e.message()); }
    }
}

// ponytail: parse "150.50" or "150" to cents. No locale handling.
fn parse_money(s: &str) -> Option<i64> {
    let s = s.trim().replace(',', ".");
    if s.is_empty() { return None; }
    if let Some(dot) = s.find('.') {
        let whole: i64 = s[..dot].parse().ok()?;
        let frac_str = &s[dot + 1..];
        let frac: i64 = match frac_str.len() {
            0 => 0,
            1 => frac_str.parse::<i64>().ok()? * 10,
            _ => frac_str[..2].parse().ok()?,
        };
        Some(whole * 100 + frac)
    } else {
        let whole: i64 = s.parse().ok()?;
        Some(whole * 100)
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub(crate) fn render_transaction_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.transaction_form;
    let types = ["← Gasto →", "← Ingreso →"];

    let cat_display = if form.category_adding {
        format!("Nuevo: {}▌", form.category_new_name)
    } else if state.finance_categories.is_empty() {
        "(sin categorías — a:agregar)".to_string()
    } else {
        let name = &state.finance_categories[form.category_idx.min(state.finance_categories.len().saturating_sub(1))].name;
        format!("← {} →  (a:agregar  d:eliminar)", name)
    };

    let mut lines = vec![
        super::form_line("Monto", form.amount.clone(), form.field == 0),
        super::form_line("Tipo", types[form.tx_type_idx].to_string(), form.field == 1),
        super::form_line("Categoría", cat_display, form.field == 2),
        super::form_line("Descripción", form.description.clone(), form.field == 3),
        date_input_line(
            "Fecha", &form.date, form.field == 4,
            &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
        ),
    ];
    if let Some(err) = &form.error {
        lines.push(Line::from(ratatui::text::Span::styled(err.as_str(), Style::default().fg(Color::Red))));
    }
    let block = Block::default().title("Nueva Transacción").borders(Borders::ALL);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

pub(crate) fn render_debt_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.debt_form;
    let mut lines = vec![
        super::form_line("Acreedor", form.creditor.clone(), form.field == 0),
        super::form_line("Monto total", form.total_amount.clone(), form.field == 1),
        super::form_line("Tasa interés %", form.interest_rate.clone(), form.field == 2),
        super::form_line("Pago mensual", form.monthly_payment.clone(), form.field == 3),
        super::form_line("Día vencimiento", form.due_day.clone(), form.field == 4),
        date_input_line(
            "Fecha inicio", &form.start_date, form.field == 5,
            &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
        ),
    ];
    if let Some(err) = &form.error {
        lines.push(Line::from(ratatui::text::Span::styled(err.as_str(), Style::default().fg(Color::Red))));
    }
    let block = Block::default().title("Nueva Deuda").borders(Borders::ALL);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

pub(crate) fn render_goal_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.goal_form;
    let horizons = ["← Corto →", "← Mediano →", "← Largo →"];
    let mut lines = vec![
        super::form_line("Nombre", form.name.clone(), form.field == 0),
        super::form_line("Monto objetivo", form.target_amount.clone(), form.field == 1),
        super::form_line("Monto actual", form.current_amount.clone(), form.field == 2),
        date_input_line(
            "Fecha límite", &form.deadline, form.field == 3,
            &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
        ),
        super::form_line("Horizonte", horizons[form.horizon_idx].to_string(), form.field == 4),
    ];
    if let Some(err) = &form.error {
        lines.push(Line::from(ratatui::text::Span::styled(err.as_str(), Style::default().fg(Color::Red))));
    }
    let block = Block::default().title("Nueva Meta").borders(Borders::ALL);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

pub(crate) fn render_budget_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.budget_form;
    let cat_display = if state.finance_categories.is_empty() {
        "(sin categorías)".to_string()
    } else {
        let name = &state.finance_categories[form.category_idx.min(state.finance_categories.len().saturating_sub(1))].name;
        format!("← {} →", name)
    };
    let mut lines = vec![
        super::form_line("Categoría", cat_display, form.field == 0),
        super::form_line("Límite mensual", form.monthly_limit.clone(), form.field == 1),
    ];
    if let Some(err) = &form.error {
        lines.push(Line::from(ratatui::text::Span::styled(err.as_str(), Style::default().fg(Color::Red))));
    }
    let block = Block::default().title("Presupuesto").borders(Borders::ALL);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
