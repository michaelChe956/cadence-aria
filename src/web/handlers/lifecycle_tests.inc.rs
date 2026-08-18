// lifecycle handler 测试（T4 分组视图 + T7 delivery_summary）：拆分到独立文件，
// 经 lifecycle.rs 的 `include!` 引入（large_file_guard 1200 行红线）。共享 `mod tests`
// 作用域内 `use super::*` 的导入。

    use crate::product::coding_models::{RemoteKind, ReviewRequest, ReviewRequestKind, ReviewRequestOwnerKind};
    use crate::product::issue_store::CreateProductIssueInput;
    use crate::product::lifecycle_store::CreateWorkItemInput;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
        IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
        LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{RepositoryRecord, WorkItemPlanLineage};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::work_item_revision_store::WorkItemRevisionStore;
    use crate::web::app::build_web_router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::TempDir;
    use tower::ServiceExt;
    use uuid::Uuid;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const REPOSITORY_ID: &str = "repo-1";

    /// 建 issue + 3 个 work item：wi-a/wi-b 带不同 target_repository_id（有 logical codebase
    /// 权威记录支撑），wi-c 无 target。
    fn seed_issue_and_work_items(lifecycle: &LifecycleStore, paths: &ProductAppPaths) {
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "lifecycle test project".to_string(),
                description: None,
                multi_repo: false,
            })
            .unwrap();
        let issue = IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some(REPOSITORY_ID.to_string()),
                title: "分组视图测试".to_string(),
                description: None,
                change_id: None,
            })
            .unwrap();
        assert_eq!(issue.id, ISSUE_ID);

        let logical_a = LogicalRepositoryId(Uuid::new_v4());
        let logical_b = LogicalRepositoryId(Uuid::new_v4());
        seed_logical_codebase(
            paths,
            &[(logical_a, "checkout-wi-a"), (logical_b, "checkout-wi-b")],
        );

        for (index, (id, target)) in [
            ("wi-a", Some(logical_a)),
            ("wi-b", Some(logical_b)),
            ("wi-c", None),
        ]
        .into_iter()
        .enumerate()
        {
            let work_item = lifecycle
                .create_work_item(CreateWorkItemInput {
                    id: Some(id.to_string()),
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    repository_id: REPOSITORY_ID.to_string(),
                    title: format!("工作项 {index}"),
                    kind: WorkItemKind::Backend,
                    plan_status: WorkItemPlanStatus::Confirmed,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(work_item.id, id);
            set_target_repository_id(lifecycle, id, target);
        }
    }

    /// 播种 logical codebase 权威记录（manifest + selection + member + checkout + repos.json
    /// 兼容投影），使 `resolve_logical_repository_strict` / `PlanningContextSetResolver` 都能解析。
    /// 成员 alias 与 repository.name 统一为 `REPOSITORY_ID`，保持分组视图 alias 断言不变。
    fn seed_logical_codebase(paths: &ProductAppPaths, members: &[(LogicalRepositoryId, &str)]) {
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            paths.root().join("aggregate-root"),
            members.iter().map(|(id, _)| *id).collect(),
        );
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(PROJECT_ID, &manifest)
            .unwrap();
        let now = "2026-08-13T00:00:00Z".to_string();
        let mut repositories = Vec::new();
        for (index, (logical_id, checkout_name)) in members.iter().enumerate() {
            let physical_repository_id = format!("physical-{checkout_name}");
            let checkout_path = paths.root().join(checkout_name);
            let source_identity = RepositorySourceIdentity::from_git_parts(
                &checkout_path,
                checkout_path.join(".git"),
                None,
            );
            let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
            LogicalCodebaseStore::new(paths.clone())
                .save_member(
                    PROJECT_ID,
                    &CodebaseMemberRecord {
                        logical_repository_id: *logical_id,
                        physical_repository_id: physical_repository_id.clone(),
                        alias: REPOSITORY_ID.to_string(),
                        role: "repository".to_string(),
                        ordinal: (index + 1) as u32,
                        source_identity: source_identity.clone(),
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
            LogicalCodebaseStore::new(paths.clone())
                .save_checkout(
                    PROJECT_ID,
                    &RepositoryCheckoutRecord {
                        checkout_id,
                        logical_repository_id: *logical_id,
                        physical_repository_id: physical_repository_id.clone(),
                        kind: CheckoutKind::Main,
                        canonical_path: checkout_path.clone(),
                        checkout_path_hash: format!("sha256:{checkout_name}"),
                        git_dir_identity: source_identity.git_dir_identity(),
                        revision: Some("abcdef".to_string()),
                        availability: CheckoutAvailability::Available,
                        observed_at: now.clone(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .unwrap();
            repositories.push(RepositoryRecord {
                id: physical_repository_id,
                project_id: PROJECT_ID.to_string(),
                name: REPOSITORY_ID.to_string(),
                path: checkout_path,
                repo_hash: format!("sha256:{checkout_name}"),
                runtime_root: paths.root().join(checkout_name).join(".aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: now.clone(),
                logical_repository_id: Some(*logical_id),
                primary_checkout_id: Some(checkout_id),
                identity_schema_version: 1,
                updated_at: now.clone(),
            });
        }
        crate::product::json_store::write_json(
            &paths.project_root(PROJECT_ID).join("repos.json"),
            &repositories,
        )
        .unwrap();
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                PROJECT_ID,
                ISSUE_ID,
                members.iter().map(|(id, _)| *id).collect(),
                Vec::new(),
                Vec::new(),
                None,
            ))
            .unwrap();
    }

    fn seed_completed_attempt(
        store: &CodingAttemptStore,
        work_item_id: &str,
        branch_name: &str,
        commit_sha: &str,
    ) -> CodingExecutionAttempt {
        let created = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: work_item_id.to_string(),
                base_branch: "main".to_string(),
                branch_name: branch_name.to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: Some(ProviderName::Fake),
                    review_rounds: 1,
                    permission_modes: WorkspaceRolePermissionModes::default(),
                },
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .unwrap();
        let attempt = CodingExecutionAttempt {
            status: CodingAttemptStatus::Completed,
            head_commit: Some(commit_sha.to_string()),
            ..created
        };
        store.write_coding_attempt_for_test(&attempt).unwrap();
        attempt
    }

    fn seed_review_request(
        store: &CodingAttemptStore,
        attempt: &CodingExecutionAttempt,
        push_status: PushStatus,
        push_error: Option<String>,
    ) {
        let request = ReviewRequest {
            id: format!("review_request_{}", Uuid::new_v4().simple()),
            attempt_id: attempt.id.clone(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind: RemoteKind::GenericGit,
            remote: "origin".to_string(),
            base_branch: "main".to_string(),
            branch_name: attempt.branch_name.clone(),
            commit_sha: attempt.head_commit.clone().unwrap_or_default(),
            push_status,
            external_url: None,
            manual_instructions: Vec::new(),
            push_error,
            owner_kind: ReviewRequestOwnerKind::Attempt,
            pointer_publication_id: None,
            revoked: false,
            created_at: "2026-08-13T00:00:00Z".to_string(),
            updated_at: "2026-08-13T00:00:00Z".to_string(),
        };
        store.save_review_request(attempt, &request).unwrap();
    }

    /// create_work_item 恒置 target_repository_id=None，测试直接改写落盘 JSON。
    fn set_target_repository_id(
        lifecycle: &LifecycleStore,
        work_item_id: &str,
        target: Option<LogicalRepositoryId>,
    ) {
        let path = lifecycle
            .work_items_root(PROJECT_ID, ISSUE_ID)
            .join(format!("{work_item_id}.json"));
        let mut record: LifecycleWorkItemRecord =
            crate::product::json_store::read_json(&path).unwrap();
        record.target_repository_id = target;
        crate::product::json_store::write_json(&path, &record).unwrap();
    }

    fn build_test_router(root: &std::path::Path) -> axum::Router {
        build_web_router(WebAppState::new(
            root.to_path_buf(),
            WebRuntime::new_fake(root.to_path_buf()),
        ))
    }

    async fn get_issue_lifecycle(app: &axum::Router) -> axum::http::Response<Body> {
        let uri = format!("/api/issues/{ISSUE_ID}/lifecycle?project_id={PROJECT_ID}");
        app.clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn grouped_view_returns_work_item_repository_groups_with_dto_shape() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        seed_issue_and_work_items(&lifecycle, &paths);
        let app = build_test_router(temp.path());

        let response = get_issue_lifecycle(&app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // 扁平 work_items 与分组视图同源（都来自已持久化记录）。
        let flat = value["work_items"].as_array().expect("work_items");
        assert_eq!(flat.len(), 3);
        let groups = value["work_item_repository_groups"]
            .as_array()
            .expect("work_item_repository_groups");
        assert_eq!(groups.len(), 3, "target A / target B / 未指定仓库 三组");

        // 分组 DTO 形状：target_repository_id / alias / status / compatibility_projection / items。
        for group in groups {
            let obj = group.as_object().expect("group object");
            for field in [
                "target_repository_id",
                "alias",
                "status",
                "compatibility_projection",
                "items",
            ] {
                assert!(obj.contains_key(field), "分组缺少字段 {field}");
            }
            let items = obj["items"].as_array().expect("items array");
            assert!(!items.is_empty(), "分组 items 不应为空");
            for item in items {
                assert!(item.get("work_item_id").is_some(), "item 缺 work_item_id");
                assert!(item.get("title").is_some(), "item 缺 title");
            }
        }

        // 未指定仓库组 compatibility_projection = true 且恒置末。
        let unassigned = groups.last().expect("unassigned group");
        assert!(
            unassigned["compatibility_projection"]
                .as_bool()
                .expect("compatibility_projection")
        );
        assert!(unassigned["target_repository_id"].is_null());

        // 指定仓库组：target_repository_id 为 UUID 字符串，alias 回落到物理投影名 repo-1。
        let assigned: Vec<_> = groups
            .iter()
            .filter(|g| !g["compatibility_projection"].as_bool().unwrap())
            .collect();
        assert_eq!(assigned.len(), 2);
        for group in assigned {
            assert!(group["target_repository_id"].is_string());
            assert_eq!(group["alias"].as_str(), Some(REPOSITORY_ID));
        }
    }

    #[tokio::test]
    async fn issue_lifecycle_returns_delivery_summary_with_partial_entries() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        seed_issue_and_work_items(&lifecycle, &paths);

        // 多仓 partial 场景：wi-a 已推（Pushed）、wi-b push 失败（Failed）、wi-c 无 attempt。
        let coding_store = CodingAttemptStore::new(paths.clone());
        let attempt_a =
            seed_completed_attempt(&coding_store, "wi-a", "aria/wi-a/attempt-1", "sha111");
        seed_review_request(&coding_store, &attempt_a, PushStatus::Pushed, None);
        let attempt_b =
            seed_completed_attempt(&coding_store, "wi-b", "aria/wi-b/attempt-1", "sha222");
        seed_review_request(
            &coding_store,
            &attempt_b,
            PushStatus::Failed,
            Some("push rejected".to_string()),
        );

        let app = build_test_router(temp.path());
        let response = get_issue_lifecycle(&app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let summary = &value["delivery_summary"];
        assert_eq!(summary["project_id"].as_str(), Some(PROJECT_ID));
        assert_eq!(summary["issue_id"].as_str(), Some(ISSUE_ID));
        assert_eq!(summary["overall"].as_str(), Some("partial"));

        let entries = summary["entries"].as_array().expect("delivery entries");
        assert_eq!(entries.len(), 3);

        let entry = |work_item_id: &str| -> &serde_json::Value {
            entries
                .iter()
                .find(|entry| entry["work_item_id"].as_str() == Some(work_item_id))
                .unwrap_or_else(|| panic!("missing delivery entry for {work_item_id}"))
        };

        let wi_a = entry("wi-a");
        assert_eq!(wi_a["repository_name"].as_str(), Some("checkout-wi-a"));
        assert_eq!(wi_a["attempt_status"].as_str(), Some("completed"));
        assert_eq!(wi_a["push_status"].as_str(), Some("pushed"));
        assert!(wi_a["push_error"].is_null());

        let wi_b = entry("wi-b");
        assert_eq!(wi_b["repository_name"].as_str(), Some("checkout-wi-b"));
        assert_eq!(wi_b["attempt_status"].as_str(), Some("completed"));
        assert_eq!(wi_b["push_status"].as_str(), Some("failed"));
        assert_eq!(wi_b["push_error"].as_str(), Some("push rejected"));

        let wi_c = entry("wi-c");
        assert!(wi_c["attempt_status"].is_null());
        assert!(wi_c["push_status"].is_null());
        assert!(wi_c["push_error"].is_null());
    }

    #[tokio::test]
    async fn grouped_view_propagates_list_work_items_error_instead_of_silent_empty_groups() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        seed_issue_and_work_items(&lifecycle, &paths);

        // 构造 schema-v2 plan lineage：让 legacy 路径跳过 list_work_items，
        // 从而使分组视图成为唯一调用 list_work_items 的地方——失败必须传播为 handler 错误，
        // 而不是静默返回空 work_item_repository_groups（回归 Task10 fix）。
        let plan = lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some("plan-1".to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                options: crate::product::models::IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec!["wi-a".to_string()],
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            })
            .unwrap();
        let revision_store = WorkItemRevisionStore::new(paths.clone());
        revision_store
            .put_plan_lineage(&WorkItemPlanLineage {
                id: plan.id.clone(),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_refs: Vec::new(),
                design_spec_refs: Vec::new(),
                active_revision_id: None,
                active_amendment_id: None,
                created_at: "2026-08-09T00:00:00Z".to_string(),
                updated_at: "2026-08-09T00:00:00Z".to_string(),
            })
            .unwrap();

        // 破坏 work_items 数据文件：list_work_items（分组路径）必然失败。
        let corrupt_path = lifecycle
            .work_items_root(PROJECT_ID, ISSUE_ID)
            .join("wi-a.json");
        std::fs::write(&corrupt_path, "{ not valid json").unwrap();

        let app = build_test_router(temp.path());
        let response = get_issue_lifecycle(&app).await;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "list_work_items 失败必须传播为 500，而不是静默返回空分组"
        );
    }
