use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    FakeStreamingProvider, PermissionRequestData, ProviderCommand, ProviderCompletion,
    ProviderEvent, ProviderSession, RiskLevel, StreamChunk, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::cross_cutting::structured_output::StructuredOutputContract;
use crate::protocol::contracts::{AdapterInput, AdapterRole};
use crate::web::state::WebAppState;

use super::{ReviewFixture, TestControls};

#[derive(Debug, Deserialize)]
pub struct PermissionFixtureRequest {
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PermissionTimeoutRequest {
    pub timeout_ms: u64,
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

impl TestControls {
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
            .entry(session_id)
            .or_default()
            .push_back(fixture);
    }

    pub async fn consume_permission_fixture(&self, session_id: &str) -> bool {
        self.inner
            .permission_fixture_sessions
            .lock()
            .expect("test controls permission fixture lock")
            .remove(session_id)
    }

    pub async fn consume_review_fixture(&self, session_id: &str) -> Option<ReviewFixture> {
        let mut fixtures = self
            .inner
            .review_fixture_sessions
            .lock()
            .expect("test controls review fixture lock");
        let queue = fixtures.get_mut(session_id)?;
        let fixture = queue.pop_front();
        if queue.is_empty() {
            fixtures.remove(session_id);
        }
        fixture
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
    fn supports_tool_calls(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let structured_output_contract = input.structured_output_contract.clone();
        if input.role == AdapterRole::Reviewer
            && let Some(session_id) = input.workspace_session_id.as_deref()
            && let Some(fixture) = self.controls.consume_review_fixture(session_id).await
        {
            return Ok(start_review_fixture_session(
                fixture,
                structured_output_contract,
                cancel,
            ));
        }

        let use_fixture = match input.workspace_session_id.as_deref() {
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
    structured_output_contract: Option<StructuredOutputContract>,
    cancel: tokio_util::sync::CancellationToken,
) -> ProviderSession {
    let (event_tx, event_rx) = mpsc::channel(8);
    let (command_tx, _command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let preserve_raw_completion = fixture.raw_text.is_some();
        let mut output = if let Some(raw_text) = fixture.raw_text {
            raw_text
        } else if let Some(raw_json) = fixture.raw_json {
            raw_json.to_string()
        } else {
            let contract = json!({
                "verdict": fixture.verdict,
                "summary": fixture.summary,
                "findings": fixture.findings,
            });
            format!("{}\n\n```json\n{}\n```", fixture.comments, contract)
        };
        if preserve_raw_completion && let Some(contract) = structured_output_contract.as_ref() {
            output = output.replace("__NONCE__", &contract.nonce);
        }
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
            .send(ProviderEvent::Completed(review_fixture_completion(
                output,
                structured_output_contract.as_ref(),
                preserve_raw_completion,
            )))
            .await;
    });

    ProviderSession {
        events: event_rx,
        commands: command_tx,
    }
}

fn review_fixture_completion(
    output: String,
    contract: Option<&StructuredOutputContract>,
    preserve_raw_completion: bool,
) -> ProviderCompletion {
    let provider_session_id = Some("test-review-session".to_string());
    if preserve_raw_completion {
        return ProviderCompletion::from_output(output, contract, provider_session_id);
    }
    let Some(contract) = contract else {
        return ProviderCompletion::plain(output, provider_session_id);
    };
    let (comments, json) = review_fixture_output_parts(&output);
    let mut json: serde_json::Value =
        serde_json::from_str(json).expect("review fixture JSON must be an object");
    json.as_object_mut()
        .expect("review fixture JSON must be an object")
        .insert("nonce".to_string(), serde_json::json!(contract.nonce));
    let full_output = format!(
        "{comments}\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">{json}</ARIA_STRUCTURED_OUTPUT>",
        contract.nonce
    );
    ProviderCompletion::from_output(full_output, Some(contract), provider_session_id)
}

fn review_fixture_output_parts(output: &str) -> (&str, &str) {
    let trimmed = output.trim();
    if trimmed.starts_with('{') {
        return ("", trimmed);
    }
    let Some(end) = output.rfind("```") else {
        return ("", trimmed);
    };
    let before_end = &output[..end];
    let Some(start) = before_end.rfind("```") else {
        return ("", trimmed);
    };
    let json = before_end[start + 3..]
        .trim()
        .strip_prefix("json")
        .unwrap_or(&before_end[start + 3..])
        .trim();
    (output[..start].trim_end(), json)
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
                                    .send(ProviderEvent::Completed(ProviderCompletion::plain(
                                        output, None,
                                    )))
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
                        Some(ProviderCommand::ToolResult(_)) => {}
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
