//! Kimi Code 的 LC 权限投影(Task 4c,REQ-LCG-03/06)。
//!
//! `KimiPolicyProjector` 实现 1a 冻结的 `ProviderPolicyProjector`:消费
//! gateway 构造的 `ProviderProjectionInput`,产出 21 个冻结字段全量填充的
//! `ProviderPolicyProjection`。digest 语义遵循计划「证据摘要分层」:
//! - `capability_projection_digest` = 该 provider+version+action 的完整权限
//!   profile 摘要(ClientServicePolicy 控制面、approval、boundary 模式与
//!   MCP 控制规范),**不随单次 role/target/config 变化**;action row 的
//!   `projection_digest` 与之同源。
//! - `projection_digest` = 当前会话全投影摘要(profile 摘要 + role/cwd/
//!   target/roots/trust/config/authority/permission),任一漂移即变。
//! - digest 输入 = 固定字段序 + 长度分隔 + schema 前缀;禁止 Debug 文本与
//!   递归哈希(profile 摘要不含 evidence_ref/digest 自身)。
//!
//! 与 4a/4b(Claude/Pi)的差异:
//! - Kimi 通用 tool_policy 恒为 `None`:非空通用策略直接拒绝(Global
//!   Constraints「Kimi 拒绝非空通用策略,继续既有 ClientServicePolicy」,
//!   稳定码 `provider_generic_tool_policy_forbidden`);argv 冻结 `acp`,
//!   无任何策略物理片段,profile 摘要相应以 ClientServicePolicy 控制面为
//!   工具控制规范(无 argv deny token 字段)。
//! - 读写面分离:host root(canonical LC root)保持 ro、target root rw;
//!   fs read/write 与 terminal 宿主 handler 消费同一不可伪造 target
//!   boundary plan(见 `client_services`),terminal 实际 cwd 不改变
//!   provider 进程 cwd(ACP cwd 保持 root)。
//! - `mcp_bundle_digest` 只对应 Aria 注入 bundle;无注入时以
//!   `KIMI_NATIVE_MCP_SOURCE` 单独标记 native 项目配置来源,空串来源
//!   fail-closed(不冒充 Aria bundle digest)。

use std::path::PathBuf;

use sha2::{Digest as ShaDigest, Sha256};

use crate::cross_cutting::kimi_code_provider::TOOL_POLICY_PROVIDER_NAME;
use crate::cross_cutting::provider_boundary::{ProviderBoundaryMode, ProviderBoundaryPlan};
use crate::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderToolPolicy, TOOL_POLICY_APPROVAL_POLICY_VERSION,
    adapter_role_text, tool_policy_digest,
};
use crate::product::logical_codebase::policy::{
    ProviderDialect, ProviderWireDialect, SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_gateway::ProviderRefType;
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

/// capability profile 摘要的 schema 前缀(字段序变化必须换 schema 版本)。
const PROFILE_DIGEST_SCHEMA: &str = "lc-kimi-profile-v1";
/// 会话全投影摘要的 schema 前缀。
const SESSION_DIGEST_SCHEMA: &str = "lc-kimi-session-v1";
/// boundary 计划内容引用的 schema 前缀。
const BOUNDARY_PLAN_REF_SCHEMA: &str = "lc-kimi-boundary-plan-v1";

/// 固定 role×policy 映射的冻结快照(Kimi 形态:无通用 tool policy,角色
/// 控制由既有 24 格 ClientServicePolicy 决策表承担,direct/LC 同表)。
const KIMI_LC_ROLE_POLICY_MAP: &str = "generic=none-rejected;control=client_service_policy_matrix";

/// 宿主 client services 的控制面快照(读写面分离的 profile 事实):
/// fs 读保持 host root 只读面;fs 写与 terminal 的可写面唯一锚定 target
/// boundary plan(read-only action 无写面)。
const KIMI_LC_CLIENT_SERVICE_FACE: &str =
    "fs_read=host_ro;fs_write=target_boundary;terminal=target_boundary";

/// Kimi LC projector:构造时冻结已实测 exact version。真实 version 未知
/// (空串)时 `project` fail-closed 拒绝,不产出投影。
#[derive(Debug, Clone)]
pub struct KimiPolicyProjector {
    exact_version: String,
}

impl KimiPolicyProjector {
    /// 以已解析的 provider exact version 构造 projector(registry 装配/测试
    /// seam;version 解析本身由 adapter 侧 CLI `--version` 探测承担)。
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

/// LC 会话的 tool-policy canonical digest(Kimi 形态):通用 tool policy
/// 恒 `None`——`None` 使用空 token 序列的 canonical 形态,digest 仍非空且
/// 稳定,统一 launch audit 不以 `tool_policy=None` 跳过;`Some(_)` 直接
/// 拒绝(稳定码,与 `start_validated` 同源的第二道门)。
pub(crate) fn lc_tool_policy_canonical_digest(
    policy: Option<&ProviderToolPolicy>,
) -> Result<String, ProviderProjectionError> {
    match policy {
        Some(_) => Err(ProviderProjectionError::Unsupported(format!(
            "{KIMI_GENERIC_TOOL_POLICY_FORBIDDEN}: kimi carries no generic tool policy"
        ))),
        None => Ok(tool_policy_digest(
            TOOL_POLICY_PROVIDER_NAME,
            &[],
            TOOL_POLICY_APPROVAL_POLICY_VERSION,
        )),
    }
}

/// 由 gateway 冻结的 envelope 派生不可伪造 boundary plan(Coding 恰一个可写
/// target;read-only action 无可写面)。`protected_roots` 的 launcher/probe
/// 语义归 Task 6a/6c,此处为空并不宣称物理隔离;该 plan 同时是宿主 client
/// services(fs/terminal)的读写面授权来源(Task 4c 读写面分离)。
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

/// MCP 来源字段校验:只接受 Aria 注入 bundle digest(`sha256:` 前缀)或
/// native 项目配置来源标记;空串 fail-closed(来源必须显式标注,不冒充)。
fn verify_mcp_source(mcp_bundle_digest: &str) -> Result<(), ProviderProjectionError> {
    if mcp_bundle_digest.trim().is_empty() {
        return Err(ProviderProjectionError::Invalid(
            "kimi mcp source must be an aria bundle digest or the native-project-config marker"
                .to_string(),
        ));
    }
    Ok(())
}

impl ProviderPolicyProjector for KimiPolicyProjector {
    fn project(
        &self,
        input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        let envelope = input.envelope();

        // 匹配门:只投影 Kimi Code(真实 version 未知/adapter 不匹配拒绝)。
        if input.provider().provider_type != ProviderRefType::KimiCode {
            return Err(ProviderProjectionError::Unsupported(format!(
                "kimi projector cannot project provider {}",
                provider_ref_type_text(input.provider().provider_type)
            )));
        }
        if envelope.provider_dialect != ProviderDialect::KimiAcpV1 {
            return Err(ProviderProjectionError::Unsupported(format!(
                "kimi projector requires dialect {}, got {}",
                provider_dialect_text(ProviderDialect::KimiAcpV1),
                provider_dialect_text(envelope.provider_dialect)
            )));
        }
        if self.exact_version.trim().is_empty() {
            return Err(ProviderProjectionError::Unsupported(
                "kimi exact version is unknown; refusing to project".to_string(),
            ));
        }
        if input.action() != envelope.action {
            return Err(ProviderProjectionError::Invalid(
                "projection action disagrees with the frozen envelope action".to_string(),
            ));
        }

        // Kimi 通用 tool policy 恒 None:非空即拒(稳定码;与
        // `start_validated` 在 version/child 前的拒绝同源)。
        if let Some(policy) = input.tool_policy() {
            return Err(ProviderProjectionError::Unsupported(format!(
                "{KIMI_GENERIC_TOOL_POLICY_FORBIDDEN}: kimi carries no generic tool policy (got {policy:?})"
            )));
        }

        // MCP 来源:只接受 Aria bundle digest 或 native 标记,空串拒绝。
        verify_mcp_source(input.mcp_bundle_digest())?;

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
        //    action 的完整权限 profile——ClientServicePolicy 控制面、宿主
        //    client service 读写面、approval、boundary 模式与 MCP 控制规范。
        //    不含单次 role/target/config/trust,不含任何 evidence_ref/digest
        //    (禁止递归哈希),不随单次会话变化。与 4a/4b 的差异:无 argv
        //    deny token / allowlist 字段(kimi argv 冻结 `acp`)。
        let profile_parts = [
            provider_ref_type_text(ProviderRefType::KimiCode),
            provider_dialect_text(envelope.provider_dialect),
            wire_dialect_text(ProviderWireDialect::KimiAcp),
            self.exact_version.as_str(),
            action_text(envelope.action),
            KIMI_LC_ROLE_POLICY_MAP,
            KIMI_LC_CLIENT_SERVICE_FACE,
            KIMI_LC_APPROVAL_POLICY,
            boundary_mode_text(boundary.mode()),
            input.mcp_bundle_digest(),
        ];
        let capability_projection_digest = lc_digest(PROFILE_DIGEST_SCHEMA, &profile_parts);

        // 2) 会话全投影摘要:profile 摘要 + 当前 role/permission/cwd/target/
        //    roots/trust/config/policy/authority + 实际 tool policy(恒 None
        //    的 canonical 形态)。分层包含 profile 摘要(先算 profile 再算
        //    session,单向无递归)。
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
            KIMI_LC_APPROVAL_POLICY,
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
            ProviderRefType::KimiCode,
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::KimiAcp,
            self.exact_version.clone(),
            envelope.action,
            input.role().clone(),
            input.permission_mode().clone(),
            // Kimi 通用 tool policy 恒 None(角色控制由 ClientServicePolicy
            // 承担,不进通用 tool policy 字段)。
            None,
            KIMI_LC_APPROVAL_POLICY.to_string(),
            boundary_mode_text(boundary.mode()).to_string(),
            envelope.working_directory.clone(),
            // kimi 单 cwd(Task 4 Interfaces:ACP cwd 保持 root):进程 cwd
            // 与协议 cwd 同为 canonical LC root;target 写面经 boundary plan
            // 由宿主 fs/terminal handler 消费,不进协议 cwd(codex 的双 cwd
            // 分离不适用于此)。
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
