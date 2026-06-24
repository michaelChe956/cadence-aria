use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::LifecycleStore;
use crate::protocol::contracts::{AdapterRole, ProviderType};

use super::{
    ReviewFixture, TestControlledFakeStreamingProvider, TestControls, TestingFixture,
    WorkspaceSocketControl, create_large_workspace_fixture, test_controls_enabled,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn test_controls_are_disabled_without_e2e_env() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    unsafe {
        std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
    }

    assert!(!test_controls_enabled());
}

#[test]
fn test_controls_are_enabled_in_e2e_env() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    unsafe {
        std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
    }

    assert!(test_controls_enabled());

    unsafe {
        std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
    }
}

#[test]
fn large_workspace_fixture_contains_large_lazy_loaded_content() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let app_paths = ProductAppPaths::new(temp_dir.path());

    let session_id = create_large_workspace_fixture(app_paths.clone()).expect("large fixture");

    let lifecycle = LifecycleStore::new(app_paths);
    let nodes = lifecycle
        .load_timeline_nodes(&session_id)
        .expect("timeline nodes");
    let provider_nodes = nodes
        .iter()
        .filter(|node| {
            matches!(
                node.node_type,
                crate::web::workspace_ws_types::TimelineNodeType::AuthorRun
                    | crate::web::workspace_ws_types::TimelineNodeType::ReviewerRun
            )
        })
        .count();
    let detail = lifecycle
        .load_node_detail(&session_id, "timeline_node_034")
        .expect("first node detail");
    let output = detail.execution_events[0]
        .get("output")
        .and_then(|value| value.as_str())
        .expect("large output");
    let artifacts = lifecycle
        .list_artifact_versions(&session_id)
        .expect("artifact versions");

    assert_eq!(nodes.len(), 45);
    assert!(provider_nodes >= 10);
    assert!(detail.prompt.expect("large prompt").len() > 100_000);
    assert!(output.len() > 100_000);
    assert_eq!(artifacts.len(), 5);
    assert!(artifacts[4].markdown().contains("# Large Artifact v5"));
}

#[tokio::test]
async fn workspace_socket_drop_notifies_registered_session_connection() {
    let controls = TestControls::default();
    let (tx, mut rx) = mpsc::channel(1);
    controls
        .register_workspace_socket("workspace_session_1".to_string(), tx)
        .await;

    let dropped = controls.drop_workspace_socket("workspace_session_1").await;

    assert!(dropped);
    assert_eq!(
        rx.recv().await,
        Some(WorkspaceSocketControl::CloseForTestDrop)
    );
}

#[tokio::test]
async fn workspace_socket_drop_waits_for_late_session_registration() {
    let controls = TestControls::default();
    let delayed_controls = controls.clone();
    let (tx, mut rx) = mpsc::channel(1);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        delayed_controls
            .register_workspace_socket("workspace_session_1".to_string(), tx)
            .await;
    });

    let dropped = controls
        .drop_workspace_socket_when_registered("workspace_session_1", Duration::from_millis(200))
        .await;

    assert!(dropped);
    assert_eq!(
        rx.recv().await,
        Some(WorkspaceSocketControl::CloseForTestDrop)
    );
}

#[tokio::test]
async fn workspace_socket_rejects_are_consumed_per_session() {
    let controls = TestControls::default();
    controls
        .reject_next_workspace_sockets("workspace_session_1".to_string(), 2)
        .await;

    assert!(
        controls
            .consume_workspace_socket_reject("workspace_session_1")
            .await
    );
    assert!(
        controls
            .consume_workspace_socket_reject("workspace_session_1")
            .await
    );
    assert!(
        !controls
            .consume_workspace_socket_reject("workspace_session_1")
            .await
    );
    assert!(
        !controls
            .consume_workspace_socket_reject("workspace_session_2")
            .await
    );
}

#[tokio::test]
async fn permission_fixture_is_session_scoped_and_consumed_once() {
    let controls = TestControls::default();

    assert!(
        !controls
            .consume_permission_fixture("workspace_session_1")
            .await
    );

    controls
        .enable_permission_fixture("workspace_session_1".to_string())
        .await;

    assert!(
        controls
            .consume_permission_fixture("workspace_session_1")
            .await
    );
    assert!(
        !controls
            .consume_permission_fixture("workspace_session_1")
            .await
    );
    assert!(
        !controls
            .consume_permission_fixture("workspace_session_2")
            .await
    );
}

#[tokio::test]
async fn review_fixture_is_session_scoped_and_consumed_once() {
    let controls = TestControls::default();

    assert!(
        controls
            .consume_review_fixture("workspace_session_1")
            .await
            .is_none()
    );

    controls
        .enable_review_fixture(
            "workspace_session_1".to_string(),
            ReviewFixture {
                verdict: "revise".to_string(),
                summary: "补充异常路径".to_string(),
                comments: "需要补充失败路径。".to_string(),
                raw_json: None,
                raw_text: None,
                findings: Vec::new(),
            },
        )
        .await;

    let fixture = controls
        .consume_review_fixture("workspace_session_1")
        .await
        .expect("review fixture");

    assert_eq!(fixture.verdict, "revise");
    assert_eq!(fixture.summary, "补充异常路径");
    assert!(
        controls
            .consume_review_fixture("workspace_session_1")
            .await
            .is_none()
    );
    assert!(
        controls
            .consume_review_fixture("workspace_session_2")
            .await
            .is_none()
    );
}

#[tokio::test]
async fn permission_timeout_override_defaults_and_updates() {
    let controls = TestControls::default();

    assert_eq!(controls.permission_timeout(), Duration::from_secs(900));

    controls
        .set_permission_timeout(Duration::from_millis(500))
        .await;

    assert_eq!(controls.permission_timeout(), Duration::from_millis(500));
}

#[tokio::test]
async fn server_idle_timeout_override_defaults_and_updates() {
    let controls = TestControls::default();

    assert_eq!(controls.server_idle_timeout(), Duration::from_secs(90));

    controls
        .set_server_idle_timeout(Duration::from_millis(750))
        .await;

    assert_eq!(controls.server_idle_timeout(), Duration::from_millis(750));
}

#[tokio::test]
async fn permission_fixture_fake_provider_waits_for_approval_and_completes() {
    let controls = TestControls::default();
    controls
        .enable_permission_fixture("workspace_session_1".to_string())
        .await;
    let provider = TestControlledFakeStreamingProvider::new(controls);
    let mut session = provider
        .start(
            streaming_input("workspace_session_1"),
            CancellationToken::new(),
        )
        .await
        .expect("provider session");

    match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
        .await
        .expect("stream event")
        .expect("text delta")
    {
        ProviderEvent::TextDelta { content } => {
            assert!(content.contains("E2E permission fixture stream"));
        }
        other => panic!("unexpected provider event: {other:?}"),
    }

    let request_id = match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
        .await
        .expect("permission event")
        .expect("permission request")
    {
        ProviderEvent::PermissionRequest(request) => request.id,
        other => panic!("unexpected provider event: {other:?}"),
    };
    session
        .commands
        .send(ProviderCommand::PermissionResponse {
            id: request_id,
            approved: true,
            reason: None,
        })
        .await
        .expect("send approval");

    for _ in 0..3 {
        match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
            .await
            .expect("completed event")
            .expect("completed")
        {
            ProviderEvent::Completed { full_output, .. } => {
                assert!(full_output.contains("Permission Fixture"));
                return;
            }
            ProviderEvent::TextDelta { .. } => {}
            other => panic!("unexpected provider event: {other:?}"),
        }
    }
    panic!("permission fixture did not complete");
}

#[tokio::test]
async fn review_fixture_fake_provider_emits_json_contract_for_reviewer() {
    let controls = TestControls::default();
    controls
        .enable_review_fixture(
            "workspace_session_1".to_string(),
            ReviewFixture {
                verdict: "revise".to_string(),
                summary: "补充异常路径".to_string(),
                comments: "需要补充失败路径。".to_string(),
                raw_json: None,
                raw_text: None,
                findings: Vec::new(),
            },
        )
        .await;
    let provider = TestControlledFakeStreamingProvider::new(controls);
    let mut session = provider
        .start(
            StreamingProviderInput {
                provider_type: ProviderType::Codex,
                role: AdapterRole::Reviewer,
                prompt: "请作为 reviewer 审核当前 Workspace 产物。".to_string(),
                working_dir: std::env::current_dir().expect("current dir"),
                workspace_session_id: Some("workspace_session_1".to_string()),
                resume_provider_session_id: None,
                permission_mode: ProviderPermissionMode::Supervised,
                env_vars: Default::default(),
                timeout_secs: 60,
            },
            CancellationToken::new(),
        )
        .await
        .expect("review fixture provider session");

    let mut output = String::new();
    while let Some(event) = session.events.recv().await {
        match event {
            ProviderEvent::TextDelta { content } => output.push_str(&content),
            ProviderEvent::Completed { full_output, .. } => {
                output.push_str(&full_output);
                break;
            }
            _ => {}
        }
    }

    assert!(output.contains("需要补充失败路径。"));
    assert!(output.contains("\"verdict\":\"revise\""));
    assert!(output.contains("\"summary\":\"补充异常路径\""));
}

#[tokio::test]
async fn testing_fixture_fake_provider_emits_plan_and_step_results() {
    let controls = TestControls::default();
    controls
        .enable_testing_fixture(
            "coding_attempt_0001".to_string(),
            TestingFixture {
                plan_output: json!({
                    "summary": "controlled QA plan",
                    "steps": [
                        {
                            "id": "unit",
                            "title": "Unit tests",
                            "intent": "prove unit behavior",
                            "required": true,
                            "tool": "run_command",
                            "risk_level": "low",
                            "command_or_tool_input": {"command": ["true"]},
                            "evidence_expectation": "exit 0",
                            "related_requirements": ["REQ-UNIT"],
                            "related_design_constraints": ["DEC-UNIT"],
                            "related_work_item_tasks": ["TASK-UNIT"]
                        },
                        {
                            "id": "security",
                            "title": "Security check",
                            "intent": "prove security checklist",
                            "required": true,
                            "tool": "provider_managed",
                            "risk_level": "medium",
                            "command_or_tool_input": {"note": "controlled missing step"},
                            "evidence_expectation": "provider evidence",
                            "related_requirements": ["REQ-SECURITY"],
                            "related_design_constraints": ["DEC-SECURITY"],
                            "related_work_item_tasks": ["TASK-SECURITY"]
                        }
                    ]
                }),
                step_results: vec![json!({"step_id": "unit", "status": "passed"})],
                malformed_plan_output: None,
                provider_failure: None,
            },
        )
        .await;
    let provider = TestControlledFakeStreamingProvider::new(controls);
    assert!(provider.supports_tool_calls());
    let mut plan_session = provider
        .start(
            StreamingProviderInput {
                provider_type: ProviderType::Codex,
                role: AdapterRole::Reviewer,
                prompt: "plan_tests".to_string(),
                working_dir: std::env::current_dir().expect("current dir"),
                workspace_session_id: Some("coding_attempt_0001".to_string()),
                resume_provider_session_id: None,
                permission_mode: ProviderPermissionMode::Supervised,
                env_vars: Default::default(),
                timeout_secs: 60,
            },
            CancellationToken::new(),
        )
        .await
        .expect("plan fixture provider session");
    let plan_output = completed_output(&mut plan_session).await;
    assert!(plan_output.contains("controlled QA plan"));

    let mut execute_session = provider
        .start(
            StreamingProviderInput {
                provider_type: ProviderType::Codex,
                role: AdapterRole::Reviewer,
                prompt: "Phase: plan_tests -> execute_test_plan\nexecute_test_plan".to_string(),
                working_dir: std::env::current_dir().expect("current dir"),
                workspace_session_id: Some("coding_attempt_0001".to_string()),
                resume_provider_session_id: None,
                permission_mode: ProviderPermissionMode::Supervised,
                env_vars: Default::default(),
                timeout_secs: 60,
            },
            CancellationToken::new(),
        )
        .await
        .expect("execute fixture provider session");
    let execute_output = completed_output(&mut execute_session).await;
    assert!(execute_output.contains("\"step_id\":\"unit\""));
    assert!(execute_output.contains("\"status\":\"passed\""));
}

#[tokio::test]
async fn review_fixture_can_emit_alias_findings_and_malformed_json() {
    let controls = TestControls::default();
    controls
        .enable_review_fixture(
            "coding_attempt_0001".to_string(),
            ReviewFixture {
                verdict: "request_changes".to_string(),
                summary: "alias finding".to_string(),
                comments: String::new(),
                raw_json: Some(json!({
                    "verdict": "request_changes",
                    "summary": "alias finding",
                    "findings": [{
                        "file": "src/lib.rs",
                        "description": "missing validation",
                        "recommendation": "add validation"
                    }]
                })),
                raw_text: None,
                findings: Vec::new(),
            },
        )
        .await;
    let provider = TestControlledFakeStreamingProvider::new(controls.clone());
    let mut session = provider
        .start(
            StreamingProviderInput {
                provider_type: ProviderType::Codex,
                role: AdapterRole::Reviewer,
                prompt: "code review".to_string(),
                working_dir: std::env::current_dir().expect("current dir"),
                workspace_session_id: Some("coding_attempt_0001".to_string()),
                resume_provider_session_id: None,
                permission_mode: ProviderPermissionMode::Supervised,
                env_vars: Default::default(),
                timeout_secs: 60,
            },
            CancellationToken::new(),
        )
        .await
        .expect("review fixture provider session");
    let output = completed_output(&mut session).await;
    assert!(output.contains("\"file\":\"src/lib.rs\""));
    assert!(output.contains("\"description\":\"missing validation\""));
    assert!(output.contains("\"recommendation\":\"add validation\""));

    controls
        .enable_review_fixture(
            "coding_attempt_0001".to_string(),
            ReviewFixture {
                verdict: "blocked".to_string(),
                summary: "malformed".to_string(),
                comments: String::new(),
                raw_json: None,
                raw_text: Some("not json at all".to_string()),
                findings: Vec::new(),
            },
        )
        .await;
    let provider = TestControlledFakeStreamingProvider::new(controls);
    let mut malformed_session = provider
        .start(
            StreamingProviderInput {
                provider_type: ProviderType::Codex,
                role: AdapterRole::Reviewer,
                prompt: "code review".to_string(),
                working_dir: std::env::current_dir().expect("current dir"),
                workspace_session_id: Some("coding_attempt_0001".to_string()),
                resume_provider_session_id: None,
                permission_mode: ProviderPermissionMode::Supervised,
                env_vars: Default::default(),
                timeout_secs: 60,
            },
            CancellationToken::new(),
        )
        .await
        .expect("malformed review fixture provider session");
    assert_eq!(
        completed_output(&mut malformed_session).await,
        "not json at all"
    );
}

#[tokio::test]
async fn review_fixture_provider_consumes_queued_outputs_in_order() {
    let controls = TestControls::default();
    controls
        .enable_review_fixture(
            "coding_attempt_0001".to_string(),
            ReviewFixture {
                verdict: "no_issue".to_string(),
                summary: "analyst pass".to_string(),
                comments: String::new(),
                raw_json: Some(json!({
                    "verdict": "no_issue",
                    "summary": "analyst pass"
                })),
                raw_text: None,
                findings: Vec::new(),
            },
        )
        .await;
    controls
        .enable_review_fixture(
            "coding_attempt_0001".to_string(),
            ReviewFixture {
                verdict: "request_changes".to_string(),
                summary: "review alias".to_string(),
                comments: String::new(),
                raw_json: Some(json!({
                    "verdict": "request_changes",
                    "summary": "review alias",
                    "findings": [{
                        "file": "src/lib.rs",
                        "description": "missing validation",
                        "recommendation": "add validation"
                    }]
                })),
                raw_text: None,
                findings: Vec::new(),
            },
        )
        .await;

    let provider = TestControlledFakeStreamingProvider::new(controls);
    let mut analyst_session = provider
        .start(
            StreamingProviderInput {
                provider_type: ProviderType::Codex,
                role: AdapterRole::Reviewer,
                prompt: "analyst".to_string(),
                working_dir: std::env::current_dir().expect("current dir"),
                workspace_session_id: Some("coding_attempt_0001".to_string()),
                resume_provider_session_id: None,
                permission_mode: ProviderPermissionMode::Supervised,
                env_vars: Default::default(),
                timeout_secs: 60,
            },
            CancellationToken::new(),
        )
        .await
        .expect("analyst fixture provider session");
    assert!(
        completed_output(&mut analyst_session)
            .await
            .contains("\"verdict\":\"no_issue\"")
    );

    let mut review_session = provider
        .start(
            StreamingProviderInput {
                provider_type: ProviderType::Codex,
                role: AdapterRole::Reviewer,
                prompt: "code review".to_string(),
                working_dir: std::env::current_dir().expect("current dir"),
                workspace_session_id: Some("coding_attempt_0001".to_string()),
                resume_provider_session_id: None,
                permission_mode: ProviderPermissionMode::Supervised,
                env_vars: Default::default(),
                timeout_secs: 60,
            },
            CancellationToken::new(),
        )
        .await
        .expect("review fixture provider session");
    let review_output = completed_output(&mut review_session).await;
    assert!(review_output.contains("\"verdict\":\"request_changes\""));
    assert!(review_output.contains("\"file\":\"src/lib.rs\""));
}

async fn completed_output(
    session: &mut crate::cross_cutting::streaming_provider::ProviderSession,
) -> String {
    while let Some(event) = session.events.recv().await {
        match event {
            ProviderEvent::Completed { full_output, .. } => return full_output,
            ProviderEvent::TextDelta { .. } => {}
            other => panic!("unexpected provider event: {other:?}"),
        }
    }
    panic!("provider session ended without completion")
}

#[tokio::test]
async fn permission_fixture_fake_provider_emits_timeout_when_unanswered() {
    let controls = TestControls::default();
    controls
        .enable_permission_fixture("workspace_session_1".to_string())
        .await;
    controls
        .set_permission_timeout(Duration::from_millis(20))
        .await;
    let provider = TestControlledFakeStreamingProvider::new(controls);
    let mut session = provider
        .start(
            streaming_input("workspace_session_1"),
            CancellationToken::new(),
        )
        .await
        .expect("provider session");

    match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
        .await
        .expect("stream event")
        .expect("text delta")
    {
        ProviderEvent::TextDelta { content } => {
            assert!(content.contains("E2E permission fixture stream"));
        }
        other => panic!("unexpected provider event: {other:?}"),
    }

    let request_id = match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
        .await
        .expect("permission event")
        .expect("permission request")
    {
        ProviderEvent::PermissionRequest(request) => request.id,
        other => panic!("unexpected provider event: {other:?}"),
    };

    match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
        .await
        .expect("timeout event")
        .expect("permission timeout")
    {
        ProviderEvent::PermissionTimeout { permission_id } => {
            assert_eq!(permission_id, request_id)
        }
        other => panic!("unexpected provider event: {other:?}"),
    }
}

fn streaming_input(session_id: &str) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: ProviderType::Fake,
        role: AdapterRole::Orchestrator,
        prompt: "生成测试产物".to_string(),
        working_dir: std::env::current_dir().expect("current dir"),
        workspace_session_id: Some(session_id.to_string()),
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Supervised,
        env_vars: Default::default(),
        timeout_secs: 60,
    }
}
