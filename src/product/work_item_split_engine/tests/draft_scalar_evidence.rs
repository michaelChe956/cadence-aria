#[test]
fn pi_recorded_scalar_required_evidence_payload_parses() {
    // Regression fixture: real Pi draft output from workspace_session_0003/timeline_node_014
    // where every acceptance criterion emitted `required_evidence` as a scalar string
    // instead of an array, failing with "data did not match any variant of untagged enum
    // ProviderWorkItemDraftInput".
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/product/work_item_split_engine/tests/fixtures")
        .join("pi_draft_scalar_required_evidence.json");
    let raw = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|error| panic!("read fixture {fixture_path:?}: {error}"));
    let value: serde_json::Value = serde_json::from_str(&raw).expect("fixture must be valid JSON");

    let candidate = crate::product::work_item_split_engine::parse::parse_work_item_draft_output(
        value.clone(),
    )
    .expect("scalar required_evidence payload must parse");

    assert_eq!(candidate.outline_id, "outline_backend");
    assert_eq!(candidate.logical_work_item_id, "wi_backend");
    for criterion in &candidate.canonical_contract_candidate.acceptance_criteria {
        assert!(
            !criterion.required_evidence.is_empty(),
            "normalized evidence list must be non-empty for {}",
            criterion.criterion_id
        );
    }
}

// Providers may treat `canonical_contract.verification_checks` and
// `verification_plan.checks` as one field because the schema listed them on the
// same line. Aria requires both and the draft validator enforces that they are
// equal (`verification_plan_not_derived_from_contract`), so a missing contract
// copy is backfilled from the plan instead of failing the whole draft.
#[test]
fn pi_recorded_missing_verification_checks_payload_parses() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/product/work_item_split_engine/tests/fixtures")
        .join("pi_draft_missing_verification_checks.json");
    let raw = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|error| panic!("read fixture {fixture_path:?}: {error}"));
    let value: serde_json::Value = serde_json::from_str(&raw).expect("fixture must be valid JSON");
    assert!(
        value["draft"]["canonical_contract"]
            .get("verification_checks")
            .is_none(),
        "fixture must reproduce the missing contract checks"
    );

    let candidate = crate::product::work_item_split_engine::parse::parse_work_item_draft_output(value)
        .expect("payload missing contract verification_checks must parse");

    assert!(
        !candidate
            .canonical_contract_candidate
            .verification_checks
            .is_empty(),
        "contract checks must be backfilled"
    );
    assert_eq!(
        candidate.canonical_contract_candidate.verification_checks,
        candidate.verification_plan.checks,
        "backfilled contract checks must equal the verification plan checks"
    );
}

#[test]
fn missing_verification_checks_stays_rejected_when_verification_plan_is_empty() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/product/work_item_split_engine/tests/fixtures")
        .join("pi_draft_missing_verification_checks.json");
    let raw = std::fs::read_to_string(&fixture_path).expect("read fixture");
    let mut value: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    value["draft"]["verification_plan"]["checks"] = serde_json::json!([]);

    assert!(
        crate::product::work_item_split_engine::parse::parse_work_item_draft_output(value).is_err(),
        "an empty verification plan must not silently satisfy the contract"
    );
}
