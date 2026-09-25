//! Outline（JSON 子链第一阶段）prompt 合同族：初次/修订共享的 strict contract、
//! 真实来源 ID 条款、最小正确示例与 write scope 规则渲染。
//!
//! 自 prompts.rs 机械拆出以满足 large_file_guard 的 1200 行上限（逐行移动、
//! 零行为变更，先例：sc_revision/sc_author_teaching 子文件）。对外入口经
//! prompts.rs re-export，调用方路径不变；`OUTLINE_WRITE_SCOPE_RULES` 仅本族
//! 使用，随族同迁。

use super::{
    IssueRecord, OutlineContextBlockerResolution, RepositoryRecord, RoutingReferenceContext,
    WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA, format_context_resolutions, format_string_list,
    structured_output_nonce, work_item_plan_runtime_contract,
};
use crate::web::types::GenerateWorkItemsRequest;

const OUTLINE_WRITE_SCOPE_RULES: &str = "\
         [write_scope_partition_rules]\n\
         依赖链上的 exclusive_write_scopes 必须互斥：如果 A depends_on B，则 A 与 B 不得拥有相同路径、父子路径或可能匹配同一文件的 glob。\n\
         integration/e2e 测试 outline 只能拥有与实现目录不共享前缀的测试、fixtures、mock 或 CI 配置路径；不要把被测功能实现目录写入测试 outline 的 exclusive_write_scopes。\n\
         不要让 outline_frontend 与 outline_integration_tests 同时拥有 web/src/**；也不要把 web/src/**/*.test.tsx 交给 integration/e2e outline，因为它会与 web/src/components/**、web/src/pages/** 等 frontend 实现范围重叠。\n\
         常见做法是 frontend outline 拥有 web/src/components/**、web/src/pages/** 及其同目录单元测试；integration_tests/e2e outline 只拥有 web/e2e/**、tests/e2e/**、fixtures/**、mocks/**、playwright.config.* 或 CI 配置。\n\
         如果两个依赖 outline 都需要改同一个 shared helper、schema、fixture 或 test harness，请拆出独立前置 outline 作为唯一 owner，其他 outline 通过 depends_on 读取 handoff；若 shared 文件位于 web/src/** 下，不要再让 frontend outline 拥有覆盖它的父级 glob。\n\
         forbidden_write_scopes 应显式写出依赖方或被依赖方已拥有的实现目录，帮助后续 draft 避免越界。\n\n";

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
    context: &RoutingReferenceContext,
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
        context,
    )
    .0
}

/// Renders the semantics of enabled split/test option flags so outline authors
/// classify `kind` correctly before the strict Final Compile validator enforces
/// the composition. Returns an empty string when no relevant flag is enabled.
fn split_option_semantics(request: &GenerateWorkItemsRequest) -> String {
    let mut rules: Vec<&'static str> = Vec::new();
    if request.force_frontend_backend_split.unwrap_or(false) {
        rules.push(
            "- force_frontend_backend_split=true：本次拆分必须产出至少一个 kind=backend 和至少一个 kind=frontend 的 outline。被拆离页面/演示的纯库函数、共享实现、核心逻辑归 backend；页面、UI、演示内容归 frontend。为满足该前后端拆分要求而产出的 backend/frontend 两方均不得标为 other；额外独立的 docs、infra 等工作仍按实际 kind 标注。",
        );
    }
    if request.include_integration_tests.unwrap_or(false) {
        rules.push(
            "- include_integration_tests=true：必须产出至少一个 kind=integration 的 outline。",
        );
    }
    if request.include_e2e_tests.unwrap_or(false) {
        rules.push("- include_e2e_tests=true：必须产出至少一个 kind=e2e 的 outline。");
    }
    if rules.is_empty() {
        return String::new();
    }
    format!(
        "[user_option_semantics]\n\
         以下用户选项已开启，outline 的 kind 组成必须满足对应约束（Final Compile 会严格校验，不满足将整体失败）：\n\
         {}\n\n",
        rules.join("\n")
    )
}

/// Renders a non-empty JSON string array for the outline prompt 最小正确示例。
///
/// 校验器强制 work_item_outlines 每项的 source spec ID 非空；示例若用空数组，
/// 弱模型 provider 照抄示例会导致第一轮 outline 必失败。优先注入 request 中
/// 的真实 spec ID；request 为空时退回占位 ID，并通过返回的 `using_placeholder`
/// 标记让调用方显式标注占位（F-60 P1b，方案 v1.1 因素二：不得把占位当成
/// 真实来源教给 author，不通过机械伪造来源骗 gate）。
fn example_source_spec_id_array(ids: &[String], placeholder: &str) -> (String, bool) {
    if ids.is_empty() {
        return (format!("[\"{placeholder}\"]"), true);
    }
    let quoted = ids.iter().map(|id| format!("\"{id}\"")).collect::<Vec<_>>();
    (format!("[{}]", quoted.join(",")), false)
}

/// F-60 P1b：最小正确示例前缀。占位仅在无真实 ID 时使用，且必须显式标注
/// 「仅为形状占位」，防止弱模型照抄占位 ID 伪造来源骗 gate。
fn minimal_example_prefix(using_placeholder: bool) -> &'static str {
    if using_placeholder {
        "最小正确示例（示例中的 spec ID 仅为形状占位、不是真实来源，必须替换为会话中已确认 spec 的真实 ID，禁止照抄占位值）："
    } else {
        "最小正确示例："
    }
}

/// F-60 P1b：Outline 真实来源 ID 条款（初次/修订共享入口，按轮次指向不同）。
///
/// - 初次：指向 prompt 内 [confirmed_story_specs]/[confirmed_design_specs] 小节；
/// - 修订：prompt 不重复全量 spec 上下文，改为直接点名 request 已确认的真实
///   ID；对应一侧为空时指回上一版 outline 的既有来源（增量修订依赖同会话
///   历史），两侧都禁止空数组、禁止虚构或照抄占位 ID。
fn outline_source_spec_id_rule(
    story_ids: &[String],
    design_ids: &[String],
    revision: bool,
) -> String {
    if !revision {
        return "work_item_outlines[] 每项的 source_story_spec_ids/source_design_spec_ids 必须填写 [confirmed_story_specs]/[confirmed_design_specs] 中的真实 spec ID，禁止空数组。\n\
                "
            .to_string();
    }
    let story_clause = if story_ids.is_empty() {
        "source_story_spec_ids 必须沿用上一版 outline 已引用的真实 story spec ID".to_string()
    } else {
        format!(
            "source_story_spec_ids 必须从本请求已确认的真实 ID 中填写（story: {}）",
            story_ids.join(", ")
        )
    };
    let design_clause = if design_ids.is_empty() {
        "source_design_spec_ids 必须沿用上一版 outline 已引用的真实 design spec ID".to_string()
    } else {
        format!(
            "source_design_spec_ids 必须从本请求已确认的真实 ID 中填写（design: {}）",
            design_ids.join(", ")
        )
    };
    format!(
        "work_item_outlines[] 每项的 source_story_spec_ids/source_design_spec_ids：{story_clause}；{design_clause}；两数组都禁止空数组，禁止虚构或照抄占位 ID。\n"
    )
}

/// F-60 P1b：Outline 最小正确示例（初次/修订共享渲染）。
///
/// 真实来源 ID 优先取自 request；无真实 ID 时回退占位并经 prefix 显式标注。
/// 修订轮与初次使用同一份示例，保证弱模型在增量轮同样看到非空双来源的
/// 正确形状（Bug A：空数组示例会被照抄导致第一轮必失败）。
fn outline_minimal_correct_example(
    nonce: &str,
    project_id: &str,
    issue_id: &str,
    example_story_spec_ids: &str,
    example_design_spec_ids: &str,
    using_placeholder: bool,
) -> String {
    format!(
        "{prefix}{{\"nonce\":\"{nonce}\",\"outline\":{{\"id\":\"outline_artifact_1\",\"project_id\":\"{project_id}\",\"issue_id\":\"{issue_id}\",\"source_story_spec_ids\":{example_story_spec_ids},\"source_design_spec_ids\":{example_design_spec_ids},\"strategy_summary\":\"...\",\"work_item_outlines\":[{{\"outline_id\":\"outline_backend\",\"logical_work_item_id\":\"wi_backend\",\"title\":\"...\",\"kind\":\"backend\",\"goal\":\"...\",\"scope\":[],\"non_goals\":[],\"estimated_context_tokens\":12000,\"session_fit\":\"fits_single_agent_session\",\"source_story_spec_ids\":{example_story_spec_ids},\"source_design_spec_ids\":{example_design_spec_ids},\"exclusive_write_scopes\":[],\"forbidden_write_scopes\":[],\"depends_on\":[],\"verification_intent\":[],\"trusted_verification_commands\":[],\"handoff_notes\":\"...\"}},{{\"outline_id\":\"outline_frontend\",\"logical_work_item_id\":\"wi_frontend\",\"title\":\"...\",\"kind\":\"frontend\",\"goal\":\"...\",\"scope\":[],\"non_goals\":[],\"estimated_context_tokens\":10000,\"session_fit\":\"fits_single_agent_session\",\"source_story_spec_ids\":{example_story_spec_ids},\"source_design_spec_ids\":{example_design_spec_ids},\"exclusive_write_scopes\":[],\"forbidden_write_scopes\":[],\"depends_on\":[\"outline_backend\"],\"verification_intent\":[],\"trusted_verification_commands\":[],\"handoff_notes\":\"...\"}}],\"risks\":[],\"handoff_strategy\":\"...\",\"status\":\"draft\"}},\"context_blockers\":[]}}\n",
        prefix = minimal_example_prefix(using_placeholder),
    )
}

/// F-60 P1b：Outline 初次/修订共享的 [strict_output_contract] 渲染（增量契约等价）。
///
/// 修订轮 prompt 不重复全量 story/design/repository 上下文，但 gate 必需条款
/// （nonce sentinel 规则、JSON 转义、context_blockers 纪律、真实来源 ID 要求、
/// 最小正确示例、完整 JSON schema）与初次同源自同一渲染，不依赖会话历史补
/// 合同。轮次差异仅限：措辞「提供/保留」、context_blockers 首猜指引（仅初次
/// ——修订轮已有上一版 outline 可依）与过程说明用词。
///
/// P0 分层对齐（方案 v1.1 因素三）：Markdown author 族的「后续轮一行短引用」
/// 形态不适用于 JSON 子链——nonce 每轮新生成，短引用会指向旧轮 sentinel 规则；
/// JSON schema 无法一行穷举。JSON 子链的分层体现为「上下文两档」（初次全量
/// 上下文/修订短上下文），合同本体两轮完整同源。
fn outline_strict_output_contract(
    nonce: &str,
    source_spec_id_rule: &str,
    minimal_example: &str,
    revision: bool,
) -> String {
    let keep = if revision { "保留" } else { "提供" };
    let first_guess_blockers = if revision {
        ""
    } else {
        "如果无法补齐模块边界、关键路径或测试策略，请不要猜测完整拆分；请在 context_blockers 数组中写明需要用户补充的上下文。\n"
    };
    let process_note = if revision {
        "修改说明"
    } else {
        "规划过程"
    };
    format!(
        "[strict_output_contract]\n\
         只能输出 WorkItemPlan Outline，不得输出完整 Work Item。\n\
         不得输出 VerificationPlan、verification_plan、verification_plans、work_item_id、work_item_ids。\n\
         不得输出 repository_profile，不得输出 parallel_groups。\n\
         不要输出 implementation plan 或旧版 Work Item 拆分计划字段：work_item_outlines[] 中不要使用 id、layer、summary、key_paths、reuse_modules、test_strategy、acceptance_refs。\n\
         work_item_outlines[] 每项必须同时{keep}稳定且唯一的 outline_id 与 logical_work_item_id；依赖只能写在各 item 的 depends_on 数组中。\n\
         不要输出 dependency_graph；后端会从 work_item_outlines[].depends_on 自动派生内部 dependency_graph。\n\
         work_item_outlines[] 每项必须包含 estimated_context_tokens(1..=50000) 与 session_fit=\"fits_single_agent_session\"。\n\
         {source_spec_id_rule}\
         work_item_outlines[] 每项必须{keep} trusted_verification_commands：仅登记已确认仓库/Design/Outline 证据支持的 command、cwd、purpose、source_ref；证据不足时使用空数组，绝不根据 WorkItemKind 猜测命令。\n\
         不得修改仓库文件，不得创建计划文档。\n\
         {first_guess_blockers}\
         如果能输出完整 outline，不得输出非空 context_blockers。\n\
         只有完全无法产出 outline 时才输出 context_blockers，且不要同时输出 outline。\n\
         路径不确定性写入 risks 或 handoff_notes，不要用 context_blockers 阻塞。\n\
         JSON 字符串内不得直接包含未转义英文双引号；自然语言引用请改用中文引号「」或转义为 \\\"，输出前必须确认 sentinel block 内 JSON 可被标准 JSON.parse/serde_json 解析。\n\
         可以在最终结构化 JSON 前输出简短、可读的{process_note}，供 Workbench 流式展示。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT> block。\n\
         标签内部必须是一个完整 JSON object，JSON 顶层必须含 `\"nonce\":\"{nonce}\"` 并与开始标签一致，不要输出 Markdown code fence。\n\
         {minimal_example}\
         严格按以下 JSON schema 输出。\n\n\
         {schema}",
        keep = keep,
        first_guess_blockers = first_guess_blockers,
        process_note = process_note,
        source_spec_id_rule = source_spec_id_rule,
        minimal_example = minimal_example,
        nonce = nonce,
        schema = WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA,
    )
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
    context: &RoutingReferenceContext,
) -> (String, String) {
    let nonce = structured_output_nonce();
    let runtime_contract = work_item_plan_runtime_contract("WorkItemPlan Outline Planner", context);
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
    // F-60 P1b：初次/修订共享合同渲染（增量契约等价），真实来源 ID 优先。
    let (example_story_spec_ids, story_placeholder) =
        example_source_spec_id_array(&request.story_spec_ids, "story_spec_0001");
    let (example_design_spec_ids, design_placeholder) =
        example_source_spec_id_array(&request.design_spec_ids, "design_spec_0001");
    let strict_output_contract = outline_strict_output_contract(
        &nonce,
        &outline_source_spec_id_rule(&request.story_spec_ids, &request.design_spec_ids, false),
        &outline_minimal_correct_example(
            &nonce,
            &issue.project_id,
            &issue.id,
            &example_story_spec_ids,
            &example_design_spec_ids,
            story_placeholder || design_placeholder,
        ),
        false,
    );
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
         {split_option_semantics}\
         {outline_write_scope_rules}\
         {strict_output_contract}",
        title = issue.title,
        runtime_contract = runtime_contract,
        description = issue.description.as_deref().unwrap_or("无"),
        repo_id = repository.id,
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
        split_option_semantics = split_option_semantics(request),
        outline_write_scope_rules = OUTLINE_WRITE_SCOPE_RULES,
        strict_output_contract = strict_output_contract,
    );
    (prompt, nonce)
}

pub(crate) fn build_outline_revision_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    feedback: &str,
    context: &RoutingReferenceContext,
) -> (String, String) {
    let nonce = structured_output_nonce();
    let runtime_contract = work_item_plan_runtime_contract("WorkItemPlan Outline Planner", context);
    // F-60 P1b（增量契约等价）：修订轮与初次共用同一 strict contract 渲染与
    // 最小正确示例；真实来源 ID 直接取自 request，无真实 ID 时占位显式标注、
    // 条款指回上一版 outline 的既有来源——修订不依赖会话历史补 gate 必需条款。
    let (example_story_spec_ids, story_placeholder) =
        example_source_spec_id_array(&request.story_spec_ids, "story_spec_0001");
    let (example_design_spec_ids, design_placeholder) =
        example_source_spec_id_array(&request.design_spec_ids, "design_spec_0001");
    let strict_output_contract = outline_strict_output_contract(
        &nonce,
        &outline_source_spec_id_rule(&request.story_spec_ids, &request.design_spec_ids, true),
        &outline_minimal_correct_example(
            &nonce,
            &issue.project_id,
            &issue.id,
            &example_story_spec_ids,
            &example_design_spec_ids,
            story_placeholder || design_placeholder,
        ),
        true,
    );
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
         [user_options]\n\
         include_integration_tests: {include_integration_tests}\n\
         include_e2e_tests: {include_e2e_tests}\n\
         force_frontend_backend_split: {force_frontend_backend_split}\n\
         require_execution_plan_confirm: {require_execution_plan_confirm}\n\n\
         {split_option_semantics}\
         {outline_write_scope_rules}\
         {strict_output_contract}",
        project_id = issue.project_id,
        runtime_contract = runtime_contract,
        issue_id = issue.id,
        title = issue.title,
        feedback = feedback,
        include_integration_tests = request.include_integration_tests.unwrap_or(false),
        include_e2e_tests = request.include_e2e_tests.unwrap_or(false),
        force_frontend_backend_split = request.force_frontend_backend_split.unwrap_or(false),
        require_execution_plan_confirm = request.require_execution_plan_confirm.unwrap_or(false),
        split_option_semantics = split_option_semantics(request),
        outline_write_scope_rules = OUTLINE_WRITE_SCOPE_RULES,
        strict_output_contract = strict_output_contract,
    );
    (prompt, nonce)
}
