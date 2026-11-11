//! 生效 npm 全局前缀的解析（安装 / 更新落到哪里）。
//!
//! 背景（bug：更新按钮点了没效果）：CLI 往往由 fnm / nvm / asdf 之类的
//! 版本管理器装在**用户自己的 node 前缀**下（`<prefix>/bin/<cmd>` →
//! `../lib/node_modules/<pkg>`），也可能装在系统全局目录。若安装 / 更新
//! 一律落到 `$HOME/.npm-global`，新版本就装在 PATH 之外 ⇒ 用户手上的
//! 命令仍是旧版本（更新「没效果」，界面上的版本号也不变）。
//!
//! 解析顺序（先到先得，只取可写者）：
//! 1. **PATH 上当前生效的 CLI 所在前缀** —— 更新它，用户手上的命令才真的
//!    变成新版本；
//! 2. `npm prefix -g`（npm 自己的全局前缀）—— 未安装时的新装落点；
//! 3. 兜底：`NPM_PREFIX` 环境变量 / `$HOME/.npm-global`（REQ-PCI-03 的
//!    「可写前缀」，避开 sudo）。
//!
//! 全部是纯函数（无 IO 副作用之外的环境读取），便于单测。

use std::path::{Path, PathBuf};

pub const PATH_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// PATH 上第一个提供 `command` 的目录（PATH 顺序 = 生效顺序）。
pub fn first_path_dir_with_command(command: &str, path_value: Option<&str>) -> Option<PathBuf> {
    let path_value = path_value?;
    path_value
        .split(PATH_SEPARATOR)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .find(|dir| dir.join(command).exists())
}

/// PATH 上当前生效的 CLI 所在 npm 前缀（`<prefix>/bin/<cmd>` 布局）。
///
/// 只认 npm 的 `<prefix>/bin/<cmd>` 布局：命令所在目录的父目录必须存在，
/// 否则不猜（避免把 `/usr/local/bin` 这类非 npm 布局误判成前缀）。
/// 前缀取规范化（软链解析）后的真实路径 —— fnm 之类会把
/// `aliases/default` 软链到版本目录，规范后才是 npm 真正的前缀。
pub fn visible_cli_prefix(command: &str, path_value: Option<&str>) -> Option<PathBuf> {
    let bin_dir = first_path_dir_with_command(command, path_value)?;
    let prefix = bin_dir.parent()?.to_path_buf();
    if !prefix.exists() {
        return None;
    }
    Some(canonicalize_or_keep(&prefix))
}

/// 规范化路径（软链解析）；失败时保留原路径。
pub fn canonicalize_or_keep(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 两个目录是不是同一个（按规范化后的真实路径比较）。
pub fn same_dir(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    canonicalize_or_keep(left) == canonicalize_or_keep(right)
}

/// 目录可写吗（目录不存在时看最近的存在祖先）。
pub fn is_writable_dir(path: &Path) -> bool {
    let mut candidate = Some(path);
    while let Some(dir) = candidate {
        match std::fs::metadata(dir) {
            Ok(metadata) if metadata.is_dir() => return dir_is_writable(&metadata),
            Ok(_) => return false,
            Err(_) => candidate = dir.parent(),
        }
    }
    false
}

#[cfg(unix)]
fn dir_is_writable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o222 != 0
}

#[cfg(not(unix))]
fn dir_is_writable(metadata: &std::fs::Metadata) -> bool {
    !metadata.permissions().readonly()
}

/// 选出生效前缀：优先 PATH 上当前生效的 CLI 所在前缀，其次 npm 自己的
/// 全局前缀，最后兜底（兜底可能不可写 ⇒ npm 报权限错 ⇒ 走手动命令兜底）。
pub fn choose_effective_prefix(
    visible: Option<&Path>,
    npm_global: Option<&Path>,
    fallback: &Path,
) -> PathBuf {
    for candidate in [visible, npm_global].into_iter().flatten() {
        if is_writable_dir(candidate) {
            return candidate.to_path_buf();
        }
    }
    fallback.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_stub(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).expect("dir");
        let path = dir.join(name);
        std::fs::write(&path, b"#!/bin/sh\n").expect("stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("executable bit");
        }
    }

    /// PATH 上第一个提供命令的目录 = 生效的那个（PATH 顺序即优先级）。
    #[test]
    fn first_path_dir_with_command_follows_path_order() {
        let root =
            std::env::temp_dir().join(format!("effective-prefix-order-{}", std::process::id()));
        let first = root.join("first/bin");
        let second = root.join("second/bin");
        write_stub(&first, "my-cli-xyz");
        write_stub(&second, "my-cli-xyz");
        let path_value = format!("{}:{}", first.display(), second.display());

        assert_eq!(
            first_path_dir_with_command("my-cli-xyz", Some(&path_value)),
            Some(first.clone())
        );
        assert_eq!(
            first_path_dir_with_command("not-installed-xyz", Some(&path_value)),
            None
        );
        assert_eq!(
            first_path_dir_with_command("my-cli-xyz", None),
            None,
            "PATH 缺失时不得瞎猜"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 生效 CLI 所在目录的父目录 = npm 前缀（`<prefix>/bin/<cmd>` 布局）。
    #[test]
    fn visible_cli_prefix_is_parent_of_bin_dir() {
        let root =
            std::env::temp_dir().join(format!("effective-prefix-visible-{}", std::process::id()));
        let prefix = root.join("installation");
        let bin = prefix.join("bin");
        write_stub(&bin, "my-cli-xyz");
        let path_value = bin.to_string_lossy().into_owned();

        assert_eq!(
            visible_cli_prefix("my-cli-xyz", Some(&path_value)),
            Some(canonicalize_or_keep(&prefix))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 生效前缀优先于 npm 全局前缀（更新它才真的换掉用户手上的命令）。
    #[test]
    fn choose_effective_prefix_prefers_visible_then_npm_global() {
        let root =
            std::env::temp_dir().join(format!("effective-prefix-choose-{}", std::process::id()));
        let visible = root.join("visible");
        let npm_global = root.join("npm-global");
        let fallback = root.join("fallback");
        for dir in [&visible, &npm_global, &fallback] {
            std::fs::create_dir_all(dir).expect("dir");
        }

        assert_eq!(
            choose_effective_prefix(Some(&visible), Some(&npm_global), &fallback),
            visible
        );
        assert_eq!(
            choose_effective_prefix(None, Some(&npm_global), &fallback),
            npm_global
        );
        assert_eq!(choose_effective_prefix(None, None, &fallback), fallback);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 不可写的前缀不得被选中（否则 npm 必然 EACCES）。
    #[test]
    fn choose_effective_prefix_skips_unwritable_candidates() {
        let root = std::env::temp_dir().join(format!("effective-prefix-ro-{}", std::process::id()));
        let readonly = root.join("readonly");
        let writable = root.join("writable");
        let fallback = root.join("fallback");
        for dir in [&writable, &fallback] {
            std::fs::create_dir_all(dir).expect("dir");
        }
        std::fs::create_dir_all(&readonly).expect("dir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o500))
                .expect("readonly bit");
        }

        assert_eq!(
            choose_effective_prefix(Some(&readonly), Some(&writable), &fallback),
            writable,
            "不可写前缀必须跳过"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 不存在的目录按最近的存在祖先判可写（npm 会自己建目录）。
    #[test]
    fn is_writable_dir_falls_back_to_nearest_existing_ancestor() {
        let root =
            std::env::temp_dir().join(format!("effective-prefix-missing-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("dir");
        assert!(is_writable_dir(&root.join("not-created-yet")));
        assert!(!is_writable_dir(Path::new(
            "/proc/1/root/definitely-not-here"
        )));
        let _ = std::fs::remove_dir_all(&root);
    }
}
