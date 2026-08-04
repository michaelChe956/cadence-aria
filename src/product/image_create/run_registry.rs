use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::models::{ImageCreateError, RunKind};

pub struct ActiveRun {
    pub cancel: CancellationToken,
    pub join: Option<JoinHandle<()>>,
    pub kind: RunKind,
}

#[derive(Debug, Clone)]
pub struct Reservation {
    pub id: String,
    pub cancel: CancellationToken,
}

#[derive(Clone, Default)]
pub struct ImageCreateRunRegistry {
    runs: Arc<AsyncMutex<HashMap<String, ActiveRun>>>,
}

impl ImageCreateRunRegistry {
    pub async fn try_reserve(&self, id: &str, kind: RunKind) -> Option<Reservation> {
        let mut runs = self.runs.lock().await;
        if runs.contains_key(id) {
            return None;
        }

        let cancel = CancellationToken::new();
        runs.insert(
            id.to_string(),
            ActiveRun {
                cancel: cancel.clone(),
                join: None,
                kind,
            },
        );
        Some(Reservation {
            id: id.to_string(),
            cancel,
        })
    }

    pub async fn attach_join(
        &self,
        id: &str,
        join: JoinHandle<()>,
    ) -> Result<(), ImageCreateError> {
        let mut runs = self.runs.lock().await;
        if let Some(active) = runs.get_mut(id) {
            active.join = Some(join);
            return Ok(());
        }
        drop(runs);
        let _ = join.await;
        Err(ImageCreateError::SessionGone)
    }

    pub async fn release(&self, id: &str) {
        self.runs.lock().await.remove(id);
    }

    pub async fn take(&self, id: &str) -> Option<ActiveRun> {
        self.runs.lock().await.remove(id)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::oneshot;

    use super::*;

    #[tokio::test]
    async fn reserve_is_exclusive_and_release_reopens_the_slot() {
        let registry = ImageCreateRunRegistry::default();
        assert!(
            registry
                .try_reserve("session", RunKind::Iteration)
                .await
                .is_some()
        );
        assert!(
            registry
                .try_reserve("session", RunKind::Generate)
                .await
                .is_none()
        );

        registry.release("session").await;
        assert!(
            registry
                .try_reserve("session", RunKind::Generate)
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn attached_join_can_be_taken_and_awaited() {
        let registry = ImageCreateRunRegistry::default();
        registry
            .try_reserve("session", RunKind::Generate)
            .await
            .expect("reservation");
        let (done_tx, done_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            let _ = done_tx.send(());
        });
        registry
            .attach_join("session", join)
            .await
            .expect("attach join");

        let active = registry.take("session").await.expect("active run");
        assert_eq!(active.kind, RunKind::Generate);
        active.join.expect("join").await.expect("task completes");
        done_rx.await.expect("task signalled");
    }

    #[tokio::test]
    async fn take_between_reserve_and_attach_is_safe_and_cancels_the_token() {
        let registry = ImageCreateRunRegistry::default();
        let reservation = registry
            .try_reserve("session", RunKind::Iteration)
            .await
            .expect("reservation");
        let active = registry.take("session").await.expect("active run");
        assert!(active.join.is_none());
        active.cancel.cancel();
        reservation.cancel.cancelled().await;

        let orphan = tokio::spawn(async {});
        let error = registry
            .attach_join("session", orphan)
            .await
            .expect_err("taken reservation must reject join");
        assert!(matches!(error, ImageCreateError::SessionGone));
    }

    #[tokio::test]
    async fn spawned_task_observes_reservation_cancellation() {
        let registry = ImageCreateRunRegistry::default();
        let reservation = registry
            .try_reserve("session", RunKind::Generate)
            .await
            .expect("reservation");
        let cancel = reservation.cancel.clone();
        let (cancelled_tx, cancelled_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            cancel.cancelled().await;
            let _ = cancelled_tx.send(());
        });
        registry
            .attach_join("session", join)
            .await
            .expect("attach join");

        let active = registry.take("session").await.expect("active run");
        active.cancel.cancel();
        active.join.expect("join").await.expect("task completes");
        tokio::time::timeout(Duration::from_secs(1), cancelled_rx)
            .await
            .expect("cancellation observed")
            .expect("signal");
    }
}
