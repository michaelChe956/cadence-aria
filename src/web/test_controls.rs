use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    FakeStreamingProvider, PermissionRequestData, ProviderCommand, ProviderEvent, ProviderSession,
    RiskLevel, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole};
use crate::web::state::WebAppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceSocketControl {
    CloseForTestDrop,
}

#[derive(Clone, Default)]
pub struct TestControls {
    inner: Arc<TestControlsInner>,
}

#[derive(Default)]
struct TestControlsInner {
    workspace_sockets: Mutex<HashMap<String, Vec<mpsc::Sender<WorkspaceSocketControl>>>>,
    workspace_socket_rejects: Mutex<HashMap<String, u32>>,
    permission_fixture_sessions: Mutex<HashSet<String>>,
    review_fixture_sessions: Mutex<HashMap<String, ReviewFixture>>,
    permission_timeout: Mutex<Option<Duration>>,
    server_idle_timeout: Mutex<Option<Duration>>,
}

pub fn test_controls_enabled() -> bool {
    std::env::var("ARIA_E2E_TEST_CONTROLS").as_deref() == Ok("1")
}

#[derive(Debug, Deserialize)]
pub struct PermissionFixtureRequest {
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewFixture {
    pub verdict: String,
    pub summary: String,
    pub comments: String,
}

#[derive(Debug, Deserialize)]
pub struct PermissionTimeoutRequest {
    pub timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct WsTimeoutRequest {
    pub server_idle_timeout_ms: Option<u64>,
    pub client_idle_timeout_ms: Option<u64>,
    pub suppress_server_messages: Option<bool>,
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsRejectRequest {
    pub count: u32,
}

pub async fn drop_workspace_socket(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
) -> Json<serde_json::Value> {
    let dropped = state
        .test_controls
        .drop_workspace_socket_when_registered(&session_id, Duration::from_secs(2))
        .await;
    Json(json!({"status": "ok", "dropped": dropped}))
}

pub async fn reject_next_workspace_sockets(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WsRejectRequest>,
) -> Json<serde_json::Value> {
    state
        .test_controls
        .reject_next_workspace_sockets(session_id, request.count)
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn enable_permission_fixture(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<PermissionFixtureRequest>,
) -> Json<serde_json::Value> {
    let _mode = request.mode.as_deref().unwrap_or("single-request");
    state
        .test_controls
        .enable_permission_fixture(session_id)
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn enable_review_fixture(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<ReviewFixture>,
) -> Json<serde_json::Value> {
    state
        .test_controls
        .enable_review_fixture(session_id, request)
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn set_permission_timeout(
    State(state): State<WebAppState>,
    Json(request): Json<PermissionTimeoutRequest>,
) -> Json<serde_json::Value> {
    state
        .test_controls
        .set_permission_timeout(Duration::from_millis(request.timeout_ms))
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn set_ws_timeout(
    State(state): State<WebAppState>,
    Json(request): Json<WsTimeoutRequest>,
) -> Json<serde_json::Value> {
    if let Some(timeout_ms) = request.server_idle_timeout_ms {
        state
            .test_controls
            .set_server_idle_timeout(Duration::from_millis(timeout_ms))
            .await;
    }
    let _ = (
        request.client_idle_timeout_ms,
        request.suppress_server_messages,
        request.session_id,
    );
    Json(json!({"status": "ok"}))
}

impl TestControls {
    pub async fn register_workspace_socket(
        &self,
        session_id: String,
        sender: mpsc::Sender<WorkspaceSocketControl>,
    ) {
        self.inner
            .workspace_sockets
            .lock()
            .expect("test controls workspace socket lock")
            .entry(session_id)
            .or_default()
            .push(sender);
    }

    pub async fn reject_next_workspace_sockets(&self, session_id: String, count: u32) {
        if count == 0 {
            self.inner
                .workspace_socket_rejects
                .lock()
                .expect("test controls workspace socket rejects lock")
                .remove(&session_id);
            return;
        }
        self.inner
            .workspace_socket_rejects
            .lock()
            .expect("test controls workspace socket rejects lock")
            .insert(session_id, count);
    }

    pub async fn consume_workspace_socket_reject(&self, session_id: &str) -> bool {
        let mut rejects = self
            .inner
            .workspace_socket_rejects
            .lock()
            .expect("test controls workspace socket rejects lock");
        let Some(count) = rejects.get_mut(session_id) else {
            return false;
        };
        if *count <= 1 {
            rejects.remove(session_id);
        } else {
            *count -= 1;
        }
        true
    }

    pub async fn drop_workspace_socket(&self, session_id: &str) -> bool {
        let senders = self
            .inner
            .workspace_sockets
            .lock()
            .expect("test controls workspace socket lock")
            .remove(session_id)
            .unwrap_or_default();

        let mut dropped = false;
        for sender in senders {
            if sender
                .send(WorkspaceSocketControl::CloseForTestDrop)
                .await
                .is_ok()
            {
                dropped = true;
            }
        }
        dropped
    }

    pub async fn drop_workspace_socket_when_registered(
        &self,
        session_id: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.drop_workspace_socket(session_id).await {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    pub async fn enable_permission_fixture(&self, session_id: String) {
        self.inner
            .permission_fixture_sessions
            .lock()
            .expect("test controls permission fixture lock")
            .insert(session_id);
    }

    pub async fn enable_review_fixture(&self, session_id: String, fixture: ReviewFixture) {
        self.inner
            .review_fixture_sessions
            .lock()
            .expect("test controls review fixture lock")
            .insert(session_id, fixture);
    }

    pub async fn consume_permission_fixture(&self, session_id: &str) -> bool {
        self.inner
            .permission_fixture_sessions
            .lock()
            .expect("test controls permission fixture lock")
            .remove(session_id)
    }

    pub async fn consume_review_fixture(&self, session_id: &str) -> Option<ReviewFixture> {
        self.inner
            .review_fixture_sessions
            .lock()
            .expect("test controls review fixture lock")
            .remove(session_id)
    }

    pub fn permission_timeout(&self) -> Duration {
        self.inner
            .permission_timeout
            .lock()
            .expect("test controls permission timeout lock")
            .unwrap_or(Duration::from_secs(900))
    }

    pub async fn set_permission_timeout(&self, timeout: Duration) {
        *self
            .inner
            .permission_timeout
            .lock()
            .expect("test controls permission timeout lock") = Some(timeout);
    }

    pub fn server_idle_timeout(&self) -> Duration {
        self.inner
            .server_idle_timeout
            .lock()
            .expect("test controls server idle timeout lock")
            .unwrap_or(Duration::from_secs(90))
    }

    pub async fn set_server_idle_timeout(&self, timeout: Duration) {
        *self
            .inner
            .server_idle_timeout
            .lock()
            .expect("test controls server idle timeout lock") = Some(timeout);
    }
}

pub struct TestControlledFakeStreamingProvider {
    controls: TestControls,
    fallback: FakeStreamingProvider,
}

impl TestControlledFakeStreamingProvider {
    pub fn new(controls: TestControls) -> Self {
        Self {
            controls,
            fallback: FakeStreamingProvider,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for TestControlledFakeStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        if input.role == AdapterRole::Reviewer
            && let Some(session_id) = input.session_id.as_deref()
            && let Some(fixture) = self.controls.consume_review_fixture(session_id).await
        {
            return Ok(start_review_fixture_session(fixture, cancel));
        }

        let use_fixture = match input.session_id.as_deref() {
            Some(session_id) => self.controls.consume_permission_fixture(session_id).await,
            None => false,
        };

        if !use_fixture {
            return self.fallback.start(input, cancel).await;
        }

        Ok(start_permission_fixture_session(
            self.controls.permission_timeout(),
            cancel,
        ))
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.fallback.run_streaming(input, cancel).await
    }
}

fn start_review_fixture_session(
    fixture: ReviewFixture,
    cancel: tokio_util::sync::CancellationToken,
) -> ProviderSession {
    let (event_tx, event_rx) = mpsc::channel(8);
    let (command_tx, _command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let contract = json!({
            "verdict": fixture.verdict,
            "summary": fixture.summary,
        });
        let output = format!("{}\n\n```json\n{}\n```", fixture.comments, contract);
        if cancel.is_cancelled() {
            return;
        }
        if event_tx
            .send(ProviderEvent::TextDelta {
                content: output.clone(),
            })
            .await
            .is_err()
        {
            return;
        }
        if cancel.is_cancelled() {
            return;
        }
        let _ = event_tx
            .send(ProviderEvent::Completed {
                full_output: output,
                provider_session_id: None,
            })
            .await;
    });

    ProviderSession {
        events: event_rx,
        commands: command_tx,
    }
}

fn start_permission_fixture_session(
    timeout_after: Duration,
    cancel: tokio_util::sync::CancellationToken,
) -> ProviderSession {
    let (event_tx, event_rx) = mpsc::channel(16);
    let (command_tx, mut command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let permission_id = "e2e_permission_1".to_string();
        if event_tx
            .send(ProviderEvent::TextDelta {
                content: "E2E permission fixture stream\n".to_string(),
            })
            .await
            .is_err()
        {
            return;
        }

        if event_tx
            .send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: permission_id.clone(),
                tool_name: "Bash".to_string(),
                description: "E2E permission fixture request".to_string(),
                risk_level: RiskLevel::Medium,
            }))
            .await
            .is_err()
        {
            return;
        }

        let timeout = tokio::time::sleep(timeout_after);
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = &mut timeout => {
                    let _ = event_tx
                        .send(ProviderEvent::PermissionTimeout {
                            permission_id: permission_id.clone(),
                        })
                        .await;
                    return;
                }
                command = command_rx.recv() => {
                    match command {
                        Some(ProviderCommand::PermissionResponse { id, approved, .. }) if id == permission_id => {
                            if approved {
                                let output = "# Permission Fixture\n\nApproved request completed.\n".to_string();
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
                            } else {
                                let _ = event_tx
                                    .send(ProviderEvent::Failed {
                                        message: "permission denied".to_string(),
                                    })
                                    .await;
                            }
                            return;
                        }
                        Some(ProviderCommand::PermissionResponse { id, .. }) => {
                            let _ = event_tx
                                .send(ProviderEvent::ProtocolError {
                                    code: "PERMISSION_ID_UNMATCHED".to_string(),
                                    message: format!("PermissionResponse id={id} not found in pending"),
                                    context: Some(json!({ "permission_id": id })),
                                })
                                .await;
                        }
                        Some(ProviderCommand::ChoiceResponse { .. }) => {}
                        Some(ProviderCommand::Abort) | None => return,
                    }
                }
            }
        }
    });

    ProviderSession {
        events: event_rx,
        commands: command_tx,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderAdapter,
        StreamingProviderInput,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    use super::{
        ReviewFixture, TestControlledFakeStreamingProvider, TestControls, WorkspaceSocketControl,
        test_controls_enabled,
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
            .drop_workspace_socket_when_registered(
                "workspace_session_1",
                Duration::from_millis(200),
            )
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
                    session_id: Some("workspace_session_1".to_string()),
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
            session_id: Some(session_id.to_string()),
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: Default::default(),
            timeout_secs: 60,
        }
    }
}
