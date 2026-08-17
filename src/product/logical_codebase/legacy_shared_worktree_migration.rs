//! REQ-COD-03 历史 issue 级 shared worktree 的显式迁移协议。
//!
//! C-1 对发现的旧文件 fail-closed；本模块只供受控的人工迁移工具调用。迁移以
//! identity migration journal 的 physical→logical mapping 为唯一身份来源，绝不从
//! 路径或当前 WorkItem 猜测目标仓。协议顺序固定为 legacy lock → repository-id
//! 字典序的新锁；写入按「journal → new record → redirect」可重放推进。

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_attempt_store::locking::ExclusiveFileLock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::IssueSharedWorktree;

use super::{IdentityMigrationJournalStore, LogicalRepositoryId, RepositoryCheckoutId};

const LEGACY_SHARED_WORKTREE_MIGRATION_JOURNAL_FILE: &str = "legacy-shared-worktree-migration.json";
const LEGACY_SHARED_WORKTREE_REDIRECT_KIND: &str = "legacy_shared_worktree_redirect";
const LEGACY_SHARED_WORKTREE_MIGRATION_SCHEMA_VERSION: u16 = 1;

/// 可迁移的旧 `issue-shared-worktree.json` record。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySharedWorktreeRecord {
    pub worktree: IssueSharedWorktree,
}

/// 写回旧路径的持久 redirect/tombstone。它使新逻辑路径忽略旧路径，同时让人工恢复
/// 能验证对应的仓维 record 与 journal 尚完整。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LegacySharedWorktreeRedirect {
    pub kind: String,
    pub schema_version: u16,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: LogicalRepositoryId,
    pub migration_journal: String,
    pub migrated_at: String,
}

/// 迁移的 durable state：先持久 mapping/目标 record，再写每一个后续阶段，保证进程
/// 在任一写入后崩溃都能由相同输入幂等恢复。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LegacySharedWorktreeMigrationJournal {
    pub schema_version: u16,
    pub project_id: String,
    pub issue_id: String,
    pub legacy_repository_id: String,
    pub repository_id: LogicalRepositoryId,
    pub checkout_id: RepositoryCheckoutId,
    pub migrated_record: IssueSharedWorktree,
    #[serde(default)]
    pub new_record_persisted: bool,
    #[serde(default)]
    pub redirect_persisted: bool,
    #[serde(default)]
    pub legacy_cleanup_completed: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 单次迁移或恢复后可审计的状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySharedWorktreeMigrationResult {
    pub repository_id: LogicalRepositoryId,
    pub migrated_record: IssueSharedWorktree,
    pub redirect_persisted: bool,
    pub legacy_cleanup_completed: bool,
}

/// 仅暴露显式人工迁移所需的最小 API；不挂接到 admission 或正常 coding 工作流。
#[derive(Debug, Clone, Copy)]
pub struct LegacySharedWorktreeMigration;

impl LegacySharedWorktreeMigration {
    /// 读取旧 record。缺失与已写 redirect 均表示没有会阻断新路径的 legacy record；
    /// 坏 JSON、record/redirect 字段缺失或 identity 不成立则严格失败。
    pub fn load_legacy_shared_worktree(
        paths: &ProductAppPaths,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<LegacySharedWorktreeRecord>, ProductStoreError> {
        validate_scope(project_id, issue_id)?;
        let path = legacy_path(paths, project_id, issue_id);
        match read_json_value_if_exists(&path)? {
            None => Ok(None),
            Some(value) if is_redirect_value(&value) => {
                let redirect = parse_redirect(value)?;
                validate_redirect(&redirect, project_id, issue_id)?;
                Ok(None)
            }
            Some(value) => {
                let worktree = serde_json::from_value::<IssueSharedWorktree>(value)
                    .map_err(|error| inconsistent(format!("legacy record decode: {error}")))?;
                validate_legacy_record(&worktree, project_id, issue_id)?;
                Ok(Some(LegacySharedWorktreeRecord { worktree }))
            }
        }
    }

    /// 读取旧路径上的 redirect/tombstone，供人工迁移演练与恢复工具检查旧 API 的
    /// 可见语义。不存在或尚是普通 legacy record 时返回 `None`。
    pub fn load_legacy_shared_worktree_redirect(
        paths: &ProductAppPaths,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<LegacySharedWorktreeRedirect>, ProductStoreError> {
        validate_scope(project_id, issue_id)?;
        let path = legacy_path(paths, project_id, issue_id);
        let Some(value) = read_json_value_if_exists(&path)? else {
            return Ok(None);
        };
        if !is_redirect_value(&value) {
            return Ok(None);
        }
        let redirect = parse_redirect(value)?;
        validate_redirect(&redirect, project_id, issue_id)?;
        Ok(Some(redirect))
    }

    /// 按 REQ-COD-03 的锁及写入顺序执行一次可重放迁移：
    ///
    /// 1. 取得 `.issue-shared-worktree.json.lock`；
    /// 2. 从 identity migration journal 唯一解析 legacy physical repository；
    /// 3. 按 logical repository ID 字典序取得新路径锁（当前旧 record 仅能映射一个）；
    /// 4. 先持久 migration journal，原子写新 record，最后原子写 legacy redirect。
    ///
    /// 活动 attempt 或 legacy record 的有效 lock owner 都会阻断，不会写入半迁移状态。
    /// redirect 写成后由 [`Self::finalize_if_no_active_references`] 在双前提成立时清旧文件。
    pub fn migrate_to_repository_keyed(
        paths: &ProductAppPaths,
        record: LegacySharedWorktreeRecord,
    ) -> Result<LegacySharedWorktreeMigrationResult, ProductStoreError> {
        let worktree = &record.worktree;
        validate_legacy_record(worktree, &worktree.project_id, &worktree.issue_id)?;
        let project_id = worktree.project_id.clone();
        let issue_id = worktree.issue_id.clone();
        let legacy_path = legacy_path(paths, &project_id, &issue_id);
        let _legacy_lock = ExclusiveFileLock::acquire(&legacy_path)?;

        let current = Self::load_legacy_shared_worktree(paths, &project_id, &issue_id)?;
        match current {
            Some(current) if current != record => {
                return Err(inconsistent(
                    "legacy record changed while migration lock was held",
                ));
            }
            None => {
                return Self::resume_redirected_migration(paths, &project_id, &issue_id);
            }
            Some(_) => {}
        }

        if has_active_references(paths, &project_id, &issue_id, worktree)? {
            return Err(ProductStoreError::Conflict {
                kind: "legacy_shared_worktree_active",
                id: format!("{project_id}/{issue_id}"),
            });
        }

        let mapping = unique_identity_mapping(paths, &project_id, &worktree.repository_id)?;
        validate_legacy_mapping_identity(worktree, mapping)?;
        let migrated_record = migrated_record(worktree, mapping.repository_id, mapping.checkout_id);
        let mut journal = load_or_create_journal(
            paths,
            &project_id,
            &issue_id,
            &worktree.repository_id,
            mapping.repository_id,
            mapping.checkout_id,
            &migrated_record,
        )?;

        // `Vec` 使未来 journal 扩展为多个 record 时继续保持固定全局顺序；当前一条
        // legacy record 的列表长度为一，但仍明确遵循 F-6 的排序规则。
        let mut repository_ids = vec![mapping.repository_id];
        repository_ids.sort_unstable();
        let _repository_locks =
            acquire_repository_locks(paths, &project_id, &issue_id, &repository_ids)?;

        let new_path = repository_path(paths, &project_id, &issue_id, mapping.repository_id);
        ensure_migrated_record(&new_path, &migrated_record)?;
        if !journal.new_record_persisted {
            journal.new_record_persisted = true;
            touch(&mut journal);
            save_journal(paths, &journal)?;
        }

        let redirect = redirect_for(&journal);
        ensure_redirect(&legacy_path, &redirect)?;
        if !journal.redirect_persisted {
            journal.redirect_persisted = true;
            touch(&mut journal);
            save_journal(paths, &journal)?;
        }

        Ok(result_from_journal(&journal))
    }

    /// 崩溃恢复与旧 JSON/锁清理：只接受已持久化 redirect，并且旧 record 已没有活动
    /// attempt/lock owner 时才删除旧路径。其余情况返回 `false`，不会越过双条件。
    pub fn finalize_if_no_active_references(
        paths: &ProductAppPaths,
        project_id: &str,
        issue_id: &str,
    ) -> Result<bool, ProductStoreError> {
        validate_scope(project_id, issue_id)?;
        let legacy_path = legacy_path(paths, project_id, issue_id);
        // Do not recreate a retired legacy lock merely to answer an idempotent recovery call.
        if !legacy_path.exists() {
            return Ok(load_journal(paths, project_id, issue_id)?
                .is_some_and(|journal| journal.legacy_cleanup_completed));
        }
        let legacy_lock = ExclusiveFileLock::acquire(&legacy_path)?;
        let Some(redirect) =
            Self::load_legacy_shared_worktree_redirect(paths, project_id, issue_id)?
        else {
            // A concurrent finalizer may have removed the redirect after the initial existence
            // check; its journal state is authoritative for this idempotent retry.
            return Ok(load_journal(paths, project_id, issue_id)?
                .is_some_and(|journal| journal.legacy_cleanup_completed));
        };
        let mut journal = load_journal(paths, project_id, issue_id)?
            .ok_or_else(|| inconsistent("legacy redirect exists without migration journal"))?;
        validate_journal(&journal, project_id, issue_id)?;
        validate_redirect_journal(&redirect, &journal)?;

        let mut repository_ids = vec![journal.repository_id];
        repository_ids.sort_unstable();
        let repository_locks =
            acquire_repository_locks(paths, project_id, issue_id, &repository_ids)?;
        ensure_migrated_record_identity(
            &repository_path(paths, project_id, issue_id, journal.repository_id),
            &journal.migrated_record,
        )?;

        if has_active_references(paths, project_id, issue_id, &journal.migrated_record)? {
            return Ok(false);
        }
        if !journal.redirect_persisted {
            return Err(inconsistent("migration journal has no persisted redirect"));
        }

        // Persist completion before removing the old paths. If interrupted afterwards, recovery
        // simply sees the redirect again and repeats idempotent cleanup.
        if !journal.legacy_cleanup_completed {
            journal.legacy_cleanup_completed = true;
            touch(&mut journal);
            save_journal(paths, &journal)?;
        }

        let retired_lock = retired_legacy_lock_path(&legacy_path);
        fs::remove_file(&legacy_path).map_err(|error| {
            ProductStoreError::Io(format!("remove {}: {error}", legacy_path.display()))
        })?;
        // Do not unlink a lock file while its guard is still live: rename keeps the locked inode
        // visible to waiters that opened it before this point, then it is removed after Drop.
        let legacy_lock_path = lock_path_for(&legacy_path);
        fs::rename(&legacy_lock_path, &retired_lock).map_err(|error| {
            ProductStoreError::Io(format!(
                "retire legacy lock {}: {error}",
                legacy_lock_path.display()
            ))
        })?;
        drop(repository_locks);
        drop(legacy_lock);
        remove_file_if_exists(&retired_lock)?;
        Ok(true)
    }

    fn resume_redirected_migration(
        paths: &ProductAppPaths,
        project_id: &str,
        issue_id: &str,
    ) -> Result<LegacySharedWorktreeMigrationResult, ProductStoreError> {
        let redirect = Self::load_legacy_shared_worktree_redirect(paths, project_id, issue_id)?
            .ok_or_else(|| inconsistent("legacy record is absent without redirect"))?;
        let journal = load_journal(paths, project_id, issue_id)?
            .ok_or_else(|| inconsistent("legacy redirect exists without migration journal"))?;
        validate_journal(&journal, project_id, issue_id)?;
        validate_redirect_journal(&redirect, &journal)?;
        let mut repository_ids = vec![journal.repository_id];
        repository_ids.sort_unstable();
        let _repository_locks =
            acquire_repository_locks(paths, project_id, issue_id, &repository_ids)?;
        ensure_migrated_record_identity(
            &repository_path(paths, project_id, issue_id, journal.repository_id),
            &journal.migrated_record,
        )?;
        Ok(result_from_journal(&journal))
    }
}

#[derive(Debug, Clone, Copy)]
struct IdentityMapping {
    repository_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
}

fn unique_identity_mapping(
    paths: &ProductAppPaths,
    project_id: &str,
    legacy_repository_id: &str,
) -> Result<IdentityMapping, ProductStoreError> {
    validate_relative_id(legacy_repository_id)?;
    let journal = IdentityMigrationJournalStore::new(paths.clone())
        .load(project_id)?
        .ok_or_else(|| inconsistent("identity migration journal is missing"))?;
    let mappings = journal
        .mappings
        .iter()
        .filter(|mapping| {
            mapping.legacy_repository_id == legacy_repository_id
                && mapping.physical_repository_id == legacy_repository_id
        })
        .collect::<Vec<_>>();
    let [mapping] = mappings.as_slice() else {
        return Err(inconsistent(format!(
            "legacy repository {} has no unique physical-to-logical mapping",
            legacy_repository_id
        )));
    };
    Ok(IdentityMapping {
        repository_id: mapping.logical_repository_id,
        checkout_id: mapping.primary_checkout_id,
    })
}

fn validate_legacy_record(
    record: &IssueSharedWorktree,
    project_id: &str,
    issue_id: &str,
) -> Result<(), ProductStoreError> {
    validate_scope(project_id, issue_id)?;
    validate_relative_id(&record.id)?;
    validate_relative_id(&record.project_id)?;
    validate_relative_id(&record.issue_id)?;
    validate_relative_id(&record.repository_id)?;
    if record.project_id != project_id || record.issue_id != issue_id {
        return Err(inconsistent(
            "legacy record scope does not match migration scope",
        ));
    }
    if record.path_schema_version > 1 {
        return Err(inconsistent(
            "legacy record has unsupported path schema version",
        ));
    }
    Ok(())
}

fn validate_legacy_mapping_identity(
    record: &IssueSharedWorktree,
    mapping: IdentityMapping,
) -> Result<(), ProductStoreError> {
    if record
        .target_repository_id
        .is_some_and(|repository_id| repository_id != mapping.repository_id)
        || record
            .checkout_id
            .is_some_and(|checkout_id| checkout_id != mapping.checkout_id)
    {
        return Err(inconsistent(
            "legacy record logical identity fields differ from migration journal mapping",
        ));
    }
    Ok(())
}

fn migrated_record(
    legacy: &IssueSharedWorktree,
    repository_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
) -> IssueSharedWorktree {
    let mut migrated = legacy.clone();
    migrated.id = format!(
        "repo_shared_worktree_{}_{}_{}",
        legacy.project_id, legacy.issue_id, repository_id.0
    );
    migrated.repository_id = repository_id.0.to_string();
    migrated.target_repository_id = Some(repository_id);
    migrated.checkout_id = Some(checkout_id);
    migrated.path_schema_version = 1;
    migrated
}

fn has_active_references(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    record: &IssueSharedWorktree,
) -> Result<bool, ProductStoreError> {
    if record.current_active_work_item_id.is_some() || record.current_lock_owner_id.is_some() {
        return Ok(true);
    }
    Ok(CodingAttemptStore::new(paths.clone())
        .list_attempts_for_issue(project_id, issue_id)?
        .into_iter()
        .any(|attempt| attempt.status.is_active()))
}

fn load_or_create_journal(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    legacy_repository_id: &str,
    repository_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
    migrated_record: &IssueSharedWorktree,
) -> Result<LegacySharedWorktreeMigrationJournal, ProductStoreError> {
    if let Some(journal) = load_journal(paths, project_id, issue_id)? {
        validate_journal(&journal, project_id, issue_id)?;
        if journal.legacy_repository_id != legacy_repository_id
            || journal.repository_id != repository_id
            || journal.checkout_id != checkout_id
            || journal.migrated_record != *migrated_record
        {
            return Err(inconsistent(
                "migration journal identity differs from legacy record",
            ));
        }
        return Ok(journal);
    }
    let now = Utc::now().to_rfc3339();
    let journal = LegacySharedWorktreeMigrationJournal {
        schema_version: LEGACY_SHARED_WORKTREE_MIGRATION_SCHEMA_VERSION,
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        legacy_repository_id: legacy_repository_id.to_string(),
        repository_id,
        checkout_id,
        migrated_record: migrated_record.clone(),
        new_record_persisted: false,
        redirect_persisted: false,
        legacy_cleanup_completed: false,
        created_at: now.clone(),
        updated_at: now,
    };
    save_journal(paths, &journal)?;
    Ok(journal)
}

fn load_journal(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Option<LegacySharedWorktreeMigrationJournal>, ProductStoreError> {
    let path = journal_path(paths, project_id, issue_id);
    if !path.exists() {
        return Ok(None);
    }
    read_json(&path).map(Some).map_err(|error| match error {
        ProductStoreError::Json(message) => {
            inconsistent(format!("migration journal decode: {message}"))
        }
        other => other,
    })
}

fn save_journal(
    paths: &ProductAppPaths,
    journal: &LegacySharedWorktreeMigrationJournal,
) -> Result<(), ProductStoreError> {
    validate_journal(journal, &journal.project_id, &journal.issue_id)?;
    write_json(
        &journal_path(paths, &journal.project_id, &journal.issue_id),
        journal,
    )
}

fn validate_journal(
    journal: &LegacySharedWorktreeMigrationJournal,
    project_id: &str,
    issue_id: &str,
) -> Result<(), ProductStoreError> {
    validate_scope(project_id, issue_id)?;
    validate_relative_id(&journal.project_id)?;
    validate_relative_id(&journal.issue_id)?;
    validate_relative_id(&journal.legacy_repository_id)?;
    if journal.schema_version != LEGACY_SHARED_WORKTREE_MIGRATION_SCHEMA_VERSION
        || journal.project_id != project_id
        || journal.issue_id != issue_id
    {
        return Err(inconsistent("migration journal scope or schema is invalid"));
    }
    let expected = migrated_record(
        &journal.migrated_record,
        journal.repository_id,
        journal.checkout_id,
    );
    if journal.migrated_record != expected
        || journal.migrated_record.repository_id != journal.repository_id.0.to_string()
    {
        return Err(inconsistent(
            "migration journal migrated record identity is invalid",
        ));
    }
    Ok(())
}

fn redirect_for(journal: &LegacySharedWorktreeMigrationJournal) -> LegacySharedWorktreeRedirect {
    LegacySharedWorktreeRedirect {
        kind: LEGACY_SHARED_WORKTREE_REDIRECT_KIND.to_string(),
        schema_version: LEGACY_SHARED_WORKTREE_MIGRATION_SCHEMA_VERSION,
        project_id: journal.project_id.clone(),
        issue_id: journal.issue_id.clone(),
        repository_id: journal.repository_id,
        migration_journal: LEGACY_SHARED_WORKTREE_MIGRATION_JOURNAL_FILE.to_string(),
        migrated_at: Utc::now().to_rfc3339(),
    }
}

fn ensure_redirect(
    path: &Path,
    expected: &LegacySharedWorktreeRedirect,
) -> Result<(), ProductStoreError> {
    match read_json_value_if_exists(path)? {
        Some(value) if is_redirect_value(&value) => {
            let actual = parse_redirect(value)?;
            if actual.kind != expected.kind
                || actual.schema_version != expected.schema_version
                || actual.project_id != expected.project_id
                || actual.issue_id != expected.issue_id
                || actual.repository_id != expected.repository_id
                || actual.migration_journal != expected.migration_journal
            {
                return Err(inconsistent(
                    "legacy redirect identity differs from migration journal",
                ));
            }
            Ok(())
        }
        Some(_) => write_json(path, expected),
        None => Err(inconsistent(
            "legacy record disappeared before redirect write",
        )),
    }
}

fn ensure_migrated_record(
    path: &Path,
    expected: &IssueSharedWorktree,
) -> Result<(), ProductStoreError> {
    match read_json_value_if_exists(path)? {
        Some(value) => {
            let actual = serde_json::from_value::<IssueSharedWorktree>(value).map_err(|error| {
                inconsistent(format!("repository-keyed record decode: {error}"))
            })?;
            if actual != *expected {
                return Err(inconsistent(
                    "repository-keyed record conflicts with migration journal",
                ));
            }
            Ok(())
        }
        None => write_json(path, expected),
    }
}

fn ensure_migrated_record_identity(
    path: &Path,
    expected: &IssueSharedWorktree,
) -> Result<(), ProductStoreError> {
    let Some(value) = read_json_value_if_exists(path)? else {
        return Err(inconsistent(
            "repository-keyed record is missing after redirect",
        ));
    };
    let actual = serde_json::from_value::<IssueSharedWorktree>(value)
        .map_err(|error| inconsistent(format!("repository-keyed record decode: {error}")))?;
    if actual.id != expected.id
        || actual.project_id != expected.project_id
        || actual.issue_id != expected.issue_id
        || actual.repository_id != expected.repository_id
        || actual.target_repository_id != expected.target_repository_id
        || actual.checkout_id != expected.checkout_id
        || actual.path_schema_version != expected.path_schema_version
    {
        return Err(inconsistent(
            "repository-keyed record identity conflicts with migration journal",
        ));
    }
    Ok(())
}

fn validate_redirect(
    redirect: &LegacySharedWorktreeRedirect,
    project_id: &str,
    issue_id: &str,
) -> Result<(), ProductStoreError> {
    validate_scope(project_id, issue_id)?;
    validate_relative_id(&redirect.project_id)?;
    validate_relative_id(&redirect.issue_id)?;
    if redirect.kind != LEGACY_SHARED_WORKTREE_REDIRECT_KIND
        || redirect.schema_version != LEGACY_SHARED_WORKTREE_MIGRATION_SCHEMA_VERSION
        || redirect.project_id != project_id
        || redirect.issue_id != issue_id
        || redirect.migration_journal != LEGACY_SHARED_WORKTREE_MIGRATION_JOURNAL_FILE
    {
        return Err(inconsistent("legacy redirect schema or scope is invalid"));
    }
    Ok(())
}

fn validate_redirect_journal(
    redirect: &LegacySharedWorktreeRedirect,
    journal: &LegacySharedWorktreeMigrationJournal,
) -> Result<(), ProductStoreError> {
    if redirect.repository_id != journal.repository_id || !journal.redirect_persisted {
        return Err(inconsistent(
            "legacy redirect does not match migration journal",
        ));
    }
    Ok(())
}

fn acquire_repository_locks(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    repository_ids: &[LogicalRepositoryId],
) -> Result<Vec<ExclusiveFileLock>, ProductStoreError> {
    repository_ids
        .iter()
        .map(|repository_id| {
            ExclusiveFileLock::acquire(&repository_path(
                paths,
                project_id,
                issue_id,
                *repository_id,
            ))
        })
        .collect()
}

fn result_from_journal(
    journal: &LegacySharedWorktreeMigrationJournal,
) -> LegacySharedWorktreeMigrationResult {
    LegacySharedWorktreeMigrationResult {
        repository_id: journal.repository_id,
        migrated_record: journal.migrated_record.clone(),
        redirect_persisted: journal.redirect_persisted,
        legacy_cleanup_completed: journal.legacy_cleanup_completed,
    }
}

fn parse_redirect(
    value: serde_json::Value,
) -> Result<LegacySharedWorktreeRedirect, ProductStoreError> {
    serde_json::from_value(value)
        .map_err(|error| inconsistent(format!("legacy redirect decode: {error}")))
}

fn is_redirect_value(value: &serde_json::Value) -> bool {
    value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == LEGACY_SHARED_WORKTREE_REDIRECT_KIND)
}

fn read_json_value_if_exists(path: &Path) -> Result<Option<serde_json::Value>, ProductStoreError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            read_json(path).map(Some).map_err(|error| match error {
                ProductStoreError::Json(message) => {
                    inconsistent(format!("decode {}: {message}", path.display()))
                }
                other => other,
            })
        }
        Ok(_) => Err(inconsistent(format!(
            "{} is not a regular file",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

fn remove_file_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

fn validate_scope(project_id: &str, issue_id: &str) -> Result<(), ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(issue_id)
}

fn legacy_path(paths: &ProductAppPaths, project_id: &str, issue_id: &str) -> PathBuf {
    paths
        .issue_root(project_id, issue_id)
        .join("issue-shared-worktree.json")
}

fn repository_path(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    repository_id: LogicalRepositoryId,
) -> PathBuf {
    paths
        .issue_root(project_id, issue_id)
        .join("shared-worktrees")
        .join(format!("{}.json", repository_id.0))
}

fn journal_path(paths: &ProductAppPaths, project_id: &str, issue_id: &str) -> PathBuf {
    paths
        .issue_root(project_id, issue_id)
        .join(LEGACY_SHARED_WORKTREE_MIGRATION_JOURNAL_FILE)
}

fn lock_path_for(target: &Path) -> PathBuf {
    let file_name = target
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "coding-attempt-store".into());
    target.with_file_name(format!(".{file_name}.lock"))
}

fn retired_legacy_lock_path(legacy_path: &Path) -> PathBuf {
    legacy_path.with_file_name(".issue-shared-worktree.json.lock.retired")
}

fn touch(journal: &mut LegacySharedWorktreeMigrationJournal) {
    journal.updated_at = Utc::now().to_rfc3339();
}

fn inconsistent(reason: impl Into<String>) -> ProductStoreError {
    ProductStoreError::InvalidRecord {
        kind: "legacy_shared_worktree_migration",
        reason: format!("legacy_shared_worktree_inconsistent: {}", reason.into()),
    }
}

#[cfg(test)]
#[path = "legacy_shared_worktree_migration_tests.rs"]
mod tests;
