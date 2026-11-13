//! 共享 PATH overlay（计划 2026-11-13 包 2.4/2.4b/2.4c）。
//!
//! 平台「命令可见 PATH」的进程级注册表：装完 CLI 的 bin 目录、启动期发现的
//! 用户工具目录都登记进来；两个命令汇聚点（`process_manager.rs` 的
//! `spawn_with_environment` 与 `cli_adapter.rs` 的 `run_command_capture_with_stream`）
//! 在构造子进程 env 时用 [`effective_path`] 合并 overlay。
//!
//! **不改进程真实 env**（仓库 edition 2024，`std::env::set_var` 是 `unsafe fn`
//! 且多线程改全局 env 有竞态）——只提供合并后的 PATH 给子进程。overlay 是
//! 进程级共享状态：只追加、可去重、测试可清空；一切触碰注册表的用例必须持
//! [`overlay_test_lock`]（或 [`OverlayStateGuard::acquire`]）执行并在结束清空
//! （计划 §6.1 测试隔离纪律），否则会污染并行的 PATH 断言用例。

use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};

/// overlay 目录的合并位置：装完的目录用 `Prepend`（置于合并 PATH 最前）；
/// 启动期发现的目录用 `Append`（不抢占用户已有工具）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayPosition {
    Prepend,
    Append,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OverlayEntry {
    dir: PathBuf,
    position: OverlayPosition,
}

fn registry() -> &'static Mutex<Vec<OverlayEntry>> {
    static REGISTRY: LazyLock<Mutex<Vec<OverlayEntry>>> =
        LazyLock::new(|| Mutex::new(Vec::new()));
    &REGISTRY
}

/// 登记一个目录进 overlay。不存在的目录（含普通文件）被忽略；同一目录
/// 重复登记只保留首次的位置（去重，只追加）。只动注册表：不改进程真实
/// env、不写盘、不创建目录。
pub fn register(dir: &Path, position: OverlayPosition) {
    let _ = (dir, position);
}

/// 当前 overlay 的目录快照（登记顺序）。
pub fn snapshot() -> Vec<PathBuf> {
    Vec::new()
}

/// 合并 overlay 与 base 成子进程 PATH：`Prepend` 目录置前（登记顺序），
/// `Append` 目录其后（登记顺序），再接 base 各目录；整体去重、保留首次
/// 出现。`base` 缺省取进程 env PATH；overlay 为空时原样返回 base（零行为
/// 变化，既有调用方语义不受影响）。
pub fn effective_path(base: Option<&str>) -> String {
    match base {
        Some(value) => value.to_string(),
        None => std::env::var_os("PATH")
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    }
}

/// 测试用：清空 overlay 注册表。
#[cfg(test)]
pub fn clear() {}

/// 进程级 overlay 测试锁：一切触碰注册表的用例（含其他文件里的合并点
/// 断言）必须持本锁执行，避免并行用例互相污染（计划 §6.1）。
#[cfg(test)]
pub(crate) fn overlay_test_lock() -> &'static Mutex<()> {
    static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    &LOCK
}

/// 持全局锁 + 进入时清空 + 退出（含 panic 展开）时清空的 RAII 守卫。
#[cfg(test)]
pub(crate) struct OverlayStateGuard(MutexGuard<'static, ()>);

#[cfg(test)]
impl OverlayStateGuard {
    pub(crate) fn acquire() -> Self {
        let guard = overlay_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clear();
        Self(guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_ignores_missing_directories() {
        let _state = OverlayStateGuard::acquire();

        let missing = std::env::temp_dir().join("aria-path-overlay-definitely-missing");
        let _ = std::fs::remove_dir_all(&missing);
        register(&missing, OverlayPosition::Prepend);

        let holder = tempfile::tempdir().expect("tempdir");
        let plain_file = holder.path().join("plain-file");
        std::fs::write(&plain_file, "not a dir").expect("write file");
        register(&plain_file, OverlayPosition::Append);

        assert!(
            snapshot().is_empty(),
            "不存在/非目录的路径不得登记: {:?}",
            snapshot()
        );
        assert!(!missing.exists(), "登记不得创建目录");
    }

    #[test]
    fn register_deduplicates_same_directory() {
        let _state = OverlayStateGuard::acquire();

        let dir = tempfile::tempdir().expect("tempdir");
        register(dir.path(), OverlayPosition::Prepend);
        // 同目录重登（换位置）→ 去重，保留首次的 Prepend。
        register(dir.path(), OverlayPosition::Append);

        assert_eq!(snapshot(), vec![dir.path().to_path_buf()]);
        let merged = effective_path(Some("/usr/bin:/bin"));
        assert_eq!(
            std::env::split_paths(&merged).next(),
            Some(dir.path().to_path_buf()),
            "首次登记的 Prepend 位置必须保留: {merged}"
        );
    }

    #[test]
    fn effective_path_orders_prepend_then_append_then_base_deduped() {
        let _state = OverlayStateGuard::acquire();

        let root = tempfile::tempdir().expect("tempdir");
        let a = root.path().join("a");
        let b = root.path().join("b");
        let c = root.path().join("c");
        let base_dir = root.path().join("base");
        for dir in [&a, &b, &c, &base_dir] {
            std::fs::create_dir_all(dir).expect("mkdir");
        }

        register(&a, OverlayPosition::Prepend);
        register(&b, OverlayPosition::Append);
        register(&c, OverlayPosition::Prepend);

        let base = format!(
            "{base}:{a}:{base}",
            base = base_dir.display(),
            a = a.display()
        );
        let merged = effective_path(Some(&base));
        let entries: Vec<PathBuf> = std::env::split_paths(&merged).collect();

        assert_eq!(
            entries,
            vec![a.clone(), c.clone(), b.clone(), base_dir.clone()],
            "Prepend(登记序) → Append(登记序) → base(去重保首次): {merged}"
        );
    }

    #[test]
    fn effective_path_returns_base_unchanged_when_overlay_empty() {
        let _state = OverlayStateGuard::acquire();

        assert_eq!(effective_path(Some("/usr/bin:/bin")), "/usr/bin:/bin");
        assert_eq!(effective_path(Some("")), "");
        assert_eq!(
            effective_path(None),
            std::env::var("PATH").unwrap_or_default(),
            "overlay 为空时 base 缺省取进程 env PATH 且原样返回"
        );
    }

    #[test]
    fn clear_empties_registry() {
        let _state = OverlayStateGuard::acquire();

        let dir = tempfile::tempdir().expect("tempdir");
        register(dir.path(), OverlayPosition::Prepend);
        assert_eq!(snapshot().len(), 1);

        clear();
        assert!(snapshot().is_empty());
        assert_eq!(
            effective_path(Some("/usr/bin:/bin")),
            "/usr/bin:/bin",
            "清空后合并退化为 base 原值"
        );
    }
}
