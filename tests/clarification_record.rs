use cadence_aria::cross_cutting::artifact_validate::{ArtifactContent, canonical_validator};
use cadence_aria::protocol::artifacts::ArtifactKind;
use cadence_aria::runtime_units::clarification::normalize_clarification_record_candidate;
use serde_json::json;

#[test]
fn clarification_record_normalization_defaults_missing_array_fields() {
    let mut record = json!({
        "artifact_kind": "clarification_record",
        "goal_summary": "实现登录功能",
        "constraints": ["JWT"],
        "suggested_scope": "前后端登录闭环"
    });

    normalize_clarification_record_candidate(&mut record);

    assert_eq!(record["assumptions"], json!([]));
    assert_eq!(record["open_questions"], json!([]));
    canonical_validator(
        ArtifactKind::ClarificationRecord,
        &ArtifactContent::Json(record),
    )
    .expect("normalized clarification record should validate");
}
