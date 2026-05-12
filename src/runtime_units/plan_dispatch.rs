use crate::cross_cutting::artifact_projection::compile_plan_projection;
use crate::cross_cutting::artifact_validate::{
    ArtifactContent, ArtifactIndex, ConstraintBundleIndex, ProjectionIndex, ProviderRunIndex,
    TraceabilityIndex, canonical_validator, phase1_profile_validator, projection_validator,
};
use crate::cross_cutting::document_ops::read_document_model;
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef};
use crate::protocol::constraints::{BundleStatus, OpenSpecConstraintBundle};
use crate::protocol::contracts::ProviderRunRecord;
use crate::protocol::loop_counters::{LoopCounterName, LoopCounterRegistry};
use crate::protocol::phase1_profile::PHASE1_PROFILE_VERSION;
use crate::protocol::projections::{
    ArtifactProjectionRecord, ProjectionPayload, WorkPackageProjection,
};
use crate::runtime_units::clarification::{
    PlanningChainState, PlanningNodeTrace, PlanningStartChainInput, PlanningUnitError,
    provider_run_id, record_protocol_step, requirement_constraint_summary, run_clarification,
    run_provider_node, structured_output, write_checkpoint, write_json_artifact,
    write_markdown_artifact, write_tasks_to_openspec_and_recompile,
};
use crate::runtime_units::design_authoring::run_design_authoring;
use crate::runtime_units::design_review::{DesignReviewRoute, run_design_review};
use crate::runtime_units::design_revision::run_design_revision;
use crate::runtime_units::spec_authoring::run_spec_authoring;
use crate::runtime_units::spec_gate_review::run_spec_gate_review;
use crate::runtime_units::{
    CanonicalNodeInput, DaemonContext, RuntimeProtocolStep, RuntimeUnit, RuntimeUnitError,
    RuntimeUnitResult,
};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanDispatchUnit;

impl RuntimeUnit for PlanDispatchUnit {
    fn unit_id(&self) -> &'static str {
        "plan_dispatch"
    }

    fn covered_protocol_nodes(&self) -> Vec<&'static str> {
        vec!["N10", "N11", "N12"]
    }

    async fn execute(
        &self,
        _input: CanonicalNodeInput,
        _ctx: &DaemonContext,
    ) -> Result<RuntimeUnitResult, RuntimeUnitError> {
        Err(RuntimeUnitError {
            code: "provider_adapter_required".to_string(),
            message: "N10-N12 require ProviderAdapter injection via run_planning_full_chain"
                .to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct PlanningFullChainResult {
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub provider_run_records: Vec<ProviderRunRecord>,
    pub checkpoint_paths: Vec<PathBuf>,
    pub node_traces: Vec<PlanningNodeTrace>,
    pub design_review: Value,
    pub design_revision_record: Option<Value>,
    pub design_markdown: String,
    pub initial_design_ref: ArtifactRef,
    pub design_ref: ArtifactRef,
    pub design_projection: ArtifactProjectionRecord,
    pub design_writeback_stale_status: BundleStatus,
    pub openspec_bundle_after_design: OpenSpecConstraintBundle,
    pub readiness_check: Value,
    pub plan_markdown: String,
    pub plan_ref: ArtifactRef,
    pub plan_projection: ArtifactProjectionRecord,
    pub tasks_writeback_stale_status: BundleStatus,
    pub openspec_bundle_after_tasks: OpenSpecConstraintBundle,
    pub dispatch_package: Value,
    pub dispatch_ref: ArtifactRef,
    pub superseded_artifact_refs: Vec<String>,
}

pub fn run_planning_full_chain(
    input: PlanningStartChainInput,
    provider: &dyn ProviderAdapter,
) -> Result<PlanningFullChainResult, PlanningUnitError> {
    let mut state = PlanningChainState::new(input);
    let (clarification_record, _clarification_ref) = run_clarification(&mut state, provider)?;
    let (spec_markdown, _spec_ref, spec_projection) =
        run_spec_authoring(&mut state, provider, &clarification_record)?;
    let (spec_gate_decision, _spec_gate_decision_ref, _spec_stale_status, _bundle_after_spec) =
        run_spec_gate_review(
            &mut state,
            provider,
            &clarification_record,
            &spec_markdown,
            &spec_projection,
        )?;
    let (mut design_markdown, initial_design_ref, mut design_projection) = run_design_authoring(
        &mut state,
        provider,
        &spec_markdown,
        &spec_gate_decision,
        spec_projection.projection_id.clone(),
    )?;
    let mut design_ref = initial_design_ref.clone();
    let mut design_revision_record = None;
    let mut design_revision_counter = 0_u32;
    let design_revision_threshold =
        LoopCounterRegistry::phase1().threshold(LoopCounterName::DesignRevision);

    let (design_review, design_writeback_stale_status, openspec_bundle_after_design) = loop {
        match run_design_review(
            &mut state,
            provider,
            &spec_projection,
            &design_markdown,
            &design_projection,
        )? {
            DesignReviewRoute::Pass {
                review,
                stale_status,
                bundle_after_design,
                ..
            } => break (review, stale_status, bundle_after_design),
            DesignReviewRoute::Revise { review, .. } => {
                design_revision_counter += 1;
                if design_revision_counter > design_revision_threshold {
                    return Err(PlanningUnitError::DesignRevisionLimitExceeded {
                        current: design_revision_counter,
                        threshold: design_revision_threshold,
                    });
                }
                let revision = run_design_revision(
                    &mut state,
                    provider,
                    &spec_projection,
                    &review,
                    &design_ref,
                    &design_projection,
                )?;
                design_revision_record = Some(revision.revision_record);
                design_markdown = revision.revised_design_markdown;
                design_ref = revision.revised_design_ref;
                design_projection = revision.revised_design_projection;
            }
        }
    };

    let (readiness_check, _readiness_ref) =
        run_readiness_check(&mut state, provider, &spec_projection, &design_projection)?;
    let (plan_markdown, plan_ref, plan_projection, tasks_stale_status, bundle_after_tasks) =
        run_plan_authoring(&mut state, provider, &spec_projection, &design_projection)?;
    let (dispatch_package, dispatch_ref) =
        run_dispatch_authoring(&mut state, provider, &plan_projection)?;

    Ok(PlanningFullChainResult {
        protocol_steps: state.protocol_steps,
        provider_run_records: state.provider_run_records,
        checkpoint_paths: state.checkpoint_paths,
        node_traces: state.node_traces,
        design_review,
        design_revision_record,
        design_markdown,
        initial_design_ref,
        design_ref,
        design_projection,
        design_writeback_stale_status,
        openspec_bundle_after_design,
        readiness_check,
        plan_markdown,
        plan_ref,
        plan_projection,
        tasks_writeback_stale_status: tasks_stale_status,
        openspec_bundle_after_tasks: bundle_after_tasks,
        dispatch_package,
        dispatch_ref,
        superseded_artifact_refs: state.superseded_artifact_refs,
    })
}

fn run_readiness_check(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
    spec_projection: &ArtifactProjectionRecord,
    design_projection: &ArtifactProjectionRecord,
) -> Result<(Value, ArtifactRef), PlanningUnitError> {
    let output = run_provider_node(
        state,
        provider,
        "N10",
        json!({
            "spec_projection_ref": spec_projection.projection_id,
            "spec_projection_payload": spec_projection.payload.clone(),
            "design_projection_ref": design_projection.projection_id,
            "design_projection_payload": design_projection.payload.clone(),
            "constraint_bundle_ref": state.current_bundle.constraint_bundle_id,
        }),
        spec_design_projection_summary(spec_projection, design_projection)?,
        vec![
            spec_projection.projection_id.clone(),
            design_projection.projection_id.clone(),
        ],
        requirement_and_design_constraint_summary(&state.current_bundle),
        Vec::new(),
    )?;
    let readiness = structured_output("N10", &output)?;
    canonical_validator(
        ArtifactKind::ReadinessCheck,
        &ArtifactContent::Json(readiness.clone()),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let readiness_ref =
        write_json_artifact(state, ArtifactKind::ReadinessCheck, "N10", &readiness)?;
    let checkpoint_path = write_checkpoint(
        state,
        "N10",
        "planning",
        vec![readiness_ref.artifact_ref_id.clone()],
        vec![
            spec_projection.projection_id.clone(),
            design_projection.projection_id.clone(),
        ],
        json!({
            "readiness_check_ref": readiness_ref.artifact_ref_id,
        }),
    )?;
    record_protocol_step(
        state,
        "N10",
        vec![
            "requirement_constraints".to_string(),
            "design_constraints".to_string(),
        ],
        vec!["readiness_check".to_string()],
        checkpoint_path,
    );
    Ok((readiness, readiness_ref))
}

fn run_plan_authoring(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
    spec_projection: &ArtifactProjectionRecord,
    design_projection: &ArtifactProjectionRecord,
) -> Result<
    (
        String,
        ArtifactRef,
        ArtifactProjectionRecord,
        BundleStatus,
        OpenSpecConstraintBundle,
    ),
    PlanningUnitError,
> {
    let output = run_provider_node(
        state,
        provider,
        "N11",
        json!({
            "spec_projection_ref": spec_projection.projection_id,
            "spec_projection_payload": spec_projection.payload.clone(),
            "design_projection_ref": design_projection.projection_id,
            "design_projection_payload": design_projection.payload.clone(),
            "constraint_bundle_ref": state.current_bundle.constraint_bundle_id,
        }),
        spec_design_projection_summary(spec_projection, design_projection)?,
        vec![
            spec_projection.projection_id.clone(),
            design_projection.projection_id.clone(),
        ],
        requirement_and_design_constraint_summary(&state.current_bundle),
        Vec::new(),
    )?;
    let plan_markdown = markdown_from_plan_output(&output)?;
    canonical_validator(
        ArtifactKind::Plan,
        &ArtifactContent::Markdown(plan_markdown.clone()),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let plan_ref = write_markdown_artifact(state, ArtifactKind::Plan, "N11", &plan_markdown)?;
    let source = read_document_model(std::path::Path::new(&plan_ref.path))
        .map_err(|error| PlanningUnitError::Io(error.to_string()))?;
    let plan_projection = compile_plan_projection(&source, &plan_ref, "N11".to_string())
        .map_err(PlanningUnitError::ProjectionCompile)?;
    projection_validator(
        &plan_projection,
        &ArtifactIndex::from_active_refs(vec![plan_ref.clone()]),
        None,
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    write_projection(state, &plan_projection)?;
    let tasks_markdown = tasks_markdown_from_plan_projection(&plan_projection)?;
    let (stale_status, bundle_after_tasks) = write_tasks_to_openspec_and_recompile(
        state,
        &tasks_markdown,
        vec![
            spec_projection.projection_id.clone(),
            design_projection.projection_id.clone(),
            plan_projection.projection_id.clone(),
        ],
    )?;
    let checkpoint_path = write_checkpoint(
        state,
        "N11",
        "dispatch",
        vec![plan_ref.artifact_ref_id.clone()],
        vec![plan_projection.projection_id.clone()],
        json!({
            "plan_ref": plan_ref.artifact_ref_id,
            "plan_projection_ref": plan_projection.projection_id,
            "openspec_bundle_ref": bundle_after_tasks.constraint_bundle_id,
            "stale_status_after_tasks_write": stale_status,
        }),
    )?;
    record_protocol_step(
        state,
        "N11",
        vec![
            "requirement_constraints".to_string(),
            "design_constraints".to_string(),
        ],
        vec![
            "plan".to_string(),
            "plan_projection".to_string(),
            "openspec_tasks_writeback".to_string(),
            "openspec_constraint_bundle".to_string(),
        ],
        checkpoint_path,
    );
    Ok((
        plan_markdown,
        plan_ref,
        plan_projection,
        stale_status,
        bundle_after_tasks,
    ))
}

fn run_dispatch_authoring(
    state: &mut PlanningChainState,
    provider: &dyn ProviderAdapter,
    plan_projection: &ArtifactProjectionRecord,
) -> Result<(Value, ArtifactRef), PlanningUnitError> {
    let output = run_provider_node(
        state,
        provider,
        "N12",
        json!({
            "plan_projection_ref": plan_projection.projection_id,
            "plan_projection_payload": plan_projection.payload.clone(),
            "constraint_bundle_ref": state.current_bundle.constraint_bundle_id,
        }),
        plan_projection_summary(plan_projection)?,
        vec![plan_projection.projection_id.clone()],
        task_constraint_summary(&state.current_bundle),
        Vec::new(),
    )?;
    let candidate = structured_output("N12", &output)?;
    canonical_validator(
        ArtifactKind::DispatchPackage,
        &ArtifactContent::Json(candidate),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let routing = worktask_routing_from_plan_projection(plan_projection)?;
    let traceability_refs = routing_traceability_refs(&routing);
    let dispatch_package = json!({
        "artifact_kind": "dispatch_package",
        "worktask_routing": routing,
        "_aria": {
            "profile_version": PHASE1_PROFILE_VERSION,
            "constraint_check_ref": state.current_bundle.constraint_bundle_id,
            "traceability_refs": traceability_refs,
            "provider_run_refs": [provider_run_id(&state.input.task_id, "N12")],
            "projection_refs": [plan_projection.projection_id.clone()],
            "worktask_routing": routing,
        }
    });
    let work_package_ids = plan_work_package_ids(plan_projection)?;
    phase1_profile_validator(
        &dispatch_package,
        ArtifactKind::DispatchPackage,
        &ProjectionIndex::with_work_packages(
            vec![plan_projection.projection_id.clone()],
            work_package_ids,
        ),
        &ConstraintBundleIndex::with_checks(vec![
            state.current_bundle.constraint_bundle_id.clone(),
        ]),
        &TraceabilityIndex::with_known_refs(traceability_refs),
        &ProviderRunIndex::with_runs(vec![provider_run_id(&state.input.task_id, "N12")]),
    )
    .map_err(PlanningUnitError::ArtifactValidate)?;
    let dispatch_ref = write_json_artifact(
        state,
        ArtifactKind::DispatchPackage,
        "N12",
        &dispatch_package,
    )?;
    let checkpoint_path = write_checkpoint(
        state,
        "N12",
        "execution",
        vec![dispatch_ref.artifact_ref_id.clone()],
        vec![plan_projection.projection_id.clone()],
        json!({
            "dispatch_package_ref": dispatch_ref.artifact_ref_id,
        }),
    )?;
    record_protocol_step(
        state,
        "N12",
        vec!["task_constraints".to_string()],
        vec!["dispatch_package".to_string()],
        checkpoint_path,
    );
    Ok((dispatch_package, dispatch_ref))
}

fn spec_design_projection_summary(
    spec_projection: &ArtifactProjectionRecord,
    design_projection: &ArtifactProjectionRecord,
) -> Result<String, PlanningUnitError> {
    Ok(format!(
        "[spec_projection_ref]\n{}\n\n[spec_projection_payload]\n{}\n\n[design_projection_ref]\n{}\n\n[design_projection_payload]\n{}",
        spec_projection.projection_id,
        serde_json::to_string_pretty(&spec_projection.payload)
            .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?,
        design_projection.projection_id,
        serde_json::to_string_pretty(&design_projection.payload)
            .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?
    ))
}

fn plan_projection_summary(
    plan_projection: &ArtifactProjectionRecord,
) -> Result<String, PlanningUnitError> {
    Ok(format!(
        "[plan_projection_ref]\n{}\n\n[plan_projection_payload]\n{}",
        plan_projection.projection_id,
        serde_json::to_string_pretty(&plan_projection.payload)
            .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?
    ))
}

fn markdown_from_plan_output(
    output: &crate::protocol::contracts::AdapterOutput,
) -> Result<String, PlanningUnitError> {
    let value = structured_output("N11", output)?;
    let got = value
        .get("artifact_kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if got != "plan" {
        return Err(PlanningUnitError::IncompatibleOutput {
            node_id: "N11".to_string(),
            expected: "plan".to_string(),
            got: got.to_string(),
        });
    }
    value
        .get("markdown")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| PlanningUnitError::IncompatibleOutput {
            node_id: "N11".to_string(),
            expected: "markdown".to_string(),
            got: "missing".to_string(),
        })
}

fn write_projection(
    state: &PlanningChainState,
    projection: &ArtifactProjectionRecord,
) -> Result<(), PlanningUnitError> {
    let dir = state.task_root().join("projections");
    std::fs::create_dir_all(&dir)
        .map_err(|error| PlanningUnitError::Io(format!("create projections dir: {error}")))?;
    let path = dir.join(format!("{}.json", projection.projection_id));
    let bytes = serde_json::to_vec_pretty(projection)
        .map_err(|error| PlanningUnitError::Serialization(error.to_string()))?;
    std::fs::write(&path, bytes)
        .map_err(|error| PlanningUnitError::Io(format!("write {}: {error}", path.display())))
}

fn tasks_markdown_from_plan_projection(
    plan_projection: &ArtifactProjectionRecord,
) -> Result<String, PlanningUnitError> {
    let ProjectionPayload::PlanProjection(plan) = &plan_projection.payload else {
        return Err(PlanningUnitError::IncompatibleOutput {
            node_id: "N11".to_string(),
            expected: "plan_projection".to_string(),
            got: format!("{:?}", plan_projection.projection_kind),
        });
    };
    let mut output = String::from("# Tasks\n\n");
    for work_package in &plan.work_packages {
        let task_id = first_ref_with_prefix(&work_package.traceability_refs, "task-")
            .map(uppercase_id)
            .unwrap_or_else(|| uppercase_id(&work_package.work_package_id.replace("wt-", "task-")));
        let reqs = refs_with_prefix(&work_package.traceability_refs, "req-");
        let designs = refs_with_prefix_any(&work_package.traceability_refs, &["dd-", "dec-"]);
        let acceptance = work_package
            .acceptance_targets
            .iter()
            .map(|value| uppercase_id(value))
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str("- [ ] ");
        output.push_str(&task_id);
        output.push(' ');
        output.push_str(&work_package.description);
        output.push_str(". Reqs: ");
        output.push_str(&reqs.join(", "));
        output.push_str("; Designs: ");
        output.push_str(&designs.join(", "));
        output.push_str("; Acceptance: ");
        output.push_str(&acceptance);
        output.push('\n');
    }
    Ok(output)
}

fn worktask_routing_from_plan_projection(
    plan_projection: &ArtifactProjectionRecord,
) -> Result<Value, PlanningUnitError> {
    let ProjectionPayload::PlanProjection(plan) = &plan_projection.payload else {
        return Err(PlanningUnitError::IncompatibleOutput {
            node_id: "N12".to_string(),
            expected: "plan_projection".to_string(),
            got: format!("{:?}", plan_projection.projection_kind),
        });
    };
    Ok(Value::Array(
        plan.work_packages
            .iter()
            .map(|work_package| {
                let allowed_write_scope = allowed_write_scope_for_work_package(work_package);
                json!({
                    "worktask_id": format!("work_{}", work_package.work_package_id.replace('-', "_")),
                    "source_work_package_id": work_package.work_package_id,
                    "execution_mode": work_package.execution_mode.to_string(),
                    "human_required_reason": work_package.human_required_reason.clone(),
                    "allowed_write_scope": allowed_write_scope,
                    "traceability_refs": work_package.traceability_refs.clone(),
                    "verification_commands": [],
                })
            })
            .collect(),
    ))
}

fn allowed_write_scope_for_work_package(work_package: &WorkPackageProjection) -> Vec<&'static str> {
    let mut scope = vec!["src/", "tests/"];
    let description = work_package.description.to_ascii_lowercase();
    if description.contains("package.json") {
        scope.push("package.json");
    }
    scope
}

fn plan_work_package_ids(
    plan_projection: &ArtifactProjectionRecord,
) -> Result<Vec<String>, PlanningUnitError> {
    let ProjectionPayload::PlanProjection(plan) = &plan_projection.payload else {
        return Err(PlanningUnitError::IncompatibleOutput {
            node_id: "N12".to_string(),
            expected: "plan_projection".to_string(),
            got: format!("{:?}", plan_projection.projection_kind),
        });
    };
    Ok(plan
        .work_packages
        .iter()
        .map(|work_package| work_package.work_package_id.clone())
        .collect())
}

fn routing_traceability_refs(routing: &Value) -> Vec<String> {
    let mut refs = Vec::new();
    for item in routing.as_array().into_iter().flatten() {
        for value in item
            .get("traceability_refs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(ref_id) = value.as_str() {
                let ref_id = ref_id.to_string();
                if !refs.contains(&ref_id) {
                    refs.push(ref_id);
                }
            }
        }
    }
    refs
}

fn requirement_and_design_constraint_summary(bundle: &OpenSpecConstraintBundle) -> String {
    format!(
        "{}; design_decision_ids={}",
        requirement_constraint_summary(bundle),
        bundle.design_constraints.design_decision_ids.join(",")
    )
}

fn task_constraint_summary(bundle: &OpenSpecConstraintBundle) -> String {
    format!("task_ids={}", bundle.task_constraints.task_ids.join(","))
}

fn first_ref_with_prefix<'a>(refs: &'a [String], prefix: &str) -> Option<&'a str> {
    refs.iter()
        .map(String::as_str)
        .find(|value| value.starts_with(prefix))
}

fn refs_with_prefix(refs: &[String], prefix: &str) -> Vec<String> {
    refs.iter()
        .filter(|value| value.starts_with(prefix))
        .map(|value| uppercase_id(value))
        .collect()
}

fn refs_with_prefix_any(refs: &[String], prefixes: &[&str]) -> Vec<String> {
    refs.iter()
        .filter(|value| prefixes.iter().any(|prefix| value.starts_with(prefix)))
        .map(|value| uppercase_id(value))
        .collect()
}

fn uppercase_id(value: &str) -> String {
    value.to_ascii_uppercase()
}
