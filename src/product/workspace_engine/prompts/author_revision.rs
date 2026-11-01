use super::*;

impl WorkspaceEngine {
    /// AuthorConfirm 反馈修订专用 prompt：当前产物 + 用户自由文本反馈 → 增量修订 + 改动摘要。
    /// 独立于 build_revision_full_prompt/build_revision_delta_prompt（后者面向 reviewer 返修，
    /// 且 add-monorepo 分支已参数化，避免触碰）。
    ///
    /// B1a（Task 4.1，设计 §5.2）：`resumed_session=false`（fresh provider 会话）
    /// 时注入压缩历史——fresh 会话无会话内记忆，prompt 必须自足；无历史材料
    /// （无消息且无 review verdict）时不注入空小节。反馈正文只在「## 用户反馈」
    /// 出现一次（反馈不重复 append）；trusted 材料经压缩历史携带（verdict 随
    /// `HistoryCompactionInput` 进入重放），不改变「用户反馈优先级高于 Reviewer
    /// 审核意见」的既有优先级。
    /// F-60 P0：输出契约由 build_revision_input_with_resume 出口统一装配（见 revision.rs），
    /// 本函数只负责修订指令与材料本身。
    pub(crate) fn build_author_revision_prompt(
        &self,
        feedback: &str,
        resumed_session: bool,
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
        if !resumed_session
            && (!self.session.messages.is_empty() || self.latest_review_verdict.is_some())
        {
            prompt.push_str("\n会话上下文（滑动窗口压缩；最近 2 轮保留原文）:\n");
            prompt.push_str(
                &super::compact_history(super::HistoryCompactionInput {
                    messages: &self.session.messages,
                    artifact_versions: &self.artifact_versions,
                    timeline_nodes: &self.timeline_nodes,
                    latest_review_verdict: self.latest_review_verdict.as_ref(),
                    mode: super::HistoryCompactionMode::Author,
                    detail_findings: self.is_logical_codebase_issue(),
                })
                .rendered,
            );
        }

        if is_design {
            self.append_missing_context_notes_to_prompt(&mut prompt);
        }
        // F-60 P0（因素三）：输出契约（含 fence/schema/决策归档/负面清单/骨架）
        // 不再由本分支 Design-only 追加——Story/Design/WorkItem/WorkItemPlan 的
        // 用户反馈修订统一由 build_revision_input_with_resume 出口装配到末端，
        // 补齐 F-60 确认缺口 2（此前仅 Design 有契约）。

        prompt
    }
}
