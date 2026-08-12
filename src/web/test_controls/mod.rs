mod fixtures;
mod git;
mod plan_repair;
mod provider;
mod socket;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{Notify, mpsc};

pub use crate::product::git_workspace_service::{
    GitCommandPause, pause_next_git_command_after_exit,
};
pub use fixtures::{
    CodingRoleRunFixtureRequest, seed_coding_role_run_fixture, seed_large_workspace_fixture,
};
pub use plan_repair::{
    PlanRepairFaultPoint, PlanRepairFixtureControl, PlanRepairFixtureError,
    PlanRepairFixtureRecovered, PlanRepairFixtureRuntime, PlanRepairFixtureWaiting,
};
pub use provider::{
    PermissionFixtureRequest, PermissionTimeoutRequest, TestControlledFakeStreamingProvider,
    enable_permission_fixture, enable_review_fixture, set_permission_timeout,
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
    review_fixture_sessions: Mutex<HashMap<String, VecDeque<ReviewFixture>>>,
    permission_timeout: Mutex<Option<Duration>>,
    server_idle_timeout: Mutex<Option<Duration>>,
    coding_attempt_acquire_pause: Mutex<Option<CodingAttemptAcquirePause>>,
    group_attempt_acquire_pause: Mutex<Option<CodingAttemptAcquirePause>>,
    coding_attempt_persist_failure: Mutex<bool>,
    group_attempt_initialization_failure: Mutex<Option<GroupAttemptInitializationCheckpoint>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupAttemptInitializationCheckpoint {
    PreparedBeforeAttemptPersisted,
    PersistedBeforeBind,
    BoundBeforePhaseAdvance,
    BoundBeforePlanBinding,
    FirstUnitPersisted,
}

#[derive(Clone, Default)]
pub struct CodingAttemptAcquirePause {
    entered: Arc<Notify>,
    released: Arc<Notify>,
}

impl CodingAttemptAcquirePause {
    pub async fn wait_until_paused(&self) {
        self.entered.notified().await;
    }

    pub fn resume(&self) {
        self.released.notify_one();
    }
}

impl TestControls {
    pub fn fail_next_group_attempt_initialization_at(
        &self,
        checkpoint: GroupAttemptInitializationCheckpoint,
    ) {
        *self
            .inner
            .group_attempt_initialization_failure
            .lock()
            .expect("group attempt initialization failure lock") = Some(checkpoint);
    }

    pub fn consume_group_attempt_initialization_failure(
        &self,
        checkpoint: GroupAttemptInitializationCheckpoint,
    ) -> bool {
        let mut configured = self
            .inner
            .group_attempt_initialization_failure
            .lock()
            .expect("group attempt initialization failure lock");
        if *configured != Some(checkpoint) {
            return false;
        }
        configured.take();
        true
    }

    pub fn fail_next_coding_attempt_after_persist_before_bind(&self) {
        *self
            .inner
            .coding_attempt_persist_failure
            .lock()
            .expect("coding attempt persist failure lock") = true;
    }

    pub fn consume_coding_attempt_after_persist_before_bind_failure(&self) -> bool {
        let mut configured = self
            .inner
            .coding_attempt_persist_failure
            .lock()
            .expect("coding attempt persist failure lock");
        std::mem::take(&mut *configured)
    }

    pub fn pause_next_coding_attempt_after_worktree_acquire(&self) -> CodingAttemptAcquirePause {
        let pause = CodingAttemptAcquirePause::default();
        *self
            .inner
            .coding_attempt_acquire_pause
            .lock()
            .expect("coding attempt acquire pause lock") = Some(pause.clone());
        pause
    }

    pub async fn pause_coding_attempt_after_worktree_acquire_if_configured(&self) {
        let pause = self
            .inner
            .coding_attempt_acquire_pause
            .lock()
            .expect("coding attempt acquire pause lock")
            .take();
        if let Some(pause) = pause {
            pause.entered.notify_one();
            pause.released.notified().await;
        }
    }

    pub fn pause_next_group_attempt_after_worktree_acquire(&self) -> CodingAttemptAcquirePause {
        let pause = CodingAttemptAcquirePause::default();
        *self
            .inner
            .group_attempt_acquire_pause
            .lock()
            .expect("group attempt acquire pause lock") = Some(pause.clone());
        pause
    }

    pub async fn pause_group_attempt_after_worktree_acquire_if_configured(&self) {
        let pause = self
            .inner
            .group_attempt_acquire_pause
            .lock()
            .expect("group attempt acquire pause lock")
            .take();
        if let Some(pause) = pause {
            pause.entered.notify_one();
            pause.released.notified().await;
        }
    }
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
