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
        .expect("failed turn");
    assert_eq!(
        turn.status,
        crate::product::models::HumanGateTurnStatus::Failed
    );
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
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: budget,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
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

