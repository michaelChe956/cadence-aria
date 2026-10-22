use tempfile::tempdir;

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::WorkItemPlanSessionOptions;
use crate::product::models::ProviderName;
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};

fn create_input(
    workspace_type: WorkspaceType,
    work_item_plan_options: Option<WorkItemPlanSessionOptions>,
) -> CreateWorkspaceSessionInput {
    CreateWorkspaceSessionInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "entity_0001".to_string(),
        workspace_type,
        author_provider: ProviderName::Codex,
        reviewer_provider: Some(ProviderName::ClaudeCode),

        review_rounds: 1,
        superpowers_enabled: false,
        openspec_enabled: false,
        work_item_plan_options,
    }
}

#[test]
fn create_workspace_session_persists_work_item_plan_options() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let options = WorkItemPlanSessionOptions {
        flow_kind: WorkItemPlanFlowKind::SingleCandidate,
        run_policy: RunPolicy::AutoIfValid,
        rollout_snapshot: true,
    };

    let session = store
        .create_workspace_session(create_input(
            WorkspaceType::WorkItemPlan,
            Some(options.clone()),
        ))
        .unwrap();

    assert_eq!(session.flow_kind, options.flow_kind);
    assert_eq!(session.run_policy, options.run_policy);
    assert_eq!(
        session.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Prepare)
    );
    let reissued = store
        .create_workspace_session_with_id(
            create_input(WorkspaceType::WorkItemPlan, Some(options.clone())),
            session.id.clone(),
        )
        .expect("reissued create returns immutable existing snapshot");
    assert_eq!(reissued.run_policy, options.run_policy);
    assert_eq!(
        store.get_workspace_session(&session.id).unwrap().flow_kind,
        WorkItemPlanFlowKind::SingleCandidate
    );
}

#[test]
fn create_workspace_session_rejects_work_item_plan_options_for_other_workspace_types() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));

    let error = store
        .create_workspace_session(create_input(
            WorkspaceType::Story,
            Some(WorkItemPlanSessionOptions::default()),
        ))
        .unwrap_err();

    assert!(matches!(
        error,
        ProductStoreError::InvalidRecord {
            kind: "workspace_session",
            ..
        }
    ));
}

/// 批 2 P2（REQ-WIGA-02 修订）：绑定创建接受明确的 AutoIfValid——run
/// policy 的授权口径移至 enrollment 冻结策略（创建/binding/准入/投影
/// 各接缝互证）；store 侧保留完整身份比较，run policy 漂移即 IdentityMismatch。
#[test]
fn create_workspace_session_bound_accepts_auto_if_valid_with_full_identity_comparison() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let options = WorkItemPlanSessionOptions {
        flow_kind: WorkItemPlanFlowKind::SingleCandidate,
        run_policy: RunPolicy::AutoIfValid,
        rollout_snapshot: true,
    };
    let input = create_input(WorkspaceType::WorkItemPlan, Some(options.clone()));

    let session = store
        .create_workspace_session_bound(input.clone(), "workspace_session_bound_auto".to_string())
        .expect("bound creation accepts explicit AutoIfValid");
    assert_eq!(session.id, "workspace_session_bound_auto");
    assert_eq!(session.run_policy, RunPolicy::AutoIfValid);

    // 同输入重放：幂等返回既存快照，不二次创建。
    let replayed = store
        .create_workspace_session_bound(input, "workspace_session_bound_auto".to_string())
        .expect("idempotent bound replay");
    assert_eq!(replayed.created_at, session.created_at);

    // 同 id 但 run policy 漂移（AutoIfValid → Interactive 降级）：完整身份
    // 比较拒绝，不覆盖既存会话。
    let drifted = WorkItemPlanSessionOptions {
        run_policy: RunPolicy::Interactive,
        ..options
    };
    let drifted_input = create_input(WorkspaceType::WorkItemPlan, Some(drifted));
    assert!(matches!(
        store.create_workspace_session_bound(
            drifted_input,
            "workspace_session_bound_auto".to_string()
        ),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

/// REQ-ENV-13（设计 A 条目 17）：每次创建 workspace 会话 MUST 生成
/// 全新的唯一评审身份（UUID simple 形态），两会话身份互不相同且非空。
#[test]
fn create_workspace_session_assigns_unique_review_identity() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));

    let first = store
        .create_workspace_session(create_input(WorkspaceType::Story, None))
        .unwrap();
    let second = store
        .create_workspace_session(create_input(WorkspaceType::Story, None))
        .unwrap();

    assert!(
        !first.review_identity_id.is_empty(),
        "新会话必须携带非空评审身份"
    );
    assert!(
        !second.review_identity_id.is_empty(),
        "新会话必须携带非空评审身份"
    );
    assert_ne!(
        first.review_identity_id, second.review_identity_id,
        "两会话评审身份必须互不相同"
    );
}

/// REQ-ENV-13（设计 A 条目 17）：存量会话（旧 JSON 无 review_identity_id
/// 字段）首次读取时 MUST 补生成一个身份并持久化；二次读取 MUST 复用
/// 同一持久值，MUST NOT 重新生成。
#[test]
fn legacy_session_without_review_identity_gets_one_persisted_once() {
    fn find_session_json(dir: &std::path::Path, file_name: &str) -> std::path::PathBuf {
        for entry in std::fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                if let Some(found) = find_session_json_opt(&path, file_name) {
                    return found;
                }
            } else if path.file_name().is_some_and(|name| name == file_name) {
                return path;
            }
        }
        panic!("session json {file_name} not found");
    }
    fn find_session_json_opt(dir: &std::path::Path, file_name: &str) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                if let Some(found) = find_session_json_opt(&path, file_name) {
                    return Some(found);
                }
            } else if path.file_name().is_some_and(|name| name == file_name) {
                return Some(path);
            }
        }
        None
    }

    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let created = store
        .create_workspace_session(create_input(WorkspaceType::Story, None))
        .unwrap();

    // 还原存量旧 JSON 形态：剥掉 review_identity_id 字段再落盘。
    let session_path = find_session_json(temp.path(), &format!("{}.json", created.id));
    let mut raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&session_path).unwrap()).unwrap();
    assert!(
        raw.as_object_mut()
            .unwrap()
            .remove("review_identity_id")
            .is_some(),
        "前置：创建即写入的字段应存在，剥除后才构成存量形态"
    );
    std::fs::write(&session_path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

    // 首次读取：补生成身份并持久化。
    let first = store.get_workspace_session(&created.id).unwrap();
    assert!(
        !first.review_identity_id.is_empty(),
        "存量会话首读必须补生成评审身份"
    );
    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&session_path).unwrap()).unwrap();
    assert_eq!(
        persisted["review_identity_id"].as_str(),
        Some(first.review_identity_id.as_str()),
        "首读补生成的身份必须落盘持久化"
    );

    // 二次读取：复用同一持久值，不重新生成。
    let second = store.get_workspace_session(&created.id).unwrap();
    assert_eq!(
        first.review_identity_id, second.review_identity_id,
        "二次读取必须复用持久化的评审身份"
    );
}
