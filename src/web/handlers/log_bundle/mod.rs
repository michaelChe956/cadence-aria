//! 驾驶舱「日志打包」HTTP 面（工作包 3.1 / T4）。
//!
//! 两条端点（只此两条，不开放其它方法）：
//! - `POST /api/diagnostics/log-bundle/authorize`：网页发起一次导出申请，返回
//!   公开申请号与一次性下载句柄。**句柄本身不是批准**：宿主仍须在终端执行
//!   `aria diagnostics approve <申请号>`，经实例专用的本机 Unix socket 交互确认。
//!   本端点不读任何来源、不写凭证文件、不产生日志正文。
//! - `POST /api/diagnostics/log-bundle/download`：请求体严格为
//!   `{confirmed: bool, handle: String}`（未知字段一律拒绝，不能指定文件/目标）。
//!   仅当句柄已被本机批准、未过期、未重放、且绑定同一 Origin 时才采集，
//!   以 gzip 附件流式下发；浏览器自行决定保存位置。
//!
//! 来源门（两个端点一致）：`state.api_auth` 必须已装配；Origin 必须精确受信
//! （`auth.origin_allowed`）；外部来源必须是 HTTPS，HTTP 只放行明确的回环
//! 主机；Host authority 与 Origin 同源。不采信 `X-Forwarded-*`。
//!
//! 错误一律静态 JSON（`code`/`message`/`details`），不含路径、句柄或其它秘密；
//! 句柄不进日志、不进 URL、不进归档。

mod archive;
mod fs;
mod sessions;
mod trace;

pub(crate) mod permit;

use self::permit::Grants;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Extension, JsonRejection, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use futures_util::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::io::{ReaderStream, SyncIoBridge};

use crate::web::api_auth::parse_trusted_origin;
use crate::web::state::WebAppState;

/// 授权有效期（秒）：句柄 5 分钟内一次性核销。
const AUTHORIZATION_TTL_SECONDS: u64 = 300;
/// 下载请求体上限（字节）：只接受 `confirmed` + `handle` 两个小字段。
const DOWNLOAD_BODY_LIMIT_BYTES: usize = 1024;
/// 下载句柄长度上限（字节）：句柄是定长 hex，越长越不可能是本实例签发的。
const HANDLE_MAX_BYTES: usize = 128;

/// 下载请求体：严格 JSON，未知字段拒绝（不得指定文件、目录或采集目标）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadRequest {
    /// 用户显式确认下载。
    confirmed: bool,
    /// 一次性下载句柄（浏览器内存持有）。
    handle: String,
}

/// 下发流：持有读取器与采集门 permit——流结束或连接中断时才释放门，
/// 保证同一时刻只有一条采集/下发在跑。
struct BundleStream {
    reader: ReaderStream<SyncIoBridge<std::fs::File>>,
    _permit: OwnedSemaphorePermit,
}

impl Stream for BundleStream {
    type Item = std::io::Result<Bytes>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.reader).poll_next(cx)
    }
}

/// 驾驶舱日志打包子路由。
///
/// 授权表与并发门都用**局部** `Extension` 注入，不改全局 `WebAppState`：
/// `Arc<Grants>` 承载实例内一次性授权，`Semaphore`（单并发）限制同时进行的
/// 采集任务。
pub(crate) fn routes() -> Router<WebAppState> {
    let grants = Arc::new(Grants::default());
    let collection_gate = Arc::new(Semaphore::new(1));
    Router::new()
        .route(
            "/api/diagnostics/log-bundle/authorize",
            post(authorize),
        )
        .route(
            "/api/diagnostics/log-bundle/download",
            post(download).layer(DefaultBodyLimit::max(DOWNLOAD_BODY_LIMIT_BYTES)),
        )
        .layer(Extension(grants))
        .layer(Extension(collection_gate))
}

/// 申请一次导出授权：不读来源，只登记一次待批准的申请。
async fn authorize(
    State(state): State<WebAppState>,
    Extension(grants): Extension<Arc<Grants>>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let origin = match check_source(&state, &headers, &uri) {
        Ok(origin) => origin,
        Err(response) => return response,
    };
    let ticket = match grants.issue(&origin) {
        Ok(ticket) => ticket,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "failed to create authorization request",
            );
        }
    };
    json_response(
        StatusCode::OK,
        json!({
            "request_id": ticket.request_id,
            "handle": ticket.handle,
            "expires_in_seconds": AUTHORIZATION_TTL_SECONDS,
        }),
    )
}

/// 采集并下发诊断包：句柄被本机批准后才采集，一次性核销。
async fn download(
    State(state): State<WebAppState>,
    Extension(grants): Extension<Arc<Grants>>,
    Extension(collection_gate): Extension<Arc<Semaphore>>,
    headers: HeaderMap,
    uri: Uri,
    payload: Result<Json<DownloadRequest>, JsonRejection>,
) -> Response {
    let origin = match check_source(&state, &headers, &uri) {
        Ok(origin) => origin,
        Err(response) => return response,
    };
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(rejection) => return rejection_response(rejection),
    };
    if !request.confirmed {
        return error_response(
            StatusCode::FORBIDDEN,
            "not_approved",
            "export was not confirmed",
        );
    }
    if request.handle.is_empty() || request.handle.len() > HANDLE_MAX_BYTES {
        return error_response(
            StatusCode::FORBIDDEN,
            "not_approved",
            "export was not approved",
        );
    }
    // 单并发：忙时直接拒绝（429），不让请求排队堆积。
    let permit = match collection_gate.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "busy",
                "another export is in progress",
            );
        }
    };
    // 一次性核销：未批准/过期/重放/跨源/他实例签发一律 false。
    if !grants.redeem(&origin, &request.handle) {
        return error_response(
            StatusCode::FORBIDDEN,
            "not_approved",
            "export was not approved",
        );
    }
    // 运行信息复用既有 handler 的同一 JSON 来源，不二次 HTTP、不拼域名。
    let runtime = super::health::runtime_info(State(state)).await.0;
    // 采集在阻塞线程内完成，并**在阻塞线程内持有 permit**：请求被中断时
    // 采集仍在进行，门不会被提前释放而让并发失控。
    let collected = tokio::task::spawn_blocking(move || {
        let outcome = archive::from_host(runtime);
        let length = outcome
            .as_ref()
            .ok()
            .and_then(|file| file.metadata().ok())
            .map(|metadata| metadata.len());
        (outcome, length, permit)
    })
    .await;
    let (outcome, length, permit) = match collected {
        Ok(collected) => collected,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "failed to collect diagnostics",
            );
        }
    };
    let file = match outcome {
        Ok(file) => file,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "failed to collect diagnostics",
            );
        }
    };
    let file_name = format!(
        "aria-logs-{}.tar.gz",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    );
    // 流式下发，不复制整包；permit 随响应体存活到流结束/连接中断。
    let stream = BundleStream {
        reader: ReaderStream::new(SyncIoBridge::new(file)),
        _permit: permit,
    };
    let mut response = Response::new(Body::from_stream(stream));
    let response_headers = response.headers_mut();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/gzip"),
    );
    response_headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{file_name}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response_headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    if let Some(length) = length {
        response_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    }
    response
}

/// 来源校验：精确受信 Origin + 外部必须 HTTPS + Host 与 Origin 同源。
///
/// 返回归一化后的 Origin（`scheme://host:port`），失败时返回静态 403 响应。
fn check_source(
    state: &WebAppState,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<String, Response> {
    let Some(auth) = state.api_auth.as_ref() else {
        return Err(error_response(
            StatusCode::FORBIDDEN,
            "auth_unavailable",
            "source validation is not configured",
        ));
    };
    let raw_origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let Ok(origin) = parse_trusted_origin(raw_origin) else {
        return Err(untrusted_origin_response());
    };
    if !auth.origin_allowed(&origin.origin) {
        return Err(untrusted_origin_response());
    }
    // 拒绝外部明文 HTTP：只放行明确的回环主机（本地开发/本机使用）。
    if !origin.https && !is_loopback_host(&authority_host(&origin.authority)) {
        return Err(untrusted_origin_response());
    }
    // Host 与 Origin 同源（不采信 X-Forwarded-*）。
    let Some(host) = request_host(headers, uri) else {
        return Err(untrusted_origin_response());
    };
    if !auth.host_allowed(&host) {
        return Err(untrusted_origin_response());
    }
    let host_host = authority_host(&host);
    let origin_host = authority_host(&origin.authority);
    if host_host != origin_host && !is_loopback_host(&host_host) {
        return Err(untrusted_origin_response());
    }
    Ok(origin.origin)
}

/// 请求的 Host authority：优先 Host 头，回退 URI authority（HTTP/2 `:authority`）。
fn request_host(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| uri.authority().map(|authority| authority.as_str().to_string()))
}

/// 从 authority（`host:port`）取主机部分（IPv6 保留方括号形式）。
fn authority_host(authority: &str) -> String {
    if let Some(stripped) = authority.strip_prefix('[') {
        if let Some((host, _tail)) = stripped.split_once(']') {
            return format!("[{host}]");
        }
    }
    match authority.rsplit_once(':') {
        Some((host, _port)) => host.to_string(),
        None => authority.to_string(),
    }
}

/// 是否明确的回环主机（`localhost` 域、`127.0.0.0/8`、`::1`）。
fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(|ch| ch == '[' || ch == ']').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
}

/// 静态 JSON 错误响应：无路径、无句柄、无其它秘密。
fn error_response(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"code": code, "message": message, "details": {}})),
    )
        .into_response()
}

fn untrusted_origin_response() -> Response {
    error_response(
        StatusCode::FORBIDDEN,
        "untrusted_origin",
        "request origin not allowed",
    )
}

/// JSON 响应（禁止缓存）。
fn json_response(status: StatusCode, value: Value) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(value),
    )
        .into_response()
}

/// 请求体不合规时的静态 JSON 错误（不泄露解析细节）。
fn rejection_response(rejection: JsonRejection) -> Response {
    let status = rejection.status();
    let code = if status == StatusCode::PAYLOAD_TOO_LARGE {
        "request_too_large"
    } else {
        "invalid_request"
    };
    error_response(status, code, "invalid request body")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use std::net::SocketAddr;
    use tower::ServiceExt;

    use crate::web::api_auth::ApiAuthState;
    use crate::web::runtime::WebRuntime;

    /// 构造带来源校验状态的测试实例（工作区在 /tmp 临时目录内）。
    fn state_with_origins(root: &std::path::Path, origins: &[&str]) -> WebAppState {
        let bind: SocketAddr = "127.0.0.1:4317".parse().expect("bind addr");
        let explicit: Vec<String> = origins.iter().map(|origin| origin.to_string()).collect();
        let auth = ApiAuthState::for_bind(&bind, &explicit).expect("api auth");
        WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
            .with_api_auth(Arc::new(auth))
    }

    fn post_request(
        uri: &str,
        origin: Option<&str>,
        host: Option<&str>,
        body: &str,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        if let Some(host) = host {
            builder = builder.header(header::HOST, host);
        }
        builder
            .body(Body::from(body.to_string()))
            .expect("request")
    }

    async fn response_json(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    fn headers_with(origin: &str, host: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_str(origin).expect("origin header"),
        );
        headers.insert(
            header::HOST,
            HeaderValue::from_str(host).expect("host header"),
        );
        headers
    }

    fn request_uri() -> Uri {
        "/api/diagnostics/log-bundle/authorize"
            .parse()
            .expect("request uri")
    }

    #[tokio::test]
    async fn authorize_without_origin_is_rejected() {
        let root = tempfile::tempdir().expect("root");
        let state = state_with_origins(root.path(), &["http://localhost:5173"]);
        let app = routes().with_state(state);
        let response = app
            .oneshot(post_request(
                "/api/diagnostics/log-bundle/authorize",
                None,
                Some("localhost:5173"),
                "{}",
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "untrusted_origin");
    }

    #[tokio::test]
    async fn authorize_from_plaintext_remote_origin_is_rejected() {
        let root = tempfile::tempdir().expect("root");
        // 该明文远端来源本身受信：拒绝来自本端点的“外部明文 HTTP”规则。
        let state = state_with_origins(root.path(), &["http://arch-code.example"]);
        let app = routes().with_state(state);
        let response = app
            .oneshot(post_request(
                "/api/diagnostics/log-bundle/authorize",
                Some("http://arch-code.example"),
                Some("arch-code.example"),
                "{}",
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "untrusted_origin");
    }

    #[tokio::test]
    async fn endpoints_are_rejected_without_source_validation() {
        let root = tempfile::tempdir().expect("root");
        let state = WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()));
        for (uri, body) in [
            ("/api/diagnostics/log-bundle/authorize", "{}"),
            (
                "/api/diagnostics/log-bundle/download",
                r#"{"confirmed":true,"handle":"abc"}"#,
            ),
        ] {
            let app = routes().with_state(state.clone());
            let response = app
                .oneshot(post_request(
                    uri,
                    Some("http://localhost:5173"),
                    Some("localhost:5173"),
                    body,
                ))
                .await
                .expect("response");
            let (status, value) = response_json(response).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(value["code"], "auth_unavailable");
        }
    }

    #[tokio::test]
    async fn download_without_host_approval_is_rejected() {
        let root = tempfile::tempdir().expect("root");
        let state = state_with_origins(root.path(), &["http://localhost:5173"]);
        let app = routes().with_state(state);
        let response = app
            .oneshot(post_request(
                "/api/diagnostics/log-bundle/download",
                Some("http://localhost:5173"),
                Some("localhost:5173"),
                r#"{"confirmed":true,"handle":"0123456789abcdef"}"#,
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "not_approved");
        // 未获批准时不得给出任何归档内容。
        assert!(body.get("runtime-info.json").is_none());
    }

    #[tokio::test]
    async fn download_without_confirmation_is_rejected() {
        let root = tempfile::tempdir().expect("root");
        let state = state_with_origins(root.path(), &["http://localhost:5173"]);
        let app = routes().with_state(state);
        let response = app
            .oneshot(post_request(
                "/api/diagnostics/log-bundle/download",
                Some("http://localhost:5173"),
                Some("localhost:5173"),
                r#"{"confirmed":false,"handle":"0123456789abcdef"}"#,
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "not_approved");
    }

    #[tokio::test]
    async fn download_rejects_unknown_fields() {
        let root = tempfile::tempdir().expect("root");
        let state = state_with_origins(root.path(), &["http://localhost:5173"]);
        let app = routes().with_state(state);
        let response = app
            .oneshot(post_request(
                "/api/diagnostics/log-bundle/download",
                Some("http://localhost:5173"),
                Some("localhost:5173"),
                r#"{"confirmed":true,"handle":"0123456789abcdef","path":"/etc/passwd"}"#,
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_request");
    }

    #[tokio::test]
    async fn download_from_cross_origin_is_rejected() {
        let root = tempfile::tempdir().expect("root");
        let state = state_with_origins(root.path(), &["http://localhost:5173"]);
        let app = routes().with_state(state);
        let response = app
            .oneshot(post_request(
                "/api/diagnostics/log-bundle/download",
                Some("http://127.0.0.1:5173"),
                Some("localhost:5173"),
                r#"{"confirmed":true,"handle":"0123456789abcdef"}"#,
            ))
            .await
            .expect("response");
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "untrusted_origin");
    }

    #[test]
    fn source_check_accepts_trusted_https_origin() {
        let root = tempfile::tempdir().expect("root");
        let state = state_with_origins(root.path(), &["https://arch-code.example"]);
        let headers = headers_with("https://arch-code.example", "arch-code.example");
        assert!(check_source(&state, &headers, &request_uri()).is_ok());
    }

    #[test]
    fn source_check_accepts_loopback_http_origin() {
        let root = tempfile::tempdir().expect("root");
        let state = state_with_origins(root.path(), &["http://localhost:5173"]);
        let headers = headers_with("http://localhost:5173", "localhost:5173");
        assert!(check_source(&state, &headers, &request_uri()).is_ok());
    }

    #[test]
    fn source_check_rejects_host_from_another_trusted_origin() {
        let root = tempfile::tempdir().expect("root");
        let state = state_with_origins(
            root.path(),
            &["https://a.example", "https://b.example"],
        );
        let headers = headers_with("https://a.example", "b.example");
        assert!(check_source(&state, &headers, &request_uri()).is_err());
    }

    #[test]
    fn source_check_rejects_plaintext_remote_origin() {
        let root = tempfile::tempdir().expect("root");
        let state = state_with_origins(root.path(), &["http://arch-code.example"]);
        let headers = headers_with("http://arch-code.example", "arch-code.example");
        assert!(check_source(&state, &headers, &request_uri()).is_err());
    }
}
