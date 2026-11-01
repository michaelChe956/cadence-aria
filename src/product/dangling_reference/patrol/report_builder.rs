//! 巡检结果累加器（由 patrol.rs 按职责原样拆出：纯移动 + 必要的
//! pub(super) 放宽，零逻辑变化）。

use super::*;
/// 巡检结果累加器（三栏 + 覆盖类型，收尾统一排序去重保证逐次可重现）。
#[derive(Default)]
pub(super) struct ReportBuilder {
    findings: Vec<PatrolFinding>,
    undetermined: Vec<UndeterminedEntry>,
    unclassified: Vec<UnclassifiedEntry>,
    covered_record_types: BTreeSet<String>,
}

impl ReportBuilder {
    /// 登记「本次巡检覆盖了该记录类型」。
    pub(super) fn mark_covered(&mut self, record_type: DanglingRecordType) {
        self.covered_record_types
            .insert(record_type.as_str().to_string());
    }

    /// 记一条发现。
    pub(super) fn push_finding(&mut self, finding: PatrolFinding) {
        self.findings.push(finding);
    }

    /// 记一条不可判定（REQ-DRG-23）。
    pub(super) fn push_undetermined(
        &mut self,
        record_path: String,
        reference_field: &str,
        reason: String,
    ) {
        self.undetermined.push(UndeterminedEntry {
            record_path,
            reference_field: reference_field.to_string(),
            reason,
        });
    }

    /// 记一条未归类（REQ-DRG-09：只报不删）。
    pub(super) fn push_unclassified(&mut self, record_path: String) {
        self.mark_covered(DanglingRecordType::Unclassified);
        self.unclassified.push(UnclassifiedEntry {
            record_path,
            record_type: DanglingRecordType::Unclassified.as_str().to_string(),
        });
    }

    /// 收尾：排序 + 去重后组装报告（去掉重复扫描带来的重复行）。
    pub(super) fn finish(mut self) -> PatrolReport {
        self.findings.sort_by(|left, right| {
            left.record_path
                .cmp(&right.record_path)
                .then_with(|| left.reference_field.cmp(&right.reference_field))
                .then_with(|| left.target_kind.cmp(&right.target_kind))
                .then_with(|| left.target_id.cmp(&right.target_id))
        });
        self.findings.dedup();
        self.undetermined.sort_by(|left, right| {
            left.record_path
                .cmp(&right.record_path)
                .then_with(|| left.reference_field.cmp(&right.reference_field))
                .then_with(|| left.reason.cmp(&right.reason))
        });
        self.undetermined.dedup();
        self.unclassified.sort_by(|left, right| {
            left.record_path
                .cmp(&right.record_path)
                .then_with(|| left.record_type.cmp(&right.record_type))
        });
        self.unclassified.dedup();
        PatrolReport {
            findings: self.findings,
            undetermined: self.undetermined,
            unclassified: self.unclassified,
            covered_record_types: self.covered_record_types.into_iter().collect(),
        }
    }
}
