use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RegistrationPreflightRequest {
    pub aggregate_root: String,
    pub candidate_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RegistrationPreflightItemDto {
    pub path: String,
    pub class: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RegistrationPreflightResponse {
    pub preflight_id: String,
    pub created_at: String,
    pub items: Vec<RegistrationPreflightItemDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RegistrationSubmitRequest {
    pub aggregate_root: String,
    pub preflight_id: String,
    pub confirmed_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RegistrationBatchItemDto {
    pub path: String,
    pub status: String,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RegistrationBatchDto {
    pub batch_id: String,
    pub status: String,
    pub items: Vec<RegistrationBatchItemDto>,
}
