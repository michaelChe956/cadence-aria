use super::*;
use crate::product::app_paths::ProductAppPaths;
use std::path::Path;
use tempfile::TempDir;

/// 写一条文件（建父目录）。
fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body.as_bytes()).unwrap();
}

/// 状态根下的全部文件（相对路径升序）——用于「逐文件不变」断言。
fn state_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    files
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
            continue;
        }
        files.push(
            path.strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string(),
        );
    }
}

/// 一条指向已删 attempt 的 reviewer 窗口记录（`state` 决定 L2 条件①是否成立）。
fn window_record(window_id: &str, state: &str, attempt_id: &str) -> String {
    format!(
        r#"{{"schema":"reviewer_window_v1","window_id":"{window_id}","generation":1,"identity":{{"canonical_worktree":"/tmp/none","gitdir":"/tmp/none/.git","common_dir":"/tmp/none","lease_id":"lease_1"}},"attempt_id":"{attempt_id}","run_id":"run_1","provider":"p","state":"{state}","report_acceptance":"accepted","platform_write_paths":[],"quiesced_scopes":[],"baseline":null,"restore":null,"recovery":null,"last_error":null,"violation_pending":null,"pre_restore_git_facts":[],"transitions":[],"created_at":"2026-10-24T00:00:00Z","updated_at":"2026-10-24T00:00:00Z"}}"#
    )
}

/// 启动期治理必须在 `cadence/analysis-docs/` 落下按运行日命名的巡检清单，
/// 且空现场零删除零留痕（状态根逐文件不变）。
#[test]
fn startup_governance_writes_manifest_into_analysis_docs_without_touching_state_root() {
    let workspace = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    std::fs::create_dir_all(paths.root()).unwrap();

    let outcome = run_startup_governance(&paths, workspace.path()).expect("启动期治理应当成功");

    let expected = workspace
        .path()
        .join("cadence")
        .join("analysis-docs")
        .join(report_file_name(&run_day()));
    assert_eq!(outcome.report_path, expected, "清单落点必须是分析报告目录");
    assert!(expected.exists(), "巡检清单必须落盘：{expected:?}");
    let content = std::fs::read_to_string(&expected).expect("读取巡检清单");
    assert!(
        content.contains(outcome.findings_token.as_str()),
        "清单必须附带摘要令牌：{content}"
    );
    assert_eq!(outcome.dangling_count, 0, "空现场无悬空条目");
    assert_eq!(outcome.undetermined_count, 0, "空现场无不可判定条目");
    assert_eq!(outcome.deleted_count, 0, "空现场零删除");
    assert!(
        !paths
            .root()
            .join("state/entity-deletions/dangling-reference/cleanup.jsonl")
            .exists(),
        "零删除不得留痕"
    );
}

/// 悬空窗口若证据不足（窗口非终态 → L2 条件①不成立）→ 只报不删：清单列出该
/// 条目、删除数为零、窗口记录仍在。
#[test]
fn startup_governance_reports_dangling_window_without_deleting_when_evidence_insufficient() {
    let workspace = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    let window_path = paths
        .root()
        .join("reviewer-windows/windows/win_dangling.json");
    write(
        &window_path,
        &window_record("win_dangling", "running", "coding_attempt_gone"),
    );

    let outcome = run_startup_governance(&paths, workspace.path()).expect("启动期治理应当成功");

    assert_eq!(outcome.dangling_count, 1, "悬空窗口必须计入发现数");
    assert_eq!(outcome.deleted_count, 0, "证据不足只报不删");
    assert!(window_path.exists(), "只报不删的窗口记录必须保留");
    let content = std::fs::read_to_string(&outcome.report_path).expect("读取巡检清单");
    assert!(
        content.contains("win_dangling"),
        "清单必须列出该悬空条目：{content}"
    );
}

/// 重复启动必须幂等：第二次运行不报错、清单仍可覆盖写。
#[test]
fn startup_governance_is_idempotent_across_restarts() {
    let workspace = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    std::fs::create_dir_all(paths.root()).unwrap();

    let first = run_startup_governance(&paths, workspace.path()).expect("首次启动期治理");
    let second = run_startup_governance(&paths, workspace.path()).expect("二次启动期治理");

    assert_eq!(first.report_path, second.report_path, "清单落点固定");
    assert_eq!(
        first.dangling_count, second.dangling_count,
        "两次巡检口径一致"
    );
    assert_eq!(second.deleted_count, 0, "重入零删除");
}
