use std::collections::BTreeSet;

use crate::product::models::{HumanPresentationRevision, WorkItemPlanLineage, WorkspaceType};
use crate::product::work_item_projection::{
    HumanPresentationBase, validate_human_presentation_revision,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::ArtifactPayload;

use super::{WorkspaceEngine, WorkspaceEngineError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanPresentationScope {
    Plan,
    WorkItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveHumanPresentationRevision {
    pub source_projection_bundle_id: String,
    pub scope: HumanPresentationScope,
    pub supersedes: Option<String>,
    pub human_summary: String,
    pub why_split: Option<String>,
    pub dependency_explanation: Vec<String>,
    pub risk_explanation: Vec<String>,
    pub source_refs: Vec<String>,
}

pub fn save_human_presentation_revision(
    store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
    mut revision: HumanPresentationRevision,
) -> Result<HumanPresentationRevision, WorkspaceEngineError> {
    revision.normative = false;
    revision.used_by_provider = false;
    match (
        revision.source_plan_projection_bundle_id.as_deref(),
        revision.source_work_item_projection_bundle_id.as_deref(),
    ) {
        (Some(bundle_id), None) => {
            let bundle = store.get_plan_projection_bundle(plan, bundle_id)?;
            validate_human_presentation_revision(
                HumanPresentationBase::Plan {
                    projection_bundle_id: &bundle.id,
                    projection: &bundle.human_group_projection,
                },
                &revision,
            )?;
        }
        (None, Some(bundle_id)) => {
            let bundle = store.get_work_item_projection_bundle(plan, bundle_id)?;
            validate_human_presentation_revision(
                HumanPresentationBase::WorkItem {
                    projection_bundle_id: &bundle.id,
                    projection: &bundle.human_projection,
                },
                &revision,
            )?;
        }
        _ => return Err(WorkspaceEngineError::InvalidHumanPresentationTarget),
    }
    Ok(store.put_human_presentation_revision_cas(plan, revision)?)
}

impl WorkspaceEngine {
    pub fn save_human_presentation_revision_command(
        &self,
        command: SaveHumanPresentationRevision,
    ) -> Result<HumanPresentationRevision, WorkspaceEngineError> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan {
            return Err(WorkspaceEngineError::InvalidHumanPresentationTarget);
        }
        let store = self.revision_store();
        let plan = store.get_plan_lineage(
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        )?;
        let (source_plan_projection_bundle_id, source_work_item_projection_bundle_id) =
            match command.scope {
                HumanPresentationScope::Plan => (Some(command.source_projection_bundle_id), None),
                HumanPresentationScope::WorkItem => {
                    (None, Some(command.source_projection_bundle_id))
                }
            };
        save_human_presentation_revision(
            &store,
            &plan,
            HumanPresentationRevision {
                id: String::new(),
                source_plan_projection_bundle_id,
                source_work_item_projection_bundle_id,
                supersedes: command.supersedes,
                human_summary: command.human_summary,
                why_split: command.why_split,
                dependency_explanation: command.dependency_explanation,
                risk_explanation: command.risk_explanation,
                source_refs: command.source_refs,
                normative: false,
                used_by_provider: false,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
    }

    pub(crate) fn latest_human_presentation_revisions(&self) -> Vec<HumanPresentationRevision> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan {
            return Vec::new();
        }
        let store = self.revision_store();
        let Ok(plan) = store.get_plan_lineage(
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        ) else {
            return Vec::new();
        };
        let mut bundle_ids = BTreeSet::new();
        for version in &self.artifact_versions {
            collect_projection_bundle_id(&version.payload, &mut bundle_ids);
        }
        if let Some(artifact) = self.session.artifact.as_ref() {
            collect_projection_bundle_id(artifact, &mut bundle_ids);
        }
        bundle_ids
            .into_iter()
            .filter_map(|bundle_id| {
                store
                    .get_latest_human_presentation_revision(&plan, &bundle_id)
                    .ok()
                    .flatten()
            })
            .collect()
    }
}

fn collect_projection_bundle_id(payload: &ArtifactPayload, bundle_ids: &mut BTreeSet<String>) {
    match payload {
        ArtifactPayload::WorkItemPlanProjection { projection } => {
            bundle_ids.insert(projection.id.clone());
        }
        ArtifactPayload::WorkItemProjection { projection } => {
            bundle_ids.insert(projection.id.clone());
        }
        _ => {}
    }
}
