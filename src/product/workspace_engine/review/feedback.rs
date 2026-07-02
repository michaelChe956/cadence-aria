use crate::product::workspace_engine::serialized_string;
use crate::web::workspace_ws_types::review::ReviewVerdict;

pub(crate) fn format_review_feedback(verdict: &ReviewVerdict) -> String {
    let mut parts = Vec::new();

    if !verdict.summary.trim().is_empty() {
        parts.push(format!("[review_summary]\n{}", verdict.summary.trim()));
    }
    if !verdict.comments.trim().is_empty() {
        parts.push(format!("[review_comments]\n{}", verdict.comments.trim()));
    }
    if let Some(review) = &verdict.work_item_plan_review {
        parts.push(format!(
            "[work_item_plan_review]\nverdict: {}\nreview_scope: {}\nreview_action: {}\ntarget_outline_id: {}\ndraft_id: {}\nbatch_id: {}",
            serialized_string(&review.verdict),
            serialized_string(&review.review_scope),
            serialized_string(&review.review_action),
            review.target_outline_id.as_deref().unwrap_or(""),
            review.draft_id.as_deref().unwrap_or(""),
            review.batch_id.as_deref().unwrap_or("")
        ));
    }
    if !verdict.findings.is_empty() {
        let findings = verdict
            .findings
            .iter()
            .enumerate()
            .map(|(index, finding)| {
                format!(
                    "{}. severity: {}\n   message: {}\n   evidence: {}\n   impact: {}\n   required_action: {}",
                    index + 1,
                    serialized_string(&finding.severity),
                    finding.message.trim(),
                    finding.evidence.trim(),
                    finding.impact.trim(),
                    finding.required_action.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("[review_findings]\n{findings}"));
    }

    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::workspace_ws_types::review::{
        ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
        WorkItemPlanReviewAction, WorkItemPlanReviewComplete, WorkItemPlanReviewGate,
        WorkItemPlanReviewScope, WorkItemPlanReviewVerdict,
    };

    #[test]
    fn work_item_plan_review_feedback_includes_actionable_findings() {
        let verdict = ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "需要覆盖 provider metadata 的所有 match。".to_string(),
            summary: "遗漏 ProviderName 新枚举的边界写入范围".to_string(),
            findings: vec![ReviewFinding {
                severity: ReviewFindingSeverity::MustFix,
                message: "ProviderName 扩展遗漏 match 分支".to_string(),
                evidence: "src/product/work_item_split_engine/types.rs:86".to_string(),
                impact: "新增 provider 时 draft 会遗漏运行时映射。".to_string(),
                required_action:
                    "把 provider_name_to_type 和测试 fixture provider_name 一并纳入本 work item 的写入范围。"
                        .to_string(),
            }],
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: Some(WorkItemPlanReviewComplete {
                verdict: WorkItemPlanReviewVerdict::Revise,
                review_scope: WorkItemPlanReviewScope::Item,
                target_outline_id: Some("outline_backend_metadata_state".to_string()),
                generation_round_id: "round_0001".to_string(),
                draft_id: Some("draft_001".to_string()),
                batch_id: None,
                review_action: WorkItemPlanReviewAction::ReviseCurrentItem,
                gates: vec![WorkItemPlanReviewGate::RequiresCurrentItemRevision],
                affects_items: vec![],
                warnings: vec![],
            }),
        };

        let feedback = format_review_feedback(&verdict);

        assert!(feedback.contains("[review_summary]"));
        assert!(feedback.contains("遗漏 ProviderName 新枚举的边界写入范围"));
        assert!(feedback.contains("[review_comments]"));
        assert!(feedback.contains("[review_findings]"));
        assert!(feedback.contains("severity: must_fix"));
        assert!(feedback.contains("message: ProviderName 扩展遗漏 match 分支"));
        assert!(feedback.contains("evidence: src/product/work_item_split_engine/types.rs:86"));
        assert!(feedback.contains("impact: 新增 provider 时 draft 会遗漏运行时映射。"));
        assert!(feedback.contains("required_action: 把 provider_name_to_type"));
        assert!(feedback.contains("[work_item_plan_review]"));
        assert!(feedback.contains("review_scope: item"));
        assert!(feedback.contains("review_action: revise_current_item"));
    }
}
