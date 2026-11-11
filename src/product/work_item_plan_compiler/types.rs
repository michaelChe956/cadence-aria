//! `work-item-plan.md` 的语法树和公开诊断形状。
//!
//! 这些类型只描述 compiler 的稳定边界。parser 构造带行号的 AST，lowering 由后续
//! 任务实现；本模块本身不读取 source，也不生成诊断。

use crate::product::models::{IssueWorkItemPlanOptions, RepositoryProfile, WorkItemSplitFinding};

/// 带 1-based source 行号的 AST 值。
///
/// parser 只在语法阶段构造本类型；公开诊断不暴露 parser 内部 token。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned<T> {
    /// 保留的原始值。
    pub value: T,
    /// source 中的 1-based 行号。
    pub line: usize,
}

/// 一个结构化 key/value 行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanFieldAst {
    /// 结构化 key 及其所在行。
    pub key: Spanned<String>,
    /// 原始 value 及其所在行。
    pub value: Spanned<String>,
}

/// 一个结构化 section 及其原始字段行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanSectionAst {
    /// section 名称及 heading 所在行。
    pub name: Spanned<String>,
    /// 按 source 出现顺序保留的结构化字段。
    pub fields: Vec<WorkItemPlanFieldAst>,
}

/// 一个 work item 的结构化 markdown 内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanItemAst {
    /// heading 中的 `WI-<digits>` 标识符及 heading 所在行。
    pub id: Spanned<String>,
    /// heading 标题及 heading 所在行。
    pub title: Spanned<String>,
    /// 按 source 出现顺序保留的结构化 section。
    pub sections: Vec<WorkItemPlanSectionAst>,
}

/// `work-item-plan.md` 的文档级 AST 容器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanAst {
    /// 文档中按出现顺序排列的 work item。
    pub items: Vec<WorkItemPlanItemAst>,
    /// 文档或项目尾部 `Notes` section 的自由文本及其行号。
    pub notes: Vec<Spanned<String>>,
    /// 文档或项目尾部 `Rationale` section 的自由文本及其行号。
    pub rationale: Vec<Spanned<String>>,
}

/// compiler 对 source grammar 或 lowering 输入返回的稳定诊断形状。
///
/// parser/linter 与 lowering 均使用此形状返回真实行号、字段路径和可回喂修复示例。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerDiagnostic {
    /// 稳定的 diagnostic code，例如 `missing_section`。
    pub code: String,
    /// source 中的 1-based 行号；尚未产生实际诊断时可由调用方保留约定值。
    pub line: usize,
    /// canonical field path 或 grammar field 名称。
    pub field: String,
    /// 面向调用方的诊断消息。
    pub message: String,
    /// 恰好一个可回喂的修复示例。
    pub repair_example: String,
    /// F1.2（REQ-SG-01/02/02A）：源分支显式写入的 typed 失败 payload。
    /// 只有机器事实完整的分支才写 `Some`（首个 producer：非法二级
    /// heading，携 source 原行/field/example，REQ-SG-02 第414行场景）；
    /// 其余分支一律 `None`——缺事实不猜、line=0 不伪行，严禁从
    /// code/message 反推。旧 wire 记录只能折叠为 Unknown。
    pub kind: Option<crate::product::planning_failure::PlanningFailureKind>,
    /// REQ-CRP-02：lowering 侧信道 typed fault（独立于 wire `kind`）——
    /// 只有机器事实完整的构造点显式写 `Some`（producer：Plan Intent 分支
    /// 等价解析 fail-closed 与必填缺席），严禁从 code/message 反推。
    pub lowering_fault: Option<LoweringFault>,
    /// 工作包 2.6（MODIFIED REQ-WSC-09）：IR 侧信道 typed 载荷——四码
    /// producer（contract 交叉引用）在构造点显式写 `Some`；
    /// `source_line` 由 `attach_source_lines` 经 AST span 补全，无原行
    /// 保持 `None`，严禁从 code/message 反推。
    pub ir_fault: Option<super::ir_fault::IrFault>,
}

/// REQ-CRP-02：lowering 阶段的独立 typed fault——不进 wire 枚举
/// `PlanningFailureKind`、无 serde，仅作为 `CompilerDiagnostic` 的侧信道
/// payload；只在机器事实完整的构造点显式写 `Some`（缺事实不猜、无行不
/// 伪行），严禁从 code/message 反推。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoweringFault {
    /// Plan Intent 分支把 target 值填错字段（如 single_repository 的值填
    /// 在 `intent_target_logical_repo`）且等价解析 fail-closed（REQ-CRP-03）。
    BranchKeyMismatch {
        /// `intent_target_kind` 原值（如 `single_repository`）。
        kind: &'static str,
        /// 分支期望的字段名（如 `intent_target_repo`）。
        expected_field: &'static str,
        /// 值实际所在字段名（如 `intent_target_logical_repo`）。
        actual_field: &'static str,
        /// 模型实际写的原值。
        value: String,
        /// 值所在 1-based source 行（禁止 0 伪行）。
        line: usize,
        /// 修复示例形态。
        expected_shape: &'static str,
    },
    /// 声明在场时必填字段缺席（不从会话事实臆造补值）。
    RequiredFieldAbsent {
        /// 缺席字段名。
        field: &'static str,
        /// 修复示例形态。
        expected_shape: &'static str,
    },
}

/// options×items 预检（三族）、AC 路径×基线树核对（F-56）与 C1 Task 5 意图
/// 合同（未声明/不能执行）的共享修复路径文案口径。与
/// `work_item_split_validator` 的 finding 消息、`workspace_engine` 的机械
/// verdict 适配、SC author prompt 教学逐字同源（REQ-WSC-06 口径一致纪律）。
pub const PREFLIGHT_FINDING_CODES: &[&str] = &[
    "integration_work_item_required",
    "e2e_work_item_required",
    "frontend_backend_split_required",
    "acceptance_path_not_in_baseline",
    // C1 Task 5（REQ-C1-PLAN-01）：意图合同两类诊断同属 preflight 族——
    // Error 保留在报告里经既有修订轮回灌，不硬失败、不清既有 finding。
    "intent_undeclared",
    "intent_unexecutable",
    // bug45/t25（P1 计划自洽）：检查目标×本项 forbidden 写域冲突——Error
    // 留在报告里经既有修订轮回灌，不硬失败、不清既有 finding。
    "work_item_check_target_forbidden",
    // REQ-WSC-10：写范围×被授权仓归属——多仓（LC 聚合根）会话的 exclusive
    // 写面落在被授权成员仓 checkout 之外（聚合根非成员区/其他成员 checkout）
    // ——Error 留在报告里经既有修订轮回灌，不硬失败、不清既有 finding。
    "write_scope_outside_bound_repository",
];

/// 写范围×被授权仓归属（REQ-WSC-10，owner 2026-10-30 真机现场）的共享
/// 修复路径文案：把写范围改写为落在被授权成员仓 checkout 内的聚合根相对
/// 路径（成员目录前缀形式）；确需写其他路径的，移交给拥有该路径的 work
/// item 或调整归属声明，不得以扩大写域或删除声明绕过。与 finding 消息、
/// plan_preflight 的 required_action 适配（REQ-WSC-06）逐字同源。
pub const WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY_REPAIR_ACTION: &str = "把该写范围改写为落在本 work item 被授权成员仓 checkout 内的聚合根相对路径（带成员目录前缀，如 busi/src/...）；若交付确需写聚合根非成员区或其他成员仓的路径，把该部分移交给拥有对应成员仓的 work item 并声明依赖，或修正本项归属声明，不得以扩大写域或删除写范围声明绕过";
/// 检查目标×本项 forbidden 写域冲突（bug45/t25，P1 计划自洽）的共享修复
/// 路径文案：删除/改写该检查、改引域外路径、或移交给允许写该路径的工作
/// 项；不得以扩大写域或删除 forbidden 声明绕过。与 finding 消息、
/// plan_preflight 的 required_action 适配（REQ-WSC-06）逐字同源；SC author
/// prompt 质量预算已提额到顶，不内联本文案（教学由修订轮回灌承载）。
pub const WORK_ITEM_CHECK_TARGET_FORBIDDEN_REPAIR_ACTION: &str = "删除或改写引用该 forbidden 路径的验收标准/验证检查，或将检查目标改为本项 forbidden 写域之外的既有路径；若该检查确需覆盖此文件，把它移交给允许写该路径的 work item 并声明依赖，不得以扩大本项写域或删除 forbidden 声明绕过";

/// AC 路径×基线树核对（REQ-WSC-02 场景 13，F-56）的三条修复建议共享文案：
/// 删除该 AC / 改用基线内路径 / 将新建文件先提交入基线或改引基线内既有
/// 脚本（不以写范围声明豁免）。与 finding 消息、SC author prompt 教学
/// （REQ-WSC-06）逐字同源。
pub const ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION: &str = "删除引用基线外路径的验收标准（AC），或改用 plan 基线树内存在的既有路径；若验收依赖本次交付新建的文件，先将其作为独立提交纳入基线，或改为引用基线内已有的验证脚本/既有文件（不得引用尚未提交的路径）";

/// 将编译后的 IR 投影为既有机械 validator 输入时由外层注入的已确认上下文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCandidateValidationContext<'a> {
    pub project_id: &'a str,
    pub issue_id: &'a str,
    pub plan_id: &'a str,
    pub source_story_spec_ids: &'a [String],
    pub source_design_spec_ids: &'a [String],
    pub repository_profile: Option<&'a RepositoryProfile>,
    /// 创建计划时落库的存储 options（F-51）：候选校验 MUST 消费该事实，
    /// SHALL NOT 从 IR items 反推（反推使三族预检结构性不可触发）。
    pub plan_options: &'a IssueWorkItemPlanOptions,
    /// plan 基线树（worktree fork base 的仓库相对路径集合，F-56）。None 表示
    /// 基线不可用：AC 路径核对不触发（与现状一致），三族 options 预检不受影响。
    pub baseline_tree: Option<&'a std::collections::BTreeSet<String>>,
    /// C1 Task 5（REQ-C1-PLAN-01）：issue 下 durable 既有 work item id 集
    ///（existing 意图的授权解析面；空集＝无可引用的既有目标）。
    pub existing_work_item_ids: &'a [String],
    /// C1 Task 5（REQ-C1-PLAN-01）：enrollment 绑定 target（仅 enrolled
    /// 会话提供；None＝非 enrolled/Manual——intent target 等值核对不触发，
    /// 既有 fixture/Manual 路径零回归）。
    pub enrollment_target: Option<&'a crate::product::logical_codebase::EnrollmentTarget>,
    /// 批 2 R1（REQ-WIGA-01）：集合绑定的成员归属映射（logical 成员 →
    /// 物理成员仓 ID，与 lowering 的 PlanMemberTargets 同源）。仅 enrolled
    /// 集合绑定的 IR 物理 ID 核对消费；None＝非 enrolled/Manual——集合
    /// 上界核对所需的映射缺失时 fail-closed 出诊断，不以 None 吞错。
    pub member_targets: Option<&'a super::PlanMemberTargets>,
    /// REQ-WSC-10（写范围×被授权仓归属）：多仓（LC 聚合根）会话的成员
    /// checkout 前缀表——物理成员仓 ID → 该成员 checkout 相对聚合根的
    /// 目录（如 `busi`；空串＝成员 checkout 即聚合根本身）。仅 enrolled
    /// 多仓会话提供（与 member_targets 同装载条件/同源 manifest）；None＝
    /// 单仓/Manual——写范围归属核对不触发（零回归）。表在场而 item 的
    /// 被授权仓不在表内 → fail-closed 出诊断，不以跳过吞错。
    pub member_checkout_prefixes: Option<&'a std::collections::BTreeMap<String, String>>,
    pub now: &'a str,
}

/// 既有机械 validator 对单一候选 IR 的原样 findings 报告。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanCandidateMechanicalReport {
    pub source_revision_hash: String,
    pub compiler_version: String,
    pub findings: Vec<WorkItemSplitFinding>,
}

impl PlanCandidateMechanicalReport {
    /// Error finding 存在时，调用方不得发布该候选。
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|finding| {
            finding.severity == crate::product::models::WorkItemSplitFindingSeverity::Error
        })
    }
}

/// 已发布单候选 plan revision 对 immutable source/IR/report 的可审计绑定。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanCandidatePublicationProvenance {
    pub id: String,
    pub plan_id: String,
    pub plan_revision_id: String,
    pub source_revision_ref: String,
    pub plan_candidate_ir_ref: String,
    pub mechanical_report_ref: String,
    pub source_revision_hash: String,
    pub compiler_version: String,
    pub published_at: String,
    pub content_hash: String,
}
