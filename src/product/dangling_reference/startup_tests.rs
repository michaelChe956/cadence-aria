use super::*;
use crate::product::app_paths::ProductAppPaths;
use std::path::Path;
use tempfile::TempDir;

/// 写一条文件（建父目录）。
fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body.as_bytes()).unwrap();
}

/// 状态根的**逐文件 digest**（相对路径 + 内容，按相对路径升序）。
///
/// 与 `patrol_tests` 的 `tree_digest` 同口径：「状态根逐文件不变」是
/// tasks 5.1 的验收句原文，仅比对文件存在性不足以钉住「内容未被改写」。
fn tree_digest(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string(),
                    std::fs::read(&path).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
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

    let before = tree_digest(paths.root());
    let outcome = run_startup_governance(&paths, workspace.path()).expect("启动期治理应当成功");
    let after = tree_digest(paths.root());

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
    // tasks 5.1 验收句「状态根逐文件不变」：巡检 + 销账前后逐文件 digest 相同。
    assert_eq!(
        before, after,
        "启动期治理不得改写状态根任何文件（逐文件 digest 必须相同）：{before:?} vs {after:?}"
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

/// 重复启动必须幂等：第二次运行不报错、清单仍可覆盖写，且同一现场连续两次
/// 巡检的发现集合规范化序列化逐字节一致（tasks 2.3：两次令牌相等）。
#[test]
fn startup_governance_is_idempotent_across_restarts() {
    let workspace = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    // 非空现场：证据不足的悬空窗口永不删除，两次巡检看到的是同一现场。
    write(
        &paths.root().join("reviewer-windows/windows/win_idem.json"),
        &window_record("win_idem", "running", "coding_attempt_gone"),
    );

    let first = run_startup_governance(&paths, workspace.path()).expect("首次启动期治理");
    let second = run_startup_governance(&paths, workspace.path()).expect("二次启动期治理");

    assert_eq!(first.report_path, second.report_path, "清单落点固定");
    assert_eq!(
        first.dangling_count, second.dangling_count,
        "两次巡检口径一致"
    );
    assert_eq!(
        first.findings_token, second.findings_token,
        "同一现场连续两次巡检的发现集合规范化序列化 MUST 逐字节一致（令牌相等）"
    );
    assert_eq!(second.deleted_count, 0, "重入零删除");
}

/// 一条 plan 意图记录（引用 story_spec_0005 + plan issue_work_item_plan_0007）。
fn plan_intent_body() -> &'static str {
    r#"{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x"}"#
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

// ─── 「归零自证」落盘与逐条输出（REQ-DRG-17 / REQ-DRG-23）─────────────────

/// 取清单中的某一节（到下一个 `## ` 标题为止）。
fn section<'a>(markdown: &'a str, heading: &str) -> &'a str {
    let start = markdown
        .find(heading)
        .unwrap_or_else(|| panic!("清单缺少「{heading}」节：{markdown}"));
    let rest = &markdown[start..];
    let end = rest[heading.len()..]
        .find("\n## ")
        .map(|offset| heading.len() + offset + 1)
        .unwrap_or(rest.len());
    &rest[..end]
}

/// 自证归零（REQ-DRG-17）：本次可删条目在销账后**仍未归零**时，清单「归零自证」
/// 节与启动日志 MUST 逐条列出该条目，且 MUST NOT 宣称治理完成。
///
/// 夹具：意图文件在状态根内是**符号链接**（真实文件在状态根外）——删除护栏
/// 「目标自身是符号链接 → 不跟随、不删」，因此判定面认为可删、实际删不掉，
/// 销账后巡检仍看到它（未归零可删项）。
#[test]
fn startup_governance_self_proof_lists_unzeroed_deletable_items() {
    let workspace = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    let issue_root = paths.issue_root("project_0001", "issue_0001");
    std::fs::create_dir_all(&issue_root).unwrap();
    let real_intent = workspace.path().join("real-plan-intent.json");
    write(&real_intent, plan_intent_body());
    std::os::unix::fs::symlink(&real_intent, issue_root.join("automation-plan-intent.json"))
        .unwrap();
    write_tombstone(&paths, "story", "story_spec_0005");
    write_tombstone(&paths, "plan", "issue_work_item_plan_0007");
    // 另一条只报不删的悬空窗口（证据不足），用于钉住两节互斥。
    write(
        &paths
            .root()
            .join("reviewer-windows/windows/win_dangling.json"),
        &window_record("win_dangling", "running", "coding_attempt_gone"),
    );

    let outcome = run_startup_governance(&paths, workspace.path()).expect("启动期治理应当成功");

    assert_eq!(outcome.deleted_count, 0, "符号链接不跟随、不删");
    assert!(
        !outcome.unzeroed.is_empty(),
        "未归零可删项必须逐条保留：{outcome:?}"
    );
    assert!(
        outcome
            .unzeroed
            .iter()
            .all(|finding| finding.record_path.ends_with("automation-plan-intent.json")),
        "未归零项必须是被判定可删却未删掉的意图记录：{outcome:?}"
    );
    // 口径与既有 `zeroed_by_type` 一致：按 finding 逐条计数（该记录的全部悬空引用）。
    assert_eq!(
        outcome
            .unzeroed_by_type
            .get("automation_plan_intent")
            .copied(),
        Some(outcome.unzeroed.len()),
        "归零计数必须按记录类型给出且与逐条数一致：{outcome:?}"
    );
    assert!(
        !outcome.governance_complete,
        "存在未归零可删项时 MUST NOT 宣称治理完成：{outcome:?}"
    );
    assert_eq!(
        outcome.residual.len(),
        1,
        "只报不删残留独立计数：{outcome:?}"
    );

    // 清单：销账后的自证现场与销账前三节分节并列、互不混列。
    let content = std::fs::read_to_string(&outcome.report_path).expect("读取巡检清单");
    let self_proof = section(&content, "## 6. 归零自证（销账后现场）");
    assert!(
        self_proof.contains(&format!(
            "| automation_plan_intent | {} |",
            outcome.unzeroed.len()
        )),
        "6.1 必须给出本次可删类型的归零计数：{self_proof}"
    );
    assert!(
        self_proof.contains("automation-plan-intent.json"),
        "6.2 必须逐条列出未归零可删项：{self_proof}"
    );
    assert!(
        self_proof.contains("治理完成：否"),
        "6.4 未完成时必须写明「否」：{self_proof}"
    );
    assert!(
        !self_proof.contains("治理完成：是"),
        "未归零时 MUST NOT 宣称治理完成：{self_proof}"
    );
    let residual_section = section(&content, "## 3. 只报不删残留");
    assert!(
        residual_section.contains("win_dangling"),
        "销账前残留节列出证据不足的窗口：{residual_section}"
    );
    assert!(
        !self_proof.contains("win_dangling"),
        "归零自证节 MUST NOT 混列只报不删残留：{self_proof}"
    );

    // 启动日志：未归零项 MUST 逐条列出（不只打计数）。
    let lines = outcome.log_lines();
    assert!(
        lines.iter().any(|line| line.contains("NOT zeroed")
            && line.contains("automation-plan-intent.json")),
        "启动日志必须逐条列出未归零可删项：{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("governance_complete=false")),
        "启动日志必须如实标注治理未完成：{lines:?}"
    );
}

/// 自证归零（REQ-DRG-23）：存在不可判定条目时清单「归零自证」节与启动日志 MUST
/// 逐条列出条目与失败原因，且 MUST NOT 宣称治理完成。
#[test]
fn startup_governance_self_proof_lists_undetermined_items() {
    let workspace = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(workspace.path().join(".aria"));
    write(
        &paths.root().join("reviewer-windows/windows/win_bad.json"),
        "{ this is not json",
    );

    let outcome = run_startup_governance(&paths, workspace.path()).expect("启动期治理应当成功");

    assert_eq!(
        outcome.undetermined.len(),
        1,
        "不可判定条目必须逐条保留：{outcome:?}"
    );
    assert_eq!(outcome.undetermined_count, 1, "销账前不可判定计数不变");
    assert!(
        !outcome.governance_complete,
        "存在不可判定项时 MUST NOT 宣称治理完成：{outcome:?}"
    );

    let content = std::fs::read_to_string(&outcome.report_path).expect("读取巡检清单");
    let self_proof = section(&content, "## 6. 归零自证（销账后现场）");
    assert!(
        self_proof.contains("win_bad.json"),
        "6.3 必须逐条列出不可判定条目：{self_proof}"
    );
    assert!(
        self_proof.contains(&outcome.undetermined[0].reason),
        "6.3 必须写明失败原因：{self_proof}"
    );
    assert!(
        self_proof.contains("治理完成：否"),
        "存在不可判定项时 MUST NOT 宣称治理完成：{self_proof}"
    );

    let lines = outcome.log_lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("UNDETERMINED") && line.contains("win_bad.json")),
        "启动日志必须逐条列出不可判定项：{lines:?}"
    );
}
