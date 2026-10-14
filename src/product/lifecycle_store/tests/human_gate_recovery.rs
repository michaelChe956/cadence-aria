use crate::product::models::{HumanGateReservation, HumanGateTurn, HumanGateTurnStatus};

#[test]
fn human_gate_recovery_attempt_updates_same_turn_and_terminal_replay_reserves_again() {
    let (_tmp, store) = setup();
    let mut session = create_session(
        &store,
        "work_item_plan_attempt",
        WorkspaceType::WorkItemPlan,
    );
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    });
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).unwrap();

    let make_turn = |turn_id: &str, command_id: &str, status| HumanGateTurn {
        turn_id: turn_id.to_string(),
        session_id: session.id.clone(),
        command_id: command_id.to_string(),
        feedback_text: "recover provider".to_string(),
        status,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    let reservation = |turn: &HumanGateTurn| HumanGateReservation {
        command_id: turn.command_id.clone(),
        turn_id: turn.turn_id.clone(),
        provider_start_idempotency_key: format!("human_gate:{}:attempt:1", turn.turn_id),
        reserved_at: turn.created_at.clone(),
    };

    let first_reserved = make_turn(
        "turn_attempt_0001",
        "command_attempt_0001",
        HumanGateTurnStatus::Reserved,
    );
    let (reserved, _) = store
        .compare_and_reserve_human_gate_turn(
            &session,
            first_reserved.clone(),
            reservation(&first_reserved),
        )
        .unwrap();
    let mut running = first_reserved.clone();
    running.status = HumanGateTurnStatus::Running;
    let running_session = store
        .update_human_gate_turn(&reserved, running.clone())
        .unwrap();
    assert_eq!(
        store
            .get_human_gate_turn(&session.id, &first_reserved.turn_id)
            .unwrap()
            .status,
        HumanGateTurnStatus::Running
    );
    assert_eq!(running_session.provider_start_ledger.len(), 1);
    let mut resumed = running.clone();
    resumed.attempt_no = 2;
    let resumed_session = store
        .update_human_gate_turn(&running_session, resumed.clone())
        .unwrap();
    assert_eq!(
        store
            .get_human_gate_turn(&session.id, &first_reserved.turn_id)
            .unwrap(),
        resumed
    );
    assert_eq!(
        resumed_session
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        1
    );
    assert_eq!(resumed_session.provider_start_ledger.len(), 2);
    assert!(resumed_session.provider_start_ledger.iter().any(|entry| {
        entry.provider_start_idempotency_key == "human_gate:turn_attempt_0001:attempt:2"
    }));

    let mut failed = resumed.clone();
    failed.status = HumanGateTurnStatus::Failed;
    failed.failure_class = Some(crate::product::models::HumanGateTurnFailureClass::ProviderErr);
    let terminal_session = store
        .update_human_gate_turn(&resumed_session, failed.clone())
        .unwrap();
    let second_reserved = make_turn(
        "turn_attempt_0002",
        "command_attempt_0002",
        HumanGateTurnStatus::Reserved,
    );
    let (second_session, second_turn) = store
        .compare_and_reserve_human_gate_turn(
            &terminal_session,
            second_reserved.clone(),
            reservation(&second_reserved),
        )
        .unwrap();
    assert_eq!(second_turn.turn_id, second_reserved.turn_id);
    assert_eq!(
        second_session
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        0
    );
    assert_eq!(
        store
            .get_human_gate_turn_by_command_id(&session.id, &first_reserved.command_id)
            .unwrap(),
        Some(failed)
    );

    let nonterminal = make_turn(
        "turn_attempt_0003",
        "command_attempt_0003",
        HumanGateTurnStatus::Reserved,
    );
    assert!(matches!(
        store.compare_and_reserve_human_gate_turn(
            &second_session,
            nonterminal.clone(),
            reservation(&nonterminal),
        ),
        Err(ProductStoreError::Conflict {
            kind: "human_gate_reservation",
            ..
        })
    ));
}

// === SG4a（plan v1.1 Task SG4a）：hold/receipt/attempt 与真实启动账本 ===
// REQ-CG-02/03/05、REQ-SG-05：可用预算 = 总额 − delivered_charge −
// nonstructural_failure_charge − 活动 hold；reserve 只 hold 不标交付；
// claim/settle/release 按 turn 恰一次、三终态互斥；attempt L0/L1 共用同
// turn key 无第 3 次；旧记录零回填、旧 started 不冒充实际启动。

use crate::product::models::{
    StructuralAttemptReason, StructuralClaimOutcome, StructuralGovernanceState,
    StructuralGrantPhase, StructuralRevisionGrant, UserBudgetReceipt, UserBudgetReceiptKind,
};
use crate::product::work_item_plan_policy::ProviderStartLedgerEntry as Sg04aLedgerEntryPlaceholder;
use crate::product::workspace_engine::contract_prerevision::{
    ProtectedBusinessFacts, StructuralRepairInstruction, StructuralRepairSolution,
};

fn sg_04a_grant(
    turn_id: Option<&str>,
    command_id: Option<&str>,
    attempt_no: u32,
    phase: Option<StructuralGrantPhase>,
) -> StructuralRevisionGrant {
    StructuralRevisionGrant {
        grant_id: "sgr_sg4a_0001".to_string(),
        authorization_hash: "a".repeat(64),
        turn_id: turn_id.map(str::to_string),
        command_id: command_id.map(str::to_string),
        candidate_source_revision_ref: Some("source_revision_sg4a".to_string()),
        plan_candidate_ir_ref: Some("plan_candidate_ir_sg4a".to_string()),
        mechanical_report_ref: Some("mechanical_report_sg4a".to_string()),
        phase,
        attempt_no,
        prompt_ref: None,
        prompt_digest: None,
        instruction_digest: None,
        failing_source_hash: None,
        review_identity: None,
        granted_at: "2026-10-13T00:00:00Z".to_string(),
    }
}

fn sg_04a_governance_state(grant: Option<StructuralRevisionGrant>) -> StructuralGovernanceState {
    StructuralGovernanceState {
        current_grant: grant,
        ..StructuralGovernanceState::default()
    }
}

fn sg_04a_instruction() -> StructuralRepairInstruction {
    StructuralRepairInstruction {
        repair_id: "repair_sg4a_0001".to_string(),
        failure_id: "failure_sg4a_0001".to_string(),
        rule_version: "structure_rules/v1".to_string(),
        source_hash: "b".repeat(64),
        context_digest: "c".repeat(64),
        affected_work_item_ids: vec!["work_item_0001".to_string()],
        affected_spans: Vec::new(),
        conflict_edges: Vec::new(),
        allowed_change_closure: vec!["heading".to_string()],
        forbidden_operations: vec!["delete_required_inputs".to_string()],
        protected_facts: ProtectedBusinessFacts {
            business_request_digest: "d".repeat(64),
            confirmed_design_digest: None,
            protected_field_refs: Vec::new(),
            protected_contract_refs: Vec::new(),
            authorized_member_ids: Vec::new(),
        },
        solution: StructuralRepairSolution::Candidates {
            candidates: Vec::new(),
        },
        required_action: "修复非法 heading".to_string(),
        expected_predicates: Vec::new(),
        previous_valid_refs: None,
        instruction_digest: "e".repeat(64),
    }
}

fn sg_04a_receipt(
    turn_id: &str,
    command_id: &str,
    kind: UserBudgetReceiptKind,
) -> UserBudgetReceipt {
    UserBudgetReceipt {
        kind,
        turn_id: turn_id.to_string(),
        command_id: command_id.to_string(),
        attempt_reason: Some(StructuralAttemptReason::HumanRevision),
        source_revision_ref: Some("source_revision_sg4a".to_string()),
        plan_candidate_ir_ref: Some("plan_candidate_ir_sg4a".to_string()),
        structure_report_ref: Some("mechanical_report_sg4a".to_string()),
        context_digest: Some("c".repeat(64)),
        reason: None,
        settled_at: "2026-10-13T01:00:00Z".to_string(),
    }
}

fn sg_04a_session(
    store: &LifecycleStore,
    entity_id: &str,
    state: Option<StructuralGovernanceState>,
    manual_repairs_remaining: u32,
) -> crate::product::models::WorkspaceSessionRecord {
    let mut session = create_session(store, entity_id, WorkspaceType::WorkItemPlan);
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    });
    session.structural_governance_state = state;
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).unwrap();
    session
}

fn sg_04a_make_turn(
    session_id: &str,
    turn_id: &str,
    command_id: &str,
    status: HumanGateTurnStatus,
) -> HumanGateTurn {
    HumanGateTurn {
        turn_id: turn_id.to_string(),
        session_id: session_id.to_string(),
        command_id: command_id.to_string(),
        feedback_text: "revise structure".to_string(),
        status,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:00Z".to_string(),
    }
}

fn sg_04a_reservation(turn: &HumanGateTurn) -> HumanGateReservation {
    HumanGateReservation {
        command_id: turn.command_id.clone(),
        turn_id: turn.turn_id.clone(),
        provider_start_idempotency_key: format!("human_gate:{}:attempt:1", turn.turn_id),
        reserved_at: turn.created_at.clone(),
    }
}

#[test]
fn sg_04a_user_receipt_and_attempt() {
    let (_tmp, store) = setup();
    let session = sg_04a_session(
        &store,
        "sg4a_ledger",
        Some(sg_04a_governance_state(Some(sg_04a_grant(
            Some("turn_sg4a_0001"),
            Some("command_sg4a_0001"),
            0,
            Some(StructuralGrantPhase::AuthorRunning),
        )))),
        3,
    );

    // ① reserve：接受真实反馈＝durable hold 立即占用，可用 3→2，held 非
    //    已交付。
    let turn1 = sg_04a_make_turn(
        &session.id,
        "turn_sg4a_0001",
        "command_sg4a_0001",
        HumanGateTurnStatus::Reserved,
    );
    let (reserved, _) = store
        .compare_and_reserve_human_gate_turn(&session, turn1.clone(), sg_04a_reservation(&turn1))
        .unwrap();
    let state = reserved.structural_governance_state.as_ref().unwrap();
    assert_eq!(state.available_user_budget(), 2, "总额3−活动hold1=2");
    assert_eq!(
        state
            .active_hold_for_turn("turn_sg4a_0001")
            .map(|hold| hold.command_id.as_str()),
        Some("command_sg4a_0001")
    );
    assert_eq!(state.delivered_charges(), 0, "hold 不是已交付");
    assert!(state.receipt_for_turn("turn_sg4a_0001").is_none());
    assert_eq!(
        reserved
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        2
    );

    // duplicate reserve 恰一次：重放不重复 hold、不重复扣减。
    let (replayed_reserve, _) = store
        .compare_and_reserve_human_gate_turn(&reserved, turn1.clone(), sg_04a_reservation(&turn1))
        .unwrap();
    let replay_state = replayed_reserve
        .structural_governance_state
        .as_ref()
        .unwrap();
    assert_eq!(replay_state.active_hold_count(), 1);
    assert_eq!(replay_state.available_user_budget(), 2);
    assert_eq!(
        replayed_reserve
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        2
    );

    // ② turn Running 后 claim：L0 attempt1 在跑，L1 分配物理 attempt2（同
    //    turn attempt key）。
    let mut running = turn1.clone();
    running.status = HumanGateTurnStatus::Running;
    let running_session = store
        .update_human_gate_turn(&replayed_reserve, running)
        .unwrap();
    let instruction = sg_04a_instruction();
    let claimed = store
        .claim_structural_attempt(
            &running_session,
            "turn_sg4a_0001",
            &instruction,
            "prompt_ref_sg4a_0001",
            "digest_sg4a_0001",
        )
        .unwrap();
    let StructuralClaimOutcome::Claimed {
        attempt_no,
        prompt_ref,
        prompt_digest,
        provider_start_key,
    } = &claimed
    else {
        panic!("首次 claim 必须 Claimed，实际 {claimed:?}");
    };
    assert_eq!(*attempt_no, 2);
    assert_eq!(provider_start_key, "human_gate:turn_sg4a_0001:attempt:2");
    assert_eq!(prompt_ref, "prompt_ref_sg4a_0001");
    assert_eq!(prompt_digest, "digest_sg4a_0001");

    // claim 与真实 start 分账：不代写 provider_start_ledger。
    let after_claim = store.get_workspace_session(&session.id).unwrap();
    assert!(
        !after_claim.provider_start_ledger.iter().any(|entry| {
            entry.provider_start_idempotency_key == "human_gate:turn_sg4a_0001:attempt:2"
        }),
        "claim 不代写真实启动账本"
    );
    let grant_after = after_claim
        .structural_governance_state
        .as_ref()
        .unwrap()
        .current_grant
        .as_ref()
        .unwrap();
    assert_eq!(grant_after.attempt_no, 2);
    assert_eq!(
        grant_after.phase,
        Some(StructuralGrantPhase::Attempt2Claimed)
    );
    assert_eq!(
        grant_after.prompt_digest.as_deref(),
        Some("digest_sg4a_0001")
    );
    assert_eq!(
        grant_after.instruction_digest.as_deref(),
        Some(instruction.instruction_digest.as_str())
    );
    assert_eq!(
        grant_after.failing_source_hash.as_deref(),
        Some(instruction.source_hash.as_str())
    );

    // duplicate claim 恰一次：同 prompt 重复 → Replayed，不加 attempt。
    assert_eq!(
        store
            .claim_structural_attempt(
                &after_claim,
                "turn_sg4a_0001",
                &instruction,
                "prompt_ref_sg4a_0001",
                "digest_sg4a_0001",
            )
            .unwrap(),
        StructuralClaimOutcome::Replayed
    );
    let unchanged = store.get_workspace_session(&session.id).unwrap();
    assert_eq!(
        unchanged
            .structural_governance_state
            .as_ref()
            .unwrap()
            .current_grant
            .as_ref()
            .unwrap()
            .attempt_no,
        2,
        "重复 claim 不加 attempt"
    );

    // ③ provider 失败结转：turn Failed(provider_err) → held 结转
    //    nonstructural_failure_charge，不退、不留永久 hold。
    let mut failed = turn1.clone();
    failed.status = HumanGateTurnStatus::Failed;
    failed.failure_class = Some(crate::product::models::HumanGateTurnFailureClass::ProviderErr);
    let failed_session = store.update_human_gate_turn(&unchanged, failed).unwrap();
    let mut failure_receipt = sg_04a_receipt(
        "turn_sg4a_0001",
        "command_sg4a_0001",
        UserBudgetReceiptKind::NonstructuralFailureCharge,
    );
    failure_receipt.reason = Some("provider_err".to_string());
    let settled_failure = store
        .settle_structural_user_budget(&failed_session, "turn_sg4a_0001", failure_receipt.clone())
        .unwrap();
    let failure_state = settled_failure
        .structural_governance_state
        .as_ref()
        .unwrap();
    assert_eq!(
        failure_state.available_user_budget(),
        2,
        "3 − nonstructural_failure_charge1 − hold0 = 2（失败不退）"
    );
    assert!(
        failure_state
            .active_hold_for_turn("turn_sg4a_0001")
            .is_none(),
        "不留永久 hold"
    );
    assert_eq!(failure_state.nonstructural_failure_charges(), 1);
    assert_eq!(failure_state.delivered_charges(), 0, "不假标已交付");
    assert_eq!(
        settled_failure
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        2
    );
    assert_eq!(
        failure_state
            .receipt_for_turn("turn_sg4a_0001")
            .and_then(|receipt| receipt.reason.as_deref()),
        Some("provider_err"),
        "结算记精确 reason"
    );

    // duplicate settle 恰一次：同 receipt 重放零写。
    let replayed_settle = store
        .settle_structural_user_budget(&settled_failure, "turn_sg4a_0001", failure_receipt.clone())
        .unwrap();
    assert_eq!(replayed_settle, settled_failure);

    // 终态互斥：已结转后改判 delivered → Conflict（不退款、不二次结算）。
    let forged = sg_04a_receipt(
        "turn_sg4a_0001",
        "command_sg4a_0001",
        UserBudgetReceiptKind::DeliveredCharge,
    );
    assert!(matches!(
        store.settle_structural_user_budget(&settled_failure, "turn_sg4a_0001", forged),
        Err(ProductStoreError::Conflict { .. })
    ));

    // ④ 第二 turn：真实交付 delivered_charge；已 deliver 后复评失败不退。
    let turn2 = sg_04a_make_turn(
        &session.id,
        "turn_sg4a_0002",
        "command_sg4a_0002",
        HumanGateTurnStatus::Reserved,
    );
    let (reserved2, _) = store
        .compare_and_reserve_human_gate_turn(
            &settled_failure,
            turn2.clone(),
            sg_04a_reservation(&turn2),
        )
        .unwrap();
    assert_eq!(
        reserved2
            .structural_governance_state
            .as_ref()
            .unwrap()
            .available_user_budget(),
        1,
        "3 − failure1 − delivered0 − hold1 = 1"
    );
    let mut running2 = turn2.clone();
    running2.status = HumanGateTurnStatus::Running;
    let running2_session = store.update_human_gate_turn(&reserved2, running2).unwrap();
    let delivered_receipt = sg_04a_receipt(
        "turn_sg4a_0002",
        "command_sg4a_0002",
        UserBudgetReceiptKind::DeliveredCharge,
    );
    let settled_delivered = store
        .settle_structural_user_budget(
            &running2_session,
            "turn_sg4a_0002",
            delivered_receipt.clone(),
        )
        .unwrap();
    let delivered_state = settled_delivered
        .structural_governance_state
        .as_ref()
        .unwrap();
    assert_eq!(
        delivered_state.available_user_budget(),
        1,
        "3 − failure1 − delivered1 − hold0 = 1"
    );
    assert_eq!(delivered_state.delivered_charges(), 1);

    // 已 deliver 后复评失败：同 turn 再 settle 失败 → Conflict 不退款。
    let mut re_review_failure = sg_04a_receipt(
        "turn_sg4a_0002",
        "command_sg4a_0002",
        UserBudgetReceiptKind::NonstructuralFailureCharge,
    );
    re_review_failure.reason = Some("re_review_failed".to_string());
    assert!(matches!(
        store.settle_structural_user_budget(
            &settled_delivered,
            "turn_sg4a_0002",
            re_review_failure
        ),
        Err(ProductStoreError::Conflict { .. })
    ));

    // ⑤ 第三 turn：从未 deliver 的结构耗尽 → released_structural 恢复可用
    //    预算（不实扣用户轮次）。
    let mut completed2 = turn2.clone();
    completed2.status = HumanGateTurnStatus::Completed;
    completed2.result_artifact_ref = Some("artifact_sg4a_0002".to_string());
    let closed2 = store
        .update_human_gate_turn(&settled_delivered, completed2)
        .unwrap();
    let turn3 = sg_04a_make_turn(
        &session.id,
        "turn_sg4a_0003",
        "command_sg4a_0003",
        HumanGateTurnStatus::Reserved,
    );
    let (reserved3, _) = store
        .compare_and_reserve_human_gate_turn(&closed2, turn3.clone(), sg_04a_reservation(&turn3))
        .unwrap();
    assert_eq!(
        reserved3
            .structural_governance_state
            .as_ref()
            .unwrap()
            .available_user_budget(),
        0
    );
    let mut running3 = turn3.clone();
    running3.status = HumanGateTurnStatus::Running;
    let running3_session = store.update_human_gate_turn(&reserved3, running3).unwrap();
    let mut released_receipt = sg_04a_receipt(
        "turn_sg4a_0003",
        "command_sg4a_0003",
        UserBudgetReceiptKind::ReleasedStructural,
    );
    released_receipt.attempt_reason = Some(StructuralAttemptReason::StructuralRepair);
    released_receipt.reason = Some("structural_exhausted_before_delivery".to_string());
    let settled_release = store
        .settle_structural_user_budget(&running3_session, "turn_sg4a_0003", released_receipt)
        .unwrap();
    let release_state = settled_release
        .structural_governance_state
        .as_ref()
        .unwrap();
    assert_eq!(
        release_state.available_user_budget(),
        1,
        "released_structural 不实扣：3 − failure1 − delivered1 − hold0 = 1"
    );
    assert_eq!(release_state.charged_receipts(), 2, "release 不计费");
    assert!(
        release_state
            .active_hold_for_turn("turn_sg4a_0003")
            .is_none()
    );
    assert_eq!(
        settled_release
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        1,
        "快照可用预算同 CAS 恢复（0→1，不超总额）"
    );
}

#[test]
fn sg_04a_claim_guards_running_same_grant_and_shared_attempt_cap() {
    let (_tmp, store) = setup();
    let instruction = sg_04a_instruction();

    // ① 非 Running（Reserved）turn：claim 拒绝（attempt 推进仅 Running）。
    let session = sg_04a_session(
        &store,
        "sg4a_claim_guard",
        Some(sg_04a_governance_state(Some(sg_04a_grant(
            Some("turn_sg4a_guard"),
            Some("command_sg4a_guard"),
            0,
            Some(StructuralGrantPhase::AuthorRunning),
        )))),
        3,
    );
    let turn = sg_04a_make_turn(
        &session.id,
        "turn_sg4a_guard",
        "command_sg4a_guard",
        HumanGateTurnStatus::Reserved,
    );
    let (reserved, _) = store
        .compare_and_reserve_human_gate_turn(&session, turn.clone(), sg_04a_reservation(&turn))
        .unwrap();
    assert!(matches!(
        store.claim_structural_attempt(
            &reserved,
            "turn_sg4a_guard",
            &instruction,
            "prompt_ref_guard",
            "digest_guard"
        ),
        Err(ProductStoreError::Conflict {
            kind: "human_gate_turn",
            ..
        })
    ));

    // ② 异 turn claim：grant 绑定不同 turn → IdentityMismatch。
    assert!(matches!(
        store.claim_structural_attempt(
            &reserved,
            "turn_sg4a_other",
            &instruction,
            "prompt_ref_guard",
            "digest_guard"
        ),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));

    // ③ grant 无 turn 绑定 → Waiting（无 claim 权，零写）。
    // SG6a.3（Main 预授权迁移 2026-10-14）：复审④ grant producer 落地后
    // 真实 reserve 恒创建绑定本 turn 的 grant——未绑 grant 的 legacy 形
    // 态以 reserve 后 fixture 直写回置（SG4b 直写先例），Waiting 分支覆
    // 盖保持不变。
    let unbound = sg_04a_session(&store, "sg4a_claim_unbound", None, 3);
    let unbound_turn = sg_04a_make_turn(
        &unbound.id,
        "turn_sg4a_unbound",
        "command_sg4a_unbound",
        HumanGateTurnStatus::Reserved,
    );
    let (unbound_reserved, _) = store
        .compare_and_reserve_human_gate_turn(
            &unbound,
            unbound_turn.clone(),
            sg_04a_reservation(&unbound_turn),
        )
        .unwrap();
    let unbound_reserved = {
        let mut record = unbound_reserved.clone();
        record
            .structural_governance_state
            .get_or_insert_with(crate::product::models::StructuralGovernanceState::default)
            .current_grant = Some(sg_04a_grant(
            None,
            None,
            0,
            Some(StructuralGrantPhase::AuthorRunning),
        ));
        crate::product::json_store::write_json(
            &store
                .app_paths()
                .issue_lifecycle_root(&record.project_id, &record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist unbound-grant fixture");
        record
    };
    assert!(matches!(
        store.claim_structural_attempt(
            &unbound_reserved,
            "turn_sg4a_unbound",
            &instruction,
            "prompt_ref_guard",
            "digest_guard"
        ),
        Ok(StructuralClaimOutcome::Waiting { .. })
    ));

    // ④ L0 已耗 2 物理 attempt（Running 恢复到 attempt2）：L1 claim →
    //    Exhausted（无第 3 次），零写。
    let mut running = turn.clone();
    running.status = HumanGateTurnStatus::Running;
    let running_session = store.update_human_gate_turn(&reserved, running).unwrap();
    let mut attempt2 = turn.clone();
    attempt2.status = HumanGateTurnStatus::Running;
    attempt2.attempt_no = 2;
    let attempt2_session = store
        .update_human_gate_turn(&running_session, attempt2)
        .unwrap();
    assert_eq!(
        store
            .claim_structural_attempt(
                &attempt2_session,
                "turn_sg4a_guard",
                &instruction,
                "prompt_ref_guard",
                "digest_guard"
            )
            .unwrap(),
        StructuralClaimOutcome::Exhausted
    );
    let untouched = store.get_workspace_session(&session.id).unwrap();
    let untouched_grant = untouched
        .structural_governance_state
        .as_ref()
        .unwrap()
        .current_grant
        .as_ref()
        .unwrap();
    assert_eq!(
        (untouched_grant.attempt_no, untouched_grant.phase),
        (0, Some(StructuralGrantPhase::AuthorRunning)),
        "Exhausted 零写：grant 不变"
    );

    // ⑤ BlockedByLcRecovery → Waiting（anchor waiting 优先）；Reviewing →
    //    Conflict（评审中无新 attempt）。
    for (entity, phase, expect_conflict) in [
        (
            "sg4a_phase_blocked",
            StructuralGrantPhase::BlockedByLcRecovery,
            false,
        ),
        (
            "sg4a_phase_reviewing",
            StructuralGrantPhase::Reviewing,
            true,
        ),
    ] {
        let phase_session = sg_04a_session(
            &store,
            entity,
            Some(sg_04a_governance_state(Some(sg_04a_grant(
                Some("turn_sg4a_phase"),
                Some("command_sg4a_phase"),
                0,
                Some(phase),
            )))),
            3,
        );
        let phase_turn = sg_04a_make_turn(
            &phase_session.id,
            "turn_sg4a_phase",
            "command_sg4a_phase",
            HumanGateTurnStatus::Reserved,
        );
        let (phase_reserved, _) = store
            .compare_and_reserve_human_gate_turn(
                &phase_session,
                phase_turn.clone(),
                sg_04a_reservation(&phase_turn),
            )
            .unwrap();
        let mut phase_running = phase_turn.clone();
        phase_running.status = HumanGateTurnStatus::Running;
        let phase_running_session = store
            .update_human_gate_turn(&phase_reserved, phase_running)
            .unwrap();
        let outcome = store.claim_structural_attempt(
            &phase_running_session,
            "turn_sg4a_phase",
            &instruction,
            "prompt_ref_guard",
            "digest_guard",
        );
        if expect_conflict {
            assert!(matches!(
                outcome,
                Err(ProductStoreError::Conflict {
                    kind: "structural_revision_grant",
                    ..
                })
            ));
        } else {
            assert!(matches!(
                outcome,
                Ok(StructuralClaimOutcome::Waiting { .. })
            ));
        }
    }

    // ⑥ 身份载荷不完整（空 instruction digest / prompt）→ InvalidRecord。
    let mut bad_instruction = sg_04a_instruction();
    bad_instruction.instruction_digest = String::new();
    assert!(matches!(
        store.claim_structural_attempt(
            &attempt2_session,
            "turn_sg4a_guard",
            &bad_instruction,
            "prompt_ref_guard",
            "digest_guard"
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
    assert!(matches!(
        store.claim_structural_attempt(
            &attempt2_session,
            "turn_sg4a_guard",
            &instruction,
            "  ",
            "digest_guard"
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
}

#[test]
fn sg_04a_old_record_no_backfill_and_started_is_not_actual_launch() {
    let (_tmp, store) = setup();
    let session = sg_04a_session(&store, "sg4a_old_record", None, 2);
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));

    // 手写旧形态：ledger started=true 占位条目（无 provider/started_at）。
    let mut old = session.clone();
    old.provider_start_ledger = vec![Sg04aLedgerEntryPlaceholder {
        provider_start_idempotency_key: "human_gate:turn_sg4a_old:attempt:1".to_string(),
        started: true,
        provider: None,
        started_at: None,
    }];
    write_json(&session_path, &old).unwrap();

    // claim → Waiting（无聚合状态不补 grant/launch），盘上零回填。
    assert!(matches!(
        store.claim_structural_attempt(
            &old,
            "turn_sg4a_old",
            &sg_04a_instruction(),
            "prompt_ref_old",
            "digest_old"
        ),
        Ok(StructuralClaimOutcome::Waiting { .. })
    ));
    let reloaded = store.get_workspace_session(&session.id).unwrap();
    assert!(
        reloaded.structural_governance_state.is_none(),
        "旧记录零回填：不补 grant/launch"
    );

    // 旧 started=true 不冒充实际启动观察（launch 投影读 Unknown）。
    assert!(
        !reloaded.provider_start_ledger[0].is_actual_launch_observation(),
        "旧 started 布尔不是实际启动证据"
    );

    // settle 无聚合状态 → InvalidRecord（不凭空造账）。
    assert!(matches!(
        store.settle_structural_user_budget(
            &reloaded,
            "turn_sg4a_old",
            sg_04a_receipt(
                "turn_sg4a_old",
                "command_sg4a_old",
                UserBudgetReceiptKind::ReleasedStructural
            )
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));

    // 盘上 JSON 仍无 structural_governance_state 键。
    let raw: serde_json::Value = read_json(&session_path).unwrap();
    assert!(raw.get("structural_governance_state").is_none());
}
