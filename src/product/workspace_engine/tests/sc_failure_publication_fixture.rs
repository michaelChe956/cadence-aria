// P11 严格 publisher 用例的共享真实 fixture（2026-11-13）。
// 落位：src/product/workspace_engine/tests/sc_failure_publication_fixture.rs
// 挂载：tests/sc_failure_publication.rs 内 `#[path = "sc_failure_publication_fixture.rs"] mod fixture;`
//
// 只碰真实件：真实 LifecycleStore、真实 SC single-candidate 会话、真实 split
// provider run（只落盘，不调用任何 provider）、真实证据日志、真实 timeline 节点。
// 无模型调用、无桩 store、无内存假对象。
//
// 本文件只提供 fixture 与只读助手；断言全部落在 publication 用例文件里。

use super::super::build_sc_failure_repair_package;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceEvent, sc_failure_chain_id, sc_failure_split_locator,
};
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use crate::product::models::{
    ArtifactVersion, ProviderName, WorkspaceRolePermissionModes, WorkspaceType,
};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
use crate::web::workspace_ws_types::{
    ArtifactPayload, ProviderConfigSnapshot, ScArtifactPublishedReceipt, ScEvidenceRef,
    ScFailureChainIdentity, ScFailureClass, ScFailureScope, ScFailureRepairPackage,
    ScPackageCompleteness, ScRoundIdentity, ScRoundLaunchState, ScTeachingDecision, TimelineNode,
    TimelineNodeStatus, TimelineNodeType, WorkspaceStage,
};
use std::path::PathBuf;
use tempfile::TempDir;

pub(super) const PROJECT_ID: &str = "project_0001";
pub(super) const ISSUE_ID: &str = "issue_0001";
pub(super) const PLAN_ID: &str = "entity_0001";
pub(super) const BUSINESS_KEY: &str = "business_key_sc_0001";
pub(super) const NODE_ID: &str = "node_sc_0001";
pub(super) const RAW: &str = "provider raw：{\"plan\":\"候选一\"}\n第二行 tab\t quotes \"x\"";
pub(super) const SOURCE: &str = "normalized author delivery source（去 preamble 后）";
pub(super) const REASON: &str = "E0308:3: mismatched types";
pub(super) const ORDINARY_MARKDOWN: &str = "# 普通产物\n\n与 SC 修复包无关的既有 current 版本。\n";
pub(super) const CREATED_AT: &str = "2026-11-13T00:00:00Z";

/// 真实 fixture：store + SC 会话 + 已交付的 split provider run + 完整证据日志。
pub(super) struct PublicationFixture {
    pub(super) _tmp: TempDir,
    pub(super) store: LifecycleStore,
    pub(super) session_id: String,
    pub(super) scope: ScFailureScope,
    pub(super) chain: ScFailureChainIdentity,
    pub(super) package: ScFailureRepairPackage,
}

pub(super) fn sha_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn available(locator: &str, body: &str) -> ScEvidenceRef {
    ScEvidenceRef::Available {
        locator: locator.to_string(),
        sha256: sha_hex(body.as_bytes()),
        bytes: body.as_bytes().len() as u64,
    }
}

/// 真实 fixture 装配：会话 → provider run → 证据日志 → 严格读回 → 纯组包。
pub(super) fn publication_fixture() -> PublicationFixture {
    let tmp = TempDir::new().expect("temp dir");
    let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let record = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: PLAN_ID.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .expect("SC session");
    let session_id = record.id.clone();

    // 真实 provider run（只落盘，不调用任何 provider）：raw 正文逐字入库。
    let handle = store
        .begin_work_item_split_provider_run(
            PROJECT_ID,
            ISSUE_ID,
            &ProviderName::Codex,
            &session_id,
        )
        .expect("begin split provider run");
    store
        .complete_work_item_split_provider_run(
            &handle,
            "split prompt",
            &serde_json::Value::String(RAW.to_string()),
        )
        .expect("complete split provider run");

    let scope = ScFailureScope {
        session_id: session_id.clone(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
    };
    let chain = ScFailureChainIdentity {
        scope: scope.clone(),
        chain_id: sc_failure_chain_id(&session_id, BUSINESS_KEY, NODE_ID, handle.role_run_seq)
            .expect("chain id"),
        business_key: BUSINESS_KEY.to_string(),
        original_node_id: NODE_ID.to_string(),
        original_role_run_seq: handle.role_run_seq,
    };
    let round = ScRoundIdentity {
        chain_id: chain.chain_id.clone(),
        round_id: handle.run_ref.clone(),
        node_id: NODE_ID.to_string(),
        round_ordinal: 1,
        provider: ProviderName::Codex,
        workspace_session_id: handle.workspace_session_id.clone(),
        role_run_seq: handle.role_run_seq,
        run_ref: handle.run_ref.clone(),
    };
    let locator = sc_failure_split_locator(&session_id, handle.role_run_seq);

    store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .expect("bind round");
    store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::FailureObserved {
                round_id: round.round_id.clone(),
                failure_class: Some(ScFailureClass::Compile),
                blocking_reasons: vec![REASON.to_string()],
                repair_instructions: vec!["先修 A 再修 B".to_string()],
                raw: available(&locator, RAW),
                source: available(&locator, SOURCE),
                existing_failure_id: None,
                attempt_key: None,
            },
        )
        .expect("observe failure");
    store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::TeachingDecision {
                failed_round_id: round.round_id.clone(),
                decision: ScTeachingDecision::NotRequested,
            },
        )
        .expect("teaching decision");
    store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::ChainTerminal {
                last_failure_round_id: round.round_id.clone(),
                last_physical_round_id: Some(round.round_id.clone()),
                terminal_class: "compile".to_string(),
                terminal_reason: REASON.to_string(),
                launch_observations: vec![(round.round_id.clone(), ScRoundLaunchState::Started)],
                completeness: ScPackageCompleteness::Complete,
                missing_evidence: Vec::new(),
            },
        )
        .expect("terminal");

    // 真实 store 严格读回 → 纯组包器（与 publisher 内部同一条路径）。
    let evidence = store
        .read_sc_failure_evidence(&chain)
        .expect("strict evidence read");
    let package = build_sc_failure_repair_package(&chain, &evidence).expect("repair package");

    PublicationFixture {
        _tmp: tmp,
        store,
        session_id,
        scope,
        chain,
        package,
    }
}

/// 普通 current 版本：发布 SC 包不得动它。
pub(super) fn ordinary_version() -> ArtifactVersion {
    ArtifactVersion {
        version: 1,
        payload: ArtifactPayload::Markdown {
            markdown: ORDINARY_MARKDOWN.to_string(),
            diff: None,
        },
        generated_by: ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: CREATED_AT.to_string(),
        source_node_id: NODE_ID.to_string(),
    }
}

/// 真实 timeline 节点：节点详情落盘的前提（缺它 → "timeline node not found"）。
pub(super) fn seed_timeline_node(store: &LifecycleStore, session_id: &str) {
    store
        .save_timeline_nodes(
            session_id,
            &[TimelineNode {
                node_id: NODE_ID.to_string(),
                node_type: TimelineNodeType::AuthorRun,
                agent: Some(ProviderName::Codex),
                stage: WorkspaceStage::Running,
                round: Some(1),
                status: TimelineNodeStatus::Active,
                title: "SC author run".to_string(),
                summary: None,
                started_at: CREATED_AT.to_string(),
                completed_at: None,
                duration_ms: None,
                artifact_ref: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    permission_modes: WorkspaceRolePermissionModes {
                        author: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
                        reviewer: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
                    },
                },
                retry: None,
            }],
        )
        .expect("seed timeline node");
}

/// 证据分区目录（与 store 内部落盘布局一致：`sc-failure-evidence/<chain_id>`）。
pub(super) fn chain_dir(store: &LifecycleStore, chain: &ScFailureChainIdentity) -> PathBuf {
    store
        .workspace_timeline_root_for_session(&chain.scope.session_id)
        .expect("timeline root")
        .join("sc-failure-evidence")
        .join(&chain.chain_id)
}

/// 直接读回执文件（不新增 store 公开读接口）：分区目录里逐个 JSON 试解。
pub(super) fn receipt_from_disk(
    store: &LifecycleStore,
    chain: &ScFailureChainIdentity,
) -> Option<ScArtifactPublishedReceipt> {
    let directory = chain_dir(store, chain);
    let entries = std::fs::read_dir(&directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Ok(receipt) = serde_json::from_slice::<ScArtifactPublishedReceipt>(&bytes) {
            return Some(receipt);
        }
    }
    None
}
