//! 失败面文案的**唯一**来源（REQ-BOOT-08 / REQ-LAYOUT-02 / REQ-BOOT-08 第三出口）。
//!
//! 聚合初始化（durable 记录）、聚合索引（durable warning 投影）、登记预检
//! HTTP 错误与聚合索引同步 HTTP 错误，四条链路都把「只读事实」适配成
//! [`FailureInput`] 后交给 [`describe`]：投影层、DTO、通知与布局引导侧
//! **不得**各自维护同义文案，也不得把中文就地写在 handler/前端里。
//!
//! 成品 [`FailureCopy`] 的五个字段全部必填、均非 `Option`：
//! - `title_zh`（问题）：中文主标题，点名可确认对象，不把机器码当标题；
//! - `reason_zh`（原因）：布局码给出具体规则依据；无法判定的场合诚实写
//!   「原因尚未确定」，**不允许**缺字段或空串；
//! - `action_zh`（怎么做）：安装/移动/权限/重试等可执行下一步；
//! - `triage`（分诊）：`Environment` / `ProductDefect` / `RetryOnly`；
//! - `machine_line`（机器码）：原始输入码的**次要**展示位。
//!
//! 码表外真正未知的码由 [`neutral_fallback`] 给出中性三段（不 panic、
//! 不用英文原文当中文主标题）。已知布局码因漏登记或字段缺失被当成未知码
//! **属于实现缺陷**：本模块把它们逐一登记在 [`TABLE`] 里，并用
//! [`LAYOUT_RULES`] 维护「六条布局规则 → 七个对外稳定码」的归属。

use serde::{Deserialize, Serialize};

use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationErrorRecord;

/// 失败分诊：用户环境问题 / 产品缺陷 / 只能重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureTriage {
    Environment,
    ProductDefect,
    RetryOnly,
}

impl FailureTriage {
    /// 对外序列化值（Rust DTO / HTTP `details.copy` / TS 三值联合类型同构）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::ProductDefect => "product_defect",
            Self::RetryOnly => "retry_only",
        }
    }

    /// 前端分诊标签（中文只此一份，前端不自建映射表）。
    pub const fn label_zh(self) -> &'static str {
        match self {
            Self::Environment => "环境问题（你这边可以修）",
            Self::ProductDefect => "疑似产品缺陷（可重试并反馈）",
            Self::RetryOnly => "可直接重试",
        }
    }
}

/// 三段成品：问题 / 原因 / 怎么做 / 分诊 / 机器码次要行。
///
/// 五字段全部必填；`reason_zh` 不是 `Option`（原因不可判定用中性的
/// 「原因尚未确定」承载，而不是缺字段）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureCopy {
    pub title_zh: String,
    pub reason_zh: String,
    pub action_zh: String,
    pub triage: FailureTriage,
    pub machine_line: String,
}

impl FailureCopy {
    /// 布局失败专用的「具体规则依据」判据：reason 里必须出现规则文本。
    pub fn follows_layout_rule(&self) -> bool {
        LAYOUT_RULES
            .iter()
            .any(|rule| self.reason_zh.contains(rule.rule_zh))
    }
}

/// 借用式输入：共享**事实**，不共享业务记录。
///
/// 只有 `reason_code` 必填；其余字段按调用方已有事实填写，缺失即 `None`
/// （不编造）。`action` 是原始诊断文案，**不是**中文动作来源。
#[derive(Debug, Clone, Copy)]
pub struct FailureInput<'a> {
    pub reason_code: &'a str,
    pub stage: Option<&'a str>,
    pub cause: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub detail: Option<&'a str>,
    pub action: Option<&'a str>,
    pub subject: Option<&'a str>,
}

impl<'a> FailureInput<'a> {
    pub fn new(reason_code: &'a str) -> Self {
        Self {
            reason_code,
            stage: None,
            cause: None,
            provider: None,
            detail: None,
            action: None,
            subject: None,
        }
    }

    pub fn with_stage(mut self, stage: &'a str) -> Self {
        self.stage = Some(stage);
        self
    }

    pub fn with_cause(mut self, cause: &'a str) -> Self {
        self.cause = Some(cause);
        self
    }

    pub fn with_provider(mut self, provider: &'a str) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn with_detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn with_action(mut self, action: &'a str) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_subject(mut self, subject: &'a str) -> Self {
        self.subject = Some(subject);
        self
    }
}

impl<'a> From<&'a AggregateInitializationErrorRecord> for FailureInput<'a> {
    fn from(record: &'a AggregateInitializationErrorRecord) -> Self {
        Self {
            reason_code: &record.reason_code,
            stage: Some(record.stage.as_str()),
            cause: record.cause.as_deref(),
            provider: record.provider.as_deref(),
            detail: record.stderr_summary.as_deref(),
            action: Some(record.action.as_str()),
            subject: None,
        }
    }
}

include!("failure_copy_table.inc.rs");

/// 模板占位符替换。缺省值只做「不编造事实」的中性填充，不引入新结论。
fn fill(template: &str, input: &FailureInput<'_>) -> String {
    if !template.contains('{') {
        return template.to_string();
    }
    let member = input
        .subject
        .map(|subject| format!("「{subject}」"))
        .unwrap_or_default();
    let subject = input.subject.unwrap_or("该对象");
    let stage = input.stage.unwrap_or("当前步骤");
    let provider = input.provider.unwrap_or("该 provider");
    let detail = input
        .detail
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
        .unwrap_or("");
    let mut filled = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        filled.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            filled.push_str(&rest[start..]);
            return filled;
        };
        match &after[..end] {
            "member" => filled.push_str(&member),
            "subject" => filled.push_str(subject),
            "stage" => filled.push_str(stage),
            "provider" => filled.push_str(provider),
            "detail" => filled.push_str(detail),
            other => {
                filled.push('{');
                filled.push_str(other);
                filled.push('}');
            }
        }
        rest = &after[end + 1..];
    }
    filled.push_str(rest);
    filled
}

/// 码表外真正未知的码：中性五字段（不 panic、不用英文原文当主标题）。
fn neutral_fallback(input: &FailureInput<'_>) -> FailureCopy {
    let title_zh = match input.subject {
        Some(subject) => format!("{subject} 的本次操作未完成"),
        None => "本次操作未完成".to_string(),
    };
    FailureCopy {
        title_zh,
        reason_zh: "当前无法确定具体原因，请查看下方诊断信息。".to_string(),
        action_zh: "请重试；连续两次仍失败时，提交问题并附上机器码与下方诊断信息。".to_string(),
        triage: FailureTriage::ProductDefect,
        machine_line: input.reason_code.to_string(),
    }
}

/// 成因不可判定（`cause == "unknown"`）时的诚实标注：给候选清单，不给结论。
fn mark_undefined_cause(copy: &mut FailureCopy) {
    copy.title_zh = format!("{}（无法确定具体成因）", copy.title_zh);
    copy.reason_zh =
        "已确认这一步失败，但现有诊断里没有任何一条判据能对上具体成因；下面列的是候选，不是结论。"
            .to_string();
    copy.action_zh = "按顺序排查候选：provider 未授权（provider_unauthorized）、provider 命令缺失（provider_command_missing）、skills 命令链接缺失（skills_commands_missing）；也可以先直接「重试」并附上诊断。".to_string();
    copy.triage = FailureTriage::ProductDefect;
}

/// 已确认的 cause 分流：动作/分诊按**已确认的事实**改写，不按码名编造。
fn apply_confirmed_cause(copy: &mut FailureCopy, key: &str, input: &FailureInput<'_>) {
    let Some(cause) = input.cause else {
        return;
    };
    if cause == "unknown" {
        mark_undefined_cause(copy);
        return;
    }
    match key {
        "aggregate_machine_skills_failed" => {
            if cause.starts_with("skills_unavailable") {
                copy.action_zh =
                    "检查网络与上游仓可达性（machine skills 源取不到），恢复后点「重试」。"
                        .to_string();
            } else if cause.starts_with("skills_sync_failed") {
                copy.action_zh =
                    "点「重试」重新同步 machine skills；持续失败时检查上游仓权限。".to_string();
            } else if cause.starts_with("skills_digest_failed") {
                copy.action_zh =
                    "点「重试」重新准备 machine skills；持续失败时检查 ~/.claude/skills 是否被改动。"
                        .to_string();
            } else if cause.starts_with("skills_update_failed") {
                // 与 skills_unavailable*（源取不到）区分：这里是「拿得到但更新/落盘
                // 失败」，所以动作是重试 + 查上游可达性与权限。
                copy.action_zh =
                    "点「重试」重新准备 machine skills；持续失败时检查上游仓可达性与权限。"
                        .to_string();
            }
        }
        "aggregate_preflight_failed" | "aggregate_aggregate_preflight_failed"
            if cause.contains("aggregate_root_not_writable") =>
        {
            copy.action_zh = "修复聚合根目录的写权限后点「重试」。".to_string();
        }
        _ => {}
    }
    // 布局码：同码 detail/cause 若明确是存储/检查失败，按已确认事实转为产品缺陷，
    // 不能因为码名是布局码就编造成用户布局违规。
    if layout_rule_of(key).is_some() && cause.starts_with("store_") {
        copy.reason_zh = format!(
            "{}（补充：这次是检查过程本身失败，不是你目录摆错了。）",
            copy.reason_zh
        );
        copy.triage = FailureTriage::ProductDefect;
        copy.action_zh = "点「重试」；连续两次仍失败时，附机器码与诊断提交问题。".to_string();
    }
}

/// 唯一入口：只读事实 → 五字段成品。
pub fn describe(input: FailureInput<'_>) -> FailureCopy {
    let key = canonical_key(input.reason_code);
    let mut copy = match table_entry(key) {
        Some((_, title, reason, action, triage)) => FailureCopy {
            title_zh: fill(title, &input),
            reason_zh: fill(reason, &input),
            action_zh: fill(action, &input),
            triage,
            machine_line: input.reason_code.to_string(),
        },
        None => neutral_fallback(&input),
    };
    apply_confirmed_cause(&mut copy, key, &input);
    copy.machine_line = input.reason_code.to_string();
    copy
}

/// `BootstrapFailure` / `AggregateIndexError` 形态（无 `action` 字段）的入口：
/// 只是 `describe` 的输入适配，**不另持表、不另一套兜底**。
pub fn describe_code(code: &str, machine_line: Option<&str>) -> FailureCopy {
    let mut input = FailureInput::new(code);
    if let Some(machine_line) = machine_line {
        input = input.with_detail(machine_line);
    }
    describe(input)
}

/// durable 失败记录的「原始诊断」段：`stderr_summary` 优先，其次 `action`，
/// 都没有时给中文兜底串。
///
/// 投影层与 HTTP DTO 共用这一份构造，**绝不**出现旧实现里那种英文占位
/// （REQ-BOOT-08 的既有老记录场景）。
pub fn record_diagnostic(record: &AggregateInitializationErrorRecord) -> String {
    if let Some(summary) = record
        .stderr_summary
        .as_deref()
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
    {
        return format!("stage {}: {summary}", record.stage);
    }
    let action = record.action.trim();
    if !action.is_empty() {
        return format!("stage {}: {action}", record.stage);
    }
    "操作在完成前中断，服务端没有保留更详细的诊断输出。".to_string()
}

/// 从既有英文封套里**定向**提取可确认对象（成员目录名 / 占用方 LC）。
///
/// 只在已知码的已知句式里提取；识别不出一律 `None`（不编造事实），
/// 原始整句仍原样进折叠区诊断。
pub fn subject_from_message(code: &str, message: &str) -> Option<String> {
    let extracted = match canonical_key(code) {
        "aggregate_root_member_outside_root"
        | "aggregate_root_member_symlink_escape"
        | "aggregate_root_nested_worktree" => {
            extract_between(message, "member path ", " resolves to")
        }
        "aggregate_root_is_git" => {
            extract_between(message, "aggregate root ", " is a Git repository")
        }
        "aggregate_root_overlap" => {
            extract_between(message, "overlaps logical codebase ", " root ")
        }
        _ => None,
    }?;
    // 路径形态的对象只点名最后一段（完整路径留在诊断里）。
    let named = extracted
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(extracted.as_str());
    (!named.is_empty()).then(|| named.to_string())
}

fn extract_between(message: &str, prefix: &str, suffix: &str) -> Option<String> {
    let start = message.find(prefix)? + prefix.len();
    let rest = &message[start..];
    let end = rest.find(suffix)?;
    let extracted = rest[..end].trim();
    (!extracted.is_empty()).then(|| extracted.to_string())
}

/// 文本序列化器：问题 → 原因 → 怎么做 → 机器码 → 已脱敏原文。
///
/// 只序列化成品，不做任何查表；`sanitized_detail` 为空时不追加诊断段
/// （绝不用英文占位冒充诊断内容）。
pub fn render_detail(copy: &FailureCopy, sanitized_detail: Option<&str>) -> String {
    let mut rendered = String::with_capacity(160);
    rendered.push_str("问题：");
    rendered.push_str(&copy.title_zh);
    rendered.push('\n');
    rendered.push_str("原因：");
    rendered.push_str(&copy.reason_zh);
    rendered.push('\n');
    rendered.push_str("怎么做：");
    rendered.push_str(&copy.action_zh);
    rendered.push('\n');
    rendered.push_str("机器码：");
    rendered.push_str(&copy.machine_line);
    if let Some(detail) = sanitized_detail
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        rendered.push('\n');
        rendered.push_str("诊断：");
        rendered.push_str(detail);
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧实现里的英文占位串：拼接构造，避免被全仓字面量门禁误判。
    fn legacy_placeholder() -> String {
        concat!("no stderr ", "summary").to_string()
    }

    fn code_with_no_detail_and_action(code: &str) -> FailureCopy {
        describe(FailureInput::new(code).with_action("legacy english action line"))
    }

    /// 测试点 #1：`stderr_summary: None` + 非空 action 时三段仍然完整，
    /// 原 action 不进入中文主标题。
    #[test]
    fn missing_stderr_still_renders_three_chinese_segments() {
        let copy = code_with_no_detail_and_action("aggregate_pre_check_failed");
        assert!(copy.title_zh.contains("pre_check"), "{}", copy.title_zh);
        assert!(!copy.reason_zh.trim().is_empty());
        assert!(!copy.action_zh.trim().is_empty());
        assert!(!copy.action_zh.contains("legacy english action line"));
        assert!(copy.machine_line.contains("aggregate_pre_check_failed"));
        assert_eq!(copy.triage, FailureTriage::Environment);
    }

    /// 测试点 #2：最老记录（无 stderr、action 为空）→ 中文兜底，
    /// 且序列化结果不含旧实现那种英文占位（用拼接避免全局门禁命中该字面量）。
    #[test]
    fn oldest_record_without_detail_uses_chinese_fallback() {
        let record = AggregateInitializationErrorRecord {
            stage: "aggregate_initialization".to_string(),
            reason_code: "aggregate_machine_skills_failed".to_string(),
            stderr_summary: None,
            retryable: true,
            action: String::new(),
            cause: None,
            provider: None,
        };
        let copy = describe(FailureInput::from(&record));
        let rendered = render_detail(&copy, record.stderr_summary.as_deref());
        assert!(!rendered.contains(&legacy_placeholder()));
        assert!(rendered.contains("问题："));
        assert!(rendered.contains("原因："));
        assert!(rendered.contains("怎么做："));
        assert!(rendered.contains("机器码："));
        assert!(rendered.contains("aggregate_machine_skills_failed"));
    }

    /// 测试点 #3：码表每一行的三段都非空、标题不含机器码、分诊是合法值。
    #[test]
    fn every_table_entry_has_three_chinese_segments() {
        for entry in TABLE {
            let copy = describe_code(entry.0, None);
            assert!(!copy.title_zh.trim().is_empty(), "{} 缺问题", entry.0);
            assert!(!copy.reason_zh.trim().is_empty(), "{} 缺原因", entry.0);
            assert!(!copy.action_zh.trim().is_empty(), "{} 缺怎么做", entry.0);
            assert_eq!(copy.triage, entry.4, "{} 分诊不一致", entry.0);
            assert_eq!(copy.machine_line, entry.0);
            assert!(
                !contains_machine_code(&copy.title_zh),
                "{} 的标题里出现了机器码：{}",
                entry.0,
                copy.title_zh
            );
            assert!(
                !copy.reason_zh.contains("{") && !copy.action_zh.contains("{"),
                "{} 的模板占位符没有替换干净",
                entry.0
            );
        }
    }

    /// 测试点 #4：真正未知的码走中性兜底，不 panic、不用英文原文当主标题。
    #[test]
    fn unknown_code_falls_back_to_neutral_chinese() {
        let copy = describe_code("unknown_code_xyz", Some("raw english error text"));
        assert_eq!(copy.title_zh, "本次操作未完成");
        assert!(copy.reason_zh.contains("无法确定具体原因"));
        assert!(copy.action_zh.starts_with("请重试"));
        assert_eq!(copy.triage, FailureTriage::ProductDefect);
        assert_eq!(copy.machine_line, "unknown_code_xyz");
        assert!(!copy.title_zh.contains("raw english error text"));
    }

    /// 测试点 #14 ①：七个布局码逐一命中码表，动作是**具体**动作而非「重试」兜底。
    #[test]
    fn seven_layout_codes_hit_concrete_templates() {
        let layout_codes = [
            "aggregate_root_is_git",
            "aggregate_root_member_outside_root",
            "aggregate_root_member_symlink_escape",
            "aggregate_root_overlap",
            "aggregate_root_nested_worktree",
            "aggregate_index_layout_unsupported",
            "aggregate_index_member_coverage_failed",
        ];
        for code in layout_codes {
            let copy = describe_code(code, None);
            assert!(
                !contains_machine_code(&copy.title_zh),
                "{code} 标题含机器码"
            );
            assert!(
                copy.follows_layout_rule(),
                "{code} 的原因没有给出具体规则依据：{}",
                copy.reason_zh
            );
            assert_ne!(
                copy.action_zh,
                neutral_fallback(&FailureInput::new(code)).action_zh,
                "{code} 退化成通用重试兜底"
            );
            assert!(
                !copy
                    .action_zh
                    .contains("提交问题并附上机器码与下方诊断信息"),
                "{code} 用的是未知码兜底动作"
            );
            assert_eq!(copy.triage, FailureTriage::Environment, "{code} 分诊错误");
            assert_eq!(copy.machine_line, code);
        }
    }

    /// 内部码（别名）命中同一模板，且 `machine_line` 保留实际收到的原始码。
    #[test]
    fn internal_alias_resolves_to_external_entry_keeping_raw_code() {
        let copy = describe_code("member_path_outside_root", None);
        assert!(copy.follows_layout_rule());
        assert_eq!(copy.machine_line, "member_path_outside_root");
        assert_eq!(
            canonical_key("member_path_outside_root"),
            "aggregate_root_member_outside_root"
        );
    }

    /// 布局成员对象可点名（有 subject 时进标题），没有 subject 也不产生病句。
    #[test]
    fn member_subject_is_named_when_known() {
        let named =
            describe(FailureInput::new("member_path_outside_root").with_subject("frontend"));
        assert!(
            named.title_zh.contains("「frontend」"),
            "{}",
            named.title_zh
        );
        assert!(
            named.action_zh.contains("<LC根>/「frontend」/") || named.action_zh.contains("<LC根>/"),
            "{}",
            named.action_zh
        );
        let unnamed = describe_code("member_path_outside_root", None);
        assert!(!unnamed.title_zh.contains("「"), "{}", unnamed.title_zh);
    }

    /// 测试点 #14 ②③：码表外未知码的两个入口形态一致（中性兜底），
    /// 且 `record.action` 永远不进主文案/动作段。
    #[test]
    fn unknown_record_action_never_becomes_primary_copy() {
        let record = AggregateInitializationErrorRecord {
            stage: "aggregate_initialization".to_string(),
            reason_code: "totally_unknown_failure".to_string(),
            stderr_summary: Some("boom".to_string()),
            retryable: true,
            action: "The aggregate initialization state could not be persisted; retry.".to_string(),
            cause: None,
            provider: None,
        };
        let copy = describe(FailureInput::from(&record));
        assert_eq!(copy.title_zh, "本次操作未完成");
        assert_eq!(
            copy.action_zh,
            "请重试；连续两次仍失败时，提交问题并附上机器码与下方诊断信息。"
        );
        assert!(!copy.title_zh.contains("persisted"));
        assert!(!copy.action_zh.contains("persisted"));
        assert_eq!(copy.machine_line, "totally_unknown_failure");
    }

    /// 测试点 #12b：成因不可判定 → 诚实标注 + 候选清单，不把候选当结论。
    #[test]
    fn undefined_cause_is_marked_honestly_with_candidates() {
        let copy = describe(
            FailureInput::new("aggregate_pre_check_failed")
                .with_cause("unknown")
                .with_detail("provider turn produced no recognized diagnostic signature"),
        );
        assert!(
            copy.title_zh.contains("无法确定具体成因"),
            "{}",
            copy.title_zh
        );
        assert!(copy.action_zh.contains("provider_unauthorized"));
        assert!(copy.action_zh.contains("provider_command_missing"));
        assert!(copy.action_zh.contains("skills_commands_missing"));
        for assertive in ["已确定是", "确认为", "就是因为"] {
            assert!(
                !copy.title_zh.contains(assertive) && !copy.reason_zh.contains(assertive),
                "候选被写成了结论：{} / {}",
                copy.title_zh,
                copy.reason_zh
            );
        }
        assert_eq!(copy.triage, FailureTriage::ProductDefect);
    }

    /// 已确认的 cause 会改写动作段（结构化子码分流）。
    #[test]
    fn confirmed_cause_rewrites_action() {
        let unavailable = describe(
            FailureInput::new("aggregate_machine_skills_failed")
                .with_cause("skills_unavailable_remote"),
        );
        assert!(
            unavailable.action_zh.contains("网络"),
            "{}",
            unavailable.action_zh
        );
        let sync = describe(
            FailureInput::new("aggregate_machine_skills_failed")
                .with_cause("skills_sync_failed_shared"),
        );
        assert!(sync.action_zh.contains("重新同步"), "{}", sync.action_zh);
        // ② 「拿得到源但更新/落盘失败」必须有自己的动作，落在通用 skills 文案上不算命中。
        let update = describe(
            FailureInput::new("aggregate_machine_skills_failed")
                .with_cause("skills_update_failed_fetch"),
        );
        assert!(
            update.action_zh.contains("上游仓可达性与权限"),
            "skills_update_failed* 未命中专属分支：{}",
            update.action_zh
        );
        let generic = describe_code("aggregate_machine_skills_failed", None);
        assert_ne!(
            update.action_zh, generic.action_zh,
            "skills_update_failed* 退化成通用 skills 动作"
        );
        assert_ne!(
            update.action_zh, unavailable.action_zh,
            "skills_update_failed* 与 skills_unavailable* 的动作必须区分"
        );
    }

    /// 块2 片1 的 `error_record()` 会产出这四个选择期码：必须命中码表（不是
    /// 中性兜底），否则「候选都不可用 / 未核验 / 被拒 / 健康降级」的真实原因
    /// 会被「当前无法确定具体原因」吞掉。
    #[test]
    fn selection_phase_provider_failures_hit_the_table() {
        let expected = [
            (
                "aggregate_initialization_no_provider_available",
                FailureTriage::Environment,
                "都不可用",
            ),
            (
                "aggregate_initialization_provider_needs_verification",
                FailureTriage::Environment,
                "核验",
            ),
            (
                "aggregate_initialization_provider_capability_denied",
                FailureTriage::Environment,
                "拒绝",
            ),
            (
                "aggregate_initialization_provider_health_degraded",
                FailureTriage::ProductDefect,
                "健康",
            ),
        ];
        let neutral = neutral_fallback(&FailureInput::new("x"));
        for (code, triage, keyword) in expected {
            let copy = describe_code(code, None);
            assert_eq!(copy.triage, triage, "{code} 分诊错误");
            assert_eq!(copy.machine_line, code);
            assert!(!copy.title_zh.trim().is_empty(), "{code} 缺问题");
            assert!(!copy.reason_zh.trim().is_empty(), "{code} 缺原因");
            assert!(!copy.action_zh.trim().is_empty(), "{code} 缺怎么做");
            assert!(
                copy.reason_zh.contains(keyword) || copy.title_zh.contains(keyword),
                "{code} 的三段必须点出真实原因（{keyword}）：{} / {}",
                copy.title_zh,
                copy.reason_zh
            );
            assert_ne!(
                copy.action_zh, neutral.action_zh,
                "{code} 落进了中性兜底动作"
            );
            assert_ne!(
                copy.reason_zh, neutral.reason_zh,
                "{code} 落进了中性兜底原因"
            );
        }
    }

    /// REQ-ROUTE-01（对证补登记）：路由归属码必须命中码表——存量单仓 issue
    /// 被读取、载体判定无归属、创建提交缺归属字段都是用户可见失败面，
    /// 落进中性兜底会把真实原因吞成「无法确定具体原因」。
    #[test]
    fn issue_attribution_failure_hits_the_table() {
        let neutral = neutral_fallback(&FailureInput::new("x"));
        for copy in [
            describe_code("issue_not_bound_to_logical_codebase", None),
            describe(FailureInput::new("issue_not_bound_to_logical_codebase")),
        ] {
            assert_eq!(copy.machine_line, "issue_not_bound_to_logical_codebase");
            assert!(!copy.title_zh.trim().is_empty(), "缺问题");
            assert_ne!(copy.reason_zh, neutral.reason_zh, "落进了中性兜底原因");
            assert_ne!(copy.action_zh, neutral.action_zh, "落进了中性兜底动作");
            assert!(!copy.action_zh.trim().is_empty(), "缺怎么做");
            assert_eq!(copy.triage, FailureTriage::Environment);
            assert!(copy.title_zh.contains("逻辑代码库"), "{}", copy.title_zh);
        }
    }

    /// 布局码上已确认的存储失败不伪装成用户布局违规。
    #[test]
    fn layout_code_with_store_cause_becomes_product_defect() {
        let copy = describe(
            FailureInput::new("aggregate_root_is_git")
                .with_cause("store_unreadable")
                .with_detail("could not inspect Git metadata"),
        );
        assert_eq!(copy.triage, FailureTriage::ProductDefect);
        assert!(copy.reason_zh.contains("检查过程本身失败"));
    }

    /// 六条规则覆盖七个对外稳定码，一一可反查。
    #[test]
    fn layout_rules_cover_seven_external_codes() {
        assert_eq!(LAYOUT_RULES.len(), 6);
        let mut codes: Vec<&str> = LAYOUT_RULES
            .iter()
            .flat_map(|rule| rule.codes.iter().copied())
            .collect();
        codes.sort_unstable();
        let mut expected = vec![
            "aggregate_index_layout_unsupported",
            "aggregate_index_member_coverage_failed",
            "aggregate_root_is_git",
            "aggregate_root_member_outside_root",
            "aggregate_root_member_symlink_escape",
            "aggregate_root_nested_worktree",
            "aggregate_root_overlap",
        ];
        expected.sort_unstable();
        assert_eq!(codes, expected);
        for rule in LAYOUT_RULES {
            for code in rule.codes {
                let copy = describe_code(code, None);
                assert!(
                    copy.reason_zh.contains(rule.rule_zh),
                    "{code} 未给出规则依据"
                );
            }
        }
    }

    /// 序列化形态：五字段全部出现，triage 为 snake_case 三值之一。
    #[test]
    fn copy_serializes_five_required_fields() {
        let copy = describe_code("aggregate_root_overlap", None);
        let value = serde_json::to_value(&copy).unwrap();
        for field in [
            "title_zh",
            "reason_zh",
            "action_zh",
            "triage",
            "machine_line",
        ] {
            assert!(value.get(field).is_some(), "缺字段 {field}");
            assert!(!value[field].is_null(), "字段 {field} 为 null");
        }
        assert_eq!(value["triage"], serde_json::json!("environment"));
        assert!(!FailureTriage::ProductDefect.as_str().is_empty());
        assert!(!FailureTriage::RetryOnly.label_zh().is_empty());
    }

    fn contains_machine_code(text: &str) -> bool {
        // 机器码特征：连续 12+ 个小写字母/下划线（如 aggregate_pre_check_failed）。
        let mut run = 0usize;
        for character in text.chars() {
            if character.is_ascii_lowercase() || character == '_' {
                run += 1;
                if run >= 12 {
                    return true;
                }
            } else {
                run = 0;
            }
        }
        false
    }
}
