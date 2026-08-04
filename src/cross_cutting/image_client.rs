use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};

use crate::product::image_create::models::{
    ImageBackground, ImageCreateSettings, ImageOutputFormat, ImageQuality, ImageSize, InputFidelity,
};

const IMAGE_MODEL: &str = "gpt-image-2";
const MAX_ERROR_BODY_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageGenRequest {
    pub prompt: String,
    pub size: ImageSize,
    pub quality: ImageQuality,
    pub background: ImageBackground,
    pub output_format: ImageOutputFormat,
    pub input_fidelity: Option<InputFidelity>,
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
}

#[async_trait]
pub trait ImageClientApi: Send + Sync {
    async fn generate(
        &self,
        settings: &ImageCreateSettings,
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
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(600))
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
        settings: &ImageCreateSettings,
        req: &ImageGenRequest,
        reference: Option<ImageRefImage>,
    ) -> Result<ImageGenOutcome, ImageClientError> {
        validate_settings(settings)?;

        let endpoint = if reference.is_some() {
            "edits"
        } else {
            "generations"
        };
        let url = format!(
            "{}/v1/images/{endpoint}",
            settings.base_url.trim_end_matches('/')
        );
        let url_for_log = url.clone();

        let request = if let Some(reference) = reference {
            let extension = reference
                .declared_mime
                .split('/')
                .nth(1)
                .filter(|ext| matches!(*ext, "png" | "jpeg" | "webp"))
                .unwrap_or("png");
            let file_name = format!("reference.{extension}");
            let image = Part::bytes(reference.bytes)
                .mime_str(&reference.declared_mime)
                .map_err(|_| {
                    ImageClientError::InvalidConfig(
                        "reference image has an invalid declared MIME type".to_string(),
                    )
                })?
                .file_name(file_name);
            let form = Form::new()
                .text("model", IMAGE_MODEL)
                .text("prompt", req.prompt.clone())
                .text("size", wire_value(&req.size)?)
                .text("quality", wire_value(&req.quality)?)
                .text("background", wire_value(&req.background)?)
                .text("output_format", wire_value(&req.output_format)?)
                .part("image[]", image);
            let form = match req.input_fidelity {
                Some(value) => form.text("input_fidelity", wire_value(&value)?),
                None => form,
            };
            self.http.post(url).multipart(form)
        } else {
            self.http.post(url).json(&GenerationBody {
                model: IMAGE_MODEL,
                prompt: &req.prompt,
                size: req.size,
                quality: req.quality,
                background: req.background,
                output_format: req.output_format,
            })
        };

        let has_reference = endpoint == "edits";
        let started = std::time::Instant::now();
        eprintln!(
            "[image-create] POST {url_for_log} (endpoint={endpoint}, has_reference={has_reference}, size={:?}, quality={:?}) starting",
            req.size, req.quality
        );
        let response = request
            .bearer_auth(&settings.api_key)
            .send()
            .await
            .map_err(|error| {
                eprintln!(
                    "[image-create] POST {url_for_log} FAILED after {:?}: {error}; debug={error:?}; is_timeout={}; is_connect={}; is_request={}; is_body={}",
                    started.elapsed(),
                    error.is_timeout(),
                    error.is_connect(),
                    error.is_request(),
                    error.is_body()
                );
                let mut source = std::error::Error::source(&error);
                let mut depth = 0;
                while let Some(cause) = source {
                    eprintln!(
                        "[image-create] POST {url_for_log} error source[{depth}]: {cause}; debug={cause:?}"
                    );
                    source = cause.source();
                    depth += 1;
                }
                normalize_reqwest_error(error)
            })?;
        eprintln!(
            "[image-create] POST {url_for_log} responded {} in {:?}",
            response.status(),
            started.elapsed()
        );

        if response.status().is_redirection() {
            return Err(ImageClientError::RedirectBlocked);
        }
        if !response.status().is_success() {
            let code = response.status().as_u16();
            let body = response.text().await.map_err(normalize_reqwest_error)?;
            return Err(ImageClientError::HttpStatus {
                code,
                body: sanitize_error_body(&body, &settings.api_key),
            });
        }

        let payload = response
            .json::<ImageResponse>()
            .await
            .map_err(normalize_reqwest_error)?;
        let first = payload
            .data
            .into_iter()
            .next()
            .ok_or(ImageClientError::EmptyData)?;
        let b64 = first.b64_json.ok_or(ImageClientError::MissingB64)?;

        Ok(ImageGenOutcome {
            media_type: media_type(req.output_format).to_string(),
            b64,
        })
    }
}

#[derive(Serialize)]
struct GenerationBody<'a> {
    model: &'static str,
    prompt: &'a str,
    size: ImageSize,
    quality: ImageQuality,
    background: ImageBackground,
    output_format: ImageOutputFormat,
}

#[derive(Deserialize)]
struct ImageResponse {
    data: Vec<ImageResponseData>,
}

#[derive(Deserialize)]
struct ImageResponseData {
    b64_json: Option<String>,
}

fn validate_settings(settings: &ImageCreateSettings) -> Result<(), ImageClientError> {
    if settings.base_url.trim().is_empty() || settings.api_key.trim().is_empty() {
        return Err(ImageClientError::MissingConfig);
    }
    validate_base_url(&settings.base_url)
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
    use crate::product::image_create::models::DefaultParams;
    use mockito::Matcher;
    use serde_json::json;

    fn empty_settings() -> ImageCreateSettings {
        ImageCreateSettings::default()
    }

    fn req(output_format: ImageOutputFormat) -> ImageGenRequest {
        ImageGenRequest {
            prompt: "draw a fox".to_string(),
            size: ImageSize::Landscape,
            quality: ImageQuality::High,
            background: ImageBackground::Transparent,
            output_format,
            input_fidelity: Some(InputFidelity::High),
        }
    }

    #[tokio::test]
    async fn text_only_hits_generations_and_maps_media_type() {
        let mut server = mockito::Server::new_async().await;
        let mut settings = empty_settings();
        settings.base_url = server.url();
        settings.api_key = "sk-test".into();
        let mock = server
            .mock("POST", "/v1/images/generations")
            .match_header("authorization", "Bearer sk-test")
            .match_body(Matcher::Json(json!({
                "model": "gpt-image-2",
                "prompt": "draw a fox",
                "size": "1536x1024",
                "quality": "high",
                "background": "transparent",
                "output_format": "png"
            })))
            .with_status(200)
            .with_body(r#"{"data":[{"b64_json":"AAAA"}]}"#)
            .expect(1)
            .create_async()
            .await;

        let out = ImageClient::new()
            .generate(&settings, &req(ImageOutputFormat::Png), None)
            .await
            .unwrap();

        assert_eq!(out.media_type, "image/png");
        assert_eq!(out.b64, "AAAA");
        mock.assert_async().await;

        let webp = server
            .mock("POST", "/v1/images/generations")
            .with_status(200)
            .with_body(r#"{"data":[{"b64_json":"BBBB"}]}"#)
            .expect(1)
            .create_async()
            .await;
        let out = ImageClient::new()
            .generate(&settings, &req(ImageOutputFormat::Webp), None)
            .await
            .unwrap();
        assert_eq!(out.media_type, "image/webp");
        webp.assert_async().await;
    }

    #[tokio::test]
    async fn edits_and_fidelity_use_multipart_while_generation_omits_fidelity() {
        let mut server = mockito::Server::new_async().await;
        let settings = ImageCreateSettings {
            base_url: server.url(),
            api_key: "sk-test".into(),
            defaults: DefaultParams::default(),
        };
        let edit = server
            .mock("POST", "/v1/images/edits")
            .match_header("authorization", "Bearer sk-test")
            .match_header(
                "content-type",
                Matcher::Regex("multipart/form-data; boundary=".into()),
            )
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex(r#"name="model"\r\n\r\ngpt-image-2"#.into()),
                Matcher::Regex(r#"name="image\[\]""#.into()),
                Matcher::Regex("Content-Type: image/png".into()),
                Matcher::Regex(r#"name="prompt"\r\n\r\ndraw a fox"#.into()),
                Matcher::Regex(r#"name="size"\r\n\r\n1536x1024"#.into()),
                Matcher::Regex(r#"name="quality"\r\n\r\nhigh"#.into()),
                Matcher::Regex(r#"name="background"\r\n\r\ntransparent"#.into()),
                Matcher::Regex(r#"name="output_format"\r\n\r\nwebp"#.into()),
                Matcher::Regex(r#"name="input_fidelity"\r\n\r\nhigh"#.into()),
            ]))
            .with_status(200)
            .with_body(r#"{"data":[{"b64_json":"EDIT"}]}"#)
            .expect(1)
            .create_async()
            .await;

        let out = ImageClient::new()
            .generate(
                &settings,
                &req(ImageOutputFormat::Webp),
                Some(ImageRefImage {
                    bytes: b"reference".to_vec(),
                    declared_mime: "image/png".to_string(),
                }),
            )
            .await
            .unwrap();
        assert_eq!(out.media_type, "image/webp");
        edit.assert_async().await;
    }

    #[tokio::test]
    async fn safety_and_errors_are_normalized_without_retries() {
        let client = ImageClient::new();
        let unsafe_settings = ImageCreateSettings {
            base_url: "http://api.example.com".to_string(),
            api_key: "sk-secret".to_string(),
            defaults: DefaultParams::default(),
        };
        assert!(matches!(
            client
                .generate(&unsafe_settings, &req(ImageOutputFormat::Png), None)
                .await,
            Err(ImageClientError::InvalidConfig(_))
        ));

        let missing = ImageCreateSettings::default();
        assert_eq!(
            client
                .generate(&missing, &req(ImageOutputFormat::Png), None)
                .await,
            Err(ImageClientError::MissingConfig)
        );

        let mut server = mockito::Server::new_async().await;
        let settings = ImageCreateSettings {
            base_url: server.url(),
            api_key: "sk-test".into(),
            defaults: DefaultParams::default(),
        };

        let redirect = server
            .mock("POST", "/v1/images/generations")
            .with_status(302)
            .with_header("location", "/redirect-target")
            .expect(1)
            .create_async()
            .await;
        assert_eq!(
            client
                .generate(&settings, &req(ImageOutputFormat::Png), None)
                .await,
            Err(ImageClientError::RedirectBlocked)
        );
        redirect.assert_async().await;

        let rate_limited = server
            .mock("POST", "/v1/images/generations")
            .with_status(429)
            .with_body("slow down")
            .expect(1)
            .create_async()
            .await;
        assert_eq!(
            client
                .generate(&settings, &req(ImageOutputFormat::Png), None)
                .await,
            Err(ImageClientError::HttpStatus {
                code: 429,
                body: "slow down".to_string()
            })
        );
        rate_limited.assert_async().await;

        let echoed_secret = server
            .mock("POST", "/v1/images/generations")
            .with_status(500)
            .with_body(format!(
                "upstream echoed Authorization: Bearer {} -- {}",
                settings.api_key,
                "x".repeat(600)
            ))
            .expect(1)
            .create_async()
            .await;
        let error = client
            .generate(&settings, &req(ImageOutputFormat::Png), None)
            .await
            .expect_err("echoed secret response must fail");
        let ImageClientError::HttpStatus { code, body } = error else {
            panic!("expected HTTP status error");
        };
        assert_eq!(code, 500);
        assert!(!body.contains(&settings.api_key));
        assert!(body.contains("Bearer [REDACTED]"));
        assert!(body.chars().count() <= 500);
        echoed_secret.assert_async().await;

        let empty = server
            .mock("POST", "/v1/images/generations")
            .with_status(200)
            .with_body(r#"{"data":[]}"#)
            .expect(1)
            .create_async()
            .await;
        assert_eq!(
            client
                .generate(&settings, &req(ImageOutputFormat::Png), None)
                .await,
            Err(ImageClientError::EmptyData)
        );
        empty.assert_async().await;

        let missing_b64 = server
            .mock("POST", "/v1/images/generations")
            .with_status(200)
            .with_body(r#"{"data":[{}]}"#)
            .expect(1)
            .create_async()
            .await;
        assert_eq!(
            client
                .generate(&settings, &req(ImageOutputFormat::Png), None)
                .await,
            Err(ImageClientError::MissingB64)
        );
        missing_b64.assert_async().await;
    }

    #[test]
    fn validates_https_and_all_loopback_forms() {
        assert!(validate_base_url("https://api.example.com").is_ok());
        assert!(validate_base_url("http://localhost:3000").is_ok());
        assert!(validate_base_url("http://127.42.0.1:3000").is_ok());
        assert!(validate_base_url("http://[::1]:3000").is_ok());
        assert!(validate_base_url("ftp://localhost").is_err());
        assert!(validate_base_url("http://192.168.1.1").is_err());
    }
}
