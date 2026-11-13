//! SC 教学窗口 claim 拒绝分支的钉名补漏（REQ-WSC-28C store 面，序 3 终审
//! Minor-1 收口）：四类 claim 拒绝中 NoWindow / NotApproved / Exhausted 均有
//! 钉名用例，唯 `WindowClosed` 此前只被引擎面用例
//! `observe_on_closed_window_is_refused_not_silently_completed` 间接经过
//! （claim 判序在 observe 之前）。本文件补直接 claim 的钉名覆盖：已关窗直
//! claim ⇒ `Denied(WindowClosed)` 且 durable 字节零变化。夹具体例沿用
//! `workspace_sc_teaching_window_tests.rs`；挂载方式照
//! `workspace_sc_teaching_fault_matrix_tests.rs` 先例（#[path] 挂到 store
//! 文件的 mod 区）。

use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, ScTeachingClaimOutcome, ScTeachingClaimRejection,
    ScTeachingEstablishOutcome, WorkItemPlanSessionOptions,
};
use crate::product::models::{ProviderName, WorkspaceSessionRecord, WorkspaceType};
use crate::product::planning_failure::PlanningFailureKind;
use crate::product::work_item_plan_policy::{
    RunPolicy, ScTeachingAttemptOutcome, ScTeachingWindowRequest, ScTeachingWindowStatus,
    WorkItemPlanFlowKind,
};
use tempfile::tempdir;

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
        other => panic!("first claim must grant attempt 2, got {other:?}"),
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

/// 终审 Minor-1：已关窗（`Closed{Exhausted}`）后直接 claim ⇒ 可判定拒绝
/// `WindowClosed`，且 durable 记录字节零变化（拒绝不落盘）。防退化点：若
/// `WindowClosed` 分支被删除或后移，末 attempt 已 `Terminal{FailedTerminal}`
/// 且 attempts 已达 max=2 ⇒ 拒绝原因会串到 `Exhausted{max:2, consumed:2}`
/// 臂——本用例钉死两者的判别与「零写盘」两侧。
#[test]
fn claim_on_closed_window_is_denied_without_writing() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0021");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_granted(&store, &established);
    let observed = observe_started(&store, &granted);
    // 第二次教学重驱终局失败 ⇒ close(FailedTerminal) ⇒ 窗口 Closed{Exhausted}。
    let closed = store
        .close_sc_teaching_window(&observed, 0, 2, ScTeachingAttemptOutcome::FailedTerminal)
        .expect("close teaching window");
    assert!(matches!(
        closed.sc_generation_teaching_windows[0].status,
        ScTeachingWindowStatus::Closed { .. }
    ));
    // 拒绝前先取 durable 字节基准。
    let path = session_json_path(&store, &closed);
    let before = std::fs::read(&path).expect("read durable session bytes");
    // 已关窗直接 claim ⇒ Denied(WindowClosed)（而非 Exhausted）。
    let outcome = store
        .claim_sc_teaching_attempt(&closed, 0)
        .expect("claim on closed window resolves determinably");
    assert_eq!(
        outcome,
        ScTeachingClaimOutcome::Denied(ScTeachingClaimRejection::WindowClosed)
    );
    // 复拒仍可判定且同因（幂等拒绝）。
    let again = store
        .claim_sc_teaching_attempt(&closed, 0)
        .expect("repeat claim on closed window still resolves determinably");
    assert_eq!(
        again,
        ScTeachingClaimOutcome::Denied(ScTeachingClaimRejection::WindowClosed)
    );
    // 拒绝不写盘：claim 前后 durable 记录字节相等、重载整记录相等。
    let after = std::fs::read(&path).expect("re-read durable session bytes");
    assert_eq!(
        after, before,
        "a WindowClosed denial must not persist anything"
    );
    let reloaded = store.get_workspace_session(&closed.id).expect("reload");
    assert_eq!(reloaded, closed);
}
