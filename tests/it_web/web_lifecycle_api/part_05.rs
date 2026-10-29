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
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use cadence_aria::product::lifecycle_store::CreateIssueWorkItemPlanInput;
use cadence_aria::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, WorkspaceRolePermissionModes,
};
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;

#[tokio::test]
async fn work_item_plan_delete_rejects_zero_mutation_when_any_work_item_has_attempt() {
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
            work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
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
            reviewer_provider: Some(ProviderName::Fake),

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
            reviewer_provider: Some(ProviderName::Fake),

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

    // D2.2 迁移（REQ-DEL-05）：门禁在任何删除前拒绝——两个 work item 的 session
    // 磁盘记录与内存 manager 全部保持（旧「部分失败仍驱逐」语义被零破坏取代）。
    let disk_sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list sessions");
    assert!(
        disk_sessions
            .iter()
            .any(|record| record.entity_id == "work_item_0001"),
        "拒删零破坏：work_item_0001 的 session 磁盘记录必须保留"
    );

    assert!(
        state.workspace_sessions.get(&session.id).await.is_some(),
        "拒删不改 manager：plan session 的内存 manager 不被驱逐"
    );
}

// ---- 三类实体级联删除的内存面驱逐（v37 反馈 #3 根因修复）----
// `delete_work_item` 早已在删除后驱逐其 workspace session 的内存 manager，
// 但 Story/Design/Plan 走的 `cascade_delete_entity` 只删磁盘记录——registry
// 缓存的 manager 仍持有删除前的 engine 状态，重新生成复用同 id session 时
// （max_workspace_session_sequence 只数磁盘 json，删除后序号回退）WS 连接经
// get_or_create_and_attach 命中旧 manager，把已删会话的旧状态原样发给前端。
// 契约：删除**成功后**驱逐；门禁拒绝/删除失败**不**驱逐（失败零副作用）。

/// Design 级联删除成功后驱逐其 workspace session 的内存 manager。
#[tokio::test]
async fn design_spec_delete_evicts_cached_runtime_manager() {
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
        json!({"name":"EvictDesign","description":null}),
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
        json!({"title":"EvictDesign","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let (status, _) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"Evict design story","author_provider":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // 下游 design 生成要求 story 已确凿确认（REQ：story 未确认不得生成 design）。
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;

    let (status, design) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"Evict design",
            "author_provider":"fake",
            "story_spec_ids":["story_spec_0001"]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let design_session_id = design["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("design session id")
        .to_string();

    // 模拟用户打开过 design workspace：manager 缓存进 registry。
    let factory_state = state.clone();
    let factory_session_id = design_session_id.clone();
    state
        .workspace_sessions
        .get_or_create(&design_session_id, move || {
            let state = factory_state.clone();
            let session_id = factory_session_id.clone();
            async move { WorkspaceSessionManager::create(&state, &session_id).await }
        })
        .await
        .expect("design manager attached");

    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/design-specs/design_spec_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        state.workspace_sessions.get(&design_session_id).await.is_none(),
        "删除 design spec 后必须驱逐其 workspace session 的内存 manager"
    );
}

/// Plan 级联删除成功后驱逐其 workspace session 的内存 manager。
#[tokio::test]
async fn work_item_plan_delete_evicts_cached_runtime_manager() {
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
        json!({"name":"EvictPlan","description":null}),
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
        json!({"title":"EvictPlan","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    // draft plan（never confirmed → 删除无需 confirmed=true；无 work item
    // → 无 coding 门禁、无子会话）。
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
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create draft plan");

    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "issue_work_item_plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("create plan session");

    // 模拟用户打开过 plan workspace：manager 缓存进 registry。
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
        .expect("plan manager attached");

    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        state.workspace_sessions.get(&session.id).await.is_none(),
        "删除 work item plan 后必须驱逐其 workspace session 的内存 manager"
    );
}

/// 删除被门禁拒绝（分级确认门禁）时**不得**驱逐内存 manager（失败零副作用）。
#[tokio::test]
async fn story_spec_delete_rejected_by_gate_keeps_cached_runtime_manager() {
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
        json!({"name":"EvictGate","description":null}),
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
        json!({"title":"EvictGate","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    let (status, story) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({"title":"Evict gate story","author_provider":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let story_session_id = story["workspace_session"]["workspace_session_id"]
        .as_str()
        .expect("story session id")
        .to_string();

    // 确凿确认过 → 删除走分级确认门禁（REQ-DEL-04）：无 confirmed=true 即拒删。
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;

    let factory_state = state.clone();
    let factory_session_id = story_session_id.clone();
    state
        .workspace_sessions
        .get_or_create(&story_session_id, move || {
            let state = factory_state.clone();
            let session_id = factory_session_id.clone();
            async move { WorkspaceSessionManager::create(&state, &session_id).await }
        })
        .await
        .expect("story manager attached");

    let (status, error) = request_json(
        app.clone(),
        Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/story-specs/story_spec_0001",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "deletion_confirmation_required");

    assert!(
        state.workspace_sessions.get(&story_session_id).await.is_some(),
        "拒删不改 manager：删除被门禁拒绝时不得驱逐其 workspace session 的内存 manager"
    );
}
