use crate::protocol::artifacts::{ArtifactKind, ProjectionKind};
use crate::protocol::enums::{
    AdapterCompatibilityId, AdapterInputRefId, AdapterOutputRefId, ConstraintCheckId,
    ContextPackageId, ExternalRefId, IsoDateTime, NodeId, ProjectionId, ProviderCapabilityId,
    ProviderRunId, SessionId, TaskId, TraceabilityBindingId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRole {
    Orchestrator,
    Executor,
    Reviewer,
    AdvisoryReviewer,
}

impl RuntimeRole {
    pub fn adapter_role(&self) -> AdapterRole {
        match self {
            RuntimeRole::Orchestrator => AdapterRole::Orchestrator,
            RuntimeRole::Executor => AdapterRole::Executor,
            RuntimeRole::Reviewer | RuntimeRole::AdvisoryReviewer => AdapterRole::Reviewer,
        }
    }

    pub fn advisory_only(&self) -> bool {
        matches!(self, RuntimeRole::AdvisoryReviewer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterRole {
    Orchestrator,
    Executor,
    Reviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    ClaudeCode,
    Codex,
    Fake,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandClass {
    ReadOnly,
    FileWrite,
    Test,
    GitRead,
    GitWrite,
    ProcessSpawn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutStatus {
    NotTimedOut,
    SoftTimeoutTerminated,
    HardTimeoutKilled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Never,
    OnRequest,
    OnFailure,
    ManualGate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    RecoveredPending,
    ManualIntervention,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NodeExecutionContract {
    pub node_id: NodeId,
    pub provider_type: ProviderType,
    pub runtime_role: RuntimeRole,
    pub adapter_role: AdapterRole,
    pub advisory_only: bool,
    pub required_canonical_inputs: Vec<ArtifactKind>,
    pub required_projection_kinds: Vec<ProjectionKind>,
    pub required_constraint_kinds: Vec<String>,
    pub allowed_external_inputs: Vec<String>,
    pub allowed_write_scope: Vec<String>,
    pub allowed_command_classes: Vec<CommandClass>,
    pub forbidden_actions: Vec<String>,
    pub expected_outputs: Vec<ArtifactKind>,
    pub output_schema_ref: String,
    pub prompt_template_id: String,
    pub completion_criteria: Vec<String>,
    pub verification_commands: Vec<String>,
    pub timeout_sec: u64,
    pub max_retries: u32,
    pub failure_routes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowDisciplineSpec {
    pub node_id: NodeId,
    pub superpowers_required: Vec<String>,
    pub superpowers_optional: Vec<String>,
    pub required_skills: Vec<String>,
    pub verification_required: bool,
    pub tdd_required: bool,
    pub evidence_required: bool,
    pub enforcement_level: String,
    pub workflow_violation_route: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSection {
    System,
    NodeContract,
    CanonicalInputs,
    ProjectionSummary,
    ConstraintSummary,
    WorkflowDiscipline,
    OutputSchema,
    CompletionOrFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NodePromptTemplateRef {
    pub template_id: String,
    pub template_version: String,
    pub system_instruction_ref: String,
    pub render_order: Vec<PromptSection>,
    pub required_sections: Vec<PromptSection>,
    pub output_schema_ref: String,
    pub output_instruction_ref: String,
    pub failure_instruction_ref: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderContextPackage {
    pub context_package_id: ContextPackageId,
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub node_id: NodeId,
    pub provider_type: ProviderType,
    pub runtime_role: RuntimeRole,
    pub adapter_role: AdapterRole,
    pub advisory_only: bool,
    pub canonical_inputs: Value,
    pub projection_refs: Vec<ProjectionId>,
    pub constraint_bundle_ref: String,
    pub node_execution_contract: NodeExecutionContract,
    pub workflow_discipline: WorkflowDisciplineSpec,
    pub prompt_template: NodePromptTemplateRef,
    pub worktree_path: Option<String>,
    pub allowed_write_scope: Vec<String>,
    pub context_files: Vec<String>,
    pub instructions: Vec<String>,
    pub output_schema_ref: String,
    pub completion_criteria: Vec<String>,
    pub forbidden_actions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub timeout_sec: u64,
    pub max_retries: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AdapterInput {
    pub provider_type: ProviderType,
    pub role: AdapterRole,
    pub worktree_path: Option<String>,
    pub prompt: String,
    pub context_files: Vec<String>,
    pub output_schema: String,
    pub timeout: u64,
    pub max_retries: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AdapterOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub structured_output: Option<Value>,
    pub files_modified: Vec<String>,
    pub duration_ms: u64,
    pub timeout_status: TimeoutStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderRunRecord {
    pub provider_run_id: ProviderRunId,
    pub node_id: NodeId,
    pub provider_type: ProviderType,
    pub runtime_role: RuntimeRole,
    pub adapter_role: AdapterRole,
    pub provider_capability_ref: ProviderCapabilityId,
    pub adapter_compatibility_ref: AdapterCompatibilityId,
    pub context_package_ref: ContextPackageId,
    pub adapter_input_ref: AdapterInputRefId,
    pub adapter_output_ref: AdapterOutputRefId,
    pub raw_artifact_refs: Vec<ExternalRefId>,
    pub exit_code: Option<i32>,
    pub error_code: Option<String>,
    pub error_details: Option<String>,
    pub stdout_ref: Option<ExternalRefId>,
    pub stderr_ref: Option<ExternalRefId>,
    pub structured_output_ref: Option<ExternalRefId>,
    pub files_modified: Vec<String>,
    pub status: ProviderRunStatus,
    pub started_at: IsoDateTime,
    pub completed_at: Option<IsoDateTime>,
    pub duration_ms: Option<u64>,
    pub timeout_status: TimeoutStatus,
    pub retry_count: u32,
    pub approval_policy: ApprovalPolicy,
    pub sandbox_mode: SandboxMode,
    pub constraint_check_ref: Option<ConstraintCheckId>,
    pub traceability_binding_refs: Vec<TraceabilityBindingId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Phase1NodeContractRow {
    pub node_id: NodeId,
    pub provider_type: Option<ProviderType>,
    pub runtime_role: Option<RuntimeRole>,
    pub adapter_role: Option<AdapterRole>,
    pub advisory_only: bool,
    pub output_schema_ref: Option<String>,
    pub prompt_template_id: Option<String>,
}

pub fn execution_contract_for_node(node_id: &str) -> Option<NodeExecutionContract> {
    let (provider_type, runtime_role, output_schema_ref, prompt_template_id, expected_outputs) =
        match node_id {
            "N04" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/clarification_record/v1",
                "tpl_n04_clarification_v1",
                vec![ArtifactKind::ClarificationRecord],
            ),
            "N05" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/spec/v1",
                "tpl_n05_spec_authoring_v1",
                vec![ArtifactKind::Spec],
            ),
            "N06" => (
                ProviderType::Codex,
                RuntimeRole::AdvisoryReviewer,
                "schema://aria/advisory/spec_gate_review/v1",
                "tpl_n06_spec_gate_advisory_v1",
                Vec::new(),
            ),
            "N07" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/design/v1",
                "tpl_n07_design_authoring_v1",
                vec![ArtifactKind::Design],
            ),
            "N08" => (
                ProviderType::Codex,
                RuntimeRole::Reviewer,
                "schema://aria/artifacts/design_review/v1",
                "tpl_n08_design_review_v1",
                vec![ArtifactKind::DesignReview],
            ),
            "N09" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/design_revision_record/v1",
                "tpl_n09_design_revision_v1",
                vec![ArtifactKind::DesignRevisionRecord, ArtifactKind::Design],
            ),
            "N10" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/readiness_check/v1",
                "tpl_n10_readiness_check_v1",
                vec![ArtifactKind::ReadinessCheck],
            ),
            "N11" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/plan/v1",
                "tpl_n11_plan_authoring_v1",
                vec![ArtifactKind::Plan],
            ),
            "N12" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/dispatch_package/v1",
                "tpl_n12_dispatch_authoring_v1",
                vec![ArtifactKind::DispatchPackage],
            ),
            "N16" => (
                ProviderType::Codex,
                RuntimeRole::Executor,
                "schema://aria/artifacts/coding_report/v1",
                "tpl_n16_coding_v1",
                vec![ArtifactKind::CodingReport],
            ),
            "N17" => (
                ProviderType::Codex,
                RuntimeRole::Executor,
                "schema://aria/artifacts/testing_report/v1",
                "tpl_n17_testing_v1",
                vec![ArtifactKind::TestingReport],
            ),
            "N18" => (
                ProviderType::Codex,
                RuntimeRole::Reviewer,
                "schema://aria/artifacts/code_review_report/v1",
                "tpl_n18_code_review_v1",
                vec![ArtifactKind::CodeReviewReport],
            ),
            "N19" => (
                ProviderType::Codex,
                RuntimeRole::Executor,
                "schema://aria/artifacts/coding_report/v1",
                "tpl_n19_rework_v1",
                vec![ArtifactKind::CodingReport],
            ),
            "N20" => (
                ProviderType::Codex,
                RuntimeRole::AdvisoryReviewer,
                "schema://aria/advisory/ready_advisory/v1",
                "tpl_n20_ready_advisory_v1",
                Vec::new(),
            ),
            "N24" => (
                ProviderType::Codex,
                RuntimeRole::AdvisoryReviewer,
                "schema://aria/advisory/integration_verify_advisory/v1",
                "tpl_n24_integration_verify_advisory_v1",
                Vec::new(),
            ),
            "N25" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/final_review/v1",
                "tpl_n25_final_review_v1",
                vec![ArtifactKind::FinalReview],
            ),
            "N26" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/dispatch_package/v1",
                "tpl_n26_patch_followup_dispatch_v1",
                vec![ArtifactKind::DispatchPackage],
            ),
            "N27" => (
                ProviderType::ClaudeCode,
                RuntimeRole::Orchestrator,
                "schema://aria/artifacts/final_summary/v1",
                "tpl_n27_final_summary_v1",
                vec![ArtifactKind::FinalSummary],
            ),
            _ => return None,
        };
    let adapter_role = runtime_role.adapter_role();

    Some(NodeExecutionContract {
        node_id: node_id.to_string(),
        provider_type,
        runtime_role: runtime_role.clone(),
        adapter_role,
        advisory_only: runtime_role.advisory_only(),
        required_canonical_inputs: required_canonical_inputs(node_id),
        required_projection_kinds: required_projection_kinds(node_id),
        required_constraint_kinds: required_constraint_kinds(node_id),
        allowed_external_inputs: allowed_external_inputs(node_id),
        allowed_write_scope: allowed_write_scope(node_id),
        allowed_command_classes: allowed_command_classes(node_id),
        forbidden_actions: forbidden_actions(node_id),
        expected_outputs,
        output_schema_ref: output_schema_ref.to_string(),
        prompt_template_id: prompt_template_id.to_string(),
        completion_criteria: completion_criteria(node_id),
        verification_commands: verification_commands(node_id),
        timeout_sec: 30,
        max_retries: 1,
        failure_routes: failure_routes(node_id),
    })
}

pub fn phase1_node_contract_table() -> Vec<Phase1NodeContractRow> {
    vec![
        internal_row("N13", None),
        internal_row("N14", None),
        internal_row("N15", None),
        provider_row("N16"),
        provider_row("N17"),
        provider_row("N18"),
        provider_row("N19"),
        provider_row("N20"),
        internal_row("N21", None),
        internal_row("N22", None),
        internal_row("N23", Some("schema://aria/artifacts/integration_report/v1")),
        provider_row("N24"),
        provider_row("N25"),
        provider_row("N26"),
        provider_row("N27"),
        internal_row("N28", None),
    ]
}

pub fn workflow_discipline_for_node(node_id: &str) -> Option<WorkflowDisciplineSpec> {
    execution_contract_for_node(node_id)?;
    let mut superpowers_required = vec!["using-superpowers".to_string()];
    if matches!(node_id, "N04" | "N05" | "N07") {
        superpowers_required.push("brainstorming".to_string());
    }
    if node_id == "N11" {
        superpowers_required.push("writing-plans".to_string());
    }
    if node_id == "N16" {
        superpowers_required.push("test-driven-development".to_string());
        superpowers_required.push("verification-before-completion".to_string());
    }
    if node_id == "N17" {
        superpowers_required.push("verification-before-completion".to_string());
    }
    if node_id == "N19" {
        superpowers_required.push("receiving-code-review".to_string());
        superpowers_required.push("verification-before-completion".to_string());
    }

    let superpowers_optional = match node_id {
        "N17" => vec!["systematic-debugging".to_string()],
        "N19" => vec![
            "systematic-debugging".to_string(),
            "test-driven-development".to_string(),
        ],
        _ => vec!["verification-before-completion".to_string()],
    };

    Some(WorkflowDisciplineSpec {
        node_id: node_id.to_string(),
        tdd_required: superpowers_required.contains(&"test-driven-development".to_string()),
        verification_required: superpowers_required
            .contains(&"verification-before-completion".to_string())
            || !matches!(node_id, "N18" | "N20" | "N24" | "N25" | "N26" | "N27"),
        superpowers_required,
        superpowers_optional,
        required_skills: Vec::new(),
        evidence_required: true,
        enforcement_level: "required".to_string(),
        workflow_violation_route: "gate".to_string(),
        notes: vec!["provider output is candidate-only".to_string()],
    })
}

fn required_canonical_inputs(node_id: &str) -> Vec<ArtifactKind> {
    match node_id {
        "N04" => vec![ArtifactKind::IntakeBrief],
        "N05" => vec![ArtifactKind::IntakeBrief, ArtifactKind::ClarificationRecord],
        "N06" => vec![ArtifactKind::Spec, ArtifactKind::ClarificationRecord],
        "N07" => vec![ArtifactKind::Spec, ArtifactKind::SpecGateDecision],
        "N08" => vec![ArtifactKind::Design, ArtifactKind::Spec],
        "N09" => vec![ArtifactKind::Design, ArtifactKind::DesignReview],
        "N10" => vec![
            ArtifactKind::Spec,
            ArtifactKind::Design,
            ArtifactKind::DesignReview,
        ],
        "N11" => vec![
            ArtifactKind::ReadinessCheck,
            ArtifactKind::Spec,
            ArtifactKind::Design,
        ],
        "N12" => vec![ArtifactKind::Plan],
        "N16" => vec![ArtifactKind::DispatchPackage, ArtifactKind::Plan],
        "N17" => vec![ArtifactKind::CodingReport, ArtifactKind::DispatchPackage],
        "N18" => vec![
            ArtifactKind::CodingReport,
            ArtifactKind::TestingReport,
            ArtifactKind::DispatchPackage,
        ],
        "N19" => vec![ArtifactKind::TestingReport, ArtifactKind::CodeReviewReport],
        "N20" => vec![
            ArtifactKind::CodingReport,
            ArtifactKind::TestingReport,
            ArtifactKind::CodeReviewReport,
        ],
        "N24" => vec![ArtifactKind::IntegrationReport],
        "N25" => vec![
            ArtifactKind::IntegrationReport,
            ArtifactKind::DispatchPackage,
        ],
        "N26" => vec![ArtifactKind::FinalReview],
        "N27" => vec![ArtifactKind::FinalReview],
        _ => Vec::new(),
    }
}

fn required_projection_kinds(node_id: &str) -> Vec<ProjectionKind> {
    match node_id {
        "N07" => vec![ProjectionKind::SpecProjection],
        "N08" | "N09" | "N10" | "N11" => {
            vec![
                ProjectionKind::SpecProjection,
                ProjectionKind::DesignProjection,
            ]
        }
        "N12" => vec![ProjectionKind::PlanProjection],
        "N16" | "N17" | "N18" | "N19" | "N20" | "N24" => vec![
            ProjectionKind::SpecProjection,
            ProjectionKind::DesignProjection,
            ProjectionKind::PlanProjection,
        ],
        "N25" | "N26" | "N27" => vec![
            ProjectionKind::SpecProjection,
            ProjectionKind::DesignProjection,
            ProjectionKind::PlanProjection,
        ],
        _ => Vec::new(),
    }
}

fn required_constraint_kinds(node_id: &str) -> Vec<String> {
    match node_id {
        "N04" | "N05" | "N06" => vec!["proposal_constraints".to_string()],
        "N07" => vec!["requirement_constraints".to_string()],
        "N08" | "N09" | "N10" | "N11" => vec![
            "requirement_constraints".to_string(),
            "design_constraints".to_string(),
        ],
        "N12" => vec!["task_constraints".to_string()],
        "N16" | "N17" | "N18" | "N19" | "N20" | "N24" | "N25" | "N26" | "N27" => {
            vec!["task_constraints".to_string()]
        }
        _ => Vec::new(),
    }
}

fn allowed_external_inputs(node_id: &str) -> Vec<String> {
    let mut inputs = vec!["openspec".to_string(), "superpowers".to_string()];
    if matches!(node_id, "N16" | "N17" | "N18" | "N19" | "N20" | "N24") {
        inputs.push("worktree".to_string());
    }
    inputs
}

fn allowed_write_scope(node_id: &str) -> Vec<String> {
    match node_id {
        "N16" | "N19" => vec!["<worktask_routing.allowed_write_scope>".to_string()],
        "N25" | "N26" | "N27" => vec![
            "aria-artifacts/**".to_string(),
            "stdout-candidates/**".to_string(),
        ],
        _ => Vec::new(),
    }
}

fn allowed_command_classes(node_id: &str) -> Vec<CommandClass> {
    match node_id {
        "N16" | "N19" => vec![
            CommandClass::ReadOnly,
            CommandClass::FileWrite,
            CommandClass::Test,
            CommandClass::ProcessSpawn,
        ],
        "N17" => vec![
            CommandClass::ReadOnly,
            CommandClass::Test,
            CommandClass::ProcessSpawn,
        ],
        "N25" | "N26" | "N27" => vec![CommandClass::ReadOnly, CommandClass::FileWrite],
        _ => vec![CommandClass::ReadOnly],
    }
}

fn forbidden_actions(node_id: &str) -> Vec<String> {
    let mut actions = vec![
        "do_not_commit".to_string(),
        "do_not_modify_openspec_directly".to_string(),
        "do_not_advance_daemon_state".to_string(),
    ];
    if matches!(node_id, "N16" | "N17" | "N18" | "N19") {
        actions.push("do_not_write_outside_allowed_scope".to_string());
    }
    if matches!(node_id, "N20" | "N24") {
        actions.push("do_not_make_daemon_decision".to_string());
    }
    if node_id == "N26" {
        actions.push("do_not_dispatch_followup_without_gate".to_string());
    }
    actions
}

fn completion_criteria(node_id: &str) -> Vec<String> {
    let mut criteria = vec![
        "emit_aria_structured_output".to_string(),
        "preserve_daemon_as_runtime_truth".to_string(),
    ];
    if matches!(node_id, "N16" | "N17" | "N18" | "N19") {
        criteria.push("bind_output_to_worktask_traceability".to_string());
    }
    if matches!(node_id, "N25" | "N27") {
        criteria.push("report_coverage_summary".to_string());
    }
    criteria
}

fn verification_commands(node_id: &str) -> Vec<String> {
    match node_id {
        "N16" | "N17" | "N18" | "N19" | "N20" => {
            vec!["cargo test --test execution_chain_fake_provider".to_string()]
        }
        "N24" => vec!["cargo test --test integration_retry_limit".to_string()],
        "N25" | "N26" | "N27" => vec![
            "cargo test --test final_closure_fake_provider --test final_followup_routes"
                .to_string(),
        ],
        _ => Vec::new(),
    }
}

fn failure_routes(node_id: &str) -> Vec<String> {
    match node_id {
        "N17" => vec!["N19".to_string(), "X08".to_string()],
        "N18" => vec!["N19".to_string(), "M20".to_string(), "X08".to_string()],
        "N19" => vec!["N17".to_string(), "N18".to_string(), "M20".to_string()],
        "N20" => vec!["N21".to_string(), "N19".to_string(), "X08".to_string()],
        "N24" => vec!["N19".to_string(), "X08".to_string()],
        "N25" => vec!["N27".to_string(), "X01".to_string(), "X08".to_string()],
        "N26" => vec!["N13".to_string(), "X08".to_string()],
        "N27" => vec!["N28".to_string()],
        _ => vec!["retry".to_string(), "gate".to_string()],
    }
}

fn internal_row(node_id: &str, output_schema_ref: Option<&str>) -> Phase1NodeContractRow {
    Phase1NodeContractRow {
        node_id: node_id.to_string(),
        provider_type: None,
        runtime_role: None,
        adapter_role: None,
        advisory_only: false,
        output_schema_ref: output_schema_ref.map(str::to_string),
        prompt_template_id: None,
    }
}

fn provider_row(node_id: &str) -> Phase1NodeContractRow {
    let contract = execution_contract_for_node(node_id).expect("phase1 provider node contract");
    Phase1NodeContractRow {
        node_id: node_id.to_string(),
        provider_type: Some(contract.provider_type),
        runtime_role: Some(contract.runtime_role),
        adapter_role: Some(contract.adapter_role),
        advisory_only: contract.advisory_only,
        output_schema_ref: Some(contract.output_schema_ref),
        prompt_template_id: Some(contract.prompt_template_id),
    }
}
