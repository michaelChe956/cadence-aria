//! 按类型销账规则判断（REQ-DRG-12 / 19 / 20；技术方案 §3.4.1 / §3.4.1.1 / §3.4.1.2）。
//!
//! 本模块**只判定、不删除**：给出「这条记录能不能删」及其证据等级，删除动作与
//! 三道闸编排属销账编排（后续任务）。判定口径按记录类型分三条：
//!
//! 1. **自动化 intent**（[`decide_intent_cleanup`]）：必须先满足 REQ-DRG-20 的
//!    四道前置条件——I-1 全程在 `automation-enrollment.json` 的文件锁内、I-2 锁内
//!    重读（enrollment 状态 / generation 相位 / 墓碑证据，MUST NOT 用锁外旧值）、
//!    I-3 只认 L1 墓碑（session 与 enrollment 的缺失一律不算证据）、I-4 generation
//!    intent 的相位必须仍是 `Claimed`；四者缺一 → 只报不删。
//! 2. **reviewer 窗口簇**（[`decide_window_cluster_cleanup`]）：L2 四条件
//!    W-1..W-4 逐窗判定，但**删除粒度按 attempt 聚合**——该 attempt 名下**全部**
//!    窗口（含 `window-archive` 归档）都满足才产出一条簇级 `Delete`；任一不满足
//!    → 整个 attempt 只报不删，MUST NOT 产出部分可删的决策。
//! 3. **实体身份台账条目**（[`decide_ledger_entry_cleanup`]）：只认 plan 类型，
//!    且必须以 L1 墓碑（`{issue}/entity-deletions/plan/<plan_id>.json` 四条成立）
//!    为依据；无墓碑 → 只报不删。
//!
//! 「只报不删」的判定口径统一为 REQ-DRG-19：**不具备 L1 证据、且不满足该类型在
//! 证据等级表中的 L2 专属可算条件**的旁支记录一律不删（窗口簇走 L2，故「无 L1」
//! 本身不构成不删窗口的理由）。所有只报不删的 `reason` MUST 指明未满足哪一条。

use std::collections::BTreeMap;
use std::path::Path;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::dangling_reference::listing::EntityIdentityBindingEntry;
use crate::product::dangling_reference::reference_table::{DanglingRecordType, DanglingTargetKind};
use crate::product::dangling_reference::resolve::{
    DanglingResolver, EvidenceLevel, TargetExistence, window_l2_verdict,
};
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
use crate::product::issue_automation_store::{
    PlanGenerationIntentRead, PreparedPlanIntentRead, read_plan_generation_intent,
    read_prepared_plan_intent,
};
use crate::product::json_store::{ProductStoreError, read_json};
use crate::product::models::automation::{
    EnrollmentSource, IssueAutomationEnrollment, PlanGenerationPhase,
};
use crate::product::reviewer_window_store::{REVIEWER_WINDOW_STORE_DIR, ReviewerWindowRecord};

/// 自动化 enrollment 文件名（既有落盘约定；同时是 I-1 的锁目标）。
const ENROLLMENT_FILE: &str = "automation-enrollment.json";
/// 计划意图文件名（既有落盘约定，MUST NOT 枚举目录其它改名）。
const PLAN_INTENT_FILE: &str = "automation-plan-intent.json";
/// 生成检查点文件名（既有落盘约定）。
const GENERATION_INTENT_FILE: &str = "automation-generation-intent.json";
/// reviewer 窗口活跃记录目录（与 `patrol` 同一落盘约定）。
const WINDOWS_DIR: &str = "windows";
/// reviewer 窗口终态归档目录（归档窗口同属窗口簇，删账按 attempt 聚合）。
const WINDOW_ARCHIVE_DIR: &str = "window-archive";
/// 证据等级取值：L1（墓碑四条）。
const EVIDENCE_L1: &str = "L1";
/// 证据等级取值：L2（窗口簇四条件）。
const EVIDENCE_L2: &str = "L2";
/// 动作取值：删单条记录。
const ACTION_DELETE_RECORD: &str = "delete_record";
/// 动作取值：按簇批量删除（窗口簇 / 快照目录 / 台账条目簇）。
const ACTION_DELETE_RECORD_GROUP: &str = "delete_record_group";
/// 台账条目白名单唯一实体种类（其余只报不删）。
const LEDGER_ENTITY_KIND_PLAN: &str = "plan";

/// 一条销账判定（REQ-DRG-12）：可删 或 只报不删。
///
/// `Delete` 只表达「证据齐备、允许交给物理清理器」，本模块**不执行**删除。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CleanupDecision {
    /// 可删：`group_key` 非空表示按簇批量删除（窗口簇按 attempt id 聚合）。
    Delete {
        /// 被清理记录的路径（相对应用状态根）。
        record_path: String,
        /// 被清理记录的类型（判据表稳定标识）。
        record_type: String,
        /// 引用目标类型（判据表稳定标识）。
        target_kind: String,
        /// 引用目标 id。
        target_id: String,
        /// 本次删除所依据的证据等级（`L1` / `L2`）。
        evidence_level: String,
        /// 动作（`delete_record` / `delete_record_group`）。
        action: String,
        /// 簇键（窗口簇为 attempt id；单记录删除为 `None`）。
        group_key: Option<String>,
    },
    /// 只报不删：`reason` MUST 指明未满足哪一条前置条件 / 缺哪一种证据。
    ReportOnly {
        /// 被判定记录的路径（相对应用状态根）。
        record_path: String,
        /// 不删的原因。
        reason: String,
    },
}

/// 判定一条自动化 intent 记录能否销账（REQ-DRG-12 / 19 / 20）。
///
/// 全部判定在 `{issue_root}/automation-enrollment.json` 的**文件锁内**完成（I-1）：
/// 锁内重读意图自身的 enrollment 身份与引用集合、enrollment 记录状态、generation
/// intent 相位与目标墓碑证据（I-2 / I-3 / I-4），MUST NOT 使用锁外读到的旧值。
///
/// `intent_file_name` 只接受两个白名单文件名；表外名字 / 旧代无绑定意图一律
/// 只报不删（MUST NOT 猜引用语义）。
pub(crate) fn decide_intent_cleanup(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    intent_file_name: &str,
    resolver: &DanglingResolver<'_>,
) -> Result<CleanupDecision, ProductStoreError> {
    let issue_root = paths.issue_root(project_id, issue_id);
    let intent_path = issue_root.join(intent_file_name);
    let record_path = record_path_of(paths, &intent_path);
    let record_type = match intent_file_name {
        PLAN_INTENT_FILE => DanglingRecordType::AutomationPlanIntent,
        GENERATION_INTENT_FILE => DanglingRecordType::AutomationGenerationIntent,
        _ => {
            return Ok(CleanupDecision::ReportOnly {
                record_path,
                reason: format!("记录不在销账白名单内：{intent_file_name}"),
            });
        }
    };
    if !path_exists(&intent_path)? {
        // 记录已不存在：没有可删对象（重入语义由编排侧按「缺失视为已清」处理）。
        return Ok(CleanupDecision::ReportOnly {
            record_path,
            reason: format!("意图记录不存在：{intent_file_name}"),
        });
    }
    // I-1：与既有取代路径同一把锁（`automation-enrollment.json.lock`）。
    let enrollment_path = issue_root.join(ENROLLMENT_FILE);
    with_exclusive_lock(&enrollment_path, || {
        decide_intent_cleanup_locked(
            project_id,
            issue_id,
            &issue_root,
            record_type,
            &record_path,
            resolver,
        )
    })
}

/// 锁内的判定主体（I-2：三项 MUST 在同一把锁内重读）。
fn decide_intent_cleanup_locked(
    project_id: &str,
    issue_id: &str,
    issue_root: &Path,
    record_type: DanglingRecordType,
    record_path: &str,
    resolver: &DanglingResolver<'_>,
) -> Result<CleanupDecision, ProductStoreError> {
    let report_only = |reason: String| CleanupDecision::ReportOnly {
        record_path: record_path.to_string(),
        reason,
    };
    // I-2：锁内重读意图本身（身份 + 引用集合 + generation 相位）。
    let facts = match read_intent_in_lock(issue_root, record_type)? {
        IntentInLock::Facts(facts) => facts,
        IntentInLock::Unsupported(reason) => return Ok(report_only(reason)),
    };
    // I-2 ①：enrollment 状态（锁内重读）。
    if let Some(reason) = enrollment_block_reason(issue_root, &facts.enrollment_id)? {
        return Ok(report_only(reason));
    }
    // I-4：generation intent 相位（锁内重读）；plan intent 无相位约束。
    if let Some(phase) = facts.phase
        && phase != PlanGenerationPhase::Claimed
    {
        return Ok(report_only(format!(
            "相位已离开 Claimed（锁内重读为 {phase:?}）：保留防双跑检查点，只报不删"
        )));
    }
    // I-3：只认 L1 墓碑证据（锁内重读）；session / enrollment 缺失不算证据。
    if facts.references.is_empty() {
        return Ok(report_only(
            "意图未引用 story / design / plan：无 L1 墓碑证据可依，只报不删".to_string(),
        ));
    }
    for (kind, target_id) in &facts.references {
        let scope = tombstone_scope(project_id, issue_id, *kind, target_id);
        match resolver.target_exists(*kind, target_id, Some(&scope))? {
            TargetExistence::Exists => {
                return Ok(report_only(format!(
                    "引用目标仍存在（{} {}）：不构成悬空，只报不删",
                    kind.as_str(),
                    target_id
                )));
            }
            TargetExistence::Unknown => {
                return Ok(report_only(format!(
                    "引用目标存在性不可判定（{} {}）：只报不删",
                    kind.as_str(),
                    target_id
                )));
            }
            TargetExistence::Missing => {}
        }
        if resolver.evidence_level(*kind, target_id, Some(&scope))? != EvidenceLevel::L1 {
            return Ok(report_only(format!(
                "引用目标缺 L1 墓碑证据（{} {}）：只报不删",
                kind.as_str(),
                target_id
            )));
        }
    }
    let (target_kind, target_id) = facts
        .references
        .first()
        .cloned()
        .expect("references 非空已在上方保证");
    Ok(CleanupDecision::Delete {
        record_path: record_path.to_string(),
        record_type: record_type.as_str().to_string(),
        target_kind: target_kind.as_str().to_string(),
        target_id,
        evidence_level: EVIDENCE_L1.to_string(),
        action: ACTION_DELETE_RECORD.to_string(),
        group_key: None,
    })
}

/// 锁内重读的意图事实。
struct IntentFacts {
    /// 意图自带的 enrollment 身份 id（用于 I-2 的身份一致比对）。
    enrollment_id: String,
    /// 被判定的引用集合（只含 story / design / plan；session 与 enrollment 排除）。
    references: Vec<(DanglingTargetKind, String)>,
    /// generation intent 的相位（plan intent 为 `None`；I-4 只约束前者）。
    phase: Option<PlanGenerationPhase>,
}

/// 锁内重读结果。
enum IntentInLock {
    /// 读到完整事实，可继续判定。
    Facts(IntentFacts),
    /// 结构上不足以判定（旧代无绑定 / 表外记录）：只报不删，附原因。
    Unsupported(String),
}

/// 锁内重读意图记录（既有版本化读侧；解析失败原样上抛）。
fn read_intent_in_lock(
    issue_root: &Path,
    record_type: DanglingRecordType,
) -> Result<IntentInLock, ProductStoreError> {
    match record_type {
        DanglingRecordType::AutomationPlanIntent => {
            let path = issue_root.join(PLAN_INTENT_FILE);
            Ok(match read_prepared_plan_intent(&path)? {
                PreparedPlanIntentRead::Current(intent) => IntentInLock::Facts(IntentFacts {
                    references: intent_references(&intent.source, Some(&intent.plan_id)),
                    enrollment_id: intent.enrollment_id,
                    phase: None,
                }),
                PreparedPlanIntentRead::LegacyUnbound { .. } => IntentInLock::Unsupported(
                    "旧代无绑定意图（无 target）：无 story / design / plan 引用可证，只报不删"
                        .to_string(),
                ),
            })
        }
        DanglingRecordType::AutomationGenerationIntent => {
            let path = issue_root.join(GENERATION_INTENT_FILE);
            Ok(match read_plan_generation_intent(&path)? {
                PlanGenerationIntentRead::Current(intent) => IntentInLock::Facts(IntentFacts {
                    references: intent_references(&intent.source, Some(&intent.plan_id)),
                    enrollment_id: intent.enrollment_id,
                    phase: Some(intent.phase),
                }),
                PlanGenerationIntentRead::LegacyUnbound { .. } => IntentInLock::Unsupported(
                    "旧代无绑定检查点（无 target）：既无相位可证、也无引用可核，只报不删"
                        .to_string(),
                ),
            })
        }
        other => Ok(IntentInLock::Unsupported(format!(
            "记录类型不在销账白名单内：{}，只报不删",
            other.as_str()
        ))),
    }
}

/// 意图的待核对引用集合：story / design / plan 三类，空 id 不算引用。
///
/// `session_id` 与 `enrollment_id` **刻意不入列**：二者的缺失 MUST NOT 构成删除
/// 依据（REQ-DRG-20 ③ / 设计 S6）。
fn intent_references(
    source: &EnrollmentSource,
    plan_id: Option<&String>,
) -> Vec<(DanglingTargetKind, String)> {
    let mut references = Vec::new();
    for story in &source.stories {
        if !story.id.is_empty() {
            references.push((DanglingTargetKind::StorySpec, story.id.clone()));
        }
    }
    for design in &source.designs {
        if !design.id.is_empty() {
            references.push((DanglingTargetKind::DesignSpec, design.id.clone()));
        }
    }
    if let Some(plan_id) = plan_id.filter(|id| !id.is_empty()) {
        references.push((DanglingTargetKind::Plan, plan_id.clone()));
    }
    references
}

/// I-2 ①：enrollment 状态（锁内重读）。`Some(reason)` = 不通过。
///
/// - 文件缺失 → 通过（该 issue 已无活授权）；
/// - `enabled == true` → 不通过（intent 是活链路的补偿锚点）；
/// - `enabled == false` 且无 `terminated_at` → 不通过（仅禁用未终止＝仍有活授权，
///   保守口径见设计 §3.4.1.2 N-12）；
/// - 有 `terminated_at` 但 `enrollment_id` 与意图不一致 → 不通过；
/// - 其余（已终止且身份一致）→ 通过。
fn enrollment_block_reason(
    issue_root: &Path,
    intent_enrollment_id: &str,
) -> Result<Option<String>, ProductStoreError> {
    let path = issue_root.join(ENROLLMENT_FILE);
    if !path_exists(&path)? {
        return Ok(None);
    }
    let enrollment: IssueAutomationEnrollment = read_json(&path)?;
    if enrollment.enabled {
        return Ok(Some(
            "enrollment 仍启用（enabled == true）：意图是活链路的补偿锚点，只报不删".to_string(),
        ));
    }
    if enrollment.terminated_at.is_none() {
        return Ok(Some(
            "enrollment 仅禁用未终止（terminated_at 为空）：仍有活授权，只报不删".to_string(),
        ));
    }
    if enrollment.enrollment_id != intent_enrollment_id {
        return Ok(Some(format!(
            "enrollment 身份与意图不一致（锁内重读为 {}，意图为 {intent_enrollment_id}）：只报不删",
            enrollment.enrollment_id
        )));
    }
    Ok(None)
}

/// 判定 reviewer 窗口簇能否销账（REQ-DRG-12；按 attempt 聚合）。
///
/// 遍历 `<root>/reviewer-windows/windows/*.json` 与
/// `<root>/reviewer-windows/window-archive/*.json`（归档窗口同属窗口簇），按
/// `attempt_id` 聚合后逐窗算 W-1..W-4：该 attempt 名下**全部**窗口都成立才产出
/// 一条 `Delete`（`group_key = attempt_id`）；任一不成立 → 该 attempt 只有一条
/// `ReportOnly`，MUST NOT 产出部分可删决策。
///
/// attempt 的存在性判定（W-3 的两路核对）与记录解析失败一律不删：
/// 前者 `Exists` 视为「目标仍在」不产决策、`Unknown` 只报不删；后者使**全部簇**
/// 不可删（无法排除坏记录正属某个看似合规的簇）。
pub(crate) fn decide_window_cluster_cleanup(
    paths: &ProductAppPaths,
    resolver: &DanglingResolver<'_>,
) -> Result<Vec<CleanupDecision>, ProductStoreError> {
    let mut decisions = Vec::new();
    let mut grouped: BTreeMap<String, Vec<(String, ReviewerWindowRecord)>> = BTreeMap::new();
    let mut unreadable = false;
    for candidate in window_records(paths)? {
        match candidate.parsed {
            Ok(record) => grouped
                .entry(record.attempt_id.clone())
                .or_default()
                .push((candidate.record_path, record)),
            Err(reason) => {
                unreadable = true;
                decisions.push(CleanupDecision::ReportOnly {
                    record_path: candidate.record_path,
                    reason,
                });
            }
        }
    }
    for (attempt_id, mut windows) in grouped {
        windows.sort_by(|left, right| left.0.cmp(&right.0));
        let record_path = windows
            .first()
            .map(|(record_path, _)| record_path.clone())
            .expect("按 attempt 聚合的组必非空");
        // W-3 的前置：attempt 两路核对（任一路命中即「仍在」，读失败上抛）。
        match resolver.target_exists(DanglingTargetKind::Attempt, &attempt_id, None)? {
            TargetExistence::Exists => continue,
            TargetExistence::Unknown => {
                decisions.push(CleanupDecision::ReportOnly {
                    record_path,
                    reason: format!("attempt 存在性不可判定（{attempt_id}）：只报不删"),
                });
                continue;
            }
            TargetExistence::Missing => {}
        }
        let mut blocker = None;
        for (window_path, window) in &windows {
            let verdict = window_l2_verdict(paths, window, true)?;
            if !verdict.all_hold() && blocker.is_none() {
                blocker = Some(format!(
                    "同一 attempt（{attempt_id}）下窗口 {window_path} 的四条件不成立（{}），整 attempt 不删",
                    verdict.summary()
                ));
            }
        }
        if blocker.is_none() && unreadable {
            blocker = Some(format!(
                "存在不可解析的窗口记录：无法排除其属于 attempt（{attempt_id}），整 attempt 不删"
            ));
        }
        match blocker {
            Some(reason) => decisions.push(CleanupDecision::ReportOnly {
                record_path,
                reason,
            }),
            None => decisions.push(CleanupDecision::Delete {
                record_path,
                record_type: DanglingRecordType::ReviewerWindow.as_str().to_string(),
                target_kind: DanglingTargetKind::Attempt.as_str().to_string(),
                target_id: attempt_id.clone(),
                evidence_level: EVIDENCE_L2.to_string(),
                action: ACTION_DELETE_RECORD_GROUP.to_string(),
                group_key: Some(attempt_id),
            }),
        }
    }
    Ok(decisions)
}

/// 枚举到的窗口记录：路径 + 读取结果（解析失败保留原因，供只报不删决策使用）。
struct WindowRecordCandidate {
    /// 记录路径（相对应用状态根）。
    record_path: String,
    /// 解析结果（`Err` = 不可解析，原因即 `ReportOnly` 的 reason）。
    parsed: Result<ReviewerWindowRecord, String>,
}

/// 枚举窗口记录（活跃 + 归档两路），逐条解析结果（解析失败保留原因）。
fn window_records(
    paths: &ProductAppPaths,
) -> Result<Vec<WindowRecordCandidate>, ProductStoreError> {
    let mut items = Vec::new();
    for dir_name in [WINDOWS_DIR, WINDOW_ARCHIVE_DIR] {
        let dir = paths.root().join(REVIEWER_WINDOW_STORE_DIR).join(dir_name);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    dir.display()
                )));
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", dir.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            items.push(WindowRecordCandidate {
                record_path: record_path_of(paths, &path),
                parsed: read_json::<ReviewerWindowRecord>(&path)
                    .map_err(|error| format!("窗口记录解析失败：{error}")),
            });
        }
    }
    items.sort_by(|left, right| left.record_path.cmp(&right.record_path));
    Ok(items)
}

/// 判定一条实体身份台账条目能否销账（REQ-DRG-12：仅 L1 墓碑）。
///
/// 白名单只认 `plan` 类型条目；且必须以 L1 墓碑（墓碑存在、`phase == Deleted`、
/// `deleted_at` 有值、无待人工处置的删除失败事实四条同时成立）为依据。无墓碑 /
/// 目标仍存在 / 作用域不可用 → 只报不删（REQ-DRG-19）。
pub(crate) fn decide_ledger_entry_cleanup(
    _paths: &ProductAppPaths,
    entry: &EntityIdentityBindingEntry,
    resolver: &DanglingResolver<'_>,
) -> Result<CleanupDecision, ProductStoreError> {
    let report_only = |reason: String| CleanupDecision::ReportOnly {
        record_path: entry.record_path.clone(),
        reason,
    };
    if entry.entity_kind != LEDGER_ENTITY_KIND_PLAN {
        return Ok(report_only(format!(
            "台账条目引用的实体种类为 {}：销账白名单只认 plan，只报不删",
            entry.entity_kind
        )));
    }
    if entry.project_id.is_empty() || entry.issue_id.is_empty() {
        return Ok(report_only(
            "台账条目缺 project / issue 作用域：存在性不可判定，只报不删".to_string(),
        ));
    }
    let scope = EntityScope::new(
        entry.project_id.clone(),
        entry.issue_id.clone(),
        DeletionEntityKind::Plan,
        entry.entity_id.clone(),
    );
    match resolver.target_exists(DanglingTargetKind::Plan, &entry.entity_id, Some(&scope))? {
        TargetExistence::Exists => {
            return Ok(report_only(format!(
                "引用目标仍存在（plan {}）：不构成悬空，只报不删",
                entry.entity_id
            )));
        }
        TargetExistence::Unknown => {
            return Ok(report_only(format!(
                "引用目标存在性不可判定（plan {}）：只报不删",
                entry.entity_id
            )));
        }
        TargetExistence::Missing => {}
    }
    if resolver.evidence_level(DanglingTargetKind::Plan, &entry.entity_id, Some(&scope))?
        != EvidenceLevel::L1
    {
        return Ok(report_only(format!(
            "引用 plan 缺 L1 墓碑证据（{}）：只报不删",
            entry.entity_id
        )));
    }
    Ok(CleanupDecision::Delete {
        record_path: entry.record_path.clone(),
        record_type: DanglingRecordType::EntityIdentitySessionBinding
            .as_str()
            .to_string(),
        target_kind: DanglingTargetKind::Plan.as_str().to_string(),
        target_id: entry.entity_id.clone(),
        evidence_level: EVIDENCE_L1.to_string(),
        action: ACTION_DELETE_RECORD.to_string(),
        group_key: None,
    })
}

/// 目标类型 → 墓碑 scope（project/issue 取调用方作用域，kind 按目标类型映射）。
fn tombstone_scope(
    project_id: &str,
    issue_id: &str,
    kind: DanglingTargetKind,
    target_id: &str,
) -> EntityScope {
    let deletion_kind = match kind {
        DanglingTargetKind::StorySpec => DeletionEntityKind::Story,
        DanglingTargetKind::DesignSpec => DeletionEntityKind::Design,
        // 其余类型不进 L1 档；调用方只在 story/design/plan 上询问证据。
        _ => DeletionEntityKind::Plan,
    };
    EntityScope::new(
        project_id.to_string(),
        issue_id.to_string(),
        deletion_kind,
        target_id.to_string(),
    )
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
        .map_err(|error| ProductStoreError::Io(format!("stat {}: {error}", path.display())))
}

#[cfg(test)]
#[path = "cleanup_rules_tests.rs"]
mod cleanup_rules_tests;
