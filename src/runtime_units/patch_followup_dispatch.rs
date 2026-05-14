use crate::cross_cutting::artifact_validate::{
    ConstraintBundleIndex, ProjectionIndex, ProviderRunIndex, TraceabilityIndex,
    phase1_profile_validator,
};
use crate::cross_cutting::document_ops::{read_document_model, upsert_section};
use crate::cross_cutting::openspec_constraints::{
    OpenSpecError, build_openspec_source_manifest, check_bundle_stale, compile_constraint_bundle,
};
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::protocol::artifacts::ArtifactKind;
use crate::protocol::constraints::{BundleStatus, OpenSpecConstraintBundle};
use crate::protocol::document_ops::{DocumentBlock, HeadingPath};
use crate::protocol::phase1_profile::PHASE1_PROFILE_VERSION;
use crate::runtime_units::final_review::{
    FinalClosureError, FinalClosureInput, normalize_final_review_profile, run_final_provider_node,
    session_closeout_step,
};
use crate::runtime_units::{RuntimeProtocolStep, RuntimeStepStatus};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved { approved_by: String },
    Rejected { rejected_by: String, reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalFollowupInput {
    pub session_id: String,
    pub task_id: String,
    pub worktree_path: String,
    pub change_id: String,
    pub change_dir: PathBuf,
    pub projection_refs: Vec<String>,
    pub constraint_bundle_ref: String,
    pub current_bundle: OpenSpecConstraintBundle,
    pub risk_registry_ref: String,
    pub canonical_artifact_refs: Vec<String>,
    pub traceability_refs: Vec<String>,
    pub context_files: Vec<String>,
    pub patch_round_counter: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalFollowupResult {
    pub protocol_steps: Vec<RuntimeProtocolStep>,
    pub final_review: Value,
    pub final_summary: Value,
    pub new_dispatch_package: Value,
    pub recompiled_bundle: Option<OpenSpecConstraintBundle>,
    pub patch_round_counter: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum FinalFollowupError {
    #[error(transparent)]
    FinalClosure(#[from] FinalClosureError),
    #[error("OpenSpec operation failed: {0}")]
    OpenSpec(#[from] OpenSpecError),
    #[error("document operation failed: {0}")]
    Document(#[from] crate::cross_cutting::document_ops::DocumentOpError),
    #[error("provider structured output missing for N26")]
    StructuredOutputMissing,
    #[error("invalid patch task delta: {0}")]
    InvalidPatchTaskDelta(String),
}

pub fn run_final_followup_route(
    input: FinalFollowupInput,
    provider: &dyn ProviderAdapter,
    approval: ApprovalDecision,
) -> Result<FinalFollowupResult, FinalFollowupError> {
    let closure_input = closure_input(&input);
    let mut records = Vec::new();
    let mut final_review = run_final_provider_node(
        &closure_input,
        provider,
        "N25",
        ArtifactKind::FinalReview,
        &mut records,
    )?;
    normalize_final_review_profile(&closure_input, &mut final_review, &records)?;
    let mut protocol_steps = vec![
        RuntimeProtocolStep {
            node_id: "N25".to_string(),
            status: RuntimeStepStatus::Completed,
            node_specific_fields: json!({
                "overall_decision": final_review["overall_decision"],
                "coverage_summary": final_review["_aria"]["coverage_summary"],
                "uncovered_items": final_review["uncovered_items"],
                "manual_exemptions": [],
            }),
        },
        RuntimeProtocolStep {
            node_id: "X01".to_string(),
            status: RuntimeStepStatus::Blocked,
            node_specific_fields: json!({
                "gate_kind": "patch_followup_dispatch",
                "overall_decision": final_review["overall_decision"],
            }),
        },
    ];

    match approval {
        ApprovalDecision::Approved { approved_by } => {
            let n26 = run_final_provider_node(
                &closure_input,
                provider,
                "N26",
                ArtifactKind::DispatchPackage,
                &mut records,
            )?;
            let deltas = n26
                .get("patch_task_delta")
                .and_then(Value::as_array)
                .cloned()
                .ok_or(FinalFollowupError::StructuredOutputMissing)?;
            validate_patch_task_deltas(&deltas, &input.traceability_refs)?;
            apply_patch_task_deltas(&input.change_dir, &deltas)?;
            let manifest = build_openspec_source_manifest(&input.change_dir)?;
            let stale_status = check_bundle_stale(&input.current_bundle, &manifest);
            if !matches!(stale_status, BundleStatus::Stale) {
                return Err(FinalFollowupError::InvalidPatchTaskDelta(format!(
                    "expected stale bundle after tasks update, got {stale_status:?}"
                )));
            }
            let bundle = compile_constraint_bundle(
                &input.change_id,
                &manifest,
                input.projection_refs.clone(),
                "N26".to_string(),
            )?;
            let provider_run_id = records
                .last()
                .map(|record| record.provider_run_id.clone())
                .unwrap_or_else(|| "run_n26_0001".to_string());
            let dispatch_package =
                dispatch_package_from_deltas(&input, &deltas, &bundle, provider_run_id)?;
            let patch_round_counter = input.patch_round_counter + 1;
            protocol_steps.push(RuntimeProtocolStep {
                node_id: "N26".to_string(),
                status: RuntimeStepStatus::Completed,
                node_specific_fields: json!({
                    "patch_task_delta": deltas,
                    "new_dispatch_package_ref": dispatch_package["artifact_ref"],
                    "patch_round_counter": patch_round_counter,
                    "approved_by": approved_by,
                }),
            });
            protocol_steps.push(RuntimeProtocolStep {
                node_id: "N13".to_string(),
                status: RuntimeStepStatus::Completed,
                node_specific_fields: json!({
                    "worktask_id": dispatch_package["_aria"]["worktask_routing"][0]["worktask_id"],
                    "routing_ref": "dispatch_patch#0",
                    "state": "registered",
                }),
            });
            Ok(FinalFollowupResult {
                protocol_steps,
                final_review,
                final_summary: json!(null),
                new_dispatch_package: dispatch_package,
                recompiled_bundle: Some(bundle),
                patch_round_counter,
            })
        }
        ApprovalDecision::Rejected {
            rejected_by,
            reason,
        } => {
            let final_summary = json!({
                "artifact_kind": "final_summary",
                "overall_status": "closed_with_rejected_followup",
                "next_steps": [],
                "remaining_risks": [],
                "manual_exemptions": [{
                    "item_id": "followup",
                    "reason": reason,
                    "approved_by": rejected_by,
                }]
            });
            protocol_steps.push(RuntimeProtocolStep {
                node_id: "N27".to_string(),
                status: RuntimeStepStatus::Completed,
                node_specific_fields: json!({
                    "overall_status": "closed_with_rejected_followup",
                    "closed_items": [],
                    "remaining_risks": [],
                }),
            });
            protocol_steps.push(session_closeout_step(
                &input.task_id,
                json!({
                    "manual_exemptions": final_summary["manual_exemptions"],
                }),
            ));
            Ok(FinalFollowupResult {
                protocol_steps,
                final_review,
                final_summary,
                new_dispatch_package: json!(null),
                recompiled_bundle: None,
                patch_round_counter: input.patch_round_counter,
            })
        }
    }
}

fn closure_input(input: &FinalFollowupInput) -> FinalClosureInput {
    FinalClosureInput {
        session_id: input.session_id.clone(),
        task_id: input.task_id.clone(),
        worktree_path: input.worktree_path.clone(),
        projection_refs: input.projection_refs.clone(),
        constraint_bundle_ref: input.constraint_bundle_ref.clone(),
        risk_registry_ref: input.risk_registry_ref.clone(),
        canonical_artifact_refs: input.canonical_artifact_refs.clone(),
        traceability_refs: input.traceability_refs.clone(),
        context_files: input.context_files.clone(),
    }
}

fn apply_patch_task_deltas(
    change_dir: &std::path::Path,
    deltas: &[Value],
) -> Result<(), FinalFollowupError> {
    let tasks_path = change_dir.join("tasks.md");
    let mut model = read_document_model(&tasks_path)?;
    let existing_items = model
        .sections
        .iter()
        .find(|section| section.heading_path == vec!["Tasks".to_string()])
        .map(|section| {
            section
                .blocks
                .iter()
                .filter_map(|block| match block {
                    DocumentBlock::BulletList(items) => Some(items.clone()),
                    _ => None,
                })
                .flatten()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut items = existing_items;
    for delta in deltas {
        let task_id = delta
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or("TASK-NEW");
        let description = delta
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("Follow up task");
        let reqs = delta
            .get("related_requirement_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        let designs = delta
            .get("related_design_decision_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        let acceptance = delta
            .get("acceptance_targets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        items.push(format!(
            "[ ] {task_id} {description}. Reqs: {reqs}; Designs: {designs}; Acceptance: {acceptance}"
        ));
    }
    upsert_section(
        &mut model,
        &HeadingPath(vec!["Tasks".to_string()]),
        vec![DocumentBlock::BulletList(items)],
    )?;
    std::fs::write(&tasks_path, model.source_text).map_err(|error| {
        crate::cross_cutting::document_ops::DocumentOpError::IoError(format!(
            "write {}: {error}",
            tasks_path.display()
        ))
    })?;
    Ok(())
}

fn validate_patch_task_deltas(
    deltas: &[Value],
    known_traceability_refs: &[String],
) -> Result<(), FinalFollowupError> {
    for delta in deltas {
        for field in [
            "description",
            "acceptance_targets",
            "execution_mode",
            "traceability_refs",
        ] {
            if delta.get(field).is_none() {
                return Err(FinalFollowupError::InvalidPatchTaskDelta(format!(
                    "missing {field}"
                )));
            }
        }
        for traceability_ref in string_array_field(delta, "traceability_refs") {
            if !known_traceability_refs
                .iter()
                .any(|known| normalize_ref_id(known) == normalize_ref_id(&traceability_ref))
            {
                return Err(FinalFollowupError::InvalidPatchTaskDelta(format!(
                    "unknown traceability ref {traceability_ref}"
                )));
            }
        }
    }
    Ok(())
}

fn dispatch_package_from_deltas(
    input: &FinalFollowupInput,
    deltas: &[Value],
    bundle: &OpenSpecConstraintBundle,
    provider_run_id: String,
) -> Result<Value, FinalFollowupError> {
    let routing = deltas
        .iter()
        .enumerate()
        .map(|(index, delta)| {
            let task_id = delta.get("task_id").and_then(Value::as_str).unwrap_or("TASK-NEW");
            json!({
                "worktask_id": format!("patch_worktask_{:04}", index + 1),
                "source_task_id": task_id,
                "source_work_package_id": task_id,
                "execution_mode": delta.get("execution_mode").and_then(Value::as_str).unwrap_or("bounded_patch"),
                "allowed_write_scope": ["*"],
                "traceability_refs": delta.get("traceability_refs").cloned().unwrap_or_else(|| json!([])),
                "verification_commands": delta.get("acceptance_targets").cloned().unwrap_or_else(|| json!([])),
            })
        })
        .collect::<Vec<_>>();
    let traceability_refs = routing_traceability_refs(&routing);
    let dispatch_package = json!({
        "artifact_kind": "dispatch_package",
        "artifact_ref": format!("dispatch_pkg_patch_{}", bundle.task_constraints.task_ids.len()),
        "worktask_routing": routing,
        "_aria": {
            "profile_version": PHASE1_PROFILE_VERSION,
            "constraint_check_ref": bundle.constraint_bundle_id,
            "traceability_refs": traceability_refs,
            "provider_run_refs": [provider_run_id],
            "projection_refs": input.projection_refs,
            "worktask_routing": routing,
            "constraint_bundle_ref": bundle.constraint_bundle_id,
        }
    });
    phase1_profile_validator(
        &dispatch_package,
        ArtifactKind::DispatchPackage,
        &ProjectionIndex::with_work_packages(input.projection_refs.clone(), delta_task_ids(deltas)),
        &ConstraintBundleIndex {
            constraint_bundle_ids: vec![bundle.constraint_bundle_id.clone()],
            constraint_check_ids: Vec::new(),
        },
        &TraceabilityIndex::with_known_refs(input.traceability_refs.clone()),
        &ProviderRunIndex::with_runs(vec![
            dispatch_package["_aria"]["provider_run_refs"][0]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        ]),
    )
    .map_err(|error| FinalFollowupError::InvalidPatchTaskDelta(format!("{error:?}")))?;
    Ok(dispatch_package)
}

fn routing_traceability_refs(routing: &[Value]) -> Vec<String> {
    let mut refs = Vec::new();
    for item in routing {
        for value in item
            .get("traceability_refs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(ref_id) = value.as_str() {
                let ref_id = normalize_ref_id(ref_id);
                if !refs.contains(&ref_id) {
                    refs.push(ref_id);
                }
            }
        }
    }
    refs
}

fn delta_task_ids(deltas: &[Value]) -> Vec<String> {
    deltas
        .iter()
        .map(|delta| {
            delta
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or("TASK-NEW")
                .to_string()
        })
        .collect()
}

fn string_array_field(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn normalize_ref_id(value: &str) -> String {
    value
        .trim()
        .trim_matches(',')
        .trim_matches(';')
        .to_ascii_lowercase()
        .replace('_', "-")
}
