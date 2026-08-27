use super::{grammar, types};

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
