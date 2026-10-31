//! 「物理成员仓 → 逻辑成员」归属的唯一来源（REQ-WG-02）。
//!
//! 候选结构门（`plan_projection::assess_single_candidate_structure`）与 Final
//! Compile（`compile::finalizer::work_item_record_attribution`）都消费本模块：
//! 同一份 source 因此在两处解析出同一逻辑成员，仓序自动边在两处同口径生效
//! （REQ-WG-01「两处依赖图结论一致」的机械前提）。
//!
//! 反查是三态的——唯一命中 / 无命中 / 多命中。非唯一命中时**禁止猜测**：
//! 候选门不造归属（不计入仓序自动边），Final Compile 报
//! `work_item_target_missing`（既有 fail-closed 语义）。

use std::collections::BTreeMap;

use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
use crate::product::work_item_plan_compiler::PlanCandidateIr;
use crate::product::workspace_engine::draft_batch::compile_support::{
    PlanningStructureCanonicalInput, PlanningStructureItem, PlanningStructureSpan,
};

/// 物理成员仓反查逻辑成员的三态结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberReverse {
    /// 成员表里恰有一个逻辑成员对应此物理仓。
    Unique(LogicalRepositoryId),
    /// 成员表里没有任何成员对应此物理仓。
    Absent,
    /// 多个逻辑成员共用同一物理仓（配置矛盾）：不猜任一个。
    Ambiguous,
}

/// 唯一反查实现（REQ-WG-02）：成员表方向是「逻辑成员 → 物理仓」，本函数
/// 反向取「物理仓 → 逻辑成员」。候选门与终编译 MUST 共用此函数，不得各自
/// 迭代成员表（两套反查正是「review 说无环、终编译说成环」的根因）。
pub(crate) fn reverse_member_logical(
    physical: &str,
    physical_by_logical: &BTreeMap<LogicalRepositoryId, String>,
) -> MemberReverse {
    let mut found: Option<LogicalRepositoryId> = None;
    for (logical, candidate) in physical_by_logical {
        if candidate != physical {
            continue;
        }
        if found.is_some() {
            return MemberReverse::Ambiguous;
        }
        found = Some(*logical);
    }
    match found {
        Some(logical) => MemberReverse::Unique(logical),
        None => MemberReverse::Absent,
    }
}

/// 将候选 IR 投影为 SG1 纯图输入。lowering 后的 IR 不保留 source 行号——
/// canonical input 的 span 只携带逻辑字段路径（line=0=无行事实，缺事实不
/// 伪行，与 F1.2 validator 投影同口径）；item 级 source_spans 为空。
///
/// 这里只落 **声明事实**（`declared_member`）与可从声明直取的目标
/// （`logical_codebase` 的逻辑成员）。需要成员表反查的归属
/// （`single_repository` / 未声明 intent）由 [`resolve_member_targets`] 补齐。
pub(crate) fn planning_structure_items_from_ir(ir: &PlanCandidateIr) -> Vec<PlanningStructureItem> {
    ir.items
        .iter()
        .map(|item| {
            let (declared_member, logical_target) = match item
                .contract
                .intent_contract
                .as_ref()
                .map(|intent| &intent.target)
            {
                Some(EnrollmentTarget::LogicalCodebase {
                    logical_repository_id,
                    ..
                }) => (
                    Some(logical_repository_id.0.to_string()),
                    Some(*logical_repository_id),
                ),
                // 批 2 R1（REQ-WIGA-01）：集合 intent 显式分支不造归属
                //（门前 lower/validator 已拒集合 WI；此处签名不变、不取首
                // 成员、不展开）。
                Some(EnrollmentTarget::LogicalCodebaseMembers { .. }) => (None, None),
                // A1（REQ-WG-02）：single_repository 只声明物理仓；逻辑成员
                // 须经唯一反查（需成员表）在 resolve_member_targets 补齐。
                Some(EnrollmentTarget::SingleRepository { repository_id }) => {
                    (Some(repository_id.clone()), None)
                }
                None => (None, None),
            };
            PlanningStructureItem {
                logical_work_item_id: item.contract.identity.logical_work_item_id.clone(),
                short_id: item.contract.identity.logical_work_item_id.clone(),
                title: item.contract.identity.title.clone(),
                declared_member,
                logical_target,
                physical_target: Some(item.target_repository_id.clone()),
                canonical_inputs: item
                    .contract
                    .input_contracts
                    .iter()
                    .enumerate()
                    .map(|(index, contract)| PlanningStructureCanonicalInput {
                        contract: contract.clone(),
                        span: PlanningStructureSpan {
                            field: format!("input_contracts[{index}]"),
                            line: 0,
                        },
                    })
                    .collect(),
                declared_depends_on: item.contract.depends_on.clone(),
                source_spans: Vec::new(),
            }
        })
        .collect()
}

/// 用授权成员表补齐 IR 投影出的归属解析结果（`logical_target`），口径与
/// Final Compile 逐项同源：
///
/// - `logical_codebase`：声明即逻辑成员（投影已落，不重复计算）；
/// - `logical_codebase_members`（集合 intent）：显式不造归属（REQ-WIGA-01）；
/// - `single_repository`：声明物理仓 → 唯一反查，非唯一不猜；
/// - 未声明 intent：Final Compile 按 IR 已定的确定性回落物理仓反查
///   （lowering 已对失效/缺失回落 fail-closed），候选门同口径，避免同一份
///   source 在两处得出不同仓库归属。
///
/// 成员表为空（legacy 单仓）时上游调用方仍按既有语义清空归属，本函数不
/// 改变单仓语义。
pub(crate) fn resolve_member_targets(
    items: &mut [PlanningStructureItem],
    ir: &PlanCandidateIr,
    physical_by_logical: &BTreeMap<LogicalRepositoryId, String>,
) {
    debug_assert_eq!(
        items.len(),
        ir.items.len(),
        "structure items 与 IR items 必须一一对应（同一次投影产物）"
    );
    for (item, ir_item) in items.iter_mut().zip(ir.items.iter()) {
        let resolved = match ir_item
            .contract
            .intent_contract
            .as_ref()
            .map(|intent| &intent.target)
        {
            Some(EnrollmentTarget::LogicalCodebase { .. }) => continue,
            Some(EnrollmentTarget::LogicalCodebaseMembers { .. }) => MemberReverse::Absent,
            Some(EnrollmentTarget::SingleRepository { repository_id }) => {
                item.declared_member = Some(repository_id.clone());
                reverse_member_logical(repository_id, physical_by_logical)
            }
            None => item
                .physical_target
                .as_deref()
                .map_or(MemberReverse::Absent, |physical| {
                    reverse_member_logical(physical, physical_by_logical)
                }),
        };
        item.logical_target = match resolved {
            MemberReverse::Unique(logical) => Some(logical),
            MemberReverse::Absent | MemberReverse::Ambiguous => None,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logical(seed: u8) -> LogicalRepositoryId {
        let mut bytes = [0u8; 16];
        bytes[0] = seed;
        LogicalRepositoryId(uuid::Uuid::from_bytes(bytes))
    }

    #[test]
    fn reverse_member_logical_is_three_state_and_never_guesses() {
        let (first, second) = (logical(0x01), logical(0x02));
        let table = BTreeMap::from([
            (first, "repository_a".to_string()),
            (second, "repository_shared".to_string()),
        ]);
        assert_eq!(
            reverse_member_logical("repository_a", &table),
            MemberReverse::Unique(first)
        );
        assert_eq!(
            reverse_member_logical("repository_unknown", &table),
            MemberReverse::Absent
        );
        // 多命中：同一个物理仓对应两个逻辑成员 → Ambiguous（不取任一个）。
        let ambiguous = BTreeMap::from([
            (first, "repository_shared".to_string()),
            (second, "repository_shared".to_string()),
        ]);
        assert_eq!(
            reverse_member_logical("repository_shared", &ambiguous),
            MemberReverse::Ambiguous
        );
        // 空成员表（legacy 单仓）：任何物理仓都是 Absent，不造归属。
        assert_eq!(
            reverse_member_logical("repository_a", &BTreeMap::new()),
            MemberReverse::Absent
        );
    }
}
