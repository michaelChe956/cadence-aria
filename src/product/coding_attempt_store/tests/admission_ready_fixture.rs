//! admission 就绪夹具：project + 逻辑代码库权威（manifest/member/checkout/
//! selection/policy/repos.json）+ 与权威一致的冻结 `target_snapshot`。
//!
//! REQ-ROUTE-01 两态化（去单仓入口）后 admission 的唯一凭证是「LC 权威 +
//! 冻结快照」：无归属 attempt 一律 `FailClosed(TargetMissing)`。
//! 父模块的 `setup()` 不建任何归属，只适合不触达 admission/routing 的用例；
//! 凡以「准入后行为」为不变量的用例必须用本夹具。

use tempfile::TempDir;

use super::*;

pub(super) fn setup_admission_ready() -> (TempDir, CodingAttemptStore, CodingExecutionAttempt) {
    let tmp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let (logical_id, checkout_id) =
        crate::product::logical_codebase::logical_test_fixture::seed_logical_authority(
            &paths,
            PROJECT_ID,
            ISSUE_ID,
            tmp.path(),
        );
    let target_snapshot = crate::product::logical_codebase::logical_test_fixture::snapshot_for(
        &paths,
        PROJECT_ID,
        logical_id,
        checkout_id,
    );
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: Some(target_snapshot),
            max_auto_rework: 2,
        })
        .unwrap();
    (tmp, store, attempt)
}
