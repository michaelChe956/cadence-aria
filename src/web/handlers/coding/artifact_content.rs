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

    // ---- C4 Task 2：resolver 接入所有 LC 读取与目标解析入口 ----

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::id::repo_hash_for_path;
    use crate::product::issue_store::CreateProductIssueInput;
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
    };
    use crate::product::logical_codebase::issue_selection::IssueCodebaseSelectionStore;
    use crate::product::logical_codebase::policy::{
        AggregatePolicyArtifact, AggregatePolicyArtifactStore,
    };
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, MemberStatus,
        RepositoryCheckoutRecord, RepositoryType,
    };
    use crate::product::logical_codebase::{
        IssueCodebaseSelection, LogicalCodebaseCreateInput, LogicalRepositoryId,
        PlanningContextSetResolver, RepositoryCheckoutId,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use std::collections::BTreeMap;

    fn git(cwd: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .expect("git must start");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git_repository_with_commit(path: &std::path::Path) {
        std::fs::create_dir_all(path).unwrap();
        git(path, &["init", "-b", "main"]);
        git(path, &["config", "user.email", "task2@test.local"]);
        git(path, &["config", "user.name", "Task2 Test"]);
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", "init"]);
    }

    fn aria_inventory(root: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
        let mut inventory = BTreeMap::new();
        let mut stack = vec![root.join(".aria")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    inventory.insert(
                        relative.to_string_lossy().into_owned(),
                        std::fs::read(&path).unwrap_or_default(),
                    );
                }
            }
        }
        inventory
    }

    /// 双 LC 夹具：同名成员 alias "repo"、不同 authority root 与 source 身份。
    fn two_lc_fixture() -> (
        tempfile::TempDir,
        ProductAppPaths,
        String,
        String, /* lc_a id */
        String, /* lc_b id */
        LogicalRepositoryId,
        LogicalRepositoryId,
        String, /* issue_a id */
    ) {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let project_id = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "task2-project".to_string(),
                description: None,
            })
            .unwrap()
            .id;

        let alpha_root = temp.path().join("alpha-root");
        let beta_root = temp.path().join("beta-root");
        std::fs::create_dir_all(&alpha_root).unwrap();
        std::fs::create_dir_all(&beta_root).unwrap();
        let alpha_repo = alpha_root.join("repo");
        let beta_repo = beta_root.join("repo");
        init_git_repository_with_commit(&alpha_repo);
        init_git_repository_with_commit(&beta_repo);
        let alpha_canonical = std::fs::canonicalize(&alpha_repo).unwrap();
        let beta_canonical = std::fs::canonicalize(&beta_repo).unwrap();
        let alpha_source =
            crate::product::repository_store::resolve_repository_source(&alpha_canonical).unwrap();
        let beta_source =
            crate::product::repository_store::resolve_repository_source(&beta_canonical).unwrap();

        let store = LogicalCodebaseStore::new(paths.clone());
        let lc_a = store
            .create(
                &project_id,
                LogicalCodebaseCreateInput {
                    name: "alpha".to_string(),
                    aggregate_root: alpha_root.clone(),
                },
            )
            .unwrap()
            .id;
        let lc_b = store
            .create(
                &project_id,
                LogicalCodebaseCreateInput {
                    name: "beta".to_string(),
                    aggregate_root: beta_root.clone(),
                },
            )
            .unwrap()
            .id;

        let member_a = LogicalRepositoryId(uuid::Uuid::new_v4());
        let member_b = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_a = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let checkout_b = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let now = "2026-09-29T00:00:00Z".to_string();

        for (lc_id, member_id, checkout_id, canonical, source, root) in [
            (
                &lc_a,
                member_a,
                checkout_a,
                alpha_canonical.clone(),
                &alpha_source,
                alpha_root.clone(),
            ),
            (
                &lc_b,
                member_b,
                checkout_b,
                beta_canonical.clone(),
                &beta_source,
                beta_root.clone(),
            ),
        ] {
            let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), lc_id);
            let mut manifest =
                LogicalCodebaseManifest::new(&project_id, root.clone(), vec![member_id]);
            manifest.member_ids = vec![member_id];
            lc_store.save_manifest(&project_id, &manifest).unwrap();
            lc_store
                .save_member(
                    &project_id,
                    &CodebaseMemberRecord {
                        logical_repository_id: member_id,
                        physical_repository_id: format!("physical_{}", member_id.0),
                        alias: "repo".to_string(),
                        role: "member".to_string(),
                        ordinal: 1,
                        source_identity: source.clone(),
                        repo_type: RepositoryType::Unknown,
                        tech_stack: Vec::new(),
                        owner: None,
                        tags: Vec::new(),
                        default_ref: None,
                        checkout_ids: vec![checkout_id],
                        status: MemberStatus::Active,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .unwrap();
            lc_store
                .save_checkout(
                    &project_id,
                    &RepositoryCheckoutRecord {
                        checkout_id,
                        logical_repository_id: member_id,
                        physical_repository_id: format!("physical_{}", member_id.0),
                        kind: CheckoutKind::Main,
                        canonical_path: canonical.clone(),
                        checkout_path_hash: repo_hash_for_path(
                            canonical.to_string_lossy().as_ref(),
                        ),
                        git_dir_identity: source.git_dir_identity(),
                        revision: None,
                        availability: CheckoutAvailability::Available,
                        observed_at: now.clone(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .unwrap();
            let policy_stamp = if lc_id == &lc_a {
                "2026-09-29T00:00:00Z"
            } else {
                "2026-09-29T01:00:00Z"
            };
            AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id)
                .save(
                    &project_id,
                    &AggregatePolicyArtifact::bootstrap(
                        &project_id,
                        &manifest.logical_codebase_id.to_string(),
                        policy_stamp.to_string(),
                    ),
                )
                .unwrap();
            let mut index = AggregateIndexRecord::building(
                format!("aggregate_index_{lc_id}"),
                project_id.clone(),
                1,
                Vec::new(),
                now.clone(),
            );
            index.status = AggregateIndexStatus::Active;
            AggregateIndexStore::for_lc(paths.clone(), lc_id)
                .create(&project_id, index)
                .unwrap();
        }

        let issue_store = crate::product::issue_store::IssueStore::new(paths.clone());
        let issue_a = issue_store
            .create(CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(lc_a.clone()),
                title: "alpha issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap()
            .id;
        let issue_b = issue_store
            .create(CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(lc_b.clone()),
                title: "beta issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap()
            .id;
        IssueCodebaseSelectionStore::for_lc(paths.clone(), &lc_a)
            .save(
                &IssueCodebaseSelection::all_members(&project_id, &issue_a, None)
                    .for_logical_codebase(&lc_a),
            )
            .unwrap();
        IssueCodebaseSelectionStore::for_lc(paths.clone(), &lc_b)
            .save(
                &IssueCodebaseSelection::all_members(&project_id, &issue_b, None)
                    .for_logical_codebase(&lc_b),
            )
            .unwrap();

        (
            temp, paths, project_id, lc_a, lc_b, member_a, member_b, issue_a,
        )
    }

    fn work_item_for(
        project_id: &str,
        issue_id: &str,
        target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ) -> LifecycleWorkItemRecord {
        LifecycleWorkItemRecord {
            id: format!("work_item_{issue_id}"),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: "repository_0001".to_string(),
            target_repository_id: target,
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
            created_at: "2026-09-29T00:00:00Z".to_string(),
            updated_at: "2026-09-29T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn planning_and_coding_resolution_use_same_lc_authority_root() {
        let (temp, paths, project_id, lc_a, lc_b, member_a, member_b, issue_a) = two_lc_fixture();

        // 规划侧：PlanningContextSetResolver 只读 issue_a 归属 LC 的成员集合。
        let planning = PlanningContextSetResolver::new(paths.clone())
            .resolve(&project_id, &issue_a)
            .unwrap();
        assert_eq!(planning.set.len(), 1);
        assert_eq!(planning.set[0].member_id, member_a);
        assert_eq!(planning.set[0].alias, "repo");
        assert_eq!(planning.set[0].root_relative_path, "repo");
        assert_ne!(planning.set[0].member_id, member_b);

        // coding 侧：work item target 解析到同一 LC 成员的物理 checkout。
        let repository = resolve_work_item_repository(
            &paths,
            &project_id,
            &work_item_for(&project_id, &issue_a, Some(member_a)),
        )
        .unwrap();
        assert_eq!(repository.logical_repository_id, Some(member_a));
        assert_eq!(
            repository.path,
            std::fs::canonicalize(temp.path().join("alpha-root").join("repo")).unwrap()
        );

        // policy digest / index id 来自同一 LC 子树（不读另一 LC）。
        let policy_a = AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc_a)
            .get(&project_id)
            .unwrap()
            .expect("policy under lc_a subtree");
        let policy_b = AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc_b)
            .get(&project_id)
            .unwrap()
            .expect("policy under lc_b subtree");
        // bootstrap policy digest 是 canonical 文本哈希（跨 LC 相同）；可区分的
        // 身份是 policy_id / logical_codebase_id（各 LC 的 manifest id 不同）。
        assert_ne!(policy_a.policy_id, policy_b.policy_id);
        assert_ne!(policy_a.logical_codebase_id, policy_b.logical_codebase_id);
        let index_a = AggregateIndexStore::for_lc(paths.clone(), &lc_a)
            .active(&project_id)
            .unwrap()
            .expect("active index under lc_a subtree");
        let index_b = AggregateIndexStore::for_lc(paths.clone(), &lc_b)
            .active(&project_id)
            .unwrap()
            .expect("active index under lc_b subtree");
        assert_ne!(index_a.aggregate_index_id, index_b.aggregate_index_id);

        // issue_b 对称解析到 beta：证明入口不会读另一 LC。
        let planning_b = PlanningContextSetResolver::new(paths.clone())
            .resolve(&project_id, &format!("{issue_a}b"))
            .map(|_| ());
        assert!(planning_b.is_err(), "unknown issue must fail");
        let issue_store = crate::product::issue_store::IssueStore::new(paths.clone());
        let issues = issue_store.list(&project_id).unwrap();
        let issue_b = issues
            .iter()
            .find(|issue| issue.logical_codebase_id.as_deref() == Some(lc_b.as_str()))
            .expect("beta issue exists")
            .id
            .clone();
        let planning_beta = PlanningContextSetResolver::new(paths.clone())
            .resolve(&project_id, &issue_b)
            .unwrap();
        assert_eq!(planning_beta.set[0].member_id, member_b);
        let repository_b = resolve_work_item_repository(
            &paths,
            &project_id,
            &work_item_for(&project_id, &issue_b, Some(member_b)),
        )
        .unwrap();
        assert_eq!(repository_b.logical_repository_id, Some(member_b));
        assert_eq!(
            repository_b.path,
            std::fs::canonicalize(temp.path().join("beta-root").join("repo")).unwrap()
        );
    }

    #[test]
    fn single_repo_request_for_logical_issue_fails_before_target_lookup() {
        let (temp, paths, project_id, _lc_a, _lc_b, _member_a, _member_b, issue_a) =
            two_lc_fixture();

        let before = aria_inventory(temp.path());
        let repos_json_before =
            std::fs::read(paths.project_root(&project_id).join("repos.json")).unwrap_or_default();

        // 单仓形状（无 target_repository_id）命中 LC 归属 issue：显式 SingleRepo
        // 请求必须在 target lookup 之前 fail-closed kind_mismatch。
        let result = resolve_work_item_repository(
            &paths,
            &project_id,
            &work_item_for(&project_id, &issue_a, None),
        );
        let error = result.expect_err("single-repo request on logical issue must fail");
        assert_eq!(error.code, "repository_routing_kind_mismatch");

        // 零写入：物理 repos.json 与 `.aria` durable inventory 字节不变。
        let after = aria_inventory(temp.path());
        assert_eq!(before, after);
        let repos_json_after =
            std::fs::read(paths.project_root(&project_id).join("repos.json")).unwrap_or_default();
        assert_eq!(repos_json_before, repos_json_after);
    }
}
