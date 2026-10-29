use serde::{Deserialize, Serialize};
use sha2::Digest;

/// P0 1.3（REQ-WIGA-05）Task 10：无 driver 的人工门命令族。`expected_gate_id`
/// 与服务端当前 active gate/timeline node 比对；`command_id` 为幂等/审计键
/// （与 WS HumanGateFeedback/AbandonHumanGate 同族）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HumanActionRequest {
    Approve {
        command_id: String,
        expected_gate_id: String,
    },
    Abandon {
        command_id: String,
        expected_gate_id: String,
    },
    Feedback {
        command_id: String,
        expected_gate_id: String,
        feedback: String,
    },
    CompileRecovery {
        command_id: String,
        expected_gate_id: String,
        action: crate::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto,
        reason: Option<String>,
    },
    /// C1 Task 4（REQ-C1-GATE-01/02）：孤儿候选门恢复/重建动作（唯一
    /// additive variant，不新增第二 candidate recovery 路由）。
    CandidateRecovery {
        command_id: String,
        expected_gate_id: String,
        action: CandidateRecoveryActionDto,
    },
}

/// C1 Task 4：恢复动作 wire 枚举（`recover`＝恢复原门；`rebuild`＝从权威
/// source/IR/report 重建呈现面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateRecoveryActionDto {
    Recover,
    Rebuild,
}

impl From<CandidateRecoveryActionDto>
    for crate::product::work_item_plan_policy::CandidateRecoveryAction
{
    fn from(value: CandidateRecoveryActionDto) -> Self {
        match value {
            CandidateRecoveryActionDto::Recover => Self::Recover,
            CandidateRecoveryActionDto::Rebuild => Self::Rebuild,
        }
    }
}

/// C1 Task 4（REQ-C1-GATE-02）：候选门恢复结果（`CandidateRecovery` 变体
/// 的专用响应体）。`state` 复用统一 `OperationState`（accepted/replayed/
/// needs_human/rejected）；`complete=false` 时 `missing` 列出缺失事实。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CandidateRecoveryResult {
    pub command_id: String,
    pub gate_id: String,
    pub state: crate::product::models::OperationState,
    pub complete: bool,
    pub missing: Vec<String>,
    pub completed_steps: Vec<String>,
}

/// 人工命令回执状态（snake_case wire）：Accepted=已受理（含幂等重放与
/// AlreadyClosed）；Busy=门/轮次占用（409）；Rejected=引擎语义拒绝（422，
/// 细节经 ApiError code/message 上抛）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanActionState {
    Accepted,
    Busy,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HumanActionStatus {
    pub command_id: String,
    pub state: HumanActionState,
    pub gate_id: String,
}

/// P2 GAP-E/G（Task 0.1）：人工显式重驱失败 SingleCandidate reviewer 运行。
/// `expected_phase` 必须与 durable 当前相位一致（当前仅受理 `failed`）；
/// `command_id` 与路径中的 `failed_node_id` 组成幂等/审计键。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetryFailedScRunRequest {
    pub command_id: String,
    pub expected_phase: crate::product::models::SingleCandidatePhase,
}

/// 受理回执：accepted=首次受理；replayed=同键重放读原结果；
/// needs_human=派发副作用不明，人工分诊（不自动重试）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RetryFailedScRunStatus {
    pub command_id: String,
    pub failed_node_id: String,
    pub state: String,
}

/// 批 2 P4（REQ-WIGA-10）：plan 阶段失败继续请求。`expected_binding` 复用
/// durable 绑定身份（与 enrollment 当前代全等才可继续）；`expected_stop_key`
/// 取服务端当前停点（失败节点/人工门/生成检查点派生键），防止把旧说明
/// 送给新停点；`context` 为对话窗口补充说明（provider 失败继续可空、
/// 评审超限继续必须非空）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationContinuePlanRequest {
    pub command_id: String,
    pub expected_binding: crate::product::models::automation::EnrollmentBindingIdentity,
    pub expected_stop_key: String,
    #[serde(default)]
    pub context: Option<String>,
}

impl AutomationContinuePlanRequest {
    /// 稳定 payload 摘要（命令账本按此判同 command 异 payload，fail-closed）。
    pub fn payload_digest(&self) -> String {
        let payload =
            serde_json::to_string(self).expect("continue-plan request payload is serializable");
        format!("sha256:{:x}", sha2::Sha256::digest(payload.as_bytes()))
    }
}

/// 批 2 P4（REQ-WIGA-10）：继续受理回执。accepted=首次受理并推进；
/// replayed=同键同负载重放读首次结果（不重复写 note/派发）；
/// needs_human=派发副作用不明或收尾写入失败，人工分诊（不自动重试）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationContinuePlanStatus {
    pub command_id: String,
    pub state: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

/// 批 2 P4（REQ-WIGA-10）：只读 control 管理投影。按 durable
/// session/门/生成检查点及 N0 持久事实判定；初版 allowed_actions 只含
/// continue_plan 与既有门动作，不含 takeover；GET 不写任何事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationControlStatus {
    pub managed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<crate::product::models::automation::EnrollmentBindingIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_key: Option<String>,
    pub stage: String,
    pub allowed_actions: Vec<String>,
    pub context_required: bool,
}
