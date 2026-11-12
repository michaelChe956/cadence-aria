use std::sync::Arc;

use axum::Json;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Multipart, Path, State, WebSocketUpgrade};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::image_client::{ImageGenRequest, ImageRefImage, probe_connectivity};
use crate::cross_cutting::image_reference_validation::validate_reference_image;
use crate::product::image_create::engine::GenerateCommand;
use crate::product::image_create::image_files;
use crate::product::image_create::models::{
    CreateSessionRequest, ImageBackground, ImageCreateError, ImageOutputFormat, ImageQuality,
    ImageSize, ImageVendor, IterationEvent, SessionRecordDto, SessionStoreApi, SessionSummaryDto,
    SettingsStoreApi, SettingsUpdateRequest, validate_session_id,
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

    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: message.into(),
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            error: "image not found".to_string(),
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
            | ImageCreateError::MissingConfig(_)
            | ImageCreateError::RefImage(_)
            // 携带上一张图不可用属用户可自纠错（关闭开关或改上传参考图），映射 400。
            | ImageCreateError::PreviousImageUnavailable(_) => StatusCode::BAD_REQUEST,
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
    let sessions = sessions
        .into_iter()
        .map(SessionSummaryDto::from)
        .collect::<Vec<_>>();
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
    Ok(Json(SessionRecordDto::from(record)))
}

pub async fn delete_session(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
) -> HandlerResult<impl IntoResponse> {
    let id = checked_session_id(id)?;
    engine(&state)?.delete_session(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_session_image(
    State(state): State<WebAppState>,
    Path((session_id, image_id)): Path<(String, String)>,
) -> HandlerResult<Response> {
    let session_id = checked_session_id(session_id)?;
    if !image_files::valid_image_id(&image_id) {
        return Err(ImageCreateApiError::invalid_request("invalid image id"));
    }
    let record = session_store(&state)
        .get(&session_id)
        .await?
        .ok_or_else(ImageCreateApiError::not_found)?;
    let result = record
        .generation_results
        .iter()
        .find(|result| result.image_id.as_deref() == Some(image_id.as_str()))
        .ok_or_else(ImageCreateApiError::not_found)?;
    let bytes = image_files::read_result_image(&paths(&state), result)
        .await
        .ok_or_else(ImageCreateApiError::not_found)?;
    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_str(&result.media_type).expect("mime"),
        ),
        (
            header::ETAG,
            HeaderValue::from_str(&format!("\"{image_id}\"")).expect("etag"),
        ),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, max-age=31536000, immutable"),
        ),
    ];
    Ok((StatusCode::OK, headers, bytes).into_response())
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
    // 某家缺省 = 该家整体保留（apply_update 逐家合并）。
    let updated = store.apply_update(&current, update).await;
    // 校验点后移到合并之后：模型清单的跨家去重与轮转模型归属只能在整份设置上判定。
    // 校验失败返回 400 且不落盘。
    store.validate_settings(&updated).await?;
    store.save(&updated).await?;
    Ok(Json(store.to_masked(&updated).await))
}

/// 连通性探测请求：api_key 允许为空串或缺省 —— 为空则回落用该家已保存的 key。
#[derive(Debug, Deserialize)]
struct ConnectivityProbeRequest {
    vendor: ImageVendor,
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    models: Vec<String>,
}

/// POST /api/image-create/settings/connectivity：用草稿值探测供应方连通性
/// （不生成图片、不计费）。探测结果一律 200；仅入参非法才 400。
pub async fn probe_settings_connectivity(
    State(state): State<WebAppState>,
    Json(raw): Json<serde_json::Value>,
) -> HandlerResult<impl IntoResponse> {
    let request: ConnectivityProbeRequest = serde_json::from_value(raw)
        .map_err(|error| invalid_param(format!("探测请求不合法：{error}")))?;
    // key 回落：草稿非空优先；为空则用该家已保存的 key
    //（前端只有脱敏值、不可反推，回落只能在后端做）。
    let api_key = if request.api_key.trim().is_empty() {
        let saved = settings_store(&state).load().await;
        let saved_key = match request.vendor {
            ImageVendor::Gpt => saved.vendors.gpt.api_key,
            ImageVendor::Minimax => saved.vendors.minimax.api_key,
        };
        if saved_key.trim().is_empty() {
            return Err(invalid_param("未配置 API Key，无法测试连通性"));
        }
        saved_key
    } else {
        request.api_key
    };
    let report = probe_connectivity(request.vendor, &request.base_url, &api_key, &request.models)
        .await
        .map_err(|error| invalid_param(error.to_string()))?;
    Ok(Json(report))
}

pub async fn generate_image(
    State(state): State<WebAppState>,
    Path(id): Path<String>,
    multipart: Multipart,
) -> HandlerResult<impl IntoResponse> {
    let id = checked_session_id(id)?;
    let command = parse_generate_multipart(multipart).await?;
    let result = engine(&state)?.generate(&id, command).await?;
    // 3 个字段必有（本次结果所属的家 / 模型 / 参考图来源）。
    Ok(Json(json!({
        "image_id": result.image_id,
        "media_type": result.media_type,
        "vendor": result.vendor,
        "model": result.model,
        "reference_source": result.reference_source,
    })))
}

async fn parse_generate_multipart(mut multipart: Multipart) -> HandlerResult<GenerateCommand> {
    let mut prompt = None;
    let mut size = ImageSize::default();
    let mut quality = ImageQuality::default();
    let mut background = ImageBackground::default();
    let mut output_format = ImageOutputFormat::default();
    let mut model = None;
    let mut use_previous = None;
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
            // model：手动选定的模型 id，仅去首尾空白；必填（收尾处校验）。
            "model" => model = Some(field_text(field).await?),
            "use_previous" => {
                use_previous = Some(parse_use_previous(&field_text(field).await?)?);
            }
            // input_fidelity 等未知字段一律忽略（设计 5.5：不再解析该字段）。
            _ => {}
        }
    }

    let prompt = prompt.ok_or_else(|| invalid_param("prompt is required"))?;
    Ok(GenerateCommand {
        request: ImageGenRequest {
            prompt,
            size,
            quality,
            background,
            output_format,
        },
        model: model
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid_param("请选择图片生成模型"))?,
        use_previous: use_previous.unwrap_or(false),
        reference,
    })
}

/// use_previous 只接受 true / false；缺省或空白按 false 处理。
fn parse_use_previous(value: &str) -> HandlerResult<bool> {
    match value.trim() {
        "" => Ok(false),
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid_param("use_previous 仅支持 true 或 false")),
    }
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
#[path = "image_create_tests.rs"]
mod image_create_tests;

#[cfg(test)]
#[path = "image_create_endpoint_tests.rs"]
mod image_create_endpoint_tests;

#[cfg(test)]
#[path = "image_create_connectivity_tests.rs"]
mod image_create_connectivity_tests;
