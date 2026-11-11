//! image_client.rs 的外置测试模块：经 image_client.rs 内 `#[path = "image_client_tests.rs"]` 引入，
//! `use super::*` 语义与原内嵌 `mod tests` 完全一致（纯移动，逐字外迁）。

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
