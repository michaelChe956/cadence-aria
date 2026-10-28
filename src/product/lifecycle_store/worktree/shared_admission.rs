use super::{SharedWorktreeJournalView, SharedWorktreeOwnerHints};

use std::time::{Duration, Instant};

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::entity_deletion_store::{
    DeletionEntityKind, EntityDeletionManifest, EntityDeletionRecord, EntityDeletionStore,
    EntityScope, read_json_if_exists,
};
use crate::product::entity_operation_store::{EntityOperationError, SharedResourceKey};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id};
use crate::product::lifecycle_store::{LifecycleStore, path_is_regular_file};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{IssueSharedWorktree, LifecycleWorkItemRecord};

impl LifecycleStore {
    // ─── D1.3c（fix-cascade-deletion tasks 2.4）：shared-worktree 中央 writer 准入 ───
    //
    // 仅外围准入 guard：全部 upsert/try_acquire/bind/transfer/release/mark/
    // takeover 写入口在旧对象 flock 之外先取稳定 repo resource writer 短 lease
    // （plan §0.2 锁序 scope → resource → 对象锁），有真实 owning Plan 则先
    // scope try-shared 准入（内层消费已持 lease 保持原 owner equality/CAS/
    // 状态机/锁归属，删除 helper 语义零改动）。

    /// 真实归属判据导出（plan D1.3c Interfaces，D2.2 同判据消费）：
    /// 反推该 shared-worktree 资源的全部拥有 Plan scope（live 与墓碑均含，
    /// 调用方自行按墓碑过滤）。归属优先级：记录 current_active/last_completed
    /// WI → WI 记录 source_work_item_plan_id/一致 work_item_set_id/计划 refs；
    /// 记录 lock owner → attempt group 绑定或 group 初始化 journal 的
    /// worktree_lease_id join；target 快照与 record repository 键只用于核目标。
    pub fn resolve_shared_resource_owners(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: Option<LogicalRepositoryId>,
    ) -> Result<Vec<EntityScope>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let record =
            self.read_shared_worktree_for_admission(project_id, issue_id, repository_id)?;
        let legacy_repository = record.as_ref().map(|record| record.repository_id.clone());
        let mut scopes =
            self.shared_worktree_record_owner_scopes(record.as_ref(), project_id, issue_id)?;
        scopes.extend(self.shared_worktree_repo_plan_traces(
            project_id,
            issue_id,
            repository_id,
            legacy_repository.as_deref(),
        )?);
        sort_and_dedup_scopes(&mut scopes);
        Ok(scopes)
    }

    /// 中央 writer 准入：归属解析 → scope try-shared 准入（墓碑复核）→
    /// 稳定 resource writer 短 lease → 原对象 flock 临界区（零语义改动）。
    ///
    /// - hint（入口参数身份）可反推出 owning Plan：以 hint scope 准入，迟到
    ///   已删 Plan 写随墓碑复核拒绝；
    /// - hint 无法归属（旧档案/基础设施写）：记录 owners + 真实 coding traces
    ///   只取 live 准入；全部归属已墓碑（含 owned-index claim）→ 归属未知
    ///   fail-closed，禁止覆盖已 purging Plan 现资源或凭旧缓存 path 重建；
    /// - 无任何归属证据：纯 issue/repo 基础设施写，保持原行为（非授权放宽）。
    #[allow(clippy::too_many_arguments)] // 准入键位(hint/legacy/等待策略)逐项显式,避免私有结构体泄漏语义。
    pub(super) fn with_shared_worktree_write_admission<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: Option<LogicalRepositoryId>,
        hints: SharedWorktreeOwnerHints<'_>,
        wait_for_concurrent_writer: bool,
        legacy_repository: Option<&str>,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let record =
            self.read_shared_worktree_for_admission(project_id, issue_id, repository_id)?;
        let legacy_owned = legacy_repository
            .map(str::to_string)
            .or_else(|| record.as_ref().map(|record| record.repository_id.clone()));

        let mut strict: Vec<EntityScope> = Vec::new();
        for work_item_id in &hints.work_item_ids {
            if let Some(scope) =
                self.shared_worktree_plan_for_work_item(project_id, issue_id, work_item_id)?
            {
                strict.push(scope);
            }
        }
        for owner_id in &hints.owner_ids {
            if let Some(scope) = self.shared_worktree_plan_for_owner(
                project_id,
                issue_id,
                owner_id,
                record.as_ref(),
            )? {
                strict.push(scope);
            }
        }

        let admission_scopes: Vec<EntityScope> = if strict.is_empty() {
            let soft =
                self.shared_worktree_record_owner_scopes(record.as_ref(), project_id, issue_id)?;
            let mut live_scopes = Vec::new();
            let mut saw_owner = false;
            for scope in soft {
                saw_owner = true;
                if !self.shared_worktree_scope_tombstoned(&scope)? {
                    live_scopes.push(scope);
                }
            }
            if !live_scopes.is_empty() {
                live_scopes
            } else if saw_owner {
                // 记录归属全部已删/清理中：禁止覆盖已 purging Plan 的现资源。
                return Err(shared_resource_owner_deleting(project_id, issue_id));
            } else {
                // 无记录归属：真实存活 coding trace（另一 Plan 同仓正常创建）
                // 准入；否则 tombstone owned-index claim 阻断旧缓存 path 重建。
                let traces = self.shared_worktree_repo_plan_traces(
                    project_id,
                    issue_id,
                    repository_id,
                    legacy_owned.as_deref(),
                )?;
                let mut live_traces = Vec::new();
                for scope in traces {
                    if !self.shared_worktree_scope_tombstoned(&scope)? {
                        live_traces.push(scope);
                    }
                }
                if !live_traces.is_empty() {
                    live_traces
                } else if self.shared_resource_claimed_by_tombstoned_plans(
                    project_id,
                    issue_id,
                    repository_id,
                    legacy_owned.as_deref(),
                    record.as_ref(),
                )? {
                    return Err(shared_resource_owner_deleting(project_id, issue_id));
                } else {
                    Vec::new()
                }
            }
        } else {
            strict
        };

        self.with_entity_live_writes(&admission_scopes, || {
            let key = SharedResourceKey {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id,
            };
            self.with_shared_resource_writer_lease(&key, wait_for_concurrent_writer, operation)
        })
    }

    /// 读取准入用记录（NotFound=None）；记录 repository 键只用于核目标——
    /// repo 维（Some 键）严格比对 target；issue 维键位由文件路径确定、
    /// 容忍迁移 tagged 态，只核 project/issue 身份。错档 IdentityMismatch
    /// fail-closed（不改写归属矛盾的现资源）。
    fn read_shared_worktree_for_admission(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: Option<LogicalRepositoryId>,
    ) -> Result<Option<IssueSharedWorktree>, ProductStoreError> {
        let path = match repository_id {
            Some(repository) => self.repo_shared_worktree_path(project_id, issue_id, repository),
            None => self.issue_shared_worktree_path(project_id, issue_id),
        };
        if !path_is_regular_file(&path)? {
            return Ok(None);
        }
        let record: IssueSharedWorktree = read_json(&path)?;
        if record.project_id != project_id || record.issue_id != issue_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "shared_worktree",
                id: record.id,
            });
        }
        // issue 维键位由文件路径确定：第一方迁移 backfill 会原地在
        // issue-shared-worktree.json 写 Some(target)（tagged 态，
        // legacy_shared_worktree_migration 显式容忍），故不比对 target，
        // 只保留上方 project/issue 身份核验；repo 维（Some 键）严格比对。
        if let Some(repository) = repository_id
            && record.target_repository_id != Some(repository)
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repo_shared_worktree",
                id: record.id,
            });
        }
        Ok(Some(record))
    }

    /// 记录（若在场）的归属 scope：current_active/last_completed WI 同一 WI
    /// 读面 + lock owner（attempt/journal join）。
    fn shared_worktree_record_owner_scopes(
        &self,
        record: Option<&IssueSharedWorktree>,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<EntityScope>, ProductStoreError> {
        let mut scopes = Vec::new();
        let Some(record) = record else {
            return Ok(scopes);
        };
        for work_item_id in [
            record.current_active_work_item_id.as_deref(),
            record.last_completed_work_item_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(scope) =
                self.shared_worktree_plan_for_work_item(project_id, issue_id, work_item_id)?
            {
                scopes.push(scope);
            }
        }
        if let Some(owner_id) = record.current_lock_owner_id.as_deref()
            && let Some(scope) =
                self.shared_worktree_plan_for_owner(project_id, issue_id, owner_id, Some(record))?
        {
            scopes.push(scope);
        }
        sort_and_dedup_scopes(&mut scopes);
        Ok(scopes)
    }

    /// WI id → owning Plan scope。优先级链（plan Interfaces）：
    /// ① WI 记录 source_work_item_plan_id；② 一致 work_item_set_id join
    /// （同 set 冲突 plan = 归属错误）；③ 计划 work_item_ids refs 反查；
    /// ④ 已删除 WI 缺根时按 D1.0 owned-index（child owner 索引）反查。
    /// 全源耗尽 → None（旧无 owner 档案，不当 live 授权）。
    fn shared_worktree_plan_for_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Option<EntityScope>, ProductStoreError> {
        let work_item_path = self
            .work_items_root(project_id, issue_id)
            .join(format!("{work_item_id}.json"));
        let record = if path_is_regular_file(&work_item_path)? {
            Some(read_json::<LifecycleWorkItemRecord>(&work_item_path)?)
        } else {
            None
        };
        if let Some(record) = &record
            && let Some(plan_id) = &record.source_work_item_plan_id
        {
            return Ok(Some(plan_scope(project_id, issue_id, plan_id)));
        }
        if let Some(record) = &record
            && let Some(set_id) = &record.work_item_set_id
        {
            let mut joined: Option<String> = None;
            for other in self.list_work_items(project_id, issue_id)? {
                if other.work_item_set_id.as_deref() != Some(set_id.as_str()) {
                    continue;
                }
                let Some(plan_id) = &other.source_work_item_plan_id else {
                    continue;
                };
                match &joined {
                    None => joined = Some(plan_id.clone()),
                    Some(existing) if existing == plan_id => {}
                    Some(existing) => {
                        return Err(ProductStoreError::InvalidRecord {
                            kind: "shared_worktree_owner",
                            reason: format!(
                                "work_item_set {set_id} joins conflicting plans: {existing} vs {plan_id}"
                            ),
                        });
                    }
                }
            }
            if let Some(plan_id) = joined {
                return Ok(Some(plan_scope(project_id, issue_id, &plan_id)));
            }
        }
        for plan in self.list_issue_work_item_plans(project_id, issue_id)? {
            if plan.work_item_ids.iter().any(|id| id == work_item_id) {
                return Ok(Some(plan_scope(project_id, issue_id, &plan.id)));
            }
        }
        for (scope, manifest) in self.tombstoned_plan_owned_indexes(project_id, issue_id)? {
            if manifest.work_item_ids.iter().any(|id| id == work_item_id) {
                return Ok(Some(scope));
            }
        }
        Ok(None)
    }

    /// owner id（attempt 或不透明 lease）→ owning Plan scope。优先级链：
    /// ① attempt：group（work_item_group_id）绑定优先，attempt.work_item_id
    /// 次之；② `issue/repo_worktree_lease_*` 不透明 owner 经 group 初始化
    /// journal 实际 worktree_lease_id/lock_work_item_id/plan_id join（不按
    /// prefix 解析出 Plan）；journal lock WI 与记录 active WI 互相矛盾报
    /// 归属错误。NotFound attempt（已清档/非 attempt id）→ 继续尝试 join。
    fn shared_worktree_plan_for_owner(
        &self,
        project_id: &str,
        issue_id: &str,
        owner_id: &str,
        record: Option<&IssueSharedWorktree>,
    ) -> Result<Option<EntityScope>, ProductStoreError> {
        let attempts = CodingAttemptStore::new(self.paths.clone());
        match attempts.get_attempt(project_id, issue_id, owner_id) {
            Ok(attempt) => {
                if let Some(plan_id) = &attempt.work_item_group_id {
                    return Ok(Some(plan_scope(project_id, issue_id, plan_id)));
                }
                return self.shared_worktree_plan_for_work_item(
                    project_id,
                    issue_id,
                    &attempt.work_item_id,
                );
            }
            Err(ProductStoreError::NotFound { .. }) => {}
            Err(other) => return Err(other),
        }
        for journal in self.shared_worktree_journal_views(project_id, issue_id)? {
            if journal.worktree_lease_id != owner_id {
                continue;
            }
            if let Some(record) = record
                && let Some(active_id) = &record.current_active_work_item_id
                && active_id != &journal.lock_work_item_id
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "shared_worktree_owner",
                    reason: format!(
                        "worktree lease {owner_id} joined plan {} via lock work item {}, but record active work item is {active_id}",
                        journal.plan_id, journal.lock_work_item_id
                    ),
                });
            }
            return Ok(Some(plan_scope(project_id, issue_id, &journal.plan_id)));
        }
        Ok(None)
    }

    /// 枚举 group 初始化 journal 归属视图（单 target `{plan}.json` 与 per-target
    /// `{plan}/{repository}.json` 两个层级）。
    ///
    /// 损坏 JSON **不** fail-closed：损坏文件不再提供 lease/attempt 证据，按
    /// 缺失处理（删除侧按路径清掉），不得让残留物让整个删除/准入失败；真实
    /// IO 错误仍上抛（见 [`read_journal_ownership`]）。
    fn shared_worktree_journal_views(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<SharedWorktreeJournalView>, ProductStoreError> {
        let root = self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("coding-attempts")
            .join("group-initializations");
        let mut views = Vec::new();
        for path in json_child_paths(&root)? {
            if let JournalOwnership::View(view) = read_journal_ownership(&path)? {
                views.push(view);
            }
        }
        for plan_dir in child_directories(&root)? {
            for path in json_child_paths(&plan_dir)? {
                if let JournalOwnership::View(view) = read_journal_ownership(&path)? {
                    views.push(view);
                }
            }
        }
        Ok(views)
    }

    /// 真实存活 coding 声称 traces：target 桶匹配的 attempt（group 绑定或其
    /// work_item 归属）、target 匹配的 journal plan、目标仓 WI 记录归属 →
    /// Plan scopes（不单凭同仓判唯一 Plan，只收真实绑定）。
    fn shared_worktree_repo_plan_traces(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: Option<LogicalRepositoryId>,
        legacy_repository: Option<&str>,
    ) -> Result<Vec<EntityScope>, ProductStoreError> {
        let mut scopes = Vec::new();
        let attempts_root = self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("coding-attempts");
        for path in json_child_paths(&attempts_root)? {
            let Some(attempt) = read_json_if_exists::<
                crate::product::coding_models::CodingExecutionAttempt,
            >(&path)?
            else {
                continue;
            };
            let target_matches = match repository_id {
                Some(repository) => attempt
                    .target_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.logical_repository_id == repository),
                None => attempt.target_snapshot.is_none(),
            };
            if !target_matches {
                continue;
            }
            if let Some(plan_id) = &attempt.work_item_group_id {
                scopes.push(plan_scope(project_id, issue_id, plan_id));
            } else if let Some(scope) = self.shared_worktree_plan_for_work_item(
                project_id,
                issue_id,
                &attempt.work_item_id,
            )? {
                scopes.push(scope);
            }
        }
        let journals_root = self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("coding-attempts")
            .join("group-initializations");
        match repository_id {
            Some(repository) => {
                for plan_dir in child_directories(&journals_root)? {
                    let path = plan_dir.join(format!("{}.json", repository.0));
                    let plan_id = match read_journal_ownership(&path)? {
                        JournalOwnership::View(view) => Some(view.plan_id),
                        // 损坏残留：`{plan_id}/{repository}.json` 的父目录名即
                        // plan_id（路径界定归属），文件本身已按本 target 命中。
                        JournalOwnership::DamagedPath => plan_dir
                            .file_name()
                            .and_then(|value| value.to_str())
                            .map(str::to_string),
                        JournalOwnership::Missing => None,
                    };
                    if let Some(plan_id) = plan_id {
                        scopes.push(plan_scope(project_id, issue_id, &plan_id));
                    }
                }
            }
            None => {
                for path in json_child_paths(&journals_root)? {
                    let plan_id = match read_journal_ownership(&path)? {
                        JournalOwnership::View(view) => Some(view.plan_id),
                        // 损坏残留：`{plan_id}.json` 的文件名即 plan_id。
                        JournalOwnership::DamagedPath => path
                            .file_stem()
                            .and_then(|value| value.to_str())
                            .map(str::to_string),
                        JournalOwnership::Missing => None,
                    };
                    if let Some(plan_id) = plan_id {
                        scopes.push(plan_scope(project_id, issue_id, &plan_id));
                    }
                }
            }
        }
        for work_item in self.list_work_items(project_id, issue_id)? {
            let target_matches = match repository_id {
                Some(repository) => work_item.target_repository_id == Some(repository),
                None => {
                    work_item.target_repository_id.is_none()
                        && legacy_repository.is_some_and(|legacy| work_item.repository_id == legacy)
                }
            };
            if !target_matches {
                continue;
            }
            if let Some(plan_id) = &work_item.source_work_item_plan_id {
                scopes.push(plan_scope(project_id, issue_id, plan_id));
            }
        }
        sort_and_dedup_scopes(&mut scopes);
        Ok(scopes)
    }

    /// 已删/purging Plan 的 owned-index 集合（D1.0 child owner 索引；root/pending
    /// 均缺后仍可反查）。
    fn tombstoned_plan_owned_indexes(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<(EntityScope, EntityDeletionManifest)>, ProductStoreError> {
        let root = self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("entity-deletions")
            .join("plan");
        let mut pairs = Vec::new();
        for path in json_child_paths(&root)? {
            let Some(record) = read_json_if_exists::<EntityDeletionRecord>(&path)? else {
                continue;
            };
            if record.scope.kind != DeletionEntityKind::Plan {
                continue;
            }
            let manifest = EntityDeletionStore::new(self.paths.clone())
                .list_owned_artifact_ids(&record.scope)?;
            pairs.push((record.scope, manifest));
        }
        Ok(pairs)
    }

    /// scope 墓碑在位（purging/deleted 均真）。
    fn shared_worktree_scope_tombstoned(
        &self,
        scope: &EntityScope,
    ) -> Result<bool, ProductStoreError> {
        Ok(EntityDeletionStore::new(self.paths.clone())
            .get(scope)?
            .is_some())
    }

    /// tombstone claim：purging/deleted Plan 的 owned-index 是否声称本资源
    /// （target_repository_ids 含键仓；或记录 active/last_completed WI 落在
    /// 其 work_item_ids 内）——无可证 live owner 时阻断旧缓存 path 重建。
    #[allow(clippy::type_complexity)]
    fn shared_resource_claimed_by_tombstoned_plans(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_id: Option<LogicalRepositoryId>,
        legacy_repository: Option<&str>,
        record: Option<&IssueSharedWorktree>,
    ) -> Result<bool, ProductStoreError> {
        let claim_tokens: Vec<String> = match repository_id {
            Some(repository) => vec![repository.0.to_string()],
            None => legacy_repository.map(str::to_string).into_iter().collect(),
        };
        for (_scope, manifest) in self.tombstoned_plan_owned_indexes(project_id, issue_id)? {
            if manifest
                .target_repository_ids
                .iter()
                .any(|token| claim_tokens.contains(token))
            {
                return Ok(true);
            }
            if let Some(record) = record {
                let record_work_items = [
                    record.current_active_work_item_id.as_deref(),
                    record.last_completed_work_item_id.as_deref(),
                ];
                if manifest
                    .work_item_ids
                    .iter()
                    .any(|work_item_id| record_work_items.contains(&Some(work_item_id.as_str())))
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// 稳定 repo resource writer 短 lease（非阻塞独占；与删除 cleanup 对称
    /// 互斥，inode 在 purge 外）。争用 → 稳定 `shared_resource_busy`；
    /// `wait_for_concurrent_writer=true`（try_acquire 族）以有界短等待保持
    /// 旧对象 flock 的串行合同——并发同键 acquire 的败者必须读到胜者记录
    /// 后回稳定 CAS 错（repo/issue_worktree_active），而非资源 Busy。
    fn with_shared_resource_writer_lease<T>(
        &self,
        key: &SharedResourceKey,
        wait_for_concurrent_writer: bool,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let operations = self.entity_operation_store();
        let acquire = || match operations.try_acquire_shared_resource_write(key) {
            Ok(lease) => Ok(lease),
            Err(EntityOperationError::Busy) => Err(ProductStoreError::Conflict {
                kind: "shared_resource_busy",
                id: format!("{}/{}", key.project_id, key.issue_id),
            }),
            Err(EntityOperationError::Store(error)) => Err(error),
            Err(EntityOperationError::Deleting | EntityOperationError::Deleted) => {
                Err(ProductStoreError::Conflict {
                    kind: "entity_deleting",
                    id: format!("{}/{}", key.project_id, key.issue_id),
                })
            }
        };
        let lease = if wait_for_concurrent_writer {
            const WAIT_BUDGET: Duration = Duration::from_secs(5);
            const RETRY_BACKOFF: Duration = Duration::from_millis(2);
            let started = Instant::now();
            loop {
                match acquire() {
                    Ok(lease) => break lease,
                    Err(
                        error @ ProductStoreError::Conflict {
                            kind: "shared_resource_busy",
                            ..
                        },
                    ) => {
                        if started.elapsed() >= WAIT_BUDGET {
                            return Err(error);
                        }
                        std::thread::sleep(RETRY_BACKOFF);
                    }
                    Err(error) => return Err(error),
                }
            }
        } else {
            acquire()?
        };
        let result = operation();
        drop(lease);
        result
    }
}

fn plan_scope(project_id: &str, issue_id: &str, plan_id: &str) -> EntityScope {
    EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id)
}

fn shared_resource_owner_deleting(project_id: &str, issue_id: &str) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: "entity_deleting",
        id: format!("{project_id}/{issue_id}"),
    }
}

fn sort_and_dedup_scopes(scopes: &mut Vec<EntityScope>) {
    scopes.sort_by(|left, right| {
        (
            left.kind.path_segment(),
            left.project_id.as_str(),
            left.issue_id.as_str(),
            left.entity_id.as_str(),
        )
            .cmp(&(
                right.kind.path_segment(),
                right.project_id.as_str(),
                right.issue_id.as_str(),
                right.entity_id.as_str(),
            ))
    });
    scopes.dedup();
}

/// journal 文件在归属解析中的解读结果。
enum JournalOwnership {
    /// 内容有效：真实 owner-key（plan_id / lease / lock WI）。
    View(SharedWorktreeJournalView),
    /// 文件存在但内容不可解析：**路径界定归属**——`{plan_id}.json` 的文件名或
    /// `{plan_id}/{target}.json` 的父目录名本身即 plan_id，故它仍是本 plan 的
    /// 孤儿残留（不再携带 lease/attempt 证据）。不得据此中止删除（REQ-WPS-03）。
    DamagedPath,
    /// 文件缺失：缺失即已清。
    Missing,
}

/// journal 归属解读（**路径界定归属**的残留口径）。
///
/// - 文件缺失 → [`JournalOwnership::Missing`]；
/// - 内容解析失败 → [`JournalOwnership::DamagedPath`]：损坏的 journal 不再是
///   有效记录（删除侧按路径直接清掉），只说明它不能提供 lease/attempt 证据，
///   绝不让整个删除/准入因残留物不可解析而失败；
/// - 真实 IO 错误（权限拒绝、打开失败等）→ 上抛 fail-closed（REQ-WPS-04：只有
///   真实失败才中止，残留不可解析不是删除失败）。
fn read_journal_ownership(path: &std::path::Path) -> Result<JournalOwnership, ProductStoreError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(JournalOwnership::Missing);
        }
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "open {}: {error}",
                path.display()
            )));
        }
    };
    match serde_json::from_reader(file) {
        Ok(view) => Ok(JournalOwnership::View(view)),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "unparseable group initialization journal residue (path-scoped ownership)"
            );
            Ok(JournalOwnership::DamagedPath)
        }
    }
}

/// 目录的直接子 `.json` 文件路径（目录缺失=空；真实 IO 错误上抛）。
fn json_child_paths(root: &std::path::Path) -> Result<Vec<std::path::PathBuf>, ProductStoreError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                root.display()
            )));
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    Ok(paths)
}

/// 目录的直接子目录路径（目录缺失=空；真实 IO 错误上抛）。
fn child_directories(root: &std::path::Path) -> Result<Vec<std::path::PathBuf>, ProductStoreError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                root.display()
            )));
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            paths.push(path);
        }
    }
    Ok(paths)
}
