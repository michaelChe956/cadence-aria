use std::path::PathBuf;
use std::sync::Mutex;

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::models::RepositoryRecord;
use crate::product::repository_store::{
    CadenceSkillsPreparationSummary, RepositoryInitializationCommandSummary,
    RepositoryInitializationOperation, RepositoryInitializationOperationInput,
    RepositoryInitializationOperationStatus, RepositoryInitializationProgress,
    RepositoryInitializationStepKind, RepositoryInitializationStepStatus,
    RepositoryInitializationSummary, RepositoryRegistrationError, RepositoryRegistrationSuccess,
};

const CREATED_AT: &str = "2026-07-22T00:00:00Z";
const RUNNING_AT: &str = "2026-07-22T00:00:01Z";
const COMPLETED_AT: &str = "2026-07-22T00:02:00Z";

struct OperationFixture {
    _temp: tempfile::TempDir,
    paths: ProductAppPaths,
    store: RepositoryInitializationOperationStore,
    operation_id: String,
}

#[derive(Default)]
struct RecordingProgress {
    events: Mutex<Vec<(RepositoryInitializationStepKind, &'static str)>>,
}

impl RepositoryInitializationProgress for RecordingProgress {
    fn step_started(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), RepositoryRegistrationError> {
        self.events.lock().unwrap().push((step, "started"));
        Ok(())
    }

    fn step_completed(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), RepositoryRegistrationError> {
        self.events.lock().unwrap().push((step, "completed"));
        Ok(())
    }
}

fn report_operation_progress(
    progress: &dyn RepositoryInitializationProgress,
) -> Result<(), RepositoryRegistrationError> {
    progress.step_started(RepositoryInitializationStepKind::CadenceSkills)?;
    progress.step_completed(RepositoryInitializationStepKind::CadenceSkills)?;
    progress.step_started(RepositoryInitializationStepKind::PreCheck)?;
    progress.step_completed(RepositoryInitializationStepKind::PreCheck)
}

impl OperationFixture {
    fn operation_path(&self) -> PathBuf {
        self.paths
            .repository_initializations_root("project_0001")
            .join(format!("{}.json", self.operation_id))
    }
}

fn success_result(project_id: &str) -> RepositoryRegistrationSuccess {
    RepositoryRegistrationSuccess {
        repository: RepositoryRecord {
            id: "repository_0001".to_string(),
            project_id: project_id.to_string(),
            name: "Aria".to_string(),
            path: PathBuf::from("/tmp/aria"),
            repo_hash: "repo_hash".to_string(),
            runtime_root: PathBuf::from("/tmp/aria/.aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "claude_code".to_string(),
            created_at: COMPLETED_AT.to_string(),
            updated_at: COMPLETED_AT.to_string(),
        },
        cadence_skills: CadenceSkillsPreparationSummary {
            source_mode: "cached".to_string(),
            source_root: PathBuf::from("/tmp/cadence-skills"),
            skills_root: PathBuf::from("/tmp/aria/.claude/skills"),
            git_updated: false,
            link_sync_status: "synchronized".to_string(),
            warnings: Vec::new(),
        },
        initialization: RepositoryInitializationSummary {
            provider: "claude_code".to_string(),
            source: PathBuf::from("/tmp/cadence-skills"),
            source_mode: "cached".to_string(),
            skills_root: PathBuf::from("/tmp/aria/.claude/skills"),
            git_updated: false,
            link_sync_status: "synchronized".to_string(),
            commands: vec![RepositoryInitializationCommandSummary {
                command_index: 1,
                command: "/pre-check --no-interrupt".to_string(),
                status: "completed".to_string(),
                output_summary: Some("ok".to_string()),
            }],
        },
        warnings: Vec::new(),
        changed_paths: Vec::new(),
        completed_at: COMPLETED_AT.to_string(),
    }
}

fn repository_persist_failed_error() -> RepositoryRegistrationError {
    RepositoryRegistrationError {
        stage: "repository_persist".to_string(),
        provider: None,
        command_index: None,
        command: None,
        reason_code: "repository_persist_failed".to_string(),
        stderr_summary: Some("persist failed".to_string()),
        changed_paths: Some(vec![".aria".to_string()]),
        retryable: true,
        action: "Inspect the repository and retry persistence.".to_string(),
    }
}

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

fn created_operation() -> OperationFixture {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let store = RepositoryInitializationOperationStore::new(paths.clone());
    let operation_id = "repository_initialization_0001".to_string();
    let operation = RepositoryInitializationOperation::new(
        operation_id.clone(),
        "project_0001".to_string(),
        RepositoryInitializationOperationInput {
            name: "Aria".to_string(),
            git_root: temp.path().join("repo"),
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("claude_code".to_string()),
        },
        CREATED_AT.to_string(),
    );
    store.create(operation).unwrap();

    OperationFixture {
        _temp: temp,
        paths,
        store,
        operation_id,
    }
}

fn running_pre_check_operation() -> OperationFixture {
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
    fixture
        .store
        .mark_step_running(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::PreCheck,
            "2026-07-22T00:00:04Z".into(),
        )
        .unwrap();
    fixture
}

fn failed_pre_check_operation() -> OperationFixture {
    let fixture = running_pre_check_operation();
    fixture
        .store
        .finish_failed(
            "project_0001",
            &fixture.operation_id,
            Some(RepositoryInitializationStepKind::PreCheck),
            repository_persist_failed_error(),
            COMPLETED_AT.into(),
        )
        .unwrap();
    fixture
}

fn completed_steps_operation() -> OperationFixture {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();
    for (index, step) in RepositoryInitializationStepKind::ALL
        .iter()
        .copied()
        .enumerate()
    {
        fixture
            .store
            .mark_step_running(
                "project_0001",
                &fixture.operation_id,
                step,
                format!("2026-07-22T00:00:{:02}Z", index * 2 + 2),
            )
            .unwrap();
        fixture
            .store
            .mark_step_completed(
                "project_0001",
                &fixture.operation_id,
                step,
                format!("2026-07-22T00:00:{:02}Z", index * 2 + 3),
            )
            .unwrap();
    }
    fixture
}
