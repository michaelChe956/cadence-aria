// single_candidate_prompt 拆分（large_file_guard 1200 行上限）：review prompt
// 读取/校验测试从 mod.rs 移入；测试逻辑与断言零改动（super:: 辅助函数
// 改经 super::super:: 指向 tests 作用域）。

use super::*;

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
            }],
        },
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
    // F3 restrict-role-write-tools：single-candidate 分派分支也必须带 D2 角色×策略对。
    assert_eq!(
        input.role,
        crate::protocol::contracts::AdapterRole::Reviewer
    );
    assert!(input.tool_policy.is_some());
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

/// REQ-TOP-04 场景 6（F-52 L2 前轮注入）：复评轮 prompt 必须携带上一
/// ReviewerRun 持久化 verdict 的非 advisory findings 结构化身份清单
/// （fingerprint/category/contract_field/required_action）+ 当前候选 ref，
/// 并教学「同一 fingerprint 重现须显式标注 repeated」；首轮（无前轮
/// verdict）不注入该段。
#[test]
fn single_candidate_review_prompt_injects_previous_round_finding_identities() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::super::make_work_item_plan_engine_with_draft_candidate("previous_round_injection");
    let (ir_ref, report_ref) = super::persist_verification_artifacts(&lifecycle, &plan_id);
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.plan_candidate_ir_ref = Some(ir_ref.clone());
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
    engine.active_node_id = Some("reviewer-current".to_string());
    engine.timeline_nodes.push(TimelineNode {
        node_id: "reviewer-current".to_string(),
        node_type: TimelineNodeType::ReviewerRun,
        agent: None,
        stage: WsWorkspaceStage::CrossReview,
        round: Some(2),
        status: TimelineNodeStatus::Active,
        title: "reviewer run".to_string(),
        summary: None,
        started_at: "2026-09-25T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: crate::product::models::ProviderName::ClaudeCode,
            reviewer: Some(crate::product::models::ProviderName::KimiCode),
            review_rounds: 2,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    });

    // 首轮形态（无前轮 verdict）：不注入前轮 findings 段。
    let first_round = engine
        .build_work_item_plan_review_input()
        .expect("first-round review input");
    assert!(
        !first_round
            .prompt
            .contains("Previous Review Round Findings"),
        "the initial round must not carry a previous-round section"
    );

    // 上一轮 ReviewerRun 节点 + durable verdict。
    let previous_finding = ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: "coverage 缺口：上游未提供 capability".to_string(),
        evidence: "edge wi-a -> wi-b".to_string(),
        required_action: "补齐 capability 声明".to_string(),
        category: Some(ReviewFindingCategory::ContractGap),
        class_hint: None,
        contract_field: Some("WI-001.output_contracts[CT-001].capabilities".to_string()),
    };
    let previous_verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "previous round revise".to_string(),
        summary: "previous round revise".to_string(),
        findings: vec![previous_finding],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    let previous_node_id = "reviewer-previous".to_string();
    engine.timeline_nodes.insert(
        0,
        TimelineNode {
            node_id: previous_node_id.clone(),
            node_type: TimelineNodeType::ReviewerRun,
            agent: None,
            stage: WsWorkspaceStage::CrossReview,
            round: Some(1),
            status: TimelineNodeStatus::Completed,
            title: "reviewer run".to_string(),
            summary: None,
            started_at: "2026-09-25T00:00:00Z".to_string(),
            completed_at: Some("2026-09-25T00:01:00Z".to_string()),
            duration_ms: None,
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: crate::product::models::ProviderName::ClaudeCode,
                reviewer: Some(crate::product::models::ProviderName::KimiCode),
                review_rounds: 2,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            retry: None,
        },
    );
    lifecycle
        .save_node_detail(
            &engine.session().session_id,
            &previous_node_id.clone(),
            &crate::product::models::NodeDetail {
                node_id: previous_node_id,
                session_id: engine.session().session_id.clone(),
                node_type: TimelineNodeType::ReviewerRun,
                status: TimelineNodeStatus::Completed,
                agent_role: None,
                provider: None,
                prompt: None,
                messages: Vec::new(),
                streaming_content: String::new(),
                execution_events: Vec::new(),
                permission_events: Vec::new(),
                verdict: Some(serde_json::to_value(&previous_verdict).expect("verdict json")),
                artifact_ref: None,
                is_revision: false,
                revision_feedback: None,
                base_artifact_ref: None,
                started_at: "2026-09-25T00:00:00Z".to_string(),
                ended_at: None,
            },
        )
        .expect("persist previous reviewer verdict");

    let replay_round = engine
        .build_work_item_plan_review_input()
        .expect("replay review input");
    let (fingerprint, unstable) = FindingFingerprint::identity_for_finding(
        Some(ReviewFindingCategory::ContractGap),
        FindingClass::Repairable,
        "coverage 缺口：上游未提供 capability",
        Some("WI-001.output_contracts[CT-001].capabilities"),
    );
    assert!(!unstable, "fixture must sit in the stable identity domain");
    assert!(
        replay_round
            .prompt
            .contains("Previous Review Round Findings")
    );
    assert!(
        replay_round.prompt.contains(fingerprint.0.as_str()),
        "prompt must carry the previous-round finding fingerprint identity"
    );
    assert!(replay_round.prompt.contains("contract_gap"));
    assert!(
        replay_round
            .prompt
            .contains("WI-001.output_contracts[CT-001].capabilities")
    );
    assert!(replay_round.prompt.contains("补齐 capability 声明"));
    assert!(
        replay_round.prompt.contains(&ir_ref),
        "prompt must name the current plan_candidate_ir_ref under review"
    );
    assert!(
        replay_round.prompt.contains("repeated"),
        "prompt must teach explicit repeated marking for recurring fingerprints"
    );
}
