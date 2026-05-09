use crate::cross_cutting::artifact_validate::{
    ArtifactContent, ConstraintBundleIndex, ProjectionIndex, ProviderRunIndex, TraceabilityIndex,
    canonical_validator, phase1_profile_validator,
};
use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_context_builder::{
    ProviderContextBuildError, ProviderContextBuildResult, ProviderContextBuilderInput,
    build_provider_context,
};
use crate::cross_cutting::provider_router::ProviderRunRequest;
use crate::cross_cutting::provider_run::provider_run_record_from_output;
use crate::protocol::artifacts::ArtifactKind;
use crate::protocol::contracts::{AdapterInput, ApprovalPolicy, ProviderRunRecord, SandboxMode};
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeProtocolStep, RuntimeStepStatus, RuntimeUnit,
    RuntimeUnitError, RuntimeUnitResult,
};
use serde_json::{Value, json};
use std::future::Future;

#[derive(Debug, Clone, PartialEq)]
pub struct FinalClosureInput {
    pub session_id: String,
    pub task_id: String,
    pub projection_refs: Vec<String>,
    pub constraint_bundle_ref: String,
    pub risk_registry_ref: String,
    pub canonical_artifact_refs: Vec<String>,
    pub traceability_refs: Vec<String>,
    pub context_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalClosureResult {
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub final_review: Value,
    pub final_summary: Value,
    pub provider_run_records: Vec<ProviderRunRecord>,
}

#[derive(Debug, thiserror::Error)]
pub enum FinalClosureError {
    #[error("provider context build failed: {0}")]
    ProviderContext(#[from] ProviderContextBuildError),
    #[error("provider adapter failed: {0}")]
    ProviderAdapter(#[from] ProviderAdapterError),
    #[error("provider structured output missing for {0}")]
    StructuredOutputMissing(String),
    #[error("artifact validation failed: {0:?}")]
    ArtifactValidate(crate::cross_cutting::artifact_validate::ArtifactValidateError),
    #[error("final_summary introduced coverage item not present in final_review: {0}")]
    FinalSummaryCoverageUnknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalReviewUnit;

impl RuntimeUnit for FinalReviewUnit {
    fn unit_id(&self) -> &'static str {
        "final_review"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N25"]
    }

    fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> impl Future<Output = Result<RuntimeUnitResult, RuntimeUnitError>> + Send {
        async {
            Err(RuntimeUnitError {
                code: "provider_adapter_required".to_string(),
                message: "N25 requires provider execution chain injection".to_string(),
            })
        }
    }
}

pub fn run_final_closure_chain(
    input: FinalClosureInput,
    provider: &dyn ProviderAdapter,
) -> Result<FinalClosureResult, FinalClosureError> {
    let mut provider_run_records = Vec::new();
    let mut final_review = run_final_provider_node(
        &input,
        provider,
        "N25",
        ArtifactKind::FinalReview,
        &mut provider_run_records,
    )?;
    normalize_final_review_profile(&input, &mut final_review, &provider_run_records)?;
    let mut final_summary = run_final_provider_node(
        &input,
        provider,
        "N27",
        ArtifactKind::FinalSummary,
        &mut provider_run_records,
    )?;
    normalize_final_summary_profile(
        &input,
        &final_review,
        &mut final_summary,
        &provider_run_records,
    )?;

    Ok(FinalClosureResult {
        protocol_steps: vec![
            RuntimeProtocolStep {
                node_id: "N25".to_string(),
                status: RuntimeStepStatus::Completed,
                node_specific_fields: json!({
                    "overall_decision": final_review["overall_decision"],
                    "coverage_summary": final_review["_aria"]["coverage_summary"],
                    "uncovered_items": final_review["uncovered_items"],
                    "manual_exemptions": [],
                }),
            },
            RuntimeProtocolStep {
                node_id: "N27".to_string(),
                status: RuntimeStepStatus::Completed,
                node_specific_fields: json!({
                    "overall_status": final_summary["overall_status"],
                    "closed_items": final_summary.get("closed_items").cloned().unwrap_or_else(|| json!([])),
                    "remaining_risks": final_summary.get("remaining_risks").cloned().unwrap_or_else(|| json!([])),
                }),
            },
            session_closeout_step(&input.task_id, json!({})),
        ],
        final_review,
        final_summary,
        provider_run_records,
    })
}

pub(crate) fn run_final_provider_node(
    input: &FinalClosureInput,
    provider: &dyn ProviderAdapter,
    node_id: &str,
    artifact_kind: ArtifactKind,
    provider_run_records: &mut Vec<ProviderRunRecord>,
) -> Result<Value, FinalClosureError> {
    let context = build_provider_context(builder_input(input, node_id))?;
    let adapter_input = final_adapter_input_for_node(&context)?;
    let output = provider.run(&adapter_input)?;
    let request = provider_run_request(node_id, &context.context_package.context_package_id);
    let record = provider_run_record_from_output(&request, &adapter_input, &output);
    let artifact = output
        .structured_output
        .ok_or_else(|| FinalClosureError::StructuredOutputMissing(node_id.to_string()))?;
    canonical_validator(artifact_kind, &ArtifactContent::Json(artifact.clone()))
        .map_err(FinalClosureError::ArtifactValidate)?;
    provider_run_records.push(record);
    Ok(artifact)
}

pub(crate) fn final_adapter_input_for_node(
    context: &ProviderContextBuildResult,
) -> Result<AdapterInput, FinalClosureError> {
    Ok(context.adapter_input.clone())
}

pub(crate) fn normalize_final_review_profile(
    input: &FinalClosureInput,
    final_review: &mut Value,
    records: &[ProviderRunRecord],
) -> Result<(), FinalClosureError> {
    final_review["_aria"] = json!({
        "profile_version": "phase1.v1",
        "constraint_check_ref": input.constraint_bundle_ref,
        "traceability_refs": input.traceability_refs,
        "provider_run_refs": records.iter().map(|record| record.provider_run_id.clone()).collect::<Vec<_>>(),
        "projection_refs": input.projection_refs,
        "coverage_summary": final_review["coverage_summary"],
    });
    phase1_profile_validator(
        final_review,
        ArtifactKind::FinalReview,
        &ProjectionIndex::with_work_packages(input.projection_refs.clone(), Vec::new()),
        &ConstraintBundleIndex {
            constraint_bundle_ids: vec![input.constraint_bundle_ref.clone()],
            constraint_check_ids: Vec::new(),
        },
        &TraceabilityIndex::with_known_refs(input.traceability_refs.clone()),
        &ProviderRunIndex::with_runs(
            records
                .iter()
                .map(|record| record.provider_run_id.clone())
                .collect(),
        ),
    )
    .map_err(FinalClosureError::ArtifactValidate)?;
    Ok(())
}

pub(crate) fn normalize_final_summary_profile(
    input: &FinalClosureInput,
    final_review: &Value,
    final_summary: &mut Value,
    records: &[ProviderRunRecord],
) -> Result<(), FinalClosureError> {
    validate_final_summary_coverage(final_review, final_summary)?;
    let provider_run_refs = records
        .last()
        .map(|record| vec![record.provider_run_id.clone()])
        .unwrap_or_default();
    final_summary["_aria"] = json!({
        "profile_version": "phase1.v1",
        "constraint_check_ref": input.constraint_bundle_ref,
        "traceability_refs": input.traceability_refs,
        "provider_run_refs": provider_run_refs,
        "projection_refs": input.projection_refs,
    });
    phase1_profile_validator(
        final_summary,
        ArtifactKind::FinalSummary,
        &ProjectionIndex::with_work_packages(input.projection_refs.clone(), Vec::new()),
        &ConstraintBundleIndex {
            constraint_bundle_ids: vec![input.constraint_bundle_ref.clone()],
            constraint_check_ids: Vec::new(),
        },
        &TraceabilityIndex::with_known_refs(input.traceability_refs.clone()),
        &ProviderRunIndex::with_runs(
            records
                .iter()
                .map(|record| record.provider_run_id.clone())
                .collect(),
        ),
    )
    .map_err(FinalClosureError::ArtifactValidate)?;
    Ok(())
}

fn validate_final_summary_coverage(
    final_review: &Value,
    final_summary: &Value,
) -> Result<(), FinalClosureError> {
    let review_closed = string_array_at(final_review, &["_aria", "coverage_summary", "closed"]);
    for closed_item in string_array_at(final_summary, &["closed_items"]) {
        if !review_closed.contains(&closed_item) {
            return Err(FinalClosureError::FinalSummaryCoverageUnknown(closed_item));
        }
    }
    Ok(())
}

pub(crate) fn session_closeout_step(task_id: &str, extra: Value) -> RuntimeProtocolStep {
    let mut fields = json!({
        "session_closeout_timestamp": chrono::Utc::now().to_rfc3339(),
        "final_checkpoint_ref": format!("checkpoint_{task_id}_final"),
    });
    merge_object(&mut fields, extra);
    RuntimeProtocolStep {
        node_id: "N28".to_string(),
        status: RuntimeStepStatus::Completed,
        node_specific_fields: fields,
    }
}

fn builder_input(input: &FinalClosureInput, node_id: &str) -> ProviderContextBuilderInput {
    ProviderContextBuilderInput {
        session_id: input.session_id.clone(),
        task_id: input.task_id.clone(),
        node_id: node_id.to_string(),
        canonical_inputs: json!({
            "artifact_refs": input.canonical_artifact_refs,
            "risk_registry_ref": input.risk_registry_ref,
        }),
        canonical_input_summary: "final closure canonical inputs".to_string(),
        projection_refs: input.projection_refs.clone(),
        projection_summary: "phase1 projections".to_string(),
        constraint_bundle_ref: input.constraint_bundle_ref.clone(),
        constraint_summary: "final closure constraints".to_string(),
        context_files: input.context_files.clone(),
        worktree_path: None,
    }
}

fn provider_run_request(node_id: &str, context_package_ref: &str) -> ProviderRunRequest {
    let contract =
        crate::protocol::contracts::execution_contract_for_node(node_id).expect("contract");
    ProviderRunRequest {
        provider_run_id: format!("run_{}_0001", node_id.to_ascii_lowercase()),
        node_id: node_id.to_string(),
        runtime_role: contract.runtime_role,
        provider_capability_ref: format!("capability_{node_id}"),
        adapter_compatibility_ref: format!("compat_{node_id}"),
        context_package_ref: context_package_ref.to_string(),
        adapter_input_ref: format!("adapter_input_{node_id}"),
        adapter_output_ref: format!("adapter_output_{node_id}"),
        approval_policy: ApprovalPolicy::OnRequest,
        sandbox_mode: SandboxMode::WorkspaceWrite,
        constraint_check_ref: Some("constraint_check_final".to_string()),
        traceability_binding_refs: Vec::new(),
    }
}

fn merge_object(target: &mut Value, extra: Value) {
    let Some(target) = target.as_object_mut() else {
        return;
    };
    let Some(extra) = extra.as_object() else {
        return;
    };
    for (key, value) in extra {
        target.insert(key.clone(), value.clone());
    }
}

fn string_array_at(value: &Value, path: &[&str]) -> Vec<String> {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return Vec::new();
        };
        current = next;
    }
    current
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
