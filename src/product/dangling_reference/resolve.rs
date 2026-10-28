//! 目标存在性核对与证据分级（REQ-DRG-01 / 02 / 12 / 21）。
//!
//! 本模块回答两层问题：
//! 1. 「目标还在吗」——[`DanglingResolver::target_exists`] 按九类目标分派到
//!    既有只读 API 做存在性判定（REQ-DRG-01，三态：`Exists` / `Missing` /
//!    `Unknown`）；
//! 2. 「能证明已删到可销账吗」——[`DanglingResolver::evidence_level`] 按
//!    L1 墓碑四条 / 类型专属 L2 条件给出证据等级（REQ-DRG-02）。
//!
//! 读失败（IO / 解析 / 歧义）一律原样上抛，MUST NOT 被当成「目标不存在」
//! （REQ-DRG-21）；所需存在性数据源 / 作用域在本判定面不可用的目标（U1 四类）
//! 如实报 [`TargetExistence::Unknown`]，同样 MUST NOT 被当成「目标不存在」
//! （REQ-DRG-23）。「已判定悬空但证据不足」归 [`EvidenceLevel::Insufficient`]
//! （只报不删残留），与巡检侧的「不可判定」（读取失败或数据源 / 作用域不可用）
//! 是两个概念。
//!
//! 本任务只交付判定面：不删除任何东西、不接线巡检与销账（Task 4/7/8）。

use std::collections::BTreeSet;
use std::path::Path;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::dangling_reference::reference_table::DanglingTargetKind;
use crate::product::entity_deletion_cascade;
use crate::product::entity_deletion_store::{
    DeletionEntityKind, DeletionPhase, EntityDeletionStore, EntityScope,
    issue_entity_operations_root,
};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::reviewer_window_store::{
    REVIEWER_WINDOW_STORE_DIR, ReviewerWindowRecord, tree_key_of,
};

/// 目标存在性判定（REQ-DRG-01 / REQ-DRG-23）：只回答「仍在 / 不在 / 无法判定」，
/// 不掺删除语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetExistence {
    /// 目标记录仍在（存在性读命中）。
    Exists,
    /// 目标记录不在（存在性读**明确**未命中；读失败不落此态，原样上抛）。
    Missing,
    /// 目标存在性无法判定（所需数据源未接或作用域不足）：MUST 归「不可判定」，
    /// MUST NOT 计入悬空条目数（REQ-DRG-23）。
    Unknown,
}

/// 已删证据等级（REQ-DRG-02）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvidenceLevel {
    /// L1：墓碑四条同时成立（见 [`DanglingResolver::evidence_level`]）。
    L1,
    /// L2：该类型专属可算条件全部成立（仅 Attempt，经窗口判定面 W-1..W-4）。
    L2,
    /// 证据不足：已判定悬空但证据不够 → 只报不删残留（区别于「不可判定」）。
    Insufficient,
}

/// 销账等级（REQ-DRG-02 附表，销账白名单的单一事实源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeletableLevel {
    /// 只认 L1 墓碑证据（story_spec / design_spec / plan）。
    L1,
    /// 该类型专属 L2 条件全成立才可删（attempt）。
    L2,
    /// 只报不删（session / checkout / aggregate_index / logical_member；enrollment 永不删）。
    ReportOnly,
}

/// 证据等级表一行：目标类型 → 可删档位 + L2 专属条件（REQ-DRG-02 附表逐行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetEvidencePolicy {
    pub(crate) target_kind: DanglingTargetKind,
    pub(crate) deletable_level: DeletableLevel,
    /// L2 专属条件的可读描述（供巡检清单与销账闸引用）；非 L2 档位为空串。
    pub(crate) l2_condition: &'static str,
}

/// 证据等级表（REQ-DRG-02 附表的单一事实源）：九类目标逐行登记。
const EVIDENCE_POLICIES: [TargetEvidencePolicy; 9] = [
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::StorySpec,
        deletable_level: DeletableLevel::L1,
        l2_condition: "",
    },
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::DesignSpec,
        deletable_level: DeletableLevel::L1,
        l2_condition: "",
    },
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Plan,
        deletable_level: DeletableLevel::L1,
        l2_condition: "",
    },
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Session,
        deletable_level: DeletableLevel::ReportOnly,
        l2_condition: "",
    },
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Attempt,
        deletable_level: DeletableLevel::L2,
        l2_condition: "终态 + 无租约 + 两路目标皆无 + worktree 已不存在",
    },
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Checkout,
        deletable_level: DeletableLevel::ReportOnly,
        l2_condition: "",
    },
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::AggregateIndex,
        deletable_level: DeletableLevel::ReportOnly,
        l2_condition: "",
    },
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::LogicalMember,
        deletable_level: DeletableLevel::ReportOnly,
        l2_condition: "",
    },
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Enrollment,
        deletable_level: DeletableLevel::ReportOnly,
        l2_condition: "",
    },
];

/// 查证据等级表：九类目标必须全部登记，查无此行属编程错误（fail-fast）。
pub(crate) fn evidence_policy(target_kind: DanglingTargetKind) -> &'static TargetEvidencePolicy {
    EVIDENCE_POLICIES
        .iter()
        .find(|policy| policy.target_kind == target_kind)
        .expect("证据等级表必须覆盖全部九类目标（REQ-DRG-02 附表）")
}

/// attempt 路一探针：`(attempt_id) -> 是否命中`。
///
/// 读失败（IO / 解析 / 歧义）MUST 以 `Err` 上抛，不得返回 `Ok(false)` 冒充
/// 「不存在」（REQ-DRG-21）。
pub(crate) type AttemptProbe<'a> = &'a dyn Fn(&str) -> Result<bool, ProductStoreError>;

/// 目标存在性核对器：按九类目标分派既有只读 API（REQ-DRG-01 / 21）。
///
/// `session_ids` 是**规划面** workspace session 的全量 id 集合，MUST 由调用方
/// 「枚举」构建（遍历状态根下全部 project × issue 调
/// `LifecycleStore::list_workspace_sessions`；REQ-DRG-21 路二），MUST NOT 用
/// `get_workspace_session`（读路径写回副作用）；枚举失败时调用方 MUST NOT
/// 拿残缺集合构造本结构，须整体归「不可判定」。
pub(crate) struct DanglingResolver<'a> {
    paths: &'a ProductAppPaths,
    session_ids: &'a BTreeSet<String>,
    /// 注入的路一探针（测试用）；`None` = 内置探针（`CodingAttemptStore`）。
    attempt_probe: Option<AttemptProbe<'a>>,
}

impl<'a> DanglingResolver<'a> {
    /// 以内置路一探针（`CodingAttemptStore::get_attempt_by_id`）构造核对器。
    pub(crate) fn new(paths: &'a ProductAppPaths, session_ids: &'a BTreeSet<String>) -> Self {
        Self {
            paths,
            session_ids,
            attempt_probe: None,
        }
    }

    /// 注入路一探针构造核对器（测试与巡检侧复用同一分派逻辑）。
    pub(crate) fn with_attempt_probe(
        paths: &'a ProductAppPaths,
        session_ids: &'a BTreeSet<String>,
        attempt_probe: AttemptProbe<'a>,
    ) -> Self {
        Self {
            paths,
            session_ids,
            attempt_probe: Some(attempt_probe),
        }
    }

    /// 九类目标的存在性判定（REQ-DRG-01；三态，见 [`TargetExistence`]）。
    ///
    /// - `StorySpec` / `DesignSpec`：`LifecycleStore::list_story_specs` /
    ///   `list_design_specs` 按 `scope` 的 project/issue 列举比对 id；
    /// - `Plan`：`entity_deletion_cascade::list_live_plan_ids`（legacy 与
    ///   schema-v2 两路）；
    /// - `Session`：路二枚举集合成员关系；
    /// - `Attempt`：两路核对（REQ-DRG-21）——路一 coding attempt 探针，
    ///   路二 `session_ids` 集合成员关系；**任一路命中即判「仍在」**，
    ///   两路皆不命中才算不存在；任一路读失败原样上抛（不可判定）；
    /// - `Checkout` / `AggregateIndex` / `LogicalMember` / `Enrollment`：
    ///   存在性数据源需要本判定面没有的作用域（U1）——如实报
    ///   [`TargetExistence::Unknown`]（REQ-DRG-23），MUST NOT 伪装成
    ///   [`TargetExistence::Missing`]；其 [`Self::evidence_level`] 仍恒为
    ///   [`EvidenceLevel::Insufficient`]，销账侧 MUST NOT 据此删除。
    ///
    /// `Unknown` 与读失败上抛的共同口径：「不可判定」不计入悬空条目数，
    /// 也不计入归零（REQ-DRG-22 / REQ-DRG-23）。
    ///
    /// `scope` 缺失时对需要 project/issue 定位的目标返回
    /// `ProductStoreError::NotFound`（不得猜默认 scope）。
    pub(crate) fn target_exists(
        &self,
        kind: DanglingTargetKind,
        target_id: &str,
        scope: Option<&EntityScope>,
    ) -> Result<TargetExistence, ProductStoreError> {
        match kind {
            DanglingTargetKind::StorySpec => {
                let scope = require_scope(target_id, scope)?;
                let hit = LifecycleStore::new(self.paths.clone())
                    .list_story_specs(&scope.project_id, &scope.issue_id)?
                    .iter()
                    .any(|record| record.id == target_id);
                Ok(existence(hit))
            }
            DanglingTargetKind::DesignSpec => {
                let scope = require_scope(target_id, scope)?;
                let hit = LifecycleStore::new(self.paths.clone())
                    .list_design_specs(&scope.project_id, &scope.issue_id)?
                    .iter()
                    .any(|record| record.id == target_id);
                Ok(existence(hit))
            }
            DanglingTargetKind::Plan => {
                let scope = require_scope(target_id, scope)?;
                let hit = entity_deletion_cascade::list_live_plan_ids(
                    self.paths,
                    &scope.project_id,
                    &scope.issue_id,
                )?
                .iter()
                .any(|plan_id| plan_id == target_id);
                Ok(existence(hit))
            }
            DanglingTargetKind::Session => Ok(existence(self.session_ids.contains(target_id))),
            DanglingTargetKind::Attempt => {
                // 路一：coding attempt（内置探针或注入探针）。
                let coding_hit = match self.attempt_probe {
                    Some(probe) => probe(target_id)?,
                    None => coding_attempt_exists(self.paths, target_id)?,
                };
                // 路二：规划面 workspace session 枚举集合。任一命中即「仍在」。
                Ok(existence(
                    coding_hit || self.session_ids.contains(target_id),
                ))
            }
            // U1：checkout / aggregate_index / logical_member 的存在性读需要 LC
            // 作用域（记录落在 `logical-codebases/<lc_id>/`），本判定面无 LC
            // 上下文；enrollment 需按 project/issue 读当处文件。三者一律如实报
            // 「无法判定」，MUST NOT 伪装成「不存在」（否则 Task 4 会把正常记录
            // 整批判为悬空）。真实读须先按 `LogicalCodebaseStore::list(project_id)`
            // 枚举 LC，属持有 LC 上下文的 Task 4（REQ-DRG-23）。
            DanglingTargetKind::Checkout
            | DanglingTargetKind::AggregateIndex
            | DanglingTargetKind::LogicalMember
            | DanglingTargetKind::Enrollment => Ok(TargetExistence::Unknown),
        }
    }

    /// 目标的已删证据等级（REQ-DRG-02）。
    ///
    /// - 仅 `StorySpec` / `DesignSpec` / `Plan` 可判 L1：墓碑四条同时成立
    ///   （墓碑存在 + `phase == Deleted` + `deleted_at` 有值 + 无 pending
    ///   failure 文件；`purging` / 缺完成时间 / 带待人工处置失败事实一律
    ///   不算，见 [`Self::l1_tombstone_holds`]）；任一条不成立 →
    ///   [`EvidenceLevel::Insufficient`]（不是「不可判定」）；
    /// - `Attempt`（L2 档）的专属条件 = W-1..W-4，需要该 attempt 名下**全部**
    ///   窗口记录，由 [`window_l2_verdict`] 逐窗判定 + 销账侧按 attempt 聚合
    ///   （REQ-DRG-12 / Task 7）；本入口无窗口簇上下文，条件无法核实 →
    ///   不判 L2，恒 [`EvidenceLevel::Insufficient`]；
    /// - 只报不删档位（含 U1 降级四类与 enrollment）恒 [`EvidenceLevel::Insufficient`]。
    pub(crate) fn evidence_level(
        &self,
        kind: DanglingTargetKind,
        target_id: &str,
        scope: Option<&EntityScope>,
    ) -> Result<EvidenceLevel, ProductStoreError> {
        match evidence_policy(kind).deletable_level {
            DeletableLevel::L1 => {
                let caller_scope = require_scope(target_id, scope)?;
                let deletion_scope = tombstone_scope(kind, target_id, caller_scope);
                Ok(if self.l1_tombstone_holds(&deletion_scope)? {
                    EvidenceLevel::L1
                } else {
                    EvidenceLevel::Insufficient
                })
            }
            DeletableLevel::L2 | DeletableLevel::ReportOnly => Ok(EvidenceLevel::Insufficient),
        }
    }

    /// L1 四条（REQ-DRG-02）：全部成立才返回 `true`。
    ///
    /// 1. 墓碑存在（`EntityDeletionStore::get`，纯读、不取 lease；读失败上抛）；
    /// 2. `phase == Deleted`（`purging` 迁移中不算）；
    /// 3. `deleted_at` 有值（缺完成时间不算）；
    /// 4. 无 `{issue_root}/entity-operations/pending-failures/{kind}/{id}.json`
    ///    （`issue_entity_operations_root` 拼路径；只判存在、不读内容、不持 lease）。
    fn l1_tombstone_holds(&self, scope: &EntityScope) -> Result<bool, ProductStoreError> {
        // 第 1 条：墓碑存在（`get` 对无墓碑返回 Ok(None)，真实 IO/JSON 错误上抛）。
        let Some(record) = EntityDeletionStore::new(self.paths.clone()).get(scope)? else {
            return Ok(false);
        };
        // 第 2 条：完成相位。
        if record.phase != DeletionPhase::Deleted {
            return Ok(false);
        }
        // 第 3 条：完成时间有值。
        if record.deleted_at.is_none() {
            return Ok(false);
        }
        // 第 4 条：无待人工处置的删除失败事实（存在性判定用 path_exists，
        // stat 失败上抛而非当成「无失败事实」）。
        let pending_failure = issue_entity_operations_root(self.paths, scope)?
            .join("pending-failures")
            .join(scope.kind.path_segment())
            .join(format!("{}.json", scope.entity_id));
        Ok(!path_exists(&pending_failure)?)
    }
}

/// 需要 project/issue 定位的目标缺 `scope`：显式 NotFound（不得猜默认 scope）。
fn require_scope<'a>(
    target_id: &str,
    scope: Option<&'a EntityScope>,
) -> Result<&'a EntityScope, ProductStoreError> {
    scope.ok_or_else(|| ProductStoreError::NotFound {
        kind: "entity_scope",
        id: target_id.to_string(),
    })
}

/// 布尔命中 → 两态判定。
fn existence(hit: bool) -> TargetExistence {
    if hit {
        TargetExistence::Exists
    } else {
        TargetExistence::Missing
    }
}

/// 目标类型 → 墓碑 scope：kind 按目标类型映射（StorySpec→Story 等），
/// project/issue 取调用方 scope，entity_id 用被核对的 target_id。
fn tombstone_scope(kind: DanglingTargetKind, target_id: &str, scope: &EntityScope) -> EntityScope {
    let deletion_kind = match kind {
        DanglingTargetKind::StorySpec => DeletionEntityKind::Story,
        DanglingTargetKind::DesignSpec => DeletionEntityKind::Design,
        // 本函数仅在 L1 档（StorySpec/DesignSpec/Plan）被调用。
        _ => DeletionEntityKind::Plan,
    };
    EntityScope::new(
        scope.project_id.clone(),
        scope.issue_id.clone(),
        deletion_kind,
        target_id.to_string(),
    )
}

/// 内置路一探针：coding attempt 存储命中 → `true`；`NotFound` → `false`；
/// 其余错误（含 `Ambiguous` 歧义与真实 IO/解析错误）原样上抛——
/// 读失败 MUST NOT 被当成「目标不存在」（REQ-DRG-21）。
fn coding_attempt_exists(
    paths: &ProductAppPaths,
    attempt_id: &str,
) -> Result<bool, ProductStoreError> {
    match CodingAttemptStore::new(paths.clone()).get_attempt_by_id(attempt_id) {
        Ok(_) => Ok(true),
        Err(ProductStoreError::NotFound { .. }) => Ok(false),
        Err(other) => Err(other),
    }
}

/// 存在性判定（stat）：IO 失败上抛为 `ProductStoreError::Io`，
/// MUST NOT 被当成「不存在 / 已删除」。
fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("stat {}: {error}", path.display())))
}

/// 窗口 L2 判定面（REQ-DRG-12 的 W-1..W-4）：四条件**全部从窗口记录自身可算**，
/// 不依赖 plan / 上游可达性（无数据源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowL2Verdict {
    /// W-1：窗口处于终态（ClosedInvalidated / NewReviewRequired / ClosedAccepted）。
    pub(crate) terminal: bool,
    /// W-2：无树级 durable 租约持有（`<root>/reviewer-windows/leases/{tree_key}.json`
    /// 不存在）。
    pub(crate) no_lease: bool,
    /// W-3：attempt 两路核对皆不命中（由调用方经
    /// [`DanglingResolver::target_exists`] 核对后传入，本函数不重复读存储）。
    pub(crate) target_missing: bool,
    /// W-4：canonical worktree 路径已不存在。
    pub(crate) worktree_gone: bool,
}

impl WindowL2Verdict {
    /// W-1..W-4 是否全部成立（REQ-DRG-12：任一不满足即整窗不可按 L2 销账）。
    pub(crate) fn all_hold(&self) -> bool {
        self.terminal && self.no_lease && self.target_missing && self.worktree_gone
    }

    /// 四条件的可读摘要（供巡检清单「已删证据」列使用）。
    pub(crate) fn summary(&self) -> String {
        format!(
            "W-1 终态：{}；W-2 无租约：{}；W-3 两路目标皆无：{}；W-4 worktree 已不存在：{}",
            hold_mark(self.terminal),
            hold_mark(self.no_lease),
            hold_mark(self.target_missing),
            hold_mark(self.worktree_gone)
        )
    }
}

fn hold_mark(holds: bool) -> &'static str {
    if holds { "成立" } else { "不成立" }
}

/// 单个窗口的 L2 判定（REQ-DRG-12 W-1..W-4 判定面）。
///
/// W-3（attempt 两路目标皆无）由调用方先行核对并以 `target_missing` 传入：
/// 本函数只消费窗口记录自身与状态根下的租约文件 / worktree 路径，**全部
/// 条件从窗口记录自身可算**，不依赖 plan / 上游可达性。
pub(crate) fn window_l2_verdict(
    paths: &ProductAppPaths,
    window: &ReviewerWindowRecord,
    target_missing: bool,
) -> Result<WindowL2Verdict, ProductStoreError> {
    // W-1：终态（窗口记录自身的状态机相位）。
    let terminal = window.state.is_terminal();
    // W-2：树级 durable 租约文件不存在（存在即仍有租约持有；stat 失败上抛）。
    let lease_path = paths
        .root()
        .join(REVIEWER_WINDOW_STORE_DIR)
        .join("leases")
        .join(format!(
            "{}.json",
            tree_key_of(&window.identity.canonical_worktree)
        ));
    let no_lease = !path_exists(&lease_path)?;
    // W-4：canonical worktree 已不存在（stat 失败上抛，不当成「已不存在」）。
    let worktree_gone = !path_exists(&window.identity.canonical_worktree)?;
    Ok(WindowL2Verdict {
        terminal,
        no_lease,
        target_missing,
        worktree_gone,
    })
}

#[cfg(test)]
mod resolve_tests {
    use super::{
        DanglingResolver, EvidenceLevel, TargetExistence, evidence_policy, window_l2_verdict,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::dangling_reference::reference_table::DanglingTargetKind;
    use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
    use crate::product::reviewer_window_store::{ReviewerWindowRecord, tree_key_of};
    use std::collections::BTreeSet;
    use std::path::Path;
    use tempfile::TempDir;

    fn probe_false() -> impl Fn(&str) -> Result<bool, crate::product::json_store::ProductStoreError>
    {
        |_id: &str| Ok(false)
    }

    fn scope() -> EntityScope {
        EntityScope::new(
            "project_0001",
            "issue_0001",
            DeletionEntityKind::Story,
            "story_spec_0005",
        )
    }

    fn write_tombstone(root: &Path, phase: &str, deleted_at: Option<&str>) {
        let dir = root.join("projects/project_0001/issues/issue_0001/entity-deletions/story");
        std::fs::create_dir_all(&dir).unwrap();
        let mut record = serde_json::json!({
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "entity_kind": "story",
            "entity_id": "story_spec_0005",
            "phase": phase,
            "started_at": "2026-10-28T10:00:00Z",
        });
        if let Some(value) = deleted_at {
            record["deleted_at"] = serde_json::Value::String(value.to_string());
        }
        std::fs::write(
            dir.join("story_spec_0005.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    fn write_pending_failure(root: &Path) {
        let dir = root.join(
            "projects/project_0001/issues/issue_0001/entity-operations/pending-failures/story",
        );
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("story_spec_0005.json"), b"{}").unwrap();
    }

    #[test]
    fn dangling_resolver_l1_requires_all_four_conditions() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_tombstone(root, "deleted", Some("2026-10-28T10:05:00Z"));
        let paths = ProductAppPaths::new(root);
        let sessions = BTreeSet::new();
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);

        assert_eq!(
            resolver
                .evidence_level(
                    DanglingTargetKind::StorySpec,
                    "story_spec_0005",
                    Some(&scope())
                )
                .unwrap(),
            EvidenceLevel::L1
        );

        write_tombstone(root, "purging", None);
        assert_eq!(
            resolver
                .evidence_level(
                    DanglingTargetKind::StorySpec,
                    "story_spec_0005",
                    Some(&scope())
                )
                .unwrap(),
            EvidenceLevel::Insufficient
        );

        write_tombstone(root, "deleted", None);
        assert_eq!(
            resolver
                .evidence_level(
                    DanglingTargetKind::StorySpec,
                    "story_spec_0005",
                    Some(&scope())
                )
                .unwrap(),
            EvidenceLevel::Insufficient
        );

        write_tombstone(root, "deleted", Some("2026-10-28T10:05:00Z"));
        write_pending_failure(root);
        assert_eq!(
            resolver
                .evidence_level(
                    DanglingTargetKind::StorySpec,
                    "story_spec_0005",
                    Some(&scope())
                )
                .unwrap(),
            EvidenceLevel::Insufficient
        );
    }

    #[test]
    fn dangling_resolver_attempt_two_routes() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let sessions = BTreeSet::new();
        let coding_probe =
            |id: &str| -> Result<bool, crate::product::json_store::ProductStoreError> {
                Ok(id == "coding_attempt_hit")
            };
        let hit_coding = DanglingResolver::with_attempt_probe(&paths, &sessions, &coding_probe);
        assert_eq!(
            hit_coding
                .target_exists(DanglingTargetKind::Attempt, "coding_attempt_hit", None)
                .unwrap(),
            TargetExistence::Exists
        );
        assert_eq!(
            hit_coding
                .target_exists(DanglingTargetKind::Attempt, "coding_attempt_miss", None)
                .unwrap(),
            TargetExistence::Missing
        );

        let mut sessions = BTreeSet::new();
        sessions.insert("workspace_session_auto_hit".to_string());
        let miss_probe = probe_false();
        let hit_session = DanglingResolver::with_attempt_probe(&paths, &sessions, &miss_probe);
        assert_eq!(
            hit_session
                .target_exists(
                    DanglingTargetKind::Attempt,
                    "workspace_session_auto_hit",
                    None
                )
                .unwrap(),
            TargetExistence::Exists
        );
        assert_eq!(
            hit_session
                .target_exists(DanglingTargetKind::Attempt, "nothing", None)
                .unwrap(),
            TargetExistence::Missing
        );
    }

    #[test]
    fn dangling_window_l2_verdict_requires_all_four_conditions() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let worktree = temp.path().join("missing-worktree");
        let window: ReviewerWindowRecord = serde_json::from_value(serde_json::json!({
            "schema": "reviewer_window_v1",
            "window_id": "win_001",
            "generation": 1,
            "identity": {
                "canonical_worktree": worktree,
                "gitdir": temp.path().join("missing-worktree/.git"),
                "common_dir": temp.path().join("missing-worktree"),
                "lease_id": "lease_1",
            },
            "attempt_id": "coding_attempt_9f3",
            "run_id": "run_1",
            "provider": "codex",
            "state": "closed_accepted",
            "report_acceptance": "accepted",
            "platform_write_paths": [],
            "quiesced_scopes": [],
            "baseline": null,
            "restore": null,
            "recovery": null,
            "last_error": null,
            "violation_pending": null,
            "pre_restore_git_facts": [],
            "transitions": [],
            "created_at": "2026-10-24T00:00:00Z",
            "updated_at": "2026-10-24T00:00:00Z",
        }))
        .expect("window record");

        let verdict = window_l2_verdict(&paths, &window, true).expect("verdict");
        assert!(verdict.terminal, "closed_accepted 是终态");
        assert!(verdict.no_lease, "无租约文件即无租约持有");
        assert!(verdict.target_missing);
        assert!(verdict.worktree_gone, "worktree 路径必须不存在");
        assert!(verdict.all_hold());

        let leases = paths.root().join("reviewer-windows/leases");
        std::fs::create_dir_all(&leases).unwrap();
        std::fs::write(
            leases.join(format!("{}.json", tree_key_of(&worktree))),
            b"{}",
        )
        .unwrap();
        let verdict = window_l2_verdict(&paths, &window, true).expect("verdict");
        assert!(!verdict.no_lease, "租约文件存在即不满足 W-2");
        assert!(!verdict.all_hold());
    }

    #[test]
    fn dangling_resolver_attempt_probe_error_is_not_missing() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let sessions = BTreeSet::new();
        let failing_probe =
            |_id: &str| -> Result<bool, crate::product::json_store::ProductStoreError> {
                Err(crate::product::json_store::ProductStoreError::Io(
                    "probe read failed".to_string(),
                ))
            };
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &failing_probe);

        let error = resolver
            .target_exists(
                DanglingTargetKind::Attempt,
                "coding_attempt_unreadable",
                None,
            )
            .expect_err("读失败必须原样上抛，不得退化为 Missing（REQ-DRG-21）");
        assert!(
            matches!(&error, crate::product::json_store::ProductStoreError::Io(message)
                if message == "probe read failed"),
            "注入探针的错误必须原样上抛，实际为 {error:?}"
        );
    }

    #[test]
    fn dangling_window_l2_verdict_each_condition_required() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let worktree = temp.path().join("gone-worktree");
        let window = |state: &str| -> ReviewerWindowRecord {
            serde_json::from_value(serde_json::json!({
                "schema": "reviewer_window_v1",
                "window_id": "win_001",
                "generation": 1,
                "identity": {
                    "canonical_worktree": worktree,
                    "gitdir": temp.path().join("gone-worktree/.git"),
                    "common_dir": temp.path().join("gone-worktree"),
                    "lease_id": "lease_1",
                },
                "attempt_id": "coding_attempt_9f3",
                "run_id": "run_1",
                "provider": "codex",
                "state": state,
                "report_acceptance": "accepted",
                "platform_write_paths": [],
                "quiesced_scopes": [],
                "baseline": null,
                "restore": null,
                "recovery": null,
                "last_error": null,
                "violation_pending": null,
                "pre_restore_git_facts": [],
                "transitions": [],
                "created_at": "2026-10-24T00:00:00Z",
                "updated_at": "2026-10-24T00:00:00Z",
            }))
            .expect("window record")
        };

        // 基线：终态 + 无租约 + 两路目标皆无 + worktree 已不存在 → 四条件全成立。
        let verdict = window_l2_verdict(&paths, &window("closed_accepted"), true).expect("verdict");
        assert!(verdict.all_hold(), "四条件全部成立时 all_hold 必须为真");

        // W-1 反例：窗口未到终态。
        let verdict = window_l2_verdict(&paths, &window("running"), true).expect("verdict");
        assert!(!verdict.terminal, "running 不是终态");
        assert!(!verdict.all_hold(), "W-1 不成立时 all_hold 必须为假");

        // W-3 反例：调用方核对到两路目标仍有命中。
        let verdict =
            window_l2_verdict(&paths, &window("closed_accepted"), false).expect("verdict");
        assert!(
            !verdict.target_missing,
            "target_missing=false 即 W-3 不成立"
        );
        assert!(!verdict.all_hold(), "W-3 不成立时 all_hold 必须为假");

        // W-4 反例：canonical worktree 路径仍存在。
        std::fs::create_dir_all(&worktree).unwrap();
        let verdict = window_l2_verdict(&paths, &window("closed_accepted"), true).expect("verdict");
        assert!(!verdict.worktree_gone, "worktree 仍存在即 W-4 不成立");
        assert!(!verdict.all_hold(), "W-4 不成立时 all_hold 必须为假");
        std::fs::remove_dir_all(&worktree).unwrap();

        // W-2 反例：树级 durable 租约文件存在。
        let leases = paths.root().join("reviewer-windows/leases");
        std::fs::create_dir_all(&leases).unwrap();
        std::fs::write(
            leases.join(format!("{}.json", tree_key_of(&worktree))),
            b"{}",
        )
        .unwrap();
        let verdict = window_l2_verdict(&paths, &window("closed_accepted"), true).expect("verdict");
        assert!(!verdict.no_lease, "租约文件存在即 W-2 不成立");
        assert!(!verdict.all_hold(), "W-2 不成立时 all_hold 必须为假");
    }

    #[test]
    fn dangling_resolver_u1_targets_report_unknown() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let sessions = BTreeSet::new();
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);

        for kind in [
            DanglingTargetKind::Checkout,
            DanglingTargetKind::AggregateIndex,
            DanglingTargetKind::LogicalMember,
            DanglingTargetKind::Enrollment,
        ] {
            assert_eq!(
                resolver
                    .target_exists(kind, "target_0001", Some(&scope()))
                    .unwrap(),
                TargetExistence::Unknown,
                "{kind:?} 的存在性来源在本判定面不可用，必须报「不可判定」而非「不存在」"
            );
        }
    }

    #[test]
    fn dangling_resolver_evidence_policy_matches_spec_table() {
        let level = |kind| evidence_policy(kind).deletable_level;
        assert_eq!(
            level(DanglingTargetKind::StorySpec),
            super::DeletableLevel::L1
        );
        assert_eq!(
            level(DanglingTargetKind::DesignSpec),
            super::DeletableLevel::L1
        );
        assert_eq!(level(DanglingTargetKind::Plan), super::DeletableLevel::L1);
        assert_eq!(
            level(DanglingTargetKind::Session),
            super::DeletableLevel::ReportOnly
        );
        assert_eq!(
            level(DanglingTargetKind::Attempt),
            super::DeletableLevel::L2
        );
        assert_eq!(
            level(DanglingTargetKind::Checkout),
            super::DeletableLevel::ReportOnly
        );
        assert_eq!(
            level(DanglingTargetKind::AggregateIndex),
            super::DeletableLevel::ReportOnly
        );
        assert_eq!(
            level(DanglingTargetKind::LogicalMember),
            super::DeletableLevel::ReportOnly
        );
        assert_eq!(
            level(DanglingTargetKind::Enrollment),
            super::DeletableLevel::ReportOnly
        );
    }
}
