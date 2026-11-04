/// B2b1(Task 5.2,anchor_t14,设计 §4.2.4/§5.4):old map missing → waiting——
/// fullmap 纪元激活(同 provider/role 存在 map 记录)后,conversation-only
/// native(不在任何 Active map 绑定)不得 resume 也不得出生 fresh,typed
/// FrozenBindingUnavailable 上浮、零 provider start;绑定一致 native 正常
/// 续接(纪元不误伤);无 gateway 的 direct 形态(纪元之前)维持既有恢复
/// 行为(plan None 分支原样)。
#[test]
fn anchor_t14_plan_resume_without_trusted_map_waits_not_fresh() {
    use crate::product::models::{
        LogicalLaunchBasis, LogicalLaunchFingerprintRecord, LogicalLaunchFingerprintState,
        ProviderConversationRef, ProviderConversationRole,
    };

    let (_tmp, _checkpoint, lifecycle, _plan_id, engine_plain) =
        make_work_item_plan_engine_with_draft_candidate("sess_t14_epoch");
    let worktree = _tmp.path().join("repository").to_string_lossy().to_string();

    // direct 对照(纪元之前):无 gateway,conversation-only native 原样续接。
    let plain_engine = engine_plain;
    let session_id = plain_engine.session.session_id.clone();
    seed_t14_epoch_record(
        &lifecycle,
        &session_id,
        "native-conv-only",
        Some("native-map-bound"),
    );
    let resumed = plain_engine
        .build_work_item_plan_streaming_input(
            crate::protocol::contracts::ProviderType::ClaudeCode,
            "anchor t14 prompt".to_string(),
            worktree.clone(),
            ProviderName::ClaudeCode,
            crate::product::workspace_engine::PlanAuthorOutputContract::Structured,
        )
        .expect("纪元之前(direct)conversation-only resume 维持既有行为");
    assert_eq!(
        resumed.resume_provider_session_id.as_deref(),
        Some("native-conv-only")
    );

    // 纪元激活:gateway 在场 + 同 (role,provider) 存在 map 记录,conversation
    // native 未命中 Active 绑定 → typed 等待,零 fresh 出生。
    let engine = plain_engine.with_logical_provider_gateway(t14_epoch_gateway(
        &lifecycle,
        _tmp.path().join("t14-authority-root"),
    ));
    let error = engine
        .build_work_item_plan_streaming_input(
            crate::protocol::contracts::ProviderType::ClaudeCode,
            "anchor t14 prompt".to_string(),
            worktree.clone(),
            ProviderName::ClaudeCode,
            crate::product::workspace_engine::PlanAuthorOutputContract::Structured,
        )
        .expect_err("unbound conversation native must wait, not resume or rebirth");
    assert!(
        error.contains("native_resume_candidate_frozen_binding_unavailable"),
        "typed FrozenBindingUnavailable 必须上浮,got: {error}"
    );

    // 绑定一致对照:conversation native == Active map 绑定 → 正常续接。
    seed_t14_epoch_record(
        &lifecycle,
        &session_id,
        "native-map-bound",
        Some("native-map-bound"),
    );
    let resumed = engine
        .build_work_item_plan_streaming_input(
            crate::protocol::contracts::ProviderType::ClaudeCode,
            "anchor t14 prompt".to_string(),
            worktree,
            ProviderName::ClaudeCode,
            crate::product::workspace_engine::PlanAuthorOutputContract::Structured,
        )
        .expect("map-bound native resumes");
    assert_eq!(
        resumed.resume_provider_session_id.as_deref(),
        Some("native-map-bound")
    );

    fn seed_t14_epoch_record(
        lifecycle: &LifecycleStore,
        session_id: &str,
        conversation_native: &str,
        map_native: Option<&str>,
    ) {
        let mut record = lifecycle
            .get_workspace_session(session_id)
            .expect("session record");
        record.provider_conversations = vec![ProviderConversationRef {
            role: ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            provider_session_id: conversation_native.to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            last_node_id: Some("timeline_node_prev".to_string()),
        }];
        record.logical_launch_fingerprints = vec![LogicalLaunchFingerprintRecord {
            schema_version: 1,
            basis: LogicalLaunchBasis::FullLcProjectionV2,
            role: ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            role_run_seq: 7,
            node_id: "timeline_node_007".to_string(),
            native_session_id: map_native.map(str::to_string),
            fingerprint_digest: "sha256:t14-epoch".to_string(),
            planning_access_fingerprint: "sha256:t14-epoch".to_string(),
            state: LogicalLaunchFingerprintState::Active,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }];
        crate::product::json_store::write_json(
            &lifecycle
                .workspace_sessions_root("project_0001", "issue_0001")
                .join(format!("{session_id}.json")),
            &record,
        )
        .expect("seed epoch record");
    }

    fn t14_epoch_gateway(
        lifecycle: &LifecycleStore,
        worktree: std::path::PathBuf,
    ) -> std::sync::Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway> {
        std::fs::create_dir_all(&worktree).expect("authority root dir");
        let manifest = crate::product::logical_codebase::LogicalCodebaseManifest::new(
            "project_0001",
            worktree.clone(),
            Vec::new(),
        );
        let policy_store = crate::product::logical_codebase::AggregatePolicyArtifactStore::new(
            lifecycle.app_paths(),
        );
        policy_store
            .ensure_bootstrap(&manifest)
            .expect("bootstrap aggregate policy");
        let mut registry = crate::cross_cutting::provider_registry::ProviderRegistry::new();
        registry.register(
            ProviderName::ClaudeCode,
            std::sync::Arc::new(ReviewVerdictStreamingProvider {
                output: "t14",
                provider_type: std::sync::Arc::new(std::sync::Mutex::new(None)),
                prompt: std::sync::Arc::new(std::sync::Mutex::new(None)),
            }),
        );
        std::sync::Arc::new(
            crate::product::logical_codebase::LogicalCodebaseProviderGateway::new(
                policy_store,
                std::sync::Arc::new(ReviewStaticCapabilitySource::default()),
                std::sync::Arc::new(ReviewPassThroughTargetResolver),
                std::sync::Arc::new(registry),
                std::sync::Arc::new(ReviewStubSyncAdapter),
                review_always_available_gate(),
                worktree,
            ),
        )
    }
}
