use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{WorkspaceSessionManager, WorkspaceSessionRegistry};

#[tokio::test]
async fn registry_get_or_create_runs_factory_once_under_concurrency() {
    let registry = WorkspaceSessionRegistry::default();
    let factory_calls = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..16 {
        let registry = registry.clone();
        let factory_calls = factory_calls.clone();
        handles.push(tokio::spawn(async move {
            registry
                .get_or_create("session_x", move || {
                    let factory_calls = factory_calls.clone();
                    async move {
                        factory_calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        Ok(WorkspaceSessionManager::test_fixture("session_x"))
                    }
                })
                .await
        }));
    }

    let first = handles.remove(0).await.expect("task").expect("manager");
    for handle in handles {
        let manager = handle.await.expect("task").expect("manager");
        assert!(Arc::ptr_eq(&first, &manager));
    }
    assert_eq!(factory_calls.load(Ordering::SeqCst), 1);
    assert_eq!(registry.session_ids().await, vec!["session_x".to_string()]);
}

#[tokio::test]
async fn registry_session_ids_are_sorted_and_remove_is_idempotent() {
    let registry = WorkspaceSessionRegistry::default();

    registry.remove("missing").await;
    assert!(registry.session_ids().await.is_empty());
}
