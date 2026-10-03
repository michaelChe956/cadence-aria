// 从 provider_gateway_tests.rs 拆出的 audit/managed-settings 测试段
// （large_file_guard 1200 行红线，T11 fix round 2）。共享 mod task13_gateway_hardening 作用域。
/// 配置来源审计:`ConfigSourceAudit` 记录最终 argv 与 config digest,且
/// argv 非空、config digest 与 envelope 冻结值一致。仅 Aria-owned
/// (user/project/local/env/mcp)来源被标注;非 Aria 来源(如 managed settings)
/// 被标注为 `managed_settings_active=true` 并携带警告,绝不假装已覆盖。
#[test]
fn config_source_audit_records_argv_config_digest_and_provenance() {
    let audit = ConfigSourceAudit::from_launch(
        &[
            "claude".to_string(),
            "--permission-prompt-tool=stdio".to_string(),
        ],
        "sha256:managed-config-artifact",
        ConfigSourceProvenance {
            user_settings: true,
            project_settings: true,
            local_settings: false,
            env_overrides: true,
            managed_settings_active: false,
            managed_settings_warning: None,
            mcp_sources: vec![ConfigSourceKind::AriaOwnedBundle],
        },
    );

    assert_eq!(audit.argv, vec!["claude", "--permission-prompt-tool=stdio"]);
    assert!(audit.config_digest.starts_with("sha256:"));
    assert!(!audit.provenance.managed_settings_active);
    assert!(audit.provenance.is_aria_owned_only());
}

/// 配置来源审计:解析 provider `/status` 的 `Setting sources` 时发现 managed
/// settings(非 Aria-owned),标注 `managed_settings_active=true` + 警告,且
/// `is_aria_owned_only()` 返回 false(绝不假装已覆盖)。该已知 gap 仍可被
/// policy 配置为拒绝启动(`ManagedSettingsActive` 错误)。
#[test]
fn config_source_audit_flags_managed_settings_without_pretending_override() {
    let provenance =
        ConfigSourceProvenance::detect_from_setting_sources(&["User", "Project", "Managed"]);
    assert!(provenance.managed_settings_active);
    assert!(
        provenance
            .managed_settings_warning
            .as_ref()
            .is_some_and(|warning| warning.contains("managed settings")
                && !warning.contains("overridden")
                && !warning.contains("覆盖")),
        "warning must not claim override: {:?}",
        provenance.managed_settings_warning
    );
    assert!(!provenance.is_aria_owned_only());
}

/// 配置来源审计(Task 11 语义修正):managed settings 不再 fail-closed,而是
/// 在 `GatewayRunAudit` 追加标注(携带 config digest)后放行。`enforce_config_source_policy`
/// 返回 `Ok`,且 `managed_settings_annotations` 记录该已知 gap——绝不假装已覆盖,
/// 但不阻断启动。
#[test]
fn gateway_annotates_managed_settings_active_without_blocking() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let request = SessionLaunchRequest::planning(
        fixture.manifest().project_id,
        ProviderRef::claude_code("cap_claude_code_1_4_0"),
        PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
        vec![fixture.paths.root().to_path_buf()],
        "sha256:managed-config-artifact",
    );
    let validated = fixture.gateway().validate(request).unwrap();

    let provenance = ConfigSourceProvenance::detect_from_setting_sources(&["User", "Managed"]);
    let audit = ConfigSourceAudit::from_launch(
        &["claude".to_string()],
        "sha256:managed-config-artifact",
        provenance,
    );
    let gateway = fixture.gateway();
    gateway
        .enforce_config_source_policy(&validated, &audit)
        .expect("managed settings must be annotated, not blocked");

    let annotations = fixture.gateway_audit().managed_settings_annotations();
    assert!(
        !annotations.is_empty(),
        "managed settings must be annotated in the audit"
    );
    assert!(
        annotations
            .iter()
            .any(|annotation| annotation.contains("managed settings")),
        "annotation must mention managed settings: {annotations:?}"
    );
}

/// Task 11 审计聚合:`start_streaming` 成功启动后,audit entry 冻结该次启动的
/// config digest(Some)与最终 argv(无真实 argv 源,应为空 Vec)。
#[tokio::test]
async fn start_streaming_audit_entry_carries_config_digest_and_argv() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let launch = fixture.validated_planning_streaming_input(worktree.clone());

    fixture
        .gateway()
        .start_streaming(launch, CancellationToken::new())
        .await
        .expect("streaming launch");

    let audit = fixture.gateway_audit();
    let entries = audit.entries.lock().unwrap();
    let entry = entries
        .iter()
        .find(|entry| entry.stack == GatewayRunStack::Stream)
        .expect("stream entry");
    assert!(
        entry.config_digest.is_some(),
        "config digest must be recorded: {entry:?}"
    );
    assert!(
        entry.argv.is_empty(),
        "no real argv source in launch inputs, expected empty argv: {:?}",
        entry.argv
    );
}

/// Task 2d(mismatch):evidence/projection/record 三方不一致(evidence 的
/// projection_digest 漂移)时导入 fail-closed——返回稳定错误、durable 行
/// 字节保持旧值(launch/write_boundary 全 Unknown)、provider spawn
/// count=0;导入通道不重新 probe、不接受 record 自报。
#[test]
fn lcg_t02_probe_evidence_projection_record_mismatch_stays_unknown() {
    use crate::cross_cutting::provider_boundary::{ProviderBoundaryEvidence, ProviderBoundaryMode};
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord, ProviderCapabilityStore,
        RootRecipeEvidence,
    };
    use crate::product::logical_codebase::{
        ProviderCapabilityProbeService, ProviderPolicyProjection,
    };

    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();

    // durable 旧字节:Kimi 行未探测(全 Unknown,旧版本 1.40.0)。
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = ProductAppPaths::new(root.path());
    let store = ProviderCapabilityStore::new(paths.clone());
    let project_id = "project_0001";
    let action = SessionPolicyAction::CodingTargetWrite;
    let stale = ProviderCapabilityRecord {
        provider_type: ProviderRefType::KimiCode,
        schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
        version: "1.40.0".to_string(),
        adapter_dialect: ProviderDialect::KimiAcpV1,
        wire_dialect: ProviderWireDialect::KimiAcp,
        capability_snapshot_ref: "cap_managed_snapshot".to_string(),
        evidence: CapabilityEvidence::Declared,
        resume_evidence: ResumeEvidenceState::Unsupported,
        supported_actions: Vec::new(),
        action_matrix: ProviderActionMatrix::unknown_all(),
        trust: ProviderCapabilityEvidence::Unknown,
        probed_at: None,
        probe_artifact_ref: None,
        root_recipe_evidence: RootRecipeEvidence::None,
    };
    store
        .upsert(project_id, &stale)
        .expect("seed durable 旧字节");
    let capabilities_path =
        crate::product::logical_codebase::lc_scope_root(&paths, project_id, &None)
            .expect("lc scope root")
            .join("capabilities.json");
    let bytes_before = std::fs::read(&capabilities_path).expect("read durable bytes");

    // 三方不一致:evidence 的 projection_digest 漂移(record/projection 一致)。
    let digest_ok = format!("sha256:{}", "7".repeat(64));
    let digest_drift = format!("sha256:{}", "9".repeat(64));
    let mut record = stale.clone();
    record.version = "1.42.0".to_string();
    record.probed_at = Some("2026-10-03T08:00:00Z".to_string());
    record.probe_artifact_ref = Some("probe://boundary/kimi-code/0001".to_string());
    record.action_matrix = ProviderActionMatrix::from_rows(vec![ProviderActionCapability {
        action,
        launch: ProviderCapabilityEvidence::Confirmed,
        resume: ProviderCapabilityEvidence::Unknown,
        write_boundary: ProviderCapabilityEvidence::Confirmed,
        projection_digest: digest_ok.clone(),
        evidence_ref: "probe://boundary/kimi-code/0001".to_string(),
    }])
    .expect("唯一 action 行");
    let evidence = ProviderBoundaryEvidence::new(
        ProviderName::KimiCode,
        "1.42.0".to_string(),
        ProviderBoundaryMode::TargetWriteOnly,
        digest_drift,
        "probe://boundary/kimi-code/0001".to_string(),
        "2026-10-03T08:00:00Z".to_string(),
    );
    let projection = ProviderPolicyProjection::new(
        ProviderRefType::KimiCode,
        ProviderDialect::KimiAcpV1,
        ProviderWireDialect::KimiAcp,
        "1.42.0".to_string(),
        action,
        crate::protocol::contracts::AdapterRole::Executor,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        None,
        "never".to_string(),
        "client-service".to_string(),
        std::path::PathBuf::from("/lc-root"),
        std::path::PathBuf::from("/work/api/.worktrees/issue_1"),
        PolicyTarget::checkout("logical_repo", "checkout_1", "/work/api/.worktrees/issue_1"),
        vec![std::path::PathBuf::from("/aggregate")],
        vec![std::path::PathBuf::from("/work/api/.worktrees/issue_1")],
        "sha256:trust".to_string(),
        "sha256:config".to_string(),
        "sha256:mcp".to_string(),
        "probe://boundary/plan/1".to_string(),
        "sha256:capability-profile".to_string(),
        digest_ok,
    );

    let service = ProviderCapabilityProbeService::with_durable_writer(store.clone());
    let error = service
        .record_verified_probe(project_id, &record, &evidence, &projection)
        .expect_err("三方不一致必须 fail-closed 拒绝导入");
    assert!(
        error.to_string().contains("provider_probe_import_rejected"),
        "稳定错误码缺失: {error}"
    );

    // durable 字节保持旧值:文件字节逐位不变,行仍全 Unknown。
    let bytes_after = std::fs::read(&capabilities_path).expect("read durable bytes");
    assert_eq!(
        bytes_before, bytes_after,
        "mismatch 导入不得改动 durable 字节"
    );
    let loaded = store
        .get(project_id, ProviderRefType::KimiCode)
        .expect("durable 读取")
        .expect("旧 durable 行保持");
    assert_eq!(loaded, stale, "durable 行保持旧值");
    assert_eq!(
        loaded.action_matrix.row(&action).launch,
        ProviderCapabilityEvidence::Unknown
    );
    assert_eq!(
        loaded.action_matrix.row(&action).write_boundary,
        ProviderCapabilityEvidence::Unknown
    );

    // 导入通道不触达 provider 启动:零 spawn。
    assert_eq!(fixture.registry_start_count(), 0);
}
