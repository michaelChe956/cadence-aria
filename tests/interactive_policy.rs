use cadence_aria::interactive::policy::{
    ConfirmationDecision, NodeWriteClass, PolicyPreset, ProviderNodeMeta,
};
use std::str::FromStr;

#[test]
fn manual_all_pauses_every_provider_node() {
    let meta = ProviderNodeMeta::new("N17", "codex", NodeWriteClass::ReadOnly);
    assert_eq!(
        PolicyPreset::ManualAll.decision_for(&meta),
        ConfirmationDecision::PauseForConfirmation
    );
}

#[test]
fn manual_write_pauses_write_nodes_and_runs_readonly_nodes() {
    let write_node = ProviderNodeMeta::new("N16", "codex", NodeWriteClass::WritesWorkspace);
    let runtime_node = ProviderNodeMeta::new("N11", "claude_code", NodeWriteClass::WritesRuntime);
    let review_node = ProviderNodeMeta::new("N18", "codex", NodeWriteClass::ReadOnly);
    assert_eq!(
        PolicyPreset::ManualWrite.decision_for(&write_node),
        ConfirmationDecision::PauseForConfirmation
    );
    assert_eq!(
        PolicyPreset::ManualWrite.decision_for(&runtime_node),
        ConfirmationDecision::PauseForConfirmation
    );
    assert_eq!(
        PolicyPreset::ManualWrite.decision_for(&review_node),
        ConfirmationDecision::RunAutomatically
    );
}

#[test]
fn auto_review_pauses_design_revision_and_integration_prepare_nodes() {
    let design_revision =
        ProviderNodeMeta::new("N12", "claude_code", NodeWriteClass::WritesRuntime);
    let integration_prepare = ProviderNodeMeta::new("N19", "codex", NodeWriteClass::WritesRuntime);

    assert_eq!(
        PolicyPreset::AutoReview.decision_for(&design_revision),
        ConfirmationDecision::PauseForConfirmation
    );
    assert_eq!(
        PolicyPreset::AutoReview.decision_for(&integration_prepare),
        ConfirmationDecision::PauseForConfirmation
    );
}

#[test]
fn auto_review_pauses_planning_and_coding_but_runs_review_and_testing() {
    let planning = ProviderNodeMeta::new("N11", "claude_code", NodeWriteClass::WritesRuntime);
    let coding = ProviderNodeMeta::new("N16", "codex", NodeWriteClass::WritesWorkspace);
    let testing = ProviderNodeMeta::new("N17", "codex", NodeWriteClass::ReadOnly);
    assert_eq!(
        PolicyPreset::AutoReview.decision_for(&planning),
        ConfirmationDecision::PauseForConfirmation
    );
    assert_eq!(
        PolicyPreset::AutoReview.decision_for(&coding),
        ConfirmationDecision::PauseForConfirmation
    );
    assert_eq!(
        PolicyPreset::AutoReview.decision_for(&testing),
        ConfirmationDecision::RunAutomatically
    );
}

#[test]
fn non_interactive_never_pauses() {
    let write_node = ProviderNodeMeta::new("N16", "codex", NodeWriteClass::WritesWorkspace);
    assert_eq!(
        PolicyPreset::NonInteractive.decision_for(&write_node),
        ConfirmationDecision::RunAutomatically
    );
}

#[test]
fn policy_preset_parses_cli_names() {
    assert_eq!(
        PolicyPreset::from_str("manual-all").expect("manual all"),
        PolicyPreset::ManualAll
    );
    assert_eq!(
        PolicyPreset::from_str("manual-write").expect("manual write"),
        PolicyPreset::ManualWrite
    );
    assert_eq!(
        PolicyPreset::from_str("auto-review").expect("auto review"),
        PolicyPreset::AutoReview
    );
    assert_eq!(
        PolicyPreset::from_str("non-interactive").expect("non interactive"),
        PolicyPreset::NonInteractive
    );
    assert!(PolicyPreset::from_str("unknown").is_err());
}
