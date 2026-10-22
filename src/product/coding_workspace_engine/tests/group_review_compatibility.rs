//! Task 12（设计 B §三 A1 / REQ-GFR-CR-01）：GroupFinalReview 执行入口移除守卫。
//!
//! `execute_group_final_review_with_commands` 原是测试专用的 GroupFinalReview
//! 执行入口（生产路径走 `execute_internal_pr_review_with_commands`），随
//! `legacy_group_final_review.inc.rs` 一并删除。原「legacy hash 兼容」
//! 两个用例因入口删除同步移除；`group_review_store` /
//! `group_review_orchestrator` / `group_review_material` 整族保留，
//! 由各自直测与 `group_review_e2e` 继续覆盖。

/// 遍历 `src/` 下全部 `.rs` 源文件（含 `.inc.rs` 片段）。
fn collect_rust_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// 路径任一层为 `tests` 目录即视为测试代码（非生产面）。
fn is_test_path(path: &std::path::Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "tests")
}

/// 断言：① `internal_pr_review.rs` 不再 include
/// `legacy_group_final_review.inc.rs`；② 仓内
/// `grep -rn "execute_group_final_review_with_commands" src/` 无生产命中。
#[test]
fn group_final_review_execution_entry_is_removed() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // ① internal_pr_review.rs 不得再 include legacy_group_final_review.inc.rs
    let internal_pr_review = manifest_dir
        .join("src/product/coding_workspace_engine/internal_pr_review.rs");
    let content = std::fs::read_to_string(&internal_pr_review)
        .expect("internal_pr_review.rs must be readable");
    assert!(
        !content.contains("legacy_group_final_review"),
        "internal_pr_review.rs 不得再 include legacy_group_final_review.inc.rs"
    );

    // ② 仓内 grep "execute_group_final_review_with_commands" 无生产命中
    let mut sources = Vec::new();
    collect_rust_sources(&manifest_dir.join("src"), &mut sources);
    let production_hits: Vec<String> = sources
        .into_iter()
        .filter(|path| !is_test_path(path))
        .filter_map(|path| {
            let body = std::fs::read_to_string(&path).ok()?;
            body.contains("execute_group_final_review_with_commands")
                .then(|| path.display().to_string())
        })
        .collect();
    assert!(
        production_hits.is_empty(),
        "execute_group_final_review_with_commands 不得出现在生产源码: {production_hits:?}"
    );
}
