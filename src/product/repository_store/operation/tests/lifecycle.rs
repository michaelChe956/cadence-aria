#[test]
fn operation_progress_callback_contract_preserves_step_event_order() {
    let progress = RecordingProgress::default();

    report_operation_progress(&progress).unwrap();

    assert_eq!(
        progress.events.lock().unwrap().as_slice(),
        &[
            (RepositoryInitializationStepKind::CadenceSkills, "started"),
            (RepositoryInitializationStepKind::CadenceSkills, "completed"),
            (RepositoryInitializationStepKind::PreCheck, "started"),
            (RepositoryInitializationStepKind::PreCheck, "completed"),
        ],
    );
}

#[test]
fn operation_starts_with_exactly_five_pending_steps_and_enforces_order() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = RepositoryInitializationOperationStore::new(paths);
    let operation = RepositoryInitializationOperation::new(
        "repository_initialization_0001".to_string(),
        "project_0001".to_string(),
        RepositoryInitializationOperationInput {
            name: "Aria".to_string(),
            git_root: temp.path().join("repo"),
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("claude_code".to_string()),
        },
        CREATED_AT.to_string(),
    );

    let created = store.create(operation).unwrap();
    assert_eq!(
        created.status,
        RepositoryInitializationOperationStatus::Created
    );
    assert_eq!(
        created
            .steps
            .iter()
            .map(|step| step.step_id)
            .collect::<Vec<_>>(),
        vec![
            RepositoryInitializationStepKind::CadenceSkills,
            RepositoryInitializationStepKind::PreCheck,
            RepositoryInitializationStepKind::RuleConfig,
            RepositoryInitializationStepKind::McpConfiguration,
            RepositoryInitializationStepKind::ProjectRulesExamples,
        ],
    );
    assert!(
        created
            .steps
            .iter()
            .all(|step| { step.status == RepositoryInitializationStepStatus::Pending })
    );

    store
        .mark_running(
            "project_0001",
            "repository_initialization_0001",
            RUNNING_AT.into(),
        )
        .unwrap();
    let error = store
        .mark_step_running(
            "project_0001",
            "repository_initialization_0001",
            RepositoryInitializationStepKind::RuleConfig,
            "2026-07-22T00:00:02Z".into(),
        )
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
}

#[test]
fn interrupted_operation_marks_running_step_failed_but_preserves_completed_steps() {
    let fixture = running_pre_check_operation();
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
    assert_eq!(
        recovered.failed_step,
        Some(RepositoryInitializationStepKind::PreCheck)
    );
    assert_eq!(
        recovered.steps[0].status,
        RepositoryInitializationStepStatus::Completed
    );
    assert_eq!(
        recovered.steps[1].status,
        RepositoryInitializationStepStatus::Failed
    );
    assert!(
        recovered.steps[2..]
            .iter()
            .all(|step| { step.status == RepositoryInitializationStepStatus::Pending })
    );
    assert_eq!(
        recovered.error.as_ref().unwrap().reason_code,
        "repository_initialization_interrupted",
    );
    assert!(recovered.error.as_ref().unwrap().retryable);
    assert_eq!(
        recovered.error.as_ref().unwrap().action,
        "服务在初始化完成前中断；检查可能的部分修改后重新提交",
    );
}

#[test]
fn completed_operation_requires_all_steps_and_persists_final_result() {
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

    assert_eq!(
        completed.status,
        RepositoryInitializationOperationStatus::Completed
    );
    assert!(completed.result.is_some());
    assert!(completed.error.is_none());
    assert_eq!(
        fixture
            .store
            .get("project_0001", &fixture.operation_id)
            .unwrap(),
        completed,
    );
    let persisted: RepositoryInitializationOperation =
        read_json(&fixture.operation_path()).unwrap();
    assert_eq!(persisted, completed);
}

#[test]
fn unknown_or_wrong_project_operation_is_not_readable() {
    let fixture = created_operation();
    assert!(matches!(
        fixture.store.get("project_9999", &fixture.operation_id),
        Err(ProductStoreError::NotFound {
            kind: "repository_initialization_operation",
            ..
        })
    ));
    assert!(matches!(
        fixture.store.get("project_0001", "../escape"),
        Err(ProductStoreError::PathEscape(_))
    ));
}

#[test]
fn operation_finish_failed_without_step_preserves_completed_steps() {
    let fixture = completed_steps_operation();
    let failed = fixture
        .store
        .finish_failed(
            "project_0001",
            &fixture.operation_id,
            None,
            repository_persist_failed_error(),
            COMPLETED_AT.into(),
        )
        .unwrap();

    assert_eq!(
        failed.status,
        RepositoryInitializationOperationStatus::Failed
    );
    assert_eq!(failed.failed_step, None);
    assert!(
        failed
            .steps
            .iter()
            .all(|step| step.status == RepositoryInitializationStepStatus::Completed)
    );
    assert!(failed.result.is_none());
    assert_eq!(
        failed.error.as_ref().unwrap().reason_code,
        "repository_persist_failed"
    );
    let persisted: RepositoryInitializationOperation =
        read_json(&fixture.operation_path()).unwrap();
    assert_eq!(persisted, failed);
}
