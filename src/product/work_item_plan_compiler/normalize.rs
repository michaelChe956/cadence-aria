//! SC 交付 markdown 的确定性结构标题归一化。
//!
//! 与前言修剪(`trim_provider_preamble`)同层:在 provider 原文进入 compiler 前,
//! 把固定词表结构标题的已知中文翻译逐字映射回规范英文,吸收 provider(如 pi)
//! 随机把结构标题翻成中文导致的 `missing_section`/`unknown_structured_key`
//! fail-closed 抖动。
//!
//! 契约边界:
//! - 仅覆盖本模块固定映射表中的结构标题行(行首 `#`/`##`/`###` 后的标题文本);
//! - 表外未知标题不猜不改,照旧交给 compiler fail-closed;
//! - 正文内容零触碰(中文 plan 语义不动,只有标题行被改写);
//! - author 与 revision 两条交付路径共用本实现(单点)。
//!
//! 映射表来源为 2026-09-03 两次现场事故的 pi 交付原件:
//! - pi-full rep1:标题带英文括号注记(`### 身份 (Identity)`);
//! - 矩阵 r1 rep2:裸中文标题(`### 身份信息`)。

use super::grammar;

/// 归一化发生时的审计诊断名(与 `preamble_trimmed` 诊断同一模式)。
pub const PLAN_HEADING_NORMALIZATION_DIAGNOSTIC: &str = "plan_heading_translations_normalized";

/// 一级文档标题的已知中文翻译;归一化目标固定为 [`grammar::DOCUMENT_HEADING`]。
const DOCUMENT_HEADING_TRANSLATIONS: [&str; 2] = ["工作项计划", "工作项计划 (Work Item Plan)"];

/// Work Item 二级标题的已知中文前缀(`## ` 之后);标题文本保持原样。
const ITEM_HEADING_PREFIX_TRANSLATION: &str = "工作项 WI-";

/// 三级结构标题的已知中文翻译 → 规范英文。
///
/// 每个规范 section 至少收录一个现场裸中文变体;同词的「裸中文」与
/// 「中文 (English)」两种抖动形态成对收录。顺序与
/// [`grammar::STRUCTURED_SECTIONS`] 的 canonical 顺序一致。
const SECTION_HEADING_TRANSLATIONS: [(&str, &str); 32] = [
    // Identity
    ("身份", "Identity"),
    ("身份信息", "Identity"),
    ("身份 (Identity)", "Identity"),
    ("身份信息 (Identity)", "Identity"),
    // Goal
    ("目标", "Goal"),
    ("目标 (Goal)", "Goal"),
    // Non Goals
    ("非目标", "Non Goals"),
    ("非目标 (Non Goals)", "Non Goals"),
    // Dependencies
    ("依赖", "Dependencies"),
    ("依赖关系", "Dependencies"),
    ("依赖 (Dependencies)", "Dependencies"),
    ("依赖关系 (Dependencies)", "Dependencies"),
    // Inputs
    ("输入", "Inputs"),
    ("输入 (Inputs)", "Inputs"),
    // Outputs
    ("输出", "Outputs"),
    ("输出 (Outputs)", "Outputs"),
    // Tasks
    ("任务", "Tasks"),
    ("任务 (Tasks)", "Tasks"),
    // Write Policy
    ("写入策略", "Write Policy"),
    ("编写策略", "Write Policy"),
    ("写入策略 (Write Policy)", "Write Policy"),
    ("编写策略 (Write Policy)", "Write Policy"),
    // Acceptance Criteria
    ("验收标准", "Acceptance Criteria"),
    ("验收标准 (Acceptance Criteria)", "Acceptance Criteria"),
    // Verification
    ("验证", "Verification"),
    ("验证 (Verification)", "Verification"),
    // Handoff Schema
    ("交接模式", "Handoff Schema"),
    ("交接模式 (Handoff Schema)", "Handoff Schema"),
    // Blockers
    ("阻塞项", "Blockers"),
    ("阻塞项 (Blockers)", "Blockers"),
    // Traceability
    ("可追溯性", "Traceability"),
    ("可追溯性 (Traceability)", "Traceability"),
];

/// 归一化结果:净化后的交付源与审计报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedPlanSource {
    /// 结构标题归一化后的完整 markdown 源。
    pub source: String,
    /// 被改写的结构标题行数;`0` 表示原文未被触碰。
    pub normalized_heading_lines: usize,
}

/// 把固定词表结构标题的已知中文翻译逐字映射回规范英文。
///
/// 逐行扫描:仅当某行是「标题行」且其标题文本命中固定映射表时才重写该行;
/// 其余所有行(含正文中文)逐字节保留。表外未知标题原样返回,由 compiler
/// 按既有语法契约 fail-closed。
pub fn normalize_structural_headings(source: &str) -> NormalizedPlanSource {
    let mut normalized_heading_lines = 0usize;
    let mut lines = Vec::new();
    for line in source.split('\n') {
        match rewrite_heading_line(line) {
            Some(rewritten) => {
                normalized_heading_lines += 1;
                lines.push(rewritten);
            }
            None => lines.push(line.to_string()),
        }
    }
    NormalizedPlanSource {
        source: lines.join("\n"),
        normalized_heading_lines,
    }
}

/// 重写单行结构标题;未命中固定映射表时返回 `None`(调用方保持原行)。
fn rewrite_heading_line(line: &str) -> Option<String> {
    if let Some(text) = line.strip_prefix("# ") {
        let trimmed = text.trim();
        if DOCUMENT_HEADING_TRANSLATIONS.contains(&trimmed) {
            return Some(grammar::DOCUMENT_HEADING.to_string());
        }
        return None;
    }
    if let Some(text) = line.strip_prefix("## ") {
        if let Some(rest) = text.strip_prefix(ITEM_HEADING_PREFIX_TRANSLATION) {
            return Some(format!("{}{}", grammar::ITEM_HEADING_PREFIX, rest));
        }
        return None;
    }
    if let Some(text) = line.strip_prefix("### ") {
        let trimmed = text.trim();
        let canonical = SECTION_HEADING_TRANSLATIONS
            .iter()
            .find(|(translation, _)| *translation == trimmed)
            .map(|(_, canonical)| *canonical)?;
        return Some(format!("### {canonical}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_table_only_maps_into_the_canonical_vocabulary() {
        let mut translations: Vec<&str> = SECTION_HEADING_TRANSLATIONS
            .iter()
            .map(|(translation, _)| *translation)
            .collect();
        let original_len = translations.len();
        translations.sort_unstable();
        translations.dedup();
        assert_eq!(
            translations.len(),
            original_len,
            "中文词条必须唯一,避免映射歧义"
        );
        for (translation, canonical) in SECTION_HEADING_TRANSLATIONS {
            assert_ne!(translation, canonical, "词条不得自映射");
            assert!(
                grammar::STRUCTURED_SECTIONS.contains(&canonical),
                "目标必须是规范结构 section: {canonical}"
            );
            assert!(
                !SECTION_HEADING_TRANSLATIONS
                    .iter()
                    .any(|(other, _)| *other == canonical),
                "映射目标不得同时又是别人的词条(链式映射): {canonical}"
            );
        }
        for section in grammar::STRUCTURED_SECTIONS {
            assert!(
                SECTION_HEADING_TRANSLATIONS
                    .iter()
                    .any(|(_, canonical)| *canonical == section),
                "每个规范 section 至少收录一个中文词条: {section}"
            );
        }
    }

    #[test]
    fn document_and_item_heading_translations_map_to_the_grammar_constants() {
        assert_eq!(
            rewrite_heading_line("# 工作项计划").as_deref(),
            Some(grammar::DOCUMENT_HEADING)
        );
        assert_eq!(
            rewrite_heading_line("## 工作项 WI-001: 标题").as_deref(),
            Some("## Work Item WI-001: 标题")
        );
        assert_eq!(rewrite_heading_line("# Work Item Plan"), None);
        assert_eq!(rewrite_heading_line("## Work Item WI-001: x"), None);
    }
}
