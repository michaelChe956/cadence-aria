// 退役留档（T5/REQ-RET-02）：`work_item_plan_revision_streams_provider_output_before_candidate_artifact` 直接驱动已删除的 legacy 决策面
// （原 #[ignore]：legacy full-candidate revision 流已被 WP2 outline 生成取代；本测含 revert_work_item 退役 wire 字面量），
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn issue_lifecycle_includes_work_item_plan_groups_without_hiding_child_work_items() {
    let (app, repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let lifecycle = LifecycleStore::new(ProductAppPaths::new(repo.path().join(".aria")));
    let backend_item = lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现后端登录会话 API".to_string(),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(10),
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .unwrap();
    let frontend_item = lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现前端会话过期提示".to_string(),
            kind: WorkItemKind::Frontend,
            sequence_hint: Some(20),
            depends_on: vec![backend_item.id.clone()],
            plan_status: WorkItemPlanStatus::Draft,
            ..Default::default()
        })
        .unwrap();
    let expected_work_item_ids = vec![backend_item.id.clone(), frontend_item.id.clone()];

    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: None,
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: true,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: expected_work_item_ids.clone(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .unwrap();

    let (status, lifecycle_response) = request_json(
        app,
        Method::GET,
        "/api/issues/issue_0001/lifecycle?project_id=project_0001",
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let groups = lifecycle_response["work_item_plans"]
        .as_array()
        .expect("work_item_plans group list");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["id"], plan.id);
    assert_eq!(groups[0]["status"], "draft");
    assert_eq!(
        groups[0]["source_story_spec_ids"],
        json!(["story_spec_0001"])
    );
    assert_eq!(
        groups[0]["source_design_spec_ids"],
        json!(["design_spec_0001"])
    );
    assert_eq!(groups[0]["work_item_ids"], json!(expected_work_item_ids));

    let work_items = lifecycle_response["work_items"].as_array().unwrap();
    assert_eq!(work_items.len(), 2);
    let returned_work_item_ids = work_items
        .iter()
        .map(|item| item["work_item_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(returned_work_item_ids.contains(&backend_item.id.as_str()));
    assert!(returned_work_item_ids.contains(&frontend_item.id.as_str()));
}

// 退役留档（T5/REQ-RET-02）：`work_item_plan_author_persists_outline_without_draft_work_items_or_child_sessions` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_plan_author_emits_provider_prompt_event` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：review fixture 注入与构造辅助（enable_work_item_plan_review_fixture/
// outline_review_revise/outline_review_pass）为 staged author review 族测试专用，随消息族一并退役。
