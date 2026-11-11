//! handlers/image_create.rs 的外置测试模块：经 `#[path = "image_create_tests.rs"]` 引入，
//! `use super::*` 语义与原内嵌 `mod tests` 完全一致（纯移动，逐字外迁）。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::{FromRequest, Multipart};
use axum::http::{Request, StatusCode, header};
use futures_util::{SinkExt, StreamExt};
use image::ImageEncoder;
use image::codecs::png::PngEncoder;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc};
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use super::{HandlerResult, ImageCreateApiError, parse_generate_multipart};
use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::image_client::{
    ImageCallTarget, ImageClient, ImageClientApi, ImageClientError, ImageGenOutcome,
    ImageGenRequest, ImageRefImage,
};
use crate::cross_cutting::image_reference_validation::MAX_BYTES;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderCompletion, ProviderEvent, ProviderSession, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::product::image_create::engine::GenerateCommand;
use crate::product::image_create::models::{
    ImageCreateError, ImageCreateSettings, ImageVendor, ModelChoice, SessionStoreApi,
    SettingsStoreApi,
};
use crate::product::image_create::{
    ImageCreateEngine, ImageCreateRunRegistry, SessionStore, SettingsStore,
};
use crate::product::models::ProviderName;
use crate::web::app::build_web_router;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;

struct FakeImageClient {
    expected_reference: Option<Vec<u8>>,
}

#[async_trait]
impl ImageClientApi for FakeImageClient {
    async fn generate(
        &self,
        _target: &ImageCallTarget,
        request: &ImageGenRequest,
        reference: Option<ImageRefImage>,
    ) -> Result<ImageGenOutcome, ImageClientError> {
        assert_eq!(request.prompt, "draw a cat");
        match (&self.expected_reference, reference) {
            (Some(expected), Some(reference)) => {
                assert_eq!(&reference.bytes, expected);
                assert_eq!(reference.declared_mime, "image/png");
            }
            (None, None) => {}
            (expected, actual) => panic!(
                "unexpected reference: expected={}, actual={}",
                expected.is_some(),
                actual.is_some()
            ),
        }
        Ok(ImageGenOutcome {
            media_type: "image/png".to_string(),
            b64: "ZmFrZS1pbWFnZQ==".to_string(),
        })
    }
}

struct BlockingIterationProvider {
    starts: Arc<AtomicUsize>,
    started: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl StreamingProviderAdapter for BlockingIterationProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel::<ProviderCommand>(1);
        let release = self.release.clone();
        tokio::spawn(async move {
            release.notified().await;
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    "iteration complete",
                    None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn state(root: &std::path::Path) -> WebAppState {
    WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
}

async fn response_json<T: DeserializeOwned>(response: axum::response::Response) -> T {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("response json")
}

fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

fn valid_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(&[0x7f], 1, 1, image::ExtendedColorType::L8)
        .expect("encode png");
    bytes
}

fn valid_png_larger_than_two_mib() -> Vec<u8> {
    const WIDTH: u32 = 2048;
    const HEIGHT: u32 = 1100;
    let mut state = 0x1234_5678_u32;
    let pixels: Vec<u8> = (0..WIDTH * HEIGHT)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(&pixels, WIDTH, HEIGHT, image::ExtendedColorType::L8)
        .expect("encode large png");
    assert!(bytes.len() > 2 * 1024 * 1024);
    assert!(bytes.len() < MAX_BYTES);
    bytes
}

/// 构造 multipart 请求体：文本字段在前，参考图在后。
fn multipart_body(boundary: &str, text_fields: &[(&str, &str)], references: &[&[u8]]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in text_fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    for reference in references {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"reference\"; filename=\"reference.png\"\r\nContent-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(reference);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// 既有用例的请求体：只带 prompt 与参考图。
fn generate_multipart(boundary: &str, references: &[&[u8]]) -> Vec<u8> {
    multipart_body(boundary, &[("prompt", "draw a cat")], references)
}

/// 直接调用 handler 的 multipart 解析（不经 HTTP 层），便于断言命令内容。
async fn parse_multipart(body: Vec<u8>, boundary: &str) -> HandlerResult<GenerateCommand> {
    let request = Request::builder()
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .expect("multipart request");
    let multipart = Multipart::from_request(request, &())
        .await
        .expect("multipart extractor");
    parse_generate_multipart(multipart).await
}

async fn configured_generate_state(
    root: &std::path::Path,
    expected_reference: Option<Vec<u8>>,
) -> (WebAppState, String) {
    let paths = AriaStatePaths::from_workspace_root(root);
    let store = SessionStore::new(paths.clone());
    let session = store
        .create(
            serde_json::from_value(json!({
                "template": {"preset": "ppt_business_illustration", "custom": null},
                "provider_name": "fake"
            }))
            .expect("create request"),
        )
        .await
        .expect("session");
    // 只配置 GPT 一家：自动轮转（auto）此时选中 GPT，MiniMax 无凭据不参与轮转。
    let mut settings = ImageCreateSettings::default();
    let gpt = settings.vendors.get_mut(ImageVendor::Gpt);
    gpt.base_url = "https://images.example.com/v1".to_string();
    gpt.api_key = "sk-test".to_string();
    SettingsStore::new(paths.clone())
        .save(&settings)
        .await
        .expect("settings");
    let mut state = state(root);
    state.image_create_engine = Some(Arc::new(ImageCreateEngine::new(
        paths.clone(),
        Arc::new(SessionStore::new(paths.clone())),
        Arc::new(SettingsStore::new(paths)),
        Arc::new(FakeImageClient { expected_reference }),
        Arc::new(ProviderRegistry::new()),
        Arc::new(ImageCreateRunRegistry::default()),
    )));
    (state, session.id)
}

#[tokio::test]
async fn rest_list_create_get_delete_and_invalid_id() {
    let root = tempdir().expect("root");
    let app = build_web_router(state(root.path()));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/image-create/sessions")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json::<Value>(response).await, json!([]));

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/image-create/sessions",
            json!({
                "template": {"preset": "ppt_business_illustration", "custom": null},
                "provider_name": "fake"
            }),
        ))
        .await
        .expect("create response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let created: Value = response_json(response).await;
    let id = created["id"].as_str().expect("session id");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/image-create/sessions")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list response");
    let listed: Value = response_json(response).await;
    assert_eq!(listed.as_array().expect("list").len(), 1);
    assert_eq!(listed[0]["id"], id);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/image-create/sessions/{id}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("get response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/image-create/sessions/{id}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("delete response");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/image-create/sessions/{id}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("get response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/image-create/sessions/..")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("invalid response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn settings_are_masked_persisted_and_validate_base_url() {
    let root = tempdir().expect("root");
    let app = build_web_router(state(root.path()));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/image-create/settings")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("settings response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/image-create/settings",
            json!({
                "vendors": {
                    "gpt": {
                        "base_url": "http://example.com/v1",
                        "api_key_action": "replace",
                        "api_key": "sk-secret1234"
                    }
                },
                "defaults": null
            }),
        ))
        .await
        .expect("invalid update response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/image-create/settings",
            json!({
                "vendors": {
                    "gpt": {
                        "base_url": "https://images.example.com/v1",
                        "api_key_action": "replace",
                        "api_key": "sk-secret1234"
                    }
                },
                "defaults": null
            }),
        ))
        .await
        .expect("update response");
    assert_eq!(response.status(), StatusCode::OK);
    let masked: Value = response_json(response).await;
    assert_eq!(masked["vendors"]["gpt"]["api_key_masked"], "sk-****1234");
    assert!(!masked.to_string().contains("secret"));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/image-create/settings")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("settings response");
    let masked: Value = response_json(response).await;
    assert_eq!(
        masked["vendors"]["gpt"]["base_url"],
        "https://images.example.com/v1"
    );
    assert_eq!(masked["vendors"]["gpt"]["api_key_masked"], "sk-****1234");
}

#[tokio::test]
async fn generate_parses_multipart_and_maps_missing_config() {
    let root = tempdir().expect("root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let store = SessionStore::new(paths.clone());
    let session = store
        .create(
            serde_json::from_value(json!({
                "template": {"preset": "ppt_business_illustration", "custom": null},
                "provider_name": "fake"
            }))
            .expect("create request"),
        )
        .await
        .expect("session");
    let boundary = "image-create-test-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\ndraw a cat\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"size\"\r\n\r\n1024x1024\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"quality\"\r\n\r\nhigh\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"background\"\r\n\r\nopaque\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"output_format\"\r\n\r\npng\r\n--{boundary}--\r\n"
    );

    let response = build_web_router(state(root.path()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/image-create/sessions/{}/generate",
                    session.id
                ))
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body.clone()))
                .expect("request"),
        )
        .await
        .expect("missing config response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // 只配置 GPT 一家：自动轮转（auto）此时选中 GPT，MiniMax 无凭据不参与轮转。
    let mut saved = ImageCreateSettings::default();
    let gpt = saved.vendors.get_mut(ImageVendor::Gpt);
    gpt.base_url = "https://images.example.com/v1".to_string();
    gpt.api_key = "sk-test".to_string();
    let settings = SettingsStore::new(paths.clone());
    settings.save(&saved).await.expect("settings");
    let mut fake_state = state(root.path());
    fake_state.image_create_engine = Some(Arc::new(ImageCreateEngine::new(
        paths.clone(),
        Arc::new(SessionStore::new(paths.clone())),
        Arc::new(SettingsStore::new(paths)),
        Arc::new(FakeImageClient {
            expected_reference: None,
        }),
        Arc::new(ProviderRegistry::new()),
        Arc::new(ImageCreateRunRegistry::default()),
    )));

    let response = build_web_router(fake_state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/image-create/sessions/{}/generate",
                    session.id
                ))
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);
    let result = response_json::<Value>(response).await;
    assert!(result["image_id"].as_str().is_some());
    assert_eq!(result["media_type"], "image/png");
    assert!(result.get("b64").is_none());
}

#[tokio::test]
async fn generate_accepts_reference_above_two_mib_and_rejects_above_ten_mib() {
    let root = tempdir().expect("root");
    let large_reference = valid_png_larger_than_two_mib();
    let (state, session_id) =
        configured_generate_state(root.path(), Some(large_reference.clone())).await;
    let boundary = "image-create-large-valid-reference";
    let body = generate_multipart(boundary, &[&large_reference]);

    let response = build_web_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/image-create/sessions/{session_id}/generate"))
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);

    let root = tempdir().expect("root");
    let (state, session_id) = configured_generate_state(root.path(), None).await;
    let mut oversized = valid_png();
    oversized.resize(MAX_BYTES + 1, 0);
    let boundary = "image-create-over-ten-mib-reference";
    let body = generate_multipart(boundary, &[&oversized]);
    let response = build_web_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/image-create/sessions/{session_id}/generate"))
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("oversized response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response_json::<Value>(response).await["error"]
            .as_str()
            .expect("error")
            .contains("10 MiB")
    );
}

#[tokio::test]
async fn echoed_api_key_is_redacted_from_rest_event_and_persisted_session() {
    let root = tempdir().expect("root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let store = SessionStore::new(paths.clone());
    let session = store
        .create(
            serde_json::from_value(json!({
                "template": {"preset": "ppt_business_illustration", "custom": null},
                "provider_name": "fake"
            }))
            .expect("create request"),
        )
        .await
        .expect("session");
    let secret = "sk-secret123";
    let mut gateway = mockito::Server::new_async().await;
    let gateway_error = gateway
        .mock("POST", "/v1/images/generations")
        .with_status(500)
        .with_body(format!(
            "gateway echoed Authorization: Bearer {secret}; token={secret}"
        ))
        .expect(1)
        .create_async()
        .await;
    // 把网关地址与密钥写入 GPT 家（该家凭据齐全，自动轮转即选中 GPT）。
    let mut settings = ImageCreateSettings::default();
    let gpt = settings.vendors.get_mut(ImageVendor::Gpt);
    gpt.base_url = gateway.url();
    gpt.api_key = secret.to_string();
    SettingsStore::new(paths.clone())
        .save(&settings)
        .await
        .expect("settings");
    let mut state = state(root.path());
    state.image_create_engine = Some(Arc::new(ImageCreateEngine::new(
        paths.clone(),
        Arc::new(SessionStore::new(paths.clone())),
        Arc::new(SettingsStore::new(paths.clone())),
        Arc::new(ImageClient::new()),
        Arc::new(ProviderRegistry::new()),
        Arc::new(ImageCreateRunRegistry::default()),
    )));
    let boundary = "image-create-secret-echo";
    let body = generate_multipart(boundary, &[]);

    let response = build_web_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/image-create/sessions/{}/generate",
                    session.id
                ))
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let rest_body = response_json::<Value>(response).await.to_string();
    assert!(!rest_body.contains(secret));
    assert!(rest_body.contains("REDACTED"));

    let record = store
        .get(&session.id)
        .await
        .expect("session lookup")
        .expect("session record");
    assert_eq!(record.events.len(), 1);
    assert!(!record.events[0].message.contains(secret));
    assert!(record.events[0].message.contains("REDACTED"));
    let persisted = tokio::fs::read_to_string(paths.image_create_session_file(&session.id))
        .await
        .expect("persisted session");
    assert!(!persisted.contains(secret));
    assert!(persisted.contains("REDACTED"));
    gateway_error.assert_async().await;
}

#[tokio::test]
async fn generate_rejects_multiple_reference_fields() {
    let root = tempdir().expect("root");
    let reference = valid_png();
    let (state, session_id) = configured_generate_state(root.path(), Some(reference.clone())).await;
    let boundary = "image-create-multiple-reference";
    let body = generate_multipart(boundary, &[&reference, &reference]);

    let response = build_web_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/image-create/sessions/{session_id}/generate"))
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("generate response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json::<Value>(response).await["error"],
        "invalid parameter: 仅支持单张参考图"
    );
}

#[tokio::test]
async fn generate_passes_single_valid_reference_to_engine() {
    let root = tempdir().expect("root");
    let reference = valid_png();
    let (state, session_id) = configured_generate_state(root.path(), Some(reference.clone())).await;
    let boundary = "image-create-single-reference";
    let body = generate_multipart(boundary, &[&reference]);

    let response = build_web_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/image-create/sessions/{session_id}/generate"))
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("generate response");

    assert_eq!(response.status(), StatusCode::OK);
    let result = response_json::<Value>(response).await;
    assert!(result["image_id"].as_str().is_some());
    assert_eq!(result["media_type"], "image/png");
    assert!(result.get("b64").is_none());
}

#[tokio::test]
async fn websocket_rejects_message_immediately_while_iteration_is_active() {
    let root = tempdir().expect("root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let store = SessionStore::new(paths.clone());
    let session = store
        .create(
            serde_json::from_value(json!({
                "template": {"preset": "ppt_business_illustration", "custom": null},
                "provider_name": "fake"
            }))
            .expect("create request"),
        )
        .await
        .expect("session");
    let starts = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(BlockingIterationProvider {
            starts: starts.clone(),
            started: started.clone(),
            release: release.clone(),
        }),
    );
    let mut state = state(root.path());
    state.image_create_engine = Some(Arc::new(ImageCreateEngine::new(
        paths.clone(),
        Arc::new(SessionStore::new(paths.clone())),
        Arc::new(SettingsStore::new(paths)),
        Arc::new(FakeImageClient {
            expected_reference: None,
        }),
        Arc::new(registry),
        Arc::new(ImageCreateRunRegistry::default()),
    )));
    let app = build_web_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let (mut socket, _) = connect_async(format!(
        "ws://{address}/api/image-create/sessions/{}/chat",
        session.id
    ))
    .await
    .expect("connect websocket");

    socket
        .send(TungsteniteMessage::Text("first request".into()))
        .await
        .expect("send first request");
    timeout(Duration::from_secs(3), started.notified())
        .await
        .expect("first iteration started");
    socket
        .send(TungsteniteMessage::Text("second request".into()))
        .await
        .expect("send second request");

    let busy = timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await {
                Some(Ok(TungsteniteMessage::Text(text))) => {
                    let event: Value = serde_json::from_str(&text).expect("event json");
                    if event["kind"] == "error" {
                        break event;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => panic!("websocket receive failed: {error}"),
                None => panic!("websocket closed before busy event"),
            }
        }
    })
    .await
    .expect("busy event must arrive while first iteration is blocked");
    assert_eq!(busy["error"], "会话忙，请等待当前任务完成");
    assert_eq!(starts.load(Ordering::SeqCst), 1);

    release.notify_one();
    timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await {
                Some(Ok(TungsteniteMessage::Text(text))) => {
                    let event: Value = serde_json::from_str(&text).expect("event json");
                    if event["kind"] == "done" {
                        break;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => panic!("websocket receive failed: {error}"),
                None => panic!("websocket closed before done event"),
            }
        }
    })
    .await
    .expect("first iteration completes");
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let record = store
        .get(&session.id)
        .await
        .expect("session lookup")
        .expect("session record");
    assert_eq!(record.messages.len(), 1);
    assert_eq!(record.messages[0].content, "first request");

    socket.close(None).await.expect("close websocket");
    server.abort();
}

/// H-01：multipart 不带 model / use_previous → 命令为 Auto + false。
#[tokio::test]
async fn generate_command_defaults_to_auto_model_and_no_previous() {
    let boundary = "image-create-h01-defaults";
    let body = multipart_body(boundary, &[("prompt", "draw a cat")], &[]);

    let command = parse_multipart(body, boundary).await.expect("command");

    assert_eq!(command.model, ModelChoice::Auto);
    assert!(!command.use_previous);
    assert_eq!(command.request.prompt, "draw a cat");
    assert!(command.reference.is_none());
}

/// H-02：model=gpt-image-2.6-sunburst-2k → Explicit 且字符串原样；
/// 缺省 / 空白 / "auto"（不区分大小写）→ Auto；其余原样（仅去首尾空白）。
#[tokio::test]
async fn generate_command_parses_model_choice() {
    for (raw, expected) in [
        ("", ModelChoice::Auto),
        (" ", ModelChoice::Auto),
        ("auto", ModelChoice::Auto),
        ("AUTO", ModelChoice::Auto),
        (
            "gpt-image-2.6-sunburst-2k",
            ModelChoice::Explicit("gpt-image-2.6-sunburst-2k".to_string()),
        ),
        (
            " gpt-image-2.6-flare ",
            ModelChoice::Explicit("gpt-image-2.6-flare".to_string()),
        ),
    ] {
        let boundary = "image-create-model-choice";
        let body = multipart_body(boundary, &[("prompt", "draw a cat"), ("model", raw)], &[]);

        let command = parse_multipart(body, boundary).await.expect("command");

        assert_eq!(command.model, expected, "model={raw:?}");
    }
}

/// use_previous 只接受 true / false；空白按 false 处理。
/// H-03：use_previous=yes → 参数错误「use_previous 仅支持 true 或 false」。
#[tokio::test]
async fn generate_command_parses_use_previous_flag() {
    for (raw, expected) in [("true", true), ("false", false), ("", false)] {
        let boundary = "image-create-use-previous-flag";
        let body = multipart_body(
            boundary,
            &[("prompt", "draw a cat"), ("use_previous", raw)],
            &[],
        );

        let command = parse_multipart(body, boundary).await.expect("command");

        assert_eq!(command.use_previous, expected, "use_previous={raw:?}");
    }

    let boundary = "image-create-h03-invalid-use-previous";
    let body = multipart_body(
        boundary,
        &[("prompt", "draw a cat"), ("use_previous", "yes")],
        &[],
    );
    let Err(error) = parse_multipart(body, boundary).await else {
        panic!("use_previous=yes 必须被拒绝");
    };
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(
        error.error.contains("use_previous 仅支持 true 或 false"),
        "错误文本：{}",
        error.error
    );
}

/// H-09：仍带 input_fidelity 的请求被接受且该字段被忽略。
/// 取值刻意用旧的非法值（旧的枚举只有 low/high）——改造前会 400，改造后必须被忽略。
#[tokio::test]
async fn generate_command_ignores_removed_input_fidelity_field() {
    let boundary = "image-create-h09-input-fidelity";
    let body = multipart_body(
        boundary,
        &[
            ("prompt", "draw a cat"),
            ("input_fidelity", "ultra"),
            ("model", "auto"),
        ],
        &[],
    );

    let command = parse_multipart(body, boundary).await.expect("command");

    assert_eq!(command.request.prompt, "draw a cat");
    assert_eq!(command.model, ModelChoice::Auto);
    assert!(!command.use_previous);
}

/// H-04：生成成功响应含 vendor / model / selection / reference_source 四个字段。
#[tokio::test]
async fn generate_response_reports_vendor_model_selection_and_reference_source() {
    let root = tempdir().expect("root");
    let (state, session_id) = configured_generate_state(root.path(), None).await;
    let boundary = "image-create-h04-response-fields";
    let body = multipart_body(boundary, &[("prompt", "draw a cat")], &[]);

    let response = build_web_router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/image-create/sessions/{session_id}/generate"))
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("generate response");

    assert_eq!(response.status(), StatusCode::OK);
    let result = response_json::<Value>(response).await;
    let fields = ["vendor", "model", "selection", "reference_source"];
    for field in fields {
        assert!(
            result.get(field).is_some(),
            "响应缺少字段 {field}：{result}"
        );
    }
    // 引擎记录了这 4 项时四项必须同时有值（值的正确性由引擎用例 E-01/E-07 覆盖）；
    // 未记录时必须四项同时缺省，不允许半填。
    let recorded = fields
        .iter()
        .filter(|field| !result[*field].is_null())
        .count();
    assert!(
        recorded == 0 || recorded == 4,
        "四个字段必须同时有值或同时缺省：{result}"
    );
    if recorded == 4 {
        assert_eq!(result["vendor"], "gpt");
        assert_eq!(result["selection"], "auto");
        assert_eq!(result["reference_source"], "none");
        assert!(
            !result["model"].as_str().expect("model").is_empty(),
            "模型 id 不能为空：{result}"
        );
    }
}

/// 上一张图不可用 / 缺配置 → 400，错误文本保留中文可读说明。
#[test]
fn client_errors_map_to_bad_request_with_chinese_explanations() {
    let previous = ImageCreateApiError::from(ImageCreateError::PreviousImageUnavailable(
        "图片文件缺失或无法读取".to_string(),
    ));
    assert_eq!(previous.status, StatusCode::BAD_REQUEST);
    for expect in [
        "上一张图片不可用",
        "请关闭「基于上一张图继续修改」或上传参考图",
    ] {
        assert!(
            previous.error.contains(expect),
            "错误文本：{}",
            previous.error
        );
    }

    let missing = ImageCreateApiError::from(ImageCreateError::MissingConfig(
        "MiniMax 尚未配置 Base URL 或 API Key".to_string(),
    ));
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert!(
        missing
            .error
            .contains("MiniMax 尚未配置 Base URL 或 API Key"),
        "错误文本：{}",
        missing.error
    );
}
