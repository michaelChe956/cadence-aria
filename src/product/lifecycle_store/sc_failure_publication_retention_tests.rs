use super::prefix_tests::*;
use crate::product::models::ArtifactRef;
use crate::web::workspace_ws_types::{ArtifactPayload, ScArtifactPublishedReceipt};

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
        .find(|entry| {
            matches!(
                &entry.payload,
                ArtifactPayload::ScFailureRepairPackage { .. }
            )
        })
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
