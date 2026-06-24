use super::fields::{
    clean_checkbox_text, clean_inline_markup, clean_metadata_text, clean_table_cell, clean_text,
    extract_metadata, field_from_fields, first_metadata_position, normalize_id, normalize_key,
};
use crate::protocol::document_ops::{DocumentBlock, DocumentModel, DocumentSection};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionEntry {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) fields: HashMap<String, String>,
}

pub(crate) fn find_section<'a>(
    source: &'a DocumentModel,
    aliases: &[&str],
) -> Option<&'a DocumentSection> {
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

pub(crate) fn entries_from_section_tree(
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

pub(crate) fn synthetic_entries_from_section_tree(
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

pub(crate) fn synthetic_table_entries_from_section_tree(
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

pub(crate) fn synthetic_heading_entries_from_section_tree(
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

pub(crate) fn structured_api_entries_from_section_tree(
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
