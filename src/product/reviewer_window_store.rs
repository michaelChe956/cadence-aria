#![cfg_attr(not(test), allow(dead_code))]
//! 批2 Task 17:Reviewer 窗口 durable 持久化基座(租约/窗口记录/平台写
//! 通道闸门)。
//!
//! 复用 `json_store` 原子写(tmp+flush+sync+rename)与
//! `coding_attempt_store::locking` 的 flock 范式,不重复建锁库。全部
//! 状态都在磁盘文件里:实例无内存态,跨进程/跨重启按根目录即可重建
//! (「journal/租约跨 store 重建仍有效」)。
//!
//! 掉电窗口披露(RevB2T16 ⚠1 同款):本模块在租约/窗口/闸门记录 rename
//! 之后对所在目录补 `sync_dir`;T16 `ReviewerSnapshotStore` 的 blobs/
//! manifest 目录项 fsync 缺口按审查约定**明示接受**该掉电窗口(租约与
//! 窗口记录自身的目录项已 fsync;若掉电恰好丢失 manifest 目录项,窗口
//! 记录的 Running 态闸门复核会因 manifest 缺失/哈希不符而拒绝材料——
//! fail-closed,不产生假通过)。
//!
//! 布局(`<root>` 为平台受信数据目录):
//! ```text
//! <root>/leases/<tree_key>.json        — 树级 durable 租约(一树一窗)
//! <root>/leases/domain.lock            — 租约域锁(创建/释放串行化)
//! <root>/windows/<window_id>.json      — 窗口状态机记录(含 journal)
//! <root>/channels/<common_key>/<channel>.json — 平台写通道闸门
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cross_cutting::provider_resources::ResourceScopeKey;
use crate::product::coding_attempt_store::locking::{
    ExclusiveFileLock, with_exact_exclusive_lock, with_exclusive_lock,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::reviewer_snapshot::{PlatformWritePath, ReviewWorktreeLockIdentity};

pub(crate) const REVIEWER_WINDOW_SCHEMA: &str = "reviewer_window/1";

// ---------------------------------------------------------------------------
// 状态与通道
// ---------------------------------------------------------------------------

/// Reviewer 窗口 durable 状态机(T18 消费固定违规流;终态仅三个 Closed*,
/// 进入终态前置条件=恢复完整+裁决完成,由 `reviewer_window` 守卫)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewerWindowState {
    /// 租约/冻结/quiesce/baseline 进行中(崩溃于此态可重入 resume)。
    Preparing,
    /// 基线已完整持久化,材料采集与 ReviewerVerify 构造(T20)准入。
    Running,
    /// 窗口收口中:材料面已停,等待违规判定/恢复。
    Quiescing,
    /// 已落违规事实(T18 journal),等待恢复。
    ViolationRecorded,
    /// 基线恢复中(T16 restore,游标续接)。
    Restoring,
    /// 恢复完成但存在违规:人工门(仅作废关闭/作废后新轮)。
    ManualGate,
    /// 终态:窗口作废关闭(锁已释放)。
    ClosedInvalidated,
    /// 终态:作废且要求新一轮评审(锁已释放)。
    NewReviewRequired,
    /// 终态:恢复完整+裁决采信后关闭(锁已释放)。
    ClosedAccepted,
    /// 停等:可识别的未知事实(writer 未静止/租约冲突/冻结超时/
    /// 快照失败/恢复中断),锁保留,等恢复驱动或人工处置。
    RecoveryRequired,
}

impl ReviewerWindowState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ReviewerWindowState::Preparing => "preparing",
            ReviewerWindowState::Running => "running",
            ReviewerWindowState::Quiescing => "quiescing",
            ReviewerWindowState::ViolationRecorded => "violation_recorded",
            ReviewerWindowState::Restoring => "restoring",
            ReviewerWindowState::ManualGate => "manual_gate",
            ReviewerWindowState::ClosedInvalidated => "closed_invalidated",
            ReviewerWindowState::NewReviewRequired => "new_review_required",
            ReviewerWindowState::ClosedAccepted => "closed_accepted",
            ReviewerWindowState::RecoveryRequired => "recovery_required",
        }
    }

    /// 终态(进入即已完成锁释放;记录永久保留供审计)。
    pub(crate) fn is_terminal(self) -> bool {
        matches!(
            self,
            ReviewerWindowState::ClosedInvalidated
                | ReviewerWindowState::NewReviewRequired
                | ReviewerWindowState::ClosedAccepted
        )
    }
}

/// 报告采信投影(T18 主导推进;T17 只冻结初值 `PendingAcceptance`,
/// 新窗口报告必须先 Pending 再受信裁决,不默认 Accepted)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReportAcceptance {
    PendingAcceptance,
    Accepted,
    Invalidated,
}

/// 平台写通道(brief:「锁域覆盖 stage/commit/push/补偿」)。闸门按
/// common-dir 域键控——共享 common-dir 的 linked 工作树上的同类 git
/// mutation 落在同一闸门文件,被同一窗口冻结/排空(审查硬约束 2 的
/// git mutation 面)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WriteChannelKind {
    Stage,
    Commit,
    Push,
    Compensation,
}

impl WriteChannelKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            WriteChannelKind::Stage => "stage",
            WriteChannelKind::Commit => "commit",
            WriteChannelKind::Push => "push",
            WriteChannelKind::Compensation => "compensation",
        }
    }
}

pub(crate) const ALL_WRITE_CHANNELS: [WriteChannelKind; 4] = [
    WriteChannelKind::Stage,
    WriteChannelKind::Commit,
    WriteChannelKind::Push,
    WriteChannelKind::Compensation,
];

// ---------------------------------------------------------------------------
// 通道闸门记录
// ---------------------------------------------------------------------------

/// 在途 writer 的 durable 身份(PID-start 防复用;掉电/崩溃后由排空侧
/// 按存活证明收割——进程已死/僵尸/PID 被复用均视为已退出)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChannelWriter {
    pub(crate) writer_id: String,
    pub(crate) boot_identity: String,
    pub(crate) pid: u32,
    pub(crate) start_time_ticks: u64,
    pub(crate) entered_at: String,
}

/// 闸门态:`Open` 放行;`Draining`/`Frozen` 均拒绝新 writer(冻结不是
/// 「只禁新请求」——Draining 还要等在途 writer 真实退出才 Frozen)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub(crate) enum GateState {
    Open,
    Draining { window_id: String },
    Frozen { window_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChannelGateRecord {
    pub(crate) schema: String,
    pub(crate) common_dir: String,
    pub(crate) channel: String,
    pub(crate) state: GateState,
    pub(crate) in_flight: Vec<ChannelWriter>,
    pub(crate) generation: u64,
}

/// 排空进度:是否已 Frozen + 仍存活的在途 writer(按 PID-start 观察)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrainStatus {
    pub(crate) frozen: bool,
    pub(crate) live_writers: Vec<ChannelWriter>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ChannelGateError {
    #[error("reviewer_window_channel_blocked: channel={channel} gate={gate} window={window:?}")]
    Blocked {
        channel: String,
        gate: String,
        window: Option<String>,
    },
    #[error(
        "reviewer_window_channel_owner_mismatch: channel={channel} held_by={held_by} caller={caller}"
    )]
    OwnerMismatch {
        channel: String,
        held_by: String,
        caller: String,
    },
    #[error("reviewer_window_channel_duplicate_writer: {0}")]
    DuplicateWriter(String),
    #[error("reviewer_window_channel_store: {0}")]
    Store(#[from] ProductStoreError),
}

// ---------------------------------------------------------------------------
// 租约与窗口记录
// ---------------------------------------------------------------------------

/// 树级 durable 租约:主键 tree_key(canonical worktree),common-dir
/// 作为冲突域(共享 common-dir 的其它工作树窗口也互斥)。租约不是进程
/// 内 Mutex、无 Drop 释放:只有窗口进入终态(恢复完整+裁决完成)才由
/// `release_lease` 显式删除。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WindowLeaseRecord {
    pub(crate) schema: String,
    pub(crate) lease_id: String,
    pub(crate) tree_key: String,
    pub(crate) canonical_worktree: String,
    pub(crate) gitdir: String,
    pub(crate) common_dir: String,
    pub(crate) window_id: String,
    pub(crate) attempt_id: String,
    pub(crate) run_id: String,
    pub(crate) provider: String,
    pub(crate) acquired_at: String,
}

impl WindowLeaseRecord {
    /// 身份事实相等(忽略时间戳):同窗幂等重入核对用。
    pub(crate) fn matches_identity(&self, other: &WindowLeaseRecord) -> bool {
        self.lease_id == other.lease_id
            && self.tree_key == other.tree_key
            && self.canonical_worktree == other.canonical_worktree
            && self.gitdir == other.gitdir
            && self.common_dir == other.common_dir
            && self.window_id == other.window_id
            && self.attempt_id == other.attempt_id
            && self.run_id == other.run_id
            && self.provider == other.provider
    }
}

/// baseline 绑定(完整持久化 manifest 的引用+字节指纹:材料同源证明)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BaselineBinding {
    pub(crate) manifest_path: String,
    pub(crate) manifest_sha256: String,
    pub(crate) entry_count: usize,
    pub(crate) total_bytes: u64,
    pub(crate) captured_at: String,
}

/// 恢复绑定(T16 恢复结局与游标交接 T18 判定)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RestoreBinding {
    pub(crate) completed: bool,
    pub(crate) from_violation: bool,
    /// `restored` | `recovery_required`(T16 RecoveryRequired 停等)。
    pub(crate) outcome: String,
    pub(crate) cursor_phase: Option<String>,
    pub(crate) entry_index: Option<usize>,
    pub(crate) detail: String,
}

/// 停等记录的来源阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryPhase {
    Begin,
    Restore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RecoveryNote {
    pub(crate) phase: RecoveryPhase,
    /// 稳定原因码(`reviewer_window_*` / T5 `provider_resources_*` 原样)。
    pub(crate) reason: String,
    pub(crate) detail: String,
    pub(crate) cursor_phase: Option<String>,
}

/// 状态 journal(单调追加;随记录原子写;跨 store 重建有效)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TransitionEntry {
    pub(crate) from: Option<String>,
    pub(crate) to: String,
    pub(crate) note: String,
    pub(crate) at: String,
}

/// 窗口 durable 记录。主键 window_id;身份绑 canonical worktree/
/// gitdir/common-dir + attempt/run/provider——**不是**只按 attempt_id:
/// 同树两 attempt 由树级租约互斥。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReviewerWindowRecord {
    pub(crate) schema: String,
    pub(crate) window_id: String,
    pub(crate) generation: u64,
    pub(crate) identity: ReviewWorktreeLockIdentity,
    pub(crate) attempt_id: String,
    pub(crate) run_id: String,
    pub(crate) provider: String,
    pub(crate) state: ReviewerWindowState,
    pub(crate) report_acceptance: ReportAcceptance,
    /// 平台合法写入路径精确预枚举(T16 ⚠8:必须在 worktree 之外,
    /// `reviewer_window` 准入时校验)。
    pub(crate) platform_write_paths: Vec<PlatformWritePath>,
    pub(crate) quiesced_scopes: Vec<ResourceScopeKey>,
    pub(crate) baseline: Option<BaselineBinding>,
    pub(crate) restore: Option<RestoreBinding>,
    pub(crate) recovery: Option<RecoveryNote>,
    /// 与 `RecoveryRequired` 状态分离的瞬时 store 错误(T18 约定:
    /// 计入非通过并带出游标;见 `reviewer_window::restore_window` 文档)。
    pub(crate) last_error: Option<String>,
    pub(crate) transitions: Vec<TransitionEntry>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl ReviewerWindowRecord {
    /// 新窗口记录(Preparing,generation=1,报告采信初值 PendingAcceptance)。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        window_id: &str,
        identity: ReviewWorktreeLockIdentity,
        attempt_id: &str,
        run_id: &str,
        provider: &str,
        platform_write_paths: Vec<PlatformWritePath>,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            schema: REVIEWER_WINDOW_SCHEMA.to_string(),
            window_id: window_id.to_string(),
            generation: 1,
            identity,
            attempt_id: attempt_id.to_string(),
            run_id: run_id.to_string(),
            provider: provider.to_string(),
            state: ReviewerWindowState::Preparing,
            report_acceptance: ReportAcceptance::PendingAcceptance,
            platform_write_paths,
            quiesced_scopes: Vec::new(),
            baseline: None,
            restore: None,
            recovery: None,
            last_error: None,
            transitions: vec![TransitionEntry {
                from: None,
                to: ReviewerWindowState::Preparing.as_str().to_string(),
                note: "created".to_string(),
                at: now.clone(),
            }],
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// 数据层状态推进(调用方负责状态机合法性;store 侧再校验
    /// expected generation 与不可变身份字段)。
    pub(crate) fn bump_transition(&mut self, to: ReviewerWindowState, note: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        self.transitions.push(TransitionEntry {
            from: Some(self.state.as_str().to_string()),
            to: to.as_str().to_string(),
            note: note.to_string(),
            at: now.clone(),
        });
        self.state = to;
        self.updated_at = now;
    }

    /// 数据层停等记录(不动状态;调用方决定 RecoveryRequired 迁移)。
    pub(crate) fn note_recovery(
        &mut self,
        phase: RecoveryPhase,
        reason: &str,
        detail: &str,
        cursor_phase: Option<String>,
    ) {
        self.recovery = Some(RecoveryNote {
            phase,
            reason: reason.to_string(),
            detail: detail.to_string(),
            cursor_phase,
        });
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }
}

// ---------------------------------------------------------------------------
// 键与工具
// ---------------------------------------------------------------------------

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// tree_key:canonical worktree 路径的 sha256(租约文件名;路径转义安全)。
pub(crate) fn tree_key_of(canonical_worktree: &Path) -> String {
    sha256_hex(canonical_worktree.to_string_lossy().as_bytes())
}

/// common_key:common-dir 路径的 sha256(通道闸门文件名)。
pub(crate) fn common_key_of(common_dir: &Path) -> String {
    sha256_hex(common_dir.to_string_lossy().as_bytes())
}

/// 目录 fsync(rename 后补目录项持久化;RevB2T16 ⚠1 处置面)。
fn sync_dir(dir: &Path) -> Result<(), ProductStoreError> {
    let file = std::fs::File::open(dir)
        .map_err(|error| ProductStoreError::Io(format!("open dir {}: {error}", dir.display())))?;
    file.sync_all()
        .map_err(|error| ProductStoreError::Io(format!("sync dir {}: {error}", dir.display())))?;
    Ok(())
}

/// 通道闸门互斥(flock 同 `with_exclusive_lock` 派生锁路径;闭包错误
/// 域为 `ChannelGateError`,内部 store 错误经 `?` 自动入 Store 变体)。
fn with_channel_lock<T>(
    gate_path: &Path,
    operation: impl FnOnce() -> Result<T, ChannelGateError>,
) -> Result<T, ChannelGateError> {
    let _lock = ExclusiveFileLock::acquire(gate_path).map_err(ChannelGateError::Store)?;
    operation()
}

/// `/proc/<pid>/stat` → (state 字符, starttime ticks)。进程消失返回 None。
fn proc_stat(pid: u32) -> Option<(char, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, tail) = stat.rsplit_once(") ")?;
    let mut fields = tail.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let starttime = fields.nth(18)?.parse().ok()?;
    Some((state, starttime))
}

/// writer 存活判定:PID-start 一致且非僵尸/陈尸(ticks=0 表示无法取证
/// ——fail-closed 视为存活,等显式退出或人工处置)。
pub(crate) fn channel_writer_alive(writer: &ChannelWriter) -> bool {
    if writer.start_time_ticks == 0 {
        return true;
    }
    match proc_stat(writer.pid) {
        None => false,
        Some((state, starttime)) => {
            starttime == writer.start_time_ticks && state != 'Z' && state != 'X'
        }
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Reviewer 窗口 durable store(无内存态;同根目录跨进程/重启一致)。
pub(crate) struct ReviewerWindowStore {
    root: PathBuf,
}

impl ReviewerWindowStore {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn windows_dir(&self) -> PathBuf {
        self.root.join("windows")
    }

    fn window_path(&self, window_id: &str) -> PathBuf {
        self.windows_dir().join(format!("{window_id}.json"))
    }

    fn leases_dir(&self) -> PathBuf {
        self.root.join("leases")
    }

    fn lease_path(&self, tree_key: &str) -> PathBuf {
        self.leases_dir().join(format!("{tree_key}.json"))
    }

    fn lease_domain_lock(&self) -> PathBuf {
        self.leases_dir().join("domain.lock")
    }

    fn channel_path(&self, common_dir: &Path, channel: WriteChannelKind) -> PathBuf {
        self.root
            .join("channels")
            .join(common_key_of(common_dir))
            .join(format!("{}.json", channel.as_str()))
    }

    pub(crate) fn create_or_resume_window(
        &self,
        record: &ReviewerWindowRecord,
    ) -> Result<ReviewerWindowRecord, ProductStoreError> {
        validate_relative_id(&record.window_id)?;
        if record.schema != REVIEWER_WINDOW_SCHEMA {
            return Err(ProductStoreError::InvalidRecord {
                kind: "reviewer_window",
                reason: format!("schema 非 {REVIEWER_WINDOW_SCHEMA}"),
            });
        }
        let path = self.window_path(&record.window_id);
        with_exclusive_lock(&path, || {
            if path.is_file() {
                let existing: ReviewerWindowRecord = read_json(&path)?;
                let resumable = existing.window_id == record.window_id
                    && existing.state == ReviewerWindowState::Preparing
                    && existing.identity == record.identity
                    && existing.attempt_id == record.attempt_id
                    && existing.run_id == record.run_id
                    && existing.provider == record.provider;
                if resumable {
                    return Ok(existing);
                }
                return Err(ProductStoreError::Conflict {
                    kind: "reviewer_window",
                    id: record.window_id.clone(),
                });
            }
            write_json(&path, record)?;
            sync_dir(&self.windows_dir())?;
            Ok(record.clone())
        })
    }

    pub(crate) fn load_window(
        &self,
        window_id: &str,
    ) -> Result<ReviewerWindowRecord, ProductStoreError> {
        validate_relative_id(window_id)?;
        let path = self.window_path(window_id);
        if !path.is_file() {
            return Err(ProductStoreError::NotFound {
                kind: "reviewer_window",
                id: window_id.to_string(),
            });
        }
        read_json(&path)
    }
    /// CAS 状态写:expected generation 必须与磁盘一致(否则 Conflict),
    /// 不可变身份字段(window/identity/attempt/run/provider/created_at)
    /// 不得变化(否则 IdentityMismatch);写入 generation=expected+1 并
    /// 对目录补 fsync。
    pub(crate) fn save_window_transition(
        &self,
        next: ReviewerWindowRecord,
        expected_generation: u64,
    ) -> Result<ReviewerWindowRecord, ProductStoreError> {
        validate_relative_id(&next.window_id)?;
        let path = self.window_path(&next.window_id);
        with_exclusive_lock(&path, || {
            if !path.is_file() {
                return Err(ProductStoreError::NotFound {
                    kind: "reviewer_window",
                    id: next.window_id.clone(),
                });
            }
            let stored: ReviewerWindowRecord = read_json(&path)?;
            if stored.generation != expected_generation {
                return Err(ProductStoreError::Conflict {
                    kind: "reviewer_window_generation",
                    id: next.window_id.clone(),
                });
            }
            let immutable = stored.window_id == next.window_id
                && stored.schema == next.schema
                && stored.identity == next.identity
                && stored.attempt_id == next.attempt_id
                && stored.run_id == next.run_id
                && stored.provider == next.provider
                && stored.created_at == next.created_at;
            if !immutable {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "reviewer_window",
                    id: next.window_id.clone(),
                });
            }
            let mut saved = next;
            saved.generation = expected_generation + 1;
            write_json(&path, &saved)?;
            // F2(修复轮 1):rename 后补目录 fsync——与租约/闸门/创建
            // 一致,消除「锁已释放、记录仍旧态」的掉电撕裂窗口。
            sync_dir(&self.windows_dir())?;
            Ok(saved)
        })
    }

    // -- leases ------------------------------------------------------------

    /// 获取树级 durable 租约(租约域锁内完成读-判-写):
    /// - 同 window_id 且身份一致 → 幂等(resume);
    /// - 同 tree_key 其它窗口 → Conflict(两 attempt 同树只能一窗口);
    /// - 共享 common-dir 的其它租约 → Conflict(linked 冲突身份纳入锁集)。
    pub(crate) fn acquire_lease(&self, lease: &WindowLeaseRecord) -> Result<(), ProductStoreError> {
        let lease_path = self.lease_path(&lease.tree_key);
        with_exact_exclusive_lock(&self.lease_domain_lock(), || {
            if lease_path.is_file() {
                let existing: WindowLeaseRecord = read_json(&lease_path)?;
                if existing.window_id == lease.window_id {
                    if existing.matches_identity(lease) {
                        return Ok(());
                    }
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "reviewer_window_lease",
                        id: lease.tree_key.clone(),
                    });
                }
                return Err(ProductStoreError::Conflict {
                    kind: "reviewer_window_lease",
                    id: lease.tree_key.clone(),
                });
            }
            // 共享 common-dir 的兄弟树租约也互斥(审查硬约束 2)
            for other in self.list_leases()? {
                if other.common_dir == lease.common_dir {
                    return Err(ProductStoreError::Conflict {
                        kind: "reviewer_window_lease",
                        id: lease.tree_key.clone(),
                    });
                }
            }
            write_json(&lease_path, lease)?;
            sync_dir(&self.leases_dir())?;
            Ok(())
        })
    }

    /// 释放租约(仅终态关闭路径调用;window_id 必须匹配持有人,删除后
    /// 目录 fsync;幂等:租约已不存在=成功)。
    pub(crate) fn release_lease(
        &self,
        window_id: &str,
        tree_key: &str,
    ) -> Result<(), ProductStoreError> {
        let lease_path = self.lease_path(tree_key);
        with_exact_exclusive_lock(&self.lease_domain_lock(), || {
            if !lease_path.is_file() {
                return Ok(());
            }
            let existing: WindowLeaseRecord = read_json(&lease_path)?;
            if existing.window_id != window_id {
                return Err(ProductStoreError::Conflict {
                    kind: "reviewer_window_lease",
                    id: tree_key.to_string(),
                });
            }
            match std::fs::remove_file(&lease_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(ProductStoreError::Io(format!(
                        "remove lease {}: {error}",
                        lease_path.display()
                    )));
                }
            }
            sync_dir(&self.leases_dir())
        })
    }

    pub(crate) fn load_lease(
        &self,
        tree_key: &str,
    ) -> Result<Option<WindowLeaseRecord>, ProductStoreError> {
        let path = self.lease_path(tree_key);
        if !path.is_file() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    /// 全部租约(冲突归因/兄弟树协调用;租约域语义只增删于域锁内)。
    pub(crate) fn list_leases(&self) -> Result<Vec<WindowLeaseRecord>, ProductStoreError> {
        let dir = self.leases_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    dir.display()
                )));
            }
        };
        let mut leases = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {}: {error}", dir.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Ok(lease) = read_json::<WindowLeaseRecord>(&path) {
                leases.push(lease);
            }
        }
        Ok(leases)
    }

    // -- channels ----------------------------------------------------------

    /// 平台写通道准入:gate Open 才可进入(Draining/Frozen 一律拒绝,
    /// 携带持有窗口 id);重复 writer_id 拒绝。
    pub(crate) fn enter_write_channel(
        &self,
        common_dir: &Path,
        channel: WriteChannelKind,
        writer: ChannelWriter,
    ) -> Result<(), ChannelGateError> {
        let path = self.channel_path(common_dir, channel);
        with_channel_lock(&path, || {
            let mut gate = Self::load_or_init_gate(&path, common_dir, channel)?;
            match gate.state.clone() {
                GateState::Open => {}
                GateState::Draining { window_id } => {
                    return Err(ChannelGateError::Blocked {
                        channel: channel.as_str().to_string(),
                        gate: "draining".to_string(),
                        window: Some(window_id),
                    });
                }
                GateState::Frozen { window_id } => {
                    return Err(ChannelGateError::Blocked {
                        channel: channel.as_str().to_string(),
                        gate: "frozen".to_string(),
                        window: Some(window_id),
                    });
                }
            }
            if gate
                .in_flight
                .iter()
                .any(|in_flight| in_flight.writer_id == writer.writer_id)
            {
                return Err(ChannelGateError::DuplicateWriter(writer.writer_id));
            }
            gate.in_flight.push(writer);
            gate.generation += 1;
            Self::save_gate(&path, &gate)?;
            Ok(())
        })
    }

    /// 在途 writer 退出(Draining/Frozen 下允许——排空等的就是它);
    /// 幂等。
    pub(crate) fn exit_write_channel(
        &self,
        common_dir: &Path,
        channel: WriteChannelKind,
        writer_id: &str,
    ) -> Result<(), ChannelGateError> {
        let path = self.channel_path(common_dir, channel);
        with_channel_lock(&path, || {
            if !path.is_file() {
                return Ok(());
            }
            let mut gate: ChannelGateRecord = read_json(&path)?;
            let before = gate.in_flight.len();
            gate.in_flight
                .retain(|in_flight| in_flight.writer_id != writer_id);
            if gate.in_flight.len() != before {
                gate.generation += 1;
                Self::save_gate(&path, &gate)?;
            }
            Ok(())
        })
    }

    /// 标记 Draining(本窗口;Open→Draining,同窗幂等;他窗持有→
    /// OwnerMismatch),随即收割已死 writer;在途清零直接 Frozen。
    pub(crate) fn mark_channel_draining(
        &self,
        common_dir: &Path,
        channel: WriteChannelKind,
        window_id: &str,
    ) -> Result<DrainStatus, ChannelGateError> {
        let path = self.channel_path(common_dir, channel);
        with_channel_lock(&path, || {
            let mut gate = Self::load_or_init_gate(&path, common_dir, channel)?;
            match gate.state.clone() {
                GateState::Open => {
                    gate.state = GateState::Draining {
                        window_id: window_id.to_string(),
                    };
                }
                GateState::Draining { window_id: held } | GateState::Frozen { window_id: held }
                    if held == window_id => {}
                GateState::Draining { window_id: held } | GateState::Frozen { window_id: held } => {
                    return Err(ChannelGateError::OwnerMismatch {
                        channel: channel.as_str().to_string(),
                        held_by: held,
                        caller: window_id.to_string(),
                    });
                }
            }
            Ok(Self::drain_step(&path, &mut gate)?)
        })
    }

    /// 排空轮询:收割已死 writer;在途清零 → Frozen(本窗口)。
    pub(crate) fn poll_channel_drain(
        &self,
        common_dir: &Path,
        channel: WriteChannelKind,
        window_id: &str,
    ) -> Result<DrainStatus, ChannelGateError> {
        let path = self.channel_path(common_dir, channel);
        with_channel_lock(&path, || {
            if !path.is_file() {
                return Err(ChannelGateError::OwnerMismatch {
                    channel: channel.as_str().to_string(),
                    held_by: "(missing)".to_string(),
                    caller: window_id.to_string(),
                });
            }
            let mut gate: ChannelGateRecord = read_json(&path)?;
            match gate.state.clone() {
                GateState::Open => {
                    return Err(ChannelGateError::OwnerMismatch {
                        channel: channel.as_str().to_string(),
                        held_by: "(open)".to_string(),
                        caller: window_id.to_string(),
                    });
                }
                GateState::Draining { window_id: held } | GateState::Frozen { window_id: held }
                    if held == window_id => {}
                GateState::Draining { window_id: held } | GateState::Frozen { window_id: held } => {
                    return Err(ChannelGateError::OwnerMismatch {
                        channel: channel.as_str().to_string(),
                        held_by: held,
                        caller: window_id.to_string(),
                    });
                }
            }
            Ok(Self::drain_step(&path, &mut gate)?)
        })
    }

    /// 解冻(终态关闭路径;仅持窗口可解;Draining 且仍有在途 → 拒绝;
    /// Open 幂等)。
    pub(crate) fn unfreeze_channel(
        &self,
        common_dir: &Path,
        channel: WriteChannelKind,
        window_id: &str,
    ) -> Result<(), ChannelGateError> {
        let path = self.channel_path(common_dir, channel);
        with_channel_lock(&path, || {
            if !path.is_file() {
                return Ok(());
            }
            let mut gate: ChannelGateRecord = read_json(&path)?;
            match gate.state.clone() {
                GateState::Open => return Ok(()),
                GateState::Draining { window_id: held } if held == window_id => {
                    if !gate.in_flight.is_empty() {
                        return Err(ChannelGateError::Blocked {
                            channel: channel.as_str().to_string(),
                            gate: "draining".to_string(),
                            window: Some(held),
                        });
                    }
                }
                GateState::Frozen { window_id: held } if held == window_id => {}
                GateState::Draining { window_id: held } | GateState::Frozen { window_id: held } => {
                    return Err(ChannelGateError::OwnerMismatch {
                        channel: channel.as_str().to_string(),
                        held_by: held,
                        caller: window_id.to_string(),
                    });
                }
            }
            gate.state = GateState::Open;
            gate.in_flight.clear();
            gate.generation += 1;
            Self::save_gate(&path, &gate)?;
            Ok(())
        })
    }

    /// F5(修复轮 1):查询闸门是否由本窗口持有(终态释放只释放本窗
    /// 实际持有的闸门;未持有/他窗持有/Open 均跳过,不误伤持有人)。
    pub(crate) fn channel_owned_by(
        &self,
        common_dir: &Path,
        channel: WriteChannelKind,
        window_id: &str,
    ) -> Result<bool, ProductStoreError> {
        let path = self.channel_path(common_dir, channel);
        if !path.is_file() {
            return Ok(false);
        }
        let gate: ChannelGateRecord = read_json(&path)?;
        Ok(matches!(
            &gate.state,
            GateState::Draining { window_id: holder }
                | GateState::Frozen { window_id: holder }
                if holder == window_id
        ))
    }

    /// 收割已死 writer(进程消失/僵尸/PID 复用均为已退出);在途清零
    /// → Frozen;持久化并返回排空进度。
    fn drain_step(
        path: &Path,
        gate: &mut ChannelGateRecord,
    ) -> Result<DrainStatus, ProductStoreError> {
        gate.in_flight.retain(channel_writer_alive);
        if gate.in_flight.is_empty() && !matches!(gate.state, GateState::Frozen { .. }) {
            let holder = match gate.state.clone() {
                GateState::Draining { window_id } | GateState::Frozen { window_id } => window_id,
                GateState::Open => String::new(),
            };
            gate.state = GateState::Frozen { window_id: holder };
        }
        gate.generation += 1;
        let frozen = matches!(gate.state, GateState::Frozen { .. });
        let live = gate.in_flight.clone();
        Self::save_gate(path, gate)?;
        Ok(DrainStatus {
            frozen,
            live_writers: live,
        })
    }

    fn load_or_init_gate(
        path: &Path,
        common_dir: &Path,
        channel: WriteChannelKind,
    ) -> Result<ChannelGateRecord, ProductStoreError> {
        if path.is_file() {
            return read_json(path);
        }
        Ok(ChannelGateRecord {
            schema: REVIEWER_WINDOW_SCHEMA.to_string(),
            common_dir: common_dir.display().to_string(),
            channel: channel.as_str().to_string(),
            state: GateState::Open,
            in_flight: Vec::new(),
            generation: 0,
        })
    }

    fn save_gate(path: &Path, gate: &ChannelGateRecord) -> Result<(), ProductStoreError> {
        write_json(path, gate)?;
        if let Some(parent) = path.parent() {
            sync_dir(parent)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 内测
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    fn t17_dir() -> tempfile::TempDir {
        let root = Path::new("/tmp/aria-bug45/t17");
        fs::create_dir_all(root).expect("create /tmp/aria-bug45/t17");
        tempfile::tempdir_in(root).expect("t17 fixture dir")
    }

    fn now() -> String {
        chrono::Utc::now().to_rfc3339()
    }

    fn identity() -> ReviewWorktreeLockIdentity {
        ReviewWorktreeLockIdentity {
            canonical_worktree: PathBuf::from("/tmp/aria-bug45/t17/fake-worktree"),
            gitdir: PathBuf::from("/tmp/aria-bug45/t17/fake-worktree/.git"),
            common_dir: PathBuf::from("/tmp/aria-bug45/t17/fake-worktree/.git"),
            lease_id: "wlease-w1".to_string(),
        }
    }

    fn record(window_id: &str) -> ReviewerWindowRecord {
        ReviewerWindowRecord::new(
            window_id,
            identity(),
            "attempt-1",
            "run-1",
            "claude_code",
            Vec::new(),
        )
    }

    fn lease(window_id: &str, attempt: &str) -> WindowLeaseRecord {
        let identity = identity();
        WindowLeaseRecord {
            schema: REVIEWER_WINDOW_SCHEMA.to_string(),
            lease_id: format!("wlease-{window_id}"),
            tree_key: tree_key_of(&identity.canonical_worktree),
            canonical_worktree: identity.canonical_worktree.display().to_string(),
            gitdir: identity.gitdir.display().to_string(),
            common_dir: identity.common_dir.display().to_string(),
            window_id: window_id.to_string(),
            attempt_id: attempt.to_string(),
            run_id: "run-1".to_string(),
            provider: "claude_code".to_string(),
            acquired_at: now(),
        }
    }

    fn own_writer(writer_id: &str) -> ChannelWriter {
        ChannelWriter {
            writer_id: writer_id.to_string(),
            boot_identity: "boot-test".to_string(),
            pid: std::process::id(),
            start_time_ticks: crate::cross_cutting::process_manager::process_start_time_ticks(
                std::process::id(),
            )
            .unwrap_or(0),
            entered_at: now(),
        }
    }

    #[test]
    fn window_record_cas_identity_immutability_and_journal() {
        let root = t17_dir();
        let store = ReviewerWindowStore::new(root.path());
        let created = store
            .create_or_resume_window(&record("w-cas"))
            .expect("create record");
        assert_eq!(created.generation, 1);
        assert_eq!(created.state, ReviewerWindowState::Preparing);

        // resume:同 id + Preparing + 身份一致 → 幂等
        let resumed = store
            .create_or_resume_window(&record("w-cas"))
            .expect("resume idempotent");
        assert_eq!(resumed.window_id, "w-cas");

        // CAS 正常推进
        let mut next = store.load_window("w-cas").expect("load");
        next.bump_transition(ReviewerWindowState::Running, "test");
        let saved = store
            .save_window_transition(next.clone(), 1)
            .expect("cas save");
        assert_eq!(saved.generation, 2);
        assert_eq!(saved.state, ReviewerWindowState::Running);
        assert_eq!(saved.transitions.len(), 2);

        // 陈旧 generation → Conflict
        let stale = store.load_window("w-cas").expect("reload");
        let err = store
            .save_window_transition(stale, 1)
            .expect_err("stale generation must conflict");
        assert!(matches!(err, ProductStoreError::Conflict { .. }), "{err:?}");

        // 不可变身份字段漂移 → IdentityMismatch
        let mut drifted = store.load_window("w-cas").expect("reload");
        drifted.attempt_id = "attempt-2".to_string();
        let err = store
            .save_window_transition(drifted, 2)
            .expect_err("identity drift must be rejected");
        assert!(
            matches!(err, ProductStoreError::IdentityMismatch { .. }),
            "{err:?}"
        );

        // 非 Preparing 重复 create → Conflict
        let err = store
            .create_or_resume_window(&record("w-cas"))
            .expect_err("recreate on running window");
        assert!(matches!(err, ProductStoreError::Conflict { .. }), "{err:?}");
    }

    #[test]
    fn lease_exclusive_idempotent_sibling_and_rebuild() {
        let root = t17_dir();
        let store = ReviewerWindowStore::new(root.path());

        store
            .acquire_lease(&lease("w-1", "attempt-1"))
            .expect("acquire");
        // 同窗幂等
        store
            .acquire_lease(&lease("w-1", "attempt-1"))
            .expect("idempotent re-acquire");
        // 同树其它窗口冲突
        let err = store
            .acquire_lease(&lease("w-2", "attempt-2"))
            .expect_err("same tree second window");
        assert!(matches!(err, ProductStoreError::Conflict { .. }), "{err:?}");

        // 共享 common-dir 的兄弟树(linked worktree)也冲突
        let mut sibling = lease("w-3", "attempt-3");
        sibling.canonical_worktree = "/tmp/aria-bug45/t17/other-worktree".to_string();
        sibling.tree_key = tree_key_of(Path::new("/tmp/aria-bug45/t17/other-worktree"));
        let err = store
            .acquire_lease(&sibling)
            .expect_err("sibling common-dir must conflict");
        assert!(matches!(err, ProductStoreError::Conflict { .. }), "{err:?}");

        // 无关树可获
        let mut other = lease("w-4", "attempt-4");
        other.canonical_worktree = "/tmp/aria-bug45/t17/unrelated/.git".to_string();
        other.gitdir = other.canonical_worktree.clone();
        other.common_dir = other.canonical_worktree.clone();
        other.tree_key = tree_key_of(Path::new("/tmp/aria-bug45/t17/unrelated/.git"));
        store.acquire_lease(&other).expect("unrelated tree ok");

        // 释放:非持有人拒绝
        let tree_key = tree_key_of(&identity().canonical_worktree);
        let err = store
            .release_lease("w-2", &tree_key)
            .expect_err("release by non-holder");
        assert!(matches!(err, ProductStoreError::Conflict { .. }), "{err:?}");
        // 持有人释放 + 幂等
        store.release_lease("w-1", &tree_key).expect("release");
        store.release_lease("w-1", &tree_key).expect("idempotent");
        assert!(store.load_lease(&tree_key).expect("load").is_none());
        // 释放后可再获
        store
            .acquire_lease(&lease("w-5", "attempt-5"))
            .expect("re-acquire after release");

        // 跨 store 重建:租约仍有效
        let rebuilt = ReviewerWindowStore::new(root.path());
        let err = rebuilt
            .acquire_lease(&lease("w-6", "attempt-6"))
            .expect_err("lease survives store rebuild");
        assert!(matches!(err, ProductStoreError::Conflict { .. }), "{err:?}");
    }

    #[test]
    fn channel_gate_drain_reap_lifecycle() {
        let root = t17_dir();
        let store = ReviewerWindowStore::new(root.path());
        let common = PathBuf::from("/tmp/aria-bug45/t17/fake-common");
        let channel = WriteChannelKind::Commit;

        // 已死进程的 writer:进入后立即被排空收割
        let mut dead = own_writer("dead-writer");
        dead.pid = 999_999_999;
        dead.start_time_ticks = 12345;
        store
            .enter_write_channel(&common, channel, dead)
            .expect("enter");

        let status = store
            .mark_channel_draining(&common, channel, "w-ch")
            .expect("drain");
        assert!(status.frozen, "dead writer reaped immediately");
        assert!(status.live_writers.is_empty());

        // Frozen 下新 writer 拒绝
        let err = store
            .enter_write_channel(&common, channel, own_writer("late-writer"))
            .expect_err("frozen blocks new writers");
        assert!(matches!(err, ChannelGateError::Blocked { .. }), "{err:?}");

        // 他窗 poll/unfreeze → OwnerMismatch
        let err = store
            .poll_channel_drain(&common, channel, "w-other")
            .expect_err("foreign poll");
        assert!(matches!(err, ChannelGateError::OwnerMismatch { .. }));
        let err = store
            .unfreeze_channel(&common, channel, "w-other")
            .expect_err("foreign unfreeze");
        assert!(matches!(err, ChannelGateError::OwnerMismatch { .. }));

        // 持有窗解冻 → Open,writer 可再入
        store
            .unfreeze_channel(&common, channel, "w-ch")
            .expect("unfreeze");
        store
            .enter_write_channel(&common, channel, own_writer("after-open"))
            .expect("enter after unfreeze");
        store
            .exit_write_channel(&common, channel, "after-open")
            .expect("exit");
        store
            .exit_write_channel(&common, channel, "after-open")
            .expect("exit idempotent");

        // 存活 writer 阻塞 Frozen;退出后 poll 达成 Frozen
        store
            .enter_write_channel(&common, channel, own_writer("live-writer"))
            .expect("enter live");
        let status = store
            .mark_channel_draining(&common, channel, "w-ch2")
            .expect("drain with live writer");
        assert!(!status.frozen);
        assert_eq!(status.live_writers.len(), 1);
        // 在途 writer 退出在 Draining 态允许
        store
            .exit_write_channel(&common, channel, "live-writer")
            .expect("exit while draining");
        let status = store
            .poll_channel_drain(&common, channel, "w-ch2")
            .expect("poll");
        assert!(status.frozen);
    }

    #[test]
    fn channel_reaps_zombie_and_pid_reuse() {
        let root = t17_dir();
        let store = ReviewerWindowStore::new(root.path());
        let common = PathBuf::from("/tmp/aria-bug45/t17/fake-common-z");
        let channel = WriteChannelKind::Push;

        // 真实子进程 → SIGKILL 不 wait(僵尸):writer 不再可能写文件
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        let ticks =
            crate::cross_cutting::process_manager::process_start_time_ticks(pid).unwrap_or(0);
        let zombie = ChannelWriter {
            writer_id: "zombie-writer".to_string(),
            boot_identity: "boot-test".to_string(),
            pid,
            start_time_ticks: ticks,
            entered_at: now(),
        };
        store
            .enter_write_channel(&common, channel, zombie)
            .expect("enter zombie");
        let killed = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        assert_eq!(killed, 0, "kill child");
        // 有界等待进入 Z 态(条件等待,非计时断言)
        let mut zombified = false;
        for _ in 0..500 {
            if let Some((state, _)) = proc_stat(pid) {
                if state == 'Z' {
                    zombified = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(zombified, "child should become zombie before reap");
        let status = store
            .mark_channel_draining(&common, channel, "w-z")
            .expect("drain");
        assert!(status.frozen, "zombie writer reaped");
        let _ = child.wait();

        // PID 复用:存活无关进程 + 陈旧 ticks → 原 writer 视为已退出
        store
            .unfreeze_channel(&common, channel, "w-z")
            .expect("unfreeze");
        let reused = ChannelWriter {
            writer_id: "reused-pid".to_string(),
            boot_identity: "boot-test".to_string(),
            pid: std::process::id(),
            start_time_ticks: ticks.max(1) + 999_999,
            entered_at: now(),
        };
        store
            .enter_write_channel(&common, channel, reused)
            .expect("enter reused");
        let status = store
            .mark_channel_draining(&common, channel, "w-z")
            .expect("drain");
        assert!(status.frozen, "reused pid reaped (starttime mismatch)");
    }
}
