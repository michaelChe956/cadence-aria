use super::entry::ProjectionEntry;
use super::error::ProjectionCompileError;
use super::fields::{
    fallback_name, field, field_values, normalize_id, parse_requirement_priority, required_text,
};
use crate::protocol::projections::{
    ApiEntryProjection, ComponentProjection, CriterionProjection, DataEntityProjection,
    DependencyType, DesignDecisionProjection, ExecutionMode, OpenItemProjection,
    ParallelismGroupProjection, RequirementProjection, ResolutionMode, RiskProjection,
    RiskSeverity, UserStoryProjection, WorkDependencyProjection, WorkPackageProjection,
};
use std::collections::HashSet;

pub(crate) fn requirement_from_entry(
    entry: &ProjectionEntry,
) -> Result<RequirementProjection, ProjectionCompileError> {
    ensure_id_prefix_any(&entry.id, &["req-", "fr-"], "req-<number> or fr-<number>")?;
    requirement_projection_from_entry(entry)
}

pub(crate) fn non_functional_requirement_from_entry(
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

pub(crate) fn criterion_from_entry(
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

pub(crate) fn user_story_from_entry(
    entry: &ProjectionEntry,
) -> Result<UserStoryProjection, ProjectionCompileError> {
    Ok(UserStoryProjection {
        story_id: entry.id.clone(),
        title: required_text(entry)?,
        related_requirement_ids: field_values(entry, &["refs", "requirements"]),
    })
}

pub(crate) fn open_item_from_entry(
    entry: &ProjectionEntry,
) -> Result<OpenItemProjection, ProjectionCompileError> {
    Ok(OpenItemProjection {
        item_id: entry.id.clone(),
        text: required_text(entry)?,
        resolution_mode: ResolutionMode::Deferred,
    })
}

pub(crate) fn design_decision_from_entry(
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

pub(crate) fn component_from_entry(
    entry: &ProjectionEntry,
) -> Result<ComponentProjection, ProjectionCompileError> {
    Ok(ComponentProjection {
        component_id: entry.id.clone(),
        name: field(entry, &["name", "组件名", "模块名", "组件", "模块"])
            .unwrap_or_else(|| fallback_name(entry)),
        responsibility: field(entry, &["responsibility", "职责", "责任"]).unwrap_or_default(),
    })
}

pub(crate) fn data_entity_from_entry(
    entry: &ProjectionEntry,
) -> Result<DataEntityProjection, ProjectionCompileError> {
    Ok(DataEntityProjection {
        entity_id: entry.id.clone(),
        name: field(entry, &["name", "实体名"]).unwrap_or_else(|| fallback_name(entry)),
        fields: field_values(entry, &["fields", "字段", "字段定义"]),
    })
}

pub(crate) fn api_entry_from_entry(
    entry: &ProjectionEntry,
) -> Result<ApiEntryProjection, ProjectionCompileError> {
    Ok(ApiEntryProjection {
        api_id: entry.id.clone(),
        name: field(entry, &["name", "路径"]).unwrap_or_else(|| fallback_name(entry)),
        input: field(entry, &["input", "输入", "请求", "请求契约"]).unwrap_or_default(),
        output: field(entry, &["output", "输出", "响应", "成功响应"]).unwrap_or_default(),
    })
}

pub(crate) fn risk_from_entry(
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

pub(crate) fn work_package_from_entry(
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

pub(crate) fn dependency_from_entry(
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

pub(crate) fn parallelism_group_from_entry(
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

pub(crate) fn ensure_unique_work_packages(
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
