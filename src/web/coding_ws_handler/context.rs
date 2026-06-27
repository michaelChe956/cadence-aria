use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::artifact_extraction::extract_artifact_content;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingChatEntry, CodingContextNote, CodingEntryType,
    CodingExecutionAttempt, CodingExecutionStage, CodingProviderPermissionMode, CodingProviderRole,
    CodingStageGateState, CodingStageGateStatus, InternalPrReview, TestingReport,
};
use crate::product::coding_workspace_engine::CodingExecutionContext;
use crate::product::coding_workspace_engine::CodingWorkspaceEngineError;
use crate::product::coding_workspace_runner::{
    apply_provider_selection_to_snapshots, coding_provider_role_for_stage,
    parse_coding_provider_role,
};
use crate::product::json_store::ProductStoreError;

use super::active_coding_timeline_node_id;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    ProviderName, WorkItemExecutionPlanStatus, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceType,
};
use crate::product::repository_store::RepositoryStore;
use crate::product::test_executor::{
    TestCommandSpec, discover_test_commands, planned_test_commands_from_markdown,
};

pub(crate) fn current_work_item_id_for_attempt(attempt: &CodingExecutionAttempt) -> &str {
    attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id)
}

pub(crate) fn coding_execution_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CodingExecutionContext, ProductStoreError> {
    let current_work_item_id = current_work_item_id_for_attempt(attempt);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
    let work_item_session = sessions
        .iter()
        .rev()
        .find(|session| {
            session.entity_id == current_work_item_id
                && session.workspace_type == WorkspaceType::WorkItem
                && session.status == WorkspaceSessionStatus::Confirmed
        })
        .or_else(|| {
            sessions.iter().rev().find(|session| {
                session.entity_id == current_work_item_id
                    && session.workspace_type == WorkspaceType::WorkItem
            })
        });
    let work_item_markdown = match work_item_session {
        Some(session) => lifecycle
            .list_artifact_versions(&session.id)?
            .into_iter()
            .last()
            .map(|version| version.to_markdown_string())
            .and_then(|markdown| select_work_item_markdown(Some(markdown), session))
            .or_else(|| select_work_item_markdown(None, session)),
        None => None,
    };
    let verification_commands = work_item_markdown
        .as_deref()
        .map(planned_test_commands_from_markdown)
        .unwrap_or_default()
        .into_iter()
        .map(|spec| spec.command.join(" "))
        .collect();

    Ok(CodingExecutionContext {
        work_item_markdown,
        verification_commands,
    })
}

pub(crate) fn ensure_work_item_execution_plan_confirmed(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<(), CodingWorkspaceEngineError> {
    let current_work_item_id = current_work_item_id_for_attempt(attempt);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let work_items = lifecycle.list_work_items(&attempt.project_id, &attempt.issue_id)?;
    let Some(work_item) = work_items
        .iter()
        .find(|item| item.id == current_work_item_id)
    else {
        return Ok(());
    };
    if !work_item.require_execution_plan_confirm {
        return Ok(());
    }

    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let plan = coding_store.get_work_item_execution_plan(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    match plan.map(|p| p.status) {
        Some(WorkItemExecutionPlanStatus::Confirmed) => Ok(()),
        _ => Err(CodingWorkspaceEngineError::ExecutionPlanNotConfirmed(
            attempt.id.clone(),
        )),
    }
}

pub(crate) fn repository_path_for_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<PathBuf, CodingWorkspaceEngineError> {
    let current_work_item_id = current_work_item_id_for_attempt(attempt);
    let work_item = LifecycleStore::new(app_paths.clone())
        .list_work_items(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|work_item| work_item.id == current_work_item_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "work_item",
            id: current_work_item_id.to_string(),
        })?;
    RepositoryStore::new(app_paths.clone())
        .list(&attempt.project_id)?
        .into_iter()
        .find(|repository| repository.id == work_item.repository_id)
        .map(|repository| repository.path)
        .ok_or({
            CodingWorkspaceEngineError::Store(ProductStoreError::NotFound {
                kind: "repository",
                id: work_item.repository_id,
            })
        })
}

pub(crate) fn update_provider_selection(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: &str,
    provider: ProviderName,
) -> Result<(CodingExecutionAttempt, CodingProviderRole, ProviderName), ProductStoreError> {
    let mut snapshot = attempt.provider_config_snapshot.clone();
    let mut role_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let changed_provider = provider.clone();
    let changed_role =
        apply_provider_selection_to_snapshots(role, provider, &mut snapshot, &mut role_snapshot)
            .map_err(ProductStoreError::Io)?;
    let updated = coding_store.update_attempt_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        snapshot,
    )?;
    coding_store.update_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        role_snapshot,
    )?;
    Ok((updated, changed_role, changed_provider))
}

pub(crate) fn update_provider_permission_mode(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: &str,
    permission_mode: CodingProviderPermissionMode,
) -> Result<(CodingProviderRole, ProviderName), ProductStoreError> {
    let parsed_role = parse_coding_provider_role(role)
        .ok_or_else(|| ProductStoreError::Io(format!("unknown coding role: {role}")))?;
    let mut role_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let provider = role_snapshot.provider_for_role(&parsed_role).clone();
    role_snapshot.set_permission_mode_for_role(&parsed_role, permission_mode);
    coding_store.update_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        role_snapshot,
    )?;
    Ok((parsed_role, provider))
}

pub(crate) fn provider_selection_targets_current_running_stage(
    attempt: &CodingExecutionAttempt,
    role: &str,
) -> bool {
    if attempt.status != crate::product::coding_models::CodingAttemptStatus::Running {
        return false;
    }
    let Some(current_role) = coding_provider_role_for_stage(&attempt.stage) else {
        return false;
    };
    parse_coding_provider_role(role).as_ref() == Some(&current_role)
}

pub(crate) fn confirm_open_stage_gate(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    stage: &CodingExecutionStage,
) -> Result<Option<CodingStageGateState>, ProductStoreError> {
    let Some(gate) = coding_store
        .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .find(|gate| gate.stage == *stage)
    else {
        return Ok(None);
    };
    coding_store
        .update_stage_gate_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            CodingStageGateStatus::Confirmed,
        )
        .map(Some)
}

pub(crate) fn context_note_chat_entry(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    note: CodingContextNote,
) -> Result<CodingChatEntry, ProductStoreError> {
    let timeline_nodes =
        coding_store.get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    Ok(CodingChatEntry {
        id: chat_entry_id_for_context_note(&note.id),
        attempt_id: attempt.id.clone(),
        node_id: active_coding_timeline_node_id(&timeline_nodes),
        role: CodingAgentRole::Author,
        entry_type: CodingEntryType::UserMessage,
        content: Some(note.content),
        metadata: Some(serde_json::json!({
            "context_note_id": note.id,
        })),
        created_at: note.created_at,
    })
}

fn chat_entry_id_for_context_note(note_id: &str) -> String {
    note_id.replacen("coding_context_note", "coding_chat_entry", 1)
}

fn latest_assistant_artifact_markdown(session: &WorkspaceSessionRecord) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .map(|message| extract_artifact_content(&message.content))
        .filter(|content| !content.trim().is_empty())
}

pub(crate) fn select_work_item_markdown(
    version_markdown: Option<String>,
    session: &WorkspaceSessionRecord,
) -> Option<String> {
    match version_markdown {
        Some(markdown) if !planned_test_commands_from_markdown(&markdown).is_empty() => {
            Some(markdown)
        }
        Some(markdown) => latest_assistant_artifact_markdown(session).or(Some(markdown)),
        None => latest_assistant_artifact_markdown(session),
    }
}

pub(crate) fn test_specs_for_attempt(
    attempt: &CodingExecutionAttempt,
    context: &CodingExecutionContext,
) -> Vec<TestCommandSpec> {
    if let Some(markdown) = context.work_item_markdown.as_deref() {
        let planned = planned_test_commands_from_markdown(markdown);
        if !planned.is_empty() {
            return planned;
        }
    }
    attempt
        .worktree_path
        .as_ref()
        .map(discover_test_commands)
        .unwrap_or_default()
}

pub(crate) fn testing_rework_evidence(report: &TestingReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| {
        format!(
            "TestingReport serialization failed; overall_status={:?}",
            report.overall_status
        )
    })
}

pub(crate) fn code_review_rework_evidence(report: &CodeReviewReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| {
        format!(
            "CodeReviewReport serialization failed; verdict={:?}; summary={}",
            report.verdict, report.summary
        )
    })
}

pub(crate) fn internal_pr_review_rework_evidence(review: &InternalPrReview) -> String {
    serde_json::to_string_pretty(review).unwrap_or_else(|_| {
        format!(
            "InternalPrReview serialization failed; verdict={:?}; summary={}",
            review.verdict, review.summary
        )
    })
}
