mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId,
        RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
    };

    #[test]
    fn authority_store_roundtrips_manifest_member_and_checkout_by_uuid_file_name() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().to_path_buf(),
            vec![member_id],
        );

        store.save_manifest("project_0001", &manifest).unwrap();
        store
            .save_member("project_0001", &member_fixture(member_id, checkout_id))
            .unwrap();
        store
            .save_checkout("project_0001", &checkout_fixture(member_id, checkout_id))
            .unwrap();

        assert_eq!(
            store
                .load_manifest("project_0001")
                .unwrap()
                .unwrap()
                .member_ids,
            vec![member_id]
        );
        assert_eq!(
            store.list_members("project_0001").unwrap()[0].logical_repository_id,
            member_id
        );
        assert_eq!(
            store.list_checkouts("project_0001").unwrap()[0].checkout_id,
            checkout_id
        );
        assert!(
            temp.path()
                .join("projects/project_0001/logical-codebase/members")
                .join(format!("{}.json", member_id.0))
                .exists()
        );
        assert!(
            temp.path()
                .join("projects/project_0001/logical-codebase/checkouts")
                .join(format!("{}.json", checkout_id.0))
                .exists()
        );
    }

    #[test]
    fn changed_members_require_membership_revision_to_increase_by_one() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let first_member_id = LogicalRepositoryId(Uuid::new_v4());
        let second_member_id = LogicalRepositoryId(Uuid::new_v4());
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().to_path_buf(),
            vec![first_member_id],
        );
        store.save_manifest("project_0001", &manifest).unwrap();

        let mut changed = manifest.clone();
        changed.member_ids.push(second_member_id);
        assert!(matches!(
            store.save_manifest("project_0001", &changed),
            Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_manifest",
                ..
            })
        ));

        changed.membership_revision += 1;
        store.save_manifest("project_0001", &changed).unwrap();
        assert_eq!(
            store
                .load_manifest("project_0001")
                .unwrap()
                .unwrap()
                .membership_revision,
            2
        );
    }

    fn member_fixture(
        logical_repository_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
    ) -> CodebaseMemberRecord {
        let now = "2026-08-08T00:00:00Z".to_string();
        CodebaseMemberRecord {
            logical_repository_id,
            physical_repository_id: "repository_0001".to_string(),
            alias: "api".to_string(),
            role: "service".to_string(),
            ordinal: 1,
            source_identity: RepositorySourceIdentity::from_git_parts(
                Path::new("/workspace/api"),
                PathBuf::from("/workspace/api/.git"),
                Some("ssh://git@example.test/acme/api.git".to_string()),
            ),
            repo_type: Default::default(),
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout_id],
            status: Default::default(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn logical_codebase_create_rejects_duplicate_name_and_allows_reuse_after_tombstone() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let input = |name: &str| LogicalCodebaseCreateInput {
            name: name.to_string(),
            aggregate_root: PathBuf::from("/workspace/platform"),
        };
        let record = store.create("project_0001", input("Platform")).unwrap();

        assert!(matches!(
            store.create("project_0001", input("Platform")),
            Err(ProductStoreError::Conflict {
                kind: "logical_codebase_name",
                id,
            }) if id == "Platform"
        ));
        // 不同名不受影响。
        assert!(store.create("project_0001", input("Edge")).is_ok());

        store.delete_soft("project_0001", &record.id).unwrap();
        assert!(store.create("project_0001", input("Platform")).is_ok());
    }

    #[test]
    fn lc_manifest_and_members_fallback_to_legacy_alias() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().to_path_buf(),
            vec![member_id],
        );
        store.save_manifest("project_0001", &manifest).unwrap();
        store
            .save_member(
                "project_0001",
                &member_fixture(member_id, RepositoryCheckoutId(Uuid::new_v4())),
            )
            .unwrap();

        let legacy_id = legacy_logical_codebase_id("project_0001");
        let migrated = store.migrate_legacy("project_0001").unwrap().unwrap();
        assert_eq!(migrated.id, legacy_id);

        let loaded = store
            .load_lc_manifest("project_0001", &legacy_id)
            .unwrap()
            .expect("legacy alias manifest");
        assert_eq!(loaded.member_ids, vec![member_id]);
        assert_eq!(
            store
                .list_lc_members("project_0001", &legacy_id)
                .unwrap()
                .len(),
            1
        );

        // 新建 LC：manifest 待首批登记创建 → 空。
        let fresh = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Fresh".to_string(),
                    aggregate_root: PathBuf::from("/workspace/fresh"),
                },
            )
            .unwrap();
        assert!(
            store
                .load_lc_manifest("project_0001", &fresh.id)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .list_lc_members("project_0001", &fresh.id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn logical_codebase_record_store_round_trips_and_soft_deletes() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let record = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Platform".to_string(),
                    aggregate_root: PathBuf::from("/workspace/platform"),
                },
            )
            .unwrap();

        assert_eq!(
            store.get("project_0001", &record.id).unwrap(),
            Some(record.clone())
        );
        assert_eq!(store.list("project_0001").unwrap(), vec![record.clone()]);
        let logical_codebase_root = temp
            .path()
            .join("projects/project_0001/logical-codebases")
            .join(&record.id);
        assert!(logical_codebase_root.join("record.json").exists());
        for path in [
            "members",
            "checkouts",
            "aggregate-indexes",
            "preflights",
            "registration-batches",
            "pointer-publications",
        ] {
            assert!(logical_codebase_root.join(path).is_dir(), "missing {path}");
        }

        store.delete_soft("project_0001", &record.id).unwrap();
        assert!(store.list("project_0001").unwrap().is_empty());
        assert!(
            temp.path()
                .join("projects/project_0001/logical-codebases")
                .join(&record.id)
                .join("tombstone.json")
                .exists()
        );
    }

    #[test]
    fn logical_codebase_record_requires_a_name() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));

        assert!(matches!(
            store.create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: " ".to_string(),
                    aggregate_root: temp.path().to_path_buf(),
                },
            ),
            Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_record",
                ..
            })
        ));
    }

    #[test]
    fn legacy_logical_codebase_migration_is_stable_and_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let legacy_root = paths.logical_codebase_root("project_0001");
        std::fs::create_dir_all(legacy_root.join("members")).unwrap();
        std::fs::write(legacy_root.join("members/keep.json"), "{\"keep\":true}").unwrap();
        std::fs::write(
            legacy_root.join("manifest.json"),
            serde_json::json!({
                "schema_version": 1,
                "project_id": "project_0001",
                "logical_codebase_id": uuid::Uuid::new_v4(),
                "provider_context_root": "/workspace/platform",
                "layout": "common_non_git_parent",
                "membership_revision": 1,
                "member_ids": [],
                "created_at": "2026-08-18T00:00:00Z",
                "updated_at": "2026-08-18T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();

        let store = LogicalCodebaseStore::new(paths.clone());
        let first = store.migrate_legacy("project_0001").unwrap().unwrap();
        let second = store.migrate_legacy("project_0001").unwrap().unwrap();
        assert_eq!(first, second);
        assert_eq!(store.list("project_0001").unwrap(), vec![first.clone()]);
        assert_eq!(
            std::fs::read_to_string(
                paths
                    .logical_codebases_root("project_0001")
                    .join(&first.id)
                    .join("members/keep.json")
            )
            .unwrap(),
            "{\"keep\":true}"
        );
    }

    #[test]
    fn gateway_bootstrap_artifacts_do_not_count_as_logical_codebase_storage() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let legacy_root = paths.logical_codebase_root("project_0001");
        std::fs::create_dir_all(&legacy_root).unwrap();
        std::fs::write(legacy_root.join("capabilities.json"), "[]").unwrap();
        std::fs::write(legacy_root.join("aggregate-policy.json"), "{}").unwrap();

        let store = LogicalCodebaseStore::new(paths.clone());
        assert_eq!(store.migrate_legacy("project_0001").unwrap(), None);
        assert!(!store.has_any_storage("project_0001").unwrap());
    }

    fn checkout_fixture(
        logical_repository_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
    ) -> RepositoryCheckoutRecord {
        let now = "2026-08-08T00:00:00Z".to_string();
        RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id,
            physical_repository_id: "repository_0001".to_string(),
            kind: CheckoutKind::Main,
            canonical_path: PathBuf::from("/workspace/api"),
            checkout_path_hash: "sha256:checkout".to_string(),
            git_dir_identity: "sha256:git-dir".to_string(),
            revision: Some("abc123".to_string()),
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        }
    }
}
