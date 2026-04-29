use std::path::{Path, PathBuf};

use crate::cross_cutting::openspec_constraints::{
    DefaultDocumentOps, bootstrap_openspec_skeleton, build_openspec_source_manifest,
};
use crate::protocol::constraints::{
    BundleStatus, CoverageModel, DesignConstraints, OpenSpecConstraintBundle, ProposalConstraints,
    RequirementConstraints, TaskConstraints, TraceabilityRequirements,
};
use crate::task_run::types::TaskRunError;

pub fn bootstrap_task_openspec(
    workspace_root: &Path,
    change_id: &str,
    request_text: &str,
    task_state_path: &Path,
) -> Result<PathBuf, TaskRunError> {
    validate_change_id(change_id)?;
    let _ =
        bootstrap_openspec_skeleton(&change_id.to_string(), task_state_path, &DefaultDocumentOps)
            .map_err(|error| TaskRunError::new("openspec_bootstrap_failed", error.to_string()))?;
    let change_dir = workspace_root.join("openspec/changes").join(change_id);
    seed_proposal(&change_dir, request_text)?;
    Ok(change_dir)
}

pub fn build_initial_constraint_bundle(
    change_id: &str,
    change_dir: &Path,
    request_text: &str,
) -> Result<OpenSpecConstraintBundle, TaskRunError> {
    validate_change_id(change_id)?;
    let source_manifest = build_openspec_source_manifest(change_dir)
        .map_err(|error| TaskRunError::new("openspec_manifest_failed", error.to_string()))?;
    Ok(OpenSpecConstraintBundle {
        constraint_bundle_id: format!("constraint_bundle_openspec_{change_id}_initial"),
        bundle_version: "openspec.constraint_bundle.v1".to_string(),
        bundle_status: BundleStatus::Ready,
        change_id: change_id.to_string(),
        proposal_constraints: ProposalConstraints {
            business_intent: vec![request_text.to_string()],
            scope: vec![
                "Aria must drive the requested implementation through provider workflow."
                    .to_string(),
            ],
            non_goals: vec![
                "Do not manually implement target project code outside Aria workflow.".to_string(),
            ],
            impacted_areas: vec![
                "target workspace".to_string(),
                "OpenSpec change".to_string(),
            ],
        },
        requirement_constraints: RequirementConstraints {
            requirement_ids: Vec::new(),
            scenario_ids: Vec::new(),
            success_criteria_ids: Vec::new(),
        },
        design_constraints: DesignConstraints {
            design_decision_ids: Vec::new(),
            component_ids: Vec::new(),
            risk_ids: Vec::new(),
        },
        task_constraints: TaskConstraints {
            task_ids: Vec::new(),
            task_sequence: Vec::new(),
            related_requirement_ids_by_task: Default::default(),
            related_design_decision_ids_by_task: Default::default(),
            acceptance_target_ids_by_task: Default::default(),
        },
        traceability_requirements: TraceabilityRequirements {
            required_requirement_ids: Vec::new(),
            required_design_decision_ids: Vec::new(),
            required_task_ids: Vec::new(),
            required_acceptance_target_ids: Vec::new(),
        },
        coverage_model: CoverageModel {
            required_ids: Vec::new(),
            covered_ids: Vec::new(),
            uncovered_ids: Vec::new(),
        },
        source_manifest,
        compiled_from_projection_refs: Vec::new(),
        compiled_at: chrono::Utc::now().to_rfc3339(),
        compiled_by_node: "N03".to_string(),
    })
}

fn validate_change_id(change_id: &str) -> Result<(), TaskRunError> {
    if change_id.is_empty()
        || !change_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err(TaskRunError::new(
            "invalid_change_id",
            format!("OpenSpec change id is invalid: {change_id}"),
        ));
    }
    Ok(())
}

fn seed_proposal(change_dir: &Path, request_text: &str) -> Result<(), TaskRunError> {
    let proposal = format!(
        "# Change Proposal\n\n## Why\n\n- {request_text}\n\n## What Changes\n\n- Aria will create or update the target implementation through Claude Code and Codex provider workflow.\n\n## Non-Goals\n\n- Do not manually implement target project code outside Aria workflow.\n\n## Impact\n\n- Target workspace code may change.\n- OpenSpec change artifacts will be updated.\n"
    );
    let path = change_dir.join("proposal.md");
    std::fs::write(&path, proposal).map_err(|error| {
        TaskRunError::new(
            "openspec_bootstrap_failed",
            format!("write {}: {error}", path.display()),
        )
    })
}
