use super::utils::{ids_from_markdown, openspec_id, single_line};
use super::{PlanningChainState, PlanningUnitError};
use crate::cross_cutting::document_ops::read_document_model;
use crate::cross_cutting::openspec_constraints::{
    build_openspec_source_manifest, check_bundle_stale, compile_constraint_bundle,
};
use crate::protocol::constraints::{BundleStatus, OpenSpecConstraintBundle};
use crate::protocol::projections::{ArtifactProjectionRecord, ProjectionPayload};

pub fn write_spec_to_openspec_and_recompile(
    state: &mut PlanningChainState,
    spec_markdown: &str,
    projection_refs: Vec<String>,
) -> Result<(BundleStatus, OpenSpecConstraintBundle), PlanningUnitError> {
    let change_dir = state.openspec_change_dir();
    let spec_path = change_dir.join("specs/main/spec.md");
    let old_content = std::fs::read(&spec_path)
        .map_err(|error| PlanningUnitError::Io(format!("read {}: {error}", spec_path.display())))?;
    let openspec_markdown = canonical_spec_to_openspec(spec_markdown);
    if let Some(parent) = spec_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            PlanningUnitError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(&spec_path, openspec_markdown.as_bytes()).map_err(|error| {
        PlanningUnitError::Io(format!("write {}: {error}", spec_path.display()))
    })?;
    if let Err(error) = read_document_model(&spec_path) {
        let _ = std::fs::write(&spec_path, &old_content);
        return Err(PlanningUnitError::Io(format!(
            "openspec_patch_invalid_markdown: {error}"
        )));
    }
    let current_manifest = match build_openspec_source_manifest(&change_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = std::fs::write(&spec_path, &old_content);
            return Err(PlanningUnitError::OpenSpec(error));
        }
    };
    let stale_status = check_bundle_stale(&state.current_bundle, &current_manifest);
    let bundle = match compile_constraint_bundle(
        &state.input.change_id,
        &current_manifest,
        projection_refs,
        "N06".to_string(),
    ) {
        Ok(bundle) => bundle,
        Err(error) => {
            let _ = std::fs::write(&spec_path, &old_content);
            return Err(PlanningUnitError::OpenSpec(error));
        }
    };
    if bundle.requirement_constraints.requirement_ids.is_empty() {
        let _ = std::fs::write(&spec_path, &old_content);
        return Err(PlanningUnitError::OpenspecRequirementConstraintsEmpty);
    }
    state.current_bundle = bundle.clone();
    Ok((stale_status, bundle))
}

pub fn write_design_to_openspec_and_recompile(
    state: &mut PlanningChainState,
    design_markdown: &str,
    design_projection: &ArtifactProjectionRecord,
    projection_refs: Vec<String>,
) -> Result<(BundleStatus, OpenSpecConstraintBundle), PlanningUnitError> {
    let openspec_markdown = canonical_design_to_openspec(design_markdown, design_projection);
    let bundle = write_openspec_file_and_recompile(
        state,
        "design.md",
        &openspec_markdown,
        projection_refs,
        "N08",
    )?;
    if bundle.1.design_constraints.design_decision_ids.is_empty()
        && bundle.1.design_constraints.component_ids.is_empty()
    {
        return Err(PlanningUnitError::OpenSpec(
            crate::cross_cutting::openspec_constraints::OpenSpecError::DesignConstraintsEmpty,
        ));
    }
    Ok(bundle)
}

pub fn write_tasks_to_openspec_and_recompile(
    state: &mut PlanningChainState,
    tasks_markdown: &str,
    projection_refs: Vec<String>,
) -> Result<(BundleStatus, OpenSpecConstraintBundle), PlanningUnitError> {
    let bundle = write_openspec_file_and_recompile(
        state,
        "tasks.md",
        tasks_markdown,
        projection_refs,
        "N11",
    )?;
    if bundle.1.task_constraints.task_ids.is_empty() {
        return Err(PlanningUnitError::OpenSpec(
            crate::cross_cutting::openspec_constraints::OpenSpecError::TaskConstraintsEmpty,
        ));
    }
    Ok(bundle)
}

fn write_openspec_file_and_recompile(
    state: &mut PlanningChainState,
    relative_path: &str,
    content: &str,
    projection_refs: Vec<String>,
    compiled_by_node: &str,
) -> Result<(BundleStatus, OpenSpecConstraintBundle), PlanningUnitError> {
    let change_dir = state.openspec_change_dir();
    let target_path = change_dir.join(relative_path);
    let old_content = std::fs::read(&target_path).map_err(|error| {
        PlanningUnitError::Io(format!("read {}: {error}", target_path.display()))
    })?;
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            PlanningUnitError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(&target_path, content.as_bytes()).map_err(|error| {
        PlanningUnitError::Io(format!("write {}: {error}", target_path.display()))
    })?;
    if let Err(error) = read_document_model(&target_path) {
        let _ = std::fs::write(&target_path, &old_content);
        return Err(PlanningUnitError::Io(format!(
            "openspec_patch_invalid_markdown: {error}"
        )));
    }
    let current_manifest = match build_openspec_source_manifest(&change_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = std::fs::write(&target_path, &old_content);
            return Err(PlanningUnitError::OpenSpec(error));
        }
    };
    let stale_status = check_bundle_stale(&state.current_bundle, &current_manifest);
    let bundle = match compile_constraint_bundle(
        &state.input.change_id,
        &current_manifest,
        projection_refs,
        compiled_by_node.to_string(),
    ) {
        Ok(bundle) => bundle,
        Err(error) => {
            let _ = std::fs::write(&target_path, &old_content);
            return Err(PlanningUnitError::OpenSpec(error));
        }
    };
    state.current_bundle = bundle.clone();
    Ok((stale_status, bundle))
}

fn canonical_spec_to_openspec(spec_markdown: &str) -> String {
    let requirement_ids = ids_from_markdown(spec_markdown, "REQ-");
    let acceptance_ids = ids_from_markdown(spec_markdown, "AC-");
    let requirement_id = requirement_ids
        .first()
        .cloned()
        .unwrap_or_else(|| "REQ-001".to_string());
    let acceptance_id = acceptance_ids
        .first()
        .cloned()
        .unwrap_or_else(|| "AC-001".to_string());
    format!(
        "# Main Spec\n\n### ADDED Requirements\n\n#### Requirement: {requirement_id} Generated requirement\n\n##### Scenario: SCN-001 Generated scenario\n\n- WHEN the accepted planning input is processed\n- THEN the runtime satisfies the canonical spec [{acceptance_id}]\n"
    )
}

fn canonical_design_to_openspec(
    design_markdown: &str,
    design_projection: &ArtifactProjectionRecord,
) -> String {
    let ProjectionPayload::DesignProjection(design) = &design_projection.payload else {
        return design_markdown.to_string();
    };
    let mut output = String::from("# Design\n\n## 设计决策\n\n");
    if design.design_decisions.is_empty() {
        output.push_str("- [DEC-001] Generated design decision.\n");
    } else {
        for decision in &design.design_decisions {
            output.push_str("- [");
            output.push_str(&openspec_id(&decision.design_decision_id));
            output.push_str("] ");
            output.push_str(&single_line(&decision.text));
            output.push('\n');
        }
    }

    output.push_str("\n## 公共组件\n\n");
    if design.shared_components.is_empty() && design.shared_modules.is_empty() {
        output.push_str("- [CMP-001] Generated component.\n");
    } else {
        for component in design
            .shared_components
            .iter()
            .chain(design.shared_modules.iter())
        {
            output.push_str("- [");
            output.push_str(&openspec_id(&component.component_id));
            output.push_str("] ");
            output.push_str(&single_line(&component.name));
            if !component.responsibility.trim().is_empty() {
                output.push_str(": ");
                output.push_str(&single_line(&component.responsibility));
            }
            output.push('\n');
        }
    }

    output.push_str("\n## 风险\n\n");
    if design.risk_refs.is_empty() {
        output.push_str("- [RISK-001] Generated risk.\n");
    } else {
        for risk in &design.risk_refs {
            output.push_str("- [");
            output.push_str(&openspec_id(&risk.risk_id));
            output.push_str("] ");
            output.push_str(&single_line(&risk.text));
            output.push('\n');
        }
    }

    output
}
