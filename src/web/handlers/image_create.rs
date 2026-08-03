use std::sync::Arc;

use axum::Json;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Multipart, Path, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::image_client::{ImageGenRequest, ImageRefImage};
use crate::cross_cutting::image_reference_validation::validate_reference_image;
use crate::product::image_create::models::{
    CreateSessionRequest, ImageBackground, ImageCreateError, ImageOutputFormat, ImageQuality,
    ImageSize, IterationEvent, SessionStoreApi, SettingsStoreApi, SettingsUpdateRequest,
    validate_session_id,
};
use crate::product::image_create::{ImageCreateEngine, SessionStore, SettingsStore};
use crate::web::state::WebAppState;

#[derive(Debug, Serialize)]
pub struct ImageCreateApiError {
    #[serde(skip)]
    status: StatusCode,
    error: String,
}

impl ImageCreateApiError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: message.into(),
        }
    }
}

impl From<ImageCreateError> for ImageCreateApiError {
    fn from(error: ImageCreateError) -> Self {
        let status = match error {
            ImageCreateError::SessionNotFound | ImageCreateError::SessionGone => {
                StatusCode::NOT_FOUND
            }
            ImageCreateError::SessionClosing | ImageCreateError::SessionBusy => {
                StatusCode::CONFLICT
            }
            ImageCreateError::InvalidSessionId(_)
            | ImageCreateError::InvalidConfig(_)
            | ImageCreateError::InvalidParam(_)
            | ImageCreateError::MissingConfig
            | ImageCreateError::RefImage(_) => StatusCode::BAD_REQUEST,
            ImageCreateError::Store(_)
            | ImageCreateError::ImageClient(_)
            | ImageCreateError::Iteration(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            error: error.to_string(),
        }
    }
}

impl IntoResponse for ImageCreateApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}

type HandlerResult<T> = Result<T, ImageCreateApiError>;

fn paths(state: &WebAppState) -> AriaStatePaths {
    AriaStatePaths::from_workspace_root(&state.workspace_root)
}

fn session_store(state: &WebAppState) -> SessionStore {
    SessionStore::new(paths(state))
}

fn settings_store(state: &WebAppState) -> SettingsStore {
    SettingsStore::new(paths(state))
}

fn engine(state: &WebAppState) -> HandlerResult<Arc<ImageCreateEngine>> {
    state
        .image_create_engine
        .clone()
        .ok_or_else(|| ImageCreateApiError::internal("image creation engine is unavailable"))
}

fn checked_session_id(id: String) -> HandlerResult<String> {
    validate_session_id(&id).map_err(ImageCreateApiError::from)?;
    Ok(id)
}

pub async fn list_sessions(State(state): State<WebAppState>) -> HandlerResult<impl IntoResponse> {
    let sessions = session_store(&state).list().await?;
    Ok(Json(sessions))
}

pub async fn create_session(
    State(state): State<WebAppState>,
    Json(request): Json<CreateSessionRequest>,
) -> HandlerResult<impl IntoResponse> {
    let session = session_store(&state).create(request).await?;
    Ok((StatusCode::CREATED, Json(session)))
}

pub async fn get_session(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
) -> HandlerResult<impl IntoResponse> {
    let id = checked_session_id(id)?;
    let record = session_store(&state)
        .get(&id)
        .await?
        .ok_or(ImageCreateError::SessionNotFound)?;
    Ok(Json(record))
}

pub async fn delete_session(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
) -> HandlerResult<impl IntoResponse> {
    let id = checked_session_id(id)?;
    engine(&state)?.delete_session(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_settings(State(state): State<WebAppState>) -> HandlerResult<impl IntoResponse> {
    let store = settings_store(&state);
    let current = store.load().await;
    Ok(Json(store.to_masked(&current).await))
}

pub async fn update_settings(
    State(state): State<WebAppState>,
    Json(request): Json<SettingsUpdateRequest>,
) -> HandlerResult<impl IntoResponse> {
    let store = settings_store(&state);
    let current = store.load().await;
    let update = store.from_request(request).await;
    if let Some(base_url) = update.base_url.as_deref() {
        store.validate_base_url(base_url).await?;
    }
    let updated = store.apply_update(&current, update).await;
    store.save(&updated).await?;
    Ok(Json(store.to_masked(&updated).await))
}

pub async fn generate_image(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
    multipart: Multipart,
) -> HandlerResult<impl IntoResponse> {
    let id = checked_session_id(id)?;
    let (request, reference) = parse_generate_multipart(multipart).await?;
    let result = engine(&state)?.generate(&id, request, reference).await?;
    Ok(Json(json!({
        "media_type": result.media_type,
        "b64": result.b64,
    })))
}

async fn parse_generate_multipart(
    mut multipart: Multipart,
) -> HandlerResult<(ImageGenRequest, Option<ImageRefImage>)> {
    let mut prompt = None;
    let mut size = ImageSize::default();
    let mut quality = ImageQuality::default();
    let mut background = ImageBackground::default();
    let mut output_format = ImageOutputFormat::default();
    let mut input_fidelity = None;
    let mut reference = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| invalid_param(format!("invalid multipart body: {error}")))?
    {
        let Some(name) = field.name().map(str::to_owned) else {
            continue;
        };
        match name.as_str() {
            "reference" => {
                if reference.is_some() {
                    return Err(invalid_param("仅支持单张参考图"));
                }
                let declared_mime = field
                    .content_type()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid_param("reference content type is required"))?;
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|error| invalid_param(format!("invalid reference file: {error}")))?
                    .to_vec();
                let validated = validate_reference_image(&bytes, &declared_mime)
                    .map_err(|error| ImageCreateError::RefImage(error.to_string()))?;
                reference = Some(ImageRefImage {
                    bytes,
                    declared_mime: validated.media_type,
                });
            }
            "prompt" => prompt = Some(field_text(field).await?),
            "size" => size = parse_enum("size", &field_text(field).await?)?,
            "quality" => quality = parse_enum("quality", &field_text(field).await?)?,
            "background" => background = parse_enum("background", &field_text(field).await?)?,
            "output_format" => {
                output_format = parse_enum("output_format", &field_text(field).await?)?
            }
            "input_fidelity" => {
                let value = field_text(field).await?;
                if !value.trim().is_empty() {
                    input_fidelity = Some(parse_enum("input_fidelity", &value)?);
                }
            }
            _ => {}
        }
    }

    let prompt = prompt.ok_or_else(|| invalid_param("prompt is required"))?;
    Ok((
        ImageGenRequest {
            prompt,
            size,
            quality,
            background,
            output_format,
            input_fidelity,
        },
        reference,
    ))
}

async fn field_text(field: axum::extract::multipart::Field<'_>) -> HandlerResult<String> {
    field
        .text()
        .await
        .map_err(|error| invalid_param(format!("invalid multipart text field: {error}")))
}

fn parse_enum<T: DeserializeOwned>(name: &str, value: &str) -> HandlerResult<T> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|_| invalid_param(format!("invalid {name}: {value}")))
}

fn invalid_param(message: impl Into<String>) -> ImageCreateApiError {
    ImageCreateError::InvalidParam(message.into()).into()
}

pub async fn image_create_chat_ws(
    ws: WebSocketUpgrade,
    State(state): State<WebAppState>,
    Path(id): Path<String>,
) -> Response {
    let id = match checked_session_id(id) {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    let engine = match engine(&state) {
        Ok(engine) => engine,
        Err(error) => return error.into_response(),
    };

    ws.on_upgrade(move |socket| handle_chat_socket(socket, id, engine))
        .into_response()
}

async fn handle_chat_socket(socket: WebSocket, session_id: String, engine: Arc<ImageCreateEngine>) {
    let (mut sender, mut receiver) = socket.split();
    let mut active_iteration: Option<tokio::sync::mpsc::Receiver<IterationEvent>> = None;

    loop {
        if let Some(events) = active_iteration.as_mut() {
            tokio::select! {
                message = receiver.next() => {
                    match message {
                        Some(Ok(Message::Text(_))) => {
                            let event = iteration_error_event("会话忙，请等待当前任务完成");
                            if !send_ws_event(&mut sender, &event).await {
                                return;
                            }
                        }
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                        Some(Ok(_)) => {}
                    }
                }
                event = events.recv() => {
                    match event {
                        Some(event) => {
                            if !send_ws_event(&mut sender, &event).await {
                                return;
                            }
                        }
                        None => active_iteration = None,
                    }
                }
            }
            continue;
        }

        let message = match receiver.next().await {
            Some(Ok(Message::Text(text))) => text.to_string(),
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
            Some(Ok(_)) => continue,
        };
        match engine.start_iteration(&session_id, message).await {
            Ok(events) => active_iteration = Some(events),
            Err(error) => {
                let event = iteration_error_event(error.to_string());
                if !send_ws_event(&mut sender, &event).await {
                    return;
                }
            }
        }
    }
}

fn iteration_error_event(error: impl Into<String>) -> IterationEvent {
    IterationEvent {
        kind: "error".to_string(),
        text: None,
        suggested_prompt: None,
        provider_session_id: None,
        error: Some(error.into()),
    }
}

async fn send_ws_event<S>(sender: &mut S, event: &IterationEvent) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    let Ok(json) = serde_json::to_string(event) else {
        return false;
    };
    sender.send(Message::Text(json.into())).await.is_ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
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

    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::image_client::{
        ImageClient, ImageClientApi, ImageClientError, ImageGenOutcome, ImageGenRequest,
        ImageRefImage,
    };
    use crate::cross_cutting::image_reference_validation::MAX_BYTES;
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderCompletion, ProviderEvent, ProviderSession,
        StreamingProviderAdapter, StreamingProviderInput,
    };
    use crate::product::image_create::models::{
        ImageCreateSettings, SessionStoreApi, SettingsStoreApi,
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
            _settings: &ImageCreateSettings,
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

    fn generate_multipart(boundary: &str, references: &[&[u8]]) -> Vec<u8> {
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\ndraw a cat\r\n"
        )
        .into_bytes();
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
        SettingsStore::new(paths.clone())
            .save(&ImageCreateSettings {
                base_url: "https://images.example.com/v1".to_string(),
                api_key: "sk-test".to_string(),
                defaults: Default::default(),
            })
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
                    "base_url": "http://example.com/v1",
                    "api_key_action": "replace",
                    "api_key": "sk-secret1234",
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
                    "base_url": "https://images.example.com/v1",
                    "api_key_action": "replace",
                    "api_key": "sk-secret1234",
                    "defaults": null
                }),
            ))
            .await
            .expect("update response");
        assert_eq!(response.status(), StatusCode::OK);
        let masked: Value = response_json(response).await;
        assert_eq!(masked["api_key_masked"], "sk-****1234");
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
        assert_eq!(masked["base_url"], "https://images.example.com/v1");
        assert_eq!(masked["api_key_masked"], "sk-****1234");
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

        let settings = SettingsStore::new(paths.clone());
        settings
            .save(&ImageCreateSettings {
                base_url: "https://images.example.com/v1".to_string(),
                api_key: "sk-test".to_string(),
                defaults: Default::default(),
            })
            .await
            .expect("settings");
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
        assert_eq!(
            response_json::<Value>(response).await,
            json!({"media_type": "image/png", "b64": "ZmFrZS1pbWFnZQ=="})
        );
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
        SettingsStore::new(paths.clone())
            .save(&ImageCreateSettings {
                base_url: gateway.url(),
                api_key: secret.to_string(),
                defaults: Default::default(),
            })
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
        let (state, session_id) =
            configured_generate_state(root.path(), Some(reference.clone())).await;
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
        let (state, session_id) =
            configured_generate_state(root.path(), Some(reference.clone())).await;
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
        assert_eq!(
            response_json::<Value>(response).await,
            json!({"media_type": "image/png", "b64": "ZmFrZS1pbWFnZQ=="})
        );
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
}
