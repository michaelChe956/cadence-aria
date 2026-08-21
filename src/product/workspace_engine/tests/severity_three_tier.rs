#[test]
fn parse_review_verdict_revise_without_findings_requires_user_triage() {
    let output = r#"建议修改一些描述。

```json
{"verdict":"revise","summary":"建议修改描述"}
```"#;

    let verdict = WorkspaceEngine::parse_review_verdict(output);

    assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
    assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
    assert!(verdict.findings.is_empty());
}

#[test]
fn live_review_schema_rejects_legacy_severities_and_impact_field() {
    for severity in [
        "strong_recommend_fix",
        "minor",
        "optional",
    ] {
        let value = serde_json::json!({
            "verdict": "pass",
            "findings": [{
                "severity": severity,
                "message": "legacy finding"
            }]
        });
        assert!(
            parse_review_value(&value, "").is_err(),
            "live schema must reject legacy severity {severity}"
        );
    }

    let value = serde_json::json!({
        "verdict": "pass",
        "findings": [{
            "severity": "suggestion",
            "message": "finding",
            "impact": "legacy impact"
        }]
    });
    assert!(
        parse_review_value(&value, "").is_err(),
        "live schema must reject the removed impact field"
    );
}

#[test]
fn historical_review_deserialization_normalizes_all_legacy_severities_and_impact_idempotently() {
    let value = serde_json::json!({
        "verdict": "pass",
        "comments": "historical comments",
        "summary": "historical",
        "findings": [
            { "severity": "blocking", "message": "blocking" },
            { "severity": "must_fix", "message": "must fix" },
            { "severity": "suggestion", "message": "suggestion" },
            { "severity": "strong_recommend_fix", "message": "strong" },
            { "severity": "minor", "message": "minor" },
            { "severity": "optional", "message": "optional", "impact": "legacy impact" }
        ]
    });

    let verdict = deserialize_historical_review_verdict(value).expect("legacy verdict");
    assert_eq!(
        verdict
            .findings
            .iter()
            .map(|finding| finding.severity.clone())
            .collect::<Vec<_>>(),
        vec![
            ReviewFindingSeverity::Blocking,
            ReviewFindingSeverity::MustFix,
            ReviewFindingSeverity::Suggestion,
            ReviewFindingSeverity::MustFix,
            ReviewFindingSeverity::Suggestion,
            ReviewFindingSeverity::Suggestion,
        ]
    );
    assert_eq!(verdict.findings[5].message, "optional\n影响：legacy impact");

    let round_trip = serde_json::to_value(&verdict).expect("normalized verdict");
    let serialized = round_trip.to_string();
    assert!(!serialized.contains("strong_recommend_fix"));
    assert!(!serialized.contains("\\\"minor\\\""));
    assert!(!serialized.contains("\\\"optional\\\""));
    assert!(!serialized.contains("\\\"impact\\\""));

    let replayed = deserialize_historical_review_verdict(round_trip).expect("replayed verdict");
    assert_eq!(replayed.findings[5].message, verdict.findings[5].message);

    let already_appended = deserialize_historical_review_verdict(serde_json::json!({
        "verdict": "pass",
        "comments": "historical comments",
        "summary": "historical",
        "findings": [{
            "severity": "optional",
            "message": "optional\n影响：legacy impact",
            "impact": "legacy impact"
        }]
    }))
    .expect("idempotent legacy replay");
    assert_eq!(
        already_appended.findings[0].message,
        "optional\n影响：legacy impact"
    );
}

#[test]
fn historical_review_parser_normalizes_legacy_payload_before_strict_live_validation() {
    let verdict = parse_historical_review_json(
        r#"{"verdict":"pass","summary":"historical","findings":[{"severity":"strong_recommend_fix","message":"repair","impact":"legacy impact"}]}"#,
        "historical comments",
    )
    .expect("historical review");

    assert_eq!(verdict.findings[0].severity, ReviewFindingSeverity::MustFix);
    assert_eq!(verdict.findings[0].message, "repair\n影响：legacy impact");
    assert!(
        parse_historical_review_json(
            r#"{"verdict":"pass","findings":[{"severity":"unexpected","message":"bad"}]}"#,
            "historical comments",
        )
        .is_none(),
        "unknown historical severity must fail rather than silently downgrade"
    );
}

#[test]
fn historical_review_deserialization_rejects_unknown_severity() {
    let value = serde_json::json!({
        "verdict": "pass",
        "comments": "historical comments",
        "summary": "historical",
        "findings": [{ "severity": "unknown_legacy", "message": "bad" }]
    });
    let error = deserialize_historical_review_verdict(value).expect_err("unknown severity");
    assert!(error.contains("unknown review finding severity"));
}
