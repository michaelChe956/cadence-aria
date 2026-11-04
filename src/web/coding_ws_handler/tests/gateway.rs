//! web 侧 coding handler 测试用 LC gateway 夹具。
//!
//! REQ-ROUTE-01 两态化（去单仓入口）后，携带冻结 `target_snapshot` 的 coding
//! attempt **只能**经 `LogicalCodebaseProviderGateway` 启动 provider
//! （`provider_stream.rs` 的 `is_logical_target` 门要求 `validated_input`
//! 非空，legacy 直连被强制关闭）。因此凡是真启动 provider 的 web 侧用例，
//! 都必须给引擎注入一个能真正启动的 gateway。
//!
//! 生产 gateway 的能力源读 durable capability 记录（要求 launch/resume/
//! write_boundary 分格为 `Confirmed`，需真实 probe 产出）、可用性门读真实
//! provider 健康快照——二者都不是测试面能就地满足的。本模块按同型双件套
//! （能力源恒 `Confirmed`、可用性门恒健康）提供测试替身，与
//! `src/product/coding_workspace_engine/tests/provider_gateway_validated_input.rs`
//! 的 `build_gateway_with_registry` 同构（该函数是产品层测试模块私有，web 层
//! 不可见，故此处按同一形状另建一份 web 层替身）。
//!
//! 可用性门恒健康替身**不在本文件另建**：web 测试树共用
//! `crate::web::handlers::automation_enrollment_test_support::always_available_gate`
//! （唯一一份，见该文件注释）。

use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::policy::{
    PolicyTarget, ProviderDialect, ProviderWireDialect, SessionPolicyAction,
};
use crate::product::logical_codebase::provider_gateway::{
    GatewayRunAudit, LogicalCodebaseProviderGateway, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderRef, ProviderRefType, SessionLaunchRequest,
};
use crate::product::logical_codebase::{AggregatePolicyArtifactStore, ProviderActionCapability};
use crate::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};

/// 恒放行能力源：按 provider 家族给出对应 dialect，launch/resume/
/// write_boundary 三格恒 `Confirmed`（免真实 probe）。
pub(super) struct LcStaticCapabilitySource;

impl LcStaticCapabilitySource {
    fn capability(provider: &ProviderRef, action: SessionPolicyAction) -> ProviderCapability {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        let (adapter_dialect, wire_dialect) = match provider.provider_type {
            ProviderRefType::ClaudeCode => (
                ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            ProviderRefType::Codex => (
                ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
            ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
        };
        ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.0.0".to_string(),
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability: ProviderActionCapability {
                action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: format!("projection-digest-{action:?}"),
                evidence_ref: format!("probe://{action:?}"),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl ProviderCapabilitySource for LcStaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        use crate::product::logical_codebase::provider_gateway::ProviderGatewayError;
        if provider.provider_type != ProviderRefType::ClaudeCode {
            return Err(ProviderGatewayError::UnsupportedCapability(
                crate::product::logical_codebase::provider_gateway::PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE
                    .to_string(),
            ));
        }
        Ok(Self::capability(
            provider,
            SessionPolicyAction::PlanningReadOnly,
        ))
    }
}

/// 透传 target resolver：canonicalize 请求里的 worktree 后按请求形态重建 target。
pub(super) struct LcPassThroughTargetResolver;

impl PolicyTargetResolver for LcPassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, crate::product::logical_codebase::ProviderGatewayError> {
        use crate::product::logical_codebase::ProviderGatewayError;
        let canonical = std::fs::canonicalize(&request.target.worktree)
            .map_err(|_| ProviderGatewayError::Target("worktree missing".to_string()))?;
        if request.target.logical_repository_id.is_empty() {
            Ok(PolicyTarget::aggregate_root(canonical))
        } else {
            Ok(PolicyTarget::checkout(
                request.target.logical_repository_id.clone(),
                request.target.checkout_id.clone(),
                canonical,
            ))
        }
    }
}

/// 同步槽打桩：gateway 构造签名需要，测试面不消费。
pub(super) struct LcStubSyncAdapter;

impl ProviderAdapter for LcStubSyncAdapter {
    fn run(
        &self,
        _input: &AdapterInput,
    ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
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

/// 组装一个能真正启动 provider 的 LC gateway（能力源恒 `Confirmed`、target
/// 直通、同步槽打桩、可用性门恒健康）。
///
/// 权威根恒取 workspace root（`.aria` 的父目录）：夹具的成员主 checkout 与
/// coding worktree 均在其之下，gateway 的 cwd 权威早门才放行。
pub(super) fn build_test_gateway(
    paths: &ProductAppPaths,
    registry: Arc<ProviderRegistry>,
) -> LogicalCodebaseProviderGateway {
    LogicalCodebaseProviderGateway::with_audit(
        AggregatePolicyArtifactStore::new(paths.clone()),
        Arc::new(LcStaticCapabilitySource),
        Arc::new(LcPassThroughTargetResolver),
        registry,
        Arc::new(LcStubSyncAdapter),
        crate::web::handlers::automation_enrollment_test_support::always_available_gate(),
        Arc::new(GatewayRunAudit::new()),
        paths.root().parent().expect("workspace root").to_path_buf(),
    )
}

/// 可换挡 provider 替身。
///
/// `ProviderRegistry::register` 需 `&mut self`，一旦 gateway 持有 `Arc`
/// registry 就无法再注册——而 gateway 必须在夹具构造期就绪（引擎随夹具
/// 一起建好），用例的 fake adapter 却是在其后才有的。故夹具在 registry 里
/// 登记本替身，用例把真实 adapter 装进槽位，gateway 启动时按槽位转发。
#[derive(Clone, Default)]
pub(super) struct SwappableProvider {
    inner: Arc<Mutex<Option<Arc<dyn StreamingProviderAdapter>>>>,
}

impl SwappableProvider {
    /// 装入本次用例要用的 fake adapter。
    pub(super) fn install(&self, adapter: Arc<dyn StreamingProviderAdapter>) {
        *self.inner.lock().expect("swappable provider slot") = Some(adapter);
    }

    fn installed(&self) -> Option<Arc<dyn StreamingProviderAdapter>> {
        self.inner.lock().expect("swappable provider slot").clone()
    }

    fn missing() -> ProviderAdapterError {
        ProviderAdapterError::provider_unavailable(
            "swappable provider slot is empty: fixture must install the fake adapter",
        )
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SwappableProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        match self.installed() {
            Some(adapter) => adapter.start(input, cancel).await,
            None => Err(Self::missing()),
        }
    }

    /// LC validated 启动：拆出 input 后与裸 `start` 同路径转发（夹具里的
    /// fake adapter 只实现裸 `start`）。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<
        tokio::sync::mpsc::Receiver<crate::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        match self.installed() {
            Some(adapter) => adapter.run_streaming(input, cancel).await,
            None => Err(Self::missing()),
        }
    }
}
