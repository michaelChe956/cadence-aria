#[tokio::test]
async fn coding_runtime_handoff_revision_history_refresh_uses_real_run_and_handoff_artifacts() {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let session = lifecycle
        .create_workspace_session(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: fixture.attempt.project_id.clone(),
                issue_id: fixture.attempt.issue_id.clone(),
                entity_id: "work_item_plan_0001".to_string(),
                workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
        )
        .expect("plan workspace session");
    lifecycle
        .save_artifact_versions(&session.id, &[base_revision_history_artifact()])
        .expect("initial history artifact");

    fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .expect("runtime impact");
    let history = crate::web::workspace_ws_handler::refresh_coding_runtime_revision_history(
        &fixture.store.paths(),
        &fixture.attempt,
        None,
    )
    .expect("runtime history refresh");
    let replayed = crate::web::workspace_ws_handler::refresh_coding_runtime_revision_history(
        &fixture.store.paths(),
        &fixture.attempt,
        None,
    )
    .expect("runtime history replay");

    assert_eq!(history, replayed);
    assert!(history.entries.iter().any(|entry| {
        entry.kind == crate::web::workspace_ws_types::WorkItemHistoryEntryKind::UnitRun
            && entry.id.starts_with("coding_unit_run_")
            && entry.logical_work_item_id == "wi_registration"
            && entry.related_revision_id.as_deref() == Some("work_item_revision_wi02_v1")
    }));
    assert!(history.entries.iter().any(|entry| {
        entry.kind == crate::web::workspace_ws_types::WorkItemHistoryEntryKind::HandoffRevision
            && entry.id == "handoff_revision_0002"
            && entry.logical_work_item_id == "wi_core"
            && entry.related_revision_id.as_deref() == Some("work_item_revision_wi01_v2")
    }));
    let persisted = lifecycle
        .list_artifact_versions(&session.id)
        .expect("persisted history")
        .into_iter()
        .find_map(|version| match version.payload {
            crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory { history } => {
                Some(*history)
            }
            _ => None,
        })
        .expect("history payload");
    assert_eq!(persisted, history);
}

#[tokio::test]
async fn plan_repair_child_bootstrap_persists_broadcasts_and_recovers_authoritative_history() {
    let mut fixture = plan_repair_history_bootstrap_fixture(true, false).await;

    fixture
        .child_engine
        .ensure_plan_repair_artifacts()
        .await
        .expect("repair artifacts");

    let history = match fixture.child_rx.try_recv().expect("history artifact event") {
        crate::product::workspace_engine::EngineEvent::ArtifactUpdate {
            payload:
                crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory {
                    history,
                },
            ..
        } => *history,
        _ => panic!("expected history artifact update"),
    };
    assert!(
        history
            .entries
            .iter()
            .any(|entry| entry.id == fixture.trigger_run_id)
    );
    assert!(
        history
            .entries
            .iter()
            .any(|entry| entry.id == "handoff_revision_0002")
    );
    assert!(fixture
        .lifecycle
        .list_artifact_versions(&fixture.child.id)
        .expect("child artifacts")
        .iter()
        .any(|version| matches!(
            version.payload,
            crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory { .. }
        )));

    let restored = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            fixture.runtime.store.paths().issue_lifecycle_root(
                &fixture.runtime.attempt.project_id,
                &fixture.runtime.attempt.issue_id,
            ),
        )),
        fixture.lifecycle,
        mpsc::channel(8).0,
        crate::product::workspace_engine::WorkspaceSession::from_record(fixture.child),
    );
    let crate::web::workspace_ws_types::WsOutMessage::SessionState {
        artifact_versions, ..
    } = restored.build_session_state()
    else {
        panic!("expected session state");
    };
    assert!(artifact_versions.iter().any(|version| matches!(
        version.payload,
        crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory { .. }
    )));
}

#[tokio::test]
async fn plan_repair_child_bootstrap_rejects_missing_authoritative_base_history() {
    assert_plan_repair_history_bootstrap_fails(false).await;
}

#[tokio::test]
async fn plan_repair_child_bootstrap_rejects_corrupt_authoritative_base_history() {
    assert_plan_repair_history_bootstrap_fails(true).await;
}

async fn assert_plan_repair_history_bootstrap_fails(corrupt: bool) {
    let mut fixture = plan_repair_history_bootstrap_fixture(false, true).await;
    if corrupt {
        let path = fixture
            .lifecycle
            .workspace_timeline_root_for_issue_session(
                &fixture.runtime.attempt.project_id,
                &fixture.runtime.attempt.issue_id,
                &fixture.parent_session_id,
            )
            .expect("parent timeline")
            .join("artifact_versions.json");
        std::fs::create_dir_all(path.parent().expect("artifact parent")).unwrap();
        std::fs::write(path, b"{corrupt-json").unwrap();
    }

    let result = fixture.child_engine.ensure_plan_repair_artifacts().await;

    assert!(result.is_err());
    assert!(
        fixture
            .lifecycle
            .list_artifact_versions(&fixture.child.id)
            .unwrap()
            .is_empty()
    );
    assert!(fixture.child_rx.try_recv().is_err());
}

struct PlanRepairHistoryBootstrapFixture {
    runtime: RuntimeHandoffFixture,
    lifecycle: LifecycleStore,
    parent_session_id: String,
    child: crate::product::models::WorkspaceSessionRecord,
    child_engine: crate::product::workspace_engine::WorkspaceEngine,
    child_rx: mpsc::Receiver<crate::product::workspace_engine::EngineEvent>,
    trigger_run_id: String,
}

async fn plan_repair_history_bootstrap_fixture(
    with_base_history: bool,
    with_older_base_history: bool,
) -> PlanRepairHistoryBootstrapFixture {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::CompatibleExtension,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let parent = lifecycle
        .create_workspace_session(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: fixture.attempt.project_id.clone(),
                issue_id: fixture.attempt.issue_id.clone(),
                entity_id: "work_item_plan_0001".to_string(),
                workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
        )
        .expect("plan parent session");
    if with_base_history {
        lifecycle
            .save_artifact_versions(&parent.id, &[base_revision_history_artifact()])
            .expect("base history");
    }
    let trigger_unit = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("coding units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "wi_registration")
        .expect("registration unit");
    let trigger_run = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &trigger_unit.id)
        .expect("unit runs")
        .into_iter()
        .max_by_key(|run| run.execution_no)
        .expect("latest registration run");
    let binding = fixture
        .store
        .get_plan_binding(&fixture.attempt)
        .expect("authoritative plan binding");
    let (parent_tx, _parent_rx) = mpsc::channel(16);
    let mut parent_engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            fixture.store.paths().issue_lifecycle_root(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
            ),
        )),
        lifecycle.clone(),
        parent_tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(parent),
    );
    let child = parent_engine
        .start_plan_repair(crate::product::models::PlanRepairRequest {
            id: "plan_repair_request_history".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            base_plan_revision_id: binding.bound_plan_revision_id,
            trigger_attempt_id: fixture.attempt.id.clone(),
            trigger_unit_run_id: trigger_run.id.clone(),
            trigger_review_id: Some("code_review_history".to_string()),
            trigger_finding_id: "finding_history".to_string(),
            amendment_id: None,
            defect_class: crate::product::models::PlanDefectClass::UpstreamContractInvalid,
            reason_code: "contract_mismatch".to_string(),
            repair_target: crate::product::models::RepairTarget {
                kind: crate::product::models::RepairTargetKind::UpstreamWorkItem,
                logical_work_item_ids: vec!["wi_core".to_string()],
                work_item_revision_ids: vec!["work_item_revision_wi01_v2".to_string()],
            },
            contract_refs: vec!["registration_contract".to_string()],
            capability_refs: vec!["registration_ready".to_string()],
            evidence: Vec::new(),
            fingerprint: "repair_history_fingerprint".to_string(),
            status: crate::product::models::PlanRepairRequestStatus::Open,
            created_at: "2026-07-20T00:00:00Z".to_string(),
            updated_at: "2026-07-20T00:00:00Z".to_string(),
        })
        .await
        .expect("start repair");
    if with_older_base_history {
        let mut older = lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: fixture.attempt.project_id.clone(),
                    issue_id: fixture.attempt.issue_id.clone(),
                    entity_id: "work_item_plan_0001".to_string(),
                    workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                    author_provider: ProviderName::Codex,
                    reviewer_provider: ProviderName::ClaudeCode,
                    review_rounds: 1,
                    superpowers_enabled: true,
                    openspec_enabled: true,
                },
            )
            .expect("older plan session");
        older.created_at = "2026-07-19T00:00:00Z".to_string();
        older.updated_at = older.created_at.clone();
        let path = fixture
            .store
            .paths()
            .issue_lifecycle_root(&fixture.attempt.project_id, &fixture.attempt.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", older.id));
        std::fs::write(path, serde_json::to_vec_pretty(&older).unwrap()).unwrap();
        lifecycle
            .save_artifact_versions(&older.id, &[base_revision_history_artifact()])
            .expect("older base history");
    }
    let (child_tx, child_rx) = mpsc::channel(16);
    let child_engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            fixture.store.paths().issue_lifecycle_root(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
            ),
        )),
        lifecycle.clone(),
        child_tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(child.clone()),
    );

    PlanRepairHistoryBootstrapFixture {
        runtime: fixture,
        lifecycle,
        parent_session_id: parent_engine.session().session_id.clone(),
        child,
        child_engine,
        child_rx,
        trigger_run_id: trigger_run.id,
    }
}

fn base_revision_history_artifact() -> crate::web::workspace_ws_types::ArtifactVersion {
    crate::web::workspace_ws_types::ArtifactVersion {
        version: 1,
        payload: crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory {
            history: Box::new(crate::web::workspace_ws_types::WorkItemRevisionHistoryDto {
                entries: vec![crate::web::workspace_ws_types::WorkItemHistoryEntryDto {
                    kind: crate::web::workspace_ws_types::WorkItemHistoryEntryKind::WorkItemRevision,
                    id: "work_item_revision_wi02_v1".to_string(),
                    logical_work_item_id: "wi_registration".to_string(),
                    related_revision_id: None,
                    summary: "Compiled WorkItem revision".to_string(),
                    created_at: "2026-07-20T00:00:00Z".to_string(),
                }],
            }),
        },
        generated_by: ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: false,
        created_at: "2026-07-20T00:00:00Z".to_string(),
        source_node_id: "timeline_node_compile".to_string(),
    }
}
