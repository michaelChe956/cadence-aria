use super::super::{
    FreshnessError, PlanCandidateIr, PlanCandidateMechanicalReport, verify_publish_freshness,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::models::{WorkItemSplitFinding, WorkItemSplitFindingSeverity};
use crate::product::planning_failure::{
    PersistedEffectivePlanningEdge, PersistedPlanningStructureReport,
    PersistedPlanningStructureSpan, PlanningEdgeSource, PlanningFailureKind, PlanningFailureSet,
};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, PlanCandidatePublicationProvenance,
    SourceRevisionRecord, SourceStoreError, SourceStoreScope, WorkItemPlanSourceStore,
    persisted_structure_report_digest,
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
        // SG3a.0：旧 fixture 形态（无结构报告），不启动新 review。
        structure_report: None,
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
        // F1.2：fixture 沿旧 record 形态（kind=None，序列化字节不变）。
        kind: None,
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

fn e2e_finding(kind: Option<PlanningFailureKind>) -> WorkItemSplitFinding {
    WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Warning,
        code: "integration_or_e2e_skipped_risk".to_string(),
        message: "integration or e2e verification skipped risk".to_string(),
        work_item_ids: Vec::new(),
        kind,
    }
}

fn source_addressed_report_id(source: &str) -> String {
    format!("report-{}", &source_hash(source)[..16])
}

fn report_record_with_findings(
    id: &str,
    source_revision_id: &str,
    ir_id: &str,
    source: &str,
    findings: Vec<WorkItemSplitFinding>,
) -> PlanCandidateMechanicalReportRecord {
    let mut record = PlanCandidateMechanicalReportRecord {
        id: id.to_string(),
        source_revision_id: source_revision_id.to_string(),
        ir_id: ir_id.to_string(),
        report: PlanCandidateMechanicalReport {
            source_revision_hash: source_hash(source),
            compiler_version: COMPILER_VERSION.to_string(),
            findings,
        },
        // SG3a.0：旧 fixture 形态（无结构报告），不启动新 review。
        structure_report: None,
        content_hash: String::new(),
    };
    record.content_hash = record.content_hash().unwrap();
    record
}

fn content_addressed_report_record(
    source_revision_id: &str,
    ir_id: &str,
    source: &str,
    findings: Vec<WorkItemSplitFinding>,
) -> PlanCandidateMechanicalReportRecord {
    let mut record = report_record_with_findings("", source_revision_id, ir_id, source, findings);
    record.id = record.content_addressed_id().unwrap();
    record.content_hash = record.content_hash().unwrap();
    record
}

#[test]
fn legacy_source_addressed_report_id_collides_on_validator_kind_evolution() {
    // 回归锚（session_0003 撞历史机理）：report_id 只按 source hash 前
    // 16 位寻址时，同 source 上 validator 语义演进（F2b：findings 强制
    // typed kind）→ 同 id 不同 body → put_immutable 不可变闭包按锚字段
    // 全等落 else 分支 = SOURCE_STORE_IDENTITY_MISMATCH，plan 用户重试
    // 被确定性阻塞。该锚证明：内容寻址 id 是免撞的必要条件。
    let temp = tempfile::tempdir().unwrap();
    let store = WorkItemPlanSourceStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let source = "# Work Item Plan\n";
    let revision = source_revision("source-001", source);
    store
        .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
        .unwrap();
    let candidate = ir_record("ir-001", &revision.id, source);
    store
        .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, PLAN_ID, &candidate)
        .unwrap();

    let legacy_id = source_addressed_report_id(source);
    let legacy = report_record_with_findings(
        &legacy_id,
        &revision.id,
        &candidate.id,
        source,
        vec![e2e_finding(None)],
    );
    store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &legacy)
        .unwrap();

    let evolved = report_record_with_findings(
        &legacy_id,
        &revision.id,
        &candidate.id,
        source,
        vec![e2e_finding(Some(PlanningFailureKind::Unknown {
            code: "integration_or_e2e_skipped_risk".to_string(),
        }))],
    );
    assert_code(
        store
            .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &evolved)
            .unwrap_err(),
        "SOURCE_STORE_IDENTITY_MISMATCH",
    );
}

#[test]
fn content_addressed_report_id_appends_evolved_report_without_collision() {
    // 主案验收：同 source、findings 仅 kind 不同（旧 None vs 新 typed）
    // 的两 record 在全量内容寻址 id 下 id 不同、各自落盘成功；旧
    // id 格式存量记录不被撞；同 record 重放走整记录相等分支幂等 Ok；
    // get 按各自 canonical ref 取回正确记录。
    let temp = tempfile::tempdir().unwrap();
    let store = WorkItemPlanSourceStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let scope = scope();
    let source = "# Work Item Plan\n";
    let revision = source_revision("source-001", source);
    let source_ref = store
        .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
        .unwrap();
    let candidate = ir_record("ir-001", &revision.id, source);
    let ir_ref = store
        .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, PLAN_ID, &candidate)
        .unwrap();

    // 存量：F1.2 时代旧 id 格式报告（kind=None），模拟 13:58 落盘历史。
    let legacy = report_record_with_findings(
        &source_addressed_report_id(source),
        &revision.id,
        &candidate.id,
        source,
        vec![e2e_finding(None)],
    );
    let legacy_ref = store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &legacy)
        .unwrap();

    // 新 validate 产出（F2b 后 typed kind），内容寻址 id。
    let evolved = content_addressed_report_record(
        &revision.id,
        &candidate.id,
        source,
        vec![e2e_finding(Some(PlanningFailureKind::Unknown {
            code: "integration_or_e2e_skipped_risk".to_string(),
        }))],
    );
    assert_ne!(evolved.id, legacy.id);
    assert!(evolved.id.starts_with("report-"));
    assert_eq!(evolved.id.len(), "report-".len() + 16);
    let evolved_ref = store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &evolved)
        .unwrap();
    assert_ne!(evolved_ref, legacy_ref);

    // 同 record 重放：整记录相等分支幂等 Ok，ref 稳定。
    assert_eq!(
        store
            .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &evolved)
            .unwrap(),
        evolved_ref
    );

    // get 按各自 canonical ref 取回正确记录，存量不受影响。
    assert_eq!(
        store.get_mechanical_report(&scope, &legacy_ref).unwrap(),
        legacy
    );
    assert_eq!(
        store.get_mechanical_report(&scope, &evolved_ref).unwrap(),
        evolved
    );
    assert_eq!(
        store.get_source_revision(&scope, &source_ref).unwrap(),
        revision
    );
    assert_eq!(
        store.get_plan_candidate_ir(&scope, &ir_ref).unwrap(),
        candidate
    );

    // 同内容重放派生同 id（内容寻址确定性）：kind 演进前后各自幂等。
    let replay = content_addressed_report_record(
        &revision.id,
        &candidate.id,
        source,
        vec![e2e_finding(Some(PlanningFailureKind::Unknown {
            code: "integration_or_e2e_skipped_risk".to_string(),
        }))],
    );
    assert_eq!(replay.id, evolved.id);
    let legacy_replay = content_addressed_report_record(
        &revision.id,
        &candidate.id,
        source,
        vec![e2e_finding(None)],
    );
    assert_ne!(legacy_replay.id, evolved.id);
}

// ============ SG3a.0：immutable 结构报告存储与同提交构造兼容 ============

fn structure_payload(
    source_revision_id: &str,
    ir_id: &str,
    source: &str,
    context_digest: &str,
) -> PersistedPlanningStructureReport {
    let mut payload = PersistedPlanningStructureReport {
        source_revision_id: source_revision_id.to_string(),
        source_hash: source_hash(source),
        ir_id: ir_id.to_string(),
        compiler_version: COMPILER_VERSION.to_string(),
        rule_version: "effective-planning-graph@sg1".to_string(),
        context_digest: context_digest.to_string(),
        // R1 双来源边：WI-004→WI-005 canonical_input（契约+源定位）、
        // WI-005→WI-004 change_order（[前仓索引, 后仓索引]）。
        effective_edges: vec![
            PersistedEffectivePlanningEdge {
                from: "WI-004".to_string(),
                to: "WI-005".to_string(),
                sources: vec![PlanningEdgeSource::CanonicalInput],
                contract_refs: vec!["contract.e2e-verification".to_string()],
                spans: vec![PersistedPlanningStructureSpan {
                    field: "inputs[0]".to_string(),
                    line: 12,
                }],
                order_indices: Vec::new(),
            },
            PersistedEffectivePlanningEdge {
                from: "WI-005".to_string(),
                to: "WI-004".to_string(),
                sources: vec![PlanningEdgeSource::ChangeOrder],
                contract_refs: Vec::new(),
                spans: Vec::new(),
                order_indices: vec![1, 0],
            },
        ],
        cycle_witness: Vec::new(),
        findings: PlanningFailureSet::default(),
        structurally_valid: true,
        protected_predicates: vec![
            "canonical_inputs_preserved".to_string(),
            "acceptance_criteria_preserved".to_string(),
        ],
        instruction_refs: Vec::new(),
        repair_audit_refs: Vec::new(),
        report_digest: String::new(),
    };
    payload.report_digest = persisted_structure_report_digest(&payload).unwrap();
    payload
}

fn sealed_structure_report_record(
    source_revision_id: &str,
    ir_id: &str,
    source: &str,
    context_digest: &str,
) -> PlanCandidateMechanicalReportRecord {
    let mut record = report_record_with_findings("", source_revision_id, ir_id, source, Vec::new());
    record.structure_report = Some(structure_payload(
        source_revision_id,
        ir_id,
        source,
        context_digest,
    ));
    record.id = record.content_addressed_id().unwrap();
    record.content_hash = record.content_hash().unwrap();
    record
}

fn report_json_path(paths: &ProductAppPaths, report_id: &str) -> std::path::PathBuf {
    paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("work-item-plan-sources")
        .join(PLAN_ID)
        .join("mechanical_report")
        .join(format!("{report_id}.json"))
}

#[test]
fn sg_03a_0_report_roundtrip_join_and_legacy_hash() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemPlanSourceStore::new(paths.clone());
    let scope = scope();
    let source = "# Work Item Plan\n";
    let revision = source_revision("source-001", source);
    store
        .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
        .unwrap();
    let candidate = ir_record("ir-001", &revision.id, source);
    store
        .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, PLAN_ID, &candidate)
        .unwrap();

    // 旧 record 无 structure 字段：get/put/content_hash 仍合法。
    let legacy = mechanical_report_record("report-001", &revision.id, &candidate.id, source);
    let legacy_ref = store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &legacy)
        .unwrap();
    assert_eq!(
        store.get_mechanical_report(&scope, &legacy_ref).unwrap(),
        legacy
    );
    // 同 record 重放幂等（ref 稳定）。
    assert_eq!(
        store
            .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &legacy)
            .unwrap(),
        legacy_ref
    );

    // None skip 序列化：旧形态 JSON 不含 structure_report 键。
    let legacy_json = serde_json::to_value(&legacy).unwrap();
    assert!(legacy_json.get("structure_report").is_none());
    // 旧 JSON（缺字段）解析为 None，不伪合法结构证据。
    let legacy_text = serde_json::to_string(&legacy_json).unwrap();
    let parsed: PlanCandidateMechanicalReportRecord = serde_json::from_str(&legacy_text).unwrap();
    assert_eq!(parsed, legacy);
    assert!(parsed.structure_report.is_none());

    // 新 Some 经 content_hash 完整绑定：put/get roundtrip 全字段保真，
    // 机械三字段保留在同一 immutable record 内。
    let sealed =
        sealed_structure_report_record(&revision.id, &candidate.id, source, "ctx-digest-a");
    assert_ne!(sealed.id, legacy.id);
    let sealed_ref = store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &sealed)
        .unwrap();
    assert_eq!(
        store.get_mechanical_report(&scope, &sealed_ref).unwrap(),
        sealed
    );
    assert_eq!(sealed.report.source_revision_hash, source_hash(source));
    assert_eq!(sealed.report.compiler_version, COMPILER_VERSION);

    // get 路径篡改：盘上 payload 改 context_digest 并重算外层
    // content_hash 封口 → report_digest 失配仍拒（拒绝依据是 payload
    // 级绑定，不是外层哈希兜底）。
    let report_path = report_json_path(&paths, &sealed.id);
    let mut on_disk: PlanCandidateMechanicalReportRecord =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    on_disk.structure_report.as_mut().unwrap().context_digest = "ctx-tampered".to_string();
    on_disk.content_hash = on_disk.content_hash().unwrap();
    std::fs::write(&report_path, serde_json::to_vec(&on_disk).unwrap()).unwrap();
    assert_code(
        store
            .get_mechanical_report(&scope, &sealed_ref)
            .unwrap_err(),
        "SOURCE_STORE_CONTENT_HASH_MISMATCH",
    );
}

#[test]
fn sg_03a_0_structure_payload_bindings_reject_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemPlanSourceStore::new(paths);
    let scope = scope();
    let source = "# Work Item Plan\n";
    let revision = source_revision("source-001", source);
    store
        .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
        .unwrap();
    let candidate = ir_record("ir-001", &revision.id, source);
    store
        .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, PLAN_ID, &candidate)
        .unwrap();

    let sealed =
        sealed_structure_report_record(&revision.id, &candidate.id, source, "ctx-digest-a");
    let sealed_ref = store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &sealed)
        .unwrap();

    // 篡改任一绑定字段后重封外层 id/content_hash（模拟完整伪造），
    // put 仍必须按 payload 级绑定拒绝。
    let tamper = |mutate: &dyn Fn(&mut PersistedPlanningStructureReport), expected_code: &str| {
        let mut tampered = sealed.clone();
        mutate(tampered.structure_report.as_mut().unwrap());
        tampered.id = tampered.content_addressed_id().unwrap();
        tampered.content_hash = tampered.content_hash().unwrap();
        assert_code(
            store
                .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &tampered)
                .unwrap_err(),
            expected_code,
        );
    };

    // source/ir/context/report_digest 任一篡改拒（plan Task SG3a.0 Step1）。
    tamper(
        &|p| p.source_revision_id = "source-999".to_string(),
        "SOURCE_STORE_IDENTITY_MISMATCH",
    );
    tamper(
        &|p| p.source_hash = source_hash("other source"),
        "SOURCE_STORE_SOURCE_HASH_MISMATCH",
    );
    tamper(
        &|p| p.ir_id = "ir-999".to_string(),
        "SOURCE_STORE_IDENTITY_MISMATCH",
    );
    tamper(
        &|p| p.context_digest = "ctx-tampered".to_string(),
        "SOURCE_STORE_CONTENT_HASH_MISMATCH",
    );
    tamper(
        &|p| p.report_digest = "0".repeat(64),
        "SOURCE_STORE_CONTENT_HASH_MISMATCH",
    );
    tamper(
        &|p| p.compiler_version = "work-item-plan-compiler@old".to_string(),
        "SOURCE_STORE_COMPILER_VERSION_MISMATCH",
    );

    // 全部拒绝后，合法原 record 不受影响（旧对象字节不改）。
    assert_eq!(
        store.get_mechanical_report(&scope, &sealed_ref).unwrap(),
        sealed
    );
}

#[test]
fn sg_03a_0_same_source_new_context_gets_new_report_id_and_ref() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = WorkItemPlanSourceStore::new(paths.clone());
    let scope = scope();
    let source = "# Work Item Plan\n";
    let revision = source_revision("source-001", source);
    store
        .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
        .unwrap();
    let candidate = ir_record("ir-001", &revision.id, source);
    store
        .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, PLAN_ID, &candidate)
        .unwrap();

    let sealed_a =
        sealed_structure_report_record(&revision.id, &candidate.id, source, "ctx-digest-a");
    let ref_a = store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &sealed_a)
        .unwrap();
    let bytes_before = std::fs::read(report_json_path(&paths, &sealed_a.id)).unwrap();

    // 同 source/IR、不同 context → 须新 report ID/ref（source-only 撞历史
    // 教训：身份由 source/IR/context/report digest 共同决定）。
    let sealed_b =
        sealed_structure_report_record(&revision.id, &candidate.id, source, "ctx-digest-b");
    assert_ne!(sealed_b.id, sealed_a.id);
    let ref_b = store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &sealed_b)
        .unwrap();
    assert_ne!(ref_b, ref_a);

    // 旧对象字节不改；两 ref 各自取回正确 record。
    let bytes_after = std::fs::read(report_json_path(&paths, &sealed_a.id)).unwrap();
    assert_eq!(bytes_before, bytes_after);
    assert_eq!(
        store.get_mechanical_report(&scope, &ref_a).unwrap(),
        sealed_a
    );
    assert_eq!(
        store.get_mechanical_report(&scope, &ref_b).unwrap(),
        sealed_b
    );

    // 同 payload 重放幂等（整记录相等分支，ref 稳定）。
    assert_eq!(
        store
            .put_mechanical_report(PROJECT_ID, ISSUE_ID, PLAN_ID, &sealed_b)
            .unwrap(),
        ref_b
    );
}
