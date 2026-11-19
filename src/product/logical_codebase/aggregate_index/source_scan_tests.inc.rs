use super::*;
use std::collections::VecDeque;
use std::fs;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use tempfile::TempDir;

/// 全可见提供者：与生产「非 git 仓 ⇒ 回落全部文件」口径一致。
struct AllVisible;

impl MemberSourceVisibility for AllVisible {
    fn visible_files(
        &self,
        member_root: &Path,
        budget: &SourceScanBudget<'_>,
    ) -> Result<Vec<PathBuf>, AggregateIndexError> {
        collect_all_files(member_root, budget)
    }
}

/// 固定可见范围的提供者（模拟 `git ls-files` 的输出）。
struct VisibleFiles(Vec<PathBuf>);

impl MemberSourceVisibility for VisibleFiles {
    fn visible_files(
        &self,
        _member_root: &Path,
        _budget: &SourceScanBudget<'_>,
    ) -> Result<Vec<PathBuf>, AggregateIndexError> {
        Ok(self.0.clone())
    }
}

/// 绝不允许被调到的提供者：入口预算已耗尽时必须连探测都不发起。
struct PanicVisibility;

impl MemberSourceVisibility for PanicVisibility {
    fn visible_files(
        &self,
        _member_root: &Path,
        _budget: &SourceScanBudget<'_>,
    ) -> Result<Vec<PathBuf>, AggregateIndexError> {
        panic!("visibility must not start when the budget is exhausted at entry");
    }
}

/// 探测期间置位取消信号的提供者（模拟 git 跑完的同时批被取消）。
struct CancellingVisibility(Vec<PathBuf>);

impl MemberSourceVisibility for CancellingVisibility {
    fn visible_files(
        &self,
        _member_root: &Path,
        budget: &SourceScanBudget<'_>,
    ) -> Result<Vec<PathBuf>, AggregateIndexError> {
        budget.cancel();
        Ok(self.0.clone())
    }
}

fn write_file(root: &Path, relative: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, b"sample").unwrap();
}

fn scan(root: &Path) -> MemberScanObservation {
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

/// 按脚本顺序回放的 runner 桩：记录每个子进程实际拿到的超时。
struct ScriptedRunner {
    responses: Mutex<VecDeque<BoundedCommandResult>>,
    issued_timeouts: Mutex<Vec<Duration>>,
}

impl ScriptedRunner {
    fn new(responses: Vec<BoundedCommandResult>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            issued_timeouts: Mutex::new(Vec::new()),
        }
    }

    fn issued_timeouts(&self) -> Vec<Duration> {
        self.issued_timeouts.lock().unwrap().clone()
    }

    fn issued_count(&self) -> usize {
        self.issued_timeouts.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for ScriptedRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        self.issued_timeouts.lock().unwrap().push(request.timeout);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(BoundedCommandError::Io {
                details: "no scripted response left".to_string(),
            })
    }
}

fn git_process_result(
    exit_code: Option<i32>,
    stdout: &str,
    stdout_truncated: bool,
    timed_out: bool,
) -> BoundedCommandResult {
    BoundedCommandResult {
        exit_code,
        stdout: stdout.to_string(),
        stderr: String::new(),
        timed_out,
        cancelled: false,
        stdout_truncated,
        stderr_truncated: false,
        duration_ms: 0,
    }
}

/// rev-parse 探针的成功回放（干净、未截断的「true」）。
fn probe_inside_work_tree() -> BoundedCommandResult {
    git_process_result(Some(0), "true\n", false, false)
}

/// runner 直接报 IO 错的桩。
struct IoErrorRunner;

#[async_trait::async_trait]
impl BoundedCommandRunner for IoErrorRunner {
    async fn run(
        &self,
        _request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        Err(BoundedCommandError::Io {
            details: "synthetic runner io failure".to_string(),
        })
    }
}

/// 1：只有配置与文档 ⇒ 无源码。
#[test]
fn scan_reports_no_source_when_only_config_and_docs() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "limits.json");
    write_file(root, "README.md");
    write_file(root, "deploy.yaml");
    let observation = scan(root);
    assert_eq!(observation.verdict, MemberSourceVerdict::NoSource);
    assert_eq!(observation.reason, None);
    assert!(observation.visibility_started);
}

/// 2：清单内每个后缀都能命中源码。
#[test]
fn scan_reports_source_in_normal_path_for_aria_extensions() {
    for extension in ARIA_SOURCE_EXTENSIONS {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write_file(root, &format!("src/sample.{extension}"));
        assert_eq!(
            scan(root).verdict,
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
    assert_eq!(
        scan(root).verdict,
        MemberSourceVerdict::SourceOnlyInWorktrees
    );
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
    // git 可见范围不含被忽略的 `.worktrees/` 子树（生产口径）。
    let visible = VisibleFiles(vec![PathBuf::from("src/main.rs")]);
    let mut budget = SourceScanBudget::new_batch();
    assert_eq!(
        scan_member_source(root, &visible, &mut budget).verdict,
        MemberSourceVerdict::SourceInNormalPath
    );
}

/// 5：时限已过且无正证据 ⇒ 无法确认（入口即到点：不发 git、不枚举）。
#[test]
fn scan_returns_unknown_when_deadline_passed_without_positive_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "src/main.rs");
    let mut budget =
        SourceScanBudget::for_tests(usize::MAX, Some(Instant::now() - Duration::from_secs(1)));
    let observation = scan_member_source(root, &AllVisible, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::Deadline));
    assert!(!observation.visibility_started);
}

/// 6：条目预算耗尽且无正证据 ⇒ 无法确认（不等于无源码）。
#[test]
fn scan_returns_unknown_when_entry_budget_exhausted_without_positive_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "a/b/c/limits.json");
    let mut budget = SourceScanBudget::for_tests(1, None);
    let observation = scan_member_source(root, &AllVisible, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::BatchEntries));
    assert!(observation.visibility_started);
}

/// 7：取消信号 ⇒ 无法确认（入口即取消：连探测都不发起）。
#[test]
fn scan_returns_unknown_on_cancel() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "src/main.rs");
    let mut budget = SourceScanBudget::new_batch();
    budget.cancel();
    let observation = scan_member_source(root, &PanicVisibility, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::Cancelled));
    assert!(!observation.visibility_started);
}

/// 8：不可读的成员根 ⇒ 无法确认（不得当成无源码），原因细分到 Permission。
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
    let observation = scan_member_source(&root, &AllVisible, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::Permission));
    assert!(observation.visibility_started);
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
    assert_eq!(scan(&root).verdict, MemberSourceVerdict::NoSource);
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
    assert_eq!(scan(&root).verdict, MemberSourceVerdict::NoSource);
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
    assert_eq!(scan(root).verdict, MemberSourceVerdict::NoSource);
    write_file(root, "src/a.ts");
    assert_eq!(scan(root).verdict, MemberSourceVerdict::SourceInNormalPath);
}

/// 12：4 条非目录模式按目录名剪枝；排除表是单一来源。
#[test]
fn scan_prunes_non_directory_builtin_patterns_as_directory_names() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "credentials/src/a.rs");
    write_file(root, "secret-notes/src/b.ts");
    write_file(root, ".env.d/c.rs");
    assert_eq!(scan(root).verdict, MemberSourceVerdict::NoSource);
    assert_eq!(super::super::exclude::BUILTIN_EXCLUDES.len(), 11);
}

/// 13：命中非目录模式的文件不算源码候选。
#[test]
fn scan_ignores_source_files_matching_non_directory_builtin_patterns() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "credentials.rs");
    write_file(root, "secret_manager.ts");
    assert_eq!(scan(root).verdict, MemberSourceVerdict::NoSource);
}

/// 14：整批预算在成员之间共享（已耗尽的预算不会被重置）。
#[test]
fn scan_batch_budget_is_shared_across_members() {
    let temp = TempDir::new().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    write_file(&first, "src/main.rs");
    write_file(&second, "src/main.rs");
    let mut budget = SourceScanBudget::for_tests(1, None);
    assert_eq!(
        scan_member_source(&first, &AllVisible, &mut budget).verdict,
        MemberSourceVerdict::SourceInNormalPath
    );
    // 同一份预算继续用：条目已耗尽 ⇒ 第二个成员入口即 Unknown，不重置。
    let second_observation = scan_member_source(&second, &PanicVisibility, &mut budget);
    assert_eq!(second_observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(
        second_observation.reason,
        Some(ScanUnknownReason::BatchEntries)
    );
    assert!(!second_observation.visibility_started);
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
    let observation = scan_member_source(&root, &AllVisible, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::MemberEntries));
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
    assert_eq!(
        scan(root).verdict,
        MemberSourceVerdict::SourceOnlyInWorktrees
    );

    let too_deep = TempDir::new().unwrap();
    let deep_root = too_deep.path();
    let deep = deep_worktrees_file(deep_root, 31);
    if let Some(parent) = deep.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&deep, b"fn main() {}").unwrap();
    assert_eq!(scan(deep_root).verdict, MemberSourceVerdict::Unknown);
}

/// 17：`.worktrees/` 缺失或为符号链接 ⇒ 该段无候选（不是无法确认）。
#[test]
#[cfg(unix)]
fn scan_treats_missing_or_symlinked_worktrees_entry_as_no_candidate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("member");
    fs::create_dir_all(&root).unwrap();
    write_file(&root, "limits.json");
    assert_eq!(scan(&root).verdict, MemberSourceVerdict::NoSource);

    let outside = TempDir::new().unwrap();
    write_file(outside.path(), "src/main.rs");
    std::os::unix::fs::symlink(outside.path(), root.join(".worktrees")).unwrap();
    assert_eq!(scan(&root).verdict, MemberSourceVerdict::NoSource);
}

/// 18：后缀匹配大小写不敏感（只看最后一段）。
#[test]
fn scan_matches_extension_case_insensitively_on_last_segment() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "SRC/MAIN.RS");
    assert_eq!(scan(root).verdict, MemberSourceVerdict::SourceInNormalPath);
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
        scan_member_source(root, &GitVisibleFiles::production(), &mut budget).verdict,
        MemberSourceVerdict::SourceInNormalPath
    );

    let only_ignored = TempDir::new().unwrap();
    let ignored_root = only_ignored.path();
    init_git_repo(ignored_root);
    fs::write(ignored_root.join(".gitignore"), b"sandbox/\n").unwrap();
    write_file(ignored_root, "sandbox/ignored.ts");
    let mut budget = SourceScanBudget::new_batch();
    assert_eq!(
        scan_member_source(ignored_root, &GitVisibleFiles::production(), &mut budget).verdict,
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
        scan_member_source(root, &GitVisibleFiles::production(), &mut budget).verdict,
        MemberSourceVerdict::SourceInNormalPath
    );
}

/// 21：清单是校准 notes 的 1.6.3 实测收录集的子集，且不含配置/文档后缀。
#[test]
fn aria_source_extensions_are_subset_of_calibrated_1_6_3_set() {
    let notes =
        fs::read_to_string("cadence/notes/2026-11-18_校准_CodeGraph源码后缀清单_1.6.3_v1.0.md")
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
    for excluded in [
        "json", "md", "toml", "txt", "csv", "html", "yaml", "yml", "xml",
    ] {
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
    let budget = SourceScanBudget::new_batch();
    let visible = GitVisibleFiles::production()
        .visible_files(root, &budget)
        .expect("non git member falls back to all files");
    assert!(
        visible.iter().any(|path| path.ends_with("src/a.ts")),
        "fallback must list every file: {visible:?}"
    );
    let mut budget = SourceScanBudget::new_batch();
    assert_eq!(
        scan_member_source(root, &GitVisibleFiles::production(), &mut budget).verdict,
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
    let budget = SourceScanBudget::new_batch();
    let failing = GitVisibleFiles::with_runner(Arc::new(FailingRunner { timed_out: false }));
    assert!(
        failing.visible_files(root, &budget).is_err(),
        "git failure must not fall back to all files"
    );
    let timing_out = GitVisibleFiles::with_runner(Arc::new(FailingRunner { timed_out: true }));
    assert!(
        timing_out.visible_files(root, &budget).is_err(),
        "git timeout must not fall back to all files"
    );
    let mut budget = SourceScanBudget::new_batch();
    let observation = scan_member_source(root, &failing, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::GitFailed));
    assert!(observation.visibility_started);
}

/// 24：被跟踪源码文件所在目录不可读 ⇒ 无法确认，不得判「无源码」。
/// `git ls-files --cached` 直读 index（不需要目录可读）仍会列出该文件，
/// 但进程 stat 得到 EACCES——证据不足必须落 `Unknown`（契约：无法确认
/// MUST NOT 等同于无源码）。
#[cfg(unix)]
#[test]
fn scan_reports_unknown_when_tracked_source_dir_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_repo(root);
    write_file(root, "src/lib.rs");
    git_add(root, &["src/lib.rs"]);
    let visibility = GitVisibleFiles::production();
    let src_dir = root.join("src");
    // 目录 0o000：git 仍从 index 列出被跟踪文件；stat 因 EACCES 失败。
    fs::set_permissions(&src_dir, fs::Permissions::from_mode(0o000)).unwrap();
    let mut budget = SourceScanBudget::new_batch();
    let visible = visibility.visible_files(root, &budget).unwrap();
    let observation = scan_member_source(root, &visibility, &mut budget);
    // 无论结论如何都先恢复目录权限，保证 TempDir 清理能删除目录内文件。
    fs::set_permissions(&src_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        visible.contains(&PathBuf::from("src/lib.rs")),
        "fixture premise broken: git must still list the tracked file, got {visible:?}"
    );
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::Permission));
    assert!(observation.visibility_started);
}

/// 25：入口 deadline 已到 ⇒ Unknown(Deadline)，可见范围探测根本不发起。
#[test]
fn scan_skips_visibility_when_deadline_expired_at_entry() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "src/main.rs");
    let mut budget =
        SourceScanBudget::for_tests(usize::MAX, Some(Instant::now() - Duration::from_secs(1)));
    let observation = scan_member_source(root, &PanicVisibility, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::Deadline));
    assert!(!observation.visibility_started);
}

/// 26：入口条目预算为 0 ⇒ Unknown(BatchEntries)，可见范围探测不发起。
#[test]
fn scan_skips_visibility_when_batch_entries_exhausted_at_entry() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "src/main.rs");
    let mut budget = SourceScanBudget::for_tests(0, None);
    let observation = scan_member_source(root, &PanicVisibility, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::BatchEntries));
    assert!(!observation.visibility_started);
}

/// 27：ls-files 输出被截断 ⇒ 即使退出码为 0 也不接受：截断处之后可能
/// 还有源码（磁盘上确实放了 cap 之外的源码），必须 Unknown(OutputTruncated)。
#[test]
fn scan_reports_unknown_when_listing_truncated_with_source_beyond_cap() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "docs/a.md");
    // 截断处之外的源码：模拟被 cap 截掉的行，磁盘证据仍在。
    write_file(root, "src/main.rs");
    let runner = Arc::new(ScriptedRunner::new(vec![
        probe_inside_work_tree(),
        git_process_result(Some(0), "docs/a.md\n", true, false),
    ]));
    let visibility = GitVisibleFiles::with_runner(runner.clone());
    let mut budget = SourceScanBudget::for_tests(usize::MAX, None);
    let observation = scan_member_source(root, &visibility, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::OutputTruncated));
    assert!(observation.visibility_started);
    assert_eq!(
        runner.issued_count(),
        2,
        "probe + listing must both have run"
    );
}

/// 28：git 命令失败与「独立 git 超时」是两个不同的无法确认原因。
#[test]
fn scan_reasons_distinguish_git_failure_from_independent_git_timeout() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "src/a.ts");

    // (a) 探针非 0 退出且不是「非 git 仓」⇒ GitFailed。
    let failing =
        GitVisibleFiles::with_runner(Arc::new(ScriptedRunner::new(vec![git_process_result(
            Some(1),
            "",
            false,
            false,
        )])));
    let mut budget = SourceScanBudget::for_tests(usize::MAX, None);
    let observation = scan_member_source(root, &failing, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::GitFailed));
    assert!(observation.visibility_started);

    // (b) 无批时限时 git 自己超时（完整 10 秒上限）⇒ GitTimeout，不是 Deadline。
    let timing_out_runner = Arc::new(ScriptedRunner::new(vec![git_process_result(
        Some(0),
        "true\n",
        false,
        true,
    )]));
    let timing_out = GitVisibleFiles::with_runner(timing_out_runner.clone());
    let mut budget = SourceScanBudget::for_tests(usize::MAX, None);
    let observation = scan_member_source(root, &timing_out, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::GitTimeout));
    assert!(observation.visibility_started);
    // 无批时限 ⇒ 子进程拿到完整 10 秒独立上限（未被批预算封顶）。
    assert_eq!(
        timing_out_runner.issued_timeouts(),
        vec![Duration::from_secs(GIT_TIMEOUT_SECS)]
    );
}

/// 29：git 子进程超时按批剩余时间封顶；被批预算掐掉的超时 ⇒ Deadline
/// （用假单调时钟确定性复现，不 sleep）。
#[test]
fn git_timeout_capped_by_batch_deadline_reports_deadline() {
    let base = Instant::now();
    let clock = move || base;
    let mut budget = SourceScanBudget::new_batch_with_clock(&clock);

    let runner = Arc::new(ScriptedRunner::new(vec![git_process_result(
        Some(0),
        "true\n",
        false,
        true,
    )]));
    let visibility = GitVisibleFiles::with_runner(runner.clone());
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "src/main.rs");

    let observation = scan_member_source(root, &visibility, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::Deadline));
    assert!(observation.visibility_started);
    // 子进程拿到的超时 = 剩余时间（SOURCE_SCAN_DEADLINE），不是 10 秒。
    assert_eq!(runner.issued_timeouts(), vec![SOURCE_SCAN_DEADLINE]);
}

/// 30：批时限已归零 ⇒ 不再发出 git 子进程，直接报 Deadline。
#[test]
fn git_subprocess_not_issued_when_batch_deadline_is_gone() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "src/main.rs");
    let runner = Arc::new(ScriptedRunner::new(vec![probe_inside_work_tree()]));
    let visibility = GitVisibleFiles::with_runner(runner.clone());
    let budget =
        SourceScanBudget::for_tests(usize::MAX, Some(Instant::now() - Duration::from_secs(1)));
    let error = visibility
        .visible_files(root, &budget)
        .expect_err("expired budget must fail before issuing git");
    assert_eq!(reason_from_error(&error), ScanUnknownReason::Deadline);
    assert_eq!(runner.issued_count(), 0, "no git subprocess may be issued");
}

/// 31：runner 自身 IO 失败 ⇒ 无法确认（Io），不会误报成 git 失败或超时。
#[test]
fn scan_reports_io_when_git_runner_io_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "src/main.rs");
    let visibility = GitVisibleFiles::with_runner(Arc::new(IoErrorRunner));
    let mut budget = SourceScanBudget::for_tests(usize::MAX, None);
    let observation = scan_member_source(root, &visibility, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::Io));
    assert!(observation.visibility_started);
}

/// 32：可见范围探测完成的同时批被取消 ⇒ Unknown(Cancelled)，且已开始探测。
#[test]
fn scan_reports_cancelled_observed_after_visibility_started() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_file(root, "src/main.rs");
    let visibility = CancellingVisibility(vec![PathBuf::from("src/main.rs")]);
    let mut budget = SourceScanBudget::for_tests(usize::MAX, None);
    let observation = scan_member_source(root, &visibility, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::Cancelled));
    assert!(observation.visibility_started);
}

/// 33：假单调时钟下复现「扫描中途到点」：可见范围阶段走到一半 deadline
/// 到期 ⇒ Unknown(Deadline)（已开始探测），全程不 sleep。
#[test]
fn scan_reports_deadline_mid_walk_with_fake_monotonic_clock() {
    let base = Instant::now();
    let counter = Arc::new(AtomicUsize::new(0));
    // 前 4 次取时停在 base（预算构造 + 入口检查 + 探测后检查 + 第一轮条目），
    // 之后跳到 1 小时后 ⇒ 可见范围阶段中途到点。
    let clock = move || {
        let index = counter.fetch_add(1, Ordering::SeqCst);
        if index < 4 {
            base
        } else {
            base + Duration::from_secs(3600)
        }
    };
    let mut budget = SourceScanBudget::new_batch_with_clock(&clock);
    let visible: Vec<PathBuf> = (0..8)
        .map(|index| PathBuf::from(format!("f{index}.txt")))
        .collect();
    let visibility = VisibleFiles(visible);
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let observation = scan_member_source(root, &visibility, &mut budget);
    assert_eq!(observation.verdict, MemberSourceVerdict::Unknown);
    assert_eq!(observation.reason, Some(ScanUnknownReason::Deadline));
    assert!(observation.visibility_started);
}

/// 34：`remaining_ms` 走注入时钟：有 deadline ⇒ 剩余毫秒；无 ⇒ None；已过 ⇒ 0。
#[test]
fn budget_remaining_ms_uses_injected_clock() {
    let base = Instant::now();
    let clock = move || base;
    let budget = SourceScanBudget::new_batch_with_clock(&clock);
    assert_eq!(
        budget.remaining_ms(),
        Some(SOURCE_SCAN_DEADLINE.as_millis() as u64)
    );
    assert_eq!(
        SourceScanBudget::for_tests(usize::MAX, None).remaining_ms(),
        None
    );
    assert_eq!(
        SourceScanBudget::for_tests(usize::MAX, Some(Instant::now() - Duration::from_secs(1)))
            .remaining_ms(),
        Some(0)
    );
}
