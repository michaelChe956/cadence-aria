use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::structured_output::{
    StructuredOutputContract, StructuredOutputState, parse_structured_output,
    parse_structured_output_first_line_nonce,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

/// Test-only fake provider launch input wrapper.
///
/// `FakeStreamingProviderInput` 把 `StreamingProviderInput` 包裹起来,使其只能在
/// 测试中通过 `for_test` 构造,且必须声明 `ProviderType::Fake`。这是「Fake 经过
/// registry/编译期隔离」契约在 type 层的体现:逻辑代码库真实 provider 启动路径
/// (`LogicalCodebaseProviderGateway`/`ValidatedStreamingProviderInput`)不接受
/// 裸 `StreamingProviderInput`,而测试侧的 Fake 启动用此 wrapper 明确标记为隔离,
/// 而非通过 `if provider != Fake` 的运行时例外放行真实 provider。
///
/// 生产代码不构造本类型;`for_test` 在 `#[cfg(test)]` 下可用。
#[cfg(test)]
pub(crate) struct FakeStreamingProviderInput(StreamingProviderInput);

#[cfg(test)]
impl FakeStreamingProviderInput {
    /// 构造一个测试用 fake launch input。`provider_type` 必须 `ProviderType::Fake`,
    /// 否则 panic——这是「Fake 不可伪装成真实 provider」的编译/构造期断言。
    #[cfg(test)]
    pub fn for_test(provider_type: ProviderType, prompt: &str) -> Self {
        assert_eq!(
            provider_type,
            ProviderType::Fake,
            "FakeStreamingProviderInput::for_test 仅接受 ProviderType::Fake"
        );
        Self(StreamingProviderInput::fake_for_test(prompt))
    }

    /// 返回内部 `StreamingProviderInput` 的 provider type,供测试断言隔离。
    pub fn provider_type(&self) -> &ProviderType {
        &self.0.provider_type
    }
}

pub mod fake;

#[cfg(test)]
pub mod tests;

pub use fake::FakeStreamingProvider;

#[derive(Debug, Clone)]
pub enum StreamChunk {
    Text(String),
    Done { full_output: String },
    Error(String),
}

pub struct StreamingRunHandle {
    pub receiver: mpsc::Receiver<StreamChunk>,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPermissionMode {
    Auto,
    Supervised,
}

/// Tool-policy canonical 序列的规范版本前缀（REQ-ENV-09 digest 规范冻结：`"tp-v1"`）。
pub(crate) const TOOL_POLICY_CANONICAL_VERSION: &str = "tp-v1";

/// Codex 审批分类规则的版本后缀（digest 规范冻结：`"ap-v1"`；审批规则变化必须升级）。
pub(crate) const TOOL_POLICY_APPROVAL_POLICY_VERSION: &str = "ap-v1";

/// Tool-policy 语义意图。`DenyFileWriteBuiltins`：保护范围是 built-in 文件写
/// 工具（黑名单），不是全工具 allowlist；`BootstrapExecutorMarker`
/// （REQ-BOOT-04/D1，Task 1.2）不是 deny 投影，而是「有写权限的 Executor」
/// 的唯一显式自举通道——由 admission 从 durable Running operation 派生的
/// marker 装载，spawn 前经双向守卫复核，不进入策略会话 argv/审计机制。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPolicyIntent {
    DenyFileWriteBuiltins,
    /// LC 根 recipe 自举执行器标记（BOOT-04/D1）。
    BootstrapExecutorMarker(
        crate::product::logical_codebase::provider_admission_preflight::BootstrapExecutorMarker,
    ),
    /// 批2 Task 20：Reviewer 会话的窗口合同（ReviewerVerify profile 的 wire
    /// 投影）。`window_id` 为 durable 审查窗口引用（engine 开窗产物）——
    /// 受信装配在开窗之后铸造；深度有效性（Running/同 run/expired）由
    /// engine 材料采集前的同源 assert 承担，adapter/gateway 只复验非空
    /// 引用（fail-closed，不 fallback 补 deny）。物理面=全工具（完整
    /// 工具/terminal/MCP/skills），无 deny 片段。
    ReviewerVerify { window_id: String },
}

/// Provider 无关的语义工具策略。只表达语义意图，不携带 provider 物理片段；
/// 物理片段由各 provider translator（`translate_tool_policy`）按 provider 名冻结
///（`BootstrapExecutorMarker` 意图没有物理 deny 片段，translator fail-closed）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderToolPolicy {
    pub intent: ToolPolicyIntent,
}

impl ProviderToolPolicy {
    /// 唯一合法 deny 意图构造器：拒绝 built-in 文件写工具。策略角色统一用此
    /// 构造器；`BootstrapExecutorMarker` 意图只能经 `BootstrapExecutorMarker::new`
    ///（联合证明构造）装入。
    pub fn deny_file_write_builtins() -> Self {
        Self {
            intent: ToolPolicyIntent::DenyFileWriteBuiltins,
        }
    }

    /// 批2 Task 20：Reviewer 会话窗口合同的唯一铸造器（受信装配面：engine
    /// 开窗后 / gateway `prepare_trusted_*` 按 duty 铸造）。窗口引用非空性
    /// 由 [`validate_tool_policy_for_role`] 复验；kimi 侧由
    /// `ClientServicePolicy` 受信授予等价消费（grant 携带窗口引用）。
    pub fn reviewer_verify(window_id: impl Into<String>) -> Self {
        Self {
            intent: ToolPolicyIntent::ReviewerVerify {
                window_id: window_id.into(),
            },
        }
    }
}

/// kimi 例外映射(Task 7 spec「kimi 既有对齐维持」):kimi 隔离控制面
/// 不携带通用 tool policy,`DenyFileWriteBuiltins` 意图在 kimi 侧由既有
/// ClientServicePolicy 等价承担——映射为 `None`,避免 gateway 角色矩阵
/// 对策略角色派生的通用策略触发 kimi validated start 的
/// `provider_generic_tool_policy_forbidden` 拒绝。其余 intent(如
/// `BootstrapExecutorMarker`)原样透传,由消费侧守卫 fail-closed,不在此
/// 吞掉。调用方负责先判定 provider 是 KimiCode(本函数不重复判 provider)。
pub fn kimi_exempted_tool_policy(policy: Option<ProviderToolPolicy>) -> Option<ProviderToolPolicy> {
    match policy {
        Some(ProviderToolPolicy {
            intent: ToolPolicyIntent::DenyFileWriteBuiltins,
        }) => None,
        other => other,
    }
}

/// 双向 spawn 前守卫错误（REQ-ENV-09 Task 3.1 + Task 1.2 BOOT-04）。策略
/// 角色缺失策略、非策略角色误带策略与 BootstrapExecutor marker 结构不完整
/// 都在创建子进程之前拒绝，不 fallback 到无策略 argv。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPolicyGuardError {
    /// 策略角色（Orchestrator/WorkItemSplitter/Reviewer）缺失或非法策略。
    PolicyRequired { role: String },
    /// 非策略角色（Executor/Handoff，含 Coder 与聚合初始化 turns）误带策略。
    PolicyForbidden { role: String },
    /// BootstrapExecutor marker 结构不完整/非法（BOOT-04/D1，Task 1.2）：
    /// credential、写权限 action、canonical root 与 receipt context 四要素
    /// 缺一不可，退化 marker 在创建子进程之前拒绝。
    BootstrapMarkerInvalid { role: String, detail: String },
    /// LC validated 启动的投影角色与 input 角色不一致(Task 7):投影将冻结
    /// 的角色与真实 input 携带的角色分叉,属内部一致性缺陷,创建子进程之前
    /// fail-closed,不静默采纳任一侧。
    LaunchRoleMismatch { projection: String, input: String },
}

impl std::fmt::Display for ToolPolicyGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolPolicyGuardError::PolicyRequired { role } => write!(
                f,
                "tool policy guard: policy role {role} must carry DenyFileWriteBuiltins"
            ),
            ToolPolicyGuardError::PolicyForbidden { role } => write!(
                f,
                "tool policy guard: non-policy role {role} must not carry a tool policy"
            ),
            ToolPolicyGuardError::BootstrapMarkerInvalid { role, detail } => write!(
                f,
                "tool policy guard: bootstrap executor marker rejected for {role}: {detail}"
            ),
            ToolPolicyGuardError::LaunchRoleMismatch { projection, input } => write!(
                f,
                "tool policy guard: lc validated launch role mismatch: projection={projection} input={input}"
            ),
        }
    }
}

impl std::error::Error for ToolPolicyGuardError {}

/// AdapterRole 的真实序列化文本（durable 审计 provider_start.role 用；与
/// `UsageReportData::role_text` 的 usage 展示层归一严格分离，P1-8）。
pub(crate) fn adapter_role_text(role: &AdapterRole) -> &'static str {
    match role {
        AdapterRole::Orchestrator => "orchestrator",
        AdapterRole::Executor => "executor",
        AdapterRole::Reviewer => "reviewer",
        AdapterRole::WorkItemSplitter => "work_item_splitter",
        AdapterRole::Handoff => "handoff",
    }
}

/// Provider 版本探测统一错误（GC9，Task 3.3 正式落地；3.2 期间作为注入 supplier
/// 的错误语义先行）。策略会话遇任一错误直接 fail-closed，Coder/非策略路径零变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionProbeError {
    /// 命令失败或输出为空：版本不可得。
    Unavailable,
    /// 有界超时内未取得版本。
    Timeout,
    /// caller 主动取消(Task 3.2/B0a2):typed 失败,不缓存、不回退过期成功。
    Cancelled,
}

impl std::fmt::Display for VersionProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionProbeError::Unavailable => {
                write!(f, "provider version unavailable")
            }
            VersionProbeError::Timeout => write!(f, "provider version probe timed out"),
            VersionProbeError::Cancelled => write!(f, "provider version probe cancelled"),
        }
    }
}

impl std::error::Error for VersionProbeError {}

/// 策略会话 provider 版本 supplier（controller Ruling：3.2 以注入 supplier/fixture
/// 字符串实现 `provider_start.version`；3.3 落地真实 CLI `--version` 探测+进程内
/// 缓存后接线替换默认路径，测试 seam 保留）。
pub type ProviderVersionSupplier =
    std::sync::Arc<dyn Fn() -> Result<String, VersionProbeError> + Send + Sync>;

/// CLI `--version` 探测的进程内缓存（GC9：进程内缓存、有界超时）。按命令路径
/// 缓存成功结果；失败不缓存（下次启动重试）。supplier seam（测试）优先于缓存。
pub(crate) async fn cached_cli_version(
    command: &std::path::Path,
    probe: impl std::future::Future<Output = Result<String, VersionProbeError>>,
) -> Result<String, VersionProbeError> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, String>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(guard) = cache.lock()
        && let Some(version) = guard.get(command)
    {
        return Ok(version.clone());
    }
    let version = probe.await?;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(command.to_path_buf(), version.clone());
    }
    Ok(version)
}

// ===== Task 3.2(B0a2):LC 版本只读观测的 60 秒 TTL/single-flight 合同 =====
//
// owner 2026-10-11 裁决:允许有界只读 CLI `--version` 观测(不算创建会话
// 进程),并以 60 秒 TTL 缓存。本块是 LC 权威版本观测 helper,与上方 direct
// 路径的无过期 `cached_cli_version` 严格分离——direct 旧语义零变化。

/// LC 版本观测缓存 TTL(固定设计参数):从成功完成观测的单调时刻起算,
/// 有效条件 age<60s,边界 age>=60s 必再观测;非滑动(命中不续期)。
pub const LC_VERSION_PROBE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// LC 版本观测的缓存 key:按 provider + 实际配置 CLI 命令的 canonical
/// executable 路径/可执行身份 + 版本观测参数 + supplier 实例范围区分;
/// 路径/可执行身份变化直接换 key(不失效其它 key),测试 supplier 与真实
/// CLI 不串值。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LcVersionProbeKey {
    pub provider: crate::product::models::ProviderName,
    pub canonical_executable: PathBuf,
    pub executable_identity: String,
    pub argv: Vec<String>,
    pub supplier_scope: u64,
}

/// 一次成功的版本观测(缓存值仅版本字符串+观测时刻,不缓存 trust/policy/
/// projection/admission 或 fresh 授权)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LcVersionObservation {
    exact_version: String,
    observed_at: std::time::Instant,
}

impl LcVersionObservation {
    // B0a2 合同先落:生产消费方为 Task 3.3–3.6 四家 adapter 的 prepare
    // 接线,测试面已消费(kimi session.rs 同款 cfg_attr 先例)。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn new(exact_version: String, observed_at: std::time::Instant) -> Self {
        Self {
            exact_version,
            observed_at,
        }
    }

    pub fn exact_version(&self) -> &str {
        &self.exact_version
    }

    pub fn observed_at(&self) -> std::time::Instant {
        self.observed_at
    }
}

/// 版本观测 async factory:single-flight 下仅 winner 调用,不预造 future、
/// 不 spawn;返回有界观测(成功为版本字符串,失败为 typed 错误)。
pub type LcVersionProbe = std::sync::Arc<
    dyn Fn(
            LcVersionProbeKey,
        )
            -> std::pin::Pin<Box<dyn Future<Output = Result<String, VersionProbeError>> + Send>>
        + Send
        + Sync,
>;

/// LC 版本观测缓存实例(进程内、非 durable;进程/实例重建即失效,不跨
/// 实例复用旧值)。同 key 并发 miss/到期 single-flight——只有 winner 执行
/// 一次有界观测,其它 caller 等同一结果;不同 key 互不串行。观测失败/
/// 超时/取消不缓存、不回退过期成功;成功值在 age<`LC_VERSION_PROBE_TTL`
/// 内命中且不滑动续期。时钟默认单调 `Instant::now`,`with_clock` 为
/// crate-private 测试 seam(注入手动时钟,禁止 sleep 概率测试)。
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone)]
pub struct LcVersionCache {
    inner: std::sync::Arc<LcVersionCacheInner>,
}

// 同上:B0a2 合同先落,生产消费方为 3.3–3.6 的 adapter 接线;测试面已消费。
#[cfg_attr(not(test), allow(dead_code))]
struct LcVersionCacheInner {
    slots: std::sync::Mutex<std::collections::HashMap<LcVersionProbeKey, LcVersionCacheSlot>>,
    clock: std::sync::Arc<dyn Fn() -> std::time::Instant + Send + Sync>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone)]
enum LcVersionCacheSlot {
    Fresh(LcVersionObservation),
    InFlight(tokio::sync::watch::Sender<Option<Result<LcVersionObservation, VersionProbeError>>>),
}

#[cfg_attr(not(test), allow(dead_code))]
enum LcVersionCacheClaim {
    Fresh(LcVersionObservation),
    Waiting(tokio::sync::watch::Receiver<Option<Result<LcVersionObservation, VersionProbeError>>>),
    Won(tokio::sync::watch::Sender<Option<Result<LcVersionObservation, VersionProbeError>>>),
}

#[cfg_attr(not(test), allow(dead_code))]
impl LcVersionCache {
    pub fn new() -> Self {
        Self::with_clock(std::sync::Arc::new(std::time::Instant::now))
    }

    /// crate-private 测试 seam:注入单调时钟。
    pub(crate) fn with_clock(
        clock: std::sync::Arc<dyn Fn() -> std::time::Instant + Send + Sync>,
    ) -> Self {
        Self {
            inner: std::sync::Arc::new(LcVersionCacheInner {
                slots: std::sync::Mutex::new(std::collections::HashMap::new()),
                clock,
            }),
        }
    }

    /// TTL 合同入口:命中(age<60s,非滑动)直接返回;miss/到期由 winner
    /// 执行唯一一次有界观测。caller 取消返回 typed `Cancelled`(probe
    /// future 随之丢弃,迟到结果不复活旧 run、不入缓存)。
    pub(crate) async fn cached_lc_cli_version(
        &self,
        key: LcVersionProbeKey,
        probe: LcVersionProbe,
        cancel: CancellationToken,
    ) -> Result<LcVersionObservation, VersionProbeError> {
        match self.inner.claim(&key) {
            LcVersionCacheClaim::Fresh(observation) => Ok(observation),
            LcVersionCacheClaim::Waiting(mut receiver) => {
                // loser:与 winner 等同一结果;自身取消立即 typed 失败。
                let settled = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => Err(VersionProbeError::Cancelled),
                    settled = receiver.wait_for(|value| value.is_some()) => match settled {
                        Ok(value) => match &*value {
                            Some(result) => result.clone(),
                            None => Err(VersionProbeError::Unavailable),
                        },
                        Err(_) => Err(VersionProbeError::Unavailable),
                    },
                };
                settled
            }
            LcVersionCacheClaim::Won(sender) => {
                // panic 卸载保护(fix-loop R1):guard 在本臂任何退出路径
                // (含 unwind)丢弃;成功路径先写 Fresh,错误路径的显式逐出
                // 与 guard 幂等共存。
                let _guard = LcVersionInFlightDropGuard::new(&self.inner, &key, &sender);
                // winner:唯一一次调用 probe(不预造 future、不 spawn)。
                let probe_future = probe(key.clone());
                let observed = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => Err(VersionProbeError::Cancelled),
                    version = probe_future => version,
                };
                let settled = observed.map(|exact_version| {
                    LcVersionObservation::new(exact_version, (self.inner.clock)())
                });
                // single-flight:waiters 等同一结果(成功或 typed 失败)。
                sender.send_replace(Some(settled.clone()));
                match settled {
                    Ok(observation) => {
                        // 仅成功缓存;命中不滑动续期(observed_at 不变)。
                        self.inner
                            .slots
                            .lock()
                            .expect("lc version cache lock")
                            .insert(key, LcVersionCacheSlot::Fresh(observation.clone()));
                        Ok(observation)
                    }
                    Err(error) => {
                        // 失败/取消不缓存:逐出本 in-flight 槽(仅当仍是
                        // 自己的槽),后续 caller 重新观测,绝不回退旧值。
                        let mut slots = self.inner.slots.lock().expect("lc version cache lock");
                        if let Some(LcVersionCacheSlot::InFlight(current)) = slots.get(&key)
                            && current.same_channel(&sender)
                        {
                            slots.remove(&key);
                        }
                        Err(error)
                    }
                }
            }
        }
    }
}

/// 审查 fix-loop R1(P2):winner panic 卸载保护。winner 执行途中 panic
/// (probe factory/future panic、锁中毒级联)时,unwind 只丢弃 winner 侧的
/// sender 克隆——HashMap 槽内仍有另一克隆,watch 通道不关闭,同 key 后续
/// caller 会走 `Waiting` 永久阻塞。本 guard 在 drop(含 unwind 路径)时以
/// 与既有错误结算路径相同的语义逐出仍属自己的 InFlight 槽;成功路径在
/// guard 丢弃前已写入 `Fresh`,不受影响。锁中毒时也强制逐出
/// (`into_inner`),不把槽留给重启前的所有 caller。
struct LcVersionInFlightDropGuard {
    inner: std::sync::Arc<LcVersionCacheInner>,
    key: LcVersionProbeKey,
    sender: tokio::sync::watch::Sender<Option<Result<LcVersionObservation, VersionProbeError>>>,
}

impl LcVersionInFlightDropGuard {
    fn new(
        inner: &std::sync::Arc<LcVersionCacheInner>,
        key: &LcVersionProbeKey,
        sender: &tokio::sync::watch::Sender<
            Option<Result<LcVersionObservation, VersionProbeError>>,
        >,
    ) -> Self {
        Self {
            inner: inner.clone(),
            key: key.clone(),
            sender: sender.clone(),
        }
    }
}

impl Drop for LcVersionInFlightDropGuard {
    fn drop(&mut self) {
        let mut slots = match self.inner.slots.lock() {
            Ok(slots) => slots,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(LcVersionCacheSlot::InFlight(current)) = slots.get(&self.key)
            && current.same_channel(&self.sender)
        {
            slots.remove(&self.key);
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl Default for LcVersionCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl LcVersionCacheInner {
    fn expired(&self, observation: &LcVersionObservation) -> bool {
        (self.clock)().duration_since(observation.observed_at()) >= LC_VERSION_PROBE_TTL
    }

    /// 短锁内裁决:TTL 内命中 → Fresh;已有 winner 在观测 → Waiting;
    /// 否则逐出过期条目并注册自己为 winner。
    fn claim(&self, key: &LcVersionProbeKey) -> LcVersionCacheClaim {
        let mut slots = self.slots.lock().expect("lc version cache lock");
        if let Some(slot) = slots.get(key) {
            match slot {
                LcVersionCacheSlot::Fresh(observation) if !self.expired(observation) => {
                    return LcVersionCacheClaim::Fresh(observation.clone());
                }
                LcVersionCacheSlot::InFlight(sender) => {
                    return LcVersionCacheClaim::Waiting(sender.subscribe());
                }
                _ => {}
            }
        }
        slots.remove(key);
        let (sender, _receiver) = tokio::sync::watch::channel(None);
        slots.insert(key.clone(), LcVersionCacheSlot::InFlight(sender.clone()));
        LcVersionCacheClaim::Won(sender)
    }
}

/// 批2 Task 11（B6 公共角色矩阵）六职责→wire 工具策略权威矩阵：
/// - **Author** → `Some(DenyFileWriteBuiltins)`：岗位纪律（AuthorToolPolicy
///   「不装配编辑代码文件的内建工具」语义）的唯一 wire 物理投影，由受信
///   装配铸造（engine builder / gateway `prepare_trusted_*` / workspace
///   author 面），adapter 映射到 Claude `Edit/Write/NotebookEdit`、Pi
///   `edit/write`；
/// - **Coder / Orchestrator / Splitter / Handoff** → `None`：全工具；
/// - **Reviewer** → `None`：只携 ReviewerVerify+窗口合同（真窗归 T20），
///   wire 面不回退旧 deny fallback。
pub fn tool_policy_for_duty(
    duty: &crate::cross_cutting::provider_execution::TrustedProviderRole,
) -> Option<ProviderToolPolicy> {
    match duty {
        crate::cross_cutting::provider_execution::TrustedProviderRole::Author => {
            Some(ProviderToolPolicy::deny_file_write_builtins())
        }
        crate::cross_cutting::provider_execution::TrustedProviderRole::Orchestrator
        | crate::cross_cutting::provider_execution::TrustedProviderRole::Splitter
        | crate::cross_cutting::provider_execution::TrustedProviderRole::Handoff
        | crate::cross_cutting::provider_execution::TrustedProviderRole::Coder
        | crate::cross_cutting::provider_execution::TrustedProviderRole::Reviewer => None,
    }
}

/// 双向角色×策略守卫（批2 Task 20 收口）：按 AdapterRole 一概套 deny 的
/// 旧矩阵退役——`DenyFileWriteBuiltins` 是 Author 岗位纪律的物理投影，
/// 只在 Author 可承载的角色（Orchestrator=story/design/revision、
/// WorkItemSplitter=plan authoring）合法。**Reviewer 会话（四家启动检查）
/// 必须携带 `ReviewerVerify{window_id}` 窗口合同**：无合同/空窗口引用/
/// deny 回退一律 spawn 前拒绝（0 会话，不以 Pi 特例或 Author deny 替补）；
/// 深度窗口有效性（Running/同 run/expired）由 engine 材料采集前的同源
/// assert 承担。Coder/规划族 wire 面全开（None）；Executor 的
/// `BootstrapExecutorMarker` 自举通道（REQ-BOOT-04/D1）保持不变。非法
/// 组合在 provider 创建子进程之前拒绝（四 adapter `start` 首步调用）。
pub fn validate_tool_policy_for_role(
    role: &AdapterRole,
    policy: Option<&ProviderToolPolicy>,
) -> Result<(), ToolPolicyGuardError> {
    match policy.map(|policy| &policy.intent) {
        // 批2 Task 20：Reviewer 无窗口合同——0 会话（fail-closed）。
        None if role == &AdapterRole::Reviewer => Err(ToolPolicyGuardError::PolicyRequired {
            role: adapter_role_text(role).to_string(),
        }),
        None => Ok(()),
        // 批2 Task 20：ReviewerVerify 窗口合同专属 Reviewer；窗口引用
        // 必须非空（空引用=装配缺陷，拒绝）。
        Some(ToolPolicyIntent::ReviewerVerify { window_id }) => {
            if role != &AdapterRole::Reviewer || window_id.trim().is_empty() {
                return Err(ToolPolicyGuardError::PolicyForbidden {
                    role: adapter_role_text(role).to_string(),
                });
            }
            Ok(())
        }
        // Author 岗位投影：仅 Author 可承载的角色合法。
        Some(ToolPolicyIntent::DenyFileWriteBuiltins)
            if matches!(
                role,
                AdapterRole::Orchestrator | AdapterRole::WorkItemSplitter
            ) =>
        {
            Ok(())
        }
        // BOOT-04/D1：Executor 的唯一自举通道——marker 四要素（credential、
        // 写权限 action、canonical root、receipt context）联合完整才放行。
        Some(ToolPolicyIntent::BootstrapExecutorMarker(marker))
            if role == &AdapterRole::Executor =>
        {
            marker.incomplete_reason().map_or(Ok(()), |detail| {
                Err(ToolPolicyGuardError::BootstrapMarkerInvalid {
                    role: adapter_role_text(role).to_string(),
                    detail: detail.to_string(),
                })
            })
        }
        // BOOT-04/T10 契约（修复轮 F3）：marker 不是策略——原策略角色
        // （Orchestrator/WorkItemSplitter/Reviewer）携带 marker 在 spawn 前
        // 以 `PolicyRequired` 拒绝（Handoff 保持 `PolicyForbidden`），与
        // T10 bootstrap 负测/预检口径一致。
        Some(ToolPolicyIntent::BootstrapExecutorMarker(_))
            if matches!(
                role,
                AdapterRole::Orchestrator | AdapterRole::WorkItemSplitter | AdapterRole::Reviewer
            ) =>
        {
            Err(ToolPolicyGuardError::PolicyRequired {
                role: adapter_role_text(role).to_string(),
            })
        }
        // 其余组合（非 Author 承载角色带 deny、Handoff/Reviewer 带旧 deny、
        // 未来未知 intent）一律 fail-closed。
        Some(_) => Err(ToolPolicyGuardError::PolicyForbidden {
            role: adapter_role_text(role).to_string(),
        }),
    }
}

/// 批2 Task 20：受信装配的 duty→wire 投影（窗口感知版）。Reviewer duty
/// 必须携带有效窗口引用铸造 `ReviewerVerify` 合同（缺窗 fail-closed，
/// 返回 `Err`）；Author 保持 deny 投影（不依赖窗口）；其余职责全开
/// `None`。[`tool_policy_for_duty`] 为无窗上下文的旧口径（Reviewer 返回
/// `None` 仅用于「尚未接线的中间态」断言），生产装配一律走本函数。
pub fn tool_policy_for_duty_in_window(
    duty: &crate::cross_cutting::provider_execution::TrustedProviderRole,
    review_window_id: Option<&str>,
) -> Result<Option<ProviderToolPolicy>, ToolPolicyGuardError> {
    match duty {
        crate::cross_cutting::provider_execution::TrustedProviderRole::Author => {
            Ok(Some(ProviderToolPolicy::deny_file_write_builtins()))
        }
        crate::cross_cutting::provider_execution::TrustedProviderRole::Reviewer => {
            let window_id = review_window_id
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| ToolPolicyGuardError::PolicyRequired {
                    role: adapter_role_text(&AdapterRole::Reviewer).to_string(),
                })?;
            Ok(Some(ProviderToolPolicy::reviewer_verify(window_id)))
        }
        crate::cross_cutting::provider_execution::TrustedProviderRole::Orchestrator
        | crate::cross_cutting::provider_execution::TrustedProviderRole::Splitter
        | crate::cross_cutting::provider_execution::TrustedProviderRole::Handoff
        | crate::cross_cutting::provider_execution::TrustedProviderRole::Coder => Ok(None),
    }
}

/// LC validated 启动的共用 run-context guard(Task 7 Step 3):验证
/// projection role 与 input role 一致后调用双向 tool-policy guard。三家
/// (Claude/Codex/Pi)的 LC `start_validated` 首步调用,整个检查块早于
/// 版本探测/session child/extension——任何不一致或非法组合都在创建子
/// 进程之前拒绝。
///
/// - `projection_role`:即将冻结进 LC 投影(`ProviderProjectionInput`)的
///   角色,与 `input_role` 分叉即内部一致性缺陷(`LaunchRoleMismatch`);
/// - `root_recipe_phase`:validated policy 是否 RootRecipe 相位(durable
///   Running operation 派生凭据)。`BootstrapExecutorMarker` 是唯一只对
///   root recipe 自举通道放行的写权限载体——普通 Coder(Normal 相位)借
///   marker 获取 root 写权在 guard 层即拒绝,不进入投影/子进程;
/// - 其余组合沿 [`validate_tool_policy_for_role`] 的双向语义。
pub fn guard_lc_validated_launch(
    root_recipe_phase: bool,
    projection_role: &AdapterRole,
    input_role: &AdapterRole,
    policy: Option<&ProviderToolPolicy>,
) -> Result<(), ToolPolicyGuardError> {
    if projection_role != input_role {
        return Err(ToolPolicyGuardError::LaunchRoleMismatch {
            projection: adapter_role_text(projection_role).to_string(),
            input: adapter_role_text(input_role).to_string(),
        });
    }
    if !root_recipe_phase
        && policy.is_some_and(|policy| {
            matches!(policy.intent, ToolPolicyIntent::BootstrapExecutorMarker(_))
        })
    {
        return Err(ToolPolicyGuardError::BootstrapMarkerInvalid {
            role: adapter_role_text(input_role).to_string(),
            detail: "bootstrap executor marker requires a root recipe durable operation context; ordinary coder launches must not carry it"
                .to_string(),
        });
    }
    validate_tool_policy_for_role(input_role, policy)
}

/// Canonical tool policy 投影：provider 名 + 按 provider 冻结的 canonical token
/// 序列 + 审批规则版本 + 依规范输入实算的 sha256 digest。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalToolPolicy {
    pub provider: String,
    pub tokens: Vec<String>,
    pub approval_policy_version: String,
    pub digest: String,
}

/// Codex 审批分类（GC6 冻结）：`item/commandExecution/requestApproval` 是
/// commandExecution；`item/fileChange/requestApproval` 是 fileChange；
/// `mcpServer/elicitation/request` 仅在 `_meta.codex_approval_kind="mcp_tool_call"`
/// 时是 MCP，否则为未知 elicitation；未知 `item/*/requestApproval` 为未知 item。
/// 自然语言 `reason` 不得作为分类依据；未知形态保留原 method。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexApprovalCategory {
    McpToolCall,
    CommandExecution,
    FileChange,
    Unknown { method: String },
}

impl CodexApprovalCategory {
    /// 审计载荷中的类别文本（结构化事件用）。
    pub fn audit_text(&self) -> &'static str {
        match self {
            CodexApprovalCategory::McpToolCall => "mcp_tool_call",
            CodexApprovalCategory::CommandExecution => "command_execution",
            CodexApprovalCategory::FileChange => "file_change",
            CodexApprovalCategory::Unknown { .. } => "unknown",
        }
    }
}

/// Codex 审批应答协议结果：`Accept`/`Decline` 序列化为 `{"decision":...}`；
/// `ElicitationError` 序列化为 JSON-RPC error（未知 elicitation = `-32601` + data）。
#[derive(Debug, Clone, PartialEq)]
pub enum CodexApprovalResponse {
    Accept,
    Decline,
    ElicitationError { code: i32, data: serde_json::Value },
}

impl CodexApprovalResponse {
    /// 审计载荷中的决策文本（结构化事件用）。
    pub fn audit_text(&self) -> &'static str {
        match self {
            CodexApprovalResponse::Accept => "accept",
            CodexApprovalResponse::Decline => "decline",
            CodexApprovalResponse::ElicitationError { .. } => "protocol_error",
        }
    }
}

/// 策略会话审批决策的结构化事件（GC11 canonical `approval_decision` 的内存形态；
/// durable 落盘接线在后续 task 的 sink 注入，本类型即类型化决策出口）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexApprovalDecisionEvent {
    pub request_id: String,
    pub category: &'static str,
    pub decision: &'static str,
}

/// 未知审批形态的结构化告警事件（GC11 canonical `protocol_warning` 内存形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexProtocolWarningEvent {
    pub reason_code: String,
    pub method: String,
    pub occurrence: u32,
}

/// 未知审批风暴终止的结构化事件（GC11 canonical `session_terminated` 内存形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexSessionTerminatedEvent {
    pub reason_code: String,
}

/// resume 记录缺失的带内 superseded 标记（GC9）：reason_code 冻结
/// `superseded_policy_record_missing`。缺失路径无旧文件可写（🔴 不得伪造无
/// provider_start 首行的 durable 文件，与 drift 有旧文件可写不同），故仅经
/// 事件通道可观测；三 adapter 在清除 resume id、以全新会话启动时发出。
pub fn superseded_policy_record_missing_warning() -> ProviderEvent {
    ProviderEvent::ToolPolicyWarning(CodexProtocolWarningEvent {
        reason_code: crate::cross_cutting::tool_policy_audit::SUPERSEDED_POLICY_RECORD_MISSING
            .to_string(),
        method: "resume_provider_session_id".to_string(),
        occurrence: 1,
    })
}

/// Tool-policy 翻译/canonical 化错误。未知 provider 名 fail-closed（kimi 不接策略）。
/// `BootstrapExecutorMarker` 意图没有物理 deny 片段，不投影为策略会话。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPolicyError {
    UnsupportedProvider(String),
    /// bootstrap marker 不翻译/canonical 化（Task 1.2，BOOT-04）：自举执行器
    /// 带写权限，不是 deny 策略会话；进入本分支即调用方接线错误，fail-closed。
    BootstrapMarkerNotTranslatable,
}

impl std::fmt::Display for ToolPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolPolicyError::UnsupportedProvider(provider) => {
                write!(f, "unsupported tool-policy provider: {provider}")
            }
            ToolPolicyError::BootstrapMarkerNotTranslatable => {
                write!(
                    f,
                    "bootstrap executor marker has no tool policy translation"
                )
            }
        }
    }
}

impl std::error::Error for ToolPolicyError {}

/// 依 digest 规范实算：`"tp-v1" + \x1f + provider + \x1f + tokens.join(\x1f) + \x1f + "ap-v1"`
/// 的 sha256 hex。argv flag/value 按出现顺序原样、大小写保留。
pub(crate) fn tool_policy_digest(
    provider: &str,
    tokens: &[String],
    approval_policy_version: &str,
) -> String {
    use sha2::{Digest, Sha256};

    let canonical_input = [
        TOOL_POLICY_CANONICAL_VERSION,
        provider,
        &tokens.join("\x1f"),
        approval_policy_version,
    ]
    .join("\x1f");
    let mut hasher = Sha256::new();
    hasher.update(canonical_input.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 各 provider 冻结的 DenyFileWriteBuiltins canonical token 序列；
/// 未知 provider 返回 `UnsupportedProvider`。
fn deny_file_write_builtins_tokens(provider: &str) -> Result<Vec<String>, ToolPolicyError> {
    match provider {
        crate::cross_cutting::pi_provider::TOOL_POLICY_PROVIDER_NAME => {
            Ok(crate::cross_cutting::pi_provider::deny_file_write_builtins_tokens())
        }
        crate::cross_cutting::claude_code_provider::TOOL_POLICY_PROVIDER_NAME => {
            Ok(crate::cross_cutting::claude_code_provider::deny_file_write_builtins_tokens())
        }
        crate::cross_cutting::codex_provider::session::TOOL_POLICY_PROVIDER_NAME => {
            Ok(crate::cross_cutting::codex_provider::session::deny_file_write_builtins_tokens())
        }
        other => Err(ToolPolicyError::UnsupportedProvider(other.to_string())),
    }
}

/// 把语义策略投影为 canonical 形态（provider + tokens + 审批规则版本 + digest）。
pub fn canonical_tool_policy(
    provider: &str,
    policy: &ProviderToolPolicy,
) -> Result<CanonicalToolPolicy, ToolPolicyError> {
    match &policy.intent {
        ToolPolicyIntent::DenyFileWriteBuiltins => {
            let tokens = deny_file_write_builtins_tokens(provider)?;
            Ok(CanonicalToolPolicy {
                provider: provider.to_string(),
                digest: tool_policy_digest(provider, &tokens, TOOL_POLICY_APPROVAL_POLICY_VERSION),
                tokens,
                approval_policy_version: TOOL_POLICY_APPROVAL_POLICY_VERSION.to_string(),
            })
        }
        // 批2 Task 20：ReviewerVerify 窗口合同——物理面=全工具（无 deny
        // 片段）；canonical digest 以窗口引用为域（同窗同 digest、换窗必
        // 失配），供 durable provider_start 与 resume 三元组比对消费。
        ToolPolicyIntent::ReviewerVerify { window_id } => {
            let tokens = Vec::new();
            Ok(CanonicalToolPolicy {
                provider: provider.to_string(),
                digest: tool_policy_digest(
                    provider,
                    &[format!("reviewer-verify-window:{window_id}")],
                    TOOL_POLICY_APPROVAL_POLICY_VERSION,
                ),
                tokens,
                approval_policy_version: TOOL_POLICY_APPROVAL_POLICY_VERSION.to_string(),
            })
        }
        // marker 不是 deny 策略会话：不产出 canonical digest/tokens（自举
        // 执行器的 durable 审计由 root recipe receipt 承担，Task 1.5）。
        ToolPolicyIntent::BootstrapExecutorMarker(_) => {
            Err(ToolPolicyError::BootstrapMarkerNotTranslatable)
        }
    }
}

/// 把语义策略翻译为 provider 物理片段（argv flag/value 或 codex 启动参数原文，
/// 按出现顺序原样、大小写保留）。
pub fn translate_tool_policy(
    provider: &str,
    policy: &ProviderToolPolicy,
) -> Result<Vec<String>, ToolPolicyError> {
    match &policy.intent {
        ToolPolicyIntent::DenyFileWriteBuiltins => deny_file_write_builtins_tokens(provider),
        // 批2 Task 20：ReviewerVerify=全工具（无 deny argv 片段）；窗口纪律
        // 由平台 durable 窗口与对账承担，不物理限制工具面。
        ToolPolicyIntent::ReviewerVerify { .. } => Ok(Vec::new()),
        // marker 意图没有物理 deny 片段：自举执行器保持完整工具面（写权限
        // 来自 credential 的 admission 判定，而非 argv 黑名单）。
        ToolPolicyIntent::BootstrapExecutorMarker(_) => {
            Err(ToolPolicyError::BootstrapMarkerNotTranslatable)
        }
    }
}


/// Author 会话基线树锚点（per-issue-base-branch，REQ-PIB-02）。两级语义
///（防误读，design D3）：
/// - host-served 通道（kimi fs 桥）：通道层**硬边界**——fs 读按
///   `git show refs/heads/<branch>:<path>` 树内容供给、terminal 一律拒绝，
///   工作区与兄弟 worktree 路径不可达；树读取不 checkout、不触工作区。
/// - provider 原生通道（claude/codex/pi）：**仅获 prompt 层教学（软约束）**，
///   本字段的存在不构成其访问受限的证据——其物理可达集不受限（用户
///   2026-09-25 裁决接受残余风险；plan 期路径形态由 REQ-PIB-03 核对
///   fail-closed 兜底，沙箱形态见 design Deferred）。
///
/// `repo_path` 为仓库主检出（refs/heads 可解析）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineTreeRef {
    pub repo_path: PathBuf,
    pub branch: String,
}

#[derive(Clone)]
pub struct StreamingProviderInput {
    pub provider_type: ProviderType,
    pub role: AdapterRole,
    pub prompt: String,
    pub working_dir: PathBuf,
    /// 独立会话 cwd（Task 2.5 cwd/target 分离合同）：LC 入口注入 canonical
    /// LC root（`Some`）；`None` 时回填既有 `working_dir`（存量构造零行为
    /// 变化）。`working_dir` 继续表达 provider 实际 spawn cwd（legacy/单仓
    /// 语义不变）；gateway spawn 前复验按 `effective_working_directory` 取值。
    pub working_directory: Option<PathBuf>,
    /// 产品/工作区 session ID，用于日志追踪和关联，不用于 provider 续接。
    pub workspace_session_id: Option<String>,
    /// Provider 原生 session ID，用于续接 Claude Code / Codex 会话。
    pub resume_provider_session_id: Option<String>,
    pub permission_mode: ProviderPermissionMode,
    /// 语义工具策略（REQ-ENV-09 + BOOT-04）。策略角色（Orchestrator/
    /// WorkItemSplitter/Reviewer 的作者/评审链）携带 `DenyFileWriteBuiltins`；
    /// 普通 Executor/Coder 与非策略路径必须传 `None`（kimi 零改动，不读此
    /// 字段）；唯一例外是 LC 根 recipe 的 BootstrapExecutor marker（Task 1.2）：
    /// Executor 携带结构完整的 marker 作为「有写权限的 Executor」显式自举
    /// 通道，spawn 前由双向守卫复核。
    pub tool_policy: Option<ProviderToolPolicy>,
    /// durable tool-policy 审计 sink（REQ-ENV-09 Task 3.2/D7）。策略会话由 engine
    /// 按 provider run 绑定 `(workspace_session_id, role_run_seq)` 后注入；缺失时
    /// 策略会话启动 fail-closed。非策略路径恒为 `None`。
    pub audit_sink:
        Option<std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>>,
    pub structured_output_contract: Option<StructuredOutputContract>,
    pub env_vars: BTreeMap<String, String>,
    pub timeout_secs: u64,
    /// 基线树锚点（REQ-PIB-02）：story/design/plan author 会话由 engine 解析
    /// issue 基线后注入；`None` = 非基线会话（coder/评审/聚合 Non-Goal 面与
    /// 全部既有路径），行为不变。
    pub baseline_tree: Option<BaselineTreeRef>,
}

impl std::fmt::Debug for StreamingProviderInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingProviderInput")
            .field("provider_type", &self.provider_type)
            .field("role", &self.role)
            .field("prompt", &self.prompt)
            .field("working_dir", &self.working_dir)
            .field("working_directory", &self.working_directory)
            .field("workspace_session_id", &self.workspace_session_id)
            .field(
                "resume_provider_session_id",
                &self.resume_provider_session_id,
            )
            .field("permission_mode", &self.permission_mode)
            .field("tool_policy", &self.tool_policy)
            .field(
                "audit_sink",
                &self.audit_sink.as_ref().map(|_| "<tool-policy-audit-sink>"),
            )
            .field(
                "structured_output_contract",
                &self.structured_output_contract,
            )
            .field("env_vars", &self.env_vars)
            .field("timeout_secs", &self.timeout_secs)
            .field("baseline_tree", &self.baseline_tree)
            .finish()
    }
}

impl StreamingProviderInput {
    /// 复验用 effective cwd（Task 2.5）：优先独立 `working_directory`（canonical
    /// LC root），否则回填既有 `working_dir`（legacy/单仓路径，零行为变化）。
    pub fn effective_working_directory(&self) -> &Path {
        self.working_directory
            .as_deref()
            .unwrap_or(&self.working_dir)
    }

    /// 测试专用构造函数:产出一个 `ProviderType::Fake` 的隔离 input。生产代码不得
    /// 调用本方法;它仅供 `FakeStreamingProviderInput::for_test` 复用,使 Fake
    /// 启动路径与真实 provider 启动路径在构造上完全分离。
    #[cfg(test)]
    fn fake_for_test(prompt: &str) -> Self {
        Self {
            provider_type: ProviderType::Fake,
            role: AdapterRole::Orchestrator,
            prompt: prompt.to_string(),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            working_directory: None,
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            tool_policy: None,
            audit_sink: None,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: 60,
            baseline_tree: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequestData {
    pub id: String,
    pub tool_name: String,
    pub description: String,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChoiceOptionData {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChoiceQuestionData {
    pub id: String,
    pub prompt: String,
    pub options: Vec<ChoiceOptionData>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChoiceAnswerData {
    pub question_id: String,
    pub selected_option_ids: Vec<String>,
    pub free_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceRequestData {
    pub id: String,
    pub prompt: String,
    pub options: Vec<ChoiceOptionData>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
    pub questions: Vec<ChoiceQuestionData>,
    pub source: ChoiceRequestSource,
}

impl ChoiceRequestData {
    pub fn effective_questions(&self) -> Vec<ChoiceQuestionData> {
        if self.questions.is_empty() {
            return vec![ChoiceQuestionData {
                id: "default".to_string(),
                prompt: self.prompt.clone(),
                options: self.options.clone(),
                allow_multiple: self.allow_multiple,
                allow_free_text: self.allow_free_text,
            }];
        }
        self.questions.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceRequestSource {
    AskUserQuestion,
    RequestUserInput,
    TextFallback,
    ProviderChoice,
}

impl ChoiceRequestSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AskUserQuestion => "ask_user_question",
            Self::RequestUserInput => "request_user_input",
            Self::TextFallback => "text_fallback",
            Self::ProviderChoice => "provider_choice",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    Starting,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderExecutionEventKind {
    Provider,
    Turn,
    Command,
    Output,
    Artifact,
    /// Provider 上报的 token 用量事件（best-effort，见 `UsageReportData`）。
    Usage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderExecutionEventStatus {
    Started,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderExecutionEvent {
    pub event_id: String,
    pub kind: ProviderExecutionEventKind,
    pub status: ProviderExecutionEventStatus,
    pub title: String,
    pub detail: Option<String>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub output: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolCall {
    pub id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderToolResult {
    pub tool_use_id: String,
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCompletion {
    pub full_output: String,
    pub readable_output: String,
    pub structured_output: StructuredOutputState,
    pub provider_session_id: Option<String>,
}

impl ProviderCompletion {
    pub fn plain(full_output: impl Into<String>, provider_session_id: Option<String>) -> Self {
        let full_output = full_output.into();
        Self {
            readable_output: full_output.clone(),
            full_output,
            structured_output: StructuredOutputState::NotRequested,
            provider_session_id,
        }
    }

    pub fn from_output(
        full_output: String,
        contract: Option<&StructuredOutputContract>,
        provider_session_id: Option<String>,
    ) -> Self {
        let Some(contract) = contract else {
            return Self::plain(full_output, provider_session_id);
        };
        let parsed = if contract.schema_name == "single_candidate_work_item_plan_review" {
            parse_structured_output_first_line_nonce(&full_output, contract)
        } else {
            parse_structured_output(&full_output, contract)
        };
        if parsed.preamble_trimmed {
            tracing::info!(
                diagnostic = "preamble_trimmed",
                schema = %contract.schema_name,
                "discarded preamble before single-candidate reviewer nonce sentinel"
            );
        }
        Self {
            full_output,
            readable_output: parsed.readable_output,
            structured_output: parsed.state,
            provider_session_id,
        }
    }
}

/// 批2 Task 7（Bug45）：会话内真实终端活动快照——「活动事件」的载体。
///
/// kimi 会话在启动时经事件通道投递一次携带本快照的
/// `ProviderEvent::TerminalActivity`；消费方（coding/workspace 两层 idle
/// 看门狗）持有该 handle，在触发点读真实活动 registry：仍有在途
/// terminal ⇒ 暂停「无工作 stall」判定（不延长总 deadline）。状态与
/// terminal manager 的进程生命周期同源（同一 terminal_id 的
/// Started→Running→Exited/Killed/Released），不是伪造心跳或模型自述，
/// 也不依赖可能 try_send 丢失的 UI Execution 日志——缺失/重复/乱序
/// 终态事件都不会留下永久豁免。其余 provider 不投递（行为零变化）。
pub trait TerminalActivitySnapshot: Send + Sync {
    /// 是否仍有在途（已登记/spawn 后未收口）terminal。
    fn has_active_terminals(&self) -> bool;
}

/// `TerminalActivitySnapshot` 的事件载荷包装：为 `ProviderEvent` 的
/// derive 合同提供 Clone/Debug/PartialEq/Eq（相等性按 Arc 指针同一性）。
pub struct TerminalActivityHandle(pub std::sync::Arc<dyn TerminalActivitySnapshot>);

impl Clone for TerminalActivityHandle {
    fn clone(&self) -> Self {
        Self(std::sync::Arc::clone(&self.0))
    }
}

impl std::fmt::Debug for TerminalActivityHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalActivityHandle")
            .finish_non_exhaustive()
    }
}

impl PartialEq for TerminalActivityHandle {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for TerminalActivityHandle {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    TextDelta {
        content: String,
    },
    PermissionRequest(PermissionRequestData),
    ChoiceRequest(ChoiceRequestData),
    StatusChanged(ProviderStatus),
    Execution(ProviderExecutionEvent),
    ToolCall(ProviderToolCall),
    ToolResult(ProviderToolResult),
    Completed(ProviderCompletion),
    Failed {
        message: String,
    },
    ProtocolError {
        code: String,
        message: String,
        context: Option<serde_json::Value>,
    },
    PermissionTimeout {
        permission_id: String,
    },
    /// Provider 完成时上报的 token 用量（best-effort）。
    ///
    /// 数据源因 provider 而异（claude stream-json result.usage、pi get_state cost 等）；
    /// 任一字段不可得时为 `None`，不视为错误。
    UsageReport(UsageReportData),
    /// 策略会话审批决策的结构化出口（GC11 canonical `approval_decision` 的内存形态；
    /// durable 落盘在后续 task 的 LifecycleStore sink 注入接线）。
    ToolPolicyDecision(CodexApprovalDecisionEvent),
    /// 未知审批形态告警的结构化出口（GC11 canonical `protocol_warning` 内存形态）。
    ToolPolicyWarning(CodexProtocolWarningEvent),
    /// 未知审批风暴终止的结构化出口（GC11 canonical `session_terminated` 内存形态）。
    ToolPolicyTerminated(CodexSessionTerminatedEvent),
    /// 批2 Task 7（Bug45）：会话真实终端活动快照（每会话一次投递；语义见
    /// [TerminalActivitySnapshot]）。观测性载体：idle 计时语义沿
    /// `provider_event_is_idle_activity`（本事件算真实活动）。
    TerminalActivity(TerminalActivityHandle),
}

/// 一次 provider 会话的 token 用量快照。
///
/// `role` 为投放侧角色（`author` / `reviewer`，由 adapter 依据 `AdapterRole` 归一化）；
/// 其余字段与 Anthropic 风格 usage 命名对齐，serde 使用 snake_case。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UsageReportData {
    pub role: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_creation_tokens: Option<u64>,
}

impl UsageReportData {
    /// 将 adapter role 归一化为 usage 角色：Reviewer → `reviewer`，其余（Orchestrator/
    /// Executor/WorkItemSplitter/Handoff）在 story/design 链路上均为产出侧 → `author`。
    pub fn role_text(role: &AdapterRole) -> &'static str {
        match role {
            AdapterRole::Reviewer => "reviewer",
            AdapterRole::Orchestrator
            | AdapterRole::Executor
            | AdapterRole::WorkItemSplitter
            | AdapterRole::Handoff => "author",
        }
    }

    /// 全部 token 字段均缺失时视为无可上报用量。
    pub fn has_any_tokens(&self) -> bool {
        self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cache_read_tokens.is_some()
            || self.cache_creation_tokens.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCommand {
    PermissionResponse {
        id: String,
        approved: bool,
        reason: Option<String>,
    },
    ChoiceResponse {
        id: String,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
        answers: Vec<ChoiceAnswerData>,
        /// P0 1.3：两层回执（mpsc 仅 Resolving；provider 等待者接收才
        /// Delivered）。旧调用方传 None，行为不变。
        receipt: Option<crate::cross_cutting::choice_delivery::ChoiceDeliverySignal>,
    },
    ToolResult(ProviderToolResult),
    Abort,
}

pub struct ProviderSession {
    pub events: mpsc::Receiver<ProviderEvent>,
    pub commands: mpsc::Sender<ProviderCommand>,
    /// 原生 provider session id（Task 3.2）：策略会话在 `start` 返回前完成有界
    /// 握手后写入（codex=thread id；claude=init session id/已知 resume id；
    /// pi=预生成 `--session-id`）。非策略/Coder/kimi 路径恒为 `None`（零变化）。
    pub native_session_id: Option<String>,
}

/// 🔴 **ProviderSession 终结保证契约**(r19 裁决冻结,oracle 2026-10-05):
///
/// `start`/`start_validated` 返回的会话必须满足:
///
/// > 子进程退出/泵任务结束或死亡 ⇒ `events` 在有界时间内以终态事件
/// > (`Failed`/`Completed`/…,先于一切无界 join 发出)或通道关闭(`recv`
/// > 返回 `None`)终结;任何存活等待不得以消费方 timeout 为唯一出口。
///
/// 两条红线(违反即缺陷,k3 审查对照):
///
/// - **红线 a**:会话内新增 `await` 位于「终结条件已成立」与「终态事件已
///   发出」之间时必须有界(组清理+有界 join;参照
///   `process_manager::run_session_tail_with_exit_watch`/`bounded_child_wait`/
///   `bounded_task_join`)。
/// - **红线 b**:`event_tx` 克隆寿命必须系于会话机构(Drop/cancel,如
///   ApprovalBridge 监听任务随桥消亡退出),不得系于消费方句柄(`commands`
///   发送端被消费方持有时通道仍必须可关闭)。

#[async_trait::async_trait]
pub trait StreamingProviderAdapter: Send + Sync {
    fn supports_tool_calls(&self) -> bool {
        false
    }

    /// legacy bridge 按角色矩阵派生策略会话时注入的 durable 审计 sink（engine 侧
    /// 注入点，P2-1）：真实 adapter 的策略会话缺 sink 会 fail-closed，engine 必须
    /// 在 legacy 直连路径同时提供策略与 sink。默认 `None`（非 engine 直调方不
    /// 派生 sink，策略会话对裸直调保持 fail-closed）。
    fn legacy_tool_policy_audit_sink(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>>
    {
        None
    }
    /// direct 启动:返回的会话必须满足 [ProviderSession 终结保证契约]
    /// (见上方 🔴 契约块与红线 a/b)——子进程退出 ⇒ `events` 有界终结,
    /// 不得依赖消费方 timeout 兜底。
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "streaming provider start is not implemented",
            0,
        ))
    }

    /// LC validated 启动(Task 1a 冻结签名):只接受 gateway 产出的
    /// `ValidatedStreamingProviderInput`。默认返回 unsupported,仅供未接入
    /// validated 路径的 adapter 显式拒绝;**不得**回调裸 `start`(raw
    /// `start` 仅保留单仓 direct)。真实 provider 的接入归 Task 7/1b。
    /// 返回的会话同样必须满足 [ProviderSession 终结保证契约](上方 🔴
    /// 契约块与红线 a/b)。
    async fn start_validated(
        &self,
        _input: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "streaming provider validated start is not supported by this adapter",
            0,
        ))
    }

    /// Task 3.2(B0a2):LC 公共 prepare 合同——adapter 以只读方式观测本家
    /// facts(exact version 经 [`LcVersionCache`] 60 秒 TTL、provider 特定
    /// approval/MCP/boundary facts、与该版本同源的纯 projector),供 gateway
    /// 装配 full projection。默认返回 typed unsupported,仅供未接入/测试
    /// adapter 显式拒绝(与上方 `start_validated` 默认同构);四家真实
    /// provider 的接入归 Task 3.3–3.6,参与 LC candidate 正例的 fake 必须
    /// 提供可观测 facts,不得靠默认拒绝使正例虚绿。
    async fn prepare_lc_facts(
        &self,
        _input: &StreamingProviderInput,
        _launch: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
        _cancel: CancellationToken,
    ) -> Result<
        crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "streaming provider lc prepare facts is not supported by this adapter",
            0,
        ))
    }

    /// **Legacy bridge**:`run_streaming` 把同步 `AdapterInput` 适配为流式 session,
    /// 仅供未携带逻辑代码库 target(`attempt.target_snapshot.is_none()`)的历史调用
    /// 使用。逻辑代码库真实 provider 启动必须经 `LogicalCodebaseProviderGateway`,
    /// 不得回落到本 bridge——这是 Task 12「关闭逻辑代码库裸输入与 legacy streaming
    /// fallback」的边界。逻辑目标的 `allow_legacy_stream_fallback` 被强制为 `false`,
    /// 因此 `provider_stream` 不会在 `start` 未实现时调用本方法。
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let audit_sink = self.legacy_tool_policy_audit_sink();
        let this = self;
        run_legacy_bridge_stream(
            audit_sink,
            Box::new(move |provider_input, cancel| {
                Box::pin(async move { this.start(provider_input, cancel).await })
            }),
            input,
            cancel,
        )
        .await
    }
}

/// legacy bridge 的启动闭包（返回装箱 future，供自由函数复用默认桥接体）。
pub(crate) type LegacyBridgeStart<'a> = Box<
    dyn FnOnce(
            StreamingProviderInput,
            CancellationToken,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<ProviderSession, ProviderAdapterError>>
                    + Send
                    + 'a,
            >,
        > + Send
        + 'a,
>;

/// 默认 legacy bridge 的桥接体（P2-1 抽出为自由函数）：`run_streaming` 默认实现
/// 与 engine 侧注入装饰器（`LegacyToolPolicyAuditProvider`）共用，避免装饰器经
/// 虚分发回调自身默认实现导致递归。
pub(crate) async fn run_legacy_bridge_stream<'a>(
    audit_sink: Option<
        std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>,
    >,
    start: LegacyBridgeStart<'a>,
    input: &'a AdapterInput,
    cancel: CancellationToken,
) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
    let working_dir = input.worktree_path.as_ref().map(PathBuf::from).unwrap_or(
        std::env::current_dir().map_err(|error| {
            ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
        })?,
    );
    // 批2 Task 11（B6 公共角色矩阵）：legacy 同步直连不携带策略字段，bridge
    // 也不再按 AdapterRole 一概派生 deny——Coder/Reviewer/规划族 wire 面
    // 全开；deny 是 Author 岗位纪律的唯一物理投影，由受信装配铸造
    // （AdapterInput 无策略字段，legacy 直连无从铸造）。engine 侧 hook 提供
    // 的 run-bound sink 照常注入（Reviewer 执行审计不随去 deny 丢失）；
    // kimi 不读 `tool_policy`，零物理变化。
    let tool_policy: Option<ProviderToolPolicy> = None;
    let provider_input = StreamingProviderInput {
        tool_policy,
        // P2-1：派生策略必须同时注入 sink（engine 侧 hook）。
        audit_sink: audit_sink.clone(),
        provider_type: input.provider_type.clone(),
        role: input.role.clone(),
        prompt: input.prompt.clone(),
        working_dir,
        // Task 2.5 cwd/target 分离：跨层透传 AdapterInput 的独立 cwd（None 时
        // effective 回填 working_dir，legacy 零行为变化）。
        working_directory: input.working_directory.clone(),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: input.timeout,
        baseline_tree: None,
    };
    let bridge_cancel = cancel.clone();
    let mut session = start(provider_input, cancel).await?;
    let (tx, rx) = mpsc::channel(32);

    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                _ = bridge_cancel.cancelled() => return,
                event = session.events.recv() => {
                    match event {
                        Some(event) => event,
                        None => return,
                    }
                }
            };
            let chunk = match event {
                ProviderEvent::TextDelta { content } => StreamChunk::Text(content),
                ProviderEvent::Completed(completion) => StreamChunk::Done {
                    full_output: completion.full_output,
                },
                ProviderEvent::Failed { message } => StreamChunk::Error(message),
                ProviderEvent::ProtocolError { message, .. } => StreamChunk::Error(message),
                ProviderEvent::PermissionTimeout { permission_id } => {
                    StreamChunk::Error(format!("Permission request {permission_id} timed out"))
                }
                ProviderEvent::PermissionRequest(request) => {
                    let _ = session
                        .commands
                        .send(ProviderCommand::PermissionResponse {
                            id: request.id,
                            approved: false,
                            reason: Some(
                                "run_streaming does not support interactive permission requests"
                                    .to_string(),
                            ),
                        })
                        .await;
                    let _ = tx
                        .send(StreamChunk::Error(
                            "interactive permission request is not supported in run_streaming"
                                .to_string(),
                        ))
                        .await;
                    return;
                }
                ProviderEvent::ChoiceRequest(request) => {
                    let _ = session
                        .commands
                        .send(ProviderCommand::ChoiceResponse {
                            id: request.id,
                            selected_option_ids: vec![],
                            free_text: Some("aborted".to_string()),
                            answers: vec![],
                            receipt: None,
                        })
                        .await;
                    let _ = tx
                        .send(StreamChunk::Error(
                            "interactive choice request is not supported in run_streaming"
                                .to_string(),
                        ))
                        .await;
                    return;
                }
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::Execution(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::ToolResult(_)
                | ProviderEvent::UsageReport(_)
                | ProviderEvent::ToolPolicyDecision(_)
                | ProviderEvent::ToolPolicyWarning(_)
                | ProviderEvent::ToolPolicyTerminated(_)
                | ProviderEvent::TerminalActivity(_) => {
                    continue;
                }
            };
            tokio::select! {
                _ = bridge_cancel.cancelled() => return,
                send_result = tx.send(chunk) => {
                    if send_result.is_err() {
                        return;
                    }
                }
            }
        }
    });

    Ok(rx)
}
