//! 巡检清单渲染与 findings 摘要令牌（REQ-DRG-11 / 22）。
//!
//! 本模块消费 [`crate::product::dangling_reference::patrol::PatrolReport`]，
//! 把三栏结果渲染成 Markdown 清单并计算销账闸门用的摘要令牌。
//! 令牌口径逐字满足 REQ-DRG-22：只对 findings 的规范化序列化取摘要，
//! 不含巡检时间、令牌行自身、不可判定与未归类条目。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::product::dangling_reference::patrol::{
    PATROL_ACTION_DELETE, PATROL_ACTION_DELETE_GROUP, PatrolFinding, PatrolReport,
};
use crate::product::json_store::ProductStoreError;
use crate::product::reviewer_snapshot::sha256_hex;

/// findings 的规范化序列化（REQ-DRG-22）：按记录路径升序（同记录路径再按
/// 引用字段、目标 id 升序），每条序列化为一行紧凑 JSON（键序固定为七字段），
/// 行间以 `\n` 连接、末尾不加换行。MUST NOT 含巡检时间、令牌行、不可判定
/// 条目、未归类条目或清单说明文字。
pub(crate) fn normalized_findings(findings: &[PatrolFinding]) -> String {
    sorted_findings(findings)
        .iter()
        .map(|finding| finding_json_line(finding))
        .collect::<Vec<String>>()
        .join("\n")
}

/// findings 集合的摘要令牌：`sha256:<十六进制摘要>`（长度恒为 `sha256:` + 64）。
pub(crate) fn findings_token(findings: &[PatrolFinding]) -> String {
    format!(
        "sha256:{}",
        sha256_hex(normalized_findings(findings).as_bytes())
    )
}

/// 巡检清单文件名（REQ-DRG-11 落点：`cadence/analysis-docs/` 下按运行日命名）。
pub(crate) fn report_file_name(run_day: &str) -> String {
    format!("{run_day}_分析报告_悬空引用巡检清单_v1.0.md")
}

/// 把巡检报告渲染成 Markdown 清单（技术方案 §5.1 的五节形态）。
///
/// 五节分工（REQ-DRG-11 / 23）：「2. 明细」只收录可删条目；「3. 只报不删
/// 残留」收录命中悬空但按规则不可删的条目；「4. 不可判定」只收录读失败 /
/// 数据源不可用的条目；「5. 未归类项」只收录判据表外记录——三节 MUST NOT
/// 混列。头部说明行（含巡检时间与令牌行）属清单说明文字，不参与令牌摘要。
pub(crate) fn render_markdown_report(
    report: &PatrolReport,
    token: &str,
    scanned_at: &str,
) -> String {
    let mut markdown = String::new();

    markdown.push_str("# 分析报告：悬空引用巡检清单 v1.0\n");
    markdown.push('\n');
    markdown.push_str(&format!("- 巡检时间：{scanned_at}（不参与令牌摘要）\n"));
    markdown.push_str(
        "- 巡检范围：projects/**/issues/**、reviewer-windows/**、reviewer-snapshots/**、state/entity-identity/**、LC scope 目录\n",
    );
    markdown.push_str("- 模式：dry-run（未执行任何删除）\n");
    markdown.push_str(&format!(
        "- 清单摘要令牌：{token}（= findings 规范化序列化的摘要；开启自动清理时须原样提供）\n"
    ));

    render_summary_section(report, &mut markdown);
    render_detail_section(report, &mut markdown);
    render_residual_section(report, &mut markdown);
    render_undetermined_section(report, &mut markdown);
    render_unclassified_section(report, &mut markdown);
    render_covered_types_section(report, &mut markdown);

    markdown
}

/// 落盘清单：创建父目录并写文件（父目录 = `<workspace>/cadence/analysis-docs/`）。
pub(crate) fn write_markdown_report(path: &Path, content: &str) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(path, content)
        .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))
}

/// 按规范化次序（记录路径 → 引用字段 → 目标 id 升序）借用全部 findings。
fn sorted_findings(findings: &[PatrolFinding]) -> Vec<&PatrolFinding> {
    let mut sorted: Vec<&PatrolFinding> = findings.iter().collect();
    sorted.sort_by(|a, b| {
        (&a.record_path, &a.reference_field, &a.target_id).cmp(&(
            &b.record_path,
            &b.reference_field,
            &b.target_id,
        ))
    });
    sorted
}

/// 单条 finding 的一行紧凑 JSON（键序固定：REQ-DRG-22 的七字段次序）。
fn finding_json_line(finding: &PatrolFinding) -> String {
    let fields = [
        ("record_path", &finding.record_path),
        ("record_type", &finding.record_type),
        ("reference_field", &finding.reference_field),
        ("target_kind", &finding.target_kind),
        ("target_id", &finding.target_id),
        ("evidence_level", &finding.evidence_level),
        ("proposed_action", &finding.proposed_action),
    ];
    let body = fields
        .iter()
        .map(|(key, value)| format!("\"{key}\":{}", json_string_value(value)))
        .collect::<Vec<String>>()
        .join(",");
    format!("{{{body}}}")
}

/// 用既有 `serde_json` 把字符串值序列化成带引号与转义的 JSON 字符串。
fn json_string_value(value: &str) -> String {
    serde_json::to_string(value).expect("字符串值序列化不会失败")
}

/// 建议动作是否属于可删档（两个删除动作常量之一）。
fn is_deletable_action(proposed_action: &str) -> bool {
    proposed_action == PATROL_ACTION_DELETE || proposed_action == PATROL_ACTION_DELETE_GROUP
}

/// 「1. 汇总」：按记录类型聚合 findings（悬空条目数只统计 findings 这一栏），
/// 不可判定与未归类各占独立一行、不折入悬空计数。
fn render_summary_section(report: &PatrolReport, markdown: &mut String) {
    #[derive(Default)]
    struct TypeTally<'a> {
        total: usize,
        deletable: usize,
        report_only: usize,
        actions: BTreeSet<&'a str>,
        evidences: BTreeSet<&'a str>,
    }

    markdown.push_str("\n## 1. 汇总\n\n");
    markdown
        .push_str("| 记录类型 | 悬空条目数 | 其中可删 | 其中只报不删 | 建议动作 | 已删证据 |\n");
    markdown.push_str("|---|---|---|---|---|---|\n");

    let mut by_type: BTreeMap<&str, TypeTally<'_>> = BTreeMap::new();
    for finding in &report.findings {
        let tally = by_type.entry(finding.record_type.as_str()).or_default();
        tally.total += 1;
        if is_deletable_action(&finding.proposed_action) {
            tally.deletable += 1;
        } else {
            tally.report_only += 1;
        }
        tally.actions.insert(finding.proposed_action.as_str());
        tally.evidences.insert(finding.evidence_level.as_str());
    }
    for (record_type, tally) in &by_type {
        markdown.push_str(&format!(
            "| {record_type} | {} | {} | {} | {} | {} |\n",
            tally.total,
            tally.deletable,
            tally.report_only,
            tally
                .actions
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(" / "),
            tally
                .evidences
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(" / "),
        ));
    }
    markdown.push_str(&format!(
        "| 未归类（{} 条，只报不删） | — | — | — | 只报不删 | — |\n",
        report.unclassified.len()
    ));
    markdown.push_str(&format!(
        "| 不可判定（{} 条，未判定） | — | — | — | 不计悬空、不计归零 | — |\n",
        report.undetermined.len()
    ));
    markdown.push_str(
        "\n> **归零口径**：只有「可删」类参与归零断言；「只报不删」与「不可判定」都不算归零；不可判定与未归类条目不进令牌的发现集合（REQ-DRG-17 / 22 / 23）。\n",
    );
}

/// 「2. 明细」：只收录可删条目，每行七字段齐备，按规范化次序排列。
fn render_detail_section(report: &PatrolReport, markdown: &mut String) {
    markdown.push_str("\n## 2. 明细\n\n");
    let deletable: Vec<&PatrolFinding> = sorted_findings(&report.findings)
        .into_iter()
        .filter(|finding| is_deletable_action(&finding.proposed_action))
        .collect();
    if deletable.is_empty() {
        markdown.push_str("（无）\n");
        return;
    }
    markdown.push_str(
        "| 记录路径 | 记录类型 | 引用字段 | 引用目标类型 | 引用目标 id | 已删证据 | 建议动作 |\n",
    );
    markdown.push_str("|---|---|---|---|---|---|---|\n");
    for finding in deletable {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            finding.record_path,
            finding.record_type,
            finding.reference_field,
            finding.target_kind,
            finding.target_id,
            finding.evidence_level,
            finding.proposed_action,
        ));
    }
}

/// 「3. 只报不删残留」：收录 findings 中建议动作非删除档的条目（不参与归零断言）。
fn render_residual_section(report: &PatrolReport, markdown: &mut String) {
    markdown.push_str("\n## 3. 只报不删残留（不参与归零断言）\n\n");
    let residual: Vec<&PatrolFinding> = sorted_findings(&report.findings)
        .into_iter()
        .filter(|finding| !is_deletable_action(&finding.proposed_action))
        .collect();
    if residual.is_empty() {
        markdown.push_str("（无）\n");
        return;
    }
    markdown.push_str("| 记录路径 | 记录类型 | 引用字段 | 引用目标 id | 不删原因 |\n");
    markdown.push_str("|---|---|---|---|---|\n");
    for finding in residual {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {}（证据：{}） |\n",
            finding.record_path,
            finding.record_type,
            finding.reference_field,
            finding.target_id,
            finding.proposed_action,
            finding.evidence_level,
        ));
    }
}

/// 「4. 不可判定」：只收录 undetermined（读失败 / 数据源不可用；不计悬空、不计归零）。
fn render_undetermined_section(report: &PatrolReport, markdown: &mut String) {
    markdown.push_str("\n## 4. 不可判定（读失败，未判定）\n\n");
    if report.undetermined.is_empty() {
        markdown.push_str("（无）\n");
        return;
    }
    markdown.push_str("| 记录路径 | 引用字段 | 失败原因 |\n");
    markdown.push_str("|---|---|---|\n");
    for entry in &report.undetermined {
        markdown.push_str(&format!(
            "| {} | {} | {} |\n",
            entry.record_path, entry.reference_field, entry.reason
        ));
    }
}

/// 「5. 未归类项」：只收录 unclassified（判据表外记录，只报不删）。
fn render_unclassified_section(report: &PatrolReport, markdown: &mut String) {
    markdown.push_str("\n## 5. 未归类项（只报不删）\n\n");
    if report.unclassified.is_empty() {
        markdown.push_str("（无）\n");
        return;
    }
    markdown.push_str("| 记录路径 | 记录类型 |\n");
    markdown.push_str("|---|---|\n");
    for entry in &report.unclassified {
        markdown.push_str(&format!(
            "| {} | {} |\n",
            entry.record_path, entry.record_type
        ));
    }
}

/// 附录：本次巡检覆盖的记录类型清单（U7 / R3）。
fn render_covered_types_section(report: &PatrolReport, markdown: &mut String) {
    markdown.push_str("\n## 附：本次巡检覆盖的记录类型\n\n");
    if report.covered_record_types.is_empty() {
        markdown.push_str("（无）\n");
        return;
    }
    for record_type in &report.covered_record_types {
        markdown.push_str(&format!("- {record_type}\n"));
    }
}
#[cfg(test)]
mod report_tests {
    use super::*;
    use crate::product::dangling_reference::patrol::{
        PATROL_ACTION_DELETE, PATROL_ACTION_REPORT_ONLY, PatrolFinding, PatrolReport,
        UnclassifiedEntry, UndeterminedEntry,
    };

    fn finding(path: &str, field: &str, action: &str, evidence: &str) -> PatrolFinding {
        PatrolFinding {
            record_path: path.to_string(),
            record_type: "automation_plan_intent".to_string(),
            reference_field: field.to_string(),
            target_kind: "story_spec".to_string(),
            target_id: "story_spec_0005".to_string(),
            evidence_level: evidence.to_string(),
            proposed_action: action.to_string(),
        }
    }

    #[test]
    fn dangling_report_token_is_stable_and_excludes_undetermined() {
        let findings = vec![
            finding(
                "projects/p1/issues/i1/automation-plan-intent.json",
                "source.stories[].id",
                PATROL_ACTION_DELETE,
                "L1",
            ),
            finding(
                "projects/p1/issues/i1/automation-plan-intent.json",
                "session_id",
                PATROL_ACTION_REPORT_ONLY,
                "未确证",
            ),
        ];
        let first = normalized_findings(&findings);
        let mut reversed = findings.clone();
        reversed.reverse();
        assert_eq!(first, normalized_findings(&reversed), "排序必须规范化");
        assert_eq!(findings_token(&findings), findings_token(&reversed));

        let report = PatrolReport {
            findings: findings.clone(),
            undetermined: vec![UndeterminedEntry {
                record_path: "reviewer-windows/windows/win_bad.json".to_string(),
                reference_field: "attempt_id".to_string(),
                reason: "json parse failed".to_string(),
            }],
            unclassified: vec![UnclassifiedEntry {
                record_path: "state/other/record.json".to_string(),
                record_type: "unclassified".to_string(),
            }],
            covered_record_types: vec!["automation_plan_intent".to_string()],
        };
        assert_eq!(
            findings_token(&report.findings),
            findings_token(&findings),
            "不可判定与未归类条目不得进入摘要"
        );
        assert!(findings_token(&findings).starts_with("sha256:"));
        assert_eq!(findings_token(&findings).len(), "sha256:".len() + 64);
    }

    #[test]
    fn dangling_report_residual_entry_changes_token() {
        let base = vec![finding(
            "projects/p1/issues/i1/a.json",
            "session_id",
            PATROL_ACTION_REPORT_ONLY,
            "未确证",
        )];
        let mut changed = base.clone();
        changed[0].evidence_level = "L1".to_string();
        assert_ne!(findings_token(&base), findings_token(&changed));
    }

    #[test]
    fn dangling_report_sections_are_disjoint() {
        let report = PatrolReport {
            findings: vec![finding(
                "projects/p1/issues/i1/a.json",
                "session_id",
                PATROL_ACTION_REPORT_ONLY,
                "未确证",
            )],
            undetermined: vec![UndeterminedEntry {
                record_path: "reviewer-windows/windows/win_bad.json".to_string(),
                reference_field: "attempt_id".to_string(),
                reason: "json parse failed".to_string(),
            }],
            unclassified: vec![UnclassifiedEntry {
                record_path: "state/other/record.json".to_string(),
                record_type: "unclassified".to_string(),
            }],
            covered_record_types: vec!["automation_plan_intent".to_string()],
        };
        let markdown = render_markdown_report(&report, "sha256:abc", "2026-10-28T10:00:00Z");
        assert!(markdown.contains("## 1. 汇总"), "{markdown}");
        assert!(markdown.contains("## 2. 明细"), "{markdown}");
        assert!(markdown.contains("## 3. 只报不删残留"), "{markdown}");
        assert!(markdown.contains("## 4. 不可判定"), "{markdown}");
        assert!(markdown.contains("## 5. 未归类项"), "{markdown}");
        assert!(markdown.contains("sha256:abc"), "{markdown}");
        assert!(markdown.contains("json parse failed"), "{markdown}");
        assert!(markdown.contains("state/other/record.json"), "{markdown}");
        let detail_section = markdown
            .split("## 2. 明细")
            .nth(1)
            .and_then(|rest| rest.split("## 3.").next())
            .unwrap();
        assert!(
            !detail_section.contains("json parse failed"),
            "不可判定不得进明细"
        );
        assert!(
            !detail_section.contains("state/other/record.json"),
            "未归类不得进明细"
        );
        assert!(
            !detail_section.contains("projects/p1/issues/i1/a.json"),
            "只报不删残留不得进带删除建议的明细"
        );
    }

    /// 补充口径证据（REQ-DRG-22「令牌不随巡检时间变化」）：巡检时间只出现在
    /// 清单头部说明行，改变巡检时间后同一 findings 集合的令牌必须保持不变。
    #[test]
    fn dangling_report_token_ignores_scan_time() {
        let findings = vec![finding(
            "projects/p1/issues/i1/automation-plan-intent.json",
            "source.stories[].id",
            PATROL_ACTION_DELETE,
            "L1",
        )];
        let report = PatrolReport {
            findings: findings.clone(),
            undetermined: Vec::new(),
            unclassified: Vec::new(),
            covered_record_types: vec!["automation_plan_intent".to_string()],
        };
        let first_markdown = render_markdown_report(&report, "sha256:abc", "2026-10-28T10:00:00Z");
        let second_markdown = render_markdown_report(&report, "sha256:abc", "2037-01-01T00:00:00Z");
        assert!(first_markdown.contains("2026-10-28T10:00:00Z"));
        assert!(second_markdown.contains("2037-01-01T00:00:00Z"));
        let token_first = findings_token(&findings);
        let token_second = findings_token(&report.findings);
        assert_eq!(token_first, token_second, "同一发现集合令牌恒等");
        assert_eq!(
            findings_token(&findings),
            token_second,
            "巡检时间变化不得改变令牌"
        );
        assert!(
            !normalized_findings(&findings).contains("2026-10-28"),
            "规范化序列化不得含巡检时间"
        );
        assert!(
            !normalized_findings(&findings).contains("sha256"),
            "规范化序列化不得含令牌行自身"
        );
    }

    /// 补充三栏证据（REQ-DRG-23「不可判定不计入归零口径」）：汇总中悬空条目数
    /// 只统计 findings，不可判定与未归类条目分栏呈现且不折入悬空计数。
    #[test]
    fn dangling_report_summary_keeps_undetermined_out_of_zeroing() {
        let report = PatrolReport {
            findings: vec![finding(
                "projects/p1/issues/i1/a.json",
                "session_id",
                PATROL_ACTION_REPORT_ONLY,
                "未确证",
            )],
            undetermined: vec![UndeterminedEntry {
                record_path: "reviewer-windows/windows/win_bad.json".to_string(),
                reference_field: "attempt_id".to_string(),
                reason: "json parse failed".to_string(),
            }],
            unclassified: vec![UnclassifiedEntry {
                record_path: "state/other/record.json".to_string(),
                record_type: "unclassified".to_string(),
            }],
            covered_record_types: vec!["automation_plan_intent".to_string()],
        };
        let markdown = render_markdown_report(&report, "sha256:abc", "2026-10-28T10:00:00Z");
        assert!(
            markdown.contains("| automation_plan_intent | 1 | 0 | 1 |"),
            "悬空条目数只统计 findings（1 条），不可判定/未归类不得折入：{markdown}"
        );
        assert!(
            markdown.contains("「只报不删」与「不可判定」都不算归零"),
            "汇总必须写明归零口径排除不可判定：{markdown}"
        );
        assert!(
            markdown.contains("不可判定（1 条，未判定）"),
            "不可判定须独立分栏计数：{markdown}"
        );
        assert!(
            markdown.contains("未归类（1 条，只报不删）"),
            "未归类须独立分栏计数：{markdown}"
        );
    }
}
