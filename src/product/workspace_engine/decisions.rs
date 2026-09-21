use super::*;

// 退役留档（T5/REQ-RET-02）：context blocker resolution 三 helper
//（empty_design_context_capabilities/estimate_context_resolution_tokens/
// format_context_blocker_resolution_markdown）随 ContextBlocker 处理器删除
//（wp5-attribution-table.md §3）。

impl WorkspaceEngine {
    // 退役留档（T5/REQ-RET-02）：handle_author_decision（legacy author 逐段决策）
    // 随 AuthorDecision 消息族删除——SC 无消费（wp5-attribution-table.md §2）；
    // story/design 会话经 cockpit 的 author 决策面=T4 §4 登记的「T5 后 legacy
    // 决策通道整体终止」限制（产物确认走 HTTP confirm 端点）。

    /// spec-design-dialog-revision T5/M-1：author 反馈修订分流谓词（pending 存在且无 review verdict）。
    /// prompts/revision.rs 与 provider_drive.rs 两处共用，避免重复实现漂移。
    pub(crate) fn is_author_feedback_revision(&self) -> bool {
        self.pending_revision_context.is_some() && self.latest_review_verdict.is_none()
    }

    /// 兼容旧测试的评审启动：未启用 review 时进入人工确认；已启用时复用当前
    /// `start_review` 的 CrossReview/Fake 快速路径。
    #[cfg(test)]
    pub(crate) async fn start_review_or_skip(&mut self) {
        if self.session.review_rounds == 0 || self.session.reviewer_provider.is_none() {
            self.enter_human_confirm(Some("未启用交叉审核，等待人工确认".to_string()))
                .await;
            return;
        }
        self.start_review().await;
    }

    /// 只进 CrossReview 不再跳 HumanConfirm 的评审启动（跳过路径已由 AcceptFinalize 显式覆盖）。
    /// Fake provider 快速路径保留：标记 Skipped 并进入 HumanConfirm。
    pub(crate) async fn start_review(&mut self) {
        self.transition_stage(WorkspaceStage::CrossReview).await;
        let round = self.next_review_round();
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let review_node_id = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::ReviewerRun,
                agent: Some(reviewer.clone()),
                stage: WorkspaceStage::CrossReview,
                round: Some(round),
                title: format!("Review Round {round}"),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;

        if reviewer == ProviderName::Fake {
            self.update_timeline_node(
                &review_node_id,
                TimelineNodeStatus::Skipped,
                Some("未执行真实 review（Fake 快速路径）".to_string()),
            )
            .await;
            self.mark_latest_artifact_reviewed(Some(ProviderName::Fake), None);
            self.enter_human_confirm(Some("等待人工确认".to_string()))
                .await;
        }
    }

    /// 定稿当前产物：先过 confirm gate，再提交定稿（[`Self::commit_finalize_artifact`]）。
    /// 由 HumanConfirm::Confirm 分支（handle_confirm）与 AuthorDecision::AcceptFinalize 共用。
    pub(crate) async fn finalize_current_artifact(&mut self, summary: &str) -> Result<(), String> {
        self.validate_confirm_aggregate_spec_gate()?;
        self.commit_finalize_artifact(summary).await;
        Ok(())
    }

    /// 定稿提交面（gate 已过）：完成在途节点、标记最新产物人工确认、durable 落 Confirmed、
    /// 终态 stage 与 Completed 节点。
    ///
    /// HTTP confirm 端点在自己的 durable 写（带 `ConfirmAggregateGateError` → 4xx/404 映射的
    /// gate）之后复用本方法，避免第二实现让引擎停在门态（F-31 fix round k3 P2：Fake reviewer
    /// 快速路径的二次确认此前走 store-only 定稿，缺 `mark_latest_artifact_confirmed`、
    /// Completed 节点与终态 stage——durable 时间线与引擎投影分叉）。
    pub(crate) async fn commit_finalize_artifact(&mut self, summary: &str) {
        self.complete_active_node(Some(summary.to_string())).await;
        self.mark_latest_artifact_confirmed(Some("human".to_string()));
        if let Some(store) = &self.lifecycle_store {
            if let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Confirmed,
            ) {
                self.session.session_status = record.status;
            }
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
                summary: Some(summary.to_string()),
                status: TimelineNodeStatus::Completed,
            })
            .await;
    }

    // 退役留档（T5/REQ-RET-02）：legacy 决策引擎面随消息族删除——
    // handle_work_item_plan_outline_decision/handle_review_decision/
    // skip_work_item_plan_optional_findings（AuthorDecision+ReviewDecision
    // Response+SelectRevisionPath 唯一入口）、handle_human_confirm
    // （HumanConfirm 唯一入口）、handle_work_item_plan_context_blocker_
    // decision（ContextBlocker 决策族，oracle P2 二选一判定=(b) 随 L2 删——
    // 三入口全 legacy 流 SC 不触达）、append_work_item_plan_context_blocker_
    // resolution（唯一调用方=ContextBlocker 处理器）。处置与消费链见
    // wp5-attribution-table.md §1/§3；SC 门关门=typed 三命令，非 SC 流
    // approve=Confirm 帧直连 handle_confirm，在途 legacy 会话决策拒绝=
    // REQ-RET-03 登记限制。
    pub(crate) async fn enter_human_confirm_for_work_item_plan_author_failure(
        &mut self,
        _findings: &[WorkItemSplitFinding],
    ) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            )
        {
            self.session.session_status = record.status;
        }
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::HumanConfirm,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "WorkItemPlan validate 连续失败".to_string(),
                summary: Some("author 多次重生仍 validate 失败，需人工介入".to_string()),
                status: TimelineNodeStatus::Active,
            })
            .await;
    }

    pub(crate) async fn enter_work_item_plan_context_blocker(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            )
        {
            self.session.session_status = record.status;
        }
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanContextBlocker,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "WorkItemPlan 上下文补充".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
    }

    pub(crate) async fn enter_author_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::AuthorConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Author 结果确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            )
        {
            self.session.session_status = record.status;
        }
    }

    pub(crate) async fn enter_work_item_plan_outline_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanOutlineConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "WorkItemPlan Outline 确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            )
        {
            self.session.session_status = record.status;
        }
    }

    pub(crate) async fn enter_work_item_generation_mode(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemGenerationMode,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Work Item 生成模式选择".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            )
        {
            self.session.session_status = record.status;
        }
    }

    pub(crate) async fn enter_work_item_draft_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemDraftConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Work Item Draft 确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            )
        {
            self.session.session_status = record.status;
        }
    }

    pub(crate) async fn enter_work_item_batch_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemBatchConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Work Item Batch 确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            )
        {
            self.session.session_status = record.status;
        }
    }

    pub(crate) async fn enter_human_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::HumanConfirm,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "人工确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            )
        {
            self.session.session_status = record.status;
        }
        let _ = self
            .event_tx
            .send(EngineEvent::HumanGateOpened {
                stage: self.session.stage.as_str().to_string(),
            })
            .await;
    }
}
