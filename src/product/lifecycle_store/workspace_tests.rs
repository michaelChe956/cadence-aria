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
