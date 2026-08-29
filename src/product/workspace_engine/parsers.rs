use super::*;
use crate::cross_cutting::structured_output::parse_last_structured_output;
use crate::product::work_item_plan_policy::{ParsedReviewEnvelope, ReviewStructuredOutputError};

mod choice;
pub(crate) use choice::*;

/// Extracts the final sentinel through the cross-cutting single parser. The parser
/// authenticates the JSON envelope nonce and strips it before returning JSON to the
/// existing workspace business deserializers.
pub(crate) fn extract_structured_json(output: &str) -> Option<(String, String)> {
    parse_last_structured_output(output)
        .ok()
        .flatten()
        .map(|(comments, value)| (comments, value.to_string()))
        .or_else(|| extract_markdown_fence_json(output))
}

pub(crate) fn extract_markdown_fence_json(output: &str) -> Option<(String, String)> {
    if output.starts_with('{') && output.ends_with('}') {
        return Some((String::new(), output.to_string()));
    }

    let end = output.rfind("```")?;
    let before_end = &output[..end];
    let start = before_end.rfind("```")?;
    let comments = output[..start].to_string();
    let mut json = before_end[start + 3..].trim().to_string();
    if let Some(stripped) = json.strip_prefix("json") {
        json = stripped.trim().to_string();
    }
    Some((comments, json))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewStructuredOutputErrorCode {
    MissingVerdict,
    InvalidVerdict,
    InvalidReviewScope,
    MalformedFindings,
    InvalidOutlineReference,
    InvalidGenerationRound,
    UnknownFindingCategory,
    UnknownFindingClassHint,
    InvalidFindingField,
}

impl ReviewStructuredOutputErrorCode {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::MissingVerdict => "missing_verdict",
            Self::InvalidVerdict => "invalid_verdict",
            Self::InvalidReviewScope => "invalid_review_scope",
            Self::MalformedFindings => "malformed_findings",
            Self::InvalidOutlineReference => "invalid_outline_reference",
            Self::InvalidGenerationRound => "invalid_generation_round",
            Self::UnknownFindingCategory => "unknown_finding_category",
            Self::UnknownFindingClassHint => "unknown_class_hint",
            Self::InvalidFindingField => "invalid_finding_field",
        }
    }

    pub(crate) fn message(&self) -> &'static str {
        match self {
            Self::MissingVerdict => "审核结论缺失或不是字符串",
            Self::InvalidVerdict => "审核结论不符合当前范围的 schema",
            Self::InvalidReviewScope => "审核 review_scope 缺失、无效或与当前范围不一致",
            Self::MalformedFindings => "审核 findings 结构不合法",
            Self::InvalidOutlineReference => "审核引用了无效的 outline",
            Self::InvalidGenerationRound => "审核 generation round 缺失或为空",
            Self::UnknownFindingCategory => "审核 finding category 不在允许枚举内",
            Self::UnknownFindingClassHint => "审核 finding class_hint 不在允许枚举内",
            Self::InvalidFindingField => "审核 finding 字段结构不合法",
        }
    }
}

impl From<ReviewStructuredOutputError> for ReviewStructuredOutputErrorCode {
    fn from(error: ReviewStructuredOutputError) -> Self {
        match error {
            ReviewStructuredOutputError::UnknownFindingCategory(_) => Self::UnknownFindingCategory,
            ReviewStructuredOutputError::UnknownFindingClassHint(_) => {
                Self::UnknownFindingClassHint
            }
            ReviewStructuredOutputError::InvalidFindingField { .. }
            | ReviewStructuredOutputError::VerificationScopeViolation { .. } => {
                Self::InvalidFindingField
            }
        }
    }
}

/// 唯一的 reviewer JSON adapter：保留原始 verdict，并对 finding 的新枚举字段作显式转换。
pub(crate) fn parse_review_envelope(
    value: &serde_json::Value,
) -> Result<ParsedReviewEnvelope, ReviewStructuredOutputError> {
    let raw_verdict = parse_raw_review_verdict(value)?;
    let findings = parse_review_findings_strict(value.get("findings"))?;
    let parsed_findings = ParsedReviewFindings {
        findings: findings.clone(),
        malformed: false,
        structured_error: None,
    };

    Ok(ParsedReviewEnvelope {
        raw_verdict: raw_verdict.clone(),
        normalized_gate: review_gate_for(&raw_verdict, &parsed_findings),
        findings,
    })
}

fn parse_raw_review_verdict(
    value: &serde_json::Value,
) -> Result<ReviewVerdictType, ReviewStructuredOutputError> {
    let verdict = value
        .get("verdict")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ReviewStructuredOutputError::InvalidFindingField {
            field: "verdict".to_string(),
            details: "must be a string".to_string(),
        })?;
    match verdict {
        "pass" => Ok(ReviewVerdictType::Pass),
        "revise" => Ok(ReviewVerdictType::Revise),
        "needs_human" => Ok(ReviewVerdictType::NeedsHuman),
        _ => Err(ReviewStructuredOutputError::InvalidFindingField {
            field: "verdict".to_string(),
            details: "must be pass, revise, or needs_human".to_string(),
        }),
    }
}

pub(crate) fn parse_historical_review_json(json: &str, comments: &str) -> Option<ReviewVerdict> {
    let mut value: serde_json::Value = serde_json::from_str(json).ok()?;
    normalize_historical_review_findings(&mut value).ok()?;
    parse_review_value(&value, comments).ok()
}

pub(crate) fn deserialize_historical_review_verdict(
    mut value: serde_json::Value,
) -> Result<ReviewVerdict, String> {
    normalize_historical_review_findings(&mut value)?;
    serde_json::from_value(value).map_err(|error| error.to_string())
}

pub(crate) fn normalize_historical_review_findings(
    value: &mut serde_json::Value,
) -> Result<(), String> {
    let Some(findings) = value.get_mut("findings") else {
        return Ok(());
    };
    let Some(findings) = findings.as_array_mut() else {
        return Err("historical review findings must be an array".to_string());
    };

    for finding in findings {
        let Some(finding) = finding.as_object_mut() else {
            return Err("historical review finding must be an object".to_string());
        };
        let severity = finding
            .get("severity")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "historical review finding severity is missing".to_string())?;
        let normalized_severity = match severity {
            "blocking" => "blocking",
            "must_fix" | "strong_recommend_fix" => "must_fix",
            "suggestion" | "minor" | "optional" => "suggestion",
            other => return Err(format!("unknown review finding severity: {other}")),
        };
        finding.insert(
            "severity".to_string(),
            serde_json::Value::String(normalized_severity.to_string()),
        );

        let impact = finding.remove("impact").and_then(|impact| match impact {
            serde_json::Value::String(impact) if !impact.trim().is_empty() => Some(impact),
            _ => None,
        });
        if let Some(impact) = impact {
            let message = finding
                .get("message")
                .and_then(|message| message.as_str())
                .ok_or_else(|| "historical review finding message is missing".to_string())?;
            let suffix = format!("\n影响：{impact}");
            if !message.contains(&suffix) {
                finding.insert(
                    "message".to_string(),
                    serde_json::Value::String(format!("{message}{suffix}")),
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn parse_review_value(
    value: &serde_json::Value,
    comments: &str,
) -> Result<ReviewVerdict, ReviewStructuredOutputErrorCode> {
    let verdict = value
        .get("verdict")
        .and_then(serde_json::Value::as_str)
        .ok_or(ReviewStructuredOutputErrorCode::MissingVerdict)?;
    let parsed_verdict = match verdict {
        "pass" => ReviewVerdictType::Pass,
        "revise" => ReviewVerdictType::Revise,
        "needs_human" => ReviewVerdictType::NeedsHuman,
        _ => return Err(ReviewStructuredOutputErrorCode::InvalidVerdict),
    };
    let summary = value
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(match parsed_verdict {
            ReviewVerdictType::Pass => "审核通过",
            ReviewVerdictType::Revise => "需要返修",
            ReviewVerdictType::NeedsHuman => "需要人工确认",
        })
        .to_string();
    let parsed_findings = parse_review_findings(value.get("findings"));
    if let Some(error) = parsed_findings.structured_error.as_ref() {
        return Err(error.clone().into());
    }
    if parsed_findings.malformed {
        return Err(ReviewStructuredOutputErrorCode::MalformedFindings);
    }
    let review_gate = review_gate_for(&parsed_verdict, &parsed_findings);
    let verdict = match review_gate {
        ReviewGate::RequiresRevision => ReviewVerdictType::Revise,
        ReviewGate::UserConfirmAllowed => match parsed_verdict {
            ReviewVerdictType::Pass => ReviewVerdictType::Pass,
            ReviewVerdictType::Revise | ReviewVerdictType::NeedsHuman => {
                ReviewVerdictType::NeedsHuman
            }
        },
        ReviewGate::UserTriageRequired => ReviewVerdictType::NeedsHuman,
    };
    Ok(ReviewVerdict {
        verdict,
        comments: comments.trim().to_string(),
        summary,
        findings: parsed_findings.findings,
        review_gate,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    })
}

#[cfg(test)]
pub(crate) fn parse_work_item_plan_review_json(
    json: &str,
    comments: &str,
    valid_outline_ids: &[String],
    scope: WorkItemPlanReviewScope,
) -> Option<ReviewVerdict> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    parse_work_item_plan_review_value(&value, comments, valid_outline_ids, scope).ok()
}

pub(crate) fn parse_historical_work_item_plan_review_json(
    json: &str,
    comments: &str,
    valid_outline_ids: &[String],
    scope: WorkItemPlanReviewScope,
) -> Option<ReviewVerdict> {
    let mut value: serde_json::Value = serde_json::from_str(json).ok()?;
    normalize_historical_review_findings(&mut value).ok()?;
    parse_work_item_plan_review_value(&value, comments, valid_outline_ids, scope).ok()
}

pub(crate) fn parse_work_item_plan_review_value(
    value: &serde_json::Value,
    comments: &str,
    valid_outline_ids: &[String],
    scope: WorkItemPlanReviewScope,
) -> Result<ReviewVerdict, ReviewStructuredOutputErrorCode> {
    let payload_scope = value
        .get("review_scope")
        .and_then(|value| value.as_str())
        .ok_or(ReviewStructuredOutputErrorCode::InvalidReviewScope)?;
    let expected_scope = match scope {
        WorkItemPlanReviewScope::Outline => "outline",
        WorkItemPlanReviewScope::Item => "item",
        WorkItemPlanReviewScope::Batch => "batch",
    };
    if payload_scope != expected_scope {
        return Err(ReviewStructuredOutputErrorCode::InvalidReviewScope);
    }
    let verdict = value
        .get("verdict")
        .and_then(|value| value.as_str())
        .ok_or(ReviewStructuredOutputErrorCode::MissingVerdict)?;
    let parsed_verdict = match (&scope, verdict) {
        (_, "pass") => WorkItemPlanReviewVerdict::Pass,
        (WorkItemPlanReviewScope::Outline | WorkItemPlanReviewScope::Item, "revise") => {
            WorkItemPlanReviewVerdict::Revise
        }
        (WorkItemPlanReviewScope::Batch, "revise_batch") => WorkItemPlanReviewVerdict::ReviseBatch,
        (_, "needs_human") => WorkItemPlanReviewVerdict::NeedsHuman,
        (
            WorkItemPlanReviewScope::Item | WorkItemPlanReviewScope::Batch,
            "plan_reopen_required",
        ) => WorkItemPlanReviewVerdict::PlanReopenRequired,
        _ => return Err(ReviewStructuredOutputErrorCode::InvalidVerdict),
    };
    let summary = value
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or(match parsed_verdict {
            WorkItemPlanReviewVerdict::Pass => "审核通过",
            WorkItemPlanReviewVerdict::Revise => "需要返修当前 Work Item",
            WorkItemPlanReviewVerdict::ReviseBatch => "需要重写当前 Batch",
            WorkItemPlanReviewVerdict::NeedsHuman => "需要人工确认",
            WorkItemPlanReviewVerdict::PlanReopenRequired => "需要重开 Outline",
        })
        .to_string();
    let target_outline_id = match value.get("target_outline_id") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(id)) => Some(id.clone()),
        Some(_) => return Err(ReviewStructuredOutputErrorCode::InvalidOutlineReference),
    };
    if target_outline_id
        .as_ref()
        .is_some_and(|id| !valid_outline_ids.iter().any(|valid| valid == id))
    {
        return Err(ReviewStructuredOutputErrorCode::InvalidOutlineReference);
    }
    let parsed_findings = parse_review_findings(value.get("findings"));
    if let Some(error) = parsed_findings.structured_error.as_ref() {
        return Err(error.clone().into());
    }
    if parsed_findings.malformed {
        return Err(ReviewStructuredOutputErrorCode::MalformedFindings);
    }
    let effective_verdict =
        effective_work_item_plan_review_verdict(&parsed_verdict, &scope, &parsed_findings);

    let (affects_items, warnings, total_affects, invalid_affects) =
        parse_work_item_plan_review_affects_items(
            value.get("affects_items"),
            value.get("findings"),
            valid_outline_ids,
            &scope,
        );
    let invalid_affects_are_fatal = total_affects > 0
        && invalid_affects * 2 > total_affects
        && !(scope == WorkItemPlanReviewScope::Outline
            && effective_verdict == WorkItemPlanReviewVerdict::Pass);
    if invalid_affects_are_fatal {
        return Err(ReviewStructuredOutputErrorCode::InvalidOutlineReference);
    }

    let generation_round_id = value
        .get("generation_round_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or(ReviewStructuredOutputErrorCode::InvalidGenerationRound)?
        .to_string();
    let draft_id = value
        .get("draft_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let batch_id = value
        .get("batch_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let (generic_verdict, review_gate, review_action, gates) =
        work_item_plan_review_routing(&effective_verdict, &scope);
    let extension = WorkItemPlanReviewComplete {
        verdict: effective_verdict,
        review_scope: scope,
        target_outline_id,
        generation_round_id,
        draft_id,
        batch_id,
        review_action,
        gates,
        affects_items,
        warnings,
    };

    Ok(ReviewVerdict {
        verdict: generic_verdict,
        comments: comments.trim().to_string(),
        summary,
        findings: parsed_findings.findings,
        review_gate,
        work_item_plan_review: Some(extension),
        structured_output_diagnostic: None,
    })
}

fn effective_work_item_plan_review_verdict(
    verdict: &WorkItemPlanReviewVerdict,
    scope: &WorkItemPlanReviewScope,
    parsed_findings: &ParsedReviewFindings,
) -> WorkItemPlanReviewVerdict {
    if parsed_findings.malformed {
        return WorkItemPlanReviewVerdict::NeedsHuman;
    }
    if verdict == &WorkItemPlanReviewVerdict::Pass
        && parsed_findings.findings.iter().any(|finding| {
            matches!(
                finding.severity,
                ReviewFindingSeverity::Blocking | ReviewFindingSeverity::MustFix
            )
        })
    {
        return match scope {
            WorkItemPlanReviewScope::Outline | WorkItemPlanReviewScope::Item => {
                WorkItemPlanReviewVerdict::Revise
            }
            WorkItemPlanReviewScope::Batch => WorkItemPlanReviewVerdict::ReviseBatch,
        };
    }
    verdict.clone()
}

pub(crate) fn work_item_plan_review_routing(
    verdict: &WorkItemPlanReviewVerdict,
    scope: &WorkItemPlanReviewScope,
) -> (
    ReviewVerdictType,
    ReviewGate,
    WorkItemPlanReviewAction,
    Vec<WorkItemPlanReviewGate>,
) {
    match verdict {
        WorkItemPlanReviewVerdict::Pass => (
            ReviewVerdictType::Pass,
            ReviewGate::UserConfirmAllowed,
            WorkItemPlanReviewAction::Continue,
            Vec::new(),
        ),
        WorkItemPlanReviewVerdict::Revise => {
            if scope == &WorkItemPlanReviewScope::Outline {
                (
                    ReviewVerdictType::Revise,
                    ReviewGate::RequiresRevision,
                    WorkItemPlanReviewAction::ReviseOutline,
                    vec![WorkItemPlanReviewGate::RequiresPlanReopen],
                )
            } else {
                (
                    ReviewVerdictType::Revise,
                    ReviewGate::RequiresRevision,
                    WorkItemPlanReviewAction::ReviseCurrentItem,
                    vec![WorkItemPlanReviewGate::RequiresCurrentItemRevision],
                )
            }
        }
        WorkItemPlanReviewVerdict::ReviseBatch => (
            ReviewVerdictType::NeedsHuman,
            ReviewGate::UserTriageRequired,
            WorkItemPlanReviewAction::ReviseBatch,
            vec![WorkItemPlanReviewGate::RequiresBatchRevision],
        ),
        WorkItemPlanReviewVerdict::NeedsHuman => (
            ReviewVerdictType::NeedsHuman,
            ReviewGate::UserTriageRequired,
            WorkItemPlanReviewAction::HumanTriage,
            Vec::new(),
        ),
        WorkItemPlanReviewVerdict::PlanReopenRequired => (
            ReviewVerdictType::NeedsHuman,
            ReviewGate::UserTriageRequired,
            WorkItemPlanReviewAction::ReviseOutline,
            vec![WorkItemPlanReviewGate::RequiresPlanReopen],
        ),
    }
}

pub(crate) fn parse_work_item_plan_review_affects_items(
    legacy_value: Option<&serde_json::Value>,
    findings_value: Option<&serde_json::Value>,
    valid_outline_ids: &[String],
    scope: &WorkItemPlanReviewScope,
) -> (
    Vec<WorkItemPlanReviewAffectedItem>,
    Vec<String>,
    usize,
    usize,
) {
    let mut valid_items = Vec::new();
    let mut warnings = Vec::new();
    let mut total_count = 0;
    let mut invalid_count = 0;
    let findings = findings_value.and_then(|value| value.as_array());
    let findings_have_targets = scope == &WorkItemPlanReviewScope::Outline
        && findings.is_some_and(|findings| {
            findings.iter().any(|finding| {
                finding
                    .get("target_outline_id")
                    .is_some_and(|value| !value.is_null())
            })
        });

    if !findings_have_targets && let Some(items) = legacy_value.and_then(|value| value.as_array()) {
        for item in items {
            total_count += 1;
            let (outline_index, target_outline_id) = if let Some(id) = item.as_str() {
                (None, Some(id.to_string()))
            } else {
                (
                    item.get("outline_index")
                        .and_then(|value| value.as_u64())
                        .and_then(|value| u32::try_from(value).ok()),
                    item.get("target_outline_id")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                )
            };
            collect_work_item_plan_review_reference(
                outline_index,
                target_outline_id,
                valid_outline_ids,
                &mut valid_items,
                &mut warnings,
                &mut invalid_count,
            );
        }
    }

    if findings_have_targets && let Some(findings) = findings {
        for finding in findings {
            let Some(target_value) = finding.get("target_outline_id") else {
                continue;
            };
            if target_value.is_null() {
                continue;
            }
            total_count += 1;
            let target_outline_id = target_value.as_str().map(ToString::to_string);
            collect_work_item_plan_review_reference(
                None,
                target_outline_id,
                valid_outline_ids,
                &mut valid_items,
                &mut warnings,
                &mut invalid_count,
            );
        }
    }

    (valid_items, warnings, total_count, invalid_count)
}

fn collect_work_item_plan_review_reference(
    outline_index: Option<u32>,
    target_outline_id: Option<String>,
    valid_outline_ids: &[String],
    valid_items: &mut Vec<WorkItemPlanReviewAffectedItem>,
    warnings: &mut Vec<String>,
    invalid_count: &mut usize,
) {
    let index_valid = outline_index.is_none_or(|index| {
        usize::try_from(index)
            .ok()
            .is_some_and(|index| index < valid_outline_ids.len())
    });
    let target_valid = target_outline_id
        .as_ref()
        .is_some_and(|id| valid_outline_ids.iter().any(|valid| valid == id));
    let valid = index_valid && (target_outline_id.is_none() || target_valid);

    if valid && (outline_index.is_some() || target_outline_id.is_some()) {
        let affected_item = WorkItemPlanReviewAffectedItem {
            outline_index,
            target_outline_id,
        };
        if !valid_items.contains(&affected_item) {
            valid_items.push(affected_item);
        }
    } else {
        *invalid_count += 1;
        warnings.push(format!(
            "invalid_reference: target_outline_id={} not found",
            target_outline_id.as_deref().unwrap_or("<missing>")
        ));
    }
}

pub(crate) struct ParsedReviewFindings {
    pub(crate) findings: Vec<ReviewFinding>,
    pub(crate) malformed: bool,
    pub(crate) structured_error: Option<ReviewStructuredOutputError>,
}

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
struct RawReviewFinding {
    severity: String,
    message: String,
    #[serde(default)]
    evidence: String,
    #[serde(default)]
    required_action: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    class_hint: Option<String>,
    #[serde(default)]
    contract_field: Option<String>,
    #[serde(skip_deserializing)]
    finding_id: Option<String>,
    #[serde(skip_deserializing)]
    code: Option<String>,
    #[serde(skip_deserializing)]
    work_item_ids: Vec<String>,
}

fn parse_review_findings_strict(
    value: Option<&serde_json::Value>,
) -> Result<Vec<ReviewFinding>, ReviewStructuredOutputError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| invalid_finding_field("findings", "must be an array"))?;

    items
        .iter()
        .enumerate()
        .map(|(index, item)| parse_raw_review_finding(item, index))
        .collect()
}

fn parse_raw_review_finding(
    value: &serde_json::Value,
    index: usize,
) -> Result<ReviewFinding, ReviewStructuredOutputError> {
    let prefix = format!("findings[{index}]");
    let object = value
        .as_object()
        .ok_or_else(|| invalid_finding_field(prefix.clone(), "must be an object"))?;
    // `target_outline_id` is intentionally excluded from this generic policy
    // envelope. The WorkItemPlan legacy parser consumes it directly from the
    // raw finding to derive `affects_items`; accepting it here would otherwise
    // acknowledge and discard its routing meaning.
    for field in object.keys() {
        if !matches!(
            field.as_str(),
            "severity"
                | "message"
                | "evidence"
                | "required_action"
                | "category"
                | "class_hint"
                | "contract_field"
                | "finding_id"
                | "code"
                | "work_item_ids"
        ) {
            return Err(invalid_finding_field(
                format!("{prefix}.{field}"),
                "is not allowed",
            ));
        }
    }
    required_string_field(object, &prefix, "severity")?;
    required_string_field(object, &prefix, "message")?;
    optional_string_field(object, &prefix, "evidence")?;
    optional_string_field(object, &prefix, "required_action")?;
    optional_string_or_null_field(object, &prefix, "category")?;
    optional_string_or_null_field(object, &prefix, "class_hint")?;
    optional_string_or_null_field(object, &prefix, "contract_field")?;

    let raw = serde_json::from_value::<RawReviewFinding>(value.clone()).map_err(|error| {
        invalid_finding_field(prefix.clone(), format!("cannot be decoded: {error}"))
    })?;
    let severity = parse_live_review_finding_severity(&raw.severity).ok_or_else(|| {
        invalid_finding_field(
            format!("{prefix}.severity"),
            "must be blocking, must_fix, or suggestion",
        )
    })?;

    Ok(ReviewFinding {
        severity,
        message: raw.message,
        evidence: raw.evidence,
        required_action: raw.required_action,
        category: raw
            .category
            .map(|raw| parse_finding_category(&raw))
            .transpose()?,
        class_hint: raw
            .class_hint
            .map(|raw| parse_finding_class_hint(&raw))
            .transpose()?,
        contract_field: raw.contract_field,
    })
}

fn parse_finding_category(
    value: &str,
) -> Result<crate::product::work_item_plan_policy::ReviewFindingCategory, ReviewStructuredOutputError>
{
    use crate::product::work_item_plan_policy::ReviewFindingCategory;

    match value {
        "contract_gap" => Ok(ReviewFindingCategory::ContractGap),
        "self_contradiction" => Ok(ReviewFindingCategory::SelfContradiction),
        "scope_conflict" => Ok(ReviewFindingCategory::ScopeConflict),
        "verification_unattributable" => Ok(ReviewFindingCategory::VerificationUnattributable),
        "completeness" => Ok(ReviewFindingCategory::Completeness),
        "other" => Ok(ReviewFindingCategory::Other),
        _ => Err(ReviewStructuredOutputError::UnknownFindingCategory(
            value.to_string(),
        )),
    }
}

fn parse_finding_class_hint(
    value: &str,
) -> Result<crate::product::work_item_plan_policy::FindingClassHint, ReviewStructuredOutputError> {
    use crate::product::work_item_plan_policy::FindingClassHint;

    match value {
        "repairable" => Ok(FindingClassHint::Repairable),
        "human_required" => Ok(FindingClassHint::HumanRequired),
        "advisory" => Ok(FindingClassHint::Advisory),
        _ => Err(ReviewStructuredOutputError::UnknownFindingClassHint(
            value.to_string(),
        )),
    }
}

fn required_string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    field: &str,
) -> Result<(), ReviewStructuredOutputError> {
    if object.get(field).is_some_and(serde_json::Value::is_string) {
        Ok(())
    } else {
        Err(invalid_finding_field(
            format!("{prefix}.{field}"),
            "must be a string",
        ))
    }
}

fn optional_string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    field: &str,
) -> Result<(), ReviewStructuredOutputError> {
    match object.get(field) {
        None | Some(serde_json::Value::String(_)) => Ok(()),
        Some(_) => Err(invalid_finding_field(
            format!("{prefix}.{field}"),
            "must be a string",
        )),
    }
}

fn optional_string_or_null_field(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
    field: &str,
) -> Result<(), ReviewStructuredOutputError> {
    match object.get(field) {
        None | Some(serde_json::Value::Null) | Some(serde_json::Value::String(_)) => Ok(()),
        Some(_) => Err(invalid_finding_field(
            format!("{prefix}.{field}"),
            "must be a string or null",
        )),
    }
}

fn invalid_finding_field(
    field: impl Into<String>,
    details: impl Into<String>,
) -> ReviewStructuredOutputError {
    ReviewStructuredOutputError::InvalidFindingField {
        field: field.into(),
        details: details.into(),
    }
}

pub(crate) fn parse_review_findings(value: Option<&serde_json::Value>) -> ParsedReviewFindings {
    let Some(value) = value else {
        return ParsedReviewFindings {
            findings: Vec::new(),
            malformed: false,
            structured_error: None,
        };
    };
    let Some(items) = value.as_array() else {
        return ParsedReviewFindings {
            findings: Vec::new(),
            malformed: true,
            structured_error: None,
        };
    };

    let mut findings = Vec::new();
    let mut malformed = false;
    for (index, item) in items.iter().enumerate() {
        let Some(object) = item.as_object() else {
            malformed = true;
            continue;
        };
        if object.contains_key("impact") {
            malformed = true;
            continue;
        }
        let Some(severity) = object
            .get("severity")
            .and_then(serde_json::Value::as_str)
            .and_then(parse_live_review_finding_severity)
        else {
            malformed = true;
            continue;
        };
        let Some(message) = object.get("message").and_then(serde_json::Value::as_str) else {
            malformed = true;
            continue;
        };
        let prefix = format!("findings[{index}]");
        let category = match parse_optional_finding_category(object, &prefix) {
            Ok(category) => category,
            Err(error) => {
                return ParsedReviewFindings {
                    findings: Vec::new(),
                    malformed: false,
                    structured_error: Some(error),
                };
            }
        };
        let class_hint = match parse_optional_finding_class_hint(object, &prefix) {
            Ok(class_hint) => class_hint,
            Err(error) => {
                return ParsedReviewFindings {
                    findings: Vec::new(),
                    malformed: false,
                    structured_error: Some(error),
                };
            }
        };
        let contract_field = match parse_optional_contract_field(object, &prefix) {
            Ok(contract_field) => contract_field,
            Err(error) => {
                return ParsedReviewFindings {
                    findings: Vec::new(),
                    malformed: false,
                    structured_error: Some(error),
                };
            }
        };

        findings.push(ReviewFinding {
            severity,
            message: message.to_string(),
            evidence: object
                .get("evidence")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            required_action: object
                .get("required_action")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            category,
            class_hint,
            contract_field,
        });
    }

    ParsedReviewFindings {
        findings,
        malformed,
        structured_error: None,
    }
}

fn parse_optional_finding_category(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
) -> Result<
    Option<crate::product::work_item_plan_policy::ReviewFindingCategory>,
    ReviewStructuredOutputError,
> {
    match object.get("category") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => parse_finding_category(value).map(Some),
        Some(_) => Err(invalid_finding_field(
            format!("{prefix}.category"),
            "must be a string or null",
        )),
    }
}

fn parse_optional_finding_class_hint(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
) -> Result<
    Option<crate::product::work_item_plan_policy::FindingClassHint>,
    ReviewStructuredOutputError,
> {
    match object.get("class_hint") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => parse_finding_class_hint(value).map(Some),
        Some(_) => Err(invalid_finding_field(
            format!("{prefix}.class_hint"),
            "must be a string or null",
        )),
    }
}

fn parse_optional_contract_field(
    object: &serde_json::Map<String, serde_json::Value>,
    prefix: &str,
) -> Result<Option<String>, ReviewStructuredOutputError> {
    match object.get("contract_field") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(invalid_finding_field(
            format!("{prefix}.contract_field"),
            "must be a string or null",
        )),
    }
}

pub(crate) fn parse_live_review_finding_severity(value: &str) -> Option<ReviewFindingSeverity> {
    match value {
        "blocking" => Some(ReviewFindingSeverity::Blocking),
        "must_fix" => Some(ReviewFindingSeverity::MustFix),
        "suggestion" => Some(ReviewFindingSeverity::Suggestion),
        _ => None,
    }
}

pub(crate) fn review_gate_for(
    verdict: &ReviewVerdictType,
    parsed_findings: &ParsedReviewFindings,
) -> ReviewGate {
    if parsed_findings.findings.iter().any(|finding| {
        matches!(
            finding.severity,
            ReviewFindingSeverity::Blocking | ReviewFindingSeverity::MustFix
        )
    }) {
        return ReviewGate::RequiresRevision;
    }
    if parsed_findings.malformed {
        return ReviewGate::UserTriageRequired;
    }

    match verdict {
        ReviewVerdictType::Pass => ReviewGate::UserConfirmAllowed,
        ReviewVerdictType::NeedsHuman => ReviewGate::UserTriageRequired,
        ReviewVerdictType::Revise if parsed_findings.findings.is_empty() => {
            ReviewGate::UserTriageRequired
        }
        ReviewVerdictType::Revise => ReviewGate::UserConfirmAllowed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_envelope_parses_legacy_findings_with_new_fields_defaulted() {
        let envelope = parse_review_envelope(&serde_json::json!({
            "verdict": "revise",
            "findings": [{
                "severity": "must_fix",
                "message": "legacy finding",
                "evidence": "evidence",
                "required_action": "repair it"
            }]
        }))
        .unwrap();

        assert_eq!(envelope.raw_verdict, ReviewVerdictType::Revise);
        assert_eq!(envelope.findings[0].category, None);
        assert_eq!(envelope.findings[0].class_hint, None);
        assert_eq!(envelope.findings[0].contract_field, None);
    }

    #[test]
    fn review_envelope_ignores_provider_finding_identity_fields() {
        let envelope = parse_review_envelope(&serde_json::json!({
            "verdict": "pass",
            "findings": [{
                "severity": "suggestion",
                "message": "advisory finding",
                "category": "completeness",
                "class_hint": "advisory",
                "finding_id": "provider-finding-001",
                "code": "STYLE-001",
                "work_item_ids": ["WT-001"]
            }]
        }))
        .expect("provider identity fields should be tolerated");

        assert_eq!(envelope.raw_verdict, ReviewVerdictType::Pass);
        assert_eq!(envelope.findings.len(), 1);
        assert_eq!(envelope.findings[0].message, "advisory finding");
        assert_eq!(
            envelope.findings[0].category,
            Some(crate::product::work_item_plan_policy::ReviewFindingCategory::Completeness)
        );
        assert_eq!(
            envelope.findings[0].class_hint,
            Some(crate::product::work_item_plan_policy::FindingClassHint::Advisory)
        );
    }

    #[test]
    fn review_envelope_unknown_category_and_hint_have_distinct_stable_errors() {
        let category = parse_review_envelope(&serde_json::json!({
            "verdict": "revise",
            "findings": [{
                "severity": "must_fix",
                "message": "unknown category",
                "category": "unsupported"
            }]
        }))
        .unwrap_err();
        assert_eq!(
            category,
            ReviewStructuredOutputError::UnknownFindingCategory("unsupported".to_string())
        );
        assert_eq!(category.code(), "unknown_finding_category");
        assert_eq!(
            ReviewStructuredOutputErrorCode::from(category).as_str(),
            "unknown_finding_category"
        );

        let hint = parse_review_envelope(&serde_json::json!({
            "verdict": "revise",
            "findings": [{
                "severity": "must_fix",
                "message": "unknown hint",
                "class_hint": "unsupported"
            }]
        }))
        .unwrap_err();
        assert_eq!(
            hint,
            ReviewStructuredOutputError::UnknownFindingClassHint("unsupported".to_string())
        );
        assert_eq!(hint.code(), "unknown_class_hint");
        assert_eq!(
            ReviewStructuredOutputErrorCode::from(hint).as_str(),
            "unknown_class_hint"
        );
    }

    #[test]
    fn review_envelope_reports_finding_field_type_errors_explicitly() {
        let error = parse_review_envelope(&serde_json::json!({
            "verdict": "revise",
            "findings": [{
                "severity": "must_fix",
                "message": "invalid type",
                "category": 5
            }]
        }))
        .unwrap_err();

        assert_eq!(
            error,
            ReviewStructuredOutputError::InvalidFindingField {
                field: "findings[0].category".to_string(),
                details: "must be a string or null".to_string(),
            }
        );
        assert_eq!(error.code(), "invalid_finding_field");
    }

    #[test]
    fn review_envelope_preserves_raw_needs_human_when_strong_finding_changes_gate() {
        let envelope = parse_review_envelope(&serde_json::json!({
            "verdict": "needs_human",
            "findings": [{
                "severity": "must_fix",
                "message": "strong finding"
            }]
        }))
        .unwrap();

        assert_eq!(envelope.raw_verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(envelope.normalized_gate, ReviewGate::RequiresRevision);
    }
}
