use cadence_aria::cross_cutting::provider_adapter::{
    FakeProviderAdapter, ProviderAdapter, parse_last_structured_output,
};
use cadence_aria::cross_cutting::provider_router::{ProviderRouter, ProviderRunRequest};
use cadence_aria::cross_cutting::provider_run::write_provider_run_record;
use cadence_aria::protocol::contracts::{
    AdapterInput, AdapterRole, ApprovalPolicy, ProviderRunStatus, ProviderType, RuntimeRole,
    SandboxMode, TimeoutStatus,
};
use serde_json::{Value, json};

#[test]
fn adapter_input_and_output_use_shared_snake_case_contract() {
    let input = adapter_input(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/provider/fake_stdout_clarification.txt"
    )));
    let input_json = serde_json::to_value(&input).expect("input json");
    assert_eq!(input_json["provider_type"], json!("fake"));
    assert_eq!(input_json["role"], json!("orchestrator"));
    assert_eq!(input_json["worktree_path"], Value::Null);
    assert_eq!(
        input_json["output_schema"],
        json!("clarification_record.v1")
    );
    assert_eq!(input_json["max_retries"], json!(1));

    let adapter = FakeProviderAdapter;
    let output = adapter.run(&input).expect("fake provider output");
    let output_json = serde_json::to_value(&output).expect("output json");
    assert_eq!(output_json["exit_code"], json!(0));
    assert_eq!(output_json["timeout_status"], json!("not_timed_out"));
    assert_eq!(
        output_json["structured_output"]["artifact_kind"],
        json!("clarification_record")
    );
}

#[test]
fn runtime_role_maps_advisory_reviewer_to_adapter_reviewer() {
    assert_eq!(
        RuntimeRole::AdvisoryReviewer.adapter_role(),
        AdapterRole::Reviewer
    );
    assert!(RuntimeRole::AdvisoryReviewer.advisory_only());
    assert_eq!(RuntimeRole::Executor.adapter_role(), AdapterRole::Executor);
    assert!(!RuntimeRole::Executor.advisory_only());
}

#[test]
fn fake_provider_parses_last_structured_output_sentinel_and_keeps_raw_stdout() {
    let stdout = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/provider/fake_stdout_clarification.txt"
    ));
    let structured = parse_last_structured_output(stdout)
        .expect("parse sentinel")
        .expect("structured output");
    assert_eq!(structured["artifact_kind"], json!("clarification_record"));
    assert_eq!(structured["goal_summary"], json!("fixture goal"));

    let input = adapter_input(stdout);
    let output = FakeProviderAdapter.run(&input).expect("adapter output");
    assert!(output.stdout.starts_with("provider log line"));
    assert_eq!(output.stderr, "");
    assert_eq!(output.files_modified, Vec::<String>::new());
    assert_eq!(output.timeout_status, TimeoutStatus::NotTimedOut);
}

#[test]
fn parser_accepts_fenced_json_inside_structured_output_sentinel() {
    let stdout = "provider log\n<ARIA_STRUCTURED_OUTPUT>\n```json\n{\"artifact_kind\":\"clarification_record\"}\n```\n</ARIA_STRUCTURED_OUTPUT>\n";
    let structured = parse_last_structured_output(stdout)
        .expect("parse sentinel")
        .expect("structured output");

    assert_eq!(structured["artifact_kind"], json!("clarification_record"));
}

#[test]
fn provider_router_records_completed_run_with_external_raw_output_refs() {
    let input = adapter_input(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/provider/fake_stdout_clarification.txt"
    )));
    let router = ProviderRouter::new(Box::new(FakeProviderAdapter));
    let (output, record) = router
        .run(
            ProviderRunRequest {
                provider_run_id: "run_001".to_string(),
                node_id: "N04".to_string(),
                runtime_role: RuntimeRole::Orchestrator,
                provider_capability_ref: "cap_fake_001".to_string(),
                adapter_compatibility_ref: "compat_fake_001".to_string(),
                context_package_ref: "ctx_001".to_string(),
                adapter_input_ref: "adapter_input_001".to_string(),
                adapter_output_ref: "adapter_output_001".to_string(),
                approval_policy: ApprovalPolicy::OnRequest,
                sandbox_mode: SandboxMode::WorkspaceWrite,
                constraint_check_ref: Some("chk_001".to_string()),
                traceability_binding_refs: vec!["bind_001".to_string()],
            },
            input,
        )
        .expect("provider run");

    assert_eq!(output.exit_code, Some(0));
    assert_eq!(record.provider_run_id, "run_001");
    assert_eq!(record.node_id, "N04");
    assert_eq!(record.provider_type, ProviderType::Fake);
    assert_eq!(record.runtime_role, RuntimeRole::Orchestrator);
    assert_eq!(record.adapter_role, AdapterRole::Orchestrator);
    assert_eq!(record.status, ProviderRunStatus::Completed);
    assert_eq!(record.exit_code, Some(0));
    assert_eq!(record.error_code, None);
    assert_eq!(record.timeout_status, TimeoutStatus::NotTimedOut);
    assert_eq!(
        record.raw_artifact_refs,
        vec![
            "ext_run_001_stdout".to_string(),
            "ext_run_001_stderr".to_string(),
            "ext_run_001_structured_output".to_string()
        ]
    );
    assert_eq!(record.stdout_ref, Some("ext_run_001_stdout".to_string()));
    assert_eq!(record.stderr_ref, Some("ext_run_001_stderr".to_string()));
    assert_eq!(
        record.structured_output_ref,
        Some("ext_run_001_structured_output".to_string())
    );
    assert_eq!(record.constraint_check_ref, Some("chk_001".to_string()));
    assert_eq!(
        record.traceability_binding_refs,
        vec!["bind_001".to_string()]
    );

    let tempdir = tempfile::tempdir().expect("tempdir");
    let record_path = tempdir.path().join("provider-runs/run_001/run.json");
    write_provider_run_record(&record_path, &record).expect("write run record");
    let persisted: Value =
        serde_json::from_slice(&std::fs::read(record_path).expect("read record"))
            .expect("record json");
    assert_eq!(
        persisted["raw_artifact_refs"][0],
        json!("ext_run_001_stdout")
    );
    assert_eq!(persisted["adapter_output_ref"], json!("adapter_output_001"));
}

fn adapter_input(prompt: &str) -> AdapterInput {
    AdapterInput {
        provider_type: ProviderType::Fake,
        role: AdapterRole::Orchestrator,
        worktree_path: None,
        prompt: prompt.to_string(),
        context_files: vec!["tests/fixtures/artifacts/spec.md".to_string()],
        output_schema: "clarification_record.v1".to_string(),
        timeout: 30,
        max_retries: 1,
    }
}
