use cadence_aria::cross_cutting::openspec_constraints::{
    build_openspec_source_manifest, compile_constraint_bundle,
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

#[test]
fn constraint_bundle_top_level_schema_uses_contract_fields_only() {
    let manifest =
        build_openspec_source_manifest(Path::new("tests/fixtures/openspec/changes/sample-change"))
            .expect("manifest");
    let bundle = compile_constraint_bundle(
        &"sample-change".to_string(),
        &manifest,
        vec!["proj_spec_projection_art_spec_001_0001".to_string()],
        "N11".to_string(),
    )
    .expect("bundle");

    let value = serde_json::to_value(&bundle).expect("bundle json");
    let Value::Object(object) = value else {
        panic!("bundle must serialize as json object");
    };

    let keys: BTreeSet<String> = object.keys().cloned().collect();
    let expected = BTreeSet::from([
        "constraint_bundle_id".to_string(),
        "bundle_version".to_string(),
        "bundle_status".to_string(),
        "change_id".to_string(),
        "proposal_constraints".to_string(),
        "requirement_constraints".to_string(),
        "design_constraints".to_string(),
        "task_constraints".to_string(),
        "traceability_requirements".to_string(),
        "coverage_model".to_string(),
        "source_manifest".to_string(),
        "compiled_from_projection_refs".to_string(),
        "compiled_at".to_string(),
        "compiled_by_node".to_string(),
    ]);
    assert_eq!(keys, expected);

    for helper_key in [
        "scope_constraints",
        "requirement_ids",
        "task_ids",
        "traceability_map",
    ] {
        assert!(
            !object.contains_key(helper_key),
            "helper key leaked as bundle top-level field: {helper_key}"
        );
    }
}
