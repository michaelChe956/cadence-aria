use crate::product::cadence_skills::routing_reference::direct_cadence_routing_rules_reference;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    IssueRecord, LifecycleWorkItemRecord, OutlineContextBlockerResolution, ProviderName,
    RepositoryRecord, WorkItemDraftRecord, WorkItemGenerationMode, WorkspaceType,
};
use crate::product::workspace_engine::{allowed_outputs_for, forbidden_outputs_for};
use crate::web::error::ApiResult;
use crate::web::types::GenerateWorkItemsRequest;

use super::WorkItemSplitEngine;
use super::context::{
    collect_design_context, collect_story_context, design_context_gaps,
    merge_design_context_capabilities, summarize_repository_structure,
};
use super::schema::{WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA, WORK_ITEM_SPLIT_OUTPUT_SCHEMA};
use super::types::{
    RedoSpec, WorkItemSplitInvocation, format_context_resolutions, format_string_list,
    prompt_nonce, provider_name_to_type, structured_output_nonce, work_item_kind_text,
};

const OUTLINE_WRITE_SCOPE_RULES: &str = "\
         [write_scope_partition_rules]\n\
         依赖链上的 exclusive_write_scopes 必须互斥：如果 A depends_on B，则 A 与 B 不得拥有相同路径、父子路径或可能匹配同一文件的 glob。\n\
         integration/e2e 测试 outline 只能拥有与实现目录不共享前缀的测试、fixtures、mock 或 CI 配置路径；不要把被测功能实现目录写入测试 outline 的 exclusive_write_scopes。\n\
         不要让 outline_frontend 与 outline_integration_tests 同时拥有 web/src/**；也不要把 web/src/**/*.test.tsx 交给 integration/e2e outline，因为它会与 web/src/components/**、web/src/pages/** 等 frontend 实现范围重叠。\n\
         常见做法是 frontend outline 拥有 web/src/components/**、web/src/pages/** 及其同目录单元测试；integration_tests/e2e outline 只拥有 web/e2e/**、tests/e2e/**、fixtures/**、mocks/**、playwright.config.* 或 CI 配置。\n\
         如果两个依赖 outline 都需要改同一个 shared helper、schema、fixture 或 test harness，请拆出独立前置 outline 作为唯一 owner，其他 outline 通过 depends_on 读取 handoff；若 shared 文件位于 web/src/** 下，不要再让 frontend outline 拥有覆盖它的父级 glob。\n\
         forbidden_write_scopes 应显式写出依赖方或被依赖方已拥有的实现目录，帮助后续 draft 避免越界。\n\n";

pub const WORK_ITEM_DRAFT_PROMPT_VERSION: &str = "work_item_draft_v2";
/// Fail-closed 硬兜底：只拦截病态序列化回归（如整条持久化记录被注入 prompt）。
/// 质量预算不由本常量承担，见 WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES 的预算测试。
/// prompt 经 stdin JSON 发送给 Provider，无 OS ARG_MAX 约束；真实物理边界是模型上下文窗口。
pub(crate) const WORK_ITEM_DRAFT_PROMPT_MAX_BYTES: usize = 65_536;
/// Draft prompt 质量预算：真实规模中文 fixture 的确定性预算测试阈值。
#[cfg(test)]
pub(crate) const WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES: usize = 12_000;

fn work_item_plan_runtime_contract(role: &str) -> String {
    let workspace_type = WorkspaceType::WorkItemPlan;
    format!(
        "{}\
         当前阶段：已确认 Story/Design 后的 Work Item Plan 候选规划。\n\
         必调 Skill：using-superpowers → writing-plans。\n\
         前置 gate：Aria 的 human-confirmation gate 承接人工确认；Provider 只能输出候选，不能写入 canonical artifact。\n\n\
         [openspec_contract]\n\
         Role: {role}\n\
         - 必须基于已确认 Story Spec 与 Design Spec 的 requirement/design trace 进行拆分。\n\
         - 必须维护 Story/Design/Work Item 追踪关系，并在任务拆分中保留来源证据。\n\
         - 每个 outline/draft 必须能追溯到 source_story_spec_ids 与 source_design_spec_ids。\n\
         - 发现 Story/Design/Work Item 之间冲突、缺失验收依据或无法确定写入边界时，必须输出 blocker 或 reviewer 可处理的风险，而不是猜测。\n\
         - 不得声称已写回 OpenSpec；当前仅生成可供 daemon 后续写回 OpenSpec tasks constraints 的结构化候选。\n\n\
         [superpowers_contract]\n\
         - 必须遵守 using-superpowers 的先读规则与 writing-plans 的计划结构要求。\n\
         - 生成的是计划和任务拆分，不执行代码修改。\n\
         - 每个 outline/draft 必须给出后续 coding agent 可执行的目标、范围、非目标、TDD 顺序、结构化验证方案、依赖输入、交接输出和风险；其中 draft 只有存在目标仓库可信证据时才可给出 command，证据不足必须进入 manual/repair/blocker，不得臆造命令。\n\
         - 每个 outline/draft 的 TDD 与验证闭环必须在当前项的 exclusive_write_scopes 和已完成 depends_on handoff 下实际可执行；不得把后续 Work Item 才会提供的注册、接线、生成或部署作为当前项验证的前提。无法根据目标仓库事实建立该闭环时，必须调整拆分或进入既有 repair/blocker 路由。\n\
         - 每个 outline 必须拆到单个 Claude Code/Codex 会话可完成，并遵循最少拆分。\n\
         - 拆分目标是在每个 Work Item 能由单个 Claude Code 或 Codex coding 会话可靠完成的前提下，使 outline 数量最少。\n\
         - 必须按最大内聚任务生成，优先合并目标一致、写入范围相同或重叠、可在同一 session 完成编码与验证的工作；先合并，再证明为什么必须拆。\n\
         - estimated_context_tokens 不超过 40k 属正常范围；40001..=50000 可输出并交由 Reviewer 判断；超过 50k 必须继续拆分。\n\
         - API、数据层、UI、测试或 TDD 子步骤本身不是独立拆分理由；除用户显式拆分选项、必要外部/权限/前序结果中断点外，独立回滚边界、独立验收边界，以及写入范围/依赖交接/验证复杂度超过现有上下文代理指标时，也必须保留拆分。\n\
         - 结论必须能追溯到已提供的 Story/Design/Outline/Draft 证据。\n\n\
         [allowed_outputs]\n\
         {allowed_outputs}\n\n\
         [forbidden_outputs]\n\
         {forbidden_outputs}\n\n",
        direct_cadence_routing_rules_reference(),
        allowed_outputs = allowed_outputs_for(&workspace_type),
        forbidden_outputs = forbidden_outputs_for(&workspace_type),
    )
}

fn work_item_draft_runtime_contract() -> String {
    let workspace_type = WorkspaceType::WorkItemPlan;
    format!(
        "{}\
         当前阶段：已确认 Story/Design 后的 Work Item Plan 候选规划；必调 Skill：using-superpowers → writing-plans。\n\
         前置 gate：Aria 的 human-confirmation gate 承接人工确认；Provider 只能输出候选，不能写入 canonical artifact。\n\
         [openspec_contract]\n\
         Role: Work Item Draft author。必须基于已确认 Story Spec、Design Spec 与 source_story_spec_ids/source_design_spec_ids 追踪关系；冲突、缺失验收依据或边界不明时输出 blocker 或 reviewer 可处理风险，不得猜测。\n\
         [superpowers_contract]\n\
         遵守 using-superpowers、writing-plans、TDD 与验证纪律；只生成候选，不执行代码修改。TDD 与验证闭环必须在当前项 exclusive_write_scopes 和已完成 depends_on handoff 下实际可执行，不得把后续 Work Item 才会提供的注册、接线、生成或部署作为前提。command 仅可来自目标仓库可信证据，不得根据 WorkItemKind 推导；证据不足用 manual/repair/blocker。每项必须可由单个 Claude Code/Codex 会话完成，estimated_context_tokens 不得超过 50k。\n\
         [allowed_outputs]\n\
         {allowed_outputs}\n\
         [forbidden_outputs]\n\
         {forbidden_outputs}\n",
        direct_cadence_routing_rules_reference(),
        allowed_outputs = allowed_outputs_for(&workspace_type),
        forbidden_outputs = forbidden_outputs_for(&workspace_type),
    )
}

impl WorkItemSplitEngine {
    pub fn build_generate_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
    ) -> ApiResult<WorkItemSplitInvocation> {
        let story_context = collect_story_context(lifecycle, request, issue)?;
        let design_context = collect_design_context(lifecycle, request, issue)?;

        let repository_structure = summarize_repository_structure(&repository.path);
        let prompt = build_split_prompt(
            request,
            issue,
            repository,
            &story_context,
            &design_context,
            &repository_structure,
        );

        Ok(WorkItemSplitInvocation {
            sentinel_nonce: prompt_nonce(&prompt),
            prompt,
            provider_type: provider_name_to_type(&author_provider),
            worktree_path: repository.path.to_string_lossy().to_string(),
            author_provider,
        })
    }

    pub fn build_outline_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        context_resolutions: &[OutlineContextBlockerResolution],
    ) -> ApiResult<WorkItemSplitInvocation> {
        let story_context = collect_story_context(lifecycle, request, issue)?;
        let design_context = collect_design_context(lifecycle, request, issue)?;
        let repository_structure = summarize_repository_structure(&repository.path);
        let capabilities = merge_design_context_capabilities(&design_context);
        let gaps = design_context_gaps(&capabilities);
        let (prompt, sentinel_nonce) = build_outline_prompt_with_nonce(
            request,
            issue,
            repository,
            &story_context,
            &design_context,
            &repository_structure,
            &gaps,
            context_resolutions,
        );

        Ok(WorkItemSplitInvocation {
            prompt,
            provider_type: provider_name_to_type(&author_provider),
            worktree_path: repository.path.to_string_lossy().to_string(),
            author_provider,
            sentinel_nonce,
        })
    }

    /// 基于同一会话中上一版 outline 进行增量返修。
    ///
    /// Prompt 不再重复 issue/story/design/repository 完整上下文，而是依赖
    /// `resume_provider_session_id` 复用 provider 会话历史；仅注入需要修改的
    /// revision feedback，要求输出完整更新后的 outline JSON。
    pub fn build_outline_revision_invocation(
        request: &GenerateWorkItemsRequest,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        feedback: &str,
    ) -> ApiResult<WorkItemSplitInvocation> {
        let (prompt, sentinel_nonce) = build_outline_revision_prompt(request, issue, feedback);

        Ok(WorkItemSplitInvocation {
            prompt,
            provider_type: provider_name_to_type(&author_provider),
            worktree_path: repository.path.to_string_lossy().to_string(),
            author_provider,
            sentinel_nonce,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_revision_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        retained: &[LifecycleWorkItemRecord],
        redo_specs: &[RedoSpec],
    ) -> ApiResult<WorkItemSplitInvocation> {
        let story_context = collect_story_context(lifecycle, request, issue)?;
        let design_context = collect_design_context(lifecycle, request, issue)?;

        let repository_structure = summarize_repository_structure(&repository.path);
        let prompt = build_revision_prompt(
            request,
            issue,
            repository,
            retained,
            redo_specs,
            &story_context,
            &design_context,
            &repository_structure,
        );

        Ok(WorkItemSplitInvocation {
            sentinel_nonce: prompt_nonce(&prompt),
            prompt,
            provider_type: provider_name_to_type(&author_provider),
            worktree_path: repository.path.to_string_lossy().to_string(),
            author_provider,
        })
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn build_outline_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    story_context: &[String],
    design_context: &[String],
    repository_structure: &str,
    design_context_gaps: &[String],
    context_resolutions: &[OutlineContextBlockerResolution],
) -> String {
    build_outline_prompt_with_nonce(
        request,
        issue,
        repository,
        story_context,
        design_context,
        repository_structure,
        design_context_gaps,
        context_resolutions,
    )
    .0
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_outline_prompt_with_nonce(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    story_context: &[String],
    design_context: &[String],
    repository_structure: &str,
    design_context_gaps: &[String],
    context_resolutions: &[OutlineContextBlockerResolution],
) -> (String, String) {
    let nonce = structured_output_nonce();
    let runtime_contract = work_item_plan_runtime_contract("WorkItemPlan Outline Planner");
    let revision_feedback_section = request
        .revision_feedback
        .as_deref()
        .map(|feedback| {
            format!(
                "[revision_feedback]\n\
                 Previous outline attempt failed; fix these issues in the regenerated outline:\n{feedback}\n\n"
            )
        })
        .unwrap_or_default();
    let prompt = format!(
        "你是 Aria 的 WorkItemPlan Outline Planner。请基于以下输入生成第一阶段 WorkItemPlan Outline。\n\n\
         {runtime_contract}\
         [issue]\n\
         title: {title}\n\
         description: {description}\n\n\
         [repository]\n\
         id: {repo_id}\n\
         path: {repo_path}\n\n\
         [confirmed_story_specs]\n{story_context}\n\n\
         [confirmed_design_specs]\n{design_context}\n\n\
         [repository_structure_summary]\n{repository_structure}\n\n\
         [design_context_gaps]\n{design_context_gaps}\n\n\
         [context_blocker_resolutions]\n{context_resolutions}\n\n\
         {revision_feedback_section}\
         [user_options]\n\
         include_integration_tests: {include_integration_tests}\n\
         include_e2e_tests: {include_e2e_tests}\n\
         force_frontend_backend_split: {force_frontend_backend_split}\n\
         require_execution_plan_confirm: {require_execution_plan_confirm}\n\n\
         {outline_write_scope_rules}\
         [strict_output_contract]\n\
         只能输出 WorkItemPlan Outline，不得输出完整 Work Item。\n\
         不得输出 VerificationPlan、verification_plan、verification_plans、work_item_id、work_item_ids。\n\
         不得输出 repository_profile，不得输出 parallel_groups。\n\
         不要输出 implementation plan 或旧版 Work Item 拆分计划字段：work_item_outlines[] 中不要使用 id、layer、summary、key_paths、reuse_modules、test_strategy、acceptance_refs。\n\
         work_item_outlines[] 每项必须同时提供稳定且唯一的 outline_id 与 logical_work_item_id；依赖只能写在各 item 的 depends_on 数组中。\n\
         不要输出 dependency_graph；后端会从 work_item_outlines[].depends_on 自动派生内部 dependency_graph。\n\
         work_item_outlines[] 每项必须包含 estimated_context_tokens(1..=50000) 与 session_fit=\"fits_single_agent_session\"。\n\
         work_item_outlines[] 每项必须包含 trusted_verification_commands：仅登记已确认仓库/Design/Outline 证据支持的 command、cwd、purpose、source_ref；证据不足时使用空数组，绝不根据 WorkItemKind 猜测命令。\n\
         不得修改仓库文件，不得创建计划文档。\n\
         如果无法补齐模块边界、关键路径或测试策略，请不要猜测完整拆分；请在 context_blockers 数组中写明需要用户补充的上下文。\n\
         如果能输出完整 outline，不得输出非空 context_blockers。\n\
         只有完全无法产出 outline 时才输出 context_blockers，且不要同时输出 outline。\n\
         路径不确定性写入 risks 或 handoff_notes，不要用 context_blockers 阻塞。\n\
         JSON 字符串内不得直接包含未转义英文双引号；自然语言引用请改用中文引号「」或转义为 \\\"，输出前必须确认 sentinel block 内 JSON 可被标准 JSON.parse/serde_json 解析。\n\
         可以在最终结构化 JSON 前输出简短、可读的规划过程，供 Workbench 流式展示。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> block。\n\
         标签内部必须是一个完整 JSON object，不要输出 Markdown code fence。\n\
         最小正确示例：{{\"outline\":{{\"id\":\"outline_artifact_1\",\"project_id\":\"{project_id}\",\"issue_id\":\"{issue_id}\",\"source_story_spec_ids\":[],\"source_design_spec_ids\":[],\"strategy_summary\":\"...\",\"work_item_outlines\":[{{\"outline_id\":\"outline_backend\",\"logical_work_item_id\":\"wi_backend\",\"title\":\"...\",\"kind\":\"backend\",\"goal\":\"...\",\"scope\":[],\"non_goals\":[],\"estimated_context_tokens\":12000,\"session_fit\":\"fits_single_agent_session\",\"source_story_spec_ids\":[],\"source_design_spec_ids\":[],\"exclusive_write_scopes\":[],\"forbidden_write_scopes\":[],\"depends_on\":[],\"verification_intent\":[],\"trusted_verification_commands\":[],\"handoff_notes\":\"...\"}},{{\"outline_id\":\"outline_frontend\",\"logical_work_item_id\":\"wi_frontend\",\"title\":\"...\",\"kind\":\"frontend\",\"goal\":\"...\",\"scope\":[],\"non_goals\":[],\"estimated_context_tokens\":10000,\"session_fit\":\"fits_single_agent_session\",\"source_story_spec_ids\":[],\"source_design_spec_ids\":[],\"exclusive_write_scopes\":[],\"forbidden_write_scopes\":[],\"depends_on\":[\"outline_backend\"],\"verification_intent\":[],\"trusted_verification_commands\":[],\"handoff_notes\":\"...\"}}],\"risks\":[],\"handoff_strategy\":\"...\",\"status\":\"draft\"}},\"context_blockers\":[]}}\n\
         严格按以下 JSON schema 输出。\n\n\
         {schema}",
        title = issue.title,
        runtime_contract = runtime_contract,
        description = issue.description.as_deref().unwrap_or("无"),
        repo_id = repository.id,
        project_id = issue.project_id,
        issue_id = issue.id,
        repo_path = repository.path.display(),
        story_context = story_context.join("\n\n"),
        design_context = design_context.join("\n\n"),
        repository_structure = repository_structure,
        design_context_gaps = format_string_list(design_context_gaps),
        context_resolutions = format_context_resolutions(context_resolutions),
        revision_feedback_section = revision_feedback_section,
        include_integration_tests = request.include_integration_tests.unwrap_or(false),
        include_e2e_tests = request.include_e2e_tests.unwrap_or(false),
        force_frontend_backend_split = request.force_frontend_backend_split.unwrap_or(false),
        require_execution_plan_confirm = request.require_execution_plan_confirm.unwrap_or(false),
        outline_write_scope_rules = OUTLINE_WRITE_SCOPE_RULES,
        nonce = nonce,
        schema = WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA,
    );
    (prompt, nonce)
}

pub(crate) fn build_outline_revision_prompt(
    _request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    feedback: &str,
) -> (String, String) {
    let nonce = structured_output_nonce();
    let runtime_contract = work_item_plan_runtime_contract("WorkItemPlan Outline Planner");
    let prompt = format!(
        "你是 Aria 的 WorkItemPlan Outline Planner。当前请求是基于同一会话中上一版 outline 进行增量返修。\n\n\
         {runtime_contract}\
         不要重新分析完整 issue、story/design 上下文或仓库结构；上一版 outline 已在同一会话上下文中。\
         请仅根据以下反馈修改 outline，输出完整更新后的 outline。\n\n\
         [issue_ref]\n\
         project_id: {project_id}\n\
         issue_id: {issue_id}\n\
         title: {title}\n\n\
         [revision_feedback]\n{feedback}\n\n\
         {outline_write_scope_rules}\
         [strict_output_contract]\n\
         只能输出 WorkItemPlan Outline，不得输出完整 Work Item。\n\
         不得输出 VerificationPlan、verification_plan、verification_plans、work_item_id、work_item_ids。\n\
         不得输出 repository_profile，不得输出 parallel_groups。\n\
         不要输出 implementation plan 或旧版 Work Item 拆分计划字段：work_item_outlines[] 中不要使用 id、layer、summary、key_paths、reuse_modules、test_strategy、acceptance_refs。\n\
         work_item_outlines[] 每项必须同时保留稳定且唯一的 outline_id 与 logical_work_item_id；依赖只能写在各 item 的 depends_on 数组中。\n\
         不要输出 dependency_graph；后端会从 work_item_outlines[].depends_on 自动派生内部 dependency_graph。\n\
         work_item_outlines[] 每项必须包含 estimated_context_tokens(1..=50000) 与 session_fit=\"fits_single_agent_session\"。\n\
         work_item_outlines[] 每项必须保留 trusted_verification_commands；仅登记证据支持的 command、cwd、purpose、source_ref，证据不足时为 []，不得根据 WorkItemKind 猜测命令。\n\
         不得修改仓库文件，不得创建计划文档。\n\
         如果能输出完整 outline，不得输出非空 context_blockers。\n\
         只有完全无法产出 outline 时才输出 context_blockers，且不要同时输出 outline。\n\
         路径不确定性写入 risks 或 handoff_notes，不要用 context_blockers 阻塞。\n\
         JSON 字符串内不得直接包含未转义英文双引号；自然语言引用请改用中文引号「」或转义为 \\\"，输出前必须确认 sentinel block 内 JSON 可被标准 JSON.parse/serde_json 解析。\n\
         可以在最终结构化 JSON 前输出简短、可读的修改说明，供 Workbench 流式展示。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> block。\n\
         标签内部必须是一个完整 JSON object，不要输出 Markdown code fence。\n\
         严格按以下 JSON schema 输出。\n\n\
         {schema}",
        project_id = issue.project_id,
        runtime_contract = runtime_contract,
        issue_id = issue.id,
        title = issue.title,
        feedback = feedback,
        outline_write_scope_rules = OUTLINE_WRITE_SCOPE_RULES,
        nonce = nonce,
        schema = WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA,
    );
    (prompt, nonce)
}

pub(crate) fn build_split_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    story_context: &[String],
    design_context: &[String],
    repository_structure: &str,
) -> String {
    let nonce = structured_output_nonce();
    let runtime_contract = work_item_plan_runtime_contract("Work Item Splitter");
    let revision_feedback_section = request
        .revision_feedback
        .as_deref()
        .map(|feedback| {
            format!(
                "[revision_feedback]\n\
                 Previous validation found the following issues; please fix them in the regenerated plan:\n{feedback}\n\n"
            )
        })
        .unwrap_or_default();

    format!(
        "你是 Aria 的 Work Item Splitter。请基于以下输入生成 IssueWorkItemPlan 候选拆分。\n\n\
         {runtime_contract}\
         [issue]\n\
         title: {title}\n\
         description: {description}\n\n\
         [repository]\n\
         id: {repo_id}\n\
         path: {repo_path}\n\n\
         [confirmed_story_specs]\n{story_context}\n\n\
         [confirmed_design_specs]\n{design_context}\n\n\
         [repository_structure_summary]\n{repository_structure}\n\n\
         {revision_feedback_section}\n\
         [openspec_constraint_summary]\n\
         story_spec_ids: {story_ids}\n\
         design_spec_ids: {design_ids}\n\n\
         [user_options]\n\
         include_integration_tests: {include_integration_tests}\n\
         include_e2e_tests: {include_e2e_tests}\n\
         force_frontend_backend_split: {force_frontend_backend_split}\n\
         require_execution_plan_confirm: {require_execution_plan_confirm}\n\n\
         [output_schema]\n\
         可以在最终结构化 JSON 前输出简短、可读的拆分过程，供 Workbench 流式展示。\n\
         长时间分析、探索代码库或自动修正前，先输出一行简短可读状态，供 Workbench 流式展示；不要等待所有工具调用结束后才给第一段说明。\n\
         如果需要执行多步代码库探索，每完成一组探索后输出一句当前发现摘要。\n\
         这些可读状态必须位于最终 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> 之前；最终结构化 JSON 仍只放在最后一个 sentinel block 中。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> block。\n\
         标签内部必须是一个完整 JSON object，不要输出 Markdown code fence。\n\
         严格按以下 JSON schema 输出。\n\
         work_items 数组顺序即执行顺序；depends_on 使用同数组中的 0-based 索引。verification_plans 数组与 work_items 一一对应。\n\
         每个 work_item 必须包含 `kind` 字段（不要写成 `type`），合法取值为以下之一：backend、frontend、integration、e2e、docs、infra、other。\n\n\
         {schema}",
        title = issue.title,
        runtime_contract = runtime_contract,
        description = issue.description.as_deref().unwrap_or("无"),
        repo_id = repository.id,
        repo_path = repository.path.display(),
        story_context = story_context.join("\n\n"),
        design_context = design_context.join("\n\n"),
        repository_structure = repository_structure,
        revision_feedback_section = revision_feedback_section,
        story_ids = request.story_spec_ids.join(", "),
        design_ids = request.design_spec_ids.join(", "),
        include_integration_tests = request.include_integration_tests.unwrap_or(false),
        include_e2e_tests = request.include_e2e_tests.unwrap_or(false),
        force_frontend_backend_split = request.force_frontend_backend_split.unwrap_or(false),
        require_execution_plan_confirm = request.require_execution_plan_confirm.unwrap_or(false),
        nonce = nonce,
        schema = WORK_ITEM_SPLIT_OUTPUT_SCHEMA,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_revision_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    retained: &[LifecycleWorkItemRecord],
    redo_specs: &[RedoSpec],
    story_context: &[String],
    design_context: &[String],
    repository_structure: &str,
) -> String {
    if retained.is_empty() && redo_specs.is_empty() {
        return build_split_prompt(
            request,
            issue,
            repository,
            story_context,
            design_context,
            repository_structure,
        );
    }

    let nonce = structured_output_nonce();
    let runtime_contract = work_item_plan_runtime_contract("Work Item Splitter");
    let retained_section = if retained.is_empty() {
        "(无)".to_string()
    } else {
        retained
            .iter()
            .map(|wi| {
                format!(
                    "- {} [{}] {}",
                    wi.id,
                    work_item_kind_text(&wi.kind),
                    wi.title
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let redo_section = redo_specs
        .iter()
        .map(|r| format!("- {}: {}", r.old_id, r.feedback))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "你是 Aria 的 Work Item Splitter。当前请求是局部重做（revision）。请基于以下输入，仅输出需要重做的 work_items 与 verification_plans。\n\n\
         {runtime_contract}\
         [issue]\n\
         title: {title}\n\
         description: {description}\n\n\
         [repository]\n\
         id: {repo_id}\n\
         path: {repo_path}\n\n\
         [confirmed_story_specs]\n{story_context}\n\n\
         [confirmed_design_specs]\n{design_context}\n\n\
         [repository_structure_summary]\n{repository_structure}\n\n\
         [retained_work_items]\n\
         以下 WorkItem 必须保留，不得在输出中重写：\n{retained_section}\n\n\
         [redo_work_items]\n\
         以下 WorkItem 需要按用户反馈重做，请只输出这些项：\n{redo_section}\n\n\
         [output_schema]\n\
         可以在最终结构化 JSON 前输出简短、可读的拆分过程，供 Workbench 流式展示。\n\
         长时间分析、探索代码库或自动修正前，先输出一行简短可读状态，供 Workbench 流式展示；不要等待所有工具调用结束后才给第一段说明。\n\
         如果需要执行多步代码库探索，每完成一组探索后输出一句当前发现摘要。\n\
         这些可读状态必须位于最终 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> 之前；最终结构化 JSON 仍只放在最后一个 sentinel block 中。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> block。\n\
         标签内部必须是一个完整 JSON object，不要输出 Markdown code fence。\n\
         严格按以下 JSON schema 输出 redo-only 结果。\n\
         work_items 数组必须且仅包含重做项，顺序对应 redo_work_items 列表；verification_plans 与 work_items 一一对应；depends_on 使用 0-based 索引。\n\
         每个 work_item 必须包含 `kind` 字段（不要写成 `type`），合法取值为以下之一：backend、frontend、integration、e2e、docs、infra、other。\n\n\
         {schema}",
        title = issue.title,
        runtime_contract = runtime_contract,
        description = issue.description.as_deref().unwrap_or("无"),
        repo_id = repository.id,
        repo_path = repository.path.display(),
        story_context = story_context.join("\n\n"),
        design_context = design_context.join("\n\n"),
        repository_structure = repository_structure,
        retained_section = retained_section,
        redo_section = redo_section,
        nonce = nonce,
        schema = WORK_ITEM_SPLIT_OUTPUT_SCHEMA,
    )
}

pub(crate) fn build_work_item_draft_prompt(
    outline: &crate::product::models::WorkItemPlanOutline,
    current_outline: &crate::product::models::WorkItemOutline,
    generation_mode: WorkItemGenerationMode,
    direct_dependencies: &[&WorkItemDraftRecord],
    other_previous: &[&WorkItemDraftRecord],
    feedback: Option<&str>,
    nonce: &str,
) -> String {
    let runtime_contract = work_item_draft_runtime_contract();
    let confirmed_plan_trace = format!(
        "plan_id: {}\nsource_story_spec_ids: {}\nsource_design_spec_ids: {}\nstrategy_summary: {}",
        outline.id,
        outline.source_story_spec_ids.join(", "),
        outline.source_design_spec_ids.join(", "),
        outline.strategy_summary,
    );
    let current_outline_json = serde_json::to_value(current_outline)
        .map(|mut value| {
            value
                .as_object_mut()
                .expect("work item outline serializes as object")
                .remove("trusted_verification_commands");
            serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
        })
        .unwrap_or_else(|_| "{}".to_string());
    let trusted_command_catalog = if current_outline.trusted_verification_commands.is_empty() {
        "(empty: do not invent required commands; use an operational_gate blocker when verification cannot be grounded)"
            .to_string()
    } else {
        crate::product::models::trusted_draft_verification_command_catalog_prompt_projection(
            &current_outline.trusted_verification_commands,
        )
    };
    let direct_dependency_json = serde_json::to_string_pretty(
        &direct_dependencies
            .iter()
            .map(|draft| {
                serde_json::json!({
                    "outline_id": &draft.outline_id,
                    "draft_id": &draft.draft_id,
                    "logical_work_item_id": &draft.candidate.logical_work_item_id,
                    "output_contracts": &draft.candidate.canonical_contract_candidate.output_contracts,
                    "handoff_contract": &draft.candidate.canonical_contract_candidate.handoff_contract,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string());
    let previous_summaries = other_previous
        .iter()
        .map(|draft| {
            let promised_contracts = draft
                .candidate
                .canonical_contract_candidate
                .output_contracts
                .iter()
                .map(|contract| contract.contract_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "- {} / {} / {}: {}; promised contracts: {}",
                draft.outline_id,
                draft.draft_id,
                draft.candidate.logical_work_item_id,
                draft.candidate.canonical_contract_candidate.identity.title,
                promised_contracts
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let feedback_section = feedback
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("\n[user_or_reviewer_feedback]\n{value}\n"))
        .unwrap_or_default();
    let mode = match generation_mode {
        WorkItemGenerationMode::Serial => "serial",
        WorkItemGenerationMode::Batch => "batch",
    };

    format!(
        "你是 Aria 的 Work Item Draft author。只输出 Canonical Contract Candidate。\n\
         {runtime_contract}\
         [mode]\n{mode}\n\
         [confirmed_plan_trace]\n{confirmed_plan_trace}\n\
         [current_work_item_outline]\n{current_outline_json}\n\
         [trusted_verification_command_catalog]\n{trusted_command_catalog}\n\
         [直接依赖的可消费交接合同]\n{direct_dependency_json}\n\
         [其他已 accepted draft 摘要]\n{previous_summaries}\n\
         {feedback_section}\
         [canonical_projection]\n\
         - Draft 专有 Canonical projection 优先于 [allowed_outputs] 的通用表述；目标、范围和非目标映射到 identity、goal、write_policy、non_goals；TDD 与验证映射到 tasks、acceptance_criteria、verification_checks。\n\
         - 依赖、交接和风险映射到 input_contracts、output_contracts、handoff_contract、blocker_rules；不得输出 writing-plans 的 Markdown Plan 或新增 JSON 字段。\n\n\
         [registration]\n\
         内部登记 acceptance criterion ID、traceability requirement ID、input/output contract ID 与上列可信命令；不输出该登记表。\n\n\
         [projection]\n\
         done_when_refs 只能引用 criterion_id；requirement_refs 只能引用登记的 requirement_id；reviewer_check_refs 必须与全部且仅 acceptance criterion ID 集合完全一致；blocker target 只能引用 input/output contract_id；required=true 的 command 必须逐字来自可信目录。\n\n\
         [self_check]\n\
         输出前逐项验证上述集合关系、verification_plan 与 canonical checks 的逐字段同序相等。可信目录为空时，所有 verification_checks 必须 required=false 且 command=null，并且必须输出有说明的 operational_gate blocker；不得输出 required=true 或伪造 required command。\n\n\
         [canonical_field_contract]\n\
         封闭类型契约（非示例）：记号 str+=非空 string，[T]=T 数组，obj=object；每个 obj 必须且只能含所列字段，所列字段全部必填，数组可空但元素不得缺/加字段。\n\
         - draft: obj{{outline_id: str+, logical_work_item_id: str+, canonical_contract: obj, verification_plan: obj}}。\n\
         - canonical_contract.schema_version: integer literal 1；identity: obj{{logical_work_item_id: str+, title: string, kind: backend|frontend|integration|e2e|docs|infra|other}}；goal: obj{{summary: string}}；non_goals: [string]。\n\
         - input_contracts: [obj{{contract_id: str+, provider_logical_work_item_id: str+, required_capabilities: [string], compatibility_policy: require_all|require_any}}]；output_contracts: [obj{{contract_id: str+, capabilities: [string]}}]。\n\
         - tasks: [obj{{task_id: str+, statement: string, requirement_refs: [string], done_when_refs: [string]}}]；write_policy: obj{{exclusive_scopes: [string], forbidden_scopes: [string]}}。\n\
         - acceptance_criteria: [obj{{criterion_id: str+, statement: string, required_evidence: [source_diff|non_zero_test_execution|manual_check|handoff_field]}}]。\n\
         - acceptance criterion 的 statement 必须描述从最终代码状态、验证命令输出、人工检查结果或 handoff 字段可观测的结果状态；不得描述开发过程本身。\n\
         - verification_checks: [obj{{check_id: str+, command: string|null, manual_instruction: string|null, required: boolean, non_zero_test_execution_required: boolean}}]；verification_plan: obj{{checks: 与 verification_checks 完全相同的数组}}。\n\
         - handoff_contract: obj{{required_fields: 唯一 str+ 数组, provided_contract_refs: 唯一 str+ 数组（无下游消费者时为空数组）, reviewer_check_refs: 唯一 str+ 数组}}。\n\
         - blocker_rules: [obj{{reason_code: str+, route: coder_rework|verification_retry|plan_repair_current|plan_repair_upstream|subgraph_replan|story_amendment|design_amendment|operational_gate, target_contract_refs: [string]}}]；design_traceability: [obj{{source_type: string, source_id: string, requirement_id: string}}]。\n\n\
         [hard_rules]\n\
         - 当前仅处于 human-confirmation 之前的候选阶段：必须读取并遵守 writing-plans 的拆分、TDD、验证与交接质量纪律；只将这些纪律体现在本候选中。\n\
         - 不得创建 cadence/plans/ 或任何 workspace 文件；不得提前执行 writing-plans 的落盘步骤；canonical writeback 与正式 Plan 落盘由 human-confirmation gate 与 daemon 负责，不得声称已完成。\n\
         - 仅在最后一个 nonce sentinel block 返回唯一 Canonical Contract Candidate JSON（不用 Markdown code fence），其 outline_id/logical_work_item_id 对应当前 `{outline_id}`/`{logical_work_item_id}`；draft 只含 [canonical_field_contract] 所列字段。\n\
         - 不得修改、新增、删除或重命名 Outline；不得输出 work_item_id、draft_id、status 等后端状态字段；logical_work_item_id 必须与其 identity 一致。\n\
         - handoff_contract 是 Canonical singleton；required_fields、reviewer_check_refs 非空且不重复；provided_contract_refs 元素唯一且非空白，仅列出被下游 WorkItem input_contracts 消费的契约 ref，无下游消费者（链路末端）时必须为空数组。\n\
         - verification command 必须来自目标仓库的可信证据，不得根据 WorkItemKind 推导；证据不足进入 manual/repair/blocker，绝不使用 Aria 当前仓库命令兜底。\n\
         - 禁止把提交历史、提交顺序、开发时序、分支操作历史作为 acceptance criterion；non_zero_test_execution 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序。\n\
         - 不得输出面向 Coder 的长篇 implementation_context；不要提前生成或渲染 Coder Projection 或 Reviewer Projection。\n\n\
         [output]\n\
         使用 nonce `{nonce}` 包裹唯一 JSON：开始标签 `<ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">`，结束标签 `</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">`。\n\
         JSON 顶层必须是 `draft`；draft 只能包含 outline_id、logical_work_item_id、canonical_contract、verification_plan。",
        outline_id = current_outline.outline_id,
        logical_work_item_id = current_outline.logical_work_item_id,
        runtime_contract = runtime_contract,
        trusted_command_catalog = trusted_command_catalog,
    )
}
