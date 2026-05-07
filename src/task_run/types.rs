use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderMode {
    Real,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportMode {
    Text,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRunOptions {
    pub workspace: PathBuf,
    pub request_text: String,
    pub change_id: Option<String>,
    pub provider_mode: ProviderMode,
    pub non_interactive: bool,
    pub timeout_secs: u64,
    pub report_mode: ReportMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRunRequest {
    pub workspace: PathBuf,
    pub request_text: String,
    pub change_id: String,
    pub provider_mode: ProviderMode,
    pub non_interactive: bool,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRunStatus {
    Completed,
    Failed,
    BlockedByGate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRunOutcome {
    pub task_id: String,
    pub change_id: String,
    pub status: TaskRunStatus,
    pub report_path: PathBuf,
    pub openspec_change_dir: PathBuf,
    pub provider_run_refs: Vec<String>,
    pub testing_report_path: Option<PathBuf>,
    pub final_summary_path: Option<PathBuf>,
    pub blocked_report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRunError {
    pub code: String,
    pub message: String,
}

impl TaskRunError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
