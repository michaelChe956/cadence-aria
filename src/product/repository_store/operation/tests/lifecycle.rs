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
fn operation_starts_with_exactly_six_pending_steps_and_enforces_order() {
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
            RepositoryInitializationStepKind::GitFinalize,
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
fn completed_operation_keeps_repository_result_when_git_finalize_failed() {
    let fixture = running_git_finalize_operation();
    let mut result = success_result("project_0001");
    result.git_finalize_warning = Some(
        "git_finalize_push: permission denied；请手动提交推送".to_string(),
    );

    let completed = fixture
        .store
        .finish_completed(
            "project_0001",
            &fixture.operation_id,
            result,
            COMPLETED_AT.into(),
        )
        .unwrap();

    assert_eq!(
        completed.status,
        RepositoryInitializationOperationStatus::Completed
    );
    assert_eq!(
        completed.failed_step,
        Some(RepositoryInitializationStepKind::GitFinalize)
    );
    assert_eq!(
        completed.steps[5].status,
        RepositoryInitializationStepStatus::Failed
    );
    assert!(completed.error.is_none());
    assert!(completed
        .result
        .as_ref()
        .and_then(|result| result.git_finalize_warning.as_ref())
        .is_some());
}

#[test]
fn interrupted_git_finalize_keeps_registered_repository_result() {
    let fixture = running_git_finalize_operation();
    fixture
        .store
        .checkpoint_git_finalize_result(
            "project_0001",
            &fixture.operation_id,
            success_result("project_0001"),
        )
        .expect("persist registered repository before git finalize");

    let recovered = fixture
        .store
        .recover_interrupted("project_0001", &fixture.operation_id, COMPLETED_AT.into())
        .expect("git finalize interruption recovers as completed registration");
    assert_eq!(
        recovered.status,
        RepositoryInitializationOperationStatus::Completed
    );
    assert_eq!(
        recovered.failed_step,
        Some(RepositoryInitializationStepKind::GitFinalize)
    );
    assert_eq!(
        recovered.steps[5].status,
        RepositoryInitializationStepStatus::Failed
    );
    assert!(recovered.error.is_none());
    assert!(recovered
        .result
        .as_ref()
        .and_then(|result| result.git_finalize_warning.as_ref())
        .expect("manual recovery warning")
        .contains("手动执行 git commit / git push"));
}

#[test]
fn interrupted_before_git_finalize_completion_preserves_checkpoint_warning() {
    let fixture = running_git_finalize_operation();
    fixture
        .store
        .checkpoint_git_finalize_result(
            "project_0001",
            &fixture.operation_id,
            success_result("project_0001"),
        )
        .expect("persist registered repository before git finalize");
    let warning = "git_finalize: 无 upstream，已跳过 push，请手动推送".to_string();
    fixture
        .store
        .update_git_finalize_checkpoint_warning(
            "project_0001",
            &fixture.operation_id,
            warning.clone(),
        )
        .expect("persist skip-push warning while git finalize remains running");

    let recovered = fixture
        .store
        .recover_interrupted("project_0001", &fixture.operation_id, COMPLETED_AT.into())
        .expect("running git finalize recovers as completed registration");

    assert_eq!(
        recovered.status,
        RepositoryInitializationOperationStatus::Completed
    );
    assert_eq!(
        recovered.failed_step,
        Some(RepositoryInitializationStepKind::GitFinalize)
    );
    assert_eq!(
        recovered.steps[5].status,
        RepositoryInitializationStepStatus::Failed
    );
    assert_eq!(
        recovered
            .result
            .as_ref()
        .and_then(|result| result.git_finalize_warning.as_ref()),
        Some(&warning)
    );
    assert!(recovered.error.is_none());
}

#[test]
fn interrupted_after_git_finalize_completion_keeps_completed_result() {
    let fixture = running_git_finalize_operation();
    fixture
        .store
        .checkpoint_git_finalize_result(
            "project_0001",
            &fixture.operation_id,
            success_result("project_0001"),
        )
        .expect("persist registered repository before git finalize");
    fixture
        .store
        .mark_step_completed(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::GitFinalize,
            COMPLETED_AT.into(),
        )
        .expect("complete git finalize before terminal write");

    let recovered = fixture
        .store
        .recover_interrupted("project_0001", &fixture.operation_id, COMPLETED_AT.into())
        .expect("completed git finalize recovers as completed registration");
    assert_eq!(
        recovered.status,
        RepositoryInitializationOperationStatus::Completed
    );
    assert_eq!(recovered.failed_step, None);
    assert_eq!(
        recovered.steps[5].status,
        RepositoryInitializationStepStatus::Completed
    );
    assert!(recovered.error.is_none());
    assert!(recovered
        .result
        .as_ref()
        .and_then(|result| result.git_finalize_warning.as_ref())
        .is_none());
}

#[test]
fn interrupted_after_skip_push_completion_keeps_persisted_warning() {
    let fixture = running_git_finalize_operation();
    fixture
        .store
        .checkpoint_git_finalize_result(
            "project_0001",
            &fixture.operation_id,
            success_result("project_0001"),
        )
        .expect("persist registered repository before git finalize");
    let warning = "git_finalize: 无 remote，已跳过 push，请手动推送".to_string();
    fixture
        .store
        .update_git_finalize_checkpoint_warning(
            "project_0001",
            &fixture.operation_id,
            warning.clone(),
        )
        .expect("persist skip-push warning before completing git finalize");
    fixture
        .store
        .mark_step_completed(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::GitFinalize,
            COMPLETED_AT.into(),
        )
        .expect("complete git finalize before terminal write");

    let recovered = fixture
        .store
        .recover_interrupted("project_0001", &fixture.operation_id, COMPLETED_AT.into())
        .expect("completed skip-push finalize recovers as completed registration");

    assert_eq!(
        recovered.status,
        RepositoryInitializationOperationStatus::Completed
    );
    assert_eq!(recovered.failed_step, None);
    assert_eq!(
        recovered.steps[5].status,
        RepositoryInitializationStepStatus::Completed
    );
    assert_eq!(
        recovered
            .result
            .as_ref()
            .and_then(|result| result.git_finalize_warning.as_ref()),
        Some(&warning)
    );
}

#[test]
fn git_finalize_cannot_complete_without_persisted_repository_result() {
    let fixture = running_git_finalize_operation();

    let error = fixture
        .store
        .mark_step_completed(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::GitFinalize,
            COMPLETED_AT.into(),
        )
        .unwrap_err();

    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
}

#[test]
fn completed_legacy_five_step_operation_remains_readable() {
    let fixture = completed_steps_operation();
    fixture
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
    let removed_step = persisted.steps.pop().expect("git finalize step");
    assert_eq!(removed_step.step_id, RepositoryInitializationStepKind::GitFinalize);
    write_json(&path, &persisted).unwrap();

    let legacy = fixture
        .store
        .get("project_0001", &fixture.operation_id)
        .expect("legacy operation remains readable");
    assert_eq!(legacy.steps.len(), 5);
    assert_eq!(legacy.failed_step, None);
    assert!(legacy.result.is_some());
}

#[test]
fn interrupted_legacy_five_step_operation_remains_recoverable() {
    let fixture = running_pre_check_operation();
    let path = fixture.operation_path();
    let mut persisted: RepositoryInitializationOperation = read_json(&path).unwrap();
    let removed_step = persisted.steps.pop().expect("git finalize step");
    assert_eq!(removed_step.step_id, RepositoryInitializationStepKind::GitFinalize);
    write_json(&path, &persisted).unwrap();

    let recovered = fixture
        .store
        .recover_interrupted("project_0001", &fixture.operation_id, COMPLETED_AT.into())
        .expect("legacy operation remains recoverable");
    assert_eq!(recovered.status, RepositoryInitializationOperationStatus::Failed);
    assert_eq!(
        recovered.failed_step,
        Some(RepositoryInitializationStepKind::PreCheck)
    );
    assert_eq!(recovered.steps.len(), 5);
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

#[test]
fn operation_list_scopes_project_sorts_stably_and_rejects_corrupt_records() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = RepositoryInitializationOperationStore::new(paths.clone());

    let input = || RepositoryInitializationOperationInput {
        name: "Aria".to_string(),
        git_root: PathBuf::from("/tmp/aria"),
        default_policy_preset: None,
        default_provider_mode: None,
    };

    // 文件写入顺序与期望排序不一致：晚创建的先落盘，排序不依赖文件系统顺序。
    store
        .create(RepositoryInitializationOperation::new(
            "repository_initialization_b".to_string(),
            "project_0001".to_string(),
            input(),
            "2026-07-22T01:00:00Z".to_string(),
        ))
        .unwrap();
    store
        .create(RepositoryInitializationOperation::new(
            "repository_initialization_zeta".to_string(),
            "project_0001".to_string(),
            input(),
            "2026-07-22T00:00:00Z".to_string(),
        ))
        .unwrap();
    store
        .create(RepositoryInitializationOperation::new(
            "repository_initialization_a".to_string(),
            "project_0001".to_string(),
            input(),
            "2026-07-22T00:00:00Z".to_string(),
        ))
        .unwrap();
    store
        .create(RepositoryInitializationOperation::new(
            "repository_initialization_other".to_string(),
            "project_0002".to_string(),
            input(),
            "2026-07-22T00:00:00Z".to_string(),
        ))
        .unwrap();

    let listed = store.list("project_0001").unwrap();
    let ids: Vec<&str> = listed.iter().map(|op| op.operation_id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "repository_initialization_a",
            "repository_initialization_zeta",
            "repository_initialization_b",
        ],
        "sorted by (created_at, operation_id): {listed:?}"
    );

    // 无目录 project → 空列表（不报错）。
    assert!(store.list("project_9999").unwrap().is_empty());

    // 损坏记录：显式可诊断错误，不静默丢弃。
    let corrupt = paths
        .repository_initializations_root("project_0001")
        .join("repository_initialization_corrupt.json");
    std::fs::write(&corrupt, "{ not json").unwrap();
    let error = store.list("project_0001").unwrap_err();
    assert!(
        matches!(error, ProductStoreError::Json(_)),
        "corrupt record must surface diagnostics: {error:?}"
    );

    // 越权记录（record.project_id 与目录不符）→ identity mismatch。
    std::fs::remove_file(&corrupt).unwrap();
    let cross_source = paths
        .repository_initializations_root("project_0002")
        .join("repository_initialization_other.json");
    let cross_dest = paths
        .repository_initializations_root("project_0001")
        .join("repository_initialization_other.json");
    std::fs::copy(&cross_source, &cross_dest).unwrap();
    let error = store.list("project_0001").unwrap_err();
    assert!(
        matches!(error, ProductStoreError::IdentityMismatch { .. }),
        "cross-project record must be rejected: {error:?}"
    );
}
