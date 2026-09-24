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

/// C2（REQ-HGC-03→修订教学逃生/tasks 4.1）：反馈点名结构性变更时允许受影响
/// 闭包联动+影响面声明（Notes 自由文本区，grammar 标题仍是第一行）；禁删
/// 必需字段/绕过 validator 的反面清单不放宽。
#[test]
fn conversational_gate_revision_teaching_carries_structural_escape_clause() {
    let candidate =
        "# Work Item Plan\n\n## Work Item WI-001: 当前候选\n\n### Outputs\n- contract_id: CT-001\n";
    let prompt = build_sc_manual_revision_prompt(ScManualRevisionPromptInput {
        candidate_markdown: candidate,
        feedback: "把 TASK-003 移动到 WI-002，并同步受影响的完成判据",
        grammar_boundary: "[markdown_grammar]\n标题必须逐字为 # Work Item Plan；Handoff Schema 必须保留三个字段。",
        language_rule: LANGUAGE_RULE_FIXTURE,
    })
    .expect("revision prompt should fit the contract budget");

    assert!(
        prompt.contains("结构性变更"),
        "missing escape clause trigger wording: {prompt}"
    );
    assert!(
        prompt.contains("受影响的闭包"),
        "missing affected-closure linkage permission: {prompt}"
    );
    assert!(
        prompt.contains("影响面"),
        "missing impact-declaration requirement: {prompt}"
    );
    assert!(
        prompt.contains("Notes"),
        "declaration must be placed in the free-text Notes section: {prompt}"
    );
    // 反面清单不放宽：禁删字段/绕 validator 维持。
    assert!(
        prompt.contains("禁止删字段"),
        "missing delete-fields prohibition: {prompt}"
    );
    assert!(
        prompt.contains("绕过 grammar 或 validator") || prompt.contains("绕过 validator"),
        "missing validator-bypass prohibition: {prompt}"
    );
    assert!(
        prompt.len() <= SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES,
        "escape clause must fit the teaching budget: {} > {}",
        prompt.len(),
        SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES
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
        WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES, 22_000,
        "SC author red-line budget must remain independent from the revision budget (11th raise: trusted-command duplication teaching)"
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
        accepted_feedback_turns: None,
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

