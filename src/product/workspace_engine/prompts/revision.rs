use super::*;
use crate::product::cadence_skills::routing_reference::{
    RoutingReferenceContext, generation_cadence_routing_rules_reference,
};
use crate::product::workspace_engine::review::trusted_review_comments;

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
        let review = self.latest_review_verdict.as_ref();
        let context = self.routing_reference_context();
        // spec-design-dialog-revision T4：author 反馈修订路径（pending_revision_context 存在且无 review
        // verdict）走专用增量修订 prompt，reviewer 返修路径维持既有 delta/full 分流。
        // T5/M-1：谓词提取为共享 helper is_author_feedback_revision（decisions.rs），与 provider_drive.rs 同语义。
        let prompt = if self.is_author_feedback_revision() {
            let feedback = self.pending_revision_context.as_deref().unwrap_or_default();
            self.build_author_revision_prompt(feedback, resume_provider_session_id.is_some())
        } else {
            let review =
                review.ok_or_else(|| "review verdict is unavailable for revision".to_string())?;
            if resume_provider_session_id.is_some() {
                self.build_revision_delta_prompt(review, &context)
            } else {
                self.build_revision_full_prompt(&artifact, review, &context)
            }
        };

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Orchestrator,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id,
            permission_mode: permission_mode_for_provider(
                &provider,
                self.session.permission_modes.author.clone(),
            ),
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    pub(crate) fn build_revision_delta_prompt(
        &self,
        review: &ReviewVerdict,
        context: &RoutingReferenceContext,
    ) -> String {
        let mut prompt = String::new();
        prompt.push_str("请作为 author 继续返修当前 Workspace 产物。\n\n");
        prompt.push_str(&generation_cadence_routing_rules_reference(context));
        prompt.push_str(
            "当前阶段：真实 Provider resume 后的 bounded revision。\n必调 Skill：using-superpowers，并按当前返修范围重新路由；若范围、架构或验收变化，停止并交给 Aria 既有审批 gate。\n",
        );
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("这是对当前 provider 会话的增量返修指令。不要重新调研完整上下文，不要只解释；请基于本会话已有上下文、上一版 artifact 和以下 reviewer 意见，直接输出完整更新后的 artifact markdown。\n");
        self.append_missing_context_notes_to_prompt(&mut prompt);
        prompt.push_str("\nReviewer 审核意见:\n\n");
        if let Some(comments) = trusted_review_comments(review) {
            prompt.push_str(comments);
        }
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
        context: &RoutingReferenceContext,
    ) -> String {
        let mut prompt = String::new();
        prompt.push_str("请作为 author 返修当前 Workspace 产物。\n\n");
        if !self.has_direct_cadence_routing_rules_system_context() {
            prompt.push_str(&generation_cadence_routing_rules_reference(context));
        }
        prompt.push_str(
            "当前阶段：候选产物 bounded revision。\n必调 Skill：using-superpowers，并按当前返修范围重新路由；若范围、架构或验收变化，停止并交给 Aria 既有审批 gate。\n",
        );
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("会话上下文（滑动窗口压缩；最近 2 轮保留原文）:\n");
        prompt.push_str(
            &super::compact_history(super::HistoryCompactionInput {
                messages: &self.session.messages,
                artifact_versions: &self.artifact_versions,
                timeline_nodes: &self.timeline_nodes,
                latest_review_verdict: Some(review),
                mode: super::HistoryCompactionMode::Author,
            })
            .rendered,
        );
        self.append_missing_context_notes_to_prompt(&mut prompt);
        prompt.push_str("\n上一版 Artifact:\n\n");
        prompt.push_str(artifact);
        prompt.push_str("\n\nReviewer 审核意见:\n\n");
        if let Some(comments) = trusted_review_comments(review) {
            prompt.push_str(comments);
        }
        prompt.push_str("\n\nReviewer 摘要:\n");
        prompt.push_str(&review.summary);
        if let Some(context) = &self.pending_revision_context {
            prompt.push_str("\n\n用户补充信息优先级高于 Reviewer 审核意见；如二者冲突，以用户补充信息为准，并在更新后的 artifact 中体现用户补充要求。\n用户补充信息:\n");
            prompt.push_str(context);
        }
        self.append_author_artifact_output_contract(&mut prompt, true);
        prompt.push_str(super::author_artifact_skeleton_example(
            &self.session.workspace_type,
        ));
        prompt.push_str("\n\n请根据以上审核意见修改产物，输出完整更新后的 artifact markdown。\n");
        prompt
    }
}

#[cfg(test)]
mod revision_routing_reference_tests {
    use super::*;
    use crate::product::cadence_skills::routing_reference::{
        LogicalPolicyReference, RoutingReferenceContext,
    };
    use crate::product::checkpoint_store::CheckpointStore;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn logical_context() -> RoutingReferenceContext {
        RoutingReferenceContext::Logical(LogicalPolicyReference {
            policy_id: "policy/project_0001/logical_0001/3".into(),
            policy_revision: 3,
            policy_digest: "sha256:abc123".into(),
            authority_root: "/data/aria/aggregate/policy".into(),
        })
    }

    fn engine_for_revision() -> WorkspaceEngine {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let store = Arc::new(CheckpointStore::new(
            tempfile::tempdir().unwrap().path().to_path_buf(),
        ));
        let session = WorkspaceSession {
            session_id: "sess_revision_routing".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            stage: WorkspaceStage::Revision,
            messages: Vec::new(),
            artifact: Some(ArtifactPayload::Markdown {
                markdown: "# Story Spec".to_string(),
                diff: None,
            }),
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::Codex),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            provisional_reviewer_provider: None,
            reviewer_enabled_at_start: None,
            superpowers_enabled: true,
            openspec_enabled: true,
            provider_conversations: Vec::new(),
            repository_path: None,
        };
        WorkspaceEngine::new(store, event_tx, session)
    }

    fn review_verdict() -> ReviewVerdict {
        ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "请补全结构。".to_string(),
            summary: "缺少 artifact schema".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
            structured_output_diagnostic: None,
        }
    }

    #[test]
    fn revision_delta_prompt_legacy_uses_on_demand_generation_reference() {
        let engine = engine_for_revision();
        let prompt =
            engine.build_revision_delta_prompt(&review_verdict(), &RoutingReferenceContext::Legacy);
        assert!(prompt.contains("按需查阅"), "{prompt}");
        assert!(prompt.contains("忽略规则约束"), "{prompt}");
        assert!(!prompt.contains("项目规则未加载"), "{prompt}");
        assert!(!prompt.contains("完整读取"), "{prompt}");
        assert!(!prompt.contains("只报告阻塞"), "{prompt}");
        assert_eq!(prompt.matches("[cadence_project_rules]").count(), 1);
    }

    #[test]
    fn revision_delta_prompt_logical_declares_policy_envelope() {
        let engine = engine_for_revision();
        let prompt = engine.build_revision_delta_prompt(&review_verdict(), &logical_context());
        assert!(
            prompt.contains("authority_root: /data/aria/aggregate/policy"),
            "{prompt}"
        );
        assert!(
            prompt.contains("policy_id: policy/project_0001/logical_0001/3"),
            "{prompt}"
        );
        assert!(prompt.contains("policy_revision: 3"), "{prompt}");
        assert!(prompt.contains("sha256:abc123"), "{prompt}");
        assert!(prompt.contains("不作为政策正文"), "{prompt}");
        assert!(prompt.contains("只报告阻塞"), "{prompt}");
    }

    #[test]
    fn revision_full_prompt_legacy_uses_on_demand_generation_reference() {
        let engine = engine_for_revision();
        let prompt = engine.build_revision_full_prompt(
            "# Existing Artifact",
            &review_verdict(),
            &RoutingReferenceContext::Legacy,
        );
        assert!(prompt.contains("按需查阅"), "{prompt}");
        assert!(prompt.contains("忽略规则约束"), "{prompt}");
        assert!(!prompt.contains("项目规则未加载"), "{prompt}");
        assert!(!prompt.contains("完整读取"), "{prompt}");
        assert!(!prompt.contains("只报告阻塞"), "{prompt}");
        assert_eq!(prompt.matches("[cadence_project_rules]").count(), 1);
    }

    #[test]
    fn revision_full_prompt_reuses_routing_reference_present_in_generation_context() {
        let mut engine = engine_for_revision();
        let context = logical_context();
        let logical_text = generation_cadence_routing_rules_reference(&context);
        engine.session.messages.push(SessionMessage {
            id: "msg_generation_context".to_string(),
            role: "system".to_string(),
            content: format!("[workflow_discipline]\n{logical_text}"),
            checkpoint_id: None,
            created_at: "2026-06-30T00:00:00Z".to_string(),
        });

        let prompt =
            engine.build_revision_full_prompt("# Existing Artifact", &review_verdict(), &context);

        assert_eq!(
            prompt.matches("[cadence_project_rules]").count(),
            1,
            "full revision must reuse, not repeat, the logical routing reference already present: {prompt}"
        );
        assert!(
            prompt.contains("authority_root: /data/aria/aggregate/policy"),
            "{prompt}"
        );
    }

    #[test]
    fn revision_full_prompt_logical_declares_policy_envelope() {
        let engine = engine_for_revision();
        let prompt = engine.build_revision_full_prompt(
            "# Existing Artifact",
            &review_verdict(),
            &logical_context(),
        );
        assert!(
            prompt.contains("authority_root: /data/aria/aggregate/policy"),
            "{prompt}"
        );
        assert!(
            prompt.contains("policy_id: policy/project_0001/logical_0001/3"),
            "{prompt}"
        );
        assert!(prompt.contains("policy_revision: 3"), "{prompt}");
        assert!(prompt.contains("sha256:abc123"), "{prompt}");
        assert!(prompt.contains("不作为政策正文"), "{prompt}");
        assert!(prompt.contains("只报告阻塞"), "{prompt}");
    }
}
