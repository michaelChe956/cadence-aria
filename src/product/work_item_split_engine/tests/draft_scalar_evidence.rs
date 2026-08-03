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
