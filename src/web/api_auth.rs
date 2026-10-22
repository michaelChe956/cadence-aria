//! 控制面入站来源校验（Host/Origin）。
//!
//! 私有合同（owner 2026-10-22 决定：去 token/会话要求）：
//! - 控制面不要求登录：无 bootstrap token、无会话 cookie；控制面边界由
//!   Host + Origin 来源校验承担（本机单用户、浏览器场景；本机任意程序可
//!   伪造 Host/Origin 的暴露面为已知并接受，网页 drive-by 仍被挡住）。
//! - 最外层中间件 [`enforce_api_auth`] 在 handler/upgrade 之前校验 Host 与
//!   Origin；仅无敏感信息的 `GET /api/health` 公开，其余 `/api/**`（含
//!   test/evidence）与 `/ws/**`（含 alias）默认过来源门；localhost 不豁免。
//! - mutation 与 WS upgrade 必须携带受信精确 Origin；只读
//!   GET/HEAD/OPTIONS 无 Origin 放行到下一层；非法/`null` Origin、伪造/
//!   缺失 Host 一律 403。
//! - 受信 Origin/Host 只来自真实 bind 地址推导与显式配置（环境变量
//!   [`TRUSTED_ORIGINS_ENV`]），禁止 wildcard、禁止从请求头反推允许列表。
//! - Origin 端口可省略：配置侧与请求侧共用同一归一（trim + scheme/host 小写 +
//!   省略端口按 scheme 补默认 https→443/http→80 → `scheme://host:port` 全等比较）。
//! - Host 允许形式含 `host:port` 与裸 `host`：默认端口的受信 origin（反代域名
//!   的 Host 头不带端口）与回环口恒收录裸形式；非默认端口（如 8443）不放宽裸形式。

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::web::state::WebAppState;

/// 显式受信 Origin 配置（逗号分隔，精确 `scheme://host[:port]`，端口可省略并
/// 按 scheme 取默认（https→443、http→80），如开发前端 `http://localhost:5173`）。
/// bind 非 loopback 时必须提供；仅收 Origin 允许列表。
pub const TRUSTED_ORIGINS_ENV: &str = "ARIA_API_TRUSTED_ORIGINS";

/// 配置错误（fail-visible，绝不静默放宽）。
#[derive(Debug)]
pub enum ApiAuthConfigError {
    /// bind 非 loopback 且未显式配置受信 Origin。
    NonLoopbackRequiresExplicitOrigins { bind: String },
    /// 显式 Origin 非法（scheme/通配符/路径/畸形端口）。
    InvalidOrigin(String),
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
                "受信 Origin 非法（须为 scheme://host[:port]，端口可省略并按 scheme 取默认 https→443、http→80，禁止通配符/路径/query/fragment/空白）：{raw}"
            ),
        }
    }
}

impl std::error::Error for ApiAuthConfigError {}

/// 精确受信 Origin（规范化小写、端口补齐；保留 IPv6 括号形式）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedOrigin {
    /// 完整 origin 字符串，如 `http://127.0.0.1:4317`。
    pub origin: String,
    /// `host:port` authority，如 `127.0.0.1:4317` / `[::1]:5173`。
    pub authority: String,
    pub https: bool,
}

/// 解析一条显式受信 Origin：仅 http/https、无路径/query/通配符/空白；端口可
/// 省略，缺省按 scheme 取默认（https→443、http→80），内部归一为
/// `scheme://host:port`（显式端口照旧精确保留）。
pub fn parse_trusted_origin(raw: &str) -> Result<TrustedOrigin, ApiAuthConfigError> {
    normalize_origin(raw).ok_or_else(|| ApiAuthConfigError::InvalidOrigin(raw.trim().to_string()))
}

/// 归一化单条 origin（配置解析与请求侧 Origin 比较共用同一规则，防止两处
/// 漂移）：trim、scheme/host 小写、省略的端口按 scheme 补默认（https→443、
/// http→80），统一为 `scheme://host:port`。拒绝空串、非 http/https scheme、
/// 通配符、路径、query、fragment、内嵌空白、空 host 与非法端口；返回 `None`。
fn normalize_origin(raw: &str) -> Option<TrustedOrigin> {
    let raw = raw.trim();
    let (scheme, rest) = raw.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    if rest.is_empty()
        || rest.contains('/')
        || rest.contains('*')
        || rest.contains('?')
        || rest.contains('#')
        || rest.chars().any(char::is_whitespace)
    {
        return None;
    }
    let default = default_port(scheme == "https");
    // 拆 host[:port]：IPv6 形如 [::1] / [::1]:5173；否则取最后一个 ':'（无 ':' 即缺端口）。
    let (host, port) = if let Some(stripped) = rest.strip_prefix('[') {
        let (inner, tail) = stripped.split_once(']')?;
        let port = if tail.is_empty() {
            default
        } else {
            tail.strip_prefix(':')?.parse().ok()?
        };
        (format!("[{inner}]"), port)
    } else {
        match rest.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), port.parse().ok()?),
            None => (rest.to_string(), default),
        }
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let authority = format!("{host}:{port}");
    Some(TrustedOrigin {
        origin: format!("{scheme}://{authority}"),
        authority,
        https: scheme == "https",
    })
}

/// scheme 默认端口（https→443、http→80）；缺省端口按此补齐。
fn default_port(https: bool) -> u16 {
    if https { 443 } else { 80 }
}

/// 从 `host:port` authority 取出端口（IPv6 括号形式同样适用）。
fn authority_port(authority: &str) -> Option<u16> {
    authority
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
}

/// 从 `host:port` authority 取出裸 `host`（IPv6 保留括号形式，如 `[::1]`）。
fn bare_host(authority: &str) -> &str {
    match authority.rfind(':') {
        Some(index) => &authority[..index],
        None => authority,
    }
}

/// 收录一条受信 origin 的 Host 允许形式：`host:port` 恒收录；裸 `host` 仅在
/// `allow_bare_host` 为真时额外收录（反代域名的 Host 头不带端口；回环口恒收录）。
fn push_expected_authorities(
    expected_authorities: &mut Vec<String>,
    origin: &TrustedOrigin,
    allow_bare_host: bool,
) {
    expected_authorities.push(origin.authority.clone());
    if allow_bare_host {
        let bare = bare_host(&origin.authority).to_string();
        if !expected_authorities.contains(&bare) {
            expected_authorities.push(bare);
        }
    }
}

/// 解析环境变量原始值为显式 Origin 列表（逗号/空白分隔，空项跳过）。
pub fn parse_trusted_origins_env(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || char::is_whitespace(c))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// 控制面来源校验状态：受信 Origin/Host 允许列表（进程内不可变）。
pub struct ApiAuthState {
    trusted_origins: Vec<TrustedOrigin>,
    expected_authorities: Vec<String>,
}

impl ApiAuthState {
    /// 依据真实 bind 地址构造。loopback bind 默认推导精确回环 origin；非 loopback
    /// bind 必须提供非空 `explicit_origins`（显式配置，不默认推导、不接受 wildcard）。
    pub fn for_bind(
        bound: &SocketAddr,
        explicit_origins: &[String],
    ) -> Result<Self, ApiAuthConfigError> {
        let mut trusted_origins = Vec::new();
        let mut expected_authorities = Vec::new();
        if bound.ip().is_loopback() {
            for host in ["127.0.0.1", "localhost", "[::1]"] {
                let origin = TrustedOrigin {
                    origin: format!("http://{host}:{}", bound.port()),
                    authority: format!("{host}:{}", bound.port()),
                    https: false,
                };
                // 回环口恒收录裸 `host` 形式（生产入口 http://127.0.0.1:4317 的
                // Host 头可能不带端口）；裸形式仅指本机名，无对外放宽面。
                push_expected_authorities(&mut expected_authorities, &origin, true);
                trusted_origins.push(origin);
            }
        }
        for raw in explicit_origins {
            let parsed = parse_trusted_origin(raw)?;
            if !trusted_origins.contains(&parsed) {
                // 端口为 scheme 默认端口时额外收录裸 `host`（反代 https 域名的
                // Host 头不带端口）；非默认端口（如 8443）不放宽裸形式。
                let allow_bare_host =
                    authority_port(&parsed.authority) == Some(default_port(parsed.https));
                push_expected_authorities(&mut expected_authorities, &parsed, allow_bare_host);
                trusted_origins.push(parsed);
            }
        }
        if !bound.ip().is_loopback() && trusted_origins.is_empty() {
            return Err(ApiAuthConfigError::NonLoopbackRequiresExplicitOrigins {
                bind: bound.to_string(),
            });
        }
        Ok(Self {
            trusted_origins,
            expected_authorities,
        })
    }

    /// 受信 Origin 列表（诊断/测试）。
    pub fn trusted_origins(&self) -> &[TrustedOrigin] {
        &self.trusted_origins
    }

    /// 允许的 Host authority 列表（诊断/测试）：每条受信 origin 收录 `host:port`，
    /// 默认端口下额外收录裸 `host`（回环口恒收录裸形式）。
    pub fn expected_authorities(&self) -> &[String] {
        &self.expected_authorities
    }

    /// Origin 是否精确受信（与配置侧共用 [`normalize_origin`] 归一：小写、去
    /// 空白、缺省端口按 scheme 补默认；`null`/任意伪造值不匹配）。
    pub fn origin_allowed(&self, origin: &str) -> bool {
        let Some(normalized) = normalize_origin(origin).map(|parsed| parsed.origin) else {
            return false;
        };
        self.trusted_origins
            .iter()
            .any(|trusted| trusted.origin == normalized)
    }

    /// Host authority 是否允许（DNS rebinding 防线；trim + 小写归一比较）。
    /// 允许形式含 `host:port` 与（默认端口/回环口下的）裸 `host`。
    pub fn host_allowed(&self, host: &str) -> bool {
        let normalized = host.trim().to_ascii_lowercase();
        self.expected_authorities.contains(&normalized)
    }
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

/// 最外层来源校验中间件：在 handler 与 WS upgrade 之前执行。
///
/// 未装配 `api_auth`（进程内测试默认）时直通，保持既有内嵌测试行为；生产
/// `serve_web` 始终装配。顺序：Host → Origin；`GET /api/health` 公开；无任何
/// 会话/令牌要求（owner 2026-10-22 决定），`/api/evidence-query` 的鉴权由
/// handler 既有专用 evidence token 链承担（路由仅回环监听挂载）。
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

/// Host authority 来源错误：缺失、畸形（非 UTF-8）或 Host 与 URI authority 冲突。
/// 三者都按拒绝处理，不做任何“另一来源合法即放行”的降级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostAuthoritySourceError {
    Missing,
    Malformed,
    Conflict,
}

/// 解析用于 Host 校验的 authority（批2 Bug45 F18）。
///
/// HTTP/2 的 `:authority` 由 hyper 映射到请求 URI，不一定落到 `host` 头；
/// 因此仅在 `host` 头**缺失**时回退到 `req.uri().authority()`：
/// - Host 头存在且与 URI authority 冲突 → [`HostAuthoritySourceError::Conflict`]
///   （即使 authority 属于允许列表也拒绝）；
/// - Host 头存在但畸形（非 UTF-8）→ [`HostAuthoritySourceError::Malformed`]
///   （不回退、不因 authority 合法而放行）；
/// - 两者都缺失 → [`HostAuthoritySourceError::Missing`]。
///
/// 两来源同时存在时必须大小写不敏感一致（host 本身大小写不敏感）。
fn request_host_authority<'a>(
    headers: &'a HeaderMap,
    uri: &'a axum::http::Uri,
) -> Result<&'a str, HostAuthoritySourceError> {
    let header_host = match headers.get(header::HOST) {
        Some(value) => Some(
            value
                .to_str()
                .map_err(|_| HostAuthoritySourceError::Malformed)?
                .trim(),
        ),
        None => None,
    };
    let uri_authority = uri.authority().map(|authority| authority.as_str());
    match (header_host, uri_authority) {
        (Some(host), Some(authority)) => {
            if host.eq_ignore_ascii_case(authority) {
                Ok(host)
            } else {
                Err(HostAuthoritySourceError::Conflict)
            }
        }
        (Some(host), None) => Ok(host),
        (None, Some(authority)) => Ok(authority),
        (None, None) => Err(HostAuthoritySourceError::Missing),
    }
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
    // Host 先于 Origin（DNS rebinding 防线）。Host 头缺失时才以
    // HTTP/2 `:authority`（URI authority）回退；冲突/畸形保持拒绝。
    let host_authority = match request_host_authority(&headers, req.uri()) {
        Ok(host) => host.to_string(),
        Err(_) => return forbidden_response("forbidden_host", "request host not allowed"),
    };
    if !auth.host_allowed(&host_authority) {
        return forbidden_response("forbidden_host", "request host not allowed");
    }
    // mutation 与 WS upgrade 必须携带受信精确 Origin（CLI 显式带配置 Origin）；
    // 只读 GET/HEAD/OPTIONS 无 Origin 放行到下一层；非法/`null` Origin 一律拒绝。
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
    // 无会话/令牌要求：`/api/evidence-query` 专用 evidence token 语义（批2 T1
    // 修复轮 2/3 裁定）保持在 handler 侧——token→attempt→role→预算链即门，
    // 路由仅在回环监听挂载；此处来源卫生已照常生效。
    next.run(req).await
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
        // 回环口恒收录裸 host 形式（生产入口 http://127.0.0.1:4317 的 Host 头
        // 可能不带端口）；裸形式仅指本机名。
        assert!(auth.host_allowed("127.0.0.1"));
        assert!(auth.host_allowed("localhost"));
        assert!(auth.host_allowed("[::1]"));
        assert!(!auth.host_allowed("evil.com"));
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
    fn api_auth_rejects_wildcard_paths_and_unknown_schemes() {
        assert!(parse_trusted_origin("http://*").is_err());
        assert!(parse_trusted_origin("http://*.example").is_err());
        assert!(parse_trusted_origin("http://localhost:5173/").is_err());
        assert!(parse_trusted_origin("http://localhost:5173/path").is_err());
        assert!(parse_trusted_origin("ftp://localhost:5173").is_err());
        assert!(parse_trusted_origin("localhost:5173").is_err());
        assert!(parse_trusted_origin("https://host/path").is_err());
        assert!(parse_trusted_origin("https://*").is_err());
        assert!(parse_trusted_origin("ftp://host:21").is_err());
        assert!(parse_trusted_origin("https://host:443/x").is_err());
        assert!(parse_trusted_origin("").is_err());
        assert!(parse_trusted_origin("https://host:").is_err());
        assert!(parse_trusted_origin("https://host:99999").is_err());
        assert!(parse_trusted_origin("https://:443").is_err());
        assert!(parse_trusted_origin("http://[::1]:5173").is_ok());
        assert!(parse_trusted_origin(" https://localhost:5173 ").is_ok());
    }

    /// 端口可省略：按 scheme 取默认（https→443、http→80），配置侧与请求侧
    /// （`origin_allowed`）共用同一归一（trim、小写、补默认端口）后精确比较。
    #[test]
    fn api_auth_origin_port_defaults_by_scheme_when_omitted() {
        // https 无端口 → 归一为 :443（owner 场景：反代 https 域名浏览器发不带端口）。
        let https = parse_trusted_origin("https://arch-code.mikechelazyc.heiyu.space")
            .expect("portless https origin");
        assert_eq!(
            https.origin,
            "https://arch-code.mikechelazyc.heiyu.space:443"
        );
        assert_eq!(https.authority, "arch-code.mikechelazyc.heiyu.space:443");
        assert!(https.https);

        let bind: SocketAddr = "192.168.1.5:4317".parse().expect("bind");
        let auth = ApiAuthState::for_bind(
            &bind,
            &["https://arch-code.mikechelazyc.heiyu.space".to_string()],
        )
        .expect("portless explicit origin accepted");
        assert!(auth.origin_allowed("https://arch-code.mikechelazyc.heiyu.space"));
        assert!(auth.origin_allowed("HTTPS://ARCH-CODE.MIKECHELAZYC.HEIYU.SPACE"));
        assert!(
            auth.origin_allowed("https://arch-code.mikechelazyc.heiyu.space:443"),
            "portless and explicit-default-port forms normalize identically"
        );
        assert!(!auth.origin_allowed("https://arch-code.mikechelazyc.heiyu.space:8443"));
        assert!(auth.host_allowed("arch-code.mikechelazyc.heiyu.space:443"));

        // 请求侧同样拒绝路径/通配符/未知 scheme/空串（非法 Origin 一律 403）。
        assert!(!auth.origin_allowed("https://arch-code.mikechelazyc.heiyu.space/path"));
        assert!(!auth.origin_allowed("https://*"));
        assert!(!auth.origin_allowed("ftp://arch-code.mikechelazyc.heiyu.space:21"));
        assert!(!auth.origin_allowed(""));
        assert!(!auth.origin_allowed("null"));

        // http 无端口 → 归一为 :80。
        let http = parse_trusted_origin("http://dev.example").expect("portless http origin");
        assert_eq!(http.origin, "http://dev.example:80");
        assert_eq!(http.authority, "dev.example:80");
        assert!(!http.https);

        // 显式端口照旧精确；与缺省写法归一后等价。
        let explicit = parse_trusted_origin("https://host.example:443").expect("explicit port");
        assert_eq!(explicit.origin, "https://host.example:443");
        let auth = ApiAuthState::for_bind(&bind, &["https://host.example:443".to_string()])
            .expect("explicit port origin accepted");
        assert!(auth.origin_allowed("https://host.example:443"));
        assert!(auth.origin_allowed("https://host.example"));

        // IPv6 无端口同样补默认端口；缺省写法与显式默认端口归一后是同一条受信项。
        let v6 = parse_trusted_origin("https://[::1]").expect("portless ipv6 origin");
        assert_eq!(v6.origin, "https://[::1]:443");
        let auth = ApiAuthState::for_bind(
            &bind,
            &[
                "https://host.example".to_string(),
                "https://host.example:443".to_string(),
            ],
        )
        .expect("dedup after normalization");
        assert_eq!(auth.trusted_origins().len(), 1);
        // 同一受信 origin 归一后只留一条；其 Host 允许形式为 `host:443` + 裸 `host`。
        assert_eq!(auth.expected_authorities().len(), 2);
        assert!(
            auth.expected_authorities()
                .contains(&"host.example:443".to_string())
        );
        assert!(
            auth.expected_authorities()
                .contains(&"host.example".to_string())
        );
    }

    /// Host 同时接受带端口与不带端口：`host:port` 恒允许；裸 `host` 仅在端口为
    /// scheme 默认端口时额外允许（反代 https 域名的 Host 头不带端口），
    /// 非默认端口（如 8443）不放宽裸形式；回环口恒允许裸形式。
    #[test]
    fn api_auth_host_accepts_both_with_and_without_port() {
        let bind: SocketAddr = "192.168.1.5:4317".parse().expect("bind");

        // 默认端口（https→443）：带端口与不带端口都允许。
        let auth = ApiAuthState::for_bind(
            &bind,
            &["https://arch-code.mikechelazyc.heiyu.space".to_string()],
        )
        .expect("explicit origin accepted");
        assert!(auth.host_allowed("arch-code.mikechelazyc.heiyu.space"));
        assert!(auth.host_allowed("arch-code.mikechelazyc.heiyu.space:443"));
        assert!(!auth.host_allowed("arch-code.mikechelazyc.heiyu.space:8443"));
        assert!(!auth.host_allowed("evil.com"));

        // 非默认端口：不放宽裸形式。
        let auth = ApiAuthState::for_bind(&bind, &["https://h:8443".to_string()])
            .expect("explicit origin accepted");
        assert!(auth.host_allowed("h:8443"));
        assert!(!auth.host_allowed("h"));
        assert!(!auth.host_allowed("h:443"));

        // http 默认端口 80：裸形式同样允许。
        let auth = ApiAuthState::for_bind(&bind, &["http://dev.example".to_string()])
            .expect("explicit origin accepted");
        assert!(auth.host_allowed("dev.example:80"));
        assert!(auth.host_allowed("dev.example"));

        // 生产入口：http://127.0.0.1:4317（回环口）两种 Host 形式都允许。
        let auth = ApiAuthState::for_bind(&loopback_bind(4317), &[]).expect("auth");
        assert!(auth.host_allowed("127.0.0.1:4317"));
        assert!(auth.host_allowed("127.0.0.1"));
        assert!(auth.host_allowed("localhost:4317"));
        assert!(auth.host_allowed("localhost"));
        assert!(auth.host_allowed("[::1]:4317"));
        assert!(auth.host_allowed("[::1]"));
        assert!(!auth.host_allowed("evil.com"));
        assert!(!auth.host_allowed("127.0.0.1:4318"));

        // 无关 authority 一律拒绝。
        assert!(!auth.host_allowed("evil.com:4317"));
        assert!(!auth.host_allowed("evil.example"));
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

    /// F18 四形态：Host-only / authority-only / 冲突 / 非法 Host＋合法 authority。
    #[test]
    fn api_auth_host_authority_source_selection_covers_four_forms() {
        let auth = ApiAuthState::for_bind(&loopback_bind(39721), &[]).expect("auth");
        let path_only: axum::http::Uri = "/api/tasks".parse().expect("path uri");
        let valid_authority: axum::http::Uri = "http://127.0.0.1:39721/api/tasks"
            .parse()
            .expect("absolute uri");
        let conflicting_authority: axum::http::Uri = "http://evil.example:39721/api/tasks"
            .parse()
            .expect("absolute uri");

        // 1) Host-only（无 URI authority）。
        let mut host_only = HeaderMap::new();
        host_only.insert(header::HOST, "127.0.0.1:39721".parse().expect("host"));
        let resolved = request_host_authority(&host_only, &path_only).expect("host-only source");
        assert_eq!(resolved, "127.0.0.1:39721");
        assert!(auth.host_allowed(resolved));

        // 2) authority-only（Host 头缺失 → 回退 URI authority）。
        let no_headers = HeaderMap::new();
        let resolved =
            request_host_authority(&no_headers, &valid_authority).expect("authority-only source");
        assert_eq!(resolved, "127.0.0.1:39721");
        assert!(auth.host_allowed(resolved));

        // 2b) 两来源一致（大小写不敏感）仍按 Host 源放行。
        let mut mixed_case = HeaderMap::new();
        mixed_case.insert(header::HOST, "LOCALHOST:39721".parse().expect("host"));
        let lower_authority: axum::http::Uri = "http://localhost:39721/api/tasks"
            .parse()
            .expect("absolute uri");
        let resolved =
            request_host_authority(&mixed_case, &lower_authority).expect("consistent sources");
        assert_eq!(resolved, "LOCALHOST:39721");
        assert!(auth.host_allowed(resolved));

        // 3) 冲突：Host 与 authority 不同 → 拒绝，不因任一来源合法放行。
        assert_eq!(
            request_host_authority(&host_only, &conflicting_authority),
            Err(HostAuthoritySourceError::Conflict)
        );

        // 4) 非法 Host ＋ 合法 authority → 仍拒绝（不回退）。
        let mut invalid_host = HeaderMap::new();
        invalid_host.insert(header::HOST, "evil.example:39721".parse().expect("host"));
        assert_eq!(
            request_host_authority(&invalid_host, &valid_authority),
            Err(HostAuthoritySourceError::Conflict)
        );

        // 4b) 畸形（非 UTF-8）Host ＋ 合法 authority → 拒绝。
        let mut malformed = HeaderMap::new();
        malformed.insert(
            header::HOST,
            axum::http::HeaderValue::from_bytes(&[0xff, 0xfe]).expect("opaque header value"),
        );
        assert_eq!(
            request_host_authority(&malformed, &valid_authority),
            Err(HostAuthoritySourceError::Malformed)
        );

        // 5) 两来源都缺失 → 拒绝。
        assert_eq!(
            request_host_authority(&no_headers, &path_only),
            Err(HostAuthoritySourceError::Missing)
        );
    }

    /// F18 中间件级：Host-only 与 authority-only 均放行 Host 检查（无会话要求，
    /// 直达 handler），冲突/非法 Host/双缺失在 Host 阶段 403；Origin 门照常。
    #[tokio::test]
    async fn api_auth_middleware_accepts_uri_authority_only_when_host_header_missing() {
        use tower::ServiceExt;

        fn armed_state(root: &std::path::Path) -> WebAppState {
            let auth = Arc::new(ApiAuthState::for_bind(&loopback_bind(39721), &[]).expect("auth"));
            WebAppState::new(
                root.to_path_buf(),
                crate::web::runtime::WebRuntime::new_fake(root.to_path_buf()),
            )
            .with_api_auth(auth)
        }

        async fn status(app: &axum::Router, request: axum::extract::Request) -> StatusCode {
            app.clone()
                .oneshot(request)
                .await
                .expect("response")
                .status()
        }

        let root = tempfile::tempdir().expect("root");
        let state = armed_state(root.path());
        let app = axum::Router::new()
            .route(
                "/api/probe",
                axum::routing::get(|| async { axum::http::StatusCode::OK }),
            )
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                enforce_api_auth,
            ))
            .with_state(state);

        let request = |uri: &str, host: Option<&str>| {
            let mut builder = axum::extract::Request::builder().method("GET").uri(uri);
            if let Some(host) = host {
                builder = builder.header("host", host);
            }
            builder.body(axum::body::Body::empty()).expect("request")
        };

        // 1) Host-only → 通过 Host 校验，无会话直达 handler（200）。
        assert_eq!(
            status(&app, request("/api/probe", Some("127.0.0.1:39721"))).await,
            StatusCode::OK
        );
        // 2) authority-only（HTTP/2 :authority）→ 通过 Host 校验（200）。
        assert_eq!(
            status(&app, request("http://127.0.0.1:39721/api/probe", None)).await,
            StatusCode::OK
        );
        // 3) 冲突 → 403。
        assert_eq!(
            status(
                &app,
                request(
                    "http://evil.example:39721/api/probe",
                    Some("127.0.0.1:39721")
                )
            )
            .await,
            StatusCode::FORBIDDEN
        );
        // 4) 非法 Host ＋ 合法 authority → 403（不回退）。
        assert_eq!(
            status(
                &app,
                request(
                    "http://127.0.0.1:39721/api/probe",
                    Some("evil.example:39721")
                )
            )
            .await,
            StatusCode::FORBIDDEN
        );
        // 5) 双缺失 → 403。
        assert_eq!(
            status(&app, request("/api/probe", None)).await,
            StatusCode::FORBIDDEN
        );
        // 6) 非法 Origin（Host 受信）→ 403。
        let evil_origin = axum::extract::Request::builder()
            .method("GET")
            .uri("/api/probe")
            .header("host", "127.0.0.1:39721")
            .header("origin", "http://evil.example")
            .body(axum::body::Body::empty())
            .expect("evil origin request");
        assert_eq!(status(&app, evil_origin).await, StatusCode::FORBIDDEN);
    }
}
