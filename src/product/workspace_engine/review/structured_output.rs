use super::*;
use crate::cross_cutting::structured_output::StructuredOutputErrorCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewCompletionError {
    Syntax(StructuredOutputError),
    Schema(ReviewStructuredOutputErrorCode),
    NotRequested,
    RepairPayloadChanged,
}

impl ReviewCompletionError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Syntax(error) => error.code.as_str(),
            Self::Schema(error) => error.as_str(),
            Self::NotRequested => "structured_output_not_requested",
            Self::RepairPayloadChanged => "repair_payload_changed",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Syntax(error) => error.message.clone(),
            Self::Schema(error) => error.message().to_string(),
            Self::NotRequested => "审核输入未请求结构化输出".to_string(),
            Self::RepairPayloadChanged => "结构化输出修复改变了审核业务内容".to_string(),
        }
    }

    pub(crate) fn recoverable_value(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Syntax(error) => error.recoverable_value.as_ref(),
            Self::Schema(_) | Self::NotRequested | Self::RepairPayloadChanged => None,
        }
    }

    pub(crate) fn is_repairable(&self) -> bool {
        matches!(
            self,
            Self::Syntax(error)
                if error.recoverable_value.is_some()
                    && matches!(
                        error.code,
                        StructuredOutputErrorCode::MissingEndTag
                            | StructuredOutputErrorCode::InvalidEndTag
                            | StructuredOutputErrorCode::NonceMismatch
                            | StructuredOutputErrorCode::MissingJsonNonce
                            | StructuredOutputErrorCode::JsonNonceMismatch
                    )
        )
    }
}

pub(crate) fn repair_payload_is_compatible(
    first_error: &ReviewCompletionError,
    repaired: &ProviderCompletion,
) -> bool {
    let Some(expected) = first_error.recoverable_value() else {
        return false;
    };
    matches!(
        &repaired.structured_output,
        StructuredOutputState::Parsed(actual) if actual == expected
    )
}

pub(crate) fn success_diagnostic(
    first_error: &ReviewCompletionError,
) -> StructuredOutputDiagnostic {
    StructuredOutputDiagnostic {
        code: first_error.code().to_string(),
        message: first_error.message(),
        repair_attempted: true,
        repair_succeeded: true,
        raw_output_preview: None,
    }
}

pub(crate) fn structured_output_repair_event(
    status: ProviderExecutionEventStatus,
    error_code: &str,
) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: "structured_output_repair".to_string(),
        kind: ProviderExecutionEventKind::Output,
        status,
        title: "Structured output repair".to_string(),
        detail: Some(format!("repair reviewer structured output: {error_code}")),
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
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
                _ => WorkItemPlanReviewScope::Outline,
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
