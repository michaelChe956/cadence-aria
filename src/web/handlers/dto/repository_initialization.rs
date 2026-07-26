use super::*;
use crate::product::repository_store::{
    RepositoryInitializationOperation, RepositoryInitializationOperationStatus,
    RepositoryInitializationStepKind, RepositoryInitializationStepStatus,
    RepositoryRegistrationSuccess,
};
use crate::web::error::{
    sanitize_repository_api_changed_paths, sanitize_repository_api_path,
    sanitize_repository_api_warnings,
};

fn repository_initialization_repository_dto(record: RepositoryRecord) -> RepositoryDto {
    RepositoryDto {
        repository_id: record.id,
        project_id: record.project_id,
        name: record.name,
        path: sanitize_repository_api_path(record.path.to_string_lossy()),
        repo_hash: record.repo_hash,
        runtime_root: sanitize_repository_api_path(record.runtime_root.to_string_lossy()),
        default_policy_preset: record.default_policy_preset,
        default_provider_mode: record.default_provider_mode,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

pub(super) fn repository_initialization_result_dto_impl(
    success: &RepositoryRegistrationSuccess,
) -> RepositoryInitializationResultDto {
    RepositoryInitializationResultDto {
        repository: repository_initialization_repository_dto(success.repository.clone()),
        initialization: RepositoryRegistrationInitializationDto {
            source: success.initialization.source_mode.clone(),
            commands: success
                .initialization
                .commands
                .iter()
                .map(|item| {
                    json!({"index": item.command_index, "command": item.command, "status": item.status})
                })
                .collect(),
            warnings: sanitize_repository_api_warnings(success.warnings.clone()),
            changed_paths: sanitize_repository_api_changed_paths(success.changed_paths.clone()),
            git_finalize_warning: success
                .git_finalize_warning
                .as_ref()
                .and_then(|warning| {
                    sanitize_repository_api_warnings(vec![warning.clone()])
                        .into_iter()
                        .next()
                }),
            completed_at: success.completed_at.clone(),
        },
    }
}

pub(crate) fn repository_initialization_operation_dto(
    operation: RepositoryInitializationOperation,
) -> RepositoryInitializationOperationDto {
    let is_completed = operation.status == RepositoryInitializationOperationStatus::Completed;
    let current_step = operation
        .steps
        .iter()
        .find(|step| step.status == RepositoryInitializationStepStatus::Running)
        .map(|step| repository_initialization_step_id(step.step_id).to_string());
    RepositoryInitializationOperationDto {
        operation_id: operation.operation_id,
        status: repository_initialization_operation_status(operation.status).to_string(),
        steps: operation
            .steps
            .into_iter()
            .map(|step| RepositoryInitializationStepDto {
                step_id: repository_initialization_step_id(step.step_id).to_string(),
                status: repository_initialization_step_status(step.status).to_string(),
            })
            .collect(),
        current_step,
        failed_step: operation
            .failed_step
            .map(|step| repository_initialization_step_id(step).to_string()),
        result: is_completed
            .then(|| {
                operation
                    .result
                    .as_ref()
                    .map(super::repository_initialization_result_dto)
            })
            .flatten(),
        error: operation.error.map(ApiError::from),
        created_at: operation.created_at,
        updated_at: operation.updated_at,
        completed_at: operation.completed_at,
    }
}

fn repository_initialization_operation_status(
    status: RepositoryInitializationOperationStatus,
) -> &'static str {
    match status {
        RepositoryInitializationOperationStatus::Created => "created",
        RepositoryInitializationOperationStatus::Running => "running",
        RepositoryInitializationOperationStatus::Completed => "completed",
        RepositoryInitializationOperationStatus::Failed => "failed",
    }
}

fn repository_initialization_step_id(step: RepositoryInitializationStepKind) -> &'static str {
    match step {
        RepositoryInitializationStepKind::CadenceSkills => "cadence_skills",
        RepositoryInitializationStepKind::PreCheck => "pre_check",
        RepositoryInitializationStepKind::RuleConfig => "rule_config",
        RepositoryInitializationStepKind::McpConfiguration => "mcp_configuration",
        RepositoryInitializationStepKind::ProjectRulesExamples => "project_rules_examples",
        RepositoryInitializationStepKind::GitFinalize => "git_finalize",
    }
}

fn repository_initialization_step_status(
    status: RepositoryInitializationStepStatus,
) -> &'static str {
    match status {
        RepositoryInitializationStepStatus::Pending => "pending",
        RepositoryInitializationStepStatus::Running => "running",
        RepositoryInitializationStepStatus::Completed => "completed",
        RepositoryInitializationStepStatus::Failed => "failed",
    }
}
