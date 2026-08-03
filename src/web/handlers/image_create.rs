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
    while let Some(message) = receiver.next().await {
        let message = match message {
            Ok(Message::Text(text)) => text.to_string(),
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };

        match engine.start_iteration(&session_id, message).await {
            Ok(mut events) => {
                while let Some(event) = events.recv().await {
                    if !send_ws_event(&mut sender, &event).await {
                        return;
                    }
                }
            }
            Err(error) => {
                let event = IterationEvent {
                    kind: "error".to_string(),
                    text: None,
                    suggested_prompt: None,
                    provider_session_id: None,
                    error: Some(error.to_string()),
                };
                if !send_ws_event(&mut sender, &event).await {
                    return;
                }
            }
        }
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

    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use serde::de::DeserializeOwned;
    use serde_json::{Value, json};
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::image_client::{
        ImageClientApi, ImageClientError, ImageGenOutcome, ImageGenRequest, ImageRefImage,
    };
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::image_create::models::{
        ImageCreateSettings, SessionStoreApi, SettingsStoreApi,
    };
    use crate::product::image_create::{
        ImageCreateEngine, ImageCreateRunRegistry, SessionStore, SettingsStore,
    };
    use crate::web::app::build_web_router;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    struct FakeImageClient;

    #[async_trait]
    impl ImageClientApi for FakeImageClient {
        async fn generate(
            &self,
            _settings: &ImageCreateSettings,
            request: &ImageGenRequest,
            reference: Option<ImageRefImage>,
        ) -> Result<ImageGenOutcome, ImageClientError> {
            assert_eq!(request.prompt, "draw a cat");
            assert!(reference.is_none());
            Ok(ImageGenOutcome {
                media_type: "image/png".to_string(),
                b64: "ZmFrZS1pbWFnZQ==".to_string(),
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
            Arc::new(FakeImageClient),
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
}
