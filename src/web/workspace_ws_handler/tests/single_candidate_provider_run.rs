use super::*;
use crate::cross_cutting::provider_adapter::{ProviderAdapterError, structured_output_sentinel};
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamChunk, StreamingProviderInput,
};
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::{
    CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
    CreateWorkspaceSessionInput, WorkItemPlanSessionOptions,
};
use crate::product::models::{IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, WorkspaceType};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};

struct RecordingOutputProvider {
    output: String,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RecordingOutputProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.inputs.send(input);
        provider_session_with_output(self.output.clone()).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace provider-run tests use start")
    }
}

async fn provider_session_with_output(
    output: String,
) -> Result<ProviderSession, ProviderAdapterError> {
    let (event_tx, event_rx) = mpsc::channel(4);
    let (command_tx, _command_rx) = mpsc::channel(1);
    tokio::spawn(async move {
        let _ = event_tx
            .send(ProviderEvent::Completed(ProviderCompletion::plain(
                output, None,
            )))
            .await;
    });
    Ok(ProviderSession {
        events: event_rx,
        commands: command_tx,
    })
}

struct ProviderRunFixture {
    root: tempfile::TempDir,
    app_paths: ProductAppPaths,
    lifecycle: LifecycleStore,
    record: WorkspaceSessionRecord,
    engine: Arc<Mutex<WorkspaceEngine>>,
    current_run: Arc<Mutex<Option<WorkspaceActiveRun>>>,
    workspace_runs: WorkspaceRunRegistry,
    story_id: String,
    design_id: String,
}

impl ProviderRunFixture {
    fn new(flow_kind: WorkItemPlanFlowKind) -> Self {
        let root = tempfile::tempdir().expect("temporary workspace root");
        let repository_root = tempfile::tempdir().expect("temporary repository root");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        seed_legacy_project(&app_paths);
        let repository = RepositoryStore::new(app_paths.clone())
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "Provider run fixture repository".to_string(),
                path: repository_root.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: format!("provider-run-fixture-{flow_kind:?}"),
            })
            .expect("create repository");
        IssueStore::new(app_paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                logical_codebase_id: None,
                title: "Provider run flow dispatch".to_string(),
                description: Some("durable flow_kind must select one provider chain".to_string()),
                change_id: None,
            })
            .expect("create issue");
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository.id.clone(),
                title: "Provider run Story".to_string(),
                aggregate_codebase: None,
            })
            .expect("create story");
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "Provider run Design".to_string(),
                aggregate_codebase: None,
            })
            .expect("create design");
        let plan = lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some("plan_0001".to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![story.id.clone()],
                source_design_spec_ids: vec![design.id.clone()],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: Vec::new(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            })
            .expect("create plan");
        let session_id = format!(
            "workspace_session_{}_provider_run_{}",
            match flow_kind {
                WorkItemPlanFlowKind::Legacy => "legacy",
                WorkItemPlanFlowKind::SingleCandidate => "single_candidate",
            },
            uuid::Uuid::new_v4().simple(),
        );
        let record = lifecycle
            .create_workspace_session_with_id(
                CreateWorkspaceSessionInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    entity_id: plan.id,
                    workspace_type: WorkspaceType::WorkItemPlan,
                    author_provider: ProviderName::ClaudeCode,
                    reviewer_provider: ProviderName::Codex,
                    review_rounds: 0,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: Some(WorkItemPlanSessionOptions {
                        flow_kind,
                        run_policy: RunPolicy::Interactive,
                        rollout_snapshot: flow_kind == WorkItemPlanFlowKind::SingleCandidate,
                    }),
                },
                session_id,
            )
            .expect("create workspace session");
        let session_root = app_paths
            .issue_root("project_0001", "issue_0001")
            .join("workspace-sessions");
        assert!(
            session_root.join(format!("{}.json", record.id)).exists(),
            "created session file must exist at {}",
            session_root.display(),
        );
        assert_eq!(
            lifecycle
                .get_workspace_session(&record.id)
                .unwrap_or_else(|error| {
                    panic!(
                        "new fixture workspace session must be discoverable ({error:?}); root={}; entries={:?}",
                        session_root.display(),
                        std::fs::read_dir(&session_root)
                            .ok()
                            .into_iter()
                            .flatten()
                            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                            .collect::<Vec<_>>(),
                    )
                })
                .id,
            record.id,
        );
        let (engine_tx, _engine_rx) = mpsc::channel(64);
        let mut session = WorkspaceSession::from_record(record.clone());
        session.repository_path = Some(repository_root.path().to_path_buf());
        let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
            lifecycle.clone(),
            engine_tx,
            session,
        )));
        assert_eq!(
            lifecycle
                .get_workspace_session(&record.id)
                .expect("persistent engine construction must retain workspace session")
                .id,
            record.id,
        );
        Self {
            root,
            app_paths,
            lifecycle,
            record,
            engine,
            current_run: Arc::new(Mutex::new(None)),
            workspace_runs: WorkspaceRunRegistry::default(),
            story_id: story.id,
            design_id: design.id,
        }
    }
}

fn legacy_outline_output(story_id: &str, design_id: &str) -> String {
    let output = serde_json::json!({
        "outline": {
            "id": "outline_001",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "source_story_spec_ids": [story_id],
            "source_design_spec_ids": [design_id],
            "strategy_summary": "one backend owner",
            "work_item_outlines": [{
                "outline_id": "outline_backend",
                "logical_work_item_id": "WI-001",
                "title": "Backend API",
                "kind": "backend",
                "goal": "provide an API",
                "scope": ["src/backend/**"],
                "non_goals": ["frontend"],
                "estimated_context_tokens": 12000,
                "session_fit": "fits_single_agent_session",
                "source_story_spec_ids": [story_id],
                "source_design_spec_ids": [design_id],
                "exclusive_write_scopes": ["src/backend/**"],
                "forbidden_write_scopes": ["web/**"],
                "depends_on": [],
                "verification_intent": ["cargo test --locked --lib backend"],
                "trusted_verification_commands": [{
                    "command": "cargo test --locked --lib backend",
                    "cwd": ".",
                    "purpose": "backend test",
                    "source_ref": "design#verification"
                }],
                "handoff_notes": "provide API contract"
            }],
            "risks": [],
            "handoff_strategy": "one owner",
            "status": "draft"
        },
        "context_blockers": []
    });
    structured_output_sentinel("legacy-flow", &output)
}

fn single_candidate_context(
    fixture: &ProviderRunFixture,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> (WorkspaceInboundContext, mpsc::Receiver<OutboundControl>) {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: fixture.engine.clone(),
        current_run: fixture.current_run.clone(),
        workspace_runs: fixture.workspace_runs.clone(),
        session_id: fixture.record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths: fixture.app_paths.clone(),
        session_record: fixture.record.clone(),
    };
    let (outbound_tx, outbound_rx) = mpsc::channel(64);
    (
        WorkspaceInboundContext {
            app_state: WebAppState::new(
                fixture.root.path().to_path_buf(),
                crate::web::runtime::WebRuntime::new_fake(fixture.root.path().to_path_buf()),
            ),
            engine: fixture.engine.clone(),
            run_context,
            outbound_tx,
            current_run: fixture.current_run.clone(),
            workspace_runs: fixture.workspace_runs.clone(),
            session_id: fixture.record.id.clone(),
        },
        outbound_rx,
    )
}

fn single_candidate_markdown(story_id: &str, design_id: &str) -> String {
    format!(
        "# Work Item Plan\n\
         ## Work Item WI-001: Backend API\n\n\
         ### Identity\n- schema_version: 1\n- logical_work_item_id: WI-001\n- title: Backend API\n- kind: backend\n\n\
         ### Goal\n- summary: WHEN a request arrives THE SYSTEM SHALL return the planned API response.\n\n\
         ### Non Goals\n- non_goals: Frontend rendering is out of scope.\n\n\
         ### Dependencies\n- depends_on: []\n\n\
         ### Inputs\n\n\
         ### Outputs\n- contract_id: contract.backend-api\n- capabilities: api.backend.read\n\n\
         ### Tasks\n- task_id: TASK-001\n- statement: WHEN a request arrives THE SYSTEM SHALL return the planned API response.\n- requirement_refs: REQ-001\n- done_when_refs: AC-001\n\n\
         ### Write Policy\n- exclusive_scopes: src/backend/**\n- forbidden_scopes: web/**\n\n\
         ### Acceptance Criteria\n- criterion_id: AC-001\n- statement: WHEN a request arrives THE SYSTEM SHALL expose the backend API response.\n- required_evidence: source_diff\n- required_evidence: manual_check\n\n\
         ### Verification\n- check_id: CHECK-001\n- manual_instruction: Inspect the backend API response manually.\n- required: true\n- non_zero_test_execution_required: false\n\n\
         ### Handoff Schema\n- required_fields: commit_sha\n- provided_contract_refs: contract.backend-api\n- reviewer_check_refs: AC-001\n\n\
         ### Blockers\n- reason_code: no_trusted_command_catalog\n- route: operational_gate\n- target_contract_refs: contract.backend-api\n\n\
         ### Traceability\n- source_type: design_spec\n- source_id: {design_id}\n- requirement_id: REQ-001\n\n\
         ### Notes\nGenerated from Story {story_id}.\n\n\
         ### Rationale\nA single backend item owns the API boundary.\n"
    )
}

fn single_candidate_markdown_with_command(
    story_id: &str,
    design_id: &str,
    command: &str,
) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- check_id: CHECK-001\n- manual_instruction: Inspect the backend API response manually.",
        &format!(
            "- check_id: CHECK-001\n- command: {command}\n- manual_instruction: Inspect the backend API response manually."
        ),
        1,
    )
}

#[tokio::test]
async fn legacy_provider_run_uses_outline_builder_and_legacy_parser_only() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output = legacy_outline_output(&fixture.story_id, &fixture.design_id);
    let provider = Arc::new(RecordingOutputProvider {
        output,
        inputs: input_tx,
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: fixture.engine.clone(),
        current_run: fixture.current_run.clone(),
        workspace_runs: fixture.workspace_runs.clone(),
        session_id: fixture.record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths: fixture.app_paths.clone(),
        session_record: fixture.record.clone(),
    };
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            fixture.root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(fixture.root.path().to_path_buf()),
        ),
        engine: fixture.engine.clone(),
        run_context,
        outbound_tx,
        current_run: fixture.current_run.clone(),
        workspace_runs: fixture.workspace_runs.clone(),
        session_id: fixture.record.id.clone(),
    };

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let input = match tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("legacy provider must receive input")
    {
        Some(input) => input,
        None => {
            let engine = fixture.engine.lock().await;
            let mut outbound = Vec::new();
            while let Ok(Some(control)) =
                tokio::time::timeout(std::time::Duration::from_millis(20), outbound_rx.recv()).await
            {
                outbound.push(format!("{control:?}"));
            }
            panic!(
                "legacy provider input channel closed; stage={:?}, active={:?}, outbound={outbound:?}",
                engine.session().stage,
                engine.active_run_id(),
            );
        }
    };
    assert!(input.prompt.contains("WorkItemPlan Outline"));
    assert!(!input.prompt.contains("[markdown_grammar]"));
    wait_for_stage(&fixture.engine, WorkspaceStage::AuthorConfirm).await;
    assert_eq!(
        work_item_plan_parser_paths_for_session(&fixture.record.id),
        vec!["legacy_outline"],
        "legacy run must not reach the markdown compiler path",
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload legacy session");
    assert!(durable.work_item_plan_source_revision_ref.is_none());
}

#[tokio::test]
async fn single_candidate_provider_run_uses_markdown_builder_and_source_store_only() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output = single_candidate_markdown(&fixture.story_id, &fixture.design_id);
    let provider = Arc::new(RecordingOutputProvider {
        output,
        inputs: input_tx,
    });
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("single-candidate full author provider must receive exactly one input")
        .expect("single-candidate full author provider input");
    assert!(full_input.prompt.contains("[markdown_grammar]"));
    assert!(full_input.prompt.contains("[routing_reference]"));
    assert!(full_input.prompt.contains("[real_finding_few_shot]"));
    assert!(!full_input.prompt.contains("[outline_commands]"));
    assert!(!full_input.prompt.contains("<ARIA_STRUCTURED_OUTPUT"));
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "single-candidate must not invoke an outline provider before the full author"
    );
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    assert_eq!(
        single_candidate_generation_steps_for_session(&fixture.record.id),
        vec!["full_markdown_author", "parse_source_revision", "selector"],
        "single-candidate invocation order must stay full author → compile/source revision → internal selector diagnostic",
    );
    assert_eq!(
        work_item_plan_parser_paths_for_session(&fixture.record.id),
        vec!["single_candidate_markdown"],
        "single-candidate run must compile only the full markdown source and never reach the legacy or outline parser",
    );
    assert!(
        !fixture
            .engine
            .lock()
            .await
            .timeline_nodes
            .iter()
            .any(|node| {
                node.node_type
                    == crate::web::workspace_ws_types::TimelineNodeType::WorkItemGenerationMode
            }),
        "internal selection must not create a generation decision request node"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
    );
    let source_ref = durable
        .work_item_plan_source_revision_ref
        .as_deref()
        .expect("source revision ref");
    let ir_ref = durable.plan_candidate_ir_ref.as_deref().expect("IR ref");
    let report_ref = durable
        .mechanical_report_ref
        .as_deref()
        .expect("mechanical report ref");
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: durable.project_id.clone(),
        issue_id: durable.issue_id.clone(),
        plan_id: durable.entity_id.clone(),
    };
    let source_store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    );
    assert_eq!(
        source_store
            .get_source_revision(&scope, source_ref)
            .expect("stored source")
            .source,
        single_candidate_markdown(&fixture.story_id, &fixture.design_id),
    );
    source_store
        .get_plan_candidate_ir(&scope, ir_ref)
        .expect("stored IR");
    source_store
        .get_mechanical_report(&scope, report_ref)
        .expect("stored mechanical report");
}

#[tokio::test]
async fn single_candidate_projects_declared_verification_command_without_outline_catalog() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let markdown = single_candidate_markdown_with_command(
        &fixture.story_id,
        &fixture.design_id,
        "node --test tests/backend/",
    );
    let provider = Arc::new(RecordingOutputProvider {
        output: markdown,
        inputs: input_tx,
    });
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("full author provider must receive input")
        .expect("full author provider input");
    assert!(
        full_input
            .prompt
            .contains("Verification.command 直接声明，将按声明执行")
    );
    assert!(!full_input.prompt.contains("outline 阶段登记"));
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;

    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: durable.project_id.clone(),
        issue_id: durable.issue_id.clone(),
        plan_id: durable.entity_id.clone(),
    };
    let source_store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    );
    let ir = source_store
        .get_plan_candidate_ir(
            &scope,
            durable.plan_candidate_ir_ref.as_deref().expect("IR ref"),
        )
        .expect("full-plan declared command 必须通过 lowering");
    let trusted = &ir.ir.items[0].trusted_commands[0];
    assert_eq!(trusted.command, "node --test tests/backend/");
    assert_eq!(trusted.cwd, ".");
    assert_eq!(trusted.purpose, "Inspect the backend API response");
    assert!(trusted.source_ref.starts_with("plan-"));
}

#[tokio::test]
async fn single_candidate_full_plan_parse_failure_is_fatal_after_one_provider_call() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(RecordingOutputProvider {
        output: "# Work Item Plan\n\n## Work Item WI-001: malformed\n".to_string(),
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("full author provider must receive input")
        .expect("full author provider input");
    assert!(
        full_input
            .prompt
            .contains("完整 `work-item-plan.md` source")
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "full-plan parse failure must not start another provider invocation"
    );
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let outbound = outbound_rx
                .recv()
                .await
                .expect("full-plan parse failure outbound");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value;
            }
        }
    })
    .await
    .expect("full-plan parse failure error");
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("compile markdown source failed")
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

async fn wait_for_single_candidate_phase(
    fixture: &ProviderRunFixture,
    expected: crate::product::models::SingleCandidatePhase,
) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if fixture
                .lifecycle
                .get_workspace_session(&fixture.record.id)
                .expect("reload session")
                .single_candidate_phase
                == Some(expected.clone())
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider run must reach expected single-candidate phase");
}

async fn wait_for_stage(engine: &Arc<Mutex<WorkspaceEngine>>, expected: WorkspaceStage) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if engine.lock().await.session().stage == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider run must reach expected stage");
}
