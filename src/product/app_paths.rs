use std::path::{Path, PathBuf};

/// 与 `tool-policy-run-audit/`、`work-item-split-runs/` 并列的 per-session 暂存分区
/// （REQ-PRB-04：位于状态根下，不在仓库工作区或 provider cwd 内）。
pub const RUN_SCRATCH_PARTITION: &str = "run-scratch";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAppPaths {
    root: PathBuf,
}

impl ProductAppPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn product_data_schema_path(&self) -> PathBuf {
        self.root.join("schema.json")
    }

    pub fn projects_root(&self) -> PathBuf {
        self.root.join("projects")
    }

    pub fn run_scratch_root(&self) -> PathBuf {
        self.root.join(RUN_SCRATCH_PARTITION)
    }

    /// 纯路径计算，不做 id 校验（同层既有方法口径）。
    pub fn run_scratch_dir(&self, workspace_session_id: &str) -> PathBuf {
        self.run_scratch_root().join(workspace_session_id)
    }

    pub fn state_root(&self) -> PathBuf {
        self.root.join("state")
    }

    /// provider capability 记录的**机器级唯一作用域**（REQ-PCR-07）：
    /// 能力是本机 provider CLI 的事实 ⇒ 整机一份记录，与项目/逻辑代码库无关。
    /// 记录文件 `capabilities.json` 与探针证据 `evidence/` 同落此根下。
    pub fn provider_capabilities_root(&self) -> PathBuf {
        self.root.join("provider-capabilities")
    }

    pub fn last_project_path(&self) -> PathBuf {
        self.state_root().join("last-project.json")
    }

    pub fn project_root(&self, project_id: &str) -> PathBuf {
        self.projects_root().join(project_id)
    }

    pub fn issue_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.project_root(project_id).join("issues").join(issue_id)
    }

    pub fn issue_lifecycle_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.issue_root(project_id, issue_id)
    }

    /// 悬空引用销账留痕台账（状态根下单一全局 JSONL；目录由本能力新建）。
    pub fn dangling_reference_ledger_path(&self) -> PathBuf {
        self.state_root()
            .join("entity-deletions")
            .join("dangling-reference")
            .join("cleanup.jsonl")
    }

    pub fn project_provider_defaults_path(&self, project_id: &str) -> PathBuf {
        self.project_root(project_id).join("provider-defaults.json")
    }

    pub fn repository_initializations_root(&self, project_id: &str) -> PathBuf {
        self.project_root(project_id)
            .join("repository-initializations")
    }

    /// Legacy default logical-codebase location retained as a compatibility alias.
    pub fn logical_codebase_root(&self, project_id: &str) -> PathBuf {
        self.project_root(project_id).join("logical-codebase")
    }

    pub fn logical_codebases_root(&self, project_id: &str) -> PathBuf {
        self.project_root(project_id).join("logical-codebases")
    }

    pub fn logical_codebase_record_root(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> PathBuf {
        self.logical_codebases_root(project_id)
            .join(logical_codebase_id)
    }

    pub fn logical_codebase_migration_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebases_root(project_id)
            .join(".legacy-logical-codebase-migration.lock")
    }

    pub fn registration_batches_root(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join("registration-batches")
    }

    pub fn registration_preflights_root(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id).join("preflights")
    }

    pub fn registration_batches_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join(".registration-batches.lock")
    }

    pub fn logical_codebase_manifest_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join(".manifest-registration.lock")
    }

    pub fn identity_migration_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join(".identity-migration.lock")
    }

    pub fn aggregate_indexes_root(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join("aggregate-indexes")
    }

    pub fn aggregate_index_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join(".aggregate-index.lock")
    }

    pub fn aggregate_policy_artifact_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join("aggregate-policy.json")
    }

    pub fn aggregate_initializations_root(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join("aggregate-initializations")
    }

    /// v1.3/Task 1.5：root recipe receipt 的 legacy 别名 scope 常量
    ///（per-LC 布局由 `RootRecipeReceiptStore` 的 scope root 决定）。
    pub fn aggregate_recipe_receipts_root(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join("aggregate-recipe-receipts")
    }

    pub fn aggregate_recipe_receipt_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join(".aggregate-recipe-receipt.lock")
    }

    /// v1.3/REQ-REG-14：per-LC trust durable facts 的 legacy 别名 scope
    /// 常量（per-LC 布局由 `ProviderTrustStore` 的 `lc_scope_root` 决定）。
    pub fn provider_trust_root(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join("provider-trust")
    }

    pub fn provider_trust_registrations_root(&self, project_id: &str) -> PathBuf {
        self.provider_trust_root(project_id).join("registrations")
    }

    pub fn provider_trust_waiting_root(&self, project_id: &str) -> PathBuf {
        self.provider_trust_root(project_id).join("waiting")
    }

    pub fn provider_trust_audit_root(&self, project_id: &str) -> PathBuf {
        self.provider_trust_root(project_id).join("audit")
    }

    pub fn provider_trust_lock_path(&self, project_id: &str) -> PathBuf {
        self.logical_codebase_root(project_id)
            .join(".provider-trust.lock")
    }

    pub fn codebase_selection_path(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.issue_root(project_id, issue_id)
            .join("codebase-selection.json")
    }

    /// v1.3：逻辑代码库归属的 issue selection 键含 lc_id，落在 LC 子树。
    pub fn lc_codebase_selection_path(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        issue_id: &str,
    ) -> PathBuf {
        self.logical_codebase_record_root(project_id, logical_codebase_id)
            .join("selections")
            .join(issue_id)
            .join("codebase-selection.json")
    }

    pub fn planning_context_snapshot_path(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.issue_root(project_id, issue_id)
            .join("planning-context-snapshot.json")
    }

    /// v1.3/块2 片7A：provider / codegraph 安装能力的 durable 分区
    ///（异步作业 JSON 与 24h 版本缓存；状态根下全局唯一，不随项目走）。
    pub fn provider_installation_root(&self) -> PathBuf {
        self.state_root().join("provider-installation")
    }

    pub fn provider_installation_job_path(&self, job_id: &str) -> PathBuf {
        self.provider_installation_root()
            .join("jobs")
            .join(format!("{job_id}.json"))
    }

    pub fn provider_version_cache_path(&self) -> PathBuf {
        self.provider_installation_root().join("version-cache.json")
    }
}

#[cfg(test)]
mod tests {
    use super::ProductAppPaths;

    #[test]
    fn logical_codebase_paths_are_project_scoped() {
        let paths = ProductAppPaths::new("/tmp/aria");
        assert_eq!(
            paths.logical_codebase_root("project_0001"),
            std::path::PathBuf::from("/tmp/aria/projects/project_0001/logical-codebase")
        );
        assert_eq!(
            paths.logical_codebases_root("project_0001"),
            std::path::PathBuf::from("/tmp/aria/projects/project_0001/logical-codebases")
        );
        assert_eq!(
            paths.logical_codebase_record_root("project_0001", "logical_codebase_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebases/logical_codebase_0001"
            )
        );
        assert_eq!(
            paths.registration_batches_root("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/registration-batches"
            )
        );
        assert_eq!(
            paths.registration_preflights_root("project_0001"),
            std::path::PathBuf::from("/tmp/aria/projects/project_0001/logical-codebase/preflights")
        );
        assert_eq!(
            paths.registration_batches_lock_path("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/.registration-batches.lock"
            )
        );
        assert_eq!(
            paths.identity_migration_lock_path("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/.identity-migration.lock"
            )
        );
        assert_eq!(
            paths.provider_trust_root("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/provider-trust"
            )
        );
        assert_eq!(
            paths.provider_trust_registrations_root("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/provider-trust/registrations"
            )
        );
        assert_eq!(
            paths.provider_trust_waiting_root("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/provider-trust/waiting"
            )
        );
        assert_eq!(
            paths.aggregate_recipe_receipts_root("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/aggregate-recipe-receipts"
            )
        );
        assert_eq!(
            paths.aggregate_recipe_receipt_lock_path("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/.aggregate-recipe-receipt.lock"
            )
        );
        assert_eq!(
            paths.provider_trust_audit_root("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/provider-trust/audit"
            )
        );
        assert_eq!(
            paths.provider_trust_lock_path("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/.provider-trust.lock"
            )
        );
    }

    /// 块2 片7A：安装作业/版本缓存落在状态根下的全局分区。
    #[test]
    fn provider_installation_paths_are_state_scoped() {
        let paths = ProductAppPaths::new("/tmp/aria");
        assert_eq!(
            paths.provider_installation_root(),
            std::path::PathBuf::from("/tmp/aria/state/provider-installation")
        );
        assert_eq!(
            paths.provider_installation_job_path("job-0001"),
            std::path::PathBuf::from("/tmp/aria/state/provider-installation/jobs/job-0001.json")
        );
        assert_eq!(
            paths.provider_version_cache_path(),
            std::path::PathBuf::from("/tmp/aria/state/provider-installation/version-cache.json")
        );
    }

    /// REQ-PCR-07：provider capability 记录落**机器级**根（不按项目/逻辑代码库
    /// 分叉）。
    #[test]
    fn provider_capability_paths_are_machine_scoped() {
        let paths = ProductAppPaths::new("/tmp/aria");
        assert_eq!(
            paths.provider_capabilities_root(),
            std::path::PathBuf::from("/tmp/aria/provider-capabilities")
        );
    }
}
