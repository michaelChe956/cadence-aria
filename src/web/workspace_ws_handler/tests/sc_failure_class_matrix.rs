//! P27 remaining real-driver matrix; mount beside sc_failure_driver_fixture.
//! No test-side terminal builder/publisher and no fabricated provider-start rows.
//! FirstDenied seeds an explicitly unapproved durable policy window using the real
//! establish API before any authorization exists. The real failure gate must then
//! call authorize/consume, observe that window, and record Denied without a start.

use super::sc_failure_driver_fixture::{
    candidate_attempts, durable_sc_package_versions, events_of, evidence_entries,
    extract_source_body, failure_message, markdown_missing_five_sections,
    markdown_unknown_requirement_ref, only_chain_id, sha256_hex, start_run,
};
use super::single_candidate_provider_run::*;
use super::single_candidate_provider_run::{
    ProviderRunFixture, single_candidate_markdown, wait_for_single_candidate_phase,
};
use crate::product::models::SingleCandidatePhase;
use crate::product::work_item_plan_policy::{ScTeachingWindowRequest, WorkItemPlanFlowKind};
use serde_json::Value;

#[derive(Clone, Copy)]
enum Class {
    Compile,
    Ir,
    Structure,
}

impl Class {
    fn token(self) -> &'static str {
        match self {
            Self::Compile => "compile",
            Self::Ir => "ir_preflight",
            Self::Structure => "structure_gate",
        }
    }

    fn output(self, fixture: &ProviderRunFixture) -> String {
        let story = &fixture.story_id;
        let design = &fixture.design_id;
        match self {
            Self::Compile => markdown_missing_five_sections(story, design, "Frontend excluded."),
            Self::Ir => markdown_unknown_requirement_ref(story, design),
            // Exact existing private self-cycle fixture transformation; no new grammar.
            Self::Structure => single_candidate_markdown(story, design).replacen(
                "### Inputs\n\n### Outputs",
                "### Inputs\n- contract_id: contract.backend-internal\n- provider_logical_work_item_id: WI-001\n- required_capabilities: api.backend.read\n- compatibility_policy: require_all\n\n### Outputs",
                1,
            ),
        }
    }
}

/// Legitimate durable policy denial, not budget exhaustion fabricated with starts.
/// There is no source/prompt/failure yet: absent material stays absent, never a fake
/// digest or source ref. Establish evaluates NoProvableAuthorization and max=1.
/// Its attempt-1 placeholder is policy state, NOT an observed provider launch.
fn seed_unapproved_policy_window(fixture: &ProviderRunFixture) {
    let before = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .unwrap();
    assert!(before.provider_start_ledger.is_empty());
    assert!(before.sc_generation_teaching_windows.is_empty());
    assert_ne!(
        before.single_candidate_phase,
        Some(SingleCandidatePhase::Generate)
    );
    let request = ScTeachingWindowRequest {
        provider_path_admissible: false,
        failure_source_ref: None,
        failure_source_sha256: String::new(),
        failure_typed_kinds: Vec::new(),
        business_request_digest: String::new(),
        material_digests: Vec::new(),
        context_digest: String::new(),
        allowed_edit_closure: String::new(),
        instruction_digest: String::new(),
        prompt_ref: String::new(),
        prompt_digest: String::new(),
    };
    fixture
        .lifecycle
        .establish_sc_teaching_window(&before, &request)
        .unwrap();
    let after = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .unwrap();
    assert_eq!(after.provider_start_ledger, before.provider_start_ledger);
    let window = after.sc_generation_teaching_windows.last().unwrap();
    assert!(!window.repair_once_approved);
    assert_eq!(window.max_attempts, 1);
    assert_eq!(window.attempts.len(), 1);
    assert!(window.attempts[0].launch_ledger_key.is_none());
    assert!(window.claimable_attempt_no().is_none());
}

fn contains_string(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(text) => text == expected,
        Value::Array(items) => items.iter().any(|item| contains_string(item, expected)),
        Value::Object(fields) => fields.values().any(|item| contains_string(item, expected)),
        _ => false,
    }
}

/// Exact public durable node reference, not a substring search in session JSON.
fn assert_published_node_reference(fixture: &ProviderRunFixture, version: &Value) {
    let number = version["version"].as_u64().unwrap();
    let node = version["source_node_id"].as_str().unwrap();
    let detail = fixture
        .lifecycle
        .load_node_detail(&fixture.record.id, node)
        .unwrap();
    let reference = serde_json::to_value(detail.artifact_ref.expect("published node ref")).unwrap();
    assert!(
        contains_string(&reference, &format!("artifact_version_{number:03}")),
        "node ref must name the same durable A4 version: {reference}"
    );
}

fn assert_rounds(package: &Value, entries: &[Value], expected: &[Class]) {
    let observed = events_of(entries, "failure_observed");
    let bound = events_of(entries, "round_bound");
    let rounds = package["rounds"].as_array().unwrap();
    assert_eq!(observed.len(), expected.len());
    assert_eq!(rounds.len(), expected.len());
    assert_eq!(bound.len(), expected.len());
    for ((failure, round), class) in observed.iter().zip(rounds).zip(expected) {
        let fact = &failure["event"];
        assert_eq!(fact["failure_class"], class.token());
        let identity = &bound
            .iter()
            .find(|entry| entry["event"]["round"]["round_id"] == fact["round_id"])
            .unwrap()["event"]["round"];
        assert_eq!(&round["identity"], identity);
        assert_eq!(round["launch_state"], "started");
        // Full ordered values, not lengths/summaries/first-four slices.
        for field in [
            "failure_class",
            "blocking_reasons",
            "repair_instructions",
            "raw",
            "source",
            "existing_failure_id",
            "attempt_key",
        ] {
            assert_eq!(round[field], fact[field], "round field {field}");
        }
        assert_eq!(fact["raw"]["status"], "available");
        assert_eq!(fact["source"]["status"], "available");
        assert!(!fact["blocking_reasons"].as_array().unwrap().is_empty());
        if matches!(class, Class::Compile) {
            // 「原因不截断」：五段缺失的真实诊断必须多于四条全部落盘。
            assert!(fact["blocking_reasons"].as_array().unwrap().len() > 4);
            // 该样本（五段 section 缺失）在生产上**不产出** contract-autorepair 指令
            // （真实为 0 条），故这里只钉「若存在指令，则不得是截断摘要」；否则测试
            // 会与真实产出矛盾。`>4 条指令不截断` 的边界由
            // `MAX_AUTOREPAIR_INSTRUCTION_LINES` 的自身单测覆盖，本矩阵不重复。
            let instructions = fact["repair_instructions"].as_array().unwrap();
            assert!(
                !instructions.iter().any(|line| line
                    .as_str()
                    .is_some_and(|line| line.contains("条契约修复指令"))),
                "证据里的修复指令不得是截断摘要：{instructions:?}"
            );
        }
        if matches!(class, Class::Structure) {
            assert!(
                !fact["source"]["locator"]
                    .as_str()
                    .unwrap()
                    .starts_with("sc-split-output/")
            );
            assert!(!fact["existing_failure_id"].as_str().unwrap().is_empty());
        }
    }
}

async fn run_cell(class: Class, first_denied: bool) {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    if first_denied {
        seed_unapproved_policy_window(&fixture);
    }
    let first = class.output(&fixture);
    let second = first.replace("Backend API", "Backend API round-2");
    assert_ne!(
        first, second,
        "second attempt must have its own source identity"
    );
    let outputs = if first_denied {
        vec![first]
    } else {
        vec![first, second]
    };
    let mut run = start_run(fixture, outputs).await;
    let _original = run.queue.next_input().await;
    let teaching = if first_denied {
        None
    } else {
        Some(run.queue.next_input().await)
    };
    assert!(!failure_message(&mut run.outbound).await.is_empty());
    wait_for_single_candidate_phase(&run.fixture, SingleCandidatePhase::Failed).await;
    run.queue.expect_no_further_input().await;
    let count = if first_denied { 1 } else { 2 };
    assert_eq!(run.queue.received_count(), count);

    let chain_id = only_chain_id(&run.fixture);
    let entries = evidence_entries(&run.fixture, &chain_id);
    let decisions = events_of(&entries, "teaching_decision");
    let actual_decisions: Vec<_> = decisions
        .iter()
        .map(|entry| entry["event"]["decision"].as_str().unwrap())
        .collect();
    assert_eq!(
        actual_decisions,
        if first_denied {
            vec!["denied"]
        } else {
            vec!["granted", "denied"]
        }
    );
    let terminal = events_of(&entries, "chain_terminal");
    assert_eq!(terminal.len(), 1);
    assert_eq!(terminal[0]["event"]["terminal_class"], class.token());
    let packages = durable_sc_package_versions(&run.fixture);
    assert_eq!(packages.len(), 1);
    let package = &packages[0]["sc_failure_repair_package"];
    assert_eq!(package["chain_id"], chain_id);
    assert_eq!(
        package["terminal_class"],
        terminal[0]["event"]["terminal_class"]
    );
    assert_eq!(package["completeness"], "complete");
    assert_rounds(package, &entries, &vec![class; count]);
    assert_published_node_reference(&run.fixture, &packages[0]);

    // Prompt was necessarily built from the PREVIOUS failure, never the output it
    // had not yet requested. This fixes the draft's observed.last() source mistake.
    let observed = events_of(&entries, "failure_observed");
    if let Some(input) = teaching {
        let source = &observed[0]["event"]["source"];
        let body = extract_source_body(
            &input.prompt,
            source["sha256"].as_str().unwrap(),
            source["bytes"].as_u64().unwrap(),
        );
        assert_eq!(
            sha256_hex(body.as_bytes()),
            source["sha256"].as_str().unwrap()
        );
        assert!(input.prompt.contains(source["locator"].as_str().unwrap()));
        let second_source = &observed[1]["event"]["source"];
        assert_ne!(source["sha256"], second_source["sha256"]);
        assert_eq!(package["last_failure_source"]["source"], *second_source);
        assert_eq!(
            package["last_failure_source"]["source_round_id"],
            observed[1]["event"]["round_id"]
        );
    }
    // Raw must belong to the bound physical run, not another candidate/source.
    let scope: crate::web::workspace_ws_types::ScFailureScope =
        serde_json::from_value(entries[0]["chain"]["scope"].clone()).unwrap();
    for binding in events_of(&entries, "round_bound") {
        let identity: crate::web::workspace_ws_types::ScRoundIdentity =
            serde_json::from_value(binding["event"]["round"].clone()).unwrap();
        let raw = run
            .fixture
            .lifecycle
            .read_sc_split_output(&scope, &identity)
            .unwrap();
        let failures = events_of(&entries, "failure_observed");
        let failure = failures
            .iter()
            .find(|entry| entry["event"]["round_id"].as_str() == Some(identity.round_id.as_str()))
            .unwrap();
        assert_eq!(
            failure["event"]["raw"]["sha256"],
            sha256_hex(raw.as_bytes())
        );
        assert_eq!(failure["event"]["raw"]["bytes"], raw.len() as u64);
    }
    // Match sealed IDs against the OLD candidate-attempt producer, rather than
    // recomputing IDs from new evidence or comparing two immediate rereads.
    let attempts = candidate_attempts(&run.fixture);
    for fact in &observed {
        if let Some(id) = fact["event"]["existing_failure_id"].as_str() {
            let old = attempts
                .iter()
                .find(|row| {
                    row.failure
                        .as_ref()
                        .is_some_and(|failure| failure.failure_id == id)
                })
                .expect("old sealed failure ID");
            if let Some(key) = fact["event"]["attempt_key"].as_str() {
                assert_eq!(old.attempt_key, key);
            }
        }
    }
    let record = run
        .fixture
        .lifecycle
        .get_workspace_session(&run.fixture.record.id)
        .unwrap();
    let window = record.sc_generation_teaching_windows.last().unwrap();
    assert_eq!(window.attempts.len(), count);
    if first_denied {
        assert!(!window.repair_once_approved);
    }
}

#[tokio::test]
async fn sc_failure_matrix_compile_first_policy_denied() {
    run_cell(Class::Compile, true).await;
}
#[tokio::test]
async fn sc_failure_matrix_ir_first_policy_denied() {
    run_cell(Class::Ir, true).await;
}
#[tokio::test]
async fn sc_failure_matrix_structure_first_policy_denied() {
    run_cell(Class::Structure, true).await;
}
#[tokio::test]
async fn sc_failure_matrix_compile_second_failure() {
    run_cell(Class::Compile, false).await;
}
#[tokio::test]
async fn sc_failure_matrix_ir_second_failure_full_source() {
    run_cell(Class::Ir, false).await;
}
#[tokio::test]
async fn sc_failure_matrix_structure_second_failure_full_source() {
    run_cell(Class::Structure, false).await;
}
