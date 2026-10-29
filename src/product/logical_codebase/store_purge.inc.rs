// 彻底删除（硬删）逻辑代码库（owner 2026-10-29）。
//
// 软删（[`LogicalCodebaseStore::delete_soft`]）只写墓碑、释放 identity-registry
// 条目并清成员指针；政策正文与整个 LC 子树都保留——那是 `restore` 的前提。
// 硬删（[`LogicalCodebaseStore::purge`]）是不可恢复的终态清理：
//
// 1. **前置 = 已软删**（墓碑存在）：存活 LC 直接 `Conflict`
//    （`logical_codebase_not_tombstoned` → 409），避免误删可恢复对象；
// 2. 先清工作区产物（政策正文 + 成员指针）——清理依赖子树里的 manifest /
//    members / checkouts，必须在删子树之前做；
// 3. 删 app 数据侧 LC 子树的全部内容，**墓碑留到最后**：中途失败时墓碑还在，
//    同一入口重试即可继续；墓碑最后删掉后空目录一并收掉；
// 4. legacy 别名 LC 的权威数据仍在 `logical-codebase/` 旧根下，必须一并清掉，
//    否则下次 [`LogicalCodebaseStore::migrate_legacy`] 会把它复活；
// 5. 已不存在 → `NotFound`（与 `delete_soft` 相同的删除语义，重复 purge 稳定
//    返回 404）；
// 6. 路径全部按 project 分区（`validate_relative_id` + project 根），跨 project
//    的 purge 只会看到 NotFound，绝不触碰别人的 LC。
//
// 不清理：`AGENTS.md` / `CLAUDE.md` / `.claude/` 等 Cadence-skills 产物（见
// `aggregate_artifact_cleanup` 模块文档）；identity-registry 里已 Released 的
// 条目同样保留——Released 正是「可被重新登记」的合法状态。

use crate::product::logical_codebase::aggregate_artifact_cleanup::cleanup_logical_codebase_artifacts;

/// 已软删（墓碑）LC 的列表条目（`GET ...?include_deleted=true` 的呈现来源）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseDeletedRecord {
    pub id: String,
    pub name: String,
    pub aggregate_root: PathBuf,
    pub created_at: String,
    pub deleted_at: String,
}

impl LogicalCodebaseStore {
    /// 列出该 project 下已软删（墓碑）的 LC，`deleted_at` 取自 `tombstone.json`。
    /// 存活 LC 不在其中；没有墓碑的项目返回空列表。
    pub fn list_deleted(
        &self,
        project_id: &str,
    ) -> Result<Vec<LogicalCodebaseDeletedRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        self.migrate_legacy(project_id)?;
        let root = self.paths.logical_codebases_root(project_id);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };

        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            if !entry
                .file_type()
                .map_err(|error| {
                    ProductStoreError::Io(format!("stat {}: {error}", entry.path().display()))
                })?
                .is_dir()
            {
                continue;
            }
            let id = entry.file_name().into_string().map_err(|value| {
                ProductStoreError::InvalidRecord {
                    kind: "logical_codebase_record",
                    reason: format!(
                        "logical codebase directory name is not UTF-8: {}",
                        PathBuf::from(value).display()
                    ),
                }
            })?;
            let tombstone_path = self.tombstone_path(project_id, &id)?;
            if !path_exists(&tombstone_path)? || !path_exists(&self.record_path(project_id, &id)?)? {
                continue;
            }
            let record = self.load_record(project_id, &id)?;
            let tombstone: LogicalCodebaseTombstone = read_json(&tombstone_path)?;
            records.push(LogicalCodebaseDeletedRecord {
                id: record.id,
                name: record.name,
                aggregate_root: record.aggregate_root,
                created_at: record.created_at,
                deleted_at: tombstone.deleted_at,
            });
        }
        records.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(records)
    }

    /// 彻底删除一个**已软删**的 LC：app 数据侧子树 + 工作区政策正文 + 成员指针。
    ///
    /// 前置：墓碑必须存在（存活 LC → `Conflict { kind:
    /// "logical_codebase_not_tombstoned" }`）；LC 不存在 → `NotFound`（重复调用
    /// 稳定 404）。清理顺序见模块文档：工作区产物先清（依赖 manifest/成员），
    /// 墓碑最后删（保留失败重试锚点）。
    pub fn purge(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(logical_codebase_id)?;
        self.migrate_legacy(project_id)?;
        let tombstone_path = self.tombstone_path(project_id, logical_codebase_id)?;
        if !path_exists(&self.record_path(project_id, logical_codebase_id)?)?
            && !path_exists(&tombstone_path)?
        {
            return Err(ProductStoreError::NotFound {
                kind: "logical_codebase",
                id: logical_codebase_id.to_string(),
            });
        }
        if !path_exists(&tombstone_path)? {
            return Err(ProductStoreError::Conflict {
                kind: "logical_codebase_not_tombstoned",
                id: logical_codebase_id.to_string(),
            });
        }

        // 工作区产物清理依赖子树里的 manifest/members/checkouts：先清再删子树。
        // 清理 fail-soft（失败只 warn），绝不阻断硬删。
        cleanup_logical_codebase_artifacts(&self.paths, project_id, logical_codebase_id);

        self.remove_lc_subtree_keeping_tombstone(project_id, logical_codebase_id)?;

        // legacy 别名 LC 的权威数据在旧根下；不清会经 migrate_legacy 复活。
        if logical_codebase_id == legacy_logical_codebase_id(project_id) {
            remove_path(&self.paths.logical_codebase_root(project_id))?;
        }
        Ok(())
    }

    /// 删除 LC 子树里除墓碑之外的全部条目，最后删墓碑并收掉空目录。
    fn remove_lc_subtree_keeping_tombstone(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<(), ProductStoreError> {
        let root = self
            .paths
            .logical_codebase_record_root(project_id, logical_codebase_id);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };
        let tombstone_path = self.tombstone_path(project_id, logical_codebase_id)?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            // 墓碑留到最后：中途失败时它仍是「已软删」证据与重试锚点。
            if path == tombstone_path {
                continue;
            }
            remove_path(&path)?;
        }
        remove_path(&tombstone_path)?;
        // 子树已空：目录本身收掉（失败只留下空目录，不影响删除语义）。
        let _ = std::fs::remove_dir(&root);
        Ok(())
    }
}

/// 删除文件/目录（不跟随符号链接：`remove_dir_all` 只删链接本身）。
/// 缺失即视为已删除（幂等）。
fn remove_path(path: &Path) -> Result<(), ProductStoreError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "stat {}: {error}",
                path.display()
            )));
        }
    };
    let outcome = if metadata.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    outcome.map_err(|error| {
        ProductStoreError::Io(format!("remove {}: {error}", path.display()))
    })
}
