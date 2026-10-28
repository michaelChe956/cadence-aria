//! 销账编排（REQ-DRG-13 / 14 / 15 / 16 / 17；技术方案 §3.4.2 / §3.4.2.1 / §3.5.2）。
//!
//! 本模块是第 ③ 层里**唯一真正执行删除**的一环，三道闸在同一入口 [`run_cleanup`]
//! 内强制，顺序不可颠倒：
//!
//! 1. **闸 1（默认 dry-run + 确认令牌）**：入口先**内部重跑一次巡检**并重算 findings
//!    摘要令牌（REQ-DRG-22，[`findings_token`]）。`apply == false` → 只出清单（零删除、
//!    零留痕）；`apply == true` 缺令牌或令牌与重算值不一致 → `Err`，同样**零删除、
//!    零留痕**。令牌比对用本进程本次现场，不用任何外部清单文件（「拿旧清单开新清理」
//!    与「清单生成后现场又变了」两种错配一次挡住）。
//! 2. **闸 2（按类型白名单）**：删除集合只来自 Task 7 判定面的
//!    [`CleanupDecision::Delete`]，本模块再按记录类型做一次白名单分派（intent /
//!    窗口簇 / 台账条目三类）；表外类型、未归类记录、降级类型（checkout /
//!    aggregate_index / logical_member / session / enrollment）与证据不足者**一律不进
//!    删除集**。墓碑（`entity-deletions/*`）与用户业务数据（story / design / plan 正文、
//!    versions、artifact、work-item、attempt）**永不删除**。窗口簇删除 MUST 交给既有
//!    物理清理器 `ReviewerResourcePurger::purge_for_attempts`（不自写删除逻辑）；台账
//!    条目只删 `state/entity-identity/sessions/` 直接子项；待删路径沿用 Task 4 护栏
//!    （相对路径 + 结构校验 + canonical 父目录必须仍在状态根内 + 符号链接不跟随）。
//! 3. **闸 3（幂等 + 留痕 + 自证归零）**：目标记录缺失视为已清（不报错、不重复删）；
//!    每次实际删除向 Task 6 的单一全局 JSONL 台账追加一行，台账按 `record_path` +
//!    `target_id` + `action` 全量比对去重（重入不追加重复行）；删除完成后**再巡检一次**
//!    自证：归零计数只统计销账白名单内「本次可删」的条目（按 `record_type` 计数，已归零
//!    的类型不出现在计数里），「只报不删残留」归入 `residual`、「不可判定」归入
//!    `undetermined`，二者**都不计入**归零；`governance_complete` 只在「无未归零可删
//!    条目 + 无残留 + 无不可判定」时为真（REQ-DRG-17）。
//!
//! **intent 删除与 enrollment 文件锁**：判定由 Task 7 在 `automation-enrollment.json`
//! 的文件锁内完成（I-1 / I-2）；本模块的 `remove_file` 也 MUST 在同一把锁内执行
//! （REQ-DRG-15「幂等不得免除锁语义」）。`flock` 不可重入（同进程第二个 `File` 描述符
//! 会被自己挡住），故判定与删除是**两段临界区**：删除段进入锁后重读 intent 内容，与
//! 判定前快照逐字节比对——内容被并发改写即本轮不删（fail-closed），MUST NOT 在锁外删。
//!
//! **失败口径**：读取失败一律上抛（`CleanupError::Store`），MUST NOT 折叠成「目标已删」；
//! workspace session 枚举失败同样上抛（不拿残缺集合把活窗口判成悬空）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::dangling_reference::cleanup_rules::{
    CleanupDecision, decide_intent_cleanup, decide_ledger_entry_cleanup,
    decide_window_cluster_cleanup,
};
use crate::product::dangling_reference::ledger::{
    DanglingReferenceLedger, DeletedEntityRef, LedgerRow, cleanup_id,
};
use crate::product::dangling_reference::listing::EntityIdentityBindingEntry;
use crate::product::dangling_reference::patrol::{
    PatrolFinding, UndeterminedEntry, patrol_state_root,
};
use crate::product::dangling_reference::reference_table::{DanglingRecordType, DanglingTargetKind};
use crate::product::dangling_reference::report::findings_token;
use crate::product::dangling_reference::resolve::{AttemptProbe, DanglingResolver};
use crate::product::json_store::{ProductStoreError, read_json};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::reviewer_resource_purge::ReviewerResourcePurger;

/// 留痕行的触发来源稳定短码（REQ-DRG-16；设计 §3.5.2 行形态）。
const CLEANUP_TRIGGER: &str = "dangling_reference_cleanup";
/// 留痕行格式版本（与既有根级 JSONL 审计同规范）。
const LEDGER_SCHEMA_VERSION: u32 = 1;
/// 清理批次标识的时间戳格式（`run_id = cleanup_<stamp>`）。
const RUN_ID_STAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";
/// 自动化 enrollment 文件名（既有落盘约定；同时是 intent 删除的锁目标）。
const ENROLLMENT_FILE: &str = "automation-enrollment.json";
/// 计划意图文件名（既有落盘约定，MUST NOT 枚举目录其它改名）。
const PLAN_INTENT_FILE: &str = "automation-plan-intent.json";
/// 生成检查点文件名（既有落盘约定）。
const GENERATION_INTENT_FILE: &str = "automation-generation-intent.json";
/// 实体身份台账分区目录段（`<root>/state/entity-identity/sessions/`）。
const ENTITY_IDENTITY_DIR: &str = "entity-identity";
/// 实体身份台账绑定条目目录段。
const ENTITY_SESSIONS_DIR: &str = "sessions";

/// 一次销账请求（闸 1 的两个开关）。
///
/// `apply == false` 是**默认**：只出清单、零删除、零留痕。`apply == true` 时
/// `confirm_token` 必填，且 MUST 等于入口内部重算的 findings 摘要。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CleanupRequest {
    /// 是否真的执行删除（默认 `false` = dry-run）。
    pub(crate) apply: bool,
    /// 确认令牌（`sha256:<64 位小写 hex>`）；dry-run 下忽略。
    pub(crate) confirm_token: Option<String>,
}

/// 一次销账编排的结果。
///
/// `deleted_count` 按**销账决策**计：intent 文件一条、台账条目一条、窗口簇按
/// 「一个 attempt 一条簇」计（簇内窗口 / 租约 / 闸门 / 快照由既有清理器一并清理，
/// 不在此逐物计数）。
#[derive(Debug, Clone, Default)]
pub(crate) struct CleanupOutcome {
    /// 本次实际执行的删除条数（重入时为 0）。
    pub(crate) deleted_count: usize,
    /// 只报不删的判定清单（Task 7 的 `ReportOnly`，原样转出）。
    pub(crate) report_only: Vec<CleanupDecision>,
    /// 自证归零计数：`record_type` → **仍未归零的可删条目数**。已归零的类型不出现
    /// 在此表中（空表 = 白名单内可删类型全部归零）。
    pub(crate) zeroed_by_type: BTreeMap<String, usize>,
    /// 「只报不删残留」：自证巡检里**不可删**的悬空引用（证据不足 / 表外 / 降级），
    /// 不计入归零（REQ-DRG-17）。
    pub(crate) residual: Vec<PatrolFinding>,
    /// 「不可判定」条目（REQ-DRG-23）：读取失败或所需数据源 / 作用域不可用。
    pub(crate) undetermined: Vec<UndeterminedEntry>,
    /// 是否治理完成：白名单内可删类型全部归零 **且** 无残留 **且** 无不可判定。
    pub(crate) governance_complete: bool,
}

/// 闸 1 的拒绝原因（与存储失败分开，供 CLI 映射稳定错误码）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CleanupGateError {
    /// `apply == true` 但没给确认令牌。
    TokenMissing,
    /// 令牌与入口内部重算的 findings 摘要不一致（`expected` = 重算值，
    /// `actual` = 传入值）。
    TokenMismatch {
        /// 本次现场重算出的令牌。
        expected: String,
        /// 调用方传入的令牌。
        actual: String,
    },
}

/// 销账失败：闸 1 拒绝 或 存储 / IO 失败。
#[derive(Debug)]
pub(crate) enum CleanupError {
    /// 闸 1 拒绝（此时 MUST 已保证零删除、零留痕）。
    Gate(CleanupGateError),
    /// 巡检 / 判定 / 删除 / 留痕过程中的存储错误。
    Store(ProductStoreError),
}

impl std::fmt::Display for CleanupGateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TokenMissing => write!(
                formatter,
                "销账被拒绝：缺少确认令牌（--apply 必须同时给出 --confirm <token>）"
            ),
            Self::TokenMismatch { expected, actual } => write!(
                formatter,
                "销账被拒绝：确认令牌与当前巡检结果不匹配（本次重算 {expected}，传入 {actual}）"
            ),
        }
    }
}

impl std::fmt::Display for CleanupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gate(error) => write!(formatter, "{error}"),
            Self::Store(error) => write!(formatter, "销账失败：{error}"),
        }
    }
}

impl std::error::Error for CleanupError {}

/// 执行一次销账编排（三道闸全部在本入口内强制；见模块文档）。
///
/// 返回 [`CleanupOutcome`]；闸 1 拒绝或存储失败返回 [`CleanupError`]。
pub(crate) fn run_cleanup(
    paths: &ProductAppPaths,
    request: &CleanupRequest,
) -> Result<CleanupOutcome, CleanupError> {
    run_cleanup_with_attempt_probe(paths, request, None)
}

/// 销账主体（可注入 attempt 路一探针；生产路径 `None` = 内置 `CodingAttemptStore`）。
fn run_cleanup_with_attempt_probe(
    paths: &ProductAppPaths,
    request: &CleanupRequest,
    attempt_probe: Option<AttemptProbe<'_>>,
) -> Result<CleanupOutcome, CleanupError> {
    // 闸 1 ①：入口内部重跑一次巡检（同进程、同一现场），重算 findings 摘要。
    let before = patrol_state_root(paths).map_err(CleanupError::Store)?;
    let expected_token = findings_token(&before.findings);
    if request.apply {
        // 闸 1 ②：缺令牌 / 令牌不一致 → 拒绝，且此前的动作全部只读（零删除、零留痕）。
        let provided = request
            .confirm_token
            .as_deref()
            .ok_or(CleanupError::Gate(CleanupGateError::TokenMissing))?;
        if provided != expected_token.as_str() {
            return Err(CleanupError::Gate(CleanupGateError::TokenMismatch {
                expected: expected_token,
                actual: provided.to_string(),
            }));
        }
    }

    // 闸 2：删除集合只来自按类型判定面（本步只读）。
    let decisions = collect_decisions(paths, attempt_probe).map_err(CleanupError::Store)?;
    if !request.apply {
        // 默认 dry-run：只出清单，零删除、零留痕。
        let (zeroed_by_type, residual) = classify_findings(&before.findings, &decisions);
        return Ok(finish_outcome(CleanupOutcome {
            deleted_count: 0,
            report_only: report_only_of(&decisions),
            zeroed_by_type,
            residual,
            undetermined: before.undetermined,
            governance_complete: false,
        }));
    }

    // 闸 3 ①②：执行删除 + 逐行留痕。
    let now = Utc::now();
    let run_id = format!("cleanup_{}", now.format(RUN_ID_STAMP_FORMAT));
    let deleted_count =
        execute_deletions(paths, &decisions, &run_id, now).map_err(CleanupError::Store)?;

    // 闸 3 ③：清理后再巡检一次自证归零。
    let after = patrol_state_root(paths).map_err(CleanupError::Store)?;
    let after_decisions = collect_decisions(paths, attempt_probe).map_err(CleanupError::Store)?;
    let (zeroed_by_type, residual) = classify_findings(&after.findings, &after_decisions);
    Ok(finish_outcome(CleanupOutcome {
        deleted_count,
        report_only: report_only_of(&decisions),
        zeroed_by_type,
        residual,
        undetermined: after.undetermined,
        governance_complete: false,
    }))
}

/// 收口：`governance_complete` 只在三类计数**全空**时为真（REQ-DRG-17）。
fn finish_outcome(mut outcome: CleanupOutcome) -> CleanupOutcome {
    outcome.governance_complete = outcome.zeroed_by_type.is_empty()
        && outcome.residual.is_empty()
        && outcome.undetermined.is_empty();
    outcome
}

/// 把判定清单里的「只报不删」原样摘出（`Delete` 不进本表）。
fn report_only_of(decisions: &[CleanupDecision]) -> Vec<CleanupDecision> {
    decisions
        .iter()
        .filter(|decision| matches!(decision, CleanupDecision::ReportOnly { .. }))
        .cloned()
        .collect()
}

/// 自证口径：按判定面把 findings 分成「仍未归零的可删条目」与「只报不删残留」。
///
/// 可删性**只认判定面**（本次可删 = 该条记录 / 该 attempt 簇有 `Delete` 决策），
/// 所以证据不足的悬空条目（例如缺 L1 的 intent）永远不会进入归零计数
/// （REQ-DRG-17：只报不删残留不计入归零）。
fn classify_findings(
    findings: &[PatrolFinding],
    decisions: &[CleanupDecision],
) -> (BTreeMap<String, usize>, Vec<PatrolFinding>) {
    let mut deletable_paths: BTreeSet<&str> = BTreeSet::new();
    let mut deletable_attempts: BTreeSet<&str> = BTreeSet::new();
    for decision in decisions {
        let CleanupDecision::Delete {
            record_path,
            target_kind,
            target_id,
            group_key,
            ..
        } = decision
        else {
            continue;
        };
        deletable_paths.insert(record_path.as_str());
        if group_key.is_some() && target_kind == DanglingTargetKind::Attempt.as_str() {
            deletable_attempts.insert(target_id.as_str());
        }
    }
    let mut zeroed_by_type: BTreeMap<String, usize> = BTreeMap::new();
    let mut residual = Vec::new();
    for finding in findings {
        let deletable = deletable_paths.contains(finding.record_path.as_str())
            || (is_attempt_owned_record(&finding.record_type)
                && deletable_attempts.contains(finding.target_id.as_str()));
        if deletable {
            *zeroed_by_type
                .entry(finding.record_type.clone())
                .or_insert(0) += 1;
        } else {
            residual.push(finding.clone());
        }
    }
    (zeroed_by_type, residual)
}

/// 按 attempt 归属删除的记录类型（窗口簇及其快照基线）。
fn is_attempt_owned_record(record_type: &str) -> bool {
    record_type == DanglingRecordType::ReviewerWindow.as_str()
        || record_type == DanglingRecordType::ReviewerSnapshotManifest.as_str()
}

/// 收集按类型销账判定（Task 7 的三条判定面；本步只读、不删除）。
fn collect_decisions(
    paths: &ProductAppPaths,
    attempt_probe: Option<AttemptProbe<'_>>,
) -> Result<Vec<CleanupDecision>, ProductStoreError> {
    let sessions = workspace_session_ids(paths)?;
    let resolver = match attempt_probe {
        Some(probe) => DanglingResolver::with_attempt_probe(paths, &sessions, probe),
        None => DanglingResolver::new(paths, &sessions),
    };
    let mut decisions = Vec::new();
    // ① 自动化 intent：只认两个白名单文件名；记录不存在即无判定对象（重入视为已清）。
    for (project_id, issue_id) in issue_scopes(paths)? {
        for file_name in [PLAN_INTENT_FILE, GENERATION_INTENT_FILE] {
            let path = paths.issue_root(&project_id, &issue_id).join(file_name);
            if !path_exists(&path)? {
                continue;
            }
            decisions.push(decide_intent_cleanup(
                paths,
                &project_id,
                &issue_id,
                file_name,
                &resolver,
            )?);
        }
    }
    // ② reviewer 窗口簇（Task 7 按 attempt 聚合）。
    decisions.extend(decide_window_cluster_cleanup(paths, &resolver)?);
    // ③ 实体身份台账条目（只认 plan + L1；其余条目本步只报不删）。
    for entry in entity_identity_entries(paths)? {
        decisions.push(decide_ledger_entry_cleanup(paths, &entry, &resolver)?);
    }
    Ok(decisions)
}

/// 规划面 workspace session 全量 id 集合（REQ-DRG-21 路二）。
///
/// MUST 用枚举接口；枚举失败 fail-closed 上抛（拿残缺集合会把活窗口判成悬空）。
fn workspace_session_ids(paths: &ProductAppPaths) -> Result<BTreeSet<String>, ProductStoreError> {
    let mut ids = BTreeSet::new();
    for (project_id, issue_id) in issue_scopes(paths)? {
        let records =
            LifecycleStore::new(paths.clone()).list_workspace_sessions(&project_id, &issue_id)?;
        ids.extend(records.into_iter().map(|record| record.id));
    }
    Ok(ids)
}

/// 状态根下的 project × issue 作用域（读根失败上抛：连结构都不通时无从判定）。
fn issue_scopes(paths: &ProductAppPaths) -> Result<Vec<(String, String)>, ProductStoreError> {
    let mut scopes = Vec::new();
    for project_id in child_directory_names(&paths.projects_root())? {
        for issue_id in child_directory_names(&paths.project_root(&project_id).join("issues"))? {
            scopes.push((project_id.clone(), issue_id));
        }
    }
    Ok(scopes)
}

/// 目录下的子目录名（目录不存在 → 空集；其余 IO 失败上抛）。
fn child_directory_names(dir: &Path) -> Result<BTreeSet<String>, ProductStoreError> {
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
fn read_dir_sorted(dir: &Path) -> Result<Vec<(PathBuf, bool)>, ProductStoreError> {
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

/// 枚举实体身份台账条目（`<root>/state/entity-identity/sessions/*.json`）。
///
/// 单条解析 / 读取失败**不产生删除决策**（MUST NOT 猜引用语义、MUST NOT 放宽闸门）；
/// 该失败由巡检侧记入「不可判定」，自证阶段因此必然不为「治理完成」。
fn entity_identity_entries(
    paths: &ProductAppPaths,
) -> Result<Vec<EntityIdentityBindingEntry>, ProductStoreError> {
    let sessions_root = paths
        .state_root()
        .join(ENTITY_IDENTITY_DIR)
        .join(ENTITY_SESSIONS_DIR);
    let mut entries = Vec::new();
    for (path, is_dir) in read_dir_sorted(&sessions_root)? {
        if is_dir || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(value) = read_json::<serde_json::Value>(&path) else {
            continue;
        };
        entries.push(EntityIdentityBindingEntry {
            record_path: record_path_of(paths, &path),
            session_id: path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string(),
            entity_kind: json_string_field(&value, "entity_kind"),
            entity_id: json_string_field(&value, "entity_id"),
            project_id: json_string_field(&value, "project_id"),
            issue_id: json_string_field(&value, "issue_id"),
        });
    }
    Ok(entries)
}

/// 取 JSON 对象里的字符串字段（缺失 / 非字符串 → 空串，与既有列举面同口径）。
fn json_string_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// 记录路径（相对应用状态根 `paths.root()`）。
fn record_path_of(paths: &ProductAppPaths, path: &Path) -> String {
    path.strip_prefix(paths.root())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// 存在性判定（stat）：IO 失败上抛，MUST NOT 被当成「不存在」。
fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| io_error("stat", path, &error))
}

/// 统一的 IO 错误包装。
fn io_error(action: &str, path: &Path, error: &std::io::Error) -> ProductStoreError {
    ProductStoreError::Io(format!("{action} {}: {error}", path.display()))
}

/// 一条待执行删除的留痕事实（从 [`CleanupDecision::Delete`] 摘出，避免参数串）。
struct DeletionFacts<'a> {
    record_path: &'a str,
    record_type: &'a str,
    target_kind: &'a str,
    target_id: &'a str,
    evidence_level: &'a str,
    action: &'a str,
    group_key: Option<&'a str>,
}

/// 执行全部 `Delete` 决策（闸 2 分派 + 闸 3 幂等与留痕），返回实际删除条数。
fn execute_deletions(
    paths: &ProductAppPaths,
    decisions: &[CleanupDecision],
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<usize, ProductStoreError> {
    let ledger = DanglingReferenceLedger::new(paths);
    let mut deleted = 0usize;
    for decision in decisions {
        let CleanupDecision::Delete {
            record_path,
            record_type,
            target_kind,
            target_id,
            evidence_level,
            action,
            group_key,
        } = decision
        else {
            // 闸 2：只报不删的判定一律不动手。
            continue;
        };
        let facts = DeletionFacts {
            record_path,
            record_type,
            target_kind,
            target_id,
            evidence_level,
            action,
            group_key: group_key.as_deref(),
        };
        // 闸 2：按记录类型再做一次白名单分派（表外 / 降级类型恒不删）。
        let row = match record_type.as_str() {
            value
                if value == DanglingRecordType::AutomationPlanIntent.as_str()
                    || value == DanglingRecordType::AutomationGenerationIntent.as_str() =>
            {
                delete_intent_file(paths, &facts, run_id, now)?
            }
            value if value == DanglingRecordType::ReviewerWindow.as_str() => {
                // 窗口簇：复用既有物理清理器（活跃 / 归档窗口、租约、闸门、违规
                // journal、快照基线目录一并清；「缺失 = 已清」幂等）。
                let attempt_id = facts
                    .group_key
                    .map(str::to_string)
                    .unwrap_or_else(|| facts.target_id.to_string());
                ReviewerResourcePurger::new(paths)
                    .purge_for_attempts(std::slice::from_ref(&attempt_id))?;
                Some(build_ledger_row(&facts, None, run_id, now))
            }
            value if value == DanglingRecordType::EntityIdentitySessionBinding.as_str() => {
                delete_binding_entry(paths, &facts, run_id, now)?
            }
            _ => None,
        };
        let Some(row) = row else {
            continue;
        };
        // 闸 3 ②：逐行追加留痕（台账全量比对去重；已留痕过则不加行）。
        ledger.append(&row, now)?;
        deleted += 1;
    }
    Ok(deleted)
}

/// 删除一条自动化 intent 记录（enrollment 文件锁内完成；缺失 = 已清）。
fn delete_intent_file(
    paths: &ProductAppPaths,
    facts: &DeletionFacts<'_>,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<Option<LedgerRow>, ProductStoreError> {
    let Some(path) =
        resolve_record_file(paths, facts.record_path, intent_layout_ok, intent_name_ok)?
    else {
        return Ok(None);
    };
    let Some(enrollment_path) = path.parent().map(|parent| parent.join(ENROLLMENT_FILE)) else {
        return Ok(None);
    };
    // 判定已由 Task 7 在 enrollment 锁内完成；这里取内容快照，供锁内「内容未变」校验。
    let snapshot = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("read", &path, &error)),
    };
    // REQ-DRG-15：删除 MUST 在同一把 enrollment 文件锁内完成，MUST NOT 在锁外删除。
    let removed = with_exclusive_lock(&enrollment_path, || match std::fs::read(&path) {
        Ok(current) if current == snapshot => match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(io_error("remove", &path, &error)),
        },
        // 锁内重读发现内容已被并发改写：本轮不删（fail-closed，留给下次确认）。
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("read", &path, &error)),
    })?;
    if !removed {
        return Ok(None);
    }
    let (project_id, issue_id) = scope_from_record_path(facts.record_path);
    Ok(Some(build_ledger_row(
        facts,
        deleted_entity_ref(facts.target_kind, facts.target_id, project_id, issue_id),
        run_id,
        now,
    )))
}

/// 删除一条实体身份台账条目（缺失 = 已清）。
fn delete_binding_entry(
    paths: &ProductAppPaths,
    facts: &DeletionFacts<'_>,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<Option<LedgerRow>, ProductStoreError> {
    let Some(path) =
        resolve_record_file(paths, facts.record_path, binding_layout_ok, binding_name_ok)?
    else {
        return Ok(None);
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(Some(build_ledger_row(
            facts,
            deleted_entity_ref(facts.target_kind, facts.target_id, None, None),
            run_id,
            now,
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("remove", &path, &error)),
    }
}

/// 组一行留痕（REQ-DRG-16 三要素 + 证据等级；行形态对齐设计 §3.5.2）。
fn build_ledger_row(
    facts: &DeletionFacts<'_>,
    deleted_entity: Option<DeletedEntityRef>,
    run_id: &str,
    now: DateTime<Utc>,
) -> LedgerRow {
    LedgerRow {
        schema_version: LEDGER_SCHEMA_VERSION,
        cleanup_id: cleanup_id(facts.record_path, facts.target_id, facts.action),
        cleaned_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
        action: facts.action.to_string(),
        record_type: facts.record_type.to_string(),
        record_path: facts.record_path.to_string(),
        target_kind: facts.target_kind.to_string(),
        target_id: facts.target_id.to_string(),
        evidence_level: facts.evidence_level.to_string(),
        deleted_entity,
        trigger: CLEANUP_TRIGGER.to_string(),
        run_id: run_id.to_string(),
    }
}

/// 「因哪个实体被删」：只按**墓碑路径段**口径（story / design / plan）给出；
/// 其余目标类型没有墓碑种类 → `None`（不猜，REQ-DRG-16 子字段可省）。
fn deleted_entity_ref(
    target_kind: &str,
    target_id: &str,
    project_id: Option<String>,
    issue_id: Option<String>,
) -> Option<DeletedEntityRef> {
    let kind = match target_kind {
        value if value == DanglingTargetKind::StorySpec.as_str() => "story",
        value if value == DanglingTargetKind::DesignSpec.as_str() => "design",
        value if value == DanglingTargetKind::Plan.as_str() => "plan",
        _ => return None,
    };
    Some(DeletedEntityRef {
        kind: kind.to_string(),
        id: target_id.to_string(),
        project_id,
        issue_id,
    })
}

/// 从记录路径解出 project / issue 作用域（不符形状 → 都不给，不猜）。
fn scope_from_record_path(record_path: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = record_path.split('/').collect();
    if parts.len() == 5 && parts[0] == "projects" && parts[2] == "issues" {
        (Some(parts[1].to_string()), Some(parts[3].to_string()))
    } else {
        (None, None)
    }
}

/// intent 记录的路径形状：`projects/<p>/issues/<i>/<intent 文件名>`。
fn intent_layout_ok(components: &[&str]) -> bool {
    components.len() == 5 && components[0] == "projects" && components[2] == "issues"
}

/// intent 文件名白名单（只删这两个确定性文件名）。
fn intent_name_ok(name: &str) -> bool {
    name == PLAN_INTENT_FILE || name == GENERATION_INTENT_FILE
}

/// 台账条目的路径形状：`state/entity-identity/sessions/<session>.json`。
fn binding_layout_ok(components: &[&str]) -> bool {
    components.len() == 4
        && components[0] == "state"
        && components[1] == ENTITY_IDENTITY_DIR
        && components[2] == ENTITY_SESSIONS_DIR
}

/// 台账条目文件名：`<非空 stem>.json`，且不以 `.` 开头（非法名跳过）。
fn binding_name_ok(name: &str) -> bool {
    match name.strip_suffix(".json") {
        Some(stem) => !stem.is_empty() && !stem.starts_with('.'),
        None => false,
    }
}

/// 把待删记录路径解析成可删除的绝对路径；不满足护栏 → `None`（跳过，不删）。
///
/// 护栏沿用 Task 4 口径：① 只接受相对路径，`..` / `.` / 根 / 前缀一律视为非法名；
/// ② 结构必须是分区的**直接子项**（`layout_ok` 校验逐段形状）；③ 父目录 canonical
/// 后 MUST 仍在状态根内（父目录被符号链接搬到根外 → 不删）；④ 目标自身是符号链接
/// → 不跟随、不删。
fn resolve_record_file(
    paths: &ProductAppPaths,
    record_path: &str,
    layout_ok: fn(&[&str]) -> bool,
    name_ok: fn(&str) -> bool,
) -> Result<Option<PathBuf>, ProductStoreError> {
    let relative = Path::new(record_path);
    if relative.is_absolute() {
        return Ok(None);
    }
    let mut parts: Vec<&str> = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(value) => match value.to_str() {
                Some(value) => parts.push(value),
                None => return Ok(None),
            },
            _ => return Ok(None),
        }
    }
    let Some((name, parent_parts)) = parts.split_last() else {
        return Ok(None);
    };
    if !layout_ok(&parts) || !name_ok(name) {
        return Ok(None);
    }
    let parent = parent_parts
        .iter()
        .fold(PathBuf::new(), |path, part| path.join(part));
    let Ok(canonical_root) = std::fs::canonicalize(paths.root()) else {
        return Ok(None);
    };
    let parent = paths.root().join(parent);
    let canonical_parent = match std::fs::canonicalize(&parent) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("canonicalize", &parent, &error)),
    };
    if !canonical_parent.starts_with(&canonical_root) {
        // 父目录被符号链接搬到状态根外 → 不删。
        return Ok(None);
    }
    let target = canonical_parent.join(name);
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(None),
        Ok(_) => Ok(Some(target)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Some(target)),
        Err(error) => Err(io_error("stat", &target, &error)),
    }
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod cleanup_tests;
