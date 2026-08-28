use serde_json::Value;

use super::{DIAGNOSTIC_FIXTURES, EXPECTED_DIAGNOSTICS};

const CLASSIFIER_GOLDEN_FINDINGS: &str =
    include_str!("../../work_item_plan_policy/fixtures/golden_findings.json");
const REVIEWER_FINDING_CHANNEL_MAP: &str = include_str!(
    "../../../../openspec/changes/rearch-workitem-plan-pipeline/fixtures/reviewer-finding-channel-map.json"
);

#[test]
fn reviewer_finding_channel_boundary() {
    let classifier_goldens: Vec<Value> = serde_json::from_str(CLASSIFIER_GOLDEN_FINDINGS)
        .expect("classifier golden fixture 必须是合法 JSON");
    assert_eq!(
        classifier_goldens.len(),
        14,
        "classifier golden 数量必须固定为 14"
    );
    assert_eq!(
        classifier_goldens
            .iter()
            .filter(|finding| finding["source_kind"] == "provider_raw")
            .count(),
        11,
        "provider 原始 finding 数量必须固定为 11"
    );
    assert_eq!(
        classifier_goldens
            .iter()
            .filter(|finding| finding["source_kind"] == "annotated_variant")
            .count(),
        3,
        "人工 class_hint 变体数量必须固定为 3"
    );

    let reviewer_findings = classifier_goldens
        .iter()
        .filter(|finding| {
            finding["source_kind"] == "provider_raw"
                && matches!(
                    finding["source_run"].as_str(),
                    Some("rep2" | "rep3" | "rep4")
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reviewer_findings.len(),
        9,
        "只有 rep2/3/4 的九条 provider 原始 finding 可进入 reviewer channel map"
    );

    let channel_map: Vec<Value> = serde_json::from_str(REVIEWER_FINDING_CHANNEL_MAP)
        .expect("reviewer finding channel map 必须是合法 JSON");
    assert_eq!(
        channel_map.len(),
        reviewer_findings.len(),
        "reviewer channel map 必须与 rep2/3/4 的九条原始 finding 一一对应"
    );

    let mut reviewer_ids = reviewer_findings
        .iter()
        .map(|finding| finding["id"].as_str().expect("provider finding 必须有 ID"))
        .collect::<Vec<_>>();
    let mut channel_map_ids = channel_map
        .iter()
        .map(|entry| entry["id"].as_str().expect("channel map entry 必须有 ID"))
        .collect::<Vec<_>>();
    reviewer_ids.sort_unstable();
    channel_map_ids.sort_unstable();
    assert_eq!(
        channel_map_ids, reviewer_ids,
        "channel map 不得漏掉、重复或加入 classifier annotated variant"
    );

    for entry in &channel_map {
        let entry = entry.as_object().expect("channel map entry 必须是对象");
        let mut keys = entry.keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "channel",
                "compiler_fixture",
                "finding",
                "id",
                "source_kind",
                "source_run",
                "verdict",
            ],
            "channel map 只能携带原始 finding 与路由字段"
        );
        assert_eq!(entry["channel"], "prompt_few_shot");
        assert!(entry["compiler_fixture"].is_null());
        assert_eq!(entry["source_kind"], "provider_raw");

        let source = reviewer_findings
            .iter()
            .find(|finding| finding["id"] == entry["id"])
            .expect("channel map ID 必须存在于 provider 原始 finding");
        for field in ["source_run", "verdict", "finding"] {
            assert_eq!(entry[field], source[field]);
        }
        let finding = entry["finding"]
            .as_object()
            .expect("channel map finding 必须是对象");
        assert_eq!(finding.len(), 4, "provider finding 不得混入人工标注字段");
        for field in ["severity", "message", "evidence", "required_action"] {
            assert!(
                finding[field]
                    .as_str()
                    .is_some_and(|value| !value.is_empty()),
                "prompt few-shot finding 必须保留非空 {field}"
            );
        }
    }

    let expected_diagnostics: Vec<Value> =
        serde_json::from_str(EXPECTED_DIAGNOSTICS).expect("expected diagnostics 必须是合法 JSON");
    let mut expected_fixture_names = expected_diagnostics
        .iter()
        .map(|entry| {
            entry["fixture"]
                .as_str()
                .expect("diagnostic 必须指定 fixture")
        })
        .collect::<Vec<_>>();
    let mut compiler_fixture_names = DIAGNOSTIC_FIXTURES
        .iter()
        .map(|(fixture, _)| *fixture)
        .collect::<Vec<_>>();
    expected_fixture_names.sort_unstable();
    compiler_fixture_names.sort_unstable();
    assert_eq!(
        expected_fixture_names, compiler_fixture_names,
        "Task 1.3 的四个 compiler diagnostics 只能来自独立 grammar/lowering fixture"
    );
}
