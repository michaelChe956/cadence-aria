use crate::product::cadence_skills::routing_reference::{
    RoutingReferenceContext, generation_cadence_routing_rules_reference,
};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    IssueRecord, LifecycleWorkItemRecord, OutlineContextBlockerResolution, ProviderName,
    RepositoryRecord, WorkItemDraftRecord, WorkItemGenerationMode, WorkspaceType,
};
use crate::product::work_item_plan_compiler::grammar;
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
/// SingleCandidate markdown author 同样通过 stdin 传递，但必须为固定 grammar/few-shot
/// 预留空间；超过硬兜底时拒绝 provider 启动，而不是静默丢失规范层内容。
pub(crate) const WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES: usize = 65_536;
const WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES: usize = 32_000;

pub(crate) struct WorkItemPlanMarkdownAuthorContext<'a> {
    pub story_context: &'a str,
    pub design_context: &'a str,
    pub design_requirement_ids: &'a [String],
    pub repository_structure: &'a str,
    pub routing_context: &'a RoutingReferenceContext,
}

/// Draft prompt 质量预算：真实规模中文 fixture 的确定性预算测试阈值。
/// Task 14 起为对齐现行校验器硬规则（空可信目录必含 operational_gate blocker + plan_repair
/// 路由 target_contract_refs 必非空且逐字）；markdown 交叉引用纪律、CJK EARS 空格、Inputs 形状与跨 item 引用 few-shot、标题逐字英文、trusted command 唯一引用及空目录 blocker 教学注入后上调至 15_600。
#[cfg(test)]
pub(crate) const WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES: usize = 15_600;

/// SingleCandidate markdown author 的质量预算。契约能力覆盖教学注入后从 15_600 上调至
/// 16_200；该预算只覆盖 SC full-author，不改变 legacy draft prompt 的预算。
#[cfg(test)]
pub(crate) const WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES: usize = 16_200;

fn work_item_plan_runtime_contract(role: &str, context: &RoutingReferenceContext) -> String {
    let workspace_type = WorkspaceType::WorkItemPlan;
    format!(
        "{}\
         当前阶段：已确认 Story/Design 后的 Work Item Plan 候选规划。\n\
         前置 gate：Aria 的 human-confirmation gate 承接人工确认；Provider 只能输出候选，不能写入 canonical artifact。\n\n\
         [openspec_contract]\n\
         Role: {role}\n\
         - 必须基于已确认 Story Spec 与 Design Spec 的 requirement/design trace 进行拆分。\n\
         - 必须维护 Story/Design/Work Item 追踪关系，并在任务拆分中保留来源证据。\n\
         - 每个 outline/draft 必须能追溯到 source_story_spec_ids 与 source_design_spec_ids。\n\
         - 发现 Story/Design/Work Item 之间冲突、缺失验收依据或无法确定写入边界时，必须输出 blocker 或 reviewer 可处理的风险，而不是猜测。\n\
         - 不得声称已写回 OpenSpec；当前仅生成可供 daemon 后续写回 OpenSpec tasks constraints 的结构化候选。\n\n\
         [allowed_outputs]\n\
         {allowed_outputs}\n\n\
         [forbidden_outputs]\n\
         {forbidden_outputs}\n\n",
        generation_cadence_routing_rules_reference(context),
        allowed_outputs = allowed_outputs_for(&workspace_type),
        forbidden_outputs = forbidden_outputs_for(&workspace_type),
    )
}

const WORK_ITEM_PLAN_FEW_SHOT_FINDINGS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_policy/fixtures/golden_findings.json"
));

pub(crate) const WORK_ITEM_PLAN_FEW_SHOT_IDS: [&str; 11] = [
    "rep1-f1", "rep1-f2", "rep2-f1", "rep2-f2", "rep2-f3", "rep2-f4", "rep2-f5", "rep2-f6",
    "rep3-f1", "rep4-f1", "rep4-f2",
];

fn work_item_plan_markdown_grammar() -> String {
    format!(
        "[markdown_grammar]\n\
         输出的第一行必须精确为 `{document_heading}`；之前不得有任何前言、解释、宣布、空白行或代码围栏（```）。\n\
         标题 `{document_heading}`；item `{item_heading_prefix}{item_id_suffix}: <title>`（ID 前缀 `{item_id_prefix}`）。\n\
         所有标题必须逐字使用上列英文名（一级 `# Work Item Plan`、二级 `## Work Item WI-<三位数字>: <title>`、三级 section 名恰为上列 13 个英文名之一）；禁止翻译标题、禁止附加中文注或括号。\n\
         输出保持精炼：每个 statement 恰好一句话；同一信息不得在多个 section 重复；不写解释性散文或总结段——机械校验只消费结构化字段。\n\
         section 按序且各一次：{structured_sections}；自由文本仅 `{free_text_sections}`（`{free_text_policy}`）。\n\
         Blockers 为空时保留空 section（### Blockers 后直接下一 section），表示无 blocker；若存在 blocker 字段则仍须完整填写 reason_code、route、target_contract_refs。\n\
         Verification.command 直接声明，将按声明执行；命令证据不足时改用 manual_instruction 或 blocker，禁止臆造命令。\n\
         行 `{structured_line}`；ID 行 `{identified_line}`；statement `{ears_template}`（{ears_keywords}）。\n\
         task_id、criterion_id、check_id 在整份文档内全局唯一且全局递增——第二个 Work Item 的任务从 TASK-004、验收从 AC-004 继续（假设前一 item 用了 TASK-001~003），不得在每个 item 内重新从 001 编号；contract_id 同理在整份文档内全局唯一，不得重复。\n\
         CJK 空格规则：WHEN 与条件文本之间、条件文本与 THE SYSTEM SHALL 之间必须各有一个半角空格；条件为中文时同样必须（正例：`WHEN 服务读取静态文件 THE SYSTEM SHALL 返回五项记录`；反例：`WHEN服务读取静态文件 THE SYSTEM SHALL 返回五项记录` 非法）。
\
         Inputs 四行且 contract_id 首行：provider_logical_work_item_id、required_capabilities、compatibility_policy（require_all|require_any）各一行；四行缺一不可。\n\
         示例：若 WI-002 依赖 WI-001 的输出，则 WI-002 的 Inputs 写：\n\
         - contract_id: <逐字复制 WI-001 Outputs 的 contract_id>\n\
         - provider_logical_work_item_id: WI-001\n\
         - required_capabilities: <该契约的能力>\n\
         - compatibility_policy: require_all\n\
         无依赖时 Inputs 留空 section。\n\
         反例：provider_logical_work_item_id 的合法值只能来自本计划的 `## Work Item` 标题中的 `WI-<数字>`；story_spec_0001/design_spec_0001 等 spec id 一律非法。\n\
         同一 Work Item 内同一 command 至多声明一次；需要复合验证时合并为一条 check 或改用 manual_instruction。\n\
         key 白名单：{structured_keys}。\n\
         值域：kind={item_kinds}；compatibility_policy={compatibility_policies}；required_evidence={evidence_kinds}；route={blocker_routes}。\n\
         未知结构化 key 必须拒绝（{unknown_key_policy}）；未知 section、非法 ID、除空 Blockers 外的缺 section/field、EARS 非法均失败关闭；诊断：{diagnostic_codes}。\n\n",
        document_heading = grammar::DOCUMENT_HEADING,
        item_heading_prefix = grammar::ITEM_HEADING_PREFIX,
        item_id_suffix = grammar::ITEM_ID_SUFFIX,
        item_id_prefix = grammar::ITEM_ID_PREFIX,
        structured_sections = grammar::STRUCTURED_SECTIONS.join("、"),
        free_text_sections = grammar::FREE_TEXT_SECTIONS.join("、"),
        free_text_policy = grammar::FREE_TEXT_SECTION_POLICY,
        structured_line = grammar::STRUCTURED_LINE_PREFIX,
        identified_line = grammar::IDENTIFIED_LINE_PREFIX,
        ears_template = grammar::EARS_STATEMENT_TEMPLATE,
        ears_keywords = grammar::EARS_KEYWORDS.join("、"),
        structured_keys = grammar::STRUCTURED_KEYS.join("、"),
        item_kinds = grammar::ALLOWED_ITEM_KINDS.join("、"),
        compatibility_policies = grammar::ALLOWED_COMPATIBILITY_POLICIES.join("、"),
        evidence_kinds = grammar::ALLOWED_EVIDENCE_KINDS.join("、"),
        blocker_routes = grammar::ALLOWED_BLOCKER_ROUTES.join("、"),
        unknown_key_policy = grammar::UNKNOWN_STRUCTURED_KEY_POLICY,
        diagnostic_codes = grammar::DIAGNOSTIC_CODES.join("、"),
    )
}

fn work_item_plan_markdown_reference_discipline(requirement_ids: Option<&[String]>) -> String {
    let requirement_ids = requirement_ids
        .filter(|ids| !ids.is_empty())
        .map(|ids| ids.join("、"))
        .unwrap_or_else(|| "（无；不得编造）".to_string());
    format!(
        "[design_requirements] {requirement_ids}\n\
         [cross_reference_discipline]\n\
         done_when_refs 仅引用先定义 criterion_id。\n\
         target_contract_refs 仅逐字引用已登记 input/output contract_id。\n\
         requirement_refs 仅引用清单；清单外 REQ-* 拒绝。\n\
         handoff 的 reviewer_check_refs 必须与全部且仅本 item 的 acceptance criterion ID 集合完全一致（每条 AC 恰好被检查一次）。\n\
         同 contract_id：provider output_capabilities 覆盖 WI input_contracts required_capabilities。\n\
         require_all=全部覆盖（缺一项→required_capability_missing）；require_any=至少一项相交（交集空→required_capability_missing）。\n\
         端点/动作如 `GET /api/levels` 须显式声明；字段/记录不隐含端点。\n\
         反例：CT-001 仅「五项记录+字段名称」，WI-002 require_all「field constraints」「GET /api/levels」→ canonical required_capability_missing。\n\
         正例：CT-001 显式声明两项，或 WI-002 改引供能 contract。\n\
         canonical fail-closed：required_capability_missing 拒绝 plan。\n\
         每个被 tasks 的 requirement_refs 引用的 requirement_id，必须在本 item 的 Traceability section 有对应登记行（requirement_id 逐字相同）；登记值只能来自 [design_requirements] 清单。\n\n"
    )
}

fn work_item_plan_dependency_syntax_rules() -> String {
    let dependencies_key = grammar::DEPENDENCIES_KEY;
    let item_id_prefix = grammar::ITEM_ID_PREFIX;
    let item_id_suffix = grammar::ITEM_ID_SUFFIX;
    format!(
        "`{dependencies_key}`：`- {dependencies_key}: []`；`- {dependencies_key}: {item_id_prefix}001`（裸值）；多依赖每行：`- {dependencies_key}: {item_id_prefix}001`\n`- {dependencies_key}: {item_id_prefix}002`。禁止括号列表、空格或逗号分隔多值；值仅 `[]` 或 `{item_id_prefix}{item_id_suffix}`。"
    )
}

fn work_item_plan_minimum_legal_source() -> &'static str {
    "# Work Item Plan\n\
     ## Work Item WI-001: x\n\
     ### Identity\n\
     - schema_version: 1\n\
     - logical_work_item_id: WI-001\n\
     - title: x\n\
     - kind: backend\n\
     ### Goal\n\
     - summary: WHEN x THE SYSTEM SHALL y.\n\
     ### Non Goals\n\
     - non_goals: x\n\
     ### Dependencies\n\
     - depends_on: []\n\
     ### Inputs\n\
     ### Outputs\n\
     - contract_id: c\n\
     - capabilities: x\n\
     ### Tasks\n\
     - task_id: TASK-001\n\
     - statement: WHEN x THE SYSTEM SHALL y.\n\
     - requirement_refs: design_requirement_placeholder\n\
     - done_when_refs: AC-001\n\
     ### Write Policy\n\
     - exclusive_scopes: x\n\
     - forbidden_scopes: y\n\
     ### Acceptance Criteria\n\
     - criterion_id: AC-001\n\
     - statement: WHEN x THE SYSTEM SHALL y.\n\
     - required_evidence: source_diff\n\
     ### Verification\n\
     - check_id: CHECK-001\n\
     - command: null\n\
     - manual_instruction: x\n\
     - required: true\n\
     - non_zero_test_execution_required: false\n\
     ### Handoff Schema\n\
     - required_fields: x\n\
     - provided_contract_refs: c\n\
     - reviewer_check_refs: AC-001\n\
     ### Blockers\n\
     - reason_code: x\n\
     - route: coder_rework\n\
     - target_contract_refs: c\n\
     ### Traceability\n\
     - source_type: x\n\
     - source_id: x\n\
     - requirement_id: design_requirement_placeholder\n"
}

fn work_item_plan_real_few_shot() -> Result<String, String> {
    let findings: Vec<serde_json::Value> =
        serde_json::from_str(WORK_ITEM_PLAN_FEW_SHOT_FINDINGS)
            .map_err(|error| format!("无法读取 Work Item Plan 判例 fixture：{error}"))?;
    let mut cases = String::from(
        "[real_finding_few_shot]\n以下为真实 provider 原始 finding；rep1 是 Advisory。按错误模式→修正原则学习，勿照抄业务名、ID、路径或命令。\n",
    );

    for id in WORK_ITEM_PLAN_FEW_SHOT_IDS {
        let entry = findings
            .iter()
            .find(|entry| entry.get("id").and_then(serde_json::Value::as_str) == Some(id))
            .ok_or_else(|| format!("判例 fixture 缺少 {id}"))?;
        if entry.get("source_kind").and_then(serde_json::Value::as_str) != Some("provider_raw") {
            return Err(format!("判例 {id} 不是 provider 原始 finding"));
        }
        let finding = entry
            .get("finding")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("判例 {id} 缺少 finding"))?;
        let field = |name: &str| {
            finding
                .get(name)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("判例 {id} 缺少 {name}"))
        };
        cases.push_str(&format!(
            "\n#### {id}\n错误模式：{}\n证据：{}\n修正原则：{}\n",
            field("message")?,
            field("evidence")?,
            field("required_action")?,
        ));
    }

    Ok(cases)
}

/// 构造单候选路径专用的 markdown source author prompt。
///
/// 该 prompt 只描述待编译的 `work-item-plan.md` 产物：Provider 原始输出直接进入
/// source revision 与 compiler，不携带旧 JSON/sentinel/draft 契约，也不从上下文补齐
/// markdown 字段。legacy builder 仍由旧 flow 使用。
pub(crate) fn build_work_item_plan_markdown_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    context: WorkItemPlanMarkdownAuthorContext<'_>,
) -> Result<String, String> {
    let few_shot = work_item_plan_real_few_shot()?;
    let dependency_syntax_rules = work_item_plan_dependency_syntax_rules();
    let reference_discipline =
        work_item_plan_markdown_reference_discipline(Some(context.design_requirement_ids));
    // story/design 真实上下文可能远大于旧 JSON outline 路径。固定 grammar、最小合法
    // source 与 few-shot 永不截断；只按确定性配额压缩可再加载的上下文，并显式标记。
    let (story_context, design_context, repository_structure) = budget_markdown_context(
        context.story_context,
        context.design_context,
        context.repository_structure,
    );
    let prompt = format!(
        "只输出完整 `work-item-plan.md` source；原始输出直接成为 source revision 并交 compiler parse。\n\
         [issue] {issue_title}\n{issue_description}\nrepo={repository_id} path={repository_path}\n\
         [routing_reference]\n{routing_reference}\n\
         [confirmed_context]\nstory:{story_context}\ndesign:{design_context}\nstructure:{repository_structure}\n\
         story_spec_ids:{story_spec_ids}\ndesign_spec_ids:{design_spec_ids}\n\
         [source_boundary]\n\
         只写 markdown 字段；不得从 issue、prompt 或 runtime 补齐 markdown 缺失字段。\n\
         exclusive_scopes 仅限本项且依赖项不得重叠；non_goals 不得与 tasks、验收、write policy 矛盾；依赖、contract、验收、handoff 必须可验证。\n\
         Verification.command 直接声明，将按声明执行；命令证据不足写 manual_instruction 或 blocker，禁止臆造。不要 JSON、私有协议、私有 draft、classifier 字段、code fence 或解释。\n\
         {dependency_syntax_rules}\n\n\
         {reference_discipline}
         {grammar}\
         [minimum_legal_source] 仅示语法形状；按当前上下文替换，勿照抄。\n{minimum_source}\n\
         {few_shot}\n\
         [output] 现在仅输出完整 markdown source。",
        issue_title = issue.title,
        issue_description = issue.description.as_deref().unwrap_or("无"),
        repository_id = repository.id,
        repository_path = repository.path.display(),
        routing_reference = generation_cadence_routing_rules_reference(context.routing_context),
        story_context = story_context,
        design_context = design_context,
        repository_structure = repository_structure,
        story_spec_ids = request.story_spec_ids.join(", "),
        design_spec_ids = request.design_spec_ids.join(", "),
        reference_discipline = reference_discipline,
        grammar = work_item_plan_markdown_grammar(),
        dependency_syntax_rules = dependency_syntax_rules,
        minimum_source = work_item_plan_minimum_legal_source(),
        few_shot = few_shot,
    );
    if prompt.len() > WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES {
        return Err(format!(
            "work item plan markdown prompt exceeds hard budget: {} > {} bytes",
            prompt.len(),
            WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES
        ));
    }
    Ok(prompt)
}

fn budget_markdown_context(
    story_context: &str,
    design_context: &str,
    repository_structure: &str,
) -> (String, String, String) {
    (
        truncate_markdown_context(
            story_context,
            WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES * 9 / 20,
        ),
        truncate_markdown_context(
            design_context,
            WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES * 9 / 20,
        ),
        truncate_markdown_context(
            repository_structure,
            WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES / 10,
        ),
    )
}

fn truncate_markdown_context(value: &str, budget: usize) -> String {
    if value.len() <= budget {
        return value.to_string();
    }
    let marker =
        "\n[上下文因 prompt 预算截断；请仅依据保留内容和 source_spec_ids 输出，不得猜测。]\n";
    let keep = budget.saturating_sub(marker.len());
    let mut end = keep.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], marker)
}

fn work_item_draft_runtime_contract(context: &RoutingReferenceContext) -> String {
    let workspace_type = WorkspaceType::WorkItemPlan;
    format!(
        "{}\
         当前阶段：已确认 Story/Design 后的 Work Item Plan 候选规划。\n\
         前置 gate：Aria 的 human-confirmation gate 承接人工确认；Provider 只能输出候选，不能写入 canonical artifact。\n\
         [openspec_contract]\n\
         Role: Work Item Draft author。必须基于已确认 Story Spec、Design Spec 与 source_story_spec_ids/source_design_spec_ids 追踪关系；冲突、缺失验收依据或边界不明时输出 blocker 或 reviewer 可处理风险，不得猜测。\n\
         [allowed_outputs]\n\
         {allowed_outputs}\n\
         [forbidden_outputs]\n\
         {forbidden_outputs}\n",
        generation_cadence_routing_rules_reference(context),
        allowed_outputs = allowed_outputs_for(&workspace_type),
        forbidden_outputs = forbidden_outputs_for(&workspace_type),
    )
}

impl WorkItemSplitEngine {
    pub(crate) fn build_generate_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        context: &RoutingReferenceContext,
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
            context,
        );

        Ok(WorkItemSplitInvocation {
            sentinel_nonce: prompt_nonce(&prompt),
            prompt,
            provider_type: provider_name_to_type(&author_provider),
            worktree_path: repository.path.to_string_lossy().to_string(),
            author_provider,
        })
    }

    pub(crate) fn build_outline_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        context_resolutions: &[OutlineContextBlockerResolution],
        context: &RoutingReferenceContext,
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
            context,
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
    pub(crate) fn build_outline_revision_invocation(
        request: &GenerateWorkItemsRequest,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        feedback: &str,
        context: &RoutingReferenceContext,
    ) -> ApiResult<WorkItemSplitInvocation> {
        let (prompt, sentinel_nonce) =
            build_outline_revision_prompt(request, issue, feedback, context);

        Ok(WorkItemSplitInvocation {
            prompt,
            provider_type: provider_name_to_type(&author_provider),
            worktree_path: repository.path.to_string_lossy().to_string(),
            author_provider,
            sentinel_nonce,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_revision_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        retained: &[LifecycleWorkItemRecord],
        redo_specs: &[RedoSpec],
        context: &RoutingReferenceContext,
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
            context,
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
/// 的真实 spec ID；request 为空时退回占位 ID。
fn example_source_spec_id_array(ids: &[String], placeholder: &str) -> String {
    if ids.is_empty() {
        return format!("[\"{placeholder}\"]");
    }
    let quoted = ids.iter().map(|id| format!("\"{id}\"")).collect::<Vec<_>>();
    format!("[{}]", quoted.join(","))
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
         [strict_output_contract]\n\
         只能输出 WorkItemPlan Outline，不得输出完整 Work Item。\n\
         不得输出 VerificationPlan、verification_plan、verification_plans、work_item_id、work_item_ids。\n\
         不得输出 repository_profile，不得输出 parallel_groups。\n\
         不要输出 implementation plan 或旧版 Work Item 拆分计划字段：work_item_outlines[] 中不要使用 id、layer、summary、key_paths、reuse_modules、test_strategy、acceptance_refs。\n\
         work_item_outlines[] 每项必须同时提供稳定且唯一的 outline_id 与 logical_work_item_id；依赖只能写在各 item 的 depends_on 数组中。\n\
         不要输出 dependency_graph；后端会从 work_item_outlines[].depends_on 自动派生内部 dependency_graph。\n\
         work_item_outlines[] 每项必须包含 estimated_context_tokens(1..=50000) 与 session_fit=\"fits_single_agent_session\"。\n\
         work_item_outlines[] 每项的 source_story_spec_ids/source_design_spec_ids 必须填写 [confirmed_story_specs]/[confirmed_design_specs] 中的真实 spec ID，禁止空数组。\n\
         work_item_outlines[] 每项必须包含 trusted_verification_commands：仅登记已确认仓库/Design/Outline 证据支持的 command、cwd、purpose、source_ref；证据不足时使用空数组，绝不根据 WorkItemKind 猜测命令。\n\
         不得修改仓库文件，不得创建计划文档。\n\
         如果无法补齐模块边界、关键路径或测试策略，请不要猜测完整拆分；请在 context_blockers 数组中写明需要用户补充的上下文。\n\
         如果能输出完整 outline，不得输出非空 context_blockers。\n\
         只有完全无法产出 outline 时才输出 context_blockers，且不要同时输出 outline。\n\
         路径不确定性写入 risks 或 handoff_notes，不要用 context_blockers 阻塞。\n\
         JSON 字符串内不得直接包含未转义英文双引号；自然语言引用请改用中文引号「」或转义为 \\\"，输出前必须确认 sentinel block 内 JSON 可被标准 JSON.parse/serde_json 解析。\n\
         可以在最终结构化 JSON 前输出简短、可读的规划过程，供 Workbench 流式展示。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT> block。\n\
         标签内部必须是一个完整 JSON object，JSON 顶层必须含 `\"nonce\":\"{nonce}\"` 并与开始标签一致，不要输出 Markdown code fence。\n\
         最小正确示例：{{\"nonce\":\"{nonce}\",\"outline\":{{\"id\":\"outline_artifact_1\",\"project_id\":\"{project_id}\",\"issue_id\":\"{issue_id}\",\"source_story_spec_ids\":{example_story_spec_ids},\"source_design_spec_ids\":{example_design_spec_ids},\"strategy_summary\":\"...\",\"work_item_outlines\":[{{\"outline_id\":\"outline_backend\",\"logical_work_item_id\":\"wi_backend\",\"title\":\"...\",\"kind\":\"backend\",\"goal\":\"...\",\"scope\":[],\"non_goals\":[],\"estimated_context_tokens\":12000,\"session_fit\":\"fits_single_agent_session\",\"source_story_spec_ids\":{example_story_spec_ids},\"source_design_spec_ids\":{example_design_spec_ids},\"exclusive_write_scopes\":[],\"forbidden_write_scopes\":[],\"depends_on\":[],\"verification_intent\":[],\"trusted_verification_commands\":[],\"handoff_notes\":\"...\"}},{{\"outline_id\":\"outline_frontend\",\"logical_work_item_id\":\"wi_frontend\",\"title\":\"...\",\"kind\":\"frontend\",\"goal\":\"...\",\"scope\":[],\"non_goals\":[],\"estimated_context_tokens\":10000,\"session_fit\":\"fits_single_agent_session\",\"source_story_spec_ids\":{example_story_spec_ids},\"source_design_spec_ids\":{example_design_spec_ids},\"exclusive_write_scopes\":[],\"forbidden_write_scopes\":[],\"depends_on\":[\"outline_backend\"],\"verification_intent\":[],\"trusted_verification_commands\":[],\"handoff_notes\":\"...\"}}],\"risks\":[],\"handoff_strategy\":\"...\",\"status\":\"draft\"}},\"context_blockers\":[]}}\n\
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
        split_option_semantics = split_option_semantics(request),
        outline_write_scope_rules = OUTLINE_WRITE_SCOPE_RULES,
        nonce = nonce,
        schema = WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA,
        example_story_spec_ids =
            example_source_spec_id_array(&request.story_spec_ids, "story_spec_0001"),
        example_design_spec_ids =
            example_source_spec_id_array(&request.design_spec_ids, "design_spec_0001"),
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
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT> block。\n\
         标签内部必须是一个完整 JSON object，JSON 顶层必须含 `\"nonce\":\"{nonce}\"` 并与开始标签一致，不要输出 Markdown code fence。\n\
         严格按以下 JSON schema 输出。\n\n\
         {schema}",
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
    context: &RoutingReferenceContext,
) -> String {
    let nonce = structured_output_nonce();
    let runtime_contract = work_item_plan_runtime_contract("Work Item Splitter", context);
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
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT> block。\n\
         标签内部必须是一个完整 JSON object，JSON 顶层必须含 `\"nonce\":\"{nonce}\"` 并与开始标签一致，不要输出 Markdown code fence。\n\
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
    context: &RoutingReferenceContext,
) -> String {
    if retained.is_empty() && redo_specs.is_empty() {
        return build_split_prompt(
            request,
            issue,
            repository,
            story_context,
            design_context,
            repository_structure,
            context,
        );
    }

    let nonce = structured_output_nonce();
    let runtime_contract = work_item_plan_runtime_contract("Work Item Splitter", context);
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
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT> block。\n\
         标签内部必须是一个完整 JSON object，JSON 顶层必须含 `\"nonce\":\"{nonce}\"` 并与开始标签一致，不要输出 Markdown code fence。\n\
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

// context 参数使函数从 7 参增到 8 参；这是 T3 裁决 A 必需的签名扩展（Draft Prompt 正文与
// MAX_BYTES/runtime contract 均未改动），对 clippy 的 too_many_arguments 阈值豁免。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_work_item_draft_prompt(
    outline: &crate::product::models::WorkItemPlanOutline,
    current_outline: &crate::product::models::WorkItemOutline,
    generation_mode: WorkItemGenerationMode,
    direct_dependencies: &[&WorkItemDraftRecord],
    other_previous: &[&WorkItemDraftRecord],
    feedback: Option<&str>,
    nonce: &str,
    context: &RoutingReferenceContext,
) -> String {
    let runtime_contract = work_item_draft_runtime_contract(context);
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
        "(empty: no trusted commands available; do not invent any. Manual checks with command=null may still be required=true. When the trusted catalog is empty the draft MUST include a route=operational_gate blocker explaining that verification cannot be grounded; a manual check (command=null) cannot substitute for that blocker, otherwise the whole draft is rejected.)"
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
         done_when_refs 只能引用 criterion_id；requirement_refs 只能引用登记的 requirement_id；reviewer_check_refs 必须与全部且仅 acceptance criterion ID 集合完全一致；blocker target_contract_refs 只能引用 input/output contract_id；plan_repair_current / plan_repair_upstream / subgraph_replan 路由的 blocker，target_contract_refs 必须非空，且每个 ref 逐字等于已登记 input/output contract_id；required=true 的 command 必须逐字来自可信目录。\n\
         input_contracts 的 contract_id 与 required_capabilities 元素都是对上游的引用而非新命名：必须逐字取自 [直接依赖的可消费交接合同] 中该 provider 的 output_contracts（含标点空格），不得改写前缀（如 oc_ 换成 ic_）、意译或自行描述——两者均按字符串精确匹配，任何差异都会失配；provider_logical_work_item_id 必须是真正声明该 contract 的上游 logical_work_item_id。被消费的 output_contracts.contract_id 还须出现在其 handoff_contract.provided_contract_refs 中。\n\n\
         Inputs 条目中，provider_logical_work_item_id 必须逐字取自本计划中被依赖 item 的 logical_work_item_id（如 WI-001）；不得使用 story_spec/design_spec 或其他 id。\n\n\
         [self_check]\n\
         输出前逐项验证上述集合关系、verification_plan 与 canonical checks 的逐字段同序相等。可信目录为空时所有 check 必须 command=null。需人工操作或目视确认的 verification_intent 必须表达为 acceptance_criteria 的 required_evidence=[manual_check]；verification_checks 的 required=true 仅限 Coder 可自行执行的命令或只读检查。人工事项由末端人工确认，不构成自动阶段阻塞；不得把可由人工确认的事项升级为 operational_gate。可信目录为空时，必须输出 route=operational_gate blocker；manual check 不能替代该 blocker，否则整体被拒。\n\
         输出前把每个 input_contracts 的 contract_id 与 required_capabilities 元素在 [直接依赖的可消费交接合同] 中做字面量查找，找不到即为错误。\n\n\
         [canonical_field_contract]\n\
         封闭类型契约（非示例）：记号 str+=非空 string，[T]=T 数组，obj=object；每个 obj 必须且只能含所列字段，所列字段全部必填，数组可空但元素不得缺/加字段。\n\
         - draft: obj{{outline_id: str+, logical_work_item_id: str+, {target_schema_field}canonical_contract: obj, verification_plan: obj}}。\n\
{target_retain_instruction}\
         - canonical_contract.schema_version: integer literal 1；identity: obj{{logical_work_item_id: str+, title: string, kind: backend|frontend|integration|e2e|docs|infra|other}}；goal: obj{{summary: string}}；non_goals: [string]。\n\
         - input_contracts: [obj{{contract_id: str+, provider_logical_work_item_id: str+, required_capabilities: [string], compatibility_policy: require_all|require_any}}]；output_contracts: [obj{{contract_id: str+, capabilities: [string]}}]。\n\
         - tasks: [obj{{task_id: str+, statement: string, requirement_refs: [string], done_when_refs: [string]}}]；write_policy: obj{{exclusive_scopes: [string], forbidden_scopes: [string]}}。\n\
         - acceptance_criteria: [obj{{criterion_id: str+, statement: string, required_evidence: [source_diff|non_zero_test_execution|manual_check|handoff_field]（必为数组，单元素也需成数组）}}]。\n\
         - acceptance criterion 的 statement 必须描述从最终代码状态、验证命令输出、人工检查结果或 handoff 字段可观测的结果状态；不得描述开发过程本身。\n\
         - canonical_contract.verification_checks: [obj{{check_id: str+, command: string|null, manual_instruction: string|null, required: boolean, non_zero_test_execution_required: boolean}}]（canonical_contract 必填字段）；draft.verification_plan: obj{{checks: 与它逐字段同序相等的独立副本}}。两处都必须输出，不得只写一处。\n\
         - handoff_contract: obj{{required_fields: 唯一 str+ 数组, provided_contract_refs: 唯一 str+ 数组（无下游消费者时为空数组）, reviewer_check_refs: 唯一 str+ 数组}}。\n\
         - blocker_rules: [obj{{reason_code: str+, route: coder_rework|verification_retry|plan_repair_current|plan_repair_upstream|subgraph_replan|story_amendment|design_amendment|operational_gate, target_contract_refs: [string]}}]；design_traceability: [obj{{source_type: string, source_id: string, requirement_id: string}}]。\n\
         - plan_repair_current / plan_repair_upstream / subgraph_replan 路由的 blocker，target_contract_refs 必须非空，且每个 ref 逐字等于已登记 input/output contract_id。\n\n\
         [hard_rules]\n\
         - 不得创建 cadence/plans/ 或任何 workspace 文件；不得提前执行 writing-plans 的落盘步骤；canonical writeback 与正式 Plan 落盘由 human-confirmation gate 与 daemon 负责，不得声称已完成。\n\
         - 仅在最后一个 nonce sentinel block 返回唯一 Canonical Contract Candidate JSON（不用 Markdown code fence），其 outline_id/logical_work_item_id{target_refs} 对应当前 `{outline_id}`/`{logical_work_item_id}`{target_outline_note}；draft 只含 [canonical_field_contract] 所列字段。\n\
         - 不得修改、新增、删除或重命名 Outline；不得输出 work_item_id、draft_id、status 等后端状态字段；logical_work_item_id 必须与其 identity 一致。\n\
         - handoff_contract 是 Canonical singleton；required_fields、reviewer_check_refs 非空且不重复；provided_contract_refs 元素唯一且非空白，仅列出被下游 WorkItem input_contracts 消费的契约 ref，无下游消费者（链路末端）时必须为空数组。\n\
         - verification command 必须来自目标仓库的可信证据，不得根据 WorkItemKind 推导；证据不足进入 manual/repair/blocker，绝不使用 Aria 当前仓库命令兜底。\n\
         - 禁止把提交历史、提交顺序、开发时序、分支操作历史作为 acceptance criterion；non_zero_test_execution 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序。\n\
         - 不得输出面向 Coder 的长篇 implementation_context；不要提前生成或渲染 Coder Projection 或 Reviewer Projection。\n\n\
         [output]\n\
         使用 nonce `{nonce}` 包裹唯一 JSON：开始标签 `<ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">`，结束标签 `</ARIA_STRUCTURED_OUTPUT>`。\n\
         JSON 顶层必须先含 `\"nonce\":\"{nonce}\"`，再含 `draft`；除 nonce 外 draft 只能包含 outline_id、logical_work_item_id{target_output_fields}、canonical_contract、verification_plan。",
        outline_id = current_outline.outline_id,
        logical_work_item_id = current_outline.logical_work_item_id,
        runtime_contract = runtime_contract,
        trusted_command_catalog = trusted_command_catalog,
        target_schema_field = if current_outline.target_repository_id.is_some() {
            "target_repository_id: uuid, "
        } else {
            ""
        },
        target_retain_instruction = if current_outline.target_repository_id.is_some() {
            "        - 当前 Draft 的 target_repository_id 必须逐字保留 [current_work_item_outline] 的 target_repository_id；逻辑代码库规划中该值为必填 UUID，缺失或不确定时停止并报告 blocker，绝不猜测或回落 primary。\n"
        } else {
            ""
        },
        target_refs = if current_outline.target_repository_id.is_some() {
            "/target_repository_id"
        } else {
            ""
        },
        target_outline_note = if current_outline.target_repository_id.is_some() {
            " Outline"
        } else {
            ""
        },
        target_output_fields = if current_outline.target_repository_id.is_some() {
            "、target_repository_id"
        } else {
            ""
        },
    )
}
