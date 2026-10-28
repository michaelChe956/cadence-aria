//! 悬空引用判据表：记录类型 → 引用字段路径 → 目标实体类型（REQ-DRG-01 / REQ-DRG-09）。
//!
//! 判定依据 MUST 是**字段名**而非实体 id 前缀（REQ-DRG-01）；表外记录类型归
//! `Unclassified`，一律只报不删（REQ-DRG-09）。本表只承载静态映射，不存在任何 IO。
//!
//! 技术方案 §3.3.2 的判据表逐行对应本文件的 [`ALL_RULES`]：本表管「**发现**」，
//! 不管「能不能删」（可删档位见 REQ-DRG-02 的证据等级表与 REQ-DRG-12 的销账规则）。

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
