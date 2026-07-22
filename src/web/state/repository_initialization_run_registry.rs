use std::collections::HashSet;
use std::sync::{Arc, Mutex as StdMutex};

#[derive(Clone, Default)]
pub struct RepositoryInitializationRunRegistry {
    active: Arc<StdMutex<HashSet<String>>>,
}

pub struct RepositoryInitializationRunLease {
    registry: RepositoryInitializationRunRegistry,
    operation_id: String,
}

impl RepositoryInitializationRunRegistry {
    pub fn register(&self, operation_id: String) -> Option<RepositoryInitializationRunLease> {
        let mut active = self
            .active
            .lock()
            .expect("repository initialization run lock");
        if !active.insert(operation_id.clone()) {
            return None;
        }
        Some(RepositoryInitializationRunLease {
            registry: self.clone(),
            operation_id,
        })
    }

    pub fn is_active(&self, operation_id: &str) -> bool {
        self.active
            .lock()
            .expect("repository initialization run lock")
            .contains(operation_id)
    }
}

impl Drop for RepositoryInitializationRunLease {
    fn drop(&mut self) {
        self.registry
            .active
            .lock()
            .expect("repository initialization run lock")
            .remove(&self.operation_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_register_is_rejected_and_drop_releases_operation() {
        let registry = RepositoryInitializationRunRegistry::default();
        let lease = registry
            .register("operation_0001".to_string())
            .expect("first registration");

        assert!(registry.is_active("operation_0001"));
        assert!(registry.register("operation_0001".to_string()).is_none());

        drop(lease);

        assert!(!registry.is_active("operation_0001"));
    }
}
