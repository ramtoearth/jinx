use crate::DomainError;
use super::entity::*;
use super::value_objects::*;

pub trait FinanceRepository: Send + Sync {
    // Categories
    fn list_categories(&self, tx_type: Option<TransactionType>) -> Result<Vec<FinCategory>, DomainError>;
    fn create_category(&self, input: NewCategory) -> Result<FinCategory, DomainError>;
    fn delete_category(&self, id: i64) -> Result<(), DomainError>;

    // Transactions
    fn list_transactions(&self, filter: TransactionFilter) -> Result<Vec<Transaction>, DomainError>;
    fn create_transaction(&self, input: NewTransaction) -> Result<Transaction, DomainError>;
    fn delete_transaction(&self, id: i64) -> Result<(), DomainError>;
    fn monthly_summary(&self, month: &str) -> Result<MonthlySummary, DomainError>;

    // Recurring rules
    fn list_recurring_rules(&self) -> Result<Vec<RecurringRule>, DomainError>;
    fn create_recurring_rule(&self, input: NewRecurringRule) -> Result<RecurringRule, DomainError>;
    fn deactivate_recurring_rule(&self, id: i64) -> Result<(), DomainError>;
    fn pending_recurring_rules(&self, today: &str) -> Result<Vec<RecurringRule>, DomainError>;
    fn advance_recurring_next_due(&self, id: i64, new_next_due: &str) -> Result<(), DomainError>;

    // Budgets
    fn list_budgets(&self, month: &str) -> Result<Vec<Budget>, DomainError>;
    fn set_budget(&self, input: NewBudget) -> Result<Budget, DomainError>;
    fn delete_budget(&self, id: i64) -> Result<(), DomainError>;
    fn budget_status(&self, month: &str) -> Result<Vec<(Budget, i64)>, DomainError>;

    // Debts
    fn list_debts(&self) -> Result<Vec<Debt>, DomainError>;
    fn create_debt(&self, input: NewDebt) -> Result<Debt, DomainError>;
    fn update_debt(&self, id: i64, patch: DebtPatch) -> Result<Debt, DomainError>;
    fn delete_debt(&self, id: i64) -> Result<(), DomainError>;

    // Goals
    fn list_goals(&self) -> Result<Vec<Goal>, DomainError>;
    fn create_goal(&self, input: NewGoal) -> Result<Goal, DomainError>;
    fn update_goal(&self, id: i64, patch: GoalPatch) -> Result<Goal, DomainError>;
    fn delete_goal(&self, id: i64) -> Result<(), DomainError>;
}
