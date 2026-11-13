// P08「当前 operation 归属」RED 契约：显式 fresh 的**新** operation 受理
// 时，root 上只属于当前 operation 的物理镜像（node/seq/native）必须清空，
// 避免新 operation 复用旧物理 run；旧 operation 的历史事实（fresh_operations
// 逐字段、状态、会话引用、launch 指纹、SC ledger/phase/教学计数）原样保留
// （旧 Unknown 不被改 Failed）。
// 落位：src/product/lifecycle_store/tests/fresh_operation_binding_tests.rs
// （由 workspace.rs 以 #[path] 挂载；父上下文仍是 workspace.rs 模块，super::* 语义不变）。

use super::*;
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::{
    LcRecoveryBusinessContext, LcRecoveryRunContext, ProviderConversationRole, ProviderName,
};

const RECOVERY_ID: &str = "recovery_fresh_binding_0001";
const OLD_NODE_ID: &str = "timeline_node_old_0001";
const OLD_NATIVE_ID: &str = "native_old_0001";

fn session_fixture() -> (tempfile::TempDir, LifecycleStore, WorkspaceSessionRecord) {
    let tmp = tempfile::tempdir().unwrap();
    let store = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
        tmp.path().join(".aria"),
    ));
    let record = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("create workspace session");
    (tmp, store, record)
}

/// 旧 operation（Unknown）+ root 当前镜像（node/seq/native）的等待态。
fn waiting_recovery_with_old_operation(
    record: &WorkspaceSessionRecord,
) -> LcResumeRecoveryRecord {
    let mut old_operation = LcFreshOperationResult {
        operation_id: "op_old_0001".to_string(),
        state: LcFreshOperationState::Unknown,
        fresh_node_id: Some(OLD_NODE_ID.to_string()),
        reason_code: Some("lc_resume_round_launch_unknown".to_string()),
        role_run_seq: Some(7),
        accepted_at: "2026-11-13T00:00:00Z".to_string(),
        updated_at: "2026-11-13T00:00:00Z".to_string(),
    };
    old_operation.updated_at = "2026-11-13T00:00:00Z".to_string();
    LcResumeRecoveryRecord {
        schema_version: 1,
        recovery_id: RECOVERY_ID.to_string(),
        failed_provider_node_id: OLD_NODE_ID.to_string(),
        reason_code: "resume_fingerprint_mismatch".to_string(),
        role: ProviderConversationRole::Author,
        provider: record.author_provider.clone(),
        superseded_native_session_id: Some(OLD_NATIVE_ID.to_string()),
        previous_fingerprint_digest: Some("sha256:old".to_string()),
        previous_session_status: record.status.clone(),
        previous_stage: "revision".to_string(),
        run_context: LcRecoveryRunContext::ReviewerRevision {
            business: LcRecoveryBusinessContext {
                entity_id: record.entity_id.clone(),
                plan_id: None,
                workspace_type: record.workspace_type.clone(),
                business_repository_physical_id: None,
                business_repository_logical_id: None,
                business_checkout_id: None,
                business_repository_path: None,
                source_story_ids: Vec::new(),
                source_design_ids: Vec::new(),
                plan_options_ref: None,
                artifact_refs: Vec::new(),
                review_refs: Vec::new(),
                author_provider: record.author_provider.clone(),
                effective_reviewer_provider: record.reviewer_provider.clone(),
                original_node_id: Some(OLD_NODE_ID.to_string()),
                original_role_run_seq: None,
            },
            original_feedback: None,
        },
        state: LcResumeRecoveryState::Waiting,
        operation_id: None,
        fresh_node_id: Some(OLD_NODE_ID.to_string()),
        fresh_native_session_id: Some(OLD_NATIVE_ID.to_string()),
        fresh_role_run_seq: Some(7),
        fresh_operations: vec![old_operation],
        detected_at: "2026-11-13T00:00:00Z".to_string(),
        updated_at: "2026-11-13T00:00:00Z".to_string(),
    }
}

/// RED①：新 operation 受理后，root 当前镜像三字段清空；旧 operation
/// 历史逐字段不变（Unknown 不被改 Failed）；指纹/会话引用/SC ledger/
/// phase/教学计数零改动。
#[test]
fn fresh_operation_binding_new_claim_clears_current_operation_mirror() {
    let (_tmp, store, record) = session_fixture();
    let recovery = waiting_recovery_with_old_operation(&record);
    let old_operation = recovery.fresh_operations[0].clone();
    store
        .compare_and_save_lc_resume_state(&record, recovery)
        .expect("seed waiting recovery");

    let outcome = store
        .claim_lc_fresh(&record.id, RECOVERY_ID, "op_new_0001")
        .expect("claim new operation");
    assert!(
        matches!(outcome, LcFreshClaimOutcome::Claimed { .. }),
        "新 operation 必须被受理，实际: {outcome:?}"
    );

    let after = store
        .get_workspace_session(&record.id)
        .expect("record after claim");
    let current = after
        .lc_resume_recovery
        .as_ref()
        .expect("recovery kept after claim");
    assert_eq!(current.state, LcResumeRecoveryState::FreshReserved);
    assert_eq!(current.operation_id.as_deref(), Some("op_new_0001"));
    // 当前 operation 镜像：旧物理 node/seq/native 不得投影到新 operation。
    assert_eq!(
        current.fresh_node_id, None,
        "新 operation 不得继承旧 fresh node 镜像"
    );
    assert_eq!(
        current.fresh_role_run_seq, None,
        "新 operation 不得继承旧 fresh role_run_seq 镜像"
    );
    assert_eq!(
        current.fresh_native_session_id, None,
        "新 operation 不得继承旧 native 镜像"
    );
    // 历史 operation 逐字段原样（Unknown 不被改 Failed）。
    assert_eq!(current.fresh_operations.len(), 2);
    assert_eq!(
        current.fresh_operations[0], old_operation,
        "旧 operation 历史必须逐字段原样保留"
    );
    assert_eq!(
        current.fresh_operations[0].state,
        LcFreshOperationState::Unknown,
        "旧 Unknown operation 不得被改 Failed"
    );
    let new_operation = &current.fresh_operations[1];
    assert_eq!(new_operation.operation_id, "op_new_0001");
    assert_eq!(new_operation.state, LcFreshOperationState::Reserved);
    assert_eq!(new_operation.fresh_node_id, None);
    assert_eq!(new_operation.role_run_seq, None);
    // 非 LC 面零改动：launch 指纹/会话引用/SC ledger/phase/教学计数。
    assert!(after.logical_launch_fingerprints.is_empty());
    assert!(after.provider_conversations.is_empty());
    assert!(after.provider_start_ledger.is_empty());
    assert!(after.sc_generation_teaching_windows.is_empty());
    assert!(after.single_candidate_phase.is_none());
}

/// RED② 守卫：同 operation_id 的 Replay 不写盘（记录 bytes 不变），
/// 不清当前镜像、不新增 operation/seq。
#[test]
fn fresh_operation_binding_same_operation_replay_keeps_record_bytes() {
    let (_tmp, store, record) = session_fixture();
    let recovery = waiting_recovery_with_old_operation(&record);
    store
        .compare_and_save_lc_resume_state(&record, recovery)
        .expect("seed waiting recovery");

    let path = store
        .workspace_sessions_root(&record.project_id, &record.issue_id)
        .join(format!("{}.json", record.id));
    let before = std::fs::read(&path).expect("read record bytes before replay");
    let outcome = store
        .claim_lc_fresh(&record.id, RECOVERY_ID, "op_old_0001")
        .expect("replay same operation");
    assert!(
        matches!(outcome, LcFreshClaimOutcome::Replay { .. }),
        "同 operation_id 必须回放原受理结果，实际: {outcome:?}"
    );
    let after = std::fs::read(&path).expect("read record bytes after replay");
    assert_eq!(before, after, "Replay 不得改写记录字节");
    let current = store
        .get_workspace_session(&record.id)
        .expect("record after replay")
        .lc_resume_recovery
        .expect("recovery kept after replay");
    assert_eq!(
        current.fresh_node_id.as_deref(),
        Some(OLD_NODE_ID),
        "Replay 不是新受理，当前镜像不得被清"
    );
    assert_eq!(current.fresh_role_run_seq, Some(7));
    assert_eq!(current.fresh_native_session_id.as_deref(), Some(OLD_NATIVE_ID));
    assert_eq!(
        current.fresh_operations.len(),
        1,
        "Replay 不得新增 operation"
    );
}
