//! Reviewer 恢复游标 [`RestoreCursor`](crate::product::reviewer_snapshot::RestoreCursor)
//! 的**增量持久化**(独立 sidecar 小文件;从 `reviewer_snapshot.rs` 拆出,
//! 该文件在 `tests/it_core/large_file_guard.rs` 中已锁定行数上限)。
//!
//! 现场事故(2026-10-24):窗口基线 12,397 条目 / 4.3 MB manifest,恢复
//! 每推进一条就把整份 manifest 重写一遍 → O(n²) 写放大(累计 ≈13 GB 写、
//! 单窗恢复耗时小时级,条目推进速率被 fsync 压在 1.3 条/秒)。本模块把
//! **条目级进度**落到同目录的 `restore-cursor.json`(几十字节,O(1) 字节/
//! 次),整份 manifest 只在**阶段边界**重写一次(次数 = 阶段数,常数)。
//!
//! 崩溃可恢复性(不降低):先写 sidecar、再写 manifest,
//! `ReviewerSnapshotStore::load_manifest` 取二者中**较前**的游标续接——
//! 任一处崩溃后游标只可能偏后、绝不倒退;sidecar 正常时回退粒度是一条
//! 条目,sidecar 丢失时最坏回退到本阶段起点,已完成阶段进度不丢。
//!
//! 恢复语义零变化:本模块只换持久化载体,不参与差异判定——恢复仍是
//! 「与基线逐条比对、只恢复差异、幂等、可重入」。
//!
//! 持续写入计数([`CursorWriteStats`])供写放大验收:整份 manifest 重写
//! 次数必须是阶段数常数,与基线条目数无关。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use crate::product::json_store::{read_json, write_json};
use crate::product::reviewer_snapshot::{
    RestoreCursor, ReviewerBaselineManifest, ReviewerSnapshotError,
};

/// 游标 sidecar 文件名(与 `manifest.json` 同目录、同一窗口目录受信面)。
pub(crate) const CURSOR_SIDECAR_FILE: &str = "restore-cursor.json";

/// 游标 sidecar 的 schema(独立版本;与 manifest schema 解耦)。
pub(crate) const CURSOR_SIDECAR_SCHEMA: &str = "reviewer_restore_cursor/1";

/// 增量游标载体。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RestoreCursorSidecar {
    pub(crate) schema: String,
    /// 窗号绑定:归档/复用目录里的陈旧 sidecar 不得被套用到新窗口。
    pub(crate) window_id: String,
    pub(crate) cursor: RestoreCursor,
}

/// 游标 sidecar 路径。
pub(crate) fn sidecar_path(window_dir: &Path) -> PathBuf {
    window_dir.join(CURSOR_SIDECAR_FILE)
}

/// 游标单调序(阶段优先,同阶段比条目下标)。
fn is_ahead(cursor: &RestoreCursor, other: &RestoreCursor) -> bool {
    cursor.phase.rank() > other.phase.rank()
        || (cursor.phase == other.phase && cursor.entry_index > other.entry_index)
}

/// 读 sidecar 游标:不存在 → `None`;`schema`/窗号不符或不可读 →
/// fail-closed(原子写保证崩溃不会产出半篇 sidecar,损坏即篡改)。
pub(crate) fn load(
    window_dir: &Path,
    window_id: &str,
    manifest: &ReviewerBaselineManifest,
) -> Result<Option<RestoreCursor>, ReviewerSnapshotError> {
    let path = sidecar_path(window_dir);
    if !path.is_file() {
        return Ok(None);
    }
    let sidecar: RestoreCursorSidecar = read_json(&path).map_err(|error| {
        ReviewerSnapshotError::Integrity(format!("游标 sidecar 不可读 {}: {error}", path.display()))
    })?;
    if sidecar.schema != CURSOR_SIDECAR_SCHEMA {
        return Err(ReviewerSnapshotError::Integrity(format!(
            "游标 sidecar schema 不符: {:?}",
            sidecar.schema
        )));
    }
    if sidecar.window_id != window_id || manifest.window_id != window_id {
        return Err(ReviewerSnapshotError::Integrity(format!(
            "游标 sidecar 窗号不符: sidecar={} dir={} manifest={}",
            sidecar.window_id, window_id, manifest.window_id
        )));
    }
    Ok(Some(sidecar.cursor))
}

/// 把 manifest 的内存游标叠加到 sidecar 的进度上(取较前者)。
pub(crate) fn apply(
    window_dir: &Path,
    window_id: &str,
    manifest: &mut ReviewerBaselineManifest,
) -> Result<(), ReviewerSnapshotError> {
    if let Some(cursor) = load(window_dir, window_id, manifest)?
        && is_ahead(&cursor, &manifest.restore_cursor)
    {
        manifest.restore_cursor = cursor;
    }
    Ok(())
}

/// 游标推进:条目级增量 sidecar 原子写 + 阶段边界整份 manifest 落盘。
///
/// 返回是否真的推进(同游标重复推进不写盘)。游标回退即 Integrity 拒绝。
pub(crate) fn advance(
    window_dir: &Path,
    window_id: &str,
    manifest_path: &Path,
    manifest: &mut ReviewerBaselineManifest,
    target: RestoreCursor,
    stats: &CursorWriteStats,
) -> Result<bool, ReviewerSnapshotError> {
    let current = manifest.restore_cursor.clone();
    if current == target {
        return Ok(false);
    }
    if target.phase.rank() < current.phase.rank()
        || (target.phase == current.phase && target.entry_index < current.entry_index)
    {
        return Err(ReviewerSnapshotError::Integrity(format!(
            "游标回退: {current:?} -> {target:?}"
        )));
    }
    let phase_changed = target.phase != current.phase;
    manifest.restore_cursor = target.clone();
    write_json(
        &sidecar_path(window_dir),
        &RestoreCursorSidecar {
            schema: CURSOR_SIDECAR_SCHEMA.to_string(),
            window_id: window_id.to_string(),
            cursor: target,
        },
    )?;
    stats.note_sidecar_write();
    if phase_changed {
        write_json(manifest_path, manifest)?;
        stats.note_manifest_rewrite();
    }
    Ok(true)
}

/// 游标持久化写入计数与故障注入(写放大验收用;relaxed 原子,单条指令)。
#[derive(Debug)]
pub(crate) struct CursorWriteStats {
    manifest_rewrites: AtomicUsize,
    sidecar_writes: AtomicUsize,
    /// 仅测试:第 N 次游标持久化后注入失败(usize::MAX = 禁用)。
    #[cfg(test)]
    failpoint_after_writes: AtomicUsize,
}

impl Default for CursorWriteStats {
    fn default() -> Self {
        Self {
            manifest_rewrites: AtomicUsize::new(0),
            sidecar_writes: AtomicUsize::new(0),
            #[cfg(test)]
            failpoint_after_writes: AtomicUsize::new(usize::MAX),
        }
    }
}

impl CursorWriteStats {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn note_manifest_rewrite(&self) {
        self.manifest_rewrites.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_sidecar_write(&self) {
        self.sidecar_writes.fetch_add(1, Ordering::Relaxed);
    }

    /// 仅测试:(整份 manifest 重写次数, sidecar 写次数)。
    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> (usize, usize) {
        (
            self.manifest_rewrites.load(Ordering::Relaxed),
            self.sidecar_writes.load(Ordering::Relaxed),
        )
    }

    /// 仅测试:下一次游标持久化后(倒数第 `writes` 次)注入故障。
    #[cfg(test)]
    pub(crate) fn arm_failpoint(&self, writes: usize) {
        self.failpoint_after_writes.store(writes, Ordering::Relaxed);
    }

    /// 仅测试:登记一次游标持久化;返回是否应注入故障。
    #[cfg(test)]
    pub(crate) fn trip_failpoint(&self) -> bool {
        self.failpoint_after_writes
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                if value == usize::MAX {
                    None
                } else {
                    Some(value.saturating_sub(1))
                }
            })
            == Ok(0)
    }
}
