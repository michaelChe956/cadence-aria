mod fixtures;
mod git;
mod provider;
mod socket;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;

pub use fixtures::{
    CodingRoleRunFixtureRequest, seed_coding_role_run_fixture, seed_large_workspace_fixture,
};
pub use provider::{
    PermissionFixtureRequest, PermissionTimeoutRequest, TestControlledFakeStreamingProvider,
    enable_permission_fixture, enable_review_fixture, enable_testing_fixture,
    set_permission_timeout,
};
pub use socket::{
    WsRejectRequest, WsTimeoutRequest, drop_workspace_socket, reject_next_workspace_sockets,
    set_ws_timeout,
};

#[cfg(test)]
use fixtures::create_large_workspace_fixture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceSocketControl {
    CloseForTestDrop,
}

#[derive(Clone, Default)]
pub struct TestControls {
    inner: Arc<TestControlsInner>,
}

#[derive(Default)]
struct TestControlsInner {
    workspace_sockets: Mutex<HashMap<String, Vec<mpsc::Sender<WorkspaceSocketControl>>>>,
    workspace_socket_rejects: Mutex<HashMap<String, u32>>,
    permission_fixture_sessions: Mutex<HashSet<String>>,
    testing_fixture_sessions: Mutex<HashMap<String, TestingFixtureState>>,
    review_fixture_sessions: Mutex<HashMap<String, VecDeque<ReviewFixture>>>,
    permission_timeout: Mutex<Option<Duration>>,
    server_idle_timeout: Mutex<Option<Duration>>,
}

pub fn test_controls_enabled() -> bool {
    std::env::var("ARIA_E2E_TEST_CONTROLS").as_deref() == Ok("1")
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewFixture {
    pub verdict: String,
    pub summary: String,
    pub comments: String,
    #[serde(default)]
    pub raw_json: Option<Value>,
    #[serde(default)]
    pub raw_text: Option<String>,
    #[serde(default)]
    pub findings: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestingFixture {
    pub plan_output: Value,
    #[serde(default)]
    pub step_results: Vec<Value>,
    #[serde(default)]
    pub malformed_plan_output: Option<String>,
    #[serde(default)]
    pub provider_failure: Option<String>,
}

#[derive(Debug, Clone)]
struct TestingFixtureState {
    fixture: TestingFixture,
    plan_consumed: bool,
}

#[derive(Debug, Clone)]
enum TestingFixtureRun {
    Output(String),
    Failure(String),
}
