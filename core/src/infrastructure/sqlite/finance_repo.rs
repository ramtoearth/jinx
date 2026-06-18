use rusqlite::{params, Connection};

use crate::domain::{
    Budget, Debt, DebtPatch, DomainError, Goal, GoalHorizon, GoalPatch, MonthlySummary,
    NewBudget, NewDebt, NewGoal, NewRecurringRule, NewTransaction, RecurringPeriod, RecurringRule,
    Transaction, TransactionFilter, TransactionType,
    finance::FinanceRepository,
};

use super::{SqliteStorage, map_err};

fn map_transaction(row: &rusqlite::Row<'_>) -> rusqlite::Result<Transaction> {
    let tx_type_str: String = row.get(2)?;
    Ok(Transaction {
        id: row.get(0)?,
        amount: row.get(1)?,
        tx_type: tx_type_str.parse().unwrap_or(TransactionType::Gasto),
        category: row.get(3)?,
        description: row.get(4)?,
        date: row.get(5)?,
        recurring_id: row.get(6)?,
        group_id: row.get(7)?,
    })
}

fn map_recurring(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecurringRule> {
    let tx_type_str: String = row.get(2)?;
    let period_str: String = row.get(5)?;
    Ok(RecurringRule {
        id: row.get(0)?,
        amount: row.get(1)?,
        tx_type: tx_type_str.parse().unwrap_or(TransactionType::Gasto),
        category: row.get(3)?,
        description: row.get(4)?,
        period: period_str.parse().unwrap_or(RecurringPeriod::Monthly),
        day_of_month: row.get(6)?,
        next_due: row.get(7)?,
        active: row.get::<_, i64>(8)? != 0,
        group_id: row.get(9)?,
    })
}

fn map_budget(row: &rusqlite::Row<'_>) -> rusqlite::Result<Budget> {
    Ok(Budget {
        id: row.get(0)?,
        category: row.get(1)?,
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
    fn list_transactions(&self, filter: TransactionFilter) -> Result<Vec<Transaction>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut clauses: Vec<String> = vec![];
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![];

        if let Some(tx_type) = filter.tx_type {
            clauses.push("tx_type = ?".to_string());
            binds.push(Box::new(tx_type.as_str().to_string()));
        }
        if let Some(cat) = filter.category {
            clauses.push("category = ?".to_string());
            binds.push(Box::new(cat));
        }
        if let Some(from) = filter.from_date {
            clauses.push("date >= ?".to_string());
            binds.push(Box::new(from));
        }
        if let Some(to) = filter.to_date {
            clauses.push("date <= ?".to_string());
            binds.push(Box::new(to));
        }
        if let Some(gid) = filter.group_id {
            match gid {
                Some(id) => { clauses.push("group_id = ?".to_string()); binds.push(Box::new(id)); }
                None => { clauses.push("group_id IS NULL".to_string()); }
            }
        }

        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };

        let sql = format!(
            "SELECT id, amount, tx_type, category, description, date, recurring_id, group_id
             FROM fin_transactions {where_clause} ORDER BY date DESC, id DESC"
        );

        let mut stmt = conn.prepare(&sql).map_err(map_err)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())), map_transaction)
            .map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    fn create_transaction(&self, input: NewTransaction) -> Result<Transaction, DomainError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO fin_transactions (amount, tx_type, category, description, date, recurring_id, group_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![input.amount, input.tx_type.as_str(), input.category, input.description, input.date, input.recurring_id, input.group_id],
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

    fn list_recurring_rules(&self) -> Result<Vec<RecurringRule>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, amount, tx_type, category, description, period, day_of_month, next_due, active, group_id
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
            "INSERT INTO fin_recurring_rules (amount, tx_type, category, description, period, day_of_month, next_due, group_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![input.amount, input.tx_type.as_str(), input.category, input.description, input.period.as_str(), input.day_of_month, input.next_due, input.group_id],
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
            "SELECT id, amount, tx_type, category, description, period, day_of_month, next_due, active, group_id
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

    fn list_budgets(&self, month: &str) -> Result<Vec<Budget>, DomainError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, category, monthly_limit, month FROM fin_budgets WHERE month = ?1 ORDER BY category"
        ).map_err(map_err)?;
        let rows = stmt.query_map(params![month], map_budget).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

    fn set_budget(&self, input: NewBudget) -> Result<Budget, DomainError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO fin_budgets (category, monthly_limit, month) VALUES (?1, ?2, ?3)
             ON CONFLICT(category, month) DO UPDATE SET monthly_limit = excluded.monthly_limit",
            params![input.category, input.monthly_limit, input.month],
        ).map_err(map_err)?;
        let id = conn.last_insert_rowid();
        conn.query_row("SELECT id, category, monthly_limit, month FROM fin_budgets WHERE rowid = ?1", params![id], map_budget).map_err(map_err)
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
            "SELECT b.id, b.category, b.monthly_limit, b.month,
                    COALESCE((SELECT SUM(t.amount) FROM fin_transactions t
                              WHERE t.category = b.category AND t.tx_type = 'gasto'
                              AND t.date >= ?1 AND t.date <= ?2), 0)
             FROM fin_budgets b WHERE b.month = ?3 ORDER BY b.category"
        ).map_err(map_err)?;
        let rows = stmt.query_map(params![from, to, month], |row| {
            let budget = Budget {
                id: row.get(0)?,
                category: row.get(1)?,
                monthly_limit: row.get(2)?,
                month: row.get(3)?,
            };
            let spent: i64 = row.get(4)?;
            Ok((budget, spent))
        }).map_err(map_err)?;
        let mut out = vec![];
        for row in rows { out.push(row.map_err(map_err)?); }
        Ok(out)
    }

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
        conn.execute(
            "UPDATE fin_debts SET remaining_amount=?1, monthly_payment=?2 WHERE id=?3",
            params![remaining, payment, id],
        ).map_err(map_err)?;
        get_debt(&conn, id)
    }

    fn delete_debt(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("DELETE FROM fin_debts WHERE id=?1", params![id]).map_err(map_err)?;
        if affected == 0 { return Err(DomainError::NotFound(format!("Debt {id} not found"))); }
        Ok(())
    }

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
            "INSERT INTO fin_goals (name, target_amount, current_amount, deadline, horizon)
             VALUES (?1, ?2, ?3, ?4, ?5)",
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
        let deadline = match patch.deadline {
            Some(d) => d,
            None => existing.deadline,
        };
        conn.execute(
            "UPDATE fin_goals SET current_amount=?1, target_amount=?2, deadline=?3 WHERE id=?4",
            params![current, target, deadline, id],
        ).map_err(map_err)?;
        get_goal(&conn, id)
    }

    fn delete_goal(&self, id: i64) -> Result<(), DomainError> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute("DELETE FROM fin_goals WHERE id=?1", params![id]).map_err(map_err)?;
        if affected == 0 { return Err(DomainError::NotFound(format!("Goal {id} not found"))); }
        Ok(())
    }
}

// --- helpers ---

fn get_transaction(conn: &Connection, id: i64) -> Result<Transaction, DomainError> {
    conn.query_row(
        "SELECT id, amount, tx_type, category, description, date, recurring_id, group_id FROM fin_transactions WHERE id=?1",
        params![id], map_transaction,
    ).map_err(map_err)
}

fn get_recurring(conn: &Connection, id: i64) -> Result<RecurringRule, DomainError> {
    conn.query_row(
        "SELECT id, amount, tx_type, category, description, period, day_of_month, next_due, active, group_id FROM fin_recurring_rules WHERE id=?1",
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
