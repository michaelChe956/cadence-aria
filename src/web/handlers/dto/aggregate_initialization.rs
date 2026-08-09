use super::*;
use crate::product::logical_codebase::{
    AggregateCancellationRecord, AggregateInitializationOperation,
    AggregateInitializationOperationStatus, AggregateInitializationStepKind,
    AggregateInitializationStepStatus,
};

pub(crate) fn aggregate_initialization_dto(
    operation: AggregateInitializationOperation,
) -> crate::web::types::AggregateInitializationOperationDto {
    let status = operation.status;
    let current_step = operation
        .current_step
        .map(|step| aggregate_step_id(step).to_string());
    let failed_step = operation
        .failed_step
        .map(|step| aggregate_step_id(step).to_string());
    let cancellation = operation.cancellation.map(aggregate_cancellation_dto);
    crate::web::types::AggregateInitializationOperationDto {
        operation_id: operation.operation_id,
        project_id: operation.project_id,
        status: aggregate_operation_status(status).to_string(),
        // The profile is resolved during preflight and not persisted on the
        // operation record in this task; surfaced as None until preflight runs.
        profile: None,
        steps: operation
            .steps
            .into_iter()
            .map(|step| crate::web::types::AggregateInitializationStepDto {
                step_id: aggregate_step_id(step.step_id).to_string(),
                status: aggregate_step_status(step.status).to_string(),
            })
            .collect(),
        current_step,
        failed_step,
        member_projections: operation
            .member_projections
            .into_iter()
            .map(|member| crate::web::types::AggregateMemberProjectionDto {
                logical_repository_id: member.logical_repository_id,
                checkout_id: member.checkout_id,
                revision: member.revision,
                dirty: member.dirty,
                profile_digest: member.profile_digest,
            })
            .collect(),
        cancellation,
        error: operation.error.map(ApiError::from),
        created_at: operation.created_at,
        updated_at: operation.updated_at,
        completed_at: operation.completed_at,
    }
}

fn aggregate_operation_status(status: AggregateInitializationOperationStatus) -> &'static str {
    match status {
        AggregateInitializationOperationStatus::Created => "created",
        AggregateInitializationOperationStatus::Running => "running",
        AggregateInitializationOperationStatus::Completed => "completed",
        AggregateInitializationOperationStatus::Failed => "failed",
        AggregateInitializationOperationStatus::Cancelled => "cancelled",
    }
}

fn aggregate_step_id(step: AggregateInitializationStepKind) -> &'static str {
    match step {
        AggregateInitializationStepKind::MachineSkills => "machine_skills",
        AggregateInitializationStepKind::AggregatePreflight => "aggregate_preflight",
        AggregateInitializationStepKind::PreCheck => "pre_check",
        AggregateInitializationStepKind::RuleAndMcpConfig => "rule_and_mcp_config",
        AggregateInitializationStepKind::OpenspecAndExamples => "openspec_and_examples",
    }
}

fn aggregate_step_status(status: AggregateInitializationStepStatus) -> &'static str {
    match status {
        AggregateInitializationStepStatus::Pending => "pending",
        AggregateInitializationStepStatus::Running => "running",
        AggregateInitializationStepStatus::Completed => "completed",
        AggregateInitializationStepStatus::Failed => "failed",
    }
}

fn aggregate_cancellation_dto(
    record: AggregateCancellationRecord,
) -> crate::web::types::AggregateCancellationDto {
    crate::web::types::AggregateCancellationDto {
        reason_code: record.reason_code,
        cancelled_at: record.cancelled_at,
        detail: record.detail,
    }
}

impl From<crate::product::logical_codebase::aggregate_initialization::AggregateInitializationErrorRecord>
    for ApiError
{
    fn from(
        record: crate::product::logical_codebase::aggregate_initialization::AggregateInitializationErrorRecord,
    ) -> Self {
        ApiError::runtime(
            record.reason_code,
            record.action,
            json!({
                "stage": record.stage,
                "retryable": record.retryable,
                "stderr_summary": record.stderr_summary,
            }),
        )
    }
}
