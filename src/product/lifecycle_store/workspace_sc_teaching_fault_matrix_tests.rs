//! bound-sc-generation-teaching（序 3）Task 9：恢复/重放的故障矩阵 store 面
//! （REQ-WSC-28C，tasks 6.1/8.3）。每种故障一条独立用例 + 明确期望值：
//!
//! - `replay_same_key_no_second_start`：claim 后崩溃重连、同请求重放 ⇒
//!   读取同一窗口（`Replay`）、实际启动数不增加、无新占位条目——Task 6
//!   deferred ②（跨进程重放的物理启动数）的钉死面；
//! - `crash_before_spawn_known_not_started_continues_same_claim`：spawn 前
//!   崩溃（占位 `started=false` 且无 provider ⇒ 可证未启动）⇒ 原 key/原
//!   授权下继续同一 claim（attempt 仍 Claimed、不新增 attempt、不加预算）；
//! - `crash_after_spawn_actual_started_not_replayed`：回填后崩溃 ⇒ 重放不
//!   重播（`ActualStarted` 幂等、无第二条启动）；
//! - `unknown_waits_no_auto_replay`：`LaunchUnknown` ⇒ 沿原等待、零新启动；
//! - `source_context_prompt_drift_zero_start`：重放时 source/context/prompt
//!   digest 任一漂移 ⇒ 实际启动 0、预算与 refs 不变、
//!   `sc_teaching_frozen_input_drift` 诊断。
//!
//! 「崩溃重连」以同一 ProductAppPaths 上的**全新 store 实例**模拟（跨进程
//! 重放：durable 记录是唯一载体）。Task 6 deferred ① 的处置结论：异常退出
//! 后窗口保持 **Active** 待重放——establish `Existing` 读取同一窗口、claim
//! `Replay` 继续同一 attempt，实现侧不存在（也不需要）自动关窗语义；关窗
//! 只由终态 outcome 驱动（`close_sc_teaching_window`）。

use tempfile::tempdir;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, ScTeachingClaimOutcome, ScTeachingClaimRejection,
    ScTeachingEstablishOutcome, WorkItemPlanSessionOptions,
};
use crate::product::models::{ProviderName, WorkspaceSessionRecord, WorkspaceType};
use crate::product::planning_failure::PlanningFailureKind;
use crate::product::work_item_plan_policy::{
    ProviderStartLedgerEntry, RunPolicy, ScGenerationTeachingWindow, ScTeachingAttemptLifecycle,
    ScTeachingWindowRequest, ScTeachingWindowStatus, WorkItemPlanFlowKind,
};

fn digest64(seed: &str) -> String {
    format!("{seed:0<64}")
}

fn create_sc_session(
    store: &LifecycleStore,
    policy: RunPolicy,
    entity_id: &str,
) -> WorkspaceSessionRecord {
    store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: entity_id.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: policy,
                rollout_snapshot: true,
            }),
        })
        .expect("create candidate session")
}

fn sc_teaching_request() -> ScTeachingWindowRequest {
    ScTeachingWindowRequest {
        provider_path_admissible: true,
        failure_source_ref: Some("work_item_plan_source:s1:rev_0001".to_string()),
        failure_source_sha256: digest64("1a"),
        failure_typed_kinds: vec![PlanningFailureKind::InvalidWorkItemHeading {
            line: 3,
            field: "ears".to_string(),
            raw_line: "## ears".to_string(),
            example: "### ears".to_string(),
        }],
        business_request_digest: digest64("2b"),
        material_digests: vec![digest64("3c"), digest64("4d")],
        context_digest: digest64("5e"),
        allowed_edit_closure: "仅允许修改 ears 与 verification 段".to_string(),
        instruction_digest: digest64("6f"),
        prompt_ref: "build_work_item_plan_compile_reredrive_prompt".to_string(),
        prompt_digest: digest64("7a"),
    }
}

fn reserve_original_launch(
    store: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> WorkspaceSessionRecord {
    let key = format!("single_candidate_author:{}:0", session.id);
    let (record, did_start) = store
        .reserve_single_candidate_provider_start(session, &key)
        .expect("reserve original generation launch");
    assert!(did_start);
    record
}

fn establish_approved(
    store: &LifecycleStore,
    generate: &WorkspaceSessionRecord,
) -> WorkspaceSessionRecord {
    match store
        .establish_sc_teaching_window(generate, &sc_teaching_request())
        .expect("establish teaching window")
    {
        ScTeachingEstablishOutcome::Established {
            record,
            approved,
            rejection,
        } => {
            assert!(approved);
            assert_eq!(rejection, None);
            record
        }
        ScTeachingEstablishOutcome::Existing { .. } => {
            panic!("fresh chain must establish a new window")
        }
    }
}

fn claim_granted(
    store: &LifecycleStore,
    expected: &WorkspaceSessionRecord,
) -> WorkspaceSessionRecord {
    match store
        .claim_sc_teaching_attempt(expected, 0)
        .expect("claim teaching attempt")
    {
        ScTeachingClaimOutcome::Granted(record) => record,
        ScTeachingClaimOutcome::Replay(_) => panic!("first claim must grant attempt 2"),
        ScTeachingClaimOutcome::Denied(rejection) => panic!("unexpected denial: {rejection:?}"),
    }
}

fn observe_started(
    store: &LifecycleStore,
    expected: &WorkspaceSessionRecord,
) -> WorkspaceSessionRecord {
    store
        .observe_sc_teaching_provider_start(
            expected,
            0,
            2,
            "codex",
            "2026-11-13T02:00:00Z".to_string(),
        )
        .expect("observe teaching provider start")
}

fn session_json_path(
    store: &LifecycleStore,
    record: &WorkspaceSessionRecord,
) -> std::path::PathBuf {
    store
        .app_paths()
        .issue_lifecycle_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id))
}

fn last_window(record: &WorkspaceSessionRecord) -> &ScGenerationTeachingWindow {
    record
        .sc_generation_teaching_windows
        .last()
        .expect("window")
}

/// 教学占位条目数（`provider_attempt_key` 在场 ⇒ claim 占位）。
fn teaching_placeholder_count(record: &WorkspaceSessionRecord) -> usize {
    record
        .provider_start_ledger
        .iter()
        .filter(|entry| entry.provider_attempt_key.is_some())
        .count()
}

fn teaching_placeholder_entry<'a>(
    record: &'a WorkspaceSessionRecord,
    attempt_key: &str,
) -> &'a ProviderStartLedgerEntry {
    record
        .provider_start_ledger
        .iter()
        .find(|entry| entry.provider_attempt_key.as_deref() == Some(attempt_key))
        .expect("teaching placeholder ledger entry")
}

/// 教学启动的实际启动观察数（`started && provider && started_at`）。
fn actual_teaching_starts(record: &WorkspaceSessionRecord) -> usize {
    record
        .provider_start_ledger
        .iter()
        .filter(|entry| {
            entry.provider_attempt_key.is_some() && entry.is_actual_launch_observation()
        })
        .count()
}

/// 6.1（Task 6 deferred ②）：claim 后崩溃重连、同请求重放 ⇒ 读取同一窗口
/// ⇒ `Replay`、实际启动数不增加、无新占位条目。物理启动的唯一耐久载体是
/// `provider_start_ledger`（claim 占位 ⇒ 驱动 ⇒ 回填），「占位不新增 +
/// 会话字节零变化」即跨进程物理启动数不增加的充分证据。
#[test]
fn replay_same_key_no_second_start() {
    let temp = tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let store = LifecycleStore::new(paths.clone());
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_t9_replay");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_granted(&store, &established);
    assert_eq!(teaching_placeholder_count(&granted), 1);
    assert_eq!(actual_teaching_starts(&granted), 0);
    let attempt2_key = format!("sc_generation:{}:0:attempt:2", granted.id);

    // 崩溃重连：跨进程模拟 = 同一 ProductAppPaths 上的全新 store 实例。
    let store2 = LifecycleStore::new(paths);
    let reloaded = store2
        .get_workspace_session(&granted.id)
        .expect("reload after crash");
    let bytes_before = std::fs::read_to_string(session_json_path(&store2, &reloaded))
        .expect("read session json before replay");

    // 同请求重放 ⇒ 读取同一窗口（Existing：窗口字段零变化，不重建）。
    match store2
        .establish_sc_teaching_window(&reloaded, &sc_teaching_request())
        .expect("replay establish")
    {
        ScTeachingEstablishOutcome::Existing {
            record,
            window_generation_seq,
        } => {
            assert_eq!(window_generation_seq, 0, "同一窗口（同 generation_seq）");
            assert_eq!(
                last_window(&record),
                last_window(&granted),
                "重放必须零变化读取同一耐久窗口"
            );
        }
        other => panic!("同请求重放必须 Existing，got: {other:?}"),
    }

    // claim 重放 ⇒ Replay（同 attempt_key 幂等），不追加、不新增占位。
    let replayed = match store2
        .claim_sc_teaching_attempt(&reloaded, 0)
        .expect("replay claim")
    {
        ScTeachingClaimOutcome::Replay(record) => record,
        other => panic!("重放 claim 必须 Replay，got: {other:?}"),
    };
    let window = last_window(&replayed);
    assert_eq!(window.attempts.len(), 2, "无新 attempt");
    let attempt2 = window.attempts.last().expect("attempt 2");
    assert_eq!(attempt2.attempt_key, attempt2_key);
    assert_eq!(attempt2.lifecycle, ScTeachingAttemptLifecycle::Claimed);
    assert_eq!(teaching_placeholder_count(&replayed), 1, "无新占位条目");
    assert_eq!(
        actual_teaching_starts(&replayed),
        0,
        "实际启动数不增加（跨进程重放零新物理启动）"
    );
    // 重放链路零写盘：Existing + Replay 均不落盘（会话字节原样）。
    let bytes_after = std::fs::read_to_string(session_json_path(&store2, &replayed))
        .expect("read session json after replay");
    assert_eq!(bytes_after, bytes_before, "重放不得写盘");
    // Task 6 deferred ①：异常退出后窗口保持 Active 待重放（不自动收口）。
    assert_eq!(window.status, ScTeachingWindowStatus::Active);
}

/// 6.1：spawn 前崩溃（占位 `started=false` 且无 provider ⇒ 可证未启动）⇒
/// 在原 key/原授权下继续同一 claim：attempt 仍 Claimed、不新增 attempt、
/// 不加预算；重放后同一 claim 仍可完成回填（合法续作）。
#[test]
fn crash_before_spawn_known_not_started_continues_same_claim() {
    let temp = tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let store = LifecycleStore::new(paths.clone());
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_t9_before");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_granted(&store, &established);
    let attempt2_key = format!("sc_generation:{}:0:attempt:2", granted.id);

    // 「可证未启动」前提：占位 started=false 且 provider/started_at 缺失。
    let placeholder = teaching_placeholder_entry(&granted, &attempt2_key);
    assert!(
        !placeholder.started && placeholder.provider.is_none() && placeholder.started_at.is_none()
    );
    assert!(!placeholder.is_actual_launch_observation());

    // 崩溃重连 + 同请求重放 ⇒ Existing；claim ⇒ Replay（同一 claim 继续）。
    let store2 = LifecycleStore::new(paths);
    let reloaded = store2
        .get_workspace_session(&granted.id)
        .expect("reload after crash");
    let bytes_before = std::fs::read_to_string(session_json_path(&store2, &reloaded))
        .expect("read session json before replay");
    match store2
        .establish_sc_teaching_window(&reloaded, &sc_teaching_request())
        .expect("replay establish")
    {
        ScTeachingEstablishOutcome::Existing { record, .. } => {
            assert_eq!(last_window(&record), last_window(&granted));
        }
        other => panic!("重放必须 Existing，got: {other:?}"),
    }
    let replayed = match store2
        .claim_sc_teaching_attempt(&reloaded, 0)
        .expect("replay claim")
    {
        ScTeachingClaimOutcome::Replay(record) => record,
        other => panic!("可证未启动的重放必须 Replay 同一 claim，got: {other:?}"),
    };
    let window = last_window(&replayed);
    assert_eq!(window.attempts.len(), 2, "不新增 attempt");
    let attempt2 = window.attempts.last().expect("attempt 2");
    assert_eq!(attempt2.attempt_key, attempt2_key, "原 key");
    assert_eq!(
        attempt2.lifecycle,
        ScTeachingAttemptLifecycle::Claimed,
        "attempt 仍 Claimed（原授权下继续同一 claim，不重新授予）"
    );
    assert_eq!(window.max_attempts, 2, "不加预算（max 不变）");
    assert_eq!(teaching_placeholder_count(&replayed), 1, "无新占位");
    let bytes_after = std::fs::read_to_string(session_json_path(&store2, &replayed))
        .expect("read session json after replay");
    assert_eq!(bytes_after, bytes_before, "重放不得写盘");

    // 继续合法动作：同一 claim 上完成回填（原位补全，无第二条）。
    let observed = store2
        .observe_sc_teaching_provider_start(
            &replayed,
            0,
            2,
            "codex",
            "2026-11-13T03:00:00Z".to_string(),
        )
        .expect("continue the same claim after replay");
    assert_eq!(
        last_window(&observed)
            .attempts
            .last()
            .expect("attempt 2")
            .lifecycle,
        ScTeachingAttemptLifecycle::ActualStarted
    );
    assert_eq!(
        teaching_placeholder_count(&observed),
        1,
        "原位补全，无第二条"
    );
    assert_eq!(actual_teaching_starts(&observed), 1);
}

/// 6.1：回填后崩溃 ⇒ 重放不重播（`ActualStarted` 幂等、无第二条启动）。
#[test]
fn crash_after_spawn_actual_started_not_replayed() {
    let temp = tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let store = LifecycleStore::new(paths.clone());
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_t9_after");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_granted(&store, &established);
    let observed = observe_started(&store, &granted);
    assert_eq!(
        actual_teaching_starts(&observed),
        1,
        "前提：已恰一次实际启动"
    );
    let attempt2_key = format!("sc_generation:{}:0:attempt:2", observed.id);

    // 崩溃重连：同请求重放 ⇒ Existing（同一窗口，不重建）。
    let store2 = LifecycleStore::new(paths);
    let reloaded = store2
        .get_workspace_session(&observed.id)
        .expect("reload after crash");
    let bytes_before = std::fs::read_to_string(session_json_path(&store2, &reloaded))
        .expect("read session json before replay");
    match store2
        .establish_sc_teaching_window(&reloaded, &sc_teaching_request())
        .expect("replay establish")
    {
        ScTeachingEstablishOutcome::Existing { record, .. } => {
            assert_eq!(last_window(&record), last_window(&observed));
        }
        other => panic!("重放必须 Existing，got: {other:?}"),
    }

    // ActualStarted 幂等：同证据再 observe ⇒ 原样返回（不写盘）。
    let replay_observe = store2
        .observe_sc_teaching_provider_start(
            &reloaded,
            0,
            2,
            "codex",
            "2026-11-13T02:00:00Z".to_string(),
        )
        .expect("idempotent re-observe with the same evidence");
    assert_eq!(replay_observe, observed, "幂等返回整记录相等");
    // 不同证据 ⇒ Conflict（同次启动不得双记录）。
    assert!(matches!(
        store2.observe_sc_teaching_provider_start(
            &reloaded,
            0,
            2,
            "codex",
            "2026-11-13T09:00:00Z".to_string(),
        ),
        Err(ProductStoreError::Conflict { .. })
    ));

    // claim ⇒ Denied(Exhausted)：重放不重播、不铸造第二次启动。
    assert_eq!(
        store2
            .claim_sc_teaching_attempt(&reloaded, 0)
            .expect("replay claim"),
        ScTeachingClaimOutcome::Denied(ScTeachingClaimRejection::Exhausted {
            max_attempts: 2,
            consumed: 2,
        })
    );
    let window = last_window(&reloaded);
    assert_eq!(window.attempts.len(), 2, "无第三 attempt");
    assert_eq!(
        actual_teaching_starts(&reloaded),
        1,
        "无第二条启动记录（恰一次实际启动）"
    );
    assert_eq!(
        teaching_placeholder_entry(&reloaded, &attempt2_key)
            .provider
            .as_deref(),
        Some("codex"),
        "占位原位补全未被重放改写"
    );
    let bytes_after = std::fs::read_to_string(session_json_path(&store2, &reloaded))
        .expect("read session json after replay");
    assert_eq!(bytes_after, bytes_before, "幂等重放不得写盘");
}

/// 6.1：`LaunchUnknown` ⇒ 沿原等待、零新启动（不自动重放、不自动补全）。
#[test]
fn unknown_waits_no_auto_replay() {
    let temp = tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let store = LifecycleStore::new(paths.clone());
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_t9_unknown");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_granted(&store, &established);
    let unknown = store
        .mark_sc_teaching_attempt_unknown(&granted, 0, 2)
        .expect("mark claimed attempt unknown");
    let attempt2_key = format!("sc_generation:{}:0:attempt:2", unknown.id);

    // 崩溃重连：同请求重放 ⇒ Existing（Unknown 沿原等待，窗口不重建）。
    let store2 = LifecycleStore::new(paths);
    let reloaded = store2
        .get_workspace_session(&unknown.id)
        .expect("reload after crash");
    let bytes_before = std::fs::read_to_string(session_json_path(&store2, &reloaded))
        .expect("read session json before replay");
    match store2
        .establish_sc_teaching_window(&reloaded, &sc_teaching_request())
        .expect("replay establish")
    {
        ScTeachingEstablishOutcome::Existing { record, .. } => {
            assert_eq!(last_window(&record), last_window(&unknown));
        }
        other => panic!("Unknown 重放必须 Existing，got: {other:?}"),
    }
    let replayed = match store2
        .claim_sc_teaching_attempt(&reloaded, 0)
        .expect("replay claim")
    {
        ScTeachingClaimOutcome::Replay(record) => record,
        other => panic!("Unknown 重放 claim 必须 Replay（等待语义），got: {other:?}"),
    };
    let window = last_window(&replayed);
    assert_eq!(window.attempts.len(), 2, "零新启动：不追加 attempt");
    let attempt2 = window.attempts.last().expect("attempt 2");
    assert_eq!(attempt2.attempt_key, attempt2_key);
    assert_eq!(
        attempt2.lifecycle,
        ScTeachingAttemptLifecycle::LaunchUnknown,
        "沿原等待（仍 LaunchUnknown，不自动迁移）"
    );
    assert_eq!(window.max_attempts, 2, "预算不变");
    // Unknown 不自动补全占位、不自动重放。
    let placeholder = teaching_placeholder_entry(&replayed, &attempt2_key);
    assert!(
        !placeholder.started && placeholder.provider.is_none(),
        "Unknown 等待不得自动补全占位"
    );
    assert_eq!(actual_teaching_starts(&replayed), 0, "零新启动");
    assert_eq!(
        window.status,
        ScTeachingWindowStatus::Active,
        "待重放不关窗"
    );
    let bytes_after = std::fs::read_to_string(session_json_path(&store2, &replayed))
        .expect("read session json after replay");
    assert_eq!(bytes_after, bytes_before, "等待语义零写盘");
}

/// 8.3：重放时 source/context/prompt digest 任一漂移 ⇒ 实际启动 0、预算
/// 与 refs 不变、`sc_teaching_frozen_input_drift` 诊断（handler 消费面：
/// `consume_sc_teaching_window` 在 claim **之前**核验冻结输入，漂移 ⇒
/// 零额外调用；本用例钉住其 store 侧前提与留痕）。
#[test]
fn source_context_prompt_drift_zero_start() {
    let temp = tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let store = LifecycleStore::new(paths.clone());
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_t9_drift");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let frozen_window = last_window(&established).clone();
    let request = sc_teaching_request();

    // 三类漂移变体：source（失败原文摘要）/ context（上下文摘要）/ prompt
    //（prompt digest 独立于 frozen_input_digest 单独核验）。
    let source_drift = ScTeachingWindowRequest {
        failure_source_sha256: digest64("drift-source"),
        ..request.clone()
    };
    let context_drift = ScTeachingWindowRequest {
        context_digest: digest64("drift-context"),
        ..request.clone()
    };
    let prompt_drift = ScTeachingWindowRequest {
        prompt_digest: digest64("drift-prompt"),
        ..request.clone()
    };
    for (name, drifted) in [
        ("source", &source_drift),
        ("context", &context_drift),
        ("prompt", &prompt_drift),
    ] {
        assert!(
            frozen_window.verify_frozen_inputs(drifted).is_err(),
            "{name} digest 漂移必须 fail-closed"
        );
    }

    // 崩溃重连后重放：漂移请求 ⇒ 读取同一窗口（Existing，冻结事实不被改写）。
    let store2 = LifecycleStore::new(paths);
    let reloaded = store2
        .get_workspace_session(&established.id)
        .expect("reload after crash");
    for drifted in [&source_drift, &context_drift, &prompt_drift] {
        match store2
            .establish_sc_teaching_window(&reloaded, drifted)
            .expect("drift replay establish")
        {
            ScTeachingEstablishOutcome::Existing { record, .. } => {
                assert_eq!(
                    last_window(&record),
                    &frozen_window,
                    "漂移重放不得改写窗口冻结事实"
                );
            }
            other => panic!("漂移重放必须 Existing，got: {other:?}"),
        }
    }

    // 漂移留痕诊断（handler 面调用的同一 store API）。
    let drifted_record = store2
        .record_sc_teaching_frozen_input_drift(&reloaded)
        .expect("record frozen input drift diagnostic");
    assert!(
        drifted_record
            .policy_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "sc_teaching_frozen_input_drift"),
        "漂移必须留痕 durable 诊断 sc_teaching_frozen_input_drift"
    );
    // 实际启动 0：漂移 ⇒ 不 claim（无占位、无回填）。
    assert_eq!(teaching_placeholder_count(&drifted_record), 0);
    assert_eq!(actual_teaching_starts(&drifted_record), 0);
    // 预算与 refs 不变。
    assert_eq!(
        drifted_record.run_history, established.run_history,
        "预算计数不被漂移改写"
    );
    assert_eq!(
        drifted_record.work_item_plan_source_revision_ref,
        established.work_item_plan_source_revision_ref
    );
    assert_eq!(
        drifted_record.plan_candidate_ir_ref, established.plan_candidate_ir_ref,
        "refs 不变"
    );
    // 窗口不被漂移请求消费（Active、仅 attempt1，机会未被扣除）。
    let window = last_window(&drifted_record);
    assert_eq!(window.attempts.len(), 1);
    assert_eq!(window.status, ScTeachingWindowStatus::Active);
    assert_eq!(window.claimable_attempt_no(), Some(2), "机会未被漂移消费");
}
