//! 生产调用方的 artifact 候选选择接线（change `artifact-candidate-selection` Task 3）。
//!
//! 候选枚举与 workspace-aware 唯一 gate-passing 选择由
//! `artifact_extraction::scan_top_level_fenced_candidates` 与
//! `artifact_constraints::select_workspace_artifact` 提供；本模块只做生产侧适配：
//! 带 workspace type 的统一入口、唯一性判定、唯一通过时取出候选正文，以及失败摘要
//! 需要的逐候选上下文（REQ-ACS-01：零通过保留全部既有阻断原因并附候选位置上下文）。

use super::*;

/// 生产侧统一选择入口：provider 原文 + 当前 workspace type → 候选选择结果。
///
/// 全部生产调用方（provider drive / artifact retry / choice 检测 / lifecycle 恢复 /
/// session reload 映射 / coding context）经本入口取值，不再直调旧「首开—末闭」抽取。
pub(crate) fn workspace_artifact_selection(
    full_output: &str,
    workspace_type: &WorkspaceType,
) -> CandidateSelection {
    select_workspace_artifact(full_output, workspace_type.clone())
}

/// 恰好一个候选通过 gate（`SelectionVerdict::Unique` 的另一面）。
pub(crate) fn selection_is_unique(selection: &CandidateSelection) -> bool {
    matches!(selection.verdict, SelectionVerdict::Unique)
}

/// 唯一通过时取出该候选正文；零通过/歧义一律 `None`（fail-closed，不回落旧区间）。
///
/// 正文按既有 `extract_artifact_content` 的 `.trim()` 契约去首尾空白——产物 payload、
/// version store 去重与 UI 展示均以此为输入，形态不变。
pub(crate) fn selected_workspace_artifact_markdown(
    full_output: &str,
    workspace_type: &WorkspaceType,
) -> Option<String> {
    selected_artifact_markdown(&workspace_artifact_selection(full_output, workspace_type))
}

/// 从已有 selection 取产物正文（`.trim()` 契约同上）；非 `Unique` 时 `None`。
pub(crate) fn selected_artifact_markdown(selection: &CandidateSelection) -> Option<String> {
    selection
        .selected_markdown
        .as_deref()
        .map(|markdown| markdown.trim().to_string())
}

/// selection 结论的稳定标签（与 REQ-ACS-02 诊断 JSON 的 `selection` 字段同词表）。
pub(crate) fn selection_verdict_label(verdict: SelectionVerdict) -> &'static str {
    match verdict {
        SelectionVerdict::Unique => "unique",
        SelectionVerdict::NoPassing => "none",
        SelectionVerdict::Ambiguous => "ambiguous",
    }
}

/// 零通过/歧义时的失败原因：逐候选保留既有 gate 阻断原因，并按 REQ-ACS-01 附候选
/// 位置上下文（行号、字节范围、正文 hash）；末条给出 candidate_count /
/// passing_count / selection 结论，以及 provider 原始输出的字符数与 hash——调查者据此
/// 无需复制完整原文即可定位首个被拒候选、并核对 durable session record。
pub(crate) fn selection_failure_reasons(selection: &CandidateSelection) -> Vec<String> {
    let passing_count = selection
        .candidates
        .iter()
        .filter(|candidate| candidate.passed)
        .count();
    let mut reasons = Vec::with_capacity(selection.candidates.len() + 1);
    for (index, candidate) in selection.candidates.iter().enumerate() {
        let location = format!(
            "候选 #{}（行 {}-{}，字节 {}-{}，sha256 {}）",
            index + 1,
            candidate.opening_line,
            candidate.closing_line,
            candidate.opening_byte,
            candidate.closing_byte_end,
            hash_prefix(&candidate.sha256),
        );
        if candidate.passed {
            // 歧义（多通过）时必须能看出「哪些候选其实是合法的」。
            reasons.push(format!("{location} 通过 gate"));
        } else {
            reasons.push(format!(
                "{location} 未通过 gate：{}",
                candidate.blocking_reasons.join("、")
            ));
        }
    }
    let origin = if selection.used_legacy_fallback {
        "legacy 单候选回退"
    } else {
        "fenced 候选枚举"
    };
    reasons.push(format!(
        "candidate_count={}，passing_count={}，selection={}（来源={origin}，原始输出 {} 字符，sha256 {}）",
        selection.candidates.len(),
        passing_count,
        selection_verdict_label(selection.verdict),
        selection.raw_output_chars,
        hash_prefix(&selection.raw_output_sha256),
    ));
    reasons
}

/// hash 的短前缀（失败摘要用；REQ-ACS-02 诊断事件带完整 hex）。
fn hash_prefix(hash: &str) -> &str {
    // sha256 hex 恒为 ASCII，`get` 兜底以防非 ASCII 输入触发切片 panic。
    hash.get(..8).unwrap_or(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fenced(markdown: &str) -> String {
        format!("```artifact\n{markdown}```\n")
    }

    fn compliant_story() -> String {
        "# Story Spec\n\n\
         ## 范围\n来源 source id: Issue issue_0001；覆盖 provider 检查。\n\n\
         ## 用户故事\n作为用户，我希望 provider 状态可见。\n\n\
         ## 功能需求\n- [REQ-001] 系统支持 provider 检查。\n\n\
         ## 成功标准\n- [AC-001] 用户能看到 provider 状态。\n\n\
         ## 待确认项\n无。\n\n\
         ## 非功能需求\n无。\n"
            .to_string()
    }

    #[test]
    fn selection_failure_reasons_keep_per_candidate_context_and_counts() {
        // 零通过样本：示意块（缺 heading）+ 一个仅有一级标题的候选块。
        let no_passing = "```artifact\n# 示意标题\n```\n\n```artifact\n# Story Spec\n```\n";
        let selection = workspace_artifact_selection(no_passing, &WorkspaceType::Story);
        assert_eq!(selection.verdict, SelectionVerdict::NoPassing);

        let reasons = selection_failure_reasons(&selection);
        assert_eq!(reasons.len(), 3, "两个候选 + 一条汇总：{reasons:?}");
        assert!(reasons[0].contains("候选 #1（行 1-3，"), "{reasons:?}");
        assert!(reasons[0].contains("未通过 gate"), "{reasons:?}");
        assert!(reasons[1].contains("候选 #2（行 5-7，"), "{reasons:?}");
        assert!(reasons[1].contains("字节 "), "{reasons:?}");
        assert!(reasons[1].contains("sha256 "), "{reasons:?}");
        // 汇总：candidate_count / passing_count / selection 与原始输出身份。
        let summary = reasons.last().expect("summary reason");
        assert!(summary.contains("candidate_count=2"), "{summary}");
        assert!(summary.contains("passing_count=0"), "{summary}");
        assert!(summary.contains("selection=none"), "{summary}");
        assert!(summary.contains("来源=fenced 候选枚举"), "{summary}");
        assert!(summary.contains("原始输出"), "{summary}");
    }

    #[test]
    fn selection_failure_reasons_report_ambiguous_candidates_as_passing() {
        let output = format!(
            "{}\n{}",
            fenced(&compliant_story()),
            fenced(&compliant_story())
        );
        let selection = workspace_artifact_selection(&output, &WorkspaceType::Story);

        assert_eq!(selection.verdict, SelectionVerdict::Ambiguous);
        let reasons = selection_failure_reasons(&selection);
        assert_eq!(reasons.len(), 3, "{reasons:?}");
        assert!(reasons[0].contains("通过 gate"), "{reasons:?}");
        assert!(reasons[1].contains("通过 gate"), "{reasons:?}");
        let summary = reasons.last().expect("summary reason");
        assert!(summary.contains("candidate_count=2"), "{summary}");
        assert!(summary.contains("passing_count=2"), "{summary}");
        assert!(summary.contains("selection=ambiguous"), "{summary}");
    }

    #[test]
    fn selection_failure_reasons_mark_legacy_fallback_origin() {
        let selection = workspace_artifact_selection("# Story Spec\n", &WorkspaceType::Story);

        assert!(selection.used_legacy_fallback);
        let summary = selection_failure_reasons(&selection).pop().unwrap();
        assert!(summary.contains("来源=legacy 单候选回退"), "{summary}");
    }

    #[test]
    fn selected_markdown_is_none_outside_unique_verdict() {
        let ambiguous = format!(
            "{}\n{}",
            fenced(&compliant_story()),
            fenced(&compliant_story())
        );
        assert_eq!(
            selected_workspace_artifact_markdown(&ambiguous, &WorkspaceType::Story),
            None
        );
        assert!(!selection_is_unique(&workspace_artifact_selection(
            &ambiguous,
            &WorkspaceType::Story
        )));
        // Unique 时按既有 `.trim()` 契约返回候选正文（无首尾空白）。
        let unique =
            workspace_artifact_selection(&fenced(&compliant_story()), &WorkspaceType::Story);
        assert_eq!(
            selected_artifact_markdown(&unique).as_deref(),
            Some(compliant_story().trim()),
        );
    }
}
