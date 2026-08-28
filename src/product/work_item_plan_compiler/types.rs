//! `work-item-plan.md` 的语法树和公开诊断形状。
//!
//! 这些类型只描述 compiler 的稳定边界。parser 构造带行号的 AST，lowering 由后续
//! 任务实现；本模块本身不读取 source，也不生成诊断。

use crate::product::models::{RepositoryProfile, WorkItemSplitFinding};

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
}

/// 将编译后的 IR 投影为既有机械 validator 输入时由外层注入的已确认上下文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCandidateValidationContext<'a> {
    pub project_id: &'a str,
    pub issue_id: &'a str,
    pub plan_id: &'a str,
    pub source_story_spec_ids: &'a [String],
    pub source_design_spec_ids: &'a [String],
    pub repository_profile: Option<&'a RepositoryProfile>,
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
