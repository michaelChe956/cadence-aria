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
