async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn persisted_workspace_messages(
    root: &std::path::Path,
) -> Vec<cadence_aria::product::models::WorkspaceMessageRecord> {
    LifecycleStore::new(ProductAppPaths::new(root.join(".aria")))
        .get_workspace_session("workspace_session_0001")
        .expect("workspace session")
        .messages
}

async fn send_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    message: &WsInMessage,
) {
    ws.send(Message::Text(
        serde_json::to_string(message).unwrap().into(),
    ))
    .await
    .expect("send ws message");
}

// 退役留档（T5/REQ-RET-02）：accept_author_output 夹具（author_decision 发送）随消息族退役，
// 调用方测试（part_01 choice/review/reconnect 五测）同批退役——choice/reconnect 面
// 由 campaign_stage3 恢复矩阵与 disconnect e2e 承接。

async fn recv_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsOutMessage {
    let message = timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("ws message timeout")
        .expect("ws message")
        .expect("valid ws message");
    match message {
        Message::Text(text) => serde_json::from_str(&text).expect("ws json"),
        other => panic!("expected text ws message, got {other:?}"),
    }
}

async fn recv_until_message_complete(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> String {
    for _ in 0..600 {
        match recv_json(ws).await {
            WsOutMessage::MessageComplete { checkpoint_id, .. } => return checkpoint_id,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("message_complete not received");
}

async fn recv_until_stage(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    expected: &str,
) -> String {
    for _ in 0..40 {
        match recv_json(ws).await {
            WsOutMessage::StageChange { stage } if stage == expected => return stage,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("stage_change {expected} not received");
}

async fn recv_until_session_state(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsOutMessage {
    for _ in 0..40 {
        match recv_json(ws).await {
            state @ WsOutMessage::SessionState { .. } => return state,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("session_state not received");
}

async fn recv_until_stream_chunk(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> String {
    for _ in 0..40 {
        match recv_json(ws).await {
            WsOutMessage::StreamChunk { content, .. } => return content,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("stream_chunk not received");
}

#[derive(Debug)]
struct PermissionRequestSeen {
    id: String,
    tool_name: String,
    description: String,
}

async fn recv_until_permission_request(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> PermissionRequestSeen {
    for _ in 0..40 {
        match recv_json(ws).await {
            WsOutMessage::PermissionRequest {
                id,
                tool_name,
                description,
                ..
            } => {
                return PermissionRequestSeen {
                    id,
                    tool_name,
                    description,
                };
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("permission_request not received");
}

#[derive(Debug)]
// L2 退役（T5/REQ-RET-02）：choice 接收 helper 的 prompt/options/source 字段
// 原服务已退役的 choice 断言族，现仅 id 被消费。
#[allow(dead_code)]
struct ChoiceRequestSeen {
    id: String,
    prompt: String,
    options: Vec<cadence_aria::web::workspace_ws_types::ChoiceOption>,
    source: Option<String>,
}

async fn recv_until_choice_request(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> ChoiceRequestSeen {
    for _ in 0..600 {
        match recv_json(ws).await {
            WsOutMessage::ChoiceRequest {
                id,
                prompt,
                options,
                source,
                ..
            } => {
                return ChoiceRequestSeen {
                    id,
                    prompt,
                    options,
                    source: Some(source),
                };
            }
            WsOutMessage::MessageComplete { .. } => {
                panic!("author question was completed as artifact before choice_request")
            }
            WsOutMessage::StageChange { stage } if stage == "cross_review" => {
                panic!("reviewer started before author choice_request")
            }
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("choice_request not received");
}

async fn recv_until_protocol_error(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WsOutMessage {
    for _ in 0..40 {
        match recv_json(ws).await {
            event @ WsOutMessage::ProtocolError { .. } => return event,
            WsOutMessage::Error { message } => panic!("ws error: {message}"),
            _ => {}
        }
    }
    panic!("protocol_error not received");
}

fn long_message(token: &str) -> String {
    (0..80)
        .map(|idx| format!("{token}_{idx}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn executable_fixture(relative_path: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("fixture metadata {}: {error}", path.display()));
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions)
            .unwrap_or_else(|error| panic!("chmod fixture {}: {error}", path.display()));
    }
    path
}

fn git_repo(root: &std::path::Path) -> PathBuf {
    // REQ-PIB-02：夹具对齐生产不变量（main 分支+初始提交，裸 init 无分支
    // 会令基线解析 fail-closed）；仓库置于 root 下由调用方持有生命周期，
    // 避免 TempDir 早删令 run 期基准分支解析失效。
    // 聚合索引布局规则要求每个成员至少有 1 个可索引源码文件（aggregate_
    // index_member_coverage_failed），初始提交携带真实源码而非空提交。
    let dir = root.join("repo");
    std::fs::create_dir_all(dir.join("src")).expect("create repo src dir");
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").expect("write repo source");
    let status = Command::new("git")
        .args(["init", "--initial-branch", "main"])
        .current_dir(&dir)
        .status()
        .expect("git init");
    assert!(status.success());
    for args in [
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test User"],
        vec!["add", "."],
        vec!["commit", "-m", "fixture baseline"],
    ] {
        let status = Command::new("git")
            .args(&args)
            .current_dir(&dir)
            .status()
            .expect("git fixture");
        assert!(status.success());
    }
    dir
}
