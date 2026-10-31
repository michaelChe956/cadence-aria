//! REQ-WSC-18/19（owner 2026-10-31 真机反馈）：
//! - 编译失败的**完整**诊断必须 durable 落盘（不截断）；
//! - 门卡 `session_state.candidate_compile_failure` 必须随刷新可读；
//! - 当前候选编译不过时「确认当前版本」必须平台侧 fail-closed（不得死路）；
//! - 重写要点必须同时进入给 author 的门内修订提示词。

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use crate::product::models::{
    CandidateCompileFailure, CandidateCompileStatus, ProviderName, SingleCandidatePhase,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::{
    HumanReason, RunHistory, RunPolicy, WorkItemPlanFlowKind,
};
use tempfile::TempDir;

const SOURCE_REF: &str = "source-current-candidate";
const IR_REF: &str = "ir-current-candidate";

fn failure_record(detail: &str) -> CandidateCompileFailure {
    // witness 不完整形态：不是 producer 的 offending 模板（无位次句）。
    CandidateCompileFailure {
        failure_id: "planning_compile_failure_gate".to_string(),
        code: "final_compile_prepare_failed".to_string(),
        message:
            "「gateway 删除转发」把「frontend 行删除入口」写成 provider：依赖逆了仓序，构成依赖环"
                .to_string(),
        detail: detail.to_string(),
        recorded_at: "2026-10-31T00:00:00Z".to_string(),
    }
}

/// 现场 2adc5e41 形态：producer `cycle_diagnostic::cycle_reason` offending
/// 分支的完整模板（含位次句），witness 可解析。
fn witness_failure_record(detail: &str) -> CandidateCompileFailure {
    CandidateCompileFailure {
        failure_id: "planning_compile_failure_gate".to_string(),
        code: "final_compile_prepare_failed".to_string(),
        message: "「公告列表行删除交互与中文确认」 → 「删除链路端到端验收」 → 「公告列表行删除交互与中文确认」 构成依赖环（箭头为产出流向：左边 Work Item 的产出被右边消费，即右边依赖左边）。其中「删除链路端到端验收」把「公告列表行删除交互与中文确认」写成 provider，但「公告列表行删除交互与中文确认」所在仓是 change_order 第 3 个仓、「删除链路端到端验收」所在仓是第 2 个仓——跨仓依赖只允许后仓 Work Item 消费前仓 Work Item 的产出，这条依赖逆了仓序，因此与仓序自动边合成环。请让「删除链路端到端验收」改由仓序在前的 Work Item 提供产出，或删掉这条 depends_on。"
            .to_string(),
        detail: detail.to_string(),
        recorded_at: "2026-10-31T00:00:00Z".to_string(),
    }
}

fn status_for(
    refs: (Option<&str>, Option<&str>),
    failure: Option<CandidateCompileFailure>,
) -> CandidateCompileStatus {
    CandidateCompileStatus {
        source_revision_ref: refs.0.map(str::to_string),
        plan_candidate_ir_ref: refs.1.map(str::to_string),
        failure: failure.clone(),
        cycle_fingerprint: None,
        repeat_count: failure.as_ref().map(|_| 1),
        auto_rewrites_used: u32::from(failure.is_some()),
        // REQ-WSC-21：首次失败即进入自动重写阶段（前端据此提示「已启动/还剩 N 次」）。
        auto_rewrite: failure.map(|_| crate::product::models::CandidateAutoRewrite {
            phase: crate::product::models::CandidateAutoRewritePhase::Running,
            remaining: Some(2),
            repeated: Some(1),
        }),
        recorded_at: "2026-10-31T00:00:00Z".to_string(),
    }
}

/// 门卡夹具：SC 会话停在 `human_confirm`，候选 refs 已知。
fn gate_fixture(status: CandidateCompileStatus) -> (TempDir, LifecycleStore, WorkspaceEngine) {
    let root = TempDir::new().expect("tempdir");
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .expect("create session");
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    record.work_item_plan_source_revision_ref = Some(SOURCE_REF.to_string());
    record.plan_candidate_ir_ref = Some(IR_REF.to_string());
    record.human_gate_snapshot = Some(crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 1,
        manual_repairs_remaining: 3,
        accepted_feedback_turns: Some(0),
        candidate_recovery: None,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
    });
    record.run_history = RunHistory::default();
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist gate fixture");
    lifecycle
        .save_candidate_compile_status(&record.id, &status)
        .expect("persist candidate compile status");
    let (event_tx, _event_rx) = mpsc::channel(32);
    let mut session = WorkspaceSession::from_record(record);
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Work Item Plan\n\n## Work Item WI-001: 候选\n".to_string(),
        diff: None,
    });
    let engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    );
    (root, lifecycle, engine)
}

/// 完整诊断原文（真实形态：远超 timeline summary 的可见长度）。
fn long_detail() -> String {
    let mut detail = String::from(
        "failure_id: planning_compile_failure_gate\ncode: final_compile_prepare_failed\ncompile_run_id: compile_0001\n",
    );
    for index in 0..80 {
        detail.push_str(&format!(
            "dependency_cycle 诊断第 {index} 段：work_item_compile_003 ↔ work_item_compile_004\n"
        ));
    }
    detail
}

#[tokio::test]
async fn session_state_projects_current_candidate_compile_failure_with_full_detail() {
    let detail = long_detail();
    let (_root, _lifecycle, engine) = gate_fixture(status_for(
        (Some(SOURCE_REF), Some(IR_REF)),
        Some(witness_failure_record(&detail)),
    ));

    match engine.build_session_state() {
        crate::web::workspace_ws_types::WsOutMessage::SessionState {
            candidate_compile_failure: Some(failure),
            ..
        } => {
            assert_eq!(failure.code, "final_compile_prepare_failed");
            assert!(
                failure.message.contains("构成依赖环"),
                "message 必须是完整人话：{}",
                failure.message
            );
            assert_eq!(
                failure.detail.as_deref(),
                Some(detail.as_str()),
                "detail 必须逐字完整（不截断）"
            );
            let hint = failure.owner_hint.expect("owner hint present");
            assert!(hint.contains("这个候选无效"), "{hint}");
            // REQ-WG-05（REMOVED REQ-WSC-20）：witness 完整 → 只讲算出的
            // 事实（两个位次 + 逆序那一跳）+ 两个修法各带后果；
            // **不得**再出现任何 design 归因断言。
            assert!(hint.contains("托管在仓序第 2 个仓"), "{hint}");
            assert!(hint.contains("第 3 个仓"), "{hint}");
            assert!(hint.contains("两个合法修法"), "{hint}");
            assert!(
                hint.contains(
                    "去掉「删除链路端到端验收」对「公告列表行删除交互与中文确认」的跨仓消费"
                ),
                "{hint}"
            );
            assert!(hint.contains("后果："), "{hint}");
            assert!(
                !hint.contains("design"),
                "REQ-WG-05：owner 面不得断言根因在 design：{hint}"
            );
            // 首次失败即告知平台会自动重写，避免 owner 以为要自己推。
            assert!(hint.contains("自动让 author 重写"), "{hint}");
            assert_eq!(
                failure.rewrite_guidance.len(),
                3,
                "witness 完整时给出位次事实 + 消环方向 + 保护项：{:?}",
                failure.rewrite_guidance
            );
            assert!(
                failure
                    .rewrite_guidance
                    .iter()
                    .all(|point| !point.contains("修改 design")),
                "重写要点不得包含 design 归因条目：{:?}",
                failure.rewrite_guidance
            );
            let auto_rewrite = failure.auto_rewrite.expect("auto_rewrite projected");
            assert_eq!(
                auto_rewrite.phase,
                crate::product::models::CandidateAutoRewritePhase::Running
            );
            assert_eq!(auto_rewrite.remaining, Some(2));
            assert_eq!(auto_rewrite.repeated, Some(1));
        }
        other => panic!("expected candidate_compile_failure in session_state, got {other:?}"),
    }
}

#[tokio::test]
async fn session_state_incomplete_witness_keeps_facts_only_without_attribution() {
    // REQ-WG-05：witness 不完整 → 只有稳定码与已知事实，无位次、无选项、
    // 无任何归因断言。
    let (_root, _lifecycle, engine) = gate_fixture(status_for(
        (Some(SOURCE_REF), Some(IR_REF)),
        Some(failure_record("witness-incomplete")),
    ));
    match engine.build_session_state() {
        crate::web::workspace_ws_types::WsOutMessage::SessionState {
            candidate_compile_failure: Some(failure),
            ..
        } => {
            let hint = failure.owner_hint.expect("owner hint present");
            assert!(hint.contains("环 witness 不完整"), "{hint}");
            assert!(!hint.contains("第 2 个仓"), "不得给位次：{hint}");
            assert!(!hint.contains("两个合法修法"), "不得给选项：{hint}");
            assert!(!hint.contains("design"), "不得归因：{hint}");
        }
        other => panic!("expected candidate_compile_failure in session_state, got {other:?}"),
    }
}

#[tokio::test]
async fn stale_candidate_compile_failure_is_not_projected() {
    // 失败属于旧候选（source ref 变了）：不得投影，也不得拦确认。
    let (_root, _lifecycle, engine) = gate_fixture(status_for(
        (Some("source-old-candidate"), Some(IR_REF)),
        Some(failure_record("old")),
    ));
    match engine.build_session_state() {
        crate::web::workspace_ws_types::WsOutMessage::SessionState {
            candidate_compile_failure,
            ..
        } => assert!(
            candidate_compile_failure.is_none(),
            "旧候选的失败不得出现在当前门卡上"
        ),
        other => panic!("expected session_state, got {other:?}"),
    }
    engine
        .ensure_current_candidate_compile_not_failed()
        .expect("旧候选失败不得拦确认（fail-open）");
}

#[tokio::test]
async fn approve_is_fail_closed_when_current_candidate_compile_failed() {
    let (_root, lifecycle, mut engine) = gate_fixture(status_for(
        (Some(SOURCE_REF), Some(IR_REF)),
        Some(failure_record("detail")),
    ));
    let session_id = engine.session().session_id.clone();
    let error = engine
        .close_human_gate(crate::product::workspace_engine::HumanGateCloseDecision::Approve)
        .await
        .expect_err("候选编译不过时确认必须 fail-closed");
    assert!(
        error.starts_with("CANDIDATE_COMPILE_FAILED"),
        "稳定码必须可判别：{error}"
    );
    assert!(error.contains("再次失败"), "必须说明为什么不能点：{error}");
    assert!(error.contains("下一步"), "必须给下一步：{error}");
    // 零 durable 副作用：门仍开（status 不变），无 compile 事务。
    let durable = lifecycle
        .get_workspace_session(&session_id)
        .expect("durable session");
    assert_eq!(durable.status, WorkspaceSessionStatus::WaitingForHuman);
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
}

#[tokio::test]
async fn revision_prompt_carries_rewrite_guidance_for_failed_candidate() {
    let (_root, _lifecycle, engine) = gate_fixture(status_for(
        (Some(SOURCE_REF), Some(IR_REF)),
        Some(witness_failure_record("detail")),
    ));
    let prompt = engine
        .build_sc_manual_revision_prompt_for_turn("请修正依赖方向")
        .expect("revision prompt");
    assert!(prompt.contains("[candidate_compile_failure]"), "{prompt}");
    // REQ-WG-06：教学块携带**算出的位次**与两个消环方向 + 保护项，不再是
    // 四条通用规则教条（REQ-WG-04：也不得要求 author 理解仓序规则本身）。
    assert!(
        prompt.contains("「删除链路端到端验收」当前托管在第 2 个仓"),
        "{prompt}"
    );
    assert!(prompt.contains("在第 3 个仓"), "{prompt}");
    assert!(prompt.contains("消环方向二选一"), "{prompt}");
    assert!(
        prompt.contains("保护项：不得为消环删除真实的业务数据流"),
        "{prompt}"
    );
    // design 只由 owner 决定，author 提示里**不得**要求它去改 design。
    assert!(
        !prompt.contains("必须同步修订 Design"),
        "author 提示不得要求改 design：{prompt}"
    );
    assert!(
        !prompt.contains("根因在 design"),
        "author 提示不得包含 design 归因：{prompt}"
    );
}

#[tokio::test]
async fn revision_prompt_incomplete_witness_stays_generic_without_attribution() {
    let (_root, _lifecycle, engine) = gate_fixture(status_for(
        (Some(SOURCE_REF), Some(IR_REF)),
        Some(failure_record("detail")),
    ));
    let prompt = engine
        .build_sc_manual_revision_prompt_for_turn("请修正依赖方向")
        .expect("revision prompt");
    assert!(prompt.contains("[candidate_compile_failure]"), "{prompt}");
    assert!(
        prompt.contains("必须消依赖环后重新输出完整候选"),
        "{prompt}"
    );
    assert!(
        !prompt.contains("当前托管在第"),
        "witness 不完整时不得给位次：{prompt}"
    );
    assert!(
        !prompt.contains("根因在 design"),
        "不得包含 design 归因：{prompt}"
    );
}

#[tokio::test]
async fn revision_prompt_is_unchanged_without_compile_failure() {
    let (_root, _lifecycle, engine) =
        gate_fixture(status_for((Some(SOURCE_REF), Some(IR_REF)), None));
    let prompt = engine
        .build_sc_manual_revision_prompt_for_turn("请修正依赖方向")
        .expect("revision prompt");
    assert!(
        !prompt.contains("[candidate_compile_failure]"),
        "无编译失败时既有 prompt 必须逐字不变"
    );
}

#[tokio::test]
async fn compile_failure_records_complete_durable_diagnostic() {
    // 现场链路复现：SC Final Compile 失败（无 source revision ref → 确定性
    // 失败）落门时，必须同时留下完整 durable 诊断。
    let (_root, lifecycle, mut engine) = super::conversational_gate::compile_failure_gate_fixture(
        WorkItemPlanFlowKind::SingleCandidate,
        RunPolicy::AutoIfValid,
    );
    engine.enter_policy_valid_work_item_plan_compile().await;
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    let session_id = engine.session().session_id.clone();
    let status = lifecycle
        .load_candidate_compile_status(&session_id)
        .expect("status readable")
        .expect("compile failure must be durably recorded");
    let failure = status.failure.expect("failure recorded");
    assert!(
        !failure.code.is_empty() && !failure.message.is_empty(),
        "code/message 必填：{failure:?}"
    );
    assert!(
        failure.detail.contains("failure_id: "),
        "detail 必须携完整诊断原文：{}",
        failure.detail
    );
    assert!(
        failure.detail.contains(&failure.code),
        "detail 必须含稳定码：{}",
        failure.detail
    );
    assert!(
        !failure.message.contains("failure_id"),
        "message 是人话，不掺身份字段：{}",
        failure.message
    );
}
