use crate::product::models::WorkspaceType;
use crate::product::work_item_plan_policy::{
    FindingFingerprint, HumanReason, ReviewInvocationScope, RunBudgets, RunHistory, RunPolicy,
};
use crate::product::workspace_engine::review::policy_routing::{
    GateSnapshotContext, RoutingAction, route_outcome, route_repeated_human_gate_fingerprint,
};
use crate::product::workspace_engine::{
    HumanGateCommandOutcome, HumanGateFeedbackInput, WorkspaceStage,
    artifact_constraint_spec_call_count, artifact_constraint_spec_for,
    reset_artifact_constraint_spec_call_count,
};

fn feedback(command_id: &str) -> HumanGateFeedbackInput {
    HumanGateFeedbackInput {
        command_id: command_id.to_string(),
        feedback: "只修正这个字段".to_string(),
    }
}

#[test]
fn conversational_gate_revision_repeated_fingerprint_returns_same_gate() {
    let fingerprint = FindingFingerprint::new("a".repeat(64)).expect("fingerprint");
    let snapshot = crate::product::work_item_plan_policy::HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: vec![fingerprint.clone()],
        attempts_used: 2,
        manual_repairs_remaining: 2,
        trigger: HumanReason::RepeatedFingerprint,
        resumable: false,
        accepted_feedback_turns: None,
    };
    assert!(route_repeated_human_gate_fingerprint(&snapshot, &fingerprint).is_ok());
    let context = GateSnapshotContext {
        history: RunHistory {
            manual_repairs_used: 1,
            ..RunHistory::default()
        },
        budgets: RunBudgets {
            max_manual_repairs: 2,
            ..RunBudgets::default()
        },
        invocation: ReviewInvocationScope::initial("revision_001"),
        findings: Vec::new(),
        repeated_fingerprints: vec![fingerprint.clone()],
        trigger: HumanReason::RepeatedFingerprint,
    };
    let action = route_outcome(
        crate::product::work_item_plan_policy::PlanOutcome::HumanRequired {
            findings: Vec::new(),
            repeated_fingerprints: vec![fingerprint],
            reason: HumanReason::RepeatedFingerprint,
        },
        RunPolicy::Interactive,
        context,
    );
    let RoutingAction::EnterHumanGate { snapshot: routed } = action else {
        panic!("repeated finding must remain in the existing gate");
    };
    assert_eq!(routed.manual_repairs_remaining, 1);
}

#[test]
fn conversational_gate_revision_legacy_constraint_spy_positive_control() {
    reset_artifact_constraint_spec_call_count();
    let _ = artifact_constraint_spec_for(&WorkspaceType::Story);
    assert!(artifact_constraint_spec_call_count() > 0);
}
#[tokio::test]
async fn conversational_gate_revision_attempt_no_has_fixed_upper_bound() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(2);
    let opened = engine
        .handle_human_gate_feedback(feedback("attempt_limit"))
        .await
        .expect("open turn");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected turn, got {other:?}"),
    };
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("running");
    let expected = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session");
    let mut invalid = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("turn");
    invalid.attempt_no = 3;
    assert!(
        lifecycle
            .update_human_gate_turn(&expected, invalid)
            .is_err()
    );
}

#[tokio::test]
async fn conversational_gate_revision_ledger_counts_real_starts_only() {
    // Same command replay: no second logical budget or provider start.
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(3);
    let opened = engine
        .handle_human_gate_feedback(feedback("ledger_replay"))
        .await
        .expect("open turn");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected turn, got {other:?}"),
    };
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("before replay");
    let replay = engine
        .handle_human_gate_feedback(feedback("ledger_replay"))
        .await
        .expect("replay");
    assert!(matches!(replay, HumanGateCommandOutcome::Replayed { .. }));
    let after_replay = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("after replay");
    assert_eq!(
        after_replay.provider_start_ledger,
        before.provider_start_ledger
    );
    assert_eq!(after_replay.human_gate_snapshot, before.human_gate_snapshot);

    // Same-turn reconnect: idempotent running transition does not start again.
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("first running transition");
    engine
        .mark_human_gate_turn_running(&turn_id)
        .expect("reconnect running transition");
    let after_reconnect = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("after reconnect");
    assert_eq!(after_reconnect.provider_start_ledger.len(), 1);

    // Transport retry: exactly one additional attempt key, duplicate retry rejected.
    let mut retry = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
        .expect("running turn");
    retry.attempt_no = 2;
    let retry_session = lifecycle
        .update_human_gate_turn(&after_reconnect, retry.clone())
        .expect("attempt two");
    assert_eq!(retry_session.provider_start_ledger.len(), 2);
    assert_eq!(
        retry_session
            .provider_start_ledger
            .iter()
            .map(|entry| entry.provider_start_idempotency_key.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2
    );
    assert!(
        lifecycle
            .update_human_gate_turn(&retry_session, retry)
            .is_ok()
    );
    let final_session = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("final session");
    assert_eq!(final_session.provider_start_ledger.len(), 2);
}

#[tokio::test]
async fn legacy_revision_paths_remain_unchanged_when_conversational_gate_is_enabled() {
    let (_root, lifecycle, engine) = super::conversational_gate::gate_fixture(1);
    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("before");
    assert_eq!(before.workspace_type, WorkspaceType::WorkItemPlan);
    assert_eq!(engine.current_stage(), WorkspaceStage::HumanConfirm);
    let story_spec =
        crate::product::workspace_engine::artifact_constraint_spec_for(&WorkspaceType::Story);
    let design_spec =
        crate::product::workspace_engine::artifact_constraint_spec_for(&WorkspaceType::Design);
    assert!(!story_spec.required_headings.is_empty());
    assert!(!design_spec.required_headings.is_empty());
    assert_eq!(
        lifecycle
            .get_workspace_session(engine.session().session_id.as_str())
            .expect("after")
            .provider_start_ledger,
        before.provider_start_ledger
    );
}

// ── SG5.3（plan v1.1 Task SG5.3）：冻结修复子 prompt delivery 与 fresh builder ──

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::HumanGateTurnStatus;
use crate::product::models::workspace::{
    ResolvedStructureFact, StructuralGovernanceState, StructuralGrantPhase, StructuralRevisionGrant,
};
use crate::product::planning_failure::{
    PlanningFailure, PlanningFailureEvidence, PlanningFailureKind, PlanningFailureOrigin,
    PlanningFailureSet, PlanningStage,
};
use crate::product::workspace_engine::contract_prerevision::{
    ProtectedBusinessFacts, StructuralRepairInstruction, build_structural_repair_instructions,
};
use crate::product::workspace_engine::draft_batch::compile_support::{
    PlanningStructureContext, PlanningStructureReport,
};
use crate::product::workspace_engine::prompts::build_structural_revision_prompt;

const SG53_CONTEXT_DIGEST: &str =
    "5353535353535353535353535353535353535353535353535353535353535353";
const SG53_LOGICAL_A: &str = "11111111-1111-1111-1111-111111111111";
const SG53_LOGICAL_B: &str = "22222222-2222-2222-2222-222222222222";

fn sg53_sha256_hex(content: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(content.as_bytes()))
}

fn sg53_logical(id: &str) -> LogicalRepositoryId {
    LogicalRepositoryId(uuid::Uuid::parse_str(id).expect("fixture logical uuid"))
}

fn sg53_heading_failure(source_hash: &str) -> PlanningFailure {
    PlanningFailure {
        kind: PlanningFailureKind::InvalidWorkItemHeading {
            line: 414,
            field: "Acceptance Criteria".to_string(),
            raw_line: "### 验收标准".to_string(),
            example: "### Acceptance Criteria".to_string(),
        },
        origin: PlanningFailureOrigin::Compiler,
        stage: PlanningStage::Plan,
        diagnostic_ref: "diag://fixture/sg53/invalid-work-item-heading".to_string(),
        source_ref: "plan-source://fixture/sg53/rev-2".to_string(),
        source_hash: source_hash.to_string(),
        evidence: PlanningFailureEvidence {
            affected_ids: vec!["WI-900".to_string()],
            details: std::collections::BTreeMap::new(),
        },
    }
}

fn sg53_structure_report(source_hash: &str) -> PlanningStructureReport {
    PlanningStructureReport {
        source_hash: source_hash.to_string(),
        context_digest: SG53_CONTEXT_DIGEST.to_string(),
        rule_version: "structure-rules/v1".to_string(),
        compiler_version: "plan-compiler/v7".to_string(),
        effective_edges: Vec::new(),
        topological_order: Some(Vec::new()),
        residual_nodes: Vec::new(),
        physical_targets: std::collections::BTreeMap::new(),
        declared_dependency_facts: Vec::new(),
        failures: PlanningFailureSet {
            failures: Vec::new(),
        },
        structurally_valid: false,
    }
}

/// 指令经真实 SG2 producer 从 typed 失败生成（非手拼）。
fn sg53_instruction(source_hash: &str) -> StructuralRepairInstruction {
    build_structural_repair_instructions(
        &PlanningFailureSet {
            failures: vec![sg53_heading_failure(source_hash)],
        },
        &sg53_structure_report(source_hash),
        &ProtectedBusinessFacts {
            business_request_digest: "sha256:sg53-business-request".to_string(),
            confirmed_design_digest: Some("sha256:sg53-design".to_string()),
            protected_field_refs: vec!["Inputs/required_capabilities".to_string()],
            protected_contract_refs: vec!["contract.sg53-fixture".to_string()],
            authorized_member_ids: vec![SG53_LOGICAL_A.to_string(), SG53_LOGICAL_B.to_string()],
        },
    )
    .pop()
    .expect("heading failure yields exactly one instruction")
}

fn sg53_context(source_hash: &str) -> PlanningStructureContext {
    PlanningStructureContext {
        source_revision_ref: "plan-source://fixture/sg53/rev-2".to_string(),
        plan_candidate_ir_ref: "plan-ir://fixture/sg53/ir-2".to_string(),
        source_hash: source_hash.to_string(),
        compiler_version: "plan-compiler/v7".to_string(),
        rule_version: "structure-rules/v1".to_string(),
        context_digest: SG53_CONTEXT_DIGEST.to_string(),
        confirmed_design_id: Some("design_spec_0001".to_string()),
        confirmed_design_digest: Some("sha256:sg53-design".to_string()),
        member_table_digest: Some("sha256:sg53-member-table".to_string()),
        selection_digest: Some("sha256:sg53-selection".to_string()),
        physical_by_logical: [
            (sg53_logical(SG53_LOGICAL_A), "repo-alpha".to_string()),
            (sg53_logical(SG53_LOGICAL_B), "repo-beta".to_string()),
        ]
        .into_iter()
        .collect(),
        change_order: vec![sg53_logical(SG53_LOGICAL_A), sg53_logical(SG53_LOGICAL_B)],
        options_digest: "sha256:sg53-options".to_string(),
        baseline_tree_digest: Some("sha256:sg53-baseline".to_string()),
        intent_digest: Some("sha256:sg53-intent".to_string()),
    }
}

fn sg53_resolved(source_hash: &str) -> Vec<ResolvedStructureFact> {
    vec![ResolvedStructureFact {
        predicate: "work_item_headings_legal".to_string(),
        evidence_refs: vec!["report://fixture/sg53/rep-1".to_string()],
        source_hash: Some(source_hash.to_string()),
        resolved_at: "2026-10-14T00:00:00Z".to_string(),
    }]
}

/// 结构修订 attempt 的中间候选（hash 必须不同于 gate_fixture 原 turn 绑定
/// 的 `# Work Item Plan\n`）。
fn sg53_intermediate_candidate() -> String {
    "# Work Item Plan\n\n## Work Item WI-900: SG5.3 中间候选\n### Inputs\n- contract_id: contract.sg53-fixture\n- provider_logical_work_item_id: WI-900\n- required_capabilities: fixture.capability.read\n- compatibility_policy: require_all\n### 验收标准\n- 中间候选正文：结构修订 attempt 的失败候选，hash 不同于原 turn 绑定候选。\n".to_string()
}

fn sg53_grant(
    turn: &crate::product::models::HumanGateTurn,
    prompt_ref: Option<String>,
    prompt_digest: Option<String>,
    instruction: &StructuralRepairInstruction,
    failing_source_hash: Option<String>,
) -> StructuralRevisionGrant {
    StructuralRevisionGrant {
        grant_id: "structural_grant_sg53".to_string(),
        authorization_hash: "sha256:sg53-authorization".to_string(),
        turn_id: Some(turn.turn_id.clone()),
        command_id: Some(turn.command_id.clone()),
        candidate_source_revision_ref: Some("plan-source://fixture/sg53/rev-2".to_string()),
        plan_candidate_ir_ref: Some("plan-ir://fixture/sg53/ir-2".to_string()),
        mechanical_report_ref: Some("mechanical://fixture/sg53/report-2".to_string()),
        phase: Some(StructuralGrantPhase::Attempt2Claimed),
        attempt_no: 2,
        prompt_ref,
        prompt_digest,
        instruction_digest: Some(instruction.instruction_digest.clone()),
        failing_source_hash,
        review_identity: None,
        granted_at: "2026-10-14T00:00:00Z".to_string(),
    }
}

/// 向 durable 记录注入活动 grant（保留 reserve 已落的 user hold——真实
/// claim 后的耐久形状）。
fn sg53_persist_structural_state(
    lifecycle: &LifecycleStore,
    session_id: &str,
    grant: StructuralRevisionGrant,
) {
    let mut record = lifecycle
        .get_workspace_session(session_id)
        .expect("load session record");
    let state = record
        .structural_governance_state
        .get_or_insert_with(StructuralGovernanceState::default);
    state.current_grant = Some(grant);
    crate::product::json_store::write_json(
        &lifecycle
            .app_paths()
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist structural governance fixture");
}

fn sg53_session_partition(lifecycle: &LifecycleStore, session_id: &str) -> std::path::PathBuf {
    lifecycle
        .app_paths()
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-sessions")
        .join(session_id)
}

async fn sg53_open_turn(
    engine: &mut crate::product::workspace_engine::WorkspaceEngine,
    command_id: &str,
) -> (crate::product::models::HumanGateTurn, String) {
    match engine
        .handle_human_gate_feedback(feedback(command_id))
        .await
        .expect("open turn")
    {
        HumanGateCommandOutcome::TurnOpened { turn, prompt, .. } => (turn, prompt),
        other => panic!("expected turn, got {other:?}"),
    }
}

#[tokio::test]
async fn sg_05_3_frozen_subprompt_delivery_active_grant_intermediate_hash_exact_join() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(3);
    let (turn, original_prompt) = sg53_open_turn(&mut engine, "sg53_join").await;
    let original_candidate = "# Work Item Plan\n";
    assert_eq!(turn.source_hash, sg53_sha256_hex(original_candidate));

    // 中间候选：结构 attempt 的失败候选（≠ 原 turn 绑定候选，合法中间
    // hash），指令/context/frozen 全部绑定它。
    let intermediate = sg53_intermediate_candidate();
    let intermediate_hash = sg53_sha256_hex(&intermediate);
    assert_ne!(
        intermediate_hash, turn.source_hash,
        "合法中间 hash 必须不同于原 turn.source_hash"
    );
    let instruction = sg53_instruction(&intermediate_hash);
    let context = sg53_context(&intermediate_hash);
    let resolved = sg53_resolved(&intermediate_hash);
    let frozen = build_structural_revision_prompt(
        &turn.feedback_text,
        &intermediate,
        &instruction,
        &context,
        &resolved,
    )
    .expect("freeze structural prompt");

    engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: intermediate.clone(),
        diff: None,
    });
    sg53_persist_structural_state(
        &lifecycle,
        engine.session().session_id.as_str(),
        sg53_grant(
            &turn,
            Some(frozen.prompt_ref.clone()),
            Some(frozen.prompt_digest.clone()),
            &instruction,
            Some(intermediate_hash.clone()),
        ),
    );

    let before = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session before delivery");

    let delivered = engine
        .build_sc_revision_delivery_for_resume(&turn.command_id, &turn, Some(&frozen))
        .expect("frozen exact join delivers the frozen full prompt");

    // 交付＝冻结全文逐字节（不以复造原 turn prompt 覆盖当前 instruction）。
    assert_eq!(delivered, frozen.full_prompt);
    assert_ne!(delivered, original_prompt);
    // 原反馈/context/full candidate 完整。
    assert_eq!(delivered.matches(turn.feedback_text.as_str()).count(), 1);
    assert_eq!(delivered.matches(intermediate.as_str()).count(), 1);
    assert!(delivered.contains("[structural_repair_instruction]"));
    assert!(delivered.contains("[structure_context_facts]"));
    assert!(delivered.contains("repo-alpha"));
    assert!(!delivered.contains("[typed_human_feedback]"));

    // 零 delivery/start：turn 仍 Reserved、ledger/refs 不变、无组装/停等记录。
    let after_turn = lifecycle
        .get_human_gate_turn(engine.session().session_id.as_str(), &turn.turn_id)
        .expect("turn after delivery");
    assert_eq!(after_turn.status, HumanGateTurnStatus::Reserved);
    let after = lifecycle
        .get_workspace_session(engine.session().session_id.as_str())
        .expect("session after delivery");
    assert_eq!(after.provider_start_ledger, before.provider_start_ledger);
    let grant = after
        .structural_governance_state
        .as_ref()
        .and_then(|state| state.current_grant.as_ref())
        .expect("grant preserved");
    assert_eq!(
        grant.prompt_ref.as_deref(),
        Some(frozen.prompt_ref.as_str())
    );
    assert_eq!(
        grant.prompt_digest.as_deref(),
        Some(frozen.prompt_digest.as_str())
    );
    assert_eq!(
        grant.failing_source_hash.as_deref(),
        Some(intermediate_hash.as_str())
    );
    let partition = sg53_session_partition(&lifecycle, engine.session().session_id.as_str());
    assert!(!partition.join("sc-revision-assembly.json").exists());
    assert!(!partition.join("sc-revision-blocked.json").exists());
    assert!(!partition.join("sc-revision-inputs").exists());
}

#[tokio::test]
async fn sg_05_3_frozen_subprompt_delivery_missing_ref_or_digest_rejects_before_delivery_or_start()
{
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(3);
    let (turn, _original_prompt) = sg53_open_turn(&mut engine, "sg53_missing").await;
    let intermediate = sg53_intermediate_candidate();
    let intermediate_hash = sg53_sha256_hex(&intermediate);
    let instruction = sg53_instruction(&intermediate_hash);
    let context = sg53_context(&intermediate_hash);
    let resolved = sg53_resolved(&intermediate_hash);
    let frozen = build_structural_revision_prompt(
        &turn.feedback_text,
        &intermediate,
        &instruction,
        &context,
        &resolved,
    )
    .expect("freeze structural prompt");
    engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: intermediate.clone(),
        diff: None,
    });
    let session_id = engine.session().session_id.clone();
    let partition = sg53_session_partition(&lifecycle, &session_id);
    let ledger_before = lifecycle
        .get_workspace_session(&session_id)
        .expect("session")
        .provider_start_ledger
        .clone();

    enum Sg53MissingVariant {
        GrantPromptRefMissing,
        GrantPromptDigestMissing,
        FrozenRefBlank,
        FrozenNotSupplied,
    }

    for variant in [
        Sg53MissingVariant::GrantPromptRefMissing,
        Sg53MissingVariant::GrantPromptDigestMissing,
        Sg53MissingVariant::FrozenRefBlank,
        Sg53MissingVariant::FrozenNotSupplied,
    ] {
        let (label, prompt_ref, prompt_digest, frozen_arg, expected_code): (
            &str,
            Option<String>,
            Option<String>,
            Option<crate::product::workspace_engine::prompts::FrozenStructuralPrompt>,
            &str,
        ) = match variant {
            // grant 缺 prompt_ref。
            Sg53MissingVariant::GrantPromptRefMissing => (
                "grant missing prompt_ref",
                None,
                Some(frozen.prompt_digest.clone()),
                Some(frozen.clone()),
                "STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH",
            ),
            // grant 缺 prompt_digest。
            Sg53MissingVariant::GrantPromptDigestMissing => (
                "grant missing prompt_digest",
                Some(frozen.prompt_ref.clone()),
                None,
                Some(frozen.clone()),
                "STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH",
            ),
            // 冻结件缺内容寻址 ref（adjudicate ① fail-closed）。
            Sg53MissingVariant::FrozenRefBlank => {
                let mut stripped = frozen.clone();
                stripped.prompt_ref = String::new();
                (
                    "frozen missing ref",
                    Some(frozen.prompt_ref.clone()),
                    Some(frozen.prompt_digest.clone()),
                    Some(stripped),
                    "STRUCTURAL_REVISION_PROMPT_REF_MISSING",
                )
            }
            // 活动 grant 下未提供冻结件（不得回落复造原 turn prompt）。
            Sg53MissingVariant::FrozenNotSupplied => (
                "no frozen prompt supplied",
                Some(frozen.prompt_ref.clone()),
                Some(frozen.prompt_digest.clone()),
                None,
                "STRUCTURAL_REVISION_DELIVERY_PROMPT_IDENTITY_MISMATCH",
            ),
        };
        sg53_persist_structural_state(
            &lifecycle,
            &session_id,
            sg53_grant(
                &turn,
                prompt_ref,
                prompt_digest,
                &instruction,
                Some(intermediate_hash.clone()),
            ),
        );
        let error = engine
            .build_sc_revision_delivery_for_resume(&turn.command_id, &turn, frozen_arg.as_ref())
            .expect_err(label);
        assert!(error.starts_with(expected_code), "{label}: {error}");
        // 零 delivery/start：turn 仍 Reserved、ledger 不变、无组装/停等记录。
        let turn_after = lifecycle
            .get_human_gate_turn(&session_id, &turn.turn_id)
            .expect("turn");
        assert_eq!(turn_after.status, HumanGateTurnStatus::Reserved, "{label}");
        let session_after = lifecycle
            .get_workspace_session(&session_id)
            .expect("session");
        assert_eq!(
            session_after.provider_start_ledger, ledger_before,
            "{label}"
        );
        assert!(
            !partition.join("sc-revision-assembly.json").exists(),
            "{label}"
        );
        assert!(
            !partition.join("sc-revision-blocked.json").exists(),
            "{label}"
        );
        assert!(!partition.join("sc-revision-inputs").exists(), "{label}");
    }
}

#[tokio::test]
async fn sg_05_3_frozen_subprompt_delivery_grant_turn_mismatch_rejects_without_builder_fallback() {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(3);
    let (turn, opened_prompt) = sg53_open_turn(&mut engine, "sg53_mismatch").await;
    let intermediate = sg53_intermediate_candidate();
    let intermediate_hash = sg53_sha256_hex(&intermediate);
    let instruction = sg53_instruction(&intermediate_hash);
    let context = sg53_context(&intermediate_hash);
    let resolved = sg53_resolved(&intermediate_hash);
    let frozen = build_structural_revision_prompt(
        &turn.feedback_text,
        &intermediate,
        &instruction,
        &context,
        &resolved,
    )
    .expect("freeze structural prompt");
    // 候选保持原 turn 绑定候选不漂移：若实现误回落原 builder，本用例会以
    // Ok(原 prompt) 形式在 expect_err 处显式失败（不静默通过）。
    let session_id = engine.session().session_id.clone();
    let partition = sg53_session_partition(&lifecycle, &session_id);
    let ledger_before = lifecycle
        .get_workspace_session(&session_id)
        .expect("session")
        .provider_start_ledger
        .clone();

    // 两分支：grant 绑定异 turn / grant 无绑定 turn。
    let mut bound_elsewhere = sg53_grant(
        &turn,
        Some(frozen.prompt_ref.clone()),
        Some(frozen.prompt_digest.clone()),
        &instruction,
        Some(intermediate_hash.clone()),
    );
    bound_elsewhere.turn_id = Some("human_gate_turn_other_turn_0001".to_string());
    let mut unbound = bound_elsewhere.clone();
    unbound.turn_id = None;

    for (label, grant) in [
        ("grant bound to another turn", bound_elsewhere),
        ("grant has no bound turn", unbound),
    ] {
        sg53_persist_structural_state(&lifecycle, &session_id, grant);
        let error = engine
            .build_sc_revision_delivery_for_resume(&turn.command_id, &turn, Some(&frozen))
            .expect_err(label);
        assert!(
            error.starts_with("STRUCTURAL_REVISION_DELIVERY_GRANT_TURN_MISMATCH"),
            "{label}: {error}"
        );
        // 不回落任一 builder 输出（expect_err 已证非 Ok；再核错误非原
        // builder 的漂移/缺失码——失配必须以失配码显式拒绝）。
        assert_ne!(error, opened_prompt, "{label}");
        assert!(!error.starts_with("HUMAN_GATE_REVISION"), "{label}");
        // 零 delivery/start：turn 仍 Reserved、ledger 不变、无组装/停等记录。
        let turn_after = lifecycle
            .get_human_gate_turn(&session_id, &turn.turn_id)
            .expect("turn");
        assert_eq!(turn_after.status, HumanGateTurnStatus::Reserved, "{label}");
        let session_after = lifecycle
            .get_workspace_session(&session_id)
            .expect("session");
        assert_eq!(
            session_after.provider_start_ledger, ledger_before,
            "{label}"
        );
        assert!(
            !partition.join("sc-revision-assembly.json").exists(),
            "{label}"
        );
        assert!(
            !partition.join("sc-revision-blocked.json").exists(),
            "{label}"
        );
        assert!(!partition.join("sc-revision-inputs").exists(), "{label}");
    }
}

#[tokio::test]
async fn sg_05_3_frozen_subprompt_delivery_without_state_keeps_original_builder_and_fresh_none_resume()
 {
    let (_root, lifecycle, mut engine) = super::conversational_gate::gate_fixture(3);
    let (turn, opened_prompt) = sg53_open_turn(&mut engine, "sg53_none").await;
    let session_id = engine.session().session_id.clone();
    // SG6a.3（Main 预授权迁移 2026-10-14）：复审④ grant producer 落地后
    // 真实 reserve 恒创建 current_grant——本用例锁定的「无 grant 记录」
    // legacy 形态以 fixture 直写清空 current_grant 模拟（SG4b 直写先
    // 例），fallback 分支（无 grant 记录的 join 面）覆盖保持不变。
    {
        let mut legacy = lifecycle
            .get_workspace_session(&session_id)
            .expect("load session record for legacy shape");
        if let Some(state) = legacy.structural_governance_state.as_mut() {
            state.current_grant = None;
        }
        crate::product::json_store::write_json(
            &lifecycle
                .app_paths()
                .issue_lifecycle_root(&legacy.project_id, &legacy.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", legacy.id)),
            &legacy,
        )
        .expect("persist legacy no-grant fixture");
    }

    // durable 结构面观察：reserve 已物化聚合 state（SG4a hold），但无
    // current_grant＝无结构授权窗口（fresh None resume 的真实前置）。
    let record = lifecycle
        .get_workspace_session(&session_id)
        .expect("session");
    let governance = record.structural_governance_state.as_ref();
    assert!(governance.is_some(), "reserve materializes the state");
    assert!(
        governance
            .and_then(|state| state.current_grant.as_ref())
            .is_none(),
        "no active structural grant"
    );
    assert_eq!(
        governance
            .and_then(|state| state.user_holds.first())
            .map(|hold| hold.turn_id.as_str()),
        Some(turn.turn_id.as_str()),
        "reserve hold is the real durable shape"
    );

    // fresh None resume：delivery 落原 fresh builder 分支——与开门时交付的
    // 完整 SC manual prompt 逐字节一致（原 builder 逐字段不变）。
    let delivered = engine
        .build_sc_revision_delivery_for_resume(&turn.command_id, &turn, None)
        .expect("no structural state falls back to the original fresh builder");
    assert_eq!(delivered, opened_prompt);
    assert_eq!(
        delivered,
        engine
            .build_sc_manual_revision_prompt_for_fresh_turn(&turn)
            .expect("original fresh builder stays reachable and unchanged")
    );

    // 原 builder 漂移拒绝语义保留：候选漂移后同码 fail-closed。
    engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Work Item Plan\n\n## 漂移候选\n".to_string(),
        diff: None,
    });
    let drifted = engine
        .build_sc_revision_delivery_for_resume(&turn.command_id, &turn, None)
        .expect_err("drifted candidate must reject");
    assert!(drifted.starts_with("HUMAN_GATE_REVISION_CANDIDATE_DRIFTED"));
    let drifted_direct = engine
        .build_sc_manual_revision_prompt_for_fresh_turn(&turn)
        .expect_err("original builder rejects the same drift");
    assert_eq!(drifted, drifted_direct);
}
