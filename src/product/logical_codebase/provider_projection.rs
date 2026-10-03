//! Provider policy projection 的纯 DTO 与 projector 合同(Task 1a,REQ-LCG-03)。
//!
//! 投影(projection)是 gateway 从不可由调用方伪造的输入出发,为某次
//! provider/action 会话冻结的完整权限画像:provider/dialect/wire、exact
//! version、role、permission/tool/approval/sandbox、cwd/target/roots、
//! trust/config/MCP digest 与 boundary 证据引用。adapter 只能消费 projection,
//! 不能自造或改写;`ProviderProjectionInput` 字段私有、只能由 gateway 构造
//! (`pub(crate)` 构造函数),是该不可伪造性的承载点。
//!
//! 职责边界(计划冻结接口):真实 provider projector 实现归 Task 4/5
//! (批次 D);digest 计算(固定字段序+长度分隔+schema 前缀)与 gateway
//! 装配校验归 1b/1c;本文件只冻结 trait 与 DTO 形状。

use std::path::{Path, PathBuf};

use crate::cross_cutting::provider_boundary::ProviderBoundaryPlan;
use crate::cross_cutting::streaming_provider::{ProviderPermissionMode, ProviderToolPolicy};
use crate::product::logical_codebase::policy::{
    PolicyTarget, ProviderDialect, ProviderWireDialect, SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_gateway::{ProviderRef, ProviderRefType};
use crate::protocol::contracts::AdapterRole;

/// gateway 构造的投影输入(Task 1a 冻结承载)。字段私有:只能由 gateway
/// 经 `new`(`pub(crate)`)装配,provider、调用方与外部测试均不能自造投影
/// 输入。携带 envelope、provider/action、role、permission/tool/MCP/config/
/// trust/boundary 输入;完整 capability action row 的桥接随 Task 2 形状
/// 落地后由消费方扩展,1a 先冻结承载合同。
#[derive(Debug, Clone)]
pub struct ProviderProjectionInput {
    envelope: SessionPolicyEnvelope,
    provider: ProviderRef,
    action: SessionPolicyAction,
    role: AdapterRole,
    permission_mode: ProviderPermissionMode,
    tool_policy: Option<ProviderToolPolicy>,
    approval_policy: String,
    mcp_bundle_digest: String,
    config_artifact_ref: String,
    trust_digest: String,
    boundary: Option<ProviderBoundaryPlan>,
}

impl ProviderProjectionInput {
    /// gateway 专用构造(同 crate;外部不可构造)。
    pub(crate) fn new(
        envelope: SessionPolicyEnvelope,
        provider: ProviderRef,
        action: SessionPolicyAction,
        role: AdapterRole,
        permission_mode: ProviderPermissionMode,
        tool_policy: Option<ProviderToolPolicy>,
        approval_policy: String,
        mcp_bundle_digest: String,
        config_artifact_ref: String,
        trust_digest: String,
        boundary: Option<ProviderBoundaryPlan>,
    ) -> Self {
        Self {
            envelope,
            provider,
            action,
            role,
            permission_mode,
            tool_policy,
            approval_policy,
            mcp_bundle_digest,
            config_artifact_ref,
            trust_digest,
            boundary,
        }
    }

    /// 只读访问(crate 内 projector/gateway 消费)。
    pub(crate) fn envelope(&self) -> &SessionPolicyEnvelope {
        &self.envelope
    }

    pub(crate) fn provider(&self) -> &ProviderRef {
        &self.provider
    }

    pub(crate) fn action(&self) -> SessionPolicyAction {
        self.action
    }

    pub(crate) fn role(&self) -> &AdapterRole {
        &self.role
    }

    pub(crate) fn permission_mode(&self) -> &ProviderPermissionMode {
        &self.permission_mode
    }

    pub(crate) fn tool_policy(&self) -> Option<&ProviderToolPolicy> {
        self.tool_policy.as_ref()
    }

    pub(crate) fn approval_policy(&self) -> &str {
        &self.approval_policy
    }

    pub(crate) fn mcp_bundle_digest(&self) -> &str {
        &self.mcp_bundle_digest
    }

    pub(crate) fn config_artifact_ref(&self) -> &str {
        &self.config_artifact_ref
    }

    pub(crate) fn trust_digest(&self) -> &str {
        &self.trust_digest
    }

    pub(crate) fn boundary(&self) -> Option<&ProviderBoundaryPlan> {
        self.boundary.as_ref()
    }
}

/// provider policy projector 合同:把 gateway 构造的输入投影为某次会话的
/// 固定权限画像。实现归 provider projector owner(Task 4/5);registry 以
/// `Arc<dyn ProviderPolicyProjector>` 原子保存。
pub trait ProviderPolicyProjector: Send + Sync {
    /// 计算投影。输入由 gateway 构造;投影字段固定只读,消费方只能经
    /// `pub(crate)` getter 读取。
    fn project(
        &self,
        input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError>;
}

/// 投影结果:固定只读字段集(计划冻结列表逐字):provider_type/
/// provider_dialect/wire_dialect/exact_version/action/role/permission_mode/
/// tool_policy/approval_policy/sandbox/working_directory/
/// protocol_working_directory/target/readable_roots/writable_roots/
/// trust_digest/config_digest/mcp_bundle_digest/boundary_evidence_ref/
/// capability_projection_digest/projection_digest。
///
/// 字段私有+`pub(crate)` 构造/getter:provider 不能自造或改写;digest 的
/// 计算(固定字段序+长度分隔+schema 前缀,禁止 Debug 文本或调用方摘要)
/// 归 projector 实现与 gateway 校验(Task 4/5、1b)。
#[derive(Debug, Clone)]
pub struct ProviderPolicyProjection {
    provider_type: ProviderRefType,
    provider_dialect: ProviderDialect,
    wire_dialect: ProviderWireDialect,
    exact_version: String,
    action: SessionPolicyAction,
    role: AdapterRole,
    permission_mode: ProviderPermissionMode,
    tool_policy: Option<ProviderToolPolicy>,
    approval_policy: String,
    sandbox: String,
    working_directory: PathBuf,
    protocol_working_directory: PathBuf,
    target: PolicyTarget,
    readable_roots: Vec<PathBuf>,
    writable_roots: Vec<PathBuf>,
    trust_digest: String,
    config_digest: String,
    mcp_bundle_digest: String,
    boundary_evidence_ref: String,
    capability_projection_digest: String,
    projection_digest: String,
}

impl ProviderPolicyProjection {
    /// projector owner(crate 内)构造:一次装配全部冻结字段。
    pub(crate) fn new(
        provider_type: ProviderRefType,
        provider_dialect: ProviderDialect,
        wire_dialect: ProviderWireDialect,
        exact_version: String,
        action: SessionPolicyAction,
        role: AdapterRole,
        permission_mode: ProviderPermissionMode,
        tool_policy: Option<ProviderToolPolicy>,
        approval_policy: String,
        sandbox: String,
        working_directory: PathBuf,
        protocol_working_directory: PathBuf,
        target: PolicyTarget,
        readable_roots: Vec<PathBuf>,
        writable_roots: Vec<PathBuf>,
        trust_digest: String,
        config_digest: String,
        mcp_bundle_digest: String,
        boundary_evidence_ref: String,
        capability_projection_digest: String,
        projection_digest: String,
    ) -> Self {
        Self {
            provider_type,
            provider_dialect,
            wire_dialect,
            exact_version,
            action,
            role,
            permission_mode,
            tool_policy,
            approval_policy,
            sandbox,
            working_directory,
            protocol_working_directory,
            target,
            readable_roots,
            writable_roots,
            trust_digest,
            config_digest,
            mcp_bundle_digest,
            boundary_evidence_ref,
            capability_projection_digest,
            projection_digest,
        }
    }

    /// 以下为等价 `pub(crate)` getter(计划冻结:提供 View 或等价
    /// `pub(crate)` getter;validated launch 也只提供 `pub(crate)`
    /// boundary/projection accessor)。
    pub(crate) fn provider_type(&self) -> ProviderRefType {
        self.provider_type
    }

    pub(crate) fn provider_dialect(&self) -> ProviderDialect {
        self.provider_dialect
    }

    pub(crate) fn wire_dialect(&self) -> ProviderWireDialect {
        self.wire_dialect
    }

    pub(crate) fn exact_version(&self) -> &str {
        &self.exact_version
    }

    pub(crate) fn action(&self) -> SessionPolicyAction {
        self.action
    }

    pub(crate) fn role(&self) -> &AdapterRole {
        &self.role
    }

    pub(crate) fn permission_mode(&self) -> &ProviderPermissionMode {
        &self.permission_mode
    }

    pub(crate) fn tool_policy(&self) -> Option<&ProviderToolPolicy> {
        self.tool_policy.as_ref()
    }

    pub(crate) fn approval_policy(&self) -> &str {
        &self.approval_policy
    }

    pub(crate) fn sandbox(&self) -> &str {
        &self.sandbox
    }

    pub(crate) fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub(crate) fn protocol_working_directory(&self) -> &Path {
        &self.protocol_working_directory
    }

    pub(crate) fn target(&self) -> &PolicyTarget {
        &self.target
    }

    pub(crate) fn readable_roots(&self) -> &[PathBuf] {
        &self.readable_roots
    }

    pub(crate) fn writable_roots(&self) -> &[PathBuf] {
        &self.writable_roots
    }

    pub(crate) fn trust_digest(&self) -> &str {
        &self.trust_digest
    }

    pub(crate) fn config_digest(&self) -> &str {
        &self.config_digest
    }

    pub(crate) fn mcp_bundle_digest(&self) -> &str {
        &self.mcp_bundle_digest
    }

    pub(crate) fn boundary_evidence_ref(&self) -> &str {
        &self.boundary_evidence_ref
    }

    pub(crate) fn capability_projection_digest(&self) -> &str {
        &self.capability_projection_digest
    }

    pub(crate) fn projection_digest(&self) -> &str {
        &self.projection_digest
    }
}

/// 投影失败:不可伪造输入校验失败、provider 不支持该 action/role 的投影,
/// 或投影材料缺失时 fail-closed。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderProjectionError {
    /// 该 provider 尚未接入 projector(或投影能力未交付),一律拒绝。
    #[error("provider_projection_unsupported: {0}")]
    Unsupported(String),
    /// 投影输入不合法(缺字段/角色不匹配等)。
    #[error("provider_projection_invalid: {0}")]
    Invalid(String),
}

/// 未接入 projector 的显式拒绝实现:对一切 `project` 调用返回
/// `ProviderProjectionError::Unsupported`。
///
/// 1a 合同期占位(裁决 A1):`ProviderRegistry::register_gated` 冻结为
/// (name, adapter, projector, gate) 四参,而真实 provider projector 归
/// Task 4/5 交付;在此之前,生产装配(`state.rs::real_provider_registry`)
/// 以本类型占第四参,行为=显式拒绝(与 `start_validated`/`run_validated`
/// 默认 unsupported 只供未接入 adapter 拒绝同构)。1c-factory 落真实
/// projector 时整体替换,不留占位。
#[derive(Debug, Clone, Copy, Default)]
pub struct UnprovisionedProviderPolicyProjector;

impl ProviderPolicyProjector for UnprovisionedProviderPolicyProjector {
    fn project(
        &self,
        _input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        Err(ProviderProjectionError::Unsupported(
            "provider policy projector 未接入(1a 合同期占位,等待 Task 4/5 真实 projector)"
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::policy::AggregatePolicyArtifact;

    fn projection_input_fixture() -> ProviderProjectionInput {
        let artifact = AggregatePolicyArtifact::bootstrap(
            "project_0001",
            "logical_0001",
            "2026-10-03T00:00:00Z".to_string(),
        );
        let envelope = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::CodingTargetWrite,
            PolicyTarget::checkout("logical_repo", "checkout_1", "/work/api/.worktrees/issue_1"),
            PathBuf::from("/lc-root"),
            vec![PathBuf::from("/aggregate")],
            vec![PathBuf::from("/work/api/.worktrees/issue_1")],
            ProviderDialect::KimiAcpV1,
            "sha256:settings".to_string(),
            "2026-10-03T00:00:00Z".to_string(),
            PathBuf::from("/lc-root"),
        )
        .expect("envelope");
        ProviderProjectionInput::new(
            envelope,
            ProviderRef::kimi_code("cap_managed_snapshot"),
            SessionPolicyAction::CodingTargetWrite,
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            None,
            "never".to_string(),
            "sha256:mcp".to_string(),
            "sha256:settings".to_string(),
            "sha256:trust".to_string(),
            None,
        )
    }

    fn projection_fixture() -> ProviderPolicyProjection {
        ProviderPolicyProjection::new(
            ProviderRefType::KimiCode,
            ProviderDialect::KimiAcpV1,
            ProviderWireDialect::KimiAcp,
            "0.34.0".to_string(),
            SessionPolicyAction::CodingTargetWrite,
            AdapterRole::Executor,
            ProviderPermissionMode::Auto,
            None,
            "never".to_string(),
            "client-service".to_string(),
            PathBuf::from("/lc-root"),
            PathBuf::from("/work/api/.worktrees/issue_1"),
            PolicyTarget::checkout("logical_repo", "checkout_1", "/work/api/.worktrees/issue_1"),
            vec![PathBuf::from("/aggregate")],
            vec![PathBuf::from("/work/api/.worktrees/issue_1")],
            "sha256:trust".to_string(),
            "sha256:config".to_string(),
            "sha256:mcp".to_string(),
            "probe://boundary/1".to_string(),
            "sha256:capability-profile".to_string(),
            "sha256:session-projection".to_string(),
        )
    }

    /// Input getter 往返与构造不可外部伪造(同 crate 构造点唯一)。
    #[test]
    fn provider_projection_input_getters_round_trip() {
        let input = projection_input_fixture();
        assert_eq!(input.provider().provider_type, ProviderRefType::KimiCode);
        assert_eq!(input.action(), SessionPolicyAction::CodingTargetWrite);
        assert_eq!(input.role(), &AdapterRole::Executor);
        assert_eq!(input.approval_policy(), "never");
        assert_eq!(input.permission_mode(), &ProviderPermissionMode::Auto,);
        assert_eq!(input.config_artifact_ref(), "sha256:settings");
        assert_eq!(input.mcp_bundle_digest(), "sha256:mcp");
        assert_eq!(input.trust_digest(), "sha256:trust");
        assert_eq!(input.envelope().authority_root, PathBuf::from("/lc-root"));
        assert!(input.boundary().is_none());
        assert!(input.tool_policy().is_none());
    }

    /// Projection 冻结字段 getter 全量往返(21 字段逐项)。
    #[test]
    fn provider_policy_projection_freezes_all_readonly_fields() {
        let projection = projection_fixture();
        assert_eq!(projection.provider_type(), ProviderRefType::KimiCode);
        assert_eq!(projection.provider_dialect(), ProviderDialect::KimiAcpV1);
        assert_eq!(projection.wire_dialect(), ProviderWireDialect::KimiAcp);
        assert_eq!(projection.exact_version(), "0.34.0");
        assert_eq!(projection.action(), SessionPolicyAction::CodingTargetWrite);
        assert_eq!(projection.role(), &AdapterRole::Executor);
        assert_eq!(projection.permission_mode(), &ProviderPermissionMode::Auto,);
        assert_eq!(projection.tool_policy(), None);
        assert_eq!(projection.approval_policy(), "never");
        assert_eq!(projection.sandbox(), "client-service");
        assert_eq!(
            projection.working_directory(),
            std::path::Path::new("/lc-root")
        );
        assert_eq!(
            projection.protocol_working_directory(),
            std::path::Path::new("/work/api/.worktrees/issue_1")
        );
        assert_eq!(projection.target().checkout_id, "checkout_1");
        assert_eq!(projection.readable_roots().len(), 1);
        assert_eq!(projection.writable_roots().len(), 1);
        assert_eq!(projection.trust_digest(), "sha256:trust");
        assert_eq!(projection.config_digest(), "sha256:config");
        assert_eq!(projection.mcp_bundle_digest(), "sha256:mcp");
        assert_eq!(projection.boundary_evidence_ref(), "probe://boundary/1");
        assert_eq!(
            projection.capability_projection_digest(),
            "sha256:capability-profile"
        );
        assert_eq!(projection.projection_digest(), "sha256:session-projection");
    }

    /// Unprovisioned 占位:对任何输入显式拒绝(1c-factory 前的生产行为)。
    #[test]
    fn unprovisioned_projector_rejects_every_projection_request() {
        let projector = UnprovisionedProviderPolicyProjector;
        let result = projector.project(&projection_input_fixture());
        assert!(matches!(
            result,
            Err(ProviderProjectionError::Unsupported(_))
        ));
    }
}
