use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::product::models::RepositoryRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRegistrationInput {
    pub project_id: String,
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationCommandSummary {
    pub command_index: usize,
    pub command: String,
    pub status: String,
    pub output_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CadenceSkillsPreparationSummary {
    pub source_mode: String,
    pub source_root: PathBuf,
    pub skills_root: PathBuf,
    pub git_updated: bool,
    pub link_sync_status: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationSummary {
    pub provider: String,
    pub source: PathBuf,
    pub source_mode: String,
    pub skills_root: PathBuf,
    pub git_updated: bool,
    pub link_sync_status: String,
    pub commands: Vec<RepositoryInitializationCommandSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRegistrationSuccess {
    pub repository: RepositoryRecord,
    pub cadence_skills: CadenceSkillsPreparationSummary,
    pub initialization: RepositoryInitializationSummary,
    pub warnings: Vec<String>,
    pub changed_paths: Vec<String>,
    pub completed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{reason_code} at {stage}: {stderr_summary:?}; action: {action}")]
pub struct RepositoryRegistrationError {
    pub stage: String,
    pub provider: Option<String>,
    pub command_index: Option<usize>,
    pub command: Option<String>,
    pub reason_code: String,
    pub stderr_summary: Option<String>,
    pub changed_paths: Option<Vec<String>>,
    pub retryable: bool,
    pub action: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryInitializationOperationStatus {
    Created,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryInitializationStepKind {
    CadenceSkills,
    PreCheck,
    RuleConfig,
    McpConfiguration,
    ProjectRulesExamples,
}

impl RepositoryInitializationStepKind {
    pub const ALL: [Self; 5] = [
        Self::CadenceSkills,
        Self::RuleConfig,
        Self::PreCheck,
        Self::McpConfiguration,
        Self::ProjectRulesExamples,
    ];

    pub fn command(self) -> Option<&'static str> {
        match self {
            Self::CadenceSkills => None,
            Self::PreCheck => Some("/pre-check --no-interrupt"),
            Self::RuleConfig => Some("/rule-config --no-interrupt"),
            Self::McpConfiguration => Some("/mcp-configuration --no-interrupt"),
            Self::ProjectRulesExamples => Some("/project-rules-examples --no-interrupt"),
        }
    }

    pub fn from_command_index(command_index: usize) -> Option<Self> {
        [
            Self::RuleConfig,
            Self::PreCheck,
            Self::McpConfiguration,
            Self::ProjectRulesExamples,
        ]
        .get(command_index.checked_sub(1)?)
        .copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryInitializationStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationStepRecord {
    pub step_id: RepositoryInitializationStepKind,
    pub status: RepositoryInitializationStepStatus,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationOperationInput {
    pub name: String,
    pub git_root: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationOperation {
    pub operation_id: String,
    pub project_id: String,
    pub input: RepositoryInitializationOperationInput,
    pub status: RepositoryInitializationOperationStatus,
    pub steps: Vec<RepositoryInitializationStepRecord>,
    pub failed_step: Option<RepositoryInitializationStepKind>,
    pub result: Option<RepositoryRegistrationSuccess>,
    pub error: Option<RepositoryRegistrationError>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

impl RepositoryInitializationOperation {
    pub fn new(
        operation_id: String,
        project_id: String,
        input: RepositoryInitializationOperationInput,
        created_at: String,
    ) -> Self {
        Self {
            operation_id,
            project_id,
            input,
            status: RepositoryInitializationOperationStatus::Created,
            steps: RepositoryInitializationStepKind::ALL
                .iter()
                .copied()
                .map(|step_id| RepositoryInitializationStepRecord {
                    step_id,
                    status: RepositoryInitializationStepStatus::Pending,
                    started_at: None,
                    completed_at: None,
                })
                .collect(),
            failed_step: None,
            result: None,
            error: None,
            updated_at: created_at.clone(),
            created_at,
            completed_at: None,
        }
    }
}

pub trait RepositoryInitializationProgress: Send + Sync {
    fn step_started(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), Box<RepositoryRegistrationError>>;

    fn step_completed(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), Box<RepositoryRegistrationError>>;
}

impl RepositoryRegistrationError {
    pub(crate) fn new(
        stage: impl Into<String>,
        reason_code: impl Into<String>,
        stderr_summary: Option<String>,
        retryable: bool,
        action: impl Into<String>,
    ) -> Self {
        Self {
            stage: stage.into(),
            provider: None,
            command_index: None,
            command: None,
            reason_code: reason_code.into(),
            stderr_summary,
            changed_paths: None,
            retryable,
            action: action.into(),
        }
    }

    pub(crate) fn for_command(
        stage: impl Into<String>,
        reason_code: impl Into<String>,
        command_index: usize,
        command: &str,
        stderr_summary: Option<String>,
        retryable: bool,
        action: impl Into<String>,
    ) -> Self {
        let mut error = Self::new(stage, reason_code, stderr_summary, retryable, action);
        error.provider = Some("claude_code".to_string());
        error.command_index = Some(command_index);
        error.command = Some(command.to_string());
        error
    }
}
