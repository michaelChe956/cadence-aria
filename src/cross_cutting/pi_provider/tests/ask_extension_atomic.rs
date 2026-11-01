//! `ensure_ask_extension_in` 的**并发原子发布**回归（2026-11-01 现场）。
//!
//! 原实现先 `create_new` 建空文件、再 `write_all`：并发调用者命中
//! `AlreadyExists` 分支去 `validate` 时会读到**空文件或写了另一半的内容**，
//! 直接报 `existing extension content does not match Aria's extension`。
//! 该竞态在 CI 并行测试（实测 `pi_b1_lc_validated_launch_*`）与真实 daemon
//! 多会话同时起 pi 时都会触发。
//!
//! 修复为「同目录临时文件 + rename 原子发布」。本用例用多线程强并发地打
//! 同一份 cache 目录，断言：全线程成功 + 落盘内容完整 + 目录里不残留临时文件。

use super::*;

#[test]
fn concurrent_ensure_ask_extension_publishes_complete_content_atomically() {
    let cache = tempfile::tempdir().expect("cache dir");
    // 每次都用全新目录，确保没有已存在的正确文件可以「碰巧」短路。
    let cache_path = cache.path().join("cadence-aria");
    let threads = 16;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(threads));
    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let cache_path = cache_path.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            super::ensure_ask_extension_in(&cache_path)
        }));
    }
    let mut published = Vec::with_capacity(threads);
    for handle in handles {
        published.push(handle.join().expect("thread must not panic"));
    }
    for result in &published {
        let path = result
            .as_ref()
            .unwrap_or_else(|error| panic!("concurrent ensure must succeed: {error:?}"));
        assert!(
            path.is_file(),
            "published extension must be a regular file: {}",
            path.display(),
        );
    }
    // 全部指向同一份文件，且内容就是当前常量（完整、未被截断）。
    let first = published[0].as_ref().expect("first publish");
    assert!(
        published
            .iter()
            .all(|result| result.as_ref().is_ok_and(|path| path == first)),
        "all concurrent callers must converge on the same extension path",
    );
    let content = std::fs::read_to_string(first).expect("read published extension");
    assert_eq!(
        content.len(),
        ARIA_ASK_EXTENSION.len(),
        "published extension must be complete (no partially written content)",
    );
    // 不残留临时文件（失败路径 best-effort 清理；成功路径必须已 rename 掉）。
    let leftovers: Vec<_> = std::fs::read_dir(&cache_path)
        .expect("list cache dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary publish files must not be left behind: {leftovers:?}",
    );
}
