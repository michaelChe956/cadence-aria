use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::PlanRepairFixtureError;
use super::recovery::{fixture_error, unique_repair_link};
use super::seed::fixture_paths;
use crate::product::app_paths::ProductAppPaths;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
use crate::product::models::{
    ProviderName, WorkspaceSessionRelation, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::product_data_schema::ensure_product_data_schema;
use crate::product::workspace_engine::{
    EngineEvent, LinkedWorkspaceAmendmentTarget, LinkedWorkspaceSessionSnapshot, WorkspaceEngine,
    WorkspaceSession, restore_linked_workspace_snapshot,
};
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus,
    TimelineNodeType, WorkspaceStage as WsWorkspaceStage,
};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_plan_0001";

pub(super) async fn restore_linked_workspace_matrix(
    root: &Path,
) -> Result<Vec<LinkedWorkspaceSessionSnapshot>, PlanRepairFixtureError> {
    let paths = fixture_paths(root);
    let lifecycle = LifecycleStore::new(paths.clone());
    let repair_link = unique_repair_link(&lifecycle)?;
    let repair_child = lifecycle
        .get_workspace_session(&repair_link.child_session_id)
        .map_err(fixture_error)?;
    let (event_tx, _event_rx) = mpsc::channel::<EngineEvent>(16);
    let repair_engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            root.join("linked-workspace-checkpoints"),
        )),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(repair_child.clone()),
    );

    let mut links = Vec::new();
    for (workspace_type, relation, entity_id) in [
        (
            WorkspaceType::Story,
            WorkspaceSessionRelation::StoryAmendment,
            "story_spec_0001",
        ),
        (
            WorkspaceType::Design,
            WorkspaceSessionRelation::DesignAmendment,
            "design_spec_0001",
        ),
    ] {
        let snapshot = repair_engine
            .start_linked_workspace_amendment(LinkedWorkspaceAmendmentTarget {
                entity_id: entity_id.to_string(),
                workspace_type,
                relation,
            })
            .map_err(fixture_error)?;
        links.push(snapshot.link);
    }

    for link in &links {
        persist_human_confirmation(&lifecycle, link)?;
    }
    let restarted = LifecycleStore::new(paths);
    let mut restored = links
        .iter()
        .map(|link| {
            restore_linked_workspace_snapshot(&restarted, PROJECT_ID, ISSUE_ID, link)
                .map_err(fixture_error)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let work_item_paths = ProductAppPaths::new(root.join("linked-work-item-store/.aria"));
    ensure_product_data_schema(&work_item_paths).map_err(fixture_error)?;
    let work_item_lifecycle = LifecycleStore::new(work_item_paths.clone());
    let work_item_parent = work_item_lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: "work_item_plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
            "workspace_session_linked_work_item_parent".to_string(),
        )
        .map_err(fixture_error)?;
    let work_item_child = work_item_lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: "wi_core".to_string(),
                workspace_type: WorkspaceType::WorkItem,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
            "workspace_session_linked_wi_core".to_string(),
        )
        .map_err(fixture_error)?;
    let mut work_item_link = repair_link;
    work_item_link.id = "workspace_session_link_plan_repair_wi_core".to_string();
    work_item_link.relation = WorkspaceSessionRelation::PlanRepair;
    work_item_link.parent_session_id = work_item_parent.id;
    work_item_link.child_session_id = work_item_child.id;
    work_item_link.return_context.original_route =
        format!("/workbench/workspace/{}", work_item_link.parent_session_id);
    work_item_lifecycle
        .put_session_link(PROJECT_ID, ISSUE_ID, &work_item_link)
        .map_err(fixture_error)?;
    persist_human_confirmation(&work_item_lifecycle, &work_item_link)?;
    restored.push(
        restore_linked_workspace_snapshot(
            &LifecycleStore::new(work_item_paths),
            PROJECT_ID,
            ISSUE_ID,
            &work_item_link,
        )
        .map_err(fixture_error)?,
    );
    Ok(restored)
}

fn persist_human_confirmation(
    lifecycle: &LifecycleStore,
    link: &crate::product::models::WorkspaceSessionLink,
) -> Result<(), PlanRepairFixtureError> {
    let child = lifecycle
        .get_workspace_session(&link.child_session_id)
        .map_err(fixture_error)?;
    let node_id = format!("timeline_node_linked_{}", child.entity_id);
    lifecycle
        .append_artifact_version(
            &child.id,
            ArtifactVersion {
                version: 7,
                payload: ArtifactPayload::Markdown {
                    markdown: format!("# {} amendment", child.entity_id),
                    diff: None,
                },
                generated_by: ProviderName::ClaudeCode,
                reviewed_by: Some(ProviderName::Codex),
                review_verdict: None,
                confirmed_by: None,
                is_current: true,
                created_at: "2026-07-20T00:00:20Z".to_string(),
                source_node_id: node_id.clone(),
            },
        )
        .map_err(fixture_error)?;
    lifecycle
        .save_timeline_nodes(
            &child.id,
            &[TimelineNode {
                node_id,
                node_type: TimelineNodeType::HumanConfirm,
                agent: None,
                stage: WsWorkspaceStage::HumanConfirm,
                round: None,
                status: TimelineNodeStatus::Active,
                title: "等待确认".to_string(),
                summary: Some("等待人工确认修订".to_string()),
                started_at: "2026-07-20T00:00:20Z".to_string(),
                completed_at: None,
                duration_ms: None,
                artifact_ref: Some("artifact_0007/v7".to_string()),
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::ClaudeCode,
                    reviewer: Some(ProviderName::Codex),
                    review_rounds: 2,
                    permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(
                    ),
                },
                retry: None,
            }],
        )
        .map_err(fixture_error)?;
    lifecycle
        .update_workspace_session_status(&child.id, WorkspaceSessionStatus::WaitingForHuman)
        .map_err(fixture_error)?;
    Ok(())
}
