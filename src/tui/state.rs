#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuiTab {
    #[default]
    Overview,
    Timeline,
    Io,
    Artifacts,
    Changes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionInputMode {
    #[default]
    Idle,
    ProviderPrompt,
    RollbackConfirm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiState {
    pub active_tab: TuiTab,
    pub selected_timeline_index: Option<usize>,
    pub action_input: String,
    pub action_input_mode: ActionInputMode,
    pub pending_rollback_checkpoint: Option<String>,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            active_tab: TuiTab::Overview,
            selected_timeline_index: None,
            action_input: String::new(),
            action_input_mode: ActionInputMode::Idle,
            pending_rollback_checkpoint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    SwitchTab(TuiTab),
    SelectTimelineIndex(usize),
    SetActionInputMode(ActionInputMode),
    ReplaceActionInput(String),
    AppendActionInput(String),
    OpenRollbackConfirmation(String),
    CloseRollbackConfirmation,
}

impl TuiState {
    pub fn apply(&mut self, action: TuiAction) {
        match action {
            TuiAction::SwitchTab(tab) => self.active_tab = tab,
            TuiAction::SelectTimelineIndex(index) => self.selected_timeline_index = Some(index),
            TuiAction::SetActionInputMode(mode) => self.action_input_mode = mode,
            TuiAction::ReplaceActionInput(value) => self.action_input = value,
            TuiAction::AppendActionInput(value) => self.action_input.push_str(&value),
            TuiAction::OpenRollbackConfirmation(checkpoint_id) => {
                self.action_input_mode = ActionInputMode::RollbackConfirm;
                self.pending_rollback_checkpoint = Some(checkpoint_id);
            }
            TuiAction::CloseRollbackConfirmation => {
                self.action_input_mode = ActionInputMode::Idle;
                self.pending_rollback_checkpoint = None;
            }
        }
    }
}
