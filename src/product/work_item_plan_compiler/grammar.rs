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
pub const STRUCTURED_SECTIONS: [&str; 13] = [
    "Identity",
    "Goal",
    "Non Goals",
    "Dependencies",
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

/// 结构化区域允许的 key 白名单。
///
/// 这些 key 与 `field-source-matrix.md` 中由 markdown 提供的字段一一对应；
/// `target_repository_id`、trusted command 字段及编译期/runtime 字段刻意不在此列。
pub const STRUCTURED_KEYS: [&str; 34] = [
    "schema_version",
    "logical_work_item_id",
    "title",
    "kind",
    "summary",
    "non_goals",
    "depends_on",
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

/// `compatibility_policy` 的现有契约允许值。
pub const ALLOWED_COMPATIBILITY_POLICIES: [&str; 2] = ["require_all", "require_any"];
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

// 常量别名让调用方可以按语义读取元数据，同时保持单一字面量来源。
pub const WORK_ITEM_PLAN_HEADING: &str = DOCUMENT_HEADING;
pub const WORK_ITEM_HEADING: &str = ITEM_HEADING_PREFIX;
pub const STRUCTURED_SECTION_NAMES: [&str; 13] = STRUCTURED_SECTIONS;
pub const FREE_TEXT_SECTION_NAMES: [&str; 2] = FREE_TEXT_SECTIONS;
pub const EARS_TEMPLATE: &str = EARS_STATEMENT_TEMPLATE;
/// 文档固定一级标题的别名。
pub const PLAN_SECTION: &str = DOCUMENT_HEADING;
/// Work Item heading 的别名。
pub const WORK_ITEM_SECTION_PREFIX: &str = ITEM_HEADING_PREFIX;
/// 结构化行的 key/value 分隔符。
pub const KEY_VALUE_SEPARATOR: &str = ": ";
/// 显式 ID 行的分隔符。
pub const IDENTIFIED_LINE_SEPARATOR: &str = " | ";
/// 未知结构化 key 的处理策略元数据。
pub const UNKNOWN_STRUCTURED_KEY_POLICY: &str = "fail_closed";
/// 自由文本 section 的处理策略元数据。
pub const FREE_TEXT_SECTION_POLICY: &str = "allow_free_text";
/// EARS statement 的关键字元数据。
pub const EARS_KEYWORDS: [&str; 3] = ["WHEN", "THE SYSTEM SHALL", "observable outcome"];
