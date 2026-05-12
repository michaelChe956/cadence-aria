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

fn find_section<'a>(source: &'a DocumentModel, aliases: &[&str]) -> Option<&'a DocumentSection> {
    source.sections.iter().find(|section| {
        section.heading_path.last().is_some_and(|heading| {
            aliases
                .iter()
                .any(|alias| heading_matches_alias(heading, alias))
        })
    })
}

fn heading_matches_alias(heading: &str, alias: &str) -> bool {
    let normalized = normalized_heading(heading);
    if heading_text_matches_alias(normalized, alias) {
        return true;
    }
    leading_identifier_tail(normalized).is_some_and(|tail| heading_text_matches_alias(tail, alias))
}

fn heading_text_matches_alias(text: &str, alias: &str) -> bool {
    if text.eq_ignore_ascii_case(alias) || text == alias {
        return true;
    }
    text.strip_prefix(alias).is_some_and(|suffix| {
        let suffix = suffix.trim_start();
        suffix.starts_with('(') || suffix.starts_with('（')
    })
}

fn leading_identifier_tail(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let separator_index = trimmed
        .char_indices()
        .find_map(|(index, character)| character.is_whitespace().then_some(index))?;
    let identifier = &trimmed[..separator_index];
    let has_id_shape = identifier.contains('-')
        && identifier
            .chars()
            .any(|character| character.is_ascii_digit())
        && identifier
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.'));
    has_id_shape.then(|| trimmed[separator_index..].trim_start())
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

fn entries_from_section_tree(
    source: &DocumentModel,
    root_section: &DocumentSection,
) -> Vec<ProjectionEntry> {
    source
        .sections
        .iter()
        .filter(|section| section.heading_path.starts_with(&root_section.heading_path))
        .flat_map(|section| {
            if section.heading_path == root_section.heading_path {
                direct_entries_from_section(section)
            } else {
                entries_from_section(section)
            }
        })
        .collect()
}

fn entries_from_section(section: &DocumentSection) -> Vec<ProjectionEntry> {
    let mut entries = direct_entries_from_section(section);
    if entries.is_empty()
        && let Some(entry) = entry_from_section_heading(section)
    {
        entries.push(entry);
    }
    entries
}

fn direct_entries_from_section(section: &DocumentSection) -> Vec<ProjectionEntry> {
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
    for block in &section.blocks {
        if let DocumentBlock::Paragraph(paragraph) = block
            && let Some(entry) = entry_from_paragraph(paragraph)
        {
            entries.push(entry);
        }
    }
    entries
}

fn synthetic_entries_from_section_tree(
    source: &DocumentModel,
    root_section: &DocumentSection,
    id_prefix: &str,
) -> Vec<ProjectionEntry> {
    let mut entries = Vec::new();
    for section in source
        .sections
        .iter()
        .filter(|section| section.heading_path.starts_with(&root_section.heading_path))
    {
        for block in &section.blocks {
            match block {
                DocumentBlock::BulletList(items) | DocumentBlock::OrderedList(items) => {
                    for item in items {
                        let text = clean_checkbox_text(item);
                        if text.is_empty() {
                            continue;
                        }
                        entries.push(ProjectionEntry {
                            id: format!("{id_prefix}{:03}", entries.len() + 1),
                            text,
                            fields: HashMap::new(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    entries
}

fn synthetic_table_entries_from_section_tree(
    source: &DocumentModel,
    root_section: &DocumentSection,
    id_prefix: &str,
) -> Vec<ProjectionEntry> {
    let mut entries = Vec::new();
    for section in source
        .sections
        .iter()
        .filter(|section| section.heading_path.starts_with(&root_section.heading_path))
    {
        for block in &section.blocks {
            if let DocumentBlock::Table { headers, rows } = block {
                for row in rows {
                    let text = synthetic_table_text(headers, row);
                    if text.is_empty() {
                        continue;
                    }
                    entries.push(ProjectionEntry {
                        id: format!("{id_prefix}{:03}", entries.len() + 1),
                        text,
                        fields: table_fields(headers, row),
                    });
                }
            }
        }
    }
    entries
}

fn synthetic_heading_entries_from_section_tree(
    source: &DocumentModel,
    root_section: &DocumentSection,
    id_prefix: &str,
) -> Vec<ProjectionEntry> {
    let mut entries = Vec::new();
    for section in source.sections.iter().filter(|section| {
        section.heading_path.starts_with(&root_section.heading_path)
            && section.heading_path != root_section.heading_path
    }) {
        let Some(heading) = section.heading_path.last() else {
            continue;
        };
        let text = clean_text(normalized_heading(heading));
        if text.is_empty() {
            continue;
        }
        entries.push(ProjectionEntry {
            id: format!("{id_prefix}{:03}", entries.len() + 1),
            text,
            fields: extract_metadata(&clean_metadata_text(&section.raw_text)),
        });
    }
    entries
}

fn synthetic_table_text(headers: &[String], row: &[String]) -> String {
    headers
        .iter()
        .zip(row.iter())
        .filter_map(|(header, value)| {
            let header = clean_table_cell(header);
            let value = clean_table_cell(value);
            (!header.is_empty() && !value.is_empty()).then_some(format!("{header}: {value}"))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn structured_api_entries_from_section_tree(
    source: &DocumentModel,
    root_section: &DocumentSection,
) -> Vec<ProjectionEntry> {
    let mut entries = Vec::new();
    for section in source
        .sections
        .iter()
        .filter(|section| section.heading_path.starts_with(&root_section.heading_path))
    {
        for block in &section.blocks {
            let DocumentBlock::Table { headers, rows } = block else {
                continue;
            };
            if let Some(entry) = api_entry_from_key_value_table(headers, rows, entries.len() + 1) {
                entries.push(entry);
                continue;
            }
            entries.extend(api_entries_from_record_table(
                headers,
                rows,
                entries.len() + 1,
            ));
        }
    }
    entries
}

fn api_entry_from_key_value_table(
    headers: &[String],
    rows: &[Vec<String>],
    sequence: usize,
) -> Option<ProjectionEntry> {
    if !is_key_value_table(headers) {
        return None;
    }
    let pairs = rows
        .iter()
        .filter_map(|row| {
            let key = row.first().map(|value| normalize_key(value))?;
            let value = row.get(1).map(|value| clean_table_cell(value))?;
            (!key.is_empty() && !value.is_empty()).then_some((key, value))
        })
        .collect::<Vec<_>>();
    let name = value_for_key(&pairs, &["name", "名称"])?;
    let mut fields = pairs.iter().cloned().collect::<HashMap<String, String>>();
    fields.insert("name".to_string(), name.clone());
    let input = summarize_prefixed_fields(&pairs, "input", &["输入", "请求", "请求契约"]);
    if !input.is_empty() {
        fields.insert("input".to_string(), input);
    }
    let output = summarize_prefixed_fields(&pairs, "output", &["输出", "响应", "成功响应"]);
    if !output.is_empty() {
        fields.insert("output".to_string(), output);
    }
    Some(ProjectionEntry {
        id: format!("api-{sequence:03}"),
        text: name,
        fields,
    })
}

fn api_entries_from_record_table(
    headers: &[String],
    rows: &[Vec<String>],
    start_sequence: usize,
) -> Vec<ProjectionEntry> {
    let mut entries = Vec::new();
    for row in rows {
        let mut fields = table_fields(headers, row);
        let Some(name) = field_from_fields(&fields, &["name", "名称", "路径"]) else {
            continue;
        };
        fields.insert("name".to_string(), name.clone());
        entries.push(ProjectionEntry {
            id: format!("api-{:03}", start_sequence + entries.len()),
            text: name,
            fields,
        });
    }
    entries
}

fn is_key_value_table(headers: &[String]) -> bool {
    let first = headers.first().map(|value| normalize_key(value));
    let second = headers.get(1).map(|value| normalize_key(value));
    matches!(
        (first.as_deref(), second.as_deref()),
        (
            Some("字段" | "field" | "key" | "属性"),
            Some("值" | "value")
        )
    )
}

fn value_for_key(pairs: &[(String, String)], aliases: &[&str]) -> Option<String> {
    aliases.iter().find_map(|alias| {
        let normalized = normalize_key(alias);
        pairs
            .iter()
            .find(|(key, _)| key == &normalized)
            .map(|(_, value)| value.clone())
    })
}

fn summarize_prefixed_fields(pairs: &[(String, String)], prefix: &str, aliases: &[&str]) -> String {
    let prefix_with_dot = format!("{prefix}.");
    let normalized_aliases = aliases
        .iter()
        .map(|alias| normalize_key(alias))
        .collect::<HashSet<_>>();
    pairs
        .iter()
        .filter(|(key, _)| {
            key == prefix || key.starts_with(&prefix_with_dot) || normalized_aliases.contains(key)
        })
        .map(|(key, value)| format!("{key}: {value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn entry_from_paragraph(paragraph: &str) -> Option<ProjectionEntry> {
    let first_line = paragraph.lines().find(|line| !line.trim().is_empty())?;
    let cleaned = clean_inline_markup(first_line);
    let (id, text) = split_heading_entry(&cleaned)?;
    let id = normalize_id(id);
    if !is_projection_entry_id(&id) {
        return None;
    }
    Some(ProjectionEntry {
        id,
        text: clean_text(text),
        fields: extract_metadata(&clean_metadata_text(paragraph)),
    })
}

fn entry_from_section_heading(section: &DocumentSection) -> Option<ProjectionEntry> {
    let heading = section.heading_path.last()?;
    let normalized = normalized_heading(heading);
    let (id, text) = split_heading_entry(normalized)?;
    let id = normalize_id(id);
    if !is_projection_entry_id(&id) {
        return None;
    }
    Some(ProjectionEntry {
        id,
        text: clean_text(text),
        fields: extract_metadata(&clean_metadata_text(&section.raw_text)),
    })
}

fn split_heading_entry(heading: &str) -> Option<(&str, &str)> {
    let trimmed = heading.trim();
    let separator_index = trimmed.char_indices().find_map(|(index, character)| {
        (character.is_whitespace() || matches!(character, ':' | '：')).then_some(index)
    })?;
    let id = &trimmed[..separator_index];
    let text = trimmed[separator_index..]
        .trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, ':' | '：' | '-')
        })
        .trim();
    (!id.is_empty() && !text.is_empty()).then_some((id, text))
}

fn is_projection_entry_id(id: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "us-", "req-", "fr-", "ac-", "sc-", "oq-", "nf-", "nfr-", "dd-", "dec-", "cmp-", "api-",
        "risk-", "wt-", "de-", "sm-",
    ];
    PREFIXES.iter().any(|prefix| {
        id.strip_prefix(prefix).is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '.')
                })
        })
    })
}

fn entry_from_table_row(headers: &[String], row: &[String]) -> Option<ProjectionEntry> {
    let fields = table_fields(headers, row);
    let id = fields
        .get("id")
        .or_else(|| fields.get("需求_id"))
        .or_else(|| fields.get("需求id"))
        .or_else(|| fields.get("验收标准_id"))
        .or_else(|| fields.get("验收标准id"))
        .or_else(|| fields.get("决策_id"))
        .or_else(|| fields.get("决策id"))
        .or_else(|| fields.get("风险_id"))
        .or_else(|| fields.get("风险id"))
        .or_else(|| fields.get("组件标识"))
        .or_else(|| fields.get("组件id"))
        .or_else(|| fields.get("实体标识"))
        .or_else(|| fields.get("实体id"))
        .or_else(|| fields.get("api_标识"))
        .or_else(|| fields.get("api_id"))
        .or_else(|| fields.get("模块标识"))
        .or_else(|| fields.get("模块id"))
        .or_else(|| fields.get("编号"))
        .or_else(|| fields.get("work_package_id"))
        .or_else(|| fields.get("group"))
        .or_else(|| fields.get("from"))?
        .to_string();
    let text = fields
        .get("text")
        .or_else(|| fields.get("description"))
        .or_else(|| fields.get("风险描述"))
        .or_else(|| fields.get("风险"))
        .or_else(|| fields.get("描述"))
        .or_else(|| fields.get("说明"))
        .or_else(|| fields.get("decision"))
        .or_else(|| fields.get("决策"))
        .or_else(|| fields.get("决策项"))
        .or_else(|| fields.get("选择"))
        .or_else(|| fields.get("需求描述"))
        .or_else(|| fields.get("验收标准"))
        .or_else(|| fields.get("验收标准描述"))
        .or_else(|| fields.get("标准"))
        .or_else(|| fields.get("标准描述"))
        .or_else(|| fields.get("问题"))
        .or_else(|| fields.get("问题描述"))
        .or_else(|| fields.get("用户故事"))
        .or_else(|| fields.get("故事"))
        .or_else(|| fields.get("内容"))
        .cloned()
        .unwrap_or_default();
    Some(ProjectionEntry {
        id: normalize_id(&id),
        text,
        fields,
    })
}

fn table_fields(headers: &[String], row: &[String]) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for (header, value) in headers.iter().zip(row.iter()) {
        fields.insert(normalize_key(header), clean_table_cell(value));
    }
    fields
}

fn entry_from_bullet(item: &str) -> Option<ProjectionEntry> {
    if let Some(start) = item.find('[') {
        let end = item[start + 1..].find(']')? + start + 1;
        let id = normalize_id(&item[start + 1..end]);
        if is_projection_entry_id(&id) && !is_metadata_reference_prefix(&item[..start]) {
            let rest = item[end + 1..].trim();
            let metadata = extract_metadata(rest);
            let text_end = first_metadata_position(rest).unwrap_or(rest.len());
            let text = clean_text(&rest[..text_end]);
            return Some(ProjectionEntry {
                id,
                text,
                fields: metadata,
            });
        }
    }
    entry_from_paragraph(item)
}

fn is_metadata_reference_prefix(prefix: &str) -> bool {
    let normalized = clean_inline_markup(prefix)
        .trim()
        .trim_end_matches([':', '：'])
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_");
    matches!(
        normalized.as_str(),
        "related"
            | "refs"
            | "reqs"
            | "requirements"
            | "designs"
            | "traceability"
            | "acceptance"
            | "关联"
            | "关联需求"
            | "相关"
            | "相关需求"
            | "需求"
            | "related_requirement_ids"
            | "related_design_decision_ids"
            | "related_acceptance_criterion_ids"
    )
}

fn requirement_from_entry(
    entry: &ProjectionEntry,
) -> Result<RequirementProjection, ProjectionCompileError> {
    ensure_id_prefix_any(&entry.id, &["req-", "fr-"], "req-<number> or fr-<number>")?;
    requirement_projection_from_entry(entry)
}

fn non_functional_requirement_from_entry(
    entry: &ProjectionEntry,
) -> Result<RequirementProjection, ProjectionCompileError> {
    ensure_id_prefix_any(
        &entry.id,
        &["req-", "nf-", "nfr-"],
        "req-<number>, nf-<number>, or nfr-<number>",
    )?;
    requirement_projection_from_entry(entry)
}

fn requirement_projection_from_entry(
    entry: &ProjectionEntry,
) -> Result<RequirementProjection, ProjectionCompileError> {
    let priority = parse_requirement_priority(entry)?;
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
    ensure_id_prefix_any(&entry.id, &["ac-", "sc-"], "ac-<number> or sc-<number>")?;
    let related_requirement_ids =
        field_values(entry, &["refs", "reqs", "requirements", "关联需求", "需求"])
            .into_iter()
            .filter(|ref_id| is_requirement_ref_id(ref_id))
            .collect::<Vec<_>>();
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

fn is_requirement_ref_id(value: &str) -> bool {
    value.starts_with("req-")
        || value.starts_with("fr-")
        || value.starts_with("nf-")
        || value.starts_with("nfr-")
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
    ensure_id_prefix_any(&entry.id, &["dd-", "dec-"], "dd-<number> or dec-<number>")?;
    Ok(DesignDecisionProjection {
        design_decision_id: entry.id.clone(),
        text: required_text(entry)?,
        related_requirement_ids: field_values(
            entry,
            &["refs", "reqs", "requirements", "related_requirement_ids"],
        ),
    })
}

fn component_from_entry(
    entry: &ProjectionEntry,
) -> Result<ComponentProjection, ProjectionCompileError> {
    Ok(ComponentProjection {
        component_id: entry.id.clone(),
        name: field(entry, &["name", "组件名", "模块名", "组件", "模块"])
            .unwrap_or_else(|| fallback_name(entry)),
        responsibility: field(entry, &["responsibility", "职责", "责任"]).unwrap_or_default(),
    })
}

fn data_entity_from_entry(
    entry: &ProjectionEntry,
) -> Result<DataEntityProjection, ProjectionCompileError> {
    Ok(DataEntityProjection {
        entity_id: entry.id.clone(),
        name: field(entry, &["name", "实体名"]).unwrap_or_else(|| fallback_name(entry)),
        fields: field_values(entry, &["fields", "字段", "字段定义"]),
    })
}

fn api_entry_from_entry(
    entry: &ProjectionEntry,
) -> Result<ApiEntryProjection, ProjectionCompileError> {
    Ok(ApiEntryProjection {
        api_id: entry.id.clone(),
        name: field(entry, &["name", "路径"]).unwrap_or_else(|| fallback_name(entry)),
        input: field(entry, &["input", "输入", "请求", "请求契约"]).unwrap_or_default(),
        output: field(entry, &["output", "输出", "响应", "成功响应"]).unwrap_or_default(),
    })
}

fn risk_from_entry(
    entry: &ProjectionEntry,
    known_decisions: &HashSet<String>,
) -> Result<RiskProjection, ProjectionCompileError> {
    let risk_id = canonical_risk_id(&entry.id);
    ensure_id_prefix(&risk_id, "risk-")?;
    let related_design_decision_ids =
        field_values(entry, &["refs", "designs", "related_design_decision_ids"]);
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
        risk_id,
        text: required_text(entry)?,
        severity,
        mitigation: field(entry, &["mitigation", "缓解", "缓解措施"]),
        related_design_decision_ids,
    })
}

fn canonical_risk_id(id: &str) -> String {
    id.strip_prefix("r-")
        .map(|suffix| format!("risk-{suffix}"))
        .unwrap_or_else(|| id.to_string())
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
    ensure_id_prefix_any(id, &[prefix], &format!("{prefix}<number>"))
}

fn ensure_id_prefix_any(
    id: &str,
    prefixes: &[&str],
    expected_pattern: &str,
) -> Result<(), ProjectionCompileError> {
    if prefixes.iter().any(|prefix| id.starts_with(prefix)) {
        Ok(())
    } else {
        Err(ProjectionCompileError::InvalidIdFormat {
            id: id.to_string(),
            expected_pattern: expected_pattern.to_string(),
        })
    }
}

fn required_text(entry: &ProjectionEntry) -> Result<String, ProjectionCompileError> {
    if !entry.text.trim().is_empty() {
        Ok(entry.text.trim().to_string())
    } else if let Some(value) = field(
        entry,
        &[
            "text",
            "description",
            "风险描述",
            "风险",
            "描述",
            "说明",
            "需求描述",
            "验收标准",
            "验收标准描述",
            "标准",
            "标准描述",
            "问题",
            "问题描述",
            "用户故事",
            "故事",
            "内容",
        ],
    ) {
        Ok(value)
    } else {
        Err(ProjectionCompileError::InvalidIdFormat {
            id: entry.id.clone(),
            expected_pattern: "non-empty text".to_string(),
        })
    }
}

fn fallback_name(entry: &ProjectionEntry) -> String {
    if entry.text.trim().is_empty() {
        entry.id.clone()
    } else {
        entry.text.trim().to_string()
    }
}

fn field(entry: &ProjectionEntry, aliases: &[&str]) -> Option<String> {
    field_from_fields(&entry.fields, aliases)
}

fn field_from_fields(fields: &HashMap<String, String>, aliases: &[&str]) -> Option<String> {
    aliases
        .iter()
        .find_map(|alias| fields.get(&normalize_key(alias)).cloned())
        .map(|value| value.trim().trim_matches(';').trim().to_string())
}

fn field_values(entry: &ProjectionEntry, aliases: &[&str]) -> Vec<String> {
    field(entry, aliases)
        .map(|value| split_values(&value))
        .unwrap_or_default()
}

fn split_values(value: &str) -> Vec<String> {
    let value = clean_inline_markup(value);
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split([',', ';', '，', '；', '、'])
        .map(normalize_id)
        .filter(|value| !value.is_empty())
        .collect()
}

fn parse_requirement_priority(
    entry: &ProjectionEntry,
) -> Result<RequirementPriority, ProjectionCompileError> {
    let value = field(entry, &["priority", "优先级"]).unwrap_or_else(|| "should".to_string());
    let normalized = match value.trim().to_ascii_lowercase().as_str() {
        "p0" => "must".to_string(),
        "p1" => "should".to_string(),
        "p2" => "could".to_string(),
        "p3" => "wont".to_string(),
        "高" | "最高" | "必须" | "必需" => "must".to_string(),
        "中" | "中等" | "应该" => "should".to_string(),
        "低" | "可选" | "可以" => "could".to_string(),
        _ => value,
    };
    normalized
        .parse::<RequirementPriority>()
        .map_err(|value| ProjectionCompileError::PriorityInvalid { value })
}

fn normalize_id(value: &str) -> String {
    clean_inline_markup(value)
        .trim()
        .trim_matches(';')
        .trim_matches(',')
        .to_ascii_lowercase()
        .replace('_', "-")
}

fn normalize_key(value: &str) -> String {
    clean_inline_markup(value)
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
}

fn clean_text(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(';')
        .trim_end_matches(',')
        .trim()
        .to_string()
}

fn clean_table_cell(value: &str) -> String {
    clean_inline_markup(value).trim().to_string()
}

fn clean_inline_markup(value: &str) -> String {
    let cleaned = value.trim().replace("**", "").replace("__", "");
    strip_balanced_outer_markup(&cleaned).trim().to_string()
}

fn strip_balanced_outer_markup(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some(inner) = trimmed
        .strip_prefix('`')
        .and_then(|value| value.strip_suffix('`'))
        .or_else(|| {
            trimmed
                .strip_prefix('*')
                .and_then(|value| value.strip_suffix('*'))
        })
        .or_else(|| {
            trimmed
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
        })
    {
        inner.trim()
    } else {
        trimmed
    }
}

fn clean_checkbox_text(value: &str) -> String {
    let trimmed = value.trim();
    let without_marker = trimmed
        .strip_prefix("[ ]")
        .or_else(|| trimmed.strip_prefix("[x]"))
        .or_else(|| trimmed.strip_prefix("[X]"))
        .unwrap_or(trimmed);
    clean_text(without_marker)
}

fn clean_metadata_text(value: &str) -> String {
    value.replace("**", "").replace('`', "")
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
    let inline_value = tail[..end].lines().next().unwrap_or_default();
    Some(clean_text(inline_value))
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
    "related_requirement_ids:",
    "related_design_decision_ids:",
    "related_acceptance_criterion_ids:",
];

#[allow(dead_code)]
fn source_hash_from_text(text: &str) -> String {
    compute_sha256(text.as_bytes())
}
