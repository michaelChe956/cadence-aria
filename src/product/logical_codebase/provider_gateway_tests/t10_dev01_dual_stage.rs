//! 批2 Task 10(DEV-01):双阶段同源准入——生产 store 的桥形态记录在
//! validate 与 spawn 复验都拒绝、零 spawn;受信 builder 接线前置守卫
//! (RevB2T2 ⚠2/⚠3/⚠4:身份一致断言/合同装饰对称/resume 稳定摘要)。
use super::*;
use crate::cross_cutting::provider_execution::{
    ProviderExecutionProfile, TrustedProviderRole, TrustedRunIdentity,
};
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::production_policy_resolvers::StoreBackedProviderCapabilitySource;
use crate::product::logical_codebase::provider_capability_store::{
    CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
    ProviderActionMatrix, ProviderCapabilityRecord, ProviderCapabilityStore, RootRecipeEvidence,
};
use crate::product::logical_codebase::provider_gateway::ProviderRefType;
use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
use crate::protocol::contracts::AdapterRole;

struct T10StoreFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    store: ProviderCapabilityStore,
    streaming_adapter: Arc<CountingStreamingAdapter>,
    audit: Arc<GatewayRunAudit>,
}

impl T10StoreFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path());
        let store = ProviderCapabilityStore::new(paths.clone());
        store.ensure_bootstrap("project_0001").expect("bootstrap");
        Self {
            _root: root,
            paths,
            store,
            streaming_adapter: Arc::new(CountingStreamingAdapter::new()),
            audit: Arc::new(GatewayRunAudit::new()),
        }
    }

    fn upsert(&self, record: &ProviderCapabilityRecord) {
        self.store
            .upsert("project_0001", record)
            .expect("upsert capability record");
    }

    fn gateway(&self) -> LogicalCodebaseProviderGateway {
        let manifest =
            LogicalCodebaseManifest::new("project_0001", self.paths.root().to_path_buf(), vec![]);
        let policy_store = AggregatePolicyArtifactStore::new(self.paths.clone());
        if policy_store.get("project_0001").ok().flatten().is_none() {
            policy_store
                .ensure_bootstrap(&manifest)
                .expect("bootstrap policy");
        }
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, self.streaming_adapter.clone());
        let authority_root =
            std::fs::canonicalize(manifest.provider_context_root).expect("authority root");
        LogicalCodebaseProviderGateway::with_audit(
            AggregatePolicyArtifactStore::new(self.paths.clone()),
            Arc::new(StoreBackedProviderCapabilitySource::new(
                self.paths.clone(),
                "project_0001".to_string(),
            )),
            Arc::new(MutableTargetResolver::new(
                self.paths.root().join("baseline.git"),
            )),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            self.audit.clone(),
            authority_root,
        )
    }

    fn planning_request(&self, worktree: PathBuf) -> SessionLaunchRequest {
        SessionLaunchRequest::planning(
            "project_0001",
            ProviderRef::claude_code("cap_managed_snapshot"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
            vec![self.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        )
    }
}

fn confirmed_planning_row() -> ProviderActionCapability {
    ProviderActionCapability {
        action: SessionPolicyAction::PlanningReadOnly,
        launch: ProviderCapabilityEvidence::Confirmed,
        resume: ProviderCapabilityEvidence::Confirmed,
        write_boundary: ProviderCapabilityEvidence::Confirmed,
        projection_digest: "sha256:t10-probe".to_string(),
        evidence_ref: "probe://t10".to_string(),
    }
}

fn record(
    matrix: ProviderActionMatrix,
    supported: Vec<SessionPolicyAction>,
) -> ProviderCapabilityRecord {
    ProviderCapabilityRecord {
        provider_type: ProviderRefType::ClaudeCode,
        schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
        version: "1.4.0".to_string(),
        adapter_dialect: crate::product::logical_codebase::policy::ProviderDialect::ClaudeCodeCliV1,
        wire_dialect:
            crate::product::logical_codebase::policy::ProviderWireDialect::ClaudeCodeStreamJson,
        capability_snapshot_ref: "cap_managed_snapshot".to_string(),
        evidence: CapabilityEvidence::ProductionVerified,
        resume_evidence: ResumeEvidenceState::Confirmed,
        supported_actions: supported,
        action_matrix: matrix,
        trust: ProviderCapabilityEvidence::Confirmed,
        probed_at: Some("2026-10-17T00:00:00Z".to_string()),
        probe_artifact_ref: Some("probe://t10-artifact".to_string()),
        root_recipe_evidence: RootRecipeEvidence::None,
        execution_semantics:
            crate::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN
                .to_string(),
    }
}

fn streaming_input(
    worktree: PathBuf,
) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
    crate::cross_cutting::streaming_provider::StreamingProviderInput {
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
        provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
        role: AdapterRole::Executor,
        prompt: "t10".to_string(),
        working_dir: worktree,
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: Default::default(),
        timeout_secs: 1,
    }
}

fn audit_context() -> ProviderLaunchAuditContext {
    ProviderLaunchAuditContext {
        workspace_session_id: "ws-t10-0001".to_string(),
        role_run_seq: 1,
        audit_sink:
            crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink::new(
            ),
    }
}

/// DEV-01 主断言:生产 store 桥形态(矩阵全 Unknown+旧 allow 全列)在
/// validate 阶段与 spawn 前复验都拒绝、零 provider 启动、零审计启动。
#[tokio::test]
async fn dev01_bridge_shape_rejected_at_validate_and_spawn_zero_spawns() {
    let fixture = T10StoreFixture::new();
    let worktree = fixture.paths.root().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    fixture.upsert(&record(
        ProviderActionMatrix::from_rows(vec![confirmed_planning_row()]).unwrap(),
        Vec::new(),
    ));
    let gateway = fixture.gateway();
    let prepared = gateway
        .prepare_streaming_launch(
            streaming_input(worktree.clone()),
            fixture.planning_request(worktree.clone()),
            audit_context(),
        )
        .expect("confirmed record prepares");
    fixture.upsert(&record(
        ProviderActionMatrix::unknown_all(),
        vec![
            SessionPolicyAction::PlanningReadOnly,
            SessionPolicyAction::CodingTargetWrite,
            SessionPolicyAction::ReviewReadOnly,
        ],
    ));
    let validate_error = gateway
        .prepare_streaming_launch(
            streaming_input(worktree.clone()),
            fixture.planning_request(worktree.clone()),
            audit_context(),
        )
        .expect_err("bridge shape must be refused at validate");
    assert!(
        matches!(&validate_error, ProviderGatewayError::UnsupportedCapability(reason)
            if reason.starts_with("provider_capability_launch_not_confirmed")),
        "unexpected validate refusal: {validate_error}"
    );
    let spawn_error = match gateway
        .start_streaming(prepared, CancellationToken::new())
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("bridge shape must be refused at spawn revalidation"),
    };
    assert!(
        matches!(&spawn_error, ProviderGatewayError::UnsupportedCapability(reason)
            if reason.starts_with("provider_capability_launch_not_confirmed")
                || reason.starts_with("provider_capability_write_boundary_not_confirmed")),
        "unexpected spawn refusal: {spawn_error}"
    );
    assert_eq!(fixture.streaming_adapter.start_count(), 0);
    assert_eq!(fixture.audit.stream_launches(), 0);
}

/// RevB2T2 ⚠3:受信 identity 与启动载体一致门——会话错接与承载角色
/// 错接都在 freeze/validate 之前零副作用拒绝。
#[test]
fn trusted_identity_mismatch_is_refused_before_freeze() {
    let fixture = T10StoreFixture::new();
    let worktree = fixture.paths.root().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    fixture.upsert(&record(
        ProviderActionMatrix::from_rows(vec![confirmed_planning_row()]).unwrap(),
        Vec::new(),
    ));
    let mut context = audit_context();
    context.workspace_session_id = "ws-t10-other".to_string();
    let identity = TrustedRunIdentity::from_durable_run(
        &durable_run_record_t10("run_t10_0001", AdapterRole::Executor),
        "ws-t10-0001",
        TrustedProviderRole::Coder,
    );
    let session_error = fixture
        .gateway()
        .prepare_trusted_streaming_launch(
            streaming_input(worktree.clone()),
            fixture.planning_request(worktree.clone()),
            context,
            &identity,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_t10".to_string(),
            None,
        )
        .expect_err("cross-session identity must be refused");
    assert!(
        matches!(&session_error, ProviderGatewayError::UnsupportedCapability(reason)
            if reason.contains("identity_session_mismatch")),
        "unexpected: {session_error}"
    );
    let orchestrator_identity = TrustedRunIdentity::from_durable_run(
        &durable_run_record_t10("run_t10_0002", AdapterRole::Orchestrator),
        "ws-t10-0001",
        TrustedProviderRole::Author,
    );
    let carrier_error = fixture
        .gateway()
        .prepare_trusted_streaming_launch(
            streaming_input(worktree.clone()),
            fixture.planning_request(worktree),
            audit_context(),
            &orchestrator_identity,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_t10".to_string(),
            None,
        )
        .expect_err("carrier/role mismatch must be refused");
    assert!(
        matches!(&carrier_error, ProviderGatewayError::UnsupportedCapability(reason)
            if reason.contains("identity_carrier_mismatch")),
        "unexpected: {carrier_error}"
    );
    assert_eq!(fixture.streaming_adapter.start_count(), 0);
}

/// RevB2T2 ⚠4:合同装饰用 **resume 稳定摘要**——同一会话换新 run id
/// 指纹不变(不得每次 resume supersede);纪律事实漂移(窗口)必变。
#[test]
fn contract_resume_fingerprint_is_stable_across_run_ids_but_shifts_on_discipline() {
    let fixture = T10StoreFixture::new();
    let worktree = fixture.paths.root().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    fixture.upsert(&record(
        ProviderActionMatrix::from_rows(vec![confirmed_planning_row()]).unwrap(),
        Vec::new(),
    ));
    let gateway = fixture.gateway();
    let prepare = |run_id: &str, window: Option<String>| {
        let identity = TrustedRunIdentity::from_durable_run(
            &durable_run_record_t10(
                run_id,
                if window.is_some() {
                    AdapterRole::Reviewer
                } else {
                    AdapterRole::Executor
                },
            ),
            "ws-t10-0001",
            if window.is_some() {
                TrustedProviderRole::Reviewer
            } else {
                TrustedProviderRole::Coder
            },
        );
        let mut input = streaming_input(worktree.clone());
        if window.is_some() {
            input.role = AdapterRole::Reviewer;
        }
        gateway
            .prepare_trusted_streaming_launch(
                input,
                fixture.planning_request(worktree.clone()),
                audit_context(),
                &identity,
                if window.is_some() {
                    ProviderExecutionProfile::ReviewerVerify
                } else {
                    ProviderExecutionProfile::HostCoding
                },
                None,
                "scope_t10".to_string(),
                window,
            )
            .expect("trusted prepare")
    };
    let run_a = prepare("run_t10_a", None);
    let run_b = prepare("run_t10_b", None);
    let run_a_digest = run_a.into_parts().1.fingerprint().digest.clone();
    assert_eq!(
        run_a_digest,
        run_b.into_parts().1.fingerprint().digest,
        "run id 换新不得移动 resume 指纹(同会话同纪律)"
    );
    let drifted = prepare("run_t10_c", Some("win-t10".to_string()));
    assert_ne!(
        run_a_digest,
        drifted.into_parts().1.fingerprint().digest,
        "纪律事实(Reviewer 窗口)漂移必须移动指纹"
    );
}

/// RevB2T2 ⚠2:合同绑定晚于 LC 投影冻结时,`with_lc_projection` 按合同
/// 重装饰——spawn 前复验不得恒 PolicyDrift。
#[test]
fn contract_bound_policy_survives_lc_projection_rebind_during_revalidation() {
    let fixture = T10StoreFixture::new();
    let worktree = fixture.paths.root().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    fixture.upsert(&record(
        ProviderActionMatrix::from_rows(vec![confirmed_planning_row()]).unwrap(),
        Vec::new(),
    ));
    let identity = TrustedRunIdentity::from_durable_run(
        &durable_run_record_t10("run_t10_p", AdapterRole::Executor),
        "ws-t10-0001",
        TrustedProviderRole::Coder,
    );
    let launch = fixture
        .gateway()
        .prepare_trusted_streaming_launch(
            streaming_input(worktree.clone()),
            fixture.planning_request(worktree),
            audit_context(),
            &identity,
            ProviderExecutionProfile::HostCoding,
            None,
            "scope_t10".to_string(),
            None,
        )
        .expect("trusted prepare");
    let (input, policy) = launch.into_parts();
    let envelope = policy.envelope().clone();
    let projection =
        crate::product::logical_codebase::provider_projection::ProviderPolicyProjection::new(
            ProviderRefType::ClaudeCode,
            envelope.provider_dialect,
            crate::product::logical_codebase::policy::ProviderWireDialect::ClaudeCodeStreamJson,
            "claude 1.4.0".to_string(),
            envelope.action,
            AdapterRole::Executor,
            crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            None,
            "on-request".to_string(),
            "read-only".to_string(),
            envelope.working_directory.clone(),
            envelope.working_directory.clone(),
            envelope.target.clone(),
            envelope.readable_roots.clone(),
            envelope.writable_roots.clone(),
            "sha256:trust-t10".to_string(),
            envelope.config_digest.clone(),
            "sha256:mcp-t10".to_string(),
            "probe://boundary-t10".to_string(),
            policy.action_evidence_digest().to_string(),
            "sha256:session-projection-t10".to_string(),
        );
    let undecorated = SessionResumeFingerprint::from_envelope(
        &envelope,
        &projection,
        policy.action_evidence_digest(),
        &canonical_target_git_identity(&envelope.target.worktree),
    );
    let rebound = policy.with_lc_projection(projection, undecorated);
    fixture
        .gateway()
        .revalidate_before_spawn(&rebound, input.effective_working_directory(), false)
        .expect("revalidation must mirror the contract decoration after lc projection rebind");
}

fn durable_run_record_t10(
    run_id: &str,
    carrier: AdapterRole,
) -> crate::protocol::contracts::ProviderRunRecord {
    crate::protocol::contracts::ProviderRunRecord {
        provider_run_id: run_id.to_string(),
        node_id: "N2".to_string(),
        provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
        runtime_role: crate::protocol::contracts::RuntimeRole::Orchestrator,
        adapter_role: carrier,
        provider_capability_ref: "cap_managed_snapshot".to_string(),
        adapter_compatibility_ref: "compat_1".to_string(),
        context_package_ref: "ctx_1".to_string(),
        adapter_input_ref: "ain_1".to_string(),
        adapter_output_ref: "aout_1".to_string(),
        raw_artifact_refs: Vec::new(),
        exit_code: Some(0),
        error_code: None,
        error_details: None,
        stdout_ref: None,
        stderr_ref: None,
        structured_output_ref: None,
        files_modified: Vec::new(),
        status: crate::protocol::contracts::ProviderRunStatus::Running,
        started_at: "2026-10-17T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
        retry_count: 0,
        approval_policy: crate::protocol::contracts::ApprovalPolicy::OnRequest,
        sandbox_mode: crate::protocol::contracts::SandboxMode::FullAccess,
        constraint_check_ref: None,
        traceability_binding_refs: Vec::new(),
    }
}
