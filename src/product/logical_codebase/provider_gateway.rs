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

/// resume 启动请求(Task 13):携带当前 launch 请求与旧会话冻结的 fingerprint
/// 及其 session id。gateway 据此在 `resume_or_start` 中判定 resume 还是 supersede。
#[derive(Debug, Clone)]
pub struct ResumeSessionLaunchRequest {
    /// 当前会话的 launch 请求(与 `validate` 入参同型)。
    pub launch: SessionLaunchRequest,
    /// 旧会话冻结时的 fingerprint,用于全维度比对。
    pub previous_fingerprint: SessionResumeFingerprint,
    /// 旧会话 id;supersede 时透传给调用方以清理旧会话状态。
    pub previous_session_id: String,
}

/// `resume_or_start` 的返回决策(Task 13)。
///
/// - `Resume`:fingerprint 全维度相等,旧会话可安全 resume,返回新的 validated
///   policy 供 spawn。
/// - `StartNew`:fingerprint 漂移(policy digest/target/version/dialect/capability
///   任一不一致),旧会话被 supersede(审计),返回新 validated policy 与被
///   supersede 的旧 session id。
#[derive(Debug)]
pub enum GatewaySessionDisposition {
    Resume(ValidatedSessionLaunchPolicy),
    StartNew {
        validated: ValidatedSessionLaunchPolicy,
        superseded_session_id: String,
    },
}

/// 配置来源类别(Task 13)。区分 Aria-owned(可注入)与非 Aria-owned(managed
/// settings 等需标注的已知 gap)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSourceKind {
    /// Aria-owned bundle:user/project/local settings 或 Aria 管理的 MCP,可注入。
    AriaOwnedBundle,
    /// 非 Aria-owned:如 provider 自带 managed settings,需标注为 gap。
    NonAriaOwned,
}

/// 配置来源 provenance(Task 13):记录 provider 实际加载的配置来源构成。解析
/// provider `/status` 中 `Setting sources` 时填充,作为 `ConfigSourceAudit` 的
/// 一部分入审计。`managed_settings_active` 绝不假装已覆盖——它只标注「检测到」
/// 并携带警告,是否放行由 policy(`enforce_config_source_policy`)决定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSourceProvenance {
    pub user_settings: bool,
    pub project_settings: bool,
    pub local_settings: bool,
    pub env_overrides: bool,
    pub managed_settings_active: bool,
    pub managed_settings_warning: Option<String>,
    pub mcp_sources: Vec<ConfigSourceKind>,
}

impl ConfigSourceProvenance {
    /// 据 provider `/status` 上报的 `Setting sources` 列表检测 provenance。
    /// `Managed` 源(非 Aria-owned)触发 `managed_settings_active=true` 与警告;
    /// 警告明确说明「检测到 managed settings」且「不假装已覆盖」。
    ///
    /// 仅识别已知来源 token(User/Project/Local/Env/Managed);未知 token 视为
    /// `NonAriaOwned` 的潜在 gap,但当前不额外触发 managed_settings_active
    /// (避免误报);调用方可后续扩展。
    pub fn detect_from_setting_sources(sources: &[&str]) -> Self {
        let lowercased: Vec<String> = sources
            .iter()
            .map(|source| source.trim().to_lowercase())
            .collect();
        let contains = |key: &str| lowercased.iter().any(|source| source == key);
        let managed_settings_active = contains("managed");
        let managed_settings_warning = if managed_settings_active {
            // 明确不使用 "overridden/覆盖" 字样:绝不假装已覆盖 managed settings。
            // 仅陈述「检测到」与「无法保证这些被压制」,是否放行由 policy 决定。
            Some(
                "detected non-Aria-owned managed settings active; \
                 gateway cannot guarantee these are suppressed; \
                 known gap, policy may reject startup"
                    .to_string(),
            )
        } else {
            None
        };
        Self {
            user_settings: contains("user"),
            project_settings: contains("project"),
            local_settings: contains("local"),
            env_overrides: contains("env") || contains("environment"),
            managed_settings_active,
            managed_settings_warning,
            mcp_sources: Vec::new(),
        }
    }

    /// 是否仅由 Aria-owned 来源构成(user/project/local/env 与所有 MCP 来源都是
    /// Aria-owned)。存在 managed settings 或非 Aria MCP 来源时返回 false。
    pub fn is_aria_owned_only(&self) -> bool {
        !self.managed_settings_active
            && self
                .mcp_sources
                .iter()
                .all(|source| *source == ConfigSourceKind::AriaOwnedBundle)
    }
}

/// 配置来源审计条目(Task 13):冻结启动时点 provider 实际加载的配置来源构成、
/// 最终 argv 与 config digest。与 envelope 的 config_digest 互补:envelope 保证
/// 托管配置 artifact 未被篡改,本审计记录 provider 实际生效的来源与命令行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSourceAudit {
    /// 最终传递给 provider 的 argv(命令 + 参数)。
    pub argv: Vec<String>,
    /// config artifact ref 的 canonical SHA-256(与 envelope.config_digest 同型)。
    pub config_digest: String,
    /// provider 实际加载的配置来源 provenance。
    pub provenance: ConfigSourceProvenance,
}

impl ConfigSourceAudit {
    /// 据 argv、config artifact ref 与 provenance 构造审计条目。config_digest
    /// 由 `config_artifact_ref` 计算(与 envelope 冻结值一致),禁止外部传任意
    /// digest。
    pub fn from_launch(
        argv: &[String],
        config_artifact_ref: &str,
        provenance: ConfigSourceProvenance,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(config_artifact_ref.as_bytes());
        let config_digest = format!("sha256:{:x}", hasher.finalize());
        Self {
            argv: argv.to_vec(),
            config_digest,
            provenance,
        }
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
    /// 配置来源审计发现 provider 实际加载了 managed settings(非 Aria-owned),
    /// 且 policy 配置为拒绝此类启动。**Task 11 起不再产生**:`enforce_config_source_policy`
    /// 改为在 `GatewayRunAudit` 追加 managed-settings 标注后放行,绝不假装已覆盖。
    /// variant 保留仅供稳定码表与防御映射;若未来 policy 重新收紧为拒绝可恢复。
    #[error("provider_gateway_managed_settings_active")]
    ManagedSettingsActive,
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

/// gateway 成功启动的审计条目。Task 11 把同步/流式 provider 全入口接线到
/// gateway,每条成功启动(sync 或 stream)都留下一份可核对的 policy digest、
/// config digest 与最终 argv,使「逻辑代码库真实 provider 调用是否经 gateway」
/// 可被断言,而非仅靠代码审查。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRunAuditEntry {
    /// 启动栈:`Sync` 为同步 adapter run(work item split),`Stream` 为流式
    /// provider start(planning/coding/review/aggregate initialization)。
    pub stack: GatewayRunStack,
    /// 该次启动冻结的 envelope policy digest(`SessionPolicyEnvelope::policy_digest`)。
    pub policy_digest: String,
    /// 该次启动冻结的 config artifact digest(与 envelope 冻结值同型)。启动路径无
    /// 真实 config artifact 引用时为 `None`(兼容既有构造;Task 11 起 gateway 启动
    /// 路径会从 `ConfigSourceAudit` 聚合出 `Some`)。
    pub config_digest: Option<String>,
    /// 最终传递给 provider 的 argv。启动输入无真实 argv 源时为空 Vec(Task 11 现状,
    /// 记录为空并保留字段以便后续接入 `/status` setting sources)。
    pub argv: Vec<String>,
}

/// 审计记录的启动栈类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayRunStack {
    Sync,
    Stream,
}

/// gateway 启动审计。线程安全,供 fixture 与未来生产侧观测在 gateway 之外检查
/// 启动计数与 policy digest 完整性。
#[derive(Debug, Default)]
pub struct GatewayRunAudit {
    entries: std::sync::Mutex<Vec<GatewayRunAuditEntry>>,
    /// resume 一致性审计(Task 13):记录被 supersede 的旧会话 id 与原因
    /// (如 `resume_fingerprint_mismatch`)。供外部断言 resume 漂移是否被正确
    /// 记录为 supersede 而非静默 resume。
    supersedes: std::sync::Mutex<Vec<GatewaySupersedeEntry>>,
    /// managed-settings 标注(Task 11):`enforce_config_source_policy` 检测到
    /// `managed_settings_active` 时不再 fail-closed,而是把携带 config digest 的
    /// 标注追加到这里,供外部断言「已标注已知 gap」而非静默忽略。绝不假装已覆盖。
    managed_settings_annotations: std::sync::Mutex<Vec<String>>,
}

/// resume 一致性审计条目:旧会话被 supersede 的记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewaySupersedeEntry {
    pub superseded_session_id: String,
    pub reason: String,
}

impl GatewayRunAudit {
    /// 构造一个空的审计记录。gateway 构造时默认注入一个独立实例;测试 fixture
    /// 通过 `shared()` 共享同一份审计,跨多次 gateway 构造累计。
    pub fn new() -> Self {
        Self::default()
    }

    fn record(
        &self,
        stack: GatewayRunStack,
        policy_digest: String,
        config_digest: Option<String>,
        argv: Vec<String>,
    ) {
        let mut entries = self.entries.lock().expect("gateway audit mutex poisoned");
        entries.push(GatewayRunAuditEntry {
            stack,
            policy_digest,
            config_digest,
            argv,
        });
    }

    /// 追加一条 managed-settings 标注(Task 11):记录检测到的非 Aria-owned managed
    /// settings 及其 config digest。该标注是「已检测、已记录」的已知 gap,不阻断
    /// 启动——是否放行由 policy 决定(当前默认放行并标注)。
    fn annotate_managed_settings_active(&self, config_digest: &str) {
        let mut annotations = self
            .managed_settings_annotations
            .lock()
            .expect("gateway audit mutex poisoned");
        annotations.push(format!(
            "detected non-Aria-owned managed settings active; \
             gateway cannot guarantee these are suppressed; \
             known gap annotated (not blocked); config_digest={config_digest}"
        ));
    }

    /// 返回已记录的 managed-settings 标注快照。空 Vec 表示尚未检测到。
    pub fn managed_settings_annotations(&self) -> Vec<String> {
        let annotations = self
            .managed_settings_annotations
            .lock()
            .expect("gateway audit mutex poisoned");
        annotations.clone()
    }

    /// 同步栈成功启动次数。
    pub fn sync_launches(&self) -> usize {
        let entries = self.entries.lock().expect("gateway audit mutex poisoned");
        entries
            .iter()
            .filter(|entry| entry.stack == GatewayRunStack::Sync)
            .count()
    }

    /// 流式栈成功启动次数。
    pub fn stream_launches(&self) -> usize {
        let entries = self.entries.lock().expect("gateway audit mutex poisoned");
        entries
            .iter()
            .filter(|entry| entry.stack == GatewayRunStack::Stream)
            .count()
    }

    /// 全部成功启动是否都携带非空 policy digest。空审计返回 false。
    pub fn all_have_policy_digest(&self) -> bool {
        let entries = self.entries.lock().expect("gateway audit mutex poisoned");
        !entries.is_empty() && entries.iter().all(|entry| !entry.policy_digest.is_empty())
    }

    /// 记录一次 resume 一致性 supersede(Task 13):旧会话因 fingerprint 漂移
    /// 被 supersede。供 `resume_or_start` 在 mismatch 路径调用。
    fn supersede(
        &self,
        superseded_session_id: &str,
        reason: &str,
    ) -> Result<(), ProviderGatewayError> {
        let mut supersedes = self
            .supersedes
            .lock()
            .expect("gateway audit mutex poisoned");
        supersedes.push(GatewaySupersedeEntry {
            superseded_session_id: superseded_session_id.to_string(),
            reason: reason.to_string(),
        });
        Ok(())
    }

    /// resume 一致性 supersede 计数。
    pub fn supersede_count(&self) -> usize {
        let supersedes = self
            .supersedes
            .lock()
            .expect("gateway audit mutex poisoned");
        supersedes.len()
    }

    /// 最近一次 supersede 的原因,无则 `None`。供 fixture 断言漂移维度。
    pub fn last_supersede_reason(&self) -> Option<String> {
        let supersedes = self
            .supersedes
            .lock()
            .expect("gateway audit mutex poisoned");
        supersedes.last().map(|entry| entry.reason.clone())
    }
}

/// 逻辑代码库真实 provider 的唯一建造与启动入口。
///
/// 构造时注入 policy store、capability source、target resolver、真实 provider
/// registry 与 availability gate。`validate` 产出 opaque
/// `ValidatedSessionLaunchPolicy`,后者只能经此方法取得;`start_streaming`/
/// `run_sync` 只接受绑定该 policy 的 validated input,并在 spawn 前重新复验
/// canonical cwd/git-dir/worktree identity 与 provider 可用性。每次成功启动都
/// 写入 `audit`(Task 11),使逻辑代码库真实 provider 调用经 gateway 这一事实可审计。
pub struct LogicalCodebaseProviderGateway {
    policies: AggregatePolicyArtifactStore,
    capabilities: Arc<dyn ProviderCapabilitySource>,
    targets: Arc<dyn PolicyTargetResolver>,
    registry: Arc<ProviderRegistry>,
    sync_adapter: Arc<dyn crate::cross_cutting::provider_adapter::ProviderAdapter + Send + Sync>,
    availability_gate: Arc<ProviderAvailabilityGate>,
    audit: Arc<GatewayRunAudit>,
    /// 聚合政策权威根 locator(= manifest.provider_context_root,构造时 canonicalize)。
    /// 冻结后供 `validate` 写入 envelope.authority_root。
    authority_root: PathBuf,
}

impl LogicalCodebaseProviderGateway {
    pub fn new(
        policies: AggregatePolicyArtifactStore,
        capabilities: Arc<dyn ProviderCapabilitySource>,
        targets: Arc<dyn PolicyTargetResolver>,
        registry: Arc<ProviderRegistry>,
        sync_adapter: Arc<
            dyn crate::cross_cutting::provider_adapter::ProviderAdapter + Send + Sync,
        >,
        availability_gate: Arc<ProviderAvailabilityGate>,
        authority_root: PathBuf,
    ) -> Self {
        Self {
            policies,
            capabilities,
            targets,
            registry,
            sync_adapter,
            availability_gate,
            audit: Arc::new(GatewayRunAudit::new()),
            authority_root,
        }
    }

    /// 共享同一份启动审计构造 gateway。测试 fixture 用同一 `Arc<GatewayRunAudit>`
    /// 跨多次 `gateway()` 构造累计启动记录;生产侧未来可注入跨实例审计。
    #[allow(clippy::too_many_arguments)]
    pub fn with_audit(
        policies: AggregatePolicyArtifactStore,
        capabilities: Arc<dyn ProviderCapabilitySource>,
        targets: Arc<dyn PolicyTargetResolver>,
        registry: Arc<ProviderRegistry>,
        sync_adapter: Arc<
            dyn crate::cross_cutting::provider_adapter::ProviderAdapter + Send + Sync,
        >,
        availability_gate: Arc<ProviderAvailabilityGate>,
        audit: Arc<GatewayRunAudit>,
        authority_root: PathBuf,
    ) -> Self {
        Self {
            policies,
            capabilities,
            targets,
            registry,
            sync_adapter,
            availability_gate,
            audit,
            authority_root,
        }
    }

    /// 返回启动审计的共享句柄。供外部(测试 fixture、未来生产侧)观测启动计数与
    /// policy digest 完整性。
    pub fn audit(&self) -> Arc<GatewayRunAudit> {
        self.audit.clone()
    }

    /// 校验启动请求并产出不可缺省的 validated policy。fail-closed:缺失政策返回
    /// `PolicyMissing`,绝不退化到无政策 fallback。
    pub fn validate(
        &self,
        request: SessionLaunchRequest,
    ) -> Result<ValidatedSessionLaunchPolicy, ProviderGatewayError> {
        let project_id = request.project_id.clone();
        let result = self.validate_inner(request);
        if let Err(error) = &result {
            tracing::warn!(
                project_id,
                error = %error,
                "provider gateway denied session validation"
            );
        }
        result
    }

    fn validate_inner(
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

        // 路由级硬门(Task 13):Codex danger-full-access 在 gateway 路由级阻断,
        // 不论 UI 是否选择该 provider。该阻断发生在 envelope 冻结之前,使 Codex
        // 无法进入逻辑 route。
        self.enforce_route_policy(&capability)?;

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
            self.authority_root.clone(),
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

    /// 路由级硬门(Task 13):对解析出的 provider capability 施加 gateway-owned
    /// 路由阻断。当前唯一规则:Codex 在 `danger-full-access` sandbox 下不支持。
    ///
    /// 该检查是 gateway-owned 的,不依赖注入的 `ProviderCapabilitySource` 实现
    /// (测试 double 可各自实现业务能力校验,但路由级危险模式阻断不可被绕过)。
    /// 阻断发生在 envelope 冻结与 registry lookup 之前,使危险 provider 无法
    /// 进入逻辑 route。路由级 fail-closed 不等于 OS 级隔离:本门是 experimental
    /// +supervised 场景下的政策门,不宣称物理不可写。
    fn enforce_route_policy(
        &self,
        capability: &ProviderCapability,
    ) -> Result<(), ProviderGatewayError> {
        if capability.provider_type == ProviderRefType::Codex
            && CODEX_DANGER_FULL_ACCESS_SANDBOX_MODE
                == crate::cross_cutting::codex_provider::CODEX_DEFAULT_SANDBOX_MODE
        {
            return Err(ProviderGatewayError::UnsupportedCapability(
                CODEX_DANGER_FULL_ACCESS_UNSUPPORTED.to_string(),
            ));
        }
        Ok(())
    }

    /// resume 启动判定(Task 13):据旧会话冻结的 fingerprint 与当前 validate
    /// 产出的 validated policy 全维度比对。只有 policy digest、target、provider
    /// exact version、dialect 与 capability snapshot 全一致(fingerprint 相等)
    /// 才 `Resume`;任一维度漂移则 supersede 旧会话(写审计)并 `StartNew`,
    /// 旧 session id 透传给调用方以清理旧会话状态。
    ///
    /// resume 判定在路由级硬门之后:Codex danger-full-access 等被路由阻断的
    /// provider 在进入 resume 决策前即被拒绝。
    pub fn resume_or_start(
        &self,
        request: ResumeSessionLaunchRequest,
    ) -> Result<GatewaySessionDisposition, ProviderGatewayError> {
        let validated = self.validate(request.launch)?;
        if validated.fingerprint() == &request.previous_fingerprint {
            Ok(GatewaySessionDisposition::Resume(validated))
        } else {
            self.audit
                .supersede(&request.previous_session_id, "resume_fingerprint_mismatch")?;
            Ok(GatewaySessionDisposition::StartNew {
                validated,
                superseded_session_id: request.previous_session_id,
            })
        }
    }

    /// 配置来源审计 policy 门禁(Task 13 → Task 11 语义修正):据 `ConfigSourceAudit`
    /// 标注的 provenance 决定如何记录。当前 policy 默认对 `managed_settings_active=true`
    /// 的启动**不再 fail-closed**:绝不假装已覆盖 managed settings,但也不阻断启动,
    /// 而是在 `GatewayRunAudit` 追加标注(携带 config digest)后放行。该标注使「检测到
    /// managed settings」这一已知 gap 可被外部审计,而非静默忽略。
    ///
    /// `ManagedSettingsActive` 错误 variant 保留但不再产生:保留是为了稳定码表与
    /// 既有测试引用的可编译性,未来若 policy 重新收紧为拒绝,可直接恢复 fail-closed。
    pub fn enforce_config_source_policy(
        &self,
        _validated: &ValidatedSessionLaunchPolicy,
        audit: &ConfigSourceAudit,
    ) -> Result<(), ProviderGatewayError> {
        if audit.provenance.managed_settings_active {
            self.audit
                .annotate_managed_settings_active(&audit.config_digest);
        }
        Ok(())
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
        if let Err(error) = self.revalidate_before_spawn(&validated, &input.working_dir, is_resume)
        {
            tracing::warn!(
                project_id = %validated.project_id,
                error = %error,
                "provider gateway blocked streaming spawn during revalidation"
            );
            return Err(error);
        }
        let adapter = self.lookup_real_streaming_adapter(&validated)?;
        let policy_digest = validated.envelope().policy_digest.clone();
        let session = adapter
            .start(input, cancel)
            .await
            .map_err(ProviderGatewayError::Adapter)?;
        // Task 11 审计聚合:在 gateway 启动路径内构造 ConfigSourceAudit。当前无
        // provider `/status` setting sources 注入接口与真实 argv 源,故 sources 与
        // argv 均为空(记录为空并保留字段);config digest 由 envelope 冻结的
        // config_artifact_ref 重算。
        let ConfigSourceAudit {
            argv,
            config_digest,
            ..
        } = ConfigSourceAudit::from_launch(
            &[],
            &validated.envelope().config_artifact_ref,
            ConfigSourceProvenance::detect_from_setting_sources(&[]),
        );
        self.audit.record(
            GatewayRunStack::Stream,
            policy_digest,
            Some(config_digest),
            argv,
        );
        Ok(session)
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
        if let Err(error) = self.revalidate_before_spawn(&validated, cwd, false) {
            tracing::warn!(
                project_id = %validated.project_id,
                error = %error,
                "provider gateway blocked sync spawn during revalidation"
            );
            return Err(error);
        }
        let policy_digest = validated.envelope().policy_digest.clone();
        let output = self
            .sync_adapter
            .run(&input)
            .map_err(ProviderGatewayError::Adapter)?;
        // Task 11 审计聚合:同 start_streaming,sources/argv 为空(无真实注入源),config
        // digest 由 envelope 冻结的 config_artifact_ref 重算。
        let ConfigSourceAudit {
            argv,
            config_digest,
            ..
        } = ConfigSourceAudit::from_launch(
            &[],
            &validated.envelope().config_artifact_ref,
            ConfigSourceProvenance::detect_from_setting_sources(&[]),
        );
        self.audit.record(
            GatewayRunStack::Sync,
            policy_digest,
            Some(config_digest),
            argv,
        );
        Ok(output)
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
        // 路由级硬门在 spawn 前复验中同样施加:防 validate→spawn 间 capability
        // source 被替换为 Codex(防 TOCTOU)。
        self.enforce_route_policy(&capability)?;
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

        // 5b. target 重解析(REQ-ENV-03):用冻结值重建启动请求,调用注入的 resolver
        // 重新解析并复验 target identity;返回 target 与 envelope 冻结 target
        // 不一致 → fail-closed(validate→spawn 之间 .git 指针/target 被调包)。
        let revalidate_request = SessionLaunchRequest {
            project_id: validated.project_id.clone(),
            provider: validated.provider.clone(),
            action: validated.action,
            target: envelope.target.clone(),
            readable_roots: envelope.readable_roots.clone(),
            writable_roots: envelope.writable_roots.clone(),
            config_artifact_ref: envelope.config_artifact_ref.clone(),
        };
        let revalidated_target = self.targets.resolve_and_revalidate(&revalidate_request)?;
        if revalidated_target != envelope.target {
            return Err(ProviderGatewayError::TargetMismatch {
                field: "target".to_string(),
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

/// Codex 当前唯一配置的 sandbox 模式即 `danger-full-access`。受限写(restricted-
/// write)sandbox 尚未就绪,故 gateway 在路由级对 Codex 启动 fail-closed:不论
/// UI 是否隐藏该 provider,validate 阶段直接拒绝。这与「UI 隐藏」不同——路由级
/// 阻断无法被 CLI/脚本等 UI 外路径绕过。
///
/// 该常量与 `cross_cutting::codex_provider::CODEX_DEFAULT_SANDBOX_MODE` 对齐,作为
/// gateway 侧的路由门基准。当受限写 sandbox 配置可用后,此常量与 guard 逻辑应
/// 一并演进以放行受限写模式。
pub const CODEX_DANGER_FULL_ACCESS_SANDBOX_MODE: &str = "danger-full-access";

/// gateway 路由阻断码:Codex 在 danger-full-access sandbox 下不被支持。
pub const CODEX_DANGER_FULL_ACCESS_UNSUPPORTED: &str = "codex_danger_full_access_unsupported";

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
#[path = "provider_gateway_tests.rs"]
mod tests;
