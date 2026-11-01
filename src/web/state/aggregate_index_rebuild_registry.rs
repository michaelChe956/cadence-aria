use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// In-process per-project guard for synchronous aggregate-index rebuilds.
///
/// The durable aggregate-index lock still protects the filesystem across
/// processes. This registry provides the HTTP contract within one AppState:
/// concurrent requests fail immediately instead of waiting on that lock.
#[derive(Clone, Default)]
pub struct AggregateIndexRebuildRegistry {
    active: Arc<Mutex<HashSet<String>>>,
}

pub struct AggregateIndexRebuildLease {
    registry: AggregateIndexRebuildRegistry,
    project_id: String,
}

impl AggregateIndexRebuildRegistry {
    pub fn try_register(&self, project_id: &str) -> Option<AggregateIndexRebuildLease> {
        let mut active = self
            .active
            .lock()
            .expect("aggregate index rebuild registry lock");
        if !active.insert(project_id.to_string()) {
            return None;
        }
        Some(AggregateIndexRebuildLease {
            registry: self.clone(),
            project_id: project_id.to_string(),
        })
    }

    #[cfg(test)]
    fn is_active(&self, project_id: &str) -> bool {
        self.active
            .lock()
            .expect("aggregate index rebuild registry lock")
            .contains(project_id)
    }
}

impl Drop for AggregateIndexRebuildLease {
    fn drop(&mut self) {
        self.registry
            .active
            .lock()
            .expect("aggregate index rebuild registry lock")
            .remove(&self.project_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_register_rejects_same_project_and_drop_releases_lease() {
        let registry = AggregateIndexRebuildRegistry::default();
        let lease = registry.try_register("project_0001").expect("first lease");
        assert!(registry.is_active("project_0001"));
        assert!(registry.try_register("project_0001").is_none());
        assert!(registry.try_register("project_0002").is_some());
        drop(lease);
        assert!(!registry.is_active("project_0001"));
        assert!(registry.try_register("project_0001").is_some());
    }
}
