use super::*;

impl WorkspaceEngine {
    /// AuthorConfirm 阶段标记/取消标记单个 WorkItem 的 revert。
    ///
    /// **不产生新 artifact_version**：改 `session.artifact` 与当前 is_current
    /// `ArtifactVersion.payload` 的 candidate meta，再推同 version 的 `EngineEvent::ArtifactUpdate`。
    pub async fn apply_revert_mark(
        &mut self,
        work_item_id: &str,
        feedback: Option<String>,
        clear: bool,
    ) -> Result<(), String> {
        let payload = self
            .session
            .artifact
            .clone()
            .ok_or("no artifact to mark revert on")?;
        let mut candidate = match payload {
            ArtifactPayload::WorkItemPlanCandidate { candidate } => candidate,
            _ => return Err("artifact is not a WorkItemPlanCandidate".into()),
        };
        let wi = candidate
            .work_items
            .iter_mut()
            .find(|w| w.id == work_item_id)
            .ok_or_else(|| format!("work_item {} not in candidate", work_item_id))?;
        if clear {
            wi.meta.reverted = false;
            wi.meta.revert_feedback = None;
        } else {
            wi.meta.reverted = true;
            wi.meta.revert_feedback = feedback;
        }

        // 更新 session.artifact + 当前 ArtifactVersion.payload（不 push artifact_versions，version 不变）
        let current_version = self
            .artifact_versions
            .iter()
            .rev()
            .find(|v| v.is_current)
            .map(|v| v.version)
            .ok_or_else(|| "no current artifact version to apply revert mark on".to_string())?;
        let payload = ArtifactPayload::WorkItemPlanCandidate {
            candidate: candidate.clone(),
        };
        self.session.artifact = Some(payload.clone());
        if let Some(version) = self
            .artifact_versions
            .iter_mut()
            .rev()
            .find(|v| v.is_current)
        {
            version.payload = payload.clone();
            self.persist_artifact_versions();
        }

        // 推同 version 的 ArtifactUpdate（前端据此刷新 candidate 展示）
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version: current_version,
                payload,
            })
            .await;
        Ok(())
    }
}
