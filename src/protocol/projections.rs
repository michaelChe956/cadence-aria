use crate::protocol::artifacts::{ArtifactRef, ProjectionKind};
use crate::protocol::enums::{IsoDateTime, NodeId, ProjectionId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub type WorkPackageId = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ArtifactProjectionRecord {
    pub projection_id: ProjectionId,
    pub projection_kind: ProjectionKind,
    pub source_artifact_ref: ArtifactRef,
    pub source_artifact_version: u32,
    pub source_artifact_hash: String,
    pub compiled_at: IsoDateTime,
    pub compiled_by_node: NodeId,
    pub payload: ProjectionPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "payload_kind", content = "payload", rename_all = "snake_case")]
pub enum ProjectionPayload {
    SpecProjection(SpecProjection),
    DesignProjection(DesignProjection),
    PlanProjection(PlanProjection),
}

impl ProjectionPayload {
    pub fn projection_kind(&self) -> ProjectionKind {
        match self {
            ProjectionPayload::SpecProjection(_) => ProjectionKind::SpecProjection,
            ProjectionPayload::DesignProjection(_) => ProjectionKind::DesignProjection,
            ProjectionPayload::PlanProjection(_) => ProjectionKind::PlanProjection,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            ProjectionPayload::SpecProjection(payload) => {
                payload.functional_requirements.is_empty() && payload.success_criteria.is_empty()
            }
            ProjectionPayload::DesignProjection(payload) => payload.design_decisions.is_empty(),
            ProjectionPayload::PlanProjection(payload) => payload.work_packages.is_empty(),
        }
    }

    pub fn inner_json(&self) -> serde_json::Result<serde_json::Value> {
        match self {
            ProjectionPayload::SpecProjection(payload) => serde_json::to_value(payload),
            ProjectionPayload::DesignProjection(payload) => serde_json::to_value(payload),
            ProjectionPayload::PlanProjection(payload) => serde_json::to_value(payload),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SpecProjection {
    pub user_stories: Vec<UserStoryProjection>,
    pub functional_requirements: Vec<RequirementProjection>,
    pub success_criteria: Vec<CriterionProjection>,
    pub open_items: Vec<OpenItemProjection>,
    pub non_functional_requirements: Vec<RequirementProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UserStoryProjection {
    pub story_id: String,
    pub title: String,
    pub related_requirement_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RequirementProjection {
    pub requirement_id: String,
    pub text: String,
    pub priority: RequirementPriority,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CriterionProjection {
    pub criterion_id: String,
    pub text: String,
    pub related_requirement_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OpenItemProjection {
    pub item_id: String,
    pub text: String,
    pub resolution_mode: ResolutionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignProjection {
    pub design_decisions: Vec<DesignDecisionProjection>,
    pub shared_components: Vec<ComponentProjection>,
    pub shared_modules: Vec<ComponentProjection>,
    pub data_entities: Vec<DataEntityProjection>,
    pub api_entries: Vec<ApiEntryProjection>,
    pub risk_refs: Vec<RiskProjection>,
    pub open_items: Vec<OpenItemProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DesignDecisionProjection {
    pub design_decision_id: String,
    pub text: String,
    pub related_requirement_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ComponentProjection {
    pub component_id: String,
    pub name: String,
    pub responsibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DataEntityProjection {
    pub entity_id: String,
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApiEntryProjection {
    pub api_id: String,
    pub name: String,
    pub input: String,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RiskProjection {
    pub risk_id: String,
    pub text: String,
    pub severity: RiskSeverity,
    pub mitigation: Option<String>,
    pub related_design_decision_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanProjection {
    pub work_packages: Vec<WorkPackageProjection>,
    pub dependencies: Vec<WorkDependencyProjection>,
    pub parallelism_groups: Vec<ParallelismGroupProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkPackageProjection {
    pub work_package_id: WorkPackageId,
    pub description: String,
    pub execution_mode: ExecutionMode,
    pub human_required_reason: Option<String>,
    pub traceability_refs: Vec<String>,
    pub acceptance_targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkDependencyProjection {
    pub from_work_package_id: WorkPackageId,
    pub to_work_package_id: WorkPackageId,
    pub dependency_type: DependencyType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ParallelismGroupProjection {
    pub group_id: String,
    pub work_package_ids: Vec<WorkPackageId>,
    pub max_parallel: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementPriority {
    Must,
    Should,
    Could,
    Wont,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionMode {
    Deferred,
    Resolved,
    OutOfScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskSeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    AgentOnly,
    HumanAssisted,
    HumanRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyType {
    Blocks,
    DependsOn,
    Parallel,
}

impl fmt::Display for RequirementPriority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            RequirementPriority::Must => "must",
            RequirementPriority::Should => "should",
            RequirementPriority::Could => "could",
            RequirementPriority::Wont => "wont",
        })
    }
}

impl fmt::Display for RiskSeverity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            RiskSeverity::Low => "low",
            RiskSeverity::Medium => "medium",
            RiskSeverity::High => "high",
            RiskSeverity::Critical => "critical",
        })
    }
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            ExecutionMode::AgentOnly => "agent_only",
            ExecutionMode::HumanAssisted => "human_assisted",
            ExecutionMode::HumanRequired => "human_required",
        })
    }
}

impl fmt::Display for DependencyType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            DependencyType::Blocks => "blocks",
            DependencyType::DependsOn => "depends_on",
            DependencyType::Parallel => "parallel",
        })
    }
}

impl FromStr for RequirementPriority {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_value(value).as_str() {
            "must" => Ok(RequirementPriority::Must),
            "should" | "" => Ok(RequirementPriority::Should),
            "could" => Ok(RequirementPriority::Could),
            "wont" | "won_t" => Ok(RequirementPriority::Wont),
            _ => Err(value.to_string()),
        }
    }
}

impl FromStr for RiskSeverity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_value(value).as_str() {
            "low" => Ok(RiskSeverity::Low),
            "medium" | "" => Ok(RiskSeverity::Medium),
            "high" => Ok(RiskSeverity::High),
            "critical" => Ok(RiskSeverity::Critical),
            _ => Err(value.to_string()),
        }
    }
}

impl FromStr for ExecutionMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_value(value).as_str() {
            "agent_only" => Ok(ExecutionMode::AgentOnly),
            "human_assisted" => Ok(ExecutionMode::HumanAssisted),
            "human_required" => Ok(ExecutionMode::HumanRequired),
            _ => Err(value.to_string()),
        }
    }
}

impl FromStr for DependencyType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match normalize_enum_value(value).as_str() {
            "blocks" => Ok(DependencyType::Blocks),
            "depends_on" => Ok(DependencyType::DependsOn),
            "parallel" => Ok(DependencyType::Parallel),
            _ => Err(value.to_string()),
        }
    }
}

fn normalize_enum_value(value: &str) -> String {
    value
        .trim()
        .trim_matches(';')
        .trim_matches(',')
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
}
