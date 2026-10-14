use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
use crate::product::entity_operation_store::{
    EntityOperationError, EntityOperationStore, EntityRunLease,
};
use crate::product::json_store::{ProductStoreError, validate_relative_id};

use std::path::PathBuf;

use super::locking::{ExclusiveFileLock, canonical_lock_path_identity, canonical_path_identity};

/// fix-cascade-deletion Task D2.1c：creation guard 在原 creation 文件锁
/// **外层**取得 owning Plan scope 共享准入（plan §0.2/§0.4：scope → 对象
/// 锁），并随 guard 生命周期持有到 attempt 持久化收口——创建在途时删除
/// 独占立即 `entity_busy`（零墓碑零破坏）；purging/deleted 墓碑在位时
/// 创建稳定拒绝 `Conflict{entity_deleting}`（REQ-DEL-05/06）。guard 原有
/// identity/active uniqueness/admission 语义零变化。
pub struct WorkItemAttemptCreationGuard {
    project_id: String,
    issue_id: String,
    work_item_id: String,
    canonical_store_root: PathBuf,
    canonical_lock_path: PathBuf,
    plan_admission: Option<EntityRunLease>,
    _lock: ExclusiveFileLock,
}

impl WorkItemAttemptCreationGuard {
    /// guard 已持有的 owning Plan scope 共享准入（消费面：journal ensure
    /// 等 store 写复用同一已持 lease，不嵌套重取 scope）。
    pub(crate) fn admitted_plan_scope(&self) -> Option<&EntityScope> {
        self.plan_admission.as_ref().map(|lease| lease.scope())
    }

    pub(crate) fn validate_identity(
        &self,
        store: &super::CodingAttemptStore,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<(), ProductStoreError> {
        let expected_store_root = canonical_path_identity(store.paths.root())?;
        let expected_lock_path = canonical_lock_path_identity(
            &store.work_item_attempt_creation_path(project_id, issue_id, work_item_id),
        )?;
        if self.project_id == project_id
            && self.issue_id == issue_id
            && self.work_item_id == work_item_id
            && self.canonical_store_root == expected_store_root
            && self.canonical_lock_path == expected_lock_path
        {
            return Ok(());
        }
        Err(ProductStoreError::IdentityMismatch {
            kind: "work_item_attempt_creation_guard",
            id: work_item_id.to_string(),
        })
    }
}

impl super::CodingAttemptStore {
    pub fn acquire_work_item_attempt_creation(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<WorkItemAttemptCreationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        // D2.1c：owning Plan scope 共享准入在 creation 文件锁之前（scope →
        // 对象锁固定序）；无归属 legacy WI 保持原行为（None 不准入）。
        let plan_admission = self
            .owning_plan_scope_for_work_item(project_id, issue_id, work_item_id)?
            .map(|scope| self.acquire_plan_scope_admission(&scope))
            .transpose()?;
        let lock_target = self.work_item_attempt_creation_path(project_id, issue_id, work_item_id);
        let lock = ExclusiveFileLock::acquire(&lock_target)?;
        self.work_item_attempt_creation_guard(
            project_id,
            issue_id,
            work_item_id,
            plan_admission,
            lock,
        )
    }

    pub async fn acquire_work_item_attempt_creation_async(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<WorkItemAttemptCreationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        // D2.1c：同同步版——scope 共享准入先于 creation 文件锁。
        let plan_admission = self
            .owning_plan_scope_for_work_item(project_id, issue_id, work_item_id)?
            .map(|scope| self.acquire_plan_scope_admission(&scope))
            .transpose()?;
        let lock_target = self.work_item_attempt_creation_path(project_id, issue_id, work_item_id);
        let lock = ExclusiveFileLock::acquire_async(&lock_target).await?;
        self.work_item_attempt_creation_guard(
            project_id,
            issue_id,
            work_item_id,
            plan_admission,
            lock,
        )
    }

    fn work_item_attempt_creation_guard(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        plan_admission: Option<EntityRunLease>,
        lock: ExclusiveFileLock,
    ) -> Result<WorkItemAttemptCreationGuard, ProductStoreError> {
        Ok(WorkItemAttemptCreationGuard {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            work_item_id: work_item_id.to_string(),
            canonical_store_root: canonical_path_identity(self.paths.root())?,
            canonical_lock_path: lock.canonical_lock_path().to_path_buf(),
            plan_admission,
            _lock: lock,
        })
    }

    /// work_item → owning Plan scope（D2.1c，与删除门禁同一归属判据）：
    /// ① WI 记录 `source_work_item_plan_id`；② 缺失按 issue plan
    /// `work_item_ids` refs 反查。档案缺失/无归属 → None（legacy 创建保持
    /// 原行为，非授权放宽——此类 WI 不在任何可删 Plan 闭包内）。
    fn owning_plan_scope_for_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Option<EntityScope>, ProductStoreError> {
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(self.paths.clone());
        for record in lifecycle.list_work_items(project_id, issue_id)? {
            if record.id == work_item_id {
                if let Some(plan_id) = &record.source_work_item_plan_id {
                    return Ok(Some(EntityScope::new(
                        project_id,
                        issue_id,
                        DeletionEntityKind::Plan,
                        plan_id,
                    )));
                }
                break;
            }
        }
        for plan in lifecycle.list_issue_work_item_plans(project_id, issue_id)? {
            if plan.work_item_ids.iter().any(|id| id == work_item_id) {
                return Ok(Some(EntityScope::new(
                    project_id,
                    issue_id,
                    DeletionEntityKind::Plan,
                    plan.id,
                )));
            }
        }
        Ok(None)
    }

    /// D2.1c：owning Plan scope 共享准入（try-shared＋锁内墓碑复核）。
    /// 删除窗口（purging/deleted 墓碑或独占争用）一律稳定拒绝
    /// `Conflict{entity_deleting}`——与 `coding_plan_run`/`advance_store`
    /// 的 with_plan_live_write 惯例同族，不区分相位（REQ-DEL-06 迟到写
    /// 防御 fail-closed，非阻塞零等待）。
    fn acquire_plan_scope_admission(
        &self,
        scope: &EntityScope,
    ) -> Result<EntityRunLease, ProductStoreError> {
        EntityOperationStore::new(self.paths.clone())
            .acquire_run(scope)
            .map_err(|error| match error {
                EntityOperationError::Store(error) => error,
                EntityOperationError::Busy
                | EntityOperationError::Deleting
                | EntityOperationError::Deleted => ProductStoreError::Conflict {
                    kind: "entity_deleting",
                    id: scope.entity_id.clone(),
                },
            })
    }

    /// D2.1c：group/direct 创建面的 owning Plan live-write 准入（同
    /// `coding_plan_run.rs`/`advance_store.rs` 的 with_plan_live_write 惯例）：
    /// scope try-shared 外层（争用/墓碑立即 `Conflict{entity_deleting}`，
    /// 零阻塞等待）→ 原创建逻辑内层。advance 编排在 arbitration 锁内
    //  调用本族入口时，本 try 准入仍 fail-closed 拒墓碑（D2.1d 将在
    //  advance 入口外层持同 Plan lease，届时内层只消费已持准入）。
    pub(crate) fn with_plan_live_write<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        scope.validate()?;
        EntityOperationStore::new(self.paths.clone()).with_live_write(&scope, operation)
    }
}
