    fn legacy_fixture() -> Fixture {
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
        let snapshot = snapshot_for(&fixture);
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
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let logical_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let repository_path = temp.path().join("repository_0001");
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &repository_path,
            repository_path.join(".git"),
            None,
        );
        let authority = LogicalCodebaseStore::new(paths.clone());
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            temp.path().join("aggregate-root"),
            vec![logical_id],
        );
        authority.save_manifest(PROJECT_ID, &manifest).unwrap();
        authority
            .save_member(
                PROJECT_ID,
                &CodebaseMemberRecord {
                    logical_repository_id: logical_id,
                    physical_repository_id: "repository_0001".to_string(),
                    alias: "repository_0001".to_string(),
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
                    created_at: "2026-08-11T00:00:00Z".to_string(),
                    updated_at: "2026-08-11T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        authority
            .save_checkout(
                PROJECT_ID,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: logical_id,
                    physical_repository_id: "repository_0001".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: repository_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-08-11T00:00:00Z".to_string(),
                    created_at: "2026-08-11T00:00:00Z".to_string(),
                    updated_at: "2026-08-11T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                PROJECT_ID,
                ISSUE_ID,
                vec![logical_id],
                Vec::new(),
                vec![logical_id],
                None,
            ))
            .unwrap();
        write_json(
            &paths.project_root(PROJECT_ID).join("repos.json"),
            &[RepositoryRecord {
                id: "repository_0001".to_string(),
                project_id: PROJECT_ID.to_string(),
                name: "repository_0001".to_string(),
                path: repository_path,
                repo_hash: "sha256:repository".to_string(),
                runtime_root: PathBuf::from("runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
                logical_repository_id: Some(logical_id),
                primary_checkout_id: Some(checkout_id),
                identity_schema_version: 1,
            }],
        )
        .unwrap();
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .unwrap();
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

    fn snapshot_for(fixture: &Fixture) -> AttemptTargetSnapshot {
        let checkout = LogicalCodebaseStore::new(fixture.paths.clone())
            .load_checkout(PROJECT_ID, fixture.checkout_id)
            .unwrap()
            .unwrap();
        let manifest = LogicalCodebaseStore::new(fixture.paths.clone())
            .load_manifest(PROJECT_ID)
            .unwrap()
            .unwrap();
        let policy = AggregatePolicyArtifactStore::new(fixture.paths.clone())
            .get(PROJECT_ID)
            .unwrap()
            .unwrap();
        AttemptTargetSnapshot {
            logical_repository_id: fixture.logical_id,
            checkout_id: fixture.checkout_id,
            physical_repository_id: "repository_0001".to_string(),
            canonical_path: checkout.canonical_path,
            git_dir_identity: checkout.git_dir_identity,
            revision: checkout.revision,
            policy_digest: policy.digest,
            membership_revision: manifest.membership_revision,
            captured_at: "2026-08-11T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }
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
                target_snapshot: None,
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
