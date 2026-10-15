//! P4-a（REQ-PTR-IDLE，块4 挂起检测 A5）：coding 共享 provider stream 的
//! 零活动挂起看门狗——阈值、生效界与活动判定。
//!
//! - **C3 裁定：不造假时钟**（tokio 未启用 `test-util`）：阈值经
//!   `ARIA_CODING_*_IDLE_SECS` 环境变量覆写（先例
//!   `tests/group_review_budget.rs` 的 `ShardConcurrencyEnvironment`），
//!   每次 provider 调用启动时一次读取（并行测试下的安全读点），不每事件
//!   读 env。
//! - **C4 裁定：范式复用** `src/product/workspace_engine/provider_drive/
//!   watchdog.rs`（`sleep_until` + `tokio::pin!` + `select!` 条件臂、真实
//!   活动重置、choice/permission 人工等待暂停、耗尽收口），不重造。
//! - 触发语义（设计 §3.2）：首次挂起由 retry-cycle 同 provider fresh 重试
//!   一次；再次挂起经 `open_provider_stalled_gate` 转 attempt Blocked＋
//!   stalled 门（通知 kind=stalled 由 P4-c/P4-d 按 reason_code 消费）。

use std::time::Duration;

use crate::cross_cutting::streaming_provider::ProviderEvent;
use crate::product::coding_models::CodingProviderRole;

/// idle 挂起的稳定诊断原因码（写入既有 Timeout 事件的 payload 与错误面）。
pub(crate) const PROVIDER_IDLE_STALL_REASON_CODE: &str = "provider_idle_stall";
/// coder 挂起耗尽门 reason_code（块2 CoderTriageCard 三按钮复用面）。
pub(crate) const CODER_PROVIDER_STALLED_REASON_CODE: &str = "coder_provider_stalled";
/// reviewer（CodeReview/InternalPrReview）挂起耗尽门 reason_code。
pub(crate) const CODE_REVIEW_PROVIDER_STALLED_REASON_CODE: &str = "code_review_provider_stalled";

pub(crate) const ARIA_CODING_REVIEWER_IDLE_SECS_ENV: &str = "ARIA_CODING_REVIEWER_IDLE_SECS";
pub(crate) const ARIA_CODING_CODER_IDLE_SECS_ENV: &str = "ARIA_CODING_CODER_IDLE_SECS";
pub(crate) const ARIA_CODING_TOOL_INFLIGHT_IDLE_SECS_ENV: &str =
    "ARIA_CODING_TOOL_INFLIGHT_IDLE_SECS";

/// 块4 D-3（owner 裁定 A）：思考静默 reviewer 5 分钟 / coder 10 分钟；
/// 工具 call 未返回时静默窗口放宽到 30 分钟（减少打断长构建）。
pub(crate) const DEFAULT_REVIEWER_IDLE_SECS: u64 = 300;
pub(crate) const DEFAULT_CODER_IDLE_SECS: u64 = 600;
pub(crate) const DEFAULT_TOOL_INFLIGHT_IDLE_SECS: u64 = 1800;

/// 一次 provider 调用的 idle 界快照（调用启动时读一次）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProviderIdleLimits {
    /// 角色思考静默界（reviewer 300s / coder 600s）。
    pub(crate) role: Duration,
    /// 工具 call 未返回时的静默界（默认 1800s）。
    pub(crate) tool_inflight: Duration,
}

impl ProviderIdleLimits {
    pub(crate) fn for_role(role: &CodingProviderRole) -> Self {
        let (role_env, default_role_secs) = match role {
            CodingProviderRole::Coder => (ARIA_CODING_CODER_IDLE_SECS_ENV, DEFAULT_CODER_IDLE_SECS),
            CodingProviderRole::CodeReviewer | CodingProviderRole::InternalReviewer => (
                ARIA_CODING_REVIEWER_IDLE_SECS_ENV,
                DEFAULT_REVIEWER_IDLE_SECS,
            ),
        };
        Self {
            role: Duration::from_secs(env_positive_secs(role_env, default_role_secs)),
            tool_inflight: Duration::from_secs(env_positive_secs(
                ARIA_CODING_TOOL_INFLIGHT_IDLE_SECS_ENV,
                DEFAULT_TOOL_INFLIGHT_IDLE_SECS,
            )),
        }
    }
}

fn env_positive_secs(key: &str, default_secs: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(default_secs)
}

/// 工具 call 未返回时取 `max(角色界, 工具界)`（D-3 裁定 A）；无在途工具
/// call 用角色界。是「距最后真实活动的静默」界，不是命令总耗时。
pub(crate) fn effective_idle_limit(
    limits: ProviderIdleLimits,
    open_tool_calls: usize,
) -> Duration {
    if open_tool_calls > 0 {
        limits.role.max(limits.tool_inflight)
    } else {
        limits.role
    }
}

/// 真实 provider 活动才重置 idle 计时（设计 §3.2）：TextDelta / Execution
/// （含 `pi_empty_output_retry`——adapter 侧重发 prompt 前落的 Execution，
/// 是真实活动不是 idle 残留）/ ToolCall / ToolResult / StatusChanged /
/// PermissionRequest / ChoiceRequest；观测性事件（UsageReport 用量心跳、
/// ToolPolicy 审计出口）不算。Completed/Failed/ProtocolError/
/// PermissionTimeout 为终态事件，直接返回不进入计时循环。
pub(crate) fn provider_event_is_idle_activity(event: &ProviderEvent) -> bool {
    !matches!(
        event,
        ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_)
    )
}
