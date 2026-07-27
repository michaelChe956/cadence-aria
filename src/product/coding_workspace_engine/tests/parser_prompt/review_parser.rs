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
fn review_parser_skips_unrelated_json_fragments_before_final_verdict() {
    let payload = "全部验证证据已收集完毕。以下为代码审查结果。\n\
                   \n\
                   | `check_scope_and_dependency_boundary` | ✅ 通过 | `package.json` 仅 `{\"type\": \"module\"}`，无依赖 |\n\
                   \n\
                   ```json\n\
                   {\n\
                     \"verdict\": \"approve\",\n\
                     \"summary\": \"review complete\",\n\
                     \"findings\": []\n\
                   }\n\
                   ```";

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::Approve);
    assert_eq!(parsed.summary, "review complete");
    assert!(parsed.findings.is_empty());
}

#[test]
fn review_parser_accepts_plain_text_routing_receipt_before_final_json() {
    let payload = "工作流路由：阶段=只读代码审查；Change=无；Plan=work_item_0001；必调 Skill=using-superpowers → requesting-code-review。\n{\"verdict\":\"approve\",\"summary\":\"review complete\",\"findings\":[]}";

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::Approve);
    assert_eq!(parsed.summary, "review complete");
    assert!(parsed.findings.is_empty());
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

#[test]
fn coding_plan_repair_parser_preserves_upstream_contract_invalid() {
    let payload = serde_json::json!({
        "verdict": "blocked",
        "summary": "upstream contract invalid",
        "findings": [{
            "severity": "error",
            "defect_class": "upstream_contract_invalid",
            "reason_code": "upstream_contract_capability_missing",
            "message": "missing finalization_failure",
            "contract_refs": ["repository_initialization_finalization"],
            "capability_refs": ["finalization_failure"],
            "repair_target": {
                "kind": "upstream_work_item",
                "logical_work_item_ids": ["wi_core"],
                "work_item_revision_ids": ["work_item_revision_0001"]
            },
            "recommended_route": "plan_repair",
            "confidence": "high",
            "evidence": []
        }]
    });

    let parsed = parse_review_payload(&payload.to_string(), CodingExecutionStage::CodeReview);

    assert_eq!(
        parsed.findings[0].defect_class,
        crate::product::models::PlanDefectClass::UpstreamContractInvalid
    );
    assert_eq!(
        parsed.findings[0].confidence,
        Some(crate::product::plan_repair::PlanDefectConfidence::High)
    );
    assert_eq!(
        parsed.findings[0]
            .repair_target
            .as_ref()
            .expect("repair target")
            .kind,
        crate::product::models::RepairTargetKind::UpstreamWorkItem
    );
}

#[test]
fn coding_plan_repair_parser_maps_legacy_review_finding_only_to_implementation() {
    let parsed = parse_review_payload(
        r#"{
          "verdict": "request_changes",
          "findings": [{"message": "missing validation"}]
        }"#,
        CodingExecutionStage::CodeReview,
    );
    let finding = &parsed.findings[0];

    assert_eq!(
        finding.defect_class,
        crate::product::models::PlanDefectClass::ImplementationDefect
    );
    assert_eq!(
        finding.recommended_route,
        crate::product::models::PlanDefectRoute::CoderRework
    );
    assert!(finding.reason_code.is_none());
    assert!(finding.contract_refs.is_empty());
    assert!(finding.capability_refs.is_empty());
    assert!(finding.repair_target.is_none());
    assert!(finding.confidence.is_none());
}

#[test]
fn coding_plan_repair_coding_and_tester_plan_defects_use_the_same_finding_schema() {
    let output = serde_json::json!({
        "plan_defect_findings": [{
            "severity": "error",
            "defect_class": "upstream_contract_invalid",
            "reason_code": "upstream_contract_capability_missing",
            "message": "missing finalization_failure",
            "contract_refs": ["repository_initialization_finalization"],
            "capability_refs": ["finalization_failure"],
            "repair_target": {
                "kind": "upstream_work_item",
                "logical_work_item_ids": ["wi_core"],
                "work_item_revision_ids": ["work_item_revision_0001"]
            },
            "recommended_route": "plan_repair",
            "confidence": "high",
            "evidence": []
        }]
    })
    .to_string();

    for source in [PlanDefectSource::Coder, PlanDefectSource::Tester] {
        let parsed = parse_execution_plan_defects(source.clone(), &output).unwrap();

        assert_eq!(parsed.source, source);
        assert_eq!(
            parsed.findings[0].defect_class,
            crate::product::models::PlanDefectClass::UpstreamContractInvalid
        );
        assert_eq!(
            parsed.findings[0]
                .repair_target
                .as_ref()
                .expect("repair target")
                .kind,
            crate::product::models::RepairTargetKind::UpstreamWorkItem
        );
    }
}

#[test]
fn coding_plan_repair_tester_execution_parser_preserves_canonical_finding() {
    let output = serde_json::json!({
        "step_results": [{
            "step_id": "unit",
            "status": "blocked",
            "evidence_refs": ["unit.log"],
            "provider_analysis": "test_plan_insufficient: contract is invalid"
        }],
        "plan_defect_findings": [{
            "finding_id": "tester_finding_0001",
            "severity": "error",
            "defect_class": "current_work_item_invalid",
            "reason_code": "current_work_item_contract_invalid",
            "message": "the current work item contract is not testable",
            "evidence": [{
                "kind": "test_execution",
                "source_ref": "unit.log",
                "message": "the required contract cannot be exercised"
            }],
            "contract_refs": ["contract.current"],
            "capability_refs": ["testability"],
            "repair_target": {
                "kind": "current_work_item",
                "logical_work_item_ids": ["work_item_0001"],
                "work_item_revision_ids": ["work_item_revision_0001"]
            },
            "recommended_route": "plan_repair",
            "confidence": "high"
        }]
    })
    .to_string();

    let payload = parse_test_execution_payload_from_provider_output(&output).unwrap();
    let finding = &payload.plan_defect_findings[0];

    assert_eq!(payload.step_results[0].step_id, "unit");
    assert_eq!(finding.finding_id, "tester_finding_0001");
    assert_eq!(
        finding.defect_class,
        crate::product::models::PlanDefectClass::CurrentWorkItemInvalid
    );
    assert_eq!(
        finding.repair_target.as_ref().expect("repair target").kind,
        crate::product::models::RepairTargetKind::CurrentWorkItem
    );
    assert_eq!(finding.evidence[0].kind, "test_execution");
    assert_eq!(finding.evidence[0].source_ref, "unit.log");
}
