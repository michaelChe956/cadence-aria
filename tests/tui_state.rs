use cadence_aria::tui::state::{ActionInputMode, TuiAction, TuiState, TuiTab};

#[test]
fn tui_state_switches_tabs_and_selects_timeline_entries() {
    let mut state = TuiState::default();
    state.apply(TuiAction::SwitchTab(TuiTab::Timeline));
    state.apply(TuiAction::SelectTimelineIndex(3));

    assert_eq!(state.active_tab, TuiTab::Timeline);
    assert_eq!(state.selected_timeline_index, Some(3));
}

#[test]
fn tui_state_edits_action_input_for_pending_provider_step() {
    let mut state = TuiState::default();
    state.apply(TuiAction::SetActionInputMode(
        ActionInputMode::ProviderPrompt,
    ));
    state.apply(TuiAction::ReplaceActionInput("执行 N16".to_string()));
    state.apply(TuiAction::AppendActionInput(
        "\n补充：只改 src/ 和 tests/".to_string(),
    ));

    assert_eq!(state.action_input_mode, ActionInputMode::ProviderPrompt);
    assert!(state.action_input.contains("执行 N16"));
    assert!(state.action_input.contains("只改 src/ 和 tests/"));
}

#[test]
fn tui_state_opens_and_closes_rollback_confirmation() {
    let mut state = TuiState::default();
    state.apply(TuiAction::OpenRollbackConfirmation("ckpt_0001".to_string()));
    assert_eq!(
        state.pending_rollback_checkpoint.as_deref(),
        Some("ckpt_0001")
    );
    state.apply(TuiAction::CloseRollbackConfirmation);
    assert!(state.pending_rollback_checkpoint.is_none());
}
