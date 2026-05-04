use crate::cross_cutting::document_ops::{
    DocumentOpError, DocumentTemplateKind, compute_sha256, create_document, read_document_model,
};
use crate::protocol::constraints::{
    BundleStatus, CoverageModel, DesignConstraints, OpenSpecBootstrapStatus,
    OpenSpecConstraintBundle, OpenSpecSourceFile, OpenSpecSourceKind, ProposalConstraints,
    RequirementConstraints, TaskConstraints, TraceabilityRequirements,
};
use crate::protocol::document_ops::{DocumentBlock, DocumentModel};
use crate::protocol::enums::{ChangeId, NodeId, ProjectionId};
use chrono::Utc;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub trait DocumentOps {
    fn create_document(
        &self,
        path: &Path,
        template_kind: DocumentTemplateKind,
    ) -> Result<DocumentModel, DocumentOpError>;
}

#[derive(Debug, Clone, Copy)]
pub struct DefaultDocumentOps;

impl DocumentOps for DefaultDocumentOps {
    fn create_document(
        &self,
        path: &Path,
        template_kind: DocumentTemplateKind,
    ) -> Result<DocumentModel, DocumentOpError> {
        create_document(path, template_kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenSpecError {
    #[error("OpenSpec bootstrap is already complete")]
    BootstrapAlreadyComplete,
    #[error("task runtime state error: {0}")]
    TaskState(String),
    #[error("document operation error: {0}")]
    DocumentOperation(#[from] DocumentOpError),
    #[error("OpenSpec source missing: {kind:?} blocks {blocked_node}")]
    SourceMissing {
        kind: OpenSpecSourceKind,
        blocked_node: NodeId,
    },
    #[error("proposal constraints are empty")]
    ProposalConstraintsEmpty,
    #[error("requirement constraints are empty")]
    RequirementConstraintsEmpty,
    #[error("design constraints are empty")]
    DesignConstraintsEmpty,
    #[error("task constraints are empty")]
    TaskConstraintsEmpty,
}

pub fn bootstrap_openspec_skeleton(
    change_id: &ChangeId,
    task_runtime_state_path: &Path,
    document_ops: &dyn DocumentOps,
) -> Result<OpenSpecBootstrapStatus, OpenSpecError> {
    let mut task_state = read_task_state(task_runtime_state_path)?;
    if task_state
        .get("openspec_bootstrap_status")
        .and_then(Value::as_str)
        == Some("bootstrapped")
    {
        return Err(OpenSpecError::BootstrapAlreadyComplete);
    }

    let workspace_root = workspace_root_for_task_state(task_runtime_state_path)?;
    let change_dir = workspace_root.join("openspec/changes").join(change_id);
    for (path, template_kind) in [
        (
            change_dir.join("proposal.md"),
            DocumentTemplateKind::OpenspecProposal,
        ),
        (
            change_dir.join("specs/main/spec.md"),
            DocumentTemplateKind::OpenspecSpec,
        ),
        (
            change_dir.join("design.md"),
            DocumentTemplateKind::OpenspecDesign,
        ),
        (
            change_dir.join("tasks.md"),
            DocumentTemplateKind::OpenspecTasks,
        ),
    ] {
        document_ops.create_document(&path, template_kind)?;
    }

    let object = task_state
        .as_object_mut()
        .ok_or_else(|| OpenSpecError::TaskState("task state must be a JSON object".to_string()))?;
    object.insert(
        "openspec_bootstrap_status".to_string(),
        Value::String("bootstrapped".to_string()),
    );
    let next_state = serde_json::to_vec_pretty(&task_state)
        .map_err(|error| OpenSpecError::TaskState(format!("serialize task state: {error}")))?;
    fs::write(task_runtime_state_path, next_state).map_err(|error| {
        OpenSpecError::TaskState(format!(
            "write {}: {error}",
            task_runtime_state_path.display()
        ))
    })?;

    Ok(OpenSpecBootstrapStatus::Bootstrapped)
}

pub fn build_openspec_source_manifest(
    change_dir: &Path,
) -> Result<Vec<OpenSpecSourceFile>, OpenSpecError> {
    [
        (OpenSpecSourceKind::Proposal, "proposal.md"),
        (OpenSpecSourceKind::Spec, "specs/main/spec.md"),
        (OpenSpecSourceKind::Design, "design.md"),
        (OpenSpecSourceKind::Tasks, "tasks.md"),
    ]
    .into_iter()
    .filter_map(|(kind, relative_path)| {
        let path = change_dir.join(relative_path);
        match fs::read(&path) {
            Ok(content) => Some(Ok(OpenSpecSourceFile {
                path: path.to_string_lossy().to_string(),
                kind,
                sha256: compute_sha256(&content),
            })),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => Some(Err(OpenSpecError::TaskState(format!(
                "read {}: {error}",
                path.display()
            )))),
        }
    })
    .collect()
}

pub fn compile_constraint_bundle(
    change_id: &ChangeId,
    source_manifest: &[OpenSpecSourceFile],
    compiled_from_projection_refs: Vec<ProjectionId>,
    compiled_by_node: NodeId,
) -> Result<OpenSpecConstraintBundle, OpenSpecError> {
    let proposal_model = required_source_model(
        source_manifest,
        OpenSpecSourceKind::Proposal,
        node_for_missing(OpenSpecSourceKind::Proposal, &compiled_by_node),
    )?;
    let spec_model = required_source_model(
        source_manifest,
        OpenSpecSourceKind::Spec,
        node_for_missing(OpenSpecSourceKind::Spec, &compiled_by_node),
    )?;
    let design_model = required_source_model(
        source_manifest,
        OpenSpecSourceKind::Design,
        node_for_missing(OpenSpecSourceKind::Design, &compiled_by_node),
    )?;
    let tasks_model = required_source_model(
        source_manifest,
        OpenSpecSourceKind::Tasks,
        node_for_missing(OpenSpecSourceKind::Tasks, &compiled_by_node),
    )?;

    let proposal_constraints = compile_proposal_constraints(&proposal_model)?;
    let requirement_constraints = compile_requirement_constraints(&spec_model)?;
    let design_constraints = compile_design_constraints(&design_model)?;
    let task_constraints = compile_task_constraints(&tasks_model)?;
    let traceability_requirements = traceability_requirements(
        &requirement_constraints,
        &design_constraints,
        &task_constraints,
    );
    let coverage_model = coverage_model(&traceability_requirements, &task_constraints);

    Ok(OpenSpecConstraintBundle {
        constraint_bundle_id: format!("constraint_bundle_openspec_{change_id}_0001"),
        bundle_version: "openspec.constraint_bundle.v1".to_string(),
        bundle_status: BundleStatus::Ready,
        change_id: change_id.clone(),
        proposal_constraints,
        requirement_constraints,
        design_constraints,
        task_constraints,
        traceability_requirements,
        coverage_model,
        source_manifest: source_manifest.to_vec(),
        compiled_from_projection_refs,
        compiled_at: Utc::now().to_rfc3339(),
        compiled_by_node,
    })
}

pub fn check_bundle_stale(
    bundle: &OpenSpecConstraintBundle,
    current_manifest: &[OpenSpecSourceFile],
) -> BundleStatus {
    let current_by_path: BTreeMap<&str, &OpenSpecSourceFile> = current_manifest
        .iter()
        .map(|source| (source.path.as_str(), source))
        .collect();

    for source in &bundle.source_manifest {
        let Some(current) = current_by_path.get(source.path.as_str()) else {
            return BundleStatus::Blocked;
        };
        if current.kind != source.kind || current.sha256 != source.sha256 {
            return BundleStatus::Stale;
        }
    }

    if current_manifest.len() != bundle.source_manifest.len() {
        return BundleStatus::Stale;
    }

    bundle.bundle_status
}

fn read_task_state(path: &Path) -> Result<Value, OpenSpecError> {
    let content = fs::read(path)
        .map_err(|error| OpenSpecError::TaskState(format!("read {}: {error}", path.display())))?;
    serde_json::from_slice(&content)
        .map_err(|error| OpenSpecError::TaskState(format!("parse {}: {error}", path.display())))
}

fn workspace_root_for_task_state(path: &Path) -> Result<PathBuf, OpenSpecError> {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == ".aria"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            OpenSpecError::TaskState(format!(
                "cannot locate workspace root from {}",
                path.display()
            ))
        })
}

fn required_source_model(
    source_manifest: &[OpenSpecSourceFile],
    kind: OpenSpecSourceKind,
    blocked_node: NodeId,
) -> Result<DocumentModel, OpenSpecError> {
    let source = source_manifest
        .iter()
        .find(|source| source.kind == kind)
        .ok_or_else(|| OpenSpecError::SourceMissing {
            kind,
            blocked_node: blocked_node.clone(),
        })?;
    let path = Path::new(&source.path);
    if !path.exists() {
        return Err(OpenSpecError::SourceMissing { kind, blocked_node });
    }
    read_document_model(path).map_err(OpenSpecError::DocumentOperation)
}

fn node_for_missing(kind: OpenSpecSourceKind, requested_node: &NodeId) -> NodeId {
    match kind {
        OpenSpecSourceKind::Proposal => "N05".to_string(),
        OpenSpecSourceKind::Spec => "N07".to_string(),
        OpenSpecSourceKind::Design => "N11".to_string(),
        OpenSpecSourceKind::Tasks => {
            if requested_node == "N16" {
                "N16".to_string()
            } else {
                "N12".to_string()
            }
        }
    }
}

fn compile_proposal_constraints(
    model: &DocumentModel,
) -> Result<ProposalConstraints, OpenSpecError> {
    let constraints = ProposalConstraints {
        business_intent: section_items(model, &["Why", "背景"]),
        scope: section_items(model, &["What Changes", "范围"]),
        non_goals: section_items(model, &["Non-Goals", "不做"]),
        impacted_areas: section_items(model, &["Impact", "影响"]),
    };
    if constraints.business_intent.is_empty() || constraints.scope.is_empty() {
        return Err(OpenSpecError::ProposalConstraintsEmpty);
    }
    Ok(constraints)
}

fn compile_requirement_constraints(
    model: &DocumentModel,
) -> Result<RequirementConstraints, OpenSpecError> {
    let mut requirement_ids = Vec::new();
    let mut scenario_ids = Vec::new();
    let mut success_criteria_ids = Vec::new();

    for line in model.source_text.lines() {
        if let Some(id) = id_after_marker(line, "Requirement:", "REQ-") {
            requirement_ids.push(id);
        }
        if let Some(id) = id_after_marker(line, "Scenario:", "SCN-") {
            scenario_ids.push(id);
        }
        success_criteria_ids.extend(ids_with_prefix(line, &["AC-"]));
    }

    let requirement_ids = dedupe_preserve_order(requirement_ids);
    if requirement_ids.is_empty() {
        return Err(OpenSpecError::RequirementConstraintsEmpty);
    }

    Ok(RequirementConstraints {
        requirement_ids,
        scenario_ids: dedupe_preserve_order(scenario_ids),
        success_criteria_ids: dedupe_preserve_order(success_criteria_ids),
    })
}

fn compile_design_constraints(model: &DocumentModel) -> Result<DesignConstraints, OpenSpecError> {
    let design_decision_ids = ids_from_sections(
        model,
        &["Decisions", "Design Decisions", "设计决策"],
        &["DD-", "DEC-"],
    );
    let component_ids = ids_from_sections(
        model,
        &["Components", "Shared Components", "组件", "公共组件"],
        &["CMP-", "COMP-"],
    );
    let risk_ids = ids_from_sections(model, &["Risks", "风险"], &["RISK-"]);

    if design_decision_ids.is_empty() && component_ids.is_empty() {
        return Err(OpenSpecError::DesignConstraintsEmpty);
    }

    Ok(DesignConstraints {
        design_decision_ids,
        component_ids,
        risk_ids,
    })
}

fn compile_task_constraints(model: &DocumentModel) -> Result<TaskConstraints, OpenSpecError> {
    let mut task_ids = Vec::new();
    let mut related_requirement_ids_by_task = BTreeMap::new();
    let mut related_design_decision_ids_by_task = BTreeMap::new();
    let mut acceptance_target_ids_by_task = BTreeMap::new();

    for line in model.source_text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed
            .strip_prefix("- [ ] ")
            .or_else(|| trimmed.strip_prefix("- [x] "))
            .or_else(|| trimmed.strip_prefix("- [X] "))
        else {
            continue;
        };
        let Some(task_id) = ids_with_prefix(rest, &["TASK-"]).into_iter().next() else {
            continue;
        };
        task_ids.push(task_id.clone());
        related_requirement_ids_by_task
            .insert(task_id.clone(), metadata_ids(rest, "Reqs:", &["REQ-"]));
        related_design_decision_ids_by_task.insert(
            task_id.clone(),
            metadata_ids(rest, "Designs:", &["DD-", "DEC-"]),
        );
        acceptance_target_ids_by_task.insert(task_id, metadata_ids(rest, "Acceptance:", &["AC-"]));
    }

    let task_ids = dedupe_preserve_order(task_ids);
    if task_ids.is_empty() {
        return Err(OpenSpecError::TaskConstraintsEmpty);
    }

    Ok(TaskConstraints {
        task_sequence: task_ids.clone(),
        task_ids,
        related_requirement_ids_by_task,
        related_design_decision_ids_by_task,
        acceptance_target_ids_by_task,
    })
}

fn section_items(model: &DocumentModel, aliases: &[&str]) -> Vec<String> {
    model
        .sections
        .iter()
        .filter(|section| {
            section
                .heading_path
                .last()
                .is_some_and(|title| aliases.iter().any(|alias| title == alias))
        })
        .flat_map(|section| block_items(&section.blocks))
        .collect()
}

fn ids_from_sections(model: &DocumentModel, aliases: &[&str], prefixes: &[&str]) -> Vec<String> {
    let root_paths = model
        .sections
        .iter()
        .filter(|section| {
            section.heading_path.last().is_some_and(|title| {
                aliases
                    .iter()
                    .any(|alias| heading_matches_alias(title, alias))
            })
        })
        .map(|section| section.heading_path.clone())
        .collect::<Vec<_>>();

    let ids = model
        .sections
        .iter()
        .filter(|section| {
            root_paths
                .iter()
                .any(|root_path| section.heading_path.starts_with(root_path))
        })
        .flat_map(|section| {
            let mut items = block_items(&section.blocks);
            if let Some(title) = section.heading_path.last() {
                items.push(title.to_string());
            }
            items
        })
        .flat_map(|item| ids_with_prefix(&item, prefixes))
        .collect();
    dedupe_preserve_order(ids)
}

fn heading_matches_alias(heading: &str, alias: &str) -> bool {
    let normalized = normalized_heading(heading);
    if normalized.eq_ignore_ascii_case(alias) || normalized == alias {
        return true;
    }
    normalized.strip_prefix(alias).is_some_and(|suffix| {
        let suffix = suffix.trim_start();
        suffix.starts_with('(') || suffix.starts_with('（')
    })
}

fn normalized_heading(heading: &str) -> &str {
    let trimmed = heading.trim();
    let mut prefix_end = 0;
    for (index, character) in trimmed.char_indices() {
        if character.is_ascii_digit()
            || character.is_whitespace()
            || matches!(character, '.' | '。' | ')' | '、')
        {
            prefix_end = index + character.len_utf8();
            continue;
        }
        break;
    }
    let normalized = trimmed[prefix_end..].trim_start();
    if normalized.is_empty() {
        trimmed
    } else {
        normalized
    }
}

fn block_items(blocks: &[DocumentBlock]) -> Vec<String> {
    blocks
        .iter()
        .flat_map(|block| match block {
            DocumentBlock::Paragraph(text) => text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>(),
            DocumentBlock::BulletList(items) | DocumentBlock::OrderedList(items) => items
                .iter()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>(),
            DocumentBlock::Table { rows, .. } => rows
                .iter()
                .flat_map(|row| row.iter())
                .map(|cell| cell.trim().to_string())
                .filter(|cell| !cell.is_empty())
                .collect::<Vec<_>>(),
            DocumentBlock::CodeBlock { text, .. } => text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>(),
        })
        .collect()
}

fn id_after_marker(line: &str, marker: &str, prefix: &str) -> Option<String> {
    line.split_once(marker)
        .and_then(|(_, rest)| ids_with_prefix(rest, &[prefix]).into_iter().next())
}

fn metadata_ids(line: &str, marker: &str, prefixes: &[&str]) -> Vec<String> {
    let Some((_, rest)) = line.split_once(marker) else {
        return Vec::new();
    };
    let segment = rest.split(';').next().unwrap_or(rest);
    dedupe_preserve_order(ids_with_prefix(segment, prefixes))
}

fn ids_with_prefix(text: &str, prefixes: &[&str]) -> Vec<String> {
    let normalized = text
        .replace(['[', ']', '(', ')', ',', ';', ':', '.', '`'], " ")
        .replace('\t', " ");
    normalized
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '-'
            })
        })
        .filter(|token| prefixes.iter().any(|prefix| token.starts_with(prefix)))
        .filter(|token| token.contains('-'))
        .map(ToOwned::to_owned)
        .collect()
}

fn traceability_requirements(
    requirements: &RequirementConstraints,
    design: &DesignConstraints,
    tasks: &TaskConstraints,
) -> TraceabilityRequirements {
    TraceabilityRequirements {
        required_requirement_ids: requirements.requirement_ids.clone(),
        required_design_decision_ids: design.design_decision_ids.clone(),
        required_task_ids: tasks.task_ids.clone(),
        required_acceptance_target_ids: requirements.success_criteria_ids.clone(),
    }
}

fn coverage_model(
    traceability: &TraceabilityRequirements,
    tasks: &TaskConstraints,
) -> CoverageModel {
    let required = BTreeSet::from_iter(
        traceability
            .required_acceptance_target_ids
            .iter()
            .chain(traceability.required_design_decision_ids.iter())
            .chain(traceability.required_requirement_ids.iter())
            .chain(traceability.required_task_ids.iter())
            .cloned(),
    );
    let covered = BTreeSet::from_iter(
        tasks
            .acceptance_target_ids_by_task
            .values()
            .flatten()
            .chain(tasks.related_design_decision_ids_by_task.values().flatten())
            .chain(tasks.related_requirement_ids_by_task.values().flatten())
            .chain(tasks.task_ids.iter())
            .cloned(),
    );
    let uncovered = required.difference(&covered).cloned().collect();

    CoverageModel {
        required_ids: required.into_iter().collect(),
        covered_ids: covered.into_iter().collect(),
        uncovered_ids: uncovered,
    }
}

fn dedupe_preserve_order(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}
