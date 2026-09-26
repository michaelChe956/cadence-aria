use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceAnswerData, ChoiceRequestData, FakeStreamingProvider, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, StreamChunk,
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
    WorkItemOutlineDependencyEdge, WorkItemOutlineSessionFit, WorkItemPlanOutline,
    WorkItemPlanStatus, WorkItemSplitFinding, WorkItemSplitFindingSeverity, WorkspaceMessageRecord,
};
use crate::protocol::contracts::{AdapterInput, ProviderType};
use crate::web::workspace_ws_types::{
    ArtifactPayload, ProviderConfigSnapshot, ReviewFinding, ReviewFindingSeverity, ReviewGate,
    ReviewVerdictType, TimelineNode, TimelineNodeStatus, TimelineNodeType, WorkItemCandidateDto,
    WorkItemCandidateMetaDto, WorkItemPlanCandidateDto, WorkItemPlanDto,
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

fn complete_story_artifact(requirement: &str, acceptance: &str) -> String {
    format!(
        "# Story Spec\n\n\
         ## 范围\n来源 source id: Issue issue_0001；{requirement}\n\n\
         ## 用户故事\n作为用户，我希望当前问题被清晰解决。\n\n\
         ## 功能需求\n- [REQ-001] {requirement}\n\n\
         ## 成功标准\n- [AC-001] {acceptance}\n\n\
         ## 待确认项\n无。\n\n\
         ## 非功能需求\n无。\n"
    )
}

fn complete_design_artifact(decision: &str, api: &str) -> String {
    format!(
        "# Design Spec\n\n\
         ## 设计范围\n覆盖当前设计变更。\n\n\
         ## 设计决策\n- [DEC-001] {decision}\n\n\
         ## 公共组件\n- [CMP-001] 复用现有组件边界。\n\n\
         ## API 契约\n- [API-001] {api}\n\n\
         ## 数据模型\n- 沿用现有数据模型。\n\n\
         ## 风险\n无。\n\n\
         ## 追踪关系\n- source ids: Story Spec story_spec_0001, Issue issue_0001。\n\
         - [DEC-001] -> [REQ-001]\n"
    )
}

fn complete_work_item_artifact(goal: &str) -> String {
    format!(
        "# Work Item\n\n\
         ## 目标\n{goal}\n\n\
         ## 范围\n仅覆盖当前单个可执行任务。\n\n\
         ## 实现步骤\n- 完成当前任务实现。\n- 补充当前任务验证。\n\n\
         ## 依赖\n依赖已确认 Story Spec 与 Design Spec。\n\n\
         ## 验证命令\n- cargo test --locked\n\n\
         ## 风险\n无。\n\n\
         ## 追踪关系\n- source ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n\
         - [REQ-001]\n"
    )
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
                target_repository_id: None,
                outline_id: "outline_a".to_string(),
                logical_work_item_id: "wi_a".to_string(),
                title: "A".to_string(),
                kind: WorkItemKind::Backend,
                goal: "A".to_string(),
                scope: vec!["src/a.rs".to_string()],
                non_goals: Vec::new(),
                estimated_context_tokens: Some(12_000),
                session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
                source_story_spec_ids: vec!["story_001".to_string()],
                source_design_spec_ids: vec!["design_001".to_string()],
                exclusive_write_scopes: vec!["src/a.rs".to_string()],
                forbidden_write_scopes: Vec::new(),
                depends_on: Vec::new(),
                verification_intent: vec!["cargo test --locked --lib a".to_string()],
                trusted_verification_commands: Vec::new(),
                handoff_notes: "handoff A".to_string(),
            },
            WorkItemOutline {
                target_repository_id: None,
                outline_id: "outline_b".to_string(),
                logical_work_item_id: "wi_b".to_string(),
                title: "B".to_string(),
                kind: WorkItemKind::Frontend,
                goal: "B".to_string(),
                scope: vec!["web/b.ts".to_string()],
                non_goals: Vec::new(),
                estimated_context_tokens: Some(10_000),
                session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
                source_story_spec_ids: vec!["story_001".to_string()],
                source_design_spec_ids: vec!["design_001".to_string()],
                exclusive_write_scopes: vec!["web/b.ts".to_string()],
                forbidden_write_scopes: Vec::new(),
                depends_on: Vec::new(),
                verification_intent: vec!["pnpm -C web test".to_string()],
                trusted_verification_commands: Vec::new(),
                handoff_notes: "handoff B".to_string(),
            },
            WorkItemOutline {
                target_repository_id: None,
                outline_id: "outline_c".to_string(),
                logical_work_item_id: "wi_c".to_string(),
                title: "C".to_string(),
                kind: WorkItemKind::Integration,
                goal: "C".to_string(),
                scope: vec!["tests/c.rs".to_string()],
                non_goals: Vec::new(),
                estimated_context_tokens: Some(8_000),
                session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
                source_story_spec_ids: vec!["story_001".to_string()],
                source_design_spec_ids: vec!["design_001".to_string()],
                exclusive_write_scopes: vec!["tests/c.rs".to_string()],
                forbidden_write_scopes: Vec::new(),
                depends_on: Vec::new(),
                verification_intent: vec!["cargo test --locked --test c".to_string()],
                trusted_verification_commands: Vec::new(),
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
    let mut outline = test_work_item_plan_outline(vec![
        WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_a".to_string(),
            to_outline_id: "outline_c".to_string(),
        },
        WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_b".to_string(),
            to_outline_id: "outline_c".to_string(),
        },
    ]);
    outline.work_item_outlines[2].depends_on =
        vec!["outline_a".to_string(), "outline_b".to_string()];

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
fn work_item_plan_outline_topological_order_uses_depends_on_as_source() {
    let mut outline = test_work_item_plan_outline(Vec::new());
    outline.work_item_outlines[2].depends_on =
        vec!["outline_a".to_string(), "outline_b".to_string()];
    outline.work_item_outlines.rotate_right(1);

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
    let mut outline = test_work_item_plan_outline(vec![
        WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_a".to_string(),
            to_outline_id: "outline_b".to_string(),
        },
        WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_b".to_string(),
            to_outline_id: "outline_a".to_string(),
        },
    ]);
    outline.work_item_outlines[0].depends_on = vec!["outline_b".to_string()];
    outline.work_item_outlines[1].depends_on = vec!["outline_a".to_string()];

    let error = work_item_plan_outline_topological_order(&outline).expect_err("cycle rejected");

    assert!(error.contains("cycle"));
    assert!(error.contains("depends_on"));
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
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: true,
        openspec_enabled: true,
        session_status: crate::product::models::WorkspaceSessionStatus::Running,
        flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
        run_history: crate::product::work_item_plan_policy::RunHistory::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        policy_diagnostics: Vec::new(),
        provider_start_ledger: Vec::new(),
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        provider_conversations: Vec::new(),
        repository_path: None,
    }
}

fn empty_provider_commands() -> mpsc::Receiver<ProviderCommand> {
    let (_tx, rx) = mpsc::channel(8);
    rx
}

fn pending_author_choice() -> PendingAuthorChoice {
    PendingAuthorChoice {
        id: "author_choice_prompt_001".to_string(),
        prompt: "输出格式？".to_string(),
        options: vec![ChoiceOptionData {
            id: "json".to_string(),
            label: "JSON".to_string(),
            description: None,
        }],
        source_node_id: None,
    }
}

#[tokio::test]
async fn story_author_choice_followup_prompt_includes_output_contract_short_reference() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        make_session("sess_story_choice_prompt"),
    );
    // choice 续跑是同一物理 provider 会话的 resume 轮（双测算裁决 2026-09-26）。
    engine.session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "author-session-1".to_string(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        last_node_id: None,
    }];
    engine.pending_author_choice = Some(pending_author_choice());

    let content = engine
        .take_pending_author_choice_prompt(
            "author_choice_prompt_001",
            vec!["json".to_string()],
            Some("保持可读性".to_string()),
        )
        .await
        .expect("Story choice followup content");

    // 应答内容保持原样（问题/选择/补充与继续生成指令不丢）；F-60 P0 后 choice
    // 点只产问答内容，不再自行拼契约（避免契约文本进入 session 历史）。
    assert!(
        content.contains(
            "用户回答了 author 的确认问题：\n问题：输出格式？\n选择：\n- JSON\n补充：保持可读性"
        ),
        "{content}"
    );
    assert!(
        content.contains("请基于该回答继续生成完整候选产物"),
        "{content}"
    );
    assert!(!content.contains("[artifact_schema_contract]"), "{content}");

    // F-60 P0（防线 2 语义保留 + 用户裁决 2026-09-26 分层）：续跑 delta 经
    // build_streaming_input 出口以一行短引用收尾——点名全部必需 heading 与
    // [REQ-*]/[AC-*]/source id 关键追踪 token，指向会话开头完整合同。
    let prompt = engine
        .build_streaming_input(content.trim(), AuthorPromptMode::DeltaOnly)
        .expect("Story choice followup streaming input")
        .prompt;
    assert!(
        prompt.starts_with("用户回答了 author 的确认问题："),
        "{prompt}"
    );
    assert!(prompt.contains("输出格式契约（本轮简引"), "{prompt}");
    assert!(prompt.contains("完整 artifact fenced block"), "{prompt}");
    assert!(
        prompt.contains("fence 内第一行是 Story Spec 一级标题"),
        "{prompt}"
    );
    assert!(prompt.contains("四反引号 ````artifact"), "{prompt}");
    assert!(prompt.contains("[artifact_schema_contract]"), "{prompt}");
    assert!(prompt.contains("`[REQ-*]`"), "{prompt}");
    assert!(prompt.contains("`[AC-*]`"), "{prompt}");
    assert!(prompt.contains("source id"), "{prompt}");
    assert!(prompt.contains("## 待确认项"), "{prompt}");
    assert!(prompt.contains("## 成功标准"), "{prompt}");
    assert!(prompt.contains("author-decision-*"), "{prompt}");
    // 分层：后续轮不重复全文（完整装配独有块不得出现）。
    assert!(!prompt.contains("输出纪律（负面清单）"), "{prompt}");
    assert!(!prompt.contains("最小结构骨架示例"), "{prompt}");
}

#[tokio::test]
async fn design_author_choice_followup_prompt_includes_output_contract_and_decision_traceability() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut session = make_session("sess_design_choice_prompt");
    session.workspace_type = WorkspaceType::Design;
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    engine.session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "author-session-1".to_string(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        last_node_id: None,
    }];
    engine.pending_author_choice = Some(pending_author_choice());

    let content = engine
        .take_pending_author_choice_prompt(
            "author_choice_prompt_001",
            vec!["json".to_string()],
            None,
        )
        .await
        .expect("Design choice followup content");
    assert!(!content.contains("[artifact_schema_contract]"), "{content}");

    let prompt = engine
        .build_streaming_input(content.trim(), AuthorPromptMode::DeltaOnly)
        .expect("Design choice followup streaming input")
        .prompt;
    assert!(prompt.contains("输出格式契约（本轮简引"), "{prompt}");
    assert!(
        prompt.contains("fence 内第一行是 Design Spec 一级标题"),
        "{prompt}"
    );
    assert!(prompt.contains("四反引号 ````artifact"), "{prompt}");
    assert!(prompt.contains("## 设计决策"), "{prompt}");
    assert!(prompt.contains("## 追踪关系"), "{prompt}");
    assert!(prompt.contains("author-decision-*"), "{prompt}");
    assert!(prompt.contains("`[DEC-*]`"), "{prompt}");
    assert!(prompt.contains("source id"), "{prompt}");
    assert!(!prompt.contains("最小结构骨架示例"), "{prompt}");
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
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        output, None,
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
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
                    .to_string()
            } else {
                complete_story_artifact("对 n <= 0 返回 0。", "n <= 0 时返回 0。")
            };
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        output,
                        Some("provider-author-session-1".to_string()),
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
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
    // normalize_generation_prompt 在转发前 trim；出口装配的契约块（F-60 P0）
    // 追加在 delta 内容之后——直通语义以「内容逐字节开头 + 末端契约在场」断言。
    assert!(inputs[1].prompt.starts_with(prompt.trim()));
    assert!(inputs[1].prompt.contains("[artifact_schema_contract]"));
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
async fn structured_choice_response_is_audited_for_reviewer_for_workspace_artifacts() {
    for (workspace_type, artifact) in [
        (
            WorkspaceType::Story,
            complete_story_artifact(
                "首次启动缺少 Claude Code 时必须阻断并提示安装。",
                "缺失 Claude Code 时不能继续生成。",
            ),
        ),
        (
            WorkspaceType::Design,
            complete_design_artifact(
                "安装策略由结构化用户交互确认。",
                "Reviewer 可追溯结构化问答来源。",
            ),
        ),
        (
            WorkspaceType::WorkItem,
            complete_work_item_artifact("持久化结构化交互审计记录。"),
        ),
    ] {
        let (event_tx, mut event_rx) = mpsc::channel(32);
        let mut session = make_session(&format!("sess_choice_audit_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.author_provider = ProviderName::ClaudeCode;
        session.reviewer_provider = Some(ProviderName::Codex);
        let checkpoint_tmp = TempDir::new().unwrap();
        let mut engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );
        let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
        let (provider_command_tx, mut provider_command_rx) = mpsc::channel(8);
        let (command_tx, command_rx) = mpsc::channel(8);
        let choice_request = ChoiceRequestData {
            id: "choice_install_policy".to_string(),
            prompt: "在产出候选 Spec 前确认安装策略。".to_string(),
            options: Vec::new(),
            allow_multiple: false,
            allow_free_text: false,
            questions: vec![
                ChoiceQuestionData {
                    id: "q1".to_string(),
                    prompt: "Claude Code 是否必装？".to_string(),
                    options: vec![
                        ChoiceOptionData {
                            id: "mandatory_blocking".to_string(),
                            label: "必装且阻断".to_string(),
                            description: Some("缺失时阻断生成。".to_string()),
                        },
                        ChoiceOptionData {
                            id: "optional".to_string(),
                            label: "可选".to_string(),
                            description: None,
                        },
                    ],
                    allow_multiple: false,
                    allow_free_text: false,
                },
                ChoiceQuestionData {
                    id: "q2".to_string(),
                    prompt: "安装记录保存在哪里？".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "global_user_dir".to_string(),
                        label: "全局用户目录".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                },
                ChoiceQuestionData {
                    id: "q3".to_string(),
                    prompt: "首版 provider 范围是什么？".to_string(),
                    options: Vec::new(),
                    allow_multiple: false,
                    allow_free_text: true,
                },
            ],
            source: ChoiceRequestSource::AskUserQuestion,
        };

        let drive = engine.drive_provider_session(ProviderSessionDriveInput {
            session: Ok(ProviderSession {
                native_session_id: None,
                events: provider_event_rx,
                commands: provider_command_tx,
            }),
            command_rx,
            node_id: Some("timeline_node_author".to_string()),
            agent: Some(ProviderName::ClaudeCode),
            role: ProviderConversationRole::Author,
            artifact_retry: None,
            revision_resume_fallback: None,
        });
        let responder = async {
            provider_event_tx
                .send(ProviderEvent::ChoiceRequest(choice_request))
                .await
                .expect("send choice request");
            let event = event_rx.recv().await.expect("choice request event");
            assert!(
                matches!(
                    event,
                    EngineEvent::ChoiceRequest {
                        id,
                        source: ChoiceRequestSource::AskUserQuestion,
                        ..
                    } if id == "choice_install_policy"
                ),
                "{workspace_type:?} should emit ask_user_question choice request"
            );
            command_tx
                .send(ProviderCommand::ChoiceResponse {
                    id: "choice_install_policy".to_string(),
                    selected_option_ids: Vec::new(),
                    free_text: None,
                    answers: vec![
                        ChoiceAnswerData {
                            question_id: "q1".to_string(),
                            selected_option_ids: vec!["mandatory_blocking".to_string()],
                            free_text: None,
                        },
                        ChoiceAnswerData {
                            question_id: "q2".to_string(),
                            selected_option_ids: vec!["global_user_dir".to_string()],
                            free_text: None,
                        },
                        ChoiceAnswerData {
                            question_id: "q3".to_string(),
                            selected_option_ids: Vec::new(),
                            free_text: Some("首版仅两个 provider".to_string()),
                        },
                    ],
                    receipt: None,
                })
                .await
                .expect("send choice response");
            assert!(
                matches!(
                    provider_command_rx.recv().await,
                    Some(ProviderCommand::ChoiceResponse { id, .. }) if id == "choice_install_policy"
                ),
                "{workspace_type:?} should forward choice response to provider"
            );
            provider_event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        artifact,
                        Some("provider-author-session-1".to_string()),
                    ),
                ))
                .await
                .expect("send completed");
        };

        tokio::join!(drive, responder);

        let review_input = engine.build_review_input().expect("review input");
        assert!(
            review_input.prompt.contains("结构化交互审计记录"),
            "{workspace_type:?} reviewer prompt should include structured choice audit: {}",
            review_input.prompt
        );
        assert!(
            review_input.prompt.contains("daemon 捕获"),
            "{workspace_type:?} reviewer prompt should mark audit as daemon-captured"
        );
        assert!(review_input.prompt.contains("ask_user_question"));
        assert!(review_input.prompt.contains("choice_install_policy"));
        assert!(review_input.prompt.contains("Claude Code 是否必装？"));
        assert!(review_input.prompt.contains("必装且阻断"));
        assert!(review_input.prompt.contains("安装记录保存在哪里？"));
        assert!(review_input.prompt.contains("全局用户目录"));
        assert!(review_input.prompt.contains("首版 provider 范围是什么？"));
        assert!(review_input.prompt.contains("首版仅两个 provider"));
    }
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
            work_item_plan_options: None,
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
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
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

// ---------------------------------------------------------------------------
// P0 1.3（REQ-WIGA-05）：三条 workspace 转发路径逐字段透传完整 answers 与
// 两层回执 receipt——provider 等待者收到时 answers 不变、receipt 同一身份。
// ---------------------------------------------------------------------------

fn choice_delivery_two_question_request() -> ChoiceRequestData {
    ChoiceRequestData {
        id: "choice_delivery_fwd".to_string(),
        prompt: "确认交付策略。".to_string(),
        options: Vec::new(),
        allow_multiple: false,
        allow_free_text: true,
        questions: vec![
            ChoiceQuestionData {
                id: "q-1".to_string(),
                prompt: "是否包含集成测试？".to_string(),
                options: vec![ChoiceOptionData {
                    id: "yes".to_string(),
                    label: "包含".to_string(),
                    description: None,
                }],
                allow_multiple: false,
                allow_free_text: false,
            },
            ChoiceQuestionData {
                id: "q-2".to_string(),
                prompt: "评审轮次？".to_string(),
                options: vec![ChoiceOptionData {
                    id: "one".to_string(),
                    label: "一轮".to_string(),
                    description: None,
                }],
                allow_multiple: false,
                allow_free_text: false,
            },
        ],
        source: ChoiceRequestSource::AskUserQuestion,
    }
}

fn choice_delivery_two_answers() -> Vec<ChoiceAnswerData> {
    vec![
        ChoiceAnswerData {
            question_id: "q-1".to_string(),
            selected_option_ids: vec!["yes".to_string()],
            free_text: None,
        },
        ChoiceAnswerData {
            question_id: "q-2".to_string(),
            selected_option_ids: vec!["one".to_string()],
            free_text: None,
        },
    ]
}

type ChoiceDeliveryCapture = std::sync::Arc<
    std::sync::Mutex<
        Option<(
            Vec<ChoiceAnswerData>,
            crate::cross_cutting::choice_delivery::ChoiceDeliverySignal,
        )>,
    >,
>;

#[tokio::test]
async fn choice_delivery_author_path_forwards_full_answers_and_receipt() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};

    let (event_tx, _event_rx) = mpsc::channel(32);
    let mut session = make_session("sess_choice_delivery_author");
    session.workspace_type = WorkspaceType::Story;
    session.author_provider = ProviderName::ClaudeCode;
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
    let (provider_command_tx, mut provider_command_rx) = mpsc::channel(8);
    let (command_tx, command_rx) = mpsc::channel(8);

    let (receipt, mut status) = ChoiceDeliverySignal::new();
    let sent_receipt = receipt.clone();
    let two_answers = choice_delivery_two_answers();
    let expected_answers = two_answers.clone();
    let captured: ChoiceDeliveryCapture = std::sync::Arc::new(std::sync::Mutex::new(None));
    let responder_captured = std::sync::Arc::clone(&captured);
    let responder = async move {
        provider_event_tx
            .send(ProviderEvent::ChoiceRequest(
                choice_delivery_two_question_request(),
            ))
            .await
            .expect("send choice request");
        // 与 drive 并发：命令先入队，drive 轮询时转发给 provider 会话。
        command_tx
            .send(ProviderCommand::ChoiceResponse {
                id: "choice_delivery_fwd".to_string(),
                selected_option_ids: Vec::new(),
                free_text: None,
                answers: two_answers.clone(),
                receipt: Some(sent_receipt),
            })
            .await
            .expect("send choice response");
        while let Some(command) = provider_command_rx.recv().await {
            if let ProviderCommand::ChoiceResponse {
                id,
                answers,
                receipt,
                ..
            } = command
                && id == "choice_delivery_fwd"
            {
                *responder_captured.lock().unwrap() =
                    Some((answers, receipt.expect("receipt forwarded")));
                let _ = provider_event_tx
                    .send(ProviderEvent::Completed(
                        crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            "author output".to_string(),
                            Some("provider-choice-delivery-author".to_string()),
                        ),
                    ))
                    .await;
                break;
            }
        }
    };

    let drive = engine.drive_provider_session(ProviderSessionDriveInput {
        session: Ok(ProviderSession {
            native_session_id: None,
            events: provider_event_rx,
            commands: provider_command_tx,
        }),
        command_rx,
        node_id: Some("timeline_node_choice_delivery_author".to_string()),
        agent: Some(ProviderName::ClaudeCode),
        role: ProviderConversationRole::Author,
        artifact_retry: None,
        revision_resume_fallback: None,
    });
    tokio::join!(drive, responder);
    let (forwarded_answers, forwarded_receipt) = captured
        .lock()
        .unwrap()
        .take()
        .expect("provider side captured forwarded command");
    assert_eq!(
        forwarded_answers, expected_answers,
        "author path answers 不变"
    );
    assert_eq!(forwarded_receipt, receipt, "author path receipt 同一身份");

    // provider 等待者接收后推进 Delivered（两层回执第二层）。
    forwarded_receipt.deliver();
    assert_eq!(*status.borrow(), ChoiceReplyState::Delivered);
}

#[tokio::test]
async fn choice_delivery_work_item_plan_path_forwards_full_answers_and_receipt() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};

    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_choice_delivery_wip");
    engine.session.session_id = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("workspace sessions")
        .into_iter()
        .find(|session| session.workspace_type == WorkspaceType::WorkItemPlan)
        .expect("work item plan session")
        .id;
    let node_id = engine.begin_work_item_plan_author_run().await;
    let (provider_event_tx, provider_event_rx) = mpsc::channel(8);
    let (provider_command_tx, mut provider_command_rx) = mpsc::channel(8);
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let (receipt, status) = ChoiceDeliverySignal::new();
    let sent_receipt = receipt.clone();
    let two_answers = choice_delivery_two_answers();
    let captured: ChoiceDeliveryCapture = std::sync::Arc::new(std::sync::Mutex::new(None));
    let provider_captured = std::sync::Arc::clone(&captured);
    tokio::spawn(async move {
        provider_event_tx
            .send(ProviderEvent::TextDelta {
                content: "draft".to_string(),
            })
            .await
            .expect("send text delta");
        while let Some(command) = provider_command_rx.recv().await {
            if let ProviderCommand::ChoiceResponse {
                id,
                answers,
                receipt,
                ..
            } = command
                && id == "choice_delivery_fwd"
            {
                *provider_captured.lock().unwrap() =
                    Some((answers, receipt.expect("receipt forwarded")));
                let _ = provider_event_tx
                    .send(ProviderEvent::Completed(
                        crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            "wip output".to_string(),
                            Some("provider-choice-delivery-wip".to_string()),
                        ),
                    ))
                    .await;
                break;
            }
        }
    });
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "choice_delivery_fwd".to_string(),
            selected_option_ids: Vec::new(),
            free_text: None,
            answers: two_answers.clone(),
            receipt: Some(sent_receipt),
        })
        .await
        .expect("send choice response");

    let output = engine
        .drive_work_item_plan_provider_session_to_output(
            Ok(ProviderSession {
                native_session_id: None,
                events: provider_event_rx,
                commands: provider_command_tx,
            }),
            &mut command_rx,
            node_id,
            ProviderName::ClaudeCode,
        )
        .await
        .expect("collector output");
    assert_eq!(output, "wip output");
    let (forwarded_answers, forwarded_receipt) = captured
        .lock()
        .unwrap()
        .take()
        .expect("provider side captured forwarded command");
    assert_eq!(forwarded_answers, two_answers, "wip path answers 不变");
    assert_eq!(forwarded_receipt, receipt, "wip path receipt 同一身份");
    forwarded_receipt.deliver();
    assert_eq!(*status.borrow(), ChoiceReplyState::Delivered);
}

#[tokio::test]
async fn choice_delivery_review_path_forwards_full_answers_and_receipt() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};
    use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;

    let tmp = TempDir::new().unwrap();
    let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().join("checkpoints")));
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
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
            work_item_plan_options: None,
        })
        .unwrap();
    let mut engine = WorkspaceEngine::new(
        checkpoint_store,
        mpsc::channel(64).0,
        WorkspaceSession::from_record(session_record),
    );
    engine.session.artifact = Some(artifact_payload("# Artifact\n\n可评审版本"));
    engine.start_review().await;

    struct ReceiptCapturingReviewProvider {
        captured: ChoiceDeliveryCapture,
    }
    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ReceiptCapturingReviewProvider {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, mut command_rx) = mpsc::channel(8);
            let captured = std::sync::Arc::clone(&self.captured);
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::ChoiceRequest(
                        choice_delivery_two_question_request(),
                    ))
                    .await;
                while let Some(command) = command_rx.recv().await {
                    if let ProviderCommand::ChoiceResponse {
                        id,
                        answers,
                        receipt,
                        ..
                    } = command
                        && id == "choice_delivery_fwd"
                    {
                        *captured.lock().unwrap() =
                            Some((answers, receipt.expect("receipt forwarded")));
                        let _ = event_tx
                            .send(ProviderEvent::Completed(
                                crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                                    "# Artifact\n\n可评审版本".to_string(),
                                    Some("provider-choice-delivery-review".to_string()),
                                ),
                            ))
                            .await;
                        break;
                    }
                }
            });
            Ok(ProviderSession {
                native_session_id: None,
                events: event_rx,
                commands: command_tx,
            })
        }

        async fn run_streaming(
            &self,
            _input: &AdapterInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
            Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "run_streaming is not used by WorkspaceEngine",
                0,
            ))
        }
    }

    let (receipt, mut status) = ChoiceDeliverySignal::new();
    let sent_receipt = receipt.clone();
    let two_answers = choice_delivery_two_answers();
    let captured: ChoiceDeliveryCapture = std::sync::Arc::new(std::sync::Mutex::new(None));
    let provider = ReceiptCapturingReviewProvider {
        captured: std::sync::Arc::clone(&captured),
    };
    let (command_tx, command_rx) = mpsc::channel(8);
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "choice_delivery_fwd".to_string(),
            selected_option_ids: Vec::new(),
            free_text: None,
            answers: two_answers.clone(),
            receipt: Some(sent_receipt),
        })
        .await
        .expect("send choice response");

    engine
        .drive_review_session(Arc::new(provider), command_rx)
        .await;

    let (forwarded_answers, forwarded_receipt) = captured
        .lock()
        .unwrap()
        .take()
        .expect("provider side captured forwarded command");
    assert_eq!(forwarded_answers, two_answers, "review path answers 不变");
    assert_eq!(forwarded_receipt, receipt, "review path receipt 同一身份");
    forwarded_receipt.deliver();
    assert_eq!(*status.borrow(), ChoiceReplyState::Delivered);
}
