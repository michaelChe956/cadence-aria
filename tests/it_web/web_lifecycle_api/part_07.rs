// H4（设计 2026-10-14 C 节）：DELETE …/work-item-plans/{plan}/coding-workspaces
// 路由接线冒烟——无 Coding 对象、无残留内容的已确认 plan 上幂等成功（零弹窗），
// 回执四字段（operation_state/generation/deleted/remaining + workspace_lifecycle
// 投影）。深行为（四仓端到端/半残收敛/clearing 续清）由
// src/web/handlers/coding/plan_start_tests.rs 的 h4 测试族覆盖（真实四仓夹具
// 在彼处，不在 it_web 复制第二套）。
#[tokio::test]
async fn plan_coding_workspaces_delete_route_idempotent_success_on_clean_plan() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"H4GroupDelete","description":null}),
    )
    .await;
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"H4GroupDelete","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "任务 work_item_0001".to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item");
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec!["work_item_0001".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create legacy confirmed plan");

    let uri = "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001/coding-workspaces?command_id=plan_delete_ui_itweb&confirmed=true";
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("send group delete");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("receipt json");
    assert_eq!(
        status,
        StatusCode::OK,
        "clean plan deletes idempotently: {body}"
    );
    assert_eq!(body["operation_state"], "accepted");
    assert_eq!(body["generation"], 1);
    assert_eq!(body["deleted_attempt_ids"], serde_json::json!([]));
    assert_eq!(body["remaining_attempt_ids"], serde_json::json!([]));
    assert_eq!(body["workspace_lifecycle"]["generation"], 1);
    assert_eq!(body["workspace_lifecycle"]["existing_member_count"], 0);
    assert_eq!(body["workspace_lifecycle"]["missing_member_count"], 0);
    assert_eq!(body["workspace_lifecycle"]["state"], "clean");

    // 未确认且无内容：不弹确认（幂等成功路径零弹窗），query 无 confirmed 也 200。
    let uri_no_confirm = "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001/coding-workspaces?command_id=plan_delete_ui_itweb_b";
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(uri_no_confirm)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("send group delete without confirm");
    let status = response.status();
    assert_eq!(
        status,
        StatusCode::OK,
        "no coding objects means no confirmation"
    );
}
