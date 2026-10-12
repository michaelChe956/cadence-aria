//! trust 指引卡「登记并重试」按钮（owner 2026-10-12 直批，免双审裁定已
//! 入账）：`RegisterTrustEntry` WS 动作的服务端行为测试。
//!
//! 覆盖：
//! - root 服务端解析：LC 会话取确认链同一冻结根（gateway authority），
//!   wire 上不携带客户端路径字段；单仓会话取 repository 根（root 解析差异）；
//! - 复用现有 ensure 登记链：首次 registered、重复 replay、用户既有条目
//!   reuse_user_entry 不覆盖；ensure 审计（action=ensure，outcome 三态）
//!   的 operation_id 以 `register_trust_entry:<session>` 关联本会话；
//! - typed 回执（trust_entry_registered）与失败稳定码
//!   （trust_registration_waiting，reason_code 原样保留不折叠）；
//! - 无需 workspace trust 的 provider 显式拒绝（零 ensure 调用）；
//! - 显式按钮授权不自动重新生成：成功仅回执，零 provider 启动。

use super::*;
use crate::product::logical_codebase::provider_trust::ProviderTrustScenario;
use crate::product::logical_codebase::{
    HomeBackedProviderTrustRegistry, ProviderTrustAuditRecord, ProviderTrustHomeAdapter,
    ProviderTrustPrecondition, ProviderTrustPreparationResult, ProviderTrustStore,
    ProviderTrustWaiting,
};
use crate::web::workspace_ws_types::TrustEntryRegistrationActionDto;

/// 真实登记链夹具：LC 会话（presence gateway + manifest 冻结根）+ 确认
/// 信任上下文注入指定 precondition（真实 registry 或脚本 double）。
struct RegisterTrustFixture {
    root: tempfile::TempDir,
    home: tempfile::TempDir,
    lifecycle: LifecycleStore,
    app_paths: ProductAppPaths,
    session_id: String,
    lc_id: String,
    canonical_root: std::path::PathBuf,
}

#[allow(clippy::type_complexity)]
fn register_trust_fixture(
    precondition: Option<Arc<dyn ProviderTrustPrecondition>>,
) -> (RegisterTrustFixture, Arc<Mutex<WorkspaceEngine>>) {
    let root = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("home tempdir");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let worktree = root.path().join("register-trust-aggregate");
    std::fs::create_dir_all(&worktree).expect("create aggregate root");
    let canonical_root = std::fs::canonicalize(&worktree).expect("canonical aggregate root");
    let lc_id = "lc_register_trust_0001".to_string();
    let issue = IssueStore::new(app_paths.clone())
        .create(crate::product::issue_store::CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: Some(lc_id.clone()),
            base_branch: None,
            title: "register trust fixture".to_string(),
            description: None,
            change_id: None,
        })
        .expect("create LC issue");
    crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        app_paths.clone(),
        lc_id.clone(),
    )
    .save_manifest(
        "project_0001",
        &crate::product::logical_codebase::LogicalCodebaseManifest::new(
            "project_0001",
            canonical_root.clone(),
            Vec::new(),
        ),
    )
    .expect("save LC manifest");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let record = lifecycle
        .create_workspace_session(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: issue.id.clone(),
                entity_id: "plan_register_trust".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
        )
        .expect("create workspace session");
    let session_id = record.id.clone();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let session = WorkspaceSession::from_record(record.clone());
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(register_trust_presence_gateway(canonical_root.clone()));
    if let Some(precondition) = precondition {
        engine = engine.with_confirmation_trust_context(
            crate::product::workspace_engine::conversational_gate::ConfirmationTrustContext {
                precondition,
                issue_lc_id: Some(lc_id.clone()),
                frozen_root: canonical_root.clone(),
            },
        );
    }
    let engine = Arc::new(Mutex::new(engine));
    (
        RegisterTrustFixture {
            root,
            home,
            lifecycle,
            app_paths,
            session_id,
            lc_id,
            canonical_root,
        },
        engine,
    )
}

/// 真实登记链（临时 HOME 的 Codex/Kimi adapter）。
fn real_registry(
    fixture_home: &std::path::Path,
    app_paths: &ProductAppPaths,
) -> Arc<HomeBackedProviderTrustRegistry> {
    Arc::new(HomeBackedProviderTrustRegistry::new(
        app_paths.clone(),
        vec![
            Arc::new(crate::product::logical_codebase::CodexTrustAdapter::for_home(fixture_home)),
            Arc::new(crate::product::logical_codebase::KimiTrustAdapter::for_home(fixture_home)),
        ],
    ))
}

/// presence-only gateway：登记按钮只把 gateway 用作「LC 会话」谓词。
fn register_trust_presence_gateway(
    authority_root: std::path::PathBuf,
) -> Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway> {
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::product::logical_codebase::ProviderGatewayError;

    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    macro_rules! unsupported {
        () => {
            Err(ProviderGatewayError::UnsupportedCapability(
                "presence-only fixture gateway".to_string(),
            ))
        };
    }

    struct PresenceCapabilitySource;

    impl crate::product::logical_codebase::ProviderCapabilitySource for PresenceCapabilitySource {
        fn require_supported(
            &self,
            _provider: &crate::product::logical_codebase::ProviderRef,
            _action: crate::product::logical_codebase::SessionPolicyAction,
        ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError>
        {
            unsupported!()
        }

        fn require_resume_supported(
            &self,
            _provider: &crate::product::logical_codebase::ProviderRef,
            _action: crate::product::logical_codebase::SessionPolicyAction,
        ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError>
        {
            unsupported!()
        }

        fn require_write_boundary(
            &self,
            _provider: &crate::product::logical_codebase::ProviderRef,
            _action: crate::product::logical_codebase::SessionPolicyAction,
        ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError>
        {
            unsupported!()
        }

        fn require_root_recipe_supported(
            &self,
            _provider: &crate::product::logical_codebase::ProviderRef,
            _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError>
        {
            unsupported!()
        }
    }

    struct PresenceTargetResolver;

    impl crate::product::logical_codebase::PolicyTargetResolver for PresenceTargetResolver {
        fn resolve_and_revalidate(
            &self,
            _request: &crate::product::logical_codebase::SessionLaunchRequest,
        ) -> Result<crate::product::logical_codebase::PolicyTarget, ProviderGatewayError> {
            unsupported!()
        }
    }

    struct PresenceSyncAdapter;

    impl crate::cross_cutting::provider_adapter::ProviderAdapter for PresenceSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<
            crate::protocol::contracts::AdapterOutput,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            Ok(crate::protocol::contracts::AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
            })
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
    let paths = ProductAppPaths::new(std::env::temp_dir().join("aria-register-trust-presence"));
    Arc::new(
        crate::product::logical_codebase::LogicalCodebaseProviderGateway::new(
            crate::product::logical_codebase::AggregatePolicyArtifactStore::new(paths),
            Arc::new(PresenceCapabilitySource),
            Arc::new(PresenceTargetResolver),
            Arc::new(ProviderRegistry::new()),
            Arc::new(PresenceSyncAdapter),
            Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
                snapshot,
            )))),
            authority_root,
        ),
    )
}

impl RegisterTrustFixture {
    fn build_inbound(
        &self,
        engine: Arc<Mutex<WorkspaceEngine>>,
    ) -> (WorkspaceInboundContext, mpsc::Receiver<OutboundControl>) {
        let record = self
            .lifecycle
            .get_workspace_session(&self.session_id)
            .expect("fixture durable record");
        let (outbound_tx, outbound_rx) = mpsc::channel(8);
        let context = WorkspaceInboundContext {
            app_state: WebAppState::new(
                self.root.path().to_path_buf(),
                crate::web::runtime::WebRuntime::new_fake(self.root.path().to_path_buf()),
            ),
            engine: engine.clone(),
            run_context: ProviderRunContext::test_fixture(
                Arc::new(ProviderRegistry::new()),
                engine.clone(),
                WorkspaceRunRegistry::default(),
                self.session_id.clone(),
                self.app_paths.clone(),
                record,
            ),
            outbound_tx,
            session_id: self.session_id.clone(),
        };
        (context, outbound_rx)
    }

    fn audit(&self) -> Vec<ProviderTrustAuditRecord> {
        ProviderTrustStore::for_lc(self.app_paths.clone(), self.lc_id.clone())
            .list_audit("project_0001")
            .expect("list audit")
    }
}

async fn next_outbound(rx: &mut mpsc::Receiver<OutboundControl>) -> WsOutMessage {
    let outbound = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("outbound frame must arrive")
        .expect("outbound channel open");
    let OutboundControl::Text(json) = outbound else {
        panic!("expected text frame, got {outbound:?}")
    };
    serde_json::from_str(&json).expect("outbound json")
}

/// 幂等两连击：首次 registered、二次 replay；ensure 审计两笔且
/// operation_id 以 `register_trust_entry:<session>` 关联本会话；回执
/// canonical_root 是服务端冻结根（wire 上无客户端路径字段）。
#[tokio::test]
async fn register_trust_entry_registers_then_replays_with_audit_and_frozen_root() {
    let (fixture, engine) = register_trust_real_fixture();
    let (context, mut outbound_rx) = fixture.build_inbound(engine);

    handle_workspace_inbound_message(
        context.clone(),
        WsInMessage::RegisterTrustEntry {
            provider: ProviderName::Codex,
        },
    )
    .await;
    let first = next_outbound(&mut outbound_rx).await;
    let WsOutMessage::TrustEntryRegistered {
        provider,
        action,
        trust_key,
        canonical_root,
        ..
    } = first
    else {
        panic!("expected typed trust_entry_registered receipt, got {first:?}")
    };
    assert_eq!(provider, ProviderName::Codex);
    assert_eq!(action, TrustEntryRegistrationActionDto::Registered);
    assert_eq!(canonical_root, fixture.canonical_root.display().to_string());
    let adapter =
        crate::product::logical_codebase::CodexTrustAdapter::for_home(fixture.home.path());
    assert_eq!(trust_key, adapter.trust_key(&fixture.canonical_root));

    handle_workspace_inbound_message(
        context,
        WsInMessage::RegisterTrustEntry {
            provider: ProviderName::Codex,
        },
    )
    .await;
    let second = next_outbound(&mut outbound_rx).await;
    let WsOutMessage::TrustEntryRegistered { action, .. } = second else {
        panic!("expected typed receipt on replay, got {second:?}")
    };
    assert_eq!(action, TrustEntryRegistrationActionDto::Replay);

    let audit = fixture.audit();
    let ensure_audit: Vec<_> = audit
        .iter()
        .filter(|record| record.action == "ensure")
        .collect();
    assert_eq!(ensure_audit.len(), 2, "audit: {audit:?}");
    assert_eq!(ensure_audit[0].outcome, "registered");
    assert_eq!(ensure_audit[1].outcome, "replay");
    let expected_operation = format!("register_trust_entry:{}", fixture.session_id);
    for record in &ensure_audit {
        assert_eq!(
            record.operation_id,
            Some(expected_operation.clone()),
            "audit operation_id must bind the explicit button action to the session: {record:?}"
        );
    }

    let config = std::fs::read_to_string(fixture.home.path().join(".codex").join("config.toml"))
        .expect("codex config written");
    assert!(config.contains("trust_level = \"trusted\""), "{config}");
}

/// 真实链路夹具变体：确认上下文注入真实 registry。
fn register_trust_real_fixture() -> (RegisterTrustFixture, Arc<Mutex<WorkspaceEngine>>) {
    let root = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("home tempdir");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let worktree = root.path().join("register-trust-aggregate");
    std::fs::create_dir_all(&worktree).expect("create aggregate root");
    let canonical_root = std::fs::canonicalize(&worktree).expect("canonical aggregate root");
    let lc_id = "lc_register_trust_0001".to_string();
    let issue = IssueStore::new(app_paths.clone())
        .create(crate::product::issue_store::CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: Some(lc_id.clone()),
            base_branch: None,
            title: "register trust fixture".to_string(),
            description: None,
            change_id: None,
        })
        .expect("create LC issue");
    crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        app_paths.clone(),
        lc_id.clone(),
    )
    .save_manifest(
        "project_0001",
        &crate::product::logical_codebase::LogicalCodebaseManifest::new(
            "project_0001",
            canonical_root.clone(),
            Vec::new(),
        ),
    )
    .expect("save LC manifest");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let record = lifecycle
        .create_workspace_session(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: issue.id.clone(),
                entity_id: "plan_register_trust_real".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
        )
        .expect("create workspace session");
    let session_id = record.id.clone();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let session = WorkspaceSession::from_record(record.clone());
    let engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(register_trust_presence_gateway(canonical_root.clone()))
    .with_confirmation_trust_context(
        crate::product::workspace_engine::conversational_gate::ConfirmationTrustContext {
            precondition: real_registry(home.path(), &app_paths),
            issue_lc_id: Some(lc_id.clone()),
            frozen_root: canonical_root.clone(),
        },
    );
    let engine = Arc::new(Mutex::new(engine));
    (
        RegisterTrustFixture {
            root,
            home,
            lifecycle,
            app_paths,
            session_id,
            lc_id,
            canonical_root,
        },
        engine,
    )
}

/// 用户既有条目（无 LC 归属记录）：reuse_user_entry，home 字节不动。
#[tokio::test]
async fn register_trust_entry_reuses_user_entry_without_overwrite() {
    let (fixture, engine) = register_trust_real_fixture();
    let adapter =
        crate::product::logical_codebase::CodexTrustAdapter::for_home(fixture.home.path());
    std::fs::create_dir_all(fixture.home.path().join(".codex")).unwrap();
    let user_config = format!(
        "{}\ntrust_level = \"trusted\"\n# user note\n",
        adapter.trust_key(&fixture.canonical_root)
    );
    std::fs::write(
        fixture.home.path().join(".codex").join("config.toml"),
        &user_config,
    )
    .unwrap();
    let (context, mut outbound_rx) = fixture.build_inbound(engine);

    handle_workspace_inbound_message(
        context,
        WsInMessage::RegisterTrustEntry {
            provider: ProviderName::Codex,
        },
    )
    .await;
    let receipt = next_outbound(&mut outbound_rx).await;
    let WsOutMessage::TrustEntryRegistered { action, .. } = receipt else {
        panic!("expected typed receipt, got {receipt:?}")
    };
    assert_eq!(action, TrustEntryRegistrationActionDto::ReuseUserEntry);
    let after = std::fs::read_to_string(fixture.home.path().join(".codex").join("config.toml"))
        .expect("user config intact");
    assert_eq!(after, user_config, "user-owned entry must stay untouched");
}

/// 无需 workspace trust 的 provider：typed 拒收，零 ensure 调用。
#[tokio::test]
async fn register_trust_entry_rejects_provider_that_needs_no_trust() {
    let (fixture, engine) = register_trust_real_fixture();
    let (context, mut outbound_rx) = fixture.build_inbound(engine);

    handle_workspace_inbound_message(
        context,
        WsInMessage::RegisterTrustEntry {
            provider: ProviderName::ClaudeCode,
        },
    )
    .await;
    let err = next_outbound(&mut outbound_rx).await;
    let WsOutMessage::ProtocolError { code, .. } = err else {
        panic!("expected protocol error, got {err:?}")
    };
    assert_eq!(code, "provider_trust_not_required");
    assert!(fixture.audit().is_empty(), "no ensure must run");
}

/// ensure 失败：稳定码 trust_registration_waiting 的 ProtocolError，
/// reason_code/manual_guidance/retry_action 原样上浮（不折叠）。
#[tokio::test]
async fn register_trust_entry_waiting_is_stable_protocol_error_with_guidance() {
    let scripted: Arc<WaitingPrecondition> = Arc::new(WaitingPrecondition::default());
    scripted.script_waiting();
    let (fixture, engine) = register_trust_fixture(Some(scripted));
    let (context, mut outbound_rx) = fixture.build_inbound(engine);

    handle_workspace_inbound_message(
        context,
        WsInMessage::RegisterTrustEntry {
            provider: ProviderName::Codex,
        },
    )
    .await;
    let err = next_outbound(&mut outbound_rx).await;
    let WsOutMessage::ProtocolError {
        code,
        message,
        context,
    } = err
    else {
        panic!("expected protocol error, got {err:?}")
    };
    assert_eq!(code, "trust_registration_waiting");
    assert!(message.contains("fixture 登记等待原因"), "{message}");
    assert!(
        message.contains("fixture 人工指导：修复 Codex home 配置"),
        "{message}"
    );
    assert!(message.contains("fixture_retry_action"), "{message}");
    let context = context.expect("waiting context");
    assert_eq!(context["reason_code"], "codex_trust_key_conflict");
    assert_eq!(context["trust_key"], "fixture-trust-key");
    assert_eq!(context["retryable"], true);
}

/// LC 会话缺确认信任上下文（生产 manager 缺 trust 依赖同构）：
/// fail-closed ContextWaiting 形稳定码，reason_code 原样。
#[tokio::test]
async fn register_trust_entry_without_trust_context_returns_context_waiting() {
    let (fixture, engine) = register_trust_fixture(None);
    let (context, mut outbound_rx) = fixture.build_inbound(engine);

    handle_workspace_inbound_message(
        context,
        WsInMessage::RegisterTrustEntry {
            provider: ProviderName::Codex,
        },
    )
    .await;
    let err = next_outbound(&mut outbound_rx).await;
    let WsOutMessage::ProtocolError {
        code,
        message,
        context,
    } = err
    else {
        panic!("expected protocol error, got {err:?}")
    };
    assert_eq!(code, "trust_registration_waiting");
    assert!(message.contains("登记停等"), "{message}");
    assert_eq!(
        context.expect("context")["reason_code"],
        serde_json::json!("trust_registration_context_unavailable")
    );
}

/// wire 面：动作名 register_trust_entry、provider 为唯一业务字段
/// （不携带任何客户端路径字段）。
#[test]
fn register_trust_entry_wire_shape_carries_provider_only() {
    let parsed: WsInMessage =
        serde_json::from_str(r#"{"type":"register_trust_entry","provider":"kimi_code"}"#)
            .expect("wire parse");
    assert_eq!(
        parsed,
        WsInMessage::RegisterTrustEntry {
            provider: ProviderName::KimiCode,
        }
    );
    let wire = serde_json::to_value(&parsed).unwrap();
    assert_eq!(wire["type"], "register_trust_entry");
    assert_eq!(wire["provider"], "kimi_code");
    assert!(wire.get("canonical_root").is_none() && wire.get("root").is_none());
}

/// 单仓会话（无 gateway）：root 解析差异——取 canonicalized repository
/// 根，registry lc 标签走 legacy 别名（与 coordinator 单仓 scope 同源）。
#[tokio::test]
async fn register_trust_entry_single_repo_resolves_repository_root() {
    let root = tempfile::tempdir().expect("tempdir");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let repo = root.path().join("single-repo");
    std::fs::create_dir_all(&repo).unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let record = lifecycle
        .create_workspace_session(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0002".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "plan_single_repo".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
        )
        .expect("create single-repo session");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = WorkspaceSession::from_record(record);
    session.repository_path = Some(repo.clone());
    let engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        event_tx,
        session,
    );
    let fallback: Arc<dyn ProviderTrustPrecondition> =
        Arc::new(HomeBackedProviderTrustRegistry::new(
            app_paths.clone(),
            vec![Arc::new(
                crate::product::logical_codebase::CodexTrustAdapter::for_home(root.path()),
            )],
        ));
    let target = engine
        .prepare_explicit_trust_registration(Some(fallback))
        .expect("single-repo target must resolve");
    assert_eq!(
        target.canonical_root,
        std::fs::canonicalize(&repo).unwrap(),
        "single-repo root comes from the session repository path"
    );
    assert_eq!(
        target.registry_lc_id,
        crate::product::logical_codebase::legacy_logical_codebase_id("project_0002"),
        "single-repo scope uses the legacy alias label"
    );
}

// ---- 等待脚本 precondition（本模块私有 double） ----

#[derive(Default)]
struct WaitingPrecondition {
    scripted: std::sync::Mutex<Vec<ProviderTrustPreparationResult>>,
}

impl WaitingPrecondition {
    fn script_waiting(&self) {
        self.scripted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(ProviderTrustPreparationResult::Waiting {
                waiting: ProviderTrustWaiting {
                    provider: ProviderName::Codex,
                    canonical_root: std::path::PathBuf::from("/fixture-trust-root"),
                    trust_key: "fixture-trust-key".to_string(),
                    reason_code: "codex_trust_key_conflict".to_string(),
                    message: "fixture 登记等待原因（home 读取失败）".to_string(),
                    retry_action: "fixture_retry_action".to_string(),
                    manual_guidance: "fixture 人工指导：修复 Codex home 配置".to_string(),
                    command_markdown: None,
                    recorded_at: "2026-10-12T00:00:00Z".to_string(),
                },
            });
    }
}

impl ProviderTrustPrecondition for WaitingPrecondition {
    fn ensure_ready(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _lc_id: &str,
        _canonical_root: &std::path::Path,
        _providers: &[ProviderName],
        _scenario: ProviderTrustScenario,
    ) -> ProviderTrustPreparationResult {
        self.scripted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop()
            .unwrap_or(ProviderTrustPreparationResult::Ready {
                registrations: Vec::new(),
            })
    }
}
