//! 目标存在性核对与证据分级（REQ-DRG-01 / 02 / 12 / 21）。
//!
//! 本模块回答两层问题：
//! 1. 「目标还在吗」——[`dispatch_existence`] 按**证据等级表**（`reference_table`
//!    的 `ExistenceDispatch` 列）分派到既有只读 API 做存在性判定（REQ-DRG-01，
//!    三态：`Exists` / `Missing` / `Unknown`）；①层（[`DanglingResolver`]）与
//!    巡检面（`patrol`）**共用这一套分派**，两面的差别只在于判定面**能力**
//!    （作用域 / LC scope 读 / enrollment 读 / 路二可用性）——见 [`ExistenceEnv`]；
//! 2. 「能证明已删到可销账吗」——[`DanglingResolver::evidence_level`] 按
//!    L1 墓碑四条 / 类型专属 L2 条件给出证据等级（REQ-DRG-02），可删档位一律
//!    取自同一张表（`DeletableLevel`）。
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
use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::dangling_reference::reference_table::{
    DanglingTargetKind, DeletableLevel, ExistenceDispatch, evidence_policy,
};
use crate::product::entity_deletion_cascade;
use crate::product::entity_deletion_store::{
    DeletionPhase, EntityDeletionStore, EntityScope, issue_entity_operations_root,
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

/// 不可判定的原因类别（统一分派给出；巡检面据此渲染「不可判定」原因列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UndeterminedCause {
    /// 缺记录所属的 project/issue 作用域（REQ-DRG-23）。
    MissingIssueScope,
    /// 缺逻辑代码库 scope 上下文（REQ-DRG-23）。
    MissingLcScope,
    /// 判定面如实报「无法判定」（REQ-DRG-01 三态）。
    TargetUnknown,
    /// 数据源不可用（路二枚举失败 / 作用域内读失败）：附具体原因。
    SourceUnavailable(String),
}

/// 存在性判定结论：三态 + 不可判定原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistenceVerdict {
    /// 三态结论。
    pub(crate) existence: TargetExistence,
    /// 不可判定原因（`existence == Unknown` 时有值）。
    pub(crate) cause: Option<UndeterminedCause>,
}

impl ExistenceVerdict {
    /// 布尔命中 → 三态结论（无不可判定原因）。
    fn of(hit: bool) -> Self {
        Self {
            existence: existence(hit),
            cause: None,
        }
    }

    /// 不可判定结论（带原因）。
    fn unknown(cause: UndeterminedCause) -> Self {
        Self {
            existence: TargetExistence::Unknown,
            cause: Some(cause),
        }
    }
}

/// LC scope 内的存在性读能力（巡检面提供；① 层无 LC 上下文 → 相关目标恒
/// 「不可判定」，MUST NOT 伪装成「不存在」，REQ-DRG-23）。
pub(crate) trait LcScopeReader {
    /// 同 LC scope 内逻辑成员是否存在。
    fn member_exists(&self, target_id: &str) -> Result<bool, String>;
    /// 同 LC scope 内 checkout 是否存在。
    fn checkout_exists(&self, target_id: &str) -> Result<bool, String>;
    /// 同 LC scope 内聚合索引是否存在。
    fn aggregate_index_exists(
        &self,
        paths: &ProductAppPaths,
        target_id: &str,
    ) -> Result<bool, String>;
}

/// issue 作用域内的 enrollment 存在性读能力（巡检面提供；① 层无该读能力）。
pub(crate) trait EnrollmentReader {
    /// 当处 enrollment 记录是否存在且 enrollment id 命中。
    fn exists(&self, enrollment_id: &str) -> Result<bool, ProductStoreError>;
}

/// 存在性判定面的可用能力（作用域与数据源）。
///
/// ①层与巡检面**共用同一套分派**（[`dispatch_existence`]）；两面的差别只在本
/// 结构提供的**能力**：①层不持 LC scope 读 / enrollment 读（相关目标恒「不可
/// 判定」，REQ-DRG-23），巡检面持有（故巡检面能真读并可判悬空）。
pub(crate) struct ExistenceEnv<'a> {
    /// 状态根路径。
    pub(crate) paths: &'a ProductAppPaths,
    /// 路二：规划面 workspace session 全量 id 集合。
    pub(crate) session_ids: &'a BTreeSet<String>,
    /// 路一探针（`None` = 内置 `CodingAttemptStore`）。
    pub(crate) attempt_probe: Option<AttemptProbe<'a>>,
    /// 目标所属 issue 作用域（`None` = 本判定面不持有该作用域）。
    pub(crate) scope: Option<&'a EntityScope>,
    /// LC scope 读能力（`None` = 本判定面无 LC 上下文）。
    pub(crate) lc: Option<&'a dyn LcScopeReader>,
    /// enrollment 文件读能力（`None` = 本判定面无该读能力）。
    pub(crate) enrollment: Option<&'a dyn EnrollmentReader>,
    /// 路二不可用原因（`Some` = 枚举失败，「未命中」不得当成「不存在」）。
    pub(crate) session_unavailable_reason: Option<&'a str>,
}

/// 存在性核对的**统一分派**（REQ-DRG-01 / 23）。
///
/// 分派口径一律取自证据等级表（`reference_table::evidence_policy`）的
/// [`ExistenceDispatch`] 列：①层（`DanglingResolver`）与巡检面
/// （`patrol::PatrolContext`）都经本函数做存在性判定，MUST NOT 各写一套
/// per-target 分支；两面差别只体现在传入的**能力**（作用域 / LC scope 读 /
/// enrollment 读 / 路二可用性）上。
///
/// 读失败（IO / 解析 / 歧义）原样上抛，MUST NOT 被当成「目标不存在」。
pub(crate) fn dispatch_existence(
    env: &ExistenceEnv<'_>,
    kind: DanglingTargetKind,
    target_id: &str,
) -> Result<ExistenceVerdict, ProductStoreError> {
    let policy = evidence_policy(kind);
    match policy.dispatch {
        ExistenceDispatch::StorySpecList => {
            let Some(scope) = env.scope else {
                return Ok(ExistenceVerdict::unknown(
                    UndeterminedCause::MissingIssueScope,
                ));
            };
            let hit = LifecycleStore::new(env.paths.clone())
                .list_story_specs(&scope.project_id, &scope.issue_id)?
                .iter()
                .any(|record| record.id == target_id);
            Ok(ExistenceVerdict::of(hit))
        }
        ExistenceDispatch::DesignSpecList => {
            let Some(scope) = env.scope else {
                return Ok(ExistenceVerdict::unknown(
                    UndeterminedCause::MissingIssueScope,
                ));
            };
            let hit = LifecycleStore::new(env.paths.clone())
                .list_design_specs(&scope.project_id, &scope.issue_id)?
                .iter()
                .any(|record| record.id == target_id);
            Ok(ExistenceVerdict::of(hit))
        }
        ExistenceDispatch::LivePlanIds => {
            let Some(scope) = env.scope else {
                return Ok(ExistenceVerdict::unknown(
                    UndeterminedCause::MissingIssueScope,
                ));
            };
            let hit = entity_deletion_cascade::list_live_plan_ids(
                env.paths,
                &scope.project_id,
                &scope.issue_id,
            )?
            .iter()
            .any(|plan_id| plan_id == target_id);
            Ok(ExistenceVerdict::of(hit))
        }
        ExistenceDispatch::WorkspaceSession => {
            if env.session_ids.contains(target_id) {
                return Ok(ExistenceVerdict::of(true));
            }
            Ok(match env.session_unavailable_reason {
                Some(reason) => ExistenceVerdict::unknown(UndeterminedCause::SourceUnavailable(
                    reason.to_string(),
                )),
                None => ExistenceVerdict::of(false),
            })
        }
        ExistenceDispatch::AttemptTwoPath => {
            // 路一：coding attempt（内置探针或注入探针）；路二：workspace session
            // 枚举集合。任一路命中即「仍在」；两路皆不命中才算不存在。
            let coding_hit = match env.attempt_probe {
                Some(probe) => probe(target_id)?,
                None => coding_attempt_exists(env.paths, target_id)?,
            };
            if coding_hit || env.session_ids.contains(target_id) {
                return Ok(ExistenceVerdict::of(true));
            }
            Ok(match env.session_unavailable_reason {
                Some(reason) => ExistenceVerdict::unknown(UndeterminedCause::SourceUnavailable(
                    reason.to_string(),
                )),
                None => ExistenceVerdict::of(false),
            })
        }
        ExistenceDispatch::LogicalMember => {
            let Some(lc) = env.lc else {
                return Ok(ExistenceVerdict::unknown(UndeterminedCause::MissingLcScope));
            };
            Ok(lc_verdict(lc.member_exists(target_id)))
        }
        ExistenceDispatch::Checkout => {
            let Some(lc) = env.lc else {
                return Ok(ExistenceVerdict::unknown(UndeterminedCause::MissingLcScope));
            };
            Ok(lc_verdict(lc.checkout_exists(target_id)))
        }
        ExistenceDispatch::AggregateIndex => {
            let Some(lc) = env.lc else {
                return Ok(ExistenceVerdict::unknown(UndeterminedCause::MissingLcScope));
            };
            Ok(lc_verdict(lc.aggregate_index_exists(env.paths, target_id)))
        }
        ExistenceDispatch::EnrollmentFile => {
            let Some(reader) = env.enrollment else {
                // 本判定面不持 enrollment 读能力 → 如实报「无法判定」。
                return Ok(ExistenceVerdict::unknown(UndeterminedCause::TargetUnknown));
            };
            Ok(ExistenceVerdict::of(reader.exists(target_id)?))
        }
    }
}

/// LC scope 内读的结果 → 三态结论（读失败 = 不可判定，附原因）。
fn lc_verdict(result: Result<bool, String>) -> ExistenceVerdict {
    match result {
        Ok(true) => ExistenceVerdict::of(true),
        Ok(false) => ExistenceVerdict::of(false),
        Err(reason) => ExistenceVerdict::unknown(UndeterminedCause::SourceUnavailable(reason)),
    }
}

/// 附表要求的可删证据等级（销账判据从附表读出，MUST NOT 写死 L1 / L2）。
///
/// 只报不删档位（`ReportOnly` / `NeverDelete`）返回 `None`——无可删等级。
pub(crate) fn required_evidence_level(target_kind: DanglingTargetKind) -> Option<EvidenceLevel> {
    match evidence_policy(target_kind).deletable_level {
        DeletableLevel::L1 => Some(EvidenceLevel::L1),
        DeletableLevel::L2 => Some(EvidenceLevel::L2),
        DeletableLevel::ReportOnly | DeletableLevel::NeverDelete => None,
    }
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
    /// 分派口径取自证据等级表（`reference_table::evidence_policy` 的
    /// [`ExistenceDispatch`] 列），实现见 [`dispatch_existence`]——与巡检面
    /// **共用同一张表、同一套分派**，本方法只额外提供①层的判定面能力
    /// （无 LC scope 读 / 无 enrollment 读 / 路二完整可用）。
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
        let verdict = dispatch_existence(&self.env(scope), kind, target_id)?;
        match (verdict.existence, verdict.cause) {
            // 缺记录所属的 project/issue 作用域：① 层 API 保留「显式 NotFound」
            // 的既有语义（不得猜默认 scope）；巡检面则如实归「不可判定」。
            (TargetExistence::Unknown, Some(UndeterminedCause::MissingIssueScope)) => {
                Err(ProductStoreError::NotFound {
                    kind: "entity_scope",
                    id: target_id.to_string(),
                })
            }
            (existence, _) => Ok(existence),
        }
    }

    /// ① 层判定面的能力（无 LC scope 读 / 无 enrollment 读 / 路二完整可用）。
    fn env<'b>(&'b self, scope: Option<&'b EntityScope>) -> ExistenceEnv<'b> {
        ExistenceEnv {
            paths: self.paths,
            session_ids: self.session_ids,
            attempt_probe: self.attempt_probe,
            scope,
            lc: None,
            enrollment: None,
            session_unavailable_reason: None,
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
                let Some(deletion_scope) = tombstone_scope(kind, target_id, caller_scope) else {
                    return Ok(EvidenceLevel::Insufficient);
                };
                Ok(if self.l1_tombstone_holds(&deletion_scope)? {
                    EvidenceLevel::L1
                } else {
                    EvidenceLevel::Insufficient
                })
            }
            DeletableLevel::L2 | DeletableLevel::ReportOnly | DeletableLevel::NeverDelete => {
                Ok(EvidenceLevel::Insufficient)
            }
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

/// 目标类型 → 墓碑 scope：kind 取自证据等级表登记的墓碑实体种类
/// （附表只给 story / design / plan 登记墓碑档位），project/issue 取调用方
/// scope，entity_id 用被核对的 target_id。
fn tombstone_scope(
    kind: DanglingTargetKind,
    target_id: &str,
    scope: &EntityScope,
) -> Option<EntityScope> {
    let deletion_kind = evidence_policy(kind).deletion_kind?;
    Some(EntityScope::new(
        scope.project_id.clone(),
        scope.issue_id.clone(),
        deletion_kind,
        target_id.to_string(),
    ))
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

/// 规划面 workspace session 全量 id 集合（REQ-DRG-21 路二）。
///
/// MUST 用枚举接口；枚举失败 fail-closed 上抛（拿残缺集合会把活窗口判成悬空）。
pub(crate) fn workspace_session_ids(
    paths: &ProductAppPaths,
) -> Result<BTreeSet<String>, ProductStoreError> {
    let mut ids = BTreeSet::new();
    for (project_id, issue_id) in issue_scopes(paths)? {
        let records =
            LifecycleStore::new(paths.clone()).list_workspace_sessions(&project_id, &issue_id)?;
        ids.extend(records.into_iter().map(|record| record.id));
    }
    Ok(ids)
}

/// 状态根下的 project × issue 作用域（读根失败上抛：连结构都不通时无从判定）。
pub(crate) fn issue_scopes(
    paths: &ProductAppPaths,
) -> Result<Vec<(String, String)>, ProductStoreError> {
    let mut scopes = Vec::new();
    for project_id in child_directory_names(&paths.projects_root())? {
        for issue_id in child_directory_names(&paths.project_root(&project_id).join("issues"))? {
            scopes.push((project_id.clone(), issue_id));
        }
    }
    Ok(scopes)
}

/// 目录下的子目录名（目录不存在 → 空集；其余 IO 失败上抛）。
pub(crate) fn child_directory_names(dir: &Path) -> Result<BTreeSet<String>, ProductStoreError> {
    let mut names = BTreeSet::new();
    for (path, is_dir) in read_dir_sorted(dir)? {
        if !is_dir {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            names.insert(name.to_string());
        }
    }
    Ok(names)
}

/// 枚举目录子项（按路径升序，逐次可重现；目录不存在按空目录处理）。
pub(crate) fn read_dir_sorted(dir: &Path) -> Result<Vec<(PathBuf, bool)>, ProductStoreError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error("read", dir, &error)),
    };
    let mut items = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| io_error("read", dir, &error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| io_error("read", &entry.path(), &error))?;
        items.push((entry.path(), file_type.is_dir()));
    }
    items.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(items)
}

/// 统一的 IO 错误包装（读失败一律上抛，MUST NOT 被当成「空集」）。
fn io_error(action: &str, path: &Path, error: &std::io::Error) -> ProductStoreError {
    ProductStoreError::Io(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod resolve_tests {
    use super::{
        DanglingResolver, EvidenceLevel, TargetExistence, required_evidence_level,
        window_l2_verdict,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::dangling_reference::reference_table::{
        DanglingTargetKind, all_evidence_policies, evidence_policies, evidence_policy,
    };
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

    /// 证据等级表**逐行**锁定 spec REQ-DRG-02 附表 #1–#10 的五列字面值。
    ///
    /// 五列 = 目标类型 → 存在性来源 → 唯一可达上游 → L2 专属可算条件 →
    /// 可删证据等级；附表十行 = 九类目标（attempt 占两行：路 1 / 路 2）。
    #[test]
    fn dangling_resolver_evidence_policy_matches_spec_table() {
        // (目标类型, 存在性来源, 唯一可达上游, L2 专属可算条件, 可删证据等级)
        let expected: [(&str, &str, &str, &str, &str); 10] = [
            (
                "story_spec",
                "list_story_specs",
                "无上游（根实体）",
                "不可用（无上游可解析）",
                "L1",
            ),
            (
                "design_spec",
                "list_design_specs",
                "无上游（根实体）",
                "不可用（无上游可解析）",
                "L1",
            ),
            (
                "plan",
                "list_live_plan_ids",
                "plan → work item group → story/design（须 plan 记录本身才能解析）",
                "不可用（plan 记录已消失即无法解析）",
                "L1",
            ),
            (
                "session",
                "list_workspace_sessions",
                "session → enrollment 绑定",
                "不可用（无独立墓碑，且缺失属正常中间态）",
                "只报不删",
            ),
            (
                "attempt",
                "get_attempt_by_id",
                "attempt 记录无 plan_id，上游不可从 attempt 解析",
                "由窗口记录自身给出的四条（REQ-DRG-12）",
                "L2",
            ),
            (
                "attempt",
                "list_workspace_sessions",
                "同 #4",
                "由窗口记录自身给出的四条（REQ-DRG-12）",
                "L2",
            ),
            (
                "checkout",
                "LogicalCodebaseStore::list_checkouts（src/product/logical_codebase/store.rs:792）/ load_checkout（:777），须以 for_lc 作用域化；LC 由 LogicalCodebaseStore::list(project_id)（:249）枚举",
                "未核实，无法定义",
                "不可用（上游无法定义）",
                "只报不删",
            ),
            (
                "aggregate_index",
                "AggregateIndexStore::get（src/product/logical_codebase/aggregate_index/store.rs:72），须以 for_lc 作用域化 + 同上 LC 枚举",
                "未核实，无法定义",
                "不可用（上游无法定义）",
                "只报不删",
            ),
            (
                "logical_member",
                "LogicalCodebaseStore::list_members（src/product/logical_codebase/store.rs:751）/ load_member（:736）＋ LogicalCodebaseManifest.member_ids（store.rs:32），须以 for_lc 作用域化 + 同上 LC 枚举",
                "未核实，无法定义",
                "不可用（上游无法定义）",
                "只报不删",
            ),
            (
                "enrollment",
                "{issue_root}/automation-enrollment.json（IssueAutomationStore::get，src/product/issue_automation_store.rs:69）",
                "无上游（授权根）",
                "不可用（永不删）",
                "只报不删（永不删）",
            ),
        ];
        let rows = all_evidence_policies();
        assert_eq!(
            rows.len(),
            expected.len(),
            "证据等级表必须逐行登记 spec 附表 #1–#10"
        );
        for (index, (row, expected_row)) in rows.iter().zip(expected.iter()).enumerate() {
            assert_eq!(
                (
                    row.target_kind.as_str(),
                    row.existence_source,
                    row.upstream,
                    row.l2_condition,
                    row.deletable_level.label(),
                ),
                *expected_row,
                "证据等级表第 {} 行（spec 附表 #{}）的五列必须与 spec 逐字一致",
                index + 1,
                index + 1
            );
        }
    }

    /// 目标类型 → 可删等级（单一事实源）：九类目标逐类锁定。
    #[test]
    fn dangling_resolver_deletable_level_per_target_follows_spec_table() {
        let level = |kind| evidence_policy(kind).deletable_level.label();
        assert_eq!(level(DanglingTargetKind::StorySpec), "L1");
        assert_eq!(level(DanglingTargetKind::DesignSpec), "L1");
        assert_eq!(level(DanglingTargetKind::Plan), "L1");
        assert_eq!(level(DanglingTargetKind::Session), "只报不删");
        assert_eq!(level(DanglingTargetKind::Attempt), "L2");
        assert_eq!(level(DanglingTargetKind::Checkout), "只报不删");
        assert_eq!(level(DanglingTargetKind::AggregateIndex), "只报不删");
        assert_eq!(level(DanglingTargetKind::LogicalMember), "只报不删");
        assert_eq!(level(DanglingTargetKind::Enrollment), "只报不删（永不删）");
    }

    /// 证据等级表的行内一致性：同目标类型的各行在「可删等级 / L2 条件 / 分派」
    /// 三列上取值一致；墓碑档位（L1）与墓碑实体种类一一对应。
    #[test]
    fn dangling_evidence_policy_rows_are_consistent() {
        for kind in [
            DanglingTargetKind::StorySpec,
            DanglingTargetKind::DesignSpec,
            DanglingTargetKind::Plan,
            DanglingTargetKind::Session,
            DanglingTargetKind::Attempt,
            DanglingTargetKind::Checkout,
            DanglingTargetKind::AggregateIndex,
            DanglingTargetKind::LogicalMember,
            DanglingTargetKind::Enrollment,
        ] {
            let rows = evidence_policies(kind);
            assert!(!rows.is_empty(), "{kind:?} 必须在证据等级表登记");
            let first = rows[0];
            for row in rows {
                assert_eq!(
                    row.deletable_level, first.deletable_level,
                    "{kind:?} 的各行可删等级必须一致"
                );
                assert_eq!(
                    row.l2_condition, first.l2_condition,
                    "{kind:?} 的各行 L2 专属条件必须一致"
                );
                assert_eq!(
                    row.dispatch, first.dispatch,
                    "{kind:?} 的各行存在性分派口径必须一致"
                );
                assert_eq!(
                    row.deletion_kind.is_some(),
                    row.deletable_level.label() == "L1",
                    "{kind:?} 的墓碑档位与墓碑实体种类必须一一对应"
                );
            }
        }
    }

    /// 销账证据门取自证据等级表（不得旁路常量）。
    #[test]
    fn dangling_resolver_required_evidence_level_follows_table() {
        assert_eq!(
            required_evidence_level(DanglingTargetKind::StorySpec),
            Some(EvidenceLevel::L1)
        );
        assert_eq!(
            required_evidence_level(DanglingTargetKind::DesignSpec),
            Some(EvidenceLevel::L1)
        );
        assert_eq!(
            required_evidence_level(DanglingTargetKind::Plan),
            Some(EvidenceLevel::L1)
        );
        assert_eq!(
            required_evidence_level(DanglingTargetKind::Attempt),
            Some(EvidenceLevel::L2)
        );
        // 只报不删档位没有可删等级。
        assert_eq!(required_evidence_level(DanglingTargetKind::Session), None);
        assert_eq!(required_evidence_level(DanglingTargetKind::Checkout), None);
        assert_eq!(
            required_evidence_level(DanglingTargetKind::AggregateIndex),
            None
        );
        assert_eq!(
            required_evidence_level(DanglingTargetKind::LogicalMember),
            None
        );
        assert_eq!(
            required_evidence_level(DanglingTargetKind::Enrollment),
            None
        );
    }

    /// 存在性分派只有一处实现：①层与巡检面的生产段都不得出现 per-target 的
    /// 硬编码分支（目标类型的分派只许写在证据等级表里）。
    #[test]
    fn dangling_reference_dispatch_has_no_per_target_branches() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        for relative in [
            "src/product/dangling_reference/resolve.rs",
            "src/product/dangling_reference/patrol/context.rs",
        ] {
            let path = Path::new(manifest).join(relative);
            let source = std::fs::read_to_string(&path).expect("读取生产源码");
            // 只看生产段（跳过 `#[cfg(test)]` 之后的内联测试模块）。
            let production = source
                .split("#[cfg(test)]")
                .next()
                .unwrap_or(source.as_str());
            assert!(
                !production.contains("DanglingTargetKind::"),
                "{relative} 的生产段不得出现 per-target 硬编码分支（分派只许在证据等级表）"
            );
        }
    }
}
