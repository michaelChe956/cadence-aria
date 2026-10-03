//! Claude Code 的 LC 权限投影(Task 4a,REQ-LCG-03/06)。
//!
//! `ClaudePolicyProjector` 实现 1a 冻结的 `ProviderPolicyProjector`:消费
//! gateway 构造的 `ProviderProjectionInput`,产出 21 个冻结字段全量填充的
//! `ProviderPolicyProjection`。digest 语义遵循计划「证据摘要分层」:
//! - `capability_projection_digest` = 该 provider+version+action 的完整权限
//!   profile 摘要(固定 role×policy 映射、deny token 序列、headless allowlist、
//!   approval/sandbox/MCP 控制规范),**不随单次 role/target/config 变化**;
//!   action row 的 `projection_digest` 与之同源。
//! - `projection_digest` = 当前会话全投影摘要(profile 摘要 + role/cwd/
//!   target/roots/trust/config/permission/实际 tool policy),任一漂移即变。
//! - digest 输入 = 固定字段序 + 长度分隔 + schema 前缀;禁止 Debug 文本与
//!   递归哈希(profile 摘要不含 evidence_ref/digest 自身)。

use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjection, ProviderPolicyProjector, ProviderProjectionError,
    ProviderProjectionInput,
};

/// LC headless 会话冻结的工具 allowlist:只列既有合法只读内建工具,不含
/// Bash/Edit/Write/NotebookEdit 等写通道,也不扩大到 MCP 写工具(计划
/// Task 4 Step 3:只列既有合法工具)。
pub const CLAUDE_LC_ALLOWED_TOOLS: &str = "Read,Glob,Grep,LS,WebFetch,WebSearch,TodoWrite";

/// LC 会话 approval 投影:与最终 argv 同源(stdio 权限提示桥,
/// `--permission-prompt-tool=stdio`)。
pub const CLAUDE_LC_APPROVAL_POLICY: &str = "permission-prompt-tool=stdio";

/// Claude LC projector:构造时冻结已实测 exact version。真实 version 未知
/// (空串)时 `project` fail-closed 拒绝,不产出投影。
#[derive(Debug, Clone)]
pub struct ClaudePolicyProjector {
    exact_version: String,
}

impl ClaudePolicyProjector {
    /// 以已解析的 provider exact version 构造 projector(registry 装配/测试
    /// seam;version 解析本身由 adapter 侧 supplier/CLI 探测承担)。
    pub fn new(exact_version: impl Into<String>) -> Self {
        Self {
            exact_version: exact_version.into(),
        }
    }
}

impl ProviderPolicyProjector for ClaudePolicyProjector {
    fn project(
        &self,
        _input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        // Task 4a 阶段 1 RED 桩:真实投影在阶段 2 交付。
        Err(ProviderProjectionError::Unsupported(
            "claude lc projection is not implemented yet (task 4a red stub)".to_string(),
        ))
    }
}
