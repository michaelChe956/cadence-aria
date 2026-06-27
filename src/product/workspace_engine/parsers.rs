use super::*;

mod choice;
pub(crate) use choice::*;

pub(crate) const STRUCTURED_OUTPUT_START_PREFIX: &str = "<ARIA_STRUCTURED_OUTPUT";

pub(crate) const STRUCTURED_OUTPUT_END_PREFIX: &str = "</ARIA_STRUCTURED_OUTPUT";

pub(crate) fn extract_structured_json(output: &str) -> Option<(String, String)> {
    extract_nonce_sentinel_json(output).or_else(|| extract_markdown_fence_json(output))
}

pub(crate) fn extract_nonce_sentinel_json(output: &str) -> Option<(String, String)> {
    let mut search_end = output.len();
    while let Some(start) = output[..search_end].rfind(STRUCTURED_OUTPUT_START_PREFIX) {
        let after_start_prefix = &output[start + STRUCTURED_OUTPUT_START_PREFIX.len()..];
        let Some((Some(start_nonce), start_tag_len)) =
            parse_structured_output_tag(after_start_prefix)
        else {
            search_end = start;
            continue;
        };
        let json_start = start + STRUCTURED_OUTPUT_START_PREFIX.len() + start_tag_len;
        let after_start = &output[json_start..];
        let Some(end) = after_start.find(STRUCTURED_OUTPUT_END_PREFIX) else {
            search_end = start;
            continue;
        };
        let after_end_prefix = &after_start[end + STRUCTURED_OUTPUT_END_PREFIX.len()..];
        let Some((end_nonce, _end_tag_len)) = parse_structured_output_tag(after_end_prefix) else {
            search_end = start;
            continue;
        };
        if end_nonce.as_deref() != Some(start_nonce.as_str()) {
            search_end = start;
            continue;
        }
        return Some((
            output[..start].to_string(),
            after_start[..end].trim().to_string(),
        ));
    }
    None
}

pub(crate) fn parse_structured_output_tag(after_prefix: &str) -> Option<(Option<String>, usize)> {
    let end_offset = after_prefix.find('>')?;
    let attrs = after_prefix[..end_offset].trim();
    let nonce = parse_structured_output_nonce(attrs)?;
    Some((nonce, end_offset + 1))
}

pub(crate) fn parse_structured_output_nonce(attrs: &str) -> Option<Option<String>> {
    if attrs.is_empty() {
        return Some(None);
    }
    let nonce = attrs
        .strip_prefix("nonce=\"")
        .and_then(|value| value.strip_suffix('"'))?;
    if nonce.len() != 8 || !nonce.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    Some(Some(nonce.to_string()))
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

pub(crate) fn parse_review_json(json: &str, comments: &str) -> Option<ReviewVerdict> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let parsed_verdict = match value.get("verdict")?.as_str()? {
        "pass" => ReviewVerdictType::Pass,
        "revise" => ReviewVerdictType::Revise,
        "needs_human" => ReviewVerdictType::NeedsHuman,
        _ => return None,
    };
    let summary = value
        .get("summary")
        .and_then(|value| value.as_str())
        .unwrap_or(match parsed_verdict {
            ReviewVerdictType::Pass => "审核通过",
            ReviewVerdictType::Revise => "需要返修",
            ReviewVerdictType::NeedsHuman => "需要人工确认",
        })
        .to_string();
    let parsed_findings = parse_review_findings(value.get("findings"));
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
    Some(ReviewVerdict {
        verdict,
        comments: comments.trim().to_string(),
        summary,
        findings: parsed_findings.findings,
        review_gate,
        work_item_plan_review: None,
    })
}

pub(crate) fn parse_work_item_plan_review_json(
    json: &str,
    comments: &str,
    valid_outline_ids: &[String],
    scope: WorkItemPlanReviewScope,
) -> Option<ReviewVerdict> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let parsed_verdict = parse_work_item_plan_review_verdict(value.get("verdict")?.as_str()?);
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
    let target_outline_id = value
        .get("target_outline_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    if target_outline_id
        .as_ref()
        .is_some_and(|id| !valid_outline_ids.iter().any(|valid| valid == id))
    {
        return Some(work_item_plan_review_invalid_reference(comments));
    }

    let (affects_items, warnings, total_affects, invalid_affects) =
        parse_work_item_plan_review_affects_items(value.get("affects_items"), valid_outline_ids);
    if total_affects > 0 && invalid_affects * 2 > total_affects {
        return Some(work_item_plan_review_invalid_reference(comments));
    }

    let parsed_findings = parse_review_findings(value.get("findings"));
    let generation_round_id = value
        .get("generation_round_id")
        .and_then(|value| value.as_str())
        .unwrap_or("generation_round_unknown")
        .to_string();
    let draft_id = value
        .get("draft_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let batch_id = value
        .get("batch_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let effective_verdict =
        effective_work_item_plan_review_verdict(&parsed_verdict, &scope, &parsed_findings);
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

    Some(ReviewVerdict {
        verdict: generic_verdict,
        comments: comments.trim().to_string(),
        summary,
        findings: parsed_findings.findings,
        review_gate,
        work_item_plan_review: Some(extension),
    })
}

pub(crate) fn parse_work_item_plan_review_verdict(value: &str) -> WorkItemPlanReviewVerdict {
    match value {
        "pass" => WorkItemPlanReviewVerdict::Pass,
        "revise" => WorkItemPlanReviewVerdict::Revise,
        "revise_batch" => WorkItemPlanReviewVerdict::ReviseBatch,
        "needs_human" => WorkItemPlanReviewVerdict::NeedsHuman,
        "plan_reopen_required" => WorkItemPlanReviewVerdict::PlanReopenRequired,
        _ => WorkItemPlanReviewVerdict::NeedsHuman,
    }
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
                ReviewFindingSeverity::Blocking
                    | ReviewFindingSeverity::MustFix
                    | ReviewFindingSeverity::StrongRecommendFix
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
    value: Option<&serde_json::Value>,
    valid_outline_ids: &[String],
) -> (
    Vec<WorkItemPlanReviewAffectedItem>,
    Vec<String>,
    usize,
    usize,
) {
    let Some(items) = value.and_then(|value| value.as_array()) else {
        return (Vec::new(), Vec::new(), 0, 0);
    };

    let mut valid_items = Vec::new();
    let mut warnings = Vec::new();
    let mut invalid_count = 0;
    for item in items {
        let outline_index = item
            .get("outline_index")
            .and_then(|value| value.as_u64())
            .and_then(|value| u32::try_from(value).ok());
        let target_outline_id = item
            .get("target_outline_id")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
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
            valid_items.push(WorkItemPlanReviewAffectedItem {
                outline_index,
                target_outline_id,
            });
        } else {
            invalid_count += 1;
            warnings.push(format!(
                "invalid_reference: target_outline_id={} not found",
                target_outline_id.as_deref().unwrap_or("<missing>")
            ));
        }
    }

    (valid_items, warnings, items.len(), invalid_count)
}

pub(crate) fn work_item_plan_review_invalid_reference(comments: &str) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: comments.trim().to_string(),
        summary: "WorkItemPlan reviewer 引用无效，需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
    }
}

pub(crate) struct ParsedReviewFindings {
    pub(crate) findings: Vec<ReviewFinding>,
    pub(crate) malformed: bool,
}

pub(crate) fn parse_review_findings(value: Option<&serde_json::Value>) -> ParsedReviewFindings {
    let Some(value) = value else {
        return ParsedReviewFindings {
            findings: Vec::new(),
            malformed: false,
        };
    };
    let Some(items) = value.as_array() else {
        return ParsedReviewFindings {
            findings: Vec::new(),
            malformed: true,
        };
    };

    let mut findings = Vec::new();
    let mut malformed = false;
    for item in items {
        let Some(severity) = item
            .get("severity")
            .and_then(|value| value.as_str())
            .and_then(parse_review_finding_severity)
        else {
            malformed = true;
            continue;
        };
        let Some(message) = item.get("message").and_then(|value| value.as_str()) else {
            malformed = true;
            continue;
        };

        findings.push(ReviewFinding {
            severity,
            message: message.to_string(),
            evidence: item
                .get("evidence")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            impact: item
                .get("impact")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            required_action: item
                .get("required_action")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }

    ParsedReviewFindings {
        findings,
        malformed,
    }
}

pub(crate) fn parse_review_finding_severity(value: &str) -> Option<ReviewFindingSeverity> {
    match value {
        "blocking" => Some(ReviewFindingSeverity::Blocking),
        "must_fix" => Some(ReviewFindingSeverity::MustFix),
        "strong_recommend_fix" => Some(ReviewFindingSeverity::StrongRecommendFix),
        "suggestion" => Some(ReviewFindingSeverity::Suggestion),
        "minor" => Some(ReviewFindingSeverity::Minor),
        "optional" => Some(ReviewFindingSeverity::Optional),
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
            ReviewFindingSeverity::Blocking
                | ReviewFindingSeverity::MustFix
                | ReviewFindingSeverity::StrongRecommendFix
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
