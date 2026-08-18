#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        LogicalCodebaseFeature, LogicalCodebaseManifest, LogicalCodebaseStore,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::repository_store::RepositoryStore;

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

    #[test]
    fn attach_only_registration_creates_member_without_initialization_operation() {
        let fixture = attach_fixture();
        let member = fixture
            .coordinator
            .attach_member(AttachOnlyRegistrationInput {
                project_id: "project_0001".into(),
                alias: "api".into(),
                role: "service".into(),
                canonical_path: fixture.git_root.clone(),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".into()],
                idempotency_key: "batch-1:item-api".into(),
            })
            .unwrap();

        assert_eq!(member.alias, "api");
        assert_eq!(member.role, "service");
        assert_eq!(member.repo_type, RepositoryType::Backend);
        assert_eq!(member.tech_stack, vec!["rust"]);
        assert!(
            !fixture
                .paths
                .repository_initializations_root("project_0001")
                .exists()
        );
        assert_eq!(
            git_output(&fixture.git_root, &["rev-parse", "HEAD"]),
            fixture.head_before
        );
        assert_eq!(
            git_output(&fixture.git_root, &["branch", "--show-current"]),
            fixture.branch_before
        );
        assert_eq!(
            git_output(&fixture.git_root, &["status", "--porcelain"]),
            fixture.status_before
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rejects_git_root_symlink_escape_and_ownership_conflicts() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_root();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_is_git"
        );

        fixture.remove_root_git();
        fixture.create_symlink_member_to_outside_root();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.symlink_member),
                )
                .unwrap_err()
                .code(),
            "member_symlink_escape"
        );

        fixture.create_file("CLAUDE.md", "user instructions");
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_ownership_conflict"
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rejects_members_outside_root_nested_worktrees_and_overlap() {
        let fixture = aggregate_root_fixture();
        let outside_member = fixture.root.parent().unwrap().join("outside-member");
        fs::create_dir(&outside_member).unwrap();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&outside_member),
                )
                .unwrap_err()
                .code(),
            "member_path_outside_root"
        );
        assert!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.root),
                )
                .is_err_and(|error| error.code() == "member_path_outside_root")
        );

        fixture.init_git_at_member();
        fixture.create_nested_worktree_at_member();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.nested_worktree_member),
                )
                .unwrap_err()
                .code(),
            "nested_worktree"
        );

        fixture.remove_nested_worktree();
        fixture.save_manifest_for("project_0002", fixture.root.parent().unwrap());
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_accepts_non_git_root_with_direct_git_members() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_member();

        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap()
                .canonical_path,
            fs::canonicalize(&fixture.root).unwrap()
        );
    }

    struct AggregateRootFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        root: PathBuf,
        member: PathBuf,
        symlink_member: PathBuf,
        nested_worktree_member: PathBuf,
    }

    impl AggregateRootFixture {
        fn preflight(&self) -> AggregateRootPreflight {
            AggregateRootPreflight::new(self.paths.clone())
        }

        fn create_file(&self, relative_path: &str, contents: &str) {
            fs::write(self.root.join(relative_path), contents).unwrap();
        }

        fn init_git_at_root(&self) {
            git(&self.root, &["init", "--quiet"]);
        }

        fn remove_root_git(&self) {
            fs::remove_dir_all(self.root.join(".git")).unwrap();
        }

        fn init_git_at_member(&self) {
            git(&self.member, &["init", "--quiet"]);
            git(
                &self.member,
                &["config", "user.email", "aria@example.invalid"],
            );
            git(&self.member, &["config", "user.name", "Aria Test"]);
            fs::write(self.member.join("README.md"), "# Member\n").unwrap();
            git(&self.member, &["add", "README.md"]);
            git(&self.member, &["commit", "--quiet", "-m", "initial"]);
        }

        fn create_symlink_member_to_outside_root(&self) {
            let outside_member = self.root.parent().unwrap().join("outside-symlink-target");
            fs::create_dir(&outside_member).unwrap();
            symlink(&outside_member, &self.symlink_member).unwrap();
        }

        fn create_nested_worktree_at_member(&self) {
            git(
                &self.member,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    self.nested_worktree_member.to_str().unwrap(),
                ],
            );
        }

        fn remove_nested_worktree(&self) {
            git(
                &self.member,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    self.nested_worktree_member.to_str().unwrap(),
                ],
            );
        }

        fn save_manifest_for(&self, project_id: &str, root: &Path) {
            let manifest = LogicalCodebaseManifest::new(
                project_id,
                fs::canonicalize(root).unwrap(),
                Vec::new(),
            );
            LogicalCodebaseStore::new(self.paths.clone())
                .save_manifest(project_id, &manifest)
                .unwrap();
        }
    }

    fn aggregate_root_fixture() -> AggregateRootFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        let root = temp.path().join("aggregate-root");
        let member = root.join("member");
        let symlink_member = root.join("symlink-member");
        let nested_worktree_member = member.join("nested-worktree");
        fs::create_dir_all(&member).unwrap();
        for name in ["project", "project-other"] {
            ProjectStore::new(paths.clone())
                .create(CreateProjectInput {
                    name: name.to_string(),
                    description: None,
                multi_repo: false,
                })
                .unwrap();
        }

        AggregateRootFixture {
            _temp: temp,
            paths,
            root,
            member,
            symlink_member,
            nested_worktree_member,
        }
    }

    fn scan_fixture() -> ScanFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".into(),
                description: None,
                multi_repo: false,
            })
            .unwrap();

        let root = temp.path().join("aggregate-root");
        let clean_git = root.join("clean-git");
        let non_git = root.join("non-git");
        let nested_git = clean_git.join("nested-git");
        let dirty_git = root.join("dirty-git");
        let missing = root.join("missing");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&non_git).unwrap();
        fs::create_dir_all(&outside).unwrap();
        init_git_repository(&clean_git);
        fs::write(clean_git.join(".git/info/exclude"), "nested-git/\n").unwrap();
        init_git_repository(&nested_git);
        init_git_repository(&dirty_git);
        fs::write(dirty_git.join("README.md"), "dirty\n").unwrap();

        let repositories = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );
        ScanFixture {
            _temp: temp,
            paths: paths.clone(),
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(root).unwrap(),
            },
            clean_git,
            non_git,
            nested_git,
            dirty_git,
            missing,
            outside,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repositories,
                LogicalCodebaseFeature::enabled(),
            ),
        }
    }

    fn linked_worktree_fixture() -> LinkedWorktreeFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
                multi_repo: false,
            })
            .unwrap();
        let root_path = temp.path().join("aggregate-root");
        let main = root_path.join("main");
        let linked = root_path.join("linked");
        init_git_repository(&main);
        git(
            &main,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        let repositories = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );
        LinkedWorktreeFixture {
            _root: temp,
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(&root_path).unwrap(),
            },
            linked,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repositories,
                LogicalCodebaseFeature::enabled(),
            ),
        }
    }

    fn fifty_repository_fixture() -> FiftyRepositoryFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
                multi_repo: false,
            })
            .unwrap();
        let root_path = temp.path().join("aggregate-root");
        let repositories = (1..=50)
            .map(|index| {
                let path = root_path.join(format!("repo_{index:04}"));
                init_git_repository(&path);
                path
            })
            .collect::<Vec<_>>();
        let repository_store = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );
        FiftyRepositoryFixture {
            _root: temp,
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(root_path).unwrap(),
            },
            repositories,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repository_store,
                LogicalCodebaseFeature::enabled(),
            ),
        }
    }

    fn batch_fixture_with_two_git_repositories() -> BatchFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
                multi_repo: false,
            })
            .unwrap();
        let root_path = temp.path().join("aggregate-root");
        let first = root_path.join("first");
        let second = root_path.join("second");
        init_git_repository(&first);
        init_git_repository(&second);
        let repositories = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );
        BatchFixture {
            _root: temp,
            root: CanonicalAggregateRoot {
                canonical_path: fs::canonicalize(&root_path).unwrap(),
            },
            first,
            second,
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repositories,
                LogicalCodebaseFeature::enabled(),
            ),
        }
    }

    fn init_git_repository(path: &Path) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "--quiet"]);
        git(path, &["config", "user.email", "aria@example.invalid"]);
        git(path, &["config", "user.name", "Aria Test"]);
        fs::write(path.join("README.md"), "# Repository\n").unwrap();
        git(path, &["add", "README.md"]);
        git(path, &["commit", "--quiet", "-m", "initial"]);
    }

    fn attach_fixture() -> AttachFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".into(),
                description: None,
                multi_repo: false,
            })
            .unwrap();

        let git_root = root.path().join("api");
        fs::create_dir_all(&git_root).unwrap();
        git(&git_root, &["init", "--quiet"]);
        git(&git_root, &["config", "user.email", "aria@example.invalid"]);
        git(&git_root, &["config", "user.name", "Aria Test"]);
        fs::write(git_root.join("README.md"), "# API\n").unwrap();
        git(&git_root, &["add", "README.md"]);
        git(&git_root, &["commit", "--quiet", "-m", "initial"]);

        let head_before = git_output(&git_root, &["rev-parse", "HEAD"]);
        let branch_before = git_output(&git_root, &["branch", "--show-current"]);
        let status_before = git_output(&git_root, &["status", "--porcelain"]);
        let repositories = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );

        AttachFixture {
            _root: root,
            paths: paths.clone(),
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repositories,
                LogicalCodebaseFeature::enabled(),
            ),
            git_root,
            head_before,
            branch_before,
            status_before,
        }
    }

    fn git(cwd: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(cwd: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }
}
