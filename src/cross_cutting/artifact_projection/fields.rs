use super::entry::ProjectionEntry;
use super::error::ProjectionCompileError;
use crate::cross_cutting::document_ops::compute_sha256;
use crate::protocol::projections::RequirementPriority;
use std::collections::HashMap;

pub(crate) fn required_text(entry: &ProjectionEntry) -> Result<String, ProjectionCompileError> {
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

pub(crate) fn fallback_name(entry: &ProjectionEntry) -> String {
    if entry.text.trim().is_empty() {
        entry.id.clone()
    } else {
        entry.text.trim().to_string()
    }
}

pub(crate) fn field(entry: &ProjectionEntry, aliases: &[&str]) -> Option<String> {
    field_from_fields(&entry.fields, aliases)
}

pub(crate) fn field_from_fields(
    fields: &HashMap<String, String>,
    aliases: &[&str],
) -> Option<String> {
    aliases
        .iter()
        .find_map(|alias| fields.get(&normalize_key(alias)).cloned())
        .map(|value| value.trim().trim_matches(';').trim().to_string())
}

pub(crate) fn field_values(entry: &ProjectionEntry, aliases: &[&str]) -> Vec<String> {
    field(entry, aliases)
        .map(|value| split_values(&value))
        .unwrap_or_default()
}

pub(crate) fn split_values(value: &str) -> Vec<String> {
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

pub(crate) fn parse_requirement_priority(
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

pub(crate) fn normalize_id(value: &str) -> String {
    clean_inline_markup(value)
        .trim()
        .trim_matches(';')
        .trim_matches(',')
        .to_ascii_lowercase()
        .replace('_', "-")
}

pub(crate) fn normalize_key(value: &str) -> String {
    clean_inline_markup(value)
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
}

pub(crate) fn clean_text(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(';')
        .trim_end_matches(',')
        .trim()
        .to_string()
}

pub(crate) fn clean_table_cell(value: &str) -> String {
    clean_inline_markup(value).trim().to_string()
}

pub(crate) fn clean_inline_markup(value: &str) -> String {
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

pub(crate) fn clean_checkbox_text(value: &str) -> String {
    let trimmed = value.trim();
    let without_marker = trimmed
        .strip_prefix("[ ]")
        .or_else(|| trimmed.strip_prefix("[x]"))
        .or_else(|| trimmed.strip_prefix("[X]"))
        .unwrap_or(trimmed);
    clean_text(without_marker)
}

pub(crate) fn clean_metadata_text(value: &str) -> String {
    value.replace("**", "").replace('`', "")
}

pub(crate) fn extract_metadata(rest: &str) -> HashMap<String, String> {
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

pub(crate) fn first_metadata_position(rest: &str) -> Option<usize> {
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
