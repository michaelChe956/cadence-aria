//! 悬空引用巡检遍历器（REQ-DRG-08 / 09 / 10 / 17 / 23）。
//!
//! 本模块是第 ② 层的**只读巡检面**：遍历状态根下的五个分区（issue 级、
//! reviewer 窗口、reviewer 快照、实体身份台账、逻辑代码库 scope），按判据表
//! 逐字段核对引用目标是否仍存在，产出 [`PatrolReport`]（发现 / 不可判定 /
//! 未归类 / 本次覆盖的记录类型）。
//!
//! 四条硬边界：
//!
//! 1. **零副作用**（REQ-DRG-10）：只有读路径——不删除、不移动、不修改任何
//!    文件，也不落盘清单（清单渲染属 Task 5）。本文件 MUST NOT 出现任何
//!    `std::fs::remove_file` / `std::fs::remove_dir_all` 调用，MUST NOT 取
//!    删除准入 lease，MUST NOT 调用任何会写回的读侧（例如
//!    `LogicalCodebaseStore::list` 会先跑 legacy 迁移、落迁移记录；故 LC 枚举
//!    由本模块直接读 `logical-codebases/` 与 legacy 别名的两个根目录）。
//! 2. **不可判定隔离**（REQ-DRG-23）：记录自身解析失败、分区目录不可读、
//!    workspace session 枚举失败，以及存在性判 `Unknown`（所需数据源 /
//!    作用域不可用）一律进 `undetermined`，MUST NOT 计入 `findings`、
//!    MUST NOT 被静默跳过、MUST NOT 计入归零。
//! 3. **读失败不得折叠**：目标核对读取失败原样记入 `undetermined`（写明原因），
//!    MUST NOT 被当成「目标已删除」。
//! 4. **未归类只报不删**：记录类型不在判据表内时进 `unclassified`，只做「记录
//!    存在性」统计，MUST NOT 猜测其引用语义（REQ-DRG-09）。
//!
//! 判据驱动：引用字段一律取自 `reference_table` 的静态判据（LC 三类见下）；
//! 存在性三态（`Exists` / `Missing` / `Unknown`）按 `resolve` 的口径分流——
//! 已判定悬空才产出一条发现，并附证据等级与建议动作。
//!
//! **逻辑代码库三类目标的记录形状**（技术方案 §3.3.2 r4 偏差登记）：判据表的
//! 单数字段名与现场记录形状不一致，本实现按**实际字段路径**逐项展开，且
//! MUST NOT 把「被引用侧的自身 id」当成引用——
//! `manifest.member_ids[]` / `checkout.logical_repository_id` /
//! `aggregate-index.member_snapshots[].logical_repository_id` 指向逻辑成员，
//! `member.checkout_ids[]` / `aggregate-index.member_snapshots[].checkout_id`
//! 指向 checkout，`manifest.active_aggregate_index_id` 指向聚合索引；后者现场
//! 可能未写（`skip_serializing_if`），字段缺失即**不产引用**，MUST NOT 当成
//! 「目标不存在」。存在性真实读用 `LogicalCodebaseStore::for_lc(..).list_members`
//! / `list_checkouts` 与 `AggregateIndexStore::for_lc(..).get`，且只在**同一 LC
//! scope 内**比对 id（跨 scope 不得互认）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::dangling_reference::reference_table::{
    DanglingRecordType, DanglingTargetKind, reference_rule,
};
use crate::product::dangling_reference::resolve::{
    AttemptProbe, DanglingResolver, EvidenceLevel, TargetExistence, window_l2_verdict,
};
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
use crate::product::issue_automation_store::{
    IssueAutomationStore, PlanGenerationIntentRead, PreparedPlanIntentRead,
    read_plan_generation_intent, read_prepared_plan_intent,
};
use crate::product::json_store::{ProductStoreError, read_json};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexRecord, AggregateIndexStore,
};
use crate::product::logical_codebase::{
    CodebaseMemberRecord, LogicalCodebaseManifest, LogicalCodebaseStore, RepositoryCheckoutRecord,
    lc_scope_root,
};
use crate::product::models::IssueAutomationEnrollment;
use crate::product::reviewer_resource_purge::REVIEWER_SNAPSHOT_STORE_DIR;
use crate::product::reviewer_window_store::{REVIEWER_WINDOW_STORE_DIR, ReviewerWindowRecord};

/// 巡检建议动作：删除整条记录（L1 墓碑证据达标）。
pub(crate) const PATROL_ACTION_DELETE: &str = "删除记录";
/// 巡检建议动作：按窗口簇删除（L2 四条件达标；删除粒度按 attempt 聚合）。
pub(crate) const PATROL_ACTION_DELETE_GROUP: &str = "删除记录（窗口簇）";
/// 巡检建议动作：只报不删（证据不足 / 规则降级 / 未归类）。
pub(crate) const PATROL_ACTION_REPORT_ONLY: &str = "只报不删";

/// 证据等级列取值：L1（墓碑四条）。
const EVIDENCE_LABEL_L1: &str = "L1";
/// 证据等级列取值：L2（窗口簇四条件）。
const EVIDENCE_LABEL_L2: &str = "L2";
/// 证据等级列取值：证据不足（已判定悬空但不够销账）。
const EVIDENCE_LABEL_INSUFFICIENT: &str = "证据不足";

/// 不可判定原因：缺记录所属的 project/issue 作用域（REQ-DRG-23）。
const REASON_NO_ISSUE_SCOPE: &str =
    "存在性数据源 / 作用域不可用：缺记录所属的 project/issue 作用域";
/// 不可判定原因：缺逻辑代码库 scope 上下文（REQ-DRG-23）。
const REASON_NO_LC_SCOPE: &str = "存在性数据源 / 作用域不可用：缺逻辑代码库 scope 上下文";
/// 不可判定原因：判定面如实报 `Unknown`（REQ-DRG-01 三态）。
const REASON_TARGET_UNKNOWN: &str = "存在性数据源 / 作用域不可用：判定面无法核实目标存在性";
/// 不可判定条目的引用字段列：整条记录级失败（解析失败等，无具体字段）。
const FIELD_RECORD_LEVEL: &str = "（整条记录）";
/// 不可判定条目的引用字段列：分区级失败（目录不可读 / 枚举失败）。
const FIELD_PARTITION_LEVEL: &str = "（分区）";
/// 窗口 / 快照记录的引用字段（`attempt_id`，取判据表登记值前的兜底常量）。
const FIELD_ATTEMPT: &str = "attempt_id";

/// 计划意图文件名（既有落盘约定，MUST NOT 枚举目录其它改名）。
const PLAN_INTENT_FILE: &str = "automation-plan-intent.json";
/// 生成检查点文件名（既有落盘约定）。
const GENERATION_INTENT_FILE: &str = "automation-generation-intent.json";
/// 自动化 enrollment 文件名（既有落盘约定）。
const ENROLLMENT_FILE: &str = "automation-enrollment.json";
/// reviewer 窗口活跃记录目录。
const WINDOWS_DIR: &str = "windows";
/// reviewer 窗口终态归档目录（r3：归档窗口同属窗口簇，删账按 attempt 聚合）。
const WINDOW_ARCHIVE_DIR: &str = "window-archive";
/// reviewer 快照基线目录（`<root>/reviewer-snapshots/windows/<id>/manifest.json`）。
const SNAPSHOT_WINDOWS_DIR: &str = "windows";
/// reviewer 快照 manifest 文件名。
const SNAPSHOT_MANIFEST_FILE: &str = "manifest.json";
/// 实体身份台账分区目录名（`<root>/state/entity-identity`）。
const ENTITY_IDENTITY_DIR: &str = "entity-identity";
/// 实体身份台账绑定条目目录名。
const ENTITY_SESSIONS_DIR: &str = "sessions";
/// 逻辑代码库 manifest 文件名（scope 根下）。
const LC_MANIFEST_FILE: &str = "manifest.json";
/// 逻辑代码库成员记录目录名。
const LC_MEMBERS_DIR: &str = "members";
/// 逻辑代码库 checkout 记录目录名。
const LC_CHECKOUTS_DIR: &str = "checkouts";
/// 逻辑代码库聚合索引记录目录名。
const LC_AGGREGATE_DIR: &str = "aggregate-indexes";
/// 巡检发现的一条悬空引用（REQ-DRG-11 明细七列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PatrolFinding {
    /// 记录路径（相对应用状态根）。
    pub(crate) record_path: String,
    /// 记录类型（判据表稳定标识）。
    pub(crate) record_type: String,
    /// 引用字段路径。
    pub(crate) reference_field: String,
    /// 引用目标类型（九类之一）。
    pub(crate) target_kind: String,
    /// 引用目标 id。
    pub(crate) target_id: String,
    /// 已删证据等级（`L1` / `L2（四条件摘要）` / `证据不足（…）`）。
    pub(crate) evidence_level: String,
    /// 建议动作（三个动作常量之一）。
    pub(crate) proposed_action: String,
}

/// 一条「不可判定」条目（REQ-DRG-23）：无法完成判定，不计悬空、不计归零。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UndeterminedEntry {
    /// 记录路径（相对应用状态根）或分区路径。
    pub(crate) record_path: String,
    /// 无法判定的引用字段（整条记录 / 分区级失败时为占位说明）。
    pub(crate) reference_field: String,
    /// 失败原因（逐条写明，供清单「不可判定」节使用）。
    pub(crate) reason: String,
}

/// 一条「未归类」记录（REQ-DRG-09）：只做记录存在性统计，不猜引用语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnclassifiedEntry {
    /// 记录路径（相对应用状态根）。
    pub(crate) record_path: String,
    /// 记录类型标识（未归类固定为 `unclassified`）。
    pub(crate) record_type: String,
}

/// 一次巡检的完整结果（REQ-DRG-08 / 09 / 11 / 23）。
///
/// 三栏互不混算：`findings`（已判定悬空）与 `undetermined`（无法判定）与
/// `unclassified`（表外记录）各自独立，任一栏都不由另一栏推导。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PatrolReport {
    /// 已判定悬空的引用（每条记录的一个引用字段一条）。
    pub(crate) findings: Vec<PatrolFinding>,
    /// 无法完成的判定（读取失败 / 数据源或作用域不可用）。
    pub(crate) undetermined: Vec<UndeterminedEntry>,
    /// 判据表未覆盖的记录（只报不删）。
    pub(crate) unclassified: Vec<UnclassifiedEntry>,
    /// 本次覆盖的记录类型清单（含 `unclassified`；按标识升序）。
    pub(crate) covered_record_types: Vec<String>,
}

/// 对状态根执行一次 dry-run 巡检（REQ-DRG-10：只读、零副作用）。
///
/// 返回 [`PatrolReport`]；单条记录 / 单个分区的失败一律进 `undetermined`，
/// 只有「连状态根结构都无法枚举」才上抛错误。
pub(crate) fn patrol_state_root(
    paths: &ProductAppPaths,
) -> Result<PatrolReport, ProductStoreError> {
    patrol_state_root_with_probe(paths, None)
}

/// 巡检主体（可注入 attempt 路一探针，供测试构造「存活 / 已删」两态）。
fn patrol_state_root_with_probe(
    paths: &ProductAppPaths,
    attempt_probe: Option<AttemptProbe<'_>>,
) -> Result<PatrolReport, ProductStoreError> {
    let mut builder = ReportBuilder::default();

    // ① project 枚举（连状态根结构都不通 → 上抛；其余错误逐条进不可判定）。
    let projects = project_ids(paths)?;

    // ② project × issue 组合（issue 分区遍历与路二 session 枚举共用）。
    let mut pairs = Vec::new();
    for project_id in &projects {
        match issue_ids(paths, project_id) {
            Ok(issue_ids) => {
                for issue_id in issue_ids {
                    pairs.push((project_id.clone(), issue_id));
                }
            }
            Err(error) => builder.push_undetermined(
                record_path_of(paths, &paths.project_root(project_id).join("issues")),
                FIELD_PARTITION_LEVEL,
                format!("issue 分区枚举失败：{error}"),
            ),
        }
    }

    // ③ 路二：遍历全部 project × issue 枚举 workspace session id 集合
    //    （REQ-DRG-21：MUST 用列举接口，MUST NOT 用带写回副作用的单条读）。
    let mut session_ids = BTreeSet::new();
    let mut session_errors = Vec::new();
    for (project_id, issue_id) in &pairs {
        match LifecycleStore::new(paths.clone()).list_workspace_sessions(project_id, issue_id) {
            Ok(records) => session_ids.extend(records.into_iter().map(|record| record.id)),
            Err(error) => session_errors.push(format!("{project_id}/{issue_id}：{error}")),
        }
    }

    let mut ctx = PatrolContext {
        paths,
        session_ids,
        session_enumeration_errors: session_errors,
        attempt_probe,
        attempt_clusters: BTreeMap::new(),
    };

    // ④ 分区一：issue 级（intent / enrollment / 表外记录）。
    for (project_id, issue_id) in &pairs {
        scan_issue_partition(&ctx, project_id, issue_id, &mut builder);
    }

    // ⑤ 分区二：reviewer 窗口（活跃 + 归档）→ attempt 窗口簇 L2 聚合 → 发现。
    let windows = scan_window_partition(&ctx, &mut builder);
    ctx.attempt_clusters = build_attempt_clusters(&ctx, &windows, &mut builder);
    emit_window_findings(&ctx, &windows, &mut builder);

    // ⑥ 分区三：reviewer 快照 manifest。
    scan_snapshot_partition(&ctx, &mut builder);

    // ⑦ 分区四：实体身份台账。
    scan_entity_identity_partition(&ctx, &mut builder);

    // ⑧ 分区五：逻辑代码库 scope（新版子树 + 旧版别名，两路都扫）。
    for project_id in &projects {
        scan_logical_codebase_partition(&ctx, project_id, &mut builder);
    }

    Ok(builder.finish())
}

/// 巡检上下文：读路径共用的只读事实（session 集合 / attempt 探针 / 窗口簇结论）。
struct PatrolContext<'a> {
    paths: &'a ProductAppPaths,
    /// 路二：全量 workspace session id 集合（由列举构建）。
    session_ids: BTreeSet<String>,
    /// 路二枚举失败的组合（非空 → 「未命中」结论不可用，REQ-DRG-23）。
    session_enumeration_errors: Vec<String>,
    /// 路一探针（生产走内置 `CodingAttemptStore`；测试注入）。
    attempt_probe: Option<AttemptProbe<'a>>,
    /// attempt id → 该 attempt 名下全部窗口（含归档）的 L2 聚合结论。
    attempt_clusters: BTreeMap<String, AttemptClusterVerdict>,
}

impl PatrolContext<'_> {
    /// 构造存在性核对器（沿用 `resolve` 的九类分派，不另造一套）。
    fn resolver(&self) -> DanglingResolver<'_> {
        match self.attempt_probe {
            Some(probe) => {
                DanglingResolver::with_attempt_probe(self.paths, &self.session_ids, probe)
            }
            None => DanglingResolver::new(self.paths, &self.session_ids),
        }
    }

    /// 路二是否可用（任一组合枚举失败即不可用）。
    fn sessions_complete(&self) -> bool {
        self.session_enumeration_errors.is_empty()
    }

    /// 会话枚举失败的可读原因（供「不可判定」原因列）。
    fn session_enumeration_reason(&self) -> String {
        format!(
            "存在性数据源 / 作用域不可用：workspace session 枚举失败，路二无法完成（{}）",
            self.session_enumeration_errors.join("；")
        )
    }

    /// 单条引用的三态判定（`Exists` / `Missing` + 证据与动作 / `Undetermined`）。
    fn judge(
        &self,
        reference: &PatrolReference<'_>,
        lc: Option<&LcScopeContext>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        match reference.target_kind {
            DanglingTargetKind::StorySpec
            | DanglingTargetKind::DesignSpec
            | DanglingTargetKind::Plan => self.judge_issue_target(reference),
            DanglingTargetKind::Session => Ok(self.judge_session(reference)),
            DanglingTargetKind::Attempt => self.judge_attempt(reference),
            DanglingTargetKind::Enrollment => self.judge_enrollment(reference),
            DanglingTargetKind::Checkout
            | DanglingTargetKind::AggregateIndex
            | DanglingTargetKind::LogicalMember => Ok(self.judge_lc_target(reference, lc)),
        }
    }

    /// story / design / plan：同 issue 作用域内的真实读（L1 档位）。
    fn judge_issue_target(
        &self,
        reference: &PatrolReference<'_>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        if reference.project_id.is_empty() || reference.issue_id.is_empty() {
            return Ok(ReferenceJudgement::Undetermined {
                reason: REASON_NO_ISSUE_SCOPE.to_string(),
            });
        }
        let scope = EntityScope::new(
            reference.project_id.clone(),
            reference.issue_id.clone(),
            deletion_kind_of(reference.target_kind),
            reference.target_id.clone(),
        );
        let resolver = self.resolver();
        match resolver.target_exists(
            reference.target_kind,
            reference.target_id.as_str(),
            Some(&scope),
        )? {
            TargetExistence::Exists => Ok(ReferenceJudgement::Alive),
            TargetExistence::Unknown => Ok(ReferenceJudgement::Undetermined {
                reason: REASON_TARGET_UNKNOWN.to_string(),
            }),
            TargetExistence::Missing => {
                let level = resolver.evidence_level(
                    reference.target_kind,
                    reference.target_id.as_str(),
                    Some(&scope),
                )?;
                Ok(l1_evidence_judgement(level))
            }
        }
    }

    /// session：路二集合成员关系（session 档位恒「只报不删」）。
    fn judge_session(&self, reference: &PatrolReference<'_>) -> ReferenceJudgement {
        if self.session_ids.contains(reference.target_id.as_str()) {
            return ReferenceJudgement::Alive;
        }
        if self.sessions_complete() {
            return report_only_judgement();
        }
        // 路二未能完成：MUST NOT 当成「目标不存在」（REQ-DRG-21）。
        ReferenceJudgement::Undetermined {
            reason: self.session_enumeration_reason(),
        }
    }

    /// attempt：两路核对（路一 coding attempt + 路二 session 集合）。
    fn judge_attempt(
        &self,
        reference: &PatrolReference<'_>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        match self.attempt_existence(reference.target_id.as_str())? {
            TargetExistence::Exists => Ok(ReferenceJudgement::Alive),
            TargetExistence::Unknown => Ok(ReferenceJudgement::Undetermined {
                reason: if self.sessions_complete() {
                    REASON_TARGET_UNKNOWN.to_string()
                } else {
                    self.session_enumeration_reason()
                },
            }),
            TargetExistence::Missing => Ok(match self.attempt_clusters.get(&reference.target_id) {
                Some(cluster) if cluster.l2_ready => ReferenceJudgement::Missing {
                    evidence_level: format!("{EVIDENCE_LABEL_L2}（{}）", cluster.summary),
                    proposed_action: PATROL_ACTION_DELETE_GROUP,
                },
                Some(cluster) => ReferenceJudgement::Missing {
                    evidence_level: format!("{EVIDENCE_LABEL_INSUFFICIENT}（{}）", cluster.summary),
                    proposed_action: PATROL_ACTION_REPORT_ONLY,
                },
                None => report_only_judgement(),
            }),
        }
    }

    /// attempt 存在性（路二不可用时，「未命中」降级为 `Unknown`）。
    fn attempt_existence(&self, attempt_id: &str) -> Result<TargetExistence, ProductStoreError> {
        let existence =
            self.resolver()
                .target_exists(DanglingTargetKind::Attempt, attempt_id, None)?;
        match existence {
            TargetExistence::Missing if !self.sessions_complete() => Ok(TargetExistence::Unknown),
            other => Ok(other),
        }
    }

    /// enrollment：按记录所属 project/issue 读当处文件（REQ-DRG-01 点名数据源）。
    fn judge_enrollment(
        &self,
        reference: &PatrolReference<'_>,
    ) -> Result<ReferenceJudgement, ProductStoreError> {
        if reference.project_id.is_empty() || reference.issue_id.is_empty() {
            return Ok(ReferenceJudgement::Undetermined {
                reason: REASON_NO_ISSUE_SCOPE.to_string(),
            });
        }
        let store = IssueAutomationStore::new(self.paths.clone());
        let existing = store.get(&reference.project_id, &reference.issue_id)?;
        Ok(match existing {
            Some(record) if record.enrollment_id == reference.target_id => {
                ReferenceJudgement::Alive
            }
            // 文件缺失或当处 enrollment 是另一个 id：明确未命中。
            _ => report_only_judgement(),
        })
    }

    /// checkout / aggregate_index / logical_member：同 LC scope 内的真实读。
    fn judge_lc_target(
        &self,
        reference: &PatrolReference<'_>,
        lc: Option<&LcScopeContext>,
    ) -> ReferenceJudgement {
        let Some(lc) = lc else {
            return ReferenceJudgement::Undetermined {
                reason: REASON_NO_LC_SCOPE.to_string(),
            };
        };
        let existence = match reference.target_kind {
            DanglingTargetKind::LogicalMember => lc.member_exists(reference.target_id.as_str()),
            DanglingTargetKind::Checkout => lc.checkout_exists(reference.target_id.as_str()),
            _ => lc.aggregate_index_exists(self.paths, reference.target_id.as_str()),
        };
        match existence {
            Ok(true) => ReferenceJudgement::Alive,
            // 三类恒「只报不删」（§3.1.2 降级档位）。
            Ok(false) => report_only_judgement(),
            Err(reason) => ReferenceJudgement::Undetermined { reason },
        }
    }
}

/// 一个 attempt 名下全部窗口（含归档）的 L2 聚合结论（r3 N-9：删除按 attempt 聚合）。
#[derive(Debug, Clone)]
struct AttemptClusterVerdict {
    /// 四条是否在该 attempt 名下的**每一个**窗口上都成立。
    l2_ready: bool,
    /// 四条件可读摘要（取该簇路径最小的窗口）。
    summary: String,
}

/// 逻辑代码库 scope 的只读上下文（成员 / checkout 存在性集合 + 作用域标识）。
struct LcScopeContext {
    /// scope 所属 project。
    project_id: String,
    /// LC id（`None` = 旧版别名仓，按 unscoped store 解析 legacy 根）。
    lc_id: Option<String>,
    /// scope 根目录。
    root: PathBuf,
    /// 逻辑成员 id 集合（读失败带原因：该 scope 的成员引用一律不可判定）。
    members: Result<BTreeSet<String>, String>,
    /// checkout id 集合（读失败带原因：该 scope 的 checkout 引用一律不可判定）。
    checkouts: Result<BTreeSet<String>, String>,
}

impl LcScopeContext {
    /// 读取该 scope 的成员 / checkout 存在性集合（只读；读失败逐条进不可判定）。
    fn load(
        paths: &ProductAppPaths,
        project_id: &str,
        lc_id: Option<String>,
        root: PathBuf,
    ) -> Self {
        let store = logical_codebase_store(paths, &lc_id);
        let members = match store.list_members(project_id) {
            Ok(records) => Ok(records
                .into_iter()
                .map(|record| record.logical_repository_id.0.to_string())
                .collect()),
            Err(error) => Err(format!("逻辑成员记录读取失败：{error}")),
        };
        let checkouts = match store.list_checkouts(project_id) {
            Ok(records) => Ok(records
                .into_iter()
                .map(|record| record.checkout_id.0.to_string())
                .collect()),
            Err(error) => Err(format!("checkout 记录读取失败：{error}")),
        };
        Self {
            project_id: project_id.to_string(),
            lc_id,
            root,
            members,
            checkouts,
        }
    }

    /// 逻辑成员是否存在（同 LC scope 内比对）。
    fn member_exists(&self, id: &str) -> Result<bool, String> {
        match &self.members {
            Ok(ids) => Ok(ids.contains(id)),
            Err(reason) => Err(reason.clone()),
        }
    }

    /// checkout 是否存在（同 LC scope 内比对）。
    fn checkout_exists(&self, id: &str) -> Result<bool, String> {
        match &self.checkouts {
            Ok(ids) => Ok(ids.contains(id)),
            Err(reason) => Err(reason.clone()),
        }
    }

    /// 聚合索引是否存在（同 LC scope 内按 id 真实读）。
    fn aggregate_index_exists(&self, paths: &ProductAppPaths, id: &str) -> Result<bool, String> {
        let store = aggregate_index_store(paths, &self.lc_id);
        match store.get(&self.project_id, id) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(error) => Err(format!("聚合索引记录读取失败：{error}")),
        }
    }
}

/// 单条引用的判定结果（三态在巡检面的映射）。
enum ReferenceJudgement {
    /// 目标仍在：不产发现、不产不可判定。
    Alive,
    /// 目标已删：产一条发现（证据等级 + 建议动作）。
    Missing {
        evidence_level: String,
        proposed_action: &'static str,
    },
    /// 无法判定：产一条不可判定条目（原因必填）。
    Undetermined { reason: String },
}

/// 一条待核对的引用。
struct PatrolReference<'a> {
    /// 引用字段路径（判据表登记值，或 LC 三类的实际记录形状路径）。
    field: &'a str,
    /// 引用目标类型。
    target_kind: DanglingTargetKind,
    /// 引用目标 id。
    target_id: String,
    /// 目标所属 project（同 issue 作用域用；无作用域需求时为空串）。
    project_id: String,
    /// 目标所属 issue（同 issue 作用域用；无作用域需求时为空串）。
    issue_id: String,
}

impl<'a> PatrolReference<'a> {
    fn new(
        field: &'a str,
        target_kind: DanglingTargetKind,
        target_id: impl Into<String>,
        project_id: &str,
        issue_id: &str,
    ) -> Self {
        Self {
            field,
            target_kind,
            target_id: target_id.into(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        }
    }

    /// attempt 引用（全局目标，无 project/issue 作用域需求）。
    fn attempt(attempt_id: impl Into<String>) -> Self {
        Self {
            field: FIELD_ATTEMPT,
            target_kind: DanglingTargetKind::Attempt,
            target_id: attempt_id.into(),
            project_id: String::new(),
            issue_id: String::new(),
        }
    }
}

/// 「已判定悬空 + 有 L1 墓碑」→ 建议删除；其余（含证据不足）→ 只报不删。
fn l1_evidence_judgement(level: EvidenceLevel) -> ReferenceJudgement {
    match level {
        EvidenceLevel::L1 => ReferenceJudgement::Missing {
            evidence_level: EVIDENCE_LABEL_L1.to_string(),
            proposed_action: PATROL_ACTION_DELETE,
        },
        EvidenceLevel::L2 | EvidenceLevel::Insufficient => report_only_judgement(),
    }
}

/// 「已判定悬空 + 规则降级 / 证据不足」→ 只报不删。
fn report_only_judgement() -> ReferenceJudgement {
    ReferenceJudgement::Missing {
        evidence_level: EVIDENCE_LABEL_INSUFFICIENT.to_string(),
        proposed_action: PATROL_ACTION_REPORT_ONLY,
    }
}

/// 目标类型 → 墓碑实体种类（仅 story / design / plan 有 L1 档位）。
fn deletion_kind_of(target_kind: DanglingTargetKind) -> DeletionEntityKind {
    match target_kind {
        DanglingTargetKind::StorySpec => DeletionEntityKind::Story,
        DanglingTargetKind::DesignSpec => DeletionEntityKind::Design,
        _ => DeletionEntityKind::Plan,
    }
}

/// 判据表中某记录类型的某个目标类型的字段路径（`entity_kind` 分派用）。
fn field_path_for(
    record_type: DanglingRecordType,
    target_kind: DanglingTargetKind,
) -> Option<&'static str> {
    reference_rule(record_type)?
        .fields
        .iter()
        .find(|field| field.target_kind == target_kind)
        .map(|field| field.path)
}

/// 核对一条引用并把结论落到三栏之一（读失败 → 不可判定，不得折叠）。
fn check_reference(
    ctx: &PatrolContext<'_>,
    record_path: &str,
    record_type: DanglingRecordType,
    reference: &PatrolReference<'_>,
    lc: Option<&LcScopeContext>,
    builder: &mut ReportBuilder,
) {
    match ctx.judge(reference, lc) {
        Ok(ReferenceJudgement::Alive) => {}
        Ok(ReferenceJudgement::Undetermined { reason }) => {
            builder.push_undetermined(record_path.to_string(), reference.field, reason);
        }
        Ok(ReferenceJudgement::Missing {
            evidence_level,
            proposed_action,
        }) => builder.push_finding(PatrolFinding {
            record_path: record_path.to_string(),
            record_type: record_type.as_str().to_string(),
            reference_field: reference.field.to_string(),
            target_kind: reference.target_kind.as_str().to_string(),
            target_id: reference.target_id.clone(),
            evidence_level,
            proposed_action: proposed_action.to_string(),
        }),
        Err(error) => builder.push_undetermined(
            record_path.to_string(),
            reference.field,
            format!("存在性读取失败：{error}"),
        ),
    }
}

/// 巡检结果累加器（三栏 + 覆盖类型，收尾统一排序去重保证逐次可重现）。
#[derive(Default)]
struct ReportBuilder {
    findings: Vec<PatrolFinding>,
    undetermined: Vec<UndeterminedEntry>,
    unclassified: Vec<UnclassifiedEntry>,
    covered_record_types: BTreeSet<String>,
}

impl ReportBuilder {
    /// 登记「本次巡检覆盖了该记录类型」。
    fn mark_covered(&mut self, record_type: DanglingRecordType) {
        self.covered_record_types
            .insert(record_type.as_str().to_string());
    }

    /// 记一条发现。
    fn push_finding(&mut self, finding: PatrolFinding) {
        self.findings.push(finding);
    }

    /// 记一条不可判定（REQ-DRG-23）。
    fn push_undetermined(&mut self, record_path: String, reference_field: &str, reason: String) {
        self.undetermined.push(UndeterminedEntry {
            record_path,
            reference_field: reference_field.to_string(),
            reason,
        });
    }

    /// 记一条未归类（REQ-DRG-09：只报不删）。
    fn push_unclassified(&mut self, record_path: String) {
        self.mark_covered(DanglingRecordType::Unclassified);
        self.unclassified.push(UnclassifiedEntry {
            record_path,
            record_type: DanglingRecordType::Unclassified.as_str().to_string(),
        });
    }

    /// 收尾：排序 + 去重后组装报告（去掉重复扫描带来的重复行）。
    fn finish(mut self) -> PatrolReport {
        self.findings.sort_by(|left, right| {
            left.record_path
                .cmp(&right.record_path)
                .then_with(|| left.reference_field.cmp(&right.reference_field))
                .then_with(|| left.target_kind.cmp(&right.target_kind))
                .then_with(|| left.target_id.cmp(&right.target_id))
        });
        self.findings.dedup();
        self.undetermined.sort_by(|left, right| {
            left.record_path
                .cmp(&right.record_path)
                .then_with(|| left.reference_field.cmp(&right.reference_field))
                .then_with(|| left.reason.cmp(&right.reason))
        });
        self.undetermined.dedup();
        self.unclassified.sort_by(|left, right| {
            left.record_path
                .cmp(&right.record_path)
                .then_with(|| left.record_type.cmp(&right.record_type))
        });
        self.unclassified.dedup();
        PatrolReport {
            findings: self.findings,
            undetermined: self.undetermined,
            unclassified: self.unclassified,
            covered_record_types: self.covered_record_types.into_iter().collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// 分区一：issue 级（intent / enrollment / 表外记录）
// ---------------------------------------------------------------------------

/// 遍历单个 issue 分区（`<root>/projects/<p>/issues/<i>/**` 的顶层记录文件）。
///
/// 只认三个既有落盘文件名的记录类型；同一层的其它 `*.json` 一律未归类
/// （只报不删）。子目录（stories / designs / plans / workspace-sessions /
/// codebase-attempts / entity-deletions 等）属**被引用侧**或非引用记录，
/// 不参与「引用字段」核对；墓碑因此绝不进悬空明细（REQ-DRG-09）。
fn scan_issue_partition(
    ctx: &PatrolContext<'_>,
    project_id: &str,
    issue_id: &str,
    builder: &mut ReportBuilder,
) {
    let issue_root = ctx.paths.issue_root(project_id, issue_id);
    let entries = match read_dir_sorted(&issue_root) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &issue_root),
                FIELD_PARTITION_LEVEL,
                format!("issue 分区不可读：{error}"),
            );
            return;
        }
    };
    for (path, is_dir) in entries {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        match file_name {
            PLAN_INTENT_FILE => scan_plan_intent(ctx, &path, project_id, issue_id, builder),
            GENERATION_INTENT_FILE => {
                scan_generation_intent(ctx, &path, project_id, issue_id, builder)
            }
            ENROLLMENT_FILE => scan_enrollment(ctx, &path, project_id, issue_id, builder),
            _ => builder.push_unclassified(record_path_of(ctx.paths, &path)),
        }
    }
}

/// 自动化 intent 记录上供判据表核对的引用事实。
struct IntentFacts {
    enrollment_id: String,
    plan_id: Option<String>,
    session_id: Option<String>,
    story_ids: Vec<String>,
    design_ids: Vec<String>,
    project_id: String,
    issue_id: String,
}

/// 核对一条 intent 记录（判据表登记的全部引用字段）。
fn check_intent_record(
    ctx: &PatrolContext<'_>,
    record_path: &str,
    record_type: DanglingRecordType,
    facts: &IntentFacts,
    builder: &mut ReportBuilder,
) {
    builder.mark_covered(record_type);
    let rule = reference_rule(record_type).expect("判据表必须登记 intent 型记录");
    // 记录类型不同但字段判据同源：按该记录类型的判据表行展开字段路径。
    let mut references = Vec::new();
    for field in rule.fields {
        match field.target_kind {
            DanglingTargetKind::StorySpec => {
                for id in &facts.story_ids {
                    references.push(PatrolReference::new(
                        field.path,
                        field.target_kind,
                        id.clone(),
                        &facts.project_id,
                        &facts.issue_id,
                    ));
                }
            }
            DanglingTargetKind::DesignSpec => {
                for id in &facts.design_ids {
                    references.push(PatrolReference::new(
                        field.path,
                        field.target_kind,
                        id.clone(),
                        &facts.project_id,
                        &facts.issue_id,
                    ));
                }
            }
            DanglingTargetKind::Plan => {
                for id in non_empty(&facts.plan_id) {
                    references.push(PatrolReference::new(
                        field.path,
                        field.target_kind,
                        id,
                        &facts.project_id,
                        &facts.issue_id,
                    ));
                }
            }
            DanglingTargetKind::Session => {
                for id in non_empty(&facts.session_id) {
                    references.push(PatrolReference::new(
                        field.path,
                        field.target_kind,
                        id,
                        &facts.project_id,
                        &facts.issue_id,
                    ));
                }
            }
            DanglingTargetKind::Enrollment if !facts.enrollment_id.is_empty() => {
                references.push(PatrolReference::new(
                    field.path,
                    field.target_kind,
                    facts.enrollment_id.clone(),
                    &facts.project_id,
                    &facts.issue_id,
                ));
            }
            _ => {}
        }
    }
    for reference in &references {
        check_reference(ctx, record_path, record_type, reference, None, builder);
    }
}

/// 计划意图记录（`automation-plan-intent.json`）。
fn scan_plan_intent(
    ctx: &PatrolContext<'_>,
    path: &Path,
    project_id: &str,
    issue_id: &str,
    builder: &mut ReportBuilder,
) {
    let record_path = record_path_of(ctx.paths, path);
    let read = match read_prepared_plan_intent(path) {
        Ok(read) => read,
        Err(error) => {
            builder.push_undetermined(
                record_path,
                FIELD_RECORD_LEVEL,
                format!("记录解析失败：{error}"),
            );
            return;
        }
    };
    let facts = match read {
        PreparedPlanIntentRead::Current(intent) => IntentFacts {
            enrollment_id: intent.enrollment_id.clone(),
            plan_id: Some(intent.plan_id.clone()),
            session_id: Some(intent.session_id.clone()),
            story_ids: intent
                .source
                .stories
                .iter()
                .map(|story| story.id.clone())
                .collect(),
            design_ids: intent
                .source
                .designs
                .iter()
                .map(|design| design.id.clone())
                .collect(),
            // 记录自带作用域优先；缺失时回落到落盘位置所在的 issue。
            project_id: if intent.project_id.is_empty() {
                project_id.to_string()
            } else {
                intent.project_id.clone()
            },
            issue_id: if intent.issue_id.is_empty() {
                issue_id.to_string()
            } else {
                intent.issue_id.clone()
            },
        },
        // 旧代无绑定意图：只有身份字段，无 plan/session/source 引用。
        PreparedPlanIntentRead::LegacyUnbound { enrollment_id, .. } => IntentFacts {
            enrollment_id,
            plan_id: None,
            session_id: None,
            story_ids: Vec::new(),
            design_ids: Vec::new(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        },
    };
    check_intent_record(
        ctx,
        &record_path,
        DanglingRecordType::AutomationPlanIntent,
        &facts,
        builder,
    );
}

/// 生成检查点记录（`automation-generation-intent.json`）。
fn scan_generation_intent(
    ctx: &PatrolContext<'_>,
    path: &Path,
    project_id: &str,
    issue_id: &str,
    builder: &mut ReportBuilder,
) {
    let record_path = record_path_of(ctx.paths, path);
    let read = match read_plan_generation_intent(path) {
        Ok(read) => read,
        Err(error) => {
            builder.push_undetermined(
                record_path,
                FIELD_RECORD_LEVEL,
                format!("记录解析失败：{error}"),
            );
            return;
        }
    };
    let facts = match read {
        PlanGenerationIntentRead::Current(intent) => IntentFacts {
            enrollment_id: intent.enrollment_id.clone(),
            plan_id: Some(intent.plan_id.clone()),
            session_id: Some(intent.session_id.clone()),
            story_ids: intent
                .source
                .stories
                .iter()
                .map(|story| story.id.clone())
                .collect(),
            design_ids: intent
                .source
                .designs
                .iter()
                .map(|design| design.id.clone())
                .collect(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        },
        // 旧代无绑定检查点：只有身份字段（enrollment / plan）。
        PlanGenerationIntentRead::LegacyUnbound {
            enrollment_id,
            plan_id,
        } => IntentFacts {
            enrollment_id,
            plan_id: Some(plan_id),
            session_id: None,
            story_ids: Vec::new(),
            design_ids: Vec::new(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        },
    };
    check_intent_record(
        ctx,
        &record_path,
        DanglingRecordType::AutomationGenerationIntent,
        &facts,
        builder,
    );
}

/// 自动化 enrollment 记录上的引用事实（对应判据表的十行）。
struct EnrollmentFacts {
    project_id: String,
    issue_id: String,
    source_stories: Vec<String>,
    source_designs: Vec<String>,
    plan_id: Option<String>,
    session_id: Option<String>,
    current: Option<EnrollmentBindingRefs>,
    previous: Vec<EnrollmentBindingRefs>,
}

/// 一代绑定身份里的引用（`binding_history.*`）。
struct EnrollmentBindingRefs {
    plan_id: String,
    session_id: String,
    stories: Vec<String>,
    designs: Vec<String>,
}

/// 按判据表登记顺序展开一条 enrollment 记录的全部待核对引用（REQ-DRG-09）。
fn enrollment_references(facts: &EnrollmentFacts) -> Vec<PatrolReference<'static>> {
    let rule = reference_rule(DanglingRecordType::AutomationEnrollment)
        .expect("判据表必须登记 automation_enrollment");
    let mut references = Vec::new();
    for field in rule.fields {
        // 字段路径 → 该字段下的目标 id 集合（判据表新增路径时须同步接线，
        // 由测试 `dangling_patrol_registers_every_enrollment_reference_field` 把关）。
        let ids: Vec<String> = match field.path {
            "source.stories[].id" => facts.source_stories.clone(),
            "source.designs[].id" => facts.source_designs.clone(),
            "plan_id" => non_empty(&facts.plan_id),
            "session_id" => non_empty(&facts.session_id),
            "binding_history.current.plan_id" => facts
                .current
                .as_ref()
                .filter(|current| !current.plan_id.is_empty())
                .map(|current| vec![current.plan_id.clone()])
                .unwrap_or_default(),
            "binding_history.current.session_id" => facts
                .current
                .as_ref()
                .filter(|current| !current.session_id.is_empty())
                .map(|current| vec![current.session_id.clone()])
                .unwrap_or_default(),
            "binding_history.current.source.stories[].id" => facts
                .current
                .as_ref()
                .map(|current| current.stories.clone())
                .unwrap_or_default(),
            "binding_history.current.source.designs[].id" => facts
                .current
                .as_ref()
                .map(|current| current.designs.clone())
                .unwrap_or_default(),
            "binding_history.previous[].source.stories[].id" => facts
                .previous
                .iter()
                .flat_map(|previous| previous.stories.clone())
                .collect(),
            "binding_history.previous[].source.designs[].id" => facts
                .previous
                .iter()
                .flat_map(|previous| previous.designs.clone())
                .collect(),
            _ => Vec::new(),
        };
        for id in ids {
            references.push(PatrolReference::new(
                field.path,
                field.target_kind,
                id,
                &facts.project_id,
                &facts.issue_id,
            ));
        }
    }
    references
}

/// 自动化 enrollment 记录（`automation-enrollment.json`）：只报不删档位的漏检防线。
fn scan_enrollment(
    ctx: &PatrolContext<'_>,
    path: &Path,
    project_id: &str,
    issue_id: &str,
    builder: &mut ReportBuilder,
) {
    let record_path = record_path_of(ctx.paths, path);
    let record: IssueAutomationEnrollment = match read_json(path) {
        Ok(record) => record,
        Err(error) => {
            builder.push_undetermined(
                record_path,
                FIELD_RECORD_LEVEL,
                format!("记录解析失败：{error}"),
            );
            return;
        }
    };
    builder.mark_covered(DanglingRecordType::AutomationEnrollment);
    let binding =
        |identity: &crate::product::models::EnrollmentBindingIdentity| EnrollmentBindingRefs {
            plan_id: identity.plan_id.clone(),
            session_id: identity.session_id.clone(),
            stories: identity
                .source
                .stories
                .iter()
                .map(|story| story.id.clone())
                .collect(),
            designs: identity
                .source
                .designs
                .iter()
                .map(|design| design.id.clone())
                .collect(),
        };
    let facts = EnrollmentFacts {
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        source_stories: record
            .source
            .stories
            .iter()
            .map(|story| story.id.clone())
            .collect(),
        source_designs: record
            .source
            .designs
            .iter()
            .map(|design| design.id.clone())
            .collect(),
        plan_id: record.plan_id.clone(),
        session_id: record.session_id.clone(),
        current: record
            .binding_history
            .as_ref()
            .map(|history| binding(&history.current)),
        previous: record
            .binding_history
            .as_ref()
            .map(|history| history.previous.iter().map(&binding).collect())
            .unwrap_or_default(),
    };
    for reference in enrollment_references(&facts) {
        check_reference(
            ctx,
            &record_path,
            DanglingRecordType::AutomationEnrollment,
            &reference,
            None,
            builder,
        );
    }
}

// ---------------------------------------------------------------------------
// 分区二：reviewer 窗口（活跃 + 归档）
// ---------------------------------------------------------------------------

/// 一条已解析的窗口记录（含记录路径）。
struct WindowRecordRef {
    record_path: String,
    record: ReviewerWindowRecord,
}

/// 遍历 reviewer 窗口分区（`windows/` 与 `window-archive/` 两路都扫）。
fn scan_window_partition(
    ctx: &PatrolContext<'_>,
    builder: &mut ReportBuilder,
) -> Vec<WindowRecordRef> {
    let mut windows = Vec::new();
    for dir_name in [WINDOWS_DIR, WINDOW_ARCHIVE_DIR] {
        let dir = ctx
            .paths
            .root()
            .join(REVIEWER_WINDOW_STORE_DIR)
            .join(dir_name);
        let entries = match read_dir_sorted(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                builder.push_undetermined(
                    record_path_of(ctx.paths, &dir),
                    FIELD_PARTITION_LEVEL,
                    format!("reviewer 窗口分区不可读：{error}"),
                );
                continue;
            }
        };
        for (path, is_dir) in entries {
            if is_dir || !is_json_file(&path) {
                continue;
            }
            let record_path = record_path_of(ctx.paths, &path);
            // MUST 逐条读：既有 `list_windows` 会静默跳过坏记录（REQ-DRG-23 禁止）。
            match read_json::<ReviewerWindowRecord>(&path) {
                Ok(record) => windows.push(WindowRecordRef {
                    record_path,
                    record,
                }),
                Err(error) => builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                ),
            }
        }
    }
    windows.sort_by(|left, right| left.record_path.cmp(&right.record_path));
    windows
}

/// 按 attempt 聚合窗口簇的 L2 结论（r3 N-9：任一同簇窗口不满足即整簇不删）。
fn build_attempt_clusters(
    ctx: &PatrolContext<'_>,
    windows: &[WindowRecordRef],
    builder: &mut ReportBuilder,
) -> BTreeMap<String, AttemptClusterVerdict> {
    let mut grouped: BTreeMap<&str, Vec<&WindowRecordRef>> = BTreeMap::new();
    for window in windows {
        grouped
            .entry(window.record.attempt_id.as_str())
            .or_default()
            .push(window);
    }
    let mut clusters = BTreeMap::new();
    for (attempt_id, group) in grouped {
        match ctx.attempt_existence(attempt_id) {
            // 目标仍在 / 无法判定：不建簇（也不产发现）。
            Ok(TargetExistence::Exists) => continue,
            Ok(TargetExistence::Unknown) => continue,
            Ok(TargetExistence::Missing) => {}
            Err(error) => {
                for window in &group {
                    builder.push_undetermined(
                        window.record_path.clone(),
                        FIELD_ATTEMPT,
                        format!("attempt 存在性读取失败：{error}"),
                    );
                }
                continue;
            }
        }
        let mut l2_ready = true;
        // 摘要取「首个不满足四条件的窗口」，全部满足时取首个窗口（避免用一扇
        // 合规窗口的摘要掩盖同簇另一扇窗口的不成立条件）。
        let mut failing_summary: Option<String> = None;
        let mut first_summary = String::new();
        for window in &group {
            match window_l2_verdict(ctx.paths, &window.record, true) {
                Ok(verdict) => {
                    let holds = verdict.all_hold();
                    if first_summary.is_empty() {
                        first_summary = verdict.summary();
                    }
                    if !holds && failing_summary.is_none() {
                        failing_summary = Some(verdict.summary());
                    }
                    l2_ready &= holds;
                }
                Err(error) => {
                    builder.push_undetermined(
                        window.record_path.clone(),
                        FIELD_ATTEMPT,
                        format!("窗口 L2 四条件核对读取失败：{error}"),
                    );
                    l2_ready = false;
                }
            }
        }
        let summary = failing_summary.unwrap_or(first_summary);
        clusters.insert(
            attempt_id.to_string(),
            AttemptClusterVerdict { l2_ready, summary },
        );
    }
    clusters
}

/// 逐条产出窗口记录上的发现（悬空 attempt → 一条发现 / 不可判定 → 一条记录）。
fn emit_window_findings(
    ctx: &PatrolContext<'_>,
    windows: &[WindowRecordRef],
    builder: &mut ReportBuilder,
) {
    for window in windows {
        builder.mark_covered(DanglingRecordType::ReviewerWindow);
        let reference = PatrolReference::attempt(window.record.attempt_id.clone());
        check_reference(
            ctx,
            &window.record_path,
            DanglingRecordType::ReviewerWindow,
            &reference,
            None,
            builder,
        );
    }
}

// ---------------------------------------------------------------------------
// 分区三：reviewer 快照（`windows/<id>/manifest.json`）
// ---------------------------------------------------------------------------

/// reviewer 快照 manifest 的最小投影：巡检只需 `attempt_id`（其余字段由 ③ 层持有）。
#[derive(Debug, serde::Deserialize)]
struct SnapshotManifestProjection {
    attempt_id: String,
}

/// 遍历 reviewer 快照分区。
fn scan_snapshot_partition(ctx: &PatrolContext<'_>, builder: &mut ReportBuilder) {
    let windows_root = ctx
        .paths
        .root()
        .join(REVIEWER_SNAPSHOT_STORE_DIR)
        .join(SNAPSHOT_WINDOWS_DIR);
    let entries = match read_dir_sorted(&windows_root) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &windows_root),
                FIELD_PARTITION_LEVEL,
                format!("reviewer 快照分区不可读：{error}"),
            );
            return;
        }
    };
    for (window_dir, is_dir) in entries {
        if !is_dir {
            // `windows/` 顶层散落的 json 不是判据表内记录 → 未归类（只报不删）。
            if is_json_file(&window_dir) {
                builder.push_unclassified(record_path_of(ctx.paths, &window_dir));
            }
            continue;
        }
        let children = match read_dir_sorted(&window_dir) {
            Ok(children) => children,
            Err(error) => {
                builder.push_undetermined(
                    record_path_of(ctx.paths, &window_dir),
                    FIELD_PARTITION_LEVEL,
                    format!("reviewer 快照窗口目录不可读：{error}"),
                );
                continue;
            }
        };
        for (child, child_is_dir) in children {
            if child_is_dir || !is_json_file(&child) {
                continue;
            }
            let file_name = child
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if file_name != SNAPSHOT_MANIFEST_FILE {
                // 同目录其它记录（如 `restore-cursor.json`）不在判据表内 → 未归类。
                builder.push_unclassified(record_path_of(ctx.paths, &child));
                continue;
            }
            scan_snapshot_manifest(ctx, &child, builder);
        }
    }
}

/// 单个快照 manifest：`attempt_id` 走两路核对。
fn scan_snapshot_manifest(ctx: &PatrolContext<'_>, path: &Path, builder: &mut ReportBuilder) {
    let record_path = record_path_of(ctx.paths, path);
    let manifest: SnapshotManifestProjection = match read_json(path) {
        Ok(manifest) => manifest,
        Err(error) => {
            builder.push_undetermined(
                record_path,
                FIELD_RECORD_LEVEL,
                format!("记录解析失败：{error}"),
            );
            return;
        }
    };
    builder.mark_covered(DanglingRecordType::ReviewerSnapshotManifest);
    let Some(field) = field_path_for(
        DanglingRecordType::ReviewerSnapshotManifest,
        DanglingTargetKind::Attempt,
    ) else {
        return;
    };
    let reference = PatrolReference::attempt(manifest.attempt_id);
    let reference = PatrolReference { field, ..reference };
    check_reference(
        ctx,
        &record_path,
        DanglingRecordType::ReviewerSnapshotManifest,
        &reference,
        None,
        builder,
    );
}

// ---------------------------------------------------------------------------
// 分区四：实体身份台账（`state/entity-identity/sessions/*.json`）
// ---------------------------------------------------------------------------

/// 遍历实体身份台账分区。
fn scan_entity_identity_partition(ctx: &PatrolContext<'_>, builder: &mut ReportBuilder) {
    let identity_root = ctx.paths.state_root().join(ENTITY_IDENTITY_DIR);
    // 分区根下的其它 json（非 sessions/）不在判据表内 → 未归类（只报不删）。
    match read_dir_sorted(&identity_root) {
        Ok(entries) => {
            for (path, is_dir) in entries {
                if !is_dir && is_json_file(&path) {
                    builder.push_unclassified(record_path_of(ctx.paths, &path));
                }
            }
        }
        Err(error) => builder.push_undetermined(
            record_path_of(ctx.paths, &identity_root),
            FIELD_PARTITION_LEVEL,
            format!("实体身份台账分区不可读：{error}"),
        ),
    }

    let sessions_root = identity_root.join(ENTITY_SESSIONS_DIR);
    let entries = match read_dir_sorted(&sessions_root) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &sessions_root),
                FIELD_PARTITION_LEVEL,
                format!("实体身份台账 sessions 目录不可读：{error}"),
            );
            return;
        }
    };
    for (path, is_dir) in entries {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let record_path = record_path_of(ctx.paths, &path);
        let value: serde_json::Value = match read_json(&path) {
            Ok(value) => value,
            Err(error) => {
                builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                );
                continue;
            }
        };
        builder.mark_covered(DanglingRecordType::EntityIdentitySessionBinding);
        let entity_kind = json_str_field(&value, "entity_kind");
        // 按 `entity_kind` 值分派到判据表命中的那一行；表外值不猜引用语义。
        let Some(target_kind) = entity_kind_target_kind(&entity_kind) else {
            continue;
        };
        let entity_id = json_str_field(&value, "entity_id");
        if entity_id.is_empty() {
            continue;
        }
        let Some(field) = field_path_for(
            DanglingRecordType::EntityIdentitySessionBinding,
            target_kind,
        ) else {
            continue;
        };
        let reference = PatrolReference::new(
            field,
            target_kind,
            entity_id,
            &json_str_field(&value, "project_id"),
            &json_str_field(&value, "issue_id"),
        );
        check_reference(
            ctx,
            &record_path,
            DanglingRecordType::EntityIdentitySessionBinding,
            &reference,
            None,
            builder,
        );
    }
}

/// 台账条目的 `entity_kind` 值 → 引用目标类型（判据表按值分派）。
fn entity_kind_target_kind(entity_kind: &str) -> Option<DanglingTargetKind> {
    match entity_kind {
        "story" => Some(DanglingTargetKind::StorySpec),
        "design" => Some(DanglingTargetKind::DesignSpec),
        "plan" => Some(DanglingTargetKind::Plan),
        _ => None,
    }
}

/// 通用 JSON 字符串字段（缺失 / 非字符串 → 空串）。
fn json_str_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------------------------
// 分区五：逻辑代码库 scope（新版子树 + 旧版别名，两路都扫）
// ---------------------------------------------------------------------------

/// 遍历单个 project 的逻辑代码库 scope 分区（REQ-DRG-08 两路都扫）。
fn scan_logical_codebase_partition(
    ctx: &PatrolContext<'_>,
    project_id: &str,
    builder: &mut ReportBuilder,
) {
    let codebases_root = ctx.paths.logical_codebases_root(project_id);
    match read_dir_sorted(&codebases_root) {
        Ok(entries) => {
            for (path, is_dir) in entries {
                if !is_dir {
                    continue;
                }
                let Some(lc_id) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                scan_logical_codebase_scope(ctx, project_id, Some(lc_id.to_string()), builder);
            }
        }
        Err(error) => builder.push_undetermined(
            record_path_of(ctx.paths, &codebases_root),
            FIELD_PARTITION_LEVEL,
            format!("逻辑代码库分区不可读：{error}"),
        ),
    }

    // 旧版别名仓（`logical-codebase/`）可能与新版子树并存：两路都扫。
    let legacy_root = ctx.paths.logical_codebase_root(project_id);
    match path_exists(&legacy_root) {
        Ok(true) => scan_logical_codebase_scope(ctx, project_id, None, builder),
        Ok(false) => {}
        Err(error) => builder.push_undetermined(
            record_path_of(ctx.paths, &legacy_root),
            FIELD_PARTITION_LEVEL,
            format!("旧版逻辑代码库别名分区不可读：{error}"),
        ),
    }
}

/// 遍历单个 LC scope 下的记录文件。
fn scan_logical_codebase_scope(
    ctx: &PatrolContext<'_>,
    project_id: &str,
    lc_id: Option<String>,
    builder: &mut ReportBuilder,
) {
    let root = match lc_scope_root(ctx.paths, project_id, &lc_id) {
        Ok(root) => root,
        Err(error) => {
            builder.push_undetermined(
                format!(
                    "projects/{project_id}/{}",
                    lc_id.as_deref().unwrap_or("logical-codebase")
                ),
                FIELD_PARTITION_LEVEL,
                format!("逻辑代码库 scope 路径不可用：{error}"),
            );
            return;
        }
    };
    match path_exists(&root) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &root),
                FIELD_PARTITION_LEVEL,
                format!("逻辑代码库 scope 目录不可读：{error}"),
            );
            return;
        }
    }
    let scope = LcScopeContext::load(ctx.paths, project_id, lc_id, root.clone());
    let entries = match read_dir_sorted(&scope.root) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &scope.root),
                FIELD_PARTITION_LEVEL,
                format!("逻辑代码库 scope 目录不可读：{error}"),
            );
            return;
        }
    };
    for (path, is_dir) in entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if is_dir {
            match name {
                LC_MEMBERS_DIR => scan_lc_members(ctx, &scope, &path, builder),
                LC_CHECKOUTS_DIR => scan_lc_checkouts(ctx, &scope, &path, builder),
                LC_AGGREGATE_DIR => scan_lc_aggregate_indexes(ctx, &scope, &path, builder),
                _ => {}
            }
        } else if is_json_file(&path) {
            if name == LC_MANIFEST_FILE {
                scan_lc_manifest(ctx, &scope, &path, builder);
            } else {
                // scope 根下其它记录（如 `record.json`）不在判据表内 → 未归类。
                builder.push_unclassified(record_path_of(ctx.paths, &path));
            }
        }
    }
}

/// LC manifest：`member_ids[]` → 逻辑成员；`active_aggregate_index_id` → 聚合索引。
fn scan_lc_manifest(
    ctx: &PatrolContext<'_>,
    scope: &LcScopeContext,
    path: &Path,
    builder: &mut ReportBuilder,
) {
    let record_path = record_path_of(ctx.paths, path);
    let manifest: LogicalCodebaseManifest = match read_json(path) {
        Ok(manifest) => manifest,
        Err(error) => {
            builder.push_undetermined(
                record_path,
                FIELD_RECORD_LEVEL,
                format!("记录解析失败：{error}"),
            );
            return;
        }
    };
    builder.mark_covered(DanglingRecordType::LogicalCodebaseMember);
    let mut references = Vec::new();
    for member_id in &manifest.member_ids {
        references.push(PatrolReference::new(
            "member_ids[]",
            DanglingTargetKind::LogicalMember,
            member_id.0.to_string(),
            &scope.project_id,
            "",
        ));
    }
    // 现场可能未写该字段：缺失即不产引用（MUST NOT 当成「目标不存在」）。
    if let Some(active) = manifest.active_aggregate_index_id.as_deref()
        && !active.is_empty()
    {
        references.push(PatrolReference::new(
            "active_aggregate_index_id",
            DanglingTargetKind::AggregateIndex,
            active,
            &scope.project_id,
            "",
        ));
    }
    for reference in &references {
        check_reference(
            ctx,
            &record_path,
            DanglingRecordType::LogicalCodebaseMember,
            reference,
            Some(scope),
            builder,
        );
    }
}

/// LC 成员记录：`checkout_ids[]` → checkout（成员自身 id 不是引用）。
fn scan_lc_members(
    ctx: &PatrolContext<'_>,
    scope: &LcScopeContext,
    dir: &Path,
    builder: &mut ReportBuilder,
) {
    for (path, is_dir) in read_dir_or_undetermined(ctx, dir, builder) {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let record_path = record_path_of(ctx.paths, &path);
        let member: CodebaseMemberRecord = match read_json(&path) {
            Ok(member) => member,
            Err(error) => {
                builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                );
                continue;
            }
        };
        builder.mark_covered(DanglingRecordType::LogicalCodebaseMember);
        for checkout_id in &member.checkout_ids {
            let reference = PatrolReference::new(
                "checkout_ids[]",
                DanglingTargetKind::Checkout,
                checkout_id.0.to_string(),
                &scope.project_id,
                "",
            );
            check_reference(
                ctx,
                &record_path,
                DanglingRecordType::LogicalCodebaseMember,
                &reference,
                Some(scope),
                builder,
            );
        }
    }
}

/// LC checkout 记录：`logical_repository_id` → 逻辑成员（自身 checkout_id 不是引用）。
fn scan_lc_checkouts(
    ctx: &PatrolContext<'_>,
    scope: &LcScopeContext,
    dir: &Path,
    builder: &mut ReportBuilder,
) {
    for (path, is_dir) in read_dir_or_undetermined(ctx, dir, builder) {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let record_path = record_path_of(ctx.paths, &path);
        let checkout: RepositoryCheckoutRecord = match read_json(&path) {
            Ok(checkout) => checkout,
            Err(error) => {
                builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                );
                continue;
            }
        };
        builder.mark_covered(DanglingRecordType::LogicalCodebaseMember);
        let reference = PatrolReference::new(
            "logical_repository_id",
            DanglingTargetKind::LogicalMember,
            checkout.logical_repository_id.0.to_string(),
            &scope.project_id,
            "",
        );
        check_reference(
            ctx,
            &record_path,
            DanglingRecordType::LogicalCodebaseMember,
            &reference,
            Some(scope),
            builder,
        );
    }
}

/// LC 聚合索引记录：`member_snapshots[].checkout_id` → checkout、
/// `member_snapshots[].logical_repository_id` → 逻辑成员。
fn scan_lc_aggregate_indexes(
    ctx: &PatrolContext<'_>,
    scope: &LcScopeContext,
    dir: &Path,
    builder: &mut ReportBuilder,
) {
    for (path, is_dir) in read_dir_or_undetermined(ctx, dir, builder) {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let record_path = record_path_of(ctx.paths, &path);
        let record: AggregateIndexRecord = match read_json(&path) {
            Ok(record) => record,
            Err(error) => {
                builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                );
                continue;
            }
        };
        builder.mark_covered(DanglingRecordType::LogicalCodebaseMember);
        for snapshot in &record.member_snapshots {
            let member = PatrolReference::new(
                "member_snapshots[].logical_repository_id",
                DanglingTargetKind::LogicalMember,
                snapshot.logical_repository_id.0.to_string(),
                &scope.project_id,
                "",
            );
            check_reference(
                ctx,
                &record_path,
                DanglingRecordType::LogicalCodebaseMember,
                &member,
                Some(scope),
                builder,
            );
            let checkout = PatrolReference::new(
                "member_snapshots[].checkout_id",
                DanglingTargetKind::Checkout,
                snapshot.checkout_id.0.to_string(),
                &scope.project_id,
                "",
            );
            check_reference(
                ctx,
                &record_path,
                DanglingRecordType::LogicalCodebaseMember,
                &checkout,
                Some(scope),
                builder,
            );
        }
    }
}

/// 目录枚举；目录不可读 → 记一条不可判定并返回空表（不静默跳过）。
fn read_dir_or_undetermined(
    ctx: &PatrolContext<'_>,
    dir: &Path,
    builder: &mut ReportBuilder,
) -> Vec<(PathBuf, bool)> {
    match read_dir_sorted(dir) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, dir),
                FIELD_PARTITION_LEVEL,
                format!("分区目录不可读：{error}"),
            );
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// 只读基础工具
// ---------------------------------------------------------------------------

/// 按 scope 解析逻辑代码库 store（`None` = 旧版别名仓，解析 legacy 根）。
fn logical_codebase_store(paths: &ProductAppPaths, lc_id: &Option<String>) -> LogicalCodebaseStore {
    match lc_id {
        Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone()),
        None => LogicalCodebaseStore::new(paths.clone()),
    }
}

/// 按 scope 解析聚合索引 store。
fn aggregate_index_store(paths: &ProductAppPaths, lc_id: &Option<String>) -> AggregateIndexStore {
    match lc_id {
        Some(lc_id) => AggregateIndexStore::for_lc(paths.clone(), lc_id.clone()),
        None => AggregateIndexStore::new(paths.clone()),
    }
}

/// 状态根下的 project id 列表（读根失败上抛：连结构都不通时无巡检可言）。
fn project_ids(paths: &ProductAppPaths) -> Result<Vec<String>, ProductStoreError> {
    Ok(child_directory_names(paths.projects_root())?
        .into_iter()
        .collect())
}

/// 单个 project 的 issue id 列表（读失败由调用方记为不可判定）。
fn issue_ids(paths: &ProductAppPaths, project_id: &str) -> Result<Vec<String>, ProductStoreError> {
    Ok(
        child_directory_names(paths.project_root(project_id).join("issues"))?
            .into_iter()
            .collect(),
    )
}

/// 列出目录下的子目录名（目录不存在 → 空表；其余 IO 失败上抛）。
fn child_directory_names(dir: PathBuf) -> Result<BTreeSet<String>, ProductStoreError> {
    let mut names = BTreeSet::new();
    for (path, is_dir) in read_dir_sorted(&dir)? {
        if !is_dir {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            names.insert(name.to_string());
        }
    }
    Ok(names)
}

/// 枚举目录下的子项（按路径升序，保证逐次可重现）。
///
/// 目录不存在按「空目录」处理；其余 IO 失败上抛（MUST NOT 被当成「空」）。
fn read_dir_sorted(dir: &Path) -> Result<Vec<(PathBuf, bool)>, ProductStoreError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                dir.display()
            )));
        }
    };
    let mut items = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", dir.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        items.push((entry.path(), file_type.is_dir()));
    }
    items.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(items)
}

/// 路径是否是 `.json` 文件候选（目录由调用方区分）。
fn is_json_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("json")
}

/// 存在性判定（stat）：IO 失败上抛为 `ProductStoreError::Io`，
/// MUST NOT 被当成「不存在」。
fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("stat {}: {error}", path.display())))
}

/// 记录路径（相对应用状态根 `paths.root()`）。
fn record_path_of(paths: &ProductAppPaths, path: &Path) -> String {
    path.strip_prefix(paths.root())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// 非空 id 的借用（空串 id 不构成引用）。
fn non_empty(id: &Option<String>) -> Vec<String> {
    id.as_ref()
        .filter(|id| !id.is_empty())
        .map(|id| vec![id.clone()])
        .unwrap_or_default()
}

#[cfg(test)]
mod patrol_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use std::path::Path;
    use tempfile::TempDir;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body.as_bytes()).unwrap();
    }

    fn tree_digest(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push((
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .to_string(),
                        std::fs::read(&path).unwrap(),
                    ));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn dangling_patrol_covers_five_partitions_and_registers_all_reference_fields() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let issue_root = paths.issue_root("project_0001", "issue_0001");
        write(
            &issue_root.join("automation-enrollment.json"),
            r#"{"enrollment_id":"e1","selection_key":"s1","project_id":"project_0001","issue_id":"issue_0001","enabled":false,"policy_revision":1,"source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"prepare_intent_id":"p1","plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x","created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}"#,
        );
        write(
            &paths.root().join("reviewer-windows/windows/win_001.json"),
            r#"{"schema":"reviewer_window_v1","window_id":"win_001","generation":1,"identity":{"canonical_worktree":"/tmp/none","gitdir":"/tmp/none/.git","common_dir":"/tmp/none","lease_id":"lease_1"},"attempt_id":"coding_attempt_9f3","run_id":"run_1","provider":"p","state":"closed_accepted","report_acceptance":"accepted","platform_write_paths":[],"quiesced_scopes":[],"baseline":null,"restore":null,"recovery":null,"last_error":null,"violation_pending":null,"pre_restore_git_facts":[],"transitions":[],"created_at":"2026-10-24T00:00:00Z","updated_at":"2026-10-24T00:00:00Z"}"#,
        );
        write(
            &paths
                .root()
                .join("state/entity-identity/sessions/workspace_session_auto_x.json"),
            r#"{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"plan","entity_id":"issue_work_item_plan_0007"}"#,
        );

        let report = patrol_state_root(&paths).expect("patrol");
        assert!(
            report
                .covered_record_types
                .iter()
                .any(|kind| kind == "automation_enrollment")
        );
        assert!(
            report
                .covered_record_types
                .iter()
                .any(|kind| kind == "reviewer_window")
        );
        assert!(
            report
                .covered_record_types
                .iter()
                .any(|kind| kind == "entity_identity_session_binding")
        );
        let fields: Vec<&str> = report
            .findings
            .iter()
            .map(|finding| finding.reference_field.as_str())
            .collect();
        assert!(fields.contains(&"source.stories[].id"), "{fields:?}");
        assert!(fields.contains(&"plan_id"), "{fields:?}");
        assert!(fields.contains(&"attempt_id"), "{fields:?}");
        assert!(fields.contains(&"entity_kind+entity_id"), "{fields:?}");
    }

    #[test]
    fn dangling_patrol_isolates_undetermined_entries() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        write(
            &paths.root().join("reviewer-windows/windows/win_bad.json"),
            "{ not json",
        );

        let report = patrol_state_root(&paths).expect("patrol");
        assert_eq!(report.undetermined.len(), 1, "坏记录必须进不可判定");
        assert_eq!(report.findings.len(), 0, "坏记录不得计入悬空数");
        assert!(
            report.undetermined[0].reason.contains("json"),
            "{}",
            report.undetermined[0].reason
        );
    }

    #[test]
    fn dangling_patrol_makes_no_writes() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        write(
            &paths
                .issue_root("project_0001", "issue_0001")
                .join("automation-plan-intent.json"),
            r#"{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x"}"#,
        );
        let before = tree_digest(paths.root());
        let _ = patrol_state_root(&paths).expect("patrol");
        let after = tree_digest(paths.root());
        assert_eq!(before, after, "巡检不得改写状态根");
    }

    // -- 夹具 ------------------------------------------------------------

    /// 路一探针：只有 `alive` 名单内的 attempt 命中（其余一律「已删」）。
    fn probe_alive(
        alive: &'static [&'static str],
    ) -> impl Fn(&str) -> Result<bool, ProductStoreError> {
        move |id: &str| Ok(alive.contains(&id))
    }

    /// 路一探针：一律「已删」。
    fn probe_false() -> impl Fn(&str) -> Result<bool, ProductStoreError> {
        |_id: &str| Ok(false)
    }

    /// 计划存活判据（schema-v2 线路）：`work-item-revisions/<plan_id>/lineage.json`。
    fn write_live_plan(paths: &ProductAppPaths, plan_id: &str) {
        write(
            &paths
                .issue_root("project_0001", "issue_0001")
                .join("work-item-revisions")
                .join(plan_id)
                .join("lineage.json"),
            "{}",
        );
    }

    /// 计划意图记录（`Current` 形态：带 `target`，全引用由入参决定）。
    fn plan_intent_body(plan_id: &str, session_id: &str) -> String {
        format!(
            r#"{{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{{"stories":[],"designs":[]}},"options":{{"author_provider":"codex","reviewer_provider":"codex","plan_options":{{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}}}},"target":{{"kind":"single_repository","repository_id":"repo_1"}},"plan_id":"{plan_id}","session_id":"{session_id}"}}"#
        )
    }

    /// 自动化 enrollment 记录（无引用：`source` 空、无 plan/session 绑定）。
    fn enrollment_body() -> &'static str {
        r#"{"enrollment_id":"e1","selection_key":"s1","project_id":"project_0001","issue_id":"issue_0001","enabled":false,"policy_revision":1,"source":{"stories":[],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"prepare_intent_id":"p1","created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}"#
    }

    /// reviewer 窗口记录。
    fn window_body(window_id: &str, attempt_id: &str, state: &str, worktree: &Path) -> String {
        format!(
            r#"{{"schema":"reviewer_window_v1","window_id":"{window_id}","generation":1,"identity":{{"canonical_worktree":"{}","gitdir":"{}/.git","common_dir":"{}/.git","lease_id":"lease_1"}},"attempt_id":"{attempt_id}","run_id":"run_1","provider":"codex","state":"{state}","report_acceptance":"accepted","platform_write_paths":[],"quiesced_scopes":[],"baseline":null,"restore":null,"recovery":null,"last_error":null,"violation_pending":null,"pre_restore_git_facts":[],"transitions":[],"created_at":"2026-10-24T00:00:00Z","updated_at":"2026-10-24T00:00:00Z"}}"#,
            worktree.display(),
            worktree.display(),
            worktree.display()
        )
    }

    /// reviewer 快照 manifest（巡检只投影 `attempt_id`）。
    fn snapshot_manifest_body(attempt_id: &str) -> String {
        format!(r#"{{"attempt_id":"{attempt_id}"}}"#)
    }

    /// 实体身份台账条目。
    fn binding_body(
        entity_kind: &str,
        entity_id: &str,
        project_id: &str,
        issue_id: &str,
    ) -> String {
        format!(
            r#"{{"project_id":"{project_id}","issue_id":"{issue_id}","entity_kind":"{entity_kind}","entity_id":"{entity_id}"}}"#
        )
    }

    /// L1 墓碑（`phase=deleted` + `deleted_at` 有值 + 无 pending-failure）。
    fn write_l1_tombstone(paths: &ProductAppPaths) {
        write(
            &paths
                .issue_root("project_0001", "issue_0001")
                .join("entity-deletions/story/story_spec_0005.json"),
            r#"{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"story","entity_id":"story_spec_0005","phase":"deleted","started_at":"2026-10-28T10:00:00Z","deleted_at":"2026-10-28T10:05:00Z"}"#,
        );
    }

    const MEMBER_ID: &str = "11111111-1111-4111-8111-111111111111";
    const CHECKOUT_ID: &str = "22222222-2222-4222-8222-222222222222";

    /// 逻辑代码库 scope 根（`projects/project_0001/logical-codebases/lc_0001`）。
    fn lc_root(paths: &ProductAppPaths) -> PathBuf {
        paths.logical_codebases_root("project_0001").join("lc_0001")
    }

    /// LC 成员记录（引用 `checkout_ids`）。
    fn member_body(checkout_ids: &[&str]) -> String {
        let checkouts: Vec<String> = checkout_ids.iter().map(|id| format!("\"{id}\"")).collect();
        format!(
            r#"{{"logical_repository_id":"{MEMBER_ID}","physical_repository_id":"repo_1","alias":"r1","role":"backend","ordinal":0,"source_identity":{{"scheme":"git_dir_only_v1","key_digest":"k","canonical_git_dir":"/tmp/git","first_seen_path_hash":"h"}},"checkout_ids":[{}],"created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}}"#,
            checkouts.join(",")
        )
    }

    /// LC checkout 记录（`logical_repository_id` 指向成员）。
    fn checkout_body(checkout_id: &str) -> String {
        format!(
            r#"{{"checkout_id":"{checkout_id}","logical_repository_id":"{MEMBER_ID}","physical_repository_id":"repo_1","kind":"main","canonical_path":"/tmp/co","checkout_path_hash":"h","git_dir_identity":"g","observed_at":"2026-10-28T10:00:00Z","created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}}"#
        )
    }

    /// LC manifest（`member_ids[]` + `active_aggregate_index_id`）。
    fn manifest_body(member_ids: &[&str], active: Option<&str>) -> String {
        let members: Vec<String> = member_ids.iter().map(|id| format!("\"{id}\"")).collect();
        let active = active
            .map(|id| format!(r#","active_aggregate_index_id":"{id}""#))
            .unwrap_or_default();
        format!(
            r#"{{"schema_version":1,"project_id":"project_0001","logical_codebase_id":"33333333-3333-4333-8333-333333333333","provider_context_root":"/tmp/lc","layout":"common_non_git_parent","membership_revision":1,"member_ids":[{}]{active},"created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}}"#,
            members.join(",")
        )
    }

    /// LC 聚合索引记录（每个成员快照引用一次成员 + 一次 checkout）。
    fn aggregate_index_body(aggregate_index_id: &str, checkout_id: &str) -> String {
        format!(
            r#"{{"aggregate_index_id":"{aggregate_index_id}","project_id":"project_0001","membership_revision":1,"status":"active","member_snapshots":[{{"logical_repository_id":"{MEMBER_ID}","checkout_id":"{checkout_id}","revision":"abc","dirty":false,"included":true,"indexed_at":"2026-10-28T10:00:00Z"}}],"codegraph_version":"1.0","codegraph_root":"/tmp/cg","created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}}"#
        )
    }

    /// 断言某目标 id 未被判悬空。
    fn assert_not_flagged(report: &PatrolReport, target_id: &str) {
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.target_id == target_id),
            "存活目标被判悬空：{target_id}；{:?}",
            report.findings
        );
    }

    // -- 分区覆盖：正常条目 ----------------------------------------------

    #[test]
    fn dangling_patrol_keeps_live_references_out_of_findings() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let issue_root = paths.issue_root("project_0001", "issue_0001");
        // 分区一 / 分区四：存活 plan（schema-v2 线路），intent 与台账条目都指向它。
        write_live_plan(&paths, "issue_work_item_plan_0007");
        write(
            &issue_root.join("automation-enrollment.json"),
            enrollment_body(),
        );
        write(
            &issue_root.join("automation-plan-intent.json"),
            &plan_intent_body("issue_work_item_plan_0007", "workspace_session_auto_x"),
        );
        write(
            &paths
                .root()
                .join("state/entity-identity/sessions/workspace_session_auto_x.json"),
            &binding_body(
                "plan",
                "issue_work_item_plan_0007",
                "project_0001",
                "issue_0001",
            ),
        );
        // 分区二 / 分区三：attempt 路一探针命中（两路任一命中即「仍在」）。
        let absent_worktree = temp.path().join("absent-worktree");
        write(
            &paths.root().join("reviewer-windows/windows/win_live.json"),
            &window_body(
                "win_live",
                "coding_attempt_live",
                "closed_accepted",
                &absent_worktree,
            ),
        );
        write(
            &paths
                .root()
                .join("reviewer-snapshots/windows/win_live/manifest.json"),
            &snapshot_manifest_body("coding_attempt_live"),
        );
        // 分区五：成员 / checkout / 聚合索引三者互相命中。
        write(
            &lc_root(&paths).join("manifest.json"),
            &manifest_body(&[MEMBER_ID], Some("agg_0001")),
        );
        write(
            &lc_root(&paths).join(format!("members/{MEMBER_ID}.json")),
            &member_body(&[CHECKOUT_ID]),
        );
        write(
            &lc_root(&paths).join(format!("checkouts/{CHECKOUT_ID}.json")),
            &checkout_body(CHECKOUT_ID),
        );
        write(
            &lc_root(&paths).join("aggregate-indexes/agg_0001.json"),
            &aggregate_index_body("agg_0001", CHECKOUT_ID),
        );

        let probe = probe_alive(&["coding_attempt_live"]);
        let report = patrol_state_root_with_probe(&paths, Some(&probe)).expect("patrol");
        // 存活的 plan / attempt / 成员 / checkout / 聚合索引一律不产发现。
        for target in [
            "issue_work_item_plan_0007",
            "coding_attempt_live",
            MEMBER_ID,
            CHECKOUT_ID,
            "agg_0001",
        ] {
            assert_not_flagged(&report, target);
        }
        assert!(
            report.undetermined.is_empty(),
            "存活目标不得进不可判定：{:?}",
            report.undetermined
        );
        // 唯一残留发现 = 未落盘 session 记录的 `session_id`（正常中间态，只报不删）。
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(report.findings[0].reference_field, "session_id");
        assert_eq!(
            report.findings[0].proposed_action,
            PATROL_ACTION_REPORT_ONLY
        );
    }

    // -- 判据表字段登记完整性（REQ-DRG-09） ------------------------------

    #[test]
    fn dangling_patrol_registers_every_enrollment_reference_field() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        write(
            &paths
                .issue_root("project_0001", "issue_0001")
                .join("automation-enrollment.json"),
            r#"{"enrollment_id":"e1","selection_key":"s1","project_id":"project_0001","issue_id":"issue_0001","enabled":false,"policy_revision":1,"source":{"stories":[{"id":"story_spec_0001","version":1}],"designs":[{"id":"design_spec_0002","version":1}]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"prepare_intent_id":"p1","plan_id":"issue_work_item_plan_0003","session_id":"workspace_session_auto_0004","binding_history":{"current":{"binding_version":2,"enrollment_id":"e1","plan_id":"issue_work_item_plan_0005","session_id":"workspace_session_auto_0006","source":{"stories":[{"id":"story_spec_0007","version":1}],"designs":[{"id":"design_spec_0008","version":1}]},"target":{"kind":"single_repository","repository_id":"repo_1"},"author_provider":"codex","reviewer_provider":"codex"},"previous":[{"binding_version":1,"enrollment_id":"e1","plan_id":"issue_work_item_plan_0011","session_id":"workspace_session_auto_0012","source":{"stories":[{"id":"story_spec_0009","version":1}],"designs":[{"id":"design_spec_0010","version":1}]},"target":{"kind":"single_repository","repository_id":"repo_1"},"author_provider":"codex","reviewer_provider":"codex"}]},"created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}"#,
        );

        let report = patrol_state_root(&paths).expect("patrol");
        // 判据表登记的十个 enrollment 引用字段逐个必须出现在发现里（漏检防线）。
        let expected = [
            "source.stories[].id",
            "source.designs[].id",
            "plan_id",
            "session_id",
            "binding_history.current.plan_id",
            "binding_history.current.session_id",
            "binding_history.current.source.stories[].id",
            "binding_history.current.source.designs[].id",
            "binding_history.previous[].source.stories[].id",
            "binding_history.previous[].source.designs[].id",
        ];
        let fields: BTreeSet<&str> = report
            .findings
            .iter()
            .map(|finding| finding.reference_field.as_str())
            .collect();
        for field in expected {
            assert!(
                fields.contains(field),
                "判据表字段未登记：{field}；实际 {fields:?}"
            );
        }
        assert_eq!(
            report.findings.len(),
            expected.len(),
            "{:?}",
            report.findings
        );
        assert!(report.undetermined.is_empty(), "{:?}", report.undetermined);
        // 七列齐全 + enrollment 引用一律只报不删（§3.4.1 降级档位）。
        for finding in &report.findings {
            assert_eq!(finding.record_type, "automation_enrollment");
            assert!(!finding.record_path.is_empty());
            assert!(!finding.reference_field.is_empty());
            assert!(!finding.target_kind.is_empty());
            assert!(!finding.target_id.is_empty());
            assert_eq!(finding.evidence_level, EVIDENCE_LABEL_INSUFFICIENT);
            assert_eq!(finding.proposed_action, PATROL_ACTION_REPORT_ONLY);
        }
    }

    // -- 分区覆盖：悬空 → 发现（含 LC 三类真实读） -----------------------

    #[test]
    fn dangling_patrol_lc_partition_flags_dangling_member_and_checkout() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let missing_member = "44444444-4444-4444-8444-444444444444";
        let missing_checkout = "55555555-5555-4555-8555-555555555555";
        write(
            &lc_root(&paths).join("manifest.json"),
            &manifest_body(&[missing_member], Some("agg_missing")),
        );
        write(
            &lc_root(&paths).join(format!("members/{MEMBER_ID}.json")),
            &member_body(&[missing_checkout]),
        );

        let report = patrol_state_root(&paths).expect("patrol");
        // 三条：manifest.member_ids[]、manifest.active_aggregate_index_id、member.checkout_ids[]。
        assert_eq!(report.findings.len(), 3, "{:?}", report.findings);
        let fields: BTreeSet<&str> = report
            .findings
            .iter()
            .map(|finding| finding.reference_field.as_str())
            .collect();
        assert!(fields.contains("member_ids[]"), "{fields:?}");
        assert!(fields.contains("active_aggregate_index_id"), "{fields:?}");
        assert!(fields.contains("checkout_ids[]"), "{fields:?}");
        for finding in &report.findings {
            assert_eq!(finding.record_type, "logical_codebase_member");
            assert_eq!(
                finding.proposed_action, PATROL_ACTION_REPORT_ONLY,
                "LC 三类恒只报不删"
            );
        }
        assert!(report.undetermined.is_empty(), "{:?}", report.undetermined);
    }

    // -- 分区覆盖：U1 类 / 数据源不可用 → 不可判定 -----------------------

    #[test]
    fn dangling_patrol_marks_unavailable_sources_undetermined() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        // 分区五：成员指向 checkout，但 checkouts 数据源不可读（写成普通文件）。
        write(
            &lc_root(&paths).join(format!("members/{MEMBER_ID}.json")),
            &member_body(&[CHECKOUT_ID]),
        );
        write(&lc_root(&paths).join("checkouts"), "not a directory");
        // 分区四：台账条目缺所属 project/issue 作用域 → 无法核对。
        write(
            &paths
                .root()
                .join("state/entity-identity/sessions/workspace_session_auto_q.json"),
            &binding_body("plan", "issue_work_item_plan_0007", "", ""),
        );

        let report = patrol_state_root(&paths).expect("patrol");
        assert!(
            report.findings.is_empty(),
            "不可判定不得计入悬空：{:?}",
            report.findings
        );
        assert_eq!(report.undetermined.len(), 2, "{:?}", report.undetermined);
        assert!(
            report
                .undetermined
                .iter()
                .any(|entry| entry.reference_field == "checkout_ids[]"),
            "{:?}",
            report.undetermined
        );
        assert!(
            report
                .undetermined
                .iter()
                .any(|entry| entry.reference_field == "entity_kind+entity_id"
                    && entry.reason.contains("作用域")),
            "{:?}",
            report.undetermined
        );
    }

    // -- 分区覆盖：未归类 → 只报不删 -------------------------------------

    #[test]
    fn dangling_patrol_reports_unclassified_records_separately() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        // issue 分区表外记录。
        write(
            &paths
                .issue_root("project_0001", "issue_0001")
                .join("automation-legacy-ledger.json"),
            "{}",
        );
        // 实体身份台账分区表外记录。
        write(
            &paths.state_root().join("entity-identity/ledger.json"),
            "{}",
        );
        // 快照窗口目录里的表外记录（`restore-cursor.json`）+ 一份悬空 manifest。
        write(
            &paths
                .root()
                .join("reviewer-snapshots/windows/win_1/restore-cursor.json"),
            "{}",
        );
        write(
            &paths
                .root()
                .join("reviewer-snapshots/windows/win_1/manifest.json"),
            &snapshot_manifest_body("coding_attempt_absent"),
        );

        let report = patrol_state_root(&paths).expect("patrol");
        assert_eq!(report.unclassified.len(), 3, "{:?}", report.unclassified);
        for entry in &report.unclassified {
            assert_eq!(entry.record_type, "unclassified");
        }
        assert!(
            report
                .unclassified
                .iter()
                .any(|entry| entry.record_path.ends_with("automation-legacy-ledger.json"))
        );
        assert!(
            report
                .unclassified
                .iter()
                .any(|entry| entry.record_path.ends_with("entity-identity/ledger.json"))
        );
        assert!(
            report
                .unclassified
                .iter()
                .any(|entry| entry.record_path.ends_with("restore-cursor.json"))
        );
        assert!(
            report
                .covered_record_types
                .iter()
                .any(|kind| kind == "unclassified")
        );
        // 未归类不进悬空明细：唯一发现来自快照 manifest 的悬空 attempt。
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(report.findings[0].target_id, "coding_attempt_absent");
        assert!(report.findings[0].record_path.ends_with("manifest.json"));
    }

    // -- 墓碑不判悬空（REQ-DRG-09） --------------------------------------

    #[test]
    fn dangling_patrol_never_flags_deletion_tombstones() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        write_l1_tombstone(&paths);

        let report = patrol_state_root(&paths).expect("patrol");
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert!(report.unclassified.is_empty(), "{:?}", report.unclassified);
        assert!(report.undetermined.is_empty(), "{:?}", report.undetermined);
    }

    // -- 证据等级 → 建议动作 ---------------------------------------------

    #[test]
    fn dangling_patrol_actions_follow_evidence_levels() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let issue_root = paths.issue_root("project_0001", "issue_0001");
        // L1：story 有完整墓碑 → 建议删除记录。
        write_l1_tombstone(&paths);
        write(
            &issue_root.join("automation-plan-intent.json"),
            r#"{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x"}"#,
        );
        // L2：窗口终态 + 无租约 + 两路目标皆无 + worktree 已回收 → 建议按窗口簇删除。
        let absent_worktree = temp.path().join("absent-worktree");
        write(
            &paths.root().join("reviewer-windows/windows/win_l2.json"),
            &window_body(
                "win_l2",
                "coding_attempt_gone",
                "closed_accepted",
                &absent_worktree,
            ),
        );

        let report = patrol_state_root(&paths).expect("patrol");
        let story = report
            .findings
            .iter()
            .find(|finding| finding.target_id == "story_spec_0005")
            .expect("story 悬空发现");
        assert_eq!(story.evidence_level, EVIDENCE_LABEL_L1);
        assert_eq!(story.proposed_action, PATROL_ACTION_DELETE);
        let plan = report
            .findings
            .iter()
            .find(|finding| finding.reference_field == "plan_id")
            .expect("plan 悬空发现");
        assert_eq!(
            plan.evidence_level, EVIDENCE_LABEL_INSUFFICIENT,
            "无墓碑不得判 L1"
        );
        assert_eq!(plan.proposed_action, PATROL_ACTION_REPORT_ONLY);
        let session = report
            .findings
            .iter()
            .find(|finding| finding.reference_field == "session_id")
            .expect("session 悬空发现");
        assert_eq!(
            session.proposed_action, PATROL_ACTION_REPORT_ONLY,
            "session 恒只报不删"
        );
        let attempt = report
            .findings
            .iter()
            .find(|finding| finding.reference_field == "attempt_id")
            .expect("attempt 悬空发现");
        assert!(
            attempt.evidence_level.starts_with(EVIDENCE_LABEL_L2),
            "{}",
            attempt.evidence_level
        );
        assert_eq!(attempt.proposed_action, PATROL_ACTION_DELETE_GROUP);
        // 墓碑自身绝不进悬空明细 / 未归类。
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.record_path.contains("entity-deletions"))
        );
        assert!(report.unclassified.is_empty(), "{:?}", report.unclassified);
    }

    #[test]
    fn dangling_patrol_window_cluster_requires_all_windows() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let absent_worktree = temp.path().join("absent-worktree");
        let windows_dir = paths.root().join("reviewer-windows/windows");
        // 同一 attempt 两个窗口：一个符合四条件，一个仍非终态 → 整簇不删（r3 N-9）。
        write(
            &windows_dir.join("win_a.json"),
            &window_body(
                "win_a",
                "coding_attempt_gone",
                "closed_accepted",
                &absent_worktree,
            ),
        );
        write(
            &windows_dir.join("win_b.json"),
            &window_body("win_b", "coding_attempt_gone", "running", &absent_worktree),
        );

        let report = patrol_state_root(&paths).expect("patrol");
        assert_eq!(report.findings.len(), 2, "{:?}", report.findings);
        for finding in &report.findings {
            assert_eq!(finding.reference_field, "attempt_id");
            assert_eq!(
                finding.proposed_action, PATROL_ACTION_REPORT_ONLY,
                "同簇任一不满足即整簇不删"
            );
            assert!(
                finding
                    .evidence_level
                    .starts_with(EVIDENCE_LABEL_INSUFFICIENT),
                "{}",
                finding.evidence_level
            );
            // 摘要必须反映不成立的那一扇窗口（非终态），不得用合规窗口掩盖。
            assert!(
                finding.evidence_level.contains("W-1 终态：不成立"),
                "{}",
                finding.evidence_level
            );
        }
    }

    // -- 路二未能枚举 ≠ 目标不存在（REQ-DRG-21 / 23） --------------------

    #[test]
    fn dangling_patrol_isolates_unknown_when_session_enumeration_fails() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        // workspace-sessions 写成普通文件 → 路二枚举失败。
        write(
            &paths
                .issue_root("project_0001", "issue_0001")
                .join("workspace-sessions"),
            "not a directory",
        );
        let absent_worktree = temp.path().join("absent-worktree");
        write(
            &paths.root().join("reviewer-windows/windows/win_gone.json"),
            &window_body(
                "win_gone",
                "coding_attempt_gone",
                "closed_accepted",
                &absent_worktree,
            ),
        );

        let probe = probe_false();
        let report = patrol_state_root_with_probe(&paths, Some(&probe)).expect("patrol");
        assert!(
            report.findings.is_empty(),
            "未能枚举不得当作目标不存在：{:?}",
            report.findings
        );
        assert_eq!(report.undetermined.len(), 1, "{:?}", report.undetermined);
        assert_eq!(report.undetermined[0].reference_field, "attempt_id");
        assert!(
            report.undetermined[0].reason.contains("枚举"),
            "{}",
            report.undetermined[0].reason
        );
    }

    // -- 统计口径：三栏互不混算 ------------------------------------------

    #[test]
    fn dangling_patrol_keeps_counts_separate() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let absent_worktree = temp.path().join("absent-worktree");
        // 一栏一条：悬空（窗口 attempt 已删）/ 不可判定（台账缺作用域）/ 未归类。
        write(
            &paths.root().join("reviewer-windows/windows/win_gone.json"),
            &window_body(
                "win_gone",
                "coding_attempt_gone",
                "closed_accepted",
                &absent_worktree,
            ),
        );
        write(
            &paths
                .root()
                .join("state/entity-identity/sessions/workspace_session_auto_q.json"),
            &binding_body("plan", "issue_work_item_plan_0007", "", ""),
        );
        write(
            &paths
                .issue_root("project_0001", "issue_0001")
                .join("automation-legacy-ledger.json"),
            "{}",
        );

        let report = patrol_state_root(&paths).expect("patrol");
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(report.undetermined.len(), 1, "{:?}", report.undetermined);
        assert_eq!(report.unclassified.len(), 1, "{:?}", report.unclassified);
        assert_eq!(report.findings[0].target_id, "coding_attempt_gone");
        assert_eq!(
            report.undetermined[0].record_path,
            "state/entity-identity/sessions/workspace_session_auto_q.json"
        );
        assert_eq!(
            report.unclassified[0].record_path,
            "projects/project_0001/issues/issue_0001/automation-legacy-ledger.json"
        );
    }
}
