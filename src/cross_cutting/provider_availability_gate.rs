use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_health::{
    ProviderHealthReasonCode, ProviderHealthService, ProviderHealthSnapshot,
};
use crate::cross_cutting::session_launch::{
    ValidatedAdapterInput, ValidatedStreamingProviderInput,
};
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjector;
use crate::product::models::ProviderName;
use crate::protocol::contracts::{AdapterInput, AdapterOutput};

pub trait ProviderHealthSource: Send + Sync {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot>;
    fn degraded(&self) -> bool;
}

impl ProviderHealthSource for ProviderHealthService {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        ProviderHealthService::snapshot(self)
    }

    fn degraded(&self) -> bool {
        ProviderHealthService::degraded(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("provider_unavailable: {provider:?}: {reason}")]
pub struct ProviderAvailabilityError {
    provider: ProviderName,
    reason: String,
}

impl ProviderAvailabilityError {
    pub fn unavailable(provider: ProviderName, reason: impl Into<String>) -> Self {
        Self {
            provider,
            reason: reason.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        "provider_unavailable"
    }

    pub fn provider(&self) -> &ProviderName {
        &self.provider
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    fn into_adapter_error(self) -> ProviderAdapterError {
        ProviderAdapterError::provider_unavailable(self.to_string())
    }
}

pub trait ProviderHostReadiness: Send + Sync {
    fn ensure_ready(&self, provider: &ProviderName) -> Result<(), ProviderAvailabilityError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AlwaysReadyProviderHost;

impl ProviderHostReadiness for AlwaysReadyProviderHost {
    fn ensure_ready(&self, _provider: &ProviderName) -> Result<(), ProviderAvailabilityError> {
        Ok(())
    }
}

pub struct ProviderAvailabilityGate {
    health: Arc<dyn ProviderHealthSource>,
    host_readiness: Arc<dyn ProviderHostReadiness>,
}

impl ProviderAvailabilityGate {
    pub fn new(health: Arc<dyn ProviderHealthSource>) -> Self {
        Self::with_host_readiness(health, Arc::new(AlwaysReadyProviderHost))
    }

    pub fn with_host_readiness(
        health: Arc<dyn ProviderHealthSource>,
        host_readiness: Arc<dyn ProviderHostReadiness>,
    ) -> Self {
        Self {
            health,
            host_readiness,
        }
    }

    pub fn ensure_available(
        &self,
        provider: &ProviderName,
    ) -> Result<(), ProviderAvailabilityError> {
        if provider == &ProviderName::Fake {
            return Ok(());
        }
        if self.health.degraded() {
            return Err(ProviderAvailabilityError::unavailable(
                provider.clone(),
                "provider health state is degraded",
            ));
        }

        let snapshot = self.health.snapshot();
        let Some(entry) = snapshot.entry(provider) else {
            return Err(ProviderAvailabilityError::unavailable(
                provider.clone(),
                "provider health entry is missing",
            ));
        };
        if !entry.available {
            let reason_code = entry.reason_code.map(reason_code_str).unwrap_or("io_error");
            let reason = entry.reason.as_deref().unwrap_or("provider unavailable");
            return Err(ProviderAvailabilityError::unavailable(
                provider.clone(),
                format!("{reason_code}: {reason}"),
            ));
        }

        self.host_readiness.ensure_ready(provider)
    }
}

fn reason_code_str(reason_code: ProviderHealthReasonCode) -> &'static str {
    match reason_code {
        ProviderHealthReasonCode::CommandMissing => "command_missing",
        ProviderHealthReasonCode::Timeout => "timeout",
        ProviderHealthReasonCode::NonZeroExit => "non_zero_exit",
        ProviderHealthReasonCode::VersionUnparseable => "version_unparseable",
        ProviderHealthReasonCode::VersionTooLow => "version_too_low",
        ProviderHealthReasonCode::IoError => "io_error",
    }
}

pub struct GatedProviderAdapter {
    provider: ProviderName,
    inner: Arc<dyn ProviderAdapter>,
    projector: Option<Arc<dyn ProviderPolicyProjector>>,
    gate: Arc<ProviderAvailabilityGate>,
}

impl GatedProviderAdapter {
    pub fn new(
        provider: ProviderName,
        inner: Arc<dyn ProviderAdapter>,
        gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self::new_with_projector(provider, inner, None, gate)
    }

    /// 带 projector 的原子构造(Task 1a):装饰器透传 projector,取用时
    /// 先复验同一 gate(不可用时拒绝提供)。
    pub fn new_with_projector(
        provider: ProviderName,
        inner: Arc<dyn ProviderAdapter>,
        projector: Option<Arc<dyn ProviderPolicyProjector>>,
        gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self {
            provider,
            inner,
            projector,
            gate,
        }
    }

    /// 透传的 projector(取用时复验同一 gate;未携带 projector 的 legacy
    /// 装饰返回 `None`)。
    pub fn projector(
        &self,
    ) -> Result<Option<Arc<dyn ProviderPolicyProjector>>, ProviderAvailabilityError> {
        self.gate.ensure_available(&self.provider)?;
        Ok(self.projector.clone())
    }
}

impl ProviderAdapter for GatedProviderAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.run(input)
    }

    /// validated 透传(Task 1a):先复验同一 gate,再转发 inner 的
    /// `run_validated`;inner 未接入(默认 unsupported)时错误原样上抛,
    /// 不回落裸 `run`。
    fn run_validated(
        &self,
        input: ValidatedAdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.run_validated(input)
    }
}

pub struct GatedStreamingProviderAdapter {
    provider: ProviderName,
    inner: Arc<dyn StreamingProviderAdapter>,
    projector: Option<Arc<dyn ProviderPolicyProjector>>,
    gate: Arc<ProviderAvailabilityGate>,
}

impl GatedStreamingProviderAdapter {
    pub fn new(
        provider: ProviderName,
        inner: Arc<dyn StreamingProviderAdapter>,
        gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self::new_with_projector(provider, inner, None, gate)
    }

    /// 带 projector 的原子构造(Task 1a):装饰器透传 projector,取用时
    /// 先复验同一 gate(不可用时拒绝提供)。
    pub fn new_with_projector(
        provider: ProviderName,
        inner: Arc<dyn StreamingProviderAdapter>,
        projector: Option<Arc<dyn ProviderPolicyProjector>>,
        gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self {
            provider,
            inner,
            projector,
            gate,
        }
    }

    /// 透传的 projector(取用时复验同一 gate;未携带 projector 的 legacy
    /// 装饰返回 `None`)。
    pub fn projector(
        &self,
    ) -> Result<Option<Arc<dyn ProviderPolicyProjector>>, ProviderAvailabilityError> {
        self.gate.ensure_available(&self.provider)?;
        Ok(self.projector.clone())
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GatedStreamingProviderAdapter {
    fn supports_tool_calls(&self) -> bool {
        self.inner.supports_tool_calls()
    }

    fn legacy_tool_policy_audit_sink(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>>
    {
        self.inner.legacy_tool_policy_audit_sink()
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.start(input, cancel).await
    }

    /// validated 透传(Task 1a):先复验同一 gate,再转发 inner 的
    /// `start_validated`;inner 未接入(默认 unsupported)时错误原样上抛,
    /// 不回落裸 `start`。
    async fn start_validated(
        &self,
        input: ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.start_validated(input, cancel).await
    }

    /// B1-1(波 2)F1:受信资源归属入口必须原样转发——生产 registry 对所有
    /// 真实 provider 用本装饰器包裹,若落到 trait 默认实现,默认面先
    /// `start`(inner 收不到载体),真实子进程永远不会回填 PID/starttime,
    /// 登记停在"预留未确认",而尾部按无身份直接注销(B1 直接臂失效)。
    async fn start_owned(
        &self,
        input: StreamingProviderInput,
        ownership: Option<crate::cross_cutting::provider_resources::TrustedLaunchOwnership>,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.start_owned(input, ownership, cancel).await
    }

    /// Task 3.2(B0a2)透传:先复验同一 gate,再转发 inner 的
    /// `prepare_lc_facts`;inner 未接入(默认 typed unsupported)时错误
    /// 原样上抛,不回落裸 `start`、不伪造 facts。
    async fn prepare_lc_facts(
        &self,
        input: &StreamingProviderInput,
        launch: &crate::product::logical_codebase::provider_gateway::ValidatedSessionLaunchPolicy,
        cancel: CancellationToken,
    ) -> Result<
        crate::product::logical_codebase::provider_projection::ProviderLcPrepareFacts,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.prepare_lc_facts(input, launch, cancel).await
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.run_streaming(input, cancel).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

    use chrono::Utc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::{
        GatedProviderAdapter, GatedStreamingProviderAdapter, ProviderAvailabilityError,
        ProviderAvailabilityGate, ProviderHealthSource, ProviderHostReadiness,
    };
    use crate::cross_cutting::process_manager::{
        ManagedProcessChild, ProcessManager, process_start_time_ticks,
    };
    use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
    use crate::cross_cutting::provider_health::{
        ProviderHealthEntry, ProviderHealthReasonCode, ProviderHealthSnapshot,
    };
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::cross_cutting::provider_resources::{
        OwnedResourceKind, ProviderResourceRecord, ProviderRunResourceScope, ResourceScopeKey,
        StartedIdentity, TrustedLaunchOwnership,
    };
    use crate::cross_cutting::streaming_provider::{
        ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
    };
    use crate::product::logical_codebase::provider_projection::UnprovisionedProviderPolicyProjector;
    use crate::product::models::ProviderName;
    use crate::product::provider_resource_store::ProviderResourceStore;
    use crate::protocol::contracts::{
        AdapterInput, AdapterOutput, AdapterRole, ProviderType, TimeoutStatus,
    };
    use crate::protocol::provider_errors::ProviderErrorCode;

    struct FixedHealthSource {
        snapshot: Arc<ProviderHealthSnapshot>,
        degraded: bool,
    }

    impl ProviderHealthSource for FixedHealthSource {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.snapshot.clone()
        }

        fn degraded(&self) -> bool {
            self.degraded
        }
    }

    fn snapshot(claude_available: bool, codex_available: bool) -> Arc<ProviderHealthSnapshot> {
        let checked_at = Utc::now();
        Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 3,
            checked_at,
            providers: vec![
                ProviderHealthEntry {
                    provider: ProviderName::ClaudeCode,
                    command: "claude --version".to_string(),
                    available: claude_available,
                    version: claude_available.then(|| "1.0".to_string()),
                    reason_code: (!claude_available)
                        .then_some(ProviderHealthReasonCode::CommandMissing),
                    reason: (!claude_available).then(|| "not found".to_string()),
                    checked_at,
                },
                ProviderHealthEntry {
                    provider: ProviderName::Codex,
                    command: "codex --version".to_string(),
                    available: codex_available,
                    version: codex_available.then(|| "1.0".to_string()),
                    reason_code: (!codex_available)
                        .then_some(ProviderHealthReasonCode::CommandMissing),
                    reason: (!codex_available).then(|| "not found".to_string()),
                    checked_at,
                },
            ],
        })
    }

    fn gate(
        claude_available: bool,
        codex_available: bool,
        degraded: bool,
    ) -> Arc<ProviderAvailabilityGate> {
        Arc::new(ProviderAvailabilityGate::new(Arc::new(FixedHealthSource {
            snapshot: snapshot(claude_available, codex_available),
            degraded,
        })))
    }

    #[test]
    fn provider_availability_gate_rejects_explicit_unavailable_provider() {
        let gate = gate(false, true, false);

        let error = gate
            .ensure_available(&ProviderName::ClaudeCode)
            .expect_err("Claude must be rejected");

        assert_eq!(error.code(), "provider_unavailable");
        assert!(error.to_string().contains("command_missing"));
        assert!(gate.ensure_available(&ProviderName::Codex).is_ok());
    }

    #[test]
    fn provider_availability_gate_fails_closed_when_health_storage_is_degraded() {
        let gate = gate(true, true, true);

        let error = gate
            .ensure_available(&ProviderName::Codex)
            .expect_err("degraded storage must fail closed");

        assert_eq!(error.code(), "provider_unavailable");
        assert!(error.to_string().contains("degraded"));
    }

    #[test]
    fn provider_availability_gate_always_allows_fake_for_tests() {
        let gate = gate(false, false, true);

        assert!(gate.ensure_available(&ProviderName::Fake).is_ok());
    }

    struct BlockedHost;

    impl ProviderHostReadiness for BlockedHost {
        fn ensure_ready(&self, provider: &ProviderName) -> Result<(), ProviderAvailabilityError> {
            Err(ProviderAvailabilityError::unavailable(
                provider.clone(),
                "host_not_ready",
            ))
        }
    }

    #[test]
    fn provider_availability_gate_composes_host_readiness_seam() {
        let source = Arc::new(FixedHealthSource {
            snapshot: snapshot(true, true),
            degraded: false,
        });
        let gate = ProviderAvailabilityGate::with_host_readiness(source, Arc::new(BlockedHost));

        let error = gate
            .ensure_available(&ProviderName::ClaudeCode)
            .expect_err("host readiness must participate");

        assert!(error.to_string().contains("host_not_ready"));
    }

    struct RecordingSyncProvider(AtomicUsize);

    impl ProviderAdapter for RecordingSyncProvider {
        fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    fn adapter_input() -> AdapterInput {
        AdapterInput {
            env_vars: Default::default(),
            working_directory: None,
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Executor,
            worktree_path: None,
            provider_stream_log_dir: None,
            prompt: "probe".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 1,
            max_retries: 0,
        }
    }

    #[test]
    fn provider_availability_gate_wraps_sync_provider_adapter() {
        let inner = Arc::new(RecordingSyncProvider(AtomicUsize::new(0)));
        let allowed = GatedProviderAdapter::new(
            ProviderName::ClaudeCode,
            inner.clone(),
            gate(true, true, false),
        );
        allowed.run(&adapter_input()).expect("allowed run");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);

        let blocked = GatedProviderAdapter::new(
            ProviderName::ClaudeCode,
            inner.clone(),
            gate(false, true, false),
        );
        let error = blocked
            .run(&adapter_input())
            .expect_err("blocked run must fail");
        assert_eq!(error.code, ProviderErrorCode::ProviderUnavailable);
        assert_eq!(error.code.as_str(), "provider_unavailable");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);
    }

    struct RecordingStreamingProvider(AtomicUsize);

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for RecordingStreamingProvider {
        fn supports_tool_calls(&self) -> bool {
            true
        }

        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let (_event_tx, events) = mpsc::channel(1);
            let (commands, _command_rx) = mpsc::channel(1);
            Ok(ProviderSession {
                events,
                commands,
                native_session_id: None,
            })
        }
    }

    fn streaming_input() -> StreamingProviderInput {
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy: None,
            audit_sink: None,
            provider_type: ProviderType::Codex,
            role: AdapterRole::Executor,
            prompt: "probe".to_string(),
            working_dir: std::env::current_dir().expect("cwd"),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: Default::default(),
            timeout_secs: 1,
        }
    }

    #[tokio::test]
    async fn provider_availability_gate_wraps_streaming_provider_adapter() {
        let inner = Arc::new(RecordingStreamingProvider(AtomicUsize::new(0)));
        let allowed = GatedStreamingProviderAdapter::new(
            ProviderName::Codex,
            inner.clone(),
            gate(true, true, false),
        );
        assert!(allowed.supports_tool_calls());
        allowed
            .start(streaming_input(), CancellationToken::new())
            .await
            .expect("allowed start");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);

        let blocked = GatedStreamingProviderAdapter::new(
            ProviderName::Codex,
            inner.clone(),
            gate(true, false, false),
        );
        let error = blocked
            .start(streaming_input(), CancellationToken::new())
            .await
            .err()
            .expect("blocked start must fail");
        assert_eq!(error.code, ProviderErrorCode::ProviderUnavailable);
        assert_eq!(error.code.as_str(), "provider_unavailable");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);
    }

    /// B1 审查 F1 回归探针:inner adapter 覆写 `start_owned`,在收到受信
    /// 归属后自行预留/真实 spawn/回填 PID+starttime。门禁装饰器必须原样
    /// 转发 `start_owned`(而不是回落 trait 默认 → `start`),否则生产
    /// coder/reviewer 的真实进程永远不会被回填身份。
    struct OwnershipForwardingProbe {
        start_owned_calls: AtomicUsize,
        start_calls: AtomicUsize,
        saw_ownership: AtomicBool,
        child_pid: AtomicU32,
        child: Mutex<Option<ManagedProcessChild>>,
    }

    impl OwnershipForwardingProbe {
        fn new() -> Self {
            Self {
                start_owned_calls: AtomicUsize::new(0),
                start_calls: AtomicUsize::new(0),
                saw_ownership: AtomicBool::new(false),
                child_pid: AtomicU32::new(0),
                child: Mutex::new(None),
            }
        }

        fn child_pid(&self) -> u32 {
            self.child_pid.load(Ordering::SeqCst)
        }

        /// 终止并回收真实子进程(测试清理;注销在调用方 `release_after_session_tail`)。
        async fn terminate_child(&self) {
            let child = self.child.lock().expect("probe child lock").take();
            if let Some(mut child) = child {
                child.terminate().await;
            }
        }
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for OwnershipForwardingProbe {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.start_calls.fetch_add(1, Ordering::SeqCst);
            Err(ProviderAdapterError::execution_failed(
                None,
                String::new(),
                "gated adapter must forward start_owned, not fall back to bare start",
                0,
            ))
        }

        async fn start_owned(
            &self,
            _input: StreamingProviderInput,
            ownership: Option<TrustedLaunchOwnership>,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.start_owned_calls.fetch_add(1, Ordering::SeqCst);
            let Some(ownership) = ownership else {
                return Err(ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    "probe expected a forwarded ownership carrier",
                    0,
                ));
            };
            self.saw_ownership.store(true, Ordering::SeqCst);
            let reservation = ownership.reserve()?;
            let process = ProcessManager::spawn(
                "/bin/sleep",
                &["30"],
                &std::env::temp_dir(),
                &BTreeMap::new(),
                CancellationToken::new(),
            )
            .await?;
            let pid = process
                .child
                .spawn_leader_pid()
                .ok_or_else(|| ProviderAdapterError::provider_unavailable("probe child pid"))?;
            ownership.backfill_spawned_identity(&reservation, pid)?;
            self.child_pid.store(pid, Ordering::SeqCst);
            *self.child.lock().expect("probe child lock") = Some(process.child);
            let (_events_tx, events) = mpsc::channel(1);
            let (commands, _commands_rx) = mpsc::channel(1);
            Ok(ProviderSession {
                native_session_id: None,
                events,
                commands,
            })
        }
    }

    fn process_records(
        store_root: &std::path::Path,
        key: &ResourceScopeKey,
    ) -> Vec<ProviderResourceRecord> {
        ProviderResourceStore::new(store_root)
            .list_scope(key)
            .expect("list gate b1 scope")
            .into_iter()
            .filter(|record| matches!(record.kind, OwnedResourceKind::Process))
            .collect()
    }

    /// 生产装配路径(`register_gated`)必须把 `start_owned` 原样转发给被
    /// 装饰 adapter:inner 拿到 `Some(ownership)`、预留落盘并回填真实
    /// PID+starttime;门禁不可用时零触达 inner。
    #[tokio::test]
    async fn gated_streaming_adapter_forwards_start_owned_with_real_identity() {
        let probe = Arc::new(OwnershipForwardingProbe::new());
        let mut registry = ProviderRegistry::new();
        let adapter: Arc<dyn StreamingProviderAdapter> = probe.clone();
        registry
            .register_gated(
                ProviderName::Codex,
                adapter,
                Arc::new(UnprovisionedProviderPolicyProjector),
                gate(false, true, false),
            )
            .expect("register gated probe");

        let base = tempfile::tempdir().expect("gate b1 base");
        let private_root = base.path().join("private-root");
        std::fs::create_dir_all(&private_root).expect("private root");
        let store_root = base.path().join("resource-store");
        std::fs::create_dir_all(&store_root).expect("store root");
        let key = ResourceScopeKey {
            scope_id: "code-delivery-attempt-gate-b1".to_string(),
            run_id: "role-run-gate-b1".to_string(),
            boot_identity: format!("aria-engine-{}-gate-b1", std::process::id()),
        };
        let scope = ProviderRunResourceScope::open_for_recovery(
            &key,
            None,
            &private_root,
            &store_root,
            None,
        )
        .expect("open gate b1 scope");
        let ownership = TrustedLaunchOwnership::mint(
            Arc::new(scope),
            "code_delivery provider process: gate b1",
        );
        let scope_key = ownership.scope_key().clone();

        let gated = registry
            .get(&ProviderName::Codex)
            .expect("gated adapter from production assembly");
        let _session = gated
            .start_owned(
                streaming_input(),
                Some(ownership.clone()),
                CancellationToken::new(),
            )
            .await
            .expect("gate must forward start_owned");

        assert_eq!(
            probe.start_owned_calls.load(Ordering::SeqCst),
            1,
            "inner.start_owned 必须被调用一次"
        );
        assert_eq!(
            probe.start_calls.load(Ordering::SeqCst),
            0,
            "转发缺失时默认实现会回落 inner.start"
        );
        assert!(
            probe.saw_ownership.load(Ordering::SeqCst),
            "inner 必须收到 Some(受信归属)"
        );

        let records = process_records(&store_root, &scope_key);
        assert_eq!(records.len(), 1, "预留→回填恰一条 Process 登记");
        let record = &records[0];
        let pid = probe.child_pid();
        assert!(pid > 0, "probe 必须启动真实子进程");
        assert_eq!(record.resource_id, ownership.resource_id());
        assert_eq!(
            record.started_identity,
            Some(StartedIdentity::Process {
                pid,
                start_time_ticks: process_start_time_ticks(pid)
                    .expect("live probe child starttime must be readable"),
            }),
            "登记身份必须是 spawn 点回填的真实 PID+starttime"
        );

        probe.terminate_child().await;
        ownership.release_after_session_tail().await;
        assert!(
            process_records(&store_root, &scope_key).is_empty(),
            "收口后登记必须注销"
        );

        // 门禁不可用:先拒绝,不得触达 inner(零 spawn/零登记)。
        let blocked = GatedStreamingProviderAdapter::new(
            ProviderName::Codex,
            probe.clone(),
            gate(false, false, false),
        );
        let error = blocked
            .start_owned(streaming_input(), Some(ownership), CancellationToken::new())
            .await
            .err()
            .expect("blocked gate must reject start_owned");
        assert_eq!(error.code, ProviderErrorCode::ProviderUnavailable);
        assert_eq!(
            probe.start_owned_calls.load(Ordering::SeqCst),
            1,
            "门禁拒绝不得触达 inner.start_owned"
        );
        assert_eq!(
            probe.start_calls.load(Ordering::SeqCst),
            0,
            "门禁拒绝不得触达 inner.start"
        );
    }
}
