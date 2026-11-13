use crate::product::cadence_skills::routing_reference::{
    RoutingReferenceContext, generation_cadence_routing_rules_reference,
};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{
    IssueRecord, IssueWorkItemPlanOptions, LifecycleWorkItemRecord,
    OutlineContextBlockerResolution, ProviderName, RepositoryRecord, WorkItemDraftRecord,
    WorkItemGenerationMode, WorkspaceType,
};
use crate::product::work_item_plan_compiler::ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION;
use crate::product::work_item_plan_compiler::PlanMemberTargets;
use crate::product::workspace_engine::{allowed_outputs_for, forbidden_outputs_for};
use crate::web::error::{ApiError, ApiResult};
use crate::web::types::GenerateWorkItemsRequest;

use super::WorkItemSplitEngine;
use super::context::{
    collect_design_context, collect_story_context, design_context_gaps, is_lc_issue,
    merge_design_context_capabilities, summarize_repository_structure,
};
use super::schema::{WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA, WORK_ITEM_SPLIT_OUTPUT_SCHEMA};
use super::types::{
    RedoSpec, WorkItemSplitInvocation, format_context_resolutions, format_string_list,
    prompt_nonce, provider_name_to_type, structured_output_nonce, work_item_kind_text,
};

pub const WORK_ITEM_DRAFT_PROMPT_VERSION: &str = "work_item_draft_v2";
/// Fail-closed 硬兜底：只拦截病态序列化回归（如整条持久化记录被注入 prompt）。
/// 质量预算不由本常量承担，见 WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES 的预算测试。
/// prompt 经 stdin JSON 发送给 Provider，无 OS ARG_MAX 约束；真实物理边界是模型上下文窗口
/// （owner 环境 1M）。本批 work item plan 的评审更充分后修订块变大，曾把修订轮 prompt 顶到
/// 70 KB 并被旧的 64 KiB 兜底误拒，故与 markdown author 兜底同步抬到 256 KiB。
pub(crate) const WORK_ITEM_DRAFT_PROMPT_MAX_BYTES: usize = 262_144;
/// SingleCandidate markdown author 同样通过 stdin 传递，但必须为固定 grammar/few-shot
/// 预留空间；超过硬兜底时拒绝 provider 启动，而不是静默丢失规范层内容。
/// 与 draft 侧同源：硬兜底只拦病态序列化回归，真实边界是模型上下文窗口（owner 环境 1M）；
/// 本批评审更充分后修订块变大（实测修订轮 prompt 70,326 字节）曾顶破旧 64 KiB，故抬到 256 KiB。
pub(crate) const WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES: usize = 262_144;

pub(crate) struct WorkItemPlanMarkdownAuthorContext<'a> {
    pub story_context: &'a str,
    pub design_context: &'a str,
    pub design_requirement_ids: &'a [String],
    pub repository_structure: &'a str,
    /// SingleCandidate 启动前从目标仓库读取的 `.claude/rules/language.md` 全文。
    /// 缺失或不可读必须由调用方拒绝启动，不能回退为指针式提示。
    pub language_rules: &'a str,
    /// 已确认的创建计划选项（F-51/F-56，REQ-WSC-06）：启用 flag 的镜像教学与
    /// AC 基线纪律逐字引用校验器共享常量；由生产调用方从 plan record 提供，
    /// 测试夹具必须显式提供（不得依赖默认值丢意图）。
    pub plan_options: &'a IssueWorkItemPlanOptions,
    /// 仅在逻辑代码库中保留 gateway 已验证的政策权威信息；传统单仓不注入旧指针。
    pub routing_context: &'a RoutingReferenceContext,
    /// SG5.1（REQ-CG-03、REQ-SG-02）：已确认成员归属事实——与 Final Compile
    /// 的 per-WI 成员目标表同源（逻辑成员 UUID → 物理成员仓 ID），由生产
    /// 调用方一次装载、prompt 只读消费；`None`（legacy 单仓）时成员/仓序
    /// 教学整段缺席，首轮 prompt 逐字节不变、author 质量预算不扩大。
    pub member_targets: Option<&'a PlanMemberTargets>,
    /// SG5.1：confirmed design 的仓级执行顺序（change_order 单源装载）；
    /// 仓序自动边规则与编译器同源消费，空序如实教「无已确认仓序」，
    /// 不虚构先后顺序。
    pub change_order: &'a [LogicalRepositoryId],
    /// D5 事前（REQ-WSC-06「基线树事实直供」）：plan 基线树可用路径清单
    /// （`render_baseline_tree_section` 按成员仓分组渲染的文本），由生产
    /// 调用方在作者轮启动前从 `plan_preflight::plan_baseline_tree` 装载
    /// （与 Evaluate 期校验同函数同源）。`None` = 基线树不可得（无仓
    /// issue / 装载失败 / 空树）⇒ `baseline:` 段整段缺席，既有 prompt
    /// 逐字节不变。清单走上下文面（`[confirmed_context]` 段度量，
    /// `WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES`），不占用固定
    /// 规范面质量预算。
    pub baseline_tree: Option<&'a str>,
}

/// Draft prompt 质量预算：真实规模中文 fixture 的确定性预算测试阈值。
/// Task 14 起为对齐现行校验器硬规则（空可信目录必含 operational_gate blocker + plan_repair
/// 路由 target_contract_refs 必非空且逐字）；markdown 交叉引用纪律、CJK EARS 空格、Inputs 形状与跨 item 引用 few-shot、标题逐字英文、trusted command 唯一引用及空目录 blocker 教学注入后上调至 15_600。
#[cfg(test)]
pub(crate) const WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES: usize = 15_600;

/// SingleCandidate markdown author 的质量预算。项目规则内容式注入后上调；
/// 该预算只覆盖 SC full-author，不改变 legacy draft prompt 的预算。
/// 🔴 红线（2026-08-31 终审裁决 2b）：任何后续 SC author 教学变更必须先将本常量
/// 上调至整百级并注明 margin 惯例，再添加教学内容；预算调整必须配套更新
/// prompt bytes 测试与 margin 说明。历史上限 19,000（实测 18,934/余 66B）。
/// 2026-09-03 P1-A 按红线先上调至 20,000（第 7 次提额，先例 14,400→19,000 共 6 次），
/// 同批追加尾部 [format_clamp] 钳制块与标题正反例 few-shot；实测 19,324/余 676B，
/// 断言见 prompt_contract::sc_author_prompt_tail_clamps_heading_language_and_raises_budget。
/// 2026-09-06 F2-C 在 [format_clamp] 尾、[output] 前增反前导语教学行（~109B）；预算
/// 维持 20,000（已达整百级，余量足够），实测见 prompt_contract_sc_anti_preamble。
/// 2026-09-07 3.6 弱模型基线加固：[weak_model_precision] 精度教学两段（净增 547B，紧随
/// [cross_reference_discipline] 之后）；预算维持 20,000（整百级、余量足够），实测
/// 19,976/余 24B 与 19,984/余 16B（两 fixture），见
/// prompt_contract::work_item_plan_markdown_prompt_teaches_weak_model_precision_discipline。
/// 2026-09-08 3.6 矩阵 codex×重 三连败根因（issue_0166/0167/0168：上游 WI 输出契约
/// capabilities 未逐字覆盖下游 Inputs require_all 消费 → required_capability_missing
/// approval 编译拒）：按红线先上调至 20,500（第 8 次提额），同批追加
/// [weak_model_precision] 第三条「输出契约纪律」（净增 498B，含分隔换行）；实测
/// 20,474/余 26B 与 20,482/余 18B（两 fixture），见
/// prompt_contract_weak_model::work_item_plan_markdown_prompt_teaches_output_contract_capability_verbatim_coverage。
/// 依据：openspec/changes/archive/2026-08-31-rearch-workitem-plan-pipeline/design.md「SC author 预算余量红线」节。
#[cfg(test)]
// 第 10 次提额（2026-09-19，DEF-PVR-ALL 方案 a）：输出契约纪律补跨 WI 依赖
// 成对书写正例（门重测 codex rep1/rep2 只写消费侧忘写提供侧根因），净增约 260B。
// 第 11 次提额（2026-09-22，Final Compile 重复 trusted command 收敛方案 b）：grammar
// 「同一 command 至多声明一次」条款补编译器拒绝后果（lowering_error/trusted_commands
// 逐字消息）与正反例 few-shot，净增 448B；实测 21,838/余 162B（最大 fixture，见
// prompt_contract::work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_
// real_findings 的 eprintln）。
// 第 12 次评估（2026-09-23，复验 F-41）：Handoff Schema 条款补键白名单教学
// （只允许 required_fields/provided_contract_refs/reviewer_check_refs 三个 key，自创
// key 如 requested_fields 会被编译器拒），净增 132B。预算维持 22,000（已是整百级，
// 余量仍为正，先例 2026-09-06/09-07 同口径）；实测 21,978/余 22B（最大 fixture
// prompt_contract_weak_model，其余 fixture 余 30~40B）。
// 第 13 次评估（2026-09-24，F-48 ears-delivery-normalization）：CJK 空格规则补
// `」` 收尾判例（合法 `WHEN 点击「重开」 THE SYSTEM SHALL 清空` / 非法
// `WHEN 点击「重开」THE SYSTEM SHALL 清空`），净增 150B；因红线余量仅 22B，同批在
// 同一 grammar 块内删除/压缩语义重复表述（删「标题 `# Work Item Plan`；item …」
// 冗余行——document_heading/item_heading_prefix/item_id_suffix/item_id_prefix 四个
// 语法常量在「所有标题必须逐字使用上列英文名…」行与 dependency_syntax_rules 行另有
// 注入锚点；删紧随 EARS 模板的重复注解「（WHEN、THE SYSTEM SHALL）」；压缩
// Blockers 尾句与 section 序号措辞），共 151B。净 −1B，红线保持 22,000 不变。
// 第 14 次评估（2026-09-25，C1-T5 options 镜像+AC 基线纪律，REQ-WSC-06/F-51/F-56）：
// 固定面净增 AC 基线纪律行 394B（含 ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION
// 常量 189B 逐字同源）；控制器裁决不提额，同批按 EarsImpl 方案 A 先例删/压语义
// 重复表述共约 481B（头部删「；原始输出直接成为 source revision 并交 compiler
// parse」——[output] 指令三重重复；source_boundary 删「只写 markdown 字段；」
// 「；依赖、contract、验收、handoff 必须可验证」（cross_reference_discipline 另有
// 注入锚点）、「Verification.command 直接声明…禁止臆造。」（grammar 块逐字重复）、
// 「、code fence 或解释」（grammar/反前导语行另有注入）；format_clamp 删「重申：」
// 前缀；PROJECT_RULE_PRIORITY 删「,不做翻译」「;代码、路径、命令、契约 ID 保持
// 原样」（CODE_READING_SUMMARY 另有注入锚点）；CODE_USAGE_SUMMARY 删「保持职责与
// 范围最小化…」尾句（grammar「输出保持精炼」另有注入锚点））——未触碰任何 pinned
// 断言。全 false fixture 实测净 −87B（最大 F2-C 21,977→21,890/余 110）；三 flag
// 镜像行 717B 为条件渲染（outline split_option_semantics 先例，仅启用时出现），
// 全 true fixture 实测 22,607（未纳入红线口径，硬兜底 65,536 远未触及）。红线
// 保持 22,000 不变。
// 第 15 次评估（2026-09-28，C1-T5 existing/create 意图合同，REQ-C1-PLAN-01）：
// STRUCTURED_SECTIONS 增可选 `Plan Intent`、STRUCTURED_KEYS 增 6 个 intent*
// key、DIAGNOSTIC_CODES 增 intent_undeclared/intent_unexecutable、最小合法源
// 增 Plan Intent 语法形状示例（grammar 注入锚点自动展开），全 false fixture
// 实测 22,162；第 12 次提额至整百级 22,200（余 38B）。
pub(crate) const WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES: usize = 23_500;
// r46 第 13 次提额(非静默,批注):多 source Traceability 正例样例(约
// 400B,AT 方案:结构歧义用样例封——r46 首轮漏填 requirement_id 的震荡
// 根治)注入后最大 fixture 实测 22,603B,上调至整百级 23,000。
// kimi-10 第 14 次提额(非静默,批注):Plan Intent 分支必填教学(473B,
// kimi-10 plan fresh 现场:三个 WI 全写 kind=single_repository 却只给
// intent_target_logical_repo,缺 intent_target_repo → lowering_error
// 「Plan Intent 声明在场时该字段必填。」×3 零重驱终态——值域行 6a2e1207
// 只教 intent 枚举,分支字段语义与值锚定从未教过)注入后实测 23,309B,
// 上调至整百级 23,500。系统性「全段逐字段结构模板」评估被否:估增
// 1.2-2KB 远超红线,且 grammar 面其余条件字段(Verification command/
// manual_instruction、Inputs 条件、Blockers 空段)均已有教学,Plan Intent
// 是唯一缺席的条件面——段级补齐即条件面教学的全集。
// A+ 第 1 次(非静默,批注,2026-10-31;REQ-WG-07/tasks.md 5.0/5.4):Plan Intent
// 归属取值来源改为**按会话类型唯一**——旧形态 grammar 段无条件要求「逐字取
// [issue] 头 repo=」,同文件 LC 成员段又改口「不取 [issue] 头 repo=、改为取
// 成员表物理成员仓 ID」,同一字段两个来源并存(形态一在 prompt 侧的唯一成因)。
// 现形态:grammar 段一条权威句按会话类型分支(单仓=本 prompt 无成员仓清单→
// [issue] 头 repo=;LC 多仓=有成员仓清单→该清单的物理成员仓 ID),LC 成员段
// 只复述「取上列物理成员仓 ID」+否定式提醒,不再改口;同批删除 grammar 段
// 逐字印着 lowering_error 原文的噪声句。「模型自己填归属」的分支必填规则
// 保留(A1 平台接管未落地前不得删)。体量:grammar 段 472→512B(+40),
// LC 成员段 679→523B(-156,仅 LC 会话在场);红线维持 23,500(不提额),
// 最大 fixture 实测 23,456B/余 44B;计量锁见
// tests/prompt_contract_plan_intent_target.rs(规则句字节+硬性措辞数上限)。

/// SC markdown author prompt 的尾部输出指令。首轮与修订轮共享同一段字节；
/// 修订轮（F5-A findings 回灌）仅在其之前插入 [review_revision] 返修段，
/// 首轮 prompt 逐字节不变。
pub(crate) const WORK_ITEM_PLAN_MARKDOWN_OUTPUT_DIRECTIVE: &str =
    "[output] 现在仅输出完整 markdown source。";

mod sc_few_shot;
#[cfg(test)]
pub(crate) use sc_few_shot::WORK_ITEM_PLAN_FEW_SHOT_IDS;
mod sc_baseline_tree;
pub(crate) use sc_baseline_tree::WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES;
#[cfg(test)]
pub(crate) use sc_baseline_tree::render_baseline_tree_section;
pub(crate) use sc_baseline_tree::{author_baseline_tree_section, budget_markdown_context};
mod sc_revision;
pub(crate) use sc_revision::build_work_item_plan_markdown_fresh_revision_prompt;
pub(crate) use sc_revision::build_work_item_plan_markdown_revision_prompt;
mod sc_author_teaching;
pub(crate) use sc_author_teaching::{
    WORK_ITEM_PLAN_WEAK_MODEL_PRECISION_DISCIPLINE, work_item_plan_confirmed_member_order_teaching,
    work_item_plan_markdown_grammar, work_item_plan_markdown_reference_discipline,
    work_item_plan_minimum_legal_source, work_item_plan_option_mirror_teaching,
};
mod outline_contract;
#[cfg(test)]
pub(crate) use outline_contract::build_outline_prompt;
pub(crate) use outline_contract::{
    build_outline_fresh_revision_prompt_with_nonce, build_outline_prompt_with_nonce,
    build_outline_revision_prompt,
};

pub(crate) const SINGLE_CANDIDATE_PROJECT_RULE_PRIORITY: &str = "结构标题(##/### section 名)、字段 key、ID(WI-*/CT-*/TASK-*/AC-*/REQ-*/CHECK-* 等)、枚举值(require_all/require_any/backend/frontend/integration 等)永远保持 grammar 指定的英文原样;上述语言规则仅约束自由文本值(各 ## 标题的 <title> 部分、statement/description/capabilities 等字段的值)与说明性文字。字段行分隔符必须是半角 ASCII:每行写作 `- key: value`(冒号+一个空格均为半角),禁止全角冒号`：`或全角空格;EARS 关键词(WHEN/THE SYSTEM SHALL)、ID 前缀(WI-/CT-/TASK-/AC-/REQ-/CHECK-)与枚举值内的分隔亦为半角;中文仅出现在值的自由文本中。";

const SINGLE_CANDIDATE_CODE_USAGE_SUMMARY: &str = "任务拆分与验证设计遵循测试先行纪律：先明确可验证验收，再安排实现步骤；验证命令必须真实可执行、可复现，并与仓库现有工具链相符；遵守安全边界，不引入未授权依赖、凭据、网络访问或外部服务。";

const SINGLE_CANDIDATE_CODE_READING_SUMMARY: &str = "已给事实不重复全仓探测；缺事实才针对性查证，大范围定位优先检索工具，先依据真实文件确认事实再写计划。引用的代码、路径、命令与契约 ID 必须来自真实仓库内容，保留原样且可追溯；不得臆造、猜测不存在的接口、目录、测试或验证命令。";

fn single_candidate_project_rules_reference(
    language_rules: &str,
    routing_context: &RoutingReferenceContext,
) -> String {
    let logical_policy_authority = match routing_context {
        RoutingReferenceContext::Legacy => String::new(),
        RoutingReferenceContext::Logical(_) => {
            generation_cadence_routing_rules_reference(routing_context)
                .strip_prefix("[cadence_project_rules]\n")
                .unwrap_or_default()
                .to_string()
        }
    };
    format!(
        "[cadence_project_rules]\n{language_rules}\n\
         [project_rule_priority]\n{SINGLE_CANDIDATE_PROJECT_RULE_PRIORITY}\n\
         [code_usage_summary]\n{SINGLE_CANDIDATE_CODE_USAGE_SUMMARY}\n\
         [code_reading_summary]\n{SINGLE_CANDIDATE_CODE_READING_SUMMARY}\n\
         {logical_policy_authority}"
    )
}

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

/// r46 配套 2(AT):多 source traceability 正例样例——结构歧义用样例封。
/// ID 用真实登记形态(REQ-/AC-)但 000 后缀表样例占位(勿照抄指令在段头);
/// 每组三元组(source_type/source_id/requirement_id)各自成组,缺
/// requirement_id 的组即 r46 首轮漏填形态。
const WORK_ITEM_PLAN_MULTI_SOURCE_TRACEABILITY_EXAMPLE: &str = "### Traceability\n\
     - source_type: story\n\
     - source_id: story_spec_0001\n\
     - requirement_id: REQ-000\n\
     - source_type: acceptance\n\
     - source_id: story_spec_0001\n\
     - requirement_id: AC-000\n";
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
    let few_shot = sc_few_shot::work_item_plan_real_few_shot()?;
    let plan_options_mirror = work_item_plan_option_mirror_teaching(context.plan_options);
    let dependency_syntax_rules = sc_few_shot::work_item_plan_dependency_syntax_rules();
    let reference_discipline =
        work_item_plan_markdown_reference_discipline(Some(context.design_requirement_ids));
    // SG5.1（REQ-CG-03、REQ-SG-02）：已确认成员/仓序教学——LC 多仓 plan 注入
    // （数据单源 member targets/change_order），legacy 单仓空串缺席，首轮
    // prompt 逐字节不变；修订轮以首轮为前缀同源继承。
    let confirmed_member_order = work_item_plan_confirmed_member_order_teaching(
        context.member_targets,
        context.change_order,
    );
    // story/design 真实上下文可能远大于旧 JSON outline 路径。固定 grammar、最小合法
    // source 与 few-shot 永不截断；只按确定性配额压缩可再加载的上下文，并显式标记。
    let (story_context, design_context, repository_structure) = budget_markdown_context(
        context.story_context,
        context.design_context,
        context.repository_structure,
    );
    // D5 事前（REQ-WSC-06）：基线树清单注入 [confirmed_context]（上下文
    // 面），位于 structure 之后、story_spec_ids 之前；None/空 ⇒ 整段
    // 缺席（不留 baseline: 空标记），既有 fixture prompt 逐字节不变。
    // 清单**计入上下文面同一配额**（owner 2026-11-10 裁决：新增直供内容计入
    // 其所属面）：上下文面的**全部**字节都算在这 32,000 内 —— 三段压缩正文
    // + 清单 + 各段前缀 + spec_ids 行。故先算固定开销，再把**剩余**额度给清单
    // （上限 32,000/4）；额度不足则按行截断并显式标记；额度为 0 ⇒ 整段缺席
    // ⇒ 上下文面恒 ≤ 32,000。计算实现在 `sc_baseline_tree`。
    let baseline_tree_section = if cfg!(feature = "wig-d5-before") {
        sc_baseline_tree::baseline_tree_section(
            &story_context,
            &design_context,
            &repository_structure,
            &request.story_spec_ids,
            &request.design_spec_ids,
            context.baseline_tree,
        )
    } else {
        String::new()
    };
    let prompt = format!(
        "只输出完整 `work-item-plan.md` source。\n\
         [issue] {issue_title}\n{issue_description}\nrepo={repository_id} path={repository_path}\n\
         [routing_reference]\n{routing_reference}\n\
         [confirmed_context]\nstory:{story_context}\ndesign:{design_context}\nstructure:{repository_structure}\n\
         {baseline_tree_section}story_spec_ids:{story_spec_ids}\ndesign_spec_ids:{design_spec_ids}\n\
         [source_boundary]\n\
         不得从 issue、prompt 或 runtime 补齐 markdown 缺失字段。\n\
         exclusive_scopes 仅限本项且依赖项不得重叠；non_goals 不得与 tasks、验收、write policy 矛盾。\n\
         不要 JSON、私有协议、私有 draft、classifier 字段。\n\
         AC 与验证计划引用的仓库内文件路径必须存在于 plan 基线树，不得引用其他分支才存在的文件；缺失 → acceptance_path_not_in_baseline Error；修复动作：{acceptance_baseline_repair_action}。\n\
         {dependency_syntax_rules}\n\n\
         {reference_discipline}
         {weak_model_discipline}
         {grammar}\
         [multi_source_traceability_example] 多 source_type 的 Traceability 形态——每组三元组各自带 requirement_id(逐字取自 [design_requirements] 清单;某组缺 requirement_id → lowering_error「traceability 缺少 requirement_id」拒绝):\n{traceability_example}\n\
         [minimum_legal_source] 仅示语法形状；按当前上下文替换，勿照抄。\n{minimum_source}\n\
         {few_shot}\n\
         {plan_options_mirror}\
         {confirmed_member_order}\
         [format_clamp]\n\
         结构标题必须逐字照抄 [markdown_grammar]/[minimum_legal_source] 的英文原文（含 `# Work Item Plan` 与全部 `###` 标题）；仅自由文本值用中文；禁止翻译、改写或加中文括号。\n\
         禁止任何前言、寒暄、说明性开场或路由回执；输出的第一个字符必须是文档标题。\n\
         {output_directive}",
        issue_title = issue.title,
        issue_description = issue.description.as_deref().unwrap_or("无"),
        repository_id = repository.id,
        repository_path = repository.path.display(),
        routing_reference = single_candidate_project_rules_reference(
            context.language_rules,
            context.routing_context,
        ),
        story_context = story_context,
        design_context = design_context,
        repository_structure = repository_structure,
        baseline_tree_section = baseline_tree_section,
        story_spec_ids = request.story_spec_ids.join(", "),
        design_spec_ids = request.design_spec_ids.join(", "),
        acceptance_baseline_repair_action = ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION,
        reference_discipline = reference_discipline,
        weak_model_discipline = WORK_ITEM_PLAN_WEAK_MODEL_PRECISION_DISCIPLINE,
        grammar = work_item_plan_markdown_grammar(),
        dependency_syntax_rules = dependency_syntax_rules,
        minimum_source = work_item_plan_minimum_legal_source(),
        traceability_example = WORK_ITEM_PLAN_MULTI_SOURCE_TRACEABILITY_EXAMPLE,
        few_shot = few_shot,
        confirmed_member_order = confirmed_member_order,
        output_directive = WORK_ITEM_PLAN_MARKDOWN_OUTPUT_DIRECTIVE,
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

// 基线树清单的截断标记（渲染器内部截断与外层配额截断共用）——常量在
// prompts/sc_baseline_tree.rs。

/// 按**行边界**把已渲染的基线树清单截到 `cap` 字节；截断时以显式标记行
/// 收尾（计入 cap），MUST NOT 把部分清单冒充全量。只删行、不改行内容 ⇒
/// 不引入臆造路径。
/// D5 事前（REQ-WSC-06「基线树事实直供」）：渲染 plan 基线树可用路径清单，
/// 供 `[confirmed_context]` 的 `baseline:` 段直供（上下文面）。
///
/// - 分组：先按 `member_dirs` 顺序（成员仓优先，与 D3 成员仓摘要同序）以路径
///   顶层段分组；未命中的路径按**路径自身首段**分组（根文件归 `(根文件)`）；
///   组内路径按字典序（BTreeSet 迭代序）。
/// - 行形态：`成员目录: 路径`；路径逐字取自基线树，禁止平台臆造路径。
/// - 渲染上限 = `WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES / 4`：超限时按
///   成员仓轮转保底截断——每轮每组各取 1 条直到用满上限，保证每个成员仓
///   至少 1 条；被截断时以显式标记行收尾（计入上限），不得把部分清单
///   冒充全量。
///
/// D5 事前（REQ-WSC-06）：作者轮启动前的基线树装载决策——
/// - `Ok(Some(tree))`（非空）⇒ 渲染为成员仓分组清单，注入 prompt；
/// - `Ok(None)`（无仓 issue）/ 空树 ⇒ 该段缺席（零回归）；
/// - `Err(_)` ⇒ 仅记诊断、该段缺席，**不阻断作者轮**；既有 fail-closed
///   点仍在 Evaluate 阶段（`plan_baseline_tree` 的 `Err` 在预校验原样上抛）。
///
/// 基线树来源唯一：调用方传入 `plan_baseline_tree` 的装载结果（与
/// Evaluate 期同函数同源），本函数不新增第二套装载。
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

        let repository_structure =
            summarize_repository_structure(&repository.path, is_lc_issue(issue));
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
        let repository_structure =
            summarize_repository_structure(&repository.path, is_lc_issue(issue));
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

    /// B2b2（Task 5.3，tasks.md 5.3 / 设计 §5.3）：LC 显式 fresh 恢复的
    /// outline 反馈族 full fresh invocation——prompt 自足：完整
    /// story/design/repository 材料（与初次 full 同源收集）+
    /// context_resolutions（durable outline context index 只读保留）+
    /// 上一版 outline 全文（session artifact 版本按 run_context 版本钉选）
    /// + durable 返修反馈 + review 记录；不依赖旧 provider 会话历史，调用方
    ///   以全新会话启动（resume=None，旧 native id 不回捞）。
    ///
    /// `source_context` 仅接受 legacy outline 三 tag
    /// （legacy_outline_author / legacy_outline_revision /
    /// legacy_outline_rebuild）；其它 tag typed 拒
    /// （`lc_resume_recovery_context_unsupported`），不回落通用出生/增量
    /// 路径。revision/rebuild(outline_revision origin) 的 durable 反馈为空时
    /// 注入默认返修文案——禁止无反馈出生 full。
    pub(crate) fn build_outline_fresh_revision_invocation(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        business_repository: &RepositoryRecord,
        author: ProviderName,
        source_context: &crate::product::models::LcRecoveryRunContext,
        routing: &RoutingReferenceContext,
    ) -> ApiResult<WorkItemSplitInvocation> {
        let facts = outline_fresh_revision_facts(source_context)?;
        let story_context = collect_story_context(lifecycle, request, issue)?;
        let design_context = collect_design_context(lifecycle, request, issue)?;
        let repository_structure =
            summarize_repository_structure(&business_repository.path, is_lc_issue(issue));
        let capabilities = merge_design_context_capabilities(&design_context);
        let gaps = design_context_gaps(&capabilities);
        let context_resolutions =
            load_fresh_outline_context_resolutions(lifecycle, issue, &facts.plan_entity_id);
        let previous_outline = load_fresh_prior_outline_full_text(lifecycle, issue, &facts);
        let review_sections = load_fresh_review_feedback_sections(lifecycle, issue, &facts);
        let (prompt, sentinel_nonce) = build_outline_fresh_revision_prompt_with_nonce(
            request,
            issue,
            business_repository,
            &story_context,
            &design_context,
            &repository_structure,
            &gaps,
            &context_resolutions,
            previous_outline.as_deref(),
            facts.feedback.as_deref(),
            &review_sections,
            routing,
        );

        Ok(WorkItemSplitInvocation {
            prompt,
            provider_type: provider_name_to_type(&author),
            worktree_path: business_repository.path.to_string_lossy().to_string(),
            author_provider: author,
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

        let repository_structure =
            summarize_repository_structure(&repository.path, is_lc_issue(issue));
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

// ---------------------------------------------------------------------------
// B2b2（Task 5.3，设计 §5.3）：outline 反馈族 fresh invocation 的 durable
// 事实装载。全部只读：上一版 outline 全文与 review 记录按
// run_context 携带的 plan entity 定位 session artifact 版本；context
// resolutions 只读保留（不重算 capabilities/gaps——那是出生路径
// `load_work_item_plan_outline_context_resolutions` 的职责）。解析不到的
// 可选事实省略对应 prompt 小节，不倒算、不伪造。
// ---------------------------------------------------------------------------

/// typed 拒码——与 `workspace_ws_handler::socket` 的
/// `LC_RESUME_RECOVERY_CONTEXT_UNSUPPORTED_CODE` 同值（socket 模块对
/// product 层私有，此处为唯一可用引用形态；wire 值由 t13/t15 用例钉住）。
const OUTLINE_FRESH_CONTEXT_UNSUPPORTED_CODE: &str = "lc_resume_recovery_context_unsupported";

/// fresh 返修默认反馈——与 provider_run 臂增量路径的缺省文案同语义
/// （Author Confirm 阶段用户要求返修）；fresh 恢复的 durable 反馈为空时
/// 不得无反馈出生 full。
const OUTLINE_FRESH_DEFAULT_REVISION_FEEDBACK: &str =
    "用户在 Author Confirm 阶段要求返修当前 Outline";

struct OutlineFreshRevisionFacts {
    plan_entity_id: String,
    feedback: Option<String>,
    outline_version: Option<u32>,
    review_refs: Vec<String>,
}

/// run_context tag 的稳定短名（typed 拒错误文案用，不泄业务 payload）。
fn lc_recovery_context_tag_label_for_prompt(
    context: &crate::product::models::LcRecoveryRunContext,
) -> &'static str {
    use crate::product::models::LcRecoveryRunContext;
    match context {
        LcRecoveryRunContext::Author { .. } => "author",
        LcRecoveryRunContext::AuthorChoiceFollowup { .. } => "author_choice_followup",
        LcRecoveryRunContext::AuthorFeedback { .. } => "author_feedback",
        LcRecoveryRunContext::ReviewerRevision { .. } => "reviewer_revision",
        LcRecoveryRunContext::LegacyOutlineAuthor { .. } => "legacy_outline_author",
        LcRecoveryRunContext::LegacyOutlineRevision { .. } => "legacy_outline_revision",
        LcRecoveryRunContext::LegacyOutlineRebuild { .. } => "legacy_outline_rebuild",
        LcRecoveryRunContext::LegacyDraft { .. } => "legacy_draft",
        LcRecoveryRunContext::LegacyBatch { .. } => "legacy_batch",
        LcRecoveryRunContext::LegacyPlanRevision { .. } => "legacy_plan_revision",
        LcRecoveryRunContext::ScAuthor { .. } => "sc_author",
        LcRecoveryRunContext::ScManualRevision { .. } => "sc_manual_revision",
    }
}

/// legacy outline 三 tag → fresh 返修事实；其它 tag typed 拒。
fn outline_fresh_revision_facts(
    source_context: &crate::product::models::LcRecoveryRunContext,
) -> ApiResult<OutlineFreshRevisionFacts> {
    use crate::product::models::{LcOutlineRebuildOrigin, LcRecoveryRunContext};
    fn non_empty_or_default(feedback: &str) -> String {
        let trimmed = feedback.trim();
        if trimmed.is_empty() {
            OUTLINE_FRESH_DEFAULT_REVISION_FEEDBACK.to_string()
        } else {
            trimmed.to_string()
        }
    }
    match source_context {
        LcRecoveryRunContext::LegacyOutlineAuthor { business, .. } => {
            Ok(OutlineFreshRevisionFacts {
                plan_entity_id: business.entity_id.clone(),
                feedback: None,
                outline_version: None,
                review_refs: business.review_refs.clone(),
            })
        }
        LcRecoveryRunContext::LegacyOutlineRevision {
            business,
            normalized_feedback,
            outline_version,
            ..
        } => Ok(OutlineFreshRevisionFacts {
            plan_entity_id: business.entity_id.clone(),
            feedback: Some(non_empty_or_default(normalized_feedback)),
            outline_version: *outline_version,
            review_refs: business.review_refs.clone(),
        }),
        LcRecoveryRunContext::LegacyOutlineRebuild {
            business, origin, ..
        } => match origin {
            LcOutlineRebuildOrigin::LegacyAuthor(_payload) => Ok(OutlineFreshRevisionFacts {
                plan_entity_id: business.entity_id.clone(),
                feedback: None,
                outline_version: None,
                review_refs: business.review_refs.clone(),
            }),
            // rebuild origin=outline_revision：feedback 不丢（设计 §5.3）。
            LcOutlineRebuildOrigin::OutlineRevision(payload) => Ok(OutlineFreshRevisionFacts {
                plan_entity_id: business.entity_id.clone(),
                feedback: Some(non_empty_or_default(&payload.normalized_feedback)),
                outline_version: payload.outline_version,
                review_refs: business.review_refs.clone(),
            }),
        },
        other => Err(ApiError::validation(
            OUTLINE_FRESH_CONTEXT_UNSUPPORTED_CODE,
            format!(
                "outline fresh revision invocation requires a legacy outline run_context (legacy_outline_author/legacy_outline_revision/legacy_outline_rebuild); got tag {}",
                lc_recovery_context_tag_label_for_prompt(other)
            ),
        )),
    }
}

/// 定位 plan entity 的 WorkItemPlan 会话 record（fresh 返修 durable 事实
/// 的 session 域）；多会话时取 updated_at 最新，不倒算。
fn fresh_outline_session_record(
    lifecycle: &LifecycleStore,
    issue: &IssueRecord,
    plan_entity_id: &str,
) -> Option<crate::product::models::WorkspaceSessionRecord> {
    lifecycle
        .list_workspace_sessions(&issue.project_id, &issue.id)
        .ok()?
        .into_iter()
        .filter(|record| {
            record.entity_id == plan_entity_id
                && record.workspace_type == crate::product::models::WorkspaceType::WorkItemPlan
        })
        .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
}

/// 上一版 outline 全文：session artifact 版本中优先 run_context 钉选的
/// outline_version，缺省取最新 WorkItemPlanOutlineCandidate。
fn load_fresh_prior_outline_full_text(
    lifecycle: &LifecycleStore,
    issue: &IssueRecord,
    facts: &OutlineFreshRevisionFacts,
) -> Option<String> {
    let session = fresh_outline_session_record(lifecycle, issue, &facts.plan_entity_id)?;
    let versions = lifecycle
        .list_artifact_versions_for_issue_session(&issue.project_id, &issue.id, &session.id)
        .ok()?;
    let mut pinned = None;
    let mut latest = None;
    for version in &versions {
        if matches!(
            &version.payload,
            crate::web::workspace_ws_types::ArtifactPayload::WorkItemPlanOutlineCandidate { .. }
        ) {
            if facts.outline_version == Some(version.version) {
                pinned = Some(&version.payload);
            }
            latest = Some(&version.payload);
        }
    }
    let payload = pinned.or(latest)?;
    let crate::web::workspace_ws_types::ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate,
    } = payload
    else {
        return None;
    };
    serde_json::to_string_pretty(&outline_candidate.outline).ok()
}

/// review 记录：run_context review_refs 指向的 durable 节点 verdict 全文
/// （压缩历史的最小自足面——反馈文本已含摘要时 refs 补齐可追溯正文）。
fn load_fresh_review_feedback_sections(
    lifecycle: &LifecycleStore,
    issue: &IssueRecord,
    facts: &OutlineFreshRevisionFacts,
) -> Vec<String> {
    if facts.review_refs.is_empty() {
        return Vec::new();
    }
    let Some(session) = fresh_outline_session_record(lifecycle, issue, &facts.plan_entity_id)
    else {
        return Vec::new();
    };
    facts
        .review_refs
        .iter()
        .filter_map(|node_id| {
            let detail = lifecycle.load_node_detail(&session.id, node_id).ok()?;
            let verdict = detail.verdict.as_ref()?;
            Some(format!(
                "review 节点 {node_id} 的 durable 审核记录:\n{verdict}"
            ))
        })
        .collect()
}

/// context resolutions 只读保留：durable outline context index 的
/// blocker_resolutions（出生路径的 capabilities/gaps 重算不在 fresh 面）。
fn load_fresh_outline_context_resolutions(
    lifecycle: &LifecycleStore,
    issue: &IssueRecord,
    plan_entity_id: &str,
) -> Vec<OutlineContextBlockerResolution> {
    crate::product::work_item_plan_store::WorkItemPlanStore::new(lifecycle.app_paths())
        .load_outline_context_index(&issue.project_id, &issue.id, plan_entity_id)
        .ok()
        .flatten()
        .map(|index| index.blocker_resolutions)
        .unwrap_or_default()
}

/// F-60 P1b：Split 初次/redo 共享的 [output_schema] 合同渲染（增量契约等价）。
///
/// sentinel 规则、可读过程纪律、kind 合法值指导与完整 schema 两轮同源；仅
/// schema 限定语（`schema_tail`）与数组组成条款（`composition_rule`）按轮次
/// 置换（初次=全量执行顺序／redo=仅重做项对应 redo_work_items）。redo 轮
/// 不依赖会话历史补合同。
fn split_output_contract(nonce: &str, composition_rule: &str, schema_tail: &str) -> String {
    format!(
        "可以在最终结构化 JSON 前输出简短、可读的拆分过程，供 Workbench 流式展示。\n\
         长时间分析、探索代码库或自动修正前，先输出一行简短可读状态，供 Workbench 流式展示；不要等待所有工具调用结束后才给第一段说明。\n\
         如果需要执行多步代码库探索，每完成一组探索后输出一句当前发现摘要。\n\
         这些可读状态必须位于最终 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\"> 之前；最终结构化 JSON 仍只放在最后一个 sentinel block 中。\n\
         最后必须输出一个 nonce sentinel JSON block。\n\
         后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">...</ARIA_STRUCTURED_OUTPUT> block。\n\
         标签内部必须是一个完整 JSON object，JSON 顶层必须含 `\"nonce\":\"{nonce}\"` 并与开始标签一致，不要输出 Markdown code fence。\n\
         nonce 纪律：开始标签必须逐字回显 `<ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">`——属性名恰为 `nonce`，属性值两侧必须是英文直双引号 `\"`（禁止中文弯引号 “ ”、全角引号或单引号），nonce 值 `{nonce}` 逐字照抄不得改写。\n\
         严格按以下 JSON schema 输出{schema_tail}。\n\
         {composition_rule}\n\
         每个 work_item 必须包含 `kind` 字段（不要写成 `type`），合法取值为以下之一：backend、frontend、integration、e2e、docs、infra、other。\n\n\
         {schema}",
        nonce = nonce,
        schema_tail = schema_tail,
        composition_rule = composition_rule,
        schema = WORK_ITEM_SPLIT_OUTPUT_SCHEMA,
    )
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
         {output_contract}",
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
        output_contract = split_output_contract(
            &nonce,
            "work_items 数组顺序即执行顺序；depends_on 使用同数组中的 0-based 索引。verification_plans 数组与 work_items 一一对应。",
            "",
        ),
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
         [openspec_constraint_summary]\n\
         story_spec_ids: {story_ids}\n\
         design_spec_ids: {design_ids}\n\n\
         [user_options]\n\
         include_integration_tests: {include_integration_tests}\n\
         include_e2e_tests: {include_e2e_tests}\n\
         force_frontend_backend_split: {force_frontend_backend_split}\n\
         require_execution_plan_confirm: {require_execution_plan_confirm}\n\n\
         [output_schema]\n\
         {output_contract}",
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
        story_ids = request.story_spec_ids.join(", "),
        design_ids = request.design_spec_ids.join(", "),
        include_integration_tests = request.include_integration_tests.unwrap_or(false),
        include_e2e_tests = request.include_e2e_tests.unwrap_or(false),
        force_frontend_backend_split = request.force_frontend_backend_split.unwrap_or(false),
        require_execution_plan_confirm = request.require_execution_plan_confirm.unwrap_or(false),
        output_contract = split_output_contract(
            &nonce,
            "work_items 数组必须且仅包含重做项，顺序对应 redo_work_items 列表；verification_plans 与 work_items 一一对应；depends_on 使用 0-based 索引。",
            " redo-only 结果",
        ),
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
