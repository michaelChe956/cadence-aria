use std::path::PathBuf;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use std::convert::Infallible;
use tower::util::ServiceExt; // ServeDir 的 oneshot
use tower_http::services::{ServeDir, ServeFile};

/// 编译期嵌入的前端构建产物。`web/dist` 必须先由 `pnpm -C web build` 生成
/// （build.rs 会在缺失时硬失败）。
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct WebAssets;

const INDEX_HTML: &str = "index.html";

/// 统一的前端静态资源 fallback 服务。
///
/// - 未设置 `ARIA_WEB_DIST`：从嵌入资源服务（命中返回文件，未命中 SPA fallback 到 index.html）。
/// - 设置了 `ARIA_WEB_DIST` 且目录存在：改用 ServeDir 从该磁盘目录读取（开发/e2e 逃生口）。
pub fn static_dist_service() -> StaticDistService {
    match std::env::var_os("ARIA_WEB_DIST") {
        Some(dir) if !dir.is_empty() && PathBuf::from(&dir).is_dir() => {
            let dir = PathBuf::from(dir);
            let index = dir.join(INDEX_HTML);
            StaticDistService::Disk(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        _ => StaticDistService::Embedded,
    }
}

/// fallback 服务的两种形态：嵌入资源 或 磁盘目录（ServeDir）。
#[derive(Clone)]
pub enum StaticDistService {
    Embedded,
    Disk(ServeDir<ServeFile>),
}

impl StaticDistService {
    /// 从嵌入资源构造响应：精确命中返回该文件，未命中返回 index.html（SPA fallback）。
    fn embedded_response(uri: &Uri) -> Response {
        let path = uri.path().trim_start_matches('/');
        let candidate = if path.is_empty() { INDEX_HTML } else { path };

        if let Some(content) = WebAssets::get(candidate) {
            return embedded_file_response(candidate, content.data.into_owned());
        }
        // SPA fallback：未命中一律返回 index.html
        match WebAssets::get(INDEX_HTML) {
            Some(content) => embedded_file_response(INDEX_HTML, content.data.into_owned()),
            None => (StatusCode::NOT_FOUND, "index.html not embedded").into_response(),
        }
    }
}

fn embedded_file_response(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut response = Body::from(bytes).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref())
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    response
}

/// 作为 axum fallback 的统一 handler。根据服务形态分派到嵌入资源或磁盘 ServeDir。
pub async fn serve_static(
    service: StaticDistService,
    req: Request,
) -> Result<Response, Infallible> {
    match service {
        StaticDistService::Embedded => Ok(StaticDistService::embedded_response(req.uri())),
        StaticDistService::Disk(serve_dir) => {
            // ServeDir 的 fallback(ServeFile) 已覆盖未命中→index.html
            match serve_dir.oneshot(req).await {
                Ok(resp) => Ok(resp.into_response()),
                Err(err) => match err {},
            }
        }
    }
}
