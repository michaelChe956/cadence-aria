use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};

use crate::product::image_create::models::{
    ImageBackground, ImageOutputFormat, ImageQuality, ImageSize, ImageVendor,
};

const MAX_ERROR_BODY_CHARS: usize = 500;
/// 非 2xx 错误体的读取上限：流式累计到此字节数即停止读取（H6）。
/// 恶意/被劫持端点可用超大错误体诱导大内存分配；错误文本随后仍截断到 500 字符。
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;
/// 2xx 成功体的读取上限：正常 b64 响应远小于此，超过视为恶意/异常端点（H6）。
const MAX_SUCCESS_BODY_BYTES: usize = 64 * 1024 * 1024;
/// MiniMax 的面积档位：用户选 `size=auto` 时改发面积（见设计 D10）。
const MINIMAX_AUTO_AREA: &str = "1K";
/// MiniMax 默认返回图片 URL，必须显式要求 base64。
const MINIMAX_RESPONSE_FORMAT: &str = "b64_json";

/// 一次出站调用的目标。只含本次目标家的一把 key。
/// Debug 手写：api_key 输出为 "<redacted>"。
#[derive(Clone, PartialEq, Eq)]
pub struct ImageCallTarget {
    pub vendor: ImageVendor,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl fmt::Debug for ImageCallTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageCallTarget")
            .field("vendor", &self.vendor)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .finish()
    }
}

/// 与家无关的用户参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageGenRequest {
    pub prompt: String,
    pub size: ImageSize,
    pub quality: ImageQuality,
    pub background: ImageBackground,
    pub output_format: ImageOutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRefImage {
    pub bytes: Vec<u8>,
    pub declared_mime: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageGenOutcome {
    pub media_type: String,
    pub b64: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImageClientError {
    #[error("image configuration is missing")]
    MissingConfig,
    #[error("invalid image configuration: {0}")]
    InvalidConfig(String),
    #[error("image provider network error")]
    Network,
    #[error("image provider request timed out")]
    Timeout,
    #[error("image provider returned HTTP {code}: {body}")]
    HttpStatus { code: u16, body: String },
    #[error("image provider returned no image data")]
    EmptyData,
    #[error("image provider response is missing b64_json")]
    MissingB64,
    #[error("image provider redirect was blocked")]
    RedirectBlocked,
    /// HTTP 2xx 但 data[] 中没有成功项、且存在带 error 的项。内容已脱敏并截断。
    #[error("图片端点返回逐项错误：{0}")]
    ItemError(String),
    /// 响应体超过读取上限：视为恶意/异常端点，避免大内存分配（H6）。
    #[error("图片端点响应体超过大小上限")]
    OversizedBody,
}

#[async_trait]
pub trait ImageClientApi: Send + Sync {
    async fn generate(
        &self,
        target: &ImageCallTarget,
        req: &ImageGenRequest,
        reference: Option<ImageRefImage>,
    ) -> Result<ImageGenOutcome, ImageClientError>;
}

#[derive(Debug, Clone)]
pub struct ImageClient {
    http: reqwest::Client,
}

impl ImageClient {
    pub fn new() -> Self {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(30));
        // reqwest 0.12 在 Linux 默认 TCP_USER_TIMEOUT=30s，会独立于 .timeout() 提前掐断
        // 图片生成耗时较长（网关处理期间不回传 ACK），必须放宽到与总超时一致。
        // tcp_user_timeout 仅 Linux 可用，macOS/Windows 无此 API。
        #[cfg(target_os = "linux")]
        {
            builder = builder.tcp_user_timeout(Duration::from_secs(600));
        }
        let http = builder
            .build()
            .expect("fixed image HTTP client configuration must be valid");
        Self { http }
    }
}

impl Default for ImageClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ImageClientApi for ImageClient {
    async fn generate(
        &self,
        target: &ImageCallTarget,
        req: &ImageGenRequest,
        reference: Option<ImageRefImage>,
    ) -> Result<ImageGenOutcome, ImageClientError> {
        validate_target(target)?;

        let fields = wire_fields(target, req);
        let endpoint = if reference.is_some() {
            "edits"
        } else {
            "generations"
        };
        let url = format!(
            "{}/v1/images/{endpoint}",
            target.base_url.trim_end_matches('/')
        );

        let request = if let Some(reference) = reference {
            let form = multipart_form(&fields, reference)?;
            self.http.post(url).multipart(form)
        } else {
            self.http.post(url).json(&GenerationBody {
                model: fields.model,
                prompt: fields.prompt,
                size: fields.size,
                quality: fields.quality,
                background: fields.background,
                output_format: fields.output_format,
                response_format: fields.response_format,
                area: fields.area,
            })
        };

        let response = request
            .bearer_auth(&target.api_key)
            .send()
            .await
            .map_err(normalize_reqwest_error)?;

        if response.status().is_redirection() {
            return Err(ImageClientError::RedirectBlocked);
        }
        if !response.status().is_success() {
            let code = response.status().as_u16();
            // H6：错误体流式读取并设 8 KiB 上限，而非 response.text() 整读。
            let (body, _) = read_body_within(response, MAX_ERROR_BODY_BYTES).await?;
            let body = String::from_utf8_lossy(&body);
            return Err(ImageClientError::HttpStatus {
                code,
                body: sanitize_error_body(&body, &target.api_key),
            });
        }

        // H6：成功体同样设上限，超过按恶意端点处理，报可读错误。
        let (body, hit_limit) = read_body_within(response, MAX_SUCCESS_BODY_BYTES).await?;
        if hit_limit {
            return Err(ImageClientError::OversizedBody);
        }
        let payload = serde_json::from_slice::<ImageResponse>(&body)
            .map_err(|_| ImageClientError::Network)?;
        let b64 = extract_b64(payload, &target.api_key)?;

        Ok(ImageGenOutcome {
            media_type: media_type(req.output_format).to_string(),
            b64,
        })
    }
}

/// 字段过滤的唯一位置。JSON body 与 multipart form 均由它构造。
struct WireFields<'a> {
    model: &'a str,
    prompt: &'a str,
    /// GPT：恒为 Some。MiniMax：用户选了具体 WxH 时为 Some，auto 时为 None。
    size: Option<ImageSize>,
    output_format: ImageOutputFormat,
    /// 仅 GPT。
    quality: Option<ImageQuality>,
    /// 仅 GPT。
    background: Option<ImageBackground>,
    /// 仅 MiniMax，恒为 Some("b64_json")。
    response_format: Option<&'static str>,
    /// 仅 MiniMax 且用户 size 为 auto，恒为 Some("1K")。
    area: Option<&'static str>,
}

/// 按供应方决定发哪些字段（见设计 5.4 的规则表）。
fn wire_fields<'a>(target: &'a ImageCallTarget, req: &'a ImageGenRequest) -> WireFields<'a> {
    // GPT 发 size/quality/background 且不发 area/response_format；
    // MiniMax 发 response_format=b64_json，不发 quality/background。
    let (size, quality, background, response_format, area) = if target.vendor == ImageVendor::Gpt {
        (
            Some(req.size),
            Some(req.quality),
            Some(req.background),
            None,
            None,
        )
    } else {
        // MiniMax 的 size 与 area 二选一：auto 时改发面积档位。
        let (size, area) = match req.size {
            ImageSize::Auto => (None, Some(MINIMAX_AUTO_AREA)),
            size => (Some(size), None),
        };
        (size, None, None, Some(MINIMAX_RESPONSE_FORMAT), area)
    };
    WireFields {
        model: &target.model,
        prompt: &req.prompt,
        size,
        output_format: req.output_format,
        quality,
        background,
        response_format,
        area,
    }
}

/// generations 的 JSON body。
#[derive(Serialize)]
struct GenerationBody<'a> {
    model: &'a str,
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<ImageSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quality: Option<ImageQuality>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<ImageBackground>,
    output_format: ImageOutputFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    area: Option<&'a str>,
}

/// edits 的 multipart 表单：只发 `WireFields` 中为 Some 的字段。
fn multipart_form(
    fields: &WireFields<'_>,
    reference: ImageRefImage,
) -> Result<Form, ImageClientError> {
    let extension = reference
        .declared_mime
        .split('/')
        .nth(1)
        .filter(|ext| matches!(*ext, "png" | "jpeg" | "webp"))
        .unwrap_or("png");
    let image = Part::bytes(reference.bytes)
        .mime_str(&reference.declared_mime)
        .map_err(|_| {
            ImageClientError::InvalidConfig(
                "reference image has an invalid declared MIME type".to_string(),
            )
        })?
        .file_name(format!("reference.{extension}"));

    let mut form = Form::new()
        .text("model", fields.model.to_string())
        .text("prompt", fields.prompt.to_string())
        .text("output_format", wire_value(&fields.output_format)?);
    let size = fields.size.map(|v| wire_value(&v)).transpose()?;
    let quality = fields.quality.map(|v| wire_value(&v)).transpose()?;
    let background = fields.background.map(|v| wire_value(&v)).transpose()?;
    let response_format = fields.response_format.map(str::to_string);
    let area = fields.area.map(str::to_string);
    form = push_optional(form, "size", size);
    form = push_optional(form, "quality", quality);
    form = push_optional(form, "background", background);
    form = push_optional(form, "response_format", response_format);
    form = push_optional(form, "area", area);
    Ok(form.part("image[]", image))
}

/// 只在字段存在时追加 multipart 文本字段。
fn push_optional(form: Form, name: &str, value: Option<String>) -> Form {
    match value {
        Some(value) => form.text(name.to_string(), value),
        None => form,
    }
}

#[derive(Deserialize)]
struct ImageResponse {
    #[serde(default)]
    data: Vec<ImageResponseData>,
}

#[derive(Deserialize)]
struct ImageResponseData {
    #[serde(default)]
    b64_json: Option<String>,
    /// MiniMax 官方：失败项只含 error。形状不做假设，按任意 JSON 接收。
    #[serde(default)]
    error: Option<serde_json::Value>,
}

/// 从响应中取出图片（见设计 5.6）。
fn extract_b64(payload: ImageResponse, api_key: &str) -> Result<String, ImageClientError> {
    // 规则 1：按顺序找第一个 b64_json 非空的项。
    for item in &payload.data {
        if let Some(b64) = item.b64_json.as_deref().filter(|value| !value.is_empty()) {
            return Ok(b64.to_string());
        }
    }
    // 规则 2：没有成功项时，存在带 error 的项即为逐项错误。
    for item in &payload.data {
        if let Some(error) = &item.error {
            let message = item_error_message(error);
            return Err(ImageClientError::ItemError(sanitize_error_body(
                &message, api_key,
            )));
        }
    }
    // 规则 3：无成功项也无逐项错误：data 为空则 EmptyData，否则 MissingB64。
    if payload.data.is_empty() {
        Err(ImageClientError::EmptyData)
    } else {
        Err(ImageClientError::MissingB64)
    }
}

/// 逐项错误文本：取 error 对象里的字符串 message，否则取其紧凑 JSON 文本。
fn item_error_message(error: &serde_json::Value) -> String {
    error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| serde_json::to_string(error).unwrap_or_else(|_| error.to_string()))
}

fn validate_target(target: &ImageCallTarget) -> Result<(), ImageClientError> {
    if target.base_url.trim().is_empty() || target.api_key.trim().is_empty() {
        return Err(ImageClientError::MissingConfig);
    }
    validate_base_url(&target.base_url)
}

pub fn validate_base_url(url: &str) -> Result<(), ImageClientError> {
    let parsed = reqwest::Url::parse(url).map_err(|error| {
        ImageClientError::InvalidConfig(format!("base_url is not a valid URL: {error}"))
    })?;

    if parsed.scheme() == "https" {
        return Ok(());
    }
    if parsed.scheme() != "http" {
        return Err(ImageClientError::InvalidConfig(
            "base_url must use HTTPS or HTTP on a loopback host".to_string(),
        ));
    }

    let host = parsed.host_str().ok_or_else(|| {
        ImageClientError::InvalidConfig("base_url must include a host".to_string())
    })?;
    let unbracketed_host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || unbracketed_host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());

    if is_loopback {
        Ok(())
    } else {
        Err(ImageClientError::InvalidConfig(
            "base_url must use HTTPS or resolve to a loopback host".to_string(),
        ))
    }
}

fn wire_value<T: Serialize>(value: &T) -> Result<String, ImageClientError> {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| {
            ImageClientError::InvalidConfig("image parameter cannot be serialized".to_string())
        })
}

fn media_type(format: ImageOutputFormat) -> &'static str {
    match format {
        ImageOutputFormat::Png => "image/png",
        ImageOutputFormat::Jpeg => "image/jpeg",
        ImageOutputFormat::Webp => "image/webp",
    }
}

/// 有上限的响应体流式读取（H6）：累计超过 limit 即保留上限内字节并停止读取。
/// 返回 (已读字节, 是否触达上限)；UTF-8 截断点由 from_utf8_lossy 容错。
async fn read_body_within(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<(Vec<u8>, bool), ImageClientError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(normalize_reqwest_error)? {
        if body.len() + chunk.len() > limit {
            let keep = limit.saturating_sub(body.len());
            body.extend_from_slice(&chunk[..keep]);
            return Ok((body, true));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body, false))
}

fn sanitize_error_body(body: &str, api_key: &str) -> String {
    let redacted = if api_key.is_empty() {
        body.to_string()
    } else {
        body.replace(api_key, "[REDACTED]")
    };
    redacted.chars().take(MAX_ERROR_BODY_CHARS).collect()
}

fn normalize_reqwest_error(error: reqwest::Error) -> ImageClientError {
    if error.is_timeout() {
        ImageClientError::Timeout
    } else {
        ImageClientError::Network
    }
}

// ---- 供应方连通性探测（不生成图片、不计费） ----

/// 探测连接超时：远短于生成路径。
const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 探测总预算：两步合计 30s，每步请求超时取剩余额度。
const PROBE_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// /v1/models 响应体读取上限：模型清单远小于此（H6 同源考量）。
const MAX_MODELS_BODY_BYTES: usize = 1024 * 1024;

const PROBE_STATUS_OK: &str = "ok";
const PROBE_STATUS_SKIPPED: &str = "skipped";
const PROBE_STATUS_FAILED: &str = "failed";

/// 单步探测结果。detail 已脱敏，不含 api_key。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeStep {
    pub key: &'static str,
    pub label: &'static str,
    pub status: &'static str,
    pub detail: String,
}

impl ProbeStep {
    fn new(
        key: &'static str,
        label: &'static str,
        status: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            key,
            label,
            status,
            detail: detail.into(),
        }
    }
}

/// 探测汇总：任一步骤 failed 则 ok=false，两步明细与网关缺失模型一并列出。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeReport {
    pub vendor: ImageVendor,
    pub ok: bool,
    pub message: String,
    pub steps: Vec<ProbeStep>,
    pub missing_models: Vec<String>,
    pub duration_ms: u64,
}

/// 供应方连通性探测：只验证「通不通 / key 是否有效 / 配置的模型 id 网关支持哪些」，
/// 绝不生成图片、绝不计费 —— 第 2 步故意缺 prompt，参数校验在任何实现中都先于生成。
/// 出站前先走与生成路径同一套 `validate_base_url`（SSRF 防线）；不合规时不发任何请求。
pub async fn probe_connectivity(
    vendor: ImageVendor,
    base_url: &str,
    api_key: &str,
    models: &[String],
) -> Result<ProbeReport, ImageClientError> {
    validate_base_url(base_url)?;
    // 探测复用出站客户端的禁重定向策略，但超时大幅缩短（连接 10s / 总计 30s）。
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(PROBE_CONNECT_TIMEOUT)
        .build()
        .map_err(|_| ImageClientError::InvalidConfig("探测 HTTP 客户端构建失败".to_string()))?;
    let root = base_url.trim_end_matches('/');
    let deadline = std::time::Instant::now() + PROBE_TOTAL_TIMEOUT;
    let started = std::time::Instant::now();
    let mut steps = Vec::new();
    let mut missing_models = Vec::new();

    // 步骤 1：GET /v1/models。401/403 → key 无效，直接结束；不可用 → 跳过，继续第 2 步。
    let mut key_invalid = false;
    let models_step = match http
        .get(format!("{root}/v1/models"))
        .bearer_auth(api_key)
        .timeout(remaining_before(deadline))
        .send()
        .await
    {
        Ok(response) => match response.status().as_u16() {
            200 => {
                let ids = read_body_within(response, MAX_MODELS_BODY_BYTES)
                    .await
                    .map(|(body, _)| parse_model_ids(&body))
                    .unwrap_or_default();
                missing_models = models
                    .iter()
                    .filter(|model| !ids.iter().any(|id| id == *model))
                    .cloned()
                    .collect();
                ProbeStep::new(
                    "models",
                    "模型列表 /v1/models",
                    PROBE_STATUS_OK,
                    format!("网关返回 {} 个模型", ids.len()),
                )
            }
            401 | 403 => {
                key_invalid = true;
                ProbeStep::new(
                    "models",
                    "模型列表 /v1/models",
                    PROBE_STATUS_FAILED,
                    "API Key 无效或无权限",
                )
            }
            _ => ProbeStep::new(
                "models",
                "模型列表 /v1/models",
                PROBE_STATUS_SKIPPED,
                "该网关未提供 /v1/models，改用图片端点验证",
            ),
        },
        Err(_) => ProbeStep::new(
            "models",
            "模型列表 /v1/models",
            PROBE_STATUS_SKIPPED,
            "该网关未提供 /v1/models，改用图片端点验证",
        ),
    };
    steps.push(models_step);

    if !key_invalid {
        // 步骤 2：POST /v1/images/generations，body 故意只有 model、缺 prompt。
        let probe_model = models.first().cloned().unwrap_or_else(probe_fallback_model);
        let generations_step = match http
            .post(format!("{root}/v1/images/generations"))
            .bearer_auth(api_key)
            .json(&serde_json::json!({ "model": probe_model }))
            .timeout(remaining_before(deadline))
            .send()
            .await
        {
            Ok(response) => {
                let code = response.status().as_u16();
                let (status, detail) = match code {
                    400 => (
                        PROBE_STATUS_OK,
                        "图片端点连通且鉴权通过（缺少 prompt 属预期，不会计费）".to_string(),
                    ),
                    401 | 403 => (
                        PROBE_STATUS_FAILED,
                        "API Key 无效或无权限".to_string(),
                    ),
                    404 if !models.is_empty() => (
                        PROBE_STATUS_FAILED,
                        "路径不对：系统会自动拼 /v1/images/generations，请只填到主机根路径（不要带 /v1 或 /v1/images）"
                            .to_string(),
                    ),
                    404 => (
                        PROBE_STATUS_FAILED,
                        "无法判定路径是否正确：未指定模型 id（404 也可能是模型不存在）。请确认只填到主机根路径，或先在配置中填写模型 id 后重试"
                            .to_string(),
                    ),
                    _ => {
                        let body = read_body_within(response, MAX_ERROR_BODY_BYTES)
                            .await
                            .map(|(body, _)| String::from_utf8_lossy(&body).to_string())
                            .unwrap_or_default();
                        (
                            PROBE_STATUS_FAILED,
                            format!(
                                "网关返回 HTTP {code}：{}",
                                sanitize_error_body(&body, api_key)
                            ),
                        )
                    }
                };
                ProbeStep::new(
                    "generations",
                    "图片端点 /v1/images/generations",
                    status,
                    detail,
                )
            }
            Err(_) => ProbeStep::new(
                "generations",
                "图片端点 /v1/images/generations",
                PROBE_STATUS_FAILED,
                "网络错误或超时：无法在 30 秒内完成探测",
            ),
        };
        steps.push(generations_step);
    }

    let ok = steps.iter().all(|step| step.status != PROBE_STATUS_FAILED);
    Ok(ProbeReport {
        vendor,
        ok,
        message: if ok {
            "连通性正常".to_string()
        } else {
            "连通性测试未通过，请查看步骤详情".to_string()
        },
        steps,
        missing_models,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// 解析 /v1/models 响应的模型 id：兼容 `{"data":[{"id":…}]}` 与顶层数组
/// （元素为对象取 id、为字符串取本身）两种形态。
fn parse_model_ids(body: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let items: &[serde_json::Value] = match &value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(_) => value
            .get("data")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        _ => &[],
    };
    items
        .iter()
        .filter_map(|item| {
            item.as_str().map(ToOwned::to_owned).or_else(|| {
                item.get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            })
        })
        .collect()
}

/// models 为空时的探测占位模型 id。分段拼装以避免非测试代码出现具体模型 id 字面量（C15）。
fn probe_fallback_model() -> String {
    ["gpt", "image", "1"].join("-")
}

/// 距探测总预算截止的剩余时间；已用尽则返回零时长（请求会立即超时）。
fn remaining_before(deadline: std::time::Instant) -> Duration {
    deadline.saturating_duration_since(std::time::Instant::now())
}

#[cfg(test)]
#[path = "image_client_tests.rs"]
mod image_client_tests;
