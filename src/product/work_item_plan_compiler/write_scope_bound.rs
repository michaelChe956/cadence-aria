//! 写范围 × 被授权仓归属核对（REQ-WSC-10，owner 2026-10-30 真机现场）。
//!
//! 现场形态：多仓（LC 聚合根）plan 会话中 WI 的 `exclusive_scopes` 声明
//! `scripts/dev.sh`——该路径位于聚合根本地文件区（根目录不是 git 仓库，仅
//! 各成员仓入库），本 WI 执行期的写入面是其被授权成员仓的 worktree，任何
//! 写入都到不了这个路径。本模块在候选校验层前置拦截：逐 WI 判写面归属
//! （只判归属不判存在——被授权 checkout 内的新建路径不误伤），越界产出
//! 稳定错误码 `write_scope_outside_bound_repository` 的 Error finding
//! （preflight 族，回灌修订）。
//!
//! 零误伤边界：
//! - 成员 checkout 前缀表缺席（单仓/Manual）不触发（调用方门控，零回归）；
//! - 无固定路径锚点的 scope（`*` 等）与空 exclusive 列表不判（无确定
//!   路径域可判归属；空列表=不限制可写范围的既有语义）；
//! - 被授权成员 checkout 即聚合根（空前缀）时一切路径域都在其内；
//! - 基线树不参与（判存在属 `acceptance_path_not_in_baseline` 的错因）。
//!
//! fail-closed：前缀表在场而被授权仓不在表内（成员归属映射缺失/漂移）
//! → Error finding 说明「无法核对归属」，不以跳过吞错（与
//! `intent_target_bound_violation` 的映射缺失惯例同构）。

use std::collections::BTreeMap;

use crate::cross_cutting::worktree::scope_fixed_path_anchors;
use crate::product::models::{WorkItemSplitFinding, WorkItemSplitFindingSeverity};

use super::lower::{PlanCandidateIr, PlanCandidateItemIr};
use super::types::WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY_REPAIR_ACTION;

/// 归属核对的稳定 finding 码（preflight 族，types::PREFLIGHT_FINDING_CODES
/// 收录）。
pub(crate) const WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY: &str =
    "write_scope_outside_bound_repository";

/// 逐 work item 产出「写范围 × 被授权仓归属」越界 finding（每 WI 至多一条，
/// 冲突项聚合；顺序稳定：声明序，条目去重）。
///
/// 锚点提取复用仓内权威 [`scope_fixed_path_anchors`]（含 F-25 顿号方言与
/// 路径归一）；前缀包含判定（`anchor == prefix || anchor.starts_with("<prefix>/")`）
/// 与仓内既有前缀核对口径一致；case_sensitive 与
/// `check_target_forbidden` 的既有调用面一致。
pub(crate) fn write_scope_bound_repository_findings(
    ir: &PlanCandidateIr,
    member_checkout_prefixes: &BTreeMap<String, String>,
) -> Vec<WorkItemSplitFinding> {
    ir.items
        .iter()
        .filter_map(|item| item_finding(item, member_checkout_prefixes))
        .collect()
}

fn item_finding(
    item: &PlanCandidateItemIr,
    member_checkout_prefixes: &BTreeMap<String, String>,
) -> Option<WorkItemSplitFinding> {
    let scopes = &item.contract.write_policy.exclusive_scopes;
    if scopes.is_empty() {
        return None;
    }
    let work_item_id = &item.contract.identity.logical_work_item_id;
    let Some(bound_prefix) = member_checkout_prefixes.get(&item.target_repository_id) else {
        // fail-closed：被授权仓不在前缀表内＝成员归属映射缺失/漂移，
        // 无法核对归属 ≠ 放行（不以跳过吞错）。
        return Some(violation(
            work_item_id,
            format!(
                "work item {work_item_id} 的被授权仓 {} 不在成员 checkout 前缀表内，\
                 无法核对写范围归属（fail-closed，不以跳过吞错）；修复动作：\
                 {WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY_REPAIR_ACTION}",
                item.target_repository_id
            ),
        ));
    };
    // 被授权成员 checkout 即聚合根（空前缀）：一切路径域都在其写入面内。
    let mut violations: Vec<String> = Vec::new();
    if !bound_prefix.is_empty() {
        for scope in scopes {
            for anchor in scope_fixed_path_anchors(scope, true) {
                if let Some(entry) =
                    anchor_violation(scope, &anchor, bound_prefix, member_checkout_prefixes)
                    && !violations.contains(&entry)
                {
                    violations.push(entry);
                }
            }
        }
    }
    if violations.is_empty() {
        return None;
    }
    Some(violation(
        work_item_id,
        format!(
            "work item {work_item_id} 的写范围 [{}] 落在被授权仓 {}（checkout {bound_prefix}）之外：\
             聚合根非成员区不属于任何成员仓、其他成员 checkout 属于其归属 work item，\
             本项执行期均无法写入；修复动作：{WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY_REPAIR_ACTION}",
            violations.join("、"),
            item.target_repository_id
        ),
    ))
}

/// 单个 scope 锚点的归属判定：返回 `Some(条目文案)` 表示越界，`None`
/// 表示放行。条目以声明原文（含 glob 形态）点名，归属按其固定锚点判。
/// 三类越界文案分开：`..` 段（机械无法判归属，fail-closed）、其他成员
/// checkout（跨成员）、不落在任何成员 checkout（聚合根非成员区**或**仓内
/// 相对写法——两者语法不可区分，文案必须并列说明并给出改写示例，不得
/// 武断归类；运行期 `scope_allows_member_path` 对仓内相对写法的兼容语义
/// 不变，plan 期归属核对以聚合根为基准是两者的分工）。
fn anchor_violation(
    scope: &str,
    anchor: &str,
    bound_prefix: &str,
    member_checkout_prefixes: &BTreeMap<String, String>,
) -> Option<String> {
    // F4：`..` 相对段让字面前缀与实际落点脱钩（`busi/../x` 实际落在根区）
    // ——机械核对不展开、不猜，一律 fail-closed 出 finding。
    if anchor.split('/').any(|segment| segment == "..") {
        return Some(format!(
            "{scope}（含 .. 相对路径段，字面前缀与实际落点脱钩，机械无法判定归属，\
             fail-closed；写范围不得使用 ..，请改写为规范的聚合根相对路径）"
        ));
    }
    if path_within_prefix(anchor, bound_prefix) {
        return None;
    }
    // 跨成员写：按既有授权语义（每 WI 恰落一被授权成员仓）判越界，消息
    // 点名写面实际落在的成员 checkout 与归属物理仓；其余＝不落在任何成员
    // checkout（聚合根非成员区或仓内相对写法，见函数注释）。
    let owning = member_checkout_prefixes
        .iter()
        .find(|(_physical, checkout_dir)| {
            !checkout_dir.is_empty() && path_within_prefix(anchor, checkout_dir)
        });
    Some(match owning {
        Some((physical, checkout_dir)) => {
            format!("{scope}（成员 checkout {checkout_dir} 内，归属仓 {physical}）")
        }
        None => format!(
            "{scope}（按成员目录前缀写法不落在任何成员 checkout 下：若指聚合根非成员区\
             （根目录本地文件区）则不属于任何成员仓、本项不可写；若指被授权成员仓的\
             仓内相对路径（运行期兼容写法），plan 期判归属须改写为带成员目录前缀的\
             聚合根相对写法，如 {bound_prefix}/{anchor}）"
        ),
    })
}

/// 前缀包含判定：`anchor == prefix || anchor.starts_with("<prefix>/")`——
/// 与仓内既有前缀核对口径一致（不取 `busi-x` 误命中 `busi` 的字面前缀）。
fn path_within_prefix(anchor: &str, prefix: &str) -> bool {
    anchor == prefix || anchor.starts_with(&format!("{prefix}/"))
}

fn violation(work_item_id: &str, message: String) -> WorkItemSplitFinding {
    WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Error,
        code: WRITE_SCOPE_OUTSIDE_BOUND_REPOSITORY.to_string(),
        message,
        work_item_ids: vec![work_item_id.to_string()],
        // F1.2：preflight 族 finding 暂无 typed payload（None=Unknown 不
        // 自动；显式 kind 工厂归 F2b）。
        kind: None,
    }
}
