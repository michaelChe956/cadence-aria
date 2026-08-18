#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::product::logical_codebase::{
        IdentityMigrationJournal, IdentityMigrationJournalStore, IdentityMigrationPhase,
        LogicalCodebaseFeature, LogicalCodebaseManifest,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    struct RepositoryStoreFixture {
        _root: tempfile::TempDir,
        store: RepositoryStore,
        git_root: PathBuf,
    }

    struct ResolutionFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        store: RepositoryStore,
        logical_id: LogicalRepositoryId,
        physical_id: String,
        checkout_id: RepositoryCheckoutId,
        source_identity: RepositorySourceIdentity,
    }

    fn repository_store_fixture_with_feature_enabled() -> RepositoryStoreFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
                multi_repo: false,
            })
            .unwrap();
        let git_root = root.path().join("api");
        fs::create_dir_all(&git_root).unwrap();
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&git_root)
            .status()
            .unwrap();
        assert!(status.success());

        RepositoryStoreFixture {
            _root: root,
            store: RepositoryStore::with_logical_codebase_feature(
                paths,
                LogicalCodebaseFeature::enabled(),
            ),
            git_root,
        }
    }

    fn resolution_fixture() -> ResolutionFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
                multi_repo: false,
            })
            .expect("create project");
        let logical_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let physical_id = "repository_0001".to_string();
        let source_identity = RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: "sha256:resolution-fixture".to_string(),
            canonical_git_dir: PathBuf::from("/workspace/api/.git"),
            canonical_origin: None,
            first_seen_path_hash: "sha256:first-seen".to_string(),
        };
        let fixture = ResolutionFixture {
            _root: root,
            store: RepositoryStore::with_logical_codebase_feature(
                paths.clone(),
                LogicalCodebaseFeature::enabled(),
            ),
            paths,
            logical_id,
            physical_id,
            checkout_id,
            source_identity,
        };
        IdentityRegistryStore::new(fixture.paths.clone())
            .upsert_active(
                "project_0001",
                IdentityRegistryEntry::active(
                    fixture.source_identity.clone(),
                    fixture.logical_id,
                    fixture.physical_id.clone(),
                    fixture.checkout_id,
                    "resolution-fixture".to_string(),
                ),
            )
            .expect("write registry mapping");
        fixture
    }

    impl ResolutionFixture {
        fn write_authority_with_path(&self, path: &str) {
            let authority = LogicalCodebaseStore::new(self.paths.clone());
            authority
                .save_manifest(
                    "project_0001",
                    &LogicalCodebaseManifest::new(
                        "project_0001",
                        PathBuf::from("/workspace"),
                        vec![self.logical_id],
                    ),
                )
                .expect("write manifest");
            authority
                .save_member(
                    "project_0001",
                    &CodebaseMemberRecord {
                        logical_repository_id: self.logical_id,
                        physical_repository_id: self.physical_id.clone(),
                        alias: "api".to_string(),
                        role: "repository".to_string(),
                        ordinal: 1,
                        source_identity: self.source_identity.clone(),
                        repo_type: RepositoryType::Unknown,
                        tech_stack: Vec::new(),
                        owner: None,
                        tags: Vec::new(),
                        default_ref: None,
                        checkout_ids: vec![self.checkout_id],
                        status: MemberStatus::Active,
                        created_at: "2026-08-08T00:00:00Z".to_string(),
                        updated_at: "2026-08-08T00:00:00Z".to_string(),
                    },
                )
                .expect("write member");
            authority
                .save_checkout(
                    "project_0001",
                    &RepositoryCheckoutRecord {
                        checkout_id: self.checkout_id,
                        logical_repository_id: self.logical_id,
                        physical_repository_id: self.physical_id.clone(),
                        kind: CheckoutKind::Main,
                        canonical_path: PathBuf::from(path),
                        checkout_path_hash: "sha256:authority-path".to_string(),
                        git_dir_identity: self.source_identity.git_dir_identity(),
                        revision: None,
                        availability: CheckoutAvailability::Available,
                        observed_at: "2026-08-08T00:00:00Z".to_string(),
                        created_at: "2026-08-08T00:00:00Z".to_string(),
                        updated_at: "2026-08-08T00:00:00Z".to_string(),
                    },
                )
                .expect("write checkout");
        }

        fn write_compatible_repository_projection(&self) {
            write_json(
                &self.store.repos_path("project_0001"),
                &[RepositoryRecord {
                    id: self.physical_id.clone(),
                    project_id: "project_0001".to_string(),
                    name: "api".to_string(),
                    path: PathBuf::from("/workspace/api-projection"),
                    repo_hash: "sha256:projection-path".to_string(),
                    runtime_root: PathBuf::from("/workspace/api-projection/.aria/runtime"),
                    default_policy_preset: "manual-write".to_string(),
                    default_provider_mode: "fake".to_string(),
                    created_at: "2026-08-08T00:00:00Z".to_string(),
                    logical_repository_id: Some(self.logical_id),
                    primary_checkout_id: Some(self.checkout_id),
                    identity_schema_version: 1,
                    updated_at: "2026-08-08T00:00:00Z".to_string(),
                }],
            )
            .expect("write compatibility projection");
        }

        fn remove_authority_member(&self) {
            fs::remove_file(
                self.paths
                    .logical_codebase_root("project_0001")
                    .join("members")
                    .join(format!("{}.json", self.logical_id.0)),
            )
            .expect("remove authority member");
        }

        fn set_read_mode(&self, read_mode: &str) {
            let mut journal = IdentityMigrationJournal::new("project_0001", "fixture");
            journal.phase = IdentityMigrationPhase::Completed;
            journal.read_mode = Some(read_mode.to_string());
            IdentityMigrationJournalStore::new(self.paths.clone())
                .save("project_0001", &journal)
                .expect("save migration journal");
        }
    }

    #[test]
    fn strict_resolver_manifest_missing_is_fail_closed_not_legacy_projection() {
        // B1：逻辑状态调用 strict resolver，manifest 缺失时不得退回 dual-read legacy 投影。
        let fixture = resolution_fixture();
        fixture.write_compatible_repository_projection();
        fixture.set_read_mode("dual");

        assert!(matches!(
            fixture
                .store
                .resolve_logical_repository_strict("project_0001", fixture.logical_id),
            Err(ProductStoreError::NotFound {
                kind: "logical_repository_manifest",
                ..
            })
        ));
    }

    #[test]
    fn strict_resolver_inconsistent_member_is_fail_closed() {
        // B1：checkout 与 member/物理投影不一致时不得退回 legacy projection。
        let fixture = resolution_fixture();
        fixture.write_authority_with_path("/workspace/api-authority");
        fixture.write_compatible_repository_projection();
        fixture.set_read_mode("dual");
        LogicalCodebaseStore::new(fixture.paths.clone())
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id: fixture.checkout_id,
                    logical_repository_id: fixture.logical_id,
                    physical_repository_id: "repository_inconsistent".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: PathBuf::from("/workspace/api-authority"),
                    checkout_path_hash: "sha256:authority-path".to_string(),
                    git_dir_identity: fixture.source_identity.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-08-08T00:00:00Z".to_string(),
                    created_at: "2026-08-08T00:00:00Z".to_string(),
                    updated_at: "2026-08-08T00:00:00Z".to_string(),
                },
            )
            .expect("write inconsistent checkout");

        assert!(matches!(
            fixture
                .store
                .resolve_logical_repository_strict("project_0001", fixture.logical_id),
            Err(ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                ..
            })
        ));
    }

    #[test]
    fn resolve_logical_repository_with_source_logical_authority() {
        let fixture = resolution_fixture();
        fixture.write_authority_with_path("/workspace/api-authority");
        fixture.write_compatible_repository_projection();
        fixture.set_read_mode("dual");

        let (_, checkout, repository, source) = fixture
            .store
            .resolve_logical_repository_with_source("project_0001", fixture.logical_id)
            .expect("authority records must resolve without legacy projection");
        assert_eq!(source, ResolutionSource::LogicalAuthority);
        assert_eq!(checkout.canonical_path, PathBuf::from("/workspace/api-authority"));
        assert_eq!(repository.id, "repository_0001");
    }

    #[test]
    fn missing_authority_member_in_dual_mode_uses_unique_legacy_projection() {
        let fixture = resolution_fixture();
        fixture.write_authority_with_path("/workspace/api-authority");
        fixture.write_compatible_repository_projection();
        fixture.set_read_mode("dual");
        fixture.remove_authority_member();

        let (_, checkout, _, source) = fixture
            .store
            .resolve_logical_repository_with_source("project_0001", fixture.logical_id)
            .expect("unique dual projection must resolve");
        assert_eq!(source, ResolutionSource::LegacyProjection);
        assert_eq!(
            checkout.canonical_path,
            PathBuf::from("/workspace/api-projection")
        );
    }

    #[test]
    fn dual_mode_identity_resolution_is_missing_or_ambiguous_without_guessing() {
        let fixture = resolution_fixture();
        fixture.write_authority_with_path("/workspace/api-authority");
        fixture.write_compatible_repository_projection();
        fixture.set_read_mode("dual");
        fixture.remove_authority_member();

        let missing_id = LogicalRepositoryId(Uuid::new_v4());
        assert!(matches!(
            fixture
                .store
                .resolve_logical_repository("project_0001", missing_id),
            Err(ProductStoreError::NotFound {
                kind: "identity_resolution_missing",
                ..
            })
        ));

        IdentityRegistryStore::new(fixture.paths.clone())
            .upsert_active(
                "project_0001",
                IdentityRegistryEntry::active(
                    RepositorySourceIdentity {
                        key_digest: "sha256:resolution-fixture-duplicate".to_string(),
                        ..fixture.source_identity.clone()
                    },
                    fixture.logical_id,
                    "repository_0002".to_string(),
                    RepositoryCheckoutId(Uuid::new_v4()),
                    "resolution-fixture-duplicate".to_string(),
                ),
            )
            .expect("write duplicate logical mapping");
        assert!(matches!(
            fixture
                .store
                .resolve_logical_repository("project_0001", fixture.logical_id),
            Err(ProductStoreError::Ambiguous {
                kind: "identity_resolution_ambiguous",
                ..
            })
        ));
    }

    #[test]
    fn logical_resolution_prefers_authority_and_allows_single_legacy_fallback_only_in_dual_mode() {
        let fixture = resolution_fixture();
        fixture.write_authority_with_path("/workspace/api-authority");
        fixture.write_compatible_repository_projection();
        fixture.set_read_mode("dual");

        let (_, checkout, physical) = fixture
            .store
            .resolve_logical_repository("project_0001", fixture.logical_id)
            .unwrap();
        assert_eq!(
            checkout.canonical_path,
            std::path::PathBuf::from("/workspace/api-authority")
        );
        assert_eq!(physical.id, "repository_0001");

        let (_, legacy_checkout, legacy_physical) = fixture
            .store
            .resolve_legacy_physical_repository_if_dual("project_0001", "repository_0001")
            .unwrap();
        assert_eq!(
            legacy_checkout.canonical_path,
            std::path::PathBuf::from("/workspace/api-authority")
        );
        assert_eq!(legacy_physical.id, "repository_0001");

        fixture.remove_authority_member();
        let fallback = fixture
            .store
            .resolve_logical_repository("project_0001", fixture.logical_id)
            .unwrap();
        assert_eq!(fallback.2.id, "repository_0001");

        fixture.set_read_mode("logical_authoritative");
        assert!(matches!(
            fixture
                .store
                .resolve_logical_repository("project_0001", fixture.logical_id),
            Err(ProductStoreError::NotFound { .. })
        ));
    }

    #[test]
    fn create_is_idempotent_uses_uuid_physical_id_and_rejects_same_source() {
        let fixture = repository_store_fixture_with_feature_enabled();
        let input = CreateRepositoryInput {
            project_id: "project_0001".into(),
            name: "api".into(),
            path: fixture.git_root.clone(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "register-api-1".into(),
        };

        let first = fixture.store.create(input.clone()).unwrap();
        let replay = fixture.store.create(input).unwrap();

        assert_eq!(first, replay);
        assert!(first.id.starts_with("repository_"));
        assert!(uuid::Uuid::parse_str(first.id.strip_prefix("repository_").unwrap()).is_ok());
        assert!(matches!(
            fixture.store.create(CreateRepositoryInput {
                project_id: "project_0001".into(),
                name: "api".into(),
                path: fixture.git_root.clone(),
                default_policy_preset: Some("automatic".into()),
                default_provider_mode: None,
                idempotency_key: "register-api-1".into(),
            }),
            Err(ProductStoreError::Conflict {
                kind: "idempotency_key_reused",
                ..
            })
        ));
        assert!(matches!(
            fixture.store.create(CreateRepositoryInput {
                project_id: "project_0001".into(),
                name: "api".into(),
                path: fixture.git_root,
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: "register-api-2".into(),
            }),
            Err(ProductStoreError::Conflict {
                kind: "repository_already_registered",
                ..
            })
        ));
    }

    #[test]
    fn repository_initialization_launch_is_nameable_through_repository_store() {
        let _: Option<crate::product::repository_store::RepositoryInitializationLaunch> = None;
    }
}
