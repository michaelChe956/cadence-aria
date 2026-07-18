use std::sync::{Arc, Barrier};

use super::*;
use crate::product::coding_models::{
    CodingAmendmentApplicationJournal, CodingAmendmentApplicationPhase, CodingAttemptPlanBinding,
    CodingExecutionUnit, CodingUnitRun, CodingUnitRunStatus,
};
use crate::product::work_item_projection::RenderedExecutionContext;

pub(super) const PLAN_ID: &str = "issue_plan_0001";

pub(super) fn coding_plan_repair_attempt(store: &CodingAttemptStore) -> CodingExecutionAttempt {
    store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
            current_work_item_id: "wi_core".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            max_auto_rework: 2,
        })
        .expect("group attempt")
}

pub(super) fn coding_unit_run_unit(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    logical_work_item_id: &str,
    work_item_revision_id: &str,
    order_index: u32,
    status: CodingExecutionUnitStatus,
) -> CodingExecutionUnit {
    store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: PLAN_ID.to_string(),
            logical_work_item_id: logical_work_item_id.to_string(),
            work_item_revision_id: work_item_revision_id.to_string(),
            dependency_logical_work_item_ids: if order_index == 0 {
                Vec::new()
            } else {
                vec!["wi_core".to_string()]
            },
            order_index,
            status,
        })
        .expect("coding unit")
}

pub(super) fn coding_unit_run_record(
    id: &str,
    unit_id: &str,
    execution_no: u32,
    work_item_revision_id: &str,
    status: CodingUnitRunStatus,
) -> CodingUnitRun {
    CodingUnitRun {
        id: id.to_string(),
        unit_id: unit_id.to_string(),
        execution_no,
        work_item_revision_id: work_item_revision_id.to_string(),
        resolved_handoff_revision_ids: vec!["handoff_revision_0001".to_string()],
        canonical_contract_hash: "contract_hash".to_string(),
        projection_bundle_id: "work_item_projection_bundle_0001".to_string(),
        projection_compiler_version: "projection-v1".to_string(),
        coder_provider_renderer_version: "codex-v1".to_string(),
        reviewer_provider_renderer_version: "claude-code-v1".to_string(),
        coder_projection_hash: "coder_projection_hash".to_string(),
        reviewer_projection_hash: "reviewer_projection_hash".to_string(),
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        status,
        unit_rework_count: 2,
        verification_retry_count: 3,
        operational_retry_count: 5,
        plan_repair_count: 7,
        start_commit: Some("start_commit".to_string()),
        completion_commit: None,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    }
}

#[test]
fn coding_plan_repair_unit_binding_rejects_alias_and_invalid_dependencies() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let input =
        |logical_work_item_id: &str, dependencies: Vec<&str>| CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: PLAN_ID.to_string(),
            logical_work_item_id: logical_work_item_id.to_string(),
            work_item_revision_id: "work_item_revision_0001".to_string(),
            dependency_logical_work_item_ids: dependencies
                .into_iter()
                .map(str::to_string)
                .collect(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Pending,
        };

    for invalid in [
        input("work_item_revision_0001", Vec::new()),
        input("wi_core", vec!["wi_core"]),
        input("wi_core", vec!["wi_upstream", "wi_upstream"]),
    ] {
        assert!(matches!(
            store.create_coding_unit(invalid),
            Err(ProductStoreError::IdentityMismatch {
                kind: "coding_execution_unit_binding",
                ..
            })
        ));
    }
    assert!(
        store
            .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("units")
            .is_empty()
    );
}

#[test]
fn coding_plan_repair_statuses_expose_only_documented_active_states() {
    for status in [
        CodingAttemptStatus::Created,
        CodingAttemptStatus::Running,
        CodingAttemptStatus::WaitingForHuman,
        CodingAttemptStatus::Blocked,
        CodingAttemptStatus::AwaitingPlanAmendment,
        CodingAttemptStatus::ApplyingPlanAmendment,
        CodingAttemptStatus::AmendmentApplyFailed,
    ] {
        assert!(
            status.is_active(),
            "attempt status should be active: {status:?}"
        );
    }
    for status in [
        CodingAttemptStatus::Completed,
        CodingAttemptStatus::Failed,
        CodingAttemptStatus::Aborted,
    ] {
        assert!(
            !status.is_active(),
            "attempt status should be terminal: {status:?}"
        );
    }

    for status in [
        CodingExecutionUnitStatus::Running,
        CodingExecutionUnitStatus::WaitingForHuman,
        CodingExecutionUnitStatus::Blocked,
        CodingExecutionUnitStatus::BlockedByPlanDefect,
        CodingExecutionUnitStatus::AwaitingAmendment,
        CodingExecutionUnitStatus::NeedsRevalidation,
        CodingExecutionUnitStatus::Stale,
    ] {
        assert!(
            status.is_active(),
            "unit status should be active: {status:?}"
        );
    }
    for status in [
        CodingExecutionUnitStatus::Pending,
        CodingExecutionUnitStatus::Completed,
        CodingExecutionUnitStatus::Failed,
        CodingExecutionUnitStatus::Superseded,
        CodingExecutionUnitStatus::Skipped,
    ] {
        assert!(
            !status.is_active(),
            "unit status should be terminal: {status:?}"
        );
    }
}

#[test]
fn coding_attempt_plan_binding_is_scoped_append_only_and_idempotent() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let initial = CodingAttemptPlanBinding {
        attempt_id: attempt.id.clone(),
        plan_id: PLAN_ID.to_string(),
        bound_plan_revision_id: "plan_revision_0001".to_string(),
        applied_amendment_ids: Vec::new(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };

    store
        .save_plan_binding(&attempt, &initial)
        .expect("initial binding");
    let mut repeated = initial.clone();
    repeated.updated_at = "2026-07-18T00:00:01Z".to_string();
    store
        .save_plan_binding(&attempt, &repeated)
        .expect("semantic repeat is idempotent");
    assert_eq!(store.get_plan_binding(&attempt).unwrap(), initial);

    let advanced = CodingAttemptPlanBinding {
        attempt_id: attempt.id.clone(),
        plan_id: PLAN_ID.to_string(),
        bound_plan_revision_id: "plan_revision_0003".to_string(),
        applied_amendment_ids: vec![
            "plan_amendment_0002".to_string(),
            "plan_amendment_0001".to_string(),
        ],
        updated_at: "2026-07-18T00:00:02Z".to_string(),
    };
    store
        .save_plan_binding(&attempt, &advanced)
        .expect("advance binding");
    assert_eq!(store.get_plan_binding(&attempt).unwrap(), advanced);

    for conflicting in [
        CodingAttemptPlanBinding {
            applied_amendment_ids: vec!["plan_amendment_0002".to_string()],
            ..advanced.clone()
        },
        CodingAttemptPlanBinding {
            applied_amendment_ids: vec![
                "plan_amendment_0001".to_string(),
                "plan_amendment_0002".to_string(),
            ],
            ..advanced.clone()
        },
        CodingAttemptPlanBinding {
            applied_amendment_ids: vec![
                "plan_amendment_0002".to_string(),
                "plan_amendment_0001".to_string(),
                "plan_amendment_0001".to_string(),
            ],
            ..advanced.clone()
        },
        CodingAttemptPlanBinding {
            bound_plan_revision_id: "plan_revision_0004".to_string(),
            ..advanced.clone()
        },
    ] {
        assert!(matches!(
            store.save_plan_binding(&attempt, &conflicting),
            Err(ProductStoreError::IdentityMismatch {
                kind: "coding_attempt_plan_binding",
                ..
            })
        ));
    }
}

#[test]
fn coding_attempt_plan_binding_rejects_forged_attempt_or_plan_lineage() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let binding = CodingAttemptPlanBinding {
        attempt_id: attempt.id.clone(),
        plan_id: PLAN_ID.to_string(),
        bound_plan_revision_id: "plan_revision_0001".to_string(),
        applied_amendment_ids: Vec::new(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };

    let mut forged_attempt = attempt.clone();
    forged_attempt.work_item_id = "wi_other".to_string();
    assert!(matches!(
        store.save_plan_binding(&forged_attempt, &binding),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_attempt",
            ..
        })
    ));

    let wrong_plan = CodingAttemptPlanBinding {
        plan_id: "issue_plan_other".to_string(),
        ..binding
    };
    assert!(matches!(
        store.save_plan_binding(&attempt, &wrong_plan),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_attempt_plan_binding",
            ..
        })
    ));
}

#[test]
fn coding_amendment_journal_create_get_and_phase_advance_are_monotonic() {
    let (tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let journal = CodingAmendmentApplicationJournal {
        id: "coding_amendment_application_0001".to_string(),
        attempt_id: attempt.id.clone(),
        amendment_id: "plan_amendment_0001".to_string(),
        phase: CodingAmendmentApplicationPhase::Started,
        error: None,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };

    store
        .create_amendment_application_journal(&attempt, &journal)
        .expect("create journal");
    assert_eq!(
        store
            .get_amendment_application_journal(&attempt, &journal.amendment_id)
            .unwrap(),
        journal
    );
    assert!(
        tmp.path()
            .join(".aria/projects/project_0001/issues/issue_0001/coding-attempts")
            .join(&attempt.id)
            .join("amendment-applications/plan_amendment_0001.json")
            .is_file()
    );

    let failed = store
        .advance_amendment_application_journal(
            &attempt,
            &journal.amendment_id,
            CodingAmendmentApplicationPhase::Started,
            Some("disk full".to_string()),
            "2026-07-18T00:00:01Z".to_string(),
        )
        .expect("record phase error");
    assert_eq!(failed.error.as_deref(), Some("disk full"));

    let advanced = store
        .advance_amendment_application_journal(
            &attempt,
            &journal.amendment_id,
            CodingAmendmentApplicationPhase::PlanBindingWritten,
            None,
            "2026-07-18T00:00:02Z".to_string(),
        )
        .expect("advance phase");
    assert_eq!(
        advanced.phase,
        CodingAmendmentApplicationPhase::PlanBindingWritten
    );
    assert!(advanced.error.is_none());
    assert_eq!(advanced.created_at, journal.created_at);

    assert!(matches!(
        store.advance_amendment_application_journal(
            &attempt,
            &journal.amendment_id,
            CodingAmendmentApplicationPhase::Started,
            None,
            "2026-07-18T00:00:03Z".to_string(),
        ),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_amendment_application_journal",
            ..
        })
    ));
}

#[test]
fn coding_unit_run_create_is_immutable_and_sorted_by_execution_number() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let unit = coding_unit_run_unit(
        &store,
        &attempt,
        "wi_core",
        "work_item_revision_0001",
        0,
        CodingExecutionUnitStatus::Running,
    );
    let second = coding_unit_run_record(
        "coding_unit_run_0002",
        &unit.id,
        2,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Pending,
    );
    let first = coding_unit_run_record(
        "coding_unit_run_0001",
        &unit.id,
        1,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Completed,
    );

    store.create_coding_unit_run(&attempt, &second).unwrap();
    store.create_coding_unit_run(&attempt, &first).unwrap();
    store
        .create_coding_unit_run(&attempt, &first)
        .expect("same id and value is idempotent");
    assert_eq!(
        store.list_coding_unit_runs(&attempt, &unit.id).unwrap(),
        vec![first.clone(), second.clone()]
    );

    let mut conflicting_id = first.clone();
    conflicting_id.canonical_contract_hash = "different".to_string();
    assert!(matches!(
        store.create_coding_unit_run(&attempt, &conflicting_id),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_unit_run",
            ..
        })
    ));
    let duplicate_execution = coding_unit_run_record(
        "coding_unit_run_0003",
        &unit.id,
        2,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Pending,
    );
    assert!(matches!(
        store.create_coding_unit_run(&attempt, &duplicate_execution),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_unit_run_execution_no",
            ..
        })
    ));
}

#[test]
fn coding_unit_run_logical_lookup_is_scoped_through_authoritative_unit_mapping() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let unit = coding_unit_run_unit(
        &store,
        &attempt,
        "wi_core",
        "work_item_revision_0001",
        0,
        CodingExecutionUnitStatus::Running,
    );
    let run = coding_unit_run_record(
        "coding_unit_run_0001",
        &unit.id,
        1,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Pending,
    );
    store.create_coding_unit_run(&attempt, &run).unwrap();

    assert_eq!(
        store
            .list_unit_runs_by_logical_id(&attempt, "wi_core")
            .unwrap(),
        vec![run]
    );

    let mut forged = attempt.clone();
    forged.issue_id = "issue_other".to_string();
    assert!(matches!(
        store.list_unit_runs_by_logical_id(&forged, "wi_core"),
        Err(ProductStoreError::NotFound {
            kind: "coding_attempt",
            ..
        }) | Err(ProductStoreError::IdentityMismatch {
            kind: "coding_attempt",
            ..
        })
    ));
}

#[test]
fn coding_unit_run_active_lookup_fails_closed_for_missing_or_ambiguous_runs() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let unit = coding_unit_run_unit(
        &store,
        &attempt,
        "wi_core",
        "work_item_revision_0001",
        0,
        CodingExecutionUnitStatus::Running,
    );

    assert!(matches!(
        store.get_active_unit_run(&attempt),
        Err(ProductStoreError::NotFound {
            kind: "coding_unit_run",
            ..
        })
    ));

    let first = coding_unit_run_record(
        "coding_unit_run_0001",
        &unit.id,
        1,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Pending,
    );
    store.create_coding_unit_run(&attempt, &first).unwrap();
    assert_eq!(store.get_active_unit_run(&attempt).unwrap(), first);

    let second = coding_unit_run_record(
        "coding_unit_run_0002",
        &unit.id,
        2,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Blocked,
    );
    store.create_coding_unit_run(&attempt, &second).unwrap();
    assert!(matches!(
        store.get_active_unit_run(&attempt),
        Err(ProductStoreError::Ambiguous {
            kind: "coding_unit_run",
            ..
        })
    ));
}

#[test]
fn coding_unit_run_context_binding_preserves_facts_status_and_independent_counters() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let unit = coding_unit_run_unit(
        &store,
        &attempt,
        "wi_core",
        "work_item_revision_0001",
        0,
        CodingExecutionUnitStatus::Running,
    );
    let run = coding_unit_run_record(
        "coding_unit_run_0001",
        &unit.id,
        1,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Completed,
    );
    store.create_coding_unit_run(&attempt, &run).unwrap();
    let rendered = RenderedExecutionContext {
        text: "real rendered context".to_string(),
        renderer_version: "codex-renderer-v2".to_string(),
        content_hash: "execution_context_hash".to_string(),
    };

    let bound = store
        .bind_unit_run_execution_context(&attempt, &run.id, CodingProviderRole::Coder, &rendered)
        .expect("bind coder context");

    assert_eq!(bound.status, CodingUnitRunStatus::Completed);
    assert_eq!(bound.work_item_revision_id, run.work_item_revision_id);
    assert_eq!(
        bound.resolved_handoff_revision_ids,
        run.resolved_handoff_revision_ids
    );
    assert_eq!(bound.canonical_contract_hash, run.canonical_contract_hash);
    assert_eq!(bound.projection_bundle_id, run.projection_bundle_id);
    assert_eq!(bound.coder_projection_hash, run.coder_projection_hash);
    assert_eq!(bound.reviewer_projection_hash, run.reviewer_projection_hash);
    assert_eq!(bound.unit_rework_count, 2);
    assert_eq!(bound.verification_retry_count, 3);
    assert_eq!(bound.operational_retry_count, 5);
    assert_eq!(bound.plan_repair_count, 7);
    assert_eq!(bound.start_commit, run.start_commit);
    assert_eq!(bound.completion_commit, run.completion_commit);
    assert_eq!(bound.coder_provider_renderer_version, "codex-renderer-v2");
    assert_eq!(
        bound.coder_execution_context_hash.as_deref(),
        Some("execution_context_hash")
    );
    assert!(matches!(
        store.bind_unit_run_execution_context(
            &attempt,
            &run.id,
            CodingProviderRole::Tester,
            &rendered,
        ),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_unit_run_execution_context_role",
            ..
        })
    ));
}

#[test]
fn coding_unit_run_concurrent_role_context_binding_does_not_lose_updates() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let unit = coding_unit_run_unit(
        &store,
        &attempt,
        "wi_core",
        "work_item_revision_0001",
        0,
        CodingExecutionUnitStatus::Running,
    );
    let run = coding_unit_run_record(
        "coding_unit_run_0001",
        &unit.id,
        1,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Running,
    );
    store.create_coding_unit_run(&attempt, &run).unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for (role, version, hash) in [
        (CodingProviderRole::Coder, "coder-v2", "coder-context"),
        (
            CodingProviderRole::CodeReviewer,
            "reviewer-v2",
            "reviewer-context",
        ),
    ] {
        let store = store.clone();
        let attempt = attempt.clone();
        let run_id = run.id.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            store
                .bind_unit_run_execution_context(
                    &attempt,
                    &run_id,
                    role,
                    &RenderedExecutionContext {
                        text: format!("{version} rendered context"),
                        renderer_version: version.to_string(),
                        content_hash: hash.to_string(),
                    },
                )
                .expect("bind role context");
        }));
    }
    barrier.wait();
    for handle in handles {
        handle.join().expect("binding thread");
    }

    let persisted = store.get_active_unit_run(&attempt).unwrap();
    assert_eq!(persisted.coder_provider_renderer_version, "coder-v2");
    assert_eq!(
        persisted.coder_execution_context_hash.as_deref(),
        Some("coder-context")
    );
    assert_eq!(persisted.reviewer_provider_renderer_version, "reviewer-v2");
    assert_eq!(
        persisted.reviewer_execution_context_hash.as_deref(),
        Some("reviewer-context")
    );
}

#[test]
fn coding_unit_run_concurrent_identical_create_is_idempotent() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let unit = coding_unit_run_unit(
        &store,
        &attempt,
        "wi_core",
        "work_item_revision_0001",
        0,
        CodingExecutionUnitStatus::Running,
    );
    let run = coding_unit_run_record(
        "coding_unit_run_0001",
        &unit.id,
        1,
        &unit.work_item_revision_id,
        CodingUnitRunStatus::Pending,
    );
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let store = store.clone();
        let attempt = attempt.clone();
        let run = run.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            store.create_coding_unit_run(&attempt, &run)
        }));
    }
    barrier.wait();

    for handle in handles {
        handle
            .join()
            .expect("create thread")
            .expect("idempotent create");
    }
    assert_eq!(
        store.list_coding_unit_runs(&attempt, &unit.id).unwrap(),
        vec![run]
    );
}

#[test]
fn coding_unit_run_concurrent_execution_number_conflict_persists_one_run() {
    let (_tmp, store) = setup_store();
    let attempt = coding_plan_repair_attempt(&store);
    let unit = coding_unit_run_unit(
        &store,
        &attempt,
        "wi_core",
        "work_item_revision_0001",
        0,
        CodingExecutionUnitStatus::Running,
    );
    let runs = [
        coding_unit_run_record(
            "coding_unit_run_0001",
            &unit.id,
            1,
            &unit.work_item_revision_id,
            CodingUnitRunStatus::Pending,
        ),
        coding_unit_run_record(
            "coding_unit_run_0002",
            &unit.id,
            1,
            &unit.work_item_revision_id,
            CodingUnitRunStatus::Pending,
        ),
    ];
    let barrier = Arc::new(Barrier::new(3));
    let handles = runs
        .iter()
        .cloned()
        .map(|run| {
            let store = store.clone();
            let attempt = attempt.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store.create_coding_unit_run(&attempt, &run)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("create thread"))
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_unit_run_execution_no",
                        ..
                    })
                )
            })
            .count(),
        1
    );
    let persisted = store.list_coding_unit_runs(&attempt, &unit.id).unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].execution_no, 1);
}
