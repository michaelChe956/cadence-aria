//! 知识库 operation 的 per-LC durable store：每条 operation 一个
//! `<operation_id>.json`，落在该 LC scope 的 `knowledge-base-operations/`
//! 目录下（复用 `lc_scope_root` 的 per-LC / legacy 别名布局）。

use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
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
    fn update(
        &self,
        project_id: &str,
        operation_id: &str,
        update: impl FnOnce(&mut KnowledgeBaseOperation) -> Result<(), ProductStoreError>,
    ) -> Result<KnowledgeBaseOperation, ProductStoreError> {
        let path = self.operation_path(project_id, operation_id)?;
        if !path.exists() {
            return Err(not_found(operation_id));
        }
        let mut operation: KnowledgeBaseOperation = read_json(&path)?;
        self.ensure_record_consistency(&operation)?;
        update(&mut operation)?;
        write_json(&path, &operation)?;
        Ok(operation)
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
}
