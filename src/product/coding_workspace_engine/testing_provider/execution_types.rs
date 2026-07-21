use super::*;

pub(crate) enum ProviderTestingExecutionOutcome {
    EarlyReport(Box<TestingReport>),
    Completed(ProviderTestingExecutionPhase),
}

pub(crate) struct ProviderTestingExecutionPhase {
    pub(crate) full_output: String,
    pub(crate) step_results: Vec<TestingStepResult>,
    pub(crate) unplanned_commands: Vec<TestCommand>,
    pub(crate) unplanned_evidence: Vec<TestingUnplannedEvidence>,
    pub(crate) context_warnings: Vec<String>,
    pub(crate) blocked_summary: Option<String>,
    pub(crate) blocked_reason_code: Option<String>,
    pub(crate) chat_entry_sequence: usize,
}

pub(crate) struct ProviderTestingExecutionInput<'a> {
    pub(crate) attempt: CodingExecutionAttempt,
    pub(crate) node: CodingTimelineNode,
    pub(crate) role_run: CodingRoleRun,
    pub(crate) provider: &'a dyn StreamingProviderAdapter,
    pub(crate) worktree_path: PathBuf,
    pub(crate) tester_provider: ProviderName,
    pub(crate) plan: TestPlan,
    pub(crate) chat_entry_sequence: usize,
    pub(crate) options: &'a TesterAgentOptions,
    pub(crate) command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
}
