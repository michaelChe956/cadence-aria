//! image_client.rs 的外置测试模块：经 image_client.rs 内 `#[path = "image_client_tests.rs"]` 引入，
//! `use super::*` 语义与原内嵌 `mod tests` 完全一致（纯移动，逐字外迁）。

use super::*;
use mockito::Matcher;
use serde_json::json;

pub(super) const GPT_MODEL: &str = "gpt-image-2.6-flare";
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
    pattern.push_str(&format!("{BOUNDARY}\\r\\n{FILE_PART}{BOUNDARY}--\\r\\n$"));
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

/// 断言 edits 的 multipart 字段集合恰好等于给定列表，且不得出现给定字段。
async fn assert_multipart_fields(
    server: &mut mockito::Server,
    target: &ImageCallTarget,
    request: &ImageGenRequest,
    fields: &[(&str, &str)],
    forbidden: &[&str],
) {
    let edit = edits(server, multipart_matcher(fields)).await;
    let mut guards = Vec::new();
    for field in forbidden {
        guards.push(forbid(server, field).await);
    }
    let out = call(target, request, Some(reference()))
        .await
        .expect("edits 必须成功");
    assert_eq!(out.b64, "EDIT");
    edit.assert_async().await;
    for guard in guards {
        guard.assert_async().await;
    }
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
    assert_multipart_fields(
        &mut server,
        &target,
        &request(ImageSize::Landscape),
        &[
            ("model", GPT_MODEL),
            ("prompt", "draw a fox"),
            ("output_format", "png"),
            ("size", "1536x1024"),
            ("quality", "high"),
            ("background", "transparent"),
        ],
        &["input_fidelity", "response_format", "area"],
    )
    .await;
}

#[tokio::test]
async fn c05_minimax_edits_never_sends_quality_background_or_both_size_and_area() {
    // 具体尺寸：发 size，不发 area。
    let mut server = mockito::Server::new_async().await;
    let size_target = new_target(ImageVendor::Minimax, &server.url(), MINIMAX_MODEL);
    assert_multipart_fields(
        &mut server,
        &size_target,
        &request(ImageSize::Landscape),
        &[
            ("model", MINIMAX_MODEL),
            ("prompt", "draw a fox"),
            ("output_format", "png"),
            ("size", "1536x1024"),
            ("response_format", "b64_json"),
        ],
        &["quality", "background", "input_fidelity", "area"],
    )
    .await;

    // auto：发 area，不发 size。
    let mut server = mockito::Server::new_async().await;
    let auto_target = new_target(ImageVendor::Minimax, &server.url(), MINIMAX_MODEL);
    assert_multipart_fields(
        &mut server,
        &auto_target,
        &request(ImageSize::Auto),
        &[
            ("model", MINIMAX_MODEL),
            ("prompt", "draw a fox"),
            ("output_format", "png"),
            ("response_format", "b64_json"),
            ("area", "1K"),
        ],
        &["size"],
    )
    .await;
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
            assert!(
                !message.contains("sk-secret") && message.contains("[REDACTED]"),
                "逐项错误必须脱敏：{message}"
            );
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
        !debug.contains("sk-secret") && debug.contains("<redacted>"),
        "Debug 必须脱敏：{debug}"
    );
}

#[tokio::test]
async fn c17_huge_error_body_is_read_with_a_cap_and_truncated_to_500_chars() {
    // H6：恶意/被劫持端点可用超大错误体诱导大内存分配。错误体必须流式读取并设上限，
    // 错误文本仍按既有规则截断到 ≤500 字符，请求恰好发生 1 次（不重试）。
    let mut server = mockito::Server::new_async().await;
    let target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
    let huge = "x".repeat(16 * 1024 * 1024);
    let mock = status_mock(&mut server, 500, &huge, 1).await;
    let retry = status_mock(&mut server, 500, &huge, 0).await;

    let error = call(&target, &request(ImageSize::Auto), None)
        .await
        .expect_err("超大错误体必须失败");
    match error {
        ImageClientError::HttpStatus { code, body } => {
            assert_eq!(code, 500);
            assert!(
                body.chars().count() <= 500,
                "错误文本必须截断到 ≤500 字符，实际 {} 字符",
                body.chars().count()
            );
            assert!(body.chars().all(|c| c == 'x'), "内容仍是错误体前缀：{body}");
        }
        other => panic!("期望 HttpStatus，实际：{other:?}"),
    }
    mock.assert_async().await;
    retry.assert_async().await;
}

#[tokio::test]
async fn c18_success_body_over_128mib_is_rejected_with_a_readable_error() {
    // H6：2xx 成功体同样设上限（128 MiB，兼容 -4k 档超大合法图）。超限时不得
    // 整读后报笼统的网络错误，必须是明确的大小上限错误（可读、面向用户）。
    let mut server = mockito::Server::new_async().await;
    let target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
    let oversized = "x".repeat(128 * 1024 * 1024 + 1);
    let mock = status_mock(&mut server, 200, &oversized, 1).await;

    let error = call(&target, &request(ImageSize::Auto), None)
        .await
        .expect_err("超限成功体必须失败");
    assert_eq!(
        error,
        ImageClientError::OversizedBody,
        "超限应报大小上限错误，而非笼统 Network"
    );
    mock.assert_async().await;
}

#[tokio::test]
async fn c18b_success_body_over_old_64mib_cap_is_accepted() {
    // -4k 档（4096²）合法大图：b64 膨胀 4/3 后可越过旧的 64 MiB 上限，
    // 上限提升到 128 MiB 后必须放行并正常解码。
    let mut server = mockito::Server::new_async().await;
    let target = new_target(ImageVendor::Gpt, &server.url(), GPT_MODEL);
    let big_b64 = "A".repeat(64 * 1024 * 1024);
    let body = format!(r#"{{"data":[{{"b64_json":"{big_b64}"}}]}}"#);
    let mock = status_mock(&mut server, 200, &body, 1).await;

    let outcome = call(&target, &request(ImageSize::Auto), None)
        .await
        .expect("旧的 64 MiB 边界现在必须被接受");
    assert_eq!(
        outcome.b64.len(),
        64 * 1024 * 1024,
        "超旧上限的合法 b64 必须完整解码"
    );
    mock.assert_async().await;
}

// ---- 连通性探测（probe_connectivity）：两步出站，绝不生成图片、绝不计费 ----

/// /v1/models mock：固定状态码与响应体，校验 Bearer 鉴权头。
pub(super) async fn models_endpoint(
    server: &mut mockito::Server,
    status: usize,
    body: &str,
    hits: usize,
) -> mockito::Mock {
    server
        .mock("GET", "/v1/models")
        .match_header("authorization", "Bearer sk-test")
        .with_status(status)
        .with_body(body)
        .expect(hits)
        .create_async()
        .await
}

/// generations mock：body 必须精确等于 `{"model":…}`（缺 prompt 属预期），校验 Bearer。
pub(super) async fn probe_generations(
    server: &mut mockito::Server,
    model: &str,
    status: usize,
    body: &str,
    hits: usize,
) -> mockito::Mock {
    server
        .mock("POST", "/v1/images/generations")
        .match_header("authorization", "Bearer sk-test")
        .match_body(Matcher::Json(json!({ "model": model })))
        .with_status(status)
        .with_body(body)
        .expect(hits)
        .create_async()
        .await
}

pub(super) async fn probe(
    base_url: &str,
    models: &[&str],
) -> Result<ProbeReport, ImageClientError> {
    let models = models.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    probe_connectivity(ImageVendor::Gpt, base_url, "sk-test", &models).await
}

#[tokio::test]
async fn p01_both_steps_pass_and_missing_models_are_computed() {
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(
        &mut server,
        200,
        r#"{"data":[{"id":"gpt-image-2.6-flare"},{"id":"other"}]}"#,
        1,
    )
    .await;
    let generations = probe_generations(&mut server, GPT_MODEL, 400, "missing prompt", 1).await;

    let report = probe(&server.url(), &[GPT_MODEL, "ghost-model"])
        .await
        .expect("两步通过时探测必须成功");

    assert!(report.ok, "两步均通过 → ok=true");
    assert_eq!(report.message, "连通性正常");
    assert_eq!(report.missing_models, vec!["ghost-model".to_string()]);
    assert_eq!(report.steps.len(), 2);
    assert_eq!(
        (report.steps[0].key, report.steps[0].status),
        ("models", "ok")
    );
    assert_eq!(report.steps[0].label, "模型列表 /v1/models");
    assert_eq!(report.steps[0].detail, "网关返回 2 个模型");
    assert_eq!(
        (report.steps[1].key, report.steps[1].status),
        ("generations", "ok")
    );
    assert_eq!(report.steps[1].label, "图片端点 /v1/images/generations");
    assert_eq!(
        report.steps[1].detail,
        "图片端点连通且鉴权通过（缺少 prompt 属预期，不会计费）"
    );
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p02_models_list_accepts_bare_array_shape() {
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 200, r#"["alpha","beta"]"#, 1).await;
    let generations = probe_generations(&mut server, "alpha", 400, "missing prompt", 1).await;

    let report = probe(&server.url(), &["alpha", "nope"])
        .await
        .expect("数组形态必须可解析");

    assert_eq!(report.steps[0].detail, "网关返回 2 个模型");
    assert_eq!(report.missing_models, vec!["nope".to_string()]);
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p03_models_401_fails_and_skips_step_two() {
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 401, "unauthorized", 1).await;
    let generations = probe_generations(&mut server, GPT_MODEL, 400, "missing prompt", 0).await;

    let report = probe(&server.url(), &[GPT_MODEL])
        .await
        .expect("探测本身返回报告");

    assert!(!report.ok);
    assert_eq!(report.steps.len(), 1, "key 无效时不再发第二步");
    assert_eq!(report.steps[0].status, "failed");
    assert_eq!(report.steps[0].detail, "API Key 无效或无权限");
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p04_models_unavailable_is_skipped_and_probe_still_passes() {
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 404, "no such route", 1).await;
    let generations = probe_generations(&mut server, GPT_MODEL, 400, "missing prompt", 1).await;

    let models_list = vec![GPT_MODEL.to_string()];
    let report = probe_connectivity(ImageVendor::Minimax, &server.url(), "sk-test", &models_list)
        .await
        .expect("探测本身返回报告");

    assert_eq!(report.vendor, ImageVendor::Minimax);
    assert!(report.ok, "skipped 不算失败，图片端点通过即连通");
    assert_eq!(
        (report.steps[0].key, report.steps[0].status),
        ("models", "skipped")
    );
    assert_eq!(
        report.steps[0].detail,
        "该网关未提供 /v1/models，改用图片端点验证"
    );
    assert!(
        report.missing_models.is_empty(),
        "/v1/models 不可用时 missing_models 为空"
    );
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p05_models_network_error_is_skipped_and_step_two_fails() {
    // 绑定后立即释放端口：出站必然连接被拒（网络错误路径）。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let base_url = format!("http://127.0.0.1:{port}");

    let report = probe(&base_url, &[GPT_MODEL])
        .await
        .expect("探测本身返回报告");

    assert!(!report.ok);
    assert_eq!(
        (report.steps[0].key, report.steps[0].status),
        ("models", "skipped")
    );
    assert_eq!(report.steps[1].status, "failed");
    assert!(
        report.steps[1].detail.contains("网络错误"),
        "{}",
        report.steps[1].detail
    );
}

#[tokio::test]
async fn p06_generations_401_fails_with_invalid_key_detail() {
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(
        &mut server,
        200,
        r#"{"data":[{"id":"gpt-image-2.6-flare"}]}"#,
        1,
    )
    .await;
    let generations = probe_generations(&mut server, GPT_MODEL, 401, "bad key", 1).await;

    let report = probe(&server.url(), &[GPT_MODEL])
        .await
        .expect("探测本身返回报告");

    assert!(!report.ok);
    assert_eq!(report.steps[1].status, "failed");
    assert_eq!(report.steps[1].detail, "API Key 无效或无权限");
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p07_generations_404_with_models_hints_base_url_path() {
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 404, "no route", 1).await;
    let generations = probe_generations(&mut server, GPT_MODEL, 404, "not found", 1).await;

    let report = probe(&server.url(), &[GPT_MODEL])
        .await
        .expect("探测本身返回报告");

    assert!(!report.ok);
    assert_eq!(
        report.steps[1].detail,
        "路径不对：系统会自动拼 /v1/images/generations，请只填到主机根路径（不要带 /v1 或 /v1/images）"
    );
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p08_generations_404_without_models_distinguishes_missing_probe_model() {
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 404, "no route", 1).await;
    // models 为空 → 探测用通用占位模型 id。
    let generations = probe_generations(&mut server, "gpt-image-1", 404, "not found", 1).await;

    let report = probe(&server.url(), &[]).await.expect("探测本身返回报告");

    assert!(!report.ok);
    assert!(
        report.steps[1].detail.contains("未指定模型 id"),
        "404 且未指定模型时不得断言路径不对：{}",
        report.steps[1].detail
    );
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p09_other_status_detail_is_sanitized_and_report_hides_the_key() {
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 404, "no route", 1).await;
    let generations = probe_generations(
        &mut server,
        GPT_MODEL,
        500,
        r#"{"error":"key sk-test rejected"}"#,
        1,
    )
    .await;

    let report = probe(&server.url(), &[GPT_MODEL])
        .await
        .expect("探测本身返回报告");

    assert!(!report.ok);
    assert!(
        report.steps[1].detail.starts_with("网关返回 HTTP 500："),
        "{}",
        report.steps[1].detail
    );
    let report_text = serde_json::to_string(&report).expect("报告必须可序列化");
    assert!(
        !report_text.contains("sk-test"),
        "报告任何字段都不得含 key 明文：{report_text}"
    );
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p10_invalid_base_url_is_rejected_with_zero_requests() {
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(&mut server, 200, r#"{"data":[]}"#, 0).await;
    let generations = probe_generations(&mut server, GPT_MODEL, 400, "missing prompt", 0).await;
    let base_url = server.url().replace("http://", "ftp://");

    let error = probe(&base_url, &[GPT_MODEL])
        .await
        .expect_err("非法 base_url 必须在出站前拒绝");
    assert!(matches!(error, ImageClientError::InvalidConfig(_)));
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p11_available_models_lists_all_gateway_ids_in_gateway_order() {
    // /v1/models 成功 → available_models = 全量 id（保序）；missing_models 语义不变。
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(
        &mut server,
        200,
        r#"{"data":[{"id":"gpt-image-2.6-flare"},{"id":"other"},{"id":"extra"}]}"#,
        1,
    )
    .await;
    let generations = probe_generations(&mut server, GPT_MODEL, 400, "missing prompt", 1).await;

    let report = probe(&server.url(), &[GPT_MODEL, "ghost-model"])
        .await
        .expect("探测本身返回报告");

    assert_eq!(
        report.available_models,
        ["gpt-image-2.6-flare", "other", "extra"]
    );
    assert_eq!(report.missing_models, vec!["ghost-model".to_string()]);
    models.assert_async().await;
    generations.assert_async().await;
}

#[tokio::test]
async fn p12_available_models_is_empty_when_models_step_not_ok() {
    // skipped（网关未提供 /v1/models）与 failed（key 无效）都不携带网关清单 → 空数组。
    let mut server = mockito::Server::new_async().await;
    let skipped = models_endpoint(&mut server, 404, "no route", 1).await;
    let generations = probe_generations(&mut server, GPT_MODEL, 400, "missing prompt", 1).await;
    let report = probe(&server.url(), &[GPT_MODEL])
        .await
        .expect("探测本身返回报告");
    assert_eq!(report.steps[0].status, "skipped");
    assert!(report.available_models.is_empty(), "skipped → 空数组");
    skipped.assert_async().await;
    generations.assert_async().await;
    let mut server = mockito::Server::new_async().await;
    let failed = models_endpoint(&mut server, 401, "unauthorized", 1).await;
    let report = probe(&server.url(), &[GPT_MODEL])
        .await
        .expect("探测本身返回报告");
    assert_eq!(report.steps[0].status, "failed");
    assert!(report.available_models.is_empty(), "failed → 空数组");
    failed.assert_async().await;
}

#[tokio::test]
async fn p13_available_models_dedups_repeats_and_keeps_existing_fields_untouched() {
    // 重复 id 去重、保持首次出现顺序；steps detail 计数与 missing_models 不受去重影响。
    let mut server = mockito::Server::new_async().await;
    let models = models_endpoint(
        &mut server,
        200,
        r#"{"data":[{"id":"a"},{"id":"b"},{"id":"a"},{"id":"c"},{"id":"b"}]}"#,
        1,
    )
    .await;
    let generations = probe_generations(&mut server, "a", 400, "missing prompt", 1).await;

    let report = probe(&server.url(), &["a", "z"])
        .await
        .expect("探测本身返回报告");

    assert_eq!(report.available_models, ["a", "b", "c"]);
    assert_eq!(report.steps[0].detail, "网关返回 5 个模型");
    assert_eq!(report.missing_models, vec!["z".to_string()]);
    models.assert_async().await;
    generations.assert_async().await;
}
