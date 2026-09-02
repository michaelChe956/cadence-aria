//! 阶段 3 Task 8.3 —— Confirmed→advance campaign 用例（Step 1 主路径 + 8.3a
//! Failed/Aborted 回放与 crash 恢复）。
//!
//! 与 8.2 campaign 相同纪律：全部 advance 请求走真实 ws inbound 分发
//! （`handle_workspace_inbound_message` → `handle_advance_from_handler` →
//! `WorkspaceEngine::handle_advance`），断言一律从 durable store 落盘重开读取；
//! Confirmed 前置态由 8.2 fixture 的真实 confirm 链产生（非 seed），
//! crash/终态注入只经 `register_advance_initialization_failpoint` 测试构造器，
//! 生产路径零感知。provider ledger 在 advance 前后必须 byte 级相等：
//! 证明 Ready 不需要启动任何 coding provider（StartCoding 从不发出）。

use super::campaign_stage3_interactive::{CampaignStage3Harness, campaign_stage3_fixture};
use super::*;

use crate::product::advance_store::{AdvanceInitializationPhase, AdvanceStore};
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus};
use crate::product::workspace_engine::{
    AdvanceInitializationFailpoint, AdvanceInitializationFailpointMode, AdvanceStatus,
    register_advance_initialization_failpoint,
};
use tokio::time::{Duration, timeout};

/// Confirmed 前置态:复用 8.2 campaign fixture,经真实 typed `Confirm` 关门
/// (compile→publish→Confirmed→stage Completed 全部由真实引擎路径产生)。
/// 与 8.2 多轮用例同构:以 durable 落盘为准,不等待 outbound 转发面。
/// 按强权威链要求补齐 accepted source-draft 记录(镜像
/// `advance_handler::seed_advance_draft_records` 的既有 fixture 形态):
/// SC 编译产生的 work item revision 溯源 draft id,但 durable draft 列表
/// 是 fixture 基座的 outline 草稿;没有对应 accepted draft 时 authoritative
/// binding 解析会以 source_draft_error 拒绝。这只是前置态补齐,不触碰
/// 任何被断言的终态(Ready 仍由真实 advance 引擎路径产生)。
async fn confirmed_campaign_harness() -> CampaignStage3Harness {
    let harness = campaign_stage3_fixture(2, Vec::new()).await;
    harness.send(WsInMessage::Confirm).await;
    loop {
        let record = harness.session_record().await;
        if record.status == WorkspaceSessionStatus::Confirmed
            && record.single_candidate_phase == Some(SingleCandidatePhase::Completed)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    seed_accepted_source_drafts(&harness);
    assert_eq!(
        harness.engine.lock().await.current_stage(),
        WorkspaceStage::Completed,
        "真实 confirm 链必须把 session 推进到 Completed(SC+Completed 才收 Advance)"
    );
    harness
}

/// 与 `advance_handler::seed_advance_draft_records` 同构:按编译后的
/// work item revision 溯源补 accepted draft 记录,使 authoritative group
/// binding 能解析出全量 unit(Legacy routing 全 None target)。
fn seed_accepted_source_drafts(harness: &CampaignStage3Harness) {
    use crate::product::models::{
        IssueWorkItemPlanStatus, WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus,
        WorkItemDraftVerificationPlan, WorkItemGenerationMode,
    };
    use crate::product::work_item_plan_store::WorkItemPlanStore;

    let revision_store = crate::product::work_item_revision_store::WorkItemRevisionStore::new(
        harness.app_paths.clone(),
    );
    let lineage = revision_store
        .get_plan_lineage(
            &harness.project_id,
            &harness.issue_id,
            &harness.record_plan_id(),
        )
        .expect("confirmed plan lineage");
    let active_revision_id = lineage
        .active_revision_id
        .clone()
        .expect("confirmed active revision");
    let revision = revision_store
        .get_plan_revision(
            &harness.project_id,
            &harness.issue_id,
            &harness.record_plan_id(),
            &active_revision_id,
        )
        .expect("active plan revision");
    let plan = harness
        .lifecycle
        .get_issue_work_item_plan(
            &harness.project_id,
            &harness.issue_id,
            &harness.record_plan_id(),
        )
        .expect("confirmed plan");
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);
    let plan_store = WorkItemPlanStore::new(harness.app_paths.clone());
    for (logical_id, revision_id) in &revision.work_item_bindings {
        let work_item_revision = revision_store
            .get_work_item_revision(&lineage, logical_id, revision_id)
            .expect("compiled work item revision");
        let draft = WorkItemDraftRecord {
            project_id: harness.project_id.clone(),
            issue_id: harness.issue_id.clone(),
            plan_id: harness.record_plan_id(),
            draft_id: work_item_revision.source_draft_revision_id.clone(),
            outline_id: format!("outline_{logical_id}"),
            generation_round_id: "round_campaign_advance".to_string(),
            batch_id: None,
            attempt_index: 1,
            outline_version_ref: "outline_version_campaign_advance".to_string(),
            generation_mode: WorkItemGenerationMode::Serial,
            generation_diagnostics: None,
            candidate: WorkItemDraftCandidate {
                target_repository_id: None,
                outline_id: format!("outline_{logical_id}"),
                logical_work_item_id: logical_id.clone(),
                canonical_contract_candidate: work_item_revision.canonical_contract.clone(),
                verification_plan: WorkItemDraftVerificationPlan { checks: Vec::new() },
            },
            status: WorkItemDraftStatus::Accepted,
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id: "campaign_advance_fixture".to_string(),
            accepted_at: Some("2026-08-31T00:00:00Z".to_string()),
            superseded_at: None,
            created_at: "2026-08-31T00:00:00Z".to_string(),
            updated_at: "2026-08-31T00:00:00Z".to_string(),
        };
        plan_store
            .put_draft_record(&draft)
            .expect("seed accepted source draft");
    }
}

/// 全 issue provider-start ledger 快照(advance 前后 byte 级比对)。
fn provider_ledger_snapshot(harness: &CampaignStage3Harness) -> Vec<Vec<u8>> {
    let mut snapshots = Vec::new();
    for entry in std::fs::read_dir(
        harness
            .app_paths
            .issue_root(&harness.project_id, &harness.issue_id)
            .join("workspace-sessions"),
    )
    .expect("workspace-sessions dir")
    {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            snapshots.push(std::fs::read(&path).expect("session bytes"));
        }
    }
    snapshots.sort();
    snapshots
}

fn shared_worktree(harness: &CampaignStage3Harness) -> crate::product::models::IssueSharedWorktree {
    harness
        .lifecycle
        .get_issue_shared_worktree(&harness.project_id, &harness.issue_id)
        .expect("shared worktree lookup")
        .expect("advance must bind the issue shared worktree")
}

/// 从 durable coding store 重开读取 campaign attempt 事实(唯一权威)。
fn campaign_attempt(
    harness: &CampaignStage3Harness,
    attempt_id: &str,
) -> crate::product::coding_models::CodingExecutionAttempt {
    CodingAttemptStore::new(harness.app_paths.clone())
        .get_attempt(&harness.project_id, &harness.issue_id, attempt_id)
        .expect("durable campaign attempt")
}

#[tokio::test]
async fn campaign_stage3_advance_confirmed_plan_is_ready_without_provider_start() {
    let harness = confirmed_campaign_harness().await;
    let advance_store = AdvanceStore::new(harness.app_paths.clone());
    let coding_store = CodingAttemptStore::new(harness.app_paths.clone());
    let ledger_before = provider_ledger_snapshot(&harness);
    assert!(
        !ledger_before.is_empty(),
        "fixture 至少含 plan/child session 记录"
    );

    // —— typed advance:稳定 command_id,真实 ws 分发 ——
    harness
        .send(WsInMessage::Advance {
            command_id: "cmd-campaign-adv-1".to_string(),
        })
        .await;
    let completed = harness.await_gate_event("advance_completed").await;
    let WsOutMessage::AdvanceCompleted {
        command_id,
        attempt_id,
        workspace_entry,
    } = completed
    else {
        panic!("expected advance_completed, got {completed:?}");
    };
    assert_eq!(command_id, "cmd-campaign-adv-1");
    let attempt_id = attempt_id.clone();
    let workspace_entry = workspace_entry.clone();

    // —— durable 断言:全部落盘重开读取 ——
    let record = advance_store
        .get_advance_by_command_id(&harness.project_id, &harness.issue_id, "cmd-campaign-adv-1")
        .expect("durable advance record")
        .expect("advance record persisted");
    assert_eq!(record.status, AdvanceStatus::Ready);
    assert_eq!(record.attempt_id.as_deref(), Some(attempt_id.as_str()));
    assert_eq!(
        record.workspace_entry.as_deref(),
        Some(workspace_entry.as_str())
    );
    let journal = advance_store
        .get_advance_initialization(&record)
        .expect("journal lookup")
        .expect("durable advance initialization journal");
    assert_eq!(journal.phase, AdvanceInitializationPhase::Ready);

    // 唯一 attempt:全 issue 只有一个 group attempt。
    let attempts = coding_store
        .list_attempts_for_issue(&harness.project_id, &harness.issue_id)
        .expect("attempts for issue");
    assert_eq!(attempts.len(), 1, "advance 只初始化唯一 group attempt");
    let attempt = campaign_attempt(&harness, &attempt_id);
    assert_eq!(
        attempt.admission_kind,
        crate::product::coding_models::CodingAdmissionKind::ScAdvance
    );
    assert_eq!(
        attempt.status,
        crate::product::coding_models::CodingAttemptStatus::Created,
        "Ready 只初始化 attempt,绝不进入 Running(StartCoding 从不发出)"
    );

    // all units + binding revision。
    let group = coding_store
        .get_group_initialization(
            &harness.project_id,
            &harness.issue_id,
            &harness.record_plan_id(),
        )
        .expect("durable group initialization");
    assert_eq!(group.attempt.id, attempt_id);
    let units = coding_store
        .list_coding_units(&harness.project_id, &harness.issue_id, &attempt_id)
        .expect("durable units");
    assert_eq!(units.len(), group.units.len());
    assert_eq!(units.len(), attempts_work_item_count(&harness));
    let binding = coding_store
        .get_plan_binding(&attempt)
        .expect("durable plan binding");
    let lineage = crate::product::work_item_revision_store::WorkItemRevisionStore::new(
        harness.app_paths.clone(),
    )
    .get_plan_lineage(
        &harness.project_id,
        &harness.issue_id,
        &harness.record_plan_id(),
    )
    .expect("plan lineage");
    assert_eq!(
        binding.bound_plan_revision_id,
        lineage
            .active_revision_id
            .expect("confirmed plan active revision"),
        "binding revision 必须钉在 Confirmed 时的 active plan revision"
    );

    // worktree lease:lock owner 绑定到同一 attempt。
    let worktree = shared_worktree(&harness);
    assert_eq!(
        worktree.current_lock_owner_id.as_deref(),
        Some(attempt_id.as_str())
    );

    // —— provider ledger before/after byte 级相等:advance 不启动任何 provider ——
    assert_eq!(
        provider_ledger_snapshot(&harness),
        ledger_before,
        "advance 前后 provider-start ledger 必须完全相等"
    );

    // —— outbound workspace entry 可 GET/WS 读取 ——
    // GET:真实 HTTP handler(`get_coding_attempt`)按 attempt_id 读 group snapshot。
    let state = WebAppState::new(
        harness.root.path().to_path_buf(),
        crate::web::runtime::WebRuntime::new_fake(harness.root.path().to_path_buf()),
    );
    let path = axum::extract::Path(crate::web::handlers::CodingAttemptRoutePath {
        project_id: Some(harness.project_id.clone()),
        issue_id: Some(harness.issue_id.clone()),
        attempt_id: attempt_id.clone(),
    });
    let axum::Json(snapshot) =
        crate::web::handlers::get_coding_attempt(axum::extract::State(state), path)
            .await
            .expect("GET group attempt snapshot from advance workspace entry");
    assert_eq!(snapshot.attempt.attempt_id, attempt_id);
    assert_eq!(snapshot.attempt.status, "created");
    assert_eq!(snapshot.attempt.attempt_scope, "work_item_group");
    assert_eq!(snapshot.units.len(), units.len());
    assert!(
        snapshot.group_progress.is_some(),
        "HTTP snapshot 带 group aggregate"
    );

    // WS:真实 coding ws session-state 构建器按同一 durable facts 读出。
    let ws_state =
        crate::web::coding_ws_handler::build_coding_session_state(&coding_store, attempt.clone())
            .expect("coding WS session state readable");
    let crate::web::coding_ws_handler::CodingWsOutMessage::CodingSessionState {
        units: ws_units,
        group_progress: ws_aggregate,
        attempt_id: ws_attempt_id,
        work_item_group_id,
        ..
    } = ws_state
    else {
        panic!("expected coding session state");
    };
    assert_eq!(ws_attempt_id, attempt_id);
    assert_eq!(ws_units.len(), units.len());
    assert_eq!(
        work_item_group_id.as_deref(),
        Some(harness.record_plan_id().as_str())
    );
    assert!(ws_aggregate.is_some(), "coding WS 带 group aggregate");

    // —— 幂等 lineage:同 command 重发 / 同 plan 不同 command 均返回同 attempt/record ——
    for command_id in ["cmd-campaign-adv-1", "cmd-campaign-adv-2"] {
        harness
            .send(WsInMessage::Advance {
                command_id: command_id.to_string(),
            })
            .await;
        let replay = harness.await_gate_event("advance_completed").await;
        let WsOutMessage::AdvanceCompleted {
            attempt_id: replay_attempt,
            ..
        } = replay
        else {
            panic!("expected replayed advance_completed for {command_id}");
        };
        assert_eq!(
            replay_attempt, attempt_id,
            "{command_id} 重发必须返回同一 attempt lineage"
        );
        assert_eq!(
            coding_store
                .list_attempts_for_issue(&harness.project_id, &harness.issue_id)
                .expect("attempts after replay")
                .len(),
            1,
            "重发不得创建第二个 attempt"
        );
    }
    let plan_replay = advance_store
        .get_advance_for_plan(
            &harness.project_id,
            &harness.issue_id,
            &harness.record_plan_id(),
        )
        .expect("plan-level advance record")
        .expect("plan advance record");
    assert_eq!(plan_replay.id, record.id);
    assert_eq!(plan_replay.attempt_id.as_deref(), Some(attempt_id.as_str()));
}

impl CampaignStage3Harness {
    fn record_plan_id(&self) -> String {
        self.session_record_blocking().entity_id.clone()
    }
}

fn attempts_work_item_count(harness: &CampaignStage3Harness) -> usize {
    harness
        .lifecycle
        .list_work_items(&harness.project_id, &harness.issue_id)
        .expect("work items")
        .len()
}

/// durable 不变量快照:attempt/unit/lock 三计数(重发前后必须不变)。
#[derive(Debug, PartialEq, Eq)]
struct AdvanceDurableCounts {
    attempts: usize,
    units: usize,
    lock_owner: Option<String>,
}

fn advance_durable_counts(harness: &CampaignStage3Harness) -> AdvanceDurableCounts {
    let coding_store = CodingAttemptStore::new(harness.app_paths.clone());
    let attempts = coding_store
        .list_attempts_for_issue(&harness.project_id, &harness.issue_id)
        .expect("attempts");
    let units = attempts
        .last()
        .map(|attempt| {
            coding_store
                .list_coding_units(&harness.project_id, &harness.issue_id, &attempt.id)
                .expect("units")
                .len()
        })
        .unwrap_or(0);
    let lock_owner = harness
        .lifecycle
        .get_issue_shared_worktree(&harness.project_id, &harness.issue_id)
        .expect("worktree lookup")
        .and_then(|worktree| worktree.current_lock_owner_id);
    AdvanceDurableCounts {
        attempts: attempts.len(),
        units,
        lock_owner,
    }
}

async fn send_advance_and_await(
    harness: &CampaignStage3Harness,
    command_id: &str,
    expected: &str,
) -> WsOutMessage {
    harness
        .send(WsInMessage::Advance {
            command_id: command_id.to_string(),
        })
        .await;
    harness.await_gate_event(expected).await
}

/// 8.3a —— Failed/Aborted 终态:新旧 command 重发都只能拿回原 record,
/// attempt/unit/lock 不变量不变(不重建、不重建、不换锁)。
#[tokio::test]
async fn campaign_stage3_advance_failed_or_aborted_returns_original_attempt() {
    // —— Failed 由真实引擎路径产生:Error 模式 failpoint 注入 PlanBindingSaved ——
    let harness = confirmed_campaign_harness().await;
    let advance_store = AdvanceStore::new(harness.app_paths.clone());
    let request = crate::product::workspace_engine::AdvanceInput {
        command_id: "cmd-campaign-adv-fail".to_string(),
        project_id: harness.project_id.clone(),
        issue_id: harness.issue_id.clone(),
        plan_id: harness.record_plan_id(),
    };
    let _failpoint = register_advance_initialization_failpoint(
        &request,
        AdvanceInitializationFailpoint::PlanBindingSaved,
        AdvanceInitializationFailpointMode::Error,
    );
    let rejected =
        send_advance_and_await(&harness, "cmd-campaign-adv-fail", "advance_rejected").await;
    let WsOutMessage::AdvanceRejected { code, reason, .. } = rejected else {
        panic!("expected advance_rejected, got {rejected:?}");
    };
    assert_eq!(code, "ADVANCE_HANDLER_FAILED");
    assert!(
        reason.contains("failpoint"),
        "failpoint 注入的失败原因: {reason}"
    );
    let failed_record = advance_store
        .get_advance_by_command_id(
            &harness.project_id,
            &harness.issue_id,
            "cmd-campaign-adv-fail",
        )
        .expect("failed record lookup")
        .expect("failed record durable");
    assert_eq!(failed_record.status, AdvanceStatus::Failed);
    let counts_after_failure = advance_durable_counts(&harness);
    assert!(
        counts_after_failure.attempts >= 1,
        "失败前 group attempt 已落盘"
    );

    // —— 旧 command 重发:同一 Failed record 回放,不重建;新 command 命中
    // plan 级幂等(不为新 command 落新 record,直接回放既有 record)——
    for (label, command_id) in [
        ("old command", "cmd-campaign-adv-fail"),
        ("new command", "cmd-campaign-adv-fail-retry"),
    ] {
        let replay = send_advance_and_await(&harness, command_id, "advance_rejected").await;
        let WsOutMessage::AdvanceRejected {
            command_id: replay_command,
            code,
            ..
        } = replay
        else {
            panic!("expected advance_rejected for {label}");
        };
        assert_eq!(replay_command, command_id);
        assert_eq!(
            code, "ADVANCE_REPLAY_NOT_READY",
            "{label}: 非 Ready 终态 record 只能回放拒绝,不得发 advance_completed"
        );
        let replay_record = if command_id == "cmd-campaign-adv-fail" {
            advance_store
                .get_advance_by_command_id(&harness.project_id, &harness.issue_id, command_id)
                .expect("replay record lookup")
                .expect("replay record durable")
        } else {
            advance_store
                .get_advance_for_plan(
                    &harness.project_id,
                    &harness.issue_id,
                    &harness.record_plan_id(),
                )
                .expect("plan record lookup")
                .expect("plan record durable")
        };
        assert_eq!(
            replay_record.id, failed_record.id,
            "{label}: 同一 record lineage"
        );
        assert_eq!(replay_record.attempt_id, failed_record.attempt_id);
    }
    assert_eq!(
        advance_durable_counts(&harness),
        counts_after_failure,
        "Failed 终态重发不得改变 attempt/unit/lock"
    );

    // —— Aborted:当前尚无生产 abort 路径,按前置态 seed 落盘同一 record ——
    let mut aborted = failed_record.clone();
    aborted.status = AdvanceStatus::Aborted;
    aborted.error = None;
    advance_store
        .update_record(&aborted)
        .expect("seed aborted record");
    let counts_after_abort = advance_durable_counts(&harness);
    for (label, command_id) in [
        ("old command", "cmd-campaign-adv-fail"),
        ("new command", "cmd-campaign-adv-abort-retry"),
    ] {
        let replay = send_advance_and_await(&harness, command_id, "advance_rejected").await;
        let WsOutMessage::AdvanceRejected { code, .. } = replay else {
            panic!("expected advance_rejected for aborted {label}");
        };
        assert_eq!(code, "ADVANCE_REPLAY_NOT_READY");
    }
    assert_eq!(
        advance_durable_counts(&harness),
        counts_after_abort,
        "Aborted 终态重发不得改变 attempt/unit/lock"
    );
    let final_record = advance_store
        .get_advance_for_plan(
            &harness.project_id,
            &harness.issue_id,
            &harness.record_plan_id(),
        )
        .expect("final record lookup")
        .expect("final record durable");
    assert_eq!(final_record.status, AdvanceStatus::Aborted);
    assert_eq!(final_record.id, failed_record.id);
}

/// 重启 worker:从 durable record 重建全新 engine(stage Completed),
/// 经真实 ws 分发发送消息(与 harness 引擎实例完全隔离)。
async fn send_via_restarted_worker(
    harness: &CampaignStage3Harness,
    message: WsInMessage,
) -> mpsc::Receiver<OutboundControl> {
    let record = harness.session_record().await;
    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = WorkspaceSession::from_record(record.clone());
    session.stage = WorkspaceStage::Completed;
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            harness.root.path().join("restarted-worker-checkpoints"),
        )),
        harness.lifecycle.clone(),
        event_tx,
        session,
    )));
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let (outbound_tx, outbound_rx) = mpsc::channel(256);
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            harness.root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(harness.root.path().to_path_buf()),
        ),
        engine: engine.clone(),
        run_context: ProviderRunContext {
            provider_registry: Arc::new(ProviderRegistry::new()),
            engine,
            current_run: current_run.clone(),
            workspace_runs: workspace_runs.clone(),
            session_id: harness.session_id.clone(),
            next_run_id: Arc::new(Mutex::new(0)),
            app_paths: harness.app_paths.clone(),
            session_record: record,
        },
        outbound_tx: outbound_tx.clone(),
        current_run,
        workspace_runs,
        session_id: harness.session_id.clone(),
    };
    handle_workspace_inbound_message(context, message).await;
    outbound_rx
}

async fn next_restarted_outbound(
    rx: &mut mpsc::Receiver<OutboundControl>,
    kind: &str,
) -> WsOutMessage {
    loop {
        let outbound = timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("restarted outbound within timeout")
            .expect("restarted outbound channel open");
        let OutboundControl::Text(json) = outbound else {
            panic!("expected text outbound");
        };
        let message: WsOutMessage = serde_json::from_str(&json).expect("outbound ws json");
        let r#type = serde_json::to_value(&message)
            .expect("serialize outbound")
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if r#type == kind {
            return message;
        }
    }
}

/// 8.3a —— journal crash 恢复:record/attempt/binding/lock/units 各 checkpoint
/// 后 crash,重启同 command 恢复,同 attempt/unit ID 完成到 Ready;
/// 崩溃前事件流是恢复后事件流的前缀(崩溃前无终态 advance 事件)。
#[tokio::test]
async fn campaign_stage3_advance_journal_crash_resumes_same_identity() {
    let checkpoints = [
        ("record", AdvanceInitializationFailpoint::RecordPersisted),
        ("attempt", AdvanceInitializationFailpoint::AttemptPersisted),
        ("binding", AdvanceInitializationFailpoint::PlanBindingSaved),
        ("lock", AdvanceInitializationFailpoint::WorktreeBound),
        ("units", AdvanceInitializationFailpoint::UnitsMaterialized),
    ];
    for (label, checkpoint) in checkpoints {
        let harness = Arc::new(confirmed_campaign_harness().await);
        let advance_store = AdvanceStore::new(harness.app_paths.clone());
        let coding_store = CodingAttemptStore::new(harness.app_paths.clone());
        let command_id = format!("cmd-campaign-adv-crash-{label}");
        let request = crate::product::workspace_engine::AdvanceInput {
            command_id: command_id.clone(),
            project_id: harness.project_id.clone(),
            issue_id: harness.issue_id.clone(),
            plan_id: harness.record_plan_id(),
        };
        let _failpoint = register_advance_initialization_failpoint(
            &request,
            checkpoint,
            AdvanceInitializationFailpointMode::Crash,
        );

        // crash 在真实 ws 分发路径内发生(panic 只影响该分发 task)。
        let crashed = {
            let harness = Arc::clone(&harness);
            let command_id = command_id.clone();
            tokio::spawn(async move {
                harness.send(WsInMessage::Advance { command_id }).await;
            })
        };
        assert!(
            crashed.await.is_err(),
            "{label}: crash failpoint 必须中断真实分发路径"
        );

        // 崩溃前无终态 advance 事件(事件前缀性质:前缀里没有 advance_completed)。
        let first_record = advance_store
            .get_advance_by_command_id(&harness.project_id, &harness.issue_id, &command_id)
            .expect("crash record lookup")
            .unwrap_or_else(|| panic!("{label}: record 必须在 checkpoint 前已落盘"));
        assert_eq!(first_record.status, AdvanceStatus::Initializing);
        let first_group = coding_store
            .get_group_initialization(
                &harness.project_id,
                &harness.issue_id,
                &harness.record_plan_id(),
            )
            .ok();
        let first_attempt_id = first_group.as_ref().map(|group| group.attempt.id.clone());
        let first_unit_ids = first_group.as_ref().map(|group| {
            group
                .units
                .iter()
                .map(|unit| unit.id.clone())
                .collect::<Vec<_>>()
        });

        // 重启(全新 engine 实例)后同 command 恢复到 Ready。
        let mut outbound = Box::pin(
            send_via_restarted_worker(
                &harness,
                WsInMessage::Advance {
                    command_id: command_id.clone(),
                },
            )
            .await,
        );
        let completed = next_restarted_outbound(&mut outbound, "advance_completed").await;
        let WsOutMessage::AdvanceCompleted {
            attempt_id: resumed_attempt_id,
            ..
        } = completed
        else {
            panic!("{label}: expected advance_completed after restart");
        };

        let final_record = advance_store
            .get_advance_by_command_id(&harness.project_id, &harness.issue_id, &command_id)
            .expect("final record lookup")
            .expect("final record durable");
        assert_eq!(
            final_record.id, first_record.id,
            "{label}: 同一 record identity"
        );
        assert_eq!(final_record.status, AdvanceStatus::Ready);
        assert_eq!(
            final_record.attempt_id.as_deref(),
            Some(resumed_attempt_id.as_str())
        );
        let journal = advance_store
            .get_advance_initialization(&final_record)
            .expect("journal lookup")
            .expect("journal durable");
        assert_eq!(journal.phase, AdvanceInitializationPhase::Ready);
        if let Some(first_attempt_id) = first_attempt_id {
            assert_eq!(
                resumed_attempt_id, first_attempt_id,
                "{label}: 恢复必须复用 checkpoint 前的 attempt identity"
            );
        }
        let final_units = coding_store
            .list_coding_units(&harness.project_id, &harness.issue_id, &resumed_attempt_id)
            .expect("final units");
        if let Some(first_unit_ids) = first_unit_ids {
            assert_eq!(
                final_units
                    .iter()
                    .map(|unit| unit.id.clone())
                    .collect::<Vec<_>>(),
                first_unit_ids,
                "{label}: 恢复必须复用 checkpoint 前的 unit identities"
            );
        }
        assert_eq!(
            coding_store
                .list_attempts_for_issue(&harness.project_id, &harness.issue_id)
                .expect("attempts after resume")
                .len(),
            1,
            "{label}: crash+resume 全程只允许一个 group attempt"
        );
    }
}
