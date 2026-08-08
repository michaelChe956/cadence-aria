use super::*;
use crate::product::coding_models::{
    CodingProviderPermissionMode, CodingRolePermissionModes, CodingRoleProviderConfigSnapshot,
};

#[tokio::test]
async fn coder_rework_normalizes_pi_supervised_mode_to_auto() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Pi,
                code_reviewer: ProviderName::ClaudeCode,
                internal_reviewer: ProviderName::ClaudeCode,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes {
                    coder: CodingProviderPermissionMode::Supervised,
                    code_reviewer: CodingProviderPermissionMode::Auto,
                    internal_reviewer: CodingProviderPermissionMode::Auto,
                },
            },
        )
        .expect("set Pi role config");

    let provider = provider_driven::ReviewerDrivenReworkProvider::default();
    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    engine
        .execute_coder_fix_from_review(
            &attempt,
            &provider_driven::review_report_requesting_changes(&attempt),
            &CodingExecutionContext::default(),
            &provider,
            &mut command_rx,
        )
        .await
        .expect("execute Pi coder rework");

    let input = provider.recorded_input();
    assert_eq!(input.provider_type, ProviderType::Pi);
    assert_eq!(
        input.permission_mode,
        ProviderPermissionMode::Auto,
        "Pi coder rework must normalize persisted Supervised to Auto"
    );
}
