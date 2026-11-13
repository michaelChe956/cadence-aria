//! P03：`artifact_variant` 测试helper的新宿主（原 `review/tests_policy_routing.rs`）。
//!
//! 由 `review/tests_policy_routing.rs` 通过 `#[path]` 挂载；该文件由
//! `engine/tests.rs` include，故 `#[path]` 相对本文件所在目录（`review/`）解析。
//! 本次为等价搬迁：返回值与全部旧分支逐字不变，A4新变体由P17追加。

use super::{ArtifactPayload, WorkspaceEngine};

pub(super) fn artifact_variant(engine: &WorkspaceEngine) -> Option<&'static str> {
    engine
        .session()
        .artifact
        .as_ref()
        .map(|artifact| match artifact {
            ArtifactPayload::Markdown { .. } => "markdown",
            ArtifactPayload::WorkItemPlanCandidate { .. } => "work_item_plan_candidate",
            ArtifactPayload::WorkItemPlanOutlineCandidate { .. } => {
                "work_item_plan_outline_candidate"
            }
            ArtifactPayload::WorkItemPlanContextBlocker { .. } => "work_item_plan_context_blocker",
            ArtifactPayload::WorkItemDraftCandidate { .. } => "work_item_draft_candidate",
            ArtifactPayload::WorkItemBatchState { .. } => "work_item_batch_state",
            ArtifactPayload::WorkItemPlanCompileReport { .. } => "work_item_plan_compile_report",
            ArtifactPayload::WorkItemPlanProjection { .. } => "work_item_plan_projection",
            ArtifactPayload::WorkItemProjection { .. } => "work_item_projection",
            ArtifactPayload::WorkItemRevisionHistory { .. } => "work_item_revision_history",
            ArtifactPayload::ProjectionValidation { .. } => "projection_validation",
            ArtifactPayload::PlanAmendmentManifest { .. } => "plan_amendment_manifest",
            ArtifactPayload::ScFailureRepairPackage { .. } => "sc_failure_repair_package",
        })
}
