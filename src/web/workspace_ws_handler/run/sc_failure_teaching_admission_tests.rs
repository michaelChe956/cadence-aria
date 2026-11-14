//! P08 教学重驱入口红测：A（native fresh）+ B（准入闸）。
//!
//! A：即便存在旧 Author provider 会话，教学重驱的输入也必须
//! `resume_provider_session_id == None`（全新 native；以 `--resume` 复用旧会话会
//! 触发网关 `provider_gateway_resume_not_supported`）。
//!
//! B：入口准入闸——上一轮未严格 completed / raw 与引用不同代 ⇒ 不启动（Waiting，
//! 留可读原因）；审计分区按 `role_run_seq` 非空 ⇒ 重放不得二次启动；同 key 两次
//! 入口复用同一物理轮（CAS 单胜者），不新分配 seq、不新增证据行。
//!
//! fixture/启动计数复用父模块 `tests` 的既有 helper（`sc_round_fixture` /
//! `actual_starts`），不复制第二份；真实 `provider_start` 经 `LifecycleStore` 的
//! `ToolPolicyAuditSink::append`（与 adapter 的 run-bound sink 同一准入）。

use super::tests::{actual_starts, sc_round_fixture};
use super::*;

use crate::product::workspace_engine::PlanAuthorOutputContract;

const NODE_ID: &str = "node_sc_0001";
const BUSINESS_KEY: &str = "workspace_author:session:0";

/// 原轮首绑（从未收口）：`Ready` 且 run 状态为 `running`。
fn bind_original(lifecycle: &LifecycleStore, scope: &ScFailureScope) -> ScBoundProviderRound {
    match begin_sc_failure_round(
        lifecycle,
        scope,
        BUSINESS_KEY,
        None,
        NODE_ID,
        &ProviderName::Fake,
        0,
    )
    .expect("original round binds")
    {
        ScRoundEntry::Ready(bound) => bound,
        _ => panic!("fresh original round must classify Ready"),
    }
}

/// 真实 `provider_start` 审计行（同一 `role_run_seq` 分区）。
fn audited_start(lifecycle: &LifecycleStore, session_id: &str, role_run_seq: u64) {
    use crate::cross_cutting::tool_policy_audit::{
        DurableToolPolicyEvent, ProviderStartAudit, ToolPolicyAuditSink,
    };
    lifecycle
        .append(
            session_id,
            role_run_seq,
            DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: "fake".to_string(),
                ..ProviderStartAudit::default()
            }),
        )
        .expect("real provider start audit line");
}

/// 复用 SC fixture 的会话，构建带「旧 Author 会话」的 engine。
fn sc_round_engine() -> (
    tempfile::TempDir,
    LifecycleStore,
    crate::product::workspace_engine::WorkspaceEngine,
) {
    let (tmp, lifecycle, scope) = sc_round_fixture();
    let record = lifecycle
        .get_workspace_session(&scope.session_id)
        .expect("reload fixture session");
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(8);
    std::mem::forget(event_rx);
    let mut engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            tmp.path().join("checkpoints"),
        )),
        lifecycle.clone(),
        event_tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(record),
    );
    engine.session.provider_conversations = vec![crate::product::models::ProviderConversationRef {
        role: crate::product::models::ProviderConversationRole::Author,
        provider: ProviderName::Fake,
        provider_session_id: "author-session-1".to_string(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        last_node_id: Some("node-1".to_string()),
    }];
    (tmp, lifecycle, engine)
}

// A：教学重驱输入全新 native（旧 Author 会话在场也不 resume）。
#[test]
fn teaching_reredrive_input_never_resumes_provider_session() {
    let (_tmp, _lifecycle, engine) = sc_round_engine();
    let input = sc_reredrive_streaming_input(
        &engine,
        &ProviderName::Fake,
        "teaching prompt",
        std::path::Path::new("/tmp/worktree"),
    )
    .expect("fresh input builds");
    assert_eq!(
        input.resume_provider_session_id, None,
        "教学重驱必须全新 native：不得携带 provider resume 会话 id"
    );

    // 对照：legacy builder 仍会 resume 旧 Author 会话——证明 fixture 真的暴露了
    // bug 形态，被改的是教学入口决策（fresh），不是 builder 本身。
    let resumed = engine
        .build_work_item_plan_streaming_input(
            crate::product::work_item_split_engine::types::provider_name_to_type(
                &ProviderName::Fake,
            ),
            "teaching prompt".to_string(),
            "/tmp/worktree".to_string(),
            ProviderName::Fake,
            PlanAuthorOutputContract::Structured,
        )
        .expect("legacy input builds");
    assert_eq!(
        resumed.resume_provider_session_id,
        Some("author-session-1".to_string()),
        "对照：legacy builder 仍会 resume（教学入口已改为 fresh）"
    );
}

// B1：上一轮未严格 completed ⇒ 不启动（Waiting），留可读原因、零 start。
#[test]
fn teaching_entry_refuses_when_previous_round_not_completed() {
    let (_tmp, lifecycle, scope) = sc_round_fixture();
    let original = bind_original(&lifecycle, &scope);
    // 上一轮只绑定、从未收口（status 仍为 `running`）。伪造一个 `Available` 引用
    // 触发 B1（真实 `Available` 只在 completed 收口后才产生）。
    let forged_previous = (
        ScEvidenceRef::Available {
            locator: String::new(),
            sha256: "deadbeef".to_string(),
            bytes: 8,
        },
        original.round.clone(),
    );
    match begin_sc_teaching_round(
        &lifecycle,
        &original,
        Some(&forged_previous),
        NODE_ID,
        &ProviderName::Fake,
    )
    .expect("teaching entry classifies")
    {
        ScRoundEntry::Waiting {
            reason_code,
            detail,
            ..
        } => {
            assert_eq!(
                reason_code,
                super::super::sc_failure_evidence::SC_TEACHING_PREVIOUS_ROUND_NOT_COMPLETED
            );
            assert!(!detail.is_empty(), "must leave a readable reason");
        }
        _ => panic!("non-completed previous round must refuse to start (Waiting)"),
    }
    assert_eq!(
        actual_starts(&lifecycle, &scope.session_id),
        0,
        "refused entry starts nothing"
    );
}

// B5 + CAS 单胜者：同 key 两次入口只产生一个物理轮；审计分区非空 ⇒ 不得二次启动。
#[test]
fn teaching_entry_same_key_single_physical_round_and_no_second_start() {
    let (_tmp, lifecycle, scope) = sc_round_fixture();
    let original = bind_original(&lifecycle, &scope);
    let previous = (
        ScEvidenceRef::Missing {
            code: "sc_failure_raw_missing".to_string(),
            detail: "not yet delivered".to_string(),
            round_id: original.round.round_id.clone(),
        },
        original.round.clone(),
    );

    let teaching = match begin_sc_teaching_round(
        &lifecycle,
        &original,
        Some(&previous),
        NODE_ID,
        &ProviderName::Fake,
    )
    .expect("first teaching entry")
    {
        ScRoundEntry::Ready(bound) => bound,
        _ => panic!("un-started teaching round must classify Ready"),
    };

    // 同 key 第二次入口（尚未启动）：复用同一物理轮（既有 CAS），不新分配 seq。
    let replay = match begin_sc_teaching_round(
        &lifecycle,
        &original,
        Some(&previous),
        NODE_ID,
        &ProviderName::Fake,
    )
    .expect("second teaching entry")
    {
        ScRoundEntry::Ready(bound) => bound,
        _ => panic!("same key must resolve the same physical round"),
    };
    assert_eq!(replay.round.round_id, teaching.round.round_id);
    assert_eq!(replay.round.role_run_seq, teaching.round.role_run_seq);

    let rows_after_bind = lifecycle
        .read_sc_failure_evidence(&original.chain)
        .expect("evidence readable")
        .entries
        .len();

    // 模拟真实启动：该 seq 追加唯一一条 provider_start 审计。
    audited_start(&lifecycle, &scope.session_id, teaching.round.role_run_seq);

    // 再入同一 key：审计分区已非空 ⇒ 不得二次启动（Waiting，零 Err/Failed）。
    match begin_sc_teaching_round(
        &lifecycle,
        &original,
        Some(&previous),
        NODE_ID,
        &ProviderName::Fake,
    )
    .expect("replay after start classifies")
    {
        ScRoundEntry::Waiting {
            reason_code, round, ..
        } => {
            assert_eq!(reason_code, SC_ROUND_LAUNCH_UNKNOWN);
            assert_eq!(round.role_run_seq, teaching.round.role_run_seq);
        }
        _ => panic!("non-empty audit partition must refuse a second start"),
    }

    let audit = lifecycle
        .read_tool_policy_lines_with_warnings(&scope.session_id, teaching.round.role_run_seq)
        .expect("audit partition readable");
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|line| line.event_type() == "provider_start")
            .count(),
        1,
        "exactly one provider_start; replay appended no second start"
    );
    assert_eq!(
        lifecycle
            .read_sc_failure_evidence(&original.chain)
            .expect("evidence readable")
            .entries
            .len(),
        rows_after_bind,
        "replay adds no evidence row"
    );
    assert_eq!(
        actual_starts(&lifecycle, &scope.session_id),
        0,
        "no durable provider start ledger observation on this path"
    );
}
