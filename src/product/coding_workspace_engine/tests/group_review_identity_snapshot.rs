use super::*;
use crate::product::coding_models::{
    CompactFindingDigest, ReviewVerdict, UnitReviewConclusionSnapshot,
};
use crate::product::coding_workspace_engine::tests::provider_execution_context::CapturingProjectionProvider;

#[test]
fn unit_review_conclusion_snapshot_write_is_idempotent() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let snapshot = UnitReviewConclusionSnapshot {
        attempt_id: attempt.id.clone(),
        unit_id: "unit_0001".to_string(),
        unit_run_id: "unit_run_0001".to_string(),
        logical_work_item_id: "work_item_0001".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        code_review_report_id: "code_review_0001".to_string(),
        verdict: ReviewVerdict::Approve,
        finding_digest: vec![CompactFindingDigest {
            defect_class: None,
            reason_code: None,
            severity: "info".to_string(),
            message_digest: "message-hash".to_string(),
        }],
        evidence_refs: vec!["test-output/verification.txt".to_string()],
        diff_refs: vec!["HEAD..worktree".to_string()],
        raw_report_hash: "raw-report-hash".to_string(),
    };

    store
        .write_unit_review_conclusion_snapshot(&snapshot)
        .expect("first write");
    store
        .write_unit_review_conclusion_snapshot(&snapshot)
        .expect("idempotent write");

    assert_eq!(
        store
            .get_unit_review_conclusion_snapshot(&attempt.id, &snapshot.unit_run_id)
            .expect("get snapshot"),
        Some(snapshot)
    );
}

#[test]
fn rebuilding_legacy_report_without_unit_run_id_fails_closed() {
    let (_root, store, attempt, unit, unit_run) = group_attempt_with_completed_unit_run();
    let report = code_review_report(&attempt.id, "code_review_0001", None, None);
    store
        .save_code_review_report(&attempt, &report)
        .expect("persist legacy report");

    let error = store
        .rebuild_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
        .expect_err("legacy report must fail closed");

    assert!(matches!(
        error,
        crate::product::coding_models::SnapshotRebuildError::MissingUnitRunId(id)
            if id == report.id
    ));
    assert!(
        store
            .get_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
            .expect("get snapshot")
            .is_none()
    );
    assert_eq!(unit.id, unit_run.unit_id);
}

#[test]
fn rebuilding_snapshot_from_report_and_authoritative_binding_is_deterministic() {
    let (root, store, attempt, unit, unit_run) = group_attempt_with_completed_unit_run();
    let raw_output = "review raw output\n";
    let raw_path = root
        .path()
        .join(".aria")
        .join("projects")
        .join(&attempt.project_id)
        .join("issues")
        .join(&attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join("provider-raw/code-review/code_review_0001.txt");
    std::fs::create_dir_all(raw_path.parent().expect("raw parent")).expect("raw parent");
    std::fs::write(&raw_path, raw_output).expect("raw output");
    let raw_ref = "provider-raw/code-review/code_review_0001.txt".to_string();
    let report = code_review_report(
        &attempt.id,
        "code_review_0001",
        Some(unit_run.id.clone()),
        Some(raw_ref),
    );
    store
        .save_code_review_report(&attempt, &report)
        .expect("persist report");
    let direct = UnitReviewConclusionSnapshot {
        attempt_id: attempt.id.clone(),
        unit_id: unit.id.clone(),
        unit_run_id: unit_run.id.clone(),
        logical_work_item_id: unit.logical_work_item_id.clone(),
        work_item_revision_id: unit.work_item_revision_id.clone(),
        code_review_report_id: report.id.clone(),
        verdict: report.verdict.clone(),
        finding_digest: Vec::new(),
        evidence_refs: report.tested_evidence_refs.clone(),
        diff_refs: report.diff_refs.clone(),
        raw_report_hash: sha256_hex(raw_output),
    };

    let rebuilt = store
        .rebuild_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
        .expect("rebuild snapshot");

    assert_eq!(rebuilt, direct);
    assert_eq!(
        store
            .get_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
            .expect("get rebuilt snapshot"),
        Some(direct)
    );
}

#[tokio::test]
async fn code_review_snapshot_write_failure_rolls_back_report() {
    let (root, store, attempt, _unit, unit_run) = group_attempt_with_completed_unit_run();
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let snapshot_root = root
        .path()
        .join(".aria")
        .join("projects")
        .join(&attempt.project_id)
        .join("issues")
        .join(&attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join("unit-review-conclusion-snapshots");
    std::fs::create_dir_all(&snapshot_root).expect("snapshot root");
    std::fs::write(
        snapshot_root.join(&unit_run.id).with_extension("json"),
        "not json",
    )
    .expect("poison snapshot path");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "approve",
            "summary": "review approved",
            "findings": []
        })
        .to_string(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("snapshot failure must fail review persistence");

    assert!(error.to_string().contains("product_store_json"));
    assert!(
        store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("list reports")
            .is_empty()
    );
}

#[tokio::test]
async fn code_review_persists_report_and_snapshot_on_normal_path() {
    let (_root, store, attempt, _unit, unit_run) = group_attempt_with_completed_unit_run();
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "approve",
            "summary": "review approved",
            "findings": []
        })
        .to_string(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let report = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect("review succeeds");

    assert_eq!(report.unit_run_id.as_deref(), Some(unit_run.id.as_str()));
    assert_eq!(
        store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("list reports"),
        vec![report.clone()]
    );
    let snapshot = store
        .get_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
        .expect("get snapshot")
        .expect("snapshot persisted");
    assert_eq!(snapshot.code_review_report_id, report.id);
}

fn sha256_hex(value: &str) -> String {
    use sha2::{Digest, Sha256};

    hex::encode(Sha256::digest(value.as_bytes()))
}

fn group_attempt_with_completed_unit_run() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
    crate::product::coding_models::CodingExecutionUnit,
    crate::product::coding_models::CodingUnitRun,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let unit = store
        .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("active unit")
        .expect("unit");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("plan lineage");
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &unit.logical_work_item_id,
            &unit.work_item_revision_id,
        )
        .expect("work item revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("projection bundle");
    let unit_run = crate::product::coding_models::CodingUnitRun {
        id: "coding_unit_run_0001".to_string(),
        unit_id: unit.id.clone(),
        execution_no: 1,
        work_item_revision_id: unit.work_item_revision_id.clone(),
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: bundle.canonical_contract_hash,
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: renderer_for(&ProviderName::Fake)
            .renderer_version()
            .to_string(),
        reviewer_provider_renderer_version: renderer_for(&ProviderName::Fake)
            .renderer_version()
            .to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: crate::product::coding_models::CodingUnitRunStatus::Running,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some("start-commit".to_string()),
        completion_commit: Some("completion-commit".to_string()),
        created_at: "2026-08-04T00:00:00Z".to_string(),
        updated_at: "2026-08-04T00:00:00Z".to_string(),
    };
    store
        .create_coding_unit_run(&attempt, &unit_run)
        .expect("completed unit run");
    (root, store, attempt, unit, unit_run)
}

fn code_review_report(
    attempt_id: &str,
    id: &str,
    unit_run_id: Option<String>,
    raw_provider_output_ref: Option<String>,
) -> crate::product::coding_models::CodeReviewReport {
    crate::product::coding_models::CodeReviewReport {
        id: id.to_string(),
        attempt_id: attempt_id.to_string(),
        round: 1,
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        tested_evidence_refs: vec!["test-output/verification.txt".to_string()],
        diff_refs: vec!["HEAD..worktree".to_string()],
        summary: "review approved".to_string(),
        created_at: "2026-08-04T00:00:00Z".to_string(),
        raw_provider_output_ref,
        role_run_id: None,
        run_no: None,
        unit_run_id,
    }
}

// ================= Task 2.7：Group/Internal Reviewer root cwd（REQ-ENV-03/10/11） =================

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::coding_workspace_engine::group_review_orchestrator::{
    GroupReviewExecutor, RealGroupReviewExecutor,
};
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
    GatewayRunAudit, LogicalCodebaseManifest, LogicalCodebaseProviderGateway, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, PolicyTarget, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderDialect, ProviderGatewayError, ProviderRef, ProviderRefType,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    SessionLaunchRequest, SessionPolicyAction,
};
use crate::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};

/// pass-through target resolver：直接返回请求中冻结的 target（与
/// provider_gateway_validated_input.rs 同构，本文件自持）。
struct PassThroughTargetResolver;

impl PolicyTargetResolver for PassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 测试 capability source:按 provider ref 返回对应 dialect 的 capability
/// (Task 2b 分格形状:launch/resume/write_boundary 恒 Confirmed)。
struct StaticCapabilitySource;

impl StaticCapabilitySource {
    fn capability(provider: &ProviderRef, action: SessionPolicyAction) -> ProviderCapability {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
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

impl ProviderCapabilitySource for StaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
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

/// 同步 adapter stub：`validate` 不触达 sync adapter，仅满足 gateway 构造签名。
struct StubSyncAdapter;

impl ProviderAdapter for StubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
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

    let checked_at = Utc::now();
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

/// 组装一个 bootstrap 政策就绪、可注册 fake streaming adapter 的 gateway；
/// manifest/authority 根取 workspace root（.aria 的父目录），成员 checkout
/// 位于聚合根之下的真实拓扑（Task 2.8 cwd authority 契约）。
fn build_gateway_with_registry(
    paths: &ProductAppPaths,
    project_id: &str,
    registry: Arc<ProviderRegistry>,
    audit: Arc<GatewayRunAudit>,
) -> LogicalCodebaseProviderGateway {
    let manifest = LogicalCodebaseManifest::new(
        project_id,
        paths.root().parent().expect("workspace root").to_path_buf(),
        vec![],
    );
    let policies = AggregatePolicyArtifactStore::new(paths.clone());
    policies
        .ensure_bootstrap(&manifest)
        .expect("bootstrap policy");
    LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(StaticCapabilitySource),
        Arc::new(PassThroughTargetResolver),
        registry,
        Arc::new(StubSyncAdapter),
        always_available_gate(),
        audit,
        manifest.provider_context_root.clone(),
    )
}

/// 覆写 attempt 的 `target_snapshot` 为逻辑代码库 target 并落盘。
fn with_target_snapshot(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    let worktree = attempt
        .worktree_path
        .clone()
        .expect("running attempt must have a worktree");
    let mut logical = attempt.clone();
    logical.target_snapshot = Some(AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
        checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
        physical_repository_id: "repository_0001".to_string(),
        canonical_path: worktree,
        git_dir_identity: "git-dir-identity".to_string(),
        revision: None,
        policy_digest: String::new(),
        membership_revision: 1,
        captured_at: "2026-08-09T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    });
    let attempt_path = store
        .paths()
        .issue_lifecycle_root(&logical.project_id, &logical.issue_id)
        .join("coding-attempts")
        .join(format!("{}.json", logical.id));
    crate::product::json_store::write_json(&attempt_path, &logical)
        .expect("write attempt with target snapshot");
    logical
}

/// 播种 logical manifest 与成员记录：target 成员=attempt worktree（nil 身份，
/// 与 `with_target_snapshot` 对齐）；`with_non_target_member` 时追加一个 active
/// 非 target 成员（自带 git repo，供 D4 baseline/漂移观测），返回其主 checkout。
fn seed_reviewer_member_codebase(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    with_non_target_member: bool,
) -> Option<std::path::PathBuf> {
    let logical_store = LogicalCodebaseStore::new(store.paths());
    let repository_id = LogicalRepositoryId(uuid::Uuid::nil());
    let worktree = attempt.worktree_path.clone().expect("worktree");
    let mut member_ids = vec![repository_id];
    let extra_member = with_non_target_member.then(|| {
        let id = LogicalRepositoryId(uuid::Uuid::new_v4());
        member_ids.push(id);
        id
    });
    logical_store
        .save_manifest(
            &attempt.project_id,
            &LogicalCodebaseManifest::new(
                &attempt.project_id,
                store.paths().root().to_path_buf(),
                member_ids,
            ),
        )
        .expect("save manifest");

    fn seed_member_checkout(
        logical_store: &LogicalCodebaseStore,
        project_id: &str,
        member_id: LogicalRepositoryId,
        alias: &str,
        checkout_path: &std::path::Path,
    ) {
        let now = "2026-10-02T00:00:00Z".to_string();
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        logical_store
            .save_member(
                project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_0001".to_string(),
                    alias: alias.to_string(),
                    role: "repository".to_string(),
                    ordinal: 1,
                    source_identity: RepositorySourceIdentity::from_git_parts(
                        checkout_path,
                        checkout_path.join(".git"),
                        None,
                    ),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .expect("save member");
        logical_store
            .save_checkout(
                project_id,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_0001".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: checkout_path.to_path_buf(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: "sha256:git-dir".to_string(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-10-02T00:00:00Z".to_string(),
                    created_at: "2026-10-02T00:00:00Z".to_string(),
                    updated_at: "2026-10-02T00:00:00Z".to_string(),
                },
            )
            .expect("save checkout");
    }

    seed_member_checkout(
        &logical_store,
        &attempt.project_id,
        repository_id,
        "target_member",
        &worktree,
    );
    extra_member.map(|member_id| {
        let checkout_path = store
            .paths()
            .root()
            .parent()
            .expect("workspace root")
            .join("non_target_member");
        std::fs::create_dir_all(&checkout_path).expect("non-target member dir");
        init_test_git_repo(&checkout_path);
        seed_member_checkout(
            &logical_store,
            &attempt.project_id,
            member_id,
            "non_target_member",
            &checkout_path,
        );
        checkout_path
    })
}

/// Task 2.7 探针：记录 spawn 时点 effective cwd、spawn 计数、baseline 目录在
/// spawn 时点已落盘的文件数；可选在会话期间向 `drift_target` 写一次越界文件
/// （模拟 provider 写非 target 成员主 checkout）。
struct ReviewerRootCwdProbeAdapter {
    output: String,
    spawns: Arc<AtomicUsize>,
    cwd_at_spawns: Arc<std::sync::Mutex<Vec<std::path::PathBuf>>>,
    baseline_files_at_spawns: Arc<std::sync::Mutex<Vec<usize>>>,
    baselines_root: std::path::PathBuf,
    drift_target: Option<std::path::PathBuf>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewerRootCwdProbeAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        self.cwd_at_spawns
            .lock()
            .expect("reviewer cwd probe mutex")
            .push(input.effective_working_directory().to_path_buf());
        let baseline_files = std::fs::read_dir(&self.baselines_root)
            .map(|entries| entries.filter_map(|entry| entry.ok()).count())
            .unwrap_or(0);
        self.baseline_files_at_spawns
            .lock()
            .expect("reviewer baseline probe mutex")
            .push(baseline_files);
        if let Some(drift_target) = &self.drift_target {
            std::fs::write(drift_target, "out of worktree write\n")
                .expect("simulated non-target member drift");
        }
        let structured_output_contract = input.structured_output_contract.clone();
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        let output = self.output.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::from_output(
                    output,
                    structured_output_contract.as_ref(),
                    None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
    /// Task 1b 段③:prepared launch 经 `start_validated` 分发——拆出 input
    /// 后与裸 `start` 同路径(probe 副作用/计数保持)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }
}

enum ReviewerRootCwdFace {
    /// group review shard/reduction executor（生产入口 `RealGroupReviewExecutor`）。
    GroupReviewShard,
    /// InternalReviewer（生产入口 `execute_internal_pr_review`）。
    InternalPrReview,
}

struct ReviewerRootLaunchObservation {
    _root: tempfile::TempDir,
    store: CodingAttemptStore,
    attempt: CodingExecutionAttempt,
    canonical_root: std::path::PathBuf,
    worktree: std::path::PathBuf,
    snapshot_repository_id: String,
    snapshot_checkout_id: String,
    envelope_working_directory: std::path::PathBuf,
    envelope_target_worktree: std::path::PathBuf,
    envelope_target_repository_id: String,
    envelope_target_checkout_id: String,
    envelope_writable_roots: Vec<std::path::PathBuf>,
    envelope_readable_roots: Vec<std::path::PathBuf>,
    spawns: Arc<AtomicUsize>,
    cwd_at_spawns: Arc<std::sync::Mutex<Vec<std::path::PathBuf>>>,
    baseline_files_at_spawns: Arc<std::sync::Mutex<Vec<usize>>>,
    non_target_member_checkout: Option<std::path::PathBuf>,
}

/// 以 reviewer 生产入口驱动一次 LC reviewer 启动：input 经生产工厂构造
/// （`group_review_streaming_input` / `internal_pr_review_streaming_input`），
/// policy 经 `resolve_launch_policy_for_role` resolve，spawn 唯一经 gateway。
/// 返回观测面（spawn 计数/cwd/baseline/envelope/drift 目标）。
#[allow(clippy::type_complexity)]
async fn drive_reviewer_root_launch(
    face: ReviewerRootCwdFace,
    with_non_target_member: bool,
    drift: bool,
) -> ReviewerRootLaunchObservation {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().expect("worktree"));
    let mut logical = with_target_snapshot(&store, &attempt);
    let non_target_member_checkout =
        seed_reviewer_member_codebase(&store, &logical, with_non_target_member);
    let probe = Arc::new(ReviewerRootCwdProbeAdapter {
        output: serde_json::json!({
            "verdict": "approve",
            "summary": "root cwd reviewer ok",
            "findings": []
        })
        .to_string(),
        spawns: Arc::new(AtomicUsize::new(0)),
        cwd_at_spawns: Arc::new(std::sync::Mutex::new(Vec::new())),
        baseline_files_at_spawns: Arc::new(std::sync::Mutex::new(Vec::new())),
        baselines_root: CodingAttemptStore::new(store.paths()).attempt_cross_target_baselines_root(
            &logical.project_id,
            &logical.issue_id,
            &logical.id,
        ),
        drift_target: drift.then(|| {
            non_target_member_checkout
                .as_ref()
                .expect("drift requires a non-target member")
                .join("trespass.txt")
        }),
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, probe.clone());
    let gateway = build_gateway_with_registry(
        &store.paths(),
        &logical.project_id,
        Arc::new(registry),
        Arc::new(GatewayRunAudit::new()),
    );
    let canonical_root = gateway.authority_root().to_path_buf();
    let gateway = Arc::new(gateway);
    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_logical_provider_gateway(gateway);

    match face {
        ReviewerRootCwdFace::GroupReviewShard => {
            logical = store
                .update_attempt_stage(
                    &logical.project_id,
                    &logical.issue_id,
                    &logical.id,
                    CodingExecutionStage::InternalPrReview,
                )
                .expect("review stage");
            let role_run = store
                .create_role_run(
                    &logical,
                    CodingExecutionStage::InternalPrReview,
                    CodingProviderRole::InternalReviewer,
                    CodingRoleRunTrigger::Initial,
                    Some("root-cwd-group-node".to_string()),
                )
                .expect("role run");
            let executor = RealGroupReviewExecutor::new(
                &engine,
                logical.clone(),
                probe.as_ref(),
                "root-cwd-group-node".to_string(),
                role_run,
                ProviderName::ClaudeCode,
            );
            executor
                .execute("root cwd group review probe")
                .await
                .expect("group review executor must complete through the gateway");
        }
        ReviewerRootCwdFace::InternalPrReview => {
            store
                .save_review_request(
                    &logical,
                    &ReviewRequest {
                        id: "review_request_0001".to_string(),
                        attempt_id: logical.id.clone(),
                        kind: ReviewRequestKind::GitBranchOnly,
                        remote_kind: RemoteKind::GenericGit,
                        remote: "origin".to_string(),
                        base_branch: logical.base_branch.clone(),
                        branch_name: logical.branch_name.clone(),
                        commit_sha: git_stdout(
                            logical.worktree_path.as_ref().expect("worktree"),
                            &["rev-parse", "HEAD"],
                        ),
                        push_status: PushStatus::Pushed,
                        external_url: None,
                        manual_instructions: Vec::new(),
                        created_at: "2026-10-02T00:00:00Z".to_string(),
                        updated_at: "2026-10-02T00:00:00Z".to_string(),
                        push_error: None,
                        owner_kind: ReviewRequestOwnerKind::Attempt,
                        pointer_publication_id: None,
                        revoked: false,
                    },
                )
                .expect("review request");
            let review = engine
                .execute_internal_pr_review(&logical, probe.as_ref())
                .await
                .expect("internal pr review must complete through the gateway");
            assert_eq!(review.verdict, ReviewVerdict::Approve);
        }
    }

    let worktree = logical.worktree_path.clone().expect("worktree");
    let (snapshot_repository_id, snapshot_checkout_id) = match logical.target_snapshot.as_ref() {
        Some(snapshot) => (
            snapshot.logical_repository_id.0.to_string(),
            snapshot.checkout_id.0.to_string(),
        ),
        None => panic!("logical attempt must carry a target snapshot"),
    };
    let policy = engine
        .resolve_launch_policy_for_role(&logical, CodingProviderRole::InternalReviewer, &worktree)
        .expect("policy resolve returns Ok")
        .expect("logical attempt + gateway must produce reviewer policy");
    let envelope = policy.envelope();
    ReviewerRootLaunchObservation {
        _root: root,
        store,
        attempt: logical,
        canonical_root,
        worktree,
        snapshot_repository_id,
        snapshot_checkout_id,
        envelope_working_directory: envelope.working_directory.clone(),
        envelope_target_worktree: envelope.target.worktree.clone(),
        envelope_target_repository_id: envelope.target.logical_repository_id.clone(),
        envelope_target_checkout_id: envelope.target.checkout_id.clone(),
        envelope_writable_roots: envelope.writable_roots.clone(),
        envelope_readable_roots: envelope.readable_roots.clone(),
        spawns: probe.spawns.clone(),
        cwd_at_spawns: probe.cwd_at_spawns.clone(),
        baseline_files_at_spawns: probe.baseline_files_at_spawns.clone(),
        non_target_member_checkout,
    }
}

/// Task 2.7（REQ-ENV-10）：group review 与 InternalReviewer 的 spawn cwd 重绑
/// canonical LC root——与 attempt target worktree 分离；恰一次 spawn；target/git
/// identity 仍由 resolver 钉在 attempt snapshot（cwd 分离不放松 target 维度）。
#[tokio::test]
async fn group_review_and_internal_reviewer_separate_root_cwd_from_attempt_target() {
    let group =
        drive_reviewer_root_launch(ReviewerRootCwdFace::GroupReviewShard, false, false).await;
    assert_eq!(
        group.spawns.load(Ordering::SeqCst),
        1,
        "group review shard executor 恰一次 spawn"
    );
    assert_ne!(
        group.canonical_root, group.worktree,
        "LC canonical root 与 attempt target worktree 必须可分离"
    );
    assert_eq!(
        *group.cwd_at_spawns.lock().expect("cwd probe mutex"),
        vec![group.canonical_root.clone()],
        "group review spawn cwd == canonical LC root（而非 target worktree）"
    );
    assert_eq!(
        group.envelope_working_directory, group.canonical_root,
        "resolver envelope cwd 已迁 canonical root"
    );
    assert_eq!(
        group.envelope_target_worktree, group.worktree,
        "target 维度仍钉 attempt worktree"
    );
    assert_eq!(
        group.envelope_target_repository_id, group.snapshot_repository_id,
        "target 仓库身份仍由 resolver 从 snapshot 校验"
    );
    assert_eq!(
        group.envelope_target_checkout_id, group.snapshot_checkout_id,
        "target checkout 身份仍由 resolver 从 snapshot 校验"
    );

    let internal =
        drive_reviewer_root_launch(ReviewerRootCwdFace::InternalPrReview, false, false).await;
    assert_eq!(
        internal.spawns.load(Ordering::SeqCst),
        1,
        "internal reviewer 恰一次 spawn"
    );
    assert_ne!(internal.canonical_root, internal.worktree);
    assert_eq!(
        *internal.cwd_at_spawns.lock().expect("cwd probe mutex"),
        vec![internal.canonical_root.clone()],
        "internal reviewer spawn cwd == canonical LC root（而非 target worktree）"
    );
    assert_eq!(internal.envelope_working_directory, internal.canonical_root);
    assert_eq!(internal.envelope_target_worktree, internal.worktree);
}

/// Task 2.7（REQ-ENV-03/ENV-11 + D4）：reviewer cwd 迁 root 后 writable roots
/// 恒空（不随 cwd 扩大）、D4 baseline 仍在 spawn 前落盘，非 target 成员漂移
/// 仍阻断交付（Task 15 统一门不放松）。
#[tokio::test]
async fn logical_reviewer_keeps_empty_writable_roots_and_d4_baseline() {
    let launch =
        drive_reviewer_root_launch(ReviewerRootCwdFace::InternalPrReview, true, true).await;
    assert_eq!(
        launch.spawns.load(Ordering::SeqCst),
        1,
        "internal reviewer 恰一次 spawn"
    );
    assert_eq!(launch.envelope_working_directory, launch.canonical_root);
    assert!(
        launch.envelope_writable_roots.is_empty(),
        "reviewer writable roots 必须恒空（cwd 迁 root 不扩大写边界）"
    );
    assert_eq!(
        launch.envelope_readable_roots,
        vec![launch.canonical_root.clone()],
        "readable roots 随 cwd root 化"
    );
    assert_eq!(
        *launch
            .baseline_files_at_spawns
            .lock()
            .expect("baseline probe mutex"),
        vec![1],
        "cross-target baseline 必须先于 reviewer spawn 落盘"
    );
    let non_target = launch
        .non_target_member_checkout
        .as_ref()
        .expect("non-target member fixture");
    assert!(
        non_target.join("trespass.txt").exists(),
        "probe 漂移写必须真实发生"
    );

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(launch.store.clone(), GitWorkspaceService::new(), tx);
    match engine
        .execute_review_request(&launch.attempt, "origin", "feat: root cwd drift probe")
        .await
    {
        Err(CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(code)) => {
            assert_eq!(code, "cross_target_violation_detected");
        }
        other => panic!("expected CrossTargetDeliveryBlocked, got {other:?}"),
    }
}

/// Task 3.2（D4 全角色）：group review shard/reduction executor（生产入口
/// `RealGroupReviewExecutor::execute`）的 D4 基线锁——与 InternalReviewer/
/// Coder 同享 spawn 前基线语义：group review role run 启动前基线已落盘、
/// 非 target 成员主 checkout 的越界写（review run 内）在交付统一门被检测
/// 并阻断。补齐 2.7 group face 仅锁 cwd 的缺口，使 D4 覆盖
/// coder/reviewer/group/internal 全角色。
#[tokio::test]
async fn group_review_shard_keeps_d4_baseline_and_blocks_cross_target_drift() {
    let launch =
        drive_reviewer_root_launch(ReviewerRootCwdFace::GroupReviewShard, true, true).await;
    assert_eq!(
        launch.spawns.load(Ordering::SeqCst),
        1,
        "group review shard executor 恰一次 spawn"
    );
    assert_eq!(
        *launch
            .baseline_files_at_spawns
            .lock()
            .expect("baseline probe mutex"),
        vec![1],
        "cross-target baseline 必须先于 group review spawn 落盘"
    );
    assert!(
        launch.envelope_writable_roots.is_empty(),
        "group review writable roots 必须恒空（review 角色零写根）"
    );
    let non_target = launch
        .non_target_member_checkout
        .as_ref()
        .expect("non-target member fixture");
    assert!(
        non_target.join("trespass.txt").exists(),
        "probe 漂移写必须真实发生在 group review run 内"
    );

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(launch.store.clone(), GitWorkspaceService::new(), tx);
    match engine
        .execute_review_request(&launch.attempt, "origin", "feat: group d4 drift probe")
        .await
    {
        Err(CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(code)) => {
            assert_eq!(code, "cross_target_violation_detected");
        }
        other => panic!("expected CrossTargetDeliveryBlocked, got {other:?}"),
    }
}
