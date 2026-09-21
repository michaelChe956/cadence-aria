// v37 反馈 #3：story spec 删除后重新生成仍旧错误态。
// 根因之二（内存面）：DELETE /story-specs 只删磁盘记录，WorkspaceSessionRegistry
// 缓存的 manager（持有删除前 engine 状态）不被驱逐；重新生成复用同 id session
// （max_workspace_session_sequence 只数磁盘 json，删除后序号回退）时，WS 连接经
// get_or_create_and_attach 命中旧 manager，把已删会话的旧状态原样发给前端。
use std::sync::Arc;

use cadence_aria::web::workspace_session::WorkspaceSessionManager;

#[tokio::test]
async fn story_spec_delete_evicts_cached_runtime_manager_and_regenerate_starts_clean() {
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
        json!({"name":"Evict","description":null}),
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
        json!({"title":"Evict","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let (status, first) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"Evict story","author_provider":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first_session_id = first["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("first session id")
        .to_string();

    // 模拟用户打开过 workspace：WS 连接把 manager 缓存进 registry。
    let factory_state = state.clone();
    let factory_session_id = first_session_id.clone();
    let first_manager = state
        .workspace_sessions
        .get_or_create(&first_session_id, move || {
            let state = factory_state.clone();
            let session_id = factory_session_id.clone();
            async move { WorkspaceSessionManager::create(&state, &session_id).await }
        })
        .await
        .expect("first manager attached");

    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        state
            .workspace_sessions
            .get(&first_session_id)
            .await
            .is_none(),
        "删除 story spec 后必须驱逐其 workspace session 的内存 manager"
    );

    // 重新生成：磁盘 json 已删 → 序号回退复用 workspace_session_0001，
    // 必须得到全新 manager 而非删除前的旧运行时状态。
    let (status, second) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"Evict story again","author_provider":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second_session_id = second["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("second session id")
        .to_string();

    let factory_state = state.clone();
    let factory_session_id = second_session_id.clone();
    let second_manager = state
        .workspace_sessions
        .get_or_create(&second_session_id, move || {
            let state = factory_state.clone();
            let session_id = factory_session_id.clone();
            async move { WorkspaceSessionManager::create(&state, &session_id).await }
        })
        .await
        .expect("second manager attached");

    assert!(
        !Arc::ptr_eq(&first_manager, &second_manager),
        "删除后重新生成必须是新 manager，而不是复用旧运行时状态"
    );
}

// del-review k3 P2：plan 删除部分失败路径跳过内存驱逐。
// legacy plan 下属两个 work item：第一个删除时其 session 磁盘级联已完成，
// 第二个被 work item 级门禁（存在 coding attempt）拒绝 → 整体 409 中断。
// 此时第一个 work item 已处于「磁盘 session 已删」状态，若驱逐被跳过，
// registry 残留的旧 manager 会在重新生成复用同 id session 时把删除前的
// engine 状态原样发给前端——必须即使失败也先驱逐再传播错误。
use cadence_aria::product::coding_attempt_store::{
    CodingAttemptStore, CreateCodingAttemptInput,
};
use cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput;
use cadence_aria::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, WorkspaceRolePermissionModes,
};
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;

#[tokio::test]
async fn work_item_plan_delete_partial_failure_still_evicts_deleted_work_item_manager() {
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
        json!({"name":"EvictPartial","description":null}),
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
        json!({"title":"EvictPartial","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                title: format!("任务 {work_item_id}"),
                plan_status: WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .expect("create work item");
    }
    // legacy plan（无 revision lineage）：删除走 plan 遍历下属 work item 支路。
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
            work_item_ids: vec![
                "work_item_0001".to_string(),
                "work_item_0002".to_string(),
            ],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create legacy plan");

    // work_item_0001 的 WorkItem 型 session：删除级联会清掉其磁盘记录，
    // 用于证明部分失败时「磁盘已删」确实发生。
    lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_0001".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("create work item session");

    // plan 自身的 WorkItemPlan 型 session（entity_id=plan_id，同属删除前的
    // 驱逐收集集合），模拟用户打开过 workspace：manager 已缓存进 registry。
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "issue_work_item_plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("create plan session");
    let factory_state = state.clone();
    let factory_session_id = session.id.clone();
    state
        .workspace_sessions
        .get_or_create(&session.id, move || {
            let state = factory_state.clone();
            let session_id = factory_session_id.clone();
            async move { WorkspaceSessionManager::create(&state, &session_id).await }
        })
        .await
        .expect("manager attached");

    // 第二个 work item 挂 per-work-item coding attempt（work_item_group_id=None，
    // plan 级门禁放行，work item 级门禁拒绝）→ 构造「第一个已删、第二个被拒」的部分失败。
    CodingAttemptStore::new(app_paths.clone())
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0002".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
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
        .expect("create coding attempt");

    let (status, error) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "coding_workspace_exists");

    // 部分失败确实发生：第一个 work item 已删，其 session 磁盘级联已完成。
    let disk_sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list sessions");
    assert!(
        disk_sessions
            .iter()
            .all(|record| record.entity_id != "work_item_0001"),
        "前置失效：work_item_0001 的 session 磁盘记录应已被级联删除"
    );

    // 内存面：即使整体删除失败，已删 work item 的 manager 也必须被驱逐。
    assert!(
        state.workspace_sessions.get(&session.id).await.is_none(),
        "plan 删除部分失败时也必须驱逐已删 work item 的内存 manager"
    );
}
