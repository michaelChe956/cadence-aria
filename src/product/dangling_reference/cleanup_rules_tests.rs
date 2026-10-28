//! 销账规则判定面的内联测试（按 `patrol_tests.rs` 同款方式独立成文件：
//! 生产段保持在行数门禁 1200 行以内，测试原样独立可读）。
//!
//! 覆盖 REQ-DRG-12 / 19 / 20：intent 四道前置条件（I-1..I-4）、窗口簇按 attempt
//! 聚合的 W-1..W-4、台账条目的 L1 墓碑判定。

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::dangling_reference::cleanup::{CleanupRequest, run_cleanup};
use crate::product::dangling_reference::listing::EntityIdentityBindingEntry;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::EnrollmentTarget;
use crate::product::models::automation::{
    EnrollmentOptions, EnrollmentSource, EnrollmentTerminationRequest, EnrollmentWriteCommand,
    PlanApprovalPolicy, SourceRevisionRef,
};
use crate::product::models::lifecycle::IssueWorkItemPlanOptions;
use crate::product::models::provider::ProviderName;
use crate::product::reviewer_window_store::tree_key_of;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

fn probe_false() -> impl Fn(&str) -> Result<bool, ProductStoreError> {
    |_id: &str| Ok(false)
}

fn plan_intent_body() -> &'static str {
    r#"{"enrollment_id":"e1","prepare_intent_id":"p1","project_id":"project_0001","issue_id":"issue_0001","source":{"stories":[{"id":"story_spec_0005","version":1}],"designs":[]},"options":{"author_provider":"codex","reviewer_provider":"codex","plan_options":{"include_integration_tests":false,"include_e2e_tests":false,"force_frontend_backend_split":false,"require_execution_plan_confirm":false}},"target":{"kind":"single_repository","repository_id":"repo_1"},"plan_id":"issue_work_item_plan_0007","session_id":"workspace_session_auto_x"}"#
}

fn generation_intent_body(phase: &str) -> String {
    serde_json::json!({
        "enrollment_id": "e1",
        "plan_id": "issue_work_item_plan_0007",
        "session_id": "workspace_session_auto_x",
        "action_key": "e1:issue_work_item_plan_0007:start_generation",
        "source": {"stories": [{"id": "story_spec_0005", "version": 1}], "designs": []},
        "options": {
            "author_provider": "codex",
            "reviewer_provider": "codex",
            "plan_options": {
                "include_integration_tests": false,
                "include_e2e_tests": false,
                "force_frontend_backend_split": false,
                "require_execution_plan_confirm": false,
            },
        },
        "target": {"kind": "single_repository", "repository_id": "repo_1"},
        "phase": phase,
    })
    .to_string()
}

fn write_enrollment(issue_root: &Path, enrollment_id: &str, enabled: bool, terminated: bool) {
    let mut body = serde_json::json!({
        "enrollment_id": enrollment_id,
        "selection_key": "selection_1",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "enabled": enabled,
        "policy_revision": 1,
        "source": {"stories": [{"id": "story_spec_0005", "version": 1}], "designs": []},
        "options": {
            "author_provider": "codex",
            "reviewer_provider": "codex",
            "plan_options": {
                "include_integration_tests": false,
                "include_e2e_tests": false,
                "force_frontend_backend_split": false,
                "require_execution_plan_confirm": false,
            },
        },
        "prepare_intent_id": "p1",
        "plan_id": "issue_work_item_plan_0007",
        "session_id": "workspace_session_auto_x",
        "created_at": "2026-10-28T09:00:00Z",
        "updated_at": "2026-10-28T09:00:00Z",
    });
    if terminated {
        body["terminated_at"] = serde_json::Value::String("2026-10-28T09:30:00Z".to_string());
    }
    std::fs::write(
        issue_root.join("automation-enrollment.json"),
        serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();
}

fn write_tombstone(root: &Path, kind: &str, id: &str) {
    let dir = root
        .join("projects/project_0001/issues/issue_0001/entity-deletions")
        .join(kind);
    std::fs::create_dir_all(&dir).unwrap();
    let record = serde_json::json!({
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "entity_kind": kind,
        "entity_id": id,
        "phase": "deleted",
        "started_at": "2026-10-28T10:00:00Z",
        "deleted_at": "2026-10-28T10:05:00Z",
    });
    std::fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

fn tombstone_path(root: &Path, kind: &str, id: &str) -> PathBuf {
    root.join("projects/project_0001/issues/issue_0001/entity-deletions")
        .join(kind)
        .join(format!("{id}.json"))
}

fn write_pending_failure(root: &Path, kind: &str, id: &str) {
    let dir = root
        .join("projects/project_0001/issues/issue_0001/entity-operations/pending-failures")
        .join(kind);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{id}.json")), b"{}").unwrap();
}

/// 建好 issue 目录并写入 plan intent，返回 (paths, issue_root)。
fn plan_intent_issue(root: &Path, body: &str) -> (ProductAppPaths, PathBuf) {
    let paths = ProductAppPaths::new(root);
    let issue_root = paths.issue_root("project_0001", "issue_0001");
    std::fs::create_dir_all(&issue_root).unwrap();
    std::fs::write(issue_root.join("automation-plan-intent.json"), body).unwrap();
    (paths, issue_root)
}

/// 建好 issue 目录并写入 generation intent，返回 (paths, issue_root)。
fn generation_intent_issue(root: &Path, phase: &str) -> (ProductAppPaths, PathBuf) {
    let paths = ProductAppPaths::new(root);
    let issue_root = paths.issue_root("project_0001", "issue_0001");
    std::fs::create_dir_all(&issue_root).unwrap();
    std::fs::write(
        issue_root.join("automation-generation-intent.json"),
        generation_intent_body(phase),
    )
    .unwrap();
    (paths, issue_root)
}

fn write_window(
    paths: &ProductAppPaths,
    window_id: &str,
    attempt_id: &str,
    worktree: &Path,
    state: &str,
) {
    let windows = paths.root().join("reviewer-windows/windows");
    std::fs::create_dir_all(&windows).unwrap();
    std::fs::write(
        windows.join(format!("{window_id}.json")),
        serde_json::to_vec(&serde_json::json!({
            "schema": "reviewer_window_v1",
            "window_id": window_id,
            "generation": 1,
            "identity": {
                "canonical_worktree": worktree,
                "gitdir": worktree.join(".git"),
                "common_dir": worktree,
                "lease_id": "lease_1",
            },
            "attempt_id": attempt_id,
            "run_id": "run_1",
            "provider": "codex",
            "state": state,
            "report_acceptance": "accepted",
            "platform_write_paths": [],
            "quiesced_scopes": [],
            "baseline": null,
            "restore": null,
            "recovery": null,
            "last_error": null,
            "violation_pending": null,
            "pre_restore_git_facts": [],
            "transitions": [],
            "created_at": "2026-10-24T00:00:00Z",
            "updated_at": "2026-10-24T00:00:00Z",
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_lease(paths: &ProductAppPaths, worktree: &Path) {
    let leases = paths.root().join("reviewer-windows/leases");
    std::fs::create_dir_all(&leases).unwrap();
    std::fs::write(
        leases.join(format!("{}.json", tree_key_of(worktree))),
        b"{}",
    )
    .unwrap();
}

/// 判定必须只报不删，且 `reason` 指明未满足的那一条前置条件。
fn assert_report_only(decision: &CleanupDecision, keyword: &str) {
    match decision {
        CleanupDecision::ReportOnly { reason, .. } => {
            assert!(
                reason.contains(keyword),
                "reason 未指明「{keyword}」：{reason}"
            );
        }
        other => panic!("必须只报不删：{other:?}"),
    }
}

#[test]
fn dangling_intent_without_l1_evidence_is_report_only() {
    let temp = TempDir::new().unwrap();
    let (paths, _issue_root) = plan_intent_issue(temp.path(), plan_intent_body());

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decision = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-plan-intent.json",
        &resolver,
    )
    .expect("decide");
    assert!(
        matches!(decision, CleanupDecision::ReportOnly { .. }),
        "无 L1 墓碑必须只报不删：{decision:?}"
    );
    // session 缺失不构成删除依据（REQ-DRG-20 ③）。
    assert_report_only(&decision, "L1");
}

#[test]
fn dangling_intent_with_l1_evidence_is_delete() {
    let temp = TempDir::new().unwrap();
    let (paths, _issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decision = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-plan-intent.json",
        &resolver,
    )
    .expect("decide");
    match decision {
        CleanupDecision::Delete {
            evidence_level,
            action,
            record_type,
            group_key,
            ..
        } => {
            assert_eq!(evidence_level, "L1");
            assert_eq!(action, "delete_record");
            assert_eq!(record_type, "automation_plan_intent");
            assert!(group_key.is_none(), "intent 为单记录删除，不带簇键");
        }
        other => panic!("必须判定为可删：{other:?}"),
    }
}

#[test]
fn dangling_window_cluster_is_grouped_by_attempt() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let ok_worktree = temp.path().join("missing-ok");
    let leased_worktree = temp.path().join("missing-leased");
    for (window_id, worktree) in [("win_ok", &ok_worktree), ("win_leased", &leased_worktree)] {
        write_window(
            &paths,
            window_id,
            "coding_attempt_9f3",
            worktree,
            "closed_accepted",
        );
    }
    write_lease(&paths, &leased_worktree);

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decisions = decide_window_cluster_cleanup(&paths, &resolver).expect("decide");
    assert_eq!(
        decisions.len(),
        1,
        "同一 attempt 只出一条簇级决策：{decisions:?}"
    );
    assert!(
        decisions
            .iter()
            .all(|decision| matches!(decision, CleanupDecision::ReportOnly { .. })),
        "同一 attempt 下一窗不满足即整 attempt 不删：{decisions:?}"
    );
}

/// 聚合的另一面：该 attempt 名下全部窗口都满足 W-1..W-4 → 一条簇级 Delete。
#[test]
fn dangling_window_cluster_all_windows_hold_is_delete() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    for window_id in ["win_a", "win_b"] {
        write_window(
            &paths,
            window_id,
            "coding_attempt_9f3",
            &temp.path().join(format!("missing-{window_id}")),
            "closed_accepted",
        );
    }

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decisions = decide_window_cluster_cleanup(&paths, &resolver).expect("decide");
    assert_eq!(
        decisions.len(),
        1,
        "同一 attempt 聚合为一条决策：{decisions:?}"
    );
    match &decisions[0] {
        CleanupDecision::Delete {
            evidence_level,
            action,
            target_kind,
            target_id,
            group_key,
            ..
        } => {
            assert_eq!(evidence_level, "L2");
            assert_eq!(action, "delete_record_group");
            assert_eq!(target_kind, "attempt");
            assert_eq!(target_id, "coding_attempt_9f3");
            assert_eq!(group_key.as_deref(), Some("coding_attempt_9f3"));
        }
        other => panic!("四条件全满足必须可删：{other:?}"),
    }
}

/// 任一窗口非终态 → 整簇不删（删除粒度按 attempt，禁止部分可删）。
#[test]
fn dangling_window_cluster_non_terminal_window_blocks_cluster() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write_window(
        &paths,
        "win_terminal",
        "coding_attempt_9f3",
        &temp.path().join("missing-a"),
        "closed_accepted",
    );
    write_window(
        &paths,
        "win_running",
        "coding_attempt_9f3",
        &temp.path().join("missing-b"),
        "running",
    );

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decisions = decide_window_cluster_cleanup(&paths, &resolver).expect("decide");
    assert_eq!(decisions.len(), 1);
    assert!(
        matches!(decisions[0], CleanupDecision::ReportOnly { .. }),
        "非终态窗口使整簇不删：{decisions:?}"
    );
}

#[test]
fn dangling_ledger_entry_without_tombstone_is_report_only() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let entry = EntityIdentityBindingEntry {
        record_path: "state/entity-identity/sessions/s1.json".to_string(),
        session_id: "s1".to_string(),
        entity_kind: "plan".to_string(),
        entity_id: "issue_work_item_plan_0007".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
    };
    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decision = decide_ledger_entry_cleanup(&paths, &entry, &resolver).expect("decide");
    assert!(
        matches!(decision, CleanupDecision::ReportOnly { .. }),
        "无墓碑的台账条目必须只报不删：{decision:?}"
    );
    assert_report_only(&decision, "L1");
}

/// 台账条目的另一面：plan 有 L1 墓碑 → 可删。
#[test]
fn dangling_ledger_entry_with_tombstone_is_delete() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    let entry = EntityIdentityBindingEntry {
        record_path: "state/entity-identity/sessions/s1.json".to_string(),
        session_id: "s1".to_string(),
        entity_kind: "plan".to_string(),
        entity_id: "issue_work_item_plan_0007".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
    };
    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decision = decide_ledger_entry_cleanup(&paths, &entry, &resolver).expect("decide");
    match decision {
        CleanupDecision::Delete {
            evidence_level,
            record_path,
            action,
            ..
        } => {
            assert_eq!(evidence_level, "L1");
            assert_eq!(record_path, "state/entity-identity/sessions/s1.json");
            assert_eq!(action, "delete_record");
        }
        other => panic!("有 L1 墓碑必须可删：{other:?}"),
    }
}

// ─── 四道前置条件（I-1..I-4；REQ-DRG-20）────────────────────────────

/// I-2 ①：enrollment 仍启用 → 只报不删。
#[test]
fn dangling_intent_four_preconditions_blocks_enabled_enrollment() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    write_enrollment(&issue_root, "e1", true, false);

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decision = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-plan-intent.json",
        &resolver,
    )
    .expect("decide");
    assert_report_only(&decision, "仍启用");
}

/// I-2 ①：仅禁用未终止（`enabled == false` 且无 `terminated_at`）＝仍有活授权 → 只报不删。
#[test]
fn dangling_intent_four_preconditions_blocks_disabled_but_not_terminated() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    write_enrollment(&issue_root, "e1", false, false);

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decision = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-plan-intent.json",
        &resolver,
    )
    .expect("decide");
    assert_report_only(&decision, "未终止");
}

/// I-2 ①：已终止但 enrollment 身份与 intent 不一致 → 只报不删。
#[test]
fn dangling_intent_four_preconditions_blocks_identity_mismatch() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    write_enrollment(&issue_root, "e_other", false, true);

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decision = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-plan-intent.json",
        &resolver,
    )
    .expect("decide");
    assert_report_only(&decision, "身份");
}

/// I-2 ①的正向对照：已终止且身份一致 → 前置条件放行（L1 齐备即可删）。
#[test]
fn dangling_intent_four_preconditions_allows_terminated_matching_enrollment() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    write_enrollment(&issue_root, "e1", false, true);

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decision = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-plan-intent.json",
        &resolver,
    )
    .expect("decide");
    assert!(
        matches!(decision, CleanupDecision::Delete { .. }),
        "四前置全满足必须可删：{decision:?}"
    );
}

/// I-4：generation intent 已派发（`ProviderDispatched`）→ 只报不删。
#[test]
fn dangling_intent_four_preconditions_blocks_dispatched_phase() {
    let temp = TempDir::new().unwrap();
    let (paths, _issue_root) = generation_intent_issue(temp.path(), "provider_dispatched");
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decision = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-generation-intent.json",
        &resolver,
    )
    .expect("decide");
    assert_report_only(&decision, "相位");
}

/// I-2 ②/I-4：相位以**锁内重读值**为准——同一现场先看到 `claimed`（可删），
/// 把相位推进为 `provider_dispatched` 后立刻转为只报不删；判定不缓存任何锁外旧值。
#[test]
fn dangling_intent_four_preconditions_uses_lock_inner_phase() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = generation_intent_issue(temp.path(), "claimed");
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let claimed = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-generation-intent.json",
        &resolver,
    )
    .expect("decide");
    assert!(
        matches!(claimed, CleanupDecision::Delete { .. }),
        "phase == claimed 时四前置可全满足：{claimed:?}"
    );

    // 相位在锁内被推进：下一次进入锁重读即看到 provider_dispatched。
    std::fs::write(
        issue_root.join("automation-generation-intent.json"),
        generation_intent_body("provider_dispatched"),
    )
    .unwrap();
    let advanced = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-generation-intent.json",
        &resolver,
    )
    .expect("decide");
    assert_report_only(&advanced, "相位");
}

/// I-2 ③/I-3：墓碑证据以**锁内重读值**为准——`purging` 迁移中或已出现待人工
/// 处置的删除失败事实时，L1 四条件不再成立 → 只报不删。
#[test]
fn dangling_intent_four_preconditions_uses_lock_inner_evidence() {
    let temp = TempDir::new().unwrap();
    let (paths, _issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let before = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-plan-intent.json",
        &resolver,
    )
    .expect("decide");
    assert!(
        matches!(before, CleanupDecision::Delete { .. }),
        "L1 四条件成立时必须可删：{before:?}"
    );

    // 锁内重读为「迁移中」：墓碑相位推进到 purging → L1 不再成立。
    let plan_tombstone = tombstone_path(temp.path(), "plan", "issue_work_item_plan_0007");
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&plan_tombstone).unwrap()).unwrap();
    record["phase"] = serde_json::Value::String("purging".to_string());
    record.as_object_mut().unwrap().remove("deleted_at");
    std::fs::write(&plan_tombstone, serde_json::to_vec(&record).unwrap()).unwrap();
    let purging = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-plan-intent.json",
        &resolver,
    )
    .expect("decide");
    assert_report_only(&purging, "L1");

    // 恢复为已完成，但出现待人工处置的删除失败事实 → L1 第 4 条不成立。
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    write_pending_failure(temp.path(), "plan", "issue_work_item_plan_0007");
    let pending = decide_intent_cleanup(
        &paths,
        "project_0001",
        "issue_0001",
        "automation-plan-intent.json",
        &resolver,
    )
    .expect("decide");
    assert_report_only(&pending, "L1");
}

// ─── 与既有取代路径并发（REQ-DRG-15「幂等不得免除锁语义」的并发面）─────────

/// 销账删除 intent 与既有「取代」路径（terminated enrollment 人工重开 →
/// `supersede_terminated_enrollment` 删旧代意图）**并发**对同一 intent 动手：
/// 双方均 MUST NOT 报错。
///
/// 构造性保证：两路都在 `automation-enrollment.json` 的同一把文件锁内动手，且
/// 后到者遇 `NotFound` 一律视为「已清」（`Ok(None)` / `metadata().is_ok()` 跳过）。
/// 本用例只钉「双方均无错误 + 结果确定」（intent 被清、enrollment 回到启用），
/// 不锁定谁先谁后，也不锁定内部删除条数。
#[test]
fn dangling_intent_cleanup_concurrent_with_supersede_path_never_errors() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let issue_root = paths.issue_root("project_0001", "issue_0001");
    std::fs::create_dir_all(&issue_root).unwrap();

    // ① 既有 enrollment 生命周期：启用 → 终止（已终止记录既是 intent 销账的
    //    I-1 前置条件，也是「取代」路径的入口状态）。
    let store = IssueAutomationStore::new(paths.clone());
    let source = EnrollmentSource {
        stories: vec![SourceRevisionRef {
            id: "story_spec_0005".into(),
            version: 1,
        }],
        designs: Vec::new(),
    };
    let options = EnrollmentOptions {
        author_provider: ProviderName::Codex,
        reviewer_provider: ProviderName::Codex,
        review_rounds: 1,
        superpowers_enabled: false,
        openspec_enabled: false,
        plan_approval: PlanApprovalPolicy::Manual,
        plan_options: IssueWorkItemPlanOptions {
            include_integration_tests: false,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        },
    };
    let target = EnrollmentTarget::SingleRepository {
        repository_id: "repo_1".into(),
    };
    let created = store
        .compare_and_set(
            "project_0001",
            "issue_0001",
            None,
            EnrollmentWriteCommand::Enable {
                command_id: None,
                selection_key: "selection_1".into(),
                source: source.clone(),
                options: options.clone(),
                target: target.clone(),
            },
        )
        .expect("启用 enrollment");
    store
        .terminate_plan(
            "project_0001",
            "issue_0001",
            EnrollmentTerminationRequest {
                command_id: "terminate-concurrent-1".into(),
                expected_enrollment_id: created.enrollment_id.clone(),
                expected_policy_revision: created.policy_revision,
                reason: "并发取代用例".into(),
            },
        )
        .expect("终止 enrollment");
    let terminated_revision = store
        .get("project_0001", "issue_0001")
        .expect("读 enrollment")
        .expect("enrollment 必须存在")
        .policy_revision;

    // ② 待删 plan intent（身份与已终止 enrollment 一致）+ 两条 L1 墓碑。
    let intent_body = serde_json::json!({
        "enrollment_id": created.enrollment_id,
        "prepare_intent_id": "p1",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "source": source.clone(),
        "options": options.clone(),
        "target": target.clone(),
        "plan_id": "issue_work_item_plan_0007",
        "session_id": "workspace_session_auto_x",
    });
    std::fs::write(
        issue_root.join("automation-plan-intent.json"),
        serde_json::to_vec(&intent_body).unwrap(),
    )
    .unwrap();
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");

    // ③ 两路并发（barrier 保证尽量重叠）。
    let barrier = Arc::new(Barrier::new(2));
    let cleanup_paths = paths.clone();
    let cleanup_barrier = Arc::clone(&barrier);
    let cleanup = std::thread::spawn(move || {
        cleanup_barrier.wait();
        run_cleanup(&cleanup_paths, &CleanupRequest::automatic())
    });
    let supersede_store = store.clone();
    let supersede_barrier = Arc::clone(&barrier);
    let supersede = std::thread::spawn(move || {
        supersede_barrier.wait();
        supersede_store.compare_and_set_reenable(
            "project_0001",
            "issue_0001",
            Some(terminated_revision),
            EnrollmentWriteCommand::Enable {
                command_id: Some("reenable-concurrent-1".into()),
                selection_key: "selection_1".into(),
                source,
                options,
                target,
            },
            false,
        )
    });

    let cleanup_result = cleanup.join().expect("销账线程不得 panic");
    let supersede_result = supersede.join().expect("取代线程不得 panic");
    assert!(
        cleanup_result.is_ok(),
        "销账 MUST NOT 因并发取代路径报错：{cleanup_result:?}"
    );
    assert!(
        supersede_result.is_ok(),
        "取代路径 MUST NOT 因并发销账报错：{supersede_result:?}"
    );

    // 结果确定（不锁内部次序）：intent 必被清掉（先到者删、后到者视为已清），
    // 取代后的 enrollment 必回到启用态。
    assert!(
        !issue_root.join("automation-plan-intent.json").exists(),
        "同一 intent 必须被清掉（无论哪一路先到）"
    );
    let final_enrollment = store
        .get("project_0001", "issue_0001")
        .expect("读 enrollment")
        .expect("enrollment 必须存在");
    assert!(
        final_enrollment.enabled && final_enrollment.terminated_at.is_none(),
        "取代路径必须落定启用态：{final_enrollment:?}"
    );
}

// ─── P2-A / P2-B 的销账侧口径（与巡检侧同一口径）──────────────────────

/// P2-A（销账侧既有口径的回归钉子）：窗口分区存在**不可解析**记录 → 同 attempt
/// 的窗口簇一律只报不删（巡检侧 P2-A 修复即对齐这条全局阻断口径）。
#[test]
fn dangling_window_cluster_is_report_only_when_any_window_record_unreadable() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    // 一扇四条件全成立的窗口 + 一扇坏窗口（同属窗口分区）。
    write_window(
        &paths,
        "win_ok",
        "coding_attempt_9f3",
        &temp.path().join("missing-ok"),
        "closed_accepted",
    );
    let windows_dir = paths.root().join("reviewer-windows/windows");
    std::fs::write(windows_dir.join("win_bad.json"), b"{ not json").unwrap();

    let sessions = BTreeSet::new();
    let probe = probe_false();
    let resolver = DanglingResolver::with_attempt_probe(&paths, &sessions, &probe);
    let decisions = decide_window_cluster_cleanup(&paths, &resolver).expect("decide");
    let cluster = decisions
        .iter()
        .find(|decision| match decision {
            CleanupDecision::ReportOnly { record_path, .. } => record_path.ends_with("win_ok.json"),
            CleanupDecision::Delete { record_path, .. } => record_path.ends_with("win_ok.json"),
        })
        .expect("合规窗口所在 attempt 必须有一条簇级决策");
    assert_report_only(cluster, "不可解析");
}

/// P2-B：台账条目缺 `entity_id` MUST NOT 让整轮自动销账失败——该条判只报不删，
/// 其余删除照常执行（此前空 `entity_id` → `EntityScope::validate` 的
/// `PathEscape("")` → `run_cleanup` 整轮 `Err`，一条都不删）。
#[test]
fn dangling_ledger_entry_without_entity_id_is_report_only_and_cleanup_continues() {
    let temp = TempDir::new().unwrap();
    let (paths, issue_root) = plan_intent_issue(temp.path(), plan_intent_body());
    // ① 一条可删的 plan intent（L1 齐备）。
    write_tombstone(temp.path(), "story", "story_spec_0005");
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007");
    // ② 一条可删的窗口簇（全部窗口四条件成立）。
    for window_id in ["win_a", "win_b"] {
        write_window(
            &paths,
            window_id,
            "coding_attempt_9f3",
            &temp.path().join(format!("missing-{window_id}")),
            "closed_accepted",
        );
    }
    // ③ 一条缺 entity_id 的台账条目（坏条目：entity_kind 可判、引用目标为空）。
    let sessions_dir = paths.root().join("state/entity-identity/sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::write(
        sessions_dir.join("s_bad.json"),
        serde_json::to_vec(&serde_json::json!({
            "entity_kind": "plan",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
        }))
        .unwrap(),
    )
    .unwrap();

    let outcome =
        run_cleanup(&paths, &CleanupRequest::automatic()).expect("单条坏台账不得使整轮销账失败");
    assert!(
        outcome.report_only.iter().any(|decision| matches!(
            decision,
            CleanupDecision::ReportOnly { record_path, reason }
                if record_path == "state/entity-identity/sessions/s_bad.json"
                    && reason.contains("entity_id")
        )),
        "坏条目必须只报不删并写明原因：{:?}",
        outcome.report_only
    );
    // 其余删除照常执行。
    assert!(
        !issue_root.join("automation-plan-intent.json").exists(),
        "可删 intent 必须照常删除"
    );
    assert!(
        !paths
            .root()
            .join("reviewer-windows/windows/win_a.json")
            .exists()
            && !paths
                .root()
                .join("reviewer-windows/windows/win_b.json")
                .exists(),
        "可删窗口簇必须照常删除"
    );
    assert!(
        outcome.deleted_count >= 2,
        "至少两类删除照常执行：{}",
        outcome.deleted_count
    );
}
