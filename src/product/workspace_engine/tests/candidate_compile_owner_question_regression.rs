//! 工作包 6.1–6.4 核验（REQ-CG-08/09 已交付面）：环失败 owner 问句的
//! 「一次答完」与「同一未变事实不重复提问」回归钉子。
//!
//! 钉的链路全部复用既有已交付面，不新造机制：
//! - 问句与选项由 typed witness 生成（`auto_rewrite_verdict`，REQ-CG-09 /
//!   REQ-WG-05；6.1/6.2 的既有钉子在 `compile_failure_surface.rs` 内测）；
//! - owner 处置走 durable 已处置清单（`record_human_dispositions`，
//!   REQ-WSC-13；写入面先例为 `conversational_gate.rs` 的
//!   `record_owner_finding_exclusions`）；
//! - 下一轮同一未变事实（同环结构指纹 ⇒ 同 finding 身份）经会话级平台裁决
//!   （`apply_session_owner_dispositions`）不再作为 owner 决定面重复提问。
//!
//! 6.4 收窄口径：`HumanFindingDispositionKind` 现只有 `Excluded`，本 change
//! 只承诺「同一未变事实不重复提问」，不承诺「持久化选项内容」——
//! `Decided{option_id}` 变体不新增（无写入点的变体是死代码）。

use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::mpsc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
use crate::product::models::{
    CandidateCompileFailure, HumanDispositionRecord, HumanFindingDispositionKind, ProviderName,
    WorkspaceType,
};
use crate::product::workspace_engine::compile_failure_surface::{
    CYCLE_FINGERPRINT_MARKER, auto_rewrite_verdict,
};
use crate::product::workspace_engine::{WorkspaceEngine, WorkspaceSession};
use crate::web::workspace_ws_types::{ReviewGate, ReviewVerdictType};

/// 与 `review_disposition_platform.rs` 的 `platform_fixture` 同法：真实
/// LifecycleStore + workspace session + engine，处置走真实 durable 落盘。
struct OwnerQuestionFixture {
    tmp: TempDir,
    store: LifecycleStore,
    record: crate::product::models::WorkspaceSessionRecord,
}

fn owner_question_fixture() -> OwnerQuestionFixture {
    let tmp = TempDir::new().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let record = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_owner_question_regression".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
        })
        .expect("create workspace session");
    OwnerQuestionFixture { tmp, store, record }
}

impl OwnerQuestionFixture {
    fn engine(&self) -> WorkspaceEngine {
        let (event_tx, _event_rx) = mpsc::channel(8);
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(self.tmp.path().to_path_buf())),
            self.store.clone(),
            event_tx,
            WorkspaceSession::from_record(self.record.clone()),
        )
    }
}

/// 含环 owner_question 的失败面（同 `compile_failure_surface.rs` 既有用例
/// `auto_rewrite_verdict_carries_owner_question_and_options_from_witness` 的
/// witness 完整模板；结构指纹由 detail 的指纹行携带）。
fn cycle_compile_failure() -> CandidateCompileFailure {
    CandidateCompileFailure {
        failure_id: "planning_compile_failure_regression".to_string(),
        code: "final_compile_prepare_failed".to_string(),
        message: "「公告列表行删除交互与中文确认」 → 「删除链路端到端验收」 → 「公告列表行删除交互与中文确认」 构成依赖环。其中「删除链路端到端验收」把「公告列表行删除交互与中文确认」写成 provider，但「公告列表行删除交互与中文确认」所在仓是 change_order 第 3 个仓、「删除链路端到端验收」所在仓是第 2 个仓——跨仓依赖只允许后仓消费前仓。"
            .to_string(),
        detail: format!("failure_id: x\ncode: y{CYCLE_FINGERPRINT_MARKER}{}", "9".repeat(64)),
        recorded_at: "2026-10-31T00:00:00Z".to_string(),
    }
}

/// 工作包 6.3 回归钉子：同一未变事实（同环结构指纹 ⇒ 同 finding 身份）经
/// owner 处置后，下一轮失败面不再把该事实作为 owner 决定面（actionable
/// `owner_question`）重复提问——平台按已处置清单强制裁决，不依赖模型标对。
#[test]
fn answered_fact_not_reasked_as_owner_question() {
    let fixture = owner_question_fixture();

    // 第一轮：witness 完整 ⇒ 失败面携带 owner 问句与至少两个带后果选项。
    let failure = cycle_compile_failure();
    let first = auto_rewrite_verdict(&failure);
    assert_eq!(first.verdict, ReviewVerdictType::Revise);
    assert_eq!(first.review_gate, ReviewGate::RequiresRevision);
    let first_finding = first.findings.first().expect("failure surface finding");
    let question = first_finding
        .owner_question
        .as_deref()
        .expect("前置：环 witness 完整时失败面携带 owner 问句（REQ-CG-09）");
    assert!(
        first_finding
            .owner_options
            .as_ref()
            .is_some_and(|options| options.len() >= 2),
        "前置：问句带至少两个互斥选项（REQ-WG-05）"
    );
    let key = first_finding.computed_finding_key();
    assert!(
        HumanDispositionRecord::is_valid_finding_key(&key),
        "REQ-WSC-13 形态：finding 身份必须是 16 位小写 hex（实际 {key}）"
    );

    // owner 处置写入 durable 已处置清单（复用既有通路，不另造平行机制）。
    fixture
        .store
        .record_human_dispositions(
            &fixture.record.id,
            &[HumanDispositionRecord {
                finding_key: key.clone(),
                message_summary: question.to_string(),
                disposition: HumanFindingDispositionKind::Excluded,
                recorded_at: "2026-11-12T00:00:00Z".to_string(),
            }],
        )
        .expect("record human disposition");

    // 下一轮失败面：事实未变（同环、同结构指纹），只是新一次编译尝试。
    let mut next_failure = failure.clone();
    next_failure.failure_id = "planning_compile_failure_regression_next".to_string();
    next_failure.recorded_at = "2026-11-12T00:01:00Z".to_string();
    let next = auto_rewrite_verdict(&next_failure);
    let next_finding = next.findings.first().expect("next round finding");
    assert_eq!(
        next_finding.computed_finding_key(),
        key,
        "同环同指纹 ⇒ 同一 finding 身份（REQ-WSC-22）：处置才能跨轮对上同一事实"
    );
    assert_eq!(
        next_finding.owner_question.as_deref(),
        Some(question),
        "事实未变 ⇒ 下一轮重渲染出同一问句（这正是「不得重复提问」的场景前提）"
    );

    // 会话级平台裁决（读 durable 清单）：该未变事实已处置 ⇒ 不再作为
    // owner 决定面出现——问句不再挂在任何 actionable finding 上。
    let engine = fixture.engine();
    let normalized = engine.apply_session_owner_dispositions(next);
    let disposed = normalized
        .findings
        .iter()
        .find(|finding| finding.computed_finding_key() == key)
        .expect("已处置 finding 仍在失败面上，但已被平台裁决降级");
    assert!(
        !disposed.is_actionable(),
        "已处置事实被平台强制按 advisory 处理（owner 事实优先，不依赖模型标对）"
    );
    assert!(
        normalized
            .findings
            .iter()
            .filter(|finding| finding.is_actionable())
            .all(|finding| finding.owner_question.as_deref() != Some(question)),
        "同一未变事实不再作为 actionable owner_question 重复提问"
    );
    assert_eq!(
        normalized.verdict,
        ReviewVerdictType::Pass,
        "revise 无 actionable ⇒ 归一回 pass：owner 决定面不再为该事实打开"
    );
    assert_eq!(
        normalized.review_gate,
        ReviewGate::UserConfirmAllowed,
        "门不再因该已处置事实要求人工介入"
    );
}
