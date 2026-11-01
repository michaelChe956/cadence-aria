use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

impl ReviewFindingSeverity {
    /// wire 词表（finding_key 身份成分与呈现共用，不另造第二口径）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blocking => "blocking",
            Self::MustFix => "must_fix",
            Self::Suggestion => "suggestion",
        }
    }
}

/// REQ-WSC-12（owner 2026-10-30 裁定，取舍 1=B）：finding 的处置归属。
/// `Actionable`＝本 Work Item 可通过修订自身收敛；`Advisory`＝只作提示
/// （其它文档的当前状态、产品侧规范/流程待办、与本 Work Item 无关的外部
/// 事实）。平台只把 `Actionable` 计入阻断/返修/门卡必处理计数——模型标错
/// 也不会让 advisory 卡住流程。
///
/// 缺失语义＝`Actionable`（fail-safe：宁可多阻断不可漏阻断）；历史记录
/// （wire 无该字段）因此保持逐字兼容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingDisposition {
    Actionable,
    Advisory,
}

impl ReviewFindingDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Actionable => "actionable",
            Self::Advisory => "advisory",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "actionable" => Some(Self::Actionable),
            "advisory" => Some(Self::Advisory),
            _ => None,
        }
    }
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

/// REQ-WSC-11（owner 可读的人工裁决）：`needs_human` 结论面向 owner 的
/// 单条互斥选项——`label` 是选项名（人话），`consequence` 是「选它之后会
/// 发生什么」。与 `owner_question` 一起构成 owner 一句话即可回答的契约。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerOption {
    pub label: String,
    pub consequence: String,
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
    /// REQ-WSC-11：面向 owner 的一句话人话提问（仅 `needs_human` 结论必须
    /// 携带）。`needs_human` 缺失时服务端判结构化输出不合规并走既有诊断
    /// 通道；`pass`/`revise` 恒 None（skip 保持旧 wire 字节不变）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_question: Option<String>,
    /// REQ-WSC-11：owner 可选的互斥处置项（2–3 项）；每项含选项名与后果。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_options: Option<Vec<OwnerOption>>,
    /// REQ-WSC-12：处置归属。`None`（含全部历史记录）语义＝`actionable`，
    /// fail-safe。历史记录（wire 无该字段）因此保持逐字兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<ReviewFindingDisposition>,
    /// REQ-WSC-12：能指到具体 Work Item 时填写；指不到留空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_work_item_id: Option<String>,
    /// REQ-WSC-13：平台计算的 finding 内容指纹（sha256(severity␁message)
    /// 前 16 位小写 hex）——owner「排除此项」与呈现共用的稳定身份。模型
    /// MUST NOT 生成该值；live 与 rebuild 两条投影路径由同一函数算出。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_key: Option<String>,
}

impl ReviewFinding {
    /// REQ-WSC-13 的身份算法（唯一实现）：`sha256(severity + "\u{1}" + message)`
    /// 取前 16 位小写 hex。live 解析与 rebuild 归一化都调用本函数，保证同一
    /// 份 findings 两条路径算出同一个 key。
    pub fn finding_key_for(severity: &ReviewFindingSeverity, message: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(severity.as_str().as_bytes());
        hasher.update([0x1u8]);
        hasher.update(message.as_bytes());
        hex::encode(hasher.finalize())[..16].to_string()
    }

    /// 本 finding 的平台身份（按内容现算，不信任持久化值）。
    pub fn computed_finding_key(&self) -> String {
        Self::finding_key_for(&self.severity, &self.message)
    }

    /// REQ-WSC-13：投影点补全 finding 身份（durable 记录不写该字段，保持存量
    /// 字节；rebuild 走同一算法，两条路径逐字同 key）。
    pub fn with_computed_finding_key(mut self) -> Self {
        if self.finding_key.is_none() {
            self.finding_key = Some(self.computed_finding_key());
        }
        self
    }
    /// 缺失＝`actionable`（fail-safe：宁可多阻断不可漏阻断）。
    pub fn disposition_or_fail_safe(&self) -> ReviewFindingDisposition {
        self.disposition
            .unwrap_or(ReviewFindingDisposition::Actionable)
    }

    /// REQ-WSC-12 平台硬规则：只有 `actionable` 计入阻断/返修/必关闭清单。
    pub fn is_actionable(&self) -> bool {
        self.disposition_or_fail_safe() == ReviewFindingDisposition::Actionable
    }

    /// P0-1 加固（REQ-WSC-12 平台硬规则）：`advisory` 不得成为「本 Work Item
    /// 真缺口」的后门。`contract_gap` / `self_contradiction` 都是本 WI 可通过
    /// 修订自身收敛的契约类缺口——模型把这类 finding 标成 advisory 时平台
    /// 按 `actionable` 处理（判定在平台侧，不依赖模型自觉标对）。
    ///
    /// owner 的显式排除仍然优先：会话级归一（`ReviewVerdict` 的 owner 处置
    /// 投影）在此之后把同一 finding 改回 advisory，按 owner 事实裁决。
    pub fn with_platform_disposition_rules(mut self) -> Self {
        if matches!(self.disposition, Some(ReviewFindingDisposition::Advisory))
            && matches!(
                self.category,
                Some(ReviewFindingCategory::ContractGap | ReviewFindingCategory::SelfContradiction)
            )
        {
            self.disposition = Some(ReviewFindingDisposition::Actionable);
        }
        self
    }
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

impl ReviewVerdict {
    /// REQ-WSC-13：投影点补全 finding 身份（durable 记录不写该字段，保持存量
    /// 字节；rebuild 走同一算法，两条路径逐字同 key）。
    pub fn with_computed_finding_keys(&self) -> Vec<ReviewFinding> {
        self.findings
            .iter()
            .cloned()
            .map(ReviewFinding::with_computed_finding_key)
            .collect()
    }
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
