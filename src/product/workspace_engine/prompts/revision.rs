use super::*;

impl WorkspaceEngine {
    pub(crate) fn build_revision_input(&self) -> Result<StreamingProviderInput, String> {
        self.build_revision_input_with_resume(true)
    }

    pub(crate) fn build_revision_input_without_resume(
        &self,
    ) -> Result<StreamingProviderInput, String> {
        self.build_revision_input_with_resume(false)
    }

    pub(crate) fn build_revision_input_with_resume(
        &self,
        allow_resume: bool,
    ) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };

        let artifact = self
            .session
            .artifact
            .clone()
            .map(|payload| payload.into_markdown().unwrap_or_default())
            .unwrap_or_default();
        let provider = self.session.author_provider.clone();
        let resume_provider_session_id = if allow_resume {
            self.provider_resume_session_id(ProviderConversationRole::Author, &provider)
        } else {
            None
        };
        let review = self
            .latest_review_verdict
            .as_ref()
            .ok_or_else(|| "review verdict is unavailable for revision".to_string())?;
        let prompt = if resume_provider_session_id.is_some() {
            self.build_revision_delta_prompt(review)
        } else {
            self.build_revision_full_prompt(&artifact, review)
        };

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Orchestrator,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    pub(crate) fn build_revision_delta_prompt(&self, review: &ReviewVerdict) -> String {
        let mut prompt = String::new();
        prompt.push_str("请作为 author 继续返修当前 Workspace 产物。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("这是对当前 provider 会话的增量返修指令。不要重新调研完整上下文，不要只解释；请基于本会话已有上下文、上一版 artifact 和以下 reviewer 意见，直接输出完整更新后的 artifact markdown。\n");
        self.append_missing_context_notes_to_prompt(&mut prompt);
        prompt.push_str("\nReviewer 审核意见:\n\n");
        prompt.push_str(&review.comments);
        prompt.push_str("\n\nReviewer 摘要:\n");
        prompt.push_str(&review.summary);
        if let Some(context) = &self.pending_revision_context {
            prompt.push_str("\n\n用户补充信息优先级高于 Reviewer 审核意见；如二者冲突，以用户补充信息为准，并在更新后的 artifact 中体现用户补充要求。\n用户补充信息:\n");
            prompt.push_str(context);
        }
        self.append_author_artifact_output_contract(&mut prompt, false);
        prompt.push_str("\n\n请根据以上审核意见修改产物，输出完整更新后的 artifact markdown。\n");
        prompt
    }

    pub(crate) fn build_revision_full_prompt(
        &self,
        artifact: &str,
        review: &ReviewVerdict,
    ) -> String {
        let mut prompt = String::new();
        prompt.push_str("请作为 author 返修当前 Workspace 产物。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        self.append_missing_context_notes_to_prompt(&mut prompt);
        prompt.push_str("\n上一版 Artifact:\n\n");
        prompt.push_str(artifact);
        prompt.push_str("\n\nReviewer 审核意见:\n\n");
        prompt.push_str(&review.comments);
        prompt.push_str("\n\nReviewer 摘要:\n");
        prompt.push_str(&review.summary);
        if let Some(context) = &self.pending_revision_context {
            prompt.push_str("\n\n用户补充信息优先级高于 Reviewer 审核意见；如二者冲突，以用户补充信息为准，并在更新后的 artifact 中体现用户补充要求。\n用户补充信息:\n");
            prompt.push_str(context);
        }
        self.append_author_artifact_output_contract(&mut prompt, true);
        prompt.push_str("\n\n请根据以上审核意见修改产物，输出完整更新后的 artifact markdown。\n");
        prompt
    }
}
