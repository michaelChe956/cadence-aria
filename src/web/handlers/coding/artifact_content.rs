use super::*;

pub(crate) async fn coding_attempt_artifact_content(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptArtifactRoutePath>,
) -> ApiResult<Json<ArtifactContentResponse>> {
    let artifact_id = path.artifact_id;
    validate_relative_id(&artifact_id)
        .map_err(|_| ApiError::validation("invalid_artifact_id", "invalid artifact id"))?;
    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    let artifact_path = coding_store
        .attempt_test_output_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &artifact_id,
        )
        .map_err(product_store_api_error)?;
    if !artifact_path.is_file() {
        return Err(ApiError::runtime(
            "artifact_not_found",
            "coding attempt artifact not found",
            json!({}),
        ));
    }
    let content = fs::read_to_string(&artifact_path).map_err(|error| {
        ApiError::runtime(
            "artifact_read_failed",
            "coding attempt artifact could not be read",
            json!({"error": error.to_string()}),
        )
    })?;

    Ok(Json(ArtifactContentResponse {
        artifact_ref: artifact_id,
        artifact_kind: "coding_attempt_artifact".to_string(),
        producer_node: None,
        path: artifact_path.to_string_lossy().to_string(),
        content_type: "text/plain".to_string(),
        content,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::{LogicalCodebaseManifest, LogicalCodebaseStore};
    use crate::product::models::{
        RepositoryRecord, WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind,
        WorkItemPlanStatus, WorkItemStatus,
    };
    use uuid::Uuid;

    #[test]
    fn resolve_work_item_repository_manifest_with_bad_target_is_fail_closed() {
        // 有 manifest、work_item target 指向不存在成员 → blocker，不回退物理仓库。
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        write_physical_repository_fixture(&paths, root.path());
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    root.path().join("aggregate-root"),
                    Vec::new(),
                ),
            )
            .unwrap();
        let work_item =
            lifecycle_work_item_with_target_fixture("00000000-0000-0000-0000-000000000000");

        let result = resolve_work_item_repository(&paths, "project_0001", &work_item);

        assert!(result.is_err());
    }

    fn write_physical_repository_fixture(paths: &ProductAppPaths, root: &std::path::Path) {
        crate::product::json_store::write_json(
            &paths.project_root("project_0001").join("repos.json"),
            &[RepositoryRecord {
                id: "repository_0001".to_string(),
                project_id: "project_0001".to_string(),
                name: "物理仓库".to_string(),
                path: root.join("repository_0001"),
                repo_hash: "sha256:repository".to_string(),
                runtime_root: root.join("repository_0001/.aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                logical_repository_id: None,
                primary_checkout_id: None,
                identity_schema_version: 1,
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            }],
        )
        .unwrap();
    }

    fn lifecycle_work_item_with_target_fixture(
        target_repository_id: &str,
    ) -> LifecycleWorkItemRecord {
        LifecycleWorkItemRecord {
            id: "work_item_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            target_repository_id: Some(crate::product::logical_codebase::LogicalRepositoryId(
                Uuid::parse_str(target_repository_id).unwrap(),
            )),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "工作项".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            kind: WorkItemKind::default(),
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: WorkItemContextBudget::default(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
        }
    }
}
