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
    assert_eq!(saved_turn, turn);
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

// === 聚合视野：StorySpec 校验（Task 7）===
// 稳定 UUID：禁止运行时随机，保证测试可复现；ID 组成磁盘路径前经 validate_relative_id
// 约束（本测试使用 project_0001 / issue_0001 等稳定 id）。
const fn stable_member_uuid(seed: u16) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[14] = (seed >> 8) as u8;
    bytes[15] = seed as u8;
    // version 7 + variant 10xx，满足 Uuid::from_bytes 的合法构造。
    bytes[6] = 0x70;
    bytes[8] = 0x80;
    Uuid::from_bytes(bytes)
}

const CODEBASE_REF: Uuid = stable_member_uuid(0x0100);
const API_MEMBER: LogicalRepositoryId = LogicalRepositoryId(stable_member_uuid(0x0001));
const WEB_MEMBER: LogicalRepositoryId = LogicalRepositoryId(stable_member_uuid(0x0002));
const UNKNOWN_MEMBER: LogicalRepositoryId = LogicalRepositoryId(stable_member_uuid(0x00ff));

/// 构造聚合 StorySpec scope：effective 成员 = {api, web}。`involved_repository_ids`
/// 与 `focus_repository_id` 留空，由各测试用例用 `..` 语法覆盖。
fn two_effective_members_scope() -> AggregateStorySpecScope {
    AggregateStorySpecScope {
        logical_codebase_ref: CODEBASE_REF,
        effective_member_ids: vec![API_MEMBER, WEB_MEMBER],
        involved_repository_ids: Vec::new(),
        focus_repository_id: None,
    }
}

fn story_spec_dto_path(store: &LifecycleStore, story_id: &str) -> std::path::PathBuf {
    store
        .story_specs_root(PROJECT_ID, ISSUE_ID)
        .join(format!("{story_id}.json"))
}

/// AI 未明确涉及任何仓库（involved_repository_ids 为空）→ 方案 X 阶段 1：Draft 态允许空
/// involved（AI 尚未产出），create 时恒为 Draft → 成功持久化为 Draft，不回落 primary 也不
/// blocker；Confirmed 态强制非空由 spec::tests 的 validate_aggregate_story_scope 单测覆盖。
#[test]
fn story_without_involved_repositories_persists_as_draft() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: Vec::new(),
                focus_repository_id: None,
                ..two_effective_members_scope()
            }),
        })
        .unwrap();

    // Draft 态允许空 involved：不回落 primary，聚合字段按原值持久化。
    assert_eq!(
        story.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );
    assert!(story.involved_repository_ids.is_empty());
    assert_eq!(story.focus_repository_id, None);
    assert_eq!(story.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(story.repository_id, REPOSITORY_ID);

    // 磁盘持久化一致（不回落 primary）。
    let persisted: StorySpecRecord = read_json(&story_spec_dto_path(&store, &story.id)).unwrap();
    assert!(persisted.involved_repository_ids.is_empty());
    assert_eq!(
        persisted.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );
}

/// AI 输出不在有效集合内的仓库 → blocker，不持久化 StorySpec，不回落 primary。
#[test]
fn story_with_involved_outside_effective_becomes_blocker() {
    let (_tmp, store) = setup();
    let error = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: vec![API_MEMBER, UNKNOWN_MEMBER],
                focus_repository_id: Some(API_MEMBER),
                ..two_effective_members_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("involved_repository_not_effective")),
        "越界 involved_repository_ids 应 fail-closed 为 blocker，错误: {error:?}"
    );
    assert_eq!(
        store.list_story_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 StorySpec"
    );
}

/// 校验通过 → 持久化 StorySpec，填充聚合视野字段。
#[test]
fn story_with_involved_within_effective_persists_aggregate_scope() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: vec![WEB_MEMBER, API_MEMBER],
                focus_repository_id: Some(API_MEMBER),
                ..two_effective_members_scope()
            }),
        })
        .unwrap();

    assert_eq!(story.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(story.involved_repository_ids, vec![WEB_MEMBER, API_MEMBER]);
    assert_eq!(story.focus_repository_id, Some(API_MEMBER));
    // 迁移期 repository_id 仅作 primary 投影，保持传入值。
    assert_eq!(story.repository_id, REPOSITORY_ID);

    // 磁盘持久化一致。
    let persisted: StorySpecRecord = read_json(&story_spec_dto_path(&store, &story.id)).unwrap();
    assert_eq!(persisted.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(
        persisted.involved_repository_ids,
        vec![WEB_MEMBER, API_MEMBER]
    );
    assert_eq!(persisted.focus_repository_id, Some(API_MEMBER));
}

/// focus_repository_id ∈ involved_repository_ids 但不在 involved 列表中 → blocker。
#[test]
fn story_focus_repository_must_be_within_involved() {
    let (_tmp, store) = setup();
    let error = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: vec![API_MEMBER],
                focus_repository_id: Some(WEB_MEMBER),
                ..two_effective_members_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("focus_repository_not_involved")),
        "focus 必须在 involved 集合内，错误: {error:?}"
    );
    assert_eq!(
        store.list_story_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 StorySpec"
    );
}

/// 传统单仓 issue（aggregate_codebase = None）仍走原 repository_id 单值路径，
/// 聚合视野字段保持空/None。
#[test]
fn story_without_aggregate_scope_keeps_single_repository_path() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "single repo story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();

    assert_eq!(story.repository_id, REPOSITORY_ID);
    assert_eq!(story.logical_codebase_ref, None);
    assert!(story.involved_repository_ids.is_empty());
    assert_eq!(story.focus_repository_id, None);
}

// === 聚合视野：DesignSpec 校验与改动顺序持久化（Task 8）===
// 复用 Task 7 稳定 UUID 常量；Design 不回落 issue.repo_id，involved_repository_ids 必须
// ⊆ effective_member_ids；change_order 由 AI 显式给出（执行顺序图，非服务调用图）。
fn two_effective_members_design_scope() -> AggregateDesignSpecScope {
    AggregateDesignSpecScope {
        logical_codebase_ref: CODEBASE_REF,
        effective_member_ids: vec![API_MEMBER, WEB_MEMBER],
        involved_repository_ids: Vec::new(),
        change_order: Vec::new(),
    }
}

fn design_spec_dto_path(store: &LifecycleStore, design_id: &str) -> std::path::PathBuf {
    store
        .design_specs_root(PROJECT_ID, ISSUE_ID)
        .join(format!("{design_id}.json"))
}

/// AI 未明确涉及任何仓库（involved_repository_ids 为空）→ 方案 X 阶段 1：Draft 态允许空
/// involved（AI 尚未产出），create 时恒为 Draft → 成功持久化为 Draft，不回落 issue.repo_id
/// 也不 blocker；Confirmed 态强制非空由 spec::tests 的 validate_aggregate_design_scope 单测覆盖。
#[test]
fn design_without_involved_repositories_persists_as_draft() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: Vec::new(),
                change_order: Vec::new(),
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    // Draft 态允许空 involved：不回落 issue.repo_id，聚合字段按原值持久化。
    assert_eq!(
        design.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );
    assert!(design.involved_repository_ids.is_empty());
    assert!(design.change_order.is_empty());
    assert_eq!(design.logical_codebase_ref, Some(CODEBASE_REF));

    // 磁盘持久化一致。
    let persisted: DesignSpecRecord = read_json(&design_spec_dto_path(&store, &design.id)).unwrap();
    assert!(persisted.involved_repository_ids.is_empty());
    assert_eq!(
        persisted.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );
}

/// AI 输出不在有效集合内的仓库 → blocker，不持久化 DesignSpec，不回落 issue.repo_id。
#[test]
fn design_with_involved_outside_effective_becomes_blocker() {
    let (_tmp, store) = setup();
    let error = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, UNKNOWN_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("involved_repository_not_effective")),
        "越界 involved_repository_ids 应 fail-closed 为 blocker，错误: {error:?}"
    );
    assert_eq!(
        store.list_design_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 DesignSpec"
    );
}

/// change_order 的任一 id ∉ involved_repository_ids → blocker（改动顺序必须覆盖全部涉及仓库）。
#[test]
fn design_change_order_outside_involved_becomes_blocker() {
    let (_tmp, store) = setup();
    let error = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("change_order_repository_not_involved")),
        "change_order 越界应 fail-closed 为 blocker，错误: {error:?}"
    );
    assert_eq!(
        store.list_design_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 DesignSpec"
    );
}

/// change_order 出现重复 involved 仓库 → blocker（执行顺序图不得重复顶点）。
#[test]
fn design_change_order_with_duplicate_repository_becomes_blocker() {
    let (_tmp, store) = setup();
    let error = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER, API_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("change_order_duplicate_repository")),
        "change_order 重复顶点应 fail-closed 为 blocker，错误: {error:?}"
    );
}

/// change_order 缺失（空）→ 非 blocker（AI 可不给改动顺序）；仅当 WorkItem 编译时若有才作
/// depends_on 依据（Task 9 消费）。involved 有效即持久化，聚合字段填充，磁盘一致。
/// Design 不回落 issue.repo_id：involved_repository_ids 全部 ∈ effective_member_ids，
/// 且 DesignSpecRecord 无 repository_id 字段（conceptual repository_id_or_none() == None）。
#[test]
fn design_change_order_optional_but_involved_valid_persists_aggregate_scope() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: Vec::new(),
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    // involved_repository_ids ⊆ effective_member_ids，且不回落 issue.repo_id。
    assert_eq!(design.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(design.involved_repository_ids, vec![API_MEMBER, WEB_MEMBER]);
    assert!(design.change_order.is_empty());

    // 磁盘持久化一致。
    let persisted: DesignSpecRecord = read_json(&design_spec_dto_path(&store, &design.id)).unwrap();
    assert_eq!(persisted.logical_codebase_ref, Some(CODEBASE_REF));
    assert_eq!(
        persisted.involved_repository_ids,
        vec![API_MEMBER, WEB_MEMBER]
    );
    assert!(persisted.change_order.is_empty());
}

/// change_order 由 AI 显式给出（例：公共契约 → provider → consumer）→ 持久化为执行顺序图，
/// involved_repository_ids 全部 ∈ effective_member_ids。Design 不回落 issue.repo_id。
#[test]
fn design_change_order_drives_persisted_order_and_does_not_fallback_to_repo_id() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "aggregate design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    // change_order 被持久化、顺序保留；involved 全部 ∈ effective_member_ids。
    assert_eq!(design.change_order.len(), 2);
    assert_eq!(design.change_order, vec![API_MEMBER, WEB_MEMBER]);
    assert!(
        design
            .involved_repository_ids
            .iter()
            .all(|id| [API_MEMBER, WEB_MEMBER].contains(id))
    );
    // DesignSpecRecord 无 repository_id 字段，即 conceptual repository_id_or_none() == None。
    let encoded = serde_json::to_value(&design).unwrap();
    assert!(
        encoded.get("repository_id").is_none(),
        "Design 不应回落 issue.repo_id：encoded={encoded}"
    );

    // 磁盘持久化一致。
    let persisted: DesignSpecRecord = read_json(&design_spec_dto_path(&store, &design.id)).unwrap();
    assert_eq!(persisted.change_order, vec![API_MEMBER, WEB_MEMBER]);
}

/// 传统单仓 issue（aggregate_codebase = None）仍走原 DesignSpec 单值路径，聚合视野字段空。
#[test]
fn design_without_aggregate_scope_keeps_single_repository_path() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "single repo design".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();

    assert_eq!(design.logical_codebase_ref, None);
    assert!(design.involved_repository_ids.is_empty());
    assert!(design.change_order.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// 确认 gate（Task 6 下沉，Blocker 2）：validate_confirm_aggregate_spec
// ─────────────────────────────────────────────────────────────────────────────

/// 多仓 Story（logical_codebase_ref Some）involved 为空 → blocker involved_repositories_undetermined。
#[test]
fn validate_confirm_aggregate_spec_rejects_multi_repo_story_without_involved() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: String::new(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: Vec::new(),
                focus_repository_id: None,
                ..two_effective_members_scope()
            }),
        })
        .unwrap();

    let error = store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &story.id, &WorkspaceType::Story)
        .unwrap_err();
    assert_eq!(
        error.stable_code(),
        "involved_repositories_undetermined",
        "expected involved_repositories_undetermined, got: {error:?}"
    );
    assert!(
        matches!(
            error,
            ConfirmAggregateGateError::Violation {
                violation: ConfirmGateViolation::InvolvedUndetermined,
                ..
            }
        ),
        "expected InvolvedUndetermined violation, got: {error:?}"
    );
}

/// 多仓 Design（involved=2）缺 change_order → blocker change_order_required_for_logical_codebase。
#[test]
fn validate_confirm_aggregate_spec_rejects_multi_repo_design_without_change_order() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "multi repo design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: Vec::new(),
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    let error = store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &design.id, &WorkspaceType::Design)
        .unwrap_err();
    assert_eq!(
        error.stable_code(),
        "change_order_required_for_logical_codebase",
        "expected change_order_required_for_logical_codebase, got: {error:?}"
    );
    assert!(
        matches!(
            error,
            ConfirmAggregateGateError::Violation {
                violation: ConfirmGateViolation::ChangeOrderRequired,
                ..
            }
        ),
        "expected ChangeOrderRequired violation, got: {error:?}"
    );
}

/// 多仓 Design 带 change_order → 通过 gate。
#[test]
fn validate_confirm_aggregate_spec_passes_multi_repo_design_with_change_order() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "multi repo design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                change_order: vec![API_MEMBER, WEB_MEMBER],
                ..two_effective_members_design_scope()
            }),
        })
        .unwrap();

    store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &design.id, &WorkspaceType::Design)
        .expect("multi-repo design with change_order must pass the confirm gate");
}

/// 多仓 Story 带 involved → 通过 gate。
#[test]
fn validate_confirm_aggregate_spec_passes_multi_repo_story_with_involved() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: String::new(),
            title: "aggregate story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                involved_repository_ids: vec![API_MEMBER, WEB_MEMBER],
                focus_repository_id: None,
                ..two_effective_members_scope()
            }),
        })
        .unwrap();

    store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &story.id, &WorkspaceType::Story)
        .expect("multi-repo story with involved must pass the confirm gate");
}

/// 单仓 Story（无 aggregate）→ 不校验，gate 通过（红线）。
#[test]
fn validate_confirm_aggregate_spec_passes_single_repo_story() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "single repo story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();

    store
        .validate_confirm_aggregate_spec(PROJECT_ID, ISSUE_ID, &story.id, &WorkspaceType::Story)
        .expect("single-repo story must pass the confirm gate");
}

/// 非 Story/Design workspace（如 WorkItem）→ gate 直接通过（不校验）。
#[test]
fn validate_confirm_aggregate_spec_ignores_non_story_design_workspace() {
    let (_tmp, store) = setup();
    store
        .validate_confirm_aggregate_spec(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            &WorkspaceType::WorkItem,
        )
        .expect("non story/design workspace must skip the aggregate confirm gate");
}

// Task 9 三元键 shared worktree 回归测试拆分到独立文件，经 include! 引入（large_file_guard 1200 行红线）。
include!("tests/task9_repo_worktree.rs");
include!("tests/human_gate_recovery.rs");
