//! ① 层四个容错列举只读 API（REQ-DRG-03 / 04 / 05 / 06 / 23）。
//!
//! 本模块是第 ① 层的**容错列举面**：四个只读 API（自动化 intent 清单、
//! reviewer 窗口清单、reviewer 快照清单、实体身份台账清单）在遇到「指向
//! 已删对象」的条目时**跳过该条目并产生一条告警**，MUST NOT 使整次列举
//! 失败（REQ-DRG-03）。全部为只读 IO（不写回、不取 lease）。
//!
//! 三条边界：
//! 1. **只吞「目标已删」**（REQ-DRG-05）：本模块自身产生的读错（JSON 解析失败 /
//!    schema 不符 / IO 失败 / 权限错误）一律以 `Err` 原样上抛，MUST NOT 被降级成
//!    「跳过该条目」；唯一既有豁免是 reviewer 窗口面——`ReviewerWindowStore::list_windows`
//!    （`src/product/reviewer_window_store.rs:1160`）对单条坏记录按既有语义
//!    「warn + 跳过」，其目录级读取失败仍照旧上抛（本模块不改既有函数）；
//! 2. **主记录缺失语义不变**（REQ-DRG-04）：本模块只新增旁支 / 记账类
//!    记录的列举面，MUST NOT 改动任何既有主记录读路径（`story_spec_not_found`
//!    等错误码与文案逐字不变），既有 fail-closed 读路径不接入容错
//!    （REQ-DRG-06）；
//! 3. **不可判定不计入悬空**（REQ-DRG-23）：存在性判为 `Unknown`（数据源
//!    不可用 / 作用域不足）的引用既不跳过条目、也不产生告警。

use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::dangling_reference::reference_table::{
    DanglingRecordType, DanglingReferenceRule, DanglingTargetKind, reference_rule,
};
use crate::product::dangling_reference::resolve::{DanglingResolver, TargetExistence};
use crate::product::dangling_reference::{
    DANGLING_REFERENCE_WARNING_CODE, DanglingReferenceWarning, DanglingReferenceWarnings,
};
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
use crate::product::issue_automation_store::{
    PlanGenerationIntentRead, PreparedPlanIntentRead, read_plan_generation_intent,
    read_prepared_plan_intent,
};
use crate::product::json_store::{ProductStoreError, read_json};
use crate::product::reviewer_resource_purge::REVIEWER_SNAPSHOT_STORE_DIR;
use crate::product::reviewer_snapshot::ReviewerBaselineManifest;
use crate::product::reviewer_window_store::{REVIEWER_WINDOW_STORE_DIR, ReviewerWindowStore};

/// 计划意图文件名（既有落盘约定，MUST NOT 枚举目录其它文件）。
const PLAN_INTENT_FILE: &str = "automation-plan-intent.json";
/// 生成检查点文件名（既有落盘约定，MUST NOT 枚举目录其它文件）。
const GENERATION_INTENT_FILE: &str = "automation-generation-intent.json";
/// 被跳过的条目的处置动作（告警 `action` 列取值）。
const ACTION_SKIPPED: &str = "skipped";

/// 单次容错列举的结果（REQ-DRG-03）：正常条目 + 悬空告警。
///
/// `items` 只含**未被跳过**的条目；被跳过的条目各产生**一条**告警
/// （MUST NOT 按字段逐条刷告警）。
#[derive(Debug, Clone)]
pub(crate) struct ListingOutcome<T> {
    pub(crate) items: Vec<T>,
    pub(crate) warnings: DanglingReferenceWarnings,
}

/// 自动化 intent 清单条目（一个 intent 文件一条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutomationIntentEntry {
    /// 落盘文件名（`automation-plan-intent.json` / `automation-generation-intent.json`）。
    pub(crate) file_name: &'static str,
    /// 记录路径（相对应用状态根）。
    pub(crate) record_path: String,
    /// enrollment 身份 id。
    pub(crate) enrollment_id: String,
    /// 绑定的 plan id（旧代无绑定意图为 `None`）。
    pub(crate) plan_id: Option<String>,
    /// 绑定的 workspace session id（旧代无绑定意图为 `None`）。
    pub(crate) session_id: Option<String>,
    /// 冻结的 story spec id 集合。
    pub(crate) story_ids: Vec<String>,
    /// 冻结的 design spec id 集合。
    pub(crate) design_ids: Vec<String>,
}

/// reviewer 窗口清单条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewerWindowEntry {
    /// 记录路径（相对应用状态根）。
    pub(crate) record_path: String,
    /// 窗口 id。
    pub(crate) window_id: String,
    /// 窗口归属的 coding attempt id（两路核对的目标）。
    pub(crate) attempt_id: String,
}

/// reviewer 快照清单条目（快照 manifest）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewerSnapshotEntry {
    /// 记录路径（相对应用状态根）。
    pub(crate) record_path: String,
    /// 窗口 id。
    pub(crate) window_id: String,
    /// 快照归属的 coding attempt id（两路核对的目标）。
    pub(crate) attempt_id: String,
}

/// 实体身份台账清单条目（session → 实体绑定）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntityIdentityBindingEntry {
    /// 记录路径（相对应用状态根）。
    pub(crate) record_path: String,
    /// 台账所属 workspace session id（取自落盘文件名）。
    pub(crate) session_id: String,
    /// 绑定实体的种类（`story` / `design` / `plan`；表外值原样保留）。
    pub(crate) entity_kind: String,
    /// 绑定实体 id。
    pub(crate) entity_id: String,
    /// 绑定实体所属 project。
    pub(crate) project_id: String,
    /// 绑定实体所属 issue。
    pub(crate) issue_id: String,
}

/// 一条待核对引用：字段路径 + 目标类型 + 目标 id + 作用域。
struct CandidateReference {
    /// 命中的引用字段路径（取自判据表登记值）。
    field_path: &'static str,
    /// 引用目标类型。
    target_kind: DanglingTargetKind,
    /// 引用目标 id。
    target_id: String,
    /// 目标所属 project（无作用域需求的目标留空）。
    project_id: String,
    /// 目标所属 issue（无作用域需求的目标留空）。
    issue_id: String,
}

impl CandidateReference {
    /// 需要 project/issue 定位的目标（story / design / plan）的核对作用域。
    ///
    /// 作用域不可用（project / issue 缺失）时返回 `None`——调用方据此归
    /// 「不可判定」，MUST NOT 猜默认作用域（REQ-DRG-23）。
    fn scope(&self) -> Option<EntityScope> {
        let kind = match self.target_kind {
            DanglingTargetKind::StorySpec => DeletionEntityKind::Story,
            DanglingTargetKind::DesignSpec => DeletionEntityKind::Design,
            DanglingTargetKind::Plan => DeletionEntityKind::Plan,
            _ => return None,
        };
        Some(EntityScope::new(
            self.project_id.clone(),
            self.issue_id.clone(),
            kind,
            self.target_id.clone(),
        ))
    }
}

/// 单条引用的核对结论（存在性三态在列举面的映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReferenceVerdict {
    /// 目标仍在：条目保留。
    Alive,
    /// 不可判定：条目保留且**不产生告警**，MUST NOT 计入悬空条目数
    /// （REQ-DRG-23）。
    Undetermined,
    /// 目标已删：条目跳过 + 一条告警（REQ-DRG-03）。
    Missing,
}

/// 自动化 intent 记录上供判据表核对的事实。
struct IntentFacts {
    /// 记录路径（相对应用状态根）。
    record_path: String,
    /// enrollment 身份 id。
    enrollment_id: String,
    /// 绑定的 plan id（旧代无绑定意图为 `None`）。
    plan_id: Option<String>,
    /// 绑定的 workspace session id（旧代无绑定意图为 `None`）。
    session_id: Option<String>,
    /// 冻结的 story spec id 集合。
    story_ids: Vec<String>,
    /// 冻结的 design spec id 集合。
    design_ids: Vec<String>,
    /// 记录所属 project。
    project_id: String,
    /// 记录所属 issue。
    issue_id: String,
}

impl IntentFacts {
    /// 按判据表登记的字段路径与目标类型登记一条引用。
    fn reference(
        &self,
        field_path: &'static str,
        target_kind: DanglingTargetKind,
        target_id: String,
    ) -> CandidateReference {
        CandidateReference {
            field_path,
            target_kind,
            target_id,
            project_id: self.project_id.clone(),
            issue_id: self.issue_id.clone(),
        }
    }
}

/// 空结果（无条目、无告警）。
fn empty_outcome<T>() -> ListingOutcome<T> {
    ListingOutcome {
        items: Vec::new(),
        warnings: DanglingReferenceWarnings::default(),
    }
}

/// 把存在性三态映射为列举面的核对结论（REQ-DRG-01 / 23）。
///
/// - `Exists` → 条目保留；
/// - `Unknown` → **不可判定**：条目保留、不产生告警、不计入悬空条目数；
/// - `Missing` → 条目跳过 + 一条告警。
fn classify_reference(
    resolver: &DanglingResolver<'_>,
    reference: &CandidateReference,
) -> Result<ReferenceVerdict, ProductStoreError> {
    // 需要 project/issue 定位的目标：作用域不可用即「不可判定」，不得猜。
    if requires_scope(reference.target_kind)
        && (reference.project_id.is_empty() || reference.issue_id.is_empty())
    {
        return Ok(ReferenceVerdict::Undetermined);
    }
    let scope = reference.scope();
    Ok(
        match resolver.target_exists(
            reference.target_kind,
            reference.target_id.as_str(),
            scope.as_ref(),
        )? {
            TargetExistence::Exists => ReferenceVerdict::Alive,
            TargetExistence::Unknown => ReferenceVerdict::Undetermined,
            TargetExistence::Missing => ReferenceVerdict::Missing,
        },
    )
}

/// 按判据表登记顺序核对一条记录的全部引用，返回第一条判定为「目标已删」的引用。
///
/// 「不可判定」的引用**不**构成悬空（REQ-DRG-23）：既不跳过条目，也不产生
/// 告警。读失败由 [`classify_reference`] 原样上抛（REQ-DRG-05）。
fn first_dangling_reference<'a>(
    resolver: &DanglingResolver<'_>,
    references: &'a [CandidateReference],
) -> Result<Option<&'a CandidateReference>, ProductStoreError> {
    for reference in references {
        if classify_reference(resolver, reference)? == ReferenceVerdict::Missing {
            return Ok(Some(reference));
        }
    }
    Ok(None)
}

/// 记录一条「跳过该条目」告警（REQ-DRG-03：每条被跳过的记录只产生一条告警）。
fn push_skipped_warning(
    warnings: &mut DanglingReferenceWarnings,
    record_path: &str,
    record_type: DanglingRecordType,
    reference: &CandidateReference,
) {
    warnings.push(DanglingReferenceWarning {
        code: DANGLING_REFERENCE_WARNING_CODE.to_string(),
        record_path: record_path.to_string(),
        record_type: record_type.as_str().to_string(),
        reference_field: reference.field_path.to_string(),
        target_kind: reference.target_kind.as_str().to_string(),
        target_id: reference.target_id.clone(),
        action: ACTION_SKIPPED.to_string(),
    });
}

/// 目标类型是否需要 project/issue 作用域。
fn requires_scope(target_kind: DanglingTargetKind) -> bool {
    matches!(
        target_kind,
        DanglingTargetKind::StorySpec | DanglingTargetKind::DesignSpec | DanglingTargetKind::Plan
    )
}

/// 存在性判定（stat）：IO 失败上抛为 `ProductStoreError::Io`，
/// MUST NOT 被当成「不存在」。
fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("stat {}: {error}", path.display())))
}

/// 记录路径（相对应用状态根 `paths.root()`）。
pub(crate) fn record_path_of(paths: &ProductAppPaths, path: &Path) -> String {
    path.strip_prefix(paths.root())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
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

/// 判据表中某记录类型的某个目标类型对应的字段路径。
pub(crate) fn field_path_for(
    rule: &DanglingReferenceRule,
    target_kind: DanglingTargetKind,
) -> &'static str {
    rule.fields
        .iter()
        .find(|field| field.target_kind == target_kind)
        .map(|field| field.path)
        .expect("判据表必须登记该引用字段")
}

/// 自动化 intent 清单（容错列举；REQ-DRG-03）。
///
/// 只读两个确定文件名（`automation-plan-intent.json` /
/// `automation-generation-intent.json`），MUST NOT 枚举目录其它文件
/// （enrollment 记录不在本清单内）。指向已删对象的 intent 被**跳过**并
/// 产生一条告警；读失败原样上抛（REQ-DRG-05）。
pub(crate) fn list_automation_intents(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    resolver: &DanglingResolver<'_>,
) -> Result<ListingOutcome<AutomationIntentEntry>, ProductStoreError> {
    let issue_root = paths.issue_root(project_id, issue_id);
    let mut outcome = empty_outcome();

    let plan_path = issue_root.join(PLAN_INTENT_FILE);
    if path_exists(&plan_path)? {
        let record_path = record_path_of(paths, &plan_path);
        let facts = read_plan_intent_facts(&plan_path, &record_path)?;
        push_intent_entry(
            &mut outcome,
            resolver,
            DanglingRecordType::AutomationPlanIntent,
            PLAN_INTENT_FILE,
            &facts,
        )?;
    }

    let generation_path = issue_root.join(GENERATION_INTENT_FILE);
    if path_exists(&generation_path)? {
        let record_path = record_path_of(paths, &generation_path);
        let facts =
            read_generation_intent_facts(&generation_path, &record_path, project_id, issue_id)?;
        push_intent_entry(
            &mut outcome,
            resolver,
            DanglingRecordType::AutomationGenerationIntent,
            GENERATION_INTENT_FILE,
            &facts,
        )?;
    }

    Ok(outcome)
}

/// 读计划意图（既有版本化读侧；旧代 `LegacyUnbound` 语义不变）。
fn read_plan_intent_facts(
    path: &Path,
    record_path: &str,
) -> Result<IntentFacts, ProductStoreError> {
    Ok(match read_prepared_plan_intent(path)? {
        PreparedPlanIntentRead::Current(intent) => IntentFacts {
            record_path: record_path.to_string(),
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
            project_id: intent.project_id.clone(),
            issue_id: intent.issue_id.clone(),
        },
        // 旧代无绑定意图：只有身份字段，无 plan/session/source 引用。
        PreparedPlanIntentRead::LegacyUnbound { enrollment_id, .. } => IntentFacts {
            record_path: record_path.to_string(),
            enrollment_id,
            plan_id: None,
            session_id: None,
            story_ids: Vec::new(),
            design_ids: Vec::new(),
            project_id: String::new(),
            issue_id: String::new(),
        },
    })
}

/// 读生成检查点（既有版本化读侧；旧代 `LegacyUnbound` 语义不变）。
fn read_generation_intent_facts(
    path: &Path,
    record_path: &str,
    project_id: &str,
    issue_id: &str,
) -> Result<IntentFacts, ProductStoreError> {
    Ok(match read_plan_generation_intent(path)? {
        PlanGenerationIntentRead::Current(intent) => IntentFacts {
            record_path: record_path.to_string(),
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
            record_path: record_path.to_string(),
            enrollment_id,
            plan_id: Some(plan_id),
            session_id: None,
            story_ids: Vec::new(),
            design_ids: Vec::new(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
        },
    })
}

/// 按判据表登记顺序展开一条 intent 记录的全部待核对引用。
fn intent_references(rule: &DanglingReferenceRule, facts: &IntentFacts) -> Vec<CandidateReference> {
    let mut references = Vec::new();
    for field in rule.fields {
        match field.target_kind {
            DanglingTargetKind::StorySpec => {
                for id in &facts.story_ids {
                    references.push(facts.reference(field.path, field.target_kind, id.clone()));
                }
            }
            DanglingTargetKind::DesignSpec => {
                for id in &facts.design_ids {
                    references.push(facts.reference(field.path, field.target_kind, id.clone()));
                }
            }
            DanglingTargetKind::Plan => {
                if let Some(id) = facts.plan_id.as_ref().filter(|id| !id.is_empty()) {
                    references.push(facts.reference(field.path, field.target_kind, id.clone()));
                }
            }
            DanglingTargetKind::Session => {
                if let Some(id) = facts.session_id.as_ref().filter(|id| !id.is_empty()) {
                    references.push(facts.reference(field.path, field.target_kind, id.clone()));
                }
            }
            DanglingTargetKind::Enrollment if !facts.enrollment_id.is_empty() => {
                references.push(facts.reference(
                    field.path,
                    field.target_kind,
                    facts.enrollment_id.clone(),
                ));
            }
            _ => {}
        }
    }
    references
}

/// 判定一条 intent 条目：悬空 → 跳过 + 一条告警；否则保留。
fn push_intent_entry(
    outcome: &mut ListingOutcome<AutomationIntentEntry>,
    resolver: &DanglingResolver<'_>,
    record_type: DanglingRecordType,
    file_name: &'static str,
    facts: &IntentFacts,
) -> Result<(), ProductStoreError> {
    let rule = reference_rule(record_type).expect("判据表必须登记 intent 型记录");
    let references = intent_references(rule, facts);
    if let Some(dangling) = first_dangling_reference(resolver, &references)? {
        push_skipped_warning(
            &mut outcome.warnings,
            &facts.record_path,
            record_type,
            dangling,
        );
        return Ok(());
    }
    outcome.items.push(AutomationIntentEntry {
        file_name,
        record_path: facts.record_path.clone(),
        enrollment_id: facts.enrollment_id.clone(),
        plan_id: facts.plan_id.clone(),
        session_id: facts.session_id.clone(),
        story_ids: facts.story_ids.clone(),
        design_ids: facts.design_ids.clone(),
    });
    Ok(())
}

/// reviewer 窗口清单（容错列举；REQ-DRG-03）。
///
/// 包装既有 `ReviewerWindowStore::list_windows`（既有函数不改）；按
/// `attempt_id` 走两路核对，两路皆无 → 跳过 + 告警；任一路命中 → 保留。
pub(crate) fn list_reviewer_windows(
    paths: &ProductAppPaths,
    resolver: &DanglingResolver<'_>,
) -> Result<ListingOutcome<ReviewerWindowEntry>, ProductStoreError> {
    let store = ReviewerWindowStore::new(paths.root().join(REVIEWER_WINDOW_STORE_DIR));
    let records = store.list_windows()?;
    let rule =
        reference_rule(DanglingRecordType::ReviewerWindow).expect("判据表必须登记 reviewer window");
    let field_path = field_path_for(rule, DanglingTargetKind::Attempt);
    let mut outcome = empty_outcome();
    for record in records {
        let path = paths
            .root()
            .join(REVIEWER_WINDOW_STORE_DIR)
            .join("windows")
            .join(format!("{}.json", record.window_id));
        let record_path = record_path_of(paths, &path);
        let reference = CandidateReference {
            field_path,
            target_kind: DanglingTargetKind::Attempt,
            target_id: record.attempt_id.clone(),
            project_id: String::new(),
            issue_id: String::new(),
        };
        if let Some(dangling) =
            first_dangling_reference(resolver, std::slice::from_ref(&reference))?
        {
            push_skipped_warning(
                &mut outcome.warnings,
                &record_path,
                DanglingRecordType::ReviewerWindow,
                dangling,
            );
            continue;
        }
        outcome.items.push(ReviewerWindowEntry {
            record_path,
            window_id: record.window_id.clone(),
            attempt_id: record.attempt_id.clone(),
        });
    }
    Ok(outcome)
}

/// reviewer 快照清单（容错列举；REQ-DRG-03）。
///
/// 枚举 `<root>/reviewer-snapshots/windows/*/manifest.json`，读既有
/// `ReviewerBaselineManifest`；解析失败原样上抛（不得折叠成「跳过」），
/// 两路皆无 → 跳过 + 告警。
pub(crate) fn list_reviewer_snapshots(
    paths: &ProductAppPaths,
    resolver: &DanglingResolver<'_>,
) -> Result<ListingOutcome<ReviewerSnapshotEntry>, ProductStoreError> {
    let windows_root = paths
        .root()
        .join(REVIEWER_SNAPSHOT_STORE_DIR)
        .join("windows");
    let rule = reference_rule(DanglingRecordType::ReviewerSnapshotManifest)
        .expect("判据表必须登记 reviewer 快照 manifest");
    let field_path = field_path_for(rule, DanglingTargetKind::Attempt);
    let mut outcome = empty_outcome();
    for (window_dir, is_dir) in read_dir_sorted(&windows_root)? {
        if !is_dir {
            continue;
        }
        let manifest_path = window_dir.join("manifest.json");
        if !path_exists(&manifest_path)? {
            continue;
        }
        let manifest: ReviewerBaselineManifest = read_json(&manifest_path)?;
        let record_path = record_path_of(paths, &manifest_path);
        let reference = CandidateReference {
            field_path,
            target_kind: DanglingTargetKind::Attempt,
            target_id: manifest.attempt_id.clone(),
            project_id: String::new(),
            issue_id: String::new(),
        };
        if let Some(dangling) =
            first_dangling_reference(resolver, std::slice::from_ref(&reference))?
        {
            push_skipped_warning(
                &mut outcome.warnings,
                &record_path,
                DanglingRecordType::ReviewerSnapshotManifest,
                dangling,
            );
            continue;
        }
        outcome.items.push(ReviewerSnapshotEntry {
            record_path,
            window_id: manifest.window_id.clone(),
            attempt_id: manifest.attempt_id.clone(),
        });
    }
    Ok(outcome)
}

/// 实体身份台账清单（容错列举；REQ-DRG-03）。
///
/// 枚举 `<root>/state/entity-identity/sessions/*.json`，按通用 JSON 读
/// `entity_kind` / `entity_id` / `project_id` / `issue_id`（既有绑定结构体
/// 是私有类型，MUST NOT 改动它）；`entity_kind` 为 `story` / `design` /
/// `plan` 时按判据表分派到命中的那一行核对，目标不存在 → 跳过 + 告警。
pub(crate) fn list_entity_identity_bindings(
    paths: &ProductAppPaths,
    resolver: &DanglingResolver<'_>,
) -> Result<ListingOutcome<EntityIdentityBindingEntry>, ProductStoreError> {
    let sessions_root = paths.state_root().join("entity-identity").join("sessions");
    let rule = reference_rule(DanglingRecordType::EntityIdentitySessionBinding)
        .expect("判据表必须登记实体身份台账");
    let mut outcome = empty_outcome();
    for (path, _) in read_dir_sorted(&sessions_root)? {
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let value: serde_json::Value = read_json(&path)?;
        let record_path = record_path_of(paths, &path);
        let session_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        let project_id = json_str_field(&value, "project_id");
        let issue_id = json_str_field(&value, "issue_id");
        let entity_kind = json_str_field(&value, "entity_kind");
        let entity_id = json_str_field(&value, "entity_id");
        let entry = EntityIdentityBindingEntry {
            record_path: record_path.clone(),
            session_id,
            entity_kind: entity_kind.clone(),
            entity_id: entity_id.clone(),
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
        };
        // 按 `entity_kind` 值分派（表外值不猜引用语义 → 条目保留、无告警）。
        let Some(target_kind) = entity_kind_target_kind(&entity_kind) else {
            outcome.items.push(entry);
            continue;
        };
        if entity_id.is_empty() {
            outcome.items.push(entry);
            continue;
        }
        let reference = CandidateReference {
            field_path: field_path_for(rule, target_kind),
            target_kind,
            target_id: entity_id,
            project_id,
            issue_id,
        };
        if let Some(dangling) =
            first_dangling_reference(resolver, std::slice::from_ref(&reference))?
        {
            push_skipped_warning(
                &mut outcome.warnings,
                &record_path,
                DanglingRecordType::EntityIdentitySessionBinding,
                dangling,
            );
            continue;
        }
        outcome.items.push(entry);
    }
    Ok(outcome)
}

/// 台账条目的 `entity_kind` 值 → 引用目标类型（判据表按值分派）。
pub(crate) fn entity_kind_target_kind(entity_kind: &str) -> Option<DanglingTargetKind> {
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

#[cfg(test)]
mod listing_tests {
    use super::*;
    use crate::cross_cutting::provider_resources::ResourceScopeKey;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::json_store::ProductStoreError;
    use crate::product::reviewer_snapshot::{
        GitObservationBaseline, REVIEWER_BASELINE_SCHEMA, RestoreCursor, RestorePhase,
        ReviewWorktreeLockIdentity, ReviewerBaselineManifest,
    };
    use crate::product::reviewer_snapshot_domain::{DOMAIN_BOUNDARY_DISCLOSURE, IgnoreRulesFreeze};
    use crate::product::reviewer_window_store::ReviewerWindowRecord;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn probe_false() -> impl Fn(&str) -> Result<bool, ProductStoreError> {
        |_id: &str| Ok(false)
    }

    /// 路一探针：只有 `alive` 名单内的 attempt 命中（其余一律「已删」）。
    fn probe_for(
        alive: &'static [&'static str],
    ) -> impl Fn(&str) -> Result<bool, ProductStoreError> {
        move |id: &str| Ok(alive.contains(&id))
    }

    fn write(path: &Path, body: &str) {
        write_bytes(path, body.as_bytes());
    }

    fn write_bytes(path: &Path, body: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn plan_intent_body() -> &'static str {
        r#"{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x"}"#
    }

    fn generation_intent_body() -> &'static str {
        r#"{"enrollment_id":"e1","plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x","action_key":"e1:issue_work_item_plan_0007:start_generation","source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"phase":"claimed"}"#
    }

    /// 全部引用都还活着的 intent（空 source + 存活的 plan + 存活的 session）。
    fn alive_plan_intent_body() -> &'static str {
        r#"{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{"stories":[],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x"}"#
    }

    /// 让 `issue_work_item_plan_0007` 成为存活 plan（schema-v2 线路）。
    fn write_live_plan(paths: &ProductAppPaths, project_id: &str, issue_id: &str, plan_id: &str) {
        write(
            &paths
                .issue_root(project_id, issue_id)
                .join("work-item-revisions")
                .join(plan_id)
                .join("lineage.json"),
            "{}",
        );
    }

    fn window_record(window_id: &str, attempt_id: &str) -> ReviewerWindowRecord {
        ReviewerWindowRecord::new(
            window_id,
            ReviewWorktreeLockIdentity {
                canonical_worktree: PathBuf::from("/tmp/aria-listing/wt"),
                gitdir: PathBuf::from("/tmp/aria-listing/wt/.git"),
                common_dir: PathBuf::from("/tmp/aria-listing/wt/.git"),
                lease_id: "lease-1".to_string(),
            },
            attempt_id,
            "run-1",
            "codex",
            Vec::new(),
        )
    }

    fn baseline_manifest(window_id: &str, attempt_id: &str) -> ReviewerBaselineManifest {
        ReviewerBaselineManifest {
            schema: REVIEWER_BASELINE_SCHEMA.to_string(),
            window_id: window_id.to_string(),
            attempt_id: attempt_id.to_string(),
            run_id: "run-1".to_string(),
            lock_identity: ReviewWorktreeLockIdentity {
                canonical_worktree: PathBuf::from("/tmp/aria-listing/wt"),
                gitdir: PathBuf::from("/tmp/aria-listing/wt/.git"),
                common_dir: PathBuf::from("/tmp/aria-listing/wt/.git"),
                lease_id: "lease-1".to_string(),
            },
            entries: Vec::new(),
            git_observation: GitObservationBaseline {
                head_symref: Some("refs/heads/main".to_string()),
                head_sha: "0".repeat(40),
                index_blob: "blobs/0".to_string(),
                index_sha256: "0".repeat(64),
                managed_refs: Vec::new(),
                refs_digest: "0".repeat(64),
                index_semantic_sha256: None,
            },
            platform_write_baseline: Vec::new(),
            scope_key: ResourceScopeKey {
                scope_id: "scope".to_string(),
                run_id: "run-1".to_string(),
                boot_identity: "boot".to_string(),
            },
            scope_manifest: Vec::new(),
            quiesced_at: "2026-10-17T00:00:00Z".to_string(),
            captured_at: "2026-10-17T00:00:00Z".to_string(),
            git_annex_observation: None,
            restore_cursor: RestoreCursor {
                phase: RestorePhase::Completed,
                entry_index: 0,
            },
            ignore_rules_freeze: IgnoreRulesFreeze::synthetic(),
            domain_boundary: DOMAIN_BOUNDARY_DISCLOSURE.to_string(),
        }
    }

    #[test]
    fn dangling_listing_skips_dangling_entries_and_warns_once() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let issue_root = paths.issue_root("project_0001", "issue_0001");
        write(
            &issue_root.join("automation-plan-intent.json"),
            plan_intent_body(),
        );
        write(
            &issue_root.join("automation-generation-intent.json"),
            generation_intent_body(),
        );
        write(
            &issue_root.join("automation-enrollment.json"),
            plan_intent_body(),
        );

        let sessions = BTreeSet::new();
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
        let outcome = list_automation_intents(&paths, "project_0001", "issue_0001", &resolver)
            .expect("list intents");
        assert_eq!(outcome.items.len(), 0, "两条 intent 均悬空，均须跳过");
        assert_eq!(outcome.warnings.len(), 2, "每条悬空 intent 各一条告警");
        assert_eq!(
            outcome.warnings.items()[0].code,
            "dangling_reference_target_missing"
        );
        // 告警集合按去重键升序（`record_path` / `reference_field` / `target_id`），
        // 故按记录类型定位断言，不依赖入列次序。
        let plan_warning = outcome
            .warnings
            .items()
            .iter()
            .find(|warning| warning.record_type == "automation_plan_intent")
            .expect("计划意图告警");
        assert_eq!(plan_warning.code, "dangling_reference_target_missing");
        assert_eq!(plan_warning.reference_field, "source.stories[].id");
        assert_eq!(plan_warning.target_kind, "story_spec");
        assert_eq!(plan_warning.target_id, "story_spec_0005");
        assert_eq!(plan_warning.action, "skipped");
        assert!(
            plan_warning
                .record_path
                .ends_with("automation-plan-intent.json")
        );
        let generation_warning = outcome
            .warnings
            .items()
            .iter()
            .find(|warning| warning.record_type == "automation_generation_intent")
            .expect("生成检查点告警");
        assert_eq!(generation_warning.reference_field, "source.stories[].id");
    }

    #[test]
    fn dangling_listing_intent_excludes_enrollment_record() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let issue_root = paths.issue_root("project_0001", "issue_0001");
        write(
            &issue_root.join("automation-plan-intent.json"),
            plan_intent_body(),
        );
        write(
            &issue_root.join("automation-generation-intent.json"),
            generation_intent_body(),
        );
        write(
            &issue_root.join("automation-enrollment.json"),
            plan_intent_body(),
        );

        let sessions = BTreeSet::new();
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
        let outcome = list_automation_intents(&paths, "project_0001", "issue_0001", &resolver)
            .expect("list intents");
        for item in &outcome.items {
            assert!(
                item.file_name == "automation-plan-intent.json"
                    || item.file_name == "automation-generation-intent.json",
                "intent 清单不得包含 enrollment：{}",
                item.file_name
            );
        }
    }

    #[test]
    fn dangling_listing_propagates_parse_and_io_errors() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let issue_root = paths.issue_root("project_0001", "issue_0001");
        write(
            &issue_root.join("automation-plan-intent.json"),
            "{ not json",
        );
        let sessions = BTreeSet::new();
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
        let error = list_automation_intents(&paths, "project_0001", "issue_0001", &resolver)
            .expect_err("解析失败必须上抛");
        assert!(matches!(error, ProductStoreError::Json(_)));

        let temp2 = TempDir::new().unwrap();
        let paths2 = ProductAppPaths::new(temp2.path());
        std::fs::write(paths2.root().join("reviewer-windows"), b"not a directory").unwrap();
        let sessions2 = BTreeSet::new();
        let probe2 = probe_false();
        let resolver2 = DanglingResolver::with_attempt_probe(&paths2, &sessions2, &probe2);
        let error = list_reviewer_windows(&paths2, &resolver2).expect_err("IO 失败必须上抛");
        assert!(matches!(error, ProductStoreError::Io(_)));
    }

    #[test]
    fn dangling_listing_intent_returns_alive_entries() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let issue_root = paths.issue_root("project_0001", "issue_0001");
        write(
            &issue_root.join("automation-plan-intent.json"),
            alive_plan_intent_body(),
        );
        write_live_plan(
            &paths,
            "project_0001",
            "issue_0001",
            "issue_work_item_plan_0007",
        );

        let mut sessions = BTreeSet::new();
        sessions.insert("workspace_session_auto_x".to_string());
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
        let outcome = list_automation_intents(&paths, "project_0001", "issue_0001", &resolver)
            .expect("list intents");
        assert_eq!(outcome.items.len(), 1, "存活 intent 必须全部返回");
        assert_eq!(outcome.warnings.len(), 0, "存活 intent 不得产生告警");
        assert_eq!(outcome.items[0].file_name, "automation-plan-intent.json");
        assert_eq!(outcome.items[0].enrollment_id, "e1");
        assert_eq!(
            outcome.items[0].plan_id.as_deref(),
            Some("issue_work_item_plan_0007")
        );
        assert_eq!(
            outcome.items[0].session_id.as_deref(),
            Some("workspace_session_auto_x")
        );
        assert!(outcome.items[0].story_ids.is_empty());
        assert!(outcome.items[0].design_ids.is_empty());
    }

    #[test]
    fn dangling_listing_window_skips_dangling_and_keeps_alive() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let windows_dir = paths.root().join("reviewer-windows").join("windows");
        write_bytes(
            &windows_dir.join("win_alive.json"),
            &serde_json::to_vec(&window_record("win_alive", "attempt_alive")).unwrap(),
        );
        write_bytes(
            &windows_dir.join("win_gone.json"),
            &serde_json::to_vec(&window_record("win_gone", "attempt_gone")).unwrap(),
        );

        let sessions = BTreeSet::new();
        let probe = probe_for(&["attempt_alive"]);
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
        let outcome = list_reviewer_windows(&paths, &resolver).expect("list windows");
        assert_eq!(outcome.items.len(), 1, "悬空窗口被跳过，其余窗口仍返回");
        assert_eq!(outcome.items[0].window_id, "win_alive");
        assert_eq!(outcome.items[0].attempt_id, "attempt_alive");
        assert_eq!(outcome.warnings.len(), 1, "每条悬空窗口一条告警");
        let warning = &outcome.warnings.items()[0];
        assert_eq!(warning.code, "dangling_reference_target_missing");
        assert_eq!(warning.record_type, "reviewer_window");
        assert_eq!(warning.reference_field, "attempt_id");
        assert_eq!(warning.target_kind, "attempt");
        assert_eq!(warning.target_id, "attempt_gone");
        assert_eq!(warning.action, "skipped");
    }

    #[test]
    fn dangling_listing_snapshot_skips_dangling_and_keeps_alive() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let windows_root = paths.root().join("reviewer-snapshots").join("windows");
        write_bytes(
            &windows_root.join("win_alive").join("manifest.json"),
            &serde_json::to_vec(&baseline_manifest("win_alive", "attempt_alive")).unwrap(),
        );
        write_bytes(
            &windows_root.join("win_gone").join("manifest.json"),
            &serde_json::to_vec(&baseline_manifest("win_gone", "attempt_gone")).unwrap(),
        );

        let sessions = BTreeSet::new();
        let probe = probe_for(&["attempt_alive"]);
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
        let outcome = list_reviewer_snapshots(&paths, &resolver).expect("list snapshots");
        assert_eq!(outcome.items.len(), 1, "悬空快照被跳过，其余快照仍返回");
        assert_eq!(outcome.items[0].window_id, "win_alive");
        assert_eq!(outcome.items[0].attempt_id, "attempt_alive");
        assert_eq!(outcome.warnings.len(), 1, "每条悬空快照一条告警");
        let warning = &outcome.warnings.items()[0];
        assert_eq!(warning.record_type, "reviewer_snapshot_manifest");
        assert_eq!(warning.reference_field, "attempt_id");
        assert_eq!(warning.target_kind, "attempt");
        assert_eq!(warning.target_id, "attempt_gone");
    }

    #[test]
    fn dangling_listing_snapshot_propagates_parse_error() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        write(
            &paths
                .root()
                .join("reviewer-snapshots")
                .join("windows")
                .join("win_broken")
                .join("manifest.json"),
            "{ not json",
        );
        let sessions = BTreeSet::new();
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
        let error = list_reviewer_snapshots(&paths, &resolver).expect_err("解析失败必须上抛");
        assert!(matches!(error, ProductStoreError::Json(_)));
    }

    #[test]
    fn dangling_listing_entity_binding_skips_dangling_and_keeps_alive() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let sessions_dir = paths.state_root().join("entity-identity").join("sessions");
        write(
            &sessions_dir.join("workspace_session_alive.json"),
            r#"{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"plan","entity_id":"issue_work_item_plan_0007"}"#,
        );
        write(
            &sessions_dir.join("workspace_session_gone.json"),
            r#"{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"story","entity_id":"story_spec_0005"}"#,
        );
        write_live_plan(
            &paths,
            "project_0001",
            "issue_0001",
            "issue_work_item_plan_0007",
        );

        let sessions = BTreeSet::new();
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
        let outcome =
            list_entity_identity_bindings(&paths, &resolver).expect("list entity bindings");
        assert_eq!(outcome.items.len(), 1, "悬空台账条目被跳过，其余条目仍返回");
        assert_eq!(outcome.items[0].session_id, "workspace_session_alive");
        assert_eq!(outcome.items[0].entity_kind, "plan");
        assert_eq!(outcome.items[0].entity_id, "issue_work_item_plan_0007");
        assert_eq!(outcome.items[0].project_id, "project_0001");
        assert_eq!(outcome.items[0].issue_id, "issue_0001");
        assert_eq!(outcome.warnings.len(), 1, "每条悬空台账条目一条告警");
        let warning = &outcome.warnings.items()[0];
        assert_eq!(warning.record_type, "entity_identity_session_binding");
        assert_eq!(warning.reference_field, "entity_kind+entity_id");
        assert_eq!(warning.target_kind, "story_spec");
        assert_eq!(warning.target_id, "story_spec_0005");
    }

    #[test]
    fn dangling_listing_entity_binding_propagates_parse_error() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        write(
            &paths
                .state_root()
                .join("entity-identity")
                .join("sessions")
                .join("workspace_session_broken.json"),
            "{ not json",
        );
        let sessions = BTreeSet::new();
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
        let error = list_entity_identity_bindings(&paths, &resolver).expect_err("解析失败必须上抛");
        assert!(matches!(error, ProductStoreError::Json(_)));
    }

    #[test]
    fn dangling_listing_undetermined_u1_kinds_never_produce_finding() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let sessions = BTreeSet::new();
        let probe = probe_false();
        let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);

        // U1 四类：存在性来源在本判定面不可用 → 一律「不可判定」，不得当悬空。
        for kind in [
            DanglingTargetKind::Checkout,
            DanglingTargetKind::AggregateIndex,
            DanglingTargetKind::LogicalMember,
            DanglingTargetKind::Enrollment,
        ] {
            let reference = CandidateReference {
                field_path: "u1",
                target_kind: kind,
                target_id: "target_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
            };
            assert_eq!(
                classify_reference(&resolver, &reference).unwrap(),
                ReferenceVerdict::Undetermined,
                "{kind:?} 的存在性来源不可用，必须归「不可判定」而非「已删」"
            );
        }

        // 列举面：唯一引用为 U1 类的 intent（enrollment_id）→ 条目保留、零告警。
        let issue_root = paths.issue_root("project_0001", "issue_0001");
        write(
            &issue_root.join("automation-plan-intent.json"),
            alive_plan_intent_body(),
        );
        write_live_plan(
            &paths,
            "project_0001",
            "issue_0001",
            "issue_work_item_plan_0007",
        );
        let mut alive_sessions = BTreeSet::new();
        alive_sessions.insert("workspace_session_auto_x".to_string());
        let alive_probe = probe_false();
        let alive_resolver =
            DanglingResolver::with_attempt_probe(&paths, &alive_sessions, &alive_probe);
        let outcome =
            list_automation_intents(&paths, "project_0001", "issue_0001", &alive_resolver)
                .expect("list intents");
        assert_eq!(outcome.items.len(), 1, "不可判定的引用不得使条目被跳过");
        assert_eq!(
            outcome.warnings.len(),
            0,
            "不可判定的引用不得产生告警、不得计入悬空条目数"
        );
    }
}
