//! k3 独立双裁定的 P0-2B / P0-3 / REQ-WSC-14 锚点（后端平台裁决面）。
//!
//! 三件事必须在**有会话上下文**的地方被钉住（解析层拿不到 session）：
//!
//! 1. P0-2B：`finding_key ∈ 该 session 已处置清单` → 平台强制按 advisory 处理
//!    （不依赖模型听话）；撤销后恢复为 actionable。
//! 2. P0-3：读历史 review verdict 不得把投影身份 `finding_key` 回写 durable。
//! 3. REQ-WSC-14：拒收信息刷新后仍可读——`session_state` 全量投影携带最近
//!    一次拒收四要素。

use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::mpsc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use crate::product::models::{
    AgentRole, HumanDispositionRecord, HumanFindingDispositionKind, HumanGateTurnRejection,
    NodeDetail, ProviderName, WorkItemSplitFinding, WorkspaceType,
};
use crate::product::work_item_plan_policy::{
    FindingClass, FindingClassHint, ReviewFindingCategory, RunPolicy, WorkItemPlanFlowKind,
    classify_finding,
};
use crate::product::workspace_engine::{WorkspaceEngine, WorkspaceSession, session_state};
use crate::web::workspace_ws_types::{
    MachineRepairDto, MachineRepairStatus, OwnerOption, ProviderConfigSnapshot, ReviewFinding,
    ReviewFindingDisposition, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
    StructuredOutputDiagnostic, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage, WsOutMessage,
};

struct PlatformFixture {
    tmp: TempDir,
    store: LifecycleStore,
    record: crate::product::models::WorkspaceSessionRecord,
}

fn platform_fixture(entity_id: &str) -> PlatformFixture {
    platform_fixture_with(entity_id, WorkspaceType::WorkItemPlan, None)
}

/// B1：服务端可信会话 = WorkItemPlan + SingleCandidate 的夹具（窄例外只对
/// 这种会话生效；Legacy 流与其它 workspace type 走原语义）。
fn sc_platform_fixture(entity_id: &str) -> PlatformFixture {
    platform_fixture_with(
        entity_id,
        WorkspaceType::WorkItemPlan,
        Some(WorkItemPlanSessionOptions {
            flow_kind: WorkItemPlanFlowKind::SingleCandidate,
            run_policy: RunPolicy::Interactive,
            rollout_snapshot: true,
        }),
    )
}

fn platform_fixture_with(
    entity_id: &str,
    workspace_type: WorkspaceType,
    work_item_plan_options: Option<WorkItemPlanSessionOptions>,
) -> PlatformFixture {
    let tmp = TempDir::new().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let record = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: entity_id.to_string(),
            workspace_type,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options,
        })
        .expect("create workspace session");
    PlatformFixture { tmp, store, record }
}

impl PlatformFixture {
    fn engine(&self) -> WorkspaceEngine {
        let (event_tx, _event_rx) = mpsc::channel(8);
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(self.tmp.path().to_path_buf())),
            self.store.clone(),
            event_tx,
            WorkspaceSession::from_record(self.record.clone()),
        )
    }
}

fn finding(severity: ReviewFindingSeverity, message: &str) -> ReviewFinding {
    ReviewFinding {
        severity,
        message: message.to_string(),
        evidence: String::new(),
        required_action: "回写该事实".to_string(),
        category: None,
        class_hint: None,
        contract_field: None,
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    }
}

fn disposition(key: &str, summary: &str) -> HumanDispositionRecord {
    HumanDispositionRecord {
        finding_key: key.to_string(),
        message_summary: summary.to_string(),
        disposition: HumanFindingDispositionKind::Excluded,
        recorded_at: "2026-10-30T00:00:00Z".to_string(),
    }
}

/// P0-2B（REQ-WSC-14「平台裁决」）：模型照旧报 actionable，但 owner 已处置的
/// key 必须在平台侧被强制降级——不依赖模型听话；撤销后恢复可报。
#[tokio::test]
async fn owner_excluded_finding_is_platform_adjudicated_and_revocable() {
    let fixture = platform_fixture("plan_platform_adjudication");
    let session_id = fixture.record.id.clone();
    let gap = finding(ReviewFindingSeverity::MustFix, "中文 locale 尚未配置");
    let key = gap.computed_finding_key();
    fixture
        .store
        .record_human_dispositions(&session_id, &[disposition(&key, "中文 locale 尚未配置")])
        .unwrap();

    let mut engine = fixture.engine();
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: String::new(),
        summary: "范围外文档缺口".to_string(),
        findings: vec![gap.clone()],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };

    let normalized = engine.apply_session_owner_dispositions(verdict);
    assert_eq!(
        normalized.findings[0].disposition,
        Some(crate::web::workspace_ws_types::review::ReviewFindingDisposition::Advisory),
        "owner 已处置事实优先：模型报 actionable 也降级"
    );
    assert_eq!(normalized.verdict, ReviewVerdictType::Pass);
    assert_eq!(normalized.review_gate, ReviewGate::UserConfirmAllowed);
    // 平台裁决同样压过旧轴分类（否则指纹闸门仍会把它算进返修）。
    assert_eq!(
        classify_finding(normalized.verdict.clone(), &normalized.findings[0]).class,
        FindingClass::Advisory
    );

    // 撤销排除 → 该 finding 恢复可报为 actionable。
    engine
        .revoke_owner_finding_exclusions(std::slice::from_ref(&key))
        .await
        .expect("revoke exclusion");
    let restored = engine.apply_session_owner_dispositions(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: String::new(),
        summary: "范围外文档缺口".to_string(),
        findings: vec![gap],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    });
    assert!(restored.findings[0].is_actionable());
    assert_eq!(restored.verdict, ReviewVerdictType::Revise);
    assert_eq!(restored.review_gate, ReviewGate::RequiresRevision);
}

fn review_timeline_node(node_id: &str) -> TimelineNode {
    TimelineNode {
        node_id: node_id.to_string(),
        node_type: TimelineNodeType::ReviewerRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::Completed,
        round: Some(1),
        status: TimelineNodeStatus::Completed,
        title: "Reviewer".to_string(),
        summary: None,
        started_at: "2026-10-30T00:00:00Z".to_string(),
        completed_at: Some("2026-10-30T00:01:00Z".to_string()),
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    }
}

fn review_detail(session_id: &str, node_id: &str, verdict: serde_json::Value) -> NodeDetail {
    NodeDetail {
        node_id: node_id.to_string(),
        session_id: session_id.to_string(),
        node_type: TimelineNodeType::ReviewerRun,
        status: TimelineNodeStatus::Completed,
        agent_role: Some(AgentRole::Reviewer),
        provider: None,
        prompt: None,
        messages: Vec::new(),
        streaming_content: "review stream".to_string(),
        execution_events: Vec::new(),
        permission_events: Vec::new(),
        verdict: Some(verdict),
        artifact_ref: None,
        is_revision: false,
        revision_feedback: None,
        base_artifact_ref: None,
        started_at: "2026-10-30T00:00:00Z".to_string(),
        ended_at: Some("2026-10-30T00:01:00Z".to_string()),
    }
}

/// P0-3：读历史 review verdict 是**读**路径——反序列化按内容补出的
/// `finding_key` 是投影身份，MUST NOT 被回写进 durable（design §2.1/§4）。
#[test]
fn reading_historical_review_verdict_does_not_write_finding_key_to_durable() {
    let fixture = platform_fixture("plan_durable_no_rewrite");
    let session_id = fixture.record.id.clone();
    let node_id = "review_node_0001";
    let persisted = serde_json::json!({
        "verdict": "revise",
        "comments": "",
        "summary": "范围外文档缺口",
        "findings": [{
            "severity": "must_fix",
            "message": "中文 locale 尚未配置",
            "evidence": "docs/frontend.md",
            "required_action": "回写该事实"
        }],
        "review_gate": "requires_revision"
    });
    assert!(
        persisted["findings"][0].get("finding_key").is_none(),
        "夹具前提：durable verdict 不含投影身份"
    );
    fixture
        .store
        .save_node_detail(
            &session_id,
            node_id,
            &review_detail(&session_id, node_id, persisted.clone()),
        )
        .unwrap();
    let nodes = vec![review_timeline_node(node_id)];

    let verdict = session_state::latest_review_verdict_from_node_details(
        &fixture.store,
        &fixture.record.project_id,
        &fixture.record.issue_id,
        &session_id,
        &nodes,
    )
    .expect("historical verdict must be readable");
    assert!(
        verdict.findings[0].finding_key.is_some(),
        "读路径按内容现算 key（呈现/排除引用需要）"
    );

    let reloaded = fixture
        .store
        .load_node_detail(&session_id, node_id)
        .unwrap();
    assert_eq!(
        reloaded.verdict.as_ref(),
        Some(&persisted),
        "durable 字节零改写：finding_key 不得落盘"
    );
}

/// REQ-WSC-14：拒收信息刷新后仍可读——`build_session_state` 全量投影带上该
/// 会话最近一次拒收的四要素（否则换一次页面就只剩失败行）。
#[test]
fn latest_gate_turn_rejection_survives_session_state_projection() {
    let fixture = platform_fixture("plan_rejection_refresh");
    let session_id = fixture.record.id.clone();
    let rejection = HumanGateTurnRejection {
        turn_id: "turn_0001".to_string(),
        rejected_feedback: "计划里不写文档更新任务，本次只交付代码".to_string(),
        reason: "本轮反馈未通过平台校验，没有被采纳；门仍停在原处。".to_string(),
        next_actions: vec!["把反馈收窄成一条可执行的修改后重新提交".to_string()],
        diagnostics: vec![
            "STRUCTURAL_EXISTING_GATE: failure requires the original human gate".to_string(),
        ],
        recorded_at: "2026-10-30T00:00:00Z".to_string(),
    };
    fixture
        .store
        .save_human_gate_turn_rejection(&session_id, &rejection)
        .unwrap();

    let engine = fixture.engine();
    match engine.build_session_state() {
        WsOutMessage::SessionState {
            human_gate_turn_rejection: Some(dto),
            ..
        } => {
            assert_eq!(dto.turn_id, rejection.turn_id);
            assert_eq!(dto.rejected_feedback, rejection.rejected_feedback);
            assert_eq!(dto.rejection_reason, rejection.reason);
            assert_eq!(dto.rejection_next_actions, rejection.next_actions);
            assert_eq!(dto.rejection_diagnostics, rejection.diagnostics);
        }
        other => panic!("expected session_state with rejection, got {other:?}"),
    }

    // 从未拒收的会话：字段缺席（旧 wire 零变化）。
    let plain = platform_fixture("plan_rejection_absent");
    match plain.engine().build_session_state() {
        WsOutMessage::SessionState {
            human_gate_turn_rejection: None,
            candidate_compile_failure: None,
            ..
        } => {}
        other => panic!("expected session_state without rejection, got {other:?}"),
    }
}

/// REQ-WSC-13：下一轮 reviewer prompt 必须携带已处置事实——排除落 durable 后，
/// 生产读取点（reviewer prompt 段构造器）能读到同一批条目。
#[test]
fn next_review_prompt_section_carries_owner_dispositions() {
    let fixture = platform_fixture("plan_prompt_carries_dispositions");
    fixture
        .store
        .record_human_dispositions(
            &fixture.record.id,
            &[disposition("0123456789abcdef", "中文 locale 尚未配置")],
        )
        .unwrap();

    let engine = fixture.engine();
    let section = crate::product::workspace_engine::prompts::human_dispositions_section(&engine)
        .expect("非空清单必须注入 prompt 段");
    assert!(section.contains("[human_dispositions]"), "{section}");
    assert!(section.contains("0123456789abcdef"), "{section}");
    assert!(section.contains("中文 locale 尚未配置"), "{section}");
    assert!(section.contains("MUST NOT 再报为 actionable"), "{section}");
}

/// REQ-WSC-14（作者/评审输入面一致性）：上一轮 durable verdict 里的 owner 已处置
/// finding MUST NOT 再以「上一轮未收敛 finding」的身份注入复评 prompt——否则同一份
/// prompt 既说「这是上一轮未收敛项」又说「该事实已被 owner 排除」，自相矛盾。
#[test]
fn previous_round_section_omits_owner_disposed_findings() {
    let fixture = platform_fixture("plan_previous_round_disposed");
    let session_id = fixture.record.id.clone();
    // D6 扩展：finding 带 plan_preflight. 前缀 contract_field（可机械判定），
    // 并落一份当前候选机械报告——owner 排除口径在分区注入形态下继续成立。
    let mut disposed = finding(ReviewFindingSeverity::MustFix, "中文 locale 尚未配置");
    disposed.contract_field = Some("plan_preflight.locale_unavailable".to_string());
    let mut still_open = finding(ReviewFindingSeverity::MustFix, "本 Work Item 验收标准缺口");
    still_open.required_action = "补齐验收标准".to_string();
    still_open.contract_field = Some("plan_preflight.acceptance_missing".to_string());
    let key = disposed.computed_finding_key();
    fixture
        .store
        .record_human_dispositions(&session_id, &[disposition(&key, "中文 locale 尚未配置")])
        .unwrap();

    let previous_verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: String::new(),
        summary: "上一轮评审".to_string(),
        findings: vec![disposed, still_open],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    let previous_node_id = "reviewer-previous";
    fixture
        .store
        .save_node_detail(
            &session_id,
            previous_node_id,
            &review_detail(
                &session_id,
                previous_node_id,
                serde_json::to_value(&previous_verdict).expect("previous verdict json"),
            ),
        )
        .unwrap();
    let (_ir_ref, report_ref) =
        super::single_candidate_prompt::persist_verification_artifacts_with_findings(
            &fixture.store,
            &fixture.record.entity_id,
            vec![WorkItemSplitFinding {
                severity: crate::product::models::WorkItemSplitFindingSeverity::Warning,
                code: "acceptance_missing".to_string(),
                message: "mechanical summary: acceptance missing".to_string(),
                work_item_ids: vec!["wi-a".to_string()],
                kind: None,
            }],
        );

    let mut engine = fixture.engine();
    engine.session.mechanical_report_ref = Some(report_ref);
    engine.active_node_id = Some("reviewer-current".to_string());
    engine
        .timeline_nodes
        .push(review_timeline_node("reviewer-current"));
    engine
        .timeline_nodes
        .push(review_timeline_node(previous_node_id));

    let section =
        crate::product::workspace_engine::prompts::single_candidate_previous_round_findings_section(
            &engine,
            "ir_ref_0001",
        )
        .expect("未处置的上一轮 finding 仍必须注入");
    assert!(section.contains("补齐验收标准"), "{section}");
    assert_eq!(
        section.matches("\"fingerprint\"").count(),
        1,
        "只有未处置的 finding 能保留身份条目（待办/已关闭任一区都算）: {section}"
    );
    assert!(
        !section.contains("回写该事实"),
        "owner 已处置事实不得再以未收敛 finding 注入: {section}"
    );
    // D6 分区形态下的 owner 排除复核：未处置项落待办区；已处置项两个区都不在。
    let payload = previous_round_section_payload(&section);
    let unresolved = payload
        .get("unresolved_findings")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("unresolved_findings zone missing: {payload}"));
    assert_eq!(unresolved.len(), 1, "{payload}");
    assert_eq!(unresolved[0]["content_identity"], "acceptance_missing");
    assert!(
        !section.contains("locale_unavailable"),
        "owner 已处置项的内容身份不得出现在任一分区: {section}"
    );
}

/// 截取前轮清单段的 JSON 载荷（`### Previous Review Round Findings` 与纪律段之间）。
fn previous_round_section_payload(section: &str) -> serde_json::Value {
    let body = section
        .split("### Previous Review Round Findings")
        .nth(1)
        .expect("section heading present");
    let discipline = body
        .find("[previous_round_repetition_discipline]")
        .expect("discipline tag present");
    serde_json::from_str(body[..discipline].trim()).expect("payload JSON")
}

// ---------------------------------------------------------------------------
// B1 · SC 会话有效投影（REQ-WSC-12 窄例外）：severity=Suggestion、category
// 显式属完备度/验证不可归因/其它、class_hint=Repairable 的 finding 只在
// WorkItemPlan 单候选会话的**内存有效视图**里投影为 advisory；契约类 /
// 缺或未知 category / must_fix·blocking / 机械诊断 / 原生 NeedsHuman /
// 产品问句一律保持原语义，durable 不回写。
// ---------------------------------------------------------------------------

/// 窄例外命中形状的 finding（disposition 缺失 → fail-safe 视作 actionable）。
fn sc_narrow_finding(message: &str) -> ReviewFinding {
    let mut finding = finding(ReviewFindingSeverity::Suggestion, message);
    finding.class_hint = Some(FindingClassHint::Repairable);
    finding.category = Some(ReviewFindingCategory::Completeness);
    finding
}

fn sc_narrow_verdict() -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: String::new(),
        summary: "SC 会话有效投影".to_string(),
        findings: vec![sc_narrow_finding("建议补一个可选字段说明")],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

struct ScTruthRow {
    severity: ReviewFindingSeverity,
    class_hint: Option<FindingClassHint>,
    disposition: Option<ReviewFindingDisposition>,
    category: Option<ReviewFindingCategory>,
    verdict: ReviewVerdictType,
    structured_diagnostic: bool,
    machine_repair: bool,
    owner_question: bool,
    owner_options: bool,
    expect_advisory: bool,
}

/// 基线行 = 窄例外命中的全部条件（期望投影为 advisory）。
fn sc_truth_row() -> ScTruthRow {
    ScTruthRow {
        severity: ReviewFindingSeverity::Suggestion,
        class_hint: Some(FindingClassHint::Repairable),
        disposition: None,
        category: Some(ReviewFindingCategory::Completeness),
        verdict: ReviewVerdictType::Revise,
        structured_diagnostic: false,
        machine_repair: false,
        owner_question: false,
        owner_options: false,
        expect_advisory: true,
    }
}

fn tweaked(mut row: ScTruthRow, tweak: impl FnOnce(&mut ScTruthRow)) -> ScTruthRow {
    tweak(&mut row);
    row
}

/// 基线行被破坏任一条件 ⇒ 不期望投影（保持原语义）。
fn miss(row: ScTruthRow, tweak: impl FnOnce(&mut ScTruthRow)) -> ScTruthRow {
    let mut row = tweaked(row, tweak);
    row.expect_advisory = false;
    row
}

fn sc_truth_finding(row: &ScTruthRow) -> ReviewFinding {
    let mut finding = finding(row.severity.clone(), "建议补一个可选字段说明");
    finding.class_hint = row.class_hint;
    finding.category = row.category;
    finding.disposition = row.disposition;
    if row.machine_repair {
        finding.machine_repair = Some(MachineRepairDto {
            repair_id: "structural-repair/0123456789abcdef/x#0".to_string(),
            failure_id: "diag/ref-1#0".to_string(),
            source_hash: "a".repeat(64),
            context_digest: "b".repeat(64),
            status: MachineRepairStatus::Offered,
            instruction_ref: format!("structural-repair-instruction/{}", "c".repeat(64)),
        });
    }
    if row.owner_question {
        finding.owner_question = Some("要不要顺手补上这个说明？".to_string());
    }
    if row.owner_options {
        finding.owner_options = Some(vec![OwnerOption {
            label: "保持现状".to_string(),
            consequence: "不补字段说明".to_string(),
        }]);
    }
    finding
}

fn sc_structured_diagnostic() -> StructuredOutputDiagnostic {
    StructuredOutputDiagnostic {
        code: "invalid_finding_field".to_string(),
        message: "结构化输出不合规".to_string(),
        repair_attempted: false,
        repair_succeeded: false,
        raw_output_preview: None,
    }
}

/// REQ-WSC-12 真值表：severity × class_hint × disposition × category（含缺失）
/// × verdict × 有无 structured_output_diagnostic / machine_repair /
/// owner_question / owner_options——只有完整命中窄例外的行才投影为 advisory，
/// 其余一律保持原语义（缺 disposition 亦 fail-closed）。
#[tokio::test]
async fn sc_suggestion_truth_table_projects_only_narrow_slice() {
    let fixture = sc_platform_fixture("plan_sc_truth_table");
    let mut engine = fixture.engine();
    let rows = vec![
        sc_truth_row(),
        tweaked(sc_truth_row(), |row| {
            row.category = Some(ReviewFindingCategory::VerificationUnattributable);
        }),
        tweaked(sc_truth_row(), |row| {
            row.category = Some(ReviewFindingCategory::Other);
        }),
        tweaked(sc_truth_row(), |row| {
            row.disposition = Some(ReviewFindingDisposition::Actionable);
        }),
        tweaked(sc_truth_row(), |row| {
            row.disposition = Some(ReviewFindingDisposition::Advisory);
        }),
        tweaked(sc_truth_row(), |row| {
            row.verdict = ReviewVerdictType::Pass;
        }),
        miss(sc_truth_row(), |row| {
            row.category = Some(ReviewFindingCategory::ContractGap);
        }),
        miss(sc_truth_row(), |row| {
            row.category = Some(ReviewFindingCategory::SelfContradiction);
        }),
        miss(sc_truth_row(), |row| {
            row.category = Some(ReviewFindingCategory::ScopeConflict);
        }),
        miss(sc_truth_row(), |row| row.category = None),
        miss(sc_truth_row(), |row| {
            row.severity = ReviewFindingSeverity::MustFix;
        }),
        miss(sc_truth_row(), |row| {
            row.severity = ReviewFindingSeverity::Blocking;
        }),
        miss(sc_truth_row(), |row| {
            row.class_hint = Some(FindingClassHint::HumanRequired);
        }),
        miss(sc_truth_row(), |row| {
            row.class_hint = Some(FindingClassHint::Advisory);
        }),
        miss(sc_truth_row(), |row| row.class_hint = None),
        miss(sc_truth_row(), |row| {
            row.verdict = ReviewVerdictType::NeedsHuman;
        }),
        miss(sc_truth_row(), |row| row.structured_diagnostic = true),
        miss(sc_truth_row(), |row| row.machine_repair = true),
        miss(sc_truth_row(), |row| row.owner_question = true),
        miss(sc_truth_row(), |row| row.owner_options = true),
    ];

    for (index, row) in rows.into_iter().enumerate() {
        let row_severity = row.severity.clone();
        let row_verdict = row.verdict.clone();
        let finding = sc_truth_finding(&row);
        let identity = finding.computed_finding_key();
        let verdict = ReviewVerdict {
            verdict: row_verdict.clone(),
            comments: String::new(),
            summary: format!("真值表第 {index} 行"),
            findings: vec![finding],
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
            structured_output_diagnostic: row.structured_diagnostic.then(sc_structured_diagnostic),
        };
        let effective = engine
            .finalize_review_verdict(
                &format!("truth_row_{index}"),
                format!("row {index}"),
                verdict,
            )
            .await;
        let projected = &effective.findings[0];
        let expected_disposition = if row.expect_advisory {
            Some(ReviewFindingDisposition::Advisory)
        } else {
            row.disposition
        };
        assert_eq!(
            projected.disposition, expected_disposition,
            "真值表第 {index} 行：disposition 投影口径"
        );
        assert_eq!(
            projected.is_actionable(),
            !row.expect_advisory,
            "真值表第 {index} 行：actionable 口径"
        );
        // 投影只改 disposition：severity / message / action / 内容身份不动。
        assert_eq!(projected.severity, row_severity, "row {index}");
        assert_eq!(projected.message, "建议补一个可选字段说明", "row {index}");
        assert_eq!(projected.required_action, "回写该事实", "row {index}");
        assert_eq!(projected.computed_finding_key(), identity, "row {index}");
        let expected_verdict = match row_verdict {
            ReviewVerdictType::NeedsHuman => ReviewVerdictType::NeedsHuman,
            ReviewVerdictType::Pass => ReviewVerdictType::Pass,
            ReviewVerdictType::Revise
                if expected_disposition == Some(ReviewFindingDisposition::Advisory) =>
            {
                // 单条 finding 全非 actionable ⇒ 「revise 归一」回 pass，
                // 建议级条目不得单独驱作者返修。
                ReviewVerdictType::Pass
            }
            ReviewVerdictType::Revise => ReviewVerdictType::Revise,
        };
        assert_eq!(
            effective.verdict, expected_verdict,
            "真值表第 {index} 行：verdict 归一"
        );
        let expected_gate = if projected.is_actionable()
            && matches!(
                row.severity,
                ReviewFindingSeverity::Blocking | ReviewFindingSeverity::MustFix
            ) {
            ReviewGate::RequiresRevision
        } else {
            match expected_verdict {
                ReviewVerdictType::Pass => ReviewGate::UserConfirmAllowed,
                ReviewVerdictType::NeedsHuman => ReviewGate::UserTriageRequired,
                ReviewVerdictType::Revise => ReviewGate::UserConfirmAllowed,
            }
        };
        assert_eq!(
            effective.review_gate, expected_gate,
            "真值表第 {index} 行：review_gate"
        );
    }
}

/// REQ-WSC-12「live 与历史同源」+「MUST NOT 回写 durable」：live 在存原始
/// verdict 之后套会话有效视图；历史刷新在同一投影之后给出同一有效视图；
/// 新 advisory 两个入口都不落 durable。
#[tokio::test]
async fn sc_projection_live_and_historical_share_view_and_keep_durable_raw() {
    let fixture = sc_platform_fixture("plan_sc_live_history_same_view");
    let session_id = fixture.record.id.clone();
    let node_id = "review_sc_live_history";
    let raw_verdict = sc_narrow_verdict();
    let raw_json = serde_json::to_value(&raw_verdict).expect("raw verdict json");
    assert!(
        raw_json["findings"][0].get("disposition").is_none(),
        "夹具前提：provider 原始输出没有 disposition"
    );

    // live：先持久化 provider 原始 verdict，再用会话有效视图。
    let mut engine = fixture.engine();
    engine.timeline_nodes.push(review_timeline_node(node_id));
    let live = engine
        .finalize_review_verdict(node_id, "readable output".to_string(), raw_verdict.clone())
        .await;
    assert_eq!(
        live.findings[0].disposition,
        Some(ReviewFindingDisposition::Advisory),
        "窄例外命中 ⇒ 内存有效视图投影为 advisory"
    );
    assert_eq!(live.verdict, ReviewVerdictType::Pass);
    assert_eq!(live.review_gate, ReviewGate::UserConfirmAllowed);
    assert_eq!(
        live.findings[0].computed_finding_key(),
        raw_verdict.findings[0].computed_finding_key(),
        "投影不得改变内容身份"
    );
    assert_eq!(live.findings[0].required_action, "回写该事实");
    let durable = fixture
        .store
        .load_node_detail(&session_id, node_id)
        .expect("live durable detail");
    assert_eq!(
        durable.verdict.as_ref(),
        Some(&raw_json),
        "live 投影不得回写 durable"
    );

    // 历史刷新：反序列化/历史兼容写回之后再投影，与 live 同一有效视图。
    let session = WorkspaceSession::from_record(fixture.record.clone());
    let nodes = vec![review_timeline_node(node_id)];
    let historical =
        session_state::latest_review_verdict_for_session(&fixture.store, &session, &nodes)
            .expect("historical verdict readable");
    assert_eq!(
        historical.findings[0].disposition,
        Some(ReviewFindingDisposition::Advisory),
        "历史刷新与 live 同一投影"
    );
    assert_eq!(historical.verdict, live.verdict);
    assert_eq!(historical.review_gate, live.review_gate);
    let durable_after = fixture
        .store
        .load_node_detail(&session_id, node_id)
        .expect("historical durable detail");
    assert_eq!(
        durable_after.verdict.as_ref(),
        Some(&raw_json),
        "历史投影不得回写 durable"
    );
}

/// 窄例外只对服务端可信的 WorkItemPlan 单候选会话生效：Legacy 流与
/// Story 会话同形状 finding 保持原语义（actionable、照旧阻断）。
#[tokio::test]
async fn sc_projection_requires_trusted_single_candidate_session() {
    for entity in ["plan_legacy_flow_untouched", "story_session_untouched"] {
        let workspace_type = if entity.starts_with("plan") {
            WorkspaceType::WorkItemPlan
        } else {
            WorkspaceType::Story
        };
        let fixture = platform_fixture_with(entity, workspace_type, None);
        let mut engine = fixture.engine();
        let effective = engine
            .finalize_review_verdict("not_a_timeline_node", String::new(), sc_narrow_verdict())
            .await;
        assert_eq!(effective.findings[0].disposition, None, "{entity}");
        assert!(effective.findings[0].is_actionable(), "{entity}");
        assert_eq!(effective.verdict, ReviewVerdictType::Revise, "{entity}");
        assert_eq!(
            effective.review_gate,
            ReviewGate::UserConfirmAllowed,
            "{entity}"
        );
    }
}

/// REQ-WSC-12「缺 disposition 时仅在窄例外命中时降级」+「真必修与契约类
/// 仍阻断」：不命中窄例外的缺 disposition 条目继续 fail-closed；must_fix
/// 与平台加固后的契约类缺口在 SC 会话里照样进入阻断通道；命中的建议条目
/// 被降级，但不得把同 verdict 里的真必修「顺带」放行。
#[tokio::test]
async fn missing_disposition_outside_slice_and_real_blockers_stay_fail_closed() {
    let fixture = sc_platform_fixture("plan_sc_fail_closed_blockers");
    let mut engine = fixture.engine();

    // 缺 disposition + 不命中窄例外（severity=must_fix / category 契约类 /
    // class_hint 缺失）⇒ 一律保持 actionable。
    let effective = engine
        .finalize_review_verdict(
            "not_a_timeline_node_a",
            String::new(),
            ReviewVerdict {
                verdict: ReviewVerdictType::Revise,
                comments: String::new(),
                summary: "缺 disposition fail-closed".to_string(),
                findings: vec![
                    finding(ReviewFindingSeverity::MustFix, "本 Work Item 验收标准缺失"),
                    {
                        let mut contract_gap =
                            finding(ReviewFindingSeverity::Suggestion, "契约缺少 400 分支");
                        contract_gap.category = Some(ReviewFindingCategory::ContractGap);
                        contract_gap
                    },
                    {
                        let mut no_hint =
                            finding(ReviewFindingSeverity::Suggestion, "没有归类建议");
                        no_hint.category = Some(ReviewFindingCategory::Other);
                        no_hint
                    },
                ],
                review_gate: ReviewGate::RequiresRevision,
                work_item_plan_review: None,
                structured_output_diagnostic: None,
            },
        )
        .await;
    assert!(
        effective.findings.iter().all(ReviewFinding::is_actionable),
        "未命中窄例外的缺 disposition 条目必须 fail-closed"
    );
    assert_eq!(effective.verdict, ReviewVerdictType::Revise);
    assert_eq!(effective.review_gate, ReviewGate::RequiresRevision);

    // 混合 verdict：命中的建议被投影，真必修仍阻断（不因建议降级而放行）；
    // 模型标 advisory 的契约类缺口经平台加固仍是 actionable。
    let mut contract_advisory = finding(
        ReviewFindingSeverity::Suggestion,
        "自相矛盾：目标同时要求做与不做",
    );
    contract_advisory.category = Some(ReviewFindingCategory::SelfContradiction);
    contract_advisory.disposition = Some(ReviewFindingDisposition::Advisory);
    let contract_advisory = contract_advisory.with_platform_disposition_rules();
    assert!(contract_advisory.is_actionable(), "P0-1 契约类加固前置");
    let effective = engine
        .finalize_review_verdict(
            "not_a_timeline_node_b",
            String::new(),
            ReviewVerdict {
                verdict: ReviewVerdictType::Revise,
                comments: String::new(),
                summary: "混合 verdict".to_string(),
                findings: vec![
                    sc_narrow_finding("建议补一个可选字段说明"),
                    finding(ReviewFindingSeverity::MustFix, "本 Work Item 验收标准缺失"),
                    contract_advisory,
                ],
                review_gate: ReviewGate::RequiresRevision,
                work_item_plan_review: None,
                structured_output_diagnostic: None,
            },
        )
        .await;
    assert_eq!(
        effective.findings[0].disposition,
        Some(ReviewFindingDisposition::Advisory),
        "命中的建议条目被投影"
    );
    assert_eq!(
        effective.findings[1].disposition, None,
        "真必修不得被顺带降级"
    );
    assert!(effective.findings[1].is_actionable());
    assert!(
        effective.findings[2].is_actionable(),
        "契约类（平台加固）不得被窄例外放行"
    );
    assert_eq!(effective.verdict, ReviewVerdictType::Revise);
    assert_eq!(effective.review_gate, ReviewGate::RequiresRevision);
}

/// 「未知 category 不纳入」的接线证明：未知类别与非法 disposition 在 wire
/// 解析层即被拒绝（不猜值），typed 谓词的显式白名单是窄例外唯一入口——
/// 缺失（None）或未知都进不了窄例外。
#[test]
fn unknown_category_fails_wire_parse_so_whitelist_is_the_only_gate() {
    let base = serde_json::json!({
        "verdict": "revise",
        "comments": "",
        "summary": "未知类别",
        "findings": [{
            "severity": "suggestion",
            "message": "m",
            "category": "completeness",
            "class_hint": "repairable"
        }],
        "review_gate": "requires_revision"
    });
    let mut unknown_category = base.clone();
    unknown_category["findings"][0]["category"] = "mystery_kind".into();
    assert!(serde_json::from_value::<ReviewVerdict>(unknown_category).is_err());

    let mut invalid_disposition = base;
    invalid_disposition["findings"][0]["disposition"] = "maybe".into();
    assert!(
        serde_json::from_value::<ReviewVerdict>(invalid_disposition).is_err(),
        "非法 disposition 按既有 invalid_finding_field 口径拒收"
    );
}
