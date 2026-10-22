//! 逐仓实况（member reality）——多仓逻辑代码库专属。
//!
//! REQ-PLN-14：多仓评审材料 MUST 为每个成员仓渲染固定实况行——`职责`、`关键目录`、
//! `入口`、`构建/测试命令`、`基线` 各一行；无法识别的项 MUST 写「未识别」并保留该行，
//! MUST NOT 省略行或整仓。
//!
//! 逐仓基线（设计 A 条目 16）：取该仓 checkout 当前 HEAD 所在分支（`git rev-parse
//! --abbrev-ref HEAD`），MUST NOT 猜 `main` / `master`；detached HEAD / 空仓 → 该仓
//! 「基线未识别」并附诊断，其余仓继续渲染（该仓失败不终止整轮）。
//!
//! 单仓（单成员）代码库不注入本模块：装配点在 lc 专属 resolver 之后（条目 6 已定
//! 「单仓不注入成员材料」）。

use crate::product::issue_baseline::resolve_member_head_branch;
use std::path::Path;

/// 不可识别项的统一文案（缺项 MUST 保留该行并写「未识别」）。
pub const MEMBER_REALITY_UNKNOWN: &str = "未识别";

/// 逐仓实况段标题（材料块内的独立小节）。
pub const MEMBER_REALITY_SECTION_MARKER: &str = "## 逐仓实况";

/// 关键目录上限（≤12 项）。
pub const MEMBER_REALITY_MAX_KEY_DIRECTORIES: usize = 12;

/// 入口上限（≤6 项）。
pub const MEMBER_REALITY_MAX_ENTRYPOINTS: usize = 6;

/// 分层目录摘要的顶层上限（顶层 ≤12 项）。
pub const MEMBER_REALITY_MAX_TOP_DIRECTORIES: usize = 12;

/// 分层目录摘要的每个顶层目录一级子项上限（≤6 项）。
pub const MEMBER_REALITY_MAX_CHILD_DIRECTORIES: usize = 6;

/// 分层目录摘要的总行数上限（≤40 行）。
pub const MEMBER_REALITY_MAX_LAYERED_LINES: usize = 40;

/// 单仓实况字节上限（2 KiB）——实况过大时按项裁剪，绝不整仓省略。
pub const MEMBER_REALITY_MAX_BYTES: usize = 2 * 1024;

/// 单个成员仓的实况摘要。
///
/// 字段缺失一律以 `MEMBER_REALITY_UNKNOWN`（基线为 `基线未识别(<诊断>)`）表达——
/// MUST NOT 省略字段，MUST NOT 省略整仓。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberReality {
    pub member_id: String,
    pub role: String,
    pub key_directories: Vec<String>,
    pub entrypoints: Vec<String>,
    pub build_test_command: String,
    pub baseline: String,
}

/// 逐仓实况采集（只读文件系统 + 一次只读 git 查询，不做代码分析）。
///
/// - `member_id` / `role` 取自 `RepositoryContextSet`（`role` 为空 → 「未识别」）；
/// - `checkout_root` 为该成员仓本地 checkout 根目录；`None`（无 checkout 记录）时
///   目录/入口/命令/基线全部标「未识别」，MUST NOT 影响其余成员；
/// - 任何单仓失败（目录不存在、git 不可用、detached HEAD、空仓）都只降级本仓，
///   MUST NOT 终止整轮渲染。
pub fn collect_member_reality(
    member_id: &str,
    role: &str,
    checkout_root: Option<&Path>,
) -> MemberReality {
    let role = role.trim();
    let mut reality = MemberReality {
        member_id: member_id.to_string(),
        role: if role.is_empty() {
            MEMBER_REALITY_UNKNOWN.to_string()
        } else {
            role.to_string()
        },
        key_directories: Vec::new(),
        entrypoints: Vec::new(),
        build_test_command: MEMBER_REALITY_UNKNOWN.to_string(),
        baseline: MEMBER_REALITY_UNKNOWN.to_string(),
    };
    let Some(checkout_root) = checkout_root else {
        reality.baseline = "基线未识别(该成员无 checkout 记录)".to_string();
        return reality;
    };
    reality.key_directories = summarize_key_directories(checkout_root);
    reality.entrypoints = detect_entrypoints(checkout_root);
    reality.build_test_command = detect_build_test_command(checkout_root);
    reality.baseline = match resolve_member_head_branch(checkout_root) {
        Ok(branch) => branch,
        Err(diagnosis) => format!("基线未识别({diagnosis})"),
    };
    reality
}

/// 渲染逐仓实况（每仓固定 5 行）。
///
/// 行格式：`<标签>: <值>（成员 <member_id>）`；标签固定为 `职责` / `关键目录` / `入口` /
/// `构建/测试命令` / `基线`，MUST NOT 省略任何一行。单仓实况字节上限 2 KiB——
/// 超出时按项裁剪（列表项先裁，行绝不省略）。
pub fn render_member_reality(members: &[MemberReality]) -> String {
    let mut rendered = String::new();
    for member in members {
        let mut key_directories = member.key_directories.clone();
        let mut entrypoints = member.entrypoints.clone();
        let mut role = member.role.clone();
        let mut baseline = member.baseline.clone();
        loop {
            let block = render_member_lines(
                &member.member_id,
                &role,
                &key_directories,
                &entrypoints,
                &member.build_test_command,
                &baseline,
            );
            if block.len() <= MEMBER_REALITY_MAX_BYTES {
                rendered.push_str(&block);
                break;
            }
            if !key_directories.is_empty() {
                key_directories.pop();
                continue;
            }
            if !entrypoints.is_empty() {
                entrypoints.pop();
                continue;
            }
            if role.len() > 64 {
                role.truncate(32);
                continue;
            }
            if baseline.len() > 64 {
                baseline.truncate(32);
                continue;
            }
            rendered.push_str(&block);
            break;
        }
    }
    rendered
}

/// 渲染单仓 5 行实况。
fn render_member_lines(
    member_id: &str,
    role: &str,
    key_directories: &[String],
    entrypoints: &[String],
    build_test_command: &str,
    baseline: &str,
) -> String {
    let mut block = String::new();
    block.push_str(&format!(
        "职责: {}（成员 {}）\n",
        value_or_unknown(role),
        member_id
    ));
    block.push_str(&format!(
        "关键目录: {}（成员 {}）\n",
        list_or_unknown(key_directories),
        member_id
    ));
    block.push_str(&format!(
        "入口: {}（成员 {}）\n",
        list_or_unknown(entrypoints),
        member_id
    ));
    block.push_str(&format!(
        "构建/测试命令: {}（成员 {}）\n",
        value_or_unknown(build_test_command),
        member_id
    ));
    block.push_str(&format!(
        "基线: {}（成员 {}）\n",
        value_or_unknown(baseline),
        member_id
    ));
    block
}

/// 关键目录摘要（分层摘要，≤12 项；只读目录项，不做代码分析）。
///
/// 分层含义：顶层目录 + 其一级子目录（`顶层/子项`）。超过 12 项时按顶层优先裁剪。
pub fn summarize_key_directories(root: &Path) -> Vec<String> {
    summarize_layered_directories(root)
        .into_iter()
        .take(MEMBER_REALITY_MAX_KEY_DIRECTORIES)
        .collect()
}

/// 分层目录摘要（顶层 ≤12 项 + 每个顶层目录一级子项 ≤6，总 ≤40 行）。
///
/// Work Item splitter 的多仓目录摘要复用本实现（设计 A 条目 14）。
pub fn summarize_layered_directories(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut top_level: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_noise_directory(&name) {
            continue;
        }
        top_level.push((name, entry.path()));
    }
    top_level.sort();
    top_level.truncate(MEMBER_REALITY_MAX_TOP_DIRECTORIES);
    let mut layered: Vec<String> = Vec::new();
    for (name, path) in top_level {
        layered.push(format!("{name}/"));
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };
        let mut child_names: Vec<String> = Vec::new();
        for child in children.flatten() {
            let Ok(child_type) = child.file_type() else {
                continue;
            };
            if !child_type.is_dir() {
                continue;
            }
            let child_name = child.file_name().to_string_lossy().into_owned();
            if is_noise_directory(&child_name) {
                continue;
            }
            child_names.push(format!("{name}/{child_name}/"));
        }
        child_names.sort();
        child_names.truncate(MEMBER_REALITY_MAX_CHILD_DIRECTORIES);
        layered.extend(child_names);
        if layered.len() >= MEMBER_REALITY_MAX_LAYERED_LINES {
            break;
        }
    }
    layered.truncate(MEMBER_REALITY_MAX_LAYERED_LINES);
    layered
}

/// 入口文件探测（根下构建/入口清单文件 ≤6 项）。
pub fn detect_entrypoints(root: &Path) -> Vec<String> {
    const CANDIDATES: [&str; 11] = [
        "Cargo.toml",
        "package.json",
        "go.mod",
        "pom.xml",
        "Makefile",
        "build.gradle",
        "pyproject.toml",
        "requirements.txt",
        "composer.json",
        "Gemfile",
        "CMakeLists.txt",
    ];
    CANDIDATES
        .iter()
        .filter(|name| root.join(name).is_file())
        .map(|name| (*name).to_string())
        .take(MEMBER_REALITY_MAX_ENTRYPOINTS)
        .collect()
}

/// 构建/测试命令判定（可判定则给出命令，否则「未识别」）。
pub fn detect_build_test_command(root: &Path) -> String {
    if root.join("Cargo.toml").is_file() {
        "cargo test".to_string()
    } else if root.join("package.json").is_file() {
        "npm test".to_string()
    } else if root.join("pom.xml").is_file() {
        "mvn test".to_string()
    } else {
        MEMBER_REALITY_UNKNOWN.to_string()
    }
}

/// 噪声目录（构建产物 / 依赖缓存）不计入关键目录。
fn is_noise_directory(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".idea" | ".vscode" | "node_modules" | "target" | "__pycache__" | ".venv"
    )
}

/// 空值兜底为「未识别」（MUST 保留该行）。
fn value_or_unknown(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        MEMBER_REALITY_UNKNOWN.to_string()
    } else {
        trimmed.to_string()
    }
}

/// 空列表兜底为「未识别」（MUST 保留该行）。
fn list_or_unknown(values: &[String]) -> String {
    if values.is_empty() {
        return MEMBER_REALITY_UNKNOWN.to_string();
    }
    values.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn rust_member() -> MemberReality {
        MemberReality {
            member_id: "member-rust".to_string(),
            role: "后端服务".to_string(),
            key_directories: vec!["src".to_string(), "tests".to_string()],
            entrypoints: vec!["Cargo.toml".to_string()],
            build_test_command: "cargo test".to_string(),
            baseline: "feature/rust".to_string(),
        }
    }

    fn web_member() -> MemberReality {
        MemberReality {
            member_id: "member-web".to_string(),
            role: "前端应用".to_string(),
            key_directories: vec!["src".to_string()],
            entrypoints: vec!["package.json".to_string()],
            build_test_command: "npm test".to_string(),
            baseline: "feature/web".to_string(),
        }
    }

    #[test]
    fn member_reality_renders_five_fixed_lines_per_member() {
        // REQ-PLN-14：每仓固定 5 行（职责 / 关键目录 / 入口 / 构建/测试命令 / 基线）。
        let rendered = render_member_reality(&[rust_member(), web_member()]);

        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(
            lines.len(),
            10,
            "两仓必须各渲染 5 行固定实况行：\n{rendered}"
        );
        for label in ["职责", "关键目录", "入口", "构建/测试命令", "基线"] {
            let needle = format!("{label}:");
            assert_eq!(
                lines.iter().filter(|line| line.contains(&needle)).count(),
                2,
                "「{label}」必须每仓一行：\n{rendered}"
            );
        }
        assert!(
            rendered.contains("职责: 后端服务（成员 member-rust）"),
            "{rendered}"
        );
        assert!(
            rendered.contains("基线: feature/rust（成员 member-rust）"),
            "{rendered}"
        );
        assert!(
            rendered.contains("构建/测试命令: npm test（成员 member-web）"),
            "{rendered}"
        );
    }

    #[test]
    fn member_reality_marks_unknown_items_without_dropping_lines() {
        // 缺项 MUST 写「未识别」并保留该行，MUST NOT 省略行或整仓。
        let mut unknown = rust_member();
        unknown.role = String::new();
        unknown.key_directories = Vec::new();
        unknown.entrypoints = Vec::new();
        unknown.build_test_command = String::new();
        unknown.baseline = String::new();

        let rendered = render_member_reality(&[unknown]);

        assert_eq!(rendered.lines().count(), 5, "{rendered}");
        for label in ["职责", "关键目录", "入口", "构建/测试命令", "基线"] {
            assert!(
                rendered.contains(&format!("{label}: 未识别")),
                "缺项必须显「未识别」：{rendered}"
            );
        }
        assert_eq!(
            rendered
                .lines()
                .filter(|line| line.contains("未识别"))
                .count(),
            5,
            "五个缺项都必须显「未识别」：{rendered}"
        );
    }

    #[test]
    fn member_reality_collects_role_dirs_entrypoints_and_command() {
        let dir = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(dir.path().join("src")).expect("mkdir src");
        std::fs::create_dir_all(dir.path().join("tests")).expect("mkdir tests");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").expect("write Cargo.toml");

        let reality = collect_member_reality("api", "后端服务", Some(dir.path()));

        assert_eq!(reality.role, "后端服务");
        assert!(
            reality.key_directories.iter().any(|item| item == "src/"),
            "关键目录必须含 src：{:?}",
            reality.key_directories
        );
        assert!(
            reality.entrypoints.iter().any(|item| item == "Cargo.toml"),
            "入口必须含 Cargo.toml：{:?}",
            reality.entrypoints
        );
        assert_eq!(reality.build_test_command, "cargo test");
        // 非 git 目录 → 基线未识别（该仓失败不影响其余仓）。
        assert!(
            reality.baseline.starts_with("基线未识别("),
            "非 git 目录基线必须未识别：{}",
            reality.baseline
        );
    }

    #[test]
    fn member_reality_detached_head_marks_baseline_unknown_only_for_that_member() {
        let dir = TempDir::new().expect("temp repo");
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .stdin(std::process::Stdio::null())
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git {args:?}");
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test User"]);
        std::fs::write(dir.path().join("README.md"), "base\n").expect("write readme");
        run(&["add", "README.md"]);
        run(&["commit", "-m", "base"]);
        run(&["checkout", "--detach", "HEAD"]);

        let detached = collect_member_reality("api", "后端服务", Some(dir.path()));
        assert!(
            detached.baseline.starts_with("基线未识别("),
            "detached HEAD 必须标基线未识别：{}",
            detached.baseline
        );

        let healthy = rust_member();
        let rendered = render_member_reality(&[detached, healthy]);
        assert!(rendered.contains("基线未识别"), "{rendered}");
        assert!(
            rendered.contains("基线: feature/rust（成员 member-rust）"),
            "其余仓必须继续渲染：{rendered}"
        );
    }

    #[test]
    fn member_reality_without_checkout_marks_all_unknown() {
        let reality = collect_member_reality("api", "", None);
        assert_eq!(reality.role, MEMBER_REALITY_UNKNOWN);
        assert!(reality.key_directories.is_empty());
        assert!(reality.entrypoints.is_empty());
        assert_eq!(reality.build_test_command, MEMBER_REALITY_UNKNOWN);
        assert!(reality.baseline.starts_with("基线未识别("));
    }

    #[test]
    fn member_reality_block_stays_within_two_kib() {
        // 单仓实况 ≤ 2 KiB：超限时按项裁剪，行绝不省略。
        let mut heavy = rust_member();
        heavy.key_directories = (0..40)
            .map(|index| format!("very-long-directory-name-{index}/"))
            .collect();
        heavy.entrypoints = (0..6).map(|index| format!("entrypoint-{index}.bin")).collect();

        let rendered = render_member_reality(&[heavy]);

        assert_eq!(rendered.lines().count(), 5, "{rendered}");
        assert!(
            rendered.len() <= MEMBER_REALITY_MAX_BYTES,
            "单仓实况必须 ≤ 2 KiB，实际 {} 字节",
            rendered.len()
        );
    }
}
