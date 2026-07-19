fn app_with_group_full_chain_attempt(root_path: &Path) -> axum::Router {
    app_with_group_full_chain_attempt_fixture(
        root_path,
        Arc::new(FullChainStreamingProvider),
        true,
    )
}

fn app_with_group_full_chain_attempt_and_provider(
    root_path: &Path,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> axum::Router {
    app_with_group_full_chain_attempt_fixture(root_path, provider, false)
}

fn app_with_group_full_chain_attempt_fixture(
    root_path: &Path,
    provider: Arc<dyn StreamingProviderAdapter>,
    use_existing_worktree: bool,
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
            path: repo.clone(),
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("fake".to_string()),
        })
        .expect("create repository");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id.clone(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "实现爬楼梯".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            sequence_hint: Some(10),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item 1");
    lifecycle
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
        .expect("create work item session 1");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "补充边界校验".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            sequence_hint: Some(20),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item 2");
    lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_0002".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("create work item session 2");
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("work_item_plan_0001".to_string()),
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
            dependency_graph: vec![cadence_aria::product::models::IssueWorkItemDependencyEdge {
                from_work_item_id: "work_item_0001".to_string(),
                to_work_item_id: "work_item_0002".to_string(),
            }],
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create work item plan");

    let store = CodingAttemptStore::new(app_paths);
    let attempt = create_legacy_group_coding_attempt_fixture(
        &store,
        CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: use_existing_worktree.then_some(repo),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        },
    );
    seed_authoritative_group_plan_fixture(&store, &attempt);
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Running,
        })
        .expect("create coding unit 1");
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id,
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            logical_work_item_id: "work_item_0002".to_string(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            dependency_logical_work_item_ids: vec!["work_item_0001".to_string()],
            order_index: 1,
            status: CodingExecutionUnitStatus::Pending,
        })
        .expect("create coding unit 2");

    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Fake, provider);
    build_web_router(WebAppState::with_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        registry,
    ))
}

struct GroupFinalReviewPlanDefectProvider {
    block_first_final_review: bool,
    final_review_calls: std::sync::atomic::AtomicUsize,
}

impl GroupFinalReviewPlanDefectProvider {
    fn normal() -> Self {
        Self {
            block_first_final_review: false,
            final_review_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn recovery() -> Self {
        Self {
            block_first_final_review: true,
            final_review_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GroupFinalReviewPlanDefectProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        start_web_test_provider_driven_testing_session(&input.prompt, cancel)
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        let full_output = match input.role {
            AdapterRole::Executor => {
                let worktree = input
                    .worktree_path
                    .as_ref()
                    .map(PathBuf::from)
                    .expect("worktree path");
                fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                })?;
                "implemented climb_stairs".to_string()
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_internal_pr_review_json" =>
            {
                let call = self
                    .final_review_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if self.block_first_final_review && call == 0 {
                    serde_json::json!({
                        "verdict": "blocked",
                        "summary": "retry group final review",
                        "findings": [],
                        "impact_scope": ["work_item_0002"],
                        "pr_description": "retry required",
                        "commit_message_suggestion": "fix: retry review"
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                    "verdict": "approve",
                    "summary": "group review found a plan defect",
                    "findings": [{
                        "source_stage": "group_final_review",
                        "severity": "error",
                        "defect_class": "current_work_item_invalid",
                        "reason_code": "current_work_item_contract_invalid",
                        "message": "the final unit contract is invalid",
                        "contract_refs": [],
                        "capability_refs": [],
                        "repair_target": {
                            "kind": "current_work_item",
                            "logical_work_item_ids": ["work_item_0002"],
                            "work_item_revision_ids": ["work_item_revision_0002"]
                        },
                        "recommended_route": "plan_repair",
                        "confidence": "high",
                        "evidence": []
                    }],
                    "impact_scope": ["work_item_0002"],
                    "pr_description": "plan repair required",
                    "commit_message_suggestion": "fix: repair plan"
                    })
                    .to_string()
                }
            }
            AdapterRole::Reviewer => {
                r#"{"verdict":"approve","summary":"review ok","findings":[]}"#.to_string()
            }
            _ => "ok".to_string(),
        };
        tx.try_send(StreamChunk::Done { full_output })
            .expect("send done");
        Ok(rx)
    }
}

fn materialize_completed_unit_run_for_logical(
    store: &CodingAttemptStore,
    logical_work_item_id: &str,
) -> cadence_aria::product::coding_models::CodingUnitRun {
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    let unit = units
        .iter()
        .find(|unit| unit.logical_work_item_id == logical_work_item_id)
        .expect("unit");
    let runs = store
        .list_coding_unit_runs(&attempt, &unit.id)
        .expect("unit runs");
    if let Some(run) = runs.iter().find(|run| {
        run.status == cadence_aria::product::coding_models::CodingUnitRunStatus::Completed
    }) {
        return run.clone();
    }
    let mut run = runs.last().expect("materialized unit run").clone();
    run.id = format!("coding_unit_run_completed_{}", logical_work_item_id);
    run.execution_no += 1;
    run.status = cadence_aria::product::coding_models::CodingUnitRunStatus::Completed;
    run.completion_commit = attempt.head_commit.clone();
    run.created_at = "2026-07-19T00:00:00Z".to_string();
    run.updated_at = run.created_at.clone();
    store
        .create_coding_unit_run(&attempt, &run)
        .expect("completed unit run");
    run
}

fn materialize_running_unit_run_for_logical(store: &CodingAttemptStore, logical_work_item_id: &str) {
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let unit = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == logical_work_item_id)
        .expect("unit");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, "work_item_plan_0001")
        .expect("plan lineage");
    let revision = revision_store
        .get_work_item_revision(&lineage, logical_work_item_id, &unit.work_item_revision_id)
        .expect("work item revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("projection bundle");
    let renderer_version = cadence_aria::product::work_item_projection::renderer_for(
        &ProviderName::Fake,
    )
    .renderer_version()
    .to_string();
    store
        .create_coding_unit_run(
            &attempt,
            &cadence_aria::product::coding_models::CodingUnitRun {
                id: format!("coding_unit_run_running_{logical_work_item_id}"),
                unit_id: unit.id,
                execution_no: 1,
                work_item_revision_id: revision.id,
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: bundle.canonical_contract_hash,
                projection_bundle_id: bundle.id,
                projection_compiler_version: bundle.compiler_version,
                coder_provider_renderer_version: renderer_version.clone(),
                reviewer_provider_renderer_version: renderer_version.clone(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: bundle.coder_projection_hash,
                reviewer_projection_hash: bundle.reviewer_projection_hash,
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status: cadence_aria::product::coding_models::CodingUnitRunStatus::Running,
                unit_rework_count: 0,
                verification_retry_count: 0,
                operational_retry_count: 0,
                plan_repair_count: 0,
                start_commit: attempt.head_commit.clone(),
                completion_commit: None,
                created_at: "2026-07-19T00:00:00Z".to_string(),
                updated_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .expect("running unit run");
}

fn bind_completed_first_unit_handoff_revision(store: &CodingAttemptStore) {
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("units");
    let first = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0001")
        .expect("first unit");
    let run = materialize_completed_unit_run_for_logical(store, "work_item_0001");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, "work_item_plan_0001")
        .expect("lineage");
    let handoff = revision_store
        .get_handoff_revision(
            &lineage,
            &first.logical_work_item_id,
            &format!("handoff_revision_{}", run.id),
        )
        .expect("canonical handoff revision");
    assert_eq!(handoff.coding_unit_run_id, run.id);
    store
        .update_coding_unit_completion_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &first.id,
            Some(handoff.commit_sha.clone()),
        )
        .expect("completion commit binding");
    store
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &first.id,
            Some(handoff.id),
        )
        .expect("handoff binding");
}

#[tokio::test]
async fn coding_plan_repair_group_final_review_normal_path_does_not_complete_approve_with_plan_finding()
{
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt_and_provider(
        root.path(),
        Arc::new(GroupFinalReviewPlanDefectProvider::normal()),
    );
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
    let mut saw_group_final_review = false;
    let mut bound_first_handoff = false;
    for _ in 0..520 {
        match timeout(Duration::from_secs(2), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate }) => {
                if gate.kind == CodingGateKind::StageGate
                    && let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    if stage == CodingExecutionStage::InternalPrReview {
                        materialize_completed_unit_run_for_logical(&store, "work_item_0002");
                    }
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            Ok(CodingWsOutMessage::CodingSessionState {
                current_work_item_id,
                ..
            }) if current_work_item_id.as_deref() == Some("work_item_0002")
                && !bound_first_handoff =>
            {
                bind_completed_first_unit_handoff_revision(&store);
                bound_first_handoff = true;
            }
            Ok(CodingWsOutMessage::InternalPrReviewComplete { review }) => {
                assert_eq!(review.verdict, ReviewVerdict::Approve);
                assert_eq!(review.findings.len(), 1);
                saw_group_final_review = true;
                break;
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) => {}
            Err(_) => {
                let attempt = store
                    .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("attempt after timeout");
                let gates = store
                    .list_open_blocked_gates("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("blocked gates");
                let requests = store
                    .list_review_requests("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("review requests");
                panic!(
                    "timed out before GroupFinalReview: status={:?} stage={:?} current={:?} gates={gates:?} requests={requests:?}",
                    attempt.status, attempt.stage, attempt.current_work_item_id,
                );
            }
        }
    }
    assert!(saw_group_final_review, "expected GroupFinalReview output");
    tokio::task::yield_now().await;
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_ne!(attempt.status, CodingAttemptStatus::Completed);
    assert_eq!(attempt.stage, CodingExecutionStage::InternalPrReview);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_group_attempt_completes_first_unit_before_review_request_and_resumes_next_unit() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt(root.path());
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
    let mut saw_second_unit_progress = false;
    for _ in 0..220 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if gate.kind == CodingGateKind::StageGate
                    && let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            CodingWsOutMessage::CodingSessionState {
                current_work_item_id,
                stage,
                ..
            } => {
                if current_work_item_id.as_deref() == Some("work_item_0002")
                    && stage != CodingExecutionStage::PrepareContext
                {
                    saw_second_unit_progress = true;
                    break;
                }
            }
            CodingWsOutMessage::ReviewRequestUpdate { .. } => {
                panic!("group attempt emitted review request before all units completed");
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        saw_second_unit_progress,
        "expected runner to resume the second unit after first unit completion"
    );
    assert!(
        store
            .list_review_requests("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("review requests")
            .is_empty(),
        "group attempt must not create final review request before all units complete"
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.current_work_item_id.as_deref(), Some("work_item_0002"));
    let units = store
        .list_coding_units("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("units");
    assert_eq!(units[0].status, CodingExecutionUnitStatus::Completed);
    assert_eq!(units[1].status, CodingExecutionUnitStatus::Running);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_group_attempt_recovers_review_request_running_unit_without_rerunning_review() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt(root.path());
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Running,
        )
        .expect("set running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingExecutionStage::ReviewRequest,
        )
        .expect("set review request stage");
    materialize_running_unit_run_for_logical(&store, "work_item_0001");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut saw_second_unit = false;
    for _ in 0..80 {
        match recv_json(&mut ws).await {
            CodingWsOutMessage::CodingGateRequired { gate } => {
                if gate.stage == Some(CodingExecutionStage::CodeReview) {
                    panic!("review_request recovery must not rerun CodeReviewer");
                }
            }
            CodingWsOutMessage::CodingSessionState {
                current_work_item_id,
                ..
            } => {
                if current_work_item_id.as_deref() == Some("work_item_0002") {
                    saw_second_unit = true;
                    break;
                }
            }
            CodingWsOutMessage::CodingProtocolError { code, message } => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            _ => {}
        }
    }

    assert!(
        saw_second_unit,
        "expected review_request recovery to advance to the next unit"
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.current_work_item_id.as_deref(), Some("work_item_0002"));
    let units = store
        .list_coding_units("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("units");
    assert_eq!(units[0].status, CodingExecutionUnitStatus::Completed);
    assert_eq!(units[1].status, CodingExecutionUnitStatus::Running);

    ws.close(None).await.expect("close ws");
    server.abort();
}

#[tokio::test]
async fn coding_ws_group_session_state_hides_completed_unit_handoff_from_active_unit_snapshot() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_group_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .save_coding_unit_handoff(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            "coding_unit_0001",
            &cadence_aria::product::coding_models::WorkItemHandoff {
                id: "work_item_handoff_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                attempt_id: "coding_attempt_0001".to_string(),
                provider_run_ref: None,
                summary: "unit1 done".to_string(),
                files_changed: Vec::new(),
                commit_sha: None,
                diff_summary: String::new(),
                tests_run: Vec::new(),
                test_result_summary: String::new(),
                review_summary: None,
                api_or_contract_changes: Vec::new(),
                open_risks: Vec::new(),
                next_work_item_notes: Vec::new(),
                created_at: "2026-06-27T00:00:00Z".to_string(),
            },
        )
        .expect("save unit1 handoff");
    store
        .update_coding_unit_latest_handoff_revision_id(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            "coding_unit_0001",
            Some("handoff_revision_0001".to_string()),
        )
        .expect("update unit1 handoff ref");
    store
        .update_coding_unit_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            "coding_unit_0001",
            CodingExecutionUnitStatus::Completed,
            Some("unit1 done".to_string()),
        )
        .expect("complete unit1");
    store
        .update_coding_unit_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            "coding_unit_0002",
            CodingExecutionUnitStatus::Running,
            Some("unit2 running".to_string()),
        )
        .expect("start unit2");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let state = match ws.next().await {
        Some(Ok(Message::Text(text))) => {
            serde_json::from_str::<serde_json::Value>(&text).expect("session state json")
        }
        other => panic!("expected text websocket message, got {other:?}"),
    };

    assert_eq!(state["current_work_item_id"], "work_item_0002");
    assert!(state["work_item_handoff"].is_null());
    assert_eq!(
        state["units"][0]["latest_handoff_revision_id"],
        "handoff_revision_0001"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
