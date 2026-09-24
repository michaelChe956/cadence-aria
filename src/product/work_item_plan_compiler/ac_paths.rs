//! AC×基线树核对的受限路径提取（REQ-WSC-02 场景 13，F-56）。
//!
//! 只认显式路径形态——statement 中的方法前缀路由（`GET /status.html`）、
//! 反引号/引号 span、验证命令 token——不做全文模糊匹配。文件形态判定共用
//! 一条规则（末段必须含扩展名点、无内部空白、不含 `..`/`:`/通配符/shell
//! 标点、非绝对路径/URL/flag/变量，尾部 CJK 句读剥除），使 `/api/status`
//! 等无扩展名路由与裸 prose 一律不触发。

use super::lower::PlanCandidateIr;

/// statement 路由形态认定的 HTTP 方法集（小写不认：路由惯例全大写）。
const HTTP_METHODS: [&str; 6] = ["GET", "POST", "PUT", "DELETE", "HEAD", "PATCH"];

/// 命令链/引号 span 尾缀常见的 shell 标点：含任一即整体拒绝——`test/x.test.js;`
/// 这类拼接残渣无法与路径本体安全区分，宁拒勿误报（拒绝方向 fail-safe，
/// 不产生基线核对假阳性）。
const SHELL_PUNCT: &[u8] = b";><|&(),=";

/// 路径 token 的允许字符（ASCII 字母数字 + `._-/`）：遇到其余字符（空白、
/// CJK 连接词、标点）即终止，天然兼容中文 statement 中的 `与`/`。` 等边界。
fn is_path_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')
}

/// 逐 WI 提取验收面（AC statement + 验证命令）引用的仓库相对路径候选。
///
/// 返回 `(logical_work_item_id, 去重路径清单)`；提取形态受限（见模块注释），
/// 顺序稳定（先 statement 后命令、首见序）。
pub(crate) fn extract_acceptance_path_refs(ir: &PlanCandidateIr) -> Vec<(String, Vec<String>)> {
    ir.items
        .iter()
        .map(|item| {
            let mut refs: Vec<String> = Vec::new();
            for criterion in &item.contract.acceptance_criteria {
                collect_statement_paths(&criterion.statement, &mut refs);
            }
            for entry in &item.trusted_commands {
                collect_command_paths(&entry.command, &mut refs);
            }
            for check in &item.verification_plan.checks {
                if let Some(command) = &check.command {
                    collect_command_paths(command, &mut refs);
                }
            }
            (item.contract.identity.logical_work_item_id.clone(), refs)
        })
        .collect()
}

/// statement 的受限提取：方法前缀路由 + 反引号/引号 span。
fn collect_statement_paths(statement: &str, refs: &mut Vec<String>) {
    collect_method_routes(statement, refs);
    collect_quoted_spans(statement, refs);
}

/// 扫描 `METHOD /<path>` 形态：方法为独立词（前置字符非字母数字），后随
/// 单空格与 `/`；路径 token 按 [`is_path_char`] 截取后剥首 `/`。
fn collect_method_routes(statement: &str, refs: &mut Vec<String>) {
    let bytes = statement.as_bytes();
    for method in HTTP_METHODS {
        let mut cursor = 0;
        while let Some(offset) = statement[cursor..].find(method) {
            let start = cursor + offset;
            cursor = start + method.len();
            // 方法必须是独立词（前置边界：串首或非字母数字字符）。
            if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
                continue;
            }
            let Some(rest) = statement[cursor..].strip_prefix(' ') else {
                continue;
            };
            let Some(route) = rest.strip_prefix('/') else {
                continue;
            };
            cursor += 2;
            let end = route
                .bytes()
                .position(|byte| !is_path_char(byte))
                .unwrap_or(route.len());
            if end > 0 {
                push_repo_path(&route[..end], refs);
            }
        }
    }
}

/// 扫描反引号与双/单引号 span：内容可带 `GET ` 等方法前缀（剥掉后按文件
/// 形态判定）。成对匹配（首个闭引号截断），不跨 span 嵌套。
fn collect_quoted_spans(statement: &str, refs: &mut Vec<String>) {
    for delimiter in ['`', '"', '\''] {
        let mut cursor = 0;
        while let Some(open) = statement[cursor..].find(delimiter) {
            let open_at = cursor + open + delimiter.len_utf8();
            let Some(relative_close) = statement[open_at..].find(delimiter) else {
                break;
            };
            let close_at = open_at + relative_close;
            let span = &statement[open_at..close_at];
            cursor = close_at + delimiter.len_utf8();
            let span = strip_method_prefix(span);
            push_repo_path(span, refs);
        }
    }
}

/// 命令 token 提取：按空白切分、剥外层配对引号后按文件形态判定
/// （`node --test test/x.test.js` → `test/x.test.js`；`--test`/`node` 排除）。
fn collect_command_paths(command: &str, refs: &mut Vec<String>) {
    for token in command.split_whitespace() {
        let token = token
            .strip_prefix('`')
            .or_else(|| token.strip_prefix('"'))
            .or_else(|| token.strip_prefix('\''))
            .unwrap_or(token);
        let token = token
            .strip_suffix('`')
            .or_else(|| token.strip_suffix('"'))
            .or_else(|| token.strip_suffix('\''))
            .unwrap_or(token);
        push_repo_path(token, refs);
    }
}

/// 剥 span 内可选的方法前缀（`GET /x` → `/x`），无前缀原样返回。
fn strip_method_prefix(span: &str) -> &str {
    for method in HTTP_METHODS {
        if let Some(rest) = span.strip_prefix(method)
            && let Some(route) = rest.strip_prefix(' ')
        {
            return route;
        }
    }
    span
}

/// 文件形态判定与入列：剥前导 `./` 与尾部 CJK 句读（`。` `，` `；` `、`——
/// 中文 statement 反引号 span 常见 `status.html。` 形态，句读不属路径本体）；
/// 拒绝含内部空白/`..`/`:`/`*`/`?`/`\`、shell 标点（[`SHELL_PUNCT`]）、绝对
/// 路径、`-` flag、`$` 变量；要求末段含 `.`（扩展名）——含 `/` 的目录形态
/// （`api/status`）与无扩展名裸词（`CommonJS`）一律不入列。
fn push_repo_path(candidate: &str, refs: &mut Vec<String>) {
    let candidate = candidate
        .trim()
        .trim_start_matches("./")
        .trim_end_matches(['。', '，', '；', '、']);
    if candidate.is_empty()
        || candidate.starts_with('/')
        || candidate.starts_with('-')
        || candidate.starts_with('$')
    {
        return;
    }
    if candidate
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b':' | b'*' | b'?' | b'\\'))
        || candidate.bytes().any(|byte| SHELL_PUNCT.contains(&byte))
    {
        return;
    }
    if candidate.contains("..") {
        return;
    }
    let last_segment = candidate.rsplit('/').next().unwrap_or(candidate);
    if !last_segment.contains('.') {
        return;
    }
    if !refs.iter().any(|existing| existing == candidate) {
        refs.push(candidate.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_form_rule_excludes_routes_and_accepts_file_paths() {
        let mut refs = Vec::new();
        push_repo_path("api/status", &mut refs);
        push_repo_path("CommonJS", &mut refs);
        push_repo_path("src/product", &mut refs);
        push_repo_path("--test", &mut refs);
        push_repo_path("$PATH", &mut refs);
        push_repo_path("https://example.com/x.js", &mut refs);
        push_repo_path("node:http", &mut refs);
        push_repo_path("a/../b.js", &mut refs);
        push_repo_path("status.html", &mut refs);
        push_repo_path("./test/x.test.js", &mut refs);
        push_repo_path("test/status.test.js", &mut refs);
        push_repo_path("status.html", &mut refs);
        assert_eq!(
            refs,
            vec!["status.html", "test/x.test.js", "test/status.test.js"]
        );
    }

    /// C1K3 P2 负例：命令链尾分号 token——`test/x.test.js;` 属 shell 拼接残渣，
    /// 必须整体拒绝（基线树只有 `test/x.test.js`，带分号入列即基线核对误报）；
    /// 无标点 token 不受影响。
    #[test]
    fn command_chain_trailing_shell_punctuation_is_rejected() {
        let mut refs = Vec::new();
        collect_command_paths("node --test test/x.test.js; npm run lint", &mut refs);
        assert!(refs.is_empty(), "带尾分号 token 不得入列：{refs:?}");

        let mut clean = Vec::new();
        collect_command_paths("node --test test/x.test.js", &mut clean);
        assert_eq!(clean, vec!["test/x.test.js".to_string()]);
    }

    /// C1K3 P2 负例：中文 statement 反引号 span 尾部全角句号（`status.html。`）
    /// 属句读而非路径本体——剥离后命中真实路径；无句读 span 不受影响。
    #[test]
    fn quoted_span_trailing_cjk_punctuation_is_stripped() {
        let mut refs = Vec::new();
        collect_quoted_spans("更新 `status.html。` 后重启服务", &mut refs);
        assert_eq!(refs, vec!["status.html".to_string()]);

        let mut clean = Vec::new();
        collect_quoted_spans("更新 `status.html` 后重启服务", &mut clean);
        assert_eq!(clean, vec!["status.html".to_string()]);
    }

    #[test]
    fn method_routes_stop_at_cjk_and_punctuation() {
        let mut refs = Vec::new();
        collect_method_routes(
            "请求 GET /api/status 与 GET /status.html 与未知路径 THE SYSTEM SHALL 保持 text/html。",
            &mut refs,
        );
        assert_eq!(refs, vec!["status.html".to_string()]);
    }
}
