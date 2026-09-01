use super::*;

fn group_input(plan_id: &str, current_work_item_id: &str) -> CreateGroupCodingAttemptInput {
    CreateGroupCodingAttemptInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        current_work_item_id: current_work_item_id.to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria/issues/{ISSUE_ID}"),
        worktree_path: None,
        provider_config_snapshot: provider_snapshot(),
        target_snapshot: None,
        max_auto_rework: 2,
    }
}

fn binding(logical_work_item_id: &str, dependencies: &[&str]) -> AuthoritativeCodingUnitBinding {
    AuthoritativeCodingUnitBinding {
        logical_work_item_id: logical_work_item_id.to_string(),
        work_item_revision_id: format!("{logical_work_item_id}_revision_0001"),
        verification_plan_revision_id: format!("{logical_work_item_id}_verification_0001"),
        projection_bundle_id: format!("{logical_work_item_id}_projection_0001"),
        target_repository_id: None,
        source_draft_error: None,
        dependency_logical_work_item_ids: dependencies.iter().map(|id| (*id).to_string()).collect(),
    }
}

#[test]
fn advance_attempt_persists_sc_advance_admission_kind_and_immutable_plan_binding() {
    let (_tmp, store) = setup_store();
    let input = group_input("plan_0001", WORK_ITEM_ID);
    let units = vec![binding(WORK_ITEM_ID, &[])];

    let attempt = store
        .ensure_group_attempt_for_advance(&input, "plan_revision_0001", "command_0001", &units)
        .expect("SC advance attempt");
    assert_eq!(attempt.admission_kind, CodingAdmissionKind::ScAdvance);

    let journal = store
        .get_group_initialization(PROJECT_ID, ISSUE_ID, "plan_0001")
        .expect("group journal");
    assert_eq!(journal.attempt.id, attempt.id);
    assert_eq!(
        journal.plan_binding.bound_plan_revision_id,
        "plan_revision_0001"
    );
    store
        .save_plan_binding(&attempt, &journal.plan_binding)
        .expect("immutable plan binding");

    let persisted = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.admission_kind, CodingAdmissionKind::ScAdvance);
    assert_eq!(
        store
            .get_plan_binding(&persisted)
            .unwrap()
            .bound_plan_revision_id,
        "plan_revision_0001"
    );

    for status in [
        CodingAttemptStatus::Created,
        CodingAttemptStatus::Running,
        CodingAttemptStatus::AwaitingPlanAmendment,
        CodingAttemptStatus::Completed,
        CodingAttemptStatus::Failed,
        CodingAttemptStatus::Aborted,
    ] {
        let mut fixture = persisted.clone();
        fixture.status = status;
        store
            .write_coding_attempt_for_test(&fixture)
            .expect("persist status matrix fixture");
        let replay = store
            .ensure_group_attempt_for_advance(
                &input,
                "plan_revision_0001",
                "command_matrix",
                &units,
            )
            .expect("status matrix replay");
        assert_eq!(replay.id, persisted.id);
    }
    assert_eq!(
        store
            .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn advance_attempt_repeated_plan_returns_same_attempt() {
    let (_tmp, store) = setup_store();
    let input = group_input("plan_0002", WORK_ITEM_ID);
    let units = vec![binding(WORK_ITEM_ID, &[])];
    let first = store
        .ensure_group_attempt_for_advance(&input, "plan_revision_0001", "command_0001", &units)
        .expect("first SC advance attempt");
    let journal = store
        .get_group_initialization(PROJECT_ID, ISSUE_ID, "plan_0002")
        .expect("group journal");
    store
        .save_plan_binding(&first, &journal.plan_binding)
        .expect("save first plan binding");

    let replay = store
        .ensure_group_attempt_for_advance(&input, "plan_revision_0001", "command_0002", &units)
        .expect("repeated plan must replay");
    assert_eq!(replay.id, first.id);
    assert_eq!(
        store
            .list_attempts_for_work_item(PROJECT_ID, ISSUE_ID, WORK_ITEM_ID)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn advance_attempt_units_follow_dependency_topology_with_stable_tie_break() {
    let (_tmp, store) = setup_store();
    let input = group_input("plan_0003", "work_item_a");
    let units = vec![
        binding("work_item_a", &[]),
        binding("work_item_c", &["work_item_a", "work_item_b"]),
        binding("work_item_b", &[]),
    ];

    let attempt = store
        .ensure_group_attempt_for_advance(&input, "plan_revision_0001", "command_0001", &units)
        .expect("topologically ordered SC advance attempt");
    let journal = store
        .get_group_initialization(PROJECT_ID, ISSUE_ID, "plan_0003")
        .expect("group journal");
    assert_eq!(
        journal
            .units
            .iter()
            .map(|unit| unit.logical_work_item_id.as_str())
            .collect::<Vec<_>>(),
        ["work_item_a", "work_item_b", "work_item_c"]
    );

    for (index, unit) in journal.units.iter().enumerate() {
        assert_eq!(unit.order_index, index as u32);
        assert_eq!(unit.attempt_id, attempt.id);
    }
    assert_eq!(
        journal.units[2].dependency_logical_work_item_ids,
        ["work_item_a", "work_item_b"]
    );
}

#[test]
fn legacy_group_initialization_defaults_admission_and_keeps_input_order_index() {
    let (_tmp, store) = setup_store();
    let input = group_input("plan_0004", "work_item_b");
    let units = vec![binding("work_item_b", &[]), binding("work_item_a", &[])];

    let journal = store
        .prepare_group_initialization(&input, "plan_revision_0001", &units)
        .expect("legacy group journal");
    assert_eq!(
        journal.attempt.admission_kind,
        CodingAdmissionKind::LegacyGroup
    );
    assert_eq!(
        journal
            .units
            .iter()
            .map(|unit| unit.logical_work_item_id.as_str())
            .collect::<Vec<_>>(),
        ["work_item_b", "work_item_a"]
    );
    assert_eq!(journal.units[0].order_index, 0);
    assert_eq!(journal.units[1].order_index, 1);
}
