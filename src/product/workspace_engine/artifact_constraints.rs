use crate::product::models::WorkspaceType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactHeadingRule {
    pub(crate) label: &'static str,
    pub(crate) aliases: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactTokenRule {
    pub(crate) label: &'static str,
    pub(crate) pattern: ArtifactTokenPattern,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArtifactTokenPattern {
    BracketPrefix(&'static str),
    WordPrefix(&'static str),
    Literal(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactIdPatternRule {
    pub(crate) label: &'static str,
    pub(crate) pattern: ArtifactTokenPattern,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactConstraintSpec {
    pub(crate) workspace_type: WorkspaceType,
    pub(crate) required_headings: Vec<ArtifactHeadingRule>,
    pub(crate) forbidden_headings: Vec<ArtifactHeadingRule>,
    pub(crate) forbidden_tokens: Vec<ArtifactTokenRule>,
    pub(crate) required_tokens: Vec<ArtifactTokenRule>,
    pub(crate) required_id_patterns: Vec<ArtifactIdPatternRule>,
    pub(crate) reviewer_must_fix_rules: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactValidationReport {
    pub(crate) passed: bool,
    pub(crate) missing_required_headings: Vec<String>,
    pub(crate) forbidden_headings: Vec<String>,
    pub(crate) forbidden_tokens: Vec<String>,
    pub(crate) missing_required_ids: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

pub(crate) fn artifact_constraint_spec_for(
    workspace_type: &WorkspaceType,
) -> ArtifactConstraintSpec {
    match workspace_type {
        WorkspaceType::Story => ArtifactConstraintSpec {
            workspace_type: workspace_type.clone(),
            required_headings: vec![
                heading_rule("范围", &["Scope"]),
                heading_rule("用户故事", &["User Stories", "User Story"]),
                heading_rule("功能需求", &["Functional Requirements"]),
                heading_rule("成功标准", &["Acceptance Criteria", "Success Criteria"]),
                heading_rule("待确认项", &["Open Questions", "open_items"]),
                heading_rule(
                    "非功能需求",
                    &["Non Functional Requirements", "Non-functional Requirements"],
                ),
            ],
            forbidden_headings: vec![
                heading_rule("Work Items", &["Work Item"]),
                heading_rule("任务拆分", &["Task Breakdown"]),
                heading_rule("实施计划", &["Implementation Plan"]),
                heading_rule("执行计划", &["Execution Plan"]),
                heading_rule("开发任务", &["Development Tasks"]),
            ],
            forbidden_tokens: vec![
                token_rule("[TASK-*]", ArtifactTokenPattern::BracketPrefix("TASK-")),
                token_rule("WI-*", ArtifactTokenPattern::WordPrefix("WI-")),
            ],
            required_tokens: vec![token_rule(
                "source id",
                ArtifactTokenPattern::Literal("source id"),
            )],
            required_id_patterns: vec![
                id_rule("[REQ-*]", ArtifactTokenPattern::BracketPrefix("REQ-")),
                id_rule("[AC-*]", ArtifactTokenPattern::BracketPrefix("AC-")),
            ],
            reviewer_must_fix_rules: vec![
                "Story artifact: Work Item heading, task splitting, [TASK-*], or WI-* content must be reported as must_fix.",
            ],
        },
        WorkspaceType::Design => ArtifactConstraintSpec {
            workspace_type: workspace_type.clone(),
            required_headings: vec![
                heading_rule("设计范围", &["Design Scope"]),
                heading_rule("设计决策", &["Design Decisions"]),
                heading_rule("公共组件", &["Shared Components"]),
                heading_rule("API 契约", &["API Contract"]),
                heading_rule("数据模型", &["Data Model", "Data Entities"]),
                heading_rule("风险", &["Risks"]),
                heading_rule("追踪关系", &["Traceability"]),
            ],
            forbidden_headings: vec![
                heading_rule("Work Item Plan", &[]),
                heading_rule("任务拆分", &["Task Breakdown"]),
                heading_rule("开发任务", &["Development Tasks"]),
                heading_rule("执行 checklist", &["Execution Checklist"]),
            ],
            forbidden_tokens: vec![
                token_rule("[TASK-*]", ArtifactTokenPattern::BracketPrefix("TASK-")),
                token_rule("WI-*", ArtifactTokenPattern::WordPrefix("WI-")),
            ],
            required_tokens: vec![token_rule(
                "source id",
                ArtifactTokenPattern::Literal("source id"),
            )],
            required_id_patterns: vec![
                id_rule("[DEC-*]", ArtifactTokenPattern::BracketPrefix("DEC-")),
                id_rule("[CMP-*]", ArtifactTokenPattern::BracketPrefix("CMP-")),
                id_rule("[API-*]", ArtifactTokenPattern::BracketPrefix("API-")),
            ],
            reviewer_must_fix_rules: vec![
                "Design artifact: Work Item Plan, development task list, task splitting, or execution checklist content must be reported as must_fix.",
            ],
        },
        WorkspaceType::WorkItem => ArtifactConstraintSpec {
            workspace_type: workspace_type.clone(),
            required_headings: vec![
                heading_rule("目标", &["Goal"]),
                heading_rule("范围", &["Scope"]),
                heading_rule("实现步骤", &["子步骤", "Implementation Steps"]),
                heading_rule("依赖", &["Dependencies"]),
                heading_rule("验证命令", &["Verification Commands"]),
                heading_rule("风险", &["Risks"]),
                heading_rule("追踪关系", &["Traceability"]),
            ],
            forbidden_headings: vec![
                heading_rule("兄弟任务", &["Sibling Tasks"]),
                heading_rule("任务拆分", &["Task Breakdown"]),
                heading_rule("整体实施计划", &["Overall Implementation Plan"]),
                heading_rule("Issue 级计划", &["Issue Plan"]),
                heading_rule("Work Items", &[]),
            ],
            forbidden_tokens: vec![token_rule(
                "跨任务 WI-* 章节",
                ArtifactTokenPattern::WordPrefix("WI-"),
            )],
            required_tokens: vec![token_rule(
                "source id",
                ArtifactTokenPattern::Literal("source id"),
            )],
            required_id_patterns: Vec::new(),
            reviewer_must_fix_rules: vec![
                "Work Item artifact: sibling tasks, issue-level full plans, or cross-task content must be reported as must_fix.",
            ],
        },
        WorkspaceType::WorkItemPlan => ArtifactConstraintSpec {
            workspace_type: workspace_type.clone(),
            required_headings: vec![
                heading_rule("计划范围", &["Plan Scope"]),
                heading_rule("任务拆分", &["Task Breakdown"]),
                heading_rule("依赖图", &["Dependency Graph"]),
                heading_rule("验证计划", &["Verification Plan"]),
                heading_rule("执行顺序", &["Execution Order"]),
                heading_rule("风险", &["Risks"]),
                heading_rule("追踪关系", &["Traceability"]),
            ],
            forbidden_headings: vec![
                heading_rule("代码实现", &["Implementation Code"]),
                heading_rule("完整 Story Spec", &["Full Story Spec"]),
                heading_rule("完整 Design Spec", &["Full Design Spec"]),
            ],
            forbidden_tokens: Vec::new(),
            required_tokens: vec![token_rule(
                "source id",
                ArtifactTokenPattern::Literal("source id"),
            )],
            required_id_patterns: vec![id_rule(
                "[TASK-*]",
                ArtifactTokenPattern::BracketPrefix("TASK-"),
            )],
            reviewer_must_fix_rules: vec![
                "Work Item Plan artifact: code implementation or rewritten full Story/Design content must be reported as must_fix.",
            ],
        },
    }
}

pub(crate) fn validate_workspace_artifact_constraints(
    content: &str,
    workspace_type: &WorkspaceType,
) -> ArtifactValidationReport {
    let spec = artifact_constraint_spec_for(workspace_type);
    let headings = content
        .lines()
        .filter_map(super::normalize_workspace_heading_line)
        .collect::<Vec<_>>();
    let searchable_content = content_without_code_fences_and_traceability(content);

    let missing_required_headings = spec
        .required_headings
        .iter()
        .filter(|rule| {
            !headings
                .iter()
                .any(|heading| heading_matches_rule(heading, rule))
        })
        .map(|rule| rule.label.to_string())
        .collect::<Vec<_>>();
    let forbidden_headings = headings
        .iter()
        .filter_map(|heading| {
            spec.forbidden_headings
                .iter()
                .find(|rule| heading_matches_rule(heading, rule))
                .map(|_| heading.clone())
        })
        .collect::<Vec<_>>();
    let mut forbidden_tokens = spec
        .forbidden_tokens
        .iter()
        .filter_map(|rule| {
            token_pattern_match(&searchable_content, &rule.pattern)
                .map(|token| format!("{}: {token}", rule.label))
        })
        .collect::<Vec<_>>();
    if matches!(workspace_type, WorkspaceType::WorkItem) {
        let sibling_task_tokens = find_bracket_prefixed_tokens(&searchable_content, "TASK-");
        if sibling_task_tokens.len() > 1 {
            forbidden_tokens.push(format!(
                "多个兄弟 [TASK-*]: {}",
                sibling_task_tokens.join(", ")
            ));
        }
    }
    let mut missing_required_ids = spec
        .required_id_patterns
        .iter()
        .filter(|rule| token_pattern_match(content, &rule.pattern).is_none())
        .map(|rule| rule.label.to_string())
        .collect::<Vec<_>>();
    missing_required_ids.extend(
        spec.required_tokens
            .iter()
            .filter(|rule| token_pattern_match(content, &rule.pattern).is_none())
            .map(|rule| rule.label.to_string()),
    );

    let passed = missing_required_headings.is_empty()
        && forbidden_headings.is_empty()
        && forbidden_tokens.is_empty()
        && missing_required_ids.is_empty();

    ArtifactValidationReport {
        passed,
        missing_required_headings,
        forbidden_headings,
        forbidden_tokens,
        missing_required_ids,
        warnings: Vec::new(),
    }
}

pub(crate) fn reviewer_boundary_rules_for(workspace_type: &WorkspaceType) -> String {
    let spec = artifact_constraint_spec_for(workspace_type);
    let mut output = String::from("\n[artifact_boundary_must_fix_rules]\n");
    for rule in spec.reviewer_must_fix_rules {
        output.push_str("- ");
        output.push_str(rule);
        output.push('\n');
    }
    output
}

pub(crate) fn allowed_outputs_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "用户故事、功能需求、成功标准、非功能需求、待确认项",
        WorkspaceType::Design => "架构、数据流、接口、风险、技术约束、验证策略",
        WorkspaceType::WorkItemPlan => "多任务拆解、任务追踪关系、依赖图、验收与验证建议",
        WorkspaceType::WorkItem => "单个可执行任务的目标、范围、实现步骤、验收、验证与追踪关系",
    }
}

pub(crate) fn forbidden_outputs_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "Work Item、任务拆分、执行计划、实现步骤",
        WorkspaceType::Design => "Work Item Plan、开发任务列表、执行 checklist",
        WorkspaceType::WorkItemPlan => "代码实现、Story/Design 重写",
        WorkspaceType::WorkItem => "兄弟任务、Issue 级完整计划、其它任务的交叉内容",
    }
}

fn heading_rule(label: &'static str, aliases: &'static [&'static str]) -> ArtifactHeadingRule {
    ArtifactHeadingRule { label, aliases }
}

fn token_rule(label: &'static str, pattern: ArtifactTokenPattern) -> ArtifactTokenRule {
    ArtifactTokenRule { label, pattern }
}

fn id_rule(label: &'static str, pattern: ArtifactTokenPattern) -> ArtifactIdPatternRule {
    ArtifactIdPatternRule { label, pattern }
}

fn heading_matches_rule(heading: &str, rule: &ArtifactHeadingRule) -> bool {
    heading.eq_ignore_ascii_case(rule.label)
        || rule
            .aliases
            .iter()
            .any(|alias| heading.eq_ignore_ascii_case(alias))
}

fn content_without_code_fences_and_traceability(content: &str) -> String {
    let mut output = String::new();
    let mut in_code_fence = false;
    let mut in_traceability_section = false;

    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("````") {
            in_code_fence = !in_code_fence;
            continue;
        }
        if in_code_fence {
            continue;
        }
        if let Some(heading) = super::normalize_workspace_heading_line(line) {
            in_traceability_section = heading.eq_ignore_ascii_case("追踪关系")
                || heading.eq_ignore_ascii_case("Traceability");
        }
        if in_traceability_section {
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }

    output
}

fn token_pattern_match(content: &str, pattern: &ArtifactTokenPattern) -> Option<String> {
    match pattern {
        ArtifactTokenPattern::BracketPrefix(prefix) => find_bracket_prefixed_token(content, prefix),
        ArtifactTokenPattern::WordPrefix(prefix) => find_word_prefixed_token(content, prefix),
        ArtifactTokenPattern::Literal(literal) => find_literal_token(content, literal),
    }
}

fn find_bracket_prefixed_token(content: &str, prefix: &str) -> Option<String> {
    find_bracket_prefixed_tokens(content, prefix)
        .into_iter()
        .next()
}

fn find_bracket_prefixed_tokens(content: &str, prefix: &str) -> Vec<String> {
    let mut matches = Vec::new();
    for token in content.split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';') {
        let trimmed = token.trim_matches(|ch: char| {
            matches!(ch, '-' | '*' | ')' | '(' | '。' | '，' | ':' | '：')
        });
        if trimmed.starts_with('[')
            && trimmed
                .get(1..)
                .is_some_and(|value| value.starts_with(prefix))
            && let Some(end) = trimmed.find(']')
        {
            let token = trimmed[..=end].to_string();
            if !matches.contains(&token) {
                matches.push(token);
            }
        }
    }
    matches
}

fn find_word_prefixed_token(content: &str, prefix: &str) -> Option<String> {
    for token in content.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_') {
        if token.starts_with(prefix) && token.len() > prefix.len() {
            return Some(token.to_string());
        }
    }
    None
}

fn find_literal_token(content: &str, literal: &str) -> Option<String> {
    content
        .to_ascii_lowercase()
        .contains(&literal.to_ascii_lowercase())
        .then(|| literal.to_string())
}
