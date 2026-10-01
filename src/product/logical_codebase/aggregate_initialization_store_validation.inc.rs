/// Task 1.5（REQ-BOOT-03）：五步 aggregate operation 的 root recipe 命令
/// 索引——provider-turn 步骤到四条无中断命令的固定扁平序（全局
/// `command_index` 1..=4）。命令文本唯一来源仍是
/// [`crate::product::repository_store::RepositoryInitializationStepKind::command`]
///（Task 1.1 隔离锁显式豁免的消费点），映射与 Task 1.4 驱动器的
/// `recipe_commands` 一致：`PreCheck`=命令 1，`RuleAndMcpConfig`=命令 2+3，
/// `OpenspecAndExamples`=命令 4；确定性步骤不映射任何命令。receipt/
/// auditor（`root_recipe_receipt`）按此索引校验命令身份与「四命令全部
/// 审计通过」。
pub fn root_recipe_command_index() -> Vec<(AggregateInitializationStepKind, usize, &'static str)> {
    use crate::product::repository_store::RepositoryInitializationStepKind as RepoStep;

    let provider_turn_steps: [(AggregateInitializationStepKind, &[RepoStep]); 3] = [
        (
            AggregateInitializationStepKind::PreCheck,
            &[RepoStep::PreCheck],
        ),
        (
            AggregateInitializationStepKind::RuleAndMcpConfig,
            &[RepoStep::RuleConfig, RepoStep::McpConfiguration],
        ),
        (
            AggregateInitializationStepKind::OpenspecAndExamples,
            &[RepoStep::ProjectRulesExamples],
        ),
    ];

    let mut index = Vec::new();
    let mut command_index = 1usize;
    for (step, repo_steps) in provider_turn_steps {
        for repo_step in repo_steps {
            if let Some(command) = repo_step.command() {
                index.push((step, command_index, command));
                command_index += 1;
            }
        }
    }
    index
}

fn validate_initial_operation(
    operation: &AggregateInitializationOperation,
) -> Result<(), ProductStoreError> {
    validate_relative_id(&operation.project_id)?;
    validate_relative_id(&operation.operation_id)?;
    if operation.operation_kind != AGGREGATE_INITIALIZATION_OPERATION_KIND
        || operation.status != AggregateInitializationOperationStatus::Created
        || operation.steps.len() != AggregateInitializationStepKind::V1.len()
        || operation
            .steps
            .iter()
            .zip(AggregateInitializationStepKind::V1)
            .any(|(step, expected)| {
                step.step_id != expected
                    || step.status != AggregateInitializationStepStatus::Pending
                    || step.started_at.is_some()
                    || step.completed_at.is_some()
                    || step.input_digest.is_some()
                    || step.output_artifact_ref.is_some()
            })
        || operation.current_step.is_some()
        || operation.failed_step.is_some()
        || !operation.member_projections.is_empty()
        || operation.cancellation.is_some()
        || operation.error.is_some()
        || operation.completed_at.is_some()
    {
        return Err(identity_mismatch(&operation.operation_id));
    }
    Ok(())
}

fn ensure_identity(
    operation: &AggregateInitializationOperation,
    project_id: &str,
    operation_id: &str,
) -> Result<(), ProductStoreError> {
    if operation.project_id != project_id || operation.operation_id != operation_id {
        return Err(identity_mismatch(operation_id));
    }
    Ok(())
}

fn validate_record_shape(
    operation: &AggregateInitializationOperation,
) -> Result<(), ProductStoreError> {
    if !has_supported_step_layout(operation) || !valid_operation_state(operation) {
        return Err(identity_mismatch(&operation.operation_id));
    }
    Ok(())
}

fn has_supported_step_layout(operation: &AggregateInitializationOperation) -> bool {
    operation.steps.len() == AggregateInitializationStepKind::V1.len()
        && operation
            .steps
            .iter()
            .zip(AggregateInitializationStepKind::V1)
            .all(|(step, expected)| step.step_id == expected)
}

fn valid_operation_state(operation: &AggregateInitializationOperation) -> bool {
    match operation.status {
        AggregateInitializationOperationStatus::Created => {
            operation.steps.iter().all(is_pending_step)
                && operation.current_step.is_none()
                && operation.failed_step.is_none()
                && operation.cancellation.is_none()
                && operation.error.is_none()
                && operation.completed_at.is_none()
        }
        AggregateInitializationOperationStatus::Running => {
            valid_running_steps(operation)
                && operation.failed_step.is_none()
                && operation.cancellation.is_none()
                && operation.error.is_none()
                && operation.completed_at.is_none()
        }
        AggregateInitializationOperationStatus::Completed => {
            operation.steps.iter().all(is_completed_step)
                && operation.current_step.is_none()
                && operation.failed_step.is_none()
                && operation.cancellation.is_none()
                && operation.error.is_none()
                && operation.completed_at.is_some()
        }
        AggregateInitializationOperationStatus::Failed => {
            valid_terminal_failed_state(operation) && operation.cancellation.is_none()
        }
        AggregateInitializationOperationStatus::Cancelled => {
            operation.cancellation.is_some() && operation.error.is_none()
        }
    }
}

fn valid_running_steps(operation: &AggregateInitializationOperation) -> bool {
    match operation
        .steps
        .iter()
        .position(|step| step.status == AggregateInitializationStepStatus::Running)
    {
        Some(running_index) => {
            operation.steps[..running_index]
                .iter()
                .all(is_completed_step)
                && is_running_step(&operation.steps[running_index])
                && operation.steps[running_index + 1..]
                    .iter()
                    .all(is_pending_step)
                && operation.current_step == Some(operation.steps[running_index].step_id)
        }
        None => completed_prefix_pending_suffix(operation) && operation.current_step.is_none(),
    }
}

fn valid_terminal_failed_state(operation: &AggregateInitializationOperation) -> bool {
    let has_terminal = operation.error.is_some() && operation.completed_at.is_some();
    match operation.failed_step {
        Some(failed_step) => {
            let failed_index = failed_step.index();
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
                && operation.current_step.is_none()
                && has_terminal
        }
        None => {
            operation.current_step.is_none()
                && has_terminal
                && (operation.steps.iter().all(is_completed_step)
                    || completed_prefix_pending_suffix(operation))
        }
    }
}

fn completed_prefix_pending_suffix(operation: &AggregateInitializationOperation) -> bool {
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

fn is_pending_step(step: &AggregateInitializationStepRecord) -> bool {
    step.status == AggregateInitializationStepStatus::Pending
        && step.started_at.is_none()
        && step.completed_at.is_none()
        && step.input_digest.is_none()
        && step.output_artifact_ref.is_none()
}

fn is_running_step(step: &AggregateInitializationStepRecord) -> bool {
    step.status == AggregateInitializationStepStatus::Running
        && step.started_at.is_some()
        && step.completed_at.is_none()
        && step.input_digest.is_some()
}

fn is_completed_step(step: &AggregateInitializationStepRecord) -> bool {
    step.status == AggregateInitializationStepStatus::Completed
        && step.started_at.is_some()
        && step.completed_at.is_some()
        && step.output_artifact_ref.is_some()
}

fn is_failed_step(step: &AggregateInitializationStepRecord) -> bool {
    step.status == AggregateInitializationStepStatus::Failed
        && step.started_at.is_some()
        && step.completed_at.is_some()
}

fn identity_mismatch(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: AGGREGATE_INITIALIZATION_OPERATION_KIND,
        id: id.to_string(),
    }
}

fn conflict(operation_id: &str) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: AGGREGATE_INITIALIZATION_OPERATION_KIND,
        id: operation_id.to_string(),
    }
}

fn not_found(operation_id: &str) -> ProductStoreError {
    ProductStoreError::NotFound {
        kind: AGGREGATE_INITIALIZATION_OPERATION_KIND,
        id: operation_id.to_string(),
    }
}
