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
            Some(existing) if existing == expected_registry => {}
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

    fn backfill_compatibility(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        let mappings = mappings_by_physical_id(journal)?;
        self.backfill_repository_projections(journal, &mappings)?;
        for issue in self.legacy_issues(&journal.project_id)? {
            self.backfill_issue_records(journal, &mappings, &issue)?;
        }
        self.backfill_attempt_snapshots(journal, &mappings)?;

        for index in 0..journal.mappings.len() {
            if journal.mappings[index].compatibility_backfilled {
                continue;
            }
            let physical_repository_id = journal.mappings[index].physical_repository_id.clone();
            journal.mappings[index].compatibility_backfilled = true;
            journal.completed_keys.push(format!(
                "backfill:{}:repository:{physical_repository_id}",
                journal.migration_id
            ));
            touch(journal);
            self.journals.save(&journal.project_id, journal)?;
        }
        journal.phase = IdentityMigrationPhase::DualReadWrite;
        journal.read_mode = Some("dual".to_string());
        journal.last_error = None;
        touch(journal);
        self.journals.save(&journal.project_id, journal)
    }

    fn backfill_repository_projections(
        &self,
        journal: &IdentityMigrationJournal,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let path = self
            .paths
            .project_root(&journal.project_id)
            .join("repos.json");
        if !path.exists() {
            return Ok(());
        }
        let mut repositories: Vec<RepositoryRecord> = read_json(&path)?;
        let mut changed = false;
        for repository in &mut repositories {
            validate_relative_id(&repository.id)?;
            let mapping = mapping_for_physical(mappings, &repository.id)?;
            let expected = (
                Some(mapping.logical_repository_id),
                Some(mapping.primary_checkout_id),
                1,
            );
            if repository.logical_repository_id.is_some()
                || repository.primary_checkout_id.is_some()
                || repository.identity_schema_version != 0
            {
                if (
                    repository.logical_repository_id,
                    repository.primary_checkout_id,
                    repository.identity_schema_version,
                ) != expected
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "repository_projection",
                        id: repository.id.clone(),
                    });
                }
            } else {
                repository.logical_repository_id = expected.0;
                repository.primary_checkout_id = expected.1;
                repository.identity_schema_version = expected.2;
                changed = true;
            }
        }
        if changed {
            write_json(&path, &repositories)?;
        }
        Ok(())
    }

    fn backfill_issue_records(
        &self,
        journal: &IdentityMigrationJournal,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
        issue: &IssueRecord,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&issue.id)?;
        if issue.project_id != journal.project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "issue",
                id: issue.id.clone(),
            });
        }
        let issue_root = self.paths.issue_root(&journal.project_id, &issue.id);
        if let Some(physical_id) = issue.repo_id.as_deref() {
            validate_relative_id(physical_id)?;
            self.write_issue_selection(&issue_root, mapping_for_physical(mappings, physical_id)?)?;
        }
        self.backfill_bindings(&journal.project_id, &issue.id, mappings)?;
        self.backfill_stories(&journal.project_id, &issue.id, mappings)?;
        self.backfill_work_items(&journal.project_id, &issue.id, mappings)?;
        self.backfill_shared_worktree(&journal.project_id, &issue.id, mappings)?;
        self.backfill_repository_profiles(&journal.project_id, &issue.id, mappings)
    }

    fn write_issue_selection(
        &self,
        issue_root: &Path,
        mapping: &RepositoryIdentityMapping,
    ) -> Result<(), ProductStoreError> {
        let path = issue_root.join("codebase-selection.json");
        let expected = IssueCodebaseSelection {
            included: vec![mapping.logical_repository_id],
            focus: vec![mapping.logical_repository_id],
            selection_policy: "explicit".to_string(),
        };
        if path.exists() {
            let existing: IssueCodebaseSelection = read_json(&path)?;
            if existing != expected {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "issue_codebase_selection",
                    id: mapping.physical_repository_id.clone(),
                });
            }
            return Ok(());
        }
        write_json(&path, &expected)
    }

    fn backfill_bindings(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self.paths.issue_root(project_id, issue_id).join("bindings");
        rewrite_json_records::<IssueRuntimeBindingRecord, _>(&root, |binding| {
            validate_relative_id(&binding.id)?;
            let mapping = mapping_for_physical(mappings, &binding.repo_id)?;
            assign_optional_identity(
                &mut binding.logical_repository_id,
                mapping.logical_repository_id,
                "runtime_binding",
                &binding.id,
            )?;
            assign_optional_identity(
                &mut binding.checkout_id,
                mapping.primary_checkout_id,
                "runtime_binding",
                &binding.id,
            )
        })
    }

    fn backfill_stories(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("story-specs");
        let manifest = self.required_manifest(project_id)?;
        rewrite_json_records::<StorySpecRecord, _>(&root, |story| {
            validate_relative_id(&story.id)?;
            let mapping = mapping_for_physical(mappings, &story.repository_id)?;
            assign_optional_identity(
                &mut story.logical_codebase_ref,
                manifest.logical_codebase_id,
                "story_spec",
                &story.id,
            )?;
            assign_vec_identity(
                &mut story.involved_repository_ids,
                vec![mapping.logical_repository_id],
                "story_spec",
                &story.id,
            )?;
            assign_optional_identity(
                &mut story.focus_repository_id,
                mapping.logical_repository_id,
                "story_spec",
                &story.id,
            )
        })
    }

    fn backfill_work_items(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-items");
        rewrite_json_records::<LifecycleWorkItemRecord, _>(&root, |work_item| {
            validate_relative_id(&work_item.id)?;
            let mapping = mapping_for_physical(mappings, &work_item.repository_id)?;
            assign_optional_identity(
                &mut work_item.target_repository_id,
                mapping.logical_repository_id,
                "work_item",
                &work_item.id,
            )
        })
    }

    fn backfill_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let path = self
            .paths
            .issue_root(project_id, issue_id)
            .join("issue-shared-worktree.json");
        if !path.exists() {
            return Ok(());
        }
        let mut worktree: IssueSharedWorktree = read_json(&path)?;
        validate_relative_id(&worktree.id)?;
        let mapping = mapping_for_physical(mappings, &worktree.repository_id)?;
        assign_optional_identity(
            &mut worktree.target_repository_id,
            mapping.logical_repository_id,
            "issue_shared_worktree",
            &worktree.id,
        )?;
        assign_optional_identity(
            &mut worktree.checkout_id,
            mapping.primary_checkout_id,
            "issue_shared_worktree",
            &worktree.id,
        )?;
        if worktree.path_schema_version == 0 {
            worktree.path_schema_version = 1;
        } else if worktree.path_schema_version != 1 {
            return Err(ProductStoreError::InvalidRecord {
                kind: "issue_shared_worktree",
                reason: format!("unsupported path_schema_version for {}", worktree.id),
            });
        }
        write_json(&path, &worktree)
    }

    fn backfill_repository_profiles(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("repository-profiles");
        let manifest = self.required_manifest(project_id)?;
        rewrite_json_records::<RepositoryProfile, _>(&root, |profile| {
            validate_relative_id(&profile.id)?;
            let mapping = mapping_for_physical(mappings, &profile.repository_id)?;
            assign_optional_identity(
                &mut profile.logical_repository_id,
                mapping.logical_repository_id,
                "repository_profile",
                &profile.id,
            )?;
            if profile.membership_revision == 0 {
                profile.membership_revision = manifest.membership_revision;
                Ok(())
            } else if profile.membership_revision == manifest.membership_revision {
                Ok(())
            } else {
                Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_profile",
                    id: profile.id.clone(),
                })
            }
        })
    }

    fn backfill_attempt_snapshots(
        &self,
        journal: &IdentityMigrationJournal,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let manifest = self.required_manifest(&journal.project_id)?;
        for issue in self.legacy_issues(&journal.project_id)? {
            let root = self
                .paths
                .issue_root(&journal.project_id, &issue.id)
                .join("coding-attempts");
            if !root.exists() {
                continue;
            }
            for entry in std::fs::read_dir(&root).map_err(|error| {
                ProductStoreError::Io(format!("read {}: {error}", root.display()))
            })? {
                let path = entry
                    .map_err(|error| {
                        ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
                    })?
                    .path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                    continue;
                }
                let mut attempt: CodingExecutionAttempt = read_json(&path)?;
                self.backfill_attempt_snapshot(journal, mappings, &manifest, &mut attempt)?;
                write_json(&path, &attempt)?;
            }
        }
        Ok(())
    }

    fn backfill_attempt_snapshot(
        &self,
        journal: &IdentityMigrationJournal,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
        manifest: &LogicalCodebaseManifest,
        attempt: &mut CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&attempt.id)?;
        validate_relative_id(&attempt.issue_id)?;
        if attempt.project_id != journal.project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_attempt",
                id: attempt.id.clone(),
            });
        }
        if attempt.target_snapshot.is_some() {
            return Ok(());
        }
        if attempt.status.is_active() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "target_snapshot_missing",
                reason: format!("active legacy attempt {} cannot resume", attempt.id),
            });
        }
        let work_item = self.resolve_attempt_work_item(attempt)?;
        let mapping = mapping_for_physical(mappings, &work_item.repository_id)?;
        let checkout = self
            .authority
            .load_checkout(&journal.project_id, mapping.primary_checkout_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository_checkout",
                id: mapping.primary_checkout_id.0.to_string(),
            })?;
        if checkout.logical_repository_id != mapping.logical_repository_id
            || checkout.physical_repository_id != mapping.physical_repository_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_checkout",
                id: checkout.checkout_id.0.to_string(),
            });
        }
        attempt.target_snapshot = Some(AttemptTargetSnapshot {
            logical_repository_id: mapping.logical_repository_id,
            checkout_id: mapping.primary_checkout_id,
            physical_repository_id: mapping.physical_repository_id.clone(),
            canonical_path: checkout.canonical_path,
            git_dir_identity: checkout.git_dir_identity,
            revision: None,
            policy_digest: manifest.context_policy_digest.clone(),
            membership_revision: manifest.membership_revision,
            captured_at: Utc::now().to_rfc3339(),
            capture_source: "migration_observed".to_string(),
        });
        Ok(())
    }

    fn resolve_attempt_work_item(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let current_work_item_id = match attempt.scope {
            CodingAttemptScope::WorkItem => attempt
                .current_work_item_id
                .as_deref()
                .unwrap_or(&attempt.work_item_id),
            CodingAttemptScope::WorkItemGroup => attempt
                .current_work_item_id
                .as_deref()
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "target_snapshot_missing",
                    reason: format!("group attempt {} has no current_work_item_id", attempt.id),
                })?,
        };
        validate_relative_id(current_work_item_id)?;
        let current =
            self.load_work_item(&attempt.project_id, &attempt.issue_id, current_work_item_id)?;
        if attempt.scope == CodingAttemptScope::WorkItemGroup {
            self.validate_group_attempt_target(attempt, &current)?;
        }
        Ok(current)
    }

    fn validate_group_attempt_target(
        &self,
        attempt: &CodingExecutionAttempt,
        current: &LifecycleWorkItemRecord,
    ) -> Result<(), ProductStoreError> {
        let group_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            ProductStoreError::InvalidRecord {
                kind: "target_snapshot_missing",
                reason: format!("group attempt {} has no group id", attempt.id),
            }
        })?;
        validate_relative_id(group_id)?;
        let root = self
            .paths
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join(&attempt.id);
        let mut work_item_ids = BTreeSet::from([current.id.clone()]);
        for unit in read_json_records::<CodingExecutionUnit>(&root.join("units"))? {
            validate_relative_id(&unit.id)?;
            validate_relative_id(&unit.logical_work_item_id)?;
            if unit.attempt_id != attempt.id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_execution_unit",
                    id: unit.id,
                });
            }
            work_item_ids.insert(unit.logical_work_item_id);
        }
        let binding_path = root.join("plan-binding.json");
        if binding_path.exists() {
            let binding: CodingAttemptPlanBinding = read_json(&binding_path)?;
            if binding.attempt_id != attempt.id || binding.plan_id != group_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_attempt_plan_binding",
                    id: attempt.id.clone(),
                });
            }
        }
        let initialization_path = self
            .paths
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join("group-initializations")
            .join(format!("{group_id}.json"));
        if initialization_path.exists() {
            let initialization: serde_json::Value = read_json(&initialization_path)?;
            let initialized_attempt_id = initialization
                .get("attempt")
                .and_then(|value| value.get("id"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "target_snapshot_missing",
                    reason: format!("group initialization for {} is unresolved", attempt.id),
                })?;
            let initialized_current = initialization
                .get("attempt")
                .and_then(|value| value.get("current_work_item_id"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "target_snapshot_missing",
                    reason: format!("group initialization for {} is unresolved", attempt.id),
                })?;
            if initialized_attempt_id != attempt.id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization",
                    id: attempt.id.clone(),
                });
            }
            validate_relative_id(initialized_current)?;
            work_item_ids.insert(initialized_current.to_string());
        }
        for work_item_id in work_item_ids {
            let candidate =
                self.load_work_item(&attempt.project_id, &attempt.issue_id, &work_item_id)?;
            if candidate.repository_id != current.repository_id {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "target_snapshot_missing",
                    reason: format!("group attempt {} has mixed work item targets", attempt.id),
                });
            }
        }
        Ok(())
    }

    fn switch_reads(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        IdentityMigrationVerifier::new(self.paths.clone()).verify(&journal.project_id)?;
        let manifest = self.required_manifest(&journal.project_id)?;
        journal.phase = IdentityMigrationPhase::SwitchingReads;
        journal.read_mode = Some("logical_authoritative".to_string());
        journal.completed_keys.push(format!(
            "switch:{}:{}:{}",
            journal.migration_id, journal.source_repos_digest, manifest.membership_revision
        ));
        journal.last_error = None;
        touch(journal);
        // The marker is the last migration write: a pre-marker crash remains dual.
        self.journals.save(&journal.project_id, journal)
    }

    fn required_manifest(
        &self,
        project_id: &str,
    ) -> Result<LogicalCodebaseManifest, ProductStoreError> {
        self.authority
            .load_manifest(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "logical_codebase_manifest",
                id: project_id.to_string(),
            })
    }

    fn legacy_issues(&self, project_id: &str) -> Result<Vec<IssueRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let root = self.paths.project_root(project_id).join("issues");
        let mut issues = Vec::new();
        for path in child_json_paths(&root, Some("issue.json"))? {
            let issue: IssueRecord = read_json(&path)?;
            validate_relative_id(&issue.id)?;
            issues.push(issue);
        }
        issues.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(issues)
    }

    fn load_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        let path = self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-items")
            .join(format!("{work_item_id}.json"));
        if !path.exists() {
            return Err(ProductStoreError::NotFound {
                kind: "work_item",
                id: work_item_id.to_string(),
            });
        }
        let work_item: LifecycleWorkItemRecord = read_json(&path)?;
        if work_item.project_id != project_id
            || work_item.issue_id != issue_id
            || work_item.id != work_item_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "work_item",
                id: work_item_id.to_string(),
            });
        }
        Ok(work_item)
    }

    fn load_scanned_repositories(
        &self,
        journal: &mut IdentityMigrationJournal,
    ) -> Result<Vec<RepositoryRecord>, ProductStoreError> {
        let repositories = self.legacy_repositories(&journal.project_id)?;
        let digest = source_repositories_digest(&repositories)?;
        if journal.source_repos_digest != digest {
            return self.fail_identity_mismatch(
                journal,
                "identity_migration_source_repositories",
                journal.project_id.clone(),
            );
        }
        Ok(repositories)
    }

    fn legacy_repositories(
        &self,
        project_id: &str,
    ) -> Result<Vec<RepositoryRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let path = self.paths.project_root(project_id).join("repos.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut repositories: Vec<RepositoryRecord> = read_json(&path)?;
        repositories.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(repositories)
    }

    fn fail_identity_mismatch<T>(
        &self,
        journal: &mut IdentityMigrationJournal,
        kind: &'static str,
        id: String,
    ) -> Result<T, ProductStoreError> {
        journal.phase = IdentityMigrationPhase::Failed;
        journal.last_error = Some(format!("identity mismatch: {kind} {id}"));
        touch(journal);
        self.journals.save(&journal.project_id, journal)?;
        Err(ProductStoreError::IdentityMismatch { kind, id })
    }

    fn failed_migration_error<T>(
        &self,
        journal: &IdentityMigrationJournal,
    ) -> Result<T, ProductStoreError> {
        Err(ProductStoreError::Conflict {
            kind: "identity_migration_failed",
            id: journal.project_id.clone(),
        })
    }
}
