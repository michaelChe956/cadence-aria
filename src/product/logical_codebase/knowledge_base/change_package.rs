//! 知识库变更包的平台侧机械门禁（Task 3）。
//!
//! 产品裁决：变更包是 `knowledge-base-update` skill 的输入，由用户按
//! Cadence-skills 插件模板撰写；平台只做**机械可判定**的结构门禁——
//! 字段是否填写、枚举是否二选一、七行领域矩阵是否齐全、敏感内容位置
//! 报告、「有变更」分支的起止提交是否可解析、配置分支与 Manifest
//! `scope.configurations.status` 是否一致。语义一致性（Git Diff 与声明
//! 是否相符、DDL 与字段是否冲突、快照指纹、影响链、幂等标识）一律交回
//! skill 门禁；**平台门禁通过 != skill 通过**。
//!
//! 八个稳定码（共享前缀 `knowledge_base_change_package_`）：
//! `path_invalid` / `missing_document` / `field_empty` / `reason_missing`
//! / `domain_matrix_invalid` / `sensitive_content` / `commit_unresolvable`
//! / `scope_mismatch`。码到 HTTP 失败面的登记在 Task 7，本模块只产出码。
//!
//! 硬要求：敏感内容命中的 `detail` 只给位置（文档 + 行号 + 模式名），
//! **绝不回显命中值原文**。

/// 所有稳定码的共享前缀。
const CODE_PREFIX: &str = "knowledge_base_change_package_";

/// 单条门禁违规：`code` 是稳定码；`document` / `field` 定位到变更包内
/// 的具体文档与字段；`detail` 是人读原因（敏感命中时只含位置信息）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePackageViolation {
    /// 稳定码（完整形态为 `CODE_PREFIX` + 后缀，登记见 Task 7）。
    pub code: String,
    /// 变更包内相对路径（如 `code-change.md`）。
    pub document: Option<String>,
    /// 字段名（如 `Merge Request 地址或编号`）。
    pub field: Option<String>,
    /// 人读原因；绝不包含敏感命中值原文。
    pub detail: String,
}

/// 字段级填写状态，供前端编辑器渲染「哪些字段还没填」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePackageFieldStatus {
    pub document: String,
    pub field: String,
    pub filled: bool,
}

/// 门禁判定结果：违规列表 + 全量字段的填写状态。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangePackageLint {
    pub violations: Vec<ChangePackageViolation>,
    pub fields: Vec<ChangePackageFieldStatus>,
}

/// P7 提交解析用的成员仓主 checkout（由调用方从 LC 成员仓收集）。
#[derive(Debug, Clone)]
pub struct MemberRepoPaths {
    pub repo_root: std::path::PathBuf,
}

use std::collections::HashMap;
use std::path::Path;

use super::manifest::KnowledgeBaseManifestSnapshot;

/// scope / 枚举里表示「本领域不适用」的取值。
const NOT_APPLICABLE: &str = "不适用";
/// 模板占位词：`不适用（具体原因）` 的括号内仍是它时视为没填原因。
const NA_PLACEHOLDER_REASON: &str = "具体原因";
/// 「变更状态」的候选（未二选一的占位形态视为未填）。
const CHANGE_STATUSES: [&str; 2] = ["有变更", "无变更"];
/// 配置领域 scope 的候选（Manifest `scope.configurations.status` 同集）。
const CONFIGURATION_SCOPE_STATUSES: [&str; 3] = ["全量", "指定", NOT_APPLICABLE];

/// 领域变更矩阵固定七行（顺序即 change-summary.md 模板顺序）。
const MATRIX_DOMAINS: [&str; 7] = [
    "代码",
    "数据模型",
    "配置",
    "中间件",
    "接口",
    "页面",
    "业务知识",
];

const CHANGE_SUMMARY: &str = "change-summary.md";
const CODE_CHANGE: &str = "code-change.md";
const DATABASE_CHANGE: &str = "database-change.md";
const CONFIGURATION_CHANGE: &str = "configuration-change.md";
const VERIFICATION: &str = "verification.md";
/// 变更包必备的五份文档（skill 契约）。
const DOCUMENTS: [&str; 5] = [
    CHANGE_SUMMARY,
    CODE_CHANGE,
    DATABASE_CHANGE,
    CONFIGURATION_CHANGE,
    VERIFICATION,
];

/// 字段形态：决定取值解析与空值判定方式。
enum FieldKind {
    /// 普通必填文本。
    Text,
    /// 枚举：模板给出候选，仍含多个候选的占位形态视为未填。
    Enum(&'static [&'static str]),
    /// 典型形态是「不适用（具体原因）」或判断依据的文本（判定同 Text）。
    NotApplicable,
    /// 领域变更矩阵行（Markdown 表格取值，字段名为「领域变更矩阵/<领域>」）。
    TableRow,
}

/// 五份文档的固定字段清单（逐字来自插件模板
/// `cadence-init/skills/knowledge-base-update/user-input/change-package/`）：
/// 基本信息 9 + 领域矩阵 7 + 代码 11 + 数据库 11 + 配置 29 + 验证 7 = 74 项。
/// 行式字段按「- 字段：值」取值；TableRow 由领域矩阵解析器取值。
/// configuration-change.md 的「配置差异明细」表属 skill 语义判定，不在清单内。
const FIELD_TABLE: &[(&str, &str, FieldKind)] = &[
    // change-summary.md：基本信息 + 关联说明 + 七行领域矩阵。
    (CHANGE_SUMMARY, "变更标识", FieldKind::Text),
    (CHANGE_SUMMARY, "变更目的", FieldKind::Text),
    (CHANGE_SUMMARY, "目标环境", FieldKind::Text),
    (CHANGE_SUMMARY, "涉及服务", FieldKind::Text),
    (CHANGE_SUMMARY, "业务影响", FieldKind::Text),
    (CHANGE_SUMMARY, "风险", FieldKind::Text),
    (CHANGE_SUMMARY, "需求、缺陷或发布单", FieldKind::Text),
    (CHANGE_SUMMARY, "负责人或确认人", FieldKind::Text),
    (CHANGE_SUMMARY, "补充说明", FieldKind::Text),
    (CHANGE_SUMMARY, "领域变更矩阵/代码", FieldKind::TableRow),
    (CHANGE_SUMMARY, "领域变更矩阵/数据模型", FieldKind::TableRow),
    (CHANGE_SUMMARY, "领域变更矩阵/配置", FieldKind::TableRow),
    (CHANGE_SUMMARY, "领域变更矩阵/中间件", FieldKind::TableRow),
    (CHANGE_SUMMARY, "领域变更矩阵/接口", FieldKind::TableRow),
    (CHANGE_SUMMARY, "领域变更矩阵/页面", FieldKind::TableRow),
    (CHANGE_SUMMARY, "领域变更矩阵/业务知识", FieldKind::TableRow),
    // code-change.md
    (CODE_CHANGE, "变更状态", FieldKind::Enum(&CHANGE_STATUSES)),
    (CODE_CHANGE, "Merge Request 地址或编号", FieldKind::Text),
    (CODE_CHANGE, "源分支", FieldKind::Text),
    (CODE_CHANGE, "目标分支", FieldKind::Text),
    (CODE_CHANGE, "起始提交", FieldKind::Text),
    (CODE_CHANGE, "结束提交", FieldKind::Text),
    (CODE_CHANGE, "修改工程", FieldKind::Text),
    (CODE_CHANGE, "修改文件与符号", FieldKind::Text),
    (CODE_CHANGE, "代码变更说明", FieldKind::Text),
    (CODE_CHANGE, "本地可验证范围", FieldKind::Text),
    (CODE_CHANGE, "无变更判断依据", FieldKind::NotApplicable),
    // database-change.md
    (
        DATABASE_CHANGE,
        "变更状态",
        FieldKind::Enum(&CHANGE_STATUSES),
    ),
    (DATABASE_CHANGE, "数据库 / Schema", FieldKind::Text),
    (DATABASE_CHANGE, "逻辑表", FieldKind::Text),
    (DATABASE_CHANGE, "字段变化", FieldKind::Text),
    (DATABASE_CHANGE, "索引变化", FieldKind::Text),
    (DATABASE_CHANGE, "约束变化", FieldKind::Text),
    (DATABASE_CHANGE, "DDL 或迁移路径", FieldKind::Text),
    (DATABASE_CHANGE, "上线状态", FieldKind::Text),
    (DATABASE_CHANGE, "兼容性", FieldKind::Text),
    (DATABASE_CHANGE, "回滚方式", FieldKind::Text),
    (DATABASE_CHANGE, "无变更判断依据", FieldKind::NotApplicable),
    // configuration-change.md：双快照字段组
    (
        CONFIGURATION_CHANGE,
        "Manifest 配置领域状态",
        FieldKind::Enum(&CONFIGURATION_SCOPE_STATUSES),
    ),
    (CONFIGURATION_CHANGE, "配置领域不适用原因", FieldKind::Text),
    (
        CONFIGURATION_CHANGE,
        "变更状态",
        FieldKind::Enum(&CHANGE_STATUSES),
    ),
    (
        CONFIGURATION_CHANGE,
        "环境（基线与目标必须相同）",
        FieldKind::Text,
    ),
    (CONFIGURATION_CHANGE, "基线快照标识", FieldKind::Text),
    (CONFIGURATION_CHANGE, "基线快照发布批次", FieldKind::Text),
    (
        CONFIGURATION_CHANGE,
        "基线快照生成或获取时间",
        FieldKind::Text,
    ),
    (CONFIGURATION_CHANGE, "基线快照来源类型", FieldKind::Text),
    (CONFIGURATION_CHANGE, "基线快照外部目录", FieldKind::Text),
    (CONFIGURATION_CHANGE, "基线快照指纹", FieldKind::Text),
    (CONFIGURATION_CHANGE, "基线范围摘要", FieldKind::Text),
    (
        CONFIGURATION_CHANGE,
        "基线纳入文件数量或文件清单摘要",
        FieldKind::Text,
    ),
    (CONFIGURATION_CHANGE, "基线服务摘要", FieldKind::Text),
    (CONFIGURATION_CHANGE, "基线文件规则摘要", FieldKind::Text),
    (CONFIGURATION_CHANGE, "目标快照标识", FieldKind::Text),
    (CONFIGURATION_CHANGE, "目标快照发布批次", FieldKind::Text),
    (
        CONFIGURATION_CHANGE,
        "目标快照生成或获取时间",
        FieldKind::Text,
    ),
    (CONFIGURATION_CHANGE, "目标快照来源类型", FieldKind::Text),
    (CONFIGURATION_CHANGE, "目标快照外部目录", FieldKind::Text),
    (CONFIGURATION_CHANGE, "目标快照指纹", FieldKind::Text),
    (CONFIGURATION_CHANGE, "目标范围摘要", FieldKind::Text),
    (
        CONFIGURATION_CHANGE,
        "目标纳入文件数量或文件清单摘要",
        FieldKind::Text,
    ),
    (CONFIGURATION_CHANGE, "目标服务摘要", FieldKind::Text),
    (CONFIGURATION_CHANGE, "目标文件规则摘要", FieldKind::Text),
    (CONFIGURATION_CHANGE, "纳入文件范围", FieldKind::Text),
    (CONFIGURATION_CHANGE, "涉及服务", FieldKind::Text),
    (CONFIGURATION_CHANGE, "配置组", FieldKind::Text),
    (CONFIGURATION_CHANGE, "已知差异", FieldKind::Text),
    (
        CONFIGURATION_CHANGE,
        "无变更判断依据",
        FieldKind::NotApplicable,
    ),
    // verification.md：记录项组
    (VERIFICATION, "已执行测试", FieldKind::Text),
    (VERIFICATION, "发布验证", FieldKind::Text),
    (VERIFICATION, "数据兼容验证", FieldKind::Text),
    (VERIFICATION, "配置生效验证", FieldKind::Text),
    (VERIFICATION, "回滚方式", FieldKind::Text),
    (VERIFICATION, "未验证项目", FieldKind::Text),
    (VERIFICATION, "风险", FieldKind::Text),
];

/// 对 `package_dir` 下的五文档变更包做平台侧机械门禁，返回违规列表与
/// 全量字段的填写状态（供编辑器渲染「哪些字段还没填」）。
///
/// - `manifest`：Task 2 的 Manifest 快照，用于配置分支一致性判定；
/// - `members`：提交解析用的成员仓主 checkout，任一仓可解析即通过。
///
/// 门禁只判「结构与取值」；平台门禁通过 != skill 门禁通过。
pub fn lint_change_package(
    package_dir: &Path,
    manifest: &KnowledgeBaseManifestSnapshot,
    members: &[MemberRepoPaths],
) -> ChangePackageLint {
    let mut violations = Vec::new();
    let mut fields = Vec::new();

    // P1：包路径形态（目录位置 / 目录名前缀）。
    lint_package_path(package_dir, &mut violations);

    // P2：五份文档必须存在且非空。
    let mut contents: HashMap<&'static str, String> = HashMap::new();
    for document in DOCUMENTS {
        match std::fs::read_to_string(package_dir.join(document)) {
            Ok(text) if !text.trim().is_empty() => {
                contents.insert(document, text);
            }
            Ok(_) => violations.push(mk(
                "missing_document",
                Some(document),
                None,
                "文档存在但内容为空".to_string(),
            )),
            Err(_) => violations.push(mk(
                "missing_document",
                Some(document),
                None,
                "文档缺失或不可读".to_string(),
            )),
        }
    }

    // 行式字段与领域矩阵取值。
    let values = parse_line_fields(&contents);
    let matrix = parse_domain_matrix(
        contents
            .get(CHANGE_SUMMARY)
            .map(String::as_str)
            .unwrap_or(""),
    );

    // P1 的标识项：变更标识不得携带路径分隔符或 `..`。
    if let Some(id) = values
        .get(CHANGE_SUMMARY)
        .and_then(|per_document| per_document.get("变更标识"))
    {
        let id = id.trim();
        if id.contains('/') || id.contains('\\') || id.contains("..") {
            violations.push(mk(
                "path_invalid",
                Some(CHANGE_SUMMARY),
                Some("变更标识"),
                "变更标识含路径分隔符或 `..`，存在路径逃逸风险".to_string(),
            ));
        }
    }

    // P3/P4：逐字段判定空值、枚举占位与「不适用」缺原因，并产出字段状态。
    for (document, field, kind) in FIELD_TABLE {
        if matches!(kind, FieldKind::TableRow) {
            let domain = field
                .strip_prefix("领域变更矩阵/")
                .expect("TableRow 字段名带领域前缀");
            let filled = matrix
                .get(domain)
                .map(|(status, basis)| {
                    CHANGE_STATUSES.contains(&status.as_str())
                        && (status == CHANGE_STATUSES[0] || !basis.trim().is_empty())
                })
                .unwrap_or(false);
            fields.push(ChangePackageFieldStatus {
                document: document.to_string(),
                field: field.to_string(),
                filled,
            });
            continue;
        }
        if !contents.contains_key(document) {
            // 文档缺失：字段级只报未填状态，违规由 missing_document 承担。
            fields.push(ChangePackageFieldStatus {
                document: document.to_string(),
                field: field.to_string(),
                filled: false,
            });
            continue;
        }
        let value = values
            .get(document)
            .and_then(|per_document| per_document.get(*field));
        let filled = lint_scalar_field(document, field, kind, value, &mut violations);
        fields.push(ChangePackageFieldStatus {
            document: document.to_string(),
            field: field.to_string(),
            filled,
        });
    }

    // P5：七行领域矩阵。
    lint_domain_matrix(&matrix, &mut violations);

    // P6：敏感内容扫描（只报位置，不回显命中值）。
    for document in DOCUMENTS {
        if let Some(text) = contents.get(document) {
            scan_sensitive(document, text, &mut violations);
        }
    }

    // P7：「有变更」分支的起止提交必须可解析。
    lint_commits(&values, members, &mut violations);

    // P8/P9：配置分支与 Manifest scope.configurations.status 一致性。
    lint_configuration_scope(&values, manifest, &mut violations);

    ChangePackageLint { violations, fields }
}

/// 组装一条违规（后缀拼进稳定码前缀）。
fn mk(
    code_suffix: &str,
    document: Option<&str>,
    field: Option<&str>,
    detail: String,
) -> ChangePackageViolation {
    ChangePackageViolation {
        code: format!("{CODE_PREFIX}{code_suffix}"),
        document: document.map(str::to_string),
        field: field.map(str::to_string),
        detail,
    }
}

/// P1：目录必须在 `user-input/updates/` 之下且目录名以 `CHANGE-` 开头。
fn lint_package_path(package_dir: &Path, violations: &mut Vec<ChangePackageViolation>) {
    let components: Vec<String> = package_dir
        .iter()
        .map(|component| component.to_string_lossy().into_owned())
        .collect();
    let under_updates = components.windows(2).enumerate().any(|(at, window)| {
        window[0] == "user-input" && window[1] == "updates" && at + 2 < components.len()
    });
    if !under_updates {
        violations.push(mk(
            "path_invalid",
            None,
            None,
            format!("目录 {} 不在 user-input/updates/ 下", package_dir.display()),
        ));
    }
    let dir_name = package_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !dir_name.starts_with("CHANGE-") {
        violations.push(mk(
            "path_invalid",
            None,
            None,
            format!("目录名「{dir_name}」不以 CHANGE- 开头"),
        ));
    }
}

/// P3/P4 的单字段判定，返回该字段是否视为「已填」。
fn lint_scalar_field(
    document: &'static str,
    field: &'static str,
    kind: &FieldKind,
    value: Option<&String>,
    violations: &mut Vec<ChangePackageViolation>,
) -> bool {
    let Some(raw) = value else {
        violations.push(mk(
            "field_empty",
            Some(document),
            Some(field),
            "必填字段缺失（模板行不存在）".to_string(),
        ));
        return false;
    };
    let value = raw.trim();
    if value.is_empty() {
        violations.push(mk(
            "field_empty",
            Some(document),
            Some(field),
            "必填字段为空".to_string(),
        ));
        return false;
    }
    if let FieldKind::Enum(options) = kind {
        let hits = options
            .iter()
            .filter(|option| value.contains(**option))
            .count();
        if hits >= 2 {
            violations.push(mk(
                "field_empty",
                Some(document),
                Some(field),
                "枚举字段仍含多个候选，未二选一".to_string(),
            ));
            return false;
        }
        // 恰有 0 个候选说明填了表外取值：机械门禁只看「有内容且已二选一」，
        // 取值合法性交回 skill 门禁。
        return true;
    }
    if classify_not_applicable(value) == NotApplicableFill::ReasonMissing {
        violations.push(mk(
            "reason_missing",
            Some(document),
            Some(field),
            "填写「不适用」但括号内没有具体原因".to_string(),
        ));
        return false;
    }
    true
}

/// P5：七行领域矩阵必须齐全、状态二选一、无变更行带判断依据。
fn lint_domain_matrix(
    matrix: &HashMap<&'static str, (String, String)>,
    violations: &mut Vec<ChangePackageViolation>,
) {
    for domain in MATRIX_DOMAINS {
        let field = format!("领域变更矩阵/{domain}");
        match matrix.get(domain) {
            None => violations.push(mk(
                "domain_matrix_invalid",
                Some(CHANGE_SUMMARY),
                Some(&field),
                format!("领域变更矩阵缺少「{domain}」行"),
            )),
            Some((status, basis)) => {
                if !CHANGE_STATUSES.contains(&status.as_str()) {
                    violations.push(mk(
                        "domain_matrix_invalid",
                        Some(CHANGE_SUMMARY),
                        Some(&field),
                        format!("「{domain}」行的变更状态不是 有变更/无变更"),
                    ));
                } else if status == CHANGE_STATUSES[1] && basis.trim().is_empty() {
                    violations.push(mk(
                        "domain_matrix_invalid",
                        Some(CHANGE_SUMMARY),
                        Some(&field),
                        format!("「{domain}」行为无变更但缺少判断依据"),
                    ));
                }
            }
        }
    }
}

/// P7：「有变更」分支的起止提交在成员仓的解析校验。
fn lint_commits(
    values: &HashMap<&'static str, HashMap<&'static str, String>>,
    members: &[MemberRepoPaths],
    violations: &mut Vec<ChangePackageViolation>,
) {
    let Some(status) = values
        .get(CODE_CHANGE)
        .and_then(|per_document| per_document.get("变更状态"))
    else {
        return;
    };
    if single_choice(status, &CHANGE_STATUSES) != Some(CHANGE_STATUSES[0]) {
        return;
    }
    for field in ["起始提交", "结束提交"] {
        let Some(revision) = values
            .get(CODE_CHANGE)
            .and_then(|per_document| per_document.get(field))
        else {
            continue;
        };
        let revision = revision.trim();
        if revision.is_empty() {
            continue; // 空值由 field_empty 报告
        }
        if !commit_resolves_in_any_member(revision, members) {
            violations.push(mk(
                "commit_unresolvable",
                Some(CODE_CHANGE),
                Some(field),
                format!("提交「{revision}」在所有成员仓都无法解析"),
            ));
        }
    }
}

/// P8/P9：configuration-change.md 与 Manifest `scope.configurations.status`
/// 的一致性：声明一致、不适用分支的状态/原因相符、快照字段组与分支匹配。
fn lint_configuration_scope(
    values: &HashMap<&'static str, HashMap<&'static str, String>>,
    manifest: &KnowledgeBaseManifestSnapshot,
    violations: &mut Vec<ChangePackageViolation>,
) {
    let Some(actual) = manifest.configurations_scope.as_deref() else {
        return; // Manifest 未声明配置领域：结构问题由 Task 2 不变量管
    };
    let declared = values
        .get(CONFIGURATION_CHANGE)
        .and_then(|per_document| per_document.get("Manifest 配置领域状态"));
    if let Some(declared) = declared
        && let Some(choice) = single_choice(declared, &CONFIGURATION_SCOPE_STATUSES)
        && choice != actual
    {
        violations.push(mk(
            "scope_mismatch",
            Some(CONFIGURATION_CHANGE),
            Some("Manifest 配置领域状态"),
            format!(
                "变更包声明为「{choice}」，Manifest scope.configurations.status 为「{actual}」"
            ),
        ));
    }
    if actual == NOT_APPLICABLE {
        let status = values
            .get(CONFIGURATION_CHANGE)
            .and_then(|per_document| per_document.get("变更状态"));
        if let Some(status) = status
            && single_choice(status, &CHANGE_STATUSES) != Some(CHANGE_STATUSES[1])
        {
            violations.push(mk(
                "scope_mismatch",
                Some(CONFIGURATION_CHANGE),
                Some("变更状态"),
                "Manifest 配置领域为「不适用」时变更状态必须为「无变更」".to_string(),
            ));
        }
        let reason = values
            .get(CONFIGURATION_CHANGE)
            .and_then(|per_document| per_document.get("配置领域不适用原因"));
        let reason_missing = match reason {
            Some(reason) => {
                reason.trim().is_empty()
                    || classify_not_applicable(reason) == NotApplicableFill::ReasonMissing
            }
            None => true,
        };
        if reason_missing {
            violations.push(mk(
                "scope_mismatch",
                Some(CONFIGURATION_CHANGE),
                Some("配置领域不适用原因"),
                "Manifest 配置领域为「不适用」时必须给出具体的不适用原因".to_string(),
            ));
        }
    }
    // 快照字段组与 Manifest 分支的双向一致性：「不适用」分支的快照字段
    // 应填「不适用（原因）」；「全量/指定」分支不得填「不适用」。
    for (document, field, _) in FIELD_TABLE {
        if *document != CONFIGURATION_CHANGE || !is_snapshot_field(field) {
            continue;
        }
        let Some(value) = values
            .get(CONFIGURATION_CHANGE)
            .and_then(|per_document| per_document.get(*field))
        else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue; // 空值由 field_empty 报告
        }
        let not_applicable = classify_not_applicable(value) != NotApplicableFill::NotUsed;
        if actual == NOT_APPLICABLE && !not_applicable {
            violations.push(mk(
                "scope_mismatch",
                Some(CONFIGURATION_CHANGE),
                Some(field),
                "Manifest 配置领域为「不适用」，快照字段应填写不适用（原因）".to_string(),
            ));
        } else if actual != NOT_APPLICABLE && not_applicable {
            violations.push(mk(
                "scope_mismatch",
                Some(CONFIGURATION_CHANGE),
                Some(field),
                format!("Manifest 配置领域为「{actual}」，快照字段不得填写不适用"),
            ));
        }
    }
}

/// P6：保守敏感内容扫描。模板自带的 `>` 说明行（警示文案本身就含
/// Token/AccessKey/Secret 等词）不是用户填写内容，跳过以免每个按模板
/// 撰写的包都误报。命中只报「文档 + 行号 + 模式名」，绝不回显原文。
fn scan_sensitive(
    document: &'static str,
    text: &str,
    violations: &mut Vec<ChangePackageViolation>,
) {
    for (offset, line) in text.lines().enumerate() {
        if line.trim_start().starts_with('>') {
            continue;
        }
        let line_number = offset + 1;
        let lower = line.to_ascii_lowercase();
        if let Some(label) = sensitive_label(&lower) {
            violations.push(mk(
                "sensitive_content",
                Some(document),
                None,
                format!("第 {line_number} 行疑似包含敏感内容（{label}），为避免泄露不回显原文"),
            ));
        }
    }
}

/// 保守敏感模式集合（命中第一个即返回；标签本身不含任何被扫关键字）。
fn sensitive_label(lower: &str) -> Option<&'static str> {
    if lower.contains("password") || lower.contains("passwd") {
        return Some("password/passwd 关键字");
    }
    if lower.contains("secret") {
        return Some("secret 关键字");
    }
    if lower.contains("token") {
        return Some("token 关键字");
    }
    if lower.contains("accesskey")
        || lower.contains("access_key")
        || lower.contains("access-key")
        || lower.contains("access key")
    {
        return Some("access key 关键字");
    }
    if lower.contains("private key")
        || lower.contains("private_key")
        || lower.contains("private-key")
        || lower.contains("privatekey")
    {
        return Some("private key 关键字");
    }
    if lower.contains("jdbc:") {
        return Some("jdbc 连接串");
    }
    if lower.contains("mysql://")
        || lower.contains("postgres://")
        || lower.contains("postgresql://")
        || lower.contains("redis://")
    {
        return Some("数据库连接串");
    }
    if has_internal_http_host(lower) {
        return Some("内网地址 URL");
    }
    None
}

/// `http(s)://` 后紧跟内网形态主机（localhost / 127.0.0.1 / 10.* /
/// 192.168.* / 172.16-31.*）即命中。
fn has_internal_http_host(lower: &str) -> bool {
    ["http://", "https://"].iter().any(|scheme| {
        let mut from = 0;
        while let Some(at) = lower[from..].find(scheme) {
            let after = &lower[from + at + scheme.len()..];
            if points_to_internal_host(after) {
                return true;
            }
            from += at + scheme.len();
        }
        false
    })
}

/// 判断 URL scheme 之后的主机段是否内网形态。
fn points_to_internal_host(after_scheme: &str) -> bool {
    let end = after_scheme
        .find(|c: char| {
            c == '/' || c == ':' || c == '?' || c == '"' || c == '\'' || c.is_whitespace()
        })
        .unwrap_or(after_scheme.len());
    let host = &after_scheme[..end];
    if host == "localhost" || host == "127.0.0.1" {
        return true;
    }
    if host.starts_with("10.") || host.starts_with("192.168.") {
        return true;
    }
    match host.strip_prefix("172.") {
        Some(rest) => match rest.find('.') {
            Some(dot) => rest[..dot]
                .parse::<u16>()
                .map(|octet| (16..=31).contains(&octet))
                .unwrap_or(false),
            None => false,
        },
        None => false,
    }
}

/// 解析各文档的「- 字段：值」行，取 FIELD_TABLE 已知字段。
fn parse_line_fields(
    contents: &HashMap<&'static str, String>,
) -> HashMap<&'static str, HashMap<&'static str, String>> {
    let mut values: HashMap<&'static str, HashMap<&'static str, String>> = HashMap::new();
    for (document, text) in contents {
        for line in text.lines() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed.strip_prefix("- ") else {
                continue;
            };
            let Some((name, value)) = split_field_colon(rest) else {
                continue;
            };
            // 容忍「- 」后、字段名前的多余空白（如「-  变更目的：x」），
            // 避免纯空白差异把合法字段行判成「模板行不存在」。
            let name = name.trim();
            let Some(spec) = FIELD_TABLE.iter().find(|(doc, field, kind)| {
                doc == document && field == &name && !matches!(kind, FieldKind::TableRow)
            }) else {
                continue;
            };
            values
                .entry(*document)
                .or_default()
                .insert(spec.1, value.trim().to_string());
        }
    }
    values
}

/// 在「- 字段：值」的键段取第一个全角（或半角）冒号切分键值。
fn split_field_colon(rest: &str) -> Option<(&str, &str)> {
    if let Some(at) = rest.find('：') {
        Some((&rest[..at], &rest[at + '：'.len_utf8()..]))
    } else {
        let at = rest.find(':')?;
        Some((&rest[..at], &rest[at + 1..]))
    }
}

/// 解析 change-summary.md 的领域矩阵：领域 →（变更状态, 摘要或判断依据）。
fn parse_domain_matrix(text: &str) -> HashMap<&'static str, (String, String)> {
    let mut rows: HashMap<&'static str, (String, String)> = HashMap::new();
    for line in text.lines() {
        let Some(cells) = table_cells(line) else {
            continue;
        };
        if cells.len() < 2 || cells[0].is_empty() {
            continue;
        }
        if let Some(&domain) = MATRIX_DOMAINS.iter().find(|domain| **domain == cells[0]) {
            rows.insert(
                domain,
                (
                    cells.get(1).cloned().unwrap_or_default(),
                    cells.get(2).cloned().unwrap_or_default(),
                ),
            );
        }
    }
    rows
}

/// 取 Markdown 表格行的单元格（去首尾竖线、逐格 trim；空单元格保留）。
fn table_cells(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if trimmed.len() < 2 || !trimmed.starts_with('|') || !trimmed.ends_with('|') {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    Some(
        inner
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect(),
    )
}

/// 枚举值里「恰好出现一个」的候选（多个 = 未二选一，零个 = 表外取值）。
fn single_choice<'a>(value: &str, options: &'a [&'a str]) -> Option<&'a str> {
    let mut hits = options
        .iter()
        .copied()
        .filter(|option| value.contains(*option));
    let first = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    Some(first)
}

/// 「不适用」取值的填写形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotApplicableFill {
    /// 取值不是「不适用」形态。
    NotUsed,
    /// 「不适用（原因）」且括号内是具体原因。
    ReasonFilled,
    /// 裸「不适用」，或括号内为空 / 仍是占位词「具体原因」。
    ReasonMissing,
}

/// 判定「不适用」取值是否带具体原因：模板要求 `不适用（具体原因）`，
/// 括号内为空或仍是占位词「具体原因」都视为未填原因；裸「不适用」与
/// 未闭合括号同理。`不适用` 后随**非括号**文字（如「不适用于存量数据」
/// 「不适用：无数据变更」）是自然语言写法而非模板形态，返回 NotUsed，
/// 不按不适用取值参与缺原因与快照形态判定。
fn classify_not_applicable(value: &str) -> NotApplicableFill {
    let trimmed = value.trim();
    let Some(rest) = trimmed.strip_prefix(NOT_APPLICABLE) else {
        return NotApplicableFill::NotUsed;
    };
    let inner = if let Some(body) = rest.strip_prefix('（') {
        body.find('）').map(|end| &body[..end])
    } else if let Some(body) = rest.strip_prefix('(') {
        body.find(')').map(|end| &body[..end])
    } else if !rest.is_empty() {
        // 「不适用」后随非括号文字：自然语言写法，不是「不适用（原因）」
        // 模板形态，不判为不适用取值（避免误报缺原因 / 快照形态不符）。
        return NotApplicableFill::NotUsed;
    } else {
        None
    };
    match inner {
        Some(inner) => {
            let inner = inner.trim();
            if inner.is_empty() || inner == NA_PLACEHOLDER_REASON {
                NotApplicableFill::ReasonMissing
            } else {
                NotApplicableFill::ReasonFilled
            }
        }
        None => NotApplicableFill::ReasonMissing,
    }
}

/// 快照字段组：环境 + 全部「基线…」「目标…」字段（模板「双快照字段组」）。
fn is_snapshot_field(field: &str) -> bool {
    field.starts_with("基线") || field.starts_with("目标") || field == "环境（基线与目标必须相同）"
}
/// P7：对每个成员仓跑 `git rev-parse --verify <提交>^{commit}`；
/// 任一仓可解析即通过，全部失败（含成员列表为空）才判失败。
fn commit_resolves_in_any_member(revision: &str, members: &[MemberRepoPaths]) -> bool {
    let spec = format!("{revision}^{{commit}}");
    members.iter().any(|member| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&member.repo_root)
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(&spec)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    })
}

// 测试模块按仓库惯例拆入 `.inc.rs`（同模块命名空间，符号路径与可见性不变），
// 保持本文件低于 large_file_guard 的 1200 行上限：
// - change_package_tests.inc.rs：tests 模块（五文档夹具与 25 条门禁用例）。
include!("change_package_tests.inc.rs");
