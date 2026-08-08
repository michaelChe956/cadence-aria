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
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            title: "coding ws group fixture".to_string(),
            description: None,
            change_id: None,
        })
        .expect("create issue");
    assert_eq!(issue.id, "issue_0001");
    let lifecycle = LifecycleStore::new(app_paths.clone());
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
        .expect("create schema v2 plan metadata");
    let plan_session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("create work item plan session");
    seed_group_revision_history_fixture(&lifecycle, &plan_session.id);

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
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        },
    );
    seed_authoritative_group_plan_fixture(&store, &attempt, false);
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

#[tokio::test]
async fn schema_v2_group_coding_websocket_initializes_without_legacy_work_items() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_group_full_chain_attempt(root.path());
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .expect("list legacy work items")
            .is_empty(),
        "Schema v2 Group fixture must not create Legacy Work Item records"
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let initial = recv_json(&mut ws).await;
    assert!(matches!(
        initial,
        CodingWsOutMessage::CodingSessionState {
            attempt_scope,
            work_item_group_id: Some(_),
            ..
        } if attempt_scope == "work_item_group"
    ));
    ws.close(None).await.expect("close ws");
    server.abort();
}

struct IndependentCodeReviewPlanDefectProvider {
    block_first_code_review: bool,
    emit_plan_defect: bool,
    code_review_calls: std::sync::atomic::AtomicUsize,
}

impl IndependentCodeReviewPlanDefectProvider {
    fn normal() -> Self {
        Self {
            block_first_code_review: false,
            emit_plan_defect: true,
            code_review_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn approve_all() -> Self {
        Self {
            block_first_code_review: false,
            emit_plan_defect: false,
            code_review_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn recovery() -> Self {
        Self {
            block_first_code_review: true,
            emit_plan_defect: true,
            code_review_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for IndependentCodeReviewPlanDefectProvider {
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
            AdapterRole::Reviewer if input.output_schema == "coding_workspace_code_review_json" => {
                let call = self
                    .code_review_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if self.block_first_code_review && call == 0 {
                    serde_json::json!({
                        "verdict": "blocked",
                        "summary": "retry independent code review",
                        "findings": []
                    })
                    .to_string()
                } else if !self.emit_plan_defect
                    || call == usize::from(self.block_first_code_review)
                {
                    serde_json::json!({
                        "verdict": "approve",
                        "summary": "independent code review approved",
                        "findings": []
                    })
                    .to_string()
                } else {
                    serde_json::json!({
                        "verdict": "request_changes",
                        "summary": "independent code review found a plan defect",
                        "findings": [{
                            "source_stage": "code_review",
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
                        }]
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
    store
        .write_unit_review_conclusion_snapshot(
            &cadence_aria::product::coding_models::UnitReviewConclusionSnapshot {
                attempt_id: attempt.id.clone(),
                unit_id: run.unit_id.clone(),
                unit_run_id: run.id.clone(),
                logical_work_item_id: logical_work_item_id.to_string(),
                work_item_revision_id: run.work_item_revision_id.clone(),
                code_review_report_id: format!("fixture_review_{}", run.id),
                verdict: ReviewVerdict::Approve,
                finding_digest: Vec::new(),
                evidence_refs: Vec::new(),
                diff_refs: Vec::new(),
                raw_report_hash: format!("fixture_raw_{}", run.id),
            },
        )
        .expect("completed unit review snapshot");
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
        Arc::new(IndependentCodeReviewPlanDefectProvider::normal()),
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
    let mut bound_first_handoff = false;
    let mut emitted_plan_repair_request = None;
    let mut saw_plan_repair_state = false;
    for _ in 0..320 {
        match timeout(Duration::from_secs(2), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate })
                if gate.kind == CodingGateKind::StageGate =>
            {
                if let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
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
            Ok(CodingWsOutMessage::CodingSessionState { status, stage, .. })
                if status == CodingAttemptStatus::AwaitingPlanAmendment
                    && stage == CodingExecutionStage::CodeReview =>
            {
                saw_plan_repair_state = true;
                if emitted_plan_repair_request.is_some() {
                    break;
                }
            }
            Ok(CodingWsOutMessage::PlanRepairRequired { request, .. }) => {
                emitted_plan_repair_request = Some(*request);
                if saw_plan_repair_state {
                    break;
                }
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) => {}
            Err(_) if emitted_plan_repair_request.is_some() && saw_plan_repair_state => break,
            Err(_) => {
                let attempt = store
                    .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("attempt after timeout");
                let gates = store
                    .list_open_blocked_gates("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("blocked gates");
                let reports = store
                    .list_code_review_reports("project_0001", "issue_0001", "coding_attempt_0001")
                    .expect("code review reports");
                panic!(
                    "timed out before independent CodeReview routed the plan finding: status={:?} stage={:?} current={:?} gates={gates:?} reports={reports:?}",
                    attempt.status, attempt.stage, attempt.current_work_item_id,
                );
            }
        }
    }

    let request = emitted_plan_repair_request.expect("expected PlanRepairRequired from CodeReview");
    assert!(
        saw_plan_repair_state,
        "plan finding must safe-stop the group for Plan Repair"
    );
    let attempt = store
        .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
        .expect("attempt");
    assert_eq!(attempt.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert_eq!(attempt.stage, CodingExecutionStage::CodeReview);
    assert_ne!(attempt.status, CodingAttemptStatus::Completed);
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("plan lineage");
    let requests = revision_store
        .list_repair_requests(&lineage)
        .expect("repair requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0], request);
    assert_eq!(requests[0].trigger_review_id.as_deref(), Some("code_review_0002"));
    assert_eq!(
        requests[0].trigger_finding_id,
        "code_review_0002_finding_0001"
    );
    assert!(
        store
            .list_internal_pr_reviews("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("internal reviews")
            .is_empty(),
        "fresh groups must not run the removed provider group review"
    );

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
async fn coding_ws_group_session_state_omits_work_item_handoff_summary() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_group_attempt(root.path());
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
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
    assert!(state.get("work_item_handoff").is_none());
    assert_eq!(
        state["units"][0]["latest_handoff_revision_id"],
        "handoff_revision_0001"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}

