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
use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::session_launch::{
    ValidatedAdapterInput, ValidatedStreamingProviderInput,
};
use crate::cross_cutting::streaming_provider::{ProviderSession, StreamingProviderAdapter};
use crate::product::logical_codebase::policy::{
    AggregatePolicyArtifactStore, PolicyTarget, ProviderDialect, ProviderWireDialect,
    SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_admission_preflight::ProviderAdmissionError;
use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
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
    /// Task 3b:action row 的 evidence profile 摘要(冻结已实测
    /// version/action 的完整权限画像,不能随单次 role 变化)。
    projection_digest: String,
    capability_snapshot_ref: String,
    /// Task 2b 第二段:私有冻结相位。`Normal` 由普通 `validate` 产出;
    /// `RootRecipe` 只能由 gateway 内部 `validate_root_recipe_request`
    /// (durable Running 派生凭据)产出并携带冻结凭据供 spawn 前重验——
    /// 枚举与字段私有、无 public constructor,普通调用不可构造。
    phase: ValidatedSessionLaunchPhase,
    /// Task 3a/3b:#8 发布链冻结的 locator digest 事实;`None` = 本次
    /// validate 未消费发布链(自举桩/RootRecipe 相位/未注入只读事实源),
    /// spawn 前复验跳过正文重读。
    policy_locator: Option<PolicyLocatorDigests>,
    /// Task 1b:prepare 阶段冻结的 run-bound launch audit 上下文。`None` =
    /// 未经 `prepare_*_launch` 的存量构造(1b 之前/测试 fixture);`Some` =
    /// 已在 prepare 前绑定 run-bound sink 的 prepared launch(gateway 同步
    /// 分发与 sync bridge 消费)。
    launch_audit: Option<ProviderLaunchAuditContext>,
    /// Task 9a:完整上下文路径(`resume_or_start_with_projection`)冻结的
    /// 候选 LC 投影与其配套指纹。`None` = 裸 `validate` 的 envelope+
    /// capability 基准(legacy/单仓语义,由 `resume_or_start` 保持);`Some`
    /// = 调用者先完整 prepare 得候选投影后经 gateway 全上下文判定,指纹
    /// 与投影同源冻结,spawn 前复验按同一基准重算。
    lc_projection:
        Option<crate::product::logical_codebase::provider_projection::ProviderPolicyProjection>,
    /// 批2 Task 2:裸 validate 的 gateway 基准投影副本。受信执行合同绑定时
    /// 以合同摘要装饰该基准(或 full 投影)重算指纹,spawn 前复验按同一
    /// 装饰重算——绑定前后指纹必然不同,旧记录不可能与合同绑定后的启动
    /// 比对相等(不默认续接)。
    basis_projection:
        crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
    /// 批2 Task 2:受信执行合同(`freeze_provider_execution` 产物,crate 内
    /// `prepare_trusted_*_launch` 绑定;字段 private、无 public constructor)。
    /// `None` = 新执行合同**显式缺失**(存量构造/旧记录):消费方不得把
    /// 缺失默认为 HostCoding 续接旧 session,只能走显式标缺/显式新会话路径。
    execution_contract: Option<crate::cross_cutting::provider_execution::FrozenProviderExecution>,
}

/// #8 发布链在 validate 时点冻结的 digest 事实(spawn 前复验重读比对)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicyLocatorDigests {
    pub(crate) policy_digest: String,
    pub(crate) rule_digest: String,
}

/// Task 1b 冻结接口「准备与同步」:LC launch 的 run-bound audit 上下文。
///
/// `prepare_*_launch` 在 validate 之后、启动之前把它冻结进 validated policy:
/// 所有 LC 角色(含无通用 tool_policy 的 Coder/Kimi)都先绑定 run-bound sink
/// 再启动,统一写 launch audit。`audit_sink` 是 `LifecycleStore` 等生产 sink;
/// `(workspace_session_id, role_run_seq)` 与 durable tool-policy 审计分区的
/// 文件 key 同源。
#[derive(Clone)]
pub struct ProviderLaunchAuditContext {
    pub workspace_session_id: String,
    pub role_run_seq: u64,
    pub audit_sink: Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>,
}

impl std::fmt::Debug for ProviderLaunchAuditContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderLaunchAuditContext")
            .field("workspace_session_id", &self.workspace_session_id)
            .field("role_run_seq", &self.role_run_seq)
            .field("audit_sink", &"<run-bound tool policy audit sink>")
            .finish()
    }
}

/// Task 3b:early action 资格判定的返回面——只携带两枚不可伪造的引用
/// (capability snapshot 与 action row 的 evidence profile 摘要),不携带
/// 未来 target/worktree 或 D4 事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderActionAdmission {
    pub capability_snapshot_ref: String,
    pub projection_ref: String,
}

/// validated policy 的私有冻结相位(Task 2b 第二段,模块外不可见)。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ValidatedSessionLaunchPhase {
    Normal,
    RootRecipe(
        crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ),
}

impl ValidatedSessionLaunchPolicy {
    /// 返回冻结的 envelope 快照。getter 是外部唯一访问字段的方式。
    pub fn envelope(&self) -> &SessionPolicyEnvelope {
        &self.envelope
    }

    /// resume 复验指纹。
    pub fn fingerprint(&self) -> &SessionResumeFingerprint {
        &self.fingerprint
    }

    /// 冻结的 capability snapshot 引用（C4 Task 8 admission 预检消费）。
    pub fn capability_snapshot_ref(&self) -> &str {
        &self.capability_snapshot_ref
    }

    /// 是否 root-recipe 相位(crate 内观测面;普通 validate 恒 false)。
    pub(crate) fn is_root_recipe_phase(&self) -> bool {
        matches!(self.phase, ValidatedSessionLaunchPhase::RootRecipe(_))
    }

    /// Task 1b:冻结 prepare 阶段绑定的 run-bound launch audit 上下文(仅供
    /// gateway 的 `prepare_*_launch` 使用,模块外不可构造 prepared policy)。
    pub(crate) fn with_launch_audit(mut self, context: ProviderLaunchAuditContext) -> Self {
        self.launch_audit = Some(context);
        self
    }

    /// Task 9a:完整上下文路径冻结的候选 LC 投影(裸 validate 为 `None`)。
    pub(crate) fn lc_projection(
        &self,
    ) -> Option<&crate::product::logical_codebase::provider_projection::ProviderPolicyProjection>
    {
        self.lc_projection.as_ref()
    }

    /// Task 9a:action row 的 evidence profile 摘要(`capability_projection_
    /// digest` 同源;冻结已实测 version/action 的完整权限画像,不随单次
    /// role 变化)——五参 `from_envelope` 的 action evidence 维度。
    pub(crate) fn action_evidence_digest(&self) -> &str {
        &self.projection_digest
    }

    /// Task 9a:以候选 LC 投影与配套全上下文指纹覆写基准(仅
    /// `resume_or_start_with_projection` 内部使用)。
    ///
    /// T10(RevB2T2 ⚠2):合同绑定晚于投影冻结时,本方法必须按冻结合同的
    /// **resume 稳定摘要**重装饰新投影并重算指纹——与
    /// [`Self::with_frozen_execution`]、spawn 前复验保持同一装饰纪律;
    /// 漏装饰会让复验指纹恒 `PolicyDrift(resume_fingerprint)`。
    fn with_lc_projection(
        mut self,
        projection: crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
        fingerprint: SessionResumeFingerprint,
    ) -> Self {
        self.fingerprint = match self.execution_contract.as_ref() {
            Some(contract) => SessionResumeFingerprint::from_envelope(
                &self.envelope,
                &projection
                    .clone()
                    .with_execution_digest(contract.contract_resume_digest()),
                &self.projection_digest,
                &canonical_target_git_identity(&self.envelope.target.worktree),
            ),
            None => fingerprint,
        };
        self.lc_projection = Some(projection);
        self
    }

    /// prepared launch 的 audit 上下文(sync bridge 消费 sink/身份;未 prepare
    /// 的存量构造为 `None`)。
    pub(crate) fn launch_audit(&self) -> Option<&ProviderLaunchAuditContext> {
        self.launch_audit.as_ref()
    }

    /// 批2 Task 2:冻结的执行合同。`None` = 新执行合同显式缺失(存量构造/
    /// 旧记录)——**不是** HostCoding 默认,消费方不得据此续接旧 session。
    pub fn frozen_execution(
        &self,
    ) -> Option<&crate::cross_cutting::provider_execution::FrozenProviderExecution> {
        self.execution_contract.as_ref()
    }

    /// 批2 Task 2:受信 builder 绑定执行合同(crate 内,`prepare_trusted_*
    /// _launch` 专用)。指纹按合同的 **resume 稳定摘要**(T10:不含
    /// `provider_run_id`——同一会话的后续 run 换新 run id 不得让每次
    /// resume 都 supersede;纪律事实漂移仍必然失配)装饰投影基准后重算
    /// (spawn 前复验按同一装饰重算):执行合同成为 resume 指纹的非敏感
    /// 漂移维度,旧记录(无合同)与绑定后的指纹永不相等。
    pub(crate) fn with_frozen_execution(
        mut self,
        contract: crate::cross_cutting::provider_execution::FrozenProviderExecution,
    ) -> Self {
        let basis = self
            .lc_projection
            .clone()
            .unwrap_or_else(|| self.basis_projection.clone());
        let decorated = basis.with_execution_digest(contract.contract_resume_digest());
        self.fingerprint = SessionResumeFingerprint::from_envelope(
            &self.envelope,
            &decorated,
            &self.projection_digest,
            &canonical_target_git_identity(&self.envelope.target.worktree),
        );
        self.execution_contract = Some(contract);
        self
    }
}

/// Task 3.2(B0a2):gateway `prepare_lc_streaming_candidate` 冻结的 opaque
/// LC streaming 候选。字段私有、只能由 gateway 构造:私有 input/request/
/// validated/full projection/facts/run-bound audit/adapter 句柄。
///
/// 冻结+复验矩阵:候选冻结 full projection 与 facts(exact version/观测
/// 时刻);spawn 前经 gateway `reobserve_lc_prepare_facts` 重新取当前事实,
/// 并以 [`Self::assert_frozen_version`] 比对——version 单独允许 60 秒 TTL
/// 命中,但 identity 变更后的新观测与冻结事实不一致必须 typed 拒,不得
/// spawn。
///
/// 转换纪律:workspace LC 仅在 **full-matching resume**(以本候选 full
/// projection 走 `resume_or_start_with_projection` 判定为 `Resume`)或
/// **明确 fresh** 后才可经 [`Self::into_validated_streaming_input`] 转换
/// 为可 spawn 输入;`StartNew` 等待面的 validated 不得直接 start。
#[derive(Clone)]
pub struct PreparedLcStreamingCandidate {
    input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
    request: SessionLaunchRequest,
    validated: ValidatedSessionLaunchPolicy,
    full_projection:
        crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
    facts: crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts,
    run_bound_audit: ProviderLaunchAuditContext,
    adapter: Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter>,
}

impl PreparedLcStreamingCandidate {
    /// 冻结的 streaming input(prepare 已绑定 run-bound audit/会话身份)。
    pub fn input(&self) -> &crate::cross_cutting::streaming_provider::StreamingProviderInput {
        &self.input
    }

    /// 冻结的启动请求。
    pub fn request(&self) -> &SessionLaunchRequest {
        &self.request
    }

    /// 冻结的 validated policy(携带 run-bound audit 上下文)。
    pub fn validated(&self) -> &ValidatedSessionLaunchPolicy {
        &self.validated
    }

    /// 冻结的 full projection(prepare 时点由 facts 同版本 projector 产出)。
    pub fn full_projection(
        &self,
    ) -> &crate::product::logical_codebase::provider_projection::ProviderPolicyProjection {
        &self.full_projection
    }

    /// 冻结的 adapter facts(exact version/观测时刻/probe key)。
    pub fn facts(
        &self,
    ) -> &crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts {
        &self.facts
    }

    /// prepare 绑定的 run-bound audit 上下文。
    pub fn run_bound_audit(&self) -> &ProviderLaunchAuditContext {
        &self.run_bound_audit
    }

    /// 冻结+复验矩阵:以重观测的当前 facts 比对冻结 full projection 的
    /// exact version;不一致 → typed `PolicyDrift(provider_version)` 拒
    /// resume,不可 spawn。
    pub fn assert_frozen_version(
        &self,
        current: &crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts,
    ) -> Result<(), ProviderGatewayError> {
        if self.full_projection.exact_version() != current.exact_version() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "provider_version".to_string(),
            });
        }
        Ok(())
    }

    /// Task 3.7(B0f):候选 full 上下文 resume 指纹——五参 `from_envelope`
    /// 以冻结 full projection 与 validated 冻结的 capability row 摘要
    /// (action evidence;admission 已保证与投影 capability 摘要逐字相等)
    /// 计算。与 spawn 前复验重算、adapter audit 字面量、B1/B2 prepared
    /// full map(basis=`FullLcProjectionV2`)的 `fingerprint_digest` 同源。
    pub fn full_context_fingerprint(&self) -> SessionResumeFingerprint {
        SessionResumeFingerprint::from_envelope(
            self.validated.envelope(),
            &self.full_projection,
            self.validated.action_evidence_digest(),
            &canonical_target_git_identity(&self.validated.envelope().target.worktree),
        )
    }

    /// 转换面(冻结合同):仅在调用方完成 full-matching resume 判定或取得
    /// 显式 fresh 授权后调用;不得把 `StartNew` 等待面的 validated 直接
    /// start(crate 内唯一转换出口)。生产 caller 接线归 B1/B2(Task
    /// 4.x/5.x),此前只有合同测试消费。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn into_validated_streaming_input(
        self,
    ) -> crate::cross_cutting::session_launch::ValidatedStreamingProviderInput {
        crate::cross_cutting::session_launch::ValidatedStreamingProviderInput::new(
            self.input,
            self.validated,
        )
    }
}

/// provider 启动请求。gateway 据此解析政策 artifact、target 与 capability。
#[derive(Debug, Clone)]
pub struct SessionLaunchRequest {
    pub project_id: String,
    pub provider: ProviderRef,
    pub action: SessionPolicyAction,
    pub target: PolicyTarget,
    /// 会话 cwd（canonical LC root，Task 2.5 cwd/target 分离合同）。单仓/
    /// 现状入口与 target worktree 同值（零行为变化）；LC 入口注入 root，
    /// 禁止回退 member cwd。gateway 冻结进 envelope 并纳入 resume fingerprint。
    pub working_directory: PathBuf,
    pub readable_roots: Vec<PathBuf>,
    pub writable_roots: Vec<PathBuf>,
    /// 托管配置 artifact 引用(envelope 冻结其 digest);非空否则 envelope 校验失败。
    pub config_artifact_ref: String,
}

impl SessionLaunchRequest {
    /// 构造一个 planning 只读启动请求:read-only action 必须没有 writable roots。
    ///
    /// Task 2.5 兼容策略:`working_directory` 默认映射 target worktree——现状
    /// 所有 planning 入口 cwd==target(单仓两字段映射旧目录,零行为变化)。
    /// LC 根 cwd(root≠member target)由调用方经结构体字面量显式提供
    /// (Task 2.2/2.8 接线),不经本构造函数静默派生。
    pub fn planning(
        project_id: impl Into<String>,
        provider: ProviderRef,
        target: PolicyTarget,
        readable_roots: Vec<PathBuf>,
        config_artifact_ref: impl Into<String>,
    ) -> Self {
        let working_directory = target.worktree.clone();
        Self {
            project_id: project_id.into(),
            provider,
            action: SessionPolicyAction::PlanningReadOnly,
            target,
            working_directory,
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

    pub fn pi(capability_snapshot_ref: impl Into<String>) -> Self {
        Self {
            provider_type: ProviderRefType::Pi,
            capability_snapshot_ref: capability_snapshot_ref.into(),
        }
    }

    pub fn kimi_code(capability_snapshot_ref: impl Into<String>) -> Self {
        Self {
            provider_type: ProviderRefType::KimiCode,
            capability_snapshot_ref: capability_snapshot_ref.into(),
        }
    }

    /// 由 session/role 配置的 `ProviderName` 派生 gateway 启动 ref(C-2 集中
    /// 映射,Task 1a 起四家真实 provider 显式映射)。
    ///
    /// ClaudeCode/Codex/Pi/KimiCode 四家各自映射到显式 `ProviderRefType`;
    /// Fake(及未来新增 provider)一律 fail-closed 返回 `UnsupportedCapability`
    /// (错误信息含稳定判别码 `PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH` 与
    /// provider 名)。🔴 禁止 `_ => claude_code` 之类的静默回退:用户配置的
    /// provider 不允许被悄悄换成 Claude 或其它 provider 启动。match 保持无
    /// `_` 分支的穷举形态——未来新增 `ProviderName` 变体时编译期即强制补
    /// 显式映射决策。Codex 的 danger-full-access 路由阻断(REQ-ENV-05)由
    /// gateway 路由级硬门施加,与本映射正交——Codex 配置=显式阻断错误而非
    /// 被改成 Claude。
    pub fn from_provider_name(
        provider: &ProviderName,
        capability_snapshot_ref: impl Into<String>,
    ) -> Result<Self, ProviderGatewayError> {
        match provider {
            ProviderName::ClaudeCode => Ok(Self::claude_code(capability_snapshot_ref)),
            ProviderName::Codex => Ok(Self::codex(capability_snapshot_ref)),
            ProviderName::Pi => Ok(Self::pi(capability_snapshot_ref)),
            ProviderName::KimiCode => Ok(Self::kimi_code(capability_snapshot_ref)),
            ProviderName::Fake => Err(ProviderGatewayError::UnsupportedCapability(format!(
                "{PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH}:{provider:?}"
            ))),
        }
    }
}

/// gateway 知晓的真实 provider 类型。四家真实 provider(Task 1a,
/// REQ-LCG-01):Claude Code、Codex、Pi、Kimi Code。Fake/测试路径不经
/// 此 gateway,故此处不含 Fake。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRefType {
    ClaudeCode,
    Codex,
    Pi,
    KimiCode,
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

    /// 该状态是否允许 resume 启动。仅 `Confirmed` 为真。
    pub fn allows_resume(self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

/// gateway 解析出的 provider capability 快照(Task 2b,冻结接口):承载
/// record v2 的逐 action 三态行与 trust,供 envelope/fingerprint 复验与
/// phase-aware 门禁消费。`resume_evidence` 旧二态字段已移除——resume 判定
/// 只消费 `action_capability.resume` 分格(`require_resume_supported`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapability {
    pub provider_type: ProviderRefType,
    pub version: String,
    pub adapter_dialect: ProviderDialect,
    /// wire(传输)dialect(record v2;与 adapter dialect 正交)。
    pub wire_dialect: ProviderWireDialect,
    pub capability_snapshot_ref: String,
    /// 正常会话的逐 action 三态行(launch/resume/write_boundary 分格)。
    /// root-recipe 相位返回的行仅镜像 durable normal 状态,不因 recipe 事实
    /// 铸造 Confirmed(隔离契约)。
    pub action_capability: ProviderActionCapability,
    /// provider trust 证据(三态)。
    pub trust: ProviderCapabilityEvidence,
}

/// resume 复验指纹(Task 9a 起 v2):覆盖 policy 指纹(id/revision/digest)、
/// canonical authority root/cwd、target 三元组与 git identity、action evidence
/// 摘要,以及候选 LC 投影携带的全部冻结维度(provider 映射/exact version/
/// wire dialect、trust/config/MCP bundle digest、boundary 引用与分层投影
/// 摘要)。spawn 前与 provider 上报状态重新比对;旧会话缺 v2 材料即视为
/// 漂移(supersede),不存在仅 envelope 指纹的可 spawn 权。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResumeFingerprint {
    pub digest: String,
}

impl SessionResumeFingerprint {
    /// 是否携带 full projection v2 basis 的 schema 前缀(fix-loop P2):
    /// `from_envelope` 产出的 digest 以 [`LC_RESUME_FINGERPRINT_SCHEMA`]
    /// 开头;无前缀(legacy/外来形态)不是可混比的 full basis,候选
    /// 消费面 `PreparedLcResumeBasis::Full` 对其 fail-closed 为
    /// unavailable 等待,不与 full 指纹混比。
    pub fn is_full_projection_v2(&self) -> bool {
        self.digest.starts_with(LC_RESUME_FINGERPRINT_SCHEMA)
    }
}

/// 五参 `from_envelope` 的 schema 域版本:长度分隔域序列的前置标签。域
/// 集合或域语义变化时必须递增版本,旧 digest 与新 digest 永不相等。
pub const LC_RESUME_FINGERPRINT_SCHEMA: &str = "lc-session-resume-fingerprint/2";

/// resume 指纹漂移的 supersede 稳定原因码(gateway 审计与 `StartNew`
/// 等待投影同源)。
pub const RESUME_FINGERPRINT_MISMATCH: &str = "resume_fingerprint_mismatch";

/// 缺 full map/旧 basis 的等待稳定原因码(Task 3.7/设计 §4.1 reason 表):
/// 旧会话无可信 full 指纹可比(legacy basis/缺 prepared map),不与 full
/// 指纹混比、不折成出生 fresh,等待显式 `StartGeneration`。
pub const RESUME_FINGERPRINT_UNAVAILABLE: &str = "resume_fingerprint_unavailable";

impl SessionResumeFingerprint {
    /// 由 envelope、候选 LC 投影、action evidence 摘要与 target git identity
    /// 计算 canonical SHA-256(Task 9a 冻结五参签名)。每个域以 8 字节大端
    /// 长度前缀分隔(无边界歧义),schema 域版本前置;policy/authority/cwd/
    /// target/git identity/trust/tool/MCP bundle/exact version/投影摘要任一
    /// 漂移都会产生不同 digest。
    pub fn from_envelope(
        envelope: &SessionPolicyEnvelope,
        projection: &crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
        action_evidence_digest: &str,
        target_git_identity: &str,
    ) -> Self {
        let mut hasher = Sha256::new();
        let mut domain = |part: &[u8]| {
            hasher.update(&(part.len() as u64).to_be_bytes());
            hasher.update(part);
        };
        domain(LC_RESUME_FINGERPRINT_SCHEMA.as_bytes());
        domain(envelope.policy_id.as_bytes());
        domain(&envelope.policy_revision.to_be_bytes());
        domain(envelope.policy_digest.as_bytes());
        domain(format!("{:?}", envelope.action).as_bytes());
        // canonical authority root 与 cwd(独立维度,Task 2.5/2.8)。
        domain(envelope.authority_root.to_string_lossy().as_bytes());
        domain(envelope.working_directory.to_string_lossy().as_bytes());
        // target 三元组 + git identity(REQ-ENV-03)。
        domain(envelope.target.logical_repository_id.as_bytes());
        domain(envelope.target.checkout_id.as_bytes());
        domain(envelope.target.worktree.to_string_lossy().as_bytes());
        domain(target_git_identity.as_bytes());
        domain(action_evidence_digest.as_bytes());
        // 候选投影携带的冻结维度:provider 映射与 exact version、trust/
        // config/MCP bundle digest、boundary 引用、分层投影摘要。
        domain(format!("{:?}", projection.provider_type()).as_bytes());
        domain(format!("{:?}", projection.provider_dialect()).as_bytes());
        domain(format!("{:?}", projection.wire_dialect()).as_bytes());
        domain(projection.exact_version().as_bytes());
        domain(projection.trust_digest().as_bytes());
        domain(projection.config_digest().as_bytes());
        domain(projection.mcp_bundle_digest().as_bytes());
        domain(projection.boundary_evidence_ref().as_bytes());
        domain(projection.capability_projection_digest().as_bytes());
        domain(projection.projection_digest().as_bytes());
        // logical roots(计划冻结:Planning/Review 空 writable,Coding 恰一个
        // canonical target)。
        domain(&projection.readable_roots().len().to_be_bytes());
        for root in projection.readable_roots() {
            domain(root.to_string_lossy().as_bytes());
        }
        domain(&projection.writable_roots().len().to_be_bytes());
        for root in projection.writable_roots() {
            domain(root.to_string_lossy().as_bytes());
        }
        // 批2 Task 2:受信执行合同摘要(非敏感事实;未绑定为空串)。绑定
        // 合同后的指纹与任何 pre-合同记录的指纹必然不等——旧记录(无新
        // 执行合同)不可能默认续接,只能失配走显式新会话路径。schema 常量
        // 维持 `/2`:追加域本身已保证新旧 digest 永不相等(版本递增规则的
        // 目的),同时保住 `is_full_projection_v2` 的 prefix 门对新旧 digest
        // 一致可比(失配走 supersede 而非 unavailable,诊断更精确)。
        domain(projection.execution_contract_digest().as_bytes());
        let digest = format!(
            "{LC_RESUME_FINGERPRINT_SCHEMA}:sha256:{:x}",
            hasher.finalize()
        );
        Self { digest }
    }
}

/// 解析 target worktree 的 canonical git identity(REQ-ENV-03 的 git-dir
/// identity 形态):`.git` 为目录时取其 canonical 路径,为 worktree 指针文件
/// 时取 `gitdir:` 指向的 canonical 路径;解析失败退化为 worktree canonical
/// 文本(确定性维度——identity 值只需随 git identity 漂移而变化,解析
/// 失败本身不是 spawn 权威判定,spawn 前仍由 resolver 复验)。
pub(crate) fn canonical_target_git_identity(worktree: &Path) -> String {
    let git_entry = worktree.join(".git");
    let resolved = if git_entry.is_dir() {
        git_entry.canonicalize().ok()
    } else if let Ok(pointer) = std::fs::read_to_string(&git_entry) {
        pointer
            .lines()
            .find_map(|line| line.trim().strip_prefix("gitdir:"))
            .map(str::trim)
            .and_then(|gitdir| Path::new(gitdir).canonicalize().ok())
    } else {
        None
    };
    match resolved {
        Some(identity) => identity.to_string_lossy().into_owned(),
        None => worktree.to_string_lossy().into_owned(),
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

/// `StartNew` 决策的稳定等待投影(Task 9a):旧会话已 supersede,唯一稳定
/// allowed action 是显式 `StartGeneration` fresh(由现有 `WsInMessage::
/// StartGeneration`/`WorkspaceEngine::start_generation` 接受);同一 revision
/// driver 不得据此继续 `start_streaming`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LcSupersededWaiting {
    /// supersede 稳定原因码(与 gateway 审计 `supersede` 记录同源)。
    pub reason_code: String,
    /// 稳定等待动作集(仅显式 fresh)。
    pub allowed_actions:
        Vec<crate::product::logical_codebase::provider_admission_preflight::BootstrapActionKind>,
}

impl LcSupersededWaiting {
    /// `StartNew` 的唯一等待形态:等待用户显式 `StartGeneration`。
    fn superseded() -> Self {
        Self {
            reason_code: RESUME_FINGERPRINT_MISMATCH.to_string(),
            allowed_actions: vec![
                crate::product::logical_codebase::provider_admission_preflight::BootstrapActionKind::StartGeneration,
            ],
        }
    }

    /// 缺 full map/旧 basis 的等待形态(Task 3.7,B0f):无可信比对材料,
    /// 不与 full 指纹混比、不 supersede 审计未知会话,仅等待显式
    /// `StartGeneration` fresh(候选消费面 `resume_or_start_prepared_lc_
    /// candidate` 的 `Unavailable` 臂;与 `superseded()` 的 full 漂移形态
    /// 以稳定原因码区分)。
    fn fingerprint_unavailable() -> Self {
        Self {
            reason_code: RESUME_FINGERPRINT_UNAVAILABLE.to_string(),
            allowed_actions: vec![
                crate::product::logical_codebase::provider_admission_preflight::BootstrapActionKind::StartGeneration,
            ],
        }
    }
}

/// `resume_or_start` 的返回决策(Task 13;Task 9a 起 `StartNew` 携带稳定
/// 等待投影)。
///
/// - `Resume`:fingerprint 全维度相等,旧会话可安全 resume,返回新的 validated
///   policy 供 spawn。
/// - `StartNew`:fingerprint 漂移,旧会话被 supersede(审计),返回新 validated
///   policy、被 supersede 的旧 session id 与稳定等待投影——仅等待显式
///   `StartGeneration` fresh,不在同一 revision driver 静默 fresh。
#[derive(Debug)]
pub enum GatewaySessionDisposition {
    Resume(ValidatedSessionLaunchPolicy),
    StartNew {
        validated: ValidatedSessionLaunchPolicy,
        superseded_session_id: String,
        /// Task 9a:稳定等待投影(仅显式 `StartGeneration`)。
        waiting: LcSupersededWaiting,
    },
}

/// Task 3.7(B0f):workspace LC 候选消费面的 resume 判定输入——旧会话
/// full 冻结材料的可用性形态。
#[derive(Debug, Clone)]
pub enum PreparedLcResumeBasis {
    /// full map 完整:旧会话冻结的 full 指纹与 native id,参与 full 对
    /// full 的全维度比对(漂移 → supersede 审计并等待
    /// [`RESUME_FINGERPRINT_MISMATCH`])。
    ///
    /// **只接受 basis=`FullLcProjectionV2` 的 digest**(带
    /// [`LC_RESUME_FINGERPRINT_SCHEMA`] 前缀):B1/B2 读 durable
    /// `LogicalLaunchFingerprintRecord` 时,`basis` 非 `FullLcProjectionV2`
    /// 必须传 [`Self::Unavailable`],不得把 legacy/外来 digest 喂入本臂。
    /// 误传(无 v2 schema 前缀)由 gateway fail-closed 为 unavailable
    /// 等待(fix-loop P2:不同 basis 不与 full 混比、不 supersede 从未
    /// 可信的会话),不进入比对。
    Full {
        previous_fingerprint: SessionResumeFingerprint,
        previous_session_id: String,
    },
    /// 缺 full map/旧 basis:无可信比对材料,不与 full 指纹混比,等待
    /// 显式 fresh(稳定原因码 [`RESUME_FINGERPRINT_UNAVAILABLE`]),
    /// 不 supersede 审计未知会话。
    Unavailable,
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
    /// resume 启动但 action row 的 resume 分格未 `Confirmed`(Task 2b B-2 消费者,
    /// `require_resume_supported`;旧 `resume_evidence` 二态不再消费)。
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

/// 解析 provider capability 并按相位校验(Task 2b 冻结接口)。
///
/// - [`Self::require_supported`]:保留名,走**正常 action row** 的 launch 分格
///   (fresh 门的一半;write_boundary 由 [`Self::require_write_boundary`] 把关);
/// - [`Self::require_resume_supported`]/[`Self::require_write_boundary`]:同参
///   返回型,分别消费 resume/write_boundary 分格;
/// - [`Self::require_root_recipe_supported`]:仅现有固定 Claude recipe 事实
///   (credential 每次重验 durable Running),不推 normal Confirmed。
///
/// 旧 `supported_actions`/provenance 不得产生正常会话 Confirmed;Denied 是
/// 真实负向证据,fail-closed 拒绝。
pub trait ProviderCapabilitySource: Send + Sync {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError>;

    /// 明确 resume 门:仅 resume 分格 `Confirmed` 放行;Unknown/Denied 一律
    /// 拒绝(不得静默改 fresh)。
    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError>;

    /// fresh 门的 write-boundary 半边:仅 write_boundary 分格 `Confirmed` 放行。
    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError>;

    /// root-recipe 相位:仅现有固定 Claude recipe 事实放行,凭据每次重验
    /// durable Running;返回的 capability 不携带任何 normal Confirmed 事实。
    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
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
    /// 批2 Task 2:该次启动冻结的受信执行合同摘要。`None` = 新执行合同
    /// 显式缺失(存量启动/旧记录)——缺失如实记录,从不被记为默认
    /// HostCoding 续接。
    pub execution_contract_digest: Option<String>,
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
        execution_contract_digest: Option<String>,
    ) {
        let mut entries = self.entries.lock().expect("gateway audit mutex poisoned");
        entries.push(GatewayRunAuditEntry {
            stack,
            policy_digest,
            config_digest,
            argv,
            execution_contract_digest,
        });
    }

    /// 批2 Task 2:最近一次成功启动的审计条目(无启动记录为 `None`)。供
    /// fixture/诊断断言该次启动冻结(或显式缺失)的执行合同摘要。
    pub fn last_entry(&self) -> Option<GatewayRunAuditEntry> {
        let entries = self.entries.lock().expect("gateway audit mutex poisoned");
        entries.last().cloned()
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

/// [`LogicalCodebaseProviderGateway::classify_launch_trust_failure`] 的
/// 结构化输出：平铺 details（既有协议不变）＋同源等待投影（author/
/// reviewer 启动臂紧随 error 帧发出 `trust_missing_guidance` 结构化
/// 指导帧的来源，见 `EngineEvent::TrustMissingGuidance`）。未命中
/// trust 分型时 `trust_waiting=None`（details 原样，零结构化帧）。
#[derive(Debug, Clone)]
pub struct LaunchTrustFailureClassification {
    pub details: String,
    pub trust_waiting:
        Option<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting>,
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
    /// Task 3b:LC 作用域的 #8 receipt 只读事实源(factory 装配);未注入
    /// 时 validate/verdict 不消费发布链(裸 gateway 测试/旧链零回归)。
    receipts: Option<crate::product::logical_codebase::RootRecipeReceiptStore>,
    /// Task 3b:只读 trust source(GET/early 与 spawn 复验共用同一 source;
    /// 禁止 durable 副作用的 registry verify)。未注入时不检查 trust 维度。
    trust: Option<Arc<dyn crate::product::logical_codebase::provider_trust::ProviderTrustSource>>,
    /// trust source 调用所需的 LC 作用域(与 receipts 同源装配)。
    lc_scope: Option<String>,
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
        Self::with_audit(
            policies,
            capabilities,
            targets,
            registry,
            sync_adapter,
            availability_gate,
            Arc::new(GatewayRunAudit::new()),
            authority_root,
        )
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
            receipts: None,
            trust: None,
            lc_scope: None,
        }
    }

    /// Task 3b:注入 LC 只读事实源(#8 receipt store + 只读 trust source
    /// + LC 作用域)。factory 生产装配恒注入;未注入的裸 gateway 保持
    /// 既有语义(既有测试/旧链零回归)。
    pub fn with_readonly_lc_facts(
        mut self,
        receipts: crate::product::logical_codebase::RootRecipeReceiptStore,
        trust: Arc<dyn crate::product::logical_codebase::provider_trust::ProviderTrustSource>,
        lc_scope: Option<String>,
    ) -> Self {
        self.receipts = Some(receipts);
        self.trust = Some(trust);
        self.lc_scope = lc_scope;
        self
    }

    /// 6.1(方案 A 附带,D5):generation 首次 trust 失败的同源只读指导投影。
    ///
    /// 经注入的只读 trust source 与本 gateway 冻结的 canonical authority
    /// root 构造 `Generation` 场景等待面(场景化 retry_action「人工登记/
    /// 修复后显式重新生成」+同源 manual_guidance);**零 durable 副作用**
    /// ——不 ensure、不写 waiting/ownership/audit(start_generation 不自动
    /// 登记)。无注入 source、provider 无需用户级 workspace trust 或条目
    /// 当前已 trusted 时返回 `None`:调用方保持既有投影,不伪造
    /// capability、不给漂移根指导。仅供携带已确认 trust 失败原因的
    /// 只读桥接消费(admission 预检分类与 generation waiting 指导)。
    pub fn read_only_generation_trust_waiting(
        &self,
        provider: &crate::product::models::ProviderName,
    ) -> Option<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting> {
        let trust = self.trust.as_ref()?;
        if !crate::product::logical_codebase::provider_trust::requires_workspace_trust(provider) {
            return None;
        }
        trust.read_only_generation_waiting(provider, &self.authority_root)
    }

    /// T6 fix-loop r2（信任批量确认按需登记）：gateway 启动口 trust 拒绝的
    /// **共享分类桥**（产品层共享边界，web 与 product engine 共用，取代
    /// web 层 6.1 的七处外围包装）。
    ///
    /// 供全部生产 gateway 启动口——Workspace 通用 Author/ChoiceFollowup
    /// （`author_root_launch`）、Author Revision 与 Reviewer 首发
    /// （`review/drive`）、WIP plan author 家族
    /// （`start_work_item_plan_author` 的 Logical 分支，含七已包 caller 与
    /// followups 续驱）及 Coding 共享启动口（`provider_stream/launch`）——
    /// 在 typed `ProviderGatewayError` 压平为 `ProviderAdapterError`
    /// **之前**消费：仅 `ProviderUnavailable` 变体（保留原变体判据，不以
    /// 压平后的 code 代替）且注入的只读 trust source 当前仍判未通过时，
    /// 把 `mapped_details` 提升为 `provider_trust_missing` 分类＋同源
    /// manual_guidance＋场景化 retry_action；原拒绝原文保留。其余 gateway
    /// 变体（policy/root/target 漂移、capability 等）、无注入 source、
    /// provider 无需 workspace trust 或条目当前已 trusted 时原样返回
    /// `mapped_details`——不给漂移根 trust 指导、不误标 availability
    /// 拒绝。只读、零 trust 写、零 provider 启动、不自动重试。
    pub fn classify_launch_trust_failure_details(
        &self,
        gateway_error: &ProviderGatewayError,
        provider: &crate::product::models::ProviderName,
        mapped_details: &str,
        scenario: crate::product::logical_codebase::provider_trust::ProviderTrustScenario,
    ) -> String {
        self.classify_launch_trust_failure(gateway_error, provider, mapped_details, scenario)
            .details
    }

    /// [`classify_launch_trust_failure_details`] 的结构化变体（单一事实
    /// 源）：分类平铺 details 与同源等待投影一次产出——author/reviewer
    /// 启动臂消费 `trust_waiting` 紧随 error 帧发出结构化
    /// `trust_missing_guidance` 指导帧（`EngineEvent::TrustMissingGuidance`
    /// → `WsOutMessage::TrustMissingGuidance`，前端「两行说明＋可复制
    /// 命令窗」直渲）；判据/文案与平铺变体完全一致，未命中 trust 分型
    /// 时 `trust_waiting=None`（零结构化帧，不误发）。
    pub fn classify_launch_trust_failure(
        &self,
        gateway_error: &ProviderGatewayError,
        provider: &crate::product::models::ProviderName,
        mapped_details: &str,
        scenario: crate::product::logical_codebase::provider_trust::ProviderTrustScenario,
    ) -> LaunchTrustFailureClassification {
        if !matches!(gateway_error, ProviderGatewayError::ProviderUnavailable(_)) {
            return LaunchTrustFailureClassification {
                details: mapped_details.to_string(),
                trust_waiting: None,
            };
        }
        let Some(waiting) = self.read_only_trust_waiting_for_scenario(provider, scenario) else {
            return LaunchTrustFailureClassification {
                details: mapped_details.to_string(),
                trust_waiting: None,
            };
        };
        LaunchTrustFailureClassification {
            details: format!(
                "provider_trust_missing: {}: {}；原拒绝：{}；人工登记指导：{}（retry_action：{}）",
                waiting.reason_code,
                waiting.message,
                mapped_details,
                waiting.manual_guidance,
                waiting.retry_action,
            ),
            trust_waiting: Some(waiting),
        }
    }

    /// [`classify_launch_trust_failure_details`] 的场景化只读判据（与
    /// [`Self::read_only_generation_trust_waiting`] 同源 adapter/键算法，
    /// 仅多场景参数）。
    fn read_only_trust_waiting_for_scenario(
        &self,
        provider: &crate::product::models::ProviderName,
        scenario: crate::product::logical_codebase::provider_trust::ProviderTrustScenario,
    ) -> Option<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting> {
        let trust = self.trust.as_ref()?;
        if !crate::product::logical_codebase::provider_trust::requires_workspace_trust(provider) {
            return None;
        }
        trust.read_only_waiting_for_scenario(provider, &self.authority_root, scenario)
    }

    /// 返回启动审计的共享句柄。供外部(测试 fixture、未来生产侧)观测启动计数与
    /// policy digest 完整性。
    pub fn audit(&self) -> Arc<GatewayRunAudit> {
        self.audit.clone()
    }

    /// Task 2.3：gateway 冻结的 canonical authority root（构造时自 manifest
    /// `provider_context_root` canonicalize；与 aggregate 生产 driver 的 root
    /// canonical 一致性由工厂断言，Task 2.8）。review 等 launch 组装方以此
    /// 作为独立 cwd 来源——cwd 不再从 target/worktree 推导。
    pub fn authority_root(&self) -> &Path {
        &self.authority_root
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
        let capability = self
            .capabilities
            .require_supported(&request.provider, request.action)?;
        // Task 2b:fresh 门 = launch + write-boundary 两半,validate 阶段即
        // 消费 write_boundary 分格(spawn 前复验会再次施加,防 TOCTOU)。
        self.capabilities
            .require_write_boundary(&request.provider, request.action)?;

        // 路由级硬门(Task 13):Codex danger-full-access 在 gateway 路由级阻断,
        // 不论 UI 是否选择该 provider。该阻断发生在 envelope 冻结之前,使 Codex
        // 无法进入逻辑 route。
        self.enforce_route_policy(&capability)?;

        self.assemble_validated(request, capability, ValidatedSessionLaunchPhase::Normal)
    }

    /// Task 2b 第二段:root-recipe 相位的 gateway 内部校验入口(pub(crate),
    /// 不对普通调用开放)。capability 只消费固定 Claude recipe 事实
    /// (`require_root_recipe_supported`,凭据每次对 durable Running 重验),
    /// 不套 normal action row 的 launch/write-boundary 分格门;policy/target/
    /// envelope 形状/cwd authority 与普通链一致(不误套 normal
    /// target-only/read-only action 门——envelope 只做形状冻结,recipe 的写
    /// 面由 BootstrapExecutorMarker/receipt 链持有)。产出的 policy 冻结
    /// `RootRecipe(credential)` 相位,revalidate 据此走 recipe 分支。
    pub(crate) fn validate_root_recipe_request(
        &self,
        request: SessionLaunchRequest,
        credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ValidatedSessionLaunchPolicy, ProviderGatewayError> {
        let capability = self
            .capabilities
            .require_root_recipe_supported(&request.provider, credential)?;
        self.enforce_route_policy(&capability)?;
        self.assemble_validated(
            request,
            capability,
            ValidatedSessionLaunchPhase::RootRecipe(credential.clone()),
        )
    }

    /// policy artifact 解析 + target 复验 + envelope 冻结 + cwd authority 早门
    /// + fingerprint 计算的共享装配(Normal/RootRecipe 两相位共用;分格门在
    /// 各自入口先行施加)。
    fn assemble_validated(
        &self,
        request: SessionLaunchRequest,
        capability: ProviderCapability,
        phase: ValidatedSessionLaunchPhase,
    ) -> Result<ValidatedSessionLaunchPolicy, ProviderGatewayError> {
        let artifact = self
            .policies
            .get(&request.project_id)
            .map_err(ProviderGatewayError::policy)?
            .ok_or_else(|| ProviderGatewayError::PolicyMissing(request.project_id.clone()))?;

        let target = self.targets.resolve_and_revalidate(&request)?;

        let now = chrono::Utc::now().to_rfc3339();
        let envelope = SessionPolicyEnvelope::new(
            &artifact,
            request.action,
            target,
            // Task 2.5：冻结请求的独立 canonical cwd（单仓现状=target worktree）。
            request.working_directory.clone(),
            request.readable_roots,
            request.writable_roots,
            capability.adapter_dialect,
            request.config_artifact_ref,
            now,
            self.authority_root.clone(),
        )
        .map_err(ProviderGatewayError::policy)?;

        // Task 2.8（REQ-ENV-11 cwd authority 早门）：envelope 冻结的独立 cwd
        // 必须位于 authority root 允许范围。此处为词法前缀早门（validate 时
        // 点 cwd 目录允许尚不存在）；canonical 复验（symlink 逃逸/外来根）
        // 在 `revalidate_before_spawn` 强制——spawn 不可能绕过。越界即
        // fail-closed，绝不回落 target cwd。
        if !envelope.working_directory.starts_with(&self.authority_root) {
            return Err(ProviderGatewayError::TargetMismatch {
                field: "cwd_authority".to_string(),
            });
        }

        // Task 9a:指纹为五参全上下文计算;裸 validate 无调用者准备的候选
        // 投影,以 capability+envelope 组装的 gateway 基准投影计算(基准
        // 维度=policy/target/cwd/git identity/action evidence/provider 映射
        // 与 exact version/config digest;会话级投影摘要为常量空串,不构成
        // 该路径的漂移维度)。完整上下文路径(`resume_or_start_with_projection`)
        // 以调用者候选投影覆写本基准——仅 envelope 指纹不构成可 spawn 权。
        let basis_projection = Self::lc_basis_projection(&capability, &envelope);
        let fingerprint = SessionResumeFingerprint::from_envelope(
            &envelope,
            &basis_projection,
            &capability.action_capability.projection_digest,
            &canonical_target_git_identity(&envelope.target.worktree),
        );
        // Task 3a/3b:#8 发布链消费(factory 注入 receipt store、Normal 相位
        // 且 artifact 非自举桩时):校验 locator 正文/digest 链并冻结
        // policy_locator 事实,供 spawn 前复验重读。自举桩(存量迁移前)
        // 与 RootRecipe 相位(沿自身 receipt auditor 合同)不在此消费。
        let policy_locator = match (&self.receipts, &phase) {
            (Some(receipts), ValidatedSessionLaunchPhase::Normal)
                if !artifact.is_bootstrap_placeholder() =>
            {
                receipts
                    .latest_finalized(&request.project_id)
                    .map_err(ProviderGatewayError::policy)?
                    .map(|receipt| {
                        verify_published_policy_body(&self.authority_root, &artifact, &receipt)?;
                        Ok::<_, ProviderGatewayError>(PolicyLocatorDigests {
                            policy_digest: receipt.policy_digest,
                            rule_digest: receipt.rule_digest,
                        })
                    })
                    .transpose()?
            }
            _ => None,
        };

        // Task 3b:冻结 action row 的 evidence profile 摘要。
        let projection_digest = capability.action_capability.projection_digest.clone();

        Ok(ValidatedSessionLaunchPolicy {
            envelope,
            fingerprint,
            project_id: request.project_id,
            provider: request.provider,
            action: request.action,
            version: capability.version,
            projection_digest,
            capability_snapshot_ref: capability.capability_snapshot_ref,
            phase,
            policy_locator,
            launch_audit: None,
            lc_projection: None,
            basis_projection,
            execution_contract: None,
        })
    }

    /// 路由级硬门(Task 13;r47 桩化;T12 收口):原「Codex 在
    /// danger-full-access 下路由全阻」规则已退役——danger-full-access 是
    /// LC 全相位(REQ-LCG-04 改版)与 direct Coder 的当前执行基线,LC 会话
    /// 放行由 capability 行(探针 Confirmed)与投影复验裁决;本桩仅保留
    /// 调用形状,不再阻断任何 provider 形态。
    fn enforce_route_policy(
        &self,
        _capability: &ProviderCapability,
    ) -> Result<(), ProviderGatewayError> {
        Ok(())
    }

    /// 裸 validate 的 gateway 基准投影(Task 9a):以 capability+envelope 可
    /// 得材料组装的确定性投影(会话级 role/permission/approval 不进入任何
    /// 指纹维度,`projection_digest` 为常量空串——裸 validate 路径的指纹基准
    /// 不含会话级投影上下文,也不据此授予 resume 权)。完整上下文路径经
    /// `resume_or_start_with_projection` 的调用者候选投影覆写。
    fn lc_basis_projection(
        capability: &ProviderCapability,
        envelope: &SessionPolicyEnvelope,
    ) -> crate::product::logical_codebase::provider_projection::ProviderPolicyProjection {
        use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;
        ProviderPolicyProjection::new(
            capability.provider_type,
            capability.adapter_dialect,
            capability.wire_dialect,
            capability.version.clone(),
            envelope.action,
            crate::protocol::contracts::AdapterRole::Executor,
            crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            None,
            String::new(),
            String::new(),
            envelope.working_directory.clone(),
            envelope.working_directory.clone(),
            envelope.target.clone(),
            envelope.readable_roots.clone(),
            envelope.writable_roots.clone(),
            String::new(),
            envelope.config_digest.clone(),
            String::new(),
            String::new(),
            capability.action_capability.projection_digest.clone(),
            String::new(),
        )
    }

    /// resume 启动判定(Task 13,legacy/单仓语义):据旧会话冻结的 fingerprint
    /// 与当前 validate 产出的 validated policy(envelope+capability 基准指纹)
    /// 比对。只有基准维度全一致(fingerprint 相等)才 `Resume`;任一漂移则
    /// supersede 旧会话(写审计)并 `StartNew`,旧 session id 透传给调用方
    /// 以清理旧会话状态。
    ///
    /// Task 9a:该入口保持既有调用方(SingleRepository/review revision
    /// resume)的原行为;LC 全上下文判定走 `resume_or_start_with_projection`
    /// ——本入口的基准指纹不构成 LC 会话的可 spawn 权来源。
    ///
    /// (T12)旧「路由级 Codex danger 阻断先于 resume 决策」的门已退役;
    /// resume 判定仍晚于 capability/投影复验。
    pub fn resume_or_start(
        &self,
        request: ResumeSessionLaunchRequest,
    ) -> Result<GatewaySessionDisposition, ProviderGatewayError> {
        let validated = self.validate(request.launch)?;
        if validated.fingerprint() == &request.previous_fingerprint {
            Ok(GatewaySessionDisposition::Resume(validated))
        } else {
            self.audit
                .supersede(&request.previous_session_id, RESUME_FINGERPRINT_MISMATCH)?;
            Ok(GatewaySessionDisposition::StartNew {
                validated,
                superseded_session_id: request.previous_session_id,
                waiting: LcSupersededWaiting::superseded(),
            })
        }
    }

    /// LC resume 全上下文判定(Task 9a 冻结签名):调用者先完整 prepare 得
    /// 候选投影再比指纹。gateway 先 validate 请求(policy/target/envelope
    /// 冻结),再施加显式 resume 能力分格与投影身份一致性门,最后以
    /// (envelope, 候选投影, action evidence 摘要, target git identity) 五参
    /// 重算当前指纹与旧会话冻结值比对:
    /// - 相等 → `Resume`,返回的 validated 冻结候选投影与全上下文指纹
    ///   (spawn 前复验与调用方存储同源);
    /// - 不等 → 仅 supersede(审计)+ `StartNew`(等待显式 `StartGeneration`),
    ///   决策层零 spawn,不得用仅 envelope 指纹生成可 spawn 权。
    ///
    /// 显式 resume 的能力分格先行:resume 证据 Unknown/Denied →
    /// `ResumeNotSupported` 稳定拒绝、零 spawn、不静默转 fresh(resume 为
    /// Unknown 不阻止之后的合法显式 fresh)。
    pub fn resume_or_start_with_projection(
        &self,
        request: ResumeSessionLaunchRequest,
        projection: crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
    ) -> Result<GatewaySessionDisposition, ProviderGatewayError> {
        let mut validated = self.validate(request.launch)?;
        // 显式 resume 能力分格(路由级/launch/write-boundary 门已在 validate
        // 内施加)。
        self.capabilities
            .require_resume_supported(&validated.provider, validated.action)?;
        // 候选投影与请求/envelope 的身份一致(provider 映射/action/cwd/
        // target),错配即 fail-closed——防投影与请求错接后产生伪 resume 权。
        Self::assert_projection_matches_envelope(&validated, &projection)?;

        let action_evidence_digest = validated.action_evidence_digest().to_string();
        let target_git_identity =
            canonical_target_git_identity(&validated.envelope().target.worktree);
        let current = SessionResumeFingerprint::from_envelope(
            validated.envelope(),
            &projection,
            &action_evidence_digest,
            &target_git_identity,
        );
        if current == request.previous_fingerprint {
            Ok(GatewaySessionDisposition::Resume(
                validated.with_lc_projection(projection, current),
            ))
        } else {
            self.audit
                .supersede(&request.previous_session_id, RESUME_FINGERPRINT_MISMATCH)?;
            Ok(GatewaySessionDisposition::StartNew {
                validated,
                superseded_session_id: request.previous_session_id,
                waiting: LcSupersededWaiting::superseded(),
            })
        }
    }

    /// 候选投影与 validated envelope 的身份一致性门(同源判定的最小身份
    /// 面:provider 映射/action/cwd/target;完整 admission verdict 归 spawn
    /// 前复验)。
    fn assert_projection_matches_envelope(
        validated: &ValidatedSessionLaunchPolicy,
        projection: &crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
    ) -> Result<(), ProviderGatewayError> {
        let envelope = validated.envelope();
        if validated.provider.provider_type != projection.provider_type() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "provider_identity".to_string(),
            });
        }
        if envelope.provider_dialect != projection.provider_dialect() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "provider_dialect".to_string(),
            });
        }
        if envelope.action != projection.action() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "action".to_string(),
            });
        }
        if envelope.working_directory != *projection.working_directory() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "working_directory".to_string(),
            });
        }
        if envelope.target != *projection.target() {
            return Err(ProviderGatewayError::TargetMismatch {
                field: "target".to_string(),
            });
        }
        Ok(())
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
    /// spawn 前基于 validated policy 与 input 的 effective working directory
    /// (Task 2.5:优先独立 `working_directory`,否则回填 `working_dir`)重新
    /// 复验政策指纹(policy revision/digest、provider version/dialect/capability
    /// snapshot、config digest)、canonical cwd/git-dir/worktree identity、
    /// provider 可用性与 resume 能力。任一复验失败都发生在 registry lookup/
    /// 真实 adapter start 之前(fail-closed)。
    pub async fn start_streaming(
        &self,
        launch: ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderGatewayError> {
        let (mut input, validated) = launch.into_parts();
        // kimi 例外映射(r61 kimi 现场):角色矩阵对策略角色派生的
        // DenyFileWriteBuiltins 在 kimi 侧由既有 ClientServicePolicy 等价
        // 承担(spec「kimi 既有对齐维持」例外)——LC 流式分发唯一漏斗在此
        // 按 provider_type==KimiCode 映射为 None(覆盖全部流式路径,含
        // legacy-reuse),否则 kimi validated start 以
        // provider_generic_tool_policy_forbidden 在极早帧拒绝;其余 intent
        // 原样透传(kimi 自身 fail-closed)。prepare_streaming_launch 的
        // 角色 guard 不变(上游仍按角色矩阵装配)。
        if input.provider_type == crate::protocol::contracts::ProviderType::KimiCode {
            input.tool_policy = crate::cross_cutting::streaming_provider::kimi_exempted_tool_policy(
                input.tool_policy,
            );
        }
        // 批2 Task 20（gateway 只复验 profile/window，不 fallback 补 deny）：
        // Reviewer 启动必须携带非空 ReviewerVerify 窗口合同——缺失即拒
        //（深层窗口有效性由 engine 开窗/assert 承担）。
        if input.role == crate::protocol::contracts::AdapterRole::Reviewer {
            let windowed = input.tool_policy.as_ref().is_some_and(|policy| {
                matches!(
                    &policy.intent,
                    crate::cross_cutting::streaming_provider::ToolPolicyIntent::ReviewerVerify {
                        window_id
                    } if !window_id.trim().is_empty()
                )
            });
            if !windowed {
                return Err(ProviderGatewayError::UnsupportedCapability(
                    "reviewer streaming launch requires a ReviewerVerify window contract"
                        .to_string(),
                ));
            }
        }
        let is_resume = input.resume_provider_session_id.is_some();
        if let Err(error) =
            self.revalidate_before_spawn(&validated, input.effective_working_directory(), is_resume)
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
        // 批2 Task 2:该次启动冻结(或显式缺失)的执行合同摘要进审计。
        let execution_contract_digest = validated
            .frozen_execution()
            .map(|contract| contract.contract_digest().to_string());
        // Task 7(Step 3 分流收口):LC 启动唯一经 `start_validated` 分发,
        // 1b 过渡期的 prepared/raw 分叉就此取消——未经 prepare 的存量
        // validated 构造同样走 validated trait,绝不回落裸 `start`(raw
        // `start` 仅保留单仓 direct)。
        let config_artifact_ref = validated.envelope().config_artifact_ref.clone();
        let session = adapter
            .start_validated(
                ValidatedStreamingProviderInput::new(input, validated),
                cancel,
            )
            .await
            .map_err(ProviderGatewayError::Adapter)?;
        let ConfigSourceAudit {
            argv,
            config_digest,
            ..
        } = ConfigSourceAudit::from_launch(
            &[],
            &config_artifact_ref,
            ConfigSourceProvenance::detect_from_setting_sources(&[]),
        );
        self.audit.record(
            GatewayRunStack::Stream,
            policy_digest,
            Some(config_digest),
            argv,
            execution_contract_digest,
        );
        Ok(session)
    }

    /// 同步运行 adapter。只接受绑定 validated policy 的 input;spawn 前基于
    /// validated policy 与 input 的 effective working directory(Task 2.5:优先
    /// 独立 `working_directory`,否则回填 `worktree_path`)重新复验政策指纹、
    /// canonical cwd/git-dir/worktree identity、provider 可用性与 resume 能力。
    /// 任一复验失败都发生在真实 adapter run 之前(fail-closed)。
    /// Task 1b→Task 7:同步栈只调用 validated trait(`run_validated`——
    /// 生产侧为 `GatewaySyncProvider` streaming→sync bridge);Task 7 已
    /// 收口为 validated-only,任何 validated 构造都不回落 raw `run`。
    pub fn run_sync(
        &self,
        launch: ValidatedAdapterInput,
    ) -> Result<AdapterOutput, ProviderGatewayError> {
        let (input, validated) = launch.into_parts();
        let cwd = input
            .effective_working_directory()
            .ok_or(ProviderGatewayError::MissingCwd)?;
        // 同步 adapter input 不携带 resume session id;同步路径默认非 resume。
        if let Err(error) = self.revalidate_before_spawn(&validated, cwd.as_path(), false) {
            tracing::warn!(
                project_id = %validated.project_id,
                error = %error,
                "provider gateway blocked sync spawn during revalidation"
            );
            return Err(error);
        }
        let policy_digest = validated.envelope().policy_digest.clone();
        // 批2 Task 2:该次启动冻结(或显式缺失)的执行合同摘要进审计。
        let execution_contract_digest = validated
            .frozen_execution()
            .map(|contract| contract.contract_digest().to_string());
        let config_artifact_ref = validated.envelope().config_artifact_ref.clone();
        // Task 7(Step 3 分流收口):同步栈唯一经 `run_validated`(生产侧为
        // `GatewaySyncProvider` streaming→sync bridge,只经 streaming
        // `start_validated` 桥接);1b 过渡期的 prepared/raw 分叉就此取消,
        // 未经 prepare 的存量构造同样走 validated trait,绝不回落裸 `run`。
        let prepared = ValidatedAdapterInput::new(input, validated);
        let output = self
            .sync_adapter
            .run_validated(prepared)
            .map_err(ProviderGatewayError::Adapter)?;
        // Task 11 审计聚合:同 start_streaming,sources/argv 为空(无真实注入源),config
        // digest 由 envelope 冻结的 config_artifact_ref 重算。
        let ConfigSourceAudit {
            argv,
            config_digest,
            ..
        } = ConfigSourceAudit::from_launch(
            &[],
            &config_artifact_ref,
            ConfigSourceProvenance::detect_from_setting_sources(&[]),
        );
        self.audit.record(
            GatewayRunStack::Sync,
            policy_digest,
            Some(config_digest),
            argv,
            execution_contract_digest,
        );
        Ok(output)
    }

    /// Task 1b 冻结接口「准备与同步」:同步栈的 prepare 入口。validate 请求并
    /// 把 `ProviderLaunchAuditContext` 冻结进 validated policy——所有 LC 角色
    /// 在 prepare 前绑定 run-bound sink(含无通用 tool_policy 的 Coder/Kimi,
    /// 统一写 launch audit);`run_sync` 据此分流到 validated trait。
    pub fn prepare_sync_launch(
        &self,
        input: crate::protocol::contracts::AdapterInput,
        request: SessionLaunchRequest,
        context: ProviderLaunchAuditContext,
    ) -> Result<ValidatedAdapterInput, ProviderGatewayError> {
        let validated = self.validate(request)?;
        Ok(ValidatedAdapterInput::new(
            input,
            validated.with_launch_audit(context),
        ))
    }

    /// Task 1b 冻结接口「准备与同步」:流式栈的 prepare 入口。所有 LC 角色
    /// 在 prepare 前绑定 run-bound sink(无通用 tool_policy 的 Coder/Kimi 同样
    /// 统一写 launch audit);非法外来 `Some(policy)` 直接拒绝(双向守卫语义,
    /// 不偷清、不回退裸 start——Task 7 统一 guard 顺序)。
    ///
    /// 批2 Task 11:gateway 不再按 AdapterRole 派生 deny——deny 是 Author
    /// 岗位纪律的唯一 wire 物理投影,只由受信装配铸造(下方
    /// [`Self::prepare_trusted_streaming_launch`] 按 duty 强制投影;普通
    /// prepare 保持 caller 语义并经双向守卫)。Reviewer/Coder/规划族普通
    /// launch wire 面全开。
    pub fn prepare_streaming_launch(
        &self,
        mut input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        request: SessionLaunchRequest,
        context: ProviderLaunchAuditContext,
    ) -> Result<ValidatedStreamingProviderInput, ProviderGatewayError> {
        crate::cross_cutting::streaming_provider::validate_tool_policy_for_role(
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| ProviderGatewayError::UnsupportedCapability(error.to_string()))?;
        let validated = self.validate(request)?;
        input.workspace_session_id = Some(context.workspace_session_id.clone());
        input.audit_sink = Some(
            crate::cross_cutting::tool_policy_audit::RoleRunBoundAuditSink::new(
                context.audit_sink.clone(),
                context.workspace_session_id.clone(),
                context.role_run_seq,
            )
            .into_sink(),
        );
        Ok(ValidatedStreamingProviderInput::new(
            input,
            validated.with_launch_audit(context),
        ))
    }

    /// 批2 Task 2:受信执行的 private launch 面(`pub(crate)`,engine 受信
    /// builder 专用;普通 caller 凭 role 字符串无从铸造)。在既有
    /// `prepare_streaming_launch` 语义(双向守卫/run-bound audit 绑定)之上,
    /// 先按权威装配矩阵冻结执行合同(职责×profile/窗口/岗位纪律
    /// fail-closed,任何失配都在 validate 与 spawn 之前、零副作用拒绝),
    /// 再把合同冻结进 validated launch(指纹按合同摘要重算)。
    ///
    /// 职责来自 `TrustedRunIdentity`(engine durable run/store 构造);prompt/
    /// env/canonical_inputs 里的自报角色不参与装配。legacy Direct 由 engine
    /// 沿同一 builder(`freeze_provider_execution` + 载体绑定)绑等价合同。
    ///
    /// 批2 Task 11:wire `tool_policy` 由受信装配按 duty 强制投影
    /// (`tool_policy_for_duty`:Author → DenyFileWriteBuiltins 岗位纪律
    /// 物理投影,其余职责全开)——caller 传入值被覆盖,不得凭 input 铸造/
    /// 剥离岗位投影;kimi 例外在 start 漏斗统一映射。
    #[cfg_attr(not(test), allow(dead_code))]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_trusted_streaming_launch(
        &self,
        mut input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        request: SessionLaunchRequest,
        context: ProviderLaunchAuditContext,
        identity: &crate::cross_cutting::provider_execution::TrustedRunIdentity,
        profile: crate::cross_cutting::provider_execution::ProviderExecutionProfile,
        author_policy: Option<crate::cross_cutting::provider_execution::AuthorToolPolicy>,
        resource_scope_id: String,
        review_window_id: Option<String>,
    ) -> Result<ValidatedStreamingProviderInput, ProviderGatewayError> {
        Self::assert_trusted_identity_matches_launch(
            identity.workspace_session_id(),
            identity.carrier(),
            &context.workspace_session_id,
            &input.role,
        )?;
        let contract = crate::cross_cutting::provider_execution::freeze_provider_execution(
            identity,
            profile,
            author_policy,
            resource_scope_id,
            review_window_id.clone(),
        )
        .map_err(|error| ProviderGatewayError::UnsupportedCapability(error.details))?;
        // 批2 Task 11→T20:受信装配独占 wire 投影——按 duty+窗口强制覆盖
        // caller 传入的 tool_policy(Author=deny;Reviewer=ReviewerVerify
        // 窗口合同,缺窗 freeze 已拒;其余=None),随后经
        // prepare_streaming_launch 的双向守卫复核。
        input.tool_policy =
            crate::cross_cutting::streaming_provider::tool_policy_for_duty_in_window(
                &identity.duty(),
                review_window_id.as_deref(),
            )
            .map_err(|error| ProviderGatewayError::UnsupportedCapability(error.to_string()))?;
        // A1-b1(Bug45 批2 环境回退):受信装配停止注入环境——子进程经底座
        // (process_manager/cli_adapter)继承宿主环境,caller 的 env_vars
        // 原样透传(仅作显式覆盖层);冻结合同仍绑定 launch
        // (with_frozen_execution),执行基线事实与 resume 指纹不变。
        self.prepare_streaming_launch(input, request, context)
            .map(|launch| launch.with_frozen_execution(contract))
    }

    /// 批2 Task 2:同步栈的受信执行 private launch 面(与
    /// [`Self::prepare_trusted_streaming_launch`] 同一权威装配矩阵与
    /// fail-closed 顺序;Direct/LC、同步/流式共同消费同一冻结合同)。
    #[cfg_attr(not(test), allow(dead_code))]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_trusted_sync_launch(
        &self,
        input: crate::protocol::contracts::AdapterInput,
        request: SessionLaunchRequest,
        context: ProviderLaunchAuditContext,
        identity: &crate::cross_cutting::provider_execution::TrustedRunIdentity,
        profile: crate::cross_cutting::provider_execution::ProviderExecutionProfile,
        author_policy: Option<crate::cross_cutting::provider_execution::AuthorToolPolicy>,
        resource_scope_id: String,
        review_window_id: Option<String>,
    ) -> Result<ValidatedAdapterInput, ProviderGatewayError> {
        Self::assert_trusted_identity_matches_launch(
            identity.workspace_session_id(),
            identity.carrier(),
            &context.workspace_session_id,
            &input.role,
        )?;
        let contract = crate::cross_cutting::provider_execution::freeze_provider_execution(
            identity,
            profile,
            author_policy,
            resource_scope_id,
            review_window_id,
        )
        .map_err(|error| ProviderGatewayError::UnsupportedCapability(error.details))?;
        // A1-b1(Bug45 批2 环境回退):同步栈同源停止注入——caller 的
        // `AdapterInput::env_vars` 原样透传(仅作显式覆盖层),子进程经底座
        // (cli_adapter)继承宿主;冻结合同仍绑定 launch。
        self.prepare_sync_launch(input, request, context)
            .map(|launch| launch.with_frozen_execution(contract))
    }

    /// T10(RevB2T2 ⚠3):受信 identity 与启动载体的一致性硬门——身份归属
    /// 的 workspace 会话必须与本次 run-bound audit 上下文同会话;identity
    /// 的承载角色(wire 面 `carrier`)必须与 input 自带 role 一致。失配在
    /// freeze/validate 之前零副作用拒绝,防跨会话/跨角色身份错接后铸出
    /// 可启动合同(legacy Direct 经 `prepare_trusted_*` 载体接线时同一
    /// 硬门生效)。
    fn assert_trusted_identity_matches_launch(
        identity_session: &str,
        identity_carrier: &crate::protocol::contracts::AdapterRole,
        launch_session: &str,
        input_role: &crate::protocol::contracts::AdapterRole,
    ) -> Result<(), ProviderGatewayError> {
        if identity_session != launch_session {
            return Err(ProviderGatewayError::UnsupportedCapability(format!(
                "provider_execution_identity_session_mismatch: identity={identity_session} launch_context={launch_session}"
            )));
        }
        if identity_carrier != input_role {
            return Err(ProviderGatewayError::UnsupportedCapability(format!(
                "provider_execution_identity_carrier_mismatch: identity={identity_carrier:?} input_role={input_role:?}"
            )));
        }
        Ok(())
    }

    /// Task 3.2(B0a2):LC 全上下文公共 prepare 合同。与
    /// `prepare_streaming_launch` 同源的角色矩阵派生/双向守卫与 run-bound
    /// audit 绑定之上,再读取真实 adapter 的只读 facts(`prepare_lc_facts`,
    /// exact version 消费 60 秒 TTL 合同)、以 B0a1 只读 trust facts 冻结
    /// trust 维度、用 facts 携带的**同版本** projector 装配 gateway 私有
    /// `ProviderProjectionInput` 冻结 full projection,最后施加身份一致门
    /// 与统一无副作用 `admission_verdict`。全程零 spawn/零 durable 写;
    /// 未接入 adapter 得 typed unsupported 拒绝(不伪造 facts)。
    ///
    /// 返回 opaque [`PreparedLcStreamingCandidate`]:resume 判定仍走既有
    /// `resume_or_start_with_projection`(以候选冻结的 full projection 为
    /// 基准);spawn 前复验经 `reobserve_lc_prepare_facts` 重新取当前事实
    /// (version 单独允许 TTL;trust/policy/target 等不得借版本缓存冻结)。
    pub async fn prepare_lc_streaming_candidate(
        &self,
        mut input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        request: SessionLaunchRequest,
        audit: ProviderLaunchAuditContext,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<PreparedLcStreamingCandidate, ProviderGatewayError> {
        // 1. 批2 Task 11:gateway 不再按 AdapterRole 派生 deny(Author 岗位
        //    投影只由受信装配铸造);外来非法 Some(policy) 由双向守卫拒绝。
        crate::cross_cutting::streaming_provider::validate_tool_policy_for_role(
            &input.role,
            input.tool_policy.as_ref(),
        )
        .map_err(|error| ProviderGatewayError::UnsupportedCapability(error.to_string()))?;

        // 2. validate(policy/target/envelope 冻结)+ run-bound audit 绑定。
        let validated = self
            .validate(request.clone())?
            .with_launch_audit(audit.clone());
        input.workspace_session_id = Some(audit.workspace_session_id.clone());
        input.audit_sink = Some(
            crate::cross_cutting::tool_policy_audit::RoleRunBoundAuditSink::new(
                audit.audit_sink.clone(),
                audit.workspace_session_id.clone(),
                audit.role_run_seq,
            )
            .into_sink(),
        );

        // 3. registry 真实 adapter(register_gated 装配面恒为 availability
        //    wrapper;查找先于 facts,未注册即 fail-closed)。
        let provider_name = provider_name_for_dialect(validated.envelope().provider_dialect);
        let adapter = self
            .registry
            .get(&provider_name)
            .ok_or_else(|| ProviderGatewayError::RegistryLookup(format!("{provider_name:?}")))?;

        // 4. adapter 只读 facts(默认 typed unsupported 仅未接入/测试 adapter)。
        let facts = adapter.prepare_lc_facts(&input, &validated, cancel).await?;

        // 5. 只读 trust 维度:注入 source 时以 B0a1 `fingerprint_facts` 的
        //    真实工件摘要冻结(缺/不可信 typed 拒,不冻结假事实);无需
        //    workspace trust 的 provider 冻结「不适用」事实本身;未注入
        //    source 的裸 gateway 维持空串现状(既有测试/旧链零回归)。零写。
        let trust_digest = self.lc_trust_digest(&provider_name, &validated.project_id)?;

        // 6. gateway 私有装配 full projection(不可伪造输入;projector 来自
        //    facts,与 exact_version 同源)。
        let projection_input =
            crate::product::logical_codebase::provider_projection::ProviderProjectionInput::new(
                validated.envelope().clone(),
                validated.provider.clone(),
                validated.envelope().action,
                input.role.clone(),
                input.permission_mode.clone(),
                input.tool_policy.clone(),
                facts.approval_policy().to_string(),
                facts.mcp_bundle_digest().to_string(),
                validated.envelope().config_artifact_ref.clone(),
                trust_digest,
                Some(facts.boundary().clone()),
            );
        let full_projection = facts
            .projector()
            .project(&projection_input)
            .map_err(|error| ProviderGatewayError::UnsupportedCapability(error.to_string()))?;

        // 7. 身份一致门 + 统一无副作用 admission verdict(不写 audit、
        //    不 ensure/revoke、不 spawn)。
        Self::assert_projection_matches_envelope(&validated, &full_projection)?;
        let is_resume = input.resume_provider_session_id.is_some();
        self.admission_verdict(&request, &full_projection, is_resume)?;

        Ok(PreparedLcStreamingCandidate {
            input,
            request,
            validated,
            full_projection,
            facts,
            run_bound_audit: audit,
            adapter,
        })
    }

    /// Task 3.2(B0a2):spawn 前重新取当前 facts——同一 adapter/key 的再次
    /// 只读观测(version 命中 60 秒 TTL 合同;命令 identity 变化则换 key
    /// 再观测)。调用方以 [`PreparedLcStreamingCandidate::assert_frozen_version`]
    /// 与冻结事实比对;trust/policy/target 等其它维度不借版本缓存冻结。
    pub async fn reobserve_lc_prepare_facts(
        &self,
        candidate: &PreparedLcStreamingCandidate,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts,
        ProviderGatewayError,
    > {
        candidate
            .adapter
            .prepare_lc_facts(candidate.input(), candidate.validated(), cancel)
            .await
            .map_err(ProviderGatewayError::Adapter)
    }

    /// Task 3.7(B0f):workspace LC 候选消费面的 resume 收口。
    ///
    /// - `Full`:委托 [`Self::resume_or_start_with_projection`](full 对 full
    ///   比对;指纹漂移 → supersede 审计并等待 `RESUME_FINGERPRINT_MISMATCH`)。
    /// - `Unavailable`(旧 basis/缺 full map):不与 full 指纹混比、不
    ///   supersede 审计未知会话,直接 `StartNew` 等待显式 fresh(稳定
    ///   原因码 `RESUME_FINGERPRINT_UNAVAILABLE`);旧 native id 已知时经
    ///   `superseded_session_id` 透传,未知为空串。
    ///
    /// 两臂返回/冻结的 validated 都携带候选冻结 full projection;后续
    /// spawn 仅经 [`Self::start_lc_prepared_streaming`]。
    pub fn resume_or_start_prepared_lc_candidate(
        &self,
        candidate: &PreparedLcStreamingCandidate,
        basis: PreparedLcResumeBasis,
    ) -> Result<GatewaySessionDisposition, ProviderGatewayError> {
        match basis {
            PreparedLcResumeBasis::Full {
                previous_fingerprint,
                previous_session_id,
            } => {
                // fix-loop P2:无 v2 schema 前缀(legacy/外来 digest)不是
                // 可混比的 full basis——不进比对、不 supersede 从未可信
                // 的会话,直接 unavailable 等待(同 `Unavailable` 待遇)。
                if !previous_fingerprint.is_full_projection_v2() {
                    let superseded_session_id = previous_session_id;
                    return Ok(GatewaySessionDisposition::StartNew {
                        validated: candidate.validated().clone().with_lc_projection(
                            candidate.full_projection().clone(),
                            candidate.full_context_fingerprint(),
                        ),
                        superseded_session_id,
                        waiting: LcSupersededWaiting::fingerprint_unavailable(),
                    });
                }
                self.resume_or_start_with_projection(
                    ResumeSessionLaunchRequest {
                        launch: candidate.request().clone(),
                        previous_fingerprint,
                        previous_session_id,
                    },
                    candidate.full_projection().clone(),
                )
            }
            PreparedLcResumeBasis::Unavailable => {
                let superseded_session_id = candidate
                    .input()
                    .resume_provider_session_id
                    .clone()
                    .unwrap_or_default();
                Ok(GatewaySessionDisposition::StartNew {
                    validated: candidate.validated().clone().with_lc_projection(
                        candidate.full_projection().clone(),
                        candidate.full_context_fingerprint(),
                    ),
                    superseded_session_id,
                    waiting: LcSupersededWaiting::fingerprint_unavailable(),
                })
            }
        }
    }

    /// Task 3.7(B0f):workspace LC PlanningReadOnly full 唯一 start 路径。
    ///
    /// 只消费 [`Self::prepare_lc_streaming_candidate`] 冻结的 opaque 候选
    /// (裸 validated/旧 basis 旁路不可达:候选字段私有、仅 gateway 可构造,
    /// crate 内唯一 spawn 出口归本路径)。spawn 前 fail-closed 链,全部在
    /// 真实 adapter start 之前:
    ///
    /// 1. **相位收口**:仅 `PlanningReadOnly` 收口候选 full;Coding/Recipe
    ///    等既有相位仍走同名 gateway API(`start_streaming` 等),不经本路径。
    /// 2. **重观测当前 facts**(version 单独允许 60 秒 TTL 命中;命令
    ///    identity 变化换 key 再观测)→ **冻结版本比对**(漂移 → typed
    ///    `PolicyDrift(provider_version)`,零 spawn)。
    /// 3. **混合 provider 矩阵守卫**:registry 当前为该 dialect 绑定的
    ///    adapter 必须仍是候选冻结的同一实例(prepare→start 间换绑
    ///    adapter/跨家串接即 typed `PolicyDrift(provider_adapter)`)。
    /// 4. **audit v2 完整性门**(四家同判据):以冻结 full projection 装配
    ///    `LcProviderStartAudit` 字面量并要求 `is_v2_complete` 且会话投影
    ///    摘要非空——v2 材料不完整的 projector 不可达生产 spawn(action/
    ///    wire 文本不入 v2 判据,同源文本归 adapter 字面量)。
    /// 5. **admission verdict + 完整复验**:身份一致门、policy/trust/
    ///    config/cwd/target/git identity/availability 按冻结 full projection
    ///    复验(capability row 摘要与投影 capability 摘要逐字相等由
    ///    admission 第 3 步持有);validated 附带冻结 full projection 后
    ///    `revalidate_before_spawn` 以同一基准重算指纹。
    ///
    /// 全部通过后把 validated(携带冻结 full projection,adapter 侧 audit
    /// 字面量与 prepared full projection 同源、无 basis 旁路)交给候选
    /// 冻结 adapter 的 `start_validated`;成功后与 `start_streaming` 同源
    /// 记录 gateway stream 审计。kimi 通用策略例外映射同样在此施加
    /// (LC 流式分发唯一漏斗)。
    pub async fn start_lc_prepared_streaming(
        &self,
        candidate: PreparedLcStreamingCandidate,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderGatewayError> {
        let validated = candidate.validated();
        // 1. 相位收口(仅 workspace LC PlanningReadOnly)。
        if validated.envelope().action != SessionPolicyAction::PlanningReadOnly {
            return Err(ProviderGatewayError::UnsupportedCapability(
                "lc prepared streaming start converges PlanningReadOnly launches only; \
                 use the existing gateway APIs for other phases"
                    .to_string(),
            ));
        }
        // 2. spawn 前重观测 + 冻结版本比对。
        let current = self
            .reobserve_lc_prepare_facts(&candidate, cancel.clone())
            .await?;
        candidate.assert_frozen_version(&current)?;
        // 3. 混合 provider 矩阵守卫:registry 绑定的 adapter 同一性。
        let registered = self.lookup_real_streaming_adapter(validated)?;
        if !Arc::ptr_eq(&registered, &candidate.adapter) {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "provider_adapter".to_string(),
            });
        }
        // 4. audit v2 完整性门(四家同判据)。
        let fingerprint = candidate.full_context_fingerprint();
        let capability_digest = candidate
            .full_projection
            .capability_projection_digest()
            .to_string();
        let expected_audit = crate::cross_cutting::tool_policy_audit::LcProviderStartAudit {
            action: String::new(),
            wire_dialect: String::new(),
            capability_projection_digest: capability_digest.clone(),
            projection_digest: candidate.full_projection.projection_digest().to_string(),
            boundary_evidence_ref: candidate
                .full_projection
                .boundary_evidence_ref()
                .to_string(),
            resume_fingerprint: fingerprint.digest.clone(),
            action_evidence_digest: capability_digest,
            target_git_identity: canonical_target_git_identity(
                &validated.envelope().target.worktree,
            ),
        };
        if !expected_audit.is_v2_complete() || expected_audit.projection_digest.trim().is_empty() {
            return Err(ProviderGatewayError::UnsupportedCapability(
                "lc start audit v2 material incomplete; refusing to spawn".to_string(),
            ));
        }
        // 5. 身份一致门 + admission verdict + 完整复验(指纹基准 = 候选
        //    full projection,与冻结指纹逐字一致)。
        let mut input = candidate.input.clone();
        let is_resume = input.resume_provider_session_id.is_some();
        Self::assert_projection_matches_envelope(validated, &candidate.full_projection)?;
        self.admission_verdict(&candidate.request, &candidate.full_projection, is_resume)?;
        let validated_full = candidate
            .validated
            .clone()
            .with_lc_projection(candidate.full_projection.clone(), fingerprint);
        self.revalidate_before_spawn(
            &validated_full,
            input.effective_working_directory(),
            is_resume,
        )?;
        // kimi 例外映射(与 `start_streaming` 同源;LC 流式分发唯一漏斗)。
        if input.provider_type == crate::protocol::contracts::ProviderType::KimiCode {
            input.tool_policy = crate::cross_cutting::streaming_provider::kimi_exempted_tool_policy(
                input.tool_policy,
            );
        }
        // 批2 Task 20(gateway 只复验,不 fallback 补 deny;与 start_streaming
        // 同款):Reviewer LC 启动必须携带非空 ReviewerVerify 窗口合同。
        if input.role == crate::protocol::contracts::AdapterRole::Reviewer {
            let windowed = input.tool_policy.as_ref().is_some_and(|policy| {
                matches!(
                    &policy.intent,
                    crate::cross_cutting::streaming_provider::ToolPolicyIntent::ReviewerVerify {
                        window_id
                    } if !window_id.trim().is_empty()
                )
            });
            if !windowed {
                return Err(ProviderGatewayError::UnsupportedCapability(
                    "reviewer lc streaming launch requires a ReviewerVerify window contract"
                        .to_string(),
                ));
            }
        }
        let policy_digest = validated_full.envelope().policy_digest.clone();
        // 批2 Task 2:该次启动冻结(或显式缺失)的执行合同摘要进审计。
        let execution_contract_digest = validated_full
            .frozen_execution()
            .map(|contract| contract.contract_digest().to_string());
        let config_artifact_ref = validated_full.envelope().config_artifact_ref.clone();
        let session = candidate
            .adapter
            .start_validated(
                crate::cross_cutting::session_launch::ValidatedStreamingProviderInput::new(
                    input,
                    validated_full,
                ),
                cancel,
            )
            .await
            .map_err(ProviderGatewayError::Adapter)?;
        let ConfigSourceAudit {
            argv,
            config_digest,
            ..
        } = ConfigSourceAudit::from_launch(
            &[],
            &config_artifact_ref,
            ConfigSourceProvenance::detect_from_setting_sources(&[]),
        );
        self.audit.record(
            GatewayRunStack::Stream,
            policy_digest,
            Some(config_digest),
            argv,
            execution_contract_digest,
        );
        Ok(session)
    }

    /// 只读 trust 维度装配(B0a1 `ProviderTrustFingerprintFacts` 消费面)。
    /// 零写:不 ensure/revoke、不追加 trust 审计;不造无适用 trust 记录。
    fn lc_trust_digest(
        &self,
        provider_name: &crate::product::models::ProviderName,
        project_id: &str,
    ) -> Result<String, ProviderGatewayError> {
        let Some(trust) = self.trust.as_ref() else {
            return Ok(String::new());
        };
        if !crate::product::logical_codebase::provider_trust::requires_workspace_trust(
            provider_name,
        ) {
            return Ok(format!(
                "lc-trust-v1:{provider_name:?}:workspace_trust_not_required"
            ));
        }
        let facts = trust.fingerprint_facts(
            project_id,
            self.lc_scope.as_deref().unwrap_or(""),
            provider_name,
            &self.authority_root,
        )?;
        Ok(format!(
            "lc-trust-v1:{provider_name:?}:{}",
            facts.record_digest.as_deref().unwrap_or("no_record_digest")
        ))
    }

    /// spawn 前完整复验(B-1)。逐维度比对 envelope 冻结值与 spawn 时点的真实值,
    /// 阶段顺序遵循 Task 3 冻结合同:identity/manifest(authority root)→
    /// policy body/artifact/receipt → provider mapping/version/action
    /// capability(含 write_boundary(D4)与 resume 追加分格)→ trust source →
    /// config digest → canonical cwd/logical roots → target/git identity →
    /// availability。role/tool 策略 guard 由 Task 7 统一接线:
    ///
    /// 1. **authority root**(Task 3b):envelope 冻结的 authority 与 gateway
    ///    冻结的 manifest root canonical 相等(防 validate→spawn 间换根/跨
    ///    gateway 投影漂移)。
    /// 2. **policy 指纹**:重新加载 store 中的 `AggregatePolicyArtifact`,比对
    ///    `policy_revision` 与 `policy_digest`(防 validate→spawn 间政策被升级);
    ///    validate 冻结了 #8 发布链 locator digest 时,重读 locator/AGENTS
    ///    原字节比对(正文/规则篡改 → `PolicyDrift`)。
    /// 3. **provider 能力指纹**:重新查询 capability source,比对 `version`、
    ///    `adapter_dialect`、`capability_snapshot_ref` 与 action row 的
    ///    evidence profile 摘要(`projection_digest`),并据当前
    ///    artifact+capability 重算 `SessionResumeFingerprint` 与冻结值逐字
    ///    一致(防 provider 被替换)。
    /// 4. **config digest**:据 envelope 的 `config_artifact_ref` 重算 digest,
    ///    与 envelope 冻结的 `config_digest` 一致(防托管配置被篡改)。
    /// 5. **resume 追加**(Task 2b B-2 + Task 3b):fresh 与 resume 都施加
    ///    write_boundary(D4)分格;resume 在此之上追加 resume 分格
    ///    (`require_resume_supported`),否则 fail-closed 为
    ///    `ResumeNotSupported`(不得静默转 fresh)。
    /// 6. **trust source**(Task 3b):factory 注入只读 source 且 provider
    ///    需要用户级 workspace trust 时重验;绝不 ensure/revoke。
    /// 7. **canonical cwd 权威**(Task 2.8):重新 canonicalize spawn cwd 与
    ///    envelope 冻结的独立 `working_directory`,不一致返回
    ///    `TargetMismatch { field: "cwd" }`;冻结 cwd 越出 authority root
    ///    返回 `TargetMismatch { field: "cwd_authority" }`。cwd 与 target 是
    ///    独立维度,cwd≠target 的合法分离形态放行;target identity 由 8 复验。
    /// 8. **target/git identity**(REQ-ENV-03):resolver 重新解析复验。
    /// 9. **availability**:调用 availability gate,不可用返回 `ProviderUnavailable`。
    ///
    /// 任一维度漂移都发生在 registry lookup 之前。路由级 fail-closed 不等于 OS
    /// 级隔离:本复验是 supervised 场景下的 TOCTOU 门禁,不宣称物理不可写。
    pub(crate) fn revalidate_before_spawn(
        &self,
        validated: &ValidatedSessionLaunchPolicy,
        cwd: &Path,
        is_resume: bool,
    ) -> Result<(), ProviderGatewayError> {
        let envelope = validated.envelope();

        // 0. authority root 复验(Task 3b:identity/manifest 阶段)。
        let canonical_envelope_authority =
            envelope.authority_root.canonicalize().map_err(|error| {
                ProviderGatewayError::Target(format!(
                    "canonicalize envelope authority root {}: {error}",
                    envelope.authority_root.display()
                ))
            })?;
        let canonical_authority = self.authority_root.canonicalize().map_err(|error| {
            ProviderGatewayError::Target(format!(
                "canonicalize authority root {}: {error}",
                self.authority_root.display()
            ))
        })?;
        if canonical_envelope_authority != canonical_authority {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "authority_root".to_string(),
            });
        }

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

        // 1b. #8 发布链正文复验(Task 3a/3b):validate 冻结了 locator digest
        //     事实时,spawn 前重读 locator/AGENTS 原字节比对。
        if let Some(locator) = &validated.policy_locator {
            let raw_body =
                std::fs::read(canonical_authority.join(&artifact.policy_id)).map_err(|error| {
                    ProviderGatewayError::Target(format!(
                        "re-read policy locator {}: {error}",
                        artifact.policy_id
                    ))
                })?;
            let body_digest = format!("sha256:{:x}", Sha256::digest(&raw_body));
            if body_digest != locator.policy_digest || body_digest != artifact.digest {
                return Err(ProviderGatewayError::PolicyDrift {
                    dimension: "policy_body".to_string(),
                });
            }
            let rule_bytes =
                std::fs::read(canonical_authority.join(
                    crate::product::logical_codebase::root_recipe_receipt::ROOT_RULE_ENTRY_FILE,
                ))
                .map_err(|error| {
                    ProviderGatewayError::Target(format!("re-read root rule entry: {error}"))
                })?;
            let rule_digest = format!("sha256:{:x}", Sha256::digest(&rule_bytes));
            if rule_digest != locator.rule_digest {
                return Err(ProviderGatewayError::PolicyDrift {
                    dimension: "rule_digest".to_string(),
                });
            }
        }

        // 2. 重新查询能力,按冻结相位分流(Task 2b 第二段):
        //    - RootRecipe:重新消费固定 recipe 事实——冻结凭据再次对 durable
        //      Running 重验(每次重验),不消费 normal launch/write/resume 分格
        //      (root-recipe 不误套 normal action 门);
        //    - Normal:fresh 与 resume 都施加 write_boundary(D4)分格;resume
        //      在此之上追加 resume 分格(仅 Confirmed 放行,不静默转 fresh)。
        let capability = match &validated.phase {
            ValidatedSessionLaunchPhase::RootRecipe(credential) => self
                .capabilities
                .require_root_recipe_supported(&validated.provider, credential)?,
            ValidatedSessionLaunchPhase::Normal => {
                let capability = self
                    .capabilities
                    .require_supported(&validated.provider, validated.action)?;
                self.capabilities
                    .require_write_boundary(&validated.provider, validated.action)?;
                if is_resume {
                    self.capabilities
                        .require_resume_supported(&validated.provider, validated.action)?;
                }
                capability
            }
        };
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
        if capability.action_capability.projection_digest != validated.projection_digest {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "projection_digest".to_string(),
            });
        }
        // Task 9a:按 validated 冻结的指纹基准重算——完整上下文路径冻结了
        // 候选投影则按投影重算,裸 validate 基准则按 gateway 基准投影重算;
        // 任一输入维度的 spawn 时点漂移都会在此 fail-closed。
        let mut fingerprint_basis = validated
            .lc_projection()
            .cloned()
            .unwrap_or_else(|| Self::lc_basis_projection(&capability, envelope));
        // 批2 Task 2/T10:绑定执行合同的 policy 按合同 **resume 稳定摘要**
        // 装饰同一基准(与 `with_frozen_execution`/`with_lc_projection` 的
        // 绑定重算同装饰;摘要不含 run id——run 级身份换新不得在 spawn
        // 复验误报 resume_fingerprint 漂移,纪律事实漂移仍失配)。
        if let Some(contract) = validated.execution_contract.as_ref() {
            fingerprint_basis =
                fingerprint_basis.with_execution_digest(contract.contract_resume_digest());
        }
        let current_fingerprint = SessionResumeFingerprint::from_envelope(
            envelope,
            &fingerprint_basis,
            &capability.action_capability.projection_digest,
            &canonical_target_git_identity(&envelope.target.worktree),
        );
        if current_fingerprint.digest != validated.fingerprint.digest {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "resume_fingerprint".to_string(),
            });
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

        // 4b. 只读 trust 复验(Task 3b:trust source 阶段)。factory 注入
        //     source 且 provider 需要用户级 workspace trust 时重验;同一
        //     source 供 GET/early 与 spawn 复验,绝不 ensure/revoke、不写
        //     durable audit。RootRecipe 相位的 trust 硬前置由
        //     ensure_ready 合同持有,不在此重复施加。
        let provider_name = provider_name_for_dialect(envelope.provider_dialect);
        if let Some(trust) = &self.trust
            && crate::product::logical_codebase::provider_trust::requires_workspace_trust(
                &provider_name,
            )
            && matches!(validated.phase, ValidatedSessionLaunchPhase::Normal)
        {
            let verification = trust.verify_trusted(
                &validated.project_id,
                self.lc_scope.as_deref().unwrap_or(""),
                &provider_name,
                &self.authority_root,
            )?;
            if !verification.trusted {
                return Err(ProviderGatewayError::ProviderUnavailable(format!(
                    "provider trust not established for {provider_name:?}: {}",
                    verification.detail
                )));
            }
        }

        // 5. canonical cwd 权威复验（Task 2.8：拆除「canonical cwd ==
        //    canonical target」错误等式——cwd 与 target 是两个独立维度，
        //    cwd≠target 的合法分离形态放行）。
        //    a) spawn 时点 cwd 必须仍与 envelope 冻结的独立 canonical
        //       working_directory 一致（validate→spawn 间被调包 →
        //       fail-closed；Task 2.5 冻结字段在此消费）；
        //    b) 冻结 cwd canonicalize 后必须位于 authority root 允许范围
        //       （REQ-ENV-11）——symlink 逃逸/外来根（root factory 不一致）
        //       零 spawn。target 的 canonical/git identity 复验由 5b 独立
        //       保留，不受 cwd 维度影响。
        let canonical_cwd = cwd.canonicalize().map_err(|error| {
            ProviderGatewayError::Target(format!("canonicalize cwd {}: {error}", cwd.display()))
        })?;
        let canonical_frozen_cwd = envelope.working_directory.canonicalize().map_err(|error| {
            ProviderGatewayError::Target(format!(
                "canonicalize working directory {}: {error}",
                envelope.working_directory.display()
            ))
        })?;
        if canonical_cwd != canonical_frozen_cwd {
            return Err(ProviderGatewayError::TargetMismatch {
                field: "cwd".to_string(),
            });
        }
        let canonical_authority = self.authority_root.canonicalize().map_err(|error| {
            ProviderGatewayError::Target(format!(
                "canonicalize authority root {}: {error}",
                self.authority_root.display()
            ))
        })?;
        if !canonical_frozen_cwd.starts_with(&canonical_authority) {
            return Err(ProviderGatewayError::TargetMismatch {
                field: "cwd_authority".to_string(),
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
            // Task 2.5：重建请求携带 envelope 冻结的同一 canonical cwd
            //（resume request 必须携带同一 cwd，见设计 §2.4）。
            working_directory: envelope.working_directory.clone(),
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

        // 9. availability gate。
        let provider_name = provider_name_for_dialect(envelope.provider_dialect);
        self.availability_gate
            .ensure_available(&provider_name)
            .map_err(|error| ProviderGatewayError::unavailable(error.to_string()))?;
        Ok(())
    }

    /// Task 3b:统一无副作用 admission verdict——按冻结阶段顺序对
    /// (request, projection) 对完整复验当前 durable 事实。GET/early/
    /// 复验面消费;不写 audit、不 ensure/revoke、不 spawn。
    ///
    /// 阶段顺序:identity/manifest(cwd canonical 必须等于 manifest root,
    /// 不能仅 prefix 允许子目录)→ policy body/artifact/receipt(+config
    /// digest 重算比对)→ provider mapping/version/action capability(含
    /// write_boundary(D4)与 resume 追加分格)→ trust source(只读)→
    /// role/tool(Task 7 统一 guard 接线,本层不另造第二套判定)→
    /// logical roots → projection/adapter(registry 存在性)→ availability
    /// → boundary/D4(coding projection 必须携带 boundary evidence 引用)。
    pub fn admission_verdict(
        &self,
        request: &SessionLaunchRequest,
        projection: &crate::product::logical_codebase::provider_projection::ProviderPolicyProjection,
        is_resume: bool,
    ) -> Result<(), ProviderGatewayError> {
        // (projection getter 均 pub(crate),直接消费冻结字段。)

        // 1. identity/manifest:provider 映射与 action 一致。
        if request.provider.provider_type != projection.provider_type() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "provider_identity".to_string(),
            });
        }
        if request.action != projection.action() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "action".to_string(),
            });
        }
        // cwd canonical 全等:projection cwd == manifest root canonical,
        // 且请求 cwd 与 projection cwd canonical 相等。
        crate::product::logical_codebase::assert_canonical_lc_root_consistent(
            None,
            None,
            &self.authority_root,
            projection.working_directory(),
        )?;
        let canonical_request_cwd = request.working_directory.canonicalize().map_err(|error| {
            ProviderGatewayError::Target(format!(
                "canonicalize request cwd {}: {error}",
                request.working_directory.display()
            ))
        })?;
        let canonical_projection_cwd =
            projection
                .working_directory()
                .canonicalize()
                .map_err(|error| {
                    ProviderGatewayError::Target(format!(
                        "canonicalize projection cwd {}: {error}",
                        projection.working_directory().display()
                    ))
                })?;
        if canonical_request_cwd != canonical_projection_cwd {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "cwd".to_string(),
            });
        }

        // 2. policy body/artifact/receipt + config/MCP 通道。
        let artifact = self
            .policies
            .get(&request.project_id)
            .map_err(ProviderGatewayError::policy)?
            .ok_or_else(|| ProviderGatewayError::PolicyMissing(request.project_id.clone()))?;
        if let Some(receipts) = &self.receipts
            && let Some(receipt) = receipts
                .latest_finalized(&request.project_id)
                .map_err(ProviderGatewayError::policy)?
            && !artifact.is_bootstrap_placeholder()
        {
            verify_published_policy_body(&self.authority_root, &artifact, &receipt)?;
        }
        let config_digest =
            SessionPolicyEnvelope::recompute_config_digest(&request.config_artifact_ref)
                .map_err(ProviderGatewayError::policy)?;
        if config_digest != projection.config_digest() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "config_digest".to_string(),
            });
        }

        // 3. provider mapping/version/action capability(+resume 追加)。
        let capability = self
            .capabilities
            .require_supported(&request.provider, request.action)?;
        self.capabilities
            .require_write_boundary(&request.provider, request.action)?;
        if is_resume {
            self.capabilities
                .require_resume_supported(&request.provider, request.action)?;
        }
        self.enforce_route_policy(&capability)?;
        if capability.version != projection.exact_version() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "provider_version".to_string(),
            });
        }
        if capability.adapter_dialect != projection.provider_dialect() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "provider_dialect".to_string(),
            });
        }
        if capability.wire_dialect != projection.wire_dialect() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "wire_dialect".to_string(),
            });
        }
        if capability.action_capability.projection_digest
            != projection.capability_projection_digest()
        {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "projection_digest".to_string(),
            });
        }

        // 4. trust source(只读;需要 workspace trust 的 provider)。
        let provider_name = provider_name_for_dialect(capability.adapter_dialect);
        if let Some(trust) = &self.trust
            && crate::product::logical_codebase::provider_trust::requires_workspace_trust(
                &provider_name,
            )
        {
            let verification = trust.verify_trusted(
                &request.project_id,
                self.lc_scope.as_deref().unwrap_or(""),
                &provider_name,
                &self.authority_root,
            )?;
            if !verification.trusted {
                return Err(ProviderGatewayError::ProviderUnavailable(format!(
                    "provider trust not established for {provider_name:?}: {}",
                    verification.detail
                )));
            }
        }

        // 5. role/tool:Task 7 统一 guard 接线,本层无独立事实源。

        // 6. logical roots:projection 冻结的读写根与请求一致(canonical)。
        let canonical_roots = |roots: &[PathBuf]| -> Result<Vec<PathBuf>, ProviderGatewayError> {
            roots
                .iter()
                .map(|root| {
                    root.canonicalize().map_err(|error| {
                        ProviderGatewayError::Target(format!(
                            "canonicalize logical root {}: {error}",
                            root.display()
                        ))
                    })
                })
                .collect()
        };
        if canonical_roots(&request.readable_roots)?
            != canonical_roots(projection.readable_roots())?
            || canonical_roots(&request.writable_roots)?
                != canonical_roots(projection.writable_roots())?
        {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "logical_roots".to_string(),
            });
        }

        // 7. projection/adapter:registry 必须有该 provider 的真实 adapter。
        self.registry
            .get(&provider_name)
            .ok_or_else(|| ProviderGatewayError::RegistryLookup(format!("{provider_name:?}")))?;

        // 8. availability。
        self.availability_gate
            .ensure_available(&provider_name)
            .map_err(|error| ProviderGatewayError::unavailable(error.to_string()))?;

        // 9. boundary/D4:coding projection 必须携带 boundary evidence 引用。
        if matches!(request.action, SessionPolicyAction::CodingTargetWrite)
            && projection.boundary_evidence_ref().is_empty()
        {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "boundary_evidence".to_string(),
            });
        }
        Ok(())
    }

    /// Task 3b:early action 资格判定——只消费 durable capability 证据
    /// (launch 分格),返回不携未来 target/worktree 或 D4 事实的两枚引用;
    /// attempt 未创建(无 worktree/target/D4 事实)即可判定,全程零写入
    /// (不写 policy/capability/trust/audit,不 ensure/revoke)。
    ///
    /// `role`/`permission_mode` 随冻结签名保留:role/tool 策略 guard 由
    /// Task 7 统一接线,本层不另造第二套判定。
    pub fn action_admission_verdict(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
        _role: &crate::protocol::contracts::AdapterRole,
        _permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode,
    ) -> Result<ProviderActionAdmission, ProviderAdmissionError> {
        let capability = self
            .capabilities
            .require_supported(provider, action)
            .map_err(admission_waiting_from_gateway)?;
        Ok(ProviderActionAdmission {
            capability_snapshot_ref: capability.capability_snapshot_ref.clone(),
            projection_ref: capability.action_capability.projection_digest.clone(),
        })
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
/// `ProviderName`。dialect 与 provider 类型一一对应。admission 预检的 trust
/// 维度分类(6.1)与 spawn 复验共用,不复制第二份。
pub(crate) fn provider_name_for_dialect(dialect: ProviderDialect) -> ProviderName {
    match dialect {
        ProviderDialect::ClaudeCodeCliV1 => ProviderName::ClaudeCode,
        ProviderDialect::CodexCliV1 => ProviderName::Codex,
        ProviderDialect::PiRpcV1 => ProviderName::Pi,
        ProviderDialect::KimiAcpV1 => ProviderName::KimiCode,
    }
}

/// (T12,REQ-LCG-04 改版)历史诊断解码冻结码:改版前存量 capability 记录
/// 迁移等待原因 `codex_danger_full_access_unsupported` 的逐字解码依赖
/// (provider_capability_store 迁移码表/automation preflight 历史行诊断)。
/// 当前启动路径不再产生本码——danger-full-access 是 LC 全相位的合法基线,
/// 新合法 danger 绝不判 unsupported。
pub const CODEX_DANGER_FULL_ACCESS_UNSUPPORTED: &str = "codex_danger_full_access_unsupported";

/// C-2 集中映射的 fail-closed 判别码:`ProviderName` 不属于 gateway 支持的
/// ClaudeCode/Codex 真实 dialect 时,`ProviderRef::from_provider_name` 返回的
/// `UnsupportedCapability` 错误以此为前缀,后接 provider 名。
pub const PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH: &str = "provider_unsupported_for_gateway_launch";

// ---- Task 2b(lcg_t02):capability 分格门与 root-recipe 相位的稳定错误码 ----

/// 正常 action row 的 launch 分格非 `Confirmed` 时的稳定判别码。
pub const PROVIDER_CAPABILITY_LAUNCH_NOT_CONFIRMED: &str =
    "provider_capability_launch_not_confirmed";

/// 正常 action row 的 write_boundary 分格非 `Confirmed` 时的稳定判别码。
pub const PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED: &str =
    "provider_capability_write_boundary_not_confirmed";

/// root-recipe 相位仅接受固定 Claude recipe provider 的稳定判别码。
pub const PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE: &str =
    "root_recipe_requires_fixed_claude_provider";

/// root-recipe 凭据对 durable Running operation 重核验失败时的稳定判别码。
pub const PROVIDER_ROOT_RECIPE_CREDENTIAL_RECHECK_DENIED: &str =
    "root_recipe_credential_recheck_denied";

/// source 未携带 durable 重核验通道(无 paths/lc scope)时的稳定判别码。
pub const PROVIDER_ROOT_RECIPE_CREDENTIAL_RECHECK_UNAVAILABLE: &str =
    "root_recipe_credential_recheck_unavailable";

/// 已交付 recipe evidence 钉定版本与当前记录版本漂移时的稳定判别码。
pub const PROVIDER_ROOT_RECIPE_EVIDENCE_VERSION_DRIFT: &str = "root_recipe_evidence_version_drift";

/// 把 gateway 校验错误映射为 admission waiting 事实(与 preflight `check`
/// 的映射同型;early verdict 只读,不产生 store 错误面)。
fn admission_waiting_from_gateway(error: ProviderGatewayError) -> ProviderAdmissionError {
    use crate::product::logical_codebase::provider_admission_preflight::BootstrapActionKind;
    let reason_code = match &error {
        ProviderGatewayError::UnsupportedCapability(_) => "provider_capability_not_satisfied",
        ProviderGatewayError::PolicyMissing(_) => "aggregate_policy_artifact_missing",
        _ => "provider_gateway_denied",
    };
    ProviderAdmissionError::Waiting {
        reason_code: reason_code.to_string(),
        detail: error.to_string(),
        missing_materials: Vec::new(),
        allowed_actions: vec![BootstrapActionKind::Revalidate, BootstrapActionKind::Retry],
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

// ---------------------------------------------------------------------------
// Task 3a(lcg_t03):#8 已发布政策正文的只读校验
// ---------------------------------------------------------------------------

/// 校验 #8 发布的最终政策正文与 digest 链(只读,零副作用)。
///
/// 消费契约(Global Constraints #8):gateway 只消费最终
/// `artifact.policy_text` 原始 UTF-8 字节(= canonical root 下 `policy_id`
/// locator 文件原字节)。本函数逐项核对:
///
/// 1. `policy_id` 是安全的相对 locator(非绝对路径、无 `..`/`.` 组件);
/// 2. locator 解析无 symlink 逃逸:从 canonical root 逐组件走查,中间组件
///    必须是真实目录、终组件必须是普通文件(拒绝任何 symlink 形态);
/// 3. locator 原始字节 == `artifact.policy_text` 原始 UTF-8 字节;
/// 4. digest 链三方一致:`artifact.digest == receipt.policy_digest ==
///    SHA-256(locator 原字节)`;
/// 5. receipt 冻结的 `canonical_root` 与 `authority_root` canonical 相等,
///    且 artifact 的 `policy_id`/`revision` 自洽(id 以
///    `policy/{project_id}/{logical_codebase_id}/{revision}` 组成);
/// 6. `rule_digest` 独立读取 canonical root `AGENTS.md` 原字节重算比对
///    (与 policy digest 不要求相等)。
///
/// 任何缺失/漂移 fail-closed 为 `Target`(IO/形态)或 `PolicyDrift`(digest
/// 链维度);本函数绝不写文件、不物化 locator/AGENTS/成员规则副本。
pub fn verify_published_policy_body(
    authority_root: &Path,
    artifact: &crate::product::logical_codebase::policy::AggregatePolicyArtifact,
    receipt: &crate::product::logical_codebase::RootRecipeReceipt,
) -> Result<(), ProviderGatewayError> {
    // 1. policy_id 必须是安全相对 locator。
    let locator_relative = Path::new(&artifact.policy_id);
    if locator_relative.is_absolute() || artifact.policy_id.is_empty() {
        return Err(ProviderGatewayError::Target(format!(
            "policy id {} is not a safe relative locator",
            artifact.policy_id
        )));
    }
    let components: Vec<std::path::Component> = locator_relative.components().collect();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ProviderGatewayError::Target(format!(
            "policy id {} contains non-normal path components",
            artifact.policy_id
        )));
    }

    // 2. receipt 与 authority 的 canonical root 一致。
    let canonical_root = authority_root.canonicalize().map_err(|error| {
        ProviderGatewayError::Target(format!(
            "canonicalize authority root {}: {error}",
            authority_root.display()
        ))
    })?;
    let canonical_receipt_root = receipt.canonical_root.canonicalize().map_err(|error| {
        ProviderGatewayError::Target(format!(
            "canonicalize receipt root {}: {error}",
            receipt.canonical_root.display()
        ))
    })?;
    if canonical_root != canonical_receipt_root {
        return Err(ProviderGatewayError::PolicyDrift {
            dimension: "policy_receipt_root".to_string(),
        });
    }

    // 3. artifact 的 policy_id/revision 自洽(id 组成即 revision 尾缀)。
    let expected_id = format!(
        "policy/{}/{}/{}",
        artifact.project_id, artifact.logical_codebase_id, artifact.revision
    );
    if artifact.policy_id != expected_id {
        return Err(ProviderGatewayError::PolicyDrift {
            dimension: "policy_artifact_identity".to_string(),
        });
    }

    // 4. locator 逐组件走查:无 symlink 逃逸,终组件是普通文件。
    let mut locator = canonical_root.clone();
    let last_index = components.len().saturating_sub(1);
    for (index, component) in components.iter().enumerate() {
        locator.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&locator).map_err(|error| {
            ProviderGatewayError::Target(format!(
                "read policy locator {}: {error}",
                locator.display()
            ))
        })?;
        if metadata.is_symlink() {
            return Err(ProviderGatewayError::PolicyDrift {
                dimension: "policy_locator_symlink".to_string(),
            });
        }
        if index == last_index {
            if !metadata.is_file() {
                return Err(ProviderGatewayError::Target(format!(
                    "policy locator {} is not a regular file",
                    locator.display()
                )));
            }
        } else if !metadata.is_dir() {
            return Err(ProviderGatewayError::Target(format!(
                "policy locator parent {} is not a directory",
                locator.display()
            )));
        }
    }

    // 5. 原始字节比对 + digest 链三方一致。
    let raw_body = std::fs::read(&locator).map_err(|error| {
        ProviderGatewayError::Target(format!("read policy body {}: {error}", locator.display()))
    })?;
    if raw_body != artifact.policy_text.as_bytes() {
        return Err(ProviderGatewayError::PolicyDrift {
            dimension: "policy_body".to_string(),
        });
    }
    let body_digest = format!("sha256:{:x}", Sha256::digest(&raw_body));
    if artifact.digest != body_digest || receipt.policy_digest != body_digest {
        return Err(ProviderGatewayError::PolicyDrift {
            dimension: "policy_digest_chain".to_string(),
        });
    }

    // 6. rule digest 独立读取 AGENTS.md 原字节重算。
    let rule_entry = canonical_root
        .join(crate::product::logical_codebase::root_recipe_receipt::ROOT_RULE_ENTRY_FILE);
    let rule_bytes = std::fs::read(&rule_entry).map_err(|error| {
        ProviderGatewayError::Target(format!(
            "read root rule entry {}: {error}",
            rule_entry.display()
        ))
    })?;
    let rule_digest = format!("sha256:{:x}", Sha256::digest(&rule_bytes));
    if receipt.rule_digest != rule_digest {
        return Err(ProviderGatewayError::PolicyDrift {
            dimension: "rule_digest".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "provider_gateway_tests.rs"]
pub(crate) mod tests;
