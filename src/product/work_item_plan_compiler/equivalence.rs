//! REQ-CRP-02/03：Plan Intent 分支等价解析（编译器侧）与 compile 审计出口。
//!
//! `lower.rs` 贴近 1200 行红线，按 Global Constraints「新增代码一律落新
//! 文件」整体落此；命名与签名按 Plan Task 1 Produces 契约固定，后续 Task
//! 经 `work_item_plan_compiler` 的 re-export 消费。等价解析只读 source
//! 事实与 context，绝不改写 source。

use super::lower::{PlanCandidateIr, WorkItemPlanSourceContext, diagnostic, lower_work_item_plan};
use super::types::{CompilerDiagnostic, LoweringFault};
use crate::product::lifecycle_store::candidate_attempts::EquivalenceCanonicalization;

/// 等价解析三态：唯一命中才采用；不在成员表/双命中一律 fail-closed。
enum EquivResolution {
    Adopted {
        repository_id: String,
        basis: &'static str,
    },
    NotInMemberTable,
    Ambiguous,
}

/// REQ-CRP-03：single_repository 分支的等价解析——值在成员表
/// `physical_by_logical` 唯一命中才采用；双命中（同时是某成员逻辑 key
/// 与任一成员物理仓值）或不在成员表一律 fail-closed，绝不取第一个命
/// 中；legacy 无成员表时会话 target 非聚合根视图才作回落（basis
/// 「session target」），聚合视图 typed 拒绝（B1 铁律，不猜）。
fn resolve_equivalent_single_repository_target(
    value: &str,
    context: &WorkItemPlanSourceContext,
) -> EquivResolution {
    use crate::product::logical_codebase::LogicalRepositoryId;

    let Some(members) = context.member_targets.as_ref() else {
        if context
            .target_repository_id
            .starts_with(crate::product::workspace_repository::AGGREGATE_ROOT_VIEW_ID_PREFIX)
        {
            return EquivResolution::NotInMemberTable;
        }
        return EquivResolution::Adopted {
            repository_id: context.target_repository_id.clone(),
            basis: "session target",
        };
    };
    let logical_hit = uuid::Uuid::parse_str(value)
        .ok()
        .and_then(|uuid| members.physical_by_logical.get(&LogicalRepositoryId(uuid)));
    let double_hit = members
        .physical_by_logical
        .values()
        .any(|physical| physical == value);
    match (logical_hit, double_hit) {
        (Some(physical), false) => EquivResolution::Adopted {
            repository_id: physical.clone(),
            basis: "physical_by_logical",
        },
        (Some(_), true) => EquivResolution::Ambiguous,
        (None, _) => EquivResolution::NotInMemberTable,
    }
}

/// REQ-CRP-02：显式 typed fault 工厂——只在机器事实完整的构造点调用
///（Plan Intent 等价解析 fail-closed / 必填缺席），不从 code/message 派生。
pub(super) fn diagnostic_with_fault(
    field: &str,
    message: &str,
    line: usize,
    repair_example: &str,
    fault: LoweringFault,
) -> CompilerDiagnostic {
    CompilerDiagnostic {
        lowering_fault: Some(fault),
        ..diagnostic(field, message, line, repair_example)
    }
}

/// `lower_plan_intent` 的 single_repository 分支接线：值在
/// `intent_target_logical_repo` 时做等价解析——`Adopted` 采用映射物理仓
/// 并把审计（kind 固定 `intent_target_repository`）push 进
/// canonicalizations；`NotInMemberTable`/`Ambiguous` 返回携带 typed
/// `BranchKeyMismatch` 的 fail-closed 诊断（REQ-CRP-02：fault 在本构造点
/// 显式写入，不从 code/message 派生；行号取调用方传入的实际行）。
pub(super) fn equivalent_single_repository_target(
    equivalent: &str,
    equivalent_line: usize,
    context: &WorkItemPlanSourceContext,
    canonicalizations: &mut Vec<EquivalenceCanonicalization>,
) -> Result<String, Box<CompilerDiagnostic>> {
    match resolve_equivalent_single_repository_target(equivalent, context) {
        EquivResolution::Adopted {
            repository_id,
            basis,
        } => {
            canonicalizations.push(EquivalenceCanonicalization {
                kind: "intent_target_repository".to_string(),
                original_key: "intent_target_logical_repo".to_string(),
                value: equivalent.to_string(),
                resolution_basis: basis.to_string(),
            });
            Ok(repository_id)
        }
        rejected => {
            let reason = match rejected {
                EquivResolution::Ambiguous => "双命中：同时命中成员逻辑 key 与另一成员物理仓值",
                _ => "不在成员表，或 legacy 无成员表且会话 target 为聚合根视图",
            };
            Err(Box::new(diagnostic_with_fault(
                "contract.intent.intent_target_logical_repo",
                &format!(
                    "intent_target_kind=single_repository 的 target 值填在了 intent_target_logical_repo（{reason}）；single_repository 分支应写 intent_target_repo。"
                ),
                equivalent_line,
                "- intent_target_repo: value",
                LoweringFault::BranchKeyMismatch {
                    kind: "single_repository",
                    expected_field: "intent_target_repo",
                    actual_field: "intent_target_logical_repo",
                    value: equivalent.to_string(),
                    line: equivalent_line,
                    expected_shape: "- intent_target_repo: value",
                },
            )))
        }
    }
}

/// compile 阶段产出的等价规范化审计（REQ-CRP-03；DTO 直接复用
/// lifecycle_store 的 `EquivalenceCanonicalization`，与尝试流水同序列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPlanAudit {
    pub equivalence_canonicalizations: Vec<EquivalenceCanonicalization>,
}

/// 审计出口：等价解析的 canonicalizations 随 IR 一并返回；既有
/// `compile_work_item_plan` 委托本入口并丢弃 audit（旧签名与调用点不变）。
pub fn compile_work_item_plan_audited(
    source: &str,
    context: &WorkItemPlanSourceContext,
) -> Result<(PlanCandidateIr, CompiledPlanAudit), Vec<CompilerDiagnostic>> {
    let ast = super::parse_work_item_plan(source)?;
    let mut canonicalizations = Vec::new();
    let ir = lower_work_item_plan(source, ast, context, &mut canonicalizations)?;
    Ok((
        ir,
        CompiledPlanAudit {
            equivalence_canonicalizations: canonicalizations,
        },
    ))
}
