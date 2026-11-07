//! 批2 Task 17:Reviewer 窗口 durable 租约与「先停 writer 后采材料」
//! 准入编排。
//!
//! 冻结顺序(brief 逐字):**先 durable 租约 + 平台写通道锁 → 停止/确认
//! 全部已登记 writer 及在途 Git mutation → 持久完整 baseline → 允许采
//! 材料 → 构造 ReviewerVerify → Running**。未知 writer/锁失败/快照失败
//! 不采材料、不 spawn;租约不是进程内 Mutex、无 Drop 释放——锁持有跨
//! 取消/重启,只有恢复完整且相应裁决完成(终态)才释放。
//!
//! 窗口主键绑定 canonical worktree identity(共享 common-dir 的 linked
//! 冲突身份纳入锁集:树级租约互斥 + 通道闸门按 common-dir 键控),并
//! 绑定 attempt/run/provider——**不能只按 attempt_id 锁同一树**。
//!
//! T18 契约(条目级 Io Err 与 RecoveryRequired 的分离,审查硬约束 4):
//! `restore_window` 里 T16 `Err`(Io/Store/Integrity 等瞬时/存储错误)
//! **不**迁移窗口状态,只记 `last_error` 并原样返回——T18 对账时计入
//! 非通过并从 `restore.cursor_phase/entry_index` 带出游标;T16
//! `RestoreOutcome::RecoveryRequired`(域内无法安全恢复)才把窗口迁入
//! `RecoveryRequired` 停等(锁保留)。
//! T18 采信判定(本模块 + `reviewer_reconciliation`):恢复成功 ≠ 可
//! 采信。违规 journal(路径/前后摘要/时间/window/attempt/run/report/
//! restore_cursor)在恢复开始前先落盘,恢复不可删除/覆盖;三域独立
//! 对账 + 记录一致 + 无违规才可 `Accepted`;自提交/改 Git/平台元数据/
//! 通道绕过/可观察篡改 → 整轮 `Invalidated`(人工门只许作废关闭/
//! 作废后新轮,不能把旧报告改 Accepted)。
//!
//! T16 掉电窗口披露(RevB2T16 ⚠1):blobs/manifest 目录项 fsync 缺口
//! 按审查约定接受该掉电窗口;本模块自身的租约/窗口/闸门记录 rename 后
//! 由 `reviewer_window_store::sync_dir` 补目录 fsync。若掉电丢失
//! manifest 目录项,`assert_review_window_current` 的 Running 复核因
//! manifest 缺失/哈希不符拒绝材料——fail-closed,不产生假通过。

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::cross_cutting::provider_resources::{
    ProviderRunResourceScope, QuiesceOutcome, QuiescentScope,
};
use crate::product::json_store::ProductStoreError;
use crate::product::reviewer_snapshot::{
    CaptureRequest, PlatformWritePath, QuiescenceEvidence, RestoreOutcome, RestoreReport,
    ReviewWorktreeLockIdentity, ReviewerSnapshotError, ReviewerSnapshotStore, SnapshotBudget,
};
use crate::product::reviewer_window_store::{
    ALL_WRITE_CHANNELS, BaselineBinding, ChannelGateError, ChannelWriter, RecoveryPhase,
    ReportAcceptance, RestoreBinding, ReviewerWindowRecord, ReviewerWindowState,
    ReviewerWindowStore, ViolationJournalEntry, ViolationPending, WindowLeaseRecord, tree_key_of,
};

/// 排空轮询间隔(测试缩短保持秒级;同 T5 cfg 分档)。
#[cfg(not(test))]
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(test)]
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(25);

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReviewerWindowError {
    #[error("reviewer_window_invalid_request: {0}")]
    InvalidRequest(String),
    #[error("reviewer_window_identity_drift: {0}")]
    IdentityDrift(String),
    /// 跨窗口互斥(同树/共享 common-dir/通道持有/同窗重建)。
    #[error("reviewer_window_conflict[{kind}]: {id}: {detail}")]
    Conflict {
        kind: &'static str,
        id: String,
        detail: String,
    },
    /// 排空超时:在途 writer 仍存活——停等(窗口 RecoveryRequired,锁保留)。
    #[error("reviewer_window_channel_drain_timeout: {detail}")]
    ChannelDrainTimeout { detail: String },
    /// T5 静止未证明(未知 writer/killpg 后仍写/未确认预留)——停等。
    #[error("reviewer_window_quiesce_not_proven: {reason}: {detail}")]
    QuiesceNotProven { reason: String, detail: String },
    /// T5 quiesce 存储/引擎通道错误——停等(不伪称静止)。
    #[error("reviewer_window_quiesce_failed: {0}")]
    QuiesceFailed(String),
    #[error("reviewer_window_snapshot: {0}")]
    Snapshot(#[from] ReviewerSnapshotError),
    #[error("reviewer_window_channel: {0}")]
    Channel(#[from] ChannelGateError),
    #[error("reviewer_window_store: {0}")]
    Store(#[from] ProductStoreError),
    /// `assert_review_window_current` 同源复验失败(gateway/spawn 与平台
    /// mutation 守卫的统一拒绝面)。
    #[error("reviewer_window_not_current: window={window_id}: {reason}")]
    WindowNotCurrent { window_id: String, reason: String },
    #[error("reviewer_window_invalid_transition: {from} -> {to}: {detail}")]
    InvalidTransition {
        from: String,
        to: String,
        detail: String,
    },
    /// 恢复域无法安全继续(T16 RecoveryRequired)——窗口停等,锁保留。
    #[error("reviewer_window_recovery_stopped: {reason}: {detail} (cursor={cursor})")]
    RecoveryStopped {
        reason: String,
        detail: String,
        cursor: String,
    },
    /// 违规已观测但 journal 未落盘(停等标记持有)——恢复与采信关闭
    /// 均拒绝,直到 journal 落盘成功(重入 `record_violation` 幂等续接)。
    #[error("reviewer_window_violation_journal_pending: window={window_id}: {detail}")]
    ViolationJournalPending { window_id: String, detail: String },
}

// ---------------------------------------------------------------------------
// 请求/产物/证明
// ---------------------------------------------------------------------------

pub(crate) struct ReviewerWindowBeginRequest<'a> {
    pub(crate) window_id: &'a str,
    pub(crate) attempt_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) provider: &'a str,
    pub(crate) worktree: &'a Path,
    /// 全部已登记 writer 的 T5 scope(顺序固定:**[0] 为主 scope**——
    /// 其静止证明交接 T16 manifest;后续为协调 scope,如共享 common-dir
    /// 兄弟树的 coder scope)。主 scope 即使无登记资源也必须传入
    /// (空 manifest 的 Quiescent 即合法静止证明)。
    pub(crate) quiesce_targets: &'a [&'a ProviderRunResourceScope],
    /// 平台合法写入路径精确预枚举(必须在 worktree 之外且不在 Git 域)。
    pub(crate) platform_write_paths: Vec<PlatformWritePath>,
    /// 快照预算(生产默认由调用方给定;测试用 `SnapshotBudget::generous`)。
    pub(crate) budget: SnapshotBudget,
    /// 通道排空等待窗(在途 writer 退出/被收割的期限;超时停等)。
    pub(crate) drain_timeout: Duration,
}

/// 开窗成功产物(Running;材料采集与 ReviewerVerify 构造从此刻准入)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewerWindowHandle {
    pub(crate) window_id: String,
    pub(crate) generation: u64,
    pub(crate) identity: ReviewWorktreeLockIdentity,
    /// snapshot store 内相对引用(非绝对路径;经 load_manifest 解析)。
    pub(crate) manifest_path: PathBuf,
    pub(crate) manifest_sha256: String,
    pub(crate) entry_count: usize,
    pub(crate) total_bytes: u64,
}

/// `assert_review_window_current` 的期望(window_id + attempt/run/provider
/// + worktree):同源复验时全部重核,任何漂移即拒。
pub(crate) struct WindowExpectation<'a> {
    pub(crate) window_id: &'a str,
    pub(crate) attempt_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) provider: &'a str,
    pub(crate) worktree: &'a Path,
}

/// 同源复验证明:gateway/spawn/材料采集方据此确认窗口当前有效且材料
/// 与 baseline manifest 同指纹。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentWindowProof {
    pub(crate) window_id: String,
    pub(crate) generation: u64,
    pub(crate) state: ReviewerWindowState,
    pub(crate) report_acceptance: ReportAcceptance,
    /// snapshot store 内相对引用(非绝对路径;经 load_manifest 解析)。
    pub(crate) manifest_path: PathBuf,
    pub(crate) manifest_sha256: String,
    pub(crate) entry_count: usize,
    pub(crate) total_bytes: u64,
}

/// 恢复结局摘要(T18 消费 `RestoreReport` 做三域判定;`manual_gate`
#[derive(Debug)]
pub(crate) struct RestoreSummary {
    pub(crate) record: ReviewerWindowRecord,
    pub(crate) manual_gate: bool,
}

/// 人工门裁决(ManualGate 专属)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManualGateDecision {
    /// 作废关闭(旧报告永久 Invalidated)。
    Invalidate,
    /// 作废后要求新一轮评审(新窗口重加锁/新 baseline/新材料)。
    RequireNewReview,
}

/// 观测违规事实(T18 journal 输入;由观察方——T5 监视/通道闸门/平台
/// 写监视——提供)。`kind` 用稳定码(见 `ViolationJournalEntry::kind`)。
#[cfg_attr(not(test), allow(dead_code))] // 违规 journal 输入载体:生产观察方(T5 监视/通道闸门)接线前由恢复测试直接构造
pub(crate) struct ViolationRecord<'a> {
    pub(crate) kind: &'a str,
    /// 违规涉及的路径(相对 worktree/gitdir 或绝对平台路径)。
    pub(crate) path: &'a str,
    /// 前摘要(基线/期望内容指纹;未知为 None)。
    pub(crate) before_digest: Option<&'a str>,
    /// 后摘要(观察到的内容指纹;未知为 None)。
    pub(crate) after_digest: Option<&'a str>,
    pub(crate) detail: &'a str,
    /// 触发违规判定时绑定的报告(可空)。
    pub(crate) report_id: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// 身份解析(worktree → canonical/gitdir/common-dir;镜像 T16 语义,
// capture/restore 侧仍以 T16 严格校验为准)
// ---------------------------------------------------------------------------

/// 解析被审 worktree 的锁身份三路径。root `.git` 为 symlink/特殊文件
/// 拒绝;gitfile 指针与 commondir 相对/绝对均解析。
pub(crate) fn resolve_worktree_git_identity(
    worktree: &Path,
) -> Result<(PathBuf, PathBuf, PathBuf), ReviewerWindowError> {
    let canonical = worktree.canonicalize().map_err(|error| {
        ReviewerWindowError::InvalidRequest(format!(
            "worktree 不可解析 {}: {error}",
            worktree.display()
        ))
    })?;
    let drift = |detail: String| ReviewerWindowError::IdentityDrift(detail);
    let dot_git = canonical.join(".git");
    let metadata = std::fs::symlink_metadata(&dot_git)
        .map_err(|error| drift(format!("stat {}: {error}", dot_git.display())))?;
    let gitdir = if metadata.is_dir() {
        dot_git
            .canonicalize()
            .map_err(|error| drift(format!("canonicalize gitdir: {error}")))?
    } else if metadata.is_file() {
        let content = std::fs::read_to_string(&dot_git)
            .map_err(|error| drift(format!("read gitfile: {error}")))?;
        let target = content
            .strip_prefix("gitdir:")
            .map(str::trim)
            .ok_or_else(|| drift(format!("gitfile 内容非法: {content:?}")))?;
        let target_path = PathBuf::from(target);
        let resolved = if target_path.is_absolute() {
            target_path
        } else {
            canonical.join(target_path)
        };
        resolved
            .canonicalize()
            .map_err(|error| drift(format!("gitfile 指向不可解析: {error}")))?
    } else {
        return Err(drift("root .git 为 symlink/特殊文件,拒绝解析".to_string()));
    };
    let common_dir = match std::fs::read_to_string(gitdir.join("commondir")) {
        Ok(raw) => {
            let target = PathBuf::from(raw.trim());
            let resolved = if target.is_absolute() {
                target
            } else {
                gitdir.join(target)
            };
            resolved
                .canonicalize()
                .map_err(|error| drift(format!("commondir 不可解析: {error}")))?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => gitdir.clone(),
        Err(error) => return Err(drift(format!("read commondir: {error}"))),
    };
    Ok((canonical, gitdir, common_dir))
}

/// 词法规范化绝对路径(平台合法写入路径可能尚不存在,不能 canonicalize)。
fn normalized_absolute(path: &Path) -> Result<PathBuf, ReviewerWindowError> {
    if !path.is_absolute() {
        return Err(ReviewerWindowError::InvalidRequest(format!(
            "平台合法写入路径必须为绝对路径: {}",
            path.display()
        )));
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => out.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
            Component::Prefix(_) => {
                return Err(ReviewerWindowError::InvalidRequest(
                    "平台合法写入路径含非 unix 前缀".to_string(),
                ));
            }
        }
    }
    Ok(out)
}

/// 校验平台合法写入路径预枚举(T16 ⚠8):必须在被审 worktree 之外,
/// 且不得位于 Git 域(gitdir/common-dir——Git 观察域由 T16 单独基线)。
fn validate_platform_write_paths(
    paths: &[PlatformWritePath],
    canonical_worktree: &Path,
    gitdir: &Path,
    common_dir: &Path,
) -> Result<(), ReviewerWindowError> {
    for spec in paths {
        if spec.ownership.trim().is_empty() {
            return Err(ReviewerWindowError::InvalidRequest(
                "平台合法写入域归属必须非空".to_string(),
            ));
        }
        let normalized = normalized_absolute(&spec.path)?;
        let inside = |root: &Path| normalized == *root || normalized.starts_with(root);
        if inside(canonical_worktree) {
            return Err(ReviewerWindowError::InvalidRequest(format!(
                "平台合法写入路径必须在 worktree 之外: {} (worktree={})",
                normalized.display(),
                canonical_worktree.display()
            )));
        }
        if inside(gitdir) || inside(common_dir) {
            return Err(ReviewerWindowError::InvalidRequest(format!(
                "平台合法写入路径不得位于 Git 域: {} (gitdir/common-dir)",
                normalized.display()
            )));
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// 开窗编排
// ---------------------------------------------------------------------------

/// 开窗(冻结顺序的单一实现点)。任何失败:窗口记录迁入
/// `RecoveryRequired`(保留可诊断恢复状态,不重新冻结当前态)、租约与
/// 已冻结通道保留(fail-closed),由恢复驱动/人工经 `abandon_window`
/// 或重试处置;材料/spawn 闸门(`assert_review_window_current`)因窗口
/// 未达 Running 而全部拒绝。
///
/// 同 id 已有记录时:非终态且身份一致(Preparing)→ 续接(崩溃重入
/// 幂等);非终态且身份不一致 → 拒绝(fail-closed,活窗口不得被抢);
/// **已终态** → 允许新 attempt 接管(旧记录与旧违规 journal 归档留证,
/// 按新身份重建)——终态窗口已收口、锁已释放,不再是活窗口。
pub(crate) async fn begin_review_window(
    store: &ReviewerWindowStore,
    snapshot: &ReviewerSnapshotStore,
    request: &ReviewerWindowBeginRequest<'_>,
) -> Result<ReviewerWindowHandle, ReviewerWindowError> {
    for (label, value) in [
        ("window_id", request.window_id),
        ("attempt_id", request.attempt_id),
        ("run_id", request.run_id),
        ("provider", request.provider),
    ] {
        if value.trim().is_empty() {
            return Err(ReviewerWindowError::InvalidRequest(format!(
                "{label} 必须非空"
            )));
        }
    }
    crate::product::json_store::validate_relative_id(request.window_id)
        .map_err(|error| ReviewerWindowError::InvalidRequest(format!("window_id 非法: {error}")))?;

    // 冻结顺序从此处开始固定:身份解析 → 租约 → 通道锁 → 静止 →
    // baseline → Running(材料/spawn 准入)。
    let (canonical, gitdir, common_dir) = resolve_worktree_git_identity(request.worktree)?;
    // T16 ⚠8:平台合法写入路径预枚举必须在 worktree 之外(且不在 Git 域)。
    validate_platform_write_paths(
        &request.platform_write_paths,
        &canonical,
        &gitdir,
        &common_dir,
    )?;
    let identity = ReviewWorktreeLockIdentity {
        canonical_worktree: canonical.clone(),
        gitdir: gitdir.clone(),
        common_dir: common_dir.clone(),
        lease_id: format!("wlease-{}", request.window_id),
    };

    // 1. 窗口 durable 记录(Preparing;同 id 崩溃重入幂等)。
    // 同 id 旧记录**已终态**(窗口已收口、租约/通道已释放)→ 允许新
    // attempt 接管:旧记录归档留证,按新身份重建(旧记录不被静默覆盖
    // 成活跃态)。旧记录**非终态**且身份不一致 → 活窗口不得被他人抢
    // 走(fail-closed 拒绝,不放宽)。
    let creation = store
        .create_or_takeover_window(&ReviewerWindowRecord::new(
            request.window_id,
            identity.clone(),
            request.attempt_id,
            request.run_id,
            request.provider,
            request.platform_write_paths.clone(),
        ))
        .map_err(|error| match error {
            ProductStoreError::Conflict { id, .. } => ReviewerWindowError::Conflict {
                kind: "reviewer_window",
                id,
                detail: "窗口记录已存在(非 Preparing 或身份不一致;终态旧记录可被新 attempt 接管)"
                    .to_string(),
            },
            other => ReviewerWindowError::Store(other),
        })?;
    let takeover = creation.takeover;
    let mut current = creation.record;

    // 2. durable 树级租约(先于一切冻结动作;两 attempt 同树只能一窗,
    // 共享 common-dir 的兄弟树窗口互斥)。
    let lease = WindowLeaseRecord {
        schema: crate::product::reviewer_window_store::REVIEWER_WINDOW_SCHEMA.to_string(),
        lease_id: identity.lease_id.clone(),
        tree_key: tree_key_of(&canonical),
        canonical_worktree: canonical.display().to_string(),
        gitdir: gitdir.display().to_string(),
        common_dir: common_dir.display().to_string(),
        window_id: request.window_id.to_string(),
        attempt_id: request.attempt_id.to_string(),
        run_id: request.run_id.to_string(),
        provider: request.provider.to_string(),
        acquired_at: now_rfc3339(),
        // ⚠3(T21):同窗单写者进程标识——跨进程并发 begin 互斥(owner
        // 存活时他人同窗重入 Conflict;崩溃后接管续写)。
        owner_pid: std::process::id(),
        owner_start_ticks: crate::cross_cutting::process_manager::process_start_time_ticks(
            std::process::id(),
        )
        .unwrap_or(0),
    };
    if let Err(error) = store.acquire_lease(&lease) {
        let detail = lease_conflict_detail(store, &lease, &error);
        mark_begin_recovery(
            store,
            &current,
            "reviewer_window_lease_conflict",
            detail.clone(),
        );
        return Err(ReviewerWindowError::Conflict {
            kind: "reviewer_window_lease",
            id: lease.tree_key,
            detail,
        });
    }

    // 3. 平台写通道锁:四通道 Draining→等待在途 writer 退出→Frozen
    // (锁域覆盖 stage/commit/push/补偿,不能只禁新请求)。
    if let Err(error) =
        freeze_all_channels(store, &common_dir, request.window_id, request.drain_timeout).await
    {
        let reason = match &error {
            ReviewerWindowError::ChannelDrainTimeout { .. } => {
                "reviewer_window_channel_drain_timeout"
            }
            ReviewerWindowError::Conflict { .. } => "reviewer_window_channel_conflict",
            _ => "reviewer_window_channel_failure",
        };
        mark_begin_recovery(store, &current, reason, error.to_string());
        return Err(error);
    }

    // 4. 停止/确认全部已登记 writer(T5 真实静止;killpg 成功≠静止,
    // 未知 daemon/未确认预留停等)。
    let mut evidences: Vec<QuiescentScope> = Vec::new();
    let mut quiesced_scope_keys = Vec::new();
    for scope in request.quiesce_targets {
        match scope.quiesce_scope().await {
            Err(error) => {
                let detail = error.to_string();
                mark_begin_recovery(
                    store,
                    &current,
                    "reviewer_window_quiesce_store_failure",
                    detail.clone(),
                );
                return Err(ReviewerWindowError::QuiesceFailed(detail));
            }
            Ok(QuiesceOutcome::RecoveryRequired(unknown)) => {
                mark_begin_recovery(store, &current, &unknown.reason, unknown.detail.clone());
                return Err(ReviewerWindowError::QuiesceNotProven {
                    reason: unknown.reason,
                    detail: unknown.detail,
                });
            }
            Ok(QuiesceOutcome::Quiescent(evidence)) => {
                quiesced_scope_keys.push(evidence.key().clone());
                evidences.push(evidence);
            }
        }
    }
    let Some(primary) = evidences.first() else {
        let detail = "quiesce_targets 至少包含主 scope(空 manifest 的静止证明亦须传入)".to_string();
        mark_begin_recovery(
            store,
            &current,
            "reviewer_window_quiesce_target_missing",
            detail.clone(),
        );
        return Err(ReviewerWindowError::InvalidRequest(detail));
    };

    // 5. 持久完整 baseline(T16:全部 blob+manifest 落盘成功才返回;
    // 失败整单拒绝,无已发布 manifest)。F4(修复轮 1):崩溃于
    // 「manifest 已发布、Running CAS 未落」窗口时,同窗重入对已发布
    // manifest 校验归属(window/attempt/run/identity)后**采用**,不
    // 作废最贵一步;仅身份不符才失败停等。
    // 终态接管(旧记录已终态、新 attempt 复用同 id)时:旧基线归属不符
    // → 归档旧基线并重新采集(旧窗口已收口,其基线不再是新窗口的恢复
    // 依据);归属相符则照旧采用(同 attempt 重入,保留原基线)。
    let baseline = match capture_or_adopt_baseline(
        snapshot, request, &canonical, &identity, primary, takeover,
    ) {
        Ok(baseline) => baseline,
        Err(failure) => {
            mark_begin_recovery(store, &current, failure.reason, failure.detail);
            return Err(failure.error);
        }
    };

    // 6. Preparing → Running:完整持久 manifest 之后才允许采材料与
    // 构造 ReviewerVerify(T20 经 assert_review_window_current 准入)。
    let expected = current.generation;
    current.baseline = Some(BaselineBinding {
        manifest_path: baseline.manifest_path.clone(),
        manifest_sha256: baseline.manifest_sha256.clone(),
        entry_count: baseline.entry_count,
        total_bytes: baseline.total_bytes,
        captured_at: baseline.captured_at.clone(),
    });
    current.quiesced_scopes = quiesced_scope_keys;
    current.bump_transition(
        ReviewerWindowState::Running,
        "baseline 持久完整;材料采集与 ReviewerVerify 构造准入",
    );
    let saved = store.save_window_transition(current, expected)?;
    Ok(ReviewerWindowHandle {
        window_id: request.window_id.to_string(),
        generation: saved.generation,
        identity,
        manifest_path: PathBuf::from(&baseline.manifest_path),
        manifest_sha256: baseline.manifest_sha256,
        entry_count: baseline.entry_count,
        total_bytes: baseline.total_bytes,
    })
}

/// 基线捕获失败(停等落账的 reason/detail + 原错误)。
struct BaselineFailure {
    error: ReviewerWindowError,
    reason: &'static str,
    detail: String,
}

/// 开窗第 5 步:基线捕获/采用(含终态接管的旧基线归档重采)。
///
/// - 无既有 manifest → 全新采集;
/// - 既有 manifest 归属一致(window/attempt/run/identity)→ 采用
///   (F4 崩溃重入;**CAS blob 完整性预检**同源——manifest 已发布≠基线
///   完整,掉电可丢 blob 目录项);
/// - 既有 manifest 归属不一致 → **终态接管**时归档旧基线并重新采集
///   (旧窗口已收口,其基线不再是新窗口的恢复依据);非接管场景
///   fail-closed(IdentityDrift,不得跨身份采信旧基线);
/// - 既有基线损坏(blob 缺失/manifest 不可读)→ 终态接管时同处置;
///   非接管场景停等(不绑定不可恢复基线)。
fn capture_or_adopt_baseline(
    snapshot: &ReviewerSnapshotStore,
    request: &ReviewerWindowBeginRequest<'_>,
    canonical: &Path,
    identity: &ReviewWorktreeLockIdentity,
    primary: &QuiescentScope,
    takeover: bool,
) -> Result<BaselineFacts, Box<BaselineFailure>> {
    let capture = || CaptureRequest {
        window_id: request.window_id,
        attempt_id: request.attempt_id,
        run_id: request.run_id,
        worktree: canonical,
        lock_identity: Some(identity.clone()),
        quiescence: Some(QuiescenceEvidence::from_quiescent_scope(primary)),
        platform_write_paths: request.platform_write_paths.clone(),
        budget: request.budget,
    };
    // 终态接管时最多归档重采一次(归档后不应再有既有 manifest)。
    let mut archived = false;
    loop {
        match snapshot.capture_baseline(capture()) {
            Ok(_captured) => {
                return match snapshot.load_manifest(request.window_id) {
                    Ok(manifest) => Ok(BaselineFacts::from_manifest(
                        &manifest,
                        manifest_store_ref(request.window_id),
                        now_rfc3339(),
                    )),
                    Err(error) => {
                        let detail = error.to_string();
                        Err(Box::new(BaselineFailure {
                            error: ReviewerWindowError::Snapshot(error),
                            reason: "reviewer_window_manifest_unreadable",
                            detail,
                        }))
                    }
                };
            }
            Err(ReviewerSnapshotError::Store(ProductStoreError::Conflict { ref id, .. }))
                if id == request.window_id =>
            {
                // 同窗 manifest 已存在(崩溃重入):归属校验 + CAS blob 完
                // 整性预检,全部通过才采用。
                let verdict = match snapshot.verify_baseline_cas(request.window_id) {
                    Ok(manifest)
                        if manifest.window_id == request.window_id
                            && manifest.attempt_id == request.attempt_id
                            && manifest.run_id == request.run_id
                            && &manifest.lock_identity == identity =>
                    {
                        BaselineVerdict::Adopted(BaselineFacts::from_manifest(
                            &manifest,
                            manifest_store_ref(request.window_id),
                            now_rfc3339(),
                        ))
                    }
                    Ok(_) => BaselineVerdict::OwnerMismatch,
                    Err(error) => BaselineVerdict::Broken(error),
                };
                match verdict {
                    BaselineVerdict::Adopted(facts) => return Ok(facts),
                    BaselineVerdict::OwnerMismatch => {
                        if takeover && !archived {
                            match snapshot.archive_window_baseline(request.window_id) {
                                Ok(_) => {
                                    archived = true;
                                    continue;
                                }
                                Err(error) => {
                                    let detail = error.to_string();
                                    return Err(Box::new(BaselineFailure {
                                        error: ReviewerWindowError::Snapshot(error),
                                        reason: "reviewer_window_snapshot_failed",
                                        detail,
                                    }));
                                }
                            }
                        }
                        let detail =
                            "已发布 manifest 归属不符(window/attempt/run/identity)".to_string();
                        return Err(Box::new(BaselineFailure {
                            error: ReviewerWindowError::IdentityDrift(detail.clone()),
                            reason: "reviewer_window_manifest_owner_mismatch",
                            detail,
                        }));
                    }
                    BaselineVerdict::Broken(error) => {
                        if takeover && !archived {
                            match snapshot.archive_window_baseline(request.window_id) {
                                Ok(_) => {
                                    archived = true;
                                    continue;
                                }
                                Err(archive_error) => {
                                    let detail = archive_error.to_string();
                                    return Err(Box::new(BaselineFailure {
                                        error: ReviewerWindowError::Snapshot(archive_error),
                                        reason: "reviewer_window_snapshot_failed",
                                        detail,
                                    }));
                                }
                            }
                        }
                        // blob 缺失/损坏或 manifest 不可读:停等,不绑定
                        // 不可恢复基线。
                        let detail = error.to_string();
                        return Err(Box::new(BaselineFailure {
                            error: ReviewerWindowError::Snapshot(error),
                            reason: "reviewer_window_snapshot_failed",
                            detail,
                        }));
                    }
                }
            }
            Err(error) => {
                let detail = error.to_string();
                return Err(Box::new(BaselineFailure {
                    error: ReviewerWindowError::Snapshot(error),
                    reason: "reviewer_window_snapshot_failed",
                    detail,
                }));
            }
        }
    }
}

/// 既有基线核验结论。
enum BaselineVerdict {
    /// 归属一致且基线完整 → 采用。
    Adopted(BaselineFacts),
    /// manifest 归属与本次开窗身份不符。
    OwnerMismatch,
    /// 基线损坏(blob 缺失/manifest 不可读)。
    Broken(ReviewerSnapshotError),
}

/// baseline 事实(捕获/采用两路径同构;指纹=manifest 规范化 JSON 序列
/// 化的 sha256——内容寻址,与文件字节排版无关,assert 侧同源可复核)。
struct BaselineFacts {
    manifest_path: String,
    manifest_sha256: String,
    entry_count: usize,
    total_bytes: u64,
    captured_at: String,
}

impl BaselineFacts {
    fn from_manifest(
        manifest: &crate::product::reviewer_snapshot::ReviewerBaselineManifest,
        manifest_path: String,
        captured_at: String,
    ) -> Self {
        Self {
            manifest_path,
            manifest_sha256: manifest_fingerprint(manifest),
            entry_count: manifest.entries.len(),
            total_bytes: manifest.entries.iter().map(|entry| entry.size).sum(),
            captured_at,
        }
    }
}

/// manifest 指纹:规范化 JSON 序列化 sha256(内容寻材证明)。
///
/// 规范化规则:`restore_cursor` 归一到 capture 初值(Started+0)再序列化
/// ——恢复推进的是执行进度,不是基线内容;不归一的话任何成功恢复都会
/// 使指纹漂移,同源核验永远失败。capture 时游标本就是初值,故 BaselineFacts
/// 取值不变。经 `load_manifest` 同源加载后计算,**不可按文件字节复算**
/// (审查硬约束 3:证据链同源)。
pub(crate) fn manifest_fingerprint(
    manifest: &crate::product::reviewer_snapshot::ReviewerBaselineManifest,
) -> String {
    let mut canonical = manifest.clone();
    canonical.restore_cursor = crate::product::reviewer_snapshot::RestoreCursor::start();
    serde_json::to_string(&canonical)
        .map(|serialized| sha256_hex(serialized.as_bytes()))
        .unwrap_or_default()
}

/// manifest 在 snapshot store 内的**相对引用**(NEW-2:两路径同形;
/// 不是绝对路径,消费方经 `ReviewerSnapshotStore::load_manifest` 解析,
/// 不得直接 fs::read)。
fn manifest_store_ref(window_id: &str) -> String {
    format!("windows/{window_id}/manifest.json")
}

/// 通道排空冻结(四个通道全部 Draining→在途清零→Frozen;「不能只禁
/// 新请求」)。任何通道他窗持有/超时未排空 → Err(调用方停等)。
async fn freeze_all_channels(
    store: &ReviewerWindowStore,
    common_dir: &Path,
    window_id: &str,
    drain_timeout: Duration,
) -> Result<(), ReviewerWindowError> {
    let mut frozen = [false; ALL_WRITE_CHANNELS.len()];
    for (index, channel) in ALL_WRITE_CHANNELS.iter().enumerate() {
        let status = store
            .mark_channel_draining(common_dir, *channel, window_id)
            .map_err(|error| map_channel_gate_error(window_id, error))?;
        frozen[index] = status.frozen;
    }
    let deadline = tokio::time::Instant::now() + drain_timeout;
    loop {
        if frozen.iter().all(|&done| done) {
            return Ok(());
        }
        for (index, channel) in ALL_WRITE_CHANNELS.iter().enumerate() {
            if frozen[index] {
                continue;
            }
            let status = store
                .poll_channel_drain(common_dir, *channel, window_id)
                .map_err(|error| map_channel_gate_error(window_id, error))?;
            frozen[index] = status.frozen;
        }
        if frozen.iter().all(|&done| done) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            let mut live_writer_ids = Vec::new();
            for channel in ALL_WRITE_CHANNELS {
                if let Ok(status) = store.poll_channel_drain(common_dir, channel, window_id) {
                    live_writer_ids.extend(
                        status
                            .live_writers
                            .into_iter()
                            .map(|writer| writer.writer_id),
                    );
                }
            }
            return Err(ReviewerWindowError::ChannelDrainTimeout {
                detail: format!("在途 writer 未退出: [{}]", live_writer_ids.join(",")),
            });
        }
        tokio::time::sleep(DRAIN_POLL_INTERVAL).await;
    }
}

fn map_channel_gate_error(window_id: &str, error: ChannelGateError) -> ReviewerWindowError {
    match error {
        ChannelGateError::OwnerMismatch {
            channel: held_channel,
            held_by,
            caller,
        } => ReviewerWindowError::Conflict {
            kind: "reviewer_window_channel",
            id: held_channel,
            detail: format!("channel 持有人不匹配: held_by={held_by} caller={caller}({window_id})"),
        },
        ChannelGateError::Store(error) => ReviewerWindowError::Store(error),
        other => ReviewerWindowError::Channel(other),
    }
}

/// 开窗失败停等落账(尽力写:写失败时窗口停在 Preparing——同样非
/// Running,fail-closed;返回原错误)。
fn mark_begin_recovery(
    store: &ReviewerWindowStore,
    record: &ReviewerWindowRecord,
    reason: &str,
    detail: String,
) {
    let mut next = record.clone();
    next.note_recovery(RecoveryPhase::Begin, reason, &detail, None);
    next.bump_transition(
        ReviewerWindowState::RecoveryRequired,
        "开窗失败停等(租约/已冻结通道保留)",
    );
    let expected = record.generation;
    let _ = store.save_window_transition(next, expected);
}

/// T21:补偿驱动停等落账(镜像 `mark_begin_recovery`;任意非终态 →
/// `RecoveryRequired`,租约与已冻结通道保留,fail-closed)。best-effort:
/// 写失败时返回窗口当前态(非终态即继续持锁)。phase 记 `Restore`——
/// 停等发生在收口/恢复路径,重入恢复由此续接。
pub(crate) fn park_window_recovery(
    store: &ReviewerWindowStore,
    record: &ReviewerWindowRecord,
    reason: &str,
    detail: &str,
) -> ReviewerWindowState {
    let mut next = record.clone();
    next.note_recovery(RecoveryPhase::Restore, reason, detail, None);
    next.bump_transition(
        ReviewerWindowState::RecoveryRequired,
        "补偿停等(静止未证明/未知 writer;租约与冻结通道保留)",
    );
    let expected = record.generation;
    match store.save_window_transition(next, expected) {
        Ok(saved) => saved.state,
        Err(_) => record.state,
    }
}

/// 租约冲突归因(同树 vs 共享 common-dir 兄弟树;审查硬约束 2 的
/// 协调面:冲突身份纳入锁集,报可诊断 detail)。
fn lease_conflict_detail(
    store: &ReviewerWindowStore,
    lease: &WindowLeaseRecord,
    error: &ProductStoreError,
) -> String {
    if let Ok(Some(existing)) = store.load_lease(&lease.tree_key)
        && existing.window_id != lease.window_id
    {
        if existing.canonical_worktree != lease.canonical_worktree {
            return format!(
                "共享 common-dir 的树 {} 已被窗口 {} (attempt={}) 持有",
                existing.canonical_worktree, existing.window_id, existing.attempt_id
            );
        }
        return format!(
            "同树已被窗口 {} (attempt={}) 持有",
            existing.window_id, existing.attempt_id
        );
    }
    if let Ok(leases) = store.list_leases()
        && let Some(other) = leases.iter().find(|other| {
            other.common_dir == lease.common_dir && other.window_id != lease.window_id
        })
    {
        return format!(
            "共享 common-dir 的树 {} 已被窗口 {} (attempt={}) 持有",
            other.canonical_worktree, other.window_id, other.attempt_id
        );
    }
    error.to_string()
}

// ---------------------------------------------------------------------------
// 同源复验(gateway/spawn/材料采集守卫)
// ---------------------------------------------------------------------------

/// 同源复验当前窗口:重载 durable 记录,核对期望身份(attempt/run/
/// provider + worktree 现场重解析 canonical/gitdir/common-dir 均不得
/// 漂移)与状态;`Running` 时额外经 snapshot store 重载 baseline manifest
/// 并核验内容指纹(规范化 JSON 序列化 sha256)与绑定一致(材料同源
/// 证明)。gateway/spawn 用
/// `allowed=[Running]`;平台写通道守卫走 `enter_platform_write`(同一
/// durable 闸门文件,同源)。
pub(crate) fn assert_review_window_current(
    store: &ReviewerWindowStore,
    snapshot: &ReviewerSnapshotStore,
    expectation: &WindowExpectation<'_>,
    allowed_states: &[ReviewerWindowState],
) -> Result<CurrentWindowProof, ReviewerWindowError> {
    let not_current = |reason: String| ReviewerWindowError::WindowNotCurrent {
        window_id: expectation.window_id.to_string(),
        reason,
    };
    let record = store
        .load_window(expectation.window_id)
        .map_err(|error| match error {
            ProductStoreError::NotFound { .. } => not_current(format!("窗口记录不存在: {error}")),
            other => ReviewerWindowError::Store(other),
        })?;
    if record.attempt_id != expectation.attempt_id
        || record.run_id != expectation.run_id
        || record.provider != expectation.provider
    {
        return Err(not_current(
            "attempt/run/provider 与窗口记录不符".to_string(),
        ));
    }
    let canonical = expectation
        .worktree
        .canonicalize()
        .map_err(|error| not_current(format!("worktree 不可解析: {error}")))?;
    if canonical != record.identity.canonical_worktree {
        return Err(not_current(format!(
            "worktree 漂移: {} != {}",
            canonical.display(),
            record.identity.canonical_worktree.display()
        )));
    }
    let (_, gitdir, common_dir) = resolve_worktree_git_identity(&canonical)
        .map_err(|error| not_current(format!("git 身份解析失败: {error}")))?;
    if gitdir != record.identity.gitdir || common_dir != record.identity.common_dir {
        return Err(not_current("gitdir/common-dir 漂移".to_string()));
    }
    if !allowed_states.contains(&record.state) {
        return Err(not_current(format!(
            "窗口状态 {} 不在允许集",
            record.state.as_str()
        )));
    }
    let (manifest_path, manifest_sha256, entry_count, total_bytes) =
        if record.state == ReviewerWindowState::Running {
            // 材料同源证明:Running 窗口的材料必须来自同一 manifest——
            // 经 snapshot store 重载并计算内容指纹(manifest 规范化 JSON
            // 序列化 sha256),与绑定指纹一致;被改/被删即拒。内容寻址
            // 与 F4 采用路径(绝对路径未必可得)同源可复核。
            let baseline = record
                .baseline
                .as_ref()
                .ok_or_else(|| not_current("Running 无 baseline 绑定".to_string()))?;
            let manifest = snapshot
                .load_manifest(expectation.window_id)
                .map_err(|error| not_current(format!("baseline manifest 重载失败: {error}")))?;
            let digest = manifest_fingerprint(&manifest);
            if digest != baseline.manifest_sha256 {
                return Err(not_current(
                    "manifest 指纹不符——材料不再与 baseline 同源".to_string(),
                ));
            }
            if manifest.entries.len() != baseline.entry_count {
                return Err(not_current("manifest 条目数与绑定不符".to_string()));
            }
            (
                PathBuf::from(&baseline.manifest_path),
                digest,
                baseline.entry_count,
                baseline.total_bytes,
            )
        } else {
            (PathBuf::new(), String::new(), 0, 0)
        };
    Ok(CurrentWindowProof {
        window_id: record.window_id,
        generation: record.generation,
        state: record.state,
        report_acceptance: record.report_acceptance,
        manifest_path,
        manifest_sha256,
        entry_count,
        total_bytes,
    })
}

// ---------------------------------------------------------------------------
// 平台写通道守卫(T19 接线消费;与窗口同源)
// ---------------------------------------------------------------------------

/// 平台 git mutation(stage/commit/push/补偿)准入:窗口冻结期间拒绝
pub(crate) fn enter_platform_write(
    store: &ReviewerWindowStore,
    worktree: &Path,
    channel: crate::product::reviewer_window_store::WriteChannelKind,
    writer: ChannelWriter,
) -> Result<(), ReviewerWindowError> {
    let (_, _, common_dir) = resolve_worktree_git_identity(worktree)?;
    store
        .enter_write_channel(&common_dir, channel, writer)
        .map_err(ReviewerWindowError::Channel)
}

pub(crate) fn exit_platform_write(
    store: &ReviewerWindowStore,
    worktree: &Path,
    channel: crate::product::reviewer_window_store::WriteChannelKind,
    writer_id: &str,
) -> Result<(), ReviewerWindowError> {
    let (_, _, common_dir) = resolve_worktree_git_identity(worktree)?;
    store
        .exit_write_channel(&common_dir, channel, writer_id)
        .map_err(ReviewerWindowError::Channel)
}

fn invalid_transition(
    from: ReviewerWindowState,
    to: ReviewerWindowState,
    detail: &str,
) -> ReviewerWindowError {
    ReviewerWindowError::InvalidTransition {
        from: from.as_str().to_string(),
        to: to.as_str().to_string(),
        detail: detail.to_string(),
    }
}

/// T16 恢复阶段 → 稳定字符串(T18 游标交接用)。
fn restore_phase_str(phase: crate::product::reviewer_snapshot::RestorePhase) -> &'static str {
    use crate::product::reviewer_snapshot::RestorePhase;
    match phase {
        RestorePhase::Started => "started",
        RestorePhase::WorktreeRestore => "worktree_restore",
        RestorePhase::WorktreeExtra => "worktree_extra",
        RestorePhase::GitDomain => "git_domain",
        RestorePhase::PlatformDomain => "platform_domain",
        RestorePhase::Completed => "completed",
    }
}

// ---------------------------------------------------------------------------
// 状态机迁移(全部 CAS + 状态合法性守卫;journal 单调追加)
// ---------------------------------------------------------------------------

fn load_for_transition(
    store: &ReviewerWindowStore,
    window_id: &str,
    expected_generation: u64,
) -> Result<ReviewerWindowRecord, ReviewerWindowError> {
    let record = store.load_window(window_id)?;
    if record.generation != expected_generation {
        return Err(ReviewerWindowError::Conflict {
            kind: "reviewer_window_generation",
            id: window_id.to_string(),
            detail: format!(
                "expected={expected_generation} actual={}",
                record.generation
            ),
        });
    }
    Ok(record)
}

/// Running → Quiescing(窗口收口:材料面停,等违规判定/恢复)。
pub(crate) fn begin_quiesce(
    store: &ReviewerWindowStore,
    window_id: &str,
    expected_generation: u64,
    note: &str,
) -> Result<ReviewerWindowRecord, ReviewerWindowError> {
    let mut record = load_for_transition(store, window_id, expected_generation)?;
    if record.state != ReviewerWindowState::Running {
        return Err(invalid_transition(
            record.state,
            ReviewerWindowState::Quiescing,
            "仅 Running 可进入 Quiescing",
        ));
    }
    record.bump_transition(ReviewerWindowState::Quiescing, note);
    Ok(store.save_window_transition(record, expected_generation)?)
}

/// Quiescing → ViolationRecorded(违规事实已落 journal;T18 驱动)。
///
/// 落盘顺序(fail-closed):
/// 1. durable 停等标记 `violation_pending`(先声明「违规已观测」);
/// 2. journal 追加(单调 seq,原子写 + 目录 fsync;`entry_id` 内容
///    哈希幂等去重——同一违规重复收口只落一条);
/// 3. 清除停等标记并迁移状态(Quiescing → ViolationRecorded;
///    ViolationRecorded 下追加更多违规不迁移)。
///
/// 步骤 2 失败 → 返回 Err 且标记保留:`restore_window` /
/// `record_report_acceptance(Accepted)` / `close_accepted` 全部拒绝
/// (「违规已观测但 journal 写失败」不得开始恢复),重入本函数按
/// `entry_id` 幂等续接。条目同时快照 T16 持久化恢复游标
/// (`restore_cursor`):恢复开始前 journal 即含续接位置;恢复路径
/// 只追加、不删除/覆盖 journal。
#[cfg_attr(not(test), allow(dead_code))] // journal-first 违规落账:生产观察方接线位;合同先由收口/恢复测试钉住
pub(crate) fn record_violation(
    store: &ReviewerWindowStore,
    snapshot: &ReviewerSnapshotStore,
    window_id: &str,
    expected_generation: u64,
    violation: ViolationRecord<'_>,
) -> Result<ReviewerWindowRecord, ReviewerWindowError> {
    let mut record = load_for_transition(store, window_id, expected_generation)?;
    match record.state {
        ReviewerWindowState::Quiescing | ReviewerWindowState::ViolationRecorded => {}
        other => {
            return Err(invalid_transition(
                other,
                ReviewerWindowState::ViolationRecorded,
                "仅 Quiescing/ViolationRecorded 可记录违规",
            ));
        }
    }
    let cursor = snapshot_cursor_string(snapshot, window_id);
    let entry_id = violation_entry_id(
        window_id,
        violation.kind,
        violation.path,
        violation.before_digest,
        violation.after_digest,
        violation.detail,
        violation.report_id,
    );
    let journal = store.load_violations(window_id)?;
    let journal_has_entry = journal.iter().any(|entry| entry.entry_id == entry_id);
    // P3-4(修复轮 2):journal 已含 pending.entry_id → 陈旧标记(落盘
    // 后清除前崩溃残留)先清,再处理新违规,不误挡。
    if record.violation_pending.as_ref().is_some_and(|pending| {
        journal
            .iter()
            .any(|entry| entry.entry_id == pending.entry_id)
    }) {
        record.violation_pending = None;
        record = store.save_window_transition(record, expected_generation)?;
    }
    let pending_retry = record
        .violation_pending
        .as_ref()
        .is_some_and(|pending| pending.entry_id == entry_id);
    // F2(审查修复轮 1):pending 非空且不属本 entry → fail-closed 拒绝,
    // 不得覆盖他人停等标记;仅本 entry(重试或已落 journal)可推进/清除。
    if let Some(pending) = record.violation_pending.as_ref()
        && !pending_retry
        && !journal_has_entry
    {
        return Err(ReviewerWindowError::ViolationJournalPending {
            window_id: window_id.to_string(),
            detail: format!(
                "pending={}(kind={}) 与新违规 {}({}) 不符,不得覆盖",
                pending.entry_id, pending.kind, violation.kind, violation.path
            ),
        });
    }
    if !journal_has_entry && !pending_retry {
        record.violation_pending = Some(ViolationPending {
            entry_id: entry_id.clone(),
            kind: violation.kind.to_string(),
            detail: violation.detail.to_string(),
        });
        let expected = record.generation;
        record = store.save_window_transition(record, expected)?;
    }
    let mut durable = journal_has_entry;
    if !journal_has_entry {
        let entry = ViolationJournalEntry {
            seq: 0,
            entry_id,
            at: now_rfc3339(),
            window_id: window_id.to_string(),
            attempt_id: record.attempt_id.clone(),
            run_id: record.run_id.clone(),
            report_id: violation.report_id.map(str::to_string),
            kind: violation.kind.to_string(),
            path: violation.path.to_string(),
            before_digest: violation.before_digest.map(str::to_string),
            after_digest: violation.after_digest.map(str::to_string),
            detail: violation.detail.to_string(),
            restore_cursor: cursor,
        };
        store.append_violation(window_id, entry)?;
        durable = true;
    }
    // journal 已 durable:清**本 entry 的**停等标记(F2:他人 pending 保留,
    // 由其自身重试路径清除)+ 迁移。
    if pending_retry || durable {
        record.violation_pending = None;
    }
    let next_state = if record.state == ReviewerWindowState::Quiescing {
        ReviewerWindowState::ViolationRecorded
    } else {
        record.state
    };
    record.bump_transition(
        next_state,
        &format!(
            "违规落盘 journal: kind={} path={}",
            violation.kind, violation.path
        ),
    );
    let expected = record.generation;
    Ok(store.save_window_transition(record, expected)?)
}

/// 恢复(Quiescing[无违规]/ViolationRecorded/恢复停等重入 → Restoring;
/// T16 差异恢复+游标续接)。结局:
/// - `Restored`:无违规 → 记录 `restore.completed`(等 `close_accepted`);
///   有违规 → ManualGate。
/// - T16 `RecoveryRequired`:窗口 → `RecoveryRequired`(锁保留,停等;
///   原因/游标入记录,可重入本函数续接)。
/// - T16 `Err`(Io/Store/Integrity):**状态不动**(仍在 Restoring),
///   `last_error` 记录并原样返回——T18 对账计入非通过并带出游标
///   (审查硬约束 4 的接口约定)。
///
/// T18 违规收口(恢复成功也不能洗白):
/// - 入口见 `violation_pending` → 拒绝开始恢复(journal 未落盘);
/// - 恢复完成后从 `RestoreReport` 派生 Git 域违规事实(自提交对象/
///   HEAD/index/受管 ref 漂移)+ 可观察 journal 篡改/洗白形态,
///   **先落 journal 再迁移 ManualGate**(journal-first;派生条目
///   `entry_id` 不含恢复游标,重试幂等不重复落条);
/// - 派生落盘失败 → Err 且 `violation_pending` 保留(不得采信关闭),
///   重入本函数按 `entry_id` 幂等续接。
pub(crate) async fn restore_window(
    store: &ReviewerWindowStore,
    snapshot: &ReviewerSnapshotStore,
    window_id: &str,
    expected_generation: u64,
) -> Result<RestoreSummary, ReviewerWindowError> {
    let mut record = load_for_transition(store, window_id, expected_generation)?;
    // T18:「违规已观测但 journal 未落盘」必须不开始恢复(fail-closed)。
    // 例外:Restoring 态的 pending 是恢复后派生收口的崩溃残留——重入
    // 本函数幂等续接(entry_id 去重追加后清除),否则永久死锁。
    if let Some(pending) = record.violation_pending.as_ref()
        && record.state != ReviewerWindowState::Restoring
    {
        return Err(ReviewerWindowError::ViolationJournalPending {
            window_id: window_id.to_string(),
            detail: format!("kind={} entry={}", pending.kind, pending.entry_id),
        });
    }
    // F1(修复轮 1):违规标记不得因恢复重入丢失——入口同时看当前状态
    // 与已持久化 RestoreBinding(停等重入时 state=RecoveryRequired,
    // 违规事实只在 binding 里),防止违规窗被洗白为可采信关闭。
    let from_violation = record.state == ReviewerWindowState::ViolationRecorded
        || record
            .restore
            .as_ref()
            .is_some_and(|binding| binding.from_violation);
    // Restoring 重入 = 瞬时 Err 后的恢复重试(T16 游标幂等续接)。
    let resumable = matches!(
        record.state,
        ReviewerWindowState::Quiescing
            | ReviewerWindowState::ViolationRecorded
            | ReviewerWindowState::Restoring
    ) || (record.state == ReviewerWindowState::RecoveryRequired
        && record.baseline.is_some()
        && record
            .recovery
            .as_ref()
            .is_some_and(|note| note.phase == RecoveryPhase::Restore));
    if !resumable {
        return Err(invalid_transition(
            record.state,
            ReviewerWindowState::Restoring,
            "仅 Quiescing/ViolationRecorded/Restoring 重试/恢复停等(带 baseline)可进入恢复",
        ));
    }
    record.restore = Some(RestoreBinding {
        completed: false,
        from_violation,
        outcome: "in_progress".to_string(),
        cursor_phase: None,
        entry_index: None,
        detail: String::new(),
        git_restored_mutation_paths: Vec::new(),
        git_extra_refs_observed: Vec::new(),
        platform_verified: Vec::new(),
    });
    // P1(修复轮 2):恢复前观察——restore_baseline 回写之前对当前仓
    // 独立观察(index-only 直写等回写后不可再观的漂移在此可见),
    // 与 in_progress 绑定同笔持久化;Restored/重入均消费,journal 落盘
    // 后清除。manifest 不可读 → fail-closed 观察事实(P3-3)。
    // 重入合并:保留既有持久化事实(此前入口观察到的漂移可能已被上次
    // 部分恢复回写),按 entry_id 去重并入本次新观察——不覆盖、不丢事实。
    let mut pre_restore = std::mem::take(&mut record.pre_restore_git_facts);
    let existing_pre: std::collections::HashSet<String> = pre_restore
        .iter()
        .map(|entry| entry.entry_id.clone())
        .collect();
    let fresh: Vec<ViolationJournalEntry> = match snapshot.load_manifest(window_id) {
        Ok(manifest) => observed_git_violation_facts(&record, &manifest)
            .into_iter()
            .map(|fact| pre_restore_entry(&record, &fact))
            .collect(),
        Err(error) => vec![ViolationJournalEntry {
            seq: 0,
            entry_id: violation_entry_id(
                window_id,
                "git_observation_unreadable",
                ".git[manifest]",
                None,
                None,
                &error.to_string(),
                None,
            ),
            at: now_rfc3339(),
            window_id: window_id.to_string(),
            attempt_id: record.attempt_id.clone(),
            run_id: record.run_id.clone(),
            report_id: None,
            kind: "git_observation_unreadable".to_string(),
            path: ".git[manifest]".to_string(),
            before_digest: None,
            after_digest: None,
            detail: format!("恢复前观察无法加载 manifest(fail-closed): {error}"),
            restore_cursor: snapshot_cursor_string(snapshot, window_id),
        }],
    };
    for entry in fresh {
        if !existing_pre.contains(&entry.entry_id) {
            pre_restore.push(entry);
        }
    }
    record.pre_restore_git_facts = pre_restore;
    record.last_error = None;
    record.bump_transition(ReviewerWindowState::Restoring, "T16 差异恢复开始(游标续接)");
    record = store.save_window_transition(record, expected_generation)?;
    let expected_next = record.generation;

    match snapshot.restore_baseline(window_id, &record.identity) {
        Ok(RestoreOutcome::Restored(report)) => {
            // T18:恢复成功 ≠ 可采信。先收集违规事实(journal-first),
            // 再决定 Restoring(待采信)还是 ManualGate(整轮作废)。
            let journal = store.load_violations(window_id)?;
            let mut facts = derived_violation_facts(&record, &report, &journal);
            // F1(修复轮 1):并入恢复后独立观察;P1(修复轮 2):并入
            // **恢复前持久化观察**——T16 回写后不可再观的漂移(index-only
            // 直写/RecoveryRequired 停等重入/崩溃续接)全部由此续接,
            // entry_id 去重不重复落 journal。
            match snapshot.load_manifest(window_id) {
                Ok(manifest) => {
                    facts.extend(observed_git_violation_facts(&record, &manifest));
                }
                Err(error) => {
                    // 顺带(修复轮 3):不再静默吞——统一转 fail-closed 事实。
                    facts.push(DerivedViolationFact {
                        kind: "git_observation_unreadable".to_string(),
                        path: ".git[manifest]".to_string(),
                        before_digest: None,
                        after_digest: None,
                        detail: format!("恢复后观察无法加载 manifest(fail-closed): {error}"),
                    });
                }
            }
            facts.extend(
                record
                    .pre_restore_git_facts
                    .iter()
                    .map(|entry| DerivedViolationFact {
                        kind: entry.kind.clone(),
                        path: entry.path.clone(),
                        before_digest: entry.before_digest.clone(),
                        after_digest: entry.after_digest.clone(),
                        detail: entry.detail.clone(),
                    }),
            );
            // P2-a(修复轮 2):journal/pending 存在即 gate——无可复观
            // 事实时不再静默放行,防止窗口永久卡 Restoring 后被采信。
            let gate = from_violation
                || !facts.is_empty()
                || !journal.is_empty()
                || record.violation_pending.is_some();
            let binding = restored_binding(gate, &report);
            if gate {
                // journal-first:停等标记(首条 entry_id)→ 逐条追加 →
                // 清标记并迁移 ManualGate。任一步失败 → Err + 标记保留,
                // 重入本函数幂等续接(entry_id 去重)。
                let cursor = format!(
                    "{}+{}",
                    restore_phase_str(report.resumed_from.phase),
                    report.resumed_from.entry_index
                );
                let first_id = facts
                    .first()
                    .map(|fact| {
                        violation_entry_id(
                            window_id,
                            &fact.kind,
                            &fact.path,
                            fact.before_digest.as_deref(),
                            fact.after_digest.as_deref(),
                            &fact.detail,
                            None,
                        )
                    })
                    .unwrap_or_else(|| "journal-empty".to_string());
                if record.violation_pending.is_none() && !facts.is_empty() {
                    record.violation_pending = Some(ViolationPending {
                        entry_id: first_id,
                        kind: facts
                            .first()
                            .map(|fact| fact.kind.to_string())
                            .unwrap_or_else(|| "journal_tamper".to_string()),
                        detail: "恢复后违规收口:journal 落盘前停等".to_string(),
                    });
                    record = store.save_window_transition(record, expected_next)?;
                }
                // 既有停等标记与本次首条不一致(先前 journal 写失败续接):
                // 保留原标记占位,追加完成后统一清除。
                let existing_ids: std::collections::HashSet<String> = store
                    .load_violations(window_id)?
                    .into_iter()
                    .map(|entry| entry.entry_id)
                    .collect();
                for fact in &facts {
                    let entry_id = violation_entry_id(
                        window_id,
                        &fact.kind,
                        &fact.path,
                        fact.before_digest.as_deref(),
                        fact.after_digest.as_deref(),
                        &fact.detail,
                        None,
                    );
                    if existing_ids.contains(&entry_id) {
                        continue;
                    }
                    let entry = ViolationJournalEntry {
                        seq: 0,
                        entry_id,
                        at: now_rfc3339(),
                        window_id: window_id.to_string(),
                        attempt_id: record.attempt_id.clone(),
                        run_id: record.run_id.clone(),
                        report_id: None,
                        kind: fact.kind.to_string(),
                        path: fact.path.clone(),
                        before_digest: fact.before_digest.clone(),
                        after_digest: fact.after_digest.clone(),
                        detail: fact.detail.clone(),
                        restore_cursor: cursor.clone(),
                    };
                    store.append_violation(window_id, entry)?;
                }
                // 顺带(修复轮 3):仅因 pending/journal 非空而 gate、且
                // 无任何可落事实时,把 pending 违规以占位条目写入 journal
                // ——否则清标记后 from_violation 绑定真而 journal 空,
                // reconcile 会误报 journal_tamper。
                if let Some(pending) = record.violation_pending.as_ref()
                    && facts.is_empty()
                {
                    // B3(修复轮 4):占位幂等——崩溃于 append 后保存前,
                    // 重入时 journal 已含占位(entry_id 键不同,须显式查)。
                    let placeholder_id = violation_entry_id(
                        window_id,
                        &pending.kind,
                        &format!("pending:{}", pending.entry_id),
                        None,
                        None,
                        &pending.detail,
                        None,
                    );
                    if !existing_ids.contains(&placeholder_id) {
                        let entry = ViolationJournalEntry {
                            seq: 0,
                            entry_id: placeholder_id,
                            at: now_rfc3339(),
                            window_id: window_id.to_string(),
                            attempt_id: record.attempt_id.clone(),
                            run_id: record.run_id.clone(),
                            report_id: None,
                            kind: pending.kind.clone(),
                            path: format!("pending:{}", pending.entry_id),
                            before_digest: None,
                            after_digest: None,
                            detail: format!(
                                "违规已观测但无可复观事实(停等标记占位落盘): {}",
                                pending.detail
                            ),
                            restore_cursor: cursor.clone(),
                        };
                        store.append_violation(window_id, entry)?;
                    }
                }
                record.violation_pending = None;
                record.pre_restore_git_facts = Vec::new();
                record.restore = Some(binding);
                record.bump_transition(
                    ReviewerWindowState::ManualGate,
                    "违规流:恢复完整+违规事实落 journal,进入人工门(仅作废关闭/作废后新轮)",
                );
                let manual_gate = true;
                let expected_final = record.generation;
                record = store.save_window_transition(record, expected_final)?;
                return Ok(RestoreSummary {
                    record,
                    manual_gate,
                });
            }
            record.restore = Some(binding);
            record.bump_transition(ReviewerWindowState::Restoring, "恢复完整,待采信裁决后关闭");
            let manual_gate = false;
            record = store.save_window_transition(record, expected_next)?;
            Ok(RestoreSummary {
                record,
                manual_gate,
            })
        }
        Ok(RestoreOutcome::RecoveryRequired {
            reason,
            detail,
            cursor,
        }) => {
            // T16 域内无法安全恢复 → 窗口停等(锁保留);游标入记录,
            // 处置后可重入本函数续接。
            let cursor_phase = restore_phase_str(cursor.phase).to_string();
            record.restore = Some(RestoreBinding {
                completed: false,
                from_violation,
                outcome: "recovery_required".to_string(),
                cursor_phase: Some(cursor_phase.clone()),
                entry_index: Some(cursor.entry_index),
                detail: detail.clone(),
                git_restored_mutation_paths: Vec::new(),
                git_extra_refs_observed: Vec::new(),
                platform_verified: Vec::new(),
            });
            record.note_recovery(
                RecoveryPhase::Restore,
                &reason,
                &detail,
                Some(cursor_phase.clone()),
            );
            record.bump_transition(
                ReviewerWindowState::RecoveryRequired,
                "T16 恢复停等(游标已记录;锁保留)",
            );
            let stopped = ReviewerWindowError::RecoveryStopped {
                reason,
                detail,
                cursor: format!("{cursor_phase}+{}", cursor.entry_index),
            };
            if let Err(error) = store.save_window_transition(record, expected_next) {
                return Err(ReviewerWindowError::Store(error));
            }
            Err(stopped)
        }
        Err(error) => {
            // 审查硬约束 4:瞬时 store 错误(Io/Store/Integrity)不迁移
            // 窗口状态——T18 对账计入非通过并从 restore.cursor 带出游标。
            // F3(修复轮 1):best-effort 读回 T16 持久化恢复游标回填
            // 绑定,使「计入非通过并带出游标」三处接口约定真实可达。
            record.last_error = Some(error.to_string());
            let persisted_cursor = snapshot
                .load_manifest(window_id)
                .ok()
                .map(|manifest| manifest.restore_cursor);
            record.restore = Some(RestoreBinding {
                completed: false,
                from_violation,
                outcome: "error".to_string(),
                cursor_phase: persisted_cursor
                    .as_ref()
                    .map(|cursor| restore_phase_str(cursor.phase).to_string()),
                entry_index: persisted_cursor.map(|cursor| cursor.entry_index),
                detail: error.to_string(),
                git_restored_mutation_paths: Vec::new(),
                git_extra_refs_observed: Vec::new(),
                platform_verified: Vec::new(),
            });
            let _ = store.save_window_transition(record, expected_next);
            Err(ReviewerWindowError::Snapshot(error))
        }
    }
}

/// 恢复完成后的结构化绑定(三域事实入记录;T18 对账消费)。
fn restored_binding(from_violation: bool, report: &RestoreReport) -> RestoreBinding {
    RestoreBinding {
        completed: true,
        from_violation,
        outcome: "restored".to_string(),
        cursor_phase: Some(restore_phase_str(report.resumed_from.phase).to_string()),
        entry_index: Some(report.resumed_from.entry_index),
        detail: format!(
            "worktree_fixed={} removed_extras={} git_head={} git_index={} refs_restored={} extra_refs_observed={} platform_verified={}",
            report.worktree_fixed_entries.len(),
            report.worktree_removed_extras.len(),
            report.git_head_restored as u8,
            report.git_index_restored as u8,
            report.git_refs_restored.len(),
            report.git_extra_refs_observed.len(),
            report.platform_verified.len()
        ),
        git_restored_mutation_paths: git_mutation_paths(report),
        git_extra_refs_observed: report.git_extra_refs_observed.clone(),
        platform_verified: report.platform_verified.clone(),
    }
}

/// 恢复报告派生的违规事实(冻结窗口内 Git 域漂移/自提交对象/可观察
/// journal 篡改或洗白形态)。外部推送类外部副作用已在 kind/detail
/// 中明示不可回滚——不伪造「交付零外部影响」。
#[derive(Debug)]
pub(crate) struct DerivedViolationFact {
    pub(crate) kind: String,
    pub(crate) path: String,
    pub(crate) before_digest: Option<String>,
    pub(crate) after_digest: Option<String>,
    pub(crate) detail: String,
}

fn derived_violation_facts(
    record: &ReviewerWindowRecord,
    report: &RestoreReport,
    journal: &[ViolationJournalEntry],
) -> Vec<DerivedViolationFact> {
    let mut facts = Vec::new();
    for path in git_mutation_paths(report) {
        facts.push(DerivedViolationFact {
            kind: "git_domain_mutation".to_string(),
            path: path.clone(),
            before_digest: None,
            after_digest: None,
            detail: format!(
                "冻结窗口内 Git 域漂移({path}),恢复已回写基线;改 Git 整轮作废。外部 push 类副作用不可回滚,不伪称交付零外部影响"
            ),
        });
    }
    for extra in &report.git_extra_refs_observed {
        facts.push(DerivedViolationFact {
            kind: "git_extra_ref".to_string(),
            path: extra.clone(),
            before_digest: None,
            after_digest: None,
            detail: format!(
                "窗口内新增 ref {extra}(Reviewer 自提交对象证据保留,不抹共享数据);整轮作废"
            ),
        });
    }
    // 可观察 journal 完整性(fail-closed;不承诺同 UID 恶意重写整文件
    // 必被检测——R8 边界):序号断裂/窗号不符 → 篡改;from_violation
    // 但 journal 空 → 篡改;journal 非空但绑定未标违规 → 洗白形态。
    let integrity_ok = journal_integrity_ok(journal, &record.window_id);
    if !integrity_ok {
        facts.push(DerivedViolationFact {
            kind: "journal_tamper".to_string(),
            path: format!("violations/{}.json", record.window_id),
            before_digest: None,
            after_digest: None,
            detail: format!(
                "违规 journal 序号断裂或窗号不符(观察到 {} 条,连续={})",
                journal.len(),
                journal_windows_contiguous(journal)
            ),
        });
    }
    if record
        .restore
        .as_ref()
        .is_some_and(|binding| binding.from_violation)
        && journal.is_empty()
    {
        facts.push(DerivedViolationFact {
            kind: "journal_tamper".to_string(),
            path: format!("violations/{}.json", record.window_id),
            before_digest: None,
            after_digest: None,
            detail: "from_violation 绑定为真但 journal 为空(可观察缺失,fail-closed 作废)"
                .to_string(),
        });
    }
    // F3(审查修复轮 1):本窗重入态(pending 残留=append 部分成功后
    // 崩溃)不算洗白——已落条目来自本窗派生收口,非外部篡改。
    if record.violation_pending.is_none()
        && !record
            .restore
            .as_ref()
            .is_some_and(|binding| binding.from_violation)
        && record.state != ReviewerWindowState::ViolationRecorded
        && !journal.is_empty()
    {
        facts.push(DerivedViolationFact {
            kind: "journal_tamper".to_string(),
            path: format!("violations/{}.json", record.window_id),
            before_digest: None,
            after_digest: None,
            detail: format!(
                "journal 非空({} 条)但绑定未标记违规(洗白形态,fail-closed 作废)",
                journal.len()
            ),
        });
    }
    facts
}

/// F1/F4(审查修复轮 1):恢复后**独立只读 Git 观察**——不依赖 T16
/// RestoreReport 的内存事实(Completed 游标重入会跳过恢复块导致报告
/// 全空,派生事实丢失),而是每次 `Restored`(含重入)直接对当前仓
/// 重观察:extra refs(自提交对象留仓仍可见)/HEAD/受管 ref/index/
/// config/objects/reflog/子模块 gitdir vs manifest 基线。观察失败
/// fail-closed 作为 drift 事实(停等处理,不伪称通过)。
pub(crate) fn observed_git_violation_facts(
    record: &ReviewerWindowRecord,
    manifest: &crate::product::reviewer_snapshot::ReviewerBaselineManifest,
) -> Vec<DerivedViolationFact> {
    let worktree = record.identity.canonical_worktree.clone();
    let mut facts = Vec::new();
    // extra refs:当前 refs - 受管 refs(自提交证据保留,重入仍可见)。
    let refs_observation = list_current_refs(&worktree);
    if let Ok(current) = refs_observation.as_ref() {
        for (path, _) in current {
            if !manifest
                .git_observation
                .managed_refs
                .iter()
                .any(|managed| managed.ref_path == *path)
            {
                facts.push(DerivedViolationFact {
                    kind: "git_extra_ref".to_string(),
                    path: path.clone(),
                    before_digest: None,
                    after_digest: None,
                    detail: format!(
                        "窗口内新增 ref {path}(Reviewer 自提交对象证据保留,不抹共享数据);整轮作废"
                    ),
                });
            }
        }
        // 受管 ref 当前 sha vs 基线(恢复后仍漂移=窗口内 mutation)。
        for managed in &manifest.git_observation.managed_refs {
            if let Some((_, sha)) = current.iter().find(|(path, _)| path == &managed.ref_path)
                && sha != &managed.sha
            {
                facts.push(DerivedViolationFact {
                    kind: "git_domain_mutation".to_string(),
                    path: managed.ref_path.clone(),
                    before_digest: Some(managed.sha.clone()),
                    after_digest: Some(sha.clone()),
                    detail: format!(
                        "受管 ref {} 漂移(基线 {} -> 当前 {sha}),改 Git 整轮作废",
                        managed.ref_path, managed.sha
                    ),
                });
            }
        }
    } else if let Err(error) = refs_observation.as_ref() {
        // P3-3(修复轮 2):观察失败 fail-closed 计事实,不再静默吞。
        facts.push(DerivedViolationFact {
            kind: "git_observation_unreadable".to_string(),
            path: ".git[refs]".to_string(),
            before_digest: None,
            after_digest: None,
            detail: format!("refs 观察不可读(fail-closed): {error}"),
        });
    }
    // HEAD:当前 symref/sha vs 基线。
    let head_observation = git_stdout_trimmed(&worktree, &["rev-parse", "HEAD"]);
    if let Ok(head_sha) = head_observation.as_deref() {
        if head_sha != manifest.git_observation.head_sha {
            facts.push(DerivedViolationFact {
                kind: "git_domain_mutation".to_string(),
                path: ".git/HEAD".to_string(),
                before_digest: Some(manifest.git_observation.head_sha.clone()),
                after_digest: Some(head_sha.to_string()),
                detail: "HEAD 指向漂移(改 Git 整轮作废)".to_string(),
            });
        }
    } else if let Err(error) = head_observation.as_ref() {
        // P3-3:观察失败 fail-closed 计事实。
        facts.push(DerivedViolationFact {
            kind: "git_observation_unreadable".to_string(),
            path: ".git[HEAD]".to_string(),
            before_digest: None,
            after_digest: None,
            detail: format!("HEAD 观察不可读(fail-closed): {error}"),
        });
    }
    // index 漂移判定(N4 修复轮 3):语义内容摘要(`git ls-files --stage`)
    // 比对——stat-refresh 不改 OID/路径/模式故不误伤;`git add`(OID 变)
    // 仍检出。raw 字节仅由 T16 用于恢复回写,不作违规判定。旧 manifest
    // 无语义摘要(None)→ 该面跳过(不把旧窗口当漂移)。
    if let Some(expected) = manifest.git_observation.index_semantic_sha256.as_ref() {
        match crate::product::reviewer_snapshot::index_semantic_digest(&worktree) {
            Ok(current) => {
                if current != *expected {
                    facts.push(DerivedViolationFact {
                        kind: "git_domain_mutation".to_string(),
                        path: ".git/index".to_string(),
                        before_digest: Some(expected.clone()),
                        after_digest: Some(current),
                        detail: "index 语义内容漂移(模式/OID/路径;改 Git 整轮作废)".to_string(),
                    });
                }
            }
            Err(error) => {
                // P3-3:观察失败 fail-closed 计事实。
                facts.push(DerivedViolationFact {
                    kind: "git_observation_unreadable".to_string(),
                    path: ".git[index]".to_string(),
                    before_digest: None,
                    after_digest: None,
                    detail: format!("index 语义摘要不可观察(fail-closed): {error}"),
                });
            }
        }
    }
    // F4:annex 补充面(config/objects/reflog/子模块 gitdir)。
    if let Some(baseline) = manifest.git_annex_observation.as_ref() {
        match crate::product::reviewer_snapshot::observe_git_annex(
            &record.identity.gitdir,
            &record.identity.common_dir,
        ) {
            Ok(current) => {
                let comparisons = [
                    (
                        "gitdir_config",
                        &baseline.config_sha256,
                        &current.config_sha256,
                    ),
                    (
                        "common_dir_config",
                        &baseline.common_config_sha256,
                        &current.common_config_sha256,
                    ),
                    ("objects", &baseline.objects_digest, &current.objects_digest),
                    ("reflogs", &baseline.reflogs_digest, &current.reflogs_digest),
                    // N2(修复轮 3):config.worktree 面此前采集从不比对,
                    // 现入对比表;旧 manifest 字段缺失反序列化为空串 →
                    // 跳过(N3),不把旧窗口当漂移。
                    (
                        "worktree_config",
                        &baseline.worktree_config_sha256,
                        &current.worktree_config_sha256,
                    ),
                    (
                        "submodule_gitdirs",
                        &baseline.submodule_gitdirs_digest,
                        &current.submodule_gitdirs_digest,
                    ),
                ];
                for (face, before, after) in comparisons {
                    if !before.is_empty() && before != after {
                        facts.push(DerivedViolationFact {
                            kind: "git_annex_drift".to_string(),
                            path: format!(".git[{face}]"),
                            before_digest: Some(before.clone()),
                            after_digest: Some(after.clone()),
                            detail: format!(
                                "Git 观察域补充面 {face} 摘要漂移(直写 .git 逃逸检测);整轮作废"
                            ),
                        });
                    }
                }
            }
            Err(error) => {
                facts.push(DerivedViolationFact {
                    kind: "git_annex_drift".to_string(),
                    path: ".git[annex]".to_string(),
                    before_digest: None,
                    after_digest: None,
                    detail: format!("Git 观察域补充面不可观察(fail-closed): {error}"),
                });
            }
        }
    }
    facts
}

fn list_current_refs(worktree: &Path) -> Result<Vec<(String, String)>, String> {
    let output = std::process::Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
            "refs/tags",
        ])
        .current_dir(worktree)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (path, sha) = line.split_once(' ')?;
            Some((path.to_string(), sha.to_string()))
        })
        .collect())
}

fn git_stdout_trimmed(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// P1(修复轮 2):恢复前观察事实 → 可持久化 journal 条目形态(与
/// in_progress 绑定同笔保存;Restored 消费时转回事实并入,entry_id
/// 输入一致故去重稳定)。
fn pre_restore_entry(
    record: &ReviewerWindowRecord,
    fact: &DerivedViolationFact,
) -> ViolationJournalEntry {
    ViolationJournalEntry {
        seq: 0,
        entry_id: violation_entry_id(
            &record.window_id,
            &fact.kind,
            &fact.path,
            fact.before_digest.as_deref(),
            fact.after_digest.as_deref(),
            &fact.detail,
            None,
        ),
        at: now_rfc3339(),
        window_id: record.window_id.clone(),
        attempt_id: record.attempt_id.clone(),
        run_id: record.run_id.clone(),
        report_id: None,
        kind: fact.kind.clone(),
        path: fact.path.clone(),
        before_digest: fact.before_digest.clone(),
        after_digest: fact.after_digest.clone(),
        detail: fact.detail.clone(),
        restore_cursor: "started+0".to_string(),
    }
}

/// journal 序号从 1 连续且窗号全部相符。
fn journal_integrity_ok(entries: &[ViolationJournalEntry], window_id: &str) -> bool {
    entries
        .iter()
        .enumerate()
        .all(|(index, entry)| entry.seq == (index + 1) as u64 && entry.window_id == window_id)
}

fn journal_windows_contiguous(entries: &[ViolationJournalEntry]) -> bool {
    entries
        .iter()
        .enumerate()
        .all(|(index, entry)| entry.seq == (index + 1) as u64)
}

/// 冻结窗口内被恢复动作覆盖的 Git 域路径(HEAD/index/受管 ref)。
fn git_mutation_paths(report: &RestoreReport) -> Vec<String> {
    let mut paths = Vec::new();
    if report.git_head_restored {
        paths.push(".git/HEAD".to_string());
    }
    if report.git_index_restored {
        paths.push(".git/index".to_string());
    }
    paths.extend(report.git_refs_restored.iter().cloned());
    paths
}

/// T16 持久化恢复游标的稳定串(journal 记录用;manifest 不可读时回退
/// `started+0`——恢复未开始/无法取证时不伪造续接位置)。
fn snapshot_cursor_string(snapshot: &ReviewerSnapshotStore, window_id: &str) -> String {
    snapshot
        .load_manifest(window_id)
        .ok()
        .map(|manifest| {
            format!(
                "{}+{}",
                restore_phase_str(manifest.restore_cursor.phase),
                manifest.restore_cursor.entry_index
            )
        })
        .unwrap_or_else(|| "started+0".to_string())
}

/// 违规条目幂等键:事实字段的 sha256(**不含**恢复游标/时间——同一
/// 违规在不同游标位置重复收口仍只落一条)。
#[allow(clippy::too_many_arguments)]
fn violation_entry_id(
    window_id: &str,
    kind: &str,
    path: &str,
    before_digest: Option<&str>,
    after_digest: Option<&str>,
    detail: &str,
    report_id: Option<&str>,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in [
        window_id.as_bytes(),
        kind.as_bytes(),
        path.as_bytes(),
        before_digest.unwrap_or("-").as_bytes(),
        after_digest.unwrap_or("-").as_bytes(),
        detail.as_bytes(),
        report_id.unwrap_or("-").as_bytes(),
    ] {
        hasher.update(part);
        hasher.update([0u8]);
    }
    hex::encode(hasher.finalize())
}

/// 报告采信投影记录(T18 主导;数据级 CAS 写,不改状态)。
///
/// 守卫(RevB2T17 ⚠4 处置 + brief):
/// - Preparing/Running 拒绝(新窗口报告必须先 Pending 再受信裁决);
/// - **终态拒绝**(旧报告永久定格,人工不能把旧报告改 Accepted);
/// - `Accepted` 仅在 Restoring+恢复完整+无违规+journal 干净+同源
///   baseline 指纹一致时可记(指纹=规范化 JSON 内容指纹,经
///   `load_manifest`+`manifest_fingerprint` 同源计算,不按文件字节复算);
/// - `Invalidated` 从 Quiescing 起任意非终态可记(违规/人工作废流)。
pub(crate) fn record_report_acceptance(
    store: &ReviewerWindowStore,
    snapshot: &ReviewerSnapshotStore,
    window_id: &str,
    expected_generation: u64,
    acceptance: ReportAcceptance,
) -> Result<ReviewerWindowRecord, ReviewerWindowError> {
    let mut record = load_for_transition(store, window_id, expected_generation)?;
    if matches!(
        record.state,
        ReviewerWindowState::Preparing | ReviewerWindowState::Running
    ) {
        return Err(invalid_transition(
            record.state,
            record.state,
            "采信投影仅 Quiescing 起(T18 采信流)记录",
        ));
    }
    if record.state.is_terminal() {
        return Err(invalid_transition(
            record.state,
            record.state,
            "终态不可改采信投影(旧报告永久定格,人工不能改旧报告)",
        ));
    }
    if acceptance == ReportAcceptance::Accepted {
        let binding = record.restore.as_ref();
        let eligible = record.state == ReviewerWindowState::Restoring
            && binding.is_some_and(|binding| {
                binding.completed && !binding.from_violation && binding.outcome == "restored"
            })
            && record.violation_pending.is_none();
        if !eligible {
            return Err(invalid_transition(
                record.state,
                record.state,
                "Accepted 仅限 Restoring+恢复完整+无违规+journal 已清停等",
            ));
        }
        let journal = store.load_violations(window_id)?;
        if !journal.is_empty() {
            return Err(invalid_transition(
                record.state,
                record.state,
                "违规 journal 非空(违规流只能 Invalidated/人工门)",
            ));
        }
        if let Some(binding) = binding
            && (!binding.git_restored_mutation_paths.is_empty()
                || !binding.git_extra_refs_observed.is_empty())
        {
            return Err(invalid_transition(
                record.state,
                record.state,
                "恢复事实含 Git 域漂移(自提交/改 Git 整轮作废)",
            ));
        }
        // 同源指纹核验(审查硬约束 3):manifest 经 load_manifest 加载,
        // 指纹=规范化 JSON 序列化 sha256,不得按文件字节复算。
        let manifest = snapshot.load_manifest(window_id)?;
        let baseline = record.baseline.as_ref().ok_or_else(|| {
            invalid_transition(
                record.state,
                record.state,
                "Restoring 记录缺 baseline 绑定(记录不一致)",
            )
        })?;
        if manifest_fingerprint(&manifest) != baseline.manifest_sha256 {
            return Err(invalid_transition(
                record.state,
                record.state,
                "baseline 指纹不一致(证据链同源核验失败,fail-closed)",
            ));
        }
    }
    record.report_acceptance = acceptance;
    record.bump_transition(record.state, "报告采信投影记录(T18)");
    Ok(store.save_window_transition(record, expected_generation)?)
}

/// 终态释放(先解冻通道+释放租约,再落终态记录;释放幂等,中间崩溃
/// 重入即收口)。仅终态进入调用。
fn finalize_terminal(
    store: &ReviewerWindowStore,
    record: &ReviewerWindowRecord,
    terminal: ReviewerWindowState,
    note: &str,
    expected_generation: u64,
) -> Result<ReviewerWindowRecord, ReviewerWindowError> {
    // 释放顺序:解冻四通道 → 释放租约 → 落终态。任何一步失败即返回
    // (记录停在非终态,重入幂等收口);只有全部释放成功才 Closed*。
    // F5(修复轮 1):只释放本窗**实际持有**的闸门/租约——租约冲突等
    // 未持锁失败窗的 abandon 不得被持有人闸门/租约 OwnerMismatch 卡死。
    for channel in ALL_WRITE_CHANNELS {
        if store.channel_owned_by(&record.identity.common_dir, channel, &record.window_id)? {
            store
                .unfreeze_channel(&record.identity.common_dir, channel, &record.window_id)
                .map_err(|error| map_channel_gate_error(&record.window_id, error))?;
        }
    }
    let tree_key = tree_key_of(&record.identity.canonical_worktree);
    if let Some(lease) = store.load_lease(&tree_key)?
        && lease.window_id == record.window_id
    {
        store.release_lease(&record.window_id, &tree_key)?;
    }
    let mut final_record = record.clone();
    final_record.bump_transition(terminal, note);
    Ok(store.save_window_transition(final_record, expected_generation)?)
}

/// 采信关闭:Restoring(恢复完整,无违规)+ 报告采信 Accepted →
/// ClosedAccepted(锁释放)。
pub(crate) fn close_accepted(
    store: &ReviewerWindowStore,
    window_id: &str,
    expected_generation: u64,
) -> Result<ReviewerWindowRecord, ReviewerWindowError> {
    let record = load_for_transition(store, window_id, expected_generation)?;
    let restore = record.restore.as_ref();
    let completed = restore.is_some_and(|binding| binding.completed);
    let from_violation = restore.is_some_and(|binding| binding.from_violation);
    if record.state != ReviewerWindowState::Restoring || !completed || from_violation {
        return Err(invalid_transition(
            record.state,
            ReviewerWindowState::ClosedAccepted,
            "需 Restoring 且恢复完整且无违规(违规流只能走人工门)",
        ));
    }
    if record.report_acceptance != ReportAcceptance::Accepted {
        return Err(invalid_transition(
            record.state,
            ReviewerWindowState::ClosedAccepted,
            "需先经 T18 采信流记录 Accepted",
        ));
    }
    // T18 防御:采信关闭前 journal 必须干净、停等标记必须清除
    // (晚到的违规 journal/未落盘标记均 fail-closed 拒绝)。
    if record.violation_pending.is_some() {
        return Err(invalid_transition(
            record.state,
            ReviewerWindowState::ClosedAccepted,
            "违规 journal 未落盘(violation_pending 停等)",
        ));
    }
    let journal = store.load_violations(window_id)?;
    if !journal.is_empty() {
        return Err(invalid_transition(
            record.state,
            ReviewerWindowState::ClosedAccepted,
            "违规 journal 非空(整轮作废,走人工门)",
        ));
    }
    finalize_terminal(
        store,
        &record,
        ReviewerWindowState::ClosedAccepted,
        "恢复完整+采信完成;通道解冻、租约释放",
        expected_generation,
    )
}

/// 人工门裁决:ManualGate → ClosedInvalidated | NewReviewRequired
/// (锁释放;旧报告永久 Invalidated,新轮重开新窗口)。
pub(crate) fn resolve_manual_gate(
    store: &ReviewerWindowStore,
    window_id: &str,
    expected_generation: u64,
    decision: ManualGateDecision,
) -> Result<ReviewerWindowRecord, ReviewerWindowError> {
    let mut record = load_for_transition(store, window_id, expected_generation)?;
    if record.state != ReviewerWindowState::ManualGate {
        return Err(invalid_transition(
            record.state,
            ReviewerWindowState::ClosedInvalidated,
            "仅 ManualGate 可人工裁决",
        ));
    }
    let (terminal, note) = match decision {
        ManualGateDecision::Invalidate => (
            ReviewerWindowState::ClosedInvalidated,
            "人工门:作废关闭;旧报告永久 Invalidated",
        ),
        ManualGateDecision::RequireNewReview => (
            ReviewerWindowState::NewReviewRequired,
            "人工门:作废并要求新一轮评审(新窗口重加锁/新 baseline)",
        ),
    };
    // RevB2T17 ⚠4:人工门关闭即写 Invalidated(与「旧报告永久
    // Invalidated」一致;人工不能把旧报告改 Accepted——终态拒绝)。
    record.report_acceptance = ReportAcceptance::Invalidated;
    finalize_terminal(store, &record, terminal, note, expected_generation)
}

/// 停等/卡死窗口的人工弃置(Preparing/RecoveryRequired/ManualGate/
/// Restoring → ClosedInvalidated,锁释放)。Running 窗口必须先走
/// Quiescing 恢复流(取消也保持锁到恢复完整)。作废即旧报告永久
/// Invalidated。
pub(crate) fn abandon_window(
    store: &ReviewerWindowStore,
    window_id: &str,
    expected_generation: u64,
    reason: &str,
) -> Result<ReviewerWindowRecord, ReviewerWindowError> {
    let mut record = load_for_transition(store, window_id, expected_generation)?;
    match record.state {
        ReviewerWindowState::Preparing
        | ReviewerWindowState::RecoveryRequired
        | ReviewerWindowState::ManualGate
        | ReviewerWindowState::Restoring => {}
        other => {
            return Err(invalid_transition(
                other,
                ReviewerWindowState::ClosedInvalidated,
                "Running 须先 Quiescing/恢复;仅停等/人工门/恢复中/Preparing 可弃置",
            ));
        }
    }
    record.report_acceptance = ReportAcceptance::Invalidated;
    finalize_terminal(
        store,
        &record,
        ReviewerWindowState::ClosedInvalidated,
        &format!("人工弃置: {reason}"),
        expected_generation,
    )
}

// ---------------------------------------------------------------------------
// 内测
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_resources::{
        OwnedEngineObserver, OwnedResourceSpec, ResourceScopeKey, StartedResourceIdentity,
        new_boot_identity,
    };
    use crate::product::reviewer_window_store::WriteChannelKind;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    // ---------- fixtures ----------

    fn t17_dir() -> tempfile::TempDir {
        let root = Path::new("/tmp/aria-bug45/t17");
        fs::create_dir_all(root).expect("create /tmp/aria-bug45/t17");
        tempfile::tempdir_in(root).expect("t17 fixture dir")
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} in {} failed: {}",
            args,
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    struct WindowFixture {
        worktree: PathBuf,
        window_root: PathBuf,
        snapshot_root: PathBuf,
        base: PathBuf,
        _keep: Vec<tempfile::TempDir>,
    }

    impl WindowFixture {
        fn store(&self) -> ReviewerWindowStore {
            ReviewerWindowStore::new(&self.window_root)
        }

        fn snapshot(&self) -> ReviewerSnapshotStore {
            ReviewerSnapshotStore::new(&self.snapshot_root)
        }

        fn common_dir(&self) -> PathBuf {
            resolve_worktree_git_identity(&self.worktree)
                .expect("resolve identity")
                .2
        }
    }

    fn init_repo(worktree: &Path) {
        fs::write(worktree.join("tracked.txt"), "base\n").unwrap();
        git_ok(worktree, &["init", "-q", "-b", "main"]);
        git_ok(worktree, &["config", "user.email", "t17@aria.invalid"]);
        git_ok(worktree, &["config", "user.name", "t17-fixture"]);
        git_ok(worktree, &["add", "-A"]);
        git_ok(worktree, &["commit", "-qm", "base"]);
    }

    fn window_fixture() -> WindowFixture {
        let base = t17_dir();
        let win = t17_dir();
        let snap = t17_dir();
        let worktree = base.path().join("repo");
        fs::create_dir_all(&worktree).unwrap();
        init_repo(&worktree);
        WindowFixture {
            worktree,
            window_root: win.path().to_path_buf(),
            snapshot_root: snap.path().to_path_buf(),
            base: base.path().to_path_buf(),
            _keep: vec![base, win, snap],
        }
    }

    /// 主仓 + linked worktree(共享 common-dir 的兄弟树)。
    fn linked_fixture() -> (WindowFixture, PathBuf) {
        let fx = window_fixture();
        let linked = fx.base.join("linked");
        let linked_arg = linked.display().to_string();
        git_ok(&fx.worktree, &["worktree", "add", "-q", &linked_arg]);
        fx.worktree.canonicalize().unwrap();
        (fx, linked)
    }

    fn open_scope(
        base: &Path,
        scope_id: &str,
        engine: Option<Arc<dyn OwnedEngineObserver>>,
    ) -> ProviderRunResourceScope {
        let store_root = base.join("scope-store");
        let private = base.join("private").join(scope_id);
        fs::create_dir_all(&private).unwrap();
        let key = ResourceScopeKey {
            scope_id: scope_id.to_string(),
            run_id: format!("run-{scope_id}"),
            boot_identity: new_boot_identity(),
        };
        ProviderRunResourceScope::open_for_recovery(
            &key,
            Some("w-test"),
            &private,
            &store_root,
            engine,
        )
        .expect("open scope")
    }

    /// fake 引擎观察者:stop 声称成功但容器仍在运行(killpg 成功≠静止)。
    struct StopOkRunningObserver;

    impl OwnedEngineObserver for StopOkRunningObserver {
        fn observe_running(
            &self,
            _kind: crate::cross_cutting::provider_resources::OwnedResourceKind,
            _exact_id: &str,
        ) -> Result<bool, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
            Ok(true)
        }

        fn find_by_reserved_labels(
            &self,
            _kind: crate::cross_cutting::provider_resources::OwnedResourceKind,
            _labels: &std::collections::BTreeMap<String, String>,
        ) -> Result<Vec<String>, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            Ok(Vec::new())
        }

        fn stop_exact(
            &self,
            _kind: crate::cross_cutting::provider_resources::OwnedResourceKind,
            _exact_id: &str,
            _labels: &std::collections::BTreeMap<String, String>,
        ) -> Result<bool, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
            Ok(true)
        }

        fn remove_exact(
            &self,
            _kind: crate::cross_cutting::provider_resources::OwnedResourceKind,
            _exact_id: &str,
            _labels: &std::collections::BTreeMap<String, String>,
        ) -> Result<bool, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
            Ok(false)
        }
    }

    fn begin_request<'a>(
        window_id: &'a str,
        attempt: &'a str,
        worktree: &'a Path,
        targets: &'a [&'a ProviderRunResourceScope],
    ) -> ReviewerWindowBeginRequest<'a> {
        ReviewerWindowBeginRequest {
            window_id,
            attempt_id: attempt,
            run_id: "run-1",
            provider: "claude_code",
            worktree,
            quiesce_targets: targets,
            platform_write_paths: Vec::new(),
            budget: SnapshotBudget::generous(),
            drain_timeout: Duration::from_secs(5),
        }
    }

    fn expectation<'a>(
        window_id: &'a str,
        attempt: &'a str,
        worktree: &'a Path,
    ) -> WindowExpectation<'a> {
        WindowExpectation {
            window_id,
            attempt_id: attempt,
            run_id: "run-1",
            provider: "claude_code",
            worktree,
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
            entered_at: now_rfc3339(),
        }
    }

    /// 材料采集/ReviewerVerify spawn 的同源闸门 harness:闸门放行才计数。
    #[derive(Default)]
    struct GatedCounts {
        material: usize,
        spawn: usize,
    }

    impl GatedCounts {
        fn try_material_and_spawn(
            &mut self,
            store: &ReviewerWindowStore,
            snapshot: &ReviewerSnapshotStore,
            exp: &WindowExpectation<'_>,
        ) {
            if assert_review_window_current(store, snapshot, exp, &[ReviewerWindowState::Running])
                .is_ok()
            {
                self.material += 1;
                self.spawn += 1;
            }
        }
    }

    /// 完整采信收口(Running → Quiescing → Restoring → Accepted →
    /// ClosedAccepted);返回终态记录。
    async fn close_accepted_flow(
        fx: &WindowFixture,
        window_id: &str,
    ) -> Result<ReviewerWindowRecord, ReviewerWindowError> {
        let store = fx.store();
        let snapshot = fx.snapshot();
        let quiescing = begin_quiesce(&store, window_id, 2, "cancel")?;
        let summary = restore_window(&store, &snapshot, window_id, quiescing.generation).await?;
        assert!(!summary.manual_gate);
        let accepted = record_report_acceptance(
            &store,
            &snapshot,
            window_id,
            summary.record.generation,
            ReportAcceptance::Accepted,
        )?;
        close_accepted(&store, window_id, accepted.generation)
    }

    fn futures_block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    // ---------- Step 1 红测矩阵 ----------

    #[tokio::test]
    async fn two_attempts_same_tree_only_one_window() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];

        let w1 = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-1", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("first window opens");
        assert_eq!(
            store.load_window("w-1").unwrap().state,
            ReviewerWindowState::Running
        );

        // 同树第二 attempt:不能因 attempt_id 不同而放行
        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-2", "attempt-2", &fx.worktree, &targets),
        )
        .await
        .expect_err("same tree second attempt must conflict");
        assert!(
            matches!(&err, ReviewerWindowError::Conflict { kind, .. } if *kind == "reviewer_window_lease"),
            "{err:?}"
        );
        let w2_record = store.load_window("w-2").unwrap();
        assert_eq!(w2_record.state, ReviewerWindowState::RecoveryRequired);
        // 失败开窗保留可诊断恢复状态,不扰动现有窗口
        assert_eq!(
            store.load_window("w-1").unwrap().state,
            ReviewerWindowState::Running
        );
        // 闸门:w1 放行,w2 拒绝
        assert!(
            assert_review_window_current(
                &store,
                &snapshot,
                &expectation("w-1", "attempt-1", &fx.worktree),
                &[ReviewerWindowState::Running]
            )
            .is_ok()
        );
        assert!(
            assert_review_window_current(
                &store,
                &snapshot,
                &expectation("w-2", "attempt-2", &fx.worktree),
                &[ReviewerWindowState::Running]
            )
            .is_err()
        );

        // 无关树可开(租约按树)
        let other = window_fixture();
        let other_scope = open_scope(&other.base, "coder", None);
        let other_targets: [&ProviderRunResourceScope; 1] = [&other_scope];
        begin_review_window(
            &other.store(),
            &other.snapshot(),
            &begin_request("w-other", "attempt-1", &other.worktree, &other_targets),
        )
        .await
        .expect("unrelated tree opens");
        let _ = w1;
    }

    #[tokio::test]
    async fn inflight_channel_writer_barrier_blocks_material_until_drained() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let common = fx.common_dir();

        // 在途 git mutation writer 先进入通道
        store
            .enter_write_channel(
                &common,
                WriteChannelKind::Commit,
                own_writer("git-writer-1"),
            )
            .expect("writer enters open channel");

        let begin_store = fx.store();
        let begin_snapshot = fx.snapshot();
        let worktree = fx.worktree.clone();
        let base = fx.base.clone();
        let begin = std::thread::spawn(move || {
            let scope = open_scope(&base, "coder", None);
            let targets: [&ProviderRunResourceScope; 1] = [&scope];
            futures_block_on(begin_review_window(
                &begin_store,
                &begin_snapshot,
                &begin_request("w-bar", "attempt-1", &worktree, &targets),
            ))
        });

        // 排空屏障:新 writer 被拒 = Draining 已置(baseline 前写通道已关;
        // 有界条件等待,不靠 sleep 竞速断言)
        let mut draining = false;
        for i in 0..2000 {
            let probe_id = format!("probe-{i}");
            match store.enter_write_channel(
                &common,
                WriteChannelKind::Commit,
                own_writer(&probe_id),
            ) {
                Err(ChannelGateError::Blocked { .. }) => {
                    draining = true;
                    break;
                }
                Ok(()) => {
                    store
                        .exit_write_channel(&common, WriteChannelKind::Commit, &probe_id)
                        .unwrap();
                }
                Err(other) => panic!("unexpected channel error: {other:?}"),
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(draining, "drain must engage before baseline");

        // 屏障点:在途 writer 未退出 → snapshot/material/spawn 计数全 0
        assert!(
            !fx.snapshot_root.join("windows/w-bar").exists(),
            "no snapshot window dir while writer in flight"
        );
        let mut counts = GatedCounts::default();
        counts.try_material_and_spawn(
            &store,
            &snapshot,
            &expectation("w-bar", "attempt-1", &fx.worktree),
        );
        assert_eq!(counts.material, 0, "material count must be 0");
        assert_eq!(counts.spawn, 0, "spawn count must be 0");

        // 在途 writer 退出(等待已有 writer 退出,而非只禁新请求)
        store
            .exit_write_channel(&common, WriteChannelKind::Commit, "git-writer-1")
            .expect("exit while draining");

        let handle = begin
            .join()
            .expect("join begin thread")
            .expect("begin completes after drain");
        assert_eq!(handle.window_id, "w-bar");
        let record = store.load_window("w-bar").unwrap();
        assert_eq!(record.state, ReviewerWindowState::Running);

        // 材料闸门放行;材料与 manifest 同指纹(同一材料源)
        counts.try_material_and_spawn(
            &store,
            &snapshot,
            &expectation("w-bar", "attempt-1", &fx.worktree),
        );
        assert_eq!(counts.material, 1);
        assert_eq!(counts.spawn, 1);
        let manifest = snapshot.load_manifest("w-bar").unwrap();
        assert_eq!(manifest_fingerprint(&manifest), handle.manifest_sha256);
        assert_eq!(manifest.entries.len(), handle.entry_count);

        // 窗口期间新 git mutation 仍拒绝
        let err = store
            .enter_write_channel(&common, WriteChannelKind::Commit, own_writer("late"))
            .expect_err("frozen during window");
        assert!(matches!(err, ChannelGateError::Blocked { .. }));
    }

    #[tokio::test]
    async fn unconfirmed_writer_waits_no_capture_no_material() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        // 未知 daemon:预留未确认(崩溃窗口)——是否已 spawn 不可证
        let scope = open_scope(&fx.base, "coder", None);
        scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "coder-1",
                crate::cross_cutting::provider_resources::OwnedResourceKind::Process,
                "claude-code session",
            ))
            .expect("reserve only");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];

        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-unconf", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect_err("unconfirmed writer must stop-and-wait");
        match &err {
            ReviewerWindowError::QuiesceNotProven { reason, .. } => {
                assert!(reason.contains("unconfirmed"), "{reason}");
            }
            other => panic!("expected QuiesceNotProven, got {other:?}"),
        }
        let record = store.load_window("w-unconf").unwrap();
        assert_eq!(record.state, ReviewerWindowState::RecoveryRequired);
        assert!(!fx.snapshot_root.join("windows/w-unconf").exists());
        let mut counts = GatedCounts::default();
        counts.try_material_and_spawn(
            &store,
            &snapshot,
            &expectation("w-unconf", "attempt-1", &fx.worktree),
        );
        assert_eq!((counts.material, counts.spawn), (0, 0));
        // 停等窗口持锁:同树新窗口仍拒绝
        let empty = open_scope(&fx.base, "coder-2", None);
        let empty_targets: [&ProviderRunResourceScope; 1] = [&empty];
        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-after", "attempt-9", &fx.worktree, &empty_targets),
        )
        .await
        .expect_err("lease held while recovery pending");
        assert!(
            matches!(err, ReviewerWindowError::Conflict { .. }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn container_stop_ok_but_still_running_waits() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        // killpg/stop 声称成功但登记容器仍在写——T5 观察语义否决静止
        let scope = open_scope(&fx.base, "coder", Some(Arc::new(StopOkRunningObserver)));
        let reservation = scope
            .reserve_owned_resource(&OwnedResourceSpec::new(
                "builder-1",
                crate::cross_cutting::provider_resources::OwnedResourceKind::Container,
                "podman build helper",
            ))
            .expect("reserve container");
        scope
            .confirm_started_resource(
                &reservation,
                StartedResourceIdentity::container("ctr-exact-1"),
            )
            .expect("confirm container");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];

        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-ctr", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect_err("container still running must stop-and-wait");
        match &err {
            ReviewerWindowError::QuiesceNotProven { reason, .. } => {
                assert!(reason.contains("registered_resource_running"), "{reason}");
            }
            other => panic!("expected QuiesceNotProven, got {other:?}"),
        }
        assert!(!fx.snapshot_root.join("windows/w-ctr").exists());
        assert_eq!(
            store.load_window("w-ctr").unwrap().state,
            ReviewerWindowState::RecoveryRequired
        );
    }

    #[tokio::test]
    async fn quiesce_store_failure_waits() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        // 登记失败面:scope store 损坏(list_scope Err)——停等,不伪称静止
        let scope_root = fx.base.join("broken-scope-store");
        fs::write(&scope_root, "not-a-dir").unwrap();
        let private = fx.base.join("private/broken");
        fs::create_dir_all(&private).unwrap();
        let key = ResourceScopeKey {
            scope_id: "broken".into(),
            run_id: "run-broken".into(),
            boot_identity: new_boot_identity(),
        };
        let scope =
            ProviderRunResourceScope::open_for_recovery(&key, None, &private, &scope_root, None)
                .expect("open broken-store scope");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];

        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-broken", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect_err("quiesce store failure must stop-and-wait");
        assert!(
            matches!(err, ReviewerWindowError::QuiesceFailed(_)),
            "{err:?}"
        );
        assert_eq!(
            store.load_window("w-broken").unwrap().state,
            ReviewerWindowState::RecoveryRequired
        );
        assert!(!fx.snapshot_root.join("windows/w-broken").exists());
    }

    #[tokio::test]
    async fn snapshot_failure_no_material_no_spawn() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        // 快照失败:非 UTF-8 文件名不支持项 → capture 整单拒绝 → 不采
        // 材料不 spawn(裁定乙后 FIFO 属 git 不可见域外,不再触发拒绝)
        use std::os::unix::ffi::OsStringExt as _;
        let bad_name = std::ffi::OsString::from_vec(vec![0x88, 0x99, 0xaa]);
        std::fs::write(fx.worktree.join(bad_name), "x\n").unwrap();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];

        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-snapfail", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect_err("snapshot failure must stop-and-wait");
        assert!(matches!(err, ReviewerWindowError::Snapshot(_)), "{err:?}");
        let record = store.load_window("w-snapfail").unwrap();
        assert_eq!(record.state, ReviewerWindowState::RecoveryRequired);
        // capture 整单拒绝:无已发布 manifest
        assert!(
            !fx.snapshot_root
                .join("windows/w-snapfail/manifest.json")
                .exists()
        );
        let mut counts = GatedCounts::default();
        counts.try_material_and_spawn(
            &store,
            &snapshot,
            &expectation("w-snapfail", "attempt-1", &fx.worktree),
        );
        assert_eq!((counts.material, counts.spawn), (0, 0));
    }

    #[test]
    fn preparing_window_material_rejected() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        // 崩溃于开窗中途:Preparing 记录(无 Running/baseline)→ 材料拒绝
        let (canonical, gitdir, common) = resolve_worktree_git_identity(&fx.worktree).unwrap();
        let record = ReviewerWindowRecord::new(
            "w-prep",
            ReviewWorktreeLockIdentity {
                canonical_worktree: canonical,
                gitdir,
                common_dir: common,
                lease_id: "wlease-w-prep".into(),
            },
            "attempt-1",
            "run-1",
            "claude_code",
            Vec::new(),
        );
        store.create_or_takeover_window(&record).unwrap();
        let mut counts = GatedCounts::default();
        counts.try_material_and_spawn(
            &store,
            &snapshot,
            &expectation("w-prep", "attempt-1", &fx.worktree),
        );
        assert_eq!((counts.material, counts.spawn), (0, 0));
    }

    #[tokio::test]
    async fn lease_and_journal_survive_store_rebuild() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-rebuild", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");

        // 「重启」:store/snapshot 实例全部重建(实例无内存态)
        let rebuilt_store = fx.store();
        let rebuilt_snapshot = fx.snapshot();
        let err = begin_review_window(
            &rebuilt_store,
            &rebuilt_snapshot,
            &begin_request("w-rebuild-2", "attempt-2", &fx.worktree, &targets),
        )
        .await
        .expect_err("lease survives rebuild");
        assert!(
            matches!(err, ReviewerWindowError::Conflict { .. }),
            "{err:?}"
        );
        // journal 与材料闸门跨重建有效
        let record = rebuilt_store.load_window("w-rebuild").unwrap();
        assert_eq!(record.state, ReviewerWindowState::Running);
        assert!(record.transitions.iter().any(|entry| entry.to == "running"));
        let proof = assert_review_window_current(
            &rebuilt_store,
            &rebuilt_snapshot,
            &expectation("w-rebuild", "attempt-1", &fx.worktree),
            &[ReviewerWindowState::Running],
        )
        .expect("gate works after rebuild");
        assert_eq!(proof.state, ReviewerWindowState::Running);

        // 完整收口后同树可再开
        let terminal = close_accepted_flow(&fx, "w-rebuild").await.expect("close");
        assert_eq!(terminal.state, ReviewerWindowState::ClosedAccepted);
        begin_review_window(
            &rebuilt_store,
            &rebuilt_snapshot,
            &begin_request("w-rebuild-3", "attempt-3", &fx.worktree, &targets),
        )
        .await
        .expect("tree reopens after accepted close");
    }

    #[tokio::test]
    async fn lease_held_through_cancel_and_restart_until_verdict() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-cancel", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");
        let window_id = handle.window_id.clone();

        // 取消+「重启」:丢句柄、重建 store——租约仍在(fail-closed)
        drop(handle);
        let store2 = fx.store();
        let snapshot2 = fx.snapshot();
        let empty = open_scope(&fx.base, "coder-2", None);
        let empty_targets: [&ProviderRunResourceScope; 1] = [&empty];
        let conflict = begin_review_window(
            &store2,
            &snapshot2,
            &begin_request("w-other", "attempt-2", &fx.worktree, &empty_targets),
        )
        .await
        .expect_err("lease held across restart");
        assert!(matches!(conflict, ReviewerWindowError::Conflict { .. }));

        // Quiescing/Restoring 期间仍持锁
        let quiescing = begin_quiesce(&store2, &window_id, 2, "cancel").unwrap();
        let conflict = begin_review_window(
            &store2,
            &snapshot2,
            &begin_request("w-other", "attempt-2", &fx.worktree, &empty_targets),
        )
        .await
        .expect_err("lease held while quiescing");
        assert!(matches!(conflict, ReviewerWindowError::Conflict { .. }));

        let summary = restore_window(&store2, &snapshot2, &window_id, quiescing.generation)
            .await
            .expect("restore");
        assert!(!summary.manual_gate);
        let conflict = begin_review_window(
            &store2,
            &snapshot2,
            &begin_request("w-other", "attempt-2", &fx.worktree, &empty_targets),
        )
        .await
        .expect_err("lease held until verdict complete");
        assert!(matches!(conflict, ReviewerWindowError::Conflict { .. }));

        // 裁决完成(采信关闭)才释放
        let accepted = record_report_acceptance(
            &store2,
            &snapshot2,
            &window_id,
            summary.record.generation,
            ReportAcceptance::Accepted,
        )
        .unwrap();
        let terminal = close_accepted(&store2, &window_id, accepted.generation).unwrap();
        assert_eq!(terminal.state, ReviewerWindowState::ClosedAccepted);
        begin_review_window(
            &store2,
            &snapshot2,
            &begin_request("w-next", "attempt-3", &fx.worktree, &empty_targets),
        )
        .await
        .expect("lease released after terminal verdict");
    }

    #[tokio::test]
    async fn assert_rejects_identity_mismatch_state_and_tamper() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-assert", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");
        let running = &[ReviewerWindowState::Running];

        // attempt/run/provider 不匹配 → 拒
        for wrong in [
            expectation("w-assert", "attempt-2", &fx.worktree),
            WindowExpectation {
                window_id: "w-assert",
                attempt_id: "attempt-1",
                run_id: "run-2",
                provider: "claude_code",
                worktree: &fx.worktree,
            },
            WindowExpectation {
                window_id: "w-assert",
                attempt_id: "attempt-1",
                run_id: "run-1",
                provider: "codex",
                worktree: &fx.worktree,
            },
        ] {
            assert!(
                assert_review_window_current(&store, &snapshot, &wrong, running).is_err(),
                "identity mismatch must reject"
            );
        }

        // worktree 现场漂移(以另一棵真实树冒充)→ 拒(canonical 不符)
        let other_tree = fx.base.join("other-repo");
        fs::create_dir_all(&other_tree).unwrap();
        init_repo(&other_tree);
        assert!(
            assert_review_window_current(
                &store,
                &snapshot,
                &expectation("w-assert", "attempt-1", &other_tree),
                running
            )
            .is_err(),
            "different worktree must reject (identity drift)"
        );

        // 状态不符(Quiescing 后要求 Running)→ 拒
        begin_quiesce(&store, "w-assert", handle.generation, "cancel").unwrap();
        assert!(
            assert_review_window_current(
                &store,
                &snapshot,
                &expectation("w-assert", "attempt-1", &fx.worktree),
                running
            )
            .is_err(),
            "quiescing window must not admit material"
        );

        // manifest 字节被改 → Running 指纹不符 → 拒(材料不再同源)。
        // 以独立 Running 窗口验证(w-assert 已 Quiescing,不走指纹分支)
        let fx2 = window_fixture();
        let scope2 = open_scope(&fx2.base, "coder", None);
        let targets2: [&ProviderRunResourceScope; 1] = [&scope2];
        let h2 = begin_review_window(
            &fx2.store(),
            &fx2.snapshot(),
            &begin_request("w-tamper", "attempt-1", &fx2.worktree, &targets2),
        )
        .await
        .expect("begin 2");
        let manifest2 = fx2.snapshot_root.join("windows/w-tamper/manifest.json");
        let raw = fs::read_to_string(&manifest2).unwrap();
        // 内容篡改(改条目 rel_path 值):解析仍合法但内容指纹漂移
        assert!(raw.contains("\"tracked.txt\""), "fixture entry present");
        fs::write(
            &manifest2,
            raw.replace("\"tracked.txt\"", "\"tampered.txt\""),
        )
        .unwrap();
        assert!(
            assert_review_window_current(
                &fx2.store(),
                &fx2.snapshot(),
                &expectation("w-tamper", "attempt-1", &fx2.worktree),
                running
            )
            .is_err(),
            "tampered manifest must fail fingerprint"
        );
        let _ = h2;
    }

    #[tokio::test]
    async fn platform_write_paths_must_be_outside_worktree() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];

        let inside = PlatformWritePath {
            path: fx.worktree.join("aria-store/report.json"),
            ownership: "report-store".into(),
        };
        let mut request = begin_request("w-plat", "attempt-1", &fx.worktree, &targets);
        request.platform_write_paths = vec![inside];
        let err = begin_review_window(&store, &snapshot, &request)
            .await
            .expect_err("inside-worktree platform path rejected");
        assert!(
            matches!(err, ReviewerWindowError::InvalidRequest(_)),
            "{err:?}"
        );
        assert!(store.load_window("w-plat").is_err(), "no window created");

        // Git 域也拒绝
        let git_domain = PlatformWritePath {
            path: fx.common_dir().join("config"),
            ownership: "git-config".into(),
        };
        let mut request = begin_request("w-plat", "attempt-1", &fx.worktree, &targets);
        request.platform_write_paths = vec![git_domain];
        assert!(
            begin_review_window(&store, &snapshot, &request)
                .await
                .is_err()
        );

        // worktree 之外的合法平台路径放行,并进入基线
        let outside = PlatformWritePath {
            path: fx.window_root.join("platform/report.json"),
            ownership: "report-store".into(),
        };
        let mut request = begin_request("w-plat", "attempt-1", &fx.worktree, &targets);
        request.platform_write_paths = vec![outside];
        begin_review_window(&store, &snapshot, &request)
            .await
            .expect("outside platform path accepted");
        let manifest = snapshot.load_manifest("w-plat").unwrap();
        assert_eq!(manifest.platform_write_baseline.len(), 1);
    }

    #[tokio::test]
    async fn sibling_linked_worktree_conflict_and_channel_drain() {
        let (fx, linked) = linked_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let common = fx.common_dir();

        // 兄弟树(linked,共享 common-dir)的在途 git writer:经兄弟身份
        // 进入同一 common-dir 通道(闸门按 common-dir 键控)
        store
            .enter_write_channel(
                &common,
                WriteChannelKind::Push,
                own_writer("sibling-writer"),
            )
            .expect("sibling writer enters via common-dir channel");

        let begin_store = fx.store();
        let begin_snapshot = fx.snapshot();
        let worktree = fx.worktree.clone();
        let base = fx.base.clone();
        let begin = std::thread::spawn(move || {
            let scope = open_scope(&base, "coder", None);
            let targets: [&ProviderRunResourceScope; 1] = [&scope];
            futures_block_on(begin_review_window(
                &begin_store,
                &begin_snapshot,
                &begin_request("w-main", "attempt-1", &worktree, &targets),
            ))
        });

        // 兄弟 writer 在途 → 主树窗口不能达成 baseline(排空等待)
        let mut draining = false;
        for i in 0..2000 {
            let probe_id = format!("sib-probe-{i}");
            match store.enter_write_channel(&common, WriteChannelKind::Push, own_writer(&probe_id))
            {
                Err(ChannelGateError::Blocked { .. }) => {
                    draining = true;
                    break;
                }
                Ok(()) => {
                    store
                        .exit_write_channel(&common, WriteChannelKind::Push, &probe_id)
                        .unwrap();
                }
                Err(other) => panic!("{other:?}"),
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(draining, "sibling writer must be drained before baseline");
        assert!(!fx.snapshot_root.join("windows/w-main").exists());
        store
            .exit_write_channel(&common, WriteChannelKind::Push, "sibling-writer")
            .unwrap();
        begin.join().unwrap().expect("main window after drain");

        // 主树窗口持有期间:兄弟树窗口(共享 common-dir)被纳入协调——拒绝
        let linked_scope = open_scope(&fx.base, "coder-2", None);
        let linked_targets: [&ProviderRunResourceScope; 1] = [&linked_scope];
        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-linked", "attempt-1", &linked, &linked_targets),
        )
        .await
        .expect_err("sibling window on shared common-dir must conflict");
        match &err {
            ReviewerWindowError::Conflict { kind, detail, .. } => {
                assert_eq!(*kind, "reviewer_window_lease");
                assert!(detail.contains("common-dir"), "{detail}");
            }
            other => panic!("expected lease conflict, got {other:?}"),
        }
        assert_eq!(
            store.load_window("w-linked").unwrap().state,
            ReviewerWindowState::RecoveryRequired
        );
    }

    #[tokio::test]
    async fn violation_flow_reaches_manual_gate_and_release() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-viol", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");

        // 固定违规流:Running→Quiescing→ViolationRecorded→Restoring→ManualGate
        let quiescing = begin_quiesce(&store, "w-viol", handle.generation, "review end").unwrap();
        let violated = record_violation(
            &store,
            &snapshot,
            "w-viol",
            quiescing.generation,
            ViolationRecord {
                kind: "platform_domain_write",
                path: ".aria/direct-write.json",
                before_digest: None,
                after_digest: Some("sha256:deadbeef"),
                detail: ".aria 直写(平台合法写入域外)",
                report_id: None,
            },
        )
        .unwrap();
        let summary = restore_window(&store, &snapshot, "w-viol", violated.generation)
            .await
            .expect("violation restore");
        assert!(summary.manual_gate, "violation path must gate manually");
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);

        // 恢复成功也不能洗白:人工仅作废/新轮,不能 Accepted
        let err = close_accepted(&store, "w-viol", summary.record.generation)
            .expect_err("manual gate cannot accept");
        assert!(
            matches!(err, ReviewerWindowError::InvalidTransition { .. }),
            "{err:?}"
        );
        // 人工门期间锁仍持有
        let empty = open_scope(&fx.base, "coder-2", None);
        let empty_targets: [&ProviderRunResourceScope; 1] = [&empty];
        assert!(
            begin_review_window(
                &store,
                &snapshot,
                &begin_request("w-after", "attempt-2", &fx.worktree, &empty_targets),
            )
            .await
            .is_err()
        );

        let terminal = resolve_manual_gate(
            &store,
            "w-viol",
            summary.record.generation,
            ManualGateDecision::Invalidate,
        )
        .unwrap();
        assert_eq!(terminal.state, ReviewerWindowState::ClosedInvalidated);
        // 终态释放:同树可开新窗口(新轮重加锁/新 baseline)
        begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-new-round", "attempt-2", &fx.worktree, &empty_targets),
        )
        .await
        .expect("new round after invalidation");
    }

    #[tokio::test]
    async fn invalid_transitions_and_generation_cas_rejected() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-guard", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");

        // Running 直接采信关闭(未恢复)→ 拒
        let err = close_accepted(&store, "w-guard", handle.generation)
            .expect_err("running cannot close accepted");
        assert!(
            matches!(err, ReviewerWindowError::InvalidTransition { .. }),
            "{err:?}"
        );
        // Running 弃置 → 拒(必须先 Quiescing/恢复)
        let err = abandon_window(&store, "w-guard", handle.generation, "nope")
            .expect_err("running cannot abandon");
        assert!(matches!(err, ReviewerWindowError::InvalidTransition { .. }));
        // 陈旧 generation → Conflict
        let quiescing = begin_quiesce(&store, "w-guard", handle.generation, "c").unwrap();
        let err = begin_quiesce(&store, "w-guard", handle.generation, "stale")
            .expect_err("stale generation rejected");
        assert!(
            matches!(err, ReviewerWindowError::Conflict { .. }),
            "{err:?}"
        );
        // Quiescing 直接采信关闭(未恢复)→ 拒
        let err = close_accepted(&store, "w-guard", quiescing.generation)
            .expect_err("must restore before accept");
        assert!(matches!(err, ReviewerWindowError::InvalidTransition { .. }));
        // 恢复未采信就关闭 → 拒
        let summary = restore_window(&store, &snapshot, "w-guard", quiescing.generation)
            .await
            .expect("restore");
        let err = close_accepted(&store, "w-guard", summary.record.generation)
            .expect_err("acceptance must be recorded (T18)");
        assert!(matches!(err, ReviewerWindowError::InvalidTransition { .. }));

        // 开窗失败的 RecoveryRequired 窗口(无 baseline)不能重入恢复。
        // 用独立 fixture:w-guard 仍持本树租约,同树新开只会得到租约冲突。
        let fx2 = window_fixture();
        let broken_root = fx2.base.join("broken2");
        fs::write(&broken_root, "x").unwrap();
        let private2 = fx2.base.join("private/broken2");
        fs::create_dir_all(&private2).unwrap();
        let key2 = ResourceScopeKey {
            scope_id: "b2".into(),
            run_id: "r2".into(),
            boot_identity: new_boot_identity(),
        };
        let broken_scope =
            ProviderRunResourceScope::open_for_recovery(&key2, None, &private2, &broken_root, None)
                .unwrap();
        let broken_targets: [&ProviderRunResourceScope; 1] = [&broken_scope];
        let err = begin_review_window(
            &fx2.store(),
            &fx2.snapshot(),
            &begin_request("w-nb", "attempt-1", &fx2.worktree, &broken_targets),
        )
        .await
        .expect_err("begin failure");
        assert!(matches!(err, ReviewerWindowError::QuiesceFailed(_)));
        let nb = fx2.store().load_window("w-nb").unwrap();
        assert_eq!(nb.state, ReviewerWindowState::RecoveryRequired);
        let err = restore_window(&fx2.store(), &fx2.snapshot(), "w-nb", nb.generation)
            .await
            .expect_err("no-baseline recovery cannot resume into restore");
        assert!(
            matches!(err, ReviewerWindowError::InvalidTransition { .. }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn report_starts_pending_acceptance() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-pend", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");

        let record = store.load_window("w-pend").unwrap();
        assert_eq!(
            record.report_acceptance,
            ReportAcceptance::PendingAcceptance
        );
        let proof = assert_review_window_current(
            &store,
            &snapshot,
            &expectation("w-pend", "attempt-1", &fx.worktree),
            &[ReviewerWindowState::Running],
        )
        .unwrap();
        assert_eq!(proof.report_acceptance, ReportAcceptance::PendingAcceptance);
    }

    #[tokio::test]
    async fn drain_timeout_waits_when_writer_never_exits() {
        // 在途 writer 永不退出 → 冻结超时停等(不伪称静止、不采基线)
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let common = fx.common_dir();
        let mut stuck = own_writer("stuck-writer");
        // 存活但永不过期的"外部进程":以本测试进程身份注册(必存活)
        stuck.writer_id = "stuck-writer".into();
        store
            .enter_write_channel(&common, WriteChannelKind::Commit, stuck)
            .unwrap();

        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let mut request = begin_request("w-stuck", "attempt-1", &fx.worktree, &targets);
        request.drain_timeout = Duration::from_millis(300);
        let err = begin_review_window(&store, &snapshot, &request)
            .await
            .expect_err("stuck writer must time out the drain");
        match &err {
            ReviewerWindowError::ChannelDrainTimeout { detail } => {
                assert!(detail.contains("stuck-writer"), "{detail}");
            }
            other => panic!("expected drain timeout, got {other:?}"),
        }
        assert_eq!(
            store.load_window("w-stuck").unwrap().state,
            ReviewerWindowState::RecoveryRequired
        );
        assert!(!fx.snapshot_root.join("windows/w-stuck").exists());
        // 停等窗口持锁:清理后弃置可释放
        store
            .exit_write_channel(&common, WriteChannelKind::Commit, "stuck-writer")
            .unwrap();
        let rec = store.load_window("w-stuck").unwrap();
        abandon_window(&store, "w-stuck", rec.generation, "drain stuck").unwrap();
    }

    // ---------- 修复轮 1 钉测(F1/F3/F4/F5) ----------

    /// F1(P1):违规流恢复中断(T16 RecoveryRequired)后重入续接,
    /// 违规标记不得丢失——恢复完整仍必须进 ManualGate,不能洗白为
    /// 可采信关闭。
    #[tokio::test]
    async fn violation_recovery_resume_still_reaches_manual_gate() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        // 平台合法写入路径:worktree 之外的独立平台文件
        let platform_dir = fx.base.join("platform");
        fs::create_dir_all(&platform_dir).unwrap();
        let platform_file = platform_dir.join("report.json");
        fs::write(&platform_file, "v0\n").unwrap();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let mut request = begin_request("w-f1", "attempt-1", &fx.worktree, &targets);
        request.platform_write_paths = vec![PlatformWritePath {
            path: platform_file.clone(),
            ownership: "report-store".to_string(),
        }];
        let handle = begin_review_window(&store, &snapshot, &request)
            .await
            .expect("begin");

        // 违规流:收口 + 落违规
        let quiescing = begin_quiesce(&store, "w-f1", handle.generation, "end").unwrap();
        let violated = record_violation(
            &store,
            &snapshot,
            "w-f1",
            quiescing.generation,
            ViolationRecord {
                kind: "platform_domain_write",
                path: ".aria/direct-write.json",
                before_digest: None,
                after_digest: Some("sha256:deadbeef"),
                detail: ".aria 直写(平台合法写入域外)",
                report_id: None,
            },
        )
        .unwrap();

        // Reviewer 残留 + 平台域漂移 → T16 恢复停等(PlatformDomain 游标)
        fs::write(fx.worktree.join("residue.txt"), "junk\n").unwrap();
        fs::write(&platform_file, "v1-tampered\n").unwrap();
        let outcome = restore_window(&store, &snapshot, "w-f1", violated.generation).await;
        match outcome.expect_err("platform drift must stop-and-wait") {
            ReviewerWindowError::RecoveryStopped { reason, cursor, .. } => {
                assert!(reason.contains("platform_domain_drift"), "{reason}");
                assert!(cursor.contains("platform_domain"), "{cursor}");
            }
            other => panic!("expected RecoveryStopped, got {other:?}"),
        }
        let stopped = store.load_window("w-f1").unwrap();
        assert_eq!(stopped.state, ReviewerWindowState::RecoveryRequired);

        // 处置平台漂移(回基线)后重入续接:from_violation 必须从持久化
        // 绑定恢复 → ManualGate(而非可采信关闭)
        fs::write(&platform_file, "v0\n").unwrap();
        let summary = restore_window(&store, &snapshot, "w-f1", stopped.generation)
            .await
            .expect("resume restore completes");
        assert!(
            summary.manual_gate,
            "violation must survive recovery resume"
        );
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);
        let err = close_accepted(&store, "w-f1", summary.record.generation)
            .expect_err("manual gate cannot accept");
        assert!(matches!(err, ReviewerWindowError::InvalidTransition { .. }));
    }

    /// F3(P2):T16 瞬时 Err(Integrity)不迁移状态,但恢复游标必须
    /// best-effort 回填绑定(T18「计入非通过并带出游标」可达)。
    #[tokio::test]
    async fn restore_error_keeps_state_and_backfills_cursor() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-err", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-err", handle.generation, "cancel").unwrap();

        // 抽掉一个 CAS blob → T16 恢复 CAS 预检 Integrity 拒绝(树不动)
        let manifest = snapshot.load_manifest("w-err").unwrap();
        let blob = manifest
            .entries
            .iter()
            .find_map(|entry| entry.blob.clone())
            .expect("fixture has file entries");
        let blob_path = fx.snapshot_root.join("windows/w-err").join(&blob);
        let blob_bytes = fs::read(&blob_path).unwrap();
        fs::remove_file(&blob_path).unwrap();

        let err = restore_window(&store, &snapshot, "w-err", quiescing.generation)
            .await
            .expect_err("CAS precheck must fail");
        assert!(matches!(err, ReviewerWindowError::Snapshot(_)), "{err:?}");
        let record = store.load_window("w-err").unwrap();
        assert_eq!(
            record.state,
            ReviewerWindowState::Restoring,
            "transient Err must not move state"
        );
        assert!(record.last_error.is_some(), "last_error recorded");
        let binding = record.restore.as_ref().expect("binding present");
        assert!(!binding.completed);
        assert_eq!(binding.outcome, "error");
        assert_eq!(
            binding.cursor_phase.as_deref(),
            Some("started"),
            "cursor backfilled from persisted manifest"
        );
        assert_eq!(binding.entry_index, Some(0));

        // Restoring 重入(瞬时 Err 后重试):补回原字节 → 恢复完成
        fs::write(&blob_path, blob_bytes).unwrap();
        let summary = restore_window(&store, &snapshot, "w-err", record.generation)
            .await
            .expect("retry after transient error");
        assert!(!summary.manual_gate);
    }

    /// F4(P2):崩溃于「manifest 已发布、Running CAS 未落」——同窗重入
    /// 采用已发布 manifest(校验归属),不作废最贵一步。
    #[tokio::test]
    async fn crash_after_capture_resumes_by_adopting_manifest() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let first = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-adopt", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("first begin");

        // 模拟崩溃点:窗口记录回 Preparing(manifest 已发布、租约/闸门
        // 仍为本窗持有——create_or_resume 同身份重建即该状态)
        fs::remove_file(fx.window_root.join("windows/w-adopt.json")).unwrap();
        store
            .create_or_takeover_window(&ReviewerWindowRecord::new(
                "w-adopt",
                first.identity.clone(),
                "attempt-1",
                "run-1",
                "claude_code",
                Vec::new(),
            ))
            .expect("recreate preparing record");

        // 同窗重入:capture 见 manifest 冲突 → 归属校验 → 采用 → Running
        let second = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-adopt", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("resume adopts published manifest");
        assert_eq!(second.manifest_sha256, first.manifest_sha256);
        assert_eq!(second.entry_count, first.entry_count);
        assert_eq!(
            store.load_window("w-adopt").unwrap().state,
            ReviewerWindowState::Running
        );
        assert!(
            assert_review_window_current(
                &store,
                &snapshot,
                &expectation("w-adopt", "attempt-1", &fx.worktree),
                &[ReviewerWindowState::Running]
            )
            .is_ok(),
            "adopted baseline admits material via same-source gate"
        );
        // 归属防线说明:异窗口重入在租约面即被拒(见
        // two_attempts/abandon_gateless 钉测);manifest 归属校验为
        // 租约已过(同窗)但 manifest 异主时的纵深防御分支。
    }

    /// F5(P3):未持闸门/租约的失败窗(租约冲突停等)abandon 时不得被
    /// 持有人(合法在窗窗口)的闸门/租约 OwnerMismatch 卡死。
    #[tokio::test]
    async fn abandon_gateless_conflict_window_not_blocked_by_holder() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-owner", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("owner window running");

        // 租约冲突失败窗:从未持有租约/闸门
        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-loser", "attempt-2", &fx.worktree, &targets),
        )
        .await
        .expect_err("lease conflict");
        assert!(matches!(err, ReviewerWindowError::Conflict { .. }));
        let loser = store.load_window("w-loser").unwrap();
        assert_eq!(loser.state, ReviewerWindowState::RecoveryRequired);

        // F5:abandon 跳过非本窗持有的闸门/租约,顺利终态
        let terminal =
            abandon_window(&store, "w-loser", loser.generation, "lease conflict").unwrap();
        assert_eq!(terminal.state, ReviewerWindowState::ClosedInvalidated);

        // 持有人不受影响:仍 Running、闸门仍冻、租约仍在
        assert_eq!(
            store.load_window("w-owner").unwrap().state,
            ReviewerWindowState::Running
        );
        let common = fx.common_dir();
        let err = store
            .enter_write_channel(&common, WriteChannelKind::Commit, own_writer("probe-late"))
            .expect_err("owner freeze intact");
        assert!(matches!(err, ChannelGateError::Blocked { .. }));
        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-third", "attempt-3", &fx.worktree, &targets),
        )
        .await
        .expect_err("owner lease intact");
        assert!(matches!(err, ReviewerWindowError::Conflict { .. }));
    }

    // ---------- 修复轮 2 钉测(NEW-1) ----------

    /// NEW-1(P2):「manifest 已发布」≠「基线完整」——采用路径必须先过
    /// CAS blob 存在性+hash 预检;blob 缺失 → 停等(非 Running,不绑定
    /// 不可恢复基线);修复后旧窗弃置、同树新窗全新 capture 收口。
    #[tokio::test]
    async fn adoption_with_missing_blob_waits_not_running() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let first = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-n1", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("first begin");

        // 崩溃点 + blob 目录项丢失(已接受的掉电窗口形态)
        fs::remove_file(fx.window_root.join("windows/w-n1.json")).unwrap();
        store
            .create_or_takeover_window(&ReviewerWindowRecord::new(
                "w-n1",
                first.identity.clone(),
                "attempt-1",
                "run-1",
                "claude_code",
                Vec::new(),
            ))
            .expect("recreate preparing record");
        let manifest = snapshot.load_manifest("w-n1").unwrap();
        let blob = manifest
            .entries
            .iter()
            .find_map(|entry| entry.blob.clone())
            .expect("fixture has file entries");
        let blob_path = fx.snapshot_root.join("windows/w-n1").join(&blob);
        let blob_bytes = fs::read(&blob_path).unwrap();
        fs::remove_file(&blob_path).unwrap();

        // 采用:blob 缺失 → CAS 预检 Integrity 拒 → 停等,非 Running
        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-n1", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect_err("missing blob must stop-and-wait");
        match &err {
            ReviewerWindowError::Snapshot(error) => {
                assert!(error.to_string().contains("blob"), "{error:?}");
            }
            other => panic!("expected Snapshot, got {other:?}"),
        }
        let record = store.load_window("w-n1").unwrap();
        assert_eq!(record.state, ReviewerWindowState::RecoveryRequired);
        assert!(record.recovery.as_ref().unwrap().reason == "reviewer_window_snapshot_failed");
        assert!(
            assert_review_window_current(
                &store,
                &snapshot,
                &expectation("w-n1", "attempt-1", &fx.worktree),
                &[ReviewerWindowState::Running]
            )
            .is_err(),
            "incomplete baseline must not admit material"
        );

        // 修复 blob → 旧窗弃置(释放锁)→ 同树新窗全新 capture(非采用)
        fs::write(&blob_path, blob_bytes).unwrap();
        abandon_window(&store, "w-n1", record.generation, "blob 缺失停等").unwrap();
        begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-n2", "attempt-2", &fx.worktree, &targets),
        )
        .await
        .expect("fresh capture on new window after repair");
        assert_eq!(
            store.load_window("w-n2").unwrap().state,
            ReviewerWindowState::Running
        );
    }

    // ---------- T18 采信/违规收口新增 ----------

    #[tokio::test]
    async fn acceptance_projection_guards_terminal_running_and_manual_gate() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-guard2", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");

        // Running 拒绝(新窗口报告必须先 Pending 再受信裁决)。
        let err = record_report_acceptance(
            &store,
            &snapshot,
            "w-guard2",
            handle.generation,
            ReportAcceptance::Invalidated,
        )
        .expect_err("running cannot record acceptance");
        assert!(matches!(err, ReviewerWindowError::InvalidTransition { .. }));

        // Quiescing 未恢复:Accepted 拒绝(仅完整恢复才可采信)。
        let quiescing = begin_quiesce(&store, "w-guard2", handle.generation, "end").unwrap();
        let err = record_report_acceptance(
            &store,
            &snapshot,
            "w-guard2",
            quiescing.generation,
            ReportAcceptance::Accepted,
        )
        .expect_err("quiescing without restore cannot accept");
        assert!(matches!(err, ReviewerWindowError::InvalidTransition { .. }));

        // 违规流:人工门上 Accepted 拒绝(人工不能把旧报告改 Accepted)。
        let violated = record_violation(
            &store,
            &snapshot,
            "w-guard2",
            quiescing.generation,
            ViolationRecord {
                kind: "channel_bypass",
                path: "channels/commit",
                before_digest: None,
                after_digest: None,
                detail: "commit 通道绕过",
                report_id: None,
            },
        )
        .unwrap();
        let summary = restore_window(&store, &snapshot, "w-guard2", violated.generation)
            .await
            .expect("restore");
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);
        let err = record_report_acceptance(
            &store,
            &snapshot,
            "w-guard2",
            summary.record.generation,
            ReportAcceptance::Accepted,
        )
        .expect_err("manual gate cannot accept old report");
        assert!(matches!(err, ReviewerWindowError::InvalidTransition { .. }));

        // 终态拒绝(旧报告永久定格)。
        let terminal = resolve_manual_gate(
            &store,
            "w-guard2",
            summary.record.generation,
            ManualGateDecision::Invalidate,
        )
        .unwrap();
        assert_eq!(terminal.report_acceptance, ReportAcceptance::Invalidated);
        for acceptance in [
            ReportAcceptance::Accepted,
            ReportAcceptance::Invalidated,
            ReportAcceptance::PendingAcceptance,
        ] {
            assert!(
                record_report_acceptance(
                    &store,
                    &snapshot,
                    "w-guard2",
                    terminal.generation,
                    acceptance,
                )
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn close_accepted_rejects_late_journal_write() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-late", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-late", handle.generation, "end").unwrap();
        let summary = restore_window(&store, &snapshot, "w-late", quiescing.generation)
            .await
            .expect("restore");
        assert!(!summary.manual_gate);
        let accepted = record_report_acceptance(
            &store,
            &snapshot,
            "w-late",
            summary.record.generation,
            ReportAcceptance::Accepted,
        )
        .unwrap();

        // 晚到的违规 journal(模拟恢复后观测方补落)→ 采信关闭 fail-closed。
        store
            .append_violation(
                "w-late",
                ViolationJournalEntry {
                    seq: 0,
                    entry_id: "late-1".to_string(),
                    at: now_rfc3339(),
                    window_id: "w-late".to_string(),
                    attempt_id: "attempt-1".to_string(),
                    run_id: "run-1".to_string(),
                    report_id: None,
                    kind: "platform_domain_write".to_string(),
                    path: ".aria/late.json".to_string(),
                    before_digest: None,
                    after_digest: None,
                    detail: "恢复后补落".to_string(),
                    restore_cursor: "completed+0".to_string(),
                },
            )
            .unwrap();
        let err = close_accepted(&store, "w-late", accepted.generation)
            .expect_err("late journal must block acceptance");
        assert!(matches!(err, ReviewerWindowError::InvalidTransition { .. }));
    }

    #[tokio::test]
    async fn abandon_window_invalidates_report_permanently() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-ab2", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");
        // Running 须先 Quiescing→恢复(Restoring 可弃置):作废即旧报告永久 Invalidated。
        let quiescing = begin_quiesce(&store, "w-ab2", handle.generation, "end").unwrap();
        let summary = restore_window(&store, &snapshot, "w-ab2", quiescing.generation)
            .await
            .unwrap();
        let terminal = abandon_window(&store, "w-ab2", summary.record.generation, "stuck").unwrap();
        assert_eq!(terminal.state, ReviewerWindowState::ClosedInvalidated);
        assert_eq!(terminal.report_acceptance, ReportAcceptance::Invalidated);
    }

    /// 终态旧窗口可被新 attempt 接管(同 id):旧记录**归档留证**(不被
    /// 静默覆盖成活跃态),新窗口按新身份重建(新基线)。
    #[tokio::test]
    async fn terminal_window_is_taken_over_by_new_attempt() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-takeover", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");

        // 旧窗口收口为终态(违规 → 人工门 → 作废关闭;租约/通道已释放)。
        let quiescing = begin_quiesce(&store, "w-takeover", handle.generation, "abort").unwrap();
        let violated = record_violation(
            &store,
            &snapshot,
            "w-takeover",
            quiescing.generation,
            ViolationRecord {
                kind: "platform_domain_write",
                path: ".aria/direct-write.json",
                before_digest: None,
                after_digest: Some("sha256:deadbeef"),
                detail: "夹具:旧轮违规(写入违规 journal)",
                report_id: None,
            },
        )
        .unwrap();
        let summary = restore_window(&store, &snapshot, "w-takeover", violated.generation)
            .await
            .expect("restore");
        assert!(summary.manual_gate, "旧轮违规 → 人工门");
        let terminal = resolve_manual_gate(
            &store,
            "w-takeover",
            summary.record.generation,
            ManualGateDecision::Invalidate,
        )
        .expect("terminal close");
        assert_eq!(terminal.state, ReviewerWindowState::ClosedInvalidated);
        assert!(
            !store
                .load_violations("w-takeover")
                .expect("old journal")
                .is_empty(),
            "前置:旧轮违规已落 journal"
        );

        // 同 id 终态 → 新 attempt 接管:开窗成功。
        let scope2 = open_scope(&fx.base, "coder-2", None);
        let targets2: [&ProviderRunResourceScope; 1] = [&scope2];
        let taken = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-takeover", "attempt-2", &fx.worktree, &targets2),
        )
        .await
        .expect("terminal window must be takeable");
        assert_eq!(taken.window_id, "w-takeover");

        // 当前记录 = 新 attempt 的活窗(旧报告采信投影不继承)。
        let current = store.load_window("w-takeover").expect("current record");
        assert_eq!(current.attempt_id, "attempt-2");
        assert_eq!(current.state, ReviewerWindowState::Running);
        assert_eq!(
            current.report_acceptance,
            ReportAcceptance::PendingAcceptance,
            "接管的新窗口不得继承旧报告采信投影"
        );
        assert!(
            current
                .transitions
                .iter()
                .any(|entry| entry.note.contains("接管终态旧窗口")),
            "接管必须在新记录里留痕"
        );

        // 旧记录归档留证:终态/采信投影仍在,不是被静默覆盖。
        let archived = store
            .load_archived_window("w-takeover")
            .expect("archive read")
            .expect("archived record");
        assert_eq!(archived.attempt_id, "attempt-1");
        assert_eq!(archived.state, ReviewerWindowState::ClosedInvalidated);
        assert_eq!(archived.report_acceptance, ReportAcceptance::Invalidated);

        // 旧轮违规 journal 随旧轮归档:新轮从空 journal 起算(旧轮违规
        // 不得让新轮永久作废)。
        assert!(
            store
                .load_violations("w-takeover")
                .expect("new journal")
                .is_empty(),
            "接管的新窗口不得继承旧轮违规 journal"
        );
        assert!(
            fs::read_dir(fx.window_root.join("violation-archive"))
                .expect("violation archive dir")
                .count()
                > 0,
            "旧轮 journal 必须归档留证"
        );
    }

    /// 非终态窗口(活窗)不得被同 id 的其他 attempt 抢走(fail-closed
    /// 不因接管语义而放宽)。
    #[tokio::test]
    async fn live_window_with_other_attempt_is_rejected() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-live", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("begin");

        let scope2 = open_scope(&fx.base, "coder-2", None);
        let targets2: [&ProviderRunResourceScope; 1] = [&scope2];
        let err = begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-live", "attempt-2", &fx.worktree, &targets2),
        )
        .await
        .expect_err("live window must not be taken over");
        match err {
            ReviewerWindowError::Conflict { kind, .. } => {
                assert_eq!(kind, "reviewer_window", "必须是窗口记录冲突");
            }
            other => panic!("expected reviewer_window conflict, got {other:?}"),
        }
        // 原窗口仍是 attempt-1 的活窗,未被顶掉;被拒的接管不留归档。
        let current = store.load_window("w-live").expect("record");
        assert_eq!(current.attempt_id, "attempt-1");
        assert_eq!(current.state, ReviewerWindowState::Running);
        assert!(
            store
                .load_archived_window("w-live")
                .expect("archive read")
                .is_none(),
            "被拒的接管不得留下归档"
        );
    }

    /// 同 id + 同 attempt 的 Preparing 记录仍按现状续接(不重建、不接管)。
    #[tokio::test]
    async fn preparing_same_attempt_is_resumed() {
        let fx = window_fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let (canonical, gitdir, common_dir) =
            resolve_worktree_git_identity(&fx.worktree).expect("identity");
        let prepared = store
            .create_or_takeover_window(&ReviewerWindowRecord::new(
                "w-resume",
                ReviewWorktreeLockIdentity {
                    canonical_worktree: canonical,
                    gitdir,
                    common_dir,
                    lease_id: "wlease-w-resume".to_string(),
                },
                "attempt-1",
                "run-1",
                "claude_code",
                Vec::new(),
            ))
            .expect("preparing record")
            .record;
        assert_eq!(prepared.state, ReviewerWindowState::Preparing);

        let scope = open_scope(&fx.base, "coder", None);
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-resume", "attempt-1", &fx.worktree, &targets),
        )
        .await
        .expect("resume preparing window");
        let current = store.load_window("w-resume").expect("record");
        assert_eq!(current.state, ReviewerWindowState::Running);
        assert_eq!(
            current.created_at, prepared.created_at,
            "续接不得重建记录(仍为同一条 Preparing 记录)"
        );
        assert_eq!(current.generation, prepared.generation + 1);
        assert!(
            store
                .load_archived_window("w-resume")
                .expect("archive read")
                .is_none(),
            "续接不是接管,不得归档"
        );
    }
}
