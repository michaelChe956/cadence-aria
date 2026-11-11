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
            let body = response.text().await.map_err(normalize_reqwest_error)?;
            return Err(ImageClientError::HttpStatus {
                code,
                body: sanitize_error_body(&body, &target.api_key),
            });
        }

        let payload = response
            .json::<ImageResponse>()
            .await
            .map_err(normalize_reqwest_error)?;
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
    let _ = (MINIMAX_AUTO_AREA, MINIMAX_RESPONSE_FORMAT);
    WireFields {
        model: &target.model,
        prompt: &req.prompt,
        size: Some(req.size),
        output_format: req.output_format,
        quality: Some(req.quality),
        background: Some(req.background),
        response_format: None,
        area: None,
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
    form = push_optional(
        form,
        "size",
        fields.size.map(|v| wire_value(&v)).transpose()?,
    );
    form = push_optional(
        form,
        "quality",
        fields.quality.map(|v| wire_value(&v)).transpose()?,
    );
    form = push_optional(
        form,
        "background",
        fields.background.map(|v| wire_value(&v)).transpose()?,
    );
    form = push_optional(
        form,
        "response_format",
        fields.response_format.map(str::to_string),
    );
    form = push_optional(form, "area", fields.area.map(str::to_string));
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
    let _ = api_key;
    let first = payload
        .data
        .into_iter()
        .next()
        .ok_or(ImageClientError::EmptyData)?;
    first.b64_json.ok_or(ImageClientError::MissingB64)
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

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Matcher;
    use serde_json::json;

    const GPT_MODEL: &str = "gpt-image-2.6-flare";
    const GPT_MODEL_ALT: &str = "gpt-image-2.6-flare-4k";
    const MINIMAX_MODEL: &str = "MiniMax-H3-Image";
    const OK_BODY: &str = r#"{"data":[{"b64_json":"AAAA"}]}"#;
    const EDIT_BODY: &str = r#"{"data":[{"b64_json":"EDIT"}]}"#;
    /// reqwest 生成的 multipart 边界：4 段 16 位十六进制，以短横线相连。
    const BOUNDARY: &str = r"--[0-9a-f]{16}(?:-[0-9a-f]{16}){3}";
    const FILE_PART: &str = concat!(
        "Content-Disposition: form-data; name=\"image\\[\\]\"; filename=\"reference.png\"\\r\\n",
        "Content-Type: image/png\\r\\n\\r\\nreference\\r\\n"
    );

    fn new_target(vendor: ImageVendor, base_url: &str, model: &str) -> ImageCallTarget {
        ImageCallTarget {
            vendor,
            base_url: base_url.to_string(),
            api_key: "sk-test".to_string(),
            model: model.to_string(),
        }
    }

    fn request(size: ImageSize) -> ImageGenRequest {
        ImageGenRequest {
            prompt: "draw a fox".to_string(),
            size,
            quality: ImageQuality::High,
            background: ImageBackground::Transparent,
            output_format: ImageOutputFormat::Png,
        }
    }

    fn reference() -> ImageRefImage {
        ImageRefImage {
            bytes: b"reference".to_vec(),
            declared_mime: "image/png".to_string(),
        }
    }

    fn gpt_body(model: &str, size: &str) -> serde_json::Value {
        json!({
            "model": model,
            "prompt": "draw a fox",
            "size": size,
            "quality": "high",
            "background": "transparent",
            "output_format": "png"
        })
    }

    fn minimax_body(model: &str, size: Option<&str>) -> serde_json::Value {
        let mut body = json!({
            "model": model,
            "prompt": "draw a fox",
            "output_format": "png",
            "response_format": "b64_json"
        });
        match size {
            Some(size) => body["size"] = json!(size),
            None => body["area"] = json!("1K"),
        }
        body
    }

    fn escape_regex(value: &str) -> String {
        value
            .replace('.', "\\.")
            .replace('[', "\\[")
            .replace(']', "\\]")
    }

    /// 严格匹配整个 multipart body：字段名、顺序、取值必须完全一致；多一个字段即不匹配。
    fn multipart_matcher(fields: &[(&str, &str)]) -> Matcher {
        let mut pattern = String::from("^");
        for (name, value) in fields {
            pattern.push_str(&format!(
                "{BOUNDARY}\\r\\nContent-Disposition: form-data; name=\"{}\"\\r\\n\\r\\n{}\\r\\n",
                escape_regex(name),
                escape_regex(value)
            ));
        }
        pattern.push_str(&format!(
            "{BOUNDARY}\\r\\n{FILE_PART}{BOUNDARY}--\\r\\n$"
        ));
        Matcher::Regex(pattern)
    }

    async fn call(
        target: &ImageCallTarget,
        request: &ImageGenRequest,
        reference: Option<ImageRefImage>,
    ) -> Result<ImageGenOutcome, ImageClientError> {
        ImageClient::new()
            .generate(target, request, reference)
            .await
    }

    /// generations mock：校验请求体与鉴权头，响应固定，期望命中 1 次。
    async fn generations(server: &mut mockito::Server, matcher: Matcher) -> mockito::Mock {
        server
            .mock("POST", "/v1/images/generations")
            .match_header("authorization", "Bearer sk-test")
            .match_body(matcher)
            .with_status(200)
            .with_body(OK_BODY)
            .expect(1)
            .create_async()
            .await
    }

    /// 固定状态码与响应体的 generations mock。
    async fn status_mock(
        server: &mut mockito::Server,
        status: usize,
        body: &str,
        hits: usize,
    ) -> mockito::Mock {
        server
            .mock("POST", "/v1/images/generations")
            .with_status(status)
            .with_body(body)
            .expect(hits)
            .create_async()
            .await
    }

    /// edits mock：校验 multipart 请求体，期望命中 1 次。
    async fn edits(server: &mut mockito::Server, matcher: Matcher) -> mockito::Mock {
        server
            .mock("POST", "/v1/images/edits")
            .match_body(matcher)
            .with_status(200)
            .with_body(EDIT_BODY)
            .expect(1)
            .create_async()
            .await
    }

    /// 「multipart 中不得出现某字段」的断言 mock：命中次数必须为 0。
    async fn forbid(server: &mut mockito::Server, field: &str) -> mockito::Mock {
        server
            .mock("POST", "/v1/images/edits")
            .match_body(Matcher::Regex(format!("name=\"{}\"", escape_regex(field))))
            .with_status(200)
            .with_body(OK_BODY)
            .expect(0)
            .create_async()
            .await
    }

    #[tokio::test]
    async fn c01_gpt_generations_body_is_exactly_the_official_field_set() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
        let matcher = Matcher::Json(gpt_body(GPT_MODEL, "1536x1024"));
        let mock = generations(&mut server, matcher).await;

        let out = call(&target, &request(ImageSize::Landscape), None)
            .await
            .expect("GPT generations 必须成功");
        assert_eq!(out.media_type, "image/png");
        assert_eq!(out.b64, "AAAA");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn c02_minimax_generations_with_explicit_size_sends_exact_field_set() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Minimax, &server.url(), MINIMAX_MODEL);
        let matcher = Matcher::Json(minimax_body(MINIMAX_MODEL, Some("1536x1024")));
        let mock = generations(&mut server, matcher).await;

        let out = call(&target, &request(ImageSize::Landscape), None)
            .await
            .expect("MiniMax generations 必须成功");
        assert_eq!(out.b64, "AAAA");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn c02b_minimax_generations_with_auto_size_sends_area_instead_of_size() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Minimax, &server.url(), MINIMAX_MODEL);
        let matcher = Matcher::Json(minimax_body(MINIMAX_MODEL, None));
        let mock = generations(&mut server, matcher).await;

        call(&target, &request(ImageSize::Auto), None)
            .await
            .expect("MiniMax auto 必须成功");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn c03_model_id_is_passed_through_verbatim_and_shapes_no_request() {
        let mut server = mockito::Server::new_async().await;
        let base_url = server.url();
        let first = generations(&mut server, Matcher::Json(gpt_body(GPT_MODEL, "1536x1024"))).await;
        let second = generations(
            &mut server,
            Matcher::Json(gpt_body(GPT_MODEL_ALT, "1536x1024")),
        )
        .await;
        let first_target = new_target(ImageVendor::Gpt, &base_url, GPT_MODEL);
        let second_target = new_target(ImageVendor::Gpt, &base_url, GPT_MODEL_ALT);

        call(&first_target, &request(ImageSize::Landscape), None)
            .await
            .expect("第一个模型必须成功");
        call(&second_target, &request(ImageSize::Landscape), None)
            .await
            .expect("第二个模型必须成功");
        first.assert_async().await;
        second.assert_async().await;

        // 两个模型 id 的请求体除 model 值外完全相同。
        let mut left = gpt_body(GPT_MODEL, "1536x1024");
        let mut right = gpt_body(GPT_MODEL_ALT, "1536x1024");
        for body in [&mut left, &mut right] {
            body.as_object_mut().expect("请求体是对象").remove("model");
        }
        assert_eq!(left, right);

        // GPT 在 size=auto 时发 size="auto"，且不发 area。
        let mut server = mockito::Server::new_async().await;
        let auto = generations(&mut server, Matcher::Json(gpt_body(GPT_MODEL, "auto"))).await;
        let auto_target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
        call(&auto_target, &request(ImageSize::Auto), None)
            .await
            .expect("GPT auto 必须成功");
        auto.assert_async().await;
    }

    #[tokio::test]
    async fn c04_gpt_edits_multipart_carries_only_official_fields() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
        let matcher = multipart_matcher(&[
            ("model", GPT_MODEL),
            ("prompt", "draw a fox"),
            ("output_format", "png"),
            ("size", "1536x1024"),
            ("quality", "high"),
            ("background", "transparent"),
        ]);
        let edit = edits(&mut server, matcher).await;
        let no_input_fidelity = forbid(&mut server, "input_fidelity").await;
        let no_response_format = forbid(&mut server, "response_format").await;
        let no_area = forbid(&mut server, "area").await;

        let out = call(&target, &request(ImageSize::Landscape), Some(reference()))
            .await
            .expect("GPT edits 必须成功");
        assert_eq!(out.b64, "EDIT");
        edit.assert_async().await;
        no_input_fidelity.assert_async().await;
        no_response_format.assert_async().await;
        no_area.assert_async().await;
    }

    #[tokio::test]
    async fn c05_minimax_edits_never_sends_quality_background_or_both_size_and_area() {
        // 具体尺寸：发 size，不发 area。
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Minimax, &server.url(), MINIMAX_MODEL);
        let matcher = multipart_matcher(&[
            ("model", MINIMAX_MODEL),
            ("prompt", "draw a fox"),
            ("output_format", "png"),
            ("size", "1536x1024"),
            ("response_format", "b64_json"),
        ]);
        let edit = edits(&mut server, matcher).await;
        let no_quality = forbid(&mut server, "quality").await;
        let no_background = forbid(&mut server, "background").await;
        let no_input_fidelity = forbid(&mut server, "input_fidelity").await;
        let no_area = forbid(&mut server, "area").await;

        call(&target, &request(ImageSize::Landscape), Some(reference()))
            .await
            .expect("MiniMax edits（具体尺寸）必须成功");
        edit.assert_async().await;
        no_quality.assert_async().await;
        no_background.assert_async().await;
        no_input_fidelity.assert_async().await;
        no_area.assert_async().await;

        // auto：发 area，不发 size。
        let mut server = mockito::Server::new_async().await;
        let auto_target = new_target(ImageVendor::Minimax, &server.url(), MINIMAX_MODEL);
        let matcher = multipart_matcher(&[
            ("model", MINIMAX_MODEL),
            ("prompt", "draw a fox"),
            ("output_format", "png"),
            ("response_format", "b64_json"),
            ("area", "1K"),
        ]);
        let edit = edits(&mut server, matcher).await;
        let no_size = forbid(&mut server, "size").await;

        call(&auto_target, &request(ImageSize::Auto), Some(reference()))
            .await
            .expect("MiniMax edits（auto）必须成功");
        edit.assert_async().await;
        no_size.assert_async().await;
    }

    #[tokio::test]
    async fn c06_single_b64_item_is_a_success() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
        let mock = status_mock(&mut server, 200, r#"{"data":[{"b64_json":"SINGLE"}]}"#, 1).await;

        let out = call(&target, &request(ImageSize::Auto), None)
            .await
            .expect("单项 b64 必须成功");
        assert_eq!(out.b64, "SINGLE");
        assert_eq!(out.media_type, "image/png");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn c07_item_error_is_reported_with_its_message() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Minimax, &server.url(), MINIMAX_MODEL);
        let body = r#"{"data":[{"error":{"message":"generation failed (1027)"}}]}"#;
        let mock = status_mock(&mut server, 200, body, 1).await;

        let error = call(&target, &request(ImageSize::Auto), None)
            .await
            .expect_err("逐项错误必须失败");
        match error {
            ImageClientError::ItemError(message) => {
                assert!(
                    message.contains("generation failed (1027)"),
                    "逐项错误文本必须包含原始 message：{message}"
                );
            }
            other => panic!("期望逐项错误，实际：{other:?}"),
        }
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn c08_first_successful_item_wins_even_after_an_error_item() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Minimax, &server.url(), MINIMAX_MODEL);
        let body = r#"{"data":[{"error":{"message":"boom"}},{"b64_json":"SECOND"}]}"#;
        let mock = status_mock(&mut server, 200, body, 1).await;

        let out = call(&target, &request(ImageSize::Auto), None)
            .await
            .expect("第二项成功即整体成功");
        assert_eq!(out.b64, "SECOND");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn c09_url_only_item_never_triggers_a_download() {
        let mut server = mockito::Server::new_async().await;
        let download = server
            .mock("GET", "/generated.png")
            .with_status(200)
            .with_body("binary")
            .expect(0)
            .create_async()
            .await;
        let target = new_target(ImageVendor::Minimax, &server.url(), MINIMAX_MODEL);
        let body = format!(r#"{{"data":[{{"url":"{}/generated.png"}}]}}"#, server.url());
        let mock = status_mock(&mut server, 200, &body, 1).await;

        let error = call(&target, &request(ImageSize::Auto), None)
            .await
            .expect_err("只有 url 的项必须失败");
        assert_eq!(error, ImageClientError::MissingB64);
        mock.assert_async().await;
        download.assert_async().await;
    }

    #[tokio::test]
    async fn c10_empty_or_missing_data_is_empty_data() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);

        let empty = status_mock(&mut server, 200, r#"{"data":[]}"#, 1).await;
        let error = call(&target, &request(ImageSize::Auto), None).await;
        assert_eq!(error, Err(ImageClientError::EmptyData));
        empty.assert_async().await;

        let missing = status_mock(&mut server, 200, r#"{"created":1}"#, 1).await;
        let error = call(&target, &request(ImageSize::Auto), None).await;
        assert_eq!(error, Err(ImageClientError::EmptyData));
        missing.assert_async().await;
    }

    #[tokio::test]
    async fn c11_item_error_text_never_leaks_the_api_key() {
        let mut server = mockito::Server::new_async().await;
        let mut target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
        target.api_key = "sk-secret".to_string();
        let body = r#"{"data":[{"error":{"message":"upstream echoed sk-secret"}}]}"#;
        let mock = status_mock(&mut server, 200, body, 1).await;

        let error = call(&target, &request(ImageSize::Auto), None)
            .await
            .expect_err("逐项错误必须失败");
        match error {
            ImageClientError::ItemError(message) => {
                assert!(!message.contains("sk-secret"), "逐项错误不得含 key 明文");
                assert!(message.contains("[REDACTED]"), "key 必须被替换：{message}");
            }
            other => panic!("期望逐项错误，实际：{other:?}"),
        }
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn c12_non_2xx_is_reported_once_without_retry() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
        let failing = status_mock(&mut server, 500, "boom", 1).await;
        let retry = status_mock(&mut server, 500, "boom", 0).await;

        let error = call(&target, &request(ImageSize::Auto), None)
            .await
            .expect_err("非 2xx 必须失败");
        assert_eq!(
            error,
            ImageClientError::HttpStatus {
                code: 500,
                body: "boom".to_string()
            }
        );
        failing.assert_async().await;
        retry.assert_async().await;
    }

    #[tokio::test]
    async fn c13_invalid_base_url_is_rejected_before_any_request() {
        let mut server = mockito::Server::new_async().await;
        let never = status_mock(&mut server, 200, OK_BODY, 0).await;
        let base_url = server.url().replace("http://", "ftp://");
        let target = new_target(ImageVendor::Gpt, &base_url, GPT_MODEL);

        let error = call(&target, &request(ImageSize::Auto), None)
            .await
            .expect_err("非法 base_url 必须失败");
        assert!(matches!(error, ImageClientError::InvalidConfig(_)));
        never.assert_async().await;
    }

    #[tokio::test]
    async fn c14_redirect_is_blocked() {
        let mut server = mockito::Server::new_async().await;
        let target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
        let redirect = server
            .mock("POST", "/v1/images/generations")
            .with_status(302)
            .with_header("location", "/redirect-target")
            .expect(1)
            .create_async()
            .await;

        let error = call(&target, &request(ImageSize::Auto), None).await;
        assert_eq!(error, Err(ImageClientError::RedirectBlocked));
        redirect.assert_async().await;
    }

    #[test]
    fn c15_non_test_code_contains_no_model_id_literal() {
        let source = include_str!("image_client.rs");
        let (non_test, _) = source.split_once("#[cfg(test)]").expect("必须有测试模块");
        for literal in ["gpt-image", "MiniMax-H3"] {
            assert!(
                !non_test.contains(literal),
                "非测试代码不得含模型 id：{literal}"
            );
        }
    }

    #[test]
    fn c16_debug_output_never_contains_the_api_key() {
        let mut target = new_target(ImageVendor::Gpt, "https://api.example.com", "some-model");
        target.api_key = "sk-secret".to_string();
        let debug = format!("{target:?}");
        assert!(
            !debug.contains("sk-secret"),
            "Debug 不得含 key 明文：{debug}"
        );
        assert!(
            debug.contains("<redacted>"),
            "Debug 必须显示脱敏占位：{debug}"
        );
    }
}
