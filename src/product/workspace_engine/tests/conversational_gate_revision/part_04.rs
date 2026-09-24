
/// 把 rep4 的结构标题翻成两份现场事故的中文变体(混合括号注记与裸中文,
/// 覆盖 pi-full rep1 与矩阵 r1 rep2 两种抖动形态)。
fn with_field_chinese_heading_translations(markdown: &str) -> String {
    let mut translated = markdown
        .lines()
        .map(|line| match line {
            "# Work Item Plan" => "# 工作项计划".to_string(),
            l if l.starts_with("## Work Item WI-") => {
                format!("## 工作项{}", &l["## Work Item".len()..])
            }
            "### Identity" => "### 身份信息".to_string(),
            "### Goal" => "### 目标 (Goal)".to_string(),
            "### Non Goals" => "### 非目标".to_string(),
            "### Dependencies" => "### 依赖关系".to_string(),
            "### Inputs" => "### 输入".to_string(),
            "### Outputs" => "### 输出 (Outputs)".to_string(),
            "### Tasks" => "### 任务".to_string(),
            "### Write Policy" => "### 编写策略".to_string(),
            "### Acceptance Criteria" => "### 验收标准".to_string(),
            "### Verification" => "### 验证".to_string(),
            "### Handoff Schema" => "### 交接模式 (Handoff Schema)".to_string(),
            "### Blockers" => "### 阻塞项".to_string(),
            "### Traceability" => "### 可追溯性".to_string(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    if markdown.ends_with('\n') {
        translated.push('\n');
    }
    translated
}

#[test]
fn conversational_gate_revision_delivery_normalizes_fixed_chinese_headings() {
    let translated = with_field_chinese_heading_translations(REP4_FIXTURE);

    let delivery = crate::product::workspace_engine::conversational_gate::prepare_revision_delivery_for_compile(
        &format!("provider preamble\n{translated}"),
    );

    // 前言被修剪，结构标题全部归一化，正文行逐字保留。rep4 的 45 个标题行中
    // `### Notes`/`### Rationale` 属自由文本 section，不在固定映射表内、保持英文。
    assert_eq!(delivery.normalized_heading_lines, 43);
    let normalized_lines: Vec<&str> = delivery.source.split('\n').collect();
    let fixture_lines: Vec<&str> = REP4_FIXTURE.split('\n').collect();
    assert_eq!(
        normalized_lines, fixture_lines,
        "归一化后必须与规范英文原文逐行一致"
    );
}

#[tokio::test]
async fn conversational_gate_revision_result_accepts_field_chinese_heading_translations() {
    let (_root, _lifecycle, mut engine) = durable_revision_fixture("revision_zh_headings", 2);
    let turn_id = open_running_revision_turn(&mut engine, "revision_zh_command").await;
    let provider_output = format!(
        "provider preamble\n{}",
        with_field_chinese_heading_translations(REP4_FIXTURE)
    );

    let result = engine
        .run_sc_manual_revision_turn(&turn_id, provider_output)
        .await
        .expect("中文标题交付必须经确定性归一化后被接受");

    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    // 落盘候选必须已是规范英文标题(正文逐字等于 rep4 原文)。
    assert!(
        engine
            .session()
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown())
            .is_some_and(|markdown| markdown == REP4_FIXTURE)
    );
}

#[tokio::test]
async fn conversational_gate_revision_heading_normalized_audit_events_coexist_across_turns() {
    let (_root, lifecycle, mut engine) = durable_revision_fixture("revision_audit_coexist", 2);
    // 归一化审计事件按 event_id upsert 进 active 节点的 node detail。
    // 挂一个人工确认节点,模拟同一 human-confirm 节点上的多轮修订:
    // 两轮 turn 各自发生标题归一化时,必须各留一条审计记录而非互相覆盖。
    let node_id = engine
        .create_timeline_node(crate::product::workspace_engine::TimelineNodeDraft {
            node_type: crate::web::workspace_ws_types::TimelineNodeType::HumanConfirm,
            agent: Some(crate::product::models::ProviderName::ClaudeCode),
            stage: crate::product::workspace_engine::WorkspaceStage::HumanConfirm,
            round: None,
            title: "人工确认".to_string(),
            summary: None,
            status: crate::web::workspace_ws_types::TimelineNodeStatus::Active,
        })
        .await;

    let first_turn = open_running_revision_turn(&mut engine, "revision_audit_round_one").await;
    engine.active_node_id = Some(node_id.clone());
    engine
        .run_sc_manual_revision_turn(
            &first_turn,
            format!(
                "provider preamble\n{}",
                with_field_chinese_heading_translations(REP4_FIXTURE)
            ),
        )
        .await
        .expect("first zh-heading revision should succeed");

    let second_outcome = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "revision_audit_round_two".to_string(),
            feedback: "再次修订标题".to_string(),
        })
        .await
        .expect("second feedback");
    let second_turn = match second_outcome {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected second turn, got {other:?}"),
    };
    engine
        .mark_human_gate_turn_running(&second_turn)
        .expect("second turn running");
    let second_content = with_field_chinese_heading_translations(REP4_FIXTURE)
        .replace("Backend levels API", "Backend levels API v2");
    engine.active_node_id = Some(node_id.clone());
    engine
        .run_sc_manual_revision_turn(&second_turn, format!("provider preamble\n{second_content}"))
        .await
        .expect("second zh-heading revision should succeed");

    let detail = lifecycle
        .load_node_detail(engine.session().session_id.as_str(), &node_id)
        .expect("load human-confirm node detail");
    let audit_ids: Vec<String> = detail
        .execution_events
        .iter()
        .filter_map(|event| event["event_id"].as_str())
        .filter(|event_id| event_id.starts_with("human_gate_revision_heading_normalized_"))
        .map(str::to_string)
        .collect();
    assert_eq!(
        audit_ids.len(),
        2,
        "同一 human-confirm 节点两轮归一化必须各留一条审计记录,实际: {audit_ids:?}"
    );
    assert!(audit_ids.contains(&format!(
        "human_gate_revision_heading_normalized_{node_id}_{first_turn}"
    )));
    assert!(audit_ids.contains(&format!(
        "human_gate_revision_heading_normalized_{node_id}_{second_turn}"
    )));
}

/// F-49/A5：门内轮次切换（反馈 → 修订 → 复评 → 新门）必须收口被取代的门节点。
///
/// 现场同构（issue_0002/workspace_session_0009 实测）：timeline_node_006 与
/// timeline_node_010 同时 `status=active`——F-42 的收口判据是「活动节点恰为 Active
/// HumanConfirm」，只在终态确认链（approve/compile）成立；门内轮次之间零收口，
/// 旧门节点永久滞留 Active，重连投影据残留 Active 节点恢复门态。
#[tokio::test]
async fn conversational_gate_round_switch_closes_superseded_human_confirm_node() {
    use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

    let (_root, lifecycle, mut engine) = evaluate_gate_revision_fixture("gate_round_switch", 2, 1);
    // 第一轮门：真实开门（enter_human_confirm 建 Active HumanConfirm 节点）。
    engine
        .enter_human_confirm(Some("第一轮人工确认".to_string()))
        .await;
    let first_gate = engine.active_timeline_node_id().expect("first gate node");
    assert_eq!(
        timeline_node_status(&engine, &first_gate),
        TimelineNodeStatus::Active
    );

    // 门内反馈 → 修订 → 复评 pass → 第二轮以新门取代旧门。
    let turn_id = open_running_revision_turn(&mut engine, "gate_round_switch_command").await;
    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("valid revision must complete");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "review".to_string(),
                None,
            ),
            pass_revision_review_verdict(),
        )
        .await;
    let second_gate = engine.active_timeline_node_id().expect("second gate node");
    assert_ne!(second_gate, first_gate, "轮次切换必须开新一轮门节点");

    // 核心断言：旧门节点被收口；任一时刻只有一个 Active 门节点。
    assert_eq!(
        timeline_node_status(&engine, &first_gate),
        TimelineNodeStatus::Completed,
        "门内轮次切换必须收口被取代的门节点（实测缺陷：双 active human_confirm）"
    );
    let active_gates: Vec<&str> = engine
        .timeline_nodes
        .iter()
        .filter(|node| {
            node.node_type == TimelineNodeType::HumanConfirm
                && node.status == TimelineNodeStatus::Active
        })
        .map(|node| node.node_id.as_str())
        .collect();
    assert_eq!(active_gates, vec![second_gate.as_str()]);

    // durable 同步：重启/重连投影只据落盘事实恢复门态。
    let session_id = engine.session().session_id.as_str();
    let durable = lifecycle
        .load_timeline_nodes(session_id)
        .expect("durable timeline nodes");
    let durable_first = durable
        .iter()
        .find(|node| node.node_id == first_gate)
        .expect("durable first gate node");
    assert_eq!(durable_first.status, TimelineNodeStatus::Completed);
    assert!(
        durable_first.completed_at.is_some(),
        "收口必须落完成时间（与 F-42 同源链路）"
    );
    assert_eq!(
        durable
            .iter()
            .filter(|node| {
                node.node_type == TimelineNodeType::HumanConfirm
                    && node.status == TimelineNodeStatus::Active
            })
            .count(),
        1,
        "durable 时间线只允许一个 Active 门节点"
    );
}

/// F-49/A6：门内人工修订轮必须与普通 SC 修订同构地落在一个 author 节点上。
///
/// 现状（现场同构 workspace_session_0009）：修订由 `active_timeline_node_id()`
/// （门节点）驱动，revision prompt / streaming_content / output 事件 / artifact_ref
/// 全部写进门节点 detail；门节点 `node_type=human_confirm` 在对话流 rebuild 的
/// `chatRoleForTimelineNode` 判为 null，整节点零条目——实测 artifact v3 挂
/// timeline_node_006，用户侧「author 修订步不可见」。
#[tokio::test]
async fn conversational_gate_revision_round_records_facts_on_author_node() {
    use crate::product::models::AgentRole;
    use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

    let (_root, lifecycle, mut engine) =
        evaluate_gate_revision_fixture("gate_revision_author_node", 2, 1);
    engine
        .enter_human_confirm(Some("门内修订".to_string()))
        .await;
    let gate_node = engine.active_timeline_node_id().expect("gate node");
    let author_nodes_before = engine
        .timeline_nodes
        .iter()
        .filter(|node| node.node_type == TimelineNodeType::AuthorRun)
        .count();

    let turn_id =
        open_running_revision_turn(&mut engine, "gate_revision_author_node_command").await;
    // ws 层（provider_run.rs 的 HumanGateScManualRevision 臂）在驱动 provider 前
    // 以本入口选节点：门修订轮的 author 载体。
    let run_node = engine.begin_work_item_plan_human_gate_revision_run().await;
    assert_ne!(run_node, gate_node, "门修订不得把修订事实写进门节点");
    assert_eq!(
        timeline_node_status(&engine, &run_node),
        TimelineNodeStatus::Active
    );
    assert_eq!(
        engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == run_node)
            .map(|node| node.node_type.clone()),
        Some(TimelineNodeType::AuthorRun)
    );
    // stage 必须与会话阶段一致：`new_persistent` 对非空 durable 时间线以活动节点的
    // stage 重建 session.stage，门内修订期间会话阶段恒为 human_confirm（REQ-CG-02
    // 门命令以该阶段为前提）。写成 running 会让重连/第二 worker 把阶段推导成
    // running，门内 feedback/approve/abandon 被拒为 STAGE_INVALID（campaign 单飞
    // 用例实测回归）。
    assert_eq!(
        engine
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == run_node)
            .map(|node| node.stage.clone()),
        Some(crate::web::workspace_ws_types::WorkspaceStage::HumanConfirm)
    );
    // provider 中断后以同一 turn 重跑（恢复）：复用同一 author 节点，不重复建节点。
    assert_eq!(
        engine.begin_work_item_plan_human_gate_revision_run().await,
        run_node,
        "同一修订轮重跑必须复用既有 author 节点"
    );
    assert_eq!(
        engine
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::AuthorRun)
            .count(),
        author_nodes_before + 1
    );

    let result = engine
        .run_sc_manual_revision_turn(&turn_id, handoff_clean_rep4_v2())
        .await
        .expect("valid revision must complete");
    assert!(matches!(
        result,
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
    ));

    // 修订事实同源：`human_gate_turn_completed` 帧携带的 artifact_ref 必须等于 author
    // 节点 detail 的 artifact_ref（前端可从帧与 node detail 两处消费同一修订事实）。
    let session_id = engine.session().session_id.as_str();
    // 修订成功即收口该节点（与普通修订 node_004/node_008 的 Completed 同构），
    // 不留永久 Active 的「修订中」节点。
    assert_eq!(
        timeline_node_status(&engine, &run_node),
        TimelineNodeStatus::Completed
    );
    let turn = lifecycle
        .get_human_gate_turn(session_id, &turn_id)
        .expect("durable turn");
    let turn_artifact_ref = turn
        .result_artifact_ref
        .expect("completed turn artifact ref");
    let author_detail = lifecycle
        .load_node_detail(session_id, &run_node)
        .expect("author node detail");
    assert_eq!(author_detail.node_type, TimelineNodeType::AuthorRun);
    assert_eq!(
        author_detail.agent_role,
        Some(AgentRole::Author),
        "修订事实必须落在 author 角色节点（前端 rebuild 据 agent_role 渲染气泡）"
    );
    assert_eq!(
        author_detail
            .artifact_ref
            .as_ref()
            .map(|artifact| artifact.artifact_id.as_str()),
        Some(turn_artifact_ref.as_str()),
        "author 节点 detail 必须携带本次修订产物（现状缺陷：v3 挂门节点）"
    );
    // 门节点不得携带修订产物引用。
    match lifecycle.load_node_detail(session_id, &gate_node) {
        Ok(gate_detail) => assert!(
            gate_detail.artifact_ref.is_none(),
            "门节点不得携带修订产物: {:?}",
            gate_detail.artifact_ref
        ),
        Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => {}
        Err(error) => panic!("load gate node detail failed: {error}"),
    }
}

fn timeline_node_status(
    engine: &crate::product::workspace_engine::WorkspaceEngine,
    node_id: &str,
) -> crate::web::workspace_ws_types::TimelineNodeStatus {
    engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .map(|node| node.status.clone())
        .unwrap_or_else(|| panic!("timeline node not found: {node_id}"))
}

fn pass_revision_review_verdict() -> crate::web::workspace_ws_types::ReviewVerdict {
    crate::web::workspace_ws_types::ReviewVerdict {
        verdict: crate::web::workspace_ws_types::ReviewVerdictType::Pass,
        comments: "review pass".to_string(),
        summary: "review pass".to_string(),
        findings: Vec::new(),
        review_gate: crate::web::workspace_ws_types::ReviewGate::UserConfirmAllowed,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

#[tokio::test]
async fn conversational_gate_revision_result_rejects_unknown_chinese_heading() {
    let (_root, _lifecycle, mut engine) =
        durable_revision_fixture("revision_zh_unknown_heading", 2);
    let turn_id = open_running_revision_turn(&mut engine, "revision_zh_unknown_command").await;
    // 表外中文标题(`溯源清单` 不在固定映射表内)不得被猜测改写。
    let unknown_translation = with_field_chinese_heading_translations(REP4_FIXTURE)
        .replace("### 可追溯性", "### 溯源清单");
    let provider_output = format!("provider preamble\n{unknown_translation}");

    let result = engine
        .run_sc_manual_revision_turn(&turn_id, provider_output)
        .await
        .expect("表外标题必须是校验拒绝而非协议错误");

    match result {
        crate::product::workspace_engine::ScManualRevisionResult::ValidationRejected {
            diagnostics,
        } => assert!(
            diagnostics
                .iter()
                .any(|message| message.contains("unknown_structured_key")),
            "未知中文标题必须仍被 fail-closed 拒绝: {diagnostics:?}"
        ),
        crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. } => {
            panic!("expected validation rejection, got accepted")
        }
    }
}
