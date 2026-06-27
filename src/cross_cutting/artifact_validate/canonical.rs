use super::rules::artifact_validation_rule;
use super::types::{
    ArtifactContent, ArtifactContentFamily, ArtifactValidateError, ValidationResult, json_type_name,
};
use crate::protocol::artifacts::ArtifactKind;
use serde_json::Value;

pub fn canonical_validator(
    artifact_kind: ArtifactKind,
    content: &ArtifactContent,
) -> Result<ValidationResult, ArtifactValidateError> {
    let rule = artifact_validation_rule(artifact_kind).expect("phase1 artifact kind");
    match (&rule.content_family, content) {
        (ArtifactContentFamily::Markdown, ArtifactContent::Markdown(markdown)) => {
            validate_markdown_canonical(artifact_kind, markdown)
        }
        (ArtifactContentFamily::Json, ArtifactContent::Json(value)) => {
            validate_json_canonical(artifact_kind, value)
        }
        (ArtifactContentFamily::Json, ArtifactContent::JsonText(text)) => {
            let value: Value = serde_json::from_str(text).map_err(|_| {
                ArtifactValidateError::CanonicalTypeMismatch {
                    field: "$".to_string(),
                    expected: "valid_json".to_string(),
                    got: "invalid_json".to_string(),
                }
            })?;
            validate_json_canonical(artifact_kind, &value)
        }
        (ArtifactContentFamily::Markdown, ArtifactContent::Json(_)) => {
            Err(content_family_error("content", "markdown", "json"))
        }
        (ArtifactContentFamily::Markdown, ArtifactContent::JsonText(_)) => {
            Err(content_family_error("content", "markdown", "json_text"))
        }
        (ArtifactContentFamily::Json, ArtifactContent::Markdown(_)) => {
            Err(content_family_error("content", "json", "markdown"))
        }
    }
}

fn validate_markdown_canonical(
    artifact_kind: ArtifactKind,
    markdown: &str,
) -> Result<ValidationResult, ArtifactValidateError> {
    if markdown.trim().is_empty() {
        return Err(ArtifactValidateError::CanonicalMissingField {
            field: "markdown_body".to_string(),
            artifact_kind,
        });
    }
    let (required_heading, heading_aliases): (&str, &[&str]) = match artifact_kind {
        ArtifactKind::Spec => ("功能需求", &["功能需求"]),
        ArtifactKind::Design => ("设计决策", &["设计决策", "Design Decisions"]),
        ArtifactKind::Plan => ("工作包", &["工作包", "任务拆解", "Work Packages"]),
        _ => return Ok(ValidationResult::valid()),
    };
    let has_required_heading = markdown.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with('#')
            && heading_aliases
                .iter()
                .any(|heading_alias| trimmed.contains(heading_alias))
    });
    if !has_required_heading {
        return Err(ArtifactValidateError::CanonicalMissingField {
            field: format!("heading:{required_heading}"),
            artifact_kind,
        });
    }
    Ok(ValidationResult::valid())
}

fn validate_json_canonical(
    artifact_kind: ArtifactKind,
    value: &Value,
) -> Result<ValidationResult, ArtifactValidateError> {
    let object = value
        .as_object()
        .ok_or_else(|| content_family_error("$", "json_object", json_type_name(value)))?;
    let kind_value = object.get("artifact_kind").ok_or_else(|| {
        ArtifactValidateError::CanonicalMissingField {
            field: "artifact_kind".to_string(),
            artifact_kind,
        }
    })?;
    let got = kind_value
        .as_str()
        .ok_or_else(|| ArtifactValidateError::CanonicalTypeMismatch {
            field: "artifact_kind".to_string(),
            expected: "string".to_string(),
            got: json_type_name(kind_value).to_string(),
        })?;
    if got != artifact_kind.as_str() {
        return Err(ArtifactValidateError::CanonicalTypeMismatch {
            field: "artifact_kind".to_string(),
            expected: artifact_kind.as_str().to_string(),
            got: got.to_string(),
        });
    }

    for field in required_json_fields(artifact_kind) {
        let Some(field_value) = object.get(field.name) else {
            return Err(ArtifactValidateError::CanonicalMissingField {
                field: field.name.to_string(),
                artifact_kind,
            });
        };
        if !field.kind.matches(field_value) {
            return Err(ArtifactValidateError::CanonicalTypeMismatch {
                field: field.name.to_string(),
                expected: field.kind.as_str().to_string(),
                got: json_type_name(field_value).to_string(),
            });
        }
    }
    Ok(ValidationResult::valid())
}

fn content_family_error(field: &str, expected: &str, got: &str) -> ArtifactValidateError {
    ArtifactValidateError::CanonicalTypeMismatch {
        field: field.to_string(),
        expected: expected.to_string(),
        got: got.to_string(),
    }
}

#[derive(Debug, Clone, Copy)]
struct RequiredField {
    name: &'static str,
    kind: RequiredFieldKind,
}

#[derive(Debug, Clone, Copy)]
enum RequiredFieldKind {
    String,
    Array,
    Object,
    Bool,
}

impl RequiredFieldKind {
    fn as_str(self) -> &'static str {
        match self {
            RequiredFieldKind::String => "string",
            RequiredFieldKind::Array => "array",
            RequiredFieldKind::Object => "object",
            RequiredFieldKind::Bool => "bool",
        }
    }

    fn matches(self, value: &Value) -> bool {
        match self {
            RequiredFieldKind::String => value.as_str().is_some_and(|text| !text.is_empty()),
            RequiredFieldKind::Array => value.as_array().is_some(),
            RequiredFieldKind::Object => value.as_object().is_some(),
            RequiredFieldKind::Bool => value.as_bool().is_some(),
        }
    }
}

fn required_json_fields(artifact_kind: ArtifactKind) -> &'static [RequiredField] {
    match artifact_kind {
        ArtifactKind::IntakeBrief => &INTAKE_BRIEF_FIELDS,
        ArtifactKind::ClarificationRecord => &CLARIFICATION_RECORD_FIELDS,
        ArtifactKind::SpecGateDecision => &SPEC_GATE_DECISION_FIELDS,
        ArtifactKind::DesignReview => &DESIGN_REVIEW_FIELDS,
        ArtifactKind::DesignRevisionRecord => &DESIGN_REVISION_RECORD_FIELDS,
        ArtifactKind::ReadinessCheck => &READINESS_CHECK_FIELDS,
        ArtifactKind::DispatchPackage => &DISPATCH_PACKAGE_FIELDS,
        ArtifactKind::CodingReport => &CODING_REPORT_FIELDS,
        ArtifactKind::TestingReport => &TESTING_REPORT_FIELDS,
        ArtifactKind::CodeReviewReport => &CODE_REVIEW_REPORT_FIELDS,
        ArtifactKind::IntegrationReport => &INTEGRATION_REPORT_FIELDS,
        ArtifactKind::FinalReview => &FINAL_REVIEW_FIELDS,
        ArtifactKind::FinalSummary => &FINAL_SUMMARY_FIELDS,
        ArtifactKind::RuntimeSnapshot => &RUNTIME_SNAPSHOT_FIELDS,
        ArtifactKind::Spec | ArtifactKind::Design | ArtifactKind::Plan => &[],
    }
}

const fn field(name: &'static str, kind: RequiredFieldKind) -> RequiredField {
    RequiredField { name, kind }
}

const INTAKE_BRIEF_FIELDS: [RequiredField; 5] = [
    field("request_summary", RequiredFieldKind::String),
    field("raw_user_request", RequiredFieldKind::String),
    field("repo_context", RequiredFieldKind::Object),
    field("initial_constraints", RequiredFieldKind::Array),
    field("requested_goal", RequiredFieldKind::String),
];

const CLARIFICATION_RECORD_FIELDS: [RequiredField; 5] = [
    field("goal_summary", RequiredFieldKind::String),
    field("constraints", RequiredFieldKind::Array),
    field("assumptions", RequiredFieldKind::Array),
    field("open_questions", RequiredFieldKind::Array),
    field("suggested_scope", RequiredFieldKind::String),
];

const SPEC_GATE_DECISION_FIELDS: [RequiredField; 2] = [
    field("decision", RequiredFieldKind::String),
    field("review_notes", RequiredFieldKind::Array),
];

const DESIGN_REVIEW_FIELDS: [RequiredField; 2] = [
    field("review_decision", RequiredFieldKind::String),
    field("findings", RequiredFieldKind::Array),
];

const DESIGN_REVISION_RECORD_FIELDS: [RequiredField; 2] = [
    field("revision_summary", RequiredFieldKind::String),
    field("resolved_findings", RequiredFieldKind::Array),
];

const READINESS_CHECK_FIELDS: [RequiredField; 2] = [
    field("ready", RequiredFieldKind::Bool),
    field("blocking_items", RequiredFieldKind::Array),
];

const DISPATCH_PACKAGE_FIELDS: [RequiredField; 1] =
    [field("worktask_routing", RequiredFieldKind::Array)];

const CODING_REPORT_FIELDS: [RequiredField; 5] = [
    field("worktask_id", RequiredFieldKind::String),
    field("files_modified", RequiredFieldKind::Array),
    field("commands_run", RequiredFieldKind::Array),
    field("candidate_traceability_refs", RequiredFieldKind::Array),
    field("status", RequiredFieldKind::String),
];

const TESTING_REPORT_FIELDS: [RequiredField; 5] = [
    field("worktask_id", RequiredFieldKind::String),
    field("commands_run", RequiredFieldKind::Array),
    field("tests_passed", RequiredFieldKind::Bool),
    field("failures", RequiredFieldKind::Array),
    field("candidate_traceability_refs", RequiredFieldKind::Array),
];

const CODE_REVIEW_REPORT_FIELDS: [RequiredField; 4] = [
    field("worktask_id", RequiredFieldKind::String),
    field("findings", RequiredFieldKind::Array),
    field("blocking", RequiredFieldKind::Bool),
    field("candidate_traceability_refs", RequiredFieldKind::Array),
];

const INTEGRATION_REPORT_FIELDS: [RequiredField; 2] = [
    field("integrated_worktasks", RequiredFieldKind::Array),
    field("status", RequiredFieldKind::String),
];

const FINAL_REVIEW_FIELDS: [RequiredField; 4] = [
    field("overall_decision", RequiredFieldKind::String),
    field("coverage_summary", RequiredFieldKind::Object),
    field("uncovered_items", RequiredFieldKind::Array),
    field("followup_required", RequiredFieldKind::Bool),
];

const FINAL_SUMMARY_FIELDS: [RequiredField; 3] = [
    field("overall_status", RequiredFieldKind::String),
    field("next_steps", RequiredFieldKind::Array),
    field("remaining_risks", RequiredFieldKind::Array),
];

const RUNTIME_SNAPSHOT_FIELDS: [RequiredField; 3] = [
    field("phase", RequiredFieldKind::String),
    field("timestamp", RequiredFieldKind::String),
    field("risk_registry", RequiredFieldKind::Object),
];
