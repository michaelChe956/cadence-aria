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
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
use crate::product::models::{
    AgentRole, HumanDispositionRecord, HumanFindingDispositionKind, HumanGateTurnRejection,
    NodeDetail, ProviderName, WorkspaceType,
};
use crate::product::work_item_plan_policy::{FindingClass, classify_finding};
use crate::product::workspace_engine::{WorkspaceEngine, WorkspaceSession, session_state};
use crate::web::workspace_ws_types::{
    ProviderConfigSnapshot, ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict,
    ReviewVerdictType, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage, WsOutMessage,
};

struct PlatformFixture {
    tmp: TempDir,
    store: LifecycleStore,
    record: crate::product::models::WorkspaceSessionRecord,
}

fn platform_fixture(entity_id: &str) -> PlatformFixture {
    let tmp = TempDir::new().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let record = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: entity_id.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
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
    let disposed = finding(ReviewFindingSeverity::MustFix, "中文 locale 尚未配置");
    let mut still_open = finding(ReviewFindingSeverity::MustFix, "本 Work Item 验收标准缺口");
    still_open.required_action = "补齐验收标准".to_string();
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

    let mut engine = fixture.engine();
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
        "只有未处置的 finding 能保留身份条目: {section}"
    );
    assert!(
        !section.contains("回写该事实"),
        "owner 已处置事实不得再以未收敛 finding 注入: {section}"
    );
}
