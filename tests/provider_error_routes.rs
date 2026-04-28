mod support;

use cadence_aria::cross_cutting::adapter_compatibility::fixture_compatibility_entry;
use cadence_aria::cross_cutting::cli_adapter::{CliAdapterConfig, CliProviderAdapter};
use cadence_aria::cross_cutting::provider_adapter::ProviderAdapter;
use cadence_aria::cross_cutting::provider_router::ProviderRunRequest;
use cadence_aria::cross_cutting::provider_run::failed_provider_run_record_from_error;
use cadence_aria::protocol::contracts::{
    AdapterInput, AdapterRole, ApprovalPolicy, ProviderRunStatus, ProviderType, RuntimeRole,
    SandboxMode,
};
use cadence_aria::protocol::provider_errors::{
    route_provider_error, ProviderErrorCode, ProviderErrorRoute,
};

#[test]
fn unauthorized_and_permission_denied_are_stable_manual_intervention_errors() {
    let unauthorized = run_error(support::unauthorized_provider_script(), 3);
    assert_eq!(unauthorized.code, ProviderErrorCode::ProviderUnauthorized);
    assert_eq!(
        route_provider_error(&unauthorized.code, 0, 3),
        ProviderErrorRoute::ManualIntervention
    );

    let denied = run_error(support::permission_denied_provider_script(), 3);
    assert_eq!(denied.code, ProviderErrorCode::ProviderPermissionDenied);
    assert_eq!(
        route_provider_error(&denied.code, 0, 3),
        ProviderErrorRoute::ManualIntervention
    );
}

#[test]
fn parse_error_timeout_and_incompatible_output_have_stable_routes() {
    let parse_error = run_error(support::parse_error_provider_script(), 3);
    assert_eq!(parse_error.code, ProviderErrorCode::ProviderParseError);
    assert_eq!(
        route_provider_error(&parse_error.code, 0, 3),
        ProviderErrorRoute::Retry
    );
    assert_eq!(
        route_provider_error(&parse_error.code, 1, 3),
        ProviderErrorRoute::Gate
    );

    let timeout = run_error(support::timeout_provider_script(), 1);
    assert_eq!(timeout.code, ProviderErrorCode::ProviderTimeout);
    assert_eq!(
        route_provider_error(&timeout.code, 0, 2),
        ProviderErrorRoute::Retry
    );
    assert_eq!(
        route_provider_error(&timeout.code, 2, 2),
        ProviderErrorRoute::ManualIntervention
    );

    let incompatible = run_error(support::incompatible_output_provider_script(), 3);
    assert_eq!(
        incompatible.code,
        ProviderErrorCode::ProviderIncompatibleOutput
    );
    assert_eq!(
        route_provider_error(&incompatible.code, 0, 3),
        ProviderErrorRoute::Gate
    );
}

#[test]
fn failed_provider_run_record_contains_error_code_and_audit_fields() {
    let error = run_error(support::parse_error_provider_script(), 3);
    let input = adapter_input(3);
    let request = provider_run_request();

    let record = failed_provider_run_record_from_error(&request, &input, &error);

    assert_eq!(record.provider_run_id, "run_error_001");
    assert_eq!(record.status, ProviderRunStatus::Failed);
    assert_eq!(record.error_code, Some("provider_parse_error".to_string()));
    assert!(record
        .error_details
        .expect("error details")
        .contains("structured output"));
    assert_eq!(record.provider_capability_ref, "cap_fixture_001");
    assert_eq!(record.adapter_compatibility_ref, "compat_fixture_001");
    assert_eq!(record.approval_policy, ApprovalPolicy::OnRequest);
    assert_eq!(record.sandbox_mode, SandboxMode::WorkspaceWrite);
}

fn run_error(
    script: &str,
    timeout: u64,
) -> cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let command = support::write_executable_script(tempdir.path(), "fixture-provider", script);
    let compatibility = fixture_compatibility_entry(ProviderType::Fake, command);
    let adapter = CliProviderAdapter::new(CliAdapterConfig {
        compatibility,
        expected_artifact_kind: Some("clarification_record".to_string()),
    });

    adapter
        .run(&adapter_input(timeout))
        .expect_err("fixture script should fail")
}

fn adapter_input(timeout: u64) -> AdapterInput {
    AdapterInput {
        provider_type: ProviderType::Fake,
        role: AdapterRole::Orchestrator,
        worktree_path: None,
        prompt: "fixture prompt".to_string(),
        context_files: Vec::new(),
        output_schema: "clarification_record.v1".to_string(),
        timeout,
        max_retries: 1,
    }
}

fn provider_run_request() -> ProviderRunRequest {
    ProviderRunRequest {
        provider_run_id: "run_error_001".to_string(),
        node_id: "N04".to_string(),
        runtime_role: RuntimeRole::Orchestrator,
        provider_capability_ref: "cap_fixture_001".to_string(),
        adapter_compatibility_ref: "compat_fixture_001".to_string(),
        context_package_ref: "ctx_001".to_string(),
        adapter_input_ref: "adapter_input_001".to_string(),
        adapter_output_ref: "adapter_output_001".to_string(),
        approval_policy: ApprovalPolicy::OnRequest,
        sandbox_mode: SandboxMode::WorkspaceWrite,
        constraint_check_ref: Some("chk_001".to_string()),
        traceability_binding_refs: vec!["bind_001".to_string()],
    }
}
