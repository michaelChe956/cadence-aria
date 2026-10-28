use super::*;
use crate::product::app_paths::ProductAppPaths;
use std::path::Path;
use tempfile::TempDir;

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body.as_bytes()).unwrap();
}

fn tree_digest(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((
                    path.strip_prefix(root)
                        .unwrap()
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

#[test]
fn dangling_patrol_covers_five_partitions_and_registers_all_reference_fields() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let issue_root = paths.issue_root("project_0001", "issue_0001");
    write(
        &issue_root.join("automation-enrollment.json"),
        r#"{"enrollment_id":"e1","selection_key":"s1","project_id":"project_0001","issue_id":"issue_0001","enabled":false,"policy_revision":1,"source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"prepare_intent_id":"p1","plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x","created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}"#,
    );
    write(
        &paths.root().join("reviewer-windows/windows/win_001.json"),
        r#"{"schema":"reviewer_window_v1","window_id":"win_001","generation":1,"identity":{"canonical_worktree":"/tmp/none","gitdir":"/tmp/none/.git","common_dir":"/tmp/none","lease_id":"lease_1"},"attempt_id":"coding_attempt_9f3","run_id":"run_1","provider":"p","state":"closed_accepted","report_acceptance":"accepted","platform_write_paths":[],"quiesced_scopes":[],"baseline":null,"restore":null,"recovery":null,"last_error":null,"violation_pending":null,"pre_restore_git_facts":[],"transitions":[],"created_at":"2026-10-24T00:00:00Z","updated_at":"2026-10-24T00:00:00Z"}"#,
    );
    write(
        &paths
            .root()
            .join("state/entity-identity/sessions/workspace_session_auto_x.json"),
        r#"{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"plan","entity_id":"issue_work_item_plan_0007"}"#,
    );

    let report = patrol_state_root(&paths).expect("patrol");
    assert!(
        report
            .covered_record_types
            .iter()
            .any(|kind| kind == "automation_enrollment")
    );
    assert!(
        report
            .covered_record_types
            .iter()
            .any(|kind| kind == "reviewer_window")
    );
    assert!(
        report
            .covered_record_types
            .iter()
            .any(|kind| kind == "entity_identity_session_binding")
    );
    let fields: Vec<&str> = report
        .findings
        .iter()
        .map(|finding| finding.reference_field.as_str())
        .collect();
    assert!(fields.contains(&"source.stories[].id"), "{fields:?}");
    assert!(fields.contains(&"plan_id"), "{fields:?}");
    assert!(fields.contains(&"attempt_id"), "{fields:?}");
    assert!(fields.contains(&"entity_kind+entity_id"), "{fields:?}");
}

#[test]
fn dangling_patrol_isolates_undetermined_entries() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write(
        &paths.root().join("reviewer-windows/windows/win_bad.json"),
        "{ not json",
    );

    let report = patrol_state_root(&paths).expect("patrol");
    assert_eq!(report.undetermined.len(), 1, "坏记录必须进不可判定");
    assert_eq!(report.findings.len(), 0, "坏记录不得计入悬空数");
    assert!(
        report.undetermined[0].reason.contains("json"),
        "{}",
        report.undetermined[0].reason
    );
}

#[test]
fn dangling_patrol_makes_no_writes() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write(
        &paths
            .issue_root("project_0001", "issue_0001")
            .join("automation-plan-intent.json"),
        r#"{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x"}"#,
    );
    let before = tree_digest(paths.root());
    let _ = patrol_state_root(&paths).expect("patrol");
    let after = tree_digest(paths.root());
    assert_eq!(before, after, "巡检不得改写状态根");
}

// -- 夹具 ------------------------------------------------------------

/// 路一探针：只有 `alive` 名单内的 attempt 命中（其余一律「已删」）。
fn probe_alive(alive: &'static [&'static str]) -> impl Fn(&str) -> Result<bool, ProductStoreError> {
    move |id: &str| Ok(alive.contains(&id))
}

/// 路一探针：一律「已删」。
fn probe_false() -> impl Fn(&str) -> Result<bool, ProductStoreError> {
    |_id: &str| Ok(false)
}

/// 计划存活判据（schema-v2 线路）：`work-item-revisions/<plan_id>/lineage.json`。
fn write_live_plan(paths: &ProductAppPaths, plan_id: &str) {
    write(
        &paths
            .issue_root("project_0001", "issue_0001")
            .join("work-item-revisions")
            .join(plan_id)
            .join("lineage.json"),
        "{}",
    );
}

/// 计划意图记录（`Current` 形态：带 `target`，全引用由入参决定）。
fn plan_intent_body(plan_id: &str, session_id: &str) -> String {
    format!(
        r#"{{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{{"stories":[],"designs":[]}},"options":{{"author_provider":"codex","reviewer_provider":"codex","plan_options":{{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}}}},"target":{{"kind":"single_repository","repository_id":"repo_1"}},"plan_id":"{plan_id}","session_id":"{session_id}"}}"#
    )
}

/// 自动化 enrollment 记录（无引用：`source` 空、无 plan/session 绑定）。
fn enrollment_body() -> &'static str {
    r#"{"enrollment_id":"e1","selection_key":"s1","project_id":"project_0001","issue_id":"issue_0001","enabled":false,"policy_revision":1,"source":{"stories":[],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"prepare_intent_id":"p1","created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}"#
}

/// reviewer 窗口记录。
fn window_body(window_id: &str, attempt_id: &str, state: &str, worktree: &Path) -> String {
    format!(
        r#"{{"schema":"reviewer_window_v1","window_id":"{window_id}","generation":1,"identity":{{"canonical_worktree":"{}","gitdir":"{}/.git","common_dir":"{}/.git","lease_id":"lease_1"}},"attempt_id":"{attempt_id}","run_id":"run_1","provider":"codex","state":"{state}","report_acceptance":"accepted","platform_write_paths":[],"quiesced_scopes":[],"baseline":null,"restore":null,"recovery":null,"last_error":null,"violation_pending":null,"pre_restore_git_facts":[],"transitions":[],"created_at":"2026-10-24T00:00:00Z","updated_at":"2026-10-24T00:00:00Z"}}"#,
        worktree.display(),
        worktree.display(),
        worktree.display()
    )
}

/// reviewer 快照 manifest（巡检只投影 `attempt_id`）。
fn snapshot_manifest_body(attempt_id: &str) -> String {
    format!(r#"{{"attempt_id":"{attempt_id}"}}"#)
}

/// 实体身份台账条目。
fn binding_body(entity_kind: &str, entity_id: &str, project_id: &str, issue_id: &str) -> String {
    format!(
        r#"{{"project_id":"{project_id}","issue_id":"{issue_id}","entity_kind":"{entity_kind}","entity_id":"{entity_id}"}}"#
    )
}

/// L1 墓碑（`phase=deleted` + `deleted_at` 有值 + 无 pending-failure）。
fn write_l1_tombstone(paths: &ProductAppPaths) {
    write(
        &paths
            .issue_root("project_0001", "issue_0001")
            .join("entity-deletions/story/story_spec_0005.json"),
        r#"{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"story","entity_id":"story_spec_0005","phase":"deleted","started_at":"2026-10-28T10:00:00Z","deleted_at":"2026-10-28T10:05:00Z"}"#,
    );
}

const MEMBER_ID: &str = "11111111-1111-4111-8111-111111111111";
const CHECKOUT_ID: &str = "22222222-2222-4222-8222-222222222222";

/// 逻辑代码库 scope 根（`projects/project_0001/logical-codebases/lc_0001`）。
fn lc_root(paths: &ProductAppPaths) -> PathBuf {
    paths.logical_codebases_root("project_0001").join("lc_0001")
}

/// LC 成员记录（引用 `checkout_ids`）。
fn member_body(checkout_ids: &[&str]) -> String {
    let checkouts: Vec<String> = checkout_ids.iter().map(|id| format!("\"{id}\"")).collect();
    format!(
        r#"{{"logical_repository_id":"{MEMBER_ID}","physical_repository_id":"repo_1","alias":"r1","role":"backend","ordinal":0,"source_identity":{{"scheme":"git_dir_only_v1","key_digest":"k","canonical_git_dir":"/tmp/git","first_seen_path_hash":"h"}},"checkout_ids":[{}],"created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}}"#,
        checkouts.join(",")
    )
}

/// LC checkout 记录（`logical_repository_id` 指向成员）。
fn checkout_body(checkout_id: &str) -> String {
    format!(
        r#"{{"checkout_id":"{checkout_id}","logical_repository_id":"{MEMBER_ID}","physical_repository_id":"repo_1","kind":"main","canonical_path":"/tmp/co","checkout_path_hash":"h","git_dir_identity":"g","observed_at":"2026-10-28T10:00:00Z","created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}}"#
    )
}

/// LC manifest（`member_ids[]` + `active_aggregate_index_id`）。
fn manifest_body(member_ids: &[&str], active: Option<&str>) -> String {
    let members: Vec<String> = member_ids.iter().map(|id| format!("\"{id}\"")).collect();
    let active = active
        .map(|id| format!(r#","active_aggregate_index_id":"{id}""#))
        .unwrap_or_default();
    format!(
        r#"{{"schema_version":1,"project_id":"project_0001","logical_codebase_id":"33333333-3333-4333-8333-333333333333","provider_context_root":"/tmp/lc","layout":"common_non_git_parent","membership_revision":1,"member_ids":[{}]{active},"created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}}"#,
        members.join(",")
    )
}

/// LC 聚合索引记录（每个成员快照引用一次成员 + 一次 checkout）。
fn aggregate_index_body(aggregate_index_id: &str, checkout_id: &str) -> String {
    format!(
        r#"{{"aggregate_index_id":"{aggregate_index_id}","project_id":"project_0001","membership_revision":1,"status":"active","member_snapshots":[{{"logical_repository_id":"{MEMBER_ID}","checkout_id":"{checkout_id}","revision":"abc","dirty":false,"included":true,"indexed_at":"2026-10-28T10:00:00Z"}}],"codegraph_version":"1.0","codegraph_root":"/tmp/cg","created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}}"#
    )
}

/// 断言某目标 id 未被判悬空。
fn assert_not_flagged(report: &PatrolReport, target_id: &str) {
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.target_id == target_id),
        "存活目标被判悬空：{target_id}；{:?}",
        report.findings
    );
}

// -- 分区覆盖：正常条目 ----------------------------------------------

#[test]
fn dangling_patrol_keeps_live_references_out_of_findings() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let issue_root = paths.issue_root("project_0001", "issue_0001");
    // 分区一 / 分区四：存活 plan（schema-v2 线路），intent 与台账条目都指向它。
    write_live_plan(&paths, "issue_work_item_plan_0007");
    write(
        &issue_root.join("automation-enrollment.json"),
        enrollment_body(),
    );
    write(
        &issue_root.join("automation-plan-intent.json"),
        &plan_intent_body("issue_work_item_plan_0007", "workspace_session_auto_x"),
    );
    write(
        &paths
            .root()
            .join("state/entity-identity/sessions/workspace_session_auto_x.json"),
        &binding_body(
            "plan",
            "issue_work_item_plan_0007",
            "project_0001",
            "issue_0001",
        ),
    );
    // 分区二 / 分区三：attempt 路一探针命中（两路任一命中即「仍在」）。
    let absent_worktree = temp.path().join("absent-worktree");
    write(
        &paths.root().join("reviewer-windows/windows/win_live.json"),
        &window_body(
            "win_live",
            "coding_attempt_live",
            "closed_accepted",
            &absent_worktree,
        ),
    );
    write(
        &paths
            .root()
            .join("reviewer-snapshots/windows/win_live/manifest.json"),
        &snapshot_manifest_body("coding_attempt_live"),
    );
    // 分区五：成员 / checkout / 聚合索引三者互相命中。
    write(
        &lc_root(&paths).join("manifest.json"),
        &manifest_body(&[MEMBER_ID], Some("agg_0001")),
    );
    write(
        &lc_root(&paths).join(format!("members/{MEMBER_ID}.json")),
        &member_body(&[CHECKOUT_ID]),
    );
    write(
        &lc_root(&paths).join(format!("checkouts/{CHECKOUT_ID}.json")),
        &checkout_body(CHECKOUT_ID),
    );
    write(
        &lc_root(&paths).join("aggregate-indexes/agg_0001.json"),
        &aggregate_index_body("agg_0001", CHECKOUT_ID),
    );

    let probe = probe_alive(&["coding_attempt_live"]);
    let report = patrol_state_root_with_probe(&paths, Some(&probe)).expect("patrol");
    // 存活的 plan / attempt / 成员 / checkout / 聚合索引一律不产发现。
    for target in [
        "issue_work_item_plan_0007",
        "coding_attempt_live",
        MEMBER_ID,
        CHECKOUT_ID,
        "agg_0001",
    ] {
        assert_not_flagged(&report, target);
    }
    assert!(
        report.undetermined.is_empty(),
        "存活目标不得进不可判定：{:?}",
        report.undetermined
    );
    // 唯一残留发现 = 未落盘 session 记录的 `session_id`（正常中间态，只报不删）。
    assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
    assert_eq!(report.findings[0].reference_field, "session_id");
    assert_eq!(
        report.findings[0].proposed_action,
        PATROL_ACTION_REPORT_ONLY
    );
}

// -- 判据表字段登记完整性（REQ-DRG-09） ------------------------------

#[test]
fn dangling_patrol_registers_every_enrollment_reference_field() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write(
        &paths
            .issue_root("project_0001", "issue_0001")
            .join("automation-enrollment.json"),
        r#"{"enrollment_id":"e1","selection_key":"s1","project_id":"project_0001","issue_id":"issue_0001","enabled":false,"policy_revision":1,"source":{"stories":[{"id":"story_spec_0001","version":1}],"designs":[{"id":"design_spec_0002","version":1}]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"prepare_intent_id":"p1","plan_id":"issue_work_item_plan_0003","session_id":"workspace_session_auto_0004","binding_history":{"current":{"binding_version":2,"enrollment_id":"e1","plan_id":"issue_work_item_plan_0005","session_id":"workspace_session_auto_0006","source":{"stories":[{"id":"story_spec_0007","version":1}],"designs":[{"id":"design_spec_0008","version":1}]},"target":{"kind":"single_repository","repository_id":"repo_1"},"author_provider":"codex","reviewer_provider":"codex"},"previous":[{"binding_version":1,"enrollment_id":"e1","plan_id":"issue_work_item_plan_0011","session_id":"workspace_session_auto_0012","source":{"stories":[{"id":"story_spec_0009","version":1}],"designs":[{"id":"design_spec_0010","version":1}]},"target":{"kind":"single_repository","repository_id":"repo_1"},"author_provider":"codex","reviewer_provider":"codex"}]},"created_at":"2026-10-28T10:00:00Z","updated_at":"2026-10-28T10:00:00Z"}"#,
    );

    let report = patrol_state_root(&paths).expect("patrol");
    // 判据表登记的十个 enrollment 引用字段逐个必须出现在发现里（漏检防线）。
    let expected = [
        "source.stories[].id",
        "source.designs[].id",
        "plan_id",
        "session_id",
        "binding_history.current.plan_id",
        "binding_history.current.session_id",
        "binding_history.current.source.stories[].id",
        "binding_history.current.source.designs[].id",
        "binding_history.previous[].source.stories[].id",
        "binding_history.previous[].source.designs[].id",
    ];
    let fields: BTreeSet<&str> = report
        .findings
        .iter()
        .map(|finding| finding.reference_field.as_str())
        .collect();
    for field in expected {
        assert!(
            fields.contains(field),
            "判据表字段未登记：{field}；实际 {fields:?}"
        );
    }
    assert_eq!(
        report.findings.len(),
        expected.len(),
        "{:?}",
        report.findings
    );
    assert!(report.undetermined.is_empty(), "{:?}", report.undetermined);
    // 七列齐全 + enrollment 引用一律只报不删（§3.4.1 降级档位）。
    for finding in &report.findings {
        assert_eq!(finding.record_type, "automation_enrollment");
        assert!(!finding.record_path.is_empty());
        assert!(!finding.reference_field.is_empty());
        assert!(!finding.target_kind.is_empty());
        assert!(!finding.target_id.is_empty());
        assert_eq!(finding.evidence_level, EVIDENCE_LABEL_INSUFFICIENT);
        assert_eq!(finding.proposed_action, PATROL_ACTION_REPORT_ONLY);
    }
}

// -- 分区覆盖：悬空 → 发现（含 LC 三类真实读） -----------------------

#[test]
fn dangling_patrol_lc_partition_flags_dangling_member_and_checkout() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let missing_member = "44444444-4444-4444-8444-444444444444";
    let missing_checkout = "55555555-5555-4555-8555-555555555555";
    write(
        &lc_root(&paths).join("manifest.json"),
        &manifest_body(&[missing_member], Some("agg_missing")),
    );
    write(
        &lc_root(&paths).join(format!("members/{MEMBER_ID}.json")),
        &member_body(&[missing_checkout]),
    );

    let report = patrol_state_root(&paths).expect("patrol");
    // 三条：manifest.member_ids[]、manifest.active_aggregate_index_id、member.checkout_ids[]。
    assert_eq!(report.findings.len(), 3, "{:?}", report.findings);
    let fields: BTreeSet<&str> = report
        .findings
        .iter()
        .map(|finding| finding.reference_field.as_str())
        .collect();
    assert!(fields.contains("member_ids[]"), "{fields:?}");
    assert!(fields.contains("active_aggregate_index_id"), "{fields:?}");
    assert!(fields.contains("checkout_ids[]"), "{fields:?}");
    for finding in &report.findings {
        assert_eq!(finding.record_type, "logical_codebase_member");
        assert_eq!(
            finding.proposed_action, PATROL_ACTION_REPORT_ONLY,
            "LC 三类恒只报不删"
        );
    }
    assert!(report.undetermined.is_empty(), "{:?}", report.undetermined);
}

// -- 分区覆盖：U1 类 / 数据源不可用 → 不可判定 -----------------------

#[test]
fn dangling_patrol_marks_unavailable_sources_undetermined() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    // 分区五：成员指向 checkout，但 checkouts 数据源不可读（写成普通文件）。
    write(
        &lc_root(&paths).join(format!("members/{MEMBER_ID}.json")),
        &member_body(&[CHECKOUT_ID]),
    );
    write(&lc_root(&paths).join("checkouts"), "not a directory");
    // 分区四：台账条目缺所属 project/issue 作用域 → 无法核对。
    write(
        &paths
            .root()
            .join("state/entity-identity/sessions/workspace_session_auto_q.json"),
        &binding_body("plan", "issue_work_item_plan_0007", "", ""),
    );

    let report = patrol_state_root(&paths).expect("patrol");
    assert!(
        report.findings.is_empty(),
        "不可判定不得计入悬空：{:?}",
        report.findings
    );
    assert_eq!(report.undetermined.len(), 2, "{:?}", report.undetermined);
    assert!(
        report
            .undetermined
            .iter()
            .any(|entry| entry.reference_field == "checkout_ids[]"),
        "{:?}",
        report.undetermined
    );
    assert!(
        report
            .undetermined
            .iter()
            .any(|entry| entry.reference_field == "entity_kind+entity_id"
                && entry.reason.contains("作用域")),
        "{:?}",
        report.undetermined
    );
}

// -- 分区覆盖：未归类 → 只报不删 -------------------------------------

#[test]
fn dangling_patrol_reports_unclassified_records_separately() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    // issue 分区表外记录。
    write(
        &paths
            .issue_root("project_0001", "issue_0001")
            .join("automation-legacy-ledger.json"),
        "{}",
    );
    // 实体身份台账分区表外记录。
    write(
        &paths.state_root().join("entity-identity/ledger.json"),
        "{}",
    );
    // 快照窗口目录里的表外记录（`restore-cursor.json`）+ 一份悬空 manifest。
    write(
        &paths
            .root()
            .join("reviewer-snapshots/windows/win_1/restore-cursor.json"),
        "{}",
    );
    write(
        &paths
            .root()
            .join("reviewer-snapshots/windows/win_1/manifest.json"),
        &snapshot_manifest_body("coding_attempt_absent"),
    );

    let report = patrol_state_root(&paths).expect("patrol");
    assert_eq!(report.unclassified.len(), 3, "{:?}", report.unclassified);
    for entry in &report.unclassified {
        assert_eq!(entry.record_type, "unclassified");
    }
    assert!(
        report
            .unclassified
            .iter()
            .any(|entry| entry.record_path.ends_with("automation-legacy-ledger.json"))
    );
    assert!(
        report
            .unclassified
            .iter()
            .any(|entry| entry.record_path.ends_with("entity-identity/ledger.json"))
    );
    assert!(
        report
            .unclassified
            .iter()
            .any(|entry| entry.record_path.ends_with("restore-cursor.json"))
    );
    assert!(
        report
            .covered_record_types
            .iter()
            .any(|kind| kind == "unclassified")
    );
    // 未归类不进悬空明细：唯一发现来自快照 manifest 的悬空 attempt。
    assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
    assert_eq!(report.findings[0].target_id, "coding_attempt_absent");
    assert!(report.findings[0].record_path.ends_with("manifest.json"));
}

// -- 分区覆盖：坏记录 → 不可判定（REQ-DRG-23） ----------------------

#[test]
fn dangling_patrol_isolates_bad_records_per_partition() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    // 分区三：快照 manifest 解析失败。
    write(
        &paths
            .root()
            .join("reviewer-snapshots/windows/win_bad/manifest.json"),
        "{ not json",
    );
    // 分区四：台账条目解析失败。
    write(
        &paths
            .root()
            .join("state/entity-identity/sessions/workspace_session_auto_bad.json"),
        "{ not json",
    );

    let report = patrol_state_root(&paths).expect("patrol");
    assert_eq!(report.undetermined.len(), 2, "{:?}", report.undetermined);
    assert!(
        report.findings.is_empty(),
        "坏记录不得计入悬空：{:?}",
        report.findings
    );
    for entry in &report.undetermined {
        assert!(entry.reason.contains("json"), "{}", entry.reason);
    }
    assert!(
        report
            .undetermined
            .iter()
            .any(|entry| entry.record_path.ends_with("manifest.json")),
        "{:?}",
        report.undetermined
    );
    assert!(
        report.undetermined.iter().any(|entry| entry
            .record_path
            .ends_with("workspace_session_auto_bad.json")),
        "{:?}",
        report.undetermined
    );
}

// -- 墓碑不判悬空（REQ-DRG-09） --------------------------------------

#[test]
fn dangling_patrol_never_flags_deletion_tombstones() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write_l1_tombstone(&paths);

    let report = patrol_state_root(&paths).expect("patrol");
    assert!(report.findings.is_empty(), "{:?}", report.findings);
    assert!(report.unclassified.is_empty(), "{:?}", report.unclassified);
    assert!(report.undetermined.is_empty(), "{:?}", report.undetermined);
}

// -- 证据等级 → 建议动作 ---------------------------------------------

#[test]
fn dangling_patrol_actions_follow_evidence_levels() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let issue_root = paths.issue_root("project_0001", "issue_0001");
    // L1：story 有完整墓碑 → 建议删除记录。
    write_l1_tombstone(&paths);
    write(
        &issue_root.join("automation-plan-intent.json"),
        r#"{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x"}"#,
    );
    // L2：窗口终态 + 无租约 + 两路目标皆无 + worktree 已回收 → 建议按窗口簇删除。
    let absent_worktree = temp.path().join("absent-worktree");
    write(
        &paths.root().join("reviewer-windows/windows/win_l2.json"),
        &window_body(
            "win_l2",
            "coding_attempt_gone",
            "closed_accepted",
            &absent_worktree,
        ),
    );

    let report = patrol_state_root(&paths).expect("patrol");
    let story = report
        .findings
        .iter()
        .find(|finding| finding.target_id == "story_spec_0005")
        .expect("story 悬空发现");
    assert_eq!(story.evidence_level, EVIDENCE_LABEL_L1);
    assert_eq!(story.proposed_action, PATROL_ACTION_DELETE);
    let plan = report
        .findings
        .iter()
        .find(|finding| finding.reference_field == "plan_id")
        .expect("plan 悬空发现");
    assert_eq!(
        plan.evidence_level, EVIDENCE_LABEL_INSUFFICIENT,
        "无墓碑不得判 L1"
    );
    assert_eq!(plan.proposed_action, PATROL_ACTION_REPORT_ONLY);
    let session = report
        .findings
        .iter()
        .find(|finding| finding.reference_field == "session_id")
        .expect("session 悬空发现");
    assert_eq!(
        session.proposed_action, PATROL_ACTION_REPORT_ONLY,
        "session 恒只报不删"
    );
    let attempt = report
        .findings
        .iter()
        .find(|finding| finding.reference_field == "attempt_id")
        .expect("attempt 悬空发现");
    assert!(
        attempt.evidence_level.starts_with(EVIDENCE_LABEL_L2),
        "{}",
        attempt.evidence_level
    );
    assert_eq!(attempt.proposed_action, PATROL_ACTION_DELETE_GROUP);
    // 墓碑自身绝不进悬空明细 / 未归类。
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.record_path.contains("entity-deletions"))
    );
    assert!(report.unclassified.is_empty(), "{:?}", report.unclassified);
}

#[test]
fn dangling_patrol_window_cluster_requires_all_windows() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let absent_worktree = temp.path().join("absent-worktree");
    let windows_dir = paths.root().join("reviewer-windows/windows");
    // 同一 attempt 两个窗口：一个符合四条件，一个仍非终态 → 整簇不删（r3 N-9）。
    write(
        &windows_dir.join("win_a.json"),
        &window_body(
            "win_a",
            "coding_attempt_gone",
            "closed_accepted",
            &absent_worktree,
        ),
    );
    write(
        &windows_dir.join("win_b.json"),
        &window_body("win_b", "coding_attempt_gone", "running", &absent_worktree),
    );

    let report = patrol_state_root(&paths).expect("patrol");
    assert_eq!(report.findings.len(), 2, "{:?}", report.findings);
    for finding in &report.findings {
        assert_eq!(finding.reference_field, "attempt_id");
        assert_eq!(
            finding.proposed_action, PATROL_ACTION_REPORT_ONLY,
            "同簇任一不满足即整簇不删"
        );
        assert!(
            finding
                .evidence_level
                .starts_with(EVIDENCE_LABEL_INSUFFICIENT),
            "{}",
            finding.evidence_level
        );
        // 摘要必须反映不成立的那一扇窗口（非终态），不得用合规窗口掩盖。
        assert!(
            finding.evidence_level.contains("W-1 终态：不成立"),
            "{}",
            finding.evidence_level
        );
    }
}

// -- 路二未能枚举 ≠ 目标不存在（REQ-DRG-21 / 23） --------------------

#[test]
fn dangling_patrol_isolates_unknown_when_session_enumeration_fails() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    // workspace-sessions 写成普通文件 → 路二枚举失败。
    write(
        &paths
            .issue_root("project_0001", "issue_0001")
            .join("workspace-sessions"),
        "not a directory",
    );
    let absent_worktree = temp.path().join("absent-worktree");
    write(
        &paths.root().join("reviewer-windows/windows/win_gone.json"),
        &window_body(
            "win_gone",
            "coding_attempt_gone",
            "closed_accepted",
            &absent_worktree,
        ),
    );

    let probe = probe_false();
    let report = patrol_state_root_with_probe(&paths, Some(&probe)).expect("patrol");
    assert!(
        report.findings.is_empty(),
        "未能枚举不得当作目标不存在：{:?}",
        report.findings
    );
    assert_eq!(report.undetermined.len(), 1, "{:?}", report.undetermined);
    assert_eq!(report.undetermined[0].reference_field, "attempt_id");
    assert!(
        report.undetermined[0].reason.contains("枚举"),
        "{}",
        report.undetermined[0].reason
    );
}

// -- 统计口径：三栏互不混算 ------------------------------------------

#[test]
fn dangling_patrol_keeps_counts_separate() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let absent_worktree = temp.path().join("absent-worktree");
    // 一栏一条：悬空（窗口 attempt 已删）/ 不可判定（台账缺作用域）/ 未归类。
    write(
        &paths.root().join("reviewer-windows/windows/win_gone.json"),
        &window_body(
            "win_gone",
            "coding_attempt_gone",
            "closed_accepted",
            &absent_worktree,
        ),
    );
    write(
        &paths
            .root()
            .join("state/entity-identity/sessions/workspace_session_auto_q.json"),
        &binding_body("plan", "issue_work_item_plan_0007", "", ""),
    );
    write(
        &paths
            .issue_root("project_0001", "issue_0001")
            .join("automation-legacy-ledger.json"),
        "{}",
    );

    let report = patrol_state_root(&paths).expect("patrol");
    assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
    assert_eq!(report.undetermined.len(), 1, "{:?}", report.undetermined);
    assert_eq!(report.unclassified.len(), 1, "{:?}", report.unclassified);
    assert_eq!(report.findings[0].target_id, "coding_attempt_gone");
    assert_eq!(
        report.undetermined[0].record_path,
        "state/entity-identity/sessions/workspace_session_auto_q.json"
    );
    assert_eq!(
        report.unclassified[0].record_path,
        "projects/project_0001/issues/issue_0001/automation-legacy-ledger.json"
    );
}
