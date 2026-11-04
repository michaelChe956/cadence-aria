    /// 无 LC 归属的 attempt（无 project/逻辑代码库/selection，且无
    /// `target_snapshot`）。REQ-ROUTE-01 两态化后这类输入一律
    /// `FailClosed(TargetMissing)` ⇒ admission 必须 fail-closed，绝不再走
    /// 已删除的单仓 `legacy_snapshot_digest` 准入路径。
    ///
    /// 只用于验证「无归属 ⇒ fail-closed」契约本身；所有以「状态机/租约
    /// 行为」为不变量的用例请用 `logical_fixture_with_snapshot()`。
    fn unbound_fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        Fixture {
            _temp: temp,
            paths,
            store,
            attempt,
            logical_id: LogicalRepositoryId(Uuid::nil()),
            checkout_id: RepositoryCheckoutId(Uuid::nil()),
        }
    }

    fn logical_fixture() -> Fixture {
        logical_fixture_inner(None)
    }

    fn logical_fixture_with_snapshot() -> Fixture {
        let mut fixture = logical_fixture_inner(None);
        let snapshot = snapshot_for(&fixture.paths, fixture.logical_id, fixture.checkout_id);
        let mut attempt = fixture.attempt.clone();
        attempt.target_snapshot = Some(snapshot);
        fixture
            .store
            .write_coding_attempt_for_test(&attempt)
            .expect("persist snapshot");
        fixture.attempt = attempt;
        fixture
    }

    fn logical_fixture_inner(snapshot: Option<AttemptTargetSnapshot>) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let (logical_id, checkout_id) = seed_logical_authority(&paths, temp.path());
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: snapshot,
                max_auto_rework: 0,
            })
            .unwrap();
        Fixture {
            _temp: temp,
            paths,
            store,
            attempt,
            logical_id,
            checkout_id,
        }
    }

    /// 在给定 paths 下建 project + 逻辑代码库权威（manifest/member/checkout/
    /// issue selection/policy/repos.json）。REQ-ROUTE-01 两态化后 admission 的
    /// 唯一凭证是「LC 权威 + 冻结快照」：凡以 admission 为不变量的用例都必须
    /// 先建它，无归属输入一律 `FailClosed(TargetMissing)`。
    fn seed_logical_authority(
        paths: &ProductAppPaths,
        root: &std::path::Path,
    ) -> (LogicalRepositoryId, RepositoryCheckoutId) {
        crate::product::logical_codebase::logical_test_fixture::seed_logical_authority(
            paths, PROJECT_ID, ISSUE_ID, root,
        )
    }

    fn snapshot_for(
        paths: &ProductAppPaths,
        logical_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
    ) -> AttemptTargetSnapshot {
        crate::product::logical_codebase::logical_test_fixture::snapshot_for(paths, PROJECT_ID, logical_id, checkout_id)
    }

    fn provider_snapshot() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 0,
            permission_modes: Default::default(),
        }
    }

    /// 批 2 T1（REQ-WIGA-09）：精确绑定已 terminated 的 plan 拒绝 admission
    /// 进入 Running；manual 无绑定 attempt 不受影响（回归）。
    #[test]
    fn terminated_automation_plan_rejects_admission_but_manual_unaffected() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        // REQ-ROUTE-01 两态：admission 的唯一凭证是「LC 权威 + 冻结快照」——
        // manual 路径也必须先有真实归属，否则 fail-closed 的
        // `TargetSnapshotIdentityDrifted` 会掩盖本用例真正要钉的
        // `automation_plan_terminated` 拒绝。
        let (logical_id, checkout_id) = seed_logical_authority(&paths, temp.path());
        let snapshot = snapshot_for(&paths, logical_id, checkout_id);
        let store = CodingAttemptStore::new(paths.clone());
        let group_input = |issue_id: &str, plan_id: &str| {
            crate::product::coding_attempt_store::CreateGroupCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: issue_id.to_string(),
                plan_id: plan_id.to_string(),
                current_work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: format!("aria/{plan_id}/attempt-1"),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: Some(snapshot.clone()),
                max_auto_rework: 0,
                start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            }
        };

        // manual：无 enrollment 绑定的 plan，admission 正常签票并转 Running。
        let manual = store
            .create_group_attempt(group_input(ISSUE_ID, "work_item_plan_9999"))
            .unwrap();
        store
            .admit_and_transition_attempt_to_executable(
                &manual.project_id,
                &manual.issue_id,
                &manual.id,
            )
            .expect("manual plan admission is unaffected");
        let admitted = store
            .get_attempt(&manual.project_id, &manual.issue_id, &manual.id)
            .unwrap();
        assert_eq!(admitted.status, CodingAttemptStatus::Running);

        // 精确绑定已 terminated 的 plan：admission fail-closed 拒绝。第二现场
        // 用独立 issue——同 issue 下已有活跃 manual attempt 时，
        // create_group_attempt 被 active_coding_attempt_exists 准入拒绝。
        const TERMINATED_ISSUE_ID: &str = "issue_0002";
        let terminated_attempt = store
            .create_group_attempt(group_input(TERMINATED_ISSUE_ID, "work_item_plan_0001"))
            .unwrap();
        let enrollment_store =
            crate::product::issue_automation_store::IssueAutomationStore::new(paths.clone());
        let created = enrollment_store
            .compare_and_set(
                PROJECT_ID,
                TERMINATED_ISSUE_ID,
                None,
                crate::product::models::automation::EnrollmentWriteCommand::Enable {
                    command_id: None,
                    selection_key: "human-choice-1".to_string(),
                    source: crate::product::models::automation::EnrollmentSource {
                        stories: vec![crate::product::models::automation::SourceRevisionRef {
                            id: "story_spec_0001".into(),
                            version: 1,
                        }],
                        designs: vec![],
                    },
                    options: crate::product::models::automation::EnrollmentOptions {
                        author_provider: ProviderName::Fake,
                        reviewer_provider: ProviderName::Fake,
                        review_rounds: 1,
                        superpowers_enabled: false,
                        openspec_enabled: false,
                        plan_approval:
                            crate::product::models::automation::PlanApprovalPolicy::Manual,
                        plan_options: crate::product::models::lifecycle::IssueWorkItemPlanOptions {
                            include_integration_tests: true,
                            include_e2e_tests: false,
                            force_frontend_backend_split: false,
                            require_execution_plan_confirm: false,
                        },
                    },
                    target: crate::product::logical_codebase::EnrollmentTarget::LogicalCodebase {
                        logical_codebase_id: "logical_codebase_0001".into(),
                        logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
                    },
                },
            )
            .unwrap();
        let bound = enrollment_store
            .bind_plan(
                PROJECT_ID,
                TERMINATED_ISSUE_ID,
                created.policy_revision,
                "work_item_plan_0001",
                "session_0001",
            )
            .unwrap();
        enrollment_store
            .terminate_plan(
                PROJECT_ID,
                TERMINATED_ISSUE_ID,
                crate::product::models::automation::EnrollmentTerminationRequest {
                    command_id: "terminate-command-1".to_string(),
                    expected_enrollment_id: bound.enrollment_id.clone(),
                    expected_policy_revision: bound.policy_revision,
                    reason: "user requested termination".to_string(),
                },
            )
            .unwrap();

        let error = store
            .admit_and_transition_attempt_to_executable(
                &terminated_attempt.project_id,
                &terminated_attempt.issue_id,
                &terminated_attempt.id,
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("automation_plan_terminated"),
            "unexpected error: {error}"
        );
        let rejected = store
            .get_attempt(
                &terminated_attempt.project_id,
                &terminated_attempt.issue_id,
                &terminated_attempt.id,
            )
            .unwrap();
        assert_eq!(
            rejected.status,
            CodingAttemptStatus::Created,
            "terminated plan attempt must not enter Running"
        );
    }

    mod admission_restart_tests;

    include!("admission_lease_tests.rs");
