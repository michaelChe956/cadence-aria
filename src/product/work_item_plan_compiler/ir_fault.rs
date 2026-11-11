//! 四类 IR 诊断的独立 typed 载荷（MODIFIED REQ-WSC-09，工作包 2.6）。
//!
//! `IrFault` 是与 wire 枚举 `PlanningFailureKind`（REQ-SG-01/02 供需族）
//! 正交的新侧信道：只在 `work_item_contract::validation` 四码 producer
//!（unknown_done_when_ref / unknown_requirement_ref /
//! unknown_reviewer_check_ref / acceptance_criterion_without_reviewer_check）
//! 的构造点显式写入——缺事实不猜、无行不伪行，严禁从 code/message 反推。
//! serde 派生与 `IrFaultKind` 的 tag 形状（`tag = "kind"` +
//! `rename_all = "snake_case"`）与既有 `PlanningFailureKind` 完全同形；
//! `ContractValidationFinding.ir_fault` 以 Option + default + skip 落库，
//! 旧记录再序列化字节不变（plan §0.5 哈希兼容纪律）。

use serde::{Deserialize, Serialize};

use super::lower::split_value;
use super::parse::parse_work_item_plan;
use super::types::{WorkItemPlanAst, WorkItemPlanSectionAst};

/// 四类 IR 交叉引用诊断的 typed 载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrFault {
    /// 四类之一的 typed 故障身份。
    pub kind: IrFaultKind,
    /// 所属 work item 的 logical id。
    pub work_item_id: String,
    /// 定位（钉死规则：task 引用错 → `Task`；reviewer check 引用错 →
    /// `Handoff`；未被覆盖 criterion → `Criterion`）。
    pub locator: IrFaultLocator,
    /// 出错的具体引用值。
    pub offending_ref: String,
    /// 本 WI 合法集合或真实 spec 集合（producer 按循环前已算好的集合
    /// 填充，不臆造、不排序改写）。
    pub legal_refs: Vec<String>,
    /// 1-based source 行号；`None`＝不可定位（REQ-WSC-09：无原行即不授
    /// 资格），由 [`attach_source_lines`] 经 AST span 补全，不以 0 冒充。
    pub source_line: Option<usize>,
    /// 所需字段形态（[`IrFaultKind::expected_shape`] 单一事实源）。
    pub expected_shape: String,
}

/// 四类 IR 诊断的身份枚举；serde 形状与 `PlanningFailureKind` 同形
/// （`tag = "kind"` + `snake_case` 变体名）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IrFaultKind {
    UnknownDoneWhenRef,
    UnknownReviewerCheckRef,
    AcceptanceCriterionWithoutReviewerCheck,
    UnknownRequirementRef,
}

impl IrFaultKind {
    /// 该类故障所需字段形态（producer 构造与教学渲染的单一事实源）。
    pub(crate) fn expected_shape(&self) -> &'static str {
        match self {
            Self::UnknownDoneWhenRef => {
                "- done_when_refs: AC-001（逐字取自本 work item Acceptance Criteria 的 criterion_id，多个用逗号分隔）"
            }
            Self::UnknownReviewerCheckRef => {
                "- reviewer_check_refs: AC-001（逐字取自本 work item Acceptance Criteria 的 criterion_id，多个用逗号分隔）"
            }
            Self::AcceptanceCriterionWithoutReviewerCheck => {
                "- reviewer_check_refs: AC-001（Acceptance Criteria 的每个 criterion_id 都必须出现在 reviewer_check_refs 中）"
            }
            Self::UnknownRequirementRef => {
                "- requirement_refs: REQ-001（逐字取自本 work item Traceability 的 requirement_id，多个用逗号分隔）"
            }
        }
    }
}

/// 载荷定位：出错引用所在的声明位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrFaultLocator {
    /// task 的 `done_when_refs` / `requirement_refs` 引用错。
    Task { task_id: String },
    /// handoff 的 `reviewer_check_refs` 引用错。
    Handoff,
    /// criterion 未被任何 reviewer check 覆盖。
    Criterion { criterion_id: String },
}

/// 纯函数：用既有 [`parse_work_item_plan`] 解析 source、从 AST span 补
/// `source_line`（与编译器同一解析器，不是 message 反解析）。
///
/// - 找到出错引用所在字段行 → 写入该值所在 1-based 行号；
/// - source 中无该行（或 source 不可解析、kind×locator 组合不在钉死
///   规则内）→ 保持 `None`，不以 0 冒充（REQ-WSC-09：无原行即不授
///   资格）；
/// - 已有行号的载荷不覆盖（幂等）。
pub fn attach_source_lines(faults: &mut [IrFault], source: &str) {
    let Ok(ast) = parse_work_item_plan(source) else {
        return;
    };
    for fault in faults.iter_mut() {
        if fault.source_line.is_none() {
            fault.source_line = locate_fault_line(&ast, fault);
        }
    }
}

/// 按 kind×locator 的钉死规则在 AST 中定位出错引用行；无事实返回 None。
fn locate_fault_line(ast: &WorkItemPlanAst, fault: &IrFault) -> Option<usize> {
    let item = ast
        .items
        .iter()
        .find(|item| item.id.value == fault.work_item_id)?;
    let section = |name: &str| {
        item.sections
            .iter()
            .find(|section| section.name.value == name)
    };
    match (&fault.kind, &fault.locator) {
        (IrFaultKind::UnknownDoneWhenRef, IrFaultLocator::Task { task_id }) => task_ref_line(
            section("Tasks")?,
            task_id,
            "done_when_refs",
            &fault.offending_ref,
        ),
        (IrFaultKind::UnknownRequirementRef, IrFaultLocator::Task { task_id }) => task_ref_line(
            section("Tasks")?,
            task_id,
            "requirement_refs",
            &fault.offending_ref,
        ),
        (IrFaultKind::UnknownReviewerCheckRef, IrFaultLocator::Handoff) => {
            section("Handoff Schema")?
                .fields
                .iter()
                .filter(|field| field.key.value == "reviewer_check_refs")
                .find(|field| split_value(&field.value.value).contains(&fault.offending_ref))
                .map(|field| field.value.line)
        }
        (
            IrFaultKind::AcceptanceCriterionWithoutReviewerCheck,
            IrFaultLocator::Criterion { criterion_id },
        ) => section("Acceptance Criteria")?
            .fields
            .iter()
            .filter(|field| field.key.value == "criterion_id")
            .find(|field| field.value.value == *criterion_id)
            .map(|field| field.value.line),
        // kind×locator 的其它组合不在钉死规则内：缺事实不猜，保持 None。
        _ => None,
    }
}

/// 在 Tasks section 的扁平字段流里按 task_id 边界定位引用错行——与
/// `lower_tasks` 同一分组口径（task_id 字段开新条目），值列表切分与
/// lowering 的 `split_value` 同一语义。
fn task_ref_line(
    section: &WorkItemPlanSectionAst,
    task_id: &str,
    key: &str,
    offending_ref: &String,
) -> Option<usize> {
    let mut current_task: Option<&str> = None;
    for field in &section.fields {
        match field.key.value.as_str() {
            "task_id" => current_task = Some(field.value.value.as_str()),
            field_key
                if field_key == key
                    && current_task == Some(task_id)
                    && split_value(&field.value.value).contains(offending_ref) =>
            {
                return Some(field.value.line);
            }
            _ => {}
        }
    }
    None
}
