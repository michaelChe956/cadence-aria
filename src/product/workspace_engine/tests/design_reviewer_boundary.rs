use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::cross_cutting::structured_output::StructuredOutputErrorCode;
use crate::product::models::WorkspaceType;

const BOUNDARY_EXAMPLES_MARKER: &str = "[design_reviewer_boundary_examples]";
const REVIEWER_COVERAGE_TITLE: &str = "Reviewer Capability Coverage Projection";
const BOUNDARY_EXAMPLES_FINAL_LINE: &str = "命中才可 must_fix，未命中最高 suggestion。";

fn design_candidate_with_abstract_traceability() -> String {
    complete_design_artifact("删除采用软删除并保留审计字段。", "删除后查询不再返回该记录。")
        .replace(
            "- [DEC-001] -> [REQ-001]",
            "- [DEC-001] -> [REQ-003] / [AC-003]（验收口径：删除后查询不再返回该记录）",
        )
}

fn design_candidate_with_executable_test_plan() -> String {
    complete_design_artifact("删除采用软删除并保留审计字段。", "删除后查询不再返回该记录。")
        .replace(
            "- [CMP-001] 复用现有组件边界。",
            "- [CMP-002] RetryPolicy：统一重试与退避，并负责为自身编写单元测试。",
        )
        .replace("[API-001]", "[API-002]")
        .replace(
            "## 风险\n无。",
            "## 风险\n- 回归验证计划：第一步在 tests/idempotency.rs 用 mockito 搭建夹具，第二步执行 cargo test --locked idempotency，第三步补充 3 条超时场景用例。",
        )
        .replace(
            "- [DEC-001] -> [REQ-001]",
            "- [DEC-001] -> [REQ-003] / [AC-003]（验收口径：删除后查询不再返回该记录）",
        )
}

fn design_candidate_with_risk_mentioning_verification() -> String {
    complete_design_artifact("删除采用软删除并保留审计字段。", "删除后查询不再返回该记录。")
        .replace(
            "## 风险\n无。",
            "## 风险\n- [DEC-001] 幂等键冲突概率未知；缓解：由下游 Work Item 阶段安排验证，Design 阶段不定义测试方案。",
        )
        .replace(
            "- [DEC-001] -> [REQ-001]",
            "- [DEC-001] -> [REQ-003] / [AC-003]（验收口径：删除后查询不再返回该记录）",
        )
}

fn boundary_examples_from_prompt(prompt: &str) -> &str {
    let start = prompt
        .find(BOUNDARY_EXAMPLES_MARKER)
        .expect("design prompt must include the boundary examples marker");
    let end = prompt[start..]
        .find(BOUNDARY_EXAMPLES_FINAL_LINE)
        .map(|offset| start + offset + BOUNDARY_EXAMPLES_FINAL_LINE.len())
        .expect("design prompt must include the complete boundary examples");
    &prompt[start..end]
}

fn design_review_engine(session_id: &str, candidate: &str) -> WorkspaceEngine {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(16);
    let mut session = make_session(session_id);
    session.workspace_type = WorkspaceType::Design;
    session.artifact = Some(artifact_payload(candidate));
    WorkspaceEngine::new(store, tx, session)
}

fn review_completion_for(
    input: &StreamingProviderInput,
    mut value: serde_json::Value,
) -> ProviderCompletion {
    let contract = input
        .structured_output_contract
        .as_ref()
        .expect("review input must carry a structured output contract");
    value
        .as_object_mut()
        .expect("review fixture must be an object")
        .insert(
            "nonce".to_string(),
            serde_json::Value::String(contract.nonce.clone()),
        );
    ProviderCompletion::from_output(
        format!(
            "审核意见\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">{value}</ARIA_STRUCTURED_OUTPUT>",
            contract.nonce
        ),
        Some(contract),
        None,
    )
}

#[test]
fn design_reviewer_boundary_prompt_injects_examples_once_before_actual_template_and_examples_stay_unframed() {
    let engine = design_review_engine(
        "sess_design_reviewer_boundary_prompt",
        &design_candidate_with_abstract_traceability(),
    );
    let input = engine.build_review_input().expect("design review input");
    let contract = input
        .structured_output_contract
        .as_ref()
        .expect("design review contract");
    let marker_index = input
        .prompt
        .find(BOUNDARY_EXAMPLES_MARKER)
        .expect("design prompt must include the boundary examples marker");
    let actual_template_index = input
        .prompt
        .find("实际输出模板（必须使用本请求 nonce）：")
        .expect("design prompt must include the actual output template");

    assert_eq!(
        input.prompt.matches(BOUNDARY_EXAMPLES_MARKER).count(),
        1,
        "design prompt must inject boundary examples exactly once: {}",
        input.prompt
    );
    assert!(
        marker_index < actual_template_index,
        "boundary examples must precede the actual output template: {}",
        input.prompt
    );
    assert!(
        actual_template_index
            < input
                .prompt
                .rfind(&format!("nonce=\"{}\"", contract.nonce))
                .expect("actual output template must carry the request nonce"),
        "actual output template must carry its request nonce after the examples: {}",
        input.prompt
    );
    assert_eq!(
        input.prompt.matches("[artifact_boundary_must_fix_rules]").count(),
        1
    );
    assert_eq!(input.prompt.matches("[artifact_schema_review_gate]").count(), 1);
    assert!(!input.prompt.contains("</ARIA_STRUCTURED_OUTPUT nonce="));

    let examples = boundary_examples_from_prompt(&input.prompt);
    assert!(!examples.contains("ARIA_STRUCTURED_OUTPUT"));
    assert!(!examples.contains("nonce="));
    assert!(!examples.contains("EXAMPLE_NONCE"));
    for id in ["DEC-001", "CMP-002", "API-002", "REQ-003"] {
        assert!(examples.contains(id), "missing fixed example fingerprint ID {id}");
    }
}

#[test]
fn design_reviewer_boundary_non_design_prompts_exclude_examples() {
    for workspace_type in [WorkspaceType::Story, WorkspaceType::WorkItem] {
        let (_tmp, store) = setup();
        let (tx, _rx) = mpsc::channel(16);
        let mut session = make_session(&format!("sess_non_design_boundary_{workspace_type:?}"));
        session.workspace_type = workspace_type.clone();
        session.artifact = Some(artifact_payload("# Existing artifact"));
        let engine = WorkspaceEngine::new(store, tx, session);
        let input = engine.build_review_input().expect("non-design review input");

        assert_eq!(
            input.prompt.matches(BOUNDARY_EXAMPLES_MARKER).count(),
            0,
            "{workspace_type:?} reviewer prompt must not include design examples: {}",
            input.prompt
        );
        assert!(!input.prompt.contains(REVIEWER_COVERAGE_TITLE));
        assert!(!input.prompt.contains("handoff_consumption"));
        assert!(!input.prompt.contains("write_scope_conflicts"));
        assert!(!input.prompt.contains("category=contract_gap"));
    }

    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_non_design_boundary_work_item_plan");
    assert_eq!(
        engine.session().flow_kind,
        crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy
    );
    let input = engine
        .build_review_input()
        .expect("work item plan review input");
    assert_eq!(input.prompt.matches(BOUNDARY_EXAMPLES_MARKER).count(), 0);
    assert!(!input.prompt.contains(REVIEWER_COVERAGE_TITLE));
    assert!(!input.prompt.contains("handoff_consumption"));
    assert!(!input.prompt.contains("write_scope_conflicts"));
    assert!(!input.prompt.contains("category=contract_gap"));

    let design_engine = design_review_engine(
        "sess_non_design_boundary_design",
        &design_candidate_with_abstract_traceability(),
    );
    let input = design_engine
        .build_review_input()
        .expect("design review input");
    assert!(!input.prompt.contains(REVIEWER_COVERAGE_TITLE));
    assert!(!input.prompt.contains("handoff_consumption"));
    assert!(!input.prompt.contains("write_scope_conflicts"));
    assert!(!input.prompt.contains("category=contract_gap"));
}

#[test]
fn design_reviewer_boundary_case_verdicts_map_to_expected_gates_through_structured_output() {
    let abstract_candidate = design_candidate_with_abstract_traceability();
    let abstract_engine = design_review_engine("sess_design_boundary_abstract", &abstract_candidate);
    let abstract_input = abstract_engine
        .build_review_input()
        .expect("abstract traceability review input");
    let abstract_completion = review_completion_for(
        &abstract_input,
        serde_json::json!({
            "verdict": "pass",
            "summary": "抽象追踪可进入人工确认",
            "findings": [{
                "severity": "suggestion",
                "message": "可选补充数据保留期，不影响当前阶段可用性",
                "evidence": "[DEC-001] -> [REQ-003] / [AC-003]（验收口径：删除后查询不再返回该记录）",
                "required_action": "可选：补充保留期约束"
            }]
        }),
    );
    let abstract_verdict = abstract_engine
        .parse_review_completion_for_active_node(&abstract_completion)
        .expect("abstract traceability completion must parse");
    assert_eq!(abstract_verdict.verdict, ReviewVerdictType::Pass);
    assert_eq!(abstract_verdict.review_gate, ReviewGate::UserConfirmAllowed);
    assert!(abstract_verdict
        .findings
        .iter()
        .all(|finding| finding.severity == ReviewFindingSeverity::Suggestion));
    assert!(!abstract_verdict.findings.iter().any(|finding| {
        matches!(
            finding.severity,
            ReviewFindingSeverity::MustFix | ReviewFindingSeverity::Blocking
        )
    }));

    let executable_candidate = design_candidate_with_executable_test_plan();
    let executable_engine =
        design_review_engine("sess_design_boundary_executable", &executable_candidate);
    let executable_input = executable_engine
        .build_review_input()
        .expect("executable test plan review input");
    let executable_completion = review_completion_for(
        &executable_input,
        serde_json::json!({
            "verdict": "revise",
            "summary": "Design 越界写入可执行测试计划与测试职责分派",
            "findings": [
                {
                    "severity": "must_fix",
                    "message": "可执行测试内容属于 Work Item 阶段",
                    "evidence": "第一步在 tests/idempotency.rs 用 mockito 搭建夹具，第二步执行 cargo test --locked idempotency",
                    "required_action": "删除测试文件、框架、命令与分步场景"
                },
                {
                    "severity": "must_fix",
                    "message": "测试职责不能分派给组件",
                    "evidence": "[CMP-002] RetryPolicy：统一重试与退避，并负责为自身编写单元测试。",
                    "required_action": "删除组件测试负责方表述"
                }
            ]
        }),
    );
    let executable_verdict = executable_engine
        .parse_review_completion_for_active_node(&executable_completion)
        .expect("executable test plan completion must parse");
    assert_eq!(executable_verdict.verdict, ReviewVerdictType::Revise);
    assert_eq!(
        executable_verdict.review_gate,
        ReviewGate::RequiresRevision
    );
    assert_eq!(
        executable_verdict
            .findings
            .iter()
            .filter(|finding| finding.severity == ReviewFindingSeverity::MustFix)
            .count(),
        2
    );
    assert!(executable_verdict
        .findings
        .iter()
        .all(|finding| !finding.evidence.is_empty()));

    let risk_candidate = design_candidate_with_risk_mentioning_verification();
    let risk_engine = design_review_engine("sess_design_boundary_risk", &risk_candidate);
    let risk_input = risk_engine
        .build_review_input()
        .expect("risk mention review input");
    let risk_completion = review_completion_for(
        &risk_input,
        serde_json::json!({
            "verdict": "pass",
            "summary": "风险缓解只声明验证归属，未越界",
            "findings": []
        }),
    );
    let risk_verdict = risk_engine
        .parse_review_completion_for_active_node(&risk_completion)
        .expect("risk mention completion must parse");
    assert_eq!(risk_verdict.verdict, ReviewVerdictType::Pass);
    assert_eq!(risk_verdict.review_gate, ReviewGate::UserConfirmAllowed);
    assert!(risk_verdict.findings.is_empty());
}

#[test]
fn design_reviewer_boundary_copied_examples_inside_a_sentinel_cannot_form_a_verdict() {
    let engine = design_review_engine(
        "sess_design_boundary_copy",
        &design_candidate_with_executable_test_plan(),
    );
    let input = engine.build_review_input().expect("design review input");
    let contract = input
        .structured_output_contract
        .as_ref()
        .expect("design review contract");
    let examples = boundary_examples_from_prompt(&input.prompt);
    assert!(!examples.contains("ARIA_STRUCTURED_OUTPUT"), "{examples}");
    assert!(!examples.contains("EXAMPLE_NONCE"), "{examples}");
    let completion = ProviderCompletion::from_output(
        format!(
            "<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">{examples}</ARIA_STRUCTURED_OUTPUT>",
            contract.nonce
        ),
        Some(contract),
        None,
    );

    let result = engine.parse_review_completion_for_active_node(&completion);
    assert!(
        matches!(
            result,
            Err(ReviewCompletionError::Syntax(ref error))
                if error.code == StructuredOutputErrorCode::InvalidJson
        ),
        "unframed examples inside a sentinel must fail structured parsing: {result:?}"
    );
}

#[test]
fn design_reviewer_boundary_candidates_stay_out_of_the_deterministic_gate() {
    for candidate in [
        design_candidate_with_abstract_traceability(),
        design_candidate_with_executable_test_plan(),
        design_candidate_with_risk_mentioning_verification(),
    ] {
        assert!(
            validate_workspace_artifact_constraints(&candidate, &WorkspaceType::Design).passed,
            "boundary candidate must remain a reviewer concern rather than a deterministic gate failure: {candidate}"
        );
    }
}
