pub struct IdentityMigrationExecutor {
    paths: ProductAppPaths,
    journals: IdentityMigrationJournalStore,
    authority: LogicalCodebaseStore,
    registry: IdentityRegistryStore,
    fault_injector: Arc<dyn MigrationFaultInjector>,
}

impl IdentityMigrationExecutor {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self::with_fault_injector(paths, Arc::new(NoopMigrationFaultInjector))
    }

    pub fn with_fault_injector(
        paths: ProductAppPaths,
        fault_injector: Arc<dyn MigrationFaultInjector>,
    ) -> Self {
        Self {
            journals: IdentityMigrationJournalStore::new(paths.clone()),
            authority: LogicalCodebaseStore::new(paths.clone()),
            registry: IdentityRegistryStore::new(paths.clone()),
            paths,
            fault_injector,
        }
    }

    /// Runs the complete identity schema migration through the logical-authoritative
    /// read switch. The switch marker is persisted only after verification succeeds.
    pub fn ensure_identity_schema(&self, project_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        let lock_path = self.paths.identity_migration_lock_path(project_id);
        with_exact_exclusive_lock(&lock_path, || {
            let mut journal = self.load_or_begin_scanning(project_id)?;
            match journal.phase {
                IdentityMigrationPhase::Scanning => self.scan_legacy_repositories(&mut journal)?,
                IdentityMigrationPhase::Failed => return self.failed_migration_error(&journal),
                _ => {}
            }
            if journal.phase == IdentityMigrationPhase::Mapping {
                self.persist_mappings_from_source_identity(&mut journal)?;
            }
            if journal.phase == IdentityMigrationPhase::WritingAuthority {
                self.write_authority_records(&mut journal)?;
            }
            if journal.phase == IdentityMigrationPhase::BackfillingCompatibility {
                self.backfill_compatibility(&mut journal)?;
            }
            match journal.phase {
                IdentityMigrationPhase::DualReadWrite => self.switch_reads(&mut journal)?,
                IdentityMigrationPhase::SwitchingReads
                | IdentityMigrationPhase::LegacyFallbackRemoved
                | IdentityMigrationPhase::Completed => {}
                IdentityMigrationPhase::Failed => return self.failed_migration_error(&journal),
                phase => {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "identity_migration_phase",
                        reason: format!(
                            "unsupported migration phase after authority migration: {phase:?}"
                        ),
                    });
                }
            }
            Ok(())
        })
    }

    /// Runs discovery, source mapping, and authority writes. Later migration
    /// stages intentionally start from `BackfillingCompatibility`.
    pub fn ensure_through_authority(&self, project_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        let lock_path = self.paths.identity_migration_lock_path(project_id);
        with_exact_exclusive_lock(&lock_path, || {
            let mut journal = self.load_or_begin_scanning(project_id)?;
            match journal.phase {
                IdentityMigrationPhase::Scanning => self.scan_legacy_repositories(&mut journal)?,
                IdentityMigrationPhase::Failed => return self.failed_migration_error(&journal),
                _ => {}
            }
            if journal.phase == IdentityMigrationPhase::Mapping {
                self.persist_mappings_from_source_identity(&mut journal)?;
            }
            match journal.phase {
                IdentityMigrationPhase::WritingAuthority => {
                    self.write_authority_records(&mut journal)?
                }
                IdentityMigrationPhase::BackfillingCompatibility
                | IdentityMigrationPhase::DualReadWrite
                | IdentityMigrationPhase::SwitchingReads
                | IdentityMigrationPhase::LegacyFallbackRemoved
                | IdentityMigrationPhase::Completed => {}
                IdentityMigrationPhase::Failed => return self.failed_migration_error(&journal),
                phase => {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "identity_migration_phase",
                        reason: format!("unsupported migration phase through authority: {phase:?}"),
                    });
                }
            }
            Ok(())
        })
    }

    fn load_or_begin_scanning(
        &self,
        project_id: &str,
    ) -> Result<IdentityMigrationJournal, ProductStoreError> {
        if let Some(journal) = self.journals.load(project_id)? {
            return Ok(journal);
        }

        let journal = IdentityMigrationJournal::new(project_id, "");
        self.journals.save(project_id, &journal)?;
        Ok(journal)
    }

    fn scan_legacy_repositories(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        let repositories = self.legacy_repositories(&journal.project_id)?;
        if let Some(duplicate_id) = duplicate_repository_id(&repositories) {
            return self.fail_identity_mismatch(journal, "legacy_repository", duplicate_id);
        }

        journal.source_repos_digest = source_repositories_digest(&repositories)?;
        journal.phase = IdentityMigrationPhase::Mapping;
        journal.last_error = None;
        touch(journal);
        self.journals.save(&journal.project_id, journal)
    }

    fn persist_mappings_from_source_identity(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        let repositories = self.load_scanned_repositories(journal)?;
        if let Some(duplicate_id) = duplicate_mapping_legacy_id(&journal.mappings) {
            return self.fail_identity_mismatch(
                journal,
                "identity_migration_mapping",
                duplicate_id,
            );
        }

        for repository in &repositories {
            let source_identity = repository_source_identity(repository)?;
            if let Some(mapping) = journal
                .mappings
                .iter()
                .find(|mapping| mapping.legacy_repository_id == repository.id)
            {
                if mapping.source_identity_digest != source_identity.key_digest
                    || mapping.physical_repository_id != repository.id
                    || mapping.idempotency_key
                        != mapping_idempotency_key(
                            &journal.project_id,
                            &repository.id,
                            &source_identity.key_digest,
                        )
                {
                    return self.fail_identity_mismatch(
                        journal,
                        "identity_migration_mapping",
                        repository.id.clone(),
                    );
                }
                continue;
            }

            let idempotency_key = mapping_idempotency_key(
                &journal.project_id,
                &repository.id,
                &source_identity.key_digest,
            );
            let (logical_repository_id, primary_checkout_id) = match self
                .registry
                .find_by_source(&journal.project_id, &source_identity)?
            {
                Some(entry) if entry.state == IdentityRegistryState::Active => {
                    if entry.physical_repository_id != repository.id {
                        return self.fail_identity_mismatch(
                            journal,
                            "identity_registry",
                            repository.id.clone(),
                        );
                    }
                    (entry.logical_repository_id, entry.primary_checkout_id)
                }
                Some(_) => {
                    return self.fail_identity_mismatch(
                        journal,
                        "identity_registry",
                        repository.id.clone(),
                    );
                }
                None => (
                    LogicalRepositoryId(Uuid::new_v4()),
                    RepositoryCheckoutId(Uuid::new_v4()),
                ),
            };

            journal.mappings.push(RepositoryIdentityMapping {
                legacy_repository_id: repository.id.clone(),
                source_identity_digest: source_identity.key_digest,
                logical_repository_id,
                primary_checkout_id,
                physical_repository_id: repository.id.clone(),
                idempotency_key,
                authority_written: false,
                compatibility_backfilled: false,
            });
            touch(journal);
            // The generated UUIDs become durable before the next allocation.
            self.journals.save(&journal.project_id, journal)?;
        }

        if journal.mappings.len() != repositories.len() {
            return self.fail_identity_mismatch(
                journal,
                "identity_migration_mapping",
                journal.project_id.clone(),
            );
        }
        journal.phase = IdentityMigrationPhase::WritingAuthority;
        journal.last_error = None;
        touch(journal);
        self.journals.save(&journal.project_id, journal)
    }

    fn write_authority_records(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        let repositories = self.load_scanned_repositories(journal)?;
        let inputs = self.authority_inputs(journal, &repositories)?;
        let member_ids = inputs
            .iter()
            .map(|input| input.mapping.logical_repository_id)
            .collect::<Vec<_>>();
        if duplicate_logical_repository_id(&member_ids).is_some() {
            return self.fail_identity_mismatch(
                journal,
                "identity_migration_mapping",
                journal.project_id.clone(),
            );
        }

        self.ensure_manifest(journal, &inputs, member_ids)?;
        for input in inputs {
            self.ensure_authority_for_mapping(journal, input)?;
        }

        // WP0 bootstrap 政策闭环:在 authority 写出后、切换读路径前,确保存在
        // bootstrap AggregatePolicyArtifact。消除「WP2 provider 启动需 envelope、
        // envelope 需 policy artifact」的 bootstrap 环。ensure_bootstrap 幂等,
        // 相同 identity 无副作用;identity 不匹配返回 IdentityMismatch 不覆盖。
        let manifest = self.required_manifest(&journal.project_id)?;
        AggregatePolicyArtifactStore::new(self.paths.clone()).ensure_bootstrap(&manifest)?;

        journal.phase = IdentityMigrationPhase::BackfillingCompatibility;
        journal.last_error = None;
        touch(journal);
        self.journals.save(&journal.project_id, journal)
    }

    fn authority_inputs(
        &self,
        journal: &mut IdentityMigrationJournal,
        repositories: &[RepositoryRecord],
    ) -> Result<Vec<AuthorityInput>, ProductStoreError> {
        let mut inputs = Vec::with_capacity(repositories.len());
        for (index, repository) in repositories.iter().enumerate() {
            let mapping = match journal
                .mappings
                .iter()
                .find(|mapping| mapping.legacy_repository_id == repository.id)
            {
                Some(mapping) => mapping.clone(),
                None => {
                    return self.fail_identity_mismatch(
                        journal,
                        "identity_migration_mapping",
                        repository.id.clone(),
                    );
                }
            };
            let source_identity = repository_source_identity(repository)?;
            if mapping.source_identity_digest != source_identity.key_digest
                || mapping.physical_repository_id != repository.id
                || mapping.idempotency_key
                    != mapping_idempotency_key(
                        &journal.project_id,
                        &repository.id,
                        &source_identity.key_digest,
                    )
            {
                return self.fail_identity_mismatch(
                    journal,
                    "identity_migration_mapping",
                    repository.id.clone(),
                );
            }
            let ordinal =
                u32::try_from(index + 1).map_err(|_| ProductStoreError::InvalidRecord {
                    kind: "identity_migration_mapping",
                    reason: "repository ordinal exceeds u32".to_string(),
                })?;
            let canonical_path = canonicalize_repository_path(&repository.path)?;
            inputs.push(AuthorityInput {
                repository: repository.clone(),
                mapping,
                source_identity,
                canonical_path,
                ordinal,
            });
        }
        Ok(inputs)
    }

    fn ensure_manifest(
        &self,
        journal: &mut IdentityMigrationJournal,
        inputs: &[AuthorityInput],
        member_ids: Vec<LogicalRepositoryId>,
    ) -> Result<(), ProductStoreError> {
        let provider_context_root = common_non_git_parent(inputs)
            .unwrap_or_else(|| self.paths.project_root(&journal.project_id));
        match self.authority.load_manifest(&journal.project_id)? {
            Some(manifest)
                if manifest.schema_version == 1
                    && manifest.project_id == journal.project_id
                    && manifest.layout == LogicalCodebaseLayout::CommonNonGitParent
                    && manifest.provider_context_root == provider_context_root
                    && manifest.member_ids == member_ids =>
            {
                Ok(())
            }
            // Registration creates the aggregate-root manifest before the
            // first repository enters identity migration. With no legacy
            // inputs yet, preserve that root; membership is added by the
            // repository create transaction below.
            Some(manifest) if manifest.member_ids.is_empty() && inputs.is_empty() => Ok(()),
            Some(_) => self.fail_identity_mismatch(
                journal,
                "logical_codebase_manifest",
                journal.project_id.clone(),
            ),
            None => {
                let manifest = LogicalCodebaseManifest::new(
                    &journal.project_id,
                    provider_context_root,
                    member_ids,
                );
                self.authority.save_manifest(&journal.project_id, &manifest)
            }
        }
    }

    fn ensure_authority_for_mapping(
        &self,
        journal: &mut IdentityMigrationJournal,
        input: AuthorityInput,
    ) -> Result<(), ProductStoreError> {
        let member = expected_member(&input);
        match self
            .authority
            .load_member(&journal.project_id, input.mapping.logical_repository_id)?
        {
            Some(existing) if existing == member => {}
            Some(_) => {
                return self.fail_identity_mismatch(
                    journal,
                    "logical_codebase_member",
                    input.mapping.logical_repository_id.0.to_string(),
                );
            }
            None => self.authority.save_member(&journal.project_id, &member)?,
        }

        let checkout = expected_checkout(&input);
        match self
            .authority
            .load_checkout(&journal.project_id, input.mapping.primary_checkout_id)?
        {
            Some(existing) if existing == checkout => {}
            Some(_) => {
                return self.fail_identity_mismatch(
                    journal,
                    "repository_checkout",
                    input.mapping.primary_checkout_id.0.to_string(),
                );
            }
            None => self
                .authority
                .save_checkout(&journal.project_id, &checkout)?,
        }

        let expected_registry = IdentityRegistryEntry::active(
            input.source_identity.clone(),
            input.mapping.logical_repository_id,
            input.mapping.physical_repository_id.clone(),
            input.mapping.primary_checkout_id,
            input.mapping.idempotency_key.clone(),
        );
        match self
            .registry
            .find_by_source(&journal.project_id, &input.source_identity)?
        {
            // C4 Task 7：Active 条目按身份相关字段比对——tombstone→reactivate
            // 复活后的生命周期审计戳（deleted_at/reactivated_at）不参与身份
            // 判定，否则经 repair 复活的原链永远无法通过复验。
            Some(existing)
                if existing.state == IdentityRegistryState::Active
                    && existing.source_identity == expected_registry.source_identity
                    && existing.logical_repository_id
                        == expected_registry.logical_repository_id
                    && existing.primary_checkout_id
                        == expected_registry.primary_checkout_id
                    && existing.physical_repository_id
                        == expected_registry.physical_repository_id
                    && existing.created_by_key == expected_registry.created_by_key => {}
            Some(_) => {
                return self.fail_identity_mismatch(
                    journal,
                    "identity_registry",
                    input.mapping.physical_repository_id.clone(),
                );
            }
            None => self
                .registry
                .upsert_active(&journal.project_id, expected_registry)?,
        }

        let mapping_index = journal
            .mappings
            .iter()
            .position(|mapping| mapping.legacy_repository_id == input.mapping.legacy_repository_id)
            .expect("authority input must have a journal mapping");
        if !journal.mappings[mapping_index].authority_written {
            journal.mappings[mapping_index].authority_written = true;
            touch(journal);
            // The marker is persisted after all authority files and before a
            // failpoint can simulate an abrupt process exit.
            self.journals.save(&journal.project_id, journal)?;
        }
        let persisted_mapping = journal.mappings[mapping_index].clone();
        self.fault_injector
            .after_authority_write(&journal.project_id, &persisted_mapping)
    }
}
