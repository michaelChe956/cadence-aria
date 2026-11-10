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
    // D6 扩展：机械报告注入 code=intent_unexecutable ⇒ 前轮 finding（同内容
    // 身份）在本轮机械复算仍存在，应落待办区（required_action 仍须在场）。
    let (ir_ref, report_ref) = super::persist_verification_artifacts_with_findings(
        &lifecycle,
        &plan_id,
        vec![WorkItemSplitFinding {
            severity: crate::product::models::WorkItemSplitFindingSeverity::Warning,
            code: "intent_unexecutable".to_string(),
            message: "mechanical summary: intent target codebase unavailable".to_string(),
            work_item_ids: vec!["wi-a".to_string()],
            kind: None,
        }],
    );
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
        contract_field: Some("plan_preflight.intent_unexecutable".to_string()),
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
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
        Some("plan_preflight.intent_unexecutable"),
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
            .contains("plan_preflight.intent_unexecutable")
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
    // D6：前轮清单按内容身份求交分区注入——同码仍存在 ⇒ 待办区在场。
    assert!(
        replay_round.prompt.contains("unresolved_findings"),
        "partitioned previous-round payload must carry the unresolved zone"
    );
    assert!(
        replay_round.prompt.contains("resolved_findings"),
        "partitioned previous-round payload must carry the resolved zone"
    );
    assert!(
        replay_round
            .prompt
            .contains("previous_round_repetition_discipline"),
        "discipline tag must stay attached to the partitioned payload"
    );
}

// ===================== D6：前轮 finding 状态复算（内容身份求交 + 去重 + 分区注入） =====================

struct PreviousRoundReplay {
    // TempDir 必须存活到断言结束：lifecycle 全部落盘在它的 .aria 下。
    _tmp: tempfile::TempDir,
    engine: crate::product::workspace_engine::WorkspaceEngine,
    ir_ref: String,
    #[allow(dead_code)]
    report_ref: String,
}

fn mechanical_finding(code: &str, message: &str) -> WorkItemSplitFinding {
    WorkItemSplitFinding {
        severity: crate::product::models::WorkItemSplitFindingSeverity::Warning,
        code: code.to_string(),
        message: message.to_string(),
        work_item_ids: vec!["wi-a".to_string()],
        kind: None,
    }
}

fn previous_finding(
    message: &str,
    contract_field: Option<&str>,
    required_action: &str,
) -> ReviewFinding {
    ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: message.to_string(),
        evidence: "edge wi-a -> wi-b".to_string(),
        required_action: required_action.to_string(),
        category: Some(ReviewFindingCategory::ContractGap),
        class_hint: None,
        contract_field: contract_field.map(str::to_string),
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    }
}

/// 复评轮夹具：当前候选机械报告（findings 可注入）+ 上一轮 ReviewerRun
/// durable verdict；沿既有 `single_candidate_review_prompt_injects_previous_
/// round_finding_identities` 的搭建形态。
fn previous_round_replay(
    entity: &str,
    mechanical_findings: Vec<WorkItemSplitFinding>,
    previous_findings: Vec<ReviewFinding>,
) -> PreviousRoundReplay {
    let (tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::super::make_work_item_plan_engine_with_draft_candidate(entity);
    let (ir_ref, report_ref) = super::persist_verification_artifacts_with_findings(
        &lifecycle,
        &plan_id,
        mechanical_findings,
    );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.plan_candidate_ir_ref = Some(ir_ref.clone());
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

    let previous_verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "previous round revise".to_string(),
        summary: "previous round revise".to_string(),
        findings: previous_findings,
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
    PreviousRoundReplay {
        _tmp: tmp,
        engine,
        ir_ref,
        report_ref,
    }
}

/// 截取 `### Previous Review Round Findings` 段的 JSON 载荷（纪律段之前）。
fn previous_round_payload(text: &str) -> serde_json::Value {
    let section = text
        .split("### Previous Review Round Findings")
        .nth(1)
        .expect("text must carry the previous-round findings section");
    let discipline = section
        .find("[previous_round_repetition_discipline]")
        .expect("section must close with the repetition discipline tag");
    serde_json::from_str(section[..discipline].trim())
        .expect("previous-round payload must be valid JSON")
}

fn zone<'a>(payload: &'a serde_json::Value, key: &str) -> &'a Vec<serde_json::Value> {
    payload
        .get(key)
        .unwrap_or_else(|| panic!("payload must carry the `{key}` zone: {payload}"))
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` must be a list: {payload}"))
}

/// D6：本轮机械报告仍存在同内容身份 ⇒ 前轮 finding 落待办区（附
/// required_action 与 contract_field），不落已关闭区。
#[test]
fn previous_round_section_marks_unresolved_finding_as_pending() {
    let replay = previous_round_replay(
        "previous_round_unresolved",
        vec![mechanical_finding(
            "intent_unexecutable",
            "mechanical summary: intent target codebase unavailable",
        )],
        vec![previous_finding(
            "intent 不可执行：目标代码库 ID 未授权",
            Some("plan_preflight.intent_unexecutable"),
            "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
        )],
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let unresolved = zone(&payload, "unresolved_findings");
    assert_eq!(unresolved.len(), 1, "{payload}");
    assert_eq!(unresolved[0]["content_identity"], "intent_unexecutable");
    assert_eq!(
        unresolved[0]["contract_field"],
        "plan_preflight.intent_unexecutable"
    );
    assert_eq!(
        unresolved[0]["required_action"],
        "把 intent_target_codebase 改为已授权的逻辑代码库 ID"
    );
    assert_eq!(payload["mechanical_report_available"], true, "{payload}");
    assert!(
        zone(&payload, "resolved_findings").is_empty(),
        "仍存在的 finding 不得进已关闭区: {payload}"
    );
}

/// D6：本轮机械报告已无该内容身份 ⇒ 前轮 finding 落已关闭区（附
/// resolved_by），且该区不得携带 required_action（不得要求作者再改）。
#[test]
fn previous_round_section_marks_resolved_finding_as_closed() {
    let replay = previous_round_replay(
        "previous_round_resolved",
        Vec::new(),
        vec![previous_finding(
            "intent 不可执行：目标代码库 ID 未授权",
            Some("plan_preflight.intent_unexecutable"),
            "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
        )],
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let resolved = zone(&payload, "resolved_findings");
    assert_eq!(resolved.len(), 1, "{payload}");
    assert_eq!(resolved[0]["content_identity"], "intent_unexecutable");
    assert_eq!(
        resolved[0]["resolved_by"].as_str(),
        Some(replay.ir_ref.as_str()),
        "已关闭依据必须是当前候选 revision 标识: {payload}"
    );
    assert!(
        resolved[0].get("required_action").is_none(),
        "已关闭区条目 MUST NOT 携带可执行修改要求: {payload}"
    );
    assert!(
        zone(&payload, "unresolved_findings").is_empty(),
        "已消失的 finding 不得进待办区: {payload}"
    );
    assert!(
        !prompt.contains("把 intent_target_codebase 改为已授权的逻辑代码库 ID"),
        "已关闭 finding 的 required_action 不得以任何形式回灌 prompt"
    );
}

/// D6：前轮多条同内容身份（同一 preflight 码对多个 WI 各登记一次）⇒ 只保留
/// 一条，重复条目折叠为计数。
#[test]
fn previous_round_section_deduplicates_same_content_identity() {
    let previous = (1..=4)
        .map(|index| {
            previous_finding(
                &format!("WI-00{index} intent 不可执行：目标代码库 ID 未授权"),
                Some("plan_preflight.intent_unexecutable"),
                "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
            )
        })
        .collect();
    let replay = previous_round_replay(
        "previous_round_dedup",
        vec![mechanical_finding(
            "intent_unexecutable",
            "mechanical summary",
        )],
        previous,
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let unresolved = zone(&payload, "unresolved_findings");
    assert_eq!(
        unresolved.len(),
        1,
        "同一内容身份只保留首条，不得以多条重复条目呈现: {payload}"
    );
    assert_eq!(
        payload["duplicate_count"].as_u64(),
        Some(3),
        "4 条同身份前轮 finding ⇒ 1 条 + 折叠计数 3: {payload}"
    );
}

/// D6：求交键是内容身份（contract_field vs 机械报告 code），不是含 message
/// 的指纹——机械报告 message 与评审 message 天然不同。A：评审 message 带
/// `[code]` 前缀（≠ 机械 message）但同码 ⇒ 仍判待办；B：评审 message 与
/// 某机械 finding message 逐字相同但不同码 ⇒ 仍判已关闭。按 message/指纹
/// 求交的实现两条都会判错。
#[test]
fn previous_round_section_resolves_by_content_identity_not_fingerprint() {
    let replay = previous_round_replay(
        "previous_round_identity_not_fingerprint",
        vec![
            mechanical_finding("intent_unexecutable", "xxx"),
            mechanical_finding("unrelated_code", "yyy"),
        ],
        vec![
            previous_finding(
                "[intent_unexecutable] xxx",
                Some("plan_preflight.intent_unexecutable"),
                "修复 A",
            ),
            previous_finding("yyy", Some("plan_preflight.other_code"), "修复 B"),
        ],
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let unresolved = zone(&payload, "unresolved_findings");
    let resolved = zone(&payload, "resolved_findings");
    assert_eq!(
        unresolved.len(),
        1,
        "message 不同但内容身份相同 ⇒ 仍须判待办: {payload}"
    );
    assert_eq!(unresolved[0]["content_identity"], "intent_unexecutable");
    assert_eq!(
        resolved.len(),
        1,
        "message 逐字相同但内容身份不同 ⇒ 仍须判已关闭: {payload}"
    );
    assert_eq!(resolved[0]["content_identity"], "other_code");
}

/// D6 降级分支一：`contract_field` 缺失或无 `plan_preflight.` 前缀 ⇒ 不可机械
/// 判定——既不进待办也不进已关闭（不得当作已解决）。
#[test]
fn previous_round_section_skips_non_mechanically_judgeable_findings() {
    let replay = previous_round_replay(
        "previous_round_non_mechanical",
        vec![mechanical_finding(
            "intent_unexecutable",
            "mechanical summary",
        )],
        vec![
            previous_finding(
                "coverage 缺口：上游未提供 capability",
                Some("WI-001.output_contracts[CT-001].capabilities"),
                "补齐 capability 声明",
            ),
            previous_finding("无 contract_field 的 finding", None, "修复缺字段项"),
            previous_finding(
                "intent 不可执行：目标代码库 ID 未授权",
                Some("plan_preflight.intent_unexecutable"),
                "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
            ),
        ],
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let unresolved = zone(&payload, "unresolved_findings");
    let resolved = zone(&payload, "resolved_findings");
    assert_eq!(
        unresolved.len(),
        1,
        "只有带 plan_preflight. 前缀的可机械判定项能进待办: {payload}"
    );
    assert_eq!(unresolved[0]["content_identity"], "intent_unexecutable");
    assert!(
        resolved.is_empty(),
        "不可机械判定的 finding 不得被当作已解决: {payload}"
    );
}

/// D6 降级分支二：机械报告 ref 为 None 或装载失败 ⇒ 如实降级——
/// mechanical_report_available=false，前轮 finding 既不进待办也不进已关闭；
/// 不得 panic、不得当作「全部已解决」。
#[test]
fn previous_round_section_degrades_when_mechanical_report_unavailable() {
    let mut replay = previous_round_replay(
        "previous_round_report_unavailable",
        Vec::new(),
        vec![previous_finding(
            "intent 不可执行：目标代码库 ID 未授权",
            Some("plan_preflight.intent_unexecutable"),
            "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
        )],
    );
    let section_for = |engine: &crate::product::workspace_engine::WorkspaceEngine| {
        crate::product::workspace_engine::prompts::single_candidate_previous_round_findings_section(
            engine,
            &replay.ir_ref,
        )
        .expect("降级必须如实注入清单段（不 panic、不清空清单）")
    };
    for unavailable_ref in [None, Some("mechanical_report/does-not-resolve".to_string())] {
        replay.engine.session.mechanical_report_ref = unavailable_ref;
        let section = section_for(&replay.engine);
        let payload = previous_round_payload(&section);
        assert_eq!(
            payload["mechanical_report_available"], false,
            "机械报告不可得必须显式标注: {payload}"
        );
        assert!(
            zone(&payload, "unresolved_findings").is_empty(),
            "机械报告不可得 ⇒ 不得把前轮 finding 判为待办: {payload}"
        );
        assert!(
            zone(&payload, "resolved_findings").is_empty(),
            "机械报告不可得 ⇒ 不得静默当作「全部已解决」: {payload}"
        );
    }
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
}
