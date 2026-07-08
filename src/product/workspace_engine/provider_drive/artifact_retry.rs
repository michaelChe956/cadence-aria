use super::*;

impl WorkspaceEngine {
    pub(crate) fn workspace_artifact_blocking_reasons(&self, full_content: &str) -> Vec<String> {
        let artifact_markdown = extract_artifact_content(full_content);
        validate_workspace_artifact_constraints(&artifact_markdown, &self.session.workspace_type)
            .blocking_reasons()
    }

    pub(crate) async fn begin_artifact_retry_node(
        &mut self,
        previous_node_id: Option<&str>,
        agent: Option<ProviderName>,
        blocking_reasons: &[String],
    ) -> Option<String> {
        let previous_node = previous_node_id.and_then(|node_id| {
            self.timeline_nodes
                .iter()
                .find(|node| node.node_id == node_id)
                .cloned()
        })?;
        let _ = self.flush_stream_buffer(&previous_node.node_id).await;
        let artifact_name = workspace_type_title(&self.session.workspace_type);
        let summary = if blocking_reasons.is_empty() {
            format!("Provider 未返回有效的 {artifact_name} artifact")
        } else {
            format!(
                "Provider 未返回有效的 {artifact_name} artifact：{}",
                blocking_reasons.join("；")
            )
        };
        self.update_timeline_node(
            &previous_node.node_id,
            TimelineNodeStatus::Failed,
            Some(summary),
        )
        .await;

        let retry_attempt = previous_node
            .retry
            .as_ref()
            .map(|retry| retry.retry_attempt + 1)
            .unwrap_or(1);
        let retry_error_message = if blocking_reasons.is_empty() {
            format!("缺失或无效的 {artifact_name} artifact")
        } else {
            blocking_reasons.join("；")
        };
        Some(
            self.create_timeline_node_with_retry(
                TimelineNodeDraft {
                    node_type: previous_node.node_type.clone(),
                    agent,
                    stage: workspace_stage_from_ws_stage(&previous_node.stage),
                    round: previous_node.round,
                    title: format!("{artifact_name} 自动续写"),
                    summary: None,
                    status: TimelineNodeStatus::Active,
                },
                Some(TimelineNodeRetry {
                    retry_of_node_id: previous_node.node_id.clone(),
                    retry_attempt,
                    retry_reason: "自动续写缺失或无效 artifact".to_string(),
                    retry_error: TimelineNodeRetryError {
                        code: "workspace_artifact_invalid".to_string(),
                        message: retry_error_message,
                    },
                }),
            )
            .await,
        )
    }

    pub(crate) fn build_artifact_retry_input(
        &self,
        base_input: &StreamingProviderInput,
        previous_output: &str,
        provider_session_id: Option<String>,
    ) -> StreamingProviderInput {
        let mut input = base_input.clone();
        let blocking_reasons = self.workspace_artifact_blocking_reasons(previous_output);
        input.prompt = build_artifact_retry_prompt(
            &self.session.workspace_type,
            previous_output,
            &blocking_reasons,
        );
        if let Some(provider_session_id) = provider_session_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        {
            input.resume_provider_session_id = Some(provider_session_id);
        }
        input
    }
}
