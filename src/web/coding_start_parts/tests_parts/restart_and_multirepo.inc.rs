/// C5 Task 5：单仓互证通过——attempt 无 target_snapshot 且 issue 当前
/// authority 载体解析为 enrollment 同一物理仓 → typed 首启可发生
///（provider 真实入口被 probe 暂停，不冒充启动）。
#[tokio::test]
async fn single_repository_start_coding_passes_mutual_verification() {
    let fixture = ready_single_repository_attempt_fixture().await;
    let attempt = fixture.attempt();
    let (probe, _entry, _hold) = paused_start_probe();
    let outcome = crate::web::coding_start::start_coding_once_with_probe(
        &fixture.state,
        &attempt.project_id,
        &attempt.issue_id,
        StartCodingCommand {
            attempt_id: attempt.id.clone(),
            command_id: "single-repo-start-ok".into(),
            origin: fixture.auto_origin(),
        },
        probe,
    )
    .await
    .expect("single-repository mutual verification passes");
    assert!(
        matches!(outcome, StartCodingOutcome::Started { .. }),
        "unexpected outcome: {outcome:?}"
    );
    assert_eq!(
        fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
        1
    );
}

/// C2 Task 3（REQ-CRO-03）：abort 置 retired 后，显式 restart 在同进程内
/// 清退役标记→重开 admission→spawn 新 runner，无需重启服务；同 command
/// 同 payload 重放首次 durable 结果（Replayed，不二次 spawn）；错版本
/// Rejected（不改 attempt、不启动 provider）。断言全部同步执行，被
/// spawn 的 runner 任务在单线程 runtime 下于测试结束前不推进。
#[tokio::test]
async fn restart_after_abort_readmits_attempt_in_same_process() {
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CodingAttemptStore;
    use crate::product::coding_models::CodingAttemptStatus;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::models::automation::OperationState;
    use crate::web::coding_start::{RestartCodingAttemptRequest, restart_coding_attempt};

    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let state = crate::web::state::WebAppState::new(
        root.clone(),
        crate::web::runtime::WebRuntime::new_fake(root.clone()),
    );
    let paths = ProductAppPaths::new(root.join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "restart issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let attempt = store
        .create_attempt(
            crate::product::coding_attempt_store::CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/restart-attempt".to_string(),
                worktree_path: None,
                provider_config_snapshot: crate::web::workspace_ws_types::ProviderConfigSnapshot {
                    author: crate::product::models::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 0,
            },
        )
        .expect("attempt");
    let running = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running attempt");

    // abort：registry retired ＋ durable 终态（Aborted）。
    let key = CodingAttemptRunKey::from_attempt(&running);
    state.coding_runs.abort_attempt(&key).await;
    let mut aborted = running.clone();
    aborted.status = CodingAttemptStatus::Aborted;
    // 真实 abort 链（transition_to_terminal）清 admission ticket（admission.rs:223）。
    aborted.admission_ticket_consumed_at = None;
    store
        .write_coding_attempt_for_test(&aborted)
        .expect("aborted");

    let request = RestartCodingAttemptRequest {
        command_id: "cmd-restart-1".to_string(),
        attempt_id: attempt.id.clone(),
        expected_attempt_version: aborted.version,
    };
    let result = restart_coding_attempt(
        &state,
        &attempt.project_id,
        &attempt.issue_id,
        request.clone(),
    )
    .await
    .expect("restart service");
    assert_eq!(result.state, OperationState::Accepted);
    assert_eq!(result.attempt_id, attempt.id);

    // durable 重开为 Running；退役围栏已清——新 runner 已在 registry 注册
    // （spawn 成功即 retired 不再拦截 insert_cancellable）。
    let current = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(current.status, CodingAttemptStatus::Running);
    assert_eq!(
        state.coding_runs.runner_count(&key),
        1,
        "new runner must be registered in the same process"
    );

    // 同 command 同 payload：重放首次 durable 结果，不改变状态。
    let replay = restart_coding_attempt(&state, &attempt.project_id, &attempt.issue_id, request)
        .await
        .expect("replay");
    assert_eq!(replay.state, OperationState::Replayed);

    // 错版本（错对象）：Rejected，不改变当前 attempt。
    let wrong = restart_coding_attempt(
        &state,
        &attempt.project_id,
        &attempt.issue_id,
        RestartCodingAttemptRequest {
            command_id: "cmd-restart-2".to_string(),
            attempt_id: attempt.id.clone(),
            expected_attempt_version: current.version + 999,
        },
    )
    .await
    .expect("wrong version maps to Rejected result");
    assert_eq!(wrong.state, OperationState::Rejected);
    let after = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(after.status, CodingAttemptStatus::Running);
}

/// 多仓热修：restart 租约预检按 attempt 路由收窄——兄弟仓预绑定锁
///（异仓、活跃异主）不得把本 attempt 的显式 restart 误判为
/// `coding_run_already_running` 停等；本仓自持终态残留按 G5 释放+重绑。
#[tokio::test]
async fn restart_multi_repo_attempt_passes_precheck_despite_sibling_prebound_lock() {
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CodingAttemptStore;
    use crate::product::coding_models::{AttemptTargetSnapshot, CodingAttemptStatus};
    use crate::product::lifecycle_store::{LifecycleStore, UpsertRepoSharedWorktreeInput};
    use crate::product::logical_codebase::{
        IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId, RepositoryCheckoutId,
    };
    use crate::product::models::automation::OperationState;
    use crate::web::coding_start::{RestartCodingAttemptRequest, restart_coding_attempt};
    use uuid::Uuid;

    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let state = crate::web::state::WebAppState::new(
        root.clone(),
        crate::web::runtime::WebRuntime::new_fake(root.clone()),
    );
    let paths = ProductAppPaths::new(root.join(".aria"));
    // sibling UUID 恒小于 own：旧 issue 级 classify 按 UUID 升序必然先取
    // 兄弟仓预绑定锁（事故形态）。
    let own_repo = LogicalRepositoryId(Uuid::from_u128(2));
    let sibling_repo = LogicalRepositoryId(Uuid::from_u128(1));
    // 真实工件链（durable 重开 admission 重验快照身份与 policy）：
    // project → manifest → member/checkout → repos.json → policy。
    crate::product::project_store::ProjectStore::new(paths.clone())
        .create(crate::product::project_store::CreateProjectInput {
            name: "project".to_string(),
            description: None,
        })
        .expect("project");
    let authority = LogicalCodebaseStore::new(paths.clone());
    let manifest = LogicalCodebaseManifest::new(
        "project_0001",
        root.join("aggregate-root"),
        vec![own_repo, sibling_repo],
    );
    authority
        .save_manifest("project_0001", &manifest)
        .expect("manifest");
    let mut members = Vec::new();
    let mut snapshots = std::collections::HashMap::new();
    for (ordinal, (repo, physical)) in [
        (own_repo, "repository_0001"),
        (sibling_repo, "repository_0002"),
    ]
    .into_iter()
    .enumerate()
    {
        let repository_path = root.join(physical);
        let source_identity =
            crate::product::logical_codebase::RepositorySourceIdentity::from_git_parts(
                &repository_path,
                repository_path.join(".git"),
                None,
            );
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        authority
            .save_member(
                "project_0001",
                &crate::product::logical_codebase::CodebaseMemberRecord {
                    logical_repository_id: repo,
                    physical_repository_id: physical.to_string(),
                    alias: physical.to_string(),
                    role: "repository".to_string(),
                    ordinal: ordinal as u32 + 1,
                    source_identity: source_identity.clone(),
                    repo_type: crate::product::logical_codebase::RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: crate::product::logical_codebase::MemberStatus::Active,
                    created_at: "2026-10-14T00:00:00Z".to_string(),
                    updated_at: "2026-10-14T00:00:00Z".to_string(),
                },
            )
            .expect("member");
        authority
            .save_checkout(
                "project_0001",
                &crate::product::logical_codebase::RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: repo,
                    physical_repository_id: physical.to_string(),
                    kind: crate::product::logical_codebase::CheckoutKind::Main,
                    canonical_path: repository_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    availability: crate::product::logical_codebase::CheckoutAvailability::Available,
                    observed_at: "2026-10-14T00:00:00Z".to_string(),
                    created_at: "2026-10-14T00:00:00Z".to_string(),
                    updated_at: "2026-10-14T00:00:00Z".to_string(),
                },
            )
            .expect("checkout");
        members.push(crate::product::models::RepositoryRecord {
            id: physical.to_string(),
            project_id: "project_0001".to_string(),
            name: physical.to_string(),
            path: repository_path.clone(),
            repo_hash: format!("sha256:{physical}"),
            runtime_root: std::path::PathBuf::from("runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-10-14T00:00:00Z".to_string(),
            updated_at: "2026-10-14T00:00:00Z".to_string(),
            logical_repository_id: Some(repo),
            primary_checkout_id: Some(checkout_id),
            identity_schema_version: 1,
        });
        snapshots.insert(
            repo,
            (
                checkout_id,
                repository_path,
                physical.to_string(),
                source_identity.git_dir_identity(),
            ),
        );
    }
    crate::product::json_store::write_json(
        &paths.project_root("project_0001").join("repos.json"),
        &members,
    )
    .expect("repos.json");
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![own_repo, sibling_repo],
            Vec::new(),
            vec![own_repo, sibling_repo],
            None,
        ))
        .expect("selection");
    let policy_store =
        crate::product::logical_codebase::AggregatePolicyArtifactStore::new(paths.clone());
    policy_store
        .ensure_bootstrap(&manifest)
        .expect("policy bootstrap");
    let policy_digest = policy_store
        .get("project_0001")
        .expect("policy")
        .expect("policy present")
        .digest;
    let snapshot = |repo: LogicalRepositoryId| AttemptTargetSnapshot {
        logical_repository_id: repo,
        checkout_id: snapshots[&repo].0,
        physical_repository_id: snapshots[&repo].2.clone(),
        canonical_path: snapshots[&repo].1.clone(),
        git_dir_identity: snapshots[&repo].3.clone(),
        revision: Some("abcdef".to_string()),
        policy_digest: policy_digest.clone(),
        membership_revision: manifest.membership_revision,
        captured_at: "2026-10-14T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    };
    let lifecycle = LifecycleStore::new(paths.clone());
    for (repo, name) in [
        (own_repo, "own-worktree"),
        (sibling_repo, "sibling-worktree"),
    ] {
        let worktree = root.join(name);
        std::fs::create_dir_all(&worktree).expect("worktree dir");
        lifecycle
            .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repo,
                branch_name: "main".to_string(),
                worktree_path: worktree,
                base_branch: "main".to_string(),
            })
            .expect("repo shared worktree");
    }
    let store = CodingAttemptStore::new(paths.clone());
    let provider = crate::web::workspace_ws_types::ProviderConfigSnapshot {
        author: crate::product::models::ProviderName::Fake,
        reviewer: None,
        review_rounds: 0,
        permission_modes: Default::default(),
    };
    let own = store
        .create_attempt(
            crate::product::coding_attempt_store::CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/restart-multi-own".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider.clone(),
                target_snapshot: Some(snapshot(own_repo)),
                max_auto_rework: 0,
            },
        )
        .expect("own attempt");
    let sibling = store
        .create_attempt(
            crate::product::coding_attempt_store::CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/restart-multi-sibling".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider,
                target_snapshot: Some(snapshot(sibling_repo)),
                max_auto_rework: 0,
            },
        )
        .expect("sibling attempt");
    // 本仓自持（后随 abort 成为终态残留）；兄弟仓预绑定到未启动的
    // sibling attempt（Created 亦是活跃态）。
    for (repo, work_item_id, attempt_id) in [
        (own_repo, "work_item_0001", own.id.clone()),
        (sibling_repo, "work_item_0002", sibling.id.clone()),
    ] {
        let lease = lifecycle
            .try_acquire_repo_worktree_lock(
                "project_0001",
                "issue_0001",
                repo,
                work_item_id,
                &format!("repo_worktree_lease_{attempt_id}"),
            )
            .expect("acquire repo lease");
        assert!(lease.acquired);
        lifecycle
            .bind_repo_worktree_lock_to_attempt(
                "project_0001",
                "issue_0001",
                repo,
                work_item_id,
                &attempt_id,
            )
            .expect("bind repo lease");
    }
    let mut aborted = own.clone();
    aborted.status = CodingAttemptStatus::Aborted;
    aborted.admission_ticket_consumed_at = None;
    store
        .write_coding_attempt_for_test(&aborted)
        .expect("aborted own");

    let result = restart_coding_attempt(
        &state,
        "project_0001",
        "issue_0001",
        RestartCodingAttemptRequest {
            command_id: "cmd-restart-multi-1".to_string(),
            attempt_id: own.id.clone(),
            expected_attempt_version: aborted.version,
        },
    )
    .await
    .expect("restart service");
    assert_eq!(
        result.state,
        OperationState::Accepted,
        "sibling repo prebound lock must not stall restart: {:?}",
        result.reason
    );
    // 兄弟仓锁事实原样保留；本仓重绑回重启的 attempt。
    let sibling_record = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", sibling_repo)
        .expect("sibling record")
        .expect("sibling record present");
    assert_eq!(
        sibling_record.current_lock_owner_id.as_deref(),
        Some(sibling.id.as_str())
    );
    let own_record = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", own_repo)
        .expect("own record")
        .expect("own record present");
    assert_eq!(
        own_record.current_lock_owner_id.as_deref(),
        Some(own.id.as_str())
    );
}

/// lcg_t13(计划 Task 13 Step 1):plan 到 durable Ready 后不得自动为
/// 其他 target 派工——显式 StartCoding 是唯一 provider 启动入口。
/// ready 时不发任何命令:attempt 集合不增(other target 零新 attempt)、
/// runner 零注册(provider 启动计数 0)、attempt 保持 Ready;随后一条
/// 显式 Manual StartCoding 即可放行(probe 暂停 provider 入口,不冒充
/// 启动)。
#[tokio::test]
async fn lcg_t13_ready_plan_does_not_start_other_target_without_manual_command() {
    let fixture = ready_single_repository_attempt_fixture().await;
    let attempt = fixture.attempt();
    let store = fixture.store();
    let before_count = store
        .list_attempts_for_issue(&attempt.project_id, &attempt.issue_id)
        .expect("list attempts before settle")
        .len();
    let status_before = attempt.status.clone();

    // ready 后静置:无后台自动派工/自动 provider 启动。
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let after_count = store
        .list_attempts_for_issue(&attempt.project_id, &attempt.issue_id)
        .expect("list attempts after settle")
        .len();
    assert_eq!(
        after_count, before_count,
        "a ready plan must not auto-dispatch attempts for other targets"
    );
    assert_eq!(
        fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
        0,
        "provider start count before an explicit start_coding command must be 0"
    );
    let settled = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("settled attempt");
    assert_eq!(
        settled.status, status_before,
        "no background worker may move a ready attempt without a command"
    );

    // 显式 Manual StartCoding 仍是唯一放行入口(probe 暂停 provider 真实
    // 入口):Started 且恰一 runner——证明上方零启动不是门坏,而是没有
    // 命令就没有派工。
    let (probe, _entry, _hold) = paused_start_probe();
    let outcome = crate::web::coding_start::start_coding_once_with_probe(
        &fixture.state,
        &attempt.project_id,
        &attempt.issue_id,
        StartCodingCommand {
            attempt_id: attempt.id.clone(),
            command_id: "lcg-t13-manual-1".into(),
            origin: CodingStartOrigin::Manual,
        },
        probe,
    )
    .await
    .expect("explicit manual start coding must be admitted at Ready");
    assert!(
        matches!(outcome, StartCodingOutcome::Started { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
        1
    );
}

/// k3 波2 F4（选型 A 读侧回退）：集合绑定的成员解析双形态读——
/// ①legacy 旧根现场（enrollment 冻结 manifest UUID，成员驻留 project 级
/// 旧根）：uuid 子树失败回退 issue 作用域旧根解析（修复前红：
/// logical_repository_manifest not found）；②per-LC uuid 子树现场直接
/// 命中；③成员不存在仍 fail-closed。
#[test]
fn set_enrollment_member_resolution_reads_legacy_root_fallback() {
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
        RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use crate::web::handlers::automation_enrollment_test_support::{
        ISSUE_ID as PROJECT_ISSUE_ID, PROJECT_ID, seed_fixture,
    };

    let resolve = |paths: &crate::product::app_paths::ProductAppPaths,
                   lc: &str,
                   member: LogicalRepositoryId| {
        crate::web::coding_start::resolve_set_enrollment_member(
            paths,
            PROJECT_ID,
            PROJECT_ISSUE_ID,
            lc,
            member,
        )
    };

    // ① legacy 旧根现场：seed_fixture 播种 project 级旧根 manifest/成员/
    //checkout/repos.json；enrollment 冻结任意 manifest UUID 形态。
    let fixture = seed_fixture(2, true);
    let enrolled_lc = uuid::Uuid::new_v4().to_string();
    let member_one = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let (member, checkout, repository) = resolve(&fixture.paths, &enrolled_lc, member_one)
        .expect("legacy root fallback must resolve set enrollment member");
    assert_eq!(member.logical_repository_id, member_one);
    assert_eq!(checkout.kind, CheckoutKind::Main);
    assert_eq!(repository.logical_repository_id, Some(member_one));

    // ② per-LC uuid 子树现场：for_lc 权威子树直接命中（无回退参与）。
    let subtree_root = tempfile::tempdir().unwrap();
    let subtree_paths =
        crate::product::app_paths::ProductAppPaths::new(subtree_root.path().join(".aria"));
    let lc_uuid = uuid::Uuid::new_v4().to_string();
    let authority = LogicalCodebaseStore::for_lc(subtree_paths.clone(), &lc_uuid);
    let manifest = LogicalCodebaseManifest::new(
        PROJECT_ID,
        subtree_root.path().to_path_buf(),
        vec![member_one],
    );
    authority.save_manifest(PROJECT_ID, &manifest).unwrap();
    let now = "2026-10-16T00:00:00Z".to_string();
    let checkout_path = subtree_root.path().join("checkout-f4");
    std::fs::create_dir_all(&checkout_path).unwrap();
    let source_identity =
        RepositorySourceIdentity::from_git_parts(&checkout_path, checkout_path.join(".git"), None);
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    authority
        .save_member(
            PROJECT_ID,
            &CodebaseMemberRecord {
                logical_repository_id: member_one,
                physical_repository_id: "physical-f4".to_string(),
                alias: "repo-f4".to_string(),
                role: "repository".to_string(),
                ordinal: 1,
                source_identity: source_identity.clone(),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .unwrap();
    authority
        .save_checkout(
            PROJECT_ID,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_one,
                physical_repository_id: "physical-f4".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: checkout_path,
                checkout_path_hash: "sha256:checkout-f4".to_string(),
                git_dir_identity: source_identity.git_dir_identity(),
                revision: Some("abcdef".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            },
        )
        .unwrap();
    let (subtree_member, _, subtree_repository) = resolve(&subtree_paths, &lc_uuid, member_one)
        .expect("uuid subtree site must resolve directly");
    assert_eq!(subtree_member.logical_repository_id, member_one);
    assert_eq!(subtree_repository.id, "physical-f4");

    // ③ 成员不存在：两路皆失败 → 原始（uuid 子树）错误 fail-closed。
    let missing = LogicalRepositoryId(uuid::Uuid::from_u128(4242));
    assert!(
        resolve(&fixture.paths, &enrolled_lc, missing).is_err(),
        "nonexistent member must fail closed"
    );
    assert!(resolve(&subtree_paths, &lc_uuid, missing).is_err(),);
}
