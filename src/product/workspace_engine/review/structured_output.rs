use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewCompletionError {
    Syntax(StructuredOutputError),
    Schema(ReviewStructuredOutputErrorCode),
    NotRequested,
}

impl ReviewCompletionError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Syntax(error) => error.code.as_str(),
            Self::Schema(error) => error.as_str(),
            Self::NotRequested => "not_requested",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Syntax(error) => error.message.clone(),
            Self::Schema(error) => error.message().to_string(),
            Self::NotRequested => "review structured output was not requested".to_string(),
        }
    }
}

impl WorkspaceEngine {
    pub(crate) fn parse_review_completion_for_active_node(
        &self,
        completion: &ProviderCompletion,
    ) -> Result<ReviewVerdict, ReviewCompletionError> {
        let StructuredOutputState::Parsed(value) = &completion.structured_output else {
            return match &completion.structured_output {
                StructuredOutputState::Failed(error) => {
                    Err(ReviewCompletionError::Syntax(error.clone()))
                }
                StructuredOutputState::NotRequested => Err(ReviewCompletionError::NotRequested),
                StructuredOutputState::Parsed(_) => unreachable!(),
            };
        };

        if self.session.workspace_type == WorkspaceType::WorkItemPlan {
            let scope = match self.active_node_type() {
                Some(TimelineNodeType::WorkItemPlanOutlineReview) => {
                    WorkItemPlanReviewScope::Outline
                }
                Some(TimelineNodeType::WorkItemDraftReview) => WorkItemPlanReviewScope::Item,
                Some(TimelineNodeType::WorkItemBatchReview) => WorkItemPlanReviewScope::Batch,
                _ => WorkItemPlanReviewScope::Batch,
            };
            return parse_work_item_plan_review_value(
                value,
                &completion.readable_output,
                &self.current_work_item_plan_outline_ids(),
                scope,
            )
            .map_err(ReviewCompletionError::Schema);
        }

        parse_review_value(value, &completion.readable_output)
            .map_err(ReviewCompletionError::Schema)
    }
}

pub(crate) fn fallback_review_verdict(
    completion: &ProviderCompletion,
    error: &ReviewCompletionError,
    repair_attempted: bool,
) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: completion.readable_output.clone(),
        summary: "需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: error.code().to_string(),
            message: error.message(),
            repair_attempted,
            repair_succeeded: false,
            raw_output_preview: Some(preview(&completion.full_output)),
        }),
    }
}
