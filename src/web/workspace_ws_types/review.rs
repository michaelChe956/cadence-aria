use serde::{Deserialize, Serialize};

use crate::product::work_item_plan_policy::{FindingClassHint, ReviewFindingCategory};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdictType {
    Pass,
    Revise,
    NeedsHuman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingSeverity {
    Blocking,
    MustFix,
    Suggestion,
}

/// SG7（plan v1.1 §0.5/§0.6.3）：机器修复 finding 的服务端只读状态。本包
/// 唯一生产值 `Offered`——服务端已证实 L1 且指令已显式配对到 finding；
/// 执行态（claim/交付/耗尽）单源沿 `StructuralGovernanceSnapshot` 的
/// grant/receipt 投影，不在本 DTO 复制第二状态机。后续 owner 需要新状态
/// 时按 wire 兼容纪律追加变体，不放宽封闭性。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineRepairStatus {
    Offered,
}

/// SG7（plan v1.1 §0.5）：机器修复 wire DTO——由服务端从已证 L1 的
/// `StructuralRepairInstruction` 纯投影（identity 字段逐字来自指令，
/// `instruction_ref` 为按 `instruction_digest` 内容寻址的确定性 ref）。
/// 只读呈现用，不构成任何入站许可/启动授权（REQ-SG-06A：采纳只是编辑器
/// 预填，自动链不依赖点击）。Raw provider 输入不接受本类型字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineRepairDto {
    pub repair_id: String,
    pub failure_id: String,
    pub source_hash: String,
    pub context_digest: String,
    pub status: MachineRepairStatus,
    pub instruction_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewFinding {
    pub severity: ReviewFindingSeverity,
    pub message: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub required_action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<ReviewFindingCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_hint: Option<FindingClassHint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_field: Option<String>,
    /// SG7（§0.6.3）：服务端已证 L1 且同 source/context 显式配对的机器
    /// 修复投影；旧记录/Raw provider 输出恒 None（skip 保持旧 JSON 字节
    /// 不变）。provider 伪造字段按封闭策略拒/丢，不建许可。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_repair: Option<MachineRepairDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredOutputDiagnostic {
    pub code: String,
    pub message: String,
    pub repair_attempted: bool,
    pub repair_succeeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewGate {
    RequiresRevision,
    UserConfirmAllowed,
    UserTriageRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewVerdict {
    Pass,
    Revise,
    ReviseBatch,
    NeedsHuman,
    PlanReopenRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewScope {
    Outline,
    Item,
    Batch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewAction {
    Continue,
    ReviseOutline,
    ReviseCurrentItem,
    ReviseBatch,
    HumanTriage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewGate {
    RequiresCurrentItemRevision,
    RequiresBatchRevision,
    RequiresPlanReopen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanReviewAffectedItem {
    pub outline_index: Option<u32>,
    pub target_outline_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanReviewComplete {
    pub verdict: WorkItemPlanReviewVerdict,
    pub review_scope: WorkItemPlanReviewScope,
    pub target_outline_id: Option<String>,
    pub generation_round_id: String,
    pub draft_id: Option<String>,
    pub batch_id: Option<String>,
    pub review_action: WorkItemPlanReviewAction,
    pub gates: Vec<WorkItemPlanReviewGate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affects_items: Vec<WorkItemPlanReviewAffectedItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewVerdict {
    pub verdict: ReviewVerdictType,
    pub comments: String,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
    #[serde(default = "default_review_gate")]
    pub review_gate: ReviewGate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_plan_review: Option<WorkItemPlanReviewComplete>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output_diagnostic: Option<StructuredOutputDiagnostic>,
}

fn default_review_gate() -> ReviewGate {
    ReviewGate::UserConfirmAllowed
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDecision {
    pub decision: String,
    pub extra_context: Option<String>,
    pub decided_at: String,
}

#[cfg(test)]
mod sg07_tests {
    use super::*;

    fn dto() -> MachineRepairDto {
        MachineRepairDto {
            repair_id: "structural-repair/0123456789abcdef/required_capability_missing#0"
                .to_string(),
            failure_id: "diag/ref-1#0".to_string(),
            source_hash: "a".repeat(64),
            context_digest: "b".repeat(64),
            status: MachineRepairStatus::Offered,
            instruction_ref: format!("structural-repair-instruction/{}", "c".repeat(64)),
        }
    }

    /// SG7 Step1 `sg_07_machine_repair_optional_literal_cutover`：旧 finding
    /// JSON 无 machine_repair 解析为 None 且再序列化不新增字段（旧 wire/
    /// immutable 记录字节不变）；服务端 Some 完整 roundtrip 六字段。
    #[test]
    fn sg_07_machine_repair_optional_literal_cutover() {
        let legacy = serde_json::json!({
            "severity": "must_fix",
            "message": "canonical gap",
            "evidence": "work_item=WI-002",
            "required_action": "补齐 capability",
            "category": "contract_gap",
            "class_hint": "repairable",
            "contract_field": "WI-002::contract.levels-api"
        });
        let finding: ReviewFinding =
            serde_json::from_value(legacy).expect("legacy finding JSON must parse");
        assert_eq!(finding.machine_repair, None);
        let reserialized = serde_json::to_value(&finding).expect("reserialize");
        assert!(
            reserialized.get("machine_repair").is_none(),
            "None must skip serialization to keep legacy bytes identical: {reserialized}"
        );

        let paired = ReviewFinding {
            machine_repair: Some(dto()),
            ..finding.clone()
        };
        let wire = serde_json::to_value(&paired).expect("wire json");
        let machine_repair = wire.get("machine_repair").expect("Some must serialize");
        assert_eq!(
            machine_repair.get("repair_id").and_then(|v| v.as_str()),
            Some("structural-repair/0123456789abcdef/required_capability_missing#0")
        );
        assert_eq!(
            machine_repair.get("failure_id").and_then(|v| v.as_str()),
            Some("diag/ref-1#0")
        );
        assert_eq!(
            machine_repair.get("source_hash").and_then(|v| v.as_str()),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(
            machine_repair
                .get("context_digest")
                .and_then(|v| v.as_str()),
            Some("b".repeat(64).as_str())
        );
        assert_eq!(
            machine_repair.get("status").and_then(|v| v.as_str()),
            Some("offered")
        );
        assert_eq!(
            machine_repair
                .get("instruction_ref")
                .and_then(|v| v.as_str()),
            Some(format!("structural-repair-instruction/{}", "c".repeat(64)).as_str())
        );
        let roundtrip: ReviewFinding = serde_json::from_value(wire).expect("roundtrip must parse");
        assert_eq!(roundtrip, paired);
    }

    /// SG7 Step1 `sg_07_wire_server_authority`：wire 面未知字段仍 deny——
    /// ReviewFinding 与 MachineRepairDto 都是封闭类型，伪造字段不能穿透。
    #[test]
    fn sg_07_wire_server_authority_denies_unknown_fields_on_wire_types() {
        let forged_finding = serde_json::json!({
            "severity": "must_fix",
            "message": "finding",
            "evidence": "",
            "required_action": "",
            "machine_repair": {
                "repair_id": "forged",
                "failure_id": "forged",
                "source_hash": "forged",
                "context_digest": "forged",
                "status": "offered",
                "instruction_ref": "forged",
                "execution_permission": "grant-me"
            }
        });
        let error = serde_json::from_value::<ReviewFinding>(forged_finding)
            .expect_err("unknown machine_repair subfield must be denied");
        assert!(
            error.to_string().contains("execution_permission"),
            "deny must name the forged field: {error}"
        );

        let forged_top_level = serde_json::json!({
            "severity": "must_fix",
            "message": "finding",
            "evidence": "",
            "required_action": "",
            "provider_repair_authority": "true"
        });
        assert!(
            serde_json::from_value::<ReviewFinding>(forged_top_level).is_err(),
            "unknown finding-level field must stay denied"
        );
    }

    /// SG7 Step1：snapshot 纯投影的 receipt/launch 摘要不携带 home/secret
    /// 类字段（SG7a 定案接口的 wire 级核验；旧 None 聚合不冒新权）。
    #[test]
    fn sg_07_wire_server_authority_snapshot_omits_home_and_secret() {
        use crate::product::models::workspace::StructuralGovernanceState;
        use crate::web::workspace_ws_types::out::project_structural_governance_snapshot;

        let state = StructuralGovernanceState::default();
        let snapshot = project_structural_governance_snapshot(&state, None, &[], None, None, None);
        assert_eq!(snapshot.current_grant, None);
        assert_eq!(snapshot.available_user_budget, 3);
        let json = serde_json::to_string(&snapshot).expect("snapshot json");
        for forbidden in ["home", "secret", "credential", "api_key"] {
            assert!(
                !json.to_lowercase().contains(forbidden),
                "snapshot must not carry {forbidden}: {json}"
            );
        }
    }
}
