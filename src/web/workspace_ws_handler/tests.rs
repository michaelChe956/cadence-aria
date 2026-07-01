use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderSession, StreamChunk, StreamingProviderInput,
};
use crate::web::workspace_ws_types::{
    AuthorDecision, HumanConfirmDecision, ProviderConfigSnapshot, RevisionPath, StructuredFeedback,
};
use std::sync::atomic::{AtomicBool, Ordering};

fn provider_config() -> ProviderConfigSnapshot {
    ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: None,
        review_rounds: 0,
    }
}

#[test]
fn context_note_is_only_valid_in_prepare_context() {
    let msg = WsInMessage::ContextNote {
        content: "补充上下文".to_string(),
    };

    assert!(is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::PrepareContext
    ));
    assert!(!is_message_valid_for_stage(&msg, &WorkspaceStage::Running));
}

#[test]
fn start_generation_is_only_valid_in_prepare_context() {
    let msg = WsInMessage::StartGeneration {
        provider_config: provider_config(),
        reviewer_enabled: false,
    };

    assert!(is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::PrepareContext
    ));
    assert!(!is_message_valid_for_stage(&msg, &WorkspaceStage::Running));
}

#[test]
fn hello_and_ping_are_valid_for_every_stage() {
    let hello = WsInMessage::Hello {
        session_id: "session-1".to_string(),
        last_seen_node_id: Some("node-1".to_string()),
    };
    let ping = WsInMessage::Ping;

    for stage in [
        WorkspaceStage::PrepareContext,
        WorkspaceStage::Running,
        WorkspaceStage::AuthorConfirm,
        WorkspaceStage::CrossReview,
        WorkspaceStage::ReviewDecision,
        WorkspaceStage::Revision,
        WorkspaceStage::HumanConfirm,
        WorkspaceStage::Completed,
    ] {
        assert!(is_message_valid_for_stage(&hello, &stage));
        assert!(is_message_valid_for_stage(&ping, &stage));
    }
}

#[tokio::test]
async fn idle_timeout_sends_close_control_after_client_quiet() {
    let last_client_message_at = Arc::new(Mutex::new(tokio::time::Instant::now()));
    let (tx, mut rx) = mpsc::channel(1);

    let task = spawn_idle_timeout_task(
        last_client_message_at,
        tx,
        Arc::new(|| false),
        std::time::Duration::from_millis(5),
        std::time::Duration::from_millis(1),
    );

    let control = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
        .await
        .expect("idle timeout control")
        .expect("close control");
    assert!(matches!(control, OutboundControl::CloseDueToIdleTimeout));

    task.abort();
}

#[tokio::test]
async fn idle_timeout_waits_while_provider_run_is_active() {
    let last_client_message_at = Arc::new(Mutex::new(tokio::time::Instant::now()));
    let (tx, mut rx) = mpsc::channel(1);
    let active = Arc::new(AtomicBool::new(true));
    let active_for_task = active.clone();

    let task = spawn_idle_timeout_task(
        last_client_message_at,
        tx,
        Arc::new(move || active_for_task.load(Ordering::SeqCst)),
        std::time::Duration::from_millis(5),
        std::time::Duration::from_millis(1),
    );

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), rx.recv())
            .await
            .is_err()
    );

    active.store(false, Ordering::SeqCst);
    let control = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
        .await
        .expect("idle timeout after active run")
        .expect("close control");
    assert!(matches!(control, OutboundControl::CloseDueToIdleTimeout));

    task.abort();
}

#[test]
fn revision_path_messages_are_only_valid_in_review_decision() {
    let select_path = WsInMessage::SelectRevisionPath {
        path: RevisionPath::ReviseWithContext,
        extra_context: Some("补充修改约束".to_string()),
    };
    let legacy_decision = WsInMessage::ReviewDecisionResponse {
        decision: "continue".to_string(),
        extra_context: None,
    };

    assert!(is_message_valid_for_stage(
        &select_path,
        &WorkspaceStage::ReviewDecision
    ));
    assert!(is_message_valid_for_stage(
        &legacy_decision,
        &WorkspaceStage::ReviewDecision
    ));
    assert!(!is_message_valid_for_stage(
        &select_path,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(!is_message_valid_for_stage(
        &legacy_decision,
        &WorkspaceStage::HumanConfirm
    ));
}

#[test]
fn author_decision_is_only_valid_in_author_confirm() {
    let msg = WsInMessage::AuthorDecision {
        decision: AuthorDecision::Accept,
    };

    assert!(is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::AuthorConfirm
    ));
    assert!(!is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::PrepareContext
    ));
    assert!(!is_message_valid_for_stage(
        &msg,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(requires_stage_validation(&msg));
    assert_eq!(message_type(&msg), "author_decision");
}

#[test]
fn human_confirm_messages_are_only_valid_in_human_confirm() {
    let human_confirm = WsInMessage::HumanConfirm {
        decision: HumanConfirmDecision::RequestChange,
        payload: Some(serde_json::json!({"description": "补充验收条件"})),
    };
    let legacy_request_revision = WsInMessage::RequestRevision {
        feedback: StructuredFeedback {
            feedback_types: vec!["clarity".to_string()],
            description: "补充验收条件".to_string(),
            target_artifact_version: Some(1),
        },
    };
    let legacy_confirm = WsInMessage::Confirm;

    assert!(is_message_valid_for_stage(
        &human_confirm,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(is_message_valid_for_stage(
        &legacy_request_revision,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(is_message_valid_for_stage(
        &legacy_confirm,
        &WorkspaceStage::HumanConfirm
    ));
    assert!(!is_message_valid_for_stage(
        &human_confirm,
        &WorkspaceStage::ReviewDecision
    ));
    assert!(!is_message_valid_for_stage(
        &legacy_request_revision,
        &WorkspaceStage::ReviewDecision
    ));
}

#[test]
fn completed_stage_rejects_business_messages() {
    assert!(!is_message_valid_for_stage(
        &WsInMessage::Abort,
        &WorkspaceStage::Completed
    ));
    assert!(!is_message_valid_for_stage(
        &WsInMessage::ContextNote {
            content: "late note".to_string()
        },
        &WorkspaceStage::Completed
    ));
}

#[test]
fn control_and_legacy_messages_do_not_require_stage_lock_validation() {
    assert!(!requires_stage_validation(&WsInMessage::Abort));
    assert!(!requires_stage_validation(
        &WsInMessage::PermissionResponse {
            id: "permission-1".to_string(),
            approved: true,
            reason: None,
        }
    ));
    assert!(!requires_stage_validation(&WsInMessage::ChoiceResponse {
        id: "choice-1".to_string(),
        selected_option_ids: vec!["continue".to_string()],
        free_text: None,
        answers: vec![],
    }));
    assert!(!requires_stage_validation(&WsInMessage::UserMessage {
        content: "legacy generation request".to_string(),
    }));
    assert!(!requires_stage_validation(&WsInMessage::Rollback {
        checkpoint_id: "cp_001".to_string(),
    }));
    assert!(!requires_stage_validation(&WsInMessage::Hello {
        session_id: "session-1".to_string(),
        last_seen_node_id: None,
    }));
    assert!(!requires_stage_validation(&WsInMessage::Ping));
    assert!(requires_stage_validation(&WsInMessage::ContextNote {
        content: "new protocol action".to_string(),
    }));
}

#[test]
fn choice_response_message_type_is_reported_for_protocol_errors() {
    assert_eq!(
        message_type(&WsInMessage::ChoiceResponse {
            id: "choice-1".to_string(),
            selected_option_ids: vec!["continue".to_string()],
            free_text: None,
            answers: vec![],
        }),
        "choice_response"
    );
}

#[test]
fn missing_active_run_error_uses_protocol_error() {
    let error = missing_active_run_error("choice_response", "choice-1");

    match error {
        WsOutMessage::ProtocolError {
            code,
            message,
            context,
        } => {
            assert_eq!(code, "ACTIVE_RUN_NOT_FOUND");
            assert!(message.contains("choice_response"));
            assert_eq!(
                context
                    .as_ref()
                    .and_then(|value| value.get("id"))
                    .and_then(|value| value.as_str()),
                Some("choice-1")
            );
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[test]
fn revision_path_maps_to_existing_review_decision_contract() {
    assert_eq!(
        map_revision_path(RevisionPath::Revise, Some("ignored".to_string())),
        ("continue".to_string(), None)
    );
    assert_eq!(
        map_revision_path(
            RevisionPath::ReviseWithContext,
            Some("补充约束".to_string())
        ),
        (
            "continue_with_context".to_string(),
            Some("补充约束".to_string())
        )
    );
    assert_eq!(
        map_revision_path(RevisionPath::SkipToHuman, Some("ignored".to_string())),
        ("human_intervene".to_string(), None)
    );
}

#[tokio::test]
async fn start_generation_refreshes_stale_provider_guidance_before_prompting_author() {
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::lifecycle_store::{CreateStorySpecInput, CreateWorkspaceSessionInput};
    use crate::product::models::WorkspaceType;
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};

    let root = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "Repo".to_string(),
            path: repo.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
        })
        .unwrap();
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            title: "Provider guidance refresh".to_string(),
            description: Some("旧 context 不能把 Codex 交互纪律注入 Claude Code run".to_string()),
            change_id: None,
        })
        .unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id.clone(),
            title: "Provider guidance Story Spec".to_string(),
        })
        .unwrap();
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let session_record = ensure_workspace_context_message(&app_paths, &lifecycle, session_record)
        .expect("initial context");
    assert!(
        session_record.messages[0]
            .content
            .contains("当前 author provider 是 Codex")
    );

    let checkpoint_store = Arc::new(CheckpointStore::new(root.path().join("checkpoints")));
    let (engine_tx, _engine_rx) = mpsc::channel::<EngineEvent>(64);
    let mut session = WorkspaceSession::from_record(session_record.clone());
    session.repository_path = Some(repo.path().to_path_buf());
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle.clone(),
        engine_tx,
        session,
    )));
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(PromptRecordingProvider { input_tx }),
    );

    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths: app_paths.clone(),
        session_record: session_record.clone(),
    };
    let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(64);
    let inbound_context = WorkspaceInboundContext {
        engine,
        run_context,
        outbound_tx,
        current_run,
        workspace_runs,
        session_id: session_record.id,
    };

    handle_workspace_inbound_message(
        inbound_context,
        WsInMessage::StartGeneration {
            provider_config: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: Some(ProviderName::Codex),
                review_rounds: 1,
            },
            reviewer_enabled: true,
        },
    )
    .await;

    let input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("provider input should be sent")
        .expect("provider input");
    assert!(input.prompt.contains("当前 author provider 是 Claude Code"));
    assert!(input.prompt.contains("必须使用结构化 AskUserQuestion"));
    assert!(!input.prompt.contains("当前 author provider 是 Codex"));
    assert!(!input.prompt.contains("requestUserInput"));
}

#[tokio::test]
async fn provider_select_refreshes_provider_guidance_in_session_state() {
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::lifecycle_store::{CreateStorySpecInput, CreateWorkspaceSessionInput};
    use crate::product::models::WorkspaceType;
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};

    let root = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "Repo".to_string(),
            path: repo.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
        })
        .unwrap();
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            title: "Provider guidance select refresh".to_string(),
            description: Some(
                "prepare context should reflect selected author provider".to_string(),
            ),
            change_id: None,
        })
        .unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            title: "Provider guidance Story Spec".to_string(),
        })
        .unwrap();
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    let session_record = ensure_workspace_context_message(&app_paths, &lifecycle, session_record)
        .expect("initial context");
    assert!(
        session_record.messages[0]
            .content
            .contains("当前 author provider 是 Codex")
    );

    let checkpoint_store = Arc::new(CheckpointStore::new(root.path().join("checkpoints")));
    let (engine_tx, _engine_rx) = mpsc::channel::<EngineEvent>(64);
    let mut session = WorkspaceSession::from_record(session_record.clone());
    session.repository_path = Some(repo.path().to_path_buf());
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle,
        engine_tx,
        session,
    )));
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(ProviderRegistry::new()),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths,
        session_record: session_record.clone(),
    };
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(64);
    let inbound_context = WorkspaceInboundContext {
        engine,
        run_context,
        outbound_tx,
        current_run,
        workspace_runs,
        session_id: session_record.id,
    };

    handle_workspace_inbound_message(
        inbound_context,
        WsInMessage::ProviderSelect {
            role: "author".to_string(),
            provider: ProviderName::ClaudeCode,
        },
    )
    .await;

    let control = tokio::time::timeout(std::time::Duration::from_secs(1), outbound_rx.recv())
        .await
        .expect("session state should be sent")
        .expect("outbound control");
    let OutboundControl::Text(text) = control else {
        panic!("expected text outbound control");
    };
    let message = serde_json::from_str::<WsOutMessage>(&text).expect("ws out message");
    let WsOutMessage::SessionState {
        messages,
        providers,
        ..
    } = message
    else {
        panic!("expected session state");
    };
    assert_eq!(providers.author, ProviderName::ClaudeCode);
    let context = &messages[0].content;
    assert!(context.contains("当前 author provider 是 Claude Code"));
    assert!(context.contains("必须使用结构化 AskUserQuestion"));
    assert!(!context.contains("当前 author provider 是 Codex"));
    assert!(!context.contains("requestUserInput"));
}

struct PromptRecordingProvider {
    input_tx: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PromptRecordingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.input_tx.send(input);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = "# Story Spec\n\n\
                ## 范围\n来源 source id: Issue issue_0001；Provider guidance refresh.\n\n\
                ## 用户故事\n作为用户，我希望 provider guidance 与所选 provider 一致。\n\n\
                ## 功能需求\n- [REQ-001] provider guidance 与 Claude Code 匹配。\n\n\
                ## 成功标准\n- [AC-001] prompt 不包含 Codex requestUserInput 纪律。\n\n\
                ## 待确认项\n无。\n\n\
                ## 非功能需求\n无。\n";
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output.to_string(),
                    provider_session_id: None,
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by workspace ws handler tests",
            0,
        ))
    }
}

#[test]
fn build_work_item_plan_generate_request_includes_validator_findings_as_revision_feedback() {
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::lifecycle_store::{
        CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
        CreateWorkspaceSessionInput,
    };
    use crate::product::models::{
        IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName, WorkItemSplitFinding,
        WorkItemSplitFindingSeverity, WorkspaceType,
    };
    use std::sync::Arc;

    let tmp = tempfile::tempdir().unwrap();
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));

    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repo_0001".to_string(),
            title: "Story".to_string(),
        })
        .unwrap();
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
        })
        .unwrap();

    let finding = WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Error,
        code: "write_scope_required".to_string(),
        message: "work item must have at least one exclusive_write_scope".to_string(),
        work_item_ids: vec!["wi_001".to_string()],
    };
    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec![story.id],
            source_design_spec_ids: vec![design.id],
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
            validator_findings: vec![finding],
        })
        .unwrap();

    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: plan.id,
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .unwrap();

    let session = WorkspaceSession::from_record(session_record);
    let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
    let (tx, _rx) = mpsc::channel(1);
    let engine = WorkspaceEngine::new_persistent(checkpoint_store, lifecycle.clone(), tx, session);

    let request = build_work_item_plan_generate_request(&engine, &lifecycle).unwrap();

    let feedback = request
        .revision_feedback
        .expect("revision_feedback should be set when plan has findings");
    assert!(feedback.contains("write_scope_required"));
    assert!(feedback.contains("work item must have at least one exclusive_write_scope"));
}
