    #[test]
    fn frontend_and_lib_manifest_profiles_are_detected_without_java_initialization() {
        let fixture = tempfile::tempdir().unwrap();
        let web = fixture.path().join("web");
        let shared = fixture.path().join("shared");
        fs::create_dir_all(&web).unwrap();
        fs::create_dir_all(&shared).unwrap();
        fs::write(
            web.join("package.json"),
            r#"{"name":"web","scripts":{"test":"vitest"}}"#,
        )
        .unwrap();
        fs::write(web.join("pnpm-lock.yaml"), "lockfileVersion: '9.0'").unwrap();
        fs::write(web.join("vite.config.ts"), "export default {}").unwrap();
        fs::write(shared.join("package.json"), r#"{"name":"shared"}"#).unwrap();

        let frontend = RepositoryProfileDetector::detect(&web).unwrap();
        let library = RepositoryProfileDetector::detect(&shared).unwrap();
        let parsed: RepositoryType = serde_json::from_str("\"lib\"").unwrap();

        assert_eq!(frontend.repo_type, RepositoryType::Frontend);
        assert_eq!(frontend.tech_stack, vec!["package.json", "pnpm", "vite"]);
        assert!(frontend.initialization_commands.is_empty());
        assert_eq!(library.repo_type, RepositoryType::Library);
        assert!(library.initialization_commands.is_empty());
        assert_eq!(parsed, RepositoryType::Library);
    }

    #[test]
    fn java_maven_and_gradle_profiles_are_detected_as_backend() {
        let fixture = tempfile::tempdir().unwrap();
        let maven = fixture.path().join("api");
        let gradle = fixture.path().join("worker");
        fs::create_dir_all(&maven).unwrap();
        fs::create_dir_all(&gradle).unwrap();
        fs::write(
            maven.join("pom.xml"),
            "<project><modelVersion>4.0.0</modelVersion></project>",
        )
        .unwrap();
        fs::write(gradle.join("build.gradle.kts"), "plugins {}").unwrap();

        let maven_profile = RepositoryProfileDetector::detect(&maven).unwrap();
        let gradle_profile = RepositoryProfileDetector::detect(&gradle).unwrap();
        assert_eq!(maven_profile.repo_type, RepositoryType::Backend);
        assert!(maven_profile.tech_stack.contains(&"maven".to_string()));
        assert_eq!(gradle_profile.repo_type, RepositoryType::Backend);
        assert!(gradle_profile.tech_stack.contains(&"gradle".to_string()));
        // Backend detection never runs initialization commands.
        assert!(maven_profile.initialization_commands.is_empty());
        assert!(gradle_profile.initialization_commands.is_empty());
    }

    struct AttachFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        coordinator: LogicalCodebaseRegistrationCoordinator,
        git_root: PathBuf,
        head_before: String,
        branch_before: String,
        status_before: String,
    }

    struct ScanFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        root: CanonicalAggregateRoot,
        clean_git: PathBuf,
        non_git: PathBuf,
        nested_git: PathBuf,
        dirty_git: PathBuf,
        missing: PathBuf,
        outside: PathBuf,
        coordinator: LogicalCodebaseRegistrationCoordinator,
    }

    struct BatchFixture {
        _root: tempfile::TempDir,
        root: CanonicalAggregateRoot,
        first: PathBuf,
        second: PathBuf,
        coordinator: LogicalCodebaseRegistrationCoordinator,
    }

    struct LinkedWorktreeFixture {
        _root: tempfile::TempDir,
        root: CanonicalAggregateRoot,
        linked: PathBuf,
        coordinator: LogicalCodebaseRegistrationCoordinator,
    }

    struct FiftyRepositoryFixture {
        _root: tempfile::TempDir,
        root: CanonicalAggregateRoot,
        repositories: Vec<PathBuf>,
        coordinator: LogicalCodebaseRegistrationCoordinator,
    }

    impl FiftyRepositoryFixture {
        fn confirmed_preflight(&self) -> ConfirmedRegistrationBatchInput {
            ConfirmedRegistrationBatchInput::from_preflight(
                &self
                    .coordinator
                    .preflight(RegistrationPreflightInput {
                        project_id: "project_0001".to_string(),
                        aggregate_root: self.root.clone(),
                        paths: self.repositories.clone(),
                    })
                    .unwrap(),
                false,
            )
        }
    }

    impl BatchFixture {
        fn preflight(&self) -> RegistrationPreflightResult {
            self.coordinator
                .preflight(RegistrationPreflightInput {
                    project_id: "project_0001".to_string(),
                    aggregate_root: self.root.clone(),
                    paths: vec![self.first.clone(), self.second.clone()],
                })
                .unwrap()
        }

        fn fail_after_first_completed_item(&self) {
            self.coordinator
                .failure_after_completed_items
                .store(1, Ordering::SeqCst);
        }

        fn change_head_of_second_repository(&self) {
            fs::write(self.second.join("README.md"), "# Second changed\n").unwrap();
            git(&self.second, &["add", "README.md"]);
            git(&self.second, &["commit", "--quiet", "-m", "changed"]);
        }
    }

    #[test]
    fn registering_fifty_members_preserves_every_git_checkout_byte_for_byte() {
        let fixture = fifty_repository_fixture();
        let before: Vec<_> = fixture
            .repositories
            .iter()
            .map(|root| RepositoryGitSnapshot::capture(root).unwrap())
            .collect();
        let batch = fixture
            .coordinator
            .submit_confirmed_batch(fixture.confirmed_preflight())
            .unwrap();
        let completed = fixture
            .coordinator
            .resume_batch("project_0001", &batch.id)
            .unwrap();
        assert_eq!(completed.status, RegistrationBatchStatus::Completed);

        for (root, snapshot) in fixture.repositories.iter().zip(before.iter()) {
            snapshot
                .assert_unchanged(&RepositoryGitSnapshot::capture(root).unwrap())
                .unwrap();
            assert!(!root.join(".aria").exists());
            assert!(!root.join(".codegraph").exists());
        }
    }

    #[test]
    fn resumed_batch_revalidates_revision_skips_completed_item_and_marks_changed_item_for_reconfirmation()
     {
        let fixture = batch_fixture_with_two_git_repositories();
        let preflight = fixture.preflight();
        let batch = fixture
            .coordinator
            .submit_confirmed_batch(ConfirmedRegistrationBatchInput::from_preflight(
                &preflight, true,
            ))
            .unwrap();
        fixture.fail_after_first_completed_item();
        let interrupted = fixture
            .coordinator
            .resume_batch("project_0001", &batch.id)
            .unwrap();
        assert_eq!(interrupted.status, RegistrationBatchStatus::PartialFailed);

        fixture.change_head_of_second_repository();

        let resumed = fixture
            .coordinator
            .resume_batch("project_0001", &batch.id)
            .unwrap();
        let first = resumed
            .items
            .iter()
            .find(|item| item.canonical_path == fixture.first)
            .expect("first repository batch item");
        let second = resumed
            .items
            .iter()
            .find(|item| item.canonical_path == fixture.second)
            .expect("second repository batch item");
        assert_eq!(first.status, RegistrationItemStatus::Completed);
        assert_eq!(second.status, RegistrationItemStatus::NeedsAttention);
        assert_eq!(
            second.failure_reason.as_deref(),
            Some("preflight_revision_changed")
        );
    }

    #[test]
    fn preflight_groups_mixed_manifest_and_marks_dirty_repository_needs_attention() {
        let fixture = scan_fixture();
        let result = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![
                    fixture.clean_git.clone(),
                    fixture.non_git.clone(),
                    fixture.clean_git.clone(),
                    fixture.nested_git.clone(),
                    fixture.dirty_git.clone(),
                    fixture.missing.clone(),
                    fixture.outside.clone(),
                ],
            })
            .unwrap();

        assert_eq!(result.count(RegistrationCandidateState::Eligible), 1);
        assert_eq!(result.count(RegistrationCandidateState::NonGit), 1);
        assert_eq!(result.count(RegistrationCandidateState::Duplicate), 1);
        assert_eq!(result.count(RegistrationCandidateState::Nested), 1);
        assert_eq!(result.count(RegistrationCandidateState::NeedsAttention), 1);
        assert_eq!(result.count(RegistrationCandidateState::Missing), 1);
        assert_eq!(result.count(RegistrationCandidateState::OutsideRoot), 1);
    }

    #[test]
    fn preflight_classifies_linked_worktree_candidate_as_nested() {
        let fixture = linked_worktree_fixture();
        let result = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.linked.clone()],
            })
            .unwrap();

        assert_eq!(result.candidates.len(), 1);
        assert_eq!(
            result.candidates[0].state,
            RegistrationCandidateState::Nested
        );
        assert_eq!(result.candidates[0].reason, "nested_worktree");
    }

    #[test]
    fn preflight_classifies_aggregate_root_itself_as_outside_root() {
        let fixture = scan_fixture();
        let result = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.root.canonical_path.clone()],
            })
            .unwrap();

        assert_eq!(
            result.count(RegistrationCandidateState::OutsideRoot),
            1
        );
        assert_eq!(result.candidates[0].reason, "outside_aggregate_root");
    }

    #[test]
    fn preflight_discovers_direct_child_repositories_and_registry_duplicates() {
        let fixture = scan_fixture();
        let initial = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![],
            })
            .unwrap();
        assert_eq!(initial.count(RegistrationCandidateState::Eligible), 1);
        assert_eq!(initial.count(RegistrationCandidateState::NeedsAttention), 1);
        assert_eq!(initial.count(RegistrationCandidateState::Nested), 1);

        let clean = initial
            .candidates
            .iter()
            .find(|candidate| candidate.state == RegistrationCandidateState::Eligible)
            .unwrap();
        let source_identity = clean.source_identity.clone().unwrap();
        IdentityRegistryStore::new(fixture.paths.clone())
            .upsert_active(
                "project_0001",
                crate::product::logical_codebase::IdentityRegistryEntry::active(
                    source_identity,
                    crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::nil()),
                    "repository_0001".into(),
                    crate::product::logical_codebase::RepositoryCheckoutId(uuid::Uuid::nil()),
                    "test-create".into(),
                ),
            )
            .unwrap();

        let duplicate = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.clean_git.clone()],
            })
            .unwrap();
        assert_eq!(duplicate.count(RegistrationCandidateState::Duplicate), 1);
        assert_eq!(duplicate.candidates[0].reason, "already_registered");
    }

    #[test]
    fn preflight_revisions_change_with_head_and_worktree_status() {
        let fixture = scan_fixture();
        let first = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.clean_git.clone()],
            })
            .unwrap();
        fs::write(fixture.clean_git.join("README.md"), "changed\n").unwrap();
        let dirty = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.clean_git.clone()],
            })
            .unwrap();
        assert_eq!(
            dirty.candidates[0].state,
            RegistrationCandidateState::NeedsAttention
        );
        assert_ne!(
            first.candidates[0].preflight_revision,
            dirty.candidates[0].preflight_revision
        );

        git(&fixture.clean_git, &["add", "README.md"]);
        git(&fixture.clean_git, &["commit", "--quiet", "-m", "changed"]);
        let committed = fixture
            .coordinator
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: fixture.root.clone(),
                paths: vec![fixture.clean_git.clone()],
            })
            .unwrap();
        assert_eq!(
            committed.candidates[0].state,
            RegistrationCandidateState::Eligible
        );
        assert_ne!(
            dirty.candidates[0].preflight_revision,
            committed.candidates[0].preflight_revision
        );
    }

