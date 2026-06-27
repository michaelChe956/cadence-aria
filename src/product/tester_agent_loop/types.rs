use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use crate::cross_cutting::provider_adapter::DEFAULT_PROVIDER_TIMEOUT_SECS;
use crate::cross_cutting::streaming_provider::ProviderToolResult;
use crate::product::coding_models::{TestCommand, TestPlanStep};
use crate::product::test_executor::TestExecutorError;

pub const TESTER_TOOL_FAILURE_LIMIT: usize = 3;

pub(crate) const MAX_LISTED_FILES: usize = 200;
pub(crate) const MAX_SEARCH_MATCHES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesterAgentOptions {
    pub timeout: Duration,
    pub failure_limit: usize,
}

impl Default for TesterAgentOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_PROVIDER_TIMEOUT_SECS),
            failure_limit: TESTER_TOOL_FAILURE_LIMIT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesterToolOutcome {
    pub result: ProviderToolResult,
    pub command: Option<TestCommand>,
}

#[derive(Debug, Error)]
pub enum TesterAgentError {
    #[error("tester tool failed: {0}")]
    Tool(String),
    #[error("tester plan invalid: {0}")]
    Plan(String),
    #[error(transparent)]
    TestExecutor(#[from] TestExecutorError),
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ProviderTestPlanPayload {
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) context_warnings: Vec<String>,
    #[serde(default)]
    pub(crate) assumptions: Vec<String>,
    pub(crate) steps: Vec<TestPlanStep>,
}
