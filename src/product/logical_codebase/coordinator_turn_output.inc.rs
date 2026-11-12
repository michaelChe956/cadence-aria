// provider turn 输出的**有界保留**与**脚本报告结论**判定（从
// coordinator_provider_turn.inc.rs 拆出：该文件的行数有登记上限，见
// tests/it_core/large_file_guard.rs）。
//
// 两件事：
// ① turn 输出超预算时保留**头尾**（只留头会把收尾处的脚本报告整段截掉）；
// ② 从**工具结果原文**读 Cadence-skills 脚本的报告结论（`overall`）——
//    模型自述不可信，脚本 stdout 才是事实。

/// 向下对齐到 UTF-8 字符边界（绝不切碎多字节字符）。
fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    if index >= value.len() {
        return value.len();
    }
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Cadence-skills 脚本报告的结论（**只认工具结果里的脚本 stdout 原文**）。
///
/// 脚本报告的 JSON 含 `"overall": "success" | "failed"`；本函数在文本里找
/// 该字段并取其结论。找不到 ⇒ [`RecipeCommandReport::Absent`]（命令可能根本
/// 没执行——按失败处理，绝不猜）。
fn recipe_report_verdict(tool_output: &str) -> RecipeCommandReport {
    const FIELD: &str = "\"overall\"";
    const WINDOW: usize = 64;
    let mut verdict = RecipeCommandReport::Absent;
    let mut rest = tool_output;
    while let Some(index) = rest.find(FIELD) {
        let after = &rest[index + FIELD.len()..];
        let window = &after[..after.len().min(WINDOW)];
        if window.contains("failed") {
            verdict = RecipeCommandReport::Failed;
        } else if window.contains("success") && verdict == RecipeCommandReport::Absent {
            verdict = RecipeCommandReport::Success;
        }
        rest = &rest[index + 1..];
    }
    verdict
}

/// 合并多份脚本报告：**失败优先**（任一份报告 failed ⇒ 整轮判失败；
/// 没有失败但有成功 ⇒ 成功；都没有 ⇒ 没有报告）。
fn merge_recipe_report(
    current: RecipeCommandReport,
    next: RecipeCommandReport,
) -> RecipeCommandReport {
    match (current, next) {
        (RecipeCommandReport::Failed, _) | (_, RecipeCommandReport::Failed) => {
            RecipeCommandReport::Failed
        }
        (RecipeCommandReport::Success, _) | (_, RecipeCommandReport::Success) => {
            RecipeCommandReport::Success
        }
        _ => RecipeCommandReport::Absent,
    }
}

/// 向上对齐到 UTF-8 字符边界（绝不切碎多字节字符）。
fn ceil_char_boundary(value: &str, mut index: usize) -> usize {
    if index >= value.len() {
        return value.len();
    }
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}


/// OracleAggInitScope 红绿测共用断言(镜像 r54
/// `assert_worktree_discipline` 形态):root recipe prompt 必须携带 scope
/// 纪律段四句结构(聚合根真实绝对路径硬钉+成员仓负边界+auditor 后果+
/// zero-action 正向句)。供本文件 `provider_turn_lcg_tests` 与
/// coordinator_tests 的 `mod tests`(profile_trust 全链测试)共用。
#[cfg(test)]
pub(crate) fn assert_aggregate_scope_discipline(prompt: &str, aggregate_root: &std::path::Path) {
    assert!(
        prompt.contains("[aggregate_scope_discipline]"),
        "OracleAggInitScope:root recipe prompt 必须携带 scope 纪律段:\n{prompt}"
    );
    assert!(
        prompt.contains(&aggregate_root.display().to_string()),
        "纪律段必须钉 provider_context_root 真实绝对路径:\n{prompt}"
    );
    assert!(
        prompt.contains("你的工作区是且只是聚合根"),
        "纪律段第一句必须硬钉聚合根为唯一工作区:\n{prompt}"
    );
    assert!(
        prompt.contains("禁止在任何成员仓目录"),
        "纪律段必须携带成员仓目录负边界禁线:\n{prompt}"
    );
    assert!(
        prompt.contains("root recipe auditor 逐路径把关"),
        "纪律段必须声明 auditor 逐路径把关:\n{prompt}"
    );
    assert!(
        prompt.contains("fail-closed"),
        "纪律段必须声明越界=该命令 rejected+发布 fail-closed 后果:\n{prompt}"
    );
    assert!(
        prompt.contains("零文件变更的空完成"),
        "纪律段必须携带 zero-action 正向句:\n{prompt}"
    );
}

#[cfg(test)]
mod turn_output_tests {
    use super::BoundedOutput;

    /// turn 输出超预算时必须**保留头尾**：Cadence-skills 脚本的 JSON 报告与
    /// 失败原文都在收尾处，只留头会把最有诊断价值的事实整段截掉
    /// （本仓 2026-11-12 真机：诊断只剩 `zero_change=true`）。
    #[test]
    fn bounded_output_keeps_head_and_tail_after_budget() {
        let mut output = BoundedOutput::new();
        output.push(&"HEAD-MARKER ".repeat(64));
        output.push("TAIL-MARKER");
        let summary = output.summary();
        assert!(
            summary.contains("HEAD-MARKER"),
            "超预算后必须保留输出头部:\n{summary}"
        );
        assert!(
            summary.contains("TAIL-MARKER"),
            "超预算后必须保留输出尾部（脚本报告/失败原文所在处）:\n{summary}"
        );
    }
}
