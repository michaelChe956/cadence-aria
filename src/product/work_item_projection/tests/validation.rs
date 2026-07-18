use super::{compiled_fixture, compiled_plan_fixture, contract_fixture};
use crate::product::models::{
    PlanProjectionBundle, PlanValidationReportArtifact, WorkItemProjectionBundle,
};
use crate::product::work_item_contract::{
    BlockerRule, ContractValidationReport, RequiredInputContract, VerificationCheck, WorkItemTask,
};
use crate::product::work_item_projection::{
    PlanProjectionCompileInput, PlanProjectionCompiler, ProjectionValidationReport,
    WorkItemProjectionCompiler, projection_hashes, validate_projection_coverage,
};

#[test]
fn work_item_projection_validation_accepts_complete_compilation() {
    let contract = contract_fixture();
    let compiled = WorkItemProjectionCompiler
        .compile(&contract, "work_item_revision_0001")
        .unwrap();

    assert!(
        validate_projection_coverage(&contract, "work_item_revision_0001", &compiled).is_valid()
    );
}

#[test]
fn work_item_projection_validation_covers_missing_and_invented_normative_content() {
    let contract = contract_fixture();
    let baseline = compiled_fixture();

    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_missing_contract_ref",
        "coder.task_refs",
        Some("task_1"),
        |value| {
            value.coder.task_refs.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_missing_contract_ref",
        "coder.tasks",
        Some("task_1"),
        |value| {
            value.coder.tasks.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_invented_contract_ref",
        "coder.tasks",
        Some("task_invented"),
        |value| {
            value.coder.tasks.push(WorkItemTask {
                task_id: "task_invented".to_string(),
                statement: "Invented".to_string(),
                requirement_refs: vec![],
                done_when_refs: vec![],
            });
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_missing_contract_ref",
        "reviewer.criterion_refs",
        Some("AC-001"),
        |value| {
            value.reviewer.criterion_refs.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_missing_contract_ref",
        "coder.acceptance_criteria",
        Some("AC-001"),
        |value| {
            value.coder.acceptance_criteria.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_invented_contract_ref",
        "reviewer.criterion_refs",
        Some("AC-INVENTED"),
        |value| {
            value
                .reviewer
                .criterion_refs
                .push("AC-INVENTED".to_string());
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_missing_contract_ref",
        "coder.inputs",
        Some("contract.source"),
        |value| {
            value.coder.required_input_contracts.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_invented_contract_ref",
        "reviewer.inputs",
        Some("contract.input.invented"),
        |value| {
            value
                .reviewer
                .input_contract_checks
                .push(RequiredInputContract {
                    contract_id: "contract.input.invented".to_string(),
                    ..contract.input_contracts[0].clone()
                });
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_missing_contract_ref",
        "reviewer.outputs",
        Some("contract.canonical"),
        |value| {
            value.reviewer.output_contract_checks.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_invented_contract_ref",
        "reviewer.outputs",
        Some("contract.invented"),
        |value| {
            value.reviewer.output_contract_checks[0].contract_id = "contract.invented".to_string();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_missing_contract_ref",
        "coder.verification",
        Some("check_canonical"),
        |value| {
            value.coder.verification_checks.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_invented_contract_ref",
        "reviewer.verification",
        Some("check_invented"),
        |value| {
            value
                .reviewer
                .verification_evidence_rules
                .push(VerificationCheck {
                    check_id: "check_invented".to_string(),
                    ..contract.verification_checks[0].clone()
                });
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_missing_contract_ref",
        "reviewer.blockers",
        Some("contract_invalid"),
        |value| {
            value.reviewer.blocker_routing.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_invented_contract_ref",
        "coder.blockers",
        Some("blocker_invented"),
        |value| {
            value.coder.blocker_rules.push(BlockerRule {
                reason_code: "blocker_invented".to_string(),
                ..contract.blocker_rules[0].clone()
            });
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_contract_mismatch",
        "coder",
        None,
        |value| {
            value.coder.write_policy.exclusive_scopes.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_contract_mismatch",
        "reviewer.scope",
        Some("wi_consumer"),
        |value| {
            value
                .reviewer
                .scope_policy
                .exclusive_scopes
                .push("invented/scope".to_string());
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_revision_binding_mismatch",
        "coder",
        Some("work_item_revision_0001"),
        |value| {
            value.coder.work_item_revision_id.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_revision_binding_mismatch",
        "reviewer",
        Some("work_item_revision_0001"),
        |value| {
            value.reviewer.work_item_revision_id = "different_revision".to_string();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "human_projection_invalid_flags",
        "human",
        None,
        |value| {
            value.human.normative = true;
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_invented_contract_ref",
        "human.source_refs",
        Some("invented_ref"),
        |value| {
            value.human.source_refs.push("invented_ref".to_string());
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_missing_contract_ref",
        "human.source_refs",
        Some("AC-001"),
        |value| {
            value.human.source_refs.clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_contract_mismatch",
        "coder.tasks",
        Some("task_1"),
        |value| {
            value.coder.tasks[0].statement = "Drifted task content".to_string();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_contract_mismatch",
        "reviewer.requirement_matrix",
        Some("AC-001"),
        |value| {
            value.reviewer.requirement_matrix[0]
                .required_evidence
                .clear();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_contract_mismatch",
        "human.source_refs",
        None,
        |value| {
            value.human.source_refs.reverse();
        },
    );
    assert_mutation_finding(
        &contract,
        &baseline,
        "projection_contract_mismatch",
        "human.source_refs",
        None,
        |value| {
            value
                .human
                .source_refs
                .push(value.human.source_refs[0].clone());
        },
    );
}

#[test]
fn work_item_projection_validation_findings_are_deterministic_and_deduplicated() {
    let contract = contract_fixture();
    let mut compiled = compiled_fixture();
    compiled
        .reviewer
        .criterion_refs
        .extend(["AC-INVENTED".to_string(), "AC-INVENTED".to_string()]);

    let first = validate_projection_coverage(&contract, "work_item_revision_0001", &compiled);
    let second = validate_projection_coverage(&contract, "work_item_revision_0001", &compiled);
    let invented = first
        .findings
        .iter()
        .filter(|finding| {
            finding.code == "projection_invented_contract_ref"
                && finding.projection == "reviewer.criterion_refs"
                && finding.contract_ref.as_deref() == Some("AC-INVENTED")
        })
        .count();

    assert_eq!(first, second);
    assert_eq!(invented, 1);
}

#[test]
fn work_item_projection_validation_rejects_same_wrong_revision_in_both_roles() {
    let contract = contract_fixture();
    let mut compiled = compiled_fixture();
    compiled.coder.work_item_revision_id = "work_item_revision_wrong".to_string();
    compiled.reviewer.work_item_revision_id = "work_item_revision_wrong".to_string();

    let report = validate_projection_coverage(&contract, "work_item_revision_0001", &compiled);
    for projection in ["coder", "reviewer"] {
        assert!(report.findings.iter().any(|finding| {
            finding.code == "projection_revision_binding_mismatch"
                && finding.projection == projection
                && finding.contract_ref.as_deref() == Some("work_item_revision_0001")
        }));
    }
}

#[test]
fn work_item_projection_validation_rejects_empty_revision_bindings() {
    let contract = contract_fixture();

    for revision_id in ["", "  \t"] {
        let Err(crate::product::work_item_projection::ProjectionCompileError::Validation(report)) =
            WorkItemProjectionCompiler.compile(&contract, revision_id)
        else {
            panic!("expected empty revision binding validation failure");
        };
        for projection in ["work_item.revision_binding", "coder", "reviewer"] {
            assert_finding(
                &report,
                "projection_revision_binding_invalid",
                projection,
                None,
            );
        }
    }

    let compiled = compiled_fixture();
    let report = validate_projection_coverage(&contract, "   ", &compiled);
    assert_finding(
        &report,
        "projection_revision_binding_invalid",
        "work_item.revision_binding",
        None,
    );
}

#[test]
fn work_item_projection_hashes_are_stable_lowercase_sha256() {
    let compiled = compiled_fixture();

    let first = projection_hashes(&compiled).unwrap();
    let rebuilt = serde_json::from_value(serde_json::to_value(&compiled.coder).unwrap()).unwrap();
    let mut equivalent = compiled;
    equivalent.coder = rebuilt;
    let second = projection_hashes(&equivalent).unwrap();

    assert_eq!(first, second);
    for hash in [&first.human, &first.coder, &first.reviewer] {
        assert_eq!(hash.len(), 64);
        assert!(
            hash.chars()
                .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
        );
    }
}

#[test]
fn work_item_projection_strong_bundles_and_validation_artifact_roundtrip() {
    let work_item = compiled_fixture();
    let hashes = projection_hashes(&work_item).unwrap();
    let work_item_bundle = WorkItemProjectionBundle {
        id: "work_item_projection_bundle_0001".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        canonical_contract_hash: "contract_hash".to_string(),
        projection_schema_version: 1,
        compiler_version: "compiler-v1".to_string(),
        human_projection: work_item.human.clone(),
        coder_projection: work_item.coder.clone(),
        reviewer_projection: work_item.reviewer.clone(),
        human_projection_hash: hashes.human,
        coder_projection_hash: hashes.coder,
        reviewer_projection_hash: hashes.reviewer,
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };
    assert_roundtrip(&work_item_bundle);

    let (graph, work_items) = compiled_plan_fixture();
    let plan = PlanProjectionCompiler
        .compile(PlanProjectionCompileInput {
            plan_id: "plan_0001",
            goal: "Compile plan",
            split_reason: "Contract boundaries",
            source_refs: vec!["design_0001".to_string()],
            dependency_graph: &graph,
            work_item_projections: &work_items,
            expected_work_item_revision_ids: super::expected_plan_revision_ids(),
        })
        .unwrap();
    let plan_bundle = PlanProjectionBundle {
        id: "plan_projection_bundle_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "graph_revision_0001".to_string(),
        work_item_projection_bundle_refs: vec![work_item_bundle.id.clone()],
        human_group_projection: plan.human,
        coder_group_context: plan.coder,
        reviewer_group_matrix: plan.reviewer,
        human_group_projection_hash: "human_group_hash".to_string(),
        coder_group_context_hash: "coder_group_hash".to_string(),
        reviewer_group_matrix_hash: "reviewer_group_hash".to_string(),
        compiler_version: "compiler-v1".to_string(),
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };
    assert_roundtrip(&plan_bundle);

    let artifact = PlanValidationReportArtifact {
        id: "validation_0001".to_string(),
        plan_id: "plan_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        contract_validation: ContractValidationReport { findings: vec![] },
        projection_validation: ProjectionValidationReport { findings: vec![] },
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };
    assert_roundtrip(&artifact);
}

#[test]
fn work_item_projection_strong_bundles_reject_legacy_untyped_json() {
    let legacy_work_item = serde_json::json!({
        "id": "bundle_0001",
        "work_item_revision_id": "revision_0001",
        "canonical_contract_hash": "hash",
        "projection_schema_version": 1,
        "compiler_version": "v1",
        "human_projection": {"title": "human"},
        "coder_projection": {"title": "coder"},
        "reviewer_projection": {"title": "reviewer"},
        "human_projection_hash": "human",
        "coder_projection_hash": "coder",
        "reviewer_projection_hash": "reviewer",
        "created_at": "2026-07-17T00:00:00Z"
    });
    assert!(serde_json::from_value::<WorkItemProjectionBundle>(legacy_work_item).is_err());

    let legacy_artifact = serde_json::json!({
        "id": "validation_0001",
        "plan_id": "plan_0001",
        "contract_validation": {"valid": true},
        "projection_validation": {"valid": true},
        "created_at": "2026-07-17T00:00:00Z"
    });
    assert!(serde_json::from_value::<PlanValidationReportArtifact>(legacy_artifact).is_err());

    let legacy_plan = serde_json::json!({
        "id": "plan_bundle_0001",
        "plan_revision_id": "plan_revision_0001",
        "dependency_graph_revision_id": "graph_revision_0001",
        "work_item_projection_bundle_refs": ["work_item_bundle_0001"],
        "human_group_projection": {"title": "human"},
        "coder_group_context": {"title": "coder"},
        "reviewer_group_matrix": {"title": "reviewer"},
        "human_group_projection_hash": "human",
        "coder_group_context_hash": "coder",
        "reviewer_group_matrix_hash": "reviewer",
        "compiler_version": "v1",
        "created_at": "2026-07-17T00:00:00Z"
    });
    assert!(serde_json::from_value::<PlanProjectionBundle>(legacy_plan).is_err());
}

fn assert_mutation_finding(
    contract: &crate::product::work_item_contract::CanonicalWorkItemContract,
    baseline: &crate::product::work_item_projection::CompiledWorkItemProjections,
    code: &str,
    projection: &str,
    contract_ref: Option<&str>,
    mutate: impl FnOnce(&mut crate::product::work_item_projection::CompiledWorkItemProjections),
) {
    let mut changed = baseline.clone();
    mutate(&mut changed);
    let report = validate_projection_coverage(contract, "work_item_revision_0001", &changed);
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == code
                && finding.projection == projection
                && finding.contract_ref.as_deref() == contract_ref
        }),
        "missing finding ({code}, {projection}, {contract_ref:?}): {:?}",
        report.findings
    );
}

fn assert_roundtrip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + PartialEq,
{
    let rebuilt = serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap();
    assert_eq!(value, &rebuilt);
}

fn assert_finding(
    report: &ProjectionValidationReport,
    code: &str,
    projection: &str,
    contract_ref: Option<&str>,
) {
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == code
                && finding.projection == projection
                && finding.contract_ref.as_deref() == contract_ref
        }),
        "missing finding ({code}, {projection}, {contract_ref:?}): {:?}",
        report.findings
    );
}
