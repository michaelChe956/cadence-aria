use super::*;

#[test]
fn only_pi_is_excluded_from_review_repair() {
    assert!(provider_allows_review_repair(&ProviderName::KimiCode));
    assert!(!provider_allows_review_repair(&ProviderName::Pi));
    assert!(provider_allows_review_repair(&ProviderName::Codex));
}

/// 诊断直通（claude×轻 握手谜团第 2 轮）：gateway 错误映射不得丢弃 adapter
/// error 的 stderr——Adapter 变体的 stderr 尾部（有界）需并入新错误的
/// details（驱动 result/WS error 消息的上游），stderr 字段同源保留。
#[test]
fn gateway_adapter_error_mapping_carries_bounded_stderr_tail() {
    let stderr_head = "NOISE_HEAD_MARKER".to_string() + &"b".repeat(700);
    let stderr = format!("{stderr_head}\nSENTINEL_GW_STDERR_TAIL");
    let inner = ProviderAdapterError::parse_error(
        "claude policy session: handshake failed: claude policy handshake cancelled",
        String::new(),
        stderr,
    );
    let mapped = map_gateway_error_to_adapter(&ProviderGatewayError::Adapter(inner));
    assert!(
        mapped.details.contains("provider_gateway_adapter"),
        "gateway 映射应保留既有 Display 前缀文案，got: {}",
        mapped.details
    );
    assert!(
        mapped.details.contains("SENTINEL_GW_STDERR_TAIL"),
        "gateway 映射应携带 adapter stderr 尾部，got: {}",
        mapped.details
    );
    assert!(
        !mapped.details.contains("NOISE_HEAD_MARKER"),
        "stderr 尾部应有界（丢头保尾），got: {}",
        mapped.details
    );
    assert!(
        mapped.stderr.contains("SENTINEL_GW_STDERR_TAIL"),
        "stderr 字段应同源保留，got: {}",
        mapped.stderr
    );
}

// ---------------------------------------------------------------------------
// T6 fix-loop r2：reviewer 首发 gateway 启动口（U3 位点）trust 分类。
// ---------------------------------------------------------------------------

/// U3 夹具 capability double：任意 provider×action 恒 Confirmed（真实
/// dialect 映射，Kimi/Codex/Claude/Pi 全支持）。
struct TrustScenarioCapabilitySource;

impl crate::product::logical_codebase::ProviderCapabilitySource for TrustScenarioCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError> {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
        let (adapter_dialect, wire_dialect) = match provider.provider_type {
            crate::product::logical_codebase::ProviderRefType::ClaudeCode => (
                crate::product::logical_codebase::policy::ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            crate::product::logical_codebase::ProviderRefType::Codex => (
                crate::product::logical_codebase::policy::ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            crate::product::logical_codebase::ProviderRefType::Pi => (
                crate::product::logical_codebase::policy::ProviderDialect::PiRpcV1,
                ProviderWireDialect::PiRpc,
            ),
            crate::product::logical_codebase::ProviderRefType::KimiCode => (
                crate::product::logical_codebase::policy::ProviderDialect::KimiAcpV1,
                ProviderWireDialect::KimiAcp,
            ),
        };
        Ok(crate::product::logical_codebase::ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.4.0".to_string(),
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
        })
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError> {
        self.require_supported(provider, action)
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError> {
        self.require_supported(provider, action)
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError> {
        self.require_supported(
            provider,
            crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
        )
    }
}

/// U3 夹具 target resolver：透传 request target。
struct TrustScenarioTargetResolver;

impl crate::product::logical_codebase::PolicyTargetResolver for TrustScenarioTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// U3 夹具 sync adapter stub。
struct TrustScenarioSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for TrustScenarioSyncAdapter {
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

/// U3 夹具：bootstrap 政策就绪 + Kimi/Codex 隔离 home 只读 trust source
///（条目缺失）的 gateway；provider adapter 不注册（trust 拒绝先于
/// registry lookup，零启动由断言保证）。
fn reviewer_trust_gateway(
    paths: &crate::product::app_paths::ProductAppPaths,
    trust_home: &std::path::Path,
) -> std::sync::Arc<LogicalCodebaseProviderGateway> {
    let manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
        "project_0001",
        paths.root().to_path_buf(),
        vec![],
    );
    crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");
    let lc_id = "logical_codebase_0001".to_string();
    let gateway = LogicalCodebaseProviderGateway::with_audit(
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::new(paths.clone()),
        std::sync::Arc::new(TrustScenarioCapabilitySource),
        std::sync::Arc::new(TrustScenarioTargetResolver),
        std::sync::Arc::new(crate::cross_cutting::provider_registry::ProviderRegistry::new()),
        std::sync::Arc::new(TrustScenarioSyncAdapter),
        std::sync::Arc::new(crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(
            std::sync::Arc::new(TrustScenarioAlwaysHealthy),
        )),
        std::sync::Arc::new(
            crate::product::logical_codebase::provider_gateway::GatewayRunAudit::new(),
        ),
        std::fs::canonicalize(&manifest.provider_context_root).expect("authority root"),
    )
    .with_readonly_lc_facts(
        crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(paths.clone(), &lc_id),
        std::sync::Arc::new(
            crate::product::logical_codebase::provider_trust::ReadonlyProviderTrustSource::new(
                paths.clone(),
                Some(lc_id.clone()),
                vec![
                    std::sync::Arc::new(
                        crate::product::logical_codebase::provider_trust_adapters::CodexTrustAdapter::for_home(trust_home),
                    ),
                    std::sync::Arc::new(
                        crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::for_home(trust_home),
                    ),
                ],
            ),
        ),
        Some(lc_id),
    );
    std::sync::Arc::new(gateway)
}

/// U3 夹具 availability gate：恒健康。
struct TrustScenarioAlwaysHealthy;

impl crate::cross_cutting::provider_availability_gate::ProviderHealthSource
    for TrustScenarioAlwaysHealthy
{
    fn snapshot(
        &self,
    ) -> std::sync::Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot> {
        let checked_at = chrono::Utc::now();
        std::sync::Arc::new(
            crate::cross_cutting::provider_health::ProviderHealthSnapshot {
                schema_version: 1,
                generation: 1,
                checked_at,
                providers: [
                    crate::product::models::ProviderName::ClaudeCode,
                    crate::product::models::ProviderName::Codex,
                    crate::product::models::ProviderName::KimiCode,
                ]
                .into_iter()
                .map(
                    |provider| crate::cross_cutting::provider_health::ProviderHealthEntry {
                        provider,
                        command: "stub".to_string(),
                        available: true,
                        version: Some("1.0".to_string()),
                        reason_code: None,
                        reason: None,
                        checked_at,
                    },
                )
                .collect(),
            },
        )
    }

    fn degraded(&self) -> bool {
        false
    }
}

fn reviewer_trust_streaming_input(
    working_dir: std::path::PathBuf,
) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
    crate::cross_cutting::streaming_provider::StreamingProviderInput {
        provider_type: crate::protocol::contracts::ProviderType::KimiCode,
        role: crate::protocol::contracts::AdapterRole::Reviewer,
        prompt: "review the change".to_string(),
        working_dir,
        working_directory: None,
        workspace_session_id: Some("session_0001".to_string()),
        resume_provider_session_id: None,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        tool_policy: None,
        audit_sink: None,
        structured_output_contract: None,
        env_vars: Default::default(),
        timeout_secs: 30,
        baseline_tree: None,
    }
}

/// T6 fix-loop r2：reviewer 首发 gateway 启动口（`start_review_session_via_
/// gateway`，U3 位点）Kimi reviewer 条目缺失 → spawn trust 拒绝以
/// `provider_trust_missing` 分类＋同源指导＋生成场景 retry_action 拒收
///（`ReviewProviderRunFailure::Start` 形态，经 finish_review_provider_run_
/// failure → EngineEvent::Error 上浮）；provider 零启动。负例：Claude
/// reviewer 无 workspace trust 面——validate/复验后错误不携带 trust 分类。
#[tokio::test]
async fn reviewer_gateway_spawn_trust_missing_classifies_with_manual_guidance() {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let gateway = reviewer_trust_gateway(&paths, trust_home.path());
    let target_dir = tempfile::tempdir().expect("review target anchor");

    let error = match start_review_session_via_gateway(
        &gateway,
        &crate::product::models::ProviderName::KimiCode,
        reviewer_trust_streaming_input(target_dir.path().to_path_buf()),
        "project_0001".to_string(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    {
        Ok(_) => panic!("reviewer spawn must reject on missing trust"),
        Err(error) => error,
    };

    assert!(
        error.details.contains("provider_trust_missing"),
        "reviewer gateway 启动臂 trust 拒绝必须提升 provider_trust_missing 分类，got: {}",
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
        "reviewer 臂必须携带生成场景 retry_action，got: {}",
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
        "reviewer 臂指导必须同源（真实工件路径），got: {}",
        error.details
    );
}

/// U3 负例：Claude reviewer 无用户级 workspace trust 面——gateway 拒绝
///（registry lookup 等）不得携带 trust 分类/指导。
#[tokio::test]
async fn reviewer_gateway_rejection_without_trust_face_stays_bare() {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let gateway = reviewer_trust_gateway(&paths, trust_home.path());
    let target_dir = tempfile::tempdir().expect("review target anchor");

    let error = match start_review_session_via_gateway(
        &gateway,
        &crate::product::models::ProviderName::ClaudeCode,
        reviewer_trust_streaming_input(target_dir.path().to_path_buf()),
        "project_0001".to_string(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    {
        Ok(_) => panic!("claude reviewer launch must fail without registered adapter"),
        Err(error) => error,
    };

    assert!(
        !error.details.contains("provider_trust_missing")
            && !error.details.contains("人工登记指导"),
        "无 workspace trust 面的 provider 拒绝不得误附 trust 指导，got: {}",
        error.details
    );
}
