use tempfile::TempDir;

use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::lifecycle_store::{AggregateDesignSpecScope, AggregateStorySpecScope};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{
    DesignSpecRecord, ProviderName, StorySpecRecord, WorkItemRuntimeBinding, WorkspaceType,
};

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
        })
        .unwrap()
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

/// AI 未明确涉及任何仓库（involved_repository_ids 为空）→ blocker，不持久化 StorySpec。
#[test]
fn story_without_involved_repositories_becomes_blocker_not_primary() {
    let (_tmp, store) = setup();
    let error = store
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
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("involved_repositories_undetermined")),
        "空 involved_repository_ids 应 fail-closed 为 blocker，错误: {error:?}"
    );
    assert_eq!(
        store.list_story_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 StorySpec"
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

/// AI 未明确涉及任何仓库（involved_repository_ids 为空）→ blocker，不持久化 DesignSpec，
/// 不回落 issue.repo_id（REQ-PLN-08）。
#[test]
fn design_without_involved_repositories_becomes_blocker_not_repo_id() {
    let (_tmp, store) = setup();
    let error = store
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
        .unwrap_err();

    assert!(
        matches!(error, ProductStoreError::InvalidRecord { ref reason, .. }
            if reason.contains("involved_repositories_undetermined")),
        "空 involved_repository_ids 应 fail-closed 为 blocker，错误: {error:?}"
    );
    assert_eq!(
        store.list_design_specs(PROJECT_ID, ISSUE_ID).unwrap().len(),
        0,
        "blocker 时不应持久化 DesignSpec"
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
