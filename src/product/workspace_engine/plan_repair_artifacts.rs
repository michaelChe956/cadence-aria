use chrono::Utc;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::models::{ArtifactRef, PlanRepairSessionStage, WorkspaceType};
use crate::product::plan_repair::PlanRepairError;
use crate::product::work_item_revision_history::build_authoritative_coding_revision_history;
use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion, WorkItemHistoryEntryKind};

use super::*;

impl WorkspaceEngine {
    pub async fn ensure_plan_repair_artifacts(&mut self) -> Result<(), PlanRepairError> {
        let Some(snapshot) = self.plan_repair_snapshot.clone() else {
            return Ok(());
        };
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || snapshot.error.is_some()
            || snapshot.stage == PlanRepairSessionStage::Failed
        {
            return Ok(());
        }
        let lifecycle = self.persistent_lifecycle()?;
        let coding_store = CodingAttemptStore::new(lifecycle.app_paths());
        let attempt = coding_store
            .get_attempt(
                &self.session.project_id,
                &self.session.issue_id,
                &snapshot.request.trigger_attempt_id,
            )
            .map_err(PlanRepairError::Store)?;
        let projection = build_authoritative_coding_revision_history(
            &lifecycle.app_paths(),
            &attempt,
            Some(&self.session.session_id),
        )
        .map_err(PlanRepairError::Store)?;
        if projection.plan_id != snapshot.request.plan_id
            || projection.plan_revision_id != snapshot.request.base_plan_revision_id
            || !projection.history.entries.iter().any(|entry| {
                entry.kind == WorkItemHistoryEntryKind::UnitRun
                    && entry.id == snapshot.request.trigger_unit_run_id
            })
        {
            return Err(PlanRepairError::InvalidRepairTarget(
                "plan repair history authority does not match the linked coding attempt"
                    .to_string(),
            ));
        }
        self.ensure_plan_repair_artifact(ArtifactPayload::WorkItemRevisionHistory {
            history: Box::new(projection.history),
        })
        .await?;
        if snapshot.amendment.is_some() {
            self.ensure_plan_repair_manifest_artifact().await?;
        }
        Ok(())
    }

    pub(crate) async fn ensure_plan_repair_manifest_artifact(
        &mut self,
    ) -> Result<(), PlanRepairError> {
        let Some(snapshot) = self.plan_repair_snapshot.as_ref() else {
            return Ok(());
        };
        let Some(amendment) = snapshot.amendment.clone() else {
            return Ok(());
        };
        self.ensure_plan_repair_artifact(ArtifactPayload::PlanAmendmentManifest {
            manifest: Box::new(amendment),
        })
        .await
    }

    async fn ensure_plan_repair_artifact(
        &mut self,
        payload: ArtifactPayload,
    ) -> Result<(), PlanRepairError> {
        let matching = self
            .artifact_versions
            .iter()
            .enumerate()
            .filter(|(_, version)| same_plan_repair_artifact_kind(&version.payload, &payload))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(PlanRepairError::InvalidRepairTarget(
                "duplicate plan repair artifact versions".to_string(),
            ));
        }
        let update = if let Some(index) = matching.first().copied() {
            if self.artifact_versions[index].payload == payload {
                return Ok(());
            }
            if matches!(payload, ArtifactPayload::PlanAmendmentManifest { .. }) {
                return Err(PlanRepairError::InvalidRepairTarget(
                    "plan amendment manifest artifact identity mismatch".to_string(),
                ));
            }
            self.artifact_versions[index].payload = payload.clone();
            if self.artifact_versions[index].is_current {
                self.session.artifact = Some(payload.clone());
            }
            ArtifactUpdateEvent {
                version: self.artifact_versions[index].version,
                payload,
            }
        } else {
            for version in &mut self.artifact_versions {
                version.is_current = false;
            }
            let version = self
                .artifact_versions
                .iter()
                .map(|version| version.version)
                .max()
                .unwrap_or(0)
                + 1;
            let source_node_id = self
                .active_node_id
                .clone()
                .unwrap_or_else(|| "plan_repair_bootstrap".to_string());
            self.session.artifact = Some(payload.clone());
            self.artifact_versions.push(ArtifactVersion {
                version,
                payload: payload.clone(),
                generated_by: self.session.author_provider.clone(),
                reviewed_by: None,
                review_verdict: None,
                confirmed_by: None,
                is_current: true,
                created_at: Utc::now().to_rfc3339(),
                source_node_id: source_node_id.clone(),
            });
            let _ = self
                .persist_artifact_ref(
                    &source_node_id,
                    ArtifactRef {
                        artifact_id: format!("artifact_version_{version:03}"),
                        version,
                    },
                )
                .await;
            ArtifactUpdateEvent { version, payload }
        };
        self.persistent_lifecycle()?
            .save_artifact_versions(&self.session.session_id, &self.artifact_versions)
            .map_err(PlanRepairError::Store)?;
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version: update.version,
                payload: update.payload,
            })
            .await;
        Ok(())
    }
}

fn same_plan_repair_artifact_kind(left: &ArtifactPayload, right: &ArtifactPayload) -> bool {
    matches!(
        (left, right),
        (
            ArtifactPayload::WorkItemRevisionHistory { .. },
            ArtifactPayload::WorkItemRevisionHistory { .. }
        ) | (
            ArtifactPayload::PlanAmendmentManifest { .. },
            ArtifactPayload::PlanAmendmentManifest { .. }
        )
    )
}
