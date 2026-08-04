use std::fs;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::coding_models::{
    CasOutcome, CodeReviewReport, CompactFindingDigest, GroupReviewReductionReport,
    GroupReviewShardReport, SnapshotRebuildError, UnitReviewConclusionSnapshot,
};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

impl super::CodingAttemptStore {
    pub fn claim_group_review_lease(
        &self,
        attempt_id: &str,
        snapshot_hash: &str,
        stage: &str,
        shard_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        validate_group_review_lease_key(attempt_id, snapshot_hash, stage, shard_id)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let root = group_review_root(self, &attempt);
        let active_path = root.join("active-snapshot.json");
        let lease_path = root.join("lease-state.json");
        with_exclusive_lock(&active_path, || {
            let mut state = read_group_review_lease_state(&lease_path)?;
            if state.leases.iter().any(|lease| {
                lease.snapshot_hash == snapshot_hash
                    && lease.stage == stage
                    && lease.shard_id == shard_id
            }) {
                return Ok(None);
            }
            let lease_id = format!("group_review_lease_{}", Uuid::new_v4().simple());
            state.leases.push(GroupReviewLease {
                lease_id: lease_id.clone(),
                snapshot_hash: snapshot_hash.to_string(),
                stage: stage.to_string(),
                shard_id: shard_id.to_string(),
                completed_result_ref: None,
            });
            write_json(&lease_path, &state)?;
            Ok(Some(lease_id))
        })
    }

    pub fn release_group_review_lease(
        &self,
        attempt_id: &str,
        lease_id: &str,
        result_ref: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(attempt_id)?;
        validate_relative_id(lease_id)?;
        if !result_ref.is_empty() {
            validate_relative_id(result_ref)?;
        }
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let root = group_review_root(self, &attempt);
        let active_path = root.join("active-snapshot.json");
        let lease_path = root.join("lease-state.json");
        with_exclusive_lock(&active_path, || {
            let mut state = read_group_review_lease_state(&lease_path)?;
            let Some(index) = state
                .leases
                .iter()
                .position(|lease| lease.lease_id == lease_id)
            else {
                return Ok(());
            };
            if result_ref.is_empty() {
                state.leases.remove(index);
            } else {
                state.leases[index].completed_result_ref = Some(result_ref.to_string());
            }
            write_json(&lease_path, &state)
        })
    }

    pub fn get_completed_group_review_result(
        &self,
        attempt_id: &str,
        snapshot_hash: &str,
        stage: &str,
        shard_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        validate_group_review_lease_key(attempt_id, snapshot_hash, stage, shard_id)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let root = group_review_root(self, &attempt);
        let active_path = root.join("active-snapshot.json");
        let lease_path = root.join("lease-state.json");
        with_exclusive_lock(&active_path, || {
            Ok(read_group_review_lease_state(&lease_path)?
                .leases
                .iter()
                .find(|lease| {
                    lease.snapshot_hash == snapshot_hash
                        && lease.stage == stage
                        && lease.shard_id == shard_id
                })
                .and_then(|lease| lease.completed_result_ref.clone()))
        })
    }

    pub fn activate_group_review_snapshot(
        &self,
        attempt_id: &str,
        snapshot_hash: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(attempt_id)?;
        validate_relative_id(snapshot_hash)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let active_path = group_review_root(self, &attempt).join("active-snapshot.json");
        with_exclusive_lock(&active_path, || {
            write_json(
                &active_path,
                &ActiveGroupReviewSnapshot {
                    content_hash: snapshot_hash.to_string(),
                },
            )
        })
    }

    pub fn get_active_group_review_snapshot_hash(
        &self,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let active_path = group_review_root(self, &attempt).join("active-snapshot.json");
        if !super::path_is_regular_file(&active_path)? {
            return Ok(None);
        }
        Ok(Some(
            read_json::<ActiveGroupReviewSnapshot>(&active_path)?.content_hash,
        ))
    }

    pub fn write_group_review_shard_report_cas(
        &self,
        attempt_id: &str,
        report: GroupReviewShardReport,
    ) -> Result<CasOutcome, ProductStoreError> {
        validate_group_review_report(&report.id, &report.attempt_id, attempt_id)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        write_group_review_report_cas(
            self,
            &attempt,
            &report.snapshot_hash,
            "shard-reports",
            &report.id,
            &report,
        )
    }

    pub fn write_group_review_reduction_report_cas(
        &self,
        attempt_id: &str,
        report: GroupReviewReductionReport,
    ) -> Result<CasOutcome, ProductStoreError> {
        validate_group_review_report(&report.id, &report.attempt_id, attempt_id)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        write_group_review_report_cas(
            self,
            &attempt,
            &report.snapshot_hash,
            "reduction-reports",
            &report.id,
            &report,
        )
    }

    pub fn list_group_review_shard_reports(
        &self,
        attempt_id: &str,
    ) -> Result<Vec<GroupReviewShardReport>, ProductStoreError> {
        self.list_group_review_reports(attempt_id, "shard-reports")
    }

    pub fn list_group_review_reduction_reports(
        &self,
        attempt_id: &str,
    ) -> Result<Vec<GroupReviewReductionReport>, ProductStoreError> {
        self.list_group_review_reports(attempt_id, "reduction-reports")
    }

    fn list_group_review_reports<T: serde::de::DeserializeOwned>(
        &self,
        attempt_id: &str,
        kind: &str,
    ) -> Result<Vec<T>, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let root = group_review_root(self, &attempt).join(kind);
        let mut reports = Vec::new();
        for path in super::json_file_paths(&root)? {
            reports.push(read_json(&path)?);
        }
        Ok(reports)
    }

    pub fn write_unit_review_conclusion_snapshot(
        &self,
        snapshot: &UnitReviewConclusionSnapshot,
    ) -> Result<(), ProductStoreError> {
        validate_snapshot(snapshot)?;
        let attempt = self.find_attempt_by_id(&snapshot.attempt_id)?;
        let path = self.unit_review_conclusion_snapshot_path(&attempt, &snapshot.unit_run_id);
        if super::path_is_regular_file(&path)? {
            let existing: UnitReviewConclusionSnapshot = read_json(&path)?;
            if existing.unit_run_id == snapshot.unit_run_id
                && existing.raw_report_hash == snapshot.raw_report_hash
            {
                return Ok(());
            }
            if existing.attempt_id != snapshot.attempt_id
                || existing.unit_run_id != snapshot.unit_run_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "unit_review_conclusion_snapshot",
                    id: snapshot.unit_run_id.clone(),
                });
            }
        }
        if attempt.id != snapshot.attempt_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "unit_review_conclusion_snapshot",
                id: snapshot.unit_run_id.clone(),
            });
        }
        write_json(&path, snapshot)
    }

    pub fn get_unit_review_conclusion_snapshot(
        &self,
        attempt_id: &str,
        unit_run_id: &str,
    ) -> Result<Option<UnitReviewConclusionSnapshot>, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        validate_relative_id(unit_run_id)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let path = self.unit_review_conclusion_snapshot_path(&attempt, unit_run_id);
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        let snapshot: UnitReviewConclusionSnapshot = read_json(&path)?;
        if snapshot.attempt_id != attempt_id || snapshot.unit_run_id != unit_run_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "unit_review_conclusion_snapshot",
                id: unit_run_id.to_string(),
            });
        }
        Ok(Some(snapshot))
    }

    pub(crate) fn write_snapshot_for_code_review_report(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        report: &CodeReviewReport,
        raw_report: &str,
    ) -> Result<(), ProductStoreError> {
        let Some(unit_run_id) = report.unit_run_id.as_deref() else {
            return Ok(());
        };
        self.validate_scoped_attempt_record(
            attempt,
            &report.attempt_id,
            "code_review_report",
            &report.id,
        )?;
        let (_, unit_run) = self
            .find_unit_run_by_id(attempt, unit_run_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_unit_run",
                id: unit_run_id.to_string(),
            })?;
        let unit = self.authoritative_unit(attempt, &unit_run.unit_id)?;
        if unit.work_item_revision_id != unit_run.work_item_revision_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "unit_review_conclusion_snapshot",
                id: unit_run_id.to_string(),
            });
        }
        let snapshot = snapshot_from_report(&attempt.id, &unit, unit_run_id, report, raw_report);
        self.write_unit_review_conclusion_snapshot(&snapshot)
    }

    pub fn rebuild_unit_review_conclusion_snapshot(
        &self,
        attempt_id: &str,
        unit_run_id: &str,
    ) -> Result<UnitReviewConclusionSnapshot, SnapshotRebuildError> {
        validate_relative_id(attempt_id)?;
        validate_relative_id(unit_run_id)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let (_, unit_run) = self
            .find_unit_run_by_id(&attempt, unit_run_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_unit_run",
                id: unit_run_id.to_string(),
            })?;
        let unit = self.authoritative_unit(&attempt, &unit_run.unit_id)?;
        let reports =
            self.list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let report = uniquely_matching_report(reports, unit_run_id)?;
        let Some(report_unit_run_id) = report.unit_run_id.as_deref() else {
            return Err(SnapshotRebuildError::MissingUnitRunId(report.id));
        };
        if report_unit_run_id != unit_run_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "code_review_report_unit_run",
                id: report.id,
            }
            .into());
        }
        if unit.work_item_revision_id != unit_run.work_item_revision_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "unit_review_conclusion_snapshot",
                id: unit_run_id.to_string(),
            }
            .into());
        }
        let raw_report = raw_report_text(self, &attempt, &report)?;
        let snapshot = snapshot_from_report(&attempt.id, &unit, unit_run_id, &report, &raw_report);
        self.write_unit_review_conclusion_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    pub(crate) fn delete_code_review_report(
        &self,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        report_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(report_id)?;
        let path = self.code_review_report_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            report_id,
        );
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ProductStoreError::Io(format!(
                "remove {}: {error}",
                path.display()
            ))),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct GroupReviewLeaseState {
    leases: Vec<GroupReviewLease>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GroupReviewLease {
    lease_id: String,
    snapshot_hash: String,
    stage: String,
    shard_id: String,
    completed_result_ref: Option<String>,
}

fn read_group_review_lease_state(
    path: &std::path::Path,
) -> Result<GroupReviewLeaseState, ProductStoreError> {
    if super::path_is_regular_file(path)? {
        read_json(path)
    } else {
        Ok(GroupReviewLeaseState::default())
    }
}

fn validate_group_review_lease_key(
    attempt_id: &str,
    snapshot_hash: &str,
    stage: &str,
    shard_id: &str,
) -> Result<(), ProductStoreError> {
    for value in [attempt_id, snapshot_hash, stage, shard_id] {
        validate_relative_id(value)?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct ActiveGroupReviewSnapshot {
    content_hash: String,
}

fn group_review_root(
    store: &super::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> std::path::PathBuf {
    store
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join("group-review")
}

fn write_group_review_report_cas<T: Serialize>(
    store: &super::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    report_snapshot_hash: &str,
    kind: &str,
    report_id: &str,
    report: &T,
) -> Result<CasOutcome, ProductStoreError> {
    validate_relative_id(report_snapshot_hash)?;
    let root = group_review_root(store, attempt);
    let active_path = root.join("active-snapshot.json");
    with_exclusive_lock(&active_path, || {
        let active_hash = if super::path_is_regular_file(&active_path)? {
            Some(read_json::<ActiveGroupReviewSnapshot>(&active_path)?.content_hash)
        } else {
            None
        };
        let (path, outcome) = if active_hash.as_deref() == Some(report_snapshot_hash) {
            (
                root.join(kind).join(format!("{report_id}.json")),
                CasOutcome::Written,
            )
        } else {
            (
                root.join("stale")
                    .join(kind)
                    .join(format!("{report_id}.json")),
                CasOutcome::StoredStale,
            )
        };
        write_json(&path, report)?;
        Ok(outcome)
    })
}

fn validate_group_review_report(
    report_id: &str,
    report_attempt_id: &str,
    attempt_id: &str,
) -> Result<(), ProductStoreError> {
    validate_relative_id(report_id)?;
    validate_relative_id(attempt_id)?;
    if report_attempt_id != attempt_id {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "group_review_report",
            id: report_id.to_string(),
        });
    }
    Ok(())
}

fn validate_snapshot(snapshot: &UnitReviewConclusionSnapshot) -> Result<(), ProductStoreError> {
    for id in [
        snapshot.attempt_id.as_str(),
        snapshot.unit_id.as_str(),
        snapshot.unit_run_id.as_str(),
        snapshot.logical_work_item_id.as_str(),
        snapshot.work_item_revision_id.as_str(),
        snapshot.code_review_report_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    if snapshot.raw_report_hash.is_empty() {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "unit_review_conclusion_snapshot",
            id: snapshot.unit_run_id.clone(),
        });
    }
    Ok(())
}

fn uniquely_matching_report(
    reports: Vec<CodeReviewReport>,
    unit_run_id: &str,
) -> Result<CodeReviewReport, SnapshotRebuildError> {
    let mut matching = reports
        .iter()
        .filter(|report| report.unit_run_id.as_deref() == Some(unit_run_id));
    if let Some(report) = matching.next() {
        if matching.next().is_some() {
            return Err(ProductStoreError::Ambiguous {
                kind: "code_review_report",
                id: unit_run_id.to_string(),
            }
            .into());
        }
        return Ok(report.clone());
    }

    let mut legacy_reports = reports
        .into_iter()
        .filter(|report| report.unit_run_id.is_none());
    let legacy = legacy_reports
        .next()
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "code_review_report",
            id: unit_run_id.to_string(),
        })?;
    if legacy_reports.next().is_some() {
        return Err(ProductStoreError::Ambiguous {
            kind: "code_review_report",
            id: unit_run_id.to_string(),
        }
        .into());
    }
    Err(SnapshotRebuildError::MissingUnitRunId(legacy.id))
}

fn raw_report_text(
    store: &super::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    report: &CodeReviewReport,
) -> Result<String, ProductStoreError> {
    let raw_ref =
        report
            .raw_provider_output_ref
            .as_deref()
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "code_review_raw_provider_output",
                id: report.id.clone(),
            })?;
    store.read_attempt_artifact_text(attempt, raw_ref)
}

fn snapshot_from_report(
    attempt_id: &str,
    unit: &crate::product::coding_models::CodingExecutionUnit,
    unit_run_id: &str,
    report: &CodeReviewReport,
    raw_report: &str,
) -> UnitReviewConclusionSnapshot {
    UnitReviewConclusionSnapshot {
        attempt_id: attempt_id.to_string(),
        unit_id: unit.id.clone(),
        unit_run_id: unit_run_id.to_string(),
        logical_work_item_id: unit.logical_work_item_id.clone(),
        work_item_revision_id: unit.work_item_revision_id.clone(),
        code_review_report_id: report.id.clone(),
        verdict: report.verdict.clone(),
        finding_digest: report
            .findings
            .iter()
            .map(|finding| CompactFindingDigest {
                defect_class: Some(format!("{:?}", finding.defect_class)),
                reason_code: finding.reason_code.clone(),
                severity: format!("{:?}", finding.severity).to_ascii_lowercase(),
                message_digest: sha256_hex(&normalize_message(&finding.message)),
            })
            .collect(),
        evidence_refs: report.tested_evidence_refs.clone(),
        diff_refs: report.diff_refs.clone(),
        raw_report_hash: sha256_hex(raw_report),
    }
}

fn normalize_message(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sha256_hex(value: &str) -> String {
    use sha2::{Digest, Sha256};

    hex::encode(Sha256::digest(value.as_bytes()))
}
