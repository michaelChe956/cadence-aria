use tokio::sync::mpsc;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingGateAction, CodingGateActionType, CodingGateKind,
    CodingGateRequired as CodingGateRequiredModel, CodingProviderRole, CodingRoleRunEvent,
    CodingRoleRunEventPreview, CodingRoleRunEventSummary, CodingRoleRunEventType,
    CodingRoleRunSnapshot, CodingTimelineNode, CodingTimelineNodeStatus, GroupReviewArtifactRef,
};
use crate::product::coding_workspace_engine::{
    CodingWorkspaceEngineError, recoverable_failed_code_review,
};
use crate::product::json_store::ProductStoreError;
use crate::product::models::ProviderName;
use crate::web::handlers::{coding_attempt_scope_text, coding_execution_unit_dto};
use crate::web::types::GroupReviewArtifactProjection;
use crate::web::workspace_ws_types::{
    WsExecutionEvent, WsExecutionEventKind, WsExecutionEventStatus,
};

use super::protocol::CodingExecutionEventReplay;
use super::{
    CodingWsOutMessage, coding_choice_request_frame, coding_execution_context, stage_gate_required,
};

pub(crate) fn build_coding_session_state(
    coding_store: &CodingAttemptStore,
    attempt: CodingExecutionAttempt,
) -> Result<CodingWsOutMessage, CodingWorkspaceEngineError> {
    let reconciliation = coding_store.reconcile_linked_plan_repair_pause(&attempt)?;
    let attempt = reconciliation.attempt;
    let execution_context = coding_execution_context(&coding_store.paths(), &attempt)?;
    let timeline_nodes =
        coding_store.get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let active_node_id = active_coding_timeline_node_id(&timeline_nodes);
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
    let group_final_readiness = coding_store.get_group_final_readiness_snapshot(&attempt)?;
    let group_review_artifacts = {
        let projection = GroupReviewArtifactProjection {
            shard_reports: coding_store
                .list_group_review_shard_reports_for_attempt(&attempt)?
                .into_iter()
                .map(|report| GroupReviewArtifactRef {
                    id: report.id,
                    raw_provider_output_refs: report.raw_provider_output_refs,
                })
                .collect(),
            reduction_reports: coding_store
                .list_group_review_reduction_reports_for_attempt(&attempt)?
                .into_iter()
                .map(|report| GroupReviewArtifactRef {
                    id: report.id,
                    raw_provider_output_refs: report.raw_provider_output_refs,
                })
                .collect(),
        };
        let has_artifacts =
            !projection.shard_reports.is_empty() || !projection.reduction_reports.is_empty();
        has_artifacts.then_some(projection)
    };
    let pending_gates = coding_pending_gates(coding_store, &attempt)?;
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
    let execution_events = coding_execution_event_replay(coding_store, &attempt);
    let work_item_execution_plan = coding_store.get_work_item_execution_plan(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let linked_plan_repair = reconciliation.snapshot;
    let units = if matches!(attempt.scope, CodingAttemptScope::WorkItemGroup) {
        coding_store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .map(|unit| coding_execution_unit_dto(&unit))
            .collect()
    } else {
        Vec::new()
    };
    let (group_coding_progress, group_progress) =
        if matches!(attempt.scope, CodingAttemptScope::WorkItemGroup) {
            let (progress, aggregate) =
                crate::web::handlers::build_group_work_item_progress(coding_store, &attempt)?;
            (Some(progress), Some(aggregate))
        } else {
            (None, None)
        };

    Ok(CodingWsOutMessage::CodingSessionState {
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        attempt_id: attempt.id.clone(),
        attempt_scope: coding_attempt_scope_text(&attempt.scope).to_string(),
        work_item_group_id: attempt.work_item_group_id.clone(),
        current_work_item_id: attempt.current_work_item_id.clone(),
        active_unit_id: attempt.active_unit_id.clone(),
        units,
        group_coding_progress: Box::new(group_coding_progress),
        group_progress: Box::new(group_progress),
        status: attempt.status,
        stage: attempt.stage,
        branch_name: attempt.branch_name,
        base_branch: attempt.base_branch,
        worktree_path: attempt.worktree_path,
        rework_count: attempt.rework_count,
        max_auto_rework: attempt.max_auto_rework,
        head_commit: Box::new(attempt.head_commit),
        pushed_remote: Box::new(attempt.pushed_remote),
        role_provider_config_snapshot: Box::new(role_provider_config_snapshot),
        provider_config_snapshot: Box::new(attempt.provider_config_snapshot),
        chat_entries: Box::new(chat_entries),
        execution_events: Box::new(execution_events),
        timeline_nodes: Box::new(timeline_nodes),
        active_node_id: Box::new(active_node_id),
        code_review_reports: Box::new(code_review_reports),
        review_request: Box::new(review_request),
        internal_pr_review: Box::new(internal_pr_review),
        group_review_artifacts: Box::new(group_review_artifacts),
        group_final_readiness: Box::new(group_final_readiness),
        pending_gates: Box::new(pending_gates),
        pending_choices: Box::new(pending_choices),
        role_runs: Box::new(role_runs),
        work_item_markdown: Box::new(execution_context.work_item_markdown),
        verification_commands: Box::new(execution_context.verification_commands),
        work_item_execution_plan: Box::new(work_item_execution_plan),
        linked_plan_repair: Box::new(linked_plan_repair),
    })
}

/// F-43：新连接 attach 时必须补发的未决 provider choice 帧。
///
/// 源是 durable choice-gate 目录（引擎 `emit_choice_request` 落盘的那份），只取
/// `status=Open`：`resolve_choice_gate` 会把已答 gate 移入 `resolved/`，过期/撤销
/// 亦不在 Open 集内，因此「已答/过期不重发」由数据本身保证，无需额外时间戳比对。
///
/// 为什么投影（快照 `pending_choices`）之外还要补帧：快照只重建卡片数据，不重建
/// 客户端按帧维护的「待答卡」接线；页面刷新/新开连接若不补帧，coder 等服务端、
/// 界面等入口——刷新即死锁（同构 F-24 workspace WS 的重订阅补发）。
pub(crate) fn pending_choice_frames(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<Vec<CodingWsOutMessage>, CodingWorkspaceEngineError> {
    Ok(coding_store
        .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .iter()
        .map(coding_choice_request_frame)
        .collect())
}

/// F-13 刷新回放：实时 `coding_execution_event` 只广播不落 chat_entries，运行期
/// 对话（prompt、provider 生命周期、命令执行）持久化在 role-run 审计 journal。
/// 此处从 journal 重建与实时广播同形的 `WsExecutionEvent` 列表，随
/// `coding_session_state` 快照下发，前端刷新/重连后按序装载回运行对话；
/// TextDelta/权限/选择等仍走既有实时与快照通道，不在此回放。
pub(crate) fn coding_execution_event_replay(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Vec<CodingExecutionEventReplay> {
    let Ok(runs) = coding_store.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
    else {
        return Vec::new();
    };
    let mut replay = Vec::new();
    for run in runs {
        let Ok(events) = coding_store.list_role_run_events(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &run.id,
        ) else {
            continue;
        };
        for event in events {
            if let Some(ws_event) = replay_ws_event_from_journal_event(&event) {
                replay.push(CodingExecutionEventReplay {
                    event: ws_event,
                    created_at: event.created_at.clone(),
                });
            }
        }
    }
    // runs 已按 id 排序、journal 已按 sequence 排序；跨 run 以 created_at 稳定归并。
    replay.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    replay
}

fn replay_ws_event_from_journal_event(event: &CodingRoleRunEvent) -> Option<WsExecutionEvent> {
    let payload = event.payload.as_object()?;
    let node_id = event.node_id.clone();
    let agent = payload
        .get("provider")
        .and_then(|value| serde_json::from_value::<ProviderName>(value.clone()).ok());
    match event.event_type {
        CodingRoleRunEventType::ProviderPrompt => Some(WsExecutionEvent {
            event_id: format!(
                "{}_prompt",
                node_id.as_deref().unwrap_or(&event.role_run_id)
            ),
            node_id,
            agent,
            kind: WsExecutionEventKind::Output,
            status: WsExecutionEventStatus::Started,
            title: "Provider Prompt".to_string(),
            detail: journal_payload_text(payload, "role"),
            command: None,
            cwd: None,
            output: journal_payload_text(payload, "prompt"),
            exit_code: None,
        }),
        CodingRoleRunEventType::StatusChanged => {
            let status_text = journal_payload_text(payload, "status")?;
            let (status, snake) = replay_provider_status(&status_text)?;
            Some(WsExecutionEvent {
                event_id: format!(
                    "{}_provider_status_{snake}",
                    node_id.as_deref().unwrap_or(&event.role_run_id)
                ),
                node_id,
                agent,
                kind: WsExecutionEventKind::Provider,
                status,
                title: format!("Provider {snake}"),
                detail: None,
                command: None,
                cwd: None,
                output: None,
                exit_code: None,
            })
        }
        CodingRoleRunEventType::ExecutionEvent => Some(WsExecutionEvent {
            event_id: journal_payload_text(payload, "event_id")
                .unwrap_or_else(|| format!("{}_{}", event.role_run_id, event.sequence)),
            node_id,
            agent,
            kind: journal_payload_text(payload, "kind")
                .map(|text| text.to_lowercase())
                .and_then(|text| replay_event_kind(&text))
                .unwrap_or(WsExecutionEventKind::Provider),
            status: journal_payload_text(payload, "status")
                .and_then(|text| replay_event_status(&text))
                .unwrap_or(WsExecutionEventStatus::Completed),
            title: journal_payload_text(payload, "title")
                .unwrap_or_else(|| "Execution event".to_string()),
            detail: journal_payload_text(payload, "detail"),
            command: journal_payload_text(payload, "command"),
            cwd: journal_payload_text(payload, "cwd"),
            output: journal_payload_text(payload, "output"),
            exit_code: payload
                .get("exit_code")
                .and_then(|value| value.as_i64())
                .map(|code| code as i32),
        }),
        CodingRoleRunEventType::ToolCall => {
            let tool_name = journal_payload_text(payload, "tool_name")?;
            Some(WsExecutionEvent {
                event_id: journal_payload_text(payload, "id").unwrap_or_else(|| {
                    format!("{}_{}_tool_call", event.role_run_id, event.sequence)
                }),
                node_id,
                agent,
                kind: WsExecutionEventKind::Command,
                status: WsExecutionEventStatus::Started,
                title: tool_name,
                detail: payload
                    .get("input")
                    .and_then(|value| serde_json::to_string(value).ok()),
                command: payload
                    .get("input")
                    .and_then(|value| value.get("command"))
                    .and_then(|value| value.as_str())
                    .map(|text| text.to_string()),
                cwd: None,
                output: None,
                exit_code: None,
            })
        }
        CodingRoleRunEventType::ToolResult => {
            let is_error = payload
                .get("is_error")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            Some(WsExecutionEvent {
                event_id: journal_payload_text(payload, "tool_use_id").unwrap_or_else(|| {
                    format!("{}_{}_tool_result", event.role_run_id, event.sequence)
                }),
                node_id,
                agent,
                kind: WsExecutionEventKind::Command,
                status: if is_error {
                    WsExecutionEventStatus::Failed
                } else {
                    WsExecutionEventStatus::Completed
                },
                title: "Tool result".to_string(),
                detail: None,
                command: None,
                cwd: None,
                output: journal_payload_text(payload, "output"),
                exit_code: Some(i32::from(is_error)),
            })
        }
        _ => None,
    }
}

/// journal 超长文本字段被归一化为 `{preview, artifact_ref, truncated}`，
/// 回放取 preview（全文在 artifact，按需扩展）。
fn journal_payload_text(
    payload: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Option<String> {
    match payload.get(field)? {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Object(object) => object
            .get("preview")
            .and_then(|value| value.as_str())
            .map(|text| text.to_string()),
        _ => None,
    }
}

/// journal 记录的 `ProviderStatus` Debug 名 →（Ws 状态, snake 文案），
/// 与 `ws_event_from_provider_status` 实时映射保持同形。
fn replay_provider_status(text: &str) -> Option<(WsExecutionEventStatus, &'static str)> {
    match text {
        "Starting" => Some((WsExecutionEventStatus::Started, "starting")),
        "Running" => Some((WsExecutionEventStatus::Running, "running")),
        "WaitingApproval" => Some((WsExecutionEventStatus::WaitingApproval, "waiting_approval")),
        "Completed" => Some((WsExecutionEventStatus::Completed, "completed")),
        "Failed" => Some((WsExecutionEventStatus::Failed, "failed")),
        "Aborted" => Some((WsExecutionEventStatus::Aborted, "aborted")),
        _ => None,
    }
}

/// journal 记录的 `ProviderExecutionEventKind` Debug 名（转小写后即 serde 名）。
fn replay_event_kind(snake: &str) -> Option<WsExecutionEventKind> {
    match snake {
        "provider" => Some(WsExecutionEventKind::Provider),
        "turn" => Some(WsExecutionEventKind::Turn),
        "command" => Some(WsExecutionEventKind::Command),
        "output" => Some(WsExecutionEventKind::Output),
        "artifact" => Some(WsExecutionEventKind::Artifact),
        "usage" => Some(WsExecutionEventKind::Usage),
        _ => None,
    }
}

/// journal 记录的 `ProviderExecutionEventStatus` Debug 名。
fn replay_event_status(text: &str) -> Option<WsExecutionEventStatus> {
    match text {
        "Started" => Some(WsExecutionEventStatus::Started),
        "Running" => Some(WsExecutionEventStatus::Running),
        "WaitingApproval" => Some(WsExecutionEventStatus::WaitingApproval),
        "Completed" => Some(WsExecutionEventStatus::Completed),
        "Failed" => Some(WsExecutionEventStatus::Failed),
        "Aborted" => Some(WsExecutionEventStatus::Aborted),
        _ => None,
    }
}

pub(crate) fn coding_pending_gates(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<Vec<CodingGateRequiredModel>, CodingWorkspaceEngineError> {
    let mut pending_gates: Vec<CodingGateRequiredModel> = coding_store
        .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .map(stage_gate_required)
        .collect();
    pending_gates.extend(
        coding_store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .filter(|gate| blocked_gate_is_actionable_for_attempt(attempt, gate)),
    );
    if let Some(recovery) = recoverable_failed_code_review(coding_store, attempt)? {
        pending_gates.push(CodingGateRequiredModel {
            gate_id: recovery.gate_id,
            kind: CodingGateKind::Blocked,
            title: "代码审查中断".to_string(),
            description: "上次代码审查已中断，可保留当前修改并重试 Reviewer。".to_string(),
            stage: Some(CodingExecutionStage::CodeReview),
            role: Some(CodingProviderRole::CodeReviewer),
            expires_at: None,
            provider_snapshot: None,
            available_actions: vec![CodingGateAction {
                action_id: "retry_review".to_string(),
                label: "重试代码审查".to_string(),
                action_type: CodingGateActionType::RetryReview,
            }],
            reason_code: Some("failed_code_review_recoverable".to_string()),
            evidence_refs: vec![recovery.failed_node_id, recovery.stale_role_run_id],
            raw_provider_output_ref: None,
            diagnostic: None,
        });
    }
    Ok(pending_gates)
}

fn blocked_gate_is_actionable_for_attempt(
    attempt: &CodingExecutionAttempt,
    gate: &CodingGateRequiredModel,
) -> bool {
    let stage_matches = match gate.stage.as_ref() {
        Some(stage) => stage == &attempt.stage,
        None => true,
    };
    if !stage_matches {
        return false;
    }

    match attempt.status {
        CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman => true,
        CodingAttemptStatus::Running => attempt.stage == CodingExecutionStage::FinalConfirm,
        CodingAttemptStatus::AwaitingManualRecovery
        | CodingAttemptStatus::AwaitingPlanAmendment
        | CodingAttemptStatus::ApplyingPlanAmendment
        | CodingAttemptStatus::AmendmentApplyFailed => false,
        CodingAttemptStatus::Created
        | CodingAttemptStatus::Completed
        | CodingAttemptStatus::Failed
        | CodingAttemptStatus::Aborted => false,
    }
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
        .last()
        .filter(|node| {
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
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<(), CodingWorkspaceEngineError> {
    let current = coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let snapshot = build_coding_session_state(coding_store, current)?;
    let permit = tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            return Err(CodingWorkspaceEngineError::Aborted);
        }
        permit = event_tx.reserve() => permit,
    };
    let permit = permit.map_err(|_| {
        CodingWorkspaceEngineError::ProviderStream("coding_event_channel_closed".to_string())
    })?;
    permit.send(snapshot);
    Ok(())
}
