use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn provider_testing_cancellation_aborts_role_run_without_late_tool_result_or_artifacts() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let entered = worktree.join("tester-entered");
    let late = worktree.join("tester-late");
    let shell = format!(
        "printf entered > {}; sleep 1; printf late > {}",
        entered.display(),
        late.display()
    );
    let command = vec!["sh".to_string(), "-c".to_string(), shell];
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let successful_tool_result_seen = Arc::new(AtomicBool::new(false));
    let provider = CancellableTesterProvider {
        command: command.clone(),
        successful_tool_result_seen: Arc::clone(&successful_tool_result_seen),
    };
    let cancellation = CancellationToken::new();
    let (event_tx, _event_rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_cancellation(cancellation.clone());
    let execute = tokio::spawn({
        let attempt = attempt.clone();
        async move {
            engine
                .execute_testing_with_provider(
                    &attempt,
                    &provider,
                    &CodingExecutionContext::default(),
                    &[TestCommandSpec {
                        id: "planned_001".to_string(),
                        command,
                    }],
                    TesterAgentOptions::default(),
                )
                .await
        }
    });
    wait_for_path(&entered).await;

    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), execute)
        .await
        .expect("tester cancellation must be bounded")
        .expect("testing task");

    assert!(matches!(result, Err(CodingWorkspaceEngineError::Aborted)));
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        !late.exists(),
        "cancelled tester command wrote a late side effect"
    );
    assert!(!successful_tool_result_seen.load(Ordering::SeqCst));
    let artifact_root =
        store.attempt_test_output_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
    assert!(!artifact_root.join("tool_call_0001.stdout.log").exists());
    assert!(!artifact_root.join("tool_call_0001.stderr.log").exists());
    let role_run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .pop()
        .expect("tester role run");
    assert_eq!(role_run.status, CodingRoleRunStatus::Aborted);
    let events = store
        .list_role_run_events(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        )
        .expect("role run events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::Aborted)
    );
    assert!(
        events
            .iter()
            .all(|event| event.event_type != CodingRoleRunEventType::ToolResult)
    );
}

#[tokio::test]
async fn cancellation_after_artifact_settle_wins_before_tool_result_commit() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        "printf committed-output".to_string(),
    ];
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let successful_tool_result_seen = Arc::new(AtomicBool::new(false));
    let provider = CancellableTesterProvider {
        command: command.clone(),
        successful_tool_result_seen: Arc::clone(&successful_tool_result_seen),
    };
    let artifact_root =
        store.attempt_test_output_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
    let (_pause, reached, resume) = testing_provider::register_tester_tool_commit_pause(
        &artifact_root,
        testing_provider::TesterToolCommitTestPoint::BeforeProviderSend,
    );
    let cancellation = CancellationToken::new();
    let (event_tx, _event_rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_cancellation(cancellation.clone());
    let execute = tokio::spawn({
        let attempt = attempt.clone();
        async move {
            engine
                .execute_testing_with_provider(
                    &attempt,
                    &provider,
                    &CodingExecutionContext::default(),
                    &[TestCommandSpec {
                        id: "planned_001".to_string(),
                        command,
                    }],
                    TesterAgentOptions::default(),
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_millis(500), reached)
        .await
        .expect("tester tool commit boundary must be reached")
        .expect("tester tool commit pause sender");

    cancellation.cancel();
    resume.send(()).expect("resume tester tool commit");
    let result = tokio::time::timeout(Duration::from_secs(2), execute)
        .await
        .expect("tester cancellation must be bounded")
        .expect("testing task");

    assert!(matches!(result, Err(CodingWorkspaceEngineError::Aborted)));
    assert!(!successful_tool_result_seen.load(Ordering::SeqCst));
    assert!(!artifact_root.join("tool_call_0001.stdout.log").exists());
    assert!(!artifact_root.join("tool_call_0001.stderr.log").exists());
    let role_run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .pop()
        .expect("tester role run");
    assert_eq!(role_run.status, CodingRoleRunStatus::Aborted);
    let events = store
        .list_role_run_events(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        )
        .expect("role run events");
    assert!(
        events
            .iter()
            .all(|event| event.event_type != CodingRoleRunEventType::ToolResult)
    );
    let chat_entries = store
        .list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("chat entries");
    assert!(
        chat_entries
            .iter()
            .all(|entry| !matches!(entry.entry_type, CodingEntryType::ToolResult { .. }))
    );
}

#[tokio::test]
async fn cancellation_after_tool_result_send_prevents_late_store_commit() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        "printf committed-output".to_string(),
    ];
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let successful_tool_result_seen = Arc::new(AtomicBool::new(false));
    let provider = CancellableTesterProvider {
        command: command.clone(),
        successful_tool_result_seen: Arc::clone(&successful_tool_result_seen),
    };
    let artifact_root =
        store.attempt_test_output_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
    let (_pause, reached, resume) = testing_provider::register_tester_tool_commit_pause(
        &artifact_root,
        testing_provider::TesterToolCommitTestPoint::AfterProviderSend,
    );
    let cancellation = CancellationToken::new();
    let (event_tx, _event_rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_cancellation(cancellation.clone());
    let execute = tokio::spawn({
        let attempt = attempt.clone();
        async move {
            engine
                .execute_testing_with_provider(
                    &attempt,
                    &provider,
                    &CodingExecutionContext::default(),
                    &[TestCommandSpec {
                        id: "planned_001".to_string(),
                        command,
                    }],
                    TesterAgentOptions::default(),
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_millis(500), reached)
        .await
        .expect("post-send tester tool boundary must be reached")
        .expect("post-send tester tool pause sender");
    wait_for_flag(&successful_tool_result_seen).await;

    cancellation.cancel();
    resume.send(()).expect("resume tester tool commit");
    let result = tokio::time::timeout(Duration::from_secs(2), execute)
        .await
        .expect("tester cancellation must be bounded")
        .expect("testing task");

    assert!(matches!(result, Err(CodingWorkspaceEngineError::Aborted)));
    assert!(successful_tool_result_seen.load(Ordering::SeqCst));
    assert!(!artifact_root.join("tool_call_0001.stdout.log").exists());
    assert!(!artifact_root.join("tool_call_0001.stderr.log").exists());
    let role_run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .pop()
        .expect("tester role run");
    assert_eq!(role_run.status, CodingRoleRunStatus::Aborted);
    let events = store
        .list_role_run_events(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        )
        .expect("role run events");
    assert!(
        events
            .iter()
            .all(|event| event.event_type != CodingRoleRunEventType::ToolResult)
    );
    let chat_entries = store
        .list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("chat entries");
    assert!(
        chat_entries
            .iter()
            .all(|entry| !matches!(entry.entry_type, CodingEntryType::ToolResult { .. }))
    );
}

struct CancellableTesterProvider {
    command: Vec<String>,
    successful_tool_result_seen: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CancellableTesterProvider {
    fn supports_tool_calls(&self) -> bool {
        true
    }

    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let command = self.command.clone();
        let successful_tool_result_seen = Arc::clone(&self.successful_tool_result_seen);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            if input.prompt.contains("Phase: plan_tests") {
                let _ = event_tx
                    .send(ProviderEvent::Completed(
                        crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            json!({
                                "summary": "cancellable tester command",
                                "steps": [{
                                    "id": "planned_001",
                                    "title": "Long command",
                                    "intent": "prove cancellation reaches the real command",
                                    "required": true,
                                    "tool": "run_command",
                                    "risk_level": "low",
                                    "command_or_tool_input": { "command": command },
                                    "evidence_expectation": "command is cancelled without late output",
                                    "related_requirements": ["REQ-CANCEL"],
                                    "related_design_constraints": ["DEC-CANCEL"],
                                    "related_work_item_tasks": ["TASK-CANCEL"]
                                }]
                            })
                            .to_string(),
                            None,
                        ),
                    ))
                    .await;
                return;
            }

            if event_tx
                .send(ProviderEvent::ToolCall(ProviderToolCall {
                    id: "tool_call_0001".to_string(),
                    tool_name: "run_command".to_string(),
                    input: json!({
                        "step_id": "planned_001",
                        "command": command
                    }),
                }))
                .await
                .is_err()
            {
                return;
            }
            tokio::select! {
                _ = cancel.cancelled() => {}
                command = command_rx.recv() => {
                    if let Some(ProviderCommand::ToolResult(result)) = command
                        && !result.is_error
                    {
                        successful_tool_result_seen.store(true, Ordering::SeqCst);
                    }
                }
            }
        });

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

async fn wait_for_path(path: &Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("command did not reach entered marker");
}

async fn wait_for_flag(flag: &AtomicBool) {
    tokio::time::timeout(Duration::from_millis(500), async {
        while !flag.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider must receive the tester ToolResult");
}
