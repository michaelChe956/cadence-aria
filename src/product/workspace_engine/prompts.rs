use super::*;
use crate::product::cadence_skills::routing_reference::direct_cadence_routing_rules_reference;

mod review;
mod review_context;
mod review_repair;
mod reviewer_context_filter;
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

fn initial_author_runtime_contract(
    workspace_type: &WorkspaceType,
    include_direct_routing_reference: bool,
) -> String {
    let (phase, required_skill) = match workspace_type {
        WorkspaceType::Story => (
            "新需求/行为变化的 Story 候选探索",
            "using-superpowers → brainstorming",
        ),
        WorkspaceType::Design => (
            "已确认 Story 范围内的 Design 候选探索",
            "using-superpowers → brainstorming",
        ),
        WorkspaceType::WorkItemPlan => (
            "已确认 Story/Design 后的 Work Item Plan 候选规划",
            "using-superpowers → writing-plans",
        ),
        WorkspaceType::WorkItem => (
            "已确认 Work Item Plan 范围内的单项 Work Item 候选规划",
            "using-superpowers → writing-plans",
        ),
    };

    let routing_reference = if include_direct_routing_reference {
        direct_cadence_routing_rules_reference()
    } else {
        ""
    };

    format!(
        "{routing_reference}当前阶段：{phase}。\n必调 Skill：{required_skill}。\n前置 gate：仅生成候选产物；Aria 的人工确认与 daemon canonical writeback 边界保持不变。\n\n",
    )
}

pub(crate) fn build_artifact_retry_prompt(
    workspace_type: &WorkspaceType,
    previous_output: &str,
    blocking_reasons: &[String],
) -> String {
    let artifact_name = workspace_type_title(workspace_type);
    let mut prompt = format!(
        "上一轮已结束，但没有输出完整 artifact。\n\
         不要继续调研，不要只解释。\n\
         请基于已有上下文和刚才读取的文件，立即输出完整 ```artifact``` {artifact_name}。\n\
         只能输出一个完整 artifact fenced block；不要拆成多个 artifact block，不要在 artifact 内输出 <thinking>。\n\
         如仍有需要用户确认的问题，必须先使用 AskUserQuestion 等结构化交互；不要把未解决问题写进最终 artifact 的待确认项/open_items，若 schema 包含待确认项则写“无”。\n"
    );
    if !blocking_reasons.is_empty() {
        prompt.push_str("\n具体失败原因:\n");
        for reason in blocking_reasons {
            prompt.push_str("- ");
            prompt.push_str(reason);
            prompt.push('\n');
        }
    }
    let previous_output = previous_output.trim();
    if !previous_output.is_empty() {
        prompt.push_str("\n上一轮可见输出:\n");
        prompt.push_str(previous_output);
        prompt.push('\n');
    }
    prompt.push('\n');
    if let Some(schema) = author_artifact_schema_contract_for(workspace_type) {
        prompt.push_str(&schema);
    }
    prompt.push_str(structured_interaction_artifact_decision_contract(
        workspace_type,
    ));
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
        "{}\
         当前阶段：候选产物审查。\n\
         必调 Skill：using-superpowers。\n\
         前置 gate：仅只读审核当前材料；Aria 的人工确认与 daemon canonical writeback 边界保持不变。\n\
         {intro}\
         <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n\
         {schema}\n\
         </ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n",
        direct_cadence_routing_rules_reference(),
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
            permission_mode: permission_mode_for_provider(
                &provider,
                self.session.permission_modes.author.clone(),
            ),
            structured_output_contract: None,
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
            permission_mode: permission_mode_for_provider(
                &author_provider,
                self.session.permission_modes.author.clone(),
            ),
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        }
    }

    pub(crate) fn build_prompt(&self, user_content: &str) -> String {
        let mut prompt = initial_author_runtime_contract(
            &self.session.workspace_type,
            !self.has_direct_cadence_routing_rules_system_context(),
        );
        if !self.has_author_artifact_schema_system_context()
            && let Some(schema) = author_artifact_schema_contract_for(&self.session.workspace_type)
        {
            prompt.push_str(&schema);
        }
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

    fn has_author_artifact_schema_system_context(&self) -> bool {
        self.session.messages.iter().any(|message| {
            message.role == "system" && message.content.contains(ARTIFACT_SCHEMA_CONTRACT_MARKER)
        })
    }

    pub(crate) fn has_direct_cadence_routing_rules_system_context(&self) -> bool {
        self.session.messages.iter().any(|message| {
            message.role == "system"
                && message
                    .content
                    .contains(direct_cadence_routing_rules_reference())
        })
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
        if !prompt.contains(ARTIFACT_SCHEMA_CONTRACT_MARKER)
            && let Some(schema) = author_artifact_schema_contract_for(&self.session.workspace_type)
        {
            prompt.push_str(&schema);
        }
        prompt.push_str(structured_interaction_artifact_decision_contract(
            &self.session.workspace_type,
        ));
    }
}

fn structured_interaction_artifact_decision_contract(
    workspace_type: &WorkspaceType,
) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => {
            "如果本轮或历史会话包含结构化交互审计记录（daemon 捕获的 AskUserQuestion、requestUserInput 或 text_fallback 回答），更新后的 Story Spec 必须在 artifact 正文加入或维护 ## 用户确认决策，使用 author-decision-* 稳定 ID 记录问题、用户选择、来源机制，并把影响范围、需求或验收的决策绑定到对应 [REQ-*]/[AC-*]；已解决的选择不得再写入 ## 待确认项。实现细节类选择只记录为 Design 阶段输入，不要固化成 Story 范围或验收标准。"
        }
        WorkspaceType::Design => {
            "如果本轮或历史会话包含结构化交互审计记录（daemon 捕获的 AskUserQuestion、requestUserInput 或 text_fallback 回答），更新后的 Design Spec 必须把用户确认决策写入 ## 设计决策 或 ## 追踪关系，保留 author-decision-* 或映射到 [DEC-*]，并绑定到来源 [REQ-*]/[AC-*]/[DEC-*]。"
        }
        WorkspaceType::WorkItem => {
            "如果本轮或历史会话包含结构化交互审计记录（daemon 捕获的 AskUserQuestion、requestUserInput 或 text_fallback 回答），更新后的 Work Item 必须在目标、范围或追踪关系中写明相关用户确认决策 author-decision-*，并绑定到来源需求/设计/验收 ID。"
        }
        WorkspaceType::WorkItemPlan => {
            "如果本轮或历史会话包含结构化交互审计记录（daemon 捕获的 AskUserQuestion、requestUserInput 或 text_fallback 回答），更新后的 Work Item Plan 必须在计划范围、任务拆分或追踪关系中写明相关用户确认决策 author-decision-*，并绑定到来源 Story/Design ID。"
        }
    }
}

/// 逻辑代码库分支的 Story prompt 注入片段（Task 7）。
///
/// 附在 Story 生成/修订 prompt 末尾：渲染紧凑成员 inventory 清单，并附加「必须列出涉及仓库，
/// 不确定即 blocker」指令。`inventory_injection.rendered` 来自
/// `render_compact_inventory`（已按预算截断），`effective_member_ids` 来自
/// `PlanningContextSnapshot.effective_member_ids`（权威），用于在指令中明示可选仓库范围。
///
/// 接入点：由 Web Story 生成/修订入口在逻辑代码库分支调用（`LogicalCodebaseFeature::is_enabled()`
/// 且 issue 有 codebase-selection.json 时，经 `PlanningContextResolver::build` 取 inventory_injection
/// 后注入）。本 task 已实现并单测，Web 接入在本计划后续 task。
#[allow(dead_code)]
pub fn aggregate_story_scope_prompt(
    inventory_rendered: &str,
    effective_member_ids: &[crate::product::logical_codebase::LogicalRepositoryId],
) -> String {
    let mut prompt = String::new();
    prompt.push_str("\n\n## 聚合代码库成员清单（involved repositories 必须从此集合中选取）：\n");
    prompt.push_str(inventory_rendered);
    prompt.push_str("\n## 聚合视野要求\n");
    prompt.push_str(
        "本次 Story 位于聚合代码库。你必须在 artifact 中明确列出涉及的逻辑仓库 \n\
         (`involved_repository_ids`，只能取上述成员清单中的 logical_repository_id)。\n\
         若无法确定具体涉及仓库，必须明确声明并进入 blocker，禁止猜测、禁止默认全部成员、\n\
         禁止回落到任意单一 primary 仓库。",
    );
    if !effective_member_ids.is_empty() {
        prompt.push_str("\n可选仓库范围（logical_repository_id）：");
        for member in effective_member_ids {
            prompt.push_str(&format!("\n- {member:?}"));
        }
    }
    prompt
}

/// 逻辑代码库分支的 Design prompt 注入片段（Task 8 修复）。
///
/// 附在 Design 生成/修订 prompt 末尾：渲染紧凑成员 inventory 清单，并附加「必须列出涉及仓库
/// 与改动顺序，不确定即 blocker」指令。`inventory_injection.rendered` 来自
/// `render_compact_inventory`（已按预算截断），`effective_member_ids` 来自
/// `PlanningContextSnapshot.effective_member_ids`（权威），用于在指令中明示可选仓库范围。
///
/// 与 `aggregate_story_scope_prompt` 同模式：`involved_repository_ids` 只能取成员清单中的
/// logical_repository_id；`change_order` 作为 WorkItem `depends_on` 依据（Task 9 消费），
/// 顺序为执行顺序图（如「先改公共契约 → 再改 provider → 最后改 consumer」），必须恰好覆盖
/// 全部 involved 仓库且不重复；AI 不确定涉及仓库或顺序时必须进入 blocker，禁止猜测、
/// 禁止默认全部成员、禁止回落到任意单一 primary 仓库。
///
/// 接入点：由 Web Design 生成/修订入口在逻辑代码库分支调用（`LogicalCodebaseFeature::is_enabled()`
/// 且 issue 有 codebase-selection.json 时，经 `PlanningContextResolver::build` 取 inventory_injection
/// 后注入）。本 task 已实现并单测，Web 接入在本计划后续 task。
#[allow(dead_code)]
pub fn aggregate_design_scope_prompt(
    inventory_rendered: &str,
    effective_member_ids: &[crate::product::logical_codebase::LogicalRepositoryId],
) -> String {
    let mut prompt = String::new();
    prompt.push_str("\n\n## 聚合代码库成员清单（involved repositories 必须从此集合中选取）：\n");
    prompt.push_str(inventory_rendered);
    prompt.push_str("\n## 聚合视野要求\n");
    prompt.push_str(
        "本次 Design 位于聚合代码库。你必须在 artifact 中明确列出涉及的逻辑仓库 \n\
         (`involved_repository_ids`，只能取上述成员清单中的 logical_repository_id)。\n\
         若无法确定具体涉及仓库，必须明确声明并进入 blocker，禁止猜测、禁止默认全部成员、\n\
         禁止回落到任意单一 primary 仓库。\n\n",
    );
    prompt.push_str(
        "改动顺序（change_order）是执行顺序图而非服务调用图，将作为后续 Work Item 的 \n\
         depends_on 依据。change_order 必须恰好覆盖全部 involved_repository_ids 且不重复；\n\
         顺序按契约依赖推进，例如「先改公共契约 → 再改 provider → 最后改 consumer」。\n\
         若无法确定涉及仓库或改动顺序，必须明确声明并进入 blocker，禁止猜测。",
    );
    if !effective_member_ids.is_empty() {
        prompt.push_str("\n可选仓库范围（logical_repository_id）：");
        for member in effective_member_ids {
            prompt.push_str(&format!("\n- {member:?}"));
        }
    }
    prompt
}

/// 逻辑代码库分支的 WorkItem Outline prompt 注入片段（Task 9）。
///
/// Outline 的每一个 item 都必须声明目标逻辑仓库。候选 Draft 将逐字继承该 target，
/// Final Compile 只接受 `IssueCodebaseSelection` 的有效成员，绝不猜测 primary。
#[allow(dead_code)] // 定义后由后续 Web 接入 task 接线（与 Task 7/8 的 aggregate prompt 一致）
pub fn aggregate_work_item_target_scope_prompt(
    inventory_rendered: &str,
    effective_member_ids: &[crate::product::logical_codebase::LogicalRepositoryId],
) -> String {
    let mut prompt = String::new();
    prompt.push_str("\n\n## 聚合代码库成员清单（WorkItem target 必须从此集合中选取）：\n");
    prompt.push_str(inventory_rendered);
    prompt.push_str("\n## WorkItem 目标仓库要求\n");
    prompt.push_str(
        "本次 WorkItem Plan 位于聚合代码库。work_item_outlines[] 的每一项必须提供 \\\n         target_repository_id，且只能取上述成员清单中的 logical_repository_id。\\n\
         target_repository_id 不确定、缺失或不在有效成员中时必须进入 blocker；禁止猜测、\\n\
         禁止回落到任意 primary 仓库。多个 WorkItem 可以使用同一 target_repository_id。",
    );
    if !effective_member_ids.is_empty() {
        prompt.push_str("\n可选目标仓库范围（logical_repository_id）：");
        for member in effective_member_ids {
            prompt.push_str(&format!("\n- {member:?}"));
        }
    }
    prompt
}

#[cfg(test)]
mod aggregate_work_item_target_scope_prompt_tests {
    use super::aggregate_work_item_target_scope_prompt;
    use crate::product::logical_codebase::LogicalRepositoryId;
    use uuid::Uuid;

    #[test]
    fn aggregate_work_item_target_scope_prompt_requires_member_target_and_blocker() {
        let api = LogicalRepositoryId(Uuid::from_u128(1));
        let web = LogicalRepositoryId(Uuid::from_u128(2));
        let prompt = aggregate_work_item_target_scope_prompt("api | service", &[api, web]);

        assert!(prompt.contains("target_repository_id"));
        assert!(prompt.contains("只能取上述成员清单"));
        assert!(prompt.contains("禁止回落到任意 primary 仓库"));
        assert!(prompt.contains("00000000-0000-0000-0000-000000000001"));
        assert!(prompt.contains("00000000-0000-0000-0000-000000000002"));
    }
}

#[cfg(test)]
mod aggregate_scope_prompt_tests {
    use super::{aggregate_design_scope_prompt, aggregate_story_scope_prompt};
    use crate::product::logical_codebase::LogicalRepositoryId;
    use uuid::Uuid;

    const fn stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    const API: LogicalRepositoryId = LogicalRepositoryId(stable_uuid(0x0001));
    const WEB: LogicalRepositoryId = LogicalRepositoryId(stable_uuid(0x0002));

    #[test]
    fn aggregate_story_scope_prompt_lists_inventory_and_blocker_directive() {
        let inventory = "00000000-0000-7000-8000-000000000001 | api | api/ | service\n";
        let prompt = aggregate_story_scope_prompt(inventory, &[API, WEB]);

        assert!(
            prompt.contains("聚合代码库成员清单"),
            "缺成员清单标题：{prompt}"
        );
        assert!(prompt.contains("api/ | service"), "缺成员行：{prompt}");
        assert!(
            prompt.contains("involved_repository_ids"),
            "缺 involved_repository_ids 指令：{prompt}"
        );
        assert!(
            prompt.contains("禁止回落到任意单一 primary 仓库"),
            "缺禁止 primary 回落指令：{prompt}"
        );
        // 有效成员 ID 列出以限定 involved 取值范围。
        assert!(prompt.contains("00000000-0000-7000-8000-000000000001"));
        assert!(prompt.contains("00000000-0000-7000-8000-000000000002"));
    }

    #[test]
    fn aggregate_design_scope_prompt_lists_inventory_involved_change_order_and_blocker() {
        let inventory = "00000000-0000-7000-8000-000000000001 | api | api/ | service\n";
        let prompt = aggregate_design_scope_prompt(inventory, &[API, WEB]);

        assert!(
            prompt.contains("聚合代码库成员清单"),
            "缺成员清单标题：{prompt}"
        );
        assert!(prompt.contains("api/ | service"), "缺成员行：{prompt}");
        assert!(
            prompt.contains("involved_repository_ids"),
            "缺 involved_repository_ids 指令：{prompt}"
        );
        assert!(
            prompt.contains("禁止回落到任意单一 primary 仓库"),
            "缺禁止 primary 回落指令：{prompt}"
        );
        // change_order 作为 WorkItem depends_on 依据，且必须恰好覆盖全部 involved 仓库。
        assert!(
            prompt.contains("change_order"),
            "缺 change_order 指令：{prompt}"
        );
        assert!(
            prompt.contains("depends_on"),
            "缺 depends_on 依据说明：{prompt}"
        );
        assert!(
            prompt.contains("先改公共契约") && prompt.contains("最后改 consumer"),
            "缺改动顺序示例（公共契约 → provider → consumer）：{prompt}"
        );
        assert!(
            prompt.contains("恰好覆盖全部 involved_repository_ids"),
            "缺 change_order 覆盖 involved 指令：{prompt}"
        );
        // 不确定涉及仓库或改动顺序 → blocker，禁止猜测。
        assert!(
            prompt.contains("进入 blocker，禁止猜测"),
            "缺 blocker 指令：{prompt}"
        );
        // 有效成员 ID 列出以限定 involved 取值范围。
        assert!(prompt.contains("00000000-0000-7000-8000-000000000001"));
        assert!(prompt.contains("00000000-0000-7000-8000-000000000002"));
    }
}
