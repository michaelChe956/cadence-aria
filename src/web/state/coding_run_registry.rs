use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, mpsc};

use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_workspace_runner::CodingRunnerCommand;

#[derive(Clone, Default)]
pub struct CodingRunRegistry {
    inner: Arc<StdMutex<CodingRunRegistryInner>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodingAttemptRunKey {
    project_id: String,
    issue_id: String,
    attempt_id: String,
}

impl CodingAttemptRunKey {
    pub fn new(
        project_id: impl Into<String>,
        issue_id: impl Into<String>,
        attempt_id: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            issue_id: issue_id.into(),
            attempt_id: attempt_id.into(),
        }
    }

    pub fn from_attempt(attempt: &CodingExecutionAttempt) -> Self {
        Self::new(&attempt.project_id, &attempt.issue_id, &attempt.id)
    }
}

#[derive(Default)]
struct CodingRunRegistryInner {
    next_run_id: u64,
    runs: HashMap<CodingAttemptRunKey, HashMap<u64, mpsc::Sender<CodingRunnerCommand>>>,
    reservations: HashMap<CodingAttemptRunKey, u64>,
    exclusive_runs: HashMap<CodingAttemptRunKey, u64>,
    attempt_guards: HashMap<CodingAttemptRunKey, Arc<AsyncMutex<()>>>,
    attempt_mutation_guards: HashMap<CodingAttemptRunKey, Arc<AsyncMutex<()>>>,
    named_guards: HashMap<String, Arc<AsyncMutex<()>>>,
}

pub(crate) struct CodingAttemptMutationLease {
    _guard: OwnedMutexGuard<()>,
}

pub struct CodingRunReservation {
    registry: CodingRunRegistry,
    attempt_key: CodingAttemptRunKey,
    reservation_id: u64,
    released: bool,
}

impl CodingRunReservation {
    pub fn activate(mut self, command_tx: mpsc::Sender<CodingRunnerCommand>) -> Option<u64> {
        let mut inner = self
            .registry
            .inner
            .lock()
            .expect("coding run registry lock");
        if inner.reservations.get(&self.attempt_key) != Some(&self.reservation_id) {
            return None;
        }
        inner.reservations.remove(&self.attempt_key);
        inner
            .runs
            .entry(self.attempt_key.clone())
            .or_default()
            .insert(self.reservation_id, command_tx);
        inner
            .exclusive_runs
            .insert(self.attempt_key.clone(), self.reservation_id);
        self.released = true;
        Some(self.reservation_id)
    }

    pub fn release(mut self) {
        self.registry
            .release_reservation(&self.attempt_key, self.reservation_id);
        self.released = true;
    }
}

impl Drop for CodingRunReservation {
    fn drop(&mut self) {
        if !self.released {
            self.registry
                .release_reservation(&self.attempt_key, self.reservation_id);
        }
    }
}

impl CodingRunRegistry {
    pub fn insert(
        &self,
        attempt_key: &CodingAttemptRunKey,
        command_tx: mpsc::Sender<CodingRunnerCommand>,
    ) -> Option<u64> {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner.reservations.contains_key(attempt_key)
            || inner.exclusive_runs.contains_key(attempt_key)
        {
            return None;
        }
        inner.next_run_id += 1;
        let run_id = inner.next_run_id;
        inner
            .runs
            .entry(attempt_key.clone())
            .or_default()
            .insert(run_id, command_tx);
        Some(run_id)
    }

    pub fn remove(&self, attempt_key: &CodingAttemptRunKey, run_id: u64) {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner.exclusive_runs.get(attempt_key) == Some(&run_id) {
            inner.exclusive_runs.remove(attempt_key);
        }
        if let Some(runs) = inner.runs.get_mut(attempt_key) {
            runs.remove(&run_id);
            if runs.is_empty() {
                inner.runs.remove(attempt_key);
            }
        }
    }

    pub async fn abort_attempt(&self, attempt_key: &CodingAttemptRunKey) -> usize {
        let senders = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            inner.exclusive_runs.remove(attempt_key);
            inner
                .runs
                .remove(attempt_key)
                .map(|runs| runs.into_values().collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let mut sent = 0;
        for sender in senders {
            if sender.send(CodingRunnerCommand::AbortAttempt).await.is_ok() {
                sent += 1;
            }
        }
        sent
    }

    pub fn runner_count(&self, attempt_key: &CodingAttemptRunKey) -> usize {
        self.inner
            .lock()
            .expect("coding run registry lock")
            .runs
            .get(attempt_key)
            .map(HashMap::len)
            .unwrap_or(0)
    }

    pub fn try_reserve_attempt(
        &self,
        attempt_key: &CodingAttemptRunKey,
    ) -> Option<CodingRunReservation> {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner
            .runs
            .get(attempt_key)
            .is_some_and(|runs| !runs.is_empty())
            || inner.reservations.contains_key(attempt_key)
        {
            return None;
        }
        inner.next_run_id += 1;
        let reservation_id = inner.next_run_id;
        inner
            .reservations
            .insert(attempt_key.clone(), reservation_id);
        Some(CodingRunReservation {
            registry: self.clone(),
            attempt_key: attempt_key.clone(),
            reservation_id,
            released: false,
        })
    }

    pub async fn lock_attempt(&self, attempt_key: &CodingAttemptRunKey) -> OwnedMutexGuard<()> {
        let guard = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            Arc::clone(
                inner
                    .attempt_guards
                    .entry(attempt_key.clone())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        guard.lock_owned().await
    }

    pub async fn lock_named(&self, name: &str) -> OwnedMutexGuard<()> {
        let guard = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            Arc::clone(
                inner
                    .named_guards
                    .entry(name.to_string())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        guard.lock_owned().await
    }

    pub(crate) async fn lock_attempt_mutation(
        &self,
        attempt_key: &CodingAttemptRunKey,
    ) -> CodingAttemptMutationLease {
        let guard = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            Arc::clone(
                inner
                    .attempt_mutation_guards
                    .entry(attempt_key.clone())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        CodingAttemptMutationLease {
            _guard: guard.lock_owned().await,
        }
    }

    pub fn has_active_recovery_reservation(&self, attempt_key: &CodingAttemptRunKey) -> bool {
        self.inner
            .lock()
            .expect("coding run registry lock")
            .reservations
            .contains_key(attempt_key)
    }

    pub fn attempt_is_reserved_or_running(&self, attempt_key: &CodingAttemptRunKey) -> bool {
        let inner = self.inner.lock().expect("coding run registry lock");
        inner.reservations.contains_key(attempt_key)
            || inner
                .runs
                .get(attempt_key)
                .is_some_and(|runs| !runs.is_empty())
    }

    fn release_reservation(&self, attempt_key: &CodingAttemptRunKey, reservation_id: u64) {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner.reservations.get(attempt_key) == Some(&reservation_id) {
            inner.reservations.remove(attempt_key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn aborts_all_runs_for_attempt_and_removes_them() {
        let registry = CodingRunRegistry::default();
        let attempt = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
        let other = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0002");
        let (first_tx, mut first_rx) = mpsc::channel(1);
        let (second_tx, mut second_rx) = mpsc::channel(1);
        let (other_tx, mut other_rx) = mpsc::channel(1);

        registry.insert(&attempt, first_tx).expect("first runner");
        registry.insert(&attempt, second_tx).expect("second runner");
        registry.insert(&other, other_tx).expect("other runner");

        assert_eq!(registry.runner_count(&attempt), 2);
        assert_eq!(registry.abort_attempt(&attempt).await, 2);
        assert_eq!(registry.runner_count(&attempt), 0);
        assert_eq!(registry.runner_count(&other), 1);
        assert_eq!(
            first_rx.recv().await.expect("first abort"),
            CodingRunnerCommand::AbortAttempt
        );
        assert_eq!(
            second_rx.recv().await.expect("second abort"),
            CodingRunnerCommand::AbortAttempt
        );
        assert!(other_rx.try_recv().is_err());
    }

    #[test]
    fn scopes_reservations_by_attempt_identity() {
        let registry = CodingRunRegistry::default();
        let first = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
        let second = CodingAttemptRunKey::new("project_0001", "issue_0002", "coding_attempt_0001");

        let first_reservation = registry
            .try_reserve_attempt(&first)
            .expect("first reservation");
        let second_reservation = registry
            .try_reserve_attempt(&second)
            .expect("second scoped reservation");

        assert!(registry.has_active_recovery_reservation(&first));
        assert!(registry.has_active_recovery_reservation(&second));
        first_reservation.release();
        second_reservation.release();
    }

    #[tokio::test]
    async fn aborts_only_exact_attempt_identity() {
        let registry = CodingRunRegistry::default();
        let first = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
        let second = CodingAttemptRunKey::new("project_0001", "issue_0002", "coding_attempt_0001");
        let (first_tx, mut first_rx) = mpsc::channel(1);
        let (second_tx, mut second_rx) = mpsc::channel(1);
        registry.insert(&first, first_tx).expect("first runner");
        registry.insert(&second, second_tx).expect("second runner");

        assert_eq!(registry.abort_attempt(&second).await, 1);
        assert_eq!(registry.runner_count(&first), 1);
        assert_eq!(registry.runner_count(&second), 0);
        assert!(first_rx.try_recv().is_err());
        assert_eq!(
            second_rx.recv().await.expect("second abort"),
            CodingRunnerCommand::AbortAttempt
        );
    }
}
