use rusqlite::{params, Connection};

use crate::domain::{
    Budget, Debt, DebtPatch, DomainError, FinCategory, Goal, GoalHorizon, GoalPatch,
    MonthlySummary, NewBudget, NewCategory, NewDebt, NewGoal, NewRecurringRule, NewTransaction,
    RecurringPeriod, RecurringRule, Transaction, TransactionFilter, TransactionType,
    finance::FinanceRepository,
};

use super::{SqliteStorage, map_err};

fn map_category(row: &rusqlite::Row<'_>) -> rusqlite::Result<FinCategory> {
    let tx_type_str: Option<String> = row.get(2)?;
    Ok(FinCategory {
        id: row.get(0)?,
        name: row.get(1)?,
        tx_type: tx_type_str.and_then(|s| s.parse().ok()),
    })
}

fn map_transaction(row: &rusqlite::Row<'_>) -> rusqlite::Result<Transaction> {
    let tx_type_str: String = row.get(2)?;
    Ok(Transaction {
        id: row.get(0)?,
        amount: row.get(1)?,
        tx_type: tx_type_str.parse().unwrap_or(TransactionType::Gasto),
        category_id: row.get(3)?,
        description: row.get(4)?,
        date: row.get(5)?,
        recurring_id: row.get(6)?,
    })
}

fn map_recurring(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecurringRule> {
    let tx_type_str: String = row.get(2)?;
    let period_str: String = row.get(5)?;
    Ok(RecurringRule {
        id: row.get(0)?,
        amount: row.get(1)?,
        tx_type: tx_type_str.parse().unwrap_or(TransactionType::Gasto),
        category_id: row.get(3)?,
        description: row.get(4)?,
        period: period_str.parse().unwrap_or(RecurringPeriod::Monthly),
        day_of_month: row.get(6)?,
        next_due: row.get(7)?,
        active: row.get::<_, i64>(8)? != 0,
    })
}

fn map_budget(row: &rusqlite::Row<'_>) -> rusqlite::Result<Budget> {
    Ok(Budget {
        id: row.get(0)?,
        category_id: row.get(1)?,
        monthly_limit: row.get(2)?,
        month: row.get(3)?,
    })
}

fn map_debt(row: &rusqlite::Row<'_>) -> rusqlite::Result<Debt> {
    Ok(Debt {
        id: row.get(0)?,
        creditor: row.get(1)?,
        total_amount: row.get(2)?,
        remaining_amount: row.get(3)?,
        interest_rate: row.get(4)?,
        monthly_payment: row.get(5)?,
        due_day: row.get(6)?,
        start_date: row.get(7)?,
    })
}

fn map_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Goal> {
    let horizon_str: String = row.get(5)?;
    Ok(Goal {
        id: row.get(0)?,
        name: row.get(1)?,
        target_amount: row.get(2)?,
        current_amount: row.get(3)?,
        deadline: row.get(4)?,
        horizon: horizon_str.parse().unwrap_or(GoalHorizon::Corto),
    })
}

impl FinanceRepository for SqliteStorage {
    // --- Categories ---

    fn list_categories(&self, tx_type: Option<TransactionType>) -> Result<Vec<FinCategory>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let (sql, binds): (String, Vec<Box<dyn rusqlite::ToSql>>) = match tx_type {
            Some(t) => (
                "SELECT id, name, tx_type FROM fin_categories WHERE tx_type IS NULL OR tx_type = ?1 ORDER BY name".into(),
                vec![Box::new(t.as_str().to_string())],
            ),
            None => (
                "SELECT id, name, tx_type FROM fin_categories ORDER BY name".into(),
                vec![],
            ),
        };
        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())), map_category).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    fn create_category(&self, input: NewCategory) -> Result<FinCategory, DomainError> {
        let conn = self.conn.lock().unwrap();
        let tx_type_str = input.tx_type.map(|t| t.as_str().to_string());
        conn.execute(
            "INSERT INTO fin_categories (name, tx_type) VALUES (?1, ?2)",
            params![input.name, tx_type_str],
        ).map_err(map_err)?;
        let id = conn.last_insert_rowid();
        conn.query_row("SELECT id, name, tx_type FROM fin_categories WHERE id=?1", params![id], map_category).map_err(map_err)
    }

    fn delete_category(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        let in_use: i64 = conn.query_row(
            "SELECT COUNT(*) FROM fin_transactions WHERE category_id = ?1", params![id], |r| r.get(0),
        ).unwrap_or(0);
        if in_use > 0 {
            return Err(DomainError::ValidationFailed(format!(
                "category {id} is used by {in_use} transactions, cannot delete"
            )));
        }
        let affected = conn.execute("DELETE FROM fin_categories WHERE id=?1", params![id]).map_err(map_err)?;
        if affected == 0 { return Err(DomainError::NotFound(format!("Category {id} not found"))); }
        Ok(())
    }

    // --- Transactions ---

    fn list_transactions(&self, filter: TransactionFilter) -> Result<Vec<Transaction>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut clauses: Vec<String> = vec![];
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![];

        if let Some(tx_type) = filter.tx_type {
            clauses.push("tx_type = ?".to_string());
            binds.push(Box::new(tx_type.as_str().to_string()));
        }
        if let Some(cat_id) = filter.category_id {
            clauses.push("category_id = ?".to_string());
            binds.push(Box::new(cat_id));
        }
        if let Some(from) = filter.from_date {
            clauses.push("date >= ?".to_string());
            binds.push(Box::new(from));
        }
        if let Some(to) = filter.to_date {
            clauses.push("date <= ?".to_string());
            binds.push(Box::new(to));
        }

        let where_clause = if clauses.is_empty() { String::new() } else { format!("WHERE {}", clauses.join(" AND ")) };
        let sql = format!(
            "SELECT id, amount, tx_type, category_id, description, date, recurring_id
             FROM fin_transactions {where_clause} ORDER BY date DESC, id DESC"
        );

        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())), map_transaction).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    fn create_transaction(&self, input: NewTransaction) -> Result<Transaction, DomainError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO fin_transactions (amount, tx_type, category_id, description, date, recurring_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![input.amount, input.tx_type.as_str(), input.category_id, input.description, input.date, input.recurring_id],
        ).map_err(map_err)?;
        let id = conn.last_insert_rowid();
        get_transaction(&conn, id)
    }

    fn delete_transaction(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("DELETE FROM fin_transactions WHERE id=?1", params![id]).map_err(map_err)?;
        if affected == 0 { return Err(DomainError::NotFound(format!("Transaction {id} not found"))); }
        Ok(())
    }

    fn monthly_summary(&self, month: &str) -> Result<MonthlySummary, DomainError> {
        let conn = self.conn.lock().unwrap();
        let from = format!("{month}-01");
        let to = format!("{month}-31");

        let income: i64 = conn.query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM fin_transactions WHERE tx_type='ingreso' AND date >= ?1 AND date <= ?2",
            params![from, to], |r| r.get(0),
        ).map_err(map_err)?;

        let expenses: i64 = conn.query_row(
            "SELECT COALESCE(SUM(amount), 0) FROM fin_transactions WHERE tx_type='gasto' AND date >= ?1 AND date <= ?2",
            params![from, to], |r| r.get(0),
        ).map_err(map_err)?;

        let balance = income - expenses;
        let savings_rate = if income > 0 { balance as f64 / income as f64 * 100.0 } else { 0.0 };
        Ok(MonthlySummary { month: month.to_string(), total_income: income, total_expenses: expenses, balance, savings_rate })
    }

    // --- Recurring ---

    fn list_recurring_rules(&self) -> Result<Vec<RecurringRule>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, amount, tx_type, category_id, description, period, day_of_month, next_due, active
             FROM fin_recurring_rules WHERE active = 1 ORDER BY next_due ASC"
        ).map_err(map_err)?;
        let rows = stmt.query_map([], map_recurring).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    fn create_recurring_rule(&self, input: NewRecurringRule) -> Result<RecurringRule, DomainError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO fin_recurring_rules (amount, tx_type, category_id, description, period, day_of_month, next_due)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![input.amount, input.tx_type.as_str(), input.category_id, input.description, input.period.as_str(), input.day_of_month, input.next_due],
        ).map_err(map_err)?;
        let id = conn.last_insert_rowid();
        get_recurring(&conn, id)
    }

    fn deactivate_recurring_rule(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("UPDATE fin_recurring_rules SET active = 0 WHERE id=?1", params![id]).map_err(map_err)?;
        if affected == 0 { return Err(DomainError::NotFound(format!("RecurringRule {id} not found"))); }
        Ok(())
    }

    fn pending_recurring_rules(&self, today: &str) -> Result<Vec<RecurringRule>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, amount, tx_type, category_id, description, period, day_of_month, next_due, active
             FROM fin_recurring_rules WHERE active = 1 AND next_due <= ?1"
        ).map_err(map_err)?;
        let rows = stmt.query_map(params![today], map_recurring).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    fn advance_recurring_next_due(&self, id: i64, new_next_due: &str) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE fin_recurring_rules SET next_due = ?1 WHERE id = ?2", params![new_next_due, id]).map_err(map_err)?;
        Ok(())
    }

    // --- Budgets ---

    fn list_budgets(&self, month: &str) -> Result<Vec<Budget>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, category_id, monthly_limit, month FROM fin_budgets WHERE month = ?1 ORDER BY category_id"
        ).map_err(map_err)?;
        let rows = stmt.query_map(params![month], map_budget).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    fn set_budget(&self, input: NewBudget) -> Result<Budget, DomainError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO fin_budgets (category_id, monthly_limit, month) VALUES (?1, ?2, ?3)
             ON CONFLICT(category_id, month) DO UPDATE SET monthly_limit = excluded.monthly_limit",
            params![input.category_id, input.monthly_limit, input.month],
        ).map_err(map_err)?;
        let id = conn.last_insert_rowid();
        conn.query_row("SELECT id, category_id, monthly_limit, month FROM fin_budgets WHERE rowid = ?1", params![id], map_budget).map_err(map_err)
    }

    fn delete_budget(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("DELETE FROM fin_budgets WHERE id=?1", params![id]).map_err(map_err)?;
        if affected == 0 { return Err(DomainError::NotFound(format!("Budget {id} not found"))); }
        Ok(())
    }

    fn budget_status(&self, month: &str) -> Result<Vec<(Budget, i64)>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let from = format!("{month}-01");
        let to = format!("{month}-31");
        let mut stmt = conn.prepare(
            "SELECT b.id, b.category_id, b.monthly_limit, b.month,
                    COALESCE((SELECT SUM(t.amount) FROM fin_transactions t
                              WHERE t.category_id = b.category_id AND t.tx_type = 'gasto'
                              AND t.date >= ?1 AND t.date <= ?2), 0)
             FROM fin_budgets b WHERE b.month = ?3 ORDER BY b.category_id"
        ).map_err(map_err)?;
        let rows = stmt.query_map(params![from, to, month], |row| {
            let budget = Budget { id: row.get(0)?, category_id: row.get(1)?, monthly_limit: row.get(2)?, month: row.get(3)? };
            let spent: i64 = row.get(4)?;
            Ok((budget, spent))
        }).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    // --- Debts ---

    fn list_debts(&self) -> Result<Vec<Debt>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, creditor, total_amount, remaining_amount, interest_rate, monthly_payment, due_day, start_date
             FROM fin_debts WHERE remaining_amount > 0 ORDER BY remaining_amount DESC"
        ).map_err(map_err)?;
        let rows = stmt.query_map([], map_debt).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    fn create_debt(&self, input: NewDebt) -> Result<Debt, DomainError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO fin_debts (creditor, total_amount, remaining_amount, interest_rate, monthly_payment, due_day, start_date)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![input.creditor, input.total_amount, input.remaining_amount, input.interest_rate, input.monthly_payment, input.due_day, input.start_date],
        ).map_err(map_err)?;
        let id = conn.last_insert_rowid();
        get_debt(&conn, id)
    }

    fn update_debt(&self, id: i64, patch: DebtPatch) -> Result<Debt, DomainError> {
        let conn = self.conn.lock().unwrap();
        let existing = get_debt(&conn, id)?;
        let remaining = patch.remaining_amount.unwrap_or(existing.remaining_amount);
        let payment = patch.monthly_payment.unwrap_or(existing.monthly_payment);
        conn.execute("UPDATE fin_debts SET remaining_amount=?1, monthly_payment=?2 WHERE id=?3", params![remaining, payment, id]).map_err(map_err)?;
        get_debt(&conn, id)
    }

    fn delete_debt(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("DELETE FROM fin_debts WHERE id=?1", params![id]).map_err(map_err)?;
        if affected == 0 { return Err(DomainError::NotFound(format!("Debt {id} not found"))); }
        Ok(())
    }

    // --- Goals ---

    fn list_goals(&self) -> Result<Vec<Goal>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, target_amount, current_amount, deadline, horizon FROM fin_goals ORDER BY horizon, name"
        ).map_err(map_err)?;
        let rows = stmt.query_map([], map_goal).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    fn create_goal(&self, input: NewGoal) -> Result<Goal, DomainError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO fin_goals (name, target_amount, current_amount, deadline, horizon) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![input.name, input.target_amount, input.current_amount, input.deadline, input.horizon.as_str()],
        ).map_err(map_err)?;
        let id = conn.last_insert_rowid();
        get_goal(&conn, id)
    }

    fn update_goal(&self, id: i64, patch: GoalPatch) -> Result<Goal, DomainError> {
        let conn = self.conn.lock().unwrap();
        let existing = get_goal(&conn, id)?;
        let current = patch.current_amount.unwrap_or(existing.current_amount);
        let target = patch.target_amount.unwrap_or(existing.target_amount);
        let deadline = match patch.deadline { Some(d) => d, None => existing.deadline };
        conn.execute("UPDATE fin_goals SET current_amount=?1, target_amount=?2, deadline=?3 WHERE id=?4", params![current, target, deadline, id]).map_err(map_err)?;
        get_goal(&conn, id)
    }

    fn delete_goal(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("DELETE FROM fin_goals WHERE id=?1", params![id]).map_err(map_err)?;
        if affected == 0 { return Err(DomainError::NotFound(format!("Goal {id} not found"))); }
        Ok(())
    }
}

fn get_transaction(conn: &Connection, id: i64) -> Result<Transaction, DomainError> {
    conn.query_row(
        "SELECT id, amount, tx_type, category_id, description, date, recurring_id FROM fin_transactions WHERE id=?1",
        params![id], map_transaction,
    ).map_err(map_err)
}

fn get_recurring(conn: &Connection, id: i64) -> Result<RecurringRule, DomainError> {
    conn.query_row(
        "SELECT id, amount, tx_type, category_id, description, period, day_of_month, next_due, active FROM fin_recurring_rules WHERE id=?1",
        params![id], map_recurring,
    ).map_err(map_err)
}

fn get_debt(conn: &Connection, id: i64) -> Result<Debt, DomainError> {
    conn.query_row(
        "SELECT id, creditor, total_amount, remaining_amount, interest_rate, monthly_payment, due_day, start_date FROM fin_debts WHERE id=?1",
        params![id], map_debt,
    ).map_err(map_err)
}

fn get_goal(conn: &Connection, id: i64) -> Result<Goal, DomainError> {
    conn.query_row(
        "SELECT id, name, target_amount, current_amount, deadline, horizon FROM fin_goals WHERE id=?1",
        params![id], map_goal,
    ).map_err(map_err)
}
