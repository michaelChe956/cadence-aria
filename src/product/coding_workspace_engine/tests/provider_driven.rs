use super::*;
use crate::product::coding_models::FindingSeverity;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct ReviewerDrivenReworkProvider {
    input: Arc<Mutex<Option<StreamingProviderInput>>>,
}

struct NonJsonCodeReviewProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for NonJsonCodeReviewProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = "验证完成。结论：当前实现还有问题，需要人工介入。".to_string();
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        output, None,
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

impl ReviewerDrivenReworkProvider {
    fn recorded_input(&self) -> StreamingProviderInput {
        self.input
            .lock()
            .expect("input lock")
            .clone()
            .expect("recorded input")
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewerDrivenReworkProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        *self.input.lock().expect("input lock") = Some(input);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = "coder fixed reviewer findings".to_string();
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        output,
                        Some("coder-session-after-rework".to_string()),
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[test]
fn coding_provider_role_maps_to_provider_conversation_role() {
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::Coder),
        ProviderConversationRole::Coder
    );
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::CodeReviewer),
        ProviderConversationRole::CodeReviewer
    );
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::InternalReviewer),
        ProviderConversationRole::InternalReviewer
    );
}

#[test]
fn coding_provider_resume_session_id_is_isolated_by_role_and_provider() {
    let store = CodingAttemptStore::new(ProductAppPaths::new(
        tempdir().expect("tempdir").path().join(".aria"),
    ));
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let mut attempt = test_attempt("coding_attempt_0001");
    attempt.provider_conversations = vec![
        ProviderConversationRef {
            role: ProviderConversationRole::Coder,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "coder-session".to_string(),
            updated_at: "2026-06-01T00:00:00Z".to_string(),
            last_node_id: Some("coder-node".to_string()),
        },
        ProviderConversationRef {
            role: ProviderConversationRole::CodeReviewer,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "reviewer-session".to_string(),
            updated_at: "2026-06-01T00:01:00Z".to_string(),
            last_node_id: Some("reviewer-node".to_string()),
        },
    ];

    assert_eq!(
        engine.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Coder,
            &ProviderName::ClaudeCode,
        ),
        Some("coder-session".to_string())
    );
    assert_eq!(
        engine.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::CodeReviewer,
            &ProviderName::ClaudeCode,
        ),
        None
    );
    assert_eq!(
        engine.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Coder,
            &ProviderName::Codex,
        ),
        None
    );
}

#[test]
fn clearing_coder_conversation_preserves_other_roles() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::Codex,
                    provider_session_id: "stale-coder-thread".to_string(),
                    updated_at: "2026-07-13T00:00:00Z".to_string(),
                    last_node_id: Some("coding_node_0001".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::CodeReviewer,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "review-thread".to_string(),
                    updated_at: "2026-07-13T00:00:01Z".to_string(),
                    last_node_id: Some("coding_node_0002".to_string()),
                },
            ],
        )
        .expect("seed conversations");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let updated = engine
        .clear_attempt_provider_conversation(
            &attempt,
            &CodingProviderRole::Coder,
            &ProviderName::Codex,
        )
        .expect("clear coder conversation");

    assert_eq!(updated.provider_conversations.len(), 1);
    assert_eq!(
        updated.provider_conversations[0].role,
        ProviderConversationRole::CodeReviewer
    );
}

#[tokio::test]
async fn reviewer_driven_rework_increments_rework_count_and_resumes_coder() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Coder,
                provider: ProviderName::Codex,
                provider_session_id: "coder-session-before-rework".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("coding_node_0001".to_string()),
            }],
        )
        .expect("record coder conversation");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ReviewerDrivenReworkProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let updated = engine
        .execute_coder_fix_from_review(
            &attempt,
            &review_report_requesting_changes(&attempt),
            &CodingExecutionContext::default(),
            &provider,
            &mut command_rx,
        )
        .await
        .expect("coder fix from review");

    assert_eq!(updated.rework_count, 1);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    assert_eq!(updated.status, CodingAttemptStatus::Running);

    let instructions = store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    assert_eq!(
        instructions[0].source_stage,
        CodingExecutionStage::CodeReview
    );
    assert_eq!(instructions[0].rework_round, 1);
    assert!(
        instructions[0]
            .fix_hints
            .iter()
            .any(|hint| hint.contains("src/lib.rs:42 missing validation"))
    );

    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes");
    let coder_retry_node = nodes
        .iter()
        .find(|node| {
            node.stage == CodingExecutionStage::Coding
                && node.agent_role == Some(CodingAgentRole::Author)
                && node
                    .summary
                    .as_deref()
                    .is_some_and(|summary| summary.contains("reviewer 修复 round 1"))
        })
        .expect("coder retry timeline node");
    assert_eq!(coder_retry_node.title, "代码编写");
    assert_eq!(coder_retry_node.status, CodingTimelineNodeStatus::Completed);
    assert!(
        coder_retry_node
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("reviewer 修复 round 1"))
    );

    let role_run = store
        .latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
        )
        .expect("role run")
        .expect("coder retry role run");
    assert_eq!(role_run.status, CodingRoleRunStatus::Completed);

    let chat_entries = store
        .list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("chat entries");
    let rework_entry = chat_entries
        .iter()
        .find(|entry| entry.node_id.as_deref() == Some(coder_retry_node.id.as_str()))
        .expect("coder retry chat entry");
    assert_eq!(rework_entry.role, CodingAgentRole::Author);
    assert_eq!(rework_entry.entry_type, CodingEntryType::AssistantMessage);
    assert_eq!(
        rework_entry.content.as_deref(),
        Some("coder fixed reviewer findings")
    );
    let metadata = rework_entry.metadata.as_ref().expect("rework metadata");
    assert_eq!(metadata["source"], "coding");
    assert_eq!(metadata["role_run_id"].as_str(), Some(role_run.id.as_str()));
    assert_eq!(
        metadata["raw_provider_output_ref"],
        "provider-raw/coding/coder_output_0001.txt"
    );

    let input = provider.recorded_input();
    assert_eq!(
        input.resume_provider_session_id.as_deref(),
        Some("coder-session-before-rework")
    );
    assert!(input.prompt.contains("本轮修复要求"));
    assert!(input.prompt.contains("missing validation"));
    assert!(input.prompt.contains("add validation"));
}

#[tokio::test]
async fn coder_fix_from_review_blocks_when_auto_fix_limit_is_reached() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = store
        .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("first rework count");
    let attempt = store
        .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("second rework count");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ReviewerDrivenReworkProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let updated = engine
        .execute_coder_fix_from_review(
            &attempt,
            &review_report_requesting_changes(&attempt),
            &CodingExecutionContext::default(),
            &provider,
            &mut command_rx,
        )
        .await
        .expect("blocked coder fix from review");

    assert_eq!(updated.status, CodingAttemptStatus::WaitingForHuman);
    assert_eq!(updated.rework_count, 2);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    assert!(provider.input.lock().expect("input lock").is_none());
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].stage, Some(CodingExecutionStage::CodeReview));
    assert_eq!(gates[0].role, Some(CodingProviderRole::CodeReviewer));
    assert!(gates[0].title.contains("修复超上限"));
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["provide_context", "send_to_coder", "abort"]
    );
}

#[tokio::test]
async fn blocked_code_review_without_structured_findings_accepts_manual_feedback_for_coder() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt
        .worktree_path
        .as_ref()
        .expect("attempt worktree")
        .clone();
    init_test_git_repo(&worktree);
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let report = engine
        .execute_code_review(&attempt, &NonJsonCodeReviewProvider)
        .await
        .expect("code review blocks on non-json output");

    assert_eq!(report.verdict, ReviewVerdict::Blocked);
    assert!(
        report.findings.is_empty(),
        "non-json reviewer output has no structured findings"
    );
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].reason_code.as_deref(), Some("code_review_blocked"));
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_review", "send_to_coder", "abort"]
    );

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gates[0].gate_id,
            "send_to_coder",
            Some("人工意见：先按用户说明修复 JSON 输出协议".to_string()),
        )
        .await
        .expect("send manual feedback to coder");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.rework_count, 1);
    let notes = store
        .list_context_notes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("context notes");
    assert!(
        notes.iter().any(|note| note
            .content
            .contains("人工意见：先按用户说明修复 JSON 输出协议")),
        "manual feedback must be preserved for the next coder prompt"
    );
    drop(root);
}

pub(super) fn review_report_requesting_changes(
    attempt: &CodingExecutionAttempt,
) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_report_0001".to_string(),
        attempt_id: attempt.id.clone(),
        round: 1,
        verdict: ReviewVerdict::RequestChanges,
        findings: vec![ReviewFinding {
            severity: FindingSeverity::Error,
            file_path: Some("src/lib.rs".to_string()),
            line: Some(42),
            message: "missing validation".to_string(),
            required_action: Some("add validation".to_string()),
            source_stage: CodingExecutionStage::CodeReview,
            evidence: vec!["review-output.log".to_string()],
            plan_defect_evidence: Vec::new(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
            defect_class: crate::product::models::PlanDefectClass::ImplementationDefect,
            reason_code: None,
            contract_refs: Vec::new(),
            capability_refs: Vec::new(),
            repair_target: None,
            recommended_route: crate::product::models::PlanDefectRoute::CoderRework,
            confidence: None,
        }],
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "reviewer requested changes".to_string(),
        created_at: "2026-06-01T00:00:00Z".to_string(),
        raw_provider_output_ref: Some("provider-raw/code-review.txt".to_string()),
        role_run_id: None,
        run_no: None,
    }
}
