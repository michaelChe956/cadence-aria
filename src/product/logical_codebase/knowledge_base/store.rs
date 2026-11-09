//! 知识库 operation 的 per-LC durable store：每条 operation 一个
//! `<operation_id>.json`，落在该 LC scope 的 `knowledge-base-operations/`
//! 目录下（复用 `lc_scope_root` 的 per-LC / legacy 别名布局）。

use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationErrorRecord;
use crate::product::logical_codebase::lc_scope_root;

use super::operation::{
    KnowledgeBaseOperation, KnowledgeBaseOperationStatus, KnowledgeBaseStepKind,
    KnowledgeBaseStepStatus, derive_operation_id,
};

/// 知识库 operation 的 durable store。绑定到一个 LC 后，所有读写只作用于
/// 该 LC 自己的 `knowledge-base-operations/` 目录，天然与其它 LC 隔离。
#[derive(Debug, Clone)]
pub struct KnowledgeBaseOperationStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl KnowledgeBaseOperationStore {
    /// 把 store 绑定到一个 LC；之后的全部读写都限定在该 LC 的 scope 内。
    pub fn for_lc(paths: ProductAppPaths, lc_id: String) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id),
        }
    }

    /// 当前 LC 的 operation 落盘根目录。
    fn operations_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(lc_scope_root(&self.paths, project_id, &self.lc_id)?.join("knowledge-base-operations"))
    }

    fn operation_path(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        Ok(self
            .operations_root(project_id)?
            .join(format!("{operation_id}.json")))
    }

    /// 创建 operation（幂等）：目标文件已存在时直接返回既有记录。
    pub fn create(
        &self,
        operation: KnowledgeBaseOperation,
    ) -> Result<KnowledgeBaseOperation, ProductStoreError> {
        self.ensure_record_consistency(&operation)?;
        let path = self.operation_path(&operation.project_id, &operation.operation_id)?;
        if path.exists() {
            let existing: KnowledgeBaseOperation = read_json(&path)?;
            self.ensure_record_consistency(&existing)?;
            return Ok(existing);
        }
        write_json(&path, &operation)?;
        Ok(operation)
    }

    pub fn get(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<KnowledgeBaseOperation, ProductStoreError> {
        let path = self.operation_path(project_id, operation_id)?;
        if !path.exists() {
            return Err(not_found(operation_id));
        }
        let operation: KnowledgeBaseOperation = read_json(&path)?;
        self.ensure_record_consistency(&operation)?;
        Ok(operation)
    }

    /// 当前 LC 最新一条 operation（按 `created_at` 降序；无记录时为 `None`）。
    pub fn latest(
        &self,
        project_id: &str,
    ) -> Result<Option<KnowledgeBaseOperation>, ProductStoreError> {
        let mut operations = self.read_all(project_id)?;
        sort_newest_first(&mut operations);
        Ok(operations.into_iter().next())
    }

    /// 标记失败：operation 必须处于 `Running`；`step` 给出时把该步骤同步
    /// 置为 `Failed` 并记录完成时间。
    pub fn finish_failed(
        &self,
        project_id: &str,
        operation_id: &str,
        step: Option<KnowledgeBaseStepKind>,
        error: AggregateInitializationErrorRecord,
        at: String,
    ) -> Result<(), ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != KnowledgeBaseOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            if let Some(step_id) = step {
                let record = operation
                    .steps
                    .iter_mut()
                    .find(|record| record.step_id == step_id)
                    .ok_or_else(|| identity_mismatch(operation_id))?;
                record.status = KnowledgeBaseStepStatus::Failed;
                record.completed_at = Some(at.clone());
            }
            operation.status = KnowledgeBaseOperationStatus::Failed;
            operation.error = Some(error);
            operation.completed_at = Some(at);
            Ok(())
        })
        .map(|_| ())
    }

    /// 标记成功：operation 必须处于 `Running`。
    pub fn finish_completed(
        &self,
        project_id: &str,
        operation_id: &str,
        at: String,
    ) -> Result<(), ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != KnowledgeBaseOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            operation.status = KnowledgeBaseOperationStatus::Completed;
            operation.completed_at = Some(at);
            Ok(())
        })
        .map(|_| ())
    }

    /// 原子成功收口：把指定步骤置为 `Completed`（写完成时间与输出摘要），
    /// 同时把 operation 置为 `Completed`；operation 必须处于 `Running`，
    /// 且步骤必须存在。步骤与 operation 在同一次读改写中落盘，不会出现
    /// 「operation 已完成、步骤仍运行」的中间态。
    pub fn complete_step(
        &self,
        project_id: &str,
        operation_id: &str,
        step: KnowledgeBaseStepKind,
        output_summary: Option<String>,
        at: String,
    ) -> Result<(), ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != KnowledgeBaseOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            let record = operation
                .steps
                .iter_mut()
                .find(|record| record.step_id == step)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            record.status = KnowledgeBaseStepStatus::Completed;
            record.completed_at = Some(at.clone());
            record.output_summary = output_summary;
            operation.status = KnowledgeBaseOperationStatus::Completed;
            operation.completed_at = Some(at);
            Ok(())
        })
        .map(|_| ())
    }

    /// 取消并附带一条附加失败事实（如取消前 provider 已越界写）。
    ///
    /// - `Running`：置 `Cancelled`，写入 `error`；
    /// - 已 `Cancelled` 且尚无 `error`：只补写 `error`，保留原完成时间；
    /// - 其它状态（含已带 `error` 的 `Cancelled`）：`IdentityMismatch`。
    pub fn cancel_with_error(
        &self,
        project_id: &str,
        operation_id: &str,
        error: AggregateInitializationErrorRecord,
        at: String,
    ) -> Result<(), ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            match operation.status {
                KnowledgeBaseOperationStatus::Running => {
                    operation.status = KnowledgeBaseOperationStatus::Cancelled;
                    operation.completed_at = Some(at);
                }
                KnowledgeBaseOperationStatus::Cancelled if operation.error.is_none() => {}
                _ => return Err(identity_mismatch(operation_id)),
            }
            operation.error = Some(error);
            Ok(())
        })
        .map(|_| ())
    }

    /// 在当前 LC 的跨进程排他锁内执行 `critical`。
    ///
    /// 单飞的「查重 + 创建」必须放进同一临界区：durable `list_running` 只是
    /// 观察，不是占位；两个共享本 store 的进程若各自先查后建，会同时放行。
    /// 锁文件落在该 LC 的 operations 目录内（非 `.json`，不会被 `read_all`
    /// 当作 operation 读取），flock 对同进程内不同 fd 同样互斥。
    pub fn with_begin_lock<T>(
        &self,
        project_id: &str,
        critical: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let lock_path = self.operations_root(project_id)?.join(".begin.lock");
        with_exact_exclusive_lock(&lock_path, critical)
    }

    /// 取消：operation 必须处于 `Running`。
    pub fn cancel(
        &self,
        project_id: &str,
        operation_id: &str,
        at: String,
    ) -> Result<(), ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != KnowledgeBaseOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            operation.status = KnowledgeBaseOperationStatus::Cancelled;
            operation.completed_at = Some(at);
            Ok(())
        })
        .map(|_| ())
    }

    /// 列出当前 LC 仍处于 `Running` 的 operation（按 `created_at` 降序）。
    pub fn list_running(
        &self,
        project_id: &str,
    ) -> Result<Vec<KnowledgeBaseOperation>, ProductStoreError> {
        let mut operations = self.read_all(project_id)?;
        operations.retain(|operation| operation.status == KnowledgeBaseOperationStatus::Running);
        sort_newest_first(&mut operations);
        Ok(operations)
    }

    /// 读取当前 LC 目录下的全部 operation 记录（目录不存在视为空）。
    fn read_all(&self, project_id: &str) -> Result<Vec<KnowledgeBaseOperation>, ProductStoreError> {
        let root = self.operations_root(project_id)?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut operations = Vec::new();
        for entry in std::fs::read_dir(&root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
        {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let operation: KnowledgeBaseOperation = read_json(&path)?;
            self.ensure_record_consistency(&operation)?;
            operations.push(operation);
        }
        Ok(operations)
    }

    /// 读取-变更-写回。终态转换方法都经由它执行，保证落盘前校验一致性。
    ///
    /// 整个读改写在 `<operation_id>.lock` 的跨进程排他锁内进行（与 begin 锁
    /// 同机制）：JSON 原子替换只保证读者读到完整文件，挡不住「两个转换各自
    /// 读到旧副本、后写者覆盖前写者」的丢失更新——普通 `cancel` 覆盖
    /// `cancel_with_error` 附加的审计事实就是这种交错。锁文件不是 `.json`，
    /// 不会被 `read_all` 当作 operation 读取；flock 对同进程内不同 fd 同样
    /// 互斥。读者（`get` / `read_all`）不加锁，仍靠原子替换读到一致快照。
    fn update(
        &self,
        project_id: &str,
        operation_id: &str,
        update: impl FnOnce(&mut KnowledgeBaseOperation) -> Result<(), ProductStoreError>,
    ) -> Result<KnowledgeBaseOperation, ProductStoreError> {
        let path = self.operation_path(project_id, operation_id)?;
        let lock_path = self
            .operations_root(project_id)?
            .join(format!("{operation_id}.lock"));
        with_exact_exclusive_lock(&lock_path, || {
            if !path.exists() {
                return Err(not_found(operation_id));
            }
            let mut operation: KnowledgeBaseOperation = read_json(&path)?;
            self.ensure_record_consistency(&operation)?;
            update(&mut operation)?;
            write_json(&path, &operation)?;
            Ok(operation)
        })
    }

    /// 记录自洽性：project / LC id 是合法相对 id；`operation_id` 与
    /// 「project + LC + idempotency_key」的派生值一致；且记录归属当前
    /// store 绑定的 LC（防止目录与记录身份漂移）。
    fn ensure_record_consistency(
        &self,
        operation: &KnowledgeBaseOperation,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&operation.project_id)?;
        validate_relative_id(&operation.logical_codebase_id)?;
        let expected = derive_operation_id(
            &operation.project_id,
            &operation.logical_codebase_id,
            &operation.input.idempotency_key,
        );
        if operation.operation_id != expected
            || Some(&operation.logical_codebase_id) != self.lc_id.as_ref()
        {
            return Err(identity_mismatch(&operation.operation_id));
        }
        Ok(())
    }
}

/// 按 `created_at` 降序排列；时间相同用 `operation_id` 保证顺序稳定。
fn sort_newest_first(operations: &mut [KnowledgeBaseOperation]) {
    operations.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.operation_id.cmp(&left.operation_id))
    });
}

fn not_found(operation_id: &str) -> ProductStoreError {
    ProductStoreError::NotFound {
        kind: "knowledge_base_operation",
        id: operation_id.to_string(),
    }
}

fn identity_mismatch(operation_id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: "knowledge_base_operation",
        id: operation_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::operation::{
        KnowledgeBaseOperationInput, KnowledgeBaseOperationKind, KnowledgeBaseStepRecord,
    };
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_initialization::AggregateProviderSelection;

    const PROJECT: &str = "project_0001";
    const LC_ALPHA: &str = "lc_alpha";
    const LC_BETA: &str = "lc_beta";
    const CREATED_AT: &str = "2026-11-09T00:00:00Z";
    const FINISHED_AT: &str = "2026-11-09T00:00:05Z";

    fn provider_selection() -> AggregateProviderSelection {
        AggregateProviderSelection {
            selected: "pi".to_string(),
            candidates: Vec::new(),
            rule_version: "kb-v1".to_string(),
            decided_at: CREATED_AT.to_string(),
        }
    }

    fn new_operation(
        lc_id: &str,
        idempotency_key: &str,
        kind: KnowledgeBaseOperationKind,
        created_at: &str,
    ) -> KnowledgeBaseOperation {
        let (step_id, package) = match kind {
            KnowledgeBaseOperationKind::Bootstrap => (KnowledgeBaseStepKind::Bootstrap, None),
            KnowledgeBaseOperationKind::Update => (
                KnowledgeBaseStepKind::ApplyUpdate,
                Some(("packages/2026-11-09_demo", "2026-11-09_demo")),
            ),
        };
        KnowledgeBaseOperation {
            operation_id: derive_operation_id(PROJECT, lc_id, idempotency_key),
            project_id: PROJECT.to_string(),
            logical_codebase_id: lc_id.to_string(),
            kind,
            input: KnowledgeBaseOperationInput {
                idempotency_key: idempotency_key.to_string(),
                provider: "pi".to_string(),
                package_path: package.map(|(path, _)| path.to_string()),
                package_id: package.map(|(_, id)| id.to_string()),
            },
            status: KnowledgeBaseOperationStatus::Running,
            steps: vec![KnowledgeBaseStepRecord {
                step_id,
                status: KnowledgeBaseStepStatus::Pending,
                started_at: None,
                completed_at: None,
                output_summary: None,
            }],
            provider_selection: Some(provider_selection()),
            error: None,
            created_at: created_at.to_string(),
            started_at: Some(created_at.to_string()),
            completed_at: None,
        }
    }

    #[test]
    fn knowledge_base_operation_id_includes_logical_codebase() {
        let alpha = derive_operation_id(PROJECT, LC_ALPHA, "kb-key-0001");
        let beta = derive_operation_id(PROJECT, LC_BETA, "kb-key-0001");
        assert_ne!(
            alpha, beta,
            "同一 idempotency_key 在不同 LC 下必须派生出不同 operation_id"
        );
        // 同一 LC 与 key 重复派生保持稳定（幂等创建依赖这一点）。
        assert_eq!(alpha, derive_operation_id(PROJECT, LC_ALPHA, "kb-key-0001"));
    }

    #[test]
    fn knowledge_base_store_roundtrips_operation() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let paths = ProductAppPaths::new(temp.path());
        let store = KnowledgeBaseOperationStore::for_lc(paths, LC_ALPHA.to_string());
        let operation = new_operation(
            LC_ALPHA,
            "kb-key-0001",
            KnowledgeBaseOperationKind::Update,
            CREATED_AT,
        );

        let created = store
            .create(operation.clone())
            .expect("创建 operation 失败");
        assert_eq!(created, operation);

        let loaded = store
            .get(PROJECT, &operation.operation_id)
            .expect("读取 operation 失败");
        assert_eq!(loaded, operation);

        let latest = store.latest(PROJECT).expect("查询最新 operation 失败");
        assert_eq!(latest, Some(operation.clone()));

        // 重复 create 同一条 operation 幂等返回既有记录。
        let replayed = store
            .create(operation.clone())
            .expect("重复创建 operation 失败");
        assert_eq!(replayed, operation);
    }

    #[test]
    fn knowledge_base_store_isolates_operations_per_logical_codebase() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let paths = ProductAppPaths::new(temp.path());
        let store_alpha = KnowledgeBaseOperationStore::for_lc(paths.clone(), LC_ALPHA.to_string());
        let store_beta = KnowledgeBaseOperationStore::for_lc(paths, LC_BETA.to_string());

        let alpha_op = new_operation(
            LC_ALPHA,
            "kb-key-alpha",
            KnowledgeBaseOperationKind::Bootstrap,
            CREATED_AT,
        );
        let beta_op = new_operation(
            LC_BETA,
            "kb-key-beta",
            KnowledgeBaseOperationKind::Bootstrap,
            CREATED_AT,
        );
        store_alpha
            .create(alpha_op.clone())
            .expect("创建 LC-A operation 失败");
        store_beta
            .create(beta_op.clone())
            .expect("创建 LC-B operation 失败");

        // 每个 store 只能看到自己 LC 的最新 operation。
        let latest_alpha = store_alpha
            .latest(PROJECT)
            .expect("查询 LC-A 最新 operation 失败");
        assert_eq!(latest_alpha, Some(alpha_op.clone()));
        let latest_beta = store_beta
            .latest(PROJECT)
            .expect("查询 LC-B 最新 operation 失败");
        assert_eq!(latest_beta, Some(beta_op.clone()));

        // LC-A 的 store 按 LC-B 的 operation_id 查不到 B 的记录。
        assert!(matches!(
            store_alpha.get(PROJECT, &beta_op.operation_id),
            Err(ProductStoreError::NotFound { .. })
        ));
    }

    #[test]
    fn knowledge_base_store_lists_only_running_operations() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let paths = ProductAppPaths::new(temp.path());
        let store = KnowledgeBaseOperationStore::for_lc(paths, LC_ALPHA.to_string());

        let running = new_operation(
            LC_ALPHA,
            "kb-key-running",
            KnowledgeBaseOperationKind::Bootstrap,
            "2026-11-09T00:00:00Z",
        );
        let completed = new_operation(
            LC_ALPHA,
            "kb-key-completed",
            KnowledgeBaseOperationKind::Bootstrap,
            "2026-11-09T00:00:01Z",
        );
        store
            .create(running.clone())
            .expect("创建 running operation 失败");
        store
            .create(completed.clone())
            .expect("创建 completed operation 失败");
        store
            .finish_completed(PROJECT, &completed.operation_id, FINISHED_AT.to_string())
            .expect("完结 operation 失败");

        let running_operations = store
            .list_running(PROJECT)
            .expect("列出 running operation 失败");
        assert_eq!(running_operations, vec![running]);
    }

    #[test]
    fn knowledge_base_store_finishes_failed_and_cancels_running_operations() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let paths = ProductAppPaths::new(temp.path());
        let store = KnowledgeBaseOperationStore::for_lc(paths, LC_ALPHA.to_string());

        // 失败路径：给出失败步骤时同步置 Failed 并记录 error。
        let failing = new_operation(
            LC_ALPHA,
            "kb-key-failing",
            KnowledgeBaseOperationKind::Update,
            CREATED_AT,
        );
        store
            .create(failing.clone())
            .expect("创建 failing operation 失败");
        let error = AggregateInitializationErrorRecord {
            stage: "knowledge_base".to_string(),
            reason_code: "knowledge_base_manifest_invalid".to_string(),
            stderr_summary: None,
            retryable: false,
            action: "修正 manifest 后重试".to_string(),
            cause: None,
            provider: Some("pi".to_string()),
        };
        store
            .finish_failed(
                PROJECT,
                &failing.operation_id,
                Some(KnowledgeBaseStepKind::ApplyUpdate),
                error.clone(),
                FINISHED_AT.to_string(),
            )
            .expect("标记 operation 失败失败");
        let failed = store
            .get(PROJECT, &failing.operation_id)
            .expect("读取 failed operation 失败");
        assert_eq!(failed.status, KnowledgeBaseOperationStatus::Failed);
        assert_eq!(failed.error, Some(error));
        assert_eq!(failed.completed_at, Some(FINISHED_AT.to_string()));
        assert_eq!(failed.steps[0].status, KnowledgeBaseStepStatus::Failed);
        assert_eq!(
            failed.steps[0].completed_at,
            Some(FINISHED_AT.to_string()),
            "失败步骤必须记录完成时间"
        );

        // 取消路径：Running → Cancelled。
        let cancelling = new_operation(
            LC_ALPHA,
            "kb-key-cancelling",
            KnowledgeBaseOperationKind::Bootstrap,
            CREATED_AT,
        );
        store
            .create(cancelling.clone())
            .expect("创建 cancelling operation 失败");
        store
            .cancel(PROJECT, &cancelling.operation_id, FINISHED_AT.to_string())
            .expect("取消 operation 失败");
        let cancelled = store
            .get(PROJECT, &cancelling.operation_id)
            .expect("读取 cancelled operation 失败");
        assert_eq!(cancelled.status, KnowledgeBaseOperationStatus::Cancelled);
        assert_eq!(cancelled.completed_at, Some(FINISHED_AT.to_string()));

        // 终态后的 operation 不再进入 running 列表。
        let running_operations = store
            .list_running(PROJECT)
            .expect("列出 running operation 失败");
        assert!(running_operations.is_empty());
    }

    fn error_record() -> AggregateInitializationErrorRecord {
        AggregateInitializationErrorRecord {
            stage: "knowledge_base".to_string(),
            reason_code: "knowledge_base_write_scope_violation".to_string(),
            stderr_summary: Some("alpha/src/main.rs".to_string()),
            retryable: false,
            action: "检查越界改动".to_string(),
            cause: Some("knowledge_base_cancelled".to_string()),
            provider: Some("pi".to_string()),
        }
    }

    #[test]
    fn complete_step_closes_step_and_operation_atomically() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let store =
            KnowledgeBaseOperationStore::for_lc(ProductAppPaths::new(temp.path()), LC_ALPHA.into());
        let operation = new_operation(
            LC_ALPHA,
            "kb-key-0001",
            KnowledgeBaseOperationKind::Bootstrap,
            CREATED_AT,
        );
        store.create(operation.clone()).unwrap();

        store
            .complete_step(
                PROJECT,
                &operation.operation_id,
                KnowledgeBaseStepKind::Bootstrap,
                Some("turn ok".to_string()),
                FINISHED_AT.to_string(),
            )
            .unwrap();
        let stored = store.get(PROJECT, &operation.operation_id).unwrap();
        assert_eq!(stored.status, KnowledgeBaseOperationStatus::Completed);
        assert_eq!(stored.completed_at.as_deref(), Some(FINISHED_AT));
        assert_eq!(stored.steps[0].status, KnowledgeBaseStepStatus::Completed);
        assert_eq!(stored.steps[0].completed_at.as_deref(), Some(FINISHED_AT));
        assert_eq!(stored.steps[0].output_summary.as_deref(), Some("turn ok"));

        // 已终态不可再收口。
        assert!(matches!(
            store.complete_step(
                PROJECT,
                &operation.operation_id,
                KnowledgeBaseStepKind::Bootstrap,
                None,
                FINISHED_AT.to_string(),
            ),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }

    #[test]
    fn complete_step_with_unknown_step_leaves_operation_untouched() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let store =
            KnowledgeBaseOperationStore::for_lc(ProductAppPaths::new(temp.path()), LC_ALPHA.into());
        let operation = new_operation(
            LC_ALPHA,
            "kb-key-0001",
            KnowledgeBaseOperationKind::Bootstrap,
            CREATED_AT,
        );
        store.create(operation.clone()).unwrap();
        assert!(matches!(
            store.complete_step(
                PROJECT,
                &operation.operation_id,
                KnowledgeBaseStepKind::ApplyUpdate,
                None,
                FINISHED_AT.to_string(),
            ),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
        let stored = store.get(PROJECT, &operation.operation_id).unwrap();
        assert_eq!(stored.status, KnowledgeBaseOperationStatus::Running);
        assert_eq!(stored.steps[0].status, KnowledgeBaseStepStatus::Pending);
    }

    #[test]
    fn cancel_with_error_attaches_failure_fact_to_cancelled_operation() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let store =
            KnowledgeBaseOperationStore::for_lc(ProductAppPaths::new(temp.path()), LC_ALPHA.into());

        // Running → Cancelled + error。
        let running = new_operation(
            LC_ALPHA,
            "kb-key-0001",
            KnowledgeBaseOperationKind::Bootstrap,
            CREATED_AT,
        );
        store.create(running.clone()).unwrap();
        store
            .cancel_with_error(
                PROJECT,
                &running.operation_id,
                error_record(),
                FINISHED_AT.to_string(),
            )
            .unwrap();
        let stored = store.get(PROJECT, &running.operation_id).unwrap();
        assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);
        assert_eq!(stored.completed_at.as_deref(), Some(FINISHED_AT));
        assert_eq!(stored.error, Some(error_record()));
        // 已带 error 的 Cancelled 不可重复附加。
        assert!(matches!(
            store.cancel_with_error(
                PROJECT,
                &running.operation_id,
                error_record(),
                "later".to_string(),
            ),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));

        // 已被 cancel API 取消（无 error）：只补 error，保留原完成时间。
        let cancelled = new_operation(
            LC_ALPHA,
            "kb-key-0002",
            KnowledgeBaseOperationKind::Bootstrap,
            CREATED_AT,
        );
        store.create(cancelled.clone()).unwrap();
        store
            .cancel(PROJECT, &cancelled.operation_id, CREATED_AT.to_string())
            .unwrap();
        store
            .cancel_with_error(
                PROJECT,
                &cancelled.operation_id,
                error_record(),
                FINISHED_AT.to_string(),
            )
            .unwrap();
        let stored = store.get(PROJECT, &cancelled.operation_id).unwrap();
        assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);
        assert_eq!(stored.completed_at.as_deref(), Some(CREATED_AT));
        assert_eq!(stored.error, Some(error_record()));

        // 其它终态拒绝。
        let done = new_operation(
            LC_ALPHA,
            "kb-key-0003",
            KnowledgeBaseOperationKind::Bootstrap,
            CREATED_AT,
        );
        store.create(done.clone()).unwrap();
        store
            .finish_completed(PROJECT, &done.operation_id, FINISHED_AT.to_string())
            .unwrap();
        assert!(matches!(
            store.cancel_with_error(
                PROJECT,
                &done.operation_id,
                error_record(),
                FINISHED_AT.to_string(),
            ),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }

    #[test]
    fn begin_lock_is_exclusive_across_independent_handles_and_not_listed_as_operation() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let paths = ProductAppPaths::new(temp.path());
        let holder = KnowledgeBaseOperationStore::for_lc(paths.clone(), LC_ALPHA.into());
        let contender = KnowledgeBaseOperationStore::for_lc(paths, LC_ALPHA.into());

        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder_thread = std::thread::spawn(move || {
            holder
                .with_begin_lock(PROJECT, || {
                    held_tx.send(()).unwrap();
                    release_rx
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .ok();
                    Ok(())
                })
                .unwrap();
        });
        held_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();

        // 持锁期间，另一个独立 store 句柄（等价于另一进程的新 fd）进不了临界区。
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let contender_thread = std::thread::spawn(move || {
            contender
                .with_begin_lock(PROJECT, || {
                    entered_tx.send(()).unwrap();
                    Ok(())
                })
                .unwrap();
        });
        assert!(
            entered_rx
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "持锁期间竞争者不得进入临界区"
        );
        release_tx.send(()).unwrap();
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("释放后竞争者应进入临界区");
        holder_thread.join().unwrap();
        contender_thread.join().unwrap();

        // 锁文件不是 `.json`，不会被当作 operation 记录读取。
        let store =
            KnowledgeBaseOperationStore::for_lc(ProductAppPaths::new(temp.path()), LC_ALPHA.into());
        assert!(store.list_running(PROJECT).unwrap().is_empty());
        assert!(store.latest(PROJECT).unwrap().is_none());
    }

    #[test]
    fn transition_updates_serialize_on_the_operation_level_lock() {
        // R2：终态转换（cancel / cancel_with_error / finish_* / complete_step）
        // 共享 `<operation_id>.lock` 的跨进程排他锁：外部持锁期间转换必须
        // 等待，防止「普通取消用旧副本覆盖附加审计」的丢失更新交错。
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let paths = ProductAppPaths::new(temp.path());
        let store = KnowledgeBaseOperationStore::for_lc(paths.clone(), LC_ALPHA.into());
        let running = new_operation(
            LC_ALPHA,
            "kb-key-0001",
            KnowledgeBaseOperationKind::Bootstrap,
            CREATED_AT,
        );
        store.create(running.clone()).unwrap();
        let op_lock = store
            .operations_root(PROJECT)
            .unwrap()
            .join(format!("{}.lock", running.operation_id));

        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder_path = op_lock.clone();
        let holder = std::thread::spawn(move || {
            with_exact_exclusive_lock(&holder_path, || {
                held_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .ok();
                Ok(())
            })
            .unwrap();
        });
        held_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();

        // 外部持锁：cancel_with_error 必须阻塞在 operation 级锁上，不得进入
        // 读改写（否则它可能读到旧副本并在释放后覆盖他人写入的事实）。
        let contender_store = KnowledgeBaseOperationStore::for_lc(paths, LC_ALPHA.into());
        let contender_op = running.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let contender = std::thread::spawn(move || {
            let result = contender_store.cancel_with_error(
                PROJECT,
                &contender_op.operation_id,
                error_record(),
                FINISHED_AT.to_string(),
            );
            entered_tx.send(result).unwrap();
        });
        assert!(
            entered_rx
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "外部持锁期间终态转换不得进入读改写"
        );
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .expect("释放后转换应完成");
        contender.join().unwrap();

        let stored = store.get(PROJECT, &running.operation_id).unwrap();
        assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);
        assert_eq!(stored.error, Some(error_record()));
    }
}
