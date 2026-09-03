use crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES;
use crate::product::workspace_engine::conversational_gate::trim_provider_preamble;
use crate::product::workspace_engine::prompts::{
    SC_MANUAL_REVISION_FEEDBACK_MAX_BYTES, SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES,
    ScManualRevisionPromptInput, build_sc_manual_revision_prompt,
};
use crate::product::workspace_engine::{HumanGateCommandOutcome, HumanGateFeedbackInput};

const LANGUAGE_RULE_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/.claude/rules/language.md"
));
const PRIORITY_RULE_MARKER: &str =
    "结构标题(##/### section 名)、字段 key、ID(WI-*/CT-*/TASK-*/AC-*/REQ-*/CHECK-* 等)、枚举值";

#[test]
fn conversational_gate_revision_prompt_includes_candidate_feedback_grammar_language_and_teaching() {
    let candidate =
        "# Work Item Plan\n\n## Work Item WI-001: 当前候选\n\n### Outputs\n- contract_id: CT-001\n";
    let feedback = "只补充 WI-001 的 Outputs 能力，不要动其他内容";
    let grammar =
        "[markdown_grammar]\n标题必须逐字为 # Work Item Plan；Handoff Schema 必须保留三个字段。";

    let prompt = build_sc_manual_revision_prompt(ScManualRevisionPromptInput {
        candidate_markdown: candidate,
        feedback,
        grammar_boundary: grammar,
        language_rule: LANGUAGE_RULE_FIXTURE,
    })
    .expect("revision prompt should fit the contract budget");

    for expected in [candidate, feedback, grammar, LANGUAGE_RULE_FIXTURE] {
        assert!(
            prompt.contains(expected),
            "missing injected content: {expected}"
        );
    }
    assert!(
        prompt.contains(PRIORITY_RULE_MARKER),
        "missing priority rule: {prompt}"
    );
    assert!(
        prompt.contains("只改反馈点名的内容，其余逐字保留"),
        "missing positive teaching: {prompt}"
    );
    assert!(
        prompt.contains("禁止删字段"),
        "missing delete-fields prohibition: {prompt}"
    );
    assert!(
        prompt.contains("清空 Outputs"),
        "missing Outputs prohibition: {prompt}"
    );
    assert!(
        prompt.contains("遗漏 Handoff Schema 三字段"),
        "missing handoff prohibition: {prompt}"
    );
    assert!(
        !prompt.contains("code-usage"),
        "code-usage summary must not be injected: {prompt}"
    );
    assert!(
        !prompt.contains("code-reading"),
        "code-reading summary must not be injected: {prompt}"
    );
    assert!(
        prompt.len() <= SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES,
        "revision prompt exceeds budget: {} > {}",
        prompt.len(),
        SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES
    );
    println!(
        "sc manual revision prompt bytes={} margin={}",
        prompt.len(),
        SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES - prompt.len()
    );
}

#[tokio::test]
async fn conversational_gate_revision_prompt_rejects_oversized_feedback_before_reservation() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session before oversized feedback");
    let before_session_bytes = serde_json::to_vec(&before).expect("serialize session before");
    let oversized = "x".repeat(SC_MANUAL_REVISION_FEEDBACK_MAX_BYTES + 1);

    let outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_oversized_revision".to_string(),
            feedback: oversized,
        })
        .await
        .expect("oversized feedback rejection");
    assert!(matches!(
        outcome,
        HumanGateCommandOutcome::Rejected { ref code, .. }
            if code == "HUMAN_GATE_FEEDBACK_TOO_LARGE"
    ));

    let after = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("durable session after oversized feedback");
    assert_eq!(
        serde_json::to_vec(&after).expect("serialize session after"),
        before_session_bytes,
        "oversized feedback must not mutate durable session"
    );
    assert!(
        lifecycle
            .list_human_gate_turns(engine.session().session_id.as_str())
            .expect("list turns")
            .is_empty(),
        "oversized feedback must not create a turn"
    );
    assert!(
        after.provider_start_ledger.is_empty(),
        "oversized feedback must not reserve a provider start"
    );
}

#[tokio::test]
async fn conversational_gate_revision_rejects_missing_candidate_before_reservation() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    engine.session.stage = super::WorkspaceStage::HumanConfirm;
    engine.session.artifact = None;
    let outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_missing_candidate".to_string(),
            feedback: "修正字段".to_string(),
        })
        .await
        .expect("missing candidate rejection");
    assert_eq!(
        outcome,
        HumanGateCommandOutcome::Rejected {
            code: "HUMAN_GATE_REVISION_CANDIDATE_MISSING".to_string(),
            reason: "current candidate markdown is required".to_string(),
        }
    );
    assert!(
        lifecycle
            .list_human_gate_turns(engine.session().session_id.as_str())
            .expect("list turns")
            .is_empty()
    );
}

#[tokio::test]
async fn conversational_gate_revision_rejects_when_no_markdown_artifact_version_exists() {
    // 批准链 compile 后 current artifact 是非 Markdown 投影，回落只能依赖版本列表；
    // 版本列表也完全无 Markdown 时必须保持既有拒绝语义（不放宽 CANDIDATE_MISSING）。
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    engine.session.stage = super::WorkspaceStage::HumanConfirm;
    let non_markdown_payload =
        crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory {
            history: Box::new(crate::web::workspace_ws_types::WorkItemRevisionHistoryDto {
                entries: Vec::new(),
            }),
        };
    engine.session.artifact = Some(non_markdown_payload.clone());
    engine.artifact_versions = vec![crate::web::workspace_ws_types::ArtifactVersion {
        version: 1,
        payload: non_markdown_payload,
        generated_by: crate::product::models::ProviderName::Fake,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        source_node_id: "timeline_node_unknown".to_string(),
    }];
    let outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_no_markdown_artifact_version".to_string(),
            feedback: "修正字段".to_string(),
        })
        .await
        .expect("no markdown artifact version rejection");
    assert_eq!(
        outcome,
        HumanGateCommandOutcome::Rejected {
            code: "HUMAN_GATE_REVISION_CANDIDATE_MISSING".to_string(),
            reason: "current candidate markdown is required".to_string(),
        }
    );
    assert!(
        lifecycle
            .list_human_gate_turns(engine.session().session_id.as_str())
            .expect("list turns")
            .is_empty(),
        "missing candidate must not reserve a turn"
    );
}

#[test]
fn conversational_gate_revision_trim_is_deterministic_and_only_removes_preamble() {
    let source = "provider preamble\n# Work Item Plan\n## Work Item WI-001: x\n";
    assert_eq!(
        trim_provider_preamble(source),
        "# Work Item Plan\n## Work Item WI-001: x\n"
    );
    let malformed = "provider output without canonical heading";
    assert_eq!(trim_provider_preamble(malformed), malformed);
}
#[test]
fn conversational_gate_revision_prompt_budget_is_independent_from_author_budget() {
    assert_eq!(
        WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES, 19_000,
        "SC author red-line budget must remain unchanged"
    );
    assert_eq!(SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES, 32_000);
    assert_ne!(
        SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES,
        WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES
    );
}

fn durable_revision_fixture(
    session_id: &str,
    budget: u32,
) -> (
    tempfile::TempDir,
    crate::product::lifecycle_store::LifecycleStore,
    crate::product::workspace_engine::WorkspaceEngine,
) {
    let (tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate(session_id);
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session");
    record.flow_kind = crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate;
    // 本 fixture 的主驗主体是「门内修订持久化」契约:无 reviewer 时修订后经本地
    // synthetic Pass 路由回到 HumanConfirm 门,多轮 feedback 循环可继续。带 reviewer
    // 的重启评审分支由 evaluate_gate_revision_fixture 系列用例单独锁定。
    record.review_rounds = 0;
    record.status = crate::product::models::WorkspaceSessionStatus::WaitingForHuman;
    record.human_gate_snapshot = Some(crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: budget,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: false,
    });
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist human gate session");
    engine.session.flow_kind = record.flow_kind;
    engine.session.stage = crate::product::workspace_engine::WorkspaceStage::HumanConfirm;
    engine.session.session_status = crate::product::models::WorkspaceSessionStatus::WaitingForHuman;
    engine.session.human_gate_snapshot = record.human_gate_snapshot;
    engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
        ))
        .to_string(),
        diff: None,
    });
    (tmp, lifecycle, engine)
}

async fn open_running_revision_turn(
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
    command_id: &str,
) -> String {
    let outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: command_id.to_string(),
            feedback: "修订当前候选标题".to_string(),
        })
        .await
        .expect("open human gate turn");
    let turn_id = match outcome {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected opened turn, got {other:?}"),
    };
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("mark turn running");
    turn_id
}

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
fn handoff_clean_rep4() -> String {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ))
    .replace(
        "- provided_contract_refs: contract.levels-integration",
        "- provided_contract_refs: []",
    )
}

fn handoff_clean_rep4_v2() -> String {
    handoff_clean_rep4().replace("Backend levels API", "Backend levels API round-2")
}

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
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
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
        .handle_human_gate_termination(
            crate::web::workspace_ws_types::HumanConfirmDecision::Confirm,
        )
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
