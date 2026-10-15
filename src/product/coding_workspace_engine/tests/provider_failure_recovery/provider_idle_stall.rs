// P4-a（REQ-PTR-IDLE，块4 挂起检测 A5）红测组：coding 共享 provider
// stream 的零活动挂起看门狗。
//
// C3 裁定：不造假时钟（tokio 未启用 `test-util`），用 env 覆写
// `ARIA_CODING_*_IDLE_SECS` 为 ≈1s 短阈值触发挂起（先例
// `tests/group_review_budget.rs::ShardConcurrencyEnvironment`——env 写入经
// 本组静态 mutex 串行化，配置在调用启动时一次读取）。C4 裁定：语义对照
// `workspace_engine/tests/provider_idle_watchdog.rs`（真实活动重置、
// choice/permission 悬置期间不计时、耗尽可诊断收口）。
//
// 覆盖 TDD 验收：
// ①短阈值触发挂起：Timeout 事件 reason=provider_idle_stall＋空闲诊断、
//   role-run Failed、同 provider fresh 重试恰一次；
// ②真实事件重置计时、choice/permission 人工等待暂停、pi_empty_output_retry
//   算新事件；
// ③两次挂起只调 2 次 provider → attempt Blocked＋stalled 门（coder 三动作
//   / reviewer 按阶段动作），不切 provider、不改 transport 3 次预算；
// ④工具 call 未返回用 max(角色界, 工具界)。

use super::*;
use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestData, ChoiceRequestSource, PermissionRequestData,
    ProviderCompletion, RiskLevel,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex as AsyncMutex;

// ---------------------------------------------------------------------------
// env 覆写 helper（先例 group_review_budget.rs：静态 mutex 串行化 + Drop 恢复）
// ---------------------------------------------------------------------------


fn idle_env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(()));
    &LOCK
}

struct IdleSecsEnvironment {
    _guard: std::sync::MutexGuard<'static, ()>,
    originals: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl IdleSecsEnvironment {
    /// 持锁期间覆写给定 idle env 项；Drop 时恢复原值（未设则移除）。
    /// 锁中毒（兄弟用例 panic）不级联——沿用锁内状态即可。
    fn set(overrides: &[(&'static str, &str)]) -> Self {
        let guard = idle_env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let originals = overrides
            .iter()
            .map(|(key, value)| {
                let original = std::env::var_os(key);
                // SAFETY: 本测试组经 idle_env_lock 串行化持有期间独占写这几项 env。
                unsafe { std::env::set_var(key, value) };
                (*key, original)
            })
            .collect();
        Self {
            _guard: guard,
            originals,
        }
    }
}

impl Drop for IdleSecsEnvironment {
    fn drop(&mut self) {
        for (key, original) in &self.originals {
            match original {
                Some(value) => unsafe { std::env::set_var(key, value) },
                // SAFETY: 同上，持锁串行化期间恢复。
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// stub providers
// ---------------------------------------------------------------------------

/// 前 `stalls_before_success` 次 start 后事件通道恒静默（通道开着但零事件，
/// 即「子进程在位但不干活」的楔死形态）；此后正常完成。
struct IdleStallThenCompleteProvider {
    stalls_before_success: usize,
    starts: AtomicUsize,
    inputs: AsyncMutex<Vec<StreamingProviderInput>>,
    output: String,
}

impl IdleStallThenCompleteProvider {
    fn new(stalls_before_success: usize, output: impl Into<String>) -> Self {
        Self {
            stalls_before_success,
            starts: AtomicUsize::new(0),
            inputs: AsyncMutex::new(Vec::new()),
            output: output.into(),
        }
    }

    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    async fn recorded_inputs(&self) -> Vec<StreamingProviderInput> {
        self.inputs.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for IdleStallThenCompleteProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start_no = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        self.inputs.lock().await.push(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        let output = self.output.clone();
        if start_no <= self.stalls_before_success {
            // 楔死形态：event_tx 挂起持有（通道恒开）、永不发事件，直到 cancel。
            tokio::spawn(async move {
                tokio::select! {
                    _ = cancel.cancelled() => {}
                    _ = std::future::pending::<()>() => {}
                }
            });
        } else {
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::Completed(ProviderCompletion::plain(
                        output,
                        Some(format!("provider-session-{start_no}")),
                    )))
                    .await;
            });
        }
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
            "run_streaming is not used by CodingWorkspaceEngine",
            0,
        ))
    }
}

/// 周期性发真实活动事件（Execution，event_id=pi_empty_output_retry，交错
/// TextDelta）再完成——钉「真实事件重置 idle 计时、pi_empty_output_retry
/// 算新事件」。
struct PeriodicActivityProvider {
    rounds: usize,
    interval_ms: u64,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PeriodicActivityProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(4);
        let rounds = self.rounds;
        let interval = Duration::from_millis(self.interval_ms);
        tokio::spawn(async move {
            for round in 0..rounds {
                tokio::time::sleep(interval).await;
                let _ = event_tx
                    .send(ProviderEvent::Execution(ProviderExecutionEvent {
                        event_id: format!("pi_empty_output_retry_{round}"),
                        kind: ProviderExecutionEventKind::Turn,
                        status: ProviderExecutionEventStatus::Running,
                        title: "pi empty output retry".to_string(),
                        detail: None,
                        command: None,
                        cwd: None,
                        output: None,
                        exit_code: None,
                    }))
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: format!("activity round {round}\n"),
                    })
                    .await;
            }
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    "periodic activity completed".to_string(),
                    Some("provider-session-periodic".to_string()),
                )))
                .await;
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
            "run_streaming is not used by CodingWorkspaceEngine",
            0,
        ))
    }
}

/// 发 ChoiceRequest 后事件通道静默等待人工应答；收到转发的
/// ProviderCommand::ChoiceResponse 才完成（choice 悬置期间看门狗必须暂停）。
struct ChoiceGateSilentProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ChoiceGateSilentProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, mut command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "provider_choice_idle".to_string(),
                    prompt: "选择下一步".to_string(),
                    options: vec![ChoiceOptionData {
                        id: "continue".to_string(),
                        label: "继续".to_string(),
                        description: None,
                    }],
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: Vec::new(),
                    source: ChoiceRequestSource::ProviderChoice,
                }))
                .await;
            tokio::select! {
                _ = cancel.cancelled() => {}
                command = command_rx.recv() => {
                    if let Some(ProviderCommand::ChoiceResponse { .. }) = command {
                        let _ = event_tx
                            .send(ProviderEvent::Completed(ProviderCompletion::plain(
                                "choice answered output".to_string(),
                                Some("provider-session-choice".to_string()),
                            )))
                            .await;
                    }
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
            "run_streaming is not used by CodingWorkspaceEngine",
            0,
        ))
    }
}

/// 发 PermissionRequest 后静默等待人工权限应答；收到转发的
/// ProviderCommand::PermissionResponse 才完成（权限悬置期间看门狗必须暂停）。
struct PermissionGateSilentProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for PermissionGateSilentProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, mut command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::PermissionRequest(PermissionRequestData {
                    id: "permission_idle_0001".to_string(),
                    tool_name: "Bash".to_string(),
                    description: "run tests".to_string(),
                    risk_level: RiskLevel::Medium,
                }))
                .await;
            tokio::select! {
                _ = cancel.cancelled() => {}
                command = command_rx.recv() => {
                    if let Some(ProviderCommand::PermissionResponse { .. }) = command {
                        let _ = event_tx
                            .send(ProviderEvent::Completed(ProviderCompletion::plain(
                                "permission approved output".to_string(),
                                Some("provider-session-permission".to_string()),
                            )))
                            .await;
                    }
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
            "run_streaming is not used by CodingWorkspaceEngine",
            0,
        ))
    }
}

/// 发一个 ToolCall 后恒静默（工具 call 未返回形态）——钉「在途工具用
/// max(角色界, 工具界)」。
struct ToolCallThenSilentProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ToolCallThenSilentProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ToolCall(ProviderToolCall {
                    id: "tool_call_idle_0001".to_string(),
                    tool_name: "Bash".to_string(),
                    input: serde_json::json!({"command": "cargo build --release"}),
                }))
                .await;
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = std::future::pending::<()>() => {}
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
            "run_streaming is not used by CodingWorkspaceEngine",
            0,
        ))
    }
}

// ---------------------------------------------------------------------------
// invocation 直调 helper（先例 provider_start_persistence.rs）
// ---------------------------------------------------------------------------

fn invocation_inputs(attempt: &CodingExecutionAttempt) -> (AdapterInput, StreamingProviderInput) {
    let worktree = attempt.worktree_path.clone().expect("worktree path");
    let legacy_input = AdapterInput {
        working_directory: None,
        provider_type: ProviderType::Codex,
        role: AdapterRole::Executor,
        worktree_path: Some(worktree.to_string_lossy().to_string()),
        provider_stream_log_dir: None,
        prompt: "idle watchdog invocation".to_string(),
        context_files: Vec::new(),
        output_schema: "coding_workspace_markdown".to_string(),
        timeout: 30,
        max_retries: 0,
    };
    let input = streaming_input_from_adapter(
        &legacy_input,
        worktree,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    );
    (legacy_input, input)
}

async fn run_coder_invocation(
    engine: &CodingWorkspaceEngine,
    attempt: &CodingExecutionAttempt,
    role_run: &CodingRoleRun,
    provider: &dyn StreamingProviderAdapter,
    command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
) -> ProviderInvocationOutcome {
    let (legacy_input, input) = invocation_inputs(attempt);
    let provider_name = ProviderName::Codex;
    engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt,
            node_id: "coding_node_idle_watchdog",
            role_run: Some(role_run),
            provider,
            legacy_input: &legacy_input,
            input,
            provider_name: &provider_name,
            provider_role: CodingProviderRole::Coder,
            command_rx,
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: true,
            validated_input: None,
        })
        .await
}

fn coder_role_run(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingRoleRun {
    store
        .create_role_run(
            attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_idle_watchdog".to_string()),
        )
        .expect("coder role run")
}

fn idle_stall_timeout_event(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role_run_id: &str,
) -> serde_json::Value {
    store
        .list_role_run_events(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            role_run_id,
        )
        .expect("role run events")
        .into_iter()
        .find(|event| {
            event.event_type == CodingRoleRunEventType::Timeout
                && event
                    .payload
                    .get("reason_code")
                    .and_then(serde_json::Value::as_str)
                    == Some(PROVIDER_IDLE_STALL_REASON_CODE)
        })
        .expect("idle stall timeout event with diagnostics")
        .payload
}

/// 等待 open choice gate 出现（durable 落盘即悬置开始）。
async fn wait_for_open_choice(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    for _ in 0..100 {
        let gates = store
            .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open choice gates");
        if gates.iter().any(|gate| gate.choice_id == "provider_choice_idle") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("choice gate did not open within 5s");
}

// ---------------------------------------------------------------------------
// ⓪ 阈值读取单测：默认 300/600/1800，env 覆写生效、非法值回默认。
// ---------------------------------------------------------------------------

#[test]
fn provider_idle_limits_default_and_env_override() {
    let _env = IdleSecsEnvironment::set(&[
        (ARIA_CODING_CODER_IDLE_SECS_ENV, "7"),
        (ARIA_CODING_REVIEWER_IDLE_SECS_ENV, "9"),
        (ARIA_CODING_TOOL_INFLIGHT_IDLE_SECS_ENV, "11"),
    ]);
    let coder = ProviderIdleLimits::for_role(&CodingProviderRole::Coder);
    assert_eq!(coder.role, Duration::from_secs(7));
    assert_eq!(coder.tool_inflight, Duration::from_secs(11));
    let reviewer = ProviderIdleLimits::for_role(&CodingProviderRole::CodeReviewer);
    assert_eq!(reviewer.role, Duration::from_secs(9));
    let internal = ProviderIdleLimits::for_role(&CodingProviderRole::InternalReviewer);
    assert_eq!(internal.role, Duration::from_secs(9));
}

#[test]
fn provider_idle_limits_fall_back_to_defaults_on_invalid_env() {
    let _env = IdleSecsEnvironment::set(&[
        (ARIA_CODING_CODER_IDLE_SECS_ENV, "not-a-number"),
        (ARIA_CODING_REVIEWER_IDLE_SECS_ENV, "0"),
        (ARIA_CODING_TOOL_INFLIGHT_IDLE_SECS_ENV, "-5"),
    ]);
    let coder = ProviderIdleLimits::for_role(&CodingProviderRole::Coder);
    assert_eq!(coder.role, Duration::from_secs(DEFAULT_CODER_IDLE_SECS));
    let reviewer = ProviderIdleLimits::for_role(&CodingProviderRole::CodeReviewer);
    assert_eq!(
        reviewer.role,
        Duration::from_secs(DEFAULT_REVIEWER_IDLE_SECS)
    );
    assert_eq!(
        coder.tool_inflight,
        Duration::from_secs(DEFAULT_TOOL_INFLIGHT_IDLE_SECS)
    );
}

// ---------------------------------------------------------------------------
// ① 短阈值触发挂起：Timeout 事件＋空闲诊断、role-run Failed、同 provider
//    fresh 重试恰一次后成功继续原流程。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn idle_stall_writes_timeout_event_fails_role_run_and_retries_same_provider_fresh_once() {
    let _env = IdleSecsEnvironment::set(&[(ARIA_CODING_CODER_IDLE_SECS_ENV, "1")]);
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = IdleStallThenCompleteProvider::new(1, "coder output after idle retry");
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    // 红测先行的有界等待：看门狗缺失时静默 provider 会永久挂起。
    let completed = tokio::time::timeout(
        Duration::from_secs(15),
        engine.execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext {
                work_item_markdown: Some("# Idle retry work item\n\nFull context.".to_string()),
                verification_commands: vec!["cargo test --locked --lib idle".to_string()],
            },
            &mut command_rx,
        ),
    )
    .await
    .expect("bounded wait for idle retry cycle")
    .expect("首次 idle 挂起后同 provider fresh 重试成功");

    assert_eq!(completed.status, CodingAttemptStatus::Running);
    assert_eq!(provider.starts(), 2, "首次挂起重试恰一次（同 provider）");
    let inputs = provider.recorded_inputs().await;
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].working_dir, worktree);
    assert_eq!(inputs[1].working_dir, inputs[0].working_dir);
    // fresh 重试：不 resume 挂起前的旧 provider 会话。
    assert_eq!(inputs[1].resume_provider_session_id, None);

    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder role runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Failed);
    assert_eq!(
        runs[0].reason_code.as_deref(),
        Some(PROVIDER_IDLE_STALL_REASON_CODE)
    );
    assert_eq!(runs[0].trigger, CodingRoleRunTrigger::Initial);
    assert_eq!(runs[1].status, CodingRoleRunStatus::Completed);
    assert_eq!(runs[1].trigger, CodingRoleRunTrigger::AutomaticRetry);

    // Timeout 事件 reason=provider_idle_stall＋空闲诊断。
    let payload = idle_stall_timeout_event(&store, &attempt, &runs[0].id);
    assert_eq!(payload["phase"], "provider_stream");
    assert_eq!(payload["role"], "Coder");
    assert!(payload["idle_secs"].as_u64().expect("idle_secs") >= 1);
    assert_eq!(payload["idle_limit_secs"], 1);

    let nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder timeline nodes");
    let coder_nodes = nodes
        .iter()
        .filter(|node| node.stage == CodingExecutionStage::Coding)
        .collect::<Vec<_>>();
    assert_eq!(coder_nodes.len(), 2);
    assert_eq!(coder_nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert_eq!(coder_nodes[1].status, CodingTimelineNodeStatus::Completed);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty(),
        "恢复成功不得落门"
    );
}

// ---------------------------------------------------------------------------
// ③ 两次挂起只调 2 次 provider → attempt Blocked＋coder_provider_stalled 门
//    （三动作），不切 provider、不改 transport 3 次预算。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn second_idle_stall_blocks_attempt_with_coder_provider_stalled_gate() {
    let _env = IdleSecsEnvironment::set(&[(ARIA_CODING_CODER_IDLE_SECS_ENV, "1")]);
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = IdleStallThenCompleteProvider::new(usize::MAX, "never reached");
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = tokio::time::timeout(
        Duration::from_secs(15),
        engine.execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext {
                work_item_markdown: Some("# Idle stall work item".to_string()),
                verification_commands: vec![],
            },
            &mut command_rx,
        ),
    )
    .await
    .expect("bounded wait for two idle stalls")
    .expect_err("两次挂起转 Blocked");

    assert!(
        matches!(
            &error,
            CodingWorkspaceEngineError::ProviderIdleStall { idle_limit_secs, .. }
                if *idle_limit_secs == 1
        ),
        "unexpected error: {error:?}"
    );
    // 两次挂起只调 2 次 provider（不是 transport 的 3 次）、不切 provider。
    assert_eq!(provider.starts(), 2);

    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);

    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    let gate = gates
        .iter()
        .find(|gate| {
            gate.reason_code.as_deref() == Some(CODER_PROVIDER_STALLED_REASON_CODE)
        })
        .expect("coder_provider_stalled gate");
    let action_ids: Vec<&str> = gate
        .available_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect();
    assert_eq!(
        action_ids,
        vec!["retry_coding", "retry_coding_with_context", "abort"],
        "stalled 门三动作（复用块2三按钮卡片）"
    );

    let runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("coder role runs");
    assert_eq!(runs.len(), 2, "纯 idle 最多 2 次调用");
    assert!(runs.iter().all(|run| {
        run.status == CodingRoleRunStatus::Failed
            && run.reason_code.as_deref() == Some(PROVIDER_IDLE_STALL_REASON_CODE)
    }));
}

// ---------------------------------------------------------------------------
// ③ reviewer 面：两次挂起 → code_review_provider_stalled 门（retry_review/
//    abort），attempt Blocked。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reviewer_second_idle_stall_blocks_with_code_review_provider_stalled_gate() {
    let _env = IdleSecsEnvironment::set(&[(ARIA_CODING_REVIEWER_IDLE_SECS_ENV, "1")]);
    let (_root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = IdleStallThenCompleteProvider::new(usize::MAX, "never reached");
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = tokio::time::timeout(
        Duration::from_secs(15),
        engine.execute_code_review_with_commands(&attempt, &provider, &mut command_rx),
    )
    .await
    .expect("bounded wait for reviewer idle stalls")
    .expect_err("reviewer 两次挂起转 Blocked");

    assert!(
        matches!(
            &error,
            CodingWorkspaceEngineError::ProviderIdleStall { idle_limit_secs, .. }
                if *idle_limit_secs == 1
        ),
        "unexpected error: {error:?}"
    );
    assert_eq!(provider.starts(), 2, "reviewer 同 provider 恰调 2 次");

    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);

    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    let gate = gates
        .iter()
        .find(|gate| {
            gate.reason_code.as_deref() == Some(CODE_REVIEW_PROVIDER_STALLED_REASON_CODE)
        })
        .expect("code_review_provider_stalled gate");
    let action_ids: Vec<&str> = gate
        .available_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect();
    assert_eq!(action_ids, vec!["retry_review", "abort"]);
}

// ---------------------------------------------------------------------------
// ② 真实事件重置计时、pi_empty_output_retry 算新事件：周期活动跨越 idle
//    界仍不触发挂起并正常完成。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_events_and_pi_empty_output_retry_reset_idle_timer() {
    let _env = IdleSecsEnvironment::set(&[(ARIA_CODING_CODER_IDLE_SECS_ENV, "1")]);
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = coder_role_run(&store, &attempt);
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = PeriodicActivityProvider {
        rounds: 5,
        interval_ms: 300,
    };
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let outcome = tokio::time::timeout(
        Duration::from_secs(15),
        run_coder_invocation(&engine, &attempt, &role_run, &provider, &mut command_rx),
    )
    .await
    .expect("bounded wait for periodic activity run");

    assert!(
        matches!(outcome, ProviderInvocationOutcome::Completed(_)),
        "周期真实活动（含 pi_empty_output_retry）不得触发挂起: {outcome:?}"
    );
}

// ---------------------------------------------------------------------------
// ② choice 悬置期间不计时：悬置静默超界若干倍后应答仍正常完成。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn choice_wait_suspends_idle_watchdog_until_answered() {
    let _env = IdleSecsEnvironment::set(&[(ARIA_CODING_CODER_IDLE_SECS_ENV, "1")]);
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = coder_role_run(&store, &attempt);
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ChoiceGateSilentProvider;
    let (command_tx, mut command_rx) = mpsc::channel(4);

    let invocation =
        run_coder_invocation(&engine, &attempt, &role_run, &provider, &mut command_rx);
    tokio::pin!(invocation);

    wait_for_open_choice(&store, &attempt).await;
    // choice 悬置静默 2.4s（> 2× idle 界）：看门狗必须暂停不触发。
    tokio::time::sleep(Duration::from_millis(2400)).await;
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        outcome = &mut invocation => {
            panic!("choice 悬置期间看门狗不得收口 run: {outcome:?}");
        }
    }

    command_tx
        .send(CodingRunnerCommand::ChoiceResponse {
            id: "provider_choice_idle".to_string(),
            selected_option_ids: vec!["continue".to_string()],
            free_text: None,
            answers: Vec::new(),
            receipt: None,
        })
        .await
        .expect("send choice response");

    let outcome = tokio::time::timeout(Duration::from_secs(10), invocation)
        .await
        .expect("bounded wait after choice answer");
    assert!(
        matches!(outcome, ProviderInvocationOutcome::Completed(_)),
        "应答后 run 应正常完成: {outcome:?}"
    );
}

// ---------------------------------------------------------------------------
// ② permission 悬置期间不计时：悬置静默超界若干倍后应答仍正常完成。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn permission_wait_suspends_idle_watchdog_until_answered() {
    let _env = IdleSecsEnvironment::set(&[(ARIA_CODING_CODER_IDLE_SECS_ENV, "1")]);
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = coder_role_run(&store, &attempt);
    let (tx, mut rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = PermissionGateSilentProvider;
    let (command_tx, mut command_rx) = mpsc::channel(4);

    let invocation =
        run_coder_invocation(&engine, &attempt, &role_run, &provider, &mut command_rx);
    tokio::pin!(invocation);

    // 等 PermissionRequest 帧送达（悬置开始）。
    let mut permission_seen = false;
    for _ in 0..100 {
        while let Ok(message) = rx.try_recv() {
            if matches!(message, CodingWsOutMessage::CodingPermissionRequest { .. }) {
                permission_seen = true;
            }
        }
        if permission_seen {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(permission_seen, "permission request frame not observed");

    // 权限悬置静默 2.4s（> 2× idle 界）：看门狗必须暂停不触发。
    tokio::time::sleep(Duration::from_millis(2400)).await;
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        outcome = &mut invocation => {
            panic!("permission 悬置期间看门狗不得收口 run: {outcome:?}");
        }
    }

    command_tx
        .send(CodingRunnerCommand::PermissionResponse {
            id: "permission_idle_0001".to_string(),
            approved: true,
            reason: None,
        })
        .await
        .expect("send permission response");

    let outcome = tokio::time::timeout(Duration::from_secs(10), invocation)
        .await
        .expect("bounded wait after permission answer");
    assert!(
        matches!(outcome, ProviderInvocationOutcome::Completed(_)),
        "应答后 run 应正常完成: {outcome:?}"
    );
}

// ---------------------------------------------------------------------------
// ④ 工具 call 未返回用 max(角色界, 工具界)：角色界 1s、工具界 3s，在途
//    工具静默必须按 3s 收口（idle_limit_secs=3、耗时 > 角色界）。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tool_call_without_result_widens_idle_limit_to_tool_inflight_max() {
    let _env = IdleSecsEnvironment::set(&[
        (ARIA_CODING_CODER_IDLE_SECS_ENV, "1"),
        (ARIA_CODING_TOOL_INFLIGHT_IDLE_SECS_ENV, "3"),
    ]);
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = coder_role_run(&store, &attempt);
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ToolCallThenSilentProvider;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let started = tokio::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(15),
        run_coder_invocation(&engine, &attempt, &role_run, &provider, &mut command_rx),
    )
    .await
    .expect("bounded wait for tool inflight stall");

    let ProviderInvocationOutcome::IdleStall {
        idle_secs,
        idle_limit_secs,
        ..
    } = outcome
    else {
        panic!("tool inflight silence must idle-stall");
    };
    assert_eq!(idle_limit_secs, 3, "在途工具静默用 max(角色,工具) 界");
    assert!(idle_secs >= 1);
    assert!(
        started.elapsed() >= Duration::from_millis(2800),
        "必须等满工具界（3s）而非角色界（1s）才收口，实际 {:?}",
        started.elapsed()
    );

    let payload = idle_stall_timeout_event(&store, &attempt, &role_run.id);
    assert_eq!(payload["idle_limit_secs"], 3);
    assert_eq!(payload["open_tool_calls"], 1);
}
