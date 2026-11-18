//! Aria 源码口径 + 有界盘检扫描器。
//!
//! 用途：判定「成员主 checkout 的磁盘上是否存在 Aria 口径的源码」，供聚合索引
//! 验收阶段区分「有源码却没被索引」（漏收 ⇒ 失败）与「没有源码」（⇒ 跳过）。
//!
//! 口径：**源码 = git 可见范围 ∩ `ARIA_SOURCE_EXTENSIONS` ∩ 非 Aria 自身排除**。
//!
//! - **可见范围**：`git ls-files --cached --others --exclude-standard`（跟踪的 +
//!   未跟踪且未被忽略的）⇒ 被 `.gitignore` 忽略的文件**不算源码**。
//! - **排除口径**：复用 `exclude.rs` 的 `BUILTIN_EXCLUDES`（Aria 排除集 =
//!   codegraph 排除集，同源）：目录级剪枝 + 4 条非目录模式在文件级同样扣除。
//! - **非 git 仓**：回落「该根下全部文件」（非 git 目录没有 ignore 语义）；
//!   git 不可用 / 命令失败 / 超时 / 取消 ⇒ **无法确认**（不得回落成全部文件）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};

use super::exclude::BUILTIN_EXCLUDES;
use super::AggregateIndexError;

/// `git` 可见范围探测的超时与输出上限。
const GIT_TIMEOUT_SECS: u64 = 10;
const GIT_OUTPUT_LIMIT_BYTES: usize = 4 * 1024 * 1024;

/// Aria 自有源码后缀清单（ASCII 小写，不含点；44 项）。
///
/// **校准基线版本 = codegraph 1.6.3**（`codegraph --version` 原始输出与逐后缀
/// 实测收录表见 `cadence/notes/2026-11-18_校准_CodeGraph源码后缀清单_1.6.3_v1.0.md`）。
///
/// **测量命令（可复现）**：临时目录里每个候选后缀放一个**该语言的最小有效样本**
/// → `codegraph init .` → `codegraph files --json` → 出现在收录结果里的后缀 =
/// 1.6.3 会收录的后缀；本清单取其**子集**（只含我们希望当源码看待的常见语言；
/// 配置/文档格式与冷门语言不纳入）。
///
/// **复校准步骤**：换 codegraph 版本后 ⇒ 重跑上面的测量 ⇒ 更新本清单 ⇒
/// 更新本注释的基线版本 ⇒ 在校准 notes 文件追加一行新基线 ⇒ 同步更新
/// `source_scan` 的红测试期望。
///
/// **版本漂移的后果**：新版若不再收录本清单内某个后缀，该类仓会被判成
/// 「源码存在却未被收录」（漏收）而失败；届时按上面的复校准步骤更新本清单。
/// 运行期遇到版本漂移**只记可审计告警、不阻断索引构建**。
pub const ARIA_SOURCE_EXTENSIONS: &[&str] = &[
    "astro", "c", "cc", "cpp", "cs", "cshtml", "cjs", "cts", "cu", "cuh", "cxx", "dart", "go", "h",
    "hpp", "hxx", "java", "js", "jsx", "kt", "kts", "lua", "luau", "m", "mjs", "mm", "mts", "php",
    "py", "pyw", "r", "rake", "rb", "razor", "rs", "scala", "sc", "sol", "svelte", "swift", "ts",
    "tsx", "vb", "vue",
];

/// 物理扫描段（`.worktrees/` 诊断）的深度上限（根深度 = 0）。
pub const SOURCE_SCAN_MAX_DEPTH: usize = 32;
/// 单成员条目上限（独立于整批上限）。
pub const SOURCE_SCAN_MAX_ENTRIES_PER_MEMBER: usize = 20_000;
/// 整批条目上限（两阶段、所有成员共享同一计数）。
pub const SOURCE_SCAN_MAX_ENTRIES_PER_BATCH: usize = 100_000;
/// 整批盘检时限。
pub const SOURCE_SCAN_DEADLINE: Duration = Duration::from_secs(2);

/// 单个成员的源码判定结果（四值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberSourceVerdict {
    /// 可见范围的普通路径上存在 Aria 源码。
    SourceInNormalPath,
    /// 普通路径干净，但 `.worktrees/` 子树内存在 Aria 源码。
    SourceOnlyInWorktrees,
    /// 两段都完成且都没有 Aria 源码。
    NoSource,
    /// 无法确认（git 不可用 / 权限 / IO / 深度 / 条目数 / 超时 / 取消 / 预算不足）。
    Unknown,
}

/// 单段扫描的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageOutcome {
    /// 找到源码候选（正证据）。
    Found,
    /// 该段完整走完且没有候选。
    Clean,
    /// 该段未能完整走完（权限 / IO / 深度 / 条目数 / 超时 / 取消 / 预算不足）。
    Incomplete,
}

/// 可注入预算：batch 级共享「剩余条目 + deadline + 取消信号」，测试不依赖真实 sleep。
pub struct SourceScanBudget {
    remaining_entries: usize,
    deadline: Option<Instant>,
    cancelled: AtomicBool,
}

impl SourceScanBudget {
    /// 生产口径：整批 10 万条目 + 2 秒时限。
    pub fn new_batch() -> Self {
        Self {
            remaining_entries: SOURCE_SCAN_MAX_ENTRIES_PER_BATCH,
            deadline: Some(Instant::now() + SOURCE_SCAN_DEADLINE),
            cancelled: AtomicBool::new(false),
        }
    }

    /// 测试口径：显式给定剩余条目数与 deadline（`None` = 不限时）。
    pub fn for_tests(remaining_entries: usize, deadline: Option<Instant>) -> Self {
        Self {
            remaining_entries,
            deadline,
            cancelled: AtomicBool::new(false),
        }
    }

    /// 置位取消信号。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// 剩余条目数（整批共享计数的可观测口径）。
    pub fn remaining_entries(&self) -> usize {
        self.remaining_entries
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn deadline_passed(&self) -> bool {
        self.deadline.is_some_and(|deadline| Instant::now() > deadline)
    }

    /// 预算耗尽（取消 / 超时 / 条目用尽）⇒ 该段不再可信。
    fn exhausted(&self) -> bool {
        self.is_cancelled() || self.deadline_passed() || self.remaining_entries == 0
    }

    /// 消耗一个条目；预算耗尽时返回 `false`。
    fn consume_entry(&mut self) -> bool {
        if self.remaining_entries == 0 {
            return false;
        }
        self.remaining_entries -= 1;
        true
    }
}

/// 成员主 checkout 的「git 可见范围」提供者。
///
/// 可见范围 = `git ls-files --cached --others --exclude-standard` 的输出
/// （跟踪的 + 未跟踪且未被忽略的）—— 与 codegraph 的收录口径一致。
pub trait MemberSourceVisibility: Send + Sync {
    /// 返回该成员主 checkout 内 git 可见的文件（相对成员根的路径）。
    ///
    /// **非 git 仓** ⇒ 返回该根下的全部文件（非 git 目录没有 ignore 语义）。
    /// git 不可用 / 命令失败 / 超时 / 取消 ⇒ `Err`（⇒ 该成员落 `Unknown`，
    /// **不得**当作「无源码」）。
    fn visible_files(&self, member_root: &Path) -> Result<Vec<PathBuf>, AggregateIndexError>;
}

/// 生产实现：经 `BoundedCommandRunner` 跑 `git`。
pub struct GitVisibleFiles {
    runner: Arc<dyn BoundedCommandRunner>,
}

impl GitVisibleFiles {
    /// 生产构造：自建 `TokioBoundedCommandRunner`（`snapshot.rs` 的 runner 字段
    /// 私有且无访问器，故此处自建，**不改** `snapshot.rs`）。
    pub fn production() -> Self {
        Self {
            runner: Arc::new(TokioBoundedCommandRunner),
        }
    }

    /// 测试构造：注入 runner 桩。
    pub fn with_runner(runner: Arc<dyn BoundedCommandRunner>) -> Self {
        Self { runner }
    }

    /// 跑一条 `git` 命令（平台受信禁桥参数 + 净化环境 + 超时 + 输出上限）。
    fn git(
        &self,
        cwd: &Path,
        argv: &[&str],
    ) -> Result<BoundedCommandResult, AggregateIndexError> {
        // 平台受信启动面：argv 前置禁桥参数（与 cross_cutting::git_command
        // 同源），避免仓库 hooks / fsmonitor 探针借平台进程自动执行。
        let mut full_argv: Vec<String> = crate::cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS
            .iter()
            .map(|argument| (*argument).to_string())
            .collect();
        full_argv.extend(argv.iter().map(|argument| (*argument).to_string()));
        let request = BoundedCommandRequest {
            executable: "git".to_string(),
            argv: full_argv,
            working_dir: cwd.to_path_buf(),
            timeout: Duration::from_secs(GIT_TIMEOUT_SECS),
            cancellation: CancellationToken::new(),
            environment: sanitized_environment(),
            stdout_limit: GIT_OUTPUT_LIMIT_BYTES,
            stderr_limit: GIT_OUTPUT_LIMIT_BYTES,
        };
        let runner = self.runner.clone();
        std::thread::spawn(move || -> Result<BoundedCommandResult, AggregateIndexError> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| AggregateIndexError::Failed {
                    code: "aggregate_index_source_scan_git_runtime",
                    message: error.to_string(),
                })?;
            runtime
                .block_on(runner.run(request))
                .map_err(map_runner_error)
        })
        .join()
        .map_err(|_| AggregateIndexError::Failed {
            code: "aggregate_index_source_scan_git_panic",
            message: "bounded Git runner thread panicked".to_string(),
        })?
    }
}

impl MemberSourceVisibility for GitVisibleFiles {
    fn visible_files(&self, member_root: &Path) -> Result<Vec<PathBuf>, AggregateIndexError> {
        let probe = self.git(member_root, &["rev-parse", "--is-inside-work-tree"])?;
        if probe.timed_out {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_git_timeout",
                message: "git rev-parse --is-inside-work-tree timed out".to_string(),
            });
        }
        if probe.cancelled {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_git_cancelled",
                message: "git rev-parse --is-inside-work-tree was cancelled".to_string(),
            });
        }
        if probe.exit_code != Some(0) {
            // 只有「确定不是 git 仓」才回落全部文件；其它失败一律「无法确认」。
            let stderr = probe.stderr.to_lowercase();
            if probe.exit_code == Some(128) && stderr.contains("not a git repository") {
                return collect_all_files(member_root);
            }
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_git_failed",
                message: format!(
                    "git rev-parse --is-inside-work-tree exited with {:?}",
                    probe.exit_code
                ),
            });
        }
        let listing = self.git(member_root, &["ls-files", "--cached", "--others", "--exclude-standard"])?;
        if listing.timed_out || listing.cancelled || listing.exit_code != Some(0) {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_git_failed",
                message: format!(
                    "git ls-files --cached --others --exclude-standard exited with {:?}",
                    listing.exit_code
                ),
            });
        }
        Ok(listing
            .stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(PathBuf::from)
            .collect())
    }
}

/// 排除规则（从 `BUILTIN_EXCLUDES` 派生，**不另写第二份字面量表**）。
struct ExcludeRules {
    /// 目录规则：目录名精确匹配 ⇒ 整个目录剪枝。
    directories: Vec<String>,
    /// 非目录固定名模式：目录名命中 ⇒ 剪枝；文件名命中 ⇒ 不算源码候选。
    file_patterns: Vec<String>,
}

/// 从内建排除表派生排除规则（目录规则 + 非目录模式各一份）。
static EXCLUDE_RULES: LazyLock<ExcludeRules> = LazyLock::new(|| {
    let mut directories = Vec::new();
    let mut file_patterns = Vec::new();
    for rule in BUILTIN_EXCLUDES {
        if let Some(name) = rule.strip_suffix('/') {
            let name = name.strip_prefix("**/").unwrap_or(name);
            directories.push(name.to_string());
        } else {
            let name = rule.strip_prefix("**/").unwrap_or(rule);
            file_patterns.push(name.to_string());
        }
    }
    ExcludeRules {
        directories,
        file_patterns,
    }
});

fn exclude_rules() -> &'static ExcludeRules {
    &EXCLUDE_RULES
}

/// 名字匹配 glob（只支持 `*` 与 `?`）。
fn glob_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let mut p = 0usize;
    let mut n = 0usize;
    let mut star: Option<usize> = None;
    let mut star_end = 0usize;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            star_end = n;
            p += 1;
        } else if let Some(star) = star {
            p = star + 1;
            star_end += 1;
            n = star_end;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// 目录名是否命中排除规则（目录规则 + 非目录模式）。
fn is_excluded_directory_name(rules: &ExcludeRules, name: &str) -> bool {
    rules.directories.iter().any(|rule| rule == name)
        || rules.file_patterns.iter().any(|rule| glob_matches(rule, name))
}

/// 文件名是否命中排除规则（4 条非目录模式在文件级同样扣除）。
fn is_excluded_file_name(rules: &ExcludeRules, name: &str) -> bool {
    rules.file_patterns.iter().any(|rule| glob_matches(rule, name))
}

/// 相对路径是否落在排除范围（中间目录按目录规则剪枝，末段按文件规则扣除）。
fn is_excluded_relative_path(rules: &ExcludeRules, relative: &Path) -> bool {
    let components: Vec<Component<'_>> = relative
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .collect();
    let Some((last, parents)) = components.split_last() else {
        return false;
    };
    if is_excluded_file_name(rules, &last.as_os_str().to_string_lossy()) {
        return true;
    }
    parents.iter().any(|component| {
        is_excluded_directory_name(rules, &component.as_os_str().to_string_lossy())
    })
}

/// 文件名是否属于 Aria 源码清单（取最后一个扩展名，大小写不敏感）。
fn is_source_name(name: &str) -> bool {
    let Some(extension) = Path::new(name).extension() else {
        return false;
    };
    let extension = extension.to_string_lossy().to_ascii_lowercase();
    ARIA_SOURCE_EXTENSIONS.contains(&extension.as_str())
}

/// 相对路径是否指向 Aria 源码文件。
fn is_source_path(relative: &Path) -> bool {
    relative
        .file_name()
        .map(|name| is_source_name(&name.to_string_lossy()))
        .unwrap_or(false)
}

/// 枚举成员根下的全部普通文件（不含符号链接、不跟随符号链接目录）。
///
/// 这是「非 git 仓 ⇒ 回落全部文件」的实现：非 git 目录没有 ignore 语义，
/// 枚举其全部普通文件后由扫描阶段按后缀与排除规则判定。
fn collect_all_files(root: &Path) -> Result<Vec<PathBuf>, AggregateIndexError> {
    let mut files = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth > SOURCE_SCAN_MAX_DEPTH {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_incomplete",
                message: format!("directory depth limit exceeded under {}", root.display()),
            });
        }
        let entries = fs::read_dir(&directory).map_err(|error| {
            AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_io",
                message: format!("{}: {error}", directory.display()),
            }
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_io",
                message: format!("{}: {error}", directory.display()),
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                AggregateIndexError::Failed {
                    code: "aggregate_index_source_scan_io",
                    message: format!("{}: {error}", path.display()),
                }
            })?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push((path, depth + 1));
            } else if metadata.is_file() {
                if files.len() >= SOURCE_SCAN_MAX_ENTRIES_PER_BATCH {
                    return Err(AggregateIndexError::Failed {
                        code: "aggregate_index_source_scan_incomplete",
                        message: format!("file count limit exceeded under {}", root.display()),
                    });
                }
                files.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
    }
    Ok(files)
}

/// 扫描成员主 checkout 的 Aria 源码情况。
///
/// 行为表：
///
/// | 情况 | 返回值 |
/// |---|---|
/// | 可见范围（git 可见 ∩ 清单 ∩ 非排除）内找到源码 | `SourceInNormalPath`（立即停，不再扫 `.worktrees/`） |
/// | 可见范围内无源码且无错误、`.worktrees/` 子树内找到源码 | `SourceOnlyInWorktrees` |
/// | 两段都完成且都无候选 | `NoSource` |
/// | 可见范围探测失败（git 不可用）或任一段因权限/IO/深度/条目数/超时/取消/预算不足未能完成，且**没有**普通路径源码正证据 | `Unknown` |
/// | 已找到普通路径源码后才出现的错误 | 仍 `SourceInNormalPath`（正证据优先） |
pub fn scan_member_source(
    member_root: &Path,
    visibility: &dyn MemberSourceVisibility,
    budget: &mut SourceScanBudget,
) -> MemberSourceVerdict {
    unimplemented!()
}

/// 第一段：可见范围内的源码判定。
fn scan_visible_range(
    root: &Path,
    visible: &[PathBuf],
    budget: &mut SourceScanBudget,
    member_entries: &mut usize,
) -> StageOutcome {
    unimplemented!()
}

/// 第二段：`.worktrees/` 子树的物理扫描（独立诊断段，不计入普通路径判定）。
fn scan_worktrees_tree(
    root: &Path,
    budget: &mut SourceScanBudget,
    member_entries: &mut usize,
) -> StageOutcome {
    unimplemented!()
}

/// 物理扫描：递归遍历一个目录（深度受限、条目受限、不跟随符号链接）。
fn walk_directory(
    directory: &Path,
    depth: usize,
    budget: &mut SourceScanBudget,
    member_entries: &mut usize,
) -> StageOutcome {
    unimplemented!()
}

/// 净化环境（PATH-only）。
///
/// 仓内已有三份同款私有副本（`snapshot.rs:240` / `codegraph_cli.rs:345` /
/// `codegraph_coverage.rs:690`），此处沿用 `snapshot.rs:240` 的同款实现
/// （**不改**那三个文件，也不新增共享入口）。
fn sanitized_environment() -> BTreeMap<String, String> {
    std::env::var("PATH")
        .map(|path| BTreeMap::from([("PATH".to_string(), path)]))
        .unwrap_or_default()
}

/// 把 runner 错误映射成聚合索引错误。
fn map_runner_error(error: BoundedCommandError) -> AggregateIndexError {
    match error {
        BoundedCommandError::CommandMissing {
            executable,
            details,
        } => AggregateIndexError::Failed {
            code: "aggregate_index_source_scan_git_missing",
            message: format!("{executable}: {details}"),
        },
        BoundedCommandError::Io { details } => AggregateIndexError::Failed {
            code: "aggregate_index_source_scan_git_io",
            message: details,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// 全可见提供者：与生产「非 git 仓 ⇒ 回落全部文件」口径一致。
    struct AllVisible;

    impl MemberSourceVisibility for AllVisible {
        fn visible_files(
            &self,
            member_root: &Path,
        ) -> Result<Vec<PathBuf>, AggregateIndexError> {
            collect_all_files(member_root)
        }
    }

    fn write_file(root: &Path, relative: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"sample").unwrap();
    }

    fn scan(root: &Path) -> MemberSourceVerdict {
        let mut budget = SourceScanBudget::new_batch();
        scan_member_source(root, &AllVisible, &mut budget)
    }

    fn init_git_repo(root: &Path) {
        let status = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success(), "git init failed");
    }

    fn git_add(root: &Path, paths: &[&str]) {
        let status = std::process::Command::new("git")
            .args(["add", "--"])
            .args(paths)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success(), "git add failed");
    }

    fn deep_worktrees_file(root: &Path, levels: usize) -> PathBuf {
        let mut path = root.join(".worktrees").join("wt");
        for index in 0..levels {
            path = path.join(format!("d{index}"));
        }
        path.join("main.rs")
    }

    /// 1：只有配置与文档 ⇒ 无源码。
    #[test]
    fn scan_reports_no_source_when_only_config_and_docs() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "limits.json");
        write_file(root, "README.md");
        write_file(root, "deploy.yaml");
        assert_eq!(scan(root), MemberSourceVerdict::NoSource);
    }

    /// 2：清单内每个后缀都能命中源码。
    #[test]
    fn scan_reports_source_in_normal_path_for_aria_extensions() {
        for extension in ARIA_SOURCE_EXTENSIONS {
            let temp = TempDir::new().unwrap();
            let root = temp.path();
            write_file(root, &format!("src/sample.{extension}"));
            assert_eq!(
                scan(root),
                MemberSourceVerdict::SourceInNormalPath,
                "extension {extension} must be an Aria source"
            );
        }
    }

    /// 3：普通路径干净、`.worktrees/` 内有源码 ⇒ 只在工作树命中。
    #[test]
    fn scan_reports_source_only_in_worktrees_when_normal_path_clean() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "limits.json");
        write_file(root, ".worktrees/wt/src/main.rs");
        assert_eq!(scan(root), MemberSourceVerdict::SourceOnlyInWorktrees);
    }

    /// 4：普通路径命中源码后不再触碰 `.worktrees/`。
    #[test]
    fn scan_stops_at_normal_path_source_without_touching_worktrees() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "src/main.rs");
        let locked = root.join(".worktrees/locked");
        fs::create_dir_all(&locked).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        }
        assert_eq!(scan(root), MemberSourceVerdict::SourceInNormalPath);
    }

    /// 5：时限已过且无正证据 ⇒ 无法确认。
    #[test]
    fn scan_returns_unknown_when_deadline_passed_without_positive_evidence() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "src/main.rs");
        let mut budget = SourceScanBudget::for_tests(
            usize::MAX,
            Some(Instant::now() - Duration::from_secs(1)),
        );
        assert_eq!(
            scan_member_source(root, &AllVisible, &mut budget),
            MemberSourceVerdict::Unknown
        );
    }

    /// 6：条目预算耗尽且无正证据 ⇒ 无法确认（不等于无源码）。
    #[test]
    fn scan_returns_unknown_when_entry_budget_exhausted_without_positive_evidence() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "a/b/c/limits.json");
        let mut budget = SourceScanBudget::for_tests(1, None);
        assert_eq!(
            scan_member_source(root, &AllVisible, &mut budget),
            MemberSourceVerdict::Unknown
        );
    }

    /// 7：取消信号 ⇒ 无法确认。
    #[test]
    fn scan_returns_unknown_on_cancel() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "src/main.rs");
        let mut budget = SourceScanBudget::new_batch();
        budget.cancel();
        assert_eq!(
            scan_member_source(root, &AllVisible, &mut budget),
            MemberSourceVerdict::Unknown
        );
    }

    /// 8：不可读的成员根 ⇒ 无法确认（不得当成无源码）。
    #[test]
    #[cfg(unix)]
    fn scan_returns_unknown_on_unreadable_member_root() {
        use std::os::unix::fs::PermissionsExt;
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("member");
        fs::create_dir_all(&root).unwrap();
        write_file(&root, "src/main.rs");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(&root).is_ok() {
            eprintln!("skip: 当前进程仍可读该目录（root 身份），不判定权限语义");
            return;
        }
        let mut budget = SourceScanBudget::new_batch();
        assert_eq!(
            scan_member_source(&root, &AllVisible, &mut budget),
            MemberSourceVerdict::Unknown
        );
    }

    /// 9：符号链接文件与符号链接目录都不算源码。
    #[test]
    #[cfg(unix)]
    fn scan_ignores_symlinked_files_and_dirs() {
        let outside = TempDir::new().unwrap();
        write_file(outside.path(), "main.rs");
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("member");
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(outside.path().join("main.rs"), root.join("link.rs")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("linkdir")).unwrap();
        assert_eq!(scan(&root), MemberSourceVerdict::NoSource);
    }

    /// 10：指向成员根之外的符号链接目录不被跟随。
    #[test]
    #[cfg(unix)]
    fn scan_does_not_follow_symlink_escape_out_of_member_root() {
        let outside = TempDir::new().unwrap();
        write_file(outside.path(), "src/escaped.rs");
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("member");
        fs::create_dir_all(root.join("inner")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("inner/escape")).unwrap();
        assert_eq!(scan(&root), MemberSourceVerdict::NoSource);
    }

    /// 11：内建排除目录被剪枝，但同仓其它位置的源码仍命中。
    #[test]
    fn scan_prunes_builtin_excluded_directories_but_keeps_nested_source() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "node_modules/x/a.js");
        write_file(root, "target/a.rs");
        write_file(root, "build/a.py");
        write_file(root, ".aria/a.ts");
        assert_eq!(scan(root), MemberSourceVerdict::NoSource);
        write_file(root, "src/a.ts");
        assert_eq!(scan(root), MemberSourceVerdict::SourceInNormalPath);
    }

    /// 12：4 条非目录模式按目录名剪枝；排除表是单一来源。
    #[test]
    fn scan_prunes_non_directory_builtin_patterns_as_directory_names() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "credentials/src/a.rs");
        write_file(root, "secret-notes/src/b.ts");
        write_file(root, ".env.d/c.rs");
        assert_eq!(scan(root), MemberSourceVerdict::NoSource);
        assert_eq!(super::super::exclude::BUILTIN_EXCLUDES.len(), 11);
    }

    /// 13：命中非目录模式的文件不算源码候选。
    #[test]
    fn scan_ignores_source_files_matching_non_directory_builtin_patterns() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "credentials.rs");
        write_file(root, "secret_manager.ts");
        assert_eq!(scan(root), MemberSourceVerdict::NoSource);
    }

    /// 14：整批预算在成员之间共享。
    #[test]
    fn scan_batch_budget_is_shared_across_members() {
        let temp = TempDir::new().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        write_file(&first, "src/main.rs");
        write_file(&second, "src/main.rs");
        let mut budget = SourceScanBudget::for_tests(1, None);
        assert_eq!(
            scan_member_source(&first, &AllVisible, &mut budget),
            MemberSourceVerdict::SourceInNormalPath
        );
        assert_eq!(
            scan_member_source(&second, &AllVisible, &mut budget),
            MemberSourceVerdict::Unknown
        );
    }

    /// 15：单成员条目上限独立于整批上限。
    #[test]
    fn scan_enforces_per_member_entry_cap() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("many");
        fs::create_dir_all(&root).unwrap();
        for index in 0..=SOURCE_SCAN_MAX_ENTRIES_PER_MEMBER {
            fs::write(root.join(format!("f{index}.txt")), b"x").unwrap();
        }
        let mut budget = SourceScanBudget::new_batch();
        assert_eq!(
            scan_member_source(&root, &AllVisible, &mut budget),
            MemberSourceVerdict::Unknown
        );
    }

    /// 16：物理扫描段的深度上限 = 32。
    #[test]
    fn scan_enforces_depth_limit_32_on_physical_scan_stage() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let allowed = deep_worktrees_file(root, 30);
        if let Some(parent) = allowed.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&allowed, b"fn main() {}").unwrap();
        assert_eq!(scan(root), MemberSourceVerdict::SourceOnlyInWorktrees);

        let too_deep = TempDir::new().unwrap();
        let deep_root = too_deep.path();
        let deep = deep_worktrees_file(deep_root, 31);
        if let Some(parent) = deep.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&deep, b"fn main() {}").unwrap();
        assert_eq!(scan(deep_root), MemberSourceVerdict::Unknown);
    }

    /// 17：`.worktrees/` 缺失或为符号链接 ⇒ 该段无候选（不是无法确认）。
    #[test]
    #[cfg(unix)]
    fn scan_treats_missing_or_symlinked_worktrees_entry_as_no_candidate() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("member");
        fs::create_dir_all(&root).unwrap();
        write_file(&root, "limits.json");
        assert_eq!(scan(&root), MemberSourceVerdict::NoSource);

        let outside = TempDir::new().unwrap();
        write_file(outside.path(), "src/main.rs");
        std::os::unix::fs::symlink(outside.path(), root.join(".worktrees")).unwrap();
        assert_eq!(scan(&root), MemberSourceVerdict::NoSource);
    }

    /// 18：后缀匹配大小写不敏感（只看最后一段）。
    #[test]
    fn scan_matches_extension_case_insensitively_on_last_segment() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "SRC/MAIN.RS");
        assert_eq!(scan(root), MemberSourceVerdict::SourceInNormalPath);
    }

    /// 19：被 `.gitignore` 忽略的文件不算源码。
    #[test]
    fn scan_ignores_gitignored_source_files() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        init_git_repo(root);
        write_file(root, ".gitignore");
        fs::write(root.join(".gitignore"), b"sandbox/\n").unwrap();
        write_file(root, "src/tracked.ts");
        git_add(root, &["src/tracked.ts", ".gitignore"]);
        write_file(root, "sandbox/ignored.ts");
        let mut budget = SourceScanBudget::new_batch();
        assert_eq!(
            scan_member_source(root, &GitVisibleFiles::production(), &mut budget),
            MemberSourceVerdict::SourceInNormalPath
        );

        let only_ignored = TempDir::new().unwrap();
        let ignored_root = only_ignored.path();
        init_git_repo(ignored_root);
        fs::write(ignored_root.join(".gitignore"), b"sandbox/\n").unwrap();
        write_file(ignored_root, "sandbox/ignored.ts");
        let mut budget = SourceScanBudget::new_batch();
        assert_eq!(
            scan_member_source(ignored_root, &GitVisibleFiles::production(), &mut budget),
            MemberSourceVerdict::NoSource
        );
    }

    /// 20：未跟踪但未被忽略的文件在可见范围内。
    #[test]
    fn scan_reports_visible_source_that_codegraph_missed() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        init_git_repo(root);
        write_file(root, "src/visible.ts");
        let mut budget = SourceScanBudget::new_batch();
        assert_eq!(
            scan_member_source(root, &GitVisibleFiles::production(), &mut budget),
            MemberSourceVerdict::SourceInNormalPath
        );
    }

    /// 21：清单是校准 notes 的 1.6.3 实测收录集的子集，且不含配置/文档后缀。
    #[test]
    fn aria_source_extensions_are_subset_of_calibrated_1_6_3_set() {
        let notes = fs::read_to_string(
            "cadence/notes/2026-11-18_校准_CodeGraph源码后缀清单_1.6.3_v1.0.md",
        )
        .expect("calibration notes must exist");
        for extension in ARIA_SOURCE_EXTENSIONS {
            let row = format!("| .{extension} |");
            let line = notes
                .lines()
                .find(|line| line.starts_with(&row))
                .unwrap_or_else(|| panic!("{extension} missing from calibration notes"));
            assert!(
                line.ends_with("| 是 |"),
                "{extension} is not collected by codegraph 1.6.3: {line}"
            );
        }
        for excluded in ["json", "md", "toml", "txt", "csv", "html", "yaml", "yml", "xml"] {
            assert!(
                !ARIA_SOURCE_EXTENSIONS.contains(&excluded),
                "{excluded} must not be an Aria source extension"
            );
        }
    }

    /// 22：非 git 成员 ⇒ 回落全部文件。
    #[test]
    fn visibility_falls_back_to_all_files_for_non_git_member() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "src/a.ts");
        let visible = GitVisibleFiles::production()
            .visible_files(root)
            .expect("non git member falls back to all files");
        assert!(
            visible.iter().any(|path| path.ends_with("src/a.ts")),
            "fallback must list every file: {visible:?}"
        );
        let mut budget = SourceScanBudget::new_batch();
        assert_eq!(
            scan_member_source(root, &GitVisibleFiles::production(), &mut budget),
            MemberSourceVerdict::SourceInNormalPath
        );
    }

    /// 23：git 失败 ⇒ 无法确认，不得回落成「全部文件」。
    #[test]
    fn visibility_returns_err_on_git_failure_instead_of_all_files() {
        struct FailingRunner {
            timed_out: bool,
        }

        #[async_trait::async_trait]
        impl BoundedCommandRunner for FailingRunner {
            async fn run(
                &self,
                _request: BoundedCommandRequest,
            ) -> Result<BoundedCommandResult, BoundedCommandError> {
                Ok(BoundedCommandResult {
                    exit_code: Some(1),
                    stdout: String::new(),
                    stderr: "boom".to_string(),
                    timed_out: self.timed_out,
                    cancelled: false,
                    stdout_truncated: false,
                    stderr_truncated: false,
                    duration_ms: 0,
                })
            }
        }

        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, "src/a.ts");
        let failing = GitVisibleFiles::with_runner(Arc::new(FailingRunner { timed_out: false }));
        assert!(
            failing.visible_files(root).is_err(),
            "git failure must not fall back to all files"
        );
        let timing_out = GitVisibleFiles::with_runner(Arc::new(FailingRunner { timed_out: true }));
        assert!(
            timing_out.visible_files(root).is_err(),
            "git timeout must not fall back to all files"
        );
        let mut budget = SourceScanBudget::new_batch();
        assert_eq!(
            scan_member_source(root, &failing, &mut budget),
            MemberSourceVerdict::Unknown
        );
    }
}
