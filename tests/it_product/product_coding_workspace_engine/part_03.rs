// ── 本组（part_03..06）共用：LC 权威 + 冻结快照 + LC provider gateway ──────
//
// REQ-ROUTE-01 两态化（去单仓入口）后：
// ① 无 LC 归属的 issue 一律 fail-closed ⇒ provider run 的租约三态判成
//    `coding_run_lease_unknown: … product_store_not_found: issue issue_0001`；
// ② 逻辑 target（带冻结 `target_snapshot`）的 provider run 唯一经
//    `LogicalCodebaseProviderGateway` 启动（否则 `logical_provider_gateway_required`）。
//
// 故本组用例统一三件套：`seed_lc_authority`（LC 权威）→
// `create_input_with_snapshot`（冻结快照）→ `p3_engine_with_logical_gateway`
// （gateway 注入；替身同时交给引擎方法与 registry，两处同一实例）。

/// 本组共用装配：替身同时交给引擎方法（断言捕获面）与 gateway registry
/// （真实 spawn 面，两处同一实例），并把 LC provider gateway 注入引擎。
fn p3_engine_with_logical_gateway(
    store: CodingAttemptStore,
    paths: &ProductAppPaths,
    authority: &LogicalAuthority,
    provider: Arc<dyn StreamingProviderAdapter>,
    provider_name: ProviderName,
    tx: mpsc::Sender<CodingWsOutMessage>,
) -> CodingWorkspaceEngine {
    let mut registry = ProviderRegistry::new();
    registry.register(provider_name, provider);
    let gateway = crate::context_builder::build_gateway(
        paths,
        &authority.project_id,
        &authority.logical_codebase_id,
        Arc::new(registry),
    );
    CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx)
        .with_logical_provider_gateway(Arc::new(gateway))
}

#[tokio::test]
async fn retry_code_review_prompt_includes_previous_role_run_diagnostic() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let authority = seed_lc_authority(&paths, root.path());
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            ..create_input_with_snapshot(&paths, &authority)
        })
        .expect("create attempt");
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");

    let first_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("first run");
    store
        .append_role_run_event(
            &attempt,
            &first_run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Code reviewer task update",
                "status": "blocked",
                "detail": "Review context was missing"
            }),
        )
        .expect("event");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &first_run.id,
            CodingRoleRunStatus::Blocked,
            Some("code_review_blocked".to_string()),
        )
        .expect("block first run");
    let resumed = crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
    let retry_run = store
        .supersede_latest_role_run_and_create(
            &resumed,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::RetryReview,
            None,
            Some("code_review_blocked".to_string()),
            None,
        )
        .expect("retry run");
    assert_eq!(
        retry_run.supersedes_run_id.as_deref(),
        Some(first_run.id.as_str())
    );

    let stub = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"approve","summary":"code review ok","findings":[]}"#],
        [None],
    );
    let captured_inputs = Arc::clone(&stub.inputs);
    let provider = Arc::new(crate::context_builder::GatewayValidatedStartAdapter {
        inner: Arc::new(stub),
        provider_type: ProviderType::ClaudeCode,
    });
    let (tx, _rx) = mpsc::channel(16);
    let engine = p3_engine_with_logical_gateway(
        store.clone(),
        &paths,
        &authority,
        provider.clone(),
        ProviderName::ClaudeCode,
        tx,
    );

    engine
        .execute_code_review(&resumed, provider.as_ref())
        .await
        .expect("execute retry code review");

    let inputs = captured_inputs.lock().expect("inputs");
    let prompt = &inputs.first().expect("first input").prompt;
    assert!(prompt.contains("[previous_role_run_diagnostic]"));
    assert!(prompt.contains("reason_code: code_review_blocked"));
    assert!(prompt.contains("Code reviewer task update"));
    assert!(prompt.contains("Review context was missing"));
    assert!(prompt.contains("最终审查结论仅输出裸 JSON"));
    assert!(!prompt.contains(&format!("role_run_id: {}", retry_run.id)));
}

#[tokio::test]
async fn coding_code_reviewer_run_records_session_without_resume() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\nreviewed\n").expect("modify file");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let authority = seed_lc_authority(&paths, root.path());
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Pi,
                reviewer: Some(ProviderName::Pi),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            ..create_input_with_snapshot(&paths, &authority)
        })
        .expect("create attempt");
    let config = CodingRoleProviderConfigSnapshot {
        coder: ProviderName::Pi,
        code_reviewer: Some(ProviderName::Pi),
        internal_reviewer: Some(ProviderName::Pi),
        review_rounds: 1,
        permission_modes: CodingRolePermissionModes {
            coder: CodingProviderPermissionMode::Supervised,
            code_reviewer: CodingProviderPermissionMode::Supervised,
            internal_reviewer: CodingProviderPermissionMode::Supervised,
        },
    };
    store
        .update_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id, config)
        .expect("set Pi role config");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::Pi,
                    provider_session_id: "coder-session-1".to_string(),
                    updated_at: "2026-06-01T00:00:00Z".to_string(),
                    last_node_id: Some("coding-node-1".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::CodeReviewer,
                    provider: ProviderName::Pi,
                    provider_session_id: "reviewer-session-1".to_string(),
                    updated_at: "2026-06-01T00:01:00Z".to_string(),
                    last_node_id: Some("testing-node-1".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::CodeReviewer,
                    provider: ProviderName::Pi,
                    provider_session_id: "code-reviewer-session-0".to_string(),
                    updated_at: "2026-06-01T00:02:00Z".to_string(),
                    last_node_id: Some("code-review-node-0".to_string()),
                },
            ],
        )
        .expect("persist conversations");
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
    let (tx, _rx) = mpsc::channel(8);
    let stub = SessionInputCapturingProvider::with_outputs(
        [r#"{"verdict":"approve","summary":"code review ok","findings":[]}"#],
        [Some("code-reviewer-session-1".to_string())],
    );
    let captured_inputs = Arc::clone(&stub.inputs);
    let provider = Arc::new(crate::context_builder::GatewayExactInputAdapter {
        inner: Arc::new(stub),
    });
    let engine = p3_engine_with_logical_gateway(
        store.clone(),
        &paths,
        &authority,
        provider.clone(),
        ProviderName::Pi,
        tx,
    );

    let report = engine
        .execute_code_review(&attempt, provider.as_ref())
        .await
        .expect("code review provider run");

    assert_eq!(report.verdict, ReviewVerdict::Approve);
    let inputs = captured_inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].permission_mode, ProviderPermissionMode::Auto);
    assert_eq!(inputs[0].provider_type, ProviderType::Pi);
    assert_eq!(inputs[0].timeout_secs, 10_800);
    // 契约(仅 Coder 可 resume):reviewer 每轮 fresh spawn,历史
    // CodeReviewer 会话不进 resume 通道(窗口 id 进入 LC 全投影,
    // 跨轮 resume 必然审计漂移)。
    assert_eq!(inputs[0].resume_provider_session_id, None);
    let updated = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("updated attempt");
    assert!(updated.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::CodeReviewer
            && conversation.provider == ProviderName::Pi
            && conversation.provider_session_id == "code-reviewer-session-1"
    }));
}

#[tokio::test]
async fn coding_internal_reviewer_uses_fresh_provider_session() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    fs::write(worktree.join("src.txt"), "hello\ninternal review\n").expect("modify file");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let authority = seed_lc_authority(&paths, root.path());
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Pi,
                reviewer: Some(ProviderName::Pi),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            ..create_input_with_snapshot(&paths, &authority)
        })
        .expect("create attempt");
    let config = CodingRoleProviderConfigSnapshot {
        coder: ProviderName::Pi,
        code_reviewer: Some(ProviderName::Pi),
        internal_reviewer: Some(ProviderName::Pi),
        review_rounds: 1,
        permission_modes: CodingRolePermissionModes {
            coder: CodingProviderPermissionMode::Supervised,
            code_reviewer: CodingProviderPermissionMode::Supervised,
            internal_reviewer: CodingProviderPermissionMode::Supervised,
        },
    };
    store
        .update_role_provider_config_snapshot("project_0001", "issue_0001", &attempt.id, config)
        .expect("set Pi role config");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::InternalReviewer,
                provider: ProviderName::Pi,
                provider_session_id: "internal-reviewer-session-1".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("internal-review-node-1".to_string()),
            }],
        )
        .expect("persist internal reviewer conversation");
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("review request stage");
    store
        .save_review_request(&attempt, &sample_review_request(&attempt.id))
        .expect("save review request");
    let (tx, _rx) = mpsc::channel(8);
    let stub = SessionInputCapturingProvider::with_outputs(
        [
            r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#,
        ],
        [Some("internal-reviewer-session-2".to_string())],
    );
    let captured_inputs = Arc::clone(&stub.inputs);
    let provider = Arc::new(crate::context_builder::GatewayExactInputAdapter {
        inner: Arc::new(stub),
    });
    let engine = p3_engine_with_logical_gateway(
        store.clone(),
        &paths,
        &authority,
        provider.clone(),
        ProviderName::Pi,
        tx,
    );

    let review = engine
        .execute_internal_pr_review(&attempt, provider.as_ref())
        .await
        .expect("internal reviewer provider run");

    assert_eq!(review.verdict, ReviewVerdict::Approve);
    let inputs = captured_inputs.lock().expect("inputs lock");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].permission_mode, ProviderPermissionMode::Auto);
    assert_eq!(inputs[0].provider_type, ProviderType::Pi);
    assert_eq!(inputs[0].timeout_secs, 10_800);
    assert_eq!(inputs[0].resume_provider_session_id, None);
}

#[tokio::test]
async fn execute_coding_includes_work_item_context_in_provider_prompt() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let authority = seed_lc_authority(&paths, root.path());
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            ..create_input_with_snapshot(&paths, &authority)
        })
        .expect("create attempt");
    crate::seed_coding_attempt_running(&store, "project_0001", "issue_0001", &attempt.id);
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = Arc::new(crate::context_builder::GatewayBridgedAdapter::new(
        Arc::new(PromptCapturingProvider {
            prompt: Arc::clone(&captured_prompt),
        }),
        ProviderType::ClaudeCode,
    ));
    let (tx, _rx) = mpsc::channel(8);
    let engine = p3_engine_with_logical_gateway(
        store.clone(),
        &paths,
        &authority,
        provider.clone(),
        ProviderName::ClaudeCode,
        tx,
    );
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# 爬楼梯问题 Work Item\n\n## 验证命令\n\n- `uv run python -m unittest -v tests.test_climbing_stairs`"
                .to_string(),
        ),
        verification_commands: vec![
            "uv run python -m unittest -v tests.test_climbing_stairs".to_string(),
        ],
    };

    engine
        .execute_coding(&attempt, provider.as_ref(), &context)
        .await
        .expect("execute coding");

    let prompt = captured_prompt
        .lock()
        .expect("prompt lock")
        .clone()
        .expect("captured prompt");
    assert!(prompt.contains("不要只输出计划或 Story/Design/Work Item 文档"));
    assert!(prompt.contains("# 爬楼梯问题 Work Item"));
    assert!(prompt.contains("uv run python -m unittest -v tests.test_climbing_stairs"));
}
