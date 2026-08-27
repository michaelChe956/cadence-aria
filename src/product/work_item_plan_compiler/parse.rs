//! `work-item-plan.md` 的 fail-closed source linter 与最小 AST parser。
//!
//! linter 只处理 grammar 层的 source 形状：结构化 heading/key、必填 section/field、
//! identifier、dependency 与 EARS。自由文本 section 保留原样，不把其中的冒号或表格
//! 解释为结构化 token；typed lowering 由后续阶段负责。

use std::collections::{HashMap, HashSet};

use super::{
    grammar,
    types::{
        CompilerDiagnostic, Spanned, WorkItemPlanAst, WorkItemPlanFieldAst, WorkItemPlanItemAst,
        WorkItemPlanSectionAst,
    },
};

const MISSING_SECTION_CODE: &str = grammar::DIAGNOSTIC_CODES[0];
const UNKNOWN_STRUCTURED_KEY_CODE: &str = grammar::DIAGNOSTIC_CODES[1];
const INVALID_WORK_ITEM_ID_CODE: &str = grammar::DIAGNOSTIC_CODES[2];
const INVALID_EARS_CODE: &str = grammar::DIAGNOSTIC_CODES[3];

pub(super) const REQUIRED_FIELDS: [(&str, &[&str]); 13] = [
    (
        "Identity",
        &["schema_version", "logical_work_item_id", "title", "kind"],
    ),
    ("Goal", &["summary"]),
    ("Non Goals", &["non_goals"]),
    ("Dependencies", &["depends_on"]),
    ("Inputs", &[]),
    ("Outputs", &["contract_id", "capabilities"]),
    (
        "Tasks",
        &["task_id", "statement", "requirement_refs", "done_when_refs"],
    ),
    ("Write Policy", &["exclusive_scopes", "forbidden_scopes"]),
    (
        "Acceptance Criteria",
        &["criterion_id", "statement", "required_evidence"],
    ),
    (
        "Verification",
        &[
            "check_id",
            "command",
            "manual_instruction",
            "required",
            "non_zero_test_execution_required",
        ],
    ),
    (
        "Handoff Schema",
        &[
            "required_fields",
            "provided_contract_refs",
            "reviewer_check_refs",
        ],
    ),
    (
        "Blockers",
        &["reason_code", "route", "target_contract_refs"],
    ),
    (
        "Traceability",
        &["source_type", "source_id", "requirement_id"],
    ),
];

#[derive(Debug)]
struct ParsedField {
    section: String,
    key: String,
    value: String,
    line: usize,
}

#[derive(Debug)]
struct ParsedItem {
    id: String,
    title: String,
    heading_line: usize,
    section_lines: HashMap<String, usize>,
    fields: Vec<ParsedField>,
}

impl ParsedItem {
    fn new(id: String, title: String, heading_line: usize) -> Self {
        Self {
            id,
            title,
            heading_line,
            section_lines: HashMap::new(),
            fields: Vec::new(),
        }
    }
}

/// 对 markdown source 执行 grammar 层失败关闭检查。
pub fn lint_work_item_plan_source(source: &str) -> Vec<CompilerDiagnostic> {
    let (_, diagnostics) = parse_source(source);
    diagnostics
}

/// 在 grammar 通过时构造最小 AST；失败时返回按稳定顺序排列的 diagnostics。
pub fn parse_work_item_plan(source: &str) -> Result<WorkItemPlanAst, Vec<CompilerDiagnostic>> {
    let (ast, diagnostics) = parse_source(source);
    if diagnostics.is_empty() {
        Ok(ast)
    } else {
        Err(diagnostics)
    }
}

fn parse_source(source: &str) -> (WorkItemPlanAst, Vec<CompilerDiagnostic>) {
    let mut diagnostics = Vec::new();
    let mut items = Vec::new();
    let mut notes = Vec::new();
    let mut rationale = Vec::new();
    let mut current_item = None;
    let mut current_section: Option<String> = None;
    let mut document_heading_seen = false;

    for (index, raw_line) in source.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw_line.trim();

        if raw_line == grammar::DOCUMENT_HEADING {
            document_heading_seen = true;
            current_section = None;
            continue;
        }

        if raw_line.starts_with("## ") {
            current_section = None;
            if let Some((id, title)) = parse_work_item_heading(raw_line) {
                items.push(ParsedItem::new(id, title, line));
                current_item = Some(items.len() - 1);
            } else {
                diagnostics.push(diagnostic(
                    UNKNOWN_STRUCTURED_KEY_CODE,
                    line,
                    "work_item_heading",
                    "二级 heading 必须是 Work Item heading。",
                    "## Work Item WI-001: Levels API fixture",
                ));
                current_item = None;
            }
            continue;
        }

        if let Some(section) = raw_line.strip_prefix("### ") {
            let section = section.trim();
            current_section = Some(section.to_string());
            if grammar::STRUCTURED_SECTIONS.contains(&section) {
                if let Some(item_index) = current_item {
                    let item = &mut items[item_index];
                    if item.section_lines.contains_key(section) {
                        diagnostics.push(diagnostic(
                            UNKNOWN_STRUCTURED_KEY_CODE,
                            line,
                            section,
                            "结构化 section 不得重复。",
                            &format!("### {section}"),
                        ));
                    } else {
                        item.section_lines.insert(section.to_string(), line);
                    }
                } else {
                    diagnostics.push(diagnostic(
                        UNKNOWN_STRUCTURED_KEY_CODE,
                        line,
                        section,
                        "结构化 section 必须属于一个 Work Item。",
                        "## Work Item WI-001: Levels API fixture",
                    ));
                }
            } else if grammar::FREE_TEXT_SECTIONS.contains(&section) {
                if current_item.is_none() {
                    diagnostics.push(diagnostic(
                        UNKNOWN_STRUCTURED_KEY_CODE,
                        line,
                        section,
                        "自由文本 section 必须位于 Work Item 之后。",
                        "## Work Item WI-001: Levels API fixture",
                    ));
                }
            } else {
                diagnostics.push(diagnostic(
                    UNKNOWN_STRUCTURED_KEY_CODE,
                    line,
                    section,
                    "未知结构化 section 必须拒绝。",
                    "### Goal",
                ));
            }
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        match current_section.as_deref() {
            Some(section) if grammar::STRUCTURED_SECTIONS.contains(&section) => {
                let Some(item_index) = current_item else {
                    continue;
                };
                let Some((key, value)) = raw_line
                    .strip_prefix("- ")
                    .and_then(|entry| entry.split_once(grammar::KEY_VALUE_SEPARATOR))
                else {
                    diagnostics.push(diagnostic(
                        UNKNOWN_STRUCTURED_KEY_CODE,
                        line,
                        "structured_line",
                        "结构化 section 中每行都必须是 key/value 形式。",
                        grammar::STRUCTURED_LINE_PREFIX,
                    ));
                    continue;
                };
                let key = key.trim();
                let value = value.trim();
                if !grammar::STRUCTURED_KEYS.contains(&key) {
                    diagnostics.push(diagnostic(
                        UNKNOWN_STRUCTURED_KEY_CODE,
                        line,
                        key,
                        "未知结构化 key 必须拒绝。",
                        "- kind: backend",
                    ));
                    continue;
                }
                let item = &mut items[item_index];
                item.fields.push(ParsedField {
                    section: section.to_string(),
                    key: key.to_string(),
                    value: value.to_string(),
                    line,
                });
            }
            Some(section) if section == grammar::FREE_TEXT_SECTIONS[0] => {
                notes.push(Spanned {
                    value: raw_line.to_string(),
                    line,
                });
            }
            Some(section) if section == grammar::FREE_TEXT_SECTIONS[1] => {
                rationale.push(Spanned {
                    value: raw_line.to_string(),
                    line,
                });
            }
            _ => diagnostics.push(diagnostic(
                UNKNOWN_STRUCTURED_KEY_CODE,
                line,
                "document_content",
                "非空内容必须位于合法 section 中。",
                grammar::DOCUMENT_HEADING,
            )),
        }
    }

    if !document_heading_seen {
        diagnostics.push(diagnostic(
            MISSING_SECTION_CODE,
            1,
            "document_heading",
            "文档必须包含固定一级标题。",
            grammar::DOCUMENT_HEADING,
        ));
    }
    if items.is_empty() {
        diagnostics.push(diagnostic(
            MISSING_SECTION_CODE,
            1,
            "work_item",
            "文档必须至少包含一个 Work Item。",
            "## Work Item WI-001: Levels API fixture",
        ));
    }

    validate_required_parts(&items, &mut diagnostics);
    validate_identifiers(&items, &mut diagnostics);
    validate_ears(&items, &mut diagnostics);
    validate_dependencies(&items, &mut diagnostics);
    sort_diagnostics(&mut diagnostics);

    (
        WorkItemPlanAst {
            items: items.into_iter().map(ast_item).collect(),
            notes,
            rationale,
        },
        diagnostics,
    )
}

fn ast_item(item: ParsedItem) -> WorkItemPlanItemAst {
    let mut section_entries = item.section_lines.iter().collect::<Vec<_>>();
    section_entries.sort_by_key(|(_, line)| **line);
    let sections = section_entries
        .into_iter()
        .map(|(section, line)| WorkItemPlanSectionAst {
            name: Spanned {
                value: section.clone(),
                line: *line,
            },
            fields: item
                .fields
                .iter()
                .filter(|field| field.section == *section)
                .map(|field| WorkItemPlanFieldAst {
                    key: Spanned {
                        value: field.key.clone(),
                        line: field.line,
                    },
                    value: Spanned {
                        value: field.value.clone(),
                        line: field.line,
                    },
                })
                .collect(),
        })
        .collect();

    WorkItemPlanItemAst {
        id: Spanned {
            value: item.id,
            line: item.heading_line,
        },
        title: Spanned {
            value: item.title,
            line: item.heading_line,
        },
        sections,
    }
}

fn parse_work_item_heading(heading: &str) -> Option<(String, String)> {
    let remainder = heading.strip_prefix(grammar::ITEM_HEADING_PREFIX)?;
    let (id_suffix, title) = remainder.split_once(": ")?;
    let title = title.trim();
    (!title.is_empty()).then(|| {
        (
            format!("{}{}", grammar::ITEM_ID_PREFIX, id_suffix),
            title.to_string(),
        )
    })
}

fn validate_required_parts(items: &[ParsedItem], diagnostics: &mut Vec<CompilerDiagnostic>) {
    for item in items {
        for (section_index, section) in grammar::STRUCTURED_SECTIONS.iter().enumerate() {
            if !item.section_lines.contains_key(*section) {
                let line = grammar::STRUCTURED_SECTIONS[section_index + 1..]
                    .iter()
                    .find_map(|next| item.section_lines.get(*next).copied())
                    .unwrap_or(item.heading_line);
                diagnostics.push(diagnostic(
                    MISSING_SECTION_CODE,
                    line,
                    section,
                    "Work Item 缺少必需 section。",
                    &format!("### {section}"),
                ));
                continue;
            }

            let required_fields = REQUIRED_FIELDS
                .iter()
                .find_map(|(required_section, fields)| {
                    (*required_section == *section).then_some(*fields)
                })
                .expect("每个结构化 section 都必须有 required-field 定义");
            for field in required_fields {
                if !item.fields.iter().any(|entry| {
                    entry.section == *section && entry.key == *field && !entry.value.is_empty()
                }) {
                    let line = *item
                        .section_lines
                        .get(*section)
                        .expect("已确认 section 存在");
                    diagnostics.push(diagnostic(
                        MISSING_SECTION_CODE,
                        line,
                        field,
                        "结构化 section 缺少必需字段。",
                        &format!("- {field}: value"),
                    ));
                }
            }
        }
    }
}

fn validate_identifiers(items: &[ParsedItem], diagnostics: &mut Vec<CompilerDiagnostic>) {
    let mut work_item_ids = HashSet::new();
    let mut task_ids = HashSet::new();
    let mut criterion_ids = HashSet::new();
    let mut check_ids = HashSet::new();

    for item in items {
        validate_identifier(
            &item.id,
            item.heading_line,
            "work_item_id",
            grammar::ITEM_ID_PREFIX,
            "## Work Item WI-001: Levels API fixture",
            &mut work_item_ids,
            diagnostics,
        );
        for field in &item.fields {
            let (prefix, repair_example, seen) = match field.key.as_str() {
                "task_id" => ("TASK-", "- task_id: TASK-001", &mut task_ids),
                "criterion_id" => ("AC-", "- criterion_id: AC-001", &mut criterion_ids),
                "check_id" => ("CHECK-", "- check_id: CHECK-001", &mut check_ids),
                _ => continue,
            };
            validate_identifier(
                &field.value,
                field.line,
                &field.key,
                prefix,
                repair_example,
                seen,
                diagnostics,
            );
        }
    }
}

fn validate_identifier(
    value: &str,
    line: usize,
    field: &str,
    prefix: &str,
    repair_example: &str,
    seen: &mut HashSet<String>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    if !is_valid_identifier(value, prefix) {
        diagnostics.push(diagnostic(
            INVALID_WORK_ITEM_ID_CODE,
            line,
            field,
            "标识符必须使用固定前缀并以数字结尾。",
            repair_example,
        ));
    } else if !seen.insert(value.to_string()) {
        diagnostics.push(diagnostic(
            INVALID_WORK_ITEM_ID_CODE,
            line,
            field,
            "标识符不得重复。",
            repair_example,
        ));
    }
}

fn is_valid_identifier(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn validate_ears(items: &[ParsedItem], diagnostics: &mut Vec<CompilerDiagnostic>) {
    for item in items {
        for field in &item.fields {
            if field.key == "statement" && !is_ears_statement(&field.value) {
                diagnostics.push(diagnostic(
                    INVALID_EARS_CODE,
                    field.line,
                    "statement",
                    "statement 必须符合 EARS 模板。",
                    "- statement: WHEN the selector loads THE SYSTEM SHALL render returned options.",
                ));
            }
        }
    }
}

fn is_ears_statement(statement: &str) -> bool {
    statement
        .strip_prefix(grammar::EARS_WHEN_PREFIX)
        .and_then(|rest| rest.split_once(grammar::EARS_SHALL_PREFIX))
        .is_some_and(|(condition, outcome)| {
            !condition.trim().is_empty() && !outcome.trim().is_empty()
        })
}

fn validate_dependencies(items: &[ParsedItem], diagnostics: &mut Vec<CompilerDiagnostic>) {
    let known_ids = items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    let mut graph = HashMap::<&str, Vec<(&str, usize)>>::new();

    for item in items {
        for field in item
            .fields
            .iter()
            .filter(|field| field.section == "Dependencies" && field.key == "depends_on")
        {
            let dependency = field.value.as_str();
            if dependency == "[]" {
                continue;
            }
            if !is_valid_identifier(dependency, grammar::ITEM_ID_PREFIX) {
                diagnostics.push(diagnostic(
                    INVALID_WORK_ITEM_ID_CODE,
                    field.line,
                    "depends_on",
                    "依赖必须引用 WI-<digits> 或 []。",
                    "- depends_on: WI-001",
                ));
                continue;
            }
            if dependency == item.id {
                diagnostics.push(diagnostic(
                    INVALID_WORK_ITEM_ID_CODE,
                    field.line,
                    "depends_on",
                    "Work Item 不得依赖自身。",
                    "- depends_on: []",
                ));
                continue;
            }
            if !known_ids.contains(dependency) {
                diagnostics.push(diagnostic(
                    INVALID_WORK_ITEM_ID_CODE,
                    field.line,
                    "depends_on",
                    "依赖的 Work Item 必须存在。",
                    "- depends_on: WI-001",
                ));
                continue;
            }
            graph
                .entry(item.id.as_str())
                .or_default()
                .push((dependency, field.line));
        }
    }

    let mut state = HashMap::new();
    let mut cycle_lines = HashSet::new();
    for item in items {
        find_dependency_cycles(item.id.as_str(), &graph, &mut state, &mut cycle_lines);
    }
    for line in cycle_lines {
        diagnostics.push(diagnostic(
            INVALID_WORK_ITEM_ID_CODE,
            line,
            "depends_on",
            "Work Item dependency 不得形成循环。",
            "- depends_on: []",
        ));
    }
}

fn find_dependency_cycles<'a>(
    node: &'a str,
    graph: &HashMap<&'a str, Vec<(&'a str, usize)>>,
    state: &mut HashMap<&'a str, u8>,
    cycle_lines: &mut HashSet<usize>,
) {
    match state.get(node).copied().unwrap_or_default() {
        1 | 2 => return,
        _ => {}
    }
    state.insert(node, 1);
    if let Some(edges) = graph.get(node) {
        for (next, line) in edges {
            match state.get(next).copied().unwrap_or_default() {
                1 => {
                    cycle_lines.insert(*line);
                }
                2 => {}
                _ => find_dependency_cycles(next, graph, state, cycle_lines),
            }
        }
    }
    state.insert(node, 2);
}

fn diagnostic(
    code: &str,
    line: usize,
    field: &str,
    message: &str,
    repair_example: &str,
) -> CompilerDiagnostic {
    CompilerDiagnostic {
        code: code.to_string(),
        line,
        field: field.to_string(),
        message: message.to_string(),
        repair_example: repair_example.to_string(),
    }
}

fn sort_diagnostics(diagnostics: &mut [CompilerDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        (left.line, left.field.as_str(), left.code.as_str()).cmp(&(
            right.line,
            right.field.as_str(),
            right.code.as_str(),
        ))
    });
}
