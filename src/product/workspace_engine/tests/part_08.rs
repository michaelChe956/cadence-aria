#[tokio::test]
async fn handle_user_message_provider_error_returns_to_prepare_context() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let session = make_session("sess_008");
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .handle_user_message(
            "start".to_string(),
            Arc::new(ErrorStreamingProvider),
            empty_provider_commands(),
        )
        .await;

    let mut saw_error = false;
    let mut saw_prepare = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            EngineEvent::Error { message } if message == "provider unavailable" => {
                saw_error = true;
            }
            EngineEvent::StageChange { stage } if stage == "prepare_context" => {
                saw_prepare = true;
            }
            _ => {}
        }
    }
    assert!(saw_error);
    assert!(saw_prepare);
    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
    assert_eq!(engine.session().messages.len(), 1);
}

#[tokio::test]
async fn complete_work_item_plan_author_pushes_candidate_and_enters_author_confirm() {
    use crate::product::lifecycle_store::{
        CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
    };
    use crate::product::models::{
        IssueWorkItemDependencyEdge, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
        LifecycleWorkItemRecord, RepositoryProfile, RepositoryProfileConfidence,
        VerificationCommand, VerificationCommandSafety, VerificationCommandSource,
        VerificationFallbackPolicy, VerificationManualCheck, VerificationPlan, VerificationScope,
        WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind, WorkItemPlanStatus,
        WorkItemStatus,
    };
    use crate::product::work_item_split_engine::WorkItemSplitProviderOutput;

    let (_tmp, checkpoint_store) = setup();
    let app_root = tempfile::tempdir().expect("app root");
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(app_root.path().join(".aria")));
    let project_id = "project_0001";
    let issue_id = "issue_0001";
    let repository_id = "repo_0001";

    let story = lifecycle_store
        .create_story_spec(CreateStorySpecInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.to_string(),
            title: "Story".to_string(),
        })
        .unwrap();
    let design = lifecycle_store
        .create_design_spec(CreateDesignSpecInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
        })
        .unwrap();

    let plan = lifecycle_store
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

    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .unwrap();

    let session = WorkspaceSession::from_record(session_record);
    let (tx, _rx) = mpsc::channel(64);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);

    let now = chrono::Utc::now().to_rfc3339();
    let repository_profile = RepositoryProfile {
        id: "repo_profile_0001".to_string(),
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        repository_id: repository_id.to_string(),
        provider_run_ref: None,
        languages: vec!["rust".to_string()],
        frameworks: vec![],
        package_managers: vec!["cargo".to_string()],
        test_frameworks: vec![],
        build_systems: vec!["cargo".to_string()],
        verification_capabilities: vec![],
        detected_layers: vec!["backend".to_string()],
        split_recommendation: "backend_only".to_string(),
        confidence: RepositoryProfileConfidence::High,
        uncertainties: vec![],
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let work_items = vec![
        LifecycleWorkItemRecord {
            id: "wi_001".to_string(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_spec_ids: vec![design.id.clone()],
            title: "Backend work item".to_string(),
            plan_status: WorkItemPlanStatus::Draft,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            planned_handoff_summary: None,
            kind: WorkItemKind::Backend,
            sequence_hint: None,
            depends_on: vec![],
            exclusive_write_scopes: vec!["src/backend.rs".to_string()],
            forbidden_write_scopes: vec![],
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: vec![],
            verification_plan_ref: Some("vp_001".to_string()),
            require_execution_plan_confirm: false,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            handoff_summary_ref: None,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        LifecycleWorkItemRecord {
            id: "wi_002".to_string(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: repository_id.to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_spec_ids: vec![design.id.clone()],
            title: "Frontend work item".to_string(),
            plan_status: WorkItemPlanStatus::Draft,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            planned_handoff_summary: None,
            kind: WorkItemKind::Frontend,
            sequence_hint: None,
            depends_on: vec!["wi_001".to_string()],
            exclusive_write_scopes: vec!["src/frontend.rs".to_string()],
            forbidden_write_scopes: vec![],
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: vec![],
            verification_plan_ref: Some("vp_002".to_string()),
            require_execution_plan_confirm: false,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            handoff_summary_ref: None,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    ];

    let verification_plans = vec![
        VerificationPlan {
            id: "vp_001".to_string(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            work_item_id: "wi_001".to_string(),
            repository_profile_ref: Some("repo_profile_0001".to_string()),
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
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        VerificationPlan {
            id: "vp_002".to_string(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            work_item_id: "wi_002".to_string(),
            repository_profile_ref: Some("repo_profile_0001".to_string()),
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
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    ];

    let plan_record = IssueWorkItemPlan {
        id: plan.id.clone(),
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
        work_item_ids: vec!["wi_001".to_string(), "wi_002".to_string()],
        repository_profile_ref: Some("repo_profile_0001".to_string()),
        verification_plan_ids: vec!["vp_001".to_string(), "vp_002".to_string()],
        dependency_graph: vec![IssueWorkItemDependencyEdge {
            from_work_item_id: "wi_001".to_string(),
            to_work_item_id: "wi_002".to_string(),
        }],
        created_from_provider_run: None,
        validator_findings: vec![],
        review_summary: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let output = WorkItemSplitProviderOutput {
        repository_profile,
        plan: plan_record,
        work_items,
        verification_plans,
    };

    let outcome = engine
        .complete_work_item_plan_author(output)
        .await
        .expect("author completion should succeed");
    assert!(matches!(outcome, WorkItemPlanAuthorOutcome::AuthorConfirm));

    let artifact = engine
        .session()
        .artifact
        .as_ref()
        .expect("artifact should be set");
    assert!(matches!(
        artifact,
        ArtifactPayload::WorkItemPlanCandidate { .. }
    ));
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);

    if let ArtifactPayload::WorkItemPlanCandidate { candidate } = artifact {
        assert_eq!(candidate.work_items.len(), 2);
        assert_eq!(candidate.plan.id, plan.id);
    }
}

#[test]
fn build_work_item_plan_review_input_includes_trimmed_candidate_fields() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_review_prompt");

    let input = engine
        .build_work_item_plan_review_input()
        .expect("review input");

    assert_eq!(input.role, AdapterRole::Reviewer);
    assert_work_item_plan_boundary_rules(&input.prompt);
    assert!(
        input.prompt.contains("Work Item Plan"),
        "prompt 应含 workspace 类型标题"
    );
    assert!(input.prompt.contains("work_item_0001"));
    assert!(input.prompt.contains("work_item_0002"));
    assert!(input.prompt.contains("depends_on"));
    assert!(input.prompt.contains("exclusive_write_scopes"));
    assert!(input.prompt.contains("verification_plan_ref"));
    assert!(input.prompt.contains("dependency_graph"));
    assert!(
        input.prompt.contains("high"),
        "prompt 应含 repository_profile confidence"
    );
    assert!(input.prompt.contains("backend"));
    assert!(
        !input.prompt.contains("frameworks"),
        "prompt 不应含 repository_profile 的 frameworks 字段"
    );
    assert!(
        input
            .prompt
            .contains("\"verdict\":\"pass|revise|needs_human\"")
    );
    assert!(input.prompt.contains("\"summary\""));
    assert!(input.prompt.contains("\"findings\""));
}

#[test]
fn build_work_item_plan_outline_review_input_includes_boundary_rules() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_review_boundary");
    let outline_payload = work_item_plan_outline_artifact();
    let ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } = outline_payload else {
        panic!("expected outline candidate artifact");
    };

    let input = engine
        .build_work_item_plan_outline_review_input(&outline_candidate)
        .expect("outline review input");

    assert_work_item_plan_boundary_rules(&input.prompt);
    assert!(input.prompt.contains("estimated_context_tokens"));
    assert!(input.prompt.contains("session_fit"));
    assert!(input.prompt.contains("单个 Claude Code 或 Codex coding 会话"));
    assert!(input.prompt.contains("小于 20k"));
}

#[tokio::test]
async fn build_work_item_draft_review_input_includes_boundary_rules() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_draft_review_boundary");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    let draft_payload =
        work_item_draft_artifact_payload(&plan_id, "outline_a", "draft_a", WorkItemDraftStatus::Draft);
    let ArtifactPayload::WorkItemDraftCandidate { draft_candidate } = draft_payload else {
        panic!("expected draft candidate artifact");
    };

    let input = engine
        .build_work_item_draft_review_input(&draft_candidate)
        .expect("draft review input");

    assert_work_item_plan_boundary_rules(&input.prompt);
}

#[tokio::test]
async fn build_work_item_batch_review_input_includes_boundary_rules() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_batch_review_boundary");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);

    let input = engine
        .build_work_item_batch_review_input()
        .expect("batch review input");

    assert_work_item_plan_boundary_rules(&input.prompt);
}

fn assert_work_item_plan_boundary_rules(prompt: &str) {
    assert!(
        prompt.contains("[artifact_boundary_must_fix_rules]"),
        "WorkItemPlan review prompt should include artifact boundary rules: {prompt}"
    );
    assert!(
        prompt.contains("Work Item Plan artifact"),
        "WorkItemPlan review prompt should include plan-specific boundary wording: {prompt}"
    );
    assert!(
        prompt.contains("must_fix"),
        "WorkItemPlan review prompt should classify boundary violations as must_fix: {prompt}"
    );
}

#[test]
fn build_review_input_routes_work_item_plan_to_dedicated_helper() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_review_route");

    let input = engine.build_review_input().expect("review input");

    assert_eq!(input.role, AdapterRole::Reviewer);
    assert!(input.prompt.contains("work_item_0001"));
    assert!(
        !input.prompt.contains("当前已提取 Artifact Markdown"),
        "WorkItemPlan 分支不应走 Story/Design 的 artifact markdown 提示"
    );
}

#[test]
fn build_work_item_draft_review_input_requires_verdict_and_severity_consistency() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_draft_review_prompt_gate");
    let store = WorkItemPlanStore::new(lifecycle.app_paths());
    let outline_id = "outline_backend";
    let round_id = "round_0001";
    let draft_id = "draft_0001";
    let now = chrono::Utc::now().to_rfc3339();
    let outline = WorkItemPlanOutline {
        id: "outline_artifact_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        source_story_spec_ids: vec!["story_spec_0001".to_string()],
        source_design_spec_ids: vec!["design_spec_0001".to_string()],
        strategy_summary: "serial backend split".to_string(),
        work_item_outlines: vec![WorkItemOutline {
            outline_id: outline_id.to_string(),
            title: "Backend".to_string(),
            kind: WorkItemKind::Backend,
            goal: "实现后端能力".to_string(),
            scope: vec!["实现 src/backend.rs".to_string()],
            non_goals: vec![],
            estimated_context_tokens: Some(12_000),
            session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
            source_story_spec_ids: vec!["story_spec_0001".to_string()],
            source_design_spec_ids: vec!["design_spec_0001".to_string()],
            exclusive_write_scopes: vec!["src/backend.rs".to_string()],
            forbidden_write_scopes: vec![],
            depends_on: vec![],
            verification_intent: vec!["cargo test".to_string()],
            handoff_notes: "handoff".to_string(),
        }],
        dependency_graph: vec![],
        risks: vec![],
        handoff_strategy: "serial".to_string(),
        status: "draft".to_string(),
    };
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline,
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some(round_id.to_string()),
            selected_generation_mode: Some(WorkItemGenerationModeDto::Serial),
        }),
    });
    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: plan_id.clone(),
            current_generation_round_id: round_id.to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: Some(outline_id.to_string()),
            outline_to_current_draft_id: BTreeMap::new(),
            draft_statuses: BTreeMap::new(),
            batches: vec![],
            updated_at: now.clone(),
        })
        .expect("save active index");
    let draft_payload = WorkItemDraftCandidatePayload {
        draft_record: WorkItemDraftRecord {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id,
            draft_id: draft_id.to_string(),
            outline_id: outline_id.to_string(),
            generation_round_id: round_id.to_string(),
            batch_id: None,
            attempt_index: 1,
            outline_version_ref: "outline_artifact_0001".to_string(),
            generation_mode: WorkItemGenerationMode::Serial,
            candidate: WorkItemDraftCandidate {
                outline_id: outline_id.to_string(),
                title: "Backend".to_string(),
                kind: WorkItemKind::Backend,
                goal: "实现后端能力".to_string(),
                implementation_context: "实现 src/backend.rs".to_string(),
                exclusive_write_scopes: vec!["src/backend.rs".to_string()],
                forbidden_write_scopes: vec![],
                depends_on_outline_ids: vec![],
                required_handoff_from_outline_ids: vec![],
                handoff_summary: "handoff".to_string(),
                verification_plan: serde_json::json!({
                    "commands": [],
                    "manual_checks": [],
                    "required_gates": []
                }),
            },
            status: WorkItemDraftStatus::Draft,
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id: "timeline_node_001".to_string(),
            accepted_at: None,
            superseded_at: None,
            created_at: now.clone(),
            updated_at: now,
        },
        validator_findings: vec![],
        can_accept: true,
    };

    let input = engine
        .build_work_item_draft_review_input(&draft_payload)
        .expect("draft review input");

    assert!(input.prompt.contains("blocking|must_fix|strong_recommend_fix"));
    assert!(input.prompt.contains("suggestion|minor|optional"));
    assert!(
        input
            .prompt
            .contains("不要输出 `verdict=pass` 同时给出 blocking/must_fix/strong_recommend_fix finding")
    );
    assert!(
        input
            .prompt
            .contains("如果问题只需当前 item author 修改，必须返回 `revise`")
    );
}

#[test]
fn build_work_item_plan_review_input_returns_error_when_lifecycle_store_missing() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_review_no_lifecycle");
    engine.lifecycle_store = None;

    let result = engine.build_work_item_plan_review_input();

    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(
        error.contains("lifecycle_store unavailable"),
        "错误信息应提示 lifecycle_store 不可用，实际为: {error}"
    );
}

#[tokio::test]
async fn begin_work_item_plan_author_run_creates_standard_author_node() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_author_stream_node");

    let node_id = engine.begin_work_item_plan_author_run().await;
    let node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .expect("author node");

    assert_eq!(node.node_type, TimelineNodeType::AuthorRun);
    assert_eq!(node.stage, WsWorkspaceStage::Running);
    assert_eq!(node.agent, Some(ProviderName::ClaudeCode));
    assert_eq!(node.status, TimelineNodeStatus::Active);
    assert_eq!(node.title, "Work Item Plan 生成");
    assert_eq!(
        engine.active_timeline_node_id().as_deref(),
        Some(node_id.as_str())
    );
}
