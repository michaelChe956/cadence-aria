//! 5.3（REQ-WSC-24 残留）：Final Compile prepare 错误通道的 typed 分流。
//!
//! [`prepare_initial_plan_compile`](super::prepare_initial_plan_compile) 的
//! 错误从 String 通道换成 [`InitialPlanCompileFailure`]：
//! - 依赖环（compile_support 的 change_order 环检测，REQ-WSC-18 稳定码
//!   `dependency_cycle: ` 前缀 + 机器标记段）透传为
//!   `Structure(PlanningFailureSet)`；
//! - 其余（identity/draft/分配等）按 `Operational(cause)` 透传。
//!
//! 失败面分流（`compile/single_candidate.rs` 的 prepare 包裹点）单源在本文件
//! 的 [`into_compile_failure`]：`Operational` → 既有
//! `operational("final_compile_prepare_failed", cause)`（稳定码保留一个
//! 版本周期的兼容面）；`Structure` → `PlanningCompileFailure::structure`。

use std::collections::BTreeMap;

use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{
    IssueWorkItemPlan, LifecycleWorkItemRecord, VerificationPlan, WorkItemDraftRecord,
};
use crate::product::planning_failure::{
    PlanningFailure, PlanningFailureEvidence, PlanningFailureKind, PlanningFailureOrigin,
    PlanningFailureSet, PlanningStage,
};
use crate::product::workspace_engine::compile_failure_surface::CYCLE_IDS_MARKER;
use crate::product::workspace_engine::draft_batch::compile_support::project_work_item_plan_drafts_for_compile;
use crate::product::workspace_engine::types::{
    InitialPlanCompileFailure, PlanningCompileFailure, WorkItemPlanCompileProjectionContext,
};

/// 失败面分流（5.3）：`Operational` → 既有
/// `operational("final_compile_prepare_failed", cause)`；`Structure` →
/// `PlanningCompileFailure::structure(failures)`。
pub(crate) fn into_compile_failure(failure: InitialPlanCompileFailure) -> PlanningCompileFailure {
    match failure {
        InitialPlanCompileFailure::Operational(cause) => {
            PlanningCompileFailure::operational("final_compile_prepare_failed", cause)
        }
        InitialPlanCompileFailure::Structure(failures) => {
            PlanningCompileFailure::structure(failures)
        }
    }
}

/// 终编译投影入口（compile_support 单源算法 + 环形态 typed 分流）——
/// `prepare_initial_plan_compile` 的唯一投影调用点，错误通道直接产
/// `InitialPlanCompileFailure`（环 → Structure，其余 → Operational）。
pub(super) fn project_work_item_plan_drafts(
    previous_plan: &IssueWorkItemPlan,
    draft_records: &[WorkItemDraftRecord],
    context: WorkItemPlanCompileProjectionContext<'_>,
    change_order: &[LogicalRepositoryId],
) -> Result<
    (
        IssueWorkItemPlan,
        Vec<LifecycleWorkItemRecord>,
        Vec<VerificationPlan>,
    ),
    InitialPlanCompileFailure,
> {
    project_work_item_plan_drafts_for_compile(previous_plan, draft_records, context, change_order)
        .map_err(from_projection_cause)
}

/// prepare 内部既有 String 错误点（identity/draft/分配失败等）的通道适配：
/// 一律按 operational cause 透传，语义与改前逐字一致。
impl From<String> for InitialPlanCompileFailure {
    fn from(cause: String) -> Self {
        Self::Operational(cause)
    }
}

/// 字面量错误点（prepare 内的固定文案）的直通适配。
impl From<&'static str> for InitialPlanCompileFailure {
    fn from(cause: &'static str) -> Self {
        Self::Operational(cause.to_string())
    }
}

/// 旧 String 边界（legacy run 与 SC recovery 的人读渲染通道）——只取渲染，
/// 不作为身份来源（同 `compile.rs` run_work_item_plan_compile 的边界口径）。
impl From<InitialPlanCompileFailure> for String {
    fn from(failure: InitialPlanCompileFailure) -> Self {
        match failure {
            InitialPlanCompileFailure::Operational(cause) => cause,
            InitialPlanCompileFailure::Structure(failures) => {
                PlanningCompileFailure::structure(failures).render()
            }
        }
    }
}

/// 终编译投影错误 → typed 分流：环形态转 `Structure(集合)`，其余按
/// operational 透传。环形态的识别依据是 REQ-WSC-18 的稳定码前缀
/// （`dependency_cycle_cause` 单源装配，两侧产者同模板），不是自由文本。
fn from_projection_cause(cause: String) -> InitialPlanCompileFailure {
    if cause.starts_with("dependency_cycle: ") {
        InitialPlanCompileFailure::Structure(cycle_failure_set(&cause))
    } else {
        InitialPlanCompileFailure::Operational(cause)
    }
}

/// 环形态的 typed 结构失败集合：
/// - kind = `DependencyCycle`；affected_ids 从 REQ-WSC-18 机器 id 标记段还原
///   （环上 Work Item id 原文）；
/// - 完整 cause 原文进 `evidence.details`——结构指纹与机器 id 标记段随原文
///   保留，`compile_failure_surface` 的环事实解析与同环指纹判定（5.4）不回退。
///
/// witness 留空：逐跳 witness 只在 compile_support 合并图内产生，prepare 纯层
/// 不可得（不复制边算法、不以残余集合画环——REQ-PLN-10）；按 REQ-SG-01 事实
/// 不完整即 fail-closed——该形态只落 owner 结构停点，不产生 L1 author 资格。
/// source_ref/source_hash 同理留空：prepare 纯层无 source 身份，身份由失败
/// 边界的 `PlanningCompileFailure::seal` 补齐。
fn cycle_failure_set(cause: &str) -> PlanningFailureSet {
    let affected_ids = cause
        .split_once(CYCLE_IDS_MARKER)
        .map(|(_, ids)| {
            ids.lines()
                .next()
                .unwrap_or_default()
                .split(", ")
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut details = BTreeMap::new();
    details.insert("dependency_cycle_cause".to_string(), cause.to_string());
    PlanningFailureSet {
        failures: vec![PlanningFailure {
            kind: PlanningFailureKind::DependencyCycle {
                witness: Vec::new(),
            },
            origin: PlanningFailureOrigin::EffectiveGraph,
            stage: PlanningStage::Plan,
            diagnostic_ref: "planning-structure/final-compile/dependency_cycle".to_string(),
            source_ref: String::new(),
            source_hash: String::new(),
            evidence: PlanningFailureEvidence {
                affected_ids,
                details,
            },
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::workspace_engine::compile_failure_surface::CYCLE_FINGERPRINT_MARKER;

    /// 环 cause（REQ-WSC-18 单源模板形态）→ Structure 集合：机器 id 段还原
    /// affected_ids，cause 原文与指纹标记进 details。
    #[test]
    fn cycle_cause_builds_structure_set_with_machine_facts() {
        let cause = format!(
            "dependency_cycle: 「A」 → 「B」 → 「A」 构成依赖环\
             {CYCLE_FINGERPRINT_MARKER}de56fc5b{CYCLE_IDS_MARKER}\
             work_item_compile_x_001, work_item_compile_x_002"
        );
        let failure = from_projection_cause(cause.clone());
        let InitialPlanCompileFailure::Structure(set) = failure else {
            panic!("环形态必须是 Structure：{failure:?}");
        };
        assert_eq!(set.failures.len(), 1);
        let failure = &set.failures[0];
        assert!(matches!(
            failure.kind,
            PlanningFailureKind::DependencyCycle { .. }
        ));
        assert_eq!(
            failure.evidence.affected_ids,
            vec![
                "work_item_compile_x_001".to_string(),
                "work_item_compile_x_002".to_string()
            ]
        );
        assert_eq!(
            failure.evidence.details.get("dependency_cycle_cause"),
            Some(&cause)
        );
    }

    /// 非环 cause 原样走 operational，不吞不改。
    #[test]
    fn non_cycle_cause_stays_operational_verbatim() {
        let failure =
            from_projection_cause("initial plan compile input identity mismatch".to_string());
        assert_eq!(
            failure,
            InitialPlanCompileFailure::Operational(
                "initial plan compile input identity mismatch".to_string()
            )
        );
    }
}
