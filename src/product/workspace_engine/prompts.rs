use super::*;

mod review;
mod revision;

pub(crate) fn workspace_type_title(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "Story Spec",
        WorkspaceType::Design => "Design Spec",
        WorkspaceType::WorkItem => "Work Item",
        WorkspaceType::WorkItemPlan => "Work Item Plan",
    }
}

pub(crate) fn normalize_generation_prompt(
    content: String,
    workspace_type: &WorkspaceType,
) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        format!(
            "Workspace 类型: {}\n开始生成 {}",
            workspace_type_title(workspace_type),
            workspace_type_title(workspace_type)
        )
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn build_artifact_retry_prompt(
    workspace_type: &WorkspaceType,
    previous_output: &str,
) -> String {
    let artifact_name = workspace_type_title(workspace_type);
    let mut prompt = format!(
        "上一轮已结束，但没有输出完整 artifact。\n\
         不要继续调研，不要只解释。\n\
         请基于已有上下文和刚才读取的文件，立即输出完整 ```artifact``` {artifact_name}。\n"
    );
    let previous_output = previous_output.trim();
    if !previous_output.is_empty() {
        prompt.push_str("\n上一轮可见输出:\n");
        prompt.push_str(previous_output);
        prompt.push('\n');
    }
    prompt
}

pub(crate) fn structured_output_nonce() -> String {
    uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect()
}

pub(crate) fn reviewer_output_contract(nonce: &str, schema: &str, intro: &str) -> String {
    format!(
        "{intro}\
         <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n\
         {schema}\n\
         </ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n"
    )
}

impl WorkspaceEngine {
    pub(crate) fn build_streaming_input(
        &self,
        user_content: &str,
        prompt_mode: AuthorPromptMode,
    ) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        let provider = self.session.author_provider.clone();
        let resume_provider_session_id =
            self.provider_resume_session_id(ProviderConversationRole::Author, &provider);

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Orchestrator,
            prompt: match prompt_mode {
                AuthorPromptMode::FullConversation => self.build_prompt(user_content),
                AuthorPromptMode::DeltaOnly => user_content.to_string(),
            },
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    pub fn build_work_item_plan_streaming_input(
        &self,
        provider_type: ProviderType,
        prompt: String,
        worktree_path: String,
        author_provider: ProviderName,
    ) -> StreamingProviderInput {
        let resume_provider_session_id =
            self.provider_resume_session_id(ProviderConversationRole::Author, &author_provider);
        StreamingProviderInput {
            provider_type,
            role: AdapterRole::WorkItemSplitter,
            prompt,
            working_dir: PathBuf::from(worktree_path),
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        }
    }

    pub(crate) fn build_prompt(&self, user_content: &str) -> String {
        let mut prompt = String::new();
        let last_current_user_message_index =
            self.session.messages.len().checked_sub(1).filter(|index| {
                let message = &self.session.messages[*index];
                message.role == "user" && message.content == user_content
            });
        for (index, msg) in self.session.messages.iter().enumerate() {
            if Some(index) == last_current_user_message_index {
                continue;
            }
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }

        for note in self.missing_context_note_summaries() {
            prompt.push_str(&format!("[user]: {note}\n"));
        }

        if let Some(index) = last_current_user_message_index {
            let msg = &self.session.messages[index];
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        } else {
            prompt.push_str(&format!("[user]: {user_content}\n"));
        }
        prompt
    }

    pub(crate) fn missing_context_note_summaries(&self) -> Vec<String> {
        let known_message_contents = self
            .session
            .messages
            .iter()
            .map(|message| message.content.trim().to_string())
            .collect::<Vec<_>>();

        self.timeline_nodes
            .iter()
            .filter_map(|node| {
                if node.node_type != TimelineNodeType::ContextNote {
                    return None;
                }
                let note = node.summary.as_deref()?.trim();
                (!note.is_empty()
                    && !known_message_contents
                        .iter()
                        .any(|content| content.as_str() == note))
                .then(|| note.to_string())
            })
            .collect()
    }

    pub(crate) fn append_missing_context_notes_to_prompt(&self, prompt: &mut String) {
        let notes = self.missing_context_note_summaries();
        if notes.is_empty() {
            return;
        }

        prompt.push_str("\n准备阶段用户补充上下文:\n");
        for note in notes {
            prompt.push_str(&format!("- {note}\n"));
        }
    }

    pub(crate) fn append_author_artifact_output_contract(
        &self,
        prompt: &mut String,
        mentions_prior_artifact: bool,
    ) {
        prompt.push_str("\n\n输出格式契约：");
        if mentions_prior_artifact {
            prompt.push_str(
                "上一版 Artifact 是 daemon 已提取的 markdown，外层 artifact fence 已被剥离；不要把上一版 Artifact 的裸 markdown 形态当作原始返回格式样例。",
            );
        } else {
            prompt.push_str(
                "当前 provider 会话中的既有 artifact 是 daemon 已提取的 markdown，外层 artifact fence 可能已被剥离；不要把裸 markdown 形态当作原始返回格式样例。",
            );
        }
        prompt.push_str("原始返回必须使用完整 artifact fenced block，fence 内第一行必须是 ");
        prompt.push_str(workspace_type_title(&self.session.workspace_type));
        prompt.push_str(
            " 一级标题。正文内部包含 ``` 代码块时，外层使用四反引号 ````artifact ... ````，避免和内部代码块冲突。\
             过程说明必须放在 artifact fence 外，最终候选产物必须放在 artifact fence 内。",
        );
    }
}
