#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateGroupCodingAttemptInput};
    use crate::product::coding_models::{
        CodingAdmissionKind, CodingExecutionAttempt, CodingExecutionUnitStatus, CodingUnitRun,
        CodingUnitRunStatus,
    };
    use crate::product::coding_workspace_engine::group_dependency_gate::GroupUnitSelectionOutcome as SelectionOutcome;
    use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, group_dependency_gate};
    use crate::product::git_workspace_service::GitWorkspaceService;
    use crate::product::models::{HandoffRevision, ProviderName};
    use crate::product::work_item_revision_store::WorkItemRevisionStore;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;
    use tokio::sync::mpsc;

    struct Fixture {
        _root: tempfile::TempDir,
        store: CodingAttemptStore,
        engine: CodingWorkspaceEngine,
        attempt: CodingExecutionAttempt,
    }

    fn fixture() -> Fixture {
        let root = tempfile::tempdir().expect("root");
        let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let attempt = store
            .create_group_attempt(CreateGroupCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                current_work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: Some(ProviderName::Fake),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .expect("group attempt");
        super::super::seed_group_attempt_fixture(&store, &attempt, true, true);
        let mut attempt = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("persisted attempt");
        attempt.admission_kind = CodingAdmissionKind::ScAdvance;
        store
            .write_coding_attempt_for_test(&attempt)
            .expect("SC admission kind");
        let (event_tx, _event_rx) = mpsc::channel(8);
        let engine =
            CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
        Fixture {
            _root: root,
            store,
            engine,
            attempt,
        }
    }

    fn unit(
        fixture: &Fixture,
        logical_work_item_id: &str,
    ) -> crate::product::coding_models::CodingExecutionUnit {
        fixture
            .store
            .list_coding_units(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("units")
            .into_iter()
            .find(|unit| unit.logical_work_item_id == logical_work_item_id)
            .expect("unit")
    }

    fn completed_run(fixture: &Fixture, logical_work_item_id: &str, completion_commit: &str) {
        let unit = unit(fixture, logical_work_item_id);
        let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
        let lineage = revision_store
            .get_plan_lineage(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &unit.plan_id,
            )
            .expect("lineage");
        let revision = revision_store
            .get_work_item_revision(
                &lineage,
                &unit.logical_work_item_id,
                &unit.work_item_revision_id,
            )
            .expect("work item revision");
        let bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
            .expect("projection bundle");
        let run = CodingUnitRun {
            id: format!("{}_run_0001", unit.id),
            unit_id: unit.id.clone(),
            execution_no: 1,
            work_item_revision_id: unit.work_item_revision_id.clone(),
            resolved_handoff_revision_ids: Vec::new(),
            canonical_contract_hash: bundle.canonical_contract_hash,
            projection_bundle_id: bundle.id,
            projection_compiler_version: bundle.compiler_version,
            coder_provider_renderer_version: "test-renderer-v1".to_string(),
            reviewer_provider_renderer_version: "test-renderer-v1".to_string(),
            internal_reviewer_provider_renderer_version: None,
            coder_projection_hash: bundle.coder_projection_hash,
            reviewer_projection_hash: bundle.reviewer_projection_hash,
            coder_execution_context_hash: None,
            reviewer_execution_context_hash: None,
            internal_reviewer_execution_context_hash: None,
            status: CodingUnitRunStatus::Completed,
            unit_rework_count: 0,
            verification_retry_count: 0,
            operational_retry_count: 0,
            plan_repair_count: 0,
            start_commit: Some("start_commit_0001".to_string()),
            completion_commit: Some(completion_commit.to_string()),
            created_at: "2026-08-31T00:00:00Z".to_string(),
            updated_at: "2026-08-31T00:00:00Z".to_string(),
        };
        fixture
            .store
            .create_coding_unit_run(&fixture.attempt, &run)
            .expect("completed run");
        fixture
            .store
            .update_coding_unit_completion_commit(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                Some(completion_commit.to_string()),
            )
            .expect("completion commit");
        let handoff = HandoffRevision {
            id: format!("handoff_revision_{}", run.id),
            logical_work_item_id: unit.logical_work_item_id.clone(),
            work_item_revision_id: unit.work_item_revision_id.clone(),
            coding_unit_run_id: run.id,
            provided_contracts: Vec::new(),
            provided_capabilities: BTreeMap::new(),
            contract_hash: "contract_hash_0001".to_string(),
            commit_sha: completion_commit.to_string(),
            created_at: "2026-08-31T00:00:00Z".to_string(),
        };
        revision_store
            .put_handoff_revision(&lineage, &handoff)
            .expect("handoff");
        fixture
            .store
            .update_coding_unit_latest_handoff_revision_id(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                Some(handoff.id),
            )
            .expect("handoff pointer");
        fixture
            .store
            .update_coding_unit_status(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                Some("completed fixture dependency".to_string()),
            )
            .expect("completed unit");
    }

    fn complete_unit_without_handoff(fixture: &Fixture, logical_work_item_id: &str) {
        let unit = unit(fixture, logical_work_item_id);
        fixture
            .store
            .update_coding_unit_status(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                Some("completed unrelated fixture unit".to_string()),
            )
            .expect("completed unrelated unit");
    }

    fn provider_ledger_count(fixture: &Fixture) -> usize {
        fixture
            .store
            .list_role_runs(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("provider ledger")
            .len()
    }

    #[tokio::test]
    async fn sc_group_dependency_waiting_clears_active_unit_and_writes_no_provider_ledger() {
        let fixture = fixture();
        complete_unit_without_handoff(&fixture, "work_item_0003");
        let current = fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("attempt");
        assert!(current.active_unit_id.is_some());
        let before = provider_ledger_count(&fixture);

        let updated = fixture
            .engine
            .advance_to_next_group_unit(&current)
            .await
            .expect("waiting advance");

        assert!(updated.active_unit_id.is_none());
        assert_eq!(
            updated.current_work_item_id.as_deref(),
            Some("work_item_0001")
        );
        let snapshot = fixture
            .store
            .get_group_dependency_gate_snapshot(&updated)
            .expect("gate snapshot")
            .expect("waiting snapshot");
        assert_eq!(
            snapshot.status,
            crate::product::coding_models::GroupDependencyGateStatus::Waiting
        );
        assert_eq!(snapshot.pending_unit_ids.len(), 1);
        assert_eq!(provider_ledger_count(&fixture), before);
    }

    #[test]
    fn sc_group_dependency_gate_blocks_consumer_until_dependency_completed_and_handoff_published() {
        let fixture = fixture();
        complete_unit_without_handoff(&fixture, "work_item_0003");
        let waiting = fixture
            .engine
            .select_next_sc_group_unit(&fixture.attempt)
            .expect("selector");
        assert!(matches!(waiting, SelectionOutcome::Waiting { .. }));

        completed_run(&fixture, "work_item_0001", "commit_0001");
        let ready = fixture
            .engine
            .select_next_sc_group_unit(&fixture.attempt)
            .expect("selector");
        let SelectionOutcome::Ready { unit_id, .. } = ready else {
            panic!("consumer should be ready after dependency handoff");
        };
        assert_eq!(unit_id, unit(&fixture, "work_item_0002").id);
    }

    #[test]
    fn sc_group_dependency_gate_waits_when_all_pending_units_are_unready() {
        let cases = [
            ("not_completed", false, false),
            ("handoff_not_published", true, false),
            ("handoff_pointer_missing", true, true),
        ];
        for (name, complete, clear_handoff) in cases {
            let fixture = fixture();
            complete_unit_without_handoff(&fixture, "work_item_0003");
            if complete && clear_handoff {
                completed_run(&fixture, "work_item_0001", "commit_0001");
                let dependency = unit(&fixture, "work_item_0001");
                fixture
                    .store
                    .update_coding_unit_latest_handoff_revision_id(
                        &fixture.attempt.project_id,
                        &fixture.attempt.issue_id,
                        &fixture.attempt.id,
                        &dependency.id,
                        Some("missing_handoff_revision".to_string()),
                    )
                    .expect("missing handoff pointer");
            } else if complete {
                complete_unit_without_handoff(&fixture, "work_item_0001");
            }
            let outcome = fixture
                .engine
                .select_next_sc_group_unit(&fixture.attempt)
                .unwrap_or_else(|error| panic!("{name}: selector failed: {error}"));
            let SelectionOutcome::Waiting {
                pending_unit_ids,
                message,
                ..
            } = outcome
            else {
                panic!("{name}: expected waiting outcome");
            };
            assert!(pending_unit_ids.contains(&unit(&fixture, "work_item_0002").id));
            assert!(!message.is_empty());
        }
    }

    #[tokio::test]
    async fn sc_group_dependency_gate_fails_closed_on_mismatched_handoff_without_start_or_provider_ledger()
     {
        let fixture = fixture();
        complete_unit_without_handoff(&fixture, "work_item_0003");
        completed_run(&fixture, "work_item_0001", "commit_0001");
        let dependency = unit(&fixture, "work_item_0001");
        let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
        let lineage = revision_store
            .get_plan_lineage(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &dependency.plan_id,
            )
            .expect("lineage");
        let mismatched_handoff = HandoffRevision {
            id: "handoff_mismatch_0001".to_string(),
            logical_work_item_id: dependency.logical_work_item_id.clone(),
            work_item_revision_id: "work_item_revision_0002".to_string(),
            coding_unit_run_id: format!("{}_run_0001", dependency.id),
            provided_contracts: Vec::new(),
            provided_capabilities: BTreeMap::new(),
            contract_hash: "contract_hash_0001".to_string(),
            commit_sha: "commit_0001".to_string(),
            created_at: "2026-08-31T00:00:00Z".to_string(),
        };
        revision_store
            .put_handoff_revision(&lineage, &mismatched_handoff)
            .expect("mismatched handoff");
        fixture
            .store
            .update_coding_unit_latest_handoff_revision_id(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &dependency.id,
                Some(mismatched_handoff.id.clone()),
            )
            .expect("mismatched pointer");
        let current = fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("attempt");
        let before = provider_ledger_count(&fixture);

        let updated = fixture
            .engine
            .advance_to_next_group_unit(&current)
            .await
            .expect("mismatch advance");

        assert!(updated.active_unit_id.is_none());
        let snapshot = fixture
            .store
            .get_group_dependency_gate_snapshot(&updated)
            .expect("gate snapshot")
            .expect("failed-closed snapshot");
        assert_eq!(
            snapshot.status,
            crate::product::coding_models::GroupDependencyGateStatus::FailedClosed
        );
        assert_eq!(
            snapshot.reason_code.as_deref(),
            Some("SC_GROUP_HANDOFF_PLAN_BINDING_MISMATCH")
        );
        assert_eq!(
            snapshot.handoff_id.as_deref(),
            Some("handoff_mismatch_0001")
        );
        assert_eq!(
            snapshot.dependency_work_item_revision_id.as_deref(),
            Some("work_item_revision_0001")
        );
        assert_eq!(
            snapshot.handoff_work_item_revision_id.as_deref(),
            Some("work_item_revision_0002")
        );
        assert_eq!(provider_ledger_count(&fixture), before);
        assert!(
            fixture
                .store
                .list_coding_unit_runs(&updated, &unit(&fixture, "work_item_0002").id)
                .expect("consumer runs")
                .is_empty()
        );
    }

    #[test]
    fn sc_group_dependency_gate_fails_closed_on_handoff_binding_mismatch() {
        let fixture = fixture();
        complete_unit_without_handoff(&fixture, "work_item_0003");
        completed_run(&fixture, "work_item_0001", "commit_0001");
        let dependency = unit(&fixture, "work_item_0001");
        let outcome = fixture
            .engine
            .select_next_sc_group_unit(&fixture.attempt)
            .expect("selector");
        assert!(matches!(outcome, SelectionOutcome::Ready { .. }));
        fixture
            .store
            .update_coding_unit_latest_handoff_revision_id(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &dependency.id,
                Some("missing_handoff_revision".to_string()),
            )
            .expect("mismatched pointer");
        let outcome = fixture
            .engine
            .select_next_sc_group_unit(&fixture.attempt)
            .expect("selector");
        let SelectionOutcome::Waiting { message, .. } = outcome else {
            panic!("missing handoff should wait");
        };
        assert!(message.contains("missing_handoff_revision"));
    }

    #[test]
    fn sc_group_dependency_gate_fails_closed_on_unknown_self_or_cycle() {
        let (_, cycle) = group_dependency_gate::topological_layers(&BTreeMap::from([
            ("A".to_string(), ["B".to_string()].into_iter().collect()),
            ("B".to_string(), ["A".to_string()].into_iter().collect()),
        ]));
        assert!(cycle);
        assert!(matches!(
            group_dependency_gate::failed_unknown("unknown", "bad"),
            SelectionOutcome::FailedClosed { reason_code, .. }
                if reason_code == "SC_GROUP_DEPENDENCY_UNKNOWN"
        ));
        assert!(matches!(
            group_dependency_gate::failed_self("self"),
            SelectionOutcome::FailedClosed { reason_code, .. }
                if reason_code == "SC_GROUP_DEPENDENCY_SELF"
        ));
    }

    #[test]
    fn sc_group_dependency_gate_returns_complete_when_no_pending_units_remain() {
        let fixture = fixture();
        for logical_id in ["work_item_0001", "work_item_0002", "work_item_0003"] {
            let unit = unit(&fixture, logical_id);
            fixture
                .store
                .update_coding_unit_status(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                    &unit.id,
                    CodingExecutionUnitStatus::Completed,
                    Some("complete fixture".to_string()),
                )
                .expect("completed unit");
        }
        assert!(matches!(
            fixture
                .engine
                .select_next_sc_group_unit(&fixture.attempt)
                .expect("selector"),
            SelectionOutcome::Complete
        ));
    }
}
