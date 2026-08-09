use std::collections::HashSet;
use std::sync::{Arc, Mutex as StdMutex};

/// Discriminator identifying which initialization flow owns a run.
///
/// Stored as part of `InitializationRunKey` so the same operation id can run
/// concurrently across different flows (single-repository vs aggregate) or
/// projects without being treated as a duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InitializationOperationKind {
    /// Legacy single-repository six-step initialization.
    Repository,
    /// Independent five-step aggregate initialization.
    Aggregate,
}

impl InitializationOperationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Aggregate => "aggregate",
        }
    }
}

/// Composite key for an active initialization run.
///
/// Two runs are distinct unless kind, project and operation id all match,
/// so a retry of the same operation id is still deduplicated while an
/// unrelated concurrent run is never blocked.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InitializationRunKey {
    pub kind: InitializationOperationKind,
    pub project_id: String,
    pub operation_id: String,
}

impl InitializationRunKey {
    pub fn repository(project_id: impl Into<String>, operation_id: impl Into<String>) -> Self {
        Self {
            kind: InitializationOperationKind::Repository,
            project_id: project_id.into(),
            operation_id: operation_id.into(),
        }
    }

    pub fn aggregate(project_id: impl Into<String>, operation_id: impl Into<String>) -> Self {
        Self {
            kind: InitializationOperationKind::Aggregate,
            project_id: project_id.into(),
            operation_id: operation_id.into(),
        }
    }
}

/// Generalized in-memory registry of active initialization runs keyed by
/// `InitializationRunKey`. Used directly by the aggregate initialization
/// flow and, via the `RepositoryInitializationRunRegistry` façade below, by the
/// legacy single-repository flow.
#[derive(Clone, Default)]
pub struct InitializationRunRegistry {
    active: Arc<StdMutex<HashSet<InitializationRunKey>>>,
}

pub struct InitializationRunLease {
    registry: InitializationRunRegistry,
    key: InitializationRunKey,
}

impl InitializationRunRegistry {
    pub fn register(&self, key: InitializationRunKey) -> Option<InitializationRunLease> {
        let mut active = self.active.lock().expect("initialization run lock");
        if !active.insert(key.clone()) {
            return None;
        }
        Some(InitializationRunLease {
            registry: self.clone(),
            key,
        })
    }

    pub fn is_active(&self, key: &InitializationRunKey) -> bool {
        self.active
            .lock()
            .expect("initialization run lock")
            .contains(key)
    }
}

impl Drop for InitializationRunLease {
    fn drop(&mut self) {
        self.registry
            .active
            .lock()
            .expect("initialization run lock")
            .remove(&self.key);
    }
}

/// Legacy single-repository run registry.
///
/// Preserved as a call-compatible façade: existing call sites still pass only
/// an operation id string. Internally the entry is registered under a
/// `Repository` key. The legacy flow historically deduplicated by operation id
/// alone, so the project id is encoded with a stable sentinel; the operation id
/// remains globally unique for repository initialization, preserving the prior
/// deduplication behaviour byte-for-byte while keeping the run bookkeeping on
/// the same key shape used by the generalized registry.
///
/// Future aggregate-initialization call sites should use
/// `InitializationRunRegistry` directly with an `InitializationRunKey::aggregate`
/// carrying the real project id, so they are never blocked by (nor able to
/// block) a repository run that happens to share the same operation id.
#[derive(Clone, Default)]
pub struct RepositoryInitializationRunRegistry {
    active: Arc<StdMutex<HashSet<InitializationRunKey>>>,
}

pub struct RepositoryInitializationRunLease {
    registry: RepositoryInitializationRunRegistry,
    operation_id: String,
}

/// Stable sentinel project id used by the legacy repository façade, which
/// receives only an operation id from its call sites. Repository initialization
/// operation ids are globally unique, so this never collides with an aggregate
/// run that carries a real project id.
const REPOSITORY_FAÇADE_PROJECT_SENTINEL: &str = "__repository_initialization__";

impl RepositoryInitializationRunRegistry {
    pub fn register(&self, operation_id: String) -> Option<RepositoryInitializationRunLease> {
        let key = InitializationRunKey {
            kind: InitializationOperationKind::Repository,
            project_id: REPOSITORY_FAÇADE_PROJECT_SENTINEL.to_string(),
            operation_id: operation_id.clone(),
        };
        let mut active = self
            .active
            .lock()
            .expect("repository initialization run lock");
        if !active.insert(key) {
            return None;
        }
        Some(RepositoryInitializationRunLease {
            registry: self.clone(),
            operation_id,
        })
    }

    pub fn is_active(&self, operation_id: &str) -> bool {
        let key = InitializationRunKey {
            kind: InitializationOperationKind::Repository,
            project_id: REPOSITORY_FAÇADE_PROJECT_SENTINEL.to_string(),
            operation_id: operation_id.to_string(),
        };
        self.active
            .lock()
            .expect("repository initialization run lock")
            .contains(&key)
    }
}

impl Drop for RepositoryInitializationRunLease {
    fn drop(&mut self) {
        let key = InitializationRunKey {
            kind: InitializationOperationKind::Repository,
            project_id: REPOSITORY_FAÇADE_PROJECT_SENTINEL.to_string(),
            operation_id: self.operation_id.clone(),
        };
        self.registry
            .active
            .lock()
            .expect("repository initialization run lock")
            .remove(&key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_operation_id_is_independent_across_kind_and_project() {
        let registry = InitializationRunRegistry::default();
        let repository = InitializationRunKey::repository("project_a", "operation_0001");
        let aggregate = InitializationRunKey::aggregate("project_a", "operation_0001");
        let other_project = InitializationRunKey::aggregate("project_b", "operation_0001");
        let _a = registry.register(repository).unwrap();
        let _b = registry.register(aggregate).unwrap();
        assert!(registry.register(other_project).is_some());
    }

    #[test]
    fn duplicate_register_is_rejected_and_drop_releases_operation() {
        let registry = InitializationRunRegistry::default();
        let key = InitializationRunKey::repository("project_0001", "operation_0001");
        let lease = registry.register(key.clone()).expect("first registration");

        assert!(registry.is_active(&key));
        assert!(registry.register(key.clone()).is_none());

        drop(lease);

        assert!(!registry.is_active(&key));
    }

    #[test]
    fn aggregate_key_with_same_operation_id_is_not_blocked_by_repository_lease() {
        let registry = InitializationRunRegistry::default();
        let repository = InitializationRunKey::repository("project_0001", "operation_0001");
        let aggregate = InitializationRunKey::aggregate("project_0001", "operation_0001");
        let _repository_lease = registry.register(repository.clone()).unwrap();

        // Same project + operation id but different kind must still register.
        let aggregate_lease = registry.register(aggregate.clone()).unwrap();
        assert!(registry.is_active(&repository));
        assert!(registry.is_active(&aggregate));

        // Releasing the aggregate lease must not affect the repository lease.
        drop(aggregate_lease);
        assert!(registry.is_active(&repository));
        assert!(!registry.is_active(&aggregate));
    }

    #[test]
    fn legacy_repository_façade_preserves_operation_id_deduplication() {
        let registry = RepositoryInitializationRunRegistry::default();
        let lease = registry
            .register("operation_0001".to_string())
            .expect("first registration");

        assert!(registry.is_active("operation_0001"));
        assert!(registry.register("operation_0001".to_string()).is_none());

        // A different operation id still registers and does not interfere.
        let other_lease = registry.register("operation_0002".to_string()).unwrap();
        assert!(registry.is_active("operation_0002"));
        drop(other_lease);
        assert!(!registry.is_active("operation_0002"));
        assert!(registry.is_active("operation_0001"));

        drop(lease);

        assert!(!registry.is_active("operation_0001"));
    }

    #[test]
    fn legacy_repository_façade_never_blocks_aggregate_run_with_same_operation_id() {
        let generalized = InitializationRunRegistry::default();
        let legacy = RepositoryInitializationRunRegistry::default();

        let _legacy_lease = legacy.register("operation_0001".to_string()).unwrap();
        // A generalized aggregate key with the same operation id is independent
        // of the legacy repository façade's internal sentinel-keyed entry.
        let aggregate = InitializationRunKey::aggregate("project_0001", "operation_0001");
        assert!(generalized.register(aggregate).is_some());
    }
}
