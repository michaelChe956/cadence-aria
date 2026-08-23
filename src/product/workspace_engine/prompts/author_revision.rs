use super::*;

impl WorkspaceEngine {
    /// AuthorConfirm 反馈修订专用 prompt：当前产物 + 用户自由文本反馈 → 增量修订 + 改动摘要。
    /// 独立于 build_revision_full_prompt/build_revision_delta_prompt（后者面向 reviewer 返修，
    /// 且 add-monorepo 分支已参数化，避免触碰）。
    ///
    /// `_resumed_session` 为后续启用 compact_history 预留：届时仅 fresh 会话注入压缩历史。
    /// 本期无论 fresh 或 resume，Design 均注入相同的结构输出契约。
    pub(crate) fn build_author_revision_prompt(
        &self,
        feedback: &str,
        _resumed_session: bool,
    ) -> String {
        let artifact = self
            .session
            .artifact
            .as_ref()
            .map(|a| a.markdown_or_empty())
            .unwrap_or_default();
        let is_design = self.session.workspace_type == WorkspaceType::Design;
        let mut prompt = String::new();
        prompt.push_str("请作为 author 基于用户反馈对当前 Workspace 产物做增量修订。\n\n");
        prompt.push_str("## 修订规则\n");
        prompt.push_str("- 只修改与反馈相关的部分，保持其余章节原样（增量修订，不是重写）。\n");
        prompt
            .push_str("- 若反馈要求全部重写（如方向调整），保留仍然有效的事实性内容并整体重组。\n");
        prompt.push_str("- 输出修订后的完整产物正文（markdown），并在文末追加「## 改动摘要」小节：逐条列出本次改动的位置与原因。\n\n");
        prompt.push_str("## 当前产物\n\n");
        if is_design {
            prompt.push_str(
                "下方当前产物用四反引号围栏包裹，产物内部的三反引号代码块属于产物内容，不要当作输入边界；输入围栏仅用于界定材料，输出请按 artifact fence 契约重新包裹。\n\n````\n",
            );
        } else {
            prompt.push_str("```\n");
        }
        prompt.push_str(artifact);
        if is_design {
            prompt.push_str("\n````\n\n");
        } else {
            prompt.push_str("\n```\n\n");
        }
        prompt.push_str("## 用户反馈\n\n");
        prompt.push_str(feedback.trim());
        prompt.push('\n');

        if is_design {
            self.append_missing_context_notes_to_prompt(&mut prompt);
            self.append_author_artifact_output_contract(&mut prompt, true);
            prompt.push_str(author_artifact_skeleton_example(&WorkspaceType::Design));
        }

        prompt
    }
}
