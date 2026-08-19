use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use tokio_util::sync::CancellationToken;

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
/// Two runs are distinct unless kind, project, logical codebase and operation
/// id all match, so a retry of the same operation id within the same logical
/// codebase is still deduplicated while an unrelated concurrent run (including
/// the same project + operation id in a different logical codebase) is never
/// blocked.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InitializationRunKey {
    pub kind: InitializationOperationKind,
    pub project_id: String,
    pub lc_id: String,
    pub operation_id: String,
}

impl InitializationRunKey {
    pub fn repository(project_id: impl Into<String>, operation_id: impl Into<String>) -> Self {
        Self {
            kind: InitializationOperationKind::Repository,
            project_id: project_id.into(),
            lc_id: REPOSITORY_FAÇADE_LC_SENTINEL.to_string(),
            operation_id: operation_id.into(),
        }
    }

    pub fn aggregate(
        project_id: impl Into<String>,
        lc_id: impl Into<String>,
        operation_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: InitializationOperationKind::Aggregate,
            project_id: project_id.into(),
            lc_id: lc_id.into(),
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
    active: Arc<StdMutex<HashMap<InitializationRunKey, CancellationToken>>>,
    run_ids: Arc<StdMutex<HashMap<InitializationRunKey, u64>>>,
    next_run_id: Arc<AtomicU64>,
}

pub struct InitializationRunLease {
    registry: InitializationRunRegistry,
    key: InitializationRunKey,
    cancellation: CancellationToken,
    run_id: u64,
}

impl InitializationRunLease {
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

impl InitializationRunRegistry {
    pub fn register(&self, key: InitializationRunKey) -> Option<InitializationRunLease> {
        let cancellation = CancellationToken::new();
        let run_id = self.next_run_id.fetch_add(1, Ordering::Relaxed);
        let mut active = self.active.lock().expect("initialization run lock");
        if let Entry::Vacant(entry) = active.entry(key.clone()) {
            entry.insert(cancellation.clone());
        } else {
            return None;
        }
        self.run_ids
            .lock()
            .expect("initialization run lock")
            .insert(key.clone(), run_id);
        Some(InitializationRunLease {
            registry: self.clone(),
            key,
            cancellation,
            run_id,
        })
    }

    pub fn is_active(&self, key: &InitializationRunKey) -> bool {
        self.active
            .lock()
            .expect("initialization run lock")
            .contains_key(key)
    }

    pub fn cancel(&self, key: &InitializationRunKey) -> bool {
        let cancellation = self
            .active
            .lock()
            .expect("initialization run lock")
            .get(key)
            .cloned();
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
            true
        } else {
            false
        }
    }
}

impl Drop for InitializationRunLease {
    fn drop(&mut self) {
        let mut active = self
            .registry
            .active
            .lock()
            .expect("initialization run lock");
        let mut run_ids = self
            .registry
            .run_ids
            .lock()
            .expect("initialization run lock");
        if run_ids.get(&self.key) == Some(&self.run_id) {
            active.remove(&self.key);
            run_ids.remove(&self.key);
        }
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
/// carrying the real project id and logical codebase id, so they are never
/// blocked by (nor able to block) a repository run that happens to share the
/// same operation id.
#[derive(Clone, Default)]
pub struct RepositoryInitializationRunRegistry {
    active: InitializationRunRegistry,
}

pub struct RepositoryInitializationRunLease {
    _lease: InitializationRunLease,
}

/// Stable sentinel project id used by the legacy repository façade, which
/// receives only an operation id from its call sites. Repository initialization
/// operation ids are globally unique, so this never collides with an aggregate
/// run that carries a real project id.
const REPOSITORY_FAÇADE_PROJECT_SENTINEL: &str = "__repository_initialization__";

/// Stable sentinel logical codebase id used by the legacy repository façade and
/// the `repository` key constructor. The repository flow has no logical
/// codebase, so its in-memory keys carry this sentinel; aggregate keys always
/// carry a real lc_id, so the two never collide on the lc_id axis.
const REPOSITORY_FAÇADE_LC_SENTINEL: &str = "__repository_initialization_lc__";

impl RepositoryInitializationRunRegistry {
    pub fn register(&self, operation_id: String) -> Option<RepositoryInitializationRunLease> {
        let key = InitializationRunKey {
            kind: InitializationOperationKind::Repository,
            project_id: REPOSITORY_FAÇADE_PROJECT_SENTINEL.to_string(),
            lc_id: REPOSITORY_FAÇADE_LC_SENTINEL.to_string(),
            operation_id,
        };
        Some(RepositoryInitializationRunLease {
            _lease: self.active.register(key)?,
        })
    }

    pub fn is_active(&self, operation_id: &str) -> bool {
        let key = InitializationRunKey {
            kind: InitializationOperationKind::Repository,
            project_id: REPOSITORY_FAÇADE_PROJECT_SENTINEL.to_string(),
            lc_id: REPOSITORY_FAÇADE_LC_SENTINEL.to_string(),
            operation_id: operation_id.to_string(),
        };
        self.active.is_active(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_lease_exposes_token_cancel_and_drop_removes_registry_entry() {
        let registry = InitializationRunRegistry::default();
        let key = InitializationRunKey::aggregate("project_0001", "lc_0001", "operation_0001");
        let lease = registry.register(key.clone()).unwrap();
        assert!(!lease.cancellation_token().is_cancelled());
        assert!(registry.cancel(&key));
        assert!(lease.cancellation_token().is_cancelled());
        drop(lease);
        assert!(!registry.is_active(&key));
        assert!(!registry.cancel(&key));
    }

    #[test]
    fn stale_lease_drop_does_not_remove_replaced_run_entry() {
        let registry = InitializationRunRegistry::default();
        let key = InitializationRunKey::aggregate("project_0001", "lc_0001", "operation_0001");
        let lease = registry.register(key.clone()).unwrap();

        registry
            .active
            .lock()
            .expect("initialization run lock")
            .insert(key.clone(), CancellationToken::new());
        registry
            .run_ids
            .lock()
            .expect("initialization run lock")
            .insert(key.clone(), u64::MAX);

        drop(lease);

        assert!(registry.is_active(&key));
    }

    #[test]
    fn panicking_worker_drop_releases_registry_entry() {
        let registry = InitializationRunRegistry::default();
        let key = InitializationRunKey::aggregate("project_0001", "lc_0001", "operation_0001");
        let worker_registry = registry.clone();
        let worker_key = key.clone();

        let result = std::panic::catch_unwind(move || {
            let _lease = worker_registry.register(worker_key).unwrap();
            panic!("worker failed");
        });

        assert!(result.is_err());
        assert!(!registry.is_active(&key));
    }

    #[test]
    fn same_operation_id_is_independent_across_kind_and_project() {
        let registry = InitializationRunRegistry::default();
        let repository = InitializationRunKey::repository("project_a", "operation_0001");
        let aggregate = InitializationRunKey::aggregate("project_a", "lc_0001", "operation_0001");
        let other_project =
            InitializationRunKey::aggregate("project_b", "lc_0002", "operation_0001");
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
        let aggregate =
            InitializationRunKey::aggregate("project_0001", "lc_0001", "operation_0001");
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
        let aggregate =
            InitializationRunKey::aggregate("project_0001", "lc_0001", "operation_0001");
        assert!(generalized.register(aggregate).is_some());
    }

    #[test]
    fn aggregate_same_project_same_operation_id_across_lcs_must_not_crosstalk() {
        let registry = InitializationRunRegistry::default();
        let lc_a = InitializationRunKey::aggregate("project_0001", "lc_0001", "operation_0001");
        let lc_b = InitializationRunKey::aggregate("project_0001", "lc_0002", "operation_0001");

        // 并发场景：同 project + 同 operation id 的两个 LC 各自注册，互不干扰。
        let lease_a = registry.register(lc_a.clone()).unwrap();
        let lease_b = registry.register(lc_b.clone()).unwrap();
        assert!(registry.is_active(&lc_a));
        assert!(registry.is_active(&lc_b));

        // cancel 只命中对应 LC 的 token。
        assert!(registry.cancel(&lc_a));
        assert!(lease_a.cancellation_token().is_cancelled());
        assert!(!lease_b.cancellation_token().is_cancelled());

        // 顺序场景：LC A 释放后不影响 LC B 的租约，同 key 可在 LC A 重新注册。
        drop(lease_a);
        assert!(!registry.is_active(&lc_a));
        assert!(registry.is_active(&lc_b));
        assert!(registry.register(lc_a.clone()).is_some());
        drop(lease_b);
        assert!(!registry.is_active(&lc_b));
    }
}
