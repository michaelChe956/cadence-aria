use super::*;
use crate::cross_cutting::structured_output::{
    StructuredOutputError, StructuredOutputErrorCode, StructuredOutputState,
};

#[test]
fn code_review_outcome_uses_parsed_sentinel_payload() {
    let outcome = ProviderStreamOutcome {
        full_output: "工作流路由回执".to_string(),
        structured_output: StructuredOutputState::Parsed(serde_json::json!({
            "verdict": "request_changes",
            "summary": "需要补充边界测试",
            "findings": [{
                "severity": "warning",
                "file_path": "src/lib.rs",
                "line": 42,
                "message": "缺少边界测试",
                "required_action": "补充边界测试",
                "source_stage": "code_review"
            }]
        })),
    };

    let parsed = parse_code_review_outcome(&outcome);

    assert_eq!(parsed.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(parsed.findings[0].line, Some(42));
}

#[test]
fn code_review_outcome_recovers_valid_payload_from_failed_sentinel() {
    let outcome = ProviderStreamOutcome {
        full_output: "不完整的 sentinel".to_string(),
        structured_output: StructuredOutputState::Failed(StructuredOutputError {
            code: StructuredOutputErrorCode::MissingEndTag,
            message: "end tag missing".to_string(),
            expected_nonce: Some("nonce".to_string()),
            observed_nonce: None,
            recoverable_value: Some(serde_json::json!({
                "verdict": "approve",
                "summary": "恢复成功",
                "findings": []
            })),
        }),
    };

    let parsed = parse_code_review_outcome(&outcome);

    assert_eq!(parsed.verdict, ReviewVerdict::Approve);
    assert_eq!(parsed.summary, "恢复成功");
}

#[test]
fn code_review_outcome_fails_closed_for_prose_without_recoverable_value() {
    let outcome = ProviderStreamOutcome {
        full_output: "审查已完成，未发现问题。".to_string(),
        structured_output: StructuredOutputState::Failed(StructuredOutputError {
            code: StructuredOutputErrorCode::MissingStartTag,
            message: "start tag missing".to_string(),
            expected_nonce: Some("nonce".to_string()),
            observed_nonce: None,
            recoverable_value: None,
        }),
    };

    let parsed = parse_code_review_outcome(&outcome);

    assert_eq!(parsed.verdict, ReviewVerdict::Blocked);
    assert!(parsed.summary.contains("review 输出不是有效 JSON"));
}

#[test]
fn group_review_parser_returns_error_for_malformed_output_instead_of_blocked_payload() {
    let result = parse_group_review_payload(
        "provider output is not json",
        CodingExecutionStage::InternalPrReview,
    );
    assert!(result.is_err());
}

#[test]
fn group_review_parser_accepts_fenced_json() {
    let result = parse_group_review_payload(
        "```json {\"verdict\":\"approve\",\"findings\":[]} ```",
        CodingExecutionStage::InternalPrReview,
    )
    .expect("valid fenced payload");
    assert_eq!(result.verdict, ReviewVerdict::Approve);
}

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
fn review_parser_rejects_removed_testing_source_stage() {
    let parsed = parse_review_payload(
        r#"{"verdict":"blocked","findings":[{"severity":"error","message":"stale stage","source_stage":"testing"}]}"#,
        CodingExecutionStage::CodeReview,
    );

    assert_eq!(parsed.verdict, ReviewVerdict::Blocked);
    assert!(parsed.summary.contains("review JSON Schema 校验失败"));
    assert!(parsed.summary.contains("unknown variant"));
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
fn review_parser_normalizes_incident_verification_finding_without_coder_route() {
    let payload = serde_json::json!({
        "verdict": "request_changes",
        "summary": "verification evidence is missing",
        "findings": [{
            "severity": "error",
            "message": "verification evidence is missing",
            "defect_class": "missing_verification_evidence",
            "reason_code": "verification_evidence_incomplete",
            "repair_target": "VerificationRetry",
            "recommended_route": "VerificationRetry",
            "confidence": "high",
            "evidence": ["cargo test --locked was not recorded"]
        }]
    });

    let parsed =
        parse_group_review_payload(&payload.to_string(), CodingExecutionStage::InternalPrReview)
            .expect("incident aliases must normalize");
    let finding = &parsed.findings[0];

    assert_eq!(
        finding.defect_class,
        crate::product::models::PlanDefectClass::VerificationIncomplete
    );
    assert_eq!(finding.repair_target, None);
    assert_eq!(
        finding.recommended_route,
        crate::product::models::PlanDefectRoute::VerificationRetry
    );
    assert_eq!(
        finding.confidence,
        Some(crate::product::plan_repair::PlanDefectConfidence::High)
    );
    assert_ne!(
        finding.recommended_route,
        crate::product::models::PlanDefectRoute::CoderRework
    );
}

#[test]
fn review_parser_rejects_unknown_defect_class_after_compatibility_normalization() {
    let payload = serde_json::json!({
        "verdict": "request_changes",
        "findings": [{
            "message": "unknown class",
            "defect_class": "unrecognized_future_class"
        }]
    });

    assert!(
        parse_group_review_payload(&payload.to_string(), CodingExecutionStage::InternalPrReview,)
            .is_err()
    );
}

#[test]
fn review_parser_rejects_unknown_recommended_route_after_compatibility_normalization() {
    let payload = serde_json::json!({
        "verdict": "request_changes",
        "findings": [{
            "message": "unknown route",
            "recommended_route": "VerifyRetry"
        }]
    });

    assert!(
        parse_group_review_payload(&payload.to_string(), CodingExecutionStage::InternalPrReview,)
            .is_err()
    );
}

#[test]
fn review_parser_rejects_non_object_non_whitelisted_repair_targets() {
    for repair_target in [serde_json::json!(42), serde_json::json!(true)] {
        let payload = serde_json::json!({
            "verdict": "request_changes",
            "findings": [{
                "message": "invalid repair target",
                "repair_target": repair_target
            }]
        });

        assert!(
            parse_group_review_payload(
                &payload.to_string(),
                CodingExecutionStage::InternalPrReview,
            )
            .is_err()
        );
    }
}

#[test]
fn review_parser_accepts_only_canonical_confidence_values() {
    for (value, expected) in [
        (
            "high",
            crate::product::plan_repair::PlanDefectConfidence::High,
        ),
        (
            "medium",
            crate::product::plan_repair::PlanDefectConfidence::Medium,
        ),
        (
            "low",
            crate::product::plan_repair::PlanDefectConfidence::Low,
        ),
    ] {
        let payload = serde_json::json!({
            "verdict": "request_changes",
            "findings": [{"message": "confidence", "confidence": value}]
        });
        let parsed = parse_group_review_payload(
            &payload.to_string(),
            CodingExecutionStage::InternalPrReview,
        )
        .expect("canonical confidence must parse");
        assert_eq!(parsed.findings[0].confidence, Some(expected));
    }

    let payload = serde_json::json!({
        "verdict": "request_changes",
        "findings": [{"message": "confidence", "confidence": "certain"}]
    });
    assert!(
        parse_group_review_payload(&payload.to_string(), CodingExecutionStage::InternalPrReview,)
            .is_err()
    );
}

#[test]
fn review_parser_preserves_canonical_plan_defect_finding_without_compatibility_path() {
    let payload = serde_json::json!({
        "verdict": "request_changes",
        "findings": [{
            "message": "canonical verification finding",
            "defect_class": "verification_incomplete",
            "repair_target": null,
            "recommended_route": "verification_retry",
            "confidence": "high"
        }]
    });

    let parsed =
        parse_group_review_payload(&payload.to_string(), CodingExecutionStage::InternalPrReview)
            .expect("canonical finding must parse");
    let finding = &parsed.findings[0];
    assert_eq!(
        finding.defect_class,
        crate::product::models::PlanDefectClass::VerificationIncomplete
    );
    assert_eq!(finding.repair_target, None);
    assert_eq!(
        finding.recommended_route,
        crate::product::models::PlanDefectRoute::VerificationRetry
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

/// Coder 正文常出现 JS 解构、Rust 块等带花括号的片段（例如
/// `import { formatCompactDuration } from '../src/x.js'`）。这些片段不是结构化
/// 结论，不能因为它们无法反序列化就把整次执行判为契约违规并转人工分诊。
#[test]
fn coding_plan_repair_parser_ignores_prose_braces_without_plan_defect_findings() {
    let output = "已完成实现。\n\
        - **downstream_import_specifier**：从 `demo/compact-duration.js` 导入写作 \
        `import { formatCompactDuration } from '../src/format-compact-duration.js';`\n\
        - **package_manifest_module_semantics**：仓库根 `package.json` 声明 \
        `\"type\": \"module\"`\n";

    let parsed = parse_execution_plan_defects(PlanDefectSource::Coder, output)
        .expect("正文花括号不构成 plan defect 契约违规");

    assert_eq!(parsed.source, PlanDefectSource::Coder);
    assert!(parsed.findings.is_empty());
}

/// 真实结论位于输出末尾，前面的散文片段不得遮蔽它。
#[test]
fn coding_plan_repair_parser_selects_plan_defect_findings_after_prose_braces() {
    let findings = serde_json::json!({
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
    let output = format!(
        "示例写法 `import {{ formatCompactDuration }} from '../src/x.js';`\n\n{findings}\n"
    );

    let parsed = parse_execution_plan_defects(PlanDefectSource::Coder, &output)
        .expect("散文片段之后的结论必须被识别");

    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].defect_class,
        crate::product::models::PlanDefectClass::UpstreamContractInvalid
    );
}

/// `plan_defect_findings` 带 `#[serde(default)]`，任何合法 JSON 对象都能反序列化成
/// 空 findings。因此不能只看「能否反序列化」，否则无关 JSON 片段会静默吞掉真实结论。
/// 契约要求结论位于输出末尾（同 `review_parser`）。Provider 可能先复述一个空的
/// `plan_defect_findings` 示例再给出真实结论；取首个声明候选会静默吞掉真实结论。
#[test]
fn coding_plan_repair_parser_prefers_the_last_declared_findings_candidate() {
    let findings = serde_json::json!({
        "plan_defect_findings": [{
            "severity": "error",
            "defect_class": "current_work_item_invalid",
            "reason_code": "current_work_item_contract_invalid",
            "message": "the current work item contract is not implementable",
            "contract_refs": ["contract.current"],
            "capability_refs": ["implementability"],
            "repair_target": {
                "kind": "current_work_item",
                "logical_work_item_ids": ["work_item_0001"],
                "work_item_revision_ids": ["work_item_revision_0001"]
            },
            "recommended_route": "plan_repair",
            "confidence": "high",
            "evidence": []
        }]
    })
    .to_string();
    let output = format!(
        "无缺陷时的输出形如 {{\"plan_defect_findings\": []}}，本次结论如下：\n\n{findings}\n"
    );

    let parsed =
        parse_execution_plan_defects(PlanDefectSource::Coder, &output).expect("末尾结论必须被识别");

    assert_eq!(
        parsed.findings.len(),
        1,
        "在前的空示例不得遮蔽末尾的真实结论"
    );
    assert_eq!(
        parsed.findings[0].defect_class,
        crate::product::models::PlanDefectClass::CurrentWorkItemInvalid
    );
}

#[test]
fn coding_plan_repair_parser_skips_unrelated_json_objects_before_findings() {
    let findings = serde_json::json!({
        "plan_defect_findings": [{
            "severity": "error",
            "defect_class": "current_work_item_invalid",
            "reason_code": "current_work_item_contract_invalid",
            "message": "the current work item contract is not implementable",
            "contract_refs": ["contract.current"],
            "capability_refs": ["implementability"],
            "repair_target": {
                "kind": "current_work_item",
                "logical_work_item_ids": ["work_item_0001"],
                "work_item_revision_ids": ["work_item_revision_0001"]
            },
            "recommended_route": "plan_repair",
            "confidence": "high",
            "evidence": []
        }]
    })
    .to_string();
    let output = format!("仓库根 package.json：{{\"type\": \"module\"}}\n\n{findings}\n");

    let parsed = parse_execution_plan_defects(PlanDefectSource::Coder, &output)
        .expect("无关 JSON 片段不得遮蔽真实结论");

    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].defect_class,
        crate::product::models::PlanDefectClass::CurrentWorkItemInvalid
    );
}

#[test]
fn coding_plan_repair_coder_plan_defects_use_the_canonical_finding_schema() {
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

    let parsed = parse_execution_plan_defects(PlanDefectSource::Coder, &output).unwrap();

    assert_eq!(parsed.source, PlanDefectSource::Coder);
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
