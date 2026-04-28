use crate::cross_cutting::artifact_validate::{
    canonical_validator, phase1_profile_validator, ArtifactContent, ConstraintBundleIndex,
    ProjectionIndex, ProviderRunIndex, TraceabilityIndex,
};
use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_context_builder::{
    build_provider_context, ProviderContextBuildError, ProviderContextBuilderInput,
};
use crate::cross_cutting::provider_router::ProviderRunRequest;
use crate::cross_cutting::provider_run::provider_run_record_from_output;
use crate::cross_cutting::traceability::{normalize_traceability, TraceabilityIndexes};
use crate::protocol::artifacts::ArtifactKind;
use crate::protocol::contracts::{ApprovalPolicy, ProviderRunRecord, SandboxMode};
use crate::protocol::projections::PlanProjection;
use crate::protocol::loop_counters::{LoopCounterName, LoopCounterRegistry};
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeProtocolStep, RuntimeStepStatus, RuntimeUnit,
    RuntimeUnitError, RuntimeUnitResult,
};
use serde_json::{json, Value};
use std::future::Future;
use std::path::PathBuf;

pub const EXECUTION_CHAIN: &[&str] = &[
    "canonical_node_input",
    "projection_or_bundle",
    "provider_context_package",
    "adapter_input",
    "provider_call",
    "provider_run_record",
    "normalize_output",
    "artifact_validate",
    "phase1_profile_validate",
    "openspec_coverage",
    "checkpoint",
];

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionWorktaskInput {
    pub session_id: String,
    pub task_id: String,
    pub worktask_id: String,
    pub source_work_package_id: String,
    pub worktree_path: PathBuf,
    pub allowed_write_scope: Vec<String>,
    pub dispatch_package: Value,
    pub plan_projection: PlanProjection,
    pub projection_refs: Vec<String>,
    pub constraint_bundle_ref: String,
    pub risk_registry_ref: String,
    pub context_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionNodeTrace {
    pub node_id: String,
    pub execution_chain: Vec<String>,
    pub produced_artifact_kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionWorktaskResult {
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub artifacts: Vec<Value>,
    pub provider_run_records: Vec<ProviderRunRecord>,
    pub node_traces: Vec<ExecutionNodeTrace>,
    pub workflow_skills_activated: Vec<String>,
    pub superseded_report_refs: Vec<String>,
    pub rework_counter: u32,
    pub next_node: String,
    pub manual_intervention_reason: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionChainError {
    #[error("provider context build failed: {0}")]
    ProviderContext(#[from] ProviderContextBuildError),
    #[error("provider adapter failed: {0}")]
    ProviderAdapter(#[from] ProviderAdapterError),
    #[error("provider structured output missing for {0}")]
    StructuredOutputMissing(String),
    #[error("artifact validation failed: {0:?}")]
    ArtifactValidate(crate::cross_cutting::artifact_validate::ArtifactValidateError),
    #[error("traceability normalize failed: {0:?}")]
    Traceability(crate::cross_cutting::traceability::TraceabilityError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodingUnit;

impl RuntimeUnit for CodingUnit {
    fn unit_id(&self) -> &'static str {
        "coding"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N16"]
    }

    fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> impl Future<Output = Result<RuntimeUnitResult, RuntimeUnitError>> + Send {
        async {
            Err(RuntimeUnitError {
                code: "provider_adapter_required".to_string(),
                message: "N16 requires provider execution chain injection".to_string(),
            })
        }
    }
}

pub fn run_worktask_execution_chain(
    input: ExecutionWorktaskInput,
    provider: &dyn ProviderAdapter,
) -> Result<ExecutionWorktaskResult, ExecutionChainError> {
    let mut state = ExecutionChainState::new(input);
    state.run_report_node(provider, "N16", ArtifactKind::CodingReport, None)?;

    loop {
        let testing_report =
            state.run_report_node(provider, "N17", ArtifactKind::TestingReport, None)?;
        if !testing_report
            .get("tests_passed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            state.push_skill("systematic-debugging");
            if state.rework_or_hold(provider, "testing_report_worktask_001_0001")? {
                continue;
            }
            break;
        }

        let review_report =
            state.run_report_node(provider, "N18", ArtifactKind::CodeReviewReport, None)?;
        if review_report
            .get("blocking")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if state.rework_or_hold(provider, "code_review_report_worktask_001_0001")? {
                continue;
            }
            break;
        }

        state.next_node = "M20".to_string();
        break;
    }

    Ok(state.finish())
}

struct ExecutionChainState {
    input: ExecutionWorktaskInput,
    protocol_steps: Vec<RuntimeProtocolStep>,
    artifacts: Vec<Value>,
    provider_run_records: Vec<ProviderRunRecord>,
    node_traces: Vec<ExecutionNodeTrace>,
    workflow_skills_activated: Vec<String>,
    superseded_report_refs: Vec<String>,
    rework_counter: u32,
    next_node: String,
    manual_intervention_reason: Option<String>,
}

impl ExecutionChainState {
    fn new(input: ExecutionWorktaskInput) -> Self {
        Self {
            input,
            protocol_steps: Vec::new(),
            artifacts: Vec::new(),
            provider_run_records: Vec::new(),
            node_traces: Vec::new(),
            workflow_skills_activated: Vec::new(),
            superseded_report_refs: Vec::new(),
            rework_counter: 0,
            next_node: String::new(),
            manual_intervention_reason: None,
        }
    }

    fn rework_or_hold(
        &mut self,
        provider: &dyn ProviderAdapter,
        source_ref: &str,
    ) -> Result<bool, ExecutionChainError> {
        self.rework_counter += 1;
        let threshold = LoopCounterRegistry::phase1()
            .threshold(LoopCounterName::Rework);
        if self.rework_counter > threshold {
            self.next_node = "X08".to_string();
            self.manual_intervention_reason = Some("rework_limit_exceeded".to_string());
            self.protocol_steps.push(RuntimeProtocolStep {
                node_id: "X08".to_string(),
                status: RuntimeStepStatus::Blocked,
                node_specific_fields: json!({
                    "reason": "rework_limit_exceeded",
                    "rework_counter": self.rework_counter,
                    "worktask_id": self.input.worktask_id,
                    "trigger_node": "N19",
                }),
            });
            return Ok(false);
        }
        self.push_skill("receiving-code-review");
        let superseded = self.latest_coding_report_ref();
        if let Some(superseded) = superseded {
            self.superseded_report_refs.push(superseded);
        }
        self.run_report_node(
            provider,
            "N19",
            ArtifactKind::CodingReport,
            Some(source_ref),
        )?;
        Ok(true)
    }

    fn run_report_node(
        &mut self,
        provider: &dyn ProviderAdapter,
        node_id: &str,
        artifact_kind: ArtifactKind,
        rework_source_ref: Option<&str>,
    ) -> Result<Value, ExecutionChainError> {
        let context = build_provider_context(self.builder_input(node_id))?;
        let output = provider.run(&context.adapter_input)?;
        let request = provider_run_request(node_id, &context.context_package.context_package_id);
        let record = provider_run_record_from_output(&request, &context.adapter_input, &output);
        let mut artifact = output
            .structured_output
            .clone()
            .ok_or_else(|| ExecutionChainError::StructuredOutputMissing(node_id.to_string()))?;
        ensure_artifact_ref(&mut artifact, node_id, &self.input.worktask_id);
        canonical_validator(artifact_kind, &ArtifactContent::Json(artifact.clone()))
            .map_err(ExecutionChainError::ArtifactValidate)?;
        self.normalize_and_validate_profile(&mut artifact, artifact_kind, &record)?;

        self.protocol_steps.push(RuntimeProtocolStep {
            node_id: node_id.to_string(),
            status: RuntimeStepStatus::Completed,
            node_specific_fields: node_specific_fields(
                node_id,
                &artifact,
                rework_source_ref,
                &self.superseded_report_refs,
            ),
        });
        self.node_traces.push(ExecutionNodeTrace {
            node_id: node_id.to_string(),
            execution_chain: EXECUTION_CHAIN
                .iter()
                .map(|step| step.to_string())
                .collect(),
            produced_artifact_kind: artifact_kind.as_str().to_string(),
        });
        self.provider_run_records.push(record);
        self.artifacts.push(artifact.clone());
        Ok(artifact)
    }

    fn normalize_and_validate_profile(
        &self,
        artifact: &mut Value,
        artifact_kind: ArtifactKind,
        record: &ProviderRunRecord,
    ) -> Result<(), ExecutionChainError> {
        let candidate_refs = string_array_field(artifact, "candidate_traceability_refs");
        let indexes = TraceabilityIndexes::new(known_traceability_refs(
            &self.input.plan_projection,
            &candidate_refs,
        ));
        normalize_traceability(
            artifact,
            candidate_refs,
            &self.input.dispatch_package,
            &self.input.plan_projection,
            &indexes,
        )
        .map_err(ExecutionChainError::Traceability)?;
        artifact["_aria"]["profile_version"] = json!("phase1.v1");
        artifact["_aria"]["constraint_check_ref"] = json!(self.input.constraint_bundle_ref);
        artifact["_aria"]["provider_run_refs"] = json!([record.provider_run_id.clone()]);
        artifact["_aria"]["projection_refs"] = json!(self.input.projection_refs);

        let work_package_ids = self
            .input
            .plan_projection
            .work_packages
            .iter()
            .map(|work_package| work_package.work_package_id.clone())
            .collect::<Vec<_>>();
        phase1_profile_validator(
            artifact,
            artifact_kind,
            &ProjectionIndex::with_work_packages(
                self.input.projection_refs.clone(),
                work_package_ids,
            ),
            &ConstraintBundleIndex {
                constraint_bundle_ids: vec![self.input.constraint_bundle_ref.clone()],
                constraint_check_ids: Vec::new(),
            },
            &TraceabilityIndex::with_known_refs(known_traceability_refs(
                &self.input.plan_projection,
                &Vec::new(),
            )),
            &ProviderRunIndex::with_runs(vec![record.provider_run_id.clone()]),
        )
        .map_err(ExecutionChainError::ArtifactValidate)?;
        Ok(())
    }

    fn builder_input(&self, node_id: &str) -> ProviderContextBuilderInput {
        ProviderContextBuilderInput {
            session_id: self.input.session_id.clone(),
            task_id: self.input.task_id.clone(),
            node_id: node_id.to_string(),
            canonical_inputs: json!({
                "artifact_refs": self
                    .artifacts
                    .iter()
                    .filter_map(|artifact| artifact.get("artifact_ref").and_then(Value::as_str))
                    .collect::<Vec<_>>(),
                "risk_registry_ref": self.input.risk_registry_ref,
                "acceptance_targets": self
                    .input
                    .plan_projection
                    .work_packages
                    .iter()
                    .find(|work_package| work_package.work_package_id == self.input.source_work_package_id)
                    .map(|work_package| work_package.acceptance_targets.clone())
                    .unwrap_or_default(),
                "worktask_routing": {
                    "worktask_id": self.input.worktask_id,
                    "source_work_package_id": self.input.source_work_package_id,
                    "allowed_write_scope": self.input.allowed_write_scope,
                }
            }),
            canonical_input_summary: format!("worktask {}", self.input.worktask_id),
            projection_refs: self.input.projection_refs.clone(),
            projection_summary: "spec/design/plan projection summary".to_string(),
            constraint_bundle_ref: self.input.constraint_bundle_ref.clone(),
            constraint_summary: "task constraints".to_string(),
            context_files: self.input.context_files.clone(),
            worktree_path: Some(self.input.worktree_path.to_string_lossy().to_string()),
        }
    }

    fn latest_coding_report_ref(&self) -> Option<String> {
        self.artifacts
            .iter()
            .rev()
            .find(|artifact| artifact["artifact_kind"] == "coding_report")
            .and_then(|artifact| artifact.get("artifact_ref"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    fn push_skill(&mut self, skill: &str) {
        let skill = skill.to_string();
        if !self.workflow_skills_activated.contains(&skill) {
            self.workflow_skills_activated.push(skill);
        }
    }

    fn finish(self) -> ExecutionWorktaskResult {
        ExecutionWorktaskResult {
            protocol_steps: self.protocol_steps,
            artifacts: self.artifacts,
            provider_run_records: self.provider_run_records,
            node_traces: self.node_traces,
            workflow_skills_activated: self.workflow_skills_activated,
            superseded_report_refs: self.superseded_report_refs,
            rework_counter: self.rework_counter,
            next_node: self.next_node,
            manual_intervention_reason: self.manual_intervention_reason,
        }
    }
}

fn provider_run_request(node_id: &str, context_package_ref: &str) -> ProviderRunRequest {
    let contract = crate::protocol::contracts::execution_contract_for_node(node_id)
        .expect("execution contract");
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
        constraint_check_ref: Some("constraint_check_task_constraints".to_string()),
        traceability_binding_refs: Vec::new(),
    }
}

fn node_specific_fields(
    node_id: &str,
    artifact: &Value,
    rework_source_ref: Option<&str>,
    superseded_report_refs: &[String],
) -> Value {
    match node_id {
        "N16" => json!({
            "coding_report_ref": artifact_ref(artifact),
            "changed_files": artifact.get("files_modified").cloned().unwrap_or_else(|| json!([])),
        }),
        "N17" => json!({
            "testing_report_ref": artifact_ref(artifact),
            "test_results": artifact.get("failures").cloned().unwrap_or_else(|| json!([])),
            "coverage_summary": {
                "closed": artifact.pointer("/_aria/traceability_refs").cloned().unwrap_or_else(|| json!([])),
                "uncovered": [],
                "exempted": [],
            },
        }),
        "N18" => json!({
            "code_review_report_ref": artifact_ref(artifact),
            "findings": artifact.get("findings").cloned().unwrap_or_else(|| json!([])),
        }),
        "N19" => json!({
            "rework_scope": {
                "source": rework_source_ref.unwrap_or("unknown"),
            },
            "superseded_report_refs": superseded_report_refs,
        }),
        _ => json!({}),
    }
}

fn artifact_ref(artifact: &Value) -> String {
    artifact
        .get("artifact_ref")
        .and_then(Value::as_str)
        .unwrap_or("artifact_ref_unknown")
        .to_string()
}

fn ensure_artifact_ref(artifact: &mut Value, node_id: &str, worktask_id: &str) {
    if artifact
        .get("artifact_ref")
        .and_then(Value::as_str)
        .is_none()
    {
        artifact["artifact_ref"] = json!(format!(
            "{}_{}_0001",
            artifact_kind_for_node(node_id).replace('_', "-"),
            worktask_id
        ));
    }
}

fn artifact_kind_for_node(node_id: &str) -> &'static str {
    match node_id {
        "N16" | "N19" => "coding_report",
        "N17" => "testing_report",
        "N18" => "code_review_report",
        _ => "artifact",
    }
}

fn string_array_field(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn known_traceability_refs(
    plan_projection: &PlanProjection,
    candidate_refs: &[String],
) -> Vec<String> {
    plan_projection
        .work_packages
        .iter()
        .flat_map(|work_package| work_package.traceability_refs.iter())
        .chain(candidate_refs.iter())
        .map(|ref_id| ref_id.to_ascii_lowercase())
        .collect()
}
