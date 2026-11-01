//! 悬空引用判据表：记录类型 → 引用字段路径 → 目标实体类型（REQ-DRG-01 / REQ-DRG-09）。
//!
//! 判定依据 MUST 是**字段名**而非实体 id 前缀（REQ-DRG-01）；表外记录类型归
//! `Unclassified`，一律只报不删（REQ-DRG-09）。本表只承载静态映射，不存在任何 IO。
//!
//! 技术方案 §3.3.2 的判据表逐行对应本文件的 [`ALL_RULES`]：本表管「**发现**」。
//!
//! 「**能不能删**」由本文件的**证据等级表**（[`EVIDENCE_POLICIES`]）决定：
//! REQ-DRG-02 附表 #1–#10 **逐行照抄**「目标类型 → 存在性来源 → 唯一可达上游
//! → L2 专属可算条件 → 可删证据等级」五列，是销账白名单与证据等级的**单一
//! 事实源**。附表同时登记**分派口径**（[`ExistenceDispatch`]）、**作用域需求**
//! （[`ScopeRequirement`]）与**墓碑实体种类**，使①层（`resolve`）与巡检面
//! （`patrol`）共用**同一张表**做存在性分派，不再各写一套 per-target 分支。

use crate::product::entity_deletion_store::DeletionEntityKind;

/// 引用目标的九类实体类型（REQ-DRG-01）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DanglingTargetKind {
    StorySpec,
    DesignSpec,
    Plan,
    Session,
    Attempt,
    Checkout,
    AggregateIndex,
    /// 逻辑代码库成员。
    ///
    /// 注：spec REQ-DRG-01 正文把本类写作 `logical_repository(member)`，该处措辞
    /// 待订正（属契约修订，另行处理）；实现按技术方案 §3.1.2 / §3.3.2 / §5.1 与
    /// 实施计划的 `logical_member` 取值。
    LogicalMember,
    Enrollment,
}

impl DanglingTargetKind {
    /// 稳定标识：告警与巡检清单的「引用目标类型」列取值。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::StorySpec => "story_spec",
            Self::DesignSpec => "design_spec",
            Self::Plan => "plan",
            Self::Session => "session",
            Self::Attempt => "attempt",
            Self::Checkout => "checkout",
            Self::AggregateIndex => "aggregate_index",
            Self::LogicalMember => "logical_member",
            Self::Enrollment => "enrollment",
        }
    }
}

/// 参与悬空判定的一类持久化记录（REQ-DRG-09）。
///
/// `Unclassified` 表示判据表未覆盖的记录类型：只报不删，且 MUST NOT 为它推断
/// 引用语义（REQ-DRG-01 的「表外记录类型不猜引用语义」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DanglingRecordType {
    AutomationPlanIntent,
    AutomationGenerationIntent,
    AutomationEnrollment,
    ReviewerWindow,
    ReviewerSnapshotManifest,
    EntityIdentitySessionBinding,
    LogicalCodebaseMember,
    Unclassified,
}

impl DanglingRecordType {
    /// 稳定标识：告警与巡检清单的「记录类型」列取值。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AutomationPlanIntent => "automation_plan_intent",
            Self::AutomationGenerationIntent => "automation_generation_intent",
            Self::AutomationEnrollment => "automation_enrollment",
            Self::ReviewerWindow => "reviewer_window",
            Self::ReviewerSnapshotManifest => "reviewer_snapshot_manifest",
            Self::EntityIdentitySessionBinding => "entity_identity_session_binding",
            Self::LogicalCodebaseMember => "logical_codebase_member",
            Self::Unclassified => "unclassified",
        }
    }
}

/// 一条引用字段的静态判据：字段路径 + 该字段指向的目标实体类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DanglingReferenceField {
    pub(crate) path: &'static str,
    pub(crate) target_kind: DanglingTargetKind,
}

/// 一类记录的全部引用字段判据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DanglingReferenceRule {
    pub(crate) record_type: DanglingRecordType,
    pub(crate) fields: &'static [DanglingReferenceField],
}

/// 自动化 intent 记录的五个授权引用字段（REQ-DRG-09 逐项清单：intent 型）。
///
/// `plan_id` / `session_id` / `enrollment_id` 是预派生 id，其目标缺失属于正常
/// 中间态——本表只负责登记与发现，可删性由 L1 证据决定（不因本表登记而可删）。
const INTENT_FIELDS: &[DanglingReferenceField] = &[
    DanglingReferenceField {
        path: "source.stories[].id",
        target_kind: DanglingTargetKind::StorySpec,
    },
    DanglingReferenceField {
        path: "source.designs[].id",
        target_kind: DanglingTargetKind::DesignSpec,
    },
    DanglingReferenceField {
        path: "plan_id",
        target_kind: DanglingTargetKind::Plan,
    },
    DanglingReferenceField {
        path: "session_id",
        target_kind: DanglingTargetKind::Session,
    },
    DanglingReferenceField {
        path: "enrollment_id",
        target_kind: DanglingTargetKind::Enrollment,
    },
];

/// 自动化 enrollment 记录的全部引用字段（REQ-DRG-09 逐项清单：enrollment 型）。
///
/// 含顶层 `plan_id` / `session_id` / `source.*`、当前绑定代际
/// `binding_history.current.*` 的 plan / session / source，以及历史代
/// `binding_history.previous[].*` 的 source 留痕副本。enrollment 记录本身永不删。
const ENROLLMENT_FIELDS: &[DanglingReferenceField] = &[
    DanglingReferenceField {
        path: "source.stories[].id",
        target_kind: DanglingTargetKind::StorySpec,
    },
    DanglingReferenceField {
        path: "source.designs[].id",
        target_kind: DanglingTargetKind::DesignSpec,
    },
    DanglingReferenceField {
        path: "plan_id",
        target_kind: DanglingTargetKind::Plan,
    },
    DanglingReferenceField {
        path: "session_id",
        target_kind: DanglingTargetKind::Session,
    },
    DanglingReferenceField {
        path: "binding_history.current.plan_id",
        target_kind: DanglingTargetKind::Plan,
    },
    DanglingReferenceField {
        path: "binding_history.current.session_id",
        target_kind: DanglingTargetKind::Session,
    },
    DanglingReferenceField {
        path: "binding_history.current.source.stories[].id",
        target_kind: DanglingTargetKind::StorySpec,
    },
    DanglingReferenceField {
        path: "binding_history.current.source.designs[].id",
        target_kind: DanglingTargetKind::DesignSpec,
    },
    DanglingReferenceField {
        path: "binding_history.previous[].source.stories[].id",
        target_kind: DanglingTargetKind::StorySpec,
    },
    DanglingReferenceField {
        path: "binding_history.previous[].source.designs[].id",
        target_kind: DanglingTargetKind::DesignSpec,
    },
];

/// 评审窗口 / 快照 manifest 的引用字段：`attempt_id` 指向 attempt（两路核对）。
const WINDOW_FIELDS: &[DanglingReferenceField] = &[DanglingReferenceField {
    path: "attempt_id",
    target_kind: DanglingTargetKind::Attempt,
}];

/// 实体身份台账条目的引用：`entity_kind` 值决定目标类型，`entity_id` 是目标 id。
///
/// 同一路径重复登记三行 = **按 `entity_kind` 值分派**到其中之一核对
/// （技术方案 §3.3.2：Story/Design/Plan 按 `entity_kind` 分派）。逐条核对时 MUST
/// 只核对分派命中的那一行，MUST NOT 把同一引用按三类目标各报一次。
const ENTITY_BINDING_FIELDS: &[DanglingReferenceField] = &[
    DanglingReferenceField {
        path: "entity_kind+entity_id",
        target_kind: DanglingTargetKind::StorySpec,
    },
    DanglingReferenceField {
        path: "entity_kind+entity_id",
        target_kind: DanglingTargetKind::DesignSpec,
    },
    DanglingReferenceField {
        path: "entity_kind+entity_id",
        target_kind: DanglingTargetKind::Plan,
    },
];

/// 逻辑代码库 scope 记录的三个引用字段（§3.1.2 已降级为只报不删）。
const LOGICAL_CODEBASE_FIELDS: &[DanglingReferenceField] = &[
    DanglingReferenceField {
        path: "repository_id",
        target_kind: DanglingTargetKind::LogicalMember,
    },
    DanglingReferenceField {
        path: "checkout_id",
        target_kind: DanglingTargetKind::Checkout,
    },
    DanglingReferenceField {
        path: "aggregate_index_id",
        target_kind: DanglingTargetKind::AggregateIndex,
    },
];

/// 记录类型 → 引用字段 → 目标类型静态表（7 条记录类型，`Unclassified` 不在表内）。
const ALL_RULES: &[DanglingReferenceRule] = &[
    DanglingReferenceRule {
        record_type: DanglingRecordType::AutomationPlanIntent,
        fields: INTENT_FIELDS,
    },
    DanglingReferenceRule {
        record_type: DanglingRecordType::AutomationGenerationIntent,
        fields: INTENT_FIELDS,
    },
    DanglingReferenceRule {
        record_type: DanglingRecordType::AutomationEnrollment,
        fields: ENROLLMENT_FIELDS,
    },
    DanglingReferenceRule {
        record_type: DanglingRecordType::ReviewerWindow,
        fields: WINDOW_FIELDS,
    },
    DanglingReferenceRule {
        record_type: DanglingRecordType::ReviewerSnapshotManifest,
        fields: WINDOW_FIELDS,
    },
    DanglingReferenceRule {
        record_type: DanglingRecordType::EntityIdentitySessionBinding,
        fields: ENTITY_BINDING_FIELDS,
    },
    DanglingReferenceRule {
        record_type: DanglingRecordType::LogicalCodebaseMember,
        fields: LOGICAL_CODEBASE_FIELDS,
    },
];

/// 取某记录类型的判据；表外类型（含 `Unclassified`）返回 `None`。
pub(crate) fn reference_rule(
    record_type: DanglingRecordType,
) -> Option<&'static DanglingReferenceRule> {
    ALL_RULES
        .iter()
        .find(|rule| rule.record_type == record_type)
}

/// 全量判据表（供巡检输出「本次覆盖的记录类型」用）。
pub(crate) fn all_reference_rules() -> &'static [DanglingReferenceRule] {
    ALL_RULES
}

/// 某记录类型的**首个**引用字段的目标类型。
///
/// 窗口 / 快照记录的判据只登记一个引用字段（`attempt_id`），故其目标类型
/// 即首个字段的目标类型；取自判据表，MUST NOT 在调用方另写 per-target 常量。
pub(crate) fn primary_target_kind(record_type: DanglingRecordType) -> Option<DanglingTargetKind> {
    reference_rule(record_type)
        .and_then(|rule| rule.fields.first())
        .map(|field| field.target_kind)
}

// ---------------------------------------------------------------------------
// 证据等级表（REQ-DRG-02 附表 #1–#10：销账白名单的单一事实源）
// ---------------------------------------------------------------------------

/// 销账等级（附表「可删证据等级」列）。
///
/// 该列的**字面值**由 [`DeletableLevel::label`] 给出（`L1` / `L2` / `只报不删`
/// / `只报不删（永不删）`）；任何代码 MUST 从本枚举取值，MUST NOT 另立旁路常量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeletableLevel {
    /// L1：墓碑直证可删（story_spec / design_spec / plan）。
    L1,
    /// L2：该类型的专属可算条件成立才可删（attempt）。
    L2,
    /// 只报不删（session / checkout / aggregate_index / logical_member）。
    ReportOnly,
    /// 只报不删（永不删）（enrollment 记录永不删）。
    NeverDelete,
}

impl DeletableLevel {
    /// 附表「可删证据等级」列的字面值（单一事实源）。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::ReportOnly => "只报不删",
            Self::NeverDelete => "只报不删（永不删）",
        }
    }
}

/// 附表行的存在性核对路径（attempt 有两路；其余目标类型唯一一路）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistenceRoute {
    /// 唯一读路径（附表 #1–#4、#7–#10）。
    Sole,
    /// attempt 路 1：coding attempt（附表 #5）。
    CodingAttempt,
    /// attempt 路 2：规划面 workspace session（附表 #6）。
    WorkspaceSession,
}

/// 存在性读的分派口径（附表「存在性来源」列的**可执行**形态）。
///
/// ①层（`resolve`）与巡检面（`patrol`）都按本枚举分派，MUST NOT 各写一套
/// per-target 分支；具体读法见各变体注释。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistenceDispatch {
    /// 列举 issue 作用域下的 story_spec（`LifecycleStore::list_story_specs`）。
    StorySpecList,
    /// 列举 issue 作用域下的 design_spec（`LifecycleStore::list_design_specs`）。
    DesignSpecList,
    /// 列举 issue 作用域下的 live plan id（`list_live_plan_ids`）。
    LivePlanIds,
    /// 规划面 workspace session 枚举集合的成员关系（`list_workspace_sessions`）。
    WorkspaceSession,
    /// attempt 两路核对（路一 coding attempt + 路二 workspace session 集合）。
    AttemptTwoPath,
    /// 同 LC scope 内读逻辑成员（`LogicalCodebaseStore::list_members`）。
    LogicalMember,
    /// 同 LC scope 内读 checkout（`LogicalCodebaseStore::list_checkouts`）。
    Checkout,
    /// 同 LC scope 内读聚合索引（`AggregateIndexStore::get`）。
    AggregateIndex,
    /// 同 issue 作用域内读 enrollment 文件（`IssueAutomationStore::get`）。
    EnrollmentFile,
}

/// 作用域需求：判定面是否必须持有记录所属的 project/issue。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeRequirement {
    /// 不需要 issue 作用域（session / attempt / LC 三类）。
    None,
    /// 需要记录所属的 project/issue 作用域（story / design / plan / enrollment）。
    IssueScope,
}

/// 证据等级表一行（REQ-DRG-02 附表五列 + 分派列）。
///
/// 前五列**逐字照抄** spec 附表 #1–#10；后四列是同表登记的可执行口径，使
/// 「存在性怎么读」「要不要作用域」「墓碑是哪一种」也只此一处。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetEvidencePolicy {
    /// 附表「目标类型」列。
    pub(crate) target_kind: DanglingTargetKind,
    /// 存在性核对路径（附表 #5 / #6 的 attempt 两路；其余类型唯一一路）。
    pub(crate) route: ExistenceRoute,
    /// 附表「存在性来源」列（字面值）。
    pub(crate) existence_source: &'static str,
    /// 附表「唯一可达上游」列（字面值）。
    pub(crate) upstream: &'static str,
    /// 附表「L2 专属可算条件」列（字面值）。
    pub(crate) l2_condition: &'static str,
    /// 附表「可删证据等级」列。
    pub(crate) deletable_level: DeletableLevel,
    /// 存在性读的分派口径。
    pub(crate) dispatch: ExistenceDispatch,
    /// 作用域需求。
    pub(crate) scope_requirement: ScopeRequirement,
    /// 删除墓碑的实体种类（`None` = 该类型无墓碑档位，只报不删）。
    pub(crate) deletion_kind: Option<DeletionEntityKind>,
}

/// 证据等级表（REQ-DRG-02 附表 #1–#10 逐行照抄）：**销账白名单与证据等级的
/// 单一事实源**。
///
/// 附表十行 = 九类目标（attempt 占两行：路 1 / 路 2）。白名单与证据等级的取值
/// MUST 从本表读出，MUST NOT 旁路常量。
const EVIDENCE_POLICIES: [TargetEvidencePolicy; 10] = [
    // 附表 #1：StorySpec。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::StorySpec,
        route: ExistenceRoute::Sole,
        existence_source: "list_story_specs",
        upstream: "无上游（根实体）",
        l2_condition: "不可用（无上游可解析）",
        deletable_level: DeletableLevel::L1,
        dispatch: ExistenceDispatch::StorySpecList,
        scope_requirement: ScopeRequirement::IssueScope,
        deletion_kind: Some(DeletionEntityKind::Story),
    },
    // 附表 #2：DesignSpec。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::DesignSpec,
        route: ExistenceRoute::Sole,
        existence_source: "list_design_specs",
        upstream: "无上游（根实体）",
        l2_condition: "不可用（无上游可解析）",
        deletable_level: DeletableLevel::L1,
        dispatch: ExistenceDispatch::DesignSpecList,
        scope_requirement: ScopeRequirement::IssueScope,
        deletion_kind: Some(DeletionEntityKind::Design),
    },
    // 附表 #3：Plan。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Plan,
        route: ExistenceRoute::Sole,
        existence_source: "list_live_plan_ids",
        upstream: "plan → work item group → story/design（须 plan 记录本身才能解析）",
        l2_condition: "不可用（plan 记录已消失即无法解析）",
        deletable_level: DeletableLevel::L1,
        dispatch: ExistenceDispatch::LivePlanIds,
        scope_requirement: ScopeRequirement::IssueScope,
        deletion_kind: Some(DeletionEntityKind::Plan),
    },
    // 附表 #4：Session。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Session,
        route: ExistenceRoute::Sole,
        existence_source: "list_workspace_sessions",
        upstream: "session → enrollment 绑定",
        l2_condition: "不可用（无独立墓碑，且缺失属正常中间态）",
        deletable_level: DeletableLevel::ReportOnly,
        dispatch: ExistenceDispatch::WorkspaceSession,
        scope_requirement: ScopeRequirement::None,
        deletion_kind: None,
    },
    // 附表 #5：Attempt 路 1（coding attempt）。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Attempt,
        route: ExistenceRoute::CodingAttempt,
        existence_source: "get_attempt_by_id",
        upstream: "attempt 记录无 plan_id，上游不可从 attempt 解析",
        l2_condition: "由窗口记录自身给出的四条（REQ-DRG-12）",
        deletable_level: DeletableLevel::L2,
        dispatch: ExistenceDispatch::AttemptTwoPath,
        scope_requirement: ScopeRequirement::None,
        deletion_kind: None,
    },
    // 附表 #6：Attempt 路 2（规划面 workspace session）。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Attempt,
        route: ExistenceRoute::WorkspaceSession,
        existence_source: "list_workspace_sessions",
        upstream: "同 #4",
        l2_condition: "由窗口记录自身给出的四条（REQ-DRG-12）",
        deletable_level: DeletableLevel::L2,
        dispatch: ExistenceDispatch::AttemptTwoPath,
        scope_requirement: ScopeRequirement::None,
        deletion_kind: None,
    },
    // 附表 #7：Checkout。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Checkout,
        route: ExistenceRoute::Sole,
        existence_source: "LogicalCodebaseStore::list_checkouts（src/product/logical_codebase/store.rs:792）/ load_checkout（:777），须以 for_lc 作用域化；LC 由 LogicalCodebaseStore::list(project_id)（:249）枚举",
        upstream: "未核实，无法定义",
        l2_condition: "不可用（上游无法定义）",
        deletable_level: DeletableLevel::ReportOnly,
        dispatch: ExistenceDispatch::Checkout,
        scope_requirement: ScopeRequirement::None,
        deletion_kind: None,
    },
    // 附表 #8：AggregateIndex。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::AggregateIndex,
        route: ExistenceRoute::Sole,
        existence_source: "AggregateIndexStore::get（src/product/logical_codebase/aggregate_index/store.rs:72），须以 for_lc 作用域化 + 同上 LC 枚举",
        upstream: "未核实，无法定义",
        l2_condition: "不可用（上游无法定义）",
        deletable_level: DeletableLevel::ReportOnly,
        dispatch: ExistenceDispatch::AggregateIndex,
        scope_requirement: ScopeRequirement::None,
        deletion_kind: None,
    },
    // 附表 #9：LogicalMember。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::LogicalMember,
        route: ExistenceRoute::Sole,
        existence_source: "LogicalCodebaseStore::list_members（src/product/logical_codebase/store.rs:751）/ load_member（:736）＋ LogicalCodebaseManifest.member_ids（store.rs:32），须以 for_lc 作用域化 + 同上 LC 枚举",
        upstream: "未核实，无法定义",
        l2_condition: "不可用（上游无法定义）",
        deletable_level: DeletableLevel::ReportOnly,
        dispatch: ExistenceDispatch::LogicalMember,
        scope_requirement: ScopeRequirement::None,
        deletion_kind: None,
    },
    // 附表 #10：Enrollment。
    TargetEvidencePolicy {
        target_kind: DanglingTargetKind::Enrollment,
        route: ExistenceRoute::Sole,
        existence_source: "{issue_root}/automation-enrollment.json（IssueAutomationStore::get，src/product/issue_automation_store.rs:69）",
        upstream: "无上游（授权根）",
        l2_condition: "不可用（永不删）",
        deletable_level: DeletableLevel::NeverDelete,
        dispatch: ExistenceDispatch::EnrollmentFile,
        scope_requirement: ScopeRequirement::IssueScope,
        deletion_kind: None,
    },
];

/// 查证据等级表：九类目标必须全部登记，查无此行属编程错误（fail-fast）。
///
/// attempt 有两行（路 1 / 路 2），本函数取**第一行**（两行的「可删证据等级」
/// 与「L2 专属可算条件」取值一致，由 `dangling_evidence_policy_rows_are_consistent`
/// 锁定）。
pub(crate) fn evidence_policy(target_kind: DanglingTargetKind) -> &'static TargetEvidencePolicy {
    EVIDENCE_POLICIES
        .iter()
        .find(|policy| policy.target_kind == target_kind)
        .expect("证据等级表必须覆盖全部九类目标（REQ-DRG-02 附表）")
}

/// 某目标类型的全部附表行（attempt 两行，其余一行）。
pub(crate) fn evidence_policies(
    target_kind: DanglingTargetKind,
) -> &'static [TargetEvidencePolicy] {
    // 静态表的借用视图：同目标类型的行在表内连续，直接切出区间。
    let first = EVIDENCE_POLICIES
        .iter()
        .position(|policy| policy.target_kind == target_kind)
        .expect("证据等级表必须覆盖全部九类目标（REQ-DRG-02 附表）");
    let count = EVIDENCE_POLICIES
        .iter()
        .filter(|policy| policy.target_kind == target_kind)
        .count();
    &EVIDENCE_POLICIES[first..first + count]
}

/// 全量证据等级表（附表 #1–#10，供测试逐行锁定字面值）。
pub(crate) fn all_evidence_policies() -> &'static [TargetEvidencePolicy] {
    &EVIDENCE_POLICIES
}

#[cfg(test)]
mod reference_table_tests {
    use super::{DanglingRecordType, DanglingTargetKind, all_reference_rules, reference_rule};

    fn field_paths(record_type: DanglingRecordType) -> Vec<&'static str> {
        reference_rule(record_type)
            .expect("rule")
            .fields
            .iter()
            .map(|field| field.path)
            .collect()
    }

    /// 记录类型 → 引用字段判据的 `(字段路径, 目标类型标识)` 逐字视图。
    fn field_pairs(record_type: DanglingRecordType) -> Vec<(&'static str, &'static str)> {
        reference_rule(record_type)
            .expect("rule")
            .fields
            .iter()
            .map(|field| (field.path, field.target_kind.as_str()))
            .collect()
    }

    #[test]
    fn dangling_reference_rule_table_registers_intent_and_enrollment_fields() {
        assert_eq!(
            field_paths(DanglingRecordType::AutomationPlanIntent),
            vec![
                "source.stories[].id",
                "source.designs[].id",
                "plan_id",
                "session_id",
                "enrollment_id",
            ]
        );
        assert_eq!(
            field_paths(DanglingRecordType::AutomationGenerationIntent),
            vec![
                "source.stories[].id",
                "source.designs[].id",
                "plan_id",
                "session_id",
                "enrollment_id",
            ]
        );
        assert_eq!(
            field_paths(DanglingRecordType::AutomationEnrollment),
            vec![
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
            ]
        );
        assert!(reference_rule(DanglingRecordType::Unclassified).is_none());
        assert_eq!(all_reference_rules().len(), 7);
    }

    #[test]
    fn dangling_target_kind_as_str_locks_literal_values() {
        // 逐字锁定：九类目标类型的稳定标识是对外（告警 / 巡检清单）取值。
        let cases = [
            (DanglingTargetKind::StorySpec, "story_spec"),
            (DanglingTargetKind::DesignSpec, "design_spec"),
            (DanglingTargetKind::Plan, "plan"),
            (DanglingTargetKind::Session, "session"),
            (DanglingTargetKind::Attempt, "attempt"),
            (DanglingTargetKind::Checkout, "checkout"),
            (DanglingTargetKind::AggregateIndex, "aggregate_index"),
            (DanglingTargetKind::LogicalMember, "logical_member"),
            (DanglingTargetKind::Enrollment, "enrollment"),
        ];
        assert_eq!(cases.len(), 9, "REQ-DRG-01 规定九类目标类型");
        for (kind, literal) in cases {
            assert_eq!(kind.as_str(), literal, "{kind:?} 的稳定标识逐字锁定");
        }
    }

    #[test]
    fn dangling_record_type_as_str_locks_literal_values() {
        // 逐字锁定：八类记录类型的稳定标识是对外取值。
        let cases = [
            (
                DanglingRecordType::AutomationPlanIntent,
                "automation_plan_intent",
            ),
            (
                DanglingRecordType::AutomationGenerationIntent,
                "automation_generation_intent",
            ),
            (
                DanglingRecordType::AutomationEnrollment,
                "automation_enrollment",
            ),
            (DanglingRecordType::ReviewerWindow, "reviewer_window"),
            (
                DanglingRecordType::ReviewerSnapshotManifest,
                "reviewer_snapshot_manifest",
            ),
            (
                DanglingRecordType::EntityIdentitySessionBinding,
                "entity_identity_session_binding",
            ),
            (
                DanglingRecordType::LogicalCodebaseMember,
                "logical_codebase_member",
            ),
            (DanglingRecordType::Unclassified, "unclassified"),
        ];
        assert_eq!(cases.len(), 8, "REQ-DRG-09 规定八类记录类型");
        for (record_type, literal) in cases {
            assert_eq!(
                record_type.as_str(),
                literal,
                "{record_type:?} 的稳定标识逐字锁定"
            );
        }
    }

    #[test]
    fn dangling_reference_rule_table_locks_field_path_to_target_kind() {
        let intent = vec![
            ("source.stories[].id", "story_spec"),
            ("source.designs[].id", "design_spec"),
            ("plan_id", "plan"),
            ("session_id", "session"),
            ("enrollment_id", "enrollment"),
        ];
        assert_eq!(
            field_pairs(DanglingRecordType::AutomationPlanIntent),
            intent
        );
        assert_eq!(
            field_pairs(DanglingRecordType::AutomationGenerationIntent),
            intent
        );

        assert_eq!(
            field_pairs(DanglingRecordType::AutomationEnrollment),
            vec![
                ("source.stories[].id", "story_spec"),
                ("source.designs[].id", "design_spec"),
                ("plan_id", "plan"),
                ("session_id", "session"),
                ("binding_history.current.plan_id", "plan"),
                ("binding_history.current.session_id", "session"),
                ("binding_history.current.source.stories[].id", "story_spec"),
                ("binding_history.current.source.designs[].id", "design_spec"),
                (
                    "binding_history.previous[].source.stories[].id",
                    "story_spec"
                ),
                (
                    "binding_history.previous[].source.designs[].id",
                    "design_spec"
                ),
            ]
        );

        // 实体身份台账：同一路径三行 = 按 `entity_kind` 值分派（只核对命中那一行）。
        assert_eq!(
            field_pairs(DanglingRecordType::EntityIdentitySessionBinding),
            vec![
                ("entity_kind+entity_id", "story_spec"),
                ("entity_kind+entity_id", "design_spec"),
                ("entity_kind+entity_id", "plan"),
            ]
        );

        // 逻辑代码库 scope：三条引用各自的目标类型。
        assert_eq!(
            field_pairs(DanglingRecordType::LogicalCodebaseMember),
            vec![
                ("repository_id", "logical_member"),
                ("checkout_id", "checkout"),
                ("aggregate_index_id", "aggregate_index"),
            ]
        );

        // 评审窗口与快照 manifest 共用同一判据：`attempt_id` → attempt（两路核对）。
        assert_eq!(
            field_pairs(DanglingRecordType::ReviewerWindow),
            vec![("attempt_id", "attempt")]
        );
        assert_eq!(
            field_pairs(DanglingRecordType::ReviewerSnapshotManifest),
            vec![("attempt_id", "attempt")]
        );
        assert_eq!(
            reference_rule(DanglingRecordType::ReviewerWindow)
                .expect("rule")
                .fields,
            reference_rule(DanglingRecordType::ReviewerSnapshotManifest)
                .expect("rule")
                .fields
        );
    }

    #[test]
    fn dangling_reference_rule_table_locks_registered_record_types() {
        // 登记顺序同样是判据表的一部分：增删记录类型必须显式改这里。
        let registered: Vec<DanglingRecordType> = all_reference_rules()
            .iter()
            .map(|rule| rule.record_type)
            .collect();
        assert_eq!(
            registered,
            vec![
                DanglingRecordType::AutomationPlanIntent,
                DanglingRecordType::AutomationGenerationIntent,
                DanglingRecordType::AutomationEnrollment,
                DanglingRecordType::ReviewerWindow,
                DanglingRecordType::ReviewerSnapshotManifest,
                DanglingRecordType::EntityIdentitySessionBinding,
                DanglingRecordType::LogicalCodebaseMember,
            ]
        );
        assert!(reference_rule(DanglingRecordType::Unclassified).is_none());
        assert!(registered.iter().all(|record_type| {
            !reference_rule(*record_type)
                .expect("rule")
                .fields
                .is_empty()
        }));
    }
}
