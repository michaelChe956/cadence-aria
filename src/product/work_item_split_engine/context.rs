use std::path::Path;

use serde_json::json;

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{DesignContextCapabilities, IssueRecord};
use crate::web::error::{ApiError, ApiResult};
use crate::web::types::GenerateWorkItemsRequest;

pub fn design_context_capabilities_for_request(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
) -> ApiResult<DesignContextCapabilities> {
    let design_context = collect_design_context(lifecycle, request, issue)?;
    Ok(merge_design_context_capabilities(&design_context))
}

pub fn design_context_gaps(capabilities: &DesignContextCapabilities) -> Vec<String> {
    let mut gaps = Vec::new();
    if !capabilities.has_architecture {
        gaps.push("missing_architecture".to_string());
    }
    if !capabilities.has_module_breakdown {
        gaps.push("missing_module_breakdown".to_string());
    }
    if !capabilities.has_tech_stack {
        gaps.push("missing_tech_stack".to_string());
    }
    if !capabilities.has_test_strategy {
        gaps.push("missing_test_strategy".to_string());
    }
    if !capabilities.has_key_paths {
        gaps.push("missing_key_paths".to_string());
    }
    gaps
}

pub(crate) fn collect_story_context(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
) -> ApiResult<Vec<String>> {
    let project_id = &issue.project_id;
    let issue_id = &issue.id;
    let story_specs = lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(super::types::product_store_api_error)?;

    request
        .story_spec_ids
        .iter()
        .map(|id| {
            let spec = story_specs.iter().find(|s| &s.id == id).ok_or_else(|| {
                ApiError::runtime("story_spec_not_found", "story spec not found", json!({}))
            })?;
            let markdown = latest_markdown(lifecycle, project_id, issue_id, id)?;
            Ok(format!(
                "Story Spec: {} ({})\n{}",
                spec.title, spec.id, markdown
            ))
        })
        .collect::<ApiResult<Vec<_>>>()
}

pub(crate) fn collect_design_context(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
) -> ApiResult<Vec<String>> {
    let project_id = &issue.project_id;
    let issue_id = &issue.id;
    let design_specs = lifecycle
        .list_design_specs(project_id, issue_id)
        .map_err(super::types::product_store_api_error)?;

    request
        .design_spec_ids
        .iter()
        .map(|id| {
            let spec = design_specs.iter().find(|s| &s.id == id).ok_or_else(|| {
                ApiError::runtime("design_spec_not_found", "design spec not found", json!({}))
            })?;
            let markdown = latest_markdown(lifecycle, project_id, issue_id, id)?;
            Ok(format!(
                "Design Spec: {} ({})\n{}",
                spec.title, spec.id, markdown
            ))
        })
        .collect::<ApiResult<Vec<_>>>()
}

fn latest_markdown(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_id: &str,
) -> ApiResult<String> {
    let versions = lifecycle
        .list_versions(project_id, issue_id, entity_id)
        .map_err(super::types::product_store_api_error)?;
    Ok(versions
        .into_iter()
        .max_by_key(|v| v.version)
        .map(|v| v.markdown)
        .unwrap_or_else(|| "(no version)".to_string()))
}

pub(crate) fn summarize_repository_structure(path: &Path) -> String {
    let mut entries = Vec::new();
    if let Ok(reader) = std::fs::read_dir(path) {
        for entry in reader.flatten() {
            if let Ok(metadata) = entry.metadata() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".git" {
                    continue;
                }
                let kind = if metadata.is_dir() { "dir" } else { "file" };
                entries.push(format!("{kind}: {name}"));
            }
        }
    }
    entries.sort();
    entries.truncate(30);
    if entries.is_empty() {
        "(empty repository)".to_string()
    } else {
        entries.join("\n")
    }
}

pub fn extract_design_context_capabilities(markdown: &str) -> DesignContextCapabilities {
    let normalized = markdown_headings(markdown).join("\n").to_lowercase();
    DesignContextCapabilities {
        has_architecture: contains_any(&normalized, &["架构概览", "系统架构", "architecture"]),
        has_module_breakdown: contains_any(
            &normalized,
            &["模块划分", "模块拆分", "modules", "module breakdown"],
        ),
        has_tech_stack: contains_any(
            &normalized,
            &["技术选型", "技术栈", "tech stack", "technology"],
        ),
        has_test_strategy: contains_any(
            &normalized,
            &["测试框架", "测试策略", "test strategy", "testing strategy"],
        ),
        has_key_paths: contains_any(
            &normalized,
            &[
                "关键目录结构",
                "关键路径",
                "key paths",
                "directory structure",
            ],
        ),
    }
}

pub(crate) fn merge_design_context_capabilities(
    design_context: &[String],
) -> DesignContextCapabilities {
    design_context.iter().fold(
        DesignContextCapabilities {
            has_architecture: false,
            has_module_breakdown: false,
            has_tech_stack: false,
            has_test_strategy: false,
            has_key_paths: false,
        },
        |mut merged, markdown| {
            let current = extract_design_context_capabilities(markdown);
            merged.has_architecture |= current.has_architecture;
            merged.has_module_breakdown |= current.has_module_breakdown;
            merged.has_tech_stack |= current.has_tech_stack;
            merged.has_test_strategy |= current.has_test_strategy;
            merged.has_key_paths |= current.has_key_paths;
            merged
        },
    )
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn markdown_headings(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                Some(trimmed.trim_matches('#').trim().to_string())
            } else {
                None
            }
        })
        .collect()
}
