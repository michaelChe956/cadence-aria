//! SC author prompt 固定教学面：markdown grammar、交叉引用纪律、弱模型精度
//! 教学、最小合法源与 options 镜像/AC 基线纪律。自 prompts.rs 拆出以守
//! 1200 行文件守护（先例：prompts/sc_revision.rs、conversational_gate 拆
//! story_terminate.rs）。内容零改动，纯移动。

use super::IssueWorkItemPlanOptions;
use crate::product::work_item_plan_compiler::grammar;
use crate::product::work_item_split_validator::{
    E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION, FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION,
    INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION,
};

pub(crate) fn work_item_plan_markdown_grammar() -> String {
    format!(
        "[markdown_grammar]\n\
         输出的第一行必须精确为 `{document_heading}`；之前不得有任何前言、解释、宣布、空白行或代码围栏（```）。\n\
         所有标题必须逐字使用上列英文名（一级 `# Work Item Plan`、二级 `## Work Item WI-<三位数字>: <title>`、三级 section 名恰为上列 13 个英文名之一）；禁止翻译标题、禁止附加中文注或括号。\n\
         输出保持精炼：每个 statement 恰好一句话；同一信息不得在多个 section 重复；不写解释性散文或总结段——机械校验只消费结构化字段。\n\
         section 按序各一次：{structured_sections}；自由文本仅 `{free_text_sections}`（`{free_text_policy}`）。\n\
         Blockers 为空时保留空 section（### Blockers 后直接下一 section）；存在 blocker 字段时须完整填写 reason_code、route、target_contract_refs。\n\
         Verification.command 直接声明，将按声明执行；命令证据不足时改用 manual_instruction 或 blocker，禁止臆造命令。\n\
         行 `{structured_line}`；ID 行 `{identified_line}`；statement `{ears_template}`。\n\
         task_id、criterion_id、check_id 在整份文档内全局唯一且全局递增——第二个 Work Item 的任务从 TASK-004、验收从 AC-004 继续（假设前一 item 用了 TASK-001~003），不得在每个 item 内重新从 001 编号；contract_id 同理在整份文档内全局唯一，不得重复。\n\
         CJK 空格规则：WHEN 与条件文本之间、条件文本与 THE SYSTEM SHALL 之间必须各有一个半角空格；条件为中文时同样必须（正例：`WHEN 服务读取静态文件 THE SYSTEM SHALL 返回五项记录`；反例：`WHEN服务读取静态文件 THE SYSTEM SHALL 返回五项记录` 非法）。`」` 收尾时同样必须（`WHEN 点击「重开」 THE SYSTEM SHALL 清空` 合法；`WHEN 点击「重开」THE SYSTEM SHALL 清空` 非法）。
\
         Inputs 四行且 contract_id 首行：provider_logical_work_item_id、required_capabilities、compatibility_policy（require_all|require_any）各一行；四行缺一不可。\n\
         示例：若 WI-002 依赖 WI-001 的输出，则 WI-002 的 Inputs 写：\n\
         - contract_id: <逐字复制 WI-001 Outputs 的 contract_id>\n\
         - provider_logical_work_item_id: WI-001\n\
         - required_capabilities: <该契约的能力>\n\
         - compatibility_policy: require_all\n\
         无依赖时 Inputs 留空 section。\n\
         反例：provider_logical_work_item_id 的合法值只能来自本计划的 `## Work Item` 标题中的 `WI-<数字>`；story_spec_0001/design_spec_0001 等 spec id 一律非法。\n\
         同一 Work Item 内同一 command 至多声明一次；需要复合验证时合并为一条 check 或改用 manual_instruction。违反会被编译器最终校验拒绝：lowering_error/trusted_commands「同一 Work Item 不得重复引用 trusted command。」——整份计划编译失败，需人工修复后重新编译。\n\
         反例：同一 item 的 CHECK-001 与 CHECK-002 都写 `- command: npm test` → lowering_error 拒绝。\n\
         正例：合并为一条 `- command: npm test` 的 check；或第二条写 `- command: null` 并配 `- manual_instruction: <人工步骤>`。\n\
         key 白名单：{structured_keys}。\n\
         值域：kind={item_kinds}；compatibility_policy={compatibility_policies}；required_evidence={evidence_kinds}；route={blocker_routes}。\n\
         未知结构化 key 必须拒绝（{unknown_key_policy}）；未知 section、非法 ID、除空 Blockers 外的缺 section/field、EARS 非法均失败关闭；诊断：{diagnostic_codes}。\n\n",
        document_heading = grammar::DOCUMENT_HEADING,
        structured_sections = grammar::STRUCTURED_SECTIONS.join("、"),
        free_text_sections = grammar::FREE_TEXT_SECTIONS.join("、"),
        free_text_policy = grammar::FREE_TEXT_SECTION_POLICY,
        structured_line = grammar::STRUCTURED_LINE_PREFIX,
        identified_line = grammar::IDENTIFIED_LINE_PREFIX,
        ears_template = grammar::EARS_STATEMENT_TEMPLATE,
        structured_keys = grammar::STRUCTURED_KEYS.join("、"),
        item_kinds = grammar::ALLOWED_ITEM_KINDS.join("、"),
        compatibility_policies = grammar::ALLOWED_COMPATIBILITY_POLICIES.join("、"),
        evidence_kinds = grammar::ALLOWED_EVIDENCE_KINDS.join("、"),
        blocker_routes = grammar::ALLOWED_BLOCKER_ROUTES.join("、"),
        unknown_key_policy = grammar::UNKNOWN_STRUCTURED_KEY_POLICY,
        diagnostic_codes = grammar::DIAGNOSTIC_CODES.join("、"),
    )
}

pub(crate) fn work_item_plan_markdown_reference_discipline(
    requirement_ids: Option<&[String]>,
) -> String {
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
         能力必须原子化：每个 `- capabilities:` 与 `- required_capabilities:` 字段一行只写一个精确能力字符串；禁止用「A + B」、逗号、斜杠或自然语言把多个能力合并为一个字符串。\n\
         `require_all` 按字符串逐字精确匹配；将两个能力合并成一个字符串会制造假性 `required_capability_missing`，即使语义上看似都已提供。\n\
         require_all=全部覆盖（缺一项→required_capability_missing）；require_any=至少一项相交（交集空→required_capability_missing）。\n\
         端点/动作如 `GET /api/levels` 须显式声明；字段/记录不隐含端点。\n\
         反例：CT-001 仅「五项记录+字段名称」，WI-002 require_all「field constraints」「GET /api/levels」→ canonical required_capability_missing。\n\
         正例：CT-001 显式声明两项，或 WI-002 改引供能 contract。\n\
         canonical fail-closed：required_capability_missing 拒绝 plan。\n\
         Handoff Schema 必须显式输出 required_fields、provided_contract_refs、reviewer_check_refs 三字段；禁止省略 section 或字段。\n\
         Handoff Schema 段只允许这三个 key；其他 key（如 requested_fields）会被编译器拒绝（unknown_structured_key）。\n\
         Outputs 与 Handoff Schema 相互独立：每个 Work Item 的 Outputs 必须声明至少一个契约（contract_id + capabilities），永不为空；验证/集成类 Work Item 同样声明其产出的证据类契约。\n\
         provided_contract_refs 列出本 WI 交接给下游的契约引用：只列会被下游 Work Item 的 input_contracts 以 (provider_logical_work_item_id, contract_id) 逐字二元组消费的引用；没有要交接的引用时，该字段的值写 []——[] 是 provided_contract_refs 的合法取值，不是省略字段，也不表示其他 section 可以为空。\n\
         正例：WI-002 提供 CT-005，WI-003 Inputs 写 provider_logical_work_item_id: WI-002 与 contract_id: CT-005 → 合法。\n\
         反例：WI-002 提供 CT-005 但无下游引用它 → 从 provided_contract_refs 移除 CT-005（该字段值可为 []）或补齐精确下游 Inputs；不得以省略字段、写 blocker、改 ID 或自然语言掩盖。\n\
         每个被 tasks 的 requirement_refs 引用的 requirement_id，必须在本 item 的 Traceability section 有对应登记行（requirement_id 逐字相同）；登记值只能来自 [design_requirements] 清单。\n\n"
    )
}

/// 3.6 弱模型基线加固：面向 flash 级基线的精度教学（rep1b/rep1c 两连败实证：
/// AC 缺 reviewer check、幻觉引用不存在的 REQ-002）。字段名、错误码与判定语义
/// 必须与 `work_item_contract::validation.rs` 的 IR 校验逐字对齐（对齐断言见
/// prompt_contract::weak_model_precision_teaching_matches_contract_validator_judgement）。
/// 2026-09-08 3.6 矩阵 codex×重 三连败（issue_0166/0167/0168：上游 Outputs
/// capabilities 未逐字覆盖下游 require_all 消费）追加第三条「输出契约纪律」，错误码
/// 与消息片段与 `work_item_contract::dependency.rs` 逐字对齐（对齐断言见
/// prompt_contract_weak_model::output_contract_capability_teaching_matches_dependency_validator_judgement）。
/// 🔴 不删不改既有教学段；预算红线见 WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES
/// 批注（前两条净增 547B；第三条净增 498B+2026-09-09 补合并子句，第 8/9 次提额 20,500/21,000）。
pub(crate) const WORK_ITEM_PLAN_WEAK_MODEL_PRECISION_DISCIPLINE: &str = "\
[weak_model_precision]
\
AC 纪律：每个 `- criterion_id: AC-xxx` 必须在 Handoff Schema 配对一行 `- reviewer_check_refs: AC-xxx`；正例：`- criterion_id: AC-001` 配 `- reviewer_check_refs: AC-001`；漏写 → acceptance_criterion_without_reviewer_check 拒绝。
\
引用纪律：requirement_refs/done_when_refs 只能逐字复制 spec 已定义 id（REQ-*/AC-*/NFR-*）；task 的 requirement_refs 必须逐字取自 [design_requirements] 清单。反例：引用清单没有的 REQ-002 → unknown_requirement_ref 拒绝。
\
输出契约纪律：SC 计划由你在同一文档先后写出——下游 Inputs required_capabilities 写什么，被 (provider_logical_work_item_id, contract_id) 指向的上游 Outputs capabilities 就逐字复制什么，require_all 逐项覆盖。同一契约全计划只声明一次：多个下游消费同一契约时把全部能力串合并进这唯一一份 capabilities，禁止按下游重复声明契约。正例：下游 `- required_capabilities: [数据错误返回 500 且 code=LEVEL_DATA_UNAVAILABLE]` → 上游 `capabilities:` 逐字同串；两个下游消费同一 CT-001 → 一份 CT-001 含两者能力串。反例①改写/概括/漏一项 → required_capability_missing 拒绝；反例②按下游重复声明契约 → duplicate_contract_id 拒绝。
成对书写纪律（DEF-PVR-ALL）：跨 WI 依赖必须两侧成对书写——写下下游一行 `- required_capabilities: [X]` 的同一时刻，回到被 (provider_logical_work_item_id, contract_id) 指向的上游 Outputs 同一契约补 `- capabilities: X`，两侧能力串逐字相同才算写完这一对。正例：WI-002 Inputs 写 `- provider_logical_work_item_id: WI-001` + `- contract_id: CT-001` + `- required_capabilities: [GET /api/levels 返回 200]`，WI-001 Outputs 的 CT-001 就有 `- capabilities: GET /api/levels 返回 200`。反例：只写消费侧忘写提供侧 → required_capability_missing 拒绝。
\
";

pub(crate) fn work_item_plan_minimum_legal_source() -> &'static str {
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

/// F-51/F-56（REQ-WSC-06）：SC author prompt 的创建计划选项镜像教学。
/// 仅渲染已启用 flag 的镜像行（条件渲染先例：outline 路径 split_option_semantics）；
/// 修复动作逐字引用校验器共享常量（两种修复路径），错误后果在段头明示。
/// 全 false 时返回空串（教学缺席；权威校验仍由编译前预检承担）。
pub(crate) fn work_item_plan_option_mirror_teaching(options: &IssueWorkItemPlanOptions) -> String {
    let mut lines = Vec::new();
    if options.include_integration_tests {
        lines.push(format!(
            "- include_integration_tests=true：{INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION}。"
        ));
    }
    if options.include_e2e_tests {
        lines.push(format!(
            "- include_e2e_tests=true：{E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION}。"
        ));
    }
    if options.force_frontend_backend_split {
        lines.push(format!(
            "- force_frontend_backend_split=true：{FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION}。"
        ));
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "[plan_options_mirror]\n\
         以下创建计划选项已启用；启用而缺对应 kind 的 Work Item 将被编译前预检 Error 拒绝（修复动作）：\n\
         {}\n\n",
        lines.join("\n")
    )
}
