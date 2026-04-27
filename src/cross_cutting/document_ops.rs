use crate::protocol::document_ops::{
    DocumentBlock, DocumentModel, DocumentPatchResult, DocumentSection, HeadingPath,
};
use pulldown_cmark::{Options, Parser};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentTemplateKind {
    OpenspecProposal,
    OpenspecSpec,
    OpenspecDesign,
    OpenspecTasks,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocumentOpError {
    #[error("io error: {0}")]
    IoError(String),
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("section not found: {0:?}")]
    SectionNotFound(HeadingPath),
    #[error("invalid heading path: {0}")]
    InvalidHeadingPath(String),
    #[error("patch conflict: {0}")]
    PatchConflict(String),
    #[error("unknown template: {0}")]
    TemplateUnknown(String),
    #[error("missing optional tool: {0}")]
    MissingOptionalTool(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JsonPatch {
    pub operations: Vec<JsonPatchOperation>,
}

impl JsonPatch {
    pub fn new(operations: Vec<JsonPatchOperation>) -> Self {
        Self { operations }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum JsonPatchOperation {
    Add { path: String, value: Value },
    Replace { path: String, value: Value },
}

#[derive(Debug, Clone)]
struct HeadingSpan {
    heading_path: Vec<String>,
    level: u8,
    start_line: u32,
}

pub fn read_document_model(path: &Path) -> Result<DocumentModel, DocumentOpError> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| DocumentOpError::IoError(format!("read {}: {error}", path.display())))?;
    parse_document_model(path.to_path_buf(), content)
}

pub fn create_document(
    path: &Path,
    template_kind: DocumentTemplateKind,
) -> Result<DocumentModel, DocumentOpError> {
    if path.exists() {
        return Err(DocumentOpError::IoError(format!(
            "refusing to overwrite existing document {}",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            DocumentOpError::IoError(format!("create {}: {error}", parent.display()))
        })?;
    }

    let template = template_for(&template_kind)?;
    std::fs::write(path, template)
        .map_err(|error| DocumentOpError::IoError(format!("write {}: {error}", path.display())))?;
    parse_document_model(path.to_path_buf(), template.to_string())
}

pub fn upsert_section(
    model: &mut DocumentModel,
    path: &HeadingPath,
    new_blocks: Vec<DocumentBlock>,
) -> Result<DocumentPatchResult, DocumentOpError> {
    if path.0.is_empty() {
        return Err(DocumentOpError::InvalidHeadingPath(
            "heading path must not be empty".to_string(),
        ));
    }

    let section = model
        .sections
        .iter()
        .find(|section| section.heading_path == path.0)
        .ok_or_else(|| DocumentOpError::SectionNotFound(path.clone()))?;
    let old_sha256 = model.sha256.clone();
    let offsets = line_start_offsets(&model.source_text);
    let content_start = offset_for_line(&offsets, section.start_line + 1);
    let content_end = offset_for_line(&offsets, section.end_line + 1);
    let replacement = render_blocks(&new_blocks);

    let mut next_source = model.source_text.clone();
    next_source.replace_range(content_start..content_end, &replacement);
    let changed = next_source != model.source_text;
    let new_sha256 = compute_sha256(next_source.as_bytes());

    let mut reparsed = parse_document_model(PathBuf::from(&model.source_path), next_source)?;
    model.sha256 = reparsed.sha256;
    model.sections = std::mem::take(&mut reparsed.sections);
    model.source_text = reparsed.source_text;

    Ok(DocumentPatchResult {
        changed,
        old_sha256,
        new_sha256,
        updated_heading_path: path.clone(),
        warnings: Vec::new(),
    })
}

pub fn extract_projection_source(
    model: &DocumentModel,
    heading_path: &HeadingPath,
) -> Result<String, DocumentOpError> {
    model
        .sections
        .iter()
        .find(|section| section.heading_path == heading_path.0)
        .map(|section| section.raw_text.clone())
        .ok_or_else(|| DocumentOpError::SectionNotFound(heading_path.clone()))
}

pub fn apply_json_patch(value: &mut Value, patch: &JsonPatch) -> Result<(), DocumentOpError> {
    for operation in &patch.operations {
        match operation {
            JsonPatchOperation::Add { path, value: next } => {
                apply_json_value(value, path, next.clone(), PatchMode::Add)?
            }
            JsonPatchOperation::Replace { path, value: next } => {
                apply_json_value(value, path, next.clone(), PatchMode::Replace)?
            }
        }
    }
    Ok(())
}

pub fn compute_sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

pub fn render_document_model(model: &DocumentModel) -> String {
    model.source_text.clone()
}

fn parse_document_model(
    source_path: PathBuf,
    source_text: String,
) -> Result<DocumentModel, DocumentOpError> {
    let headings = collect_heading_spans(&source_text);
    let offsets = line_start_offsets(&source_text);
    let sections = headings
        .iter()
        .enumerate()
        .map(|(index, heading)| {
            let next_start_line = headings
                .get(index + 1)
                .map(|next| next.start_line)
                .unwrap_or_else(|| line_count(&source_text) + 1);
            let body_start = offset_for_line(&offsets, heading.start_line + 1);
            let body_end = offset_for_line(&offsets, next_start_line);
            let raw_text = source_text[body_start..body_end].to_string();
            DocumentSection {
                heading_path: heading.heading_path.clone(),
                level: heading.level,
                start_line: heading.start_line,
                end_line: next_start_line.saturating_sub(1),
                blocks: parse_blocks(&raw_text),
                raw_text,
            }
        })
        .collect();

    Ok(DocumentModel {
        source_path: source_path.to_string_lossy().to_string(),
        sha256: compute_sha256(source_text.as_bytes()),
        sections,
        source_text,
    })
}

fn collect_heading_spans(source_text: &str) -> Vec<HeadingSpan> {
    let mut stack: Vec<(u8, String)> = Vec::new();
    let mut headings = Vec::new();

    for (index, line) in source_text.lines().enumerate() {
        if let Some((level, title)) = parse_heading(line) {
            while stack
                .last()
                .is_some_and(|(existing_level, _)| *existing_level >= level)
            {
                stack.pop();
            }
            stack.push((level, title));
            headings.push(HeadingSpan {
                heading_path: stack.iter().map(|(_, title)| title.clone()).collect(),
                level,
                start_line: (index + 1) as u32,
            });
        }
    }

    headings
}

fn parse_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    let level = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.starts_with(' ') {
        return None;
    }
    let title = rest.trim().trim_end_matches('#').trim().to_string();
    (!title.is_empty()).then_some((level as u8, title))
}

fn parse_blocks(raw_text: &str) -> Vec<DocumentBlock> {
    // pulldown-cmark 是一期固定 tokenizer；下面的行级模型只消费已切分 section 的源码。
    Parser::new_ext(raw_text, Options::ENABLE_TABLES).for_each(drop);

    let lines: Vec<&str> = raw_text.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty() {
            index += 1;
            continue;
        }

        if let Some((lang, next_index, text)) = parse_code_block(&lines, index) {
            blocks.push(DocumentBlock::CodeBlock { lang, text });
            index = next_index;
            continue;
        }

        if is_table_start(&lines, index) {
            let (table, next_index) = parse_table(&lines, index);
            blocks.push(table);
            index = next_index;
            continue;
        }

        if is_bullet(line) {
            let (items, next_index) = parse_list(&lines, index, false);
            blocks.push(DocumentBlock::BulletList(items));
            index = next_index;
            continue;
        }

        if is_ordered(line) {
            let (items, next_index) = parse_list(&lines, index, true);
            blocks.push(DocumentBlock::OrderedList(items));
            index = next_index;
            continue;
        }

        let (paragraph, next_index) = parse_paragraph(&lines, index);
        blocks.push(DocumentBlock::Paragraph(paragraph));
        index = next_index;
    }

    blocks
}

fn parse_code_block(lines: &[&str], start: usize) -> Option<(Option<String>, usize, String)> {
    let first = lines[start].trim_start();
    if !first.starts_with("```") {
        return None;
    }
    let lang = first
        .trim_start_matches("```")
        .trim()
        .is_empty()
        .then_some(None)
        .unwrap_or_else(|| Some(first.trim_start_matches("```").trim().to_string()));
    let mut text = String::new();
    let mut index = start + 1;
    while index < lines.len() {
        if lines[index].trim_start().starts_with("```") {
            return Some((lang, index + 1, text));
        }
        text.push_str(lines[index]);
        text.push('\n');
        index += 1;
    }
    Some((lang, index, text))
}

fn is_table_start(lines: &[&str], index: usize) -> bool {
    index + 1 < lines.len()
        && lines[index].trim_start().starts_with('|')
        && lines[index + 1].contains("---")
}

fn parse_table(lines: &[&str], start: usize) -> (DocumentBlock, usize) {
    let headers = split_table_row(lines[start]);
    let mut rows = Vec::new();
    let mut index = start + 2;
    while index < lines.len() && lines[index].trim_start().starts_with('|') {
        rows.push(split_table_row(lines[index]));
        index += 1;
    }
    (DocumentBlock::Table { headers, rows }, index)
}

fn split_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn is_bullet(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- ") || trimmed.starts_with("* ")
}

fn is_ordered(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(dot_index) = trimmed.find('.') else {
        return false;
    };
    dot_index > 0
        && trimmed[..dot_index]
            .chars()
            .all(|character| character.is_ascii_digit())
        && trimmed[dot_index + 1..].starts_with(' ')
}

fn parse_list(lines: &[&str], start: usize, ordered: bool) -> (Vec<String>, usize) {
    let mut items = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index];
        let matches = if ordered {
            is_ordered(line)
        } else {
            is_bullet(line)
        };
        if !matches {
            break;
        }
        items.push(strip_list_marker(line, ordered));
        index += 1;
    }
    (items, index)
}

fn strip_list_marker(line: &str, ordered: bool) -> String {
    let trimmed = line.trim_start();
    if ordered {
        let dot_index = trimmed.find('.').expect("ordered marker");
        trimmed[dot_index + 1..].trim_start().to_string()
    } else {
        trimmed[2..].to_string()
    }
}

fn parse_paragraph(lines: &[&str], start: usize) -> (String, usize) {
    let mut text = String::new();
    let mut index = start;
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty()
            || line.trim_start().starts_with("```")
            || is_table_start(lines, index)
            || is_bullet(line)
            || is_ordered(line)
        {
            break;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line.trim_end());
        index += 1;
    }
    (text, index)
}

fn render_blocks(blocks: &[DocumentBlock]) -> String {
    if blocks.is_empty() {
        return String::new();
    }

    let mut output = String::from("\n");
    for block in blocks {
        match block {
            DocumentBlock::Paragraph(text) => {
                output.push_str(text);
                output.push_str("\n\n");
            }
            DocumentBlock::BulletList(items) => {
                for item in items {
                    output.push_str("- ");
                    output.push_str(item);
                    output.push('\n');
                }
                output.push('\n');
            }
            DocumentBlock::OrderedList(items) => {
                for (index, item) in items.iter().enumerate() {
                    output.push_str(&(index + 1).to_string());
                    output.push_str(". ");
                    output.push_str(item);
                    output.push('\n');
                }
                output.push('\n');
            }
            DocumentBlock::Table { headers, rows } => {
                output.push_str(&format!("| {} |\n", headers.join(" | ")));
                output.push_str(&format!(
                    "| {} |\n",
                    headers
                        .iter()
                        .map(|_| "---")
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
                for row in rows {
                    output.push_str(&format!("| {} |\n", row.join(" | ")));
                }
                output.push('\n');
            }
            DocumentBlock::CodeBlock { lang, text } => {
                output.push_str("```");
                if let Some(lang) = lang {
                    output.push_str(lang);
                }
                output.push('\n');
                output.push_str(text);
                if !text.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("```\n\n");
            }
        }
    }
    output
}

fn line_start_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(index + 1);
        }
    }
    offsets
}

fn offset_for_line(offsets: &[usize], line_number: u32) -> usize {
    if line_number == 0 {
        return 0;
    }
    offsets
        .get(line_number as usize - 1)
        .copied()
        .unwrap_or_else(|| *offsets.last().unwrap_or(&0))
}

fn line_count(text: &str) -> u32 {
    text.lines().count() as u32
}

#[derive(Debug, Clone, Copy)]
enum PatchMode {
    Add,
    Replace,
}

fn apply_json_value(
    root: &mut Value,
    path: &str,
    next: Value,
    mode: PatchMode,
) -> Result<(), DocumentOpError> {
    let tokens = parse_json_pointer(path)?;
    if tokens.is_empty() {
        *root = next;
        return Ok(());
    }

    let (parent_tokens, key) = tokens.split_at(tokens.len() - 1);
    let parent = descend_json_mut(root, parent_tokens)?;
    let key = &key[0];
    match parent {
        Value::Object(object) => match mode {
            PatchMode::Add => {
                object.insert(key.clone(), next);
                Ok(())
            }
            PatchMode::Replace => {
                if !object.contains_key(key) {
                    return Err(DocumentOpError::PatchConflict(format!(
                        "path {path} does not exist"
                    )));
                }
                object.insert(key.clone(), next);
                Ok(())
            }
        },
        Value::Array(array) => {
            let index = parse_array_index(key, array.len(), matches!(mode, PatchMode::Add), path)?;
            match mode {
                PatchMode::Add => {
                    if index == array.len() {
                        array.push(next);
                    } else {
                        array.insert(index, next);
                    }
                    Ok(())
                }
                PatchMode::Replace => {
                    if index >= array.len() {
                        return Err(DocumentOpError::PatchConflict(format!(
                            "path {path} does not exist"
                        )));
                    }
                    array[index] = next;
                    Ok(())
                }
            }
        }
        _ => Err(DocumentOpError::PatchConflict(format!(
            "path {path} parent is not an object or array"
        ))),
    }
}

fn parse_json_pointer(path: &str) -> Result<Vec<String>, DocumentOpError> {
    if path.is_empty() {
        return Ok(Vec::new());
    }
    if !path.starts_with('/') {
        return Err(DocumentOpError::PatchConflict(format!(
            "json pointer must start with '/': {path}"
        )));
    }
    Ok(path
        .split('/')
        .skip(1)
        .map(|token| token.replace("~1", "/").replace("~0", "~"))
        .collect())
}

fn descend_json_mut<'a>(
    value: &'a mut Value,
    tokens: &[String],
) -> Result<&'a mut Value, DocumentOpError> {
    let mut current = value;
    for token in tokens {
        match current {
            Value::Object(object) => {
                current = object.get_mut(token).ok_or_else(|| {
                    DocumentOpError::PatchConflict(format!("path segment {token} does not exist"))
                })?;
            }
            Value::Array(array) => {
                let index = parse_array_index(token, array.len(), false, token)?;
                current = array.get_mut(index).ok_or_else(|| {
                    DocumentOpError::PatchConflict(format!("array index {token} does not exist"))
                })?;
            }
            _ => {
                return Err(DocumentOpError::PatchConflict(format!(
                    "path segment {token} is not traversable"
                )));
            }
        }
    }
    Ok(current)
}

fn parse_array_index(
    token: &str,
    len: usize,
    allow_append: bool,
    path: &str,
) -> Result<usize, DocumentOpError> {
    if allow_append && token == "-" {
        return Ok(len);
    }
    token.parse::<usize>().map_err(|_| {
        DocumentOpError::PatchConflict(format!("array path segment is not a valid index: {path}"))
    })
}

fn template_for(kind: &DocumentTemplateKind) -> Result<&'static str, DocumentOpError> {
    match kind {
        DocumentTemplateKind::OpenspecProposal => Ok(
            "# 变更提案\n\n## 目标\n\n待补充目标。\n\n## 范围\n\n待补充范围。\n",
        ),
        DocumentTemplateKind::OpenspecSpec => Ok(
            "# 变更规格\n\n## 需求\n\n- [REQ-001] 待补充需求。\n\n## 验收标准\n\n- [AC-001] 待补充验收标准。Refs: REQ-001\n",
        ),
        DocumentTemplateKind::OpenspecDesign => Ok(
            "# 设计\n\n## 设计决策\n\n- [DEC-001] 待补充设计决策。\n\n## 风险\n\n- [RISK-001] 待补充风险。\n",
        ),
        DocumentTemplateKind::OpenspecTasks => Ok(
            "# 任务清单\n\n## 实施任务\n\n- [ ] TASK-001 待补充任务。\n",
        ),
    }
}
