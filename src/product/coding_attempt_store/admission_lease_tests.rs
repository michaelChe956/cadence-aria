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
        let fixture = legacy_fixture();
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
        let fixture = legacy_fixture();
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
        let fixture = legacy_fixture();
        let decision = lease_engine(&fixture).classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::UnknownNeedsHuman);
    }

    fn lease_enrollment_binding(fixture: &Fixture) -> crate::product::models::automation::EnrollmentBindingIdentity {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::models::automation::{
            EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, SourceRevisionRef,
        };

        let store = IssueAutomationStore::new(fixture.paths.clone());
        store
            .compare_and_set(
                PROJECT_ID,
                ISSUE_ID,
                None,
                EnrollmentWriteCommand::Enable {
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

        let fixture = legacy_fixture();
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

    /// 第二个 kick 撞上活跃租约（另一活跃 attempt 持有）：admission 必须
    /// `AlreadyRunning`（通知"已在运行／请等待"），不抢租约、不启动 provider；
    /// 命令账本记录首次 durable 结果，同 command 同 payload 重放同一结论，
    /// 同 command 异 payload fail-closed。
    #[test]
    fn second_kick_on_active_lease_returns_already_running() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;
        use crate::product::models::automation::LeaseDisposition;

        let fixture = legacy_fixture();
        // 第二个 kick 的目标：同 issue 下另一 attempt B。B 必须在 A 活跃前
        // 创建（active_coding_attempt 不变式），创建后停留在 Created——双 kick
        // 撞活跃租约的现场形态。
        let attempt_b = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-b".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        // 活跃持有者：attempt A（Running）绑定 issue worktree 租约。
        seed_attempt_status(&fixture, CodingAttemptStatus::Running);
        let lifecycle = lease_worktree_and_bind_attempt(&fixture);
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-kick-b-1", "digest-b-1")
            .expect("admission must classify instead of failing");
        match &decision {
            CodingRunExclusionDecision::AlreadyRunning { lease } => {
                assert_eq!(lease.disposition, LeaseDisposition::ActiveWait);
                assert_eq!(lease.lease_id, fixture.attempt.id);
            }
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }
        // 租约事实未被抢占：owner 仍是 attempt A。
        let record = lifecycle
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("record")
            .expect("present");
        assert_eq!(
            record.current_lock_owner_id.as_deref(),
            Some(fixture.attempt.id.as_str())
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
    /// 继续 → `Allowed`；证据缺失/瞬态 owner → `LeaseUnknown` 停等。
    #[test]
    fn coding_run_admission_takeover_and_unknown_lease() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;
        use crate::product::models::automation::LeaseDisposition;

        // ── 死亡（owner attempt 终态）：TakeoverRequired，未确认不继续。
        let fixture = legacy_fixture();
        let attempt_b = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-b".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        seed_attempt_status(&fixture, CodingAttemptStatus::Running);
        let lifecycle = lease_worktree_and_bind_attempt(&fixture);
        // 持有者 attempt A 转终态 → 租约死亡。
        seed_attempt_status(&fixture, CodingAttemptStatus::Failed);
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
        lifecycle
            .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("takeover cleared the dead owner");
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-takeover-1", "digest-t-1")
            .expect("classify after takeover");
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::Allowed(ref admitted)
                if admitted.id == attempt_b.id
        ));

        // ── 未知（瞬态 lease owner，未绑定 attempt）：LeaseUnknown 停等。
        let fixture = legacy_fixture();
        let attempt_b = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-b2".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        seed_attempt_status(&fixture, CodingAttemptStatus::Running);
        let lifecycle = lease_worktree(&fixture);
        lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                "issue_worktree_lease_transient",
            )
            .expect("acquire transient lease");
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
        let fixture = legacy_fixture();
        let running = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .unwrap();
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-self-1", "digest-s-1")
            .expect("no lease facts must not block legacy attempts");
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::Allowed(ref admitted) if admitted.id == running.id
        ));
        // 自持活跃租约（owner == 本 attempt）也是合法续跑。
        lease_worktree_and_bind_attempt(&fixture);
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-self-2", "digest-s-2")
            .expect("self-held active lease is legitimate re-entry");
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::Allowed(ref admitted) if admitted.id == running.id
        ));
    }
