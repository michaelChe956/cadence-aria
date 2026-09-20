use tempfile::TempDir;

use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::lifecycle_store::{AggregateDesignSpecScope, AggregateStorySpecScope};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{
    DesignSpecRecord, LifecycleConfirmationStatus, ProviderName, StorySpecRecord,
    WorkItemRuntimeBinding, WorkspaceType,
};
use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason};

use super::*;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const REPOSITORY_ID: &str = "repository_0001";

fn setup() -> (TempDir, LifecycleStore) {
    let tmp = TempDir::new().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    (tmp, store)
}

fn create_session(
    store: &LifecycleStore,
    entity_id: &str,
    workspace_type: WorkspaceType,
) -> crate::product::models::WorkspaceSessionRecord {
    store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: entity_id.to_string(),
            workspace_type,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
        })
        .unwrap()
}

#[test]
fn human_gate_reservation_cas_writes_turn_budget_and_provider_key_atomically() {
    let (_tmp, store) = setup();
    let mut session = create_session(&store, "work_item_plan_0001", WorkspaceType::WorkItemPlan);
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
    });
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).unwrap();

    let turn = crate::product::models::HumanGateTurn {
        turn_id: "turn_0001".to_string(),
        session_id: session.id.clone(),
        command_id: "command_0001".to_string(),
        feedback_text: "please revise".to_string(),
        status: crate::product::models::HumanGateTurnStatus::Reserved,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    let reservation = crate::product::models::HumanGateReservation {
        command_id: turn.command_id.clone(),
        turn_id: turn.turn_id.clone(),
        provider_start_idempotency_key: "human_gate_start_0001".to_string(),
        reserved_at: "2026-08-31T00:00:00Z".to_string(),
    };

    let (saved, saved_turn) = store
        .compare_and_reserve_human_gate_turn(&session, turn.clone(), reservation.clone())
        .unwrap();
    assert_eq!(
        saved
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        1
    );
    assert_eq!(saved.human_gate_reservation, Some(reservation.clone()));
    let mut altered = turn.clone();
    altered.source_hash = "b".repeat(64);
    assert!(matches!(
        store.update_human_gate_turn(&saved, altered),
        Err(crate::product::json_store::ProductStoreError::Conflict {
            kind: "human_gate_turn",
            ..
        })
    ));
    let session_bytes = std::fs::read(&session_path).unwrap();
    let turn_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(&session.id)
        .join("human-gate-turns")
        .join("turn_0001.json");
    let turn_bytes = std::fs::read(&turn_path).unwrap();
    assert!(
        session_bytes
            .windows(b"human_gate_reservation".len())
            .any(|bytes| bytes == b"human_gate_reservation")
    );
    assert!(
        turn_bytes
            .windows(b"reserved".len())
            .any(|bytes| bytes == b"reserved")
    );
    assert_eq!(
        store.get_human_gate_turn(&session.id, "turn_0001").unwrap(),
        saved_turn
    );
    assert_eq!(
        store
            .get_human_gate_turn_by_command_id(&session.id, "command_0001")
            .unwrap(),
        Some(saved_turn.clone())
    );
    assert!(
        saved
            .provider_start_ledger
            .iter()
            .any(|entry| entry.provider_start_idempotency_key == "human_gate_start_0001")
    );

    let (replayed, replayed_turn) = store
        .compare_and_reserve_human_gate_turn(&saved, turn, reservation)
        .unwrap();
    assert_eq!(replayed, saved);
    assert_eq!(replayed_turn, saved_turn);
    assert_eq!(
        replayed
            .human_gate_snapshot
            .as_ref()
            .unwrap()
            .manual_repairs_remaining,
        1
    );

    let duplicate_turn = crate::product::models::HumanGateTurn {
        turn_id: "turn_0002".to_string(),
        ..saved_turn.clone()
    };
    let duplicate_path = turn_path.with_file_name("turn_0002.json");
    write_json(&duplicate_path, &duplicate_turn).unwrap();
    assert!(matches!(
        store.get_human_gate_turn_by_command_id(&session.id, "command_0001"),
        Err(ProductStoreError::Conflict {
            kind: "human_gate_turn_command",
            ..
        })
    ));
}

#[test]
fn human_gate_reservation_replay_repairs_torn_turn_file_without_double_debit() {
    let (_tmp, store) = setup();
    let mut session = create_session(&store, "work_item_plan_torn", WorkspaceType::WorkItemPlan);
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
    });
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    let reservation = crate::product::models::HumanGateReservation {
        command_id: "command_torn".to_string(),
        turn_id: "turn_torn".to_string(),
        provider_start_idempotency_key: "human_gate_start_torn".to_string(),
        reserved_at: "2026-08-31T00:00:00Z".to_string(),
    };
    session
        .human_gate_snapshot
        .as_mut()
        .unwrap()
        .manual_repairs_remaining = 0;
    session.human_gate_reservation = Some(reservation.clone());
    session.provider_start_ledger.push(
        crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
            provider_start_idempotency_key: reservation.provider_start_idempotency_key.clone(),
            started: true,
            provider: None,
            started_at: None,
        },
    );
    write_json(&session_path, &session).unwrap();

    let turn = crate::product::models::HumanGateTurn {
        turn_id: reservation.turn_id.clone(),
        session_id: session.id.clone(),
        command_id: reservation.command_id.clone(),
        feedback_text: "recover torn reservation".to_string(),
        status: crate::product::models::HumanGateTurnStatus::Reserved,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };

    let (recovered, recovered_turn) = store
        .compare_and_reserve_human_gate_turn(&session, turn.clone(), reservation)
        .unwrap();
    assert_eq!(recovered, session);
    assert_eq!(recovered_turn, turn);
    assert_eq!(
        store.get_human_gate_turn(&session.id, "turn_torn").unwrap(),
        turn
    );
}

#[test]
fn new_session_defaults_permission_modes_to_auto() {
    let (_tmp, store) = setup();
    let record = create_session(&store, "story_spec_permissions", WorkspaceType::Story);

    assert_eq!(
        record.permission_modes,
        crate::product::models::WorkspaceRolePermissionModes::default()
    );
}

fn runtime_binding() -> WorkItemRuntimeBinding {
    WorkItemRuntimeBinding {
        plan_id: "work_item_plan_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        logical_work_item_id: "wi_library_export".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        projection_bundle_id: "work_item_projection_bundle_0001".to_string(),
        verification_plan_revision_id: "verification_plan_revision_0001".to_string(),
        canonical_contract_hash: "sha256:contract".to_string(),
        projection_compiler_version: "projection-compiler-v1".to_string(),
        human_projection_hash: "sha256:human".to_string(),
        coder_projection_hash: "sha256:coder".to_string(),
        reviewer_projection_hash: "sha256:reviewer".to_string(),
    }
}

#[test]
fn ensure_work_item_runtime_binding_persists_and_replays_the_same_binding() {
    let (_tmp, store) = setup();
    let session = create_session(&store, "wi_library_export", WorkspaceType::WorkItem);
    let binding = runtime_binding();

    let first = store
        .ensure_work_item_runtime_binding(&session.id, &binding)
        .unwrap();
    let replay = store
        .ensure_work_item_runtime_binding(&session.id, &binding)
        .unwrap();

    assert_eq!(first.work_item_runtime_binding.as_ref(), Some(&binding));
    assert_eq!(replay, first);
}

#[test]
fn ensure_work_item_runtime_binding_rejects_a_different_binding() {
    let (_tmp, store) = setup();
    let session = create_session(&store, "wi_library_export", WorkspaceType::WorkItem);
    let binding = runtime_binding();
    store
        .ensure_work_item_runtime_binding(&session.id, &binding)
        .unwrap();
    let mut different = binding;
    different.work_item_revision_id = "work_item_revision_0002".to_string();

    let error = store
        .ensure_work_item_runtime_binding(&session.id, &different)
        .unwrap_err();

    assert!(matches!(
        error,
        ProductStoreError::IdentityMismatch {
            kind: "work_item_runtime_binding",
            ..
        }
    ));
}

#[test]
fn ensure_work_item_runtime_binding_rejects_story_and_design_sessions() {
    let (_tmp, store) = setup();
    let binding = runtime_binding();

    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
        let session = create_session(&store, "shared_entity", workspace_type);
        let error = store
            .ensure_work_item_runtime_binding(&session.id, &binding)
            .unwrap_err();

        assert!(matches!(
            error,
            ProductStoreError::IdentityMismatch {
                kind: "workspace_session_type",
                ..
            }
        ));
    }
}

#[test]
fn ensure_version_repairs_current_version_without_appending_duplicate() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "Recover current version".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    let input = AppendSpecVersionInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        entity_id: story.id.clone(),
        markdown: "# Story Spec\n\nRecovered markdown".to_string(),
        provider_run_refs: vec![],
        review_refs: vec![],
        confirmed_by: None,
    };
    store.append_version(input.clone()).unwrap();

    let story_path = store
        .story_specs_root(PROJECT_ID, ISSUE_ID)
        .join(format!("{}.json", story.id));
    let mut stale_story: StorySpecRecord = read_json(&story_path).unwrap();
    stale_story.current_version = None;
    write_json(&story_path, &stale_story).unwrap();

    let ensured = store.ensure_version(input).unwrap();

    assert_eq!(ensured.version, 1);
    assert_eq!(
        store
            .list_versions(PROJECT_ID, ISSUE_ID, &story.id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .list_story_specs(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .into_iter()
            .find(|record| record.id == story.id)
            .and_then(|record| record.current_version),
        Some(1)
    );
}

#[test]
fn delete_story_spec_removes_record_versions_session_and_timeline() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "Session expired story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    store
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: story.id.clone(),
            markdown: "story markdown".to_string(),
            provider_run_refs: vec![],
            review_refs: vec![],
            confirmed_by: None,
        })
        .unwrap();
    let session = create_session(&store, &story.id, WorkspaceType::Story);
    store.save_timeline_nodes(&session.id, &[]).unwrap();
    let versions_root = store.versions_root(PROJECT_ID, ISSUE_ID, &story.id);
    let timeline_root = store
        .workspace_timeline_root_for_session(&session.id)
        .unwrap();

    store
        .delete_story_spec(PROJECT_ID, ISSUE_ID, &story.id)
        .unwrap();

    assert!(
        store
            .list_story_specs(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_versions(PROJECT_ID, ISSUE_ID, &story.id)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(!versions_root.exists());
    assert!(!timeline_root.exists());
}

#[test]
fn delete_design_spec_removes_record_versions_session_and_timeline() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "Frontend design".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    store
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: design.id.clone(),
            markdown: "design markdown".to_string(),
            provider_run_refs: vec![],
            review_refs: vec![],
            confirmed_by: None,
        })
        .unwrap();
    let session = create_session(&store, &design.id, WorkspaceType::Design);
    store.save_timeline_nodes(&session.id, &[]).unwrap();
    let versions_root = store.versions_root(PROJECT_ID, ISSUE_ID, &design.id);
    let timeline_root = store
        .workspace_timeline_root_for_session(&session.id)
        .unwrap();

    store
        .delete_design_spec(PROJECT_ID, ISSUE_ID, &design.id)
        .unwrap();

    assert!(
        store
            .list_design_specs(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_versions(PROJECT_ID, ISSUE_ID, &design.id)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(!versions_root.exists());
    assert!(!timeline_root.exists());
}

#[test]
fn delete_work_item_removes_record_session_and_timeline() {
    let (_tmp, store) = setup();
    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "Implement prompt component".to_string(),
            ..Default::default()
        })
        .unwrap();
    let session = create_session(&store, &work_item.id, WorkspaceType::WorkItem);
    store.save_timeline_nodes(&session.id, &[]).unwrap();
    let timeline_root = store
        .workspace_timeline_root_for_session(&session.id)
        .unwrap();

    store
        .delete_work_item(PROJECT_ID, ISSUE_ID, &work_item.id)
        .unwrap();

    assert!(
        store
            .list_work_items(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(!timeline_root.exists());
}

/// change `remove-work-item-handoff` 工作包 1.12：
/// 移除交接摘要引用后，work item 的完成 commit 记录必须仍可写入并读取。
/// 原摘要更新函数曾是 `completion_commit` 的唯一写入点，
/// 不能随交接摘要一并删除。
#[test]
fn work_item_completion_commit_is_persisted_and_readable() {
    let (_tmp, store) = setup();
    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "Work item with completion commit".to_string(),
            ..Default::default()
        })
        .unwrap();

    let updated = store
        .update_work_item_completion_commit(
            PROJECT_ID,
            ISSUE_ID,
            &work_item.id,
            Some("abc1234".to_string()),
        )
        .expect("write completion commit");
    assert_eq!(updated.completion_commit.as_deref(), Some("abc1234"));

    let reloaded = store
        .list_work_items(PROJECT_ID, ISSUE_ID)
        .expect("list work items")
        .into_iter()
        .find(|item| item.id == work_item.id)
        .expect("work item exists");
    assert_eq!(
        reloaded.completion_commit.as_deref(),
        Some("abc1234"),
        "完成 commit 必须持久化并可读"
    );
}

#[test]
fn delete_issue_shared_worktree_removes_json_and_lock() {
    let (tmp, store) = setup();
    let root = tmp
        .path()
        .join(".aria")
        .join("projects")
        .join(PROJECT_ID)
        .join("issues")
        .join(ISSUE_ID);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("issue-shared-worktree.json"), "{}").unwrap();
    std::fs::write(root.join(".issue-shared-worktree.json.lock"), "{}").unwrap();

    store
        .delete_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert!(
        !root.join("issue-shared-worktree.json").exists(),
        "shared-worktree json 应被删除"
    );
    assert!(
        !root.join(".issue-shared-worktree.json.lock").exists(),
        "shared-worktree lock 应被删除"
    );
}

#[test]
fn delete_issue_shared_worktree_succeeds_when_absent() {
    let (_tmp, store) = setup();
    // 不播种任何产物；NotFound 视为成功
    store
        .delete_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
        .unwrap();
}

#[test]
fn lifecycle_creation_uses_ids_above_deleted_middle_records() {
    let (_tmp, store) = setup();
    let create_story = |title: &str| {
        store
            .create_story_spec(CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                title: title.to_string(),
                aggregate_codebase: None,
            })
            .unwrap()
    };
    let _first = create_story("First");
    let middle = create_story("Middle");
    let _last = create_story("Last");
    store
        .delete_story_spec(PROJECT_ID, ISSUE_ID, &middle.id)
        .unwrap();

    assert_eq!(create_story("Replacement").id, "story_spec_0004");
}

// F-19：provider start 诊断登记（story/design 等 legacy 流）——条目必须带
// provider/时间戳且同 key 幂等；缺省两字段条目与旧 JSON 双向兼容。
#[test]
fn claim_provider_start_with_details_registers_provider_and_timestamp_idempotently() {
    let (_tmp, store) = setup();
    let session = create_session(&store, "story_spec_0001", WorkspaceType::Story);
    let started_at = "2026-09-20T02:00:00+00:00".to_string();

    let first = store
        .claim_provider_start_with_details(
            &session.id,
            "workspace_author:session:0",
            Some("codex"),
            Some(started_at.clone()),
        )
        .expect("claim provider start");
    assert!(first, "首次登记必须成功");
    let replay = store
        .claim_provider_start_with_details(
            &session.id,
            "workspace_author:session:0",
            Some("codex"),
            Some(started_at.clone()),
        )
        .expect("replay claim");
    assert!(!replay, "同 key 重放不得二次登记");

    let reloaded = store.get_workspace_session(&session.id).expect("reload");
    assert_eq!(reloaded.provider_start_ledger.len(), 1);
    let entry = &reloaded.provider_start_ledger[0];
    assert_eq!(
        entry.provider_start_idempotency_key,
        "workspace_author:session:0"
    );
    assert!(entry.started);
    assert_eq!(entry.provider.as_deref(), Some("codex"));
    assert_eq!(entry.started_at.as_deref(), Some(started_at.as_str()));

    // 旧两字段 ledger JSON 必须继续可读（wire 兼容：缺省字段反序列化为 None）。
    let legacy_entry: crate::product::work_item_plan_policy::ProviderStartLedgerEntry =
        serde_json::from_str(r#"{"provider_start_idempotency_key":"legacy:1","started":true}"#)
            .expect("legacy two-field entry deserializes");
    assert_eq!(legacy_entry.provider, None);
    assert_eq!(legacy_entry.started_at, None);
}

// Task 9 三元键 shared worktree 回归测试拆分到独立文件，经 include! 引入（large_file_guard 1200 行红线）。
include!("tests/task9_repo_worktree.rs");
include!("tests/human_gate_recovery.rs");
include!("tests/human_gate_snapshot_cleanup.rs");
include!("tests/human_gate_close_promotion.rs");
// Task 3.2（REQ-ENV-09）：durable tool-policy-run-audit 分区测试。
include!("tests/tool_policy_audit.rs");
// 聚合视野 StorySpec/DesignSpec scope 校验与 confirm gate 测试拆分到独立文件，经 include! 引入（large_file_guard 1200 行红线）。
include!("tests/aggregate_spec_scope.rs");
