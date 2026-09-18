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
use crate::web::workspace_session::WorkspaceSessionManager;

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
        native_session_id: None,
        events: event_rx,
        commands: command_tx,
    })
}

pub(super) struct ProviderRunFixture {
    root: tempfile::TempDir,
    repository_root: tempfile::TempDir,
    pub(super) app_paths: ProductAppPaths,
    pub(super) lifecycle: LifecycleStore,
    pub(super) record: WorkspaceSessionRecord,
    pub(super) engine: Arc<Mutex<WorkspaceEngine>>,
    pub(super) engine_tx: mpsc::Sender<EngineEvent>,
    pub(super) manager: Arc<WorkspaceSessionManager>,
    pub(super) workspace_runs: WorkspaceRunRegistry,
    pub(super) story_id: String,
    pub(super) design_id: String,
}

impl ProviderRunFixture {
    pub(super) fn new(flow_kind: WorkItemPlanFlowKind) -> Self {
        // 既有语义：engine 事件接收端在 fixture 返回前释放，engine 侧 `send` 立即
        // 失败且不阻塞。需要观测 engine 事件的用例改用 `new_with_engine_rx`。
        let (engine_tx, engine_rx) = mpsc::channel(64);
        let fixture = Self::build(flow_kind, engine_tx);
        drop(engine_rx);
        fixture
    }

    /// 保留 engine 事件接收端：仅用于必须观测内部接力事件（如
    /// `ProviderRunRequested{WorkItemPlanSingleCandidateAuthor}`）的用例。调用方
    /// MUST 持续 drain，否则 engine 事件发送端在缓冲区满后阻塞。
    pub(super) fn new_with_engine_rx(
        flow_kind: WorkItemPlanFlowKind,
    ) -> (Self, mpsc::Receiver<EngineEvent>) {
        let (engine_tx, engine_rx) = mpsc::channel(64);
        (Self::build(flow_kind, engine_tx), engine_rx)
    }

    pub(super) fn root_path(&self) -> std::path::PathBuf {
        self.root.path().to_path_buf()
    }

    fn build(flow_kind: WorkItemPlanFlowKind, engine_tx: mpsc::Sender<EngineEvent>) -> Self {
        let root = tempfile::tempdir().expect("temporary workspace root");
        let repository_root = tempfile::tempdir().expect("temporary repository root");
        std::fs::create_dir_all(repository_root.path().join(".claude/rules"))
            .expect("create language rules directory");
        std::fs::write(
            repository_root.path().join(".claude/rules/language.md"),
            "## 语言规则\n\n- **必须使用中文** - 所有响应、解释、注释和文档必须使用中文。\n",
        )
        .expect("write language rules");
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
        let mut session = WorkspaceSession::from_record(record.clone());
        session.repository_path = Some(repository_root.path().to_path_buf());
        let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
            lifecycle.clone(),
            engine_tx.clone(),
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
            repository_root,
            app_paths: app_paths.clone(),
            lifecycle,
            record: record.clone(),
            engine: engine.clone(),
            engine_tx,
            manager: WorkspaceSessionManager::test_fixture_with_parts(
                &record.id,
                engine.clone(),
                Arc::new(ProviderRegistry::new()),
                app_paths.clone(),
                record.clone(),
            ),
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

pub(super) fn single_candidate_context(
    fixture: &ProviderRunFixture,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> (WorkspaceInboundContext, mpsc::Receiver<OutboundControl>) {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
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
            session_id: fixture.record.id.clone(),
        },
        outbound_rx,
    )
}

pub(super) fn single_candidate_markdown(story_id: &str, design_id: &str) -> String {
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
         ### Handoff Schema\n- required_fields: commit_sha\n- provided_contract_refs: []\n- reviewer_check_refs: AC-001\n\n\
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
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            fixture.root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(fixture.root.path().to_path_buf()),
        ),
        engine: fixture.engine.clone(),
        run_context,
        outbound_tx,
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
    // F3 修复轮 P1-3：WorkItemPlan author（WorkItemSplitter）是策略角色——真实
    // web 启动路径必须在 provider.start 前绑定 run-bound durable sink（Legacy
    // 直连不得 policy+缺 sink 运行时 fail-closed）。
    assert!(
        input.tool_policy.is_some(),
        "WorkItemPlan author input must carry the deny policy"
    );
    assert!(
        input.audit_sink.is_some(),
        "WorkItemPlan author legacy web path must carry the run-bound durable audit sink"
    );
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
    assert!(full_input.prompt.contains("[cadence_project_rules]"));
    assert!(full_input.prompt.contains("必须使用中文"));
    assert!(full_input.prompt.contains("保持 grammar 指定的英文原样"));
    assert!(
        full_input
            .prompt
            .contains("任务拆分与验证设计遵循测试先行纪律")
    );
    assert!(full_input.prompt.contains("大范围定位优先检索工具"));
    assert!(!full_input.prompt.contains("按需查阅其中适用章节即可"));
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
async fn single_candidate_author_rejects_missing_language_rules_before_provider_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let language_rules_path = fixture
        .repository_root
        .path()
        .join(".claude/rules/language.md");
    std::fs::remove_file(&language_rules_path).expect("remove fixture language rules");
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(RecordingOutputProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
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

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let outbound = outbound_rx
                .recv()
                .await
                .expect("missing language rules must emit an error");
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
    .expect("missing language rules rejection");
    let message = error["message"].as_str().expect("error message");
    assert!(message.contains(&fixture.repository_root.path().display().to_string()));
    assert!(message.contains(&language_rules_path.display().to_string()));
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "missing language rules must reject before provider startup"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
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
async fn single_candidate_full_plan_parse_failure_is_fatal_after_one_teaching_reredrive() {
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
    // F2-B：missing_section 类失败给恰一次教学重驱；重驱仍败则终态。
    let reredrive_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("teaching re-drive must invoke the provider exactly once more")
        .expect("teaching re-drive input");
    assert!(
        reredrive_input
            .prompt
            .contains("立即输出完整 work-item-plan markdown source"),
        "re-drive prompt must carry the immediate-output teaching: {}",
        reredrive_input.prompt
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "teaching re-drive must be bounded to exactly one extra provider invocation"
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
    let message = error["message"].as_str().expect("error message");
    assert!(message.contains("compile markdown source failed"));
    assert!(
        message.contains("first round") && message.contains("re-drive round"),
        "terminal failure must carry both rounds' diagnostics: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

pub(super) async fn wait_for_single_candidate_phase(
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

pub(super) async fn wait_for_stage(engine: &Arc<Mutex<WorkspaceEngine>>, expected: WorkspaceStage) {
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

// F2-B（SC compile 失败教学重驱）：missing_section 类 compile 失败给一次教学重驱
// 自修机会（错误原文进重驱 prompt）；重驱成功→正常继续；重驱再败→终态失败含
// 两轮信息；非 missing_section 错误不触发重驱。

use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) struct SequenceOutputProvider {
    pub(super) outputs: Vec<String>,
    pub(super) inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    pub(super) next: AtomicUsize,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SequenceOutputProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.inputs.send(input);
        let index = self.next.fetch_add(1, Ordering::SeqCst);
        let output = self
            .outputs
            .get(index)
            .or_else(|| self.outputs.last())
            .cloned()
            .unwrap_or_default();
        provider_session_with_output(output).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("sequence provider tests use start")
    }
}

/// 从合法 SC markdown 中删除一个 section 块，制造 missing_section 类 compile 失败。
fn markdown_without_section(story_id: &str, design_id: &str, section_heading: &str) -> String {
    single_candidate_markdown(story_id, design_id)
        .split("\n\n")
        .filter(|chunk| !chunk.starts_with(section_heading))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 在结构化 section 内追加表外 key，制造 unknown_structured_key 类 compile 失败
/// （所有必需 section/字段仍在，不产生 missing_section）。
fn markdown_with_unknown_structured_key(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "### Handoff Schema\n- required_fields: commit_sha",
        "### Handoff Schema\n- bogus_key: x\n- required_fields: commit_sha",
        1,
    )
}

pub(super) async fn next_provider_input(
    input_rx: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
) -> StreamingProviderInput {
    tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("provider input expected")
        .expect("provider input channel open")
}

pub(super) async fn no_more_provider_inputs(
    input_rx: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
) {
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "no further provider invocation is allowed here"
    );
}

pub(super) async fn next_error_message(
    outbound_rx: &mut mpsc::Receiver<OutboundControl>,
) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outbound = outbound_rx.recv().await.expect("outbound control expected");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value["message"]
                    .as_str()
                    .expect("error message")
                    .to_string();
            }
        }
    })
    .await
    .expect("error outbound expected")
}

#[tokio::test]
async fn single_candidate_compile_missing_section_reredrive_recovers_and_continues() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_without_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
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

    let first_input = next_provider_input(&mut input_rx).await;
    assert!(
        first_input.prompt.contains("[markdown_grammar]"),
        "first round must stay the full markdown author prompt"
    );
    let reredrive_input = next_provider_input(&mut input_rx).await;
    for required in [
        "立即输出完整 work-item-plan markdown source",
        "第一行即文档标题 `# Work Item Plan`",
        "missing_section",
        "Work Item 缺少必需 section",
    ] {
        assert!(
            reredrive_input.prompt.contains(required),
            "teaching re-drive prompt must contain {required}: {}",
            reredrive_input.prompt
        );
    }
    no_more_provider_inputs(&mut input_rx).await;
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    assert_eq!(
        single_candidate_generation_steps_for_session(&fixture.record.id),
        vec!["full_markdown_author", "parse_source_revision", "selector"],
        "re-drive recovery must continue through the canonical compile path exactly once",
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
    );
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: durable.project_id.clone(),
        issue_id: durable.issue_id.clone(),
        plan_id: durable.entity_id.clone(),
    };
    let source_store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    );
    let stored = source_store
        .get_source_revision(
            &scope,
            durable
                .work_item_plan_source_revision_ref
                .as_deref()
                .expect("source revision ref"),
        )
        .expect("stored source");
    assert_eq!(
        stored.source,
        single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        "the recovered re-drive output must become the persisted source revision"
    );
}

#[tokio::test]
async fn single_candidate_compile_missing_section_reredrive_failure_is_terminal_with_both_rounds() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            markdown_without_section(&fixture.story_id, &fixture.design_id, "### Goal"),
            markdown_without_section(&fixture.story_id, &fixture.design_id, "### Tasks"),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
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

    let _first_input = next_provider_input(&mut input_rx).await;
    let _reredrive_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed"),
        "{message}"
    );
    assert!(
        message.contains("first round") && message.contains("re-drive round"),
        "terminal failure must carry both rounds' diagnostics: {message}"
    );
    assert!(
        message.matches("missing_section").count() >= 2,
        "terminal failure must contain both rounds' error text: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

#[tokio::test]
async fn single_candidate_compile_unknown_key_failure_stays_terminal_without_reredrive() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![markdown_with_unknown_structured_key(
            &fixture.story_id,
            &fixture.design_id,
        )],
        inputs: input_tx,
        next: AtomicUsize::new(0),
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

    let _first_input = next_provider_input(&mut input_rx).await;
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed")
            && message.contains("unknown_structured_key"),
        "non-missing_section compile failure must stay terminal: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

#[tokio::test]
async fn single_candidate_failed_reopen_claims_next_ledger_key_and_starts_provider() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let language_rules_path = fixture
        .repository_root
        .path()
        .join(".claude/rules/language.md");
    let language_rules = std::fs::read_to_string(&language_rules_path)
        .expect("fixture language rules must exist before the failed attempt");
    std::fs::remove_file(&language_rules_path).expect("remove language rules for first attempt");

    let (first_input_tx, first_input_rx) = mpsc::unbounded_channel();
    let first_provider = Arc::new(RecordingOutputProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        inputs: first_input_tx,
    });
    let (first_context, mut first_outbound_rx) = single_candidate_context(&fixture, first_provider);
    handle_workspace_inbound_message(
        first_context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;
    let _ = next_error_message(&mut first_outbound_rx).await;
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
    drop(first_input_rx);

    std::fs::write(&language_rules_path, language_rules)
        .expect("restore language rules before explicit user reopen");
    let (second_input_tx, mut second_input_rx) = mpsc::unbounded_channel();
    let second_provider = Arc::new(RecordingOutputProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        inputs: second_input_tx,
    });
    let (second_context, _second_outbound_rx) = single_candidate_context(&fixture, second_provider);
    handle_workspace_inbound_message(
        second_context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    let _ = next_provider_input(&mut second_input_rx).await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload reopened session");
    assert_eq!(
        durable
            .provider_start_ledger
            .iter()
            .map(|entry| entry.provider_start_idempotency_key.as_str())
            .collect::<Vec<_>>(),
        vec![
            format!("single_candidate_author:{}:0", fixture.record.id),
            format!("single_candidate_author:{}:1", fixture.record.id),
        ],
        "a failed explicit reopen must claim a new durable provider-start key"
    );
}

#[tokio::test]
async fn single_candidate_completed_reopen_is_rejected_without_starting_or_running() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    {
        let mut engine = fixture.engine.lock().await;
        engine.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Completed,
        );
    }

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(RecordingOutputProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
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
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("reopen SingleCandidate session rejected"),
        "completed reopen must return a visible rejection: {message}"
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "completed reopen must not invoke a provider"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload completed session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Completed)
    );
    assert_eq!(
        durable.status,
        crate::product::models::WorkspaceSessionStatus::Confirmed,
        "completed reopen must not pre-burn the durable status to running"
    );
    assert_eq!(
        fixture.engine.lock().await.current_stage(),
        WorkspaceStage::PrepareContext
    );
    assert!(fixture.manager.active_run().await.is_none());
}

#[tokio::test]
async fn single_candidate_reopen_preserves_failed_author_node_terminal_fields() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let node_id = "failed-author-run".to_string();
    let original_completed_at = "2026-09-18T08:09:10Z".to_string();
    let original_summary = "原始 author 失败摘要".to_string();
    {
        let mut engine = fixture.engine.lock().await;
        engine
            .timeline_nodes
            .push(crate::web::workspace_ws_types::TimelineNode {
                node_id: node_id.clone(),
                node_type: crate::web::workspace_ws_types::TimelineNodeType::AuthorRun,
                agent: Some(ProviderName::ClaudeCode),
                stage: crate::web::workspace_ws_types::WorkspaceStage::Running,
                round: None,
                status: crate::web::workspace_ws_types::TimelineNodeStatus::Failed,
                title: "SingleCandidate author".to_string(),
                summary: Some(original_summary.clone()),
                started_at: "2026-09-18T08:00:00Z".to_string(),
                completed_at: Some(original_completed_at.clone()),
                duration_ms: Some(10),
                artifact_ref: None,
                provider_config_snapshot: provider_config(),
                retry: None,
            });
        engine.active_node_id = Some(node_id.clone());
        engine.persist_timeline_nodes();
        engine.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Failed,
        );

        engine
            .start_generation(provider_config(), false)
            .await
            .expect("explicit reopen should re-arm a failed SingleCandidate session");

        let node = engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .expect("original failed author node");
        assert_eq!(
            node.status,
            crate::web::workspace_ws_types::TimelineNodeStatus::Failed
        );
        assert_eq!(
            node.completed_at.as_deref(),
            Some(original_completed_at.as_str())
        );
        assert_eq!(node.summary.as_deref(), Some(original_summary.as_str()));
    }
}

/// 永不完成的 provider：一旦被（错误地）启动即可被 starts 计数捕获。
struct HeldStartProvider {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HeldStartProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (_event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, _command_rx) = mpsc::channel(1);
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace provider-run tests use start")
    }
}

/// k3 P2 败者让位锚：provider-start 键已被健康持有者领走（在途 run 持有
/// `:{id}:0`，durable phase=Generate）时，迟到的第二条 StartGeneration run 必须
/// 静默让位——不启动 provider、不落 Failed 节点、不广播 Error、不翻转 durable
/// phase。修复前 `Ok(false)` 一律映射 Message：失败节点 + Error 广播 + 会话
/// 呈现与键持有者的健康在途状态矛盾。
#[tokio::test]
async fn single_candidate_late_run_yields_silently_when_start_key_already_claimed() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 模拟健康胜者：reserve 已领取首轮键（phase Prepare→Generate，ledger [:0]）。
    {
        let mut engine = fixture.engine.lock().await;
        let claimed = engine
            .reserve_single_candidate_author_start()
            .expect("healthy winner claims the first provider start key");
        assert!(claimed);
    }
    let starts = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(HeldStartProvider {
        starts: starts.clone(),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    let outbound_errors = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let recorded = outbound_errors.clone();
    let drain = tokio::spawn(async move {
        while let Some(control) = outbound_rx.recv().await {
            let OutboundControl::Text(text) = control else {
                continue;
            };
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                && value["type"] == "error"
            {
                recorded
                    .lock()
                    .await
                    .push(value["message"].as_str().unwrap_or("").to_string());
            }
        }
    });

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    // 等待迟到 run 退场（manager 注册被 finish_run 清空）。
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while fixture.manager.active_run().await.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("late run must exit instead of hanging on the claimed key");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "败者不得启动 provider——键的持有者独占本次启动"
    );
    assert!(
        outbound_errors.lock().await.is_empty(),
        "败者让位不得广播 Error：{:?}",
        outbound_errors.lock().await
    );
    let (failed_nodes, phase) = {
        let engine = fixture.engine.lock().await;
        (
            engine
                .timeline_nodes
                .iter()
                .filter(|node| {
                    node.status == crate::web::workspace_ws_types::TimelineNodeStatus::Failed
                })
                .count(),
            engine.session.single_candidate_phase.clone(),
        )
    };
    assert_eq!(failed_nodes, 0, "败者让位不得产生虚假 Failed 节点");
    assert_eq!(
        phase,
        Some(crate::product::models::SingleCandidatePhase::Generate),
        "败者让位不得翻转 durable phase"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Generate),
        "durable phase 必须保持键持有者留下的 Generate"
    );
    drain.abort();
}

/// k3 P2 区分锚的另一半：终态 Failed 且无他者在跑时（恢复/迟到 spawn 形态，
/// 不经过入站 re-arm），无法启动必须保持可见 Message 失败路径——Error 广播 +
/// PrepareContext 回滚是既有恢复入口，不得被让位语义吞掉。
#[tokio::test]
async fn single_candidate_terminal_failed_start_reports_visible_recovery_error() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    {
        let mut engine = fixture.engine.lock().await;
        engine.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Failed,
        );
    }
    let starts = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(HeldStartProvider {
        starts: starts.clone(),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);

    spawn_provider_run_from_event(
        run_context,
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
        None,
        outbound_tx,
    )
    .await
    .expect("spawn the terminal-failed start attempt");

    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("已终态失败"),
        "terminal-failed start must stay visibly rejected, got: {message}"
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "终态失败会话不得再启动 provider"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Failed),
        "durable 终态必须保持 Failed（恢复入口语义）"
    );
    let _ = fixture.manager.abort_active_run().await;
    drop(outbound_rx);
}
