//! SC 作者交付进入 compiler 前的**确定性净化**面。
//!
//! 从 `single_candidate.rs` 拆出，独立承载前言修剪（provider 在 markdown
//! 文档标题前输出的前言）。语义与既有 parser 的失败语义保持一致：找不到固定
//! 文档标题时**原样返回**，避免把缺少标题的输出静默转换成另一种错误。

use crate::product::work_item_plan_compiler::grammar;

/// 丢弃 provider 在 markdown 文档标题前输出的前言，保留既有 parser 的失败语义。
///
/// 定位首个固定文档标题的字节偏移并从该处修剪；找不到标题时原样返回，避免把
/// 缺少标题的输出静默转换成另一种错误。
pub(crate) fn trim_provider_preamble(source: &str) -> &str {
    let document_heading = format!("{}\n", grammar::DOCUMENT_HEADING);
    source
        .find(&document_heading)
        .map(|offset| &source[offset..])
        .unwrap_or(source)
}
