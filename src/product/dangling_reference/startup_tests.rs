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

/// 写一条实体身份台账绑定（`state/entity-identity/sessions/<session_id>.json`）。
fn write_binding(paths: &ProductAppPaths, session_id: &str, kind: &str, entity_id: &str) {
    let dir = paths.root().join("state/entity-identity/sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let body = format!(
        r#"{{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"{kind}","entity_id":"{entity_id}"}}"#
    );
    std::fs::write(dir.join(format!("{session_id}.json")), body).unwrap();
}

/// 写一份 L1 墓碑（issue 级 `entity-deletions/<kind>/<id>.json`）。
fn write_tombstone(paths: &ProductAppPaths, kind: &str, id: &str) {
    let path = paths
        .issue_root("project_0001", "issue_0001")
        .join("entity-deletions")
        .join(kind)
        .join(format!("{id}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body = format!(
        r#"{{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"{kind}","entity_id":"{id}","phase":"deleted","started_at":"2026-10-28T10:00:00Z","deleted_at":"2026-10-28T10:05:00Z"}}"#
    );
    std::fs::write(&path, body).unwrap();
}

/// 缺陷复现（实测）：自动模式实删 6 条，清单却写「模式：dry-run（未执行任何
/// 删除）」。清单模式行 MUST 如实标注自动模式与本次实际删除数（REQ-DRG-11）。
#[test]
fn startup_governance_report_mode_line_records_actual_deletions() {
    let workspace = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    write_binding(&paths, "s_l1", "plan", "issue_work_item_plan_0007");
    write_tombstone(&paths, "plan", "issue_work_item_plan_0007");

    let outcome = run_startup_governance(&paths, workspace.path()).expect("启动期治理应当成功");

    assert_eq!(outcome.deleted_count, 1, "L1 台账条目必须被删：{outcome:?}");
    let content = std::fs::read_to_string(&outcome.report_path).expect("读取巡检清单");
    assert!(
        content.contains("自动销账（服务启动时自动执行；本次已删除 1 条）"),
        "清单模式行必须如实写明自动模式与删除数：{content}"
    );
    assert!(
        !content.contains("dry-run（未执行任何删除）"),
        "自动模式 MUST NOT 谎报未执行删除：{content}"
    );
    assert!(
        content.contains(".aria/state/entity-deletions/dangling-reference/cleanup.jsonl"),
        "清单必须指向留痕台账：{content}"
    );
}

/// 销账失败时清单仍然写出，且模式行 MUST 写明失败原因、MUST NOT 声称未删除
/// ——失败可能发生在部分删除之后，逐条真相以留痕台账为准。
#[test]
fn startup_governance_report_mode_line_admits_cleanup_failure() {
    let workspace = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    write_binding(&paths, "s_l1", "plan", "issue_work_item_plan_0007");
    write_tombstone(&paths, "plan", "issue_work_item_plan_0007");
    // 用普通文件占住台账目录位 → 销账在「删除后留痕」处失败（即部分删除后失败）。
    std::fs::write(paths.state_root().join("entity-deletions"), b"blocked").unwrap();

    let error = run_startup_governance(&paths, workspace.path()).expect_err("销账应当失败");
    assert!(!error.to_string().is_empty());

    let report_path = workspace
        .path()
        .join("cadence")
        .join("analysis-docs")
        .join(report_file_name(&run_day()));
    let content = std::fs::read_to_string(&report_path).expect("销账失败时清单仍必须写出");
    assert!(
        content.contains("销账失败（错误："),
        "清单模式行必须写明销账失败原因：{content}"
    );
    assert!(
        !content.contains("未删除任何记录"),
        "销账失败 MUST NOT 谎称未删除：{content}"
    );
    assert!(
        !content.contains("dry-run（未执行任何删除）"),
        "销账失败 MUST NOT 沿用 dry-run 文案：{content}"
    );
}
