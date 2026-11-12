use crate::product::lifecycle_store::PolicyRoutePersist;

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
            reviewer_provider: Some(ProviderName::ClaudeCode),

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
        candidate_recovery: None,
        findings: vec![crate::product::work_item_plan_policy::ClassifiedFinding {
            class,
            fingerprint: crate::product::work_item_plan_policy::FindingFingerprint::for_finding(
                None,
                class,
                "需要人工决定范围",
                Some("scope"),
            ),
            category: Some(
                crate::product::work_item_plan_policy::ReviewFindingCategory::ScopeConflict,
            ),
            severity: "blocking".to_string(),
            message: "需要人工决定范围".to_string(),
            evidence: Some("冲突证据".to_string()),
            required_action: Some("确认范围".to_string()),
            contract_field: Some("scope".to_string()),
            identity_unstable: false,
        }],
        repeated_fingerprints: Vec::new(),
        attempts_used: 2,
        manual_repairs_remaining: 1,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable,
        accepted_feedback_turns: None,
    }
}

fn recovery_engine(
    checkpoints: Arc<CheckpointStore>,
    store: LifecycleStore,
    record: WorkspaceSessionRecord,
) -> WorkspaceEngine {
    let (events, _events_rx) = mpsc::channel(8);
    WorkspaceEngine::new_persistent(
        checkpoints,
        store,
        events,
        WorkspaceSession::from_record(record),
    )
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
                single_candidate_phase: None,
                run_history: parent.run_history.clone(),
                scope: Some(
                    crate::product::work_item_plan_policy::ReviewInvocationScope::initial(format!(
                        "outline:{entity_id}"
                    )),
                ),
                gate: Some(recovery_gate(resumable)),
                diagnostics: Vec::new(),
                repair_reservation: None,
                provider_start_ledger: Vec::new(),
                author_rewrite_budget: parent.author_rewrite_budget.clone(),
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
    let ledger = vec![
        crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
            provider_start_idempotency_key: "provider-start-1".to_string(),
            started: true,
            provider: None,
            started_at: None,
        },
    ];
    let saved = store
        .compare_and_save_policy_route(
            &expected,
            PolicyRoutePersist {
                status: WorkspaceSessionStatus::Running,
                single_candidate_phase: None,
                run_history: expected.run_history.clone(),
                scope: expected.review_invocation_scope.clone(),
                gate: expected.human_gate_snapshot.clone(),
                diagnostics: expected.policy_diagnostics.clone(),
                repair_reservation: Some(reservation.clone()),
                provider_start_ledger: ledger.clone(),
                author_rewrite_budget: expected.author_rewrite_budget.clone(),
            },
        )
        .expect("CAS save");
    assert_eq!(saved.repair_reservation, Some(reservation));
    assert_eq!(saved.provider_start_ledger, ledger);
    assert_eq!(
        store
            .get_workspace_session(&expected.id)
            .unwrap()
            .provider_start_ledger,
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
                single_candidate_phase: None,
                run_history: stale.run_history.clone(),
                scope: stale.review_invocation_scope.clone(),
                gate: Some(recovery_gate(true)),
                diagnostics: Vec::new(),
                repair_reservation: None,
                provider_start_ledger: Vec::new(),
                author_rewrite_budget: stale.author_rewrite_budget.clone(),
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

    // 锚定语义下 SC policy 路由需要 durable 候选 ref（无 scope 无候选即
    // protocol violation）；为该 CAS 重演夹具提供确定性的候选 ref。
    let mut anchored = concurrent.clone();
    anchored.plan_candidate_ir_ref = Some(
        "project/project_0001/issue/issue_0001/plan/work_item_plan_cas_conflict/plan_candidate_ir/ir-cas-conflict"
            .to_string(),
    );
    write_json(
        &store
            .app_paths()
            .issue_root(&anchored.project_id, &anchored.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", anchored.id)),
        &anchored,
    )
    .expect("persist anchored candidate ref");

    let mut engine = recovery_engine(checkpoints, store.clone(), anchored);
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
        persisted.run_history.review_cycles["sc:candidate:conflict"].initial_count, 1,
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
                single_candidate_phase: None,
                run_history: parent.run_history.clone(),
                scope: Some(
                    crate::product::work_item_plan_policy::ReviewInvocationScope::initial(
                        "outline:awaiting".to_string(),
                    ),
                ),
                gate: Some(gate.clone()),
                diagnostics: Vec::new(),
                repair_reservation: None,
                provider_start_ledger: Vec::new(),
                author_rewrite_budget: parent.author_rewrite_budget.clone(),
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
                single_candidate_phase: None,
                run_history: history,
                scope: Some(
                    crate::product::work_item_plan_policy::ReviewInvocationScope::initial(
                        "outline:takeover".to_string(),
                    ),
                ),
                gate: Some(recovery_gate(true)),
                diagnostics: Vec::new(),
                repair_reservation: None,
                provider_start_ledger: Vec::new(),
                author_rewrite_budget: parent.author_rewrite_budget.clone(),
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
    assert_eq!(child.status, WorkspaceSessionStatus::WaitingForHuman);
    assert_eq!(child.human_gate_snapshot, persisted.human_gate_snapshot);
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
fn conversational_gate_takeover_inherits_snapshot_budget_and_refs() {
    let (_tmp, store, _checkpoints) = recovery_store();
    let parent = stopped_takeover_parent(&store, "work_item_plan_conversational_inherit", true);
    store
        .append_workspace_message(
            &parent.id,
            "assistant".to_string(),
            "# Candidate\n\n## Work item\n\ncontent".to_string(),
        )
        .expect("persist candidate context");
    let parent = store
        .get_workspace_session(&parent.id)
        .expect("reload stopped parent");
    let parent_path = store
        .app_paths()
        .issue_lifecycle_root(&parent.project_id, &parent.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", parent.id));
    let parent_bytes_before = std::fs::read(&parent_path).expect("read parent bytes");

    let child = store
        .takeover_stopped_needs_human(&parent.id)
        .expect("takeover stopped run");
        assert_eq!(
        child.run_policy,
        crate::product::work_item_plan_policy::RunPolicy::Interactive
    );
    assert_eq!(child.flow_kind, parent.flow_kind);
    assert_eq!(child.human_gate_snapshot, parent.human_gate_snapshot);
    assert_eq!(child.run_history, parent.run_history);
    assert_eq!(
        child.review_invocation_scope,
        parent.review_invocation_scope
    );
    assert_eq!(child.policy_diagnostics, parent.policy_diagnostics);
    assert_eq!(
        child.work_item_plan_source_revision_ref,
        parent.work_item_plan_source_revision_ref
    );
    assert_eq!(child.plan_candidate_ir_ref, parent.plan_candidate_ir_ref);
    assert_eq!(child.mechanical_report_ref, parent.mechanical_report_ref);
    assert_eq!(
        child.publication_provenance_ref,
        parent.publication_provenance_ref
    );
    assert_eq!(child.messages, parent.messages);
    assert_eq!(
        std::fs::read(&parent_path).expect("read parent after takeover"),
        parent_bytes_before
    );
}

#[test]
fn conversational_gate_takeover_replay_returns_same_child_without_parent_mutation() {
    let (_tmp, store, _checkpoints) = recovery_store();
    let parent = stopped_takeover_parent(&store, "work_item_plan_conversational_replay", true);
    let parent_path = store
        .app_paths()
        .issue_lifecycle_root(&parent.project_id, &parent.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", parent.id));
    let parent_bytes_before = std::fs::read(&parent_path).expect("read parent bytes");

    let first = store
        .takeover_stopped_needs_human(&parent.id)
        .expect("first takeover");
    let child_path = store
        .app_paths()
        .issue_lifecycle_root(&first.project_id, &first.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", first.id));
    let child_bytes_before = std::fs::read(&child_path).expect("read child bytes");
    let event = store
        .get_human_gate_takeover_event(&parent.id)
        .expect("read takeover event")
        .expect("takeover event exists");
    let event_path = store
        .app_paths()
        .issue_lifecycle_root(&parent.project_id, &parent.issue_id)
        .join("human-gate-takeovers")
        .join(format!("{}.json", event.id));
    let event_bytes_before = std::fs::read(&event_path).expect("read event bytes");

    let second = store
        .takeover_stopped_needs_human(&parent.id)
        .expect("replayed takeover");
    assert_eq!(second.id, first.id);
    assert_eq!(
        std::fs::read(&parent_path).expect("read parent after replay"),
        parent_bytes_before
    );
    assert_eq!(
        std::fs::read(&child_path).expect("read child after replay"),
        child_bytes_before
    );
    assert_eq!(
        std::fs::read(&event_path).expect("read event after replay"),
        event_bytes_before
    );
    assert_eq!(
        store
            .list_workspace_sessions(&parent.project_id, &parent.issue_id)
            .expect("list sessions")
            .len(),
        2
    );
}

#[test]
fn conversational_gate_takeover_rejects_fatal_or_persistence_diagnostic() {
    for diagnostic_code in ["state_corruption", "persistence_failure"] {
        let (_tmp, store, _checkpoints) = recovery_store();
        let parent = stopped_takeover_parent(
            &store,
            &format!("work_item_plan_conversational_{diagnostic_code}"),
            true,
        );
        let parent = store
            .compare_and_save_policy_route(
                &parent,
                PolicyRoutePersist {
                    status: WorkspaceSessionStatus::StoppedNeedsHuman,
                    single_candidate_phase: None,
                    run_history: parent.run_history.clone(),
                    scope: parent.review_invocation_scope.clone(),
                    gate: parent.human_gate_snapshot.clone(),
                    diagnostics: vec![crate::product::work_item_plan_policy::PolicyDiagnostic {
                        code: diagnostic_code.to_string(),
                        message: "fatal takeover diagnostic".to_string(),
                        field: None,
                    }],
                    repair_reservation: None,
                    provider_start_ledger: Vec::new(),
                    author_rewrite_budget: parent.author_rewrite_budget.clone(),
                },
            )
            .expect("persist fatal diagnostic");
        let error = store
            .takeover_stopped_needs_human(&parent.id)
            .expect_err("fatal diagnostic must reject takeover");
        assert!(matches!(
            error,
            ProductStoreError::InvalidRecord {
                kind: "human_gate_takeover",
                ..
            }
        ));
        assert!(
            store
                .get_human_gate_takeover_event(&parent.id)
                .expect("read rejected event")
                .is_none()
        );
        assert_eq!(
            store
                .list_workspace_sessions(&parent.project_id, &parent.issue_id)
                .expect("list sessions after rejection")
                .len(),
            1
        );
    }
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
    assert!(
        store
            .get_human_gate_takeover_event(&parent.id)
            .expect("read absent event")
            .is_none()
    );
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
    let non_resumable =
        stopped_takeover_parent(&store, "work_item_plan_takeover_not_resumable", false);

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
        assert!(
            store
                .get_human_gate_takeover_event(&parent.id)
                .expect("invalid parent must not create an event")
                .is_none()
        );
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
                    single_candidate_phase: None,
                    run_history: parent.run_history.clone(),
                    scope: None,
                    gate: None,
                    diagnostics: Vec::new(),
                    repair_reservation: None,
                    provider_start_ledger: Vec::new(),
                    author_rewrite_budget: parent.author_rewrite_budget.clone(),
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
                    single_candidate_phase: None,
                    run_history: session.run_history.clone(),
                    scope: session.review_invocation_scope.clone(),
                    gate: None,
                    diagnostics: Vec::new(),
                    repair_reservation: Some(reservation.clone()),
                    provider_start_ledger: Vec::new(),
                    author_rewrite_budget: session.author_rewrite_budget.clone(),
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
        assert!(
            store
                .claim_provider_start(&persisted.id, &reservation.provider_start_idempotency_key)
                .expect("initial start claim")
        );
        assert!(
            !store
                .claim_provider_start(&persisted.id, &reservation.provider_start_idempotency_key)
                .expect("recovery replay claim")
        );
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

// ---------------------------------------------------------------------------
// A2a（Task 2.5）：durable 等待 CAS 与 Replay 优先 claim（anchor_t06）
// ---------------------------------------------------------------------------

fn anchor_t06_business() -> crate::product::models::LcRecoveryBusinessContext {
    crate::product::models::LcRecoveryBusinessContext {
        entity_id: "entity_0001".to_string(),
        plan_id: None,
        workspace_type: WorkspaceType::WorkItemPlan,
        business_repository_physical_id: None,
        business_repository_logical_id: None,
        business_checkout_id: None,
        business_repository_path: None,
        source_story_ids: Vec::new(),
        source_design_ids: Vec::new(),
        plan_options_ref: None,
        artifact_refs: Vec::new(),
        review_refs: Vec::new(),
        author_provider: ProviderName::Codex,
        effective_reviewer_provider: Some(ProviderName::ClaudeCode),
        original_node_id: Some("node_0001".to_string()),
        original_role_run_seq: Some(1),
    }
}

fn anchor_t06_operation(
    operation_id: &str,
    state: crate::product::models::LcFreshOperationState,
) -> crate::product::models::LcFreshOperationResult {
    crate::product::models::LcFreshOperationResult {
        operation_id: operation_id.to_string(),
        state,
        fresh_node_id: None,
        reason_code: (state == crate::product::models::LcFreshOperationState::Failed)
            .then(|| "lc_provider_prepare_failed".to_string()),
        role_run_seq: None,
        accepted_at: "2026-10-11T00:03:00Z".to_string(),
        updated_at: "2026-10-11T00:03:00Z".to_string(),
    }
}

fn anchor_t06_recovery(
    recovery_id: &str,
    state: crate::product::models::LcResumeRecoveryState,
    operation_id: Option<&str>,
    operations: Vec<crate::product::models::LcFreshOperationResult>,
) -> crate::product::models::LcResumeRecoveryRecord {
    crate::product::models::LcResumeRecoveryRecord {
        schema_version: 1,
        recovery_id: recovery_id.to_string(),
        failed_provider_node_id: "node_0001".to_string(),
        reason_code: "resume_fingerprint_mismatch".to_string(),
        role: crate::product::models::ProviderConversationRole::Author,
        provider: ProviderName::Codex,
        superseded_native_session_id: Some("native_session_0001".to_string()),
        previous_fingerprint_digest: Some("sha256:full_projection".to_string()),
        previous_session_status: WorkspaceSessionStatus::Running,
        previous_stage: "running".to_string(),
        run_context: crate::product::models::LcRecoveryRunContext::Author {
            business: anchor_t06_business(),
            original_content: None,
        },
        state,
        operation_id: operation_id.map(str::to_string),
        fresh_node_id: None,
        fresh_native_session_id: None,
        fresh_role_run_seq: None,
        fresh_operations: operations,
        detected_at: "2026-10-11T00:02:00Z".to_string(),
        updated_at: "2026-10-11T00:03:00Z".to_string(),
    }
}

fn anchor_t06_session_json_path(root: &std::path::Path, session_id: &str) -> std::path::PathBuf {
    fn visit(dir: &std::path::Path, file_name: &str, found: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, file_name, found);
            } else if path.file_name().is_some_and(|name| name == file_name) {
                found.push(path);
            }
        }
    }
    let file_name = format!("{session_id}.json");
    let mut found = Vec::new();
    visit(root, &file_name, &mut found);
    assert_eq!(found.len(), 1, "session json fixture 必须唯一: {file_name}");
    found.remove(0)
}

/// 直接在 durable session JSON 上播种多代 tombstone（A1a 字面量合同）；
/// claim/CAS 实现不得增删改这些代际证据。
fn anchor_t06_seed_tombstones(root: &std::path::Path, session_id: &str, generations: &[&str]) {
    let path = anchor_t06_session_json_path(root, session_id);
    let mut record: WorkspaceSessionRecord =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read session json fixture"))
            .expect("parse session json fixture");
    record.superseded_native_sessions = generations
        .iter()
        .enumerate()
        .map(
            |(index, recovery_id)| crate::product::models::SupersededNativeSessionRecord {
                role: crate::product::models::ProviderConversationRole::Author,
                provider: ProviderName::Codex,
                native_session_id: format!("native_superseded_{index}"),
                reason_code: "resume_fingerprint_mismatch".to_string(),
                recovery_id: (*recovery_id).to_string(),
                superseded_at: format!("2026-10-11T00:0{}:00Z", index + 2),
            },
        )
        .collect();
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&record).expect("serialize session json fixture"),
    )
    .expect("write session json fixture");
}

#[test]
fn anchor_t06_claim_replay_precedes_waiting_and_busy() {
    use crate::product::lifecycle_store::workspace::LcFreshClaimOutcome;
    use crate::product::models::{LcFreshOperationState, LcResumeRecoveryState};

    let (tmp, store, _checkpoints) = recovery_store();
    let parent = stopped_takeover_parent(&store, "entity_a2a_replay", true);
    let gate_before = parent.human_gate_snapshot.clone();
    assert!(gate_before.is_some(), "复用人审 fixture：非 LC 字段基线");

    anchor_t06_seed_tombstones(tmp.path(), &parent.id, &["recovery_gen1", "recovery_gen2"]);
    let before = store
        .get_workspace_session(&parent.id)
        .expect("reload session with tombstones");
    assert_eq!(before.superseded_native_sessions.len(), 2);

    // Waiting 出生：整记录 expected CAS 只替换 LC 字段。
    let seeded = store
        .compare_and_save_lc_resume_state(
            &before,
            anchor_t06_recovery(
                "recovery_gen2",
                LcResumeRecoveryState::Waiting,
                None,
                Vec::new(),
            ),
        )
        .expect("CAS 出生 waiting recovery");
    assert_eq!(
        seeded.lc_resume_recovery.as_ref().expect("recovery").state,
        LcResumeRecoveryState::Waiting
    );
    assert_eq!(
        seeded.superseded_native_sessions.len(),
        2,
        "tombstone 多代保留"
    );
    assert_eq!(
        seeded.human_gate_snapshot, gate_before,
        "expected CAS 保留非 LC 字段"
    );
    // 过期 expected 的整记录 CAS 冲突（single-flight）。
    assert!(
        store
            .compare_and_save_lc_resume_state(
                &before,
                anchor_t06_recovery(
                    "recovery_gen2",
                    LcResumeRecoveryState::Waiting,
                    None,
                    Vec::new()
                ),
            )
            .is_err(),
        "过期 expected 必须冲突拒收"
    );

    // Waiting 新 ID 受理 → FreshReserved（provider start/spawn 在 store 之外）。
    let first_claim = store
        .claim_lc_fresh(&parent.id, "recovery_gen2", "op_prepare_1")
        .expect("首个 fresh claim 受理");
    let LcFreshClaimOutcome::Claimed {
        record: reserved,
        operation,
    } = first_claim
    else {
        panic!("waiting 下新 ID 必须受理为 Claimed，得到 {first_claim:?}");
    };
    assert_eq!(operation.state, LcFreshOperationState::Reserved);
    assert_eq!(
        reserved
            .lc_resume_recovery
            .as_ref()
            .expect("recovery")
            .state,
        LcResumeRecoveryState::FreshReserved
    );

    // prepare 失败回 Waiting：结果保留，同 ID 只 Replay；新 ID 才能再受理。
    let failed_recovery = anchor_t06_recovery(
        "recovery_gen2",
        LcResumeRecoveryState::Waiting,
        Some("op_prepare_1"),
        vec![anchor_t06_operation(
            "op_prepare_1",
            LcFreshOperationState::Failed,
        )],
    );
    store
        .compare_and_save_lc_resume_state(&reserved, failed_recovery)
        .expect("prepare 失败回 waiting");
    let same_failed = store
        .claim_lc_fresh(&parent.id, "recovery_gen2", "op_prepare_1")
        .expect("failed-waiting 同 ID 判定");
    let LcFreshClaimOutcome::Replay {
        operation,
        recovery_state,
    } = same_failed
    else {
        panic!("failed-waiting 同 ID 必须 Replay，得到 {same_failed:?}");
    };
    assert_eq!(operation.state, LcFreshOperationState::Failed);
    assert_eq!(recovery_state, LcResumeRecoveryState::Waiting);

    let retry_claim = store
        .claim_lc_fresh(&parent.id, "recovery_gen2", "op_retry_2")
        .expect("失败后新 ID 重试");
    let LcFreshClaimOutcome::Claimed {
        record: reserved_again,
        ..
    } = retry_claim
    else {
        panic!("waiting 下新 ID 必须再次受理，得到 {retry_claim:?}");
    };

    // FreshRunning：running 持锁期间同 ID 立即 typed Replay（只读 durable，
    // 不等待 engine mutex、不 spawn）；新 ID 才是 Busy——Replay 优先于 busy。
    let mut running_recovery = reserved_again.lc_resume_recovery.clone().expect("recovery");
    running_recovery.state = LcResumeRecoveryState::FreshRunning;
    running_recovery.fresh_node_id = Some("node_fresh_1".to_string());
    running_recovery.fresh_native_session_id = Some("native_fresh_1".to_string());
    for operation in running_recovery.fresh_operations.iter_mut() {
        if operation.operation_id == "op_retry_2" {
            operation.state = LcFreshOperationState::Running;
            operation.fresh_node_id = Some("node_fresh_1".to_string());
        }
    }
    let running = store
        .compare_and_save_lc_resume_state(&reserved_again, running_recovery)
        .expect("推进到 fresh_running");

    let same_running = store
        .claim_lc_fresh(&parent.id, "recovery_gen2", "op_retry_2")
        .expect("running 同 ID 判定");
    assert!(matches!(same_running, LcFreshClaimOutcome::Replay { .. }));
    let LcFreshClaimOutcome::Replay {
        operation,
        recovery_state,
    } = same_running
    else {
        unreachable!("上方 matches! 已锁定 Replay");
    };
    assert_eq!(operation.state, LcFreshOperationState::Running);
    assert_eq!(recovery_state, LcResumeRecoveryState::FreshRunning);

    let competing = store
        .claim_lc_fresh(&parent.id, "recovery_gen2", "op_competing_3")
        .expect("running 期间新 ID 判定");
    let LcFreshClaimOutcome::Busy {
        active_operation_id,
    } = competing
    else {
        panic!("running 期间新 ID 必须 Busy，得到 {competing:?}");
    };
    assert_eq!(active_operation_id.as_deref(), Some("op_retry_2"));

    // Resolved：同 ID Replay（卡不复活），新 ID 拒绝第二会话。
    let mut resolved_recovery = running.lc_resume_recovery.clone().expect("recovery");
    resolved_recovery.state = LcResumeRecoveryState::Resolved;
    for operation in resolved_recovery.fresh_operations.iter_mut() {
        if operation.operation_id == "op_retry_2" {
            operation.state = LcFreshOperationState::Completed;
            operation.fresh_node_id = Some("node_fresh_1".to_string());
        }
    }
    store
        .compare_and_save_lc_resume_state(&running, resolved_recovery)
        .expect("推进到 resolved");

    let same_resolved = store
        .claim_lc_fresh(&parent.id, "recovery_gen2", "op_retry_2")
        .expect("resolved 同 ID 判定");
    let LcFreshClaimOutcome::Replay {
        operation,
        recovery_state,
    } = same_resolved
    else {
        panic!("resolved 同 ID 必须 Replay，得到 {same_resolved:?}");
    };
    assert_eq!(operation.state, LcFreshOperationState::Completed);
    assert_eq!(recovery_state, LcResumeRecoveryState::Resolved);

    let after_resolved = store
        .claim_lc_fresh(&parent.id, "recovery_gen2", "op_second_session_4")
        .expect("resolved 新 ID 判定");
    let LcFreshClaimOutcome::Stale { reason_code } = after_resolved else {
        panic!("resolved 新 ID 必须拒绝，得到 {after_resolved:?}");
    };
    assert_eq!(reason_code, LcFreshClaimOutcome::STALE_RESOLVED);

    // 旧代 recovery_id（已被替换）→ Stale；无 recovery 的 session → Stale。
    let old_generation = store
        .claim_lc_fresh(&parent.id, "recovery_gen1", "op_old_gen_5")
        .expect("旧代 recovery_id 判定");
    let LcFreshClaimOutcome::Stale { reason_code } = old_generation else {
        panic!("旧 recovery_id 必须 Stale，得到 {old_generation:?}");
    };
    assert_eq!(reason_code, LcFreshClaimOutcome::STALE_SUPERSEDED);

    let plain = recovery_session(
        &store,
        "entity_a2a_plain",
        crate::product::work_item_plan_policy::RunPolicy::AutoIfValid,
    );
    let missing = store
        .claim_lc_fresh(&plain.id, "recovery_gen2", "op_missing_6")
        .expect("无 recovery session 判定");
    let LcFreshClaimOutcome::Stale { reason_code } = missing else {
        panic!("无 recovery 必须 Stale，得到 {missing:?}");
    };
    assert_eq!(reason_code, LcFreshClaimOutcome::STALE_NO_RECOVERY);

    // fault writes 不 false accept：写失败必须 Err 且不残留半受理状态。
    let fault_session = recovery_session(
        &store,
        "entity_a2a_fault",
        crate::product::work_item_plan_policy::RunPolicy::AutoIfValid,
    );
    store
        .compare_and_save_lc_resume_state(
            &fault_session,
            anchor_t06_recovery(
                "recovery_fault",
                LcResumeRecoveryState::Waiting,
                None,
                Vec::new(),
            ),
        )
        .expect("播种 fault session waiting recovery");
    let fault_dir = anchor_t06_session_json_path(tmp.path(), &fault_session.id)
        .parent()
        .expect("session 目录")
        .to_path_buf();
    let fault_perms = std::fs::metadata(&fault_dir)
        .expect("session 目录元数据")
        .permissions();
    let mut read_only = fault_perms.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&fault_dir, read_only).expect("session 目录改只读");
    let fault_claim = store.claim_lc_fresh(&fault_session.id, "recovery_fault", "op_fault_7");
    std::fs::set_permissions(&fault_dir, fault_perms).expect("恢复 session 目录权限");
    assert!(
        fault_claim.is_err(),
        "fault write 不得 false accept: {fault_claim:?}"
    );
    let claim_after_fault = store
        .claim_lc_fresh(&fault_session.id, "recovery_fault", "op_fault_7")
        .expect("fault 恢复后 claim");
    assert!(
        matches!(claim_after_fault, LcFreshClaimOutcome::Claimed { .. }),
        "fault 后无半受理残留，同 ID 首次真实受理: {claim_after_fault:?}"
    );

    // 终态对账：非 LC 字段与多代 tombstone 全程未动。
    let after = store
        .get_workspace_session(&parent.id)
        .expect("终态 reload");
    assert_eq!(after.human_gate_snapshot, gate_before);
    assert_eq!(
        after.superseded_native_sessions,
        before.superseded_native_sessions
    );
    assert_eq!(after.superseded_native_sessions.len(), 2);
}

#[test]
fn anchor_t06_competing_fresh_claims_have_one_winner() {
    use crate::product::lifecycle_store::workspace::LcFreshClaimOutcome;
    use crate::product::models::{LcFreshOperationState, LcResumeRecoveryState};
    use std::sync::Arc;

    let (_tmp, store, _checkpoints) = recovery_store();
    let parent = stopped_takeover_parent(&store, "entity_a2a_winner", true);
    let gate_before = parent.human_gate_snapshot.clone();
    let seeded = store
        .compare_and_save_lc_resume_state(
            &parent,
            anchor_t06_recovery(
                "recovery_winner",
                LcResumeRecoveryState::Waiting,
                None,
                Vec::new(),
            ),
        )
        .expect("播种 waiting recovery");
    assert_eq!(
        seeded.lc_resume_recovery.as_ref().expect("recovery").state,
        LcResumeRecoveryState::Waiting
    );

    // 真实排它锁下并发竞争：不同 operation_id 只有一个 Claimed。
    let store = Arc::new(store);
    let race: Vec<_> = ["op_race_a", "op_race_b", "op_race_c", "op_race_d"]
        .iter()
        .map(|operation_id| {
            let store = Arc::clone(&store);
            let session_id = parent.id.clone();
            let operation_id = (*operation_id).to_string();
            std::thread::spawn(move || {
                store.claim_lc_fresh(&session_id, "recovery_winner", &operation_id)
            })
        })
        .collect();
    let mut accepted_claims = 0usize;
    let mut busy_claims = 0usize;
    for handle in race {
        match handle.join().expect("竞争线程").expect("竞争 claim io") {
            LcFreshClaimOutcome::Claimed { .. } => accepted_claims += 1,
            LcFreshClaimOutcome::Busy { .. } => busy_claims += 1,
            other => panic!("竞争期只允许 Claimed/Busy，得到 {other:?}"),
        }
    }
    assert_eq!(accepted_claims, 1, "不同 ID 竞争只有一个受理");
    assert_eq!(busy_claims, 3);

    let after_race = store
        .get_workspace_session(&parent.id)
        .expect("竞争后 reload");
    let recovery = after_race.lc_resume_recovery.as_ref().expect("recovery");
    assert_eq!(recovery.state, LcResumeRecoveryState::FreshReserved);
    assert_eq!(recovery.fresh_operations.len(), 1, "只落一条受理结果");
    let winner_op = recovery.operation_id.clone().expect("受理 operation");

    // 同 ID 重发 Replay、迟到新 ID Busy：零额外受理 → 零额外 fresh start。
    let mut fresh_session_starts = 0usize;
    for operation_id in [winner_op.as_str(), "op_late_5"] {
        if let LcFreshClaimOutcome::Claimed { .. } = store
            .claim_lc_fresh(&parent.id, "recovery_winner", operation_id)
            .expect("重发判定 io")
        {
            fresh_session_starts += 1;
        }
    }
    assert_eq!(fresh_session_starts, 0, "赢家产生后零额外 fresh start");
    let same_id = store
        .claim_lc_fresh(&parent.id, "recovery_winner", &winner_op)
        .expect("同 ID 重发 io");
    assert!(matches!(same_id, LcFreshClaimOutcome::Replay { .. }));
    let LcFreshClaimOutcome::Replay {
        operation,
        recovery_state,
    } = &same_id
    else {
        unreachable!("上方 matches! 已锁定 Replay");
    };
    assert_eq!(operation.operation_id, winner_op);
    assert_eq!(operation.state, LcFreshOperationState::Reserved);
    assert_eq!(*recovery_state, LcResumeRecoveryState::FreshReserved);

    // query_operation 只读：durable 最新态，record 全等不变。
    let before_query = store
        .get_workspace_session(&parent.id)
        .expect("查询前 reload");
    let queried = store
        .query_lc_fresh_operation(&parent.id, "recovery_winner", &winner_op)
        .expect("查询 io")
        .expect("命中已受理 operation");
    assert_eq!(queried.operation_id, winner_op);
    assert_eq!(queried.state, LcFreshOperationState::Reserved);
    assert!(
        store
            .query_lc_fresh_operation(&parent.id, "recovery_winner", "op_unknown_6")
            .expect("查询 io")
            .is_none(),
        "未受理 operation 查询为 None"
    );
    assert!(
        store
            .query_lc_fresh_operation(&parent.id, "recovery_missing", &winner_op)
            .expect("查询 io")
            .is_none(),
        "recovery 身份不匹配查询为 None"
    );
    let after_query = store
        .get_workspace_session(&parent.id)
        .expect("查询后 reload");
    assert_eq!(after_query, before_query, "query_operation 只读不写 record");

    // 非容忍回归：expected CAS 全程保留人审快照。
    assert_eq!(after_query.human_gate_snapshot, gate_before);
}
