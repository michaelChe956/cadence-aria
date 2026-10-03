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

use sha2::{Digest as ShaDigest, Sha256};

use crate::cross_cutting::codex_provider::session::{
    TOOL_POLICY_PROVIDER_NAME, deny_file_write_builtins_tokens,
};
use crate::cross_cutting::provider_boundary::{ProviderBoundaryMode, ProviderBoundaryPlan};
use crate::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderToolPolicy, TOOL_POLICY_APPROVAL_POLICY_VERSION,
    adapter_role_text, canonical_tool_policy, tool_policy_digest,
};
use crate::product::logical_codebase::policy::{
    ProviderDialect, ProviderWireDialect, SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_gateway::{
    CODEX_DANGER_FULL_ACCESS_UNSUPPORTED, ProviderRefType,
};
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
/// gateway 提供的 boundary plan 与 envelope 冻结面不符时的稳定拒绝码
/// (Task 5b;计划 Step 3:native/plan 未证实不得放行)。
pub const CODEX_TARGET_BOUNDARY_UNVERIFIED: &str = "codex_target_boundary_unverified";

/// capability profile 摘要的 schema 前缀(字段序变化必须换 schema 版本)。
const PROFILE_DIGEST_SCHEMA: &str = "lc-codex-profile-v1";
/// 会话全投影摘要的 schema 前缀。
const SESSION_DIGEST_SCHEMA: &str = "lc-codex-session-v1";
/// boundary 计划内容引用的 schema 前缀。
const BOUNDARY_PLAN_REF_SCHEMA: &str = "lc-codex-boundary-plan-v1";

/// 固定 role×policy 映射的冻结快照(与 `validate_tool_policy_for_role` 同源:
/// 策略角色必带 DenyFileWriteBuiltins,普通 Executor/Handoff 无通用策略;
/// BootstrapExecutorMarker 是 root-recipe 例外,不进 LC profile)。
const CODEX_LC_ROLE_POLICY_MAP: &str = "orchestrator=deny_file_write_builtins;work_item_splitter=deny_file_write_builtins;reviewer=deny_file_write_builtins;executor=none;handoff=none";

/// action×permission → approval 的冻结映射快照(REQ-LCG-04:只读恒
/// on-request;Coding Auto→never/Supervised→on-request)。
const CODEX_LC_APPROVAL_MAP: &str =
    "read_only=on-request;coding_auto=never;coding_supervised=on-request";

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
    /// `process_cwd`/`protocol_cwd` 消费同源,调用方不能自造)。
    pub(crate) fn sandbox_projection(
        &self,
        projection: &ProviderPolicyProjection,
    ) -> CodexSandboxProjection {
        let mode = match projection.action() {
            SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
                CodexSandboxMode::ReadOnly
            }
            SessionPolicyAction::CodingTargetWrite => CodexSandboxMode::WorkspaceWrite,
        };
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

/// LC 会话的 tool-policy canonical digest:`Some(deny)` 沿既有 canonical
/// 形态;`None`(无通用策略角色)使用空 token 序列的 canonical 形态——digest
/// 仍非空且稳定,统一 launch audit 不以 `tool_policy=None` 跳过。
pub(crate) fn lc_tool_policy_canonical_digest(
    policy: Option<&ProviderToolPolicy>,
) -> Result<String, ProviderProjectionError> {
    match policy {
        Some(policy) => canonical_tool_policy(TOOL_POLICY_PROVIDER_NAME, policy)
            .map(|canonical| canonical.digest)
            .map_err(|error| {
                ProviderProjectionError::Invalid(format!(
                    "codex lc tool policy is not canonicalizable: {error}"
                ))
            }),
        None => Ok(tool_policy_digest(
            TOOL_POLICY_PROVIDER_NAME,
            &[],
            TOOL_POLICY_APPROVAL_POLICY_VERSION,
        )),
    }
}

/// 由 gateway 冻结的 envelope 派生不可伪造 boundary plan(Coding 恰一个可写
/// target;read-only action 无可写面)。`protected_roots` 的 launcher/probe
/// 语义归 Task 6a/6c,此处为空并不宣称物理隔离。
///
/// REQ-LCG-04 危险门:Coding 写面非 target-only 时受限投影无法成立——
/// direct 映射的唯一替代是 danger-full-access,以稳定码拒绝(调用侧在
/// `ProcessManager::spawn` 之前失败,零 child)。
pub(crate) fn lc_boundary_plan(
    envelope: &SessionPolicyEnvelope,
) -> Result<ProviderBoundaryPlan, ProviderProjectionError> {
    match envelope.action {
        SessionPolicyAction::CodingTargetWrite => {
            let expected = envelope.target.worktree.clone();
            if envelope.writable_roots.len() != 1 || envelope.writable_roots[0] != expected {
                return Err(coding_danger_refusal(envelope));
            }
            Ok(ProviderBoundaryPlan::new(
                ProviderBoundaryMode::TargetWriteOnly,
                envelope.working_directory.clone(),
                Some(expected),
                Vec::new(),
            ))
        }
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            if !envelope.writable_roots.is_empty() {
                return Err(ProviderProjectionError::Invalid(
                    "codex lc projection: read-only envelope must freeze no writable roots"
                        .to_string(),
                ));
            }
            Ok(ProviderBoundaryPlan::new(
                ProviderBoundaryMode::ReadOnly,
                envelope.working_directory.clone(),
                None,
                Vec::new(),
            ))
        }
    }
}

/// Coder 形态判定(Coding action + 无通用 tool policy):该形态的 direct
/// 映射是 danger-full-access——其一切预认证失败(写面未冻结、exact
/// version 不可得等)都以 `CODEX_DANGER_FULL_ACCESS_UNSUPPORTED` 稳定码
/// 拒绝,不放行泛化错误(Task 5b 受限门深化)。
pub(crate) fn lc_coder_danger_shape(
    envelope: &SessionPolicyEnvelope,
    tool_policy: Option<&ProviderToolPolicy>,
) -> bool {
    envelope.action == SessionPolicyAction::CodingTargetWrite && tool_policy.is_none()
}

/// REQ-LCG-04 危险门:Coder 形态的 LC 启动若受限写面未冻结(非
/// target-only),启动后的 direct 映射只能是 danger-full-access——永久
/// 拒绝,details 即稳定码(与 gateway 路由门同源字节)。返回 `Some(稳定码)`
/// 表示必须以该 reason 拒绝(spawn 之前,零 child);`lc_boundary_plan`
/// 的 Coding 形状分支复用本错误。
pub(crate) fn lc_danger_refusal(
    envelope: &SessionPolicyEnvelope,
    tool_policy: Option<&ProviderToolPolicy>,
) -> Option<&'static str> {
    if !lc_coder_danger_shape(envelope, tool_policy) {
        return None;
    }
    let target_only = envelope.writable_roots.len() == 1
        && envelope.writable_roots[0] == envelope.target.worktree;
    if target_only {
        None
    } else {
        Some(CODEX_DANGER_FULL_ACCESS_UNSUPPORTED)
    }
}

/// `lc_boundary_plan` Coding 形状违规的 danger 拒绝错误(稳定码进消息,
/// 供 projector 直连调用方观测;adapter 侧 `start_lc_validated` 先行危险门
/// 返回 details=稳定码本体)。
fn coding_danger_refusal(envelope: &SessionPolicyEnvelope) -> ProviderProjectionError {
    ProviderProjectionError::Invalid(format!(
        "{CODEX_DANGER_FULL_ACCESS_UNSUPPORTED}: coding write face must freeze exactly one \
         writable root equal to the target worktree (got {} roots); danger-full-access \
         fallback is permanently refused (target {})",
        envelope.writable_roots.len(),
        envelope.target.worktree.display()
    ))
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

/// action×permission 的冻结 approval 映射(REQ-LCG-04:只读恒 on-request;
/// Coding Auto→never/Supervised→on-request)。
fn lc_approval_policy(
    action: SessionPolicyAction,
    permission_mode: &ProviderPermissionMode,
) -> String {
    match action {
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            CODEX_LC_APPROVAL_ON_REQUEST.to_string()
        }
        SessionPolicyAction::CodingTargetWrite => match permission_mode {
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
        // envelope 派生(Coding 写面非 target-only → danger 稳定码拒绝)。
        let boundary = match input.boundary() {
            Some(plan) => {
                verify_boundary_shape(envelope, plan)?;
                plan.clone()
            }
            None => lc_boundary_plan(envelope)?,
        };

        // 冻结 mapping(REQ-LCG-04):sandbox/approval/协议 cwd 都由投影派生,
        // 不读 raw 输入。
        let sandbox_mode = match envelope.action {
            SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
                CodexSandboxMode::ReadOnly
            }
            SessionPolicyAction::CodingTargetWrite => CodexSandboxMode::WorkspaceWrite,
        };
        let approval_policy = lc_approval_policy(envelope.action, input.permission_mode());
        let protocol_cwd = lc_protocol_cwd(envelope);

        // 1) capability profile 摘要(证据摘要分层):该 provider+version+
        //    action 的完整权限 profile——固定 role×policy 映射、deny token
        //    序列、approval/sandbox/MCP 控制规范。不含单次 role/target/
        //    config/trust,不含任何 evidence_ref/digest(禁止递归哈希)。
        let deny_tokens = deny_file_write_builtins_tokens().join(",");
        let profile_parts = [
            provider_ref_type_text(ProviderRefType::Codex),
            provider_dialect_text(envelope.provider_dialect),
            wire_dialect_text(ProviderWireDialect::CodexAppServerRpc),
            self.exact_version.as_str(),
            action_text(envelope.action),
            CODEX_LC_ROLE_POLICY_MAP,
            deny_tokens.as_str(),
            CODEX_LC_APPROVAL_MAP,
            sandbox_mode.wire_text(),
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
