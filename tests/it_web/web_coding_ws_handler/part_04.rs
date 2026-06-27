#[tokio::test]
async fn coding_ws_analyst_next_stage_testing_reruns_tester_before_code_review() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(RerunTestingProvider::default());
    let app = app_with_full_chain_attempt_and_provider(root.path(), provider.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut completed = false;
    let mut testing_nodes = 0;
    for _ in 0..180 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if respond_to_testing_result_review_gate(&mut ws, &gate).await {
                    continue;
                }
                assert_eq!(
                    gate.kind,
                    CodingGateKind::StageGate,
                    "unexpected non-stage gate: {:?} {:?}",
                    gate.reason_code,
                    gate.description
                );
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingTimelineNodeCreated { node }
                if node.stage == CodingExecutionStage::Testing =>
            {
                testing_nodes += 1;
            }
            CodingWsOutMessage::CodingSessionState { status, stage, .. }
                if status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::FinalConfirm =>
            {
                completed = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        completed,
        "expected completed attempt after analyst requested tester rerun"
    );
    assert_eq!(
        provider.testing_execute_calls(),
        2,
        "analyst next_stage=testing should rerun Tester"
    );
    assert_eq!(testing_nodes, 2);
    assert_eq!(
        store
            .list_testing_reports("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("testing reports")
            .len(),
        2
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn internal_review_rework_creates_new_review_request_commit() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_internal_review_rework_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut completed = false;
    for _ in 0..140 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if respond_to_testing_result_review_gate(&mut ws, &gate).await {
                    continue;
                }
                assert_eq!(
                    gate.kind,
                    CodingGateKind::StageGate,
                    "unexpected non-stage gate: {:?} {:?}",
                    gate.reason_code,
                    gate.description
                );
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingSessionState { status, stage, .. }
                if status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::FinalConfirm =>
            {
                completed = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        completed,
        "expected completed attempt after internal review rework"
    );
    let requests = store
        .list_review_requests("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("review requests");
    assert_eq!(requests.len(), 2);
    assert_ne!(requests[0].commit_sha, requests[1].commit_sha);
    assert_eq!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("internal reviews")
            .len(),
        2
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(
        attempt.review_request_id.as_deref(),
        Some("review_request_0002")
    );
    let worktree = attempt.worktree_path.expect("worktree path");
    assert!(worktree.join("src/internal_fix.rs").is_file());

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn code_review_findings_are_injected_into_next_coding_round() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let provider = Arc::new(CodeReviewReworkProvider::default());
    let app = app_with_code_review_rework_attempt(root.path(), provider.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut completed = false;
    for _ in 0..140 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if respond_to_testing_result_review_gate(&mut ws, &gate).await {
                    continue;
                }
                assert_eq!(
                    gate.kind,
                    CodingGateKind::StageGate,
                    "unexpected non-stage gate: {:?} {:?}",
                    gate.reason_code,
                    gate.description
                );
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingSessionState { status, stage, .. }
                if status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::FinalConfirm =>
            {
                completed = true;
                break;
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        completed,
        "expected completed attempt after code review rework"
    );
    let prompts = provider.coding_prompts();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("上一轮返修要求"));
    assert!(prompts[1].contains("移除 __pycache__ 和 .pyc 文件"));
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let worktree = attempt.worktree_path.expect("worktree path");
    let latest_request = store
        .list_review_requests("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("review requests")
        .pop()
        .expect("review request");
    let committed = git_stdout(
        &worktree,
        &[
            "show",
            "--name-only",
            "--format=",
            &latest_request.commit_sha,
        ],
    );
    assert!(!committed.contains("__pycache__"));
    assert!(!committed.contains(".pyc"));
    assert!(!committed.contains(".aria/coding-artifacts"));

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_final_confirm_completes_attempt_and_sends_snapshot() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_final_confirm_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState { status, stage, .. } => {
            assert_eq!(status, CodingAttemptStatus::WaitingForHuman);
            assert_eq!(stage, CodingExecutionStage::FinalConfirm);
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    send_json(&mut ws, &CodingWsInMessage::FinalConfirm).await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Completed);
            assert_eq!(summary.as_deref(), Some("用户已确认完成"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected final confirm timeline update, got {other:?}"),
    }
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            status,
            stage,
            active_node_id,
            ..
        } => {
            assert_eq!(status, CodingAttemptStatus::Completed);
            assert_eq!(stage, CodingExecutionStage::FinalConfirm);
            assert!(active_node_id.is_none());
        }
        other => panic!("expected completed coding session state, got {other:?}"),
    }
    let updated = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Completed);
    assert!(updated.completed_at.is_some());

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_abort_attempt_closes_active_node_and_sends_snapshot() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_running_testing_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            status,
            stage,
            active_node_id,
            ..
        } => {
            assert_eq!(status, CodingAttemptStatus::Running);
            assert_eq!(stage, CodingExecutionStage::Testing);
            assert_eq!(active_node_id.as_deref(), Some("coding_node_0001"));
        }
        other => panic!("expected coding session state, got {other:?}"),
    }

    send_json(&mut ws, &CodingWsInMessage::AbortAttempt).await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Failed);
            assert_eq!(summary.as_deref(), Some("用户已中止"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected abort timeline update, got {other:?}"),
    }
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            status,
            stage,
            active_node_id,
            ..
        } => {
            assert_eq!(status, CodingAttemptStatus::Aborted);
            assert_eq!(stage, CodingExecutionStage::Testing);
            assert!(active_node_id.is_none());
        }
        other => panic!("expected aborted coding session state, got {other:?}"),
    }
    let updated = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("updated attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Aborted);
    assert!(updated.completed_at.is_some());

    ws.close(None).await.expect("close ws");
    server.abort();
}

fn app_with_attempt(root_path: &std::path::Path) -> axum::Router {
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repo = root_path.join("repo");
    init_simple_git_repo(&repo);
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo,
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
        })
        .expect("create repository");
    LifecycleStore::new(app_paths.clone())
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Coding work item".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    let store = CodingAttemptStore::new(app_paths);
    store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    build_web_router(WebAppState::new(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
    ))
}

fn app_with_attempt_requiring_execution_plan_confirm(root_path: &std::path::Path) -> axum::Router {
    let app = app_with_attempt(root_path);
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut work_item = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("list work items")
        .into_iter()
        .find(|item| item.id == "work_item_0001")
        .expect("work item");
    work_item.require_execution_plan_confirm = true;
    let path = app_paths
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("work-items/work_item_0001.json");
    serde_json::to_writer_pretty(std::fs::File::create(path).expect("open"), &work_item)
        .expect("write work item");

    let store = CodingAttemptStore::new(app_paths);
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    store
        .save_work_item_execution_plan(&WorkItemExecutionPlan {
            id: "work_item_execution_plan_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_id: attempt.id.clone(),
            status: WorkItemExecutionPlanStatus::Draft,
            goal: work_item.title.clone(),
            allowed_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            dependency_handoffs: Vec::new(),
            story_refs: Vec::new(),
            design_refs: Vec::new(),
            openspec_refs: Vec::new(),
            superpowers_contract: String::new(),
            tdd_contract: String::new(),
            verification_plan_ref: None,
            verification_summary: None,
            risk_notes: Vec::new(),
            created_at: attempt.created_at.clone(),
            updated_at: attempt.updated_at.clone(),
        })
        .expect("save plan");
    app
}

fn app_with_confirmed_work_item_context(root_path: &std::path::Path) -> axum::Router {
    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repo = root_path.join("repo");
    init_simple_git_repo(&repo);
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo,
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
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
            title: "实现爬楼梯问题".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_0001".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("create workspace session");
    lifecycle
        .append_artifact_version(
            &session.id,
            ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::Markdown {
                    markdown: CLIMB_STAIRS_WORK_ITEM.to_string(),
                    diff: None,
                },
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: None,
                confirmed_by: Some("user".to_string()),
                is_current: true,
                created_at: "2026-05-28T00:00:00Z".to_string(),
                source_node_id: "node_0001".to_string(),
            },
        )
        .expect("append artifact version");
    lifecycle
        .update_workspace_session_status(&session.id, WorkspaceSessionStatus::Confirmed)
        .expect("confirm workspace session");
    CodingAttemptStore::new(app_paths)
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    build_web_router(WebAppState::new(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
    ))
}

fn app_with_full_chain_attempt(root_path: &Path) -> axum::Router {
    app_with_full_chain_attempt_and_provider(root_path, Arc::new(FullChainStreamingProvider))
}

fn app_with_full_chain_attempt_and_provider(
    root_path: &Path,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> axum::Router {
    let repo = root_path.join("repo");
    let remote = root_path.join("remote.git");
    init_cargo_repo(&repo);
    run_git(root_path, &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );

    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo,
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
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
    CodingAttemptStore::new(app_paths)
        .create_attempt(CreateCodingAttemptInput {
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
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");

    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, provider);
    build_web_router(WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    ))
}

