//! P27 real interruption/Unknown and five nonterminal package-negative cases.
//! NotStarted is a supplied observation only; never synthesize it from a refusal.
//! Existing cancellation test uses manager.abort_active_run; do not duplicate it.

use super::sc_failure_driver_fixture::{
    chain_ids, durable_sc_package_versions, evidence_entries, events_of,
    failure_message, markdown_missing_five_sections, markdown_unknown_requirement_ref,
    only_chain_id, start_run, start_run_with,
};
use super::single_candidate_provider_run::{
    single_candidate_markdown, wait_for_single_candidate_phase, wait_for_stage,
    ProviderRunFixture,
};
use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamChunk, StreamingProviderInput,
};
use crate::product::models::SingleCandidatePhase;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::web::workspace_ws_handler::StreamingProviderAdapter;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

/// Deterministic adapter, not a fake HTTP service. At callback 2 either return an
/// actual session with no delivery, or refuse to return a session at all.
struct InterruptedProvider {
    first: String,
    refuse: bool,
    callbacks: AtomicUsize,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for InterruptedProvider {
    async fn start(&self, input: StreamingProviderInput, _cancel: CancellationToken)
        -> Result<ProviderSession, ProviderAdapterError>
    {
        self.inputs.send(input).unwrap();
        let ordinal = self.callbacks.fetch_add(1, Ordering::SeqCst);
        if ordinal > 0 && self.refuse {
            return Err(ProviderAdapterError::provider_unavailable("fixture start refusal"));
        }
        let (events, receiver) = mpsc::channel(4);
        let (commands, _command_rx) = mpsc::channel(1);
        if ordinal == 0 {
            let output = self.first.clone();
            tokio::spawn(async move {
                events.send(ProviderEvent::Completed(
                    ProviderCompletion::plain(output, None))).await.unwrap();
            });
        } else {
            drop(events);
        }
        Ok(ProviderSession { native_session_id: None, events: receiver, commands })
    }

    async fn run_streaming(&self, _: &crate::protocol::contracts::AdapterInput,
        _: CancellationToken) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError>
    {
        unreachable!("SC driver uses start")
    }
}

/// Synchronization is a durable predicate, not an ignored timeout/settling sleep.
async fn wait_for_waiting(fixture: &ProviderRunFixture) -> serde_json::Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let record = fixture.lifecycle.get_workspace_session(&fixture.record.id).unwrap();
            let value = serde_json::to_value(record).unwrap();
            if value["lc_resume_recovery"]["state"] == "waiting" { return value; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("real Unknown branch must durably enter Waiting")
}

fn assert_no_terminal_package(fixture: &ProviderRunFixture) {
    assert!(durable_sc_package_versions(fixture).is_empty());
    for chain in chain_ids(fixture) {
        assert!(events_of(&evidence_entries(fixture, &chain), "chain_terminal").is_empty());
    }
}

#[tokio::test]
async fn sc_failure_teaching_interrupted_has_started_observation() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let first = markdown_missing_five_sections(&fixture.story_id, &fixture.design_id, "Excluded.");
    let (inputs, receiver) = mpsc::unbounded_channel();
    let provider = Arc::new(InterruptedProvider {
        first, refuse: false, inputs, callbacks: AtomicUsize::new(0),
    });
    let mut run = start_run_with(fixture, provider, receiver).await;
    let _ = run.queue.next_input().await;
    let _ = run.queue.next_input().await;
    assert!(!failure_message(&mut run.outbound).await.is_empty());
    wait_for_single_candidate_phase(&run.fixture, SingleCandidatePhase::Failed).await;
    run.queue.expect_no_further_input().await;
    assert_eq!(run.queue.received_count(), 2);
    let entries = evidence_entries(&run.fixture, &only_chain_id(&run.fixture));
    let failures = events_of(&entries, "failure_observed");
    assert_eq!(failures.len(), 1);
    let terminals = events_of(&entries, "chain_terminal");
    assert_eq!(terminals.len(), 1);
    let terminal = &terminals[0]["event"];
    assert_eq!(terminal["terminal_class"], "teaching_interrupted");
    assert_eq!(terminal["completeness"], "incomplete");
    let interrupted = &terminal["last_physical_round_id"];
    assert_ne!(*interrupted, failures[0]["event"]["round_id"]);
    let observation = terminal["launch_observations"].as_array().unwrap().iter()
        .find(|pair| pair[0] == *interrupted).unwrap();
    assert_eq!(observation[1], "started");
    assert!(terminal["missing_evidence"].as_array().unwrap().iter()
        .any(|item| item["round_id"] == *interrupted && item["kind"] == "provider_output"));
    let versions = durable_sc_package_versions(&run.fixture);
    assert_eq!(versions.len(), 1);
    let package = &versions[0]["sc_failure_repair_package"];
    assert_eq!(package["terminal_class"], "teaching_interrupted");
    assert_eq!(package["completeness"], "incomplete");
    assert_eq!(package["rounds"].as_array().unwrap().len(), 2);
    assert_eq!(package["rounds"][1]["launch_state"], "started");
    assert_eq!(package["last_provider_output"]["status"], "missing");
    assert_eq!(package["last_failure_source"]["source_round_id"],
        failures[0]["event"]["round_id"]);
}

#[tokio::test]
async fn sc_failure_teaching_start_refused_is_real_unknown_waiting() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let first = markdown_missing_five_sections(&fixture.story_id, &fixture.design_id, "Excluded.");
    let (inputs, receiver) = mpsc::unbounded_channel();
    let provider = Arc::new(InterruptedProvider {
        first, refuse: true, inputs, callbacks: AtomicUsize::new(0),
    });
    let mut run = start_run_with(fixture, provider, receiver).await;
    let _ = run.queue.next_input().await;
    let _refused_callback = run.queue.next_input().await;
    let state = wait_for_waiting(&run.fixture).await;
    assert_ne!(state["single_candidate_phase"], "failed");
    run.queue.expect_no_further_input().await;
    assert_eq!(run.queue.received_count(), 2, "callbacks are not successful launches");
    let record = run.fixture.lifecycle.get_workspace_session(&run.fixture.record.id).unwrap();
    let window = record.sc_generation_teaching_windows.last().unwrap();
    let attempt = window.attempts.iter().find(|attempt| attempt.attempt_no == 2).unwrap();
    let ledger: Vec<_> = record.provider_start_ledger.iter().filter(|entry|
        entry.provider_attempt_key.as_deref() == Some(attempt.attempt_key.as_str())).collect();
    assert_eq!(ledger.len(), 1, "claimed placeholder remains attributable");
    assert!(!ledger[0].is_actual_launch_observation());
    let entries = evidence_entries(&run.fixture, &only_chain_id(&run.fixture));
    assert_eq!(events_of(&entries, "round_bound").len(), 2);
    assert_eq!(events_of(&entries, "failure_observed").len(), 1);
    assert_no_terminal_package(&run.fixture);
    let prompts = events_of(&entries, "prompt_prepared");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0]["event"]["evidence"]["disposition"], "prepared");
}

/// Five real nonterminal scenarios. Existing tests exercise their flow, but this
/// test adds the missing durable zero-A4/zero-chain-terminal assertions.
#[tokio::test]
async fn sc_failure_nonterminal_five_cases_publish_no_failure_package() {
    for scenario in ["success", "compile_recovered", "ir_recovered",
        "structure_recovered", "admission_waiting"]
    {
        let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
        let good = single_candidate_markdown(&fixture.story_id, &fixture.design_id);
        let outputs = match scenario {
            "compile_recovered" => vec![markdown_missing_five_sections(
                &fixture.story_id, &fixture.design_id, "Excluded."), good],
            "ir_recovered" => vec![markdown_unknown_requirement_ref(
                &fixture.story_id, &fixture.design_id), good],
            "structure_recovered" => vec![good.replacen(
                "### Inputs\n\n### Outputs",
                "### Inputs\n- contract_id: contract.backend-internal\n- provider_logical_work_item_id: WI-001\n- required_capabilities: api.backend.read\n- compatibility_policy: require_all\n\n### Outputs", 1), good],
            _ => vec![good],
        };
        if scenario == "admission_waiting" {
            std::fs::remove_file(fixture.member_checkout_root()
                .join(".claude/rules/language.md")).unwrap();
        }
        let expected = if scenario == "admission_waiting" { 0 }
            else if scenario == "success" { 1 } else { 2 };
        let mut run = start_run(fixture, outputs).await;
        for _ in 0..expected { let _ = run.queue.next_input().await; }
        if scenario == "admission_waiting" {
            assert!(!failure_message(&mut run.outbound).await.is_empty());
            let record = run.fixture.lifecycle.get_workspace_session(&run.fixture.record.id).unwrap();
            // 成员语言规则缺失属「准入期 fail-closed，先于 provider 启动」：
            // 既有产品用例 `single_candidate_author_rejects_missing_language_rules_before_provider_start`
            // 明确要求 phase = Failed 且零 provider 输入（本矩阵已断言 received_count()==0）。
            // 该路径没有观察到机械失败 ⇒ 不应产生终局包（下方 assert_no_terminal_package）。
            assert_eq!(record.single_candidate_phase, Some(SingleCandidatePhase::Failed));
        } else {
            wait_for_stage(&run.fixture.engine, WorkspaceStage::HumanConfirm).await;
        }
        run.queue.expect_no_further_input().await;
        assert_eq!(run.queue.received_count(), expected, "{scenario}");
        assert_no_terminal_package(&run.fixture);
        for chain in chain_ids(&run.fixture) {
            let entries = evidence_entries(&run.fixture, &chain);
            assert_eq!(events_of(&entries, "failure_observed").len(),
                usize::from(expected == 2), "{scenario}");
        }
    }
}
