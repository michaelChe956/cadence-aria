use std::fs;
use std::sync::{Arc, Mutex};

use cadence_aria::cross_cutting::provider_adapter::ProviderAdapterError;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderSession, ProviderToolCall, StreamingProviderAdapter,
    StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use cadence_aria::product::coding_models::{
    CodingAttemptStatus, CodingEntryType, CodingExecutionStage, CodingTimelineNodeStatus,
    TestCommandStatus, TestingOverallStatus,
};
use cadence_aria::product::coding_workspace_engine::{
    CodingExecutionContext, CodingWorkspaceEngine,
};
use cadence_aria::product::git_workspace_service::GitWorkspaceService;
use cadence_aria::product::models::ProviderName;
use cadence_aria::product::test_executor::TestCommandSpec;
use cadence_aria::product::tester_agent_loop::{
    TesterAgentOptions, execute_tester_tool_call, tester_allowed_tools,
};
use cadence_aria::web::coding_ws_handler::CodingWsOutMessage;
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn tester_tool_whitelist_rejects_write_tools_without_touching_worktree() {
    let root = tempdir().expect("root");
    let call = ProviderToolCall {
        id: "tool_call_0001".to_string(),
        tool_name: "write_file".to_string(),
        input: json!({"path": "generated.py", "content": "unsafe"}),
    };

    let artifact_root = root.path().join("attempt-artifacts/test-output");
    let outcome = execute_tester_tool_call(&call, root.path(), &artifact_root)
        .await
        .expect("execute tool call");

    assert_eq!(
        tester_allowed_tools(),
        ["run_command", "read_file", "list_files", "search_code"]
    );
    assert!(outcome.result.is_error);
    assert_eq!(outcome.result.tool_use_id, "tool_call_0001");
    assert!(outcome.result.output.contains("Tester 不允许修改文件"));
    assert!(outcome.command.is_none());
    assert!(!root.path().join("generated.py").exists());
}

#[tokio::test]
async fn tester_run_command_executes_in_worktree_and_records_artifacts() {
    let root = tempdir().expect("root");
    let call = ProviderToolCall {
        id: "tool_call_0001".to_string(),
        tool_name: "run_command".to_string(),
        input: json!({"command": ["sh", "-c", "printf ok"]}),
    };

    let artifact_root = root.path().join("attempt-artifacts/test-output");
    let outcome = execute_tester_tool_call(&call, root.path(), &artifact_root)
        .await
        .expect("execute tool call");

    let command = outcome.command.expect("recorded command");
    assert_eq!(command.status, TestCommandStatus::Passed);
    assert_eq!(
        fs::read_to_string(artifact_root.join(&command.stdout_ref)).expect("stdout"),
        "ok"
    );
    assert!(!outcome.result.is_error);
    assert!(outcome.result.output.contains("\"status\":\"passed\""));
}

#[tokio::test]
async fn execute_testing_with_tool_provider_streams_tool_entries_and_persists_report() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree");
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
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let captured_prompt = Arc::new(Mutex::new(None));
    let provider = ScriptedTesterProvider {
        prompt: Arc::clone(&captured_prompt),
    };
    let specs = vec![TestCommandSpec {
        id: "planned_001".to_string(),
        command: vec!["sh".to_string(), "-c".to_string(), "printf ok".to_string()],
    }];
    let context = CodingExecutionContext {
        work_item_markdown: Some("# Work Item\n\n## 验证命令\n\n- `sh -c 'printf ok'`".to_string()),
        verification_commands: vec!["sh -c 'printf ok'".to_string()],
    };

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &provider,
            &context,
            &specs,
            TesterAgentOptions::default(),
        )
        .await
        .expect("execute testing");

    assert_eq!(report.overall_status, TestingOverallStatus::Passed);
    assert_eq!(report.commands.len(), 1);
    assert_eq!(report.commands[0].status, TestCommandStatus::Passed);
    assert_eq!(
        report
            .provider_claim
            .as_ref()
            .and_then(|value| value.get("summary")),
        Some(&json!("ok"))
    );
    assert!(report.backend_verified);
    assert!(
        captured_prompt
            .lock()
            .expect("prompt")
            .as_ref()
            .expect("captured prompt")
            .contains("Tester Agent Loop")
    );

    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    assert!(events.iter().any(|event| matches!(
        event,
        CodingWsOutMessage::CodingTimelineNodeCreated { node }
            if node.stage == CodingExecutionStage::Testing
                && node.status == CodingTimelineNodeStatus::Running
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        CodingWsOutMessage::CodingChatEntryCreated { entry }
            if matches!(entry.entry_type, CodingEntryType::ToolCall { ref tool_name, .. } if tool_name == "run_command")
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        CodingWsOutMessage::CodingChatEntryCreated { entry }
            if matches!(entry.entry_type, CodingEntryType::ToolResult { is_error: false, .. })
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        CodingWsOutMessage::TestingReportUpdate { report }
            if report.overall_status == TestingOverallStatus::Passed
    )));
}

struct ScriptedTesterProvider {
    prompt: Arc<Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScriptedTesterProvider {
    fn supports_tool_calls(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        *self.prompt.lock().expect("prompt lock") = Some(input.prompt);
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            event_tx
                .send(ProviderEvent::ToolCall(ProviderToolCall {
                    id: "tool_call_0001".to_string(),
                    tool_name: "run_command".to_string(),
                    input: json!({"command": ["sh", "-c", "printf ok"]}),
                }))
                .await
                .expect("send tool call");
            match command_rx.recv().await.expect("tool result command") {
                ProviderCommand::ToolResult(result) => {
                    assert_eq!(result.tool_use_id, "tool_call_0001");
                    assert!(!result.is_error);
                    assert!(result.output.contains("\"status\":\"passed\""));
                }
                other => panic!("expected tool result, got {other:?}"),
            }
            event_tx
                .send(ProviderEvent::Completed {
                    full_output: r#"{"summary":"ok","bugs_found":[]}"#.to_string(),
                    provider_session_id: None,
                })
                .await
                .expect("send completed");
        });

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}
