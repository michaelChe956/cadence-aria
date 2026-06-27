use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    FakeStreamingProvider, ProviderExecutionEvent, ProviderExecutionEventKind,
    ProviderExecutionEventStatus, StreamChunk,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{
    CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateRepositoryProfileInput,
    CreateStorySpecInput, CreateVerificationPlanInput, CreateWorkItemInput,
    CreateWorkspaceSessionInput, IssueWorkItemPlanUpdate, LifecycleStore,
};
use crate::product::models::{
    AgentRole, ArtifactRef, IssueWorkItemDependencyEdge, IssueWorkItemPlan,
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, NodeDetail, PermissionEvent,
    ProviderSnapshot, RepositoryProfileConfidence, VerificationCommand, VerificationCommandSafety,
    VerificationCommandSource, VerificationFallbackPolicy, VerificationManualCheck,
    VerificationScope, WorkItemContextBudget, WorkItemKind, WorkItemOutline,
    WorkItemOutlineDependencyEdge, WorkItemPlanOutline, WorkItemPlanStatus, WorkItemSplitFinding,
    WorkItemSplitFindingSeverity, WorkspaceMessageRecord,
};
use crate::protocol::contracts::{AdapterInput, ProviderType};
use crate::web::workspace_ws_types::{
    ArtifactPayload, AuthorDecision, ProviderConfigSnapshot, ReviewFinding, ReviewFindingSeverity,
    ReviewGate, ReviewVerdictType, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkItemCandidateDto, WorkItemCandidateMetaDto, WorkItemPlanCandidateDto, WorkItemPlanDto,
    WorkItemPlanOutlineCandidateDto, WorkItemPlanReviewAction, WorkItemPlanReviewComplete,
    WorkItemPlanReviewGate, WorkItemPlanReviewScope, WorkItemPlanReviewVerdict,
    WorkItemSplitOptionsDto, WorkspaceStage as WsWorkspaceStage,
};
use std::sync::Mutex;
use tempfile::TempDir;

fn setup() -> (TempDir, Arc<CheckpointStore>) {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
    (tmp, store)
}

fn artifact_payload(markdown: &str) -> ArtifactPayload {
    ArtifactPayload::Markdown {
        markdown: markdown.to_string(),
        diff: None,
    }
}

fn test_work_item_plan_outline(
    dependency_graph: Vec<WorkItemOutlineDependencyEdge>,
) -> WorkItemPlanOutline {
    WorkItemPlanOutline {
        id: "outline_001".to_string(),
        project_id: "project_001".to_string(),
        issue_id: "issue_001".to_string(),
        source_story_spec_ids: vec!["story_001".to_string()],
        source_design_spec_ids: vec!["design_001".to_string()],
        strategy_summary: "test strategy".to_string(),
        work_item_outlines: vec![
            WorkItemOutline {
                outline_id: "outline_a".to_string(),
                title: "A".to_string(),
                kind: WorkItemKind::Backend,
                goal: "A".to_string(),
                scope: vec!["src/a.rs".to_string()],
                non_goals: Vec::new(),
                source_story_spec_ids: vec!["story_001".to_string()],
                source_design_spec_ids: vec!["design_001".to_string()],
                exclusive_write_scopes: vec!["src/a.rs".to_string()],
                forbidden_write_scopes: Vec::new(),
                depends_on: Vec::new(),
                verification_intent: vec!["cargo test --locked --lib a".to_string()],
                handoff_notes: "handoff A".to_string(),
            },
            WorkItemOutline {
                outline_id: "outline_b".to_string(),
                title: "B".to_string(),
                kind: WorkItemKind::Frontend,
                goal: "B".to_string(),
                scope: vec!["web/b.ts".to_string()],
                non_goals: Vec::new(),
                source_story_spec_ids: vec!["story_001".to_string()],
                source_design_spec_ids: vec!["design_001".to_string()],
                exclusive_write_scopes: vec!["web/b.ts".to_string()],
                forbidden_write_scopes: Vec::new(),
                depends_on: Vec::new(),
                verification_intent: vec!["pnpm -C web test".to_string()],
                handoff_notes: "handoff B".to_string(),
            },
            WorkItemOutline {
                outline_id: "outline_c".to_string(),
                title: "C".to_string(),
                kind: WorkItemKind::Integration,
                goal: "C".to_string(),
                scope: vec!["tests/c.rs".to_string()],
                non_goals: Vec::new(),
                source_story_spec_ids: vec!["story_001".to_string()],
                source_design_spec_ids: vec!["design_001".to_string()],
                exclusive_write_scopes: vec!["tests/c.rs".to_string()],
                forbidden_write_scopes: Vec::new(),
                depends_on: Vec::new(),
                verification_intent: vec!["cargo test --locked --test c".to_string()],
                handoff_notes: "handoff C".to_string(),
            },
        ],
        dependency_graph,
        risks: Vec::new(),
        handoff_strategy: "handoff".to_string(),
        status: "draft".to_string(),
    }
}

#[test]
fn work_item_plan_outline_topological_order_keeps_original_order_for_ready_items() {
    let outline = test_work_item_plan_outline(vec![
        WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_a".to_string(),
            to_outline_id: "outline_c".to_string(),
        },
        WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_b".to_string(),
            to_outline_id: "outline_c".to_string(),
        },
    ]);

    let order = work_item_plan_outline_topological_order(&outline).expect("topological order");

    assert_eq!(
        order,
        vec![
            "outline_a".to_string(),
            "outline_b".to_string(),
            "outline_c".to_string()
        ]
    );
}

#[test]
fn work_item_plan_outline_topological_order_rejects_cycles() {
    let outline = test_work_item_plan_outline(vec![
        WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_a".to_string(),
            to_outline_id: "outline_b".to_string(),
        },
        WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_b".to_string(),
            to_outline_id: "outline_a".to_string(),
        },
    ]);

    let error = work_item_plan_outline_topological_order(&outline).expect_err("cycle rejected");

    assert!(error.contains("cycle"));
}

#[test]
fn build_artifact_version_summary_derives_size_for_markdown_and_candidate() {
    let markdown_version = ArtifactVersion {
        version: 1,
        payload: ArtifactPayload::Markdown {
            markdown: "hello".to_string(),
            diff: None,
        },
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-06-01T00:00:00Z".to_string(),
        source_node_id: "node_001".to_string(),
    };
    let summary = build_artifact_version_summary(&markdown_version);
    assert_eq!(summary.markdown_size, 5);
    assert_eq!(summary.markdown_preview, "hello");

    let candidate = WorkItemPlanCandidateDto {
        plan: WorkItemPlanDto {
            id: "plan_001".to_string(),
            status: "draft".to_string(),
            options: WorkItemSplitOptionsDto {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            dependency_graph: vec![],
        },
        work_items: vec![WorkItemCandidateDto {
            id: "wi_001".to_string(),
            kind: "backend".to_string(),
            title: "first work item".to_string(),
            depends_on: vec![],
            exclusive_write_scopes: vec![],
            verification_plan_ref: None,
            meta: WorkItemCandidateMetaDto {
                reverted: false,
                revert_feedback: None,
            },
        }],
        verification_plans: vec![],
        repository_profile: None,
        validator_findings: vec![],
    };
    let candidate_version = ArtifactVersion {
        version: 2,
        payload: ArtifactPayload::WorkItemPlanCandidate {
            candidate: Box::new(candidate.clone()),
        },
        generated_by: ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: false,
        created_at: "2026-06-01T00:00:01Z".to_string(),
        source_node_id: "node_002".to_string(),
    };
    let summary = build_artifact_version_summary(&candidate_version);
    assert_eq!(
        summary.markdown_size,
        serde_json::to_string(&candidate).unwrap().len()
    );
    assert!(
        summary.markdown_preview.contains("first work item")
            || summary.markdown_preview.contains("plan_001"),
        "candidate preview should contain title or plan id: {}",
        summary.markdown_preview
    );
}

fn make_session(session_id: &str) -> WorkspaceSession {
    WorkspaceSession {
        session_id: session_id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "story_spec_0001".to_string(),
        workspace_type: WorkspaceType::Story,
        stage: WorkspaceStage::PrepareContext,
        messages: Vec::new(),
        artifact: None,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),
        review_rounds: 2,
        superpowers_enabled: true,
        openspec_enabled: true,
        provider_conversations: Vec::new(),
        repository_path: None,
    }
}

fn empty_provider_commands() -> mpsc::Receiver<ProviderCommand> {
    let (_tx, rx) = mpsc::channel(8);
    rx
}

#[derive(Default)]
struct SessionRecordingProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    calls: Arc<Mutex<u32>>,
}

struct ImmediateOutputRecordingProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    output: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ImmediateOutputRecordingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().unwrap().push(input);
        let output = self.output.clone();
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
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
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by this test provider",
            0,
        ))
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SessionRecordingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().unwrap().push(input);
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let call_no = *calls;
        drop(calls);

        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = if call_no == 1 {
                "climb_stairs(n) 对 n <= 0 应该如何处理？\nA. 返回 0\nB. 抛出 ValueError\n"
            } else {
                "# Story Spec\n\n## 功能需求\n- 对 n <= 0 返回 0。\n\n## 成功标准\n- n <= 0 时返回 0。\n"
            };
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output.to_string(),
                    provider_session_id: Some("provider-author-session-1".to_string()),
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
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by this test provider",
            0,
        ))
    }
}

#[tokio::test]
async fn author_choice_followup_resumes_author_provider_session() {
    let (event_tx, _event_rx) = mpsc::channel(32);
    let mut session = make_session("sess_resume_author");
    session.workspace_type = WorkspaceType::Story;
    session.author_provider = ProviderName::Codex;
    session.reviewer_provider = None;
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    let provider = Arc::new(SessionRecordingProvider::default());

    let (_command_tx, command_rx) = mpsc::channel(8);
    engine
        .handle_user_message(
            "开始生成 Story Spec".to_string(),
            provider.clone(),
            command_rx,
        )
        .await;

    let prompt = engine
        .take_pending_author_choice_prompt("author_choice_msg_002", vec!["A".to_string()], None)
        .await
        .expect("pending author choice prompt");

    let (_command_tx2, command_rx2) = mpsc::channel(8);
    engine
        .handle_author_choice_followup_message(prompt.clone(), provider.clone(), command_rx2)
        .await;

    let inputs = provider.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    assert_eq!(
        inputs[1].resume_provider_session_id.as_deref(),
        Some("provider-author-session-1")
    );
    assert_eq!(inputs[1].prompt, prompt);
    assert!(
        inputs[1]
            .prompt
            .starts_with("用户回答了 author 的确认问题：")
    );
    assert!(!inputs[1].prompt.contains("[system]:"));
    assert!(!inputs[1].prompt.contains("[assistant]:"));
}

#[tokio::test]
async fn claude_code_text_choice_output_uses_text_fallback_as_recovery_path() {
    let (_tmp, store) = setup();
    let (tx, mut rx) = mpsc::channel(64);
    let mut session = make_session("sess_claude_text_choice_fallback");
    session.author_provider = ProviderName::ClaudeCode;
    session.reviewer_provider = None;
    let mut engine = WorkspaceEngine::new(store, tx, session);

    engine
        .drive_provider_session(ProviderSessionDriveInput {
            session: Ok(text_choice_provider_session(
                "climb_stairs(n) 对 n <= 0 应该如何处理？\nA. 返回 0\nB. 抛出 ValueError\n",
            )),
            command_rx: empty_provider_commands(),
            node_id: Some("timeline_node_author".to_string()),
            agent: Some(ProviderName::ClaudeCode),
            role: ProviderConversationRole::Author,
            artifact_retry: None,
            revision_resume_fallback: None,
        })
        .await;

    let events = drain_engine_events(&mut rx);
    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                EngineEvent::ChoiceRequest {
                    prompt,
                    source,
                    ..
                } if prompt.contains("n <= 0")
                    && *source == ChoiceRequestSource::TextFallback
            )
        }),
        "Claude Code 文本选择题应该作为兜底进入 text_fallback choice_request"
    );
    assert!(
        !events.iter().any(|event| {
            matches!(event, EngineEvent::ProtocolError { code, .. }
                if code == "CLAUDE_CODE_STRUCTURED_QUESTION_REQUIRED")
        }),
        "Claude Code 可解析文本选择题不应该再被结构化提问 protocol error 拦截"
    );
    let prompt = engine
        .take_pending_author_choice_prompt("author_choice_msg_001", vec!["A".to_string()], None)
        .await
        .expect("pending Claude Code text fallback choice prompt");
    assert!(prompt.contains("用户回答了 author 的确认问题"));
    assert!(prompt.contains("A. 返回 0"));
}

#[tokio::test]
async fn persistent_engine_recovers_pending_text_fallback_choice_after_restart() {
    let (_tmp, checkpoint_store) = setup();
    let app_root = tempfile::tempdir().expect("app root");
    let app_paths = ProductAppPaths::new(app_root.path().join(".aria"));
    let lifecycle_store = LifecycleStore::new(app_paths.clone());
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("workspace session");
    lifecycle_store
        .replace_workspace_messages(
            &session_record.id,
            vec![
                WorkspaceMessageRecord {
                    role: "system".to_string(),
                    content: "context".to_string(),
                    created_at: "2026-06-05T00:00:00Z".to_string(),
                },
                WorkspaceMessageRecord {
                    role: "user".to_string(),
                    content: "开始生成".to_string(),
                    created_at: "2026-06-05T00:00:01Z".to_string(),
                },
                WorkspaceMessageRecord {
                    role: "assistant".to_string(),
                    content: "我先说明一下当前判断。\n\n首次启动检测到缺失 Claude Code/Codex 时，Aria 应采用哪种安装策略？\n\n1. `确认后安装`\n2. `自动静默安装`\n3. `只检查不安装`".to_string(),
                    created_at: "2026-06-05T00:00:02Z".to_string(),
                },
            ],
        )
        .expect("replace messages");
    let provider_config_snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::ClaudeCode),
        review_rounds: 1,
    };
    lifecycle_store
        .save_timeline_nodes(
            &session_record.id,
            &[TimelineNode {
                node_id: "timeline_node_002".to_string(),
                node_type: TimelineNodeType::AuthorRun,
                agent: Some(ProviderName::Codex),
                stage: WsWorkspaceStage::Running,
                round: None,
                status: TimelineNodeStatus::Paused,
                title: "Story Spec 生成".to_string(),
                summary: Some("等待用户选择".to_string()),
                started_at: "2026-06-05T00:00:02Z".to_string(),
                completed_at: None,
                duration_ms: None,
                artifact_ref: None,
                provider_config_snapshot,
                retry: None,
            }],
        )
        .expect("replace timeline nodes");

    let session = WorkspaceSession::from_record(
        lifecycle_store
            .get_workspace_session(&session_record.id)
            .expect("reload session"),
    );
    let (tx, _rx) = mpsc::channel(8);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store, tx, session);

    let prompt = engine
        .take_pending_author_choice_prompt("author_choice_msg_003", vec!["1".to_string()], None)
        .await
        .expect("pending choice should be recovered from persisted assistant text");

    assert!(prompt.contains("用户回答了 author 的确认问题"));
    assert!(prompt.contains("首次启动检测到缺失 Claude Code/Codex"));
    assert!(prompt.contains("1. `确认后安装`"));
    assert!(!prompt.contains("我先说明一下当前判断"));
}

#[test]
fn provider_resume_session_id_is_isolated_by_role_and_provider() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_role_isolation");
    session.author_provider = ProviderName::ClaudeCode;
    session.reviewer_provider = Some(ProviderName::ClaudeCode);
    session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "author-session".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("node-author".to_string()),
    }];
    let checkpoint_tmp = TempDir::new().unwrap();
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );

    assert_eq!(
        engine.provider_resume_session_id(
            ProviderConversationRole::Author,
            &ProviderName::ClaudeCode
        ),
        Some("author-session".to_string())
    );
    assert_eq!(
        engine.provider_resume_session_id(
            ProviderConversationRole::Reviewer,
            &ProviderName::ClaudeCode
        ),
        None
    );
    assert_eq!(
        engine.provider_resume_session_id(ProviderConversationRole::Author, &ProviderName::Codex),
        None
    );
}

#[test]
fn design_artifact_gate_accepts_numbered_canonical_headings() {
    let content = r#"# Provider 依赖自检 Design Spec

## 1. 设计范围

本设计覆盖 provider 依赖自检与安装。

## 2. 设计决策

- [DEC-001] 新建 ProviderCatalog。

## 3. 组件 / API / 数据模型

- [CMP-001] ProviderCatalog。

## 4. 风险

无。
"#;

    assert!(content_has_complete_workspace_artifact(
        content,
        &WorkspaceType::Design
    ));
}

#[test]
fn design_artifact_gate_rejects_legacy_key_decision_heading() {
    let content = r#"# Provider 依赖自检 Design Spec

## 1. 设计范围

本设计覆盖 provider 依赖自检与安装。

## 关键决策

- [DEC-001] 新建 ProviderCatalog。

## 组件 / API / 数据模型

- [CMP-001] ProviderCatalog。
"#;

    assert!(!content_has_complete_workspace_artifact(
        content,
        &WorkspaceType::Design
    ));
}

#[test]
fn review_input_does_not_resume_prior_reviewer_provider_session() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_review_no_resume");
    session.reviewer_provider = Some(ProviderName::Codex);
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
    ));
    session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Reviewer,
        provider: ProviderName::Codex,
        provider_session_id: "codex-review-thread-1".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("timeline_node_003".to_string()),
    }];
    let checkpoint_tmp = TempDir::new().unwrap();
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );

    let input = engine.build_review_input().expect("review input");

    assert_eq!(input.resume_provider_session_id, None);
    assert!(input.prompt.contains("当前 Artifact"));
}
