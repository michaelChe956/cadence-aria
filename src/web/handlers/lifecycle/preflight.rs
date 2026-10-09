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

/// 多仓 plan preflight 决策(add-multi-repo-issue-entry,REQ-WSC-08 修订):
/// 上界内子集放行 + involved 空回退「上界恰一仓」口径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlanRepositoryPreflightDecision {
    Eligible { repository_ids: Vec<String> },
    Ineligible { reason: String },
}

/// 多仓 plan 单候选 preflight(REQ-WSC-08 修订,design 关键点 0/1):
/// - involved ⊆ 上界且非空 → Eligible(含真子集——AI 在勾选上界内收敛);
/// - involved 空 → 回退「上界恰一仓」口径(缺陷#7:单成员上界旧行为通过,
///   多成员上界确定性拒,reason 沿用旧措辞使 durable 失败消息可对照);
/// - involved 含上界外成员 → Ineligible(reason 指名界外仓)。
///
/// 上界由 [`logical_repository_upper_bound_for_plan`] 产出(focus 非空取勾选
/// 原集;空取 resolved 有效成员集——绝不以空集为上界)。
pub(crate) fn preflight_plan_repository_candidates(
    upper_bound: &[String],
    involved_repository_ids: &[String],
) -> PlanRepositoryPreflightDecision {
    let bound = upper_bound.iter().collect::<BTreeSet<_>>();
    if involved_repository_ids.is_empty() {
        // involved 空回退「上界恰一仓」口径(缺陷#7 保留):单成员上界旧行为
        // 通过;多成员/空上界确定性拒,reason 沿用旧措辞使 durable 失败消息
        // 与既有测试断言可对照。
        return match upper_bound {
            [repository_id] => PlanRepositoryPreflightDecision::Eligible {
                repository_ids: vec![repository_id.clone()],
            },
            _ => PlanRepositoryPreflightDecision::Ineligible {
                reason: format!(
                    "single-candidate preflight requires exactly one logical repository; found {}",
                    upper_bound.len()
                ),
            },
        };
    }
    let out_of_bound = involved_repository_ids
        .iter()
        .filter(|repository_id| !bound.contains(*repository_id))
        .collect::<Vec<_>>();
    if !out_of_bound.is_empty() {
        // 界外即拒(勾选集=硬上界):reason 指名界外仓,诉求走修订反馈。
        return PlanRepositoryPreflightDecision::Ineligible {
            reason: format!(
                "work-item-plan preflight requires design involved repositories within the \
                 selection upper bound; out-of-bound: {}",
                out_of_bound
                    .iter()
                    .map(|repository_id| repository_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
    }
    // involved ⊆ 上界且非空(含真子集)→ 放行;保序去重输出 eligible 仓集。
    let mut repository_ids = Vec::new();
    for repository_id in involved_repository_ids {
        if !repository_ids.contains(repository_id) {
            repository_ids.push(repository_id.clone());
        }
    }
    PlanRepositoryPreflightDecision::Eligible { repository_ids }
}

/// plan preflight 上界(design 关键点 0):focus 非空取勾选原集;空取
/// resolved 有效成员集(AllMembers 历史语义=manifest 全成员,Explicit=
/// include − exclude)——与 design 钉定/出生值/write-back 四面同源引用
/// `IssueCodebaseSelection::resolved_upper_bound`,绝不在 preflight 面自算
/// 第二套 focus 判定。manifest 成员过滤与 String 归一由本包装完成:陈旧
/// 引用 fail-closed 收窄,绝不放宽上界。
pub(crate) fn logical_repository_upper_bound_for_plan(
    manifest: &LogicalCodebaseManifest,
    selection: &IssueCodebaseSelection,
) -> Vec<String> {
    let resolved = match selection.selection_policy {
        SelectionPolicy::AllMembers => manifest.member_ids.clone(),
        SelectionPolicy::Explicit => selection.resolve_effective_members(),
    };
    let bound = selection.resolved_upper_bound(&resolved);
    let manifest_ids = manifest.member_ids.iter().collect::<BTreeSet<_>>();
    bound
        .into_iter()
        .filter(|repository_id| manifest_ids.contains(repository_id))
        .map(|repository_id| repository_id.0.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod multi_repo_preflight_tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// 六象限①(design 点 1/REQ-WSC-08 修订):involved ⊆ 上界且非空 → 放行,
    /// 真子集与全集同过——多仓 plan 的入口闸由此打开。
    #[test]
    fn quadrant_1_non_empty_involved_subset_of_upper_bound_passes() {
        // 全集(involved = 勾选上界全成员)
        assert_eq!(
            preflight_plan_repository_candidates(&ids(&["api", "web"]), &ids(&["api", "web"])),
            PlanRepositoryPreflightDecision::Eligible {
                repository_ids: ids(&["api", "web"]),
            }
        );
        // 真子集(AI 在勾选上界内收敛到更小集合)
        assert_eq!(
            preflight_plan_repository_candidates(
                &ids(&["api", "gateway", "web"]),
                &ids(&["api", "web"])
            ),
            PlanRepositoryPreflightDecision::Eligible {
                repository_ids: ids(&["api", "web"]),
            }
        );
        // 勾 N 用 1:单仓 involved 在多仓上界内同样过
        assert_eq!(
            preflight_plan_repository_candidates(&ids(&["api", "web"]), &ids(&["api"])),
            PlanRepositoryPreflightDecision::Eligible {
                repository_ids: ids(&["api"]),
            }
        );
    }

    /// 六象限②(design 点 1,缺陷#7 口径保留):involved 空 → 回退「上界恰一仓」:
    /// 单成员上界旧行为通过,多成员上界确定性拒。
    #[test]
    fn quadrant_2_empty_involved_falls_back_to_exactly_one_upper_bound_member() {
        assert_eq!(
            preflight_plan_repository_candidates(&ids(&["api"]), &[]),
            PlanRepositoryPreflightDecision::Eligible {
                repository_ids: ids(&["api"]),
            }
        );
        assert_eq!(
            preflight_plan_repository_candidates(&ids(&["api", "web"]), &[]),
            PlanRepositoryPreflightDecision::Ineligible {
                reason:
                    "single-candidate preflight requires exactly one logical repository; found 2"
                        .to_string(),
            }
        );
    }

    /// 六象限③(design 点 0):focus=∅ 存量 AllMembers selection 走 resolved 上界
    /// (manifest 全成员历史语义)放行——绝不以空集为上界,存量链零回归;
    /// involved 非空子集在 resolved 上界内即过(旧恰一仓闸的缺口修复)。
    #[test]
    fn quadrant_3_all_members_empty_focus_uses_resolved_upper_bound() {
        // resolved 上界 = manifest 全成员(由 logical_repository_upper_bound_for_plan
        // 产出,判定面只认上界集)
        let resolved_upper_bound = ids(&["api", "gateway", "web"]);
        assert_eq!(
            preflight_plan_repository_candidates(&resolved_upper_bound, &ids(&["api", "web"])),
            PlanRepositoryPreflightDecision::Eligible {
                repository_ids: ids(&["api", "web"]),
            }
        );
        // involved 空且 resolved 多成员:回退恰一仓口径拒——存量 AllMembers
        // 多成员 issue 的空 involved Design 与旧版一致确定性拒。
        assert!(matches!(
            preflight_plan_repository_candidates(&resolved_upper_bound, &[]),
            PlanRepositoryPreflightDecision::Ineligible { .. }
        ));
    }

    /// 六象限④:involved 含上界外成员 → 确定性拒,reason 指名界外仓。
    #[test]
    fn quadrant_4_involved_outside_upper_bound_is_rejected() {
        let decision =
            preflight_plan_repository_candidates(&ids(&["api", "web"]), &ids(&["api", "mobile"]));
        match decision {
            PlanRepositoryPreflightDecision::Ineligible { reason } => {
                assert!(
                    reason.contains("mobile"),
                    "reason must name out-of-bound repositories: {reason}"
                );
                assert!(
                    reason.contains("upper bound"),
                    "reason must state the bound rule: {reason}"
                );
            }
            other => panic!("out-of-bound involved must be ineligible: {other:?}"),
        }
        // 界外 + 空上界(fail-closed:resolved 空时任何 involved 都越界)
        assert!(matches!(
            preflight_plan_repository_candidates(&[], &ids(&["api"])),
            PlanRepositoryPreflightDecision::Ineligible { .. }
        ));
    }

    /// 六象限⑤(design 点 1 单仓零回归):单成员上界下新判定与旧恰一仓语义等价。
    #[test]
    fn quadrant_5_single_member_upper_bound_matches_legacy_exactly_one() {
        // involved 非空且 = 上界唯一成员:新旧同过
        assert_eq!(
            preflight_plan_repository_candidates(&ids(&["api"]), &ids(&["api"])),
            PlanRepositoryPreflightDecision::Eligible {
                repository_ids: ids(&["api"]),
            }
        );
        // involved 空:回退恰一仓 → 过,等价旧函数对单成员集的 Eligible
        assert_eq!(
            preflight_plan_repository_candidates(&ids(&["api"]), &[]),
            PlanRepositoryPreflightDecision::Eligible {
                repository_ids: ids(&["api"]),
            }
        );
        assert_eq!(
            preflight_single_repository_candidate(&ids(&["api"])),
            SingleCandidatePreflightDecision::Eligible {
                repository_id: "api".to_string(),
            }
        );
    }

    /// 六象限⑥:多成员上界 + 空 involved → 确定性拒(回退口径在多成员上界下
    /// 唯一结果即拒,reason 沿用旧措辞使 durable 失败消息可对照)。
    #[test]
    fn quadrant_6_multi_member_upper_bound_with_empty_involved_rejects() {
        assert_eq!(
            preflight_plan_repository_candidates(&ids(&["api", "gateway", "web"]), &[]),
            PlanRepositoryPreflightDecision::Ineligible {
                reason:
                    "single-candidate preflight requires exactly one logical repository; found 3"
                        .to_string(),
            }
        );
    }

    /// 上界包装(design 点 0 全矩阵,六象限③的上界半边):focus=∅ 存量
    /// AllMembers → resolved=manifest 全成员(绝不以空集为上界);focus 非空
    /// → 勾选原集(include 更大不放宽);Explicit focus=∅ → include−exclude;
    /// 陈旧勾选引用 manifest 外成员 → fail-closed 过滤收窄。
    #[test]
    fn upper_bound_resolves_focus_and_resolved_members_per_design_point_0() {
        use crate::product::logical_codebase::LogicalCodebaseManifest;
        use crate::product::logical_codebase::LogicalRepositoryId;
        use std::path::PathBuf;

        fn logical_id(seed: u128) -> LogicalRepositoryId {
            LogicalRepositoryId(uuid::Uuid::from_u128(seed))
        }
        fn id_strings(seeds: &[u128]) -> Vec<String> {
            seeds
                .iter()
                .map(|seed| logical_id(*seed).0.to_string())
                .collect()
        }
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            PathBuf::from("/tmp/preflight-upper-bound-test"),
            vec![logical_id(1), logical_id(2), logical_id(3)],
        );

        // focus=∅ + AllMembers(存量 LC issue 全量形态):上界=manifest 全成员
        let all_members = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        assert_eq!(
            logical_repository_upper_bound_for_plan(&manifest, &all_members),
            id_strings(&[1, 2, 3])
        );

        // focus=∅ + Explicit:上界=include − exclude(resolve_effective_members 同源)
        let explicit_no_focus = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![logical_id(1), logical_id(2), logical_id(3)],
            vec![logical_id(2)],
            vec![],
            None,
        );
        assert_eq!(
            logical_repository_upper_bound_for_plan(&manifest, &explicit_no_focus),
            id_strings(&[1, 3])
        );

        // focus 非空:上界=勾选原集(include 三成员不放宽到全成员)
        let explicit_focus = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![logical_id(1), logical_id(2), logical_id(3)],
            vec![],
            vec![logical_id(1), logical_id(3)],
            None,
        );
        assert_eq!(
            logical_repository_upper_bound_for_plan(&manifest, &explicit_focus),
            id_strings(&[1, 3])
        );

        // focus 引用 manifest 外成员(陈旧勾选):过滤收窄,绝不放宽
        let stale_focus = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![logical_id(1), logical_id(2), logical_id(3), logical_id(9)],
            vec![],
            vec![logical_id(3), logical_id(9)],
            None,
        );
        assert_eq!(
            logical_repository_upper_bound_for_plan(&manifest, &stale_focus),
            id_strings(&[3])
        );
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
