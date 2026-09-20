//! 方案X阶段2：AI run 完成后的聚合代码库 involved/change_order 回写与可见诊断。
//! 自 `provider_drive.rs` 拆出（1200 行守护，Wave 3.1）：纯移动零语义变化。

use super::*;

impl WorkspaceEngine {
    /// 方案X阶段2：AI run 完成后解析 structured output，将 AI 声明的 involved/change_order
    /// 回写到 Spec record（Task 4 集成点，复用 Task 3 的 parse_* 与 Task 2 的 update_*）。
    ///
    /// 仅对**聚合代码库** Story/Design 生效（record.logical_codebase_ref 非空）；传统单仓
    /// （logical_codebase_ref 为 None）不执行回写，保持既有 append_version 行为不变。
    ///
    /// 返回：
    /// - `Ok(None)`：回写成功，或回写不适用（单仓/非 Story/Design），无需诊断。
    /// - `Ok(Some(message))`：回写被跳过但需可见——AI 未产出结构化 involved（缺 tag /
    ///   schema 非法，REQ-PLN-04）或规划上下文 snapshot 缺失，消息供调用方记录诊断。
    /// - `Err(ProductStoreError)`：真实 store 错误（加载 record/snapshot 失败、update
    ///   校验失败），由调用方转为可见诊断（约束4：不沿用 `let _ =` 静默吞）。
    pub(super) fn write_back_aggregate_output(
        &self,
        store: &LifecycleStore,
        artifact_markdown: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        if !matches!(
            self.session.workspace_type,
            WorkspaceType::Story | WorkspaceType::Design
        ) {
            return Ok(None);
        }
        let project_id = &self.session.project_id;
        let issue_id = &self.session.issue_id;
        let entity_id = &self.session.entity_id;

        // 从 record 读 logical_codebase_ref（单仓 None → 跳过回写）。
        let record = store.load_existing_spec(project_id, issue_id, entity_id)?;
        let logical_codebase_ref = match &record {
            ExistingSpecRecord::Story { record, .. } => record.logical_codebase_ref,
            ExistingSpecRecord::Design { record, .. } => record.logical_codebase_ref,
        };
        let Some(logical_codebase_ref) = logical_codebase_ref else {
            return Ok(None);
        };

        // 从 snapshot 读 effective_member_ids（权威；缺失 → 可见诊断，无法校验）。
        let effective_member_ids = match PlanningContextSnapshotStore::new(store.app_paths())
            .load(project_id, issue_id)?
        {
            Some(snapshot) => snapshot.effective_member_ids,
            None => {
                return Ok(Some(
                    "聚合代码库回写跳过：规划上下文 snapshot 缺失，无法校验 effective_member_ids"
                        .to_string(),
                ));
            }
        };

        match self.session.workspace_type {
            WorkspaceType::Story => {
                let output = match parse_story_aggregate_output(artifact_markdown) {
                    Ok(output) => output,
                    Err(error) => {
                        return Ok(Some(format!(
                            "Story 聚合回写跳过：AI 未产出结构化 involved（REQ-PLN-04）：{error:?}"
                        )));
                    }
                };
                let scope = AggregateStorySpecScope {
                    logical_codebase_ref,
                    effective_member_ids,
                    involved_repository_ids: output.involved_repository_ids,
                    focus_repository_id: output.focus_repository_id,
                };
                store.update_story_spec_aggregate(project_id, issue_id, entity_id, &scope)?;
            }
            WorkspaceType::Design => {
                let output = match parse_design_aggregate_output(artifact_markdown) {
                    Ok(output) => output,
                    Err(error) => {
                        return Ok(Some(format!(
                            "Design 聚合回写跳过：AI 未产出结构化 involved（REQ-PLN-04）：{error:?}"
                        )));
                    }
                };
                let scope = AggregateDesignSpecScope {
                    logical_codebase_ref,
                    effective_member_ids,
                    involved_repository_ids: output.involved_repository_ids,
                    change_order: output.change_order,
                };
                store.update_design_spec_aggregate(project_id, issue_id, entity_id, &scope)?;
            }
            _ => return Ok(None),
        }
        Ok(None)
    }

    /// 将聚合回写诊断追加为 session 的 system 消息（约束4：回写失败可见，不静默吞）。
    /// 需在 checkpoint 创建**之后**调用：避免改变 `session.messages.len()` 或末尾消息，
    /// 干扰 checkpoint 对最后一条 assistant 消息的 checkpoint_id 绑定。
    pub(super) fn append_aggregate_write_back_diagnostic(&mut self, message: &str) {
        tracing::warn!(message, "aggregate involved write-back diagnostic");
        let msg_id = format!("msg_{:03}", self.session.messages.len() + 1);
        let now = chrono::Utc::now().to_rfc3339();
        self.session.messages.push(SessionMessage {
            id: msg_id,
            role: "system".to_string(),
            content: message.to_string(),
            checkpoint_id: None,
            created_at: now,
        });
        if let Some(store) = &self.lifecycle_store {
            let _ = store.append_workspace_message(
                &self.session.session_id,
                "system".to_string(),
                message.to_string(),
            );
        }
    }
}
