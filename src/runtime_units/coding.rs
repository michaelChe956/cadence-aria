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
use crate::cross_cutting::provider_run::{
    failed_provider_run_record_from_error, provider_run_record_from_output,
};
use crate::cross_cutting::runtime_event_log::append_node_event;
use crate::cross_cutting::traceability::{TraceabilityIndexes, normalize_traceability};
use crate::interactive::controller::PendingProviderStep;
use crate::protocol::artifacts::ArtifactKind;
use crate::protocol::contracts::{AdapterInput, ApprovalPolicy, ProviderRunRecord, SandboxMode};
use crate::protocol::loop_counters::{LoopCounterName, LoopCounterRegistry};
use crate::protocol::projections::PlanProjection;
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeProtocolStep, RuntimeStepStatus, RuntimeUnit,
    RuntimeUnitError, RuntimeUnitResult,
};
use crate::task_run::types::TaskRunError;
use serde_json::{Value, json};
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
    #[error("provider execution routed to manual hold for {0}")]
    ProviderBlocked(String),
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
    match state.run_report_node(provider, "N16", ArtifactKind::CodingReport, None) {
        Ok(_) => {}
        Err(ExecutionChainError::ProviderBlocked(_)) => return Ok(state.finish()),
        Err(error) => return Err(error),
    }

    loop {
        let testing_report =
            match state.run_report_node(provider, "N17", ArtifactKind::TestingReport, None) {
                Ok(report) => report,
                Err(ExecutionChainError::ProviderBlocked(_)) => return Ok(state.finish()),
                Err(error) => return Err(error),
            };
        if testing_report_requires_current_worktask_rework(&testing_report) {
            state.push_skill("systematic-debugging");
            let testing_report_ref = artifact_ref(&testing_report);
            if state.rework_or_hold(provider, &testing_report_ref)? {
                continue;
            }
            break;
        }

        let review_report =
            match state.run_report_node(provider, "N18", ArtifactKind::CodeReviewReport, None) {
                Ok(report) => report,
                Err(ExecutionChainError::ProviderBlocked(_)) => return Ok(state.finish()),
                Err(error) => return Err(error),
            };
        if review_report
            .get("blocking")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let code_review_report_ref = artifact_ref(&review_report);
            if state.rework_or_hold(provider, &code_review_report_ref)? {
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
        let threshold = LoopCounterRegistry::phase1().threshold(LoopCounterName::Rework);
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
        match self.run_report_node(
            provider,
            "N19",
            ArtifactKind::CodingReport,
            Some(source_ref),
        ) {
            Ok(_) => {}
            Err(ExecutionChainError::ProviderBlocked(_)) => return Ok(false),
            Err(error) => return Err(error),
        }
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
        let adapter_input = execution_adapter_input_for_node(&context)?;
        let request = provider_run_request(node_id, &context.context_package.context_package_id);
        let _pending_step = pending_provider_step_for_context(node_id, &adapter_input)
            .map_err(|error| ExecutionChainError::ProviderBlocked(error.message))?;
        append_node_event(
            &self.task_root(),
            &self.input.task_id,
            node_id,
            "node_enter",
            "started",
            json!({
                "provider_run_id": request.provider_run_id.clone(),
                "context_package_ref": context.context_package.context_package_id.clone(),
                "output_schema": adapter_input.output_schema.clone(),
            }),
        );
        let output = match provider.run(&adapter_input) {
            Ok(output) => output,
            Err(error) => {
                let record =
                    failed_provider_run_record_from_error(&request, &adapter_input, &error);
                self.provider_run_records.push(record.clone());
                append_node_event(
                    &self.task_root(),
                    &self.input.task_id,
                    node_id,
                    "node_exit",
                    "failed",
                    json!({
                        "provider_run_id": request.provider_run_id.clone(),
                        "error_code": error.code.as_str(),
                        "error_details": error.details.clone(),
                    }),
                );
                self.route_provider_error_to_manual_hold(
                    node_id,
                    error.code.as_str(),
                    &record.provider_run_id,
                );
                return Err(ExecutionChainError::ProviderBlocked(node_id.to_string()));
            }
        };
        let record = provider_run_record_from_output(&request, &adapter_input, &output);
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
        let record_ref = self
            .provider_run_records
            .last()
            .expect("provider run record just pushed");
        append_node_event(
            &self.task_root(),
            &self.input.task_id,
            node_id,
            "node_exit",
            "completed",
            json!({
                "provider_run_id": record_ref.provider_run_id,
                "duration_ms": record_ref.duration_ms,
                "retry_count": record_ref.retry_count,
            }),
        );
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
        let indexes =
            TraceabilityIndexes::new(known_traceability_refs(&self.input.plan_projection));
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
            )),
            &ProviderRunIndex::with_runs(vec![record.provider_run_id.clone()]),
        )
        .map_err(ExecutionChainError::ArtifactValidate)?;
        Ok(())
    }

    fn builder_input(&self, node_id: &str) -> ProviderContextBuilderInput {
        let verification_commands = self.verification_commands_for_node(node_id);
        let canonical_inputs = self.canonical_inputs_for_node(&verification_commands);
        let canonical_input_summary = prompt_json(&canonical_inputs);
        let projection_summary = prompt_json(&json!({
            "projection_refs": self.input.projection_refs,
            "plan_projection": self.input.plan_projection,
        }));
        ProviderContextBuilderInput {
            session_id: self.input.session_id.clone(),
            task_id: self.input.task_id.clone(),
            node_id: node_id.to_string(),
            canonical_inputs,
            canonical_input_summary,
            projection_refs: self.input.projection_refs.clone(),
            projection_summary,
            constraint_bundle_ref: self.input.constraint_bundle_ref.clone(),
            constraint_summary: "task constraints".to_string(),
            context_files: self.input.context_files.clone(),
            worktree_path: Some(self.input.worktree_path.to_string_lossy().to_string()),
        }
    }

    fn canonical_inputs_for_node(&self, verification_commands: &[String]) -> Value {
        json!({
            "artifact_refs": self
                .artifacts
                .iter()
                .filter_map(|artifact| artifact.get("artifact_ref").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            "prior_artifacts": self.artifacts,
            "risk_registry_ref": self.input.risk_registry_ref,
            "acceptance_targets": self.active_acceptance_targets(),
            "active_work_package": self.active_work_package(),
            "worktask_routing": self.worktask_route_for_prompt(verification_commands),
            "worktree_path": self.input.worktree_path,
        })
    }

    fn active_work_package(&self) -> Value {
        self.input
            .plan_projection
            .work_packages
            .iter()
            .find(|work_package| work_package.work_package_id == self.input.source_work_package_id)
            .and_then(|work_package| serde_json::to_value(work_package).ok())
            .unwrap_or_else(|| {
                json!({
                    "work_package_id": self.input.source_work_package_id,
                    "acceptance_targets": [],
                    "traceability_refs": [],
                })
            })
    }

    fn active_acceptance_targets(&self) -> Vec<String> {
        self.input
            .plan_projection
            .work_packages
            .iter()
            .find(|work_package| work_package.work_package_id == self.input.source_work_package_id)
            .map(|work_package| work_package.acceptance_targets.clone())
            .unwrap_or_default()
    }

    fn worktask_route_for_prompt(&self, verification_commands: &[String]) -> Value {
        let mut route = self.worktask_route().cloned().unwrap_or_else(|| json!({}));
        route["worktask_id"] = json!(self.input.worktask_id);
        route["source_work_package_id"] = json!(self.input.source_work_package_id);
        route["allowed_write_scope"] = json!(self.input.allowed_write_scope);
        route["verification_commands"] = json!(verification_commands);
        route
    }

    fn verification_commands_for_node(&self, node_id: &str) -> Vec<String> {
        let route_commands = self.route_verification_commands_for_worktask();
        if !route_commands.is_empty() {
            return route_commands;
        }
        if matches!(node_id, "N17" | "N18" | "N19") {
            let latest_commands = self.latest_coding_report_commands();
            if !latest_commands.is_empty() {
                return latest_commands;
            }
        }
        Vec::new()
    }

    fn route_verification_commands_for_worktask(&self) -> Vec<String> {
        self.worktask_route()
            .and_then(|route| route.get("verification_commands"))
            .and_then(Value::as_array)
            .map(|values| string_values(values))
            .unwrap_or_default()
    }

    fn latest_coding_report_commands(&self) -> Vec<String> {
        self.artifacts
            .iter()
            .rev()
            .find(|artifact| artifact["artifact_kind"] == "coding_report")
            .and_then(|artifact| artifact.get("commands_run"))
            .and_then(Value::as_array)
            .map(|commands| {
                commands
                    .iter()
                    .filter_map(|command| {
                        command.as_str().map(str::to_string).or_else(|| {
                            command
                                .get("command")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn worktask_route(&self) -> Option<&Value> {
        self.input
            .dispatch_package
            .pointer("/_aria/worktask_routing")
            .and_then(Value::as_array)
            .or_else(|| {
                self.input
                    .dispatch_package
                    .get("worktask_routing")
                    .and_then(Value::as_array)
            })
            .into_iter()
            .flatten()
            .find(|route| {
                route
                    .get("worktask_id")
                    .and_then(Value::as_str)
                    .is_some_and(|worktask_id| worktask_id == self.input.worktask_id)
            })
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

    fn route_provider_error_to_manual_hold(
        &mut self,
        node_id: &str,
        reason: &str,
        provider_run_id: &str,
    ) {
        self.next_node = "X08".to_string();
        self.manual_intervention_reason = Some(reason.to_string());
        self.protocol_steps.push(RuntimeProtocolStep {
            node_id: "X08".to_string(),
            status: RuntimeStepStatus::Blocked,
            node_specific_fields: json!({
                "reason": reason,
                "worktask_id": self.input.worktask_id,
                "trigger_node": node_id,
                "provider_run_id": provider_run_id,
            }),
        });
    }

    fn task_root(&self) -> PathBuf {
        self.input
            .worktree_path
            .join(".aria/runtime/tasks")
            .join(&self.input.task_id)
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

pub(crate) fn execution_adapter_input_for_node(
    context: &ProviderContextBuildResult,
) -> Result<AdapterInput, ExecutionChainError> {
    Ok(context.adapter_input.clone())
}

fn pending_provider_step_for_context(
    node_id: &str,
    adapter_input: &AdapterInput,
) -> Result<PendingProviderStep, TaskRunError> {
    crate::task_run::step_runner::provider_step_from_adapter_input(node_id, adapter_input)
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

fn testing_report_requires_current_worktask_rework(testing_report: &Value) -> bool {
    if testing_report
        .get("tests_passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    !testing_report_has_only_out_of_scope_acceptance_failures(testing_report)
}

fn testing_report_has_only_out_of_scope_acceptance_failures(testing_report: &Value) -> bool {
    let scope_result = testing_report
        .get("scope_result")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !scope_result.ends_with("_scoped_verification_passed") {
        return false;
    }

    let Some(failures) = testing_report.get("failures").and_then(Value::as_array) else {
        return false;
    };
    !failures.is_empty()
        && failures.iter().all(|failure| {
            failure
                .get("failure_type")
                .and_then(Value::as_str)
                .is_some_and(|failure_type| failure_type == "out_of_scope_acceptance_failure")
        })
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

fn string_values(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn prompt_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

fn known_traceability_refs(plan_projection: &PlanProjection) -> Vec<String> {
    plan_projection
        .work_packages
        .iter()
        .flat_map(|work_package| work_package.traceability_refs.iter())
        .map(|ref_id| ref_id.to_ascii_lowercase())
        .collect()
}
