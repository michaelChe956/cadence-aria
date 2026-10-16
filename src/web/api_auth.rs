//! 批2 Task 1（C1.1 / B1 / REQ-API-AUTH-01/02）：控制面统一入站认证。
//!
//! 私有合同（plan 批量2 T1）：
//! - `ApiAuthState` 进程内持有一次性 bootstrap token 与会话；token/会话 ID 均由
//!   OS CSPRNG 产生至少 32 个随机字节（不使用时间/PID/hash 造熵）；进程重启全部失效。
//! - `POST /api/auth/session` 仅从请求 body 接 token，原子消费一次并签发
//!   `aria_session` cookie（`HttpOnly; SameSite=Strict; Path=/`，全 HTTPS 受信来源
//!   时附加 `Secure`）；复用/错误 token 一律 401，不回显 token 或匹配前缀。
//! - 最外层中间件 [`enforce_api_auth`] 在 handler/upgrade 之前校验 Host/Origin/身份；
//!   仅无敏感信息的 `GET /api/health` 公开，其余 `/api/**`（含 test/evidence）与
//!   `/ws/**`（含 alias）默认认证；localhost 不豁免。
//! - 受信 Origin/Host 只来自真实 bind 地址推导与显式配置（环境变量
//!   [`TRUSTED_ORIGINS_ENV`]），禁止 wildcard、禁止从请求头反推允许列表。
//! - token 不进入 URL/结构化日志/env/provider 环境；令牌比较使用常时比较。

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::web::state::WebAppState;

/// 会话 cookie 名。
pub const SESSION_COOKIE_NAME: &str = "aria_session";

/// 显式受信 Origin 配置（逗号分隔，精确 `scheme://host:port`，如开发前端
/// `http://localhost:5173`）。bind 非 loopback 时必须提供；仅收 Origin 允许列表，
/// 与任何令牌无关。
pub const TRUSTED_ORIGINS_ENV: &str = "ARIA_API_TRUSTED_ORIGINS";

/// token / 会话 ID 的随机字节数（合同下限 32）。
const SECRET_BYTES: usize = 32;

/// OS CSPRNG：直接读 `/dev/urandom`；不可用即显式报错，绝不降级到时间/PID/hash 熵。
fn os_random_bytes(count: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut source = std::fs::File::open("/dev/urandom")?;
    let mut buffer = vec![0u8; count];
    source.read_exact(&mut buffer)?;
    Ok(buffer)
}

/// 常时字节比较：不因比较顺序泄漏匹配前缀。
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 配置错误（fail-visible，绝不静默放宽）。
#[derive(Debug)]
pub enum ApiAuthConfigError {
    /// bind 非 loopback 且未显式配置受信 Origin。
    NonLoopbackRequiresExplicitOrigins { bind: String },
    /// 显式 Origin 非法（scheme/通配符/路径/缺端口）。
    InvalidOrigin(String),
    /// OS CSPRNG 不可用。
    EntropyUnavailable(String),
}

impl std::fmt::Display for ApiAuthConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonLoopbackRequiresExplicitOrigins { bind } => write!(
                f,
                "bind 非 loopback（{bind}）时必须经 {TRUSTED_ORIGINS_ENV} 显式配置受信 Origin，不默认推导"
            ),
            Self::InvalidOrigin(raw) => write!(
                f,
                "受信 Origin 非法（须精确 scheme://host:port，禁止通配符/路径/缺端口）：{raw}"
            ),
            Self::EntropyUnavailable(details) => {
                write!(f, "OS CSPRNG 不可用，拒绝生成认证材料：{details}")
            }
        }
    }
}

impl std::error::Error for ApiAuthConfigError {}

/// 精确受信 Origin（规范化小写；保留 IPv6 括号形式）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedOrigin {
    /// 完整 origin 字符串，如 `http://127.0.0.1:4317`。
    pub origin: String,
    /// `host:port` authority，如 `127.0.0.1:4317` / `[::1]:5173`。
    pub authority: String,
    pub https: bool,
}

/// 解析一条显式受信 Origin：仅 http/https、无路径/query/通配符、必须显式端口。
pub fn parse_trusted_origin(raw: &str) -> Result<TrustedOrigin, ApiAuthConfigError> {
    let raw = raw.trim();
    let invalid = || ApiAuthConfigError::InvalidOrigin(raw.to_string());
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Err(invalid());
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(invalid());
    }
    if rest.is_empty()
        || rest.contains('/')
        || rest.contains('*')
        || rest.contains('?')
        || rest.contains('#')
        || rest.chars().any(char::is_whitespace)
    {
        return Err(invalid());
    }
    // 拆 host:port：IPv6 形如 [::1]:5173；否则取最后一个 ':'。
    let (host, port) = if let Some(stripped) = rest.strip_prefix('[') {
        let Some((inner, tail)) = stripped.split_once(']') else {
            return Err(invalid());
        };
        let Some(port) = tail.strip_prefix(':') else {
            return Err(invalid());
        };
        (format!("[{inner}]"), port)
    } else {
        let Some((host, port)) = rest.rsplit_once(':') else {
            return Err(invalid());
        };
        (host.to_string(), port)
    };
    let port: u16 = port.parse().map_err(|_| invalid())?;
    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return Err(invalid());
    }
    let authority = format!("{host}:{port}");
    Ok(TrustedOrigin {
        origin: format!("{scheme}://{authority}"),
        authority,
        https: scheme == "https",
    })
}

/// 解析环境变量原始值为显式 Origin 列表（逗号/空白分隔，空项跳过）。
pub fn parse_trusted_origins_env(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || char::is_whitespace(c))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// 认证内部状态（一把锁保证 bootstrap 原子消费与会话集合一致）。
struct AuthInner {
    bootstrap_token: Vec<u8>,
    consumed: bool,
    sessions: HashSet<Vec<u8>>,
}

/// 控制面认证状态：进程内一次性 bootstrap token + 会话集合 + 受信 Origin/Host。
pub struct ApiAuthState {
    inner: Mutex<AuthInner>,
    trusted_origins: Vec<TrustedOrigin>,
    expected_authorities: Vec<String>,
    secure_cookie: bool,
}

impl ApiAuthState {
    /// 依据真实 bind 地址构造。loopback bind 默认推导精确回环 origin；非 loopback
    /// bind 必须提供非空 `explicit_origins`（显式配置，不默认推导、不接受 wildcard）。
    pub fn for_bind(
        bound: &SocketAddr,
        explicit_origins: &[String],
    ) -> Result<Self, ApiAuthConfigError> {
        let bootstrap_token = os_random_bytes(SECRET_BYTES)
            .map_err(|error| ApiAuthConfigError::EntropyUnavailable(error.to_string()))?;
        let mut trusted_origins = Vec::new();
        let mut expected_authorities = Vec::new();
        if bound.ip().is_loopback() {
            for host in ["127.0.0.1", "localhost", "[::1]"] {
                trusted_origins.push(TrustedOrigin {
                    origin: format!("http://{host}:{}", bound.port()),
                    authority: format!("{host}:{}", bound.port()),
                    https: false,
                });
                expected_authorities.push(format!("{host}:{}", bound.port()));
            }
        }
        for raw in explicit_origins {
            let parsed = parse_trusted_origin(raw)?;
            if !trusted_origins.contains(&parsed) {
                expected_authorities.push(parsed.authority.clone());
                trusted_origins.push(parsed);
            }
        }
        if !bound.ip().is_loopback() && trusted_origins.is_empty() {
            return Err(ApiAuthConfigError::NonLoopbackRequiresExplicitOrigins {
                bind: bound.to_string(),
            });
        }
        // TLS 时 Secure：仅当全部受信来源均为 https（本地 http 部署不加，避免 cookie 失效）。
        let secure_cookie =
            !trusted_origins.is_empty() && trusted_origins.iter().all(|origin| origin.https);
        Ok(Self {
            inner: Mutex::new(AuthInner {
                bootstrap_token,
                consumed: false,
                sessions: HashSet::new(),
            }),
            trusted_origins,
            expected_authorities,
            secure_cookie,
        })
    }

    /// 受信 Origin 列表（诊断/测试；不包含任何令牌材料）。
    pub fn trusted_origins(&self) -> &[TrustedOrigin] {
        &self.trusted_origins
    }

    /// 允许的 Host authority 列表（诊断/测试）。
    pub fn expected_authorities(&self) -> &[String] {
        &self.expected_authorities
    }

    /// 会话 cookie 是否附 `Secure`。
    pub fn secure_cookie(&self) -> bool {
        self.secure_cookie
    }

    /// bootstrap token 十六进制形式。**仅**供 owner 受信终端交付与测试使用，
    /// 严禁进入结构化日志、文件、URL、env 或 provider 环境。
    pub(crate) fn bootstrap_token_hex(&self) -> String {
        let inner = self.inner.lock().expect("api auth lock");
        hex::encode(&inner.bootstrap_token)
    }

    pub fn exchange(&self, presented: &str) -> Option<String> {
        let presented_bytes = hex::decode(presented.trim()).ok()?;
        let mut inner = self.inner.lock().expect("api auth lock");
        // 先常时比较再判消费：任何失败路径耗时一致，不泄漏匹配前缀。
        let matched = constant_time_eq(&presented_bytes, &inner.bootstrap_token);
        if !matched || inner.consumed {
            return None;
        }
        // CSPRNG 不可用则拒绝交换（fail-closed），绝不降级造熵。
        let session = os_random_bytes(SECRET_BYTES).ok()?;
        inner.consumed = true;
        inner.sessions.insert(session.clone());
        Some(hex::encode(session))
    }

    /// 会话 cookie 值是否有效（常时比较；进程内会话，重启即失效）。
    pub fn session_valid(&self, presented: &str) -> bool {
        let Some(presented_bytes) = hex::decode(presented.trim()).ok() else {
            return false;
        };
        let inner = self.inner.lock().expect("api auth lock");
        inner
            .sessions
            .iter()
            .any(|session| constant_time_eq(&presented_bytes, session))
    }

    /// Origin 是否精确受信（大小写不敏感；`null`/任意伪造值不匹配）。
    pub fn origin_allowed(&self, origin: &str) -> bool {
        let normalized = origin.trim().to_ascii_lowercase();
        self.trusted_origins
            .iter()
            .any(|trusted| trusted.origin == normalized)
    }

    /// Host authority 是否允许（DNS rebinding 防线；小写归一比较）。
    pub fn host_allowed(&self, host: &str) -> bool {
        let normalized = host.trim().to_ascii_lowercase();
        self.expected_authorities.contains(&normalized)
    }
}

/// 生成会话 cookie 的 `Set-Cookie` 值：`HttpOnly; SameSite=Strict; Path=/`，
/// `secure` 时附加 `Secure`。
pub fn set_session_cookie(session_hex: &str, secure: bool) -> String {
    let mut cookie =
        format!("{SESSION_COOKIE_NAME}={session_hex}; HttpOnly; SameSite=Strict; Path=/");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// 从请求 `Cookie` 头提取指定 cookie 的值（无则 `None`）。
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    for header_value in headers.get_all(header::COOKIE) {
        let raw = header_value.to_str().ok()?;
        for pair in raw.split(';') {
            let pair = pair.trim();
            if let Some(value) = pair.strip_prefix(&prefix) {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({
            "code": "unauthorized",
            "message": "authentication required",
            "details": {},
        })),
    )
        .into_response()
}

fn forbidden_response(code: &str, message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(serde_json::json!({
            "code": code,
            "message": message,
            "details": {},
        })),
    )
        .into_response()
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    let connection_upgrade = headers
        .get(header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    let websocket = headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    connection_upgrade && websocket
}

fn path_guarded_by_auth(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/") || path == "/ws" || path.starts_with("/ws/")
}

/// 最外层认证/来源校验中间件：在 handler 与 WS upgrade 之前执行。
///
/// 未装配 `api_auth`（进程内测试默认）时直通，保持既有内嵌测试行为；生产
/// `serve_web` 始终装配。顺序：Host → Origin → 身份；`GET /api/health` 公开；
/// `GET/POST /api/auth/session` 免身份（会话检查 401/204 / token 交换由 handler 完成）。
pub async fn enforce_api_auth(
    State(state): State<WebAppState>,
    req: Request,
    next: Next,
) -> Response {
    let Some(auth) = state.api_auth.clone() else {
        return next.run(req).await;
    };
    enforce(auth, req, next).await
}

async fn enforce(auth: Arc<ApiAuthState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if !path_guarded_by_auth(&path) {
        return next.run(req).await;
    }
    let method = req.method().clone();
    // 仅无敏感信息的健康检查公开。
    if method == axum::http::Method::GET && path == "/api/health" {
        return next.run(req).await;
    }
    let headers = req.headers().clone();
    // Host 先于 Origin/身份/交换（DNS rebinding 防线）。
    let host_ok = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(|host| auth.host_allowed(host))
        .unwrap_or(false);
    if !host_ok {
        return forbidden_response("forbidden_host", "request host not allowed");
    }
    // mutation 与 WS upgrade 必须携带受信精确 Origin（CLI 显式带配置 Origin）；
    // 敏感 GET 无 Origin 仍放行到身份检查；非法/`null` Origin 一律拒绝。
    let wants_ws = is_websocket_upgrade(&headers);
    let is_read = method == axum::http::Method::GET
        || method == axum::http::Method::HEAD
        || method == axum::http::Method::OPTIONS;
    let origin_ok = match headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        Some(raw) => auth.origin_allowed(raw),
        None => is_read && !wants_ws,
    };
    if !origin_ok {
        return forbidden_response("forbidden_origin", "request origin not allowed");
    }
    // 会话检查 / token 交换端点免身份（handler 自身给出 401/204/400）。
    if path == "/api/auth/session"
        && (method == axum::http::Method::GET || method == axum::http::Method::POST)
    {
        return next.run(req).await;
    }
    let session_ok = cookie_value(&headers, SESSION_COOKIE_NAME)
        .map(|value| auth.session_valid(&value))
        .unwrap_or(false);
    if !session_ok {
        return unauthorized_response();
    }
    next.run(req).await
}

/// `GET /api/auth/session`：已认证 204（空体，无 token 回显），未认证 401。
pub async fn session_status(State(state): State<WebAppState>, headers: HeaderMap) -> Response {
    let Some(auth) = state.api_auth else {
        return unauthorized_response();
    };
    let authenticated = cookie_value(&headers, SESSION_COOKIE_NAME)
        .map(|value| auth.session_valid(&value))
        .unwrap_or(false);
    if authenticated {
        StatusCode::NO_CONTENT.into_response()
    } else {
        unauthorized_response()
    }
}

/// `POST /api/auth/session`：仅 body 接 token（`{"token": "<hex>"}`），原子消费一次
/// 并以 204 + `Set-Cookie` 签发会话；失败统一 401（不回显材料）。query/头里的 token
/// 一律不读取。
pub async fn create_session(
    State(state): State<WebAppState>,
    body: Result<axum::Json<serde_json::Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Some(auth) = state.api_auth else {
        return unauthorized_response();
    };
    let token = match body {
        Ok(axum::Json(value)) => value
            .get("token")
            .and_then(|token| token.as_str())
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty()),
        Err(_) => None,
    };
    let Some(token) = token else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "code": "invalid_request",
                "message": "token must be provided in the request body",
                "details": {},
            })),
        )
            .into_response();
    };
    match auth.exchange(&token) {
        Some(session_hex) => {
            let cookie = set_session_cookie(&session_hex, auth.secure_cookie());
            ([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response()
        }
        None => unauthorized_response(),
    }
}

/// bootstrap token 的受信终端交付文案。`tty` 为 true 时包含 token（仅面向 owner
/// 终端输出）；false 时显式报告缺少交付前提，绝不包含 token、绝不绕道集中日志。
pub fn delivery_notice(auth: &ApiAuthState, tty: bool) -> String {
    if tty {
        format!(
            "[aria-auth] 一次性 bootstrap token（仅本进程有效，请粘贴到浏览器认证入口；不会再次显示，不写入任何日志/文件/env）：\n[aria-auth] {}",
            auth.bootstrap_token_hex()
        )
    } else {
        "[aria-auth] 缺少受信终端交付前提（stderr 非 TTY）：bootstrap token 不输出，也不写入日志/文件/env 绕过。\n[aria-auth] 需要浏览器会话时，请在 owner 交互终端（或 tmux/screen）启动 aria web 完成交付。"
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loopback_bind(port: u16) -> SocketAddr {
        format!("127.0.0.1:{port}")
            .parse()
            .expect("loopback bind addr")
    }

    #[test]
    fn api_auth_bootstrap_token_is_64_hex_chars_and_unique_per_state() {
        let a = ApiAuthState::for_bind(&loopback_bind(39721), &[]).expect("auth a");
        let b = ApiAuthState::for_bind(&loopback_bind(39721), &[]).expect("auth b");
        let token_a = a.bootstrap_token_hex();
        let token_b = b.bootstrap_token_hex();
        assert_eq!(token_a.len(), 64);
        assert!(token_a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(
            token_a, token_b,
            "CSPRNG token must not repeat across states"
        );
    }

    #[test]
    fn api_auth_exchange_consumes_bootstrap_token_exactly_once() {
        let auth = ApiAuthState::for_bind(&loopback_bind(39721), &[]).expect("auth");
        let token = auth.bootstrap_token_hex();
        let first = auth
            .exchange(&token)
            .expect("first exchange issues session");
        assert_eq!(first.len(), 64);
        assert!(auth.session_valid(&first), "issued session must validate");
        assert!(
            auth.exchange(&token).is_none(),
            "bootstrap token reuse must be rejected"
        );
        assert!(
            auth.exchange("deadbeef").is_none(),
            "wrong token must be rejected"
        );
        assert!(
            auth.exchange("not-hex!").is_none(),
            "non-hex token must be rejected uniformly"
        );
        assert!(auth.exchange("").is_none(), "empty token must be rejected");
    }

    #[test]
    fn api_auth_unknown_session_rejected() {
        let auth = ApiAuthState::for_bind(&loopback_bind(39721), &[]).expect("auth");
        assert!(!auth.session_valid(&"0".repeat(64)));
        assert!(!auth.session_valid("garbage"));
    }

    #[test]
    fn api_auth_loopback_bind_derives_exact_loopback_origins() {
        let auth = ApiAuthState::for_bind(&loopback_bind(39721), &[]).expect("auth");
        let origins: Vec<&str> = auth
            .trusted_origins()
            .iter()
            .map(|origin| origin.origin.as_str())
            .collect();
        assert!(origins.contains(&"http://127.0.0.1:39721"));
        assert!(origins.contains(&"http://localhost:39721"));
        assert!(origins.contains(&"http://[::1]:39721"));
        assert_eq!(origins.len(), 3, "no wildcard/extra origins by default");

        assert!(auth.origin_allowed("http://127.0.0.1:39721"));
        assert!(auth.origin_allowed("HTTP://LOCALHOST:39721"));
        assert!(!auth.origin_allowed("http://127.0.0.1:39722"));
        assert!(!auth.origin_allowed("null"));
        assert!(!auth.origin_allowed(""));
        assert!(!auth.origin_allowed("http://evil.example:39721"));

        assert!(auth.host_allowed("127.0.0.1:39721"));
        assert!(auth.host_allowed("localhost:39721"));
        assert!(auth.host_allowed("[::1]:39721"));
        assert!(!auth.host_allowed("127.0.0.1:39722"));
        assert!(!auth.host_allowed("evil.example:4317"));
        assert!(!auth.host_allowed("127.0.0.1"));
    }

    #[test]
    fn api_auth_non_loopback_bind_requires_explicit_origins() {
        let bind: SocketAddr = "192.168.1.5:4317".parse().expect("bind");
        let error = match ApiAuthState::for_bind(&bind, &[]) {
            Err(error) => error,
            Ok(_) => panic!("non-loopback without explicit origins must fail"),
        };
        assert!(matches!(
            error,
            ApiAuthConfigError::NonLoopbackRequiresExplicitOrigins { .. }
        ));

        let auth = ApiAuthState::for_bind(&bind, &["http://localhost:5173".to_string()])
            .expect("explicit origins accepted");
        let origins: Vec<&str> = auth
            .trusted_origins()
            .iter()
            .map(|origin| origin.origin.as_str())
            .collect();
        assert_eq!(
            origins,
            &["http://localhost:5173"],
            "no derivation for non-loopback"
        );
        assert!(auth.origin_allowed("http://localhost:5173"));
        assert!(auth.host_allowed("localhost:5173"));
        assert!(!auth.host_allowed("192.168.1.5:4317"));
    }

    #[test]
    fn api_auth_rejects_wildcard_missing_port_paths_and_unknown_schemes() {
        assert!(parse_trusted_origin("http://*").is_err());
        assert!(parse_trusted_origin("http://*.example").is_err());
        assert!(parse_trusted_origin("http://localhost").is_err());
        assert!(parse_trusted_origin("http://localhost:5173/").is_err());
        assert!(parse_trusted_origin("http://localhost:5173/path").is_err());
        assert!(parse_trusted_origin("ftp://localhost:5173").is_err());
        assert!(parse_trusted_origin("localhost:5173").is_err());
        assert!(parse_trusted_origin("http://[::1]:5173").is_ok());
        assert!(parse_trusted_origin(" https://localhost:5173 ").is_ok());
    }

    #[test]
    fn api_auth_https_only_origins_enable_secure_cookie() {
        let bind: SocketAddr = "192.168.1.5:4317".parse().expect("bind");
        let https = ApiAuthState::for_bind(&bind, &["https://aria.example:443".to_string()])
            .expect("https origin");
        assert!(https.secure_cookie());
        let mixed = ApiAuthState::for_bind(
            &bind,
            &[
                "https://aria.example:443".to_string(),
                "http://localhost:5173".to_string(),
            ],
        )
        .expect("mixed origins");
        assert!(!mixed.secure_cookie());
        let loopback = ApiAuthState::for_bind(&loopback_bind(39721), &[]).expect("loopback");
        assert!(!loopback.secure_cookie());
    }

    #[test]
    fn api_auth_session_cookie_attributes() {
        let plain = set_session_cookie(&"a".repeat(64), false);
        assert!(plain.starts_with("aria_session="));
        assert!(plain.contains("HttpOnly"));
        assert!(plain.contains("SameSite=Strict"));
        assert!(plain.contains("Path=/"));
        assert!(!plain.contains("Secure"));
        let secure = set_session_cookie(&"a".repeat(64), true);
        assert!(secure.contains("Secure"));
        assert!(secure.ends_with("; Secure"));
    }

    #[test]
    fn api_auth_cookie_value_extraction() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=1; aria_session=abc123; more=2"
                .parse()
                .expect("cookie header"),
        );
        assert_eq!(
            cookie_value(&headers, "aria_session").as_deref(),
            Some("abc123")
        );
        assert_eq!(cookie_value(&headers, "missing"), None);

        let mut empty = HeaderMap::new();
        empty.insert(header::COOKIE, "foo=bar".parse().expect("cookie header"));
        assert_eq!(cookie_value(&empty, "aria_session"), None);
        assert_eq!(cookie_value(&HeaderMap::new(), "aria_session"), None);
    }

    #[test]
    fn api_auth_trusted_origins_env_parsing() {
        assert_eq!(
            parse_trusted_origins_env("http://localhost:5173, https://x.example:8443 "),
            vec!["http://localhost:5173", "https://x.example:8443"]
        );
        assert!(parse_trusted_origins_env("").is_empty());
        assert!(parse_trusted_origins_env("  ").is_empty());
    }

    #[test]
    fn api_auth_delivery_notice_tty_contains_token_non_tty_reports_prerequisite() {
        let auth = ApiAuthState::for_bind(&loopback_bind(39721), &[]).expect("auth");
        let token = auth.bootstrap_token_hex();

        let tty = delivery_notice(&auth, true);
        assert!(tty.contains(&token), "tty delivery must carry the token");

        let non_tty = delivery_notice(&auth, false);
        assert!(!non_tty.contains(&token), "non-tty must not leak the token");
        assert!(
            non_tty.contains("受信终端"),
            "non-tty must explicitly report the missing delivery prerequisite, got: {non_tty}"
        );
    }

    #[test]
    fn api_auth_constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
