use serde::{Deserialize, Serialize};

use crate::web::error::ApiError;
use crate::web::types::RepositoryDto;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryRegistrationInitializationDto {
    pub source: String,
    pub commands: Vec<serde_json::Value>,
    pub warnings: Vec<String>,
    pub changed_paths: Vec<String>,
    pub git_finalize_warning: Option<String>,
    pub completed_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryInitializationStepDto {
    pub step_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryInitializationResultDto {
    pub repository: RepositoryDto,
    pub initialization: RepositoryRegistrationInitializationDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryInitializationOperationDto {
    pub operation_id: String,
    pub status: String,
    pub steps: Vec<RepositoryInitializationStepDto>,
    pub current_step: Option<String>,
    pub failed_step: Option<String>,
    pub result: Option<RepositoryInitializationResultDto>,
    pub error: Option<ApiError>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}
