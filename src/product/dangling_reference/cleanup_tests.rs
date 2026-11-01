use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::dangling_reference::report::findings_token;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// ─── 夹具 ───────────────────────────────────────────────────────────

fn plan_intent_body() -> &'static str {
    r#"{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x"}"#
}

/// 台账 / enrollment 夹具的原始 JSON 模板（`__ENABLED__` 由调用方替换）。
const ENROLLMENT_JSON: &str = r#"{"enrollment_id":"e1","selection_key":"selection_1","project_id":"project_0001","issue_id":"issue_0001","enabled":__ENABLED__,"policy_revision":1,"source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"prepare_intent_id":"p1","plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x","created_at":"2026-10-28T09:00:00Z","updated_at":"2026-10-28T09:00:00Z"}"#;

fn write_enrollment(issue_root: &Path, enabled: bool, terminated: bool) {
    let flag = if enabled { "true" } else { "false" };
    let text = ENROLLMENT_JSON.replace("__ENABLED__", flag);
    let mut body: serde_json::Value = serde_json::from_str(&text).unwrap();
    if terminated {
        body["terminated_at"] = serde_json::json!("2026-10-28T09:30:00Z");
    }
    std::fs::write(issue_root.join(ENROLLMENT_FILE), body.to_string()).unwrap();
}

fn tombstone_path(root: &Path, kind: &str, id: &str) -> PathBuf {
    root.join("projects/project_0001/issues/issue_0001/entity-deletions")
        .join(kind)
        .join(format!("{id}.json"))
}

fn write_tombstone(root: &Path, kind: &str, id: &str) {
    let path = tombstone_path(root, kind, id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body = format!(
        r#"{{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"{kind}","entity_id":"{id}","phase":"deleted","started_at":"2026-10-28T10:00:00Z","deleted_at":"2026-10-28T10:05:00Z"}}"#
    );
    std::fs::write(&path, body).unwrap();
}

fn write_window(
    paths: &ProductAppPaths,
    window_id: &str,
    attempt_id: &str,
    worktree: &Path,
    state: &str,
) {
    let dir = paths.root().join("reviewer-windows/windows");
    std::fs::create_dir_all(&dir).unwrap();
    let worktree = worktree.display();
    let body = format!(
        r#"{{"schema":"reviewer_window_v1","window_id":"{window_id}","generation":1,"identity":{{"canonical_worktree":"{worktree}","gitdir":"{worktree}/.git","common_dir":"{worktree}","lease_id":"lease_1"}},"attempt_id":"{attempt_id}","run_id":"run_1","provider":"codex","state":"{state}","report_acceptance":"accepted","platform_write_paths":[],"quiesced_scopes":[],"baseline":null,"restore":null,"recovery":null,"last_error":null,"violation_pending":null,"pre_restore_git_facts":[],"transitions":[],"created_at":"2026-10-24T00:00:00Z","updated_at":"2026-10-24T00:00:00Z"}}"#
    );
    std::fs::write(dir.join(format!("{window_id}.json")), body).unwrap();
}

fn write_binding(paths: &ProductAppPaths, session_id: &str, kind: &str, entity_id: &str) {
    let dir = paths
        .state_root()
        .join(ENTITY_IDENTITY_DIR)
        .join(ENTITY_SESSIONS_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    let body = format!(
        r#"{{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"{kind}","entity_id":"{entity_id}"}}"#
    );
    std::fs::write(dir.join(format!("{session_id}.json")), body).unwrap();
}

/// 用户业务数据（正文 / versions / artifact / work-item / attempt）：销账 MUST NOT 删除。
fn write_user_business_data(root: &Path) -> Vec<PathBuf> {
    let issue_root = root.join("projects/project_0001/issues/issue_0001");
    let mut written = Vec::new();
    for (relative, body) in [
        ("stories/story_spec_0005.json", "{\"id\":\"s0005\"}"),
        ("designs/design_spec_0001.json", "{\"id\":\"d0001\"}"),
        ("plans/issue_work_item_plan_0007.json", "{\"id\":\"p0007\"}"),
        ("versions/story_spec_0005/v0001.json", "{\"version\":1}"),
        ("artifacts/s0005_d0001.md", "# artifact"),
        ("work-items/work_item_0001.json", "{\"id\":\"w0001\"}"),
        (
            "coding-attempts/coding_attempt_9f3.json",
            "{\"id\":\"a9f3\"}",
        ),
    ] {
        let path = issue_root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        written.push(path);
    }
    written
}

/// 建好 issue 目录并写入 plan intent，返回 (paths, issue_root)。
fn plan_intent_issue(root: &Path, body: &str) -> (ProductAppPaths, PathBuf) {
    let paths = ProductAppPaths::new(root);
    let issue_root = paths.issue_root("project_0001", "issue_0001");
    std::fs::create_dir_all(&issue_root).unwrap();
    std::fs::write(issue_root.join(PLAN_INTENT_FILE), body).unwrap();
    (paths, issue_root)
}

/// 入口内部重算的确认令牌（= 当前现场 findings 摘要）。
fn current_token(paths: &ProductAppPaths) -> String {
    let report = patrol_state_root(paths).expect("patrol");
    findings_token(&report.findings)
}

/// 自动模式请求（无令牌、无人工确认）。
fn automatic_request() -> CleanupRequest {
    CleanupRequest::automatic()
}

fn ledger_lines(paths: &ProductAppPaths) -> Vec<String> {
    std::fs::read_to_string(paths.dangling_reference_ledger_path())
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect()
}

fn probe_missing() -> impl Fn(&str) -> Result<bool, ProductStoreError> {
    |_id: &str| Ok(false)
}

// ─── 闸 1：自动模式（无人工令牌环节）──────────────────────────────

/// 自动模式**不需要任何令牌**：证据达标即删——清单摘要不再是删除凭据
/// （REQ-DRG-22：仅用于审计关联）。
#[test]
fn dangling_cleanup_automatic_mode_needs_no_token() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    let intent_path = issue_root.join(PLAN_INTENT_FILE);

    let outcome = run_cleanup(&paths, &automatic_request()).expect("自动模式应当成功");
    assert_eq!(outcome.deleted_count, 1, "{outcome:?}");
    assert!(!intent_path.exists(), "白名单内 intent 记录必须被删");
    assert_eq!(ledger_lines(&paths).len(), 1, "实际删除必须留痕一行");
}

#[test]
fn dangling_cleanup_dry_run_never_deletes() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let request = CleanupRequest::report_only();
    let outcome = run_cleanup(&paths, &request).expect("dry run");
    assert_eq!(outcome.deleted_count, 0);
    assert!(
        !paths.dangling_reference_ledger_path().exists(),
        "dry-run 零留痕"
    );
}

/// dry-run 只出清单：零删除、零留痕，且不得宣称治理完成。
#[test]
fn dangling_cleanup_report_only_never_deletes_record() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    let intent_path = issue_root.join(PLAN_INTENT_FILE);
    let ledger = paths.dangling_reference_ledger_path();

    let dry_run = run_cleanup(&paths, &CleanupRequest::report_only()).expect("dry run");
    assert_eq!(dry_run.deleted_count, 0);
    assert!(intent_path.exists(), "dry-run MUST NOT 删除记录");
    assert!(!ledger.exists(), "dry-run 零留痕");
    assert!(
        dry_run
            .zeroed_by_type
            .contains_key("automation_plan_intent")
    );
    assert!(!dry_run.governance_complete, "dry-run 不得宣称治理完成");
}

// ─── 自动模式：真实删除 + 留痕 + 自证归零 ────────────────────────

#[test]
fn dangling_cleanup_automatic_mode_deletes_whitelisted_intent() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    let business = write_user_business_data(temp.path());
    let intent_path = issue_root.join(PLAN_INTENT_FILE);

    // 清单摘要仍按原口径算出（仅供审计关联，不是删除凭据）。
    let token = current_token(&paths);
    assert!(token.starts_with("sha256:"));
    let outcome = run_cleanup(&paths, &automatic_request()).expect("apply");

    // 闸 1 通过 + 闸 2 白名单内：intent 记录被删。
    assert_eq!(outcome.deleted_count, 1, "'{outcome:?}'");
    assert!(!intent_path.exists(), "白名单内 intent 记录必须被删");
    // 闸 3 ③：清理后再巡检自证归零（本现场只剩已清记录）。
    assert!(outcome.zeroed_by_type.is_empty() && outcome.residual.is_empty());
    assert!(outcome.undetermined.is_empty(), "{outcome:?}");
    assert!(outcome.governance_complete, "全部归零时必须宣称治理完成");

    // 闸 3 ②：留痕落盘内容（一行 JSONL，三要素 + 证据等级齐备）。
    let lines = ledger_lines(&paths);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let row: LedgerRow = serde_json::from_str(&lines[0]).expect("ledger row");
    assert_eq!(row.schema_version, 1);
    assert_eq!(
        row.cleanup_id,
        cleanup_id(&row.record_path, &row.target_id, &row.action)
    );
    assert_eq!(row.action, "delete_record");
    assert_eq!(row.record_type, "automation_plan_intent");
    let expected_path = "projects/project_0001/issues/issue_0001/automation-plan-intent.json";
    assert_eq!(row.record_path, expected_path);
    assert_eq!(row.target_kind, "story_spec");
    assert_eq!(row.target_id, "story_spec_0005");
    assert_eq!(row.evidence_level, "L1");
    assert_eq!(row.trigger, CLEANUP_TRIGGER);
    assert!(row.run_id.starts_with("cleanup_"), "{}", row.run_id);
    let deleted_entity = row.deleted_entity.expect("deleted_entity");
    assert_eq!(deleted_entity.kind, "story");
    assert_eq!(deleted_entity.id, "story_spec_0005");
    assert_eq!(deleted_entity.project_id.as_deref(), Some("project_0001"));
    assert_eq!(deleted_entity.issue_id.as_deref(), Some("issue_0001"));

    // 删除范围：墓碑与用户业务数据（正文 / versions / artifact / work-item / attempt）全在。
    assert!(
        tombstone_path(temp.path(), "story", "story_spec_0005").exists(),
        "墓碑永不删"
    );
    assert!(tombstone_path(temp.path(), "plan", "issue_work_item_plan_0007").exists());
    for path in business {
        assert!(
            path.exists(),
            "用户业务数据 MUST NOT 被删：{}",
            path.display()
        );
    }
}

#[test]
fn dangling_cleanup_deletes_binding_entry_only_for_l1_plan() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write_binding(&paths, "s_l1", "plan", "issue_work_item_plan_0007");
    write_binding(
        &paths,
        "s_no_tombstone",
        "plan",
        "issue_work_item_plan_9999",
    );
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");

    let outcome = run_cleanup(&paths, &automatic_request()).expect("apply");

    assert_eq!(outcome.deleted_count, 1, "{outcome:?}");
    let dir = paths
        .state_root()
        .join(ENTITY_IDENTITY_DIR)
        .join(ENTITY_SESSIONS_DIR);
    assert!(
        !dir.join("s_l1.json").exists(),
        "有 L1 墓碑的台账条目必须被删"
    );
    assert!(
        dir.join("s_no_tombstone.json").exists(),
        "无 L1 墓碑者 MUST NOT 被删"
    );
    assert!(
        !outcome.governance_complete,
        "仍有只报不删残留 → 不得宣称治理完成"
    );
    let row: LedgerRow = serde_json::from_str(&ledger_lines(&paths)[0]).expect("ledger row");
    assert_eq!(row.record_type, "entity_identity_session_binding");
    assert_eq!(row.target_kind, "plan");
    assert_eq!(row.evidence_level, "L1");
    assert_eq!(row.action, "delete_record");
}

#[test]
fn dangling_cleanup_purges_window_cluster_by_attempt() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    for window_id in ["win_a", "win_b"] {
        let worktree = temp.path().join(format!("missing-{window_id}"));
        write_window(
            &paths,
            window_id,
            "coding_attempt_9f3",
            &worktree,
            "closed_accepted",
        );
    }
    let windows = paths.root().join("reviewer-windows/windows");
    let probe = probe_missing();
    let outcome =
        run_cleanup_with_attempt_probe(&paths, &automatic_request(), Some(&probe)).expect("apply");

    assert_eq!(
        outcome.deleted_count, 1,
        "窗口簇按 attempt 一条决策：{outcome:?}"
    );
    assert!(!windows.join("win_a.json").exists(), "簇内窗口记录必须被清");
    assert!(!windows.join("win_b.json").exists());
    let row: LedgerRow = serde_json::from_str(&ledger_lines(&paths)[0]).expect("ledger row");
    assert_eq!(row.record_type, "reviewer_window");
    assert_eq!(row.action, "delete_record_group");
    assert_eq!(row.target_kind, "attempt");
    assert_eq!(row.target_id, "coding_attempt_9f3");
    assert_eq!(row.evidence_level, "L2");
    assert!(
        row.deleted_entity.is_none(),
        "attempt 无墓碑种类 → 不猜 deleted_entity"
    );
}

// ─── 闸 2：白名单外 / 证据不足 一律只报不删 ───────────────────────

#[test]
fn dangling_cleanup_keeps_non_whitelisted_records() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    // 未结束的 enrollment：intent 仍有活授权 → Task 7 只报不删，销账不得动文件。
    write_enrollment(&issue_root, true, false);
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    let intent_path = issue_root.join(PLAN_INTENT_FILE);
    let enrollment_path = issue_root.join(ENROLLMENT_FILE);

    let outcome = run_cleanup(&paths, &automatic_request()).expect("apply");

    assert_eq!(
        outcome.deleted_count, 0,
        "证据不达标 MUST NOT 删：{outcome:?}"
    );
    assert!(
        intent_path.exists(),
        "intent MUST NOT 被删（enrollment 仍启用）"
    );
    assert!(enrollment_path.exists(), "enrollment 记录 MUST NOT 被删");
    let reported = outcome.report_only.iter().any(|decision| {
        matches!(
            decision,
            CleanupDecision::ReportOnly { record_path, .. }
                if record_path.ends_with(PLAN_INTENT_FILE)
        )
    });
    assert!(reported, "intent 必须进只报不删清单：{outcome:?}");
    assert!(ledger_lines(&paths).is_empty(), "零删除 → 零留痕");
    assert!(!outcome.governance_complete);
}

#[test]
fn dangling_cleanup_residual_is_not_counted_as_zero() {
    let temp = TempDir::new().unwrap();
    // intent 缺 L1 证据（无墓碑）：已判定悬空，但按规则只报不删。
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    let intent_path = issue_root.join(PLAN_INTENT_FILE);

    let outcome = run_cleanup(&paths, &automatic_request()).expect("apply");

    assert_eq!(outcome.deleted_count, 0, "{outcome:?}");
    assert!(intent_path.exists(), "证据不足 MUST NOT 删");
    let residual = outcome
        .residual
        .iter()
        .any(|f| f.record_path.ends_with(PLAN_INTENT_FILE));
    assert!(residual, "证据不足的悬空条目必须进 residual：{outcome:?}");
    assert!(
        !outcome
            .zeroed_by_type
            .contains_key("automation_plan_intent"),
        "只报不删残留 MUST NOT 计入归零计数：{:?}",
        outcome.zeroed_by_type
    );
    assert!(
        !outcome.governance_complete,
        "只有残留时 MUST NOT 宣称治理完成"
    );
    assert!(ledger_lines(&paths).is_empty(), "零删除 → 零留痕");
}

// ─── 闸 3：幂等 + 不可判定阻断自证 ────────────────────────────────

#[test]
fn dangling_cleanup_is_idempotent_on_reentry() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");

    let first = run_cleanup(&paths, &automatic_request()).expect("apply");
    assert_eq!(first.deleted_count, 1, "{first:?}");
    assert!(!issue_root.join(PLAN_INTENT_FILE).exists());
    let after_first = ledger_lines(&paths);
    assert_eq!(after_first.len(), 1, "{after_first:?}");

    // 重入：记录已不存在 → 无判定对象，必须零删除且不报错。
    let second = run_cleanup(&paths, &automatic_request()).expect("reentry");
    assert_eq!(second.deleted_count, 0, "重入 MUST NOT 重复删：{second:?}");
    let after_second = ledger_lines(&paths);
    assert_eq!(after_second, after_first, "重入 MUST NOT 追加重复行");
}

#[test]
fn dangling_cleanup_undetermined_blocks_governance_complete() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let windows = paths.root().join("reviewer-windows/windows");
    std::fs::create_dir_all(&windows).unwrap();
    std::fs::write(windows.join("win_bad.json"), b"{not-json").unwrap();

    let outcome = run_cleanup(&paths, &automatic_request()).expect("apply");

    assert_eq!(outcome.undetermined.len(), 1, "{outcome:?}");
    assert!(
        !outcome.governance_complete,
        "存在不可判定条目时 MUST NOT 宣称归零"
    );
    assert_eq!(outcome.deleted_count, 0, "{outcome:?}");
    assert!(ledger_lines(&paths).is_empty(), "零删除 → 零留痕");
    assert!(
        windows.join("win_bad.json").exists(),
        "不可判定记录 MUST NOT 被删"
    );
}

/// 护栏：非法记录路径（`..` / 绝对路径 / 结构不符 / 非白名单文件名）一律跳过。
#[test]
fn dangling_cleanup_skips_illegal_record_paths() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let victim = temp.path().join("victim.json");
    std::fs::write(&victim, b"{}").unwrap();
    let sessions = paths
        .state_root()
        .join(ENTITY_IDENTITY_DIR)
        .join(ENTITY_SESSIONS_DIR);
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(sessions.join("legal.json"), b"{}").unwrap();

    for record_path in [
        "../../../victim.json",
        "/etc/passwd",
        "state/entity-identity/sessions/../legal.json",
        "state/entity-identity/sessions/sub/legal.json",
        "projects/p1/issues/i1/other.json",
    ] {
        let resolved = resolve_record_file(&paths, record_path, binding_layout_ok, binding_name_ok)
            .expect("resolve");
        assert!(resolved.is_none(), "非法记录路径必须跳过：{record_path}");
    }
    let legal = resolve_record_file(
        &paths,
        "state/entity-identity/sessions/legal.json",
        binding_layout_ok,
        binding_name_ok,
    )
    .expect("resolve")
    .expect("合法路径必须解析成功");
    assert_eq!(
        legal,
        std::fs::canonicalize(&sessions).unwrap().join("legal.json")
    );
    assert!(victim.exists(), "根外文件 MUST NOT 被删");
}
