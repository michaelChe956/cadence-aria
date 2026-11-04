//! 诊断输出的脱敏与截断（7.1~7.3 的公共实现）。
//!
//! 失败面/诊断文本来自 provider CLI、Git、codegraph 等外部进程，可能带
//! 密钥、用户 home 绝对路径与聚合根绝对路径。这里把它们统一收敛成
//! 「可安全展示 + 可安全落盘」的形态：
//!
//! - 控制字符与 ANSI 转义清洗；
//! - 7 条密钥规则脱敏（`KEY=value` 赋值、`sk-`、`ghp_` 族、`xox*-`、
//!   `AKIA*`、`Authorization: Bearer/Basic/Token`、`--api-key` 类命令行旗标）；
//! - 路径降级：home → `~/`，聚合根 → `<聚合根>/`；**成员仓内相对路径保留**；
//! - 上限：4096 字节 / 40 行（超出追加 `…[truncated]`）。
//!
//! 同一条实现同时服务 `LimitedOutput`（仓库初始化输出）与失败面诊断：禁止
//! 再留第二份 sanitize/redact 实现。

/// 诊断文本字节上限。
pub const MAX_DIAGNOSTIC_BYTES: usize = 4096;
/// 诊断文本行数上限。
pub const MAX_DIAGNOSTIC_LINES: usize = 40;
/// 截断标记（唯一一份）。
pub const TRUNCATION_MARKER: &str = "…[truncated]";

/// 路径脱敏上下文：只放**已知的**前缀事实，识别不出的路径原样保留。
#[derive(Debug, Clone, Copy, Default)]
pub struct RedactionContext<'a> {
    /// 用户 home 目录绝对路径。
    pub home: Option<&'a str>,
    /// 聚合根（canonical root）绝对路径。
    pub aggregate_root: Option<&'a str>,
}

impl<'a> RedactionContext<'a> {
    pub fn new(home: Option<&'a str>, aggregate_root: Option<&'a str>) -> Self {
        Self {
            home,
            aggregate_root,
        }
    }
}

/// 清洗控制字符与 ANSI 转义（不截断、不脱敏）。
fn clean_control(value: &str) -> String {
    let mut cleaned = String::with_capacity(value.len());
    let mut in_escape = false;
    for character in value.chars() {
        if in_escape {
            if character.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            in_escape = true;
            continue;
        }
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            continue;
        }
        cleaned.push(character);
    }
    cleaned
}

fn looks_secret(token: &str) -> bool {
    let token = token.trim_matches(|character: char| {
        matches!(character, '"' | '\'' | '`' | ',' | ';' | ')' | ']' | '}')
    });
    if token.len() < 8 {
        return false;
    }
    token.starts_with("sk-")
        || token.starts_with("ghp_")
        || token.starts_with("gho_")
        || token.starts_with("ghu_")
        || token.starts_with("ghs_")
        || token.starts_with("ghr_")
        || token.starts_with("github_pat_")
        || token.starts_with("xoxb-")
        || token.starts_with("xoxp-")
        || token.starts_with("xoxa-")
        || token.starts_with("xoxr-")
        || token.starts_with("xoxs-")
        || (token.starts_with("AKIA") && token.len() >= 20)
}

/// 命令行旗标：`--api-key <value>` 的取值要被吞掉。
fn is_secret_flag(token: &str) -> bool {
    let flag = token.trim_end_matches(':');
    matches!(
        flag,
        "--api-key"
            | "--apikey"
            | "--api_token"
            | "--token"
            | "--auth-token"
            | "--password"
            | "--secret"
            | "--client-secret"
    )
}

/// 单行脱敏：token 化后逐条套规则（保留行内语义，不跨行猜测）。
fn redact_line(line: &str) -> String {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut redacted: Vec<String> = Vec::with_capacity(tokens.len());
    let mut index = 0usize;
    while index < tokens.len() {
        let token = tokens[index];
        // 规则 1：`KEY=value` 赋值（KEY/TOKEN/SECRET/PASSWORD 语义）。
        if let Some((key, _)) = token.split_once('=')
            && !key.is_empty()
        {
            let upper = key.to_ascii_uppercase();
            if ["KEY", "TOKEN", "SECRET", "PASSWORD"]
                .iter()
                .any(|marker| upper.contains(marker))
            {
                redacted.push(format!("{key}=[REDACTED]"));
                index += 1;
                continue;
            }
        }
        // 规则 6：`Authorization: Bearer/Basic/Token <value>`。
        let lower = token.to_ascii_lowercase();
        let authorization_header = lower.starts_with("authorization:");
        let pending_authorization = redacted
            .last()
            .map(|previous| previous.to_ascii_lowercase().starts_with("authorization:"))
            .unwrap_or(false);
        if authorization_header {
            redacted.push(token.to_string());
            index += 1;
            // 下一 token 可能是 scheme，再下一个才是凭据。
            if let Some(scheme) = tokens.get(index) {
                let scheme_lower = scheme.to_ascii_lowercase();
                if matches!(
                    scheme_lower.as_str(),
                    "bearer" | "basic" | "token" | "digest"
                ) {
                    redacted.push(scheme.to_string());
                    index += 1;
                    if tokens.get(index).is_some() {
                        redacted.push("[REDACTED]".to_string());
                        index += 1;
                    }
                } else {
                    redacted.push("[REDACTED]".to_string());
                    index += 1;
                }
            }
            continue;
        }
        if pending_authorization
            && matches!(lower.as_str(), "bearer" | "basic" | "token" | "digest")
        {
            redacted.push(token.to_string());
            index += 1;
            if tokens.get(index).is_some() {
                redacted.push("[REDACTED]".to_string());
                index += 1;
            }
            continue;
        }
        // 规则 7：`--api-key <value>` 类旗标。
        if is_secret_flag(token) {
            redacted.push(token.to_string());
            index += 1;
            if tokens.get(index).is_some() {
                redacted.push("[REDACTED]".to_string());
                index += 1;
            }
            continue;
        }
        // 规则 2~5：裸 token 形态的密钥（sk- / ghp_ / xox* / AKIA）。
        if looks_secret(token) {
            redacted.push("[REDACTED]".to_string());
            index += 1;
            continue;
        }
        redacted.push(token.to_string());
        index += 1;
    }
    redacted.join(" ")
}

/// 密钥脱敏（按行处理，保留换行结构）。
pub fn redact_sensitive_assignments(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for (index, line) in value.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&redact_line(line));
    }
    out
}

/// 路径降级：只替换**已知前缀**，成员仓内相对路径原样保留。
pub fn redact_paths(value: &str, context: &RedactionContext<'_>) -> String {
    let mut redacted = value.to_string();
    if let Some(root) = context.aggregate_root.filter(|root| !root.is_empty()) {
        redacted = redacted.replace(root, "<聚合根>");
    }
    if let Some(home) = context.home.filter(|home| !home.is_empty()) {
        redacted = redacted.replace(home, "~");
    }
    redacted
}

fn truncate_bytes(value: &str, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        return (value.to_string(), false);
    }
    let mut truncated = String::with_capacity(limit + TRUNCATION_MARKER.len());
    for character in value.chars() {
        if truncated.len() + character.len_utf8() > limit {
            truncated.push_str(TRUNCATION_MARKER);
            return (truncated, true);
        }
        truncated.push(character);
    }
    (truncated, true)
}

fn truncate_lines(value: &str, limit: usize) -> (String, bool) {
    if value.split('\n').count() <= limit {
        return (value.to_string(), false);
    }
    let mut kept = String::with_capacity(value.len());
    for (index, line) in value.split('\n').enumerate() {
        if index == limit {
            kept.push('\n');
            kept.push_str(TRUNCATION_MARKER);
            return (kept, true);
        }
        if index > 0 {
            kept.push('\n');
        }
        kept.push_str(line);
    }
    (kept, true)
}

/// 既有语义（`LimitedOutput` 依赖）：清洗 + 密钥脱敏 + 截断到 `limit` 字节。
pub fn sanitize_and_truncate(value: &str, limit: usize) -> String {
    let cleaned = clean_control(value);
    let redacted = redact_sensitive_assignments(&cleaned);
    truncate_bytes(&redacted, limit).0
}

/// 失败面诊断的唯一清洗入口：清洗 → 密钥脱敏 → 路径降级 → 4096 字节/40 行上限。
pub fn sanitize_diagnostic(value: &str, context: &RedactionContext<'_>) -> String {
    let cleaned = clean_control(value);
    let redacted = redact_sensitive_assignments(&cleaned);
    let localized = redact_paths(&redacted, context);
    let (bytes_limited, _) = truncate_bytes(&localized, MAX_DIAGNOSTIC_BYTES);
    let (lines_limited, _) = truncate_lines(&bytes_limited, MAX_DIAGNOSTIC_LINES);
    lines_limited
}

/// provider turn 失败分支的诊断输出：空输出给非空中文兜底串，非空则
/// 清洗 + 密钥脱敏 + 路径降级 + 限长（provider 输出不允许明文令牌进
/// durable 事实）。
pub fn sanitize_turn_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return "未捕获到 provider 输出。".to_string();
    }
    sanitize_diagnostic(trimmed, &RedactionContext::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试点 #5：七条规则覆盖的凭据形态一律不出现明文。
    #[test]
    fn seven_secret_rules_do_not_leak_plaintext() {
        let raw = concat!(
            "sent Authorization: Bearer abcdef1234567890\n",
            "env ANTHROPIC_API_KEY=sk-ant-xxxxxxxxxxxxxxxxxxxx\n",
            "token ghp_abcdefghijklmnopqrstuvwx here\n",
            "slack xoxb-1234567890-abcdefghijk\n",
            "aws AKIAABCDEFGHIJKLMNOP trailing\n",
            "cli --api-key v-secret-value tail\n"
        );
        let sanitized = sanitize_diagnostic(raw, &RedactionContext::default());
        for secret in [
            "abcdef1234567890",
            "sk-ant-xxxxxxxxxxxxxxxxxxxx",
            "ghp_abcdefghijklmnopqrstuvwx",
            "xoxb-1234567890-abcdefghijk",
            "AKIAABCDEFGHIJKLMNOP",
            "v-secret-value",
        ] {
            assert!(
                !sanitized.contains(secret),
                "明文泄漏：{secret}\n{sanitized}"
            );
        }
        assert!(sanitized.contains("[REDACTED]"));
    }

    /// 测试点 #6：home / 聚合根绝对路径降级，成员内相对路径保留。
    #[test]
    fn known_path_prefixes_are_localized_relative_paths_kept() {
        let context = RedactionContext::new(Some("/home/alice"), Some("/srv/aggregate"));
        let raw = "/home/alice/work/root/x\n/srv/aggregate/frontend/src/main.rs\nsrc/main.rs";
        let sanitized = sanitize_diagnostic(raw, &context);
        assert!(sanitized.contains("~/work/root/x"), "{sanitized}");
        assert!(
            sanitized.contains("<聚合根>/frontend/src/main.rs"),
            "{sanitized}"
        );
        assert!(!sanitized.contains("/home/alice"));
        assert!(!sanitized.contains("/srv/aggregate"));
        assert!(sanitized.contains("src/main.rs"));
        // 成员仓内相对路径不被吞掉。
        assert_eq!(sanitized.lines().last(), Some("src/main.rs"));
    }

    /// 测试点 #7：100 KB 输入按字节截断；41 行输入按行截断，都带标记。
    #[test]
    fn oversized_diagnostics_are_truncated_with_marker() {
        let huge = "a".repeat(100 * 1024);
        let sanitized = sanitize_diagnostic(&huge, &RedactionContext::default());
        assert!(
            sanitized.ends_with(TRUNCATION_MARKER),
            "{}",
            &sanitized[..20]
        );
        assert!(sanitized.len() <= MAX_DIAGNOSTIC_BYTES + 15);

        let forty_one_lines = (1..=41)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let sanitized = sanitize_diagnostic(&forty_one_lines, &RedactionContext::default());
        assert!(
            sanitized.ends_with(TRUNCATION_MARKER),
            "行截断没有标记：{sanitized}"
        );
        let lines: Vec<&str> = sanitized.lines().collect();
        assert_eq!(lines.len(), MAX_DIAGNOSTIC_LINES + 1, "{}", lines.len());
        assert_eq!(lines[0], "line 1");
        assert_eq!(lines[MAX_DIAGNOSTIC_LINES - 1], "line 40");
    }

    /// 测试点 #8：命令名、可执行文件路径、错误码不被吞。
    #[test]
    fn command_names_and_error_codes_survive_redaction() {
        let raw = "npm ERR! code E404\n/usr/bin/node --version\ncodegraph: command not found";
        let sanitized = sanitize_diagnostic(raw, &RedactionContext::default());
        assert!(sanitized.contains("npm ERR! code E404"), "{sanitized}");
        assert!(sanitized.contains("/usr/bin/node"), "{sanitized}");
        assert!(
            sanitized.contains("codegraph: command not found"),
            "{sanitized}"
        );
        assert!(!sanitized.contains("[REDACTED]"));
    }

    /// 既有 `LimitedOutput` 语义不变：字节上限 + 密钥脱敏。
    #[test]
    fn legacy_sanitize_and_truncate_contract_is_kept() {
        let summary = sanitize_and_truncate("API_TOKEN=abcdef and more", 64);
        assert!(summary.contains("[REDACTED]"));
        assert!(!summary.contains("abcdef"));
        let long = "x".repeat(200);
        let truncated = sanitize_and_truncate(&long, 32);
        assert!(truncated.ends_with(TRUNCATION_MARKER));
        assert!(truncated.len() <= 32 + TRUNCATION_MARKER.len());
    }

    /// ANSI 转义与控制字符被清洗，换行保留。
    #[test]
    fn ansi_escapes_are_stripped() {
        let raw = "\u{1b}[31mred\u{1b}[0m\nsecond";
        let sanitized = sanitize_diagnostic(raw, &RedactionContext::default());
        assert!(!sanitized.contains('\u{1b}'));
        assert_eq!(sanitized.lines().count(), 2);
    }
}
