use super::*;

/// REQ-WSC-11：owner 可选项的 fail-closed 下界——「唯一选项」不构成互斥
/// 选择，owner 仍无法据以作答。上限不设（模型给 4 个选项不是缺陷）。
pub(crate) const MIN_OWNER_OPTIONS: usize = 2;

/// REQ-WSC-11：`findings[].owner_question` —— 面向 owner 的一句话人话提问。
/// 语义空（全空白）视为未提供，交由 `owner_guidance_error` 在
/// `needs_human` 上判不合规；非 `needs_human` 结论不要求。
pub(crate) fn parse_optional_owner_question(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
) -> Result<Option<String>, ReviewStructuredOutputError> {
    match object.get("owner_question") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.trim().to_string())),
        Some(_) => Err(invalid_finding_field(
            format!("{prefix}.owner_question"),
            "must be a string or null",
        )),
    }
}

/// REQ-WSC-11：`findings[].owner_options` —— 面向 owner 的互斥处置项。
/// 空白数组视为未提供；每项 `label`/`consequence` 必须 trim 后非空，缺一即
/// `invalid_finding_field`（不是"少一个选项"，是结构不合法）。
pub(crate) fn parse_optional_owner_options(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
) -> Result<Option<Vec<crate::web::workspace_ws_types::OwnerOption>>, ReviewStructuredOutputError> {
    let Some(value) = object.get("owner_options") else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Array(items) if items.is_empty() => Ok(None),
        serde_json::Value::Array(items) => {
            let mut options = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let item_prefix = format!("{prefix}.owner_options[{index}]");
                let Some(option) = item.as_object() else {
                    return Err(invalid_finding_field(item_prefix, "must be an object"));
                };
                for field in option.keys() {
                    if !matches!(field.as_str(), "label" | "consequence") {
                        return Err(invalid_finding_field(
                            format!("{item_prefix}.{field}"),
                            "is not allowed",
                        ));
                    }
                }
                let label = option
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|label| !label.is_empty())
                    .ok_or_else(|| {
                        invalid_finding_field(
                            format!("{item_prefix}.label"),
                            "must be a non-empty string",
                        )
                    })?;
                let consequence = option
                    .get("consequence")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|consequence| !consequence.is_empty())
                    .ok_or_else(|| {
                        invalid_finding_field(
                            format!("{item_prefix}.consequence"),
                            "must be a non-empty string",
                        )
                    })?;
                options.push(crate::web::workspace_ws_types::OwnerOption {
                    label: label.to_string(),
                    consequence: consequence.to_string(),
                });
            }
            Ok(Some(options))
        }
        _ => Err(invalid_finding_field(
            format!("{prefix}.owner_options"),
            "must be an array or null",
        )),
    }
}

/// REQ-WSC-11 fail-closed：`needs_human` 结论必须至少有一条 finding 同时
/// 携带非空 `owner_question` 与非空 `owner_options`；否则判
/// `missing_owner_guidance`（调用侧经 `fallback_review_verdict` 收敛为
/// `needs_human` + 既有 `structured_output_diagnostic`）。
pub(crate) fn owner_guidance_error(
    requires_owner_decision: bool,
    findings: &[ReviewFinding],
) -> Option<ReviewStructuredOutputErrorCode> {
    if !requires_owner_decision {
        return None;
    }
    let complete = findings.iter().any(|finding| {
        finding
            .owner_question
            .as_deref()
            .is_some_and(|question| !question.trim().is_empty())
            && finding
                .owner_options
                .as_ref()
                .is_some_and(|options| options.len() >= MIN_OWNER_OPTIONS)
    });
    (!complete).then_some(ReviewStructuredOutputErrorCode::MissingOwnerGuidance)
}

/// REQ-WSC-11：两项字段必须成对出现——只有提问没有选项（或反之）是结构
/// 不合法，与 verdict 无关，避免半截呈现漏到 owner。
pub(crate) fn validate_owner_guidance_pairing(
    findings: &[ReviewFinding],
) -> Result<(), ReviewStructuredOutputErrorCode> {
    let unpaired = findings
        .iter()
        .any(|finding| finding.owner_question.is_some() != finding.owner_options.is_some());
    if unpaired {
        return Err(ReviewStructuredOutputErrorCode::InvalidFindingField);
    }
    Ok(())
}

/// REQ-WSC-11（owner 可读的人工裁决）：`needs_human` 结论必须带
/// `owner_question`（一句话人话提问）与 `owner_options`（2–3 个互斥选项，
/// 每项含 `label`/`consequence`）；缺失即判结构化输出不合规，由既有
/// `structured_output_diagnostic` 通道收口（`missing_owner_guidance`）。
/// `pass`/`revise` 不要求这两个字段，行为逐字不变。
#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::web::workspace_ws_types::OwnerOption;

    fn complete_owner_finding() -> serde_json::Value {
        serde_json::json!({
            "severity": "suggestion",
            "message": "WI-004 的写入范围包含仓库根目录的 scripts/dev.sh",
            "evidence": "exclusive_write_scopes=[scripts/dev.sh]",
            "required_action": "由人工裁决 WI-004 归属",
            "owner_question": "WI-004 要不要继续负责 scripts/dev.sh 的改动？",
            "owner_options": [
                {"label": "不负责，只做验证", "consequence": "WI-004 变成纯验证项，脚本改动另行安排"},
                {"label": "继续负责", "consequence": "把 scripts/ 纳入可写入范围，后续改动都记在 WI-004 上"}
            ]
        })
    }

    fn generic_review(verdict: &str, findings: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "verdict": verdict,
            "summary": "一句话摘要",
            "findings": findings,
        })
    }

    fn plan_review(verdict: &str, scope: &str, findings: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "verdict": verdict,
            "review_scope": scope,
            "generation_round_id": "round-1",
            "summary": "一句话摘要",
            "findings": findings,
        })
    }

    fn block_finding(message: &str) -> serde_json::Value {
        serde_json::json!({
            "severity": "must_fix",
            "message": message,
            "evidence": "evidence",
            "required_action": "repair",
        })
    }

    #[test]
    fn needs_human_without_owner_guidance_is_rejected() {
        let error = parse_review_value(
            &generic_review(
                "needs_human",
                serde_json::json!([block_finding("需要你决定")]),
            ),
            "",
        )
        .unwrap_err();

        assert_eq!(error, ReviewStructuredOutputErrorCode::MissingOwnerGuidance);
        assert_eq!(error.as_str(), "missing_owner_guidance");
    }

    #[test]
    fn needs_human_without_any_finding_is_rejected() {
        let error = parse_review_value(&generic_review("needs_human", serde_json::json!([])), "")
            .unwrap_err();

        assert_eq!(error, ReviewStructuredOutputErrorCode::MissingOwnerGuidance);
    }

    #[test]
    fn needs_human_with_owner_question_and_options_parses() {
        let verdict = parse_review_value(
            &generic_review("needs_human", serde_json::json!([complete_owner_finding()])),
            "",
        )
        .expect("complete owner guidance must parse");

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        let finding = &verdict.findings[0];
        assert_eq!(
            finding.owner_question.as_deref(),
            Some("WI-004 要不要继续负责 scripts/dev.sh 的改动？")
        );
        assert_eq!(
            finding.owner_options,
            Some(vec![
                OwnerOption {
                    label: "不负责，只做验证".to_string(),
                    consequence: "WI-004 变成纯验证项，脚本改动另行安排".to_string(),
                },
                OwnerOption {
                    label: "继续负责".to_string(),
                    consequence: "把 scripts/ 纳入可写入范围，后续改动都记在 WI-004 上".to_string(),
                },
            ])
        );
    }

    #[test]
    fn revise_without_findings_keeps_existing_human_triage_without_owner_guidance() {
        // 派生分诊（模型自称 revise 却不给 finding → gate 升级为人工分诊）是既有
        // 行为：结论保留 reviewer 的 summary/comments，不被强制人话字段拦下。
        let verdict = parse_review_value(&generic_review("revise", serde_json::json!([])), "")
            .expect("derived triage must not be gated by owner guidance");

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        assert_eq!(verdict.summary, "一句话摘要");
        assert!(verdict.findings.is_empty());
    }

    #[test]
    fn pass_and_revise_do_not_require_owner_guidance() {
        let pass = parse_review_value(&generic_review("pass", serde_json::json!([])), "")
            .expect("pass without owner guidance stays valid");
        assert_eq!(pass.verdict, ReviewVerdictType::Pass);
        assert!(pass.findings.is_empty());

        let revise = parse_review_value(
            &generic_review("revise", serde_json::json!([block_finding("缺验证命令")])),
            "",
        )
        .expect("revise without owner guidance stays valid");
        assert_eq!(revise.verdict, ReviewVerdictType::Revise);
        assert_eq!(revise.findings[0].owner_question, None);
        assert_eq!(revise.findings[0].owner_options, None);
    }

    #[test]
    fn needs_human_with_blank_question_is_rejected() {
        let mut value = complete_owner_finding();
        value["owner_question"] = serde_json::json!("   ");

        let error = parse_review_value(
            &generic_review("needs_human", serde_json::json!([value])),
            "",
        )
        .unwrap_err();

        assert_eq!(error, ReviewStructuredOutputErrorCode::MissingOwnerGuidance);
    }

    #[test]
    fn needs_human_with_empty_options_is_rejected() {
        let mut value = complete_owner_finding();
        value["owner_options"] = serde_json::json!([]);

        let error = parse_review_value(
            &generic_review("needs_human", serde_json::json!([value])),
            "",
        )
        .unwrap_err();

        assert_eq!(error, ReviewStructuredOutputErrorCode::MissingOwnerGuidance);
    }

    #[test]
    fn needs_human_with_single_option_is_rejected() {
        // F1：唯一选项不构成互斥选择——owner 仍然无从作答，按 fail-closed 判不合规。
        let mut value = complete_owner_finding();
        value["owner_options"] =
            serde_json::json!([{"label": "继续负责", "consequence": "把 scripts/ 纳入可写入范围"}]);

        let error = parse_review_value(
            &generic_review("needs_human", serde_json::json!([value])),
            "",
        )
        .unwrap_err();
        assert_eq!(error, ReviewStructuredOutputErrorCode::MissingOwnerGuidance);

        // 三条仍合规（上限不设）：模型多给不是缺陷。
        let mut three = complete_owner_finding();
        three["owner_options"] = serde_json::json!([
            {"label": "a", "consequence": "ca"},
            {"label": "b", "consequence": "cb"},
            {"label": "c", "consequence": "cc"}
        ]);
        assert!(
            parse_review_value(
                &generic_review("needs_human", serde_json::json!([three])),
                ""
            )
            .is_ok()
        );
    }

    #[test]
    fn historical_review_reparse_does_not_require_owner_guidance() {
        // F3：本 change 之前的存量 reviewer 消息不可能带人话字段，历史重解析
        // 必须保留既有 findings/summary（否则恢复链丢掉待关闭 finding）。
        let json = serde_json::json!({
            "verdict": "needs_human",
            "summary": "存量结论",
            // severity 用 suggestion：must_fix 会按既有 gate 优先级归一成 revise，
            // 那与 F3 要验证的 needs_human 历史重解析无关。
            "findings": [{
                "severity": "suggestion",
                "message": "存量 needs_human finding",
                "evidence": "evidence",
                "required_action": "decide"
            }]
        })
        .to_string();
        let verdict = parse_historical_review_json(&json, "存量 comments")
            .expect("historical needs_human must still parse");
        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.summary, "存量结论");
        assert_eq!(verdict.findings.len(), 1);
        assert!(verdict.findings[0].owner_question.is_none());

        // live 入口对同一 payload 仍 fail-closed。
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parse_review_value(&value, "").unwrap_err(),
            ReviewStructuredOutputErrorCode::MissingOwnerGuidance
        );
    }

    #[test]
    fn owner_option_requires_label_and_consequence() {
        let mut value = complete_owner_finding();
        value["owner_options"] = serde_json::json!([{"label": "继续负责", "consequence": " "}]);

        let error = parse_review_value(
            &generic_review("needs_human", serde_json::json!([value])),
            "",
        )
        .unwrap_err();

        assert_eq!(error, ReviewStructuredOutputErrorCode::InvalidFindingField);
    }

    #[test]
    fn owner_guidance_fields_must_co_occur() {
        let mut value = complete_owner_finding();
        value.as_object_mut().unwrap().remove("owner_options");

        let error = parse_review_value(
            &generic_review("pass", serde_json::json!([block_finding("x"), value])),
            "",
        )
        .unwrap_err();

        assert_eq!(error, ReviewStructuredOutputErrorCode::InvalidFindingField);
    }

    #[test]
    fn work_item_plan_needs_human_requires_owner_guidance() {
        let error = parse_work_item_plan_review_value(
            &plan_review(
                "needs_human",
                "outline",
                serde_json::json!([block_finding("需要你决定")]),
            ),
            "",
            &[],
            WorkItemPlanReviewScope::Outline,
        )
        .unwrap_err();

        assert_eq!(error, ReviewStructuredOutputErrorCode::MissingOwnerGuidance);
    }

    #[test]
    fn work_item_plan_needs_human_with_owner_guidance_parses() {
        let verdict = parse_work_item_plan_review_value(
            &plan_review(
                "needs_human",
                "outline",
                serde_json::json!([complete_owner_finding()]),
            ),
            "",
            &[],
            WorkItemPlanReviewScope::Outline,
        )
        .expect("outline needs_human with owner guidance must parse");

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert!(verdict.findings[0].owner_question.is_some());
        assert_eq!(
            verdict.findings[0]
                .owner_options
                .as_ref()
                .map(|options| options.len()),
            Some(2)
        );
    }

    #[test]
    fn work_item_plan_automatic_revision_verdicts_do_not_require_owner_guidance() {
        let pass = parse_work_item_plan_review_value(
            &plan_review("pass", "outline", serde_json::json!([])),
            "",
            &[],
            WorkItemPlanReviewScope::Outline,
        )
        .expect("outline pass stays valid");
        assert_eq!(pass.verdict, ReviewVerdictType::Pass);

        let revise = parse_work_item_plan_review_value(
            &plan_review(
                "revise",
                "item",
                serde_json::json!([block_finding("缺验证命令")]),
            ),
            "",
            &[],
            WorkItemPlanReviewScope::Item,
        )
        .expect("item revise stays valid");
        assert_eq!(revise.verdict, ReviewVerdictType::Revise);

        let revise_batch = parse_work_item_plan_review_value(
            &plan_review("revise_batch", "batch", serde_json::json!([])),
            "",
            &[],
            WorkItemPlanReviewScope::Batch,
        )
        .expect("batch revise stays valid without owner guidance");
        assert_eq!(revise_batch.verdict, ReviewVerdictType::NeedsHuman);

        let reopen = parse_work_item_plan_review_value(
            &plan_review("plan_reopen_required", "item", serde_json::json!([])),
            "",
            &[],
            WorkItemPlanReviewScope::Item,
        )
        .expect("plan reopen stays valid without owner guidance");
        assert_eq!(reopen.verdict, ReviewVerdictType::NeedsHuman);
    }

    #[test]
    fn missing_owner_guidance_is_reported_through_structured_output_diagnostic() {
        let value = generic_review("needs_human", serde_json::json!([]));
        let completion = ProviderCompletion {
            full_output: format!(
                "<ARIA_STRUCTURED_OUTPUT nonce=\"nonce\">\n{value}\n</ARIA_STRUCTURED_OUTPUT>"
            ),
            readable_output: String::new(),
            structured_output: StructuredOutputState::Parsed(value),
            provider_session_id: None,
        };
        let error = parse_review_value(&generic_review("needs_human", serde_json::json!([])), "")
            .unwrap_err();
        let verdict =
            fallback_review_verdict(&completion, &ReviewCompletionError::Schema(error), false);

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        let diagnostic = verdict
            .structured_output_diagnostic
            .expect("existing diagnostic channel must carry the code");
        assert_eq!(diagnostic.code, "missing_owner_guidance");
        assert!(!diagnostic.repair_succeeded);
    }
}
