use tokio::sync::mpsc;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptScope, CodingExecutionAttempt, CodingGateRequired as CodingGateRequiredModel,
    CodingRoleRunEvent, CodingRoleRunEventPreview, CodingRoleRunEventSummary,
    CodingRoleRunEventType, CodingRoleRunSnapshot, CodingTimelineNode, CodingTimelineNodeStatus,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngineError;
use crate::product::json_store::ProductStoreError;
use crate::web::handlers::{coding_attempt_scope_text, coding_execution_unit_dto};

use super::{CodingWsOutMessage, coding_execution_context, stage_gate_required};

pub(crate) fn build_coding_session_state(
    coding_store: &CodingAttemptStore,
    attempt: CodingExecutionAttempt,
) -> Result<CodingWsOutMessage, ProductStoreError> {
    let execution_context = coding_execution_context(&coding_store.paths(), &attempt)?;
    let timeline_nodes =
        coding_store.get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let active_node_id = active_coding_timeline_node_id(&timeline_nodes);
    let testing_report = coding_store
        .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .last();
    let code_review_reports = coding_store.list_code_review_reports(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let review_request = coding_store
        .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .last();
    let internal_pr_review = coding_store
        .list_internal_pr_reviews(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .last();
    let latest_analyst_decision = coding_store.latest_analyst_decision(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let mut pending_gates: Vec<CodingGateRequiredModel> = coding_store
        .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .map(stage_gate_required)
        .collect();
    pending_gates.extend(coding_store.list_open_blocked_gates(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?);
    let role_provider_config_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let pending_choices =
        coding_store.list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let chat_entries =
        coding_store.list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let role_runs = coding_role_run_snapshots(coding_store, &attempt)?;
    let work_item_execution_plan = coding_store.get_work_item_execution_plan(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let work_item_handoff = coding_store.get_visible_work_item_handoff(&attempt)?;
    let units = if matches!(attempt.scope, CodingAttemptScope::WorkItemGroup) {
        coding_store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .map(|unit| coding_execution_unit_dto(&unit))
            .collect()
    } else {
        Vec::new()
    };

    Ok(CodingWsOutMessage::CodingSessionState {
        attempt_id: attempt.id.clone(),
        attempt_scope: coding_attempt_scope_text(&attempt.scope).to_string(),
        work_item_group_id: attempt.work_item_group_id.clone(),
        current_work_item_id: attempt.current_work_item_id.clone(),
        active_unit_id: attempt.active_unit_id.clone(),
        units,
        status: attempt.status,
        stage: attempt.stage,
        branch_name: attempt.branch_name,
        base_branch: attempt.base_branch,
        worktree_path: attempt.worktree_path,
        rework_count: attempt.rework_count,
        max_auto_rework: attempt.max_auto_rework,
        head_commit: attempt.head_commit,
        pushed_remote: attempt.pushed_remote,
        role_provider_config_snapshot: Box::new(role_provider_config_snapshot),
        provider_config_snapshot: Box::new(attempt.provider_config_snapshot),
        chat_entries: Box::new(chat_entries),
        timeline_nodes: Box::new(timeline_nodes),
        active_node_id,
        testing_report: Box::new(testing_report),
        code_review_reports: Box::new(code_review_reports),
        review_request: Box::new(review_request),
        internal_pr_review: Box::new(internal_pr_review),
        pending_gates: Box::new(pending_gates),
        pending_choices: Box::new(pending_choices),
        latest_analyst_decision: Box::new(latest_analyst_decision),
        role_runs: Box::new(role_runs),
        work_item_markdown: execution_context.work_item_markdown,
        verification_commands: Box::new(execution_context.verification_commands),
        work_item_execution_plan: Box::new(work_item_execution_plan),
        work_item_handoff: Box::new(work_item_handoff),
    })
}

pub(crate) fn coding_role_run_snapshots(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<Vec<CodingRoleRunSnapshot>, ProductStoreError> {
    coding_store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .map(|run| {
            let events = match coding_store.list_role_run_events(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &run.id,
            ) {
                Ok(events) => events,
                Err(error) => {
                    tracing::warn!(
                        attempt_id = %attempt.id,
                        role_run_id = %run.id,
                        error = ?error,
                        "failed to read coding role run events for snapshot"
                    );
                    return Ok(CodingRoleRunSnapshot {
                        run,
                        event_summary: None,
                        recent_events: Vec::new(),
                    });
                }
            };
            let event_summary = role_run_event_summary(&events);
            let recent_events = recent_role_run_events(&events, 10);
            Ok(CodingRoleRunSnapshot {
                run,
                event_summary,
                recent_events,
            })
        })
        .collect()
}

fn role_run_event_summary(events: &[CodingRoleRunEvent]) -> Option<CodingRoleRunEventSummary> {
    let last = events.last()?;
    let terminal = events.iter().rev().find(|event| {
        matches!(
            event.event_type,
            CodingRoleRunEventType::MessageComplete
                | CodingRoleRunEventType::ProviderFailed
                | CodingRoleRunEventType::Timeout
                | CodingRoleRunEventType::Aborted
        )
    });
    Some(CodingRoleRunEventSummary {
        event_count: events.len(),
        last_event_at: Some(last.created_at.clone()),
        last_event_type: Some(last.event_type),
        last_event_title: role_run_event_title(last),
        last_event_status: role_run_event_status(last),
        terminal_event_type: terminal.map(|event| event.event_type),
        terminal_reason: terminal.and_then(role_run_event_reason),
    })
}

fn recent_role_run_events(
    events: &[CodingRoleRunEvent],
    limit: usize,
) -> Vec<CodingRoleRunEventPreview> {
    let start = events.len().saturating_sub(limit);
    events[start..]
        .iter()
        .map(|event| CodingRoleRunEventPreview {
            sequence: event.sequence,
            event_type: event.event_type,
            created_at: event.created_at.clone(),
            title: role_run_event_title(event),
            status: role_run_event_status(event),
            detail: role_run_event_payload_text(event, "detail"),
            truncated: event.truncated,
            artifact_ref: event.artifact_ref.clone(),
        })
        .collect()
}

fn role_run_event_title(event: &CodingRoleRunEvent) -> Option<String> {
    role_run_event_payload_text(event, "title")
        .or_else(|| role_run_event_payload_text(event, "mode"))
        .or_else(|| Some(format!("{:?}", event.event_type)))
}

fn role_run_event_status(event: &CodingRoleRunEvent) -> Option<String> {
    role_run_event_payload_text(event, "status")
}

fn role_run_event_reason(event: &CodingRoleRunEvent) -> Option<String> {
    role_run_event_payload_text(event, "reason_code")
        .or_else(|| role_run_event_payload_text(event, "reason"))
        .or_else(|| role_run_event_payload_text(event, "message"))
}

fn role_run_event_payload_text(event: &CodingRoleRunEvent, field: &str) -> Option<String> {
    let value = event.payload.get(field)?;
    value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| value.get("preview")?.as_str().map(ToOwned::to_owned))
}

pub(crate) fn active_coding_timeline_node_id(nodes: &[CodingTimelineNode]) -> Option<String> {
    nodes
        .iter()
        .rev()
        .find(|node| {
            matches!(
                node.status,
                CodingTimelineNodeStatus::Pending
                    | CodingTimelineNodeStatus::Running
                    | CodingTimelineNodeStatus::Blocked
            )
        })
        .map(|node| node.id.clone())
}

pub(crate) async fn emit_current_session_state(
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<(), CodingWorkspaceEngineError> {
    let current = coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let snapshot = build_coding_session_state(coding_store, current)?;
    let _ = event_tx.send(snapshot).await;
    Ok(())
}
