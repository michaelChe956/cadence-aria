use crate::product::work_item_plan_policy::ReviewStructuredOutputError;

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
    /// REQ-WSC-11：`needs_human` 结论缺少面向 owner 的人话提问或选项——判
    /// 结构化输出不合规，交由既有 `structured_output_diagnostic` 通道收口。
    MissingOwnerGuidance,
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
            Self::MissingOwnerGuidance => "missing_owner_guidance",
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
            Self::MissingOwnerGuidance => "需要人工裁决的审核结论缺少面向用户的问题或选项",
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
