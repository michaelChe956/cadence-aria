pub struct IdentityMigrationVerifier {
    paths: ProductAppPaths,
    authority: LogicalCodebaseStore,
}

impl IdentityMigrationVerifier {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self {
            authority: LogicalCodebaseStore::new(paths.clone()),
            paths,
        }
    }

    pub fn verify(&self, project_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        let journal = IdentityMigrationJournalStore::new(self.paths.clone())
            .load(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "identity_migration_journal",
                id: project_id.to_string(),
            })?;
        if journal.read_mode.as_deref() != Some("dual") {
            return Err(ProductStoreError::InvalidRecord {
                kind: "identity_migration_verifier",
                reason: "read_mode must be dual before switching".to_string(),
            });
        }
        if !journal
            .mappings
            .iter()
            .all(|mapping| mapping.authority_written && mapping.compatibility_backfilled)
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "identity_migration_verifier",
                reason: "migration compatibility backfill is incomplete".to_string(),
            });
        }
        let manifest = self.authority.load_manifest(project_id)?.ok_or_else(|| {
            ProductStoreError::NotFound {
                kind: "logical_codebase_manifest",
                id: project_id.to_string(),
            }
        })?;
        let mappings = mappings_by_physical_id(&journal)?;
        let mut expected_members = BTreeSet::new();
        for mapping in mappings.values() {
            expected_members.insert(mapping.logical_repository_id);
            let member = self
                .authority
                .load_member(project_id, mapping.logical_repository_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_codebase_member",
                    id: mapping.logical_repository_id.0.to_string(),
                })?;
            let checkout = self
                .authority
                .load_checkout(project_id, mapping.primary_checkout_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "repository_checkout",
                    id: mapping.primary_checkout_id.0.to_string(),
                })?;
            if member.physical_repository_id != mapping.physical_repository_id
                || !member.checkout_ids.contains(&mapping.primary_checkout_id)
                || checkout.logical_repository_id != mapping.logical_repository_id
                || checkout.physical_repository_id != mapping.physical_repository_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "logical_authority",
                    id: mapping.physical_repository_id.clone(),
                });
            }
        }
        if manifest.member_ids.iter().copied().collect::<BTreeSet<_>>() != expected_members {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_manifest",
                id: project_id.to_string(),
            });
        }
        self.verify_repository_projections(project_id, &mappings)?;
        self.verify_issue_projections(project_id, &manifest, &mappings)?;
        self.verify_attempts(project_id, &manifest, &mappings)
    }

    fn verify_repository_projections(
        &self,
        project_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let path = self.paths.project_root(project_id).join("repos.json");
        if !path.exists() {
            return Ok(());
        }
        let records: Vec<RepositoryRecord> = read_json(&path)?;
        for record in records {
            validate_relative_id(&record.id)?;
            let mapping = mapping_for_physical(mappings, &record.id)?;
            if record.logical_repository_id != Some(mapping.logical_repository_id)
                || record.primary_checkout_id != Some(mapping.primary_checkout_id)
                || record.identity_schema_version != 1
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_projection",
                    id: record.id,
                });
            }
        }
        Ok(())
    }

    fn verify_issue_projections(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        for issue_path in child_json_paths(
            &self.paths.project_root(project_id).join("issues"),
            Some("issue.json"),
        )? {
            let issue: IssueRecord = read_json(&issue_path)?;
            validate_relative_id(&issue.id)?;
            if issue.project_id != project_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "issue",
                    id: issue.id,
                });
            }
            let issue_root = self.paths.issue_root(project_id, &issue.id);
            if let Some(physical_id) = issue.repo_id.as_deref() {
                let mapping = mapping_for_physical(mappings, physical_id)?;
                let selection_path = issue_root.join("codebase-selection.json");
                let selection: IssueCodebaseSelection = read_json(&selection_path)?;
                let expected = IssueCodebaseSelection {
                    included: vec![mapping.logical_repository_id],
                    focus: vec![mapping.logical_repository_id],
                    selection_policy: "explicit".to_string(),
                };
                if selection != expected {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "issue_codebase_selection",
                        id: issue.id,
                    });
                }
            }
            self.verify_bindings(project_id, &issue.id, mappings)?;
            self.verify_stories(project_id, &issue.id, manifest, mappings)?;
            self.verify_work_items(project_id, &issue.id, mappings)?;
            self.verify_shared_worktree(project_id, &issue.id, mappings)?;
            self.verify_repository_profiles(project_id, &issue.id, manifest, mappings)?;
        }
        Ok(())
    }

    fn verify_bindings(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let root = self.paths.issue_root(project_id, issue_id).join("bindings");
        for binding in read_json_records::<IssueRuntimeBindingRecord>(&root)? {
            validate_relative_id(&binding.id)?;
            let mapping = mapping_for_physical(mappings, &binding.repo_id)?;
            if binding.logical_repository_id != Some(mapping.logical_repository_id)
                || binding.checkout_id != Some(mapping.primary_checkout_id)
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "runtime_binding",
                    id: binding.id,
                });
            }
        }
        Ok(())
    }

    fn verify_stories(
        &self,
        project_id: &str,
        issue_id: &str,
        manifest: &LogicalCodebaseManifest,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("story-specs");
        for story in read_json_records::<StorySpecRecord>(&root)? {
            validate_relative_id(&story.id)?;
            let mapping = mapping_for_physical(mappings, &story.repository_id)?;
            if story.logical_codebase_ref != Some(manifest.logical_codebase_id)
                || story.involved_repository_ids != vec![mapping.logical_repository_id]
                || story.focus_repository_id != Some(mapping.logical_repository_id)
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "story_spec",
                    id: story.id,
                });
            }
        }
        Ok(())
    }

    fn verify_work_items(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-items");
        for work_item in read_json_records::<LifecycleWorkItemRecord>(&root)? {
            validate_relative_id(&work_item.id)?;
            let mapping = mapping_for_physical(mappings, &work_item.repository_id)?;
            if work_item.target_repository_id != Some(mapping.logical_repository_id) {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "work_item",
                    id: work_item.id,
                });
            }
        }
        Ok(())
    }

    fn verify_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let path = self
            .paths
            .issue_root(project_id, issue_id)
            .join("issue-shared-worktree.json");
        if !path.exists() {
            return Ok(());
        }
        let worktree: IssueSharedWorktree = read_json(&path)?;
        validate_relative_id(&worktree.id)?;
        let mapping = mapping_for_physical(mappings, &worktree.repository_id)?;
        if worktree.target_repository_id != Some(mapping.logical_repository_id)
            || worktree.checkout_id != Some(mapping.primary_checkout_id)
            || worktree.path_schema_version != 1
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "issue_shared_worktree",
                id: worktree.id,
            });
        }
        Ok(())
    }

    fn verify_repository_profiles(
        &self,
        project_id: &str,
        issue_id: &str,
        manifest: &LogicalCodebaseManifest,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        let root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("repository-profiles");
        for profile in read_json_records::<RepositoryProfile>(&root)? {
            validate_relative_id(&profile.id)?;
            let mapping = mapping_for_physical(mappings, &profile.repository_id)?;
            if profile.logical_repository_id != Some(mapping.logical_repository_id)
                || profile.membership_revision != manifest.membership_revision
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_profile",
                    id: profile.id,
                });
            }
        }
        Ok(())
    }

    fn verify_attempts(
        &self,
        project_id: &str,
        manifest: &LogicalCodebaseManifest,
        mappings: &BTreeMap<String, RepositoryIdentityMapping>,
    ) -> Result<(), ProductStoreError> {
        for issue_path in child_json_paths(
            &self.paths.project_root(project_id).join("issues"),
            Some("issue.json"),
        )? {
            let issue: IssueRecord = read_json(&issue_path)?;
            let root = self
                .paths
                .issue_root(project_id, &issue.id)
                .join("coding-attempts");
            for attempt in read_json_records_shallow::<CodingExecutionAttempt>(&root)? {
                validate_relative_id(&attempt.id)?;
                if attempt.project_id != project_id || attempt.issue_id != issue.id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_attempt",
                        id: attempt.id,
                    });
                }
                let Some(snapshot) = attempt.target_snapshot.as_ref() else {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "target_snapshot_missing",
                        reason: format!(
                            "legacy attempt {} blocks logical-authoritative reads",
                            attempt.id
                        ),
                    });
                };
                let mapping = mapping_for_physical(mappings, &snapshot.physical_repository_id)?;
                let checkout = self
                    .authority
                    .load_checkout(project_id, snapshot.checkout_id)?
                    .ok_or_else(|| ProductStoreError::NotFound {
                        kind: "repository_checkout",
                        id: snapshot.checkout_id.0.to_string(),
                    })?;
                if snapshot.logical_repository_id != mapping.logical_repository_id
                    || snapshot.checkout_id != mapping.primary_checkout_id
                    || checkout.logical_repository_id != snapshot.logical_repository_id
                    || checkout.physical_repository_id != snapshot.physical_repository_id
                    || snapshot.membership_revision != manifest.membership_revision
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "attempt_target_snapshot",
                        id: attempt.id,
                    });
                }
                if attempt.status.is_active() && snapshot.capture_source == "migration_observed" {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "target_snapshot_missing",
                        reason: format!(
                            "active migration-observed attempt {} blocks switch",
                            attempt.id
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}
