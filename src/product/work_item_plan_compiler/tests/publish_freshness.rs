use super::super::{
    FreshnessError, PlanCandidateIr, PlanCandidateMechanicalReport, verify_publish_freshness,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::models::{WorkItemSplitFinding, WorkItemSplitFindingSeverity};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, PlanCandidatePublicationProvenance,
    SourceRevisionRecord, SourceStoreError, SourceStoreScope, WorkItemPlanSourceStore,
};
use sha2::{Digest, Sha256};

const PROJECT_ID: &str = "project-001";
const ISSUE_ID: &str = "issue-001";
const PLAN_ID: &str = "plan-001";
const COMPILER_VERSION: &str =
    crate::product::work_item_plan_compiler::WORK_ITEM_PLAN_COMPILER_VERSION;

fn source_hash(source: &str) -> String {
    hex::encode(Sha256::digest(source.as_bytes()))
}

fn ir(source: &str) -> PlanCandidateIr {
    PlanCandidateIr {
        source_revision_hash: source_hash(source),
        compiler_version: COMPILER_VERSION.to_string(),
        items: Vec::new(),
    }
}

fn report(source: &str) -> PlanCandidateMechanicalReport {
    PlanCandidateMechanicalReport {
        source_revision_hash: source_hash(source),
        compiler_version: COMPILER_VERSION.to_string(),
        findings: Vec::new(),
    }
}

fn source_revision(id: &str, source: &str) -> SourceRevisionRecord {
    let mut revision = SourceRevisionRecord {
        id: id.to_string(),
        source: source.to_string(),
        source_revision_hash: source_hash(source),
        content_hash: String::new(),
    };
    revision.content_hash = revision.content_hash().unwrap();
    revision
}

fn ir_record(id: &str, source_revision_id: &str, source: &str) -> PlanCandidateIrRecord {
    let mut record = PlanCandidateIrRecord {
        id: id.to_string(),
        source_revision_id: source_revision_id.to_string(),
        ir: ir(source),
        content_hash: String::new(),
    };
    record.content_hash = record.content_hash().unwrap();
    record
}

fn mechanical_report_record(
    id: &str,
    source_revision_id: &str,
    ir_id: &str,
    source: &str,
) -> PlanCandidateMechanicalReportRecord {
    let mut record = PlanCandidateMechanicalReportRecord {
        id: id.to_string(),
        source_revision_id: source_revision_id.to_string(),
        ir_id: ir_id.to_string(),
        report: report(source),
        content_hash: String::new(),
    };
    record.content_hash = record.content_hash().unwrap();
    record
}

fn scope() -> SourceStoreScope {
    SourceStoreScope {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
    }
}

fn assert_code(error: SourceStoreError, expected: &str) {
    assert_eq!(
        error.code(),
        expected,
        "unexpected source-store error: {error:?}"
    );
}

#[test]
fn verify_publish_freshness_fails_closed_for_stale_source_compiler_and_report() {
    let source = "# Work Item Plan\n";
    let candidate = ir(source);
    let mechanical_report = report(source);

    assert_eq!(
        verify_publish_freshness("# Work Item Plan\nchanged", &candidate, &mechanical_report),
        Err(FreshnessError::SourceRevisionMismatch)
    );

    let mut old_compiler = candidate.clone();
    old_compiler.compiler_version = "work-item-plan-compiler@old".to_string();
    assert_eq!(
        verify_publish_freshness(source, &old_compiler, &mechanical_report),
        Err(FreshnessError::CompilerVersionMismatch)
    );

    let mut wrong_report_hash = mechanical_report.clone();
    wrong_report_hash.source_revision_hash = source_hash("other source");
    assert_eq!(
        verify_publish_freshness(source, &candidate, &wrong_report_hash),
        Err(FreshnessError::MechanicalValidationFailed)
    );

    let mut wrong_report_version = mechanical_report.clone();
    wrong_report_version.compiler_version = "work-item-plan-compiler@old".to_string();
    assert_eq!(
        verify_publish_freshness(source, &candidate, &wrong_report_version),
        Err(FreshnessError::MechanicalValidationFailed)
    );

    let mut error_report = mechanical_report;
    error_report.findings.push(WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Error,
        code: "mechanical_failure".to_string(),
        message: "mechanical validation found an error".to_string(),
        work_item_ids: Vec::new(),
    });
    assert_eq!(
        verify_publish_freshness(source, &candidate, &error_report),
        Err(FreshnessError::MechanicalValidationFailed)
    );
}

#[test]
fn source_store_round_trips_typed_immutable_records_and_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkItemPlanSourceStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let source = "# Work Item Plan\n";
    let scope = scope();
    let revision = source_revision("source-001", source);
    let source_ref = store
        .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
        .unwrap();
    assert_eq!(
        source_ref,
        "project/project-001/issue/issue-001/plan/plan-001/source_revision/source-001"
    );
    assert_eq!(
        store.get_source_revision(&scope, &source_ref).unwrap(),
        revision
    );
    assert_eq!(
        store
            .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
            .unwrap(),
        source_ref
    );

    let candidate = ir_record("ir-001", &revision.id, source);
    let ir_ref = store
        .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, PLAN_ID, &candidate)
        .unwrap();
    assert_eq!(
        store.get_plan_candidate_ir(&scope, &ir_ref).unwrap(),
        candidate
    );

    let mechanical = mechanical_report_record("report-001", &revision.id, &candidate.id, source);
    let mechanical_ref = store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &mechanical)
        .unwrap();
    assert_eq!(
        store
            .get_mechanical_report(&scope, &mechanical_ref)
            .unwrap(),
        mechanical
    );

    let mut provenance = PlanCandidatePublicationProvenance {
        id: "publication-001".to_string(),
        plan_id: PLAN_ID.to_string(),
        plan_revision_id: "plan-revision-001".to_string(),
        source_revision_ref: source_ref,
        plan_candidate_ir_ref: ir_ref,
        mechanical_report_ref: mechanical_ref,
        source_revision_hash: revision.source_revision_hash.clone(),
        compiler_version: COMPILER_VERSION.to_string(),
        published_at: "2026-08-27T12:34:56Z".to_string(),
        content_hash: String::new(),
    };
    provenance.content_hash = provenance.content_hash().unwrap();
    let provenance_ref = store
        .put_publication_provenance(PROJECT_ID, ISSUE_ID, PLAN_ID, &provenance)
        .unwrap();
    assert_eq!(
        store
            .get_publication_provenance(&scope, &provenance_ref)
            .unwrap(),
        provenance
    );
}

#[test]
fn source_store_put_rejects_malformed_scope_and_object_ids_with_stable_code() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkItemPlanSourceStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let revision = source_revision("bad/id", "source");
    assert_code(
        store
            .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
            .unwrap_err(),
        "SOURCE_STORE_MALFORMED_REF",
    );
    let revision = source_revision("source-001", "source");
    assert_code(
        store
            .put_source_revision("bad/scope", ISSUE_ID, PLAN_ID, &revision)
            .unwrap_err(),
        "SOURCE_STORE_MALFORMED_REF",
    );
}

#[test]
fn source_store_ref_error_precedence_and_immutable_hashes_are_stable() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemPlanSourceStore::new(paths.clone());
    let source = "# Work Item Plan\n";
    let revision = source_revision("source-001", source);
    let source_ref = store
        .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
        .unwrap();
    let expected_scope = scope();

    assert_code(
        store
            .get_source_revision(&expected_scope, "not-a-canonical-ref")
            .unwrap_err(),
        "SOURCE_STORE_MALFORMED_REF",
    );
    assert_code(
        store
            .get_plan_candidate_ir(&expected_scope, &source_ref)
            .unwrap_err(),
        "SOURCE_STORE_WRONG_KIND",
    );
    assert_code(
        store
            .get_source_revision(
                &expected_scope,
                "project/project-001/issue/issue-001/plan/plan-001/not_a_source_kind/source-001",
            )
            .unwrap_err(),
        "SOURCE_STORE_WRONG_KIND",
    );
    assert_code(
        store
            .get_source_revision(
                &SourceStoreScope {
                    plan_id: "other-plan".to_string(),
                    ..expected_scope.clone()
                },
                &source_ref,
            )
            .unwrap_err(),
        "SOURCE_STORE_SCOPE_MISMATCH",
    );
    assert_code(
        store
            .get_source_revision(
                &expected_scope,
                "project/project-001/issue/issue-001/plan/plan-001/source_revision/missing-001",
            )
            .unwrap_err(),
        "SOURCE_STORE_DANGLING_REF",
    );

    let mut changed_source = source_revision(&revision.id, "# Work Item Plan\nchanged");
    assert_code(
        store
            .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &changed_source)
            .unwrap_err(),
        "SOURCE_STORE_SOURCE_HASH_MISMATCH",
    );

    changed_source.content_hash = "not-the-canonical-content-hash".to_string();
    assert_code(
        store
            .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &changed_source)
            .unwrap_err(),
        "SOURCE_STORE_CONTENT_HASH_MISMATCH",
    );

    let candidate = ir_record("ir-001", &revision.id, source);
    store
        .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, PLAN_ID, &candidate)
        .unwrap();
    let mut changed_candidate = candidate.clone();
    changed_candidate.ir.source_revision_hash = source_hash("other source");
    changed_candidate.content_hash = changed_candidate.content_hash().unwrap();
    assert_code(
        store
            .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, PLAN_ID, &changed_candidate)
            .unwrap_err(),
        "SOURCE_STORE_SOURCE_HASH_MISMATCH",
    );

    let mut changed_version = candidate;
    changed_version.ir.compiler_version = "work-item-plan-compiler@old".to_string();
    changed_version.content_hash = changed_version.content_hash().unwrap();
    assert_code(
        store
            .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, PLAN_ID, &changed_version)
            .unwrap_err(),
        "SOURCE_STORE_COMPILER_VERSION_MISMATCH",
    );

    let mut tampered = revision;
    tampered.content_hash = "tampered-content-hash".to_string();
    std::fs::write(
        paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("work-item-plan-sources")
            .join(PLAN_ID)
            .join("source_revision")
            .join("source-001.json"),
        serde_json::to_vec(&tampered).unwrap(),
    )
    .unwrap();
    assert_code(
        store
            .get_source_revision(&expected_scope, &source_ref)
            .unwrap_err(),
        "SOURCE_STORE_CONTENT_HASH_MISMATCH",
    );
}
