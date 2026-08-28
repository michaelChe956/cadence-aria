use super::{
    WorkItemPlanSourceContext, grammar, types,
    {compile_work_item_plan, lint_work_item_plan_source, parse_work_item_plan},
};
use crate::product::models::TrustedDraftVerificationCommand;
use serde_json::Value;

mod reviewer_finding_channel_boundary;

fn assert_ast_traits<T: std::fmt::Debug + Clone + PartialEq + Eq>() {}

#[test]
fn grammar_contract() {
    assert_ast_traits::<types::WorkItemPlanAst>();
    assert_ast_traits::<types::WorkItemPlanItemAst>();
    assert_ast_traits::<types::CompilerDiagnostic>();
    assert_eq!(
        grammar::DOCUMENT_HEADING,
        "# Work Item Plan",
        "文档标题必须稳定"
    );
    assert_eq!(
        grammar::ITEM_HEADING_PREFIX,
        "## Work Item WI-",
        "Work Item heading 前缀必须稳定"
    );
    assert_eq!(
        grammar::EARS_STATEMENT_TEMPLATE,
        "WHEN <condition> THE SYSTEM SHALL <observable outcome>",
        "EARS 模板必须稳定"
    );
    assert_eq!(
        grammar::STRUCTURED_SECTIONS,
        [
            "Identity",
            "Goal",
            "Non Goals",
            "Dependencies",
            "Inputs",
            "Outputs",
            "Tasks",
            "Write Policy",
            "Acceptance Criteria",
            "Verification",
            "Handoff Schema",
            "Blockers",
            "Traceability",
        ],
        "结构化 section 集合及顺序必须稳定"
    );
    assert_eq!(
        grammar::FREE_TEXT_SECTIONS,
        ["Notes", "Rationale"],
        "仅 Notes/Rationale 允许自由文本"
    );
    assert_eq!(
        grammar::STRUCTURED_KEYS,
        [
            "schema_version",
            "logical_work_item_id",
            "title",
            "kind",
            "summary",
            "non_goals",
            "depends_on",
            "contract_id",
            "provider_logical_work_item_id",
            "required_capabilities",
            "compatibility_policy",
            "capabilities",
            "task_id",
            "statement",
            "requirement_refs",
            "done_when_refs",
            "exclusive_scopes",
            "forbidden_scopes",
            "criterion_id",
            "required_evidence",
            "check_id",
            "command",
            "manual_instruction",
            "required",
            "non_zero_test_execution_required",
            "required_fields",
            "provided_contract_refs",
            "reviewer_check_refs",
            "reason_code",
            "route",
            "target_contract_refs",
            "source_type",
            "source_id",
            "requirement_id",
        ],
        "结构化 key 集合必须覆盖矩阵中的 markdown 字段"
    );
    assert_eq!(
        grammar::ALLOWED_COMPATIBILITY_POLICIES,
        ["require_all", "require_any"]
    );
    assert_eq!(
        grammar::ALLOWED_EVIDENCE_KINDS,
        [
            "source_diff",
            "non_zero_test_execution",
            "manual_check",
            "handoff_field",
        ]
    );
    assert_eq!(
        grammar::ALLOWED_BLOCKER_ROUTES,
        [
            "coder_rework",
            "verification_retry",
            "plan_repair_current",
            "plan_repair_upstream",
            "subgraph_replan",
            "story_amendment",
            "design_amendment",
            "operational_gate",
        ]
    );
    assert_eq!(
        grammar::DIAGNOSTIC_CODES,
        [
            "missing_section",
            "unknown_structured_key",
            "invalid_work_item_id",
            "invalid_ears",
        ],
        "初始 diagnostic 词汇表必须只覆盖 Task 1.4 的 grammar 失败"
    );
    assert_eq!(
        grammar::WORK_ITEM_PLAN_COMPILER_VERSION,
        "work_item_plan_compiler/v1"
    );
    assert_eq!(grammar::KEY_VALUE_SEPARATOR, ": ");
    assert_eq!(grammar::IDENTIFIED_LINE_SEPARATOR, " | ");
    assert_eq!(grammar::UNKNOWN_STRUCTURED_KEY_POLICY, "fail_closed");
    assert_eq!(grammar::FREE_TEXT_SECTION_POLICY, "allow_free_text");
    assert_eq!(grammar::EARS_KEYWORDS, ["WHEN", "THE SYSTEM SHALL"]);

    let ast = types::WorkItemPlanAst {
        items: vec![types::WorkItemPlanItemAst {
            id: types::Spanned {
                value: "WI-001".to_string(),
                line: 2,
            },
            title: types::Spanned {
                value: "fixture".to_string(),
                line: 2,
            },
            sections: vec![types::WorkItemPlanSectionAst {
                name: types::Spanned {
                    value: "Identity".to_string(),
                    line: 4,
                },
                fields: vec![types::WorkItemPlanFieldAst {
                    key: types::Spanned {
                        value: "kind".to_string(),
                        line: 5,
                    },
                    value: types::Spanned {
                        value: "backend".to_string(),
                        line: 5,
                    },
                }],
            }],
        }],
        notes: vec![types::Spanned {
            value: "note".to_string(),
            line: 7,
        }],
        rationale: vec![types::Spanned {
            value: "rationale".to_string(),
            line: 8,
        }],
    };
    let cloned_ast = ast.clone();
    assert_eq!(ast, cloned_ast);
    assert_eq!(ast.items[0].id.value, "WI-001");
    assert_eq!(ast.items[0].id.line, 2);
    assert_eq!(ast.items[0].sections[0].fields[0].key.value, "kind");
    assert_eq!(ast.notes[0].value, "note");
    assert_eq!(ast.rationale[0].value, "rationale");

    let diagnostic = types::CompilerDiagnostic {
        code: "missing_section".to_string(),
        line: 1,
        field: "".to_string(),
        message: "message".to_string(),
        repair_example: "example".to_string(),
    };
    assert_eq!(diagnostic.code, "missing_section");
    assert_eq!(diagnostic.line, 1);
    assert_eq!(diagnostic.field, "");
    assert_eq!(diagnostic.message, "message");
    let cloned_diagnostic = diagnostic.clone();
    assert_eq!(diagnostic, cloned_diagnostic);
    assert_eq!(diagnostic.repair_example, "example");
}

#[test]
fn required_fields_define_every_structured_section() {
    assert!(
        grammar::STRUCTURED_SECTIONS.iter().all(|section| {
            super::parse::REQUIRED_FIELDS
                .iter()
                .any(|(defined_section, _)| defined_section == section)
        }),
        "每个 STRUCTURED_SECTIONS 项都必须有 REQUIRED_FIELDS 定义"
    );
    assert_eq!(
        super::parse::REQUIRED_FIELDS.len(),
        grammar::STRUCTURED_SECTIONS.len(),
        "REQUIRED_FIELDS 不得包含未声明的结构化 section"
    );
}

const FIELD_SOURCE_MATRIX: &str =
    include_str!("../../../openspec/changes/rearch-workitem-plan-pipeline/field-source-matrix.md");

const REQUIRED_FIELD_PATHS: &[&str] = &[
    "contract.schema_version",
    "contract.identity.logical_work_item_id",
    "contract.identity.title",
    "contract.identity.kind",
    "contract.goal.summary",
    "contract.non_goals[]",
    "contract.depends_on[]",
    "contract.input_contracts[].contract_id",
    "contract.input_contracts[].provider_logical_work_item_id",
    "contract.input_contracts[].required_capabilities[]",
    "contract.input_contracts[].compatibility_policy",
    "contract.output_contracts[].contract_id",
    "contract.output_contracts[].capabilities[]",
    "contract.tasks[].task_id",
    "contract.tasks[].statement",
    "contract.tasks[].requirement_refs[]",
    "contract.tasks[].done_when_refs[]",
    "contract.write_policy.exclusive_scopes[]",
    "contract.write_policy.forbidden_scopes[]",
    "contract.acceptance_criteria[].criterion_id",
    "contract.acceptance_criteria[].statement",
    "contract.acceptance_criteria[].required_evidence[]",
    "contract.verification_checks[].check_id",
    "contract.verification_checks[].command",
    "contract.verification_checks[].manual_instruction",
    "contract.verification_checks[].required",
    "contract.verification_checks[].non_zero_test_execution_required",
    "contract.handoff_contract.required_fields[]",
    "contract.handoff_contract.provided_contract_refs[]",
    "contract.handoff_contract.reviewer_check_refs[]",
    "contract.blocker_rules[].reason_code",
    "contract.blocker_rules[].route",
    "contract.blocker_rules[].target_contract_refs[]",
    "contract.design_traceability[].source_type",
    "contract.design_traceability[].source_id",
    "contract.design_traceability[].requirement_id",
    "verification_plan.checks[]",
    "trusted_commands[].command",
    "trusted_commands[].cwd",
    "trusted_commands[].purpose",
    "trusted_commands[].source_ref",
    "target_repository_id",
    "ir.source_revision_hash",
    "ir.compiler_version",
    "publication_provenance.id",
    "publication_provenance.plan_id",
    "publication_provenance.plan_revision_id",
    "publication_provenance.source_revision_ref",
    "publication_provenance.plan_candidate_ir_ref",
    "publication_provenance.mechanical_report_ref",
    "publication_provenance.source_revision_hash",
    "publication_provenance.compiler_version",
    "publication_provenance.published_at",
    "publication_provenance.content_hash",
    "compile_id",
    "now",
];

fn matrix_rows() -> Vec<Vec<&'static str>> {
    FIELD_SOURCE_MATRIX
        .lines()
        .filter(|line| line.starts_with('|'))
        .skip(2)
        .map(|line| line.trim_matches('|').split('|').map(str::trim).collect())
        .collect()
}

#[test]
fn field_source_matrix_covers_each_required_field_path_exactly_once() {
    let rows = matrix_rows();

    for field_path in REQUIRED_FIELD_PATHS {
        let matches = rows
            .iter()
            .filter(|columns| columns.first() == Some(field_path))
            .count();
        assert_eq!(matches, 1, "矩阵必须恰好包含一次字段路径 {field_path}");
    }

    assert_eq!(
        rows.len(),
        REQUIRED_FIELD_PATHS.len(),
        "矩阵不得包含未受本任务约束的字段路径"
    );
}

#[test]
fn field_source_matrix_uses_only_the_four_allowed_sources() {
    let allowed_sources = [
        "markdown",
        "session_confirmed_context",
        "compiler_derived",
        "compile_runtime",
    ];

    for columns in matrix_rows() {
        assert_eq!(columns.len(), 6, "每一行必须固定六列：{columns:?}");
        assert!(
            allowed_sources.contains(&columns[1]),
            "字段 {} 的 source 必须是四种允许值之一，实际为 {}",
            columns[0],
            columns[1]
        );
        assert!(
            !columns[2].is_empty()
                && !columns[3].is_empty()
                && !columns[4].is_empty()
                && !columns[5].is_empty(),
            "字段 {} 的缺失行为、禁止第二来源、lowering 规则和 test ID 均不得为空",
            columns[0]
        );
    }
}

#[test]
fn field_source_matrix_keeps_context_and_handoff_runtime_values_out_of_markdown_lowering() {
    let rows = matrix_rows();

    {
        let field_path = "target_repository_id";
        let row = rows
            .iter()
            .find(|columns| columns.first() == Some(&field_path))
            .expect("target_repository_id 必须存在");
        assert_ne!(row[1], "markdown", "{field_path} 不得来自 markdown");
    }

    for field_path in [
        "trusted_commands[].command",
        "trusted_commands[].cwd",
        "trusted_commands[].purpose",
        "trusted_commands[].source_ref",
    ] {
        let row = rows
            .iter()
            .find(|columns| columns.first() == Some(&field_path))
            .expect("trusted command 字段必须存在");
        assert_eq!(
            row[1], "session_confirmed_context",
            "{field_path} 必须来自已确认的 trusted command catalog，而非 prompt"
        );
    }

    for columns in rows {
        assert!(
            !columns[4].contains("handoff runtime values"),
            "lowering 规则不得引入 handoff runtime values：{}",
            columns[0]
        );
    }
}

const REP4_FIXTURE: &str = include_str!(
    "../../../openspec/changes/rearch-workitem-plan-pipeline/fixtures/work-item-plan-rep4.md"
);
const MISSING_VERIFICATION_FIXTURE: &str = include_str!(
    "../../../openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/missing-verification.md"
);
const UNKNOWN_FIELD_FIXTURE: &str = include_str!(
    "../../../openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/unknown-field.md"
);
const INVALID_ID_FIXTURE: &str = include_str!(
    "../../../openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/invalid-id.md"
);
const INVALID_EARS_FIXTURE: &str = include_str!(
    "../../../openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/invalid-ears.md"
);
const EXPECTED_DIAGNOSTICS: &str = include_str!(
    "../../../openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/expected.json"
);

const DIAGNOSTIC_FIXTURES: [(&str, &str); 4] = [
    (
        "compiler-diagnostics/missing-verification.md",
        MISSING_VERIFICATION_FIXTURE,
    ),
    (
        "compiler-diagnostics/unknown-field.md",
        UNKNOWN_FIELD_FIXTURE,
    ),
    ("compiler-diagnostics/invalid-id.md", INVALID_ID_FIXTURE),
    ("compiler-diagnostics/invalid-ears.md", INVALID_EARS_FIXTURE),
];

fn item_sections<'a>(source: &'a str, item_heading: &str) -> Vec<&'a str> {
    source
        .split(item_heading)
        .nth(1)
        .expect("fixture 必须包含指定 work item")
        .split("## Work Item WI-")
        .next()
        .expect("item 必须在下一个 heading 前结束")
        .lines()
        .filter_map(|line| line.strip_prefix("### "))
        .filter(|section| grammar::STRUCTURED_SECTIONS.contains(section))
        .collect()
}

fn structured_keys(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .filter_map(|line| line.split_once(':').map(|(key, _)| key.trim()))
        .collect()
}

fn statements(source: &str) -> Vec<&str> {
    source
        .lines()
        .filter_map(|line| line.strip_prefix("- statement: "))
        .collect()
}

fn is_ears_statement(statement: &str) -> bool {
    statement.starts_with(grammar::EARS_WHEN_PREFIX)
        && statement.contains(grammar::EARS_SHALL_PREFIX)
        && statement
            .split_once(grammar::EARS_SHALL_PREFIX)
            .is_some_and(|(_, outcome)| !outcome.trim().is_empty())
}

fn is_valid_item_heading(source: &str) -> bool {
    let Some(heading) = source
        .lines()
        .find(|line| line.starts_with(grammar::ITEM_HEADING_PREFIX))
    else {
        return false;
    };
    let Some((id, title)) = heading
        .strip_prefix(grammar::ITEM_HEADING_PREFIX)
        .and_then(|rest| rest.split_once(": "))
    else {
        return false;
    };
    !title.is_empty() && !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
}

#[test]
fn fixtures_rep4_has_complete_static_source_structure() {
    let headings = [
        "## Work Item WI-001: Backend levels API",
        "## Work Item WI-002: Frontend level selector",
        "## Work Item WI-003: Integration levels API coverage",
    ];
    assert_eq!(
        REP4_FIXTURE.matches("## Work Item WI-").count(),
        headings.len(),
        "rep4 fixture 必须恰好包含 backend、frontend、integration 三个 item"
    );

    for heading in headings {
        assert!(REP4_FIXTURE.contains(heading), "fixture 缺少 {heading}");
        assert_eq!(
            item_sections(REP4_FIXTURE, heading),
            grammar::STRUCTURED_SECTIONS,
            "{heading} 必须完整覆盖稳定 section"
        );
    }

    let integration = REP4_FIXTURE
        .split("## Work Item WI-003: Integration levels API coverage")
        .nth(1)
        .expect("fixture 必须包含 integration item");
    let integration_non_goals = integration
        .split("### Non Goals\n")
        .nth(1)
        .expect("integration item 必须包含 Non Goals")
        .split("### Dependencies")
        .next()
        .expect("Non Goals 必须在 Dependencies 前结束")
        .lines()
        .filter_map(|line| line.strip_prefix("- non_goals: "))
        .collect::<Vec<_>>();
    assert_eq!(
        integration_non_goals,
        [
            "Product implementation is out of scope; tests/integration/** is explicitly allowed.",
            "Upstream tests are out of scope.",
        ],
        "integration Non Goals 只能禁止产品实现和上游测试，并须明确允许 tests/integration/**"
    );

    let frontend = REP4_FIXTURE
        .split("## Work Item WI-002: Frontend level selector")
        .nth(1)
        .expect("fixture 必须包含 frontend item")
        .split("## Work Item WI-003")
        .next()
        .expect("frontend item 必须在 integration heading 前结束");
    assert!(
        frontend.contains(
            "WHEN the levels page loads THE SYSTEM SHALL render the #level-selector container and load level-select.js."
        ),
        "HTML 验收只能约束容器和 level-select.js"
    );
    assert!(
        !frontend.contains("/api/levels"),
        "HTML 验收不得混入 API 断言"
    );
    assert!(
        integration.contains("WHEN level-select.js runs THE SYSTEM SHALL request /api/levels and render returned options."),
        "脚本证据必须单独断言 /api/levels"
    );

    for key in grammar::STRUCTURED_KEYS {
        assert!(
            structured_keys(REP4_FIXTURE).contains(&key),
            "rep4 fixture 必须覆盖矩阵中的 markdown 语义字段 {key}"
        );
    }
    assert!(
        !REP4_FIXTURE.contains("trusted_commands"),
        "trusted commands 不得写入 markdown"
    );
    assert!(
        !REP4_FIXTURE.contains("target_repository_id"),
        "target repository 不得写入 markdown"
    );
    assert!(!REP4_FIXTURE.contains("..."), "rep4 fixture 不得使用省略号");
}

#[test]
fn fixtures_diagnostic_sources_have_one_static_target_error() {
    for (fixture_name, source) in DIAGNOSTIC_FIXTURES {
        assert!(
            source.starts_with("# Work Item Plan\n\n"),
            "{fixture_name} 必须保持完整文档标题"
        );
        assert_eq!(
            source.matches("## Work Item WI-").count(),
            1,
            "{fixture_name} 只能包含一个目标 item"
        );
    }

    let missing_sections = item_sections(MISSING_VERIFICATION_FIXTURE, "## Work Item WI-001");
    assert_eq!(
        missing_sections,
        grammar::STRUCTURED_SECTIONS
            .iter()
            .copied()
            .filter(|section| *section != "Verification")
            .collect::<Vec<_>>(),
        "missing-verification fixture 除缺少 Verification 外不得再缺 section"
    );
    assert!(is_valid_item_heading(MISSING_VERIFICATION_FIXTURE));
    assert!(
        structured_keys(MISSING_VERIFICATION_FIXTURE)
            .iter()
            .all(|key| grammar::STRUCTURED_KEYS.contains(key))
    );
    assert!(
        statements(MISSING_VERIFICATION_FIXTURE)
            .iter()
            .all(|statement| is_ears_statement(statement))
    );

    assert_eq!(
        item_sections(UNKNOWN_FIELD_FIXTURE, "## Work Item WI-001"),
        grammar::STRUCTURED_SECTIONS
    );
    assert!(is_valid_item_heading(UNKNOWN_FIELD_FIXTURE));
    let unknown_keys = structured_keys(UNKNOWN_FIELD_FIXTURE)
        .into_iter()
        .filter(|key| !grammar::STRUCTURED_KEYS.contains(key))
        .collect::<Vec<_>>();
    assert_eq!(
        unknown_keys,
        ["unexpected_key"],
        "unknown-field fixture 只能含一个未知结构化字段"
    );
    assert!(
        statements(UNKNOWN_FIELD_FIXTURE)
            .iter()
            .all(|statement| is_ears_statement(statement))
    );

    assert_eq!(
        item_sections(INVALID_ID_FIXTURE, "## Work Item WI-invalid"),
        grammar::STRUCTURED_SECTIONS
    );
    assert!(
        !is_valid_item_heading(INVALID_ID_FIXTURE),
        "invalid-id fixture 必须仅在 item ID 位置保留非法形状"
    );
    assert!(
        structured_keys(INVALID_ID_FIXTURE)
            .iter()
            .all(|key| grammar::STRUCTURED_KEYS.contains(key))
    );
    assert!(
        statements(INVALID_ID_FIXTURE)
            .iter()
            .all(|statement| is_ears_statement(statement))
    );

    assert_eq!(
        item_sections(INVALID_EARS_FIXTURE, "## Work Item WI-001"),
        grammar::STRUCTURED_SECTIONS
    );
    assert!(is_valid_item_heading(INVALID_EARS_FIXTURE));
    assert!(
        structured_keys(INVALID_EARS_FIXTURE)
            .iter()
            .all(|key| grammar::STRUCTURED_KEYS.contains(key))
    );
    let invalid_statements = statements(INVALID_EARS_FIXTURE)
        .into_iter()
        .filter(|statement| !is_ears_statement(statement))
        .collect::<Vec<_>>();
    assert_eq!(
        invalid_statements,
        ["the selector renders returned options."],
        "invalid-ears fixture 不得含第二个目标 EARS 错误"
    );
}

#[test]
fn fixtures_expected_json_has_the_diagnostic_schema() {
    let expected: Value =
        serde_json::from_str(EXPECTED_DIAGNOSTICS).expect("expected.json 必须是合法 JSON");
    let entries = expected
        .as_array()
        .expect("expected.json 顶层必须是诊断数组");
    assert_eq!(entries.len(), DIAGNOSTIC_FIXTURES.len());

    let fixture_names = DIAGNOSTIC_FIXTURES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    let mut expected_fixture_names = Vec::new();
    for entry in entries {
        let object = entry
            .as_object()
            .expect("每一条 expected diagnostic 必须是对象");
        assert_eq!(
            object.len(),
            5,
            "expected diagnostic 只能包含 fixture、code、line、field、repair_example"
        );
        for field in ["fixture", "code", "line", "field", "repair_example"] {
            assert!(
                object.contains_key(field),
                "expected diagnostic 缺少 {field}"
            );
        }

        let fixture = object["fixture"]
            .as_str()
            .filter(|value| !value.is_empty())
            .expect("fixture 必须是非空字符串");
        let code = object["code"]
            .as_str()
            .filter(|value| !value.is_empty())
            .expect("code 必须是非空字符串");
        assert!(
            grammar::DIAGNOSTIC_CODES.contains(&code),
            "code 必须来自 grammar/lowering 诊断词汇表：{code}"
        );
        assert!(
            object["line"].as_u64().is_some_and(|line| line > 0),
            "line 必须是正整数"
        );
        for field in ["field", "repair_example"] {
            assert!(
                object[field]
                    .as_str()
                    .is_some_and(|value| !value.is_empty()),
                "{field} 必须是非空字符串"
            );
        }
        expected_fixture_names.push(fixture);
    }

    expected_fixture_names.sort_unstable();
    let mut fixture_names = fixture_names;
    fixture_names.sort_unstable();
    assert_eq!(
        expected_fixture_names, fixture_names,
        "expected.json 必须与四个 diagnostic fixture 一一对应"
    );
}

fn assert_diagnostic_for_field(source: &str, code: &str, field: &str, description: &str) {
    let diagnostics = lint_work_item_plan_source(source);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == code && diagnostic.field == field),
        "{description} 必须产生 {code}/{field} 诊断，实际为 {diagnostics:#?}"
    );
}

#[test]
fn source_linter_matches_every_diagnostic_fixture_field_by_field() {
    let expected: Value =
        serde_json::from_str(EXPECTED_DIAGNOSTICS).expect("expected.json 必须是合法 JSON");
    for entry in expected
        .as_array()
        .expect("expected.json 顶层必须是诊断数组")
    {
        let fixture_name = entry["fixture"].as_str().expect("fixture 必须是字符串");
        let (_, source) = DIAGNOSTIC_FIXTURES
            .iter()
            .find(|(name, _)| *name == fixture_name)
            .expect("fixture 必须有对应 source");
        let diagnostics = lint_work_item_plan_source(source);
        assert_eq!(
            diagnostics.len(),
            1,
            "{fixture_name} 必须只产生 expected.json 标注的一条诊断"
        );
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.code, entry["code"].as_str().unwrap());
        assert_eq!(diagnostic.line, entry["line"].as_u64().unwrap() as usize);
        assert_eq!(diagnostic.field, entry["field"].as_str().unwrap());
        assert_eq!(
            diagnostic.repair_example,
            entry["repair_example"].as_str().unwrap()
        );
        assert!(!diagnostic.message.is_empty(), "message 必须非空");
        assert!(!diagnostic.field.is_empty(), "field 必须非空");
        assert!(
            !diagnostic.repair_example.is_empty(),
            "repair_example 必须恰好一个非空示例"
        );
    }
}

#[test]
fn source_linter_rejects_unknown_structured_headings_keys_and_missing_required_parts() {
    let unknown_heading = REP4_FIXTURE.replacen("### Goal", "### Unexpected Heading", 1);
    assert_diagnostic_for_field(
        &unknown_heading,
        grammar::DIAGNOSTIC_CODES[1],
        "Unexpected Heading",
        "未知结构化 heading",
    );
    assert_diagnostic_for_field(
        UNKNOWN_FIELD_FIXTURE,
        grammar::DIAGNOSTIC_CODES[1],
        "unexpected_key",
        "未知结构化 key",
    );
    assert_diagnostic_for_field(
        MISSING_VERIFICATION_FIXTURE,
        grammar::DIAGNOSTIC_CODES[0],
        "Verification",
        "缺少 required section",
    );

    let missing_kind = REP4_FIXTURE.replacen("- kind: backend\n", "", 1);
    assert_diagnostic_for_field(
        &missing_kind,
        grammar::DIAGNOSTIC_CODES[0],
        "kind",
        "缺少 required field",
    );
}

#[test]
fn source_linter_rejects_duplicate_identifiers_and_invalid_dependencies() {
    for (source, field, description) in [
        (
            REP4_FIXTURE.replacen("## Work Item WI-002:", "## Work Item WI-001:", 1),
            "work_item_id",
            "重复 WI ID",
        ),
        (
            REP4_FIXTURE.replacen("- task_id: TASK-002", "- task_id: TASK-001", 1),
            "task_id",
            "重复 TASK ID",
        ),
        (
            REP4_FIXTURE.replacen("- criterion_id: AC-002", "- criterion_id: AC-001", 1),
            "criterion_id",
            "重复 AC ID",
        ),
        (
            REP4_FIXTURE.replacen("- check_id: CHECK-002", "- check_id: CHECK-001", 1),
            "check_id",
            "重复 CHECK ID",
        ),
    ] {
        assert_diagnostic_for_field(&source, grammar::DIAGNOSTIC_CODES[2], field, description);
    }

    let invalid_dependency =
        REP4_FIXTURE.replacen("- depends_on: WI-001", "- depends_on: invalid", 1);
    assert_diagnostic_for_field(
        &invalid_dependency,
        grammar::DIAGNOSTIC_CODES[2],
        "depends_on",
        "非法 dependency ID",
    );
    let missing_dependency =
        REP4_FIXTURE.replacen("- depends_on: WI-001", "- depends_on: WI-999", 1);
    assert_diagnostic_for_field(
        &missing_dependency,
        grammar::DIAGNOSTIC_CODES[2],
        "depends_on",
        "不存在的 dependency ID",
    );
    let self_dependency = REP4_FIXTURE.replacen("- depends_on: []", "- depends_on: WI-001", 1);
    assert_diagnostic_for_field(
        &self_dependency,
        grammar::DIAGNOSTIC_CODES[2],
        "depends_on",
        "自依赖",
    );
    let cycle = REP4_FIXTURE.replacen("- depends_on: []", "- depends_on: WI-002", 1);
    let cycle_diagnostics = lint_work_item_plan_source(&cycle);
    assert!(
        cycle_diagnostics.iter().any(|diagnostic| {
            diagnostic.code == grammar::DIAGNOSTIC_CODES[2]
                && diagnostic.field == "depends_on"
                && diagnostic.message.contains("循环")
        }),
        "dependency cycle 必须失败关闭，实际为 {cycle_diagnostics:#?}"
    );
}

#[test]
fn source_linter_rejects_invalid_ears_and_keeps_free_text_uninterpreted() {
    assert_diagnostic_for_field(
        INVALID_EARS_FIXTURE,
        grammar::DIAGNOSTIC_CODES[3],
        "statement",
        "非法 EARS",
    );

    let free_text = format!(
        "{REP4_FIXTURE}\n### Notes\n任意 Unicode：中文、😀、a:b 与 | 表格 | 均保留。\n| 标题:甲 | 值:乙 |\n### Rationale\n自由文本不应被识别为 structured_key: value。\n"
    );
    assert!(
        lint_work_item_plan_source(&free_text).is_empty(),
        "Notes/Rationale 中的 Unicode、冒号与表格文本必须允许"
    );
    let ast = parse_work_item_plan(REP4_FIXTURE).expect("完整 rep4 source 必须可解析");
    assert_eq!(ast.items.len(), 3);
}

#[test]
fn source_linter_sorts_complete_diagnostics_stably() {
    let source = UNKNOWN_FIELD_FIXTURE.replacen(
        "- statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return configured levels.",
        "- statement: not an EARS statement.",
        1,
    );
    let diagnostics = lint_work_item_plan_source(&source);
    assert!(diagnostics.len() >= 2, "测试输入必须产生多个诊断");
    assert!(diagnostics.windows(2).all(|pair| {
        (pair[0].line, pair[0].field.as_str(), pair[0].code.as_str())
            <= (pair[1].line, pair[1].field.as_str(), pair[1].code.as_str())
    }));
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic.line > 0
            && !diagnostic.field.is_empty()
            && !diagnostic.message.is_empty()
            && !diagnostic.repair_example.is_empty()
    }));
}

#[test]
fn lower_typed_ir() {
    let catalog = vec![
        TrustedDraftVerificationCommand {
            command: "cargo test --locked --lib levels_api".to_string(),
            cwd: "backend".to_string(),
            purpose: "backend checks".to_string(),
            source_ref: "cargo test --locked --lib levels_api".to_string(),
        },
        TrustedDraftVerificationCommand {
            command: "pnpm test level-select".to_string(),
            cwd: "frontend".to_string(),
            purpose: "frontend checks".to_string(),
            source_ref: "pnpm test level-select".to_string(),
        },
        TrustedDraftVerificationCommand {
            command: "cargo test --locked --test levels_integration".to_string(),
            cwd: "integration".to_string(),
            purpose: "integration checks".to_string(),
            source_ref: "cargo test --locked --test levels_integration".to_string(),
        },
    ];
    let context = WorkItemPlanSourceContext {
        target_repository_id: "repo-levels".to_string(),
        trusted_command_catalog: catalog.clone(),
    };
    let ir = compile_work_item_plan(REP4_FIXTURE, &context).expect("rep4 应 lower 为 typed IR");

    assert_eq!(ir.items.len(), 3);
    assert_eq!(
        ir.items
            .iter()
            .map(|item| item.target_repository_id.as_str())
            .collect::<Vec<_>>(),
        ["repo-levels", "repo-levels", "repo-levels"]
    );
    assert_eq!(ir.items[0].contract.depends_on, Vec::<String>::new());
    assert_eq!(ir.items[1].contract.depends_on, vec!["WI-001"]);
    assert_eq!(ir.items[2].contract.depends_on, vec!["WI-001", "WI-002"]);
    assert_eq!(
        ir.items
            .iter()
            .map(|item| item.verification_plan.checks.clone())
            .collect::<Vec<_>>(),
        ir.items
            .iter()
            .map(|item| item.contract.verification_checks.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(ir.items[0].trusted_commands, vec![catalog[0].clone()]);
    assert_eq!(ir.items[1].trusted_commands, vec![catalog[1].clone()]);
    assert_eq!(ir.items[2].trusted_commands, vec![catalog[2].clone()]);

    let json = serde_json::to_value(&ir).expect("IR 必须可序列化");
    assert_eq!(
        json.as_object().unwrap().keys().collect::<Vec<_>>(),
        [&"compiler_version", &"items", &"source_revision_hash"]
    );
    for item in json["items"].as_array().unwrap() {
        assert!(
            !item
                .as_object()
                .unwrap()
                .contains_key("source_revision_hash")
        );
        assert!(!item.as_object().unwrap().contains_key("compiler_version"));
    }

    let unknown = compile_work_item_plan(
        &REP4_FIXTURE.replacen(
            "cargo test --locked --lib levels_api",
            "unknown-command-ref",
            1,
        ),
        &context,
    )
    .expect_err("未知 command ref 必须失败关闭");
    assert!(
        unknown
            .iter()
            .any(|diagnostic| diagnostic.field == "trusted_commands")
    );

    let markdown_owned = REP4_FIXTURE.replacen(
        "- logical_work_item_id: WI-001",
        "- target_repository_id: repo-evil\n- logical_work_item_id: WI-001",
        1,
    );
    assert!(compile_work_item_plan(&markdown_owned, &context).is_err());
}

#[test]
fn untrusted_command_diagnostic_points_to_its_own_command_line() {
    let source = REP4_FIXTURE.replacen(
        "### Handoff Schema\n- required_fields: commit_sha",
        "- check_id: CHECK-004\n- command: unknown-command-ref\n- manual_instruction: Confirm the extra check.\n- required: true\n- non_zero_test_execution_required: true\n\n### Handoff Schema\n- required_fields: commit_sha",
        1,
    );
    let expected_line = source
        .lines()
        .position(|line| line == "- command: unknown-command-ref")
        .expect("测试输入必须包含未知 command")
        + 1;
    let context = WorkItemPlanSourceContext {
        target_repository_id: "repo-levels".to_string(),
        trusted_command_catalog: vec![
            TrustedDraftVerificationCommand {
                command: "cargo test --locked --lib levels_api".to_string(),
                cwd: "backend".to_string(),
                purpose: "backend checks".to_string(),
                source_ref: "cargo test --locked --lib levels_api".to_string(),
            },
            TrustedDraftVerificationCommand {
                command: "pnpm test level-select".to_string(),
                cwd: "frontend".to_string(),
                purpose: "frontend checks".to_string(),
                source_ref: "pnpm test level-select".to_string(),
            },
            TrustedDraftVerificationCommand {
                command: "cargo test --locked --test levels_integration".to_string(),
                cwd: "integration".to_string(),
                purpose: "integration checks".to_string(),
                source_ref: "cargo test --locked --test levels_integration".to_string(),
            },
        ],
    };

    let diagnostics = compile_work_item_plan(&source, &context)
        .expect_err("未知 trusted command 必须让 lowering 失败关闭");
    assert_eq!(diagnostics.len(), 1, "测试输入只能产生一个 lowering 诊断");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.field, "trusted_commands");
    assert_eq!(diagnostic.line, expected_line);
}

#[test]
fn parse_diagnostics_are_document_scoped_spanned_and_stable() {
    let source = REP4_FIXTURE
        .split("## Work Item WI-002:")
        .next()
        .expect("fixture 必须包含首个 item")
        .replacen("## Work Item WI-001:", "## Work Item WI-invalid:", 1)
        .replacen(
            "### Goal\n- summary: WHEN a level list request arrives THE SYSTEM SHALL return the configured levels JSON.\n\n",
            "",
            1,
        )
        .replacen(
            "- statement: WHEN the levels API receives GET /api/levels THE SYSTEM SHALL return the configured levels JSON.",
            "- statement: the levels API returns configured levels JSON.",
            1,
        );

    let expected = [
        (
            grammar::DIAGNOSTIC_CODES[2],
            3,
            "work_item_id",
            "## Work Item WI-001: Levels API fixture",
        ),
        (grammar::DIAGNOSTIC_CODES[0], 11, "Goal", "### Goal"),
        (
            grammar::DIAGNOSTIC_CODES[3],
            25,
            "statement",
            "- statement: WHEN the selector loads THE SYSTEM SHALL render returned options.",
        ),
    ];
    let diagnostics = parse_work_item_plan(&source).expect_err("三个文档级错误必须聚合返回");
    assert_eq!(
        diagnostics.len(),
        expected.len(),
        "实际诊断为 {diagnostics:#?}"
    );
    for (diagnostic, (code, line, field, repair_example)) in diagnostics.iter().zip(expected) {
        assert_eq!(diagnostic.code, code);
        assert_eq!(diagnostic.line, line, "诊断行号必须是 1-based");
        assert_eq!(diagnostic.field, field, "field 必须使用既有 canonical path");
        assert!(!diagnostic.message.is_empty(), "中文诊断消息不得为空");
        assert_eq!(diagnostic.repair_example, repair_example);
        let is_structured_example = diagnostic.repair_example.starts_with("- ");
        assert_eq!(
            diagnostic.repair_example.matches('\n').count(),
            0,
            "repair example 必须恰好提供一个单行可回喂示例"
        );
        assert!(
            !is_structured_example
                || diagnostic.repair_example.starts_with("- key: ")
                || diagnostic.repair_example.starts_with("- statement: "),
            "结构化 repair example 只能给出一个 - key: 或 EARS 例"
        );
    }

    let ast = parse_work_item_plan(REP4_FIXTURE).expect("有效 source 必须构造 AST");
    assert_eq!(ast.items[0].id.value, "WI-001");
    assert_eq!(ast.items[0].id.line, 3);
    assert_eq!(ast.items[0].title.value, "Backend levels API");
    assert_eq!(ast.items[0].title.line, 3);
    assert_eq!(ast.items[0].sections[0].name.value, "Identity");
    assert_eq!(ast.items[0].sections[0].name.line, 5);
    assert_eq!(
        ast.items[0].sections[0].fields[0].key.value,
        "schema_version"
    );
    assert_eq!(ast.items[0].sections[0].fields[0].key.line, 6);
    assert_eq!(ast.items[0].sections[0].fields[0].value.value, "1");
    assert_eq!(ast.items[0].sections[0].fields[0].value.line, 6);
    assert!(
        ast.notes
            .iter()
            .chain(&ast.rationale)
            .all(|line| line.line > 0)
    );

    assert_eq!(
        parse_work_item_plan(REP4_FIXTURE),
        parse_work_item_plan(REP4_FIXTURE),
        "同源 parse 的 AST 与 diagnostic 顺序必须稳定"
    );
}
