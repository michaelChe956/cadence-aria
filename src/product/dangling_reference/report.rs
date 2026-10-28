//! 巡检清单渲染与 findings 摘要令牌（REQ-DRG-11 / 22）。
//!
//! 本模块消费 [`crate::product::dangling_reference::patrol::PatrolReport`]，
//! 把三栏结果渲染成 Markdown 清单，并计算发现集合的摘要令牌（**仅用于审计
//! 关联，不是删除凭据**——REQ-DRG-22）。
//! 令牌口径逐字满足 REQ-DRG-22：只对 findings 的规范化序列化取摘要，
//! 不含巡检时间、令牌行自身、不可判定与未归类条目。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::product::dangling_reference::patrol::{
    PATROL_ACTION_DELETE, PATROL_ACTION_DELETE_GROUP, PatrolFinding, PatrolReport,
    UndeterminedEntry,
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

/// 删除留痕台账**相对工作区根**的路径（REQ-DRG-11：清单须指向逐条删除明细）。
///
/// 清单落在 `<workspace>/cadence/analysis-docs/`，台账落在
/// `<workspace>/.aria/state/entity-deletions/dangling-reference/cleanup.jsonl`
/// （[`crate::product::app_paths::ProductAppPaths::dangling_reference_ledger_path`]
/// 的绝对形态）。此处给相对工作区根的形态，便于人按清单直接核对。
pub(crate) const CLEANUP_LEDGER_RELATIVE_PATH: &str =
    ".aria/state/entity-deletions/dangling-reference/cleanup.jsonl";

/// 清单头部的「本次模式与销账结果」（REQ-DRG-11）。
///
/// 模式行 MUST 如实反映本次运行：自动模式写明实际删除条数；销账失败写明错误
/// 原文且 MUST NOT 声称「未删除任何记录」——失败可能发生在部分删除之后，
/// 逐条真相以留痕台账为准。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CleanupReportStatus {
    /// 只报不删（dry-run 复核）：零删除、零留痕。
    ReportOnly,
    /// 自动销账成功：本次实际删除 `deleted_count` 条。
    Automatic { deleted_count: usize },
    /// 自动销账失败：`reason` 为错误原文；已发生的删除以留痕台账为准。
    AutomaticFailed { reason: String },
}

/// 销账后的「归零自证」现场（REQ-DRG-17 / 23）：**销账后**再巡检得到的自证结论。
///
/// 与「2. 明细 / 3. 只报不删残留 / 4. 不可判定」三节（**销账前**现场）分节并列，
/// MUST NOT 混列（REQ-DRG-11 的分节互斥要求）。
#[derive(Debug, Clone)]
pub(crate) struct GovernanceSelfProof {
    /// 仍未归零的可删条目（逐条；空 = 本次可删类型全部归零）。
    pub(crate) unzeroed: Vec<PatrolFinding>,
    /// 未归零可删条目按记录类型的计数（与 `unzeroed` 同口径，不另立判定）。
    pub(crate) unzeroed_by_type: BTreeMap<String, usize>,
    /// 只报不删残留条数（不计入归零）。
    pub(crate) residual_count: usize,
    /// 不可判定条目（逐条）。
    pub(crate) undetermined: Vec<UndeterminedEntry>,
    /// 销账侧给出的治理完成标记（清单渲染时仍会用三类计数复核，见
    /// [`render_self_proof_section`]）。
    pub(crate) governance_complete: bool,
}

/// 把巡检报告渲染成 Markdown 清单（技术方案 §5.1 / §5.2 的六节形态）。
///
/// 各节分工（REQ-DRG-11 / 17 / 23）：「2. 明细」只收录可删条目；「3. 只报不删
/// 残留」收录命中悬空但按规则不可删的条目；「4. 不可判定」只收录读失败 /
/// 数据源不可用的条目；「5. 未归类项」只收录判据表外记录——三节 MUST NOT
/// 混列。「6. 归零自证」写**销账后**现场（`self_proof`），与前三节分节并列。
/// 头部说明行（模式行、巡检时间与令牌行）属清单说明文字，不参与令牌摘要。
///
/// `status` = 本次模式与销账结果；模式行 MUST 如实标注（REQ-DRG-11）。
/// `self_proof` = 销账成功后取得的自证现场；销账失败时为 `None`（清单仍写出，
/// 但 MUST NOT 视为治理完成）。
pub(crate) fn render_markdown_report(
    report: &PatrolReport,
    token: &str,
    scanned_at: &str,
    status: &CleanupReportStatus,
    self_proof: Option<&GovernanceSelfProof>,
) -> String {
    let mut markdown = String::new();

    markdown.push_str("# 分析报告：悬空引用巡检清单 v1.0\n");
    markdown.push('\n');
    markdown.push_str(&format!("- 巡检时间：{scanned_at}（不参与令牌摘要）\n"));
    markdown.push_str(
        "- 巡检范围：projects/**/issues/**、reviewer-windows/**、reviewer-snapshots/**、state/entity-identity/**、LC scope 目录\n",
    );
    render_mode_line(status, &mut markdown);
    markdown.push_str(&format!(
        "- 清单摘要令牌：{token}（= findings 规范化序列化的摘要；仅用于审计关联，不是删除凭据）\n"
    ));
    markdown.push_str(&format!(
        "- 删除留痕台账：{CLEANUP_LEDGER_RELATIVE_PATH}（逐条删除明细）\n"
    ));

    render_summary_section(report, &mut markdown);
    render_detail_section(report, &mut markdown);
    render_residual_section(report, &mut markdown);
    render_undetermined_section(report, &mut markdown);
    render_unclassified_section(report, &mut markdown);
    render_self_proof_section(report, self_proof, &mut markdown);
    render_covered_types_section(report, &mut markdown);

    markdown
}

/// 模式行：如实标注本次模式与销账结果（REQ-DRG-11）。
///
/// 三种口径互斥：只报不删写明零删除；自动销账写明本次实际删除条数；销账失败
/// 写明错误原文并指向留痕台账——MUST NOT 声称「未删除任何记录」（失败可能发生
/// 在部分删除之后）。
fn render_mode_line(status: &CleanupReportStatus, markdown: &mut String) {
    match status {
        CleanupReportStatus::ReportOnly => {
            markdown.push_str("- 模式：只报不删（dry-run，未执行任何删除）\n");
        }
        CleanupReportStatus::Automatic { deleted_count } => {
            markdown.push_str(&format!(
                "- 模式：自动销账（服务启动时自动执行；本次已删除 {deleted_count} 条）\n"
            ));
        }
        CleanupReportStatus::AutomaticFailed { reason } => {
            markdown.push_str(&format!(
                "- 模式：自动销账（服务启动时自动执行）；销账失败（错误：{reason}）——失败可能发生在部分删除之后，逐条真相以删除留痕台账为准\n"
            ));
        }
    }
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

/// 「6. 归零自证（销账后现场）」：销账后再巡检的自证结论（REQ-DRG-17 / 23）。
///
/// 本节写**销账后**现场，与「2. 明细 / 3. 只报不删残留 / 4. 不可判定」三节
/// （**销账前**现场）分节并列、MUST NOT 混列。四条内容齐备：本次可删类型的
/// 归零计数、未归零可删项逐条、不可判定项逐条、治理是否完成。
///
/// 诚实口径：完成标记为真时仍用三类计数复核（未归零可删项 / 不可判定 / 只报
/// 不删残留），任一类非空即 MUST NOT 宣称治理完成。销账失败（`None`）时本节
/// 只写「未取得自证结果」，MUST NOT 视为治理完成。
fn render_self_proof_section(
    report: &PatrolReport,
    proof: Option<&GovernanceSelfProof>,
    markdown: &mut String,
) {
    markdown.push_str("\n## 6. 归零自证（销账后现场）\n\n");
    let Some(proof) = proof else {
        markdown.push_str(
            "（销账失败，未取得自证结果——MUST NOT 视为治理完成；逐条真相以删除留痕台账为准）\n",
        );
        return;
    };

    markdown.push_str("### 6.1 本次可删类型的归零计数\n\n");
    let mut types: BTreeSet<&str> = report
        .findings
        .iter()
        .filter(|finding| is_deletable_action(&finding.proposed_action))
        .map(|finding| finding.record_type.as_str())
        .collect();
    types.extend(proof.unzeroed_by_type.keys().map(String::as_str));
    if types.is_empty() {
        markdown.push_str("（本次无可删条目）\n");
    } else {
        markdown.push_str(
            "> 计数为**销账后**仍未归零的可删条目数（按 finding 逐条）；本次已归零的类型记 0；不可判定与只报不删残留不计入。\n\n",
        );
        markdown.push_str("| 记录类型 | 未归零可删条目数 |\n|---|---|\n");
        for record_type in types {
            let count = proof
                .unzeroed_by_type
                .get(record_type)
                .copied()
                .unwrap_or(0);
            markdown.push_str(&format!("| {record_type} | {count} |\n"));
        }
    }

    markdown.push_str("\n### 6.2 未归零的可删条目\n\n");
    if proof.unzeroed.is_empty() {
        markdown.push_str("（无：本次可删条目已全部归零）\n");
    } else {
        markdown.push_str(
            "| 记录路径 | 记录类型 | 引用字段 | 引用目标类型 | 引用目标 id | 已删证据 | 建议动作 |\n",
        );
        markdown.push_str("|---|---|---|---|---|---|---|\n");
        for finding in sorted_findings(&proof.unzeroed) {
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

    markdown.push_str("\n### 6.3 不可判定的条目\n\n");
    if proof.undetermined.is_empty() {
        markdown.push_str("（无）\n");
    } else {
        markdown.push_str("| 记录路径 | 引用字段 | 失败原因 |\n|---|---|---|\n");
        for entry in &proof.undetermined {
            markdown.push_str(&format!(
                "| {} | {} | {} |\n",
                entry.record_path, entry.reference_field, entry.reason
            ));
        }
    }

    markdown.push_str("\n### 6.4 治理是否完成\n\n");
    let complete = proof.governance_complete
        && proof.unzeroed.is_empty()
        && proof.undetermined.is_empty()
        && proof.residual_count == 0;
    if complete {
        markdown.push_str("- 治理完成：是（本次可删条目全部归零、无只报不删残留、无不可判定）\n");
    } else {
        markdown.push_str(&format!(
            "- 治理完成：否（未归零可删条目 {} 条、不可判定 {} 条、只报不删残留 {} 条）——MUST NOT 宣称悬空引用已治理完成\n",
            proof.unzeroed.len(),
            proof.undetermined.len(),
            proof.residual_count
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
        let markdown = render_markdown_report(
            &report,
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::ReportOnly,
            None,
        );
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
        let first_markdown = render_markdown_report(
            &report,
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::Automatic { deleted_count: 1 },
            None,
        );
        let second_markdown = render_markdown_report(
            &report,
            "sha256:abc",
            "2037-01-01T00:00:00Z",
            &CleanupReportStatus::Automatic { deleted_count: 1 },
            None,
        );
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
        let markdown = render_markdown_report(
            &report,
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::ReportOnly,
            None,
        );
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

    /// 空现场报告（模式行 / 令牌行 / 台账行断言只看头部）。
    fn empty_report() -> PatrolReport {
        PatrolReport {
            findings: Vec::new(),
            undetermined: Vec::new(),
            unclassified: Vec::new(),
            covered_record_types: Vec::new(),
        }
    }

    /// 自动模式（服务启动时自动执行）：模式行 MUST 写明本次实际删除条数，
    /// MUST NOT 再出现 dry-run 文案（缺陷复现：实删 6 条却写「未执行任何删除」）。
    #[test]
    fn dangling_report_automatic_mode_line_states_deleted_count() {
        let markdown = render_markdown_report(
            &empty_report(),
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::Automatic { deleted_count: 6 },
            None,
        );
        assert!(
            markdown.contains("自动销账（服务启动时自动执行；本次已删除 6 条）"),
            "自动模式必须写明实际删除数：{markdown}"
        );
        assert!(
            !markdown.contains("dry-run（未执行任何删除）"),
            "自动模式 MUST NOT 谎报未删除：{markdown}"
        );
    }

    /// 销账失败：模式行 MUST 写明错误原文，且 MUST NOT 声称「未删除任何记录」
    /// ——失败可能发生在部分删除之后，逐条真相以留痕台账为准。
    #[test]
    fn dangling_report_failed_cleanup_mode_line_admits_partial_state() {
        let reason =
            "Io(create /tmp/ws/.aria/state/entity-deletions/dangling-reference: Not a directory)";
        let markdown = render_markdown_report(
            &empty_report(),
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::AutomaticFailed {
                reason: reason.to_string(),
            },
            None,
        );
        assert!(
            markdown.contains(&format!("销账失败（错误：{reason}）")),
            "销账失败必须写明错误原因：{markdown}"
        );
        assert!(
            !markdown.contains("未删除任何记录"),
            "销账失败 MUST NOT 谎称未删除（可能已部分删除）：{markdown}"
        );
        assert!(
            !markdown.contains("dry-run（未执行任何删除）"),
            "销账失败 MUST NOT 沿用 dry-run 文案：{markdown}"
        );
    }

    /// 只报不删（dry-run 复核）路径：模式行保留「未执行任何删除」语义。
    #[test]
    fn dangling_report_report_only_mode_line_keeps_dry_run_wording() {
        let markdown = render_markdown_report(
            &empty_report(),
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::ReportOnly,
            None,
        );
        assert!(
            markdown.contains("只报不删（dry-run，未执行任何删除）"),
            "只报不删模式必须保留 dry-run 语义：{markdown}"
        );
    }

    /// 清单头部：令牌行口径与 REQ-DRG-22 一致（仅审计关联、不是删除凭据），
    /// 并指向留痕台账（逐条删除明细）。
    #[test]
    fn dangling_report_header_points_at_cleanup_ledger_and_audit_only_token() {
        let markdown = render_markdown_report(
            &empty_report(),
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::Automatic { deleted_count: 0 },
            None,
        );
        assert!(
            markdown.contains(
                "sha256:abc（= findings 规范化序列化的摘要；仅用于审计关联，不是删除凭据）"
            ),
            "令牌行必须与 REQ-DRG-22 一致：{markdown}"
        );
        assert!(
            !markdown.contains("须原样提供"),
            "令牌行 MUST NOT 再声称是删除凭据：{markdown}"
        );
        assert!(
            markdown.contains(CLEANUP_LEDGER_RELATIVE_PATH),
            "清单必须指向留痕台账：{markdown}"
        );
        assert!(markdown.contains("逐条删除明细"), "{markdown}");
    }

    /// 台账指针与 `ProductAppPaths` 的真实落点一致（防两处路径漂移）。
    #[test]
    fn dangling_report_ledger_pointer_matches_app_paths() {
        let workspace = std::path::Path::new("/tmp/ws");
        let paths = crate::product::app_paths::ProductAppPaths::new(workspace.join(".aria"));
        assert_eq!(
            workspace.join(CLEANUP_LEDGER_RELATIVE_PATH),
            paths.dangling_reference_ledger_path(),
            "清单里的台账相对路径必须与真实落点一致"
        );
    }

    /// 取清单中的某一节（到下一个 `## ` 标题为止）。
    fn section<'a>(markdown: &'a str, heading: &str) -> &'a str {
        let start = markdown
            .find(heading)
            .unwrap_or_else(|| panic!("清单缺少「{heading}」节：{markdown}"));
        let rest = &markdown[start..];
        let end = rest[heading.len()..]
            .find("\n## ")
            .map(|offset| heading.len() + offset + 1)
            .unwrap_or(rest.len());
        &rest[..end]
    }

    /// 「归零自证」节（REQ-DRG-17 / 23）：归零计数、未归零可删项逐条、不可判定项
    /// 逐条、治理是否完成四项齐备；未完成时 MUST NOT 宣称治理完成。
    #[test]
    fn dangling_report_self_proof_section_lists_unzeroed_and_undetermined() {
        let report = PatrolReport {
            findings: vec![finding(
                "projects/p1/issues/i1/a.json",
                "source.stories[].id",
                PATROL_ACTION_DELETE,
                "L1",
            )],
            undetermined: Vec::new(),
            unclassified: Vec::new(),
            covered_record_types: vec!["automation_plan_intent".to_string()],
        };
        let proof = GovernanceSelfProof {
            unzeroed: vec![finding(
                "projects/p1/issues/i1/automation-plan-intent.json",
                "source.stories[].id",
                PATROL_ACTION_DELETE,
                "L1",
            )],
            unzeroed_by_type: [("automation_plan_intent".to_string(), 1)]
                .into_iter()
                .collect(),
            residual_count: 0,
            undetermined: vec![UndeterminedEntry {
                record_path: "reviewer-windows/windows/win_bad.json".to_string(),
                reference_field: "attempt_id".to_string(),
                reason: "json parse failed".to_string(),
            }],
            governance_complete: false,
        };
        let markdown = render_markdown_report(
            &report,
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::Automatic { deleted_count: 1 },
            Some(&proof),
        );

        let self_proof = section(&markdown, "## 6. 归零自证（销账后现场）");
        assert!(
            self_proof.contains("| automation_plan_intent | 1 |"),
            "6.1 必须给出本次可删类型的归零计数：{self_proof}"
        );
        assert!(
            self_proof.contains("projects/p1/issues/i1/automation-plan-intent.json"),
            "6.2 必须逐条列出未归零可删项：{self_proof}"
        );
        assert!(
            self_proof.contains("reviewer-windows/windows/win_bad.json")
                && self_proof.contains("json parse failed"),
            "6.3 必须逐条列出不可判定项及原因：{self_proof}"
        );
        assert!(
            self_proof.contains("治理完成：否"),
            "6.4 治理未完成时必须写明「否」：{self_proof}"
        );
        assert!(
            !self_proof.contains("治理完成：是"),
            "存在未归零可删项时 MUST NOT 宣称治理完成：{self_proof}"
        );
    }

    /// 分节互斥（REQ-DRG-11）：销账后的「归零自证」节 MUST NOT 与销账前的
    /// 「明细 / 只报不删残留 / 不可判定」三节混列。
    #[test]
    fn dangling_report_self_proof_section_is_disjoint_from_pre_cleanup_sections() {
        let pre_only = finding(
            "projects/p1/issues/i1/pre.json",
            "session_id",
            PATROL_ACTION_REPORT_ONLY,
            "未确证",
        );
        let report = PatrolReport {
            findings: vec![pre_only.clone()],
            undetermined: Vec::new(),
            unclassified: Vec::new(),
            covered_record_types: vec!["automation_plan_intent".to_string()],
        };
        let proof = GovernanceSelfProof {
            unzeroed: vec![finding(
                "projects/p1/issues/i1/post.json",
                "source.stories[].id",
                PATROL_ACTION_DELETE,
                "L1",
            )],
            unzeroed_by_type: [("automation_plan_intent".to_string(), 1)]
                .into_iter()
                .collect(),
            residual_count: 0,
            undetermined: vec![UndeterminedEntry {
                record_path: "state/post/undetermined.json".to_string(),
                reference_field: "session_id".to_string(),
                reason: "post read failed".to_string(),
            }],
            governance_complete: false,
        };
        let markdown = render_markdown_report(
            &report,
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::Automatic { deleted_count: 0 },
            Some(&proof),
        );

        let residual_section = section(&markdown, "## 3. 只报不删残留");
        let undetermined_section = section(&markdown, "## 4. 不可判定");
        let self_proof = section(&markdown, "## 6. 归零自证（销账后现场）");
        assert!(
            residual_section.contains("projects/p1/issues/i1/pre.json")
                && !residual_section.contains("projects/p1/issues/i1/post.json"),
            "销账前残留节 MUST NOT 收录销账后的自证条目：{residual_section}"
        );
        assert!(
            undetermined_section.contains("（无）"),
            "销账前不可判定节 MUST NOT 收录销账后的不可判定条目：{undetermined_section}"
        );
        assert!(
            self_proof.contains("projects/p1/issues/i1/post.json")
                && !self_proof.contains("projects/p1/issues/i1/pre.json"),
            "归零自证节 MUST NOT 混列销账前的残留条目：{self_proof}"
        );
        assert!(
            self_proof.contains("state/post/undetermined.json"),
            "归零自证节必须逐条列出销账后的不可判定条目：{self_proof}"
        );
    }

    /// 销账失败（无自证现场）：本节 MUST 写明「未取得自证结果」，MUST NOT 宣称
    /// 治理完成。
    #[test]
    fn dangling_report_self_proof_section_admits_missing_proof() {
        let markdown = render_markdown_report(
            &empty_report(),
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::AutomaticFailed {
                reason: "Io(boom)".to_string(),
            },
            None,
        );
        let self_proof = section(&markdown, "## 6. 归零自证（销账后现场）");
        assert!(
            self_proof.contains("销账失败，未取得自证结果"),
            "销账失败时本节必须写明未取得自证结果：{self_proof}"
        );
        assert!(
            !self_proof.contains("治理完成：是"),
            "无自证结果时 MUST NOT 宣称治理完成：{self_proof}"
        );
    }

    /// 诚实口径：即使传入的完成标记为真，只要任一类计数非空，清单 MUST 写「否」
    /// ——MUST NOT 宣称治理完成而实际未完成。
    #[test]
    fn dangling_report_self_proof_never_claims_complete_when_counts_non_empty() {
        let proof = GovernanceSelfProof {
            unzeroed: vec![finding(
                "projects/p1/issues/i1/post.json",
                "source.stories[].id",
                PATROL_ACTION_DELETE,
                "L1",
            )],
            unzeroed_by_type: [("automation_plan_intent".to_string(), 1)]
                .into_iter()
                .collect(),
            residual_count: 0,
            undetermined: Vec::new(),
            governance_complete: true,
        };
        let markdown = render_markdown_report(
            &empty_report(),
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::Automatic { deleted_count: 0 },
            Some(&proof),
        );
        let self_proof = section(&markdown, "## 6. 归零自证（销账后现场）");
        assert!(
            self_proof.contains("治理完成：否"),
            "存在未归零可删项时 MUST NOT 宣称治理完成：{self_proof}"
        );
        assert!(
            !self_proof.contains("治理完成：是"),
            "存在未归零可删项时 MUST NOT 宣称治理完成：{self_proof}"
        );
    }

    /// 全部归零且无残留 / 无不可判定：本节 MUST 明确写「治理完成：是」。
    #[test]
    fn dangling_report_self_proof_section_states_completion_when_fully_zeroed() {
        let proof = GovernanceSelfProof {
            unzeroed: Vec::new(),
            unzeroed_by_type: BTreeMap::new(),
            residual_count: 0,
            undetermined: Vec::new(),
            governance_complete: true,
        };
        let markdown = render_markdown_report(
            &empty_report(),
            "sha256:abc",
            "2026-10-28T10:00:00Z",
            &CleanupReportStatus::Automatic { deleted_count: 0 },
            Some(&proof),
        );
        let self_proof = section(&markdown, "## 6. 归零自证（销账后现场）");
        assert!(
            self_proof.contains("治理完成：是"),
            "全归零时必须写明治理完成：{self_proof}"
        );
        assert!(
            self_proof.contains("（无：本次可删条目已全部归零）"),
            "6.2 必须显式写明无未归零项：{self_proof}"
        );
    }
}
