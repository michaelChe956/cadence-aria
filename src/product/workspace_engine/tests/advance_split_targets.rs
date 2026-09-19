//! REQ-MTG-01/02（WP2 分流创建，k3 F3 必改点一）：engine 级多 target 测试族。
//!
//! 夹具路线（wp2-split-report §六.4）：PlanRepairFixture seed + 两 logical 仓
//! （`with_logical_codebase_feature`+manifest+selection+bootstrap policy，T1
//! `split_resolution_fixture` 模式）+ 草稿 per-unit target 改写。单 target 零变化
//! 由既有 `advance_handler` 族锁定，本文件只测分流面。

use super::*;
use crate::product::advance_store::{AdvanceRecord, AdvanceStatus, AdvanceStore};
use crate::product::coding_attempt_store::{CodingAttemptStore, CodingGroupInitializationPhase};
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, IssueCodebaseSelection, IssueCodebaseSelectionStore,
    LogicalCodebaseFeature, LogicalCodebaseStore, LogicalRepositoryId,
};
use crate::product::models::{
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus,
    WorkItemDraftVerificationPlan, WorkItemGenerationMode,
};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::workspace_engine::{
    AdvanceInitializationFailpoint, AdvanceInitializationFailpointMode,
    register_advance_initialization_failpoint,
};
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

const SPLIT_PROJECT_ID: &str = "project_0001";
const SPLIT_ISSUE_ID: &str = "issue_plan_0001";
const SPLIT_PLAN_ID: &str = "work_item_plan_0001";

struct SplitAdvanceFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    api: LogicalRepositoryId,
    web: LogicalRepositoryId,
    lifecycle: LifecycleStore,
}

impl SplitAdvanceFixture {
    fn engine(&self) -> WorkspaceEngine {
        let session_record = self
            .lifecycle
            .list_workspace_sessions(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID)
            .unwrap()
            .into_iter()
            .find(|session| {
                session.workspace_type == WorkspaceType::WorkItemPlan
                    && session.entity_id == SPLIT_PLAN_ID
            })
            .expect("fixture plan workspace session");
        let (event_tx, _event_rx) = mpsc::channel(8);
        WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(self._root.path().join("checkpoints"))),
            self.lifecycle.clone(),
            event_tx,
            WorkspaceSession::from_record(session_record),
        )
    }

    fn coding_store(&self) -> CodingAttemptStore {
        CodingAttemptStore::new(self.paths.clone())
    }
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("start git");
    assert!(status.success(), "git {} failed", args.join(" "));
}

/// 两 logical 仓 + manifest + selection + bootstrap policy + per-unit target 草稿。
/// `unattributed` 为 true 时 wi_unrelated 不带 target（fail-closed 负向夹具）。
async fn split_advance_fixture(unattributed: bool) -> SplitAdvanceFixture {
    let root = tempfile::tempdir().unwrap();
    crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .expect("seed authoritative advance fixture");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store = CodingAttemptStore::new(paths.clone());
    let seeded_attempt = coding_store
        .get_attempt_for_work_item_group(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, SPLIT_PLAN_ID, None)
        .unwrap()
        .expect("seeded group attempt");
    coding_store
        .delete_attempt(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, &seeded_attempt.id)
        .unwrap();

    let mut targets = Vec::new();
    for name in ["split-api", "split-web"] {
        let canonical_path = root.path().join(name);
        fs::create_dir_all(&canonical_path).unwrap();
        run_git(&canonical_path, &["init", "--quiet"]);
        run_git(
            &canonical_path,
            &["config", "user.email", "split@example.test"],
        );
        run_git(&canonical_path, &["config", "user.name", "Split Advance"]);
        fs::write(canonical_path.join("README.md"), format!("# {name}\n")).unwrap();
        run_git(&canonical_path, &["add", "README.md"]);
        run_git(
            &canonical_path,
            &["commit", "--quiet", "-m", "initial commit"],
        );
        let repository = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        )
        .create(CreateRepositoryInput {
            project_id: SPLIT_PROJECT_ID.to_string(),
            name: name.to_string(),
            path: canonical_path,
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: format!("split-advance-{name}"),
        })
        .unwrap();
        targets.push(
            repository
                .logical_repository_id
                .expect("logical repository ID"),
        );
    }
    let [api, web] = targets.as_slice() else {
        panic!("fixture must register two targets");
    };
    let manifest = LogicalCodebaseStore::new(paths.clone())
        .load_manifest(SPLIT_PROJECT_ID)
        .unwrap()
        .expect("manifest");
    AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .unwrap();
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            vec![*api, *web],
            Vec::new(),
            Vec::new(),
            None,
        ))
        .unwrap();

    // unattributed 负向夹具：wi_registration 挪到 web 桶（保持 2 个 attributed
    // 桶），wi_unrelated 置 None——触发「≥2 桶+无归属 unit」fail-closed。
    let wi_registration_target = if unattributed { *web } else { *api };
    let mut per_unit_targets = BTreeMap::new();
    per_unit_targets.insert("wi_core", Some(*api));
    per_unit_targets.insert("wi_registration", Some(wi_registration_target));
    per_unit_targets.insert("wi_unrelated", if unattributed { None } else { Some(*web) });
    seed_split_draft_records(&paths, &per_unit_targets);

    let lifecycle = LifecycleStore::new(paths.clone());
    SplitAdvanceFixture {
        _root: root,
        paths,
        api: *api,
        web: *web,
        lifecycle,
    }
}

fn seed_split_draft_records(
    app_paths: &ProductAppPaths,
    per_unit_targets: &BTreeMap<&str, Option<LogicalRepositoryId>>,
) {
    let revision_store =
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(app_paths.clone());
    let lineage = revision_store
        .get_plan_lineage(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, SPLIT_PLAN_ID)
        .unwrap();
    let plan_store = WorkItemPlanStore::new(app_paths.clone());
    for (logical_id, revision_id) in [
        ("wi_core", "work_item_revision_wi_core_0001"),
        ("wi_registration", "work_item_revision_wi_registration_0001"),
        ("wi_unrelated", "work_item_revision_wi_unrelated_0001"),
    ] {
        let revision = revision_store
            .get_work_item_revision(&lineage, logical_id, revision_id)
            .unwrap();
        plan_store
            .put_draft_record(&WorkItemDraftRecord {
                project_id: SPLIT_PROJECT_ID.to_string(),
                issue_id: SPLIT_ISSUE_ID.to_string(),
                plan_id: SPLIT_PLAN_ID.to_string(),
                draft_id: revision.source_draft_revision_id.clone(),
                outline_id: format!("outline_{logical_id}"),
                generation_round_id: "round_advance".to_string(),
                batch_id: None,
                attempt_index: 1,
                outline_version_ref: "outline_version_advance".to_string(),
                generation_mode: WorkItemGenerationMode::Serial,
                generation_diagnostics: None,
                candidate: WorkItemDraftCandidate {
                    target_repository_id: *per_unit_targets.get(logical_id).expect("unit target"),
                    outline_id: format!("outline_{logical_id}"),
                    logical_work_item_id: logical_id.to_string(),
                    canonical_contract_candidate: revision.canonical_contract,
                    verification_plan: WorkItemDraftVerificationPlan { checks: Vec::new() },
                },
                status: WorkItemDraftStatus::Accepted,
                active: true,
                superseded_by_draft_id: None,
                supersede_reason: None,
                copied_from_draft_id: None,
                review_node_id: None,
                review_verdict_ref: None,
                generated_from_node_id: "split_advance_fixture".to_string(),
                accepted_at: Some("2026-08-31T00:00:00Z".to_string()),
                superseded_at: None,
                created_at: "2026-08-31T00:00:00Z".to_string(),
                updated_at: "2026-08-31T00:00:00Z".to_string(),
            })
            .unwrap();
    }
}

fn split_input(command_id: &str) -> AdvanceInput {
    AdvanceInput {
        command_id: command_id.to_string(),
        project_id: SPLIT_PROJECT_ID.to_string(),
        issue_id: SPLIT_ISSUE_ID.to_string(),
        plan_id: SPLIT_PLAN_ID.to_string(),
    }
}

fn attempt_bindings(record: &AdvanceRecord) -> BTreeMap<String, String> {
    record
        .target_attempts
        .iter()
        .map(|binding| {
            (
                binding.target_repository_id.clone(),
                binding.attempt_id.clone(),
            )
        })
        .collect()
}

fn unit_logical_ids(
    store: &CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> Vec<String> {
    store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap()
        .into_iter()
        .map(|unit| unit.logical_work_item_id)
        .collect()
}

/// Step1 #1（2.1）：多 target 建组产出 N attempts——各自 per-target 冻结快照
/// +units 按 target 正确分组+repo 维 worktree 登记+OQ2 命名+外层集绑定。
#[tokio::test]
async fn split_advance_creates_one_attempt_per_target_with_frozen_snapshots_and_units() {
    let fixture = split_advance_fixture(false).await;
    let mut engine = fixture.engine();

    let outcome = engine.handle_advance(split_input("cmd_split_create")).await;
    let AdvanceOutcome::Completed {
        record,
        attempt_id,
        workspace_entry,
        target_attempts,
    } = outcome.expect("split advance completes")
    else {
        panic!("expected Completed outcome");
    };

    // 每 (plan,target) 恰一 attempt：全集绑定=2 条。
    assert_eq!(target_attempts.len(), 2, "one attempt per target");
    let bindings = attempt_bindings(&record);
    assert_eq!(bindings.len(), 2);
    assert_eq!(
        record.attempt_id.as_deref(),
        Some(attempt_id.as_str()),
        "record single attempt_id carries the topologically-first target attempt",
    );

    let coding_store = fixture.coding_store();
    // api 桶：wi_core+wi_registration（同 target 内依赖序）；web 桶：wi_unrelated。
    let api_attempt = coding_store
        .get_attempt_for_work_item_group(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            SPLIT_PLAN_ID,
            Some(fixture.api),
        )
        .unwrap()
        .expect("api target attempt");
    let web_attempt = coding_store
        .get_attempt_for_work_item_group(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            SPLIT_PLAN_ID,
            Some(fixture.web),
        )
        .unwrap()
        .expect("web target attempt");
    assert_eq!(
        bindings.get(&fixture.api.0.to_string()).map(String::as_str),
        Some(api_attempt.id.as_str()),
    );
    assert_eq!(
        bindings.get(&fixture.web.0.to_string()).map(String::as_str),
        Some(web_attempt.id.as_str()),
    );

    // per-target 冻结快照：身份逐 target 断言。
    let api_snapshot = api_attempt.target_snapshot.as_ref().expect("api snapshot");
    assert_eq!(api_snapshot.logical_repository_id, fixture.api);
    let web_snapshot = web_attempt.target_snapshot.as_ref().expect("web snapshot");
    assert_eq!(web_snapshot.logical_repository_id, fixture.web);

    // units 分组物化：api=[wi_core, wi_registration]（依赖序），web=[wi_unrelated]。
    let api_units = unit_logical_ids(&coding_store, &api_attempt);
    assert_eq!(api_units, vec!["wi_core", "wi_registration"]);
    let web_units = unit_logical_ids(&coding_store, &web_attempt);
    assert_eq!(web_units, vec!["wi_unrelated"]);

    // OQ2 命名：branch/worktree per-target 限定。
    assert_eq!(
        api_attempt.branch_name,
        format!("aria/issues/{SPLIT_ISSUE_ID}/{}", fixture.api.0)
    );
    assert_eq!(
        api_attempt
            .worktree_path
            .as_ref()
            .expect("api worktree path")
            .file_name()
            .and_then(|name| name.to_str()),
        Some(fixture.api.0.to_string().as_str()),
    );

    // per-target journal 子路径 + 外层 journal 集绑定。
    let journals = coding_store
        .list_group_initialization_journals_for_plan(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            SPLIT_PLAN_ID,
        )
        .unwrap();
    assert_eq!(journals.len(), 2, "per-target journals in subpath");
    let advance_store = AdvanceStore::new(fixture.paths.clone());
    let outer = advance_store
        .get_advance_initialization(&record)
        .expect("outer advance journal");
    let mut persisted_ids = outer
        .as_ref()
        .expect("outer journal")
        .target_attempt_ids
        .clone();
    let mut expected_ids = vec![api_attempt.id.clone(), web_attempt.id.clone()];
    persisted_ids.sort_unstable();
    expected_ids.sort_unstable();
    assert_eq!(persisted_ids, expected_ids);
    assert_eq!(
        outer.as_ref().expect("outer journal").attempt_id,
        api_attempt.id,
        "topologically first"
    );

    // repo 维三元键 worktree 登记（T2S3 通过后以 repo 维 upsert 登记）。
    let lifecycle = fixture.lifecycle;
    let api_worktree = lifecycle
        .get_repo_shared_worktree(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, fixture.api)
        .unwrap()
        .expect("api repo worktree registered");
    assert_eq!(api_worktree.target_repository_id, Some(fixture.api));
    assert_eq!(
        api_worktree.current_lock_owner_id.as_deref(),
        Some(api_attempt.id.as_str())
    );
    assert!(
        lifecycle
            .get_repo_shared_worktree(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, fixture.web)
            .unwrap()
            .is_some(),
        "web repo worktree registered",
    );

    // record 首个 attempt 的 workspace entry（单值承载）。
    assert_eq!(
        record.workspace_entry.as_deref(),
        Some(workspace_entry.as_str())
    );
}

/// Step1 #2（2.1/2.3 幂等）：同 command_id 重发/异 command_id 同 plan →
/// Replayed 且不新建 attempt/journal；advance record 每 plan 仍恰一条。
#[tokio::test]
async fn split_advance_is_idempotent_across_replay_and_new_command_ids() {
    let fixture = split_advance_fixture(false).await;
    let mut engine = fixture.engine();

    let first = engine
        .handle_advance(split_input("cmd_split_idem"))
        .await
        .expect("first split advance");
    let AdvanceOutcome::Completed {
        record,
        target_attempts,
        ..
    } = first
    else {
        panic!("expected Completed");
    };
    assert_eq!(target_attempts.len(), 2);

    let AdvanceOutcome::Replayed {
        record: replay_same,
    } = engine
        .handle_advance(split_input("cmd_split_idem"))
        .await
        .expect("same command replay")
    else {
        panic!("expected Replayed for same command_id");
    };
    assert_eq!(replay_same.id, record.id);

    let AdvanceOutcome::Replayed { .. } = engine
        .handle_advance(split_input("cmd_split_other"))
        .await
        .expect("different command replay")
    else {
        panic!("expected Replayed for a new command_id on a ready plan");
    };

    let coding_store = fixture.coding_store();
    let journals = coding_store
        .list_group_initialization_journals_for_plan(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            SPLIT_PLAN_ID,
        )
        .unwrap();
    assert_eq!(journals.len(), 2, "no new journals");
    let attempts = coding_store
        .list_attempts_for_work_item_group(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, SPLIT_PLAN_ID)
        .unwrap();
    assert_eq!(attempts.len(), 2, "no new attempts");
    let advance_store = AdvanceStore::new(fixture.paths.clone());
    assert!(
        advance_store
            .get_advance_by_command_id(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, "cmd_split_other")
            .unwrap()
            .is_none(),
        "no second advance record is persisted for a ready plan",
    );
}

/// Step1 #3（2.1 恢复同套）：failpoint Crash 中断后重放——每 (plan,target)
/// 同一 attempt_id（attempt 集身份不变），per-target journal 完整走完相位。
#[tokio::test]
async fn split_advance_failpoint_recovery_resumes_same_attempt_set() {
    let fixture = split_advance_fixture(false).await;
    let coding_store = fixture.coding_store();

    let request = split_input("cmd_split_recover");
    let failpoint = register_advance_initialization_failpoint(
        &request,
        AdvanceInitializationFailpoint::WorktreeBound,
        AdvanceInitializationFailpointMode::Crash,
    );
    // Crash 模式（进程中断语义）：失败标记不落盘，record 停留 Initializing
    // ——重放走 checkpoint 恢复而非 Replayed(Failed)。
    let mut engine = fixture.engine();
    let crashed_request = request.clone();
    let crashed = tokio::spawn(async move { engine.handle_advance(crashed_request).await });
    assert!(
        crashed.await.is_err(),
        "WorktreeBound failpoint must crash the split advance"
    );
    drop(failpoint);

    let mut engine = fixture.engine();
    let outcome = engine.handle_advance(request).await;
    let AdvanceOutcome::Completed {
        record,
        target_attempts,
        ..
    } = outcome.expect("split advance recovers to completion")
    else {
        panic!("expected Completed after recovery");
    };
    assert_eq!(target_attempts.len(), 2);
    assert_eq!(record.status, AdvanceStatus::Ready);
    assert!(record.error.is_none());

    // 同套 attempts：恢复后 per-(plan,target) journal 完整且 attempt 全在集内。
    let journals = coding_store
        .list_group_initialization_journals_for_plan(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            SPLIT_PLAN_ID,
        )
        .unwrap();
    assert_eq!(journals.len(), 2);
    for journal in &journals {
        assert_eq!(
            journal.phase,
            CodingGroupInitializationPhase::Completed,
            "per-target journal completes"
        );
        assert!(
            target_attempts
                .iter()
                .any(|binding| binding.attempt_id == journal.attempt.id),
            "recovered attempt set identity preserved"
        );
    }
}

/// 失败标记适配（Error 模式）：分流中断的 error 落 record（Failed）与每个
/// per-target journal；record 保留 attempt 集身份。
#[tokio::test]
async fn split_advance_error_failure_marks_per_target_journals_and_record() {
    let fixture = split_advance_fixture(false).await;
    let advance_store = AdvanceStore::new(fixture.paths.clone());
    let coding_store = fixture.coding_store();

    let request = split_input("cmd_split_marked");
    let _failpoint = register_advance_initialization_failpoint(
        &request,
        AdvanceInitializationFailpoint::WorktreeBound,
        AdvanceInitializationFailpointMode::Error,
    );
    let mut engine = fixture.engine();
    let failure = engine.handle_advance(request).await;
    let error = failure.expect_err("failpoint interrupts split advance");
    assert!(
        error.to_lowercase().contains("worktree"),
        "unexpected error: {error}"
    );

    // record Failed + error 落 record。
    let failed_record = advance_store
        .get_advance_by_command_id(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, "cmd_split_marked")
        .unwrap()
        .expect("failed record durably present");
    assert_eq!(failed_record.status, AdvanceStatus::Failed);
    assert!(failed_record.error.is_some());
    // attempt 集身份保留：Failed record 仍携带全集绑定。
    assert_eq!(failed_record.target_attempts.len(), 2);

    // error 落每个 per-target journal。
    let journals = coding_store
        .list_group_initialization_journals_for_plan(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            SPLIT_PLAN_ID,
        )
        .unwrap();
    assert_eq!(journals.len(), 2);
    for journal in &journals {
        assert!(
            journal.error.is_some(),
            "per-target journal {} carries the failure",
            journal.id
        );
    }
}

/// Step1 #9（2.5 无自动编排）：拆分创建后 N attempts 全部停留
/// (Created, PrepareContext)——无 runner spawn、无 StartCoding 副作用。
#[tokio::test]
async fn split_advance_leaves_attempts_unorchestrated_at_created_prepare_context() {
    let fixture = split_advance_fixture(false).await;
    let mut engine = fixture.engine();

    let outcome = engine
        .handle_advance(split_input("cmd_split_noauto"))
        .await
        .expect("split advance completes");
    let AdvanceOutcome::Completed {
        target_attempts, ..
    } = outcome
    else {
        panic!("expected Completed");
    };

    let coding_store = fixture.coding_store();
    for binding in &target_attempts {
        let attempt = coding_store
            .get_attempt(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, &binding.attempt_id)
            .unwrap();
        assert_eq!(
            attempt.status,
            crate::product::coding_models::CodingAttemptStatus::Created
        );
        assert_eq!(
            attempt.stage,
            crate::product::coding_models::CodingExecutionStage::PrepareContext,
            "no target-attempt is auto-started (REQ-MTG-03)",
        );
    }
}

/// Step1 #6（2.3/2.4 守卫集绑定）：Ready record 对集内每个 target-attempt 放行
/// StartCoding 前置；集外 attempt fail-closed。
#[tokio::test]
async fn split_advance_guard_binds_every_target_attempt_and_fails_closed_for_foreign() {
    let fixture = split_advance_fixture(false).await;
    let mut engine = fixture.engine();

    let outcome = engine
        .handle_advance(split_input("cmd_split_guard"))
        .await
        .expect("split advance completes");
    let AdvanceOutcome::Completed { record, .. } = outcome else {
        panic!("expected Completed");
    };

    let advance_store = AdvanceStore::new(fixture.paths.clone());
    for binding in &record.target_attempts {
        assert!(
            advance_store
                .advance_is_ready_for_attempt(
                    SPLIT_PROJECT_ID,
                    SPLIT_ISSUE_ID,
                    SPLIT_PLAN_ID,
                    &binding.attempt_id,
                )
                .unwrap(),
            "guard accepts every bound target attempt"
        );
    }
    assert!(
        !advance_store
            .advance_is_ready_for_attempt(
                SPLIT_PROJECT_ID,
                SPLIT_ISSUE_ID,
                SPLIT_PLAN_ID,
                "attempt_not_in_split_set",
            )
            .unwrap(),
        "guard fails closed for a foreign attempt id"
    );
}

/// T2S3（OQ2 披露）：per-target worktree 父路径 `.worktrees/aria-issues/{issue_id}`
/// 已被 issue 级 shared worktree 记录占用 → fail-closed，且不登记任何 repo 维
/// worktree 子记录。
#[tokio::test]
async fn split_advance_fails_closed_when_parent_worktree_path_already_registered() {
    let fixture = split_advance_fixture(false).await;

    // 预登记 issue 级 shared worktree（占用 api 仓下的父路径）。
    let repository = RepositoryStore::new(fixture.paths.clone())
        .resolve_logical_repository_for_issue_codebase(SPLIT_PROJECT_ID, None, fixture.api)
        .map(|(_, _, repository)| repository)
        .unwrap();
    let parent = repository
        .path
        .join(".worktrees")
        .join("aria-issues")
        .join(SPLIT_ISSUE_ID);
    fixture
        .lifecycle
        .upsert_issue_shared_worktree(
            crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                project_id: SPLIT_PROJECT_ID.to_string(),
                issue_id: SPLIT_ISSUE_ID.to_string(),
                repository_id: repository.id.clone(),
                branch_name: format!("aria/issues/{SPLIT_ISSUE_ID}"),
                worktree_path: parent.clone(),
                base_branch: "master".to_string(),
            },
        )
        .unwrap();

    let mut engine = fixture.engine();
    let failure = engine.handle_advance(split_input("cmd_split_nested")).await;
    let error = failure.expect_err("occupied parent path fails closed");
    assert!(
        error.contains("already registered"),
        "unexpected error: {error}"
    );

    let lifecycle = fixture.lifecycle;
    assert!(
        lifecycle
            .get_repo_shared_worktree(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, fixture.api)
            .unwrap()
            .is_none(),
        "no repo-dim child worktree is registered for the conflicted target",
    );
    let advance_store = AdvanceStore::new(fixture.paths.clone());
    let failed_record = advance_store
        .get_advance_by_command_id(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, "cmd_split_nested")
        .unwrap()
        .expect("failed record present");
    assert_eq!(failed_record.status, AdvanceStatus::Failed);
}

/// D2.1 对偶负向：≥2 target 桶 + 存在无归属 unit → fail-closed（不静默归属）。
#[tokio::test]
async fn split_advance_rejects_unattributed_units_fail_closed() {
    let fixture = split_advance_fixture(true).await;
    let mut engine = fixture.engine();

    let failure = engine
        .handle_advance(split_input("cmd_split_unattributed"))
        .await;
    let error = failure.expect_err("unattributed units fail closed");
    assert!(
        error.contains("target repository"),
        "unexpected error: {error}"
    );
    let advance_store = AdvanceStore::new(fixture.paths.clone());
    let failed_record = advance_store
        .get_advance_by_command_id(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, "cmd_split_unattributed")
        .unwrap()
        .expect("failed record present");
    assert_eq!(failed_record.status, AdvanceStatus::Failed);
}
