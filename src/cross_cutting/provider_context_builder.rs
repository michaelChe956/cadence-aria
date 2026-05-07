use crate::protocol::contracts::{
    AdapterInput, ProviderContextPackage, execution_contract_for_node, workflow_discipline_for_node,
};
use crate::protocol::enums::{ConstraintBundleId, ProjectionId, SessionId, TaskId};
use crate::protocol::prompt_manifest::{PromptRenderError, render_prompt_template};
use crate::runtime_units::prompt_template_registry::prompt_template_for_node;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderContextBuilderInput {
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub node_id: String,
    pub canonical_inputs: Value,
    pub canonical_input_summary: String,
    pub projection_refs: Vec<ProjectionId>,
    pub projection_summary: String,
    pub constraint_bundle_ref: ConstraintBundleId,
    pub constraint_summary: String,
    pub context_files: Vec<String>,
    pub worktree_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderContextBuildResult {
    pub context_package: ProviderContextPackage,
    pub adapter_input: AdapterInput,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderContextBuildError {
    #[error("node contract not found: {0}")]
    ContractNotFound(String),
    #[error("workflow discipline not found: {0}")]
    WorkflowNotFound(String),
    #[error("prompt template not found: {0}")]
    TemplateNotFound(String),
    #[error(transparent)]
    PromptRender(#[from] PromptRenderError),
    #[error("context serialization failed: {0}")]
    Serialization(String),
    #[error("risk_registry_ref is required in canonical_inputs")]
    MissingRiskRegistryRef,
    #[error("projection_refs are required for node {0}")]
    MissingProjectionRefs(String),
    #[error("constraint_bundle_ref is required for node {0}")]
    MissingConstraintBundleRef(String),
    #[error("worktree_path is required for node {0}")]
    MissingWorktreePath(String),
    #[error("acceptance_targets are required for node {0}")]
    MissingAcceptanceTargets(String),
}

pub fn build_provider_context(
    input: ProviderContextBuilderInput,
) -> Result<ProviderContextBuildResult, ProviderContextBuildError> {
    validate_risk_registry_ref(&input.canonical_inputs)?;
    let contract = execution_contract_for_node(&input.node_id)
        .ok_or_else(|| ProviderContextBuildError::ContractNotFound(input.node_id.clone()))?;
    let workflow = workflow_discipline_for_node(&input.node_id)
        .ok_or_else(|| ProviderContextBuildError::WorkflowNotFound(input.node_id.clone()))?;
    let template = prompt_template_for_node(&input.node_id)
        .ok_or_else(|| ProviderContextBuildError::TemplateNotFound(input.node_id.clone()))?;
    validate_required_context(&input, &contract)?;
    let allowed_write_scope = resolve_allowed_write_scope(&contract, &input.canonical_inputs);
    let verification_commands = resolve_verification_commands(&contract, &input.canonical_inputs);
    let variables = prompt_variables(
        &input,
        &contract,
        &workflow,
        &allowed_write_scope,
        &verification_commands,
    )?;
    let prompt = render_prompt_template(&template, &variables)?;

    let context_package = ProviderContextPackage {
        context_package_id: format!("ctx_{}_{}", input.task_id, input.node_id.to_lowercase()),
        session_id: input.session_id,
        task_id: input.task_id,
        node_id: input.node_id,
        provider_type: contract.provider_type.clone(),
        runtime_role: contract.runtime_role.clone(),
        adapter_role: contract.adapter_role.clone(),
        advisory_only: contract.advisory_only,
        canonical_inputs: input.canonical_inputs,
        projection_refs: input.projection_refs,
        constraint_bundle_ref: input.constraint_bundle_ref,
        node_execution_contract: contract.clone(),
        workflow_discipline: workflow,
        prompt_template: template.template_ref,
        worktree_path: input.worktree_path.clone(),
        allowed_write_scope: allowed_write_scope.clone(),
        context_files: input.context_files.clone(),
        instructions: vec!["provider output is candidate-only".to_string()],
        output_schema_ref: contract.output_schema_ref.clone(),
        completion_criteria: contract.completion_criteria.clone(),
        forbidden_actions: contract.forbidden_actions.clone(),
        verification_commands: verification_commands.clone(),
        timeout_sec: contract.timeout_sec,
        max_retries: contract.max_retries,
    };

    let adapter_input = AdapterInput {
        provider_type: context_package.provider_type.clone(),
        role: context_package.adapter_role.clone(),
        worktree_path: context_package.worktree_path.clone(),
        prompt,
        context_files: context_package.context_files.clone(),
        output_schema: context_package.output_schema_ref.clone(),
        timeout: context_package.timeout_sec,
        max_retries: context_package.max_retries,
    };

    Ok(ProviderContextBuildResult {
        context_package,
        adapter_input,
    })
}

fn validate_risk_registry_ref(value: &Value) -> Result<(), ProviderContextBuildError> {
    let risk_registry_ref = value
        .get("risk_registry_ref")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if risk_registry_ref.is_empty() {
        Err(ProviderContextBuildError::MissingRiskRegistryRef)
    } else {
        Ok(())
    }
}

fn validate_required_context(
    input: &ProviderContextBuilderInput,
    contract: &crate::protocol::contracts::NodeExecutionContract,
) -> Result<(), ProviderContextBuildError> {
    if !contract.required_projection_kinds.is_empty() && input.projection_refs.is_empty() {
        return Err(ProviderContextBuildError::MissingProjectionRefs(
            input.node_id.clone(),
        ));
    }
    if !contract.required_constraint_kinds.is_empty()
        && input.constraint_bundle_ref.trim().is_empty()
    {
        return Err(ProviderContextBuildError::MissingConstraintBundleRef(
            input.node_id.clone(),
        ));
    }
    if requires_worktree(&input.node_id) && input.worktree_path.as_deref().unwrap_or("").is_empty()
    {
        return Err(ProviderContextBuildError::MissingWorktreePath(
            input.node_id.clone(),
        ));
    }
    if requires_acceptance_targets(&input.node_id)
        && !has_non_empty_array(&input.canonical_inputs, "acceptance_targets")
    {
        return Err(ProviderContextBuildError::MissingAcceptanceTargets(
            input.node_id.clone(),
        ));
    }
    Ok(())
}

fn resolve_allowed_write_scope(
    contract: &crate::protocol::contracts::NodeExecutionContract,
    canonical_inputs: &Value,
) -> Vec<String> {
    if contract
        .allowed_write_scope
        .iter()
        .any(|scope| scope == "<worktask_routing.allowed_write_scope>")
    {
        return string_array_at(
            canonical_inputs,
            &["worktask_routing", "allowed_write_scope"],
        )
        .or_else(|| string_array_at(canonical_inputs, &["allowed_write_scope"]))
        .unwrap_or_default();
    }
    contract.allowed_write_scope.clone()
}

fn resolve_verification_commands(
    contract: &crate::protocol::contracts::NodeExecutionContract,
    canonical_inputs: &Value,
) -> Vec<String> {
    string_array_at(
        canonical_inputs,
        &["worktask_routing", "verification_commands"],
    )
    .or_else(|| string_array_at(canonical_inputs, &["verification_commands"]))
    .unwrap_or_else(|| contract.verification_commands.clone())
}

fn requires_worktree(node_id: &str) -> bool {
    matches!(node_id, "N16" | "N17" | "N18" | "N19" | "N20" | "N24")
}

fn requires_acceptance_targets(node_id: &str) -> bool {
    matches!(node_id, "N16" | "N17" | "N18" | "N19")
}

fn has_non_empty_array(value: &Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
}

fn string_array_at(value: &Value, path: &[&str]) -> Option<Vec<String>> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    let values = current
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    Some(values)
}

fn prompt_variables(
    input: &ProviderContextBuilderInput,
    contract: &crate::protocol::contracts::NodeExecutionContract,
    workflow: &crate::protocol::contracts::WorkflowDisciplineSpec,
    allowed_write_scope: &[String],
    verification_commands: &[String],
) -> Result<BTreeMap<String, String>, ProviderContextBuildError> {
    let runtime_role = serde_json::to_value(&contract.runtime_role)
        .map_err(|error| ProviderContextBuildError::Serialization(error.to_string()))?
        .as_str()
        .unwrap_or_default()
        .to_string();
    let adapter_role = serde_json::to_value(&contract.adapter_role)
        .map_err(|error| ProviderContextBuildError::Serialization(error.to_string()))?
        .as_str()
        .unwrap_or_default()
        .to_string();
    let allowed_write_scope = serde_json::to_string(allowed_write_scope)
        .map_err(|error| ProviderContextBuildError::Serialization(error.to_string()))?;
    let forbidden_actions = serde_json::to_string(&contract.forbidden_actions)
        .map_err(|error| ProviderContextBuildError::Serialization(error.to_string()))?;
    let completion_criteria = serde_json::to_string(&contract.completion_criteria)
        .map_err(|error| ProviderContextBuildError::Serialization(error.to_string()))?;
    let verification_commands = serde_json::to_string(verification_commands)
        .map_err(|error| ProviderContextBuildError::Serialization(error.to_string()))?;
    let canonical_inputs_json = serde_json::to_string(&input.canonical_inputs)
        .map_err(|error| ProviderContextBuildError::Serialization(error.to_string()))?;

    Ok(BTreeMap::from([
        ("node_id".to_string(), input.node_id.clone()),
        ("runtime_role".to_string(), runtime_role),
        ("adapter_role".to_string(), adapter_role),
        (
            "advisory_only".to_string(),
            contract.advisory_only.to_string(),
        ),
        ("allowed_write_scope".to_string(), allowed_write_scope),
        ("timeout_sec".to_string(), contract.timeout_sec.to_string()),
        ("max_retries".to_string(), contract.max_retries.to_string()),
        (
            "canonical_input_summary".to_string(),
            input.canonical_input_summary.clone(),
        ),
        (
            "projection_summary".to_string(),
            input.projection_summary.clone(),
        ),
        (
            "constraint_summary".to_string(),
            input.constraint_summary.clone(),
        ),
        (
            "workflow_discipline_summary".to_string(),
            workflow.superpowers_required.join(", "),
        ),
        (
            "output_schema_summary".to_string(),
            output_schema_summary(&input.node_id, &contract.output_schema_ref),
        ),
        (
            "artifact_kind".to_string(),
            artifact_kind_for_node(&input.node_id).to_string(),
        ),
        ("forbidden_actions".to_string(), forbidden_actions),
        ("completion_criteria".to_string(), completion_criteria),
        ("verification_commands".to_string(), verification_commands),
        ("canonical_inputs_json".to_string(), canonical_inputs_json),
    ]))
}

fn artifact_kind_for_node(node_id: &str) -> &'static str {
    match node_id {
        "N04" => "clarification_record",
        "N05" => "spec",
        "N06" => "advisory_review",
        "N07" => "design",
        "N08" => "design_review",
        "N09" => "design_revision_record",
        "N10" => "readiness_check",
        "N11" => "plan",
        "N12" => "dispatch_package",
        "N16" => "coding_report",
        "N17" => "testing_report",
        "N18" => "code_review_report",
        "N19" => "coding_report",
        "N20" => "ready_advisory",
        "N24" => "integration_verify_advisory",
        "N25" => "final_review",
        "N26" => "dispatch_package",
        "N27" => "final_summary",
        _ => "unknown",
    }
}

fn output_schema_summary(node_id: &str, output_schema_ref: &str) -> String {
    let fields = match artifact_kind_for_node(node_id) {
        "clarification_record" => {
            r#"{"artifact_kind":"clarification_record","goal_summary":"...","constraints":[],"assumptions":[],"open_questions":[],"suggested_scope":"..."}"#
        }
        "spec" => r#"{"artifact_kind":"spec","markdown":"..."}"#,
        "advisory_review" => {
            r#"{"artifact_kind":"advisory_review","findings":[],"blocking_issues":[],"decision_recommendation":"pass"}"#
        }
        "design" => r#"{"artifact_kind":"design","markdown":"..."}"#,
        "design_review" => {
            r#"{"artifact_kind":"design_review","review_decision":"pass","findings":[]}"#
        }
        "design_revision_record" => {
            r#"{"artifact_kind":"design_revision_record","revision_summary":"...","resolved_findings":[],"revised_design_markdown":"..."}"#
        }
        "readiness_check" => {
            r#"{"artifact_kind":"readiness_check","ready":true,"blocking_items":[]}"#
        }
        "plan" => r#"{"artifact_kind":"plan","markdown":"..."}"#,
        "dispatch_package" => r#"{"artifact_kind":"dispatch_package","worktask_routing":[]}"#,
        "coding_report" => {
            r#"{"artifact_kind":"coding_report","worktask_id":"...","files_modified":[],"commands_run":[],"candidate_traceability_refs":[],"status":"completed"}"#
        }
        "testing_report" => {
            r#"{"artifact_kind":"testing_report","worktask_id":"...","commands_run":[],"tests_passed":true,"failures":[],"candidate_traceability_refs":[]}"#
        }
        "code_review_report" => {
            r#"{"artifact_kind":"code_review_report","worktask_id":"...","findings":[],"blocking":false,"candidate_traceability_refs":[]}"#
        }
        "final_review" => {
            r#"{"artifact_kind":"final_review","overall_decision":"pass","coverage_summary":{"closed":[],"uncovered":[],"exempted":[]},"uncovered_items":[],"followup_required":false}"#
        }
        "final_summary" => {
            r#"{"artifact_kind":"final_summary","overall_status":"closed_successfully","next_steps":[],"remaining_risks":[],"closed_items":[]}"#
        }
        _ => r#"{"artifact_kind":"..."}"#,
    };
    format!(
        "{output_schema_ref}\n最终 sentinel 内只能放一个 JSON 对象，不要放 Markdown code fence。不要省略任何 key；没有内容的数组字段必须输出 []。JSON 对象必须至少符合：\n{fields}"
    )
}
