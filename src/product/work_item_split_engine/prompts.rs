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
use super::schema::{
    WORK_ITEM_DRAFT_OUTPUT_SCHEMA, WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA,
    WORK_ITEM_SPLIT_OUTPUT_SCHEMA,
};
use super::types::{
    RedoSpec, WorkItemSplitInvocation, format_context_resolutions, format_string_list,
    prompt_nonce, provider_name_to_type, structured_output_nonce, work_item_kind_text,
};

const OUTLINE_WRITE_SCOPE_RULES: &str = "\
         [write_scope_partition_rules]\n\
         依赖链上的 exclusive_write_scopes 必须互斥：如果 A depends_on B，或 dependency_graph 中 B -> A，则 A 与 B 不得拥有相同路径、父子路径或可能匹配同一文件的 glob。\n\
         integration/e2e 测试 outline 只能拥有与实现目录不共享前缀的测试、fixtures、mock 或 CI 配置路径；不要把被测功能实现目录写入测试 outline 的 exclusive_write_scopes。\n\
         不要让 outline_frontend 与 outline_integration_tests 同时拥有 web/src/**；也不要把 web/src/**/*.test.tsx 交给 integration/e2e outline，因为它会与 web/src/components/**、web/src/pages/** 等 frontend 实现范围重叠。\n\
         常见做法是 frontend outline 拥有 web/src/components/**、web/src/pages/** 及其同目录单元测试；integration_tests/e2e outline 只拥有 web/e2e/**、tests/e2e/**、fixtures/**、mocks/**、playwright.config.* 或 CI 配置。\n\
         如果两个依赖 outline 都需要改同一个 shared helper、schema、fixture 或 test harness，请拆出独立前置 outline 作为唯一 owner，其他 outline 通过 depends_on 读取 handoff；若 shared 文件位于 web/src/** 下，不要再让 frontend outline 拥有覆盖它的父级 glob。\n\
         forbidden_write_scopes 应显式写出依赖方或被依赖方已拥有的实现目录，帮助后续 draft 避免越界。\n\n";

fn work_item_plan_runtime_contract(role: &str) -> String {
    let workspace_type = WorkspaceType::WorkItemPlan;
    format!(
        "[openspec_contract]\n\
         Role: {role}\n\
         - 必须基于已确认 Story Spec 与 Design Spec 的 requirement/design trace 进行拆分。\n\
         - 必须维护 Story/Design/Work Item 追踪关系，并在任务拆分中保留来源证据。\n\
         - 每个 outline/draft 必须能追溯到 source_story_spec_ids 与 source_design_spec_ids。\n\
         - 发现 Story/Design/Work Item 之间冲突、缺失验收依据或无法确定写入边界时，必须输出 blocker 或 reviewer 可处理的风险，而不是猜测。\n\
         - 不得声称已写回 OpenSpec；当前仅生成可供 daemon 后续写回 OpenSpec tasks constraints 的结构化候选。\n\n\
         [superpowers_contract]\n\
         - 必须遵守 using-superpowers 的先读规则与 writing-plans 的计划结构要求。\n\
         - 生成的是计划和任务拆分，不执行代码修改。\n\
         - 每个 draft 必须给出后续 coding agent 可执行的目标、范围、非目标、TDD 顺序、验证命令、依赖输入、交接输出和风险。\n\
         - 每个 outline 必须拆到单个 Claude Code 或 Codex coding 会话可完成，estimated_context_tokens 必须小于 20k；超出时继续拆分，不得把过大任务作为有效 outline 输出。\n\
         - 结论必须能追溯到已提供的 Story/Design/Outline/Draft 证据。\n\n\
         [allowed_outputs]\n\
         {allowed_outputs}\n\n\
         [forbidden_outputs]\n\
         {forbidden_outputs}\n\n",
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
         work_item_outlines[] 的条目标识字段必须叫 outline_id；dependency_graph[] 必须使用 from_outline_id/to_outline_id 边，不要使用 work_item_id/depends_on 形式。\n\
         work_item_outlines[] 每项必须包含 estimated_context_tokens(1..19999) 与 session_fit=\"fits_single_agent_session\"；如果预计超过 20k 或单个 Claude Code/Codex 会话无法完成，必须继续拆成更小 outline，不得输出该项。\n\
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
         最小正确示例：{{\"outline\":{{\"id\":\"outline_artifact_1\",\"project_id\":\"{project_id}\",\"issue_id\":\"{issue_id}\",\"source_story_spec_ids\":[],\"source_design_spec_ids\":[],\"strategy_summary\":\"...\",\"work_item_outlines\":[{{\"outline_id\":\"outline_backend\",\"title\":\"...\",\"kind\":\"backend\",\"goal\":\"...\",\"scope\":[],\"non_goals\":[],\"estimated_context_tokens\":12000,\"session_fit\":\"fits_single_agent_session\",\"source_story_spec_ids\":[],\"source_design_spec_ids\":[],\"exclusive_write_scopes\":[],\"forbidden_write_scopes\":[],\"depends_on\":[],\"verification_intent\":[],\"handoff_notes\":\"...\"}}],\"dependency_graph\":[{{\"from_outline_id\":\"outline_backend\",\"to_outline_id\":\"outline_frontend\"}}],\"risks\":[],\"handoff_strategy\":\"...\",\"status\":\"draft\"}},\"context_blockers\":[]}}\n\
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
         work_item_outlines[] 的条目标识字段必须叫 outline_id；dependency_graph[] 必须使用 from_outline_id/to_outline_id 边，不要使用 work_item_id/depends_on 形式。\n\
         work_item_outlines[] 每项必须包含 estimated_context_tokens(1..19999) 与 session_fit=\"fits_single_agent_session\"；如果预计超过 20k 或单个 Claude Code/Codex 会话无法完成，必须继续拆成更小 outline，不得输出该项。\n\
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
    let runtime_contract = work_item_plan_runtime_contract("Work Item Draft author");
    let outline_json = serde_json::to_string_pretty(outline).unwrap_or_else(|_| "{}".to_string());
    let current_outline_json =
        serde_json::to_string_pretty(current_outline).unwrap_or_else(|_| "{}".to_string());
    let direct_dependency_json =
        serde_json::to_string_pretty(direct_dependencies).unwrap_or_else(|_| "[]".to_string());
    let previous_summaries = other_previous
        .iter()
        .map(|draft| {
            format!(
                "- {} / {}: {}",
                draft.outline_id, draft.draft_id, draft.candidate.handoff_summary
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
        "你是 Aria 的 Work Item Draft author。请只为当前 WorkItemPlan Outline 中的一个 item 生成 WorkItemDraftCandidate。\n\n\
         {runtime_contract}\
         [generation_mode]\n{mode}\n\n\
         [confirmed_outline]\n{outline_json}\n\n\
         [current_work_item_outline]\n{current_outline_json}\n\n\
         [直接依赖 draft 完整内容]\n{direct_dependency_json}\n\n\
         [其他已 accepted draft 摘要]\n{previous_summaries}\n\
         {feedback_section}\
         [hard_rules]\n\
         - 只能输出一个 WorkItemDraftCandidate，字段必须对应当前 outline_id `{outline_id}`。\n\
         - 不得修改 Outline，不得新增、删除或重命名 outline。\n\
         - 不得输出 work_item_id、draft_id、status、generated_from_node_id、accepted_at、batch_id 等后端状态字段。\n\
         - verification_plan 必须包含 commands、manual_checks、required_gates 三个字段；没有 manual check 时输出 []。\n\
         - verification_plan.required_gates 必须是字符串数组，只能写同一 verification_plan 内 command/manual_check 的 id，例如 [\"cmd_unit\"]。\n\
         - 不要输出 required_gates gate 对象；禁止写 {{\"id\":\"gate_unit\",\"type\":\"command\",\"command_id\":\"cmd_unit\",\"expected\":\"exit 0\"}} 这类对象。\n\
         - 当前 outline 的 estimated_context_tokens 必须小于 20k 且 session_fit 必须为 fits_single_agent_session；implementation_context 不得扩展成超过单个 Claude Code/Codex 会话可完成的兄弟任务或 Issue 级计划。\n\
         - implementation_context 必须写给后续 coding agent，包含具体模块/文件边界、已有代码入口、TDD 起点、不要触碰的范围、验收命令顺序。\n\
         - handoff_summary 必须写给依赖它的后续 work item，列出本项完成后必须交付的类型、API、状态、测试 seam、错误码或 UI 契约。\n\
         - verification_plan.commands 必须优先包含定向快反馈命令，再包含必要的 fmt/clippy/check/test；Rust 命令必须遵守 cadence/project-rules/build-test-commands.md，禁止 -j 1。\n\
         - 若 Story/Design/Outline 证据不足以指导 coding agent，必须在 implementation_context 中显式写出阻塞点或待确认项，不得编造文件路径。\n\
         - 可以先输出简短可读状态；最终 JSON 必须放在最后一个 nonce sentinel block 中，不要输出 Markdown code fence。\n\n\
         [output]\n\
         <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">{{\"draft\":{{\"outline_id\":\"{outline_id}\",\"title\":\"...\",\"kind\":\"backend|frontend|integration|e2e|docs|infra|other\",\"goal\":\"...\",\"implementation_context\":\"...\",\"exclusive_write_scopes\":[],\"forbidden_write_scopes\":[],\"depends_on_outline_ids\":[],\"required_handoff_from_outline_ids\":[],\"handoff_summary\":\"...\",\"verification_plan\":{{\"commands\":[{{\"id\":\"cmd_unit\",\"label\":\"unit tests\",\"command\":\"cargo test --locked --lib <filter>\",\"cwd\":\"\",\"purpose\":\"验证当前 work item\",\"required\":true,\"timeout_seconds\":120,\"safety\":\"approved\"}}],\"manual_checks\":[],\"required_gates\":[\"cmd_unit\"]}}}}}}</ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">\n\n\
         [output_schema]\n\
         严格按以下 JSON schema 输出。\n\n\
         {schema}",
        outline_id = current_outline.outline_id,
        runtime_contract = runtime_contract,
        schema = WORK_ITEM_DRAFT_OUTPUT_SCHEMA,
    )
}
