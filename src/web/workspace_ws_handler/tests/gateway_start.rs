//! T10a:聚合规划 author 启动点经 gateway 的 handler 级单测。
//!
//! 覆盖:
//! - `resolve_plan_author_launch` + `start_work_item_plan_author(Logical, ...)` 经 gateway 启动并留 audit;
//! - `resolve_plan_author_launch` 无 gateway 时返回 Legacy,`start_work_item_plan_author(Legacy, ...)` 原样走 provider.start(Legacy 零变化);
//! - WorkspaceEngine 两个最小公开访问器。

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
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

fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = chrono::Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
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
    Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
        snapshot,
    ))))
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
        always_available_gate(),
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

/// Task 2.8(REQ-PLN-03,planning snapshot 贯穿):StaleContext 重建(B3)把
/// input worktree 换成 rebuilt.cwd(聚合根)——`start_work_item_plan_author`
/// 必须把 input 的独立 cwd 重绑 envelope 冻结的 root,spawn 前复验不得因
/// 「input root vs envelope 成员 cwd」漂移而拒绝重建 run;worktree 字段
/// 原样透传(target 语义)。
#[tokio::test]
async fn start_work_item_plan_author_rebinds_input_cwd_to_envelope_root_for_rebuild() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let canonical_root = t14_canonical_root(&fixture);
    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };

    // rebuild 形态:worktree=rebuilt.cwd=聚合根(provider_run.rs B3 分支),
    // working_directory 未注入(None → 旧字段回填 worktree)。
    let input = streaming_input(canonical_root.clone());
    let session = start_work_item_plan_author(
        launch,
        Arc::new(CapturingStreamingAdapter {
            inputs: mpsc::unbounded_channel().0,
            start_mark: None,
            observed_start_mark: Arc::default(),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;

    assert!(
        session.is_ok(),
        "rebuild 形态启动不得被 cwd 漂移拒绝: {:?}",
        session.as_ref().err()
    );
    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert_eq!(
        captured.working_directory.as_deref(),
        Some(canonical_root.as_path()),
        "spawn cwd 必须重绑 envelope 冻结的 canonical root,got: {:?}",
        captured.working_directory
    );
    assert_eq!(
        captured.working_dir, canonical_root,
        "rebuild worktree(rebuilt.cwd)原样透传(target 语义)"
    );
}

/// Task 2.8(REQ-PLN-01/07):正常(非重建)plan author input 的 worktree 是
/// 成员 checkout——`start_work_item_plan_author` 同样必须把独立 cwd 冻结到
/// envelope root,provider 不得以成员 checkout 为 cwd 启动。
#[tokio::test]
async fn start_work_item_plan_author_binds_normal_input_cwd_to_envelope_root() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let canonical_root = t14_canonical_root(&fixture);
    let member = fixture.member_checkout_root();
    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };

    let input = streaming_input(member.clone());
    let session = start_work_item_plan_author(
        launch,
        Arc::new(CapturingStreamingAdapter {
            inputs: mpsc::unbounded_channel().0,
            start_mark: None,
            observed_start_mark: Arc::default(),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;

    assert!(
        session.is_ok(),
        "normal 形态启动失败: {:?}",
        session.as_ref().err()
    );
    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert_eq!(
        captured.working_directory.as_deref(),
        Some(canonical_root.as_path()),
        "spawn cwd 必须冻结 canonical root(非成员 checkout),got: {:?}",
        captured.working_directory
    );
    assert_eq!(
        captured.working_dir, member,
        "正常形态 worktree 保持成员 checkout"
    );
}

/// A1-b1(Bug45 批2 环境回退):plan/split 唯一物理启动口停止注入环境——
/// caller 的 env_vars(sentinel 覆盖层)原样透传;宿主 PATH 等由底座
/// (process_manager)继承到达子进程,裸名 CLI(kimi)解析不断链。
#[tokio::test]
async fn start_work_item_plan_author_preserves_caller_env_without_injection() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let canonical_root = t14_canonical_root(&fixture);
    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };

    let mut input = streaming_input(canonical_root.clone());
    input.env_vars.insert(
        "ARIA_A1B_SENTINEL".to_string(),
        "caller-override".to_string(),
    );
    let session = start_work_item_plan_author(
        launch,
        Arc::new(CapturingStreamingAdapter {
            inputs: mpsc::unbounded_channel().0,
            start_mark: None,
            observed_start_mark: Arc::default(),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;

    assert!(
        session.is_ok(),
        "plan author 启动不得因环境构造失败: {:?}",
        session.as_ref().err()
    );
    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert_eq!(
        captured
            .env_vars
            .get("ARIA_A1B_SENTINEL")
            .map(String::as_str),
        Some("caller-override"),
        "plan author 启动必须原样透传 caller 显式覆盖层"
    );
    assert!(
        captured.env_vars.get("PATH").is_none() && captured.env_vars.get("HOME").is_none(),
        "plan author 启动不得再注入白名单 PATH/HOME(宿主值由底座继承): {:?}",
        captured.env_vars
    );
}

fn workspace_engine(fixture: &GatewayFixture, with_gateway: bool) -> WorkspaceEngine {
    workspace_engine_with_author(fixture, with_gateway, ProviderName::ClaudeCode)
}

/// C-2:构造指定 author provider 的 engine(其余与 `workspace_engine` 一致),
/// 供 provider 身份透传/失败关闭测试驱动 session 配置。
fn workspace_engine_with_author(
    fixture: &GatewayFixture,
    with_gateway: bool,
    author: ProviderName,
) -> WorkspaceEngine {
    let (event_tx, _event_rx) = mpsc::channel::<crate::product::workspace_engine::EngineEvent>(8);
    let mut session = workspace_session(fixture.aggregate_root.clone());
    session.author_provider = author;
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        event_tx,
        session,
    );
    if with_gateway {
        engine.with_logical_provider_gateway(fixture.gateway.clone())
    } else {
        engine
    }
}

#[tokio::test]
async fn start_work_item_plan_author_routes_logical_through_gateway_and_records_audit() {
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    }));
    let audit = {
        let engine = fixture.engine.lock().await;
        engine
            .logical_provider_gateway()
            .expect("fixture gateway")
            .audit()
    };
    assert_eq!(audit.stream_launches(), 0);

    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };
    let input = streaming_input(t14_canonical_root(&fixture));

    let session = start_work_item_plan_author(
        launch,
        Arc::new(CountingStreamingAdapter {
            starts: Arc::new(AtomicUsize::new(0)),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;

    assert!(
        session.is_ok(),
        "gateway launch failed: {:?}",
        session.as_ref().err()
    );
    assert_eq!(audit.stream_launches(), 1);
}

/// C-2(组1):logical author 配置 Claude 时,launch 必须携带 session 配置的
/// author provider 身份,validated envelope 冻结 ClaudeCode dialect——不得由
/// 启动点硬编码推定。
#[tokio::test]
async fn logical_plan_launch_carries_session_author_provider_identity() {
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    }));

    let plan_launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("claude author must resolve a logical launch")
    };
    let PlanAuthorLaunch::Logical(plan) = &plan_launch else {
        panic!("logical session must resolve a Logical launch");
    };
    assert_eq!(
        plan.launch.author_provider,
        ProviderName::ClaudeCode,
        "launch must carry the session-configured author provider"
    );
    assert_eq!(
        plan.validated.envelope().provider_dialect,
        ProviderDialect::ClaudeCodeCliV1,
        "validated envelope must freeze the ClaudeCode dialect"
    );
}

/// Task 1a 后合同重钉(6b carry):Pi/Kimi 是合法显式映射,「静默以 Claude
/// 跑完整 planning run」的防线从集中映射移到 capability 准入——fixture 未
/// 给 Pi/Kimi 落 durable capability 记录,logical 启动必须在 launch 分格门
/// 显式失败(判别码 + provider 名),gateway 零启动。
#[tokio::test]
async fn logical_plan_launch_fails_closed_for_unsupported_author_provider() {
    for author in [ProviderName::Pi, ProviderName::KimiCode] {
        let fixture =
            super::single_candidate_provider_run::ProviderRunFixture::new_logical_with_gateway_extras(
                true,
                Arc::new(CountingStreamingAdapter {
                    starts: Arc::new(AtomicUsize::new(0)),
                }),
                super::single_candidate_lc_admission::AliasLcGatewayExtras {
                    capability: Some(Arc::new(T14ClaudeOnlyCapabilitySource)),
                    ..super::single_candidate_lc_admission::AliasLcGatewayExtras::default()
                },
            );
        let (audit, error) = {
            let mut engine = fixture.engine.lock().await;
            engine.session.author_provider = author.clone();
            (
                engine
                    .logical_provider_gateway()
                    .expect("fixture gateway")
                    .audit(),
                resolve_plan_author_launch(&engine)
                    .err()
                    .unwrap_or_else(|| {
                        panic!("{author:?} author must not silently launch as Claude")
                    }),
            )
        };
        assert!(
            error
                .details
                .contains("provider_capability_launch_not_confirmed"),
            "expected provider_capability_launch_not_confirmed, got: {}",
            error.details
        );
        assert!(
            error.details.contains(&format!("{author:?}")),
            "error must name the configured author provider, got: {}",
            error.details
        );
        assert_eq!(audit.stream_launches(), 0, "no gateway session may start");
    }
}

/// r47 修订:author 配置 Codex 时,logical 启动不再命中路由级全阻(Task 13
/// 期形态已移除——LC 投影后 Codex 恒非 danger);改钉新契约:capability 行
/// 未探针签发(bootstrap Unknown)时 fail-closed 为
/// provider_capability_launch_not_confirmed,且不静默改 Claude 启动。
#[tokio::test]
async fn logical_plan_launch_codex_author_hits_req_env05_route_block() {
    let fixture =
        super::single_candidate_provider_run::ProviderRunFixture::new_logical_with_gateway_extras(
            true,
            Arc::new(CountingStreamingAdapter {
                starts: Arc::new(AtomicUsize::new(0)),
            }),
            super::single_candidate_lc_admission::AliasLcGatewayExtras {
                capability: Some(Arc::new(T14ClaudeOnlyCapabilitySource)),
                ..super::single_candidate_lc_admission::AliasLcGatewayExtras::default()
            },
        );
    let (audit, error) = {
        let mut engine = fixture.engine.lock().await;
        engine.session.author_provider = ProviderName::Codex;
        (
            engine
                .logical_provider_gateway()
                .expect("fixture gateway")
                .audit(),
            resolve_plan_author_launch(&engine)
                .err()
                .expect("codex author without probe-confirmed row must fail closed"),
        )
    };
    assert!(
        error
            .details
            .contains("provider_capability_launch_not_confirmed"),
        "expected capability-row fail-closed (route blanket block removed in r47), got: {}",
        error.details
    );
    assert_eq!(audit.stream_launches(), 0, "no gateway session may start");
}

#[tokio::test]
async fn start_work_item_plan_author_none_uses_legacy_provider_start_unchanged() {
    let fixture = gateway_fixture();
    let audit = fixture.gateway.audit();
    assert_eq!(audit.stream_launches(), 0);

    let starts = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn StreamingProviderAdapter> = Arc::new(CountingStreamingAdapter {
        starts: starts.clone(),
    });
    let engine = workspace_engine(&fixture, false);
    let plan_launch = resolve_plan_author_launch(&engine).expect("legacy launch");
    let input = streaming_input(fixture.aggregate_root.clone());

    let session =
        start_work_item_plan_author(plan_launch, provider, input, CancellationToken::new(), None)
            .await;

    assert!(
        session.is_ok(),
        "legacy start failed: {:?}",
        session.as_ref().err()
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "legacy provider.start must run"
    );
    assert_eq!(
        audit.stream_launches(),
        0,
        "legacy path must not touch gateway audit"
    );
}

#[test]
fn workspace_engine_accessors_expose_logical_launch() {
    let fixture = gateway_fixture();
    let engine = workspace_engine(&fixture, true);

    assert!(engine.logical_provider_gateway().is_some());
    assert_eq!(
        engine.logical_planning_launch(),
        Some(("project_0001".to_string(), fixture.aggregate_root.clone()))
    );
}

#[tokio::test]
async fn logical_plan_validate_failure_is_reported_by_handler() {
    let fixture = gateway_fixture();
    let repo_path = fixture._temp.path().join("member");
    std::fs::create_dir_all(&repo_path).expect("member checkout");
    let repository = RepositoryStore::new(fixture.paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "member".to_string(),
            path: repo_path.clone(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "logical-plan-validate-failure".to_string(),
        })
        .expect("repository");
    let issue = IssueStore::new(fixture.paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            logical_codebase_id: None,
            title: "Logical plan validation".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let lifecycle = LifecycleStore::new(fixture.paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id.clone(),
            title: "Story".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            story_spec_ids: vec![story.id.clone()],
            title: "Design".to_string(),
            aggregate_codebase: None,
        })
        .expect("design");
    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            source_story_spec_ids: vec![story.id],
            source_design_spec_ids: vec![design.id],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec![],
            repository_profile_ref: None,
            verification_plan_ids: vec![],
            dependency_graph: vec![],
            created_from_provider_run: None,
            validator_findings: vec![],
        })
        .expect("plan");
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            entity_id: plan.id,
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::Codex),

            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");

    // Keep the already-built gateway in memory, then remove its policy artifact.  This
    // deterministically exercises the validate-failure early return without a real provider.
    let gateway = fixture.gateway.clone();
    std::fs::remove_file(fixture.paths.aggregate_policy_artifact_path("project_0001"))
        .expect("remove policy artifact");
    std::fs::remove_file(
        fixture
            .paths
            .logical_codebase_root("project_0001")
            .join("manifest.json"),
    )
    .expect("remove manifest to keep repository routing legacy");

    let (engine_tx, _engine_rx) = mpsc::channel::<crate::product::workspace_engine::EngineEvent>(8);
    let mut session = WorkspaceSession::from_record(session_record.clone());
    session.stage = WorkspaceStage::PrepareContext;
    session.repository_path = Some(repo_path);
    let engine = Arc::new(Mutex::new(
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(
                fixture.paths.root().join("checkpoints"),
            )),
            lifecycle,
            engine_tx,
            session,
        )
        .with_logical_provider_gateway(gateway),
    ));
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(CountingStreamingAdapter {
            starts: Arc::new(AtomicUsize::new(0)),
        }),
    );
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine,
        WorkspaceRunRegistry::default(),
        session_record.id.clone(),
        fixture.paths.clone(),
        session_record,
    );
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(8);

    spawn_provider_run_from_handler(
        run_context,
        ProviderRunKind::WorkItemPlanLegacyAuthor,
        outbound_tx,
    )
    .await
    .expect("validation failure is reported from the async handler task");

    let OutboundControl::Text(json) =
        tokio::time::timeout(std::time::Duration::from_secs(1), outbound_rx.recv())
            .await
            .expect("handler error outbound")
            .expect("handler error message")
    else {
        panic!("expected text error outbound");
    };
    let message: WsOutMessage = serde_json::from_str(&json).expect("ws error message");
    assert!(matches!(
        message,
        WsOutMessage::Error { ref message }
            if message.starts_with("logical plan launch failed:")
    ));
}

/// Task 2.8/REQ-PLN-07 + B2b1(REQ-PLN-08):planning author launch 的 cwd
/// 必须是 gateway 冻结的 canonical 聚合根;envelope target 同锚聚合根——
/// native anchor 不从 business working_dir/成员身份推导(checkout 身份为
/// 空),成员 checkout 只作 business 材料面。
#[test]
fn logical_plan_launch_uses_root_cwd_not_member_repository_path() {
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    }));
    let canonical_root = t14_canonical_root(&fixture);

    let launch = {
        let engine = fixture.engine.blocking_lock();
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };
    let PlanAuthorLaunch::Logical(plan) = &launch else {
        panic!("logical session must resolve a Logical launch");
    };
    assert_eq!(
        plan.launch.working_dir, canonical_root,
        "cwd 必须是 gateway 冻结的 canonical 聚合根"
    );
    let request = plan.launch.planning_request().expect("planning request");
    assert_eq!(
        request.working_directory, canonical_root,
        "envelope cwd 冻结 root"
    );
    assert_eq!(
        request.target.worktree, canonical_root,
        "B2b1:envelope target 恒聚合根锚(REQ-PLN-08 native anchor)"
    );
    assert!(
        request.target.checkout_id.is_empty(),
        "target 不携带成员 checkout 身份"
    );
    assert!(
        request.target.logical_repository_id.is_empty(),
        "target 不携带成员 logical 身份"
    );
    assert_eq!(
        request.readable_roots,
        vec![canonical_root],
        "readable roots=root"
    );
    assert!(request.writable_roots.is_empty(), "planning 只读写根为空");
}

// ---------------------------------------------------------------------------
// Task 1b 段②(lcg_t01):WS streaming Plan/split 的 run handle 收口与
// 全入口 projection/sink 绑定。
// ---------------------------------------------------------------------------

/// 段②(lcg_t01):WS plan/split caller 依次 begin handle、bind sink、start、
/// parse、complete/fail——成功收口 completed、失败收口 failed;provider_start
/// (adapter 首行审计)先于 split run completed 落盘。
#[tokio::test]
async fn lcg_t01_ws_plan_split_run_handle_closes_on_success_and_failure() {
    use crate::product::models::ProviderName;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // T1B-P3-1:时序断言改由 adapter 真实观测——启动标记在 registry 内
    // adapter 的 `start_validated` 路径写入共享计数器并快照,收口标记在
    // complete 落盘后取下一值;若 prepared 分流未触达 adapter,快照缺失
    // 即失败(不再是测试线程手工 fetch_add 的恒真式)。
    let seq = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(std::sync::Mutex::new(None));

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: Some(seq.clone()),
        observed_start_mark: probe.clone(),
    }));
    let plan_launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };

    let lifecycle = fixture.lifecycle.clone();
    let run_ctx = super::super::run::begin_plan_split_run(
        &lifecycle,
        &fixture.record.project_id,
        &fixture.record.issue_id,
        &ProviderName::ClaudeCode,
        &fixture.record.id,
    )
    .expect("begin plan split run");

    let input = streaming_input(fixture.member_checkout_root());
    let session = start_work_item_plan_author(
        plan_launch,
        Arc::new(CapturingStreamingAdapter {
            inputs: mpsc::unbounded_channel().0,
            start_mark: None,
            observed_start_mark: Arc::default(),
        }),
        input,
        CancellationToken::new(),
        Some(&run_ctx),
    )
    .await
    .expect("logical plan author start with run handle");

    // prepare 前已绑定 run-bound sink + workspace 会话身份。
    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert!(
        captured.audit_sink.is_some(),
        "LC 角色必须在 prepare 前绑定 run-bound audit sink"
    );
    assert_eq!(
        captured.workspace_session_id,
        Some(fixture.record.id.clone())
    );

    // provider_start(adapter 在 start_validated 路径写入的真实启动标记)
    // 先于 split run completed(complete 落盘后取下一标记)——T1B-P3-1。
    let provider_start_seq = probe
        .lock()
        .expect("start mark probe")
        .expect("adapter must have started via start_validated dispatch");
    let structured = serde_json::json!({"work_items": []});
    run_ctx
        .complete("plan prompt", &structured)
        .expect("complete consumes the plan split run handle");
    let split_completed_seq = seq.fetch_add(1, Ordering::SeqCst);
    assert!(
        provider_start_seq < split_completed_seq,
        "provider 启动标记({provider_start_seq})必须先于 split completed 标记({split_completed_seq})"
    );
    drop(session);

    // 失败路径:fail 收口 status=failed。
    let fail_ctx = super::super::run::begin_plan_split_run(
        &lifecycle,
        &fixture.record.project_id,
        &fixture.record.issue_id,
        &ProviderName::ClaudeCode,
        &fixture.record.id,
    )
    .expect("begin failing plan split run");
    fail_ctx
        .fail("provider failed before completion")
        .expect("fail closes the plan split run handle");
    let failed_run = lifecycle
        .read_work_item_split_provider_run_status(&fail_ctx.handle)
        .expect("read back failed plan split run");
    assert!(failed_run.status == "failed");
}

/// 段②(lcg_t01):所有 LC 入口经 gateway projection——Logical 启动只命中
/// registry 内 validated streaming 分发(cwd=root、target=成员、sink 已绑),
/// 直连 provider.start 计数为 0。
#[tokio::test]
async fn lcg_t01_all_lc_entrypoints_require_projection() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: input_tx,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let plan_launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };
    let expected_provider = crate::product::logical_codebase::ProviderRefType::ClaudeCode;
    let mapped_ref = match &plan_launch {
        super::super::run::PlanAuthorLaunch::Logical(plan) => {
            plan.launch.planning_request().expect("request").provider
        }
        _ => panic!("logical session must resolve a gateway launch"),
    };

    let direct_calls = Arc::new(AtomicUsize::new(0));
    let direct_provider: Arc<dyn StreamingProviderAdapter> = Arc::new(CountingStreamingAdapter {
        starts: direct_calls.clone(),
    });
    let member_worktree = fixture.member_checkout_root();
    let input = streaming_input(member_worktree.clone());

    let lifecycle = fixture.lifecycle.clone();
    let run_ctx = super::super::run::begin_plan_split_run(
        &lifecycle,
        &fixture.record.project_id,
        &fixture.record.issue_id,
        &crate::product::models::ProviderName::ClaudeCode,
        &fixture.record.id,
    )
    .expect("begin entrypoint plan split run");

    let session = start_work_item_plan_author(
        plan_launch,
        direct_provider,
        input,
        CancellationToken::new(),
        Some(&run_ctx),
    )
    .await
    .expect("logical start via gateway projection");
    drop(session);

    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    let observed_cwd = captured.working_directory.clone();
    let observed_target = captured.working_dir.clone();

    assert_eq!(mapped_ref.provider_type, expected_provider);
    assert_eq!(direct_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(observed_cwd, Some(t14_canonical_root(&fixture)));
    assert_eq!(observed_target, member_worktree);
    assert!(
        captured.audit_sink.is_some(),
        "所有 LC 入口必须经 prepare 绑定 run-bound sink(含无通用 tool_policy 角色)"
    );
}

/// B2b1(Task 5.2,REQ-PLN-08,r27 机制重钉):native anchor 收敛 E 聚合根后,
/// 成员锚/无锚的 envelope target 漂移机制结构性消失——任意两次 resolve 的
/// target 三元组恒等(root、无成员身份),显式 resume 的投影比对不再因
/// 锚形态漂移而恒拒;business 成员身份只留在材料面。
#[test]
fn plan_author_launch_anchor_drifts_target_and_fingerprint_without_member_anchor() {
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(CapturingStreamingAdapter {
        inputs: mpsc::unbounded_channel().0,
        start_mark: None,
        observed_start_mark: Arc::default(),
    }));
    let canonical_root = t14_canonical_root(&fixture);

    let (first, second) = {
        let engine = fixture.engine.blocking_lock();
        let first = resolve_plan_author_launch(&engine).expect("first launch");
        let second = resolve_plan_author_launch(&engine).expect("second launch");
        (first, second)
    };
    let (PlanAuthorLaunch::Logical(first), PlanAuthorLaunch::Logical(second)) = (&first, &second)
    else {
        panic!("logical session must resolve Logical launches");
    };
    let first_request = first.launch.planning_request().expect("request");
    let second_request = second.launch.planning_request().expect("request");
    for request in [&first_request, &second_request] {
        assert_eq!(request.target.worktree, canonical_root);
        assert!(request.target.checkout_id.is_empty());
        assert!(request.target.logical_repository_id.is_empty());
    }
    assert_eq!(
        first.validated.fingerprint().digest,
        second.validated.fingerprint().digest,
        "同会话两次 resolve 的 envelope 指纹必须恒等——r27 锚漂移恒拒机制已消除"
    );
}

// ---------------------------------------------------------------------------
// T6 fix-loop r2：WIP plan author 家族共享启动口（U4-U7 物理口）trust 分类
// 下沉——七已包 caller 与 followups 四续驱共用 `start_work_item_plan_author`
// 的 Logical 分支，此处对该物理口直测分类合同。
// ---------------------------------------------------------------------------

/// U4-U7 共享物理启动口：Logical 分支内统一分类——Kimi author 条目缺失
/// 时 spawn trust 拒绝以 `provider_trust_missing` 分类＋同源指导＋生成
/// 场景 retry_action 拒收；provider 零启动。
#[tokio::test]
async fn start_work_item_plan_author_logical_classifies_spawn_trust_failure() {
    use super::single_candidate_lc_admission::{
        AliasLcGatewayExtras, LcDialectCapabilitySource, lc_scoped_availability_gate,
    };
    use super::single_candidate_provider_run::ProviderRunFixture;

    let starts = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(CountingStreamingAdapter {
        starts: starts.clone(),
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider,
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(lc_scoped_availability_gate(true, true)),
        },
    );
    // B2b1:E native anchor 消费要求 durable 会话记录——直接复用 fixture
    // 的持久 engine(同 gateway/record),author 改 Kimi(workspace trust 面)。
    fixture.engine.lock().await.session.author_provider =
        crate::product::models::ProviderName::KimiCode;
    let plan_launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("kimi author logical launch")
    };
    let input = streaming_input(fixture.member_checkout_root());

    let result = start_work_item_plan_author(
        plan_launch,
        Arc::new(CountingStreamingAdapter {
            starts: starts.clone(),
        }),
        input,
        CancellationToken::new(),
        None,
    )
    .await;
    let error = match result {
        Ok(_) => panic!("plan author spawn must reject on missing trust"),
        Err(error) => error,
    };

    assert!(
        error.details.contains("provider_trust_missing"),
        "Logical 分支 trust 拒绝必须提升 provider_trust_missing 分类，got: {}",
        error.details
    );
    assert!(
        error
            .details
            .contains("provider trust not established for KimiCode"),
        "原拒绝原文必须保留，got: {}",
        error.details
    );
    assert!(
        error.details.contains("人工登记") && error.details.contains("显式重新生成"),
        "Logical 分支必须携带生成场景 retry_action，got: {}",
        error.details
    );
    assert!(
        error.details.contains(
            &trust_home
                .path()
                .join(".kimi-code")
                .join("workspace-trust")
                .display()
                .to_string()
        ),
        "Logical 分支指导必须同源（真实工件路径），got: {}",
        error.details
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "trust 拒绝必须发生在 provider 启动之前"
    );
}

/// U4-U7 负例：policy 漂移变体（TOCTOU）即使 trust 同时缺失也不得误附
/// trust 分类——分类下沉后变体判据保持在共享桥内。
#[test]
fn start_work_item_plan_author_drift_variant_stays_bare_at_bridge() {
    use super::single_candidate_lc_admission::{AliasLcGatewayExtras, LcDialectCapabilitySource};
    use super::single_candidate_provider_run::ProviderRunFixture;

    let provider = Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider,
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(
                super::single_candidate_lc_admission::lc_scoped_availability_gate(true, true),
            ),
        },
    );
    let gateway = fixture
        .engine
        .blocking_lock()
        .logical_provider_gateway()
        .expect("alias fixture must inject gateway")
        .clone();
    let drift = crate::product::logical_codebase::ProviderGatewayError::PolicyDrift {
        dimension: "config_digest".to_string(),
    };
    let details = gateway.classify_launch_trust_failure_details(
        &drift,
        &crate::product::models::ProviderName::KimiCode,
        &drift.to_string(),
        crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Generation,
    );
    assert_eq!(
        details,
        drift.to_string(),
        "漂移变体不得误附 trust 分类/指导（条目同时缺失也不误标），got: {details}"
    );
}

/// U4-U7 负例：条目已 trusted——即使错误变体是 ProviderUnavailable 也不
/// 得再附 trust 分类（只读判据当前已通过，无可指导事实）。
#[test]
fn start_work_item_plan_author_trusted_entry_stays_bare_at_bridge() {
    use super::single_candidate_lc_admission::{AliasLcGatewayExtras, LcDialectCapabilitySource};
    use super::single_candidate_provider_run::ProviderRunFixture;

    let provider = Arc::new(CountingStreamingAdapter {
        starts: Arc::new(AtomicUsize::new(0)),
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider,
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(
                super::single_candidate_lc_admission::lc_scoped_availability_gate(true, true),
            ),
        },
    );
    let gateway = fixture
        .engine
        .blocking_lock()
        .logical_provider_gateway()
        .expect("alias fixture must inject gateway")
        .clone();
    // 人工登记 Kimi 条目（同源 adapter 键算法/记录形状）。
    let adapter =
        crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::for_home(
            trust_home.path(),
        );
    let authority_root = gateway.authority_root().to_path_buf();
    let trust_key =
        crate::product::logical_codebase::provider_trust_adapters::ProviderTrustHomeAdapter::trust_key(
            &adapter, &authority_root,
        );
    let trust_dir = trust_home.path().join(".kimi-code").join("workspace-trust");
    std::fs::create_dir_all(&trust_dir).expect("create kimi trust dir");
    std::fs::write(
        trust_dir.join(&trust_key),
        serde_json::json!({
            "root": authority_root.display().to_string(),
            "trustedAt": 1_u64
        })
        .to_string(),
    )
    .expect("write trusted kimi entry");

    let unavailable = crate::product::logical_codebase::ProviderGatewayError::ProviderUnavailable(
        "provider trust not established for KimiCode: entry absent".to_string(),
    );
    let details = gateway.classify_launch_trust_failure_details(
        &unavailable,
        &crate::product::models::ProviderName::KimiCode,
        &unavailable.to_string(),
        crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Generation,
    );
    assert_eq!(
        details,
        unavailable.to_string(),
        "条目已 trusted 的 ProviderUnavailable 不得再附 trust 分类，got: {details}"
    );
}

// ---------------------------------------------------------------------------
// B2b1（Task 5.2，tasks.md 5.2 / 设计 §3/§4.2/§4.3，anchor_t14）：Plan
// full native 启动与 launch 纪元 map 绑定——E root 锚、prepared map before
// start、native audit 同 run bind、FreshRunning Resolved 收口。
// ---------------------------------------------------------------------------

/// anchor_t14 专用 adapter：start 时快照 durable prepared map（证明
/// prepared-before-start），并立即以固定 native id Completed。durable 定位
/// 经晚绑定 probe（fixture 构造后回填）。
struct T14NativeProbeAdapter {
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    observed_prepared:
        Arc<std::sync::Mutex<Vec<(crate::product::models::LogicalLaunchFingerprintState, u64)>>>,
    probe: Arc<std::sync::Mutex<Option<(crate::product::lifecycle_store::LifecycleStore, String)>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for T14NativeProbeAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        // provider start 时点快照 prepared map——若写入晚于 spawn，此快照
        // 不含本 run 绑定（红判据）。
        if let Some((lifecycle, session_id)) = self.probe.lock().expect("probe").clone() {
            let record = lifecycle
                .get_workspace_session(&session_id)
                .expect("durable session record at provider start");
            *self.observed_prepared.lock().expect("prepared probe") = record
                .logical_launch_fingerprints
                .iter()
                .map(|entry| (entry.state, entry.role_run_seq))
                .collect();
        }
        let _ = self.inputs.send(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(crate::cross_cutting::streaming_provider::ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        "<ARIA_STRUCTURED_OUTPUT nonce=\"t14\">{\"nonce\":\"t14\",\"work_items\":[]}</ARIA_STRUCTURED_OUTPUT>".to_string(),
                        Some("native-t14-fresh".to_string()),
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
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

/// B2b1 主用例（tasks.md 5.2 验收全链）：
/// 1) native anchor from E root：envelope target=canonical root、checkout 空；
/// 2) prepared map before start：provider start 时点 durable 已含本 run 的
///    Prepared 绑定（I/O 失败零 start 由 fail-closed 映射保证）；
/// 3) native audit 同 run bind：`persisted_binding.role_run_seq ==
///    actual_start_seq`（handle seq 即 audit sink 绑定 seq）；
/// 4) FreshRunning 的 Resolved 完成收口：fresh_role_run_seq/fresh_native_
///    session_id/fresh_node_id/operation Completed 同 run 对账。
#[tokio::test]
async fn anchor_t14_plan_native_anchor_and_map_are_full_and_durable() {
    use crate::product::models::{
        LcFreshOperationResult, LcFreshOperationState, LcResumeRecoveryRecord,
        LcResumeRecoveryState, LogicalLaunchBasis, LogicalLaunchFingerprintState,
        ProviderConversationRole,
    };

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let observed_prepared = Arc::new(std::sync::Mutex::new(Vec::new()));
    let probe = Arc::new(std::sync::Mutex::new(None));
    let fixture = t14_logical_fixture_with_gateway_extras(Arc::new(T14NativeProbeAdapter {
        inputs: input_tx,
        observed_prepared: observed_prepared.clone(),
        probe: probe.clone(),
    }));
    let canonical_root = t14_canonical_root(&fixture);
    *probe.lock().expect("bind probe") =
        Some((fixture.lifecycle.clone(), fixture.record.id.clone()));

    // 预置 FreshRunning recovery（dispatch 已受理并 spawn 成功的纪元前态）。
    let expected = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("baseline record");
    let now = chrono::Utc::now().to_rfc3339();
    fixture
        .lifecycle
        .compare_and_save_lc_resume_state(
            &expected,
            LcResumeRecoveryRecord {
                schema_version: 1,
                recovery_id: "recovery_anchor_t14".to_string(),
                failed_provider_node_id: "timeline_node_001".to_string(),
                reason_code: "planning_context_stale".to_string(),
                role: ProviderConversationRole::Author,
                provider: ProviderName::ClaudeCode,
                superseded_native_session_id: Some("native-t14-old".to_string()),
                previous_fingerprint_digest: Some("sha256:t14-old".to_string()),
                previous_session_status: expected.status.clone(),
                previous_stage: "running".to_string(),
                run_context: crate::product::models::LcRecoveryRunContext::LegacyOutlineRebuild {
                    business: crate::product::models::LcRecoveryBusinessContext {
                        entity_id: fixture.record.entity_id.clone(),
                        plan_id: None,
                        workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                        business_repository_physical_id: None,
                        business_repository_logical_id: None,
                        business_checkout_id: None,
                        business_repository_path: None,
                        source_story_ids: vec![],
                        source_design_ids: vec![],
                        plan_options_ref: None,
                        artifact_refs: vec![],
                        review_refs: vec![],
                        author_provider: ProviderName::ClaudeCode,
                        effective_reviewer_provider: None,
                        original_node_id: Some("timeline_node_001".to_string()),
                        original_role_run_seq: None,
                    },
                    origin: crate::product::models::LcOutlineRebuildOrigin::LegacyAuthor(
                        crate::product::models::LcLegacyOutlineAuthorPayload {
                            flow:
                                crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
                            original_provider_node_id: None,
                            outline_ref: None,
                        },
                    ),
                    previous_snapshot_access_fingerprint: None,
                },
                state: LcResumeRecoveryState::FreshRunning,
                operation_id: Some("op-t14-fresh".to_string()),
                fresh_node_id: None,
                fresh_native_session_id: None,
                fresh_role_run_seq: None,
                fresh_operations: vec![LcFreshOperationResult {
                    operation_id: "op-t14-fresh".to_string(),
                    state: LcFreshOperationState::Running,
                    fresh_node_id: None,
                    reason_code: None,
                    role_run_seq: None,
                    accepted_at: now.clone(),
                    updated_at: now,
                }],
                detected_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .expect("seed FreshRunning recovery");

    // 1) E root 锚（envelope target 三元组）。
    let launch = {
        let engine = fixture.engine.lock().await;
        resolve_plan_author_launch(&engine).expect("resolve logical launch")
    };
    let PlanAuthorLaunch::Logical(plan) = &launch else {
        panic!("logical session must resolve a Logical launch");
    };
    let envelope = plan.validated.envelope();
    assert_eq!(envelope.target.worktree, canonical_root);
    assert!(envelope.target.checkout_id.is_empty());
    assert!(envelope.target.logical_repository_id.is_empty());

    // run 身份 + provider 启动 + 驱动至 Completed + 成功收口（arm 等价链）。
    let run_ctx = super::super::run::begin_plan_split_run(
        &fixture.lifecycle,
        &fixture.record.project_id,
        &fixture.record.issue_id,
        &ProviderName::ClaudeCode,
        &fixture.record.id,
    )
    .expect("begin plan split run");
    let actual_start_seq = run_ctx.handle.role_run_seq;

    let (node_id, full_output) = {
        let mut engine = fixture.engine.lock().await;
        let node_id = engine.begin_work_item_plan_author_run().await;
        let input = streaming_input(fixture.member_checkout_root());
        let session = start_work_item_plan_author(
            launch,
            Arc::new(CountingStreamingAdapter {
                starts: Arc::new(AtomicUsize::new(0)),
            }),
            input,
            CancellationToken::new(),
            Some(&run_ctx),
        )
        .await
        .expect("logical start with run handle");
        let (_command_tx, mut command_rx) = mpsc::channel(8);
        let output = engine
            .drive_work_item_plan_provider_session_to_output(
                Ok(session),
                &mut command_rx,
                node_id.clone(),
                ProviderName::ClaudeCode,
            )
            .await
            .expect("drive to Completed");
        (node_id, output)
    };
    close_plan_split_run(Some(&run_ctx), "t14 prompt", &full_output);

    // 2) prepared map before start（provider start 时点快照含本 run 绑定）。
    let snapshot = observed_prepared.lock().expect("prepared snapshot").clone();
    assert!(
        snapshot.iter().any(|(state, seq)| {
            *state == LogicalLaunchFingerprintState::Prepared && *seq == actual_start_seq
        }),
        "provider start 前 durable 必须已含本 run 的 Prepared 绑定,got: {snapshot:?}"
    );

    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("capture adapter input")
        .expect("captured input");
    assert!(
        captured.audit_sink.is_some(),
        "run-bound audit sink 必须绑定"
    );

    // 3)+4) durable 终态：map Active/native 绑定 + recovery Resolved 对账。
    let record = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("final record");
    let binding = record
        .logical_launch_fingerprints
        .iter()
        .find(|entry| {
            entry.role == ProviderConversationRole::Author
                && entry.provider == ProviderName::ClaudeCode
                && entry.role_run_seq == actual_start_seq
        })
        .expect("persisted launch binding");
    assert_eq!(binding.state, LogicalLaunchFingerprintState::Active);
    assert_eq!(
        binding.role_run_seq, actual_start_seq,
        "native audit 同 run bind:map 绑定 seq 必须==实际启动 seq"
    );
    assert_eq!(
        binding.native_session_id.as_deref(),
        Some("native-t14-fresh")
    );
    assert_eq!(binding.node_id, node_id);
    assert_eq!(binding.basis, LogicalLaunchBasis::FullLcProjectionV2);
    assert!(
        binding
            .fingerprint_digest
            .contains("lc-session-resume-fingerprint/2")
    );

    let recovery = record.lc_resume_recovery.expect("recovery kept").clone();
    assert_eq!(recovery.state, LcResumeRecoveryState::Resolved);
    assert_eq!(recovery.fresh_role_run_seq, Some(actual_start_seq));
    assert_eq!(
        recovery.fresh_native_session_id.as_deref(),
        Some("native-t14-fresh")
    );
    assert_eq!(recovery.fresh_node_id.as_deref(), Some(node_id.as_str()));
    assert_eq!(
        recovery.fresh_operations[0].state,
        LcFreshOperationState::Completed
    );
    assert_eq!(
        recovery.fresh_operations[0].role_run_seq,
        Some(actual_start_seq)
    );
}
