use std::sync::Arc;

use crate::domain::{
    Budget, Debt, DebtPatch, DomainError, FinCategory, Goal, GoalPatch, MonthlySummary, NewBudget,
    NewCategory, NewDebt, NewGoal, NewRecurringRule, NewTransaction, RecurringRule, Transaction,
    TransactionFilter, TransactionType,
    finance::FinanceRepository,
};

pub struct FinanceService {
    repo: Arc<dyn FinanceRepository>,
}

impl FinanceService {
    pub fn new(repo: Arc<dyn FinanceRepository>) -> Self {
        Self { repo }
    }

    // Categories
    pub fn list_categories(&self, tx_type: Option<TransactionType>) -> Result<Vec<FinCategory>, DomainError> {
        self.repo.list_categories(tx_type)
    }
    pub fn create_category(&self, input: NewCategory) -> Result<FinCategory, DomainError> {
        self.repo.create_category(input)
    }
    pub fn delete_category(&self, id: i64) -> Result<(), DomainError> {
        self.repo.delete_category(id)
    }

    // Transactions
    pub fn list_transactions(&self, filter: TransactionFilter) -> Result<Vec<Transaction>, DomainError> {
        self.repo.list_transactions(filter)
    }
    pub fn create_transaction(&self, input: NewTransaction) -> Result<Transaction, DomainError> {
        self.repo.create_transaction(input)
    }
    pub fn delete_transaction(&self, id: i64) -> Result<(), DomainError> {
        self.repo.delete_transaction(id)
    }
    pub fn monthly_summary(&self, month: &str) -> Result<MonthlySummary, DomainError> {
        self.repo.monthly_summary(month)
    }

    // Recurring
    pub fn list_recurring_rules(&self) -> Result<Vec<RecurringRule>, DomainError> {
        self.repo.list_recurring_rules()
    }
    pub fn create_recurring_rule(&self, input: NewRecurringRule) -> Result<RecurringRule, DomainError> {
        self.repo.create_recurring_rule(input)
    }
    pub fn deactivate_recurring_rule(&self, id: i64) -> Result<(), DomainError> {
        self.repo.deactivate_recurring_rule(id)
    }

    /// Generate all pending recurring transactions up to today.
    pub fn generate_pending_recurrings(&self, today: &str) -> Result<u32, DomainError> {
        let mut generated = 0u32;
        loop {
            let pending = self.repo.pending_recurring_rules(today)?;
            if pending.is_empty() { break; }
            for rule in pending {
                self.repo.create_transaction(NewTransaction {
                    amount: rule.amount,
                    tx_type: rule.tx_type,
                    category_id: rule.category_id,
                    description: rule.description.clone(),
                    date: rule.next_due.clone(),
                    recurring_id: Some(rule.id),
                })?;
                let next = advance_date(&rule.next_due, &rule.period);
                self.repo.advance_recurring_next_due(rule.id, &next)?;
                generated += 1;
            }
        }
        Ok(generated)
    }

    // Budgets
    pub fn list_budgets(&self, month: &str) -> Result<Vec<Budget>, DomainError> {
        self.repo.list_budgets(month)
    }
    pub fn set_budget(&self, input: NewBudget) -> Result<Budget, DomainError> {
        self.repo.set_budget(input)
    }
    pub fn delete_budget(&self, id: i64) -> Result<(), DomainError> {
        self.repo.delete_budget(id)
    }
    pub fn budget_status(&self, month: &str) -> Result<Vec<(Budget, i64)>, DomainError> {
        self.repo.budget_status(month)
    }

    // Debts
    pub fn list_debts(&self) -> Result<Vec<Debt>, DomainError> {
        self.repo.list_debts()
    }
    pub fn create_debt(&self, input: NewDebt) -> Result<Debt, DomainError> {
        self.repo.create_debt(input)
    }
    pub fn update_debt(&self, id: i64, patch: DebtPatch) -> Result<Debt, DomainError> {
        self.repo.update_debt(id, patch)
    }
    pub fn delete_debt(&self, id: i64) -> Result<(), DomainError> {
        self.repo.delete_debt(id)
    }

    // Goals
    pub fn list_goals(&self) -> Result<Vec<Goal>, DomainError> {
        self.repo.list_goals()
    }
    pub fn create_goal(&self, input: NewGoal) -> Result<Goal, DomainError> {
        self.repo.create_goal(input)
    }
    pub fn update_goal(&self, id: i64, patch: GoalPatch) -> Result<Goal, DomainError> {
        self.repo.update_goal(id, patch)
    }
    pub fn delete_goal(&self, id: i64) -> Result<(), DomainError> {
        self.repo.delete_goal(id)
    }
}

fn advance_date(date: &str, period: &crate::domain::RecurringPeriod) -> String {
    use chrono::{NaiveDate, Days, Months};
    use crate::domain::RecurringPeriod;
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .unwrap_or(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
    let next = match period {
        RecurringPeriod::Weekly => d + Days::new(7),
        RecurringPeriod::Biweekly => d + Days::new(14),
        RecurringPeriod::Monthly => d + Months::new(1),
    };
    next.format("%Y-%m-%d").to_string()
}
