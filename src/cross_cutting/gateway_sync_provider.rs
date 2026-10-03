//! LC 专用 validated sync bridge(Task 1b 段①,REQ-LCG-01/02)。
//!
//! 把 gateway 同步栈的 `ValidatedAdapterInput` 桥接到 registry 内真实 streaming
//! adapter 的 `start_validated`:在一个**专用 OS 线程**的自有 current-thread
//! Tokio runtime 内驱动会话到终态,绝不嵌套调用方线程的 `block_on`(同步调用
//! 语义与既有 `ProviderAdapter::run` 一致,由调用方在 async 上下文直接同步调用
//! 或自行 `spawn_blocking`)。
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

    /// 段① RED 占位:validated 桥接行为由 `lcg_t01_sync_bridge_*` 测试锁定,
    /// 绿阶段实现专用 OS 线程 + 自有 Tokio runtime + completion/sentinel parser。
    fn run_validated(
        &self,
        _launch: ValidatedAdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "gateway sync bridge validated run is not implemented yet",
            0,
        ))
    }
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
            "工作项拆分结果\n<ARIA_STRUCTURED_OUTPUT>{\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>"
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
            "<ARIA_STRUCTURED_OUTPUT>{\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>".to_string(),
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
}
