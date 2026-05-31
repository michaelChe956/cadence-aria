use cadence_aria::cross_cutting::integration_queue::{
    IntegrationQueue, candidate_commit_is_not_integrated,
};
use cadence_aria::cross_cutting::provider_run::recover_provider_run_records;
use cadence_aria::cross_cutting::worktree::{WorktreeLeaseManager, WorktreeLeaseStatus};
use cadence_aria::daemon::recovery::{
    RecoverableGate, RecoverableGateStatus, RecoverableRuntimeEvent, recover_open_gates,
    replay_events_after,
};
use cadence_aria::protocol::contracts::{
    ApprovalPolicy, ProviderRunRecord, ProviderRunStatus, ProviderType, RuntimeRole, SandboxMode,
    TimeoutStatus,
};

#[test]
fn recovery_restores_open_gates_worktree_leases_provider_runs_and_replay_cursor() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut manager =
        WorktreeLeaseManager::new("session_001", "task_001", workspace.path(), "main");
    let lease = manager
        .acquire(
            "worktask_001",
            "aria/worktask_001",
            vec!["src/feature/".to_string()],
        )
        .expect("lease");
    let serialized = serde_json::to_string(manager.leases()).expect("leases json");
    let recovered_leases = serde_json::from_str(&serialized).expect("recover leases");
    let recovered = WorktreeLeaseManager::recover(
        "session_001",
        "task_001",
        workspace.path(),
        "main",
        recovered_leases,
    );
    let recovered_lease = recovered.lease(&lease.lease_id).expect("recovered lease");
    assert_eq!(recovered_lease.status, WorktreeLeaseStatus::Acquired);
    assert_eq!(recovered_lease.allowed_write_scope, vec!["src/feature/"]);
    assert_eq!(recovered_lease.branch_name, "aria/worktask_001");
    assert_eq!(recovered_lease.worktask_id, "worktask_001");

    let gates = recover_open_gates(&[
        RecoverableGate {
            gate_id: "gate_followup".to_string(),
            status: RecoverableGateStatus::Open,
        },
        RecoverableGate {
            gate_id: "gate_resolved".to_string(),
            status: RecoverableGateStatus::Resolved,
        },
    ]);
    assert_eq!(gates, vec!["gate_followup".to_string()]);

    let recovered_runs = recover_provider_run_records(vec![
        provider_record("run_pending", ProviderRunStatus::Pending),
        provider_record("run_running", ProviderRunStatus::Running),
        provider_record("run_completed", ProviderRunStatus::Completed),
    ]);
    assert_eq!(
        recovered_runs[0].status,
        ProviderRunStatus::RecoveredPending
    );
    assert_eq!(
        recovered_runs[1].status,
        ProviderRunStatus::RecoveredPending
    );
    assert_eq!(recovered_runs[2].status, ProviderRunStatus::Completed);

    let replay = replay_events_after(
        &[
            RecoverableRuntimeEvent {
                event_id: 10,
                event_type: "provider.started".to_string(),
            },
            RecoverableRuntimeEvent {
                event_id: 11,
                event_type: "worktree.lease_acquired".to_string(),
            },
            RecoverableRuntimeEvent {
                event_id: 12,
                event_type: "provider.completed".to_string(),
            },
        ],
        Some(11),
    );
    assert_eq!(
        replay
            .iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>(),
        vec![12]
    );

    let mut queue = IntegrationQueue::default();
    let queued = queue.enqueue("worktask_001", "candidate_sha_001");
    assert!(candidate_commit_is_not_integrated(&queued));
}

fn provider_record(provider_run_id: &str, status: ProviderRunStatus) -> ProviderRunRecord {
    ProviderRunRecord {
        provider_run_id: provider_run_id.to_string(),
        node_id: "N16".to_string(),
        provider_type: ProviderType::Codex,
        runtime_role: RuntimeRole::Executor,
        adapter_role: RuntimeRole::Executor.adapter_role(),
        provider_capability_ref: "capability".to_string(),
        adapter_compatibility_ref: "compat".to_string(),
        context_package_ref: "ctx".to_string(),
        adapter_input_ref: "input".to_string(),
        adapter_output_ref: "output".to_string(),
        raw_artifact_refs: vec![],
        exit_code: None,
        error_code: None,
        error_details: None,
        stdout_ref: None,
        stderr_ref: None,
        structured_output_ref: None,
        files_modified: vec![],
        status,
        started_at: "2026-04-27T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        timeout_status: TimeoutStatus::NotTimedOut,
        retry_count: 0,
        approval_policy: ApprovalPolicy::OnRequest,
        sandbox_mode: SandboxMode::WorkspaceWrite,
        constraint_check_ref: None,
        traceability_binding_refs: vec![],
    }
}
