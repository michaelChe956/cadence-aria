use crate::product::lifecycle_store::workspace::PolicyRoutePersist;

fn recovery_store() -> (tempfile::TempDir, LifecycleStore, Arc<CheckpointStore>) {
    let tmp = tempfile::tempdir().expect("temporary product root");
    let paths = crate::product::app_paths::ProductAppPaths::new(tmp.path().join(".aria"));
    let checkpoints = Arc::new(CheckpointStore::new(
        paths.issue_lifecycle_root("project_0001", "issue_0001"),
    ));
    (tmp, LifecycleStore::new(paths), checkpoints)
}

fn recovery_session(
    store: &LifecycleStore,
    entity_id: &str,
    policy: crate::product::work_item_plan_policy::RunPolicy,
) -> WorkspaceSessionRecord {
    store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: entity_id.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: Some(
                crate::product::lifecycle_store::WorkItemPlanSessionOptions {
                    flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                    run_policy: policy,
                    rollout_snapshot: true,
                },
            ),
        })
        .expect("create recovery session")
}

fn recovery_gate(resumable: bool) -> crate::product::work_item_plan_policy::HumanGateSnapshot {
    let class = crate::product::work_item_plan_policy::FindingClass::HumanRequired;
    crate::product::work_item_plan_policy::HumanGateSnapshot {
        findings: vec![crate::product::work_item_plan_policy::ClassifiedFinding {
            class,
            fingerprint: crate::product::work_item_plan_policy::FindingFingerprint::for_finding(
                None,
                class,
                "需要人工决定范围",
                Some("scope"),
            ),
            category: Some(crate::product::work_item_plan_policy::ReviewFindingCategory::ScopeConflict),
            severity: "blocking".to_string(),
            message: "需要人工决定范围".to_string(),
            evidence: Some("冲突证据".to_string()),
            required_action: Some("确认范围".to_string()),
            contract_field: Some("scope".to_string()),
        }],
        repeated_fingerprints: Vec::new(),
        attempts_used: 2,
        manual_repairs_remaining: 1,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable,
    }
}

fn recovery_engine(
    checkpoints: Arc<CheckpointStore>,
    store: LifecycleStore,
    record: WorkspaceSessionRecord,
) -> WorkspaceEngine {
    let (events, _events_rx) = mpsc::channel(8);
    WorkspaceEngine::new_persistent(checkpoints, store, events, WorkspaceSession::from_record(record))
}

fn stopped_takeover_parent(
    store: &LifecycleStore,
    entity_id: &str,
    resumable: bool,
) -> WorkspaceSessionRecord {
    let parent = recovery_session(
        store,
        entity_id,
        crate::product::work_item_plan_policy::RunPolicy::AutoIfValid,
    );
    store
        .compare_and_save_policy_route(
            &parent,
            PolicyRoutePersist {
                status: WorkspaceSessionStatus::StoppedNeedsHuman,
                run_history: parent.run_history.clone(),
                scope: Some(crate::product::work_item_plan_policy::ReviewInvocationScope::initial(
                    format!("outline:{entity_id}"),
                )),
                gate: Some(recovery_gate(resumable)),
                diagnostics: Vec::new(),
                repair_reservation: None,
                provider_start_ledger: Vec::new(),
            },
        )
        .expect("persist stopped takeover parent")
}

#[test]
fn policy_cas_persists_repair_reservation_and_provider_start_ledger_atomically() {
    let (_tmp, store, _checkpoints) = recovery_store();
    let expected = recovery_session(
        &store,
        "work_item_plan_cas",
        crate::product::work_item_plan_policy::RunPolicy::AutoIfValid,
    );
    let reservation = crate::product::work_item_plan_policy::RepairReservation {
        token: "repair-token".to_string(),
        owner_session_id: expected.id.clone(),
        owner_run_id: "run-1".to_string(),
        provider_start_idempotency_key: "provider-start-1".to_string(),
        state: crate::product::work_item_plan_policy::RepairReservationState::Reserved,
        commit_id: None,
    };
    let ledger = vec![crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
        provider_start_idempotency_key: "provider-start-1".to_string(),
        started: true,
    }];

    let saved = store
        .compare_and_save_policy_route(
            &expected,
            PolicyRoutePersist {
                status: WorkspaceSessionStatus::Running,
                run_history: expected.run_history.clone(),
                scope: expected.review_invocation_scope.clone(),
                gate: expected.human_gate_snapshot.clone(),
                diagnostics: expected.policy_diagnostics.clone(),
                repair_reservation: Some(reservation.clone()),
                provider_start_ledger: ledger.clone(),
            },
        )
        .expect("CAS save");
    assert_eq!(saved.repair_reservation, Some(reservation));
    assert_eq!(saved.provider_start_ledger, ledger);
    assert_eq!(
        store.get_workspace_session(&expected.id).unwrap().provider_start_ledger,
        saved.provider_start_ledger
    );
}

#[test]
fn policy_cas_conflict_rejects_stale_record_and_routing_reloads_then_reevaluates() {
    let (_tmp, store, checkpoints) = recovery_store();
    let stale = recovery_session(
        &store,
        "work_item_plan_cas_conflict",
        crate::product::work_item_plan_policy::RunPolicy::AutoIfValid,
    );
    let concurrent = store
        .update_workspace_session_status(&stale.id, WorkspaceSessionStatus::Running)
        .expect("concurrent write makes expected record stale");

    let error = store
        .compare_and_save_policy_route(
            &stale,
            PolicyRoutePersist {
                status: WorkspaceSessionStatus::StoppedNeedsHuman,
                run_history: stale.run_history.clone(),
                scope: stale.review_invocation_scope.clone(),
                gate: Some(recovery_gate(true)),
                diagnostics: Vec::new(),
                repair_reservation: None,
                provider_start_ledger: Vec::new(),
            },
        )
        .expect_err("stale expected record must fail the CAS");
    assert!(matches!(
        error,
        ProductStoreError::Conflict {
            kind: "workspace_session",
            ref id,
        } if id == &stale.id
    ));

    let mut engine = recovery_engine(checkpoints, store.clone(), concurrent);
    engine.policy_route_before_persist = Some(Box::new(|store, session_id| {
        store
            .update_workspace_session_status(session_id, WorkspaceSessionStatus::WaitingForHuman)
            .expect("second concurrent write must be durable");
    }));
    let action = engine
        .work_item_policy_action(
            "outline_review",
            &review_verdict(ReviewVerdictType::NeedsHuman),
        )
        .expect("caller must reload and re-evaluate after the conflict");

    assert!(matches!(action, RoutingAction::StopNeedsHuman { .. }));
    assert_eq!(engine.session.run_history.initial_review_count, 1);
    let persisted = store
        .get_workspace_session(&engine.session.session_id)
        .expect("retried route must be persisted");
    assert_eq!(persisted.status, WorkspaceSessionStatus::StoppedNeedsHuman);
    assert_eq!(persisted.run_history.initial_review_count, 1);
    assert_eq!(
        persisted.run_history.review_cycles["review:outline_review"].initial_count,
        1,
        "re-evaluation must not merge the stale history delta twice"
    );
}

#[test]
fn awaiting_human_reconnect_restores_durable_gate_without_provider_restart_or_event_mutation() {
    let (_tmp, store, checkpoints) = recovery_store();
    let parent = recovery_session(
        &store,
        "work_item_plan_awaiting",
        crate::product::work_item_plan_policy::RunPolicy::Interactive,
    );
    let gate = recovery_gate(false);
    let persisted = store
        .compare_and_save_policy_route(
            &parent,
            PolicyRoutePersist {
                status: WorkspaceSessionStatus::WaitingForHuman,
                run_history: parent.run_history.clone(),
                scope: Some(crate::product::work_item_plan_policy::ReviewInvocationScope::initial(
                    "outline:awaiting".to_string(),
                )),
                gate: Some(gate.clone()),
                diagnostics: Vec::new(),
                repair_reservation: None,
                provider_start_ledger: Vec::new(),
            },
        )
        .expect("persist awaiting JSON");
    let events_before = store
        .load_timeline_nodes_for_issue_session(
            &persisted.project_id,
            &persisted.issue_id,
            &persisted.id,
        )
        .expect("events before reconnect");

    let engine = recovery_engine(checkpoints, store.clone(), persisted.clone());
    match engine.build_session_state() {
        WsOutMessage::SessionState {
            session_status,
            human_gate_snapshot,
            provider_start_ledger,
            ..
        } => {
            assert_eq!(session_status, WorkspaceSessionStatus::WaitingForHuman);
            assert_eq!(human_gate_snapshot, Some(gate));
            assert!(provider_start_ledger.is_empty());
        }
        _ => panic!("expected SessionState"),
    }
    assert_eq!(
        store
            .load_timeline_nodes_for_issue_session(
                &persisted.project_id,
                &persisted.issue_id,
                &persisted.id,
            )
            .expect("events after reconnect"),
        events_before
    );
}

#[test]
fn stopped_needs_human_takeover_creates_interactive_child_without_mutating_parent() {
    let (_tmp, store, checkpoints) = recovery_store();
    let parent = recovery_session(
        &store,
        "work_item_plan_takeover",
        crate::product::work_item_plan_policy::RunPolicy::AutoIfValid,
    );
    let mut history = parent.run_history.clone();
    history.repairs_used = 1;
    let persisted = store
        .compare_and_save_policy_route(
            &parent,
            PolicyRoutePersist {
                status: WorkspaceSessionStatus::StoppedNeedsHuman,
                run_history: history,
                scope: Some(crate::product::work_item_plan_policy::ReviewInvocationScope::initial(
                    "outline:takeover".to_string(),
                )),
                gate: Some(recovery_gate(true)),
                diagnostics: Vec::new(),
                repair_reservation: None,
                provider_start_ledger: Vec::new(),
            },
        )
        .expect("persist stopped parent");
    let parent_json_before = serde_json::to_value(&persisted).expect("parent JSON");
    let parent_events_before = store
        .load_timeline_nodes_for_issue_session(
            &persisted.project_id,
            &persisted.issue_id,
            &persisted.id,
        )
        .expect("parent events");

    let engine = recovery_engine(checkpoints, store.clone(), persisted.clone());
    let child = engine
        .takeover_stopped_needs_human()
        .expect("takeover stopped run");
    assert_eq!(
        child.run_policy,
        crate::product::work_item_plan_policy::RunPolicy::Interactive
    );
    assert_eq!(child.status, WorkspaceSessionStatus::Open);
    assert!(child.human_gate_snapshot.is_none());
    assert!(child.provider_start_ledger.is_empty());
    assert_eq!(
        serde_json::to_value(store.get_workspace_session(&persisted.id).unwrap()).unwrap(),
        parent_json_before
    );
    assert_eq!(
        store
            .load_timeline_nodes_for_issue_session(
                &persisted.project_id,
                &persisted.issue_id,
                &persisted.id,
            )
            .expect("parent events after takeover"),
        parent_events_before
    );
    let event = store
        .get_human_gate_takeover_event(&persisted.id)
        .expect("load associated event")
        .expect("takeover event");
    assert_eq!(event.parent_session_id, persisted.id);
    assert_eq!(event.child_session_id, child.id);

    let repeated = store
        .takeover_stopped_needs_human(&persisted.id)
        .expect("repeated takeover must return the event child");
    assert_eq!(repeated.id, child.id);
    assert!(repeated.provider_start_ledger.is_empty());
    assert_eq!(
        store
            .list_workspace_sessions(&persisted.project_id, &persisted.issue_id)
            .expect("list sessions after repeated takeover")
            .len(),
        2,
        "the durable takeover event must prevent duplicate child creation"
    );
}

#[test]
fn stopped_needs_human_takeover_recovers_precreated_child_without_event() {
    let (_tmp, store, _checkpoints) = recovery_store();
    let parent = stopped_takeover_parent(&store, "work_item_plan_takeover_crash", true);
    let child_id = format!("workspace_session_takeover_{}", parent.id);
    let precreated_child = store
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: parent.project_id.clone(),
                issue_id: parent.issue_id.clone(),
                entity_id: parent.entity_id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: parent.author_provider.clone(),
                reviewer_provider: parent.reviewer_provider.clone(),
                review_rounds: parent.review_rounds,
                superpowers_enabled: parent.superpowers_enabled,
                openspec_enabled: parent.openspec_enabled,
                work_item_plan_options: Some(
                    crate::product::lifecycle_store::WorkItemPlanSessionOptions {
                        flow_kind: parent.flow_kind,
                        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
                        rollout_snapshot: false,
                    },
                ),
            },
            child_id,
        )
        .expect("simulate crash after child creation and before event write");
    assert!(store
        .get_human_gate_takeover_event(&parent.id)
        .expect("read absent event")
        .is_none());

    let retried = store
        .takeover_stopped_needs_human(&parent.id)
        .expect("identity-idempotent child must allow event recovery");
    assert_eq!(retried.id, precreated_child.id);
    assert!(retried.provider_start_ledger.is_empty());
    let event = store
        .get_human_gate_takeover_event(&parent.id)
        .expect("read recovered event")
        .expect("retry must persist the event");
    assert_eq!(event.child_session_id, precreated_child.id);
    assert_eq!(
        store
            .list_workspace_sessions(&parent.project_id, &parent.issue_id)
            .expect("list sessions after crash-window recovery")
            .len(),
        2,
        "the retry must not create a second child"
    );
}

#[test]
fn stopped_needs_human_takeover_rejects_non_stopped_or_non_resumable_parents() {
    let (_tmp, store, _checkpoints) = recovery_store();
    let non_stopped = recovery_session(
        &store,
        "work_item_plan_takeover_not_stopped",
        crate::product::work_item_plan_policy::RunPolicy::AutoIfValid,
    );
    let non_resumable = stopped_takeover_parent(
        &store,
        "work_item_plan_takeover_not_resumable",
        false,
    );

    for parent in [&non_stopped, &non_resumable] {
        let error = store
            .takeover_stopped_needs_human(&parent.id)
            .expect_err("invalid parent must not produce a takeover child");
        assert!(matches!(
            error,
            ProductStoreError::InvalidRecord {
                kind: "human_gate_takeover",
                ..
            }
        ));
        assert!(store
            .get_human_gate_takeover_event(&parent.id)
            .expect("invalid parent must not create an event")
            .is_none());
    }
    assert_eq!(
        store
            .list_workspace_sessions(&non_stopped.project_id, &non_stopped.issue_id)
            .expect("list sessions after rejected takeovers")
            .len(),
        2,
        "rejected takeovers must not create children"
    );
}

#[test]
fn completed_and_failed_replay_do_not_recreate_human_gate_or_start_provider() {
    let (_tmp, store, checkpoints) = recovery_store();
    for (suffix, status) in [
        ("completed", WorkspaceSessionStatus::Confirmed),
        ("failed", WorkspaceSessionStatus::Failed),
    ] {
        let parent = recovery_session(
            &store,
            &format!("work_item_plan_{suffix}"),
            crate::product::work_item_plan_policy::RunPolicy::AutoIfValid,
        );
        let persisted = store
            .compare_and_save_policy_route(
                &parent,
                PolicyRoutePersist {
                    status: status.clone(),
                    run_history: parent.run_history.clone(),
                    scope: None,
                    gate: None,
                    diagnostics: Vec::new(),
                    repair_reservation: None,
                    provider_start_ledger: Vec::new(),
                },
            )
            .expect("persist terminal JSON");
        let events_before = store
            .load_timeline_nodes_for_issue_session(
                &persisted.project_id,
                &persisted.issue_id,
                &persisted.id,
            )
            .expect("terminal events");

        let engine = recovery_engine(checkpoints.clone(), store.clone(), persisted.clone());
        match engine.build_session_state() {
            WsOutMessage::SessionState {
                session_status,
                human_gate_snapshot,
                provider_start_ledger,
                ..
            } => {
                assert_eq!(session_status, status, "{suffix}");
                assert!(human_gate_snapshot.is_none(), "{suffix}");
                assert!(provider_start_ledger.is_empty(), "{suffix}");
            }
            _ => panic!("expected SessionState"),
        }
        assert_eq!(
            store
                .load_timeline_nodes_for_issue_session(
                    &persisted.project_id,
                    &persisted.issue_id,
                    &persisted.id,
                )
                .expect("terminal events after replay"),
            events_before,
            "{suffix}"
        );
    }
}

#[test]
fn provider_start_ledger_claim_is_idempotent_across_generate_and_repair_recovery() {
    let (_tmp, store, _checkpoints) = recovery_store();
    for operation in ["generate", "repair"] {
        let session = recovery_session(
            &store,
            &format!("work_item_plan_{operation}"),
            crate::product::work_item_plan_policy::RunPolicy::AutoIfValid,
        );
        let reservation = crate::product::work_item_plan_policy::RepairReservation {
            token: format!("{operation}-reservation"),
            owner_session_id: session.id.clone(),
            owner_run_id: format!("{operation}-run"),
            provider_start_idempotency_key: format!("{operation}-provider-key"),
            state: crate::product::work_item_plan_policy::RepairReservationState::Reserved,
            commit_id: None,
        };
        let persisted = store
            .compare_and_save_policy_route(
                &session,
                PolicyRoutePersist {
                    status: WorkspaceSessionStatus::Running,
                    run_history: session.run_history.clone(),
                    scope: session.review_invocation_scope.clone(),
                    gate: None,
                    diagnostics: Vec::new(),
                    repair_reservation: Some(reservation.clone()),
                    provider_start_ledger: Vec::new(),
                },
            )
            .expect("persist interrupted operation");
        let history_before = persisted.run_history.clone();
        let events_before = store
            .load_timeline_nodes_for_issue_session(
                &persisted.project_id,
                &persisted.issue_id,
                &persisted.id,
            )
            .expect("events before recovery");

        assert!(store
            .claim_provider_start(&persisted.id, &reservation.provider_start_idempotency_key)
            .expect("initial start claim"));
        assert!(!store
            .claim_provider_start(&persisted.id, &reservation.provider_start_idempotency_key)
            .expect("recovery replay claim"));
        let restored = store.get_workspace_session(&persisted.id).expect("restore");
        assert_eq!(restored.repair_reservation, Some(reservation));
        assert_eq!(restored.run_history, history_before);
        assert_eq!(
            restored
                .provider_start_ledger
                .iter()
                .filter(|entry| entry.started)
                .map(|entry| entry.provider_start_idempotency_key.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            1,
            "{operation}"
        );
        assert_eq!(
            store
                .load_timeline_nodes_for_issue_session(
                    &persisted.project_id,
                    &persisted.issue_id,
                    &persisted.id,
                )
                .expect("events after recovery"),
            events_before,
            "{operation}"
        );
    }
}
