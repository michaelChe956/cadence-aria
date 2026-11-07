    // ─── C1 Task 6（REQ-WIGA-03）：租约三态判定与确认接管 ───

    fn lease_engine(
        fixture: &Fixture,
    ) -> crate::product::coding_workspace_engine::CodingWorkspaceEngine {
        let (event_tx, _event_rx) =
            tokio::sync::mpsc::channel::<crate::web::coding_ws_handler::CodingWsOutMessage>(1);
        crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
            fixture.store.clone(),
            crate::product::git_workspace_service::GitWorkspaceService::new(),
            event_tx,
        )
    }

    fn lease_worktree(fixture: &Fixture) -> crate::product::lifecycle_store::LifecycleStore {
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone());
        lifecycle
            .upsert_issue_shared_worktree(
                crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    repository_id: "repository_0001".to_string(),
                    branch_name: "main".to_string(),
                    worktree_path: std::path::PathBuf::from("/tmp/lease-worktree"),
                    base_branch: "main".to_string(),
                },
            )
            .expect("seed issue shared worktree");
        lifecycle
    }

    fn lease_worktree_and_bind_attempt(fixture: &Fixture) -> crate::product::lifecycle_store::LifecycleStore {
        let lifecycle = lease_worktree(fixture);
        let lease = lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                &format!("issue_worktree_lease_{}", fixture.attempt.id),
            )
            .expect("acquire lease");
        assert!(lease.acquired);
        lifecycle
            .bind_issue_worktree_lock_to_attempt(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                &fixture.attempt.id,
            )
            .expect("bind lease to attempt");
        lifecycle
    }

    /// 三态 fail-closed：活跃只等待（第二 acquire 不改 owner/attempt）；
    /// 终态 attempt/已释放 owner → DeadNeedsTakeover（未确认不推进）；
    /// owner 未绑定 attempt、记录缺失 → UnknownNeedsHuman（绝不抢占）。
    #[test]
    fn lease_disposition_active_dead_unknown_is_fail_closed() {
        use crate::product::models::automation::LeaseDisposition;

        // ── 活跃：owner attempt 活跃 → ActiveWait；第二 try_acquire 不改
        //    owner/attempt。
        let fixture = unbound_fixture();
        let lifecycle = lease_worktree_and_bind_attempt(&fixture);
        let engine = lease_engine(&fixture);
        let decision = engine.classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::ActiveWait);
        assert_eq!(decision.lease_id, fixture.attempt.id);
        let second = lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                "work_item_0002",
                "issue_worktree_lease_second",
            )
            .expect_err("second work item cannot steal an active lease");
        assert!(second.to_string().contains("issue_worktree_active"));
        let record = lifecycle
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("record")
            .expect("present");
        assert_eq!(record.current_lock_owner_id.as_deref(), Some(fixture.attempt.id.as_str()));
        assert_eq!(
            record.current_active_work_item_id.as_deref(),
            Some(WORK_ITEM_ID)
        );

        // ── 死亡（终态 attempt）：owner attempt Failed → DeadNeedsTakeover；
        //    判定只读，未确认不推进。
        seed_attempt_status(&fixture, CodingAttemptStatus::Failed);
        let decision = engine.classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::DeadNeedsTakeover);
        assert_eq!(decision.lease_id, fixture.attempt.id);
        let record = lifecycle
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("record")
            .expect("present");
        assert_eq!(record.current_lock_owner_id.as_deref(), Some(fixture.attempt.id.as_str()));

        // ── 死亡（已明确释放 owner）：owner/active 清空 → DeadNeedsTakeover。
        lifecycle
            .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("release by owner");
        let decision = engine.classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::DeadNeedsTakeover);

        // ── 未知（owner 是未绑定 attempt 的瞬态 lease）→ 绝不抢占。
        let fixture = unbound_fixture();
        let lifecycle = lease_worktree(&fixture);
        lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                "issue_worktree_lease_transient",
            )
            .expect("acquire transient lease");
        let decision = lease_engine(&fixture).classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::UnknownNeedsHuman);

        // ── 未知（记录缺失）：无 worktree 证据 → UnknownNeedsHuman。
        let fixture = unbound_fixture();
        let decision = lease_engine(&fixture).classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::UnknownNeedsHuman);
    }

    fn lease_enrollment_binding(fixture: &Fixture) -> crate::product::models::automation::EnrollmentBindingIdentity {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::models::automation::{
            EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
            SourceRevisionRef,
        };

        let store = IssueAutomationStore::new(fixture.paths.clone());
        store
            .compare_and_set(
                PROJECT_ID,
                ISSUE_ID,
                None,
                EnrollmentWriteCommand::Enable {
                    command_id: None,
                    selection_key: "selection_0001".to_string(),
                    source: EnrollmentSource {
                        stories: vec![SourceRevisionRef {
                            id: "story_spec_0001".to_string(),
                            version: 1,
                        }],
                        designs: vec![],
                    },
                    options: EnrollmentOptions {
                        author_provider: ProviderName::Fake,
                        reviewer_provider: ProviderName::Fake,
                        review_rounds: 1,
                        superpowers_enabled: false,
                        openspec_enabled: false,
                        plan_approval: PlanApprovalPolicy::Manual,
                        plan_options: crate::product::models::IssueWorkItemPlanOptions {
                            include_integration_tests: true,
                            include_e2e_tests: false,
                            force_frontend_backend_split: false,
                            require_execution_plan_confirm: false,
                        },
                    },
                    target: crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                        repository_id: "repository_0001".to_string(),
                    },
                },
            )
            .expect("enable enrollment");
        store
            .get(PROJECT_ID, ISSUE_ID)
            .expect("read enrollment")
            .expect("present")
            .binding_history
            .expect("binding history")
            .current
    }

    /// 确认接管：死亡 owner 在 worktree 文件锁内 CAS 清出（新 acquire 成为
    /// 新 owner）；活跃/未知不写任何文件；迟到旧 lease/旧 binding 写入
    /// IdentityMismatch 且 current owner 不变；并发确认只有一个成功；
    /// 同 command 重放返回首次 durable 结果。
    #[test]
    fn lease_takeover_confirm_writes_owner_with_cas_and_rejects_stale_writes() {
        use crate::product::json_store::ProductStoreError;
        use crate::product::models::automation::{
            LeaseTakeoverRequest, LeaseTakeoverResult, OperationState,
        };

        let fixture = unbound_fixture();
        let lifecycle = lease_worktree_and_bind_attempt(&fixture);
        seed_attempt_status(&fixture, CodingAttemptStatus::Failed);
        let binding = lease_enrollment_binding(&fixture);
        let engine = lease_engine(&fixture);

        let request = LeaseTakeoverRequest {
            command_id: "lease_takeover_cmd_0001".to_string(),
            expected_binding: binding.clone(),
            expected_lease_id: fixture.attempt.id.clone(),
            expected_attempt_id: fixture.attempt.id.clone(),
        };

        // 活跃租约拒绝接管（先复活 attempt 验证 Rejected 不写文件）。
        {
            let mut revived = fixture
                .store
                .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
                .expect("attempt");
            revived.status = CodingAttemptStatus::Running;
            fixture
                .store
                .write_coding_attempt_for_test(&revived)
                .expect("revive");
            let result = engine
                .confirm_takeover(PROJECT_ID, ISSUE_ID, &request)
                .expect("classified without error");
            assert_eq!(result.state, OperationState::Rejected);
            let record = lifecycle
                .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
                .expect("record")
                .expect("present");
            assert_eq!(
                record.current_lock_owner_id.as_deref(),
                Some(fixture.attempt.id.as_str())
            );
            seed_attempt_status(&fixture, CodingAttemptStatus::Failed);
        }

        // 合法接管：Accepted，死亡 owner 原子清出，下一次 acquire 成为
        // 新 owner（不建第二 attempt）。
        let LeaseTakeoverResult { command_id, state, lease } = engine
            .confirm_takeover(PROJECT_ID, ISSUE_ID, &request)
            .expect("takeover accepted");
        assert_eq!(command_id, "lease_takeover_cmd_0001");
        assert_eq!(state, OperationState::Accepted);
        assert!(lease
            .evidence
            .iter()
            .any(|line| line.contains("takeover_confirmed")));
        let record = lifecycle
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("record")
            .expect("present");
        assert_eq!(record.current_lock_owner_id, None);
        assert_eq!(record.current_active_work_item_id, None);
        let next = lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                "issue_worktree_lease_next",
            )
            .expect("next owner acquires after takeover");
        assert!(next.acquired);

        // 同 command 同 payload 重放：Replayed（首次 durable 结果）。
        lifecycle
            .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, "issue_worktree_lease_next")
            .expect("release next");
        let replay = engine
            .confirm_takeover(PROJECT_ID, ISSUE_ID, &request)
            .expect("replay");
        assert_eq!(replay.state, OperationState::Replayed);

        // 迟到旧 lease 写入：owner 已换代 → IdentityMismatch，current
        // owner/attempt 不变。
        let stale = lifecycle
            .takeover_issue_worktree_lock_after_dead_owner(
                PROJECT_ID,
                ISSUE_ID,
                &fixture.attempt.id,
            )
            .expect_err("stale owner write rejected");
        assert!(matches!(
            stale,
            ProductStoreError::IdentityMismatch { kind: "issue_worktree_lock_takeover", .. }
        ));

        // 过期 binding 拒绝：expected binding 漂移 → IdentityMismatch。
        let mut drifted = binding.clone();
        drifted.binding_version += 1;
        let drifted_request = LeaseTakeoverRequest {
            command_id: "lease_takeover_cmd_0002".to_string(),
            expected_binding: drifted,
            expected_lease_id: fixture.attempt.id.clone(),
            expected_attempt_id: fixture.attempt.id.clone(),
        };
        let error = engine
            .confirm_takeover(PROJECT_ID, ISSUE_ID, &drifted_request)
            .expect_err("drifted binding rejected");
        assert!(matches!(
            error,
            ProductStoreError::IdentityMismatch { kind: "enrollment_binding", .. }
        ));
    }

    // ─── C2 Task 2（REQ-CRO-02）：最小编码互斥、接管判别与 attempt 命令账本 ───

    /// 第二个 kick 撞上活跃租约（另一活跃 attempt 持有同仓租约）：admission
    /// 必须 `AlreadyRunning`（通知"已在运行／请等待"），不抢租约、不启动
    /// provider；命令账本记录首次 durable 结果，同 command 同 payload 重放
    /// 同一结论，同 command 异 payload fail-closed。
    ///
    /// REQ-ROUTE-01 两态化后租约只按 attempt 自身冻结快照的**仓维**路由
    /// 分类（issue 维单仓租约随 `Legacy` 变体删除），故夹具使用带快照的
    /// attempt + 仓维 worktree 锁。
    #[test]
    fn second_kick_on_active_lease_returns_already_running() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;
        use crate::product::models::automation::LeaseDisposition;

        let fixture = multi_repo_lease_fixture();
        // 第二个 kick 的目标：同 issue、同仓路由下另一 attempt B。B 必须在
        // A 活跃前创建（active_coding_attempt 不变式），创建后停留在
        // Created——双 kick 撞活跃租约的现场形态。
        let attempt_b = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0003".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-b".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: Some(multi_repo_snapshot(fixture.own_repo, "0001")),
                max_auto_rework: 0,
            })
            .unwrap();
        // 活跃持有者：attempt A（Running）绑定同仓 worktree 租约。
        let holder = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.own.id)
            .unwrap();
        bind_repo_lock_to_attempt(&fixture, fixture.own_repo, WORK_ITEM_ID, &holder.id);
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-kick-b-1", "digest-b-1")
            .expect("admission must classify instead of failing");
        match &decision {
            CodingRunExclusionDecision::AlreadyRunning { lease } => {
                assert_eq!(lease.disposition, LeaseDisposition::ActiveWait);
                assert_eq!(lease.lease_id, holder.id);
            }
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }
        // 租约事实未被抢占：owner 仍是 attempt A。
        let record = fixture
            .lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, fixture.own_repo)
            .expect("record")
            .expect("present");
        assert_eq!(
            record.current_lock_owner_id.as_deref(),
            Some(holder.id.as_str())
        );
        // 不启动 provider：attempt B 无任何 role run。
        assert!(
            fixture
                .store
                .list_role_runs(PROJECT_ID, ISSUE_ID, &attempt_b.id)
                .expect("role runs")
                .is_empty()
        );
        // 命令账本已记录首次 durable 结果（NeedsHuman 停等）。
        let recorded = fixture
            .store
            .find_attempt_command_result(PROJECT_ID, ISSUE_ID, &attempt_b.id, "cmd-kick-b-1")
            .expect("ledger read")
            .expect("ledger entry recorded");
        assert_eq!(
            recorded,
            crate::product::coding_attempt_store::CodingAttemptCommandRecord {
                command_id: "cmd-kick-b-1".to_string(),
                payload_digest: "digest-b-1".to_string(),
                state: crate::product::models::automation::OperationState::NeedsHuman,
                recorded_at: recorded.recorded_at.clone(),
            }
        );
        // 同 command 同 payload 重放首次 durable 结果（仍 AlreadyRunning）。
        let replay = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-kick-b-1", "digest-b-1")
            .expect("replay must not fail");
        assert!(matches!(
            replay,
            CodingRunExclusionDecision::AlreadyRunning { .. }
        ));
        // 同 command 异 payload fail-closed（请刷新）。
        let conflict = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-kick-b-1", "digest-b-OTHER")
            .expect_err("payload drift must fail closed");
        assert!(matches!(
            conflict,
            crate::product::json_store::ProductStoreError::Conflict { .. }
        ));
    }

    /// 死亡租约 → `TakeoverRequired`（未确认不继续）；接管清出后同请求
    /// 继续 → `Allowed`；瞬态 owner → `LeaseUnknown` 停等；无租约事实与
    /// 自持活跃租约是合法续跑。
    ///
    /// REQ-ROUTE-01 两态化后租约只按 attempt 自身冻结快照的**仓维**路由
    /// 分类（issue 维单仓租约随 `Legacy` 变体删除），夹具统一使用带快照的
    /// attempt + 仓维 worktree 锁。
    #[test]
    fn coding_run_admission_takeover_and_unknown_lease() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;
        use crate::product::models::automation::LeaseDisposition;

        // ── 死亡（owner attempt 终态）：TakeoverRequired，未确认不继续。
        let fixture = multi_repo_lease_fixture();
        let attempt_b = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0003".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-b".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: Some(multi_repo_snapshot(fixture.own_repo, "0001")),
                max_auto_rework: 0,
            })
            .unwrap();
        // 持有者 attempt A 先活跃绑定同仓锁，随后转终态 → 租约死亡。
        let holder = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.own.id)
            .unwrap();
        bind_repo_lock_to_attempt(&fixture, fixture.own_repo, WORK_ITEM_ID, &holder.id);
        let mut failed_holder = holder.clone();
        failed_holder.status = CodingAttemptStatus::Failed;
        fixture
            .store
            .write_coding_attempt_for_test(&failed_holder)
            .expect("seed terminal holder");
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-takeover-1", "digest-t-1")
            .expect("classify");
        match &decision {
            CodingRunExclusionDecision::TakeoverRequired { lease } => {
                assert_eq!(lease.disposition, LeaseDisposition::DeadNeedsTakeover);
            }
            other => panic!("expected TakeoverRequired, got {other:?}"),
        }
        // 用户确认接管（复用 C1 owner CAS 清出）后，同请求继续 → Allowed。
        fixture
            .lifecycle
            .release_repo_worktree_lock_by_owner(
                PROJECT_ID,
                ISSUE_ID,
                fixture.own_repo,
                &holder.id,
            )
            .expect("takeover cleared the dead owner");
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-takeover-1", "digest-t-1")
            .expect("classify after takeover");
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::Allowed(admitted)
                if admitted.id == attempt_b.id
        ));

        // ── 未知（瞬态 lease owner，未绑定 attempt）：LeaseUnknown 停等。
        let fixture = multi_repo_lease_fixture();
        let attempt_b = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0003".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-b2".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: Some(multi_repo_snapshot(fixture.own_repo, "0001")),
                max_auto_rework: 0,
            })
            .unwrap();
        fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.own.id)
            .unwrap();
        let lease = fixture
            .lifecycle
            .try_acquire_repo_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                fixture.own_repo,
                WORK_ITEM_ID,
                "repo_worktree_lease_transient",
            )
            .expect("acquire transient lease");
        assert!(lease.acquired);
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-unknown-1", "digest-u-1")
            .expect("classify");
        match &decision {
            CodingRunExclusionDecision::LeaseUnknown { lease } => {
                assert_eq!(lease.disposition, LeaseDisposition::UnknownNeedsHuman);
            }
            other => panic!("expected LeaseUnknown, got {other:?}"),
        }
        // 无租约事实（未启动过 worktree 锁）不得拦截既有链路：自持活跃租约的
        // 阶段续跑（self re-entry）照常 Allowed。
        let fixture = multi_repo_lease_fixture();
        let running = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.own.id)
            .unwrap();
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-self-1", "digest-s-1")
            .expect("no lease facts must not block admitted attempts");
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::Allowed(admitted) if admitted.id == running.id
        ));
        // 自持活跃租约（owner == 本 attempt）也是合法续跑。
        bind_repo_lock_to_attempt(&fixture, fixture.own_repo, WORK_ITEM_ID, &running.id);
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-self-2", "digest-s-2")
            .expect("self-held active lease is legitimate re-entry");
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::Allowed(admitted) if admitted.id == running.id
        ));
    }

    // ─── 多仓热修：租约分类收窄 attempt 作用域 ───
    //
    // 事故：多仓 plan 一键启动，advance 为各仓建 attempt 并预绑定
    // shared-worktree 锁；首项 runner 在 Coding 阶段 admission 被 issue 级
    // `classify_worktree_lease` 的跨仓扫描误杀（UUID 升序首个有 owner 的
    // 仓恰是兄弟仓的合法预绑定锁）。修复语义：admission 只按 attempt
    // 自身路由（target_snapshot→仓维）分类——异仓合法预绑定不构成冲突，
    // 同仓异主活跃仍 `AlreadyRunning`，证据异常 fail-closed 停等。

    struct MultiRepoLeaseFixture {
        _temp: tempfile::TempDir,
        store: CodingAttemptStore,
        lifecycle: crate::product::lifecycle_store::LifecycleStore,
        own: CodingExecutionAttempt,
        sibling: CodingExecutionAttempt,
        own_repo: LogicalRepositoryId,
        sibling_repo: LogicalRepositoryId,
    }

    fn multi_repo_snapshot(repo: LogicalRepositoryId, suffix: &str) -> AttemptTargetSnapshot {
        AttemptTargetSnapshot {
            logical_repository_id: repo,
            checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
            physical_repository_id: format!("repository_{suffix}"),
            canonical_path: PathBuf::from(format!("/tmp/repository_{suffix}")),
            git_dir_identity: format!("sha256:{suffix}"),
            revision: Some("abcdef".to_string()),
            policy_digest: "policy_digest".to_string(),
            membership_revision: 1,
            captured_at: "2026-10-14T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }
    }

    /// 事故现场最小化：同 issue 双 logical 仓（own=busi、sibling=api）。
    /// `sibling_repo` UUID 恒小于 `own_repo`——旧 issue 级 classify 按 UUID
    /// 升序扫仓取首个有 owner 的记录，必然先撞 sibling 的预绑定锁。两仓
    /// worktree 记录均已建（advance 语义），锁绑定由各用例自行完成。
    fn multi_repo_lease_fixture() -> MultiRepoLeaseFixture {
        use crate::product::lifecycle_store::{LifecycleStore, UpsertRepoSharedWorktreeInput};

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        // REQ-ROUTE-01：routing 只从 issue 的持久归属解析；issue 记录缺失时
        // `load_for_issue` 按 T1.2 契约返回 `NotFound`（不是静默 FailClosed），
        // 故夹具必须落一条归属 legacy 别名 LC 的 issue——该别名子树与项目级
        // legacy 布局字节等价（`store::lc_scope_root` 契约）。
        crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id:
                    crate::product::logical_codebase::legacy_logical_codebase_id(PROJECT_ID),
                title: "multi repo lease fixture issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        let own_repo = LogicalRepositoryId(Uuid::from_u128(2));
        let sibling_repo = LogicalRepositoryId(Uuid::from_u128(1));
        let authority = LogicalCodebaseStore::new(paths.clone());
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            temp.path().join("aggregate-root"),
            vec![own_repo, sibling_repo],
        );
        authority.save_manifest(PROJECT_ID, &manifest).unwrap();
        for (repo, physical, ordinal) in [
            (own_repo, "repository_0001", 1u32),
            (sibling_repo, "repository_0002", 2),
        ] {
            let repository_path = temp.path().join(physical);
            authority
                .save_member(
                    PROJECT_ID,
                    &CodebaseMemberRecord {
                        logical_repository_id: repo,
                        physical_repository_id: physical.to_string(),
                        alias: physical.to_string(),
                        role: "repository".to_string(),
                        ordinal,
                        source_identity: RepositorySourceIdentity::from_git_parts(
                            &repository_path,
                            repository_path.join(".git"),
                            None,
                        ),
                        repo_type: RepositoryType::Unknown,
                        tech_stack: Vec::new(),
                        owner: None,
                        tags: Vec::new(),
                        default_ref: None,
                        checkout_ids: Vec::new(),
                        status: MemberStatus::Active,
                        created_at: "2026-10-14T00:00:00Z".to_string(),
                        updated_at: "2026-10-14T00:00:00Z".to_string(),
                    },
                )
                .unwrap();
        }
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                PROJECT_ID,
                ISSUE_ID,
                vec![own_repo, sibling_repo],
                Vec::new(),
                vec![own_repo, sibling_repo],
                None,
            ))
            .unwrap();
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .unwrap();
        let store = CodingAttemptStore::new(paths.clone());
        let own = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-own".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: Some(multi_repo_snapshot(own_repo, "0001")),
                max_auto_rework: 0,
            })
            .unwrap();
        let sibling = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-sibling".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: Some(multi_repo_snapshot(sibling_repo, "0002")),
                max_auto_rework: 0,
            })
            .unwrap();
        let lifecycle = LifecycleStore::new(paths.clone());
        for (repo, name) in [(own_repo, "own"), (sibling_repo, "sibling")] {
            std::fs::create_dir_all(temp.path().join(name)).unwrap();
            lifecycle
                .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    repository_id: repo,
                    branch_name: "main".to_string(),
                    worktree_path: temp.path().join(name),
                    base_branch: "main".to_string(),
                })
                .unwrap();
        }
        MultiRepoLeaseFixture {
            _temp: temp,
            store,
            lifecycle,
            own,
            sibling,
            own_repo,
            sibling_repo,
        }
    }

    /// 仓维锁获取并绑定到 attempt（advance 预绑定语义）。
    fn bind_repo_lock_to_attempt(
        fixture: &MultiRepoLeaseFixture,
        repo: LogicalRepositoryId,
        work_item_id: &str,
        attempt_id: &str,
    ) {
        let lease = fixture
            .lifecycle
            .try_acquire_repo_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                repo,
                work_item_id,
                &format!("repo_worktree_lease_{attempt_id}"),
            )
            .unwrap();
        assert!(lease.acquired, "seed repo lease must acquire");
        fixture
            .lifecycle
            .bind_repo_worktree_lock_to_attempt(
                PROJECT_ID,
                ISSUE_ID,
                repo,
                work_item_id,
                attempt_id,
            )
            .unwrap();
    }

    /// 事故回归：兄弟仓预绑定锁（异仓、异 attempt、Created 亦是活跃态）
    /// 不得拦本 attempt 的 admission——首项 runner 必须放行进入 provider；
    /// 兄弟仓锁事实原样保留；同 command 重放幂等放行。
    #[test]
    fn multi_repo_admission_ignores_sibling_repo_prebound_lock() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;

        let fixture = multi_repo_lease_fixture();
        // 事故形态：两仓锁均在场——own 仓自持（首项合法自绑），sibling
        // 仓预绑定到尚未启动的兄弟 attempt。
        bind_repo_lock_to_attempt(&fixture, fixture.own_repo, WORK_ITEM_ID, &fixture.own.id);
        bind_repo_lock_to_attempt(
            &fixture,
            fixture.sibling_repo,
            "work_item_0002",
            &fixture.sibling.id,
        );
        let running = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.own.id)
            .unwrap();

        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-multi-own-1", "digest-own-1")
            .expect("sibling repo prebound lock must not block own admission");
        assert!(matches!(
            &decision,
            CodingRunExclusionDecision::Allowed(admitted) if admitted.id == running.id
        ));
        // 兄弟仓锁事实不被触碰（不抢、不改）。
        let sibling_record = fixture
            .lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, fixture.sibling_repo)
            .unwrap()
            .expect("sibling repo worktree record");
        assert_eq!(
            sibling_record.current_lock_owner_id.as_deref(),
            Some(fixture.sibling.id.as_str())
        );
        // 同 command 同 payload 重放仍放行（账本 Accepted 重放）。
        let replay = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-multi-own-1", "digest-own-1")
            .unwrap();
        assert!(matches!(
            &replay,
            CodingRunExclusionDecision::Allowed(admitted) if admitted.id == running.id
        ));
    }

    /// 同仓真冲突语义保留：同仓异主活跃 → `AlreadyRunning`（命令账本
    /// NeedsHuman 恰一条；同 command 重放同一结论；异 payload
    /// fail-closed；锁事实不被抢占）。
    #[test]
    fn multi_repo_admission_same_repo_foreign_active_holder_is_already_running() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;
        use crate::product::models::automation::{LeaseDisposition, OperationState};

        let fixture = multi_repo_lease_fixture();
        // 同仓第三方 attempt C（活跃）持有本 attempt 路由的 own 仓锁。
        let holder = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0003".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-holder".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: Some(multi_repo_snapshot(fixture.own_repo, "0001")),
                max_auto_rework: 0,
            })
            .unwrap();
        let holder = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &holder.id)
            .unwrap();
        bind_repo_lock_to_attempt(&fixture, fixture.own_repo, "work_item_0003", &holder.id);
        let running = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.own.id)
            .unwrap();

        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-multi-conflict-1", "digest-c-1")
            .expect("admission must classify instead of failing");
        match &decision {
            CodingRunExclusionDecision::AlreadyRunning { lease } => {
                assert_eq!(lease.disposition, LeaseDisposition::ActiveWait);
                assert_eq!(lease.lease_id, holder.id);
            }
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }
        // 命令账本：首次 durable 结果恰一条 NeedsHuman。
        let records = fixture
            .store
            .list_attempt_command_records(PROJECT_ID, ISSUE_ID, &running.id)
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].state, OperationState::NeedsHuman);
        // 同 command 同 payload 重放同一结论；异 payload fail-closed。
        let replay = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-multi-conflict-1", "digest-c-1")
            .unwrap();
        assert!(matches!(
            replay,
            CodingRunExclusionDecision::AlreadyRunning { .. }
        ));
        let conflict = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-multi-conflict-1", "digest-OTHER")
            .unwrap_err();
        assert!(matches!(
            conflict,
            crate::product::json_store::ProductStoreError::Conflict { .. }
        ));
        // 锁事实未被抢占：own 仓 owner 仍是持有者 C。
        let own_record = fixture
            .lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, fixture.own_repo)
            .unwrap()
            .expect("own repo worktree record");
        assert_eq!(
            own_record.current_lock_owner_id.as_deref(),
            Some(holder.id.as_str())
        );
    }

    /// 同仓异主死亡/未知：死亡他人 → `TakeoverRequired`（确认接管面），
    /// 不可解析 owner（瞬态 lease 残留）→ `LeaseUnknown` 停等——两者都
    /// 不抢占锁事实。
    #[test]
    fn multi_repo_admission_same_repo_dead_or_unknown_holder_never_preempts() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;

        // a) 死亡他人：owner 是终态 attempt。
        let fixture = multi_repo_lease_fixture();
        let mut dead_holder = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0004".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-dead".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: Some(multi_repo_snapshot(fixture.own_repo, "0001")),
                max_auto_rework: 0,
            })
            .unwrap();
        dead_holder.status = CodingAttemptStatus::Aborted;
        fixture
            .store
            .write_coding_attempt_for_test(&dead_holder)
            .unwrap();
        bind_repo_lock_to_attempt(&fixture, fixture.own_repo, "work_item_0004", &dead_holder.id);
        let running = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.own.id)
            .unwrap();
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-multi-dead-1", "digest-d-1")
            .unwrap();
        match &decision {
            CodingRunExclusionDecision::TakeoverRequired { lease } => {
                assert_eq!(lease.lease_id, dead_holder.id);
            }
            other => panic!("expected TakeoverRequired, got {other:?}"),
        }

        // b) 未知 owner：瞬态 lease 残留（acquire 后未 bind）。
        let fixture = multi_repo_lease_fixture();
        let lease = fixture
            .lifecycle
            .try_acquire_repo_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                fixture.own_repo,
                WORK_ITEM_ID,
                "repo_worktree_lease_transient_seed",
            )
            .unwrap();
        assert!(lease.acquired);
        let running = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.own.id)
            .unwrap();
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-multi-unknown-1", "digest-u-1")
            .unwrap();
        match &decision {
            CodingRunExclusionDecision::LeaseUnknown { lease } => {
                assert_eq!(lease.lease_id, "repo_worktree_lease_transient_seed");
            }
            other => panic!("expected LeaseUnknown, got {other:?}"),
        }
    }

    /// fail-closed 收紧面：无 snapshot 的 Logical 路由 attempt、仓维路由
    /// 下 legacy issue 维记录混在——一律 `LeaseUnknown` 停等，绝不从异
    /// 质记录推导租约事实。
    #[test]
    fn multi_repo_scoped_classification_fails_closed_on_missing_snapshot_or_mixed_records() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;
        use crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput;
        use crate::product::models::automation::LeaseDisposition;

        let fixture = multi_repo_lease_fixture();
        // a) 无 snapshot + Logical 路由 → target_snapshot_missing_for_logical。
        let snapshotless = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0005".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-snapshotless".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        let decision = fixture
            .store
            .classify_worktree_lease_for_attempt(&snapshotless);
        assert_eq!(decision.disposition, LeaseDisposition::UnknownNeedsHuman);
        assert!(
            decision
                .evidence
                .iter()
                .any(|fact| fact.contains("target_snapshot_missing_for_logical")),
            "evidence: {:?}",
            decision.evidence
        );
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&snapshotless, "cmd-multi-fc-1", "digest-f-1")
            .unwrap();
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::LeaseUnknown { .. }
        ));

        // b) snapshot attempt + legacy issue 维记录混在 → legacy present。
        let legacy = fixture
            .lifecycle
            .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: "repository_0001".to_string(),
                branch_name: "main".to_string(),
                worktree_path: std::path::PathBuf::from("/tmp/legacy-worktree"),
                base_branch: "main".to_string(),
            })
            .unwrap();
        drop(legacy);
        let decision = fixture
            .store
            .classify_worktree_lease_for_attempt(&fixture.own);
        assert_eq!(decision.disposition, LeaseDisposition::UnknownNeedsHuman);
        assert!(
            decision
                .evidence
                .iter()
                .any(|fact| fact.contains("legacy_shared_worktree_present")),
            "evidence: {:?}",
            decision.evidence
        );
    }
