use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use cadence_aria::web::static_assets::{serve_static, static_dist_service};
use tower::util::ServiceExt;

fn static_router() -> Router {
    let service = static_dist_service();
    Router::new().fallback(move |req: Request| {
        let service = service.clone();
        async move { serve_static(service, req).await }
    })
}

async fn html_get(app: Router, uri: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

/// 单一串行测试，避免 `ARIA_WEB_DIST` 进程级环境变量在并发用例间互相污染。
#[tokio::test]
async fn static_assets_embedded_and_override_behavior() {
    // 1) 嵌入：根路径返回 index.html(html)
    unsafe { std::env::remove_var("ARIA_WEB_DIST") };
    let resp = html_get(static_router(), "/").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );

    // 2) 嵌入：未知路径 SPA fallback 到 html
    let resp = html_get(static_router(), "/some/spa/route").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );

    // 3) 嵌入：显式 /index.html 命中
    let resp = html_get(static_router(), "/index.html").await;
    assert_eq!(resp.status(), StatusCode::OK);

    // 4) 逃生口：ARIA_WEB_DIST 指向磁盘目录时改读磁盘
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>disk-override</html>").unwrap();
    unsafe { std::env::set_var("ARIA_WEB_DIST", dir.path()) };
    let resp = html_get(static_router(), "/").await;
    unsafe { std::env::remove_var("ARIA_WEB_DIST") };
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("disk-override"));
}
