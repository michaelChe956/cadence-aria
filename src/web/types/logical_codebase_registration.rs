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
