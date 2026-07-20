use super::*;
use crate::product::coding_models::{CodingUnitRun, CodingUnitRunStatus};
use crate::product::models::HandoffRevision;
use crate::product::work_item_projection::renderer_for;

struct GroupCompletionFixture {
    _root: tempfile::TempDir,
    worktree: std::path::PathBuf,
    store: CodingAttemptStore,
    engine: CodingWorkspaceEngine,
    attempt: CodingExecutionAttempt,
    original_head: String,
}

fn group_completion_fixture(with_dependency: bool, dirty: bool) -> GroupCompletionFixture {
    group_completion_fixture_at_stage(with_dependency, dirty, CodingExecutionStage::ReviewRequest)
}

fn create_authoritative_active_run(
    fixture: &GroupCompletionFixture,
    id: &str,
    execution_no: u32,
    status: CodingUnitRunStatus,
    completion_commit: Option<String>,
    canonical_contract_hash_override: Option<&str>,
) -> CodingUnitRun {
    create_authoritative_active_run_with_handoffs(
        fixture,
        id,
        execution_no,
        status,
        completion_commit,
        canonical_contract_hash_override,
        Vec::new(),
    )
}

fn create_authoritative_active_run_with_handoffs(
    fixture: &GroupCompletionFixture,
    id: &str,
    execution_no: u32,
    status: CodingUnitRunStatus,
    completion_commit: Option<String>,
    canonical_contract_hash_override: Option<&str>,
    resolved_handoff_revision_ids: Vec<String>,
) -> CodingUnitRun {
    let unit = fixture
        .store
        .get_active_coding_unit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("active unit lookup")
        .expect("active unit");
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &unit.logical_work_item_id,
            &unit.work_item_revision_id,
        )
        .expect("revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("projection bundle");
    let providers = fixture
        .store
        .get_role_provider_config_snapshot(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("provider snapshot");
    let run = CodingUnitRun {
        id: id.to_string(),
        unit_id: unit.id,
        execution_no,
        work_item_revision_id: revision.id,
        resolved_handoff_revision_ids,
        canonical_contract_hash: canonical_contract_hash_override
            .unwrap_or(&revision.canonical_contract_hash)
            .to_string(),
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: renderer_for(&providers.coder)
            .renderer_version()
            .to_string(),
        reviewer_provider_renderer_version: renderer_for(&providers.code_reviewer)
            .renderer_version()
            .to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some(fixture.original_head.clone()),
        completion_commit,
        created_at: "2026-07-19T00:00:00Z".to_string(),
        updated_at: "2026-07-19T00:00:00Z".to_string(),
    };
    fixture
        .store
        .create_coding_unit_run(&fixture.attempt, &run)
        .expect("unit run");
    run
}

fn save_active_legacy_handoff(
    fixture: &GroupCompletionFixture,
    tests_run: Vec<String>,
    files_changed: Vec<String>,
) -> WorkItemHandoff {
    let unit = fixture
        .store
        .get_active_coding_unit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("active unit lookup")
        .expect("active unit");
    let handoff = WorkItemHandoff {
        id: "work_item_handoff_0001".to_string(),
        project_id: fixture.attempt.project_id.clone(),
        issue_id: fixture.attempt.issue_id.clone(),
        work_item_id: unit.logical_work_item_id.clone(),
        attempt_id: fixture.attempt.id.clone(),
        provider_run_ref: None,
        summary: "completed work item".to_string(),
        files_changed,
        commit_sha: None,
        diff_summary: "unit diff".to_string(),
        tests_run,
        test_result_summary: "passed".to_string(),
        review_summary: Some("approved".to_string()),
        api_or_contract_changes: Vec::new(),
        open_risks: Vec::new(),
        next_work_item_notes: Vec::new(),
        created_at: "2026-07-19T00:00:00Z".to_string(),
    };
    fixture
        .store
        .save_coding_unit_handoff(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &unit.id,
            &handoff,
        )
        .expect("legacy handoff");
    handoff
}

fn expected_handoff_revision(
    run: &CodingUnitRun,
    commit_sha: &str,
    created_at: &str,
) -> HandoffRevision {
    HandoffRevision {
        id: format!("handoff_revision_{}", run.id),
        logical_work_item_id: "work_item_0001".to_string(),
        work_item_revision_id: run.work_item_revision_id.clone(),
        coding_unit_run_id: run.id.clone(),
        provided_contracts: vec!["contract_work_item_0001".to_string()],
        provided_capabilities: std::collections::BTreeMap::from([(
            "contract_work_item_0001".to_string(),
            vec!["capability_work_item_0001".to_string()],
        )]),
        contract_hash: "5d1465e86ea2fbad8df040b5eac6ab52130ce6b06d2bd3b6403305c3b3e83b23"
            .to_string(),
        commit_sha: commit_sha.to_string(),
        tests: vec![
            "cargo check --locked".to_string(),
            "cargo test --locked".to_string(),
        ],
        artifacts: vec!["src/a.rs".to_string(), "src/z.rs".to_string()],
        created_at: created_at.to_string(),
    }
}

async fn assert_completion_preflight_is_zero_write(
    fixture: &GroupCompletionFixture,
    error_fragment: &str,
) {
    let units_before = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units before");
    let runs_before = units_before
        .iter()
        .map(|unit| {
            fixture
                .store
                .list_coding_unit_runs(&fixture.attempt, &unit.id)
                .expect("unit runs before")
        })
        .collect::<Vec<_>>();
    let handoffs_before = units_before
        .iter()
        .map(|unit| {
            fixture.store.get_coding_unit_handoff(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &unit.id,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .expect("handoffs before");
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let revision_handoffs_before = revision_store
        .list_handoff_revisions(&lineage, "work_item_0001")
        .expect("revision handoffs before");
    let error = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect_err("preflight must fail before writes");
    assert!(error.to_string().contains(error_fragment), "{error}");
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("attempt after"),
        fixture.attempt
    );
    assert_eq!(
        fixture
            .store
            .list_coding_units(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("units after"),
        units_before
    );
    assert_eq!(
        fixture
            .store
            .list_coding_units(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("units after")
            .iter()
            .map(|unit| {
                fixture
                    .store
                    .list_coding_unit_runs(&fixture.attempt, &unit.id)
                    .expect("unit runs after")
            })
            .collect::<Vec<_>>(),
        runs_before
    );
    assert_eq!(
        units_before
            .iter()
            .map(|unit| {
                fixture.store.get_coding_unit_handoff(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                    &unit.id,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .expect("handoffs after"),
        handoffs_before
    );
    assert_eq!(
        revision_store
            .list_handoff_revisions(&lineage, "work_item_0001")
            .expect("revision handoffs after"),
        revision_handoffs_before
    );
    assert_eq!(
        git_stdout(&fixture.worktree, &["rev-parse", "HEAD"]).trim(),
        fixture.original_head
    );
    assert!(git_stdout(&fixture.worktree, &["status", "--porcelain"]).contains("unit1.txt"));
}

#[tokio::test]
async fn coding_plan_repair_group_completion_missing_run_is_zero_write() {
    let fixture = group_completion_fixture(false, true);
    assert_completion_preflight_is_zero_write(&fixture, "coding_unit_run").await;
}

#[tokio::test]
async fn coding_plan_repair_group_completion_ambiguous_runs_are_zero_write() {
    let fixture = group_completion_fixture(false, true);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0002",
        2,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    assert_completion_preflight_is_zero_write(&fixture, "ambiguous").await;
}

#[tokio::test]
async fn coding_plan_repair_group_completion_stale_run_is_zero_write() {
    let fixture = group_completion_fixture(false, true);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Stale,
        None,
        None,
    );
    assert_completion_preflight_is_zero_write(&fixture, "not_authoritative").await;
}

#[tokio::test]
async fn coding_plan_repair_group_completion_mismatched_run_is_zero_write() {
    let fixture = group_completion_fixture(false, true);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        Some("wrong_contract_hash"),
    );
    assert_completion_preflight_is_zero_write(&fixture, "binding_mismatch").await;
}

#[tokio::test]
async fn coding_plan_repair_group_completion_conflicting_handoff_is_zero_write() {
    let fixture = group_completion_fixture(false, true);
    let run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let mut conflicting =
        expected_handoff_revision(&run, &fixture.original_head, "2026-07-19T00:00:00Z");
    conflicting.contract_hash = "conflicting_contract_hash".to_string();
    revision_store
        .put_handoff_revision(&lineage, &conflicting)
        .expect("conflicting immutable handoff");

    assert_completion_preflight_is_zero_write(&fixture, "handoff_revision_conflict").await;
    assert_eq!(
        revision_store
            .get_handoff_revision(&lineage, &conflicting.logical_work_item_id, &conflicting.id,)
            .expect("conflicting handoff remains immutable"),
        conflicting
    );
}

#[tokio::test]
async fn coding_plan_repair_group_completion_publishes_dependency_handoff_for_next_context() {
    let fixture = group_completion_fixture(true, true);
    let source_run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    save_active_legacy_handoff(
        &fixture,
        vec![
            "cargo test --locked".to_string(),
            "cargo check --locked".to_string(),
            "cargo test --locked".to_string(),
        ],
        vec![
            "src/z.rs".to_string(),
            "src/a.rs".to_string(),
            "src/z.rs".to_string(),
        ],
    );

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("complete first unit");
    let units = fixture
        .store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units");
    let first = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0001")
        .expect("first unit");
    let handoff_id = first
        .latest_handoff_revision_id
        .as_deref()
        .expect("canonical handoff pointer");
    let rendered = fixture
        .engine
        .render_coder_unit_run_context(&updated, &ProviderName::Codex, None)
        .expect("next unit context")
        .expect("group context");
    let next_run = fixture
        .store
        .get_active_unit_run(&updated)
        .expect("next unit run");
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &updated.project_id,
            &updated.issue_id,
            updated.work_item_group_id.as_deref().expect("plan id"),
        )
        .expect("lineage");
    let handoff = revision_store
        .get_handoff_revision(&lineage, &first.logical_work_item_id, handoff_id)
        .expect("canonical handoff");
    let persisted_source_run = fixture
        .store
        .list_coding_unit_runs(&updated, &first.id)
        .expect("source runs")
        .into_iter()
        .find(|run| run.id == source_run.id)
        .expect("source run");

    assert_eq!(
        next_run.resolved_handoff_revision_ids,
        vec![handoff_id.to_string()]
    );
    assert!(rendered.text.contains(handoff_id));
    assert_eq!(persisted_source_run.status, CodingUnitRunStatus::Completed);
    assert_eq!(
        handoff,
        expected_handoff_revision(
            &persisted_source_run,
            persisted_source_run
                .completion_commit
                .as_deref()
                .expect("completion commit"),
            &handoff.created_at,
        )
    );
}

#[tokio::test]
async fn coding_plan_repair_group_completion_rejects_dependency_handoff_binding_mismatch() {
    let mut fixture = group_completion_fixture(true, true);
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    let after_first = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("complete source unit");
    fixture.attempt = fixture
        .store
        .update_attempt_stage(
            &after_first.project_id,
            &after_first.issue_id,
            &after_first.id,
            CodingExecutionStage::ReviewRequest,
        )
        .expect("second unit review request stage");
    fixture.original_head = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(fixture.worktree.join("unit1.txt"), "unit 2 change\n").expect("unit 2 change");
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0002",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );

    assert_completion_preflight_is_zero_write(&fixture, "handoff_binding_mismatch").await;
}

#[tokio::test]
async fn coding_plan_repair_group_completion_rejects_noncanonical_dependency_handoff_identity() {
    for alias_handoff in [false, true] {
        let mut fixture = group_completion_fixture(true, true);
        create_authoritative_active_run(
            &fixture,
            "coding_unit_run_0001",
            1,
            CodingUnitRunStatus::Running,
            None,
            None,
        );
        fixture.attempt = fixture
            .engine
            .complete_group_unit_after_code_review(&fixture.attempt)
            .await
            .expect("complete source unit");
        fixture.attempt = fixture
            .store
            .update_attempt_stage(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                CodingExecutionStage::ReviewRequest,
            )
            .expect("second unit review request stage");
        fixture.original_head = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
            .trim()
            .to_string();
        let units = fixture
            .store
            .list_coding_units(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("units");
        let dependency = units
            .iter()
            .find(|unit| unit.logical_work_item_id == "work_item_0001")
            .expect("dependency unit");
        let handoff_id = dependency
            .latest_handoff_revision_id
            .as_ref()
            .expect("dependency handoff")
            .clone();
        let resolved_handoff_id = if alias_handoff {
            let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
            let lineage = revision_store
                .get_plan_lineage(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    "work_item_plan_0001",
                )
                .expect("lineage");
            let mut alias = revision_store
                .get_handoff_revision(&lineage, "work_item_0001", &handoff_id)
                .expect("canonical handoff");
            alias.id = "handoff_revision_alias".to_string();
            revision_store
                .put_handoff_revision(&lineage, &alias)
                .expect("alias handoff");
            fixture
                .store
                .update_coding_unit_latest_handoff_revision_id(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                    &dependency.id,
                    Some(alias.id.clone()),
                )
                .expect("alias pointer");
            alias.id
        } else {
            fixture
                .store
                .update_coding_unit_completion_commit(
                    &fixture.attempt.project_id,
                    &fixture.attempt.issue_id,
                    &fixture.attempt.id,
                    &dependency.id,
                    Some("mismatched_commit".to_string()),
                )
                .expect("mismatched unit commit");
            handoff_id
        };
        fs::write(fixture.worktree.join("unit1.txt"), "unit 2 change\n").expect("unit 2 change");
        create_authoritative_active_run_with_handoffs(
            &fixture,
            "coding_unit_run_0002",
            1,
            CodingUnitRunStatus::Running,
            None,
            None,
            vec![resolved_handoff_id],
        );

        assert_completion_preflight_is_zero_write(&fixture, "handoff_binding_mismatch").await;
    }
}

#[tokio::test]
async fn coding_plan_repair_group_completion_recovers_completed_run_without_new_commit() {
    let fixture = group_completion_fixture(true, true);
    run_test_git(&fixture.worktree, &["add", "."]);
    run_test_git(
        &fixture.worktree,
        &["commit", "-m", "feat: complete work_item_0001"],
    );
    let completion_commit = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let commit_count = git_stdout(&fixture.worktree, &["rev-list", "--count", "HEAD"]);
    let source_run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Completed,
        Some(completion_commit.clone()),
        None,
    );
    save_active_legacy_handoff(
        &fixture,
        vec![
            "cargo test --locked".to_string(),
            "cargo check --locked".to_string(),
            "cargo test --locked".to_string(),
        ],
        vec![
            "src/z.rs".to_string(),
            "src/a.rs".to_string(),
            "src/z.rs".to_string(),
        ],
    );
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    let existing_handoff =
        expected_handoff_revision(&source_run, &completion_commit, "2026-07-19T00:00:00Z");
    revision_store
        .put_handoff_revision(&lineage, &existing_handoff)
        .expect("partial canonical handoff");

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("recover partial completion");
    let units = fixture
        .store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units");
    let first = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0001")
        .expect("first unit");
    let second = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("second unit");
    let handoff_id = first
        .latest_handoff_revision_id
        .as_deref()
        .expect("canonical handoff pointer");
    let handoff = revision_store
        .get_handoff_revision(&lineage, &first.logical_work_item_id, handoff_id)
        .expect("canonical handoff");
    let persisted_run = fixture
        .store
        .list_coding_unit_runs(&updated, &first.id)
        .expect("unit runs")
        .into_iter()
        .find(|run| run.id == source_run.id)
        .expect("source run");

    assert_eq!(
        git_stdout(&fixture.worktree, &["rev-parse", "HEAD"]).trim(),
        completion_commit
    );
    assert_eq!(
        git_stdout(&fixture.worktree, &["rev-list", "--count", "HEAD"]),
        commit_count
    );
    assert_eq!(
        updated.head_commit.as_deref(),
        Some(completion_commit.as_str())
    );
    assert_eq!(
        first.completion_commit.as_deref(),
        Some(completion_commit.as_str())
    );
    assert_eq!(first.status, CodingExecutionUnitStatus::Completed);
    assert_eq!(second.status, CodingExecutionUnitStatus::Running);
    assert_eq!(updated.active_unit_id.as_deref(), Some(second.id.as_str()));
    assert_eq!(persisted_run.status, CodingUnitRunStatus::Completed);
    assert_eq!(
        persisted_run.completion_commit.as_deref(),
        Some(completion_commit.as_str())
    );
    assert_eq!(handoff.coding_unit_run_id, source_run.id);
    assert_eq!(handoff.commit_sha, completion_commit);
    assert_eq!(handoff, existing_handoff);
    assert!(
        fixture
            .store
            .get_coding_unit_handoff(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                &first.id,
            )
            .expect("legacy handoff lookup")
            .is_some()
    );
}

include!("group_completion_recovery.rs");
include!("runtime_handoff_group_completion.rs");
