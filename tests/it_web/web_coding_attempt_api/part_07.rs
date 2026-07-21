use cadence_aria::product::coding_models::CodingAttemptPlanBinding;
use cadence_aria::product::models::{LogicalWorkItem, WorkItemRevision};
use cadence_aria::product::work_item_contract::{
    CanonicalWorkItemContract, HandoffContract, WorkItemContractIdentity, WorkItemGoal,
    WorkItemWritePolicy, canonical_contract_hash,
};

fn group_canonical_contract(logical_id: &str, title: &str) -> CanonicalWorkItemContract {
    CanonicalWorkItemContract {
        schema_version: 1,
        identity: WorkItemContractIdentity {
            logical_work_item_id: logical_id.to_string(),
            title: title.to_string(),
            kind: "implementation".to_string(),
        },
        goal: WorkItemGoal {
            summary: title.to_string(),
        },
        non_goals: Vec::new(),
        input_contracts: Vec::new(),
        output_contracts: Vec::new(),
        tasks: Vec::new(),
        write_policy: WorkItemWritePolicy {
            exclusive_scopes: vec!["src/".to_string()],
            forbidden_scopes: Vec::new(),
        },
        acceptance_criteria: Vec::new(),
        verification_checks: Vec::new(),
        handoff_contract: HandoffContract {
            required_fields: Vec::new(),
            provided_contract_refs: Vec::new(),
            reviewer_check_refs: Vec::new(),
        },
        blocker_rules: Vec::new(),
        design_traceability: Vec::new(),
    }
}

fn seed_group_work_item_revisions(
    store: &WorkItemRevisionStore,
    lineage: &WorkItemPlanLineage,
) {
    for (logical_id, revision_id, title) in [
        (
            "work_item_0001",
            "work_item_revision_0001",
            "实现爬楼梯",
        ),
        (
            "work_item_0002",
            "work_item_revision_0002",
            "实现爬楼梯 part 2",
        ),
    ] {
        let logical = LogicalWorkItem {
            id: logical_id.to_string(),
            plan_id: lineage.id.clone(),
            title: title.to_string(),
            active_revision_id: None,
            created_at: "2026-07-18T00:00:00Z".to_string(),
            updated_at: "2026-07-18T00:00:00Z".to_string(),
        };
        store
            .put_logical_work_item(lineage, &logical)
            .expect("logical work item");
        let contract = group_canonical_contract(logical_id, title);
        store
            .put_work_item_revision(
                lineage,
                &WorkItemRevision {
                    id: revision_id.to_string(),
                    logical_work_item_id: logical_id.to_string(),
                    source_draft_revision_id: format!("draft_{revision_id}"),
                    canonical_contract_hash: canonical_contract_hash(&contract)
                        .expect("contract hash"),
                    canonical_contract: contract,
                    work_item_projection_bundle_id: format!("projection_{revision_id}"),
                    verification_plan_revision_id: format!("verification_{revision_id}"),
                    created_at: "2026-07-18T00:00:00Z".to_string(),
                },
            )
            .expect("work item revision");
        store
            .set_active_work_item_revision(lineage, &logical, None, revision_id)
            .expect("active work item revision");
    }
}

fn partial_group_attempt(store: &CodingAttemptStore) -> CodingExecutionAttempt {
    store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("partial group attempt")
}

fn save_group_plan_binding(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    store
        .save_plan_binding(
            attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: "work_item_plan_0001".to_string(),
                bound_plan_revision_id: "plan_revision_0001".to_string(),
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-07-18T00:00:00Z".to_string(),
            },
        )
        .expect("plan binding");
}

fn save_first_group_unit(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("first group unit");
}

#[tokio::test]
async fn coding_plan_repair_missing_bound_revision_entity_fails_without_side_effects() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    fs::remove_file(
        app_paths
            .issue_root("project_0001", "issue_0001")
            .join("work-item-revisions/work_item_plan_0001/logical-work-items/work_item_0002/revisions/work_item_revision_0002.json"),
    )
    .expect("remove bound revision");

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "coding_plan_revision_binding_missing");
    assert_group_attempt_creation_rolled_back(&app_paths);
}

#[tokio::test]
async fn coding_plan_repair_partial_group_attempt_retries_fail_closed() {
    for initialized_parts in 0..3 {
        let root = tempdir().expect("root");
        let repo = git_repo();
        let app = build_web_router(WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        ));
        bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
        let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let attempt = partial_group_attempt(&store);
        if initialized_parts >= 1 {
            save_group_plan_binding(&store, &attempt);
        }
        if initialized_parts >= 2 {
            save_first_group_unit(&store, &attempt);
        }

        let (status, body) = request_json(
            app,
            Method::POST,
            "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
            json!({}),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "coding_group_attempt_incomplete");
        assert_eq!(
            store
                .get_attempt("project_0001", "issue_0001", &attempt.id)
                .expect("existing attempt")
                .id,
            attempt.id
        );
    }
}

#[tokio::test]
async fn coding_plan_repair_existing_group_attempt_requires_exact_units() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = partial_group_attempt(&store);
    save_group_plan_binding(&store, &attempt);
    save_first_group_unit(&store, &attempt);
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("mismatched second unit");

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "coding_group_attempt_incomplete");
}

#[tokio::test]
async fn coding_plan_repair_existing_group_attempt_rejects_invalid_active_pointers() {
    for corruption in ["missing", "pending", "mismatch", "completed", "multiple"] {
        let root = tempdir().expect("root");
        let repo = git_repo();
        let app = build_web_router(WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        ));
        bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
        let path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";
        let (first_status, first) = request_json(app.clone(), Method::POST, path, json!({})).await;
        assert_eq!(first_status, StatusCode::OK);
        let attempt_id = assert_global_attempt_id(&first);
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = CodingAttemptStore::new(app_paths.clone());
        let mut attempt = store
            .get_attempt("project_0001", "issue_0001", &attempt_id)
            .expect("attempt");
        match corruption {
            "missing" => {
                attempt.active_unit_id = None;
                attempt.current_work_item_id = None;
            }
            "pending" => {
                attempt.active_unit_id = Some("coding_unit_0002".to_string());
                attempt.current_work_item_id = Some("work_item_0002".to_string());
            }
            "mismatch" => {
                attempt.active_unit_id = Some("coding_unit_0001".to_string());
                attempt.current_work_item_id = Some("work_item_0002".to_string());
            }
            "completed" => {
                store
                    .update_coding_unit_status(
                        "project_0001",
                        "issue_0001",
                        &attempt_id,
                        "coding_unit_0001",
                        CodingExecutionUnitStatus::Completed,
                        None,
                    )
                    .expect("complete first unit");
                attempt = store
                    .get_attempt("project_0001", "issue_0001", &attempt_id)
                    .expect("attempt after completion");
                attempt.active_unit_id = Some("coding_unit_0001".to_string());
                attempt.current_work_item_id = Some("work_item_0001".to_string());
            }
            "multiple" => {
                let second_unit_path = app_paths
                    .issue_root("project_0001", "issue_0001")
                    .join(format!("coding-attempts/{attempt_id}/units/coding_unit_0002.json"));
                let mut second: Value = serde_json::from_slice(
                    &fs::read(&second_unit_path).expect("second unit"),
                )
                .expect("parse second unit");
                second["status"] = json!("running");
                fs::write(
                    second_unit_path,
                    serde_json::to_vec_pretty(&second).expect("serialize second unit"),
                )
                .expect("write second active unit");
            }
            _ => unreachable!(),
        }
        store
            .save_coding_attempt(&attempt)
            .expect("persist corrupt pointers");

        let (status, body) = request_json(app, Method::POST, path, json!({})).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "corruption={corruption}");
        assert_eq!(body["code"], "coding_group_attempt_incomplete");
    }
}

#[tokio::test]
async fn coding_plan_repair_corrupt_existing_binding_remains_store_error() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";
    let (first_status, first) = request_json(app.clone(), Method::POST, path, json!({})).await;
    assert_eq!(first_status, StatusCode::OK);
    let attempt_id = assert_global_attempt_id(&first);
    fs::write(
        ProductAppPaths::new(root.path().join(".aria"))
            .issue_root("project_0001", "issue_0001")
            .join(format!("coding-attempts/{attempt_id}/plan-binding.json")),
        "{invalid json",
    )
    .expect("corrupt plan binding");

    let (status, body) = request_json(app, Method::POST, path, json!({})).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["code"], "product_store_error");
}
