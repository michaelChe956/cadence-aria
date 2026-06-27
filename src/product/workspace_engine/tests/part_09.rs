#[tokio::test]
async fn drive_work_item_plan_provider_session_returns_output_and_persists_stream() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_stream_collector");
    engine.session.session_id = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .find(|session| session.workspace_type == WorkspaceType::WorkItemPlan)
        .expect("work item plan session")
        .id;
    let node_id = engine.begin_work_item_plan_author_run().await;
    let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
    let (provider_command_tx, _provider_command_rx) = mpsc::channel(8);
    provider_event_tx
        .send(ProviderEvent::TextDelta {
            content: "Fake Work Item Plan streaming draft\n".to_string(),
        })
        .await
        .expect("send text delta");
    provider_event_tx
        .send(ProviderEvent::Completed {
            full_output: "Final structured output".to_string(),
            provider_session_id: Some("provider-work-item-plan-author-1".to_string()),
        })
        .await
        .expect("send completed");
    drop(provider_event_tx);
    let mut command_rx = empty_provider_commands();

    let output = engine
        .drive_work_item_plan_provider_session_to_output(
            Ok(ProviderSession {
                events: provider_event_rx,
                commands: provider_command_tx,
            }),
            &mut command_rx,
            node_id.clone(),
            ProviderName::ClaudeCode,
        )
        .await
        .expect("collector output");

    assert_eq!(output, "Final structured output");
    let detail = lifecycle
        .load_node_detail(engine.session().session_id.as_str(), &node_id)
        .expect("node detail");
    assert!(
        detail
            .streaming_content
            .contains("Fake Work Item Plan streaming draft")
    );
    assert!(
        engine
            .session()
            .provider_conversations
            .iter()
            .any(|conversation| {
                conversation.role == ProviderConversationRole::Author
                    && conversation.provider == ProviderName::ClaudeCode
                    && conversation.provider_session_id == "provider-work-item-plan-author-1"
                    && conversation.last_node_id.as_deref() == Some(node_id.as_str())
            })
    );
}

#[tokio::test]
async fn drive_work_item_plan_provider_session_hides_structured_output_from_stream() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_stream_filter");
    engine.session.session_id = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .find(|session| session.workspace_type == WorkspaceType::WorkItemPlan)
        .expect("work item plan session")
        .id;
    let node_id = engine.begin_work_item_plan_author_run().await;
    let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
    let (provider_command_tx, _provider_command_rx) = mpsc::channel(8);
    let full_output = "Readable Work Item Plan draft\n<ARIA_STRUCTURED_OUTPUT>{\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>".to_string();
    provider_event_tx
        .send(ProviderEvent::TextDelta {
            content: "Readable Work Item Plan draft\n<ARIA_STRUCTURED".to_string(),
        })
        .await
        .expect("send text delta");
    provider_event_tx
        .send(ProviderEvent::TextDelta {
            content: "_OUTPUT>{\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>".to_string(),
        })
        .await
        .expect("send structured delta");
    provider_event_tx
        .send(ProviderEvent::Completed {
            full_output: full_output.clone(),
            provider_session_id: None,
        })
        .await
        .expect("send completed");
    drop(provider_event_tx);
    let mut command_rx = empty_provider_commands();

    let output = engine
        .drive_work_item_plan_provider_session_to_output(
            Ok(ProviderSession {
                events: provider_event_rx,
                commands: provider_command_tx,
            }),
            &mut command_rx,
            node_id.clone(),
            ProviderName::ClaudeCode,
        )
        .await
        .expect("collector output");

    assert_eq!(output, full_output);
    let detail = lifecycle
        .load_node_detail(engine.session().session_id.as_str(), &node_id)
        .expect("node detail");
    assert!(
        detail
            .streaming_content
            .contains("Readable Work Item Plan draft")
    );
    assert!(!detail.streaming_content.contains("ARIA_STRUCTURED_OUTPUT"));
    assert!(!detail.streaming_content.contains("\"work_items\""));
}

#[test]
fn build_work_item_plan_streaming_input_uses_splitter_role() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_splitter_input");

    let input = engine.build_work_item_plan_streaming_input(
        ProviderType::Fake,
        "split prompt".to_string(),
        "/tmp/worktree".to_string(),
        ProviderName::Fake,
    );

    assert_eq!(input.provider_type, ProviderType::Fake);
    assert_eq!(input.role, AdapterRole::WorkItemSplitter);
    assert_eq!(input.prompt, "split prompt");
    assert_eq!(input.working_dir, PathBuf::from("/tmp/worktree"));
    assert_eq!(
        input.workspace_session_id.as_deref(),
        Some(engine.session().session_id.as_str())
    );
    assert_eq!(input.resume_provider_session_id, None);
    assert_eq!(input.permission_mode, ProviderPermissionMode::Supervised);
    assert_eq!(input.timeout_secs, DEFAULT_PROVIDER_TIMEOUT_SECS);
}

#[test]
fn build_work_item_plan_streaming_input_reuses_author_provider_session() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_resume_author");
    engine.session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "author-session-1".to_string(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        last_node_id: Some("node-1".to_string()),
    }];

    let input = engine.build_work_item_plan_streaming_input(
        ProviderType::ClaudeCode,
        "split prompt".to_string(),
        "/tmp/worktree".to_string(),
        ProviderName::ClaudeCode,
    );

    assert_eq!(
        input.resume_provider_session_id,
        Some("author-session-1".to_string()),
        "should reuse persisted author provider session id"
    );
}

#[test]
fn work_item_plan_outline_revision_feedback_assembles_review_and_context() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_feedback");
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "拆分粒度太粗".to_string(),
        summary: "需要细化 outline".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "backend outline 缺少 exclusive_write_scope".to_string(),
            evidence: "outline 中 backend 项 exclusive_write_scopes 为空".to_string(),
            impact: "会导致 draft 阶段写入冲突".to_string(),
            required_action: "为 backend outline 补充 exclusive_write_scope".to_string(),
        }],
        review_gate: ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
    });

    let feedback = engine
        .work_item_plan_outline_revision_feedback(Some("用户补充：请把 frontend 再拆成两个"))
        .expect("feedback should be assembled");

    assert!(
        feedback.contains("需要细化 outline"),
        "missing summary: {feedback}"
    );
    assert!(
        feedback.contains("拆分粒度太粗"),
        "missing comments: {feedback}"
    );
    assert!(
        feedback.contains("backend outline 缺少 exclusive_write_scope"),
        "missing finding: {feedback}"
    );
    assert!(
        feedback.contains("用户补充：请把 frontend 再拆成两个"),
        "missing user context: {feedback}"
    );
}

#[test]
fn work_item_plan_outline_revision_feedback_returns_none_when_empty() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_feedback_empty");

    assert_eq!(
        engine.work_item_plan_outline_revision_feedback(None),
        None,
        "should return None when no review verdict and no context"
    );
    assert_eq!(
        engine.work_item_plan_outline_revision_feedback(Some("  ")),
        None,
        "should return None when context is whitespace only"
    );
}

fn make_work_item_plan_engine_with_draft_candidate(
    session_id: &str,
) -> (
    TempDir,
    Arc<CheckpointStore>,
    LifecycleStore,
    String,
    WorkspaceEngine,
) {
    let tmp = TempDir::new().unwrap();
    let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);

    let project_id = "project_0001";
    let issue_id = "issue_0001";
    let repository_id = "repo_0001";

    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.to_string(),
            title: "Story".to_string(),
        })
        .unwrap();
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
        })
        .unwrap();

    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            source_story_spec_ids: vec![story.id.clone()],
            source_design_spec_ids: vec![design.id.clone()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![],
            repository_profile_ref: None,
            verification_plan_ids: vec![],
            dependency_graph: vec![],
            created_from_provider_run: None,
            validator_findings: vec![],
        })
        .unwrap();

    let profile = lifecycle
        .create_repository_profile(CreateRepositoryProfileInput {
            id: Some("repo_profile_0001".to_string()),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.to_string(),
            provider_run_ref: None,
            languages: vec!["rust".to_string()],
            frameworks: vec!["axum".to_string()],
            package_managers: vec!["cargo".to_string()],
            test_frameworks: vec![],
            build_systems: vec!["cargo".to_string()],
            verification_capabilities: vec![],
            detected_layers: vec!["backend".to_string(), "frontend".to_string()],
            split_recommendation: "frontend_backend".to_string(),
            confidence: RepositoryProfileConfidence::High,
            uncertainties: vec![],
        })
        .unwrap();

    let work_item_1 = lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_spec_ids: vec![design.id.clone()],
            title: "Backend work item".to_string(),
            work_item_set_id: None,
            kind: WorkItemKind::Backend,
            sequence_hint: None,
            depends_on: vec![],
            exclusive_write_scopes: vec!["src/backend.rs".to_string()],
            forbidden_write_scopes: vec![],
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: vec![],
            verification_plan_ref: Some("vp_0001".to_string()),
            require_execution_plan_confirm: false,
            plan_status: WorkItemPlanStatus::Draft,
        })
        .unwrap();
    let work_item_2 = lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_spec_ids: vec![design.id.clone()],
            title: "Frontend work item".to_string(),
            work_item_set_id: None,
            kind: WorkItemKind::Frontend,
            sequence_hint: None,
            depends_on: vec!["work_item_0001".to_string()],
            exclusive_write_scopes: vec!["src/frontend.rs".to_string()],
            forbidden_write_scopes: vec![],
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: vec![],
            verification_plan_ref: Some("vp_0002".to_string()),
            require_execution_plan_confirm: false,
            plan_status: WorkItemPlanStatus::Draft,
        })
        .unwrap();

    let vp_1 = lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some("vp_0001".to_string()),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            work_item_id: work_item_1.id.clone(),
            repository_profile_ref: Some(profile.id.clone()),
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_001".to_string(),
                label: "cargo test".to_string(),
                command: "cargo test".to_string(),
                cwd: "".to_string(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: vec![VerificationManualCheck {
                id: "check_001".to_string(),
                label: "manual".to_string(),
                instructions: "check".to_string(),
                required: false,
            }],
            required_gates: vec!["cmd_001".to_string()],
            risk_notes: vec![],
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .unwrap();
    let vp_2 = lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some("vp_0002".to_string()),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            work_item_id: work_item_2.id.clone(),
            repository_profile_ref: Some(profile.id.clone()),
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_002".to_string(),
                label: "cargo test".to_string(),
                command: "cargo test".to_string(),
                cwd: "".to_string(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: vec![],
            required_gates: vec![],
            risk_notes: vec![],
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .unwrap();

    lifecycle
        .update_issue_work_item_plan(
            project_id,
            issue_id,
            &plan.id,
            IssueWorkItemPlanUpdate {
                work_item_ids: vec![work_item_1.id.clone(), work_item_2.id.clone()],
                verification_plan_ids: vec![vp_1.id.clone(), vp_2.id.clone()],
                repository_profile_ref: Some(profile.id.clone()),
                dependency_graph: vec![IssueWorkItemDependencyEdge {
                    from_work_item_id: work_item_1.id.clone(),
                    to_work_item_id: work_item_2.id.clone(),
                }],
                created_from_provider_run: None,
                validator_findings: vec![WorkItemSplitFinding {
                    severity: WorkItemSplitFindingSeverity::Warning,
                    code: "W001".to_string(),
                    message: "scope overlap risk".to_string(),
                    work_item_ids: vec![work_item_1.id.clone()],
                }],
            },
        )
        .unwrap();

    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .unwrap();

    let session = WorkspaceSession::from_record(session_record);
    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        checkpoint_store.clone(),
        lifecycle.clone(),
        event_tx,
        session,
    );
    engine.session.session_id = session_id.to_string();
    engine.session.stage = WorkspaceStage::AuthorConfirm;
    engine.session.reviewer_provider = Some(ProviderName::Codex);

    (tmp, checkpoint_store, lifecycle, plan.id, engine)
}
