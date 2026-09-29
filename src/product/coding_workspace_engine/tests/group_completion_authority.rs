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

struct UnitRunHandoffStart {
    resolved_handoff_revision_ids: Vec<String>,
    start_commit: Option<String>,
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
        UnitRunHandoffStart {
            resolved_handoff_revision_ids: Vec::new(),
            start_commit: Some(fixture.original_head.clone()),
        },
    )
}

fn create_authoritative_active_run_with_handoffs(
    fixture: &GroupCompletionFixture,
    id: &str,
    execution_no: u32,
    status: CodingUnitRunStatus,
    completion_commit: Option<String>,
    canonical_contract_hash_override: Option<&str>,
    handoff_start: UnitRunHandoffStart,
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
        resolved_handoff_revision_ids: handoff_start.resolved_handoff_revision_ids,
        canonical_contract_hash: canonical_contract_hash_override
            .unwrap_or(&revision.canonical_contract_hash)
            .to_string(),
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: renderer_for(&providers.coder)
            .renderer_version()
            .to_string(),
        reviewer_provider_renderer_version: renderer_version_for_optional_provider(
            providers.code_reviewer.as_ref(),
        )
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
        start_commit: handoff_start.start_commit,
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
async fn group_completion_records_existing_coder_head_without_staging_or_commit() {
    let fixture = group_completion_fixture(false, false);
    fs::write(fixture.worktree.join("unit1.txt"), "coder-owned change\n").expect("coder change");
    run_test_git(&fixture.worktree, &["add", "unit1.txt"]);
    run_test_git(&fixture.worktree, &["commit", "-m", "coder completes unit"]);
    let coder_head = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    fs::write(
        fixture.worktree.join("unowned-staged.txt"),
        "preserve and report\n",
    )
    .expect("unowned staged change");
    run_test_git(&fixture.worktree, &["add", "unowned-staged.txt"]);
    let source_run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("record the existing coder head");
    let persisted_run = fixture
        .store
        .list_coding_unit_runs(&updated, &source_run.unit_id)
        .expect("source runs")
        .into_iter()
        .find(|run| run.id == source_run.id)
        .expect("source run");

    assert_eq!(
        git_stdout(&fixture.worktree, &["rev-parse", "HEAD"]).trim(),
        coder_head
    );
    assert!(
        git_stdout(&fixture.worktree, &["diff", "--cached", "--name-only"])
            .contains("unowned-staged.txt")
    );
    assert_eq!(
        persisted_run.completion_commit.as_deref(),
        Some(coder_head.as_str())
    );
}

/// C2 Task 6（决策 6/#13）：execution 完成时无已记录 start_commit：
/// MUST NOT 按 base_branch HEAD 回填——attempt 转人工恢复停等并持久化
/// 起点缺失诊断（稳定码 unit_run_start_commit_missing），run 保持未完成。
#[tokio::test]
async fn group_unit_completion_missing_start_commit_stops_with_diagnostic_without_backfill() {
    let fixture = group_completion_fixture(false, false);
    fs::write(fixture.worktree.join("unit1.txt"), "coder-owned change\n").expect("coder change");
    run_test_git(&fixture.worktree, &["add", "unit1.txt"]);
    run_test_git(
        &fixture.worktree,
        &["commit", "-m", "coder completes first unit"],
    );
    let source_run = create_authoritative_active_run_with_handoffs(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
        UnitRunHandoffStart {
            resolved_handoff_revision_ids: Vec::new(),
            start_commit: None,
        },
    );

    let error = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect_err("missing start_commit must stop completion");
    assert!(
        error.to_string().contains("unit_run_start_commit_missing"),
        "{error}"
    );
    let persisted_attempt = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("attempt after stop");
    assert_eq!(
        persisted_attempt.status,
        CodingAttemptStatus::AwaitingManualRecovery
    );
    assert_eq!(
        persisted_attempt.manual_recovery_reason.as_deref(),
        Some("unit_run_start_commit_missing")
    );
    let persisted_run = fixture
        .store
        .list_coding_unit_runs(&persisted_attempt, &source_run.unit_id)
        .expect("source runs")
        .into_iter()
        .find(|run| run.id == source_run.id)
        .expect("source run");
    assert!(
        persisted_run.start_commit.is_none(),
        "MUST NOT 按 base HEAD 回填：{:?}",
        persisted_run.start_commit
    );
    assert_eq!(persisted_run.status, CodingUnitRunStatus::Running);
    let diagnostics = fixture
        .store
        .list_chat_entries(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("chat entries");
    assert!(diagnostics.iter().any(|entry| {
        matches!(
            &entry.entry_type,
            crate::product::coding_models::CodingEntryType::SystemEvent {
                event_type,
                message
            } if event_type == "manual_recovery_transition"
                && message.contains("unit_run_start_commit_missing")
        )
    }));
}

#[tokio::test]
async fn group_scope_gate_validates_full_unit_run_range_after_rework() {
    let fixture = group_completion_fixture(false, false);
    fs::write(
        fixture.worktree.join("initial-change.rs"),
        "initial implementation\n",
    )
    .expect("initial change");
    run_test_git(&fixture.worktree, &["add", "initial-change.rs"]);
    run_test_git(
        &fixture.worktree,
        &["commit", "-m", "coder initial implementation"],
    );
    fs::write(fixture.worktree.join("rework-change.rs"), "review rework\n").expect("rework change");
    run_test_git(&fixture.worktree, &["add", "rework-change.rs"]);
    run_test_git(&fixture.worktree, &["commit", "-m", "coder review rework"]);
    let completion_commit = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Completed,
        Some(completion_commit),
        None,
    );

    let changed_files = fixture
        .engine
        .changed_files_for_unit_completion_range(&fixture.attempt, &run)
        .await
        .expect("read the completed unit range");

    assert_eq!(
        changed_files,
        vec![
            "initial-change.rs".to_string(),
            "rework-change.rs".to_string()
        ]
    );
}

#[tokio::test]
async fn group_scope_gate_treats_equal_start_and_completion_as_empty() {
    let fixture = group_completion_fixture(false, false);
    let run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Completed,
        Some(fixture.original_head.clone()),
        None,
    );

    let changed_files = fixture
        .engine
        .changed_files_for_unit_completion_range(&fixture.attempt, &run)
        .await
        .expect("equal range is an empty observation");

    assert!(changed_files.is_empty());
}

#[tokio::test]
async fn group_scope_gate_keeps_paths_reverted_later_in_the_unit_run() {
    let fixture = group_completion_fixture(false, false);
    fs::write(
        fixture.worktree.join("forbidden-during-run.rs"),
        "temporary violation\n",
    )
    .expect("introduce forbidden path");
    run_test_git(&fixture.worktree, &["add", "forbidden-during-run.rs"]);
    run_test_git(
        &fixture.worktree,
        &["commit", "-m", "coder introduces forbidden path"],
    );
    fs::remove_file(fixture.worktree.join("forbidden-during-run.rs"))
        .expect("revert forbidden path");
    run_test_git(&fixture.worktree, &["add", "-u"]);
    run_test_git(
        &fixture.worktree,
        &["commit", "-m", "coder reverts forbidden path"],
    );
    let completion_commit = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Completed,
        Some(completion_commit),
        None,
    );

    let changed_files = fixture
        .engine
        .changed_files_for_unit_completion_range(&fixture.attempt, &run)
        .await
        .expect("preserve all paths touched by the unit run");

    assert_eq!(changed_files, vec!["forbidden-during-run.rs".to_string()]);
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
            UnitRunHandoffStart {
                resolved_handoff_revision_ids: vec![resolved_handoff_id],
                start_commit: Some(fixture.original_head.clone()),
            },
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
}

/// C2 Task 6（决策 6/#13）：重试 execution 的 start_commit 不继承旧值；
/// 认领前以当时 worktree 真实 HEAD（含用户人工 WIP 提交）冻结为自身
/// 起点；零提交完成＝空区间（start==completion），人工提交不归属当前
/// Work Item；同 execution 重复冻结不改写起点。
#[tokio::test]
async fn retry_execution_freezes_real_head_and_empty_range_excludes_manual_wip() {
    let fixture = group_completion_fixture(false, false);
    let prior = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    // 前一 execution 失败后，用户在工作树留下人工提交（不属于本 Work Item）。
    fs::write(fixture.worktree.join("manual-wip.txt"), "human wip\n").expect("manual wip");
    run_test_git(&fixture.worktree, &["add", "manual-wip.txt"]);
    run_test_git(
        &fixture.worktree,
        &["commit", "-m", "manual wip outside coding"],
    );
    let manual_head = git_stdout(&fixture.worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();

    let retry = fixture
        .store
        .create_retry_coding_unit_run(&fixture.attempt, &prior.unit_id, &prior.id)
        .expect("retry execution");
    assert_eq!(retry.execution_no, 2);
    assert!(
        retry.start_commit.is_none(),
        "重试 execution 不继承旧 start_commit"
    );

    // 认领前冻结真实 HEAD（包含人工 WIP 提交）。
    fixture
        .engine
        .freeze_active_unit_run_start_commit(&fixture.attempt, &fixture.worktree)
        .await
        .expect("freeze real head before claim");
    let frozen = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &retry.unit_id)
        .expect("unit runs")
        .into_iter()
        .find(|run| run.id == retry.id)
        .expect("retry run");
    assert_eq!(frozen.start_commit.as_deref(), Some(manual_head.as_str()));

    // 重连／恢复同一 execution：起点保持不变（不覆盖为新含义）。
    fixture
        .engine
        .freeze_active_unit_run_start_commit(&fixture.attempt, &fixture.worktree)
        .await
        .expect("re-freeze keeps the recorded start");
    let refrozen = fixture
        .store
        .list_coding_unit_runs(&fixture.attempt, &retry.unit_id)
        .expect("unit runs")
        .into_iter()
        .find(|run| run.id == retry.id)
        .expect("retry run");
    assert_eq!(
        refrozen.start_commit.as_deref(),
        Some(manual_head.as_str())
    );

    // 零提交完成：区间为空（start==completion），人工提交不进入当前 Work Item。
    let completed = fixture
        .store
        .complete_coding_unit_run(&fixture.attempt, &retry.id, &manual_head)
        .expect("zero-commit completion");
    let changed_files = fixture
        .engine
        .changed_files_for_unit_completion_range(&fixture.attempt, &completed)
        .await
        .expect("empty completion range");
    assert!(
        changed_files.is_empty(),
        "人工 WIP 提交不归属当前 Work Item：{changed_files:?}"
    );
}


/// C2 Task 7（#18／BYPASS-18）：渲染失败不消费——rework 路径在完整 prompt
/// 渲染失败（组 attempt 投影绑定损坏）时，指令必须保持未消费、可被用户
/// 重试读取；消费标记 MUST NOT 早于渲染完成。
#[tokio::test]
async fn rework_render_failure_keeps_instruction_unconsumed() {
    let fixture =
        group_completion_fixture_at_stage(false, false, CodingExecutionStage::CodeReview);
    // 预置一个 canonical_contract_hash 不匹配的活跃 run，令渲染 fail-closed。
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        1,
        CodingUnitRunStatus::Running,
        None,
        Some("wrong_contract_hash"),
    );
    let provider = super::provider_driven::ReviewerDrivenReworkProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = fixture
        .engine
        .execute_coder_fix_from_review(
            &fixture.attempt,
            &super::provider_driven::review_report_requesting_changes(&fixture.attempt),
            &CodingExecutionContext::default(),
            &provider,
            &mut command_rx,
        )
        .await
        .expect_err("render failure must stop the rework path");
    assert!(
        error.to_string().contains("unit_run_projection_binding_mismatch"),
        "{error}"
    );

    // 指令已落地但保持未消费，用户重试仍能读取。
    let instructions = fixture
        .store
        .list_rework_instructions(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    assert!(
        instructions[0].consumed_at.is_none(),
        "渲染失败时指令必须保持未消费"
    );
    assert!(instructions[0].consumed_by_node_id.is_none());
    assert!(
        fixture
            .store
            .list_rework_instruction_claims(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id
            )
            .expect("claims")
            .is_empty(),
        "渲染失败禁认领"
    );
}

include!("group_completion_recovery.rs");
include!("runtime_handoff_group_completion.rs");
include!("runner_fallback_commit.rs");

