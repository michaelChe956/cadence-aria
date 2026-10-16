use std::sync::Arc;

use uuid::Uuid;

use super::IssueAutomationStore;
use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_automation_store::EnrollmentError;
use crate::product::json_store::{read_json, write_json};
use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
use crate::product::models::automation::{
    AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
    EnrollmentBindingIdentity, EnrollmentBindingIdentityInput, EnrollmentOptions,
    EnrollmentRebindRequest, EnrollmentSource, EnrollmentTerminationRequest,
    EnrollmentWriteCommand, IssueAutomationEnrollment, OperationState, PlanApprovalPolicy,
    SourceRevisionRef,
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
        plan_approval: PlanApprovalPolicy::Manual,
        plan_options: IssueWorkItemPlanOptions {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        },
    }
}

fn enable(selection_key: &str, repository: LogicalRepositoryId) -> EnrollmentWriteCommand {
    enable_with_target(selection_key, target_logical_at(repository))
}

fn enable_with_options(
    selection_key: &str,
    repository: LogicalRepositoryId,
    options: EnrollmentOptions,
) -> EnrollmentWriteCommand {
    let target = target_logical_at(repository);
    EnrollmentWriteCommand::Enable {
        selection_key: selection_key.into(),
        source: source(),
        options,
        target,
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

// ---- 批 2 P1：plan_approval 冻结归一（REQ-WIGA-02 修订）----

/// ①④auto 授权冻结 plan_approval 并把 require_execution_plan_confirm 压
/// false（不论复选框原值）；同 payload 幂等；③manual 复选框原样保留。
#[test]
fn issue_automation_store_freezes_plan_approval_and_normalizes_confirm_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));

    let mut auto_options = options();
    auto_options.plan_approval = PlanApprovalPolicy::Auto;
    auto_options.plan_options.require_execution_plan_confirm = true;
    let enrolled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable_with_options("human-choice-1", logical_repo(1), auto_options.clone()),
        )
        .unwrap();
    assert_eq!(enrolled.options.plan_approval, PlanApprovalPolicy::Auto);
    assert!(!enrolled.options.plan_options.require_execution_plan_confirm);

    // durable 事实与返回一致（冻结归一已落盘）。
    let on_disk = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(on_disk.options, enrolled.options);

    // 同 payload（复选框仍 true）重发 → 幂等 Unchanged：revision 不变。
    let replayed = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable_with_options("human-choice-1", logical_repo(1), auto_options),
        )
        .unwrap();
    assert_eq!(replayed.policy_revision, enrolled.policy_revision);
    assert_eq!(replayed.enrollment_id, enrolled.enrollment_id);
    assert!(!replayed.options.plan_options.require_execution_plan_confirm);

    // manual + 复选框 true → 原样保留（手动复选框行为不变）。
    let mut manual_options = options();
    manual_options.plan_options.require_execution_plan_confirm = true;
    let manual = store
        .compare_and_set(
            "project_2",
            "issue_1",
            None,
            enable_with_options("human-choice-1", logical_repo(1), manual_options),
        )
        .unwrap();
    assert_eq!(manual.options.plan_approval, PlanApprovalPolicy::Manual);
    assert!(manual.options.plan_options.require_execution_plan_confirm);
}

/// ①旧 durable JSON 缺 plan_approval 字段按 manual 读（旧记录不迁移，
/// 已绑定旧 Interactive session 不受影响）。
#[test]
fn issue_automation_store_reads_legacy_options_without_plan_approval_as_manual() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path());
    let store = IssueAutomationStore::new(paths.clone());

    let mut auto_options = options();
    auto_options.plan_approval = PlanApprovalPolicy::Auto;
    let enrolled = store
        .compare_and_set(
            "project_1",
            "issue_1",
            None,
            enable_with_options("human-choice-1", logical_repo(1), auto_options),
        )
        .unwrap();

    // 抹掉 durable JSON 的 plan_approval 字段，模拟批 2 之前的旧记录。
    let path = paths
        .issue_root("project_1", "issue_1")
        .join("automation-enrollment.json");
    let mut raw: serde_json::Value = read_json(&path).unwrap();
    raw["options"]
        .as_object_mut()
        .unwrap()
        .remove("plan_approval");
    write_json(&path, &raw).unwrap();

    let loaded = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(loaded.options.plan_approval, PlanApprovalPolicy::Manual);
    assert_eq!(loaded.enrollment_id, enrolled.enrollment_id);
    assert!(!loaded.options.plan_options.require_execution_plan_confirm);
}
// ---- P1 WIGA Task 4：enrollment-bound 创建意图、唯一绑定与半提交恢复 ----

fn intent_path(paths: &ProductAppPaths) -> std::path::PathBuf {
    paths
        .issue_root("project_1", "issue_1")
        .join("automation-plan-intent.json")
}

fn read_intent(paths: &ProductAppPaths) -> crate::product::models::automation::PreparedPlanIntent {
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
        .ensure_plan_binding(
            "project_1",
            "issue_1",
            &enrolled.enrollment_id,
            |current, intent| {
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
            },
        )
        .unwrap();
    assert_eq!(bound.policy_revision, enrolled.policy_revision + 1);
    assert_eq!(
        bound.plan_id.as_deref(),
        Some(format!("issue_work_item_plan_auto_{}", enrolled.prepare_intent_id).as_str())
    );
    assert_eq!(
        bound.session_id.as_deref(),
        Some(format!("workspace_session_auto_{}", enrolled.prepare_intent_id).as_str())
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
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
            Ok(())
        })
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
        .ensure_plan_binding("project_1", "issue_1", &enrolled.enrollment_id, |_, _| {
            Ok(())
        })
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
    use crate::product::models::automation::{PlanGenerationIntent, PlanGenerationPhase};

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
    let bound_revision = store
        .get("project_1", "issue_1")
        .unwrap()
        .unwrap()
        .policy_revision;
    store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(bound_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let disabled_revision = store
        .get("project_1", "issue_1")
        .unwrap()
        .unwrap()
        .policy_revision;
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
            let result =
                store.ensure_plan_binding("project_1", "issue_1", &enrollment_id, |_, _| {
                    creates.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                });
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
// 批 2 N0（REQ-WIGA-09/12）：自动化待处理事实 store 与 terminated_at 纯字段。
// ---------------------------------------------------------------------------

/// N0 测试共享现场：enable＋bind 后返回当前 durable binding 与 revision。
fn bound_enrollment_with_binding() -> (
    tempfile::TempDir,
    IssueAutomationStore,
    EnrollmentBindingIdentity,
    u64,
) {
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
    let binding = bound
        .binding_history
        .clone()
        .expect("binding history present after bind")
        .current;
    (tmp, store, binding, bound.policy_revision)
}

fn attention_fact(key: &str, occurred_at: &str) -> AutomationAttentionFact {
    AutomationAttentionFact {
        key: key.to_string(),
        kind: AutomationAttentionKind::AutomationFailed,
        stage: AutomationAttentionStage::Advance,
        plan_id: Some("plan_0001".to_string()),
        session_id: Some("session_0001".to_string()),
        attempt_id: None,
        reason_code: "initialization_failed".to_string(),
        occurred_at: occurred_at.to_string(),
        choice_id: None,
        gate_id: None,
    }
}

fn assert_invalid_scope(error: EnrollmentError) {
    assert!(
        matches!(error, EnrollmentError::InvalidScope(_)),
        "expected InvalidScope, got {error:?}"
    );
}

fn enrollment_json_path(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    ProductAppPaths::new(tmp.path())
        .issue_root("project_1", "issue_1")
        .join("automation-enrollment.json")
}

/// 红测①：精确绑定写/清；同 key 不增项、occurred_at 首次冻结（重入
/// 保留）；无 checkpoint 不伪造 attempt；事实写不递增 policy_revision。
#[test]
fn issue_automation_store_upsert_attention_fact_writes_resolves_and_freezes_time() {
    let (_tmp, store, binding, revision) = bound_enrollment_with_binding();
    let mut fact = attention_fact("plan-q:choice-1", "2026-10-16T00:00:00Z");
    fact.kind = AutomationAttentionKind::PlanQuestion;
    fact.stage = AutomationAttentionStage::Plan;
    fact.attempt_id = Some("attempt_0001".to_string());
    fact.choice_id = Some("choice_0001".to_string());
    fact.gate_id = Some("gate_0001".to_string());
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &binding, fact.clone())
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts, vec![fact.clone()]);
    assert_eq!(
        saved.policy_revision, revision,
        "事实写不得递增 policy_revision"
    );

    // 同 key 重入：不增项，occurred_at 首次冻结，reason 取最新。
    let mut again = attention_fact("plan-q:choice-1", "2026-10-16T09:00:00Z");
    again.kind = AutomationAttentionKind::PlanQuestion;
    again.stage = AutomationAttentionStage::Plan;
    again.attempt_id = Some("attempt_0001".to_string());
    again.reason_code = "trust_waiting_again".to_string();
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &binding, again)
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts.len(), 1, "同 key 重复 upsert 不增项");
    assert_eq!(
        saved.attention_facts[0].occurred_at, "2026-10-16T00:00:00Z",
        "occurred_at 首次冻结，重入保留"
    );
    assert_eq!(saved.attention_facts[0].reason_code, "trust_waiting_again");
    assert_eq!(saved.policy_revision, revision);

    // 无 checkpoint 的 advance 失败事实：attempt 身份不伪造（None 原样）。
    let checkpointless = attention_fact("cmd-1:advance:retry", "2026-10-16T00:00:00Z");
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &binding, checkpointless.clone())
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts.len(), 2);
    assert_eq!(saved.attention_facts[1], checkpointless);

    // 精确绑定清 key；重复清幂等成功。
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "plan-q:choice-1")
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts, vec![checkpointless]);
    assert_eq!(saved.policy_revision, revision);
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "plan-q:choice-1")
        .unwrap();
}

/// 红测③/④：旧/别的绑定拒绝；事实身份与绑定互证失败 InvalidScope；
/// disabled 未终止阻断写/清；缺失与错误上抛不吞。
#[test]
fn issue_automation_store_attention_fact_requires_exact_binding() {
    let (_tmp, store, binding, revision) = bound_enrollment_with_binding();
    let fact = attention_fact("k1", "2026-10-16T00:00:00Z");

    // 别的 enrollment 身份：写/清均拒绝。
    let mut foreign = binding.clone();
    foreign.enrollment_id = "other_enrollment".to_string();
    assert_conflict(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &foreign, fact.clone())
            .unwrap_err(),
        revision,
    );
    assert_conflict(
        store
            .resolve_automation_attention_fact("project_1", "issue_1", &foreign, "k1")
            .unwrap_err(),
        revision,
    );

    // 事实身份与绑定互证失败：plan/session 漂移 InvalidScope。
    let mut drift = attention_fact("k2", "2026-10-16T00:00:00Z");
    drift.plan_id = Some("plan_other".to_string());
    assert_invalid_scope(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &binding, drift)
            .unwrap_err(),
    );
    let mut drift = attention_fact("k3", "2026-10-16T00:00:00Z");
    drift.session_id = Some("session_other".to_string());
    assert_invalid_scope(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &binding, drift)
            .unwrap_err(),
    );

    // 换代后旧代 binding 写/清拒绝。
    let bound = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .rebind(
            "project_1",
            "issue_1",
            rebind_request(
                "cmd-rebind-1",
                bound.policy_revision,
                binding.binding_version,
                "plan_0002",
                "session_0002",
                target_logical(),
            ),
        )
        .unwrap();
    let after = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_conflict(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &binding, fact)
            .unwrap_err(),
        after.policy_revision,
    );
    assert_conflict(
        store
            .resolve_automation_attention_fact("project_1", "issue_1", &binding, "k1")
            .unwrap_err(),
        after.policy_revision,
    );

    // disabled 未终止：同一绑定写/清一律 Conflict（终止收尾除外，见下个测试）。
    let v2 = after.binding_history.clone().unwrap().current;
    let mut fact_v2 = attention_fact("k1", "2026-10-16T00:00:00Z");
    fact_v2.plan_id = Some("plan_0002".to_string());
    fact_v2.session_id = Some("session_0002".to_string());
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &v2, fact_v2.clone())
        .unwrap();
    let before_disable = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(before_disable.policy_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let disabled = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.terminated_at, None);
    assert_conflict(
        store
            .upsert_automation_attention_fact("project_1", "issue_1", &v2, fact_v2)
            .unwrap_err(),
        disabled.policy_revision,
    );
    assert_conflict(
        store
            .resolve_automation_attention_fact("project_1", "issue_1", &v2, "k1")
            .unwrap_err(),
        disabled.policy_revision,
    );

    // enrollment 缺失：NotFound 上抛不吞。
    assert!(matches!(
        store
            .upsert_automation_attention_fact(
                "project_1",
                "issue_9",
                &v2,
                attention_fact("k9", "t"),
            )
            .unwrap_err(),
        EnrollmentError::NotFound
    ));
    // 非法路径 id：Store 错误上抛。
    assert!(matches!(
        store
            .upsert_automation_attention_fact(
                "../escape",
                "issue_1",
                &v2,
                attention_fact("k9", "t"),
            )
            .unwrap_err(),
        EnrollmentError::Store(_)
    ));
    assert!(matches!(
        store
            .resolve_automation_attention_fact("project_1", "../escape", &v2, "k9")
            .unwrap_err(),
        EnrollmentError::Store(_)
    ));
}

/// 红测③：同一已 terminated 绑定（enabled=false）可清旧事实/写
/// termination 清理失败，不因 enabled=false 阻断终止收尾；重清成功即消失。
#[test]
fn issue_automation_store_attention_fact_terminated_binding_clears_and_writes() {
    let (tmp, store, binding, _revision) = bound_enrollment_with_binding();
    store
        .upsert_automation_attention_fact(
            "project_1",
            "issue_1",
            &binding,
            attention_fact("plan-q1", "2026-10-16T00:00:00Z"),
        )
        .unwrap();

    // 模拟 T1 终止 CAS 的 durable 结果：terminated_at 落盘＋enabled=false
    //（N0 只提供纯字段，写入归 T1）。
    let mut terminated = store.get("project_1", "issue_1").unwrap().unwrap();
    terminated.enabled = false;
    terminated.terminated_at = Some("2026-10-16T01:00:00Z".to_string());
    write_json(&enrollment_json_path(&tmp), &terminated).unwrap();

    // 同一 terminated 绑定：清旧 plan 事实＋写 termination 清理失败均放行。
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "plan-q1")
        .unwrap();
    let mut cleanup = attention_fact("terminate:cleanup", "2026-10-16T02:00:00Z");
    cleanup.kind = AutomationAttentionKind::AutomationFailed;
    cleanup.stage = AutomationAttentionStage::Termination;
    store
        .upsert_automation_attention_fact("project_1", "issue_1", &binding, cleanup.clone())
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(saved.attention_facts, vec![cleanup.clone()]);
    assert_eq!(saved.terminated_at.as_deref(), Some("2026-10-16T01:00:00Z"));

    // 重清成功即消失（幂等）。
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "terminate:cleanup")
        .unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(saved.attention_facts.is_empty());
}

/// 红测②：旧 JSON 无 attention_facts/terminated_at 键（及显式空数组/
/// null、pre-C1 无 binding_history）均可读；旧代无绑定事实写 fail-closed。
#[test]
fn issue_automation_store_reads_legacy_json_without_attention_fields() {
    let (tmp, store, binding, _revision) = bound_enrollment_with_binding();
    let path = enrollment_json_path(&tmp);
    let current = store.get("project_1", "issue_1").unwrap().unwrap();

    // 批 2 前旧 wire：无 attention_facts/terminated_at 键。
    let mut legacy = serde_json::to_value(&current).unwrap();
    let object = legacy.as_object_mut().unwrap();
    object.remove("attention_facts");
    object.remove("terminated_at");
    write_json(&path, &legacy).unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(saved.attention_facts.is_empty());
    assert_eq!(saved.terminated_at, None);

    // 显式空数组/null 同样读为空/None。
    let mut explicit = serde_json::to_value(&current).unwrap();
    let object = explicit.as_object_mut().unwrap();
    object.insert("attention_facts".to_string(), serde_json::json!([]));
    object.insert("terminated_at".to_string(), serde_json::Value::Null);
    write_json(&path, &explicit).unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(saved.attention_facts.is_empty());
    assert_eq!(saved.terminated_at, None);

    // 更旧的 pre-C1 wire（无 binding_history）：读仍成功；事实写按
    // InvalidScope fail-closed（旧代无绑定不驱动自动链）。
    let mut pre_c1 = serde_json::to_value(&current).unwrap();
    pre_c1.as_object_mut().unwrap().remove("binding_history");
    write_json(&path, &pre_c1).unwrap();
    let saved = store.get("project_1", "issue_1").unwrap().unwrap();
    assert!(saved.binding_history.is_none());
    assert!(saved.attention_facts.is_empty());
    assert_eq!(saved.terminated_at, None);
    assert_invalid_scope(
        store
            .upsert_automation_attention_fact(
                "project_1",
                "issue_1",
                &binding,
                attention_fact("k9", "t"),
            )
            .unwrap_err(),
    );
}

/// 红测④：损坏 JSON fail-closed，读错误原样上抛（写/清两入口）。
#[test]
fn issue_automation_store_attention_fact_corrupt_json_propagates() {
    let (tmp, store, binding, _revision) = bound_enrollment_with_binding();
    std::fs::write(&enrollment_json_path(&tmp), "{ not json").unwrap();
    assert!(matches!(
        store
            .upsert_automation_attention_fact(
                "project_1",
                "issue_1",
                &binding,
                attention_fact("k", "t"),
            )
            .unwrap_err(),
        EnrollmentError::Store(_)
    ));
    assert!(matches!(
        store
            .resolve_automation_attention_fact("project_1", "issue_1", &binding, "k")
            .unwrap_err(),
        EnrollmentError::Store(_)
    ));
}

// ---------------------------------------------------------------------------
// 批 2 T1（REQ-WIGA-09）：终止 CAS、命令账本与不可恢复准入。
// ---------------------------------------------------------------------------

/// 红测①：terminate CAS——锁内 terminated_at＋enabled=false、revision+1、
/// 命令账本 Accepted；同 command 同 payload 重入 Replayed（零写入、
/// terminated_at 冻结）；异 payload/漂移 revision 拒绝且零副作用。
#[test]
fn issue_automation_store_terminate_plan_cas_and_idempotent_replay() {
    let (_tmp, store, _binding, revision) = bound_enrollment_with_binding();
    let request = EnrollmentTerminationRequest {
        command_id: "terminate-command-1".to_string(),
        expected_enrollment_id: store
            .get("project_1", "issue_1")
            .unwrap()
            .expect("enrollment exists")
            .enrollment_id,
        expected_policy_revision: revision,
        reason: "user requested termination".to_string(),
    };
    let result = store
        .terminate_plan("project_1", "issue_1", request.clone())
        .expect("first terminate applies");
    assert_eq!(result.state, OperationState::Accepted);
    let terminated = result.enrollment;
    assert!(!terminated.enabled, "terminated must disable enrollment");
    let terminated_at = terminated.terminated_at.clone().expect("terminated_at set");
    assert_eq!(result.terminated_at, terminated_at);
    assert_eq!(terminated.policy_revision, revision + 1);
    assert!(
        terminated
            .command_ledger
            .iter()
            .any(|entry| entry.command_id == "terminate-command-1"
                && entry.state == OperationState::Accepted),
        "termination command must be ledgered"
    );

    // 同命令同 payload 重入：Replayed，零写入（terminated_at/revision 冻结）。
    let replay = store
        .terminate_plan("project_1", "issue_1", request)
        .expect("same-command replay is idempotent");
    assert_eq!(replay.state, OperationState::Replayed);
    assert_eq!(replay.terminated_at, terminated_at);
    assert_eq!(replay.enrollment.policy_revision, revision + 1);
    assert_eq!(replay.enrollment, terminated);

    // 异 payload（同 command_id）：fail-closed Conflict。
    let divergent = EnrollmentTerminationRequest {
        command_id: "terminate-command-1".to_string(),
        expected_enrollment_id: terminated.enrollment_id.clone(),
        expected_policy_revision: revision + 1,
        reason: "divergent payload".to_string(),
    };
    assert_conflict(
        store
            .terminate_plan("project_1", "issue_1", divergent)
            .unwrap_err(),
        revision + 1,
    );

    // 预期 revision 漂移：Conflict，零副作用。
    let stale = EnrollmentTerminationRequest {
        command_id: "terminate-command-2".to_string(),
        expected_enrollment_id: terminated.enrollment_id.clone(),
        expected_policy_revision: revision,
        reason: "stale revision".to_string(),
    };
    assert_conflict(
        store
            .terminate_plan("project_1", "issue_1", stale)
            .unwrap_err(),
        revision + 1,
    );
    let after = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(after, terminated, "rejected terminate must not mutate");
}

/// 红测②：terminated 不可恢复——Enable（同/异 payload）、Disable、rebind、
/// bind_plan 全拒且零副作用；异 enrollment_id 终止也拒。N0 口径对齐：同一
/// terminated 绑定仍可写/清终止收尾事实（不被准入拒绝阻断）。
#[test]
fn issue_automation_store_terminated_enrollment_is_irrecoverable() {
    let (_tmp, store, binding, revision) = bound_enrollment_with_binding();
    let enrollment = store.get("project_1", "issue_1").unwrap().unwrap();
    store
        .terminate_plan(
            "project_1",
            "issue_1",
            EnrollmentTerminationRequest {
                command_id: "terminate-command-1".to_string(),
                expected_enrollment_id: enrollment.enrollment_id.clone(),
                expected_policy_revision: revision,
                reason: "user requested termination".to_string(),
            },
        )
        .unwrap();
    let terminated = store.get("project_1", "issue_1").unwrap().unwrap();

    // Enable（与原 payload 完全一致也不可重开）→ Conflict 零副作用。
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                enable("human-choice-1", logical_repo(1)),
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // Disable 也拒（terminated 即终态，不再接受任何写命令）。
    assert_conflict(
        store
            .compare_and_set(
                "project_1",
                "issue_1",
                Some(terminated.policy_revision),
                EnrollmentWriteCommand::Disable,
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // rebind → Conflict。
    assert_conflict(
        store
            .rebind(
                "project_1",
                "issue_1",
                rebind_request(
                    "rebind-command-9",
                    terminated.policy_revision,
                    binding.binding_version,
                    "plan_0002",
                    "session_0002",
                    target_logical_at(logical_repo(1)),
                ),
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // bind_plan → Conflict。
    assert_conflict(
        store
            .bind_plan(
                "project_1",
                "issue_1",
                terminated.policy_revision,
                "plan_0003",
                "session_0003",
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // 异 enrollment_id 的终止命令 → Conflict（身份互证）。
    assert_conflict(
        store
            .terminate_plan(
                "project_1",
                "issue_1",
                EnrollmentTerminationRequest {
                    command_id: "terminate-command-8".to_string(),
                    expected_enrollment_id: "another_enrollment".to_string(),
                    expected_policy_revision: terminated.policy_revision,
                    reason: "wrong identity".to_string(),
                },
            )
            .unwrap_err(),
        terminated.policy_revision,
    );
    // 零副作用：durable 记录未被上述任一命令改写。
    assert_eq!(
        store.get("project_1", "issue_1").unwrap().unwrap(),
        terminated
    );

    // N0 口径对齐：同一 terminated 绑定放行写/清终止收尾事实。
    store
        .upsert_automation_attention_fact(
            "project_1",
            "issue_1",
            &binding,
            AutomationAttentionFact {
                key: "termination_cleanup".to_string(),
                kind: AutomationAttentionKind::AutomationFailed,
                stage: AutomationAttentionStage::Termination,
                plan_id: Some("plan_0001".to_string()),
                session_id: Some("session_0001".to_string()),
                attempt_id: None,
                reason_code: "cleanup_failed".to_string(),
                occurred_at: "2026-10-16T00:00:00Z".to_string(),
                choice_id: None,
                gate_id: None,
            },
        )
        .expect("terminated binding may still record termination facts");
    store
        .resolve_automation_attention_fact("project_1", "issue_1", &binding, "termination_cleanup")
        .expect("terminated binding may still resolve facts");
}

/// 红测③：缺失 enrollment 的终止 NotFound；普通 Disable→Enable 重开回归
/// 保持（终止语义不波及非终止路径）。
#[test]
fn issue_automation_store_termination_missing_and_manual_disable_regression() {
    let tmp = tempfile::tempdir().unwrap();
    let store = IssueAutomationStore::new(ProductAppPaths::new(tmp.path()));
    assert!(matches!(
        store
            .terminate_plan(
                "project_1",
                "issue_1",
                EnrollmentTerminationRequest {
                    command_id: "c".to_string(),
                    expected_enrollment_id: "e".to_string(),
                    expected_policy_revision: 1,
                    reason: String::new(),
                },
            )
            .unwrap_err(),
        EnrollmentError::NotFound
    ));
    drop(store);

    // 普通 Disable 后 Enable 重开（无 terminated_at）保持既有行为。
    let (_tmp2, store2, _binding2, revision2) = bound_enrollment_with_binding();
    let disabled = store2
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(revision2),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    assert!(!disabled.enabled);
    assert!(disabled.terminated_at.is_none());
    let reopened = store2
        .compare_and_set(
            "project_1",
            "issue_1",
            Some(disabled.policy_revision),
            enable("human-choice-1", logical_repo(1)),
        )
        .unwrap();
    assert!(reopened.enabled, "plain disable still allows re-enable");
}
include!("target_union_rebind_tests.inc.rs");
