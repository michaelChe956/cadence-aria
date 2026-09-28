use std::sync::Arc;

use uuid::Uuid;

use super::IssueAutomationStore;
use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_automation_store::EnrollmentError;
use crate::product::json_store::{read_json, write_json};
use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
use crate::product::models::automation::{
    EnrollmentBindingIdentityInput, EnrollmentOptions, EnrollmentRebindRequest, EnrollmentSource,
    EnrollmentWriteCommand, IssueAutomationEnrollment, OperationState, SourceRevisionRef,
};
use crate::product::models::lifecycle::IssueWorkItemPlanOptions;
use crate::product::models::provider::ProviderName;

fn logical_repo(id: u16) -> LogicalRepositoryId {
    let uuid = format!("{id:08x}-0000-0000-0000-000000000000");
    LogicalRepositoryId(Uuid::parse_str(&uuid).unwrap())
}

fn source() -> EnrollmentSource {
    EnrollmentSource {
        stories: vec![SourceRevisionRef {
            id: "story_spec_0001".into(),
            version: 1,
        }],
        designs: vec![SourceRevisionRef {
            id: "design_spec_0001".into(),
            version: 1,
        }],
    }
}

fn options() -> EnrollmentOptions {
    EnrollmentOptions {
        author_provider: ProviderName::Fake,
        reviewer_provider: ProviderName::Fake,
        review_rounds: 1,
        superpowers_enabled: false,
        openspec_enabled: false,
        plan_options: IssueWorkItemPlanOptions {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        },
    }
}

fn enable(selection_key: &str, repository: LogicalRepositoryId) -> EnrollmentWriteCommand {
    EnrollmentWriteCommand::Enable {
        selection_key: selection_key.into(),
        source: source(),
        options: options(),
        logical_repository_id: repository,
        target: None,
    }
}

fn enable_with_options(
    selection_key: &str,
    repository: LogicalRepositoryId,
    options: EnrollmentOptions,
) -> EnrollmentWriteCommand {
    EnrollmentWriteCommand::Enable {
        selection_key: selection_key.into(),
        source: source(),
        options,
        logical_repository_id: repository,
        target: None,
    }
}

fn assert_conflict(error: crate::product::models::automation::EnrollmentError, expected: u64) {
    assert!(
        matches!(
            error,
            crate::product::models::automation::EnrollmentError::Conflict {
                current_revision: Some(revision)
            } if revision == expected
        ),
        "expected Conflict at revision {expected}, got {error:?}"
    );
}

#[test]
fn issue_automation_store_starts_empty_for_new_issue() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    assert_eq!(store.get("project_1", "issue_1").unwrap(), None);
}

#[test]
fn issue_automation_store_concurrent_identical_enable_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Arc::new(ProductAppPaths::new(tmp.path()));
    let (tx, rx) = std::sync::mpsc::channel();

    for _ in 0..2 {
        let paths = Arc::clone(&paths);
        let tx = tx.clone();
        std::thread::spawn(move || {
            let store = IssueAutomationStore::new((*paths).clone());
            let result = store.compare_and_set(
                "project_1",
                "issue_1",
                None,
                enable("human-choice-1", logical_repo(1)),
            );
            let _ = tx.send(result);
        });
    }
    drop(tx);
    let results: Vec<_> = rx.iter().collect();
    assert_eq!(results.len(), 2);
    let first = results[0].as_ref().unwrap();
    let second = results[1].as_ref().unwrap();
    assert_eq!(first.enrollment_id, second.enrollment_id);
    assert_eq!(first.policy_revision, 1);
    assert_eq!(second.policy_revision, 1);

    // durable 事实仅一条 JSON，且与内存返回一致。
    let on_disk: IssueAutomationEnrollment = read_json(
        &paths
            .issue_root("project_1", "issue_1")
            .join("automation-enrollment.json"),
    )
    .unwrap();
    assert_eq!(on_disk.enrollment_id, first.enrollment_id);
    assert_eq!(on_disk.prepare_intent_id, first.enrollment_id);
    assert!(on_disk.plan_id.is_none());
    assert!(on_disk.session_id.is_none());
    assert!(on_disk.enabled);
}

#[test]
fn issue_automation_store_conflicts_on_divergent_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    let first = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    assert_eq!(first.policy_revision, 1);

    let mut other_options = options();
    other_options.review_rounds = 2;
    let error = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable_with_options("human-choice-1", logical_repo(1), other_options),
        )
        .unwrap_err();
    assert_conflict(error, 1);

    // 异 selection_key / 异 logical repository 同样冲突。
    let error = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-2", logical_repo(1)),
        )
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }));
    let error = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(2)),
        )
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }));

    // durable 记录未被覆盖。
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), first);
}

#[test]
fn issue_automation_store_disable_reenable_conflicts_and_preserves_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));

    let created = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    assert_eq!(created.policy_revision, 1);

    let bound = store
        .bind_plan("project_1", "issue_1", 1, "plan_0001", "session_0001")
        .unwrap();
    assert_eq!(bound.plan_id.as_deref(), Some("plan_0001"));
    assert_eq!(bound.session_id.as_deref(), Some("session_0001"));

    let disabled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(bound.policy_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.policy_revision, bound.policy_revision + 1);

    // 旧 revision 启用 → Conflict（关闭后未认领许可无效）。
    let error = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(bound.policy_revision),
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap_err();
    assert_conflict(error, disabled.policy_revision);

    // 以当前 revision 重开：revision+1 且不抹绑定/prepare_intent。
    let reopened = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled.policy_revision),
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    assert!(reopened.enabled);
    assert_eq!(reopened.policy_revision, disabled.policy_revision + 1);
    assert_eq!(reopened.enrollment_id, created.enrollment_id);
    assert_eq!(reopened.prepare_intent_id, created.prepare_intent_id);
    assert_eq!(reopened.plan_id.as_deref(), Some("plan_0001"));
    assert_eq!(reopened.session_id.as_deref(), Some("session_0001"));
}

/// REQ-WIGA-02 契约（T2 Step 3）：enabled 状态下的 Enable 只允许同键同
/// payload 幂等；异内容（source/options/target 任一不同）一律 Conflict——
/// 即使携带当前 revision 也不得改写授权内容（改 payload 必须先 Disable
/// 重开，重开才走 revision+1 换新授权）。
#[test]
fn issue_automation_store_rejects_enabled_payload_rewrite_with_current_revision() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    let created = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    assert_eq!(created.policy_revision, 1);

    // 携当前 revision 的异 options 重写必须 Conflict，且 durable 不被改写。
    let mut other_options = options();
    other_options.review_rounds = 2;
    let error = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(created.policy_revision),
            enable_with_options("human-choice-1", logical_repo(1), other_options),
        )
        .unwrap_err();
    assert_conflict(error, created.policy_revision);
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), created);

    // 异 selection_key / 异 target 携当前 revision 同样冲突。
    let error = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(created.policy_revision),
            enable("human-choice-2", logical_repo(1)),
        )
        .unwrap_err();
    assert_conflict(error, created.policy_revision);
    let error = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(created.policy_revision),
            enable("human-choice-1", logical_repo(2)),
        )
        .unwrap_err();
    assert_conflict(error, created.policy_revision);

    // 对照：同键同 payload 携当前 revision 仍幂等返回原值。
    let idempotent = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(created.policy_revision),
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    assert_eq!(idempotent, created);

    // 对照：Disable 后重开（disabled 态）允许新 payload，revision+1 且保留身份。
    let disabled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(created.policy_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    assert!(!disabled.enabled);
    let mut reopened_options = options();
    reopened_options.review_rounds = 3;
    let reopened = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled.policy_revision),
            enable_with_options("human-choice-1", logical_repo(1), reopened_options),
        )
        .unwrap();
    assert!(reopened.enabled);
    assert_eq!(reopened.policy_revision, disabled.policy_revision + 1);
    assert_eq!(reopened.enrollment_id, created.enrollment_id);
    assert_eq!(reopened.options.review_rounds, 3);
}

#[test]
fn issue_automation_store_bind_plan_is_idempotent_and_exclusive() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    let created = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();

    let bound = store
        .bind_plan(
            "project_1",
            "issue_1",
            created.policy_revision,
            "plan_0001",
            "session_0001",
        )
        .unwrap();
    assert!(bound.policy_revision > created.policy_revision);
    let bound_revision = bound.policy_revision;

    // 同键重试：revision 不变，返回原值。
    let retried = store
        .bind_plan(
            "project_1",
            "issue_1",
            bound_revision,
            "plan_0001",
            "session_0001",
        )
        .unwrap();
    assert_eq!(retried.policy_revision, bound_revision);
    assert_eq!(retried, bound);

    // 重绑他者 plan 失败，绑定仍唯一。
    let error = store
        .bind_plan(
            "project_1",
            "issue_1",
            bound_revision,
            "plan_0002",
            "session_0002",
        )
        .unwrap_err();
    assert_conflict(error, bound_revision);
    let current = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(current.plan_id.as_deref(), Some("plan_0001"));
    assert_eq!(current.session_id.as_deref(), Some("session_0001"));

    // 缺失 enrollment / 旧 revision 拒绝绑定。
    let error = store
        .bind_plan("project_1", "issue_9", 1, "plan_0001", "session_0001")
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::NotFound));
    let error = store
        .bind_plan(
            "project_1",
            "issue_1",
            created.policy_revision,
            "plan_0001",
            "session_0001",
        )
        .unwrap_err();
    assert_conflict(error, bound_revision);
}

#[test]
fn issue_automation_store_disable_requires_existing_enrollment() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    let error = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            EnrollmentWriteCommand::Disable,
        )
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::NotFound));
}
// ---- P1 WIGA Task 4：enrollment-bound 创建意图、唯一绑定与半提交恢复 ----

fn intent_path(paths: &ProductAppPaths) -> std::path::PathBuf {
    paths
        .issue_root("project_1", "issue_1")
        .join("automation-plan-intent.json")
}

fn read_intent(
    paths: &ProductAppPaths,
) -> crate::product::models::automation::PreparedPlanIntent {
    read_json(&intent_path(paths)).unwrap()
}

#[test]
fn issue_automation_store_ensure_plan_binding_binds_once_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());
    let enrolled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();

    let creates = std::sync::atomic::AtomicUsize::new(0);
    let bound = store
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |current, intent| {
            creates.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(current.enrollment_id, enrolled.enrollment_id);
            assert_eq!(
                intent.plan_id,
                format!("issue_work_item_plan_auto_{}", current.prepare_intent_id)
            );
            assert_eq!(
                intent.session_id,
                format!("workspace_session_auto_{}", current.prepare_intent_id)
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(bound.policy_revision, enrolled.policy_revision + 1);
    assert_eq!(
        bound.plan_id.as_deref(),
        Some(
            format!("issue_work_item_plan_auto_{}", enrolled.prepare_intent_id).as_str()
        )
    );
    assert_eq!(
        bound.session_id.as_deref(),
        Some(
            format!("workspace_session_auto_{}", enrolled.prepare_intent_id).as_str()
        )
    );

    // 重复补偿：完整绑定 → Unchanged 幂等，不再调用 create，revision 不再 +1。
    let again = store
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
            panic!("bound enrollment must not re-create");
        })
        .unwrap();
    assert_eq!(again.policy_revision, bound.policy_revision);
    assert_eq!(again.plan_id, bound.plan_id);
    assert_eq!(creates.load(std::sync::atomic::Ordering::SeqCst), 1);

    // 意图文件冻结且与 enrollment 派生一致。
    let intent = read_intent(&paths);
    assert_eq!(intent.enrollment_id, enrolled.enrollment_id);
    assert_eq!(intent.source, enrolled.source);
    assert_eq!(intent.options, enrolled.options);
}

/// 中窗恢复：create 失败（如 preflight/源校验）→ 失败原样穿出且不落绑定；
/// 意图已先于 create 落盘，下一次补偿按同一冻结身份重建。
#[test]
fn issue_automation_store_ensure_plan_binding_recovers_after_create_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());
    let enrolled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();

    let error = store
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
            Err(EnrollmentError::InvalidScope("create failed".into()))
        })
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::InvalidScope(_)));
    let current = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(current.plan_id.is_none());
    assert!(current.session_id.is_none());
    assert!(intent_path(&paths).exists());

    let bound = store
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| Ok(()))
        .unwrap();
    assert_eq!(bound.policy_revision, enrolled.policy_revision + 1);
}

/// Disable → 异 payload 重开（保留绑定/prepare_intent_id）：intent 文件
/// 仍是旧冻结快照，与当前 enrollment 派生的新意图不一致 → fail-closed，
/// 绝不把旧 plan/session 隐式重授权给新 payload。
#[test]
fn issue_automation_store_ensure_plan_binding_fails_closed_on_divergent_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());
    let enrolled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    let bound = store
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| Ok(()))
        .unwrap();
    assert_eq!(bound.policy_revision, enrolled.policy_revision + 1);

    let disabled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(enrolled.policy_revision + 1),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let mut other_options = options();
    other_options.review_rounds = 2;
    let reopened = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled.policy_revision),
            enable_with_options("human-choice-1", logical_repo(1), other_options),
        )
        .unwrap();
    assert!(reopened.enabled);

    let error = store
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
            panic!("divergent reopen must not re-create");
        })
        .unwrap_err();
    assert_conflict(error, reopened.policy_revision);
    let current = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(current.plan_id, bound.plan_id);
    assert_eq!(read_intent(&paths).options, options());
}

/// 半提交损坏（单边 plan_id/session_id）与禁用/缺失/异 enrollment_id fail-closed。
#[test]
fn issue_automation_store_ensure_plan_binding_rejects_corrupt_disabled_and_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());

    let error = store
        .ensure_plan_binding("project_1", "issue_1", "missing", |_, _| Ok(()))
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::NotFound));

    let enrolled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();

    let disabled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(enrolled.policy_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let error = store
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
            panic!("disabled enrollment must not create");
        })
        .unwrap_err();
    assert_conflict(error, disabled.policy_revision);

    store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled.policy_revision),
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    let enrollment_file = paths
        .issue_root("project_1", "issue_1")
        .join("automation-enrollment.json");
    let mut corrupted = store.get("project_1", "issue_1").unwrap().unwrap();
    corrupted.plan_id = Some("plan_leftover".into());
    write_json(&enrollment_file, &corrupted).unwrap();
    let error = store
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
            panic!("half-bound enrollment must not create");
        })
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }));

    let mut other = corrupted.clone();
    other.plan_id = None;
    write_json(&enrollment_file, &other).unwrap();
    let error = store
        .ensure_plan_binding("project_1", "issue_1", "another-enrollment", |_, _| Ok(()))
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }));
}

/// P1 Task 5：生成检查点——首次认领 Claimed；EngineStarted 后重复认领
/// 原样返回（进程重建按 phase 分诊）；换源重开身份漂移 fail-closed。
#[test]
fn issue_automation_store_plan_generation_checkpoint_phases_and_conflicts() {
    use crate::product::models::automation::{PlanGenerationPhase, PlanGenerationIntent};

    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());
    let enrolled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    store
        .bind_plan("project_1", "issue_1", 1, "plan_bound", "session_bound")
        .unwrap();

    // 未绑定时拒绝认领。
    let unbound_dir = tempfile::tempdir().unwrap();
    let unbound_store = IssueAutomationStore::new(ProductAppPaths::new(unbound_dir.path()));
    let unbound = unbound_store
        .compare_and_set("project_2", "issue_1", None, enable("k", logical_repo(1)))
        .unwrap();
    let error = unbound_store
        .claim_plan_generation("project_2", "issue_1", &unbound.enrollment_id)
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::InvalidScope(_)));

    let claimed = store
        .claim_plan_generation("project_1", "issue_1", &enrolled.enrollment_id)
        .unwrap();
    assert_eq!(claimed.phase, PlanGenerationPhase::Claimed);
    assert_eq!(claimed.plan_id, "plan_bound");
    assert_eq!(claimed.session_id, "session_bound");
    assert_eq!(
        claimed.action_key,
        PlanGenerationIntent::action_key_for(&enrolled.enrollment_id, "plan_bound")
    );

    // 重复认领幂等返回既存检查点（含已推进 phase）。
    store
        .mark_plan_generation_phase(
            "project_1",
            "issue_1",
            &enrolled.enrollment_id,
            PlanGenerationPhase::EngineStarted,
        )
        .unwrap();
    let resumed = store
        .claim_plan_generation("project_1", "issue_1", &enrolled.enrollment_id)
        .unwrap();
    assert_eq!(resumed.phase, PlanGenerationPhase::EngineStarted);

    // 禁用/异 enrollment_id 拒绝推进。
    let error = store
        .mark_plan_generation_phase(
            "project_1",
            "issue_1",
            "another-enrollment",
            PlanGenerationPhase::ProviderDispatched,
        )
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }));

    // Disable → 异 payload 重开：派生身份漂移 → 认领 fail-closed。
    let bound_revision = store.get("project_1", "issue_1").unwrap().unwrap().policy_revision;
    store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(bound_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let disabled_revision = store.get("project_1", "issue_1").unwrap().unwrap().policy_revision;
    let mut other_options = options();
    other_options.review_rounds = 2;
    store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled_revision),
            enable_with_options("human-choice-1", logical_repo(1), other_options),
        )
        .unwrap();
    let error = store
        .claim_plan_generation("project_1", "issue_1", &enrolled.enrollment_id)
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }));
}

/// 双实例/双线程并发补偿同一 enrollment：恰一次 create+bind，另一侧 Unchanged。
#[test]
fn issue_automation_store_ensure_plan_binding_concurrent_workers_bind_once() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Arc::new(ProductAppPaths::new(tmp.path()));
    let enrolled = IssueAutomationStore::new((*paths).clone())
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    let enrollment_id = Arc::new(enrolled.enrollment_id.clone());
    let creates = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let (tx, rx) = std::sync::mpsc::channel();

    for _ in 0..2 {
        let paths = Arc::clone(&paths);
        let enrollment_id = Arc::clone(&enrollment_id);
        let creates = Arc::clone(&creates);
        let barrier = Arc::clone(&barrier);
        let tx = tx.clone();
        std::thread::spawn(move || {
            barrier.wait();
            let store = IssueAutomationStore::new((*paths).clone());
            let result = store.ensure_plan_binding(
                "project_1",
                "issue_1",
                &enrollment_id,
                |_, _| {
                    creates.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                },
            );
            let _ = tx.send(result);
        });
    }
    drop(tx);
    let results: Vec<_> = rx.iter().collect();
    assert_eq!(results.len(), 2);
    let first = results[0].as_ref().unwrap();
    let second = results[1].as_ref().unwrap();
    assert_eq!(first.enrollment_id, second.enrollment_id);
    assert_eq!(first.plan_id, second.plan_id);
    assert_eq!(first.session_id, second.session_id);
    assert_eq!(first.policy_revision, second.policy_revision);
    assert_eq!(creates.load(std::sync::atomic::Ordering::SeqCst), 1);
    let on_disk: IssueAutomationEnrollment = read_json(
        &paths
            .issue_root("project_1", "issue_1")
            .join("automation-enrollment.json"),
    )
    .unwrap();
    assert_eq!(on_disk.plan_id, first.plan_id);
    assert_eq!(on_disk.session_id, first.session_id);
}

// ---------------------------------------------------------------------------
// C1 Task 1：双载体 target union 与版本化 enrollment binding（REQ-C1-TARGET-01）。
// ---------------------------------------------------------------------------

fn target_logical() -> EnrollmentTarget {
    EnrollmentTarget::LogicalCodebase {
        logical_codebase_id: "logical_codebase_0001".into(),
        logical_repository_id: logical_repo(1),
    }
}

fn enable_with_target(
    selection_key: &str,
    repository: LogicalRepositoryId,
    target: EnrollmentTarget,
) -> EnrollmentWriteCommand {
    EnrollmentWriteCommand::Enable {
        selection_key: selection_key.into(),
        source: source(),
        options: options(),
        logical_repository_id: repository,
        target: Some(target),
    }
}

fn rebind_request(
    command_id: &str,
    expected_policy_revision: u64,
    expected_binding_version: u64,
    plan_id: &str,
    session_id: &str,
    target: EnrollmentTarget,
) -> EnrollmentRebindRequest {
    EnrollmentRebindRequest {
        command_id: command_id.into(),
        expected_policy_revision,
        expected_binding_version,
        binding: EnrollmentBindingIdentityInput {
            plan_id: plan_id.into(),
            session_id: session_id.into(),
            source: source(),
            target,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
        },
        reason: "recover after failed generation".into(),
    }
}

/// 单仓 target 序列化保留真实 physical id，读取后不生成 logical 替身；
/// 逻辑 target 缺任一级身份即拒绝（REQ-C1-TARGET-01）。
#[test]
fn issue_automation_store_target_union_roundtrip_preserves_physical_id() {
    let single = EnrollmentTarget::SingleRepository {
        repository_id: "repo_physical_1".into(),
    };
    let json = serde_json::to_value(&single).unwrap();
    assert_eq!(json["kind"], "single_repository");
    assert_eq!(json["repository_id"], "repo_physical_1");
    assert!(
        json.get("logical_repository_id").is_none(),
        "single repository target must not grow a logical stand-in: {json}"
    );
    let back: EnrollmentTarget = serde_json::from_value(json).unwrap();
    assert_eq!(back, single);

    let logical = target_logical();
    let json = serde_json::to_value(&logical).unwrap();
    assert_eq!(json["kind"], "logical_codebase");
    let back: EnrollmentTarget = serde_json::from_value(json).unwrap();
    assert_eq!(back, logical);

    let missing_codebase = serde_json::json!({
        "kind": "logical_codebase",
        "logical_repository_id": logical_repo(1).0.to_string(),
    });
    assert!(
        serde_json::from_value::<EnrollmentTarget>(missing_codebase).is_err(),
        "logical target without logical_codebase_id must be rejected"
    );
    let missing_repository = serde_json::json!({
        "kind": "logical_codebase",
        "logical_codebase_id": "logical_codebase_0001",
    });
    assert!(
        serde_json::from_value::<EnrollmentTarget>(missing_repository).is_err(),
        "logical target without logical_repository_id must be rejected"
    );
}

/// 显式 rebind 在 enrollment 文件锁内追加版本历史：previous 逐字保留旧代、
/// 同 command 同 payload 幂等重放、异 payload / 旧 expected version / 跨载体
/// target fail-closed 且 durable 不变（REQ-WIGA-01、REQ-C1-TARGET-01）。
#[test]
fn issue_automation_store_rebind_appends_version_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));

    // 新式 enable 显式声明 target → durable binding v1（plan/session 绑定后补全）。
    let created = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable_with_target("human-choice-1", logical_repo(1), target_logical()),
        )
        .unwrap();
    assert_eq!(created.policy_revision, 1);
    assert_eq!(
        created
            .binding_history
            .as_ref()
            .unwrap()
            .current
            .binding_version,
        1
    );
    let bound = store
        .bind_plan(
            "project_1",
            "issue_1",
            created.policy_revision,
            "plan_0001",
            "session_0001",
        )
        .unwrap();
    let v1 = bound.binding_history.as_ref().unwrap().current.clone();
    assert_eq!(v1.plan_id, "plan_0001");
    assert_eq!(v1.session_id, "session_0001");
    assert_eq!(v1.target, target_logical());
    assert_eq!(v1.author_provider, bound.options.author_provider);
    assert!(bound.binding_history.as_ref().unwrap().previous.is_empty());

    // 首次 rebind：expected policy/binding 匹配当前 → binding_version=2，
    // previous 保留 v1 的完整 plan/session/source/target/provider。
    let request = rebind_request(
        "rebind_cmd_0001",
        bound.policy_revision,
        1,
        "plan_0002",
        "session_0002",
        target_logical(),
    );
    let result = store
        .rebind("project_1", "issue_1", request.clone())
        .unwrap();
    assert_eq!(result.command_id, "rebind_cmd_0001");
    assert_eq!(result.state, OperationState::Accepted);
    let after = store.get("project_1", "issue_1").unwrap().unwrap();
    let history = after.binding_history.as_ref().unwrap();
    assert_eq!(history.current.binding_version, 2);
    assert_eq!(history.current.plan_id, "plan_0002");
    assert_eq!(history.current.session_id, "session_0002");
    assert_eq!(history.current.enrollment_id, bound.enrollment_id);
    assert_eq!(history.previous.len(), 1);
    assert_eq!(history.previous[0], v1);
    // enrollment 的当前绑定投影与 current 代一致；rebind 是 CAS 修订。
    assert_eq!(after.plan_id.as_deref(), Some("plan_0002"));
    assert_eq!(after.session_id.as_deref(), Some("session_0002"));
    assert!(after.policy_revision > bound.policy_revision);

    // 同 command 同 payload 重放：返回同一 binding_version=2，durable 不变。
    let replay = store
        .rebind("project_1", "issue_1", request.clone())
        .unwrap();
    assert_eq!(replay.state, OperationState::Replayed);
    assert_eq!(replay.enrollment, after);
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), after);

    // 同 command 异 payload → Conflict，current/previous 不变。
    let mut diverged = request.clone();
    diverged.binding.plan_id = "plan_0003".into();
    let error = store
        .rebind("project_1", "issue_1", diverged)
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }), "{error:?}");
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), after);

    // 旧 expected binding version → Conflict。
    let stale_binding = rebind_request(
        "rebind_cmd_0002",
        after.policy_revision,
        1,
        "plan_0003",
        "session_0003",
        target_logical(),
    );
    let error = store
        .rebind("project_1", "issue_1", stale_binding)
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }), "{error:?}");

    // 旧 expected policy revision → Conflict。
    let stale_policy = rebind_request(
        "rebind_cmd_0003",
        bound.policy_revision,
        2,
        "plan_0003",
        "session_0003",
        target_logical(),
    );
    let error = store
        .rebind("project_1", "issue_1", stale_policy)
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::Conflict { .. }), "{error:?}");

    // 跨载体 target（logical → single）→ 拒绝，durable 不变。
    let cross_carrier = rebind_request(
        "rebind_cmd_0004",
        after.policy_revision,
        2,
        "plan_0003",
        "session_0003",
        EnrollmentTarget::SingleRepository {
            repository_id: "repo_physical_1".into(),
        },
    );
    let error = store
        .rebind("project_1", "issue_1", cross_carrier)
        .unwrap_err();
    assert!(
        matches!(error, EnrollmentError::Conflict { .. })
            || matches!(error, EnrollmentError::InvalidScope(_)),
        "{error:?}"
    );
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), after);
}

/// 缺新字段的旧 enrollment JSON 按 off/Manual 兼容读：不自动补
/// plan/session/binding；无声明 target 的旧 enrollment rebind fail-closed。
#[test]
fn issue_automation_store_legacy_enrollment_json_reads_without_binding() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());
    let created = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable_with_target("human-choice-1", logical_repo(1), target_logical()),
        )
        .unwrap();
    let enrollment_file = paths
        .issue_root("project_1", "issue_1")
        .join("automation-enrollment.json");

    // 剥离 C1 新字段模拟旧格式 durable JSON（空 command_ledger 本就
    // skip 序列化，与旧格式逐字节一致）。
    let mut legacy: serde_json::Value = read_json(&enrollment_file).unwrap();
    let object = legacy.as_object_mut().unwrap();
    assert!(object.remove("target").is_some());
    assert!(object.remove("binding_history").is_some());
    assert!(object.remove("command_ledger").is_none());
    write_json(&enrollment_file, &legacy).unwrap();

    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(enrollment.target.is_none());
    assert!(enrollment.binding_history.is_none());
    assert!(enrollment.command_ledger.is_empty());
    assert_eq!(enrollment.plan_id, None);
    assert_eq!(enrollment.session_id, None);
    assert_eq!(enrollment.enrollment_id, created.enrollment_id);
    assert_eq!(
        enrollment.logical_repository_id,
        created.logical_repository_id
    );

    // 旧 enrollment 无声明 target：rebind 不猜 target，fail-closed。
    let error = store
        .rebind(
            "project_1",
            "issue_1",
            rebind_request(
                "rebind_cmd_legacy",
                enrollment.policy_revision,
                1,
                "plan_0002",
                "session_0002",
                target_logical(),
            ),
        )
        .unwrap_err();
    assert!(matches!(error, EnrollmentError::InvalidScope(_)), "{error:?}");
    assert_eq!(store.get("project_1", "issue_1").unwrap().unwrap(), enrollment);
}
