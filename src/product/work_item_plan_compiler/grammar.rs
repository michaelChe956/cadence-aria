//! `work-item-plan.md` 的稳定语法元数据。
//!
//! 本模块只登记语法契约，不执行 markdown 解析或诊断。解析器应将这些值视为
//! fail-closed 的白名单：结构化 section/key 不在白名单时必须拒绝，只有
//! [`FREE_TEXT_SECTIONS`] 中的尾部 section 可以保留自由文本。

/// 计划文档的固定一级标题。
pub const DOCUMENT_HEADING: &str = "# Work Item Plan";
/// 每个 work item 的固定二级标题前缀，后接 `WI-<digits>`。
pub const ITEM_HEADING_PREFIX: &str = "## Work Item WI-";
/// Work item 标识符的固定前缀。
pub const ITEM_ID_PREFIX: &str = "WI-";
/// Work item 标识符后缀允许的字符描述；实际数字校验由 parser 执行。
pub const ITEM_ID_SUFFIX: &str = "<digits>";
/// 结构化行的通用形式。
pub const STRUCTURED_LINE_PREFIX: &str = "- key: value";
/// 带显式 ID 的任务行形式。
pub const IDENTIFIED_LINE_PREFIX: &str = "- TASK-001 | ...";
/// 任务和验收 statement 的稳定 EARS 模板。
pub const EARS_STATEMENT_TEMPLATE: &str = "WHEN <condition> THE SYSTEM SHALL <observable outcome>";
/// EARS statement 的条件部分前缀。
pub const EARS_WHEN_PREFIX: &str = "WHEN ";
/// EARS statement 的可观察结果部分前缀。
pub const EARS_SHALL_PREFIX: &str = " THE SYSTEM SHALL ";

/// 固定的结构化 section，顺序也是文档规范中的 canonical 顺序。
pub const STRUCTURED_SECTIONS: [&str; 14] = [
    "Identity",
    "Goal",
    "Non Goals",
    "Dependencies",
    // C1 Task 5（REQ-C1-PLAN-01）：existing/create 意图声明（可选 section——
    // 旧 plan 缺席合法，涉及基线外新建写面时由 validator 以
    // intent_undeclared 停等修订，不在 parse 期硬失败）。
    "Plan Intent",
    "Inputs",
    "Outputs",
    "Tasks",
    "Write Policy",
    "Acceptance Criteria",
    "Verification",
    "Handoff Schema",
    "Blockers",
    "Traceability",
];

/// 文档或项目尾部允许自由文本的 section。
pub const FREE_TEXT_SECTIONS: [&str; 2] = ["Notes", "Rationale"];

/// Dependencies section 的结构化依赖 key。
pub const DEPENDENCIES_KEY: &str = "depends_on";

/// 结构化区域允许的 key 白名单。
///
/// 这些 key 与 `field-source-matrix.md` 中由 markdown 提供的字段一一对应；
/// IR 级 `target_repository_id`、trusted command 字段及编译期/runtime 字段
/// 刻意不在此列（C1 Task 5 的 intent target key 用 `intent_target_*` 前缀，
/// 不携带 `target_repository_id` 子串）。
pub const STRUCTURED_KEYS: [&str; 40] = [
    "schema_version",
    "logical_work_item_id",
    "title",
    "kind",
    "summary",
    "non_goals",
    DEPENDENCIES_KEY,
    // C1 Task 5（REQ-C1-PLAN-01）：Plan Intent 的结构化 key。
    "intent",
    "provider_work_item_id",
    "intent_target_kind",
    "intent_target_repo",
    "intent_target_codebase",
    "intent_target_logical_repo",
    "contract_id",
    "provider_logical_work_item_id",
    "required_capabilities",
    "compatibility_policy",
    "capabilities",
    "task_id",
    "statement",
    "requirement_refs",
    "done_when_refs",
    "exclusive_scopes",
    "forbidden_scopes",
    "criterion_id",
    "required_evidence",
    "check_id",
    "command",
    "manual_instruction",
    "required",
    "non_zero_test_execution_required",
    "required_fields",
    "provided_contract_refs",
    "reviewer_check_refs",
    "reason_code",
    "route",
    "target_contract_refs",
    "source_type",
    "source_id",
    "requirement_id",
];

/// 每个结构化 section 允许的 key 词表（section → key 单一事实源）。
///
/// 与 [`STRUCTURED_KEYS`] 同源：映射里出现的每个 key 都取自该白名单，白名单里
/// 的每个 key 至少被一个 section 认领。`contract_id` 由 `Inputs` 与 `Outputs`
/// 共用、`statement` 由 `Tasks` 与 `Acceptance Criteria` 共用——两者在各自
/// section 语义本就相同形状，故在映射中各出现两次（其余 key 恰好一次）。
///
/// 渲染面（SC author prompt 的 `[section_keys]` 段）只从本映射生成，不另立
/// 第二份 key 列表；映射与白名单的一致性由本模块的 `section_keys_*` 用例
/// 双向钉死（防漂移）。
pub const SECTION_KEYS: &[(&str, &[&str])] = &[
    (
        "Identity",
        &["schema_version", "logical_work_item_id", "title", "kind"],
    ),
    ("Goal", &["summary"]),
    ("Non Goals", &["non_goals"]),
    ("Dependencies", &[DEPENDENCIES_KEY]),
    (
        "Plan Intent",
        &[
            "intent",
            "provider_work_item_id",
            "intent_target_kind",
            "intent_target_repo",
            "intent_target_codebase",
            "intent_target_logical_repo",
        ],
    ),
    (
        "Inputs",
        &[
            "contract_id",
            "provider_logical_work_item_id",
            "required_capabilities",
            "compatibility_policy",
        ],
    ),
    ("Outputs", &["contract_id", "capabilities"]),
    (
        "Tasks",
        &["task_id", "statement", "requirement_refs", "done_when_refs"],
    ),
    ("Write Policy", &["exclusive_scopes", "forbidden_scopes"]),
    (
        "Acceptance Criteria",
        &["criterion_id", "statement", "required_evidence"],
    ),
    (
        "Verification",
        &[
            "check_id",
            "command",
            "manual_instruction",
            "required",
            "non_zero_test_execution_required",
        ],
    ),
    (
        "Handoff Schema",
        &[
            "required_fields",
            "provided_contract_refs",
            "reviewer_check_refs",
        ],
    ),
    (
        "Blockers",
        &["reason_code", "route", "target_contract_refs"],
    ),
    (
        "Traceability",
        &["source_type", "source_id", "requirement_id"],
    ),
];

/// `kind` 的现有契约允许值，与 [`crate::product::models::WorkItemKind`] 保持一致。
pub const ALLOWED_ITEM_KINDS: [&str; 7] = [
    crate::product::models::WorkItemKind::Backend.as_str(),
    crate::product::models::WorkItemKind::Frontend.as_str(),
    crate::product::models::WorkItemKind::Integration.as_str(),
    crate::product::models::WorkItemKind::E2e.as_str(),
    crate::product::models::WorkItemKind::Docs.as_str(),
    crate::product::models::WorkItemKind::Infra.as_str(),
    crate::product::models::WorkItemKind::Other.as_str(),
];

/// `compatibility_policy` 的现有契约允许值。
pub const ALLOWED_COMPATIBILITY_POLICIES: [&str; 2] = ["require_all", "require_any"];
/// `intent` 的现有契约允许值,与 [`crate::product::work_item_contract::WorkItemIntent`]
/// 的 lowering 分支(`lower_plan_intent`)逐字对齐;prompt 值域教学与 pi-6
/// 现场教训(`intent: modify` 越界枚举→compile 终态)由此同源注入。
pub const ALLOWED_WORK_ITEM_INTENTS: [&str; 2] = ["create", "existing"];
/// `required_evidence` 的现有契约允许值。
pub const ALLOWED_EVIDENCE_KINDS: [&str; 4] = [
    "source_diff",
    "non_zero_test_execution",
    "manual_check",
    "handoff_field",
];
/// blocker route 的现有契约允许值。
pub const ALLOWED_BLOCKER_ROUTES: [&str; 8] = [
    "coder_rework",
    "verification_retry",
    "plan_repair_current",
    "plan_repair_upstream",
    "subgraph_replan",
    "story_amendment",
    "design_amendment",
    "operational_gate",
];

/// 初始 compiler diagnostic 词汇表；Task 2.1 可在此追加 lowering 诊断码。
pub const DIAGNOSTIC_CODES: [&str; 4] = [
    "missing_section",
    "unknown_structured_key",
    "invalid_work_item_id",
    "invalid_ears",
];

/// 编译器版本元数据；source matrix 的 `ir.compiler_version` 由此常量派生。
pub const WORK_ITEM_PLAN_COMPILER_VERSION: &str = "work_item_plan_compiler/v1";

/// 结构化行的 key/value 分隔符。
pub const KEY_VALUE_SEPARATOR: &str = ": ";
/// 显式 ID 行的分隔符。
pub const IDENTIFIED_LINE_SEPARATOR: &str = " | ";
/// 未知结构化 key 的处理策略元数据。
pub const UNKNOWN_STRUCTURED_KEY_POLICY: &str = "fail_closed";
/// 自由文本 section 的处理策略元数据。
pub const FREE_TEXT_SECTION_POLICY: &str = "allow_free_text";
/// EARS statement 的关键字元数据。
pub const EARS_KEYWORDS: [&str; 2] = ["WHEN", "THE SYSTEM SHALL"];

#[cfg(test)]
mod tests {
    use super::{SECTION_KEYS, STRUCTURED_KEYS, STRUCTURED_SECTIONS};

    /// section→key 映射与 `STRUCTURED_KEYS` 白名单双向一致（防漂移）：
    /// 映射里不得出现白名单外的 key；白名单里的每个 key 至少被一个 section
    /// 认领。跨 section 共用的 key 只有 `contract_id`（Inputs/Outputs）与
    /// `statement`（Tasks/Acceptance Criteria）——两者在各自 section 语义
    /// 本就同形，故各出现两次；其余 key 恰好出现一次。
    #[test]
    fn section_keys_match_structured_key_whitelist() {
        let mapped: Vec<&str> = SECTION_KEYS
            .iter()
            .flat_map(|(_, keys)| keys.iter().copied())
            .collect();
        for key in &mapped {
            assert!(
                STRUCTURED_KEYS.contains(key),
                "section→key 映射不得引入白名单外 key：{key}"
            );
        }
        for key in STRUCTURED_KEYS {
            assert!(
                mapped.contains(&key),
                "白名单 key {key} 必须在 section→key 映射中被至少一个 section 认领"
            );
        }
        let shared = ["contract_id", "statement"];
        for key in STRUCTURED_KEYS {
            let count = mapped
                .iter()
                .filter(|mapped_key| **mapped_key == key)
                .count();
            let expected = if shared.contains(&key) { 2 } else { 1 };
            assert_eq!(
                count, expected,
                "key {key} 在 section→key 映射中的出现次数必须为 {expected}"
            );
        }
    }

    /// section→key 映射覆盖 `STRUCTURED_SECTIONS` 的每个 section，且顺序
    /// 与 canonical 顺序一致（渲染面按此顺序输出）。
    #[test]
    fn section_keys_cover_every_structured_section() {
        let mapped_sections: Vec<&str> = SECTION_KEYS.iter().map(|(section, _)| *section).collect();
        assert_eq!(
            mapped_sections, STRUCTURED_SECTIONS,
            "section→key 映射必须逐个覆盖 STRUCTURED_SECTIONS 且顺序一致"
        );
        for (section, keys) in SECTION_KEYS {
            assert!(
                !keys.is_empty(),
                "section {section} 在映射中显式列出结构化 key（无结构化 key 的 section 才可为空）"
            );
        }
    }
}
