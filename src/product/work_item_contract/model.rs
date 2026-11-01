use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalWorkItemContract {
    pub schema_version: u32,
    pub identity: WorkItemContractIdentity,
    pub goal: WorkItemGoal,
    pub non_goals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub input_contracts: Vec<RequiredInputContract>,
    pub output_contracts: Vec<PromisedOutputContract>,
    pub tasks: Vec<WorkItemTask>,
    pub write_policy: WorkItemWritePolicy,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub verification_checks: Vec<VerificationCheck>,
    pub handoff_contract: HandoffContract,
    pub blocker_rules: Vec<BlockerRule>,
    pub design_traceability: Vec<DesignTraceabilityRef>,
    /// C1 Task 5（REQ-C1-PLAN-01）：可选但显式的 existing/create 意图合同；
    /// 旧 plan JSON 缺省 None——不自动补声明，走既有 Manual/旧路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_contract: Option<WorkItemIntentContract>,
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
    #[serde(deserialize_with = "deserialize_evidence_kind_list")]
    pub required_evidence: Vec<EvidenceKind>,
}

/// Providers occasionally emit `required_evidence` as a scalar enum string instead of
/// the required array (observed with Pi, workspace_session_0003/timeline_node_014).
/// Accept a single valid evidence-kind string and normalize it into a one-element
/// list; invalid values keep failing deserialization.
fn deserialize_evidence_kind_list<'de, D>(deserializer: D) -> Result<Vec<EvidenceKind>, D::Error>
where
    D: Deserializer<'de>,
{
    struct EvidenceKindListVisitor;

    impl<'de> serde::de::Visitor<'de> for EvidenceKindListVisitor {
        type Value = Vec<EvidenceKind>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an array of evidence kinds or a single evidence kind string")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut items = Vec::new();
            while let Some(item) = seq.next_element::<EvidenceKind>()? {
                items.push(item);
            }
            Ok(items)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            EvidenceKind::deserialize(de::value::StrDeserializer::<E>::new(value))
                .map(|kind| vec![kind])
                .map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(EvidenceKindListVisitor)
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

/// C1 Task 5（REQ-C1-PLAN-01）：work item 写入意图。`Existing`＝引用既有
/// Work Item；`Create`＝本 plan 明确新建。缺失意图在涉及基线外新建写面时
/// 以 `intent_undeclared` 停等修订，不得由系统自动补声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemIntent {
    Existing,
    Create,
}

/// C1 Task 5：existing/create 意图合同——provider Work Item 身份、依赖闭包、
/// exclusive/forbidden scope 与 target 的冻结快照（scope/依赖由 lowering 从
/// 本 item 的 Dependencies/Write Policy 单一来源快照，不在 markdown 重复）。
/// 校验失败区分 `intent_undeclared`（未声明）与 `intent_unexecutable`
///（不能执行），fail-closed 拒绝 compile，不清既有 finding。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItemIntentContract {
    pub intent: WorkItemIntent,
    pub provider_work_item_id: String,
    pub depends_on: Vec<String>,
    pub exclusive_scopes: Vec<String>,
    pub forbidden_scopes: Vec<String>,
    pub target: crate::product::logical_codebase::EnrollmentTarget,
}
