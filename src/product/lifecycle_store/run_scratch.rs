//! REQ-PRB-04/05/06/07/08：per-session scratch 分区与三段式生命周期。
//!
//! 位置：`<state>/run-scratch/<workspace_session_id>/`（`<state>` = `ProductAppPaths::root()`），
//! unix 权限 0700，位于仓库工作区与 provider cwd 之外。所有删除一律经
//! `remove_run_scratch_dir` 的单入口护栏（只删分区直接子目录，分区根永不删）。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::WorkspaceSessionStatus;

use super::LifecycleStore;
use super::utils::{read_workspace_session_record, remove_dir_all_if_exists};

/// owner 裁定 D6：孤儿 scratch TTL = 6 小时（REQ-PRB-07 要求具名常量、可由 owner 调整）。
pub const RUN_SCRATCH_ORPHAN_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// 删除结果。`Skipped` = 护栏判定目标不归本能力所有（非法 id、非真实目录、
/// 分区根被替换、路径漂移），此时路径保持原样。
#[derive(Debug, PartialEq, Eq)]
pub enum RunScratchRemoval {
    Removed,
    Absent,
    Skipped,
}

impl LifecycleStore {
    pub fn run_scratch_dir(&self, workspace_session_id: &str) -> PathBuf {
        self.app_paths().run_scratch_dir(workspace_session_id)
    }

    /// REQ-PRB-05：清空旧内容 → 重建 → 0700。经 session 存活准入桥，purging /
    /// deleted 的 session 不因准备而复活 scratch（准入错误原样返回，调用点按
    /// 契约只 warn）。
    pub fn prepare_run_scratch(&self, workspace_session_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(workspace_session_id)?;
        self.with_workspace_session_live_write(workspace_session_id, || {
            let root = self.app_paths().run_scratch_root();
            let target = self.run_scratch_dir(workspace_session_id);
            // 分区根被替换为符号链接/普通文件：不跟随、也不在其下创建。
            if !run_scratch_root_usable(&root)? {
                tracing::warn!(path = %root.display(), "run scratch prepare skip: partition root not a real directory");
                return Ok(());
            }
            if self.remove_run_scratch_dir(workspace_session_id)? == RunScratchRemoval::Skipped {
                return Ok(());
            }
            std::fs::create_dir_all(&target).map_err(|error| {
                ProductStoreError::Io(format!("create_dir_all {}: {error}", target.display()))
            })?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    ProductStoreError::Io(format!(
                        "set_permissions {}: {error}",
                        target.display()
                    ))
                },
            )?;
            Ok(())
        })
    }

    /// REQ-PRB-06：幂等清理能力（"是否已无活动 run" 的判定由 manager 侧作出）。
    pub fn cleanup_run_scratch(
        &self,
        workspace_session_id: &str,
    ) -> Result<RunScratchRemoval, ProductStoreError> {
        self.remove_run_scratch_dir(workspace_session_id)
    }

    /// 唯一删除入口：id 校验通过后交由 `remove_run_scratch_target` 做护栏判定。
    fn remove_run_scratch_dir(
        &self,
        workspace_session_id: &str,
    ) -> Result<RunScratchRemoval, ProductStoreError> {
        if validate_relative_id(workspace_session_id).is_err() {
            tracing::warn!(session_id = %workspace_session_id, "run scratch skip: illegal id");
            return Ok(RunScratchRemoval::Skipped);
        }
        let root = self.app_paths().run_scratch_root();
        let target = root.join(workspace_session_id);
        remove_run_scratch_target(&root, &target)
    }

    /// REQ-PRB-07：启动后一次孤儿回收。只处理分区直接子目录；三类条件
    /// （session 记录不存在 / 终态 / mtime 超 TTL）任一成立即删。单项失败
    /// 只 warn，不影响其余项，也不向调用方传播（启动 sweep 不阻塞启动）。
    pub fn sweep_orphan_run_scratch(
        &self,
        orphan_ttl: Duration,
    ) -> Result<usize, ProductStoreError> {
        let root = self.app_paths().run_scratch_root();
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read_dir {}: {error}",
                    root.display()
                )));
            }
        };
        let mut removed = 0usize;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(error = %error, "run scratch sweep: read_dir entry failed");
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            // 只处理 session 目录形状的直接子项（见 `is_run_scratch_session_dir_name`）。
            if !is_run_scratch_session_dir_name(&name) {
                tracing::warn!(name = %name, "run scratch sweep: non-session name skipped");
                continue;
            }
            // `DirEntry::file_type` 不跟随符号链接：非真实目录（含链接）跳过。
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                tracing::warn!(name = %name, "run scratch sweep: not a real directory skipped");
                continue;
            }
            if !self.run_scratch_is_orphan(&name, &entry.path(), orphan_ttl) {
                continue;
            }
            match self.remove_run_scratch_dir(&name) {
                Ok(RunScratchRemoval::Removed) => removed += 1,
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(name = %name, error = %error, "run scratch sweep: removal failed");
                }
            }
        }
        Ok(removed)
    }

    /// 三类孤儿判定。会话记录是否存在走 `find_workspace_session_path` +
    /// `read_workspace_session_record`——**不得**走 `get_workspace_session`
    /// （后者在 `cfg(test)` 下记读并可 panic，且会污染既有用例的读计数）。
    fn run_scratch_is_orphan(&self, session_id: &str, dir: &Path, orphan_ttl: Duration) -> bool {
        let session = match self.find_workspace_session_path(session_id) {
            Ok(path) => read_workspace_session_record(&path).ok().flatten(),
            Err(ProductStoreError::NotFound { .. }) => None,
            Err(error) => {
                tracing::warn!(session_id = %session_id, error = %error, "run scratch sweep: session lookup failed");
                return false;
            }
        };
        let Some(record) = session else {
            return true;
        };
        // 终态集合只认这三项：其余（Open/Running/WaitingForHuman/ChangeRequested/
        // BlockedProviderUnavailable/StoppedNeedsHuman）可能仍有活动 run。
        if matches!(
            record.status,
            WorkspaceSessionStatus::Confirmed
                | WorkspaceSessionStatus::Terminated
                | WorkspaceSessionStatus::Failed
        ) {
            return true;
        }
        let age = std::fs::metadata(dir)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| std::time::SystemTime::now().duration_since(modified).ok())
            .unwrap_or_default();
        age > orphan_ttl
    }
}

/// sweep 的目标形状判定：既要是安全相对 id，又要形如 session 目录名
/// （`workspace_session_*`，与 `workspace_session_file_stem` 同一口径）。
/// `validate_relative_id` 单用不够——`..hidden` 这类名字能通过 id 校验，
/// 但不是本能力创建的路径（REQ-PRB-08 MUST NOT 删除）。
fn is_run_scratch_session_dir_name(name: &str) -> bool {
    validate_relative_id(name).is_ok()
        && super::utils::workspace_session_file_stem(Path::new(name)).is_some()
}

/// 分区根可用性：不存在（待创建）或真实目录 → `true`；被替换为符号链接/
/// 普通文件 → `false`（不跟随，避免把目录建到别处或删到分区外）。
fn run_scratch_root_usable(root: &Path) -> Result<bool, ProductStoreError> {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) => Ok(metadata.file_type().is_dir()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(ProductStoreError::Io(format!(
            "symlink_metadata {}: {error}",
            root.display()
        ))),
    }
}

/// 删除护栏主体（设计 §5.4 / REQ-PRB-08）：只删分区根的真实子目录；
/// `NotFound` 视为成功；符号链接/非目录不删不跟随；路径漂移不删并告警。
fn remove_run_scratch_target(
    root: &Path,
    target: &Path,
) -> Result<RunScratchRemoval, ProductStoreError> {
    let metadata = match std::fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RunScratchRemoval::Absent);
        }
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "symlink_metadata {}: {error}",
                target.display()
            )));
        }
    };
    // `symlink_metadata` 不跟随链接：符号链接与非目录一律跳过。
    if !metadata.file_type().is_dir() {
        tracing::warn!(path = %target.display(), "run scratch skip: not a real directory");
        return Ok(RunScratchRemoval::Skipped);
    }
    if !run_scratch_root_usable(root)? {
        tracing::warn!(path = %root.display(), "run scratch skip: partition root not a real directory");
        return Ok(RunScratchRemoval::Skipped);
    }
    let canonical_root = match std::fs::canonicalize(root) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RunScratchRemoval::Absent);
        }
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "canonicalize {}: {error}",
                root.display()
            )));
        }
    };
    // 路径漂移核对：目标 canonical 的父目录必须仍是 canonical 分区根。
    let canonical_parent = match std::fs::canonicalize(target) {
        Ok(path) => path.parent().map(Path::to_path_buf),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RunScratchRemoval::Absent);
        }
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "canonicalize {}: {error}",
                target.display()
            )));
        }
    };
    if canonical_parent.as_deref() != Some(canonical_root.as_path()) {
        tracing::warn!(path = %target.display(), "run scratch skip: path drift");
        return Ok(RunScratchRemoval::Skipped);
    }
    // `fs::remove_dir_all` 不跟随符号链接：目录内的链接只删链接本身。
    remove_dir_all_if_exists(target)?;
    Ok(RunScratchRemoval::Removed)
}

#[cfg(test)]
mod run_scratch_tests {
    use super::{RUN_SCRATCH_ORPHAN_TTL, RunScratchRemoval};
    use crate::product::app_paths::{ProductAppPaths, RUN_SCRATCH_PARTITION};
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::models::WorkspaceSessionStatus;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    fn store() -> (tempfile::TempDir, LifecycleStore) {
        let tmp = tempfile::tempdir().expect("tmp");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        (tmp, LifecycleStore::new(paths))
    }

    #[test]
    fn run_scratch_dir_lives_under_state_root_not_repo() {
        let (_tmp, store) = store();
        let paths = store.app_paths();
        let scratch = paths.run_scratch_dir("workspace_session_auto_1");
        assert_eq!(RUN_SCRATCH_PARTITION, "run-scratch");
        assert_eq!(
            scratch,
            paths
                .root()
                .join("run-scratch")
                .join("workspace_session_auto_1")
        );
        assert!(scratch.starts_with(paths.root()));
        assert!(!scratch.starts_with(paths.projects_root()));
        assert_eq!(store.run_scratch_dir("workspace_session_auto_1"), scratch);
    }

    #[test]
    fn gitignore_covers_aria_state_root() {
        // Review Focus 之外的候选：scratch 不得进 git status（.gitignore:3 `.aria/`）。
        let gitignore = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/.gitignore"));
        assert!(
            gitignore.lines().any(|line| line.trim() == ".aria/"),
            "{gitignore}"
        );
    }

    #[test]
    fn remove_run_scratch_dir_removes_only_partition_child() {
        let (_tmp, store) = store();
        let dir = store.run_scratch_dir("workspace_session_auto_1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("draft.md"), "x").unwrap();
        let sibling = store.app_paths().run_scratch_root().join("keep.txt");
        std::fs::write(&sibling, "keep").unwrap();

        assert_eq!(
            store
                .cleanup_run_scratch("workspace_session_auto_1")
                .unwrap(),
            RunScratchRemoval::Removed
        );
        assert!(!dir.exists());
        assert!(sibling.exists(), "分区内 sibling 非本能力所有，不得删除");
        assert!(
            store.app_paths().run_scratch_root().exists(),
            "分区根永不删"
        );
    }

    #[test]
    fn remove_run_scratch_dir_refuses_illegal_and_symlinked_target() {
        let (tmp, store) = store();
        assert_eq!(
            store.cleanup_run_scratch("../escape").unwrap(),
            RunScratchRemoval::Skipped
        );

        let external = tmp.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("keep.md"), "keep").unwrap();
        let dir = store.run_scratch_dir("workspace_session_auto_2");
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&external, &dir).unwrap();
        assert_eq!(
            store
                .cleanup_run_scratch("workspace_session_auto_2")
                .unwrap(),
            RunScratchRemoval::Skipped
        );
        assert!(external.join("keep.md").exists(), "符号链接目标不得被删");
        std::fs::remove_file(&dir).unwrap();

        let file_target = store.run_scratch_dir("workspace_session_auto_3");
        std::fs::write(&file_target, "not a dir").unwrap();
        assert_eq!(
            store
                .cleanup_run_scratch("workspace_session_auto_3")
                .unwrap(),
            RunScratchRemoval::Skipped
        );
        assert!(file_target.exists(), "非目录目标不得删除");
    }

    #[test]
    fn cleanup_is_idempotent_and_reports_absent() {
        let (_tmp, store) = store();
        assert_eq!(
            store
                .cleanup_run_scratch("workspace_session_auto_4")
                .unwrap(),
            RunScratchRemoval::Absent
        );
    }

    #[test]
    fn prepare_run_scratch_clears_previous_content() {
        let (_tmp, store) = store();
        let dir = store.run_scratch_dir("workspace_session_auto_5");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.md"), "前序 run 残留").unwrap();

        store
            .prepare_run_scratch("workspace_session_auto_5")
            .unwrap();
        assert!(dir.is_dir());
        assert!(!dir.join("old.md").exists(), "新 run 不得看见前序内容");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn prepare_run_scratch_sets_0700_permissions() {
        let (_tmp, store) = store();
        store
            .prepare_run_scratch("workspace_session_auto_6")
            .unwrap();
        let mode = std::fs::metadata(store.run_scratch_dir("workspace_session_auto_6"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn prepare_run_scratch_refused_for_purging_session() {
        // Review Focus #2：purging / deleted 的 session 不得因 prepare 复活 scratch。
        use crate::product::entity_deletion_store::{
            DeletionEntityKind, EntityDeletionStore, EntityScope,
        };
        use crate::product::entity_operation_store::EntityOperationStore;
        use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
        use crate::product::models::{ProviderName, WorkspaceType};

        let (_tmp, store) = store();
        let record = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("create session");
        let scope = EntityScope::new(
            "project_0001",
            "issue_0001",
            DeletionEntityKind::Story,
            "story_spec_0001",
        );
        let lease = EntityOperationStore::new(store.app_paths())
            .try_acquire_delete(&scope)
            .expect("acquire delete lease");
        EntityDeletionStore::new(store.app_paths())
            .begin(&lease)
            .expect("begin purging tombstone");

        let result = store.prepare_run_scratch(&record.id);

        assert!(result.is_err(), "purging 会话的 prepare 必须被准入拒绝");
        assert!(
            !store.run_scratch_dir(&record.id).exists(),
            "不得复活 scratch 目录"
        );
        let _ = EntityDeletionStore::new(store.app_paths()).complete(&lease);
    }

    #[test]
    fn remove_run_scratch_dir_unlinks_inner_symlink_without_following() {
        // REQ-PRB-08：目录内部的符号链接只删链接本身，不进入其指向的外部目录。
        let (tmp, store) = store();
        let external = tmp.path().join("external-inner");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("keep.md"), "keep").unwrap();
        let dir = store.run_scratch_dir("workspace_session_auto_10");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&external, dir.join("escape")).unwrap();

        assert_eq!(
            store
                .cleanup_run_scratch("workspace_session_auto_10")
                .unwrap(),
            RunScratchRemoval::Removed
        );
        assert!(
            external.join("keep.md").exists(),
            "目录内符号链接不得被跟随"
        );
    }

    #[test]
    fn remove_run_scratch_target_skips_path_drift() {
        // 护栏：目标 canonical 的父目录不是 canonical 分区根 → 不删。
        let (tmp, store) = store();
        let root = store.app_paths().run_scratch_root();
        std::fs::create_dir_all(&root).unwrap();
        let target = tmp
            .path()
            .join("elsewhere")
            .join("workspace_session_auto_9");
        std::fs::create_dir_all(&target).unwrap();

        assert_eq!(
            super::remove_run_scratch_target(&root, &target).unwrap(),
            RunScratchRemoval::Skipped
        );
        assert!(target.is_dir(), "漂移目标不得删除");
    }

    #[test]
    fn run_scratch_partition_root_replaced_by_symlink_is_not_followed() {
        // 分区根被替换为符号链接：既不删分区外目录，也不在分区外建目录。
        let (tmp, store) = store();
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(outside.join("workspace_session_auto_7")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".aria")).unwrap();
        std::os::unix::fs::symlink(&outside, store.app_paths().run_scratch_root()).unwrap();

        assert_eq!(
            store
                .cleanup_run_scratch("workspace_session_auto_7")
                .unwrap(),
            RunScratchRemoval::Skipped
        );
        store
            .prepare_run_scratch("workspace_session_auto_8")
            .unwrap();
        assert!(
            outside.join("workspace_session_auto_7").exists(),
            "分区根不可信时不得删分区外目录"
        );
        assert!(
            !outside.join("workspace_session_auto_8").exists(),
            "分区根不可信时不得在分区外建目录"
        );
    }

    /// 建一个真实 session 记录（持久 store），返回其 session_id。
    fn create_session(store: &LifecycleStore, status: WorkspaceSessionStatus) -> String {
        use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
        use crate::product::models::{ProviderName, WorkspaceType};
        let record = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("create session");
        if record.status != status {
            store
                .update_workspace_session_status(&record.id, status)
                .expect("set session status");
        }
        record.id
    }

    #[test]
    fn sweep_orphan_run_scratch_removes_three_orphan_classes() {
        let (_tmp, store) = store();
        // ① session 记录不存在
        let ghost = store.run_scratch_dir("workspace_session_auto_ghost");
        std::fs::create_dir_all(&ghost).unwrap();
        // ② 终态 session
        let confirmed = create_session(&store, WorkspaceSessionStatus::Confirmed);
        let confirmed_dir = store.run_scratch_dir(&confirmed);
        std::fs::create_dir_all(&confirmed_dir).unwrap();
        // ③ 超 TTL 的活 session（用可注入 ttl = ZERO 表达年龄规则）
        let running = create_session(&store, WorkspaceSessionStatus::Running);
        let running_dir = store.run_scratch_dir(&running);
        std::fs::create_dir_all(&running_dir).unwrap();

        let removed = store.sweep_orphan_run_scratch(Duration::ZERO).unwrap();
        assert_eq!(removed, 3);
        assert!(!ghost.exists() && !confirmed_dir.exists() && !running_dir.exists());
    }

    #[test]
    fn sweep_orphan_run_scratch_keeps_live_run_scratch() {
        // Review Focus #4：活 run（session 存活、状态非终态、mtime 在 TTL 内）不得被回收。
        let (_tmp, store) = store();
        for status in [
            WorkspaceSessionStatus::Running,
            WorkspaceSessionStatus::WaitingForHuman,
        ] {
            let id = create_session(&store, status);
            std::fs::create_dir_all(store.run_scratch_dir(&id)).unwrap();
        }
        let removed = store
            .sweep_orphan_run_scratch(RUN_SCRATCH_ORPHAN_TTL)
            .unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn sweep_orphan_run_scratch_skips_illegal_names_and_keeps_processing() {
        let (_tmp, store) = store();
        std::fs::create_dir_all(store.app_paths().run_scratch_root()).unwrap();
        std::fs::create_dir_all(store.app_paths().run_scratch_root().join("..hidden")).unwrap();
        let ghost = store.run_scratch_dir("workspace_session_auto_ghost2");
        std::fs::create_dir_all(&ghost).unwrap();

        let removed = store
            .sweep_orphan_run_scratch(RUN_SCRATCH_ORPHAN_TTL)
            .unwrap();
        assert_eq!(removed, 1);
        assert!(
            store
                .app_paths()
                .run_scratch_root()
                .join("..hidden")
                .exists(),
            "非法名跳过不删"
        );
    }

    #[test]
    fn sweep_orphan_run_scratch_does_not_trigger_session_read_spy() {
        // Review Focus #2：sweep 必须走 find_workspace_session_path + read_workspace_session_record。
        let (_tmp, store) = store();
        let id = create_session(&store, WorkspaceSessionStatus::Running);
        std::fs::create_dir_all(store.run_scratch_dir(&id)).unwrap();
        crate::product::lifecycle_store::workspace_session_read_spy::reset_workspace_session_read_spy();
        crate::product::lifecycle_store::workspace_session_read_spy::set_workspace_session_read_panic(true);

        let removed = store
            .sweep_orphan_run_scratch(RUN_SCRATCH_ORPHAN_TTL)
            .unwrap();

        crate::product::lifecycle_store::workspace_session_read_spy::set_workspace_session_read_panic(false);
        assert_eq!(removed, 0);
        assert_eq!(
            crate::product::lifecycle_store::workspace_session_read_spy::workspace_session_read_count(),
            0,
            "sweep 不得调用 get_workspace_session"
        );
        assert!(store.run_scratch_dir(&id).exists());
    }

    #[test]
    fn run_scratch_orphan_ttl_exceeds_provider_idle_watchdog() {
        // 设计 §11.3：TTL 须大于 provider 零活动看门狗阈值（生产值 1800s，
        // `provider_drive/watchdog.rs`）。该常量在 `cfg(test)` 下被改成 150ms
        // 且 `provider_drive` 为私有模块，故此处按生产值断言，不用测试期常量
        // 做无效比较。
        assert!(
            RUN_SCRATCH_ORPHAN_TTL > Duration::from_secs(1800),
            "{RUN_SCRATCH_ORPHAN_TTL:?}"
        );
        assert_eq!(RUN_SCRATCH_ORPHAN_TTL, Duration::from_secs(6 * 60 * 60));
    }
}
