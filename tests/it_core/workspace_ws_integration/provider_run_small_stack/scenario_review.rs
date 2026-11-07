//! C 层真实 WS 场景（任务10 续）：**送审轮（ReviewOnly）**——真机爆栈所在的轮次。
//!
//! 背景（gdb 实测根因）：崩溃是「深 inbound 链（~1.5 MiB）+ 评审链 poll 帧
//! （drive_body + task_body）」之和超了 tokio worker 的 2 MiB 栈。既有 C 层
//! 场景虽然用了真实 WS 深链 + 2 MiB worker，但**没走到评审链那几个 await 的
//! 深度** ⇒ 绿 ⇒ 门禁抓不到这次崩溃。本场景把门禁真正推进到送审轮。
//!
//! 路径（与 part_02 `http_confirm_on_story_author_confirm_starts_reviewer_then_finalizes`
//! 同一条生产语义，但走真实 socket + 真实 2 MiB worker）：
//! author 轮真实启动 → 产物落 AuthorConfirm 门 → HTTP confirm(with_review=true)
//! 接管本轮 → **ReviewOnly 轮次真实启动 reviewer**（gateway validated start）。
//!
//! 四条证据（设计 §6.1 C 判据）：
//! ① 服务未 abort（`/api/health` + WS 重连）；
//! ② 可信 provider-start 探针（reviewer 替身的真实 validated start 次数）+
//!    durable `provider_start_ledger` 的 reviewer 条目 + ReviewerRun 节点；
//! ③ 完整 JSON 帧与因果偏序（reviewer 的真实 start 先于评审轮首个输出帧）；
//! ④ durable 进展（时间线节点 / 消息 / ledger）。
use super::ws_scenario_support::{
    FrameCollector, REVIEW_ROUND_OUTPUT, connect_ws_with_origin, hello_driver_admission,
    http_health_check, http_post_json, record_evidence, recv_frame_bounded,
    reviewer_start_ledger_keys, serve_ws_app,
};
use super::ws_small_stack::verify_ws_smoke_evidence;
use super::*;

/// 送审轮启动帧：author=ClaudeCode、reviewer=Pi、review_rounds=1、
/// `reviewer_enabled=true`。
///
/// `reviewer_enabled=false` 会把 reviewer 配置清空（`start_generation` 的
/// provider lock 面），送审轮根本不会发生——必须显式开启评审。
fn review_round_start_generation() -> WsInMessage {
    WsInMessage::StartGeneration {
        provider_config: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Pi),
            review_rounds: 1,
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(
            ),
        },
        reviewer_enabled: true,
        resume_recovery: None,
    }
}

/// 送审轮场景执行体（隔离子进程内运行于显式 2 MiB worker）。
///
/// 返回 `Err` 表示场景自身的前置条件/证据不成立；栈溢出不在此处返回——
/// 它以 SIGABRT 终结子进程，由父隔离器捕获并归因「stack overflow」。
pub(super) async fn run_review_round_scenario() -> Result<(), String> {
    let root = tempdir().expect("review round scenario root");
    // 数据根先落证据文件：栈溢出是 SIGABRT ⇒ 子进程被 abort 时 TempDir 不析构、
    // stdout 也可能停在半路——父进程靠这一行找到数据根，读出**崩溃当时**已经
    // 落盘的 durable 证据（ReviewerRun 节点 / provider_start_ledger）。
    record_evidence(&format!(
        "[ws-2mib][review_round] data_root={}",
        root.path().display()
    ));
    // author=ClaudeCode、reviewer=Pi：gateway 对 Codex/KimiCode 强制**用户级
    // workspace trust**（读真实 HOME 的 ~/.codex/config.toml），集成测试写不了
    // 用户 home；Pi 无该前置。且 author/reviewer 替身**各自独立注册** ⇒
    // reviewer 的 provider start 可被独立探针直接观测（不靠 prompt 内容猜）。
    create_workspace_session_fixture_with_providers(&root, "claude_code", "pi", 1).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let session_id = "workspace_session_0001".to_string();
    let author_prompts = Arc::new(Mutex::new(Vec::new()));
    let reviewer_prompts = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ScriptedStreamingProvider::new(
            [VALID_STORY_SPEC],
            author_prompts.clone(),
        )),
    );
    registry.register(
        ProviderName::Pi,
        Arc::new(ScriptedStreamingProvider::new(
            [REVIEW_ROUND_OUTPUT],
            reviewer_prompts.clone(),
        )),
    );
    let app = build_web_router(with_available_provider_health(
        WebAppState::with_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            registry,
        ),
        root.path(),
    ));
    let (addr, server) = serve_ws_app(app).await;
    let url = format!("ws://{addr}/api/workspace-sessions/{session_id}/ws");
    let origin = format!("http://{addr}");

    let mut ws = connect_ws_with_origin(&url, &origin).await;
    let mut author_collector =
        FrameCollector::new(app_paths.clone(), &session_id, author_prompts.clone());
    let initial = author_collector.next(&mut ws).await;
    assert!(
        matches!(initial, WsOutMessage::SessionState { .. }),
        "expected initial session_state, got {initial:?}"
    );
    hello_driver_admission(&mut ws, &session_id).await;

    // ── 阶段一：author 轮（真实 provider start → 产物 → AuthorConfirm 门）──
    send_json(&mut ws, &review_round_start_generation()).await;
    author_collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| matches!(frame, WsOutMessage::StreamChunk { .. }),
            "author 轮首个 stream_chunk（真实 validated start 之后）",
        )
        .await;
    let author_starts = author_prompts.lock().unwrap().len();
    assert!(
        author_starts >= 1,
        "author 轮必须真实 validated start（探针计数）"
    );
    author_collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| matches!(frame, WsOutMessage::MessageComplete { .. }),
            "author 轮 message_complete",
        )
        .await;
    author_collector
        .until(
            &mut ws,
            &|frame: &WsOutMessage| {
                matches!(frame, WsOutMessage::StageChange { stage } if stage == "author_confirm")
            },
            "author 轮终态门态 author_confirm",
        )
        .await;
    record_evidence(&format!(
        "[ws-2mib][review_round] author 轮完成: author_validated_starts={author_starts} frames={:?}",
        author_collector.frames
    ));
    let author_evidence =
        author_collector.into_evidence("review_round_author", author_starts, true);
    verify_ws_smoke_evidence(&author_evidence)
        .map_err(|error| format!("review_round 的 author 轮四条件证据不成立: {error}"))?;

    // ── 阶段二：送审轮（HTTP confirm with_review=true 接管本轮）────────────
    // 评审轮收集器在**送审之前**构造：durable 基线必须落在「送审前」——
    // confirm 自身会建 ReviewerRun 节点，若基线取在送审之后，该节点就被算进
    // 基线，送审轮的 durable 进展就无从观测（而这正是本场景要证明的事实之一）。
    let mut review_collector =
        FrameCollector::new(app_paths.clone(), &session_id, reviewer_prompts.clone());
    // 确认走**真实 socket**（不得用进程内 oneshot 旁路：旁路丢掉真实入站深度）。
    let (status, body) = http_post_json(
        addr,
        &format!("/api/workspace-sessions/{session_id}/confirm"),
        &serde_json::json!({"confirmed_by": "owner", "with_review": true}),
    )
    .await
    .map_err(|error| format!("confirm(with_review=true) 请求失败: {error}"))?;
    assert_eq!(
        status, 200,
        "confirm(with_review=true) 必须被接受（否则送审轮不会发生）: status={status} body={body}"
    );
    record_evidence(&format!(
        "[ws-2mib][review_round] confirm(with_review=true) 已受理: status={status}"
    ));

    // 送审轮观测：reviewer 替身的**真实 validated start**（可信探针）+ durable
    // `provider_start_ledger` 的 reviewer 条目 + durable ReviewerRun 节点。
    // 三者互为独立证据；任何一项成立即证明「真的走进了 ReviewOnly 轮次」。
    let mut reviewer_start_observed = false;
    let mut review_complete_seen = false;
    let mut review_error: Option<String> = None;
    for _ in 0..600 {
        if !reviewer_start_observed {
            let starts = reviewer_prompts.lock().unwrap().len();
            let ledger = reviewer_start_ledger_keys(&app_paths, &session_id);
            if starts >= 1 || !ledger.is_empty() {
                reviewer_start_observed = true;
                record_evidence(&format!(
                    "[ws-2mib][review_round] 送审轮已真实启动: reviewer_validated_starts={starts} durable_provider_start_ledger={ledger:?}"
                ));
            }
        }
        let frame = review_collector.next(&mut ws).await;
        if let WsOutMessage::Error { message } = &frame {
            review_error = Some(message.clone());
        }
        if matches!(frame, WsOutMessage::ReviewComplete { .. }) {
            review_complete_seen = true;
        }
        if reviewer_start_observed && review_complete_seen {
            break;
        }
    }
    let reviewer_starts = reviewer_prompts.lock().unwrap().len();
    let ledger = reviewer_start_ledger_keys(&app_paths, &session_id);
    let nodes = LifecycleStore::new(app_paths.clone())
        .load_timeline_nodes(&session_id)
        .unwrap_or_default();
    let reviewer_node = nodes
        .iter()
        .find(|node| node.node_type == TimelineNodeType::ReviewerRun)
        .map(|node| node.node_id.clone());
    record_evidence(&format!(
        "[ws-2mib][review_round] 送审轮观测: reviewer_validated_starts={reviewer_starts} durable_provider_start_ledger={ledger:?} reviewer_run_node={reviewer_node:?} review_complete={review_complete_seen}"
    ));

    // 证据②：可信 provider-start 探针 + durable ledger + ReviewerRun 节点。
    assert!(
        reviewer_starts >= 1,
        "送审轮必须真的启动 reviewer（可信 validated start 探针计数）——未走到送审轮"
    );
    assert!(
        !ledger.is_empty(),
        "送审轮必须留下 durable provider_start_ledger 的 reviewer 实际启动条目"
    );
    assert!(
        reviewer_node.is_some(),
        "送审轮必须创建 durable ReviewerRun 时间线节点"
    );
    assert!(
        review_error.is_none(),
        "送审轮不得有 WS 错误回显: {review_error:?}"
    );

    // 证据③+④：评审轮自身的四条件（帧完整 / 因果偏序 / durable 进展）。
    let review_evidence = review_collector.into_evidence("review_round", reviewer_starts, false);
    verify_ws_smoke_evidence(&review_evidence)
        .map_err(|error| format!("review_round 的送审轮四条件证据不成立: {error}"))?;

    // 证据①：服务未 abort（同一 socket 的 health + 全新 WS 重连）。
    http_health_check(addr)
        .await
        .map_err(|error| format!("review_round health check failed: {error}"))?;
    let mut reconnected = connect_ws_with_origin(&url, &origin).await;
    let reload = recv_frame_bounded(&mut reconnected)
        .await
        .expect("送审轮场景重连后必须收到 session_state 帧");
    assert!(
        matches!(reload, WsOutMessage::SessionState { .. }),
        "reconnect must yield session_state, got {reload:?}"
    );
    record_evidence(&format!(
        "[ws-2mib][review_round] evidence={review_evidence:?} reviewer_starts={reviewer_starts}"
    ));
    drop(ws);
    drop(reconnected);
    server.abort();
    Ok(())
}
