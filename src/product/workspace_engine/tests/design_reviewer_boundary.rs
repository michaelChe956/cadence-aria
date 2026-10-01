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

// ============================================================================
// Task 2.4（lc-root-initialization，REQ-PLN-01/PLN-07、REQ-ENV-09/ENV-10）：
// WorkItemPlan review 全分支 builder 的 LC root-cwd 迁移。
//
// `all_logical_plan_review_builders_keep_reviewer_write_guard`：LC 会话下
// legacy 整组候选 / outline / draft / batch / single-candidate / projection
// 六个 plan review builder 与 ReviewOnly 泛型分支均以 canonical LC root 为
// cwd（消费 Task 2.5 `working_directory` 字段合同）、成员 checkout 为显式
// review target（不从 cwd 推导）；Reviewer 写工具策略（DenyFileWriteBuiltins）
// 不放宽。伴随断言：LC 缺显式 target fail-closed（绝不回退进程 cwd/成员
// cwd）；单仓（无 gateway）builder 原值不变（`working_directory=None`、
// cwd 语义仍由 `working_dir` 承载）。
// ============================================================================

/// 最小 LC gateway 夹具：authority root = canonical manifest root，成员
/// checkout 位于 root 子树（与 Task 2.3 revision 夹具同构，去掉与本测试
/// 无关的登记/selection 链——builder 面只消费 `authority_root`，不触
/// gateway validate）。
fn logical_plan_review_gateway_fixture() -> (
    tempfile::TempDir,
    Arc<LogicalCodebaseProviderGateway>,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseProviderGateway,
    };

    let root = tempfile::tempdir().expect("temporary logical root");
    let manifest_root = root.path().join("lc-root");
    let member_root = manifest_root.join("member-checkout");
    std::fs::create_dir_all(&member_root).expect("create member checkout");
    // REQ-PIB-02：baseline 解析对裸目录 fail-closed——成员 checkout 需 main
    // 分支 + 初始提交（builder 的 baseline teaching 经 repository_path 解析）。
    init_fixture_git_repo(&member_root);
    let authority_root = std::fs::canonicalize(&manifest_root).expect("canonical root");
    let registry = ProviderRegistry::new();
    let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
        AggregatePolicyArtifactStore::new(ProductAppPaths::new(root.path().join(".aria-lc"))),
        Arc::new(ReviewStaticCapabilitySource::default()),
        Arc::new(ReviewPassThroughTargetResolver),
        Arc::new(registry),
        Arc::new(ReviewStubSyncAdapter),
        review_always_available_gate(),
        Arc::new(GatewayRunAudit::new()),
        authority_root.clone(),
    ));
    (root, gateway, authority_root, member_root)
}

/// LC 分支 builder 输出的统一断言：root cwd + 显式成员 target + Reviewer
/// 写工具策略保持。
fn assert_logical_plan_review_input(
    input: &StreamingProviderInput,
    case: &str,
    authority_root: &std::path::Path,
    member_root: &std::path::Path,
) {
    use crate::cross_cutting::streaming_provider::ProviderToolPolicy;

    assert_eq!(input.role, AdapterRole::Reviewer, "{case}: reviewer role");
    assert_eq!(
        input.tool_policy,
        Some(ProviderToolPolicy::deny_file_write_builtins()),
        "{case}: ReviewOnly 写工具策略不得放宽（DenyFileWriteBuiltins 保持）"
    );
    assert_eq!(
        input.working_directory.as_deref(),
        Some(authority_root),
        "{case}: LC cwd 必须是 canonical root——不得回退成员 worktree 或进程 cwd"
    );
    assert_eq!(
        input.working_dir, member_root,
        "{case}: review target 保持显式成员 checkout（不从 cwd 推导）"
    );
}

#[tokio::test]
async fn all_logical_plan_review_builders_keep_reviewer_write_guard() {
    use crate::product::work_item_plan_compiler::{
        PlanCandidateIr, PlanCandidateItemIr, PlanCandidateMechanicalReport,
        WORK_ITEM_PLAN_COMPILER_VERSION,
    };
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
    use crate::product::work_item_plan_source_store::{
        PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
        WorkItemPlanSourceStore,
    };
    use sha2::{Digest, Sha256};

    let (_lc_root, gateway, authority_root, member_root) = logical_plan_review_gateway_fixture();

    // —— WorkItemPlan 面：legacy + delegated builders 共用一台 LC engine ——
    let (_tmp, _checkpoint_store, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_logical_plan_review_builders");
    let mut engine = engine.with_logical_provider_gateway(gateway.clone());
    engine.session.repository_path = Some(member_root.clone());

    // legacy 整组拆分候选分支。
    let input = engine
        .build_work_item_plan_review_input()
        .expect("legacy plan review input");
    assert_logical_plan_review_input(&input, "legacy", &authority_root, &member_root);

    // outline 分支。
    let outline_payload = work_item_plan_outline_artifact();
    let ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } = outline_payload
    else {
        panic!("expected outline candidate artifact");
    };
    let input = engine
        .build_work_item_plan_outline_review_input(&outline_candidate)
        .expect("outline review input");
    assert_logical_plan_review_input(&input, "outline", &authority_root, &member_root);

    // draft 分支。
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    let draft_payload = work_item_draft_artifact_payload(
        &plan_id,
        "outline_a",
        "draft_a",
        WorkItemDraftStatus::Draft,
    );
    let ArtifactPayload::WorkItemDraftCandidate { draft_candidate } = draft_payload else {
        panic!("expected draft candidate artifact");
    };
    let input = engine
        .build_work_item_draft_review_input(&draft_candidate)
        .expect("draft review input");
    assert_logical_plan_review_input(&input, "draft", &authority_root, &member_root);

    // batch 分支。
    save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
    let input = engine
        .build_work_item_batch_review_input()
        .expect("batch review input");
    assert_logical_plan_review_input(&input, "batch", &authority_root, &member_root);

    // single-candidate 分支（durable IR + mechanical report）。
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source = "# logical plan review builders source\n";
    let mut source_revision = SourceRevisionRecord {
        id: "source-logical-plan-review".to_string(),
        source: source.to_string(),
        source_revision_hash: hex::encode(Sha256::digest(source.as_bytes())),
        content_hash: String::new(),
    };
    source_revision.content_hash = source_revision.content_hash().expect("source content hash");
    source_store
        .put_source_revision("project_0001", "issue_0001", &plan_id, &source_revision)
        .expect("persist source revision");
    let contract_a = crate::product::work_item_contract::canonical_contract_fixture("wi-a");
    let mut contract_b = crate::product::work_item_contract::canonical_contract_fixture("wi-b");
    contract_b.depends_on = vec!["wi-a".to_string()];
    let ir = PlanCandidateIr {
        source_revision_hash: source_revision.source_revision_hash.clone(),
        compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
        items: vec![
            PlanCandidateItemIr {
                target_repository_id: "repository_0001".to_string(),
                contract: contract_a,
                verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                    checks: Vec::new(),
                },
                trusted_commands: Vec::new(),
            },
            PlanCandidateItemIr {
                target_repository_id: "repository_0001".to_string(),
                contract: contract_b,
                verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                    checks: Vec::new(),
                },
                trusted_commands: Vec::new(),
            },
        ],
    };
    let mut ir_record = PlanCandidateIrRecord {
        id: "ir-logical-plan-review".to_string(),
        source_revision_id: source_revision.id.clone(),
        ir,
        content_hash: String::new(),
    };
    ir_record.content_hash = ir_record.content_hash().expect("IR content hash");
    let ir_ref = source_store
        .put_plan_candidate_ir("project_0001", "issue_0001", &plan_id, &ir_record)
        .expect("persist compiled IR");
    let mut report = PlanCandidateMechanicalReportRecord {
        id: "report-logical-plan-review".to_string(),
        source_revision_id: source_revision.id,
        ir_id: ir_record.id,
        report: PlanCandidateMechanicalReport {
            source_revision_hash: ir_record.ir.source_revision_hash.clone(),
            compiler_version: ir_record.ir.compiler_version.clone(),
            findings: Vec::new(),
        },
        content_hash: String::new(),
    };
    report.content_hash = report.content_hash().expect("report content hash");
    let report_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", &plan_id, &report)
        .expect("persist mechanical report");
    engine.session.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    engine.session.plan_candidate_ir_ref = Some(ir_ref);
    engine.session.mechanical_report_ref = Some(report_ref);
    engine.session.artifact = Some(ArtifactPayload::Markdown {
        markdown: "compiled markdown is not authoritative".to_string(),
        diff: None,
    });
    let input = engine
        .build_work_item_plan_review_input()
        .expect("single-candidate review input");
    assert_logical_plan_review_input(&input, "single-candidate", &authority_root, &member_root);

    // projection 分支（accepted-contract 夹具 + 初始编译产物；编译在 LC 覆盖
    // 前完成，避免 repository_path 参与编译链）。
    let (_projection_tmp, _projection_lifecycle, _projection_plan_id, mut projection_engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = projection_engine
        .run_work_item_plan_compile()
        .await
        .expect("initial plan compile");
    projection_engine = projection_engine.with_logical_provider_gateway(gateway.clone());
    projection_engine.session.repository_path = Some(member_root.clone());
    projection_engine.session.artifact = Some(ArtifactPayload::WorkItemPlanProjection {
        projection: Box::new(outcome.plan_projection_bundle),
    });
    let input = projection_engine
        .build_work_item_plan_review_input()
        .expect("projection review input");
    assert_logical_plan_review_input(&input, "projection", &authority_root, &member_root);

    // ReviewOnly 泛型分支（Design review）同规则。
    let (_design_tmp, design_store) = setup();
    let (design_tx, _design_rx) = mpsc::channel(16);
    let mut design_session = make_session("sess_logical_review_only_cwd");
    design_session.workspace_type = WorkspaceType::Design;
    design_session.artifact = Some(artifact_payload(&complete_design_artifact(
        "删除采用软删除并保留审计字段。",
        "删除后查询不再返回该记录。",
    )));
    design_session.repository_path = Some(member_root.clone());
    let design_engine = WorkspaceEngine::new(design_store, design_tx, design_session)
        .with_logical_provider_gateway(gateway.clone());
    let input = design_engine
        .build_review_input()
        .expect("review-only input");
    assert_logical_plan_review_input(&input, "review-only", &authority_root, &member_root);

    // —— LC 缺显式 target：fail-closed，绝不回退进程 cwd / 成员 cwd ——
    engine.session.repository_path = None;
    let error = engine
        .build_work_item_plan_review_input()
        .expect_err("logical review without explicit target must fail closed");
    assert!(
        error.contains("explicit member target"),
        "unexpected fail-closed message: {error}"
    );

    // —— 单仓零变化：无 gateway 时 builder 原值不变 ——
    let (_single_tmp, _single_checkpoint, _single_lifecycle, _single_plan, mut single_engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_single_repo_plan_review_unchanged");
    // repository_path 缺省：保留进程 cwd 回退（legacy 语义原值）。
    let input = single_engine
        .build_work_item_plan_review_input()
        .expect("single-repo plan review input");
    assert_eq!(input.working_directory, None, "单仓 builder 不注入 LC cwd");
    assert_eq!(
        input.working_dir,
        std::env::current_dir().expect("process cwd"),
        "单仓 repository_path 缺省仍回退进程 cwd（原值不变）"
    );
    // repository_path 显式：cwd = target 原值（单仓 cwd==target 等式保持）。
    single_engine.session.repository_path = Some(_single_tmp.path().join("repository"));
    let input = single_engine
        .build_work_item_plan_review_input()
        .expect("single-repo plan review input with explicit path");
    assert_eq!(input.working_directory, None);
    assert_eq!(
        input.working_dir,
        _single_tmp.path().join("repository"),
        "单仓 repository_path 显式时 cwd=target（原值不变）"
    );
}
