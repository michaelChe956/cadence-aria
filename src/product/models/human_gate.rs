use serde::{Deserialize, Serialize};

/// Durable lifecycle state for one human-gate feedback turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanGateTurnStatus {
    Reserved,
    Running,
    Completed,
    Failed,
}

/// Terminal failure classification for a human-gate feedback turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanGateTurnFailureClass {
    ProviderErr,
    ValidationReject,
    Timeout,
    BudgetExhausted,
}

/// Durable record for one accepted human-gate feedback command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanGateTurn {
    pub turn_id: String,
    pub session_id: String,
    pub command_id: String,
    pub feedback_text: String,
    pub status: HumanGateTurnStatus,
    pub attempt_no: u32,
    pub budget_reserved: u32,
    #[serde(deserialize_with = "super::deserialize_required_option")]
    pub result_artifact_ref: Option<String>,
    #[serde(deserialize_with = "super::deserialize_required_option")]
    pub failure_class: Option<HumanGateTurnFailureClass>,
    pub created_at: String,
    pub updated_at: String,
}

/// The reservation committed alongside the session budget decrement and
/// provider-start ledger entry. It is retained on the session as the command's
/// idempotency anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanGateReservation {
    pub command_id: String,
    pub turn_id: String,
    pub provider_start_idempotency_key: String,
    pub reserved_at: String,
}
