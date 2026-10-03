//! Codex 的 LC 受限 sandbox 投影(Task 5,REQ-LCG-04)。
//!
//! `CodexPolicyProjector` 实现 1a 冻结的 `ProviderPolicyProjector`:消费
//! gateway 构造的 `ProviderProjectionInput`,产出 21 个冻结字段全量填充的
//! `ProviderPolicyProjection`,并派生 codex 专属的 `CodexSandboxProjection`
//! (双 cwd 合同:process cwd=canonical root,protocol cwd=action 冻结面)。
//!
//! REQ-LCG-04 冻结语义:
//! - `CodexSandboxMode::{ReadOnly,WorkspaceWrite}`——无 DangerFullAccess 变体;
//!   danger-full-access 永久拒绝(稳定码 `codex_danger_full_access_unsupported`,
//!   与 gateway 路由门同源),LC 启动绝不回退 direct Coder 危险映射。
//! - Planning/Review 固定 `read-only` + `on-request`;Coding 固定
//!   `workspace-write`,approval=Auto→`never`/Supervised→`on-request`。
//! - Coding 写面必须 target-only(恰一个等于 target 的可写 root);受限投影
//!   无法成立时唯一替代是 danger-full-access,以稳定码拒绝——拒绝发生在
//!   `ProcessManager::spawn` 之前,零 child。
//!
//! digest 分层同三家先例(Task 4):`capability_projection_digest` 为该
//! provider+version+action 的完整权限 profile 摘要(不随单次 role/target
//! 变化);`projection_digest` 为含当前 role/cwd/target/roots/trust/config
//! 的会话全投影摘要;digest 输入=固定字段序+长度分隔+schema 前缀,禁止
//! Debug 文本与递归哈希。

use std::path::PathBuf;

use crate::cross_cutting::provider_boundary::ProviderBoundaryPlan;
use crate::product::logical_codebase::policy::{SessionPolicyAction, SessionPolicyEnvelope};
use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjection, ProviderPolicyProjector, ProviderProjectionError,
    ProviderProjectionInput,
};

/// LC 只读 action(Planning/Review)的冻结 sandbox wire 文本。
pub const CODEX_LC_SANDBOX_READ_ONLY: &str = "read-only";
/// LC Coding 的冻结 sandbox wire 文本(REQ-LCG-04:workspace-write)。
pub const CODEX_LC_SANDBOX_WORKSPACE_WRITE: &str = "workspace-write";
/// LC 只读 approval 冻结值(Planning/Review 恒 on-request)。
pub const CODEX_LC_APPROVAL_ON_REQUEST: &str = "on-request";
/// LC Coding Auto 档的 approval 冻结值。
pub const CODEX_LC_APPROVAL_NEVER: &str = "never";

/// codex 受限 sandbox 模式(REQ-LCG-04 冻结):只有只读与 target 写两种,
/// 无 DangerFullAccess 变体——danger-full-access 永久拒绝,不进枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexSandboxMode {
    ReadOnly,
    WorkspaceWrite,
}

impl CodexSandboxMode {
    /// 与 wire(`thread/start`/`thread/resume` params 的 `sandbox` 字段)
    /// 同源的冻结文本。
    pub fn wire_text(self) -> &'static str {
        match self {
            CodexSandboxMode::ReadOnly => CODEX_LC_SANDBOX_READ_ONLY,
            CodexSandboxMode::WorkspaceWrite => CODEX_LC_SANDBOX_WORKSPACE_WRITE,
        }
    }
}

/// codex 的 LC sandbox 投影(计划冻结接口逐字段):由 projector 从全投影
/// 派生,字段私有 + `pub(crate)` getter,调用方不能自造。双 cwd 合同:
/// `process_cwd`=canonical root(进程 cwd),`protocol_cwd`=协议 cwd
/// (Coding=target worktree,只读=root)。
#[derive(Debug, Clone)]
pub struct CodexSandboxProjection {
    mode: CodexSandboxMode,
    approval_policy: String,
    process_cwd: PathBuf,
    protocol_cwd: PathBuf,
    target_root: PathBuf,
    boundary_evidence_ref: String,
}

impl CodexSandboxProjection {
    /// crate 内构造(projector 单一来源)。
    pub(crate) fn new(
        mode: CodexSandboxMode,
        approval_policy: String,
        process_cwd: PathBuf,
        protocol_cwd: PathBuf,
        target_root: PathBuf,
        boundary_evidence_ref: String,
    ) -> Self {
        Self {
            mode,
            approval_policy,
            process_cwd,
            protocol_cwd,
            target_root,
            boundary_evidence_ref,
        }
    }

    pub(crate) fn mode(&self) -> CodexSandboxMode {
        self.mode
    }

    pub(crate) fn approval_policy(&self) -> &str {
        &self.approval_policy
    }

    pub(crate) fn process_cwd(&self) -> &std::path::Path {
        &self.process_cwd
    }

    pub(crate) fn protocol_cwd(&self) -> &std::path::Path {
        &self.protocol_cwd
    }

    pub(crate) fn target_root(&self) -> &std::path::Path {
        &self.target_root
    }

    pub(crate) fn boundary_evidence_ref(&self) -> &str {
        &self.boundary_evidence_ref
    }
}

/// Codex LC projector:构造时冻结已实测 exact version。真实 version 未知
/// (空串)时 `project` fail-closed 拒绝,不产出投影。
#[derive(Debug, Clone)]
pub struct CodexPolicyProjector {
    exact_version: String,
}

impl CodexPolicyProjector {
    /// 以已解析的 provider exact version 构造 projector(registry 装配/测试
    /// seam;version 解析本身由 adapter 侧 supplier/CLI 探测承担)。
    pub fn new(exact_version: impl Into<String>) -> Self {
        Self {
            exact_version: exact_version.into(),
        }
    }

    /// 由全投影派生 codex 受限 sandbox 投影(单一来源;wire 参数与
    /// `process_cwd`/`protocol_cwd` 消费同源)。
    ///
    /// Task 5a 阶段 1 RED 桩:阶段 2 实现真实派生。
    pub(crate) fn sandbox_projection(
        &self,
        _projection: &ProviderPolicyProjection,
    ) -> CodexSandboxProjection {
        CodexSandboxProjection::new(
            CodexSandboxMode::ReadOnly,
            String::new(),
            PathBuf::new(),
            PathBuf::new(),
            PathBuf::new(),
            String::new(),
        )
    }
}

/// `SessionPolicyAction` 的稳定文本(serde snake_case 序列化同形;审计
/// `lc_projection.action` 的单一来源)。
pub(crate) fn action_text(action: SessionPolicyAction) -> &'static str {
    match action {
        SessionPolicyAction::PlanningReadOnly => "planning_read_only",
        SessionPolicyAction::CodingTargetWrite => "coding_target_write",
        SessionPolicyAction::ReviewReadOnly => "review_read_only",
    }
}

/// 由 gateway 冻结的 envelope 派生不可伪造 boundary plan(Coding 恰一个可写
/// target;read-only action 无可写面)。`protected_roots` 的 launcher/probe
/// 语义归 Task 6a/6c,此处为空并不宣称物理隔离。
///
/// Task 5a 阶段 1 RED 桩:阶段 2 实现真实派生。
pub(crate) fn lc_boundary_plan(
    _envelope: &SessionPolicyEnvelope,
) -> Result<ProviderBoundaryPlan, ProviderProjectionError> {
    Err(ProviderProjectionError::Unsupported(
        "codex lc boundary plan is not implemented yet (task 5a red stub)".to_string(),
    ))
}

impl ProviderPolicyProjector for CodexPolicyProjector {
    fn project(
        &self,
        _input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        // Task 5a 阶段 1 RED 桩:真实受限投影在阶段 2 交付。
        Err(ProviderProjectionError::Unsupported(
            "codex lc projection is not implemented yet (task 5a red stub)".to_string(),
        ))
    }
}
