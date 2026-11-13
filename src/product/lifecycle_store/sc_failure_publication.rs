//! P14 publication-store barrier — **RED landing** (consumer tests only).
//!
//! This module intentionally contains **no implementation** yet: the four frozen
//! `LifecycleStore` methods and the `ArtifactVersionsWrite::save` A4 merge land
//! only on the GREEN GO. Until then these tests are compile-RED against the
//! missing interfaces and the pending `ArtifactPayload::ScFailureRepairPackage`
//! variant (owner: P11 publisher), forming the atomic G-DTO group.
//!
//! Frozen binding (do not drift):
//! - receipt path: `<timeline-root>/sc-failure-evidence/<chain_id>/published/<package_id>.json`
//! - validate loads `load_node_detail(scope.session_id, &receipt.node_id)` and
//!   requires `artifact_ref` same version/id as the persisted version entry;
//! - `put_sc_artifact_published_receipt` performs real SC admission via
//!   `read_sc_failure_evidence(identity)?` before any write.

#[cfg(test)]
mod tests {
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::sc_failure_evidence::sc_failure_chain_id;
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
        ScPackageCompleteness, TimelineNodeStatus, TimelineNodeType,
    };
    use tempfile::TempDir;

    const PROJECT: &str = "project_p14_publication";
    const ISSUE: &str = "issue_p14_publication";
    const PLAN: &str = "plan_p14_publication";
    const NODE: &str = "node_p14_publication";

    fn scope(session_id: &str) -> ScFailureScope {
        ScFailureScope {
            session_id: session_id.to_string(),
            project_id: PROJECT.to_string(),
            issue_id: ISSUE.to_string(),
            plan_id: PLAN.to_string(),
        }
    }

    fn identity(scope: &ScFailureScope) -> ScFailureChainIdentity {
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

    fn package(identity: &ScFailureChainIdentity) -> ScFailureRepairPackage {
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

    fn package_digest(package: &ScFailureRepairPackage) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(serde_json::to_vec(package).expect("package is serializable"));
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn ordinary(version: u32) -> ArtifactVersion {
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

    fn a4(version: u32, package: &ScFailureRepairPackage, is_current: bool) -> ArtifactVersion {
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

    fn node_detail(session_id: &str, artifact_ref: Option<ArtifactRef>) -> NodeDetail {
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

    fn fixture_with_kind(
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

    fn fixture() -> (TempDir, LifecycleStore, WorkspaceSessionRecord) {
        fixture_with_kind(WorkItemPlanFlowKind::Legacy)
    }

    fn fixture_sc() -> (TempDir, LifecycleStore, WorkspaceSessionRecord) {
        fixture_with_kind(WorkItemPlanFlowKind::SingleCandidate)
    }

    fn receipt_path(
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

    // Regression 1: a stale ordinary save that omits a pending A4 entry must retain
    // that A4 verbatim; ordinary deletion must not resurrect; A4 tamper/current rejected.
    #[test]
    fn p14_stale_save_preserves_pending_a4_and_ordinary_deletion_does_not_resurrect() {
        let (_tmp, store, session) = fixture();
        let sid = session.id.clone();
        let sc = scope(&sid);
        let id = identity(&sc);
        let package = package(&id);

        store.save_artifact_versions(&sid, &[ordinary(1)]).unwrap();
        store
            .append_artifact_version(&sid, a4(0, &package, false))
            .unwrap();
        let raw = store.list_artifact_versions(&sid).unwrap();
        let pending = raw
            .iter()
            .find(|entry| matches!(&entry.payload, ArtifactPayload::ScFailureRepairPackage { .. }))
            .cloned()
            .expect("A4 pending entry persisted");
        assert_eq!(pending.version, 2, "append allocates max(raw)+1");
        assert!(!pending.is_current, "A4 must never be current");

        // Stale ordinary writer proposes only v1 (read before the A4 existed).
        store.save_artifact_versions(&sid, &[ordinary(1)]).unwrap();
        let after = store.list_artifact_versions(&sid).unwrap();
        assert_eq!(after.len(), 2, "pending A4 retained on stale save");
        assert_eq!(
            after
                .iter()
                .find(|entry| entry.version == 2)
                .expect("A4 retained"),
            &pending,
            "retained A4 keeps every field verbatim"
        );

        // Ordinary deletion: proposed [] removes v1; A4 retained; v1 not resurrected.
        store.save_artifact_versions(&sid, &[]).unwrap();
        let deleted = store.list_artifact_versions(&sid).unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].version, 2);
        assert!(matches!(
            &deleted[0].payload,
            ArtifactPayload::ScFailureRepairPackage { .. }
        ));

        // Same-version A4 payload change is rejected.
        let mut tampered = pending.clone();
        if let ArtifactPayload::ScFailureRepairPackage {
            sc_failure_repair_package,
        } = &mut tampered.payload
        {
            sc_failure_repair_package.evidence_digest = "f".repeat(64);
        }
        assert!(
            store.save_artifact_versions(&sid, &[tampered]).is_err(),
            "same-version A4 payload change must be rejected"
        );

        // A4 with is_current=true is rejected.
        let mut current = pending.clone();
        current.is_current = true;
        assert!(
            store.save_artifact_versions(&sid, &[current]).is_err(),
            "A4 must stay is_current=false"
        );

        // Same package_id with a different digest at another version is rejected.
        let mut other_package = package.clone();
        other_package.evidence_digest = "1".repeat(64);
        assert!(
            store
                .save_artifact_versions(&sid, &[pending.clone(), a4(5, &other_package, false)])
                .is_err(),
            "same package_id with different digest must be rejected"
        );

        // Rejected writes left raw untouched.
        assert_eq!(store.list_artifact_versions(&sid).unwrap(), deleted);
    }

    // Regression 2: A4 without receipt is excluded (pending); real receipt makes it
    // visible; tampered receipt is a hard Err (never an empty Ok list). Receipt lives
    // under the chain dir; validate cross-checks the durable node ArtifactRef by
    // `receipt.node_id` (same version/id), never receipt self-attestation.
    #[test]
    fn p14_pending_a4_filtered_then_published_visible_and_tamper_is_err() {
        let (_tmp, store, session) = fixture_sc();
        let sid = session.id.clone();
        let sc = scope(&sid);
        let id = identity(&sc);
        let package = package(&id);
        let receipt = ScArtifactPublishedReceipt {
            schema_version: 1,
            package_id: package.package_id.clone(),
            chain_id: id.chain_id.clone(),
            node_id: NODE.to_string(),
            package_digest: package_digest(&package),
            version: 2,
        };

        store
            .save_artifact_versions(&sid, &[ordinary(1), a4(2, &package, false)])
            .unwrap();
        // Durable node ref must point at the exact published version.
        store
            .save_node_detail(
                &sid,
                NODE,
                &node_detail(
                    &sid,
                    Some(ArtifactRef {
                        artifact_id: format!("artifact_version_{:03}", 2),
                        version: 2,
                    }),
                ),
            )
            .unwrap();
        let raw = store.list_artifact_versions(&sid).unwrap();
        let a4_version = raw
            .iter()
            .find(|entry| entry.version == 2)
            .cloned()
            .expect("A4 persisted");

        // Missing receipt: validate is incomplete Err; filter drops the pending A4.
        let missing = store
            .validate_sc_artifact_publication(&sc, &a4_version)
            .expect_err("missing receipt must fail validation");
        assert!(missing.publication_incomplete);
        assert_eq!(missing.code, "sc_failure_publication_receipt_missing");
        let pending_view = store
            .filter_sc_published_artifact_versions(&sc, raw.clone())
            .unwrap();
        assert_eq!(
            pending_view
                .iter()
                .map(|entry| entry.version)
                .collect::<Vec<_>>(),
            vec![1],
            "pending A4 excluded from public view"
        );

        // Real receipt (chain-dir layout): version becomes visible and validates.
        store
            .put_sc_artifact_published_receipt(&id, &receipt)
            .unwrap();
        store
            .validate_sc_artifact_publication(&sc, &a4_version)
            .unwrap();
        let published_view = store
            .filter_sc_published_artifact_versions(&sc, raw.clone())
            .unwrap();
        assert_eq!(
            published_view
                .iter()
                .map(|entry| entry.version)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "published A4 visible after receipt"
        );

        // Tampered digest: hard Err from both validate and filter (not an empty list).
        let mut bad_receipt = receipt.clone();
        bad_receipt.package_digest = "f".repeat(64);
        std::fs::write(
            receipt_path(&store, &sid, &id.chain_id, &package.package_id),
            serde_json::to_vec(&bad_receipt).unwrap(),
        )
        .unwrap();
        assert!(
            store
                .validate_sc_artifact_publication(&sc, &a4_version)
                .is_err(),
            "tampered receipt digest must error"
        );
        let tamper_err = store
            .filter_sc_published_artifact_versions(&sc, raw.clone())
            .expect_err("tampered receipt must not become an empty Ok list");
        assert_eq!(tamper_err.code, "sc_failure_publication_receipt_mismatch");
        assert!(tamper_err.publication_incomplete);

        // Broken node ref must also fail validation even with a good receipt.
        store
            .save_node_detail(
                &sid,
                NODE,
                &node_detail(
                    &sid,
                    Some(ArtifactRef {
                        artifact_id: format!("artifact_version_{:03}", 1),
                        version: 1,
                    }),
                ),
            )
            .unwrap();
        std::fs::write(
            receipt_path(&store, &sid, &id.chain_id, &package.package_id),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        assert!(
            store
                .validate_sc_artifact_publication(&sc, &a4_version)
                .is_err(),
            "node ref pointing at another version must fail validation"
        );

        // Restore the correct node ref and remove the receipt: filter drops the pending A4.
        store
            .save_node_detail(
                &sid,
                NODE,
                &node_detail(
                    &sid,
                    Some(ArtifactRef {
                        artifact_id: format!("artifact_version_{:03}", 2),
                        version: 2,
                    }),
                ),
            )
            .unwrap();
        std::fs::remove_file(receipt_path(
            &store,
            &sid,
            &id.chain_id,
            &package.package_id,
        ))
        .unwrap();
        let dropped = store
            .filter_sc_published_artifact_versions(&sc, raw)
            .unwrap();
        assert_eq!(
            dropped
                .iter()
                .map(|entry| entry.version)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }
}
