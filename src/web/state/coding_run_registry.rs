use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, mpsc, watch};
use tokio_util::sync::CancellationToken;

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
    runs: HashMap<CodingAttemptRunKey, HashMap<u64, CodingRunEntry>>,
    reservations: HashMap<CodingAttemptRunKey, u64>,
    exclusive_runs: HashMap<CodingAttemptRunKey, u64>,
    retired_attempts: HashSet<CodingAttemptRunKey>,
    attempt_guards: HashMap<CodingAttemptRunKey, Arc<AsyncMutex<()>>>,
    attempt_mutation_guards: HashMap<CodingAttemptRunKey, Arc<AsyncMutex<()>>>,
    named_guards: HashMap<String, Arc<AsyncMutex<()>>>,
}

struct CodingRunEntry {
    command_tx: mpsc::Sender<CodingRunnerCommand>,
    completion_tx: watch::Sender<bool>,
    cancellation: Option<CancellationToken>,
}

pub(crate) struct CodingRunRegistration {
    pub(crate) run_id: u64,
    pub(crate) cancellation: CancellationToken,
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
        if inner.retired_attempts.contains(&self.attempt_key)
            || inner.reservations.get(&self.attempt_key) != Some(&self.reservation_id)
        {
            return None;
        }
        inner.reservations.remove(&self.attempt_key);
        let (completion_tx, _completion_rx) = watch::channel(false);
        inner
            .runs
            .entry(self.attempt_key.clone())
            .or_default()
            .insert(
                self.reservation_id,
                CodingRunEntry {
                    command_tx,
                    completion_tx,
                    cancellation: None,
                },
            );
        inner
            .exclusive_runs
            .insert(self.attempt_key.clone(), self.reservation_id);
        self.released = true;
        Some(self.reservation_id)
    }

    pub(crate) fn activate_cancellable(
        mut self,
        command_tx: mpsc::Sender<CodingRunnerCommand>,
    ) -> Option<CodingRunRegistration> {
        let mut inner = self
            .registry
            .inner
            .lock()
            .expect("coding run registry lock");
        if inner.retired_attempts.contains(&self.attempt_key)
            || inner.reservations.get(&self.attempt_key) != Some(&self.reservation_id)
        {
            return None;
        }
        inner.reservations.remove(&self.attempt_key);
        let cancellation = CancellationToken::new();
        let (completion_tx, _completion_rx) = watch::channel(false);
        inner
            .runs
            .entry(self.attempt_key.clone())
            .or_default()
            .insert(
                self.reservation_id,
                CodingRunEntry {
                    command_tx,
                    completion_tx,
                    cancellation: Some(cancellation.clone()),
                },
            );
        inner
            .exclusive_runs
            .insert(self.attempt_key.clone(), self.reservation_id);
        self.released = true;
        Some(CodingRunRegistration {
            run_id: self.reservation_id,
            cancellation,
        })
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
        if inner.retired_attempts.contains(attempt_key)
            || inner.reservations.contains_key(attempt_key)
            || inner.exclusive_runs.contains_key(attempt_key)
        {
            return None;
        }
        inner.next_run_id += 1;
        let run_id = inner.next_run_id;
        let (completion_tx, _completion_rx) = watch::channel(false);
        inner.runs.entry(attempt_key.clone()).or_default().insert(
            run_id,
            CodingRunEntry {
                command_tx,
                completion_tx,
                cancellation: None,
            },
        );
        Some(run_id)
    }

    pub(crate) fn insert_cancellable(
        &self,
        attempt_key: &CodingAttemptRunKey,
        command_tx: mpsc::Sender<CodingRunnerCommand>,
    ) -> Option<CodingRunRegistration> {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner.retired_attempts.contains(attempt_key)
            || inner.reservations.contains_key(attempt_key)
            || inner.exclusive_runs.contains_key(attempt_key)
        {
            return None;
        }
        inner.next_run_id += 1;
        let run_id = inner.next_run_id;
        let cancellation = CancellationToken::new();
        let (completion_tx, _completion_rx) = watch::channel(false);
        inner.runs.entry(attempt_key.clone()).or_default().insert(
            run_id,
            CodingRunEntry {
                command_tx,
                completion_tx,
                cancellation: Some(cancellation.clone()),
            },
        );
        Some(CodingRunRegistration {
            run_id,
            cancellation,
        })
    }

    pub fn remove(&self, attempt_key: &CodingAttemptRunKey, run_id: u64) {
        let mut inner = self.inner.lock().expect("coding run registry lock");
        if inner.exclusive_runs.get(attempt_key) == Some(&run_id) {
            inner.exclusive_runs.remove(attempt_key);
        }
        if let Some(runs) = inner.runs.get_mut(attempt_key) {
            if let Some(entry) = runs.remove(&run_id) {
                entry.completion_tx.send_replace(true);
            }
            if runs.is_empty() {
                inner.runs.remove(attempt_key);
            }
        }
    }

    pub async fn abort_attempt(&self, attempt_key: &CodingAttemptRunKey) -> usize {
        let runners = {
            let mut inner = self.inner.lock().expect("coding run registry lock");
            inner.retired_attempts.insert(attempt_key.clone());
            inner.reservations.remove(attempt_key);
            inner
                .runs
                .get(attempt_key)
                .map(|runs| {
                    runs.iter()
                        .map(|(run_id, entry)| {
                            (
                                *run_id,
                                entry.command_tx.clone(),
                                entry.cancellation.clone(),
                                entry.completion_tx.subscribe(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let mut sent = 0;
        for (_, _, cancellation, _) in &runners {
            if let Some(cancellation) = cancellation {
                cancellation.cancel();
                sent += 1;
            }
        }
        for (run_id, sender, cancellation, _) in &runners {
            if cancellation.is_some() {
                continue;
            }
            match sender.try_send(CodingRunnerCommand::AbortAttempt) {
                Ok(()) => sent += 1,
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    self.remove(attempt_key, *run_id);
                }
                Err(mpsc::error::TrySendError::Full(_)) => {}
            }
        }
        for (_, _, _, mut completion_rx) in runners {
            while !*completion_rx.borrow() {
                if completion_rx.changed().await.is_err() {
                    break;
                }
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
        if inner.retired_attempts.contains(attempt_key)
            || inner
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

        let first_run_id = registry.insert(&attempt, first_tx).expect("first runner");
        let second_run_id = registry.insert(&attempt, second_tx).expect("second runner");
        registry.insert(&other, other_tx).expect("other runner");

        assert_eq!(registry.runner_count(&attempt), 2);
        let registry_for_abort = registry.clone();
        let attempt_for_abort = attempt.clone();
        let abort =
            tokio::spawn(async move { registry_for_abort.abort_attempt(&attempt_for_abort).await });
        assert_eq!(
            first_rx.recv().await.expect("first abort"),
            CodingRunnerCommand::AbortAttempt
        );
        assert_eq!(
            second_rx.recv().await.expect("second abort"),
            CodingRunnerCommand::AbortAttempt
        );
        tokio::task::yield_now().await;
        assert!(!abort.is_finished(), "abort must wait for runner removal");
        assert_eq!(registry.runner_count(&attempt), 2);
        registry.remove(&attempt, first_run_id);
        assert!(!abort.is_finished(), "abort must wait for every runner");
        registry.remove(&attempt, second_run_id);
        assert_eq!(abort.await.expect("abort task"), 2);
        assert_eq!(registry.runner_count(&attempt), 0);
        assert_eq!(registry.runner_count(&other), 1);
        assert!(other_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn abort_retires_attempt_and_revokes_pending_reservation() {
        let registry = CodingRunRegistry::default();
        let attempt = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
        let reservation = registry
            .try_reserve_attempt(&attempt)
            .expect("pending reservation");

        assert_eq!(registry.abort_attempt(&attempt).await, 0);
        let (late_tx, _late_rx) = mpsc::channel(1);
        assert!(reservation.activate(late_tx.clone()).is_none());
        assert!(registry.insert(&attempt, late_tx).is_none());
        assert!(registry.try_reserve_attempt(&attempt).is_none());
        assert!(!registry.attempt_is_reserved_or_running(&attempt));
    }

    #[tokio::test]
    async fn abort_completes_when_command_receiver_is_closed() {
        let registry = CodingRunRegistry::default();
        let attempt = CodingAttemptRunKey::new(
            "project_0001",
            "issue_0001",
            "coding_attempt_closed_receiver",
        );
        let (command_tx, command_rx) = mpsc::channel(1);
        registry
            .insert(&attempt, command_tx)
            .expect("closed receiver runner");
        drop(command_rx);

        let sent = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            registry.abort_attempt(&attempt),
        )
        .await
        .expect("abort must not wait forever after command receiver closes");

        assert_eq!(sent, 0);
        assert_eq!(registry.runner_count(&attempt), 0);
    }

    #[tokio::test]
    async fn cancellable_abort_ignores_full_command_channel_and_waits_for_removal() {
        let registry = CodingRunRegistry::default();
        let attempt = CodingAttemptRunKey::new(
            "project_0001",
            "issue_0001",
            "coding_attempt_cancellable_backpressure",
        );
        let (command_tx, _command_rx) = mpsc::channel(1);
        command_tx
            .try_send(CodingRunnerCommand::AbortAttempt)
            .expect("fill command channel");
        let registration = registry
            .insert_cancellable(&attempt, command_tx)
            .expect("cancellable runner");
        let cancellation = registration.cancellation.clone();
        let run_id = registration.run_id;
        let registry_for_runner = registry.clone();
        let attempt_for_runner = attempt.clone();
        let runner = tokio::spawn(async move {
            cancellation.cancelled().await;
            registry_for_runner.remove(&attempt_for_runner, run_id);
        });

        let cancelled = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            registry.abort_attempt(&attempt),
        )
        .await
        .expect("abort must not wait for command channel capacity");

        runner.await.expect("runner cancellation observer");
        assert_eq!(cancelled, 1);
        assert_eq!(registry.runner_count(&attempt), 0);
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
        let second_run_id = registry.insert(&second, second_tx).expect("second runner");

        let registry_for_abort = registry.clone();
        let second_for_abort = second.clone();
        let abort =
            tokio::spawn(async move { registry_for_abort.abort_attempt(&second_for_abort).await });
        assert_eq!(
            second_rx.recv().await.expect("second abort"),
            CodingRunnerCommand::AbortAttempt
        );
        tokio::task::yield_now().await;
        assert!(
            !abort.is_finished(),
            "abort must wait for exact runner removal"
        );
        assert_eq!(registry.runner_count(&first), 1);
        assert_eq!(registry.runner_count(&second), 1);
        assert!(first_rx.try_recv().is_err());
        registry.remove(&second, second_run_id);
        assert_eq!(abort.await.expect("abort task"), 1);
        assert_eq!(registry.runner_count(&second), 0);
    }
}
