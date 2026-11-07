// P9（任务10）D 层制品进程 smoke —— **证据合同与制品工具**。
//
// 经 `workspace_ws_integration.rs` include! 进同一模块作用域——本文件以
// `#[path]` 声明为 `ws_process_smoke_contract` 具名子模块（与 C 层
// `provider_run_small_stack/scenario_*.rs` 同一手法），`use super::*` 可达
// 父 harness 的公共导入。
//
// 契约要点（设计 §6.1 D / §6.3）：
// - **制品指纹**：sha256 + profile + 工具链（`sha256sum` / `rustc --version`
//   真实取，不猜）；
// - **生效栈**：读产品启动日志 `[aria-runtime] profile=<p>
//   worker_stack_bytes=<n>`（任务13 的编译期分档自报）——D 层只**读取与交叉
//   核对**（profile 必须与二进制 profile 一致），不自己推断生效栈；
// - **进程收口**：自有进程组 + 持续排日志 + 整组 SIGKILL + wait + ESRCH 残留
//   探测；
// - **真实 WS 客户端**：带可信同源 Origin 的连接与完整 JSON 帧收帧。
// 本文件经 `include!` **文本内联**进 `provider_run_process_smoke.rs` 的
// `process_smoke` 模块体——故直接继承该模块的 `use super::*`（父 harness 公共
// 导入），无需本文件再导一次。

    /// 测试输出，供验收报告摘录）。
    #[derive(Debug)]
    struct ProcessSmokeEvidence {
        binary: String,
        binary_sha256: String,
        profile: &'static str,
        toolchain: String,
        /// 启动时进程环境继承的 RUST_MIN_STACK（如实记录环境值，**不当作生效栈**）。
        inherited_rust_min_stack: Option<String>,
        /// **生效栈**（产品启动日志实测）：`[aria-runtime] profile=<p>
        /// worker_stack_bytes=<n>` —— 产品侧编译期分档自报的真实生效 worker 栈
        /// 字节数（D 层只读取与交叉核对，不自己推断）。
        effective_stack_bytes: Option<u64>,
        /// 产品启动日志自报的 profile（与二进制 profile 交叉核对，防张冠李戴）。
        startup_log_profile: Option<String>,
        server_port: u16,
        stdout_lines: usize,
        stderr_lines: usize,
        /// 无进程残留证据：整组 SIGKILL + wait 收口后进程组 ESRCH 探测结果。
        process_group_residue_free: bool,
    }

    /// 解析产品启动日志里的 `[aria-runtime] profile=<p> worker_stack_bytes=<n>` 行。
    ///
    /// 只认**完整匹配**的行（产品自报的事实，不做前缀猜测）：`profile` 取
    /// debug|release，`worker_stack_bytes` 取十进制字节数。找不到 ⇒ (None, None)
    /// （字段缺失不臆造，由调用方显式判失败）。
    fn parse_startup_stack_line(stderr: &str) -> (Option<String>, Option<u64>) {
        let mut profile = None;
        let mut bytes = None;
        for line in stderr.lines() {
            let Some(rest) = line.trim().strip_prefix("[aria-runtime] ") else {
                continue;
            };
            let mut line_profile = None;
            let mut line_bytes = None;
            for field in rest.split_whitespace() {
                if let Some(value) = field.strip_prefix("profile=") {
                    line_profile = Some(value.to_string());
                } else if let Some(value) = field.strip_prefix("worker_stack_bytes=") {
                    line_bytes = value.parse::<u64>().ok();
                }
            }
            if let (Some(line_profile), Some(line_bytes)) = (line_profile, line_bytes) {
                profile = Some(line_profile);
                bytes = Some(line_bytes);
            }
        }
        (profile, bytes)
    }

    fn binary_sha256(binary: &std::path::Path) -> Result<String, String> {
        let output = std::process::Command::new("sha256sum")
            .arg(binary)
            .output()
            .map_err(|error| format!("sha256sum failed: {error}"))?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        text.split_whitespace()
            .next()
            .map(|digest| digest.to_string())
            .ok_or_else(|| format!("sha256sum produced no digest: {text}"))
    }

    fn rustc_version() -> String {
        std::process::Command::new("rustc")
            .arg("--version")
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// 自有空闲端口：bind :0 取端口号后释放（服务进程随后自 bind）。
    fn acquire_free_port() -> Result<u16, String> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("probe bind failed: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("probe local_addr failed: {error}"))?
            .port();
        drop(listener);
        Ok(port)
    }

    type ProcessWsStream =
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

    /// 带可信同源 Origin 的 WS 连接（生产 serve_web 装配 api_auth：WS upgrade
    /// 必须携带受信精确 Origin——loopback bind 的默认受信来源即 `http://<bind>`）。
    async fn connect_process_ws(url: &str, origin: &str) -> Result<ProcessWsStream, String> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = url
            .into_client_request()
            .map_err(|error| format!("ws request build failed: {error}"))?;
        request.headers_mut().insert(
            "origin",
            tokio_tungstenite::tungstenite::http::HeaderValue::from_str(origin)
                .map_err(|error| format!("origin header failed: {error}"))?,
        );
        connect_async(request)
            .await
            .map_err(|error| format!("connect ws failed: {error}"))
            .map(|(ws, _)| ws)
    }

    /// 收帧并完整解析为 `Value`（serde 失败即失败——不完整帧不是证据）。
    async fn recv_process_frame(ws: &mut ProcessWsStream) -> Result<Value, String> {
        // 真实 adapter 走真实 `claude` CLI 会话：冷启动 + 鉴权 + 首包远超
        // 进程内替身的量级——30s 不足以区分「慢」与「缺前置」，故给足 180s。
        let frame = match timeout(Duration::from_secs(180), ws.next()).await {
            Err(_) => return Err("D 未验证：客户端 180s 读超时（服务无帧）".to_string()),
            Ok(None) => return Err("D 未验证：连接关闭".to_string()),
            Ok(Some(Err(_))) => return Err("D 未验证：非文本/非法帧".to_string()),
            Ok(Some(Ok(message))) => message,
        };
        let Message::Text(text) = frame else {
            return Err(format!("D 未验证：期望文本帧，得到 {frame:?}"));
        };
        serde_json::from_str(&text).map_err(|error| format!("D 未验证：JSON 帧解析失败: {error}"))
    }

    fn process_start_generation_message() -> WsInMessage {
        WsInMessage::StartGeneration {
            provider_config: ProviderConfigSnapshot {
                author: ProviderName::ClaudeCode,
                reviewer: None,
                review_rounds: 0,
                permission_modes:
                    cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            reviewer_enabled: false,
            resume_recovery: None,
        }
    }
