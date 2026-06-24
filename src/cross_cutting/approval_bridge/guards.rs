use std::sync::Arc;

use super::{PendingChoices, PendingPermissions};

pub(super) struct PendingPermissionGuard {
    id: Option<String>,
    pending: PendingPermissions,
}

impl PendingPermissionGuard {
    pub(super) fn new(id: String, pending: PendingPermissions) -> Self {
        Self {
            id: Some(id),
            pending,
        }
    }

    pub(super) async fn remove_now(&mut self) {
        let Some(id) = self.id.as_ref().cloned() else {
            return;
        };
        self.pending.lock().await.remove(&id);
        self.id = None;
    }
}

impl Drop for PendingPermissionGuard {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let pending = Arc::clone(&self.pending);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                pending.lock().await.remove(&id);
            });
        }
    }
}

pub(super) struct PendingChoiceGuard {
    id: Option<String>,
    pending: PendingChoices,
}

impl PendingChoiceGuard {
    pub(super) fn new(id: String, pending: PendingChoices) -> Self {
        Self {
            id: Some(id),
            pending,
        }
    }

    pub(super) async fn remove_now(&mut self) {
        let Some(id) = self.id.as_ref().cloned() else {
            return;
        };
        self.pending.lock().await.remove(&id);
        self.id = None;
    }
}

impl Drop for PendingChoiceGuard {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let pending = Arc::clone(&self.pending);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                pending.lock().await.remove(&id);
            });
        }
    }
}
