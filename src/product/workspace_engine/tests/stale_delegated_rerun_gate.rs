//! C（陈旧人工门）：SC 委托返修接力的「瞬时启动失败 ⇒ 回落人工门」与
//! 「接力随后真的接管接续路径」的判定与收口。
//!
//! 现场（陈旧人工门）：委托返修接力一次瞬时启动失败后**真的跑成功了**
//! （author 节点 completed、评审继续），但人工门已经开着且没人关——臂终态
//! `stopped_needs_human`，门节点仍 active。
//!
//! 判据（durable 事实，不依赖内存副本）：
//! - 门开态证据＝durable 会话 `status == WaitingForHuman`（该门不写门快照）；
//! - 「接力是否接管」＝durable `repair_reservation.state`（`Reserved` ⇒ 接力尚未
//!   领走 provider start；被消费 ⇒ 接力已接管，人工门已陈旧）；
//! - 「门是否由恢复路径开出」＝durable 时间线里 Active 的 HumanConfirm 节点摘要
//!   带恢复路径标记（**用户**开出的门无此标记，绝不被关）。
//!
//! 用例 A：开门后接力成功接管 ⇒ 不得残留开启的门（门节点已收口、无门快照、
//! 会话回到 Running）。
//! 用例 B：接力确实起不来（预领未被消费）⇒ 人工门仍然打开且原因可读。

use super::*;
use crate::product::json_store::write_json;
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus};
use crate::product::work_item_plan_policy::{
    RepairReservation, RepairReservationState, RunPolicy, WorkItemPlanFlowKind,
};
use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

/// 铺出「委托返修接力孤儿」形态：SC 流、phase=Generate、status=Running、
/// repair 预领 `Reserved`（接力 spawn 从未落地 ⇒ provider 可证明未启动）。
fn stale_delegated_rerun_gate_fixture()
-> (tempfile::TempDir, LifecycleStore, String, WorkspaceEngine) {
    let (tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.run_policy = RunPolicy::AutoIfValid;
    record.single_candidate_phase = Some(SingleCandidatePhase::Generate);
    record.status = WorkspaceSessionStatus::Running;
    record.repair_reservation = Some(RepairReservation {
        token: format!("single_candidate_author_repair:{}:1", record.id),
        owner_session_id: record.id.clone(),
        owner_run_id: "review_scope_v1:test".to_string(),
        provider_start_idempotency_key: format!("single_candidate_author:{}:1", record.id),
        state: RepairReservationState::Reserved,
        commit_id: None,
    });
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist delegated rerun orphan session");
    let artifact = engine.session.artifact.clone();
    engine.session = WorkspaceSession::from_record(record);
    engine.session.artifact = artifact;
    (tmp, lifecycle, plan_id, engine)
}

/// durable 时间线里的人工门节点（可观测面：门节点）。
fn durable_human_gate_nodes(
    lifecycle: &LifecycleStore,
    engine: &WorkspaceEngine,
) -> Vec<TimelineNode> {
    lifecycle
        .load_timeline_nodes_for_issue_session(
            &engine.session().project_id,
            &engine.session().issue_id,
            &engine.session().session_id,
        )
        .expect("load durable timeline")
        .into_iter()
        .filter(|node| node.node_type == TimelineNodeType::HumanConfirm)
        .collect()
}

/// C 用例 A（陈旧人工门回归）：接力瞬时启动失败回落人工门后，接力真的接管
/// 接续路径 ⇒ 不得残留开着的门。
///
/// 断言全部落在 durable 事实：会话状态、门快照、门节点。
#[tokio::test]
async fn stale_delegated_rerun_gate_is_closed_once_the_relay_takes_over() {
    let (_tmp, lifecycle, _plan_id, mut engine) = stale_delegated_rerun_gate_fixture();

    // ① 接力瞬时启动失败 ⇒ 回落人工门（durable：waiting_for_human + Active 门节点）。
    engine
        .recover_delegated_author_rerun_failure("进程重启：委托返修接力未启动")
        .await;
    let opened = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session after gate opened");
    assert_eq!(
        opened.status,
        WorkspaceSessionStatus::WaitingForHuman,
        "回落人工门必须落 durable waiting_for_human"
    );
    let gate_nodes = durable_human_gate_nodes(&lifecycle, &engine);
    assert_eq!(gate_nodes.len(), 1, "回落必须建一扇人工门节点");
    assert_eq!(
        gate_nodes[0].status,
        TimelineNodeStatus::Active,
        "刚开出的门节点必须是 active"
    );
    assert!(
        gate_nodes[0]
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("委托返修接力未启动")),
        "门摘要必须携带失败原因（原因可读）"
    );

    // ② 接力真的接管接续路径（消费 Reserved 预领）⇒ 陈旧门必须被收口。
    let should_start = engine
        .reserve_single_candidate_author_start()
        .expect("relay provider start reservation");
    assert!(should_start, "接力必须被放行启动");
    // 收口（生产接线：`run_single_candidate_author` 在接力放行后调用同一入口）。
    assert!(
        engine
            .close_stale_delegated_rerun_gate()
            .await
            .expect("close stale delegated rerun gate"),
        "接力接管后必须收口陈旧人工门"
    );

    // ③ durable 事实：会话回到 Running、门节点已收口、无残留门快照。
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session after relay took over");
    assert_eq!(
        durable.status,
        WorkspaceSessionStatus::Running,
        "接力接管后不得仍停在人工门（陈旧人工门回归）"
    );
    assert!(
        durable.human_gate_snapshot.is_none(),
        "回落门不写门快照，收口后不得残留门快照"
    );
    assert_eq!(
        durable
            .repair_reservation
            .as_ref()
            .map(|reservation| reservation.state),
        Some(RepairReservationState::ProviderStarted),
        "接力必须消费 Reserved 预领（接管的 durable 证据）"
    );
    let gate_nodes = durable_human_gate_nodes(&lifecycle, &engine);
    assert_eq!(gate_nodes.len(), 1, "门节点不得被删除，只收口");
    assert_eq!(
        gate_nodes[0].status,
        TimelineNodeStatus::Completed,
        "陈旧人工门节点必须收口（不得残留 active 门）"
    );
}

/// C 用例 B（人工出口保留）：接力确实起不来（预领未被消费）⇒ 人工门仍然打开，
/// 且原因可读。
#[tokio::test]
async fn stale_delegated_rerun_gate_stays_open_when_the_relay_never_takes_over() {
    let (_tmp, lifecycle, _plan_id, mut engine) = stale_delegated_rerun_gate_fixture();

    engine
        .recover_delegated_author_rerun_failure("进程重启：委托返修接力未启动")
        .await;

    // 接力确实起不来：预领仍是 Reserved（provider 可证明未启动）。
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session after gate opened");
    assert_eq!(
        durable
            .repair_reservation
            .as_ref()
            .map(|reservation| reservation.state),
        Some(RepairReservationState::Reserved),
        "未接管的接力预领必须保持 Reserved"
    );
    assert_eq!(
        durable.status,
        WorkspaceSessionStatus::WaitingForHuman,
        "接力起不来时人工门必须仍然打开（保留人工出口）"
    );
    let gate_nodes = durable_human_gate_nodes(&lifecycle, &engine);
    assert_eq!(gate_nodes.len(), 1, "人工门节点必须在场");
    assert_eq!(
        gate_nodes[0].status,
        TimelineNodeStatus::Active,
        "人工门节点必须仍是 active"
    );
    let summary = gate_nodes[0]
        .summary
        .clone()
        .expect("人工门摘要必须携带失败原因");
    assert!(
        summary.contains("委托返修接力未启动"),
        "人工门原因必须可读：{summary}"
    );

    // 收口入口在接力未接管时必须是 no-op（不得误关人工出口）。
    assert!(
        !engine
            .close_stale_delegated_rerun_gate()
            .await
            .expect("close stale delegated rerun gate"),
        "接力未接管时不得收口人工门"
    );
    let durable = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("durable session after no-op close");
    assert_eq!(
        durable.status,
        WorkspaceSessionStatus::WaitingForHuman,
        "no-op 后人工门必须仍然打开"
    );
    assert_eq!(
        durable_human_gate_nodes(&lifecycle, &engine)[0].status,
        TimelineNodeStatus::Active,
        "no-op 后门节点必须仍然 active"
    );
}
