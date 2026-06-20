use super::entity::{GoalHorizon, RecurringPeriod, TransactionType};

#[derive(Debug, Clone)]
pub struct NewCategory {
    pub name: String,
    pub tx_type: Option<TransactionType>,
}

#[derive(Debug, Clone)]
pub struct NewTransaction {
    pub amount: i64,
    pub tx_type: TransactionType,
    pub category_id: i64,
    pub description: String,
    pub date: String,
    pub recurring_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct TransactionFilter {
    pub tx_type: Option<TransactionType>,
    pub category_id: Option<i64>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewRecurringRule {
    pub amount: i64,
    pub tx_type: TransactionType,
    pub category_id: i64,
    pub description: String,
    pub period: RecurringPeriod,
    pub day_of_month: Option<u32>,
    pub next_due: String,
}

#[derive(Debug, Clone)]
pub struct NewBudget {
    pub category_id: i64,
    pub monthly_limit: i64,
    pub month: String,
}

#[derive(Debug, Clone)]
pub struct NewDebt {
    pub creditor: String,
    pub total_amount: i64,
    pub remaining_amount: i64,
    pub interest_rate: Option<f64>,
    pub monthly_payment: i64,
    pub due_day: Option<u32>,
    pub start_date: String,
}

#[derive(Debug, Clone, Default)]
pub struct DebtPatch {
    pub remaining_amount: Option<i64>,
    pub monthly_payment: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewGoal {
    pub name: String,
    pub target_amount: i64,
    pub current_amount: i64,
    pub deadline: Option<String>,
    pub horizon: GoalHorizon,
}

#[derive(Debug, Clone, Default)]
pub struct GoalPatch {
    pub current_amount: Option<i64>,
    pub target_amount: Option<i64>,
    pub deadline: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct MonthlySummary {
    pub month: String,
    pub total_income: i64,
    pub total_expenses: i64,
    pub balance: i64,
    pub savings_rate: f64,
}
