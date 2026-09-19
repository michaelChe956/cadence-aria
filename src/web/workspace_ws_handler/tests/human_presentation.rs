use std::collections::BTreeMap;

use super::*;
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::{
    PlanProjectionBundle, WorkItemPlanLineage, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::work_item_projection::{
    CoderGroupContext, HumanGroupProjection, ReviewerGroupMatrix,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

// 退役留档（T5/REQ-RET-02）：`human_presentation_save_is_stage_independent_and_non_plan_workspaces_stay_unsupported` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`human_presentation_save_handler_acknowledges_success_and_returns_recoverable_conflict` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

async fn receive_outbound(outbound_rx: &mut mpsc::Receiver<OutboundControl>) -> WsOutMessage {
    let control = tokio::time::timeout(std::time::Duration::from_secs(1), outbound_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let OutboundControl::Text(text) = control else {
        panic!("expected text outbound");
    };
    serde_json::from_str(&text).unwrap()
}

fn plan_projection_bundle() -> PlanProjectionBundle {
    PlanProjectionBundle {
        id: "plan_projection_bundle_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
        work_item_projection_bundle_refs: vec![],
        human_group_projection: HumanGroupProjection {
            plan_id: "work_item_plan_0001".to_string(),
            goal: "Goal".to_string(),
            split_reason: "Split".to_string(),
            work_items: vec![],
            contract_flow: vec![],
            risks: vec![],
            source_refs: vec!["story:001".to_string()],
            normative: false,
            used_by_provider: false,
        },
        coder_group_context: CoderGroupContext {
            plan_id: "work_item_plan_0001".to_string(),
            ordered_logical_work_item_ids: vec![],
            dependency_edges: vec![],
            group_write_scopes: BTreeMap::new(),
        },
        reviewer_group_matrix: ReviewerGroupMatrix {
            plan_id: "work_item_plan_0001".to_string(),
            work_items: vec![],
            dependency_edges: vec![],
            design_traceability_refs: vec![],
        },
        human_group_projection_hash: "human".to_string(),
        coder_group_context_hash: "coder".to_string(),
        reviewer_group_matrix_hash: "reviewer".to_string(),
        compiler_version: "projection-v1".to_string(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
    }
}
