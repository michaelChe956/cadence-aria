use super::entry::{
    ProjectionEntry, entries_from_section_tree, find_section,
    structured_api_entries_from_section_tree, synthetic_entries_from_section_tree,
    synthetic_heading_entries_from_section_tree, synthetic_table_entries_from_section_tree,
};
use super::error::ProjectionCompileError;
use super::mappers::{
    api_entry_from_entry, component_from_entry, criterion_from_entry, data_entity_from_entry,
    dependency_from_entry, design_decision_from_entry, ensure_unique_work_packages,
    non_functional_requirement_from_entry, open_item_from_entry, parallelism_group_from_entry,
    requirement_from_entry, risk_from_entry, user_story_from_entry, work_package_from_entry,
};
use crate::cross_cutting::document_ops::extract_projection_source;
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef, ProjectionKind};
use crate::protocol::document_ops::{DocumentModel, DocumentSection, HeadingPath};
use crate::protocol::enums::NodeId;
use crate::protocol::projections::{
    ArtifactProjectionRecord, DesignProjection, PlanProjection, ProjectionPayload, SpecProjection,
};
use chrono::Utc;
use std::collections::HashSet;

pub fn compile_spec_projection(
    source: &DocumentModel,
    source_artifact_ref: &ArtifactRef,
    compiled_by_node: NodeId,
) -> Result<ArtifactProjectionRecord, ProjectionCompileError> {
    ensure_source_kind(source_artifact_ref, ArtifactKind::Spec)?;
    let requirements_section = required_section(
        source,
        &["功能需求", "Functional Requirements", "Requirements"],
    )?;
    let criteria_section = required_section(
        source,
        &["成功标准", "Success Criteria", "Acceptance Criteria"],
    )?;
    let mut requirement_entries = entries_from_section_tree(source, requirements_section);
    if requirement_entries.is_empty() {
        requirement_entries =
            synthetic_entries_from_section_tree(source, requirements_section, "req-");
    }
    let mut criterion_entries = entries_from_section_tree(source, criteria_section);
    if criterion_entries.is_empty() {
        criterion_entries = synthetic_entries_from_section_tree(source, criteria_section, "ac-");
    }

    let functional_requirements = requirement_entries
        .iter()
        .map(requirement_from_entry)
        .collect::<Result<Vec<_>, _>>()?;
    if functional_requirements.is_empty() {
        return Err(ProjectionCompileError::EmptyPayload {
            projection_kind: ProjectionKind::SpecProjection,
        });
    }
    let known_requirements: HashSet<String> = functional_requirements
        .iter()
        .map(|requirement| requirement.requirement_id.clone())
        .collect();

    let success_criteria = criterion_entries
        .iter()
        .map(|entry| criterion_from_entry(entry, &known_requirements))
        .collect::<Result<Vec<_>, _>>()?;
    if success_criteria.is_empty() {
        return Err(ProjectionCompileError::MissingRequiredSection {
            heading_path: HeadingPath(vec!["成功标准".to_string()]),
        });
    }

    let payload = SpecProjection {
        user_stories: optional_entries(source, &["用户故事", "User Stories"])
            .iter()
            .map(user_story_from_entry)
            .collect::<Result<Vec<_>, _>>()?,
        functional_requirements,
        success_criteria,
        open_items: optional_entries(source, &["待确认项", "Open Items"])
            .iter()
            .map(open_item_from_entry)
            .collect::<Result<Vec<_>, _>>()?,
        non_functional_requirements: optional_entries(
            source,
            &["非功能需求", "Non Functional Requirements"],
        )
        .iter()
        .map(non_functional_requirement_from_entry)
        .collect::<Result<Vec<_>, _>>()?,
    };

    Ok(record(
        ProjectionKind::SpecProjection,
        source,
        source_artifact_ref,
        compiled_by_node,
        ProjectionPayload::SpecProjection(payload),
    ))
}

pub fn compile_design_projection(
    source: &DocumentModel,
    source_artifact_ref: &ArtifactRef,
    compiled_by_node: NodeId,
) -> Result<ArtifactProjectionRecord, ProjectionCompileError> {
    ensure_source_kind(source_artifact_ref, ArtifactKind::Design)?;
    let decisions_section = required_section(source, &["设计决策", "Design Decisions"])?;
    let mut decision_entries = entries_from_section_tree(source, decisions_section);
    if decision_entries.is_empty() {
        decision_entries =
            synthetic_table_entries_from_section_tree(source, decisions_section, "dec-");
    }
    if decision_entries.is_empty() {
        decision_entries =
            synthetic_heading_entries_from_section_tree(source, decisions_section, "dec-");
    }
    if decision_entries.is_empty() {
        decision_entries = synthetic_entries_from_section_tree(source, decisions_section, "dec-");
    }
    let decisions = decision_entries
        .iter()
        .map(design_decision_from_entry)
        .collect::<Result<Vec<_>, _>>()?;
    if decisions.is_empty() {
        return Err(ProjectionCompileError::MissingRequiredSection {
            heading_path: HeadingPath(vec!["设计决策".to_string()]),
        });
    }
    let known_decisions: HashSet<String> = decisions
        .iter()
        .map(|decision| decision.design_decision_id.clone())
        .collect();

    let risk_refs = optional_entries_with_synthetic(source, &["风险", "Risks"], "risk-", true)
        .iter()
        .map(|entry| risk_from_entry(entry, &known_decisions))
        .collect::<Result<Vec<_>, _>>()?;

    let payload = DesignProjection {
        design_decisions: decisions,
        shared_components: optional_entries_with_table_synthetic(
            source,
            &["公共组件", "Shared Components", "shared_components"],
            "cmp-",
            true,
        )
        .iter()
        .map(component_from_entry)
        .collect::<Result<Vec<_>, _>>()?,
        shared_modules: optional_entries_with_table_synthetic(
            source,
            &["共享模块", "Shared Modules", "shared_modules"],
            "sm-",
            true,
        )
        .iter()
        .map(component_from_entry)
        .collect::<Result<Vec<_>, _>>()?,
        data_entities: optional_entries_with_synthetic(
            source,
            &["数据模型", "数据实体", "Data Entities", "data_entities"],
            "de-",
            true,
        )
        .iter()
        .map(data_entity_from_entry)
        .collect::<Result<Vec<_>, _>>()?,
        api_entries: api_entries_with_synthetic(
            source,
            &["API 契约", "API Contract", "api_entries"],
        )
        .iter()
        .map(api_entry_from_entry)
        .collect::<Result<Vec<_>, _>>()?,
        risk_refs,
        open_items: optional_entries_with_synthetic(
            source,
            &["待确认项", "Open Items"],
            "oq-",
            false,
        )
        .iter()
        .map(open_item_from_entry)
        .collect::<Result<Vec<_>, _>>()?,
    };

    Ok(record(
        ProjectionKind::DesignProjection,
        source,
        source_artifact_ref,
        compiled_by_node,
        ProjectionPayload::DesignProjection(payload),
    ))
}

pub fn compile_plan_projection(
    source: &DocumentModel,
    source_artifact_ref: &ArtifactRef,
    compiled_by_node: NodeId,
) -> Result<ArtifactProjectionRecord, ProjectionCompileError> {
    ensure_source_kind(source_artifact_ref, ArtifactKind::Plan)?;
    let work_section = required_section(source, &["工作包", "Work Packages", "任务拆解"])?;
    let work_packages = entries_from_section_tree(source, work_section)
        .iter()
        .map(work_package_from_entry)
        .collect::<Result<Vec<_>, _>>()?;
    if work_packages.is_empty() {
        return Err(ProjectionCompileError::EmptyPayload {
            projection_kind: ProjectionKind::PlanProjection,
        });
    }
    ensure_unique_work_packages(&work_packages)?;
    let known_work_packages: HashSet<String> = work_packages
        .iter()
        .map(|work_package| work_package.work_package_id.clone())
        .collect();

    let dependencies = optional_entries(source, &["依赖关系", "Dependencies"])
        .iter()
        .map(|entry| dependency_from_entry(entry, &known_work_packages))
        .collect::<Result<Vec<_>, _>>()?;
    let parallelism_groups = optional_entries(source, &["并行分组", "Parallelism Groups"])
        .iter()
        .map(parallelism_group_from_entry)
        .collect::<Result<Vec<_>, _>>()?;

    let payload = PlanProjection {
        work_packages,
        dependencies,
        parallelism_groups,
    };

    Ok(record(
        ProjectionKind::PlanProjection,
        source,
        source_artifact_ref,
        compiled_by_node,
        ProjectionPayload::PlanProjection(payload),
    ))
}

fn ensure_source_kind(
    source_artifact_ref: &ArtifactRef,
    expected: ArtifactKind,
) -> Result<(), ProjectionCompileError> {
    if source_artifact_ref.artifact_kind != expected {
        return Err(ProjectionCompileError::InvalidIdFormat {
            id: source_artifact_ref.artifact_kind.as_str().to_string(),
            expected_pattern: expected.as_str().to_string(),
        });
    }
    Ok(())
}

fn record(
    projection_kind: ProjectionKind,
    source: &DocumentModel,
    source_artifact_ref: &ArtifactRef,
    compiled_by_node: NodeId,
    payload: ProjectionPayload,
) -> ArtifactProjectionRecord {
    ArtifactProjectionRecord {
        projection_id: format!(
            "proj_{}_{}_0001",
            projection_kind.as_str(),
            source_artifact_ref.artifact_id
        ),
        projection_kind,
        source_artifact_ref: source_artifact_ref.clone(),
        source_artifact_version: source_artifact_ref.version,
        source_artifact_hash: source.sha256.clone(),
        compiled_at: Utc::now().to_rfc3339(),
        compiled_by_node,
        payload,
    }
}

fn required_section<'a>(
    source: &'a DocumentModel,
    aliases: &[&str],
) -> Result<&'a DocumentSection, ProjectionCompileError> {
    let section = find_section(source, aliases).ok_or_else(|| {
        ProjectionCompileError::MissingRequiredSection {
            heading_path: HeadingPath(vec![aliases[0].to_string()]),
        }
    })?;
    let _ = extract_projection_source(source, &HeadingPath(section.heading_path.clone()));
    Ok(section)
}

fn optional_entries(source: &DocumentModel, aliases: &[&str]) -> Vec<ProjectionEntry> {
    find_section(source, aliases)
        .map(|section| entries_from_section_tree(source, section))
        .unwrap_or_default()
}

fn optional_entries_with_synthetic(
    source: &DocumentModel,
    aliases: &[&str],
    id_prefix: &str,
    prefer_headings: bool,
) -> Vec<ProjectionEntry> {
    let Some(section) = find_section(source, aliases) else {
        return Vec::new();
    };
    let entries = entries_from_section_tree(source, section);
    if !entries.is_empty() {
        return entries;
    }
    let first_fallback = if prefer_headings {
        synthetic_heading_entries_from_section_tree(source, section, id_prefix)
    } else {
        synthetic_entries_from_section_tree(source, section, id_prefix)
    };
    if !first_fallback.is_empty() {
        return first_fallback;
    }
    if prefer_headings {
        synthetic_entries_from_section_tree(source, section, id_prefix)
    } else {
        synthetic_heading_entries_from_section_tree(source, section, id_prefix)
    }
}

fn optional_entries_with_table_synthetic(
    source: &DocumentModel,
    aliases: &[&str],
    id_prefix: &str,
    prefer_headings: bool,
) -> Vec<ProjectionEntry> {
    let Some(section) = find_section(source, aliases) else {
        return Vec::new();
    };
    let entries = entries_from_section_tree(source, section);
    if !entries.is_empty() {
        return entries;
    }
    let table_entries = synthetic_table_entries_from_section_tree(source, section, id_prefix);
    if !table_entries.is_empty() {
        return table_entries;
    }
    let first_fallback = if prefer_headings {
        synthetic_heading_entries_from_section_tree(source, section, id_prefix)
    } else {
        synthetic_entries_from_section_tree(source, section, id_prefix)
    };
    if !first_fallback.is_empty() {
        return first_fallback;
    }
    if prefer_headings {
        synthetic_entries_from_section_tree(source, section, id_prefix)
    } else {
        synthetic_heading_entries_from_section_tree(source, section, id_prefix)
    }
}

fn api_entries_with_synthetic(source: &DocumentModel, aliases: &[&str]) -> Vec<ProjectionEntry> {
    let Some(section) = find_section(source, aliases) else {
        return Vec::new();
    };
    let direct_api_entries = entries_from_section_tree(source, section)
        .into_iter()
        .filter(|entry| entry.id.starts_with("api-"))
        .collect::<Vec<_>>();
    if !direct_api_entries.is_empty() {
        return direct_api_entries;
    }
    let structured_entries = structured_api_entries_from_section_tree(source, section);
    if !structured_entries.is_empty() {
        return structured_entries;
    }
    let heading_entries = synthetic_heading_entries_from_section_tree(source, section, "api-");
    if !heading_entries.is_empty() {
        return heading_entries;
    }
    synthetic_entries_from_section_tree(source, section, "api-")
}
