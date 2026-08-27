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

    for field_path in ["target_repository_id"] {
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
