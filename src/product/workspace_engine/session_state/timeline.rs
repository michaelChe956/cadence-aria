use super::*;

fn invalid_artifact_message(base: String, blocking_reasons: &[String]) -> String {
    if blocking_reasons.is_empty() {
        return base;
    }
    format!("{base}：{}", blocking_reasons.join("；"))
}

impl WorkspaceEngine {
    pub(crate) async fn transition_stage(&mut self, new_stage: WorkspaceStage) {
        self.session.stage = new_stage;
        let _ = self
            .event_tx
            .send(EngineEvent::StageChange {
                stage: self.session.stage.as_str().to_string(),
            })
            .await;
    }

    pub(crate) async fn finish_failed_run(&mut self) {
        // 批 2 T2（REQ-WIGA-09）：durable 已 Terminated 的 session，迟到失败
        // 收口不覆盖状态、不重置 stage（终止终态；store 层另有第二道防线）。
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.get_workspace_session(&self.session.session_id)
            && record.status == WorkspaceSessionStatus::Terminated
        {
            self.session.session_status = WorkspaceSessionStatus::Terminated;
            self.active_run_id = None;
            return;
        }
        // P2 GAP-E/G（Task 0.1）：SC 已收敛 durable 终态 Failed 的会话保留
        // Failed（人工显式重驱的恢复现场），其他 flow 维持旧 Open 重置。
        let status = if self.session.flow_kind
            == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
            && self.session.single_candidate_phase
                == Some(crate::product::models::SingleCandidatePhase::Failed)
        {
            WorkspaceSessionStatus::Failed
        } else {
            WorkspaceSessionStatus::Open
        };
        self.session.session_status = status.clone();
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) =
                store.update_workspace_session_status(&self.session.session_id, status)
        {
            self.session.session_status = record.status;
        }
        self.active_run_id = None;
        self.transition_stage(WorkspaceStage::PrepareContext).await;
    }

    pub async fn finish_active_run_with_failed_node(&mut self, message: impl Into<String>) {
        let message = message.into();
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) async fn finish_empty_assistant_output(&mut self) {
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: "Provider completed without assistant output".to_string(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Failed,
                Some("Provider 未返回助手内容".to_string()),
            )
            .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) async fn finish_invalid_workspace_artifact(&mut self, blocking_reasons: &[String]) {
        let artifact_name = workspace_type_title(&self.session.workspace_type);
        let message = invalid_artifact_message(
            format!("Provider 未返回有效的 {artifact_name} artifact"),
            blocking_reasons,
        );
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: message.clone(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) async fn finish_invalid_workspace_artifact_after_retry(
        &mut self,
        blocking_reasons: &[String],
    ) {
        let artifact_name = workspace_type_title(&self.session.workspace_type);
        let message = invalid_artifact_message(
            format!("自动续写后仍未返回有效的 {artifact_name} artifact"),
            blocking_reasons,
        );
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: message.clone(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) fn workspace_requires_artifact_gate(&self) -> bool {
        matches!(
            self.session.workspace_type,
            WorkspaceType::Story
                | WorkspaceType::Design
                | WorkspaceType::WorkItem
                | WorkspaceType::WorkItemPlan
        )
    }

    pub(crate) async fn finish_aborted_run(&mut self) {
        if let Some(node_id) = self.active_node_id.clone() {
            // REQ-WIGA-16：中止不是失败——节点落「中止」态（Aborted），不得以
            // failed 表述「中止」；summary 沿用既有文案逐字不变。
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Aborted,
                Some("运行已中止".to_string()),
            )
            .await;
        }
        self.finish_failed_run().await;
    }

    pub(crate) async fn handle_permission_timeout(
        &mut self,
        permission_id: String,
        node_id: Option<String>,
    ) {
        tracing::warn!(permission_id = %permission_id, "permission timed out; aborting active run");
        // 诊断打点（claude×轻 握手谜团第 2 轮，不改行为）：权限超时是 engine 自取消
        // 位点之一（会取消在途 provider 握手/流）。
        eprintln!(
            "[aria-cancellation] workspace permission_timeout trigger=permission_timeout session_id={} role=none permission_id={permission_id}",
            self.session.session_id
        );
        if let Some(node_id) = node_id.as_deref() {
            let _ = self
                .persist_permission_timeout(node_id, permission_id.clone())
                .await;
            let _ = self.flush_stream_buffer(node_id).await;
            self.update_timeline_node(
                node_id,
                TimelineNodeStatus::Failed,
                Some("权限请求超时，运行已中止".to_string()),
            )
            .await;
        }
        self.active_run_id = None;
        self.cancel.cancel();
        let _ = self
            .event_tx
            .send(EngineEvent::PermissionTimeout {
                permission_id,
                node_id,
            })
            .await;
        self.finish_failed_run().await;
    }

    pub(crate) async fn create_timeline_node(&mut self, draft: TimelineNodeDraft) -> String {
        let node_id = format!("timeline_node_{:03}", self.timeline_nodes.len() + 1);
        let node = TimelineNode {
            node_id: node_id.clone(),
            node_type: draft.node_type,
            agent: draft.agent,
            stage: ws_stage(&draft.stage),
            round: draft.round,
            status: draft.status,
            title: draft.title,
            summary: draft.summary,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: self
                .session
                .artifact
                .as_ref()
                .map(|_| "artifact_current".to_string()),
            provider_config_snapshot: self.provider_config_snapshot(),
            retry: None,
        };
        self.timeline_nodes.push(node.clone());
        self.active_node_id = Some(node_id.clone());
        self.persist_timeline_nodes();
        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeCreated { node })
            .await;
        node_id
    }

    pub(crate) async fn append_completed_timeline_event(
        &mut self,
        node_type: TimelineNodeType,
        stage: WorkspaceStage,
        title: String,
        summary: Option<String>,
        status: TimelineNodeStatus,
        make_active: bool,
    ) -> TimelineNode {
        let now = chrono::Utc::now().to_rfc3339();
        let node_id = format!("timeline_node_{:03}", self.timeline_nodes.len() + 1);
        let node = TimelineNode {
            node_id: node_id.clone(),
            node_type,
            agent: None,
            stage: ws_stage(&stage),
            round: None,
            status,
            title,
            summary,
            started_at: now.clone(),
            completed_at: Some(now),
            duration_ms: Some(0),
            artifact_ref: self
                .session
                .artifact
                .as_ref()
                .map(|_| "artifact_current".to_string()),
            provider_config_snapshot: self.provider_config_snapshot(),
            retry: None,
        };
        self.timeline_nodes.push(node.clone());
        if make_active {
            self.active_node_id = Some(node_id);
        }
        self.persist_timeline_nodes();
        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeCreated { node: node.clone() })
            .await;
        node
    }

    pub(crate) async fn create_timeline_node_with_retry(
        &mut self,
        draft: TimelineNodeDraft,
        retry: Option<TimelineNodeRetry>,
    ) -> String {
        let node_id = format!("timeline_node_{:03}", self.timeline_nodes.len() + 1);
        let node = TimelineNode {
            node_id: node_id.clone(),
            node_type: draft.node_type,
            agent: draft.agent,
            stage: ws_stage(&draft.stage),
            round: draft.round,
            status: draft.status,
            title: draft.title,
            summary: draft.summary,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: self
                .session
                .artifact
                .as_ref()
                .map(|_| "artifact_current".to_string()),
            provider_config_snapshot: self.provider_config_snapshot(),
            retry,
        };
        self.timeline_nodes.push(node.clone());
        self.active_node_id = Some(node_id.clone());
        self.persist_timeline_nodes();
        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeCreated { node })
            .await;
        node_id
    }

    pub(crate) fn empty_node_detail_for(&self, node: &TimelineNode) -> NodeDetail {
        NodeDetail {
            node_id: node.node_id.clone(),
            session_id: self.session.session_id.clone(),
            node_type: node.node_type.clone(),
            status: node.status.clone(),
            agent_role: match node.node_type {
                TimelineNodeType::AuthorRun
                | TimelineNodeType::WorkItemPlanOutlineRun
                | TimelineNodeType::WorkItemDraftRun
                | TimelineNodeType::WorkItemBatchRun => Some(AgentRole::Author),
                TimelineNodeType::ReviewerRun
                | TimelineNodeType::WorkItemPlanOutlineReview
                | TimelineNodeType::WorkItemDraftReview
                | TimelineNodeType::WorkItemBatchReview => Some(AgentRole::Reviewer),
                _ => None,
            },
            provider: node.agent.as_ref().map(|provider| ProviderSnapshot {
                name: provider_name_text(provider).to_string(),
                model: provider_name_text(provider).to_string(),
            }),
            prompt: None,
            messages: Vec::new(),
            streaming_content: String::new(),
            execution_events: Vec::new(),
            permission_events: Vec::new(),
            verdict: None,
            artifact_ref: None,
            is_revision: node.node_type == TimelineNodeType::AuthorRun
                && node.stage == WsWorkspaceStage::Revision,
            revision_feedback: None,
            base_artifact_ref: None,
            started_at: node.started_at.clone(),
            ended_at: node.completed_at.clone(),
        }
    }

    pub(crate) async fn update_node_detail<F>(
        &mut self,
        node_id: &str,
        update: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&mut NodeDetail),
    {
        let Some(store) = &self.lifecycle_store else {
            return Ok(());
        };

        let Some(node) = self
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .cloned()
            .or_else(|| {
                store
                    .load_timeline_nodes(&self.session.session_id)
                    .ok()?
                    .into_iter()
                    .find(|node| node.node_id == node_id)
            })
        else {
            return Err(format!("timeline node not found: {node_id}"));
        };

        let mut detail = match store.load_node_detail(&self.session.session_id, node_id) {
            Ok(detail) => detail,
            Err(ProductStoreError::NotFound { .. }) => self.empty_node_detail_for(&node),
            Err(error) => return Err(format!("load node detail failed: {error}")),
        };
        update(&mut detail);
        store
            .save_node_detail(&self.session.session_id, node_id, &detail)
            .map_err(|error| format!("save node detail failed: {error}"))?;
        Ok(())
    }

    pub(crate) async fn persist_prompt_snapshot(
        &mut self,
        node_id: &str,
        prompt: String,
    ) -> Result<(), String> {
        self.update_node_detail(node_id, |detail| {
            detail.prompt = Some(prompt);
        })
        .await
    }

    pub(crate) async fn complete_active_node(&mut self, summary: Option<String>) {
        let Some(node_id) = self.active_node_id.clone() else {
            return;
        };
        // F-05：Failed/Paused/Skipped 节点不得被改写为 Completed——重开场景下
        // active_node_id 可能仍指向上一次失败的 author 节点，其终态与历史字段
        // （summary/completed_at）必须原样保留。已 Completed 的节点允许幂等
        // 重放完成：review 流程先在 complete_review 落 verdict summary，路由腿
        // 再覆写流程 summary（如「Review 完成，报告已进入对话流」），这是既有
        // 双写契约，不得被守卫吞掉。
        if self.timeline_nodes.iter().any(|node| {
            node.node_id == node_id
                && matches!(
                    node.status,
                    TimelineNodeStatus::Active | TimelineNodeStatus::Completed
                )
        }) {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Completed, summary)
                .await;
        }
    }

    pub(crate) async fn update_timeline_node(
        &mut self,
        node_id: &str,
        status: TimelineNodeStatus,
        summary: Option<String>,
    ) {
        let is_terminal_status = matches!(
            status,
            TimelineNodeStatus::Completed
                | TimelineNodeStatus::Failed
                | TimelineNodeStatus::Skipped
                | TimelineNodeStatus::Aborted
        );
        let completed_at = if is_terminal_status {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            None
        };

        if let Some(node) = self
            .timeline_nodes
            .iter_mut()
            .find(|node| node.node_id == node_id)
        {
            node.status = status.clone();
            // REQ-WIGA-16：stage 与 status 必须相互一致——provider run 节点进入
            // 终态后不得仍以进行中阶段（running）表述。WorkspaceStage 值域中不存在
            // 「失败/中止」终态 stage，复用唯一终态 stage Completed（不新增对外取值）。
            //
            // 收敛（既有契约不得被本修法改写）：只归一「provider run 节点」——
            // 带 provider agent 的节点记录的是一次在途 provider 执行，其 stage=Running
            // 与终态 status 自相矛盾；无 agent 的工作流节点（如 WorkItemPlanCompile）
            // 的 stage 是「该节点所属的工作流阶段」历史标签（既有断言：compile 节点
            // 收口后仍记 running），不是执行态，不得改写。
            if is_terminal_status && node.agent.is_some() && node.stage == WsWorkspaceStage::Running
            {
                node.stage = ws_stage(&WorkspaceStage::Completed);
            }
            if summary.is_some() {
                node.summary = summary.clone();
            }
            if completed_at.is_some() {
                node.completed_at = completed_at.clone();
            }
        }
        self.persist_timeline_nodes();
        let detail_status = status.clone();
        let detail_completed_at = completed_at.clone();
        let _ = self
            .update_node_detail(node_id, |detail| {
                detail.status = detail_status;
                if detail_completed_at.is_some() {
                    detail.ended_at = detail_completed_at;
                }
            })
            .await;

        let _ = self
            .event_tx
            .send(EngineEvent::TimelineNodeUpdated {
                node_id: node_id.to_string(),
                status,
                summary,
                completed_at,
            })
            .await;
    }

    /// A2b（Task 2.6，设计 §4.3/§4.4）：LC 等待收口的旧 provider 节点幂等
    /// 终态化——仅 Active 节点落 Failed（durable timeline + node detail
    /// 同步落盘、TimelineNodeUpdated 事件广播）；已终态节点不覆写历史
    /// summary/completed_at（幂等重放零写）。summary 与纯投影派生同源
    /// （`lc_waiting_node_summary`），不引入第二套文案。
    pub(crate) async fn finalize_lc_waiting_provider_node(
        &mut self,
        node_id: &str,
        reason_code: &str,
    ) {
        let occupied = self
            .timeline_nodes
            .iter()
            .any(|node| node.node_id == node_id && node.status == TimelineNodeStatus::Active);
        if !occupied {
            return;
        }
        let summary = Some(Self::lc_waiting_node_summary(reason_code));
        self.update_timeline_node(node_id, TimelineNodeStatus::Failed, summary)
            .await;
    }

    pub(crate) fn provider_config_snapshot(&self) -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: self.session.author_provider.clone(),
            reviewer: self.session.reviewer_provider.clone(),
            review_rounds: self.session.review_rounds,
            permission_modes: self.session.permission_modes.clone(),
        }
    }

    pub(crate) fn next_review_round(&self) -> u32 {
        self.timeline_nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.node_type,
                    TimelineNodeType::ReviewerRun | TimelineNodeType::WorkItemPlanOutlineReview
                )
            })
            .count() as u32
            + 1
    }

    pub(crate) fn active_review_round(&self) -> Option<u32> {
        let active_node_id = self.active_node_id.as_ref()?;
        self.timeline_nodes
            .iter()
            .find(|node| &node.node_id == active_node_id)
            .and_then(|node| node.round)
    }

    pub(crate) fn active_node_agent(&self) -> Option<ProviderName> {
        let active_node_id = self.active_node_id.as_ref()?;
        self.timeline_nodes
            .iter()
            .find(|node| &node.node_id == active_node_id)
            .and_then(|node| node.agent.clone())
    }

    pub(crate) fn record_review_message(&mut self, content: String) {
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "reviewer".to_string(),
            content: content.clone(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "reviewer".to_string(),
                content,
            );
        }
    }

    pub(crate) fn mark_latest_artifact_reviewed(
        &mut self,
        reviewed_by: Option<ProviderName>,
        review_verdict: Option<ReviewVerdictType>,
    ) {
        // A4（SC 失败修复包）恒 is_current=false：review 标记落在最后一个
        // **普通**版本上，末尾 A4 不被标记。
        if let Some(version) = self.artifact_versions.iter_mut().rev().find(|version| {
            !matches!(
                version.payload,
                ArtifactPayload::ScFailureRepairPackage { .. }
            )
        }) {
            version.reviewed_by = reviewed_by;
            version.review_verdict = review_verdict;
            self.persist_artifact_versions();
        }
    }

    pub(crate) fn mark_latest_artifact_confirmed(&mut self, confirmed_by: Option<String>) {
        // 同上：confirm 标记只落在最后一个普通版本上。
        if let Some(version) = self.artifact_versions.iter_mut().rev().find(|version| {
            !matches!(
                version.payload,
                ArtifactPayload::ScFailureRepairPackage { .. }
            )
        }) {
            version.confirmed_by = confirmed_by;
            self.persist_artifact_versions();
        }
    }

    pub(crate) fn persist_timeline_nodes(&self) {
        if let Some(store) = &self.lifecycle_store {
            let _ = store.save_timeline_nodes(&self.session.session_id, &self.timeline_nodes);
        }
    }

    pub(crate) fn persist_artifact_versions(&self) {
        if let Some(store) = &self.lifecycle_store {
            let _ = store.save_artifact_versions(&self.session.session_id, &self.artifact_versions);
        }
    }
}

#[cfg(test)]
mod t3_termination_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
    use crate::product::models::provider::ProviderName;
    use crate::product::models::{WorkspaceSessionStatus, WorkspaceType};

    fn persistent_engine_with_status(
        root: &std::path::Path,
        durable_status: WorkspaceSessionStatus,
    ) -> (LifecycleStore, WorkspaceEngine) {
        let paths = ProductAppPaths::new(root.join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "entity_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .unwrap();
        if durable_status != WorkspaceSessionStatus::Open {
            lifecycle
                .compare_and_update_workspace_session_status(&record, durable_status)
                .unwrap();
        }
        // 引擎内存态保持 Open（模拟迟到收口现场），durable 已是目标状态。
        let mut in_memory = record.clone();
        in_memory.status = WorkspaceSessionStatus::Open;
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        let engine = WorkspaceEngine::new_persistent(
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                root.join("advance-checkpoints"),
            )),
            LifecycleStore::new(paths),
            event_tx,
            WorkspaceSession::from_record(in_memory),
        );
        (lifecycle, engine)
    }

    /// 批 2 T3（T1T2 遗留③，REQ-WIGA-09）：durable 已 Terminated 的
    /// session，迟到 `finish_aborted_run` 不覆盖状态（不以 Open/Failed
    /// 复活）、不重置 stage；对照：durable Open 的迟到收口仍写 Open。
    #[tokio::test]
    async fn finish_aborted_run_preserves_durable_terminated_session() {
        let root = tempfile::tempdir().unwrap();
        let (lifecycle, mut engine) =
            persistent_engine_with_status(root.path(), WorkspaceSessionStatus::Terminated);
        let session_id = engine.session.session_id.clone();

        engine.finish_aborted_run().await;

        assert_eq!(
            engine.session.session_status,
            WorkspaceSessionStatus::Terminated,
            "late abort finalization must not resurrect a terminated session"
        );
        assert_eq!(
            lifecycle.get_workspace_session(&session_id).unwrap().status,
            WorkspaceSessionStatus::Terminated,
        );

        // 对照：durable Open 的迟到收口维持旧语义（Open 重置写盘）。
        let (lifecycle, mut engine) =
            persistent_engine_with_status(root.path(), WorkspaceSessionStatus::Open);
        let session_id = engine.session.session_id.clone();
        engine.finish_aborted_run().await;
        assert_eq!(
            lifecycle.get_workspace_session(&session_id).unwrap().status,
            WorkspaceSessionStatus::Open,
        );
    }

    /// REQ-WIGA-16：被中止的 run 不得以「失败」表述——节点 status 落「中止」态、
    /// summary 表达「已中止」、stage 与 status 一致（终态节点不得仍停留在
    /// running）、completed_at 落盘；回归：真实失败路径仍表述为失败。
    #[tokio::test]
    async fn aborted_run_node_is_not_failed_and_stage_matches_status() {
        let root = tempfile::tempdir().unwrap();
        let (lifecycle, mut engine) =
            persistent_engine_with_status(root.path(), WorkspaceSessionStatus::Open);
        let session_id = engine.session.session_id.clone();

        let aborted_node_id = engine
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::AuthorRun,
                agent: Some(ProviderName::Fake),
                stage: WorkspaceStage::Running,
                round: None,
                title: "生成正文".to_string(),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;

        engine.finish_aborted_run().await;

        let nodes = lifecycle.load_timeline_nodes(&session_id).unwrap();
        let aborted = nodes
            .iter()
            .find(|node| node.node_id == aborted_node_id)
            .expect("aborted timeline node persisted");
        assert_ne!(
            aborted.status,
            TimelineNodeStatus::Failed,
            "被中止的 run 不得表述为失败（REQ-WIGA-16）"
        );
        assert_eq!(aborted.status, TimelineNodeStatus::Aborted);
        assert_eq!(aborted.summary.as_deref(), Some("运行已中止"));
        assert_ne!(
            aborted.stage,
            WsWorkspaceStage::Running,
            "终态节点不得仍以进行中阶段表述（stage 与 status 必须一致）"
        );
        assert_eq!(aborted.stage, WsWorkspaceStage::Completed);
        assert!(
            aborted.completed_at.is_some(),
            "中止态节点的 completed_at 必须落盘"
        );

        // 回归：真实失败路径仍表述为失败（既有 failed 语义逐字不变）。
        let failed_node_id = engine
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::AuthorRun,
                agent: Some(ProviderName::Fake),
                stage: WorkspaceStage::Running,
                round: None,
                title: "生成正文".to_string(),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;
        engine
            .finish_active_run_with_failed_node("provider 真实失败")
            .await;

        let nodes = lifecycle.load_timeline_nodes(&session_id).unwrap();
        let failed = nodes
            .iter()
            .find(|node| node.node_id == failed_node_id)
            .expect("failed timeline node persisted");
        assert_eq!(failed.status, TimelineNodeStatus::Failed);
        assert_eq!(failed.summary.as_deref(), Some("provider 真实失败"));
    }
}

/// A4（SC 失败修复包）恒 is_current=false：review/confirm 标记只落在最后一个
/// **普通**版本上——末尾 A4 不被标记，其它普通版本的 metadata 逐字段不变。
#[cfg(test)]
mod sc_published_mark_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::lifecycle_store::WorkItemPlanSessionOptions;
    use crate::product::models::WorkspaceType;
    use crate::product::models::provider::ProviderName;
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
    use crate::web::workspace_ws_types::ScEvidenceRef;
    use crate::web::workspace_ws_types::ScFailureRepairPackage;
    use crate::web::workspace_ws_types::ScFailureSourceEvidence;
    use crate::web::workspace_ws_types::ScPackageCompleteness;

    fn sc_engine(root: &std::path::Path) -> WorkspaceEngine {
        let paths = ProductAppPaths::new(root.join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "entity_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    ..Default::default()
                }),
            })
            .expect("SC session");
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        WorkspaceEngine::new_persistent(
            std::sync::Arc::new(CheckpointStore::new(root.join("advance-checkpoints"))),
            lifecycle,
            event_tx,
            WorkspaceSession::from_record(record),
        )
    }

    fn ordinary_version(version: u32) -> ArtifactVersion {
        ArtifactVersion {
            version,
            payload: ArtifactPayload::Markdown {
                markdown: format!("# 普通产物 v{version}"),
                diff: None,
            },
            generated_by: ProviderName::Fake,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: version == 2,
            created_at: "2026-11-13T00:00:00Z".to_string(),
            source_node_id: "node_sc_1".to_string(),
        }
    }

    fn sc_repair_package_version(version: u32) -> ArtifactVersion {
        // 夹具诚实性：canonical 链身份（store 真实校验 chain-v1-<64 hex>）＋显式
        // Missing 证据（夹具不构造证据存储），不用 all-zero 的假 Available。
        // chain-v1- 后缀 = sha256("p14b-mark-sc-fixture")。
        let chain_id = "chain-v1-9e3d26b7950437fe339152e4ccdad90ed0f76b9d580b9a280b6b0042105a09d0";
        let raw_missing = || ScEvidenceRef::Missing {
            code: "sc_failure_raw_missing".to_string(),
            detail: "夹具未落盘证据：终局没有 provider 原始输出".to_string(),
            round_id: "round_1".to_string(),
        };
        ArtifactVersion {
            version,
            payload: ArtifactPayload::ScFailureRepairPackage {
                sc_failure_repair_package: Box::new(ScFailureRepairPackage {
                    schema_version: 1,
                    package_id: "pkg_mark_sc_1".to_string(),
                    chain_id: chain_id.to_string(),
                    session_id: "session_sc_1".to_string(),
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    plan_id: "entity_0001".to_string(),
                    terminal_class: "compile".to_string(),
                    terminal_reason: "terminal compile failure".to_string(),
                    completeness: ScPackageCompleteness::Complete,
                    rounds: Vec::new(),
                    last_provider_output: raw_missing(),
                    last_failure_source: ScFailureSourceEvidence {
                        source_round_id: "round_1".to_string(),
                        source: raw_missing(),
                    },
                    teaching_prompts: Vec::new(),
                    evidence_log_ref: format!("sc-failure-evidence/{chain_id}"),
                    evidence_digest: "a".repeat(64),
                    evidence_through_entry_key: "entry_1".to_string(),
                    missing_evidence: Vec::new(),
                }),
            },
            generated_by: ProviderName::Fake,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: false,
            created_at: "2026-11-13T00:00:00Z".to_string(),
            source_node_id: "node_sc_1".to_string(),
        }
    }

    /// 末尾是 A4 时：review/confirm 落在最后一个**普通**版本上；A4 与更早的普通
    /// 版本逐字段不变（断言只比对标记前后的真实版本，不钉字面量）。
    #[test]
    fn mark_latest_artifact_skips_sc_repair_package_and_marks_last_ordinary() {
        let root = tempfile::tempdir().expect("temp dir");
        let mut engine = sc_engine(root.path());
        engine.artifact_versions = vec![
            ordinary_version(1),
            ordinary_version(2),
            sc_repair_package_version(3),
        ];
        let before = engine.artifact_versions.clone();

        engine.mark_latest_artifact_reviewed(
            Some(ProviderName::Codex),
            Some(ReviewVerdictType::Pass),
        );
        {
            let versions = &engine.artifact_versions;
            assert_eq!(
                versions[2], before[2],
                "A4 (SC repair package) must not be marked"
            );
            assert_eq!(
                versions[0], before[0],
                "older ordinary version must stay untouched"
            );
            assert_eq!(versions[1].reviewed_by, Some(ProviderName::Codex));
            assert_eq!(versions[1].review_verdict, Some(ReviewVerdictType::Pass));
            assert_eq!(versions[1].version, before[1].version);
            assert_eq!(versions[1].is_current, before[1].is_current);
            assert_eq!(versions[1].created_at, before[1].created_at);
            assert_eq!(versions[1].source_node_id, before[1].source_node_id);
            assert_eq!(versions[1].generated_by, before[1].generated_by);
            assert_eq!(versions[1].confirmed_by, before[1].confirmed_by);
        }

        engine.mark_latest_artifact_confirmed(Some("user".to_string()));
        {
            let versions = &engine.artifact_versions;
            assert_eq!(versions[2].confirmed_by, None, "A4 must not be confirmed");
            assert_eq!(versions[1].confirmed_by, Some("user".to_string()));
            assert_eq!(versions[0].confirmed_by, before[0].confirmed_by);
        }
    }
}
