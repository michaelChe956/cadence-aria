#[tokio::test]
async fn conversational_gate_revision_routes_evaluate_to_approval_then_confirm_succeeds() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("revision_route_confirm", 2, 0);
    let turn_id = open_running_revision_turn(&mut engine, "revision_route_command").await;

    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("valid revision must complete");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));

    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("durable turn");
    assert_eq!(
        turn.status,
        crate::product::models::HumanGateTurnStatus::Completed
    );
    assert!(turn.result_artifact_ref.is_some());

    // 核心修复断言:修订后 session 必须经 Evaluate policy route 到 Approval,
    // 否则 confirm 的 close CAS 前置永远不可达(第 5 死路)。
    let routed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("routed session");
    assert_eq!(
        routed.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    assert_eq!(
        routed.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval)
    );
    assert_eq!(
        engine.session().stage,
        crate::product::workspace_engine::WorkspaceStage::HumanConfirm
    );

    // confirm 不再撞 human_gate_close CAS 冲突,真实批准链落地终态。
    let outcome = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect("confirm must close the gate after the Evaluate route");
    assert_eq!(
        outcome,
        crate::product::workspace_engine::HumanGateCloseOutcome::Confirmed
    );
    let closed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("closed session");
    assert_eq!(
        closed.status,
        crate::product::models::WorkspaceSessionStatus::Confirmed
    );
    assert_eq!(
        closed.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Completed)
    );
}

#[tokio::test]
async fn conversational_gate_revision_with_reviewer_restarts_review_before_approval() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("revision_route_reviewer", 2, 1);
    let turn_id = open_running_revision_turn(&mut engine, "revision_route_reviewer_command").await;

    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("valid revision must complete");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));

    // 有 reviewer:修订后必须重启评审(与初始 author 同构),不得直接跳 Approval,
    // 也不得清掉 reservation(它只在 close 终态清理)。
    assert_eq!(
        engine.session().stage,
        crate::product::workspace_engine::WorkspaceStage::CrossReview,
        "revision with a reviewer must restart the review before approval"
    );
    let after_revision = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session after revision");
    assert_eq!(
        after_revision.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    assert_eq!(
        after_revision.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Evaluate)
    );
    assert!(
        after_revision.human_gate_reservation.is_some(),
        "restarting review must not clear the human gate reservation"
    );

    // reviewer pass 后才进 Approval;随后 confirm 走通批准链。
    let verdict = crate::web::workspace_ws_types::ReviewVerdict {
        verdict: crate::web::workspace_ws_types::ReviewVerdictType::Pass,
        comments: "review pass".to_string(),
        summary: "review pass".to_string(),
        findings: Vec::new(),
        review_gate: crate::web::workspace_ws_types::ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "review".to_string(),
                None,
            ),
            verdict,
        )
        .await;
    let routed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("routed session");
    assert_eq!(
        routed.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    assert_eq!(
        routed.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval)
    );

    let outcome = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect("confirm must close the gate after reviewer pass");
    assert_eq!(
        outcome,
        crate::product::workspace_engine::HumanGateCloseOutcome::Confirmed
    );
    let closed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("closed session");
    assert_eq!(
        closed.status,
        crate::product::models::WorkspaceSessionStatus::Confirmed
    );
    assert_eq!(
        closed.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Completed)
    );
}

#[tokio::test]
async fn conversational_gate_revision_route_cas_conflict_retries_without_clearing_reservation() {
    let (_root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("revision_route_conflict", 2, 0);
    // 模拟并发 worker 在 route CAS 前改写 durable 记录:首次持久化冲突必须
    // reload+重评估后成功,不得误清 reservation,也不得落入假终态。
    engine.policy_route_before_persist = Some(Box::new(|store, session_id| {
        store
            .update_workspace_session_status(
                session_id,
                crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
            )
            .expect("concurrent route update must be durable");
    }));
    let turn_id = open_running_revision_turn(&mut engine, "revision_route_conflict_command").await;
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("valid revision must complete");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));

    let routed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("routed session");
    assert_eq!(
        routed.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
        "a single route CAS conflict must be retried against fresh durable state"
    );
    assert_eq!(
        routed.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    assert!(
        routed.human_gate_reservation.is_some(),
        "route persistence conflict must not clear the reservation"
    );
}

#[tokio::test]
async fn conversational_gate_revision_completed_turn_stale_evaluate_reconnect_never_restarts_provider()
 {
    let (_root, lifecycle, mut engine) = evaluate_gate_revision_fixture("revision_reconnect", 2, 1);
    let turn_id = open_running_revision_turn(&mut engine, "revision_reconnect_command").await;
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("valid revision must complete");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));

    // 现场死锁残留形态:phase=Evaluate + completed turn + reservation。
    // 重连恢复必须零 provider 重启(completed turn 是终态),session 保持可重试。
    let record = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("stale durable session");
    assert_eq!(
        record.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Evaluate)
    );
    assert!(record.human_gate_reservation.is_some());
    let ledger_len = record.provider_start_ledger.len();

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
    let mut recovered = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            _root.path().join("reconnect-checkpoints"),
        )),
        lifecycle.clone(),
        event_tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(record),
    );
    let actions = recovered
        .recover_human_gate_turns(false)
        .expect("recover human gate turns");
    assert!(
        actions.is_empty(),
        "a completed turn must not resume or restart a provider: {actions:?}"
    );
    let after = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session after recovery");
    assert_eq!(
        after.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
        "stale Evaluate must stay retryable instead of a false terminal"
    );
    assert_eq!(
        after.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Evaluate)
    );
    assert_eq!(after.provider_start_ledger.len(), ledger_len);
}

#[tokio::test]
async fn conversational_gate_revision_reopens_evaluate_gate_at_approval_after_interruption() {
    let (root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("revision_reopen_after_interruption", 2, 0);
    let turn_id =
        open_running_revision_turn(&mut engine, "revision_reopen_after_interruption_command").await;

    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("修订候选必须重新打开人工门");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));

    let reopened = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("重开的门会话");
    assert_eq!(
        reopened.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    assert_eq!(
        reopened.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
        "修订完成必须把被中断的 Evaluate 门回置到 Approval 节点"
    );

    // 断连重连由 durable 记录重建引擎；门的相位和派生 stage 必须重新一致。
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
    let recovered = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            root.path().join("reopen-after-interruption-checkpoints"),
        )),
        lifecycle,
        event_tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(reopened),
    );
    assert_eq!(
        recovered.session().stage,
        crate::product::workspace_engine::WorkspaceStage::HumanConfirm,
        "断连重连后，Approval 门必须恢复到 human_confirm"
    );
}

// —— 确定性结构标题归一化(2026-09-03 现场 pi 两次标题翻译事故)——
//
// 现场:pi 随机把结构标题翻成中文(### 身份/### 目标/…)导致 compiler
// missing_section×N fail-closed。修复契约 = 与前言修剪同层的固定中文→英文
// 映射表;表外未知标题不猜不改,仍 fail-closed;正文零触碰。

const REP4_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
));

#[tokio::test]
async fn conversational_gate_revision_compiles_with_lc_shaped_empty_story_repository() {
    // r30 根修锚点(r29 现场):LC 会话的 story.repository_id 为空(Logical
    // story 生成落空串),修订 turn 此前经 work_item_plan_repository_id 直取
    // story.repository_id → compile lowering_error「session context 缺少
    // target repository」→ turn validation_reject,门上修订对 LC 会话恒不可用
    //(fresh 生成链经 workspace_repository_for_session 取 repository.id 不受
    // 影响)。修:修订 turn 同源经会话记录解析 repository,story 字段仅作
    // 解析失败时的 legacy 回退。
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("revision_lc_story_repository", 2, 0);
    // LC 形态:把源 story 的 repository_id 置空(模拟 Logical story 记录),
    // issue.repo_id/物理仓仍在——会话记录解析必须不依赖 story 字段。
    let plan_source_story = lifecycle
        .list_story_specs("project_0001", "issue_0001")
        .expect("story specs")
        .pop()
        .expect("at least one story");
    let mut lc_shaped = plan_source_story.clone();
    lc_shaped.repository_id = String::new();
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root("project_0001", "issue_0001")
            .join("story-specs")
            .join(format!("{}.json", lc_shaped.id)),
        &lc_shaped,
    )
    .expect("persist lc-shaped story");

    let turn_id = open_running_revision_turn(&mut engine, "revision_lc_command").await;
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("revision turn must terminate");
    assert!(
        matches!(
            result,
            crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
        ),
        "LC 形态(空 story.repository_id)修订必须经会话记录解析 repository 完成编译,不得 lowering_error"
    );
}

/// 诊断冻结缺口：SC 门内修订轮换 refs 后旧 Approval 元组未被失效 → 复评
/// 通过的 confirm 在 compile 冲突臂以新 refs 重算 attempt id ≠ 旧元组 →
/// `single_candidate_approval_conflict`，门永久卡死。本测试铺真实事故链：
/// 首轮 confirm 在 provenance 边界中断（元组/reservation/provenance 已
/// durable）→ 门重开 → 门内修订换 refs → 复评路由回 Approval → confirm。
/// 修复后修订 CAS 随 refs 真正轮换失效旧元组，confirm 以新元组/新编译
/// 代次走通完整批准链（同 source 重放不动有效元组由 store 层测试钉住）。
#[tokio::test]
async fn conversational_gate_revision_rotates_stale_approval_tuple_then_confirm_compiles() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("revision_rotate_tuple", 2, 0);

    // 首轮 confirm：close CAS 原子提升 Evaluate→Approval（人工权威升级），
    // compile 在 provenance 边界中断；durable 留下与 v1 refs 绑定的旧元组、
    // compile reservation 与旧 provenance 引用，门按 F-54 B2 重开等待人工。
    let failpoint = engine.register_single_candidate_compile_failpoint(
        crate::product::workspace_engine::SingleCandidateCompileCheckpoint::ProvenancePersisted,
    );
    let first_error = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect_err("首轮 confirm 必须在 provenance 边界中断");
    drop(failpoint);
    assert!(
        first_error.contains("single-candidate approval compile failed"),
        "unexpected first confirm error: {first_error}"
    );
    let stalled = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("stalled session after provenance boundary");
    assert_eq!(
        stalled.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval)
    );
    assert_eq!(
        stalled.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
        "F-54 B2：compile 可恢复失败必须重开门而不是落终态"
    );
    assert!(stalled.approval_attempt_id.is_some());
    let stale_attempt_id = stalled.approval_attempt_id.clone().unwrap();
    let stale_compile_id = stalled
        .compile_reservation
        .as_ref()
        .expect("reservation 必须先于 provenance durable")
        .compile_id
        .clone();
    let stale_provenance_ref = stalled
        .publication_provenance_ref
        .clone()
        .expect("provenance ref 必须先于中断 durable");

    // 门内人工修订：v2 source 内容寻址出新 refs，修订 CAS 必须随 refs 真正
    // 轮换失效旧批准绑定（红：旧代码保留旧元组 → confirm 撞冲突臂）。
    let turn_id = open_running_revision_turn(&mut engine, "revision_rotate_tuple_command").await;
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("in-gate revision must complete");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    let revised = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("revised session");
    assert_ne!(
        revised.work_item_plan_source_revision_ref, stalled.work_item_plan_source_revision_ref,
        "门内修订必须真正轮换 refs"
    );
    assert_eq!(
        revised.approval_attempt_id, None,
        "修订轮换 refs 必须失效旧 Approval 元组"
    );
    assert_eq!(revised.approved_at, None, "approved_at 随元组失效");
    assert_eq!(
        revised.compile_reservation, None,
        "compile_reservation 随元组失效"
    );
    assert_eq!(
        revised.publication_provenance_ref, None,
        "旧 provenance 引用随旧编译代次失效"
    );
    assert_eq!(
        revised.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
        "复评通过（无 reviewer 走 synthetic Pass）必须路由回 Approval"
    );

    // 再 confirm：批准链必须以新元组走通（红：`single_candidate_approval_conflict`）。
    let outcome = engine
        .handle_human_gate_termination(HumanGateCloseDecision::Approve)
        .await
        .expect("confirm after in-gate revision must re-approve the rotated refs");
    assert_eq!(
        outcome,
        crate::product::workspace_engine::HumanGateCloseOutcome::Confirmed
    );
    let closed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("closed session");
    assert_eq!(
        closed.status,
        crate::product::models::WorkspaceSessionStatus::Confirmed
    );
    assert_eq!(
        closed.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Completed)
    );
    // 新批准元组与新 refs 绑定，编译建立新代次（不静默覆盖旧 compile/provenance）。
    let new_attempt_id = closed
        .approval_attempt_id
        .clone()
        .expect("rotated approval tuple");
    assert_ne!(new_attempt_id, stale_attempt_id);
    let new_reservation = closed
        .compile_reservation
        .as_ref()
        .expect("new compile reservation");
    assert_ne!(
        new_reservation.compile_id, stale_compile_id,
        "refs 轮换后的 confirm 必须建立新编译代次"
    );
    assert_ne!(
        closed.publication_provenance_ref.as_deref(),
        Some(stale_provenance_ref.as_str()),
        "新代次 provenance 引用不得复用旧代次"
    );
}

// ---------------------------------------------------------------------------
// SG6a.1（续 part_02）：业务事实缺少走原门（ExistingGate）、operational
// waiting、类别/class_hint 不能许可、L1 多解不用户拓扑、超限 typed 系统
// 故障、terminal replay 稳定拒收。
// ---------------------------------------------------------------------------

use crate::product::workspace_engine::conversational_gate::HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN_CODE;

/// 未 typed 编译失败候选：未知三级 section（`### 未知段`）在固定归一化
/// 表外 → compiler 共享 `unknown_structured_key` 码、无 typed kind。其
/// message/字段虽指向「section/heading」，但类别与 class_hint 不得许可
/// L1——必须折叠为 Unknown、走原门（ExistingGate），零机器指令/零 claim。
fn sg61_untyped_section_candidate() -> String {
    REP4_FIXTURE.replace(
        "## Work Item WI-002: Frontend level selector",
        "## Work Item WI-002: Frontend level selector\n### 未知业务段",
    )
}

/// SG6a.1 主验收③：业务事实缺少/未 typed 失败走原门——`ExistingGate`
/// 分支零机器指令、零 claim、零 author；turn 仍 Running、门开态；类别
/// 与 class_hint（message 里的 heading/section 字样）不构成 L1 许可。
#[tokio::test]
async fn sg_06a_1_untyped_failure_goes_existing_gate_and_category_cannot_license() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a1_existing_gate", 3);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a1_existing_command").await;
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after open");
    sg61_seed_structural_grant(&lifecycle, &engine, &turn, 1);
    let output = sg61_untyped_section_candidate();

    let assessment = engine
        .assess_sc_revision_output(&turn_id, &output)
        .expect("untyped compile failure must still classify deterministically");
    match &assessment {
        StructuralAssessment::ExistingGate(set) => {
            assert!(
                set.failures.iter().all(|failure| matches!(
                    failure.kind,
                    crate::product::planning_failure::PlanningFailureKind::Unknown { .. }
                )),
                "untyped diagnostics fold to Unknown kinds, never inferred from message text"
            );
        }
        other => panic!(
            "untyped failure without business-fact evidence must hit the existing gate, got {other:?}"
        ),
    }

    let result = engine
        .run_sc_manual_revision_turn(&turn_id, output)
        .await
        .expect("existing-gate escalation is in-band");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected { .. }
    ));
    // 原门语义（SG6a.3 迁移，复审①）：机器不可修的失败回原门=turn 收口
    // Failed(validation_reject)+nonstructural_failure_charge——门重新可接
    // 新反馈；grant 未被 claim 推进（零 L1 授权）、零 provider 启动。
    let turn_after = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after existing-gate outcome");
    assert_eq!(
        turn_after.status,
        crate::product::models::HumanGateTurnStatus::Failed,
        "SG6a.3: existing-gate rejection closes the turn (unlocks the Busy face)"
    );
    assert_eq!(
        turn_after.failure_class,
        Some(crate::product::models::HumanGateTurnFailureClass::ValidationReject)
    );
    let grant = sg61_current_grant(&lifecycle, engine.session().session_id.as_str())
        .expect("seeded grant untouched");
    assert_eq!(
        grant.attempt_no, 1,
        "ExistingGate outcome must not claim an L1 attempt"
    );
    assert_eq!(
        grant.phase,
        Some(crate::product::models::workspace::StructuralGrantPhase::AuthorRunning)
    );
}

/// SG6a.1 主验收④⑤：operational 等待不驱 author（超硬限裁决 → 零
/// claim/零启动，turn 仍 Running）；L1 多解（环修复候选族）不把拓扑
/// 决策推给用户——指令为 Candidates 族（无唯一性标签）、不新开反馈轮、
/// 用户零结构动作。
#[tokio::test]
async fn sg_06a_1_operational_waits_without_claim_and_l1_multi_solution_keeps_user_topology() {
    // —— operational waiting：超 provider 硬限的失败候选，冻结 prompt
    // 裁决 fail-closed（SG5.2 adjudicate），不 claim、不启动、turn Running。
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a1_operational_wait", 3);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a1_wait_command").await;
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after open");
    sg61_seed_structural_grant(&lifecycle, &engine, &turn, 1);
    let before_ledger = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session before")
        .provider_start_ledger
        .len();
    // 环候选 + Rationale 填充超过 author provider（ClaudeCode）300,000B
    // 硬限——冻结 prompt 裁决 fail-closed（SG5.2 adjudicate）。
    let mut oversized = sg61_cycle_candidate();
    while oversized.len() < 320_000 {
        oversized.push_str("- 超限填充行：驱动冻结 prompt 超 provider 硬限的等待面。\n");
    }
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, oversized)
        .await
        .expect("operational wait is in-band");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
            ref diagnostics,
            ..
        } if diagnostics.iter().any(|line| line.contains("OVER_HARD_LIMIT"))
    ));
    let grant = sg61_current_grant(&lifecycle, engine.session().session_id.as_str())
        .expect("grant untouched by operational wait");
    assert_eq!(grant.attempt_no, 1, "operational wait must not claim");
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("session after wait")
            .provider_start_ledger
            .len(),
        before_ledger
    );
    assert_eq!(
        lifecycle
            .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
            .expect("turn after budget closure")
            .status,
        crate::product::models::HumanGateTurnStatus::Failed,
        "SG6a.3（复审①）：预算拒收口 turn——零 claim arm 不留 Running 死锁"
    );

    // —— L1 多解不用户拓扑：同一环失败的指令是 Candidates 候选族（无
    // Precise 唯一性标签），author 在保护约束下选择；用户不必选仓 UUID、
    // 删依赖或修行号——零新反馈轮（turn 计数不增）。
    let (_root2, _lifecycle2, mut engine2) = durable_revision_fixture("sg06a1_multi_solution", 3);
    let turn_id2 = open_running_revision_turn(&mut engine2, "sg06a1_multi_command").await;
    let output2 = sg61_cycle_candidate();
    let assessment = engine2
        .assess_sc_revision_output(&turn_id2, &output2)
        .expect("facts-complete cycle failure is L1-classifiable");
    let instructions = match &assessment {
        StructuralAssessment::Rejected(set, instructions) => {
            assert!(
                set.failures.iter().any(|failure| matches!(
                    failure.kind,
                    crate::product::planning_failure::PlanningFailureKind::DependencyCycle { .. }
                )),
                "cycle candidate must carry a typed DependencyCycle failure"
            );
            instructions
        }
        other => panic!("multi-solution cycle must stay L1 (Rejected), got {other:?}"),
    };
    let instruction = instructions
        .first()
        .expect("cycle failure yields machine instructions");
    assert!(
        matches!(
            &instruction.solution,
            crate::product::workspace_engine::contract_prerevision::StructuralRepairSolution::Candidates { .. }
        ),
        "topology-shaped repair space must expose candidates, never a fabricated unique fix"
    );
    assert!(
        !instruction.expected_predicates.is_empty(),
        "machine instructions must carry verifiable post-predicates"
    );
    assert!(
        !instruction.forbidden_operations.is_empty(),
        "protected business facts are enforced via explicit forbidden operations"
    );
    let turns = _lifecycle2
        .list_human_gate_turns(engine2.session().session_id.as_str())
        .expect("list turns");
    assert_eq!(
        turns.len(),
        1,
        "L1 repair drives the same turn (attempt2), not a new user feedback round"
    );
}

/// SG6a.1 主验收⑥⑦：物理 attempt 耗尽 → typed 系统故障（稳定码、零
/// claim 写、零第三次启动、turn 不先终态）；成功路径的 terminal replay
/// 稳定拒收（command_id 复用可见回显）保持不变。
#[tokio::test]
async fn sg_06a_1_attempts_exhausted_typed_system_fault_and_terminal_replay_stable() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a1_exhausted", 3);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a1_exhausted_command").await;
    // L0 已消费 attempt2（恢复链同 turn 续跑一次）——物理上限耗尽。
    let mut attempt2 = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn");
    attempt2.attempt_no = 2;
    let expected = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session");
    lifecycle
        .update_human_gate_turn(&expected, attempt2)
        .expect("bump turn to attempt 2");
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after attempt bump");
    let output = sg61_cycle_candidate();

    // 判定面：attempt 耗尽 → typed 系统故障（Err 携稳定码），不是新的
    // L1 资格。
    let assessment = engine.assess_sc_revision_output(&turn_id, &output);
    let failure = assessment.expect_err("exhausted attempts turn L1 into a typed system fault");
    assert_eq!(
        failure.stable_code(),
        "STRUCTURAL_ATTEMPTS_EXHAUSTED",
        "typed system fault must carry a stable code"
    );

    // 驱动面：同停点 in-band 回显；turn 不被先验终态化；session/grant
    // 零新写、零第三次启动。
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, output)
        .await
        .expect("typed system fault is in-band");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
            ref diagnostics,
            ..
        } if diagnostics
            .iter()
            .any(|line| line.contains("STRUCTURAL_ATTEMPTS_EXHAUSTED"))
    ));
    let after = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after exhausted stop");
    assert_eq!(
        after.provider_start_ledger.len(),
        before.provider_start_ledger.len(),
        "no third provider start beyond the physical attempt cap"
    );
    assert_eq!(
        lifecycle
            .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
            .expect("turn after exhausted closure")
            .status,
        crate::product::models::HumanGateTurnStatus::Failed,
        "SG6a.3（复审②③）：耗尽停点终结写——Failed(validation_reject)+release 恰一次"
    );
    let closed = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after exhausted closure");
    let closed_state = closed
        .structural_governance_state
        .as_ref()
        .expect("state after exhausted closure");
    assert_eq!(
        closed_state
            .user_receipts
            .iter()
            .filter(|receipt| receipt.turn_id == turn_id
                && receipt.kind
                    == crate::product::models::UserBudgetReceiptKind::ReleasedStructural)
            .count(),
        1,
        "release hold exactly once on typed structural exhaustion"
    );
    assert_eq!(
        closed_state.current_grant.as_ref().expect("grant").phase,
        Some(crate::product::models::workspace::StructuralGrantPhase::SystemFault),
        "耗尽证据终态化 grant（SystemFault）"
    );

    // —— terminal replay 稳定拒收（既有语义回归守卫）：成功完成后同
    // command_id 重放 → typed 拒收回显。
    let (_root2, lifecycle2, mut engine2) = durable_revision_fixture("sg06a1_replay_stable", 2);
    let command_id = "sg06a1_replay_command".to_string();
    let turn2 = open_running_revision_turn(&mut engine2, &command_id).await;
    engine2
        .run_sc_manual_revision_turn(&turn2, handoff_clean_rep4())
        .await
        .expect("successful revision completes the turn");
    let ledger_before_replay = lifecycle2
        .get_workspace_session(engine2.session().session_id.as_str())
        .expect("session after successful revision")
        .provider_start_ledger
        .len();
    let replay = engine2
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id,
            feedback: "重放同一命令".to_string(),
        })
        .await
        .expect("replay outcome is in-band");
    match replay {
        HumanGateCommandOutcome::Rejected { code, .. } => assert_eq!(
            code, HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN_CODE,
            "terminal replay keeps its stable rejection code"
        ),
        other => panic!("terminal replay must stay a typed rejection, got {other:?}"),
    }
    assert_eq!(
        lifecycle2
            .get_workspace_session(engine2.session().session_id.as_str())
            .expect("session after replay")
            .provider_start_ledger
            .len(),
        ledger_before_replay,
        "terminal replay is zero-side-effect on the provider ledger"
    );
}

/// SG6a.3 验收①（复审点名）：无 claim 的 in-band 拒收 arm 的 turn 收口
/// ——ExistingGate（业务事实缺少/未 typed）与预算拒（冻结 prompt 超
/// provider 硬限）都把 turn 终态化为 Failed(validation_reject) 并把
/// held 结转 nonstructural_failure_charge（REQ-CG-02「其他非结构终态
/// 维持原预约扣次不退」），`handle_human_gate_feedback` 对非终态恒
/// Busy 的面随之解锁（同 command 重放转终态 typed 拒收）。
#[tokio::test]
async fn sg_06a_3_unclaimed_rejections_close_turn_and_unlock_gate() {
    use crate::product::models::HumanGateTurnFailureClass;
    use crate::product::models::UserBudgetReceiptKind;

    // —— ExistingGate：untyped section 候选（类别/class_hint 不能许可）。
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a3_existing_close", 3);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a3_existing_command").await;
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, sg61_untyped_section_candidate())
        .await
        .expect("existing-gate escalation is in-band");
    assert!(matches!(
        &result,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
            diagnostics,
            claim: None,
        } if diagnostics
            .iter()
            .any(|line| line.contains("STRUCTURAL_EXISTING_GATE"))
    ));
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after closure");
    assert_eq!(
        turn.status,
        crate::product::models::HumanGateTurnStatus::Failed,
        "SG6a.3（复审①）：无 claim 拒收 arm 必须收口 turn，否则 Busy 面死锁"
    );
    assert_eq!(
        turn.failure_class,
        Some(HumanGateTurnFailureClass::ValidationReject)
    );
    let session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after closure");
    let state = session
        .structural_governance_state
        .as_ref()
        .expect("state after closure");
    assert_eq!(
        state
            .user_receipts
            .iter()
            .filter(|receipt| receipt.turn_id == turn_id
                && receipt.kind == UserBudgetReceiptKind::NonstructuralFailureCharge)
            .count(),
        1,
        "非结构失败扣次恰一次（不退）"
    );
    assert_eq!(state.active_hold_count(), 0);
    assert_eq!(state.available_user_budget(), 2);
    assert_eq!(
        session
            .human_gate_snapshot
            .as_ref()
            .expect("snapshot")
            .manual_repairs_remaining,
        2,
        "非结构失败不恢复快照预算"
    );

    // Busy 解锁：新反馈开新 turn；同 command 重放转终态 typed 拒收。
    let reopened = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "sg06a3_existing_command_next".to_string(),
            feedback: "按诊断重新反馈".to_string(),
        })
        .await
        .expect("gate must accept new feedback after closure");
    assert!(matches!(
        reopened,
        HumanGateCommandOutcome::TurnOpened { .. }
    ));
    let replay = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "sg06a3_existing_command".to_string(),
            feedback: "重放已收口命令".to_string(),
        })
        .await
        .expect("replay outcome is in-band");
    assert!(matches!(
        replay,
        HumanGateCommandOutcome::Rejected { ref code, .. }
            if code == HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN_CODE
    ));

    // —— 预算拒：L1 拒收但冻结 prompt 超 author 硬限（SG5.2 裁决
    // fail-closed）→ 同款收口（charge，不退）。
    let (_root2, lifecycle2, mut engine2) = durable_revision_fixture("sg06a3_budget_close", 3);
    let turn2_id = open_running_revision_turn(&mut engine2, "sg06a3_budget_command").await;
    let mut oversized = sg61_cycle_candidate();
    while oversized.len() < 320_000 {
        oversized.push_str("- 超限填充行：冻结 prompt 超 provider 硬限的收口面。\n");
    }
    let result2 = engine2
        .run_sc_manual_revision_turn(&turn2_id, oversized)
        .await
        .expect("budget adjudication rejection is in-band");
    assert!(matches!(
        &result2,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
            diagnostics,
            claim: None,
        } if diagnostics.iter().any(|line| line.contains("OVER_HARD_LIMIT"))
    ));
    let turn2 = lifecycle2
        .get_human_gate_turn(engine2.session().session_id.as_str(), &turn2_id)
        .expect("turn after budget closure");
    assert_eq!(
        turn2.status,
        crate::product::models::HumanGateTurnStatus::Failed
    );
    assert_eq!(
        turn2.failure_class,
        Some(HumanGateTurnFailureClass::ValidationReject)
    );
    let session2 = lifecycle2
        .get_workspace_session(engine2.session().session_id.as_str())
        .expect("session after budget closure");
    assert_eq!(
        session2
            .structural_governance_state
            .as_ref()
            .expect("state")
            .user_receipts
            .iter()
            .filter(|receipt| receipt.turn_id == turn2_id)
            .count(),
        1,
        "预算拒收口结转 nonstructural_failure_charge 恰一次"
    );
    let reopened2 = engine2
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "sg06a3_budget_command_next".to_string(),
            feedback: "缩短候选后重新反馈".to_string(),
        })
        .await
        .expect("gate must accept new feedback after budget closure");
    assert!(matches!(
        reopened2,
        HumanGateCommandOutcome::TurnOpened { .. }
    ));
}
