use super::artifacts::persist_provider_run;
use super::snapshot::{restore_directory_snapshot, snapshot_directory};
use super::utils::provider_run_id;
use super::{PlanningChainState, PlanningUnitError};
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_context_builder::{
    ProviderContextBuildResult, ProviderContextBuilderInput, build_provider_context,
};
use crate::cross_cutting::provider_router::ProviderRunRequest;
use crate::cross_cutting::provider_run::{
    failed_provider_run_record_from_error, provider_run_record_from_output,
};
use crate::cross_cutting::runtime_event_log::append_node_event;
use crate::protocol::contracts::{AdapterInput, AdapterOutput, ApprovalPolicy, SandboxMode};
use crate::protocol::provider_errors::{ProviderErrorRoute, route_provider_error};
use serde_json::{Value, json};

pub fn run_provider_node(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
    node_id: &str,
    mut canonical_inputs: Value,
    canonical_input_summary: impl Into<String>,
    projection_refs: Vec<String>,
    constraint_summary: impl Into<String>,
    context_files: Vec<String>,
) -> Result<AdapterOutput, PlanningUnitError> {
    attach_risk_registry_ref(&mut canonical_inputs, &state.input.task_id);
    let build_result = build_provider_context(ProviderContextBuilderInput {
        session_id: state.input.session_id.clone(),
        task_id: state.input.task_id.clone(),
        node_id: node_id.to_string(),
        canonical_inputs,
        canonical_input_summary: canonical_input_summary.into(),
        projection_refs,
        projection_summary: "planning chain projection summary".to_string(),
        constraint_bundle_ref: state.current_bundle.constraint_bundle_id.clone(),
        constraint_summary: constraint_summary.into(),
        context_files,
        worktree_path: state.input.worktree_path.clone(),
    })?;
    let adapter_input = planning_adapter_input_for_node(&build_result)?;
    let request = ProviderRunRequest {
        provider_run_id: provider_run_id(&state.input.task_id, node_id),
        node_id: node_id.to_string(),
        runtime_role: build_result.context_package.runtime_role.clone(),
        provider_capability_ref: "cap_fake_planning_provider_v1".to_string(),
        adapter_compatibility_ref: "compat_fake_planning_provider_v1".to_string(),
        context_package_ref: build_result.context_package.context_package_id.clone(),
        adapter_input_ref: format!("adapter_input_{}_{}", state.input.task_id, node_id),
        adapter_output_ref: format!("adapter_output_{}_{}", state.input.task_id, node_id),
        approval_policy: ApprovalPolicy::OnRequest,
        sandbox_mode: SandboxMode::WorkspaceWrite,
        constraint_check_ref: Some(state.current_bundle.constraint_bundle_id.clone()),
        traceability_binding_refs: Vec::new(),
    };
    append_node_event(
        &state.task_root(),
        &state.input.task_id,
        node_id,
        "node_enter",
        "started",
        json!({
            "provider_run_id": request.provider_run_id.clone(),
            "context_package_ref": build_result.context_package.context_package_id.clone(),
            "output_schema": adapter_input.output_schema.clone(),
        }),
    );
    let protected_openspec_snapshot = snapshot_directory(&state.openspec_change_dir())?;
    let mut retry_count = 0;
    loop {
        match provider.run(&adapter_input) {
            Ok(output) => {
                restore_directory_snapshot(
                    &state.openspec_change_dir(),
                    &protected_openspec_snapshot,
                )?;
                let mut record = provider_run_record_from_output(&request, &adapter_input, &output);
                record.retry_count = retry_count;
                persist_provider_run(state, &record)?;
                append_node_event(
                    &state.task_root(),
                    &state.input.task_id,
                    node_id,
                    "node_exit",
                    "completed",
                    json!({
                        "provider_run_id": record.provider_run_id,
                        "duration_ms": record.duration_ms,
                        "retry_count": retry_count,
                    }),
                );
                return Ok(output);
            }
            Err(error) => {
                restore_directory_snapshot(
                    &state.openspec_change_dir(),
                    &protected_openspec_snapshot,
                )?;
                let route =
                    route_provider_error(&error.code, retry_count, adapter_input.max_retries);
                if route == ProviderErrorRoute::Retry {
                    retry_count += 1;
                    continue;
                }
                let mut record =
                    failed_provider_run_record_from_error(&request, &adapter_input, &error);
                record.retry_count = retry_count;
                persist_provider_run(state, &record)?;
                append_node_event(
                    &state.task_root(),
                    &state.input.task_id,
                    node_id,
                    "node_exit",
                    "failed",
                    json!({
                        "provider_run_id": record.provider_run_id,
                        "error_code": record.error_code,
                        "error_details": record.error_details,
                        "retry_count": retry_count,
                    }),
                );
                return Err(PlanningUnitError::ProviderAdapter(error));
            }
        }
    }
}

pub(crate) fn planning_adapter_input_for_node(
    build_result: &ProviderContextBuildResult,
) -> Result<AdapterInput, PlanningUnitError> {
    Ok(build_result.adapter_input.clone())
}

fn attach_risk_registry_ref(value: &mut Value, task_id: &str) {
    if !value.is_object() {
        *value = json!({ "payload": value.clone() });
    }
    if value.get("risk_registry_ref").is_none() {
        value["risk_registry_ref"] = json!(format!("riskreg_{task_id}_v0001"));
    }
}
