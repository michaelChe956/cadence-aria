use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;
use crate::product::workspace_engine::LinkedWorkspaceAmendmentTarget;

use super::common::{ChoiceAnswer, ProviderConfigSnapshot, StructuredFeedback};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelloRole {
    Driver,
    Observer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsInMessage {
    UserMessage {
        content: String,
    },
    ContextNote {
        content: String,
    },
    StartGeneration {
        provider_config: ProviderConfigSnapshot,
        reviewer_enabled: bool,
        /// B1b（Task 4.2，设计 §5.1）：显式 fresh 恢复意图。缺省 `None`
        /// 保持普通出生/重跑语义（既有客户端 wire 兼容）；携带时经
        /// socket 的 LC 恢复入站门（同 ID typed Replay / 前置拒收）与
        /// decisions 的 `claim_lc_fresh` CAS 受理。
        #[serde(default)]
        resume_recovery: Option<LcResumeRecoveryIntent>,
    },
    RetryInterruptedRun {
        failed_node_id: String,
    },
    Hello {
        session_id: String,
        last_seen_node_id: Option<String>,
        #[serde(default)]
        role: Option<HelloRole>,
        #[serde(default)]
        after_event_seq: Option<u64>,
    },
    Rollback {
        checkpoint_id: String,
    },
    Confirm,
    ProviderSelect {
        role: String,
        provider: ProviderName,
    },
    PermissionResponse {
        id: String,
        approved: bool,
        reason: Option<String>,
    },
    ChoiceResponse {
        id: String,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
        #[serde(default)]
        answers: Vec<ChoiceAnswer>,
        /// P0 1.3：claim 门面字段；缺省时 WS 层绑定当前唯一 run 并生成新
        /// command_id（与 REST 同一门面仲裁）。
        #[serde(default)]
        command_id: Option<String>,
        #[serde(default)]
        expected_run_id: Option<String>,
    },
    RequestRevision {
        feedback: StructuredFeedback,
    },
    WorkItemPlanCompileRecoveryAction {
        action: WorkItemPlanCompileRecoveryActionDto,
        reason: Option<String>,
    },
    ConfirmPlanAmendment {
        amendment_id: String,
    },
    CancelPlanAmendment {
        amendment_id: String,
        reason: Option<String>,
    },
    StartLinkedWorkspaceAmendment {
        target: LinkedWorkspaceAmendmentTarget,
    },
    HumanGateFeedback {
        command_id: String,
        feedback: String,
    },
    Advance {
        command_id: String,
    },
    AbandonHumanGate {
        command_id: String,
    },
    Abort,
    /// trust 指引卡「登记并重试」按钮（owner 2026-10-12 直批）：用户显式
    /// 授权按需登记所选 provider 的用户级 workspace trust。root 由服务端
    /// 解析（LC 冻结根/单仓 repository 根，与确认链同一 authority），
    /// **不接受客户端传路径**；处理复用现有 ensure 登记链（幂等 +
    /// ensure 审计）；成功回 `trust_entry_registered`，不自动重新生成。
    RegisterTrustEntry {
        provider: ProviderName,
    },
    Ping,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceInboundEnvelope {
    pub(crate) message: WsInMessage,
    pub(crate) submitted_fields: BTreeSet<String>,
}

impl From<WsInMessage> for WorkspaceInboundEnvelope {
    fn from(message: WsInMessage) -> Self {
        Self {
            message,
            submitted_fields: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanCompileRecoveryActionDto {
    Continue,
    AbortAndRollback,
    HumanTriage,
}

/// B1b（Task 4.2，接口字典）：`StartGeneration.resume_recovery` 的显式
/// fresh 意图——`recovery_id` 为 durable 恢复权威身份，`operation_id` 为
/// 客户端生成的幂等/审计键（空/空白在 claim 前被 typed
/// `INVALID_COMMAND_ID` 拒收）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LcResumeRecoveryIntent {
    pub recovery_id: String,
    pub intent: LcResumeRecoveryIntentKind,
    pub operation_id: String,
}

/// 恢复意图种类：当前仅 `Fresh`（显式新 native 会话，不复活旧 ID）；
/// 未知值 serde 拒收，不猜测回落。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LcResumeRecoveryIntentKind {
    Fresh,
}
