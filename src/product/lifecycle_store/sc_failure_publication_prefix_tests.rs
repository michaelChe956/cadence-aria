use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceEvent, sc_failure_chain_id,
};
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use crate::product::models::{
    ArtifactRef, NodeDetail, ProviderName, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ScArtifactPublishedReceipt, ScEvidenceRef,
    ScFailureChainIdentity, ScFailureRepairPackage, ScFailureScope, ScFailureSourceEvidence,
    ScPackageCompleteness, ScRoundIdentity, TimelineNodeStatus, TimelineNodeType,
};
use tempfile::TempDir;

pub(super) const PROJECT: &str = "project_p14_publication";
pub(super) const ISSUE: &str = "issue_p14_publication";
pub(super) const PLAN: &str = "plan_p14_publication";
pub(super) const NODE: &str = "node_p14_publication";

pub(super) fn scope(session_id: &str) -> ScFailureScope {
    ScFailureScope {
        session_id: session_id.to_string(),
        project_id: PROJECT.to_string(),
        issue_id: ISSUE.to_string(),
        plan_id: PLAN.to_string(),
    }
}

pub(super) fn identity(scope: &ScFailureScope) -> ScFailureChainIdentity {
    let business_key = "bk_p14_publication".to_string();
    let original_node_id = NODE.to_string();
    let original_role_run_seq = 7u64;
    let chain_id = sc_failure_chain_id(
        &scope.session_id,
        &business_key,
        &original_node_id,
        original_role_run_seq,
    )
    .expect("valid chain identity segments");
    ScFailureChainIdentity {
        scope: scope.clone(),
        chain_id,
        business_key,
        original_node_id,
        original_role_run_seq,
    }
}

pub(super) fn package(identity: &ScFailureChainIdentity) -> ScFailureRepairPackage {
    let available = || ScEvidenceRef::Available {
        locator: "evidence".to_string(),
        sha256: "0".repeat(64),
        bytes: 0,
    };
    ScFailureRepairPackage {
        schema_version: 1,
        package_id: "pkg_p14_publication".to_string(),
        chain_id: identity.chain_id.clone(),
        session_id: identity.scope.session_id.clone(),
        project_id: identity.scope.project_id.clone(),
        issue_id: identity.scope.issue_id.clone(),
        plan_id: identity.scope.plan_id.clone(),
        terminal_class: "compile".to_string(),
        terminal_reason: "terminal compile failure".to_string(),
        completeness: ScPackageCompleteness::Complete,
        rounds: Vec::new(),
        last_provider_output: available(),
        last_failure_source: ScFailureSourceEvidence {
            source_round_id: "round_1".to_string(),
            source: available(),
        },
        teaching_prompts: Vec::new(),
        evidence_log_ref: "p14/evidence/rounds.jsonl".to_string(),
        evidence_digest: "a".repeat(64),
        evidence_through_entry_key: "entry_1".to_string(),
        missing_evidence: Vec::new(),
    }
}

pub(super) fn package_digest(package: &ScFailureRepairPackage) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(package).expect("package is serializable"));
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn ordinary(version: u32) -> ArtifactVersion {
    ArtifactVersion {
        version,
        payload: ArtifactPayload::Markdown {
            markdown: format!("# v{version}"),
            diff: None,
        },
        generated_by: ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: version == 1,
        created_at: "2026-11-13T00:00:00Z".to_string(),
        source_node_id: NODE.to_string(),
    }
}

pub(super) fn a4(
    version: u32,
    package: &ScFailureRepairPackage,
    is_current: bool,
) -> ArtifactVersion {
    ArtifactVersion {
        version,
        payload: ArtifactPayload::ScFailureRepairPackage {
            sc_failure_repair_package: Box::new(package.clone()),
        },
        generated_by: ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current,
        created_at: "2026-11-13T00:00:00Z".to_string(),
        source_node_id: NODE.to_string(),
    }
}

pub(super) fn node_detail(session_id: &str, artifact_ref: Option<ArtifactRef>) -> NodeDetail {
    NodeDetail {
        node_id: NODE.to_string(),
        session_id: session_id.to_string(),
        node_type: TimelineNodeType::AuthorRun,
        status: TimelineNodeStatus::Completed,
        agent_role: None,
        provider: None,
        prompt: None,
        messages: Vec::new(),
        streaming_content: String::new(),
        execution_events: Vec::new(),
        permission_events: Vec::new(),
        verdict: None,
        artifact_ref,
        is_revision: false,
        revision_feedback: None,
        base_artifact_ref: None,
        started_at: "2026-11-13T00:00:00Z".to_string(),
        ended_at: Some("2026-11-13T00:00:01Z".to_string()),
    }
}

pub(super) fn fixture_with_kind(
    flow_kind: WorkItemPlanFlowKind,
) -> (TempDir, LifecycleStore, WorkspaceSessionRecord) {
    let tmp = TempDir::new().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT.to_string(),
            issue_id: ISSUE.to_string(),
            entity_id: PLAN.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: false,
            }),
        })
        .unwrap();
    (tmp, store, session)
}

pub(super) fn fixture() -> (TempDir, LifecycleStore, WorkspaceSessionRecord) {
    fixture_with_kind(WorkItemPlanFlowKind::Legacy)
}

pub(super) fn fixture_sc() -> (TempDir, LifecycleStore, WorkspaceSessionRecord) {
    fixture_with_kind(WorkItemPlanFlowKind::SingleCandidate)
}

pub(super) fn receipt_path(
    store: &LifecycleStore,
    session_id: &str,
    chain_id: &str,
    package_id: &str,
) -> std::path::PathBuf {
    store
        .app_paths()
        .issue_lifecycle_root(PROJECT, ISSUE)
        .join("workspace-timelines")
        .join(session_id)
        .join("sc-failure-evidence")
        .join(chain_id)
        .join("published")
        .join(format!("{package_id}.json"))
}

fn evidence_log_path(
    store: &LifecycleStore,
    session_id: &str,
    chain_id: &str,
) -> std::path::PathBuf {
    store
        .app_paths()
        .issue_lifecycle_root(PROJECT, ISSUE)
        .join("workspace-timelines")
        .join(session_id)
        .join("sc-failure-evidence")
        .join(chain_id)
        .join("rounds.jsonl")
}

fn receipt_for(
    identity: &ScFailureChainIdentity,
    package: &ScFailureRepairPackage,
) -> ScArtifactPublishedReceipt {
    ScArtifactPublishedReceipt {
        schema_version: 1,
        package_id: package.package_id.clone(),
        chain_id: identity.chain_id.clone(),
        node_id: NODE.to_string(),
        package_digest: package_digest(package),
        version: 2,
    }
}

fn round_bound(identity: &ScFailureChainIdentity, session_id: &str) -> ScFailureEvidenceEvent {
    ScFailureEvidenceEvent::RoundBound {
        round: ScRoundIdentity {
            chain_id: identity.chain_id.clone(),
            round_id: "round_1".to_string(),
            node_id: NODE.to_string(),
            round_ordinal: 1,
            provider: ProviderName::Codex,
            workspace_session_id: session_id.to_string(),
            role_run_seq: 7,
            run_ref: "run_1".to_string(),
        },
    }
}

// Regression 3: a bad tail must return Err while preserving the real verified
// prefix (never a fabricated prefix); a non-SC session is rejected outright.
#[test]
fn p14_read_chain_preserves_verified_prefix_on_bad_log_and_rejects_non_sc() {
    let (_tmp, store, session) = fixture_sc();
    let sid = session.id.clone();
    let sc = scope(&sid);
    let id = identity(&sc);
    store
        .append_sc_failure_evidence(&id, &round_bound(&id, &sid))
        .unwrap();
    let log = evidence_log_path(&store, &sid, &id.chain_id);
    let mut bytes = std::fs::read(&log).unwrap();
    bytes.extend_from_slice(b"{\"corrupt\":\n");
    std::fs::write(&log, &bytes).unwrap();

    let err = store
        .read_sc_failure_chain_for_publication(&sc, &id.chain_id)
        .expect_err("corrupt tail must fail the strict read");
    let prefix = err
        .verified_prefix
        .expect("real verified prefix must be retained");
    assert!(!prefix.entries.is_empty(), "prefix keeps validated entries");

    // Non-SC identity is rejected even though the chain id recomputes.
    let (_tmp2, store2, session2) = fixture();
    let sid2 = session2.id.clone();
    let sc2 = scope(&sid2);
    let id2 = identity(&sc2);
    let err2 = store2
        .read_sc_failure_chain_for_publication(&sc2, &id2.chain_id)
        .expect_err("non-SC session must be rejected");
    assert_eq!(err2.code, "sc_failure_session_not_sc");
}

// Regression 4: receipt write admits a real verified-prefix SC identity on a
// bad log, refuses when no verified prefix is known, and never fabricates one.
#[test]
fn p14_put_receipt_admits_verified_prefix_on_bad_log_and_refuses_unknown_prefix() {
    // Unknown prefix: corrupt-only log => Err, zero receipt bytes.
    let (_tmp, store, session) = fixture_sc();
    let sid = session.id.clone();
    let sc = scope(&sid);
    let id = identity(&sc);
    let pkg = package(&id);
    let receipt = receipt_for(&id, &pkg);
    let log = evidence_log_path(&store, &sid, &id.chain_id);
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(&log, b"{\"corrupt\":\n").unwrap();
    let err = store
        .put_sc_artifact_published_receipt(&id, &receipt)
        .expect_err("no verified prefix must refuse the receipt");
    assert!(err.publication_incomplete);
    assert!(!receipt_path(&store, &sid, &id.chain_id, &pkg.package_id).exists());

    // Real verified prefix + corrupt tail => admitted; the receipt proves the
    // artifact publication only, never that the log is healthy.
    let (_tmp2, store2, session2) = fixture_sc();
    let sid2 = session2.id.clone();
    let _sc2 = scope(&sid2);
    let id2 = identity(&_sc2);
    let pkg2 = package(&id2);
    let receipt2 = receipt_for(&id2, &pkg2);
    store2
        .append_sc_failure_evidence(&id2, &round_bound(&id2, &sid2))
        .unwrap();
    let log2 = evidence_log_path(&store2, &sid2, &id2.chain_id);
    let mut bytes = std::fs::read(&log2).unwrap();
    bytes.extend_from_slice(b"{\"corrupt\":\n");
    std::fs::write(&log2, &bytes).unwrap();
    store2
        .put_sc_artifact_published_receipt(&id2, &receipt2)
        .unwrap();
    assert!(receipt_path(&store2, &sid2, &id2.chain_id, &pkg2.package_id).exists());
}

// F1 RED (no impl yet): a non-UTF8 tail must not destroy the real verified
// prefix. Strict log has a real RoundBound prefix; 0xFF corrupts the tail.
// Current whole-file `read_to_string` drops the prefix (verified_prefix=None),
// so this test FAILs until the module reads bytes and only the first physical
// line, then lets the existing strict read supply the verified prefix.
#[test]
fn p14_read_chain_preserves_verified_prefix_on_non_utf8_tail() {
    let (_tmp, store, session) = fixture_sc();
    let sid = session.id.clone();
    let sc = scope(&sid);
    let id = identity(&sc);
    store
        .append_sc_failure_evidence(&id, &round_bound(&id, &sid))
        .unwrap();
    let log = evidence_log_path(&store, &sid, &id.chain_id);
    let mut bytes = std::fs::read(&log).unwrap();
    bytes.push(0xFF);
    bytes.push(b'\n');
    std::fs::write(&log, &bytes).unwrap();

    let err = store
        .read_sc_failure_chain_for_publication(&sc, &id.chain_id)
        .expect_err("non-UTF8 tail must fail the strict read");
    let prefix = err
        .verified_prefix
        .expect("real verified prefix must survive a non-UTF8 tail");
    assert!(
        !prefix.entries.is_empty(),
        "prefix keeps the validated entries"
    );
}
