use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, GroupFinalReadinessSnapshot, GroupFinalReadinessStatus,
};
use crate::product::json_store::{
    ProductStoreError, read_json, validate_relative_artifact_ref, validate_relative_id, write_json,
};
use crate::product::models::PlanDefectEvidence;

const SNAPSHOT_KIND: &str = "group_final_readiness_snapshot";

impl super::CodingAttemptStore {
    pub fn write_group_final_readiness_snapshot(
        &self,
        attempt: &CodingExecutionAttempt,
        snapshot: &GroupFinalReadinessSnapshot,
    ) -> Result<(), ProductStoreError> {
        let stored_attempt = self.validate_attempt_lineage(attempt)?;
        if snapshot.attempt_id != stored_attempt.id {
            return Err(identity_mismatch(&snapshot.attempt_id));
        }
        validate_snapshot(snapshot)?;
        let mut snapshot = snapshot.clone();
        snapshot.created_at = Utc::now().to_rfc3339();
        write_json(
            &self.group_final_readiness_snapshot_path(&stored_attempt),
            &snapshot,
        )
    }

    pub fn get_group_final_readiness_snapshot(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<GroupFinalReadinessSnapshot>, ProductStoreError> {
        let stored_attempt = self.validate_attempt_lineage(attempt)?;
        let path = self.group_final_readiness_snapshot_path(&stored_attempt);
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        let snapshot: GroupFinalReadinessSnapshot = read_json(&path)?;
        if snapshot.attempt_id != stored_attempt.id {
            return Err(identity_mismatch(&snapshot.attempt_id));
        }
        validate_snapshot(&snapshot)?;
        Ok(Some(snapshot))
    }
}

fn validate_snapshot(snapshot: &GroupFinalReadinessSnapshot) -> Result<(), ProductStoreError> {
    validate_relative_id(&snapshot.attempt_id)?;
    match snapshot.status {
        GroupFinalReadinessStatus::Complete if snapshot.units.is_empty() => {
            return Err(invalid_record(
                "complete snapshot must include at least one unit",
            ));
        }
        GroupFinalReadinessStatus::Incomplete if snapshot.diagnostics.is_empty() => {
            return Err(invalid_record(
                "incomplete snapshot must include diagnostics",
            ));
        }
        _ => {}
    }

    for unit in &snapshot.units {
        for id in [
            Some(unit.unit_id.as_str()),
            Some(unit.logical_work_item_id.as_str()),
            unit.unit_run_id.as_deref(),
            unit.start_commit.as_deref(),
            unit.completion_commit.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_relative_id(id)?;
        }
        if unit.empty_observation && (!unit.commit_shas.is_empty() || !unit.diff_ref.is_empty()) {
            return Err(invalid_record(format!(
                "empty observation unit {} must not include git range facts",
                unit.unit_id
            )));
        }
        if snapshot.status == GroupFinalReadinessStatus::Complete {
            validate_complete_unit(unit)?;
        }
        for commit_sha in &unit.commit_shas {
            validate_relative_id(commit_sha)?;
        }
        if !unit.diff_ref.is_empty() {
            validate_relative_artifact_ref(&unit.diff_ref)?;
        }
        for id in [
            unit.code_review_report_id.as_deref(),
            unit.handoff_revision_id.as_deref(),
            unit.plan_revision_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_relative_id(id)?;
        }
        for artifact_ref in [unit.review_raw_provider_output_ref.as_deref()]
            .into_iter()
            .flatten()
        {
            validate_relative_artifact_ref(artifact_ref)?;
        }
        if let Some(findings) = &unit.review_findings {
            for finding in findings {
                validate_review_finding_artifact_refs(finding)?;
            }
        }
    }
    for diagnostic in &snapshot.diagnostics {
        if diagnostic.message.trim().is_empty() {
            return Err(invalid_record("diagnostic message must not be empty"));
        }
        if let Some(unit_id) = diagnostic.unit_id.as_deref() {
            validate_relative_id(unit_id)?;
        }
    }
    Ok(())
}

fn validate_complete_unit(
    unit: &crate::product::coding_models::GroupFinalReadinessUnit,
) -> Result<(), ProductStoreError> {
    for (field, value) in [
        ("unit_run_id", unit.unit_run_id.is_some()),
        ("start_commit", unit.start_commit.is_some()),
        ("completion_commit", unit.completion_commit.is_some()),
        (
            "code_review_report_id",
            unit.code_review_report_id.is_some(),
        ),
        ("review_verdict", unit.review_verdict.is_some()),
        ("review_summary", unit.review_summary.is_some()),
        ("review_findings", unit.review_findings.is_some()),
        ("handoff_revision_id", unit.handoff_revision_id.is_some()),
        ("plan_revision_id", unit.plan_revision_id.is_some()),
    ] {
        if !value {
            return Err(invalid_record(format!(
                "complete unit {} is missing {field}",
                unit.unit_id
            )));
        }
    }
    Ok(())
}

fn validate_review_finding_artifact_refs(
    finding: &crate::product::coding_models::ReviewFinding,
) -> Result<(), ProductStoreError> {
    for artifact_ref in finding
        .evidence
        .iter()
        .chain(finding.contract_refs.iter())
        .chain(finding.capability_refs.iter())
    {
        validate_relative_artifact_ref(artifact_ref)?;
    }
    for evidence in &finding.plan_defect_evidence {
        validate_plan_defect_evidence(evidence)?;
    }
    if let Some(repair_target) = &finding.repair_target {
        for id in repair_target
            .logical_work_item_ids
            .iter()
            .chain(repair_target.work_item_revision_ids.iter())
        {
            validate_relative_id(id)?;
        }
    }
    Ok(())
}

fn validate_plan_defect_evidence(evidence: &PlanDefectEvidence) -> Result<(), ProductStoreError> {
    validate_relative_artifact_ref(&evidence.source_ref)
}

fn invalid_record(reason: impl Into<String>) -> ProductStoreError {
    ProductStoreError::InvalidRecord {
        kind: SNAPSHOT_KIND,
        reason: reason.into(),
    }
}

fn identity_mismatch(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: SNAPSHOT_KIND,
        id: id.to_string(),
    }
}
