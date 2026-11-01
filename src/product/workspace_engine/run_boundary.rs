//! REQ-PRB-09/10：作者侧 tool-call 越界读写扫描与告警事件构造。
//! 只扫 `command` + `detail` 前 4 KiB，单事件最多 8 条告警；手写切词，不引 `regex`。
//! Task 10 起本模块由 `emit_execution_event` 漏斗调用（只告警，不阻断）。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::cross_cutting::streaming_provider::{
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
};
use crate::web::workspace_ws_types::TimelineNodeType;

pub(crate) const MAX_SCAN_BYTES: usize = 4096;
pub(crate) const MAX_RUN_BOUNDARY_ALERTS_PER_NODE: usize = 8;
/// 临时目录第一级名称命中的通用草稿词元（`aria-*` 归状态码，不在此列）。
pub(crate) const TMP_DRAFT_FIRST_LEVEL_TOKENS: &[&str] = &[
    "plan",
    "draft",
    "prior",
    "refined",
    "work-item",
    "spec",
    "design",
    "story",
];
pub(crate) const CODE_PROVIDER_HISTORY_READ: &str = "run_boundary_provider_history_read";
pub(crate) const CODE_ARIA_STATE_READ: &str = "run_boundary_aria_state_read";
pub(crate) const CODE_TMP_DRAFT_TOUCH: &str = "run_boundary_tmp_draft_touch";
pub(crate) const CODE_WRITE_OUTSIDE_SCRATCH: &str = "run_boundary_write_outside_scratch";

/// 扫描上下文；`history_dirs` 由调用方注入，表为空即不报 provider 历史读。
#[derive(Debug, Clone)]
pub(crate) struct RunBoundary {
    pub state_root: PathBuf,
    pub scratch: Option<PathBuf>,
    pub tmp_roots: Vec<PathBuf>,
    pub history_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunBoundaryCode {
    ProviderHistoryRead,
    AriaStateRead,
    TmpDraftTouch,
    WriteOutsideScratch,
}

impl RunBoundaryCode {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::ProviderHistoryRead => CODE_PROVIDER_HISTORY_READ,
            Self::AriaStateRead => CODE_ARIA_STATE_READ,
            Self::TmpDraftTouch => CODE_TMP_DRAFT_TOUCH,
            Self::WriteOutsideScratch => CODE_WRITE_OUTSIDE_SCRATCH,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundaryAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundaryFinding {
    pub code: RunBoundaryCode,
    pub path: String,
    pub access: BoundaryAccess,
}

/// 扫描单个执行事件；非 `Command` 类事件直接返回空。
pub(crate) fn scan_event(
    boundary: &RunBoundary,
    event: &ProviderExecutionEvent,
) -> Vec<BoundaryFinding> {
    if event.kind != ProviderExecutionEventKind::Command {
        return Vec::new();
    }
    let mut text = event.command.clone().unwrap_or_default();
    if let Some(detail) = event.detail.as_deref() {
        text.push('\n');
        text.push_str(detail);
    }
    let mut tokens = tokenize(&truncate_to_bytes(&text, MAX_SCAN_BYTES));
    // `find <tmp_root> -iname '*plan*'` 的通配名补全为 `<tmp_root>/<pattern>` 后同走分类。
    if let Some(root) = tmp_root_of(boundary, &tokens) {
        for token in &mut tokens {
            if absolute_path(&token.text).is_none() && token.text.contains(['*', '?']) {
                token.text = format!("{root}/{}", token.text);
            }
        }
    }
    let mut findings: Vec<BoundaryFinding> = Vec::new();
    for token in tokens {
        let (Some(access), Some(path)) = (token.access, absolute_path(&token.text)) else {
            continue;
        };
        for code in classify(boundary, &path, access) {
            let finding = BoundaryFinding {
                code,
                path: path.clone(),
                access,
            };
            if !findings.contains(&finding) {
                findings.push(finding);
            }
        }
    }
    findings.truncate(MAX_RUN_BOUNDARY_ALERTS_PER_NODE);
    findings
}

fn tmp_root_of(boundary: &RunBoundary, tokens: &[Token]) -> Option<String> {
    tokens.iter().find_map(|token| {
        let path = absolute_path(&token.text)?;
        let under = |root: &PathBuf| Path::new(&path).starts_with(root);
        boundary.tmp_roots.iter().any(under).then_some(path)
    })
}

/// provider 历史目录默认表；`HOME` 不可用或表为空时不报该码。
/// 真机核实（2026-10-27，`ls` + 结构抽样，均存在且为会话历史）：
/// - `~/.codex/sessions/<yyyy>/<mm>/<dd>/`（仓库内证据 `local_usage.rs:134`）
/// - `~/.kimi-code/sessions/<workspace>/session_<uuid>/`
/// - `~/.pi/agent/sessions/<cwd>/<ts>_<uuid>.jsonl`
/// - `~/.claude/projects/<project>/<uuid>.jsonl`（原标注为推断，本次核实后写入）
///
/// provider 的 skills 目录（如 `~/.claude/skills`）不得写入，避免合法读取误报。
pub(crate) fn default_history_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![
        home.join(".codex/sessions"),     // 已核实：local_usage.rs:134 + 真机
        home.join(".kimi-code/sessions"), // 已核实：local_usage.rs:141-146 + 真机
        home.join(".pi/agent/sessions"),  // 已核实：usage_tests.rs:13 + 真机
        home.join(".claude/projects"),    // 已核实：真机（<project>/<uuid>.jsonl 会话记录）
    ]
}

/// 作者侧节点白名单（reviewer 节点不扫）。
pub(crate) fn is_author_side_node(node_type: &TimelineNodeType) -> bool {
    use TimelineNodeType::{AuthorRun, Revision};
    matches!(
        node_type,
        AuthorRun
            | Revision
            | TimelineNodeType::WorkItemDraftRun
            | TimelineNodeType::WorkItemBatchRun
            | TimelineNodeType::WorkItemPlanOutlineRun
            | TimelineNodeType::PlanRepairAuthoringRevision
    )
}

/// 路径的 8 位稳定哈希（告警事件 id 用）。
pub(crate) fn path_hash8(path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())[..8].to_string()
}

/// REQ-PRB-10：越界发现 → 告警事件（`kind=Provider`，故不再触发扫描）。
///
/// `event_id` = `{稳定码}_{路径 8 位哈希}`：稳定码本身已带 `run_boundary_` 前缀，
/// 故不再重复拼前缀；同一 code + path 得到同一 id，由
/// `upsert_execution_event_json` 按 `event_id` 去重（每 node 至多 8 条）。
pub(crate) fn run_boundary_alert_event(
    finding: &BoundaryFinding,
    source_event_id: &str,
) -> ProviderExecutionEvent {
    let code = finding.code.as_str();
    ProviderExecutionEvent {
        event_id: format!("{code}_{}", path_hash8(&finding.path)),
        kind: ProviderExecutionEventKind::Provider,
        status: ProviderExecutionEventStatus::Completed,
        title: format!("运行边界告警: {code}"),
        detail: Some(format!(
            "path={} access={} source_event={source_event_id}",
            finding.path,
            match finding.access {
                BoundaryAccess::Read => "read",
                BoundaryAccess::Write => "write",
            }
        )),
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}

/// 按 UTF-8 字节截断，不切在字符中间。
fn truncate_to_bytes(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

#[derive(Debug, Clone)]
struct Token {
    text: String,
    access: Option<BoundaryAccess>,
}

/// 手写切词：空白与 shell 标点作分隔，`>` 单独成词。
///
/// 引号（`'`/`"`/`` ` ``）只作分组边界，字符本身替换为空白；引号**内**的 shell
/// 符号（`>` `<` `|` `;` `&`）同样替换为空白——它们既不成重定向词元，也不成段
/// 分隔词元（`grep -n 'a > b' f` 里的 `>` 不是任何落盘形态）。
/// 引号外的 `<` `|` `;` `&` 保留为独立词元，供 `is_separator` / `segment_end`
/// 切段（否则 `cp a b; cat x` 的写目标会串到后续命令的只读路径）。
fn tokenize(text: &str) -> Vec<Token> {
    let mut normalized = String::with_capacity(text.len() + 8);
    let mut quote: Option<char> = None;
    for ch in text.chars() {
        if let Some(open) = quote {
            match ch {
                // 只由同种引号闭合；异种引号与分组标点同按空白处理（不切换状态）。
                '>' | '<' | '|' | ';' | '&' | '\'' | '"' | '`' | '(' | ')' | ',' => {
                    normalized.push(' ');
                    if ch == open {
                        quote = None;
                    }
                }
                ch => normalized.push(ch),
            }
            continue;
        }
        match ch {
            '\'' | '"' | '`' => {
                quote = Some(ch);
                normalized.push(' ');
            }
            '>' => normalized.push_str(" > "),
            '<' | '|' | ';' | '&' => {
                normalized.push(' ');
                normalized.push(ch);
                normalized.push(' ');
            }
            '(' | ')' | ',' => normalized.push(' '),
            ch => normalized.push(ch),
        }
    }
    let words: Vec<&str> = normalized.split_whitespace().collect();
    let write_flags = write_targets(&words);
    let mut tokens = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if is_separator(word) {
            continue;
        }
        let access = if absolute_path(word).is_some() {
            if write_flags[index] {
                BoundaryAccess::Write
            } else {
                BoundaryAccess::Read
            }
        } else if word.contains(['*', '?']) {
            BoundaryAccess::Read
        } else {
            continue;
        };
        tokens.push(Token {
            text: (*word).to_string(),
            access: Some(access),
        });
    }
    tokens
}

fn is_separator(word: &str) -> bool {
    matches!(word, "|" | ";" | "&" | "<" | ">")
}

fn segment_end(words: &[&str], start: usize) -> usize {
    words[start..]
        .iter()
        .position(|word| is_separator(word))
        .map_or(words.len(), |offset| start + offset)
}

fn segment_absolutes(words: &[&str], start: usize) -> Vec<usize> {
    (start..segment_end(words, start))
        .filter(|index| absolute_path(words[*index]).is_some())
        .collect()
}

fn previous_absolute(words: &[&str], index: usize) -> Option<usize> {
    (0..index)
        .rev()
        .find(|cursor| !is_separator(words[*cursor]) && absolute_path(words[*cursor]).is_some())
}

/// `open(path, mode)`：路径之后出现写模式词元时，该路径即写目标。
fn open_target(words: &[&str], start: usize) -> Option<usize> {
    let mut candidate = None;
    let end = segment_end(words, start);
    for (offset, word) in words[start..end].iter().enumerate() {
        if matches!(*word, "w" | "wb" | "wt" | "a" | "ab" | "at" | "w+") {
            return candidate;
        }
        if absolute_path(word).is_some() {
            candidate = Some(start + offset);
        }
    }
    None
}

/// 写形态词元（重定向 / `tee` / `cp` / `mv` / `sed -i` / `apply_patch` 的 Add·Update
/// File / `write_text` / 以写模式打开）之后的首或末绝对路径词元记为写目标。
fn write_targets(words: &[&str]) -> Vec<bool> {
    let mut flags = vec![false; words.len()];
    for (index, word) in words.iter().enumerate() {
        let target = match *word {
            ">" | "tee" => segment_absolutes(words, index + 1).first().copied(),
            "cp" | "mv" => segment_absolutes(words, index + 1).last().copied(),
            "sed"
                if words
                    .get(index + 1)
                    .is_some_and(|next| next.starts_with("-i")) =>
            {
                segment_absolutes(words, index + 2).last().copied()
            }
            "***"
                if matches!(words.get(index + 1).copied(), Some("Add" | "Update"))
                    && words.get(index + 2) == Some(&"File:") =>
            {
                segment_absolutes(words, index + 3).first().copied()
            }
            "open" => open_target(words, index + 1),
            _ if word.contains("write_text") => previous_absolute(words, index),
            _ => None,
        };
        if let Some(target) = target {
            // 设备路径不是落盘目标（`2>/dev/null`、`tee /dev/null` 等不得判写越界）。
            if !is_device_path(words[target]) {
                flags[target] = true;
            }
        }
    }
    flags
}

/// `/dev/` 之下的设备/伪设备路径：重定向到设备不构成「写入磁盘上的路径」。
fn is_device_path(token: &str) -> bool {
    token.starts_with("/dev/")
}

/// 绝对路径词元（支持 `--flag=/abs/path` 形式）。
fn absolute_path(token: &str) -> Option<String> {
    let candidate = match token.rsplit_once('=') {
        Some((_, suffix)) if suffix.starts_with('/') => suffix,
        _ => token,
    };
    (candidate.starts_with('/') && candidate.len() > 1).then(|| candidate.to_string())
}

/// 四类判定：provider 历史目录 → Aria 状态树 → 临时草稿名 → 写越界。
fn classify(boundary: &RunBoundary, path: &str, access: BoundaryAccess) -> Vec<RunBoundaryCode> {
    let mut codes = Vec::new();
    let path_ref = Path::new(path);
    let history = boundary
        .history_dirs
        .iter()
        .any(|dir| path_ref.starts_with(dir) || Path::new(dir).starts_with(path_ref));
    let under_scratch = boundary
        .scratch
        .as_deref()
        .is_some_and(|scratch| path_ref.starts_with(scratch));
    let tmp_first = boundary.tmp_roots.iter().find_map(|root| {
        let first = path_ref.strip_prefix(root).ok()?.components().next()?;
        Some(first.as_os_str().to_string_lossy().to_ascii_lowercase())
    });
    let aria_backup = tmp_first
        .as_deref()
        .is_some_and(|name| name.starts_with("aria-"));
    if history {
        codes.push(RunBoundaryCode::ProviderHistoryRead);
    }
    if (path_ref.starts_with(&boundary.state_root) && !under_scratch) || aria_backup {
        codes.push(RunBoundaryCode::AriaStateRead);
    }
    let tmp_draft = tmp_first.as_deref().is_some_and(|name| {
        !name.starts_with("aria-")
            && TMP_DRAFT_FIRST_LEVEL_TOKENS
                .iter()
                .any(|token| name.contains(token))
    });
    if tmp_draft {
        codes.push(RunBoundaryCode::TmpDraftTouch);
    }
    if access == BoundaryAccess::Write && !under_scratch && !is_device_path(path) {
        codes.push(RunBoundaryCode::WriteOutsideScratch);
    }
    codes
}

// 内联测试逐字保留计划 Task 9 Step 1 的用例文本（故跳过 rustfmt 重排）。
#[rustfmt::skip]
#[cfg(test)]
mod boundary_scan_tests {
    use super::*;
    use crate::cross_cutting::streaming_provider::{
        ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    };
    use crate::web::workspace_ws_types::TimelineNodeType;
    use std::path::PathBuf;

    fn command_event(command: &str) -> ProviderExecutionEvent {
        ProviderExecutionEvent {
            event_id: "call_1".to_string(),
            kind: ProviderExecutionEventKind::Command,
            status: ProviderExecutionEventStatus::Started,
            title: "bash".to_string(),
            detail: Some(command.to_string()),
            command: Some(command.to_string()),
            cwd: None,
            output: None,
            exit_code: None,
        }
    }

    fn boundary(state_root: &str, scratch: Option<&str>, history: &[&str]) -> RunBoundary {
        RunBoundary {
            state_root: PathBuf::from(state_root),
            scratch: scratch.map(PathBuf::from),
            tmp_roots: vec![PathBuf::from("/tmp"), PathBuf::from("/var/tmp")],
            history_dirs: history.iter().map(PathBuf::from).collect(),
        }
    }

    fn codes(findings: &[BoundaryFinding]) -> Vec<&'static str> {
        findings.iter().map(|finding| finding.code.as_str()).collect()
    }

    #[test]
    fn boundary_scan_matches_incident_vectors() {
        let state = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/workspace_session_auto_510c9278"), &["/home/u/.codex/sessions"]);
        let cases: &[(&str, &str)] = &[
            ("find /tmp -iname '*work*item*plan*'", CODE_TMP_DRAFT_TOUCH),
            ("jq -r . /home/u/.codex/sessions/2026/10/27/rollout-1.jsonl", CODE_PROVIDER_HISTORY_READ),
            ("ls /tmp/prior-plan-2026-10-26.md", CODE_TMP_DRAFT_TOUCH),
            ("cat /home/u/ws/.aria/projects/project_0001/issue.json", CODE_ARIA_STATE_READ),
            ("echo x > /tmp/work-item-plan.md", CODE_WRITE_OUTSIDE_SCRATCH),
        ];
        for (command, expected) in cases {
            let findings = scan_event(&state, &command_event(command));
            assert!(codes(&findings).contains(expected), "command={command} findings={:?}", codes(&findings));
        }
        // 写到 scratch 之外：access 必须是 write
        let write = scan_event(&state, &command_event("echo x > /tmp/work-item-plan.md"));
        assert!(write.iter().any(|finding| finding.code == RunBoundaryCode::WriteOutsideScratch
            && finding.access == BoundaryAccess::Write
            && finding.path == "/tmp/work-item-plan.md"));
    }

    #[test]
    fn boundary_scan_state_read_uses_configured_state_root() {
        // 契约《事故读取向量全部命中》：/tmp/aria-restore 归 aria_state_read。
        // 口径（2026-10-27 统一）：类别②=「Aria 状态树」= 状态根之下 **或**
        // 系统临时目录下第一级名 `aria-*` 的状态/备份目录之下 → 与 state_root
        // 是否位于 /tmp 之下无关，生产口径同样归 aria_state_read。
        let incident = boundary("/tmp", Some("/tmp/run-scratch/workspace_session_auto_510c9278"), &["/home/u/.codex/sessions"]);
        let findings = scan_event(&incident, &command_event("ls /tmp/aria-restore/entity-deletions/plan"));
        assert!(codes(&findings).contains(&CODE_ARIA_STATE_READ), "{:?}", codes(&findings));

        // 生产口径（state_root 在家目录）下同一路径仍归 aria_state_read（备份树）。
        let production = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &[]);
        let findings = scan_event(&production, &command_event("ls /tmp/aria-restore/entity-deletions/plan"));
        assert!(codes(&findings).contains(&CODE_ARIA_STATE_READ), "{:?}", codes(&findings));
        // aria-* 不进 tmp_draft_touch（它归 aria_state_read）。
        assert!(!codes(&findings).contains(&CODE_TMP_DRAFT_TOUCH), "{:?}", codes(&findings));
        // 通用草稿名仍是 tmp_draft_touch（不含 aria-*）。
        let findings = scan_event(&production, &command_event("cat /tmp/work-item-plan-draft.md"));
        assert!(codes(&findings).contains(&CODE_TMP_DRAFT_TOUCH), "{:?}", codes(&findings));
        assert!(!codes(&findings).contains(&CODE_ARIA_STATE_READ), "{:?}", codes(&findings));
    }

    #[test]
    fn boundary_scan_ignores_legitimate_access() {
        let b = boundary(
            "/home/u/ws/.aria",
            Some("/home/u/ws/.aria/run-scratch/workspace_session_auto_1"),
            &["/home/u/.codex/sessions"],
        );
        for command in [
            "cat src/product/main.rs",
            "git show refs/heads/main:src/lib.rs",
            "cat /home/u/.claude/skills/review/SKILL.md",
            "echo note > /home/u/ws/.aria/run-scratch/workspace_session_auto_1/note.md",
            "cat /home/u/ws/.aria/run-scratch/workspace_session_auto_1/note.md",
        ] {
            let findings = scan_event(&b, &command_event(command));
            assert!(findings.is_empty(), "command={command} findings={:?}", codes(&findings));
        }
    }

    #[test]
    fn boundary_scan_does_not_flag_repo_under_tmp() {
        let b = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &[]);
        let findings = scan_event(&b, &command_event("cat /tmp/test-center-java/src/Main.java"));
        assert!(findings.is_empty(), "{:?}", codes(&findings));
    }

    #[test]
    fn boundary_scan_uses_configurable_history_dirs() {
        let with_table = boundary("/home/u/ws/.aria", None, &["/home/u/.acme/hist"]);
        let findings = scan_event(&with_table, &command_event("cat /home/u/.acme/hist/s.json"));
        assert!(codes(&findings).contains(&CODE_PROVIDER_HISTORY_READ), "{:?}", codes(&findings));

        let empty_table = boundary("/home/u/ws/.aria", None, &[]);
        let findings = scan_event(&empty_table, &command_event("cat /home/u/.acme/hist/s.json"));
        assert!(findings.is_empty(), "表为空即不报该码：{:?}", codes(&findings));
    }

    #[test]
    fn resume_reading_provider_history_warns() {
        // Review Focus #1：resume 会话再次读取自身历史时检测层必须告警。
        let b = boundary("/home/u/ws/.aria", None, &["/home/u/.codex/sessions"]);
        let findings = scan_event(&b, &command_event("cat /home/u/.codex/sessions/2026/10/27/rollout-1.jsonl | jq -r ."));
        assert!(codes(&findings).contains(&CODE_PROVIDER_HISTORY_READ), "{:?}", codes(&findings));
    }

    #[test]
    fn boundary_scan_is_bounded_and_never_panics() {
        let b = boundary("/home/u/ws/.aria", None, &[]);
        let padded = format!("{}cat /tmp/work-item-plan.md", " ".repeat(MAX_SCAN_BYTES));
        assert!(scan_event(&b, &command_event(&padded)).is_empty(), "超出前 4 KiB 的部分不扫");

        let empty = ProviderExecutionEvent {
            event_id: "call_2".to_string(),
            kind: ProviderExecutionEventKind::Command,
            status: ProviderExecutionEventStatus::Started,
            title: String::new(),
            detail: None,
            command: None,
            cwd: None,
            output: None,
            exit_code: None,
        };
        assert!(scan_event(&b, &empty).is_empty());
    }

    #[test]
    fn author_side_node_whitelist_excludes_reviewer() {
        for node_type in [
            TimelineNodeType::AuthorRun,
            TimelineNodeType::Revision,
            TimelineNodeType::WorkItemDraftRun,
            TimelineNodeType::WorkItemBatchRun,
            TimelineNodeType::WorkItemPlanOutlineRun,
            TimelineNodeType::PlanRepairAuthoringRevision,
        ] {
            assert!(is_author_side_node(&node_type), "{node_type:?}");
        }
        for node_type in [
            TimelineNodeType::ReviewerRun,
            TimelineNodeType::ReviewDecision,
            TimelineNodeType::WorkItemDraftReview,
            TimelineNodeType::WorkItemBatchReview,
            TimelineNodeType::WorkItemPlanOutlineReview,
            TimelineNodeType::PlanRepairPlanReview,
            TimelineNodeType::Completed,
        ] {
            assert!(!is_author_side_node(&node_type), "{node_type:?}");
        }
    }

    #[test]
    fn path_hash8_is_deterministic_and_eight_chars() {
        assert_eq!(path_hash8("/tmp/a"), path_hash8("/tmp/a"));
        assert_eq!(path_hash8("/tmp/a").len(), 8);
        assert_ne!(path_hash8("/tmp/a"), path_hash8("/tmp/b"));
    }

    #[test]
    fn default_history_dirs_includes_codex_sessions() {
        let dirs = default_history_dirs();
        // 真机核实过的四条历史目录（HOME 不可用时表为空即不报该码）。
        match std::env::var_os("HOME") {
            Some(home) => {
                let home = PathBuf::from(home);
                for dir in [".codex/sessions", ".kimi-code/sessions", ".pi/agent/sessions", ".claude/projects"] {
                    assert!(dirs.contains(&home.join(dir)), "{dir}");
                }
            }
            None => assert!(dirs.is_empty(), "HOME 不可用时表为空"),
        }
        // provider skills 目录不得进表（合法读取不得误报）。
        assert!(dirs.iter().all(|dir| !dir.to_string_lossy().contains("skills")));
        // claude 的历史目录是 projects，不是 projects 之外的路径。
        assert!(dirs.iter().all(|dir| !dir.to_string_lossy().contains(".claude/plans")));
    }

    #[test]
    fn boundary_scan_flags_each_code_table() {
        let b = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &["/home/u/.codex/sessions"]);
        let cases: &[(&str, Option<&str>)] = &[
            ("cat /home/u/.codex/sessions/2026/10/27/rollout-1.jsonl", Some(CODE_PROVIDER_HISTORY_READ)),
            ("ls /home/u/.codex", Some(CODE_PROVIDER_HISTORY_READ)),
            ("cat /home/u/.codex/config.toml", None),
            ("cat /home/u/ws/.aria/projects/project_0001/issue.json", Some(CODE_ARIA_STATE_READ)),
            ("tail -n 5 /tmp/aria-deploy/logs/run.log", Some(CODE_ARIA_STATE_READ)),
            ("cat /tmp/other/notes.md", None),
            ("cat /tmp/design/spec.md", Some(CODE_TMP_DRAFT_TOUCH)),
            ("cat /var/tmp/Story-Notes.md", Some(CODE_TMP_DRAFT_TOUCH)),
            ("cat /tmp/test-center-java/src/Main.java", None),
            ("echo x > /tmp/report.md", Some(CODE_WRITE_OUTSIDE_SCRATCH)),
            ("echo x > /home/u/ws/.aria/run-scratch/sid/note.md", None),
        ];
        for (command, expected) in cases {
            let got = codes(&scan_event(&b, &command_event(command)));
            match expected {
                Some(code) => assert!(got.contains(code), "command={command} got={got:?}"),
                None => assert!(got.is_empty(), "command={command} got={got:?}"),
            }
        }
        // aria-* 只归 aria_state_read，不进 tmp_draft_touch。
        let got = codes(&scan_event(&b, &command_event("cat /tmp/aria-restore/plan.md")));
        assert!(got.contains(&CODE_ARIA_STATE_READ), "{got:?}");
        assert!(!got.contains(&CODE_TMP_DRAFT_TOUCH), "{got:?}");
    }

    #[test]
    fn boundary_scan_flags_write_forms_table() {
        let b = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &[]);
        for command in [
            "echo x > /tmp/a.md",
            "echo x >> /tmp/a.md",
            "ls | tee /tmp/a.md",
            "cp /tmp/a.md /tmp/b.md",
            "mv /tmp/a.md /tmp/b.md",
            "sed -i 's/a/b/' /tmp/a.md",
            "apply_patch <<'EOF'\n*** Add File: /tmp/a.md\n*** Update File: /tmp/b.md\nEOF",
            "python3 -c \"open('/tmp/a.md','w')\"",
            "Path('/tmp/a.md').write_text('x')",
            "cat > /tmp/a.md <<'EOF'\nbody\nEOF",
        ] {
            let got = codes(&scan_event(&b, &command_event(command)));
            assert!(got.contains(&CODE_WRITE_OUTSIDE_SCRATCH), "command={command} got={got:?}");
        }
        for command in [
            "cat /tmp/a.md",
            "sed -n 's/a/b/p' /tmp/a.md",
            "python3 -c \"open('/tmp/a.md','r')\"",
            "cp /tmp/a.md /home/u/ws/.aria/run-scratch/sid/b.md",
            "grep -n foo /tmp/a.md",
        ] {
            let got = codes(&scan_event(&b, &command_event(command)));
            assert!(!got.contains(&CODE_WRITE_OUTSIDE_SCRATCH), "command={command} got={got:?}");
        }
    }

    #[test]
    fn boundary_scan_ignores_device_redirection() {
        // 审查应修 1：设备路径（`/dev/` 前缀）不是落盘目标，`2>/dev/null` 等
        // 常见 shell 惯用法不得产生「写越界」假告警。
        let b = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &[]);
        for command in [
            "ls /tmp 2>/dev/null",
            "bash -c 'ls /tmp' 2>/dev/null",
            "make > /dev/null 2>&1",
            "ls | tee /dev/null",
        ] {
            let findings = scan_event(&b, &command_event(command));
            assert!(findings.is_empty(), "command={command} findings={:?}", codes(&findings));
        }
        // 排除设备不改变真实落盘的召回面：同一命令里的文件重定向照常命中。
        let findings = scan_event(&b, &command_event("echo x > /dev/null 2>&1 && echo y > /tmp/report.md"));
        assert!(findings.iter().any(|finding| finding.code == RunBoundaryCode::WriteOutsideScratch
            && finding.access == BoundaryAccess::Write
            && finding.path == "/tmp/report.md"), "{:?}", findings);
        assert!(!findings.iter().any(|finding| finding.path.starts_with("/dev/")), "{:?}", findings);
    }

    fn write_paths(findings: &[BoundaryFinding]) -> Vec<&str> {
        findings
            .iter()
            .filter(|finding| finding.access == BoundaryAccess::Write)
            .map(|finding| finding.path.as_str())
            .collect()
    }

    #[test]
    fn boundary_scan_keeps_write_targets_within_segment() {
        // 审查应修 2：`cp`/`mv`/`sed -i` 的写目标只取本段路径；后续命令的绝对
        // 路径（只读）不得被当成写目标串味。
        let b = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &[]);
        let cases: &[(&str, &str)] = &[
            ("cp /repo/b.md /repo/c.md; cat /tmp/log.txt", "/repo/c.md"),
            ("cp /repo/b.md /repo/c.md && cat /tmp/log.txt", "/repo/c.md"),
            ("mv /repo/b.md /repo/c.md | cat /tmp/log.txt", "/repo/c.md"),
            ("sed -i 's/a/b/' /repo/x && cat /tmp/log.txt", "/repo/x"),
        ];
        for (command, expected) in cases {
            let findings = scan_event(&b, &command_event(command));
            let writes = write_paths(&findings);
            assert_eq!(writes, vec![*expected], "command={command} findings={:?}", codes(&findings));
            assert!(!writes.contains(&"/tmp/log.txt"), "command={command} 写目标串味：{writes:?}");
        }
        // 同段内的真实写仍命中（末个绝对路径）。
        for command in [
            "cp /repo/b.md /tmp/copy.md",
            "mv /repo/b.md /tmp/moved.md",
            "sed -i 's/a/b/' /tmp/notes.md",
        ] {
            let got = codes(&scan_event(&b, &command_event(command)));
            assert!(got.contains(&CODE_WRITE_OUTSIDE_SCRATCH), "command={command} got={got:?}");
        }
    }

    #[test]
    fn boundary_scan_ignores_operators_inside_quotes() {
        // 审查建议 3：引号内（shell 字面量）的 `>` 不是重定向，其后的只读路径
        // 不得被判写越界。
        let b = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &[]);
        for command in [
            "grep -n 'a > b' /tmp/report.txt",
            "grep -n \"a > b\" /tmp/report.txt",
            "grep -n 'a | b' /tmp/report.txt",
            "grep -n 'a;b' /tmp/report.txt",
        ] {
            let findings = scan_event(&b, &command_event(command));
            assert!(!codes(&findings).contains(&CODE_WRITE_OUTSIDE_SCRATCH), "command={command} findings={:?}", codes(&findings));
        }
        // 引号内符号不得破坏真实写形态的识别（`sed -i` 的 `|` 作分隔符）。
        let got = codes(&scan_event(&b, &command_event("sed -i 's|a|b|' /tmp/notes.md")));
        assert!(got.contains(&CODE_WRITE_OUTSIDE_SCRATCH), "{got:?}");
        // 引号外的重定向照常判写。
        let got = codes(&scan_event(&b, &command_event("echo 'a > b' > /tmp/report.md")));
        assert!(got.contains(&CODE_WRITE_OUTSIDE_SCRATCH), "{got:?}");
    }

    #[test]
    fn boundary_scan_caps_findings_per_event() {
        let b = boundary("/home/u/ws/.aria", Some("/home/u/ws/.aria/run-scratch/sid"), &[]);
        let paths: Vec<String> = (0..20).map(|i| format!("/home/u/ws/.aria/projects/p{i}/issue.json")).collect();
        let findings = scan_event(&b, &command_event(&format!("cat {}", paths.join(" "))));
        assert_eq!(findings.len(), MAX_RUN_BOUNDARY_ALERTS_PER_NODE, "{:?}", codes(&findings));
    }

    #[test]
    fn boundary_scan_handles_malformed_input_without_panic() {
        let b = boundary("/home/u/ws/.aria", None, &[]);
        for command in ["", ">>>><<<<'\"```", "sed -i", "open(", "cp", "cat '/tmp/x"] {
            let _ = scan_event(&b, &command_event(command));
        }
        // 4 KiB 截断不得切在字符中间（多字节边界）。
        let text = format!("{}中{}", "a".repeat(MAX_SCAN_BYTES - 2), "b");
        assert_eq!(truncate_to_bytes(&text, MAX_SCAN_BYTES).len(), MAX_SCAN_BYTES - 2);
        // detail 为空但 command 有内容时照常判定。
        let mut event = command_event("cat /tmp/work-item-plan.md");
        event.detail = None;
        assert!(!scan_event(&b, &event).is_empty());
    }
}
