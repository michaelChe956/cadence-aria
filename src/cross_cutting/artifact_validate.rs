use crate::cross_cutting::document_ops::compute_sha256;
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef, ArtifactStatus, ProjectionKind};
use crate::protocol::enums::{ArtifactRefId, ConstraintBundleId, ProjectionId};
use crate::protocol::phase1_profile::PHASE1_PROFILE_VERSION;
use crate::protocol::projections::{
    ArtifactProjectionRecord, ProjectionPayload, WorkDependencyProjection, WorkPackageProjection,
};
use serde_json::Value;
use std::path::Path;

pub type WorkPackageId = String;

#[derive(Debug, Clone, PartialEq)]
pub enum ArtifactContent {
    Markdown(String),
    Json(Value),
    JsonText(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationResult {
    pub valid: bool,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    fn valid() -> Self {
        Self {
            valid: true,
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactValidationRule {
    pub artifact_kind: ArtifactKind,
    pub requires_canonical: bool,
    pub requires_projection: bool,
    pub requires_phase1_profile: bool,
    pub content_family: ArtifactContentFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactContentFamily {
    Markdown,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArtifactIndex {
    pub active_artifact_ref_ids: Vec<ArtifactRefId>,
    pub superseded_artifact_ref_ids: Vec<ArtifactRefId>,
    pub artifact_refs: Vec<ArtifactRef>,
}

impl ArtifactIndex {
    pub fn from_active_refs(artifact_refs: Vec<ArtifactRef>) -> Self {
        let active_artifact_ref_ids = artifact_refs
            .iter()
            .filter(|artifact_ref| artifact_ref.status == ArtifactStatus::Active)
            .map(|artifact_ref| artifact_ref.artifact_ref_id.clone())
            .collect();
        let superseded_artifact_ref_ids = artifact_refs
            .iter()
            .filter(|artifact_ref| artifact_ref.status == ArtifactStatus::Superseded)
            .map(|artifact_ref| artifact_ref.artifact_ref_id.clone())
            .collect();
        Self {
            active_artifact_ref_ids,
            superseded_artifact_ref_ids,
            artifact_refs,
        }
    }

    pub fn with_superseded_refs(
        artifact_refs: Vec<ArtifactRef>,
        superseded_artifact_ref_ids: Vec<ArtifactRefId>,
    ) -> Self {
        let mut index = Self::from_active_refs(artifact_refs);
        for artifact_ref_id in superseded_artifact_ref_ids {
            if !index.superseded_artifact_ref_ids.contains(&artifact_ref_id) {
                index.superseded_artifact_ref_ids.push(artifact_ref_id);
            }
        }
        index
    }

    fn active_ref(&self, artifact_ref_id: &str) -> Option<&ArtifactRef> {
        if self.is_superseded(artifact_ref_id) {
            return None;
        }
        self.artifact_refs.iter().find(|artifact_ref| {
            artifact_ref.artifact_ref_id == artifact_ref_id
                && artifact_ref.status == ArtifactStatus::Active
        })
    }

    fn is_superseded(&self, artifact_ref_id: &str) -> bool {
        self.superseded_artifact_ref_ids
            .iter()
            .any(|superseded| superseded == artifact_ref_id)
            || self.artifact_refs.iter().any(|artifact_ref| {
                artifact_ref.artifact_ref_id == artifact_ref_id
                    && artifact_ref.status == ArtifactStatus::Superseded
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionIndex {
    pub projection_ids: Vec<ProjectionId>,
    pub work_package_ids: Vec<WorkPackageId>,
}

impl ProjectionIndex {
    pub fn with_work_packages(
        projection_ids: Vec<ProjectionId>,
        work_package_ids: Vec<WorkPackageId>,
    ) -> Self {
        Self {
            projection_ids,
            work_package_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConstraintBundleIndex {
    pub constraint_bundle_ids: Vec<ConstraintBundleId>,
    pub constraint_check_ids: Vec<String>,
}

impl ConstraintBundleIndex {
    pub fn with_checks(constraint_check_ids: Vec<String>) -> Self {
        Self {
            constraint_bundle_ids: Vec::new(),
            constraint_check_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TraceabilityIndex {
    pub traceability_ref_ids: Vec<String>,
}

impl TraceabilityIndex {
    pub fn with_known_refs(traceability_ref_ids: Vec<String>) -> Self {
        Self {
            traceability_ref_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderRunIndex {
    pub provider_run_ids: Vec<String>,
}

impl ProviderRunIndex {
    pub fn with_runs(provider_run_ids: Vec<String>) -> Self {
        Self { provider_run_ids }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactValidateError {
    InvalidInputSuperseded(ArtifactRefId),
    CanonicalMissingField {
        field: String,
        artifact_kind: ArtifactKind,
    },
    CanonicalTypeMismatch {
        field: String,
        expected: String,
        got: String,
    },
    ProjectionMissingField {
        field: String,
        projection_id: ProjectionId,
    },
    ProjectionInvalidId {
        id: String,
        reason: String,
    },
    ProjectionSourceNotFound(ArtifactRefId),
    ProjectionSourceHashMismatch {
        expected: String,
        got: String,
    },
    ProjectionReferenceUnknown {
        ref_id: String,
        context: String,
    },
    ProjectionPayloadEmpty(ProjectionKind),
    ProfileMissingAria,
    ProfileVersionMissing,
    ProfileProjectionRefUnknown(ProjectionId),
    ProfileConstraintRefUnknown(ConstraintBundleId),
    TraceabilityRefsMissing,
    TraceabilityRefUnknown(String),
    WorktaskRoutingSourceUnknown(WorkPackageId),
    CoverageSummaryMissing,
}

pub fn artifact_validation_matrix() -> Vec<ArtifactValidationRule> {
    ArtifactKind::all_phase1()
        .map(|artifact_kind| artifact_validation_rule(artifact_kind).expect("known artifact kind"))
        .collect()
}

pub fn artifact_validation_rule(artifact_kind: ArtifactKind) -> Option<ArtifactValidationRule> {
    let requires_projection = matches!(
        artifact_kind,
        ArtifactKind::Spec | ArtifactKind::Design | ArtifactKind::Plan
    );
    let requires_phase1_profile = matches!(
        artifact_kind,
        ArtifactKind::DispatchPackage
            | ArtifactKind::CodingReport
            | ArtifactKind::TestingReport
            | ArtifactKind::CodeReviewReport
            | ArtifactKind::IntegrationReport
            | ArtifactKind::FinalReview
            | ArtifactKind::FinalSummary
    );
    Some(ArtifactValidationRule {
        artifact_kind,
        requires_canonical: true,
        requires_projection,
        requires_phase1_profile,
        content_family: if artifact_kind.is_markdown_canonical() {
            ArtifactContentFamily::Markdown
        } else {
            ArtifactContentFamily::Json
        },
    })
}

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

pub fn validate_input_artifact_ref(
    artifact_ref: &ArtifactRef,
    artifact_index: &ArtifactIndex,
) -> Result<ValidationResult, ArtifactValidateError> {
    if artifact_ref.status == ArtifactStatus::Superseded
        || artifact_index.is_superseded(&artifact_ref.artifact_ref_id)
    {
        return Err(ArtifactValidateError::InvalidInputSuperseded(
            artifact_ref.artifact_ref_id.clone(),
        ));
    }
    if !artifact_index
        .active_artifact_ref_ids
        .contains(&artifact_ref.artifact_ref_id)
    {
        return Err(ArtifactValidateError::ProjectionSourceNotFound(
            artifact_ref.artifact_ref_id.clone(),
        ));
    }
    Ok(ValidationResult::valid())
}

pub fn record_superseded_artifact_ref(
    task_runtime_state: &mut Value,
    artifact_ref_id: ArtifactRefId,
) -> Result<ValidationResult, ArtifactValidateError> {
    let state_type = json_type_name(task_runtime_state);
    let object = task_runtime_state.as_object_mut().ok_or_else(|| {
        ArtifactValidateError::CanonicalTypeMismatch {
            field: "$".to_string(),
            expected: "json_object".to_string(),
            got: state_type.to_string(),
        }
    })?;
    let refs = object
        .entry("superseded_artifact_refs")
        .or_insert_with(|| Value::Array(Vec::new()));
    let refs_type = json_type_name(refs);
    let refs = refs
        .as_array_mut()
        .ok_or_else(|| ArtifactValidateError::CanonicalTypeMismatch {
            field: "superseded_artifact_refs".to_string(),
            expected: "array".to_string(),
            got: refs_type.to_string(),
        })?;
    if !refs
        .iter()
        .any(|existing| existing.as_str() == Some(artifact_ref_id.as_str()))
    {
        refs.push(Value::String(artifact_ref_id));
    }
    Ok(ValidationResult::valid())
}

pub fn projection_validator(
    record: &ArtifactProjectionRecord,
    artifact_index: &ArtifactIndex,
    golden_fixture: Option<&Path>,
) -> Result<ValidationResult, ArtifactValidateError> {
    if record.projection_id.trim().is_empty() {
        return Err(ArtifactValidateError::ProjectionMissingField {
            field: "projection_id".to_string(),
            projection_id: record.projection_id.clone(),
        });
    }
    let expected_prefix = format!(
        "proj_{}_{}_",
        record.projection_kind.as_str(),
        record.source_artifact_ref.artifact_id
    );
    if !record.projection_id.starts_with(&expected_prefix) {
        return Err(ArtifactValidateError::ProjectionInvalidId {
            id: record.projection_id.clone(),
            reason: format!("expected prefix {expected_prefix}"),
        });
    }
    if record.payload.projection_kind() != record.projection_kind {
        return Err(ArtifactValidateError::ProjectionInvalidId {
            id: record.projection_id.clone(),
            reason: "projection_kind does not match payload".to_string(),
        });
    }
    if record.source_artifact_ref.status == ArtifactStatus::Superseded
        || artifact_index.is_superseded(&record.source_artifact_ref.artifact_ref_id)
    {
        return Err(ArtifactValidateError::InvalidInputSuperseded(
            record.source_artifact_ref.artifact_ref_id.clone(),
        ));
    }
    let active_ref = artifact_index
        .active_ref(&record.source_artifact_ref.artifact_ref_id)
        .or_else(|| {
            artifact_index
                .active_artifact_ref_ids
                .contains(&record.source_artifact_ref.artifact_ref_id)
                .then_some(&record.source_artifact_ref)
        })
        .ok_or_else(|| {
            ArtifactValidateError::ProjectionSourceNotFound(
                record.source_artifact_ref.artifact_ref_id.clone(),
            )
        })?;
    if record.source_artifact_version != active_ref.version {
        return Err(ArtifactValidateError::ProjectionReferenceUnknown {
            ref_id: active_ref.artifact_ref_id.clone(),
            context: "source_artifact_version".to_string(),
        });
    }
    let current_hash = if !active_ref.path.is_empty() && Path::new(&active_ref.path).exists() {
        std::fs::read(&active_ref.path)
            .map(|content| compute_sha256(&content))
            .unwrap_or_else(|_| active_ref.sha256.clone())
    } else {
        active_ref.sha256.clone()
    };
    if record.source_artifact_hash != current_hash {
        return Err(ArtifactValidateError::ProjectionSourceHashMismatch {
            expected: current_hash,
            got: record.source_artifact_hash.clone(),
        });
    }
    if record.compiled_at.trim().is_empty() {
        return Err(ArtifactValidateError::ProjectionMissingField {
            field: "compiled_at".to_string(),
            projection_id: record.projection_id.clone(),
        });
    }
    if record.compiled_by_node.trim().is_empty() {
        return Err(ArtifactValidateError::ProjectionMissingField {
            field: "compiled_by_node".to_string(),
            projection_id: record.projection_id.clone(),
        });
    }
    if record.payload.is_empty() {
        return Err(ArtifactValidateError::ProjectionPayloadEmpty(
            record.projection_kind,
        ));
    }
    validate_projection_payload(&record.payload)?;
    if let Some(path) = golden_fixture {
        let golden: Value = serde_json::from_slice(&std::fs::read(path).map_err(|_| {
            ArtifactValidateError::ProjectionReferenceUnknown {
                ref_id: path.display().to_string(),
                context: "golden_fixture".to_string(),
            }
        })?)
        .map_err(|_| ArtifactValidateError::ProjectionReferenceUnknown {
            ref_id: path.display().to_string(),
            context: "golden_fixture_json".to_string(),
        })?;
        if golden
            != record.payload.inner_json().map_err(|_| {
                ArtifactValidateError::ProjectionReferenceUnknown {
                    ref_id: record.projection_id.clone(),
                    context: "projection_payload_json".to_string(),
                }
            })?
        {
            return Err(ArtifactValidateError::ProjectionReferenceUnknown {
                ref_id: record.projection_id.clone(),
                context: "golden_fixture_mismatch".to_string(),
            });
        }
    }
    Ok(ValidationResult::valid())
}

fn validate_projection_payload(payload: &ProjectionPayload) -> Result<(), ArtifactValidateError> {
    match payload {
        ProjectionPayload::SpecProjection(spec) => {
            if spec.functional_requirements.is_empty() {
                return Err(ArtifactValidateError::ProjectionPayloadEmpty(
                    ProjectionKind::SpecProjection,
                ));
            }
            let known_requirements: Vec<&str> = spec
                .functional_requirements
                .iter()
                .map(|requirement| requirement.requirement_id.as_str())
                .collect();
            for criterion in &spec.success_criteria {
                for ref_id in &criterion.related_requirement_ids {
                    if !known_requirements.contains(&ref_id.as_str()) {
                        return Err(ArtifactValidateError::ProjectionReferenceUnknown {
                            ref_id: ref_id.clone(),
                            context: "success_criteria".to_string(),
                        });
                    }
                }
            }
            Ok(())
        }
        ProjectionPayload::DesignProjection(design) => {
            if design.design_decisions.is_empty() {
                return Err(ArtifactValidateError::ProjectionPayloadEmpty(
                    ProjectionKind::DesignProjection,
                ));
            }
            Ok(())
        }
        ProjectionPayload::PlanProjection(plan) => {
            if plan.work_packages.is_empty() {
                return Err(ArtifactValidateError::ProjectionPayloadEmpty(
                    ProjectionKind::PlanProjection,
                ));
            }
            validate_plan_projection(&plan.work_packages, &plan.dependencies)
        }
    }
}

fn validate_plan_projection(
    work_packages: &[WorkPackageProjection],
    dependencies: &[WorkDependencyProjection],
) -> Result<(), ArtifactValidateError> {
    let mut known = Vec::new();
    for work_package in work_packages {
        if known.contains(&work_package.work_package_id) {
            return Err(ArtifactValidateError::ProjectionInvalidId {
                id: work_package.work_package_id.clone(),
                reason: "duplicate work_package_id".to_string(),
            });
        }
        known.push(work_package.work_package_id.clone());
        if work_package.traceability_refs.is_empty() {
            return Err(ArtifactValidateError::ProjectionReferenceUnknown {
                ref_id: work_package.work_package_id.clone(),
                context: "traceability_refs".to_string(),
            });
        }
    }
    for dependency in dependencies {
        if !known.contains(&dependency.from_work_package_id)
            || !known.contains(&dependency.to_work_package_id)
        {
            return Err(ArtifactValidateError::ProjectionReferenceUnknown {
                ref_id: format!(
                    "{}->{}",
                    dependency.from_work_package_id, dependency.to_work_package_id
                ),
                context: "dependencies".to_string(),
            });
        }
    }
    Ok(())
}

pub fn phase1_profile_validator(
    artifact_value: &Value,
    artifact_kind: ArtifactKind,
    projection_index: &ProjectionIndex,
    constraint_bundle_index: &ConstraintBundleIndex,
    traceability_index: &TraceabilityIndex,
    provider_run_index: &ProviderRunIndex,
) -> Result<ValidationResult, ArtifactValidateError> {
    let aria = artifact_value
        .get("_aria")
        .and_then(Value::as_object)
        .ok_or(ArtifactValidateError::ProfileMissingAria)?;
    let profile_version = aria
        .get("profile_version")
        .and_then(Value::as_str)
        .ok_or(ArtifactValidateError::ProfileVersionMissing)?;
    if profile_version != PHASE1_PROFILE_VERSION {
        return Err(ArtifactValidateError::ProfileVersionMissing);
    }
    let constraint_check_ref = aria
        .get("constraint_check_ref")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if constraint_check_ref.is_empty()
        || (!constraint_bundle_index
            .constraint_check_ids
            .contains(&constraint_check_ref.to_string())
            && !constraint_bundle_index
                .constraint_bundle_ids
                .contains(&constraint_check_ref.to_string()))
    {
        return Err(ArtifactValidateError::ProfileConstraintRefUnknown(
            constraint_check_ref.to_string(),
        ));
    }

    let traceability_refs = aria
        .get("traceability_refs")
        .and_then(Value::as_array)
        .ok_or(ArtifactValidateError::TraceabilityRefsMissing)?;
    for ref_value in traceability_refs {
        let ref_id = ref_value
            .as_str()
            .ok_or_else(|| ArtifactValidateError::TraceabilityRefUnknown(ref_value.to_string()))?;
        if !traceability_index
            .traceability_ref_ids
            .iter()
            .any(|known| known == ref_id)
        {
            return Err(ArtifactValidateError::TraceabilityRefUnknown(
                ref_id.to_string(),
            ));
        }
    }
    if let Some(provider_run_refs) = aria.get("provider_run_refs").and_then(Value::as_array) {
        for provider_run_ref in provider_run_refs {
            let run_id = provider_run_ref.as_str().unwrap_or_default().to_string();
            if !provider_run_index.provider_run_ids.contains(&run_id) {
                return Err(ArtifactValidateError::TraceabilityRefUnknown(run_id));
            }
        }
    }
    if let Some(projection_refs) = aria.get("projection_refs").and_then(Value::as_array) {
        for projection_ref in projection_refs {
            let projection_id = projection_ref.as_str().unwrap_or_default().to_string();
            if !projection_index.projection_ids.contains(&projection_id) {
                return Err(ArtifactValidateError::ProfileProjectionRefUnknown(
                    projection_id,
                ));
            }
        }
    }

    match artifact_kind {
        ArtifactKind::DispatchPackage => {
            validate_worktask_routing(
                aria.get("worktask_routing"),
                &projection_index.work_package_ids,
            )?;
        }
        ArtifactKind::FinalReview => {
            validate_coverage_summary(aria.get("coverage_summary"))?;
        }
        ArtifactKind::CodingReport
        | ArtifactKind::TestingReport
        | ArtifactKind::CodeReviewReport
        | ArtifactKind::IntegrationReport
            if traceability_refs.is_empty() =>
        {
            return Err(ArtifactValidateError::TraceabilityRefsMissing);
        }
        _ => {}
    }
    Ok(ValidationResult::valid())
}

fn validate_worktask_routing(
    value: Option<&Value>,
    known_work_package_ids: &[WorkPackageId],
) -> Result<(), ArtifactValidateError> {
    let routing = value
        .and_then(Value::as_array)
        .ok_or_else(|| ArtifactValidateError::WorktaskRoutingSourceUnknown(String::new()))?;
    for item in routing {
        let source_work_package_id = item
            .get("source_work_package_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !known_work_package_ids.contains(&source_work_package_id) {
            return Err(ArtifactValidateError::WorktaskRoutingSourceUnknown(
                source_work_package_id,
            ));
        }
        for required in [
            "worktask_id",
            "execution_mode",
            "allowed_write_scope",
            "traceability_refs",
            "verification_commands",
        ] {
            if item.get(required).is_none() {
                return Err(ArtifactValidateError::WorktaskRoutingSourceUnknown(
                    source_work_package_id,
                ));
            }
        }
    }
    Ok(())
}

fn validate_coverage_summary(value: Option<&Value>) -> Result<(), ArtifactValidateError> {
    let Some(summary) = value.and_then(Value::as_object) else {
        return Err(ArtifactValidateError::CoverageSummaryMissing);
    };
    for required in ["closed", "uncovered", "exempted"] {
        if !summary.get(required).is_some_and(Value::is_array) {
            return Err(ArtifactValidateError::CoverageSummaryMissing);
        }
    }
    Ok(())
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
    let required_heading = match artifact_kind {
        ArtifactKind::Spec => "功能需求",
        ArtifactKind::Design => "设计决策",
        ArtifactKind::Plan => "工作包",
        _ => return Ok(ValidationResult::valid()),
    };
    let has_required_heading = markdown.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with('#') && trimmed.contains(required_heading)
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

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
