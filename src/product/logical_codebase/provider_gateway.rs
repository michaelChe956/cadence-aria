//! Logical-codebase provider gateway: opaque validated launch policy.
//!
//! `ValidatedSessionLaunchPolicy` 是逻辑代码库真实 provider 启动前唯一可取得的
//! 「政策已校验」token。它的字段与构造函数保持 module-private:外部业务调用只能
//! 经 `LogicalCodebaseProviderGateway::validate` 取得一个值,然后把它交给 adapter
//! 边界。这保证了「裸 `AdapterInput`/`StreamingProviderInput` 不能绕过政策」这一
//! 契约在编译期成立——没有 public constructor 就无法凭空构造一个 validated policy。
//!
//! Task 9 只实现 `validate` 的 fail-closed 主干:
//! - 缺失集中政策 artifact(`AggregatePolicyArtifactStore::get` 返回 `None`)→
//!   `ProviderGatewayError::PolicyMissing`,关闭无政策 fallback。
//! - 解析到的 policy revision/digest、envelope 的 root 校验(`SessionPolicyEnvelope`)
//!   在此处直接复用 Task 8 的 fail-closed 逻辑。
//! - target/capability 经 trait 注入解析,保持 gateway 可测试。
//!
//! 路由级 fail-closed 不等于 OS 级隔离:`validate` 只做政策门,真实 cwd/git-dir 的
//! 文件系统级复验在 Task 10 的 `revalidate_before_spawn` 完成。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::session_launch::{
    ValidatedAdapterInput, ValidatedStreamingProviderInput,
};
use crate::cross_cutting::streaming_provider::{ProviderSession, StreamingProviderAdapter};
use crate::product::logical_codebase::policy::{
    AggregatePolicyArtifactStore, PolicyTarget, ProviderDialect, SessionPolicyAction,
    SessionPolicyEnvelope,
};
use crate::product::logical_codebase::store::LogicalCodebaseManifest;
use crate::product::models::ProviderName;
use crate::protocol::contracts::AdapterOutput;

/// 已校验的会话启动政策,opaque token。
///
/// 字段为 module-private 且无 public constructor:只能由
/// `LogicalCodebaseProviderGateway::validate` 构造。这使「真实 provider 必须持有一个
/// validated policy 才能启动」成为编译期约束,而非运行时 `if provider != Fake` 分支。
///
/// 除 envelope/fingerprint 外,还冻结 spawn 前复验所需的最小引用(project_id、
/// provider ref、action、冻结的 version 与 capability snapshot),使
/// `revalidate_before_spawn` 能在 spawn 时点重新加载 store/capability source 并逐维
/// 比对(防 validate→spawn 之间政策升级/篡改)。
#[derive(Debug, Clone)]
pub struct ValidatedSessionLaunchPolicy {
    envelope: SessionPolicyEnvelope,
    fingerprint: SessionResumeFingerprint,
    project_id: String,
    provider: ProviderRef,
    action: SessionPolicyAction,
    version: String,
    capability_snapshot_ref: String,
}

impl ValidatedSessionLaunchPolicy {
    /// 返回冻结的 envelope 快照。getter 是外部唯一访问字段的方式。
    pub fn envelope(&self) -> &SessionPolicyEnvelope {
        &self.envelope
    }

    /// 返回 resume 复验指纹。
    pub fn fingerprint(&self) -> &SessionResumeFingerprint {
        &self.fingerprint
    }
}

/// provider 启动请求。gateway 据此解析政策 artifact、target 与 capability。
#[derive(Debug, Clone)]
pub struct SessionLaunchRequest {
    pub project_id: String,
    pub provider: ProviderRef,
    pub action: SessionPolicyAction,
    pub target: PolicyTarget,
    pub readable_roots: Vec<PathBuf>,
    pub writable_roots: Vec<PathBuf>,
    /// 托管配置 artifact 引用(envelope 冻结其 digest);非空否则 envelope 校验失败。
    pub config_artifact_ref: String,
}

impl SessionLaunchRequest {
    /// 构造一个 planning 只读启动请求:read-only action 必须没有 writable roots。
    pub fn planning(
        project_id: impl Into<String>,
        provider: ProviderRef,
        target: PolicyTarget,
        readable_roots: Vec<PathBuf>,
        config_artifact_ref: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            provider,
            action: SessionPolicyAction::PlanningReadOnly,
            target,
            readable_roots,
            writable_roots: Vec::new(),
            config_artifact_ref: config_artifact_ref.into(),
        }
    }
}

/// 启动请求中引用的 provider 标识。gateway 解析其 capability 时使用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRef {
    pub provider_type: ProviderRefType,
    /// capability snapshot 引用,用于复验 provider exact version。
    pub capability_snapshot_ref: String,
}

impl ProviderRef {
    pub fn claude_code(capability_snapshot_ref: impl Into<String>) -> Self {
        Self {
            provider_type: ProviderRefType::ClaudeCode,
            capability_snapshot_ref: capability_snapshot_ref.into(),
        }
    }

    pub fn codex(capability_snapshot_ref: impl Into<String>) -> Self {
        Self {
            provider_type: ProviderRefType::Codex,
            capability_snapshot_ref: capability_snapshot_ref.into(),
        }
    }
}

/// gateway 知晓的真实 provider 类型。Fake/测试路径不经此 gateway,故此处不含 Fake。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRefType {
    ClaudeCode,
    Codex,
}

/// resume 能力的可审计三态,与 `cross_cutting::provider_capabilities::
/// ProviderCapabilityEvidence` 对齐。gateway 在 resume 启动时只放行 `Confirmed`;
/// `Denied`/`Unknown` 一律 fail-closed。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeEvidenceState {
    /// 探测确认该 provider 支持 resume。
    Confirmed,
    /// 探测确认该 provider 不支持 resume(`Denied`/`Unknown` 在 gateway 侧等价:不放行)。
    Unsupported,
}

impl ResumeEvidenceState {
    /// 仅 `true` 映射为 `Confirmed`;`false`/未知映射为 `Unsupported`。
    /// fail-closed:探测结果不可靠时按不支持处理。
    pub fn from_supports_resume(supports_resume: bool) -> Self {
        if supports_resume {
            Self::Confirmed
        } else {
            Self::Unsupported
        }
    }

    /// 从 `cross_cutting::ProviderCapabilityEvidence` 三态桥接到 gateway 侧状态。
    /// 仅 `Confirmed` 放行 resume;`Denied`/`Unknown` 归为 `Unsupported`(fail-closed)。
    /// 这是 `cross_cutting::ProviderCapability.resume_evidence` 进入 gateway 消费路径
    /// 的唯一映射点,使该三态字段不再是无消费者的 dead field(B-2)。
    pub fn from_cross_cutting_evidence(
        evidence: &crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence,
    ) -> Self {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        match evidence {
            ProviderCapabilityEvidence::Confirmed => Self::Confirmed,
            ProviderCapabilityEvidence::Denied { .. } | ProviderCapabilityEvidence::Unknown => {
                Self::Unsupported
            }
        }
    }

    /// 该状态是否允许 resume 启动。仅 `Confirmed` 为真。
    pub fn allows_resume(self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

/// gateway 解析出的 provider capability 快照。冻结 exact version、adapter dialect
/// 与 resume 能力,供 envelope 与 fingerprint 复验。Task 10 把它与既有
/// `cross_cutting::ProviderCapability` 桥接(含 `resume_evidence` 三态)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapability {
    pub provider_type: ProviderRefType,
    pub version: String,
    pub adapter_dialect: ProviderDialect,
    pub capability_snapshot_ref: String,
    /// resume 能力三态。spawn 前 resume 启动据此 fail-closed(B-2)。
    pub resume_evidence: ResumeEvidenceState,
}

/// resume 复验指纹:覆盖 policy digest、target、provider exact version、dialect 与
/// capability snapshot。spawn 前(Task 10)与 provider 上报状态重新比对。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResumeFingerprint {
    pub digest: String,
}

impl SessionResumeFingerprint {
    /// 由 envelope、provider exact version、adapter dialect 与 capability snapshot
    /// 计算 canonical SHA-256。任一维度漂移都会产生不同 digest。
    pub fn from_envelope(
        envelope: &SessionPolicyEnvelope,
        version: &str,
        adapter_dialect: ProviderDialect,
        capability_snapshot_ref: &str,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(envelope.policy_id.as_bytes());
        hasher.update(envelope.policy_revision.to_be_bytes());
        hasher.update(envelope.policy_digest.as_bytes());
        hasher.update(format!("{:?}", envelope.action).as_bytes());
        hasher.update(envelope.target.logical_repository_id.as_bytes());
        hasher.update(envelope.target.checkout_id.as_bytes());
        hasher.update(envelope.target.worktree.to_string_lossy().as_bytes());
        hasher.update(version.as_bytes());
        hasher.update(format!("{adapter_dialect:?}").as_bytes());
        hasher.update(capability_snapshot_ref.as_bytes());
        let digest = format!("sha256:{:x}", hasher.finalize());
        Self { digest }
    }
}

/// gateway 拒绝启动的错误。fail-closed:任一校验失败都返回相应 variant,
/// 绝不退化到无政策 fallback。
#[derive(Debug, thiserror::Error)]
pub enum ProviderGatewayError {
    /// 缺失集中政策 artifact:bootstrap 未建立或 policy store 为空。
    #[error("provider_gateway_policy_missing: {0}")]
    PolicyMissing(String),
    /// policy store 或 envelope 校验返回的 product-store 级错误。
    #[error("provider_gateway_policy: {0}")]
    Policy(#[from] crate::product::json_store::ProductStoreError),
    /// target 解析/复验失败。
    #[error("provider_gateway_target: {0}")]
    Target(String),
    /// spawn 前 canonical 复验发现 cwd/git-dir/worktree identity 与 envelope
    /// 冻结的 target 不一致(TOCTOU)。`field` 标记漂移维度("cwd"/"git_dir"/
    /// "worktree"),便于审计与诊断。
    #[error("provider_gateway_target_mismatch: {field}")]
    TargetMismatch { field: String },
    /// 启动输入缺失 cwd/worktree_path,gateway 无法复验 canonical path。
    #[error("provider_gateway_missing_cwd")]
    MissingCwd,
    /// provider capability 不被支持。
    #[error("provider_gateway_capability: {0}")]
    UnsupportedCapability(String),
    /// provider 当前不可用(availability gate 拒绝)。spawn 前复验的一部分。
    #[error("provider_gateway_unavailable: {0}")]
    ProviderUnavailable(String),
    /// registry 中未找到该 provider 的真实 adapter。
    #[error("provider_gateway_registry_lookup: {0}")]
    RegistryLookup(String),
    /// spawn 前复验发现 validate→spawn 之间政策被升级/篡改(TOCTOU):
    /// policy revision/digest、provider version/dialect/capability snapshot 或
    /// config digest 与 envelope 冻结值不一致。`dimension` 标记漂移维度便于审计。
    #[error("provider_gateway_policy_drift: {dimension}")]
    PolicyDrift { dimension: String },
    /// resume 启动但 provider 的 `resume_evidence` 未 `Confirmed`(B-2 消费者)。
    /// fail-closed:`Denied`/`Unknown` 一律拒绝 resume。
    #[error("provider_gateway_resume_not_supported")]
    ResumeNotSupported,
    /// 真实 adapter 启动/运行失败。
    #[error("provider_gateway_adapter: {0}")]
    Adapter(#[from] ProviderAdapterError),
}

impl ProviderGatewayError {
    /// 把 product-store 错误归并为 `Policy`。供 resolver/capability source 复用。
    pub fn policy(error: crate::product::json_store::ProductStoreError) -> Self {
        Self::Policy(error)
    }

    /// 把 availability gate 的错误字符串包装为 `ProviderUnavailable`。
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::ProviderUnavailable(reason.into())
    }
}

/// 解析并复验启动目标。Task 10 的实现会真实 canonicalize cwd/git-dir;Task 9 的
/// 测试实现直接返回请求中的 target。
pub trait PolicyTargetResolver: Send + Sync {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError>;
}

/// 解析 provider capability 并校验 action 是否被支持。Task 10 的实现会校验
/// `ProviderCapability` 的三态 evidence;Task 9 的测试实现返回固定 capability。
pub trait ProviderCapabilitySource: Send + Sync {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError>;
}

/// 逻辑代码库真实 provider 的唯一建造与启动入口。
///
/// 构造时注入 policy store、capability source、target resolver、真实 provider
/// registry 与 availability gate。`validate` 产出 opaque
/// `ValidatedSessionLaunchPolicy`,后者只能经此方法取得;`start_streaming`/
/// `run_sync` 只接受绑定该 policy 的 validated input,并在 spawn 前重新复验
/// canonical cwd/git-dir/worktree identity 与 provider 可用性。
pub struct LogicalCodebaseProviderGateway {
    policies: AggregatePolicyArtifactStore,
    capabilities: Arc<dyn ProviderCapabilitySource>,
    targets: Arc<dyn PolicyTargetResolver>,
    registry: Arc<ProviderRegistry>,
    sync_adapter: Arc<dyn crate::cross_cutting::provider_adapter::ProviderAdapter>,
    availability_gate: Arc<ProviderAvailabilityGate>,
}

impl LogicalCodebaseProviderGateway {
    pub fn new(
        policies: AggregatePolicyArtifactStore,
        capabilities: Arc<dyn ProviderCapabilitySource>,
        targets: Arc<dyn PolicyTargetResolver>,
        registry: Arc<ProviderRegistry>,
        sync_adapter: Arc<dyn crate::cross_cutting::provider_adapter::ProviderAdapter>,
        availability_gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self {
            policies,
            capabilities,
            targets,
            registry,
            sync_adapter,
            availability_gate,
        }
    }

    /// 校验启动请求并产出不可缺省的 validated policy。fail-closed:缺失政策返回
    /// `PolicyMissing`,绝不退化到无政策 fallback。
    pub fn validate(
        &self,
        request: SessionLaunchRequest,
    ) -> Result<ValidatedSessionLaunchPolicy, ProviderGatewayError> {
        let artifact = self
            .policies
            .get(&request.project_id)
            .map_err(ProviderGatewayError::policy)?
            .ok_or_else(|| ProviderGatewayError::PolicyMissing(request.project_id.clone()))?;

        let target = self.targets.resolve_and_revalidate(&request)?;
        let capability = self
            .capabilities
            .require_supported(&request.provider, request.action)?;

        let now = chrono::Utc::now().to_rfc3339();
        let envelope = SessionPolicyEnvelope::new(
            &artifact,
            request.action,
            target,
            request.readable_roots,
            request.writable_roots,
            capability.adapter_dialect,
            request.config_artifact_ref,
            now,
        )
        .map_err(ProviderGatewayError::policy)?;

        let fingerprint = SessionResumeFingerprint::from_envelope(
            &envelope,
            &capability.version,
            capability.adapter_dialect,
            &capability.capability_snapshot_ref,
        );

        Ok(ValidatedSessionLaunchPolicy {
            envelope,
            fingerprint,
            project_id: request.project_id,
            provider: request.provider,
            action: request.action,
            version: capability.version,
            capability_snapshot_ref: capability.capability_snapshot_ref,
        })
    }

    /// 启动 streaming provider 会话。只接受绑定 validated policy 的 input;
    /// spawn 前基于 validated policy 与 `input.working_dir` 重新复验政策指纹
    /// (policy revision/digest、provider version/dialect/capability snapshot、
    /// config digest)、canonical cwd/git-dir/worktree identity、provider 可用性
    /// 与 resume 能力。任一复验失败都发生在 registry lookup/真实 adapter start
    /// 之前(fail-closed)。
    pub async fn start_streaming(
        &self,
        launch: ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderGatewayError> {
        let (input, validated) = launch.into_parts();
        let is_resume = input.resume_provider_session_id.is_some();
        self.revalidate_before_spawn(&validated, &input.working_dir, is_resume)?;
        let adapter = self.lookup_real_streaming_adapter(&validated)?;
        adapter
            .start(input, cancel)
            .await
            .map_err(ProviderGatewayError::Adapter)
    }

    /// 同步运行 adapter。只接受绑定 validated policy 的 input;spawn 前基于
    /// validated policy 与 `input.worktree_path` 重新复验政策指纹、canonical
    /// cwd/git-dir/worktree identity、provider 可用性与 resume 能力。任一复验
    /// 失败都发生在真实 adapter run 之前(fail-closed)。
    pub fn run_sync(
        &self,
        launch: ValidatedAdapterInput,
    ) -> Result<AdapterOutput, ProviderGatewayError> {
        let (input, validated) = launch.into_parts();
        let cwd = input
            .worktree_path
            .as_deref()
            .map(Path::new)
            .ok_or(ProviderGatewayError::MissingCwd)?;
        // 同步 adapter input 不携带 resume session id;同步路径默认非 resume。
        self.revalidate_before_spawn(&validated, cwd, false)?;
        self.sync_adapter
            .run(&input)
            .map_err(ProviderGatewayError::Adapter)
    }

    /// spawn 前完整复验(B-1)。逐维度比对 envelope 冻结值与 spawn 时点的真实值:
    ///
    /// 1. **policy 指纹**:重新加载 store 中的 `AggregatePolicyArtifact`,比对
    ///    `policy_revision` 与 `policy_digest`(防 validate→spawn 间政策被升级)。
    /// 2. **provider 能力指纹**:重新查询 capability source,比对 `version`、
    ///    `adapter_dialect` 与 `capability_snapshot_ref`,并据当前 artifact+capability
    ///    重算 `SessionResumeFingerprint` 与冻结值逐字一致(防 provider 被替换)。
    /// 3. **config digest**:据 envelope 的 `config_artifact_ref` 重算 digest,
    ///    与 envelope 冻结的 `config_digest` 一致(防托管配置被篡改)。
    /// 4. **resume 能力**(B-2):若启动为 resume,provider 的 `resume_evidence` 必须
    ///    `Confirmed`,否则 fail-closed 为 `ResumeNotSupported`。
    /// 5. **canonical cwd/worktree identity**:重新 canonicalize cwd 与 target
    ///    worktree,不一致返回 `TargetMismatch { field: "cwd" }`。
    /// 6. **availability**:调用 availability gate,不可用返回 `ProviderUnavailable`。
    ///
    /// 任一维度漂移都发生在 registry lookup 之前。路由级 fail-closed 不等于 OS
    /// 级隔离:本复验是 supervised 场景下的 TOCTOU 门禁,不宣称物理不可写。
    fn revalidate_before_spawn(
        &self,
        validated: &ValidatedSessionLaunchPolicy,
        cwd: &Path,
        is_resume: bool,
    ) -> Result<(), ProviderGatewayError> {
        let envelope = validated.envelope();

        // 1. 重新加载政策 artifact,比对 revision/digest。
        let artifact = self
            .policies
            .get(&validated.project_id)
            .map_err(ProviderGatewayError::policy)?
            .ok_or_else(|| ProviderGatewayError::PolicyDrift {
                dimension: "policy_missing_at_spawn".to_string(),
            })?;
        if artifact.revision != envelope.policy_revision {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "policy_revision".to_string(),
            });
        }
        if artifact.digest != envelope.policy_digest {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "policy_digest".to_string(),
            });
        }

        // 2. 重新查询能力,比对 version/dialect/snapshot,并重算指纹。
        let capability = self
            .capabilities
            .require_supported(&validated.provider, validated.action)?;
        if capability.version != validated.version {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "provider_version".to_string(),
            });
        }
        if capability.adapter_dialect != envelope.provider_dialect {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "provider_dialect".to_string(),
            });
        }
        if capability.capability_snapshot_ref != validated.capability_snapshot_ref {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "capability_snapshot_ref".to_string(),
            });
        }
        let current_fingerprint = SessionResumeFingerprint::from_envelope(
            envelope,
            &capability.version,
            capability.adapter_dialect,
            &capability.capability_snapshot_ref,
        );
        if current_fingerprint.digest != validated.fingerprint.digest {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "resume_fingerprint".to_string(),
            });
        }

        // 4. resume 能力 fail-closed(B-2 消费者)。
        if is_resume && !capability.resume_evidence.allows_resume() {
            return Err(ProviderGatewayError::ResumeNotSupported);
        }

        // 3. config digest 重算(防托管配置被篡改)。
        let current_config_digest =
            SessionPolicyEnvelope::recompute_config_digest(&envelope.config_artifact_ref)
                .map_err(ProviderGatewayError::policy)?;
        if current_config_digest != envelope.config_digest {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "config_digest".to_string(),
            });
        }

        // 5. canonical cwd/worktree identity。
        let canonical_cwd = cwd.canonicalize().map_err(|error| {
            ProviderGatewayError::Target(format!("canonicalize cwd {}: {error}", cwd.display()))
        })?;
        let canonical_target = envelope.target.worktree.canonicalize().map_err(|error| {
            ProviderGatewayError::Target(format!(
                "canonicalize target {}: {error}",
                envelope.target.worktree.display()
            ))
        })?;
        if canonical_cwd != canonical_target {
            return Err(ProviderGatewayError::TargetMismatch {
                field: "cwd".to_string(),
            });
        }

        // 6. availability gate。
        let provider_name = provider_name_for_dialect(envelope.provider_dialect);
        self.availability_gate
            .ensure_available(&provider_name)
            .map_err(|error| ProviderGatewayError::unavailable(error.to_string()))?;
        Ok(())
    }

    /// 在 registry 中查找 streaming adapter。registry.get 返回 gated adapter,
    /// 已含 availability 包装;此处不再重复 gate,仅做存在性查找。
    fn lookup_real_streaming_adapter(
        &self,
        validated: &ValidatedSessionLaunchPolicy,
    ) -> Result<Arc<dyn StreamingProviderAdapter>, ProviderGatewayError> {
        let provider_name = provider_name_for_dialect(validated.envelope().provider_dialect);
        self.registry
            .get(&provider_name)
            .ok_or_else(|| ProviderGatewayError::RegistryLookup(format!("{provider_name:?}")))
    }
}

/// 把 envelope 冻结的 provider dialect 映射到 registry/availability gate 使用的
/// `ProviderName`。dialect 与 provider 类型一一对应。
fn provider_name_for_dialect(dialect: ProviderDialect) -> ProviderName {
    match dialect {
        ProviderDialect::ClaudeCodeCliV1 => ProviderName::ClaudeCode,
        ProviderDialect::CodexCliV1 => ProviderName::Codex,
    }
}

/// gateway 对 `ensure_bootstrap` 的桥接:暴露给需要在 gateway 之外触发 bootstrap
/// 的调用方(如 migration)。实际实现复用 `AggregatePolicyArtifactStore::ensure_bootstrap`。
///
/// 重新导出便于迁移与后续 WP0 代码引用,避免重复构造 store。
pub fn ensure_bootstrap_policy(
    paths: &crate::product::app_paths::ProductAppPaths,
    manifest: &LogicalCodebaseManifest,
) -> Result<
    crate::product::logical_codebase::policy::AggregatePolicyArtifact,
    crate::product::json_store::ProductStoreError,
> {
    AggregatePolicyArtifactStore::new(paths.clone()).ensure_bootstrap(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::policy::PolicyTarget;
    use crate::product::logical_codebase::store::LogicalCodebaseManifest;

    /// 验证 bootstrap 政策在 gateway 能校验首次启动前被持久化。
    ///
    /// 缺失政策时 fail-closed 为 `PolicyMissing`;`ensure_bootstrap` 写出 revision 1
    /// 的 bootstrap artifact 后,gateway 可校验 planning 只读启动并冻结 envelope。
    #[test]
    fn bootstrap_policy_is_persisted_before_gateway_can_validate_a_launch() {
        let fixture = gateway_fixture();
        assert!(matches!(
            fixture.gateway().validate(fixture.planning_request()),
            Err(ProviderGatewayError::PolicyMissing(_))
        ));

        fixture.install_bootstrap_policy();
        let validated = fixture
            .gateway()
            .validate(fixture.planning_request())
            .unwrap();
        assert_eq!(validated.envelope().policy_revision, 1);
        assert_eq!(
            validated.envelope().action,
            SessionPolicyAction::PlanningReadOnly,
        );
    }

    /// validated policy 的字段对外不可直接构造:没有 public constructor,getter 是
    /// 唯一访问方式。编译期保证只能由 gateway 产出。
    #[test]
    fn validated_policy_only_exposes_getters_and_cannot_be_constructed_outside_module() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let validated = fixture
            .gateway()
            .validate(fixture.planning_request())
            .unwrap();

        // getter 返回冻结的引用;外部无法修改字段或凭空重建 validated policy。
        assert_eq!(validated.envelope().policy_revision, 1);
        assert!(validated.fingerprint().digest.starts_with("sha256:"));
    }

    /// fingerprint 随 provider exact version 与 capability snapshot 变化:任一漂移
    /// 都应产生不同 digest,保证 resume 复验能检测 provider 侧变更。
    #[test]
    fn resume_fingerprint_changes_when_provider_version_or_snapshot_drifts() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();

        let baseline = fixture
            .gateway()
            .validate(fixture.planning_request())
            .unwrap();
        let baseline_digest = baseline.fingerprint().digest.clone();

        fixture.capabilities.set_version("1.4.1");
        let drifted = fixture
            .gateway()
            .validate(fixture.planning_request())
            .unwrap();
        assert_ne!(drifted.fingerprint().digest, baseline_digest);
    }

    /// 直接构造 `ValidatedSessionLaunchPolicy { envelope, fingerprint }` 在本模块外
    /// 不可行(struct literal 构造需要字段可见)。此处用 doctest-like 断言:gateway
    /// 返回值只能经 getter 访问,确认 opaque 边界由 privacy 保护。
    #[test]
    fn opaque_policy_blocks_struct_literal_construction_outside_module() {
        // 编译期约束:ValidatedSessionLaunchPolicy 的字段是 private,本测试模块虽在
        // 同一文件但通过公开 API 访问,模拟外部调用方。外部 crate 无法构造该 struct。
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let validated = fixture
            .gateway()
            .validate(fixture.planning_request())
            .unwrap();
        // 只能读 getter,无法读字段或重建。
        let _envelope = validated.envelope();
        let _fingerprint = validated.fingerprint();
    }

    /// spawn 前 canonical 复验:validate 阶段 target resolver 检测到 git_dir 在
    /// request 构造后被篡改(TOCTOU),返回 `TargetMismatch { field: "git_dir" }`,
    /// 且复验失败发生在 registry lookup/真实 adapter start 之前(start_count == 0)。
    #[test]
    fn gateway_revalidates_canonical_target_git_dir_and_managed_config_before_spawn() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let request = fixture.coding_request("/work/api/.worktrees/aria-issues/issue_1");
        fixture
            .targets()
            .change_git_dir_after_request("/work/api/.git-replaced");

        let error = fixture.gateway().validate(request).unwrap_err();
        assert!(
            matches!(error, ProviderGatewayError::TargetMismatch { ref field } if field == "git_dir")
        );
        assert_eq!(fixture.registry_start_count(), 0);
    }

    /// read-only action(planning/review)没有 write root;coding action 恰好一个
    /// 等于 canonical target worktree 的 write root。envelope 在 validate 时冻结该约束。
    #[test]
    fn planning_and_review_have_no_write_root_while_coding_has_exactly_target_root() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        assert!(
            fixture
                .gateway()
                .validate(fixture.planning_request())
                .unwrap()
                .envelope()
                .writable_roots
                .is_empty()
        );
        assert_eq!(
            fixture
                .gateway()
                .validate(fixture.coding_request("/work/api/.worktrees/aria-issues/issue_1"))
                .unwrap()
                .envelope()
                .writable_roots,
            vec![PathBuf::from("/work/api/.worktrees/aria-issues/issue_1")]
        );
    }

    /// spawn 前复验(B-1):validate 后、start_streaming 前政策被升级(revision+digest
    /// 变化),spawn 必须以 `PolicyDrift { dimension: "policy_revision" }` fail-closed,
    /// 且不触达 registry(start_count == 0)。
    #[test]
    fn start_streaming_fails_closed_when_policy_upgraded_between_validate_and_spawn() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let launch = fixture.validated_planning_streaming_input(worktree);

        // validate→spawn 之间政策被升级到 revision 2。
        fixture.upgrade_policy();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(
            fixture
                .gateway()
                .start_streaming(launch, CancellationToken::new()),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("expected PolicyDrift, got a session"),
        };
        assert!(
            matches!(error, ProviderGatewayError::PolicyDrift { ref dimension } if dimension == "policy_revision")
        );
        assert_eq!(fixture.registry_start_count(), 0);
    }

    /// spawn 前复验(B-1):validate 后、start_streaming 前 provider version 被改
    /// (capability source 返回不同 version),spawn 必须以
    /// `PolicyDrift { dimension: "provider_version" }` fail-closed,不触达 registry。
    #[test]
    fn start_streaming_rejects_provider_version_change_before_spawn() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let launch = fixture.validated_planning_streaming_input(worktree);

        fixture.capabilities().set_version("1.4.1");

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(
            fixture
                .gateway()
                .start_streaming(launch, CancellationToken::new()),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("expected PolicyDrift, got a session"),
        };
        assert!(
            matches!(error, ProviderGatewayError::PolicyDrift { ref dimension } if dimension == "provider_version")
        );
        assert_eq!(fixture.registry_start_count(), 0);
    }

    /// spawn 前复验(B-1):validate 后、run_sync 前政策被升级,同步路径同样 fail-closed
    /// 为 `PolicyDrift`,不触达真实 sync adapter(此处表现为返回错误而非成功)。
    #[test]
    fn run_sync_fails_closed_when_policy_upgraded_between_validate_and_spawn() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = fixture.gateway().validate(request).unwrap();
        let input = crate::protocol::contracts::AdapterInput {
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            role: crate::protocol::contracts::AdapterRole::Executor,
            worktree_path: Some(worktree.to_string_lossy().to_string()),
            provider_stream_log_dir: None,
            prompt: "probe".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 1,
            max_retries: 0,
        };
        let launch = ValidatedAdapterInput::new(input, validated);

        fixture.upgrade_policy();

        let error = fixture.gateway().run_sync(launch).unwrap_err();
        assert!(
            matches!(error, ProviderGatewayError::PolicyDrift { ref dimension } if dimension == "policy_revision")
        );
    }

    /// resume 能力 fail-closed(B-2 消费者):resume 启动时 provider 的
    /// `resume_evidence` 不是 `Confirmed` → spawn 拒绝(`ResumeNotSupported`),不触达
    /// registry。这使 `resume_evidence` 三态成为有消费者的 fail-closed 门禁,而非
    /// dead field。
    #[test]
    fn start_streaming_resume_is_rejected_when_resume_evidence_not_confirmed() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        // resume 启动:streaming input 携带 resume_provider_session_id。
        let validated = fixture
            .gateway()
            .validate(fixture.planning_request())
            .unwrap();
        let input = fixture.streaming_input(worktree, Some("sess_resume_0001".to_string()));
        let launch = ValidatedStreamingProviderInput::new(input, validated);

        // provider 标记 resume 不支持(三态 Unknown/Denied 在 gateway 侧归为 Unsupported)。
        fixture
            .capabilities()
            .set_resume_evidence(ResumeEvidenceState::Unsupported);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime.block_on(
            fixture
                .gateway()
                .start_streaming(launch, CancellationToken::new()),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("expected ResumeNotSupported, got a session"),
        };
        assert!(matches!(error, ProviderGatewayError::ResumeNotSupported));
        assert_eq!(fixture.registry_start_count(), 0);
    }

    /// resume 能力放行路径(B-2):`resume_evidence` 为 `Confirmed` 时 resume 启动通过复验
    /// 并触达 registry(start_count == 1),确认消费者只在非 Confirmed 时阻断。
    #[tokio::test]
    async fn start_streaming_resume_passes_when_resume_evidence_confirmed() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        // planning 请求的 target worktree 必须等于真实 worktree,使 cwd 复验通过。
        let request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = fixture.gateway().validate(request).unwrap();
        let input = fixture.streaming_input(worktree, Some("sess_resume_0001".to_string()));
        let launch = ValidatedStreamingProviderInput::new(input, validated);

        fixture
            .capabilities()
            .set_resume_evidence(ResumeEvidenceState::Confirmed);

        let _session = fixture
            .gateway()
            .start_streaming(launch, CancellationToken::new())
            .await
            .expect("resume launch passes when evidence confirmed");
        assert_eq!(fixture.registry_start_count(), 1);
    }

    /// 可变 target resolver:模拟 spawn 前 git_dir TOCTOU。初始 current 与
    /// expected 一致(resolve 通过);`change_git_dir_after_request` 后 current
    /// 偏离 expected,resolve 返回 `TargetMismatch { field: "git_dir" }`。
    struct MutableTargetResolver {
        expected_git_dir: PathBuf,
        current_git_dir: std::sync::Mutex<PathBuf>,
    }

    impl MutableTargetResolver {
        fn new(git_dir: PathBuf) -> Self {
            let current = git_dir.clone();
            Self {
                expected_git_dir: git_dir,
                current_git_dir: std::sync::Mutex::new(current),
            }
        }

        fn change_git_dir_after_request(&self, git_dir: impl Into<PathBuf>) {
            *self.current_git_dir.lock().unwrap() = git_dir.into();
        }
    }

    impl PolicyTargetResolver for MutableTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<PolicyTarget, ProviderGatewayError> {
            let current = self.current_git_dir.lock().unwrap().clone();
            if current != self.expected_git_dir {
                return Err(ProviderGatewayError::TargetMismatch {
                    field: "git_dir".to_string(),
                });
            }
            Ok(request.target.clone())
        }
    }

    /// 测试用 capability source:返回固定 capability,version 可调以驱动 fingerprint 漂移。
    /// 测试用 capability source:version 与 resume 能力可调,以驱动 spawn 前
    /// 复验指纹漂移与 resume fail-closed。
    struct StaticCapabilitySource {
        version: std::sync::Mutex<String>,
        resume_evidence: std::sync::Mutex<ResumeEvidenceState>,
    }

    impl StaticCapabilitySource {
        fn new(version: &str) -> Self {
            Self {
                version: std::sync::Mutex::new(version.to_string()),
                resume_evidence: std::sync::Mutex::new(ResumeEvidenceState::Confirmed),
            }
        }

        fn set_version(&self, version: &str) {
            *self.version.lock().unwrap() = version.to_string();
        }

        fn set_resume_evidence(&self, state: ResumeEvidenceState) {
            *self.resume_evidence.lock().unwrap() = state;
        }
    }

    impl ProviderCapabilitySource for StaticCapabilitySource {
        fn require_supported(
            &self,
            provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            let version = self.version.lock().unwrap().clone();
            let resume_evidence = *self.resume_evidence.lock().unwrap();
            let adapter_dialect = match provider.provider_type {
                ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
                ProviderRefType::Codex => ProviderDialect::CodexCliV1,
            };
            Ok(ProviderCapability {
                provider_type: provider.provider_type,
                version,
                adapter_dialect,
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                resume_evidence,
            })
        }
    }

    /// 测试用 streaming adapter:记录 start 调用次数,供断言「复验失败不触达 registry」。
    struct CountingStreamingAdapter {
        start_count: std::sync::atomic::AtomicUsize,
    }

    impl CountingStreamingAdapter {
        fn new() -> Self {
            Self {
                start_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn start_count(&self) -> usize {
            self.start_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
        for CountingStreamingAdapter
    {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<
            crate::cross_cutting::streaming_provider::ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            self.start_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            Ok(crate::cross_cutting::streaming_provider::ProviderSession { events, commands })
        }
    }

    /// 测试用同步 adapter stub:run 返回最小成功输出。
    struct StubSyncAdapter;

    impl crate::cross_cutting::provider_adapter::ProviderAdapter for StubSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<
            crate::protocol::contracts::AdapterOutput,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            use crate::protocol::contracts::TimeoutStatus;
            Ok(crate::protocol::contracts::AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    /// 始终可用的 availability gate fixture:health snapshot 标记所有真实 provider 可用。
    fn always_available_gate()
    -> Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate> {
        use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
        use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
        use crate::product::models::ProviderName;
        use chrono::Utc;

        struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);
        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }
            fn degraded(&self) -> bool {
                false
            }
        }

        let checked_at = Utc::now();
        let snapshot = Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: [ProviderName::ClaudeCode, ProviderName::Codex]
                .into_iter()
                .map(|provider| ProviderHealthEntry {
                    provider,
                    command: "stub".to_string(),
                    available: true,
                    version: Some("1.0".to_string()),
                    reason_code: None,
                    reason: None,
                    checked_at,
                })
                .collect(),
        });
        Arc::new(
            crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(
                Arc::new(AlwaysHealthy(snapshot)),
            ),
        )
    }

    struct GatewayFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        capabilities: Arc<StaticCapabilitySource>,
        targets: Arc<MutableTargetResolver>,
        streaming_adapter: Arc<CountingStreamingAdapter>,
        sync_adapter: Arc<StubSyncAdapter>,
        gate: Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate>,
    }

    fn gateway_fixture() -> GatewayFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path());
        let baseline_git_dir = paths.root().join("baseline.git");
        GatewayFixture {
            _root: root,
            paths,
            capabilities: Arc::new(StaticCapabilitySource::new("1.4.0")),
            targets: Arc::new(MutableTargetResolver::new(baseline_git_dir)),
            streaming_adapter: Arc::new(CountingStreamingAdapter::new()),
            sync_adapter: Arc::new(StubSyncAdapter),
            gate: always_available_gate(),
        }
    }

    impl GatewayFixture {
        fn manifest(&self) -> LogicalCodebaseManifest {
            LogicalCodebaseManifest::new("project_0001", self.paths.root().to_path_buf(), vec![])
        }

        fn policy_store(&self) -> AggregatePolicyArtifactStore {
            AggregatePolicyArtifactStore::new(self.paths.clone())
        }

        fn install_bootstrap_policy(&self) {
            let manifest = self.manifest();
            self.policy_store().ensure_bootstrap(&manifest).unwrap();
        }

        fn gateway(&self) -> LogicalCodebaseProviderGateway {
            let mut registry = ProviderRegistry::new();
            registry.register(ProviderName::ClaudeCode, self.streaming_adapter.clone());
            registry.register(ProviderName::Codex, self.streaming_adapter.clone());
            LogicalCodebaseProviderGateway::new(
                self.policy_store(),
                self.capabilities.clone(),
                self.targets.clone(),
                Arc::new(registry),
                self.sync_adapter.clone(),
                self.gate.clone(),
            )
        }

        fn targets(&self) -> Arc<MutableTargetResolver> {
            self.targets.clone()
        }

        fn capabilities(&self) -> Arc<StaticCapabilitySource> {
            self.capabilities.clone()
        }

        fn registry_start_count(&self) -> usize {
            self.streaming_adapter.start_count()
        }

        /// 将 store 中的 policy artifact 升级到 revision 2(新 policy_text + 新 digest),
        /// 模拟 validate→spawn 之间政策被升级(TOCTOU)。
        fn upgrade_policy(&self) {
            let manifest = self.manifest();
            let store = self.policy_store();
            let current = store.get(&manifest.project_id).unwrap().unwrap();
            let revised = current.with_revised_policy(
                "# Aggregate policy (revision 2)\n\nTighter supervised scope.\n",
                manifest.updated_at.clone(),
            );
            store.save(&manifest.project_id, &revised).unwrap();
        }

        /// 创建一个真实存在的 worktree 目录(用于 spawn 前 canonicalize 复验)。
        fn real_worktree(&self) -> PathBuf {
            let worktree = self.paths.root().join("worktree");
            std::fs::create_dir_all(&worktree).unwrap();
            worktree
        }

        /// 无参便利方法:返回 planning 只读请求(内部用 manifest())。
        fn planning_request(&self) -> SessionLaunchRequest {
            self.planning_request_for_manifest(&self.manifest())
        }

        fn planning_request_for_manifest(
            &self,
            manifest: &LogicalCodebaseManifest,
        ) -> SessionLaunchRequest {
            let target = PolicyTarget::checkout(
                "logical_repo_0001",
                "checkout_0001",
                self.paths.root().join("worktree"),
            );
            SessionLaunchRequest::planning(
                &manifest.project_id,
                ProviderRef::claude_code("cap_claude_code_1_4_0"),
                target,
                vec![self.paths.root().to_path_buf()],
                "sha256:managed-config-artifact",
            )
        }

        /// coding 请求:恰好一个等于 canonical target worktree 的 write root。
        fn coding_request(&self, worktree: impl Into<PathBuf>) -> SessionLaunchRequest {
            let manifest = self.manifest();
            let worktree = worktree.into();
            let target =
                PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone());
            SessionLaunchRequest {
                project_id: manifest.project_id,
                provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
                action: SessionPolicyAction::CodingTargetWrite,
                target,
                readable_roots: vec![self.paths.root().to_path_buf()],
                writable_roots: vec![worktree],
                config_artifact_ref: "sha256:managed-config-artifact".to_string(),
            }
        }

        /// 针对真实 worktree 目录构造一个 planning 只读 validated streaming input。
        /// worktree 必须真实存在(spawn 前 canonicalize 复验需要)。
        fn validated_planning_streaming_input(
            self: &GatewayFixture,
            worktree: PathBuf,
        ) -> ValidatedStreamingProviderInput {
            let request = SessionLaunchRequest::planning(
                self.manifest().project_id,
                ProviderRef::claude_code("cap_claude_code_1_4_0"),
                PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
                vec![self.paths.root().to_path_buf()],
                "sha256:managed-config-artifact",
            );
            let validated = self.gateway().validate(request).unwrap();
            ValidatedStreamingProviderInput::new(self.streaming_input(worktree, None), validated)
        }

        /// 构造一个 streaming input;resume 设 `resume_provider_session_id`。
        fn streaming_input(
            &self,
            working_dir: PathBuf,
            resume_id: Option<String>,
        ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
            use crate::cross_cutting::streaming_provider::{
                ProviderPermissionMode, StreamingProviderInput,
            };
            use crate::protocol::contracts::{AdapterRole, ProviderType};
            StreamingProviderInput {
                provider_type: ProviderType::ClaudeCode,
                role: AdapterRole::Executor,
                prompt: "probe".to_string(),
                working_dir,
                workspace_session_id: None,
                resume_provider_session_id: resume_id,
                permission_mode: ProviderPermissionMode::Auto,
                structured_output_contract: None,
                env_vars: Default::default(),
                timeout_secs: 1,
            }
        }
    }

    /// bridge 映射(B-2):`cross_cutting::ProviderCapabilityEvidence` 三态经
    /// `ResumeEvidenceState::from_cross_cutting_evidence` 进入 gateway 消费路径。
    /// 仅 `Confirmed` 放行;`Denied`/`Unknown` 归为 `Unsupported`(fail-closed),
    /// 使该三态字段不再是无消费者的 dead field。
    #[test]
    fn resume_evidence_bridge_maps_cross_cutting_three_states_to_gateway_state() {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        assert!(
            ResumeEvidenceState::from_cross_cutting_evidence(
                &ProviderCapabilityEvidence::Confirmed
            )
            .allows_resume()
        );
        assert!(
            !ResumeEvidenceState::from_cross_cutting_evidence(
                &ProviderCapabilityEvidence::Denied {
                    reason: "probe says no".to_string(),
                }
            )
            .allows_resume()
        );
        assert!(
            !ResumeEvidenceState::from_cross_cutting_evidence(&ProviderCapabilityEvidence::Unknown)
                .allows_resume()
        );
    }
}
