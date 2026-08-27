use crate::web::workspace_ws_types::review::{ReviewFinding, ReviewGate, ReviewVerdictType};

use super::{
    ClassifiedFinding, FindingClass, FindingClassHint, FindingFingerprint, ReviewFindingCategory,
    ReviewInvocationScope,
};

/// 保留 provider 原始 verdict；`normalized_gate` 仅服务于既有展示与协议兼容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedReviewEnvelope {
    pub(crate) raw_verdict: ReviewVerdictType,
    pub(crate) normalized_gate: ReviewGate,
    pub(crate) findings: Vec<ReviewFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewStructuredOutputError {
    UnknownFindingCategory(String),
    UnknownFindingClassHint(String),
    InvalidFindingField { field: String, details: String },
    VerificationScopeViolation { details: String },
}

impl ReviewStructuredOutputError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::UnknownFindingCategory(_) => "unknown_finding_category",
            Self::UnknownFindingClassHint(_) => "unknown_class_hint",
            Self::InvalidFindingField { .. } => "invalid_finding_field",
            Self::VerificationScopeViolation { .. } => "verification_scope_violation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClassificationError {
    UnknownCategory(String),
    UnknownClassHint(String),
    InvalidFinding(String),
    VerificationScopeViolation(String),
}

impl From<ReviewStructuredOutputError> for ClassificationError {
    fn from(error: ReviewStructuredOutputError) -> Self {
        match error {
            ReviewStructuredOutputError::UnknownFindingCategory(raw) => Self::UnknownCategory(raw),
            ReviewStructuredOutputError::UnknownFindingClassHint(raw) => {
                Self::UnknownClassHint(raw)
            }
            ReviewStructuredOutputError::InvalidFindingField { field, details } => {
                Self::InvalidFinding(format!("{field}: {details}"))
            }
            ReviewStructuredOutputError::VerificationScopeViolation { details } => {
                Self::VerificationScopeViolation(details)
            }
        }
    }
}

pub(crate) fn classify_review(
    envelope: &ParsedReviewEnvelope,
    invocation: &ReviewInvocationScope,
) -> Result<Vec<ClassifiedFinding>, ClassificationError> {
    validate_invocation_scope(invocation)?;

    let findings = envelope
        .findings
        .iter()
        .map(|finding| classify_finding(envelope.raw_verdict.clone(), finding))
        .collect::<Vec<_>>();

    // Verification scope identity is evaluated by the policy evaluator.  A
    // reviewer can legitimately surface a new finding while rephrasing its
    // report; routing that as a protocol fatal would recreate the incident
    // this scope is intended to prevent. Structural scope errors (digest and
    // missing mechanical report) remain rejected above.
    Ok(findings)
}

fn validate_invocation_scope(
    invocation: &ReviewInvocationScope,
) -> Result<(), ClassificationError> {
    invocation
        .validate_digest()
        .map_err(
            |error| ReviewStructuredOutputError::VerificationScopeViolation {
                details: format!("invalid invocation scope: {error}"),
            },
        )
        .map_err(ClassificationError::from)?;

    if let ReviewInvocationScope::Verification {
        mechanical_report_ref,
        ..
    } = invocation
        && mechanical_report_ref.trim().is_empty()
    {
        return Err(ClassificationError::VerificationScopeViolation(
            "verification invocation is missing mechanical_report_ref".to_string(),
        ));
    }

    Ok(())
}

fn classify_finding(raw_verdict: ReviewVerdictType, finding: &ReviewFinding) -> ClassifiedFinding {
    let class = finding
        .class_hint
        .map(class_for_hint)
        .unwrap_or_else(|| fallback_class(raw_verdict, finding));
    let contract_field = finding.contract_field.clone();

    ClassifiedFinding {
        class,
        fingerprint: match finding.category {
            Some(category) => FindingFingerprint::for_finding(
                Some(category),
                class,
                &finding.message,
                contract_field.as_deref(),
            ),
            None => FindingFingerprint::for_finding(
                None,
                class,
                &finding.message,
                contract_field.as_deref(),
            ),
        },
        category: finding.category,
        severity: severity_as_str(finding).to_string(),
        message: finding.message.clone(),
        evidence: non_empty_option(&finding.evidence),
        required_action: non_empty_option(&finding.required_action),
        contract_field,
    }
}

fn class_for_hint(hint: FindingClassHint) -> FindingClass {
    match hint {
        FindingClassHint::Repairable => FindingClass::Repairable,
        FindingClassHint::HumanRequired => FindingClass::HumanRequired,
        FindingClassHint::Advisory => FindingClass::Advisory,
    }
}

fn fallback_class(raw_verdict: ReviewVerdictType, finding: &ReviewFinding) -> FindingClass {
    match raw_verdict {
        ReviewVerdictType::NeedsHuman => FindingClass::HumanRequired,
        ReviewVerdictType::Pass => FindingClass::Advisory,
        ReviewVerdictType::Revise
            if matches!(
                finding.category,
                Some(ReviewFindingCategory::ContractGap | ReviewFindingCategory::SelfContradiction)
            ) && finding
                .contract_field
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()) =>
        {
            FindingClass::Repairable
        }
        ReviewVerdictType::Revise => FindingClass::HumanRequired,
    }
}

fn severity_as_str(finding: &ReviewFinding) -> &'static str {
    match finding.severity {
        crate::web::workspace_ws_types::review::ReviewFindingSeverity::Blocking => "blocking",
        crate::web::workspace_ws_types::review::ReviewFindingSeverity::MustFix => "must_fix",
        crate::web::workspace_ws_types::review::ReviewFindingSeverity::Suggestion => "suggestion",
    }
}

fn non_empty_option(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_string())
}
