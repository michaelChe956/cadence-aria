use super::*;
use crate::product::coding_models::FindingSeverity;
use std::sync::Mutex;

#[derive(Default)]
struct ResumeStallThenFreshSuccessProvider {
    inputs: Mutex<Vec<StreamingProviderInput>>,
}

#[derive(Default)]
struct AlwaysResumeStallProvider {
    inputs: Mutex<Vec<StreamingProviderInput>>,
}

impl ResumeStallThenFreshSuccessProvider {
    fn recorded_inputs(&self) -> Vec<StreamingProviderInput> {
        self.inputs.lock().expect("inputs").clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ResumeStallThenFreshSuccessProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let resumed = input.resume_provider_session_id.is_some();
        self.inputs.lock().expect("inputs").push(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            if resumed {
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message:
                            "Codex resume stalled before provider progress for thread stale-thread"
                                .to_string(),
                    })
                    .await;
            } else {
                let _ = event_tx
                    .send(ProviderEvent::Completed(
                        crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            "fresh coder completed".to_string(),
                            Some("fresh-thread".to_string()),
                        ),
                    ))
                    .await;
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for AlwaysResumeStallProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().expect("inputs").push(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Failed {
                    message: "Codex resume stalled before provider progress".to_string(),
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn initial_coder_resume_stall_retries_once_with_fresh_full_prompt() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = seed_stale_coder_conversation(&store, &attempt);
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ResumeStallThenFreshSuccessProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    engine
        .execute_coding_with_commands(&attempt, &provider, &coding_context(), &mut command_rx)
        .await
        .expect("fresh retry completes coding");

    assert_fresh_retry_inputs(provider.recorded_inputs());
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Running);
    assert_eq!(
        persisted.provider_conversations[0].provider_session_id,
        "fresh-thread"
    );
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}

#[tokio::test]
async fn coder_rework_resume_stall_retries_once_with_fresh_full_prompt() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = seed_stale_coder_conversation(&store, &attempt);
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
    let provider = ResumeStallThenFreshSuccessProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let updated = engine
        .execute_coder_fix_from_review(
            &attempt,
            &review_report_requesting_changes(&attempt),
            &coding_context(),
            &provider,
            &mut command_rx,
        )
        .await
        .expect("fresh retry completes reviewer rework");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
    let inputs = provider.recorded_inputs();
    assert_fresh_retry_inputs(inputs.clone());
    assert!(inputs[1].prompt.contains("reviewer requested changes"));
    assert!(inputs[1].prompt.contains("missing validation"));
    assert!(inputs[1].prompt.contains("add validation"));
}

#[tokio::test]
async fn coder_resume_stall_does_not_retry_without_resume_id() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = AlwaysResumeStallProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(&attempt, &provider, &coding_context(), &mut command_rx)
        .await
        .expect_err("a fresh run must not retry a resume-only failure marker");

    assert!(error.to_string().contains("Codex resume stalled"));
    assert_eq!(provider.inputs.lock().expect("inputs").len(), 1);
}

#[tokio::test]
async fn coder_resume_stall_does_not_retry_non_codex_provider() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let mut provider_config = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role provider config");
    provider_config.coder = ProviderName::ClaudeCode;
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            provider_config,
        )
        .expect("update coder provider");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Coder,
                provider: ProviderName::ClaudeCode,
                provider_session_id: "claude-thread".to_string(),
                updated_at: "2026-07-13T00:00:00Z".to_string(),
                last_node_id: Some("coding_node_0001".to_string()),
            }],
        )
        .expect("seed claude coder conversation");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = AlwaysResumeStallProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_coding_with_commands(&attempt, &provider, &coding_context(), &mut command_rx)
        .await
        .expect_err("non-Codex coder must not use the Codex recovery path");

    assert!(error.to_string().contains("Codex resume stalled"));
    assert_eq!(provider.inputs.lock().expect("inputs").len(), 1);
}

fn seed_stale_coder_conversation(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    store
        .replace_attempt_provider_conversations(
            &attempt.id,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Coder,
                provider: ProviderName::Codex,
                provider_session_id: "stale-thread".to_string(),
                updated_at: "2026-07-13T00:00:00Z".to_string(),
                last_node_id: Some("coding_node_0001".to_string()),
            }],
        )
        .expect("seed stale coder conversation")
}

fn coding_context() -> CodingExecutionContext {
    CodingExecutionContext {
        work_item_markdown: Some("# Draft 4\n\n- 修复 Provider 恢复问题".to_string()),
        verification_commands: vec!["cargo test --locked --lib coder_resume_recovery".to_string()],
    }
}

fn assert_fresh_retry_inputs(inputs: Vec<StreamingProviderInput>) {
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs[0].resume_provider_session_id.as_deref(),
        Some("stale-thread")
    );
    assert_eq!(inputs[1].resume_provider_session_id, None);
    assert!(inputs[0].prompt.contains("增量代码编写指令"));
    assert!(inputs[1].prompt.contains("已确认 Work Item"));
}

fn review_report_requesting_changes(attempt: &CodingExecutionAttempt) -> CodeReviewReport {
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
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
        }],
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "reviewer requested changes".to_string(),
        created_at: "2026-07-13T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    }
}
