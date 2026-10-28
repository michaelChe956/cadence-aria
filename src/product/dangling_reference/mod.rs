//! 悬空引用治理（`add-dangling-reference-governance`）。
//!
//! 本模块承载第 ① 层的判定基础：目标类型枚举与字段名判据表
//! （`reference_table`，纯静态表、无 IO）、告警结构与去重上界（REQ-DRG-01 /
//! REQ-DRG-07），以及目标存在性与已删证据分级（`resolve`；REQ-DRG-01 / 02 /
//! 12 / 21 / 23）。
//!
//! `resolve` 子模块**会做只读 IO**：读 deletion 墓碑、`entity-operations` 下的
//! pending-failure 事实、reviewer-window 树级租约文件、worktree 路径与
//! coding attempt / session 记录；全部为只读（不取 lease、不写回），读失败
//! 一律原样上抛、不得降级成「目标不存在」。
//!
//! `listing` 子模块**同样只做只读 IO**：读自动化 intent、reviewer 窗口记录、
//! reviewer 快照 manifest 与实体身份台账条目。它是第 ① 层的容错列举面
//! （REQ-DRG-03）：遇到指向已删对象的条目时**跳过该条目并产生一条告警**，
//! MUST NOT 使整次列举失败；只吞「目标已删」这一种错（REQ-DRG-05），
//! JSON 解析失败 / IO 失败一律原样上抛（不得折叠成「跳过」）。存在性判为
//! 「不可判定」（`Unknown`）的引用既不跳过条目也不产生告警、不计入悬空
//! 条目数（REQ-DRG-23）。
//!
//! `patrol` 子模块**同样只读、零副作用**：遍历五个分区（issue 级 / reviewer
//! 窗口 / reviewer 快照 / 实体身份台账 / 逻辑代码库 scope）核对引用目标是否
//! 仍存在，产出巡检报告（REQ-DRG-08 / 09 / 10）。它**不删除、不移动、不修改
//! 任何文件**，也不落盘清单。记录自身解析失败、分区不可读、会话枚举失败与
//! 存在性判 `Unknown` 一律进「不可判定」（REQ-DRG-23）。
//!
//! `report` 子模块把 `patrol` 产出的三栏结果渲染成 Markdown 巡检清单并计算
//! findings 摘要令牌（REQ-DRG-11 / 22）：令牌只对 findings 的规范化序列化取
//! 摘要，不含巡检时间、令牌行自身与不可判定 / 未归类条目；清单落点为
//! `cadence/analysis-docs/`。
//!
//! `ledger` 子模块是销账留痕的落盘面（REQ-DRG-16；技术方案 §3.5.2）：状态根下
//! **单一全局**追加式 JSONL 台账（`{state_root}/entity-deletions/dangling-reference/
//! cleanup.jsonl`）。它**逐行追加**紧凑 JSON（不复用整文件覆盖的写语义）、按
//! `record_path` + `target_id` + `action` 全量比对去重（同一清理动作重入不追加
//! 重复行），并按大小轮转 + 保留期分片清理。它不判定删什么、也不执行删除。
//! 销账规则与编排子模块由后续任务追加。

// Task 1 只产出静态表与纯结构，调用点自 Task 2 起接线，故非测试构建下这些项
// 暂时无用户；本行按词法覆盖整个模块树（含子模块 `reference_table` 与 `resolve`）。
// **全部接线完成后（Task 9）删除本行**，让死代码重新回到门禁视野。
#![allow(dead_code)]

pub(crate) mod ledger;
pub(crate) mod listing;
pub(crate) mod patrol;
pub(crate) mod reference_table;
pub(crate) mod report;
pub(crate) mod resolve;

/// 悬空引用告警的单一稳定码（REQ-DRG-07）。
pub(crate) const DANGLING_REFERENCE_WARNING_CODE: &str = "dangling_reference_target_missing";

/// 单次列举的告警条数上界（REQ-DRG-07；U2 实施默认值 64，owner 可调）。
pub(crate) const DANGLING_REFERENCE_WARNING_LIMIT: usize = 64;

/// 一条悬空引用告警：REQ-DRG-07 要求的七个字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DanglingReferenceWarning {
    /// 单一稳定码，恒为 [`DANGLING_REFERENCE_WARNING_CODE`]。
    pub(crate) code: String,
    /// 记录路径（相对状态根）。
    pub(crate) record_path: String,
    /// 记录类型标识（[`reference_table::DanglingRecordType::as_str`] 取值）。
    pub(crate) record_type: String,
    /// 命中的引用字段路径。
    pub(crate) reference_field: String,
    /// 引用目标类型标识（`DanglingTargetKind::as_str` 取值）。
    pub(crate) target_kind: String,
    /// 引用目标 id。
    pub(crate) target_id: String,
    /// 处置动作（如 `skipped`）。
    pub(crate) action: String,
}

/// 单次列举的告警集合：按 `(record_path, reference_field, target_id)` 去重，
/// 并按 [`DANGLING_REFERENCE_WARNING_LIMIT`] 截断上界（REQ-DRG-07）。
///
/// 内部按去重键升序存放（而非入列顺序），使 `items()` 的取值与遍历顺序无关、
/// 逐次可重现——便于对照与复跑比对。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DanglingReferenceWarnings {
    items: Vec<DanglingReferenceWarning>,
}

/// 去重键：同一「记录路径 + 引用字段 + 目标 id」只产生一条告警（REQ-DRG-07）。
fn warning_key(warning: &DanglingReferenceWarning) -> (&str, &str, &str) {
    (
        &warning.record_path,
        &warning.reference_field,
        &warning.target_id,
    )
}

impl DanglingReferenceWarnings {
    /// 入列一条告警。
    ///
    /// 命中重复键（同一记录路径 + 引用字段 + 目标 id）或已达上界时返回 `false`
    /// 且**不入列**——超出上界的部分 MUST 被丢弃，且 MUST NOT 影响列举结果。
    pub(crate) fn push(&mut self, warning: DanglingReferenceWarning) -> bool {
        let key = warning_key(&warning);
        match self
            .items
            .binary_search_by(|existing| warning_key(existing).cmp(&key))
        {
            // 重复触碰只告警一次。
            Ok(_) => false,
            Err(position) => {
                // 上界截断：超出部分直接丢弃（先到先得，不挤占已入列条目）。
                if self.items.len() >= DANGLING_REFERENCE_WARNING_LIMIT {
                    return false;
                }
                self.items.insert(position, warning);
                true
            }
        }
    }

    /// 已入列的告警（按去重键升序）。
    pub(crate) fn items(&self) -> &[DanglingReferenceWarning] {
        &self.items
    }

    /// 已入列的告警条数。
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod warning_tests {
    use super::{
        DANGLING_REFERENCE_WARNING_CODE, DANGLING_REFERENCE_WARNING_LIMIT,
        DanglingReferenceWarning, DanglingReferenceWarnings,
    };

    fn warning(
        record_path: &str,
        reference_field: &str,
        target_id: &str,
    ) -> DanglingReferenceWarning {
        DanglingReferenceWarning {
            code: DANGLING_REFERENCE_WARNING_CODE.to_string(),
            record_path: record_path.to_string(),
            record_type: "reviewer_window".to_string(),
            reference_field: reference_field.to_string(),
            target_kind: "attempt".to_string(),
            target_id: target_id.to_string(),
            action: "skipped".to_string(),
        }
    }

    #[test]
    fn dangling_reference_warning_code_and_limit_lock_literal_values() {
        // 逐字锁定契约字面值：改码或改上界必须显式改断言。
        assert_eq!(
            DANGLING_REFERENCE_WARNING_CODE,
            "dangling_reference_target_missing"
        );
        assert_eq!(DANGLING_REFERENCE_WARNING_LIMIT, 64);
    }

    #[test]
    fn dangling_reference_warning_dedupes_and_caps() {
        let mut warnings = DanglingReferenceWarnings::default();
        assert!(warnings.push(warning(
            "reviewer-windows/windows/win_001.json",
            "attempt_id",
            "coding_attempt_9f3"
        )));
        assert!(!warnings.push(warning(
            "reviewer-windows/windows/win_001.json",
            "attempt_id",
            "coding_attempt_9f3"
        )));
        assert_eq!(warnings.len(), 1);

        for index in 0..200 {
            let pushed = warnings.push(warning(
                &format!("reviewer-windows/windows/win_{index:04}.json"),
                "attempt_id",
                &format!("coding_attempt_{index:04}"),
            ));
            // 上界按「已入列条数」计：循环前集合已有 1 条（上面的去重用例），
            // 故本轮只有 LIMIT-1 条能入列。计划原文写 `index < LIMIT`，与紧随的
            // `len() == LIMIT` 断言自相矛盾（会要求共 1+64 条）。
            if index < DANGLING_REFERENCE_WARNING_LIMIT - 1 {
                assert!(pushed, "index={index}");
            } else {
                assert!(!pushed, "超出上界必须丢弃：index={index}");
            }
        }
        assert_eq!(warnings.len(), DANGLING_REFERENCE_WARNING_LIMIT);
        let first = &warnings.items()[0];
        assert_eq!(first.code, "dangling_reference_target_missing");
        assert_eq!(first.record_path, "reviewer-windows/windows/win_0000.json");
        assert_eq!(first.reference_field, "attempt_id");
        assert_eq!(first.target_kind, "attempt");
        assert_eq!(first.target_id, "coding_attempt_0000");
        assert_eq!(first.action, "skipped");
    }
}
