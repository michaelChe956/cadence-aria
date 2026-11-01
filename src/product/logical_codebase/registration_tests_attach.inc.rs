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
    fn cross_codebase_attach_serializes_on_project_identity_lock() {
        let fixture = attach_fixture();
        let paths = fixture.paths.clone();
        // identity-registry.json is project-scoped (v1.3), so one physical
        // repository cannot be registered in two logical codebases. The
        // per-LC manifest writer lock cannot enforce that invariant: two
        // codebases would hold different per-LC locks while read-modify-
        // writing the shared registry. Assert the attach path also takes the
        // project-scoped identity migration lock, so a second codebase waits
        // for the first codebase's critical section to finish.
        let lc_b = LogicalCodebaseRegistrationCoordinator::for_lc(paths.clone(), "lc-b");
        LogicalCodebaseStore::for_lc(paths.clone(), "lc-b")
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    fixture._root.path().to_path_buf(),
                    Vec::new(),
                ),
            )
            .unwrap();

        let lock_path = paths.identity_migration_lock_path("project_0001");
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let input = AttachOnlyRegistrationInput {
            project_id: "project_0001".into(),
            alias: "api".into(),
            role: "service".into(),
            canonical_path: fixture.git_root.clone(),
            repo_type: RepositoryType::Backend,
            tech_stack: vec!["rust".into()],
            idempotency_key: "lc-b:item-api".into(),
        };

        let (finished_early, worker) = with_exact_exclusive_lock(&lock_path, || {
            let worker = std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                let _ = done_tx.send(lc_b.attach_member(input));
            });
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok((
                done_rx.recv_timeout(Duration::from_secs(1)).is_ok(),
                worker,
            ))
        })
        .unwrap();

        assert!(
            !finished_early,
            "lc-b attach must block on the project identity lock held by another codebase"
        );

        let result = done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            result.is_ok(),
            "lc-b attach after lock release should succeed: {result:?}"
        );
        worker.join().unwrap();
    }

    #[test]
    fn soft_deleted_lc_releases_sources_for_recreated_codebase_registration() {
        // Bug3 症状2 现场（2026-10-16）：LC 软删不释放 identity-registry，
        // 重建同名 LC 后预检/登记同 source 被 already_registered 自锁。
        // 期望：delete_soft 同步释放条目，重建 LC 可完成登记（成员落新子树）。
        let fixture = attach_fixture();
        let paths = fixture.paths.clone();
        let aggregate_root = fixture._root.path().to_path_buf();
        let store = LogicalCodebaseStore::new(paths.clone());
        let first = store
            .create(
                "project_0001",
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "platform".into(),
                    aggregate_root: aggregate_root.clone(),
                },
            )
            .unwrap();
        LogicalCodebaseStore::for_lc(paths.clone(), &first.id)
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new()),
            )
            .unwrap();
        LogicalCodebaseRegistrationCoordinator::for_lc(paths.clone(), &first.id)
            .attach_member(AttachOnlyRegistrationInput {
                project_id: "project_0001".into(),
                alias: "api".into(),
                role: "service".into(),
                canonical_path: fixture.git_root.clone(),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".into()],
                idempotency_key: "lc1:item-api".into(),
            })
            .unwrap();

        store.delete_soft("project_0001", &first.id).unwrap();

        // 重建同名 LC（tombstone 不占名）。
        let second = store
            .create(
                "project_0001",
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "platform".into(),
                    aggregate_root: aggregate_root.clone(),
                },
            )
            .unwrap();
        assert_ne!(second.id, first.id);
        LogicalCodebaseStore::for_lc(paths.clone(), &second.id)
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new()),
            )
            .unwrap();
        let recreated = LogicalCodebaseRegistrationCoordinator::for_lc(paths.clone(), &second.id);

        // 预检：释放后的 source 不再 already_registered（红测现状：Duplicate）。
        let preflight = recreated
            .preflight(RegistrationPreflightInput {
                project_id: "project_0001".into(),
                aggregate_root: CanonicalAggregateRoot {
                    canonical_path: aggregate_root.clone(),
                },
                paths: vec![fixture.git_root.clone()],
            })
            .unwrap();
        assert_eq!(
            preflight.candidates.len(),
            1,
            "{:?}",
            preflight.candidates
        );
        assert_eq!(
            preflight.candidates[0].state,
            RegistrationCandidateState::Eligible,
            "released source must be re-registerable after its owning LC is soft-deleted"
        );

        // 登记：成员落重建 LC 子树，且不被旧条目 IdentityMismatch 拒绝。
        let member = recreated
            .attach_member(AttachOnlyRegistrationInput {
                project_id: "project_0001".into(),
                alias: "api".into(),
                role: "service".into(),
                canonical_path: fixture.git_root.clone(),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".into()],
                idempotency_key: "lc2:item-api".into(),
            })
            .unwrap();
        let members = LogicalCodebaseStore::for_lc(paths.clone(), &second.id)
            .list_members("project_0001")
            .unwrap();
        assert_eq!(members.len(), 1, "{members:?}");
        assert_eq!(
            members[0].logical_repository_id,
            member.logical_repository_id
        );
    }

