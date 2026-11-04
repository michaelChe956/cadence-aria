//! REQ-WSC-10（写范围×被授权仓归属）：多仓（LC 聚合根）plan 会话的成员
//! checkout 前缀表装载与候选校验门控（三条生产校验路径共享——author
//! 权威落盘 / 门内人工修订 / 运行期预校验，接线收敛于此，不逐点复制）。

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::repository_routing::RepositoryRouting;
use crate::product::logical_codebase::{
    IssueCodebaseSelectionStore, LogicalCodebaseStore, MemberStatus,
};
use crate::product::models::automation::EnrollmentBindingIdentity;
use crate::product::models::lifecycle::IssueWorkItemPlan;

/// 候选校验用成员 checkout 前缀表（与 `validation_member_targets` 同门控）：
/// enrolled 绑定在场才装载；装载失败 `Err`（调用方按各自路径既有形态映射
/// ——compile 失败 / operational 停点 / 预检诊断，不以 None 吞错）；单仓/
/// 非 enrolled 返回 `None`（写范围归属核对不触发，零回归）。
pub(crate) fn for_validation(
    lifecycle: &LifecycleStore,
    plan: &IssueWorkItemPlan,
    enrollment_binding: Option<&EnrollmentBindingIdentity>,
) -> Result<Option<std::collections::BTreeMap<String, String>>, String> {
    if enrollment_binding.is_none() {
        return Ok(None);
    }
    plan_member_checkout_prefixes(lifecycle, plan)
}

/// 成员 checkout 前缀表——物理成员仓 ID → 该成员 checkout 相对聚合根
///（manifest `provider_context_root`）的目录（如 `busi`；空串＝成员
/// checkout 即聚合根本身）。
/// - 无 manifest → `Ok(None)`：无聚合根，归属核对不触发（零回归）；
/// - 路由不成对（FailClosed，含无归属旧数据）→ `Err`（与 target 解析同口径，
///   绝不静默回退）；
/// - 仅 active 成员（Removed/Tombstoned 成员的原 checkout 目录按聚合根
///   非成员区处理——不在任何授权面内）；
/// - 每成员 Main checkout 恰一（与 `cross_target_check` 的
///   `resolve_active_member_main_checkouts` 同窗口口径）；`canonical_path`
///   相对 `provider_context_root`（strip_prefix＋canonicalize 兜底，与
///   gates 的 `scope_member_prefix` 同源）；不在聚合根下 → `Err`
///   fail-closed（绝不把域外 checkout 折算成前缀）。
pub(crate) fn plan_member_checkout_prefixes(
    lifecycle: &LifecycleStore,
    plan: &IssueWorkItemPlan,
) -> Result<Option<std::collections::BTreeMap<String, String>>, String> {
    let paths = lifecycle.app_paths();
    let lc_id = crate::product::logical_codebase::RepositoryAuthorityResolver::new(paths.clone())
        .resolve_for_issue(&plan.project_id, &plan.issue_id)
        .map_err(|error| format!("resolve issue logical codebase failed: {error}"))?
        .and_then(|resolution| resolution.target.logical_codebase_id);
    let logical_store = match lc_id.as_deref() {
        Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
        None => LogicalCodebaseStore::new(paths.clone()),
    };
    let manifest = logical_store
        .load_manifest(&plan.project_id)
        .map_err(|error| format!("load logical codebase manifest failed: {error}"))?;
    let Some(manifest) = manifest else {
        return Ok(None);
    };
    let selection_store = match lc_id.as_deref() {
        Some(lc_id) => IssueCodebaseSelectionStore::for_lc(paths.clone(), lc_id),
        None => IssueCodebaseSelectionStore::new(paths.clone()),
    };
    let selection = selection_store
        .load(&plan.project_id, &plan.issue_id)
        .map_err(|error| format!("load issue codebase selection failed: {error}"))?;
    let (manifest, _selection) = match RepositoryRouting::classify(Some(manifest), selection) {
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => (manifest, *selection),
        RepositoryRouting::FailClosed { code, .. } => {
            return Err(format!(
                "{}: logical codebase manifest and issue selection must both exist",
                code.stable_code()
            ));
        }
    };
    let root = manifest.provider_context_root.clone();
    let checkouts = logical_store
        .list_checkouts(&plan.project_id)
        .map_err(|error| format!("list logical codebase checkouts failed: {error}"))?;
    let mut prefixes = std::collections::BTreeMap::new();
    for member_id in &manifest.member_ids {
        let member = logical_store
            .load_member(&plan.project_id, *member_id)
            .map_err(|error| format!("load logical codebase member failed: {error}"))?
            .ok_or_else(|| {
                format!("write_scope_prefix_unresolved: member {member_id:?} missing from registry")
            })?;
        if member.status != MemberStatus::Active {
            continue;
        }
        let main_checkouts: Vec<_> = checkouts
            .iter()
            .filter(|checkout| {
                checkout.logical_repository_id == *member_id
                    && checkout.kind == crate::product::logical_codebase::CheckoutKind::Main
            })
            .collect();
        let [checkout] = main_checkouts.as_slice() else {
            return Err(format!(
                "write_scope_prefix_unresolved: member {member_id:?} must have exactly one main checkout"
            ));
        };
        let relative = checkout
            .canonical_path
            .strip_prefix(&root)
            .ok()
            .map(std::path::Path::to_path_buf)
            .or_else(|| {
                let root = root.canonicalize().ok()?;
                let member = checkout.canonical_path.canonicalize().ok()?;
                member
                    .strip_prefix(&root)
                    .ok()
                    .map(std::path::Path::to_path_buf)
            })
            .ok_or_else(|| {
                format!(
                    "write_scope_prefix_unresolved: member checkout {} is outside provider_context_root {}",
                    checkout.canonical_path.display(),
                    root.display()
                )
            })?;
        let prefix = relative.to_string_lossy().replace('\\', "/");
        let prefix = prefix.trim_matches('/').to_string();
        prefixes.insert(member.physical_repository_id.clone(), prefix);
    }
    Ok(Some(prefixes))
}
