#[tokio::test]
async fn conversational_gate_revision_result_success_returns_completed_turn_and_artifact_ref() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("revision_success", 2);
    let turn_id = open_running_revision_turn(&mut engine, "revision_success_command").await;
    let provider_output = format!(
        "provider preamble\n{}",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
        ))
    );

    let result = engine
        .run_sc_manual_revision_turn(&turn_id, provider_output)
        .await
        .expect("valid provider output should complete revision");
    let artifact_ref = match result {
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { artifact_ref } => {
            artifact_ref
        }
        other => panic!(
            "expected accepted revision, got unexpected outcome: {}",
            match other {
                crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. } =>
                    "accepted",
                crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
                    ..
                } => "validation_rejected",
            }
        ),
    };
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("completed turn");
    assert_eq!(
        turn.status,
        crate::product::models::HumanGateTurnStatus::Completed
    );
    assert_eq!(
        turn.result_artifact_ref.as_deref(),
        Some(artifact_ref.as_str())
    );
    assert!(
        engine
            .session()
            .work_item_plan_source_revision_ref
            .is_some()
    );
    assert!(engine.session().plan_candidate_ir_ref.is_some());
    assert!(engine.session().mechanical_report_ref.is_some());
    assert_eq!(
        engine.session().stage,
        crate::product::workspace_engine::WorkspaceStage::HumanConfirm
    );
    assert_eq!(
        engine.session().session_status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    assert_eq!(
        lifecycle
            .list_artifact_versions(engine.session().session_id.as_str())
            .expect("artifact versions")
            .iter()
            .filter(|version| version.is_current)
            .count(),
        1
    );
}

#[tokio::test]
async fn conversational_gate_revision_result_validation_reject_preserves_candidate() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("revision_reject", 3);
    let first_turn_id = open_running_revision_turn(&mut engine, "revision_seed_command").await;
    engine
        .run_sc_manual_revision_turn(
            &first_turn_id,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
            ))
            .to_string(),
        )
        .await
        .expect("seed revision should succeed");
    let before_session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session before rejection");
    let before_artifact = engine.session().artifact.clone();
    let before_artifact_versions = serde_json::to_vec(
        &lifecycle
            .list_artifact_versions(engine.session().session_id.as_str())
            .expect("artifact versions before rejection"),
    )
    .expect("serialize artifact versions before rejection");
    let before_ledger = before_session.provider_start_ledger.clone();
    let before_budget = before_session
        .human_gate_snapshot
        .as_ref()
        .expect("gate snapshot")
        .manual_repairs_remaining;
    let turn_id = open_running_revision_turn(&mut engine, "revision_reject_command").await;
    crate::product::workspace_engine::reset_artifact_constraint_spec_call_count();
    let reserved_session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after reservation");
    let before_candidate_refs = (
        reserved_session.work_item_plan_source_revision_ref.clone(),
        reserved_session.plan_candidate_ir_ref.clone(),
        reserved_session.mechanical_report_ref.clone(),
    );

    let result = engine
        .run_sc_manual_revision_turn(
            &turn_id,
            "# Work Item Plan\n\nnot a complete candidate\n".to_string(),
        )
        .await
        .expect("invalid provider output should be a validation rejection");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected { .. }
    ));
    assert_eq!(
        crate::product::workspace_engine::artifact_constraint_spec_call_count(),
        0,
        "validation reject must not consult legacy artifact constraints"
    );
    let after = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after rejection");
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after rejection");
    // SG6a.1（REQ-CG-03「修订不过校验」）：未 typed 编译失败不再把
    // turn 提前写成终态 Failed——同 turn 仍 Running（回原门/恢复面承
    // 接），failure_class 不落。
    assert_eq!(
        turn.status,
        crate::product::models::HumanGateTurnStatus::Running
    );
    assert_eq!(turn.failure_class, None);
    let current_candidate_hash = {
        use sha2::{Digest, Sha256};
        let markdown = engine
            .session()
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown())
            .expect("candidate");
        hex::encode(Sha256::digest(markdown.as_bytes()))
    };
    assert_eq!(turn.source_hash, current_candidate_hash);
    assert_eq!(before_artifact, engine.session().artifact);
    assert_eq!(
        before_artifact_versions,
        serde_json::to_vec(
            &lifecycle
                .list_artifact_versions(engine.session().session_id.as_str())
                .expect("artifact versions after rejection"),
        )
        .expect("serialize artifact versions after rejection"),
        "validation rejection must not append or rewrite candidate artifact versions"
    );
    assert_eq!(
        before_candidate_refs,
        (
            after.work_item_plan_source_revision_ref,
            after.plan_candidate_ir_ref,
            after.mechanical_report_ref,
        )
    );
    assert_eq!(
        after
            .human_gate_snapshot
            .as_ref()
            .expect("gate snapshot")
            .manual_repairs_remaining,
        before_budget - 1,
        "validation rejection consumes the reserved budget but never refunds it"
    );
    assert_eq!(after.provider_start_ledger.len(), before_ledger.len() + 1);
}

#[tokio::test]
async fn conversational_gate_revision_result_never_calls_legacy_chinese_title_constraint() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("revision_no_legacy", 1);
    let turn_id = open_running_revision_turn(&mut engine, "revision_no_legacy_command").await;
    crate::product::workspace_engine::reset_artifact_constraint_spec_call_count();
    let result = engine
        .run_sc_manual_revision_turn(
            &turn_id,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
            ))
            .to_string(),
        )
        .await
        .expect("valid output should complete through SC compiler/validator");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    assert_eq!(
        crate::product::workspace_engine::artifact_constraint_spec_call_count(),
        0,
        "SC revision must not consult legacy artifact constraints"
    );
    assert!(
        lifecycle
            .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
            .expect("turn")
            .result_artifact_ref
            .is_some()
    );
}

#[tokio::test]
async fn conversational_gate_revision_result_second_feedback_uses_revised_candidate() {
    let (_root, _lifecycle, mut engine) = durable_revision_fixture("revision_two_rounds", 2);
    let original = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ));
    let first = original.replace(
        "## Work Item WI-001: Backend levels API",
        "## Work Item WI-001: Revised backend levels API",
    );
    let second = first.replace(
        "## Work Item WI-001: Revised backend levels API",
        "## Work Item WI-001: Final backend levels API",
    );

    let first_turn = open_running_revision_turn(&mut engine, "revision_round_one").await;
    engine
        .run_sc_manual_revision_turn(&first_turn, first.clone())
        .await
        .expect("first revision should succeed");
    let second_outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "revision_round_two".to_string(),
            feedback: "再次修订标题".to_string(),
        })
        .await
        .expect("second feedback");
    let (second_turn, prompt) = match second_outcome {
        HumanGateCommandOutcome::TurnOpened { turn, prompt, .. } => (turn.turn_id, prompt),
        other => panic!("expected second turn, got {other:?}"),
    };
    assert!(prompt.contains("Revised backend levels API"));
    assert!(!prompt.contains("## Work Item WI-001: Backend levels API\n"));
    engine
        .mark_human_gate_turn_running(&second_turn)
        .expect("second turn running");
    engine
        .run_sc_manual_revision_turn(&second_turn, second.clone())
        .await
        .expect("second revision should succeed");
    assert!(
        engine
            .session()
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown())
            .is_some_and(|markdown| markdown.contains("Final backend levels API"))
    );
}

// —— 第 5 死路(B 裁决修复):人工修订完成后必须重走 Evaluate policy route ——
//
// 现场(levels matrix codex rep1 / issue_0104 / workspace_session_0120):门开在
// Evaluate,人工修订 turn 完成后 session 停留 Evaluate,confirm 的
// `compare_and_save_human_gate_close` 前置(WaitingForHuman+Approval)永久冲突。
// 修复契约 = 与初始 author(`complete_single_candidate_work_item_plan_author`)
// 同构:无 reviewer 本地 synthetic Pass 路由进 Approval;有 reviewer 重启评审,
// 不得让 close 绕过 Approval。

/// 现场同构的 Evaluate 门 fixture:基于 accepted contract drafts 基座(批准链
/// compile 可真实走通),门候选为 handoff-clean rep4(批准链 canonical 校验
/// 会拒绝带 unconsumed handoff 的 rep4 原文,与 campaign 基座候选一致)。
fn evaluate_gate_revision_fixture(
    session_id: &str,
    budget: u32,
    review_rounds: u32,
) -> (
    tempfile::TempDir,
    crate::product::lifecycle_store::LifecycleStore,
    crate::product::workspace_engine::WorkspaceEngine,
) {
    use crate::product::json_store::write_json;
    use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus};
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};
    use std::sync::Arc;

    let (root, lifecycle, _plan_id, mut engine) = crate::product::workspace_engine::tests::
        make_work_item_plan_engine_with_accepted_contract_drafts();
    crate::product::workspace_engine::tests::single_candidate_recovery::
        single_candidate_recovery_record(
            &lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            RunPolicy::Interactive,
        );
    let gate_refs = crate::product::workspace_engine::tests::single_candidate_recovery::
        single_candidate_recovery_persist_candidate_artifacts(
            &lifecycle,
            &engine,
            "evaluate-gate",
            &handoff_clean_rep4(),
        );
    crate::product::workspace_engine::tests::single_candidate_recovery::
        single_candidate_recovery_update_refs(
            &lifecycle,
            &mut engine,
            SingleCandidatePhase::Evaluate,
            gate_refs,
        );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("evaluate gate session record");
    record.review_rounds = review_rounds;
    record.status = WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: budget,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        // C2 新会话口径：gate-local 轮次已知为零（区别于旧会话 None=unknown）。
        accepted_feedback_turns: Some(0),
    });
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist evaluate gate session");
    let mut session = crate::product::workspace_engine::WorkspaceSession::from_record(record);
    session.stage = crate::product::workspace_engine::WorkspaceStage::HumanConfirm;
    session.session_status = WorkspaceSessionStatus::WaitingForHuman;
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: handoff_clean_rep4(),
        diff: None,
    });
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
    let engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            root.path().join(format!("{session_id}-checkpoints")),
        )),
        lifecycle.clone(),
        event_tx,
        session,
    );
    (root, lifecycle, engine)
}

/// rep4 fixture 的 WI-003 提供了 `contract.levels-integration` 却没有消费者;
/// 批准链 canonical 校验(`unconsumed_required_handoff`,Error 级)会拒绝原文。
/// 逐行剔除该 provided 行(其余逐字保留),与 campaign 基座候选同构。
pub(crate) fn handoff_clean_rep4() -> String {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ))
    .replace(
        "- provided_contract_refs: contract.levels-integration",
        "- provided_contract_refs: []",
    )
}

pub(crate) fn handoff_clean_rep4_v2() -> String {
    handoff_clean_rep4().replace("Backend levels API", "Backend levels API round-2")
}

// ---------------------------------------------------------------------------
// B2d（Task 5.6，tasks.md 5.6 / 设计 §5.3 sc_manual_revision 行）：SC 人工
// 修订 turn 的 native mismatch → typed 等待 → 显式 fresh → 同 turn 完成。
// turn 身份/gate 快照/预算/reservation 逐字段保留；等待期恢复不自动
// ResumeSameTurn（不增 attempt/不 spawn）；fresh 走同 turn 完成器
// `run_sc_manual_revision_turn`（不新 turn、不二次扣预算）。
// ---------------------------------------------------------------------------

/// seed 漂移形态：旧 Author native 会话在场且已被 tombstone 吊销——
/// `provider_resume_session_candidate` 必判 `FrozenBindingUnavailable`。
fn anchor_t18_seed_superseded_native(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
) {
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("t18 durable session");
    record.provider_conversations = vec![crate::product::models::ProviderConversationRef {
        role: crate::product::models::ProviderConversationRole::Author,
        provider: crate::product::models::ProviderName::ClaudeCode,
        provider_session_id: "t18-old-native".to_string(),
        updated_at: "2026-10-12T00:00:00Z".to_string(),
        last_node_id: Some("timeline_node_001".to_string()),
    }];
    record.superseded_native_sessions =
        vec![crate::product::models::SupersededNativeSessionRecord {
            role: crate::product::models::ProviderConversationRole::Author,
            provider: crate::product::models::ProviderName::ClaudeCode,
            native_session_id: "t18-old-native".to_string(),
            reason_code: "resume_fingerprint_unavailable".to_string(),
            recovery_id: "seed".to_string(),
            superseded_at: "2026-10-12T00:00:00Z".to_string(),
        }];
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("seed t18 superseded native");
    let repository_path = engine.session.repository_path.clone();
    let artifact = engine.session.artifact.clone();
    let stage = engine.session.stage.clone();
    let session_status = engine.session.session_status.clone();
    engine.session = crate::product::workspace_engine::WorkspaceSession::from_record(record);
    engine.session.repository_path = repository_path;
    engine.session.artifact = artifact;
    engine.session.stage = stage;
    engine.session.session_status = session_status;
}

/// B2d：manual turn native mismatch 的等待收口——ScManualRevision tagged
/// record 冻结同 turn 事实与 feedback，turn/gate 快照/预算/ledger 逐字段
/// 不变；等待 active 期间恢复跳过该 turn（BlockedByLcRecovery），Resolved
/// 后既有恢复语义回归（不永久楔死）。
#[tokio::test]
async fn anchor_t18_manual_mismatch_wait_preserves_turn_gate_budget_and_blocks_recovery() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("t18_manual_wait", 2);
    let turn_before = {
        let outcome = engine
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: "cmd-t18-manual-wait".to_string(),
                feedback: "T18:把后端接口标题改为round-2口径".to_string(),
            })
            .await
            .expect("open t18 manual turn");
        match outcome {
            HumanGateCommandOutcome::TurnOpened { turn, .. } => turn,
            other => panic!("expected opened turn, got {other:?}"),
        }
    };
    assert_eq!(
        turn_before.status,
        crate::product::models::HumanGateTurnStatus::Reserved
    );
    anchor_t18_seed_superseded_native(&lifecycle, &mut engine);
    let ledger_before = serde_json::to_vec(
        &lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("t18 record before")
            .provider_start_ledger,
    )
    .expect("serialize ledger before");

    // precheck：mismatch 家族 → Some(稳定判别码)。
    let reason = engine
        .precheck_sc_manual_revision_resume()
        .expect("t18 precheck read ok")
        .expect("superseded native must classify as mismatch waiting");
    assert_eq!(reason, "resume_fingerprint_unavailable");

    let recovery = engine
        .persist_sc_manual_revision_superseded_wait(&turn_before.turn_id, reason)
        .await
        .expect("persist t18 manual waiting");
    assert_eq!(
        recovery.state,
        crate::product::models::LcResumeRecoveryState::Waiting
    );
    assert_eq!(recovery.reason_code, "resume_fingerprint_unavailable");
    assert_eq!(
        recovery.superseded_native_session_id.as_deref(),
        Some("t18-old-native")
    );
    let crate::product::models::LcRecoveryRunContext::ScManualRevision {
        turn_id: payload_turn_id,
        command_id,
        source_hash,
        turn_status,
        attempt_no,
        budget_reserved,
        human_gate_reservation_key,
        feedback_text,
        ..
    } = &recovery.run_context
    else {
        panic!(
            "t18 必须捕获 sc_manual_revision tag，got {:?}",
            recovery.run_context
        )
    };
    assert_eq!(payload_turn_id, &turn_before.turn_id);
    assert_eq!(command_id, &turn_before.command_id);
    assert_eq!(source_hash, &turn_before.source_hash);
    assert_eq!(
        *turn_status,
        crate::product::models::HumanGateTurnStatus::Reserved
    );
    assert_eq!(*attempt_no, turn_before.attempt_no);
    assert_eq!(*budget_reserved, turn_before.budget_reserved);
    assert_eq!(
        feedback_text.as_deref(),
        Some(turn_before.feedback_text.as_str())
    );
    assert_eq!(
        human_gate_reservation_key.as_deref(),
        lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("t18 record after wait")
            .human_gate_reservation
            .as_ref()
            .map(|reservation| reservation.provider_start_idempotency_key.as_str()),
        "等待保留原 reservation key"
    );

    // turn/gate 快照/预算/账本逐字段不变（等待 ≠ 业务失败）。
    let turn_after_wait = lifecycle
        .get_human_gate_turn(&engine.session().session_id, &turn_before.turn_id)
        .expect("t18 turn after wait");
    assert_eq!(
        turn_after_wait.status,
        crate::product::models::HumanGateTurnStatus::Reserved,
        "mismatch 不 fail turn（原 Reserved 保持）"
    );
    assert_eq!(turn_after_wait.attempt_no, turn_before.attempt_no);
    assert_eq!(turn_after_wait.budget_reserved, turn_before.budget_reserved);
    assert!(turn_after_wait.failure_class.is_none());
    let record_after_wait = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("t18 record after wait");
    assert_eq!(
        record_after_wait
            .human_gate_snapshot
            .as_ref()
            .expect("gate snapshot kept")
            .manual_repairs_remaining,
        1,
        "等待不二次扣预算"
    );
    assert_eq!(
        record_after_wait.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
        "门保持开态"
    );
    assert_eq!(
        serde_json::to_vec(&record_after_wait.provider_start_ledger).expect("serialize ledger"),
        ledger_before,
        "等待不追加 provider_start_ledger"
    );

    // 等待 active：恢复跳过该 turn（不 ResumeSameTurn/不增 attempt/不 spawn）。
    let blocked_actions = engine
        .recover_human_gate_turns(false)
        .expect("t18 blocked recovery");
    assert_eq!(
        blocked_actions,
        vec![(
            turn_before.turn_id.clone(),
            crate::product::workspace_engine::HumanGateRecoveryAction::BlockedByLcRecovery {
                recovery_id: recovery.recovery_id.clone(),
            }
        )],
        "active sc_manual recovery 阻塞的 turn 不得自动恢复"
    );
    let turn_still_blocked = lifecycle
        .get_human_gate_turn(&engine.session().session_id, &turn_before.turn_id)
        .expect("t18 turn still blocked");
    assert_eq!(turn_still_blocked.attempt_no, turn_before.attempt_no);
    assert_eq!(
        turn_still_blocked.status,
        crate::product::models::HumanGateTurnStatus::Reserved
    );

    // 对照：Resolved 后既有恢复语义回归（不永久楔死）。
    {
        let expected = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("t18 record before resolve");
        let mut resolved = expected
            .lc_resume_recovery
            .clone()
            .expect("t18 recovery before resolve");
        resolved.state = crate::product::models::LcResumeRecoveryState::Resolved;
        lifecycle
            .compare_and_save_lc_resume_state(&expected, resolved)
            .expect("resolve t18 recovery");
    }
    let unblocked_actions = engine
        .recover_human_gate_turns(false)
        .expect("t18 unblocked recovery");
    assert!(matches!(
        unblocked_actions.as_slice(),
        [(
            turn_id,
            crate::product::workspace_engine::HumanGateRecoveryAction::ResumeSameTurn {
                next_attempt_no: 1,
            }
        )] if turn_id == &turn_before.turn_id
    ));
}

/// B2d：显式 fresh 完成后的同 turn 收口——run_sc_manual_revision_turn 同
/// turn_id/command/source_hash 完成，attempt/budget 不变，门预算接续
/// （回原 human gate，不二次扣预算）。
#[tokio::test]
async fn anchor_t18_manual_same_turn_completer_after_wait_keeps_turn_gate_budget() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("t18_manual_complete", 2);
    let turn_before = {
        let outcome = engine
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: "cmd-t18-manual-complete".to_string(),
                feedback: "T18:把后端接口标题改为round-2口径".to_string(),
            })
            .await
            .expect("open t18 manual turn");
        match outcome {
            HumanGateCommandOutcome::TurnOpened { turn, .. } => turn,
            other => panic!("expected opened turn, got {other:?}"),
        }
    };
    anchor_t18_seed_superseded_native(&lifecycle, &mut engine);
    let reason = engine
        .precheck_sc_manual_revision_resume()
        .expect("t18 precheck read ok")
        .expect("superseded native must classify as mismatch waiting");
    let recovery = engine
        .persist_sc_manual_revision_superseded_wait(&turn_before.turn_id, reason)
        .await
        .expect("persist t18 manual waiting");
    assert_eq!(
        recovery.state,
        crate::product::models::LcResumeRecoveryState::Waiting
    );

    // 同 turn 完成器（fresh run 产出 → run_sc_manual_revision_turn）。
    let result = engine
        .run_sc_manual_revision_turn(&turn_before.turn_id, handoff_clean_rep4_v2())
        .await
        .expect("t18 same-turn completion");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    let turn_after = lifecycle
        .get_human_gate_turn(&engine.session().session_id, &turn_before.turn_id)
        .expect("t18 turn after completion");
    assert_eq!(turn_after.turn_id, turn_before.turn_id, "同 turn_id");
    assert_eq!(
        turn_after.command_id, turn_before.command_id,
        "同 command_id"
    );
    assert_eq!(
        turn_after.source_hash, turn_before.source_hash,
        "同 source_hash"
    );
    assert_eq!(
        turn_after.status,
        crate::product::models::HumanGateTurnStatus::Completed
    );
    assert_eq!(
        turn_after.attempt_no, turn_before.attempt_no,
        "同 turn 完成不加 attempt"
    );
    assert_eq!(turn_after.budget_reserved, turn_before.budget_reserved);
    let record_after = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("t18 record after completion");
    assert_eq!(
        record_after
            .human_gate_snapshot
            .as_ref()
            .expect("gate snapshot kept")
            .manual_repairs_remaining,
        1,
        "fresh 完成不二次扣预算（门预算接续）"
    );
    assert_eq!(
        record_after.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
        "回原 human gate（门重开）"
    );
}

// ---------------------------------------------------------------------------
// SG6a.1（plan v1.1 Task SG6a.1 / REQ-CG-03、REQ-SG-02、REQ-SG-05）：assess
// 与同 turn claim 驱动前置。判定先于任何终态写——第414行 typed 诊断 turn
// 仍 Running 且 current refs 保、bad source 不可评审；claim attempt2 恰一次。
// ---------------------------------------------------------------------------

use crate::product::models::workspace::{
    StructuralGovernanceState, StructuralGrantPhase, StructuralRevisionGrant,
};
use crate::product::workspace_engine::types::StructuralAssessment;

const SG61_REP4: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
));

/// 第414行非法二级 heading 候选（F1.2 typed producer 首分支同族）：rep4
/// 尾部 Rationale 自由文本内确定性填充至第 413 行，第 414 行放一条非
/// Work Item 的 `## ` heading——固定归一化表外，compiler fail-closed 产出
/// `InvalidWorkItemHeading { line: 414, .. }` typed 诊断（原行/字段/合法
/// 模板齐备，机器事实完整）。
fn sg61_line414_illegal_heading_candidate() -> String {
    let mut lines: Vec<String> = SG61_REP4.lines().map(|line| line.to_string()).collect();
    assert_eq!(lines.first().map(String::as_str), Some("# Work Item Plan"));
    while lines.len() < 413 {
        lines.push(format!(
            "- 填充行 {}: Rationale 自由文本，不触碰结构标题。",
            lines.len() + 1
        ));
    }
    assert_eq!(lines.len(), 413);
    lines.push("## 业务补充范围: 非法非 Work Item 二级标题".to_string());
    lines.join("\n")
}

/// R1 环候选（与 single_candidate 系 `cycle_candidate` 同构、基座为本
/// fixture 工件 rep4）：WI-001 反向消费 WI-002 的
/// `contract.level-selector`，与既有 WI-002→WI-001 canonical 输入互为
/// 有效图环（SG1 witness 逐跳真实、来源 canonical_input）。
fn sg61_cycle_candidate() -> String {
    SG61_REP4.replace(
        "### Inputs\n\n### Outputs\n- contract_id: contract.levels-api",
        "### Inputs\n\
         - contract_id: contract.level-selector\n\
         - provider_logical_work_item_id: WI-002\n\
         - required_capabilities: ui.level-selector.rendered\n\
         - compatibility_policy: require_all\n\
         \n\
         ### Outputs\n\
         - contract_id: contract.levels-api",
    )
}

/// 失败候选的 source hash（与引擎内容寻址同配方）。
fn sg61_source_hash(source: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(source.as_bytes()))
}

/// 向 durable 记录注入活动 grant（SG5.3 fingerprint fixture 同款：保留
/// reserve 已落的 user hold——真实 L1 开窗后的耐久形状）。
fn sg61_seed_structural_grant(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    engine: &crate::product::workspace_engine::WorkspaceEngine,
    turn: &crate::product::models::HumanGateTurn,
    attempt_no: u32,
) {
    let mut record = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("load session record for grant seed");
    let state = record
        .structural_governance_state
        .get_or_insert_with(StructuralGovernanceState::default);
    state.current_grant = Some(StructuralRevisionGrant {
        grant_id: format!("structural_grant_{}", turn.turn_id),
        authorization_hash: format!("sha256:{}", turn.source_hash),
        turn_id: Some(turn.turn_id.clone()),
        command_id: Some(turn.command_id.clone()),
        candidate_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        phase: Some(StructuralGrantPhase::AuthorRunning),
        attempt_no,
        prompt_ref: None,
        prompt_digest: None,
        instruction_digest: None,
        failing_source_hash: None,
        review_identity: None,
        granted_at: "2026-10-14T00:00:00Z".to_string(),
    });
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist structural governance grant fixture");
}

/// 读取 durable 聚合 state 的当前 grant（无则 None）。
fn sg61_current_grant(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    session_id: &str,
) -> Option<StructuralRevisionGrant> {
    lifecycle
        .get_workspace_session(session_id)
        .expect("load session record")
        .structural_governance_state
        .and_then(|state| state.current_grant)
}

/// SG6a.1 主验收①：第414行 typed 诊断（InvalidWorkItemHeading）turn 仍
/// Running 且 current refs 保——不提前 Failed、不提升坏 source、零评审/
/// 零 provider 启动；判定面 `assess_sc_revision_output` 产出带机器指令
/// 的 `Rejected`（L1 事实完整），且判定本身零终态写。
#[tokio::test]
async fn sg_06a_1_line414_typed_rejection_keeps_turn_running_refs_and_blocks_review() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a1_line414", 3);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a1_line414_command").await;
    let turn_before = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn before rejection");
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session before rejection");
    let output = sg61_line414_illegal_heading_candidate();

    // 判定面：typed heading 失败 → L1（原行/字段/模板齐备）→ Rejected 携
    // 机器指令（RewriteWithGrammarTemplate 合法模板）；判定零终态。
    let assessment = engine
        .assess_sc_revision_output(&turn_id, &output)
        .expect("typed heading failure must produce a four-branch assessment");
    let (failure_set, instructions) = match &assessment {
        StructuralAssessment::Rejected(set, instructions) => (set, instructions),
        other => panic!("expected Rejected for line-414 typed heading, got {other:?}"),
    };
    assert!(
        failure_set.failures.iter().any(|failure| matches!(
            &failure.kind,
            crate::product::planning_failure::PlanningFailureKind::InvalidWorkItemHeading {
                line: 414,
                ..
            }
        )),
        "line-414 typed InvalidWorkItemHeading must be preserved with its original line"
    );
    assert!(
        !instructions.is_empty(),
        "facts-complete heading failure must carry machine repair instructions"
    );
    let turn_after_assess = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after assess");
    assert_eq!(
        turn_after_assess.status,
        crate::product::models::HumanGateTurnStatus::Running,
        "assessment itself must not terminalize the turn"
    );

    // 驱动面：判定拒收后 turn 仍 Running（不提前 Failed），current refs/
    // artifact/预算快照保持，零评审、零 provider 启动。
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, output)
        .await
        .expect("typed structural rejection is an in-band outcome, not an error");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected { .. }
    ));
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after rejection");
    assert_eq!(
        turn.status,
        crate::product::models::HumanGateTurnStatus::Running,
        "SG6a.1: typed structural rejection must not terminal-Fail the turn"
    );
    assert_eq!(turn.failure_class, None);
    assert_eq!(turn.source_hash, turn_before.source_hash);
    let after = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after rejection");
    assert_eq!(
        (
            &after.work_item_plan_source_revision_ref,
            &after.plan_candidate_ir_ref,
            &after.mechanical_report_ref,
        ),
        (
            &before.work_item_plan_source_revision_ref,
            &before.plan_candidate_ir_ref,
            &before.mechanical_report_ref,
        ),
        "bad source must not be promoted to current refs"
    );
    assert_eq!(
        after.review_rounds, before.review_rounds,
        "bad source is not reviewable: review rounds must not grow"
    );
    assert_eq!(
        after.provider_start_ledger.len(),
        before.provider_start_ledger.len(),
        "bad source must not start any provider run (claim 与真实 start 分账)"
    );
    assert_eq!(
        engine.session().stage,
        crate::product::workspace_engine::WorkspaceStage::HumanConfirm,
        "gate stays open at human_confirm"
    );

    // 门消息面（沿原 SC 三消息 + gate_busy）：in-flight turn 期间新反馈
    // 拒为 gate_busy（预算/门状态不变）。
    let busy = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "sg06a1_line414_busy_command".to_string(),
            feedback: "在飞行中再反馈".to_string(),
        })
        .await
        .expect("busy outcome is in-band");
    assert!(matches!(busy, HumanGateCommandOutcome::Busy { .. }));
}

/// SG6a.1 主验收②：同 turn claim 驱动前置——L1 拒收（canonical 输入环）
/// 在任何终态写之前冻结机器指令 prompt 并 `claim_structural_attempt`
/// attempt2 恰一次；同失败重放 Replayed 零新授权、零第三次。
#[tokio::test]
async fn sg_06a_1_same_turn_claim_attempt2_exactly_once() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("sg06a1_claim_once", 3);
    let turn_id = open_running_revision_turn(&mut engine, "sg06a1_claim_command").await;
    let turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after open");
    // 真实 L1 开窗后的耐久形状：活动 grant 绑定本 turn（attempt 1）。
    sg61_seed_structural_grant(&lifecycle, &engine, &turn, 1);
    let before_ledger = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session before claim")
        .provider_start_ledger
        .len();
    let output = sg61_cycle_candidate();
    let failed_hash = sg61_source_hash(&output);

    let result = engine
        .run_sc_manual_revision_turn(&turn_id, output.clone())
        .await
        .expect("L1 cycle rejection is in-band");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected { .. }
    ));

    let grant = sg61_current_grant(&lifecycle, engine.session().session_id.as_str())
        .expect("grant survives the claim");
    assert_eq!(grant.attempt_no, 2, "claim must advance attempt to 2");
    assert_eq!(grant.phase, Some(StructuralGrantPhase::Attempt2Claimed));
    assert!(
        grant.prompt_ref.as_deref().is_some_and(|r| !r.is_empty()),
        "claim binds the frozen prompt ref"
    );
    assert!(
        grant
            .prompt_digest
            .as_deref()
            .is_some_and(|d| !d.is_empty()),
        "claim binds the frozen prompt digest"
    );
    assert_eq!(
        grant.failing_source_hash.as_deref(),
        Some(failed_hash.as_str())
    );
    let turn_after = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after claim");
    assert_eq!(
        turn_after.status,
        crate::product::models::HumanGateTurnStatus::Running,
        "claimed attempt2 keeps the same turn in-flight"
    );
    assert_eq!(turn_after.turn_id, turn.turn_id);
    assert_eq!(turn_after.command_id, turn.command_id);

    // 同失败重放：同 prompt 指令身份重复 claim → Replayed（恰一次），
    // attempt 不推进、ledger 零新增、turn 仍 Running。
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, output)
        .await
        .expect("replay of the same failed output stays in-band");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected { .. }
    ));
    let grant_replay = sg61_current_grant(&lifecycle, engine.session().session_id.as_str())
        .expect("grant after replay");
    assert_eq!(
        grant_replay.attempt_no, 2,
        "duplicate claim replays exactly once: no third attempt"
    );
    assert_eq!(
        grant_replay.phase,
        Some(StructuralGrantPhase::Attempt2Claimed)
    );
    assert_eq!(grant_replay.prompt_ref, grant.prompt_ref);
    assert_eq!(grant_replay.prompt_digest, grant.prompt_digest);
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("session after replay")
            .provider_start_ledger
            .len(),
        before_ledger,
        "claim 与真实 start 分账：重放零 provider 启动"
    );
    let turn_replay = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn after replay");
    assert_eq!(
        turn_replay.status,
        crate::product::models::HumanGateTurnStatus::Running
    );
}
