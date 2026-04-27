use crate::protocol::projections::PlanProjection;
use crate::protocol::traceability::{
    ArtifactTraceabilityBinding, BindingStatus, CoverageStatus, CoverageSummary, ManualExemption,
};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashSet;

pub type WorkTaskId = String;
pub type WorkPackageId = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceabilityIndexes {
    known_ref_ids: HashSet<String>,
}

impl TraceabilityIndexes {
    pub fn new(known_ref_ids: Vec<String>) -> Self {
        Self {
            known_ref_ids: known_ref_ids
                .into_iter()
                .map(|ref_id| normalize_ref_id(&ref_id))
                .collect(),
        }
    }

    fn contains(&self, ref_id: &str) -> bool {
        self.known_ref_ids.contains(&normalize_ref_id(ref_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceabilityError {
    WorktaskMissing,
    WorktaskRoutingMissing(WorkTaskId),
    WorkPackageNotFound(WorkPackageId),
    UnknownRef { ref_id: String },
    CategoryMismatch { expected: String, got: String },
    BindingWriteFailed(String),
    CheckpointTransactionFailed(String),
}

pub fn normalize_traceability(
    report: &mut Value,
    provider_candidate_refs: Vec<String>,
    dispatch_package: &Value,
    plan_projection: &PlanProjection,
    indexes: &TraceabilityIndexes,
) -> Result<ArtifactTraceabilityBinding, TraceabilityError> {
    let worktask_id = report
        .get("worktask_id")
        .and_then(Value::as_str)
        .or_else(|| {
            report
                .get("_aria")
                .and_then(|aria| aria.get("worktask_id"))
                .and_then(Value::as_str)
        })
        .filter(|value| !value.trim().is_empty())
        .ok_or(TraceabilityError::WorktaskMissing)?
        .to_string();

    let routing = find_routing(dispatch_package, &worktask_id)
        .ok_or_else(|| TraceabilityError::WorktaskRoutingMissing(worktask_id.clone()))?;
    let source_work_package_id = routing
        .get("source_work_package_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| TraceabilityError::WorkPackageNotFound(String::new()))?;
    let work_package = plan_projection
        .work_packages
        .iter()
        .find(|work_package| work_package.work_package_id == source_work_package_id)
        .ok_or_else(|| {
            TraceabilityError::WorkPackageNotFound(source_work_package_id.to_string())
        })?;

    let expected_refs = work_package.traceability_refs.clone();
    let mut accepted_candidate_refs = Vec::new();
    let mut rejected_refs = Vec::new();
    let mut reason_codes = Vec::new();

    for candidate_ref in provider_candidate_refs {
        let normalized = normalize_ref_id(&candidate_ref);
        if indexes.contains(&normalized) {
            accepted_candidate_refs.push(normalized);
        } else {
            rejected_refs.push(normalized);
            push_unique(&mut reason_codes, "unknown_ref".to_string());
        }
    }

    let normalized_refs = stable_dedupe(
        expected_refs
            .iter()
            .map(|value| normalize_ref_id(value))
            .chain(accepted_candidate_refs.iter().cloned())
            .collect(),
    );
    write_traceability_refs(report, &normalized_refs);

    let artifact_ref = report
        .get("artifact_ref")
        .and_then(Value::as_str)
        .unwrap_or("artifact_ref_unknown")
        .to_string();
    let conflict_reason = (!rejected_refs.is_empty()).then(|| {
        json!({
            "binding_id": binding_id_for(&artifact_ref),
            "artifact_ref": artifact_ref,
            "worktask_id": worktask_id,
            "source_work_package_id": source_work_package_id,
            "expected_refs": expected_refs,
            "candidate_refs": accepted_candidate_refs.iter().chain(rejected_refs.iter()).cloned().collect::<Vec<_>>(),
            "accepted_refs": accepted_candidate_refs,
            "rejected_refs": rejected_refs,
            "reason_codes": reason_codes,
            "created_at": Utc::now().to_rfc3339(),
        })
        .to_string()
    });

    Ok(ArtifactTraceabilityBinding {
        binding_id: binding_id_for(&artifact_ref),
        canonical_artifact_ref: artifact_ref.clone(),
        projection_ref: "plan_projection".to_string(),
        related_requirement_ids: filter_refs_by_prefix(&normalized_refs, "req-"),
        related_design_decision_ids: filter_refs_by_prefix(&normalized_refs, "dd-"),
        related_task_ids: filter_refs_by_prefix(&normalized_refs, "task-"),
        related_risk_ids: filter_refs_by_prefix(&normalized_refs, "risk-"),
        evidence_artifact_refs: vec![artifact_ref],
        coverage_status: derive_coverage_status(&normalized_refs, &[worktask_id]),
        binding_status: if conflict_reason.is_some() {
            BindingStatus::Conflict
        } else {
            BindingStatus::Normalized
        },
        conflict_reason,
    })
}

pub fn derive_coverage_status(
    normalized_refs: &[String],
    evidence_refs: &[String],
) -> CoverageStatus {
    if normalized_refs.is_empty() || evidence_refs.is_empty() {
        CoverageStatus::Uncovered
    } else {
        CoverageStatus::Covered
    }
}

pub fn check_coverage_closed(
    requirement_ids: &[String],
    design_decision_ids: &[String],
    task_ids: &[String],
    bindings: &[ArtifactTraceabilityBinding],
    manual_exemptions: &[ManualExemption],
) -> CoverageSummary {
    let covered: HashSet<String> = bindings
        .iter()
        .flat_map(|binding| {
            binding
                .related_requirement_ids
                .iter()
                .chain(binding.related_design_decision_ids.iter())
                .chain(binding.related_task_ids.iter())
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect();
    let exempted: HashSet<String> = manual_exemptions
        .iter()
        .map(|exemption| exemption.item_id.clone())
        .collect();

    let mut closed = Vec::new();
    let mut uncovered = Vec::new();
    for item_id in requirement_ids
        .iter()
        .chain(design_decision_ids.iter())
        .chain(task_ids.iter())
    {
        if covered.contains(item_id) {
            closed.push(item_id.clone());
        } else if !exempted.contains(item_id) {
            uncovered.push(item_id.clone());
        }
    }
    closed.sort();
    uncovered.sort();
    let mut exempted_vec: Vec<String> = exempted.into_iter().collect();
    exempted_vec.sort();

    CoverageSummary {
        closed,
        uncovered,
        exempted: exempted_vec,
        manual_exemptions: manual_exemptions.to_vec(),
    }
}

fn find_routing<'a>(dispatch_package: &'a Value, worktask_id: &str) -> Option<&'a Value> {
    dispatch_package
        .get("_aria")
        .and_then(|aria| aria.get("worktask_routing"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("worktask_id").and_then(Value::as_str) == Some(worktask_id))
        })
}

fn write_traceability_refs(report: &mut Value, normalized_refs: &[String]) {
    if !report.get("_aria").is_some_and(Value::is_object) {
        report["_aria"] = json!({});
    }
    report["_aria"]["traceability_refs"] = json!(normalized_refs);
}

fn binding_id_for(artifact_ref: &str) -> String {
    format!("bind_{artifact_ref}")
}

fn filter_refs_by_prefix(refs: &[String], prefix: &str) -> Vec<String> {
    refs.iter()
        .filter(|ref_id| ref_id.starts_with(prefix))
        .cloned()
        .collect()
}

fn stable_dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            output.push(value);
        }
    }
    output
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn normalize_ref_id(value: &str) -> String {
    value
        .trim()
        .trim_matches(',')
        .trim_matches(';')
        .to_ascii_lowercase()
        .replace('_', "-")
}
