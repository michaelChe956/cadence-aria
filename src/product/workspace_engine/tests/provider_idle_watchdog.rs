// F-19（cadence/notes 2026-09-19 阶段4监控）：story 会话 codex run 楔死——
// app-server 拉起后静默 27min（CPU 零增长/出站 0 连接/事件流恒 137 字），
// provider_start_ledger 为空（start 未登记）、驱动循环 select 无超时臂、
// UI 无 Abort 入口。本组钉引擎侧两件：
// ① legacy 流 provider start 写 provider_start_ledger（含 provider/时间戳）；
// ② provider 会话零活动看门狗：触发后沿既有 Abort/cancel kill 链终止子进程，
//    转可诊断失败态（story/design 面的恢复语义=回到 prepare_context 可重跑，
//    与 coding 面 awaiting_manual_recovery 的 abort-only 语义对照论证见
//    provider_drive.rs 看门狗注释）。
// ③ 生成期 Abort 入口在前端面（web/src/pages/ChatCockpitPage.generation.test.tsx）。

use super::*;

use crate::cross_cutting::streaming_provider::{
    PermissionRequestData, ProviderCommand, ProviderCompletion, ProviderEvent, ProviderSession,
    RiskLevel, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
use crate::product::models::{ProviderName, WorkspaceType};
use crate::product::workspace_engine::provider_drive::{
    PROVIDER_CHOICE_WAIT_TIMEOUT, PROVIDER_IDLE_WATCHDOG_TIMEOUT,
};

/// 持久 story 会话引擎（生产形态 new_persistent，带真实 LifecycleStore）。
fn persistent_story_engine() -> (
    tempfile::TempDir,
    LifecycleStore,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
) {
    let tmp = tempfile::TempDir::new().unwrap();
    let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().join("checkpoints")));
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
        })
        .unwrap();
    let session = WorkspaceSession::from_record(session_record);
    let (tx, rx) = mpsc::channel(64);
    let engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle_store.clone(), tx, session);
    (tmp, lifecycle_store, engine, rx)
}

/// F-19 楔死形态 stub：start 成功（子进程在位的替身），此后事件通道恒开且
/// 静默——零事件、零命令。记录引擎侧发来的会话命令（断言 Abort kill 链）。
struct SilentStreamingProvider {
    received_commands: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
}

impl SilentStreamingProvider {
    fn new() -> (Self, std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>) {
        let received: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        (
            Self {
                received_commands: Arc::clone(&received),
            },
            received,
        )
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SilentStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let sink = Arc::clone(&self.received_commands);
        tokio::spawn(async move {
            while let Some(command) = command_rx.recv().await {
                let label = match command {
                    ProviderCommand::Abort => "abort",
                    ProviderCommand::PermissionResponse { .. } => "permission_response",
                    ProviderCommand::ChoiceResponse { .. } => "choice_response",
                    ProviderCommand::ToolResult(_) => "tool_result",
                };
                sink.lock().unwrap().push(label);
            }
        });
        // 事件通道保持打开且静默（forget 发送端）：驱动循环只能靠看门狗脱困。
        std::mem::forget(event_tx);
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

/// 等待人工权限应答的 stub：发出 PermissionRequest 后事件通道静默（静默期
/// 覆盖看门狗窗口），收到引擎转发的 PermissionResponse 才产出完整 artifact。
/// 钉「等待人工输入不是 provider 楔死，看门狗必须挂起」。
struct PermissionGateStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for PermissionGateStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::PermissionRequest(PermissionRequestData {
                    id: "perm-f19".to_string(),
                    tool_name: "Bash".to_string(),
                    description: "run tests".to_string(),
                    risk_level: RiskLevel::Medium,
                }))
                .await;
            while let Some(command) = command_rx.recv().await {
                if matches!(command, ProviderCommand::PermissionResponse { .. }) {
                    let _ = event_tx
                        .send(ProviderEvent::Completed(ProviderCompletion::plain(
                            complete_story_artifact("F-19 权限等待后完成", "artifact 完整"),
                            Some("provider-session-f19".to_string()),
                        )))
                        .await;
                    break;
                }
            }
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

/// ① F-19：legacy story 流 author run 拉起 provider 时必须登记
/// provider_start_ledger（对照 SC 面 reserve_single_candidate_provider_start
/// 先例）——条目含 provider 名与时间戳，监控据此区分「start 未发生」与
/// 「start 后楔死」。
#[tokio::test]
async fn author_provider_start_registers_ledger_entry_with_provider_and_timestamp() {
    let (_tmp, lifecycle_store, mut engine, mut rx) = persistent_story_engine();
    let session_id = engine.session().session_id.clone();
    let (provider, _received) = SilentStreamingProvider::new();

    // 静默 stub 即可：登记发生在 provider.start 之前，run 由看门狗收口。
    engine
        .handle_user_message(
            "开始生成 story".to_string(),
            Arc::new(provider),
            empty_provider_commands(),
        )
        .await;

    // drain 事件，仅防 channel 满阻塞（handle_user_message 已 await 完成）。
    while rx.try_recv().is_ok() {}

    let durable = lifecycle_store
        .get_workspace_session(&session_id)
        .expect("durable story session");
    assert_eq!(
        durable.provider_start_ledger.len(),
        1,
        "legacy story author run 必须登记 provider start（F-19：ledger 空即无法诊断）"
    );
    let entry = &durable.provider_start_ledger[0];
    assert_eq!(
        entry.provider_start_idempotency_key,
        format!("workspace_author:{session_id}:0"),
        "key 形对照 SC 面 single_candidate_author:{{session}}:{{n}} 先例"
    );
    assert!(entry.started);
    assert_eq!(
        entry.provider.as_deref(),
        Some("codex"),
        "条目必须携带 provider 名（监控按 provider 归因楔死）"
    );
    let started_at = entry.started_at.as_deref().expect("started_at");
    assert!(
        chrono::DateTime::parse_from_rfc3339(started_at).is_ok(),
        "started_at 必须是可解析的 RFC3339 时间戳，实际：{started_at}"
    );

    // 引擎内存镜像与 durable 同步（SessionState wire 投影来源）。
    assert_eq!(engine.session().provider_start_ledger.len(), 1);
}

/// ② F-19：provider 会话零活动看门狗——静默超过窗口后必须沿既有 Abort/cancel
/// kill 链终止 run，转可诊断失败态（失败节点带稳定原因码），会话回到
/// prepare_context 可重跑（story/design 面的恢复语义）。
#[tokio::test]
async fn idle_watchdog_aborts_silent_provider_run_with_diagnosable_failure() {
    let (_tmp, _lifecycle_store, mut engine, mut rx) = persistent_story_engine();
    let (provider, received) = SilentStreamingProvider::new();

    engine
        .handle_user_message(
            "开始生成 story".to_string(),
            Arc::new(provider),
            empty_provider_commands(),
        )
        .await;

    let mut saw_watchdog_error = false;
    let mut saw_prepare = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            EngineEvent::Error { message } if message.contains("provider_idle_watchdog") => {
                saw_watchdog_error = true;
            }
            EngineEvent::StageChange { stage } if stage == "prepare_context" => {
                saw_prepare = true;
            }
            _ => {}
        }
    }
    assert!(
        saw_watchdog_error,
        "看门狗触发必须送出带 provider_idle_watchdog 原因码的 Error 事件"
    );
    assert!(
        saw_prepare,
        "楔死 run 必须回到 prepare_context（story/design 面可重跑恢复语义）"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
    assert_eq!(
        engine.session().session_status,
        crate::product::models::WorkspaceSessionStatus::Open
    );

    // 失败节点携带诊断摘要（时间线留痕）。
    let failed_nodes: Vec<&TimelineNode> = engine
        .timeline_nodes
        .iter()
        .filter(|node| node.status == TimelineNodeStatus::Failed)
        .collect();
    assert!(
        failed_nodes.iter().any(|node| node
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("provider_idle_watchdog"))),
        "失败节点 summary 必须携带看门狗原因码，实际节点：{failed_nodes:?}"
    );

    // Abort 命令必须送达 provider 会话（kill 链第一环；引擎 cancel 触发
    // adapter 侧 cancel.cancelled() → child.kill）。入队后让步几拍，等
    // stub 的命令 drain task 被调度。
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(
        received.lock().unwrap().contains(&"abort"),
        "看门狗必须向 provider 会话发送 Abort 命令，实际收到：{:?}",
        received.lock().unwrap()
    );
}

/// ②b F-19：等待人工权限应答不是 provider 楔死——权限挂起期间（静默 >
/// 看门狗窗口）看门狗不得触发；应答后 run 正常完成。
#[tokio::test]
async fn idle_watchdog_suspends_while_permission_awaits_human_response() {
    let (_tmp, _lifecycle_store, mut engine, mut rx) = persistent_story_engine();
    let (command_tx, command_rx) = mpsc::channel(8);

    let drive = tokio::spawn(async move {
        engine
            .handle_user_message(
                "开始生成 story".to_string(),
                Arc::new(PermissionGateStreamingProvider),
                command_rx,
            )
            .await;
        engine
    });

    // 等 PermissionRequest 到达事件流。
    let mut saw_permission_request = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !saw_permission_request && std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(EngineEvent::PermissionRequest { .. }) => saw_permission_request = true,
            Ok(_) => {}
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
        }
    }
    assert!(saw_permission_request, "stub 必须先发出 PermissionRequest");

    // 权限悬置静默期：必须覆盖看门狗窗口的数倍而不触发。
    tokio::time::sleep(PROVIDER_IDLE_WATCHDOG_TIMEOUT.saturating_mul(4)).await;
    let mut premature_watchdog = false;
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::Error { message } = &event
            && message.contains("provider_idle_watchdog")
        {
            premature_watchdog = true;
        }
    }
    assert!(
        !premature_watchdog,
        "等待人工权限应答期间看门狗不得触发（人工等待由 PERMISSION_TIMEOUT 收口）"
    );

    // 人工应答后 run 正常完成（无看门狗介入）。
    command_tx
        .send(ProviderCommand::PermissionResponse {
            id: "perm-f19".to_string(),
            approved: true,
            reason: None,
        })
        .await
        .expect("send permission response");
    let engine = tokio::time::timeout(std::time::Duration::from_secs(5), drive)
        .await
        .expect("drive completes after permission response")
        .expect("drive task join");

    let mut saw_watchdog_error = false;
    let mut saw_author_confirm = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            EngineEvent::Error { message } if message.contains("provider_idle_watchdog") => {
                saw_watchdog_error = true;
            }
            EngineEvent::StageChange { stage } if stage == "author_confirm" => {
                saw_author_confirm = true;
            }
            _ => {}
        }
    }
    assert!(!saw_watchdog_error, "应答后正常完成，不得出现看门狗错误");
    assert!(
        saw_author_confirm,
        "完整 story artifact 完成后应进入 author_confirm（run 正常收口）"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

/// F-22/F-19b 楔死形态 stub（pi ask_user 实测形态，0482/0483 留档）：
/// 流式文本 → ToolCall(ask_user) → ChoiceRequest(ProviderChoice) → 事件通道
/// 静默等待 choice 应答；收到匹配 ChoiceResponse 才产出完整 artifact。
struct AskUserGateStreamingProvider {
    forwarded_choice_response: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl AskUserGateStreamingProvider {
    fn new() -> (Self, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let forwarded = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        (
            Self {
                forwarded_choice_response: std::sync::Arc::clone(&forwarded),
            },
            forwarded,
        )
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for AskUserGateStreamingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let forwarded = std::sync::Arc::clone(&self.forwarded_choice_response);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: "验收口径存在歧义，按纪律用 ask_user 确认".to_string(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::ToolCall(
                    crate::cross_cutting::streaming_provider::ProviderToolCall {
                        id: "call_ask_user".to_string(),
                        tool_name: "ask_user".to_string(),
                        input: serde_json::json!({
                            "question": "是否把未匹配路由行为纳入 Story？",
                            "options": ["纳入", "排除"],
                        }),
                    },
                ))
                .await;
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "select-ask-1".to_string(),
                    prompt: "是否把未匹配路由行为纳入 Story？".to_string(),
                    options: vec![
                        ChoiceOptionData {
                            id: "纳入".to_string(),
                            label: "纳入".to_string(),
                            description: None,
                        },
                        ChoiceOptionData {
                            id: "排除".to_string(),
                            label: "排除".to_string(),
                            description: None,
                        },
                    ],
                    allow_multiple: false,
                    allow_free_text: true,
                    questions: Vec::new(),
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            // 楔死形态：choice 悬置期间零事件零命令产出（真实 pi 在
            // extension_ui_response 到达前完全静默）。
            while let Some(command) = command_rx.recv().await {
                if let ProviderCommand::ChoiceResponse { id, .. } = &command
                    && id == "select-ask-1"
                {
                    forwarded.store(true, std::sync::atomic::Ordering::SeqCst);
                    let _ = event_tx
                        .send(ProviderEvent::Completed(ProviderCompletion::plain(
                            complete_story_artifact("F-22 choice 应答后完成", "artifact 完整"),
                            Some("provider-session-f22".to_string()),
                        )))
                        .await;
                    break;
                }
            }
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by WorkspaceEngine",
            0,
        ))
    }
}

/// ③ F-22/F-19b（v28 0482/0483 三连楔死实测）：choice 悬置不能永久豁免看门狗
/// ——choice 卡丢失（无人应答）超过等待界后，run 必须转可诊断失败态
/// （原因码 provider_choice_wait_timeout）回到 prepare_context 可重跑，
/// 而非永久楔死（此前 12–15+ 分钟零增长零出站、看门狗按设计挂起永不触发）。
#[tokio::test]
async fn choice_wait_timeout_converts_lost_choice_into_diagnosable_rerunnable_failure() {
    let (_tmp, _lifecycle_store, mut engine, mut rx) = persistent_story_engine();
    let (provider, forwarded) = AskUserGateStreamingProvider::new();

    engine
        .handle_user_message(
            "开始生成 story".to_string(),
            Arc::new(provider),
            empty_provider_commands(),
        )
        .await;

    let mut saw_choice_request = false;
    let mut saw_choice_timeout_error = false;
    let mut saw_watchdog_error = false;
    let mut saw_prepare = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            EngineEvent::ChoiceRequest { id, source, .. } => {
                assert_eq!(id, "select-ask-1");
                assert_eq!(source, ChoiceRequestSource::ProviderChoice);
                saw_choice_request = true;
            }
            EngineEvent::Error { message } => {
                if message.contains("provider_choice_wait_timeout") {
                    saw_choice_timeout_error = true;
                }
                if message.contains("provider_idle_watchdog") {
                    saw_watchdog_error = true;
                }
            }
            EngineEvent::StageChange { stage } if stage == "prepare_context" => {
                saw_prepare = true;
            }
            _ => {}
        }
    }
    assert!(
        saw_choice_request,
        "引擎必须把 provider ChoiceRequest 转发为 EngineEvent（choice 卡的上游）"
    );
    assert!(
        saw_choice_timeout_error,
        "choice 悬置超过等待界必须送出带 provider_choice_wait_timeout 原因码的 Error 事件"
    );
    assert!(
        !saw_watchdog_error,
        "choice 悬置期间零活动看门狗按设计挂起，不得以 provider_idle_watchdog 收口"
    );
    assert!(
        saw_prepare,
        "choice 楔死必须回到 prepare_context（story/design 面可重跑恢复语义）"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
    assert_eq!(
        engine.session().session_status,
        crate::product::models::WorkspaceSessionStatus::Open
    );
    assert!(
        !forwarded.load(std::sync::atomic::Ordering::SeqCst),
        "无人应答场景不应有 choice 应答被转发（choice 卡未达用户）"
    );
    let failed_nodes: Vec<&TimelineNode> = engine
        .timeline_nodes
        .iter()
        .filter(|node| node.status == TimelineNodeStatus::Failed)
        .collect();
    assert!(
        failed_nodes.iter().any(|node| node
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("provider_choice_wait_timeout"))),
        "失败节点 summary 必须携带 choice 等待界原因码，实际节点：{failed_nodes:?}"
    );

    // F-22 恢复语义钉：楔死收口后「重新开始生成」必须能再次起跑并完成
    //（对照 0482：abort/楔死后重跑再次楔死且无任何收口）。第二轮用可完成
    // stub（应答 choice → 完整 artifact）走完整链到 author_confirm。
    // 模拟 ws 层每 run 的 use_run_token 语义（provider_run.rs）：上一轮超界
    // 触发已 cancel 旧 token，新一轮 run 挂载全新 token。
    engine.use_run_token(CancellationToken::new());
    let (command_tx, command_rx) = mpsc::channel(8);
    let (rerun_provider, _rerun_forwarded) = AskUserGateStreamingProvider::new();
    let rerun = tokio::spawn(async move {
        engine
            .handle_user_message(
                "重新开始生成 story".to_string(),
                Arc::new(rerun_provider),
                command_rx,
            )
            .await;
        engine
    });
    let mut saw_rerun_choice = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !saw_rerun_choice && std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(EngineEvent::ChoiceRequest { .. }) => saw_rerun_choice = true,
            Ok(_) => {}
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
        }
    }
    assert!(saw_rerun_choice, "重跑必须再次到达 choice 悬置点");
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "select-ask-1".to_string(),
            selected_option_ids: vec!["排除".to_string()],
            free_text: None,
            answers: Vec::new(),
        })
        .await
        .expect("send choice response");
    let engine = tokio::time::timeout(std::time::Duration::from_secs(5), rerun)
        .await
        .expect("rerun completes after choice response")
        .expect("rerun task join");
    let mut saw_author_confirm = false;
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::StageChange { stage } = event
            && stage == "author_confirm"
        {
            saw_author_confirm = true;
        }
    }
    assert!(
        saw_author_confirm,
        "重跑应答 choice 后必须正常完成到 author_confirm（F-22 恢复语义）"
    );
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

/// ③b F-22/F-19b：choice 在等待界内被应答 → run 正常完成，等待界不得误伤
/// 在场的交互（真实用户看着 choice 卡作答的场景）。
#[tokio::test]
async fn choice_response_within_wait_window_completes_run_without_choice_timeout() {
    let (_tmp, _lifecycle_store, mut engine, mut rx) = persistent_story_engine();
    let (command_tx, command_rx) = mpsc::channel(8);
    let (provider, forwarded) = AskUserGateStreamingProvider::new();

    let drive = tokio::spawn(async move {
        engine
            .handle_user_message("开始生成 story".to_string(), Arc::new(provider), command_rx)
            .await;
        engine
    });

    // 等 choice 卡到达事件流（窗口内）。
    let mut saw_choice_request = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !saw_choice_request && std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(EngineEvent::ChoiceRequest { .. }) => saw_choice_request = true,
            Ok(_) => {}
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
        }
    }
    assert!(saw_choice_request, "stub 必须先发出 ChoiceRequest");

    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "select-ask-1".to_string(),
            selected_option_ids: vec!["排除".to_string()],
            free_text: None,
            answers: Vec::new(),
        })
        .await
        .expect("send choice response");
    let engine = tokio::time::timeout(PROVIDER_CHOICE_WAIT_TIMEOUT.saturating_mul(4), drive)
        .await
        .expect("drive completes after in-window choice response")
        .expect("drive task join");

    assert!(
        forwarded.load(std::sync::atomic::Ordering::SeqCst),
        "引擎必须把 choice 应答转发到 provider 会话命令通道"
    );
    let mut saw_choice_timeout_error = false;
    let mut saw_author_confirm = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            EngineEvent::Error { message } if message.contains("provider_choice_wait_timeout") => {
                saw_choice_timeout_error = true;
            }
            EngineEvent::StageChange { stage } if stage == "author_confirm" => {
                saw_author_confirm = true;
            }
            _ => {}
        }
    }
    assert!(
        !saw_choice_timeout_error,
        "窗口内应答的 choice 不得触发等待界"
    );
    assert!(saw_author_confirm, "应答后 run 应正常完成");
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
}

/// ④ F-19b 覆盖面：story/design review 驱动循环（review/drive.rs）此前完全
/// 无零活动看门狗——reviewer 静默楔死时 run 永久悬置。对照 author 面先例
/// 补看门狗臂：静默超过窗口 → Abort kill 链 + 可诊断失败。
#[tokio::test]
async fn review_idle_watchdog_aborts_silent_reviewer_with_diagnosable_failure() {
    let (_tmp, _lifecycle_store, mut engine, mut rx) = persistent_story_engine();
    engine.session.review_rounds = 1;
    engine.session.artifact = Some(artifact_payload("# Artifact\n\n可用版本"));
    engine.start_review().await;
    let (provider, received) = SilentStreamingProvider::new();

    tokio::time::timeout(
        PROVIDER_IDLE_WATCHDOG_TIMEOUT.saturating_mul(8),
        engine.drive_review_session(Arc::new(provider), empty_provider_commands()),
    )
    .await
    .expect("静默 reviewer 必须由看门狗收口，不得永久悬置");
    let mut saw_watchdog_error = false;
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::Error { message } = event
            && message.contains("provider_idle_watchdog")
        {
            saw_watchdog_error = true;
        }
    }
    assert!(
        saw_watchdog_error,
        "review 看门狗触发必须送出带 provider_idle_watchdog 原因码的 Error 事件"
    );
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::PrepareContext,
        "reviewer 楔死同样回 prepare_context 可重跑"
    );
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert!(
        received.lock().unwrap().contains(&"abort"),
        "review 看门狗必须向 reviewer 会话发送 Abort 命令，实际收到：{:?}",
        received.lock().unwrap()
    );
}

/// ⑤ F-22/F-19b review 面：reviewer choice 悬置同样受等待界收口
///（reviewer 经结构化提问等待用户，卡丢失时不得永久楔死）。
#[tokio::test]
async fn review_choice_wait_timeout_converts_lost_reviewer_choice_into_diagnosable_failure() {
    let (_tmp, _lifecycle_store, mut engine, mut rx) = persistent_story_engine();
    engine.session.review_rounds = 1;
    engine.session.artifact = Some(artifact_payload("# Artifact\n\n可用版本"));
    engine.start_review().await;
    let (provider, _forwarded) = AskUserGateStreamingProvider::new();

    tokio::time::timeout(
        PROVIDER_CHOICE_WAIT_TIMEOUT.saturating_mul(8),
        engine.drive_review_session(Arc::new(provider), empty_provider_commands()),
    )
    .await
    .expect("choice 悬置的 reviewer 必须由等待界收口，不得永久悬置");
    let mut saw_choice_timeout_error = false;
    while let Ok(event) = rx.try_recv() {
        if let EngineEvent::Error { message } = event
            && message.contains("provider_choice_wait_timeout")
        {
            saw_choice_timeout_error = true;
        }
    }
    assert!(
        saw_choice_timeout_error,
        "review choice 等待界触发必须送出带 provider_choice_wait_timeout 原因码的 Error 事件"
    );
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::PrepareContext,
        "reviewer choice 楔死同样回 prepare_context 可重跑"
    );
}
