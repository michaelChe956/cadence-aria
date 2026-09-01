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
