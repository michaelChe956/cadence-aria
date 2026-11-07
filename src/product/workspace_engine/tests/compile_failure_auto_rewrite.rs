//! A3（owner 2026-10-31 第 3 条 / REQ-WSC-25 / REQ-WG-05）：编译失败的自动
//! 重写必须真正委托 SC author。
//!
//! 现场 `auto_2adc5e41`：durable 宣称 `auto_rewrite.phase=running,
//! remaining=2`，但 0.67 秒内直落人工门、零次 author 运行。根因（物证
//! `human_gate_snapshot.trigger=verification_new_findings`）：初评通过的
//! 候选再次进评审时 cycle 已计 `initial_count=1` → 相位判 Verification，
//! 平台合成的编译失败 finding 不在 invocation 的 original_fingerprints
//!（初评通过 → 空集）→ 撞上 new-findings 守卫 → HumanRequired → 人工门。
//! F5-A 机械返修通道不踩坑只是因为它在初评前跑（initial_count=0）。
//!
//! 本文件钉住：
//! 1. 初评已通过的候选，编译失败 → 真正委托 author 重写（durable phase
//!    Generate + provider key 预领 + ProviderRunRequested 事件 + 不开门）；
//! 2. 红线：模型自己产出的新 finding 在 Verification 相位仍必落人工门；
//! 3. 红线：平台合成 finding 与模型 finding 混在同一 verdict 时仍落人工门；
//! 4. 通道改道人工门时只开一扇门，且 durable auto_rewrite 不得继续谎报
//!    running（REQ-WSC-25）。

use super::*;
use crate::product::json_store::write_json;
use crate::product::models::{
    CandidateAutoRewrite, CandidateAutoRewritePhase, CandidateCompileFailure,
    CandidateCompileStatus, SingleCandidatePhase, WorkspaceSessionStatus,
};
use crate::product::work_item_plan_policy::{
    FindingClassHint, ReviewCycleState, ReviewFindingCategory, RunPolicy, WorkItemPlanFlowKind,
};
use crate::product::workspace_engine::compile_failure_surface::CYCLE_FINGERPRINT_MARKER;
use crate::web::workspace_ws_types::TimelineNodeType;
use crate::web::workspace_ws_types::{
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
};
use sha2::{Digest, Sha256};

const REP4_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
));

/// 干净候选（campaign 同口径：终端 WI-003 handoff 提供行清空，机械报告
/// 无 Error，不产生 F5-A 机械 verdict）。
fn clean_candidate() -> String {
    REP4_FIXTURE.replace(
        "- provided_contract_refs: contract.levels-integration",
        "- provided_contract_refs: []",
    )
}

/// 现场 2adc5e41 的环诊断原文（producer `cycle_diagnostic::cycle_reason`
/// 的 offending 分支逐字形态；标题与位次与真实一致）。
const CYCLE_MESSAGE: &str = "「公告列表行删除交互与中文确认」 → 「删除链路端到端验收」 → 「公告列表行删除交互与中文确认」 构成依赖环（箭头为产出流向：左边 Work Item 的产出被右边消费，即右边依赖左边）。其中「删除链路端到端验收」把「公告列表行删除交互与中文确认」写成 provider，但「公告列表行删除交互与中文确认」所在仓是 change_order 第 3 个仓、「删除链路端到端验收」所在仓是第 2 个仓——跨仓依赖只允许后仓 Work Item 消费前仓 Work Item 的产出，这条依赖逆了仓序，因此与仓序自动边合成环。请让「删除链路端到端验收」改由仓序在前的 Work Item 提供产出，或删掉这条 depends_on。";

fn cycle_candidate_hash_segment(revision_ref: &str) -> &str {
    revision_ref.rsplit('-').next().unwrap_or(revision_ref)
}

/// 驱动 `complete_single_candidate_work_item_plan_author` 需要的 durable 形态：
/// Generate 相位、无候选 refs（首轮流）——与 F5-A 用例同构。
fn author_generate_round(lifecycle: &LifecycleStore, engine: &mut WorkspaceEngine) {
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = RunPolicy::AutoIfValid;
    record.single_candidate_phase = Some(SingleCandidatePhase::Generate);
    record.work_item_plan_source_revision_ref = None;
    record.plan_candidate_ir_ref = None;
    record.mechanical_report_ref = None;
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist author round session");
    engine.session = WorkspaceSession::from_record(record);
}

/// 现场形态夹具：干净候选已生成（真实 source/IR/report artifacts），
/// Round 1 初评已通过（cycle initial_count=1），phase=Approval，终态路由
/// 已关闭 durable scope；候选编译失败事实已落 durable（auto_rewrite
/// running，剩余 2）。返回带事件通道的新引擎。
async fn compile_failure_fixture(
    cycle_verification_count: u32,
) -> (
    tempfile::TempDir,
    LifecycleStore,
    WorkspaceEngine,
    tokio::sync::mpsc::Receiver<EngineEvent>,
    String,
) {
    let (_tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    author_generate_round(&lifecycle, &mut engine);
    engine
        .complete_single_candidate_work_item_plan_author(
            clean_candidate(),
            fixture_sc_target(&engine),
            fixture_sc_member_targets(&engine),
            false,
        )
        .await
        .expect("clean candidate must compile and persist");

    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("reload candidate session");
    let ir_ref = record.plan_candidate_ir_ref.clone().expect("ir ref");
    let cycle_key = format!("sc:candidate:{}", cycle_candidate_hash_segment(&ir_ref));
    record.run_policy = RunPolicy::AutoIfValid;
    record.status = WorkspaceSessionStatus::Running;
    record.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    // Round 1 通过的终态路由（ContinueToCompleted）关闭了 durable scope。
    record.review_invocation_scope = None;
    record.human_gate_snapshot = None;
    record.structural_governance_state = None;
    record.run_history.review_cycles.insert(
        cycle_key.clone(),
        ReviewCycleState {
            initial_count: 1,
            verification_count: cycle_verification_count,
            ..ReviewCycleState::default()
        },
    );
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist approval-phase fixture");

    let fingerprint = {
        let mut hasher = Sha256::new();
        hasher.update(b"witness");
        hex::encode(hasher.finalize())
    };
    let status = CandidateCompileStatus {
        source_revision_ref: record.work_item_plan_source_revision_ref.clone(),
        plan_candidate_ir_ref: Some(ir_ref),
        failure: Some(CandidateCompileFailure {
            failure_id: "planning_compile_failure_auto_rewrite".to_string(),
            code: "final_compile_prepare_failed".to_string(),
            message: CYCLE_MESSAGE.to_string(),
            detail: format!(
                "failure_id: planning_compile_failure_auto_rewrite\ncode: final_compile_prepare_failed\n{CYCLE_FINGERPRINT_MARKER}{fingerprint}\n"
            ),
            recorded_at: "2026-10-31T04:19:59Z".to_string(),
        }),
        cycle_fingerprint: Some(fingerprint),
        repeat_count: Some(1),
        auto_rewrites_used: 1,
        auto_rewrite: Some(CandidateAutoRewrite {
            phase: CandidateAutoRewritePhase::Running,
            remaining: Some(2),
            repeated: Some(1),
        }),
        recorded_at: "2026-10-31T04:19:59Z".to_string(),
    };
    lifecycle
        .save_candidate_compile_status(&record.id, &status)
        .expect("persist candidate compile status");

    let (event_tx, event_rx) = tokio::sync::mpsc::channel(64);
    let engine = WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(record),
    );
    (_tmp, lifecycle, engine, event_rx, cycle_key)
}

fn drain_engine_events(
    event_rx: &mut tokio::sync::mpsc::Receiver<EngineEvent>,
) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }
    events
}

/// ③ 验收：编译失败 → 真正委托 SC author 重写（REQ-WSC-25 / REQ-WG-05）。
#[tokio::test]
async fn compile_failure_after_passed_initial_review_delegates_author_rerun() {
    let (_tmp, lifecycle, mut engine, mut event_rx, cycle_key) = compile_failure_fixture(0).await;

    let routed = engine.route_compile_failure_to_rewrite().await;
    assert!(
        routed,
        "自动重写通道必须消费本次编译失败（不得回落再开第二扇门）"
    );

    // 真正的委托事实（与 F5-A TriggerAggregateRepair 同一组 durable 断言）。
    let persisted = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load persisted session");
    assert_eq!(
        persisted.single_candidate_phase,
        Some(SingleCandidatePhase::Generate),
        "编译失败的自动重写必须把相位路由回 Generate 供 author 重跑"
    );
    assert!(
        persisted.provider_start_ledger.iter().any(|entry| {
            entry.provider_start_idempotency_key
                == format!("single_candidate_author:{}:0", persisted.id)
                && entry.started
        }),
        "必须原子预领下一次 author provider 启动键"
    );
    assert!(
        persisted
            .repair_reservation
            .as_ref()
            .is_some_and(|reservation| {
                reservation.provider_start_idempotency_key
                    == format!("single_candidate_author:{}:0", persisted.id)
                    && reservation.state
                        == crate::product::work_item_plan_policy::RepairReservationState::Reserved
            }),
        "预领键必须处于 Reserved（接力 run 可消费）"
    );
    assert!(
        persisted.human_gate_snapshot.is_none(),
        "本轮不得落人工门（owner 侧「0.67 秒直落人工门、零重写」的根修）"
    );
    assert!(
        drain_engine_events(&mut event_rx)
            .iter()
            .any(|event| matches!(
                event,
                EngineEvent::ProviderRunRequested {
                    kind: crate::product::workspace_engine::types::ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
                    ..
                }
            )),
        "必须发出 SC author provider run 请求（author 运行节点的唯一接续起点）"
    );
    assert!(
        engine.sc_author_rerun_delegated(),
        "委托态必须可判定（phase=Generate）"
    );
    // 修订轮判定命中：下一次 author 重跑必须回灌编译失败 findings。
    assert!(
        engine.single_candidate_pending_revision_verdict().is_some(),
        "合成 verdict 必须注入 latest_review_verdict 供下一轮 author 回灌"
    );
    // 尝试可序列观测（REQ-WSC-25）：本轮消耗 cycle 返修预算并计入复评轮。
    let cycle = persisted
        .run_history
        .review_cycles
        .get(&cycle_key)
        .expect("cycle recorded");
    assert_eq!(cycle.repairs_used, 1, "自动重写必须消耗（可观测的）预算");
    assert_eq!(
        cycle.verification_count, 1,
        "合成轮必须计入 cycle 复评计数（可序列观测）"
    );
}

/// 红线 1：Verification 相位下，**模型产出**的新 actionable finding 仍必落
/// 人工门——new-findings 守卫不因本修复整体放宽。
#[tokio::test]
async fn model_finding_in_verification_still_lands_human_gate() {
    let (_tmp, lifecycle, mut engine, mut event_rx, _cycle_key) = compile_failure_fixture(0).await;

    let model_finding = ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: "模型复评时发现的新问题：验收命令引用了不存在的脚本".to_string(),
        evidence: "reviewer verification round".to_string(),
        required_action: "修正验收命令".to_string(),
        category: Some(ReviewFindingCategory::ContractGap),
        class_hint: Some(FindingClassHint::Repairable),
        contract_field: None,
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    };
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "复评发现新问题".to_string(),
        summary: "复评发现新问题".to_string(),
        findings: vec![model_finding],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    engine
        .route_single_candidate_contract_prerevision(verdict)
        .await;

    let persisted = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load persisted session");
    assert_ne!(
        persisted.single_candidate_phase,
        Some(SingleCandidatePhase::Generate),
        "模型产出的新 finding 不得触发自动重写委托"
    );
    assert!(
        persisted.human_gate_snapshot.is_some(),
        "模型产出的新 finding 在 Verification 相位必须落人工门（new-findings 守卫不放宽）"
    );
    assert!(
        !drain_engine_events(&mut event_rx)
            .iter()
            .any(|event| matches!(event, EngineEvent::ProviderRunRequested { .. })),
        "不得请求任何 provider run"
    );
}

/// 红线 2：平台合成 finding 与模型 finding 混在同一 verdict 时，模型侧
/// finding 仍把整轮拦进人工门（前缀豁免按 finding 生效，不得按轮次豁免）。
#[tokio::test]
async fn mixed_platform_and_model_findings_still_land_human_gate() {
    let (_tmp, lifecycle, mut engine, mut event_rx, _cycle_key) = compile_failure_fixture(0).await;

    let platform_finding = ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: format!(
            "candidate_compile_failure:final_compile_prepare_failed:{}（候选依赖图仍需消除该环）",
            "f".repeat(64)
        ),
        evidence: "final compile rejected the candidate".to_string(),
        required_action: "按重写要点修正".to_string(),
        category: Some(ReviewFindingCategory::ContractGap),
        class_hint: Some(FindingClassHint::Repairable),
        contract_field: None,
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    };
    let model_finding = ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: "模型同轮发现的新问题：标题与正文不一致".to_string(),
        evidence: "reviewer verification round".to_string(),
        required_action: "修正标题".to_string(),
        category: Some(ReviewFindingCategory::ContractGap),
        class_hint: Some(FindingClassHint::Repairable),
        contract_field: None,
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    };
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "混合轮".to_string(),
        summary: "混合轮".to_string(),
        findings: vec![platform_finding, model_finding],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    engine
        .route_single_candidate_contract_prerevision(verdict)
        .await;

    let persisted = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load persisted session");
    assert_ne!(
        persisted.single_candidate_phase,
        Some(SingleCandidatePhase::Generate),
        "混合轮不得触发自动重写委托"
    );
    assert!(
        persisted.human_gate_snapshot.is_some(),
        "混合轮必须落人工门（模型 finding 照常触发守卫）"
    );
    assert!(
        !drain_engine_events(&mut event_rx)
            .iter()
            .any(|event| matches!(event, EngineEvent::ProviderRunRequested { .. })),
        "混合轮不得请求任何 provider run"
    );
}

/// 次生 bug 收口：通道改道人工门（如同 cycle 复评预算已耗尽的 494 形态）时，
/// 只开一扇门，且 durable auto_rewrite 必须从 running 对账为 stopped——
/// 不得对 owner 谎报「正在自动重写」（REQ-WSC-25）。
#[tokio::test]
async fn misrouted_auto_rewrite_opens_single_gate_and_reconciles_status() {
    let (_tmp, lifecycle, mut engine, mut event_rx, _cycle_key) = compile_failure_fixture(1).await;

    let routed = engine.route_compile_failure_to_rewrite().await;
    assert!(
        routed,
        "改道人工门的轮次也必须被通道消费（门已开，不得再开第二扇）"
    );

    let gate_nodes = engine
        .timeline_nodes
        .iter()
        .filter(|node| matches!(node.node_type, TimelineNodeType::HumanConfirm))
        .count();
    assert_eq!(
        gate_nodes, 1,
        "只允许一扇人工门（现场曾出现「需要人工接管」+「人工确认」双门）"
    );

    let persisted_status = lifecycle
        .load_candidate_compile_status(&engine.session().session_id)
        .expect("load compile status")
        .expect("status present");
    let auto_rewrite = persisted_status
        .auto_rewrite
        .expect("auto rewrite record present");
    assert_eq!(
        auto_rewrite.phase,
        CandidateAutoRewritePhase::Stopped,
        "未真正委托时 durable 必须对账为 stopped，不得谎报 running"
    );

    assert!(
        !drain_engine_events(&mut event_rx)
            .iter()
            .any(|event| matches!(
                event,
                EngineEvent::ProviderRunRequested {
                    kind: crate::product::workspace_engine::types::ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
                    ..
                }
            )),
        "预算耗尽形态不得委托 author"
    );
}
