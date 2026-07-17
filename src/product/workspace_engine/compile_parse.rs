use super::*;
use crate::product::models::WorkItemDraftVerificationPlan;

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
