//! 把已安装的 CLI 暴露到用户终端 PATH（计划 §五 2.11）。
//!
//! 背景：PATH overlay 只让**平台自己**看见新装的 CLI；用户的终端
//! （Terminal / VS Code 终端）有自己那套 PATH（由 `.zshrc` 等决定）⇒
//! 用户在自己终端里敲 `claude` 仍可能 `command not found`。本模块在
//! **用户明确点击**时（绝不自动）把已安装二进制以软链暴露到登录 shell
//! PATH 上一个真实可写的目录（magpie `rtk_path_other.go` 同款纪律）：
//! 不写 shell profile、不改 PATH 环境变量、不删改用户其它文件。
//!
//! 测试 seam：[`expose_with_query`] 接受注入的搜索 / 登录 PATH / 候选
//! 目录，用例只写临时目录、不写真实用户 home；生产入口
//! [`expose_to_terminal_path`] 从 `path_overlay`（overlay 登记目录 +
//! 登录 shell PATH 快照）与已知落点（`~/.npm-global/bin`、
//! `/usr/local/bin`）构建查询。

use std::path::{Path, PathBuf};

use super::effective_prefix::{canonicalize_or_keep, is_writable_dir, same_dir};
use super::source_table::row_for_wire;
use crate::cross_cutting::path_overlay;

/// HTTP 错误码（与 handler 的 `{code, message}` 面对齐）。
pub const EXPOSE_TARGET_NOT_INSTALLED: &str = "expose_target_not_installed";
pub const EXPOSE_NO_CANDIDATE_DIR: &str = "expose_no_candidate_dir";
pub const EXPOSE_UNSUPPORTED_PLATFORM: &str = "expose_unsupported_platform";
pub const EXPOSE_VERIFY_FAILED: &str = "expose_verify_failed";

/// 已安装目标的查找 / 暴露查询（测试 seam，生产由
/// [`expose_to_terminal_path`] 构建）。
#[derive(Debug, Clone)]
pub struct ExposeQuery {
    /// CLI 命令名（如 `claude`）。
    pub command: String,
    /// 已安装二进制的搜索目录（先 overlay 登记目录，再登录 shell PATH，
    /// 再 `~/.npm-global/bin`、`/usr/local/bin` 等已知落点）。
    pub install_search_dirs: Vec<PathBuf>,
    /// 登录 shell PATH 的目录列表（PATH 顺序）；空 = 探测不可用。
    pub login_path_dirs: Vec<PathBuf>,
    /// 候选暴露目录：`~/.local/bin` → `~/bin` → 登录 shell PATH 上其它
    /// 存在的目录（按 PATH 顺序）。每个候选仍须过准入条件（见
    /// [`expose_with_query`]）。
    pub candidate_dirs: Vec<PathBuf>,
}

/// 暴露结果：软链（或既有同目标残留）所在目录与指向。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposeOutcome {
    /// 暴露的命令名（如 `claude`）。
    pub command: String,
    /// 软链所在目录（登录 shell PATH 上、真实可写）。
    pub dir: PathBuf,
    /// 软链指向的已安装二进制（规范化后的真实路径）。
    pub target: PathBuf,
    /// true = 候选目录里已有指向同一目标的同名残留，本次未新建软链。
    pub already_exposed: bool,
}

/// 暴露失败：`code` 对齐 HTTP 错误码；`message` 必须可直接行动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExposeError {
    pub code: &'static str,
    pub message: String,
}

/// 把 `target`（wire 标识，如 `claude_code`）已安装的 CLI 暴露到用户
/// 终端 PATH（生产入口；安装与更新共用，任意已安装版本可触发）。
///
/// Windows 本轮不支持（不引入新依赖）：明确报错，不静默降级。
pub fn expose_to_terminal_path(target: &str) -> Result<ExposeOutcome, ExposeError> {
    #[cfg(windows)]
    {
        let _ = target;
        return Err(ExposeError {
            code: EXPOSE_UNSUPPORTED_PLATFORM,
            message: "当前平台不支持把 CLI 暴露到终端 PATH（Windows 本轮不支持）".to_string(),
        });
    }
    #[cfg(not(windows))]
    {
        // 未知 wire 标识（含 fake）按「未安装/未知对象」拒绝；端点侧已先
        // 用 `InstallTarget::parse_wire` 过滤，这里兜底。
        let Some(row) = row_for_wire(target) else {
            return Err(ExposeError {
                code: EXPOSE_TARGET_NOT_INSTALLED,
                message: format!("未知安装对象：{target}"),
            });
        };
        expose_with_query(&production_query(row.command))
    }
}

/// seam 核心：按 [`ExposeQuery`] 找已安装二进制 → 依序考察候选目录 →
/// 建（或复用）软链并事后重验。每个候选的准入条件：
/// ① 确实在登录 shell PATH 上；② 目录存在且真实可写（写入探测）；
/// ③ 目录内无同名残留（有同名且已指向同一目标 ⇒ 视为已暴露）。
/// 全部候选不合格 ⇒ 汇总各自原因返回（不静默、不降级）。
pub fn expose_with_query(query: &ExposeQuery) -> Result<ExposeOutcome, ExposeError> {
    // 1. 找已安装二进制（overlay → 登录 shell PATH → 已知落点，由查询
    //    给出顺序）；找不到 ⇒ 「未安装，无法暴露」。
    let installed = find_installed(query).ok_or_else(|| ExposeError {
        code: EXPOSE_TARGET_NOT_INSTALLED,
        message: format!(
            "未找到已安装的 `{}`（已查 PATH overlay、登录 shell PATH 与常见安装落点）；请先安装再暴露",
            query.command
        ),
    })?;

    // 2. 登录 shell PATH 快照不可用（服务刚启动 ≤2s 或探测失败）时无法
    //    证明任何目录「在终端 PATH 上」⇒ 明确让用户稍后重试，不瞎猜。
    if query.login_path_dirs.is_empty() {
        return Err(ExposeError {
            code: EXPOSE_NO_CANDIDATE_DIR,
            message: format!(
                "登录 shell PATH 尚未探测完成（服务启动后 2 秒内或探测失败），无法确认安全的暴露目录；请稍后重试（已安装于 {}）",
                installed.display()
            ),
        });
    }

    let mut failures: Vec<String> = Vec::new();
    for dir in &query.candidate_dirs {
        // ① 确实在登录 shell PATH 上（目录成员语义，规范化比较）。
        if !on_login_path(dir, query) {
            failures.push(format!("「{}」不在登录 shell PATH 上", dir.display()));
            continue;
        }
        if !dir.is_dir() {
            failures.push(format!("「{}」目录不存在", dir.display()));
            continue;
        }
        // ② 真实可写（mode 预筛 + 写入探测，复用 effective_prefix 语义）。
        if !is_writable_dir(dir) {
            failures.push(format!("「{}」真实不可写（写入探测失败）", dir.display()));
            continue;
        }
        // ③ 无同名残留；有同名且已指向同一目标 ⇒ 视为已暴露，直接成功。
        let link = dir.join(&query.command);
        if link_exists(&link) {
            if resolves_to(&link, &installed) {
                return Ok(ExposeOutcome {
                    command: query.command.clone(),
                    dir: dir.clone(),
                    target: installed,
                    already_exposed: true,
                });
            }
            failures.push(format!(
                "「{}」已有同名残留 {}（不指向本次安装目标）；为避免覆盖用户文件已跳过",
                dir.display(),
                link.display(),
            ));
            continue;
        }
        // 动作：建软链 <dir>/<command> → 已安装二进制（只建这一个软链）。
        if let Err(error) = create_symlink(&installed, &link) {
            failures.push(format!("「{}」创建软链失败（{error}）", dir.display()));
            continue;
        }
        // 事后重验：目录确在登录 shell PATH 上且软链可解析到目标；失败
        // ⇒ 只清理本次新建的软链（不动用户其它文件）并如实报错。
        if !on_login_path(dir, query) || !resolves_to(&link, &installed) {
            let leftover = match std::fs::remove_file(&link) {
                Ok(()) => String::new(),
                Err(error) => format!(
                    "；清理失败软链 {} 失败（{error}），可手动删除",
                    link.display()
                ),
            };
            return Err(ExposeError {
                code: EXPOSE_VERIFY_FAILED,
                message: format!(
                    "软链 {} 建立后重验失败（目录不在登录 shell PATH 上或软链不可解析）{leftover}",
                    link.display()
                ),
            });
        }
        return Ok(ExposeOutcome {
            command: query.command.clone(),
            dir: dir.clone(),
            target: installed,
            already_exposed: false,
        });
    }

    // 3. 所有候选都不合格 ⇒ 列出候选与各自原因（可行动）。
    Err(ExposeError {
        code: EXPOSE_NO_CANDIDATE_DIR,
        message: format!(
            "没有可用的暴露目录（已安装于 {}）：{}",
            installed.display(),
            failures.join("；")
        ),
    })
}

/// 生产查询构建：**服务进程 PATH**（`effective_path` 合并后的真实生效序）
/// → 登录 shell PATH 快照 → 已知落点；候选顺序 `~/.local/bin` → `~/bin`
/// → 登录 shell PATH 上其它存在目录。
fn production_query(command: &str) -> ExposeQuery {
    let service_path = path_overlay::effective_path(None);
    let login_path = path_overlay::login_shell_path();
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    production_query_with(
        command,
        &service_path,
        login_path.as_deref(),
        home.as_deref(),
    )
}

/// [`production_query`] 的 seam（服务进程 PATH / 登录 shell PATH / HOME 注入）：
/// 查找序口径只有一份，测试注入与生产走同一条路径。
fn production_query_with(
    command: &str,
    service_path: &str,
    login_path: Option<&str>,
    home: Option<&Path>,
) -> ExposeQuery {
    let login_path_dirs: Vec<PathBuf> = login_path.map(split_path_dirs).unwrap_or_default();

    // 已安装二进制搜索：**服务进程 PATH 在前**（服务自己跑得到的 CLI 必须
    // 能暴露，不得误报「未安装」）→ 登录 shell PATH → 已知落点。
    let mut install_search_dirs = split_path_dirs(service_path);
    extend_unique(&mut install_search_dirs, login_path_dirs.iter().cloned());
    if let Some(home) = home {
        extend_unique(
            &mut install_search_dirs,
            std::iter::once(home.join(".npm-global/bin")),
        );
    }
    extend_unique(
        &mut install_search_dirs,
        std::iter::once(PathBuf::from("/usr/local/bin")),
    );

    // 候选：~/.local/bin → ~/bin → 登录 shell PATH 上其它存在目录（按
    // PATH 顺序）；「确实在登录 shell PATH 上」由 expose_with_query 的
    // 准入条件①再验（home 候选不在 PATH 上时会被拒并说明）。
    let mut candidate_dirs = Vec::new();
    if let Some(home) = home {
        candidate_dirs.push(home.join(".local/bin"));
        candidate_dirs.push(home.join("bin"));
    }
    for dir in &login_path_dirs {
        if dir.is_dir() {
            extend_unique(&mut candidate_dirs, std::iter::once(dir.clone()));
        }
    }

    ExposeQuery {
        command: command.to_string(),
        install_search_dirs,
        login_path_dirs,
        candidate_dirs,
    }
}

/// 登录 shell PATH 原始串（冒号分隔）→ 目录列表（保序、去空段）。
fn split_path_dirs(value: &str) -> Vec<PathBuf> {
    std::env::split_paths(value)
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect()
}

/// 去重追加（首次出现优先）。
fn extend_unique<I>(target: &mut Vec<PathBuf>, items: I)
where
    I: IntoIterator<Item = PathBuf>,
{
    for item in items {
        if !target.contains(&item) {
            target.push(item);
        }
    }
}

/// 搜索目录里第一个真实存在的 `<dir>/<command>`（含软链解析后存在；
/// 断链不算已安装），返回规范化路径。
fn find_installed(query: &ExposeQuery) -> Option<PathBuf> {
    query
        .install_search_dirs
        .iter()
        .map(|dir| dir.join(&query.command))
        .find(|path| path.exists())
        .map(|path| canonicalize_or_keep(&path))
}

/// 候选目录是否在登录 shell PATH 上（目录成员语义，规范化比较）。
fn on_login_path(dir: &Path, query: &ExposeQuery) -> bool {
    query.login_path_dirs.iter().any(|path| same_dir(path, dir))
}

/// 路径上有东西吗（含断链：断链也是需要如实说明的「残留」）。
fn link_exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// `link` 解析后是否指向 `installed`（规范化比较；断链 = false）。
fn resolves_to(link: &Path, installed: &Path) -> bool {
    std::fs::canonicalize(link)
        .map(|real| real == installed)
        .unwrap_or(false)
}

/// 建软链（Unix）；其它平台返回明确错误（不引入新依赖）。
#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn create_symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("当前平台不支持创建软链"))
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink as make_symlink};
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;

    fn make_query(
        command: &str,
        install_search_dirs: Vec<PathBuf>,
        login_path_dirs: Vec<PathBuf>,
        candidate_dirs: Vec<PathBuf>,
    ) -> ExposeQuery {
        ExposeQuery {
            command: command.to_string(),
            install_search_dirs,
            login_path_dirs,
            candidate_dirs,
        }
    }

    fn install_binary(root: &Path, name: &str) -> PathBuf {
        let dir = root.join("installs");
        std::fs::create_dir_all(&dir).expect("create install dir");
        let binary = dir.join(name);
        std::fs::write(&binary, b"#!/bin/sh\n").expect("write binary");
        binary
    }

    fn make_dir(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("create candidate dir");
        dir
    }

    #[test]
    fn expose_fails_when_target_not_installed() {
        let root = tempdir().expect("tempdir");
        let empty = make_dir(root.path(), "empty");
        let query = make_query(
            "claude",
            vec![empty.clone()],
            vec![empty.clone()],
            vec![empty],
        );
        let error = expose_with_query(&query).expect_err("未安装必须报错");
        assert_eq!(error.code, EXPOSE_TARGET_NOT_INSTALLED);
        assert!(
            error.message.contains("claude"),
            "错误必须点名命令：{}",
            error.message
        );
    }

    #[test]
    fn expose_creates_symlink_in_eligible_candidate() {
        let root = tempdir().expect("tempdir");
        let binary = install_binary(root.path(), "claude");
        let install_dir = binary.parent().expect("install dir").to_path_buf();
        let candidate = make_dir(root.path(), "cand-a");
        let query = make_query(
            "claude",
            vec![install_dir],
            vec![candidate.clone()],
            vec![candidate.clone()],
        );
        let outcome = expose_with_query(&query).expect("候选合格必须成功");
        assert!(!outcome.already_exposed);
        assert_eq!(outcome.command, "claude");
        assert_eq!(outcome.dir, candidate);
        assert_eq!(outcome.target, canonicalize_or_keep(&binary));
        let link = candidate.join("claude");
        let metadata = link.symlink_metadata().expect("link exists");
        assert!(metadata.file_type().is_symlink(), "必须是软链");
        assert!(link.exists(), "软链必须可解析到已安装二进制");
    }

    #[test]
    fn expose_skips_candidate_not_on_login_shell_path() {
        let root = tempdir().expect("tempdir");
        let binary = install_binary(root.path(), "claude");
        let install_dir = binary.parent().expect("install dir").to_path_buf();
        let off_path = make_dir(root.path(), "cand-off");
        let on_path = make_dir(root.path(), "cand-on");
        let query = make_query(
            "claude",
            vec![install_dir],
            // 登录 shell PATH 只含 on_path ⇒ off_path 不合格（条件①）。
            vec![on_path.clone()],
            vec![off_path, on_path.clone()],
        );
        let outcome = expose_with_query(&query).expect("on_path 候选必须成功");
        assert_eq!(outcome.dir, on_path, "必须跳过不在登录 PATH 上的候选");
        assert!(!outcome.already_exposed);
    }

    #[test]
    fn expose_rejects_unwritable_candidate_with_actionable_reason() {
        let root = tempdir().expect("tempdir");
        let binary = install_binary(root.path(), "claude");
        let install_dir = binary.parent().expect("install dir").to_path_buf();
        let candidate = make_dir(root.path(), "readonly");
        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o555))
            .expect("chmod 0o555");
        let query = make_query(
            "claude",
            vec![install_dir],
            vec![candidate.clone()],
            vec![candidate.clone()],
        );
        let error = expose_with_query(&query).expect_err("不可写必须拒绝");
        assert_eq!(error.code, EXPOSE_NO_CANDIDATE_DIR);
        assert!(
            error.message.contains("不可写"),
            "必须说明不可写：{}",
            error.message
        );
        assert!(
            error.message.contains(&candidate.display().to_string()),
            "必须列出候选目录：{}",
            error.message
        );
        // 恢复权限，保证 tempdir 清理顺畅。
        let _ = std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o755));
    }

    #[test]
    fn expose_treats_same_target_leftover_as_already_exposed() {
        let root = tempdir().expect("tempdir");
        let binary = install_binary(root.path(), "claude");
        let install_dir = binary.parent().expect("install dir").to_path_buf();
        let candidate = make_dir(root.path(), "cand-a");
        make_symlink(canonicalize_or_keep(&binary), candidate.join("claude"))
            .expect("pre-create same-target symlink");
        let query = make_query(
            "claude",
            vec![install_dir],
            vec![candidate.clone()],
            vec![candidate.clone()],
        );
        let outcome = expose_with_query(&query).expect("同目标残留视为已暴露");
        assert!(outcome.already_exposed);
        assert_eq!(outcome.dir, candidate);
        assert_eq!(outcome.target, canonicalize_or_keep(&binary));
    }

    #[test]
    fn expose_never_touches_leftover_pointing_elsewhere() {
        let root = tempdir().expect("tempdir");
        let binary = install_binary(root.path(), "claude");
        let install_dir = binary.parent().expect("install dir").to_path_buf();
        let other = install_binary(root.path(), "other-cli");
        let candidate = make_dir(root.path(), "cand-a");
        let leftover = candidate.join("claude");
        make_symlink(canonicalize_or_keep(&other), &leftover).expect("pre-create foreign symlink");
        let query = make_query(
            "claude",
            vec![install_dir],
            vec![candidate.clone()],
            vec![candidate.clone()],
        );
        let error = expose_with_query(&query).expect_err("异目标残留必须拒绝");
        assert_eq!(error.code, EXPOSE_NO_CANDIDATE_DIR);
        assert!(
            error.message.contains("同名残留"),
            "必须说明同名残留：{}",
            error.message
        );
        // 绝不删改用户既有文件：残留原样保留。
        assert_eq!(
            std::fs::read_link(&leftover).expect("leftover kept"),
            canonicalize_or_keep(&other)
        );
    }

    #[test]
    fn expose_reports_retry_when_login_shell_path_unknown() {
        let root = tempdir().expect("tempdir");
        let binary = install_binary(root.path(), "claude");
        let install_dir = binary.parent().expect("install dir").to_path_buf();
        let candidate = make_dir(root.path(), "cand-a");
        // 登录 shell PATH 探测不可用（服务刚启动 ≤2s 或探测失败）。
        let query = make_query("claude", vec![install_dir], vec![], vec![candidate]);
        let error = expose_with_query(&query).expect_err("探测不可用必须拒绝");
        assert_eq!(error.code, EXPOSE_NO_CANDIDATE_DIR);
        assert!(
            error.message.contains("稍后重试"),
            "必须给出可行动提示：{}",
            error.message
        );
    }

    /// F8：服务进程 PATH 里有该二进制、登录 shell PATH 没有 ⇒ 生产查询的
    /// 查找序必须以**服务进程 PATH** 打头，expose 能找到它（不得误报
    /// 「未安装」）。
    #[test]
    fn expose_finds_binary_from_service_process_path() {
        let root = tempdir().expect("tempdir");
        let binary = install_binary(root.path(), "claude");
        let install_dir = binary.parent().expect("install dir").to_path_buf();
        let login_bin = make_dir(root.path(), "login-bin");
        let home = root.path().join("home");

        let service_path = install_dir.display().to_string();
        let login_path = login_bin.display().to_string();
        let query = production_query_with(
            "claude",
            &service_path,
            Some(login_path.as_str()),
            Some(home.as_path()),
        );
        assert_eq!(
            query.install_search_dirs.first(),
            Some(&install_dir),
            "服务进程 PATH 必须排在已安装搜索序最前：{:?}",
            query.install_search_dirs
        );

        let outcome = expose_with_query(&query).expect("服务进程 PATH 能找到 ⇒ 不得报「未安装」");
        assert_eq!(
            outcome.dir, login_bin,
            "暴露目录仍是登录 shell PATH 上的候选"
        );
        assert_eq!(outcome.target, binary, "软链目标 = 服务进程 PATH 里的那份");
        assert!(!outcome.already_exposed);
    }
}
