//! 单候选 preflight（自 `handlers/lifecycle.rs` 拆出——1200 行守护，WP6 关闸）：
//! 纯移动零语义变化。这个函数族刻意不使用 `PlanningContextSetResolver`：后者
//! 会在发现失效成员时写入 invalidation，因此不能置于创建 SingleCandidate
//! session 前的只读分流边界（见下述 enum doc）。

use std::collections::BTreeSet;

use crate::product::logical_codebase::{
    IssueCodebaseSelection, LogicalCodebaseManifest, SelectionPolicy,
};

/// 只基于已加载的 logical codebase manifest/selection 做单候选 preflight。
///
/// 这个函数刻意不使用 `PlanningContextSetResolver`：后者会在发现失效成员时写入
/// invalidation，因此不能置于创建 SingleCandidate session 前的只读分流边界。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SingleCandidatePreflightDecision {
    Eligible {
        repository_id: String,
    },
    /// L2 退役（T5/REQ-WSC-08/D3）：legacy fallback 路径已删除——preflight 失败
    /// 一律收敛新路径 durable 终态（含原因），无 flow_kind 切换目标。
    Ineligible {
        reason: String,
    },
}

pub(crate) fn preflight_single_repository_candidate(
    repository_ids: &[String],
) -> SingleCandidatePreflightDecision {
    match repository_ids {
        [repository_id] => SingleCandidatePreflightDecision::Eligible {
            repository_id: repository_id.clone(),
        },
        _ => SingleCandidatePreflightDecision::Ineligible {
            reason: format!(
                "single-candidate preflight requires exactly one logical repository; found {}",
                repository_ids.len()
            ),
        },
    }
}

pub(crate) fn logical_repository_ids_for_preflight(
    manifest: &LogicalCodebaseManifest,
    selection: &IssueCodebaseSelection,
) -> Vec<String> {
    let selected_ids = match selection.selection_policy {
        SelectionPolicy::AllMembers => manifest.member_ids.clone(),
        SelectionPolicy::Explicit => selection.resolve_effective_members(),
    };
    let manifest_ids = manifest.member_ids.iter().collect::<BTreeSet<_>>();
    selected_ids
        .into_iter()
        .filter(|repository_id| manifest_ids.contains(repository_id))
        .map(|repository_id| repository_id.0.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
