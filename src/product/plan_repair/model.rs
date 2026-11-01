use serde::{Deserialize, Serialize};

use crate::product::json_store::ProductStoreError;
use crate::product::models::{
    PlanDefectClass, PlanDefectEvidence, PlanDefectRoute, RepairTarget, RepairTargetKind,
};
use crate::product::work_item_contract::{BlockerRoute, ContractValidationReport};
use crate::product::work_item_projection::ProjectionValidationReport;

// 3.6 矩阵族③根因修复（A 件）：defect_class 逐字枚举行（净增约 408B 渲染字节；
// coding 侧 prompt 无 MAX_BYTES 预算常量，按惯例只做批注）。取值清单与
// PlanDefectClass serde 变体名双向逐字对齐，对齐断言见
// coding_workspace_engine/tests/parser_prompt/plan_defect_prompt.rs
// （枚举加变体/改名时该测试必红，教学不得与 schema 漂移）。
// 3.6 矩阵族④根因修复（route×target 矩阵）：现场 7 例三 provider 全中
// （route OperationalGate/VerificationRetry 配 target CurrentWorkItem、
// 大写枚举名 OperationalGate、DependencyGraphInvalid 误配 CurrentWorkItem），
// 根因是契约只教了 defect_class 8 取值、从未给出合法 route×target 组合，
// 弱模型恰一次教学重驱后仍重犯。本函数逐字枚举 8 行合法矩阵（defect_class →
// recommended_route → repair_target）并附反例错误原文；矩阵与校验器判决的
// 双向对齐断言（含大写枚举名/容器字段名反例）见同一测试文件的
// plan_defect_output_contract_route_target_matrix_aligns_with_validator。
pub fn plan_defect_structured_output_contract() -> &'static str {
    "\nPlan Defect structured output contract:\n\
     - Coder 仅在发现计划、Story、Design、依赖契约、验证或运行环境阻塞时输出 plan_defect_findings 数组；普通成功输出或普通 implementation defect 可省略该数组或使用空数组。\n\
     - CodeReviewer/InternalReviewer/GroupFinalReview 在 findings 中使用同一字段；普通 implementation defect 必须显式使用 defect_class=implementation_defect、recommended_route=coder_rework，reason_code=null、contract_refs=[]、capability_refs=[]、repair_target=null、confidence=null。\n\
     - 每个 plan defect finding 必须包含 finding_id、severity、defect_class、reason_code、message、evidence、contract_refs、capability_refs、repair_target、recommended_route、confidence。\n\
     - defect_class 只能逐字使用以下 8 个取值之一：implementation_defect、verification_incomplete、current_work_item_invalid、upstream_contract_invalid、dependency_graph_invalid、design_amendment_required、story_amendment_required、operational_blocker；禁止发明、合并或概括取值（反例：defect_class=plan_defect → plan_defect_finding_invalid 拒绝并阻塞流程）。\n\
     - severity 只能使用 error、warning；阻塞问题使用 severity=error，不得使用 blocking、blocker 等其他取值。\n\
     - confidence 只能使用 low、medium、high；不得使用 0~1 的小数或百分比。\n\
     - repair_target 必须是对象，包含 kind（current_work_item、upstream_work_item 或 subgraph）、logical_work_item_ids、work_item_revision_ids；没有明确修复目标时使用 repair_target=null，不得使用字符串。\n\
     - recommended_route 与 repair_target 必须逐字使用以下合法矩阵（defect_class → recommended_route → repair_target，8 行之外无合法组合；取值一律 snake_case 小写，禁止大写枚举名或容器字段名当取值——反例：recommended_route=OperationalGate 或 plan_defect_findings → unknown variant 拒绝，错误回显里的 route OperationalGate 是 Rust 变体名，JSON 必须写 operational_gate）：implementation_defect → coder_rework → repair_target=null；verification_incomplete → verification_retry → repair_target=null；current_work_item_invalid → plan_repair → repair_target.kind=current_work_item；upstream_contract_invalid → plan_repair → repair_target.kind=upstream_work_item；dependency_graph_invalid → plan_repair → repair_target.kind=subgraph；design_amendment_required → design_amendment → repair_target=null；story_amendment_required → story_amendment → repair_target=null；operational_blocker → operational_gate → repair_target=null。\n\
     - 只有 current_work_item_invalid、upstream_contract_invalid、dependency_graph_invalid 三个 defect_class 携带非空 repair_target（kind 依次为 current_work_item、upstream_work_item、subgraph，且 logical_work_item_ids 与 work_item_revision_ids 都必须非空）；其余五个 defect_class 的 repair_target 必须为 null；无 target 路线硬塞 target、kind 错配或省略、id 列表为空都会被拒绝（反例：recommended_route=operational_gate 配 repair_target.kind=current_work_item → InvalidRepairTarget(\"route OperationalGate does not accept target CurrentWorkItem\")；recommended_route=verification_retry 配 repair_target.kind=current_work_item → InvalidRepairTarget(\"route VerificationRetry does not accept target CurrentWorkItem\")；defect_class=dependency_graph_invalid 配 repair_target.kind=current_work_item → InvalidRepairTarget(\"defect class DependencyGraphInvalid requires target Subgraph, got CurrentWorkItem\")；defect_class=current_work_item_invalid 配 repair_target.kind=upstream_work_item → InvalidRepairTarget(\"defect class CurrentWorkItemInvalid requires target CurrentWorkItem, got UpstreamWorkItem\")；defect_class=dependency_graph_invalid 省略 repair_target → InvalidRepairTarget(\"defect class DependencyGraphInvalid requires target Subgraph\")；logical_work_item_ids 或 work_item_revision_ids 为空 → InvalidRepairTarget(\"plan repair target requires logical work item and revision ids\")）。\n\
     - recommended_route 不得使用 human_triage（serde 合法但 finding 不可输出；反例：defect_class=implementation_defect 配 recommended_route=human_triage → InvalidFinding(\"defect class ImplementationDefect requires route CoderRework, got HumanTriage\")）。\n\
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
