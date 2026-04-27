use crate::cross_cutting::document_ops::{compute_sha256, extract_projection_source};
use crate::protocol::artifacts::{ArtifactKind, ArtifactRef, ProjectionKind};
use crate::protocol::document_ops::{DocumentBlock, DocumentModel, DocumentSection, HeadingPath};
use crate::protocol::enums::NodeId;
use crate::protocol::projections::{
    ApiEntryProjection, ArtifactProjectionRecord, ComponentProjection, CriterionProjection,
    DataEntityProjection, DependencyType, DesignDecisionProjection, DesignProjection,
    ExecutionMode, OpenItemProjection, ParallelismGroupProjection, PlanProjection,
    ProjectionPayload, RequirementPriority, RequirementProjection, ResolutionMode, RiskProjection,
    RiskSeverity, SpecProjection, UserStoryProjection, WorkDependencyProjection,
    WorkPackageProjection,
};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::fmt;

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
    let requirement_entries = entries_from_section(requirements_section);
    let criterion_entries = entries_from_section(criteria_section);

    let functional_requirements = requirement_entries
        .iter()
        .map(|entry| requirement_from_entry(entry))
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
        .map(requirement_from_entry)
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
    let decisions = entries_from_section(decisions_section)
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

    let risk_refs = optional_entries(source, &["风险", "Risks"])
        .iter()
        .map(|entry| risk_from_entry(entry, &known_decisions))
        .collect::<Result<Vec<_>, _>>()?;

    let payload = DesignProjection {
        design_decisions: decisions,
        shared_components: optional_entries(source, &["公共组件", "Shared Components"])
            .iter()
            .map(component_from_entry)
            .collect::<Result<Vec<_>, _>>()?,
        shared_modules: optional_entries(source, &["共享模块", "Shared Modules"])
            .iter()
            .map(component_from_entry)
            .collect::<Result<Vec<_>, _>>()?,
        data_entities: optional_entries(source, &["数据模型", "Data Entities"])
            .iter()
            .map(data_entity_from_entry)
            .collect::<Result<Vec<_>, _>>()?,
        api_entries: optional_entries(source, &["API 契约", "API Contract"])
            .iter()
            .map(api_entry_from_entry)
            .collect::<Result<Vec<_>, _>>()?,
        risk_refs,
        open_items: optional_entries(source, &["待确认项", "Open Items"])
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
    let work_packages = entries_from_section(work_section)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionCompileError {
    MissingRequiredSection {
        heading_path: HeadingPath,
    },
    InvalidIdFormat {
        id: String,
        expected_pattern: String,
    },
    DuplicateId {
        id: String,
        section: String,
    },
    ReferenceUnknown {
        ref_id: String,
        context: String,
    },
    PriorityInvalid {
        value: String,
    },
    ExecutionModeInvalid {
        value: String,
    },
    DependencyEndpointMissing {
        from: String,
        to: String,
    },
    EmptyPayload {
        projection_kind: ProjectionKind,
    },
}

impl fmt::Display for ProjectionCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectionCompileError::MissingRequiredSection { heading_path } => {
                write!(
                    formatter,
                    "missing required projection section {}",
                    heading_path
                        .0
                        .last()
                        .map(String::as_str)
                        .unwrap_or("<unknown>")
                )
            }
            ProjectionCompileError::InvalidIdFormat {
                id,
                expected_pattern,
            } => write!(
                formatter,
                "invalid projection id {id}, expected {expected_pattern}"
            ),
            ProjectionCompileError::DuplicateId { id, section } => {
                write!(formatter, "duplicate projection id {id} in {section}")
            }
            ProjectionCompileError::ReferenceUnknown { ref_id, context } => {
                write!(
                    formatter,
                    "unknown projection reference {ref_id} in {context}"
                )
            }
            ProjectionCompileError::PriorityInvalid { value } => {
                write!(formatter, "invalid requirement priority {value}")
            }
            ProjectionCompileError::ExecutionModeInvalid { value } => {
                write!(formatter, "invalid execution mode {value}")
            }
            ProjectionCompileError::DependencyEndpointMissing { from, to } => {
                write!(formatter, "dependency endpoint missing: {from} -> {to}")
            }
            ProjectionCompileError::EmptyPayload { projection_kind } => {
                write!(formatter, "empty projection payload {projection_kind:?}")
            }
        }
    }
}

impl std::error::Error for ProjectionCompileError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectionEntry {
    id: String,
    text: String,
    fields: HashMap<String, String>,
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
        .map(entries_from_section)
        .unwrap_or_default()
}

fn find_section<'a>(source: &'a DocumentModel, aliases: &[&str]) -> Option<&'a DocumentSection> {
    source.sections.iter().find(|section| {
        section.heading_path.last().is_some_and(|heading| {
            aliases
                .iter()
                .any(|alias| heading.eq_ignore_ascii_case(alias) || heading == alias)
        })
    })
}

fn entries_from_section(section: &DocumentSection) -> Vec<ProjectionEntry> {
    let mut entries = Vec::new();
    for block in &section.blocks {
        if let DocumentBlock::Table { headers, rows } = block {
            for row in rows {
                if let Some(entry) = entry_from_table_row(headers, row) {
                    entries.push(entry);
                }
            }
        }
    }
    for block in &section.blocks {
        if let DocumentBlock::BulletList(items) = block {
            for item in items {
                if let Some(entry) = entry_from_bullet(item) {
                    entries.push(entry);
                }
            }
        }
    }
    entries
}

fn entry_from_table_row(headers: &[String], row: &[String]) -> Option<ProjectionEntry> {
    let mut fields = HashMap::new();
    for (header, value) in headers.iter().zip(row.iter()) {
        fields.insert(normalize_key(header), value.trim().to_string());
    }
    let id = fields
        .get("id")
        .or_else(|| fields.get("work_package_id"))
        .or_else(|| fields.get("group"))
        .or_else(|| fields.get("from"))?
        .to_string();
    let text = fields
        .get("text")
        .or_else(|| fields.get("description"))
        .or_else(|| fields.get("说明"))
        .cloned()
        .unwrap_or_default();
    Some(ProjectionEntry {
        id: normalize_id(&id),
        text,
        fields,
    })
}

fn entry_from_bullet(item: &str) -> Option<ProjectionEntry> {
    let start = item.find('[')?;
    let end = item[start + 1..].find(']')? + start + 1;
    let id = normalize_id(&item[start + 1..end]);
    let rest = item[end + 1..].trim();
    let metadata = extract_metadata(rest);
    let text_end = first_metadata_position(rest).unwrap_or(rest.len());
    let text = clean_text(&rest[..text_end]);
    Some(ProjectionEntry {
        id,
        text,
        fields: metadata,
    })
}

fn requirement_from_entry(
    entry: &ProjectionEntry,
) -> Result<RequirementProjection, ProjectionCompileError> {
    ensure_id_prefix(&entry.id, "req-")?;
    let priority = field(entry, &["priority"])
        .unwrap_or_else(|| "should".to_string())
        .parse::<RequirementPriority>()
        .map_err(|value| ProjectionCompileError::PriorityInvalid { value })?;
    Ok(RequirementProjection {
        requirement_id: entry.id.clone(),
        text: required_text(entry)?,
        priority,
    })
}

fn criterion_from_entry(
    entry: &ProjectionEntry,
    known_requirements: &HashSet<String>,
) -> Result<CriterionProjection, ProjectionCompileError> {
    ensure_id_prefix(&entry.id, "ac-")?;
    let related_requirement_ids = field_values(entry, &["refs", "requirements"]);
    for ref_id in &related_requirement_ids {
        if !known_requirements.contains(ref_id) {
            return Err(ProjectionCompileError::ReferenceUnknown {
                ref_id: ref_id.clone(),
                context: "success_criteria".to_string(),
            });
        }
    }
    Ok(CriterionProjection {
        criterion_id: entry.id.clone(),
        text: required_text(entry)?,
        related_requirement_ids,
    })
}

fn user_story_from_entry(
    entry: &ProjectionEntry,
) -> Result<UserStoryProjection, ProjectionCompileError> {
    Ok(UserStoryProjection {
        story_id: entry.id.clone(),
        title: required_text(entry)?,
        related_requirement_ids: field_values(entry, &["refs", "requirements"]),
    })
}

fn open_item_from_entry(
    entry: &ProjectionEntry,
) -> Result<OpenItemProjection, ProjectionCompileError> {
    Ok(OpenItemProjection {
        item_id: entry.id.clone(),
        text: required_text(entry)?,
        resolution_mode: ResolutionMode::Deferred,
    })
}

fn design_decision_from_entry(
    entry: &ProjectionEntry,
) -> Result<DesignDecisionProjection, ProjectionCompileError> {
    ensure_id_prefix(&entry.id, "dd-")?;
    Ok(DesignDecisionProjection {
        design_decision_id: entry.id.clone(),
        text: required_text(entry)?,
        related_requirement_ids: field_values(entry, &["refs", "reqs", "requirements"]),
    })
}

fn component_from_entry(
    entry: &ProjectionEntry,
) -> Result<ComponentProjection, ProjectionCompileError> {
    Ok(ComponentProjection {
        component_id: entry.id.clone(),
        name: field(entry, &["name"]).unwrap_or_else(|| entry.id.clone()),
        responsibility: field(entry, &["responsibility", "职责"]).unwrap_or_default(),
    })
}

fn data_entity_from_entry(
    entry: &ProjectionEntry,
) -> Result<DataEntityProjection, ProjectionCompileError> {
    Ok(DataEntityProjection {
        entity_id: entry.id.clone(),
        name: field(entry, &["name"]).unwrap_or_else(|| entry.id.clone()),
        fields: field_values(entry, &["fields", "字段"]),
    })
}

fn api_entry_from_entry(
    entry: &ProjectionEntry,
) -> Result<ApiEntryProjection, ProjectionCompileError> {
    Ok(ApiEntryProjection {
        api_id: entry.id.clone(),
        name: field(entry, &["name"]).unwrap_or_else(|| entry.id.clone()),
        input: field(entry, &["input", "输入"]).unwrap_or_default(),
        output: field(entry, &["output", "输出"]).unwrap_or_default(),
    })
}

fn risk_from_entry(
    entry: &ProjectionEntry,
    known_decisions: &HashSet<String>,
) -> Result<RiskProjection, ProjectionCompileError> {
    ensure_id_prefix(&entry.id, "risk-")?;
    let related_design_decision_ids = field_values(entry, &["refs", "designs"]);
    for ref_id in &related_design_decision_ids {
        if !known_decisions.contains(ref_id) {
            return Err(ProjectionCompileError::ReferenceUnknown {
                ref_id: ref_id.clone(),
                context: "risk_refs".to_string(),
            });
        }
    }
    let severity = field(entry, &["severity"])
        .unwrap_or_else(|| "medium".to_string())
        .parse::<RiskSeverity>()
        .map_err(|value| ProjectionCompileError::ReferenceUnknown {
            ref_id: value,
            context: "risk_severity".to_string(),
        })?;
    Ok(RiskProjection {
        risk_id: entry.id.clone(),
        text: required_text(entry)?,
        severity,
        mitigation: field(entry, &["mitigation"]),
        related_design_decision_ids,
    })
}

fn work_package_from_entry(
    entry: &ProjectionEntry,
) -> Result<WorkPackageProjection, ProjectionCompileError> {
    ensure_id_prefix(&entry.id, "wt-")?;
    let execution_mode_text = field(entry, &["execution_mode", "执行模式", "mode"])
        .unwrap_or_else(|| "agent_only".to_string());
    let execution_mode = execution_mode_text
        .parse::<ExecutionMode>()
        .map_err(|value| ProjectionCompileError::ExecutionModeInvalid { value })?;
    let human_required_reason = field(entry, &["human_reason", "人工原因"]);
    if matches!(execution_mode, ExecutionMode::HumanRequired)
        && human_required_reason.as_deref().unwrap_or("").is_empty()
    {
        return Err(ProjectionCompileError::ExecutionModeInvalid {
            value: "human_required_without_reason".to_string(),
        });
    }
    Ok(WorkPackageProjection {
        work_package_id: entry.id.clone(),
        description: required_text(entry)?,
        execution_mode,
        human_required_reason: human_required_reason.filter(|value| !value.is_empty()),
        traceability_refs: field_values(entry, &["traceability", "追踪引用"]),
        acceptance_targets: field_values(entry, &["acceptance", "验收目标"]),
    })
}

fn dependency_from_entry(
    entry: &ProjectionEntry,
    known_work_packages: &HashSet<String>,
) -> Result<WorkDependencyProjection, ProjectionCompileError> {
    let from = field(entry, &["from"]).unwrap_or_else(|| entry.id.clone());
    let to = field(entry, &["to"]).unwrap_or_default();
    let from = normalize_id(&from);
    let to = normalize_id(&to);
    if !known_work_packages.contains(&from) || !known_work_packages.contains(&to) {
        return Err(ProjectionCompileError::DependencyEndpointMissing { from, to });
    }
    let dependency_type = field(entry, &["type"])
        .unwrap_or_else(|| "depends_on".to_string())
        .parse::<DependencyType>()
        .map_err(|value| ProjectionCompileError::ReferenceUnknown {
            ref_id: value,
            context: "dependency_type".to_string(),
        })?;
    Ok(WorkDependencyProjection {
        from_work_package_id: from,
        to_work_package_id: to,
        dependency_type,
    })
}

fn parallelism_group_from_entry(
    entry: &ProjectionEntry,
) -> Result<ParallelismGroupProjection, ProjectionCompileError> {
    let max_parallel = field(entry, &["max_parallel"])
        .unwrap_or_else(|| "1".to_string())
        .parse::<u32>()
        .unwrap_or(1);
    Ok(ParallelismGroupProjection {
        group_id: entry.id.clone(),
        work_package_ids: field_values(entry, &["work_packages"]),
        max_parallel,
    })
}

fn ensure_unique_work_packages(
    work_packages: &[WorkPackageProjection],
) -> Result<(), ProjectionCompileError> {
    let mut seen = HashSet::new();
    for work_package in work_packages {
        if !seen.insert(work_package.work_package_id.clone()) {
            return Err(ProjectionCompileError::DuplicateId {
                id: work_package.work_package_id.clone(),
                section: "work_packages".to_string(),
            });
        }
        if work_package.traceability_refs.is_empty() {
            return Err(ProjectionCompileError::ReferenceUnknown {
                ref_id: work_package.work_package_id.clone(),
                context: "traceability_refs".to_string(),
            });
        }
    }
    Ok(())
}

fn ensure_id_prefix(id: &str, prefix: &str) -> Result<(), ProjectionCompileError> {
    if id.starts_with(prefix) {
        Ok(())
    } else {
        Err(ProjectionCompileError::InvalidIdFormat {
            id: id.to_string(),
            expected_pattern: format!("{prefix}<number>"),
        })
    }
}

fn required_text(entry: &ProjectionEntry) -> Result<String, ProjectionCompileError> {
    if !entry.text.trim().is_empty() {
        Ok(entry.text.trim().to_string())
    } else if let Some(value) = field(entry, &["text", "description", "说明"]) {
        Ok(value)
    } else {
        Err(ProjectionCompileError::InvalidIdFormat {
            id: entry.id.clone(),
            expected_pattern: "non-empty text".to_string(),
        })
    }
}

fn field(entry: &ProjectionEntry, aliases: &[&str]) -> Option<String> {
    aliases
        .iter()
        .find_map(|alias| entry.fields.get(&normalize_key(alias)).cloned())
        .map(|value| value.trim().trim_matches(';').trim().to_string())
}

fn field_values(entry: &ProjectionEntry, aliases: &[&str]) -> Vec<String> {
    field(entry, aliases)
        .map(|value| split_values(&value))
        .unwrap_or_default()
}

fn split_values(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(normalize_id)
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalize_id(value: &str) -> String {
    value
        .trim()
        .trim_matches(';')
        .trim_matches(',')
        .to_ascii_lowercase()
        .replace('_', "-")
}

fn normalize_key(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_")
}

fn clean_text(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(';')
        .trim_end_matches(',')
        .trim()
        .to_string()
}

fn extract_metadata(rest: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for marker in METADATA_MARKERS {
        if let Some(value) = metadata_value(rest, marker) {
            fields.insert(normalize_key(marker), value);
        }
    }
    fields
}

fn metadata_value(rest: &str, marker: &str) -> Option<String> {
    let start = rest.find(marker)? + marker.len();
    let tail = &rest[start..];
    let end = METADATA_MARKERS
        .iter()
        .filter(|candidate| **candidate != marker)
        .filter_map(|candidate| tail.find(candidate))
        .min()
        .unwrap_or(tail.len());
    Some(clean_text(&tail[..end]))
}

fn first_metadata_position(rest: &str) -> Option<usize> {
    METADATA_MARKERS
        .iter()
        .filter_map(|marker| rest.find(marker))
        .min()
}

const METADATA_MARKERS: &[&str] = &[
    "Priority:",
    "Refs:",
    "Reqs:",
    "Designs:",
    "Acceptance:",
    "Risks:",
    "Mode:",
    "Traceability:",
    "Severity:",
    "Mitigation:",
    "Human Reason:",
];

#[allow(dead_code)]
fn source_hash_from_text(text: &str) -> String {
    compute_sha256(text.as_bytes())
}
