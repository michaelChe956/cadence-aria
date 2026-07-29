use serde::{Deserialize, Serialize};

use crate::product::json_store::ProductStoreError;
use crate::product::models::{
    PlanDefectClass, PlanDefectEvidence, PlanDefectRoute, RepairTarget, RepairTargetKind,
};
use crate::product::work_item_contract::{BlockerRoute, ContractValidationReport};
use crate::product::work_item_projection::ProjectionValidationReport;

pub fn plan_defect_structured_output_contract() -> &'static str {
    "\nPlan Defect structured output contract:\n\
     - Coder 仅在发现计划、Story、Design、依赖契约、验证或运行环境阻塞时输出 plan_defect_findings 数组；普通成功输出或普通 implementation defect 可省略该数组或使用空数组。\n\
     - CodeReviewer/InternalReviewer/GroupFinalReview 在 findings 中使用同一字段；普通 implementation defect 必须显式使用 defect_class=implementation_defect、recommended_route=coder_rework，reason_code=null、contract_refs=[]、capability_refs=[]、repair_target=null、confidence=null。\n\
     - 每个 plan defect finding 必须包含 finding_id、severity、defect_class、reason_code、message、evidence、contract_refs、capability_refs、repair_target、recommended_route、confidence。\n\
     - severity 只能使用 error、warning；阻塞问题使用 severity=error，不得使用 blocking、blocker 等其他取值。\n\
     - confidence 只能使用 low、medium、high；不得使用 0~1 的小数或百分比。\n\
     - repair_target 必须是对象，包含 kind（current_work_item、upstream_work_item 或 subgraph）、logical_work_item_ids、work_item_revision_ids；没有明确修复目标时使用 repair_target=null，不得使用字符串。\n\
     - evidence 是对象数组，每项包含 kind、source_ref、message；不得把缺失 contract、target、confidence 的普通 finding 伪造成 plan defect。\n\
     - 路由优先级固定为 Story -> Design -> Plan Repair -> Operational -> Verification -> Implementation。\n"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDefectSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDefectConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefectFinding {
    pub finding_id: String,
    pub severity: PlanDefectSeverity,
    pub defect_class: PlanDefectClass,
    pub reason_code: String,
    pub message: String,
    pub evidence: Vec<PlanDefectEvidence>,
    pub contract_refs: Vec<String>,
    pub capability_refs: Vec<String>,
    pub repair_target: Option<RepairTarget>,
    pub recommended_route: PlanDefectRoute,
    pub confidence: PlanDefectConfidence,
}

impl PlanDefectFinding {
    pub fn validate(&self) -> Result<(), PlanRepairError> {
        validate_finding(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedPlanDefectRoute {
    pub route: PlanDefectRoute,
    pub required_target_kind: Option<RepairTargetKind>,
}

#[derive(Debug)]
pub enum PlanRepairError {
    InvalidFinding(String),
    InvalidRepairTarget(String),
    ContractValidation(ContractValidationReport),
    ProjectionValidation(ProjectionValidationReport),
    ActiveAmendmentExists { amendment_id: String },
    AmendmentConflict { expected: String, actual: String },
    ConfirmationRequired,
    RiskAcceptanceRequired,
    Store(ProductStoreError),
}

pub fn default_route(class: &PlanDefectClass) -> PlanDefectRoute {
    match class {
        PlanDefectClass::ImplementationDefect => PlanDefectRoute::CoderRework,
        PlanDefectClass::VerificationIncomplete => PlanDefectRoute::VerificationRetry,
        PlanDefectClass::CurrentWorkItemInvalid
        | PlanDefectClass::UpstreamContractInvalid
        | PlanDefectClass::DependencyGraphInvalid => PlanDefectRoute::PlanRepair,
        PlanDefectClass::DesignAmendmentRequired => PlanDefectRoute::DesignAmendment,
        PlanDefectClass::StoryAmendmentRequired => PlanDefectRoute::StoryAmendment,
        PlanDefectClass::OperationalBlocker => PlanDefectRoute::OperationalGate,
    }
}

pub fn normalize_blocker_route(route: BlockerRoute) -> NormalizedPlanDefectRoute {
    match route {
        BlockerRoute::CoderRework => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::CoderRework,
            required_target_kind: None,
        },
        BlockerRoute::VerificationRetry => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::VerificationRetry,
            required_target_kind: None,
        },
        BlockerRoute::PlanRepairCurrent => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::PlanRepair,
            required_target_kind: Some(RepairTargetKind::CurrentWorkItem),
        },
        BlockerRoute::PlanRepairUpstream => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::PlanRepair,
            required_target_kind: Some(RepairTargetKind::UpstreamWorkItem),
        },
        BlockerRoute::SubgraphReplan => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::PlanRepair,
            required_target_kind: Some(RepairTargetKind::Subgraph),
        },
        BlockerRoute::StoryAmendment => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::StoryAmendment,
            required_target_kind: None,
        },
        BlockerRoute::DesignAmendment => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::DesignAmendment,
            required_target_kind: None,
        },
        BlockerRoute::OperationalGate => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::OperationalGate,
            required_target_kind: None,
        },
    }
}

pub fn validate_finding(finding: &PlanDefectFinding) -> Result<(), PlanRepairError> {
    let expected_route = default_route(&finding.defect_class);
    if finding.recommended_route != expected_route {
        return Err(PlanRepairError::InvalidFinding(format!(
            "defect class {:?} requires route {:?}, got {:?}",
            finding.defect_class, expected_route, finding.recommended_route
        )));
    }

    match (
        required_target_kind(&finding.defect_class),
        &finding.repair_target,
    ) {
        (Some(expected_kind), Some(target)) => {
            if target.kind != expected_kind {
                return Err(PlanRepairError::InvalidRepairTarget(format!(
                    "defect class {:?} requires target {:?}, got {:?}",
                    finding.defect_class, expected_kind, target.kind
                )));
            }
            if target.logical_work_item_ids.is_empty() || target.work_item_revision_ids.is_empty() {
                return Err(PlanRepairError::InvalidRepairTarget(
                    "plan repair target requires logical work item and revision ids".to_string(),
                ));
            }
        }
        (Some(expected_kind), None) => {
            return Err(PlanRepairError::InvalidRepairTarget(format!(
                "defect class {:?} requires target {:?}",
                finding.defect_class, expected_kind
            )));
        }
        (None, Some(target)) => {
            return Err(PlanRepairError::InvalidRepairTarget(format!(
                "route {:?} does not accept target {:?}",
                finding.recommended_route, target.kind
            )));
        }
        (None, None) => {}
    }

    Ok(())
}

fn required_target_kind(class: &PlanDefectClass) -> Option<RepairTargetKind> {
    match class {
        PlanDefectClass::CurrentWorkItemInvalid => Some(RepairTargetKind::CurrentWorkItem),
        PlanDefectClass::UpstreamContractInvalid => Some(RepairTargetKind::UpstreamWorkItem),
        PlanDefectClass::DependencyGraphInvalid => Some(RepairTargetKind::Subgraph),
        PlanDefectClass::ImplementationDefect
        | PlanDefectClass::VerificationIncomplete
        | PlanDefectClass::DesignAmendmentRequired
        | PlanDefectClass::StoryAmendmentRequired
        | PlanDefectClass::OperationalBlocker => None,
    }
}
