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
