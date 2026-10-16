use std::os::unix::fs::MetadataExt;
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
            reviewer_provider: Some(ProviderName::ClaudeCode),

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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
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
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
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
    // v37 issue_0001 数据面：session json / timeline / versions 被级联删除，但
    // `workspace-sessions/<sid>/`（checkpoints）与 `.<sid>.json.lock` 残留——重新
    // 生成复用同 id session 时旧 checkpoint 被新会话读到。
    let sessions_root = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions");
    let checkpoint_dir = sessions_root.join(&session.id).join("checkpoints");
    std::fs::create_dir_all(&checkpoint_dir).unwrap();
    std::fs::write(
        checkpoint_dir.join("cp_001.json"),
        r#"{"id":"cp_001","session_id":"session","message_index":7,"artifact_snapshot":null,"stage":"author_confirm","created_at":"2026-09-21T12:22:09Z"}"#,
    )
    .unwrap();
    std::fs::write(sessions_root.join(format!(".{}.json.lock", session.id)), "").unwrap();
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
    assert!(!sessions_root.join(&session.id).exists());
    assert!(
        !sessions_root
            .join(format!(".{}.json.lock", session.id))
            .exists()
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

// SG4a.0（plan v1.1 §0.6.1）：durable store 兼容——旧 session JSON 缺
// structural_governance_state 经 store 读写保持 None/不拒收；含聚合状态
// 的 JSON 落盘后 store 读出不丢；ledger 旧 started=true 不在 store 路径
// 冒充实际 start（聚合 launch 证据读 Unknown）。
#[test]
fn sg_04a_0_store_session_json_preserves_governance_state_compat() {
    let (_tmp, store) = setup();
    let session = create_session(&store, "work_item_plan_0001", WorkspaceType::WorkItemPlan);
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));

    // ① 手写旧 JSON（缺聚合字段、ledger started=true）→ store 读出成功且
    //    不凭空长出聚合状态。
    let mut old = serde_json::to_value(&session).unwrap();
    old["provider_start_ledger"] = serde_json::json!([
        {"provider_start_idempotency_key": "workspace_author:session:0", "started": true}
    ]);
    old.as_object_mut()
        .unwrap()
        .remove("structural_governance_state");
    write_json(&session_path, &old).unwrap();
    let reloaded = store.get_workspace_session(&session.id).unwrap();
    let encoded = serde_json::to_value(&reloaded).unwrap();
    assert!(
        encoded.get("structural_governance_state").is_none(),
        "旧 durable 记录经 store 读写不得凭空长出聚合状态"
    );

    // ② 含聚合状态的 JSON 落盘 → store 读出保留；缺字段 launch 证据读
    //    Unknown，不被 ledger 旧 started=true 抬成实际 start。
    let mut with_state = old;
    with_state["structural_governance_state"] =
        serde_json::json!({"user_budget_total": 3, "launch_evidence": {}});
    write_json(&session_path, &with_state).unwrap();
    let reloaded = store.get_workspace_session(&session.id).unwrap();
    let encoded = serde_json::to_value(&reloaded).unwrap();
    let state = encoded["structural_governance_state"]
        .as_object()
        .expect("聚合状态经 store 读写必须保留");
    assert_eq!(
        state
            .get("launch_evidence")
            .and_then(|value| value.get("launch_state")),
        Some(&serde_json::json!("unknown")),
        "旧 started 布尔不得在 store 路径冒充实际 start"
    );
    assert_eq!(state.get("user_budget_total"), Some(&serde_json::json!(3)));
}

// ==== cascade_d1_3a（fix-cascade-deletion Task D1.3a：生命周期/session 准入与身份不串代）====
fn d1_3a_node_detail(session_id: &str) -> crate::product::models::NodeDetail {
    crate::product::models::NodeDetail {
        node_id: "node_0001".to_string(),
        session_id: session_id.to_string(),
        node_type: crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun,
        status: crate::web::workspace_ws_types::TimelineNodeStatus::Active,
        agent_role: None,
        provider: None,
        prompt: None,
        messages: Vec::new(),
        streaming_content: String::new(),
        execution_events: Vec::new(),
        permission_events: Vec::new(),
        verdict: None,
        artifact_ref: None,
        is_revision: false,
        revision_feedback: None,
        base_artifact_ref: None,
        started_at: "2026-10-13T00:00:00Z".to_string(),
        ended_at: None,
    }
}

fn d1_3a_artifact_version() -> crate::web::workspace_ws_types::ArtifactVersion {
    crate::web::workspace_ws_types::ArtifactVersion {
        version: 1,
        payload: crate::web::workspace_ws_types::ArtifactPayload::Markdown {
            markdown: "# artifact".to_string(),
            diff: None,
        },
        generated_by: crate::product::models::ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        source_node_id: "node_0001".to_string(),
    }
}

use crate::product::entity_deletion_store::{
    DeletionEntityKind, EntityDeletionStore, EntityScope, issue_entity_operations_root,
};
use crate::product::entity_operation_store::{EntityDeletionLease, EntityOperationStore};

fn d1_3a_plan_options() -> crate::product::models::IssueWorkItemPlanOptions {
    crate::product::models::IssueWorkItemPlanOptions {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
    }
}

fn d1_3a_scope(kind: DeletionEntityKind, entity_id: &str) -> EntityScope {
    EntityScope::new(PROJECT_ID, ISSUE_ID, kind, entity_id)
}

fn d1_3a_begin_purge(store: &LifecycleStore, scope: &EntityScope) -> EntityDeletionLease {
    let lease = EntityOperationStore::new(store.app_paths())
        .try_acquire_delete(scope)
        .expect("acquire delete lease");
    EntityDeletionStore::new(store.app_paths())
        .begin(&lease)
        .expect("begin purging tombstone");
    lease
}

fn d1_3a_complete_purge(store: &LifecycleStore, lease: EntityDeletionLease) {
    EntityDeletionStore::new(store.app_paths())
        .complete(&lease)
        .expect("complete deletion tombstone");
}

fn d1_3a_story(store: &LifecycleStore) -> crate::product::models::StorySpecRecord {
    store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap()
}

fn d1_3a_plan(
    store: &LifecycleStore,
    id: &str,
    story_ids: Vec<String>,
) -> crate::product::models::IssueWorkItemPlan {
    store
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: story_ids,
            source_design_spec_ids: Vec::new(),
            options: d1_3a_plan_options(),
            status: crate::product::models::IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .unwrap()
}

fn d1_3a_sc_session(
    store: &LifecycleStore,
    plan_id: &str,
) -> crate::product::models::WorkspaceSessionRecord {
    store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: plan_id.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: None,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind:
                    crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
                rollout_snapshot: false,
            }),
        })
        .unwrap()
}

fn d1_3a_wi(
    store: &LifecycleStore,
    id: &str,
    plan_id: &str,
) -> crate::product::models::LifecycleWorkItemRecord {
    store
        .create_work_item(CreateWorkItemInput {
            id: Some(id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "wi".to_string(),
            work_item_set_id: None,
            source_work_item_plan_id: Some(plan_id.to_string()),
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            kind: crate::product::models::WorkItemKind::Backend,
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: Default::default(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            plan_status: crate::product::models::WorkItemPlanStatus::Draft,
        })
        .unwrap()
}

#[test]
fn cascade_d1_3a_spec_version_and_review_round_late_writes_rejected() {
    let (_tmp, store) = setup();
    let story = d1_3a_story(&store);
    let session = create_session(&store, &story.id, WorkspaceType::Story);

    // 存活实体正常写不受影响。
    store
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: story.id.clone(),
            markdown: "# v1".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .expect("live spec version append must succeed");

    let scope = d1_3a_scope(DeletionEntityKind::Story, &story.id);
    let lease = d1_3a_begin_purge(&store, &scope);
    let spec_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("story-specs")
        .join(format!("{}.json", story.id));
    let spec_bytes = std::fs::read(&spec_path).unwrap();

    let late_version = AppendSpecVersionInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        entity_id: story.id.clone(),
        markdown: "# late".to_string(),
        provider_run_refs: Vec::new(),
        review_refs: Vec::new(),
        confirmed_by: None,
    };
    assert!(matches!(
        store.append_version(late_version.clone()),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &story.id,
            crate::product::models::LifecycleConfirmationStatus::Confirmed
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.append_provider_review_round(AppendProviderReviewRoundInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            session_id: session.id.clone(),
            round_index: 1,
            author_provider: ProviderName::Codex,
            reviewer_provider: None,
            review_result: "ok".to_string(),
            revision_result: "ok".to_string(),
        }),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert_eq!(std::fs::read(&spec_path).unwrap(), spec_bytes);

    // deleted（完成）后仍稳定拒。
    d1_3a_complete_purge(&store, lease);
    assert!(matches!(
        store.append_version(late_version),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}

#[test]
fn cascade_d1_3a_session_message_status_conversations_rejected() {
    let (_tmp, store) = setup();
    d1_3a_plan(&store, "issue_work_item_plan_0001", Vec::new());
    let session = create_session(
        &store,
        "issue_work_item_plan_0001",
        WorkspaceType::WorkItemPlan,
    );
    store
        .append_workspace_message(&session.id, "user".to_string(), "live".to_string())
        .expect("live message append must succeed");

    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001");
    let _lease = d1_3a_begin_purge(&store, &scope);
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    let bytes = std::fs::read(&session_path).unwrap();

    assert!(matches!(
        store.append_workspace_message(&session.id, "user".to_string(), "late".to_string()),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.replace_workspace_messages(&session.id, Vec::new()),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.replace_workspace_provider_conversations(&session.id, Vec::new()),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.update_workspace_session_status(
            &session.id,
            crate::product::models::WorkspaceSessionStatus::Terminated
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.compare_and_update_workspace_session_status(
            &session,
            crate::product::models::WorkspaceSessionStatus::Terminated
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.truncate_workspace_session_messages(
            &session.id,
            0,
            crate::product::models::WorkspaceSessionStatus::Open
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.claim_provider_start(&session.id, "late_start_key"),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert_eq!(std::fs::read(&session_path).unwrap(), bytes);
}

#[test]
fn cascade_d1_3a_timeline_detail_artifact_rejected() {
    let (_tmp, store) = setup();
    d1_3a_plan(&store, "issue_work_item_plan_0001", Vec::new());
    let session = create_session(
        &store,
        "issue_work_item_plan_0001",
        WorkspaceType::WorkItemPlan,
    );
    store
        .save_timeline_nodes(&session.id, &[])
        .expect("live timeline save must succeed");

    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001");
    let _lease = d1_3a_begin_purge(&store, &scope);
    assert!(matches!(
        store.save_timeline_nodes(&session.id, &[]),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.save_node_detail(&session.id, "node_0001", &d1_3a_node_detail(&session.id)),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.append_artifact_version(&session.id, d1_3a_artifact_version()),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.save_artifact_versions(&session.id, &[]),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}

#[test]
fn cascade_d1_3a_sc_cas_late_writes_rejected() {
    let (_tmp, store) = setup();
    d1_3a_plan(&store, "issue_work_item_plan_0001", Vec::new());
    let session = d1_3a_sc_session(&store, "issue_work_item_plan_0001");
    let expected = store.get_workspace_session(&session.id).unwrap();

    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001");
    let _lease = d1_3a_begin_purge(&store, &scope);

    assert!(matches!(
        store.compare_and_save_single_candidate_phase(
            &expected,
            crate::product::models::SingleCandidatePhase::Generate,
            crate::product::models::WorkspaceSessionStatus::Running
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.compare_and_save_single_candidate_generation(
            &expected,
            "project/project_0001/issue/issue_0001/x",
            "project/project_0001/issue/issue_0001/y"
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.reserve_single_candidate_provider_start(&expected, "late_key"),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.compare_and_save_human_gate_close(
            &expected,
            crate::product::models::WorkspaceSessionStatus::Terminated
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    let turn = crate::product::models::HumanGateTurn {
        turn_id: "turn_0001".to_string(),
        session_id: expected.id.clone(),
        command_id: "command_0001".to_string(),
        feedback_text: String::new(),
        status: crate::product::models::HumanGateTurnStatus::Completed,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:00Z".to_string(),
    };
    assert!(matches!(
        store.complete_human_gate_revision(
            &expected,
            turn,
            "project/project_0001/issue/issue_0001/source",
            "project/project_0001/issue/issue_0001/ir",
            "project/project_0001/issue/issue_0001/report",
            &[],
            "project/project_0001/issue/issue_0001/artifact"
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}

#[test]
fn cascade_d1_3a_scope_lock_taken_and_tombstone_arm_covers_find_delete_race() {
    use crate::product::coding_attempt_store::locking::register_exact_lock_attempt_hook;
    use std::time::Duration;

    let (_tmp, store) = setup();
    d1_3a_plan(&store, "issue_work_item_plan_0001", Vec::new());
    let session = create_session(
        &store,
        "issue_work_item_plan_0001",
        WorkspaceType::WorkItemPlan,
    );

    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001");
    let scope_lock_path = issue_entity_operations_root(&store.app_paths(), &scope)
        .unwrap()
        .join("scope-locks")
        .join("plan")
        .join("issue_work_item_plan_0001.lock");
    std::fs::create_dir_all(scope_lock_path.parent().unwrap()).unwrap();
    std::fs::File::create(&scope_lock_path).unwrap();
    let (_hook, attempts) = register_exact_lock_attempt_hook(&scope_lock_path);

    // 存活写：scope 锁真实被取（准入接线存在，非旁路）。
    store
        .append_workspace_message(&session.id, "user".to_string(), "live".to_string())
        .expect("live write must succeed");
    attempts
        .recv_timeout(Duration::from_secs(5))
        .expect("session writer must take the entity scope lock");

    // 「先 find path 后删除再 write」竞态：即便 scope 锁空闲（无争用臂），
    // 锁内墓碑复核臂必须拒。
    let lease = d1_3a_begin_purge(&store, &scope);
    drop(lease);
    assert!(matches!(
        store.append_workspace_message(&session.id, "user".to_string(), "late".to_string()),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}

#[test]
fn cascade_d1_3a_session_and_entity_ids_never_reused() {
    let (_tmp, store) = setup();
    let story = d1_3a_story(&store);
    d1_3a_plan(&store, "issue_work_item_plan_0001", Vec::new());
    let first_session = create_session(
        &store,
        "issue_work_item_plan_0001",
        WorkspaceType::WorkItemPlan,
    );
    // 显式 id 探针：创建即落身份绑定（§0.2 显式身份不跨实体复用）。
    let probe_id = "workspace_session_d1_3a_probe".to_string();
    store
        .create_workspace_session_bound(
            CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: "issue_work_item_plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            probe_id.clone(),
        )
        .unwrap();

    // 旧删除路径清理文件 + 删除事实完成。
    store
        .delete_issue_work_item_plan(PROJECT_ID, ISSUE_ID, "issue_work_item_plan_0001")
        .unwrap();
    store
        .delete_story_spec(PROJECT_ID, ISSUE_ID, &story.id)
        .unwrap();
    let plan_lease = d1_3a_begin_purge(
        &store,
        &d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001"),
    );
    d1_3a_complete_purge(&store, plan_lease);
    let story_lease = d1_3a_begin_purge(&store, &d1_3a_scope(DeletionEntityKind::Story, &story.id));
    d1_3a_complete_purge(&store, story_lease);

    // 新 session/实体 ID 不复用已删旧 ID。
    d1_3a_plan(&store, "issue_work_item_plan_0002", Vec::new());
    let second_session = create_session(
        &store,
        "issue_work_item_plan_0002",
        WorkspaceType::WorkItemPlan,
    );
    assert_ne!(second_session.id, first_session.id);
    let new_story = d1_3a_story(&store);
    assert_ne!(new_story.id, story.id);

    // 显式 id 跨实体复用被身份绑定拒绝。
    assert!(matches!(
        store.create_workspace_session_bound(
            CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: "issue_work_item_plan_0002".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            probe_id
        ),
        Err(ProductStoreError::Conflict { .. })
    ));
}

#[test]
fn cascade_d1_3a_same_logical_wi_other_plan_preserved_and_late_wi_writes_rejected() {
    let (_tmp, store) = setup();
    d1_3a_plan(&store, "issue_work_item_plan_0001", Vec::new());
    d1_3a_plan(&store, "issue_work_item_plan_0002", Vec::new());
    let deleted_wi = d1_3a_wi(&store, "work_item_0001", "issue_work_item_plan_0001");
    let kept_wi = d1_3a_wi(&store, "work_item_0002", "issue_work_item_plan_0002");

    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001");
    let _lease = d1_3a_begin_purge(&store, &scope);

    // 另一 Plan 的同 logical WI 写保留。
    store
        .update_work_item_plan_status(
            PROJECT_ID,
            ISSUE_ID,
            &kept_wi.id,
            crate::product::models::WorkItemPlanStatus::Confirmed,
        )
        .expect("other plan work item writes must stay live");

    // 已删 Plan 名下 WI 的迟到写拒。
    assert!(matches!(
        store.update_work_item_plan_status(
            PROJECT_ID,
            ISSUE_ID,
            &deleted_wi.id,
            crate::product::models::WorkItemPlanStatus::Confirmed
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.update_work_item_execution_status(
            PROJECT_ID,
            ISSUE_ID,
            &deleted_wi.id,
            crate::product::models::WorkItemStatus::Completed
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.update_work_item_completion_commit(PROJECT_ID, ISSUE_ID, &deleted_wi.id, None),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    let mut compiled = deleted_wi.clone();
    compiled.title = "late".to_string();
    assert!(matches!(
        store.upsert_compiled_work_item_record(compiled),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.create_verification_plan(CreateVerificationPlanInput {
            id: None,
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: deleted_wi.id.clone(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: crate::product::models::VerificationScope::Unit,
            commands: Vec::new(),
            manual_checks: Vec::new(),
            required_gates: Vec::new(),
            risk_notes: Vec::new(),
            confidence: crate::product::models::RepositoryProfileConfidence::Low,
            fallback_policy: crate::product::models::VerificationFallbackPolicy::ManualGate,
        }),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}

#[test]
fn cascade_d1_3a_incoming_ref_vs_upstream_delete_single_winner() {
    let (_tmp, store) = setup();
    let story = d1_3a_story(&store);
    let story_scope = d1_3a_scope(DeletionEntityKind::Story, &story.id);

    // 顺序 A：上游删除先行 → 新增 incoming ref（新 Plan 引用）被拒，不留半截记录。
    let lease = d1_3a_begin_purge(&store, &story_scope);
    drop(lease);
    assert!(matches!(
        store.create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: vec![story.id.clone()],
            source_design_spec_ids: Vec::new(),
            options: d1_3a_plan_options(),
            status: crate::product::models::IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        }),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(
        !store
            .app_paths()
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("issue-work-item-plans")
            .join("issue_work_item_plan_0001.json")
            .exists()
    );

    // 顺序 B：写方持上游共享 lease → 关联创建成功、删除被挡（恰一赢家）。
    let story_two = d1_3a_story(&store);
    let story_two_scope = d1_3a_scope(DeletionEntityKind::Story, &story_two.id);
    let run_lease = EntityOperationStore::new(store.app_paths())
        .acquire_run(&story_two_scope)
        .expect("writer holds upstream run lease");
    let plan = store
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0002".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: vec![story_two.id.clone()],
            source_design_spec_ids: Vec::new(),
            options: d1_3a_plan_options(),
            status: crate::product::models::IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("plan creation referencing a live upstream must succeed");
    assert!(
        store
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, &plan.id)
            .is_ok()
    );
    assert!(matches!(
        EntityOperationStore::new(store.app_paths()).try_acquire_delete(&story_two_scope),
        Err(crate::product::entity_operation_store::EntityOperationError::Busy)
    ));
    drop(run_lease);
}

#[test]
fn cascade_d1_3a_plan_root_late_writes_rejected() {
    let (_tmp, store) = setup();
    let plan = d1_3a_plan(&store, "issue_work_item_plan_0001", Vec::new());
    store
        .update_issue_work_item_plan(
            PROJECT_ID,
            ISSUE_ID,
            &plan.id,
            IssueWorkItemPlanUpdate {
                work_item_ids: Vec::new(),
                verification_plan_ids: Vec::new(),
                repository_profile_ref: None,
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            },
        )
        .expect("live plan update must succeed");

    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001");
    let _lease = d1_3a_begin_purge(&store, &scope);
    assert!(matches!(
        store.update_issue_work_item_plan(
            PROJECT_ID,
            ISSUE_ID,
            &plan.id,
            IssueWorkItemPlanUpdate {
                work_item_ids: Vec::new(),
                verification_plan_ids: Vec::new(),
                repository_profile_ref: None,
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            }
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.commit_issue_work_item_plan(
            PROJECT_ID,
            ISSUE_ID,
            &plan.id,
            IssueWorkItemPlanUpdate {
                work_item_ids: Vec::new(),
                verification_plan_ids: Vec::new(),
                repository_profile_ref: None,
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            }
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.restore_issue_work_item_plan_snapshot(PROJECT_ID, ISSUE_ID, &plan.id, &plan),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    // 显式 id 复用已删 Plan 身份同样拒绝。
    assert!(matches!(
        store.ensure_issue_work_item_plan_with_identity(CreateIssueWorkItemPlanInput {
            id: Some("issue_work_item_plan_0001".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: d1_3a_plan_options(),
            status: crate::product::models::IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        }),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}

#[test]
fn cascade_d1_3a_ensure_version_idempotent_write_rejected_when_deleted() {
    use crate::product::coding_attempt_store::locking::register_exact_lock_attempt_hook;
    use std::time::Duration;

    let (_tmp, store) = setup();
    let story = d1_3a_story(&store);
    let idempotent_input = || AppendSpecVersionInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        entity_id: story.id.clone(),
        markdown: "# same".to_string(),
        provider_run_refs: Vec::new(),
        review_refs: Vec::new(),
        confirmed_by: None,
    };
    let version = store
        .append_version(idempotent_input())
        .expect("live append must succeed");

    // 存活幂等重放：scope 锁真实被取（接线存在，非旁路）。
    let scope = d1_3a_scope(DeletionEntityKind::Story, &story.id);
    let scope_lock_path = issue_entity_operations_root(&store.app_paths(), &scope)
        .unwrap()
        .join("scope-locks")
        .join("story")
        .join(format!("{}.lock", story.id));
    std::fs::create_dir_all(scope_lock_path.parent().unwrap()).unwrap();
    std::fs::File::create(&scope_lock_path).unwrap();
    let (_hook, attempts) = register_exact_lock_attempt_hook(&scope_lock_path);
    let replayed = store
        .ensure_version(idempotent_input())
        .expect("live idempotent replay must succeed");
    assert_eq!(replayed.version, version.version);
    attempts
        .recv_timeout(Duration::from_secs(5))
        .expect("ensure_version must take the entity scope lock");

    // purging（墓碑在位、scope 锁空闲）下幂等分支写必须按删除事实拒，
    // 不得把已删 Story/Design 根文件重新落盘（REQ-DEL-03）。
    let lease = d1_3a_begin_purge(&store, &scope);
    drop(lease);
    assert!(matches!(
        store.ensure_version(idempotent_input()),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}
// ==== cascade_d1_3b（fix-cascade-deletion Task D1.3b：门/checkpoint/审计辅助写全闭包）====

#[test]
fn cascade_d1_3b_human_gate_turn_and_structural_late_writes_rejected() {
    let (_tmp, store) = setup();
    d1_3a_plan(&store, "issue_work_item_plan_0001", Vec::new());
    let mut session = create_session(
        &store,
        "issue_work_item_plan_0001",
        WorkspaceType::WorkItemPlan,
    );
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    });
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    write_json(&session_path, &session).unwrap();

    let turn = crate::product::models::HumanGateTurn {
        turn_id: "turn_d13b_0001".to_string(),
        session_id: session.id.clone(),
        command_id: "command_d13b_0001".to_string(),
        feedback_text: "revise".to_string(),
        status: crate::product::models::HumanGateTurnStatus::Reserved,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-10-14T00:00:00Z".to_string(),
        updated_at: "2026-10-14T00:00:00Z".to_string(),
    };
    let reservation = crate::product::models::HumanGateReservation {
        command_id: turn.command_id.clone(),
        turn_id: turn.turn_id.clone(),
        provider_start_idempotency_key: "d13b_start_0001".to_string(),
        reserved_at: "2026-10-14T00:00:00Z".to_string(),
    };

    // 存活实体：原 CAS/预算/nonce 路径零变化（reserve → update Running）。
    let (reserved, _) = store
        .compare_and_reserve_human_gate_turn(&session, turn.clone(), reservation)
        .expect("live gate turn reserve must keep working");
    let mut running = turn.clone();
    running.status = crate::product::models::HumanGateTurnStatus::Running;
    let running_session = store
        .update_human_gate_turn(&reserved, running)
        .expect("live gate turn update must keep working");
    let live_bytes = std::fs::read(&session_path).unwrap();

    // purging：迟到 turn/session 写全部按删除事实拒（原 CAS 检查不动）。
    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001");
    let lease = d1_3a_begin_purge(&store, &scope);

    let mut late_turn = turn.clone();
    late_turn.turn_id = "turn_d13b_0002".to_string();
    late_turn.command_id = "command_d13b_0002".to_string();
    let late_reservation = crate::product::models::HumanGateReservation {
        command_id: late_turn.command_id.clone(),
        turn_id: late_turn.turn_id.clone(),
        provider_start_idempotency_key: "d13b_start_0002".to_string(),
        reserved_at: "2026-10-14T00:00:00Z".to_string(),
    };
    assert!(matches!(
        store.compare_and_reserve_human_gate_turn(&running_session, late_turn, late_reservation),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    let mut late_update = turn;
    late_update.status = crate::product::models::HumanGateTurnStatus::Completed;
    assert!(matches!(
        store.update_human_gate_turn(&running_session, late_update),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.claim_structural_attempt(
            &running_session,
            "turn_d13b_0001",
            &sg_04a_instruction(),
            "prompt_ref_d13b",
            "digest_d13b",
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.settle_structural_user_budget(
            &running_session,
            "turn_d13b_0001",
            sg_04a_receipt(
                "turn_d13b_0001",
                "command_d13b_0001",
                crate::product::models::UserBudgetReceiptKind::ReleasedStructural,
            ),
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert_eq!(std::fs::read(&session_path).unwrap(), live_bytes);

    // deleted（完成）后仍稳定拒。
    d1_3a_complete_purge(&store, lease);
    let mut late_failed = crate::product::models::HumanGateTurn {
        turn_id: "turn_d13b_0001".to_string(),
        session_id: session.id.clone(),
        command_id: "command_d13b_0001".to_string(),
        feedback_text: "revise".to_string(),
        status: crate::product::models::HumanGateTurnStatus::Failed,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: None,
        failure_class: Some(crate::product::models::HumanGateTurnFailureClass::ProviderErr),
        created_at: "2026-10-14T00:00:00Z".to_string(),
        updated_at: "2026-10-14T00:00:00Z".to_string(),
    };
    late_failed.updated_at = "2026-10-14T00:01:00Z".to_string();
    assert!(matches!(
        store.update_human_gate_turn(&running_session, late_failed),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}

#[test]
fn cascade_d1_3b_amendment_gate_reopen_close_late_writes_rejected() {
    let (_tmp, store) = setup();
    d1_3a_plan(&store, "issue_work_item_plan_0002", Vec::new());
    let mut record = d1_3a_sc_session(&store, "issue_work_item_plan_0002");
    record.single_candidate_phase = Some(crate::product::models::SingleCandidatePhase::Completed);
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    });
    record.status = crate::product::models::WorkspaceSessionStatus::Confirmed;
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id));
    write_json(&session_path, &record).unwrap();

    // 存活实体：reopen → close 原 CAS 路径零变化。
    let reopened = store
        .compare_and_reopen_amendment_gate(&record)
        .expect("live amendment reopen must keep working");
    assert_eq!(
        reopened.status,
        crate::product::models::WorkspaceSessionStatus::WaitingForHuman
    );
    let closed = store
        .compare_and_close_reopened_amendment_gate(&reopened)
        .expect("live amendment close must keep working");

    // purging：迟到 reopen 拒（旧码会直接把已删会话改回 WaitingForHuman）。
    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0002");
    let lease = d1_3a_begin_purge(&store, &scope);
    assert!(matches!(
        store.compare_and_reopen_amendment_gate(&closed),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    // 迟到 close：把开态记录放回磁盘（purging 前的真实开态形态），必须拒。
    write_json(&session_path, &reopened).unwrap();
    let reopened_bytes = std::fs::read(&session_path).unwrap();
    assert!(matches!(
        store.compare_and_close_reopened_amendment_gate(&reopened),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert_eq!(std::fs::read(&session_path).unwrap(), reopened_bytes);

    // deleted（完成）后仍稳定拒。
    d1_3a_complete_purge(&store, lease);
    assert!(matches!(
        store.compare_and_reopen_amendment_gate(&closed),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}

#[test]
fn cascade_d1_3b_policy_route_late_writes_rejected() {
    let (_tmp, store) = setup();
    d1_3a_plan(&store, "issue_work_item_plan_0003", Vec::new());
    let session = create_session(
        &store,
        "issue_work_item_plan_0003",
        WorkspaceType::WorkItemPlan,
    );
    let session_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions")
        .join(format!("{}.json", session.id));
    let persist = |status| crate::product::lifecycle_store::PolicyRoutePersist {
        status,
        single_candidate_phase: None,
        run_history: session.run_history.clone(),
        scope: None,
        gate: None,
        diagnostics: Vec::new(),
        repair_reservation: None,
        provider_start_ledger: Vec::new(),
    };

    // 存活实体：policy route CAS 与失败投影原路径零变化。
    let routed = store
        .compare_and_save_policy_route(
            &session,
            persist(crate::product::models::WorkspaceSessionStatus::Running),
        )
        .expect("live policy route CAS must keep working");
    let failed = store
        .fail_workspace_session_policy(&routed.id, &[])
        .expect("live policy failure projection must keep working");
    let live_bytes = std::fs::read(&session_path).unwrap();

    // purging：迟到 route CAS / 失败投影拒且 session 字节不变。
    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0003");
    let lease = d1_3a_begin_purge(&store, &scope);
    assert!(matches!(
        store.compare_and_save_policy_route(
            &failed,
            persist(crate::product::models::WorkspaceSessionStatus::Running),
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(matches!(
        store.fail_workspace_session_policy(&session.id, &[]),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert_eq!(std::fs::read(&session_path).unwrap(), live_bytes);

    // deleted（完成）后仍稳定拒。
    d1_3a_complete_purge(&store, lease);
    assert!(matches!(
        store.fail_workspace_session_policy(&session.id, &[]),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
}

#[test]
fn cascade_d1_3b_usage_diagnostic_late_write_rejected_no_timeline_revival() {
    let (_tmp, store) = setup();
    d1_3a_plan(&store, "issue_work_item_plan_0004", Vec::new());
    let session = create_session(
        &store,
        "issue_work_item_plan_0004",
        WorkspaceType::WorkItemPlan,
    );
    let timeline_dir = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-timelines")
        .join(&session.id);
    // 会话 timeline 根下的 usage 诊断分区文件名（脚本检索契约）。
    let diagnostic_path = timeline_dir.join("usage-diagnostics.jsonl");

    // 存活实体：usage 落盘失败降级写原路径零变化。
    store
        .append_usage_persistence_diagnostic(&session.id, &serde_json::json!({"schema_version": 1}))
        .expect("live usage diagnostic append must keep working");
    assert!(diagnostic_path.exists());

    // purging + 模拟 D1.1 清理掉 timeline 分区后：迟到降级写拒且不复活目录。
    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0004");
    let _lease = d1_3a_begin_purge(&store, &scope);
    std::fs::remove_dir_all(&timeline_dir).unwrap();
    assert!(matches!(
        store.append_usage_persistence_diagnostic(
            &session.id,
            &serde_json::json!({"schema_version": 1})
        ),
        Err(ProductStoreError::Conflict {
            kind: "entity_deleting",
            ..
        })
    ));
    assert!(
        !timeline_dir.exists(),
        "usage fallback 不得在删除后重建会话 timeline 分区"
    );
}

#[tokio::test]
async fn cascade_d1_3b_engine_assistant_checkpoint_late_write_rejected() {
    let checkpoint_root = tempfile::tempdir().expect("checkpoint root");
    let checkpoint_store =
        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            checkpoint_root.path().to_path_buf(),
        ));
    let aria_root = tempfile::tempdir().expect("aria root");
    let store = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
        aria_root.path().join(".aria"),
    ));
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "story d13b checkpoint".to_string(),
            aggregate_codebase: None,
        })
        .expect("story spec");
    let session_record = create_session(&store, &story.id, WorkspaceType::Story);
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let mut engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        checkpoint_store.clone(),
        store.clone(),
        tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(session_record.clone()),
    );
    let checkpoints_dir = checkpoint_root
        .path()
        .join("workspace-sessions")
        .join(&session_record.id)
        .join("checkpoints");

    let compliant = format!(
        "# Story Spec\n\n## 范围\n来源 source id: Issue {ISSUE_ID}；级联删除验收。\n\n\
         ## 用户故事\n作为用户，我希望当前问题被清晰解决。\n\n\
         ## 功能需求\n- [REQ-001] 级联删除验收。\n\n\
         ## 成功标准\n- [AC-001] 迟到 checkpoint 被拒绝。\n\n\
         ## 待确认项\n无。\n\n\
         ## 非功能需求\n无。\n"
    );
    let output = format!("前言\n```artifact\n{compliant}```\n尾随说明\n");

    // 存活实体：真实 assistant 完成路径照常落 checkpoint。
    engine
        .complete_assistant_message("assistant_msg_d13b_1".to_string(), output.clone(), false)
        .await;
    assert!(
        checkpoints_dir.join("cp_001.json").exists(),
        "live assistant completion must keep persisting checkpoints"
    );

    // purging：迟到完成路径的 persistent checkpoint 写被拒且目录不新增。
    let scope = d1_3a_scope(DeletionEntityKind::Story, &story.id);
    let lease = d1_3a_begin_purge(&store, &scope);
    engine
        .complete_assistant_message("assistant_msg_d13b_2".to_string(), output, false)
        .await;
    let mut saw_deleting = false;
    while let Ok(event) = rx.try_recv() {
        if let crate::product::workspace_engine::EngineEvent::Error { message } = event
            && message.contains("entity_deleting")
        {
            saw_deleting = true;
        }
    }
    assert!(
        saw_deleting,
        "迟到 checkpoint 必须以 entity_deleting 冲突上抛引擎事件"
    );
    assert!(
        !checkpoints_dir.join("cp_002.json").exists(),
        "purging 实体的迟到 checkpoint 不得落盘"
    );

    // deleted（完成）后仍稳定拒。
    d1_3a_complete_purge(&store, lease);
    engine
        .complete_assistant_message(
            "assistant_msg_d13b_3".to_string(),
            format!("前言二\n```artifact\n{compliant}```\n尾随说明二\n"),
            false,
        )
        .await;
    assert!(
        !checkpoints_dir.join("cp_002.json").exists()
            && !checkpoints_dir.join("cp_003.json").exists(),
        "deleted 实体的迟到 checkpoint 不得落盘"
    );
}

// ==== cascade_d1_3c（fix-cascade-deletion Task D1.3c：shared-worktree 中央 writer 准入与真实归属）====

/// 直接落盘一条 issue 维 shared worktree 记录（绕过被测入口，构造归因夹具）。
fn d1_3c_seed_issue_worktree(
    store: &LifecycleStore,
    mutate: impl FnOnce(&mut crate::product::models::IssueSharedWorktree),
) -> crate::product::models::IssueSharedWorktree {
    let mut record = crate::product::models::IssueSharedWorktree {
        id: format!("issue_shared_worktree_{PROJECT_ID}_{ISSUE_ID}"),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        repository_id: REPOSITORY_ID.to_string(),
        target_repository_id: None,
        checkout_id: None,
        path_schema_version: 0,
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: std::path::PathBuf::from("/tmp/aria-worktree-d13c"),
        base_branch: "main".to_string(),
        status: crate::product::models::IssueSharedWorktreeStatus::Ready,
        current_active_work_item_id: None,
        current_lock_owner_id: None,
        last_completed_work_item_id: None,
        created_at: "2026-10-14T00:00:00Z".to_string(),
        updated_at: "2026-10-14T00:00:00Z".to_string(),
    };
    mutate(&mut record);
    let path = store.issue_shared_worktree_path(PROJECT_ID, ISSUE_ID);
    write_json(&path, &record).expect("seed issue shared worktree");
    record
}

/// 直接落盘一条仓维 shared worktree 记录（归因夹具）。
fn d1_3c_seed_repo_worktree(
    store: &LifecycleStore,
    repository_id: LogicalRepositoryId,
    mutate: impl FnOnce(&mut crate::product::models::IssueSharedWorktree),
) -> crate::product::models::IssueSharedWorktree {
    let mut record = crate::product::models::IssueSharedWorktree {
        id: format!(
            "repo_shared_worktree_{PROJECT_ID}_{ISSUE_ID}_{}",
            repository_id.0
        ),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        repository_id: repository_id.0.to_string(),
        target_repository_id: Some(repository_id),
        checkout_id: None,
        path_schema_version: 0,
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: std::path::PathBuf::from("/tmp/aria-worktree-d13c"),
        base_branch: "main".to_string(),
        status: crate::product::models::IssueSharedWorktreeStatus::Ready,
        current_active_work_item_id: None,
        current_lock_owner_id: None,
        last_completed_work_item_id: None,
        created_at: "2026-10-14T00:00:00Z".to_string(),
        updated_at: "2026-10-14T00:00:00Z".to_string(),
    };
    mutate(&mut record);
    let path = store.repo_shared_worktree_path(PROJECT_ID, ISSUE_ID, repository_id);
    write_json(&path, &record).expect("seed repo shared worktree");
    record
}

/// 种一条 coding attempt 记录（经真实 create_attempt 落盘后按需改写归属字段）。
fn d1_3c_seed_attempt(
    paths: &crate::product::app_paths::ProductAppPaths,
    attempt_id: &str,
    work_item_id: &str,
    work_item_group_id: Option<&str>,
    target: Option<LogicalRepositoryId>,
) -> crate::product::coding_models::CodingExecutionAttempt {
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: crate::web::workspace_ws_types::ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            target_snapshot: target.map(|logical_id| {
                crate::product::coding_models::AttemptTargetSnapshot {
                    logical_repository_id: logical_id,
                    checkout_id: crate::product::logical_codebase::RepositoryCheckoutId(
                        Uuid::new_v4(),
                    ),
                    physical_repository_id: REPOSITORY_ID.to_string(),
                    canonical_path: std::path::PathBuf::from("/tmp/repository_0001"),
                    git_dir_identity: "sha256:test".to_string(),
                    revision: Some("abcdef".to_string()),
                    policy_digest: "policy_digest".to_string(),
                    membership_revision: 1,
                    captured_at: "2026-10-14T00:00:00Z".to_string(),
                    capture_source: "test".to_string(),
                }
            }),
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let mut patched = attempt.clone();
    patched.id = attempt_id.to_string();
    patched.work_item_group_id = work_item_group_id.map(str::to_string);
    write_json(
        &paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(format!("{attempt_id}.json")),
        &patched,
    )
    .expect("patch attempt");
    patched
}

/// 种一条 group 初始化 journal（最小归属视图：plan/lock WI/lease join 事实）。
fn d1_3c_seed_journal(
    paths: &crate::product::app_paths::ProductAppPaths,
    plan_id: &str,
    lock_work_item_id: &str,
    worktree_lease_id: &str,
) {
    let journal_path = paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("coding-attempts")
        .join("group-initializations")
        .join(format!("{plan_id}.json"));
    write_json(
        &journal_path,
        &serde_json::json!({
            "id": format!("group_init_{plan_id}"),
            "project_id": PROJECT_ID,
            "issue_id": ISSUE_ID,
            "plan_id": plan_id,
            "lock_work_item_id": lock_work_item_id,
            "worktree_lease_id": worktree_lease_id,
            "phase": "completed",
            "created_at": "2026-10-14T00:00:00Z",
            "updated_at": "2026-10-14T00:00:00Z",
        }),
    )
    .expect("seed journal");
}

/// Purge 一个 Plan：墓碑 begin + owned-index 落盘（模拟 D1.1/D2.2 收集的事实）。
fn d1_3c_begin_purge_plan_with_owned(
    store: &LifecycleStore,
    plan_id: &str,
    work_item_ids: Vec<String>,
    target_repository_ids: Vec<String>,
) -> EntityDeletionLease {
    let scope = d1_3a_scope(DeletionEntityKind::Plan, plan_id);
    let lease = EntityOperationStore::new(store.app_paths())
        .try_acquire_delete(&scope)
        .expect("acquire delete lease");
    EntityDeletionStore::new(store.app_paths())
        .begin(&lease)
        .expect("begin purging tombstone");
    let mut manifest =
        crate::product::entity_deletion_store::EntityDeletionManifest::empty(scope.clone());
    manifest.work_item_ids = work_item_ids;
    manifest.target_repository_ids = target_repository_ids;
    EntityDeletionStore::new(store.app_paths())
        .index_owned_artifact_ids(&lease, &manifest)
        .expect("index owned artifact ids");
    lease
}

fn d1_3c_issue_upsert_input(branch: &str) -> UpsertIssueSharedWorktreeInput {
    UpsertIssueSharedWorktreeInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        repository_id: REPOSITORY_ID.to_string(),
        branch_name: branch.to_string(),
        worktree_path: std::path::PathBuf::from("/tmp/aria-worktree-d13c"),
        base_branch: "main".to_string(),
    }
}

fn d1_3c_repo_upsert_input(
    repository_id: LogicalRepositoryId,
    branch: &str,
) -> UpsertRepoSharedWorktreeInput {
    UpsertRepoSharedWorktreeInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        repository_id,
        branch_name: branch.to_string(),
        worktree_path: std::path::PathBuf::from("/tmp/aria-worktree-d13c"),
        base_branch: "main".to_string(),
    }
}

fn d1_3c_wi_with_target(
    store: &LifecycleStore,
    id: &str,
    plan_id: &str,
    target: Option<LogicalRepositoryId>,
) -> crate::product::models::LifecycleWorkItemRecord {
    let mut record = d1_3a_wi(store, id, plan_id);
    if record.target_repository_id != target {
        record.target_repository_id = target;
        let path = store
            .app_paths()
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("work-items")
            .join(format!("{id}.json"));
        write_json(&path, &record).expect("patch wi target");
    }
    record
}

/// issue 两路每类真实入口：late Deleted owner 全拒且记录/旧 lock 不复活；
/// 实体 cleanup 清档后不得凭旧缓存 path 重建（归属未知 fail-closed）。
#[test]
fn cascade_d1_3c_issue_entries_reject_late_deleted_owner_without_revival() {
    let (_tmp, store) = setup();
    d1_3a_plan(&store, "work_item_plan_0001", Vec::new());
    d1_3a_wi(&store, "work_item_0001", "work_item_plan_0001");
    d1_3a_wi(&store, "work_item_0002", "work_item_plan_0001");
    store
        .upsert_issue_shared_worktree(d1_3c_issue_upsert_input("aria/issues/issue_0001"))
        .expect("seed issue worktree");
    store
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            "issue_worktree_lease_d13c",
        )
        .expect("initial lock");

    // purging 墓碑在位：晚到写者全部拒绝。
    d1_3c_begin_purge_plan_with_owned(
        &store,
        "work_item_plan_0001",
        vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
        vec![REPOSITORY_ID.to_string()],
    );
    let record_path = store.issue_shared_worktree_path(PROJECT_ID, ISSUE_ID);
    let bytes_before = std::fs::read(&record_path).expect("record exists");
    for denial in [
        store.upsert_issue_shared_worktree(d1_3c_issue_upsert_input("aria/late")),
        store
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                "work_item_0001",
                "issue_worktree_lease_late",
            )
            .map(|leased| leased.worktree),
        store.bind_issue_worktree_lock_to_attempt(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            "coding_attempt_0001",
        ),
        store.transfer_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            "work_item_0002",
            "issue_worktree_lease_d13c",
        ),
        store.release_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            "issue_worktree_lease_d13c",
        ),
        store.release_issue_worktree_lock_by_owner(
            PROJECT_ID,
            ISSUE_ID,
            "issue_worktree_lease_d13c",
        ),
        store.mark_issue_worktree_completed_item(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            "issue_worktree_lease_d13c",
        ),
        store.takeover_issue_worktree_lock_after_dead_owner(
            PROJECT_ID,
            ISSUE_ID,
            "issue_worktree_lease_d13c",
        ),
    ] {
        assert!(
            matches!(
                &denial,
                Err(ProductStoreError::Conflict {
                    kind: "entity_deleting",
                    ..
                })
            ),
            "late deleted owner 写必须以 entity_deleting 拒绝，实际: {denial:?}"
        );
    }
    assert_eq!(
        std::fs::read(&record_path).expect("record unchanged"),
        bytes_before,
        "拒绝路径不得改写记录"
    );

    // 模拟 D2.2 实体 cleanup 清档 + 删除完成：旧缓存 path 重建必须归属未知拒绝。
    store
        .delete_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
        .expect("cleanup record");
    store
        .delete_work_item(PROJECT_ID, ISSUE_ID, "work_item_0001")
        .expect("cleanup wi 1");
    store
        .delete_work_item(PROJECT_ID, ISSUE_ID, "work_item_0002")
        .expect("cleanup wi 2");
    EntityDeletionStore::new(store.app_paths())
        .complete(
            &EntityOperationStore::new(store.app_paths())
                .try_acquire_delete(&d1_3a_scope(
                    DeletionEntityKind::Plan,
                    "work_item_plan_0001",
                ))
                .expect("re-acquire delete lease"),
        )
        .expect("complete tombstone");
    assert!(
        matches!(
            store.upsert_issue_shared_worktree(d1_3c_issue_upsert_input("aria/revive")),
            Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            })
        ),
        "无可证 owner 且原资源已被实体 cleanup 清除：不得凭旧缓存 path 重建"
    );
    assert!(!record_path.exists(), "拒绝路径不得复活已清档记录");

    // 新 Plan（真实 live owner）同仓正常创建不当旧 Plan 复活。
    d1_3a_plan(&store, "work_item_plan_0002", Vec::new());
    d1_3a_wi(&store, "work_item_0003", "work_item_plan_0002");
    let revived = store
        .upsert_issue_shared_worktree(d1_3c_issue_upsert_input("aria/new-plan"))
        .expect("新 live Plan 的正常创建必须放行");
    assert_eq!(revived.branch_name, "aria/new-plan");
    store
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0003",
            "issue_worktree_lease_new_plan",
        )
        .expect("新 live Plan 正常取锁");
}

/// 仓维（三元键）同款闭包：late Deleted owner 全拒；异仓与另一 live Plan 不受牵连。
#[test]
fn cascade_d1_3c_repo_entries_reject_late_deleted_owner_and_live_plan_kept() {
    let (_tmp, store) = setup();
    let api_member = LogicalRepositoryId(stable_member_uuid(0x0001));
    let web_member = LogicalRepositoryId(stable_member_uuid(0x0002));
    d1_3a_plan(&store, "work_item_plan_0001", Vec::new());
    d1_3c_wi_with_target(
        &store,
        "work_item_0001",
        "work_item_plan_0001",
        Some(api_member),
    );
    d1_3c_wi_with_target(
        &store,
        "work_item_0002",
        "work_item_plan_0001",
        Some(api_member),
    );
    store
        .upsert_repo_shared_worktree(d1_3c_repo_upsert_input(api_member, "aria/api"))
        .expect("seed api repo worktree");
    store
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            api_member,
            "work_item_0001",
            "repo_worktree_lease_d13c",
        )
        .expect("initial repo lock");

    d1_3c_begin_purge_plan_with_owned(
        &store,
        "work_item_plan_0001",
        vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
        vec![api_member.0.to_string()],
    );
    let record_path = store.repo_shared_worktree_path(PROJECT_ID, ISSUE_ID, api_member);
    let bytes_before = std::fs::read(&record_path).expect("record exists");
    for denial in [
        store.upsert_repo_shared_worktree(d1_3c_repo_upsert_input(api_member, "aria/late")),
        store
            .try_acquire_repo_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                api_member,
                "work_item_0001",
                "repo_worktree_lease_late",
            )
            .map(|leased| leased.worktree),
        store.bind_repo_worktree_lock_to_attempt(
            PROJECT_ID,
            ISSUE_ID,
            api_member,
            "work_item_0001",
            "coding_attempt_0001",
        ),
        store.transfer_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            api_member,
            "work_item_0001",
            "work_item_0002",
            "repo_worktree_lease_d13c",
        ),
        store.release_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            api_member,
            "work_item_0001",
            "repo_worktree_lease_d13c",
        ),
        store.release_repo_worktree_lock_by_owner(
            PROJECT_ID,
            ISSUE_ID,
            api_member,
            "repo_worktree_lease_d13c",
        ),
        store.mark_repo_worktree_completed_item(
            PROJECT_ID,
            ISSUE_ID,
            api_member,
            "work_item_0001",
            "repo_worktree_lease_d13c",
        ),
        store.takeover_repo_worktree_lock_after_dead_owner(
            PROJECT_ID,
            ISSUE_ID,
            api_member,
            "repo_worktree_lease_d13c",
        ),
    ] {
        assert!(
            matches!(
                &denial,
                Err(ProductStoreError::Conflict {
                    kind: "entity_deleting",
                    ..
                })
            ),
            "仓维 late deleted owner 写必须拒绝，实际: {denial:?}"
        );
    }
    assert_eq!(
        std::fs::read(&record_path).expect("record unchanged"),
        bytes_before
    );

    // 清档完成后：该仓归属未知拒绝重建；另一 live Plan 同仓正常；异仓不受牵连。
    store
        .delete_repo_shared_worktree(PROJECT_ID, ISSUE_ID, api_member)
        .expect("cleanup api record");
    store
        .delete_work_item(PROJECT_ID, ISSUE_ID, "work_item_0001")
        .expect("cleanup wi 1");
    store
        .delete_work_item(PROJECT_ID, ISSUE_ID, "work_item_0002")
        .expect("cleanup wi 2");
    EntityDeletionStore::new(store.app_paths())
        .complete(
            &EntityOperationStore::new(store.app_paths())
                .try_acquire_delete(&d1_3a_scope(
                    DeletionEntityKind::Plan,
                    "work_item_plan_0001",
                ))
                .expect("re-acquire delete lease"),
        )
        .expect("complete tombstone");
    assert!(
        matches!(
            store.upsert_repo_shared_worktree(d1_3c_repo_upsert_input(api_member, "aria/revive")),
            Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            })
        ),
        "已删 Plan 清档后的仓维资源不得重建"
    );

    d1_3a_plan(&store, "work_item_plan_0002", Vec::new());
    d1_3c_wi_with_target(
        &store,
        "work_item_0003",
        "work_item_plan_0002",
        Some(api_member),
    );
    store
        .upsert_repo_shared_worktree(d1_3c_repo_upsert_input(api_member, "aria/new-plan"))
        .expect("另一 live Plan 同仓正常创建");
    store
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            api_member,
            "work_item_0003",
            "repo_worktree_lease_new_plan",
        )
        .expect("另一 live Plan 同仓正常取锁");
    store
        .upsert_repo_shared_worktree(d1_3c_repo_upsert_input(web_member, "aria/web"))
        .expect("异仓不受已删 Plan 牵连");
    store
        .try_acquire_repo_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            web_member,
            "work_item_0003",
            "repo_worktree_lease_web",
        )
        .expect("异仓正常取锁");
}

/// 无 plan_id 参数时按真实记录反推：WI 记录、attempt group 绑定、
/// group 初始化 journal 的 lease join 三源各自命中已删 Plan 即拒。
#[test]
fn cascade_d1_3c_ownership_reversed_from_wi_attempt_and_journal() {
    let (_tmp, store) = setup();

    // 源 1：记录 current_active WI 的 source_work_item_plan_id。
    d1_3a_plan(&store, "work_item_plan_0001", Vec::new());
    d1_3a_wi(&store, "work_item_0001", "work_item_plan_0001");
    d1_3c_seed_issue_worktree(&store, |record| {
        record.current_active_work_item_id = Some("work_item_0001".to_string());
        record.current_lock_owner_id = Some("opaque_owner_d13c_1".to_string());
    });
    d1_3c_begin_purge_plan_with_owned(&store, "work_item_plan_0001", Vec::new(), Vec::new());
    assert!(
        matches!(
            store.release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, "opaque_owner_d13c_1"),
            Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            })
        ),
        "WI 记录反推已删 Plan：迟到释放必须拒绝"
    );

    // 源 2：lock owner 是 attempt id，group 绑定（work_item_group_id）反推 Plan。
    let (_tmp2, store2) = setup();
    d1_3a_plan(&store2, "work_item_plan_0002", Vec::new());
    let attempt = d1_3c_seed_attempt(
        &store2.app_paths(),
        "coding_attempt_0002",
        "work_item_0009",
        Some("work_item_plan_0002"),
        None,
    );
    d1_3c_seed_issue_worktree(&store2, |record| {
        record.current_lock_owner_id = Some(attempt.id.clone());
    });
    d1_3c_begin_purge_plan_with_owned(&store2, "work_item_plan_0002", Vec::new(), Vec::new());
    assert!(
        matches!(
            store2.release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, &attempt.id),
            Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            })
        ),
        "attempt group 绑定反推已删 Plan：迟到释放必须拒绝"
    );

    // 源 3：lock owner 是 issue/repo_worktree_lease_* 不透明租约，经 group
    // 初始化 journal 的 worktree_lease_id join 反推（不按 prefix 解析）。
    let (_tmp3, store3) = setup();
    d1_3a_plan(&store3, "work_item_plan_0003", Vec::new());
    d1_3c_seed_journal(
        &store3.app_paths(),
        "work_item_plan_0003",
        "work_item_0010",
        "issue_worktree_lease_d13c_3",
    );
    d1_3c_seed_issue_worktree(&store3, |record| {
        record.current_active_work_item_id = Some("work_item_0010".to_string());
        record.current_lock_owner_id = Some("issue_worktree_lease_d13c_3".to_string());
    });
    d1_3c_begin_purge_plan_with_owned(&store3, "work_item_plan_0003", Vec::new(), Vec::new());
    assert!(
        matches!(
            store3.release_issue_worktree_lock_by_owner(
                PROJECT_ID,
                ISSUE_ID,
                "issue_worktree_lease_d13c_3"
            ),
            Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            })
        ),
        "journal lease join 反推已删 Plan：迟到释放必须拒绝"
    );
    assert!(
        matches!(
            store3.takeover_issue_worktree_lock_after_dead_owner(
                PROJECT_ID,
                ISSUE_ID,
                "issue_worktree_lease_d13c_3",
            ),
            Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            })
        ),
        "journal 反推同样约束死亡租约接管"
    );
}

/// 互相矛盾或损坏 fail-closed：journal join 与记录 active WI 互相矛盾报
/// 归属错误；record repository 键只用于核目标，错档拒改。
#[test]
fn cascade_d1_3c_contradictory_or_corrupt_ownership_fails_closed() {
    let (_tmp, store) = setup();
    let paths = store.app_paths();

    // journal 按 lease 命中，但其 lock_work_item_id 与记录 active WI 矛盾。
    d1_3a_plan(&store, "work_item_plan_0001", Vec::new());
    d1_3c_seed_journal(
        &paths,
        "work_item_plan_0001",
        "work_item_0010",
        "issue_worktree_lease_d13c_conflict",
    );
    d1_3c_seed_issue_worktree(&store, |record| {
        record.current_active_work_item_id = Some("work_item_0099".to_string());
        record.current_lock_owner_id = Some("issue_worktree_lease_d13c_conflict".to_string());
    });
    assert!(
        matches!(
            store.release_issue_worktree_lock_by_owner(
                PROJECT_ID,
                ISSUE_ID,
                "issue_worktree_lease_d13c_conflict",
            ),
            Err(ProductStoreError::InvalidRecord { kind, .. }) if kind == "shared_worktree_owner"
        ),
        "journal join 与记录 active WI 矛盾必须归属错误 fail-closed"
    );

    // 仓维记录 target 键错档（API 文件内容却是 WEB target）：不得改写。
    let (_tmp2, store2) = setup();
    let api_member = LogicalRepositoryId(stable_member_uuid(0x0001));
    let web_member = LogicalRepositoryId(stable_member_uuid(0x0002));
    d1_3c_seed_repo_worktree(&store2, api_member, |record| {
        record.target_repository_id = Some(web_member);
    });
    assert!(
        matches!(
            store2.upsert_repo_shared_worktree(d1_3c_repo_upsert_input(api_member, "aria/late")),
            Err(ProductStoreError::IdentityMismatch {
                kind: "repo_shared_worktree",
                ..
            })
        ),
        "record repository 键错档必须 IdentityMismatch fail-closed"
    );
}

/// delete 与 upsert 两顺序互斥（稳定 resource 短 lease），且锁 inode 稳定不 unlink。
#[test]
fn cascade_d1_3c_delete_and_upsert_mutually_exclusive_with_stable_inode() {
    let (_tmp, store) = setup();
    let api_member = LogicalRepositoryId(stable_member_uuid(0x0001));
    let operations = EntityOperationStore::new(store.app_paths());
    let key = crate::product::entity_operation_store::SharedResourceKey {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        repository_id: Some(api_member),
    };

    // 顺序 1：writer 短 lease 在持 → 同仓 upsert Busy 拒 + cleanup 互斥。
    store
        .upsert_repo_shared_worktree(d1_3c_repo_upsert_input(api_member, "aria/first"))
        .expect("seed repo worktree");
    let resource_lock_path = store
        .app_paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("entity-operations")
        .join("resource-locks")
        .join(format!("{}.lock", api_member.0));
    let _writer = operations
        .try_acquire_shared_resource_write(&key)
        .expect("writer lease");
    assert!(
        matches!(
            operations.try_acquire_shared_resource_cleanup(&key),
            Err(crate::product::entity_operation_store::EntityOperationError::Busy)
        ),
        "writer 在持：cleanup 互斥"
    );
    assert!(
        matches!(
            store
                .upsert_repo_shared_worktree(d1_3c_repo_upsert_input(api_member, "aria/contended")),
            Err(ProductStoreError::Conflict {
                kind: "shared_resource_busy",
                ..
            })
        ),
        "writer 在持：upsert 必须稳定 Busy 拒（不等待）"
    );
    let inode_before = std::fs::metadata(&resource_lock_path)
        .expect("resource lock file")
        .ino();
    drop(_writer);

    // 顺序 2：cleanup 短 lease 在持 → upsert 同样 Busy 拒。
    let _cleanup = operations
        .try_acquire_shared_resource_cleanup(&key)
        .expect("cleanup lease");
    assert!(
        matches!(
            store
                .upsert_repo_shared_worktree(d1_3c_repo_upsert_input(api_member, "aria/contended")),
            Err(ProductStoreError::Conflict {
                kind: "shared_resource_busy",
                ..
            })
        ),
        "cleanup 在持：upsert 必须稳定 Busy 拒"
    );
    drop(_cleanup);
    assert_eq!(
        std::fs::metadata(&resource_lock_path)
            .expect("resource lock file")
            .ino(),
        inode_before,
        "resource 锁 inode 稳定（purge 外、不 unlink 在持 inode）"
    );

    // lease 释放后正常放行。
    store
        .upsert_repo_shared_worktree(d1_3c_repo_upsert_input(api_member, "aria/second"))
        .expect("lease 释放后 upsert 放行");
}

/// P2 修复轮：issue 键位记录的第一方迁移 tagged 态（backfill_shared_worktree
/// 原地写 Some(target)+path_schema_version=1，legacy_shared_worktree_migration
/// 显式容忍）不得被 None 键准入当 IdentityMismatch 硬拒——过渡窗口在途
/// Legacy attempt 的合法 owner 操作必须放行；repo 维（Some 键）严格比对保持。
#[test]
fn cascade_d1_3c_issue_admission_tolerates_backfill_tagged_record() {
    let (_tmp, store) = setup();
    let api_member = LogicalRepositoryId(stable_member_uuid(0x0001));
    d1_3a_plan(&store, "work_item_plan_0001", Vec::new());
    d1_3a_wi(&store, "work_item_0001", "work_item_plan_0001");
    d1_3c_seed_issue_worktree(&store, |record| {
        record.target_repository_id = Some(api_member);
        record.path_schema_version = 1;
        record.current_active_work_item_id = Some("work_item_0001".to_string());
        record.current_lock_owner_id = Some("issue_worktree_lease_d13c_tagged".to_string());
    });

    // 在途 Legacy attempt 的合法 owner 操作：不得因迁移 tagged 态硬失败。
    let released = store
        .release_issue_worktree_lock_by_owner(
            PROJECT_ID,
            ISSUE_ID,
            "issue_worktree_lease_d13c_tagged",
        )
        .expect("迁移 tagged 态的合法 owner 操作必须放行");
    assert!(released.current_lock_owner_id.is_none());

    // hint 空 upsert 更新同样容忍 tagged 态（键位由文件路径确定）。
    let updated = store
        .upsert_issue_shared_worktree(d1_3c_issue_upsert_input("aria/tagged-update"))
        .expect("tagged 态 upsert 更新放行");
    assert_eq!(updated.branch_name, "aria/tagged-update");
    assert_eq!(updated.target_repository_id, Some(api_member));

    // 重新持锁后删除 Plan：tagged 态不放宽 late deleted owner 拒绝语义。
    store
        .try_acquire_issue_worktree_lock(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            "issue_worktree_lease_d13c_tagged",
        )
        .expect("live plan re-acquire");
    d1_3c_begin_purge_plan_with_owned(&store, "work_item_plan_0001", Vec::new(), Vec::new());
    assert!(
        matches!(
            store.release_issue_worktree_lock_by_owner(
                PROJECT_ID,
                ISSUE_ID,
                "issue_worktree_lease_d13c_tagged"
            ),
            Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            })
        ),
        "tagged 态 late deleted owner 仍必须拒绝"
    );
    assert!(
        matches!(
            store.upsert_issue_shared_worktree(d1_3c_issue_upsert_input("aria/tagged-late")),
            Err(ProductStoreError::Conflict {
                kind: "entity_deleting",
                ..
            })
        ),
        "tagged 态已删 Plan 现资源仍不得覆盖"
    );
}
include!("tests/cascade_d1_1.rs");
include!("tests/cascade_d1_4.rs");
