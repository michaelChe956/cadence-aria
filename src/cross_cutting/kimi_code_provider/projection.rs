//! Kimi Code 的 LC 权限投影(Task 4c,REQ-LCG-03/06)。
//!
//! `KimiPolicyProjector` 实现 1a 冻结的 `ProviderPolicyProjector`:消费
//! gateway 构造的 `ProviderProjectionInput`,产出 21 个冻结字段全量填充的
//! `ProviderPolicyProjection`。与 4a/4b(Claude/Pi)的差异:
//! - Kimi 通用 tool_policy 恒为 `None`:非空通用策略直接拒绝(Global
//!   Constraints「Kimi 拒绝非空通用策略,继续既有 ClientServicePolicy」,
//!   稳定码 `provider_generic_tool_policy_forbidden`;argv 冻结 `acp`,
//!   无任何策略物理片段)。
//! - 读写面分离(Task 4c 核心):host root(canonical LC root)保持 ro,
//!   target root rw;fs read/write 与 terminal 宿主 handler 消费同一
//!   不可伪造 target boundary plan(见 `client_services`),terminal 实际
//!   cwd 不改变 provider 进程 cwd(ACP cwd 保持 root)。
//! - `mcp_bundle_digest` 只对应 Aria 注入 bundle;无注入时以
//!   `KIMI_NATIVE_MCP_SOURCE` 单独标记 native 项目配置来源,不冒充 Aria
//!   bundle digest。
//! digest 语义遵循计划「证据摘要分层」:`capability_projection_digest`
//! 不随单次 role/target/config 变化;`projection_digest` 任一漂移即变。

use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjection, ProviderPolicyProjector, ProviderProjectionError,
    ProviderProjectionInput,
};

/// LC 会话 approval 投影:kimi 经 ACP `session/request_permission` 选项
/// 带内审批(与 direct 路径同源;投影文本描述同一审批通道)。
pub const KIMI_LC_APPROVAL_POLICY: &str = "acp-session-permission-options";

/// 无 Aria 注入时的 MCP 来源标记(Task 4 Interfaces:`mcp_bundle_digest`
/// 只对应 Aria 注入,native 项目配置单独标来源——不冒充 bundle digest)。
pub const KIMI_NATIVE_MCP_SOURCE: &str = "native-project-config";

/// Kimi 拒绝非空通用策略的稳定码(Global Constraints;与 Task 7 guard
/// 语义同源,实际拒绝由本目录 `start_validated`/projector 实现)。
pub const KIMI_GENERIC_TOOL_POLICY_FORBIDDEN: &str = "provider_generic_tool_policy_forbidden";

/// Kimi LC projector:构造时冻结已实测 exact version。真实 version 未知
/// (空串)时 `project` fail-closed 拒绝,不产出投影。
#[derive(Debug, Clone)]
pub struct KimiPolicyProjector {
    exact_version: String,
}

impl KimiPolicyProjector {
    /// 以已解析的 provider exact version 构造 projector(registry 装配/测试
    /// seam;version 解析本身由 adapter 侧 CLI 探测承担)。
    pub fn new(exact_version: impl Into<String>) -> Self {
        Self {
            exact_version: exact_version.into(),
        }
    }
}

impl ProviderPolicyProjector for KimiPolicyProjector {
    fn project(
        &self,
        _input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        // Task 4c 阶段 1 RED 桩:真实投影在阶段 2 交付。
        Err(ProviderProjectionError::Unsupported(
            "kimi lc projection is not implemented yet (task 4c red stub)".to_string(),
        ))
    }
}
