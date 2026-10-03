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
//!   target/roots/trust/config/authority/permission/实际 tool policy),任一
//!   漂移即变。
//! - digest 输入 = 固定字段序 + 长度分隔 + schema 前缀;禁止 Debug 文本与
//!   递归哈希(profile 摘要不含 evidence_ref/digest 自身)。

use std::path::PathBuf;

use sha2::{Digest as ShaDigest, Sha256};

use crate::cross_cutting::claude_code_provider::{
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
use crate::product::logical_codebase::provider_gateway::ProviderRefType;
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

/// capability profile 摘要的 schema 前缀(字段序变化必须换 schema 版本)。
const PROFILE_DIGEST_SCHEMA: &str = "lc-claude-profile-v1";
/// 会话全投影摘要的 schema 前缀。
const SESSION_DIGEST_SCHEMA: &str = "lc-claude-session-v1";
/// boundary 计划内容引用的 schema 前缀。
const BOUNDARY_PLAN_REF_SCHEMA: &str = "lc-claude-boundary-plan-v1";

/// 固定 role×policy 映射的冻结快照(与 `validate_tool_policy_for_role` 同源:
/// 策略角色必带 DenyFileWriteBuiltins,普通 Executor/Handoff 无通用策略;
/// BootstrapExecutorMarker 是 root-recipe 例外,不进 LC profile)。
const CLAUDE_LC_ROLE_POLICY_MAP: &str = "orchestrator=deny_file_write_builtins;work_item_splitter=deny_file_write_builtins;reviewer=deny_file_write_builtins;executor=none;handoff=none";

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

/// `SessionPolicyAction` 的稳定文本(serde snake_case 序列化同形)。
pub(crate) fn action_text(action: SessionPolicyAction) -> &'static str {
    match action {
        SessionPolicyAction::PlanningReadOnly => "planning_read_only",
        SessionPolicyAction::CodingTargetWrite => "coding_target_write",
        SessionPolicyAction::ReviewReadOnly => "review_read_only",
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
                    "claude lc tool policy is not canonicalizable: {error}"
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
pub(crate) fn lc_boundary_plan(
    envelope: &SessionPolicyEnvelope,
) -> Result<ProviderBoundaryPlan, ProviderProjectionError> {
    match envelope.action {
        SessionPolicyAction::CodingTargetWrite => {
            let expected = envelope.target.worktree.clone();
            if envelope.writable_roots.len() != 1 || envelope.writable_roots[0] != expected {
                return Err(ProviderProjectionError::Invalid(format!(
                    "coding envelope must freeze exactly one writable root equal to the target worktree: {}",
                    expected.display()
                )));
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
                    "read-only envelope must freeze no writable roots".to_string(),
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
                return Err(ProviderProjectionError::Invalid(
                    "coding boundary plan must be target-write-only with the envelope target"
                        .to_string(),
                ));
            }
        }
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            if plan.mode() != ProviderBoundaryMode::ReadOnly || plan.target_root().is_some() {
                return Err(ProviderProjectionError::Invalid(
                    "read-only boundary plan must be read-only without a target root".to_string(),
                ));
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

impl ProviderPolicyProjector for ClaudePolicyProjector {
    fn project(
        &self,
        input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        let envelope = input.envelope();

        // 匹配门:只投影 Claude Code(真实 version 未知/adapter 不匹配拒绝)。
        if input.provider().provider_type != ProviderRefType::ClaudeCode {
            return Err(ProviderProjectionError::Unsupported(format!(
                "claude projector cannot project provider {}",
                provider_ref_type_text(input.provider().provider_type)
            )));
        }
        if envelope.provider_dialect != ProviderDialect::ClaudeCodeCliV1 {
            return Err(ProviderProjectionError::Unsupported(format!(
                "claude projector requires dialect {}, got {}",
                provider_dialect_text(ProviderDialect::ClaudeCodeCliV1),
                provider_dialect_text(envelope.provider_dialect)
            )));
        }
        if self.exact_version.trim().is_empty() {
            return Err(ProviderProjectionError::Unsupported(
                "claude exact version is unknown; refusing to project".to_string(),
            ));
        }
        if input.action() != envelope.action {
            return Err(ProviderProjectionError::Invalid(
                "projection action disagrees with the frozen envelope action".to_string(),
            ));
        }

        // boundary:gateway 提供的 plan 优先并按 action 复核形状;未提供时由
        // envelope 派生(read-only action 允许无 plan)。
        let boundary = match input.boundary() {
            Some(plan) => {
                verify_boundary_shape(envelope, plan)?;
                plan.clone()
            }
            None => lc_boundary_plan(envelope)?,
        };

        // 1) capability profile 摘要(证据摘要分层):该 provider+version+
        //    action 的完整权限 profile——固定 role×policy 映射、deny token
        //    序列、headless allowlist、approval/sandbox/MCP 控制规范。不含
        //    单次 role/target/config/trust,不含任何 evidence_ref/digest
        //    (禁止递归哈希),不随单次会话变化。
        let deny_tokens = deny_file_write_builtins_tokens().join(",");
        let profile_parts = [
            provider_ref_type_text(ProviderRefType::ClaudeCode),
            provider_dialect_text(envelope.provider_dialect),
            wire_dialect_text(ProviderWireDialect::ClaudeCodeStreamJson),
            self.exact_version.as_str(),
            action_text(envelope.action),
            CLAUDE_LC_ROLE_POLICY_MAP,
            deny_tokens.as_str(),
            CLAUDE_LC_ALLOWED_TOOLS,
            CLAUDE_LC_APPROVAL_POLICY,
            boundary_mode_text(boundary.mode()),
            input.mcp_bundle_digest(),
        ];
        let capability_projection_digest = lc_digest(PROFILE_DIGEST_SCHEMA, &profile_parts);

        // 2) 会话全投影摘要:profile 摘要 + 当前 role/permission/cwd/target/
        //    roots/trust/config/policy/authority + 实际 tool policy。分层
        //    包含 profile 摘要(先算 profile 再算 session,单向无递归)。
        let session_tool_digest = lc_tool_policy_canonical_digest(input.tool_policy())?;
        let cwd = envelope.working_directory.to_string_lossy().into_owned();
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
            CLAUDE_LC_APPROVAL_POLICY,
            session_tool_digest.as_str(),
            cwd.as_str(),
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
            ProviderRefType::ClaudeCode,
            ProviderDialect::ClaudeCodeCliV1,
            ProviderWireDialect::ClaudeCodeStreamJson,
            self.exact_version.clone(),
            envelope.action,
            input.role().clone(),
            input.permission_mode().clone(),
            input.tool_policy().cloned(),
            CLAUDE_LC_APPROVAL_POLICY.to_string(),
            boundary_mode_text(boundary.mode()).to_string(),
            envelope.working_directory.clone(),
            // claude 单 cwd:进程 cwd 与协议 cwd 同一(Global Constraints
            // 的 canonical root cwd 合同;codex 的双 cwd 分离不适用于此)。
            envelope.working_directory.clone(),
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
