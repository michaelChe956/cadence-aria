use std::collections::BTreeSet;

use serde::Deserialize;

use super::{
    ClassificationError, FindingClass, FindingClassHint, FindingFingerprint, ParsedReviewEnvelope,
    ReviewFindingCategory, ReviewInvocationScope, classify_finding, classify_review,
};
use crate::web::workspace_ws_types::review::{
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdictType,
};

#[derive(Debug, Deserialize)]
struct GoldenFinding {
    id: String,
    verdict: ReviewVerdictType,
    finding: ReviewFinding,
    expected_class: FindingClass,
    expected_category: Option<ReviewFindingCategory>,
    contract_field: Option<String>,
}

#[test]
fn golden_findings_classify_to_the_expected_typed_outcomes() {
    let fixtures: Vec<GoldenFinding> =
        serde_json::from_str(include_str!("fixtures/golden_findings.json")).unwrap();
    assert_eq!(fixtures.len(), 14, "golden fixture count must remain fixed");
    let invocation = ReviewInvocationScope::initial("revision-001");

    for fixture in fixtures {
        let envelope = envelope(fixture.verdict, vec![fixture.finding]);
        let classified = classify_review(&envelope, &invocation, None)
            .unwrap_or_else(|error| panic!("fixture {} failed to classify: {error:?}", fixture.id));

        assert_eq!(classified.len(), 1, "fixture {}", fixture.id);
        assert_eq!(
            classified[0].class, fixture.expected_class,
            "fixture {}",
            fixture.id
        );
        assert_eq!(
            classified[0].category, fixture.expected_category,
            "fixture {}",
            fixture.id
        );
        assert_eq!(
            classified[0].contract_field, fixture.contract_field,
            "fixture {}",
            fixture.id
        );
    }
}

#[test]
fn hint_precedes_raw_verdict_and_fallback_rules_are_deterministic() {
    let invocation = ReviewInvocationScope::initial("revision-001");
    let cases = [
        (
            envelope(
                ReviewVerdictType::NeedsHuman,
                vec![finding(
                    "hint wins",
                    None,
                    Some(FindingClassHint::Repairable),
                    None,
                )],
            ),
            FindingClass::Repairable,
        ),
        (
            envelope(
                ReviewVerdictType::NeedsHuman,
                vec![finding("human fallback", None, None, None)],
            ),
            FindingClass::HumanRequired,
        ),
        (
            envelope(
                ReviewVerdictType::Pass,
                vec![finding("pass finding", None, None, None)],
            ),
            FindingClass::Advisory,
        ),
        (
            envelope(
                ReviewVerdictType::Revise,
                vec![finding(
                    "contract fallback",
                    Some(ReviewFindingCategory::ContractGap),
                    None,
                    Some("api.response"),
                )],
            ),
            FindingClass::Repairable,
        ),
        (
            envelope(
                ReviewVerdictType::Revise,
                vec![finding(
                    "scope fallback",
                    Some(ReviewFindingCategory::ScopeConflict),
                    None,
                    Some("api.response"),
                )],
            ),
            FindingClass::HumanRequired,
        ),
    ];

    for (envelope, expected_class) in cases {
        assert_eq!(
            classify_review(&envelope, &invocation, None).unwrap()[0].class,
            expected_class
        );
    }
}

#[test]
fn raw_verdict_is_retained_when_strong_findings_require_a_normalized_revision_gate() {
    let envelope = envelope(
        ReviewVerdictType::NeedsHuman,
        vec![finding(
            "strong finding must not rewrite raw verdict",
            None,
            None,
            None,
        )],
    );

    assert_eq!(envelope.raw_verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(envelope.normalized_gate, ReviewGate::RequiresRevision);
    assert_eq!(
        classify_review(
            &envelope,
            &ReviewInvocationScope::initial("revision-001"),
            None,
        )
        .unwrap()[0]
            .class,
        FindingClass::HumanRequired
    );
}

#[test]
fn verification_scope_allows_new_findings_but_rejects_missing_mechanical_report() {
    let original = FindingFingerprint::for_finding(
        Some(ReviewFindingCategory::ContractGap),
        FindingClass::Repairable,
        "original",
        Some("contract.field"),
    );
    let invocation = ReviewInvocationScope::verification(
        BTreeSet::from([original]),
        "revision-002",
        "mechanical-report-002",
        None,
    );
    let new_finding = envelope(
        ReviewVerdictType::Revise,
        vec![finding(
            "new finding",
            Some(ReviewFindingCategory::ContractGap),
            None,
            Some("contract.field"),
        )],
    );
    assert!(classify_review(&new_finding, &invocation, Some("mechanical-report-002"),).is_ok());

    let missing_report =
        ReviewInvocationScope::verification(BTreeSet::new(), "revision-002", "", None);
    let pass = envelope(ReviewVerdictType::Pass, Vec::new());
    assert!(matches!(
        classify_review(&pass, &missing_report, None),
        Err(ClassificationError::VerificationScopeViolation(_))
    ));
    assert!(matches!(
        classify_review(&pass, &invocation, Some("another-mechanical-report")),
        Err(ClassificationError::VerificationScopeViolation(_))
    ));
}

#[test]
fn finding_class_as_str_matches_serde_value() {
    for class in [
        FindingClass::MechanicalError,
        FindingClass::Repairable,
        FindingClass::HumanRequired,
        FindingClass::Advisory,
    ] {
        assert_eq!(serde_json::to_value(class).unwrap(), class.as_str());
    }
}

#[test]
fn parsed_enum_values_remain_available_for_schema_authors() {
    assert_eq!(
        serde_json::to_value(FindingClassHint::HumanRequired).unwrap(),
        "human_required"
    );
}

fn envelope(raw_verdict: ReviewVerdictType, findings: Vec<ReviewFinding>) -> ParsedReviewEnvelope {
    let normalized_gate = if findings.iter().any(|finding| {
        matches!(
            finding.severity,
            ReviewFindingSeverity::Blocking | ReviewFindingSeverity::MustFix
        )
    }) {
        ReviewGate::RequiresRevision
    } else {
        ReviewGate::UserConfirmAllowed
    };
    ParsedReviewEnvelope {
        raw_verdict,
        normalized_gate,
        findings,
    }
}

fn finding(
    message: &str,
    category: Option<ReviewFindingCategory>,
    class_hint: Option<FindingClassHint>,
    contract_field: Option<&str>,
) -> ReviewFinding {
    ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: message.to_string(),
        evidence: String::new(),
        required_action: String::new(),
        category,
        class_hint,
        contract_field: contract_field.map(ToString::to_string),
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    }
}

/// REQ-WSC-12 平台硬规则（k3 REQ-WSC-12 不符合项）：`disposition != actionable`
/// 先于 `class_hint` / 旧轴 fallback 生效——模型把范围外事实标成
/// `repairable`/`human_required` 也不再进指纹集合、不驱返修、不占 transition
/// budget（`actionable_findings` 的 `class != Advisory` 过滤才真正吃掉它）。
#[test]
fn advisory_disposition_preempts_class_hint_for_classification() {
    let mut advisory = finding(
        "仓库里那份前端规范文档的『中文 locale 尚未配置』缺口",
        Some(ReviewFindingCategory::Other),
        Some(FindingClassHint::HumanRequired),
        None,
    );
    advisory.disposition =
        Some(crate::web::workspace_ws_types::review::ReviewFindingDisposition::Advisory);

    let classified = classify_finding(ReviewVerdictType::Revise, &advisory);
    assert_eq!(
        classified.class,
        FindingClass::Advisory,
        "advisory 判定必须压过 class_hint，否则指纹闸门/返修预算照旧被消耗"
    );

    // 对照组：同一 payload 去掉 disposition → 旧轴照旧 human_required。
    let mut actionable = advisory.clone();
    actionable.disposition = None;
    assert_eq!(
        classify_finding(ReviewVerdictType::Revise, &actionable).class,
        FindingClass::HumanRequired
    );
}
