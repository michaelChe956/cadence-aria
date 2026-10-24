//! reviewer 窗口/租约/通道/快照基线的**物理删除**（REQ-WPS-03、REQ-WPS-06）。
//!
//! 既有 reviewer store 只提供「归档」（`archive_window_record` 把记录 rename
//! 到 `window-archive/`；`archive_window_baseline` 把快照目录 rename 到
//! `windows-archive/`）。归档记录仍带 `attempt_id` 身份，仍是旧 plan 的从属
//! 产物——REQ-WPS-03「归档不等于删除」要求把它们与活跃记录同等物理清除。
//!
//! 归属判定只用真实 owner-key：窗口记录与租约记录携带 `attempt_id`（快照
//! 基线 manifest 携带 `window_id`/`attempt_id`）。本模块按 attempt 集合反查
//! 窗口 → 再按窗口身份清租约/通道/快照，绝不按目录整删（不误伤他 plan）。
//!
//! 读取错误 fail-closed：窗口/租约 JSON 损坏即报错，不当空跳过（禁静默
//! 留下残留）。对象本已缺失视为已清。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::reviewer_window_store::{
    REVIEWER_WINDOW_STORE_DIR, ReviewerWindowRecord, WindowLeaseRecord, common_key_of,
};

/// reviewer 快照 store 根目录名（生产根 `<aria>/reviewer-snapshots`）。
///
/// 与 `coding_workspace_engine::code_review::REVIEWER_SNAPSHOT_STORE_DIR`、
/// `workspace_engine::review::drive::PLAN_REVIEWER_SNAPSHOT_STORE_DIR` 同值
/// （既有约定：窗口目录名已上收为单一常量，快照目录仍是字面量锚定约定）。
pub(crate) const REVIEWER_SNAPSHOT_STORE_DIR: &str = "reviewer-snapshots";

/// reviewer 资源物理删除器（按 attempt 归属）。
pub(crate) struct ReviewerResourcePurger {
    window_root: PathBuf,
    snapshot_root: PathBuf,
}

impl ReviewerResourcePurger {
    pub(crate) fn new(app_paths: &ProductAppPaths) -> Self {
        Self {
            window_root: app_paths.root().join(REVIEWER_WINDOW_STORE_DIR),
            snapshot_root: app_paths.root().join(REVIEWER_SNAPSHOT_STORE_DIR),
        }
    }

    /// 清除给定 attempt 集合拥有的全部 reviewer 资源。
    ///
    /// 覆盖：活跃窗口记录、**归档窗口记录**、树级租约记录、写通道记录、
    /// 违规 journal、快照基线目录及其**归档目录**。缺失=已清（幂等）。
    pub(crate) fn purge_for_attempts(
        &self,
        attempt_ids: &[String],
    ) -> Result<(), ProductStoreError> {
        if attempt_ids.is_empty() {
            return Ok(());
        }
        let wanted: BTreeSet<&str> = attempt_ids.iter().map(String::as_str).collect();
        let mut window_ids: BTreeSet<String> = BTreeSet::new();
        let mut common_dirs: BTreeSet<PathBuf> = BTreeSet::new();

        // ① 活跃窗口记录 + 归档窗口记录（两者都携带 attempt_id）。
        for directory in [self.window_root.join("windows"), self.window_root.join("window-archive")]
        {
            for path in json_files_in(&directory)? {
                let record: ReviewerWindowRecord = read_typed_json(&path)?;
                if !wanted.contains(record.attempt_id.as_str()) {
                    continue;
                }
                window_ids.insert(record.window_id.clone());
                common_dirs.insert(record.identity.common_dir.clone());
            }
        }

        // ② 窗口记录本体（活跃 + 归档）与违规 journal。
        for window_id in &window_ids {
            remove_file_if_exists(&self.window_root.join("windows").join(format!("{window_id}.json")))?;
            for path in json_files_in(&self.window_root.join("window-archive"))? {
                let name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name.starts_with(&format!("{window_id}.")) {
                    remove_file_if_exists(&path)?;
                }
            }
            remove_file_if_exists(&self.window_root.join("violations").join(format!("{window_id}.json")))?;
        }

        // ③ 树级租约记录：按 attempt_id 或窗口归属精确删除（不整目录清空）。
        for path in json_files_in(&self.window_root.join("leases"))? {
            let lease: WindowLeaseRecord = read_typed_json(&path)?;
            if wanted.contains(lease.attempt_id.as_str()) || window_ids.contains(&lease.window_id) {
                remove_file_if_exists(&path)?;
            }
        }

        // ④ 写通道记录：按 common-dir 键目录精确删除（目录空则回收）。
        for common_dir in &common_dirs {
            let channel_dir = self
                .window_root
                .join("channels")
                .join(common_key_of(common_dir));
            for path in json_files_in(&channel_dir)? {
                remove_file_if_exists(&path)?;
            }
            let _ = std::fs::remove_dir(&channel_dir);
        }

        // ⑤ 快照基线目录 + 归档目录（归档 MUST NOT 充当删除）。
        for window_id in &window_ids {
            let window_dir = self.snapshot_root.join("windows").join(window_id);
            remove_dir_all_if_exists(&window_dir)?;
            let archive_root = self.snapshot_root.join("windows-archive");
            for path in directories_in(&archive_root)? {
                let name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name.starts_with(&format!("{window_id}.")) {
                    remove_dir_all_if_exists(&path)?;
                }
            }
        }
        Ok(())
    }
}

/// 目录下的 `*.json` 文件（按名排序；缺目录=空集）。
fn json_files_in(directory: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                directory.display()
            )));
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", directory.display()))
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}

/// 目录下的子目录（按名排序；缺目录=空集）。
fn directories_in(directory: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                directory.display()
            )));
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", directory.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// 类型化读取（坏 JSON fail-closed，不当空跳过）。
fn read_typed_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ProductStoreError> {
    let file = std::fs::File::open(path)
        .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
    serde_json::from_reader(file)
        .map_err(|error| ProductStoreError::Json(format!("{}: {error}", path.display())))
}

/// 幂等删除文件；NotFound=已清。
fn remove_file_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

/// 幂等删除目录树；NotFound=已清。
fn remove_dir_all_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_window(root: &Path, window_id: &str, attempt_id: &str, common_dir: &Path) {
        let windows = root.join("reviewer-windows").join("windows");
        std::fs::create_dir_all(&windows).unwrap();
        let record = serde_json::json!({
            "schema": "reviewer_window_v1",
            "window_id": window_id,
            "generation": 1,
            "identity": {
                "canonical_worktree": common_dir.join("wt"),
                "gitdir": common_dir.join("gitdir"),
                "common_dir": common_dir,
                "lease_id": "lease_1",
            },
            "attempt_id": attempt_id,
            "run_id": "run_1",
            "provider": "p",
            "state": "running",
            "report_acceptance": "pending_acceptance",
            "platform_write_paths": [],
            "quiesced_scopes": [],
            "baseline": null,
            "restore": null,
            "recovery": null,
            "last_error": null,
            "violation_pending": null,
            "pre_restore_git_facts": [],
            "transitions": [],
            "created_at": "2026-10-24T00:00:00Z",
            "updated_at": "2026-10-24T00:00:00Z",
        });
        std::fs::write(
            windows.join(format!("{window_id}.json")),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn purge_removes_active_and_archived_windows_with_snapshot_baselines() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria"));
        let common = temp.path().join("common");
        std::fs::create_dir_all(&common).unwrap();
        seed_window(paths.root(), "win_own", "attempt_own", &common);
        seed_window(paths.root(), "win_other", "attempt_other", &common);
        // 归档副本（归档不等于删除）。
        let archive = paths.root().join("reviewer-windows").join("window-archive");
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::copy(
            paths
                .root()
                .join("reviewer-windows")
                .join("windows")
                .join("win_own.json"),
            archive.join("win_own.archived-g1-20261024T000000000Z.json"),
        )
        .unwrap();
        // 快照基线（活跃 + 归档）。
        let snapshot = paths
            .root()
            .join(REVIEWER_SNAPSHOT_STORE_DIR)
            .join("windows")
            .join("win_own");
        std::fs::create_dir_all(&snapshot).unwrap();
        std::fs::write(snapshot.join("manifest.json"), b"{}").unwrap();
        let snapshot_archive = paths
            .root()
            .join(REVIEWER_SNAPSHOT_STORE_DIR)
            .join("windows-archive")
            .join("win_own.20261024T000000000Z");
        std::fs::create_dir_all(&snapshot_archive).unwrap();

        let purger = ReviewerResourcePurger::new(&paths);
        purger
            .purge_for_attempts(&["attempt_own".to_string()])
            .expect("purge");

        let window_root = paths.root().join(REVIEWER_WINDOW_STORE_DIR);
        assert!(!window_root.join("windows").join("win_own.json").exists());
        assert!(window_root.join("windows").join("win_other.json").exists());
        assert!(
            !archive.join("win_own.archived-g1-20261024T000000000Z.json").exists(),
            "归档记录必须物理删除"
        );
        assert!(!paths.root().join(REVIEWER_SNAPSHOT_STORE_DIR).join("windows").join("win_own").exists());
        assert!(!snapshot_archive.exists(), "归档快照必须物理删除");
    }
}
