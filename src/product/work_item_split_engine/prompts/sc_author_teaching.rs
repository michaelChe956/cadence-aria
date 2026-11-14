//! SC author prompt 固定教学面：markdown grammar、交叉引用纪律、弱模型精度
//! 教学、最小合法源、options 镜像/AC 基线纪律与已确认成员/仓序教学
//! （SG5.1）。自 prompts.rs 拆出以守 1200 行文件守护（先例：
//! prompts/sc_revision.rs、conversational_gate 拆 story_terminate.rs）。
//!
//! A+（2026-10-31，openspec change `unify-work-item-group-structure`
//! tasks.md 5.0/5.4，REQ-WG-07）：`Plan Intent` 的 `intent_target_repo`
//! 取值来源改为**按会话类型唯一**——grammar 段一条权威句按会话类型分支，
//! LC 成员段只复述「取上列物理成员仓 ID」并保留否定式提醒。契约见
//! `tests/prompt_contract_plan_intent_target.rs`。
//!
//! Task 7（2026-11-12，openspec change `mechanicalize-plan-compile-repair`
//! 工作包 7.1/7.2/7.3，REQ-CRP-05）：固定教学块按计量闸门瘦身——基线实测
//! 单仓 14,861 字符/63 硬措辞、LC 16,176/69（计量用例
//! `teaching_block_volume_within_budget`），目标 ≤8,000 字符/≤25 硬措辞。
//! 删/压清单（tasks 7.2 逐项；每删一条机械条款须有平台侧覆盖可指向，
//! 逐条映射表见本 change 提交信息）：重复 trusted command 段收一条、
//! 40-key 白名单枚举删除（最小合法源示例 + `unknown_structured_key`
//! fail-closed + autorepair 指令承接）、Plan Intent 分支复述收短、
//! `lowering_error` 引文删除、模板枚举展开压缩（compatibility_policy/
//! required_evidence/route 以最小合法源为准）、few-shot 11→3（见
//! `sc_few_shot.rs`）、成环预防保留单处。祈使措辞（必须|不得|禁止|拒绝）
//! 改写为等价非祈使表述或「→ 错误码」箭头；判定语义不变，由平台侧
//! 校验器、typed 诊断与教学重驱（Task 1–5）承接。

use super::IssueWorkItemPlanOptions;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::work_item_plan_compiler::PlanMemberTargets;
use crate::product::work_item_plan_compiler::grammar;
use crate::product::work_item_split_validator::{
    E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION, FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION,
    INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION,
};

pub(crate) fn work_item_plan_markdown_grammar() -> String {
    format!(
        "[markdown_grammar]\n\
         输出的第一行精确为 `{document_heading}`；之前无任何前言、解释、空白行或代码围栏（```）。\n\
         section 按序各一次（顺序与标题形状见 [minimum_legal_source]）；自由文本仅 `{free_text_sections}`。\n\
         输出精炼：每个 statement 恰好一句话；同一信息不在多个 section 重复；不写解释性散文——机械校验只消费结构化字段。\n\
         Blockers 为空时保留空 section（### Blockers 后直接下一 section）；有 blocker 时写全 reason_code、route、target_contract_refs。\n\
         Verification.command 直接声明、将按声明执行；证据不足改用 manual_instruction 或 blocker，不臆造命令。\n\
         statement 模板 `{ears_template}`。\n\
         task_id、criterion_id、check_id、contract_id 全文档全局唯一且全局递增（第二个 Work Item 从 TASK-004、AC-004 续起，不在 item 内重新编号）。\n\
         CJK 空格规则：WHEN、条件文本、THE SYSTEM SHALL 两两之间各留一个半角空格，中文条件与 `」` 收尾同样；反例（缺空格，非法）：`WHEN服务读取静态文件 THE SYSTEM SHALL 返回五项记录`。\n\
         Inputs 四行且 contract_id 首行：contract_id、provider_logical_work_item_id、required_capabilities、compatibility_policy: require_all|require_any，四行缺一不可；无依赖时留空 section。\n\
         反例：provider_logical_work_item_id 的合法值只能来自本计划的 `## Work Item` 标题中的 `WI-<数字>`；story_spec_0001/design_spec_0001 等 spec id 一律非法。\n\
         同一 Work Item 内同一 command 至多声明一次；复合验证合并为一条 check 或改用 manual_instruction（重复 → 编译器拒绝）。\n\
         值域：kind={item_kinds}；intent=create、existing（新建一律 create）；route={blocker_routes}；required_evidence={evidence_kinds}。\n\
         Plan Intent 分支必填：intent_target_kind=single_repository 时必写 intent_target_repo；intent_target_kind=logical_codebase 时必写 intent_target_codebase 与 intent_target_logical_repo（logical codebase id 与成员仓 UUID）。两分支 key 不顶替。intent_target_repo 取值来源按会话类型唯一、不得混填：单仓会话（本 prompt 无成员仓清单）逐字取 [issue] 头 repo= 的 repository id；LC 多仓会话（有成员仓清单）逐字取该清单的物理成员仓 ID。\n\
         未知结构化 key 一律拒绝（{unknown_key_policy}）；未知 section、非法 ID、EARS 非法与缺 section/field 均失败关闭。\n\n",
        document_heading = grammar::DOCUMENT_HEADING,
        free_text_sections = grammar::FREE_TEXT_SECTIONS.join("、"),
        ears_template = grammar::EARS_STATEMENT_TEMPLATE,
        item_kinds = grammar::ALLOWED_ITEM_KINDS.join("、"),
        unknown_key_policy = grammar::UNKNOWN_STRUCTURED_KEY_POLICY,
        blocker_routes = grammar::ALLOWED_BLOCKER_ROUTES.join("|"),
        evidence_kinds = grammar::ALLOWED_EVIDENCE_KINDS.join("|"),
    )
}

pub(crate) fn work_item_plan_markdown_reference_discipline(
    requirement_ids: Option<&[String]>,
) -> String {
    let requirement_ids = requirement_ids
        .filter(|ids| !ids.is_empty())
        .map(|ids| ids.join("、"))
        .unwrap_or_else(|| "（无；不编造）".to_string());
    format!(
        "[design_requirements] {requirement_ids}\n\
         [cross_reference_discipline]\n\
         done_when_refs 仅引用**本 WI** 自己的 `- criterion_id:`；不得跨 WI，也不得取自 [design_requirements]。\n\
         target_contract_refs 仅逐字引用已登记 input/output contract_id。\n\
         requirement_refs 仅引用 [design_requirements] 清单。\n\
         handoff 的 reviewer_check_refs 与全部且仅本 item 的 acceptance criterion ID 集合完全一致（每条 AC 恰好被检查一次）。\n\
         能力原子化：每个 `- capabilities:` 与 `- required_capabilities:` 字段一行只写一个精确能力字符串，不用「A + B」、逗号、斜杠或自然语言合并多能力——`require_all` 按字符串逐字精确匹配，合并会造成假性 required_capability_missing；端点/动作（如 `GET /api/levels`）显式声明，字段/记录不隐含端点。\n\
         require_all=全部覆盖（缺一项→required_capability_missing）；require_any=至少一项相交（交集空→required_capability_missing）。\n\
         Handoff Schema 显式输出 required_fields、provided_contract_refs、reviewer_check_refs 三字段且只允许这三个 key（其他 key 如 requested_fields → unknown_structured_key 拒绝）。\n\
         Outputs 与 Handoff Schema 相互独立：每个 Work Item 的 Outputs 至少声明一个契约（contract_id + capabilities），永不为空。\n\
         provided_contract_refs 列出会被下游 Work Item 的 input_contracts 以 (provider_logical_work_item_id, contract_id) 逐字二元组消费的契约引用；同 WI 内唯一（重复即拒）；无交接时写 []。\n\
         每个被 tasks 的 requirement_refs 引用的 requirement_id，在本 item 的 Traceability section 有逐字相同的登记行；登记值只来自 [design_requirements] 清单。\n\n"
    )
}

/// 3.6 弱模型基线加固 → Task 7（REQ-CRP-05）按计量闸门瘦身：字段名、错误码
/// 与判定语义仍与 `work_item_contract::validation.rs` / `dependency.rs` 逐字
/// 对齐（对齐断言见 prompt_contract_weak_model 的两个
/// `*_teaching_matches_*_validator_judgement` 行为级用例）；祈使措辞改写为
/// 「→ 错误码」箭头，教学正例压缩，判定语义与错误码不变。
pub(crate) const WORK_ITEM_PLAN_WEAK_MODEL_PRECISION_DISCIPLINE: &str = "\
[weak_model_precision]
\
AC 纪律：每个 `- criterion_id: AC-xxx` 在 Handoff Schema 配对一行 `- reviewer_check_refs: AC-xxx`；漏写 → acceptance_criterion_without_reviewer_check。
\
引用纪律：requirement_refs 逐字取 [design_requirements] 的 spec id（REQ-*/NFR-*），清单外 → unknown_requirement_ref。done_when_refs 只引用**本 WI** 自己的 `- criterion_id:`，跨 WI 或清单外 AC-* → unknown_done_when_ref。\
\
输出契约纪律：下游 required_capabilities 写什么，上游被 (provider_logical_work_item_id, contract_id) 指向的 Outputs capabilities 就逐字复制什么（require_all 逐项覆盖）；同一契约全计划只声明一次——多个下游消费同一契约时把全部能力串合并进这唯一一份 capabilities，不按下游重复声明。反例①改写/概括/漏一项 → required_capability_missing；反例②按下游重复声明 → duplicate_contract_id。
成对书写纪律（DEF-PVR-ALL）：跨 WI 依赖两侧成对书写——下游写 `- required_capabilities: [X]` 时，在上游被 (provider_logical_work_item_id, contract_id) 指向的 Outputs 同一契约补 `- capabilities: X`，两侧能力串逐字相同才算写完这一对；反例只写消费侧忘写提供侧 → required_capability_missing。
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
     ### Plan Intent\n\
     - intent: create\n\
     - provider_work_item_id: WI-001\n\
     - intent_target_kind: single_repository\n\
     - intent_target_repo: x\n\
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

/// SG5.1（REQ-CG-03、REQ-SG-02）：SC author prompt 的已确认成员/仓序教学
/// ——初轮与修订轮共享同源（修订 prompt 以首轮为逐字节前缀，自动继承）。
///
/// 数据单源：成员表=compile member targets（logical UUID → 物理成员仓 ID）、
/// 仓序=confirmed design change_order；本段不另立白名单——成员归属合法值
/// 即上列已确认事实，grammar 值域/WI 合法 ID 仍由
/// `work_item_plan_markdown_grammar` 单源注入，仓序自动边规则与编译器
/// （`compile_support::apply_change_order_cross_repo_depends_on`）同一条
/// 规则源。`member_targets=None`（legacy 单仓）返回空串：首轮 prompt
/// 逐字节不变，`WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES`
/// 预算不扩大。Plan Intent 保持可选 section，本段不新增任何创建/入站授权。
///
/// Task 7（REQ-CRP-05）：成员归属段按计量闸门收短——取值权威句仍在
/// grammar 段（按会话类型分支），本段保留成员表事实、否定式提醒与平台侧
/// typed 拒绝码（work_item_target_missing）；写范围路径基准段整体迁平台侧
/// 承接（`write_scope_bound.rs` 越界拒收 + `tests/write_scope_bound.rs`），
/// LC 会话仅保留成员表与仓序事实。
pub(crate) fn work_item_plan_confirmed_member_order_teaching(
    member_targets: Option<&PlanMemberTargets>,
    change_order: &[LogicalRepositoryId],
) -> String {
    let Some(members) = member_targets else {
        return String::new();
    };
    if members.physical_by_logical.is_empty() {
        return String::new();
    }
    let mut lines = String::new();
    lines.push_str("[confirmed_member_order]\n");
    lines.push_str("已确认成员仓（成员归属唯一合法值域，逐字取用）：\n");
    for (logical, physical) in &members.physical_by_logical {
        lines.push_str(&format!(
            "- logical_repository: {} → physical_repository: {physical}\n",
            logical.0
        ));
    }
    if change_order.is_empty() {
        lines.push_str(
            "仓级执行顺序（change_order）：无已确认仓序——本计划无仓序自动边，跨仓依赖仅由 depends_on 与 Inputs 契约显式声明，不得虚构先后顺序。\n",
        );
    } else {
        lines.push_str("仓级执行顺序（先执行 → 后执行）：\n");
        for (index, logical) in change_order.iter().enumerate() {
            match members.physical_by_logical.get(logical) {
                Some(physical) => {
                    lines.push_str(&format!("{}. {}（{physical}）\n", index + 1, logical.0))
                }
                None => lines.push_str(&format!("{}. {}\n", index + 1, logical.0)),
            }
        }
    }
    lines.push_str(
        "仓序自动边规则（与编译器仓序规则同源）：后仓 WI 消费前仓产出时，Inputs 的 provider_logical_work_item_id 逐字填前仓 WI 在本计划 `## Work Item` 标题中的 `WI-<数字>`；同一成员仓内的 WI 之间无仓序自动边（同仓顺序仅由 depends_on 声明）；跨仓依赖逆写仓序构成 dependency_cycle，整份计划拒绝。\n",
    );
    lines.push_str(
        "成员归属纪律（LC 多仓会话）：intent_target_kind=logical_codebase 时 intent_target_logical_repo 只能逐字取上列成员 logical UUID；intent_target_kind=single_repository 时 intent_target_repo 只能逐字取上列物理成员仓 ID（不取 [issue] 头 repo=）；会话根仓/聚合根视图 ID 不得替代业务成员 target（→ work_item_target_missing 拒绝）。Plan Intent 仍为可选 section，不新增任何创建/入站授权。\n\n",
    );
    lines
}

#[cfg(test)]
mod section_keys_prompt_tests {
    use super::{
        grammar, work_item_plan_markdown_grammar, work_item_plan_markdown_reference_discipline,
    };

    /// 首轮 prompt 必须教 provided_contract_refs 在单 WI 内唯一——线上实测出现过
    /// `duplicate_handoff_provided_contract_ref` 直接终态失败（无教学资格）。
    #[test]
    fn first_round_prompt_requires_unique_provided_contract_refs() {
        let discipline = work_item_plan_markdown_reference_discipline(None);
        assert!(
            discipline.contains("同 WI 内唯一"),
            "首轮引用纪律必须声明 provided_contract_refs 在单 WI 内唯一: {discipline}"
        );
    }

    /// 首轮 prompt 必须给出两个此前缺失的 enum 完整值域（值取自同一常量，不另写）。
    #[test]
    fn first_round_prompt_lists_blockers_route_and_required_evidence_values() {
        let prompt = work_item_plan_markdown_grammar();
        for route in grammar::ALLOWED_BLOCKER_ROUTES {
            assert!(
                prompt.contains(route),
                "首轮 prompt 缺少 route 合法值: {route}"
            );
        }
        for kind in grammar::ALLOWED_EVIDENCE_KINDS {
            assert!(
                prompt.contains(kind),
                "首轮 prompt 缺少 required_evidence 合法值: {kind}"
            );
        }
    }
}
