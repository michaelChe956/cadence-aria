use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
pub use crate::product::work_item_plan_compiler::PlanCandidatePublicationProvenance;
use crate::product::work_item_plan_compiler::{PlanCandidateIr, PlanCandidateMechanicalReport};

const SOURCE_REVISION_KIND: &str = "source_revision";
const PLAN_CANDIDATE_IR_KIND: &str = "plan_candidate_ir";
const MECHANICAL_REPORT_KIND: &str = "mechanical_report";
const PUBLICATION_PROVENANCE_KIND: &str = "publication_provenance";

impl PlanCandidatePublicationProvenance {
    pub fn content_hash(&self) -> Result<String, SourceStoreError> {
        hash_without_content_hash(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStoreScope {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRevisionRecord {
    pub id: String,
    pub source: String,
    pub source_revision_hash: String,
    pub content_hash: String,
}

impl SourceRevisionRecord {
    pub fn content_hash(&self) -> Result<String, SourceStoreError> {
        hash_without_content_hash(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCandidateIrRecord {
    pub id: String,
    pub source_revision_id: String,
    pub ir: PlanCandidateIr,
    pub content_hash: String,
}

impl PlanCandidateIrRecord {
    pub fn content_hash(&self) -> Result<String, SourceStoreError> {
        hash_without_content_hash(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCandidateMechanicalReportRecord {
    pub id: String,
    pub source_revision_id: String,
    pub ir_id: String,
    pub report: PlanCandidateMechanicalReport,
    pub content_hash: String,
}

impl PlanCandidateMechanicalReportRecord {
    pub fn content_hash(&self) -> Result<String, SourceStoreError> {
        hash_without_content_hash(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceStoreError {
    MalformedRef,
    WrongKind,
    ScopeMismatch,
    DanglingRef,
    IdentityMismatch,
    ContentHashMismatch,
    SourceHashMismatch,
    CompilerVersionMismatch,
    Io(String),
    Json(String),
    Serialize(String),
}

impl SourceStoreError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MalformedRef => "SOURCE_STORE_MALFORMED_REF",
            Self::WrongKind => "SOURCE_STORE_WRONG_KIND",
            Self::ScopeMismatch => "SOURCE_STORE_SCOPE_MISMATCH",
            Self::DanglingRef => "SOURCE_STORE_DANGLING_REF",
            Self::IdentityMismatch => "SOURCE_STORE_IDENTITY_MISMATCH",
            Self::ContentHashMismatch => "SOURCE_STORE_CONTENT_HASH_MISMATCH",
            Self::SourceHashMismatch => "SOURCE_STORE_SOURCE_HASH_MISMATCH",
            Self::CompilerVersionMismatch => "SOURCE_STORE_COMPILER_VERSION_MISMATCH",
            Self::Io(_) | Self::Json(_) | Self::Serialize(_) => "persistence_failure",
        }
    }
}

impl From<ProductStoreError> for SourceStoreError {
    fn from(error: ProductStoreError) -> Self {
        match error {
            ProductStoreError::Io(message) => Self::Io(message),
            ProductStoreError::Json(message) => Self::Json(message),
            other => Self::Io(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkItemPlanSourceStore {
    paths: ProductAppPaths,
}

impl WorkItemPlanSourceStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn put_source_revision(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        revision: &SourceRevisionRecord,
    ) -> Result<String, SourceStoreError> {
        self.validate_scope_ids(project_id, issue_id, plan_id)?;
        validate_id(&revision.id)?;
        if revision.source_revision_hash != hash_bytes(revision.source.as_bytes()) {
            return Err(SourceStoreError::SourceHashMismatch);
        }
        self.verify_content_hash(revision, &revision.content_hash)?;
        let canonical_ref = canonical_ref(
            project_id,
            issue_id,
            plan_id,
            SOURCE_REVISION_KIND,
            &revision.id,
        );
        let path = self.record_path(
            project_id,
            issue_id,
            plan_id,
            SOURCE_REVISION_KIND,
            &revision.id,
        )?;
        self.put_immutable(
            &path,
            revision,
            |stored| {
                if stored.source_revision_hash != revision.source_revision_hash {
                    SourceStoreError::SourceHashMismatch
                } else {
                    SourceStoreError::IdentityMismatch
                }
            },
            |stored| {
                if stored.source_revision_hash != hash_bytes(stored.source.as_bytes()) {
                    return Err(SourceStoreError::SourceHashMismatch);
                }
                self.verify_content_hash(stored, &stored.content_hash)
            },
        )?;
        Ok(canonical_ref)
    }

    pub fn get_source_revision(
        &self,
        expected_scope: &SourceStoreScope,
        canonical_ref: &str,
    ) -> Result<SourceRevisionRecord, SourceStoreError> {
        let parsed =
            self.parse_expected_ref(expected_scope, canonical_ref, SOURCE_REVISION_KIND)?;
        let record: SourceRevisionRecord = self.read_required(&parsed)?;
        if record.id != parsed.object_id {
            return Err(SourceStoreError::IdentityMismatch);
        }
        if record.source_revision_hash != hash_bytes(record.source.as_bytes()) {
            return Err(SourceStoreError::SourceHashMismatch);
        }
        self.verify_content_hash(&record, &record.content_hash)?;
        Ok(record)
    }

    pub fn put_plan_candidate_ir(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        ir: &PlanCandidateIrRecord,
    ) -> Result<String, SourceStoreError> {
        self.validate_scope_ids(project_id, issue_id, plan_id)?;
        validate_id(&ir.id)?;
        validate_id(&ir.source_revision_id)?;
        self.verify_content_hash(ir, &ir.content_hash)?;
        let scope = SourceStoreScope {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: plan_id.to_string(),
        };
        self.validate_ir_record(&scope, ir)?;
        let canonical_ref = canonical_ref(
            project_id,
            issue_id,
            plan_id,
            PLAN_CANDIDATE_IR_KIND,
            &ir.id,
        );
        let path = self.record_path(
            project_id,
            issue_id,
            plan_id,
            PLAN_CANDIDATE_IR_KIND,
            &ir.id,
        )?;
        self.put_immutable(
            &path,
            ir,
            |stored| {
                if stored.source_revision_id != ir.source_revision_id
                    || stored.ir.source_revision_hash != ir.ir.source_revision_hash
                {
                    SourceStoreError::SourceHashMismatch
                } else if stored.ir.compiler_version != ir.ir.compiler_version {
                    SourceStoreError::CompilerVersionMismatch
                } else {
                    SourceStoreError::IdentityMismatch
                }
            },
            |stored| self.validate_ir_record(&scope, stored),
        )?;
        Ok(canonical_ref)
    }

    pub fn get_plan_candidate_ir(
        &self,
        expected_scope: &SourceStoreScope,
        canonical_ref: &str,
    ) -> Result<PlanCandidateIrRecord, SourceStoreError> {
        let parsed =
            self.parse_expected_ref(expected_scope, canonical_ref, PLAN_CANDIDATE_IR_KIND)?;
        let record: PlanCandidateIrRecord = self.read_required(&parsed)?;
        if record.id != parsed.object_id {
            return Err(SourceStoreError::IdentityMismatch);
        }
        self.verify_content_hash(&record, &record.content_hash)?;
        self.validate_ir_record(&parsed.scope, &record)?;
        Ok(record)
    }

    pub fn put_mechanical_report(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        report: &PlanCandidateMechanicalReportRecord,
    ) -> Result<String, SourceStoreError> {
        self.validate_scope_ids(project_id, issue_id, plan_id)?;
        validate_id(&report.id)?;
        validate_id(&report.source_revision_id)?;
        validate_id(&report.ir_id)?;
        self.verify_content_hash(report, &report.content_hash)?;
        let scope = SourceStoreScope {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: plan_id.to_string(),
        };
        self.validate_mechanical_report_record(&scope, report)?;
        let canonical_ref = canonical_ref(
            project_id,
            issue_id,
            plan_id,
            MECHANICAL_REPORT_KIND,
            &report.id,
        );
        let path = self.record_path(
            project_id,
            issue_id,
            plan_id,
            MECHANICAL_REPORT_KIND,
            &report.id,
        )?;
        self.put_immutable(
            &path,
            report,
            |stored| {
                if stored.source_revision_id != report.source_revision_id
                    || stored.report.source_revision_hash != report.report.source_revision_hash
                {
                    SourceStoreError::SourceHashMismatch
                } else if stored.report.compiler_version != report.report.compiler_version {
                    SourceStoreError::CompilerVersionMismatch
                } else {
                    SourceStoreError::IdentityMismatch
                }
            },
            |stored| self.validate_mechanical_report_record(&scope, stored),
        )?;
        Ok(canonical_ref)
    }

    pub fn get_mechanical_report(
        &self,
        expected_scope: &SourceStoreScope,
        canonical_ref: &str,
    ) -> Result<PlanCandidateMechanicalReportRecord, SourceStoreError> {
        let parsed =
            self.parse_expected_ref(expected_scope, canonical_ref, MECHANICAL_REPORT_KIND)?;
        let record: PlanCandidateMechanicalReportRecord = self.read_required(&parsed)?;
        if record.id != parsed.object_id {
            return Err(SourceStoreError::IdentityMismatch);
        }
        self.verify_content_hash(&record, &record.content_hash)?;
        self.validate_mechanical_report_record(&parsed.scope, &record)?;
        Ok(record)
    }

    pub fn put_publication_provenance(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        provenance: &PlanCandidatePublicationProvenance,
    ) -> Result<String, SourceStoreError> {
        self.validate_scope_ids(project_id, issue_id, plan_id)?;
        validate_id(&provenance.id)?;
        validate_id(&provenance.plan_id)?;
        if provenance.plan_id != plan_id {
            return Err(SourceStoreError::IdentityMismatch);
        }
        self.verify_content_hash(provenance, &provenance.content_hash)?;
        let canonical_ref = canonical_ref(
            project_id,
            issue_id,
            plan_id,
            PUBLICATION_PROVENANCE_KIND,
            &provenance.id,
        );
        let path = self.record_path(
            project_id,
            issue_id,
            plan_id,
            PUBLICATION_PROVENANCE_KIND,
            &provenance.id,
        )?;
        self.validate_provenance_references(project_id, issue_id, plan_id, provenance)?;
        self.put_immutable(
            &path,
            provenance,
            |stored| {
                if stored.source_revision_hash != provenance.source_revision_hash {
                    SourceStoreError::SourceHashMismatch
                } else if stored.compiler_version != provenance.compiler_version {
                    SourceStoreError::CompilerVersionMismatch
                } else {
                    SourceStoreError::IdentityMismatch
                }
            },
            |stored| self.verify_content_hash(stored, &stored.content_hash),
        )?;
        Ok(canonical_ref)
    }

    pub fn get_publication_provenance(
        &self,
        expected_scope: &SourceStoreScope,
        canonical_ref: &str,
    ) -> Result<PlanCandidatePublicationProvenance, SourceStoreError> {
        let parsed =
            self.parse_expected_ref(expected_scope, canonical_ref, PUBLICATION_PROVENANCE_KIND)?;
        let record: PlanCandidatePublicationProvenance = self.read_required(&parsed)?;
        if record.id != parsed.object_id || record.plan_id != parsed.scope.plan_id {
            return Err(SourceStoreError::IdentityMismatch);
        }
        self.verify_content_hash(&record, &record.content_hash)?;
        self.validate_provenance_references(
            &parsed.scope.project_id,
            &parsed.scope.issue_id,
            &parsed.scope.plan_id,
            &record,
        )?;
        Ok(record)
    }

    fn validate_ir_record(
        &self,
        scope: &SourceStoreScope,
        ir: &PlanCandidateIrRecord,
    ) -> Result<(), SourceStoreError> {
        let source = self.get_source_revision(
            scope,
            &canonical_ref(
                &scope.project_id,
                &scope.issue_id,
                &scope.plan_id,
                SOURCE_REVISION_KIND,
                &ir.source_revision_id,
            ),
        )?;
        if ir.ir.source_revision_hash != source.source_revision_hash {
            return Err(SourceStoreError::SourceHashMismatch);
        }
        Ok(())
    }

    fn validate_mechanical_report_record(
        &self,
        scope: &SourceStoreScope,
        report: &PlanCandidateMechanicalReportRecord,
    ) -> Result<(), SourceStoreError> {
        let source = self.get_source_revision(
            scope,
            &canonical_ref(
                &scope.project_id,
                &scope.issue_id,
                &scope.plan_id,
                SOURCE_REVISION_KIND,
                &report.source_revision_id,
            ),
        )?;
        let ir = self.get_plan_candidate_ir(
            scope,
            &canonical_ref(
                &scope.project_id,
                &scope.issue_id,
                &scope.plan_id,
                PLAN_CANDIDATE_IR_KIND,
                &report.ir_id,
            ),
        )?;
        if ir.source_revision_id != report.source_revision_id
            || report.report.source_revision_hash != source.source_revision_hash
            || ir.ir.source_revision_hash != source.source_revision_hash
        {
            return Err(SourceStoreError::SourceHashMismatch);
        }
        if report.report.compiler_version != ir.ir.compiler_version {
            return Err(SourceStoreError::CompilerVersionMismatch);
        }
        Ok(())
    }

    fn validate_provenance_references(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        provenance: &PlanCandidatePublicationProvenance,
    ) -> Result<(), SourceStoreError> {
        validate_id(&provenance.plan_revision_id)?;
        let expected_scope = SourceStoreScope {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: plan_id.to_string(),
        };
        let source = self.get_source_revision(&expected_scope, &provenance.source_revision_ref)?;
        let ir = self.get_plan_candidate_ir(&expected_scope, &provenance.plan_candidate_ir_ref)?;
        let report =
            self.get_mechanical_report(&expected_scope, &provenance.mechanical_report_ref)?;
        if provenance.source_revision_hash != source.source_revision_hash
            || ir.source_revision_id != source.id
            || ir.ir.source_revision_hash != source.source_revision_hash
            || report.source_revision_id != source.id
            || report.ir_id != ir.id
            || report.report.source_revision_hash != source.source_revision_hash
        {
            return Err(SourceStoreError::SourceHashMismatch);
        }
        if provenance.compiler_version != ir.ir.compiler_version
            || report.report.compiler_version != ir.ir.compiler_version
        {
            return Err(SourceStoreError::CompilerVersionMismatch);
        }
        Ok(())
    }

    fn validate_scope_ids(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<(), SourceStoreError> {
        validate_id(project_id)?;
        validate_id(issue_id)?;
        validate_id(plan_id)
    }

    fn record_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        object_kind: &str,
        object_id: &str,
    ) -> Result<PathBuf, SourceStoreError> {
        self.validate_scope_ids(project_id, issue_id, plan_id)?;
        validate_id(object_id)?;
        Ok(self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-item-plan-sources")
            .join(plan_id)
            .join(object_kind)
            .join(format!("{object_id}.json")))
    }

    fn parse_expected_ref(
        &self,
        expected_scope: &SourceStoreScope,
        canonical_ref: &str,
        expected_kind: &'static str,
    ) -> Result<ParsedRef, SourceStoreError> {
        let parsed = parse_canonical_ref(canonical_ref)?;
        if parsed.object_kind != expected_kind {
            return Err(SourceStoreError::WrongKind);
        }
        if parsed.scope != *expected_scope {
            return Err(SourceStoreError::ScopeMismatch);
        }
        self.validate_scope_ids(
            &expected_scope.project_id,
            &expected_scope.issue_id,
            &expected_scope.plan_id,
        )?;
        validate_id(&parsed.object_id)?;
        Ok(parsed)
    }

    fn read_required<T: for<'de> Deserialize<'de>>(
        &self,
        parsed: &ParsedRef,
    ) -> Result<T, SourceStoreError> {
        let path = self.record_path(
            &parsed.scope.project_id,
            &parsed.scope.issue_id,
            &parsed.scope.plan_id,
            &parsed.object_kind,
            &parsed.object_id,
        )?;
        if !path_exists(&path)? {
            return Err(SourceStoreError::DanglingRef);
        }
        read_json(&path).map_err(SourceStoreError::from)
    }

    fn put_immutable<T, M, V>(
        &self,
        path: &Path,
        record: &T,
        mismatch: M,
        validate_stored: V,
    ) -> Result<(), SourceStoreError>
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq,
        M: FnOnce(&T) -> SourceStoreError,
        V: FnOnce(&T) -> Result<(), SourceStoreError>,
    {
        with_exclusive_lock(path, || {
            if path_exists(path)? {
                let stored: T = read_json(path).map_err(SourceStoreError::from)?;
                validate_stored(&stored)?;
                if stored == *record {
                    return Ok(());
                }
                return Err(mismatch(&stored));
            }
            write_json(path, record).map_err(SourceStoreError::from)
        })
    }

    fn verify_content_hash<T: Serialize>(
        &self,
        record: &T,
        supplied_content_hash: &str,
    ) -> Result<(), SourceStoreError> {
        if hash_without_content_hash(record)? != supplied_content_hash {
            return Err(SourceStoreError::ContentHashMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedRef {
    scope: SourceStoreScope,
    object_kind: String,
    object_id: String,
}

fn parse_canonical_ref(canonical_ref: &str) -> Result<ParsedRef, SourceStoreError> {
    let segments = canonical_ref.split('/').collect::<Vec<_>>();
    let [
        "project",
        project_id,
        "issue",
        issue_id,
        "plan",
        plan_id,
        object_kind,
        object_id,
    ] = segments.as_slice()
    else {
        return Err(SourceStoreError::MalformedRef);
    };
    if project_id.is_empty()
        || issue_id.is_empty()
        || plan_id.is_empty()
        || object_kind.is_empty()
        || object_id.is_empty()
        || [*project_id, *issue_id, *plan_id, *object_kind, *object_id]
            .iter()
            .any(|segment| validate_relative_id(segment).is_err())
    {
        return Err(SourceStoreError::MalformedRef);
    }
    Ok(ParsedRef {
        scope: SourceStoreScope {
            project_id: (*project_id).to_string(),
            issue_id: (*issue_id).to_string(),
            plan_id: (*plan_id).to_string(),
        },
        object_kind: (*object_kind).to_string(),
        object_id: (*object_id).to_string(),
    })
}

fn canonical_ref(
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    object_kind: &str,
    object_id: &str,
) -> String {
    format!("project/{project_id}/issue/{issue_id}/plan/{plan_id}/{object_kind}/{object_id}")
}

fn hash_without_content_hash<T: Serialize>(record: &T) -> Result<String, SourceStoreError> {
    let mut value = serde_json::to_value(record)
        .map_err(|error| SourceStoreError::Serialize(error.to_string()))?;
    let Some(object) = value.as_object_mut() else {
        return Err(SourceStoreError::Serialize(
            "source-store records must serialize to JSON objects".to_string(),
        ));
    };
    object.remove("content_hash");
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| SourceStoreError::Serialize(error.to_string()))?;
    Ok(hash_bytes(&bytes))
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn validate_id(value: &str) -> Result<(), SourceStoreError> {
    validate_relative_id(value).map_err(|error| SourceStoreError::Io(error.to_string()))
}

fn path_exists(path: &Path) -> Result<bool, SourceStoreError> {
    path.try_exists()
        .map_err(|error| SourceStoreError::Io(format!("inspect {}: {error}", path.display())))
}

fn with_exclusive_lock<T>(
    target_path: &Path,
    operation: impl FnOnce() -> Result<T, SourceStoreError>,
) -> Result<T, SourceStoreError> {
    let _lock = ExclusiveFileLock::acquire(target_path)?;
    operation()
}

struct ExclusiveFileLock {
    file: File,
}

impl ExclusiveFileLock {
    fn acquire(target_path: &Path) -> Result<Self, SourceStoreError> {
        let lock_path = lock_path_for(target_path);
        if let Some(parent) = lock_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| {
                SourceStoreError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|error| {
                SourceStoreError::Io(format!("open lock {}: {error}", lock_path.display()))
            })?;
        lock_file_exclusive(&file, &lock_path)?;
        Ok(Self { file })
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
}

fn lock_path_for(target_path: &Path) -> PathBuf {
    let file_name = target_path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "work-item-plan-source".into());
    target_path.with_file_name(format!(".{file_name}.lock"))
}

#[cfg(unix)]
fn lock_file_exclusive(file: &File, lock_path: &Path) -> Result<(), SourceStoreError> {
    loop {
        // SAFETY: flock only reads the valid file descriptor and does not retain any Rust pointer.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != ErrorKind::Interrupted {
            return Err(SourceStoreError::Io(format!(
                "lock {}: {error}",
                lock_path.display()
            )));
        }
    }
}

#[cfg(unix)]
fn unlock_file(file: &File) {
    // SAFETY: flock only reads the valid file descriptor and does not retain any Rust pointer.
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(not(unix))]
fn lock_file_exclusive(_file: &File, lock_path: &Path) -> Result<(), SourceStoreError> {
    Err(SourceStoreError::Io(format!(
        "file locking is unsupported on this platform: {}",
        lock_path.display()
    )))
}

#[cfg(not(unix))]
fn unlock_file(_file: &File) {}
