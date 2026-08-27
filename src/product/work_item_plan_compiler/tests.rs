use super::{grammar, types};
use serde_json::Value;

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
    assert_eq!(grammar::PLAN_SECTION, grammar::DOCUMENT_HEADING);
    assert_eq!(
        grammar::WORK_ITEM_SECTION_PREFIX,
        grammar::ITEM_HEADING_PREFIX
    );
    assert_eq!(grammar::KEY_VALUE_SEPARATOR, ": ");
    assert_eq!(grammar::IDENTIFIED_LINE_SEPARATOR, " | ");
    assert_eq!(grammar::UNKNOWN_STRUCTURED_KEY_POLICY, "fail_closed");
    assert_eq!(grammar::FREE_TEXT_SECTION_POLICY, "allow_free_text");
    assert_eq!(
        grammar::EARS_KEYWORDS,
        ["WHEN", "THE SYSTEM SHALL", "observable outcome"]
    );

    let ast = types::WorkItemPlanAst {
        items: vec![types::WorkItemPlanItemAst {
            id: "WI-001".to_string(),
            sections: std::collections::BTreeMap::new(),
        }],
        notes: vec!["note".to_string()],
        rationale: vec!["rationale".to_string()],
    };
    let cloned_ast = ast.clone();
    assert_eq!(ast, cloned_ast);
    assert_eq!(ast.items[0].id, "WI-001");
    assert_eq!(ast.notes, ["note"]);
    assert_eq!(ast.rationale, ["rationale"]);

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
