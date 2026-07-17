use super::*;
use crate::product::models::{
    PlanRevisionReason, WorkItemDraftRevision, WorkItemDraftVerificationPlan,
};

pub(crate) fn work_item_draft_revision_from_record(
    record: &WorkItemDraftRecord,
) -> Result<WorkItemDraftRevision, WorkspaceEngineError> {
    if record.candidate.logical_work_item_id
        != record
            .candidate
            .canonical_contract_candidate
            .identity
            .logical_work_item_id
    {
        return Err(WorkspaceEngineError::InvalidInitialPlan(format!(
            "draft `{}` logical identity does not match canonical contract",
            record.draft_id
        )));
    }
    Ok(WorkItemDraftRevision {
        id: record.draft_id.clone(),
        logical_work_item_id: record.candidate.logical_work_item_id.clone(),
        revision_no: record.attempt_index,
        supersedes: record.copied_from_draft_id.clone(),
        revision_reason: PlanRevisionReason::InitialCompile,
        canonical_contract_candidate: record.candidate.canonical_contract_candidate.clone(),
        trigger_repair_request_id: None,
        created_at: record.created_at.clone(),
    })
}

pub(crate) fn parse_compile_verification_plan(
    value: &WorkItemDraftVerificationPlan,
    id: String,
    project_id: String,
    issue_id: String,
    work_item_id: String,
    now: String,
) -> VerificationPlan {
    let commands = value
        .checks
        .iter()
        .filter_map(|check| {
            check.command.as_ref().map(|command| VerificationCommand {
                id: check.check_id.clone(),
                label: check.check_id.clone(),
                command: command.clone(),
                cwd: String::new(),
                purpose: "Canonical verification check".to_string(),
                required: check.required,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            })
        })
        .collect();
    let manual_checks = value
        .checks
        .iter()
        .filter_map(|check| {
            check
                .manual_instruction
                .as_ref()
                .map(|instructions| VerificationManualCheck {
                    id: check.check_id.clone(),
                    label: check.check_id.clone(),
                    instructions: instructions.clone(),
                    required: check.required,
                })
        })
        .collect();

    VerificationPlan {
        id,
        project_id,
        issue_id,
        work_item_id,
        repository_profile_ref: None,
        provider_run_ref: None,
        scope: VerificationScope::Custom,
        commands,
        manual_checks,
        required_gates: value
            .checks
            .iter()
            .filter(|check| check.required)
            .map(|check| check.check_id.clone())
            .collect(),
        risk_notes: Vec::new(),
        confidence: RepositoryProfileConfidence::High,
        fallback_policy: VerificationFallbackPolicy::ManualGate,
        created_at: now.clone(),
        updated_at: now,
    }
}
