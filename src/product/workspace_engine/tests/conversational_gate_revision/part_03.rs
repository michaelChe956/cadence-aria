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
