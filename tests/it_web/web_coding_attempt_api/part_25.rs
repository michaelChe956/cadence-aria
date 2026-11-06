// Task 3（REQ-WIGA-17）：取消与取锁的时序尾隙 + 取消结果不得谎称。
//
// 与 part_14 的 REQ-WIGA-15 用例共用同一夹具配方（bootstrap_confirmed_
// work_item_plan_group + 绑定 session + §1.5 假 run），但验的是不同的面：
//   - 用例①：completion 信号已到而 scope 共享 flock 尚未释放的时序尾隙下，
//     删除独占仍必须最终取得成功（不得 409 entity_busy）；
//   - 用例②：取消在兜底超时内未收口（收口失联）时，删除不得谎称已取消、
//     不得因取消失败被额外阻塞（仍按既有独占判定报 entity_busy）。

/// §1.6 用例①夹具：与 part_14 的 `spawn_fake_inflight_run` 同构，但把
/// 「释放共享 flock」与「发 completion 信号」的顺序**倒转**——假 run 任务
/// 被取消后**先** `finish_run` 发真实 completion，**再**延迟 `release_delay`
/// 释放共享 flock ⇒ 制造「completion 已到、锁还没放」的时序尾隙（生产中
/// 二者先后未验证：run 释放 scope 共享 flock 由 run task 全生命周期持有）。
async fn spawn_fake_inflight_run_completion_first(
    app_paths: &ProductAppPaths,
    manager: std::sync::Arc<WorkspaceSessionManager>,
    scope: EntityScope,
    release_delay: std::time::Duration,
) -> FakeInflightRun {
    let (_run_id, token, cancel_token, _cmd_rx, _node_id) = manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("start fake in-flight run");
    let lease = EntityOperationStore::new(app_paths.clone())
        .acquire_run(&scope)
        .expect("fake run acquires scope shared lock");
    let shared_lease = std::sync::Arc::new(std::sync::Mutex::new(Some(lease)));
    let task_lease = shared_lease.clone();
    let cancel_in_task = cancel_token.clone();
    let completion_handle = tokio::spawn(async move {
        cancel_in_task.cancelled().await;
        // 先发 completion 信号（尾隙前半：等待解除 ≠ 共享锁已释放）。
        manager.finish_run(token).await;
        // 再延迟释放共享 flock（尾隙窗口 = release_delay，期间锁仍被持有）。
        tokio::time::sleep(release_delay).await;
        if let Some(lease) = task_lease.lock().unwrap().take() {
            drop(lease);
        }
    });
    FakeInflightRun {
        cancel_token,
        completion_handle,
        shared_lease,
    }
}

/// REQ-WIGA-17 场景 ①：取消已发出且该 run 已真实收口（completion 信号已
/// 到），但其 scope 共享 flock 在 completion 之后才延迟释放——删除独占仍
/// SHALL 最终取得成功（不依赖「completion ⇒ 锁已释放」的未验证时序假设），
/// 删除 SHALL NOT 以 `entity_busy` 失败，SHALL NOT 在锁未释放时开始破坏性
/// 清理。
#[tokio::test]
async fn delete_work_item_plan_succeeds_when_scope_lock_releases_after_completion_signal() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());

    // ①/② 绑定创建 entity_id == work_item_plan_0001 的 WorkItemPlan 会话。
    let session = lifecycle
        .create_workspace_session_bound(
            CreateWorkspaceSessionInput {
                project_id: GROUP_PROJECT_ID.to_string(),
                issue_id: GROUP_ISSUE_ID.to_string(),
                entity_id: GROUP_PLAN_ID.to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            "workspace_session_9301".to_string(),
        )
        .expect("create bound plan workspace session");

    // ③ manager 挂进 registry（先例 web_lifecycle_api/part_05.rs:373-383）。
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
    let manager = state
        .workspace_sessions
        .get(&session.id)
        .await
        .expect("plan manager in registry");

    // ④ 假 in-flight run：被取消后先发 completion、延迟 250ms 才放共享
    //    flock——「completion 已到、锁还没放」的确定性尾隙。
    let scope = EntityScope::new(
        GROUP_PROJECT_ID,
        GROUP_ISSUE_ID,
        DeletionEntityKind::Plan,
        GROUP_PLAN_ID,
    );
    let fake = spawn_fake_inflight_run_completion_first(
        &app_paths,
        manager,
        scope.clone(),
        std::time::Duration::from_millis(250),
    )
    .await;

    // ⑤ DELETE：取消已真实收口，删除独占必须最终取得成功。
    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "completion 已到而共享 flock 尚未释放的时序尾隙下，删除必须补足有界等待并最终成功（不得 409 entity_busy）；body: {body}"
    );
    assert_eq!(body["status"], "deleted");

    // 取消确实发生且已真实收口；删除返回时共享 flock 必须已释放——删除
    // 独占只能在锁释放后取得，成功响应本身即「未在锁未释放时开始破坏性
    // 清理」的证据。
    assert!(
        fake.cancel_token.is_cancelled(),
        "删除路径必须先取消该实体在途 run 的 CancellationToken"
    );
    fake.completion_handle
        .await
        .expect("假 run 任务必须已收口（真实 completion）");
    assert!(
        fake.shared_lease.lock().unwrap().is_none(),
        "删除返回时 scope 共享 flock 必须已释放"
    );

    // 既有删除语义未被削弱：plan 已删净（无残留半清理状态）。
    assert!(
        lifecycle
            .get_issue_work_item_plan(GROUP_PROJECT_ID, GROUP_ISSUE_ID, GROUP_PLAN_ID)
            .is_err(),
        "plan 必须已被删除"
    );
}

/// REQ-WIGA-17 场景 ②：取消已发出但该 run 在兜底超时内未收口（收口失联：
/// 既不发 completion 也不释放共享 flock）——删除 SHALL NOT 谎称已取消该
/// run、SHALL NOT 因取消失败被额外阻塞（仍按既有独占判定推进：拿不到独占
/// 按既有语义报出占用者），且必须在有限时间内结束。
///
/// 有限时间上界（5s）远低于生产 30s 兜底：若实现把「取消失败」变成长阻塞
/// （兜底固定 30s 且不可注入），本用例在此上界处见红。
#[tokio::test]
async fn delete_work_item_plan_does_not_claim_cancel_when_completion_times_out() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());

    // 绑定创建 entity_id == work_item_plan_0001 的 WorkItemPlan 会话并挂进
    // registry。
    let session = lifecycle
        .create_workspace_session_bound(
            CreateWorkspaceSessionInput {
                project_id: GROUP_PROJECT_ID.to_string(),
                issue_id: GROUP_ISSUE_ID.to_string(),
                entity_id: GROUP_PLAN_ID.to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            "workspace_session_9302".to_string(),
        )
        .expect("create bound plan workspace session");
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
    let manager = state
        .workspace_sessions
        .get(&session.id)
        .await
        .expect("plan manager in registry");

    // 兜底时长注入为可测级（200ms；生产默认仍 30s——不得为跑得快删掉
    // 兜底语义，只经 set_run_completion_wait_timeout 注入）。
    manager.set_run_completion_wait_timeout(std::time::Duration::from_millis(200));

    // 假 in-flight run：收口失联——被取消后既不发 completion 信号，也不
    // 释放 scope 共享 flock（run「卡死」仍占删除独占的对手锁）。
    let scope = EntityScope::new(
        GROUP_PROJECT_ID,
        GROUP_ISSUE_ID,
        DeletionEntityKind::Plan,
        GROUP_PLAN_ID,
    );
    let (_run_id, _token, cancel_token, _cmd_rx, _node_id) = manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("start fake in-flight run");
    let cancel_for_assert = cancel_token.clone();
    let _unresponsive_run = tokio::spawn(async move {
        cancel_token.cancelled().await;
        // 收口失联：既不 finish_run（无 completion），也永不释放共享 flock。
        std::future::pending::<()>().await;
    });
    // 共享 flock 由本用例持有（模拟 run 任务全生命周期持锁），测试结束随
    // TempDir 与 lease drop 释放。
    let _scope_shared_lease = EntityOperationStore::new(app_paths.clone())
        .acquire_run(&scope)
        .expect("fake run acquires scope shared lock");

    let started = std::time::Instant::now();
    // 有限时间上界：删除不得因取消未收口被额外阻塞（兜底不可注入 ⇒ 30s
    // 长阻塞在本处见红）。
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        request_json(app, Method::DELETE, group_plan_uri(), json!({})),
    )
    .await;
    let elapsed = started.elapsed();
    let (status, body) = response.unwrap_or_else(|_| {
        panic!(
            "删除不得因取消未收口被额外阻塞（{elapsed:?} 内未返回；\
             兜底时长固定 30s 且不可注入 ⇒ 缺可注入兜底与三态取消结果）"
        )
    });

    // 拿不到独占：既有语义逐字不变——409 + entity_busy + 既有占用文案。
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "取消未收口时删除仍按既有独占判定推进（共享锁仍被持有 ⇒ 409）；body: {body}"
    );
    assert_eq!(body["code"], "entity_busy", "body: {body}");
    assert_eq!(
        body["message"],
        "实体存在进行中的运行或并发写入，请等待收口后重试删除",
        "既有 entity_busy 文案逐字不变；body: {body}"
    );

    // 不得谎称：响应体不得出现与事实相反的「已取消/已停止」表述。
    let body_text = body.to_string();
    assert!(
        !body_text.contains("已取消"),
        "取消未真正收口，响应体不得谎称已取消：{body_text}"
    );
    assert!(
        !body_text.contains("已停止"),
        "取消未真正收口，响应体不得谎称已停止：{body_text}"
    );

    // 有限时间内结束（秒级，远低于 30s 兜底）。
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "用例必须在有限时间内结束（实测 {elapsed:?}）"
    );

    // 取消确实发出过（token 已触发）——「没谎称」不是「没取消」。
    assert!(
        cancel_for_assert.is_cancelled(),
        "删除路径必须已发出取消（CancellationToken 已触发）"
    );

    // 未开始破坏性清理：实体仍在、无删除墓碑。
    assert!(
        lifecycle
            .get_issue_work_item_plan(GROUP_PROJECT_ID, GROUP_ISSUE_ID, GROUP_PLAN_ID)
            .is_ok(),
        "取消未收口、独占未取得时不得开始破坏性清理（plan 必须仍在）"
    );
    assert!(
        cadence_aria::product::entity_deletion_store::EntityDeletionStore::new(
            app_paths.clone()
        )
        .get(&scope)
        .expect("read deletion record")
        .is_none(),
        "取消未收口、独占未取得时不得立起删除墓碑"
    );
}
