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
    ChannelGateRecord, GateState, REVIEWER_WINDOW_STORE_DIR, ReviewerWindowRecord,
    WindowLeaseRecord,
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
        // 存活窗口 id（`windows/` 现有记录，全部 attempt）——④ 闸门收割据此
        // 判定「该闸门是否还有窗口能解冻」。
        let mut live_window_ids: BTreeSet<String> = BTreeSet::new();

        // ① 活跃窗口记录 + 归档窗口记录（两者都携带 attempt_id）。
        for path in json_files_in(&self.window_root.join("windows"))? {
            let record: ReviewerWindowRecord = read_typed_json(&path)?;
            live_window_ids.insert(record.window_id.clone());
            if !wanted.contains(record.attempt_id.as_str()) {
                continue;
            }
            window_ids.insert(record.window_id.clone());
        }
        // 归档记录是**终态**窗口（接管时留证），不是存活窗口。
        for path in json_files_in(&self.window_root.join("window-archive"))? {
            let record: ReviewerWindowRecord = read_typed_json(&path)?;
            if !wanted.contains(record.attempt_id.as_str()) {
                continue;
            }
            window_ids.insert(record.window_id.clone());
        }

        // ② 窗口记录本体（活跃 + 归档）与违规 journal。
        for window_id in &window_ids {
            remove_file_if_exists(
                &self
                    .window_root
                    .join("windows")
                    .join(format!("{window_id}.json")),
            )?;
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
            remove_file_if_exists(
                &self
                    .window_root
                    .join("violations")
                    .join(format!("{window_id}.json")),
            )?;
        }

        // ②（续）：删除后本轮被清窗口即**不再存活**（`windows/` 已无其记录）
        // ——④ 的闸门收割按删除后的存活集合判定。
        for window_id in &window_ids {
            live_window_ids.remove(window_id);
        }

        // ③ 树级租约记录：按 attempt_id 或窗口归属精确删除（不整目录清空）。
        for path in json_files_in(&self.window_root.join("leases"))? {
            let lease: WindowLeaseRecord = read_typed_json(&path)?;
            if wanted.contains(lease.attempt_id.as_str()) || window_ids.contains(&lease.window_id) {
                remove_file_if_exists(&path)?;
            }
        }

        // ④ 写通道闸门记录：只删「持有者已无存活窗口记录」的那几条。
        //
        // 闸门按 common-dir 键目录存放，**同仓可有多窗口**（他 plan/issue 的
        // 存活窗口）。整目录清空会误删他窗口的 `Draining`/`Frozen` 闸门：
        // 记录被删后 `enter_write_channel` 会按缺失重建为 `Open`——平台写
        // 闸门被静默解除；他窗口的 `poll_channel_drain` 也会因闸门消失报
        // `OwnerMismatch`。故此处按 `GateState` 持有的 window_id 精确判定。
        //
        // **收割而非仅按 attempt 反查**（F1/R1）：上一轮可能停在「窗口记录
        // 已删、闸门未删」（IO/中断）；重入时该窗口已不在 `windows/`，按
        // attempt 反查的窗口集合采集不到它 → 其 common-dir 完全不被访问，
        // `Frozen` 闸门永久残留。而窗口 id 由 `role_run_window_id(attempt,
        // role_run)` 构造、attempt 不复用 → 该仓该通道永久 `Blocked`（再无
        // 窗口能解冻）。
        //
        // 故遍历**全部** `channels/*/`（不限于本次采集到的 common-dir），收割
        // 「持有者不在存活窗口集合」的 `Draining`/`Frozen` 闸门；`Open` 不被
        // 任何窗口持有，无条件保留（它不是任何窗口的闸门）。
        for channel_dir in directories_in(&self.window_root.join("channels"))? {
            for path in json_files_in(&channel_dir)? {
                let gate: ChannelGateRecord = read_typed_json(&path)?;
                let ownerless = match &gate.state {
                    GateState::Open => false,
                    GateState::Draining { window_id } | GateState::Frozen { window_id } => {
                        !live_window_ids.contains(window_id)
                    }
                };
                if ownerless {
                    remove_file_if_exists(&path)?;
                }
            }
            // 目录仅在空时回收（非空＝他窗口闸门仍在用）。
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
    use crate::product::reviewer_window_store::common_key_of;

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
            !archive
                .join("win_own.archived-g1-20261024T000000000Z.json")
                .exists(),
            "归档记录必须物理删除"
        );
        assert!(
            !paths
                .root()
                .join(REVIEWER_SNAPSHOT_STORE_DIR)
                .join("windows")
                .join("win_own")
                .exists()
        );
        assert!(!snapshot_archive.exists(), "归档快照必须物理删除");
    }

    /// F1 回归：清本窗口时**不得整删同仓的写通道闸门目录**——他窗口的
    /// `Draining`/`Frozen` 闸门必须保留（删了会被 `enter_write_channel` 重建
    /// 为 `Open`，平台写闸门被静默解除；该窗口的 `poll_channel_drain` 也会
    /// 因闸门消失报 `OwnerMismatch`）。
    #[test]
    fn purge_removes_own_channel_gate_and_keeps_other_window_gate() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria"));
        let common = temp.path().join("common");
        std::fs::create_dir_all(&common).unwrap();
        seed_window(paths.root(), "win_own", "attempt_own", &common);
        seed_window(paths.root(), "win_other", "attempt_other", &common);

        // 同一 common-dir 下两条闸门记录（按通道分文件）。
        let channel_dir = paths
            .root()
            .join(REVIEWER_WINDOW_STORE_DIR)
            .join("channels")
            .join(common_key_of(&common));
        std::fs::create_dir_all(&channel_dir).unwrap();
        let gate = |channel: &str, window_id: &str| {
            serde_json::json!({
                "schema": "reviewer_window_v1",
                "common_dir": common.display().to_string(),
                "channel": channel,
                "state": {"state": "frozen", "window_id": window_id},
                "in_flight": [],
                "generation": 1,
            })
        };
        std::fs::write(
            channel_dir.join("stage.json"),
            serde_json::to_vec(&gate("stage", "win_own")).unwrap(),
        )
        .unwrap();
        std::fs::write(
            channel_dir.join("commit.json"),
            serde_json::to_vec(&gate("commit", "win_other")).unwrap(),
        )
        .unwrap();

        ReviewerResourcePurger::new(&paths)
            .purge_for_attempts(&["attempt_own".to_string()])
            .expect("purge");

        assert!(
            !channel_dir.join("stage.json").exists(),
            "本窗口持有的闸门记录必须物理删除"
        );
        assert!(
            channel_dir.join("commit.json").exists(),
            "同仓他窗口的 Frozen 闸门必须保留（整删会解除平台写闸门）"
        );
        let preserved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(channel_dir.join("commit.json")).unwrap())
                .unwrap();
        assert_eq!(preserved["state"]["state"], serde_json::json!("frozen"));
        assert_eq!(
            preserved["state"]["window_id"],
            serde_json::json!("win_other")
        );
    }

    /// 某 common-dir 的通道闸门目录（按 `common_key_of` 键）。
    fn channel_dir(root: &Path, common_dir: &Path) -> PathBuf {
        root.join(REVIEWER_WINDOW_STORE_DIR)
            .join("channels")
            .join(common_key_of(common_dir))
    }

    /// 写一条闸门记录（`state_json` 是 `GateState` 的 serde 值）。
    fn seed_gate(channel_root: &Path, common_dir: &Path, channel: &str, state: &serde_json::Value) {
        std::fs::create_dir_all(channel_root).unwrap();
        let record = serde_json::json!({
            "schema": "reviewer_window_v1",
            "common_dir": common_dir.display().to_string(),
            "channel": channel,
            "state": state,
            "in_flight": [],
            "generation": 1,
        });
        std::fs::write(
            channel_root.join(format!("{channel}.json")),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    /// F1/R1 回归（闸门收割缺口）：上一轮在「删窗口记录之后、删闸门之前」
    /// 失败（IO/中断）→ 重入时窗口记录已不在，`window_ids` 采集不到该
    /// common-dir；而 `window_id = role_run_window_id(attempt, role_run)`
    /// 永不复用 → 该仓该通道的 `Frozen` 闸门无人能解冻，**永久 Blocked**。
    ///
    /// 修法（④ 阶段）：除本次待清窗口持有的闸门外，再收割「无任何存活
    /// 窗口记录引用」的闸门——遍历**全部** `channels/*/`（不局限于本次
    /// 采集到的 common-dir），`Open` 无条件保留。
    #[test]
    fn purge_harvests_gates_of_windows_without_surviving_record() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria"));
        let common_a = temp.path().join("common-a");
        let common_b = temp.path().join("common-b");
        std::fs::create_dir_all(&common_a).unwrap();
        std::fs::create_dir_all(&common_b).unwrap();
        // 本次待清窗口 + 同仓存活窗口。
        seed_window(paths.root(), "win_own", "attempt_own", &common_a);
        seed_window(paths.root(), "win_alive", "attempt_other", &common_a);

        let dir_a = channel_dir(paths.root(), &common_a);
        // 孤儿闸门：持有窗口的记录已不存在（重入现场）。
        seed_gate(
            &dir_a,
            &common_a,
            "stage",
            &serde_json::json!({"state": "frozen", "window_id": "win_gone"}),
        );
        // 存活他窗的冻结闸门 → 必须保留。
        seed_gate(
            &dir_a,
            &common_a,
            "commit",
            &serde_json::json!({"state": "frozen", "window_id": "win_alive"}),
        );
        // 放行闸门 → 无条件保留。
        seed_gate(
            &dir_a,
            &common_a,
            "push",
            &serde_json::json!({"state": "open"}),
        );
        // 另一 common-dir（不属本次待清窗口采集集）的孤儿冻结闸门 →
        // 收割必须覆盖全部通道目录，否则该仓同样永久 Blocked。
        let dir_b = channel_dir(paths.root(), &common_b);
        seed_gate(
            &dir_b,
            &common_b,
            "build",
            &serde_json::json!({"state": "draining", "window_id": "win_gone_b"}),
        );

        ReviewerResourcePurger::new(&paths)
            .purge_for_attempts(&["attempt_own".to_string()])
            .expect("purge");

        assert!(
            !dir_a.join("stage.json").exists(),
            "持有窗口记录已不存在的 Frozen 闸门必须被收割（否则该通道永久 Blocked）"
        );
        assert!(
            !dir_b.join("build.json").exists(),
            "收割必须覆盖本次待清窗口未触及的 common-dir（重入现场）"
        );
        assert!(
            dir_a.join("commit.json").exists(),
            "存活窗口持有的 Frozen 闸门必须保留"
        );
        let preserved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir_a.join("commit.json")).unwrap()).unwrap();
        assert_eq!(
            preserved["state"]["window_id"],
            serde_json::json!("win_alive")
        );
        assert!(
            dir_a.join("push.json").exists(),
            "Open 闸门不被任何窗口持有，必须无条件保留"
        );
        let open: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir_a.join("push.json")).unwrap()).unwrap();
        assert_eq!(open["state"]["state"], serde_json::json!("open"));
    }
}
