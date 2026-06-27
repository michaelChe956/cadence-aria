#[test]
fn replace_issue_work_item_plan_candidate_rejects_confirmed_plan() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "old work item".to_string(),
            id: Some("work_item_0001".to_string()),
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .expect("work item");

    let plan = store
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![work_item.id.clone()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("plan");

    store
        .confirm_issue_work_item_plan("project_0001", "issue_0001", &plan.id)
        .expect("confirm");

    let new_output = new_split_output_with_ids(
        "issue_work_item_plan_9999",
        "repository_profile_0002",
        &["work_item_0002", "work_item_0003"],
        &["verification_plan_0002", "verification_plan_0003"],
    );

    let result = store.replace_issue_work_item_plan_candidate(
        "project_0001",
        "issue_0001",
        &plan.id,
        &new_output,
        Vec::new(),
    );

    assert!(result.is_err());
    assert!(format!("{}", result.unwrap_err()).contains("not_draft"));
}
