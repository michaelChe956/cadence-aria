use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::product::work_item_contract::{CanonicalWorkItemContract, DependencyContractGraph};

use super::{
    CompiledPlanProjections, CompiledWorkItemProjections, ProjectionCompileError, ProjectionHashes,
    ProjectionValidationFinding, ProjectionValidationReport, compile_coder_projection,
    compile_human_projection, compile_reviewer_projection, contract_flow, design_traceability_refs,
    human_work_items, reviewer_work_items, risks_from_flow, stable_topological_order,
};

pub fn validate_projection_coverage(
    contract: &CanonicalWorkItemContract,
    compiled: &CompiledWorkItemProjections,
) -> ProjectionValidationReport {
    let mut findings = Vec::new();
    validate_revision_binding(compiled, &mut findings);
    validate_human(contract, compiled, &mut findings);
    validate_coder(contract, compiled, &mut findings);
    validate_reviewer(contract, compiled, &mut findings);
    finalized_report(findings)
}

pub fn validate_plan_projection_coverage(
    graph: &DependencyContractGraph,
    compiled: &CompiledPlanProjections,
    work_items: &BTreeMap<String, CompiledWorkItemProjections>,
) -> ProjectionValidationReport {
    let mut findings = Vec::new();
    let ordered_ids = match stable_topological_order(graph, work_items) {
        Ok(ids) => Some(ids),
        Err(ProjectionCompileError::Validation(report)) => {
            findings.extend(report.findings);
            None
        }
        Err(error) => {
            push(
                &mut findings,
                "plan_projection_work_item_mismatch",
                "plan",
                None,
                error.to_string(),
            );
            None
        }
    };

    if compiled.human.plan_id != compiled.coder.plan_id
        || compiled.human.plan_id != compiled.reviewer.plan_id
    {
        push(
            &mut findings,
            "plan_projection_work_item_mismatch",
            "plan",
            None,
            "role projections bind different plan IDs",
        );
    }
    if compiled.human.normative || compiled.human.used_by_provider {
        push(
            &mut findings,
            "human_projection_invalid_flags",
            "plan.human",
            None,
            "human plan projection must be informative only",
        );
    }

    let expected_contract_flow = contract_flow(graph);
    if compiled.coder.dependency_edges != graph.edges
        || compiled.reviewer.dependency_edges != graph.edges
        || compiled.human.contract_flow != expected_contract_flow
    {
        push(
            &mut findings,
            "plan_projection_edge_mismatch",
            "plan",
            None,
            "plan projections do not preserve dependency graph edges and contract flow",
        );
    }
    if compiled.human.risks != risks_from_flow(&expected_contract_flow) {
        push(
            &mut findings,
            "plan_projection_risk_mismatch",
            "plan.human",
            None,
            "plan risks must be derived only from structured contract capability gaps",
        );
    }

    if let Some(ordered_ids) = ordered_ids {
        let human_ids = compiled
            .human
            .work_items
            .iter()
            .map(|item| item.logical_work_item_id.clone())
            .collect::<Vec<_>>();
        let reviewer_ids = compiled
            .reviewer
            .work_items
            .iter()
            .map(|item| item.logical_work_item_id.clone())
            .collect::<Vec<_>>();
        if compiled.coder.ordered_logical_work_item_ids != ordered_ids
            || human_ids != ordered_ids
            || reviewer_ids != ordered_ids
            || compiled.human.work_items != human_work_items(&ordered_ids, graph, work_items)
        {
            push(
                &mut findings,
                "plan_projection_work_item_mismatch",
                "plan",
                None,
                "plan projections do not contain the same stable work item order and summaries",
            );
        }

        let expected_scopes = ordered_ids
            .iter()
            .map(|logical_id| {
                (
                    logical_id.clone(),
                    work_items[logical_id].coder.write_policy.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if compiled.coder.group_write_scopes != expected_scopes
            || compiled.reviewer.work_items != reviewer_work_items(&ordered_ids, work_items)
            || compiled.reviewer.design_traceability_refs != design_traceability_refs(graph)
        {
            push(
                &mut findings,
                "plan_projection_matrix_mismatch",
                "plan",
                None,
                "plan coder scopes or reviewer matrix differ from work item projections",
            );
        }
    }

    finalized_report(findings)
}

pub fn projection_hashes(
    compiled: &CompiledWorkItemProjections,
) -> Result<ProjectionHashes, ProjectionCompileError> {
    Ok(ProjectionHashes {
        human: projection_hash(&compiled.human)?,
        coder: projection_hash(&compiled.coder)?,
        reviewer: projection_hash(&compiled.reviewer)?,
    })
}

fn validate_revision_binding(
    compiled: &CompiledWorkItemProjections,
    findings: &mut Vec<ProjectionValidationFinding>,
) {
    if compiled.coder.work_item_revision_id.is_empty()
        || compiled.reviewer.work_item_revision_id.is_empty()
        || compiled.coder.work_item_revision_id != compiled.reviewer.work_item_revision_id
    {
        push(
            findings,
            "projection_revision_binding_mismatch",
            "work_item",
            None,
            "coder and reviewer projections must bind the same non-empty revision ID",
        );
    }
}

fn validate_human(
    contract: &CanonicalWorkItemContract,
    compiled: &CompiledWorkItemProjections,
    findings: &mut Vec<ProjectionValidationFinding>,
) {
    let expected = compile_human_projection(contract);
    let actual = &compiled.human;
    if actual.normative || actual.used_by_provider {
        push(
            findings,
            "human_projection_invalid_flags",
            "human",
            None,
            "human work item projection must be informative only",
        );
    }
    if actual.logical_work_item_id != expected.logical_work_item_id
        || actual.title != expected.title
        || actual.goal != expected.goal
        || actual.non_goals != expected.non_goals
        || actual.dependencies != expected.dependencies
        || actual.scope_summary != expected.scope_summary
        || actual.completion_summary != expected.completion_summary
    {
        push(
            findings,
            "projection_contract_mismatch",
            "human",
            None,
            "human projection content differs from the canonical contract",
        );
    }
    report_section(
        &expected.inputs,
        &actual.inputs,
        |value| value.contract_id.as_str(),
        "human.inputs",
        findings,
    );
    report_section(
        &expected.outputs,
        &actual.outputs,
        |value| value.contract_id.as_str(),
        "human.outputs",
        findings,
    );
    report_string_refs(
        &expected.source_refs,
        &actual.source_refs,
        "human.source_refs",
        findings,
    );
}

fn validate_coder(
    contract: &CanonicalWorkItemContract,
    compiled: &CompiledWorkItemProjections,
    findings: &mut Vec<ProjectionValidationFinding>,
) {
    let actual = &compiled.coder;
    let expected = compile_coder_projection(contract, &actual.work_item_revision_id);
    if actual.objective != expected.objective
        || actual.write_policy != expected.write_policy
        || actual.handoff_contract != expected.handoff_contract
    {
        push(
            findings,
            "projection_contract_mismatch",
            "coder",
            None,
            "coder projection scalar, scope, or handoff content differs from the canonical contract",
        );
    }
    report_section(
        &expected.required_input_contracts,
        &actual.required_input_contracts,
        |value| value.contract_id.as_str(),
        "coder.inputs",
        findings,
    );
    report_string_refs(
        &expected.task_refs,
        &actual.task_refs,
        "coder.task_refs",
        findings,
    );
    report_section(
        &expected.tasks,
        &actual.tasks,
        |value| value.task_id.as_str(),
        "coder.tasks",
        findings,
    );
    report_section(
        &expected.acceptance_criteria,
        &actual.acceptance_criteria,
        |value| value.criterion_id.as_str(),
        "coder.acceptance_criteria",
        findings,
    );
    report_section(
        &expected.verification_checks,
        &actual.verification_checks,
        |value| value.check_id.as_str(),
        "coder.verification",
        findings,
    );
    report_section(
        &expected.blocker_rules,
        &actual.blocker_rules,
        |value| value.reason_code.as_str(),
        "coder.blockers",
        findings,
    );
}

fn validate_reviewer(
    contract: &CanonicalWorkItemContract,
    compiled: &CompiledWorkItemProjections,
    findings: &mut Vec<ProjectionValidationFinding>,
) {
    let actual = &compiled.reviewer;
    let expected = compile_reviewer_projection(contract, &actual.work_item_revision_id);
    if actual.scope_policy != expected.scope_policy {
        push(
            findings,
            "projection_contract_mismatch",
            "reviewer.scope",
            None,
            "reviewer scope differs from the canonical write policy",
        );
    }
    report_string_refs(
        &expected.criterion_refs,
        &actual.criterion_refs,
        "reviewer.criterion_refs",
        findings,
    );
    report_section(
        &expected.requirement_matrix,
        &actual.requirement_matrix,
        |value| value.criterion_id.as_str(),
        "reviewer.requirement_matrix",
        findings,
    );
    report_section(
        &expected.input_contract_checks,
        &actual.input_contract_checks,
        |value| value.contract_id.as_str(),
        "reviewer.inputs",
        findings,
    );
    report_section(
        &expected.output_contract_checks,
        &actual.output_contract_checks,
        |value| value.contract_id.as_str(),
        "reviewer.outputs",
        findings,
    );
    report_section(
        &expected.verification_evidence_rules,
        &actual.verification_evidence_rules,
        |value| value.check_id.as_str(),
        "reviewer.verification",
        findings,
    );
    report_section(
        &expected.blocker_routing,
        &actual.blocker_routing,
        |value| value.reason_code.as_str(),
        "reviewer.blockers",
        findings,
    );
}

fn report_string_refs(
    expected: &[String],
    actual: &[String],
    projection: &str,
    findings: &mut Vec<ProjectionValidationFinding>,
) {
    report_section(expected, actual, String::as_str, projection, findings);
}

fn report_section<T, F>(
    expected: &[T],
    actual: &[T],
    id: F,
    projection: &str,
    findings: &mut Vec<ProjectionValidationFinding>,
) where
    T: PartialEq,
    F: Fn(&T) -> &str,
{
    let expected_ids = expected.iter().map(&id).collect::<BTreeSet<_>>();
    let actual_ids = actual.iter().map(&id).collect::<BTreeSet<_>>();
    for missing in expected_ids.difference(&actual_ids) {
        push(
            findings,
            "projection_missing_contract_ref",
            projection,
            Some((*missing).to_string()),
            format!("{projection} is missing canonical ref {missing}"),
        );
    }
    for invented in actual_ids.difference(&expected_ids) {
        push(
            findings,
            "projection_invented_contract_ref",
            projection,
            Some((*invented).to_string()),
            format!("{projection} contains invented ref {invented}"),
        );
    }
    for shared in expected_ids.intersection(&actual_ids) {
        let expected_value = expected.iter().find(|value| id(value) == *shared);
        let actual_value = actual.iter().find(|value| id(value) == *shared);
        if expected_value != actual_value {
            push(
                findings,
                "projection_contract_mismatch",
                projection,
                Some((*shared).to_string()),
                format!("{projection} content for {shared} differs from the canonical contract"),
            );
        }
    }
    if expected_ids == actual_ids && expected != actual {
        push(
            findings,
            "projection_contract_mismatch",
            projection,
            None,
            format!("{projection} order or duplication differs from the canonical contract"),
        );
    }
}

fn projection_hash<T: Serialize>(value: &T) -> Result<String, ProjectionCompileError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ProjectionCompileError::Serialization(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn push(
    findings: &mut Vec<ProjectionValidationFinding>,
    code: &str,
    projection: &str,
    contract_ref: Option<String>,
    message: impl Into<String>,
) {
    findings.push(ProjectionValidationFinding {
        code: code.to_string(),
        projection: projection.to_string(),
        contract_ref,
        message: message.into(),
    });
}

fn finalized_report(mut findings: Vec<ProjectionValidationFinding>) -> ProjectionValidationReport {
    findings.sort_by(|left, right| {
        (
            &left.code,
            &left.projection,
            &left.contract_ref,
            &left.message,
        )
            .cmp(&(
                &right.code,
                &right.projection,
                &right.contract_ref,
                &right.message,
            ))
    });
    findings.dedup();
    ProjectionValidationReport { findings }
}
