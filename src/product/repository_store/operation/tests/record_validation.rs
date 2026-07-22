#[test]
fn operation_get_rejects_partial_failed_record_without_interruption_error() {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();
    fixture
        .store
        .mark_step_running(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::CadenceSkills,
            "2026-07-22T00:00:02Z".into(),
        )
        .unwrap();
    fixture
        .store
        .mark_step_completed(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::CadenceSkills,
            "2026-07-22T00:00:03Z".into(),
        )
        .unwrap();

    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.status = RepositoryInitializationOperationStatus::Failed;
    persisted.failed_step = None;
    persisted.completed_at = Some(COMPLETED_AT.to_string());
    persisted.result = None;
    persisted.error = Some(repository_persist_failed_error());
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.get("project_0001", &fixture.operation_id),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_finish_completed_rejects_stale_terminal_fields() {
    for stale_field in ["failed_step", "result", "completed_at", "error"] {
        let fixture = completed_steps_operation();
        let path = fixture.operation_path();
        let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
        match stale_field {
            "failed_step" => {
                persisted.failed_step = Some(RepositoryInitializationStepKind::CadenceSkills)
            }
            "result" => persisted.result = Some(success_result("project_0001")),
            "completed_at" => persisted.completed_at = Some(COMPLETED_AT.to_string()),
            "error" => persisted.error = Some(repository_persist_failed_error()),
            _ => unreachable!(),
        }
        write_json(&path, &persisted).unwrap();

        assert!(matches!(
            fixture.store.finish_completed(
                "project_0001",
                &fixture.operation_id,
                success_result("project_0001"),
                COMPLETED_AT.into(),
            ),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }
}

#[test]
fn operation_get_rejects_created_record_with_terminal_fields() {
    for stale_field in ["failed_step", "result", "error", "completed_at"] {
        let fixture = created_operation();
        let path = fixture.operation_path();
        let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
        match stale_field {
            "failed_step" => {
                persisted.failed_step = Some(RepositoryInitializationStepKind::CadenceSkills)
            }
            "result" => persisted.result = Some(success_result("project_0001")),
            "error" => persisted.error = Some(repository_persist_failed_error()),
            "completed_at" => persisted.completed_at = Some(COMPLETED_AT.to_string()),
            _ => unreachable!(),
        }
        write_json(&path, &persisted).unwrap();

        assert!(matches!(
            fixture.store.get("project_0001", &fixture.operation_id),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }
}

#[test]
fn operation_get_rejects_created_record_with_step_timestamp() {
    let fixture = created_operation();
    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.steps[0].started_at = Some(RUNNING_AT.to_string());
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.get("project_0001", &fixture.operation_id),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_get_rejects_running_record_with_terminal_fields() {
    for stale_field in ["failed_step", "result", "error", "completed_at"] {
        let fixture = created_operation();
        fixture
            .store
            .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
            .unwrap();
        let path = fixture.operation_path();
        let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
        match stale_field {
            "failed_step" => {
                persisted.failed_step = Some(RepositoryInitializationStepKind::CadenceSkills)
            }
            "result" => persisted.result = Some(success_result("project_0001")),
            "error" => persisted.error = Some(repository_persist_failed_error()),
            "completed_at" => persisted.completed_at = Some(COMPLETED_AT.to_string()),
            _ => unreachable!(),
        }
        write_json(&path, &persisted).unwrap();

        assert!(matches!(
            fixture.store.get("project_0001", &fixture.operation_id),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }
}

#[test]
fn operation_get_rejects_running_record_with_failed_step() {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();
    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.steps[0].status = RepositoryInitializationStepStatus::Failed;
    persisted.steps[0].completed_at = Some(COMPLETED_AT.to_string());
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.get("project_0001", &fixture.operation_id),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_get_rejects_running_record_with_multiple_current_steps() {
    let fixture = running_pre_check_operation();
    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.steps[2].status = RepositoryInitializationStepStatus::Running;
    persisted.steps[2].started_at = Some("2026-07-22T00:00:05Z".to_string());
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.get("project_0001", &fixture.operation_id),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_get_rejects_running_record_with_invalid_step_timestamps() {
    for malformed in [
        "running_without_started_at",
        "completed_without_completed_at",
    ] {
        let fixture = created_operation();
        fixture
            .store
            .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
            .unwrap();
        fixture
            .store
            .mark_step_running(
                "project_0001",
                &fixture.operation_id,
                RepositoryInitializationStepKind::CadenceSkills,
                "2026-07-22T00:00:02Z".into(),
            )
            .unwrap();
        let path = fixture.operation_path();
        let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
        match malformed {
            "running_without_started_at" => persisted.steps[0].started_at = None,
            "completed_without_completed_at" => {
                persisted.steps[0].status = RepositoryInitializationStepStatus::Completed;
            }
            _ => unreachable!(),
        }
        write_json(&path, &persisted).unwrap();

        assert!(matches!(
            fixture.store.get("project_0001", &fixture.operation_id),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }
}

#[test]
fn operation_get_rejects_completed_record_without_result() {
    let fixture = completed_steps_operation();
    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.status = RepositoryInitializationOperationStatus::Completed;
    persisted.completed_at = Some(COMPLETED_AT.to_string());
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.get("project_0001", &fixture.operation_id),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_get_rejects_completed_record_with_non_completed_step() {
    let fixture = completed_steps_operation();
    let completed = fixture
        .store
        .finish_completed(
            "project_0001",
            &fixture.operation_id,
            success_result("project_0001"),
            COMPLETED_AT.into(),
        )
        .unwrap();
    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.steps[4].status = RepositoryInitializationStepStatus::Pending;
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.get("project_0001", &fixture.operation_id),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
    assert_eq!(
        completed.status,
        RepositoryInitializationOperationStatus::Completed
    );
}

#[test]
fn operation_get_rejects_completed_record_with_inconsistent_terminal_fields() {
    for stale_field in ["failed_step", "error", "completed_at"] {
        let fixture = completed_steps_operation();
        let completed = fixture
            .store
            .finish_completed(
                "project_0001",
                &fixture.operation_id,
                success_result("project_0001"),
                COMPLETED_AT.into(),
            )
            .unwrap();
        let path = fixture.operation_path();
        let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
        match stale_field {
            "failed_step" => {
                persisted.failed_step = Some(RepositoryInitializationStepKind::CadenceSkills)
            }
            "error" => persisted.error = Some(repository_persist_failed_error()),
            "completed_at" => persisted.completed_at = None,
            _ => unreachable!(),
        }
        write_json(&path, &persisted).unwrap();

        assert!(matches!(
            fixture.store.get("project_0001", &fixture.operation_id),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
        assert_eq!(
            completed.status,
            RepositoryInitializationOperationStatus::Completed
        );
    }
}

#[test]
fn operation_get_rejects_failed_record_with_inconsistent_failed_step() {
    let fixture = failed_pre_check_operation();
    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.failed_step = None;
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.get("project_0001", &fixture.operation_id),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_get_rejects_failed_record_with_inconsistent_terminal_fields() {
    for stale_field in [
        "result",
        "error",
        "completed_at",
        "failed_without_completed_at",
    ] {
        let fixture = failed_pre_check_operation();
        let path = fixture.operation_path();
        let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
        match stale_field {
            "result" => persisted.result = Some(success_result("project_0001")),
            "error" => persisted.error = None,
            "completed_at" => persisted.completed_at = None,
            "failed_without_completed_at" => persisted.steps[1].completed_at = None,
            _ => unreachable!(),
        }
        write_json(&path, &persisted).unwrap();

        assert!(matches!(
            fixture.store.get("project_0001", &fixture.operation_id),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }
}

#[test]
fn operation_recovery_rejects_multiple_current_steps() {
    let fixture = running_pre_check_operation();
    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.steps[2].status = RepositoryInitializationStepStatus::Running;
    persisted.steps[2].started_at = Some("2026-07-22T00:00:05Z".to_string());
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.recover_interrupted(
            "project_0001",
            &fixture.operation_id,
            "2026-07-22T00:01:00Z".into(),
        ),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_recovery_allows_running_operation_without_current_step() {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();

    let recovered = fixture
        .store
        .recover_interrupted(
            "project_0001",
            &fixture.operation_id,
            "2026-07-22T00:01:00Z".into(),
        )
        .unwrap();

    assert_eq!(
        recovered.status,
        RepositoryInitializationOperationStatus::Failed
    );
    assert_eq!(recovered.failed_step, None);
    assert!(
        recovered
            .steps
            .iter()
            .all(|step| step.status == RepositoryInitializationStepStatus::Pending)
    );
    assert_eq!(
        fixture
            .store
            .get("project_0001", &fixture.operation_id)
            .unwrap(),
        recovered
    );
}

#[test]
fn operation_recovery_allows_running_operation_with_completed_prefix_without_current_step() {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();
    fixture
        .store
        .mark_step_running(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::CadenceSkills,
            "2026-07-22T00:00:02Z".into(),
        )
        .unwrap();
    fixture
        .store
        .mark_step_completed(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::CadenceSkills,
            "2026-07-22T00:00:03Z".into(),
        )
        .unwrap();

    let recovered = fixture
        .store
        .recover_interrupted(
            "project_0001",
            &fixture.operation_id,
            "2026-07-22T00:01:00Z".into(),
        )
        .unwrap();

    assert_eq!(
        recovered.status,
        RepositoryInitializationOperationStatus::Failed
    );
    assert_eq!(recovered.failed_step, None);
    assert_eq!(
        recovered.steps[0].status,
        RepositoryInitializationStepStatus::Completed
    );
    assert!(
        recovered.steps[1..]
            .iter()
            .all(|step| step.status == RepositoryInitializationStepStatus::Pending)
    );
    assert_eq!(
        fixture
            .store
            .get("project_0001", &fixture.operation_id)
            .unwrap(),
        recovered
    );
}

#[test]
fn operation_recovery_allows_all_completed_running_operation_without_current_step() {
    let fixture = completed_steps_operation();
    let recovered = fixture
        .store
        .recover_interrupted(
            "project_0001",
            &fixture.operation_id,
            "2026-07-22T00:01:00Z".into(),
        )
        .unwrap();

    assert_eq!(
        recovered.status,
        RepositoryInitializationOperationStatus::Failed
    );
    assert_eq!(recovered.failed_step, None);
    assert!(
        recovered
            .steps
            .iter()
            .all(|step| step.status == RepositoryInitializationStepStatus::Completed)
    );
    assert_eq!(
        fixture
            .store
            .get("project_0001", &fixture.operation_id)
            .unwrap(),
        recovered
    );
}

#[test]
fn operation_rejects_running_a_later_step_before_prior_steps() {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();

    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.steps[2].status = RepositoryInitializationStepStatus::Running;
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.mark_step_running(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::RuleConfig,
            "2026-07-22T00:00:03Z".into(),
        ),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_rejects_completing_a_later_running_step_before_prior_steps() {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();

    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.steps[2].status = RepositoryInitializationStepStatus::Running;
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.mark_step_completed(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::RuleConfig,
            "2026-07-22T00:00:03Z".into(),
        ),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_rejects_malformed_persisted_step_shape() {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();

    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.steps = Vec::new();
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.finish_completed(
            "project_0001",
            &fixture.operation_id,
            success_result("project_0001"),
            COMPLETED_AT.into(),
        ),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_rejects_reordered_or_duplicate_persisted_steps() {
    for malformed_steps in [
        vec![
            RepositoryInitializationStepKind::PreCheck,
            RepositoryInitializationStepKind::CadenceSkills,
            RepositoryInitializationStepKind::RuleConfig,
            RepositoryInitializationStepKind::McpConfiguration,
            RepositoryInitializationStepKind::ProjectRulesExamples,
        ],
        vec![
            RepositoryInitializationStepKind::CadenceSkills,
            RepositoryInitializationStepKind::CadenceSkills,
            RepositoryInitializationStepKind::RuleConfig,
            RepositoryInitializationStepKind::McpConfiguration,
            RepositoryInitializationStepKind::ProjectRulesExamples,
        ],
    ] {
        let fixture = created_operation();
        let path = fixture.operation_path();
        let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
        for (step, step_id) in persisted.steps.iter_mut().zip(malformed_steps) {
            step.step_id = step_id;
        }
        write_json(&path, &persisted).unwrap();

        assert!(matches!(
            fixture.store.get("project_0001", &fixture.operation_id),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }
}

#[test]
fn operation_rejects_failing_a_later_step_before_prior_steps() {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();

    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    persisted.steps[2].status = RepositoryInitializationStepStatus::Running;
    write_json(&path, &persisted).unwrap();

    assert!(matches!(
        fixture.store.finish_failed(
            "project_0001",
            &fixture.operation_id,
            Some(RepositoryInitializationStepKind::RuleConfig),
            repository_persist_failed_error(),
            "2026-07-22T00:00:03Z".into(),
        ),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_create_rejects_duplicate_id_with_different_input() {
    let fixture = created_operation();
    let duplicate = RepositoryInitializationOperation::new(
        fixture.operation_id.clone(),
        "project_0001".to_string(),
        RepositoryInitializationOperationInput {
            name: "Different name".to_string(),
            git_root: fixture._temp.path().join("other-repo"),
            default_policy_preset: None,
            default_provider_mode: None,
        },
        CREATED_AT.to_string(),
    );

    assert!(matches!(
        fixture.store.create(duplicate),
        Err(ProductStoreError::IdentityMismatch { .. })
    ));
}

#[test]
fn operation_completion_rejects_pending_steps() {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();

    let error = fixture
        .store
        .finish_completed(
            "project_0001",
            &fixture.operation_id,
            success_result("project_0001"),
            COMPLETED_AT.into(),
        )
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
}

#[test]
fn operation_recovery_marks_created_operation_failed_without_failed_step() {
    let fixture = created_operation();
    let recovered = fixture
        .store
        .recover_interrupted(
            "project_0001",
            &fixture.operation_id,
            "2026-07-22T00:01:00Z".into(),
        )
        .unwrap();

    assert_eq!(
        recovered.status,
        RepositoryInitializationOperationStatus::Failed
    );
    assert_eq!(recovered.failed_step, None);
    assert!(
        recovered
            .steps
            .iter()
            .all(|step| step.status == RepositoryInitializationStepStatus::Pending)
    );
}

#[test]
fn operation_recovery_preserves_terminal_record() {
    let fixture = completed_steps_operation();
    let completed = fixture
        .store
        .finish_completed(
            "project_0001",
            &fixture.operation_id,
            success_result("project_0001"),
            COMPLETED_AT.into(),
        )
        .unwrap();

    let recovered = fixture
        .store
        .recover_interrupted(
            "project_0001",
            &fixture.operation_id,
            "2026-07-22T00:03:00Z".into(),
        )
        .unwrap();
    assert_eq!(recovered, completed);
}
