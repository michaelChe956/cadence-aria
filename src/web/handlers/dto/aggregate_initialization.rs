use super::*;
use crate::product::logical_codebase::{
    AggregateCancellationRecord, AggregateInitializationOperation,
    AggregateInitializationOperationStatus, AggregateInitializationStepKind,
    AggregateInitializationStepStatus, AggregateProviderCandidate, AggregateProviderSelection,
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
    let provider_selection = operation
        .provider_selection
        .map(aggregate_provider_selection_dto);
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
        provider_selection,
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
        // owner 2026-10-29：聚合根已做过 Cadence-skills 初始化时，三个
        // provider-turn 步骤被判定跳过（判定依据在 operation 记录里）。
        AggregateInitializationStepStatus::Skipped => "skipped",
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

fn aggregate_provider_selection_dto(
    selection: AggregateProviderSelection,
) -> crate::web::types::AggregateProviderSelectionDto {
    crate::web::types::AggregateProviderSelectionDto {
        selected: selection.selected,
        candidates: selection
            .candidates
            .into_iter()
            .map(aggregate_provider_candidate_dto)
            .collect(),
        rule_version: selection.rule_version,
        decided_at: selection.decided_at,
    }
}

fn aggregate_provider_candidate_dto(
    candidate: AggregateProviderCandidate,
) -> crate::web::types::AggregateProviderCandidateDto {
    crate::web::types::AggregateProviderCandidateDto {
        provider: candidate.provider,
        display_name: candidate.display_name,
        cli_available: candidate.cli_available,
        cli_reason_code: candidate.cli_reason_code,
        capability_state: candidate.capability_state,
        skipped_reason: candidate.skipped_reason,
        action_hint: candidate.action_hint,
        warning: candidate.warning,
    }
}

impl From<crate::product::logical_codebase::aggregate_initialization::AggregateInitializationErrorRecord>
    for ApiError
{
    /// 聚合初始化错误 → HTTP 错误：主文案取自**同一**文案来源的 `title_zh`
    /// （不再是原始英文 `action`），三段成品放 `details.copy`，脱敏后的
    /// 原始诊断放 `details.detail`；原始 action / stderr / cause / provider
    /// 仍按既有契约留在 details 里供折叠区展示。
    fn from(
        record: crate::product::logical_codebase::aggregate_initialization::AggregateInitializationErrorRecord,
    ) -> Self {
        use crate::cross_cutting::diagnostic_redaction::{
            sanitize_diagnostic, RedactionContext,
        };
        use crate::product::logical_codebase::failure_copy::{
            describe, record_diagnostic, FailureInput,
        };
        let copy = describe(FailureInput::from(&record));
        let diagnostic =
            sanitize_diagnostic(&record_diagnostic(&record), &RedactionContext::default());
        let action = sanitize_diagnostic(&record.action, &RedactionContext::default());
        let stderr_summary = record
            .stderr_summary
            .as_deref()
            .map(|summary| sanitize_diagnostic(summary, &RedactionContext::default()));
        ApiError::runtime(
            copy.machine_line.clone(),
            copy.title_zh.clone(),
            json!({
                "stage": record.stage,
                "retryable": record.retryable,
                "stderr_summary": stderr_summary,
                "cause": record.cause,
                "provider": record.provider,
                "action": action,
                "detail": diagnostic,
                "copy": copy,
            }),
        )
    }
}
