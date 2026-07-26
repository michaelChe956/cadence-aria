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
    ) -> Result<(), Box<RepositoryRegistrationError>> {
        self.events.lock().unwrap().push((step, "started"));
        Ok(())
    }

    fn step_completed(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), Box<RepositoryRegistrationError>> {
        self.events.lock().unwrap().push((step, "completed"));
        Ok(())
    }
}

fn report_operation_progress(
    progress: &dyn RepositoryInitializationProgress,
) -> Result<(), Box<RepositoryRegistrationError>> {
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
        git_finalize_warning: None,
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

include!("tests/lifecycle.rs");
include!("tests/record_validation.rs");

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
        if step == RepositoryInitializationStepKind::GitFinalize {
            fixture
                .store
                .checkpoint_git_finalize_result(
                    "project_0001",
                    &fixture.operation_id,
                    success_result("project_0001"),
                )
                .unwrap();
        }
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

fn running_git_finalize_operation() -> OperationFixture {
    let fixture = created_operation();
    fixture
        .store
        .mark_running("project_0001", &fixture.operation_id, RUNNING_AT.into())
        .unwrap();
    for (index, step) in [
        RepositoryInitializationStepKind::CadenceSkills,
        RepositoryInitializationStepKind::PreCheck,
        RepositoryInitializationStepKind::RuleConfig,
        RepositoryInitializationStepKind::McpConfiguration,
        RepositoryInitializationStepKind::ProjectRulesExamples,
    ]
    .into_iter()
    .enumerate()
    {
        fixture
            .store
            .mark_step_running(
                "project_0001",
                &fixture.operation_id,
                step,
                format!("2026-07-22T00:01:{:02}Z", index * 2),
            )
            .unwrap();
        fixture
            .store
            .mark_step_completed(
                "project_0001",
                &fixture.operation_id,
                step,
                format!("2026-07-22T00:01:{:02}Z", index * 2 + 1),
            )
            .unwrap();
    }
    fixture
        .store
        .mark_step_running(
            "project_0001",
            &fixture.operation_id,
            RepositoryInitializationStepKind::GitFinalize,
            "2026-07-22T00:01:10Z".into(),
        )
        .unwrap();
    fixture
}
