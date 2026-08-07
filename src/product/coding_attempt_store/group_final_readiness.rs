use crate::product::coding_models::{
    CodingExecutionAttempt, GroupFinalReadinessSnapshot, GroupFinalReadinessStatus,
};
use crate::product::json_store::{
    ProductStoreError, read_json, validate_relative_artifact_ref, validate_relative_id, write_json,
};
use crate::product::models::PlanDefectEvidence;

impl super::CodingAttemptStore {
    pub fn write_group_final_readiness_snapshot(
        &self,
        attempt: &CodingExecutionAttempt,
        snapshot: &GroupFinalReadinessSnapshot,
    ) -> Result<(), ProductStoreError> {
        let stored_attempt = self.validate_attempt_lineage(attempt)?;
        validate_snapshot(snapshot)?;
        if snapshot.attempt_id != stored_attempt.id {
            return Err(identity_mismatch(&snapshot.attempt_id));
        }
        write_json(
            &self.group_final_readiness_snapshot_path(&stored_attempt),
            snapshot,
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
        validate_snapshot(&snapshot)?;
        if snapshot.attempt_id != stored_attempt.id {
            return Err(identity_mismatch(&snapshot.attempt_id));
        }
        Ok(Some(snapshot))
    }
}

fn validate_snapshot(snapshot: &GroupFinalReadinessSnapshot) -> Result<(), ProductStoreError> {
    validate_relative_id(&snapshot.attempt_id)?;
    for unit in &snapshot.units {
        for id in [
            unit.unit_id.as_str(),
            unit.logical_work_item_id.as_str(),
            unit.unit_run_id.as_str(),
            unit.start_commit.as_str(),
            unit.completion_commit.as_str(),
        ] {
            validate_relative_id(id)?;
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
        for artifact_ref in [unit.review_raw_ref.as_deref()].into_iter().flatten() {
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
            return Err(identity_mismatch(&snapshot.attempt_id));
        }
        if let Some(unit_id) = diagnostic.unit_id.as_deref() {
            validate_relative_id(unit_id)?;
        }
    }
    if snapshot.status == GroupFinalReadinessStatus::Incomplete && snapshot.diagnostics.is_empty() {
        return Err(identity_mismatch(&snapshot.attempt_id));
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
    Ok(())
}

fn validate_plan_defect_evidence(evidence: &PlanDefectEvidence) -> Result<(), ProductStoreError> {
    validate_relative_artifact_ref(&evidence.source_ref)
}

fn identity_mismatch(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: "group_final_readiness_snapshot",
        id: id.to_string(),
    }
}
