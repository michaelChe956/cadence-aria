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
                flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
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
    let session = create_session(&store, "issue_work_item_plan_0001", WorkspaceType::WorkItemPlan);
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
    let session = create_session(&store, "issue_work_item_plan_0001", WorkspaceType::WorkItemPlan);
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
    let session = create_session(&store, "issue_work_item_plan_0001", WorkspaceType::WorkItemPlan);

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
    let first_session =
        create_session(&store, "issue_work_item_plan_0001", WorkspaceType::WorkItemPlan);
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
    store.delete_story_spec(PROJECT_ID, ISSUE_ID, &story.id).unwrap();
    let plan_lease = d1_3a_begin_purge(
        &store,
        &d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001"),
    );
    d1_3a_complete_purge(&store, plan_lease);
    let story_lease =
        d1_3a_begin_purge(&store, &d1_3a_scope(DeletionEntityKind::Story, &story.id));
    d1_3a_complete_purge(&store, story_lease);

    // 新 session/实体 ID 不复用已删旧 ID。
    d1_3a_plan(&store, "issue_work_item_plan_0002", Vec::new());
    let second_session =
        create_session(&store, "issue_work_item_plan_0002", WorkspaceType::WorkItemPlan);
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
    assert!(!store
        .app_paths()
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("issue-work-item-plans")
        .join("issue_work_item_plan_0001.json")
        .exists());

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
    assert!(store
        .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, &plan.id)
        .is_ok());
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
