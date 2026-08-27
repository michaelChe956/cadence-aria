//! `work-item-plan.md` 的语法树和公开诊断形状。
//!
//! 这些类型只描述 compiler 的稳定边界。AST 的构造、行号保留、解析和 lowering
//! 分别由后续任务实现；本模块不读取 source，也不生成诊断。

use std::collections::BTreeMap;

/// 一个 work item 的结构化 markdown 内容。
///
/// `sections` 的 key 使用 [`super::grammar::STRUCTURED_SECTIONS`] 或
/// [`super::grammar::FREE_TEXT_SECTIONS`] 中的 section 名称；value 保留该 section
/// 下的原始结构化行，便于后续 parser 在不改变语法契约的情况下补充 span 信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanItemAst {
    /// heading 中的 `WI-<digits>` 标识符。
    pub id: String,
    /// 按 section 名称保存的原始行。
    pub sections: BTreeMap<String, Vec<String>>,
}

/// `work-item-plan.md` 的文档级 AST 容器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanAst {
    /// 文档中按出现顺序排列的 work item。
    pub items: Vec<WorkItemPlanItemAst>,
    /// 文档或项目尾部 `Notes` section 的自由文本。
    pub notes: Vec<String>,
    /// 文档或项目尾部 `Rationale` section 的自由文本。
    pub rationale: Vec<String>,
}

/// compiler 对 source grammar 或 lowering 输入返回的稳定诊断形状。
///
/// 本任务只定义公开字段，不负责填写真实的行号、字段路径或修复示例；这些
/// 值必须由后续 parser/linter 根据实际输入生成。
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
