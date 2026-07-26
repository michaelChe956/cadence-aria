use std::path::PathBuf;

use super::types::{
    RepositoryInitializationOperation, RepositoryInitializationOperationStatus,
    RepositoryInitializationStepKind, RepositoryInitializationStepStatus,
    RepositoryRegistrationError, RepositoryRegistrationSuccess,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

const OPERATION_KIND: &str = "repository_initialization_operation";
const INTERRUPTION_ACTION: &str = "服务在初始化完成前中断；检查可能的部分修改后重新提交";

#[derive(Debug, Clone)]
pub struct RepositoryInitializationOperationStore {
    paths: ProductAppPaths,
}

impl RepositoryInitializationOperationStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn create(
        &self,
        operation: RepositoryInitializationOperation,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        let path = self.operation_path(&operation.project_id, &operation.operation_id)?;
        validate_initial_operation(&operation)?;
        if path.exists() {
            let existing: RepositoryInitializationOperation = read_json(&path)?;
            ensure_identity(&existing, &operation.project_id, &operation.operation_id)?;
            validate_record_shape(&existing)?;
            if existing == operation {
                return Ok(existing);
            }
            return Err(identity_mismatch(&operation.operation_id));
        }
        write_json(&path, &operation)?;
        Ok(operation)
    }

    pub fn get(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        let path = self.operation_path(project_id, operation_id)?;
        if !path.exists() {
            return Err(not_found(operation_id));
        }
        let operation: RepositoryInitializationOperation = read_json(&path)?;
        ensure_identity(&operation, project_id, operation_id)?;
        validate_record_shape(&operation)?;
        Ok(operation)
    }

    pub fn mark_running(
        &self,
        project_id: &str,
        operation_id: &str,
        updated_at: String,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != RepositoryInitializationOperationStatus::Created {
                return Err(identity_mismatch(operation_id));
            }
            operation.status = RepositoryInitializationOperationStatus::Running;
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    pub fn mark_step_running(
        &self,
        project_id: &str,
        operation_id: &str,
        step_id: RepositoryInitializationStepKind,
        updated_at: String,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != RepositoryInitializationOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }

            let step_index = step_index(step_id)?;
            if operation.steps[..step_index]
                .iter()
                .any(|step| step.status != RepositoryInitializationStepStatus::Completed)
            {
                return Err(identity_mismatch(operation_id));
            }
            let step = operation
                .steps
                .get(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            if step.status == RepositoryInitializationStepStatus::Running {
                return Ok(());
            }
            if step.status != RepositoryInitializationStepStatus::Pending {
                return Err(identity_mismatch(operation_id));
            }

            let step = operation
                .steps
                .get_mut(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            step.status = RepositoryInitializationStepStatus::Running;
            step.started_at = Some(updated_at.clone());
            step.completed_at = None;
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    pub fn mark_step_completed(
        &self,
        project_id: &str,
        operation_id: &str,
        step_id: RepositoryInitializationStepKind,
        updated_at: String,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != RepositoryInitializationOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            let step_index = step_index(step_id)?;
            if operation.steps[..step_index]
                .iter()
                .any(|step| step.status != RepositoryInitializationStepStatus::Completed)
            {
                return Err(identity_mismatch(operation_id));
            }
            if step_id == RepositoryInitializationStepKind::GitFinalize
                && !git_finalize_result_checkpoint_ready(operation)
            {
                return Err(identity_mismatch(operation_id));
            }
            let step = operation
                .steps
                .get_mut(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            if step.status != RepositoryInitializationStepStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            step.status = RepositoryInitializationStepStatus::Completed;
            step.completed_at = Some(updated_at.clone());
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    pub fn checkpoint_git_finalize_result(
        &self,
        project_id: &str,
        operation_id: &str,
        result: RepositoryRegistrationSuccess,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != RepositoryInitializationOperationStatus::Running
                || !git_finalize_running_after_repository_persist(operation)
                || operation.result.is_some()
                || operation.git_finalize_checkpoint.is_some()
            {
                return Err(identity_mismatch(operation_id));
            }
            operation.git_finalize_checkpoint = Some(result);
            Ok(())
        })
    }

    pub fn update_git_finalize_checkpoint_warning(
        &self,
        project_id: &str,
        operation_id: &str,
        warning: String,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != RepositoryInitializationOperationStatus::Running
                || !git_finalize_running_after_repository_persist(operation)
                || operation.result.is_some()
            {
                return Err(identity_mismatch(operation_id));
            }
            let Some(checkpoint) = operation.git_finalize_checkpoint.as_mut() else {
                return Err(identity_mismatch(operation_id));
            };
            if checkpoint.git_finalize_warning.is_some() {
                return Err(identity_mismatch(operation_id));
            }
            checkpoint.git_finalize_warning = Some(warning);
            Ok(())
        })
    }

    pub fn finish_completed(
        &self,
        project_id: &str,
        operation_id: &str,
        result: RepositoryRegistrationSuccess,
        completed_at: String,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            let git_finalize_failed = git_finalize_failure_ready(operation, &result);
            if operation.status != RepositoryInitializationOperationStatus::Running
                || (!all_steps_completed(operation) && !git_finalize_failed)
                || operation.error.is_some()
            {
                return Err(identity_mismatch(operation_id));
            }
            if git_finalize_failed {
                let step = operation
                    .steps
                    .last_mut()
                    .ok_or_else(|| identity_mismatch(operation_id))?;
                step.status = RepositoryInitializationStepStatus::Failed;
                step.completed_at = Some(completed_at.clone());
            }
            operation.status = RepositoryInitializationOperationStatus::Completed;
            operation.failed_step =
                git_finalize_failed.then_some(RepositoryInitializationStepKind::GitFinalize);
            operation.result = Some(result);
            operation.git_finalize_checkpoint = None;
            operation.error = None;
            operation.updated_at = completed_at.clone();
            operation.completed_at = Some(completed_at);
            Ok(())
        })
    }

    pub fn finish_failed(
        &self,
        project_id: &str,
        operation_id: &str,
        failed_step: Option<RepositoryInitializationStepKind>,
        error: RepositoryRegistrationError,
        completed_at: String,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != RepositoryInitializationOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            if let Some(step_id) = failed_step {
                let step_index = step_index(step_id)?;
                if operation.steps[..step_index]
                    .iter()
                    .any(|step| step.status != RepositoryInitializationStepStatus::Completed)
                {
                    return Err(identity_mismatch(operation_id));
                }
                let step = operation
                    .steps
                    .get_mut(step_index)
                    .ok_or_else(|| identity_mismatch(operation_id))?;
                if step.status != RepositoryInitializationStepStatus::Running {
                    return Err(identity_mismatch(operation_id));
                }
                step.status = RepositoryInitializationStepStatus::Failed;
                step.completed_at = Some(completed_at.clone());
            } else if !all_steps_completed(operation)
                && !completed_before_git_finalize_pending(operation)
            {
                return Err(identity_mismatch(operation_id));
            }

            operation.status = RepositoryInitializationOperationStatus::Failed;
            operation.failed_step = failed_step;
            operation.result = None;
            operation.git_finalize_checkpoint = None;
            operation.error = Some(error);
            operation.updated_at = completed_at.clone();
            operation.completed_at = Some(completed_at);
            Ok(())
        })
    }

    pub fn recover_interrupted(
        &self,
        project_id: &str,
        operation_id: &str,
        completed_at: String,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if matches!(
                operation.status,
                RepositoryInitializationOperationStatus::Completed
                    | RepositoryInitializationOperationStatus::Failed
            ) {
                return Ok(());
            }

            if git_finalize_result_checkpoint_ready(operation) {
                let mut checkpoint = operation
                    .git_finalize_checkpoint
                    .take()
                    .ok_or_else(|| identity_mismatch(operation_id))?;
                let git_finalize_interrupted = operation
                    .steps
                    .last()
                    .is_some_and(is_running_step);
                let step = operation
                    .steps
                    .last_mut()
                    .ok_or_else(|| identity_mismatch(operation_id))?;
                if git_finalize_interrupted {
                    step.status = RepositoryInitializationStepStatus::Failed;
                    step.completed_at = Some(completed_at.clone());
                    if checkpoint.git_finalize_warning.is_none() {
                        checkpoint.git_finalize_warning = Some(
                            "git_finalize: 服务在收尾期间中断；请检查目标仓库后手动执行 git commit / git push"
                                .to_string(),
                        );
                    }
                }
                operation.status = RepositoryInitializationOperationStatus::Completed;
                operation.failed_step = git_finalize_interrupted
                    .then_some(RepositoryInitializationStepKind::GitFinalize);
                operation.result = Some(checkpoint);
                operation.error = None;
                operation.updated_at = completed_at.clone();
                operation.completed_at = Some(completed_at);
                return Ok(());
            }

            let failed_step = operation
                .steps
                .iter_mut()
                .find(|step| step.status == RepositoryInitializationStepStatus::Running)
                .map(|step| {
                    step.status = RepositoryInitializationStepStatus::Failed;
                    step.completed_at = Some(completed_at.clone());
                    step.step_id
                });
            operation.status = RepositoryInitializationOperationStatus::Failed;
            operation.failed_step = failed_step;
            operation.result = None;
            operation.git_finalize_checkpoint = None;
            operation.error = Some(interrupted_error());
            operation.updated_at = completed_at.clone();
            operation.completed_at = Some(completed_at);
            Ok(())
        })
    }

    fn operation_path(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        Ok(self
            .paths
            .repository_initializations_root(project_id)
            .join(format!("{operation_id}.json")))
    }

    fn update(
        &self,
        project_id: &str,
        operation_id: &str,
        update: impl FnOnce(&mut RepositoryInitializationOperation) -> Result<(), ProductStoreError>,
    ) -> Result<RepositoryInitializationOperation, ProductStoreError> {
        let path = self.operation_path(project_id, operation_id)?;
        if !path.exists() {
            return Err(not_found(operation_id));
        }
        let mut operation: RepositoryInitializationOperation = read_json(&path)?;
        ensure_identity(&operation, project_id, operation_id)?;
        validate_record_shape(&operation)?;
        update(&mut operation)?;
        write_json(&path, &operation)?;
        Ok(operation)
    }
}

fn validate_initial_operation(
    operation: &RepositoryInitializationOperation,
) -> Result<(), ProductStoreError> {
    validate_relative_id(&operation.project_id)?;
    validate_relative_id(&operation.operation_id)?;
    if operation.status != RepositoryInitializationOperationStatus::Created
        || operation.steps.len() != RepositoryInitializationStepKind::ALL.len()
        || operation
            .steps
            .iter()
            .zip(RepositoryInitializationStepKind::ALL)
            .any(|(step, expected)| {
                step.step_id != expected
                    || step.status != RepositoryInitializationStepStatus::Pending
                    || step.started_at.is_some()
                    || step.completed_at.is_some()
            })
        || operation.failed_step.is_some()
        || operation.result.is_some()
        || operation.git_finalize_checkpoint.is_some()
        || operation.error.is_some()
        || operation.completed_at.is_some()
    {
        return Err(identity_mismatch(&operation.operation_id));
    }
    Ok(())
}

fn ensure_identity(
    operation: &RepositoryInitializationOperation,
    project_id: &str,
    operation_id: &str,
) -> Result<(), ProductStoreError> {
    if operation.project_id != project_id || operation.operation_id != operation_id {
        return Err(identity_mismatch(operation_id));
    }
    Ok(())
}

fn validate_record_shape(
    operation: &RepositoryInitializationOperation,
) -> Result<(), ProductStoreError> {
    if !has_supported_step_layout(operation) || !valid_operation_state(operation) {
        return Err(identity_mismatch(&operation.operation_id));
    }
    Ok(())
}

fn has_supported_step_layout(operation: &RepositoryInitializationOperation) -> bool {
    let current_layout = operation.steps.len() == RepositoryInitializationStepKind::ALL.len()
        && operation
            .steps
            .iter()
            .zip(RepositoryInitializationStepKind::ALL)
            .all(|(step, expected)| step.step_id == expected);
    let legacy_layout = operation.steps.len() + 1 == RepositoryInitializationStepKind::ALL.len()
        && operation
            .steps
            .iter()
            .zip(&RepositoryInitializationStepKind::ALL[..5])
            .all(|(step, expected)| step.step_id == *expected);
    current_layout || legacy_layout
}

fn valid_operation_state(operation: &RepositoryInitializationOperation) -> bool {
    match operation.status {
        RepositoryInitializationOperationStatus::Created => {
            operation.steps.iter().all(is_pending_step)
                && operation.failed_step.is_none()
                && operation.result.is_none()
                && operation.git_finalize_checkpoint.is_none()
                && operation.error.is_none()
                && operation.completed_at.is_none()
        }
        RepositoryInitializationOperationStatus::Running => {
            valid_running_steps(operation)
                && operation.failed_step.is_none()
                && operation.result.is_none()
                && (operation.git_finalize_checkpoint.is_none()
                    || git_finalize_result_checkpoint_ready(operation))
                && operation.error.is_none()
                && operation.completed_at.is_none()
        }
        RepositoryInitializationOperationStatus::Completed => {
            (operation.steps.iter().all(is_completed_step) && operation.failed_step.is_none()
                || completed_with_git_finalize_failure(operation))
                && operation.result.is_some()
                && operation.git_finalize_checkpoint.is_none()
                && operation.error.is_none()
                && operation.completed_at.is_some()
        }
        RepositoryInitializationOperationStatus::Failed => {
            operation.result.is_none()
                && operation.git_finalize_checkpoint.is_none()
                && operation.error.is_some()
                && operation.completed_at.is_some()
                && valid_failed_steps(operation)
        }
    }
}

fn valid_running_steps(operation: &RepositoryInitializationOperation) -> bool {
    match operation
        .steps
        .iter()
        .position(|step| step.status == RepositoryInitializationStepStatus::Running)
    {
        Some(running_index) => {
            operation.steps[..running_index]
                .iter()
                .all(is_completed_step)
                && is_running_step(&operation.steps[running_index])
                && operation.steps[running_index + 1..]
                    .iter()
                    .all(is_pending_step)
        }
        None => completed_prefix_pending_suffix(operation),
    }
}

fn valid_failed_steps(operation: &RepositoryInitializationOperation) -> bool {
    match operation.failed_step {
        Some(failed_step) => {
            let Ok(failed_index) = step_index(failed_step) else {
                return false;
            };
            let Some(failed_record) = operation.steps.get(failed_index) else {
                return false;
            };
            operation.steps[..failed_index]
                .iter()
                .all(is_completed_step)
                && is_failed_step(failed_record)
                && operation.steps[failed_index + 1..]
                    .iter()
                    .all(is_pending_step)
        }
        None => {
            operation.steps.iter().all(is_completed_step)
                || completed_before_git_finalize_pending(operation)
                || (completed_prefix_pending_suffix(operation)
                    && operation.error.as_ref() == Some(&interrupted_error()))
        }
    }
}

fn git_finalize_failure_ready(
    operation: &RepositoryInitializationOperation,
    result: &RepositoryRegistrationSuccess,
) -> bool {
    operation.steps[..operation.steps.len().saturating_sub(1)]
        .iter()
        .all(is_completed_step)
        && matches!(
            operation.steps.last(),
            Some(step)
                if step.step_id == RepositoryInitializationStepKind::GitFinalize
                    && is_running_step(step)
        )
        && result.git_finalize_warning.is_some()
}

fn git_finalize_running_after_repository_persist(
    operation: &RepositoryInitializationOperation,
) -> bool {
    operation.steps[..operation.steps.len().saturating_sub(1)]
        .iter()
        .all(is_completed_step)
        && matches!(
            operation.steps.last(),
            Some(step)
                if step.step_id == RepositoryInitializationStepKind::GitFinalize
                    && is_running_step(step)
        )
}

fn git_finalize_result_checkpoint_ready(operation: &RepositoryInitializationOperation) -> bool {
    operation.steps[..operation.steps.len().saturating_sub(1)]
        .iter()
        .all(is_completed_step)
        && matches!(
            operation.steps.last(),
            Some(step)
                if step.step_id == RepositoryInitializationStepKind::GitFinalize
                    && (is_running_step(step) || is_completed_step(step))
        )
        && operation.git_finalize_checkpoint.is_some()
}

fn completed_with_git_finalize_failure(operation: &RepositoryInitializationOperation) -> bool {
    operation.steps[..operation.steps.len().saturating_sub(1)]
        .iter()
        .all(is_completed_step)
        && matches!(
            operation.steps.last(),
            Some(step)
                if step.step_id == RepositoryInitializationStepKind::GitFinalize
                    && is_failed_step(step)
        )
        && operation.failed_step == Some(RepositoryInitializationStepKind::GitFinalize)
        && operation
            .result
            .as_ref()
            .and_then(|result| result.git_finalize_warning.as_ref())
            .is_some()
}

fn completed_before_git_finalize_pending(operation: &RepositoryInitializationOperation) -> bool {
    operation.steps[..operation.steps.len().saturating_sub(1)]
        .iter()
        .all(is_completed_step)
        && matches!(
            operation.steps.last(),
            Some(step)
                if step.step_id == RepositoryInitializationStepKind::GitFinalize
                    && is_pending_step(step)
        )
}

fn completed_prefix_pending_suffix(operation: &RepositoryInitializationOperation) -> bool {
    let mut pending_seen = false;

    for step in &operation.steps {
        if is_completed_step(step) && !pending_seen {
            continue;
        }
        if is_pending_step(step) {
            pending_seen = true;
            continue;
        }
        return false;
    }

    true
}

fn is_pending_step(step: &super::types::RepositoryInitializationStepRecord) -> bool {
    step.status == RepositoryInitializationStepStatus::Pending
        && step.started_at.is_none()
        && step.completed_at.is_none()
}

fn is_running_step(step: &super::types::RepositoryInitializationStepRecord) -> bool {
    step.status == RepositoryInitializationStepStatus::Running
        && step.started_at.is_some()
        && step.completed_at.is_none()
}

fn is_completed_step(step: &super::types::RepositoryInitializationStepRecord) -> bool {
    step.status == RepositoryInitializationStepStatus::Completed
        && step.started_at.is_some()
        && step.completed_at.is_some()
}

fn is_failed_step(step: &super::types::RepositoryInitializationStepRecord) -> bool {
    step.status == RepositoryInitializationStepStatus::Failed
        && step.started_at.is_some()
        && step.completed_at.is_some()
}

fn step_index(step_id: RepositoryInitializationStepKind) -> Result<usize, ProductStoreError> {
    RepositoryInitializationStepKind::ALL
        .iter()
        .position(|candidate| *candidate == step_id)
        .ok_or_else(|| identity_mismatch("repository_initialization_step"))
}

fn all_steps_completed(operation: &RepositoryInitializationOperation) -> bool {
    operation
        .steps
        .iter()
        .all(|step| step.status == RepositoryInitializationStepStatus::Completed)
}

fn interrupted_error() -> RepositoryRegistrationError {
    RepositoryRegistrationError {
        stage: "repository_initialization".to_string(),
        provider: None,
        command_index: None,
        command: None,
        reason_code: "repository_initialization_interrupted".to_string(),
        stderr_summary: None,
        changed_paths: None,
        retryable: true,
        action: INTERRUPTION_ACTION.to_string(),
    }
}

fn identity_mismatch(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: OPERATION_KIND,
        id: id.to_string(),
    }
}

fn not_found(operation_id: &str) -> ProductStoreError {
    ProductStoreError::NotFound {
        kind: OPERATION_KIND,
        id: operation_id.to_string(),
    }
}

#[cfg(test)]
mod tests;
