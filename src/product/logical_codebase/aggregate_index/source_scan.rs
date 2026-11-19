//! Aria 源码口径 + 有界盘检扫描器。
//!
//! 用途：判定「成员主 checkout 的磁盘上是否存在 Aria 口径的源码」，供聚合索引
//! 验收阶段区分「有源码却没被索引」（漏收 ⇒ 失败）与「没有源码」（⇒ 跳过）。
//!
//! 返回值是 [`MemberScanObservation`]：**只报告现场观测到的事实**（判定值 +
//! 无法确认时的具体原因 + 是否已开始可见范围探测），不夹带从成员名或历史
//! 推测出来的结论。
//!
//! 口径：**源码 = git 可见范围 ∩ `ARIA_SOURCE_EXTENSIONS` ∩ 非 Aria 自身排除**。
//!
//! - **可见范围**：`git ls-files --cached --others --exclude-standard`（跟踪的 +
//!   未跟踪且未被忽略的）⇒ 被 `.gitignore` 忽略的文件**不算源码**。
//! - **排除口径**：复用 `exclude.rs` 的 `BUILTIN_EXCLUDES`（Aria 排除集 =
//!   codegraph 排除集，同源）：目录级剪枝 + 4 条非目录模式在文件级同样扣除。
//! - **非 git 仓**：回落「该根下全部文件」（非 git 目录没有 ignore 语义）；
//!   git 不可用 / 命令失败 / 超时 / 取消 ⇒ **无法确认**（不得回落成全部文件）。
//! - **时限与截断**：每个 git 子进程的超时按批预算剩余时间封顶；剩余归零后
//!   不再发出子进程；清单输出被截断 ⇒ 无法确认（截断处之后可能还有源码）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    TokioBoundedCommandRunner,
};

use super::AggregateIndexError;
use super::exclude::BUILTIN_EXCLUDES;
use super::scan_diagnostics::ScanUnknownReason;

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

/// `ARIA_SOURCE_EXTENSIONS` 的**校准基线 codegraph 版本**（实测
/// `codegraph --version` 的版本号）。
///
/// - **校准基线**：1.6.3（实测收录表见
///   `cadence/notes/2026-11-18_校准_CodeGraph源码后缀清单_1.6.3_v1.0.md`）。
/// - **可复现测量命令**：逐后缀独立临时根放该语言最小有效样本 ⇒
///   `codegraph init .` ⇒ `codegraph sync .` ⇒ `codegraph files --json`，
///   样本出现在收录结果里 ⇒ 该版本会收录该后缀（生产 argv 以
///   `codegraph_cli.rs` 为准）。
/// - **复校准步骤**：换 codegraph 版本后 ⇒ 重跑上面的测量 ⇒ 更新
///   `ARIA_SOURCE_EXTENSIONS` ⇒ 更新本常量 ⇒ 在校准 notes 追加新基线 ⇒
///   同步更新 `source_scope_tests` 的期望。
/// - **漂移后果**：新版若不再收录清单内某后缀，该类仓会被判「漏收」而失败；
///   运行期遇到版本漂移**只记可审计软告警、不阻断索引构建**。
pub const ARIA_SOURCE_SUFFIX_CALIBRATED_CODEGRAPH_VERSION: &str = "1.6.3";

/// 物理扫描段（`.worktrees/` 诊断）的深度上限（根深度 = 0）。
pub const SOURCE_SCAN_MAX_DEPTH: usize = 32;
/// 单成员条目上限（独立于整批上限）。
pub const SOURCE_SCAN_MAX_ENTRIES_PER_MEMBER: usize = 20_000;
/// 整批条目上限（两阶段、所有成员共享同一计数）。
pub const SOURCE_SCAN_MAX_ENTRIES_PER_BATCH: usize = 100_000;
/// 整批盘检时限。
pub const SOURCE_SCAN_DEADLINE: Duration = Duration::from_secs(2);

/// 单个成员的源码判定结果（四值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// `scan_member_source` 的现场观测结果：只携带观测到的事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberScanObservation {
    /// 该成员的源码判定值。
    pub verdict: MemberSourceVerdict,
    /// `Unknown` 时的具体原因（观测到的第一手事实；其它判定值恒为 `None`）。
    pub reason: Option<ScanUnknownReason>,
    /// 是否已经开始可见范围探测（含 git 子进程 / 非 git 回落枚举）。
    /// `false` ⇔ 入口时预算已耗尽，连探测都没发起。
    pub visibility_started: bool,
}

/// 单段扫描的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageOutcome {
    /// 找到源码候选（正证据）。
    Found,
    /// 该段完整走完且没有候选。
    Clean,
    /// 该段未能完整走完，携带第一手的无法确认原因。
    ///
    /// 已记录的具体 IO/权限错误**不会**被后来的预算到点覆盖
    /// （先到先得，见 [`StageOutcome::or_incomplete`]）。
    Incomplete(ScanUnknownReason),
}

impl StageOutcome {
    /// 只在当前仍是 `Clean` 时降级为 `Incomplete`：保留最先观测到的具体原因，
    /// 不让 deadline / 条目耗尽之类的事后预算事实顶替它。
    fn or_incomplete(self, reason: ScanUnknownReason) -> Self {
        match self {
            StageOutcome::Clean => StageOutcome::Incomplete(reason),
            resolved => resolved,
        }
    }
}

/// 可注入预算：batch 级共享「剩余条目 + deadline + 取消信号 + 单调时钟」，
/// 测试注入假时钟即可确定性触发时限路径，不依赖真实 sleep。
///
/// 时钟以 `&dyn Fn` 形式注入一次（无逐条目分配）；预算是**一次性**的，
/// 消耗后不会被重置。
pub struct SourceScanBudget<'a> {
    remaining_entries: usize,
    deadline: Option<Instant>,
    cancelled: AtomicBool,
    clock: &'a (dyn Fn() -> Instant + Sync),
}

impl SourceScanBudget<'_> {
    /// 生产口径：整批 10 万条目 + 2 秒时限，单调时钟取 `Instant::now`。
    pub fn new_batch() -> SourceScanBudget<'static> {
        Self::new_batch_with_clock(&Instant::now)
    }

    /// 生产口径 + 注入单调时钟（deadline = 首次取时 + [`SOURCE_SCAN_DEADLINE`]）。
    pub(super) fn new_batch_with_clock(
        clock: &(dyn Fn() -> Instant + Sync),
    ) -> SourceScanBudget<'_> {
        SourceScanBudget {
            remaining_entries: SOURCE_SCAN_MAX_ENTRIES_PER_BATCH,
            deadline: Some(clock() + SOURCE_SCAN_DEADLINE),
            cancelled: AtomicBool::new(false),
            clock,
        }
    }

    /// 测试口径：显式给定剩余条目数与 deadline（`None` = 不限时），时钟取
    /// `Instant::now`。语义与生产一致：预算一次性消耗，**不会**被重置。
    pub fn for_tests(
        remaining_entries: usize,
        deadline: Option<Instant>,
    ) -> SourceScanBudget<'static> {
        SourceScanBudget {
            remaining_entries,
            deadline,
            cancelled: AtomicBool::new(false),
            clock: &Instant::now,
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

    /// 剩余时限毫秒数（无 deadline ⇒ `None`；已到点 ⇒ `Some(0)`）。
    /// 供调用方在成员入口处拍照取证（`remaining_ms_at_entry`）。
    pub(super) fn remaining_ms(&self) -> Option<u64> {
        self.remaining_time()
            .map(|remaining| remaining.as_millis() as u64)
    }

    /// 剩余时限（无 deadline ⇒ `None`）；git 子进程的超时封顶以此为准。
    fn remaining_time(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since((self.clock)()))
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn deadline_passed(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| (self.clock)() > deadline)
    }

    /// 预算耗尽的原因（取消 / 超时 / 条目用尽）⇒ 该段不再可信。
    /// 优先级：deadline 到点 ⇒ 取消 ⇒ 条目用尽。
    fn exhaustion_reason(&self) -> Option<ScanUnknownReason> {
        if self.deadline_passed() {
            return Some(ScanUnknownReason::Deadline);
        }
        if self.is_cancelled() {
            return Some(ScanUnknownReason::Cancelled);
        }
        if self.remaining_entries == 0 {
            return Some(ScanUnknownReason::BatchEntries);
        }
        None
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
    /// **非 git 仓** ⇒ 返回该根下的全部文件（非 git 目录没有 ignore 语义，
    /// 回落枚举器同样受 `budget` 约束）。
    /// git 不可用 / 命令失败 / 超时 / 取消 / 输出截断 ⇒ `Err`（⇒ 该成员落
    /// `Unknown`，**不得**当作「无源码」）。
    fn visible_files(
        &self,
        member_root: &Path,
        budget: &SourceScanBudget<'_>,
    ) -> Result<Vec<PathBuf>, AggregateIndexError>;
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
    ///
    /// - 超时按 `remaining` 封顶：剩余时间少于 [`GIT_TIMEOUT_SECS`] 时按剩余时间
    ///   掐子进程；剩余归零时**不发子进程**直接报 deadline。
    /// - 被批预算掐掉的子进程超时 ⇒ `aggregate_index_source_scan_deadline`
    ///   （[`ScanUnknownReason::Deadline`]）；批预算没介入、git 自己超时 ⇒
    ///   `aggregate_index_source_scan_git_timeout`（[`ScanUnknownReason::GitTimeout`]）。
    fn git(
        &self,
        cwd: &Path,
        argv: &[&str],
        remaining: Option<Duration>,
    ) -> Result<BoundedCommandResult, AggregateIndexError> {
        let git_limit = Duration::from_secs(GIT_TIMEOUT_SECS);
        let (timeout, capped_by_batch) = match remaining {
            None => (git_limit, false),
            Some(remaining) if remaining.is_zero() => {
                // 批预算已到点：不再发出 git 子进程。
                return Err(AggregateIndexError::Failed {
                    code: "aggregate_index_source_scan_deadline",
                    message: "batch deadline expired before issuing git".to_string(),
                });
            }
            Some(remaining) if remaining < git_limit => (remaining, true),
            Some(_) => (git_limit, false),
        };
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
            timeout,
            cancellation: CancellationToken::new(),
            environment: sanitized_environment(),
            stdout_limit: GIT_OUTPUT_LIMIT_BYTES,
            stderr_limit: GIT_OUTPUT_LIMIT_BYTES,
        };
        let runner = self.runner.clone();
        let command = argv.first().copied().unwrap_or("git");
        let result = std::thread::spawn(
            move || -> Result<BoundedCommandResult, AggregateIndexError> {
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
            },
        )
        .join()
        .map_err(|_| AggregateIndexError::Failed {
            code: "aggregate_index_source_scan_git_panic",
            message: "bounded Git runner thread panicked".to_string(),
        })??;
        if result.timed_out {
            return Err(AggregateIndexError::Failed {
                code: if capped_by_batch {
                    // 批预算掐掉的子进程：算 deadline，不算独立 git 超时。
                    "aggregate_index_source_scan_deadline"
                } else {
                    "aggregate_index_source_scan_git_timeout"
                },
                message: format!("git {command} timed out"),
            });
        }
        Ok(result)
    }
}

impl MemberSourceVisibility for GitVisibleFiles {
    fn visible_files(
        &self,
        member_root: &Path,
        budget: &SourceScanBudget<'_>,
    ) -> Result<Vec<PathBuf>, AggregateIndexError> {
        let probe = self.git(
            member_root,
            &["rev-parse", "--is-inside-work-tree"],
            budget.remaining_time(),
        )?;
        if probe.cancelled {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_git_cancelled",
                message: "git rev-parse --is-inside-work-tree was cancelled".to_string(),
            });
        }
        // rev-parse 探针必须给出干净、未截断的「true」才算站在工作树里；
        // 异常输出一律 fail closed（Err ⇒ Unknown，绝不回落成全部文件）。
        let inside_work_tree =
            probe.exit_code == Some(0) && !probe.stdout_truncated && probe.stdout.trim() == "true";
        // 只有「确定不是 git 仓」才回落全部文件。
        let definitively_not_a_repository = probe.exit_code == Some(128)
            && probe.stderr.to_lowercase().contains("not a git repository");
        if !inside_work_tree {
            if definitively_not_a_repository {
                return collect_all_files(member_root, budget);
            }
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_git_failed",
                message: format!(
                    "git rev-parse --is-inside-work-tree did not produce a usable verdict (exit {:?})",
                    probe.exit_code
                ),
            });
        }
        let listing = self.git(
            member_root,
            &["ls-files", "--cached", "--others", "--exclude-standard"],
            budget.remaining_time(),
        )?;
        if listing.cancelled {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_git_cancelled",
                message: "git ls-files --cached --others --exclude-standard was cancelled"
                    .to_string(),
            });
        }
        if listing.exit_code != Some(0) {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_git_failed",
                message: format!(
                    "git ls-files --cached --others --exclude-standard exited with {:?}",
                    listing.exit_code
                ),
            });
        }
        // 截断的清单不完整：截断处之后可能还有源码 ⇒ 成功退出码也不接受。
        if listing.stdout_truncated {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_git_truncated",
                message: "git ls-files output was truncated at the capture cap".to_string(),
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
        || rules
            .file_patterns
            .iter()
            .any(|rule| glob_matches(rule, name))
}

/// 文件名是否命中排除规则（4 条非目录模式在文件级同样扣除）。
fn is_excluded_file_name(rules: &ExcludeRules, name: &str) -> bool {
    rules
        .file_patterns
        .iter()
        .any(|rule| glob_matches(rule, name))
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
/// 枚举其全部普通文件后由扫描阶段按后缀与排除规则判定。回落枚举器同样
/// 受 `budget` 约束（deadline / 取消 / 整批剩余条目数），耗尽即 `Err`。
fn collect_all_files(
    root: &Path,
    budget: &SourceScanBudget<'_>,
) -> Result<Vec<PathBuf>, AggregateIndexError> {
    if let Some(reason) = budget.exhaustion_reason() {
        return Err(budget_exhaustion_error(reason));
    }
    // 枚举上限 = 内建上限与整批剩余条目数的较小者（回落枚举器同样有界）。
    let entry_cap = SOURCE_SCAN_MAX_ENTRIES_PER_BATCH.min(budget.remaining_entries());
    let mut files = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if let Some(reason) = budget.exhaustion_reason() {
            return Err(budget_exhaustion_error(reason));
        }
        if depth > SOURCE_SCAN_MAX_DEPTH {
            return Err(AggregateIndexError::Failed {
                code: "aggregate_index_source_scan_depth",
                message: format!("directory depth limit exceeded under {}", root.display()),
            });
        }
        let entries =
            fs::read_dir(&directory).map_err(|error| io_scan_error(&directory, &error))?;
        for entry in entries {
            let entry = entry.map_err(|error| io_scan_error(&directory, &error))?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|error| io_scan_error(&path, &error))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                pending.push((path, depth + 1));
            } else if metadata.is_file() {
                if files.len() >= entry_cap {
                    return Err(AggregateIndexError::Failed {
                        code: "aggregate_index_source_scan_batch_entries",
                        message: format!("file count limit exceeded under {}", root.display()),
                    });
                }
                files.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
    }
    Ok(files)
}

/// 扫描成员主 checkout 的 Aria 源码情况，返回**现场观测**。
///
/// 行为表：
///
/// | 情况 | 返回值 |
/// |---|---|
/// | 入口预算已耗尽 | `Unknown`（原因 = deadline/取消/条目耗尽），`visibility_started = false`（连 git 都不发） |
/// | 可见范围（git 可见 ∩ 清单 ∩ 非排除）内找到源码 | `SourceInNormalPath`（立即停，不再扫 `.worktrees/`） |
/// | 可见范围内无源码且无错误、`.worktrees/` 子树内找到源码 | `SourceOnlyInWorktrees` |
/// | 两段都完成且都无候选 | `NoSource` |
/// | 可见范围探测失败（git 不可用/超时/取消/截断/权限/IO）或任一段因权限/IO/深度/条目数/超时/取消/预算不足未能完成，且**没有**普通路径源码正证据 | `Unknown` + 第一手 `reason` |
/// | 已找到普通路径源码后才出现的错误 | 仍 `SourceInNormalPath`（正证据优先） |
///
/// 具体权限 / IO 错误一旦观测到就是最终原因，不会被之后的预算到点
/// （deadline / 条目耗尽）顶替。
pub fn scan_member_source(
    member_root: &Path,
    visibility: &dyn MemberSourceVisibility,
    budget: &mut SourceScanBudget<'_>,
) -> MemberScanObservation {
    if let Some(reason) = budget.exhaustion_reason() {
        // 入口即耗尽：不发 git、不枚举，直接报告观测到的耗尽原因。
        return MemberScanObservation {
            verdict: MemberSourceVerdict::Unknown,
            reason: Some(reason),
            visibility_started: false,
        };
    }
    let visible = match visibility.visible_files(member_root, budget) {
        Ok(visible) => visible,
        // git 不可用 / 命令失败 / 截断 / 权限 / IO ⇒ 无法确认（不得当作「无源码」）；
        // 保留 callee 的第一手错误，不用预算事实顶替。
        Err(error) => {
            return MemberScanObservation {
                verdict: MemberSourceVerdict::Unknown,
                reason: Some(reason_from_error(&error)),
                visibility_started: true,
            };
        }
    };
    if let Some(reason) = budget.exhaustion_reason() {
        // 可见范围探测耗尽了剩余预算（例如 git 刚好跑完时到点）。
        return MemberScanObservation {
            verdict: MemberSourceVerdict::Unknown,
            reason: Some(reason),
            visibility_started: true,
        };
    }
    let mut member_entries = 0usize;
    match scan_visible_range(member_root, &visible, budget, &mut member_entries) {
        // 正证据优先：普通路径命中源码后不再扫描 `.worktrees/`。
        StageOutcome::Found => {
            return MemberScanObservation {
                verdict: MemberSourceVerdict::SourceInNormalPath,
                reason: None,
                visibility_started: true,
            };
        }
        StageOutcome::Incomplete(reason) => {
            return MemberScanObservation {
                verdict: MemberSourceVerdict::Unknown,
                reason: Some(reason),
                visibility_started: true,
            };
        }
        StageOutcome::Clean => {}
    }
    match scan_worktrees_tree(member_root, budget, &mut member_entries) {
        StageOutcome::Found => MemberScanObservation {
            verdict: MemberSourceVerdict::SourceOnlyInWorktrees,
            reason: None,
            visibility_started: true,
        },
        StageOutcome::Incomplete(reason) => MemberScanObservation {
            verdict: MemberSourceVerdict::Unknown,
            reason: Some(reason),
            visibility_started: true,
        },
        StageOutcome::Clean => MemberScanObservation {
            verdict: MemberSourceVerdict::NoSource,
            reason: None,
            visibility_started: true,
        },
    }
}

/// 第一段：可见范围内的源码判定。
fn scan_visible_range(
    root: &Path,
    visible: &[PathBuf],
    budget: &mut SourceScanBudget<'_>,
    member_entries: &mut usize,
) -> StageOutcome {
    let rules = exclude_rules();
    for relative in visible {
        // 每个条目（含被排除的）都先过预算：deadline / 取消 / 条目耗尽即停。
        if let Some(reason) = budget.exhaustion_reason() {
            return StageOutcome::Incomplete(reason);
        }
        if !budget.consume_entry() {
            return StageOutcome::Incomplete(ScanUnknownReason::BatchEntries);
        }
        *member_entries += 1;
        if *member_entries > SOURCE_SCAN_MAX_ENTRIES_PER_MEMBER {
            return StageOutcome::Incomplete(ScanUnknownReason::MemberEntries);
        }
        let joined = root.join(relative);
        let metadata = match fs::symlink_metadata(&joined) {
            Ok(metadata) => metadata,
            // 已跟踪但已从工作区删除（或稀疏检出）：正常，跳过。
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            // 权限 / IO 失败 ⇒ 证据不足，不得当作「无源码」。
            Err(error) => return StageOutcome::Incomplete(io_reason(&error)),
        };
        // 符号链接一律跳过（不跟随、也不算源码）。
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        if is_excluded_relative_path(rules, relative) {
            continue;
        }
        if is_source_path(relative) {
            return StageOutcome::Found;
        }
    }
    // 负面判定（Clean ⇒ NoSource）前再核一次预算：耗尽即降级。
    if let Some(reason) = budget.exhaustion_reason() {
        return StageOutcome::Incomplete(reason);
    }
    StageOutcome::Clean
}

/// 第二段：`.worktrees/` 子树的物理扫描（独立诊断段，不计入普通路径判定）。
fn scan_worktrees_tree(
    root: &Path,
    budget: &mut SourceScanBudget<'_>,
    member_entries: &mut usize,
) -> StageOutcome {
    let worktrees = root.join(".worktrees");
    let metadata = match fs::symlink_metadata(&worktrees) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // 入口不存在 ⇒ 该段「已完成且无候选」（诊断段，不是主段）。
            return StageOutcome::Clean;
        }
        Err(error) => return StageOutcome::Incomplete(io_reason(&error)),
    };
    // 入口是符号链接 ⇒ 不跟随、无候选（也不是无法确认）。
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return StageOutcome::Clean;
    }
    walk_directory(&worktrees, 0, budget, member_entries)
}

/// 物理扫描：递归遍历一个目录（深度受限、条目受限、不跟随符号链接）。
fn walk_directory(
    directory: &Path,
    depth: usize,
    budget: &mut SourceScanBudget<'_>,
    member_entries: &mut usize,
) -> StageOutcome {
    if let Some(reason) = budget.exhaustion_reason() {
        return StageOutcome::Incomplete(reason);
    }
    let rules = exclude_rules();
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => return StageOutcome::Incomplete(io_reason(&error)),
    };
    let mut outcome = StageOutcome::Clean;
    for entry in entries {
        if let Some(reason) = budget.exhaustion_reason() {
            return outcome.or_incomplete(reason);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                // 记下第一手 IO/权限原因，继续看剩余条目（不覆盖已记录原因）。
                outcome = outcome.or_incomplete(io_reason(&error));
                continue;
            }
        };
        let child_depth = depth + 1;
        if child_depth > SOURCE_SCAN_MAX_DEPTH {
            return outcome.or_incomplete(ScanUnknownReason::Depth);
        }
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                outcome = outcome.or_incomplete(io_reason(&error));
                continue;
            }
        };
        // 符号链接一律跳过（目录与文件都跳过，不跟随）。
        if metadata.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if metadata.is_dir() {
            if is_excluded_directory_name(rules, &name) {
                continue;
            }
            if !budget.consume_entry() {
                return outcome.or_incomplete(ScanUnknownReason::BatchEntries);
            }
            *member_entries += 1;
            if *member_entries > SOURCE_SCAN_MAX_ENTRIES_PER_MEMBER {
                return outcome.or_incomplete(ScanUnknownReason::MemberEntries);
            }
            match walk_directory(&path, child_depth, budget, member_entries) {
                StageOutcome::Found => return StageOutcome::Found,
                StageOutcome::Incomplete(reason) => outcome = outcome.or_incomplete(reason),
                StageOutcome::Clean => {}
            }
        } else if metadata.is_file() {
            if is_excluded_file_name(rules, &name) {
                continue;
            }
            if !budget.consume_entry() {
                return outcome.or_incomplete(ScanUnknownReason::BatchEntries);
            }
            *member_entries += 1;
            if *member_entries > SOURCE_SCAN_MAX_ENTRIES_PER_MEMBER {
                return outcome.or_incomplete(ScanUnknownReason::MemberEntries);
            }
            if is_source_name(&name) {
                return StageOutcome::Found;
            }
        }
        // 其它类型（socket / fifo / 设备文件）跳过。
    }
    // 负面判定前再核一次预算；已记录的具体 IO 原因优先于预算事实。
    if let Some(reason) = budget.exhaustion_reason() {
        outcome = outcome.or_incomplete(reason);
    }
    outcome
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

/// 盘检路径上的 IO 错误 → 聚合索引错误（权限与一般 IO 用可区分的稳定码）。
fn io_scan_error(path: &Path, error: &std::io::Error) -> AggregateIndexError {
    AggregateIndexError::Failed {
        code: if error.kind() == std::io::ErrorKind::PermissionDenied {
            "aggregate_index_source_scan_permission"
        } else {
            "aggregate_index_source_scan_io"
        },
        message: format!("{}: {error}", path.display()),
    }
}

/// 盘检路径上的 IO 错误 → 无法确认原因（权限 / IO 分开）。
fn io_reason(error: &std::io::Error) -> ScanUnknownReason {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        ScanUnknownReason::Permission
    } else {
        ScanUnknownReason::Io
    }
}

/// 预算耗尽（deadline / 取消 / 条目）→ 聚合索引错误。
fn budget_exhaustion_error(reason: ScanUnknownReason) -> AggregateIndexError {
    let code = match reason {
        ScanUnknownReason::Deadline => "aggregate_index_source_scan_deadline",
        ScanUnknownReason::Cancelled => "aggregate_index_source_scan_cancelled",
        _ => "aggregate_index_source_scan_batch_entries",
    };
    AggregateIndexError::Failed {
        code,
        message: format!("source scan budget exhausted before completion ({reason:?})"),
    }
}

/// 把可见范围探测的第一手错误映射成无法确认原因（durable reason 只取
/// 稳定枚举，不带原始 stderr / 路径等易变细节）。
fn reason_from_error(error: &AggregateIndexError) -> ScanUnknownReason {
    let AggregateIndexError::Failed { code, .. } = error else {
        return ScanUnknownReason::Io;
    };
    match *code {
        "aggregate_index_source_scan_deadline" => ScanUnknownReason::Deadline,
        "aggregate_index_source_scan_cancelled" | "aggregate_index_source_scan_git_cancelled" => {
            ScanUnknownReason::Cancelled
        }
        "aggregate_index_source_scan_permission" => ScanUnknownReason::Permission,
        "aggregate_index_source_scan_git_timeout" => ScanUnknownReason::GitTimeout,
        "aggregate_index_source_scan_git_failed" | "aggregate_index_source_scan_git_missing" => {
            ScanUnknownReason::GitFailed
        }
        "aggregate_index_source_scan_git_truncated" => ScanUnknownReason::OutputTruncated,
        "aggregate_index_source_scan_depth" => ScanUnknownReason::Depth,
        "aggregate_index_source_scan_batch_entries" => ScanUnknownReason::BatchEntries,
        // io / git_io / git_runtime / git_panic 及未知码 ⇒ 保守归 Io。
        _ => ScanUnknownReason::Io,
    }
}

#[cfg(test)]
mod tests {
    include!("source_scan_tests.inc.rs");
}
