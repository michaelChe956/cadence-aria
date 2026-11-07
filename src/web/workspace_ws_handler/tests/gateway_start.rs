//! T10a:聚合规划 author 启动点经 gateway 的 handler 级单测。
//!
//! 覆盖:
//! - `resolve_plan_author_launch` + `start_work_item_plan_author(Logical, ...)` 经 gateway 启动并留 audit;
//! - `resolve_plan_author_launch` 无 gateway 时返回 Legacy,`start_work_item_plan_author(Legacy, ...)` 原样走 provider.start(Legacy 零变化);
//! - WorkspaceEngine 两个最小公开访问器。

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderSession, StreamingProviderInput,
};
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::{
    CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
    CreateWorkspaceSessionInput,
};
use crate::product::logical_codebase::policy::ProviderDialect;
use crate::product::logical_codebase::{
    LogicalCodebaseManifest, LogicalCodebaseProviderGateway, LogicalCodebaseStore,
};
use crate::product::models::{IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, WorkspaceType};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::protocol::contracts::{AdapterOutput, AdapterRole, ProviderType, TimeoutStatus};
use crate::web::gateway_factory::LogicalCodebaseGatewayFactory;

struct StubSyncAdapter;

impl ProviderAdapter for StubSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
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

struct CountingStreamingAdapter {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CountingStreamingAdapter {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (_event_tx, events) = mpsc::channel(1);
        let (commands, _command_rx) = mpsc::channel(1);
        Ok(ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
    }

    /// Task 1b 段③:prepared launch 经 `start_validated` 分发——fixture 拆出
    /// input 后与裸 `start` 同路径(raw 计数语义保持)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }
}

struct GatewayFixture {
    _temp: tempfile::TempDir,
    paths: ProductAppPaths,
    gateway: Arc<LogicalCodebaseProviderGateway>,
    aggregate_root: std::path::PathBuf,
}

fn gateway_fixture() -> GatewayFixture {
    gateway_fixture_with_adapter(Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    }))
}

/// Task 2.8：允许注入自定义 streaming adapter（捕获 input 断言 cwd）的
/// gateway fixture 变体；权威记录拓扑与 `gateway_fixture` 一致。
fn gateway_fixture_with_adapter(adapter: Arc<dyn StreamingProviderAdapter>) -> GatewayFixture {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    crate::product::project_store::ProjectStore::new(paths.clone())
        .create(crate::product::project_store::CreateProjectInput {
            name: "gateway fixture project".to_string(),
            description: None,
        })
        .expect("create project");

    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, adapter);

    let factory = LogicalCodebaseGatewayFactory::new(
        paths.clone(),
        Arc::new(registry),
        Arc::new(StubSyncAdapter),
        crate::web::handlers::automation_enrollment_test_support::always_available_gate(),
    );

    let aggregate_root = root.path().join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
    let manifest = LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest("project_0001", &manifest)
        .expect("save manifest");

    let gateway = Arc::new(factory.build("project_0001").expect("build gateway"));

    GatewayFixture {
        _temp: root,
        paths,
        gateway,
        aggregate_root,
    }
}

fn streaming_input(working_dir: std::path::PathBuf) -> StreamingProviderInput {
    StreamingProviderInput {
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
        provider_type: ProviderType::ClaudeCode,
        role: AdapterRole::WorkItemSplitter,
        prompt: "work item plan outline".to_string(),
        working_dir,
        workspace_session_id: Some("session_0001".to_string()),
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: std::collections::BTreeMap::new(),
        timeout_secs: 3600,
    }
}

fn workspace_session(repository_path: std::path::PathBuf) -> WorkspaceSession {
    WorkspaceSession {
        session_id: "session_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "work_item_plan_0001".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        stage: WorkspaceStage::Running,
        messages: vec![],
        artifact: None,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: Some(ProviderName::Codex),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: false,
        openspec_enabled: false,
        session_status: crate::product::models::WorkspaceSessionStatus::Running,
        flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
        run_history: crate::product::work_item_plan_policy::RunHistory::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        policy_diagnostics: vec![],
        provider_start_ledger: vec![],
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        provider_conversations: vec![],
        logical_launch_fingerprints: vec![],
        superseded_native_sessions: vec![],
        lc_resume_recovery: None,
        structural_governance_state: None,
        review_identity_id: String::new(),
        repository_path: Some(repository_path),
    }
}

/// Task 2.8：记录启动 input 并立即完成的 capture adapter（gateway registry 内）。
struct CapturingStreamingAdapter {
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    /// T1B-P3-1:provider 启动时序探针——`start`(含 `start_validated` 委托)
    /// 在共享计数器上取标记并快照,使 run 收口时序断言承载真实启动顺序
    /// 而非测试线程手工计数。`None`/缺省时零行为(其余构造点不受影响)。
    start_mark: Option<Arc<AtomicUsize>>,
    observed_start_mark: Arc<std::sync::Mutex<Option<usize>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CapturingStreamingAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        if let Some(counter) = self.start_mark.as_ref() {
            let mark = counter.fetch_add(1, Ordering::SeqCst);
            *self.observed_start_mark.lock().expect("start mark probe") = Some(mark);
        }
        let _ = self.inputs.send(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(
                    crate::cross_cutting::streaming_provider::ProviderEvent::Completed(
                        crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            "{}".to_string(),
                            None,
                        ),
                    ),
                )
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
    /// Task 1b 段③:prepared launch 经 `start_validated` 分发——fixture 拆出
    /// input 后与裸 `start` 同路径(capture 语义保持)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }
}

/// 启动握手永不返回的 provider：模拟 adapter `start` 卡死（ACP/JSON-RPC
/// 握手无应答），用于验证 plan author 启动口的停滞界。取消 token 不可见
/// ——与现场形态一致（卡在 select 之外的裸 await）。
struct HangingStartStreamingAdapter {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingStartStreamingAdapter {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        std::future::pending::<()>().await;
        unreachable!("hanging provider start never resolves")
    }

    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }
}

/// B2b1(Task 5.2)测试公共 fixture:ProviderRunFixture 的 LogicalAlias 拓扑
///(durable 会话记录 + manifest/selection/policy + gateway 注入 + 成员
/// checkout=repository_root、聚合根=其父目录),`resolve_plan_author_launch`
/// 的 E native anchor 消费面按生产形态可达。capability 用
/// `LcDialectCapabilitySource`(四家 dialect 全 Confirmed)。
fn t14_logical_fixture_with_gateway_extras(
    gateway_author: Arc<dyn StreamingProviderAdapter>,
) -> super::single_candidate_provider_run::ProviderRunFixture {
    use super::single_candidate_lc_admission::{AliasLcGatewayExtras, LcDialectCapabilitySource};

    super::single_candidate_provider_run::ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        gateway_author,
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            ..AliasLcGatewayExtras::default()
        },
    )
}

/// t14 夹具的 canonical 聚合根(成员 checkout 的父目录,gateway
/// authority_root 同源 canonicalize)。
fn t14_canonical_root(
    fixture: &super::single_candidate_provider_run::ProviderRunFixture,
) -> std::path::PathBuf {
    std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root")
}

/// B2b1 测试 capability source:仅 ClaudeCode 分格 Confirmed,其余
/// provider 以 `provider_capability_launch_not_confirmed` fail-closed
///(钉 Task 1a「集中映射防线移到 capability 准入」合同)。
struct T14ClaudeOnlyCapabilitySource;

impl crate::product::logical_codebase::ProviderCapabilitySource for T14ClaudeOnlyCapabilitySource {
    fn require_supported(
        &self,
        provider: &crate::product::logical_codebase::ProviderRef,
        _action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;

        if provider.provider_type != crate::product::logical_codebase::ProviderRefType::ClaudeCode {
            return Err(
                crate::product::logical_codebase::ProviderGatewayError::UnsupportedCapability(
                    format!(
                        "provider_capability_launch_not_confirmed: no capability row for {:?}",
                        provider.provider_type
                    ),
                ),
            );
        }
        Ok(crate::product::logical_codebase::ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.4.0".to_string(),
            adapter_dialect: crate::product::logical_codebase::ProviderDialect::ClaudeCodeCliV1,
            wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability: ProviderActionCapability {
                action: crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: "projection-digest-t14".to_string(),
                evidence_ref: "probe://t14".to_string(),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        })
    }

    fn require_resume_supported(
        &self,
        provider: &crate::product::logical_codebase::ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        self.require_supported(provider, action)
    }

    fn require_write_boundary(
        &self,
        provider: &crate::product::logical_codebase::ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        self.require_supported(provider, action)
    }

    fn require_root_recipe_supported(
        &self,
        provider: &crate::product::logical_codebase::ProviderRef,
        _credential:
            &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        self.require_supported(
            provider,
            crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
        )
    }
}

include!("gateway_start_parts/author_launch_cases.inc.rs");

include!("gateway_start_parts/anchor_and_trust_cases.inc.rs");
