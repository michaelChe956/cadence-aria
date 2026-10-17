//! 受信执行 profile、岗位策略与冻结执行快照（批2 Task 2，REQ-ENV-HOST-01）。
//!
//! 四家 provider 全角色应用层宿主全开的受信类型基座：`TrustedProviderRole`
//! 六职责、`ProviderExecutionProfile::{HostCoding, ReviewerVerify}` 与
//! `FrozenProviderExecution` 冻结快照由 crate 内受信 builder
//! （[`freeze_provider_execution`]）从不可伪造的 [`TrustedRunIdentity`] 冻结；
//! T3（白名单环境）/T5（资源归属）/T10–T13（四家接线）/T17/T20 共同消费。
//!
//! 不可伪造性合同（对齐 `session_launch`/`ValidatedSessionLaunchPolicy` 的
//! crate 内 constructor + private 字段范式）：
//! - [`TrustedRunIdentity`] 只能由现有 engine durable run/store 经
//!   `pub(crate)` 构造（`from_durable_run`），不实现 `Deserialize`——serde
//!   输入、prompt、env、canonical_inputs 自报 role 都无法铸造身份；
//! - [`FrozenProviderExecution`] 字段全 private、无 public constructor、不实现
//!   `Deserialize`——serde 输入不能直接铸造启动能力；
//! - 职责×profile 走权威装配矩阵（六职责表），参数不是开放给模型的配置：
//!   Author/Orchestrator/Splitter/Handoff/Coder → `HostCoding`（仅 Author 另带
//!   [`AuthorToolPolicy`] 岗位纪律），Reviewer → `ReviewerVerify` 且必须持窗口；
//! - `deployment_context` 本期恒 `HostLocal`（R14 二期最小接缝：只进受信冻结
//!   快照，不扩公开 API/DTO；未知 context 显式拒绝，不静默 fallback）。
//!
//! 岗位纪律（REQ-ENV-09）：`AuthorToolPolicy` 表示独立岗位（不装配编辑代码
//! 文件的内建工具），不复用 sandbox/审批档（`ProviderToolPolicy`/permission
//! mode）。双向守卫：Author 缺岗位策略 fail-closed；非 Author 被装配 author
//! deny 同样 fail-closed（不以清空/错位装配取得全开）。

use std::collections::{BTreeMap, BTreeSet};

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::protocol::contracts::{AdapterRole, ProviderRunRecord};
use sha2::{Digest, Sha256};

/// 冻结合同的 schema 版本：字段集合或字段语义变化时递增，旧摘要与新摘要
/// 永不相等。
pub const PROVIDER_EXECUTION_CONTRACT_SCHEMA: &str = "provider-execution-contract/1";

/// resume 稳定摘要的 schema 域版本(T10):与全量合同摘要的域集合不同
/// (无 `provider_run_id` 域),二者永不相等;纪律事实演进时递增。
pub const PROVIDER_EXECUTION_CONTRACT_RESUME_SCHEMA: &str = "provider-execution-contract-resume/1";

/// 执行合同自身的 profile 版本（本期冻结为 1）。
pub const PROVIDER_EXECUTION_PROFILE_VERSION: u32 = 1;

/// 环境策略版本（T2 冻结时点）：环境白名单合同由 T3 冻结并复用本版本源；
/// T2 先把执行基线事实（部署上下文/profile/职责/资源归属）纳入环境策略
/// 摘要，任何一项漂移即失配。
pub const PROVIDER_ENV_POLICY_VERSION: u32 = 1;

/// 环境策略摘要的 schema 前缀（与 [`PROVIDER_ENV_POLICY_VERSION`] 同源冻结）。
pub const PROVIDER_ENV_POLICY_SCHEMA: &str = "provider-execution-env-policy/1";

/// 六职责（权威装配矩阵）。职责来自受信装配（engine durable run/store 侧
/// 构造 [`TrustedRunIdentity`] 时显式声明），不由 prompt、env、ACP 或
/// canonical_inputs 里的自报 role 提升。Author 不在 `AdapterRole` 中——它经
/// Orchestrator 等承载角色启动时，audit 面仍记录 `Author`（见
/// [`FrozenProviderExecution::audit_facts`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrustedProviderRole {
    Author,
    Orchestrator,
    Splitter,
    Handoff,
    Coder,
    Reviewer,
}

impl TrustedProviderRole {
    /// 稳定职责标签（audit/冻结事实面；与枚举变体 snake_case 同名）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Orchestrator => "orchestrator",
            Self::Splitter => "splitter",
            Self::Handoff => "handoff",
            Self::Coder => "coder",
            Self::Reviewer => "reviewer",
        }
    }
}

/// 两执行 profile（权威装配矩阵）：`HostCoding` = 规划/编码宿主执行，
/// `ReviewerVerify` = 审查验证（必须持 B 档窗口）。Auto/Supervised 审批语义
/// 不代替 profile；四家应用层不施加 bwrap/netns/no-new-privs。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderExecutionProfile {
    HostCoding,
    ReviewerVerify,
}

impl ProviderExecutionProfile {
    /// 稳定 profile 标签（audit/冻结事实面）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HostCoding => "host_coding",
            Self::ReviewerVerify => "reviewer_verify",
        }
    }

    /// 该 profile 是否要求 review 窗口（仅 `ReviewerVerify`）。
    pub fn requires_review_window(&self) -> bool {
        matches!(self, Self::ReviewerVerify)
    }
}

/// 部署上下文（R14 二期 dev-sandbox 最小接缝）：本期唯一 `HostLocal`，只在
/// 受信冻结快照出现，不扩公开 API/DTO。二期部署 launcher 实现新 context 时
/// 必须同时覆盖 provider+本地 MCP+ACP terminal/fs；未知 context 显式拒绝，
/// 不静默按 HostLocal fallback。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeploymentContext {
    HostLocal,
}

impl DeploymentContext {
    /// 稳定部署上下文标签（冻结事实面）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HostLocal => "host_local",
        }
    }

    /// 受信侧解析部署上下文标签：仅认精确 token `host_local`，其余未知
    /// context 一律显式拒绝（不静默 fallback，不宽容别名/大小写）。
    pub fn from_trusted_str(value: &str) -> Result<Self, ProviderAdapterError> {
        match value {
            "host_local" => Ok(Self::HostLocal),
            other => Err(ProviderAdapterError::provider_unavailable(format!(
                "unknown deployment context rejected: {other}"
            ))),
        }
    }
}

/// Author 岗位纪律版本（岗位规则演进时递增并同步审计）。
pub const AUTHOR_TOOL_POLICY_VERSION: u32 = 1;

/// Author 岗位纪律（REQ-ENV-09）：独立岗位策略，不复用 sandbox 档
/// （`ProviderToolPolicy`）或审批模式。语义 = 「不装配编辑代码文件的内建
/// 工具」这一可验证能力合同；四家 adapter（T11–T13）把语义集合映射到各自
/// 的具体内建工具名，岗位规则不依赖 bwrap/路径白名单/HOME 重定向。
///
/// 字段 private：唯一公开构造 [`Self::author_v1`] 产出固定 v1 语义集合——
/// 空集合/降级集合无法构造（不以「清空 author policy」取得全开）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorToolPolicy {
    policy_version: u32,
    denied_builtin_tools: BTreeSet<String>,
}

impl AuthorToolPolicy {
    /// 当前（v1）author 岗位规则：拒绝「编辑代码文件」的内建工具语义集合
    /// （adapter 侧映射到具体工具名：Edit/Write/apply_patch/代码编辑 fs 服务）。
    pub fn author_v1() -> Self {
        Self {
            policy_version: AUTHOR_TOOL_POLICY_VERSION,
            denied_builtin_tools: BTreeSet::from([
                "builtin_edit_code_file".to_string(),
                "builtin_write_code_file".to_string(),
            ]),
        }
    }

    /// 岗位规则版本。
    pub fn policy_version(&self) -> u32 {
        self.policy_version
    }

    /// 拒绝的内建工具语义集合（audit 面冻结事实）。
    pub fn denied_builtin_tools(&self) -> &BTreeSet<String> {
        &self.denied_builtin_tools
    }

    /// 岗位摘要（Author 岗位冻结进 [`FrozenProviderExecution`] 的事实）。
    #[cfg_attr(not(test), allow(dead_code))]
    fn digest(&self) -> String {
        hash_domains(
            "provider-author-tool-policy/1",
            &[
                self.policy_version.to_be_bytes().to_vec(),
                self.denied_builtin_tools
                    .iter()
                    .flat_map(|tool| tool.as_bytes().to_vec())
                    .collect(),
            ],
        )
    }

    /// 岗位规则合法性（freeze 内消费）：版本必须是受支持的当前版本且拒绝
    /// 集合非空（REQ-ENV-09 双向守卫的「非法」臂）。
    #[cfg_attr(not(test), allow(dead_code))]
    fn validate(&self) -> Result<(), ProviderAdapterError> {
        if self.policy_version != AUTHOR_TOOL_POLICY_VERSION {
            return Err(ProviderAdapterError::provider_unavailable(format!(
                "provider_execution_author_policy_invalid: unsupported version {}",
                self.policy_version
            )));
        }
        if self.denied_builtin_tools.is_empty() {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_execution_author_policy_invalid: empty denied set".to_string(),
            ));
        }
        Ok(())
    }
}

/// 受信 run 身份：engine durable run/store 在 crate 内构造的不可伪造身份
/// 事实。**不实现 `Deserialize`**、无 public constructor——serde 输入/外部
/// caller 无法铸造身份，进而无法铸造启动能力。
///
/// 职责（`duty`）由受信装配显式声明（Author 不在 `AdapterRole` 中，必须由
/// engine 侧受信状态给出）；run/会话/能力画像引用事实取自 durable
/// `ProviderRunRecord` 与 engine 会话存储。`carrier` 记录承载角色：Author
/// 经 Orchestrator 承载时 audit 面仍记录 `Author` 职责。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedRunIdentity {
    provider_run_id: String,
    workspace_session_id: String,
    duty: TrustedProviderRole,
    carrier: AdapterRole,
    native_permission_profile_ref: String,
}

impl TrustedRunIdentity {
    /// 现有 engine durable run/store 的 crate 内构造口：run 身份事实取自
    /// durable `ProviderRunRecord`（provider_run_id、adapter 承载角色、能力
    /// 画像引用），会话归属由 engine 会话存储给出，职责由受信装配声明。
    /// 生产 caller（LC gateway 与 legacy Direct 的受信 engine builder,T10+）
    /// 都沿 durable run/store 取事实;不从 canonical_inputs 里的自报 role
    /// 提升。engine 接线前仅合同测试消费(前瞻载体,同
    /// `PreparedLcStreamingCandidate::into_validated_streaming_input` 范式)。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn from_durable_run(
        record: &ProviderRunRecord,
        workspace_session_id: &str,
        duty: TrustedProviderRole,
    ) -> Self {
        Self {
            provider_run_id: record.provider_run_id.clone(),
            workspace_session_id: workspace_session_id.to_string(),
            duty,
            carrier: record.adapter_role.clone(),
            native_permission_profile_ref: record.provider_capability_ref.clone(),
        }
    }

    /// durable provider run id。
    pub fn provider_run_id(&self) -> &str {
        &self.provider_run_id
    }

    /// 归属的 workspace 会话 id。
    pub fn workspace_session_id(&self) -> &str {
        &self.workspace_session_id
    }

    /// 受信真实职责（audit 面；Author 经 Orchestrator 承载时仍为 Author）。
    pub fn duty(&self) -> TrustedProviderRole {
        self.duty
    }

    /// 承载角色（wire 面 `AdapterRole`，可能与职责不同名）。
    pub fn carrier(&self) -> &AdapterRole {
        &self.carrier
    }

    /// 原生权限画像引用（durable run 冻结的能力画像 ref）。
    pub fn native_permission_profile_ref(&self) -> &str {
        &self.native_permission_profile_ref
    }
}

/// 冻结执行快照（批2 Task 2 载体）：由 crate 内受信 builder
/// [`freeze_provider_execution`] 从 [`TrustedRunIdentity`] 冻结。字段全
/// private、无 public constructor、**不实现 `Deserialize`**——serde 输入不能
/// 直接铸造启动能力；validated launch 绑定后经 getter 只读消费。
///
/// 冻结事实：profile 版本、profile、受信真实职责与承载角色、
/// `deployment_context=HostLocal`、环境策略版本/摘要、resource_scope_id、
/// 原生权限画像引用、Author 岗位摘要（仅 Author）、可选 review_window_id
/// （仅 Reviewer）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenProviderExecution {
    profile_version: u32,
    profile: ProviderExecutionProfile,
    duty: TrustedProviderRole,
    carrier: AdapterRole,
    provider_run_id: String,
    workspace_session_id: String,
    deployment_context: DeploymentContext,
    env_policy_version: u32,
    env_policy_digest: String,
    resource_scope_id: String,
    native_permission_profile_ref: String,
    author_policy_digest: Option<String>,
    review_window_id: Option<String>,
    contract_digest: String,
}

impl FrozenProviderExecution {
    /// 执行合同 profile 版本（本期 1）。
    pub fn profile_version(&self) -> u32 {
        self.profile_version
    }

    /// 冻结的执行 profile。
    pub fn profile(&self) -> ProviderExecutionProfile {
        self.profile
    }

    /// 受信真实职责（audit 面）。
    pub fn duty(&self) -> TrustedProviderRole {
        self.duty
    }

    /// 承载角色（wire 面；Author 经 Orchestrator 承载时此处为 Orchestrator）。
    pub fn carrier(&self) -> &AdapterRole {
        &self.carrier
    }

    /// 归属的 durable provider run id。
    pub fn provider_run_id(&self) -> &str {
        &self.provider_run_id
    }

    /// 归属的 workspace 会话 id。
    pub fn workspace_session_id(&self) -> &str {
        &self.workspace_session_id
    }

    /// 部署上下文（本期恒 `HostLocal`）。
    pub fn deployment_context(&self) -> DeploymentContext {
        self.deployment_context
    }

    /// 环境策略版本（T3 白名单环境消费/续冻结）。
    pub fn env_policy_version(&self) -> u32 {
        self.env_policy_version
    }

    /// 环境策略摘要（T2 冻结执行基线事实；T3 起纳入白名单键集合摘要）。
    pub fn env_policy_digest(&self) -> &str {
        &self.env_policy_digest
    }

    /// 资源归属主键（T5 `ProviderRunResourceScope` 消费：scope_id 维度）。
    pub fn resource_scope_id(&self) -> &str {
        &self.resource_scope_id
    }

    /// 原生权限画像引用（与 identity 的 durable 能力画像同源冻结）。
    pub fn native_permission_profile_ref(&self) -> &str {
        &self.native_permission_profile_ref
    }

    /// Author 岗位摘要（仅 Author 职责冻结 `Some`）。
    pub fn author_policy_digest(&self) -> Option<&str> {
        self.author_policy_digest.as_deref()
    }

    /// review 窗口 id（仅 Reviewer 职责冻结 `Some`）。
    pub fn review_window_id(&self) -> Option<&str> {
        self.review_window_id.as_deref()
    }

    /// 全量合同摘要（resume 指纹/projection 的非敏感事实维度；任何冻结字段的
    /// 漂移都会改变该摘要）。
    pub fn contract_digest(&self) -> &str {
        &self.contract_digest
    }

    /// resume 稳定摘要(T10,REQ-ENV-HOST-01 接线前置):覆盖全部**纪律
    /// 事实**(profile/职责/承载/部署上下文/环境策略/资源归属/原生画像/
    /// 岗位摘要/窗口),**不含 `provider_run_id`**——同一 workspace 会话的
    /// 每次 run 换新 run id,run 级身份事实不得进入 resume 指纹维度,
    /// 否则每次 resume 都 supersede(复验指纹摘要消费本方法;审计面仍用
    /// 全量 [`Self::contract_digest`],run id 漂移在审计可见)。
    pub(crate) fn contract_resume_digest(&self) -> String {
        hash_domains(
            PROVIDER_EXECUTION_CONTRACT_RESUME_SCHEMA,
            &[
                self.profile_version.to_be_bytes().to_vec(),
                self.profile.as_str().as_bytes().to_vec(),
                self.duty.as_str().as_bytes().to_vec(),
                format!("{:?}", self.carrier).into_bytes(),
                self.workspace_session_id.as_bytes().to_vec(),
                self.deployment_context.as_str().as_bytes().to_vec(),
                self.env_policy_version.to_be_bytes().to_vec(),
                self.env_policy_digest.as_bytes().to_vec(),
                self.resource_scope_id.as_bytes().to_vec(),
                self.native_permission_profile_ref.as_bytes().to_vec(),
                self.author_policy_digest
                    .as_deref()
                    .unwrap_or("")
                    .as_bytes()
                    .to_vec(),
                self.review_window_id
                    .as_deref()
                    .unwrap_or("")
                    .as_bytes()
                    .to_vec(),
            ],
        )
    }

    /// audit 面非敏感事实快照（durable 审计/诊断消费；不含任何凭据值）。
    /// Author 经 Orchestrator 承载时 `duty=author`、`carrier=orchestrator`。
    pub fn audit_facts(&self) -> BTreeMap<String, String> {
        let mut facts = BTreeMap::new();
        facts.insert(
            "profile_version".to_string(),
            self.profile_version.to_string(),
        );
        facts.insert("profile".to_string(), self.profile.as_str().to_string());
        facts.insert("duty".to_string(), self.duty.as_str().to_string());
        facts.insert(
            "carrier".to_string(),
            format!("{:?}", self.carrier).to_lowercase(),
        );
        facts.insert("provider_run_id".to_string(), self.provider_run_id.clone());
        facts.insert(
            "workspace_session_id".to_string(),
            self.workspace_session_id.clone(),
        );
        facts.insert(
            "deployment_context".to_string(),
            self.deployment_context.as_str().to_string(),
        );
        facts.insert(
            "env_policy_version".to_string(),
            self.env_policy_version.to_string(),
        );
        facts.insert(
            "env_policy_digest".to_string(),
            self.env_policy_digest.clone(),
        );
        facts.insert(
            "resource_scope_id".to_string(),
            self.resource_scope_id.clone(),
        );
        facts.insert(
            "native_permission_profile_ref".to_string(),
            self.native_permission_profile_ref.clone(),
        );
        if let Some(digest) = &self.author_policy_digest {
            facts.insert("author_policy_digest".to_string(), digest.clone());
        }
        if let Some(window) = &self.review_window_id {
            facts.insert("review_window_id".to_string(), window.clone());
        }
        facts.insert("contract_digest".to_string(), self.contract_digest.clone());
        facts
    }
}

/// crate 内受信 builder：从 [`TrustedRunIdentity`] 按权威装配矩阵冻结执行
/// 合同。职责×profile 矩阵 fail-closed：
///
/// | 职责 | 合法 profile | 窗口 | 岗位策略 |
/// |---|---|---|---|
/// | Author | `HostCoding` | 无 | 必须 `Some`（合法 v1） |
/// | Orchestrator/Splitter/Handoff/Coder | `HostCoding` | 无 | 必须 `None` |
/// | Reviewer | `ReviewerVerify` | 必须 `Some`（非空） | 必须 `None` |
///
/// 任何失配都在 spawn 之前拒绝（`ProviderAdapterError`，稳定原因码前缀
/// `provider_execution_*`）；产物嵌入 private validated launch/Direct 受信
/// 包装，不给普通 caller 凭 role 字符串铸能力。
/// 生产消费链:`prepare_trusted_*_launch`(LC)与 T10+ 受信 engine builder
/// (legacy Direct 等价合同);接线完成前仅合同测试消费(前瞻载体范式)。
#[cfg_attr(not(test), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn freeze_provider_execution(
    identity: &TrustedRunIdentity,
    profile: ProviderExecutionProfile,
    author_policy: Option<AuthorToolPolicy>,
    resource_scope_id: String,
    review_window_id: Option<String>,
) -> Result<FrozenProviderExecution, ProviderAdapterError> {
    let duty = identity.duty();

    // 权威装配矩阵:职责×profile(六职责表)。任何失配都在受信装配面拒绝——
    // profile 不来自 prompt/env/自报,Reviewer 缺窗口不能建可启动合同。
    let window = match (duty, profile) {
        (TrustedProviderRole::Reviewer, ProviderExecutionProfile::ReviewerVerify) => {
            let has_window = review_window_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty());
            if !has_window {
                return Err(ProviderAdapterError::provider_unavailable(
                    "provider_execution_review_window_required: duty=reviewer profile=reviewer_verify"
                        .to_string(),
                ));
            }
            review_window_id.clone()
        }
        (TrustedProviderRole::Reviewer, ProviderExecutionProfile::HostCoding) => {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_execution_profile_mismatch: duty=reviewer profile=host_coding"
                    .to_string(),
            ));
        }
        (_, ProviderExecutionProfile::ReviewerVerify) => {
            return Err(ProviderAdapterError::provider_unavailable(format!(
                "provider_execution_profile_mismatch: duty={} profile=reviewer_verify",
                duty.as_str()
            )));
        }
        (_, ProviderExecutionProfile::HostCoding) => None,
    };
    // 到达此处即 profile=HostCoding 且职责≠Reviewer:窗口只属 Reviewer,
    // 非 Reviewer 职责携带窗口 = 装配错位,fail-closed。
    if window.is_none() && review_window_id.is_some() {
        return Err(ProviderAdapterError::provider_unavailable(format!(
            "provider_execution_review_window_not_allowed: duty={}",
            duty.as_str()
        )));
    }

    // 岗位纪律双向守卫(REQ-ENV-09):Author 必须带合法岗位策略;非 Author
    // 不得装配 author deny;不以清空/错位装配取得全开。
    let author_policy_digest = match (duty, author_policy) {
        (TrustedProviderRole::Author, None) => {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_execution_author_policy_required: duty=author".to_string(),
            ));
        }
        (TrustedProviderRole::Author, Some(policy)) => {
            policy.validate()?;
            Some(policy.digest())
        }
        (_, Some(_)) => {
            return Err(ProviderAdapterError::provider_unavailable(format!(
                "provider_execution_author_policy_misassigned: duty={}",
                duty.as_str()
            )));
        }
        (_, None) => None,
    };

    // 资源归属主键必填(T5 ProviderRunResourceScope 的 scope_id 维度)。
    if resource_scope_id.trim().is_empty() {
        return Err(ProviderAdapterError::provider_unavailable(
            "provider_execution_resource_scope_required".to_string(),
        ));
    }

    // deployment_context 本期恒 HostLocal(R14:只进受信冻结快照,不开放为
    // 可配置参数;未知 context 由 `DeploymentContext::from_trusted_str` 在
    // 二期受信 launcher 侧显式拒绝)。
    let deployment_context = DeploymentContext::HostLocal;

    // 环境策略摘要:T2 冻结执行基线事实(部署上下文/profile/职责/资源归属);
    // T3 白名单环境在同一版本源上续冻结键集合摘要。
    let env_policy_digest = hash_domains(
        PROVIDER_ENV_POLICY_SCHEMA,
        &[
            PROVIDER_ENV_POLICY_VERSION.to_be_bytes().to_vec(),
            deployment_context.as_str().as_bytes().to_vec(),
            profile.as_str().as_bytes().to_vec(),
            duty.as_str().as_bytes().to_vec(),
            resource_scope_id.as_bytes().to_vec(),
        ],
    );

    let contract = FrozenProviderExecution {
        profile_version: PROVIDER_EXECUTION_PROFILE_VERSION,
        profile,
        duty,
        carrier: identity.carrier().clone(),
        provider_run_id: identity.provider_run_id().to_string(),
        workspace_session_id: identity.workspace_session_id().to_string(),
        deployment_context,
        env_policy_version: PROVIDER_ENV_POLICY_VERSION,
        env_policy_digest: env_policy_digest.clone(),
        resource_scope_id,
        native_permission_profile_ref: identity.native_permission_profile_ref().to_string(),
        author_policy_digest,
        review_window_id: window,
        contract_digest: String::new(),
    };

    // 全量合同摘要覆盖全部冻结字段(含 env 摘要/岗位摘要/窗口),任何漂移
    // 都改变摘要——resume 指纹/projection 以此为非敏感事实维度。
    let contract_digest = hash_domains(
        PROVIDER_EXECUTION_CONTRACT_SCHEMA,
        &[
            contract.profile_version.to_be_bytes().to_vec(),
            contract.profile.as_str().as_bytes().to_vec(),
            contract.duty.as_str().as_bytes().to_vec(),
            format!("{:?}", contract.carrier).into_bytes(),
            contract.provider_run_id.as_bytes().to_vec(),
            contract.workspace_session_id.as_bytes().to_vec(),
            contract.deployment_context.as_str().as_bytes().to_vec(),
            contract.env_policy_version.to_be_bytes().to_vec(),
            contract.env_policy_digest.as_bytes().to_vec(),
            contract.resource_scope_id.as_bytes().to_vec(),
            contract.native_permission_profile_ref.as_bytes().to_vec(),
            contract
                .author_policy_digest
                .as_deref()
                .unwrap_or("")
                .as_bytes()
                .to_vec(),
            contract
                .review_window_id
                .as_deref()
                .unwrap_or("")
                .as_bytes()
                .to_vec(),
        ],
    );
    let mut contract = contract;
    contract.contract_digest = contract_digest;
    Ok(contract)
}

/// 长度前缀域哈希（与 `SessionResumeFingerprint::from_envelope` 同型：8 字节
/// 大端长度前缀 + 域字节，schema 前置；无边界歧义）。
#[cfg_attr(not(test), allow(dead_code))]
fn hash_domains(schema: &str, domains: &[Vec<u8>]) -> String {
    let mut hasher = Sha256::new();
    let mut domain = |part: &[u8]| {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    };
    domain(schema.as_bytes());
    for part in domains {
        domain(part);
    }
    format!("{schema}:sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn durable_run_record(run_id: &str, carrier: AdapterRole) -> ProviderRunRecord {
        ProviderRunRecord {
            provider_run_id: run_id.to_string(),
            node_id: "N2".to_string(),
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            runtime_role: crate::protocol::contracts::RuntimeRole::Orchestrator,
            adapter_role: carrier,
            provider_capability_ref: "cap_claude_code_1_4_0".to_string(),
            adapter_compatibility_ref: "compat_1".to_string(),
            context_package_ref: "ctx_1".to_string(),
            adapter_input_ref: "ain_1".to_string(),
            adapter_output_ref: "aout_1".to_string(),
            raw_artifact_refs: Vec::new(),
            exit_code: Some(0),
            error_code: None,
            error_details: None,
            stdout_ref: None,
            stderr_ref: None,
            structured_output_ref: None,
            files_modified: Vec::new(),
            status: crate::protocol::contracts::ProviderRunStatus::Running,
            started_at: "2026-10-17T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
            retry_count: 0,
            approval_policy: crate::protocol::contracts::ApprovalPolicy::OnRequest,
            sandbox_mode: crate::protocol::contracts::SandboxMode::FullAccess,
            constraint_check_ref: None,
            traceability_binding_refs: Vec::new(),
        }
    }

    fn identity(duty: TrustedProviderRole, carrier: AdapterRole) -> TrustedRunIdentity {
        TrustedRunIdentity::from_durable_run(
            &durable_run_record("run_0001", carrier),
            "ws_session_0001",
            duty,
        )
    }

    /// 六职责×两 profile 权威表（Step 1 红测①）：Author 与四规划/编码职责
    /// 全部 `HostCoding`（Author 另带岗位策略），仅 Reviewer 为
    /// `ReviewerVerify` 且要求窗口。
    #[test]
    fn authority_table_assigns_host_coding_to_author_and_planning_coding_duties() {
        let host_coding_duties = [
            (TrustedProviderRole::Author, true),
            (TrustedProviderRole::Orchestrator, false),
            (TrustedProviderRole::Splitter, false),
            (TrustedProviderRole::Handoff, false),
            (TrustedProviderRole::Coder, false),
        ];
        for (duty, is_author) in host_coding_duties {
            let carrier = match duty {
                TrustedProviderRole::Author | TrustedProviderRole::Orchestrator => {
                    AdapterRole::Orchestrator
                }
                TrustedProviderRole::Splitter => AdapterRole::WorkItemSplitter,
                TrustedProviderRole::Handoff => AdapterRole::Handoff,
                TrustedProviderRole::Coder | TrustedProviderRole::Reviewer => AdapterRole::Executor,
            };
            let author_policy = is_author.then(AuthorToolPolicy::author_v1);
            let frozen = freeze_provider_execution(
                &identity(duty, carrier),
                ProviderExecutionProfile::HostCoding,
                author_policy,
                "scope_run_0001".to_string(),
                None,
            )
            .unwrap_or_else(|error| panic!("duty {:?} must freeze: {error}", duty));
            assert_eq!(frozen.profile(), ProviderExecutionProfile::HostCoding);
            assert_eq!(frozen.profile_version(), PROVIDER_EXECUTION_PROFILE_VERSION);
            assert_eq!(frozen.duty(), duty);
            assert_eq!(frozen.deployment_context(), DeploymentContext::HostLocal);
            assert_eq!(frozen.env_policy_version(), PROVIDER_ENV_POLICY_VERSION);
            assert_eq!(frozen.resource_scope_id(), "scope_run_0001");
            assert!(frozen.review_window_id().is_none());
            assert_eq!(
                frozen.native_permission_profile_ref(),
                "cap_claude_code_1_4_0"
            );
        }
    }

    /// ReviewerVerify 仅属 Reviewer 且必须持非空窗口；空窗口同样拒绝。
    #[test]
    fn reviewer_verify_requires_window_and_is_reviewer_only() {
        let reviewer = identity(TrustedProviderRole::Reviewer, AdapterRole::Reviewer);
        let ok = freeze_provider_execution(
            &reviewer,
            ProviderExecutionProfile::ReviewerVerify,
            None,
            "scope_run_0001".to_string(),
            Some("window_0001".to_string()),
        )
        .expect("reviewer with window must freeze");
        assert_eq!(ok.duty(), TrustedProviderRole::Reviewer);
        assert_eq!(ok.review_window_id(), Some("window_0001"));
        assert!(ok.author_policy_digest().is_none());

        for missing_window in [None, Some(String::new()), Some("  ".to_string())] {
            let error = freeze_provider_execution(
                &reviewer,
                ProviderExecutionProfile::ReviewerVerify,
                None,
                "scope_run_0001".to_string(),
                missing_window,
            )
            .expect_err("reviewer verify without a real window must fail closed");
            assert!(
                error.details.contains("review_window_required"),
                "unexpected details: {}",
                error.details
            );
        }
    }

    /// 职责×profile 失配全部 fail-closed：Reviewer 不能 HostCoding，非
    /// Reviewer 不能 ReviewerVerify。
    #[test]
    fn duty_profile_mismatches_fail_closed() {
        let reviewer = identity(TrustedProviderRole::Reviewer, AdapterRole::Reviewer);
        let error = freeze_provider_execution(
            &reviewer,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_run_0001".to_string(),
            None,
        )
        .expect_err("reviewer host_coding is not an assembly matrix row");
        assert!(error.details.contains("profile_mismatch"));

        let coder = identity(TrustedProviderRole::Coder, AdapterRole::Executor);
        let error = freeze_provider_execution(
            &coder,
            ProviderExecutionProfile::ReviewerVerify,
            None,
            "scope_run_0001".to_string(),
            Some("window_0001".to_string()),
        )
        .expect_err("reviewer_verify is reserved for the reviewer duty");
        assert!(error.details.contains("profile_mismatch"));
    }

    /// 岗位纪律双向守卫：Author 缺策略 fail-closed；非 Author 被装配 author
    /// deny 同样 fail-closed；非法（空集合/旧版本）策略拒绝。
    #[test]
    fn author_policy_bidirectional_guard() {
        let author = identity(TrustedProviderRole::Author, AdapterRole::Orchestrator);
        let error = freeze_provider_execution(
            &author,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_run_0001".to_string(),
            None,
        )
        .expect_err("author without duty policy must fail closed");
        assert!(error.details.contains("author_policy_required"));

        let coder = identity(TrustedProviderRole::Coder, AdapterRole::Executor);
        let error = freeze_provider_execution(
            &coder,
            ProviderExecutionProfile::HostCoding,
            Some(AuthorToolPolicy::author_v1()),
            "scope_run_0001".to_string(),
            None,
        )
        .expect_err("non-author duty must not carry the author policy");
        assert!(error.details.contains("author_policy_misassigned"));

        let mut invalid = AuthorToolPolicy::author_v1();
        invalid.denied_builtin_tools.clear();
        let error = freeze_provider_execution(
            &author,
            ProviderExecutionProfile::HostCoding,
            Some(invalid),
            "scope_run_0001".to_string(),
            None,
        )
        .expect_err("cleared author policy is not a valid duty rule");
        assert!(error.details.contains("author_policy_invalid"));

        let mut old_version = AuthorToolPolicy::author_v1();
        old_version.policy_version = 0;
        let error = freeze_provider_execution(
            &author,
            ProviderExecutionProfile::HostCoding,
            Some(old_version),
            "scope_run_0001".to_string(),
            None,
        )
        .expect_err("unsupported author policy version must fail closed");
        assert!(error.details.contains("author_policy_invalid"));
    }

    /// 非 Reviewer 职责携带窗口 = 装配错位，fail-closed（窗口只属 Reviewer）。
    #[test]
    fn non_reviewer_duty_cannot_carry_review_window() {
        let coder = identity(TrustedProviderRole::Coder, AdapterRole::Executor);
        let error = freeze_provider_execution(
            &coder,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_run_0001".to_string(),
            Some("window_0001".to_string()),
        )
        .expect_err("review window belongs to the reviewer duty only");
        assert!(error.details.contains("review_window_not_allowed"));
    }

    /// 资源归属主键必填（T5 消费 scope_id 维度；空 scope 是装配缺陷）。
    #[test]
    fn empty_resource_scope_id_is_rejected() {
        let coder = identity(TrustedProviderRole::Coder, AdapterRole::Executor);
        for empty_scope in [String::new(), "  ".to_string()] {
            let error = freeze_provider_execution(
                &coder,
                ProviderExecutionProfile::HostCoding,
                None,
                empty_scope,
                None,
            )
            .expect_err("resource scope id is a mandatory freeze fact");
            assert!(error.details.contains("resource_scope_required"));
        }
    }

    /// prompt/env 伪造无效（Step 1 红测②，载体层）：freeze 的输入面没有任何
    /// prompt/env 通道——伪造的角色/profile/窗口字符串最多充当无语义的
    /// resource_scope_id，不可能改变职责、profile 或窗口事实；职责只能来自
    /// identity 的受信装配。
    #[test]
    fn forged_strings_cannot_elevate_duty_profile_or_window() {
        let coder = identity(TrustedProviderRole::Coder, AdapterRole::Executor);
        // 伪造字符串只作为 scope id 进入（无语义位置）：
        let frozen = freeze_provider_execution(
            &coder,
            ProviderExecutionProfile::HostCoding,
            None,
            "role=reviewer&profile=reviewer_verify&window=win-forged".to_string(),
            None,
        )
        .expect("forged strings in the scope slot do not affect assembly");
        assert_eq!(frozen.duty(), TrustedProviderRole::Coder);
        assert_eq!(frozen.profile(), ProviderExecutionProfile::HostCoding);
        assert!(frozen.review_window_id().is_none());
        // profile 只能来自受信参数——伪造字符串不能把 Coder 提升为窗口合同：
        let error = freeze_provider_execution(
            &coder,
            ProviderExecutionProfile::ReviewerVerify,
            None,
            "scope".to_string(),
            Some("win-forged".to_string()),
        )
        .expect_err("profile comes from trusted assembly, not forged strings");
        assert!(error.details.contains("profile_mismatch"));
    }

    /// Author 经 Orchestrator 承载时 audit 仍 Author（Step 1 红测③）：
    /// 职责与承载分开冻结，audit 事实面记录 duty=author、carrier=orchestrator。
    #[test]
    fn author_carried_by_orchestrator_audits_as_author() {
        let author = identity(TrustedProviderRole::Author, AdapterRole::Orchestrator);
        let frozen = freeze_provider_execution(
            &author,
            ProviderExecutionProfile::HostCoding,
            Some(AuthorToolPolicy::author_v1()),
            "scope_run_0001".to_string(),
            None,
        )
        .expect("author via orchestrator carrier must freeze");
        assert_eq!(frozen.duty(), TrustedProviderRole::Author);
        assert_eq!(frozen.carrier(), &AdapterRole::Orchestrator);
        assert!(frozen.author_policy_digest().is_some());
        let facts = frozen.audit_facts();
        assert_eq!(facts.get("duty").map(String::as_str), Some("author"));
        assert_eq!(
            facts.get("carrier").map(String::as_str),
            Some("orchestrator")
        );
    }

    /// 未知 deployment_context 拒绝（Step 1 红测④）：仅认精确 `host_local`；
    /// 别名/大小写/未知 context 显式拒绝，不静默 fallback。
    #[test]
    fn unknown_deployment_context_is_rejected() {
        assert_eq!(
            DeploymentContext::from_trusted_str("host_local").unwrap(),
            DeploymentContext::HostLocal
        );
        for unknown in [
            "",
            "host-local",
            "HostLocal",
            "k8s",
            "container",
            "dev-sandbox",
        ] {
            let error = match DeploymentContext::from_trusted_str(unknown) {
                Err(error) => error,
                Ok(_) => panic!("unknown context {unknown:?} must be rejected"),
            };
            assert!(
                error
                    .details
                    .contains("unknown deployment context rejected"),
                "unexpected details: {}",
                error.details
            );
        }
    }

    /// 合同摘要确定性且覆盖全部冻结事实：同事实同摘要；任一事实漂移 → 摘要变化。
    #[test]
    fn contract_digest_is_deterministic_and_covers_frozen_facts() {
        let coder = identity(TrustedProviderRole::Coder, AdapterRole::Executor);
        let baseline = freeze_provider_execution(
            &coder,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_a".to_string(),
            None,
        )
        .unwrap();
        let same = freeze_provider_execution(
            &identity(TrustedProviderRole::Coder, AdapterRole::Executor),
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_a".to_string(),
            None,
        )
        .unwrap();
        assert_eq!(baseline.contract_digest(), same.contract_digest());
        assert!(
            baseline
                .contract_digest()
                .starts_with(&format!("{PROVIDER_EXECUTION_CONTRACT_SCHEMA}:sha256:"))
        );

        let drifted_scope = freeze_provider_execution(
            &coder,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_b".to_string(),
            None,
        )
        .unwrap();
        assert_ne!(baseline.contract_digest(), drifted_scope.contract_digest());
    }

    /// 环境策略摘要随执行基线事实漂移：profile/职责/部署上下文任一变化 →
    /// 摘要变化（T3 白名单环境在此基础上续冻结键集合摘要）。
    #[test]
    fn env_policy_digest_tracks_execution_basis_facts() {
        let reviewer = identity(TrustedProviderRole::Reviewer, AdapterRole::Reviewer);
        let coder = identity(TrustedProviderRole::Coder, AdapterRole::Executor);
        let reviewer_frozen = freeze_provider_execution(
            &reviewer,
            ProviderExecutionProfile::ReviewerVerify,
            None,
            "scope_a".to_string(),
            Some("window_0001".to_string()),
        )
        .unwrap();
        let coder_frozen = freeze_provider_execution(
            &coder,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_a".to_string(),
            None,
        )
        .unwrap();
        assert_ne!(
            reviewer_frozen.env_policy_digest(),
            coder_frozen.env_policy_digest()
        );
    }

    /// 身份事实只来自 durable run/store（构造面核对）：run id、会话 id、承载
    /// 角色、能力画像引用逐字取自 durable 记录与受信装配入参。
    #[test]
    fn identity_facts_come_from_durable_run_store_only() {
        let record = durable_run_record("run_42", AdapterRole::Orchestrator);
        let identity = TrustedRunIdentity::from_durable_run(
            &record,
            "ws_session_42",
            TrustedProviderRole::Author,
        );
        assert_eq!(identity.provider_run_id(), "run_42");
        assert_eq!(identity.workspace_session_id(), "ws_session_42");
        assert_eq!(identity.duty(), TrustedProviderRole::Author);
        assert_eq!(identity.carrier(), &AdapterRole::Orchestrator);
        assert_eq!(
            identity.native_permission_profile_ref(),
            "cap_claude_code_1_4_0"
        );
    }
}
