/// coding WS 恢复 fixture（WorkItem 作用域）。
///
/// `semi_started=true` 构造 Running + Coding 的半启动 attempt：
/// - `materialized=true` 时先在 repo 上物化 worktree（含分支）；
/// - `with_head_commit` 控制是否落盘 head；
/// - `materialized=false` 复现 sc_advance 延迟物化形态——worktree_path 已登记但目录不存在。
///
/// `semi_started=false` 保留 Created/PrepareContext 初始态（经典 StartCoding 全链入口）。
fn app_with_coding_ws_resume_fixture(
    root_path: &Path,
    materialized: bool,
    with_head_commit: bool,
    semi_started: bool,
) -> (axum::Router, WebAppState, std::path::PathBuf) {
    let repo = root_path.join("repo");
    let remote = root_path.join("remote.git");
    init_cargo_repo(&repo);
    run_git(root_path, &["init", "--bare", remote.to_str().unwrap()]);
    run_git(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);

    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo.clone(),
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: "coding-ws-part16-semi-started-repository".to_string(),
        })
        .expect("create repository");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "实现爬楼梯".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    lifecycle
        .update_work_item_plan_status(
            "project_0001",
            "issue_0001",
            "work_item_0001",
            WorkItemPlanStatus::Confirmed,
        )
        .expect("confirm work item");
    let store = CodingAttemptStore::new(app_paths);
    let attempt = create_legacy_coding_attempt_fixture(
        &store,
        CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        },
    );

    let worktree = repo
        .join(".worktrees")
        .join("aria-work-items")
        .join("work_item_0001")
        .join("attempt-1");
    if materialized {
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "aria/work-items/work_item_0001/attempt-1",
                worktree.to_str().unwrap(),
                "HEAD",
            ],
        );
        assert!(worktree.exists(), "fixture worktree must be materialized");
    }

    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
    if semi_started {
        store
            .update_attempt_stage(
                "project_0001",
                "issue_0001",
                &attempt.id,
                CodingExecutionStage::Coding,
            )
            .expect("coding stage");
        store
            .update_attempt_worktree_path(
                "project_0001",
                "issue_0001",
                &attempt.id,
                worktree.clone(),
            )
            .expect("worktree path");
    }
    if semi_started && with_head_commit {
        let head = git_stdout(&worktree, &["rev-parse", "HEAD"]);
        store
            .update_attempt_head_commit(
                "project_0001",
                "issue_0001",
                &attempt.id,
                Some(head.trim().to_string()),
            )
            .expect("head commit");
    }

    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, Arc::new(FullChainStreamingProvider));
    let state = WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    );
    (build_web_router(state.clone()), state, worktree)
}

/// 组 1：Running + Coding + 无 runner 的半启动 attempt，attach 后不发送
/// StartCoding，runner 必须被自动重启（注册表 + Coding 阶段门事件证据），
/// 且会话在门确认后继续推进到 Coding 时间线节点。
#[tokio::test]
async fn coding_ws_attach_restarts_runner_for_semi_started_coding_attempt() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let (app, state, _worktree) =
        app_with_coding_ws_resume_fixture(root.path(), true, true, true);
    let attempt_key = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { status, stage, .. } => {
            assert_eq!(status, CodingAttemptStatus::Running);
            assert_eq!(stage, CodingExecutionStage::Coding);
        }
        other => panic!("expected initial coding session state, got {other:?}"),
    }

    // 不发送 StartCoding：runner 只能由 attach 恢复路径重启。
    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(gate.stage.as_ref(), Some(&CodingExecutionStage::Coding));
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.coding_runs.runner_count(&attempt_key) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("semi-started attach must restart the coding runner");

    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        },
    )
    .await;
    let node = wait_for_timeline_node(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(node.status, CodingTimelineNodeStatus::Running);
    let persisted = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt after resume");
    assert_eq!(persisted.status, CodingAttemptStatus::Running);

    ws.close(None).await.expect("close ws");
    server.abort();
}

/// 组 2（已物化 + head_commit=null）：attach 恢复先做补物化判定——worktree
/// 已物化故不回落，runner 直接重启推进会话；WorkItem 作用域 Coding 中态
/// head_commit 本就延迟到 review request 落盘，恢复路径不得越权补写。
#[tokio::test]
async fn coding_ws_semi_started_attach_with_null_head_commit_restarts_runner() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let (app, _state, _worktree) =
        app_with_coding_ws_resume_fixture(root.path(), true, false, true);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    let _gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    let persisted = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt after resume");
    assert_eq!(
        persisted.head_commit, None,
        "work-item Coding mid-state must not have its head_commit backfilled by the resume path"
    );
    assert_eq!(persisted.stage, CodingExecutionStage::Coding);

    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        },
    )
    .await;
    let node = wait_for_timeline_node(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(node.status, CodingTimelineNodeStatus::Running);

    ws.close(None).await.expect("close ws");
    server.abort();
}

/// 组 2（未物化 + head_commit=null，sc_advance 延迟物化形态）：attach 恢复时
/// 回落 WorktreePrepare 重新物化，断言 worktree 目录被 runner 物化且会话
/// 推进到 Coding 阶段门与时间线节点。
#[tokio::test]
async fn coding_ws_semi_started_attach_with_unmaterialized_worktree_reprepares_and_restarts() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let (app, _state, worktree) =
        app_with_coding_ws_resume_fixture(root.path(), false, false, true);
    assert!(
        !worktree.exists(),
        "fixture must keep worktree unmaterialized"
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    let gate = wait_for_stage_gate(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(gate.stage.as_ref(), Some(&CodingExecutionStage::Coding));
    assert!(
        worktree.exists(),
        "resumed runner must re-materialize the worktree via WorktreePrepare"
    );

    send_json(
        &mut ws,
        &CodingWsInMessage::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        },
    )
    .await;
    let node = wait_for_timeline_node(&mut ws, CodingExecutionStage::Coding).await;
    assert_eq!(node.status, CodingTimelineNodeStatus::Running);

    ws.close(None).await.expect("close ws");
    server.abort();
}

/// 组 4（幂等负向）：活 runner 存在时第二个 socket attach 不得重复 spawn——
/// 注册表保持单一 runner，且新 socket 只收到快照（纯快照行为不变，用 Ping/Pong
/// 证明 socket 循环未被恢复路径阻塞、也没有第二个 runner 的事件涌入）。
#[tokio::test]
async fn coding_ws_attach_with_live_runner_never_respawns_and_keeps_snapshot_only() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let (app, state, _worktree) =
        app_with_coding_ws_resume_fixture(root.path(), false, false, false);
    let attempt_key = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut first_ws, _) = connect_async(&url).await.expect("connect first ws");
    let _initial = recv_json(&mut first_ws).await;
    send_json(&mut first_ws, &CodingWsInMessage::StartCoding).await;
    let _gate = wait_for_stage_gate(&mut first_ws, CodingExecutionStage::Coding).await;
    assert_eq!(state.coding_runs.runner_count(&attempt_key), 1);

    let (mut second_ws, _) = connect_async(&url).await.expect("connect second ws");
    match recv_json(&mut second_ws).await {
        CodingWsOutMessage::CodingSessionState { status, stage, .. } => {
            assert_eq!(status, CodingAttemptStatus::Running);
            // Coding 阶段门确认前持久化 stage 仍为 WorktreePrepare；这里不锁定阶段，
            // 只验证活 runner 下的 attach 是纯快照（不重复 spawn、无恢复事件）。
            assert!(
                stage == CodingExecutionStage::Coding
                    || stage == CodingExecutionStage::WorktreePrepare
            );
        }
        other => panic!("expected snapshot-only attach for live runner, got {other:?}"),
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        state.coding_runs.runner_count(&attempt_key),
        1,
        "attach with a live runner must never spawn a second runner"
    );

    send_json(&mut second_ws, &CodingWsInMessage::CodingPing).await;
    match timeout(Duration::from_secs(2), recv_json(&mut second_ws)).await {
        Ok(CodingWsOutMessage::CodingPong) => {}
        other => panic!("expected pong directly after snapshot, got {other:?}"),
    }

    first_ws.close(None).await.expect("close first ws");
    second_ws.close(None).await.expect("close second ws");
    server.abort();
}
