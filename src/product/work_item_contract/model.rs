use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalWorkItemContract {
    pub schema_version: u32,
    pub identity: WorkItemContractIdentity,
    pub goal: WorkItemGoal,
    pub non_goals: Vec<String>,
    pub input_contracts: Vec<RequiredInputContract>,
    pub output_contracts: Vec<PromisedOutputContract>,
    pub tasks: Vec<WorkItemTask>,
    pub write_policy: WorkItemWritePolicy,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub verification_checks: Vec<VerificationCheck>,
    pub handoff_contract: HandoffContract,
    pub blocker_rules: Vec<BlockerRule>,
    pub design_traceability: Vec<DesignTraceabilityRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItemContractIdentity {
    pub logical_work_item_id: String,
    pub title: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredInputContract {
    pub contract_id: String,
    pub provider_logical_work_item_id: String,
    pub required_capabilities: Vec<String>,
    pub compatibility_policy: ContractCompatibilityPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromisedOutputContract {
    pub contract_id: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItemTask {
    pub task_id: String,
    pub statement: String,
    pub requirement_refs: Vec<String>,
    pub done_when_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceCriterion {
    pub criterion_id: String,
    pub statement: String,
    pub required_evidence: Vec<EvidenceKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationCheck {
    pub check_id: String,
    pub command: Option<String>,
    pub manual_instruction: Option<String>,
    pub required: bool,
    pub non_zero_test_execution_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockerRule {
    pub reason_code: String,
    pub route: BlockerRoute,
    pub target_contract_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItemGoal {
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItemWritePolicy {
    pub exclusive_scopes: Vec<String>,
    pub forbidden_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffContract {
    pub required_fields: Vec<String>,
    pub provided_contract_refs: Vec<String>,
    pub reviewer_check_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesignTraceabilityRef {
    pub source_type: String,
    pub source_id: String,
    pub requirement_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    SourceDiff,
    NonZeroTestExecution,
    ManualCheck,
    HandoffField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractCompatibilityPolicy {
    RequireAll,
    RequireAny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerRoute {
    CoderRework,
    VerificationRetry,
    PlanRepairCurrent,
    PlanRepairUpstream,
    SubgraphReplan,
    StoryAmendment,
    DesignAmendment,
    OperationalGate,
}
