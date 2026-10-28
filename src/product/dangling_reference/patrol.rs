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
    DanglingRecordType, DanglingTargetKind, DeletableLevel, ScopeRequirement, TargetEvidencePolicy,
    evidence_policy, primary_target_kind, reference_rule,
};
use crate::product::dangling_reference::resolve::{
    AttemptProbe, DanglingResolver, EnrollmentReader, EvidenceLevel, ExistenceEnv, LcScopeReader,
    TargetExistence, UndeterminedCause, dispatch_existence, window_l2_verdict,
};
use crate::product::entity_deletion_store::EntityScope;
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

// ---------------------------------------------------------------------------
// 子模块拆分（B 任务）：生产段按职责拆到 patrol/ 子目录、内联测试拆到
// patrol_tests.rs，均为纯移动 + 必要的 pub(super) 可见性放宽，零逻辑变化。
// ---------------------------------------------------------------------------

mod context;
mod fs_util;
mod identity_partition;
mod issue_partition;
mod lc_partition;
mod report_builder;
mod snapshot_partition;
mod window_partition;

use context::*;
use fs_util::*;
use identity_partition::scan_entity_identity_partition;
use issue_partition::scan_issue_partition;
use lc_partition::scan_logical_codebase_partition;
use report_builder::*;
use snapshot_partition::scan_snapshot_partition;
use window_partition::{build_attempt_clusters, emit_window_findings, scan_window_partition};

#[cfg(test)]
#[path = "patrol_tests.rs"]
mod patrol_tests;

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
        session_unavailable_reason: if session_errors.is_empty() {
            None
        } else {
            Some(format!(
                "存在性数据源 / 作用域不可用：workspace session 枚举失败，路二无法完成（{}）",
                session_errors.join("；")
            ))
        },
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
