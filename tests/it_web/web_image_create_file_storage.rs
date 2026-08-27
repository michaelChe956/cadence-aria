//! 验收追踪：OpenSpec image-create-file-storage 7.1。
//! 覆盖旧版多图 b64 惰性迁移、图片读取缓存契约、新生成文件落盘及删除清理。

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use base64::{Engine as _, engine::general_purpose};
use cadence_aria::cross_cutting::aria_state_paths::AriaStatePaths;
use cadence_aria::cross_cutting::image_client::{
    ImageClientApi, ImageClientError, ImageGenOutcome, ImageGenRequest, ImageRefImage,
};
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::product::image_create::models::{ImageCreateSettings, SettingsStoreApi};
use cadence_aria::product::image_create::{
    ImageCreateEngine, ImageCreateRunRegistry, SessionStore, SettingsStore,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use chrono::Utc;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

const LEGACY_IMAGE_ONE: &[u8] = b"legacy-image-one";
const LEGACY_IMAGE_TWO: &[u8] = b"legacy-image-two";
const GENERATED_IMAGE: &[u8] = b"fake-gateway-image";

struct FakeImageGateway;

#[async_trait]
impl ImageClientApi for FakeImageGateway {
    async fn generate(
        &self,
        _settings: &ImageCreateSettings,
        request: &ImageGenRequest,
        reference: Option<ImageRefImage>,
    ) -> Result<ImageGenOutcome, ImageClientError> {
        assert_eq!(request.prompt, "create a file-backed image");
        assert!(reference.is_none());
        Ok(ImageGenOutcome {
            media_type: "image/png".to_string(),
            b64: general_purpose::STANDARD.encode(GENERATED_IMAGE),
        })
    }
}

async fn app_with_fake_gateway(root: &Path) -> axum::Router {
    let paths = AriaStatePaths::from_workspace_root(root);
    SettingsStore::new(paths.clone())
        .save(&ImageCreateSettings {
            base_url: "https://images.example.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            defaults: Default::default(),
        })
        .await
        .expect("configure image settings");

    let mut state = WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()));
    state.image_create_engine = Some(Arc::new(ImageCreateEngine::new(
        paths.clone(),
        Arc::new(SessionStore::new(paths.clone())),
        Arc::new(SettingsStore::new(paths)),
        Arc::new(FakeImageGateway),
        Arc::new(ProviderRegistry::new()),
        Arc::new(ImageCreateRunRegistry::default()),
    )));
    build_web_router(state)
}

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        serde_json::from_slice(&bytes).expect("response json"),
    )
}

fn generate_multipart(boundary: &str) -> Vec<u8> {
    format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"prompt\"\r\n\r\ncreate a file-backed image\r\n--{boundary}--\r\n"
    )
    .into_bytes()
}

async fn get_image(
    app: axum::Router,
    session_id: &str,
    image_id: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .uri(format!(
                "/api/image-create/sessions/{session_id}/images/{image_id}"
            ))
            .body(Body::empty())
            .expect("image request"),
    )
    .await
    .expect("image response")
}

fn assert_image_response(
    response: axum::response::Response,
    image_id: &str,
    expected_bytes: &[u8],
) -> impl std::future::Future<Output = ()> {
    let image_id = image_id.to_string();
    async move {
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&header::HeaderValue::from_static("image/png"))
        );
        assert_eq!(
            response.headers().get(header::ETAG),
            Some(&header::HeaderValue::from_str(&format!("\"{image_id}\"")).expect("etag"))
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static(
                "private, max-age=31536000, immutable"
            ))
        );
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("image body"),
            expected_bytes
        );
    }
}

#[tokio::test]
async fn image_create_file_storage_migrates_multi_image_legacy_session_and_cleans_up() {
    let root = tempdir().expect("workspace root");
    let paths = AriaStatePaths::from_workspace_root(root.path());
    let app = app_with_fake_gateway(root.path()).await;

    let (status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/image-create/sessions",
        json!({
            "template": {"preset": "ppt_business_illustration", "custom": null},
            "provider_name": "fake"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = created["id"].as_str().expect("session id").to_string();

    let session_file = paths.image_create_session_file(&session_id);
    let mut legacy_record: Value = serde_json::from_slice(
        &tokio::fs::read(&session_file)
            .await
            .expect("read newly created session"),
    )
    .expect("session json");
    legacy_record["generation_results"] = json!([
        {
            "prompt": "legacy one",
            "params": {
                "size": "auto",
                "quality": "auto",
                "background": "auto",
                "output_format": "png"
            },
            "media_type": "image/png",
            "b64": general_purpose::STANDARD.encode(LEGACY_IMAGE_ONE),
            "ts": Utc::now().to_rfc3339()
        },
        {
            "prompt": "legacy two",
            "params": {
                "size": "auto",
                "quality": "auto",
                "background": "auto",
                "output_format": "png"
            },
            "media_type": "image/png",
            "b64": general_purpose::STANDARD.encode(LEGACY_IMAGE_TWO),
            "ts": Utc::now().to_rfc3339()
        }
    ]);
    tokio::fs::write(
        &session_file,
        serde_json::to_vec_pretty(&legacy_record).expect("serialize legacy session"),
    )
    .await
    .expect("inject legacy session");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/image-create/sessions/{session_id}"))
                .body(Body::empty())
                .expect("get legacy session"),
        )
        .await
        .expect("legacy session response");
    assert_eq!(response.status(), StatusCode::OK);
    let response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("legacy session body");
    let response_text = String::from_utf8(response_body.to_vec()).expect("utf8 session body");
    assert!(!response_text.contains("\"b64\""));
    let migrated: Value = serde_json::from_slice(&response_body).expect("migrated session json");
    let migrated_results = migrated["generation_results"]
        .as_array()
        .expect("migrated generation results");
    assert_eq!(migrated_results.len(), 2);
    let legacy_image_ids: Vec<String> = migrated_results
        .iter()
        .map(|result| {
            assert_eq!(result["legacy_pending"], false);
            let image_id = result["image_id"].as_str().expect("migrated image id");
            uuid::Uuid::parse_str(image_id).expect("image id is UUID");
            image_id.to_string()
        })
        .collect();

    assert_image_response(
        get_image(app.clone(), &session_id, &legacy_image_ids[0]).await,
        &legacy_image_ids[0],
        LEGACY_IMAGE_ONE,
    )
    .await;
    assert_image_response(
        get_image(app.clone(), &session_id, &legacy_image_ids[1]).await,
        &legacy_image_ids[1],
        LEGACY_IMAGE_TWO,
    )
    .await;

    let image_files: Vec<_> = std::fs::read_dir(paths.image_create_images_dir())
        .expect("legacy images directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("legacy image directory entries");
    assert_eq!(image_files.len(), 2);
    for image_id in &legacy_image_ids {
        assert!(
            paths
                .image_create_image_file(image_id, "image/png")
                .expect("legacy image path")
                .is_file()
        );
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/image-create/sessions/{session_id}"))
                .body(Body::empty())
                .expect("get migrated session again"),
        )
        .await
        .expect("migrated session response");
    let migrated_again: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("migrated session response body"),
    )
    .expect("migrated session response json");
    assert!(
        migrated_again["generation_results"]
            .as_array()
            .expect("migrated generation results")
            .iter()
            .all(|result| result["legacy_pending"] == false)
    );

    let boundary = "image-create-file-storage-boundary";
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/image-create/sessions/{session_id}/generate"))
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(generate_multipart(boundary)))
                .expect("generate request"),
        )
        .await
        .expect("generate response");
    assert_eq!(response.status(), StatusCode::OK);
    let generated_response = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("generate response body");
    let generated_response_text = String::from_utf8(generated_response.to_vec()).expect("utf8");
    assert!(!generated_response_text.contains("\"b64\""));
    let generated: Value = serde_json::from_slice(&generated_response).expect("generate json");
    let generated_image_id = generated["image_id"]
        .as_str()
        .expect("generated image id")
        .to_string();
    assert_eq!(generated["media_type"], "image/png");
    assert!(
        paths
            .image_create_image_file(&generated_image_id, "image/png")
            .expect("generated image path")
            .is_file()
    );
    assert_image_response(
        get_image(app.clone(), &session_id, &generated_image_id).await,
        &generated_image_id,
        GENERATED_IMAGE,
    )
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/image-create/sessions/{session_id}"))
                .body(Body::empty())
                .expect("get session after generation"),
        )
        .await
        .expect("session response after generation");
    let current_session_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("current session body");
    let current_session_text = String::from_utf8(current_session_body.to_vec()).expect("utf8");
    assert!(!current_session_text.contains("\"b64\""));
    let current_session: Value =
        serde_json::from_slice(&current_session_body).expect("session json");
    assert!(
        current_session["generation_results"]
            .as_array()
            .expect("generation results")
            .iter()
            .any(|result| {
                result["image_id"] == generated_image_id && result["legacy_pending"] == false
            })
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/api/image-create/sessions/{session_id}"))
                .body(Body::empty())
                .expect("delete session request"),
        )
        .await
        .expect("delete session response");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    for image_id in legacy_image_ids
        .iter()
        .chain(std::iter::once(&generated_image_id))
    {
        assert!(
            !paths
                .image_create_image_file(image_id, "image/png")
                .expect("image path")
                .exists()
        );
        assert_eq!(
            get_image(app.clone(), &session_id, image_id).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
