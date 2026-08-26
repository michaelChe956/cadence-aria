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

#[tokio::test]
async fn session_response_strips_legacy_inline_b64() {
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
    let path = paths.image_create_session_file(&session.id);
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
                .uri(format!("/api/image-create/sessions/{}", session.id))
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

#[tokio::test]
async fn image_endpoint_serves_owned_image_with_cache_headers() {
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
    let image_id = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
    let image_bytes = b"stored-image";
    store
        .append_generation_result(
            &session.id,
            GenerationResult {
                prompt: "prompt".to_string(),
                params: Default::default(),
                media_type: "image/png".to_string(),
                image_id: Some(image_id.to_string()),
                b64: None,
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
                    session.id
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

#[tokio::test]
async fn image_endpoint_rejects_foreign_and_invalid_ids() {
    let root = tempdir().expect("root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let store = SessionStore::new(paths);
    let session_a = store
        .create(
            serde_json::from_value(json!({
                "template": {"preset": "ppt_business_illustration", "custom": null},
                "provider_name": "fake"
            }))
            .expect("create request"),
        )
        .await
        .expect("session a");
    let session_b = store
        .create(
            serde_json::from_value(json!({
                "template": {"preset": "ppt_business_illustration", "custom": null},
                "provider_name": "fake"
            }))
            .expect("create request"),
        )
        .await
        .expect("session b");
    let foreign_image_id = "1f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
    store
        .append_generation_result(
            &session_b.id,
            GenerationResult {
                prompt: "prompt".to_string(),
                params: Default::default(),
                media_type: "image/png".to_string(),
                image_id: Some(foreign_image_id.to_string()),
                b64: None,
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
                    session_a.id
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
                    session_b.id
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
                    session_a.id
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
                    session_a.id
                ))
                .body(Body::empty())
                .expect("missing request"),
        )
        .await
        .expect("missing response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn image_endpoint_serves_legacy_b64_without_file() {
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
    let image_id = "3f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
    store
        .append_generation_result(
            &session.id,
            GenerationResult {
                prompt: "legacy prompt".to_string(),
                params: Default::default(),
                media_type: "image/png".to_string(),
                image_id: Some(image_id.to_string()),
                b64: Some("bGVnYWN5LWltYWdl".to_string()),
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
                    session.id
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
