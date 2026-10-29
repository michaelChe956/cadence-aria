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
    fn logical_codebase_restore_round_trips_and_is_idempotent() {
        // Bug3 修复①：摘墓碑恢复原子树（id 不变）；已 active 幂等返回现状；
        // 未知 id → NotFound。
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let record = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Platform".to_string(),
                    aggregate_root: temp.path().to_path_buf(),
                },
            )
            .unwrap();

        store.delete_soft("project_0001", &record.id).unwrap();
        assert_eq!(store.get("project_0001", &record.id).unwrap(), None);

        match store.restore("project_0001", &record.id).unwrap() {
            LogicalCodebaseRestoreOutcome::Restored(restored) => {
                assert_eq!(restored, record);
            }
            other => panic!("expected Restored, got {other:?}"),
        }
        assert_eq!(
            store.get("project_0001", &record.id).unwrap(),
            Some(record.clone())
        );
        assert_eq!(store.list("project_0001").unwrap(), vec![record.clone()]);

        match store.restore("project_0001", &record.id).unwrap() {
            LogicalCodebaseRestoreOutcome::AlreadyActive(active) => {
                assert_eq!(active, record);
            }
            other => panic!("expected AlreadyActive, got {other:?}"),
        }

        assert!(matches!(
            store.restore("project_0001", "logical_codebase_missing"),
            Err(ProductStoreError::NotFound {
                kind: "logical_codebase",
                ..
            })
        ));
    }

    #[test]
    fn logical_codebase_restore_rejects_incomplete_subtree() {
        // Bug3 修复①：摘墓碑前校验子树完整——record.json 缺失、或成员存在而
        // manifest 缺失，均拒绝且墓碑保留（可人工修复后重试）。
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = LogicalCodebaseStore::new(paths.clone());
        let record = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Platform".to_string(),
                    aggregate_root: temp.path().to_path_buf(),
                },
            )
            .unwrap();
        let subtree = paths
            .logical_codebases_root("project_0001")
            .join(&record.id);

        // record.json 缺失。
        store.delete_soft("project_0001", &record.id).unwrap();
        std::fs::remove_file(subtree.join("record.json")).unwrap();
        assert!(matches!(
            store.restore("project_0001", &record.id),
            Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_restore",
                ..
            })
        ));
        assert!(
            subtree.join("tombstone.json").exists(),
            "tombstone must survive a rejected restore"
        );

        // 成员存在而 manifest 缺失（正常首批登记会原子创建 manifest；手工
        // 落成员不落 manifest 模拟子树残缺）。
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let second = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Second".to_string(),
                    aggregate_root: temp.path().to_path_buf(),
                },
            )
            .unwrap();
        LogicalCodebaseStore::for_lc(paths.clone(), &second.id)
            .save_member(
                "project_0001",
                &member_fixture(member_id, RepositoryCheckoutId(Uuid::new_v4())),
            )
            .unwrap();
        store.delete_soft("project_0001", &second.id).unwrap();
        assert!(matches!(
            store.restore("project_0001", &second.id),
            Err(ProductStoreError::InvalidRecord {
                kind: "logical_codebase_restore",
                ..
            })
        ));
        assert!(
            paths
                .logical_codebases_root("project_0001")
                .join(&second.id)
                .join("tombstone.json")
                .exists(),
            "tombstone must survive a rejected restore"
        );
    }

    #[test]
    fn logical_codebase_delete_soft_releases_identity_registry_entries() {
        // Bug3 症状2：delete_soft 同步把成员 source 的注册条目 Active→Released；
        // 条目缺席（空 LC）不报错、零注册表写入。
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = LogicalCodebaseStore::new(paths.clone());
        let record = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Platform".to_string(),
                    aggregate_root: temp.path().to_path_buf(),
                },
            )
            .unwrap();

        // 空 LC 软删：无注册表文件也不失败。
        store.delete_soft("project_0001", &record.id).unwrap();
        assert!(!paths
            .logical_codebase_root("project_0001")
            .join("identity-registry.json")
            .exists());

        // 带成员 LC 软删：条目释放为 Released。
        let registry = crate::product::logical_codebase::IdentityRegistryStore::new(paths.clone());
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let member = member_fixture(member_id, RepositoryCheckoutId(Uuid::new_v4()));
        let source = member.source_identity.clone();
        registry
            .upsert_active(
                "project_0001",
                crate::product::logical_codebase::IdentityRegistryEntry::active(
                    source.clone(),
                    member_id,
                    "repository_test".into(),
                    RepositoryCheckoutId(Uuid::new_v4()),
                    "test:create".into(),
                ),
            )
            .unwrap();
        let second = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Second".to_string(),
                    aggregate_root: temp.path().to_path_buf(),
                },
            )
            .unwrap();
        LogicalCodebaseStore::for_lc(paths.clone(), &second.id)
            .save_member("project_0001", &member)
            .unwrap();

        store.delete_soft("project_0001", &second.id).unwrap();
        let entry = registry
            .find_by_source("project_0001", &source)
            .unwrap()
            .expect("entry must remain present after release");
        assert_eq!(
            entry.state,
            crate::product::logical_codebase::IdentityRegistryState::Released
        );
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

    /// 2026-10-29（owner）：彻底删除（purge）必须先软删——存活 LC 拒绝直删，
    /// 未知 id 仍是 NotFound（删除语义与 delete_soft 一致）。
    #[test]
    fn logical_codebase_purge_requires_tombstone() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let record = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Platform".to_string(),
                    aggregate_root: temp.path().to_path_buf(),
                },
            )
            .unwrap();

        assert!(matches!(
            store.purge("project_0001", &record.id),
            Err(ProductStoreError::Conflict {
                kind: "logical_codebase_not_tombstoned",
                ..
            })
        ));
        // 拒绝后零副作用：LC 仍存活、可正常读取。
        assert_eq!(
            store.get("project_0001", &record.id).unwrap(),
            Some(record.clone())
        );
        assert!(matches!(
            store.purge("project_0001", "logical_codebase_missing"),
            Err(ProductStoreError::NotFound {
                kind: "logical_codebase",
                ..
            })
        ));
    }

    /// 2026-10-29（owner）：软删后 purge → LC 子树（含墓碑）消失、政策正文与
    /// 成员指针消失、Cadence-skills 产物（AGENTS.md/CLAUDE.md/.claude/rules）原样
    /// 保留；第二次 purge → NotFound（与现有删除语义一致）。
    #[test]
    fn logical_codebase_purge_removes_subtree_and_workspace_artifacts_only() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let aggregate_root = temp.path().join("workspace/aggregate");
        let member_repo = temp.path().join("workspace/api");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        std::fs::create_dir_all(&member_repo).unwrap();

        let store = LogicalCodebaseStore::new(paths.clone());
        let record = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Platform".to_string(),
                    aggregate_root: aggregate_root.clone(),
                },
            )
            .unwrap();
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            aggregate_root.clone(),
            vec![member_id],
        );
        let scoped = LogicalCodebaseStore::for_lc(paths.clone(), &record.id);
        scoped.save_manifest("project_0001", &manifest).unwrap();
        scoped
            .save_member("project_0001", &member_fixture(member_id, checkout_id))
            .unwrap();
        scoped
            .save_checkout(
                "project_0001",
                &checkout_fixture_at(member_id, checkout_id, member_repo.clone()),
            )
            .unwrap();

        // aria 产物：政策版本正文 + 成员仓指针。
        let policy_dir = aggregate_root
            .join("policy")
            .join("project_0001")
            .join(manifest.logical_codebase_id.to_string());
        std::fs::create_dir_all(&policy_dir).unwrap();
        std::fs::write(policy_dir.join("1"), "# policy body\n").unwrap();
        let pointer = member_repo.join(".aria-pointer.md");
        std::fs::write(&pointer, "# pointer\n").unwrap();

        // Cadence-skills 产物：purge 不得清理。
        std::fs::write(aggregate_root.join("AGENTS.md"), "# agents\n").unwrap();
        std::fs::write(aggregate_root.join("CLAUDE.md"), "# claude\n").unwrap();
        std::fs::create_dir_all(aggregate_root.join(".claude/rules")).unwrap();
        std::fs::write(
            aggregate_root.join(".claude/rules/language.md"),
            "# rule\n",
        )
        .unwrap();

        store.delete_soft("project_0001", &record.id).unwrap();
        assert_eq!(store.list_deleted("project_0001").unwrap().len(), 1);

        store.purge("project_0001", &record.id).unwrap();

        let subtree = paths.logical_codebases_root("project_0001").join(&record.id);
        assert!(!subtree.exists(), "purged LC subtree must be gone");
        assert_eq!(store.get("project_0001", &record.id).unwrap(), None);
        assert!(store.list_deleted("project_0001").unwrap().is_empty());
        assert!(!store.has_any_storage("project_0001").unwrap());

        assert!(!policy_dir.exists(), "policy body must be purged");
        assert!(!pointer.exists(), "member pointer must be purged");
        assert!(aggregate_root.join("AGENTS.md").exists());
        assert!(aggregate_root.join("CLAUDE.md").exists());
        assert!(aggregate_root.join(".claude/rules/language.md").exists());

        // 重复 purge：已不存在 → NotFound。
        assert!(matches!(
            store.purge("project_0001", &record.id),
            Err(ProductStoreError::NotFound {
                kind: "logical_codebase",
                ..
            })
        ));
    }

    /// 2026-10-29（owner）：purge 只作用于自己 project 下的 LC——别的 project
    /// 看到的是 NotFound，原 LC 的墓碑与子树原样保留（仍可 restore）。
    #[test]
    fn logical_codebase_purge_never_crosses_project_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = LogicalCodebaseStore::new(paths.clone());
        let record = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Platform".to_string(),
                    aggregate_root: temp.path().to_path_buf(),
                },
            )
            .unwrap();
        store.delete_soft("project_0001", &record.id).unwrap();

        assert!(matches!(
            store.purge("project_0002", &record.id),
            Err(ProductStoreError::NotFound {
                kind: "logical_codebase",
                ..
            })
        ));
        assert_eq!(store.list_deleted("project_0001").unwrap()[0].id, record.id);
        match store.restore("project_0001", &record.id).unwrap() {
            LogicalCodebaseRestoreOutcome::Restored(restored) => {
                assert_eq!(restored, record);
            }
            other => panic!("expected Restored, got {other:?}"),
        }
    }

    /// 2026-10-29（owner）：list_deleted 只报墓碑 LC，并带出 deleted_at
    /// （存活 LC 不在其中）。
    #[test]
    fn logical_codebase_list_deleted_reports_tombstones_only() {
        let temp = tempfile::tempdir().unwrap();
        let store = LogicalCodebaseStore::new(ProductAppPaths::new(temp.path()));
        let alive = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Alive".to_string(),
                    aggregate_root: temp.path().join("alive"),
                },
            )
            .unwrap();
        let deleted = store
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "Deleted".to_string(),
                    aggregate_root: temp.path().join("deleted"),
                },
            )
            .unwrap();
        store.delete_soft("project_0001", &deleted.id).unwrap();

        let listed = store.list_deleted("project_0001").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, deleted.id);
        assert_eq!(listed[0].name, "Deleted");
        assert_eq!(listed[0].aggregate_root, deleted.aggregate_root);
        assert!(!listed[0].deleted_at.trim().is_empty());
        assert!(!listed.iter().any(|entry| entry.id == alive.id));
        assert_eq!(store.list_deleted("project_missing").unwrap(), Vec::new());
    }

    /// 2026-10-29（owner）：legacy 别名 LC 的权威数据仍在旧 `logical-codebase/`
    /// 根下；purge 必须一并清掉，否则下次 migrate_legacy 会把它复活。
    #[test]
    fn logical_codebase_purge_of_legacy_alias_does_not_resurrect_it() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let aggregate_root = temp.path().join("workspace/platform");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let legacy_root = paths.logical_codebase_root("project_0001");
        std::fs::create_dir_all(&legacy_root).unwrap();
        std::fs::write(
            legacy_root.join("manifest.json"),
            serde_json::json!({
                "schema_version": 1,
                "project_id": "project_0001",
                "logical_codebase_id": Uuid::new_v4(),
                "provider_context_root": aggregate_root,
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
        let alias = store.migrate_legacy("project_0001").unwrap().unwrap();
        store.delete_soft("project_0001", &alias.id).unwrap();
        store.purge("project_0001", &alias.id).unwrap();

        assert!(!legacy_root.exists(), "legacy root must be purged");
        assert_eq!(store.migrate_legacy("project_0001").unwrap(), None);
        assert!(store.list("project_0001").unwrap().is_empty());
        assert!(store.list_deleted("project_0001").unwrap().is_empty());
    }

    /// 与 [`checkout_fixture`] 同形，但主 checkout 落在给定成员仓路径上
    /// （purge 清理成员指针需要真实目录）。
    fn checkout_fixture_at(
        logical_repository_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        canonical_path: PathBuf,
    ) -> RepositoryCheckoutRecord {
        RepositoryCheckoutRecord {
            canonical_path,
            ..checkout_fixture(logical_repository_id, checkout_id)
        }
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
