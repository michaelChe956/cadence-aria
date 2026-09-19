use super::*;

// REQ-MTG-01/02（WP2 拆分创建·Step 3 前置）：per-`(plan, target)` 唯一性、
// per-`(issue, target)` 单 active、per-target journal 子路径与 D2.1 四断言的
// 存储面测试（advance 编排面的分流循环在 workspace_engine 侧另有族覆盖）。

const TARGET_A: LogicalRepositoryId = LogicalRepositoryId(uuid::Uuid::nil());
const TARGET_B: LogicalRepositoryId = LogicalRepositoryId(uuid::Uuid::from_u64_pair(
    0x2222_2222_2222_2222,
    0x2222_2222_2222_2222,
));

fn target_snapshot(logical_id: LogicalRepositoryId) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: logical_id,
        checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
        physical_repository_id: "repository_split".to_string(),
        canonical_path: std::path::PathBuf::from("/split/repository"),
        git_dir_identity: "split-git-dir".to_string(),
        revision: Some("split-revision".to_string()),
        policy_digest: "split-policy".to_string(),
        membership_revision: 1,
        captured_at: "2026-09-19T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

fn group_input(plan_id: &str, target: Option<LogicalRepositoryId>) -> CreateGroupCodingAttemptInput {
    CreateGroupCodingAttemptInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        current_work_item_id: WORK_ITEM_ID.to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: None,
        provider_config_snapshot: provider_snapshot(),
        target_snapshot: target.map(target_snapshot),
        max_auto_rework: 2,
    }
}

fn active_attempt_for_target(store: &CodingAttemptStore, target: Option<LogicalRepositoryId>) {
    store
        .create_group_attempt(group_input("work_item_plan_0001", target))
        .expect("seed active attempt for the target bucket");
}

fn complete_attempt(store: &CodingAttemptStore, attempt: &mut CodingExecutionAttempt) {
    let completed_at = chrono::Utc::now().to_rfc3339();
    attempt.status = CodingAttemptStatus::Completed;
    attempt.completed_at = Some(completed_at.clone());
    attempt.updated_at = completed_at;
    store
        .write_coding_attempt_for_test(attempt)
        .expect("persist terminal attempt fixture");
}


#[test]
fn same_target_second_attempt_rejected_per_plan_target() {
    // REQ-MTG-02（2.2 唯一性细化）：per-(plan,target) 第二 attempt 被拒
    // （coding_attempt_group_already_exists）——同 plan 异 target 不受影响。
    let (_tmp, store) = setup_store();
    let mut first = store
        .create_group_attempt(group_input("work_item_plan_0001", Some(TARGET_A)))
        .expect("first target attempt");
    let first_id = first.id.clone();
    complete_attempt(&store, &mut first);

    let error = store
        .create_group_attempt(group_input("work_item_plan_0001", Some(TARGET_A)))
        .expect_err("same (plan, target) must reject a second attempt");
    assert!(
        error
            .to_string()
            .contains(&format!("coding_attempt_group_already_exists: {first_id}")),
        "unexpected error: {error}"
    );

    let second_target = store
        .create_group_attempt(group_input("work_item_plan_0001", Some(TARGET_B)))
        .expect("same plan different target is not blocked by per-(plan,target) uniqueness");
    assert_eq!(
        second_target
            .target_snapshot
            .as_ref()
            .map(|snapshot| snapshot.logical_repository_id),
        Some(TARGET_B)
    );
}

#[test]
fn per_target_retrieval_returns_only_matching_bucket() {
    // REQ-MTG-02 + D2.1 A2：per-(plan,target) 检索仅命中该 target——无快照
    // attempt 不入任何 target 桶（含「最早桶」）。
    let (_tmp, store) = setup_store();
    let attempt_a = store
        .create_group_attempt(group_input("work_item_plan_0001", Some(TARGET_A)))
        .expect("target A attempt");
    let attempt_b = store
        .create_group_attempt(group_input("work_item_plan_0001", Some(TARGET_B)))
        .expect("target B attempt");

    let retrieved_a = store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, "work_item_plan_0001", Some(TARGET_A))
        .expect("retrieve target A bucket")
        .expect("target A attempt exists");
    assert_eq!(retrieved_a.id, attempt_a.id);
    let retrieved_b = store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, "work_item_plan_0001", Some(TARGET_B))
        .expect("retrieve target B bucket")
        .expect("target B attempt exists");
    assert_eq!(retrieved_b.id, attempt_b.id);

    let listed = store
        .list_attempts_for_work_item_group(PROJECT_ID, ISSUE_ID, "work_item_plan_0001")
        .expect("list all plan attempts");
    assert_eq!(
        listed.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
        vec![attempt_a.id.clone(), attempt_b.id.clone()],
        "plan-level listing keeps attempt_no order across targets"
    );
}

#[test]
fn legacy_bucket_retrieval_keeps_snapshotless_attempts() {
    // 单 target/legacy 零变化：无 target 键检索=无快照桶（现行语义保持）。
    let (_tmp, store) = setup_store();
    let legacy = store
        .create_group_attempt(group_input("work_item_plan_0001", None))
        .expect("legacy attempt");

    let retrieved = store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, "work_item_plan_0001", None)
        .expect("legacy bucket retrieval")
        .expect("legacy attempt is in the snapshotless bucket");
    assert_eq!(retrieved.id, legacy.id);
}

#[test]
fn snapshotless_attempt_never_blocks_or_pollutes_target_buckets() {
    // D2.1 A1/A2/A4：同 plan 无快照 attempt 存在时新建 target-attempt 不被
    // `coding_attempt_group_already_exists` 拒绝；无快照 attempt 不出现在任何
    // target 桶检索；其 active 状态不阻塞异 target 新建（单 active 桶豁免）。
    let (_tmp, store) = setup_store();
    let legacy = store
        .create_group_attempt(group_input("work_item_plan_0001", None))
        .expect("legacy attempt is created active");

    let target_attempt = store
        .create_group_attempt(group_input("work_item_plan_0001", Some(TARGET_A)))
        .expect("A1: snapshotless attempt does not block target attempt creation");

    let retrieved = store
        .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, "work_item_plan_0001", Some(TARGET_A))
        .expect("target bucket")
        .expect("A2: the bucket holds the target attempt, never the legacy one");
    assert_eq!(retrieved.id, target_attempt.id);
    assert!(
        store
            .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, "work_item_plan_0001", Some(TARGET_B))
            .expect("empty target bucket")
            .is_none(),
        "A2: legacy attempt is not silently attributed to another target"
    );
    assert!(legacy.status.is_active(), "fixture keeps legacy attempt active");
}

#[test]
fn single_active_refined_per_issue_target() {
    // REQ-MTG-02（2.2 单 active 细化）：同 (issue, target) 第二 active attempt
    // 拒（active_coding_attempt_exists）；异 target 并存合法。
    let (_tmp, store) = setup_store();
    active_attempt_for_target(&store, Some(TARGET_A));

    let error = store
        .create_group_attempt(group_input("work_item_plan_0002", Some(TARGET_A)))
        .expect_err("same (issue, target) active attempt must reject");
    assert!(
        error.to_string().contains("active_coding_attempt_exists"),
        "unexpected error: {error}"
    );

    let different_target = store
        .create_group_attempt(group_input("work_item_plan_0002", Some(TARGET_B)))
        .expect("different target runs in parallel with the active attempt");
    assert!(different_target.status.is_active());
}

// ───────────────────────────────────────────────────────────────
// per-target journal 子路径（REQ-MTG-02，OQ2 定案 journal 路径规则）
// ───────────────────────────────────────────────────────────────

fn logical_routing_fixture() -> (TempDir, CodingAttemptStore, Vec<String>) {
    let tmp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let project = crate::product::project_store::ProjectStore::new(paths.clone())
        .create(crate::product::project_store::CreateProjectInput {
            name: "group split target".to_string(),
            description: None,
        })
        .unwrap();
    let target_a = LogicalRepositoryId(uuid::Uuid::new_v4());
    let target_b = LogicalRepositoryId(uuid::Uuid::new_v4());
    crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
        .save_manifest(
            &project.id,
            &crate::product::logical_codebase::LogicalCodebaseManifest::new(
                &project.id,
                tmp.path().join("logical-root"),
                vec![target_a, target_b],
            ),
        )
        .unwrap();
    let issue = crate::product::issue_store::IssueStore::new(paths.clone())
        .create(crate::product::issue_store::CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: None,
            logical_codebase_id: None,
            title: "group split target issue".to_string(),
            description: None,
            change_id: None,
        })
        .unwrap();
    let issue_id = issue.id;
    crate::product::logical_codebase::IssueCodebaseSelectionStore::new(paths.clone())
        .save(&crate::product::logical_codebase::IssueCodebaseSelection::explicit(
            &project.id,
            &issue_id,
            vec![target_a, target_b],
            Vec::new(),
            vec![target_a],
            None,
        ))
        .unwrap();
    (
        tmp,
        CodingAttemptStore::new(paths),
        vec![project.id, issue_id, target_a.0.to_string(), target_b.0.to_string()],
    )
}

fn split_unit(target: LogicalRepositoryId, logical_id: &str) -> AuthoritativeCodingUnitBinding {
    AuthoritativeCodingUnitBinding {
        logical_work_item_id: logical_id.to_string(),
        work_item_revision_id: format!("revision_{logical_id}"),
        verification_plan_revision_id: format!("verification_{logical_id}"),
        projection_bundle_id: format!("projection_{logical_id}"),
        target_repository_id: Some(target),
        source_draft_error: None,
        dependency_logical_work_item_ids: Vec::new(),
    }
}

#[test]
fn split_journal_lives_in_target_subpath_and_lists_back() {
    // OQ2 journal 路径规则：多 target journal 用 {plan_id}/{logical_id}.json
    // 子目录；读取先试原路径再列子目录（存量单 target journal 原路径零迁移）。
    let (_tmp, store, fixture_ids) = logical_routing_fixture();
    let [project_id, issue_id, target_a, target_b] = fixture_ids.as_slice() else {
        panic!("fixture ids");
    };
    let target_a = LogicalRepositoryId(uuid::Uuid::parse_str(target_a).unwrap());
    let target_b = LogicalRepositoryId(uuid::Uuid::parse_str(target_b).unwrap());
    let plan_id = "work_item_plan_split_0001";
    let input = CreateGroupCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.to_string(),
        current_work_item_id: "wi_split_a".to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria/issues/{issue_id}/{}", target_a.0),
        worktree_path: None,
        provider_config_snapshot: provider_snapshot(),
        target_snapshot: Some(target_snapshot(target_a)),
        max_auto_rework: 2,
    };

    let journal_a = store
        .prepare_group_initialization_with_admission_for_target(
            &input,
            "plan_revision_split",
            &[split_unit(target_a, "wi_split_a")],
            CodingAdmissionKind::ScAdvance,
            Some(target_a),
        )
        .expect("prepare per-target journal");
    assert_eq!(journal_a.attempt.target_snapshot.as_ref().map(|s| s.logical_repository_id), Some(target_a));

    // 原路径不落文件（单 target 原路径零迁移的反向：多 target 不占原路径）。
    assert!(
        store
            .get_group_initialization(project_id, issue_id, plan_id)
            .is_err(),
        "original path must stay free for the single-target journal form"
    );

    let input_b = CreateGroupCodingAttemptInput {
        current_work_item_id: "wi_split_b".to_string(),
        branch_name: format!("aria/issues/{issue_id}/{}", target_b.0),
        target_snapshot: Some(target_snapshot(target_b)),
        ..input.clone()
    };
    let journal_b = store
        .prepare_group_initialization_with_admission_for_target(
            &input_b,
            "plan_revision_split",
            &[split_unit(target_b, "wi_split_b")],
            CodingAdmissionKind::ScAdvance,
            Some(target_b),
        )
        .expect("prepare second per-target journal");
    assert_ne!(journal_a.attempt.id, journal_b.attempt.id);

    let listed = store
        .list_group_initialization_journals_for_plan(project_id, issue_id, plan_id)
        .expect("list per-target journals");
    let mut listed_ids = listed.iter().map(|j| j.attempt.id.clone()).collect::<Vec<_>>();
    let mut expected_ids = vec![journal_a.attempt.id.clone(), journal_b.attempt.id.clone()];
    listed_ids.sort();
    expected_ids.sort();
    assert_eq!(
        listed_ids, expected_ids,
        "subdirectory journals are listed deterministically by target key"
    );

    // 幂等重放：同请求重 prepare 命中同一 journal。
    let replay_a = store
        .prepare_group_initialization_with_admission_for_target(
            &input,
            "plan_revision_split",
            &[split_unit(target_a, "wi_split_a")],
            CodingAdmissionKind::ScAdvance,
            Some(target_a),
        )
        .expect("replay per-target journal");
    assert_eq!(replay_a.attempt.id, journal_a.attempt.id);
}

#[test]
fn single_target_journal_stays_on_original_path() {
    // OQ2：单 target journal 保持原路径 {plan_id}.json（存量零迁移）。
    let (_tmp, store, fixture_ids) = logical_routing_fixture();
    let [project_id, issue_id, target_a, _target_b] = fixture_ids.as_slice() else {
        panic!("fixture ids");
    };
    let target_a = LogicalRepositoryId(uuid::Uuid::parse_str(target_a).unwrap());
    let plan_id = "work_item_plan_single_0001";
    let input = CreateGroupCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.to_string(),
        current_work_item_id: "wi_single".to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria/issues/{issue_id}"),
        worktree_path: None,
        provider_config_snapshot: provider_snapshot(),
        target_snapshot: Some(target_snapshot(target_a)),
        max_auto_rework: 2,
    };
    store
        .prepare_group_initialization_with_admission(
            &input,
            "plan_revision_single",
            &[split_unit(target_a, "wi_single")],
            CodingAdmissionKind::ScAdvance,
        )
        .expect("single-target prepare keeps the existing signature");

    let journals = store
        .list_group_initialization_journals_for_plan(project_id, issue_id, plan_id)
        .expect("list journals");
    assert_eq!(journals.len(), 1, "original path is read first");
    // get_group_initialization（原路径读取）保持可用。
    let journal = store
        .get_group_initialization(project_id, issue_id, plan_id)
        .expect("original path journal");
    assert_eq!(journal.plan_id, plan_id);
}

#[test]
fn split_journal_phase_advance_locates_subpath() {
    // mark/advance 相位函数经定位规则找到子目录 journal（不依赖原路径）。
    let (_tmp, store, fixture_ids) = logical_routing_fixture();
    let [project_id, issue_id, target_a, _target_b] = fixture_ids.as_slice() else {
        panic!("fixture ids");
    };
    let target_a = LogicalRepositoryId(uuid::Uuid::parse_str(target_a).unwrap());
    let plan_id = "work_item_plan_phase_0001";
    let input = CreateGroupCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.to_string(),
        current_work_item_id: "wi_phase".to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria/issues/{issue_id}/{}", target_a.0),
        worktree_path: None,
        provider_config_snapshot: provider_snapshot(),
        target_snapshot: Some(target_snapshot(target_a)),
        max_auto_rework: 2,
    };
    let journal = store
        .prepare_group_initialization_with_admission_for_target(
            &input,
            "plan_revision_phase",
            &[split_unit(target_a, "wi_phase")],
            CodingAdmissionKind::ScAdvance,
            Some(target_a),
        )
        .expect("prepare split journal");

    let advanced = store
        .advance_group_initialization_phase(
            &journal,
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
        )
        .expect("phase advance locates the subpath journal");
    assert_eq!(
        advanced.phase,
        crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted
    );

    let marked = store
        .mark_group_initialization_error(&advanced, "injected split failure")
        .expect("error marking locates the subpath journal");
    assert_eq!(marked.error.as_deref(), Some("injected split failure"));
}

#[test]
fn split_ensure_attempt_allows_other_target_active() {
    // REQ-MTG-02（2.2 异 target 并行解禁）：ensure_group_initialization_attempt
    // 的「另一 active attempt」检查按 target 桶细化——异 target active 不阻塞。
    let (_tmp, store, fixture_ids) = logical_routing_fixture();
    let [project_id, issue_id, target_a, target_b] = fixture_ids.as_slice() else {
        panic!("fixture ids");
    };
    let target_a = LogicalRepositoryId(uuid::Uuid::parse_str(target_a).unwrap());
    let target_b = LogicalRepositoryId(uuid::Uuid::parse_str(target_b).unwrap());
    let plan_id = "work_item_plan_ensure_0001";

    let input_a = CreateGroupCodingAttemptInput {
        project_id: project_id.clone(),
        issue_id: issue_id.clone(),
        plan_id: plan_id.to_string(),
        current_work_item_id: "wi_ensure_a".to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria/issues/{issue_id}/{}", target_a.0),
        worktree_path: None,
        provider_config_snapshot: provider_snapshot(),
        target_snapshot: Some(target_snapshot(target_a)),
        max_auto_rework: 2,
    };
    let journal_a = store
        .prepare_group_initialization_with_admission_for_target(
            &input_a,
            "plan_revision_ensure",
            &[split_unit(target_a, "wi_ensure_a")],
            CodingAdmissionKind::ScAdvance,
            Some(target_a),
        )
        .expect("prepare target A journal");
    let guard_a = store
        .acquire_work_item_attempt_creation(project_id, issue_id, "wi_ensure_a")
        .expect("creation guard A");
    let attempt_a = store
        .ensure_group_initialization_attempt(&journal_a, &guard_a)
        .expect("persist target A attempt");
    assert!(attempt_a.status.is_active());

    let input_b = CreateGroupCodingAttemptInput {
        current_work_item_id: "wi_ensure_b".to_string(),
        branch_name: format!("aria/issues/{issue_id}/{}", target_b.0),
        target_snapshot: Some(target_snapshot(target_b)),
        ..input_a.clone()
    };
    let journal_b = store
        .prepare_group_initialization_with_admission_for_target(
            &input_b,
            "plan_revision_ensure",
            &[split_unit(target_b, "wi_ensure_b")],
            CodingAdmissionKind::ScAdvance,
            Some(target_b),
        )
        .expect("prepare target B journal");
    let guard_b = store
        .acquire_work_item_attempt_creation(project_id, issue_id, "wi_ensure_b")
        .expect("creation guard B");
    let attempt_b = store
        .ensure_group_initialization_attempt(&journal_b, &guard_b)
        .expect("other-target active attempt does not block target B");
    assert_ne!(attempt_a.id, attempt_b.id);
}
