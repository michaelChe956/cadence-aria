impl IdentityMigrationExecutor {
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
            let existing: serde_json::Value = read_json(&path)?;
            if existing.get("schema_version").is_some()
                && existing.get("focus_repository_ids").is_some()
            {
                return Ok(());
            }
            let existing: IssueCodebaseSelection = serde_json::from_value(existing)
                .map_err(|error| ProductStoreError::Json(error.to_string()))?;
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
        if let Err(error) = IdentityMigrationVerifier::new(self.paths.clone()).verify(&journal.project_id)
        {
            journal.phase = IdentityMigrationPhase::Failed;
            journal.last_error = Some(format!("migration verifier failed: {error}"));
            touch(journal);
            self.journals.save(&journal.project_id, journal)?;
            return Err(error);
        }
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
