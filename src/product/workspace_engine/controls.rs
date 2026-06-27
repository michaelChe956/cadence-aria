use super::*;

impl WorkspaceEngine {
    pub async fn handle_rollback(&mut self, checkpoint_id: &str) -> Result<(), String> {
        let target = self
            .checkpoint_store
            .rollback_to(&self.session.session_id, checkpoint_id)
            .map_err(|e| format!("rollback failed: {e}"))?;

        let keep_count = target.message_index as usize;
        self.session.messages.truncate(keep_count);

        if let Some(stage) = WorkspaceStage::from_stage_name(&target.stage)
            && self.session.stage != stage
        {
            self.transition_stage(stage).await;
        }

        self.session.artifact = target.artifact_snapshot.clone();
        if let Some(store) = &self.lifecycle_store {
            let _ = store.truncate_workspace_session_messages(
                &self.session.session_id,
                keep_count,
                workspace_status_for_stage(&self.session.stage),
            );
        }

        Ok(())
    }

    pub async fn handle_confirm(&mut self) -> Result<WorkspaceConfirmOutcome, String> {
        match self.session.stage {
            WorkspaceStage::HumanConfirm => {
                self.complete_active_node(Some("已确认通过".to_string()))
                    .await;
                self.mark_latest_artifact_confirmed(Some("human".to_string()));
                match self.session.workspace_type {
                    WorkspaceType::WorkItemPlan => {
                        let (plan, new_sessions) = self.confirm_work_item_plan().await?;
                        self.transition_stage(WorkspaceStage::Completed).await;
                        let _ = self
                            .create_timeline_node(TimelineNodeDraft {
                                node_type: TimelineNodeType::Completed,
                                agent: None,
                                stage: WorkspaceStage::Completed,
                                round: None,
                                title: "WorkItemPlan 已确认".to_string(),
                                summary: Some(format!(
                                    "plan {} confirmed，已建立 {} 个子 WorkItem session",
                                    plan.id,
                                    new_sessions.len()
                                )),
                                status: TimelineNodeStatus::Completed,
                            })
                            .await;

                        return Ok(WorkspaceConfirmOutcome::WorkItemPlan {
                            child_sessions: new_sessions,
                        });
                    }
                    _ => {
                        if let Some(store) = &self.lifecycle_store {
                            let _ = store.update_workspace_session_status(
                                &self.session.session_id,
                                WorkspaceSessionStatus::Confirmed,
                            );
                            let _ = match self.session.workspace_type {
                                WorkspaceType::Story | WorkspaceType::Design => store
                                    .update_spec_confirmation_status(
                                        &self.session.project_id,
                                        &self.session.issue_id,
                                        &self.session.entity_id,
                                        LifecycleConfirmationStatus::Confirmed,
                                    )
                                    .map(|_| ()),
                                WorkspaceType::WorkItem => store
                                    .update_work_item_plan_status(
                                        &self.session.project_id,
                                        &self.session.issue_id,
                                        &self.session.entity_id,
                                        WorkItemPlanStatus::Confirmed,
                                    )
                                    .map(|_| ()),
                                WorkspaceType::WorkItemPlan => Ok(()),
                            };
                        }
                        self.transition_stage(WorkspaceStage::Completed).await;
                        let _ = self
                            .create_timeline_node(TimelineNodeDraft {
                                node_type: TimelineNodeType::Completed,
                                agent: None,
                                stage: WorkspaceStage::Completed,
                                round: None,
                                title: "流程完成".to_string(),
                                summary: Some("已确认通过".to_string()),
                                status: TimelineNodeStatus::Completed,
                            })
                            .await;
                    }
                }
            }
            WorkspaceStage::Running => {
                self.transition_stage(WorkspaceStage::CrossReview).await;
            }
            _ => {}
        }
        Ok(WorkspaceConfirmOutcome::None)
    }

    /// WorkItemPlan 确认：plan/work_items Draft -> Confirmed，并幂等创建子 WorkItem session。
    pub(crate) async fn confirm_work_item_plan(
        &mut self,
    ) -> Result<(IssueWorkItemPlan, Vec<WorkspaceSessionRecord>), String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or("lifecycle_store unavailable")?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();

        let current_plan = lifecycle
            .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
            .map_err(|e| format!("load plan failed: {e}"))?;
        let plan = match current_plan.status {
            crate::product::models::IssueWorkItemPlanStatus::Draft => {
                lifecycle
                    .confirm_issue_work_item_plan(&project_id, &issue_id, &plan_id)
                    .map_err(|e| format!("confirm plan failed: {e}"))?
                    .0
            }
            crate::product::models::IssueWorkItemPlanStatus::Confirmed => current_plan,
            crate::product::models::IssueWorkItemPlanStatus::ChangeRequested => {
                return Err("cannot confirm a change_requested WorkItemPlan".to_string());
            }
        };
        if plan.work_item_ids.is_empty() {
            return Err(
                "cannot confirm WorkItemPlan without compiled WorkItem records; run Final Compile successfully first"
                    .to_string(),
            );
        }

        let _created_sessions = lifecycle
            .ensure_work_item_sessions_for_plan(
                &project_id,
                &issue_id,
                &plan_id,
                self.session.author_provider.clone(),
                self.session.reviewer_provider.clone(),
                self.session.review_rounds,
                self.session.superpowers_enabled,
                self.session.openspec_enabled,
            )
            .map_err(|e| format!("ensure child sessions failed: {e}"))?;
        let plan_work_item_ids: HashSet<String> = plan.work_item_ids.iter().cloned().collect();
        let child_sessions = lifecycle
            .list_workspace_sessions(&project_id, &issue_id)
            .map_err(|e| format!("list child sessions failed: {e}"))?
            .into_iter()
            .filter(|session| {
                session.workspace_type == WorkspaceType::WorkItem
                    && plan_work_item_ids.contains(&session.entity_id)
            })
            .collect::<Vec<_>>();

        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Confirmed,
            );
        }

        Ok((plan, child_sessions))
    }

    pub fn handle_abort(&mut self) {
        self.cancel.cancel();
    }

    pub fn set_provider(&mut self, role: &str, provider: ProviderName) -> Result<(), String> {
        if self.session.stage != WorkspaceStage::PrepareContext {
            return Err("provider selection is locked after generation starts".to_string());
        }

        match role {
            "author" => {
                self.session.author_provider = provider;
                Ok(())
            }
            "reviewer" => {
                self.session.reviewer_provider = Some(provider);
                Ok(())
            }
            _ => Err(format!("unknown provider role: {role}")),
        }?;

        if let Some(store) = &self.lifecycle_store {
            let reviewer_provider = self
                .session
                .reviewer_provider
                .clone()
                .unwrap_or(ProviderName::Codex);
            store
                .update_workspace_session_providers(
                    &self.session.session_id,
                    self.session.author_provider.clone(),
                    reviewer_provider,
                )
                .map_err(|error| format!("persist provider selection failed: {error}"))?;
        }

        Ok(())
    }

    pub async fn update_artifact(&mut self, payload: ArtifactPayload) -> ArtifactRef {
        self.session.artifact = Some(payload.clone());
        for version in &mut self.artifact_versions {
            version.is_current = false;
        }
        let version = self.artifact_versions.len() as u32 + 1;
        let source_node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "timeline_node_unknown".to_string());
        self.artifact_versions.push(ArtifactVersion {
            version,
            payload: payload.clone(),
            generated_by: self.session.author_provider.clone(),
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            source_node_id,
        });
        self.persist_artifact_versions();
        let source_node_id = self
            .artifact_versions
            .last()
            .map(|version| version.source_node_id.clone())
            .unwrap_or_else(|| "timeline_node_unknown".to_string());
        let artifact_ref = ArtifactRef {
            artifact_id: format!("artifact_version_{version:03}"),
            version,
        };
        let _ = self
            .persist_artifact_ref(&source_node_id, artifact_ref.clone())
            .await;
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version,
                payload: payload.clone(),
            })
            .await;
        artifact_ref
    }

    pub(crate) async fn replace_current_artifact_payload(
        &mut self,
        payload: ArtifactPayload,
    ) -> Result<u32, String> {
        self.session.artifact = Some(payload.clone());
        let Some(version) = self
            .artifact_versions
            .iter_mut()
            .rev()
            .find(|version| version.is_current)
        else {
            let artifact_ref = self.update_artifact(payload).await;
            return Ok(artifact_ref.version);
        };
        version.payload = payload.clone();
        let version_number = version.version;
        self.persist_artifact_versions();
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version: version_number,
                payload,
            })
            .await;
        Ok(version_number)
    }
}
