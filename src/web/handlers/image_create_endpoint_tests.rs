use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chrono::Utc;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::product::image_create::SessionStore;
use crate::product::image_create::models::{GenerationResult, SessionStoreApi};
use crate::web::app::build_web_router;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;

fn state(root: &std::path::Path) -> WebAppState {
    WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
}

/// 建一个会话，返回会话 id。
async fn create_session(paths: &AriaStatePaths) -> String {
    let store = SessionStore::new(paths.clone());
    store
        .create(
            serde_json::from_value(json!({
                "template": {"preset": "ppt_business_illustration", "custom": null},
                "provider_name": "fake"
            }))
            .expect("create request"),
        )
        .await
        .expect("session")
        .id
}

fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// 调用设置接口，返回状态码与响应体。
async fn settings_request(app: Router, method: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .oneshot(json_request(method, "/api/image-create/settings", body))
        .await
        .expect("settings response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn session_response_strips_legacy_inline_b64() {
    let root = tempdir().expect("root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let session_id = create_session(&paths).await;
    let path = paths.image_create_session_file(&session_id);
    let bytes = tokio::fs::read(&path).await.expect("read record");
    let mut record: Value = serde_json::from_slice(&bytes).expect("record json");
    record["generation_results"] = json!([{
        "prompt": "legacy prompt",
        "params": {
            "size": "auto",
            "quality": "auto",
            "background": "auto",
            "output_format": "png"
        },
        "media_type": "image/png",
        "b64": "not-base64",
        "ts": Utc::now().to_rfc3339()
    }]);
    tokio::fs::write(&path, serde_json::to_vec_pretty(&record).unwrap())
        .await
        .expect("inject legacy record");

    let app = build_web_router(state(root.path()));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/image-create/sessions/{session_id}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("get response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let body = String::from_utf8(body.to_vec()).expect("utf8 body");
    assert!(!body.contains("\"b64\""));
    assert!(body.contains("\"legacy_pending\":true"));
}

/// H-10：图片读取端点 —— 文件优先（read_result_image）。
#[tokio::test]
async fn image_endpoint_serves_owned_image_with_cache_headers() {
    let root = tempdir().expect("root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let store = SessionStore::new(paths.clone());
    let session_id = create_session(&paths).await;
    let image_id = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
    let image_bytes = b"stored-image";
    store
        .append_generation_result(
            &session_id,
            GenerationResult {
                prompt: "prompt".to_string(),
                params: Default::default(),
                media_type: "image/png".to_string(),
                image_id: Some(image_id.to_string()),
                b64: None,
                vendor: None,
                model: None,
                selection: None,
                reference_source: None,
                ts: Utc::now(),
            },
        )
        .await
        .expect("append result");
    let image_path = paths
        .image_create_image_file(image_id, "image/png")
        .expect("image path");
    tokio::fs::create_dir_all(image_path.parent().expect("image parent"))
        .await
        .expect("create image directory");
    tokio::fs::write(&image_path, image_bytes)
        .await
        .expect("write image");

    let response = build_web_router(state(root.path()))
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/image-create/sessions/{}/images/{image_id}",
                    session_id
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("image response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    assert_eq!(
        response.headers().get(header::ETAG).unwrap(),
        "\"0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0\""
    );
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "private, max-age=31536000, immutable"
    );
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body"),
        image_bytes.as_slice()
    );
}

/// H-10：图片读取端点 —— 缺失 404 与非法 id 400。
#[tokio::test]
async fn image_endpoint_rejects_foreign_and_invalid_ids() {
    let root = tempdir().expect("root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let store = SessionStore::new(paths.clone());
    let session_a_id = create_session(&paths).await;
    let session_b_id = create_session(&paths).await;
    let foreign_image_id = "1f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
    store
        .append_generation_result(
            &session_b_id,
            GenerationResult {
                prompt: "prompt".to_string(),
                params: Default::default(),
                media_type: "image/png".to_string(),
                image_id: Some(foreign_image_id.to_string()),
                b64: None,
                vendor: None,
                model: None,
                selection: None,
                reference_source: None,
                ts: Utc::now(),
            },
        )
        .await
        .expect("append foreign result");
    let app = build_web_router(state(root.path()));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/image-create/sessions/{}/images/{foreign_image_id}",
                    session_a_id
                ))
                .body(Body::empty())
                .expect("foreign request"),
        )
        .await
        .expect("foreign response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/image-create/sessions/{}/images/{foreign_image_id}",
                    session_b_id
                ))
                .body(Body::empty())
                .expect("missing file request"),
        )
        .await
        .expect("missing file response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/image-create/sessions/{}/images/not-a-uuid",
                    session_a_id
                ))
                .body(Body::empty())
                .expect("invalid request"),
        )
        .await
        .expect("invalid response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/image-create/sessions/{}/images/2f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0",
                    session_a_id
                ))
                .body(Body::empty())
                .expect("missing request"),
        )
        .await
        .expect("missing response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// H-10：图片读取端点 —— 内联 b64 回退。
#[tokio::test]
async fn image_endpoint_serves_legacy_b64_without_file() {
    let root = tempdir().expect("root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let store = SessionStore::new(paths.clone());
    let session_id = create_session(&paths).await;
    let image_id = "3f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
    store
        .append_generation_result(
            &session_id,
            GenerationResult {
                prompt: "legacy prompt".to_string(),
                params: Default::default(),
                media_type: "image/png".to_string(),
                image_id: Some(image_id.to_string()),
                b64: Some("bGVnYWN5LWltYWdl".to_string()),
                vendor: None,
                model: None,
                selection: None,
                reference_source: None,
                ts: Utc::now(),
            },
        )
        .await
        .expect("append legacy result");
    let images_dir = paths.image_create_images_dir();
    tokio::fs::create_dir_all(images_dir.parent().expect("images parent"))
        .await
        .expect("create images parent");
    tokio::fs::write(&images_dir, b"not-a-directory")
        .await
        .expect("block legacy migration write");

    let response = build_web_router(state(root.path()))
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/image-create/sessions/{}/images/{image_id}",
                    session_id
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("image response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body"),
        b"legacy-image".as_slice()
    );
}

/// H-05：设置 GET 的响应形态为 vendors.{gpt,minimax}，且不含任一家的 key 明文。
#[tokio::test]
async fn settings_get_returns_vendors_shape_without_plaintext_key() {
    let root = tempdir().expect("root");
    let app = build_web_router(state(root.path()));
    let (status, _) = settings_request(
        app.clone(),
        "PUT",
        json!({
            "vendors": {
                "gpt": {
                    "base_url": "https://images.example.com/v1",
                    "api_key_action": "replace",
                    "api_key": "sk-gpt-secret1234"
                },
                "minimax": {
                    "base_url": "https://api.minimax.chat/v1",
                    "api_key_action": "replace",
                    "api_key": "mm-minimax-secret"
                }
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, masked) = settings_request(app, "GET", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    let text = masked.to_string();
    assert!(
        !text.contains("sk-gpt-secret1234"),
        "响应泄露了 key：{text}"
    );
    assert!(
        !text.contains("mm-minimax-secret"),
        "响应泄露了 key：{text}"
    );
    assert_eq!(
        masked["vendors"]["gpt"]["base_url"],
        "https://images.example.com/v1"
    );
    assert_eq!(
        masked["vendors"]["minimax"]["base_url"],
        "https://api.minimax.chat/v1"
    );
    for vendor in ["gpt", "minimax"] {
        let masked_key = masked["vendors"][vendor]["api_key_masked"]
            .as_str()
            .expect("masked key");
        assert!(masked_key.contains("****"), "{vendor} 的 key 未脱敏");
        assert_eq!(masked["vendors"][vendor]["available"], true);
        assert!(
            !masked["vendors"][vendor]["recommended_models"]
                .as_array()
                .expect("recommended models")
                .is_empty()
        );
    }
}

/// H-06：设置 PUT 只带 vendors.minimax → GPT 的配置逐项未变。
#[tokio::test]
async fn settings_put_with_only_minimax_keeps_gpt_untouched() {
    let root = tempdir().expect("root");
    let app = build_web_router(state(root.path()));
    let (status, before) = settings_request(
        app.clone(),
        "PUT",
        json!({
            "vendors": {
                "gpt": {
                    "base_url": "https://images.example.com/v1",
                    "api_key_action": "replace",
                    "api_key": "sk-gpt-secret1234"
                },
                "minimax": {
                    "base_url": "https://api.minimax.chat/v1",
                    "api_key_action": "replace",
                    "api_key": "mm-minimax-secret"
                }
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = settings_request(
        app.clone(),
        "PUT",
        json!({
            "vendors": {
                "minimax": {
                    "base_url": "https://api.minimax.chat/v2",
                    "api_key_action": "replace",
                    "api_key": "mm-new-key-9988"
                }
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, masked) = settings_request(app, "GET", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(masked["vendors"]["gpt"], before["vendors"]["gpt"]);
    assert_eq!(
        masked["vendors"]["minimax"]["base_url"],
        "https://api.minimax.chat/v2"
    );
    assert_ne!(
        masked["vendors"]["minimax"]["api_key_masked"],
        before["vendors"]["minimax"]["api_key_masked"]
    );
}

/// H-07：设置 PUT 含跨家重复模型 id → 400，且设置文件未被改动。
#[tokio::test]
async fn settings_put_with_duplicate_model_across_vendors_is_rejected_without_persisting() {
    let root = tempdir().expect("root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let app = build_web_router(state(root.path()));
    let (status, _) = settings_request(
        app.clone(),
        "PUT",
        json!({
            "vendors": {
                "gpt": {
                    "base_url": "https://images.example.com/v1",
                    "api_key_action": "replace",
                    "api_key": "sk-gpt-secret1234"
                }
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let settings_file = paths.image_create_settings_file();
    let before = tokio::fs::read(&settings_file)
        .await
        .expect("settings file");

    let (status, body) = settings_request(
        app,
        "PUT",
        json!({
            "vendors": {
                "gpt": {"models": ["dup-model"], "default_model": "dup-model"},
                "minimax": {"models": ["dup-model"], "default_model": "dup-model"}
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .expect("error")
            .contains("同时出现在 GPT 与 MiniMax 清单中"),
        "错误文本：{}",
        body["error"]
    );
    assert_eq!(
        tokio::fs::read(&settings_file)
            .await
            .expect("settings file"),
        before
    );
}
