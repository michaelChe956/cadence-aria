use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;

use super::execution::{CodingExecutionStage, CodingProviderRole};
use super::provider_config::CodingRoleProviderConfigSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingStageGateStatus {
    Open,
    Confirmed,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingStageGateState {
    pub gate_id: String,
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub expires_at: String,
    pub provider_snapshot: CodingRoleProviderConfigSnapshot,
    pub status: CodingStageGateStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChoiceOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingChoiceGateStatus {
    Open,
    Resolved,
    Stale,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChoiceGateResponse {
    pub selected_option_ids: Vec<String>,
    #[serde(default)]
    pub free_text: Option<String>,
    pub responded_at: String,
    /// P0 1.3（REQ-WIGA-05）：完整逐题答案（Task 6 `answers` 原样落盘）；
    /// 旧单题 gate 仅有 selected/free_text，serde 缺省为空。
    #[serde(default)]
    pub answers: Vec<crate::cross_cutting::streaming_provider::ChoiceAnswerData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChoiceGate {
    pub gate_id: String,
    pub choice_id: String,
    pub attempt_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub provider: ProviderName,
    pub source: String,
    pub prompt: String,
    #[serde(default)]
    pub options: Vec<CodingChoiceOption>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
    pub status: CodingChoiceGateStatus,
    #[serde(default)]
    pub response: Option<CodingChoiceGateResponse>,
    /// P0 1.3：多问题透传（Task 6 完整 questions）；旧 gate serde 缺省空，
    /// `effective_questions` 按单一 default 题兼容投影。
    #[serde(default)]
    pub questions: Vec<CodingChoiceQuestion>,
    /// P0 1.3（REQ-WIGA-05）Task 11：应答须绑定的 run 化身——durable 记录
    /// 不落盘（handler 层按 active run 注入，REST 作答端到端可见）；无活跃
    /// run 时缺省 None，消费者不得猜 run。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_run_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// P0 1.3：coding choice 逐题结构（与 provider `ChoiceQuestionData` 同构）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingChoiceQuestion {
    pub id: String,
    pub prompt: String,
    pub options: Vec<CodingChoiceOption>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGateAction {
    pub action_id: String,
    pub label: String,
    pub action_type: CodingGateActionType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGateActionType {
    SendToCoder,
    ConfirmStage,
    AcceptRisk,
    Abort,
    RetryPush,
    ManualFix,
    ProvideContext,
    ManualContinue,
    RetryCoding,
    RetryCodingWithContext,
    RetryReview,
    RetryInternalReview,
    RetryGroupReviewShard,
    RetryGroupReduction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGateKind {
    Permission,
    StageGate,
    Blocked,
    FinalConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGateRequired {
    pub gate_id: String,
    pub kind: CodingGateKind,
    pub title: String,
    pub description: String,
    pub stage: Option<CodingExecutionStage>,
    pub role: Option<CodingProviderRole>,
    pub expires_at: Option<String>,
    pub provider_snapshot: Option<CodingRoleProviderConfigSnapshot>,
    pub available_actions: Vec<CodingGateAction>,
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
    #[serde(default)]
    pub diagnostic: Option<CodingGateDiagnostic>,
    /// P2-c（REQ-CRO-09）：分诊卡「大白话」摘要投影。additive：旧 gate JSON
    /// 反序列化为 `None`（前端缺省回落 `description`）；经
    /// `CodingAttemptStore::attach_blocked_gate_triage_summary` 独立入口落盘
    ///（C1 裁定：不给 `CreateBlockedGateInput` 加字段）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage_summary: Option<CodingTriageSummary>,
}

/// P2-c（REQ-CRO-09）：分诊卡三段大白话——「AI 遇到什么问题／做了什么／
/// 没能验证什么」。固定模板 + 结构化字段（`build_coder_triage_summary`
/// 宽容提取），不引入 LLM 翻译。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CodingTriageSummary {
    /// AI 遇到的问题（首条 finding.message，截断）。解析失败时为固定中文说明。
    pub problem: String,
    /// 全部 finding 的 (reason_code, message) 摘要；最多 5 条。
    #[serde(default)]
    pub findings: Vec<CodingTriageFinding>,
    /// AI 做了什么：取自 delivery_report（宽容解析，缺失/类型错为 None）。
    #[serde(default)]
    pub did: Option<CodingTriageDid>,
    /// 没能验证什么：finding.evidence[*].message 去重；最多 5 条。
    #[serde(default)]
    pub unverified: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CodingTriageFinding {
    pub reason_code: String,
    pub message: String,
}

/// delivery_report 的宽容投影：`changed_files` 最多 20 项；
/// `commit_sha`/`verification_exit_code`/`environment_note` 缺失为 None。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CodingTriageDid {
    pub changed_files: Vec<String>,
    pub commit_sha: Option<String>,
    pub verification_exit_code: Option<i64>,
    pub environment_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CodingGateDiagnostic {
    #[serde(default)]
    pub actual_value: Option<String>,
    #[serde(default)]
    pub limit: Option<String>,
    pub phase: String,
    pub run_failure_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityGateBypassAudit {
    pub id: String,
    pub attempt_id: String,
    pub gate_id: String,
    pub stage: CodingExecutionStage,
    #[serde(default)]
    pub reason_code: Option<String>,
    pub operator_context: String,
    pub created_at: String,
}

impl CodingChoiceGate {
    /// 旧 gate（无 questions）按单一 default 题兼容：题干/gate 级选项透传，
    /// 不猜测多题结构；新 gate 原样返回。
    pub fn effective_questions(&self) -> Vec<CodingChoiceQuestion> {
        if !self.questions.is_empty() {
            return self.questions.clone();
        }
        vec![CodingChoiceQuestion {
            id: "default".to_string(),
            prompt: self.prompt.clone(),
            options: self.options.clone(),
            allow_multiple: self.allow_multiple,
            allow_free_text: self.allow_free_text,
        }]
    }
}
