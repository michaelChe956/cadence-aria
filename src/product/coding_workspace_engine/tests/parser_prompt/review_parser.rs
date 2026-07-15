use super::*;

#[test]
fn review_parser_preserves_findings_with_common_aliases() {
    let payload = r#"{
      "verdict": "request_changes",
      "summary": "needs changes",
      "findings": [
        {
          "file": "src/lib.rs",
          "line": 42,
          "description": "missing validation",
          "recommendation": "add validation"
        }
      ]
    }"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].severity,
        crate::product::coding_models::FindingSeverity::Warning
    );
    assert_eq!(
        parsed.findings[0].source_stage,
        CodingExecutionStage::CodeReview
    );
    assert_eq!(parsed.findings[0].file_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(parsed.findings[0].message, "missing validation");
    assert_eq!(
        parsed.findings[0].required_action.as_deref(),
        Some("add validation")
    );
}

#[test]
fn review_parser_accepts_blocked_finding_severity_as_error() {
    let payload = r#"{
      "verdict": "blocked",
      "summary": "dependency handoff blocker",
      "findings": [
        {
          "severity": "blocked",
          "file_path": "src/web/runtime/provider.rs",
          "line": 109,
          "message": "shared gate is not wired",
          "required_action": "inject the shared gate",
          "source_stage": "code_review"
        }
      ]
    }"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::Blocked);
    assert_eq!(parsed.summary, "dependency handoff blocker");
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].severity,
        crate::product::coding_models::FindingSeverity::Error
    );
    assert_eq!(parsed.findings[0].message, "shared gate is not wired");
}

#[test]
fn review_parser_distinguishes_schema_error_from_json_syntax_error() {
    let schema_error = parse_review_payload(
        r#"{"verdict":"blocked","findings":[{"severity":"unexpected"}]}"#,
        CodingExecutionStage::CodeReview,
    );
    assert!(schema_error.summary.contains("review JSON Schema 校验失败"));
    assert!(schema_error.summary.contains("unknown variant"));

    let syntax_error = parse_review_payload(
        r#"{"verdict":"blocked","findings":["#,
        CodingExecutionStage::CodeReview,
    );
    assert!(syntax_error.summary.contains("review 输出不是有效 JSON"));
    assert!(!syntax_error.summary.contains("Schema 校验失败"));
}

#[test]
fn review_parser_accepts_fenced_json_with_reviewer_blocker_severity() {
    let payload = r#"Reviewer summary before the structured payload.

```json
{
  "verdict": "request_changes",
  "summary": "orphaned modules",
  "findings": [
    {
      "severity": "blocker",
      "file_path": "src/web/mod.rs",
      "message": "module is not wired",
      "required_action": "declare the module"
    }
  ]
}
```"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(parsed.summary, "orphaned modules");
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].severity,
        crate::product::coding_models::FindingSeverity::Error
    );
    assert_eq!(
        parsed.findings[0].required_action.as_deref(),
        Some("declare the module")
    );
}

#[test]
fn review_parser_accepts_group_final_review_source_stage_alias() {
    let payload = r#"{
      "verdict": "request_changes",
      "summary": "group final review found one issue",
      "findings": [
        {
          "severity": "error",
          "file_path": "src/group.rs",
          "message": "handoff contract is not closed",
          "source_stage": "group_final_review"
        }
      ]
    }"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::InternalPrReview);

    assert_eq!(parsed.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].source_stage,
        CodingExecutionStage::InternalPrReview
    );
}
