// 蒸发链修法（诊断 local://diag-manual-revision-evaporate.md）：
// 修法②——command_id 命中**终态** turn 必须以 typed Rejected 回显（稳定码
// HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN），重连幂等只覆盖
// Reserved/Running。现场（workspace_session_0003 / turn 71b47478）：第二次
// 提交复用死 turn 的 command_id 被无状态过滤的查重判为 Replayed——零 durable
// 写 + 客户端对同 turn_id 幂等吞帧 = 无声蒸发。
// 顺手项——turn 失败收口修订 author 载体节点（否则永停 Active 时间线幻影）。

/// 修法②红绿锚：Failed 终态 turn 的 command_id 重放 → typed Rejected，
/// 且拒收零副作用（会话与 turn 集合原样）。
#[tokio::test]
async fn conversational_gate_feedback_replay_after_failed_turn_is_typed_rejected() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let turn_id = match engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_terminal_replay_failed",
        ))
        .await
        .expect("open turn")
    {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    engine
        .fail_human_gate_turn(
            &turn_id,
            crate::product::models::HumanGateTurnFailureClass::ValidationReject,
        )
        .await
        .expect("fail turn");
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session before terminal replay");

    let outcome = engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_terminal_replay_failed",
        ))
        .await
        .expect("terminal replay outcome");

    match outcome {
        HumanGateCommandOutcome::Rejected { code, reason } => {
            assert_eq!(
                code, "HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN",
                "terminal replay must carry the stable typed code"
            );
            assert!(
                reason.contains(&turn_id),
                "reason must identify the terminal turn: {reason}"
            );
        }
        other => panic!("terminal replay must be typed rejected, got {other:?}"),
    }
    // 拒收是可见回显而非吞帧：不得有任何 durable 写。
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("durable session after terminal replay"),
        before,
        "terminal replay rejection must not mutate durable session"
    );
    assert_eq!(
        lifecycle
            .list_human_gate_turns(engine.session().session_id.as_str())
            .expect("list turns")
            .len(),
        1
    );
}

/// 对照锚：Running（非终态）turn 的 command_id 重放保持 Replayed 幂等——
/// 修法②不得放宽重连重放语义。
#[tokio::test]
async fn conversational_gate_feedback_replay_stays_idempotent_while_turn_running() {
    let (_root, _lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let turn_id = match engine
        .handle_human_gate_feedback(super::conversational_gate::feedback("cmd_running_replay"))
        .await
        .expect("open turn")
    {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("mark running");

    let replayed = engine
        .handle_human_gate_feedback(super::conversational_gate::feedback("cmd_running_replay"))
        .await
        .expect("running replay outcome");

    match replayed {
        HumanGateCommandOutcome::Replayed { turn } => assert_eq!(turn.turn_id, turn_id),
        other => panic!("running replay must stay Replayed, got {other:?}"),
    }
}

/// 修法②红绿锚：Completed 终态 turn（awaiting_confirm 后的重复点击形态）
/// 的 command_id 重放同样 typed Rejected。
#[tokio::test]
async fn conversational_gate_feedback_replay_after_completed_turn_is_typed_rejected() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let turn_id = match engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_terminal_replay_completed",
        ))
        .await
        .expect("open turn")
    {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    // 直接经 durable store 推进 Reserved→Completed（progression 合法路径），
    // 模拟修订成功后 awaiting_confirm 期间的重复提交。
    let expected = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session");
    let mut turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("durable turn");
    turn.status = crate::product::models::HumanGateTurnStatus::Completed;
    turn.result_artifact_ref = Some("artifact_terminal_replay".to_string());
    turn.updated_at = chrono::Utc::now().to_rfc3339();
    lifecycle
        .update_human_gate_turn(&expected, turn)
        .expect("complete turn");

    let outcome = engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_terminal_replay_completed",
        ))
        .await
        .expect("completed replay outcome");

    match outcome {
        HumanGateCommandOutcome::Rejected { code, .. } => assert_eq!(
            code, "HUMAN_GATE_COMMAND_REPLAYED_AFTER_TERMINAL_TURN",
            "completed turn replay must carry the stable typed code"
        ),
        other => panic!("completed replay must be typed rejected, got {other:?}"),
    }
}

/// 顺手项红绿锚：turn 失败必须收口修订 author 载体节点——与成功路径
/// （revision_turn.inc.rs 的 complete_active_node）同构；不收口会留下永久
/// Active 的「修订中」author 节点（现场 workspace_session_0003 node_005）。
#[tokio::test]
async fn failing_human_gate_turn_closes_revision_author_node() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let turn_id = match engine
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "cmd_fail_closes_author_node",
        ))
        .await
        .expect("open turn")
    {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("mark running");
    let author_node_id = engine.begin_work_item_plan_human_gate_revision_run().await;
    assert_eq!(
        engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == author_node_id)
            .expect("revision author node")
            .status,
        crate::web::workspace_ws_types::TimelineNodeStatus::Active
    );

    engine
        .fail_human_gate_turn(
            &turn_id,
            crate::product::models::HumanGateTurnFailureClass::ProviderErr,
        )
        .await
        .expect("fail turn");

    let failed_node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == author_node_id)
        .expect("revision author node after failure");
    assert_eq!(
        failed_node.status,
        crate::web::workspace_ws_types::TimelineNodeStatus::Failed,
        "failing the turn must close the revision author node (timeline phantom)"
    );
    assert!(
        failed_node.completed_at.is_some(),
        "closed author node must carry completed_at"
    );
    // durable turn 侧同步终态化。
    let durable_turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("durable turn after failure");
    assert_eq!(
        durable_turn.status,
        crate::product::models::HumanGateTurnStatus::Failed
    );
}

// ===========================================================================
// SG6b（plan v1.1 Task SG6b / REQ-CG-03:74、REQ-CG-05、REQ-SG-03/04A/05）：
// 复评旧路由防旁路与 anchor 恢复 join。SG6a.3 移交的 verdict 面收口
// （turn 仍 Running/gate_busy 至原 reviewer 收口；grant Resolved finalize）；
// 终态 grant tombstone 拒迟到 review（两 attempt 满后 review Revise 第三
// start 0、single_candidate_author key/reservation/repairs_used 增量 0）；
// 同 grant route 先于旧 policy/repair reservation（L1 剩 attempt2 同 key，
// L2/business/unknown 无新许可）；显式 fresh 同 attempt None-resume/子
// prompt（错 recovery/digest/ref/closed/source 漂移 0 start/预算不改写）。
// ===========================================================================

/// 带 Repairable class_hint finding 的 Revise verdict——旧 policy 路由面
/// （TriggerAggregateRepair）的唯一可复现触发形态。
fn sg06b_repairable_revise_verdict() -> crate::web::workspace_ws_types::ReviewVerdict {
    use crate::product::work_item_plan_policy::{FindingClassHint, ReviewFindingCategory};
    use crate::web::workspace_ws_types::{ReviewFinding, ReviewFindingSeverity};
    crate::web::workspace_ws_types::ReviewVerdict {
        verdict: crate::web::workspace_ws_types::ReviewVerdictType::Revise,
        comments: "结构位置需要修复".to_string(),
        summary: "结构位置需要修复".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "WI-001 的 provided_contract_refs 缺少消费者".to_string(),
            evidence: String::new(),
            required_action: "补齐 contract.levels-integration 的消费者".to_string(),
            category: Some(ReviewFindingCategory::ContractGap),
            class_hint: Some(FindingClassHint::Repairable),
            contract_field: Some("work_item_plan.wi_001".to_string()),
            machine_repair: None,
            owner_question: None,
            owner_options: None,
        }],
        review_gate: crate::web::workspace_ws_types::ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

/// SG6b 主验收①：REQ-CG-03:74「修订成功回呈」的 verdict 面全语义——
/// 合法交付后 turn 仍 Running/gate_busy（grant=Reviewing，DeliveredCharge
/// 恰一次）；原 reviewer verdict 收口后 turn 才 Completed、grant 携 review
/// 证据终态 Resolved、回等待门（Pass 原门）；终态 tombstone 拒迟到
/// review（Revise 第三 start 0：零 durable 写、零 single_candidate_author
/// key/reservation/repairs_used 增量）。
#[tokio::test]
async fn sg_06b_reviewing_recovery_and_old_policy_guard() {
    use crate::product::models::workspace::StructuralGrantPhase;
    use crate::product::models::{
        HumanGateTurnStatus, UserBudgetReceiptKind, WorkspaceSessionRecord,
    };

    let (_root, lifecycle, mut engine) = evaluate_gate_revision_fixture("sg06b_verdict_face", 3, 1);
    let session_id = engine.session().session_id.clone();
    engine
        .enter_human_confirm(Some("SG6b 复评收口门".to_string()))
        .await;

    // 门内反馈 → attempt1 即合法交付 → settle DeliveredCharge 恰一次、
    // grant→Reviewing；turn 仍 Running/gate_busy（REQ-CG-03:74）。
    let turn_id = open_running_revision_turn(&mut engine, "sg06b_verdict_command").await;
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("legal delivery completes");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    let turn_in_review = lifecycle
        .get_human_gate_turn(&session_id, &turn_id)
        .expect("turn after delivery");
    assert_eq!(
        turn_in_review.status,
        HumanGateTurnStatus::Running,
        "REQ-CG-03:74：合法交付后 turn 仍 Running，verdict 收口才 Completed"
    );
    assert!(turn_in_review.result_artifact_ref.is_some());
    let state_in_review = sg63_state(&lifecycle, &session_id).expect("state after delivery");
    let grant_in_review = state_in_review.current_grant.as_ref().expect("grant");
    assert_eq!(grant_in_review.phase, Some(StructuralGrantPhase::Reviewing));
    assert_eq!(
        state_in_review
            .user_receipts
            .iter()
            .filter(|receipt| receipt.turn_id == turn_id
                && receipt.kind == UserBudgetReceiptKind::DeliveredCharge)
            .count(),
        1,
        "deliver settlement 与终态分离：DeliveredCharge 恰一次"
    );
    // 原 reviewer verdict（Pass）收口：turn Completed + grant 携 review 证据
    // 终态 Resolved + 回等待门（原门，不自动 confirm）。
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "review pass".to_string(),
                None,
            ),
            pass_revision_review_verdict(),
        )
        .await;
    let turn_final = lifecycle
        .get_human_gate_turn(&session_id, &turn_id)
        .expect("turn after verdict");
    assert_eq!(turn_final.status, HumanGateTurnStatus::Completed);
    let state_final = sg63_state(&lifecycle, &session_id).expect("state after verdict");
    let grant_final = state_final
        .current_grant
        .as_ref()
        .expect("grant after verdict");
    assert_eq!(
        grant_final.phase,
        Some(StructuralGrantPhase::Resolved),
        "verdict 收口后 grant 携 review 证据终态 Resolved"
    );
    assert!(
        grant_final.review_identity.is_some(),
        "Resolved 必须绑定 review join 证据（review_identity）"
    );
    assert_eq!(
        engine.session().stage,
        crate::product::workspace_engine::WorkspaceStage::HumanConfirm,
        "Pass 原门：收口后回等待门，不自动 confirm"
    );

    // 终态 tombstone：迟到 review verdict（Revise+repairable，旧路由唯一
    // 旁路形态）全量拒收——零 durable 写（第三 author start 0、
    // single_candidate_author key/reservation/repairs_used 增量 0）。
    let before: WorkspaceSessionRecord = lifecycle
        .get_workspace_session(&session_id)
        .expect("session before late verdict");
    let ledger_len_before = before.provider_start_ledger.len();
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "late revise".to_string(),
                None,
            ),
            sg06b_repairable_revise_verdict(),
        )
        .await;
    let after = lifecycle
        .get_workspace_session(&session_id)
        .expect("session after late verdict");
    assert_eq!(
        after, before,
        "终态 grant tombstone：迟到 review 零 durable 写（含 ledger/reservation/run_history）"
    );
    assert_eq!(after.provider_start_ledger.len(), ledger_len_before);
    assert!(
        !after.provider_start_ledger.iter().any(|entry| entry
            .provider_start_idempotency_key
            .starts_with("single_candidate_author:")),
        "无 single_candidate_author 预领键（第三 start 0）"
    );
    assert_eq!(after.run_history.repairs_used, 0, "repairs_used 增量 0");
}

/// SG6b 主验收②：两 attempt 满后的迟到 review 全量拒收（同 tombstone 域，
/// 耗尽形态）＋ L1 剩 attempt2 窗口（grant=Reviewing）内 review Revise 的
/// 旧 policy 自动返修旁路拦截（同 grant route 先于旧 policy/repair
/// reservation：EnterHumanGate 原门、零 author start、L2/business/unknown
/// 无新许可；另一合法重组＝新反馈 turn 允许开启）。
#[tokio::test]
async fn sg_06b_review_revise_routes_gate_without_third_author_start() {
    use crate::product::models::workspace::StructuralGrantPhase;
    use crate::product::models::{
        HumanGateTurnStatus, SingleCandidatePhase, WorkspaceSessionRecord,
    };

    // —— 形态 A：两 attempt 满（grant SystemFault）后的迟到 Revise ——
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06b_exhausted_tombstone", 3);
    let session_id = engine.session().session_id.clone();
    let turn_id = open_running_revision_turn(&mut engine, "sg06b_r1_command").await;
    let cycle = sg61_cycle_candidate();
    let attempt1 = engine
        .run_sc_manual_revision_turn(&turn_id, cycle.clone())
        .await
        .expect("attempt1 rejection is in-band");
    assert!(matches!(
        &attempt1,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
            claim: Some(_),
            ..
        }
    ));
    let attempt2 = engine
        .run_sc_manual_revision_turn(&turn_id, cycle)
        .await
        .expect("exhaustion stop is in-band");
    assert!(matches!(
        &attempt2,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
            claim: None,
            ..
        }
    ));
    let exhausted_turn = lifecycle
        .get_human_gate_turn(&session_id, &turn_id)
        .expect("turn after exhaustion");
    assert_eq!(exhausted_turn.status, HumanGateTurnStatus::Failed);
    let exhausted_state = sg63_state(&lifecycle, &session_id).expect("state after exhaustion");
    assert_eq!(
        exhausted_state.current_grant.as_ref().expect("grant").phase,
        Some(StructuralGrantPhase::SystemFault)
    );
    let before: WorkspaceSessionRecord = lifecycle
        .get_workspace_session(&session_id)
        .expect("session before late exhausted review");
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "late revise".to_string(),
                None,
            ),
            sg06b_repairable_revise_verdict(),
        )
        .await;
    let after = lifecycle
        .get_workspace_session(&session_id)
        .expect("session after late exhausted review");
    assert_eq!(
        after, before,
        "两 attempt 满后迟到 review Revise 全量拒收：第三 start 0、零账本/预算写"
    );
    assert!(
        !after.provider_start_ledger.iter().any(|entry| entry
            .provider_start_idempotency_key
            .starts_with("single_candidate_author:")),
        "无 single_candidate_author 预领键"
    );
    assert_eq!(after.run_history.repairs_used, 0);

    // —— 形态 B：L1 剩 attempt2 窗口（attempt1 即交付，grant=Reviewing）
    // 内的 review Revise —— 同 grant 域拦截旧 policy 自动返修 ——
    let (_root2, lifecycle2, mut engine2) =
        evaluate_gate_revision_fixture("sg06b_reviewing_window", 3, 1);
    let session2 = engine2.session().session_id.clone();
    engine2
        .enter_human_confirm(Some("SG6b 复评窗口门".to_string()))
        .await;
    let turn2 = open_running_revision_turn(&mut engine2, "sg06b_window_command").await;
    assert!(matches!(
        engine2
            .run_sc_manual_revision_turn(&turn2, handoff_clean_rep4_v2())
            .await
            .expect("window delivery"),
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    engine2
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "revise with repairable findings".to_string(),
                None,
            ),
            sg06b_repairable_revise_verdict(),
        )
        .await;
    let window_session = lifecycle2
        .get_workspace_session(&session2)
        .expect("session after window revise");
    assert!(
        !window_session
            .provider_start_ledger
            .iter()
            .any(|entry| entry
                .provider_start_idempotency_key
                .starts_with("single_candidate_author:")),
        "同 grant route 先于旧 policy/repair reservation：零预领键"
    );
    assert!(
        window_session.repair_reservation.is_none(),
        "零 repair reservation"
    );
    assert_eq!(
        window_session.run_history.repairs_used, 0,
        "repairs_used 增量 0（用户 turn 及其 L1 结构 attempt 不入旧计数）"
    );
    assert_eq!(
        engine2.session().stage,
        crate::product::workspace_engine::WorkspaceStage::HumanConfirm,
        "Revise 改道原人工门（L2 退役语义），零 author start"
    );
    assert_eq!(
        window_session.single_candidate_phase,
        Some(SingleCandidatePhase::Approval),
        "EnterHumanGate 原门：phase 回 Approval 等待门"
    );
    let window_turn = lifecycle2
        .get_human_gate_turn(&session2, &turn2)
        .expect("window turn after revise");
    assert_eq!(window_turn.status, HumanGateTurnStatus::Completed);
    let window_state = sg63_state(&lifecycle2, &session2).expect("window state");
    assert_eq!(
        window_state.current_grant.as_ref().expect("grant").phase,
        Some(StructuralGrantPhase::Resolved),
        "Revise verdict 同样收口 grant（终态互斥，等待新反馈轮）"
    );

    // 另一合法重组允许：verdict 收口回等待门后，新反馈 turn 可开启
    //（不因 R3 同失败历史/全谓词阻断而卡死）。
    let reopened = engine2
        .handle_human_gate_feedback(super::conversational_gate::feedback(
            "sg06b_next_round_command",
        ))
        .await
        .expect("next feedback round");
    assert!(
        matches!(reopened, HumanGateCommandOutcome::TurnOpened { .. }),
        "另一合法重组：verdict 收口后新反馈轮允许开启，got {reopened:?}"
    );
}

/// SG6b 主验收③（anchor 恢复 join 的引擎面）：显式 fresh 同 attempt
/// None-resume/子 prompt——attempt1 结构拒收已 claim attempt2（grant=
/// Attempt2Claimed）后，fresh 重入交付冻结机器指令子 prompt（确定性
/// 重建＝claim 绑定身份，逐字节等于冻结全文）；错 digest（grant 绑定
/// 身份漂移）→ 稳定码拒绝、零 start、预算不改写。
#[tokio::test]
async fn sg_06b_explicit_fresh_delivers_frozen_subprompt_same_attempt() {
    use crate::product::json_store::write_json;
    use crate::product::models::HumanGateTurnStatus;
    use crate::product::models::workspace::StructuralGrantPhase;

    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06b_fresh_subprompt", 3);
    let session_id = engine.session().session_id.clone();
    let turn_id = open_running_revision_turn(&mut engine, "sg06b_fresh_command").await;
    let cycle = sg61_cycle_candidate();
    let rejected = engine
        .run_sc_manual_revision_turn(&turn_id, cycle)
        .await
        .expect("attempt1 rejection is in-band");
    let carry = match rejected {
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
            claim: Some(carry),
            ..
        } => carry,
        other => panic!("expected claimed rejection, got {other:?}"),
    };
    let grant_before = sg61_current_grant(&lifecycle, &session_id).expect("claimed grant");
    assert_eq!(
        grant_before.phase,
        Some(StructuralGrantPhase::Attempt2Claimed)
    );

    // 显式 fresh（同 attempt、resume=None 的全新 native）：fresh builder
    // 在结构窗口内必须交付冻结子 prompt，而非复造原反馈 prompt。
    let turn = lifecycle
        .get_human_gate_turn(&session_id, &turn_id)
        .expect("claimed turn");
    let fresh_prompt = engine
        .build_sc_manual_revision_prompt_for_fresh_turn(&turn)
        .expect("explicit fresh must deliver the frozen machine-instruction sub-prompt");
    assert_eq!(
        fresh_prompt, carry.frozen.full_prompt,
        "fresh 子 prompt＝冻结全文逐字节（确定性重建＝claim 绑定身份）"
    );

    // 错 digest：篡改 grant.prompt_digest（身份漂移）→ 稳定码拒绝、
    // 零 start、预算不改写（turn/ledger/receipt 原样）。
    let session_path = lifecycle
        .app_paths()
        .issue_root(
            &lifecycle
                .get_workspace_session(&session_id)
                .expect("session")
                .project_id,
            &lifecycle
                .get_workspace_session(&session_id)
                .expect("session")
                .issue_id,
        )
        .join("workspace-sessions")
        .join(format!("{session_id}.json"));
    let mut drifted = lifecycle
        .get_workspace_session(&session_id)
        .expect("session for drift seed");
    {
        let state = drifted
            .structural_governance_state
            .as_mut()
            .expect("state present after claim");
        let grant = state
            .current_grant
            .as_mut()
            .expect("grant present after claim");
        grant.prompt_digest = Some("e".repeat(64));
    }
    write_json(&session_path, &drifted).expect("persist drifted grant identity");
    let before_drift = lifecycle
        .get_workspace_session(&session_id)
        .expect("session before drifted fresh");
    let drifted_result = engine
        .build_sc_manual_revision_prompt_for_fresh_turn(&turn)
        .expect_err("drifted grant identity must reject the fresh delivery");
    assert!(
        drifted_result.starts_with("STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH"),
        "稳定码拒绝：got {drifted_result}"
    );
    let after_drift = lifecycle
        .get_workspace_session(&session_id)
        .expect("session after drifted fresh");
    assert_eq!(
        after_drift, before_drift,
        "错 digest 拒绝零 durable 写（0 start、预算不改写）"
    );
    let turn_after = lifecycle
        .get_human_gate_turn(&session_id, &turn_id)
        .expect("turn after drifted fresh");
    assert_eq!(turn_after.status, HumanGateTurnStatus::Running);
}

/// SG6b P2 修复轮（Main 审查派回）：claim→冻结件落盘的崩溃窗——
/// 置位 hook 模拟「claim 已落、冻结件写点丢失」；冻结证据必须已在
/// durable claim **之前**内容寻址持久化，否则 fresh 恢复链 load 不到→
/// CANDIDATE_DRIFTED 0 start、turn 永卡 Running+Attempt2Claimed。
#[tokio::test]
async fn sg_06b_claim_persist_crash_window_keeps_frozen_evidence() {
    use crate::product::models::workspace::StructuralGrantPhase;
    use crate::product::workspace_engine::conversational_gate::SG6B_CLAIM_PERSIST_CRASH;

    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06b_persist_crash_window", 3);
    let session_id = engine.session().session_id.clone();
    let turn_id = open_running_revision_turn(&mut engine, "sg06b_crash_command").await;
    let cycle = sg61_cycle_candidate();

    SG6B_CLAIM_PERSIST_CRASH.store(true, std::sync::atomic::Ordering::SeqCst);
    let rejected = engine
        .run_sc_manual_revision_turn(&turn_id, cycle)
        .await
        .expect("claim 崩溃窗内仍 in-band 拒收");
    assert!(matches!(
        rejected,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected { .. }
    ));

    let grant = sg61_current_grant(&lifecycle, &session_id).expect("claim 已落");
    assert_eq!(grant.phase, Some(StructuralGrantPhase::Attempt2Claimed));
    let prompt_ref = grant.prompt_ref.as_deref().expect("claim 绑定 prompt 身份");
    let digest_hex = prompt_ref.trim_start_matches("structural-prompt://");
    let record = lifecycle
        .get_workspace_session(&session_id)
        .expect("session");
    let frozen_path = lifecycle
        .app_paths()
        .issue_lifecycle_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(&session_id)
        .join("structural-prompts")
        .join(format!("{digest_hex}.json"));
    assert!(
        frozen_path.is_file(),
        "P2：冻结证据必须在 durable claim 之前已持久化（崩溃窗不丢唯一写点）：{frozen_path:?}"
    );
}
