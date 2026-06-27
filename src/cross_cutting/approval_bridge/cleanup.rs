use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::streaming_provider::ProviderEvent;

use super::PendingPermissions;

#[cfg(not(test))]
const PERMISSION_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
#[cfg(test)]
const PERMISSION_CLEANUP_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(not(test))]
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(900);
#[cfg(test)]
const PERMISSION_TIMEOUT: Duration = Duration::from_millis(30);

pub(super) async fn cleanup_pending_permissions(
    pending: PendingPermissions,
    event_tx: mpsc::Sender<ProviderEvent>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(PERMISSION_CLEANUP_INTERVAL) => {}
        }
        let now = Instant::now();
        let expired_ids: Vec<String> = {
            let guard = pending.lock().await;
            guard
                .iter()
                .filter(|(_, (_, created_at))| now.duration_since(*created_at) > PERMISSION_TIMEOUT)
                .map(|(id, _)| id.clone())
                .collect()
        };
        let timed_out_ids: Vec<String> = {
            let mut guard = pending.lock().await;
            expired_ids
                .into_iter()
                .filter_map(|id| {
                    guard.remove(&id).map(|(decision_tx, _created_at)| {
                        drop(decision_tx);
                        id
                    })
                })
                .collect()
        };

        for id in timed_out_ids {
            tokio::select! {
                _ = cancel.cancelled() => return,
                result = event_tx.send(ProviderEvent::PermissionTimeout { permission_id: id }) => {
                    let _ = result;
                }
            }
        }
    }
}
