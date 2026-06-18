use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransactionType {
    Ingreso,
    Gasto,
}

impl TransactionType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ingreso => "ingreso",
            Self::Gasto => "gasto",
        }
    }
}

impl std::str::FromStr for TransactionType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ingreso" => Ok(Self::Ingreso),
            "gasto" => Ok(Self::Gasto),
            _ => Err(format!("unknown transaction type: {s:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecurringPeriod {
    Weekly,
    Biweekly,
    Monthly,
}

impl RecurringPeriod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Weekly => "weekly",
            Self::Biweekly => "biweekly",
            Self::Monthly => "monthly",
        }
    }
}

impl std::str::FromStr for RecurringPeriod {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "weekly" => Ok(Self::Weekly),
            "biweekly" => Ok(Self::Biweekly),
            "monthly" => Ok(Self::Monthly),
            _ => Err(format!("unknown period: {s:?}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalHorizon {
    Corto,
    Mediano,
    Largo,
}

impl GoalHorizon {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Corto => "corto",
            Self::Mediano => "mediano",
            Self::Largo => "largo",
        }
    }
}

impl std::str::FromStr for GoalHorizon {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "corto" => Ok(Self::Corto),
            "mediano" => Ok(Self::Mediano),
            "largo" => Ok(Self::Largo),
            _ => Err(format!("unknown horizon: {s:?}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub id: i64,
    pub amount: i64,
    pub tx_type: TransactionType,
    pub category: String,
    pub description: String,
    pub date: String,
    pub recurring_id: Option<i64>,
    pub group_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurringRule {
    pub id: i64,
    pub amount: i64,
    pub tx_type: TransactionType,
    pub category: String,
    pub description: String,
    pub period: RecurringPeriod,
    pub day_of_month: Option<u32>,
    pub next_due: String,
    pub active: bool,
    pub group_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    pub id: i64,
    pub category: String,
    pub monthly_limit: i64,
    pub month: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Debt {
    pub id: i64,
    pub creditor: String,
    pub total_amount: i64,
    pub remaining_amount: i64,
    /// Annual interest rate as percentage (e.g. 18.5 = 18.5%)
    pub interest_rate: Option<f64>,
    pub monthly_payment: i64,
    pub due_day: Option<u32>,
    pub start_date: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goal {
    pub id: i64,
    pub name: String,
    pub target_amount: i64,
    pub current_amount: i64,
    pub deadline: Option<String>,
    pub horizon: GoalHorizon,
}
