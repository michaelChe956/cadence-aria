//! REQ-REG-17（块3 片4）防回归守卫：生产代码 MUST NOT 出现「成员数为 1」
//! 的专门分支，也 MUST NOT 出现「至少 2 个成员」类门禁——单成员（1 成员）
//! LC 与多成员（N 成员）LC 必须走同一条代码路径。
//!
//! 三条断言（T4.1~T4.3）把计划文档 §2.4.1 的核查表固化为可执行扫描：
//! - 扫描 `src/**` 与 `web/src/**` 的**生产**代码；
//! - 测试-only 文件（`tests`/`__tests__`/`fixtures` 目录、`*_tests*`/`.test.`/
//!   `.spec.` 文件名）不扫——它们不是生产分支；
//! - Rust 生产文件内的 `#[cfg(test)] mod …` 块与 `#[test]`/`#[tokio::test]`
//!   函数体按花括号配对剔除后再扫（行号保持原文件行号，命中报 file:line）。
//!
//! 每条断言先对**合成样本**做阴性对照（守卫本身能抓到植入的违规、且不会
//! 把测试代码误报为生产违规），再对真实源码树断言零命中——防止守卫自身
//! 退化为恒真的空扫描。已知保留项（计划 §2.4.1/§2.4.3）：
//! - `registration_coordinator.inc.rs` 的「at least one candidate」是 ≥1 检查
//!   （允许 1 成员），与 REQ-REG-17 一致；
//! - `repository_store_parts/create.inc.rs` 的 `matching_members.len() > 1` 是
//!   物理仓身份重名冲突检查，不是 LC 成员数门禁（本守卫的词边界匹配天然
//!   不把它算作 `members.len()` 命中）。

use std::fs;
use std::path::{Path, PathBuf};

const SCAN_ROOTS: &[&str] = &["src", "web/src"];
const CODE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "mjs", "cjs"];

/// 一份被扫描的源码文件：相对路径 + 剔除测试代码后的生产行（1-based 行号）。
struct SourceFile {
    relative_path: String,
    production_lines: Vec<(usize, String)>,
}

/// 一条命中：`相对路径:行号` + 命中模式名 + 原始行内容（折叠空白便于阅读）。
struct Hit {
    location: String,
    pattern_name: &'static str,
    line: String,
}

/// 匹配策略：`Squash`（删除全部空白后做词边界子串匹配，适合代码比较式与
/// 标识符）；`PhraseCi`（空白折叠为单空格后大小写不敏感子串匹配，适合
/// 英文文案）；`PhraseCn`（删除全部空白后直接子串匹配，适合中文文案）。
#[derive(Clone, Copy)]
enum MatchKind {
    Squash,
    PhraseCi,
    PhraseCn,
}

struct Pattern {
    name: &'static str,
    needle: &'static str,
    kind: MatchKind,
}

fn is_word_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// 删除全部空白（含 CJK 全角空格），并保留每个保留字符在原文中的字节坐标
///（squash 会把 `if members` 熔合成 `ifmembers`，词边界必须回到**原坐标**
/// 判定：命中的首字符若在原文中前面隔着空白，即视为词边界成立）。
fn squash_with_origins(text: &str) -> (String, Vec<usize>) {
    let mut squashed = String::with_capacity(text.len());
    let mut origins: Vec<usize> = Vec::with_capacity(text.len());
    for (index, character) in text.char_indices() {
        if !character.is_whitespace() {
            squashed.push(character);
            origins.push(index);
        }
    }
    (squashed, origins)
}

/// 删除全部空白（squash_with_origins 的无坐标投影）。
fn squash(text: &str) -> String {
    squash_with_origins(text).0
}

/// 空白游程折叠为单个半角空格。
fn collapse(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            pending_space = false;
            normalized.push(character);
        }
    }
    normalized
}

/// 在单行内匹配一条模式；`Squash` 模式要求命中两侧不是词字符
///（`matching_members.len() > 1` 因此不会命中 `members.len()` 类针），
/// 多次出现时逐一尝试（首个出现边界不符不遮蔽后续真命中）。
fn line_matches(original: &str, pattern: &Pattern) -> bool {
    match pattern.kind {
        MatchKind::Squash => {
            let (hay, origins) = squash_with_origins(original);
            let needle = squash(pattern.needle);
            let mut search_from = 0usize;
            while let Some(offset) = hay[search_from..].find(&needle) {
                let position = search_from + offset;
                // 原坐标边界：命中的首字符在原文中的前一个字符（若有），
                // 必须是空白（如 `if members.len() == 1` 的 if 之后）或
                // 非词字符；`matching_members.len()` 的 `_` 是词字符 ⇒ 拒绝。
                let before_ok = origins.get(position).is_none_or(|&origin| {
                    original[..origin]
                        .chars()
                        .next_back()
                        .is_none_or(|c| c.is_whitespace() || !is_word_char(c))
                });
                let after_ok =
                    origins
                        .get(position + needle.len() - 1)
                        .is_none_or(|&last_origin| {
                            let last_char = original[last_origin..].chars().next().unwrap_or(' ');
                            original[last_origin + last_char.len_utf8()..]
                                .chars()
                                .next()
                                .is_none_or(|c| c.is_whitespace() || !is_word_char(c))
                        });
                if before_ok && after_ok {
                    return true;
                }
                search_from = position + needle.len().max(1);
            }
            false
        }
        MatchKind::PhraseCn => squash(original).contains(&squash(pattern.needle)),
        MatchKind::PhraseCi => collapse(original)
            .to_ascii_lowercase()
            .contains(&collapse(pattern.needle).to_ascii_lowercase()),
    }
}

fn scan(files: &[SourceFile], patterns: &[Pattern]) -> Vec<Hit> {
    let mut hits = Vec::new();
    for file in files {
        for (line_number, line) in &file.production_lines {
            for pattern in patterns {
                if line_matches(line, pattern) {
                    hits.push(Hit {
                        location: format!("{}:{}", file.relative_path, line_number),
                        pattern_name: pattern.name,
                        line: collapse(line),
                    });
                }
            }
        }
    }
    hits
}

/// 断言失败时把全部命中拼成一条可读消息（file:line + 行内容 + 命中模式）。
fn render_hits(scope: &str, hits: &[Hit]) -> String {
    let mut message = format!(
        "{scope} 命中 {} 处（REQ-REG-17 禁止的生产成员数分支/门禁）:\n",
        hits.len()
    );
    for hit in hits {
        message.push_str(&format!(
            "  {} [{}] -> {}\n",
            hit.location, hit.pattern_name, hit.line
        ));
    }
    message
}

// ---------------------------------------------------------------------------
// 源码树收集与测试代码剔除
// ---------------------------------------------------------------------------

fn is_test_path_segment(segment: &str) -> bool {
    segment == "tests"
        || segment == "__tests__"
        || segment == "fixtures"
        || segment == "test"
        || segment.ends_with("_tests")
}

fn is_test_file_name(file_name: &str) -> bool {
    // 按「tests 复数/双扩展名」判定测试文件（`*_tests.inc.rs`、
    // `*_tests_*.inc.rs`、`*.test.tsx` 等）；`test_executor.rs`、
    // `test_pause.rs` 这类「执行测试」的生产功能文件名不含复数 tests，
    // 仍是生产代码，不排除。
    let stem = file_name.split('.').next().unwrap_or(file_name);
    let lower = file_name.to_ascii_lowercase();
    stem.contains("tests")
        || stem.ends_with("_test")
        || lower.contains(".test.")
        || lower.contains(".spec.")
}

fn collect_source_files(directory: &Path, repo_root: &Path, files: &mut Vec<SourceFile>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => panic!("read {}: {error}", directory.display()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name == ".git" || name == "node_modules" || name == "target" || name == "dist" {
                continue;
            }
            if is_test_path_segment(&name) {
                continue;
            }
            collect_source_files(&path, repo_root, files);
            continue;
        }
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !CODE_EXTENSIONS.contains(&extension) || is_test_file_name(&name) {
            continue;
        }
        let relative = path
            .strip_prefix(repo_root)
            .expect("scanned path under repo root")
            .to_string_lossy()
            .replace('\\', "/");
        let raw = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("read {}: {error}", path.display());
        });
        let production_lines = strip_inline_test_code(&raw, extension == "rs");
        files.push(SourceFile {
            relative_path: relative,
            production_lines,
        });
    }
}

/// Rust 生产文件内的内联测试代码剔除：`#[cfg(test)] mod … {…}` 块与
/// `#[test]`/`#[tokio::test]` 函数体整段置空（行号保持，便于命中定位）。
/// 非 Rust 文件原样保留。花括号失衡立即 panic——宁可守卫红，不可静默漏扫。
fn strip_inline_test_code(raw: &str, is_rust: bool) -> Vec<(usize, String)> {
    let lines: Vec<&str> = raw.lines().collect();
    let mut production: Vec<(usize, String)> = Vec::with_capacity(lines.len());
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        let is_cfg_test_attr =
            trimmed.starts_with("#[cfg(test") || trimmed.starts_with("#[cfg(all(test");
        let is_test_attr = trimmed.starts_with("#[test]")
            || trimmed.starts_with("#[tokio::test]")
            || trimmed.starts_with("#[rstest]");
        if is_rust && (is_cfg_test_attr || is_test_attr) {
            let attr_start = index;
            // 向后找 item 头（mod 名 / fn 名），最多看 6 个非空行。
            let mut header = None;
            let mut probe = index + 1;
            while probe < lines.len() && probe <= index + 8 {
                let probe_trimmed = lines[probe].trim_start();
                if probe_trimmed.starts_with("mod ")
                    || probe_trimmed.starts_with("fn ")
                    || probe_trimmed.starts_with("async fn ")
                {
                    header = Some(probe);
                    break;
                }
                if !probe_trimmed.is_empty() && !probe_trimmed.starts_with("#[") {
                    break;
                }
                probe += 1;
            }
            if let Some(header_line) = header {
                let header_trimmed = lines[header_line].trim_start();
                let is_module = header_trimmed.starts_with("mod ");
                let header_ends_with_semicolon =
                    header_trimmed.ends_with(';') && !header_trimmed.ends_with("{};");
                if !is_module || is_cfg_test_attr {
                    if header_ends_with_semicolon {
                        // `#[cfg(test)] mod tests;`（外部模块声明，无体内花括号）：
                        // 只置空属性行与声明行——测试体在独立的 tests.rs 里，
                        // 该文件按测试命名规则被路径排除。
                        for blank in attr_start..=header_line {
                            production.push((blank + 1, String::new()));
                        }
                        index = header_line + 1;
                        continue;
                    }
                    // 从属性行起整段置空到 item 体的配对右花括号；配对失败
                    // （体内字符串/宏含非常规花括号）时 fail-open：整段保留
                    // 为生产代码参与扫描——宁可多扫报红，不可静默漏扫。
                    if let Some(end) = block_end(&lines, header_line) {
                        for blank in attr_start..=end {
                            production.push((blank + 1, String::new()));
                        }
                        index = end + 1;
                        continue;
                    }
                }
            }
        }
        production.push((index + 1, line.to_string()));
        index += 1;
    }
    production
}

/// 从 `header` 行（含 item 头）起找到 item 体的配对右花括号所在行号。
fn block_end(lines: &[&str], header: usize) -> Option<usize> {
    let mut depth: i64 = 0;
    let mut seen_open = false;
    for (offset, line) in lines.iter().enumerate().skip(header) {
        for character in line.chars() {
            match character {
                '{' => {
                    depth += 1;
                    seen_open = true;
                }
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        return None;
                    }
                    if seen_open && depth == 0 {
                        // enumerate().skip(header) 的 offset 已是绝对行号。
                        return Some(offset);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_production_files() -> Vec<SourceFile> {
    let root = repo_root();
    let mut files = Vec::new();
    for scan_root in SCAN_ROOTS {
        let path = root.join(scan_root);
        assert!(path.is_dir(), "scan root {} must exist", path.display());
        collect_source_files(&path, &root, &mut files);
    }
    assert!(
        files.len() > 100,
        "scanner must collect a real source tree, got {} files",
        files.len()
    );
    files
}

/// 阴性对照样本：在临时目录里搭一棵合成源码树，把同一段违规代码分别放进
/// ① 生产文件；② `tests/` 测试目录文件；③ 生产文件的 `#[cfg(test)] mod`
/// 内联模块；④ 测试命名文件（`*_tests.rs`）。走**真实的** collect +
/// strip 管线后，守卫必须且只能检出 ①。
fn synthetic_tree_files() -> Vec<SourceFile> {
    let temp = tempfile::tempdir().expect("synthetic tree tempdir");
    let violation = "fn gated(members: &[String]) -> bool {\n    if members.len() == 1 { return true; }\n    members.len() >= 2\n}\nconst COPY_CN: &str = \"至少 2 个成员\";\nconst COPY_EN: &str = \"requires at least 2 members\";\n";
    std::fs::write(temp.path().join("gate.rs"), violation).expect("write production gate");
    let tests_dir = temp.path().join("tests");
    std::fs::create_dir_all(&tests_dir).expect("create tests dir");
    std::fs::write(tests_dir.join("skipped.rs"), violation).expect("write test-only file");
    std::fs::write(temp.path().join("helpers_tests.rs"), violation).expect("write *_tests.rs file");
    std::fs::write(
        temp.path().join("host.rs"),
        "fn production() -> u8 { 7 }\n#[cfg(test)]\nmod tests {\n    fn inner(members: &[String]) -> bool {\n        members.len() == 1\n    }\n}\n",
    )
    .expect("write inline-test host");
    let mut files = Vec::new();
    collect_source_files(temp.path(), temp.path(), &mut files);
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let names: Vec<&str> = files
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["gate.rs", "host.rs"],
        "测试目录与 *_tests.rs 文件必须被路径排除",
    );
    files
}

fn assert_negative_control(patterns: &[Pattern]) {
    let detected = scan(&synthetic_tree_files(), patterns);
    assert!(
        detected
            .iter()
            .any(|hit| hit.location.starts_with("gate.rs:")),
        "阴性对照失败：守卫未能从合成生产文件检出已植入的成员数分支/门禁/文案（扫描器退化为恒真空扫）"
    );
    for hit in &detected {
        assert!(
            hit.location.starts_with("gate.rs:"),
            "阴性对照失败：测试目录/测试命名文件/内联 #[cfg(test)] 模块被误报为生产违规 -> {} [{}] {}",
            hit.location,
            hit.pattern_name,
            hit.line
        );
    }
}

// ---------------------------------------------------------------------------
// T4.1~T4.3 模式表
// ---------------------------------------------------------------------------

/// T4.1：不得存在「成员数为 1」的专门生产分支（含等价标识符形态）。
const SINGLE_MEMBER_BRANCH_PATTERNS: &[Pattern] = &[
    Pattern {
        name: "members.len() == 1",
        needle: "members.len() == 1",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "member_ids.len() == 1",
        needle: "member_ids.len() == 1",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "member_count == 1",
        needle: "member_count == 1",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "require_multiple_members",
        needle: "require_multiple_members",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "min_members",
        needle: "min_members",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "minimum_member",
        needle: "minimum_member",
        kind: MatchKind::Squash,
    },
];

/// T4.2：不得存在「≥2 成员才允许某操作」的门禁条件/常量（`>1` 是 `>=2` 的
/// 等价写法；词边界保证物理仓身份重名检查 `matching_members.len() > 1`
/// （计划 §2.4.3 明确保留）不被误报）。
const TWO_MEMBER_GATE_PATTERNS: &[Pattern] = &[
    Pattern {
        name: "members.len() >= 2",
        needle: "members.len() >= 2",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "members.len() > 1",
        needle: "members.len() > 1",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "members.len() < 2",
        needle: "members.len() < 2",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "member_ids.len() >= 2",
        needle: "member_ids.len() >= 2",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "member_ids.len() > 1",
        needle: "member_ids.len() > 1",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "member_ids.len() < 2",
        needle: "member_ids.len() < 2",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "member_count >= 2",
        needle: "member_count >= 2",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "member_count > 1",
        needle: "member_count > 1",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "member_count < 2",
        needle: "member_count < 2",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "multiple_members_required",
        needle: "multiple_members_required",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "requires_multiple_members",
        needle: "requires_multiple_members",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "min_member_count",
        needle: "min_member_count",
        kind: MatchKind::Squash,
    },
    Pattern {
        name: "minimum_member_count",
        needle: "minimum_member_count",
        kind: MatchKind::Squash,
    },
];

/// T4.3：不得出现拒绝 1 成员 LC 的门禁文案/常量（中英文用户可见串）。
const MEMBER_COUNT_REJECTION_PHRASES: &[Pattern] = &[
    Pattern {
        name: "至少 2 个成员",
        needle: "至少 2 个成员",
        kind: MatchKind::PhraseCn,
    },
    Pattern {
        name: "至少两个成员",
        needle: "至少两个成员",
        kind: MatchKind::PhraseCn,
    },
    Pattern {
        name: "至少 2 个仓库成员",
        needle: "至少 2 个仓库成员",
        kind: MatchKind::PhraseCn,
    },
    Pattern {
        name: "成员数至少",
        needle: "成员数至少",
        kind: MatchKind::PhraseCn,
    },
    Pattern {
        name: "成员数不足",
        needle: "成员数不足",
        kind: MatchKind::PhraseCn,
    },
    Pattern {
        name: "至少需要 2 个",
        needle: "至少需要 2 个",
        kind: MatchKind::PhraseCn,
    },
    Pattern {
        name: "at least 2 member",
        needle: "at least 2 member",
        kind: MatchKind::PhraseCi,
    },
    Pattern {
        name: "at least two member",
        needle: "at least two member",
        kind: MatchKind::PhraseCi,
    },
    Pattern {
        name: "requires multiple members",
        needle: "requires multiple members",
        kind: MatchKind::PhraseCi,
    },
    Pattern {
        name: "multiple members are required",
        needle: "multiple members are required",
        kind: MatchKind::PhraseCi,
    },
    Pattern {
        name: "member count must be at least",
        needle: "member count must be at least",
        kind: MatchKind::PhraseCi,
    },
];

// ---------------------------------------------------------------------------
// T4.1~T4.3
// ---------------------------------------------------------------------------

/// T4.1：生产代码零「1 成员专属分支」。
#[test]
fn no_production_branch_special_cases_single_member() {
    assert_negative_control(SINGLE_MEMBER_BRANCH_PATTERNS);
    let hits = scan(&repo_production_files(), SINGLE_MEMBER_BRANCH_PATTERNS);
    assert!(hits.is_empty(), "{}", render_hits("T4.1", &hits));
}

/// T4.2：生产代码零「≥2 成员门禁」（条件/常量）。
#[test]
fn no_gate_requires_at_least_two_members() {
    assert_negative_control(TWO_MEMBER_GATE_PATTERNS);
    let hits = scan(&repo_production_files(), TWO_MEMBER_GATE_PATTERNS);
    assert!(hits.is_empty(), "{}", render_hits("T4.2", &hits));
}

/// T4.3：静态扫描不得出现「至少 2 个成员」类拒绝串（文案/常量）。
#[test]
fn single_member_lc_is_not_rejected_anywhere() {
    assert_negative_control(MEMBER_COUNT_REJECTION_PHRASES);
    let hits = scan(&repo_production_files(), MEMBER_COUNT_REJECTION_PHRASES);
    assert!(hits.is_empty(), "{}", render_hits("T4.3", &hits));
}
