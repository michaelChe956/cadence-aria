//! LC 专用 validated sync bridge(Task 1b 段①,REQ-LCG-01/02)。
//!
//! 把 gateway 同步栈的 `ValidatedAdapterInput` 桥接到 registry 内真实 streaming
//! adapter 的 `start_validated`:在一个**专用 OS 线程**的自有 current-thread
//! Tokio runtime 内驱动会话到终态,绝不嵌套调用方线程的 `block_on`。同步阻塞
//! (join)到终态是本方法的契约语义;🔴 **async 上下文禁止直接调用**——join
//! 会同步冻结调用方 runtime 的唯一线程(r16/r17 B案现场:全 timer/WS/HTTP
//! 无响应直至外层 kill),异步调用方必须经 `spawn_blocking` 驱动(参见
//! `WorkItemSplitEngine::invoke_provider_via_gateway` 的 B案根修形态)。
//!
//! 输出经现有 completion/sentinel parser 提取 structured output;超时、取消、
//! `Failed`、`PermissionTimeout`、malformed output 全部沿
//! `ProviderAdapterError` 既有错误返回,不用空 JSON/exit 0 兜底。
//!
//! raw `run` fail-closed:LC 同步栈只接受 validated launch,不回退裸同步直连
//! 或 legacy streaming bridge(`run_streaming`)。

use std::sync::Arc;

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::session_launch::ValidatedAdapterInput;
use crate::protocol::contracts::{AdapterInput, AdapterOutput};

/// LC 同步栈的 streaming-to-sync bridge:由 factory 注入 gateway 的
/// `sync_adapter` 位(1c 装配),`run_validated` 内部只调用真实 streaming
/// adapter 的 `start_validated`(validated trait),不触碰裸 `start`。
pub struct GatewaySyncProvider {
    registry: Arc<ProviderRegistry>,
}

impl GatewaySyncProvider {
    pub fn new(registry: Arc<ProviderRegistry>) -> Self {
        Self { registry }
    }
}

impl ProviderAdapter for GatewaySyncProvider {
    /// raw 同步 run fail-closed:裸 `AdapterInput` 不携带 validated policy,
    /// LC 同步栈禁止无政策启动(REQ-LCG-01)。
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Err(ProviderAdapterError::provider_unavailable(
            "lc gateway sync bridge only accepts validated launches; raw sync run is forbidden",
        ))
    }

    /// validated 同步桥接:把 `ValidatedAdapterInput` 适配为 streaming
    /// `start_validated` 并驱动到终态,产出 `AdapterOutput`。
    ///
    /// - **专用 OS 线程 + 自有 current-thread runtime**:本方法在调用方线程
    ///   同步阻塞(join,与既有 `ProviderAdapter::run` 阻塞语义一致),但
    ///   绝不嵌套调用方线程的 `block_on`;
    /// - **run-bound sink**:prepared launch 冻结的 `ProviderLaunchAuditContext`
    ///   在此物化为 `RoleRunBoundAuditSink` 注入 streaming input(无通用
    ///   tool_policy 的角色同样绑定);
    /// - **工具策略按角色矩阵派生**(与 legacy bridge 同源):策略角色带
    ///   `DenyFileWriteBuiltins`,Executor/Handoff 为 `None`;
    /// - **不借 streaming fresh 推导 resume**:同步 input 无 resume id,
    ///   桥接恒 fresh;
    /// - 终态映射:Completed → structured output 经现有 sentinel parser
    ///   (`parse_last_structured_output`,与 `CliProviderAdapter` 同源)提取;
    ///   Failed/ProtocolError/流提前关闭/交互请求 → 既有错误;超时 →
    ///   `ProviderAdapterError::timeout`;PermissionTimeout → 超时族错误;
    ///   malformed structured output → parse/incompatible 错误。不用空
    ///   JSON/exit 0 兜底。
    fn run_validated(
        &self,
        launch: ValidatedAdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
        let (input, policy) = launch.into_parts();
        let provider_name = provider_name_for_type(input.provider_type.clone())?;
        let adapter = self.registry.get(&provider_name).ok_or_else(|| {
            ProviderAdapterError::provider_unavailable(format!(
                "gateway sync bridge: no streaming adapter registered for {provider_name:?}"
            ))
        })?;
        let streaming_input = bridge_streaming_input(&input, policy.launch_audit());
        let validated_streaming =
            crate::cross_cutting::session_launch::ValidatedStreamingProviderInput::new(
                streaming_input,
                policy,
            );
        let timeout = std::time::Duration::from_secs(input.timeout.max(1));

        // 专用 OS 线程 + 自有 current-thread runtime:与调用方 runtime 完全
        // 隔离,join 前不触碰任何 ambient handle。
        let worker = std::thread::Builder::new()
            .name("lc-gateway-sync-bridge".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        ProviderAdapterError::execution_failed(
                            None,
                            String::new(),
                            format!("gateway sync bridge runtime build failed: {error}"),
                            0,
                        )
                    })?;
                runtime.block_on(drive_validated_session(
                    adapter,
                    validated_streaming,
                    timeout,
                ))
            })
            .map_err(|error| {
                ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    format!("gateway sync bridge spawn failed: {error}"),
                    0,
                )
            })?;
        worker.join().map_err(|_| {
            ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "gateway sync bridge worker panicked",
                0,
            )
        })?
    }
}

/// 同步 `AdapterInput` → streaming input 桥接(与 legacy bridge 的字段映射
/// 同源):cwd 独立透传,`working_dir` 仍是 target;prepared launch 的 audit
/// 上下文物化为 run-bound sink;工具策略按角色矩阵派生;恒 fresh。
fn bridge_streaming_input(
    input: &AdapterInput,
    launch_audit: Option<
        &crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext,
    >,
) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
    use crate::cross_cutting::streaming_provider::{
        ProviderPermissionMode, ProviderToolPolicy, StreamingProviderInput,
    };

    let working_dir = input
        .worktree_path
        .as_deref()
        .map(std::path::PathBuf::from)
        .or_else(|| input.working_directory.clone())
        .unwrap_or_else(||
            // 与 legacy bridge 同语义:无 target 时回填进程 cwd。
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
    let tool_policy = match input.role {
        crate::protocol::contracts::AdapterRole::Orchestrator
        | crate::protocol::contracts::AdapterRole::WorkItemSplitter
        | crate::protocol::contracts::AdapterRole::Reviewer => {
            Some(ProviderToolPolicy::deny_file_write_builtins())
        }
        crate::protocol::contracts::AdapterRole::Executor
        | crate::protocol::contracts::AdapterRole::Handoff => None,
    };
    let (workspace_session_id, audit_sink) = match launch_audit {
        Some(context) => (
            Some(context.workspace_session_id.clone()),
            Some(
                crate::cross_cutting::tool_policy_audit::RoleRunBoundAuditSink::new(
                    std::sync::Arc::clone(&context.audit_sink),
                    context.workspace_session_id.clone(),
                    context.role_run_seq,
                )
                .into_sink(),
            ),
        ),
        None => (None, None),
    };
    StreamingProviderInput {
        provider_type: input.provider_type.clone(),
        role: input.role.clone(),
        prompt: input.prompt.clone(),
        working_dir,
        working_directory: input.working_directory.clone(),
        workspace_session_id,
        // 同步栈不携带 resume id;不借 streaming fresh 推导 resume。
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        tool_policy,
        audit_sink,
        structured_output_contract: None,
        env_vars: std::collections::BTreeMap::new(),
        timeout_secs: input.timeout,
        baseline_tree: None,
    }
}

/// 协议 `ProviderType` → registry `ProviderName`(Fail 一律 fail-closed,
/// 不回退其它 provider)。
fn provider_name_for_type(
    provider_type: crate::protocol::contracts::ProviderType,
) -> Result<crate::product::models::ProviderName, ProviderAdapterError> {
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::ProviderType;
    match provider_type {
        ProviderType::ClaudeCode => Ok(ProviderName::ClaudeCode),
        ProviderType::Codex => Ok(ProviderName::Codex),
        ProviderType::Pi => Ok(ProviderName::Pi),
        ProviderType::KimiCode => Ok(ProviderName::KimiCode),
        other => Err(ProviderAdapterError::provider_unavailable(format!(
            "gateway sync bridge: provider {other:?} has no real streaming adapter"
        ))),
    }
}

/// 在 bridge 专用 runtime 内驱动 validated 会话到终态并折叠为
/// `AdapterOutput`。事件消费与 coordinator `consume_turn` 同族,但终态映射
/// 沿 `ProviderAdapterError` 既有错误(同步栈契约)。
async fn drive_validated_session(
    adapter: std::sync::Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter>,
    validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
    timeout: std::time::Duration,
) -> Result<AdapterOutput, ProviderAdapterError> {
    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderCompletion, ProviderEvent, ProviderStatus,
    };
    use crate::cross_cutting::structured_output::parse_last_structured_output;

    let started = std::time::Instant::now();
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut session = adapter.start_validated(validated, cancel.clone()).await?;
    let mut stdout = String::new();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);

    let finish_output = |full_output: String,
                         completion: ProviderCompletion|
     -> Result<AdapterOutput, ProviderAdapterError> {
        let structured_output =
            if let crate::cross_cutting::structured_output::StructuredOutputState::Parsed(value) =
                completion.structured_output
            {
                value
            } else {
                // 现有 sentinel parser(与 CliProviderAdapter 同源):缺失
                // sentinel → parse_error,不用空 JSON 兜底。
                parse_last_structured_output(&full_output)
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("structured output sentinel parse failed: {}", error.message),
                            full_output.clone(),
                            String::new(),
                        )
                    })?
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        ProviderAdapterError::parse_error(
                            "missing structured output sentinel",
                            full_output.clone(),
                            String::new(),
                        )
                    })?
            };
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: full_output,
            stderr: String::new(),
            structured_output: Some(structured_output),
            files_modified: Vec::new(),
            duration_ms: started.elapsed().as_millis() as u64,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
        })
    };

    loop {
        let event = tokio::select! {
            _ = &mut deadline => {
                best_effort_abort(&mut session).await;
                return Err(ProviderAdapterError::timeout(
                    std::mem::take(&mut stdout),
                    String::new(),
                    started.elapsed().as_millis() as u64,
                ));
            }
            _ = cancel.cancelled() => {
                best_effort_abort(&mut session).await;
                return Err(ProviderAdapterError::execution_failed(
                    None,
                    std::mem::take(&mut stdout),
                    "gateway sync bridge session cancelled".to_string(),
                    started.elapsed().as_millis() as u64,
                ));
            }
            event = session.events.recv() => event,
        };
        match event {
            Some(ProviderEvent::TextDelta { content }) => stdout.push_str(&content),
            Some(ProviderEvent::Execution(execution)) => {
                if let Some(output) = execution.output {
                    stdout.push_str(&output);
                }
            }
            Some(ProviderEvent::ToolResult(result)) => stdout.push_str(&result.output),
            Some(ProviderEvent::Completed(completion)) => {
                // completion.full_output 是 adapter 的权威全文(sentinel 所在);
                // 仅有增量流的 adapter(full_output 为空)回退累计增量。
                let full_output = if completion.full_output.is_empty() {
                    stdout
                } else {
                    completion.full_output.clone()
                };
                return finish_output(full_output, completion);
            }
            Some(ProviderEvent::Failed { message }) => {
                return Err(ProviderAdapterError::execution_failed(
                    None,
                    std::mem::take(&mut stdout),
                    format!("provider reported failure: {message}"),
                    started.elapsed().as_millis() as u64,
                ));
            }
            Some(ProviderEvent::ProtocolError { code, message, .. }) => {
                return Err(ProviderAdapterError::parse_error(
                    format!("provider protocol error {code}: {message}"),
                    std::mem::take(&mut stdout),
                    String::new(),
                ));
            }
            Some(ProviderEvent::PermissionTimeout { permission_id }) => {
                return Err(ProviderAdapterError::timeout_with_details(
                    format!("permission request {permission_id} timed out"),
                    std::mem::take(&mut stdout),
                    String::new(),
                    started.elapsed().as_millis() as u64,
                ));
            }
            Some(ProviderEvent::StatusChanged(ProviderStatus::Failed)) => {
                return Err(ProviderAdapterError::execution_failed(
                    None,
                    std::mem::take(&mut stdout),
                    "provider status failed".to_string(),
                    started.elapsed().as_millis() as u64,
                ));
            }
            Some(ProviderEvent::StatusChanged(ProviderStatus::Aborted)) => {
                return Err(ProviderAdapterError::execution_failed(
                    None,
                    std::mem::take(&mut stdout),
                    "provider status aborted".to_string(),
                    started.elapsed().as_millis() as u64,
                ));
            }
            // 同步桥无法承载交互请求:fail-closed,尽力发出 Abort。
            Some(ProviderEvent::PermissionRequest(_)) | Some(ProviderEvent::ChoiceRequest(_)) => {
                best_effort_abort(&mut session).await;
                return Err(ProviderAdapterError::execution_failed(
                    None,
                    std::mem::take(&mut stdout),
                    "provider interaction required; sync bridge cannot serve requests".to_string(),
                    started.elapsed().as_millis() as u64,
                ));
            }
            Some(
                ProviderEvent::StatusChanged(_)
                | ProviderEvent::ToolCall(_)
                | ProviderEvent::UsageReport(_)
                | ProviderEvent::ToolPolicyDecision(_)
                | ProviderEvent::ToolPolicyWarning(_)
                | ProviderEvent::ToolPolicyTerminated(_),
            ) => {}
            None => {
                return Err(ProviderAdapterError::execution_failed(
                    None,
                    std::mem::take(&mut stdout),
                    "provider event stream closed before completion".to_string(),
                    started.elapsed().as_millis() as u64,
                ));
            }
        }
    }
}

/// 尽力向会话发送 Abort(发送失败/超时静默——错误语义由返回的终态错误承载)。
async fn best_effort_abort(
    session: &mut crate::cross_cutting::streaming_provider::ProviderSession,
) {
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        session
            .commands
            .send(crate::cross_cutting::streaming_provider::ProviderCommand::Abort),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::cross_cutting::provider_adapter::ProviderAdapter;
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::cross_cutting::session_launch::ValidatedAdapterInput;
    use crate::cross_cutting::streaming_provider::{
        ProviderCompletion, ProviderEvent, ProviderSession, StreamChunk, StreamingProviderAdapter,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::GatewayRunAudit;
    use crate::product::logical_codebase::LogicalCodebaseManifest;
    use crate::product::logical_codebase::LogicalCodebaseProviderGateway;
    use crate::product::logical_codebase::PolicyTarget;
    use crate::product::logical_codebase::PolicyTargetResolver;
    use crate::product::logical_codebase::ProviderCapability;
    use crate::product::logical_codebase::ProviderCapabilitySource;
    use crate::product::logical_codebase::ProviderDialect;
    use crate::product::logical_codebase::ProviderGatewayError;
    use crate::product::logical_codebase::ProviderRef;
    use crate::product::logical_codebase::SessionLaunchRequest;
    use crate::product::logical_codebase::SessionPolicyAction;
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::{
        AdapterInput, AdapterOutput, AdapterRole, ProviderType, TimeoutStatus,
    };

    use super::GatewaySyncProvider;

    /// 计数型 streaming adapter:分别记录裸 `start`(direct)、legacy bridge
    /// (`run_streaming`)与 validated `start_validated` 的调用数,并观测
    /// validated input 的 cwd/target/sink。会话在启动后延迟发送一段带
    /// sentinel 的完整输出后 Completed。
    struct BridgeCountingStreamingAdapter {
        direct_starts: AtomicUsize,
        legacy_bridge_runs: AtomicUsize,
        validated_starts: AtomicUsize,
        observed_cwd: Mutex<Option<PathBuf>>,
        observed_target: Mutex<Option<PathBuf>>,
        observed_audit_sink: AtomicBool,
        completion_delay: Duration,
        full_output: String,
    }

    impl BridgeCountingStreamingAdapter {
        fn new(completion_delay: Duration, full_output: String) -> Arc<Self> {
            Arc::new(Self {
                direct_starts: AtomicUsize::new(0),
                legacy_bridge_runs: AtomicUsize::new(0),
                validated_starts: AtomicUsize::new(0),
                observed_cwd: Mutex::new(None),
                observed_target: Mutex::new(None),
                observed_audit_sink: AtomicBool::new(false),
                completion_delay,
                full_output,
            })
        }

        /// 永不完成的空会话(裸 `start` 不应被调用,仅提供可返回的最小形态)。
        fn session_stub(&self) -> ProviderSession {
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            ProviderSession {
                events,
                commands,
                native_session_id: None,
            }
        }
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for BridgeCountingStreamingAdapter {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            self.direct_starts.fetch_add(1, Ordering::SeqCst);
            Ok(self.session_stub())
        }

        async fn start_validated(
            &self,
            validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            self.validated_starts.fetch_add(1, Ordering::SeqCst);
            let (input, _launch) = validated.into_parts();
            *self.observed_cwd.lock().expect("bridge cwd probe") = input.working_directory.clone();
            *self.observed_target.lock().expect("bridge target probe") =
                Some(input.working_dir.clone());
            self.observed_audit_sink
                .store(input.audit_sink.is_some(), Ordering::SeqCst);
            // 在当前 runtime(bridge 专用线程的自有 runtime)内驱动延迟完成:
            // 先 TextDelta,后 Completed(plain——structured output 由 bridge 的
            // sentinel parser 从 full_output 提取)。
            let (event_tx, event_rx) = tokio::sync::mpsc::channel(8);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(8);
            let completion_delay = self.completion_delay;
            let full_output = self.full_output.clone();
            tokio::spawn(async move {
                tokio::time::sleep(completion_delay).await;
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: "bridge fixture delta".to_string(),
                    })
                    .await;
                let _ = event_tx
                    .send(ProviderEvent::Completed(ProviderCompletion::plain(
                        full_output,
                        None,
                    )))
                    .await;
            });
            Ok(ProviderSession {
                events: event_rx,
                commands,
                native_session_id: None,
            })
        }

        async fn run_streaming(
            &self,
            _input: &AdapterInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<
            tokio::sync::mpsc::Receiver<StreamChunk>,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            self.legacy_bridge_runs.fetch_add(1, Ordering::SeqCst);
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(rx)
        }
    }

    /// 静态 capability source:launch/resume/write_boundary 恒 Confirmed
    /// (bridge 测试只关心分发与输出保真,能力分格形状沿用既有 fixture)。
    struct BridgeStaticCapabilitySource;

    impl BridgeStaticCapabilitySource {
        fn capability(provider: &ProviderRef) -> ProviderCapability {
            use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
            use crate::product::logical_codebase::policy::ProviderWireDialect;
            use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
            let (adapter_dialect, wire_dialect) = match provider.provider_type {
                crate::product::logical_codebase::ProviderRefType::ClaudeCode => (
                    ProviderDialect::ClaudeCodeCliV1,
                    ProviderWireDialect::ClaudeCodeStreamJson,
                ),
                crate::product::logical_codebase::ProviderRefType::Codex => (
                    ProviderDialect::CodexCliV1,
                    ProviderWireDialect::CodexAppServerRpc,
                ),
                crate::product::logical_codebase::ProviderRefType::Pi => {
                    (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc)
                }
                crate::product::logical_codebase::ProviderRefType::KimiCode => {
                    (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp)
                }
            };
            ProviderCapability {
                provider_type: provider.provider_type,
                version: "1.0.0".to_string(),
                adapter_dialect,
                wire_dialect,
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                action_capability: ProviderActionCapability {
                    action: SessionPolicyAction::PlanningReadOnly,
                    launch: ProviderCapabilityEvidence::Confirmed,
                    resume: ProviderCapabilityEvidence::Confirmed,
                    write_boundary: ProviderCapabilityEvidence::Confirmed,
                    projection_digest: "sha256:bridge-fixture-profile".to_string(),
                    evidence_ref: "probe://bridge-fixture".to_string(),
                },
                trust: ProviderCapabilityEvidence::Confirmed,
            }
        }
    }

    impl ProviderCapabilitySource for BridgeStaticCapabilitySource {
        fn require_supported(
            &self,
            provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Ok(Self::capability(provider))
        }

        fn require_resume_supported(
            &self,
            provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Ok(Self::capability(provider))
        }

        fn require_write_boundary(
            &self,
            provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Ok(Self::capability(provider))
        }

        fn require_root_recipe_supported(
            &self,
            provider: &ProviderRef,
            _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Ok(Self::capability(provider))
        }
    }

    /// gateway 构形占位:bridge fixture 只用 `validate` 产 policy,sync 槽
    /// 不会被触达,给出最小失败 stub。
    struct BridgeStubSyncAdapter;

    impl crate::cross_cutting::provider_adapter::ProviderAdapter for BridgeStubSyncAdapter {
        fn run(
            &self,
            _input: &AdapterInput,
        ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            Err(
                crate::cross_cutting::provider_adapter::ProviderAdapterError::provider_unavailable(
                    "bridge fixture sync adapter is never invoked",
                ),
            )
        }
    }

    /// pass-through target resolver:直接返回请求冻结的 target。
    struct BridgePassThroughResolver;

    impl PolicyTargetResolver for BridgePassThroughResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<PolicyTarget, ProviderGatewayError> {
            Ok(request.target.clone())
        }
    }

    fn bridge_availability_gate() -> std::sync::Arc<ProviderAvailabilityGate> {
        struct AlwaysHealthy(std::sync::Arc<ProviderHealthSnapshot>);
        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> std::sync::Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }
            fn degraded(&self) -> bool {
                false
            }
        }
        let checked_at = chrono::Utc::now();
        let snapshot = std::sync::Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: [ProviderName::ClaudeCode, ProviderName::Codex]
                .into_iter()
                .map(|provider| ProviderHealthEntry {
                    provider,
                    command: "stub".to_string(),
                    available: true,
                    version: Some("1.0".to_string()),
                    reason_code: None,
                    reason: None,
                    checked_at,
                })
                .collect(),
        });
        std::sync::Arc::new(ProviderAvailabilityGate::new(std::sync::Arc::new(
            AlwaysHealthy(snapshot),
        )))
    }

    /// 产出一份 validated policy 供 bridge 测试组装 `ValidatedAdapterInput`
    /// (bridge 不解释 policy 内容,fixture 只需 gateway.validate 能通过)。
    fn bridge_validated_policy(
        paths: &ProductAppPaths,
        canonical_root: &std::path::Path,
        member: &std::path::Path,
    ) -> crate::product::logical_codebase::ValidatedSessionLaunchPolicy {
        let manifest =
            LogicalCodebaseManifest::new("project_0001", canonical_root.to_path_buf(), vec![]);
        let policies = AggregatePolicyArtifactStore::new(paths.clone());
        policies
            .ensure_bootstrap(&manifest)
            .expect("bootstrap policy");
        let gateway = LogicalCodebaseProviderGateway::with_audit(
            policies,
            std::sync::Arc::new(BridgeStaticCapabilitySource),
            std::sync::Arc::new(BridgePassThroughResolver),
            std::sync::Arc::new(ProviderRegistry::new()),
            std::sync::Arc::new(BridgeStubSyncAdapter),
            bridge_availability_gate(),
            std::sync::Arc::new(GatewayRunAudit::new()),
            manifest.provider_context_root.clone(),
        );
        let request = SessionLaunchRequest {
            project_id: "project_0001".to_string(),
            provider: ProviderRef::claude_code("cap_bridge_fixture"),
            action: SessionPolicyAction::PlanningReadOnly,
            target: PolicyTarget::checkout(
                "logical_repo_0001".to_string(),
                "checkout_0001".to_string(),
                member.to_path_buf(),
            ),
            working_directory: canonical_root.to_path_buf(),
            readable_roots: vec![canonical_root.to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:bridge-fixture-config".to_string(),
        };
        gateway.validate(request).expect("bridge fixture policy")
    }

    fn bridge_adapter_input(
        canonical_root: &std::path::Path,
        member: &std::path::Path,
    ) -> AdapterInput {
        AdapterInput {
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::WorkItemSplitter,
            working_directory: Some(canonical_root.to_path_buf()),
            worktree_path: Some(member.to_string_lossy().to_string()),
            provider_stream_log_dir: None,
            prompt: "split work items".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 5,
            max_retries: 1,
        }
    }

    fn bridge_root_and_member() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().expect("tempdir");
        let canonical_root = root.path().to_path_buf();
        let member = canonical_root.join("member_repo");
        std::fs::create_dir_all(&member).expect("member dir");
        (root, canonical_root, member)
    }

    /// 段①(lcg_t01):bridge 只经 `start_validated` 启动真实 streaming adapter,
    /// 不落裸 `start`/legacy bridge;输出结构保真(structured output 经现有
    /// sentinel parser 提取,timeout_status 非超时)。
    #[tokio::test]
    async fn lcg_t01_sync_bridge_uses_validated_start_and_preserves_output() {
        let (root, canonical_root, member) = bridge_root_and_member();
        let paths = ProductAppPaths::new(canonical_root.join(".aria"));

        let adapter = BridgeCountingStreamingAdapter::new(
            Duration::from_millis(30),
            "工作项拆分结果\n<ARIA_STRUCTURED_OUTPUT nonce=\"bridge00001\">{\"nonce\":\"bridge00001\",\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>"
                .to_string(),
        );
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, adapter.clone());
        let bridge = GatewaySyncProvider::new(std::sync::Arc::new(registry));

        let validated = bridge_validated_policy(&paths, &canonical_root, &member);
        let launch =
            ValidatedAdapterInput::new(bridge_adapter_input(&canonical_root, &member), validated);
        let output = bridge
            .run_validated(launch)
            .expect("bridge validated run completes");

        let expected_payload = serde_json::json!({"work_items": []});
        let direct_calls = adapter.direct_starts.load(Ordering::SeqCst);
        let legacy_bridge_calls = adapter.legacy_bridge_runs.load(Ordering::SeqCst);
        let validated_start_calls = adapter.validated_starts.load(Ordering::SeqCst);
        let observed_cwd = adapter
            .observed_cwd
            .lock()
            .expect("bridge cwd probe")
            .clone();
        let observed_target = adapter
            .observed_target
            .lock()
            .expect("bridge target probe")
            .clone();

        assert_eq!(validated_start_calls, 1);
        assert_eq!(direct_calls, 0);
        assert_eq!(legacy_bridge_calls, 0);
        assert_eq!(observed_cwd, Some(canonical_root));
        assert_eq!(
            observed_target,
            Some(member.clone()),
            "streaming working_dir must keep the member target"
        );
        assert_eq!(output.structured_output, Some(expected_payload));
        assert_eq!(output.timeout_status, TimeoutStatus::NotTimedOut);
        assert_eq!(output.exit_code, Some(0));
    }

    /// 段①(lcg_t01):bridge 在专用 OS 线程的自有 runtime 内驱动会话,不嵌套
    /// current-thread `block_on`——在 tokio current-thread 测试上下文直接同步
    /// 调用不 panic/不死锁,且独立的更短计时器先于 provider 完成触达。
    #[tokio::test]
    async fn lcg_t01_bridge_does_not_block_tokio_current_thread() {
        let (root, canonical_root, member) = bridge_root_and_member();
        let paths = ProductAppPaths::new(canonical_root.join(".aria"));

        let adapter = BridgeCountingStreamingAdapter::new(
            Duration::from_millis(400),
            "<ARIA_STRUCTURED_OUTPUT nonce=\"bridge00002\">{\"nonce\":\"bridge00002\",\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>"
                .to_string(),
        );
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, adapter.clone());
        let bridge = GatewaySyncProvider::new(std::sync::Arc::new(registry));

        // 独立 OS 线程计时器:bridge 不得占用/嵌套当前 tokio runtime,
        // 更短计时器必须先于 provider 完成触达(seq 单调记录先后)。
        let seq = std::sync::Arc::new(AtomicUsize::new(0));
        let timer_seq = std::sync::Arc::new(AtomicUsize::new(0));
        let timer_handle = {
            let seq = std::sync::Arc::clone(&seq);
            let timer_seq = std::sync::Arc::clone(&timer_seq);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(80));
                timer_seq.store(seq.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            })
        };

        let validated = bridge_validated_policy(&paths, &canonical_root, &member);
        let launch =
            ValidatedAdapterInput::new(bridge_adapter_input(&canonical_root, &member), validated);
        let output: AdapterOutput = bridge
            .run_validated(launch)
            .expect("bridge must run without nesting block_on");
        let provider_seq = seq.fetch_add(1, Ordering::SeqCst) + 1;
        timer_handle.join().expect("timer thread");

        assert_eq!(output.timeout_status, TimeoutStatus::NotTimedOut);
        let timer_completed_before_provider_finished =
            timer_seq.load(Ordering::SeqCst) < provider_seq;
        assert!(timer_completed_before_provider_finished);
    }

    /// 段③(lcg_t01):prepared launch(gateway `prepare_streaming_launch`/
    /// `prepare_sync_launch` 产出,run-bound audit 上下文已冻结)在
    /// `start_streaming`/`run_sync` 内永不回落裸 `start`/`run`——streaming
    /// 栈唯一经 `start_validated` 分发,sync 栈唯一经 bridge `run_validated`;
    /// direct start/legacy bridge 计数恒 0(冻结断言组 194-204 对应项)。
    #[tokio::test]
    async fn lcg_t01_validated_call_never_falls_back_to_raw_start_or_run() {
        use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
        use crate::product::lifecycle_store::LifecycleStore;
        use crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext;

        let (root, canonical_root, member) = bridge_root_and_member();
        let paths = ProductAppPaths::new(canonical_root.join(".aria"));

        let adapter = BridgeCountingStreamingAdapter::new(
        Duration::from_millis(10),
        "<ARIA_STRUCTURED_OUTPUT nonce=\"rawgrd01\">{\"nonce\":\"rawgrd01\",\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>"
            .to_string(),
    );
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, adapter.clone());
        let registry = std::sync::Arc::new(registry);
        // sync 槽注入 streaming→sync bridge:run_validated 内部从同一 registry
        // 经 `start_validated` 驱动,裸 `run` 通道 fail-closed。
        let sync_bridge = GatewaySyncProvider::new(registry.clone());

        let manifest =
            LogicalCodebaseManifest::new("project_0001", canonical_root.to_path_buf(), vec![]);
        let policies = AggregatePolicyArtifactStore::new(paths.clone());
        policies
            .ensure_bootstrap(&manifest)
            .expect("bootstrap policy");
        let gateway = LogicalCodebaseProviderGateway::with_audit(
            policies,
            std::sync::Arc::new(BridgeStaticCapabilitySource),
            std::sync::Arc::new(BridgePassThroughResolver),
            registry,
            std::sync::Arc::new(sync_bridge),
            bridge_availability_gate(),
            std::sync::Arc::new(GatewayRunAudit::new()),
            manifest.provider_context_root.clone(),
        );

        let request = SessionLaunchRequest {
            project_id: "project_0001".to_string(),
            provider: ProviderRef::claude_code("cap_bridge_fixture"),
            action: SessionPolicyAction::PlanningReadOnly,
            target: PolicyTarget::checkout(
                "logical_repo_0001".to_string(),
                "checkout_0001".to_string(),
                member.to_path_buf(),
            ),
            working_directory: canonical_root.to_path_buf(),
            readable_roots: vec![canonical_root.to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:bridge-fixture-config".to_string(),
        };
        let lifecycle = LifecycleStore::new(paths.clone());
        let audit_context = |role_run_seq: u64| ProviderLaunchAuditContext {
            workspace_session_id: "ws_raw_guard_0001".to_string(),
            role_run_seq,
            audit_sink: std::sync::Arc::new(lifecycle.clone()),
        };

        // streaming 栈:prepare(绑 run-bound sink)→ start_streaming——唯一经
        // `start_validated` 分发,不回落裸 `start`。
        let streaming_input = crate::cross_cutting::streaming_provider::StreamingProviderInput {
            working_directory: Some(canonical_root.clone()),
            baseline_tree: None,
            tool_policy: None,
            audit_sink: None,
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::WorkItemSplitter,
            prompt: "raw guard split prompt".to_string(),
            working_dir: member.clone(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: std::collections::BTreeMap::new(),
            timeout_secs: 30,
        };
        let prepared_streaming = gateway
            .prepare_streaming_launch(streaming_input, request.clone(), audit_context(1))
            .expect("prepare streaming launch");
        let session = gateway
            .start_streaming(
                prepared_streaming,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("start streaming via validated dispatch");
        drop(session);

        // sync 栈:prepare_sync_launch → run_sync——唯一经 bridge
        // `run_validated`,裸 `run` fail-closed。
        let prepared_sync = gateway
            .prepare_sync_launch(
                bridge_adapter_input(&canonical_root, &member),
                request,
                audit_context(2),
            )
            .expect("prepare sync launch");
        let output = gateway
            .run_sync(prepared_sync)
            .expect("run via bridge validated run");

        let validated_start_calls = adapter.validated_starts.load(Ordering::SeqCst);
        let direct_calls = adapter.direct_starts.load(Ordering::SeqCst);
        let legacy_bridge_calls = adapter.legacy_bridge_runs.load(Ordering::SeqCst);
        assert_eq!(
            validated_start_calls, 2,
            "streaming 与 sync 两栈都唯一经 start_validated 分发"
        );
        assert_eq!(direct_calls, 0, "prepared launch 不得回落裸 start");
        assert_eq!(
            legacy_bridge_calls, 0,
            "prepared launch 不得回落 legacy bridge"
        );
        assert_eq!(
            output.structured_output,
            Some(serde_json::json!({"work_items": []}))
        );
        assert_eq!(output.timeout_status, TimeoutStatus::NotTimedOut);
    }

    /// r19 现场回归(lcg 桥):同步桥驱动真实 claude adapter,CLI 子进程在
    /// init 握手后立即死亡(kill -9;同组后台 sleep 持有 stdout 写端使 EOF
    /// 被无限推迟)——桥必须秒级返回错误,不得空等 stage 超时(r19 现场:
    /// bridge 线程 epoll 空等 57min+,ps 已无 claude CLI 子进程,直至 5400s
    /// 外层 SIGKILL)。修前该形态要等到 `AdapterInput.timeout`(生产 3h)。
    #[cfg(unix)]
    #[tokio::test]
    async fn lcg_bridge_returns_promptly_when_cli_child_dies_after_init() {
        use crate::cross_cutting::claude_code_provider::ClaudeCodeProvider;
        use crate::product::lifecycle_store::LifecycleStore;
        use crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext;
        use std::os::unix::fs::PermissionsExt;

        let (_root, canonical_root, member) = bridge_root_and_member();
        let paths = ProductAppPaths::new(canonical_root.join(".aria"));

        // r19 形态 CLI:收到 user 消息后吐 init 行,随后立即 kill -9 自身;
        // 同进程组的 `sleep 300` 继承 stdout 写端,EOF 因此永不到来。
        let script = canonical_root.join("claude_dead_child_fixture.sh");
        std::fs::write(
            &script,
            "#!/usr/bin/env bash\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"type\":\"user\"'* ]]; then\n    echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-bridge-dead-1\"}'\n    sleep 300 &\n    kill -9 $$\n  fi\ndone\n",
        )
        .expect("write dead-child fixture");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod dead-child fixture");

        let adapter = std::sync::Arc::new(ClaudeCodeProvider::new(script).with_version_supplier(
            std::sync::Arc::new(|| Ok("claude 1.0.99-bridge-dead-fixture".to_string())),
        ));
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, adapter);
        let registry = std::sync::Arc::new(registry);
        let sync_bridge = GatewaySyncProvider::new(registry.clone());

        let manifest =
            LogicalCodebaseManifest::new("project_0001", canonical_root.to_path_buf(), vec![]);
        let policies = AggregatePolicyArtifactStore::new(paths.clone());
        policies
            .ensure_bootstrap(&manifest)
            .expect("bootstrap policy");
        let gateway = std::sync::Arc::new(LogicalCodebaseProviderGateway::with_audit(
            policies,
            std::sync::Arc::new(BridgeStaticCapabilitySource),
            std::sync::Arc::new(BridgePassThroughResolver),
            registry,
            std::sync::Arc::new(sync_bridge),
            bridge_availability_gate(),
            std::sync::Arc::new(GatewayRunAudit::new()),
            manifest.provider_context_root.clone(),
        ));

        let request = SessionLaunchRequest {
            project_id: "project_0001".to_string(),
            provider: ProviderRef::claude_code("cap_bridge_fixture"),
            action: SessionPolicyAction::PlanningReadOnly,
            target: PolicyTarget::checkout(
                "logical_repo_0001".to_string(),
                "checkout_0001".to_string(),
                member.to_path_buf(),
            ),
            working_directory: canonical_root.to_path_buf(),
            readable_roots: vec![canonical_root.to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:bridge-fixture-config".to_string(),
        };
        let lifecycle = LifecycleStore::new(paths.clone());
        let audit_context = ProviderLaunchAuditContext {
            workspace_session_id: "ws_bridge_dead_0001".to_string(),
            role_run_seq: 1,
            audit_sink: std::sync::Arc::new(lifecycle.clone()),
        };

        // 桥 deadline 显式拉大(镜像 r19 的 3h 形态):修前形态要等到该
        // deadline 才归,断言窗口(15s)先失败使回归必然红。
        let mut input = bridge_adapter_input(&canonical_root, &member);
        input.timeout = 120;
        let prepared = gateway
            .prepare_sync_launch(input, request, audit_context)
            .expect("prepare sync launch");

        let started = std::time::Instant::now();
        let outcome = tokio::time::timeout(Duration::from_secs(15), async move {
            tokio::task::spawn_blocking(move || gateway.run_sync(prepared))
                .await
                .expect("bridge worker must not panic")
        })
        .await
        .expect("dead CLI child must fail the bridge within seconds, not wait for stage timeout");

        let failure = outcome.expect_err("dead CLI child must fail the sync bridge");
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "bridge must return promptly after child death, took {:?}",
            started.elapsed()
        );
        assert!(
            !failure.to_string().is_empty(),
            "failure must carry the child-death cause"
        );
    }
}
