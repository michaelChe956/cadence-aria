//! F3 修复轮 P2-1：legacy 直连 `run_streaming` 路径的策略会话 durable 审计注入。
//!
//! 默认 bridge（`StreamingProviderAdapter::run_streaming`）按角色矩阵为策略角色
//! 派生 `DenyFileWriteBuiltins`，但 bridge 本体是中立层、无法构造 product 层的
//! `LifecycleStore` sink——注入点在本 engine：`run_legacy_stream_to_completion`
//! 为策略角色的 legacy run 分配 run-bound sink，经
//! [`LegacyToolPolicyAuditProvider`] 装饰器以 trait hook（`legacy_tool_policy_audit_sink`）
//! 交给默认 bridge，使 legacy 直连路径「策略+sink」同源同在，不得运行时
//! fail-closed。
//!
//! 语义保真：
//! - 策略角色优先走默认 bridge（派生策略+注入 sink，经 `start` 接受守卫与
//!   denylist argv）；
//! - adapter 未实现 `start`（run_streaming-only 测试替身）时回落 inner 既有
//!   `run_streaming`，与修复前的 legacy 语义逐字一致；
//! - 非策略角色直接透传 inner。

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::cross_cutting::tool_policy_audit::{RoleRunBoundAuditSink, ToolPolicyAuditSink};
use crate::product::lifecycle_store::LifecycleStore;
use crate::protocol::contracts::AdapterInput;
use crate::protocol::contracts::AdapterRole;

use super::CodingWorkspaceEngine;
use super::ws_event_mapper::provider_start_is_not_implemented;

/// AdapterInput 角色是否按矩阵派生策略（与默认 bridge 的派生矩阵逐字一致）。
pub(crate) fn legacy_input_role_requires_policy(input: &AdapterInput) -> bool {
    matches!(
        input.role,
        AdapterRole::Orchestrator | AdapterRole::WorkItemSplitter | AdapterRole::Reviewer
    )
}

/// engine 侧注入装饰器：携带 run-bound sink，供默认 bridge 的
/// `legacy_tool_policy_audit_sink` hook 取用。
pub(crate) struct LegacyToolPolicyAuditProvider<'a> {
    inner: &'a dyn StreamingProviderAdapter,
    sink: Arc<dyn ToolPolicyAuditSink>,
}

impl<'a> LegacyToolPolicyAuditProvider<'a> {
    pub(crate) fn new(
        inner: &'a dyn StreamingProviderAdapter,
        sink: Arc<dyn ToolPolicyAuditSink>,
    ) -> Self {
        Self { inner, sink }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for LegacyToolPolicyAuditProvider<'_> {
    fn supports_tool_calls(&self) -> bool {
        self.inner.supports_tool_calls()
    }

    fn legacy_tool_policy_audit_sink(&self) -> Option<Arc<dyn ToolPolicyAuditSink>> {
        Some(self.sink.clone())
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<crate::cross_cutting::streaming_provider::ProviderSession, ProviderAdapterError>
    {
        self.inner.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        if legacy_input_role_requires_policy(input) {
            // 默认 bridge 桥接体（自由函数，P2-1）：派生策略 + 注入本装饰器携带
            // 的 sink，经 inner.start 接受守卫（不经虚分发，避免递归）。
            let audit_sink = self.legacy_tool_policy_audit_sink();
            let inner = self.inner;
            let bridge = crate::cross_cutting::streaming_provider::run_legacy_bridge_stream(
                audit_sink,
                Box::new(move |provider_input, cancel| {
                    Box::pin(async move { inner.start(provider_input, cancel).await })
                }),
                input,
                cancel.clone(),
            )
            .await;
            match bridge {
                Ok(stream) => return Ok(stream),
                Err(error) if !provider_start_is_not_implemented(&error) => return Err(error),
                // inner 未实现 start（run_streaming-only 测试替身）：回落 inner
                // 既有 run_streaming，保持修复前语义。
                Err(_) => {}
            }
        }
        self.inner.run_streaming(input, cancel).await
    }
}

impl CodingWorkspaceEngine {
    /// legacy 直连的 provider run（P2-1 注入点）：策略角色分配 run-bound sink
    /// （与 `attach_tool_policy_audit` 同法；分配即持久化，P1-6）并经装饰器把
    /// sink 交给默认 bridge；非策略角色原样透传。
    pub(crate) async fn run_legacy_provider_stream(
        &self,
        provider: &dyn StreamingProviderAdapter,
        input: &AdapterInput,
        attempt_id: &str,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        if !legacy_input_role_requires_policy(input) {
            return provider.run_streaming(input, cancel).await;
        }
        let workspace_session_id = format!("coding-{attempt_id}");
        let store = LifecycleStore::new(self.store.paths());
        let role_run_seq = store
            .next_tool_policy_role_run_seq(&workspace_session_id)
            .map_err(|error| {
                ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    format!("legacy tool policy audit sink allocation failed: {error}"),
                    0,
                )
            })?;
        let sink =
            RoleRunBoundAuditSink::new(Arc::new(store.clone()), workspace_session_id, role_run_seq)
                .into_sink();
        LegacyToolPolicyAuditProvider::new(provider, sink)
            .run_streaming(input, cancel)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// start 可用并记录 input 的替身（默认 bridge 路径可达）。
    struct CapturingStartProvider {
        starts: AtomicUsize,
        saw_policy: AtomicBool,
        saw_sink: AtomicBool,
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for CapturingStartProvider {
        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<crate::cross_cutting::streaming_provider::ProviderSession, ProviderAdapterError>
        {
            self.starts.fetch_add(1, Ordering::SeqCst);
            self.saw_policy
                .store(input.tool_policy.is_some(), Ordering::SeqCst);
            self.saw_sink
                .store(input.audit_sink.is_some(), Ordering::SeqCst);
            let (event_tx, event_rx) = mpsc::channel(4);
            let (command_tx, _command_rx) = mpsc::channel(4);
            tokio::spawn(async move {
                let _ = event_tx
                    .send(
                        crate::cross_cutting::streaming_provider::ProviderEvent::Completed(
                            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                                "legacy policy done",
                                None,
                            ),
                        ),
                    )
                    .await;
            });
            Ok(crate::cross_cutting::streaming_provider::ProviderSession {
                native_session_id: None,
                events: event_rx,
                commands: command_tx,
            })
        }
    }

    fn policy_role_input() -> AdapterInput {
        AdapterInput {
            prompt: "legacy policy run".to_string(),
            provider_type: crate::protocol::contracts::ProviderType::Fake,
            role: AdapterRole::Reviewer,
            worktree_path: None,
            provider_stream_log_dir: None,
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 60,
            max_retries: 0,
        }
    }

    fn executor_role_input() -> AdapterInput {
        let mut input = policy_role_input();
        input.role = AdapterRole::Executor;
        input
    }

    fn bound_sink() -> Arc<dyn ToolPolicyAuditSink> {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let store = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
            tmp.path().join(".aria"),
        ));
        RoleRunBoundAuditSink::new(Arc::new(store), "ws-legacy-policy", 0).into_sink()
    }

    #[tokio::test]
    async fn legacy_policy_role_stream_carries_derived_policy_and_sink() {
        let provider = CapturingStartProvider {
            starts: AtomicUsize::new(0),
            saw_policy: AtomicBool::new(false),
            saw_sink: AtomicBool::new(false),
        };
        let wrapper = LegacyToolPolicyAuditProvider::new(&provider, bound_sink());
        let mut rx = wrapper
            .run_streaming(&policy_role_input(), CancellationToken::new())
            .await
            .expect("policy role legacy stream");
        while let Some(chunk) = rx.recv().await {
            if matches!(chunk, StreamChunk::Done { .. }) {
                break;
            }
        }
        assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
        assert!(provider.saw_policy.load(Ordering::SeqCst));
        assert!(
            provider.saw_sink.load(Ordering::SeqCst),
            "P2-1：派生策略必须同时注入 engine 分配的 sink"
        );
    }

    #[tokio::test]
    async fn legacy_executor_role_passes_through_inner_untouched() {
        let provider = CapturingStartProvider {
            starts: AtomicUsize::new(0),
            saw_policy: AtomicBool::new(false),
            saw_sink: AtomicBool::new(false),
        };
        let wrapper = LegacyToolPolicyAuditProvider::new(&provider, bound_sink());
        let mut rx = wrapper
            .run_streaming(&executor_role_input(), CancellationToken::new())
            .await
            .expect("executor legacy stream");
        while let Some(chunk) = rx.recv().await {
            if matches!(chunk, StreamChunk::Done { .. }) {
                break;
            }
        }
        assert_eq!(
            provider.starts.load(Ordering::SeqCst),
            1,
            "非策略角色经 inner 自有默认 bridge 启动（无策略派生）"
        );
        assert!(
            !provider.saw_policy.load(Ordering::SeqCst),
            "executor 角色不派生策略"
        );
        assert!(
            !provider.saw_sink.load(Ordering::SeqCst),
            "非策略角色不注入 sink（透传 inner）"
        );
    }

    /// start 未实现、仅自定义 run_streaming 的替身：回落语义保持修复前行为。
    struct RunStreamingOnlyProvider {
        legacy_calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for RunStreamingOnlyProvider {
        async fn run_streaming(
            &self,
            _input: &AdapterInput,
            _cancel: CancellationToken,
        ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
            self.legacy_calls.fetch_add(1, Ordering::SeqCst);
            let (_tx, rx) = mpsc::channel(1);
            Ok(rx)
        }
    }

    #[tokio::test]
    async fn legacy_policy_role_falls_back_to_inner_run_streaming_for_startless_doubles() {
        let provider = RunStreamingOnlyProvider {
            legacy_calls: AtomicUsize::new(0),
        };
        let wrapper = LegacyToolPolicyAuditProvider::new(&provider, bound_sink());
        let _rx = wrapper
            .run_streaming(&policy_role_input(), CancellationToken::new())
            .await
            .expect("startless double keeps its legacy run_streaming semantics");
        assert_eq!(provider.legacy_calls.load(Ordering::SeqCst), 1);
    }
}
