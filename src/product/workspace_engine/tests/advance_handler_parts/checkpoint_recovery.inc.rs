#[tokio::test]
async fn advance_checkpoint_recovery_reuses_prepared_attempt_and_unit_ids() {
    let root = tempfile::tempdir().unwrap();
    crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .expect("seed authoritative advance fixture");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    let seeded_attempt = coding_store
        .get_attempt_for_work_item_group(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            None,
        )
        .unwrap()
        .expect("seeded group attempt");
    coding_store
        .delete_attempt("project_0001", "issue_plan_0001", &seeded_attempt.id)
        .unwrap();
    seed_advance_draft_records(&app_paths);
    let authoritative = coding_store
        .resolve_authoritative_group_plan_binding_for_revision(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            "plan_revision_0001",
        )
        .unwrap();
    let repository = RepositoryStore::new(app_paths.clone())
        .list("project_0001")
        .unwrap()
        .into_iter()
        .next()
        .expect("fixture repository");
    let group = coding_store
        .prepare_group_initialization_with_admission(
            &CreateGroupCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_plan_0001".to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                current_work_item_id: authoritative.units[0].logical_work_item_id.clone(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/issues/issue_plan_0001".to_string(),
                worktree_path: Some(root.path().join("worktree")),
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 2,
                start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            },
            "plan_revision_0001",
            &authoritative.units,
            crate::product::coding_models::CodingAdmissionKind::ScAdvance,
        )
        .unwrap();
    let expected_attempt_id = group.attempt.id.clone();
    let expected_unit_ids = group
        .units
        .iter()
        .map(|unit| unit.id.clone())
        .collect::<Vec<_>>();
    let expected_lease_id = group.worktree_lease_id.clone();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session_record = lifecycle
        .list_workspace_sessions("project_0001", "issue_plan_0001")
        .unwrap()
        .into_iter()
        .find(|session| {
            session.workspace_type == WorkspaceType::WorkItemPlan
                && session.entity_id == "work_item_plan_0001"
        })
        .expect("fixture plan workspace session");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let outcome = engine
        .handle_advance(AdvanceInput {
            command_id: "command_checkpoint_prepared".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_plan_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
        })
        .await
        .expect("resume prepared checkpoint");
    assert!(matches!(outcome, AdvanceOutcome::Completed { .. }));
    let persisted_attempts = coding_store
        .list_attempts_for_issue("project_0001", "issue_plan_0001")
        .unwrap();
    assert_eq!(persisted_attempts.len(), 1);
    assert_eq!(persisted_attempts[0].id, expected_attempt_id);
    assert_eq!(
        persisted_attempts[0].admission_kind,
        crate::product::coding_models::CodingAdmissionKind::ScAdvance
    );
    let persisted_units = coding_store
        .list_coding_units("project_0001", "issue_plan_0001", &expected_attempt_id)
        .unwrap();
    assert_eq!(
        persisted_units
            .iter()
            .map(|unit| unit.id.clone())
            .collect::<Vec<_>>(),
        expected_unit_ids
    );
    assert_eq!(
        coding_store
            .get_group_initialization("project_0001", "issue_plan_0001", "work_item_plan_0001")
            .unwrap()
            .worktree_lease_id,
        expected_lease_id
    );
    let replay = engine
        .handle_advance(AdvanceInput {
            command_id: "command_checkpoint_prepared_replay".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_plan_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
        })
        .await
        .expect("replay completed advance");
    assert!(
        matches!(replay, AdvanceOutcome::Replayed { record } if record.status == AdvanceStatus::Ready)
    );
    assert_eq!(
        coding_store
            .list_attempts_for_issue("project_0001", "issue_plan_0001")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(repository.id, "repository_0001");
}

#[tokio::test]
async fn advance_checkpoint_recovery_reuses_materialized_units_without_duplicates() {
    let root = tempfile::tempdir().unwrap();
    crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .expect("seed authoritative advance fixture");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    let seeded_attempt = coding_store
        .get_attempt_for_work_item_group(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            None,
        )
        .unwrap()
        .expect("seeded group attempt");
    coding_store
        .delete_attempt("project_0001", "issue_plan_0001", &seeded_attempt.id)
        .unwrap();
    seed_advance_draft_records(&app_paths);
    let authoritative = coding_store
        .resolve_authoritative_group_plan_binding_for_revision(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            "plan_revision_0001",
        )
        .unwrap();
    let worktree_path = root.path().join("worktree");
    let group_input = CreateGroupCodingAttemptInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_plan_0001".to_string(),
        plan_id: "work_item_plan_0001".to_string(),
        current_work_item_id: authoritative.units[0].logical_work_item_id.clone(),
        base_branch: "HEAD".to_string(),
        branch_name: "aria/issues/issue_plan_0001".to_string(),
        worktree_path: Some(worktree_path.clone()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            permission_modes: Default::default(),
        },
        target_snapshot: None,
        max_auto_rework: 2,
        start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
    };
    let mut group = coding_store
        .prepare_group_initialization_with_admission(
            &group_input,
            "plan_revision_0001",
            &authoritative.units,
            crate::product::coding_models::CodingAdmissionKind::ScAdvance,
        )
        .unwrap();
    let guard = coding_store
        .acquire_work_item_attempt_creation(
            "project_0001",
            "issue_plan_0001",
            &group.lock_work_item_id,
        )
        .unwrap();
    coding_store
        .ensure_group_initialization_attempt(&group, &guard)
        .unwrap();
    group = coding_store
        .advance_group_initialization_phase(
            &group,
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
        )
        .unwrap();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(
            crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                project_id: group.project_id.clone(),
                issue_id: group.issue_id.clone(),
                repository_id: "repository_0001".to_string(),
                branch_name: group.attempt.branch_name.clone(),
                worktree_path: worktree_path.clone(),
                base_branch: group.attempt.base_branch.clone(),
            },
        )
        .unwrap();
    let lease = lifecycle
        .try_acquire_issue_worktree_lock(
            &group.project_id,
            &group.issue_id,
            &group.lock_work_item_id,
            &group.worktree_lease_id,
        )
        .unwrap();
    assert!(lease.acquired);
    lifecycle
        .bind_issue_worktree_lock_to_attempt(
            &group.project_id,
            &group.issue_id,
            &group.lock_work_item_id,
            &group.attempt.id,
        )
        .unwrap();
    group = coding_store
        .advance_group_initialization_phase(
            &group,
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
        )
        .unwrap();
    coding_store
        .ensure_group_initialization_plan_binding(&group)
        .unwrap();
    group = coding_store
        .advance_group_initialization_phase(
            &group,
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
        )
        .unwrap();
    for index in 0..group.units.len() {
        coding_store
            .ensure_group_initialization_unit(&group, index)
            .unwrap();
    }
    group = coding_store
        .advance_group_initialization_phase(
            &group,
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
        )
        .unwrap();
    let expected_attempt_id = group.attempt.id.clone();
    let expected_unit_ids = group
        .units
        .iter()
        .map(|unit| unit.id.clone())
        .collect::<Vec<_>>();
    let session_record = lifecycle
        .list_workspace_sessions("project_0001", "issue_plan_0001")
        .unwrap()
        .into_iter()
        .find(|session| {
            session.workspace_type == WorkspaceType::WorkItemPlan
                && session.entity_id == "work_item_plan_0001"
        })
        .expect("fixture plan workspace session");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let outcome = engine
        .handle_advance(AdvanceInput {
            command_id: "command_checkpoint_materialized".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_plan_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
        })
        .await
        .expect("resume materialized checkpoint");
    assert!(matches!(outcome, AdvanceOutcome::Completed { .. }));
    assert_eq!(
        coding_store
            .list_attempts_for_issue("project_0001", "issue_plan_0001")
            .unwrap()
            .iter()
            .map(|attempt| attempt.id.clone())
            .collect::<Vec<_>>(),
        vec![expected_attempt_id.clone()]
    );
    assert_eq!(
        coding_store
            .list_coding_units("project_0001", "issue_plan_0001", &expected_attempt_id)
            .unwrap()
            .iter()
            .map(|unit| unit.id.clone())
            .collect::<Vec<_>>(),
        expected_unit_ids
    );
}

#[tokio::test]
async fn advance_initialization_replay_resumes_same_record_attempt_and_units() {
    // WP5 恢复矩阵（REQ-MTG-05）：单 target 七 checkpoint 全量（回归零变化锁）。
    // GroupAttemptPersisted 为 T2 落地的第 7 checkpoint（attempt 落盘后、相位
    // 推进前中断——durable 态与 JournalPrepared 的差异恰为 attempt 文件在场）。
    let checkpoints = [
        AdvanceInitializationFailpoint::RecordPersisted,
        AdvanceInitializationFailpoint::JournalPrepared,
        AdvanceInitializationFailpoint::GroupAttemptPersisted,
        AdvanceInitializationFailpoint::AttemptPersisted,
        AdvanceInitializationFailpoint::WorktreeBound,
        AdvanceInitializationFailpoint::PlanBindingSaved,
        AdvanceInitializationFailpoint::UnitsMaterialized,
    ];
    for checkpoint in checkpoints {
        let (root, app_paths, lifecycle, mut engine) = advance_fixture().await;
        let command_id = format!("command_failpoint_{checkpoint:?}");
        let request = fixture_input(&command_id);
        let _failpoint = register_advance_initialization_failpoint(
            &request,
            checkpoint,
            AdvanceInitializationFailpointMode::Crash,
        );
        let first = tokio::spawn(async move { engine.handle_advance(request).await });
        assert!(
            first.await.is_err(),
            "{checkpoint:?} must interrupt the engine"
        );

        let advance_store = AdvanceStore::new(app_paths.clone());
        let first_record = advance_store
            .get_advance_by_command_id("project_0001", "issue_plan_0001", &command_id)
            .unwrap()
            .expect("record is durable before every checkpoint");
        let first_outer = advance_store
            .get_advance_initialization(&first_record)
            .unwrap();
        let coding_store =
            crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
        let first_group = coding_store
            .get_group_initialization("project_0001", "issue_plan_0001", "work_item_plan_0001")
            .ok();
        let first_attempt_id = first_group.as_ref().map(|group| group.attempt.id.clone());
        let first_unit_ids = first_group.as_ref().map(|group| {
            group
                .units
                .iter()
                .map(|unit| unit.id.clone())
                .collect::<Vec<_>>()
        });
        match checkpoint {
            AdvanceInitializationFailpoint::RecordPersisted => {
                assert!(first_outer.is_none());
                assert!(first_group.is_none());
            }
            AdvanceInitializationFailpoint::JournalPrepared => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::JournalPrepared)
                );
                assert!(
                    first_attempt_id.as_ref().is_none_or(|id| coding_store
                        .get_attempt("project_0001", "issue_plan_0001", id)
                        .is_err()),
                    "attempt file must not be durable before the group attempt checkpoint"
                );
            }
            AdvanceInitializationFailpoint::GroupAttemptPersisted => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::JournalPrepared)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::Prepared)
                );
                assert!(
                    first_attempt_id.as_ref().is_some_and(|id| coding_store
                        .get_attempt("project_0001", "issue_plan_0001", id)
                        .is_ok()),
                    "group attempt file is durable before the phase checkpoint"
                );
            }
            AdvanceInitializationFailpoint::AttemptPersisted => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::JournalPrepared)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted)
                );
            }
            AdvanceInitializationFailpoint::WorktreeBound => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::AttemptPersisted)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound)
                );
            }
            AdvanceInitializationFailpoint::PlanBindingSaved => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::PlanBindingSaved)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved)
                );
            }
            AdvanceInitializationFailpoint::UnitsMaterialized => {
                assert_eq!(
                    first_outer.as_ref().map(|journal| journal.phase),
                    Some(AdvanceInitializationPhase::UnitsMaterialized)
                );
                assert_eq!(
                    first_group.as_ref().map(|group| group.phase),
                    Some(crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized)
                );
            }
        }

        let mut restarted = build_advance_engine(&root, lifecycle);
        let outcome = restarted
            .handle_advance(fixture_input(&command_id))
            .await
            .expect("restart must resume the interrupted initialization");
        assert!(matches!(outcome, AdvanceOutcome::Completed { .. }));
        let final_record = advance_store
            .get_advance_by_command_id("project_0001", "issue_plan_0001", &command_id)
            .unwrap()
            .expect("final record");
        assert_eq!(final_record.id, first_record.id);
        let final_group = coding_store
            .get_group_initialization("project_0001", "issue_plan_0001", "work_item_plan_0001")
            .unwrap();
        if let Some(first_attempt_id) = first_attempt_id {
            assert_eq!(final_group.attempt.id, first_attempt_id);
        }
        if let Some(first_unit_ids) = first_unit_ids {
            assert_eq!(
                final_group
                    .units
                    .iter()
                    .map(|unit| unit.id.clone())
                    .collect::<Vec<_>>(),
                first_unit_ids
            );
        }
        assert_eq!(
            coding_store
                .list_attempts_for_issue("project_0001", "issue_plan_0001")
                .unwrap()
                .len(),
            1
        );
        let replay = restarted
            .handle_advance(fixture_input(&format!("{command_id}_replay")))
            .await
            .expect("completed advance replay");
        assert!(matches!(
            replay,
            AdvanceOutcome::Replayed { record } if record.status == AdvanceStatus::Ready
        ));
        // WP5 恢复矩阵（REQ-MTG-05）：增殖审计为分流专属——单 target 全
        // checkpoint 恢复路径不落任何 split-audit 记录（回归零变化锁）。
        assert!(
            coding_store
                .get_split_audits_for_plan("project_0001", "issue_plan_0001", "work_item_plan_0001")
                .unwrap()
                .is_empty(),
            "{checkpoint:?} single-target recovery must not write split audits"
        );
    }
}
