// single_candidate_prompt 拆分（large_file_guard 1200 行上限）：review prompt
// 读取/校验测试从 mod.rs 移入；测试逻辑与断言零改动（super:: 辅助函数
// 改经 super::super:: 指向 tests 作用域）。

use super::*;

/// D1（REQ-WSC-06「reviewer 无损紧凑序列化」）：提取 reviewer prompt 中
/// `\n### {title}\n` 段的 JSON 载荷——流式解析只消费一个完整 JSON 值，
/// 返回裁掉尾随空白后的载荷文本与解析对象。
fn compact_section_payload<'a>(prompt: &'a str, title: &str) -> (&'a str, serde_json::Value) {
    let heading = format!("\n### {title}\n");
    let start = prompt
        .find(&heading)
        .unwrap_or_else(|| panic!("review prompt section `{title}` missing"));
    let rest = &prompt[start + heading.len()..];
    let mut stream =
        serde_json::Deserializer::from_str(rest).into_iter::<serde_json::Value>();
    let value = stream
        .next()
        .unwrap_or_else(|| panic!("review prompt section `{title}` carries no JSON"))
        .unwrap_or_else(|error| panic!("section `{title}` JSON parse failed: {error}"));
    let consumed = stream.byte_offset();
    (&rest[..consumed].trim_end(), value)
}

/// D1（REQ-WSC-06）单段验收三件事：解析对象与期望 deep-equal（键、值、
/// 数组顺序、refs 全保真）；载荷单行无缩进（真 compact）；同一对象的
/// pretty 重序列化字节数严格大于载荷（字节严格下降的逐段形态；只保证
/// 字节下降，不承诺 token 同比或质量必升）。
fn assert_section_compact_deep_equal(prompt: &str, title: &str, expected: &serde_json::Value) {
    let (payload, value) = compact_section_payload(prompt, title);
    assert_eq!(
        &value, expected,
        "section `{title}` must stay deep-equal after compact serialization"
    );
    assert!(
        !payload.contains('\n'),
        "section `{title}` must be single-line compact JSON, got: {payload}"
    );
    let pretty = serde_json::to_string_pretty(&value).expect("pretty baseline");
    assert!(
        payload.len() < pretty.len(),
        "section `{title}` compact {} bytes must be strictly smaller than pretty {} bytes",
        payload.len(),
        pretty.len()
    );
}

/// 测试侧独立镜像 SC 投影的候选映射（不经被测序列化路径）。
fn expected_contract_candidate(
    target_repository_id: &str,
    contract: &crate::product::work_item_contract::CanonicalWorkItemContract,
) -> serde_json::Value {
    serde_json::json!({
        "target_repository_id": target_repository_id,
        "identity": serde_json::to_value(&contract.identity).expect("identity value"),
        "goal": serde_json::to_value(&contract.goal).expect("goal value"),
        "tasks": serde_json::to_value(&contract.tasks).expect("tasks value"),
        "write_policy": serde_json::to_value(&contract.write_policy).expect("write policy value"),
        "acceptance_criteria": serde_json::to_value(&contract.acceptance_criteria).expect("acceptance criteria value"),
        "verification_checks": serde_json::to_value(&contract.verification_checks).expect("verification checks value"),
        "depends_on": serde_json::to_value(&contract.depends_on).expect("depends_on value"),
        "input_contracts": serde_json::to_value(&contract.input_contracts).expect("input contracts value"),
        "output_contracts": serde_json::to_value(&contract.output_contracts).expect("output contracts value"),
    })
}

/// Cross-Item 供需投影的测试侧镜像（值完整保留，D1 不删语义内容）。
fn expected_cross_item_supply_demand(
    contracts: &[crate::product::work_item_contract::CanonicalWorkItemContract],
) -> serde_json::Value {
    serde_json::json!({
        "supplies": contracts.iter().map(|contract| serde_json::json!({
            "work_item_id": contract.identity.logical_work_item_id.as_str(),
            "provided_output_contracts": serde_json::to_value(&contract.output_contracts).expect("output contracts value"),
        })).collect::<Vec<_>>(),
        "demands": contracts.iter().map(|contract| serde_json::json!({
            "work_item_id": contract.identity.logical_work_item_id.as_str(),
            "depends_on": serde_json::to_value(&contract.depends_on).expect("depends_on value"),
            "required_input_contracts": serde_json::to_value(&contract.input_contracts).expect("input contracts value"),
        })).collect::<Vec<_>>(),
    })
}

/// 覆盖投影期望：用生产侧确定性投影函数对 fixture 契约重算（同输入同口径）。
fn expected_reviewer_coverage(
    contracts: &[crate::product::work_item_contract::CanonicalWorkItemContract],
) -> serde_json::Value {
    let graph = crate::product::work_item_contract::build_dependency_contract_graph(contracts)
        .expect("dependency contract graph");
    serde_json::to_value(
        crate::product::work_item_contract::project_contract_reviewer_coverage(&graph),
    )
    .expect("coverage projection value")
}
#[test]
fn single_candidate_review_prompt_reads_compiled_ir_and_mechanical_report() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::super::make_work_item_plan_engine_with_draft_candidate(
            "single_candidate_review_artifacts",
        );
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source = "# immutable single candidate review source\\n";
    let mut source_revision = SourceRevisionRecord {
        id: "source-review".to_string(),
        source: source.to_string(),
        source_revision_hash: source_hash(source),
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
                contract: contract_a.clone(),
                verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                    checks: Vec::new(),
                },
                trusted_commands: Vec::new(),
            },
            PlanCandidateItemIr {
                target_repository_id: "repository_0001".to_string(),
                contract: contract_b.clone(),
                verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                    checks: Vec::new(),
                },
                trusted_commands: Vec::new(),
            },
        ],
    };
    let mut ir_record = PlanCandidateIrRecord {
        id: "ir-review".to_string(),
        source_revision_id: source_revision.id.clone(),
        ir,
        content_hash: String::new(),
    };
    ir_record.content_hash = ir_record.content_hash().expect("IR content hash");
    let ir_ref = source_store
        .put_plan_candidate_ir("project_0001", "issue_0001", &plan_id, &ir_record)
        .expect("persist compiled IR");
    let mut report = PlanCandidateMechanicalReportRecord {
        id: "report-review".to_string(),
        source_revision_id: source_revision.id,
        ir_id: ir_record.id,
        report: PlanCandidateMechanicalReport {
            source_revision_hash: ir_record.ir.source_revision_hash.clone(),
            compiler_version: ir_record.ir.compiler_version.clone(),
            findings: vec![WorkItemSplitFinding {
                severity: crate::product::models::WorkItemSplitFindingSeverity::Warning,
                code: "review_warning".to_string(),
                message: "mechanical summary evidence".to_string(),
                work_item_ids: vec!["wi-b".to_string()],
                // F1.2：fixture 沿旧 record 形态（kind=None）。
                kind: None,
            }],
        },
        // SG3a.0：旧 fixture 形态（无结构报告）。
        structure_report: None,
        content_hash: String::new(),
    };
    report.content_hash = report.content_hash().expect("report content hash");
    let report_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", &plan_id, &report)
        .expect("persist mechanical report");

    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.plan_candidate_ir_ref = Some(ir_ref);
    record.mechanical_report_ref = Some(report_ref);
    record.work_item_plan_source_revision_ref = Some(format!(
        "project/project_0001/issue/issue_0001/plan/{plan_id}/source_revision/source-review"
    ));
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist session refs");
    engine.session = crate::product::workspace_engine::types::WorkspaceSession::from_record(record);
    engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "compiled markdown is not authoritative".to_string(),
        diff: None,
    });

    let input = engine
        .build_work_item_plan_review_input()
        .expect("single-candidate review input");
    // 批2 Task 11（T20 前中间态）：Reviewer wire 面全开（无 deny）——
    // 纪律=ReviewerVerify+窗口合同，真窗归 T20。
    assert_eq!(
        input.role,
        crate::protocol::contracts::AdapterRole::Reviewer
    );
    assert!(input.tool_policy.is_none());
    assert!(input.prompt.contains("wi-a"));
    assert!(
        input
            .prompt
            .contains("Provide the canonical work item contract")
    );
    for required_field in [
        "tasks",
        "write_policy",
        "acceptance_criteria",
        "verification_checks",
        "depends_on",
        "contract.canonical",
        "contract.source",
    ] {
        assert!(
            input.prompt.contains(required_field),
            "single-candidate reviewer view must include {required_field}"
        );
    }
    assert!(input.prompt.contains("wi-a -> wi-b"));
    assert!(input.prompt.contains("mechanical summary evidence"));
    assert!(
        !input
            .prompt
            .contains("compiled markdown is not authoritative")
    );

    // D1（REQ-WSC-06「reviewer 无损紧凑序列化」）：SC 本体 JSON 段 compact 化后，
    // 逐段解析 deep-equal（键、值、数组顺序、refs 保真）+ 单行 + 字节严格小于
    // 同一对象的 pretty 形态；该 fixture 覆盖六段（含非空 findings 的报告段）。
    let ir_ref_now = engine
        .session
        .plan_candidate_ir_ref
        .as_deref()
        .expect("ir ref");
    let report_ref_now = engine
        .session
        .mechanical_report_ref
        .as_deref()
        .expect("report ref");
    let contracts = [contract_a.clone(), contract_b.clone()];
    assert_section_compact_deep_equal(
        &input.prompt,
        "Canonical Contract Candidates",
        &serde_json::json!([
            expected_contract_candidate("repository_0001", &contract_a),
            expected_contract_candidate("repository_0001", &contract_b),
        ]),
    );
    assert_section_compact_deep_equal(
        &input.prompt,
        "Dependency Contract Graph",
        &serde_json::json!({
            "topological_order": ["wi-a", "wi-b"],
            "edges": ["wi-a -> wi-b"],
        }),
    );
    assert_section_compact_deep_equal(
        &input.prompt,
        "Reviewer Capability Coverage Projection",
        &expected_reviewer_coverage(&contracts),
    );
    assert_section_compact_deep_equal(
        &input.prompt,
        "Cross-Item Contract Supply / Demand",
        &expected_cross_item_supply_demand(&contracts),
    );
    assert_section_compact_deep_equal(
        &input.prompt,
        "Projection Validation Report",
        &serde_json::json!({
            "source_revision_hash": ir_record.ir.source_revision_hash.as_str(),
            "compiler_version": ir_record.ir.compiler_version.as_str(),
            "summary": {
                "error_count": 0,
                "warning_count": 1,
            },
            "findings": serde_json::to_value(&report.report.findings).expect("findings value"),
        }),
    );
    assert_section_compact_deep_equal(
        &input.prompt,
        "Immutable Candidate Artifact Refs",
        &serde_json::json!({
            "plan_candidate_ir_ref": ir_ref_now,
            "mechanical_report_ref": report_ref_now,
        }),
    );
}

#[test]
fn single_candidate_review_prompt_fails_closed_when_artifact_refs_are_missing() {
    let (_tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
        super::super::make_work_item_plan_engine_with_draft_candidate(
            "single_candidate_review_missing_refs",
        );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.plan_candidate_ir_ref = None;
    record.mechanical_report_ref = None;
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist session");
    engine.session = crate::product::workspace_engine::types::WorkspaceSession::from_record(record);
    let error = engine
        .build_work_item_plan_review_input()
        .expect_err("missing refs must fail closed");
    assert!(error.contains("plan_candidate_ir_ref"));
    assert!(error.contains("mechanical_report_ref"));
}

/// D1（REQ-WSC-06）：compact 序列化不触碰绑定校验——session 指向的
/// durable IR 与机械报告绑定不匹配（report 绑定另一条合法 IR）仍拒发。
#[test]
fn single_candidate_review_prompt_refuses_durable_binding_mismatch() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::super::make_work_item_plan_engine_with_draft_candidate(
            "single_candidate_review_binding_mismatch",
        );
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source = "# single candidate binding mismatch source\n";
    let mut source_revision = SourceRevisionRecord {
        id: "source-binding".to_string(),
        source: source.to_string(),
        source_revision_hash: source_hash(source),
        content_hash: String::new(),
    };
    source_revision.content_hash = source_revision.content_hash().expect("source content hash");
    source_store
        .put_source_revision("project_0001", "issue_0001", &plan_id, &source_revision)
        .expect("persist source revision");
    let build_ir_record = |id: &str| {
        let mut record = PlanCandidateIrRecord {
            id: id.to_string(),
            source_revision_id: source_revision.id.clone(),
            ir: PlanCandidateIr {
                source_revision_hash: source_revision.source_revision_hash.clone(),
                compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
                items: vec![PlanCandidateItemIr {
                    target_repository_id: "repository_0001".to_string(),
                    contract: crate::product::work_item_contract::canonical_contract_fixture(
                        "wi-a",
                    ),
                    verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                        checks: Vec::new(),
                    },
                    trusted_commands: Vec::new(),
                }],
            },
            content_hash: String::new(),
        };
        record.content_hash = record.content_hash().expect("IR content hash");
        record
    };
    let ir_record = build_ir_record("ir-bound");
    let other_ir_record = build_ir_record("ir-other");
    let ir_ref = source_store
        .put_plan_candidate_ir("project_0001", "issue_0001", &plan_id, &ir_record)
        .expect("persist compiled IR");
    source_store
        .put_plan_candidate_ir("project_0001", "issue_0001", &plan_id, &other_ir_record)
        .expect("persist other compiled IR");
    let mut report = PlanCandidateMechanicalReportRecord {
        id: "report-binding".to_string(),
        source_revision_id: source_revision.id,
        ir_id: other_ir_record.id,
        report: PlanCandidateMechanicalReport {
            source_revision_hash: ir_record.ir.source_revision_hash.clone(),
            compiler_version: ir_record.ir.compiler_version.clone(),
            findings: Vec::new(),
        },
        // SG3a.0：旧 fixture 形态（无结构报告）。
        structure_report: None,
        content_hash: String::new(),
    };
    report.content_hash = report.content_hash().expect("report content hash");
    let report_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", &plan_id, &report)
        .expect("persist mechanical report");

    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.plan_candidate_ir_ref = Some(ir_ref);
    record.mechanical_report_ref = Some(report_ref);
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist session refs");
    engine.session = crate::product::workspace_engine::types::WorkspaceSession::from_record(record);

    let error = engine
        .build_work_item_plan_review_input()
        .expect_err("durable binding mismatch must refuse to emit");
    assert!(
        error.contains("bindings mismatch"),
        "typed stop must name the durable binding mismatch: {error}"
    );
}

#[test]
fn single_candidate_initial_prompt_is_derived_from_server_scope() {
    let scope = ReviewInvocationScope::initial("revision-001");
    let instructions = crate::product::workspace_engine::review_scope_instructions(&scope)
        .expect("initial scope instructions");

    assert!(instructions.contains("Initial"));
    assert!(instructions.contains("revision-001"));
    assert!(instructions.contains("一次全候选评估"));
    assert!(instructions.contains("每个 finding 对象只能包含以下字段：severity、message、evidence（可选）、required_action（可选）、category、class_hint、contract_field（可选）——不得添加 finding_id、code、work_item_ids 或其他字段"));
    for category in [
        ReviewFindingCategory::ContractGap,
        ReviewFindingCategory::SelfContradiction,
        ReviewFindingCategory::ScopeConflict,
        ReviewFindingCategory::VerificationUnattributable,
        ReviewFindingCategory::Completeness,
        ReviewFindingCategory::Other,
    ] {
        assert!(
            instructions.contains(category.as_str()),
            "initial review prompt must teach category whitelist value {}",
            category.as_str()
        );
    }
    assert!(instructions.contains("category 只能取以上六值之一；无法归类时用 other"));
    assert!(instructions.contains(
        "severity 只能取三值之一：blocking（阻断发布）、must_fix（必须修复）、suggestion（建议）——不得使用 error/warning 等其他词"
    ));
    for class_hint in ["repairable", "human_required", "advisory"] {
        assert!(
            instructions.contains(class_hint),
            "initial review prompt must teach class_hint value {class_hint}"
        );
    }
    assert!(instructions.contains(
        "class_hint 只能取三值之一：repairable（可自动返修）、human_required（需人工裁决）、advisory（仅建议）"
    ));
    assert!(instructions.contains("must_fix"));
    assert!(instructions.contains("机械漏网硬错误或明确自相矛盾"));
    assert!(instructions.contains("advisory"));
    assert!(instructions.contains(scope.scope_digest()));
    assert!(!instructions.contains("review_invocation_scope"));
}

#[test]
fn single_candidate_verification_prompt_replays_only_original_fingerprints() {
    let fingerprint = FindingFingerprint::for_finding(
        Some(crate::product::work_item_plan_policy::ReviewFindingCategory::ContractGap),
        crate::product::work_item_plan_policy::FindingClass::Repairable,
        "original",
        Some("contract.field"),
    );
    let scope = ReviewInvocationScope::verification(
        BTreeSet::from([fingerprint.clone()]),
        "revision-002",
        "project/issue/plan/mechanical_report/report-002",
        None,
    );
    let instructions = crate::product::workspace_engine::review_scope_instructions(&scope)
        .expect("verification scope instructions");

    assert!(instructions.contains("Verification"));
    assert!(instructions.contains("revision-002"));
    assert!(instructions.contains("mechanical_report"));
    assert!(instructions.contains(fingerprint.0.as_str()));
    for category in [
        ReviewFindingCategory::ContractGap,
        ReviewFindingCategory::SelfContradiction,
        ReviewFindingCategory::ScopeConflict,
        ReviewFindingCategory::VerificationUnattributable,
        ReviewFindingCategory::Completeness,
        ReviewFindingCategory::Other,
    ] {
        assert!(
            instructions.contains(category.as_str()),
            "verification review prompt must teach category whitelist value {}",
            category.as_str()
        );
    }
    assert!(instructions.contains("category 只能取以上六值之一；无法归类时用 other"));
    assert!(instructions.contains(
        "severity 只能取三值之一：blocking（阻断发布）、must_fix（必须修复）、suggestion（建议）——不得使用 error/warning 等其他词"
    ));
    for class_hint in ["repairable", "human_required", "advisory"] {
        assert!(
            instructions.contains(class_hint),
            "verification review prompt must teach class_hint value {class_hint}"
        );
    }
    assert!(instructions.contains(
        "class_hint 只能取三值之一：repairable（可自动返修）、human_required（需人工裁决）、advisory（仅建议）"
    ));
    assert!(instructions.contains("仅复核原 fingerprints"));
    assert!(instructions.contains("每个 finding 对象只能包含以下字段：severity、message、evidence（可选）、required_action（可选）、category、class_hint、contract_field（可选）——不得添加 finding_id、code、work_item_ids 或其他字段"));
    assert!(instructions.contains("机械漏网硬错误或明确自相矛盾"));
    assert!(instructions.contains("advisory"));
    assert!(instructions.contains(scope.scope_digest()));
}

#[test]
fn single_candidate_scope_instructions_reject_invalid_digest() {
    let mut value = serde_json::to_value(ReviewInvocationScope::initial("revision-001")).unwrap();
    value["scope_digest"] = serde_json::Value::String("review_scope_v1:invalid".to_string());
    let scope = serde_json::from_value::<ReviewInvocationScope>(value);
    assert!(scope.is_err());
}

#[test]
fn single_candidate_scope_instructions_reject_empty_verification_report() {
    let scope = ReviewInvocationScope::verification(BTreeSet::new(), "revision-002", "", None);
    let error = crate::product::workspace_engine::review_scope_instructions(&scope)
        .expect_err("missing mechanical report must be fatal");
    assert!(error.contains("mechanical report"));
}

// ===================== SG3a.1：实际 SC reviewer 输入的同代结构报告 =====================

mod sg3a1_structure_input {
    use super::*;
    use crate::product::planning_failure::{
        PersistedEffectivePlanningEdge, PersistedPlanningStructureReport,
        PersistedPlanningStructureSpan, PlanningEdgeSource, PlanningFailureSet,
    };
    use crate::product::work_item_plan_source_store::persisted_structure_report_digest;

    /// 构造「current refs → Some 结构报告」的 SingleCandidate 会话：source/IR
    /// 走真实 store 落盘，payload 的 context_digest/rule/compiler 身份取自
    /// `build_single_candidate_structure_context` 对 live 事实的单源重算，
    /// effective_edges 携一条两来源边（canonical_input + change_order）。
    /// `mutate_payload` 供旧 context/缺证变体改写身份后再封口。
    fn sg3a1_sealed_session(
        tag: &str,
        mutate_payload: impl FnOnce(&mut PersistedPlanningStructureReport),
    ) -> (
        tempfile::TempDir,
        crate::product::lifecycle_store::LifecycleStore,
        crate::product::workspace_engine::WorkspaceEngine,
        String,
    ) {
        let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
            super::super::super::make_work_item_plan_engine_with_draft_candidate(&format!(
                "sg3a1_{tag}"
            ));
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source = format!("# sg3a1 single candidate structure source {tag}\n");
        let mut source_revision = SourceRevisionRecord {
            id: format!("source-sg3a1-{tag}"),
            source: source.clone(),
            source_revision_hash: source_hash(&source),
            content_hash: String::new(),
        };
        source_revision.content_hash = source_revision.content_hash().expect("source content hash");
        let source_ref = source_store
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
            id: format!("ir-sg3a1-{tag}"),
            source_revision_id: source_revision.id.clone(),
            ir,
            content_hash: String::new(),
        };
        ir_record.content_hash = ir_record.content_hash().expect("IR content hash");
        let ir_ref = source_store
            .put_plan_candidate_ir("project_0001", "issue_0001", &plan_id, &ir_record)
            .expect("persist compiled IR");

        let context = engine
            .build_single_candidate_structure_context(&ir_record.ir, &source_ref, &ir_ref)
            .expect("live structure context");
        let mut payload = PersistedPlanningStructureReport {
            source_revision_id: source_revision.id.clone(),
            source_hash: source_revision.source_revision_hash.clone(),
            ir_id: ir_record.id.clone(),
            compiler_version: ir_record.ir.compiler_version.clone(),
            rule_version: context.rule_version.clone(),
            context_digest: context.context_digest.clone(),
            effective_edges: vec![PersistedEffectivePlanningEdge {
                from: "wi-a".to_string(),
                to: "wi-b".to_string(),
                sources: vec![
                    PlanningEdgeSource::CanonicalInput,
                    PlanningEdgeSource::ChangeOrder,
                ],
                contract_refs: vec!["contract.levels-api".to_string()],
                spans: vec![PersistedPlanningStructureSpan {
                    field: "input_contracts[0]".to_string(),
                    line: 96,
                }],
                order_indices: vec![0, 1],
            }],
            cycle_witness: Vec::new(),
            findings: PlanningFailureSet::default(),
            structurally_valid: true,
            protected_predicates: vec![
                "effective_graph_acyclic".to_string(),
                "canonical_inputs_preserved".to_string(),
                "acceptance_criteria_preserved".to_string(),
                "change_order_unchanged".to_string(),
            ],
            instruction_refs: Vec::new(),
            repair_audit_refs: Vec::new(),
            report_digest: String::new(),
        };
        mutate_payload(&mut payload);
        payload.report_digest =
            persisted_structure_report_digest(&payload).expect("seal structure report digest");

        let mut report = PlanCandidateMechanicalReportRecord {
            id: format!("report-sg3a1-{tag}"),
            source_revision_id: source_revision.id.clone(),
            ir_id: ir_record.id.clone(),
            report: PlanCandidateMechanicalReport {
                source_revision_hash: ir_record.ir.source_revision_hash.clone(),
                compiler_version: ir_record.ir.compiler_version.clone(),
                findings: Vec::new(),
            },
            structure_report: Some(payload),
            content_hash: String::new(),
        };
        report.content_hash = report.content_hash().expect("report content hash");
        let report_ref = source_store
            .put_mechanical_report("project_0001", "issue_0001", &plan_id, &report)
            .expect("persist mechanical report with structure payload");

        let mut record = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load session");
        record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
        record.work_item_plan_source_revision_ref = Some(source_ref);
        record.plan_candidate_ir_ref = Some(ir_ref);
        record.mechanical_report_ref = Some(report_ref.clone());
        write_json(
            &lifecycle
                .app_paths()
                .issue_root(&record.project_id, &record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist session refs");
        engine.session =
            crate::product::workspace_engine::types::WorkspaceSession::from_record(record);
        engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
            markdown: "compiled markdown is not authoritative".to_string(),
            diff: None,
        });
        (_tmp, lifecycle, engine, report_ref)
    }

    /// SG3a.1（plan Step1：`sg_03a_1_reviewer_consumes_same_generation_structure_report`）：
    /// 实际 builder 读取同一 mechanical report record 的 Some 结构报告，注入
    /// 三可信 section——两来源边、context/report identity、初轮空修复审计；
    /// IR/canonical/原 review 合同仍在，builder 前后 review 状态零变化。
    #[test]
    fn sg_03a_1_reviewer_consumes_same_generation_structure_report() {
        let (_tmp, _lifecycle, engine, report_ref) =
            sg3a1_sealed_session("same_generation", |_| {});
        let rounds_before = engine.session.review_rounds;
        let history_before = engine.session.run_history.clone();
        let scope_before = engine.session.review_invocation_scope.clone();

        let input = engine
            .build_work_item_plan_review_input()
            .expect("same-generation structure report must be consumed");

        // Deterministic Structure Report：两来源边 + 来源/契约/仓序索引。
        assert!(
            input.prompt.contains("### Deterministic Structure Report"),
            "missing structure section: {}",
            &input.prompt[input.prompt.len().saturating_sub(400)..]
        );
        assert!(input.prompt.contains("\"from\": \"wi-a\""));
        assert!(input.prompt.contains("\"to\": \"wi-b\""));
        assert!(
            input.prompt.contains("canonical_input"),
            "canonical input source must be visible"
        );
        assert!(
            input.prompt.contains("change_order"),
            "change order source must be visible on the same edge"
        );
        assert!(input.prompt.contains("contract.levels-api"));
        assert!(input.prompt.contains("\"order_indices\""));
        assert!(input.prompt.contains("input_contracts[0]"));
        assert!(input.prompt.contains("effective_graph_acyclic"));

        // Structure Context Identity：refs/hash/digest/rule 全部可见。
        assert!(input.prompt.contains("### Structure Context Identity"));
        let source_ref = engine
            .session
            .work_item_plan_source_revision_ref
            .clone()
            .expect("source ref");
        let ir_ref = engine
            .session
            .plan_candidate_ir_ref
            .clone()
            .expect("ir ref");
        assert!(input.prompt.contains(&source_ref));
        assert!(input.prompt.contains(&ir_ref));
        assert!(input.prompt.contains(&report_ref));
        let record_payload_digest = {
            let source_store = WorkItemPlanSourceStore::new(_lifecycle.app_paths());
            let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
                project_id: engine.session.project_id.clone(),
                issue_id: engine.session.issue_id.clone(),
                plan_id: engine.session.entity_id.clone(),
            };
            source_store
                .get_mechanical_report(&scope, &report_ref)
                .expect("reload record")
                .structure_report
                .expect("payload")
                .report_digest
        };
        assert!(
            input.prompt.contains(&record_payload_digest),
            "report_digest identity must be visible"
        );
        assert!(
            input.prompt.contains(
                crate::product::workspace_engine::plan_projection::PLANNING_STRUCTURE_RULE_VERSION
            ),
            "rule version identity must be visible"
        );
        // source_hash 身份可见（payload 冻结的候选 source 摘要）。
        assert!(input.prompt.contains("source_hash"));
        assert!(input.prompt.contains("context_digest"));
        assert!(input.prompt.contains("report_digest"));

        // Structural Repair Audit：初轮无修复明确空，不伪模型许可。
        assert!(input.prompt.contains("### Structural Repair Audit"));
        assert!(input.prompt.contains("instruction_refs"));
        assert!(input.prompt.contains("初轮无修复"));

        // IR/canonical/原 review 合同仍在。
        assert!(input.prompt.contains("wi-a"));
        assert!(
            input
                .prompt
                .contains("Provide the canonical work item contract")
        );
        assert!(input.prompt.contains("### Dependency Contract Graph"));
        assert!(input.prompt.contains("### Projection Validation Report"));
        assert!(input.structured_output_contract.is_some());
        assert_eq!(
            input.role,
            crate::protocol::contracts::AdapterRole::Reviewer
        );
        // 批2 Task 11（T20 前中间态）：Reviewer wire 面全开（无 deny）。
        assert!(input.tool_policy.is_none());
        assert!(
            !input
                .prompt
                .contains("compiled markdown is not authoritative")
        );

        // 96KiB 完整输入预算：真实 prompt 未超；超限输入仍被拒。
        assert!(input.prompt.len() <= 96 * 1024);
        assert!(
            crate::product::workspace_engine::prompts::ensure_single_candidate_review_prompt_budget(
                &"x".repeat(96 * 1024 + 1)
            )
            .is_err()
        );
        assert!(
            crate::product::workspace_engine::prompts::ensure_single_candidate_review_prompt_budget(
                &input.prompt
            )
            .is_ok()
        );

        // builder 前后 review 状态零变化。
        assert_eq!(engine.session.review_rounds, rounds_before);
        assert_eq!(engine.session.run_history, history_before);
        assert_eq!(engine.session.review_invocation_scope, scope_before);
    }

    /// SG3a.1（plan Step1：`sg_03a_1_review_rejects_stale_missing_structure_context`）：
    /// 旧 context（live 事实漂移）/rule 漂移/缺 context digest 一律拒发；
    /// 另一个合法报告不能补「合法」——session 指向的记录才是唯一 join 面。
    #[test]
    fn sg_03a_1_review_rejects_stale_missing_structure_context() {
        // —— 缺证：payload 无 context digest（missing）——
        let (_tmp1, _lc1, engine1, _ref1) = sg3a1_sealed_session("missing_digest", |payload| {
            payload.context_digest = String::new();
        });
        let error = engine1
            .build_work_item_plan_review_input()
            .expect_err("missing structure context digest must refuse to issue");
        assert!(
            error.contains("structure context"),
            "typed stop must name the structure context: {error}"
        );

        // —— rule 漂移：payload 冻结在旧规则版本 ——
        let (_tmp2, _lc2, engine2, _ref2) = sg3a1_sealed_session("stale_rule", |payload| {
            payload.rule_version = "rule/planning-structure@0".to_string();
        });
        let error = engine2
            .build_work_item_plan_review_input()
            .expect_err("stale structure rule version must refuse to issue");
        assert!(
            error.contains("rule"),
            "typed stop must name the rule drift: {error}"
        );

        // —— 旧 context：freeze 后 live plan options 漂移 ——
        let (_tmp3, lifecycle3, engine3, stale_ref) = sg3a1_sealed_session("stale_context", |_| {});
        let rounds_before_drift = engine3.session.review_rounds;
        let scope_before_drift = engine3.session.review_invocation_scope.clone();
        // 另造一个「当前 context」的合法报告 B——session 指向 A 时不得被
        // 用来补「合法」（不用另一个报告补合法）。
        let (_tmp_b, _lc_b, _engine_b, fresh_ref) = sg3a1_sealed_session("fresh_report", |_| {});
        let _ = fresh_ref;
        {
            let mut plan = lifecycle3
                .get_issue_work_item_plan("project_0001", "issue_0001", "issue_work_item_plan_0001")
                .expect("plan record");
            plan.options.include_integration_tests = true;
            write_json(
                &lifecycle3
                    .issue_work_item_plans_root("project_0001", "issue_0001")
                    .join("issue_work_item_plan_0001.json"),
                &plan,
            )
            .expect("persist drifted plan options");
        }
        let error = engine3
            .build_work_item_plan_review_input()
            .expect_err("stale live structure context must refuse to issue");
        assert!(
            error.contains("stale"),
            "typed stop must name the stale context: {error}"
        );
        assert!(
            error.contains("context_digest"),
            "typed stop must carry the digest mismatch evidence: {error}"
        );
        assert_ne!(stale_ref, "");
        // 拒发不产生任何 review 状态变化。
        assert_eq!(engine3.session.review_rounds, rounds_before_drift);
        assert_eq!(engine3.session.review_invocation_scope, scope_before_drift);
    }

    /// D1（REQ-WSC-06「reviewer 无损紧凑序列化」）：同一真实 IR/report/
    /// structure fixture 的**全部 9 段**投影——逐段 deep-equal + 单行 +
    /// 字节严格小于同一对象 pretty 形态；段顺序稳定；标题、refs、
    /// nonce/scope 事实与结构修复审计说明完整；总 prompt 仍在 96KiB
    /// 既有上限内（compact 只降字节，不承诺 token 同比或质量必升）。
    #[test]
    fn sg_03a_1_review_prompt_sections_compact_deep_equal_and_smaller() {
        let (_tmp, lifecycle, engine, report_ref) =
            sg3a1_sealed_session("d1_compact", |_| {});
        let input = engine
            .build_work_item_plan_review_input()
            .expect("compact single-candidate review input");

        let source_ref = engine
            .session
            .work_item_plan_source_revision_ref
            .clone()
            .expect("source ref");
        let ir_ref = engine
            .session
            .plan_candidate_ir_ref
            .clone()
            .expect("ir ref");
        let (ir_record, payload) = {
            let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
            let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
                project_id: engine.session.project_id.clone(),
                issue_id: engine.session.issue_id.clone(),
                plan_id: engine.session.entity_id.clone(),
            };
            let ir_record = source_store
                .get_plan_candidate_ir(&scope, &ir_ref)
                .expect("reload IR record");
            let payload = source_store
                .get_mechanical_report(&scope, &report_ref)
                .expect("reload report record")
                .structure_report
                .expect("structure payload");
            (ir_record, payload)
        };

        // 本体六段（从同一 durable IR/report 重载的期望值镜像）。
        let contracts = ir_record
            .ir
            .items
            .iter()
            .map(|item| item.contract.clone())
            .collect::<Vec<_>>();
        assert_section_compact_deep_equal(
            &input.prompt,
            "Canonical Contract Candidates",
            &serde_json::json!(ir_record
                .ir
                .items
                .iter()
                .map(|item| expected_contract_candidate(&item.target_repository_id, &item.contract))
                .collect::<Vec<_>>()),
        );
        assert_section_compact_deep_equal(
            &input.prompt,
            "Dependency Contract Graph",
            &serde_json::json!({
                "topological_order": ["wi-a", "wi-b"],
                "edges": ["wi-a -> wi-b"],
            }),
        );
        assert_section_compact_deep_equal(
            &input.prompt,
            "Reviewer Capability Coverage Projection",
            &expected_reviewer_coverage(&contracts),
        );
        assert_section_compact_deep_equal(
            &input.prompt,
            "Cross-Item Contract Supply / Demand",
            &expected_cross_item_supply_demand(&contracts),
        );
        assert_section_compact_deep_equal(
            &input.prompt,
            "Projection Validation Report",
            &serde_json::json!({
                "source_revision_hash": ir_record.ir.source_revision_hash.as_str(),
                "compiler_version": ir_record.ir.compiler_version.as_str(),
                "summary": {
                    "error_count": 0,
                    "warning_count": 0,
                },
                "findings": serde_json::Value::Array(Vec::new()),
            }),
        );
        assert_section_compact_deep_equal(
            &input.prompt,
            "Immutable Candidate Artifact Refs",
            &serde_json::json!({
                "plan_candidate_ir_ref": ir_ref.as_str(),
                "mechanical_report_ref": report_ref.as_str(),
            }),
        );

        // 结构三段（同一 record 的 Some 结构 payload）。
        assert_section_compact_deep_equal(
            &input.prompt,
            "Deterministic Structure Report",
            &serde_json::json!({
                "effective_edges": serde_json::to_value(&payload.effective_edges).expect("effective edges value"),
                "cycle_witness": serde_json::to_value(&payload.cycle_witness).expect("cycle witness value"),
                "findings": serde_json::to_value(&payload.findings).expect("structure findings value"),
                "structurally_valid": payload.structurally_valid,
                "protected_predicates": serde_json::to_value(&payload.protected_predicates).expect("protected predicates value"),
            }),
        );
        assert_section_compact_deep_equal(
            &input.prompt,
            "Structure Context Identity",
            &serde_json::json!({
                "source_revision_ref": source_ref.as_str(),
                "plan_candidate_ir_ref": ir_ref.as_str(),
                "mechanical_report_ref": report_ref.as_str(),
                "source_hash": payload.source_hash.as_str(),
                "context_digest": payload.context_digest.as_str(),
                "rule_version": payload.rule_version.as_str(),
                "report_digest": payload.report_digest.as_str(),
            }),
        );
        assert_section_compact_deep_equal(
            &input.prompt,
            "Structural Repair Audit",
            &serde_json::json!({
                "instruction_refs": serde_json::to_value(&payload.instruction_refs).expect("instruction refs value"),
                "repair_audit_refs": serde_json::to_value(&payload.repair_audit_refs).expect("repair audit refs value"),
            }),
        );

        // 9 段顺序稳定（本体六段在前、结构三段在后）。
        let mut cursor = 0usize;
        for title in [
            "Canonical Contract Candidates",
            "Dependency Contract Graph",
            "Reviewer Capability Coverage Projection",
            "Cross-Item Contract Supply / Demand",
            "Projection Validation Report",
            "Immutable Candidate Artifact Refs",
            "Deterministic Structure Report",
            "Structure Context Identity",
            "Structural Repair Audit",
        ] {
            let position = input
                .prompt
                .find(&format!("\n### {title}\n"))
                .unwrap_or_else(|| panic!("section `{title}` missing"));
            assert!(
                position >= cursor,
                "section order must stay stable at `{title}`"
            );
            cursor = position;
        }

        // 标题、refs、nonce/scope 事实与结构修复审计说明完整在场。
        assert!(input.prompt.contains("## Plan Review Context"));
        assert!(input.prompt.contains("结构修复审计说明"));
        assert!(input.prompt.contains("[review_repetition_discipline]"));
        assert!(input.prompt.contains("审核边界"));
        assert!(input.prompt.contains(&report_ref));
        let contract = input
            .structured_output_contract
            .as_ref()
            .expect("structured output contract");
        assert!(
            input.prompt.contains(contract.nonce.as_str()),
            "nonce fact must stay in prompt"
        );
        assert!(
            input.prompt.contains("\"review_scope\":\"outline\""),
            "scope fact must stay in schema"
        );

        // 预算：compact 后仍在 96KiB 既有上限内。
        assert!(input.prompt.len() <= 96 * 1024);
        assert!(
            crate::product::workspace_engine::prompts::ensure_single_candidate_review_prompt_budget(
                &input.prompt
            )
            .is_ok()
        );
    }
}
