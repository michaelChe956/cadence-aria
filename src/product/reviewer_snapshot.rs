//! 批2 Task 16:Reviewer B 档的可恢复基线快照与保留 Coder dirty 的
//! 恢复矩阵(C4.1 快照部 / D S6 Reviewer / B2 / REQ-ENV-REVIEW-B)。
//!
//! 合同要点(brief 冻结):
//! - [`ReviewerBaselineManifest`] 完整持久化到平台受信数据目录
//!   (`<root>/windows/<window_id>/manifest.json` + CAS blob),绝不向被审
//!   worktree 写任何快照产物;`capture_baseline` 全部落盘成功才返回引用,
//!   预算超限/读失败/不支持项(套接字/FIFO/设备/非 UTF-8 名)整单拒绝,
//!   不留部分基线。
//! - 消费 T5 静止证明([`QuiescenceEvidence`],从 `QuiescentScope` 冻结)
//!   与本 Task 定义的 [`ReviewWorktreeLockIdentity`](canonical
//!   worktree/gitdir/common-dir + durable 窗口租约 id,纯身份材料,不授予
//!   执行能力)。生产接线(T17)必须先持 durable 窗口锁再 capture;无锁
//!   身份或无静止证明一律拒绝。
//! - [`ReviewerSnapshotStore::restore_baseline`] 只按 manifest 差异
//!   no-follow 恢复:不调用 `git clean`/整树 reset,保留原
//!   staged/unstaged/untracked/ignored、删除、chmod、symlink 与目录形状
//!   (含空目录);Reviewer 新增残留按差异删除,Coder 原残留保留。
//! - Git 观察域:objects/reflog 不抹;Reviewer 自提交对象与新 ref 留作
//!   取证(report 的 `git_extra_refs_observed`),但 HEAD/index/本窗口受管
//!   ref(baseline 记录的 refs/heads+refs/tags)按受信精确 journal/CAS
//!   恢复,不对 common-dir 盲目镜像覆盖。
//! - 平台合法写入域(调用方精确预枚举)只做基线核验:漂移即
//!   `RestoreOutcome::RecoveryRequired`,本模块不代写平台 store,不伪称
//!   三域全通过;平台合法写入与 Reviewer 写入的判定归 T18,`Restored`
//!   不等于 Accepted。
//!
//! 恢复游标 [`RestoreCursor`] 持久化在 manifest 内,经 `write_json`
//! tmp+flush+sync+rename 原子推进;恢复每步幂等,任一游标边界故障后重启
//! 都能从原 manifest 续接(测试以故障注入逐边界验收)。
// 生产消费链:T17(窗口)/T18(三域对账)/T21(恢复)接线;接线完成前仅
// 合同测试消费(前瞻载体范式,同 provider_resource_store)。
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cross_cutting::git_command::{GitCommandError, args, git_stdout, run_git};
use crate::cross_cutting::provider_resources::{
    ProviderResourceRecord, QuiescentScope, ResourceScopeKey,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

/// 快照 schema 版本(域集合变化时递增,拒绝跨版本恢复)。
pub(crate) const REVIEWER_BASELINE_SCHEMA: &str = "reviewer_baseline/1";

/// 平台合法写入域基线的最大预枚举条目数(防御性上限)。
const PLATFORM_WRITE_PATH_LIMIT: usize = 4096;

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReviewerSnapshotError {
    #[error("reviewer_snapshot_invalid_request: {0}")]
    InvalidRequest(String),
    #[error("reviewer_snapshot_lock_identity_required: {0}")]
    LockIdentityRequired(String),
    #[error("reviewer_snapshot_quiescence_required: {0}")]
    QuiescenceRequired(String),
    #[error("reviewer_snapshot_budget_exceeded: {0}")]
    BudgetExceeded(String),
    #[error("reviewer_snapshot_unsupported_entry: {0}")]
    UnsupportedEntry(String),
    #[error("reviewer_snapshot_read_failure: {0}")]
    ReadFailure(String),
    #[error("reviewer_snapshot_identity_drift: {0}")]
    IdentityDrift(String),
    /// manifest/blob 缺失、hash 不符或 manifest 记录非法。
    #[error("reviewer_snapshot_integrity: {0}")]
    Integrity(String),
    #[error("reviewer_snapshot_git: {0}")]
    Git(String),
    #[error("reviewer_snapshot_store: {0}")]
    Store(#[from] ProductStoreError),
    #[error("reviewer_snapshot_io: {0}")]
    Io(String),
}

// ---------------------------------------------------------------------------
// 身份与准入材料
// ---------------------------------------------------------------------------

/// Reviewer B 档的被审 worktree 锁身份:canonical worktree/gitdir/
/// common-dir + durable 窗口租约 id。纯身份材料——**不授予执行能力**,
/// 也不在 capture/restore 内部获取或释放任何锁(T17 生产上先持 durable
/// 窗口租约,再以本身份调用 capture)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReviewWorktreeLockIdentity {
    /// 被审 worktree 的 canonical 绝对路径(漂移即拒绝)。
    pub(crate) canonical_worktree: PathBuf,
    /// 真实 gitdir:plain 仓=worktree/.git;linked worktree=gitfile 指向的
    /// `<common>/worktrees/<name>`。
    pub(crate) gitdir: PathBuf,
    /// common 目录(refs/objects 共享面;plain 仓等于 gitdir)。
    pub(crate) common_dir: PathBuf,
    /// durable 窗口租约 id(不透明;由 T17 的租约持有方注入)。
    pub(crate) lease_id: String,
}

/// T5 资源静止证据的冻结形态(从 `QuiescentScope` 提取;不传布尔
/// `killed=true`,交接完整 scope manifest 与恢复游标给 T17/T21)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QuiescenceEvidence {
    pub(crate) scope_key: ResourceScopeKey,
    pub(crate) scope_manifest: Vec<ProviderResourceRecord>,
    pub(crate) quiesced_at: String,
}

impl QuiescenceEvidence {
    /// 从 T5 静止证明冻结(消费 `QuiesceOutcome::Quiescent` 载荷)。
    pub(crate) fn from_quiescent_scope(scope: &QuiescentScope) -> Self {
        Self {
            scope_key: scope.key().clone(),
            scope_manifest: scope.manifest().to_vec(),
            quiesced_at: scope.quiesced_at().to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// 快照预算(超限整单拒绝,不截断)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SnapshotBudget {
    pub(crate) max_entries: usize,
    pub(crate) max_total_bytes: u64,
}

impl SnapshotBudget {
    /// 测试默认预算(生产默认由 T17 调用方给定)。
    pub(crate) fn generous() -> Self {
        Self {
            max_entries: 100_000,
            max_total_bytes: 256 * 1024 * 1024,
        }
    }
}

/// 目录项类型(file/gitfile=linked `.git` 指针文件;directory/symlink;
/// other=套接字/FIFO/设备等——capture 拒绝,restore 作为 Reviewer 残留删除)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BaselineEntryKind {
    File,
    Directory,
    Symlink,
    Gitfile,
}

/// worktree 相对路径目录项(逐字节/类型/mode/symlink target 恢复的最小单元)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BaselineEntry {
    /// 相对 worktree 根的 POSIX 风格路径(UTF-8;不含 `..`/绝对路径)。
    pub(crate) rel_path: String,
    pub(crate) kind: BaselineEntryKind,
    /// 权限位(含 setuid/stick 位;symlink 恒 0o777)。
    pub(crate) mode: u32,
    pub(crate) size: u64,
    /// 内容 sha256(file/gitfile)。
    pub(crate) content_sha256: Option<String>,
    /// store 内 blob 相对引用(`blobs/<sha256>`)。
    pub(crate) blob: Option<String>,
    /// symlink target(原样字符串,不解析不追随)。
    pub(crate) symlink_target: Option<String>,
}

/// Git 观察域原始摘要与恢复材料(HEAD/index/受管 ref;objects/reflog
/// 只观察不抹)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BaselineRef {
    pub(crate) ref_path: String,
    pub(crate) sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GitObservationBaseline {
    /// HEAD 的 symref 目标(detached 为 None)。
    pub(crate) head_symref: Option<String>,
    /// HEAD 提交 sha。
    pub(crate) head_sha: String,
    /// 原始 index 字节(gitdir/index)的 CAS blob 引用与 sha256。
    pub(crate) index_blob: String,
    pub(crate) index_sha256: String,
    /// 本窗口受管 ref(baseline 记录的 refs/heads+refs/tags;恢复只写
    /// 这些,不删 Reviewer 新增 ref)。
    pub(crate) managed_refs: Vec<BaselineRef>,
    /// for-each-ref 原始输出的 sha256 摘要(观察域原始摘要)。
    pub(crate) refs_digest: String,
}

/// 平台合法写入域基线条目(调用方精确预枚举;归属标签供 T18 判定)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlatformWritePath {
    /// 绝对路径(预枚举;本模块只核验不代写)。
    pub(crate) path: PathBuf,
    /// 归属(平台 store/服务名)。
    pub(crate) ownership: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub(crate) enum PlatformWriteState {
    Absent,
    File { content_sha256: String, mode: u32 },
    Directory { mode: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlatformWriteBaselineEntry {
    pub(crate) path: String,
    pub(crate) ownership: String,
    pub(crate) state: PlatformWriteState,
}

/// 恢复阶段(单调前进;持久化在 manifest 的 restore_cursor)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RestorePhase {
    /// 基线已发布,capture 完成时的初值。
    Started,
    /// manifest 条目逐项按差异恢复(entry_index 游标)。
    WorktreeRestore,
    /// manifest 外多余项(Reviewer 残留)删除。
    WorktreeExtra,
    /// HEAD/index/受管 ref 受信精确恢复。
    GitDomain,
    /// 平台合法写入域核验。
    PlatformDomain,
    /// 三域恢复完成且终验通过。
    Completed,
}

impl RestorePhase {
    fn rank(self) -> u8 {
        match self {
            RestorePhase::Started => 0,
            RestorePhase::WorktreeRestore => 1,
            RestorePhase::WorktreeExtra => 2,
            RestorePhase::GitDomain => 3,
            RestorePhase::PlatformDomain => 4,
            RestorePhase::Completed => 5,
        }
    }
}

/// 恢复游标(原子写入;重启后从原 manifest 续接)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RestoreCursor {
    pub(crate) phase: RestorePhase,
    /// WorktreeRestore 阶段的条目下标(manifest.entries 有序不变)。
    pub(crate) entry_index: usize,
}

impl RestoreCursor {
    pub(crate) fn start() -> Self {
        Self {
            phase: RestorePhase::Started,
            entry_index: 0,
        }
    }
}

/// Reviewer B 档基线 manifest(新私有合同;持久化在平台受信数据目录)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReviewerBaselineManifest {
    pub(crate) schema: String,
    pub(crate) window_id: String,
    pub(crate) attempt_id: String,
    pub(crate) run_id: String,
    pub(crate) lock_identity: ReviewWorktreeLockIdentity,
    /// 有序(按 rel_path)的完整 worktree 目录项(root `.git` 目录除外)。
    pub(crate) entries: Vec<BaselineEntry>,
    pub(crate) git_observation: GitObservationBaseline,
    pub(crate) platform_write_baseline: Vec<PlatformWriteBaselineEntry>,
    /// T18(F4):Git 观察域补充面(config/objects/reflog/子模块 gitdir)
    /// 的基线摘要;旧 manifest 无此字段 → None(该面不对比,报告披露)。
    #[serde(default)]
    pub(crate) git_annex_observation: Option<GitAnnexObservation>,

    pub(crate) scope_key: ResourceScopeKey,
    pub(crate) scope_manifest: Vec<ProviderResourceRecord>,
    pub(crate) quiesced_at: String,
    pub(crate) captured_at: String,
    pub(crate) restore_cursor: RestoreCursor,
}

/// Git 观察域补充摘要(T18 F4:直写 `.git/config`/objects/reflog/
/// 子模块 gitdir 的逃逸检测基线)。全部为**内容寻址摘要**,恢复/对账
/// 侧同源重算对比。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GitAnnexObservation {
    /// gitdir/config 内容 sha256(linked worktree 私有 config)。
    pub(crate) config_sha256: String,
    /// common-dir/config 内容 sha256。
    pub(crate) common_config_sha256: String,
    /// objects 域摘要(loose 对象相对名 + pack 文件名与大小的有序拼接
    /// sha256;对象名即内容寻址,摘要对新增/删除敏感)。
    pub(crate) objects_digest: String,
    /// reflog 摘要(gitdir/logs 与 common-dir/logs 下全部文件的
    /// 路径+内容 sha256 有序拼接摘要)。
    pub(crate) reflogs_digest: String,
    /// 子模块 gitdir 摘要(common-dir/modules 下目录名 + 各 config
    /// 内容 sha256 有序拼接摘要)。
    pub(crate) submodule_gitdirs_digest: String,
}

/// capture 请求。
pub(crate) struct CaptureRequest<'a> {
    pub(crate) window_id: &'a str,
    pub(crate) attempt_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) worktree: &'a Path,
    /// None → 拒绝(T17 生产上必须先持锁)。
    pub(crate) lock_identity: Option<ReviewWorktreeLockIdentity>,
    /// None → 拒绝(无静止证明不采)。
    pub(crate) quiescence: Option<QuiescenceEvidence>,
    /// 平台合法写入路径精确预枚举。
    pub(crate) platform_write_paths: Vec<PlatformWritePath>,
    pub(crate) budget: SnapshotBudget,
}

/// capture 完整持久化成功后的引用(全部落盘后才返回)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedBaseline {
    pub(crate) window_id: String,
    pub(crate) manifest_path: PathBuf,
    pub(crate) entry_count: usize,
    pub(crate) total_bytes: u64,
}

/// 恢复报告(证据交接;不是采信结论)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestoreReport {
    /// 本次恢复进入时的游标(证明续接位置)。
    pub(crate) resumed_from: RestoreCursor,
    pub(crate) worktree_fixed_entries: Vec<String>,
    pub(crate) worktree_removed_extras: Vec<String>,
    pub(crate) git_head_restored: bool,
    pub(crate) git_index_restored: bool,
    pub(crate) git_refs_restored: Vec<String>,
    /// 窗口内新增、恢复后仍保留取证的 ref(Reviewer 自提交对象同理不抹)。
    pub(crate) git_extra_refs_observed: Vec<String>,
    pub(crate) platform_verified: Vec<String>,
}

/// 恢复结局:`Restored` ≠ Accepted——平台合法写入与 Reviewer 写入的
/// 判定(三域对账/人工门)归 T18;无法安全恢复的域停等。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RestoreOutcome {
    Restored(RestoreReport),
    RecoveryRequired {
        reason: String,
        detail: String,
        cursor: RestoreCursor,
    },
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

pub(crate) struct ReviewerSnapshotStore {
    root: PathBuf,
    /// 仅测试:第 N 次游标原子写完成后注入失败(红绿证据:每个游标
    /// 边界故障后重启可从原 manifest 续接)。usize::MAX = 禁用。
    #[cfg(test)]
    restore_failpoint_after_writes: std::sync::atomic::AtomicUsize,
}

impl ReviewerSnapshotStore {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            #[cfg(test)]
            restore_failpoint_after_writes: std::sync::atomic::AtomicUsize::new(usize::MAX),
        }
    }

    /// 仅测试:注入第 N 次游标写之后的故障。
    #[cfg(test)]
    pub(crate) fn set_restore_failpoint_after_cursor_writes(&self, writes: usize) {
        self.restore_failpoint_after_writes
            .store(writes, std::sync::atomic::Ordering::Relaxed);
    }

    fn window_dir(&self, window_id: &str) -> PathBuf {
        self.root.join("windows").join(window_id)
    }

    fn manifest_path(&self, window_id: &str) -> PathBuf {
        self.window_dir(window_id).join("manifest.json")
    }

    pub(crate) fn load_manifest(
        &self,
        window_id: &str,
    ) -> Result<ReviewerBaselineManifest, ReviewerSnapshotError> {
        validate_relative_id(window_id)?;
        let path = self.manifest_path(window_id);
        if !path.is_file() {
            return Err(ProductStoreError::NotFound {
                kind: "reviewer_baseline_manifest",
                id: window_id.to_string(),
            }
            .into());
        }
        Ok(read_json(&path)?)
    }

    /// T17 采用路径消费(修复轮 2):加载 manifest 并核验其引用的全部
    /// CAS blob 存在且 hash 一致(O(blob 数);restore_baseline 动树前
    /// 同源预检)——「manifest 已发布」≠「基线完整」。
    pub(crate) fn verify_baseline_cas(
        &self,
        window_id: &str,
    ) -> Result<ReviewerBaselineManifest, ReviewerSnapshotError> {
        let manifest = self.load_manifest(window_id)?;
        verify_cas_blobs(&self.window_dir(window_id), &manifest)?;
        Ok(manifest)
    }

    /// 完整持久化基线:全部 blob + manifest 落盘成功才返回;预算超限/
    /// 读失败/不支持项/身份漂移整单拒绝并清理未发布窗口目录。
    pub(crate) fn capture_baseline(
        &self,
        request: CaptureRequest<'_>,
    ) -> Result<CapturedBaseline, ReviewerSnapshotError> {
        for (label, value) in [
            ("window_id", request.window_id),
            ("attempt_id", request.attempt_id),
            ("run_id", request.run_id),
        ] {
            if value.trim().is_empty() {
                return Err(ReviewerSnapshotError::InvalidRequest(format!(
                    "{label} 必须非空"
                )));
            }
        }
        validate_relative_id(request.window_id)?;
        let identity = request.lock_identity.clone().ok_or_else(|| {
            ReviewerSnapshotError::LockIdentityRequired(
                "生产接线(T17)必须先持 durable 窗口锁再 capture".to_string(),
            )
        })?;
        if identity.lease_id.trim().is_empty()
            || !identity.canonical_worktree.is_absolute()
            || !identity.gitdir.is_absolute()
            || !identity.common_dir.is_absolute()
        {
            return Err(ReviewerSnapshotError::InvalidRequest(
                "锁身份字段非法(lease 非空 + 绝对路径)".to_string(),
            ));
        }
        let evidence = request.quiescence.clone().ok_or_else(|| {
            ReviewerSnapshotError::QuiescenceRequired("无 T5 静止证明不得采基线".to_string())
        })?;
        if evidence.quiesced_at.trim().is_empty() {
            return Err(ReviewerSnapshotError::QuiescenceRequired(
                "静止证明缺 quiesced_at".to_string(),
            ));
        }
        let canonical = request.worktree.canonicalize().map_err(|error| {
            ReviewerSnapshotError::ReadFailure(format!(
                "canonicalize {}: {error}",
                request.worktree.display()
            ))
        })?;
        if canonical != identity.canonical_worktree {
            return Err(ReviewerSnapshotError::IdentityDrift(format!(
                "worktree 漂移: {} != {}",
                canonical.display(),
                identity.canonical_worktree.display()
            )));
        }
        let (gitdir, common_dir) = resolve_git_identity(&canonical)?;
        if gitdir != identity.gitdir || common_dir != identity.common_dir {
            return Err(ReviewerSnapshotError::IdentityDrift(format!(
                "gitdir/common-dir 漂移: ({}, {}) != ({}, {})",
                gitdir.display(),
                common_dir.display(),
                identity.gitdir.display(),
                identity.common_dir.display()
            )));
        }
        if request.platform_write_paths.len() > PLATFORM_WRITE_PATH_LIMIT {
            return Err(ReviewerSnapshotError::InvalidRequest(format!(
                "平台合法写入路径预枚举超限: {} > {PLATFORM_WRITE_PATH_LIMIT}",
                request.platform_write_paths.len()
            )));
        }
        let window_dir = self.window_dir(request.window_id);
        let manifest_path = self.manifest_path(request.window_id);
        if manifest_path.is_file() {
            return Err(ProductStoreError::Conflict {
                kind: "reviewer_baseline",
                id: request.window_id.to_string(),
            }
            .into());
        }
        // 未发布残留(上次崩溃的部分写入)先清理,再全量重建。
        if window_dir.exists() {
            std::fs::remove_dir_all(&window_dir).map_err(|error| {
                ReviewerSnapshotError::Io(format!(
                    "清理未发布窗口目录 {}: {error}",
                    window_dir.display()
                ))
            })?;
        }
        match self.capture_inner(&request, &identity, &evidence, &window_dir) {
            Ok(captured) => Ok(captured),
            Err(error) => {
                // 整单拒绝:不留部分基线(manifest 未发布即无引用)。
                let _ = std::fs::remove_dir_all(&window_dir);
                Err(error)
            }
        }
    }

    fn capture_inner(
        &self,
        request: &CaptureRequest<'_>,
        identity: &ReviewWorktreeLockIdentity,
        evidence: &QuiescenceEvidence,
        window_dir: &Path,
    ) -> Result<CapturedBaseline, ReviewerSnapshotError> {
        let blob_dir = window_dir.join("blobs");
        std::fs::create_dir_all(&blob_dir)
            .map_err(|error| io_error_string("create blob dir", error))?;
        let mut sink = BlobSink { dir: blob_dir };
        let walked = {
            let mut purpose = WalkPurpose::Capture {
                budget: &request.budget,
                sink: &mut sink,
            };
            walk_worktree(&identity.canonical_worktree, &mut purpose)?
        };
        let entries: Vec<BaselineEntry> = walked
            .iter()
            .map(|walked| BaselineEntry {
                rel_path: walked.rel_path.clone(),
                kind: walked.kind,
                mode: walked.mode,
                size: walked.size,
                content_sha256: walked.content_sha256.clone(),
                blob: walked
                    .content_sha256
                    .as_ref()
                    .map(|sha| format!("blobs/{sha}")),
                symlink_target: walked.symlink_target.clone(),
            })
            .collect();
        let total_bytes: u64 = entries.iter().map(|entry| entry.size).sum();

        // Git 观察域:受信 Git 桥(T4)读 HEAD/ref 摘要;原始 index 走 CAS。
        let worktree = identity.canonical_worktree.clone();
        let head_sha =
            git_stdout(&worktree, &args(&["rev-parse", "HEAD"])).map_err(map_git_error)?;
        let head_symref = match run_git(&worktree, &args(&["symbolic-ref", "-q", "HEAD"])) {
            Ok(record) => Some(record.stdout.trim().to_string()),
            Err(GitCommandError::Failed { record }) if record.exit_code == Some(1) => None,
            Err(error) => return Err(map_git_error(error)),
        };
        let refs_raw = git_stdout(
            &worktree,
            &args(&[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/heads",
                "refs/tags",
            ]),
        )
        .map_err(map_git_error)?;
        let mut managed_refs = Vec::new();
        for line in refs_raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((ref_path, sha)) = line.split_once(' ') else {
                return Err(ReviewerSnapshotError::Git(format!(
                    "for-each-ref 非法行: {line:?}"
                )));
            };
            if !is_object_id(sha) {
                return Err(ReviewerSnapshotError::Git(format!(
                    "for-each-ref 对象号非法: {ref_path}"
                )));
            }
            managed_refs.push(BaselineRef {
                ref_path: ref_path.to_string(),
                sha: sha.to_string(),
            });
        }
        let gitdir_fd = open_root_dir_no_follow(&identity.gitdir).map_err(|error| {
            ReviewerSnapshotError::ReadFailure(format!(
                "open gitdir {}: {error}",
                identity.gitdir.display()
            ))
        })?;
        let mut index_bytes = Vec::new();
        {
            let mut index_file =
                openat_read_no_follow(&gitdir_fd, OsStr::new("index")).map_err(|error| {
                    ReviewerSnapshotError::ReadFailure(format!("open gitdir/index: {error}"))
                })?;
            index_file.read_to_end(&mut index_bytes).map_err(|error| {
                ReviewerSnapshotError::ReadFailure(format!("read gitdir/index: {error}"))
            })?;
        }
        let index_sha256 = sha256_hex(&index_bytes);
        sink.store(&index_sha256, &index_bytes)?;
        let git_observation = GitObservationBaseline {
            head_symref,
            head_sha,
            index_blob: format!("blobs/{index_sha256}"),
            index_sha256: index_sha256.clone(),
            managed_refs,
            refs_digest: sha256_hex(refs_raw.as_bytes()),
        };

        let platform_write_baseline = capture_platform_baseline(&request.platform_write_paths)?;

        let manifest = ReviewerBaselineManifest {
            schema: REVIEWER_BASELINE_SCHEMA.to_string(),
            window_id: request.window_id.to_string(),
            attempt_id: request.attempt_id.to_string(),
            run_id: request.run_id.to_string(),
            lock_identity: identity.clone(),
            entries,
            git_observation,
            git_annex_observation: Some(observe_git_annex(&identity.gitdir, &identity.common_dir)?),
            platform_write_baseline,
            scope_key: evidence.scope_key.clone(),
            scope_manifest: evidence.scope_manifest.clone(),
            quiesced_at: evidence.quiesced_at.clone(),
            captured_at: chrono::Utc::now().to_rfc3339(),
            restore_cursor: RestoreCursor::start(),
        };
        let manifest_path = window_dir.join("manifest.json");
        // 原子发布:全部 blob 落盘并 sync 后才写 manifest(write_json
        // tmp+flush+sync+rename);失败路径由调用方整目录清理。
        write_json(&manifest_path, &manifest)?;
        Ok(CapturedBaseline {
            window_id: request.window_id.to_string(),
            manifest_path,
            entry_count: manifest.entries.len(),
            total_bytes,
        })
    }

    /// 只按 manifest 差异 no-follow 恢复(不 `git clean`/整树 reset);
    /// 域内无法安全恢复 → `RecoveryRequired`。
    pub(crate) fn restore_baseline(
        &self,
        window_id: &str,
        identity: &ReviewWorktreeLockIdentity,
    ) -> Result<RestoreOutcome, ReviewerSnapshotError> {
        validate_relative_id(window_id)?;
        let manifest = self.load_manifest(window_id)?;
        validate_manifest(&manifest)?;
        if &manifest.lock_identity != identity {
            return Err(ReviewerSnapshotError::IdentityDrift(format!(
                "传入锁身份与冻结身份不符(lease={:?})",
                identity.lease_id
            )));
        }
        let canonical = identity
            .canonical_worktree
            .canonicalize()
            .map_err(|error| {
                ReviewerSnapshotError::IdentityDrift(format!(
                    "worktree 不可解析 {}: {error}",
                    identity.canonical_worktree.display()
                ))
            })?;
        if canonical != identity.canonical_worktree {
            return Err(ReviewerSnapshotError::IdentityDrift(format!(
                "worktree 漂移: {} != {}",
                canonical.display(),
                identity.canonical_worktree.display()
            )));
        }
        let (gitdir, common_dir) = resolve_git_identity(&canonical)?;
        if gitdir != identity.gitdir || common_dir != identity.common_dir {
            return Err(ReviewerSnapshotError::IdentityDrift(
                "gitdir/common-dir 与锁身份不符".to_string(),
            ));
        }

        // CAS 预检:任何改动之前核验全部 blob 存在且 hash 一致(缺失/错
        // hash 整单拒绝,不做部分恢复)。
        let window_dir = self.window_dir(window_id);
        verify_cas_blobs(&window_dir, &manifest)?;

        let resumed_from = manifest.restore_cursor.clone();
        let mut ctx = RestoreCtx {
            store: self,
            window_dir: window_dir.clone(),
            manifest_path: self.manifest_path(window_id),
            manifest,
            identity: identity.clone(),
            worktree: canonical,
            report: RestoreReport {
                resumed_from: resumed_from.clone(),
                worktree_fixed_entries: Vec::new(),
                worktree_removed_extras: Vec::new(),
                git_head_restored: false,
                git_index_restored: false,
                git_refs_restored: Vec::new(),
                git_extra_refs_observed: Vec::new(),
                platform_verified: Vec::new(),
            },
        };

        if resumed_from.phase != RestorePhase::Completed {
            let entry_cursor = resumed_from.clone();
            if entry_cursor.phase == RestorePhase::Started {
                ctx.advance(RestorePhase::WorktreeRestore, 0)?;
            }
            // Worktree 域:manifest 条目逐项按差异恢复(幂等,游标逐条推进)
            let start = match entry_cursor.phase {
                RestorePhase::Started => 0,
                RestorePhase::WorktreeRestore => entry_cursor.entry_index,
                _ => ctx.manifest.entries.len(),
            };
            let mut index = start;
            while index < ctx.manifest.entries.len() {
                let entry = ctx.manifest.entries[index].clone();
                if ctx.restore_entry(&entry)? {
                    ctx.report
                        .worktree_fixed_entries
                        .push(entry.rel_path.clone());
                }
                index += 1;
                ctx.advance(RestorePhase::WorktreeRestore, index)?;
            }

            {
                // Worktree 域:manifest 外多余项(Reviewer 残留)删除。
                // 幂等重放(不做游标门禁):即使游标已越过 Extra——如故障
                // 后有人在恢复窗口内再写入——重放 sweep 仍按 manifest 差异
                // 清残留;游标只在尚未越过 Extra 时前进推进。
                let mut removed = Vec::new();
                let rels: HashSet<String> = ctx
                    .manifest
                    .entries
                    .iter()
                    .map(|entry| entry.rel_path.clone())
                    .collect();
                {
                    let mut purpose = WalkPurpose::Observe {
                        delete_extras: true,
                        manifest_rels: Some(&rels),
                        removed: &mut removed,
                    };
                    walk_worktree(&ctx.worktree, &mut purpose)?;
                }
                ctx.report.worktree_removed_extras.extend(removed);
                if ctx.manifest.restore_cursor.phase.rank() < RestorePhase::WorktreeExtra.rank() {
                    ctx.advance(RestorePhase::WorktreeExtra, ctx.manifest.entries.len())?;
                }
            }
            // Git 观察域:HEAD/index/受管 ref 受信精确恢复(幂等重放;
            // 游标只在尚未越过 GitDomain 时前进推进)
            restore_git_domain(&mut ctx)?;
            if ctx.manifest.restore_cursor.phase.rank() < RestorePhase::GitDomain.rank() {
                ctx.advance(RestorePhase::GitDomain, ctx.manifest.entries.len())?;
            }
            // 平台合法写入域:只核验,漂移即停等(不代写平台 store)。
            // 游标在 PlatformDomain 上重入时重新核验(人工/T18 处置后续接)。
            if ctx.manifest.restore_cursor.phase.rank() <= RestorePhase::PlatformDomain.rank() {
                if ctx.manifest.restore_cursor.phase.rank() < RestorePhase::PlatformDomain.rank() {
                    ctx.advance(RestorePhase::PlatformDomain, ctx.manifest.entries.len())?;
                }
                for baseline in &ctx.manifest.platform_write_baseline {
                    let path = PathBuf::from(&baseline.path);
                    let drifted = match observe_platform_state(&path) {
                        Ok(state) => state != baseline.state,
                        // symlink/特殊文件 = 无法安全核验的漂移形态
                        Err(_) => true,
                    };
                    if drifted {
                        return Ok(RestoreOutcome::RecoveryRequired {
                            reason: "platform_domain_drift".to_string(),
                            detail: format!(
                                "{} (归属 {}): 平台合法写入域漂移,判定归 T18",
                                baseline.path, baseline.ownership
                            ),
                            cursor: ctx.manifest.restore_cursor.clone(),
                        });
                    }
                }
            }
            // 终验(三域全对账)通过才写 Completed
            if let Err((reason, detail)) = verify_all(&mut ctx)? {
                return Ok(RestoreOutcome::RecoveryRequired {
                    reason,
                    detail,
                    cursor: ctx.manifest.restore_cursor.clone(),
                });
            }
            ctx.advance(RestorePhase::Completed, ctx.manifest.entries.len())?;
        }

        // Completed(含重入):终验幂等重放;不伪称三域通过
        match verify_all(&mut ctx)? {
            Ok(()) => {
                ctx.report.platform_verified.sort();
                ctx.report.platform_verified.dedup();
                Ok(RestoreOutcome::Restored(ctx.report))
            }
            Err((reason, detail)) => Ok(RestoreOutcome::RecoveryRequired {
                reason,
                detail,
                cursor: ctx.manifest.restore_cursor.clone(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// manifest 校验与 blob 路径
// ---------------------------------------------------------------------------

fn io_error_string(context: &str, error: std::io::Error) -> ReviewerSnapshotError {
    ReviewerSnapshotError::Io(format!("{context}: {error}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_rel_path(rel: &str) -> Result<(), ReviewerSnapshotError> {
    if rel.is_empty() || rel.starts_with('/') {
        return Err(ReviewerSnapshotError::Integrity(format!(
            "非法相对路径: {rel:?}"
        )));
    }
    for component in rel.split('/') {
        if component.is_empty() || component == "." || component == ".." || component.contains('\0')
        {
            return Err(ReviewerSnapshotError::Integrity(format!(
                "非法路径分量: {rel:?}"
            )));
        }
    }
    Ok(())
}

fn blob_abs_path(window_dir: &Path, blob_rel: &str) -> Result<PathBuf, ReviewerSnapshotError> {
    let sha = blob_rel
        .strip_prefix("blobs/")
        .ok_or_else(|| ReviewerSnapshotError::Integrity(format!("blob 引用非法: {blob_rel:?}")))?;
    if !is_sha256_hex(sha) || sha.contains('/') {
        return Err(ReviewerSnapshotError::Integrity(format!(
            "blob 引用非法: {blob_rel:?}"
        )));
    }
    Ok(window_dir.join("blobs").join(sha))
}

/// CAS 预检:manifest 引用的全部 blob(条目+index)存在且 hash 一致。
/// restore_baseline 动树前与 T17 采用路径(修复轮 2:manifest 已发布
/// ≠基线完整——掉电可丢 blob 目录项)共用;缺失/错 hash 整单拒绝。
fn verify_cas_blobs(
    window_dir: &Path,
    manifest: &ReviewerBaselineManifest,
) -> Result<(), ReviewerSnapshotError> {
    let mut referenced: Vec<&str> = manifest
        .entries
        .iter()
        .filter_map(|entry| entry.blob.as_deref())
        .collect();
    referenced.push(manifest.git_observation.index_blob.as_str());
    for blob_rel in referenced {
        let path = blob_abs_path(window_dir, blob_rel)?;
        let bytes = std::fs::read(&path).map_err(|error| {
            ReviewerSnapshotError::Integrity(format!("blob 缺失 {blob_rel}: {error}"))
        })?;
        let expected = blob_rel
            .strip_prefix("blobs/")
            .expect("blob_abs_path 已校验形状");
        if sha256_hex(&bytes) != expected {
            return Err(ReviewerSnapshotError::Integrity(format!(
                "blob hash 不符 {blob_rel}"
            )));
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &ReviewerBaselineManifest) -> Result<(), ReviewerSnapshotError> {
    if manifest.schema != REVIEWER_BASELINE_SCHEMA {
        return Err(ReviewerSnapshotError::Integrity(format!(
            "schema 不符: {:?}",
            manifest.schema
        )));
    }
    let mut rels: HashSet<String> = HashSet::new();
    let mut previous: Option<&str> = None;
    for entry in &manifest.entries {
        validate_rel_path(&entry.rel_path)?;
        if previous.is_some_and(|value| entry.rel_path.as_str() <= value) {
            return Err(ReviewerSnapshotError::Integrity(
                "entries 未按 rel_path 严格有序".to_string(),
            ));
        }
        previous = Some(entry.rel_path.as_str());
        if !rels.insert(entry.rel_path.clone()) {
            return Err(ReviewerSnapshotError::Integrity(format!(
                "重复条目: {}",
                entry.rel_path
            )));
        }
        // root `.git` 只允许作为 gitfile 身份锚点出现;不得快照其内部
        if entry.rel_path.starts_with(".git/") {
            return Err(ReviewerSnapshotError::Integrity(format!(
                "不得快照 .git 内部: {}",
                entry.rel_path
            )));
        }
        if entry.rel_path == ".git" && entry.kind != BaselineEntryKind::Gitfile {
            return Err(ReviewerSnapshotError::Integrity(
                "root .git 记录必须为 gitfile 类型".to_string(),
            ));
        }
        match entry.kind {
            BaselineEntryKind::File | BaselineEntryKind::Gitfile => {
                let Some(sha) = entry.content_sha256.as_ref() else {
                    return Err(ReviewerSnapshotError::Integrity(format!(
                        "{}: 缺内容 sha",
                        entry.rel_path
                    )));
                };
                if !is_sha256_hex(sha) {
                    return Err(ReviewerSnapshotError::Integrity(format!(
                        "{}: sha 非法",
                        entry.rel_path
                    )));
                }
                if entry.blob.as_deref() != Some(format!("blobs/{sha}").as_str()) {
                    return Err(ReviewerSnapshotError::Integrity(format!(
                        "{}: blob 引用与 sha 不符",
                        entry.rel_path
                    )));
                }
            }
            BaselineEntryKind::Symlink => {
                if entry.symlink_target.is_none() || entry.blob.is_some() {
                    return Err(ReviewerSnapshotError::Integrity(format!(
                        "{}: symlink 记录非法",
                        entry.rel_path
                    )));
                }
            }
            BaselineEntryKind::Directory => {
                if entry.blob.is_some() || entry.content_sha256.is_some() {
                    return Err(ReviewerSnapshotError::Integrity(format!(
                        "{}: 目录记录带内容",
                        entry.rel_path
                    )));
                }
            }
        }
        // capture 不变量:父目录必须也在 manifest(保证多余目录可整删)
        if let Some((parent, _)) = entry.rel_path.rsplit_once('/') {
            if !parent.is_empty() && !rels.contains(parent) {
                return Err(ReviewerSnapshotError::Integrity(format!(
                    "{}: 父目录 {parent} 不在 manifest",
                    entry.rel_path
                )));
            }
        }
    }
    if !is_object_id(&manifest.git_observation.head_sha)
        || !is_sha256_hex(&manifest.git_observation.index_sha256)
        || !is_sha256_hex(&manifest.git_observation.refs_digest)
        || manifest.git_observation.index_blob
            != format!("blobs/{}", manifest.git_observation.index_sha256)
    {
        return Err(ReviewerSnapshotError::Integrity(
            "git 观察域记录非法".to_string(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// openat no-follow 基元(libc;锚定父目录,整条路径不追随 symlink——同
// kimi client_services/sandbox.rs 的原子锚定范式,但为本模块私有实现)
// ---------------------------------------------------------------------------

fn cstring_of(name: &OsStr) -> Result<std::ffi::CString, ReviewerSnapshotError> {
    std::ffi::CString::new(name.as_bytes())
        .map_err(|_| ReviewerSnapshotError::Integrity("path_contains_nul".to_string()))
}

fn invalid_input(error: ReviewerSnapshotError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, error.to_string())
}

fn open_root_dir_no_follow(path: &Path) -> std::io::Result<OwnedFd> {
    let c = cstring_of(path.as_os_str()).map_err(invalid_input)?;
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn openat_dir(parent: &OwnedFd, name: &OsStr) -> std::io::Result<OwnedFd> {
    let c = cstring_of(name).map_err(invalid_input)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn openat_read_no_follow(parent: &OwnedFd, name: &OsStr) -> std::io::Result<std::fs::File> {
    let c = cstring_of(name).map_err(invalid_input)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            c.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

fn fstatat_no_follow(parent: &OwnedFd, name: &OsStr) -> std::io::Result<libc::stat> {
    let c = cstring_of(name).map_err(invalid_input)?;
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            c.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(stat)
}

fn readlinkat_target(parent: &OwnedFd, name: &OsStr) -> std::io::Result<OsString> {
    let c = cstring_of(name).map_err(invalid_input)?;
    let mut buffer = vec![0u8; 256];
    loop {
        let length = unsafe {
            libc::readlinkat(
                parent.as_raw_fd(),
                c.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if length < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let length = length as usize;
        if length < buffer.len() {
            buffer.truncate(length);
            return Ok(OsString::from_vec(buffer));
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

fn read_dir_sorted(dir: &OwnedFd) -> std::io::Result<Vec<OsString>> {
    unsafe {
        let duplicate = libc::dup(dir.as_raw_fd());
        if duplicate < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let handle = libc::fdopendir(duplicate);
        if handle.is_null() {
            let error = std::io::Error::last_os_error();
            libc::close(duplicate);
            return Err(error);
        }
        let mut names = Vec::new();
        loop {
            let entry = libc::readdir(handle);
            if entry.is_null() {
                break;
            }
            let bytes = std::ffi::CStr::from_ptr((*entry).d_name.as_ptr())
                .to_bytes()
                .to_vec();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            names.push(OsString::from_vec(bytes));
        }
        libc::closedir(handle);
        names.sort();
        Ok(names)
    }
}

fn unlinkat_file(parent: &OwnedFd, name: &OsStr) -> std::io::Result<()> {
    let c = cstring_of(name).map_err(invalid_input)?;
    if unsafe { libc::unlinkat(parent.as_raw_fd(), c.as_ptr(), 0) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn unlinkat_dir(parent: &OwnedFd, name: &OsStr) -> std::io::Result<()> {
    let c = cstring_of(name).map_err(invalid_input)?;
    if unsafe { libc::unlinkat(parent.as_raw_fd(), c.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn mkdirat_dir(parent: &OwnedFd, name: &OsStr, mode: u32) -> std::io::Result<()> {
    let c = cstring_of(name).map_err(invalid_input)?;
    if unsafe { libc::mkdirat(parent.as_raw_fd(), c.as_ptr(), mode as libc::mode_t) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn symlinkat_link(target: &OsStr, parent: &OwnedFd, name: &OsStr) -> std::io::Result<()> {
    let target_c = cstring_of(target).map_err(invalid_input)?;
    let name_c = cstring_of(name).map_err(invalid_input)?;
    if unsafe { libc::symlinkat(target_c.as_ptr(), parent.as_raw_fd(), name_c.as_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn renameat_within(parent: &OwnedFd, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    let from_c = cstring_of(from).map_err(invalid_input)?;
    let to_c = cstring_of(to).map_err(invalid_input)?;
    if unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            from_c.as_ptr(),
            parent.as_raw_fd(),
            to_c.as_ptr(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn fchmod_file(file: &std::fs::File, mode: u32) -> std::io::Result<()> {
    if unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn fchmod_dir(dir: &OwnedFd, mode: u32) -> std::io::Result<()> {
    if unsafe { libc::fchmod(dir.as_raw_fd(), mode as libc::mode_t) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 锚定父目录的原子字节写:tmp(O_CREAT|O_EXCL|O_NOFOLLOW)+fsync+fchmod+
/// renameat(rename 不追随目标 symlink;整条写路径 no-follow)。
fn write_bytes_atomic(
    parent: &OwnedFd,
    name: &OsStr,
    bytes: &[u8],
    mode: u32,
    tag: &str,
) -> std::io::Result<()> {
    let unique = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp: OsString = format!(".aria-{tag}-{unique}").into();
    {
        let c = cstring_of(&tmp).map_err(invalid_input)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                c.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            fchmod_file(&file, mode)
        })();
        drop(file);
        result?;
    }
    if let Err(error) = renameat_within(parent, &tmp, name) {
        let _ = unlinkat_file(parent, &tmp);
        return Err(error);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// worktree 走查(fstatat AT_SYMLINK_NOFOLLOW + openat 锚定,不追随
// symlink 到 owner 文件)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct WalkedEntry {
    rel_path: String,
    kind: BaselineEntryKind,
    mode: u32,
    size: u64,
    content_sha256: Option<String>,
    symlink_target: Option<String>,
    /// 套接字/FIFO/设备/非 UTF-8 名:capture 拒绝;restore 视作可删残留。
    unsupported: bool,
}

#[derive(Default)]
struct WalkStats {
    entries: usize,
    bytes: u64,
}

struct BlobSink {
    dir: PathBuf,
}

impl BlobSink {
    /// CAS 写入(create-new;同 sha 已存在=去重,hash 由 restore 预检兜底)。
    fn store(&self, sha_hex: &str, bytes: &[u8]) -> Result<(), ReviewerSnapshotError> {
        let path = self.dir.join(sha_hex);
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Ok(());
            }
            Err(error) => {
                return Err(io_error_string(
                    &format!("blob create {}", path.display()),
                    error,
                ));
            }
        };
        file.write_all(bytes)
            .map_err(|error| io_error_string("blob write", error))?;
        file.sync_all()
            .map_err(|error| io_error_string("blob sync", error))?;
        Ok(())
    }
}

enum WalkPurpose<'a> {
    Capture {
        budget: &'a SnapshotBudget,
        sink: &'a mut BlobSink,
    },
    Observe {
        delete_extras: bool,
        manifest_rels: Option<&'a HashSet<String>>,
        removed: &'a mut Vec<String>,
    },
}

fn classify(stat_mode: u32) -> Option<BaselineEntryKind> {
    match stat_mode & libc::S_IFMT {
        libc::S_IFREG => Some(BaselineEntryKind::File),
        libc::S_IFDIR => Some(BaselineEntryKind::Directory),
        libc::S_IFLNK => Some(BaselineEntryKind::Symlink),
        _ => None,
    }
}

fn walk_worktree(
    root: &Path,
    purpose: &mut WalkPurpose<'_>,
) -> Result<Vec<WalkedEntry>, ReviewerSnapshotError> {
    let root_fd = open_root_dir_no_follow(root).map_err(|error| {
        ReviewerSnapshotError::ReadFailure(format!(
            "open worktree root {}: {error}",
            root.display()
        ))
    })?;
    let mut out = Vec::new();
    let mut stats = WalkStats::default();
    walk_dir(&root_fd, "", true, purpose, &mut out, &mut stats)?;
    out.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    Ok(out)
}

fn raw_rel_name(name: &OsStr) -> String {
    format!("raw:{}", hex::encode(name.as_bytes()))
}

fn walk_dir(
    dir: &OwnedFd,
    prefix: &str,
    is_root: bool,
    purpose: &mut WalkPurpose<'_>,
    out: &mut Vec<WalkedEntry>,
    stats: &mut WalkStats,
) -> Result<(), ReviewerSnapshotError> {
    let names = read_dir_sorted(dir).map_err(|error| {
        ReviewerSnapshotError::ReadFailure(format!("readdir {prefix:?}: {error}"))
    })?;
    let delete_extras = matches!(
        purpose,
        WalkPurpose::Observe {
            delete_extras: true,
            ..
        }
    );
    for name in names {
        let name_str = name.to_str().map(str::to_string);
        let rel = name_str.clone().map(|value| {
            if prefix.is_empty() {
                value
            } else {
                format!("{prefix}/{value}")
            }
        });
        let stat = fstatat_no_follow(dir, &name).map_err(|error| {
            ReviewerSnapshotError::ReadFailure(format!(
                "fstatat {}: {error}",
                name.to_string_lossy()
            ))
        })?;
        let mode = stat.st_mode & 0o7777;
        let size = stat.st_size.max(0) as u64;
        let classified = classify(stat.st_mode);

        // root `.git`:目录=Git 观察域另行冻结,不入 worktree 清单(恢复
        // 也永不触碰);gitfile=身份锚点条目;symlink/特殊文件=不支持。
        let effective_kind = if is_root && name == ".git" {
            match classified {
                Some(BaselineEntryKind::Directory) => {
                    continue;
                }
                Some(BaselineEntryKind::File) => Some(BaselineEntryKind::Gitfile),
                other => other,
            }
        } else {
            classified
        };

        // Observe+delete:manifest 外多余项=Reviewer 残留,删除;Coder 原样
        // 保留域全部在 manifest 内。root `.git`(任何形态)永不由 sweep 处置。
        let in_manifest = rel.as_ref().is_some_and(|value| match purpose {
            WalkPurpose::Observe {
                manifest_rels: Some(set),
                ..
            } => set.contains(value),
            _ => false,
        });
        if delete_extras && !in_manifest && !(is_root && name == ".git") {
            let label = rel.clone().unwrap_or_else(|| raw_rel_name(&name));
            match effective_kind {
                Some(BaselineEntryKind::Directory) => {
                    let child = openat_dir(dir, &name).map_err(|error| {
                        ReviewerSnapshotError::ReadFailure(format!("openat dir {label}: {error}"))
                    })?;
                    walk_dir(&child, &label, false, purpose, out, stats)?;
                    unlinkat_dir(dir, &name)
                        .map_err(|error| io_error_string("rmdir extra", error))?;
                }
                _ => {
                    unlinkat_file(dir, &name)
                        .or_else(|_| unlinkat_dir(dir, &name))
                        .map_err(|error| io_error_string("unlink extra", error))?;
                }
            }
            if let WalkPurpose::Observe { removed, .. } = purpose {
                removed.push(label);
            }
            continue;
        }

        // 不支持项/非 UTF-8 名:capture 整单拒绝;observe 记为可删残留
        if effective_kind.is_none() || rel.is_none() {
            let label = rel.clone().unwrap_or_else(|| raw_rel_name(&name));
            match purpose {
                WalkPurpose::Capture { .. } => {
                    return Err(ReviewerSnapshotError::UnsupportedEntry(format!(
                        "{label}: 不支持项(非 UTF-8 名或 socket/FIFO/设备)"
                    )));
                }
                WalkPurpose::Observe { .. } => {
                    out.push(WalkedEntry {
                        rel_path: label,
                        kind: BaselineEntryKind::File,
                        mode,
                        size,
                        content_sha256: None,
                        symlink_target: None,
                        unsupported: true,
                    });
                    continue;
                }
            }
        }
        let kind = effective_kind.expect("上方已判空");
        let rel = rel.expect("上方已判空");

        // 预算先行:超限整单拒绝,不截断称完整
        if let WalkPurpose::Capture { budget, .. } = purpose {
            if stats.entries + 1 > budget.max_entries {
                return Err(ReviewerSnapshotError::BudgetExceeded(format!(
                    "目录项数 {} 超预算 {}",
                    stats.entries + 1,
                    budget.max_entries
                )));
            }
            if stats.bytes + size > budget.max_total_bytes {
                return Err(ReviewerSnapshotError::BudgetExceeded(format!(
                    "字节 {} + {size} 超预算 {}",
                    stats.bytes, budget.max_total_bytes
                )));
            }
        }

        match kind {
            BaselineEntryKind::Directory => {
                stats.entries += 1;
                out.push(WalkedEntry {
                    rel_path: rel.clone(),
                    kind,
                    mode,
                    size: 0,
                    content_sha256: None,
                    symlink_target: None,
                    unsupported: false,
                });
                let child = openat_dir(dir, &name).map_err(|error| {
                    ReviewerSnapshotError::ReadFailure(format!("openat dir {rel}: {error}"))
                })?;
                walk_dir(&child, &rel, false, purpose, out, stats)?;
            }
            BaselineEntryKind::File | BaselineEntryKind::Gitfile => {
                let mut file = openat_read_no_follow(dir, &name).map_err(|error| {
                    ReviewerSnapshotError::ReadFailure(format!("open {rel}: {error}"))
                })?;
                let open_stat: libc::stat = {
                    let mut value: libc::stat = unsafe { std::mem::zeroed() };
                    if unsafe { libc::fstat(file.as_raw_fd(), &mut value) } != 0 {
                        return Err(ReviewerSnapshotError::ReadFailure(format!(
                            "fstat {rel}: {}",
                            std::io::Error::last_os_error()
                        )));
                    }
                    value
                };
                if open_stat.st_mode & libc::S_IFMT != libc::S_IFREG {
                    return Err(ReviewerSnapshotError::ReadFailure(format!(
                        "{rel}: 打开后类型漂移"
                    )));
                }
                let mut bytes = Vec::with_capacity(open_stat.st_size.max(0) as usize);
                file.read_to_end(&mut bytes).map_err(|error| {
                    ReviewerSnapshotError::ReadFailure(format!("read {rel}: {error}"))
                })?;
                if bytes.len() as i64 != open_stat.st_size {
                    return Err(ReviewerSnapshotError::ReadFailure(format!(
                        "{rel}: 读取期间大小漂移"
                    )));
                }
                let sha = sha256_hex(&bytes);
                if let WalkPurpose::Capture { sink, .. } = purpose {
                    sink.store(&sha, &bytes)?;
                }
                stats.entries += 1;
                stats.bytes += bytes.len() as u64;
                out.push(WalkedEntry {
                    rel_path: rel,
                    kind,
                    mode: open_stat.st_mode & 0o7777,
                    size: bytes.len() as u64,
                    content_sha256: Some(sha),
                    symlink_target: None,
                    unsupported: false,
                });
            }
            BaselineEntryKind::Symlink => {
                let target_os = readlinkat_target(dir, &name).map_err(|error| {
                    ReviewerSnapshotError::ReadFailure(format!("readlink {rel}: {error}"))
                })?;
                let Some(target) = target_os.to_str().map(str::to_string) else {
                    match purpose {
                        WalkPurpose::Capture { .. } => {
                            return Err(ReviewerSnapshotError::UnsupportedEntry(format!(
                                "{rel}: symlink target 非 UTF-8"
                            )));
                        }
                        WalkPurpose::Observe { .. } => {
                            out.push(WalkedEntry {
                                rel_path: rel,
                                kind,
                                mode: 0o777,
                                size: 0,
                                content_sha256: None,
                                symlink_target: None,
                                unsupported: true,
                            });
                            continue;
                        }
                    }
                };
                stats.entries += 1;
                out.push(WalkedEntry {
                    rel_path: rel,
                    kind,
                    mode: 0o777,
                    size: 0,
                    content_sha256: None,
                    symlink_target: Some(target),
                    unsupported: false,
                });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Git 身份解析(capture/restore 共用;解析失败一律按身份漂移拒绝)
// ---------------------------------------------------------------------------

/// 解析被审 worktree 的真实 gitdir/common-dir:
/// - root `.git` 为目录(plain)→ gitdir=该目录;commondir 文件缺失时
///   common_dir=gitdir;
/// - root `.git` 为 gitfile 文件(linked)→ 按 `gitdir: <path>` 解析
///   (相对路径锚定 worktree 根),commondir 文件相对 gitdir;
/// - root `.git` 为 symlink/特殊文件 → 拒绝。
/// linked worktree 的 gitdir/gitdir 回指文件存在时一并核对(校验 linked
/// `.git` 指针与真实 gitdir/common-dir)。
fn resolve_git_identity(
    canonical_worktree: &Path,
) -> Result<(PathBuf, PathBuf), ReviewerSnapshotError> {
    let drift = |detail: String| ReviewerSnapshotError::IdentityDrift(detail);
    let dot_git = canonical_worktree.join(".git");
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
            canonical_worktree.join(target_path)
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
    // linked 回指核对(plain 仓无该文件)
    if let Ok(back) = std::fs::read_to_string(gitdir.join("gitdir")) {
        let target = PathBuf::from(back.trim());
        let resolved = if target.is_absolute() {
            target
        } else {
            gitdir.join(target)
        };
        let resolved = resolved
            .canonicalize()
            .map_err(|error| drift(format!("gitdir 回指不可解析: {error}")))?;
        if resolved != canonical_worktree.join(".git") {
            return Err(drift(format!(
                "gitdir 回指 {resolved:?} 与被审 worktree .git 路径不符"
            )));
        }
    }
    Ok((gitdir, common_dir))
}

/// git 对象号(SHA-1 仓为 40-hex;SHA-256 仓为 64-hex)。区别于本模块
/// CAS 内容 hash(恒 64-hex)。
fn is_object_id(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn map_git_error(error: GitCommandError) -> ReviewerSnapshotError {
    ReviewerSnapshotError::Git(error.to_string())
}

// ---------------------------------------------------------------------------
// 平台合法写入域(调用方精确预枚举;本模块只基线核验,不代写)
// ---------------------------------------------------------------------------

fn capture_platform_baseline(
    paths: &[PlatformWritePath],
) -> Result<Vec<PlatformWriteBaselineEntry>, ReviewerSnapshotError> {
    let mut out = Vec::with_capacity(paths.len());
    for spec in paths {
        if spec.ownership.trim().is_empty() {
            return Err(ReviewerSnapshotError::InvalidRequest(
                "平台合法写入域归属必须非空".to_string(),
            ));
        }
        if !spec.path.is_absolute() {
            return Err(ReviewerSnapshotError::InvalidRequest(format!(
                "平台合法写入路径必须为绝对路径: {}",
                spec.path.display()
            )));
        }
        let state = observe_platform_state(&spec.path)?;
        out.push(PlatformWriteBaselineEntry {
            path: spec.path.display().to_string(),
            ownership: spec.ownership.clone(),
            state,
        });
    }
    Ok(out)
}

/// T18(F4):Git 观察域补充面采集/复观(config/objects/reflog/子模块
/// gitdir)。capture 与 T18 恢复后重观察同源调用——摘要不同即逃逸。
pub(crate) fn observe_git_annex(
    gitdir: &Path,
    common_dir: &Path,
) -> Result<GitAnnexObservation, ReviewerSnapshotError> {
    let file_sha = |path: &Path| -> Result<String, ReviewerSnapshotError> {
        let bytes = std::fs::read(path).map_err(|error| {
            ReviewerSnapshotError::ReadFailure(format!("read {}: {error}", path.display()))
        })?;
        Ok(sha256_hex(&bytes))
    };
    // config:gitdir/config 与 common-dir/config(linked worktree 时不同;
    // linked 私有 config 可合法缺失 → 记 "absent" 摘要,与采集同源)。
    let config_sha256 = match std::fs::read(gitdir.join("config")) {
        Ok(bytes) => sha256_hex(&bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent".to_string(),
        Err(error) => {
            return Err(ReviewerSnapshotError::ReadFailure(format!(
                "read {}: {error}",
                gitdir.join("config").display()
            )));
        }
    };
    let common_config_sha256 = match std::fs::read(common_dir.join("config")) {
        Ok(bytes) => sha256_hex(&bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent".to_string(),
        Err(error) => {
            return Err(ReviewerSnapshotError::ReadFailure(format!(
                "read {}: {error}",
                common_dir.join("config").display()
            )));
        }
    };

    // objects:loose 对象相对名(XX/sha…)与 pack 文件名+大小,有序拼接。
    let mut object_lines = Vec::new();
    collect_relative_files(&common_dir.join("objects"), &mut object_lines)?;
    object_lines.sort();
    let objects_digest = sha256_hex(object_lines.join("\n").as_bytes());

    // reflogs:gitdir/logs 与 common-dir/logs 下全部文件 路径+内容sha。
    let mut reflog_lines = Vec::new();
    collect_relative_files(&gitdir.join("logs"), &mut reflog_lines)?;
    if common_dir != gitdir {
        collect_relative_files(&common_dir.join("logs"), &mut reflog_lines)?;
    }
    reflog_lines.sort();
    let reflogs_digest = sha256_hex(reflog_lines.join("\n").as_bytes());

    // 子模块 gitdir:modules 下目录名 + 各 config 内容 sha。
    let mut submodule_lines = Vec::new();
    let modules = common_dir.join("modules");
    if modules.is_dir() {
        let entries = std::fs::read_dir(&modules).map_err(|error| {
            ReviewerSnapshotError::ReadFailure(format!("read {}: {error}", modules.display()))
        })?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ReviewerSnapshotError::ReadFailure(format!("read {}: {error}", modules.display()))
            })?;
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        names.sort();
        for name in names {
            let config_sha = file_sha(&modules.join(&name).join("config"))?;
            submodule_lines.push(format!("{name}:{config_sha}"));
        }
    }
    let submodule_gitdirs_digest = sha256_hex(submodule_lines.join("\n").as_bytes());

    Ok(GitAnnexObservation {
        config_sha256,
        common_config_sha256,
        objects_digest,
        reflogs_digest,
        submodule_gitdirs_digest,
    })
}

/// 递归收集目录下全部文件的「相对路径:内容sha256:大小」行(不追随
/// symlink;目录缺失视为空)。
fn collect_relative_files(root: &Path, out: &mut Vec<String>) -> Result<(), ReviewerSnapshotError> {
    if !root.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(root).map_err(|error| {
        ReviewerSnapshotError::ReadFailure(format!("read {}: {error}", root.display()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            ReviewerSnapshotError::ReadFailure(format!("read {}: {error}", root.display()))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            ReviewerSnapshotError::ReadFailure(format!("stat {}: {error}", path.display()))
        })?;
        if file_type.is_dir() {
            collect_relative_files(&path, out)?;
        } else if file_type.is_file() {
            let bytes = std::fs::read(&path).map_err(|error| {
                ReviewerSnapshotError::ReadFailure(format!("read {}: {error}", path.display()))
            })?;
            let size = bytes.len();
            let sha = sha256_hex(&bytes);
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(format!("{rel}:{sha}:{size}"));
        }
    }
    Ok(())
}

fn observe_platform_state(path: &Path) -> Result<PlatformWriteState, ReviewerSnapshotError> {
    use std::os::unix::fs::FileTypeExt;
    let metadata = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PlatformWriteState::Absent);
        }
        Err(error) => {
            return Err(ReviewerSnapshotError::ReadFailure(format!(
                "stat {}: {error}",
                path.display()
            )));
        }
        Ok(metadata) => metadata,
    };
    let mode = metadata.mode() & 0o7777;
    let file_type = metadata.file_type();
    if file_type.is_symlink()
        || file_type.is_socket()
        || file_type.is_fifo()
        || file_type.is_block_device()
        || file_type.is_char_device()
    {
        return Err(ReviewerSnapshotError::ReadFailure(format!(
            "{}: 形态漂移(symlink/特殊文件)",
            path.display()
        )));
    }
    if file_type.is_dir() {
        return Ok(PlatformWriteState::Directory { mode });
    }
    let bytes = std::fs::read(path).map_err(|error| {
        ReviewerSnapshotError::ReadFailure(format!("read {}: {error}", path.display()))
    })?;
    Ok(PlatformWriteState::File {
        content_sha256: sha256_hex(&bytes),
        mode,
    })
}

// ---------------------------------------------------------------------------
// 恢复执行
// ---------------------------------------------------------------------------

struct RestoreCtx<'a> {
    store: &'a ReviewerSnapshotStore,
    window_dir: PathBuf,
    manifest_path: PathBuf,
    manifest: ReviewerBaselineManifest,
    identity: ReviewWorktreeLockIdentity,
    worktree: PathBuf,
    report: RestoreReport,
}

impl RestoreCtx<'_> {
    /// 游标原子推进(tmp+flush+sync+rename;故障注入挂点=写成功之后)。
    fn advance(
        &mut self,
        phase: RestorePhase,
        entry_index: usize,
    ) -> Result<(), ReviewerSnapshotError> {
        let target = RestoreCursor { phase, entry_index };
        let current = self.manifest.restore_cursor.clone();
        if current == target {
            return Ok(());
        }
        if target.phase.rank() < current.phase.rank()
            || (target.phase == current.phase && target.entry_index < current.entry_index)
        {
            return Err(ReviewerSnapshotError::Integrity(format!(
                "游标回退: {current:?} -> {target:?}"
            )));
        }
        self.manifest.restore_cursor = target;
        write_json(&self.manifest_path, &self.manifest)?;
        #[cfg(test)]
        {
            let tripped = self.store.restore_failpoint_after_writes.fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |value| {
                    if value == usize::MAX {
                        None
                    } else {
                        Some(value.saturating_sub(1))
                    }
                },
            ) == Ok(0);
            if tripped {
                return Err(ReviewerSnapshotError::Io(
                    "reviewer_snapshot_injected_failure: after cursor write".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn blob_bytes(&self, blob_rel: &str) -> Result<Vec<u8>, ReviewerSnapshotError> {
        let path = blob_abs_path(&self.window_dir, blob_rel)?;
        std::fs::read(&path).map_err(|error| {
            ReviewerSnapshotError::Integrity(format!("blob 缺失 {blob_rel}: {error}"))
        })
    }

    /// 打开 rel 的父目录链(逐级 openat no-follow),返回(父 fd, 末级名)。
    fn open_parent_fd(&self, rel: &str) -> Result<(OwnedFd, OsString), ReviewerSnapshotError> {
        validate_rel_path(rel)?;
        let mut current = open_root_dir_no_follow(&self.worktree)
            .map_err(|error| io_error_string("open worktree root", error))?;
        let mut components: Vec<&str> = rel.split('/').collect();
        let name = components.pop().expect("rel 非空");
        for component in components {
            current = openat_dir(&current, OsStr::new(component)).map_err(|error| {
                ReviewerSnapshotError::ReadFailure(format!(
                    "openat 父链 {rel} @ {component}: {error}"
                ))
            })?;
        }
        Ok((current, name.to_string().into()))
    }

    /// 单条目按差异恢复(幂等;no-follow)。返回是否发生修复。
    fn restore_entry(&mut self, entry: &BaselineEntry) -> Result<bool, ReviewerSnapshotError> {
        let (parent, name) = self.open_parent_fd(&entry.rel_path)?;
        let current = fstatat_no_follow(&parent, &name);
        match entry.kind {
            BaselineEntryKind::Directory => {
                match &current {
                    Ok(stat) if classify(stat.st_mode) == Some(BaselineEntryKind::Directory) => {
                        if stat.st_mode & 0o7777 != entry.mode {
                            // F1 修复:chmod 作用于条目自身,不是父目录 fd
                            // (父目录可能是不在 manifest 内的 worktree 根)。
                            let opened = openat_dir(&parent, &name)
                                .map_err(|error| io_error_string("open dir for chmod", error))?;
                            fchmod_dir(&opened, entry.mode)
                                .map_err(|error| io_error_string("chmod dir", error))?;
                            return Ok(true);
                        }
                        return Ok(false);
                    }
                    Ok(_) => {
                        unlinkat_file(&parent, &name)
                            .or_else(|_| unlinkat_dir(&parent, &name))
                            .map_err(|error| io_error_string("移除类型漂移项", error))?;
                    }
                    Err(_) => {}
                }
                mkdirat_dir(&parent, &name, entry.mode)
                    .map_err(|error| io_error_string("mkdir", error))?;
                let opened = openat_dir(&parent, &name)
                    .map_err(|error| io_error_string("open created dir", error))?;
                fchmod_dir(&opened, entry.mode)
                    .map_err(|error| io_error_string("chmod new dir", error))?;
                Ok(true)
            }
            BaselineEntryKind::File | BaselineEntryKind::Gitfile => {
                // 当前态为目录(含 root `.git` 被换成目录)→ 先整删再写
                if let Ok(stat) = &current {
                    if classify(stat.st_mode) == Some(BaselineEntryKind::Directory) {
                        std::fs::remove_dir_all(self.worktree.join(&entry.rel_path))
                            .map_err(|error| io_error_string("移除占位目录", error))?;
                    }
                }
                let needs_write = match &current {
                    Ok(stat) => {
                        classify(stat.st_mode) != Some(BaselineEntryKind::File)
                            || stat.st_mode & 0o7777 != entry.mode
                            || sha256_of_at(&parent, &name).as_deref()
                                != entry.content_sha256.as_deref()
                    }
                    Err(_) => true,
                };
                if !needs_write {
                    return Ok(false);
                }
                let bytes = self.blob_bytes(entry.blob.as_ref().expect("validated"))?;
                write_bytes_atomic(&parent, &name, &bytes, entry.mode, "restore")
                    .map_err(|error| io_error_string(&format!("写 {}", entry.rel_path), error))?;
                Ok(true)
            }
            BaselineEntryKind::Symlink => {
                let target = entry.symlink_target.clone().expect("validated");
                let needs_relink = match &current {
                    Ok(stat) => {
                        classify(stat.st_mode) != Some(BaselineEntryKind::Symlink)
                            || readlinkat_target(&parent, &name)
                                .ok()
                                .and_then(|value| value.to_str().map(str::to_string))
                                .as_deref()
                                != Some(target.as_str())
                    }
                    Err(_) => true,
                };
                if !needs_relink {
                    return Ok(false);
                }
                if let Ok(stat) = &current {
                    if classify(stat.st_mode) == Some(BaselineEntryKind::Directory) {
                        // F2 修复:被非空目录替换时 unlinkat 双双失败
                        // (EISDIR/ENOTEMPTY)会中断且不可续接——与 File 分支
                        // 同形,先整删占位目录再重建 symlink。
                        std::fs::remove_dir_all(self.worktree.join(&entry.rel_path))
                            .map_err(|error| io_error_string("移除目录替换的 symlink", error))?;
                    } else {
                        unlinkat_file(&parent, &name)
                            .map_err(|error| io_error_string("移除旧 symlink 占位", error))?;
                    }
                }
                symlinkat_link(OsStr::new(&target), &parent, &name)
                    .map_err(|error| io_error_string("symlinkat", error))?;
                Ok(true)
            }
        }
    }
}

fn sha256_of_at(parent: &OwnedFd, name: &OsStr) -> Option<String> {
    let mut file = openat_read_no_follow(parent, name).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(sha256_hex(&bytes))
}

/// Git 观察域恢复:受信精确写(HEAD/index=gitdir 内 CAS blob;受管 ref=
/// common-dir 松散 ref 单点写)。objects/reflog 不触碰;新增 ref 不删除
/// (取证),仅记入 report。
fn restore_git_domain(ctx: &mut RestoreCtx<'_>) -> Result<(), ReviewerSnapshotError> {
    let worktree = ctx.worktree.clone();
    let gitdir = ctx.identity.gitdir.clone();
    let common_dir = ctx.identity.common_dir.clone();
    let baseline = ctx.manifest.git_observation.clone();

    let gitdir_fd =
        open_root_dir_no_follow(&gitdir).map_err(|error| io_error_string("open gitdir", error))?;
    // HEAD
    let desired_head = match &baseline.head_symref {
        Some(target) => format!("ref: {target}\n"),
        None => format!("{}\n", baseline.head_sha),
    };
    let mut current_head = String::new();
    if let Ok(mut file) = openat_read_no_follow(&gitdir_fd, OsStr::new("HEAD")) {
        file.read_to_string(&mut current_head)
            .map_err(|error| io_error_string("read HEAD", error))?;
    }
    if current_head != desired_head {
        write_bytes_atomic(
            &gitdir_fd,
            OsStr::new("HEAD"),
            desired_head.as_bytes(),
            0o644,
            "head",
        )
        .map_err(|error| io_error_string("写 HEAD", error))?;
        ctx.report.git_head_restored = true;
    }
    // index(CAS blob 回写)
    let mut current_index_sha = String::new();
    if let Ok(mut file) = openat_read_no_follow(&gitdir_fd, OsStr::new("index")) {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| io_error_string("read index", error))?;
        current_index_sha = sha256_hex(&bytes);
    }
    if current_index_sha != baseline.index_sha256 {
        let bytes = ctx.blob_bytes(&baseline.index_blob)?;
        write_bytes_atomic(&gitdir_fd, OsStr::new("index"), &bytes, 0o644, "index")
            .map_err(|error| io_error_string("写 index", error))?;
        ctx.report.git_index_restored = true;
    }
    // 受管 ref
    let current_refs = list_current_refs(&worktree)?;
    for managed in &baseline.managed_refs {
        let current = current_refs
            .iter()
            .find(|(path, _)| path == &managed.ref_path)
            .map(|(_, sha)| sha.clone());
        if current.as_deref() != Some(managed.sha.as_str()) {
            write_loose_ref(&common_dir, &managed.ref_path, &managed.sha)?;
            ctx.report.git_refs_restored.push(managed.ref_path.clone());
        }
    }
    for (path, _) in &current_refs {
        if !baseline
            .managed_refs
            .iter()
            .any(|managed| &managed.ref_path == path)
        {
            ctx.report.git_extra_refs_observed.push(path.clone());
        }
    }
    Ok(())
}

fn list_current_refs(worktree: &Path) -> Result<Vec<(String, String)>, ReviewerSnapshotError> {
    let raw = git_stdout(
        worktree,
        &args(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
            "refs/tags",
        ]),
    )
    .map_err(map_git_error)?;
    Ok(raw
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (path, sha) = line.split_once(' ')?;
            Some((path.to_string(), sha.to_string()))
        })
        .collect())
}

fn write_loose_ref(
    common_dir: &Path,
    ref_path: &str,
    sha: &str,
) -> Result<(), ReviewerSnapshotError> {
    validate_rel_path(ref_path)?;
    if !ref_path.starts_with("refs/") {
        return Err(ReviewerSnapshotError::Integrity(format!(
            "非 ref 路径: {ref_path}"
        )));
    }
    let target = common_dir.join(ref_path);
    let parent = target.parent().expect("refs 父目录");
    std::fs::create_dir_all(parent).map_err(|error| io_error_string("mkdir refs", error))?;
    let parent_fd = open_root_dir_no_follow(parent)
        .map_err(|error| io_error_string("open refs 父目录", error))?;
    let name = target.file_name().expect("ref 文件名");
    write_bytes_atomic(
        &parent_fd,
        name,
        format!("{sha}\n").as_bytes(),
        0o644,
        "ref",
    )
    .map_err(|error| io_error_string(&format!("写 ref {ref_path}"), error))?;
    Ok(())
}

/// 终验:三域(worktree/Git 观察域/平台合法写入域)全量重对账——不信任
/// 中途状态,也不因快照相同而宣称采信(判定归 T18)。
fn verify_all(
    ctx: &mut RestoreCtx<'_>,
) -> Result<Result<(), (String, String)>, ReviewerSnapshotError> {
    // worktree 域
    let mut removed = Vec::new();
    let walked = {
        let mut purpose = WalkPurpose::Observe {
            delete_extras: false,
            manifest_rels: None,
            removed: &mut removed,
        };
        walk_worktree(&ctx.worktree, &mut purpose)?
    };
    if walked.len() != ctx.manifest.entries.len() {
        let walked_rels: Vec<&str> = walked.iter().map(|entry| entry.rel_path.as_str()).collect();
        let manifest_rels: Vec<&str> = ctx
            .manifest
            .entries
            .iter()
            .map(|entry| entry.rel_path.as_str())
            .collect();
        return Ok(Err((
            "worktree_entries_mismatch".to_string(),
            format!("当前 {walked_rels:?} vs manifest {manifest_rels:?}"),
        )));
    }
    for (walked, entry) in walked.iter().zip(ctx.manifest.entries.iter()) {
        if walked.rel_path != entry.rel_path
            || walked.kind != entry.kind
            || walked.mode != entry.mode
            || walked.content_sha256 != entry.content_sha256
            || walked.symlink_target != entry.symlink_target
        {
            return Ok(Err((
                "worktree_entry_mismatch".to_string(),
                format!("{}: 当前 {walked:?} != 基线 {entry:?}", entry.rel_path),
            )));
        }
    }
    // Git 观察域
    let head = git_stdout(&ctx.worktree, &args(&["rev-parse", "HEAD"])).map_err(map_git_error)?;
    if head != ctx.manifest.git_observation.head_sha {
        return Ok(Err(("git_head_mismatch".to_string(), head)));
    }
    let symref = match run_git(&ctx.worktree, &args(&["symbolic-ref", "-q", "HEAD"])) {
        Ok(record) => Some(record.stdout.trim().to_string()),
        Err(GitCommandError::Failed { record }) if record.exit_code == Some(1) => None,
        Err(error) => return Err(map_git_error(error)),
    };
    if symref != ctx.manifest.git_observation.head_symref {
        return Ok(Err((
            "git_symref_mismatch".to_string(),
            format!("{symref:?}"),
        )));
    }
    let current_refs = list_current_refs(&ctx.worktree)?;
    for managed in &ctx.manifest.git_observation.managed_refs {
        let current = current_refs
            .iter()
            .find(|(path, _)| path == &managed.ref_path)
            .map(|(_, sha)| sha.as_str());
        if current != Some(managed.sha.as_str()) {
            return Ok(Err((
                "git_ref_mismatch".to_string(),
                managed.ref_path.clone(),
            )));
        }
    }
    let gitdir_fd = open_root_dir_no_follow(&ctx.identity.gitdir)
        .map_err(|error| io_error_string("open gitdir", error))?;
    let mut index_bytes = Vec::new();
    if let Ok(mut file) = openat_read_no_follow(&gitdir_fd, OsStr::new("index")) {
        file.read_to_end(&mut index_bytes)
            .map_err(|error| io_error_string("read index", error))?;
    }
    if sha256_hex(&index_bytes) != ctx.manifest.git_observation.index_sha256 {
        return Ok(Err((
            "git_index_mismatch".to_string(),
            "index sha 不符".to_string(),
        )));
    }
    // 平台合法写入域
    for baseline in &ctx.manifest.platform_write_baseline {
        let path = PathBuf::from(&baseline.path);
        let drifted = match observe_platform_state(&path) {
            Ok(state) => state != baseline.state,
            Err(_) => true,
        };
        if drifted {
            return Ok(Err((
                "platform_domain_drift".to_string(),
                format!("{} (归属 {})", baseline.path, baseline.ownership),
            )));
        }
        ctx.report.platform_verified.push(baseline.path.clone());
    }
    Ok(Ok(()))
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};

    use super::*;

    // ---------- fixtures ----------

    fn t16_dir() -> tempfile::TempDir {
        let root = Path::new("/tmp/aria-bug45/t16");
        fs::create_dir_all(root).expect("create /tmp/aria-bug45/t16");
        tempfile::tempdir_in(root).expect("t16 fixture dir")
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

    fn git_text(dir: &Path, args: &[&str]) -> String {
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
        String::from_utf8(out.stdout)
            .expect("utf8 git output")
            .trim()
            .to_string()
    }

    fn git_identity_config(dir: &Path) {
        git_ok(dir, &["config", "user.email", "t16@aria.invalid"]);
        git_ok(dir, &["config", "user.name", "t16-fixture"]);
    }

    fn chmod(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod fixture");
    }

    fn mode_of(path: &Path) -> u32 {
        fs::symlink_metadata(path)
            .expect("stat fixture")
            .permissions()
            .mode()
            & 0o7777
    }

    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Layout {
        Plain,
        Linked,
        Submodule,
    }

    struct Fixture {
        worktree: PathBuf,
        store_root: PathBuf,
        platform_marker: PathBuf,
        // 保持 fixture 存活
        _base: tempfile::TempDir,
        _store: tempfile::TempDir,
        _platform: tempfile::TempDir,
    }

    fn base_commit(worktree: &Path) {
        fs::write(worktree.join("tracked.txt"), "base\n").unwrap();
        fs::write(worktree.join("dirty.txt"), "v0\n").unwrap();
        fs::write(worktree.join("gone.txt"), "will be deleted\n").unwrap();
        fs::write(worktree.join("run.sh"), "#!/bin/sh\necho ok\n").unwrap();
        chmod(&worktree.join("run.sh"), 0o755);
        fs::write(worktree.join("target.txt"), "target-content\n").unwrap();
        symlink("target.txt", worktree.join("link")).unwrap();
        fs::write(worktree.join(".gitignore"), "ignored.log\n").unwrap();
        git_ok(worktree, &["add", "-A"]);
        git_ok(worktree, &["commit", "-qm", "base"]);
    }

    /// Coder 的 dirty 矩阵:staged+unstaged 同文件、untracked/ignored、
    /// 已删除、可执行位、symlink、空目录、指向 worktree 外的 symlink。
    fn dirty_matrix(worktree: &Path) {
        fs::write(worktree.join("dirty.txt"), "v1-staged\n").unwrap();
        git_ok(worktree, &["add", "dirty.txt"]);
        fs::write(worktree.join("dirty.txt"), "v2-worktree\n").unwrap();
        fs::write(worktree.join("untracked_new.txt"), "keep-me\n").unwrap();
        fs::write(worktree.join("ignored.log"), "secret\n").unwrap();
        fs::remove_file(worktree.join("gone.txt")).unwrap();
        fs::create_dir(worktree.join("empty_dir")).unwrap();
        fs::create_dir_all(worktree.join("nested/dir")).unwrap();
        symlink("/etc/hostname", worktree.join("escape")).unwrap();
    }

    /// Reviewer 会话副作用:构建残留、改 dirty、翻转 mode、重指 symlink、
    /// 弄脏空目录、stage+commit(移动 HEAD/分支)、新增取证分支。
    /// 返回 Reviewer 提交 sha。
    fn reviewer_mutations(worktree: &Path) -> String {
        fs::write(worktree.join("dirty.txt"), "reviewer-mangled\n").unwrap();
        git_ok(worktree, &["add", "dirty.txt"]);
        fs::create_dir(worktree.join("build")).unwrap();
        fs::write(worktree.join("build/artifact.bin"), "reviewer build junk").unwrap();
        fs::write(worktree.join("untracked_new.txt"), "reviewer touched\n").unwrap();
        chmod(&worktree.join("run.sh"), 0o644);
        chmod(&worktree.join("tracked.txt"), 0o755);
        fs::remove_file(worktree.join("link")).unwrap();
        symlink("evil.txt", worktree.join("link")).unwrap();
        fs::write(worktree.join("empty_dir/.junk"), "junk\n").unwrap();
        // F1 行:既有嵌套目录 chmod 漂移(恢复须改条目自身,不动父目录)
        chmod(&worktree.join("nested/dir"), 0o700);
        // F2 行:基线 symlink 被非空目录替换(恢复须整删占位后重建)
        fs::remove_file(worktree.join("escape")).unwrap();
        fs::create_dir(worktree.join("escape")).unwrap();
        fs::write(worktree.join("escape/junk.txt"), "replaced\n").unwrap();
        if worktree.join("sub").is_dir() {
            fs::write(worktree.join("sub/c.txt"), "reviewer touched child\n").unwrap();
        }
        git_ok(worktree, &["add", "-A"]);
        git_ok(worktree, &["commit", "-qm", "reviewer-change"]);
        git_ok(worktree, &["branch", "-f", "reviewer-evidence"]);
        git_text(worktree, &["rev-parse", "HEAD"])
    }

    fn build_fixture(layout: Layout) -> Fixture {
        let base = t16_dir();
        let store = t16_dir();
        let platform = t16_dir();
        let worktree: PathBuf = match layout {
            Layout::Plain | Layout::Submodule => base.path().join("repo"),
            Layout::Linked => base.path().join("linked"),
        };
        match layout {
            Layout::Plain => {
                fs::create_dir_all(&worktree).unwrap();
                git_ok(&worktree, &["init", "-q", "-b", "main"]);
                git_identity_config(&worktree);
                base_commit(&worktree);
            }
            Layout::Linked => {
                let main = base.path().join("main");
                fs::create_dir_all(&main).unwrap();
                git_ok(&main, &["init", "-q", "-b", "main"]);
                git_identity_config(&main);
                base_commit(&main);
                git_ok(
                    &main,
                    &[
                        "worktree",
                        "add",
                        "-q",
                        "-b",
                        "linked-br",
                        worktree.to_str().unwrap(),
                    ],
                );
            }
            Layout::Submodule => {
                let child = base.path().join("child");
                fs::create_dir_all(&child).unwrap();
                git_ok(&child, &["init", "-q", "-b", "main"]);
                git_identity_config(&child);
                fs::write(child.join("c.txt"), "child-content\n").unwrap();
                git_ok(&child, &["add", "."]);
                git_ok(&child, &["commit", "-qm", "child base"]);
                fs::create_dir_all(&worktree).unwrap();
                git_ok(&worktree, &["init", "-q", "-b", "main"]);
                git_identity_config(&worktree);
                base_commit(&worktree);
                git_ok(
                    &worktree,
                    &[
                        "-c",
                        "protocol.file.allow=always",
                        "submodule",
                        "add",
                        "-q",
                        "--name",
                        "sub",
                        child.to_str().unwrap(),
                        "sub",
                    ],
                );
                git_ok(&worktree, &["commit", "-qm", "add submodule"]);
            }
        }
        dirty_matrix(&worktree);

        let platform_marker = platform.path().join("legit-marker.json");
        fs::write(&platform_marker, "stable\n").unwrap();

        Fixture {
            worktree,
            store_root: store.path().to_path_buf(),
            platform_marker,
            _base: base,
            _store: store,
            _platform: platform,
        }
    }

    fn lock_identity(worktree: &Path, lease: &str) -> ReviewWorktreeLockIdentity {
        let canonical = worktree.canonicalize().expect("canonical worktree");
        let (gitdir, common_dir) = resolve_git_identity(&canonical).expect("resolve git identity");
        ReviewWorktreeLockIdentity {
            canonical_worktree: canonical,
            gitdir,
            common_dir,
            lease_id: lease.to_string(),
        }
    }

    fn capture_request<'a>(
        fixture: &'a Fixture,
        window: &'a str,
        identity: &'a ReviewWorktreeLockIdentity,
        evidence: &'a QuiescenceEvidence,
    ) -> CaptureRequest<'a> {
        CaptureRequest {
            window_id: window,
            attempt_id: "att-1",
            run_id: "run-1",
            worktree: &fixture.worktree,
            lock_identity: Some(identity.clone()),
            quiescence: Some(evidence.clone()),
            platform_write_paths: vec![PlatformWritePath {
                path: fixture.platform_marker.clone(),
                ownership: "platform:test-store".to_string(),
            }],
            budget: SnapshotBudget::generous(),
        }
    }

    // 真实 T5 静止证明:空 scope 的 quiesce_scope 立即静止。
    async fn quiescence_evidence(window_id: &str) -> QuiescenceEvidence {
        use crate::cross_cutting::provider_execution::{
            FrozenProviderExecution, ProviderExecutionProfile, TrustedProviderRole,
            TrustedRunIdentity,
        };
        use crate::cross_cutting::provider_resources::ProviderRunResourceScope;
        use crate::protocol::contracts::{
            AdapterRole, ProviderRunRecord, ProviderRunStatus, ProviderType, RuntimeRole,
        };

        let private_root = t16_dir();
        let store_root = t16_dir();
        let record = ProviderRunRecord {
            provider_run_id: "run-t16".to_string(),
            node_id: "N2".to_string(),
            provider_type: ProviderType::ClaudeCode,
            runtime_role: RuntimeRole::Orchestrator,
            adapter_role: AdapterRole::Executor,
            provider_capability_ref: "cap_1".to_string(),
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
            status: ProviderRunStatus::Running,
            started_at: "2026-10-17T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
            retry_count: 0,
            approval_policy: crate::protocol::contracts::ApprovalPolicy::OnRequest,
            sandbox_mode: crate::protocol::contracts::SandboxMode::FullAccess,
            constraint_check_ref: None,
            traceability_binding_refs: Vec::new(),
        };
        let identity = TrustedRunIdentity::from_durable_run(
            &record,
            "ws_session_t16",
            TrustedProviderRole::Reviewer,
        );
        let contract: FrozenProviderExecution =
            crate::cross_cutting::provider_execution::freeze_provider_execution(
                &identity,
                ProviderExecutionProfile::ReviewerVerify,
                None,
                "scope-t16".to_string(),
                Some(window_id.to_string()),
            )
            .expect("freeze contract");
        let scope = ProviderRunResourceScope::open(
            &contract,
            "boot-t16",
            private_root.path(),
            store_root.path(),
            None,
        )
        .expect("open scope");
        match scope.quiesce_scope().await.expect("quiesce") {
            crate::cross_cutting::provider_resources::QuiesceOutcome::Quiescent(quiescent) => {
                QuiescenceEvidence::from_quiescent_scope(&quiescent)
            }
            crate::cross_cutting::provider_resources::QuiesceOutcome::RecoveryRequired(unknown) => {
                panic!("fixture quiesce recovery: {}", unknown.reason)
            }
        }
    }

    fn tree_listing(root: &Path) -> Vec<String> {
        fn rec(dir: &Path, prefix: &str, out: &mut Vec<String>) {
            let mut names: Vec<String> = fs::read_dir(dir)
                .expect("read_dir listing")
                .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
                .collect();
            names.sort();
            for name in names {
                let rel = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                out.push(rel.clone());
                let meta = fs::symlink_metadata(dir.join(&name)).expect("stat");
                if meta.is_dir() && name != ".git" {
                    rec(&dir.join(&name), &rel, out);
                }
            }
        }
        let mut out = Vec::new();
        rec(root, "", &mut out);
        out
    }

    fn sha256_file(path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let bytes = fs::read(path).expect("read for hash");
        hex::encode(Sha256::digest(&bytes))
    }

    fn restore_report(outcome: RestoreOutcome) -> RestoreReport {
        match outcome {
            RestoreOutcome::Restored(report) => report,
            RestoreOutcome::RecoveryRequired { reason, detail, .. } => {
                panic!("expected Restored, got RecoveryRequired: {reason}: {detail}")
            }
        }
    }

    // ---------- 断言矩阵(worktree 域逐项) ----------

    fn assert_dirty_matrix_restored(fixture: &Fixture) {
        let w = &fixture.worktree;
        assert_eq!(
            fs::read_to_string(w.join("dirty.txt")).unwrap(),
            "v2-worktree\n"
        );
        assert!(
            !w.join("gone.txt").exists(),
            "Coder 的删除态必须保留:worktree 缺席,index/HEAD 仍跟踪(porcelain ' D')"
        );
        assert_eq!(mode_of(&w.join("run.sh")), 0o755);
        assert_eq!(mode_of(&w.join("tracked.txt")), 0o644);
        assert_eq!(
            fs::read_link(w.join("link")).unwrap().to_string_lossy(),
            "target.txt"
        );
        assert!(w.join("empty_dir").is_dir());
        assert_eq!(
            fs::read_dir(w.join("empty_dir")).unwrap().count(),
            0,
            "空目录必须恢复为空"
        );
        // F1 行:嵌套目录 chmod 漂移——条目自身回基线,父目录不被改写
        assert!(w.join("nested/dir").is_dir());
        assert_eq!(
            mode_of(&w.join("nested/dir")),
            0o755,
            "目录条目 mode 漂移必须修复到条目自身"
        );
        assert_eq!(
            mode_of(&w.join("nested")),
            0o755,
            "父目录 mode 不得被 chmod 波及"
        );
        assert_eq!(
            fs::read_to_string(w.join("untracked_new.txt")).unwrap(),
            "keep-me\n"
        );
        assert_eq!(
            fs::read_to_string(w.join("ignored.log")).unwrap(),
            "secret\n"
        );
        assert_eq!(
            fs::read_link(w.join("escape")).unwrap().to_string_lossy(),
            "/etc/hostname",
            "指向 owner 域的 symlink 按指针恢复,不追随(F2:被非空目录替换后重建)"
        );
        assert!(
            !w.join("escape/junk.txt").exists(),
            "F2:占位目录内容必须整删"
        );
        assert!(!w.join("build").exists(), "Reviewer 构建残留必须删除");
        assert!(!w.join("empty_dir/.junk").exists());
    }

    // ---------- 红测①:准入拒绝(无锁身份/无静止证明) ----------

    #[tokio::test]
    async fn capture_without_lock_identity_or_quiescence_is_rejected() {
        let fixture = build_fixture(Layout::Plain);
        let evidence = quiescence_evidence("win-gate").await;
        let store = ReviewerSnapshotStore::new(&fixture.store_root);

        let req_no_identity = CaptureRequest {
            window_id: "win-gate",
            attempt_id: "att-1",
            run_id: "run-1",
            worktree: &fixture.worktree,
            lock_identity: None,
            quiescence: Some(evidence.clone()),
            platform_write_paths: vec![],
            budget: SnapshotBudget::generous(),
        };
        let err = store
            .capture_baseline(req_no_identity)
            .expect_err("无锁身份必须拒绝");
        assert!(matches!(
            err,
            ReviewerSnapshotError::LockIdentityRequired(_)
        ));

        let identity = lock_identity(&fixture.worktree, "lease-gate");
        let req_no_quiescence = CaptureRequest {
            window_id: "win-gate",
            attempt_id: "att-1",
            run_id: "run-1",
            worktree: &fixture.worktree,
            lock_identity: Some(identity),
            quiescence: None,
            platform_write_paths: vec![],
            budget: SnapshotBudget::generous(),
        };
        let err = store
            .capture_baseline(req_no_quiescence)
            .expect_err("无静止证明必须拒绝");
        assert!(matches!(err, ReviewerSnapshotError::QuiescenceRequired(_)));

        // 无部分基线发布
        assert!(!store.manifest_path("win-gate").exists());
    }

    // ---------- 红测②:恢复矩阵(表驱动 plain/linked/submodule) ----------

    #[tokio::test]
    async fn restore_matrix_preserves_coder_dirty_state() {
        for layout in [Layout::Plain, Layout::Linked, Layout::Submodule] {
            let fixture = build_fixture(layout);
            let identity = lock_identity(&fixture.worktree, "lease-t16-1");
            let evidence = quiescence_evidence("win-1").await;
            let store = ReviewerSnapshotStore::new(&fixture.store_root);

            let pre_listing = tree_listing(&fixture.worktree);
            let pre_porcelain = git_text(
                &fixture.worktree,
                &["--no-optional-locks", "status", "--porcelain"],
            );

            let captured = store
                .capture_baseline(capture_request(&fixture, "win-1", &identity, &evidence))
                .expect("capture baseline");
            assert!(captured.manifest_path.starts_with(&fixture.store_root));
            assert!(!fixture.worktree.join("manifest.json").exists());
            assert_eq!(
                tree_listing(&fixture.worktree),
                pre_listing,
                "capture 绝不向被审 worktree 写快照产物"
            );

            let manifest = store.load_manifest("win-1").expect("load manifest");
            let baseline_index_sha = sha256_file(&identity.gitdir.join("index"));
            let baseline_head = git_text(&fixture.worktree, &["rev-parse", "HEAD"]);
            let baseline_symref = git_text(&fixture.worktree, &["symbolic-ref", "HEAD"]);
            let reflog_path = identity.gitdir.join("logs/HEAD");
            let pre_reflog = fs::read(&reflog_path).unwrap_or_default();

            // Reviewer 会话
            let reviewer_sha = reviewer_mutations(&fixture.worktree);
            let post_reviewer_reflog = fs::read(&reflog_path).unwrap_or_default();
            assert!(
                post_reviewer_reflog.len() > pre_reflog.len(),
                "reviewer 提交应留下 reflog 观察事实"
            );
            assert_ne!(
                git_text(&fixture.worktree, &["rev-parse", "HEAD"]),
                baseline_head
            );

            // 恢复
            let report =
                restore_report(store.restore_baseline("win-1", &identity).expect("restore"));
            assert_dirty_matrix_restored(&fixture);

            // staged+unstaged/删除/ignored 全保留:porcelain 与基线一致
            assert_eq!(
                git_text(
                    &fixture.worktree,
                    &["--no-optional-locks", "status", "--porcelain"]
                ),
                pre_porcelain,
                "layout={layout:?} dirty 状态必须逐项保留"
            );
            // Git 观察域:HEAD/index/受管 ref 恢复
            assert_eq!(
                git_text(&fixture.worktree, &["rev-parse", "HEAD"]),
                baseline_head
            );
            assert_eq!(
                sha256_file(&identity.gitdir.join("index")),
                baseline_index_sha
            );
            assert_eq!(
                git_text(&fixture.worktree, &["symbolic-ref", "HEAD"]),
                baseline_symref
            );
            // objects/reflog 不抹;取证材料保留
            git_ok(&fixture.worktree, &["cat-file", "-e", &reviewer_sha]);
            assert_eq!(
                fs::read(&reflog_path).unwrap_or_default(),
                post_reviewer_reflog,
                "reflog 是观察事实,恢复不得抹除或改写(reviewer 记录留取证)"
            );
            git_ok(
                &fixture.worktree,
                &["rev-parse", "--verify", "reviewer-evidence"],
            );
            assert!(
                report
                    .git_extra_refs_observed
                    .contains(&"refs/heads/reviewer-evidence".to_string()),
                "新增 ref 留作取证并在报告观察: {:?}",
                report.git_extra_refs_observed
            );

            // 子模块 gitdir:gitfile 与子模块工作区内容逐字节保留
            if layout == Layout::Submodule {
                assert_eq!(
                    fs::read_to_string(fixture.worktree.join("sub/.git")).unwrap(),
                    "gitdir: ../.git/modules/sub\n",
                    "submodule gitfile 逐字节恢复"
                );
                assert_eq!(
                    fs::read_to_string(fixture.worktree.join("sub/c.txt")).unwrap(),
                    "child-content\n"
                );
            }

            // 域级字节相等:重采 win-2 与 win-1 manifest 域内容一致
            let captured2 = store
                .capture_baseline(capture_request(&fixture, "win-2", &identity, &evidence))
                .expect("recapture");
            let manifest2 = store.load_manifest("win-2").expect("load manifest2");
            assert_eq!(
                manifest2.entries, manifest.entries,
                "layout={layout:?} worktree 域逐字节相等"
            );
            // Git 观察域:HEAD/index 恢复相等;受管 ref 恢复到基线值
            // (win-2 的 managed_refs 会多出取证分支,属新增观察事实,不强等)
            assert_eq!(
                manifest2.git_observation.head_sha,
                manifest.git_observation.head_sha
            );
            assert_eq!(
                manifest2.git_observation.head_symref,
                manifest.git_observation.head_symref
            );
            assert_eq!(
                manifest2.git_observation.index_sha256,
                manifest.git_observation.index_sha256
            );
            for managed in &manifest.git_observation.managed_refs {
                let restored = manifest2
                    .git_observation
                    .managed_refs
                    .iter()
                    .find(|entry| entry.ref_path == managed.ref_path)
                    .expect("基线受管 ref 在 win-2 仍在");
                assert_eq!(restored.sha, managed.sha, "ref {}", managed.ref_path);
            }
            assert_eq!(
                manifest2.platform_write_baseline,
                manifest.platform_write_baseline
            );
            assert_eq!(captured2.entry_count, captured.entry_count);

            // 幂等:Completed 游标上重放恢复
            restore_report(
                store
                    .restore_baseline("win-1", &identity)
                    .expect("restore idempotent"),
            );
            assert_dirty_matrix_restored(&fixture);
        }
    }

    // ---------- 红测③:capture 拒绝(预算/不支持项/路径漂移) ----------

    #[tokio::test]
    async fn capture_rejects_budget_unsupported_and_drift() {
        let fixture = build_fixture(Layout::Plain);
        let identity = lock_identity(&fixture.worktree, "lease-t16-2");
        let evidence = quiescence_evidence("win-reject").await;
        let store = ReviewerSnapshotStore::new(&fixture.store_root);

        // 预算超限:max_entries=3(dirty 矩阵远超)→ 整单拒绝,无部分基线
        let mut req = capture_request(&fixture, "win-budget", &identity, &evidence);
        req.budget = SnapshotBudget {
            max_entries: 3,
            max_total_bytes: u64::MAX,
        };
        let err = store.capture_baseline(req).expect_err("预算超限必须拒绝");
        assert!(matches!(err, ReviewerSnapshotError::BudgetExceeded(_)));
        assert!(!store.manifest_path("win-budget").exists());
        assert!(
            !store.window_dir("win-budget").join("blobs").exists(),
            "未发布窗口目录必须清理"
        );

        // 字节预算:单文件超限同样拒绝
        let mut req = capture_request(&fixture, "win-bytes", &identity, &evidence);
        req.budget = SnapshotBudget {
            max_entries: 100_000,
            max_total_bytes: 4,
        };
        let err = store
            .capture_baseline(req)
            .expect_err("字节预算超限必须拒绝");
        assert!(matches!(err, ReviewerSnapshotError::BudgetExceeded(_)));
        assert!(!store.manifest_path("win-bytes").exists());

        // 不支持项:FIFO
        let fifo = fixture.worktree.join("pipe");
        unsafe {
            let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
            assert_eq!(libc::mkfifo(c.as_ptr(), 0o644), 0);
        }
        let err = store
            .capture_baseline(capture_request(&fixture, "win-fifo", &identity, &evidence))
            .expect_err("FIFO 必须作为不支持项拒绝");
        assert!(matches!(err, ReviewerSnapshotError::UnsupportedEntry(_)));
        assert!(!store.manifest_path("win-fifo").exists());
        fs::remove_file(&fifo).unwrap();

        // 路径漂移:identity 的 canonical worktree 与实际树不一致 → 拒绝
        let mut drifted = identity.clone();
        drifted.canonical_worktree = fixture.worktree.parent().unwrap().join("elsewhere");
        let err = store
            .capture_baseline(capture_request(
                &fixture,
                "win-drift-cap",
                &drifted,
                &evidence,
            ))
            .expect_err("canonical 路径漂移必须拒绝");
        assert!(matches!(err, ReviewerSnapshotError::IdentityDrift(_)));
        assert!(!store.manifest_path("win-drift-cap").exists());

        // 恢复侧漂移:manifest 冻结身份与传入身份不一致
        store
            .capture_baseline(capture_request(&fixture, "win-drift", &identity, &evidence))
            .expect("capture for drift");
        let mut other = identity.clone();
        other.lease_id = "lease-other".to_string();
        let err = store
            .restore_baseline("win-drift", &other)
            .expect_err("身份不一致必须拒绝");
        assert!(matches!(err, ReviewerSnapshotError::IdentityDrift(_)));
    }

    // ---------- 红测④:restore 预检拒绝(manifest/blob 缺失、hash 错) ----------

    #[tokio::test]
    async fn restore_rejects_missing_manifest_blob_and_hash_mismatch() {
        let fixture = build_fixture(Layout::Plain);
        let identity = lock_identity(&fixture.worktree, "lease-t16-3");
        let evidence = quiescence_evidence("win-integrity").await;
        let store = ReviewerSnapshotStore::new(&fixture.store_root);

        // manifest 缺失
        let err = store
            .restore_baseline("win-nope", &identity)
            .expect_err("缺失 manifest 必须拒绝");
        assert!(matches!(
            err,
            ReviewerSnapshotError::Store(ProductStoreError::NotFound { .. })
        ));

        store
            .capture_baseline(capture_request(
                &fixture,
                "win-integrity",
                &identity,
                &evidence,
            ))
            .expect("capture");
        reviewer_mutations(&fixture.worktree);
        // reviewer 残留在场:预检拒绝不得改动被审 worktree

        // blob 缺失
        let manifest = store.load_manifest("win-integrity").unwrap();
        let blob_file = store.window_dir("win-integrity").join(
            manifest
                .entries
                .iter()
                .find(|e| e.blob.is_some())
                .unwrap()
                .blob
                .as_ref()
                .unwrap(),
        );
        fs::remove_file(&blob_file).unwrap();
        let err = store
            .restore_baseline("win-integrity", &identity)
            .expect_err("blob 缺失必须拒绝");
        assert!(matches!(err, ReviewerSnapshotError::Integrity(_)));
        assert!(
            fixture.worktree.join("build").exists(),
            "预检拒绝不得改动被审 worktree"
        );

        // hash 错:恢复 blob 但内容不符
        fs::write(&blob_file, b"tampered blob content").unwrap();
        let err = store
            .restore_baseline("win-integrity", &identity)
            .expect_err("hash 错必须拒绝");
        assert!(matches!(err, ReviewerSnapshotError::Integrity(_)));
        assert!(fixture.worktree.join("build").exists());
    }

    // ---------- 红测⑤:游标边界故障注入后从原 manifest 续接 ----------

    #[tokio::test]
    async fn restore_resumes_from_original_manifest_at_every_cursor_boundary() {
        let fixture = build_fixture(Layout::Plain);
        let identity = lock_identity(&fixture.worktree, "lease-t16-4");
        let evidence = quiescence_evidence("win-resume").await;
        let store = ReviewerSnapshotStore::new(&fixture.store_root);

        let baseline_head = git_text(&fixture.worktree, &["rev-parse", "HEAD"]);
        let baseline_porcelain = git_text(
            &fixture.worktree,
            &["--no-optional-locks", "status", "--porcelain"],
        );

        // 每轮独立窗口:基线树 → 采新 manifest → Reviewer 副作用 → 第 n 次
        // 游标写后故障 → 重启从「原 manifest」续接。游标边界 0..T-1 逐个
        // 注入,T 为该窗口总游标写数(同型树恒定)。
        let mut n: usize = 0;
        let mut faulted_any = false;
        loop {
            assert!(n < 200, "游标写次数超出测试上限");
            let window = format!("win-resume-{n}");
            store
                .capture_baseline(capture_request(&fixture, &window, &identity, &evidence))
                .expect("capture");
            let manifest = store.load_manifest(&window).unwrap();
            let frozen_domain = serde_json::to_string(&(
                &manifest.entries,
                &manifest.git_observation,
                &manifest.platform_write_baseline,
            ))
            .unwrap();

            reviewer_mutations(&fixture.worktree);
            let faulted = ReviewerSnapshotStore::new(&fixture.store_root);
            faulted.set_restore_failpoint_after_cursor_writes(n);
            match faulted.restore_baseline(&window, &identity) {
                Err(ReviewerSnapshotError::Io(detail))
                    if detail.contains("reviewer_snapshot_injected_failure") =>
                {
                    faulted_any = true;
                    // 重启:新实例无故障点,从原 manifest 续接
                    let cursor_at_fault = faulted
                        .load_manifest(&window)
                        .unwrap()
                        .restore_cursor
                        .clone();
                    let resumed = ReviewerSnapshotStore::new(&fixture.store_root);
                    let report = restore_report(
                        resumed
                            .restore_baseline(&window, &identity)
                            .expect("续接恢复"),
                    );
                    if cursor_at_fault.phase != RestorePhase::Completed {
                        assert_ne!(
                            report.resumed_from.phase,
                            RestorePhase::Completed,
                            "故障后重启必须从原游标续接(而非整段重跑)"
                        );
                    }
                    assert_dirty_matrix_restored(&fixture);
                    assert_eq!(
                        git_text(&fixture.worktree, &["rev-parse", "HEAD"]),
                        baseline_head,
                        "续接后 HEAD 必须回到基线"
                    );
                    assert_eq!(
                        git_text(
                            &fixture.worktree,
                            &["--no-optional-locks", "status", "--porcelain"]
                        ),
                        baseline_porcelain,
                        "续接后 dirty 状态必须逐项保留"
                    );
                    // 原窗口 manifest 域内容不变(只有 restore_cursor 前进)
                    let final_manifest = store.load_manifest(&window).unwrap();
                    assert_eq!(
                        serde_json::to_string(&(
                            &final_manifest.entries,
                            &final_manifest.git_observation,
                            &final_manifest.platform_write_baseline,
                        ))
                        .unwrap(),
                        frozen_domain,
                        "续接必须以原 manifest 为准,不采部分基线"
                    );
                    assert_eq!(final_manifest.restore_cursor.phase, RestorePhase::Completed);
                }
                Ok(_) => break, // n ≥ 总游标写数:无故障命中,边界覆盖完成
                other => panic!("第 {n} 次故障注入后出现意外结局: {other:?}"),
            }
            n += 1;
        }
        assert!(faulted_any && n >= 1, "至少注入一次游标边界故障");
        assert_dirty_matrix_restored(&fixture);
    }

    // ---------- 红测⑥:平台合法写入域漂移 → RecoveryRequired ----------

    #[tokio::test]
    async fn platform_domain_drift_is_recovery_required_not_fake_pass() {
        let fixture = build_fixture(Layout::Plain);
        let identity = lock_identity(&fixture.worktree, "lease-t16-5");
        let evidence = quiescence_evidence("win-platform").await;
        let store = ReviewerSnapshotStore::new(&fixture.store_root);
        store
            .capture_baseline(capture_request(
                &fixture,
                "win-platform",
                &identity,
                &evidence,
            ))
            .expect("capture");
        reviewer_mutations(&fixture.worktree);

        // Reviewer 写平台合法写入域 → 无法安全恢复:停等
        fs::write(&fixture.platform_marker, "tampered-by-reviewer\n").unwrap();
        match store
            .restore_baseline("win-platform", &identity)
            .expect("restore 结局")
        {
            RestoreOutcome::RecoveryRequired { reason, cursor, .. } => {
                assert!(reason.contains("platform"), "reason={reason}");
                assert_eq!(
                    cursor.phase,
                    RestorePhase::PlatformDomain,
                    "游标停在平台域,不伪称三域通过"
                );
            }
            RestoreOutcome::Restored(report) => {
                panic!("平台域漂移不得伪称恢复完成: {report:?}")
            }
        }
        // worktree 域已恢复(停等不回滚已完成域),但 Completed 未达成
        assert_dirty_matrix_restored(&fixture);
        let manifest = store.load_manifest("win-platform").unwrap();
        assert_ne!(manifest.restore_cursor.phase, RestorePhase::Completed);

        // 人工/T18 处置后重入:从 PlatformDomain 续接完成
        fs::write(&fixture.platform_marker, "stable\n").unwrap();
        restore_report(
            store
                .restore_baseline("win-platform", &identity)
                .expect("重入恢复"),
        );
        let manifest = store.load_manifest("win-platform").unwrap();
        assert_eq!(manifest.restore_cursor.phase, RestorePhase::Completed);
    }

    // ---------- 红测⑦:worktree 移动/gitfile 篡改 → 漂移拒绝 ----------

    #[tokio::test]
    async fn restore_rejects_worktree_path_and_gitfile_drift() {
        let fixture = build_fixture(Layout::Linked);
        let identity = lock_identity(&fixture.worktree, "lease-t16-6");
        let evidence = quiescence_evidence("win-move").await;
        let store = ReviewerSnapshotStore::new(&fixture.store_root);
        store
            .capture_baseline(capture_request(&fixture, "win-move", &identity, &evidence))
            .expect("capture");
        reviewer_mutations(&fixture.worktree);

        // gitfile 篡改:linked `.git` 指针漂移 → 拒绝(不盲写身份锚点)
        fs::write(
            fixture.worktree.join(".git"),
            "gitdir: /tmp/aria-bug45/t16/evil",
        )
        .unwrap();
        let err = store
            .restore_baseline("win-move", &identity)
            .expect_err("gitfile 漂移必须拒绝");
        assert!(matches!(err, ReviewerSnapshotError::IdentityDrift(_)));
        assert_eq!(
            fs::read_to_string(fixture.worktree.join(".git")).unwrap(),
            "gitdir: /tmp/aria-bug45/t16/evil",
            "拒绝路径上不得改写身份锚点"
        );
    }
}
