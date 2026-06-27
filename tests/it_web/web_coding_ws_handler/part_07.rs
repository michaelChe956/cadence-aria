const CLIMB_STAIRS_WORK_ITEM: &str = r#"# 实现爬楼梯问题 Work Item

请使用 python 实现函数 `climb_stairs(n: i32) -> i32`，覆盖 n=1、n=2、n=3、n=5、n=10。

## 验证命令

```bash
uv run python -m unittest discover -s tests -v
```
"#;

struct RetryAnalystCaptureProvider {
    captured_prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RetryAnalystCaptureProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.captured_prompts
            .lock()
            .expect("lock")
            .push(input.prompt.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Done {
            full_output: r#"{"verdict":"proceed","next_stage":"code_review","reason":"retry analyst accepted from test","evidence_refs":["artifacts/rework/analyst_evidence_0001.txt"],"raw_provider_output_refs":[]}"#.to_string(),
        })
        .expect("send done");
        Ok(rx)
    }
}

struct RetryInternalReviewCaptureProvider {
    captured_prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RetryInternalReviewCaptureProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.captured_prompts
            .lock()
            .expect("lock")
            .push(input.prompt.clone());
        let (tx, rx) = mpsc::channel(8);
        let full_output = if input.output_schema == "coding_workspace_internal_pr_review_json" {
            r#"{"verdict":"approve","summary":"internal reviewer retry accepted","findings":[],"impact_scope":["src"],"pr_description":"PR body","commit_message_suggestion":"feat: work"}"#
        } else if input.output_schema == "coding_workspace_analyst_verdict_json" {
            r#"{"verdict":"proceed","next_stage":"final_confirm","reason":"internal reviewer retry accepted"}"#
        } else {
            r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
        };
        tx.try_send(StreamChunk::Done {
            full_output: full_output.to_string(),
        })
        .expect("send done");
        Ok(rx)
    }
}

fn app_with_blocked_analyst_attempt(
    root_path: &Path,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> (axum::Router, CodingAttemptStore) {
    let repo = root_path.join("repo");
    init_cargo_repo(&repo);

    let app_paths = ProductAppPaths::new(root_path.join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "repo".to_string(),
            path: repo.clone(),
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
    let store = CodingAttemptStore::new(app_paths.clone());
    store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(repo),
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
    let router = build_web_router(WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    ));
    (router, store)
}

#[tokio::test]
async fn coding_ws_retry_analyst_resumes_rework_from_persisted_evidence() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (app, store) = app_with_blocked_analyst_attempt(
        root.path(),
        Arc::new(RetryAnalystCaptureProvider {
            captured_prompts: captured.clone(),
        }),
    );

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Blocked,
        )
        .expect("block attempt");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingExecutionStage::Rework,
        )
        .expect("set rework stage");

    let first_run = store
        .create_role_run(
            &store
                .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("get attempt"),
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("create first run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("analyst_human_gate".to_string()),
        )
        .expect("block first run");

    fs::create_dir_all(
        root.path()
            .join(".aria")
            .join("projects")
            .join("project_0001")
            .join("issues")
            .join("issue_0001")
            .join("coding-attempts")
            .join("coding_attempt_0001")
            .join("artifacts")
            .join("rework"),
    )
    .expect("create evidence dir");
    fs::write(
        root.path()
            .join(".aria")
            .join("projects")
            .join("project_0001")
            .join("issues")
            .join("issue_0001")
            .join("coding-attempts")
            .join("coding_attempt_0001")
            .join("artifacts")
            .join("rework")
            .join("analyst_evidence_0001.txt"),
        "persisted testing evidence",
    )
    .expect("write evidence");
    store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            &first_run.id,
            Vec::new(),
            vec!["artifacts/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("add evidence ref");
    store
        .append_role_run_event(
            &store
                .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("get attempt"),
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Analyst task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("append analyst event");

    store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: "coding_attempt_0001".to_string(),
            stage: CodingExecutionStage::Rework,
            node_id: Some("coding_node_0001".to_string()),
            role: Some(CodingProviderRole::Analyst),
            title: "Analyst human gate".to_string(),
            description: "需要重跑 Analyst".to_string(),
            reason_code: Some("analyst_human_gate".to_string()),
            evidence_refs: vec!["artifacts/rework/analyst_evidence_0001.txt".to_string()],
            raw_provider_output_ref: None,
            available_actions: vec![CodingGateAction {
                action_id: "retry_analyst".to_string(),
                label: "重试 Analyst".to_string(),
                action_type: CodingGateActionType::RetryAnalyst,
            }],
        })
        .expect("create gate");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "retry_analyst".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut saw_rework_node = false;
    for _ in 0..240 {
        match timeout(Duration::from_millis(500), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                if node.stage == CodingExecutionStage::Rework =>
            {
                saw_rework_node = true;
            }
            Ok(CodingWsOutMessage::CodingSessionState { ref stage, .. })
                if saw_rework_node && stage == &CodingExecutionStage::CodeReview =>
            {
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected protocol error {code}: {message}");
            }
            _ => {}
        }
    }
    assert!(saw_rework_node, "expected new rework timeline node");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    {
        let prompts = captured.lock().expect("lock");
        let prompt = prompts
            .iter()
            .find(|prompt| prompt.contains("persisted testing evidence"))
            .expect("expected analyst prompt to contain persisted evidence");
        assert!(prompt.contains("[previous_role_run_diagnostic]"));
        assert!(prompt.contains("Analyst task update"));
        assert!(prompt.contains("No tasks found"));
        assert_eq!(prompt.matches("[previous_role_run_diagnostic]").count(), 1);
        assert!(!prompt.contains(&format!("role_run_id: {}", runs[1].id)));
    }

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_retry_internal_review_resumes_internal_reviewer_run() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let (app, store) = app_with_blocked_analyst_attempt(
        root.path(),
        Arc::new(RetryInternalReviewCaptureProvider {
            captured_prompts: captured.clone(),
        }),
    );

    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingExecutionStage::InternalPrReview,
        )
        .expect("set internal review stage");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Blocked,
        )
        .expect("block attempt");
    store
        .save_review_request(&ReviewRequest {
            id: "review_request_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind: RemoteKind::GenericGit,
            remote: "origin".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            commit_sha: "abc123".to_string(),
            push_status: PushStatus::Pushed,
            external_url: None,
            manual_instructions: Vec::new(),
            created_at: "2026-06-13T00:00:00Z".to_string(),
            updated_at: "2026-06-13T00:00:00Z".to_string(),
        })
        .expect("save review request");
    let first_run = store
        .create_role_run(
            &store
                .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("get attempt"),
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("create first run");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("internal_review_blocked".to_string()),
        )
        .expect("block first run");
    store
        .append_role_run_event(
            &store
                .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                .expect("get attempt"),
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Internal reviewer task update",
                "status": "blocked",
                "detail": "internal_review_blocked"
            }),
        )
        .expect("append internal reviewer event");
    store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: "coding_attempt_0001".to_string(),
            stage: CodingExecutionStage::InternalPrReview,
            node_id: Some("coding_node_0001".to_string()),
            role: Some(CodingProviderRole::InternalReviewer),
            title: "Internal Reviewer blocked".to_string(),
            description: "需要重跑 Internal Reviewer".to_string(),
            reason_code: Some("internal_review_blocked".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
            available_actions: vec![CodingGateAction {
                action_id: "retry_internal_review".to_string(),
                label: "重试 Internal Reviewer".to_string(),
                action_type: CodingGateActionType::RetryInternalReview,
            }],
        })
        .expect("create gate");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "retry_internal_review".to_string(),
            extra_context: None,
        },
    )
    .await;

    let mut saw_internal_node = false;
    let mut saw_internal_complete = false;
    let mut saw_internal_chat = false;
    for _ in 0..40 {
        match timeout(Duration::from_millis(250), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                if node.stage == CodingExecutionStage::InternalPrReview =>
            {
                saw_internal_node = true;
            }
            Ok(CodingWsOutMessage::CodingTimelineNodeCreated { node })
                if node.stage == CodingExecutionStage::CodeReview =>
            {
                panic!("retry_internal_review should not resume CodeReview first");
            }
            Ok(CodingWsOutMessage::InternalPrReviewComplete { review }) => {
                assert_eq!(review.summary, "internal reviewer retry accepted");
                saw_internal_complete = true;
            }
            Ok(CodingWsOutMessage::CodingChatEntryCreated { entry })
                if entry.content.as_deref().is_some_and(|content| {
                    content.contains("internal reviewer retry accepted")
                }) =>
            {
                let metadata = entry.metadata.unwrap_or_default();
                assert_eq!(
                    metadata.get("source").and_then(|value| value.as_str()),
                    Some("internal_pr_review")
                );
                saw_internal_chat = true;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected protocol error {code}: {message}");
            }
            _ => {}
        }
        if saw_internal_node && saw_internal_complete && saw_internal_chat {
            break;
        }
    }
    assert!(saw_internal_node, "expected new internal review node");
    assert!(
        saw_internal_complete,
        "expected internal reviewer completion"
    );
    assert!(
        saw_internal_chat,
        "expected readable internal reviewer chat"
    );

    {
        let prompts = captured.lock().expect("lock");
        assert!(
            prompts
                .iter()
                .any(|prompt| prompt.contains("review_request_0001")
                    && prompt.contains("[previous_role_run_diagnostic]")
                    && prompt.contains("internal_review_blocked")),
            "expected internal reviewer prompt to contain review request and retry diagnostic"
        );
    }

    let runs = store
        .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("role runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Superseded);
    assert_eq!(runs[1].role, CodingProviderRole::InternalReviewer);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::RetryInternalReview);
    assert_eq!(runs[1].status, CodingRoleRunStatus::Completed);
    let reviews = store
        .list_internal_pr_reviews("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("internal reviews");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].role_run_id.as_deref(), Some(runs[1].id.as_str()));

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_blocks_coder_stage_when_execution_plan_requires_confirmation() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_attempt_requiring_execution_plan_confirm(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut code = None;
    for _ in 0..20 {
        let result = timeout(Duration::from_secs(5), recv_json(&mut ws)).await;
        match result {
            Ok(CodingWsOutMessage::CodingProtocolError { code: c, .. }) => {
                code = Some(c);
                break;
            }
            Ok(_) => continue,
            Err(_) => panic!("timed out waiting for execution plan error"),
        }
    }
    assert_eq!(
        code,
        Some("work_item_execution_plan_not_confirmed".to_string())
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
