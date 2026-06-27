use super::*;

impl CodingWorkspaceEngine {
    pub(crate) fn create_running_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::WorktreePrepare,
            title: "准备 worktree".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Git),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    pub(crate) fn create_testing_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            title: "执行测试".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Tester),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    pub(crate) fn create_coding_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Coding,
            title: "代码编写".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Author),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    pub(crate) fn create_review_request_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::ReviewRequest,
            title: "发起 review request".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Git),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    pub(crate) fn create_code_review_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            title: "代码审查".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    pub(crate) fn create_rework_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
        rework_round: u32,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Rework,
            title: format!("分析官判定 #{}", rework_round),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::System),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    pub(crate) fn create_internal_pr_review_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::InternalPrReview,
            title: "内部 PR 审查".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    pub(crate) fn create_completed_final_confirm_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let now = Utc::now().to_rfc3339();
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::FinalConfirm,
            title: "最终确认".to_string(),
            status: CodingTimelineNodeStatus::Completed,
            agent_role: Some(CodingAgentRole::System),
            summary: Some("Analyst 最终判定通过，attempt 已完成".to_string()),
            started_at: now.clone(),
            completed_at: Some(now),
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    pub(crate) fn active_worktree_prepare_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .store
            .get_timeline_nodes(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|node| {
                node.stage == CodingExecutionStage::WorktreePrepare
                    && node.status == CodingTimelineNodeStatus::Running
            })
            .map(|node| node.id))
    }

    pub(crate) fn active_timeline_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .store
            .get_timeline_nodes(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|node| {
                matches!(
                    node.status,
                    CodingTimelineNodeStatus::Pending
                        | CodingTimelineNodeStatus::Running
                        | CodingTimelineNodeStatus::Blocked
                )
            })
            .map(|node| node.id))
    }

    pub(crate) fn active_final_confirm_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .store
            .get_timeline_nodes(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|node| {
                node.stage == CodingExecutionStage::FinalConfirm
                    && matches!(
                        node.status,
                        CodingTimelineNodeStatus::Pending
                            | CodingTimelineNodeStatus::Running
                            | CodingTimelineNodeStatus::Blocked
                    )
            })
            .map(|node| node.id))
    }

    pub(crate) async fn complete_timeline_node(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        node_id: &str,
        status: CodingTimelineNodeStatus,
        summary: Option<String>,
    ) -> Result<(), ProductStoreError> {
        let completed_at = Utc::now().to_rfc3339();
        self.store.update_timeline_node_status(
            project_id,
            issue_id,
            attempt_id,
            node_id,
            status.clone(),
            summary.clone(),
            Some(completed_at.clone()),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id: node_id.to_string(),
                status,
                summary,
                completed_at: Some(completed_at),
            })
            .await;
        Ok(())
    }
}
