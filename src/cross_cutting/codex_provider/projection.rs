//! Codex 的 LC 全相位 danger 投影(T12,REQ-LCG-04 改版)。
//!
//! `CodexPolicyProjector` 实现 1a 冻结的 `ProviderPolicyProjector`:消费
//! gateway 构造的 `ProviderProjectionInput`,产出 21 个冻结字段全量填充的
//! `ProviderPolicyProjection`,并派生 codex 专属的 `CodexSandboxProjection`
//! (双 cwd 合同:process cwd=canonical root,protocol cwd=action 冻结面)。
//!
//! REQ-LCG-04 改版语义(provider-desandbox-full-open):
//! - Planning/Review/Coding 全相位 wire 恒 `danger-full-access`——平台不
//!   投影 read-only/workspace-write,不永久拒绝有效全开档,旧
//!   `codex_danger_full_access_unsupported` 拒绝语义废止。
//! - approval 按权限模式独立映射(Auto→`never`/Supervised→`on-request`);
//!   Author 岗位纪律(tool policy=deny)恒 `on-request`,与 sandbox 档解耦。
//! - 逻辑 Coding 唯一 target 根约束是平台交付合同(`SessionPolicyEnvelope`
//!   构造层强制);投影层不再保留物理 target-only 形状拒绝,boundary plan
//!   描述逻辑交付面(非物理隔离声明)。
//!
//! digest 分层:`capability_projection_digest` 为该 provider+version+action
//! 的完整权限 profile 摘要(不随单次 role/target 变化);`projection_digest`
//! 为含当前 role/cwd/target/roots/trust/config 的会话全投影摘要;LC 摘要使用
//! 独立 canonical 序列——绝不复用 direct 的 `deny_file_write_builtins_tokens`
//! 冻结片段或 `canonical_tool_policy`/`tool_policy_digest` 函数。

use std::path::PathBuf;

use sha2::{Digest as ShaDigest, Sha256};

use crate::cross_cutting::provider_boundary::{ProviderBoundaryMode, ProviderBoundaryPlan};
use crate::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderToolPolicy, ToolPolicyIntent, adapter_role_text,
};
use crate::product::logical_codebase::policy::{
    ProviderDialect, ProviderWireDialect, SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_gateway::ProviderRefType;
use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjection, ProviderPolicyProjector, ProviderProjectionError,
    ProviderProjectionInput,
};

/// LC 全相位(Planning/Review/Coding)的冻结 sandbox wire 文本
/// (REQ-LCG-04 改版:danger-full-access 是当前执行基线)。
pub const CODEX_LC_SANDBOX_DANGER_FULL_ACCESS: &str = "danger-full-access";
/// LC Author 岗位纪律(tool policy=deny)与 Supervised 档的 approval 冻结值。
pub const CODEX_LC_APPROVAL_ON_REQUEST: &str = "on-request";
/// LC 全开角色 Auto 档的 approval 冻结值。
pub const CODEX_LC_APPROVAL_NEVER: &str = "never";
/// gateway 提供的 boundary plan 与 envelope 冻结面不符时的稳定拒绝码
/// (Task 5b;计划 Step 3:native/plan 未证实不得放行)。
pub const CODEX_TARGET_BOUNDARY_UNVERIFIED: &str = "codex_target_boundary_unverified";

/// capability profile 摘要的 schema 前缀(字段序变化必须换 schema 版本;
/// v2:T12 改版后的字段序与 LC 独立 canonical 序列,v1 旧值不得复现)。
const PROFILE_DIGEST_SCHEMA: &str = "lc-codex-profile-v2";
/// 会话全投影摘要的 schema 前缀(v2 同上)。
const SESSION_DIGEST_SCHEMA: &str = "lc-codex-session-v2";
/// boundary 计划内容引用的 schema 前缀(内容字段未变,保持 v1)。
const BOUNDARY_PLAN_REF_SCHEMA: &str = "lc-codex-boundary-plan-v1";
/// LC tool-policy 独立 canonical 摘要的 schema 前缀。
const LC_TOOL_POLICY_DIGEST_SCHEMA: &str = "lc-codex-tool-policy-v1";

/// 固定 role×policy 映射的冻结快照(T11 公共角色矩阵:deny 是 Author 岗位
/// 纪律的物理投影,仅 Author 可承载角色合法;Reviewer/Coder/Handoff 全开)。
const CODEX_LC_ROLE_POLICY_MAP: &str = "author_duty=deny_file_write_builtins;author_carriers=orchestrator+work_item_splitter;reviewer=none;coder=none;handoff=none";

/// approval 映射的冻结快照(T12:按权限模式独立映射,Author 恒 on-request,
/// action 不参与)。
const CODEX_LC_APPROVAL_MAP: &str = "author=on-request;auto=never;supervised=on-request";

/// LC 全相位 danger 基线的独立 canonical 冻结片段(T12):LC capability
/// profile 摘要专用——绝不复用 direct `deny_file_write_builtins_tokens` 的
/// 只读冻结序列(sandbox/approval 控制面的 LC 自有快照)。
const CODEX_LC_WIRE_CONTROL_TOKENS: [&str; 2] = [
    "sandbox=danger-full-access",
    "approvalPolicy=author:on-request|auto:never|supervised:on-request",
];

/// codex LC sandbox 模式(REQ-LCG-04 改版):仅当前 danger 执行基线一种——
/// Planning/Review/Coding 全相位 wire 均 `danger-full-access`,无受限档。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexSandboxMode {
    DangerFullAccess,
}

impl CodexSandboxMode {
    /// 与 wire(`thread/start`/`thread/resume` params 的 `sandbox` 字段)
    /// 同源的冻结文本。
    pub fn wire_text(self) -> &'static str {
        match self {
            CodexSandboxMode::DangerFullAccess => CODEX_LC_SANDBOX_DANGER_FULL_ACCESS,
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

    /// 由全投影派生 codex danger sandbox 投影(单一来源;wire 参数与
    /// `process_cwd`/`protocol_cwd` 消费同源,调用方不能自造)。全相位恒
    /// danger 基线,action 只影响协议 cwd(双 cwd 合同)。
    pub(crate) fn sandbox_projection(
        &self,
        projection: &ProviderPolicyProjection,
    ) -> CodexSandboxProjection {
        let mode = CodexSandboxMode::DangerFullAccess;
        CodexSandboxProjection::new(
            mode,
            projection.approval_policy().to_string(),
            projection.working_directory().to_path_buf(),
            projection.protocol_working_directory().to_path_buf(),
            projection.target().worktree.clone(),
            projection.boundary_evidence_ref().to_string(),
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

/// `ProviderRefType` 的稳定文本(穷举显式映射,新增变体编译期强制补决策)。
pub(crate) fn provider_ref_type_text(provider: ProviderRefType) -> &'static str {
    match provider {
        ProviderRefType::ClaudeCode => "claude-code",
        ProviderRefType::Codex => "codex",
        ProviderRefType::Pi => "pi",
        ProviderRefType::KimiCode => "kimi-code",
    }
}

/// `ProviderDialect` 的稳定文本(serde snake_case 序列化同形)。
pub(crate) fn provider_dialect_text(dialect: ProviderDialect) -> &'static str {
    match dialect {
        ProviderDialect::ClaudeCodeCliV1 => "claude_code_cli_v1",
        ProviderDialect::CodexCliV1 => "codex_cli_v1",
        ProviderDialect::PiRpcV1 => "pi_rpc_v1",
        ProviderDialect::KimiAcpV1 => "kimi_acp_v1",
    }
}

/// `ProviderWireDialect` 的冻结序列化文本(与 policy.rs 的 serde rename 同值)。
pub(crate) fn wire_dialect_text(wire: ProviderWireDialect) -> &'static str {
    match wire {
        ProviderWireDialect::ClaudeCodeStreamJson => "claude-stream-json",
        ProviderWireDialect::CodexAppServerRpc => "codex-app-server-rpc",
        ProviderWireDialect::PiRpc => "pi-rpc",
        ProviderWireDialect::KimiAcp => "kimi-acp",
    }
}

fn permission_mode_text(mode: &ProviderPermissionMode) -> &'static str {
    match mode {
        ProviderPermissionMode::Auto => "auto",
        ProviderPermissionMode::Supervised => "supervised",
    }
}

fn boundary_mode_text(mode: ProviderBoundaryMode) -> &'static str {
    match mode {
        ProviderBoundaryMode::ReadOnly => "read-only",
        ProviderBoundaryMode::TargetWriteOnly => "target-write-only",
    }
}

/// 长度分隔的 canonical 字段(`len:value`,杜绝分隔符/注入歧义)。
fn canonical_field(canonical: &mut String, value: &str) {
    canonical.push_str(&value.len().to_string());
    canonical.push(':');
    canonical.push_str(value);
}

/// 投影 digest:固定字段序 + 长度分隔 + schema 前缀的 SHA-256,输出
/// `sha256:` + 64 位小写 hex(与 2c shape validator 的 digest 形状同构;
/// 禁止 Debug 文本/调用方摘要/递归哈希)。
fn lc_digest(schema: &str, parts: &[&str]) -> String {
    let mut canonical = String::new();
    canonical_field(&mut canonical, schema);
    for part in parts {
        canonical.push('|');
        canonical_field(&mut canonical, part);
    }
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// roots 的 canonical 字段串(逐 root 长度分隔,保持顺序)。
fn lc_roots_field(roots: &[PathBuf]) -> String {
    let mut field = String::new();
    for root in roots {
        if !field.is_empty() {
            field.push(',');
        }
        canonical_field(&mut field, &root.to_string_lossy());
    }
    field
}

/// LC 会话 tool-policy 的独立 canonical digest(T12):绝不与 direct 策略
/// 会话的 `canonical_tool_policy`/`tool_policy_digest` 共用函数或冻结序列。
/// `Some(DenyFileWriteBuiltins)`(Author 岗位纪律投影)与 `None`(全开
/// 角色)都走 LC 冻结 schema,digest 恒非空且稳定(统一 launch audit 不以
/// `tool_policy=None` 跳过);其余 intent(marker)对 codex LC 不合法,
/// fail-closed 拒绝(root recipe 自举通道要求固定 Claude provider)。
pub(crate) fn lc_tool_policy_canonical_digest(
    policy: Option<&ProviderToolPolicy>,
) -> Result<String, ProviderProjectionError> {
    let intent = match policy.map(|policy| &policy.intent) {
        None => "none",
        Some(ToolPolicyIntent::DenyFileWriteBuiltins) => "deny_file_write_builtins",
        Some(ToolPolicyIntent::BootstrapExecutorMarker(_)) => {
            return Err(ProviderProjectionError::Invalid(
                "codex lc tool policy digest: bootstrap executor markers are not codex LC policies"
                    .to_string(),
            ));
        }
    };
    Ok(lc_digest(LC_TOOL_POLICY_DIGEST_SCHEMA, &[intent]))
}

/// 由 gateway 冻结的 envelope 派生 boundary plan(逻辑交付面描述:Coding
/// =target 写面,read-only action 无写面)。`protected_roots` 的
/// launcher/probe 语义归 6a/6c,此处为空并不宣称物理隔离;物理 sandbox 恒
/// danger-full-access(REQ-LCG-04 改版),本 plan 只承载平台交付合同引用。
///
/// T12:物理 target-only 形状拒绝退役——Coding 唯一 target 根约束由
/// `SessionPolicyEnvelope` 构造层(平台交付合同)强制;投影层不再以废止的
/// 受限权限证明门拒绝非合同形态 envelope。
pub(crate) fn lc_boundary_plan(envelope: &SessionPolicyEnvelope) -> ProviderBoundaryPlan {
    match envelope.action {
        SessionPolicyAction::CodingTargetWrite => ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            envelope.working_directory.clone(),
            Some(envelope.target.worktree.clone()),
            Vec::new(),
        ),
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            ProviderBoundaryPlan::new(
                ProviderBoundaryMode::ReadOnly,
                envelope.working_directory.clone(),
                None,
                Vec::new(),
            )
        }
    }
}

/// gateway 提供的 boundary plan 按 action 复核形状(不合法即拒绝,不静默
/// 采纳外部 plan)。
fn verify_boundary_shape(
    envelope: &SessionPolicyEnvelope,
    plan: &ProviderBoundaryPlan,
) -> Result<(), ProviderProjectionError> {
    match envelope.action {
        SessionPolicyAction::CodingTargetWrite => {
            if plan.mode() != ProviderBoundaryMode::TargetWriteOnly
                || plan.target_root() != Some(envelope.target.worktree.as_path())
            {
                return Err(ProviderProjectionError::Invalid(format!(
                    "{CODEX_TARGET_BOUNDARY_UNVERIFIED}: coding boundary plan must be \
                     target-write-only with the envelope target"
                )));
            }
        }
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            if plan.mode() != ProviderBoundaryMode::ReadOnly || plan.target_root().is_some() {
                return Err(ProviderProjectionError::Invalid(format!(
                    "{CODEX_TARGET_BOUNDARY_UNVERIFIED}: read-only boundary plan must be \
                     read-only without a target root"
                )));
            }
        }
    }
    Ok(())
}

/// boundary 计划的内容定位引用(plan 内容 digest;非 probe 证据引用——真实
/// evidence_ref 归 6c/2d)。
fn boundary_plan_ref(plan: &ProviderBoundaryPlan) -> String {
    let target = plan
        .target_root()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let protected = lc_roots_field(plan.protected_roots());
    let cwd = plan.working_directory().to_string_lossy().into_owned();
    let digest = lc_digest(
        BOUNDARY_PLAN_REF_SCHEMA,
        &[
            boundary_mode_text(plan.mode()),
            cwd.as_str(),
            target.as_str(),
            protected.as_str(),
        ],
    );
    format!("boundary-plan:{digest}")
}

/// boundary 引用仅 Coding(有真实写面)非空;read-only 无写面为空串。
fn boundary_evidence_ref_for(plan: &ProviderBoundaryPlan) -> String {
    if plan.mode() == ProviderBoundaryMode::ReadOnly {
        return String::new();
    }
    boundary_plan_ref(plan)
}

/// approval 的独立映射(T12,REQ-LCG-04 改版):按权限模式映射
/// (Auto→never/Supervised→on-request),与 sandbox 档和 action 解耦;
/// Author 岗位纪律(tool policy=deny)恒 on-request。Task 3.4(B0c)起
/// 同时是 `prepare_lc_facts` approval fact 的单一来源。
pub(crate) fn lc_approval_policy(
    tool_policy: Option<&ProviderToolPolicy>,
    permission_mode: &ProviderPermissionMode,
) -> String {
    match tool_policy {
        // Author 岗位纪律投影(deny):恒 on-request。
        Some(_) => CODEX_LC_APPROVAL_ON_REQUEST.to_string(),
        None => match permission_mode {
            ProviderPermissionMode::Auto => CODEX_LC_APPROVAL_NEVER.to_string(),
            ProviderPermissionMode::Supervised => CODEX_LC_APPROVAL_ON_REQUEST.to_string(),
        },
    }
}

/// action 的协议 cwd 冻结面(双 cwd 合同:Coding=target worktree,只读=
/// canonical root)。
fn lc_protocol_cwd(envelope: &SessionPolicyEnvelope) -> PathBuf {
    match envelope.action {
        SessionPolicyAction::CodingTargetWrite => envelope.target.worktree.clone(),
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            envelope.working_directory.clone()
        }
    }
}

impl ProviderPolicyProjector for CodexPolicyProjector {
    fn project(
        &self,
        input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        let envelope = input.envelope();

        // 匹配门:只投影 Codex app-server(真实 version 未知/adapter 不匹配
        // 拒绝,不回退其它 provider/dialect)。
        if input.provider().provider_type != ProviderRefType::Codex {
            return Err(ProviderProjectionError::Unsupported(format!(
                "codex projector cannot project provider {}",
                provider_ref_type_text(input.provider().provider_type)
            )));
        }
        if envelope.provider_dialect != ProviderDialect::CodexCliV1 {
            return Err(ProviderProjectionError::Unsupported(format!(
                "codex projector requires dialect {}, got {}",
                provider_dialect_text(ProviderDialect::CodexCliV1),
                provider_dialect_text(envelope.provider_dialect)
            )));
        }
        if self.exact_version.trim().is_empty() {
            return Err(ProviderProjectionError::Unsupported(
                "codex exact version is unknown; refusing to project".to_string(),
            ));
        }
        if input.action() != envelope.action {
            return Err(ProviderProjectionError::Invalid(
                "projection action disagrees with the frozen envelope action".to_string(),
            ));
        }

        // boundary:gateway 提供的 plan 优先并按 action 复核形状;未提供时由
        // envelope 派生(逻辑交付面;物理 sandbox 恒 danger,plan 不承载
        // 物理隔离声明)。
        let boundary = match input.boundary() {
            Some(plan) => {
                verify_boundary_shape(envelope, plan)?;
                plan.clone()
            }
            None => lc_boundary_plan(envelope),
        };

        // 冻结 mapping(T12):sandbox/approval/协议 cwd 都由投影派生,不读
        // raw 输入;全相位恒 danger 基线。
        let sandbox_mode = CodexSandboxMode::DangerFullAccess;
        let approval_policy = lc_approval_policy(input.tool_policy(), input.permission_mode());
        let protocol_cwd = lc_protocol_cwd(envelope);

        // 1) capability profile 摘要(证据摘要分层):该 provider+version+
        //    action 的完整权限 profile——固定 role×policy 映射、LC 独立
        //    canonical wire 控制序列(danger 基线 + 独立 approval 映射)、
        //    boundary/MCP 控制规范。不含单次 role/target/config/trust,
        //    不含任何 evidence_ref/digest(禁止递归哈希);绝不复用 direct
        //    `deny_file_write_builtins_tokens` 冻结片段。
        let wire_control_tokens = CODEX_LC_WIRE_CONTROL_TOKENS.join(",");
        let profile_parts = [
            provider_ref_type_text(ProviderRefType::Codex),
            provider_dialect_text(envelope.provider_dialect),
            wire_dialect_text(ProviderWireDialect::CodexAppServerRpc),
            self.exact_version.as_str(),
            action_text(envelope.action),
            CODEX_LC_ROLE_POLICY_MAP,
            wire_control_tokens.as_str(),
            boundary_mode_text(boundary.mode()),
            input.mcp_bundle_digest(),
        ];
        let capability_projection_digest = lc_digest(PROFILE_DIGEST_SCHEMA, &profile_parts);

        // 2) 会话全投影摘要:profile 摘要 + 当前 role/permission/cwd/target/
        //    roots/trust/config/policy/authority + 实际 tool policy。分层包含
        //    profile 摘要(先算 profile 再算 session,单向无递归);codex 的
        //    双 cwd(process/protocol)都纳入。
        let session_tool_digest = lc_tool_policy_canonical_digest(input.tool_policy())?;
        let process_cwd = envelope.working_directory.to_string_lossy().into_owned();
        let protocol_cwd_field = protocol_cwd.to_string_lossy().into_owned();
        let authority = envelope.authority_root.to_string_lossy().into_owned();
        let mut target_field = String::new();
        canonical_field(&mut target_field, &envelope.target.logical_repository_id);
        target_field.push('|');
        canonical_field(&mut target_field, &envelope.target.checkout_id);
        target_field.push('|');
        canonical_field(
            &mut target_field,
            &envelope.target.worktree.to_string_lossy(),
        );
        let readable_field = lc_roots_field(&envelope.readable_roots);
        let writable_field = lc_roots_field(&envelope.writable_roots);
        let session_parts = [
            capability_projection_digest.as_str(),
            adapter_role_text(input.role()),
            permission_mode_text(input.permission_mode()),
            approval_policy.as_str(),
            session_tool_digest.as_str(),
            process_cwd.as_str(),
            protocol_cwd_field.as_str(),
            target_field.as_str(),
            readable_field.as_str(),
            writable_field.as_str(),
            input.trust_digest(),
            envelope.config_digest.as_str(),
            input.mcp_bundle_digest(),
            envelope.policy_digest.as_str(),
            authority.as_str(),
        ];
        let projection_digest = lc_digest(SESSION_DIGEST_SCHEMA, &session_parts);
        Ok(ProviderPolicyProjection::new(
            ProviderRefType::Codex,
            ProviderDialect::CodexCliV1,
            ProviderWireDialect::CodexAppServerRpc,
            self.exact_version.clone(),
            envelope.action,
            input.role().clone(),
            input.permission_mode().clone(),
            input.tool_policy().cloned(),
            approval_policy,
            sandbox_mode.wire_text().to_string(),
            envelope.working_directory.clone(),
            // 双 cwd 合同(Codex 专属):进程 cwd=canonical root,协议 cwd=
            // action 冻结面(Coding=target,只读=root);raw 输入不参与。
            protocol_cwd,
            envelope.target.clone(),
            envelope.readable_roots.clone(),
            envelope.writable_roots.clone(),
            input.trust_digest().to_string(),
            envelope.config_digest.clone(),
            input.mcp_bundle_digest().to_string(),
            boundary_evidence_ref_for(&boundary),
            capability_projection_digest,
            projection_digest,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::codex_provider::session::deny_file_write_builtins_tokens;
    use crate::cross_cutting::streaming_provider::canonical_tool_policy;
    use crate::protocol::contracts::AdapterRole;
    use crate::product::logical_codebase::policy::PolicyTarget;
    use crate::product::logical_codebase::provider_gateway::ProviderRef;
    use crate::product::logical_codebase::provider_projection::ProviderPolicyProjector;

    fn t12_root() -> PathBuf {
        std::env::temp_dir().join("codex-t12-projection-root")
    }

    fn t12_target() -> PathBuf {
        t12_root().join("member-worktree")
    }

    /// 旁路构造的 envelope(平台交付合同在 `SessionPolicyEnvelope` 构造层;
    /// 投影层测试需要非合同形态变体时经 struct literal 构造)。
    fn t12_envelope(
        action: SessionPolicyAction,
        writable_roots: Vec<PathBuf>,
    ) -> SessionPolicyEnvelope {
        let root = t12_root();
        SessionPolicyEnvelope {
            policy_id: "policy_lc_codex_t12".to_string(),
            policy_revision: 1,
            policy_digest: "sha256:policy-lc-codex-t12".to_string(),
            action,
            target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", t12_target()),
            working_directory: root.clone(),
            readable_roots: vec![root.clone()],
            writable_roots,
            provider_dialect: ProviderDialect::CodexCliV1,
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
            config_digest: "sha256:cfg-digest-t12".to_string(),
            created_at: "2026-10-17T00:00:00Z".to_string(),
            authority_root: root,
        }
    }

    fn t12_input(
        action: SessionPolicyAction,
        role: AdapterRole,
        permission_mode: ProviderPermissionMode,
        tool_policy: Option<ProviderToolPolicy>,
        writable_roots: Vec<PathBuf>,
    ) -> ProviderProjectionInput {
        let envelope = t12_envelope(action, writable_roots);
        ProviderProjectionInput::new(
            envelope.clone(),
            ProviderRef::codex("cap_codex_t12"),
            action,
            role,
            permission_mode,
            tool_policy,
            String::new(),
            String::new(),
            envelope.config_artifact_ref.clone(),
            String::new(),
            None,
        )
    }

    fn t12_coding_input(
        role: AdapterRole,
        permission_mode: ProviderPermissionMode,
        tool_policy: Option<ProviderToolPolicy>,
        writable_roots: Vec<PathBuf>,
    ) -> ProviderProjectionInput {
        t12_input(
            SessionPolicyAction::CodingTargetWrite,
            role,
            permission_mode,
            tool_policy,
            writable_roots,
        )
    }

    /// T12(REQ-LCG-04 改版):Codex LC 全相位(Planning/Review/Coding)投影
    /// danger-full-access——read-only/workspace-write 档不再产出,无 Danger
    /// 拒绝;合法 danger 不判 unsupported。
    #[test]
    fn lc_projects_danger_full_access_wire_for_all_three_actions() {
        for action in [
            SessionPolicyAction::PlanningReadOnly,
            SessionPolicyAction::CodingTargetWrite,
            SessionPolicyAction::ReviewReadOnly,
        ] {
            let writable_roots = match action {
                SessionPolicyAction::CodingTargetWrite => vec![t12_target()],
                _ => Vec::new(),
            };
            let projection = CodexPolicyProjector::new("codex 0.124.0-t12")
                .project(&t12_input(
                    action,
                    AdapterRole::Executor,
                    ProviderPermissionMode::Auto,
                    None,
                    writable_roots,
                ))
                .unwrap_or_else(|error| panic!("{action:?} danger projection: {error}"));
            assert_eq!(projection.sandbox(), "danger-full-access", "{action:?}");
            assert_ne!(projection.sandbox(), "read-only", "{action:?}");
            assert_ne!(projection.sandbox(), "workspace-write", "{action:?}");
            let sandbox =
                CodexPolicyProjector::new("codex 0.124.0-t12").sandbox_projection(&projection);
            assert_eq!(
                sandbox.mode().wire_text(),
                "danger-full-access",
                "{action:?}"
            );
            assert_eq!(projection.approval_policy(), "never", "{action:?} Auto→never");
        }
    }

    /// T12:approval 按权限模式独立映射(Auto→never/Supervised→on-request),
    /// Author 岗位纪律(tool policy=deny)恒 on-request;action 不再参与。
    #[test]
    fn lc_approval_maps_independently_by_permission_mode_with_author_on_request() {
        let deny = ProviderToolPolicy::deny_file_write_builtins();
        let cases: [(SessionPolicyAction, AdapterRole, ProviderPermissionMode, bool, &str); 6] = [
            (
                SessionPolicyAction::PlanningReadOnly,
                AdapterRole::Orchestrator,
                ProviderPermissionMode::Auto,
                true,
                "on-request",
            ),
            (
                SessionPolicyAction::PlanningReadOnly,
                AdapterRole::Orchestrator,
                ProviderPermissionMode::Supervised,
                true,
                "on-request",
            ),
            (
                SessionPolicyAction::ReviewReadOnly,
                AdapterRole::Reviewer,
                ProviderPermissionMode::Auto,
                false,
                "never",
            ),
            (
                SessionPolicyAction::ReviewReadOnly,
                AdapterRole::Reviewer,
                ProviderPermissionMode::Supervised,
                false,
                "on-request",
            ),
            (
                SessionPolicyAction::CodingTargetWrite,
                AdapterRole::Executor,
                ProviderPermissionMode::Auto,
                false,
                "never",
            ),
            (
                SessionPolicyAction::CodingTargetWrite,
                AdapterRole::Executor,
                ProviderPermissionMode::Supervised,
                false,
                "on-request",
            ),
        ];
        for (action, role, permission_mode, author, expected) in cases {
            let tool_policy = author.then(|| deny.clone());
            let writable_roots = match action {
                SessionPolicyAction::CodingTargetWrite => vec![t12_target()],
                _ => Vec::new(),
            };
            let projection = CodexPolicyProjector::new("codex 0.124.0-t12")
                .project(&t12_input(
                    action,
                    role.clone(),
                    permission_mode.clone(),
                    tool_policy,
                    writable_roots,
                ))
                .unwrap_or_else(|error| panic!("{action:?} projection: {error}"));
            assert_eq!(
                projection.approval_policy(),
                expected,
                "{action:?} {permission_mode:?} author={author}"
            );
        }
    }

    /// T12:LC capability/profile/session digest 使用独立 canonical 序列——
    /// 绝不与 direct 的 `deny_file_write_builtins_tokens` 冻结片段或
    /// `canonical_tool_policy`/`tool_policy_digest` 共用序列/函数;旧 v1
    /// 冻结序列(复用 direct 只读片段)的摘要值不得复现。
    #[test]
    fn lc_digests_use_independent_canonical_sequence() {
        let deny = ProviderToolPolicy::deny_file_write_builtins();

        // (a) tool-policy 摘要与 direct canonical 序列不同源、不共用函数产物。
        let lc_policy_digest =
            lc_tool_policy_canonical_digest(Some(&deny)).expect("lc tool policy digest");
        let direct_digest = canonical_tool_policy(
            crate::cross_cutting::codex_provider::session::TOOL_POLICY_PROVIDER_NAME,
            &deny,
        )
        .expect("direct canonical digest")
        .digest;
        assert_ne!(lc_policy_digest, direct_digest);
        assert_ne!(
            lc_policy_digest,
            format!("sha256:{direct_digest}"),
            "lc digest must not be a prefix-wrapped reuse of the direct canonical digest"
        );
        let lc_none_digest = lc_tool_policy_canonical_digest(None).expect("none digest");
        assert!(lc_none_digest.starts_with("sha256:"));
        assert_ne!(lc_none_digest, lc_policy_digest);

        // (b) capability profile 摘要不再复现旧 v1 冻结序列(direct deny tokens
        // 进 profile 的摘要值)。
        let projection = CodexPolicyProjector::new("codex 0.124.0-t12")
            .project(&t12_input(
                SessionPolicyAction::PlanningReadOnly,
                AdapterRole::Orchestrator,
                ProviderPermissionMode::Auto,
                Some(deny),
                Vec::new(),
            ))
            .expect("author planning projection");
        let deny_tokens = deny_file_write_builtins_tokens().join(",");
        let legacy_v1_parts = [
            provider_ref_type_text(ProviderRefType::Codex),
            provider_dialect_text(ProviderDialect::CodexCliV1),
            wire_dialect_text(ProviderWireDialect::CodexAppServerRpc),
            "codex 0.124.0-t12",
            action_text(SessionPolicyAction::PlanningReadOnly),
            "orchestrator=deny_file_write_builtins;work_item_splitter=deny_file_write_builtins;reviewer=deny_file_write_builtins;executor=none;handoff=none",
            deny_tokens.as_str(),
            "read_only=on-request;coding_auto=never;coding_supervised=on-request",
            "read-only",
            "read-only",
            "",
        ];
        let legacy_v1_digest = lc_digest("lc-codex-profile-v1", &legacy_v1_parts);
        assert_ne!(
            projection.capability_projection_digest(),
            legacy_v1_digest.as_str(),
            "capability digest must not reproduce the retired v1 sequence reusing direct deny tokens"
        );
    }

    /// T12:物理 target-only 形状拒绝退役——Coding 写面非 target-only(旁路
    /// 构造的 envelope)按新基线 danger 放行投影,不再以废止的受限权限证明门
    /// /`codex_danger_full_access_unsupported` 稳定码拒绝;read-only action 带
    /// 可写 root 同样不再被投影层拒绝(平台交付合同在 envelope 构造层)。
    #[test]
    fn lc_retires_physical_target_only_shape_refusal() {
        let target = t12_target();
        let other = t12_root().join("other-member");

        // Coding:写面非 target-only(writable roots 含 target 之外的 root)。
        let coding = CodexPolicyProjector::new("codex 0.124.0-t12")
            .project(&t12_coding_input(
                AdapterRole::Executor,
                ProviderPermissionMode::Auto,
                None,
                vec![target, other.clone()],
            ))
            .expect("non-target-only coding write face must project danger (retired gate)");
        assert_eq!(coding.sandbox(), "danger-full-access");
        assert!(!coding.boundary_evidence_ref().is_empty());

        // read-only action 携带可写 root:投影层不再拒绝。
        let planning = CodexPolicyProjector::new("codex 0.124.0-t12")
            .project(&t12_input(
                SessionPolicyAction::PlanningReadOnly,
                AdapterRole::Reviewer,
                ProviderPermissionMode::Auto,
                None,
                vec![other],
            ))
            .expect("planning projection no longer re-refuses writable roots at the projector");
        assert_eq!(planning.sandbox(), "danger-full-access");
        assert!(planning.boundary_evidence_ref().is_empty());
    }
}
