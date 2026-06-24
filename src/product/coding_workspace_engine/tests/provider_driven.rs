use super::*;

struct NonProviderDrivenTestingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for NonProviderDrivenTestingProvider {}

struct ProviderDrivenTestingNoToolCallProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ProviderDrivenTestingNoToolCallProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = if input.prompt.contains("Phase: plan_tests") {
                serde_json::json!({
                    "summary": "provider planned tests",
                    "steps": [{
                        "id": "unit",
                        "title": "Unit tests",
                        "intent": "verify unit behavior",
                        "required": true,
                        "tool": "provider_managed",
                        "risk_level": "low",
                        "command_or_tool_input": {
                            "command": ["cargo", "test", "--locked", "--lib", "some_filter"]
                        },
                        "evidence_expectation": "provider supplies evidence",
                        "related_requirements": ["REQ-UNIT"],
                        "related_design_constraints": ["DEC-UNIT"],
                        "related_work_item_tasks": ["TASK-UNIT"]
                    }]
                })
                .to_string()
            } else {
                serde_json::json!({
                    "step_results": [{
                        "step_id": "unit",
                        "status": "passed",
                        "evidence_refs": ["provider-managed-unit.log"],
                        "provider_analysis": "unit evidence accepted"
                    }]
                })
                .to_string()
            };
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
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
}

struct ProviderDrivenTestingStartFailsProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ProviderDrivenTestingStartFailsProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::command_missing(
            "tester provider command not found".to_string(),
        ))
    }
}

struct ProviderDrivenTestingMissingStepResultsProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ProviderDrivenTestingMissingStepResultsProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let output = if input.prompt.contains("Phase: plan_tests") {
                serde_json::json!({
                    "summary": "provider planned tests",
                    "steps": [{
                        "id": "unit",
                        "title": "Unit tests",
                        "intent": "verify unit behavior",
                        "required": true,
                        "tool": "provider_managed",
                        "risk_level": "low",
                        "command_or_tool_input": {"command": ["cargo", "test"]},
                        "evidence_expectation": "provider supplies evidence",
                        "related_requirements": ["REQ-UNIT"],
                        "related_design_constraints": ["DEC-UNIT"],
                        "related_work_item_tasks": ["TASK-UNIT"]
                    }]
                })
                .to_string()
            } else {
                "I ran the tests and they passed.".to_string()
            };
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
}

#[test]
fn coding_provider_role_maps_to_provider_conversation_role() {
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::Coder),
        ProviderConversationRole::Coder
    );
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::Tester),
        ProviderConversationRole::Tester
    );
    assert_eq!(
        provider_conversation_role_for_coding_role(&CodingProviderRole::Analyst),
        ProviderConversationRole::Analyst
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
            role: ProviderConversationRole::Tester,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "tester-session".to_string(),
            updated_at: "2026-06-01T00:01:00Z".to_string(),
            last_node_id: Some("tester-node".to_string()),
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
            &CodingProviderRole::Tester,
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

#[tokio::test]
async fn testing_without_provider_driven_capability_routes_blocked_report_to_analyst() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let specs = vec![TestCommandSpec {
        id: "legacy_true".to_string(),
        command: vec!["true".to_string()],
    }];
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &NonProviderDrivenTestingProvider,
            &CodingExecutionContext::default(),
            &specs,
            TesterAgentOptions::default(),
        )
        .await
        .expect("blocked testing report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(report.commands.is_empty());
    assert_eq!(report.plan_id, None);
    assert!(report.steps.is_empty());
    assert_eq!(report.raw_provider_output_ref, None);
    assert!(
        report
            .context_warnings
            .iter()
            .any(|warning| warning.contains("provider_driven_testing_not_supported"))
    );
    let updated = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .len(),
        0
    );
}

#[tokio::test]
async fn real_provider_driven_testing_accepts_final_step_results_without_tool_calls() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &ProviderDrivenTestingNoToolCallProvider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("provider-driven testing");

    assert_eq!(report.overall_status, TestingOverallStatus::Passed);
    assert!(report.plan_id.is_some());
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.steps[0].step_id, "unit");
    assert_eq!(
        report.steps[0].evidence_refs,
        vec!["provider-managed-unit.log"]
    );
    assert!(report.commands.is_empty());
    assert!(report.raw_provider_output_ref.is_some());

    let chat_entries = store
        .list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("chat entries");
    assert!(chat_entries.iter().any(|entry| {
        entry.role == CodingAgentRole::Tester
            && entry.entry_type == CodingEntryType::AssistantMessage
            && entry
                .content
                .as_deref()
                .is_some_and(|content| content.contains("provider planned tests"))
            && entry
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("phase"))
                .and_then(|phase| phase.as_str())
                == Some("test_plan")
    }));
    assert!(chat_entries.iter().any(|entry| {
        entry.role == CodingAgentRole::Tester
            && entry.entry_type == CodingEntryType::AssistantMessage
            && entry
                .content
                .as_deref()
                .is_some_and(|content| content.contains("provider-managed-unit.log"))
            && entry
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("phase"))
                .and_then(|phase| phase.as_str())
                == Some("testing_result")
    }));
}

#[tokio::test]
async fn provider_driven_testing_blocks_when_provider_start_fails() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &ProviderDrivenTestingStartFailsProvider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("blocked testing report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(report.commands.is_empty());
    assert!(
        report
            .context_warnings
            .iter()
            .any(|warning| warning.contains("provider_start_failed"))
    );
}

#[tokio::test]
async fn provider_driven_testing_blocks_when_execute_output_has_no_step_results() {
    let (_root, _store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(_store, GitWorkspaceService::new(), tx);

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &ProviderDrivenTestingMissingStepResultsProvider,
            &CodingExecutionContext::default(),
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("provider-driven testing");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert_eq!(report.missing_required_steps, vec!["unit"]);
    assert!(report.raw_provider_output_ref.is_some());
}
