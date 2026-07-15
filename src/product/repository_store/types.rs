use std::path::PathBuf;

use crate::product::models::RepositoryRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRegistrationInput {
    pub project_id: String,
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryInitializationCommandSummary {
    pub command_index: usize,
    pub command: String,
    pub status: String,
    pub output_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CadenceSkillsPreparationSummary {
    pub source_mode: String,
    pub source_root: PathBuf,
    pub skills_root: PathBuf,
    pub git_updated: bool,
    pub link_sync_status: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryInitializationSummary {
    pub provider: String,
    pub source: PathBuf,
    pub source_mode: String,
    pub skills_root: PathBuf,
    pub git_updated: bool,
    pub link_sync_status: String,
    pub commands: Vec<RepositoryInitializationCommandSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRegistrationSuccess {
    pub repository: RepositoryRecord,
    pub cadence_skills: CadenceSkillsPreparationSummary,
    pub initialization: RepositoryInitializationSummary,
    pub warnings: Vec<String>,
    pub changed_paths: Vec<String>,
    pub completed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
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
