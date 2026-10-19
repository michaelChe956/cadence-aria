//! 批2 Task 5:先登记后启动、真实静止与资源回收合同(C1.3 / D S1/S6
//! 公共部 / B4 / REQ-ENV-HOST-01 / REVIEW-B / R11)。
//!
//! 四家 provider 全角色宿主全开后的资源归属基座:**持久预留 → 启动 →
//! 真实身份确认 → 静止证明 → 仅清登记项**。
//!
//! - 主键 [`ResourceScopeKey`] = `(scope_id, run_id, boot_identity)`:
//!   scope/run 来自 T2 冻结执行合同([`FrozenProviderExecution`] 的
//!   resource_scope_id/provider_run_id),boot_identity 标识平台进程本次
//!   启动——跨 boot 的登记是恢复材料,不是当前 boot 可静默处置的资产。
//! - 每条登记(`ProviderResourceRecord`)携带 resource_id/kind、预留
//!   launch 身份、真实启动身份(PID+`/proc` starttime 或容器/网络精确
//!   ID)、canonical 私有路径与 cleanup_cursor(恢复游标)。
//! - 标签至少 `aria.scope_id`/`aria.run_id`,Reviewer 再绑定
//!   `aria.window_id`;**不能只用进程名或容器名前缀**判归属。
//! - [`ProviderRunResourceScope::reserve_owned_resource`] 持久预留 →
//!   启动 → [`ProviderRunResourceScope::confirm_started_resource`]
//!   固化真实身份;登记/确认失败必须停止已启动资源并进入补偿,不能遗留
//!   无归属 writer。
//! - [`ProviderRunResourceScope::quiesce_scope`] 返回
//!   [`QuiesceOutcome::Quiescent`] 或 [`QuiesceOutcome::RecoveryRequired`]
//!   (unknown):killpg 只是其中一步,kill 返回成功不等于静止——必须以
//!   真实身份观察确认进程/容器确已停止。
//! - [`ProviderRunResourceScope::cleanup_owned_scope`] 仅消费 Quiescent
//!   及匹配登记项;共享容器/网络(标签不匹配)与 owner HOME(私有根外
//!   路径)永不清理;路径漂移停等;重复清理幂等。
//! - 普通 CLI、MCP/子进程、ACP terminal 均经 [`start_owned_process`] 等
//!   包装器使用此接口,**禁止依赖 Drop 收口**(Drop kill 只是既有兜底,
//!   静止/清理证明不来自它)。
//! - 普通 shell 自行 daemonize/绕登记调用引擎仍属同 UID 残留:未登记资源
//!   不保证回收,可识别未知 writer 必须停等;本模块不为此恢复封闭 grammar,
//!   也不宣称能拦截任意外部引擎。
//!
//! 消费:T2 scope identity、T3 冻结私有根(canonical 一次、清理目标必须
//! 仍在其下,不按 HOME/XDG/TMPDIR 值推导);持久化经
//! `product::provider_resource_store`(复用 `write_json` 的
//! tmp+flush+sync+rename 与 exclusive lock 范式)。接线方:T6/T7(ACP
//! terminal)、T14(Coder 清理)、T17/T21(Reviewer 窗口恢复)——交接
//! 完整资源 manifest 与恢复游标,不传布尔 `killed=true`。
// 生产消费链:T6/T7(ACP terminal/MCP 子进程)、T14(Coder 清理)、
// T17/T21(Reviewer 窗口恢复)接线;接线完成前仅合同测试消费
// (前瞻载体范式,同 `freeze_provider_execution`/`build_provider_environment`)。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::process_manager::{
    ManagedProcess, ProcessManager, process_start_time_ticks,
};
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_execution::FrozenProviderExecution;
use crate::product::provider_resource_store::ProviderResourceStore;

/// 归属标签:scope 主键维度(与 aria.run_id 一起构成最小归属键)。
pub(crate) const PROVIDER_RESOURCE_LABEL_SCOPE: &str = "aria.scope_id";
/// 归属标签:run 维度。
pub(crate) const PROVIDER_LABEL_RUN: &str = "aria.run_id";
/// 归属标签:协调 scope 归属 worktree(T19 已预留消费合同——Reviewer
/// 窗口 quiesce_targets 按 `aria.worktree`==tree_key 枚举兄弟 Coder/
/// terminal scope;T14 Coder code_delivery 登记时由 scope 注入,
/// 调用方伪造 aria.* 即拒绝)。
pub(crate) const PROVIDER_RESOURCE_LABEL_WORKTREE: &str = "aria.worktree";
pub(crate) const PROVIDER_RESOURCE_LABEL_WINDOW: &str = "aria.window_id";

/// quiesce 等待进程组真实静止的有界窗口(测试缩短保持回归秒级;
/// 与 `PROVIDER_CHILD_EXIT_STREAM_DRAIN` 同款 cfg 分档)。
#[cfg(not(test))]
pub(crate) const PROVIDER_QUIESCE_PROCESS_WAIT: Duration = Duration::from_secs(10);
#[cfg(test)]
pub(crate) const PROVIDER_QUIESCE_PROCESS_WAIT: Duration = Duration::from_secs(2);

/// 静止轮询间隔。
#[cfg(not(test))]
const QUIESCE_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(test)]
const QUIESCE_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// 资源种类。Directory=本轮私有目录;Process=provider/工具进程;
/// Container/Network=引擎(Podman 等)资源,须经 [`OwnedEngineObserver`]
/// 可观察通道核对精确 ID+标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnedResourceKind {
    Process,
    Directory,
    Container,
    Network,
}

/// 清理恢复游标(单调前进,持久化;崩溃后从游标续走,不重复物理动作)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CleanupStep {
    /// 已持久预留(启动副作用之前)。
    Reserved,
    /// 真实身份已固化。
    Confirmed,
    /// 静止已证明(quiesce_scope 成功路径推进)。
    Quiesced,
    /// 物理清理已执行(目录已删/引擎资源已移除)。
    Removed,
    /// 登记项已解除(文件删除;终态)。
    Deregistered,
}

/// 资源归属主键。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct ResourceScopeKey {
    pub(crate) scope_id: String,
    pub(crate) run_id: String,
    pub(crate) boot_identity: String,
}

/// 真实启动身份:PID 用 `/proc/<pid>/stat` starttime(第 22 字段)做
/// PID 复用防护;容器/网络用引擎精确 ID(不是名字前缀);目录用 canonical
/// 路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartedIdentity {
    Process { pid: u32, start_time_ticks: u64 },
    Container { exact_id: String },
    Network { exact_id: String },
    Directory { canonical_path: String },
}

/// 持久登记记录(store 落盘形状;T14/T17/T21 的 manifest 即本记录列表)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderResourceRecord {
    pub(crate) key: ResourceScopeKey,
    pub(crate) resource_id: String,
    pub(crate) kind: OwnedResourceKind,
    /// 预留 launch 身份(argv 摘要/镜像意图等;崩溃窗口内按此 + 标签
    /// 查找核对未确认副作用)。
    pub(crate) launch_intent: String,
    /// 归属标签(含 aria.scope_id/aria.run_id[+aria.window_id])。
    pub(crate) reserved_labels: BTreeMap<String, String>,
    /// 真实启动身份(确认后固化)。
    pub(crate) started_identity: Option<StartedIdentity>,
    /// canonical 私有路径(目录类资源必填)。
    pub(crate) canonical_private_path: Option<String>,
    /// 清理恢复游标。
    pub(crate) cleanup_cursor: CleanupStep,
    pub(crate) reserved_at: String,
    pub(crate) updated_at: String,
}

impl ProviderResourceRecord {
    /// 登记事实相等(忽略时间戳与游标):reserve 幂等/confirm 预留核对、
    /// cleanup 匹配登记项共用。
    pub(crate) fn matches_registration(&self, other: &ProviderResourceRecord) -> bool {
        self.key == other.key
            && self.resource_id == other.resource_id
            && self.kind == other.kind
            && self.launch_intent == other.launch_intent
            && self.reserved_labels == other.reserved_labels
            && self.canonical_private_path == other.canonical_private_path
            && self.started_identity == other.started_identity
    }
}

/// 冻结类型①:资源预留意图(调用方构造;`aria.*` 归属标签由 scope 注入,
/// 调用方伪造即拒绝)。
#[derive(Debug, Clone)]
pub(crate) struct OwnedResourceSpec {
    resource_id: String,
    kind: OwnedResourceKind,
    launch_intent: String,
    canonical_private_path: Option<PathBuf>,
    labels: BTreeMap<String, String>,
}

impl OwnedResourceSpec {
    pub(crate) fn new(resource_id: &str, kind: OwnedResourceKind, launch_intent: &str) -> Self {
        Self {
            resource_id: resource_id.to_string(),
            kind,
            launch_intent: launch_intent.to_string(),
            canonical_private_path: None,
            labels: BTreeMap::new(),
        }
    }

    /// 目录类资源必填:先创建并 canonicalize 的私有路径。
    pub(crate) fn with_private_path(mut self, path: PathBuf) -> Self {
        self.canonical_private_path = Some(path);
        self
    }

    /// 附加非 `aria.*` 业务标签。
    pub(crate) fn with_label(mut self, key: &str, value: &str) -> Self {
        self.labels.insert(key.to_string(), value.to_string());
        self
    }
}

/// 冻结类型②:持久预留句柄(reserve 成功产物;字段私有,不可 serde 铸造)。
#[derive(Debug, Clone)]
pub(crate) struct ResourceReservation {
    key: ResourceScopeKey,
    resource_id: String,
    kind: OwnedResourceKind,
    launch_intent: String,
    reserved_labels: BTreeMap<String, String>,
    canonical_private_path: Option<PathBuf>,
}

impl ResourceReservation {
    pub(crate) fn key(&self) -> &ResourceScopeKey {
        &self.key
    }

    pub(crate) fn resource_id(&self) -> &str {
        &self.resource_id
    }

    #[allow(dead_code)] // 预留 kind 读取口:生产消费者随 F2 真实进程登记接线,暂无调用面不删契约
    pub(crate) fn kind(&self) -> OwnedResourceKind {
        self.kind
    }

    #[cfg_attr(not(test), allow(dead_code))] // 测试断言消费;生产消费者随 F2 接线
    pub(crate) fn reserved_labels(&self) -> &BTreeMap<String, String> {
        &self.reserved_labels
    }
}

/// 冻结类型③:真实启动身份(confirm 输入;kind 与预留一致才接受)。
#[derive(Debug, Clone)]
pub(crate) struct StartedResourceIdentity {
    identity: StartedIdentity,
}

impl StartedResourceIdentity {
    pub(crate) fn process(pid: u32, start_time_ticks: u64) -> Self {
        Self {
            identity: StartedIdentity::Process {
                pid,
                start_time_ticks,
            },
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // 容器身份铸造口:测试消费,容器清理链接线位
    pub(crate) fn container(exact_id: &str) -> Self {
        Self {
            identity: StartedIdentity::Container {
                exact_id: exact_id.to_string(),
            },
        }
    }

    #[allow(dead_code)] // 网络身份铸造口:精确 ID 清理合同面,接线前无消费者
    pub(crate) fn network(exact_id: &str) -> Self {
        Self {
            identity: StartedIdentity::Network {
                exact_id: exact_id.to_string(),
            },
        }
    }

    pub(crate) fn directory(canonical_path: &Path) -> Self {
        Self {
            identity: StartedIdentity::Directory {
                canonical_path: canonical_path.display().to_string(),
            },
        }
    }

    pub(crate) fn identity(&self) -> &StartedIdentity {
        &self.identity
    }
}

/// 冻结类型④:静止证明。
#[derive(Debug, Clone)]
pub(crate) struct QuiescentScope {
    key: ResourceScopeKey,
    /// 静止证明覆盖的完整资源 manifest(交接 T14/T17;含恢复游标)。
    manifest: Vec<ProviderResourceRecord>,
    quiesced_at: String,
}

impl QuiescentScope {
    pub(crate) fn key(&self) -> &ResourceScopeKey {
        &self.key
    }

    pub(crate) fn manifest(&self) -> &[ProviderResourceRecord] {
        &self.manifest
    }

    pub(crate) fn quiesced_at(&self) -> &str {
        &self.quiesced_at
    }
}

/// RecoveryRequired 载荷:可识别的未知/未静止事实与停等材料。
#[derive(Debug, Clone)]
pub(crate) struct QuiesceUnknown {
    /// 稳定原因码(`provider_resources_quiesce_*`)。
    pub(crate) reason: String,
    /// 诊断细节(含 resource_id/精确 ID/launch 意图;无凭据)。
    pub(crate) detail: String,
    /// 停等时的完整 manifest(恢复消费方接管材料)。
    pub(crate) manifest: Vec<ProviderResourceRecord>,
}

/// 冻结类型⑤:quiesce 结局——静止证明或停等(unknown)。killpg 成功≠静止。
#[derive(Debug, Clone)]
pub(crate) enum QuiesceOutcome {
    Quiescent(QuiescentScope),
    RecoveryRequired(QuiesceUnknown),
}

/// 引擎资源(Podman 等)可观察通道:quiesce/cleanup 只经精确 ID+标签
/// 核对,不加 cgroup/UID 或广域容器清理。T14/T17/T22 接真实引擎;T5
/// 合同 + 测试 fake。**不能用 kill/create 失败当无副作用**:停止后必须
/// 重新观察,未确认身份按预留 label 查找核对。
pub(crate) trait OwnedEngineObserver: Send + Sync {
    /// 观察精确 ID 的引擎资源当前是否仍在运行(停止后的静止复核)。
    fn observe_running(
        &self,
        kind: OwnedResourceKind,
        exact_id: &str,
    ) -> Result<bool, ProviderAdapterError>;

    /// 按预留标签查找引擎侧资源(未确认身份的副作用核对)。返回匹配的
    /// 精确 ID 列表。
    fn find_by_reserved_labels(
        &self,
        kind: OwnedResourceKind,
        labels: &BTreeMap<String, String>,
    ) -> Result<Vec<String>, ProviderAdapterError>;

    /// 停止精确 ID 且标签均匹配的引擎资源(quiesce 的引擎侧一步)。
    /// 返回是否命中(ID+标签均匹配才处理;共享资源不碰)。
    fn stop_exact(
        &self,
        kind: OwnedResourceKind,
        exact_id: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<bool, ProviderAdapterError>;

    /// 移除精确 ID 且标签均匹配的引擎资源(cleanup 步)。共享资源不碰。
    fn remove_exact(
        &self,
        kind: OwnedResourceKind,
        exact_id: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<bool, ProviderAdapterError>;
}

/// 新平台 boot 身份(进程级唯一;恢复判定用)。
#[cfg_attr(not(test), allow(dead_code))] // 生产入口 open 的 boot 身份铸造;恢复演练/测试先行(F2 接线位)
pub(crate) fn new_boot_identity() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("boot_{}_{nanos}", std::process::id())
}

#[cfg(unix)]
enum ProcessObservation {
    /// 进程已消失(或僵尸:不再可能写任何文件)。
    Gone,
    /// 存活且 (pid, starttime) 与登记身份一致 → 是登记资源本体。
    Ours,
    /// PID 已被新进程复用(starttime 不同)→ 登记的原进程已消失,
    /// **绝不向该 PID 发信号**。
    ReusedByOther,
}

#[cfg(unix)]
fn observe_process(pid: u32, start_time_ticks: u64) -> ProcessObservation {
    let Some((state, observed_start)) = read_process_stat(pid) else {
        return ProcessObservation::Gone;
    };
    if state == 'Z' {
        return ProcessObservation::Gone;
    }
    if observed_start == start_time_ticks {
        ProcessObservation::Ours
    } else {
        ProcessObservation::ReusedByOther
    }
}

/// 读 `/proc/<pid>/stat`:state(第3字段)与 starttime(第22字段,
/// 时钟 ticks)。comm 可含空格/括号,按 `) ` 右侧切分。
#[cfg(unix)]
fn read_process_stat(pid: u32) -> Option<(char, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, tail) = stat.rsplit_once(") ")?;
    let state = tail.chars().next()?;
    // tail 从第3字段(state)开始:starttime=第22字段 → nth(19)。
    let start_time_ticks = tail.split_whitespace().nth(19)?.parse().ok()?;
    Some((state, start_time_ticks))
}

/// 读 `/proc/<pid>/stat` 的 state(第3字段)与 pgrp(第5字段)——
/// 组残留扫描用(与 [`read_process_stat`] 同款 `) ` 切分解析)。
#[cfg(unix)]
fn read_process_stat_group(pid: u32) -> Option<(char, u32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, tail) = stat.rsplit_once(") ")?;
    let state = tail.chars().next()?;
    // tail 从第3字段(state)开始:pgrp=第5字段 → nth(2)。
    let pgrp = tail.split_whitespace().nth(2)?.parse().ok()?;
    Some((state, pgrp))
}

/// 身份核对后的组清理:仅当 (pid, starttime) 仍匹配登记身份时 killpg;
/// PID 复用/已消失一律不发信号。返回是否发出了信号。
#[cfg(unix)]
fn kill_process_group_if_ours(pid: u32, start_time_ticks: u64) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    if !matches!(
        observe_process(pid, start_time_ticks),
        ProcessObservation::Ours
    ) {
        return false;
    }
    // SAFETY: killpg 只作用于校验过的正整数 pgid(组长即 child pid)。
    unsafe { libc::killpg(pid as i32, libc::SIGKILL) == 0 }
}

#[cfg(not(unix))]
fn kill_process_group_if_ours(_pid: u32, _start_time_ticks: u64) -> bool {
    false
}

/// 进程组(pgid=spawn 时组长 pid)内是否仍有存活进程(非僵尸;僵尸不再
/// 可能写任何文件)。F2:组长已退出时组内残留=无法安全组杀的未知
/// writer,必须停等。
#[cfg(unix)]
fn process_group_has_members(pgid: u32) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let Some((state, pgrp)) = read_process_stat_group(pid) else {
            continue;
        };
        if state != 'Z' && pgrp == pgid {
            return true;
        }
    }
    false
}

#[cfg(not(unix))]
fn process_group_has_members(_pgid: u32) -> bool {
    false
}

fn store_error(error: crate::product::json_store::ProductStoreError) -> ProviderAdapterError {
    ProviderAdapterError::provider_unavailable(format!("provider_resources_store: {error}"))
}

fn engine_exact_id(identity: &StartedIdentity) -> Option<&str> {
    match identity {
        StartedIdentity::Container { exact_id } | StartedIdentity::Network { exact_id } => {
            Some(exact_id)
        }
        _ => None,
    }
}

fn quiesce_recovery(
    reason: &str,
    detail: String,
    manifest: Vec<ProviderResourceRecord>,
) -> QuiesceOutcome {
    QuiesceOutcome::RecoveryRequired(QuiesceUnknown {
        reason: reason.to_string(),
        detail,
        manifest,
    })
}

/// 资源归属 scope:一个 (scope_id, run_id, boot_identity) 的登记/静止/
/// 清理面。crate 内受信构造(`open` 取 T2 冻结合同;`open_for_recovery`
/// 供 T14/T17/T21 接管 stale boot 的登记驱动补偿)。
pub(crate) struct ProviderRunResourceScope {
    key: ResourceScopeKey,
    review_window_id: Option<String>,
    /// T14:协调归属 worktree(tree_key;预留时注入 aria.worktree)。
    worktree_key: Option<String>,
    /// T3 冻结私有根(open 时 canonical 一次;清理目标必须仍在其下)。
    private_root: PathBuf,
    store: ProviderResourceStore,
    engine_observer: Option<Arc<dyn OwnedEngineObserver>>,
}

impl ProviderRunResourceScope {
    /// 生产入口:以本平台 boot 身份打开 run 的资源 scope。
    #[cfg_attr(not(test), allow(dead_code))] // 生产入口(engine 开真实 scope);接线前测试经 open_scope 消费
    pub(crate) fn open(
        execution: &FrozenProviderExecution,
        boot_identity: &str,
        private_root: &Path,
        store_root: &Path,
        engine_observer: Option<Arc<dyn OwnedEngineObserver>>,
    ) -> Result<Self, ProviderAdapterError> {
        Self::open_keyed(
            ResourceScopeKey {
                scope_id: execution.resource_scope_id().to_string(),
                run_id: execution.provider_run_id().to_string(),
                boot_identity: boot_identity.to_string(),
            },
            execution.review_window_id().map(str::to_string),
            private_root,
            store_root,
            engine_observer,
        )
    }

    /// 恢复入口:接管 stale boot 的 scope(进程按 durable PID-start 身份
    /// 核对,不凭进程名)。
    pub(crate) fn open_for_recovery(
        key: &ResourceScopeKey,
        review_window_id: Option<&str>,
        private_root: &Path,
        store_root: &Path,
        engine_observer: Option<Arc<dyn OwnedEngineObserver>>,
    ) -> Result<Self, ProviderAdapterError> {
        Self::open_keyed(
            key.clone(),
            review_window_id.map(str::to_string),
            private_root,
            store_root,
            engine_observer,
        )
    }

    fn open_keyed(
        key: ResourceScopeKey,
        review_window_id: Option<String>,
        private_root: &Path,
        store_root: &Path,
        engine_observer: Option<Arc<dyn OwnedEngineObserver>>,
    ) -> Result<Self, ProviderAdapterError> {
        if key.scope_id.is_empty() || key.run_id.is_empty() || key.boot_identity.is_empty() {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_resources_scope_key_empty: scope_id/run_id/boot_identity 均必须非空",
            ));
        }
        let canonical_root = private_root.canonicalize().map_err(|error| {
            ProviderAdapterError::provider_unavailable(format!(
                "provider_resources_private_root_not_resolvable: {} ({error});私有根必须先由 engine 创建",
                private_root.display()
            ))
        })?;
        Ok(Self {
            key,
            review_window_id,
            worktree_key: None,
            private_root: canonical_root,
            store: ProviderResourceStore::new(store_root),
            engine_observer,
        })
    }

    /// T14:协调归属——绑定 worktree(tree_key)。`aria.worktree` 归属
    /// 标签由 scope 在预留时注入(调用方伪造 aria.* 即拒绝);必须在
    /// 任何 reserve 之前调用——此前已落盘的登记记录不回填该标签。
    /// 消费方:T19 Reviewer 窗口 quiesce_targets 按
    /// `aria.worktree`==tree_key 枚举兄弟 Coder scope。
    pub(crate) fn with_worktree_key(mut self, tree_key: &str) -> Self {
        self.worktree_key = Some(tree_key.to_string());
        self
    }

    pub(crate) fn key(&self) -> &ResourceScopeKey {
        &self.key
    }

    /// 归属标签(注入 aria.* 归属键;调用方标签不得伪造 aria.*)。
    fn ownership_labels(&self, extra: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut labels = extra.clone();
        labels.insert(
            PROVIDER_RESOURCE_LABEL_SCOPE.to_string(),
            self.key.scope_id.clone(),
        );
        labels.insert(PROVIDER_LABEL_RUN.to_string(), self.key.run_id.clone());
        if let Some(window) = &self.review_window_id {
            labels.insert(PROVIDER_RESOURCE_LABEL_WINDOW.to_string(), window.clone());
        }
        if let Some(tree_key) = &self.worktree_key {
            labels.insert(
                PROVIDER_RESOURCE_LABEL_WORKTREE.to_string(),
                tree_key.clone(),
            );
        }
        labels
    }

    /// 持久预留:先于任何启动副作用落盘。失败即调用方不得启动
    /// (`start_owned_process` 据此保证 spawn 计数 0)。
    pub(crate) fn reserve_owned_resource(
        &self,
        spec: &OwnedResourceSpec,
    ) -> Result<ResourceReservation, ProviderAdapterError> {
        if spec.resource_id.trim().is_empty() {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_resources_resource_id_empty",
            ));
        }
        if spec.launch_intent.trim().is_empty() {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_resources_launch_intent_empty",
            ));
        }
        // aria.* 归属标签只由 scope 注入——调用方伪造即拒绝(不以清空/错位
        // 标签取得归属)。
        for key in spec.labels.keys() {
            if key.starts_with("aria.") {
                return Err(ProviderAdapterError::provider_unavailable(format!(
                    "provider_resources_reserved_label_rejected: {key} 属 scope 归属键,调用方标签不得伪造 aria.*"
                )));
            }
        }
        let canonical_private_path = match &spec.canonical_private_path {
            Some(path) => {
                // 启动私有目录必须先创建/canonicalize/登记:路径必须已存在、
                // 已是 canonical 形态,且仍在 T3 冻结私有根之下。
                let canonical = path.canonicalize().map_err(|error| {
                    ProviderAdapterError::provider_unavailable(format!(
                        "provider_resources_private_path_not_resolvable: {} ({error});目录必须先创建再登记",
                        path.display()
                    ))
                })?;
                if canonical != *path {
                    return Err(ProviderAdapterError::provider_unavailable(format!(
                        "provider_resources_private_path_not_canonical: {} != {};必须先 canonicalize 再登记",
                        path.display(),
                        canonical.display()
                    )));
                }
                // F4(审查修复轮):根自身不可登记——remove_dir_all(根)会
                // 连带清除其它资源的私有目录;必须是严格子路径。
                if canonical == self.private_root || !canonical.starts_with(&self.private_root) {
                    return Err(ProviderAdapterError::provider_unavailable(format!(
                        "provider_resources_private_path_outside_root: {} 必须是冻结私有根 {} 的严格子路径(根自身不可登记)",
                        canonical.display(),
                        self.private_root.display()
                    )));
                }
                Some(canonical)
            }
            None if spec.kind == OwnedResourceKind::Directory => {
                return Err(ProviderAdapterError::provider_unavailable(
                    "provider_resources_directory_requires_private_path",
                ));
            }
            None => None,
        };

        let labels = self.ownership_labels(&spec.labels);
        let now = chrono::Utc::now().to_rfc3339();
        let record = ProviderResourceRecord {
            key: self.key.clone(),
            resource_id: spec.resource_id.clone(),
            kind: spec.kind,
            launch_intent: spec.launch_intent.clone(),
            reserved_labels: labels.clone(),
            started_identity: None,
            canonical_private_path: canonical_private_path
                .as_ref()
                .map(|path| path.display().to_string()),
            cleanup_cursor: CleanupStep::Reserved,
            reserved_at: now.clone(),
            updated_at: now,
        };
        self.store.reserve(&record).map_err(store_error)?;
        Ok(ResourceReservation {
            key: self.key.clone(),
            resource_id: spec.resource_id.clone(),
            kind: spec.kind,
            launch_intent: spec.launch_intent.clone(),
            reserved_labels: labels,
            canonical_private_path,
        })
    }

    /// 固化真实身份:预留事实(kind/launch_intent/labels/私有路径)必须
    /// 与 durable 记录一致;kind 与身份形态一致;Directory 身份必须等于
    /// 预留 canonical 路径。
    pub(crate) fn confirm_started_resource(
        &self,
        reservation: &ResourceReservation,
        identity: StartedResourceIdentity,
    ) -> Result<(), ProviderAdapterError> {
        if reservation.key != self.key {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_resources_reservation_scope_mismatch: 预留不属于本 scope",
            ));
        }
        let started = identity.identity().clone();
        // 身份形态必须与预留 kind 一致,且字段非空可恢复。
        match (&started, reservation.kind) {
            (
                StartedIdentity::Process {
                    pid,
                    start_time_ticks,
                },
                OwnedResourceKind::Process,
            ) => {
                if *pid == 0 || *start_time_ticks == 0 {
                    return Err(ProviderAdapterError::provider_unavailable(
                        "provider_resources_invalid_start_identity: pid/starttime 必须为真实观察值",
                    ));
                }
            }
            (StartedIdentity::Container { exact_id }, OwnedResourceKind::Container)
            | (StartedIdentity::Network { exact_id }, OwnedResourceKind::Network) => {
                if exact_id.trim().is_empty() {
                    return Err(ProviderAdapterError::provider_unavailable(
                        "provider_resources_invalid_start_identity: 引擎精确 ID 必须非空",
                    ));
                }
            }
            (StartedIdentity::Directory { canonical_path }, OwnedResourceKind::Directory) => {
                let reserved = reservation
                    .canonical_private_path
                    .as_ref()
                    .map(|path| path.display().to_string());
                if reserved.as_deref() != Some(canonical_path.as_str()) {
                    return Err(ProviderAdapterError::provider_unavailable(
                        "provider_resources_directory_identity_path_mismatch: 目录身份必须等于预留 canonical 路径",
                    ));
                }
            }
            _ => {
                return Err(ProviderAdapterError::provider_unavailable(
                    "provider_resources_identity_kind_mismatch: 身份形态与资源种类不一致",
                ));
            }
        }
        let reserved_path = reservation
            .canonical_private_path
            .as_ref()
            .map(|path| path.display().to_string());
        self.store
            .confirm_started(
                &reservation.key,
                &reservation.resource_id,
                &started,
                &reservation.launch_intent,
                &reservation.reserved_labels,
                reserved_path.as_deref(),
            )
            .map(|_| ())
            .map_err(store_error)
    }

    /// quiesce 让位交接轮数:每轮一次 `yield_now` + 重读,供在途收口注销
    /// 任务(未确认预留的解除是纯同步 store 写)完成调度。真实崩溃窗口
    /// 的未确认预留不会因让位变绿,窗口耗尽后仍停等。
    const QUIESCE_INFLIGHT_RELEASE_HANDOFF_ROUNDS: usize = 4;

    fn has_inflight_unconfirmed_process_record(records: &[ProviderResourceRecord]) -> bool {
        records.iter().any(|record| {
            record.cleanup_cursor < CleanupStep::Removed
                && matches!(record.kind, OwnedResourceKind::Process)
                && !matches!(
                    record.started_identity.as_ref(),
                    Some(StartedIdentity::Process { .. })
                )
        })
    }

    /// 静止证明:killpg/stop 只是其中一步,之后必须以真实身份观察确认;
    /// 未知 writer/未静止 → RecoveryRequired(停等,durable 状态不动)。
    pub(crate) async fn quiesce_scope(&self) -> Result<QuiesceOutcome, ProviderAdapterError> {
        let mut records = self.store.list_scope(&self.key).map_err(store_error)?;
        // N1:未确认 Process 预留可能是「收口注销任务已在途」——会话尾部
        // 对引擎 cancel/消费方丢弃的响应经 tokio::spawn 异步释放,而引擎
        // 错误链(技术重试旋转/中止)可能全程无 await 点同步回传,注销
        // 任务尚未被调度。判定未知 writer 停等前先做有界让位交接:yield
        // 后重读,在途注销完成即正常推进;窗口耗尽仍未确认才维持
        // fail-closed 停等(真实崩溃窗口语义不变,不清理、不动 durable)。
        if Self::has_inflight_unconfirmed_process_record(&records) {
            for _ in 0..Self::QUIESCE_INFLIGHT_RELEASE_HANDOFF_ROUNDS {
                tokio::task::yield_now().await;
                records = self.store.list_scope(&self.key).map_err(store_error)?;
                if !Self::has_inflight_unconfirmed_process_record(&records) {
                    break;
                }
            }
        }
        if records.is_empty() {
            return Ok(QuiesceOutcome::Quiescent(QuiescentScope {
                key: self.key.clone(),
                manifest: Vec::new(),
                quiesced_at: chrono::Utc::now().to_rfc3339(),
            }));
        }
        for record in &records {
            // P3(修复轮 2):物理清理已完成(Removed+)的恢复重入记录跳过
            // 进程/引擎检查——旧 pgid/精确 ID 可能已被复用为无关新资源,
            // 残留扫描/停等会假阳性卡死收口;不误删方向不变(cleanup 面
            // 的全部守卫保留,F1 游标保持不变)。
            if record.cleanup_cursor >= CleanupStep::Removed {
                continue;
            }
            match record.kind {
                OwnedResourceKind::Process => match record.started_identity.as_ref() {
                    Some(StartedIdentity::Process {
                        pid,
                        start_time_ticks,
                    }) => {
                        // killpg 仅其中一步:先发组信号,再以真实身份观察。
                        // F2/F2b(审查修复轮):组长已退出(daemonize/调用方
                        // 先 wait)时无法安全 killpg(PID 可能已被复用为新组
                        // 长),组内残留进程须检出停等——与
                        // ManagedProcessChild「组长回收后组清理仍须可达」
                        // 同一风险面,但 quiesce 只凭 durable 身份,不能无
                        // 身份核对地组杀。残留判定以 **killpg 实际送达** 为
                        // 准(kill 返回值),不用 kill 前快照——快照与 kill
                        // 内部二次核对之间的 µs 级退出窗口会漏判活跃 writer。
                        let group_killed = kill_process_group_if_ours(*pid, *start_time_ticks);
                        if !self.wait_process_gone(*pid, *start_time_ticks).await {
                            return Ok(quiesce_recovery(
                                "provider_resources_quiesce_registered_resource_running",
                                format!(
                                    "resource={} pid={pid}: killpg 已发但 (pid,starttime) 身份仍存活,静止未证明",
                                    record.resource_id
                                ),
                                records,
                            ));
                        }
                        if !group_killed && process_group_has_members(*pid) {
                            return Ok(quiesce_recovery(
                                "provider_resources_quiesce_process_group_residual",
                                format!(
                                    "resource={} pgid={pid}: 组信号未送达(组长身份已失效)但组内仍有存活进程——无法安全组杀,停等",
                                    record.resource_id
                                ),
                                records,
                            ));
                        }
                    }
                    _ => {
                        return Ok(quiesce_recovery(
                            "provider_resources_quiesce_unconfirmed_process_unknown",
                            format!(
                                "resource={} launch={}: 预留未确认(崩溃窗口),是否已 spawn 不可证——可识别未知 writer 停等",
                                record.resource_id, record.launch_intent
                            ),
                            records,
                        ));
                    }
                },
                OwnedResourceKind::Container | OwnedResourceKind::Network => {
                    let Some(observer) = self.engine_observer.as_ref() else {
                        return Ok(quiesce_recovery(
                            "provider_resources_quiesce_observer_missing",
                            format!(
                                "resource={}: 无引擎可观察通道,无法证明静止",
                                record.resource_id
                            ),
                            records,
                        ));
                    };
                    match record.started_identity.as_ref() {
                        Some(started) => {
                            let Some(exact_id) = engine_exact_id(started) else {
                                return Err(ProviderAdapterError::provider_unavailable(format!(
                                    "provider_resources_record_identity_kind_mismatch: {}",
                                    record.resource_id
                                )));
                            };
                            // stop(killpg 等价一步)后必须重新观察。
                            let stop_matched = observer.stop_exact(
                                record.kind,
                                exact_id,
                                &record.reserved_labels,
                            )?;
                            if observer.observe_running(record.kind, exact_id)? {
                                let reason = if stop_matched {
                                    "provider_resources_quiesce_registered_resource_running"
                                } else {
                                    // 精确 ID 在但标签不属本轮(共享):无权停止,
                                    // 也永不清理。
                                    "provider_resources_quiesce_not_owned_shared_resource_running"
                                };
                                return Ok(quiesce_recovery(
                                    reason,
                                    format!(
                                        "resource={} exact_id={exact_id}: stop 后仍运行,静止未证明",
                                        record.resource_id
                                    ),
                                    records,
                                ));
                            }
                        }
                        None => {
                            // 不能把 create 失败当无副作用:按预留 label 查找核对。
                            let found = observer
                                .find_by_reserved_labels(record.kind, &record.reserved_labels)?;
                            if !found.is_empty() {
                                return Ok(quiesce_recovery(
                                    "provider_resources_quiesce_unconfirmed_engine_side_effect",
                                    format!(
                                        "resource={} launch={}: 预留未确认但引擎侧有带同标签实例 [{}]——停等核对",
                                        record.resource_id,
                                        record.launch_intent,
                                        found.join(",")
                                    ),
                                    records,
                                ));
                            }
                        }
                    }
                }
                OwnedResourceKind::Directory => {
                    // 目录无独立 writer 面:静止由进程/引擎记录证明,此处无需动作。
                }
            }
        }
        // 全部静止:构造证明(交接完整 manifest,不传布尔 killed)。
        // F1(审查修复轮):游标已 >= Quiesced 的记录(如 cleanup 在 Removed
        // 写入后、解除登记前崩溃)原样保留,不回推 Quiesced——恢复路径
        // 重入 quiesce 不再因「回退拒绝」永久卡死;只有低于 Quiesced 才推进。
        let mut manifest = Vec::with_capacity(records.len());
        for record in records {
            let record = if record.cleanup_cursor >= CleanupStep::Quiesced {
                record
            } else {
                self.store
                    .advance_cleanup_cursor(&self.key, &record.resource_id, CleanupStep::Quiesced)
                    .map_err(store_error)?
            };
            manifest.push(record);
        }
        Ok(QuiesceOutcome::Quiescent(QuiescentScope {
            key: self.key.clone(),
            manifest,
            quiesced_at: chrono::Utc::now().to_rfc3339(),
        }))
    }

    /// 有界等待进程以登记身份真实消失(Gone 或 PID 被复用均证明原进程
    /// 不再以该身份运行);窗口耗尽返回 false。
    #[cfg(unix)]
    async fn wait_process_gone(&self, pid: u32, start_time_ticks: u64) -> bool {
        let deadline = tokio::time::Instant::now() + PROVIDER_QUIESCE_PROCESS_WAIT;
        loop {
            if !matches!(
                observe_process(pid, start_time_ticks),
                ProcessObservation::Ours
            ) {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(QUIESCE_POLL_INTERVAL).await;
        }
    }

    /// 非 unix 无 /proc 观察面(合同按 unix 观察语义验收)。
    #[cfg(not(unix))]
    #[allow(clippy::unused_self)]
    async fn wait_process_gone(&self, _pid: u32, _start_time_ticks: u64) -> bool {
        true
    }

    /// B1-1:单个 Process 登记的收口注销(spawn 点生命周期尾部——会话
    /// 真实终止/错误路径)。与 `quiesce_scope` 的进程面同一语义,只作用于
    /// 本 launch 的 resource_id:
    ///
    /// - 身份已确认:killpg(仅 (pid,starttime) 核对通过)→ 有界观察组长
    ///   真实消失 → 组残留检查(killpg 未送达但组内有存活进程=停等),
    ///   全部通过才游标推进 Quiesced 并解除登记——**真实进程组收口后
    ///   注销**,不是协议 Completed、也不是 killpg 返回成功;
    /// - 身份未确认(默认面错误路径,adapter 自身已 kill+wait;或正常
    ///   尾部——终结保证契约下源通道关闭即子进程退出后):直接注销,
    ///   durable 残留风险交由 scope 级 quiesce 的未知 writer 停等边界;
    /// - 静止不可证:Err 停等(记录保留,恢复驱动/下一轮开窗 sweep 接管);
    /// - 幂等:登记项不存在即 Ok。
    pub(crate) async fn release_process_registration(
        &self,
        resource_id: &str,
    ) -> Result<(), ProviderAdapterError> {
        let Some(record) = self
            .store
            .load_record(&self.key, resource_id)
            .map_err(store_error)?
        else {
            return Ok(());
        };
        if record.cleanup_cursor >= CleanupStep::Removed {
            return Ok(());
        }
        match record.kind {
            OwnedResourceKind::Process => {}
            other => {
                return Err(ProviderAdapterError::provider_unavailable(format!(
                    "provider_resources_release_kind_mismatch: {resource_id} 是 {:?},不是进程登记",
                    other
                )));
            }
        }
        if let Some(StartedIdentity::Process {
            pid,
            start_time_ticks,
        }) = record.started_identity.as_ref()
        {
            // killpg 仅其中一步:先发组信号,再以真实身份有界观察。
            let group_killed = kill_process_group_if_ours(*pid, *start_time_ticks);
            if !self.wait_process_gone(*pid, *start_time_ticks).await {
                return Err(ProviderAdapterError::provider_unavailable(format!(
                    "provider_resources_release_registered_resource_running: resource={resource_id} pid={pid}: killpg 已发但 (pid,starttime) 身份仍存活,静止未证明——保留登记停等"
                )));
            }
            if !group_killed && process_group_has_members(*pid) {
                return Err(ProviderAdapterError::provider_unavailable(format!(
                    "provider_resources_release_process_group_residual: resource={resource_id} pgid={pid}: 组信号未送达(组长身份已失效)但组内仍有存活进程——无法安全组杀,保留登记停等"
                )));
            }
        }
        if record.cleanup_cursor < CleanupStep::Quiesced {
            self.store
                .advance_cleanup_cursor(&self.key, resource_id, CleanupStep::Quiesced)
                .map_err(store_error)?;
        }
        self.store
            .deregister(&self.key, resource_id)
            .map_err(store_error)?;
        Ok(())
    }

    /// 仅消费 Quiescent 及匹配登记项:目录仅删私有根下 canonical 未漂移
    /// 路径;引擎资源仅移除精确 ID+标签均匹配项;共享资源与 owner HOME
    /// 永不清理;重复清理幂等。
    pub(crate) fn cleanup_owned_scope(
        &self,
        quiescent: &QuiescentScope,
    ) -> Result<(), ProviderAdapterError> {
        if quiescent.key() != &self.key {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_resources_cleanup_scope_mismatch: 静止证明不属于本 scope",
            ));
        }
        let durable = self.store.list_scope(&self.key).map_err(store_error)?;
        for manifest_record in quiescent.manifest() {
            let Some(current) = durable
                .iter()
                .find(|record| record.resource_id == manifest_record.resource_id)
            else {
                // 登记项已解除:重复清理幂等。
                continue;
            };
            if !current.matches_registration(manifest_record) {
                return Err(ProviderAdapterError::provider_unavailable(format!(
                    "provider_resources_cleanup_registration_mismatch: {}",
                    current.resource_id
                )));
            }
            // 仅消费 Quiescent:durable 游标必须已到静止。
            if current.cleanup_cursor < CleanupStep::Quiesced {
                return Err(ProviderAdapterError::provider_unavailable(format!(
                    "provider_resources_cleanup_not_quiescent: {}",
                    current.resource_id
                )));
            }
            match current.kind {
                OwnedResourceKind::Directory => {
                    let Some(path) = current.canonical_private_path.as_deref() else {
                        return Err(ProviderAdapterError::provider_unavailable(format!(
                            "provider_resources_directory_record_missing_path: {}",
                            current.resource_id
                        )));
                    };
                    let registered = PathBuf::from(path);
                    match registered.canonicalize() {
                        Ok(actual) if actual != registered => {
                            // 路径漂移停等:登记路径已不指向原 canonical 资源。
                            return Err(ProviderAdapterError::provider_unavailable(format!(
                                "provider_resources_cleanup_path_drift: {} -> {};漂移目标不受登记清理约束,停等",
                                registered.display(),
                                actual.display()
                            )));
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            // 已不在(此前清理崩溃于游标推进后):直接解除登记。
                            self.deregister_quietly(current);
                            continue;
                        }
                        Err(error) => {
                            return Err(ProviderAdapterError::provider_unavailable(format!(
                                "provider_resources_cleanup_path_stat_failed: {} ({error})",
                                registered.display()
                            )));
                        }
                    }
                    // 共享/owner HOME 面(私有根外路径)永不清理——预留期
                    // 已挡,这里再挡一次(defense in depth)。
                    // F4(审查修复轮):根自身与根外路径一样永不清理——
                    // remove_dir_all(根)会连带其它资源的私有目录。
                    if registered == self.private_root
                        || !registered.starts_with(&self.private_root)
                    {
                        return Err(ProviderAdapterError::provider_unavailable(format!(
                            "provider_resources_cleanup_outside_private_root: {} 必须是 {} 的严格子路径(根自身/根外永不清理)",
                            registered.display(),
                            self.private_root.display()
                        )));
                    }
                    std::fs::remove_dir_all(&registered).map_err(|error| {
                        ProviderAdapterError::provider_unavailable(format!(
                            "provider_resources_cleanup_remove_failed: {} ({error})",
                            registered.display()
                        ))
                    })?;
                    self.advance_and_deregister(current, CleanupStep::Removed)?;
                }
                OwnedResourceKind::Container | OwnedResourceKind::Network => {
                    let Some(observer) = self.engine_observer.as_ref() else {
                        return Err(ProviderAdapterError::provider_unavailable(
                            "provider_resources_cleanup_observer_missing",
                        ));
                    };
                    let Some(started) = current.started_identity.as_ref() else {
                        // 未确认且引擎侧无副作用(quiesce 已核对)——解除登记。
                        self.deregister_quietly(current);
                        continue;
                    };
                    let Some(exact_id) = engine_exact_id(started) else {
                        return Err(ProviderAdapterError::provider_unavailable(format!(
                            "provider_resources_record_identity_kind_mismatch: {}",
                            current.resource_id
                        )));
                    };
                    // Podman 面只清精确登记 ID+标签均匹配的本轮资源。
                    let removed =
                        observer.remove_exact(current.kind, exact_id, &current.reserved_labels)?;
                    if removed {
                        self.advance_and_deregister(current, CleanupStep::Removed)?;
                    } else if observer.observe_running(current.kind, exact_id)? {
                        // 精确 ID 在但标签不匹配(共享):永不清理,停等。
                        return Err(ProviderAdapterError::provider_unavailable(format!(
                            "provider_resources_cleanup_shared_resource_not_owned: exact_id={exact_id} 标签不属本轮,共享资源永不清理"
                        )));
                    } else {
                        // 已不在:幂等解除。
                        self.deregister_quietly(current);
                    }
                }
                OwnedResourceKind::Process => {
                    // 进程面静止已在 quiesce 证明;清理=游标推进+解除登记。
                    self.advance_and_deregister(current, CleanupStep::Removed)?;
                }
            }
        }
        Ok(())
    }

    fn advance_and_deregister(
        &self,
        record: &ProviderResourceRecord,
        step: CleanupStep,
    ) -> Result<(), ProviderAdapterError> {
        self.store
            .advance_cleanup_cursor(&self.key, &record.resource_id, step)
            .map_err(store_error)?;
        self.deregister_quietly(record);
        Ok(())
    }

    /// 解除登记(best-effort 写;失败时记录保持原状,成为恢复材料)。
    fn deregister_quietly(&self, record: &ProviderResourceRecord) {
        let _ = self.store.deregister(&self.key, &record.resource_id);
    }

    /// 资源启动包装器(进程面):持久预留 → spawn → 真实 PID-start 身份
    /// 确认,闭合预留与真实身份间的崩溃口。预留失败不 spawn;spawn 失败
    /// 解除登记;确认写失败停止子进程(整组)并进入补偿。
    #[cfg_attr(not(test), allow(dead_code))] // 进程面包装器的合同载体:预留→spawn→PID-start 确认语义的参考实现。B1 生产接线经 `TrustedLaunchOwnership` + adapter 自带 spawn(四家真实 provider 的 spawn 点),本函数在非测试构建无调用方;保留以钉住"预留先于 spawn / spawn 失败零残留 / 确认失败整组停止"合同(drop_tests),不得误判存在生产调用或把它接回 adapter。
    pub(crate) async fn start_owned_process(
        &self,
        spec: OwnedResourceSpec,
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        cancel: CancellationToken,
    ) -> Result<(ResourceReservation, ManagedProcess), ProviderAdapterError> {
        // ⚠4(审查修复轮):fail-fast——引擎资源(容器/网络/目录)不走进程
        // 包装器,先拒绝再谈预留,避免登记 kind 与启动形态错位。
        if spec.kind != OwnedResourceKind::Process {
            return Err(ProviderAdapterError::provider_unavailable(format!(
                "provider_resources_start_owned_process_kind_mismatch: {:?} 不是进程资源",
                spec.kind
            )));
        }
        // ① 持久预留先行:登记写失败必须发生在任何 spawn 副作用之前。
        let reservation = self.reserve_owned_resource(&spec)?;
        // ② spawn:确认失败时进程未产生,解除登记不留孤儿登记。
        let mut managed =
            match ProcessManager::spawn(command, args, working_dir, env_vars, cancel).await {
                Ok(managed) => managed,
                Err(error) => {
                    self.deregister_quietly(&self.reservation_record(&reservation));
                    return Err(error);
                }
            };
        // ③ 冻结真实 PID-start 身份(spawn 时组长 PID + /proc starttime)。
        let Some(pid) = managed.child.spawn_leader_pid() else {
            managed.child.terminate().await;
            self.deregister_quietly(&self.reservation_record(&reservation));
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_resources_start_identity_unavailable: spawn 未暴露组长 PID",
            ));
        };
        let Some(start_time_ticks) = process_start_time_ticks(pid) else {
            // 进程瞬逝:身份不可冻结 → 停止并补偿,不遗留无归属 writer。
            managed.child.terminate().await;
            self.deregister_quietly(&self.reservation_record(&reservation));
            return Err(ProviderAdapterError::provider_unavailable(format!(
                "provider_resources_start_identity_unavailable: pid={pid} starttime 不可读(进程已退出?)"
            )));
        };
        // ④ 确认固化;确认写失败 → 停止已启动资源(整组真实停止)并补偿。
        match self.confirm_started_resource(
            &reservation,
            StartedResourceIdentity::process(pid, start_time_ticks),
        ) {
            Ok(()) => Ok((reservation, managed)),
            Err(error) => {
                managed.child.terminate().await;
                self.deregister_quietly(&self.reservation_record(&reservation));
                Err(error)
            }
        }
    }

    /// 由预留句柄还原登记事实(补偿解除用;与 reserve 写入的记录一致)。
    #[cfg_attr(not(test), allow(dead_code))] // start_owned_process 补偿解除用,随其接线转正
    fn reservation_record(&self, reservation: &ResourceReservation) -> ProviderResourceRecord {
        ProviderResourceRecord {
            key: reservation.key.clone(),
            resource_id: reservation.resource_id.to_string(),
            kind: reservation.kind,
            launch_intent: reservation.launch_intent.clone(),
            reserved_labels: reservation.reserved_labels.clone(),
            started_identity: None,
            canonical_private_path: reservation
                .canonical_private_path
                .as_ref()
                .map(|path| path.display().to_string()),
            cleanup_cursor: CleanupStep::Reserved,
            reserved_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// 仅测试:暴露 store 以装配 failpoint/直接登记 fixture。
    #[cfg(test)]
    pub(crate) fn resource_store(&self) -> &ProviderResourceStore {
        &self.store
    }

    /// 仅测试:直接以构造事实登记记录(fake/恢复场景装配)。
    #[cfg(test)]
    #[allow(dead_code)] // 装配口暂无测试引用,保留 fake/恢复装配面
    pub(crate) fn insert_record_for_test(
        &self,
        record: ProviderResourceRecord,
    ) -> Result<(), ProviderAdapterError> {
        self.store.reserve(&record).map_err(store_error)
    }
}

// ---------------------------------------------------------------------------
// B1-1(波 2):受信 launch 归属载体——engine 已开真实 scope → raw start 点
// ---------------------------------------------------------------------------

/// 本次 launch 的受信资源归属载体(B1-1)。
///
/// **唯一铸造来源**:engine 三处真实 scope——Coder `begin_coder_delivery_
/// scope`、编码 Reviewer `reviewer-window-{attempt}` 主 scope、规划 Reviewer
/// `plan-review-{window}` 主 scope(均经 `ProviderRunResourceScope` 打开,
/// 归属三元组 scope_id/run_id/boot 与开窗/收口消费者完全相同)。字段
/// private、crate 内构造,caller/模型不可伪造第二套归属;不合成 UUID run
/// 身份(F22 env-only 合成层已随 A1 消失,本载体不恢复)。
///
/// **生命周期**(B1 raw start 4 点的默认合同,见
/// `StreamingProviderAdapter::start_owned`):预留(durable,失败零 spawn)
/// → spawn → `backfill_spawned_identity` 回填 PID+starttime → 会话真实
/// 收口(进程组静止)后注销——**注销不在协议 Completed**。预留必须紧邻
/// spawn(版本探测等可取消 await 之前零 durable 副作用);launch future
/// 被外层 drop 时由 `streaming_provider::LaunchReservationGuard` 兜底
/// 收口(未确认预留直接解除)。
///
/// **资源 ID**:每次 launch 唯一(`provider-process-{pid}-{seq}-{nanos}`,
/// 进程内单调+时钟),禁止恒定 `provider-process` 使同 run 多次调用/重试
/// 共享预留;scope 的稳定归属仍由 aria.* 标签表达。
///
/// **worktree 标签**:`aria.worktree` 由 scope 在预留时注入(开 scope 后、
/// reserve 前挂 `with_worktree_key`);Reviewer 窗口的兄弟枚举
/// (`reviewer_quiesce_targets`)据此找到本 launch 的进程登记。
///
/// v1 登记范围:coder delivery + 编码/规划 Reviewer 的 streaming launch;
/// 单仓同步路(`CliProviderAdapter`/`run_command_capture_with_stream`)与
/// Kimi ACP terminal 直接 `Command::spawn` 面明确不在 v1(后续批次接线,
/// 未登记面按既有未知 writer 停等边界承担)。
#[derive(Clone)]
pub struct TrustedLaunchOwnership {
    scope: Arc<ProviderRunResourceScope>,
    resource_id: String,
    launch_intent: String,
}

impl std::fmt::Debug for TrustedLaunchOwnership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedLaunchOwnership")
            .field("key", self.scope.key())
            .field("resource_id", &self.resource_id)
            .field("launch_intent", &self.launch_intent)
            .finish()
    }
}

impl TrustedLaunchOwnership {
    /// 从 engine 已开的真实 scope 铸造本次 launch 的归属载体。同一 scope
    /// 可铸造多次(技术重试/修复轮),每次 `resource_id` 唯一。
    pub(crate) fn mint(scope: Arc<ProviderRunResourceScope>, launch_intent: &str) -> Self {
        Self {
            scope,
            resource_id: next_launch_resource_id(),
            launch_intent: launch_intent.to_string(),
        }
    }

    /// 归属 scope 主键(与开窗/收口消费者同源的三元组)。
    #[cfg_attr(not(test), allow(dead_code))] // 测试断言消费;生产对账消费者随收口接线
    pub(crate) fn scope_key(&self) -> &ResourceScopeKey {
        self.scope.key()
    }

    /// 本次 launch 的唯一资源 ID。
    #[cfg_attr(not(test), allow(dead_code))] // 同上
    pub(crate) fn resource_id(&self) -> &str {
        &self.resource_id
    }

    /// 持久预留(先于一切 spawn 副作用;失败=调用方不得 spawn)。
    /// 同 key 同登记幂等;kind=Process、业务标签 `kind=provider_process`,
    /// aria.* 归属标签(scope/run/worktree/window)由 scope 注入。
    pub(crate) fn reserve(&self) -> Result<ResourceReservation, ProviderAdapterError> {
        self.scope.reserve_owned_resource(
            &OwnedResourceSpec::new(
                &self.resource_id,
                OwnedResourceKind::Process,
                &self.launch_intent,
            )
            .with_label("kind", "provider_process"),
        )
    }

    /// spawn 点回填真实 PID+starttime(读 `/proc/<pid>/stat` starttime,
    /// PID 复用防护)。预留事实必须属于本载体 scope。真实 adapter 的
    /// B1 接线片在 `ProcessManager::spawn` 成功后调用;默认包装面
    /// (未回填)按无身份收口。
    pub(crate) fn backfill_spawned_identity(
        &self,
        reservation: &ResourceReservation,
        pid: u32,
    ) -> Result<(), ProviderAdapterError> {
        if reservation.key() != self.scope.key() {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_resources_backfill_reservation_scope_mismatch: 预留不属于本载体 scope",
            ));
        }
        if reservation.resource_id() != self.resource_id {
            return Err(ProviderAdapterError::provider_unavailable(
                "provider_resources_backfill_reservation_id_mismatch: 预留不是本 launch 的登记",
            ));
        }
        let Some(start_time_ticks) = process_start_time_ticks(pid) else {
            return Err(ProviderAdapterError::provider_unavailable(format!(
                "provider_resources_backfill_identity_unavailable: pid={pid} starttime 不可读(进程已退出?)"
            )));
        };
        self.scope.confirm_started_resource(
            reservation,
            StartedResourceIdentity::process(pid, start_time_ticks),
        )
    }

    /// 会话尾部/错误路径的收口注销:有登记身份 → killpg(仅身份核对通过)
    /// + 有界观察组长真实消失 + 组残留检查,证明静止后游标推进并注销;
    ///   未回填身份(默认面错误路径,adapter 自身已 kill+wait)直接注销。
    ///   静止不可证时保留 durable 记录停等(scope 级 quiesce/恢复驱动
    ///   接管),不注销。幂等:登记项不存在即成功。
    pub(crate) async fn release_after_session_tail(&self) {
        if let Err(error) = self
            .scope
            .release_process_registration(&self.resource_id)
            .await
        {
            // 收口注销只做 best-effort:静止不可证/写失败时保留 durable
            // 记录,scope 级 quiesce 与恢复驱动按停等接管(不掩盖、不吞错,
            // 记录告警日志供排障)。
            tracing::warn!(
                resource = %self.resource_id,
                key = ?self.scope.key(),
                error = %error,
                "provider_launch_ownership_release_deferred"
            );
        }
    }
}

/// 进程内单调+时钟的 launch 唯一资源 ID(同 scope 多次 launch 不共享预留)。
fn next_launch_resource_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAUNCH_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = LAUNCH_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("provider-process-{}-{seq}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{Child, Command};
    use std::time::Duration;

    use super::*;
    use crate::cross_cutting::provider_execution::{
        AuthorToolPolicy, FrozenProviderExecution, ProviderExecutionProfile, TrustedProviderRole,
        TrustedRunIdentity,
    };
    use crate::product::provider_resource_store::ProviderResourceStore as Store;
    use crate::protocol::contracts::{AdapterRole, ProviderRunRecord};

    // ---------- fixtures ----------

    fn t5_dir() -> tempfile::TempDir {
        let root = Path::new("/tmp/aria-bug45/t5");
        fs::create_dir_all(root).expect("create /tmp/aria-bug45/t5");
        tempfile::tempdir_in(root).expect("t5 fixture dir")
    }

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

    fn frozen_contract(
        duty: TrustedProviderRole,
        run_id: &str,
        scope_id: &str,
        window: Option<String>,
    ) -> FrozenProviderExecution {
        let carrier = match duty {
            TrustedProviderRole::Author | TrustedProviderRole::Orchestrator => {
                AdapterRole::Orchestrator
            }
            TrustedProviderRole::Splitter => AdapterRole::WorkItemSplitter,
            TrustedProviderRole::Handoff => AdapterRole::Handoff,
            TrustedProviderRole::Coder | TrustedProviderRole::Reviewer => AdapterRole::Executor,
        };
        let identity = TrustedRunIdentity::from_durable_run(
            &durable_run_record(run_id, carrier),
            "ws_session_0001",
            duty,
        );
        let profile = match duty {
            TrustedProviderRole::Reviewer => ProviderExecutionProfile::ReviewerVerify,
            _ => ProviderExecutionProfile::HostCoding,
        };
        let author_policy =
            matches!(duty, TrustedProviderRole::Author).then(AuthorToolPolicy::author_v1);
        crate::cross_cutting::provider_execution::freeze_provider_execution(
            &identity,
            profile,
            author_policy,
            scope_id.to_string(),
            window,
        )
        .expect("freeze contract fixture")
    }

    struct ScopeEnv {
        _base: tempfile::TempDir,
        store_root: PathBuf,
        private_root: PathBuf,
    }

    fn scope_env() -> ScopeEnv {
        let base = t5_dir();
        let store_root = base.path().join("resource-store");
        let private_root = base.path().join("private-root");
        fs::create_dir_all(&private_root).expect("create private root");
        ScopeEnv {
            _base: base,
            store_root,
            private_root,
        }
    }

    fn open_scope(
        env: &ScopeEnv,
        execution: &FrozenProviderExecution,
        boot: &str,
        observer: Option<Arc<dyn OwnedEngineObserver>>,
    ) -> ProviderRunResourceScope {
        ProviderRunResourceScope::open(
            execution,
            boot,
            &env.private_root,
            &env.store_root,
            observer,
        )
        .expect("open scope")
    }

    fn unix_path_env() -> BTreeMap<String, String> {
        BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())])
    }

    /// T14:aria.worktree 协调标签由 scope 注入(硬约束2——T19
    /// quiesce_targets 据此枚举兄弟 Coder;调用方伪造 aria.* 的拒绝
    /// 由既有钉测覆盖)。
    #[test]
    fn with_worktree_key_injects_coordination_label() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_wt_1",
            "code-delivery-coding_attempt_0001",
            None,
        );
        let scope =
            open_scope(&env, &execution, "boot_wt_1", None).with_worktree_key("/wt/tree-key-a");
        let scratch = env.private_root.join("scratch");
        fs::create_dir_all(&scratch).expect("create scratch");
        let reservation = scope
            .reserve_owned_resource(
                &OwnedResourceSpec::new(
                    "coder-run-scratch",
                    OwnedResourceKind::Directory,
                    "code_delivery run-owned scratch directory",
                )
                .with_private_path(scratch),
            )
            .expect("reserve scratch");
        let labels = reservation.reserved_labels();
        assert_eq!(
            labels
                .get(PROVIDER_RESOURCE_LABEL_WORKTREE)
                .map(String::as_str),
            Some("/wt/tree-key-a"),
            "aria.worktree 协调标签必须由 scope 注入(T19 quiesce_targets 枚举依据)"
        );
        assert_eq!(
            labels
                .get(PROVIDER_RESOURCE_LABEL_SCOPE)
                .map(String::as_str),
            Some("code-delivery-coding_attempt_0001")
        );
        assert_eq!(
            labels.get(PROVIDER_LABEL_RUN).map(String::as_str),
            Some("run_wt_1")
        );
    }

    /// 引擎 fake:内存容器/网络状态,严格精确 ID+标签匹配。
    struct FakeEngine {
        state: Mutex<BTreeMap<String, EngineEntry>>,
    }

    struct EngineEntry {
        kind: OwnedResourceKind,
        labels: BTreeMap<String, String>,
        running: bool,
        /// false = 「stop 返回成功但容器仍 running」的假静止 fixture
        /// (killpg 成功≠静止)。
        stop_works: bool,
    }

    impl FakeEngine {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(BTreeMap::new()),
            })
        }

        fn add(
            &self,
            kind: OwnedResourceKind,
            exact_id: &str,
            labels: BTreeMap<String, String>,
            running: bool,
        ) {
            self.add_with_stop(kind, exact_id, labels, running, true);
        }

        /// 停止语义可控的登记(stop 成功但资源仍 running 的假静止场景)。
        fn add_with_stop(
            &self,
            kind: OwnedResourceKind,
            exact_id: &str,
            labels: BTreeMap<String, String>,
            running: bool,
            stop_works: bool,
        ) {
            self.state.lock().insert(
                exact_id.to_string(),
                EngineEntry {
                    kind,
                    labels,
                    running,
                    stop_works,
                },
            );
        }

        fn is_running(&self, exact_id: &str) -> bool {
            self.state
                .lock()
                .get(exact_id)
                .is_some_and(|entry| entry.running)
        }

        fn exists(&self, exact_id: &str) -> bool {
            self.state.lock().contains_key(exact_id)
        }
    }

    fn labels_match(entry: &EngineEntry, labels: &BTreeMap<String, String>) -> bool {
        labels
            .iter()
            .all(|(key, value)| entry.labels.get(key).is_some_and(|found| found == value))
    }

    impl OwnedEngineObserver for FakeEngine {
        fn observe_running(
            &self,
            kind: OwnedResourceKind,
            exact_id: &str,
        ) -> Result<bool, ProviderAdapterError> {
            Ok(self
                .state
                .lock()
                .get(exact_id)
                .is_some_and(|entry| entry.kind == kind && entry.running))
        }

        fn find_by_reserved_labels(
            &self,
            kind: OwnedResourceKind,
            labels: &BTreeMap<String, String>,
        ) -> Result<Vec<String>, ProviderAdapterError> {
            let state = self.state.lock();
            Ok(state
                .iter()
                .filter(|(_, entry)| {
                    entry.kind == kind && entry.running && labels_match(entry, labels)
                })
                .map(|(id, _)| id.clone())
                .collect())
        }

        fn stop_exact(
            &self,
            kind: OwnedResourceKind,
            exact_id: &str,
            labels: &BTreeMap<String, String>,
        ) -> Result<bool, ProviderAdapterError> {
            let mut state = self.state.lock();
            match state.get_mut(exact_id) {
                Some(entry) if entry.kind == kind && labels_match(entry, labels) => {
                    if entry.stop_works {
                        entry.running = false;
                    }
                    Ok(true)
                }
                _ => Ok(false),
            }
        }

        fn remove_exact(
            &self,
            kind: OwnedResourceKind,
            exact_id: &str,
            labels: &BTreeMap<String, String>,
        ) -> Result<bool, ProviderAdapterError> {
            let mut state = self.state.lock();
            match state.get(exact_id) {
                Some(entry) if entry.kind == kind && labels_match(entry, labels) => {
                    state.remove(exact_id);
                    Ok(true)
                }
                _ => Ok(false),
            }
        }
    }

    fn dir_record(
        key: &ResourceScopeKey,
        resource_id: &str,
        canonical_path: &Path,
        window: Option<&str>,
    ) -> ProviderResourceRecord {
        let mut labels = BTreeMap::new();
        labels.insert(
            PROVIDER_RESOURCE_LABEL_SCOPE.to_string(),
            key.scope_id.clone(),
        );
        labels.insert(PROVIDER_LABEL_RUN.to_string(), key.run_id.clone());
        if let Some(window) = window {
            labels.insert(
                PROVIDER_RESOURCE_LABEL_WINDOW.to_string(),
                window.to_string(),
            );
        }
        ProviderResourceRecord {
            key: key.clone(),
            resource_id: resource_id.to_string(),
            kind: OwnedResourceKind::Directory,
            launch_intent: "private scratch dir".to_string(),
            reserved_labels: labels,
            started_identity: Some(StartedIdentity::Directory {
                canonical_path: canonical_path.display().to_string(),
            }),
            canonical_private_path: Some(canonical_path.display().to_string()),
            cleanup_cursor: CleanupStep::Reserved,
            reserved_at: "2026-10-17T00:00:00Z".to_string(),
            updated_at: "2026-10-17T00:00:00Z".to_string(),
        }
    }

    async fn wait_for_path(path: &Path) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fixture path");
    }

    fn read_pid_file(path: &Path) -> i32 {
        fs::read_to_string(path)
            .expect("read pid")
            .trim()
            .parse()
            .expect("parse pid")
    }

    fn process_alive(pid: i32) -> bool {
        fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| {
                stat.rsplit_once(") ")
                    .and_then(|(_, tail)| tail.chars().next())
            })
            .is_some_and(|state| state != 'Z')
    }

    async fn wait_process_gone(pid: i32) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while process_alive(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("process must be really gone (not just kill-return)");
    }

    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).expect("write fixture");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
    }

    fn spawn_sleep_300() -> Child {
        Command::new("sleep")
            .arg("300")
            .spawn()
            .expect("spawn sleep")
    }

    // ---------- 红测①:登记写失败时 spawn 计数 0 ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn reserve_write_failure_leaves_spawn_count_zero() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        // 破坏 store:root 路径上放一个普通文件 → 任何登记写都失败。
        fs::write(&env.store_root, "not a directory").expect("block store root");
        let scope = open_scope(&env, &execution, "boot_a", None);

        let fixture_dir = env.private_root.canonicalize().unwrap();
        let marker = fixture_dir.join("spawn-marker");
        let spawner = fixture_dir.join("counting-spawner");
        write_executable(
            &spawner,
            &format!("#!/bin/sh\ntouch '{}'\nsleep 60\n", marker.display()),
        );

        let spec = OwnedResourceSpec::new(
            "res_proc_0001",
            OwnedResourceKind::Process,
            "counting-spawner",
        );
        let error = scope
            .start_owned_process(
                spec,
                &spawner.display().to_string(),
                &[],
                &fixture_dir,
                &unix_path_env(),
                CancellationToken::new(),
            )
            .await
            .expect_err("登记写失败必须先于 spawn 失败");

        assert!(
            error.details.contains("provider_resources"),
            "错误必须带稳定前缀: {}",
            error.details
        );
        assert!(
            !marker.exists(),
            "登记写失败时 spawn 计数必须为 0(fixture 未被执行)"
        );
    }

    // ---------- 红测②:预留后崩溃可补偿 ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn reserved_directory_after_crash_is_compensable() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let boot_a = open_scope(&env, &execution, "boot_a", None);
        let private_dir = env.private_root.join("scratch");
        fs::create_dir_all(&private_dir).expect("create private dir");
        let canonical = private_dir.canonicalize().unwrap();

        let reservation = boot_a
            .reserve_owned_resource(
                &OwnedResourceSpec::new(
                    "res_dir_0001",
                    OwnedResourceKind::Directory,
                    "private scratch",
                )
                .with_private_path(canonical.clone()),
            )
            .expect("reserve before crash");
        assert_eq!(reservation.key().boot_identity, "boot_a");
        // 模拟崩溃:直接丢弃 scope(不 quiesce、不 cleanup)。
        drop(boot_a);

        // 新 boot 枚举 stale 登记并接管补偿。
        let store = Store::new(&env.store_root);
        let keys = store.scope_keys().expect("enumerate scopes");
        assert_eq!(keys.len(), 1, "崩溃后 durable 登记必须可枚举");

        let recovery = ProviderRunResourceScope::open_for_recovery(
            &keys[0],
            None,
            &env.private_root,
            &env.store_root,
            None,
        )
        .expect("open recovery scope");

        match recovery.quiesce_scope().await.expect("quiesce stale scope") {
            QuiesceOutcome::Quiescent(quiescent) => {
                assert_eq!(quiescent.manifest().len(), 1);
                recovery
                    .cleanup_owned_scope(&quiescent)
                    .expect("补偿清理完成");
            }
            QuiesceOutcome::RecoveryRequired(unknown) => {
                panic!("已预留目录崩溃后必须可补偿: {unknown:?}");
            }
        }
        assert!(!canonical.exists(), "私有目录在补偿后必须被清理");
        assert!(
            store.list_scope(&keys[0]).unwrap().is_empty(),
            "补偿完成后登记项必须清空"
        );
    }

    // ---------- 红测③:启动后确认写失败会停子进程 ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn confirm_write_failure_stops_spawned_child() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);
        scope.resource_store().set_confirm_write_failpoint(true);

        let fixture_dir = env.private_root.canonicalize().unwrap();
        let pid_file = fixture_dir.join("child.pid");
        let sleeper = fixture_dir.join("pid-sleep");
        write_executable(
            &sleeper,
            &format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nsleep 60\n",
                pid_file.display()
            ),
        );

        let spec = OwnedResourceSpec::new("res_proc_0002", OwnedResourceKind::Process, "pid-sleep");
        let error = scope
            .start_owned_process(
                spec,
                &sleeper.display().to_string(),
                &[],
                &fixture_dir,
                &unix_path_env(),
                CancellationToken::new(),
            )
            .await
            .expect_err("确认写失败必须失败关闭");
        assert!(
            error.details.contains("provider_resources"),
            "确认失败错误须带稳定前缀: {}",
            error.details
        );

        // 子进程必须被真实停止(以 /proc 观察为准,不是 kill 返回值);
        // failpoint 的 200ms 宽限保证 fixture 已落 pid 文件。
        wait_for_path(&pid_file).await;
        let pid = read_pid_file(&pid_file);
        wait_process_gone(pid).await;

        // 补偿完成:无无归属 writer,登记项清除。
        scope.resource_store().set_confirm_write_failpoint(false);
        let records = scope
            .resource_store()
            .list_scope(scope.key())
            .expect("list after compensation");
        assert!(
            records.is_empty(),
            "确认失败补偿后登记项必须清除: {records:?}"
        );
    }

    // ---------- 红测④:spawn 失败解除登记(补偿,不留孤儿登记) ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_failure_deregisters_reservation() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);
        let spec = OwnedResourceSpec::new(
            "res_proc_0003",
            OwnedResourceKind::Process,
            "missing-binary",
        );
        let _ = scope
            .start_owned_process(
                spec,
                "__aria_missing_binary__",
                &[],
                &env.private_root,
                &unix_path_env(),
                CancellationToken::new(),
            )
            .await
            .expect_err("spawn 失败必须返回错误");
        let records = scope
            .resource_store()
            .list_scope(scope.key())
            .expect("list after spawn failure");
        assert!(
            records.is_empty(),
            "spawn 失败补偿后不得遗留登记: {records:?}"
        );
    }

    // ---------- 红测⑤:PID 复用不杀新进程 ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn pid_reuse_never_kills_new_process() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);

        // 真实存活进程 X(未来的 PID 复用者)。
        let mut innocent = spawn_sleep_300();
        let innocent_pid = innocent.id();

        // 登记「确认身份」为 (pid=X, ticks=陈旧值):quiesce 时 X 的真实
        // starttime 与登记不符 → 原进程已消失、X 不是登记资源 → 不得杀。
        let reservation = scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "res_proc_0004",
                OwnedResourceKind::Process,
                "sleep 300",
            ))
            .expect("reserve");
        scope
            .confirm_started_resource(
                &reservation,
                StartedResourceIdentity::process(innocent_pid, 1),
            )
            .expect("confirm with stale identity");

        match scope.quiesce_scope().await.expect("quiesce") {
            QuiesceOutcome::Quiescent(_) => {}
            QuiesceOutcome::RecoveryRequired(unknown) => {
                panic!("PID 复用应证明原进程已静止: {unknown:?}");
            }
        }
        assert!(process_alive(innocent_pid as i32), "PID 复用后绝不杀新进程");
        innocent.kill().expect("test cleanup kill");
        let _ = innocent.wait();
    }

    // ---------- 红测⑥:killpg 成功但登记容器仍 running ≠ Quiescent ----------

    #[tokio::test]
    async fn killpg_ok_but_registered_container_running_is_not_quiescent() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let engine = FakeEngine::new();
        let scope = open_scope(&env, &execution, "boot_a", Some(engine.clone()));

        let reservation = scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "res_ctr_0001",
                OwnedResourceKind::Container,
                "image fixture",
            ))
            .expect("reserve container");
        scope
            .confirm_started_resource(&reservation, StartedResourceIdentity::container("ctr_0001"))
            .expect("confirm container");

        // 容器登记在册且仍在运行(fake stop_exact 返回 true=killpg 类成功,
        // 但 observe_running 仍为 true → 假静止)。
        engine.add_with_stop(
            OwnedResourceKind::Container,
            "ctr_0001",
            reservation.reserved_labels().clone(),
            true,
            false,
        );

        match scope.quiesce_scope().await.expect("quiesce attempt") {
            QuiesceOutcome::Quiescent(_) => {
                panic!("登记容器仍 running 时不得宣称 Quiescent")
            }
            QuiesceOutcome::RecoveryRequired(unknown) => {
                assert!(
                    unknown.reason.contains("registered_resource_running"),
                    "原因码须指向登记资源未静止: {}",
                    unknown.reason
                );
                assert!(
                    unknown
                        .manifest
                        .iter()
                        .any(|r| r.resource_id == "res_ctr_0001"),
                    "停等材料须含完整 manifest"
                );
            }
        }
        assert!(engine.is_running("ctr_0001"), "停等状态不动引擎资源");
    }

    // ---------- 红测⑦:未知 daemon/未确认引擎副作用 ≠ Quiescent ----------

    #[tokio::test]
    async fn unknown_engine_side_effect_is_not_quiescent() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let engine = FakeEngine::new();
        let scope = open_scope(&env, &execution, "boot_a", Some(engine.clone()));

        // 只预留未确认(create 崩溃窗口),但引擎侧已有带预留标签的实例。
        let spec = OwnedResourceSpec::new(
            "res_ctr_0002",
            OwnedResourceKind::Container,
            "image fixture",
        );
        let reservation = scope.reserve_owned_resource(&spec).expect("reserve");
        engine.add(
            OwnedResourceKind::Container,
            "ctr_unknown_daemon",
            reservation.reserved_labels().clone(),
            true,
        );

        match scope.quiesce_scope().await.expect("quiesce attempt") {
            QuiesceOutcome::Quiescent(_) => {
                panic!("未确认引擎副作用必须停等,不能把 create 失败当无副作用")
            }
            QuiesceOutcome::RecoveryRequired(unknown) => {
                assert!(
                    unknown.reason.contains("unconfirmed"),
                    "原因码须指向未确认副作用: {}",
                    unknown.reason
                );
            }
        }
        assert!(engine.is_running("ctr_unknown_daemon"));
    }

    // ---------- 红测⑧:未确认进程预留(崩溃窗口)停等 ----------

    #[tokio::test]
    async fn unconfirmed_process_reservation_waits_in_recovery() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);
        scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "res_proc_0005",
                OwnedResourceKind::Process,
                "sh -c long-task",
            ))
            .expect("reserve without confirm (crash window)");

        match scope.quiesce_scope().await.expect("quiesce attempt") {
            QuiesceOutcome::Quiescent(_) => {
                panic!("无真实身份的进程预留不能证明静止——可识别未知 writer 停等")
            }
            QuiesceOutcome::RecoveryRequired(unknown) => {
                assert!(
                    unknown.reason.contains("unconfirmed"),
                    "原因码须指向未确认进程: {}",
                    unknown.reason
                );
                assert!(
                    unknown
                        .manifest
                        .iter()
                        .any(|r| r.resource_id == "res_proc_0005" && r.started_identity.is_none()),
                    "停等材料须含未确认登记与 launch 意图"
                );
            }
        }
    }

    // ---------- 红测⑨:共享容器永不清理 ----------

    #[tokio::test]
    async fn shared_container_is_never_cleaned() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Reviewer,
            "run_0002",
            "scope_run_0002",
            Some("window_0001".to_string()),
        );
        let engine = FakeEngine::new();
        let scope = open_scope(&env, &execution, "boot_a", Some(engine.clone()));

        let reservation = scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "res_ctr_0003",
                OwnedResourceKind::Container,
                "shared base image",
            ))
            .expect("reserve");
        scope
            .confirm_started_resource(
                &reservation,
                StartedResourceIdentity::container("shared_ctr"),
            )
            .expect("confirm");

        // 共享容器:精确 ID 相同但标签属别人(无本轮 aria.* 标签)。
        engine.add(
            OwnedResourceKind::Container,
            "shared_ctr",
            BTreeMap::from([("owner".to_string(), "someone-else".to_string())]),
            true,
        );

        match scope.quiesce_scope().await.expect("quiesce attempt") {
            QuiesceOutcome::Quiescent(_) => {
                panic!("无法停止的共享容器仍 running → 不是 Quiescent")
            }
            QuiesceOutcome::RecoveryRequired(unknown) => {
                assert!(
                    unknown.reason.contains("not_owned") || unknown.reason.contains("shared"),
                    "原因码须指向非本轮归属资源: {}",
                    unknown.reason
                );
            }
        }
        // 共享容器原样保留。
        assert!(engine.exists("shared_ctr") && engine.is_running("shared_ctr"));
    }

    // ---------- 红测⑮:容器生命周期只清精确匹配的登记项 ----------

    #[tokio::test]
    async fn container_lifecycle_cleans_exact_match_only() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let engine = FakeEngine::new();
        let scope = open_scope(&env, &execution, "boot_a", Some(engine.clone()));

        let reservation = scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "res_ctr_0004",
                OwnedResourceKind::Container,
                "image fixture",
            ))
            .expect("reserve");
        scope
            .confirm_started_resource(
                &reservation,
                StartedResourceIdentity::container("ctr_owned"),
            )
            .expect("confirm");
        engine.add(
            OwnedResourceKind::Container,
            "ctr_owned",
            reservation.reserved_labels().clone(),
            true,
        );
        // 无关容器(ID/标签均不匹配)必须原样保留。
        engine.add(
            OwnedResourceKind::Container,
            "ctr_other",
            BTreeMap::from([("owner".to_string(), "elsewhere".to_string())]),
            true,
        );

        let quiescent = match scope.quiesce_scope().await.expect("quiesce") {
            QuiesceOutcome::Quiescent(quiescent) => quiescent,
            QuiesceOutcome::RecoveryRequired(unknown) => panic!("{unknown:?}"),
        };
        assert!(
            !quiescent.quiesced_at().is_empty(),
            "静止证明须携带时间戳(交接材料)"
        );
        assert!(!engine.is_running("ctr_owned"), "登记容器被 stop");
        assert!(engine.exists("ctr_owned"), "remove 留给 cleanup");

        scope.cleanup_owned_scope(&quiescent).expect("cleanup");
        assert!(!engine.exists("ctr_owned"), "精确 ID+标签匹配的容器被移除");
        assert!(
            engine.exists("ctr_other") && engine.is_running("ctr_other"),
            "无关容器永不清理"
        );
        assert!(
            scope
                .resource_store()
                .list_scope(scope.key())
                .unwrap()
                .is_empty()
        );
    }

    // ---------- 红测⑩:私有根外路径(owner HOME 面)永不清理 ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn cleanup_refuses_paths_outside_private_root() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);

        // 模拟 owner HOME(私有根外)被错误登记:直接以 store 形状落盘。
        let outside = t5_dir();
        let owner_home = outside.path().join("owner-home");
        fs::create_dir_all(&owner_home).expect("owner home");
        let record = dir_record(scope.key(), "res_dir_0002", &owner_home, None);
        scope.resource_store().reserve(&record).expect("insert");

        let quiescent = match scope.quiesce_scope().await.expect("quiesce") {
            QuiesceOutcome::Quiescent(quiescent) => quiescent,
            QuiesceOutcome::RecoveryRequired(unknown) => {
                panic!("目录登记无进程面,应静止: {unknown:?}")
            }
        };
        let error = scope
            .cleanup_owned_scope(&quiescent)
            .expect_err("私有根外路径必须拒绝清理");
        assert!(
            error.details.contains("outside_private_root"),
            "拒绝原因须指向私有根外: {}",
            error.details
        );
        assert!(owner_home.exists(), "owner HOME 类目录永不清理");
    }

    // ---------- 红测⑪:重复 cleanup 幂等 ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn repeated_cleanup_is_idempotent() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);

        let private_dir = env.private_root.join("scratch");
        fs::create_dir_all(&private_dir).expect("create");
        let canonical = private_dir.canonicalize().unwrap();
        let reservation = scope
            .reserve_owned_resource(
                &OwnedResourceSpec::new("res_dir_0003", OwnedResourceKind::Directory, "scratch")
                    .with_private_path(canonical.clone()),
            )
            .expect("reserve");
        scope
            .confirm_started_resource(&reservation, StartedResourceIdentity::directory(&canonical))
            .expect("confirm dir");

        let quiescent = match scope.quiesce_scope().await.expect("quiesce") {
            QuiesceOutcome::Quiescent(quiescent) => quiescent,
            QuiesceOutcome::RecoveryRequired(unknown) => panic!("{unknown:?}"),
        };
        scope.cleanup_owned_scope(&quiescent).expect("cleanup #1");
        assert!(!canonical.exists());
        scope
            .cleanup_owned_scope(&quiescent)
            .expect("cleanup #2 幂等");
        assert!(
            scope
                .resource_store()
                .list_scope(scope.key())
                .unwrap()
                .is_empty()
        );
    }

    // ---------- 红测⑫:路径漂移停等 ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn path_drift_stops_cleanup() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);

        let registered = env.private_root.join("drift-me");
        fs::create_dir_all(&registered).expect("create");
        let canonical = registered.canonicalize().unwrap();
        let reservation = scope
            .reserve_owned_resource(
                &OwnedResourceSpec::new("res_dir_0004", OwnedResourceKind::Directory, "drift")
                    .with_private_path(canonical.clone()),
            )
            .expect("reserve");
        scope
            .confirm_started_resource(&reservation, StartedResourceIdentity::directory(&canonical))
            .expect("confirm");

        let quiescent = match scope.quiesce_scope().await.expect("quiesce") {
            QuiesceOutcome::Quiescent(quiescent) => quiescent,
            QuiesceOutcome::RecoveryRequired(unknown) => panic!("{unknown:?}"),
        };

        // 漂移:登记路径被替换为指向别处的符号链接。
        let drift_target = env.private_root.join("drift-target");
        fs::create_dir_all(&drift_target).expect("create target");
        fs::remove_dir_all(&canonical).expect("remove original");
        std::os::unix::fs::symlink(&drift_target, &canonical).expect("symlink swap");

        let error = scope
            .cleanup_owned_scope(&quiescent)
            .expect_err("路径漂移必须停等");
        assert!(
            error.details.contains("path_drift"),
            "拒绝原因须指向路径漂移: {}",
            error.details
        );
        assert!(drift_target.exists(), "漂移目标绝不因登记清理被删除");
    }

    // ---------- 红测⑬:归属标签注入与伪造拒绝 ----------

    #[test]
    fn reserve_binds_ownership_labels_and_rejects_forged_aria_labels() {
        let env = scope_env();
        let coder = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &coder, "boot_a", None);
        let reservation = scope
            .reserve_owned_resource(
                &OwnedResourceSpec::new("res_0009", OwnedResourceKind::Container, "image fixture")
                    .with_label("team", "dev"),
            )
            .expect("reserve");

        let labels = reservation.reserved_labels();
        assert_eq!(labels.get("team").map(String::as_str), Some("dev"));
        assert_eq!(
            labels
                .get(PROVIDER_RESOURCE_LABEL_SCOPE)
                .map(String::as_str),
            Some("scope_run_0001")
        );
        assert_eq!(
            labels.get(PROVIDER_LABEL_RUN).map(String::as_str),
            Some("run_0001")
        );
        assert!(!labels.contains_key(PROVIDER_RESOURCE_LABEL_WINDOW));

        // 调用方伪造 aria.* 归属标签 → 拒绝。
        let forged = scope
            .reserve_owned_resource(
                &OwnedResourceSpec::new("res_0010", OwnedResourceKind::Container, "image fixture")
                    .with_label(PROVIDER_LABEL_RUN, "run_9999"),
            )
            .expect_err("伪造归属标签必须拒绝");
        assert!(
            forged.details.contains("label"),
            "错误须指向标签伪造: {}",
            forged.details
        );

        // Reviewer 冻结合同绑定窗口标签。
        let reviewer = frozen_contract(
            TrustedProviderRole::Reviewer,
            "run_0002",
            "scope_run_0002",
            Some("window_0001".to_string()),
        );
        let reviewer_scope = open_scope(&env, &reviewer, "boot_a", None);
        let reviewer_reservation = reviewer_scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "res_0011",
                OwnedResourceKind::Container,
                "image fixture",
            ))
            .expect("reserve");
        assert_eq!(reviewer_scope.key().run_id, "run_0002");
        assert_eq!(
            reviewer_reservation
                .reserved_labels()
                .get(PROVIDER_RESOURCE_LABEL_WINDOW)
                .map(String::as_str),
            Some("window_0001")
        );
    }

    // ---------- 红测⑭:真实进程组 spawn/静止/清理 smoke ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn owned_process_group_spawn_quiesce_cleanup_smoke() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);

        let fixture_dir = env.private_root.canonicalize().unwrap();
        let parent_pid_file = fixture_dir.join("smoke-parent.pid");
        let grandchild_pid_file = fixture_dir.join("smoke-grandchild.pid");
        let tree = fixture_dir.join("owned-tree");
        write_executable(
            &tree,
            &format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nsh -c 'printf \"%s\" \"$$\" > \"$1\"; sleep 300' tree-child '{}' &\nwait\n",
                parent_pid_file.display(),
                grandchild_pid_file.display(),
            ),
        );

        let (reservation, managed) = scope
            .start_owned_process(
                OwnedResourceSpec::new("res_proc_smoke", OwnedResourceKind::Process, "owned-tree"),
                &tree.display().to_string(),
                &[],
                &fixture_dir,
                &unix_path_env(),
                CancellationToken::new(),
            )
            .await
            .expect("owned spawn");

        // 真实身份已固化:PID + starttime。
        wait_for_path(&parent_pid_file).await;
        wait_for_path(&grandchild_pid_file).await;
        let records = scope
            .resource_store()
            .list_scope(scope.key())
            .expect("list records");
        let record = records
            .iter()
            .find(|r| r.resource_id == reservation.resource_id())
            .expect("durable record");
        match record.started_identity.as_ref().expect("confirmed") {
            StartedIdentity::Process {
                pid,
                start_time_ticks,
            } => {
                assert_eq!(*pid, read_pid_file(&parent_pid_file) as u32);
                assert!(*start_time_ticks > 0, "starttime 必须真实读取");
            }
            other => panic!("进程登记必须是 PID-start 身份: {other:?}"),
        }

        // 静止:killpg 只是第一步,以 /proc 真实观察孙进程消失为准。
        let grandchild_pid = read_pid_file(&grandchild_pid_file);
        let quiescent = match scope.quiesce_scope().await.expect("quiesce") {
            QuiesceOutcome::Quiescent(quiescent) => quiescent,
            QuiesceOutcome::RecoveryRequired(unknown) => panic!("{unknown:?}"),
        };
        assert!(
            !process_alive(grandchild_pid),
            "孙进程必须真实静止(killpg 成功不替代观察)"
        );
        let _ = managed;

        // 清理:登记项全部解除。
        scope.cleanup_owned_scope(&quiescent).expect("cleanup");
        assert!(
            scope
                .resource_store()
                .list_scope(scope.key())
                .unwrap()
                .is_empty()
        );
    }

    // ---------- 修复轮 F1:Removed 游标崩溃窗口后恢复不卡死 ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn removed_cursor_crash_window_recovery_completes() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);

        // 目录资源走完 quiesce 并把游标推到 Removed,模拟 cleanup 在
        // 「Removed 已写、登记未删」之间崩溃。
        let private_dir = env.private_root.join("removed-window");
        fs::create_dir_all(&private_dir).expect("create");
        let canonical = private_dir.canonicalize().unwrap();
        let reservation = scope
            .reserve_owned_resource(
                &OwnedResourceSpec::new("res_dir_f1", OwnedResourceKind::Directory, "scratch")
                    .with_private_path(canonical.clone()),
            )
            .expect("reserve");
        scope
            .confirm_started_resource(&reservation, StartedResourceIdentity::directory(&canonical))
            .expect("confirm");
        fs::remove_dir_all(&canonical).expect("物理清理先完成(窗口内崩溃点)");
        scope
            .resource_store()
            .advance_cleanup_cursor(scope.key(), "res_dir_f1", CleanupStep::Removed)
            .expect("durable 停在 Removed");
        drop(scope);

        // 新 boot 接管:quiesce 不得回推 Quiesced(regression 卡死),要
        // 得 Quiescent 且 cleanup 可收口。
        let keys = Store::new(&env.store_root).scope_keys().unwrap();
        let recovery = ProviderRunResourceScope::open_for_recovery(
            &keys[0],
            None,
            &env.private_root,
            &env.store_root,
            None,
        )
        .expect("open recovery");
        let quiescent = match recovery.quiesce_scope().await.expect("quiesce") {
            QuiesceOutcome::Quiescent(quiescent) => quiescent,
            QuiesceOutcome::RecoveryRequired(unknown) => panic!("F1: {unknown:?}"),
        };
        assert_eq!(
            quiescent.manifest()[0].cleanup_cursor,
            CleanupStep::Removed,
            "已前进游标原样保留"
        );
        recovery
            .cleanup_owned_scope(&quiescent)
            .expect("Removed 态可直接收口(不重复物理动作)");
        assert!(
            recovery
                .resource_store()
                .list_scope(recovery.key())
                .unwrap()
                .is_empty()
        );
    }

    // ---------- 修复轮 F2:组长已退出+组内残留 ≠ Quiescent ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn leader_exited_with_group_residual_is_not_quiescent() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);

        // fixture:组长打印 pid 后留下组内孙进程再自己退出(daemonize 形态)。
        let fixture_dir = env.private_root.canonicalize().unwrap();
        let leader_pid_file = fixture_dir.join("f2-leader.pid");
        let child_pid_file = fixture_dir.join("f2-child.pid");
        let daemon = fixture_dir.join("f2-daemon");
        write_executable(
            &daemon,
            &format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nsh -c 'printf \"%s\" \"$$\" > \"$1\"; sleep 300' f2-child '{}' &\nsleep 1\nexit 0\n",
                leader_pid_file.display(),
                child_pid_file.display(),
            ),
        );

        let reservation = scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "res_proc_f2",
                OwnedResourceKind::Process,
                "f2-daemon",
            ))
            .expect("reserve");
        let mut managed = crate::cross_cutting::process_manager::ProcessManager::spawn(
            &daemon.display().to_string(),
            &[],
            &fixture_dir,
            &unix_path_env(),
            CancellationToken::new(),
        )
        .await
        .expect("spawn daemon fixture");
        let leader_pid = managed.child.spawn_leader_pid().expect("leader pid");
        let ticks = process_start_time_ticks(leader_pid).expect("leader starttime");
        scope
            .confirm_started_resource(
                &reservation,
                StartedResourceIdentity::process(leader_pid, ticks),
            )
            .expect("confirm");

        // 等组长自然退出(组内孙进程仍存活)。
        wait_for_path(&child_pid_file).await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while process_alive(leader_pid as i32) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("leader exits by itself");
        let child_pid = read_pid_file(&child_pid_file);
        assert!(process_alive(child_pid), "组内孙进程仍存活");

        // quiesce:组长身份已不可核对 → 不能组杀;组内残留必须检出停等。
        match scope.quiesce_scope().await.expect("quiesce attempt") {
            QuiesceOutcome::Quiescent(_) => panic!("组长退出+组内残留时不得宣称 Quiescent"),
            QuiesceOutcome::RecoveryRequired(unknown) => {
                assert!(
                    unknown.reason.contains("process_group_residual"),
                    "原因码须指向组残留: {}",
                    unknown.reason
                );
            }
        }
        assert!(process_alive(child_pid), "停等不动未知 writer");

        // 人工清掉残留后,恢复重试可正常静止收口。
        unsafe { libc::kill(child_pid, libc::SIGKILL) };
        wait_process_gone(child_pid).await;
        match scope.quiesce_scope().await.expect("quiesce retry") {
            QuiesceOutcome::Quiescent(_) => {}
            QuiesceOutcome::RecoveryRequired(unknown) => panic!("残留清后应静止: {unknown:?}"),
        }
        let _ = &mut managed; // 持有至断言后,Drop 的组清理在此无害(组已空)
    }

    // ---------- 修复轮 2 P3:Removed+ 记录跳过检查(pgid 复用不假阳性) ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn removed_cursor_skips_checks_when_pgid_reused() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);

        // 真实存活进程,自成一组的组长(模拟旧 pgid 被无关新组复用)。
        let fixture_dir = env.private_root.canonicalize().unwrap();
        let sleeper = fixture_dir.join("p3-group-owner");
        write_executable(&sleeper, "#!/bin/sh\nsleep 300\n");
        let mut managed = crate::cross_cutting::process_manager::ProcessManager::spawn(
            &sleeper.display().to_string(),
            &[],
            &fixture_dir,
            &unix_path_env(),
            CancellationToken::new(),
        )
        .await
        .expect("spawn group owner");
        let leader_pid = managed.child.spawn_leader_pid().expect("leader pid");

        // 登记身份 ticks 故意陈旧(组长身份不匹配→kill 不会送达)+ 游标已
        // 到 Removed(物理清理已完成的恢复重入形态)。
        let reservation = scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "res_proc_p3",
                OwnedResourceKind::Process,
                "p3-group-owner",
            ))
            .expect("reserve");
        scope
            .confirm_started_resource(
                &reservation,
                StartedResourceIdentity::process(leader_pid, 1),
            )
            .expect("confirm with stale ticks");
        scope
            .resource_store()
            .advance_cleanup_cursor(scope.key(), "res_proc_p3", CleanupStep::Removed)
            .expect("advance to Removed");

        // 无跳过时:group_killed=false + has_members(leader_pid)=true →
        // 假阳性 process_group_residual 停等。修复后:跳过检查 → Quiescent。
        let quiescent = match scope.quiesce_scope().await.expect("quiesce") {
            QuiesceOutcome::Quiescent(quiescent) => quiescent,
            QuiesceOutcome::RecoveryRequired(unknown) => {
                panic!("Removed+ 记录不得因 pgid 复用假阳性停等: {unknown:?}")
            }
        };
        assert_eq!(
            quiescent.manifest()[0].cleanup_cursor,
            CleanupStep::Removed,
            "游标原样保留(F1)"
        );
        assert!(
            process_alive(leader_pid as i32),
            "无关新组不受影响(kill 从未送达)"
        );

        // 收口仍完整:cleanup 解除登记。
        scope.cleanup_owned_scope(&quiescent).expect("cleanup");
        assert!(
            scope
                .resource_store()
                .list_scope(scope.key())
                .unwrap()
                .is_empty()
        );
        let _ = &mut managed; // Drop 组清理在断言后无害(身份不符,killpg 未发)
    }

    // ---------- 修复轮 F3:confirm 重放不回退游标 ----------

    #[tokio::test]
    async fn confirm_replay_does_not_regress_cursor() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);
        let private_dir = env.private_root.join("f3-scratch");
        fs::create_dir_all(&private_dir).expect("create");
        let canonical = private_dir.canonicalize().unwrap();
        let reservation = scope
            .reserve_owned_resource(
                &OwnedResourceSpec::new("res_dir_f3", OwnedResourceKind::Directory, "scratch")
                    .with_private_path(canonical.clone()),
            )
            .expect("reserve");
        let identity = StartedResourceIdentity::directory(&canonical);
        scope
            .confirm_started_resource(&reservation, identity.clone())
            .expect("confirm #1");
        scope
            .resource_store()
            .advance_cleanup_cursor(scope.key(), "res_dir_f3", CleanupStep::Quiesced)
            .expect("advance to Quiesced");

        // 重放 confirm:身份一致,游标不得回退到 Confirmed。
        scope
            .confirm_started_resource(&reservation, identity)
            .expect("replay confirm ok");
        let record = scope
            .resource_store()
            .load_record(scope.key(), "res_dir_f3")
            .unwrap()
            .expect("record");
        assert_eq!(
            record.cleanup_cursor,
            CleanupStep::Quiesced,
            "confirm 重放不得回退前进游标"
        );
    }

    // ---------- 修复轮 F4:私有根自身不可登记/不可清理 ----------

    #[cfg(unix)]
    #[tokio::test]
    async fn private_root_itself_is_never_registered_or_cleaned() {
        let env = scope_env();
        let execution = frozen_contract(
            TrustedProviderRole::Coder,
            "run_0001",
            "scope_run_0001",
            None,
        );
        let scope = open_scope(&env, &execution, "boot_a", None);
        let root_canonical = env.private_root.canonicalize().unwrap();

        // reserve 拒绝根自身。
        let error = scope
            .reserve_owned_resource(
                &OwnedResourceSpec::new("res_dir_f4", OwnedResourceKind::Directory, "root itself")
                    .with_private_path(root_canonical.clone()),
            )
            .expect_err("根自身登记必须拒绝");
        assert!(
            error.details.contains("outside_root"),
            "拒绝原因须指向根边界: {}",
            error.details
        );

        // 漂移数据面:直接落一条路径=根的记录(模拟旧数据/越权写),cleanup
        // 也必须拒绝且不删根。
        let record = dir_record(scope.key(), "res_dir_f4b", &root_canonical, None);
        scope.resource_store().reserve(&record).expect("insert");
        let quiescent = match scope.quiesce_scope().await.expect("quiesce") {
            QuiesceOutcome::Quiescent(quiescent) => quiescent,
            QuiesceOutcome::RecoveryRequired(unknown) => panic!("{unknown:?}"),
        };
        let error = scope
            .cleanup_owned_scope(&quiescent)
            .expect_err("根自身清理必须拒绝");
        assert!(
            error.details.contains("outside_private_root"),
            "清理拒绝原因须指向根边界: {}",
            error.details
        );
        assert!(root_canonical.is_dir(), "私有根目录原样保留");
    }

    // -----------------------------------------------------------------
    // B1-1(波 2):受信 launch 归属载体与 raw start 默认生命周期
    // -----------------------------------------------------------------

    use crate::cross_cutting::streaming_provider::{
        ProviderCompletion, ProviderEvent, ProviderSession, StreamingProviderAdapter,
        StreamingProviderInput, start_owned_default,
    };

    /// 以 engine 真实键型(coder delivery 同款)开 scope 的 B1 测试夹具。
    fn b1_scope_env() -> (ScopeEnv, ProviderRunResourceScope, String) {
        let env = scope_env();
        let key = ResourceScopeKey {
            scope_id: "code-delivery-attempt-b1".to_string(),
            run_id: "role-run-b1".to_string(),
            boot_identity: format!("aria-engine-{}", std::process::id()),
        };
        // tree_key 只按 canonical 路径哈希;私有根即可充当协调 worktree 锚。
        let worktree = env.private_root.canonicalize().expect("canonical worktree");
        let tree_key = crate::product::reviewer_window_store::tree_key_of(&worktree);
        let scope = ProviderRunResourceScope::open_for_recovery(
            &key,
            None,
            &env.private_root,
            &env.store_root,
            None,
        )
        .expect("open b1 scope")
        .with_worktree_key(&tree_key);
        (env, scope, tree_key)
    }

    /// 真实子进程 probe adapter:spawn `sleep 30` 后按 mode 行为
    /// (handshake_fail=kill 后返回 Err;completed_early=先发 Completed、
    /// 子进程真实退出后关闭通道;cancel_tail=cancel 后 kill+wait 再关闭)。
    struct B1SpawningAdapter {
        mode: &'static str,
        starts: std::sync::atomic::AtomicUsize,
        /// spawn 点冻结的真实探针子进程 pid。退出必须定向 kill 这一个
        /// pid——禁止按 comm 模糊扫描 `/proc/self/task/*/children`:并行
        /// 测试里别的用例的 sleep/terminal 子进程同样是本进程的直接
        /// child,模糊匹配会把它们误杀(现场:terminal deadline 用例被
        /// 本 helper SIGKILL)。
        probe_pid: std::sync::atomic::AtomicU32,
    }

    impl B1SpawningAdapter {
        fn new(mode: &'static str) -> Self {
            Self {
                mode,
                starts: std::sync::atomic::AtomicUsize::new(0),
                probe_pid: std::sync::atomic::AtomicU32::new(0),
            }
        }

        fn probe_pid(&self) -> u32 {
            self.probe_pid.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for B1SpawningAdapter {
        async fn start(
            &self,
            input: StreamingProviderInput,
            cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            use std::sync::atomic::Ordering;
            self.starts.fetch_add(1, Ordering::SeqCst);
            let mut managed = ProcessManager::spawn(
                "sleep",
                &["30"],
                &input.working_dir,
                &BTreeMap::new(),
                cancel.clone(),
            )
            .await
            .expect("spawn sleep probe");
            if let Some(pid) = managed.child.spawn_leader_pid() {
                self.probe_pid.store(pid, Ordering::SeqCst);
            }
            let (event_tx, event_rx) = tokio::sync::mpsc::channel(32);
            let (command_tx, _command_rx) = tokio::sync::mpsc::channel(4);
            match self.mode {
                "handshake_fail" => {
                    // adapter 错误路径契约:kill+wait 子进程后返回 Err。
                    let _ = managed.child.start_kill();
                    let _ = managed.child.wait().await;
                    return Err(ProviderAdapterError::provider_unavailable(
                        "b1 probe handshake failed",
                    ));
                }
                "completed_early" => {
                    let _ = event_tx
                        .send(ProviderEvent::Completed(ProviderCompletion::plain(
                            "b1 probe done",
                            None,
                        )))
                        .await;
                    tokio::spawn(async move {
                        // 子进程真实退出(测试向其发信号)后才关闭通道。
                        let _ = managed.child.wait().await;
                        drop(event_tx);
                    });
                }
                "cancel_tail" => {
                    tokio::spawn(async move {
                        cancel.cancelled().await;
                        let _ = managed.child.start_kill();
                        let _ = managed.child.wait().await;
                        drop(event_tx);
                    });
                }
                _ => unreachable!("unknown probe mode"),
            }
            Ok(ProviderSession {
                native_session_id: None,
                events: event_rx,
                commands: command_tx,
            })
        }
    }

    fn b1_probe_input(env: &ScopeEnv) -> StreamingProviderInput {
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy: None,
            audit_sink: None,
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            role: crate::protocol::contracts::AdapterRole::Executor,
            prompt: "b1 probe".to_string(),
            working_dir: env.private_root.clone(),
            workspace_session_id: Some("ws-b1".to_string()),
            resume_provider_session_id: None,
            permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: 60,
        }
    }

    fn b1_store(env: &ScopeEnv) -> Store {
        Store::new(&env.store_root)
    }

    fn b1_process_records(env: &ScopeEnv, key: &ResourceScopeKey) -> Vec<ProviderResourceRecord> {
        b1_store(env)
            .list_scope(key)
            .expect("list scope")
            .into_iter()
            .filter(|record| matches!(record.kind, OwnedResourceKind::Process))
            .collect()
    }

    /// 同源归属:engine 真实 scope 铸造载体 → 默认生命周期登记 Process
    /// 记录(scope key/run_id/boot/worktree 标签与引擎一致),spawn 点
    /// 回填真实 PID+starttime。
    #[cfg(unix)]
    #[tokio::test]
    async fn b1_carrier_registers_process_with_engine_scope_identity() {
        let (env, scope, tree_key) = b1_scope_env();
        let adapter = B1SpawningAdapter::new("cancel_tail");
        let ownership = TrustedLaunchOwnership::mint(
            Arc::new(scope),
            "code_delivery provider process: b1 probe",
        );
        let cancel = CancellationToken::new();
        let session = start_owned_default(
            &adapter,
            b1_probe_input(&env),
            Some(ownership.clone()),
            cancel.clone(),
        )
        .await
        .expect("owned start");

        // Process 记录已出现,归属与引擎 scope 完全同源。
        let records = b1_process_records(&env, ownership.scope_key());
        assert_eq!(records.len(), 1, "恰一条 Process 登记: {records:?}");
        let record = &records[0];
        assert_eq!(record.key, *ownership.scope_key());
        assert_eq!(
            record
                .reserved_labels
                .get("aria.scope_id")
                .map(String::as_str),
            Some("code-delivery-attempt-b1")
        );
        assert_eq!(
            record
                .reserved_labels
                .get("aria.run_id")
                .map(String::as_str),
            Some("role-run-b1")
        );
        assert_eq!(
            record
                .reserved_labels
                .get("aria.worktree")
                .map(String::as_str),
            Some(tree_key.as_str())
        );
        assert!(
            record.resource_id.starts_with("provider-process-"),
            "资源 ID 唯一形态: {}",
            record.resource_id
        );
        // 默认生命周期在会话终结(取消收口)后注销;等通道终结。
        cancel.cancel();
        let mut events = session.events;
        while events.recv().await.is_some() {}
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            b1_process_records(&env, ownership.scope_key()).is_empty(),
            "会话收口后注销"
        );
    }

    /// 预留失败 → 零 spawn:store 根不可写时 `start_owned` 直接失败,
    /// adapter.start 从未被调用。
    #[cfg(unix)]
    #[tokio::test]
    async fn b1_reserve_failure_means_zero_spawn() {
        let base = t5_dir();
        // 私有根正常、store 根是一个文件 → reserve 落盘必败。
        let private_root = base.path().join("private-root");
        std::fs::create_dir_all(&private_root).expect("private root");
        let store_root = base.path().join("resource-store-file");
        std::fs::write(&store_root, b"not a dir").expect("store root file");
        let key = ResourceScopeKey {
            scope_id: "code-delivery-attempt-b1z".to_string(),
            run_id: "role-run-b1z".to_string(),
            boot_identity: format!("aria-engine-{}", std::process::id()),
        };
        let scope = ProviderRunResourceScope::open_for_recovery(
            &key,
            None,
            &private_root,
            &store_root,
            None,
        )
        .expect("open scope (store root lazily used)");
        let adapter = B1SpawningAdapter::new("cancel_tail");
        let ownership =
            TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
        let result = start_owned_default(
            &adapter,
            b1_probe_input(&ScopeEnv {
                _base: base,
                store_root: store_root.clone(),
                private_root: private_root.clone(),
            }),
            Some(ownership),
            CancellationToken::new(),
        )
        .await;
        let error = match result {
            Ok(_session) => panic!("reserve 失败必须上抛,不得启动"),
            Err(error) => error,
        };
        assert!(
            !error.details.contains("unimplemented"),
            "失败来自预留而非桩: {}",
            error.details
        );
        assert_eq!(
            adapter.starts.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "预留失败 → adapter.start 零调用(零 spawn)"
        );
    }

    /// 握手失败(adapter 错误路径 kill+wait 子进程后返回 Err)→ 收口后
    /// 注销:返回 Err 时登记已解除且子进程已死。
    #[cfg(unix)]
    #[tokio::test]
    async fn b1_handshake_failure_releases_after_quiesce() {
        let (env, scope, _tree_key) = b1_scope_env();
        let adapter = B1SpawningAdapter::new("handshake_fail");
        let ownership =
            TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
        let key = ownership.scope_key().clone();
        let error = match start_owned_default(
            &adapter,
            b1_probe_input(&env),
            Some(ownership),
            CancellationToken::new(),
        )
        .await
        {
            Ok(_session) => panic!("握手失败必须上抛"),
            Err(error) => error,
        };
        assert!(error.details.contains("handshake failed"));
        assert!(
            b1_process_records(&env, &key).is_empty(),
            "错误路径收口后注销"
        );
    }

    /// 协议 Completed 不提前注销:Completed 事件后、子进程真实退出前
    /// 登记仍在;通道终结(真实退出)后才注销。
    #[cfg(unix)]
    #[tokio::test]
    async fn b1_completed_event_does_not_deregister_early() {
        let (env, scope, _tree_key) = b1_scope_env();
        let adapter = B1SpawningAdapter::new("completed_early");
        let ownership =
            TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
        let key = ownership.scope_key().clone();
        let mut session = start_owned_default(
            &adapter,
            b1_probe_input(&env),
            Some(ownership),
            CancellationToken::new(),
        )
        .await
        .expect("owned start");

        // 1) 收到协议 Completed 事件。
        let mut saw_completed = false;
        while let Some(event) = session.events.recv().await {
            if let ProviderEvent::Completed(_) = event {
                saw_completed = true;
                break;
            }
        }
        assert!(saw_completed, "probe 必须先发 Completed");
        // 2) Completed 之后、子进程退出之前:登记仍在(未提前注销)。
        assert!(
            !b1_process_records(&env, &key).is_empty(),
            "协议 Completed 不提前注销"
        );
        // 定向结束探针子进程(触发 adapter 关闭源通道)。按 pid 定向,
        // 不扫描进程表——并行测试里别人的 sleep 子进程也是本进程的 child。
        let probe_pid = adapter.probe_pid();
        assert_ne!(probe_pid, 0, "探针 pid 必须在 spawn 点冻结");
        unsafe {
            libc::kill(probe_pid as i32, libc::SIGKILL);
        }
        // 3) 通道终结后注销。
        while session.events.recv().await.is_some() {}
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(b1_process_records(&env, &key).is_empty(), "真实终止后注销");
    }

    /// 资源 ID 每次 launch 唯一:同 scope 两次 mint 不撞、两条登记共存;
    /// 同 key 同登记幂等 reserve 不冲突。
    #[cfg(unix)]
    #[tokio::test]
    async fn b1_resource_ids_unique_and_reserve_idempotent() {
        let (env, scope, _tree_key) = b1_scope_env();
        let scope = Arc::new(scope);
        let first = TrustedLaunchOwnership::mint(Arc::clone(&scope), "launch one");
        let second = TrustedLaunchOwnership::mint(Arc::clone(&scope), "launch two");
        assert_ne!(first.resource_id(), second.resource_id());
        assert_eq!(first.scope_key(), second.scope_key());

        let reservation = first.reserve().expect("first reserve");
        // 同一载体重复 reserve(同 key 同登记)幂等。
        first.reserve().expect("idempotent reserve");
        second.reserve().expect("second reserve");
        let records = b1_process_records(&env, scope.key());
        assert_eq!(records.len(), 2, "两次 launch 两条登记: {records:?}");
        let ids: Vec<&str> = records.iter().map(|r| r.resource_id.as_str()).collect();
        assert!(ids.contains(&reservation.resource_id()));
    }
}
