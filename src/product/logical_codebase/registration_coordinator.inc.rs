impl LogicalCodebaseRegistrationCoordinator {
    /// R6：多仓 project 防护语义废除——登记链不再携带 project 级 feature 开关
    /// （`logical_codebase_feature_disabled` 不再产出）；可达性由路由层
    /// `require_logical_codebase` 的 404 语义约束。
    pub fn new(paths: ProductAppPaths) -> Self {
        Self {
            paths,
            lc_id: None,
            #[cfg(test)]
            failure_after_completed_items: Arc::new(AtomicUsize::new(usize::MAX)),
        }
    }

    /// Scopes the whole registration chain (snapshot store, batch store,
    /// manifest/member writes) to one logical codebase subtree. The legacy
    /// alias codebase resolves to the legacy project-scoped root, so legacy
    /// endpoint behavior is byte-identical.
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
            #[cfg(test)]
            failure_after_completed_items: Arc::new(AtomicUsize::new(usize::MAX)),
        }
    }

    fn authority_store(&self) -> LogicalCodebaseStore {
        match &self.lc_id {
            Some(lc_id) => LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id.clone()),
            None => LogicalCodebaseStore::new(self.paths.clone()),
        }
    }

    fn batch_store(&self) -> RegistrationBatchStore {
        match &self.lc_id {
            Some(lc_id) => RegistrationBatchStore::for_lc(self.paths.clone(), lc_id.clone()),
            None => RegistrationBatchStore::new(self.paths.clone()),
        }
    }

    /// Persists the caller-confirmed preflight snapshot without attaching any
    /// member. Call [`Self::resume_batch`] to perform the revalidation and
    /// attach work; this keeps confirmation and execution separately durable.
    pub fn submit_confirmed_batch(
        &self,
        input: ConfirmedRegistrationBatchInput,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        if input.candidates.is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "registration_batch",
                reason: "confirmed preflight must contain at least one candidate".to_string(),
            });
        }
        // The submitted root is an invariant of every batch, including a
        // queued one. Check it before writing a receipt so root conflicts
        // never leave an unrelated batch behind.
        self.authority_store()
            .validate_registration_root(&input.project_id, &input.aggregate_root.canonical_path)?;

        let items = input
            .candidates
            .iter()
            .filter_map(|candidate| {
                batch_item_from_candidate(candidate, input.include_needs_attention).transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        if items.is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "registration_batch",
                reason: "confirmed preflight contains no selected registrable candidates"
                    .to_string(),
            });
        }
        let mut source_digests = std::collections::BTreeSet::new();
        if items
            .iter()
            .any(|item| !source_digests.insert(item.source_digest.clone()))
        {
            return Err(ProductStoreError::Conflict {
                kind: "registration_batch_duplicate_source",
                id: input.project_id,
            });
        }

        let canonical_manifest_digest = canonical_manifest_digest(&input.aggregate_root, &items);
        let mut revisions = items
            .iter()
            .map(|item| item.preflight_revision.as_str())
            .collect::<Vec<_>>();
        revisions.sort_unstable();
        let idempotency_key =
            batch_idempotency_key(&input.project_id, &canonical_manifest_digest, &revisions);
        let id = format!("registration_batch_{}", Uuid::new_v4().simple());
        validate_relative_id(&id)?;
        let now = Utc::now().to_rfc3339();
        self.batch_store().create_or_get(RegistrationBatchRecord {
            id,
            project_id: input.project_id,
            idempotency_key,
            aggregate_root: input.aggregate_root.canonical_path,
            status: RegistrationBatchStatus::Queued,
            items,
            retry_count: 0,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    pub fn get_batch(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        self.batch_store().load(project_id, batch_id)
    }

    pub fn cancel_batch(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        self.batch_store().cancel(project_id, batch_id)
    }

    /// Revalidates every unfinished item immediately before registration. The
    /// only operations before `attach_member` are the same read-only probes as
    /// preflight; changed path, Git root, identity, HEAD or worktree state is
    /// made visible as `needs_attention` rather than being silently attached.
    pub fn resume_batch(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(batch_id)?;
        let batches = self.batch_store();
        let initial = batches.load(project_id, batch_id)?;
        if matches!(
            initial.status,
            RegistrationBatchStatus::Cancelled | RegistrationBatchStatus::Completed
        ) {
            return Ok(initial);
        }
        // Reject an incompatible root before making a durable state change, so
        // a caller can receive the stable conflict without stranding a batch
        // in `running`.
        self.authority_store()
            .validate_registration_root(project_id, &initial.aggregate_root)?;
        // Identity drift is batch-scoped. Validate every unfinished item before
        // attaching any member; otherwise a later identity conflict could
        // leave earlier items attached even though the caller receives 409.
        if let Err(error) = self.validate_pending_batch_identities(project_id, &initial) {
            if matches!(initial.status, RegistrationBatchStatus::Running)
                && matches!(
                    &error,
                    ProductStoreError::Conflict {
                        kind: "registration_batch_candidate_identity_changed",
                        ..
                    }
                )
            {
                batches.with_batch_mutation(project_id, batch_id, |batch| {
                    if batch.status == RegistrationBatchStatus::Running {
                        batch.status = RegistrationBatchStatus::PartialFailed;
                        batch.updated_at = Utc::now().to_rfc3339();
                    }
                    Ok(())
                })?;
            }
            return Err(error);
        }
        let (batch, ()) = batches.with_batch_mutation(project_id, batch_id, |batch| {
            if batch.status == RegistrationBatchStatus::Cancelled
                || batch.status == RegistrationBatchStatus::Completed
            {
                return Ok(());
            }
            batch.status = RegistrationBatchStatus::Running;
            batch.updated_at = Utc::now().to_rfc3339();
            batches.save_unlocked(batch)
        })?;
        if matches!(
            batch.status,
            RegistrationBatchStatus::Cancelled | RegistrationBatchStatus::Completed
        ) {
            return Ok(batch);
        }

        // `with_batch_mutation` intentionally releases the lock before I/O
        // that may traverse Git metadata. The running transition arbitrates
        // concurrent callers; a second resume observes Running and receives a
        // deterministic conflict below.
        #[cfg(test)]
        let mut interrupted_for_test = false;
        #[cfg(not(test))]
        let interrupted_for_test = false;
        for index in 0..batch.items.len() {
            let mut current = batches.load(project_id, batch_id)?;
            if current.status == RegistrationBatchStatus::Cancelled {
                return Ok(current);
            }
            if current.status != RegistrationBatchStatus::Running {
                return Err(ProductStoreError::Conflict {
                    kind: "registration_batch_not_running",
                    id: batch_id.to_string(),
                });
            }
            let item = &mut current.items[index];
            if matches!(
                item.status,
                RegistrationItemStatus::Completed
                    | RegistrationItemStatus::Skipped
                    | RegistrationItemStatus::NeedsAttention
            ) {
                continue;
            }

            if self.member_already_attached(project_id, item)? {
                item.status = RegistrationItemStatus::Completed;
                item.failure_reason = None;
                item.retry_count = item.retry_count.saturating_add(1);
                current.updated_at = Utc::now().to_rfc3339();
                batches.with_batch_mutation(project_id, batch_id, |stored| {
                    replace_batch_item(stored, item.clone())?;
                    stored.updated_at = current.updated_at.clone();
                    Ok(())
                })?;
                continue;
            }

            let revalidated =
                self.revalidate_batch_item(project_id, &current.aggregate_root, item)?;
            // Identity drift is a batch-level conflict; revision drift is a
            // recoverable item-level acknowledgement. `revalidate_batch_item`
            // performs the identity comparison before returning the candidate,
            // so this revision check is reached only for the same source.
            if revalidated.preflight_revision != item.preflight_revision {
                item.status = RegistrationItemStatus::NeedsAttention;
                item.failure_reason = Some("preflight_revision_changed".to_string());
                item.retry_count = item.retry_count.saturating_add(1);
                current.updated_at = Utc::now().to_rfc3339();
                batches.with_batch_mutation(project_id, batch_id, |stored| {
                    replace_batch_item(stored, item.clone())?;
                    stored.updated_at = current.updated_at.clone();
                    Ok(())
                })?;
                continue;
            }

            item.retry_count = item.retry_count.saturating_add(1);
            let profile = RepositoryProfileDetector::detect(&item.git_root)?;
            item.repo_type = profile.repo_type.clone();
            item.tech_stack = profile.tech_stack.clone();
            let item_key = batch_item_idempotency_key(batch_id, &item.source_digest);
            match self.attach_member_with_root(
                AttachOnlyRegistrationInput {
                    project_id: project_id.to_string(),
                    alias: item.alias.clone(),
                    role: item.role.clone(),
                    canonical_path: item.canonical_path.clone(),
                    repo_type: profile.repo_type,
                    tech_stack: profile.tech_stack,
                    idempotency_key: item_key,
                },
                &current.aggregate_root,
            ) {
                Ok(_) => {
                    item.status = RegistrationItemStatus::Completed;
                    item.failure_reason = None;
                }
                Err(error) => {
                    item.status = RegistrationItemStatus::Failed;
                    item.failure_reason = Some(batch_failure_reason(&error));
                }
            }
            current.updated_at = Utc::now().to_rfc3339();
            batches.with_batch_mutation(project_id, batch_id, |stored| {
                replace_batch_item(stored, item.clone())?;
                stored.updated_at = current.updated_at.clone();
                Ok(())
            })?;
            #[cfg(test)]
            if item.status == RegistrationItemStatus::Completed
                && self.should_interrupt_after_completed_item()
            {
                interrupted_for_test = true;
                break;
            }
        }

        let (completed, ()) = batches.with_batch_mutation(project_id, batch_id, |stored| {
            if stored.status != RegistrationBatchStatus::Cancelled {
                stored.status = if interrupted_for_test {
                    RegistrationBatchStatus::PartialFailed
                } else {
                    aggregate_batch_status(&stored.items)
                };
                stored.retry_count = stored.retry_count.saturating_add(1);
                stored.updated_at = Utc::now().to_rfc3339();
            }
            Ok(())
        })?;
        Ok(completed)
    }

    fn validate_pending_batch_identities(
        &self,
        project_id: &str,
        batch: &RegistrationBatchRecord,
    ) -> Result<(), ProductStoreError> {
        for item in &batch.items {
            if matches!(
                item.status,
                RegistrationItemStatus::Completed
                    | RegistrationItemStatus::Skipped
                    | RegistrationItemStatus::NeedsAttention
            ) || self.member_already_attached(project_id, item)?
            {
                continue;
            }
            // This probe compares only canonical path, Git root and source
            // identity. Revision/worktree changes remain item-level
            // `NeedsAttention` and are handled by the execution loop.
            self.revalidate_batch_item(project_id, &batch.aggregate_root, item)?;
        }
        Ok(())
    }

    fn member_already_attached(
        &self,
        project_id: &str,
        item: &RegistrationBatchItem,
    ) -> Result<bool, ProductStoreError> {
        let registry = IdentityRegistryStore::new(self.paths.clone());
        let Some(entry) = registry.find_by_source(project_id, &item.source_identity)? else {
            return Ok(false);
        };
        if entry.state != crate::product::logical_codebase::IdentityRegistryState::Active {
            return Ok(false);
        }
        let authority = self.authority_store();
        let member = authority
            .load_member(project_id, entry.logical_repository_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "registration_batch_member_recovery",
                id: item.source_digest.clone(),
            })?;
        if member.physical_repository_id != entry.physical_repository_id
            || member.source_identity != item.source_identity
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "registration_batch_member_recovery",
                id: item.source_digest.clone(),
            });
        }
        let [checkout_id] = member.checkout_ids.as_slice() else {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "registration_batch_member_recovery",
                id: item.source_digest.clone(),
            });
        };
        let checkout = authority
            .load_checkout(project_id, *checkout_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "registration_batch_member_recovery",
                id: item.source_digest.clone(),
            })?;
        if checkout.canonical_path != item.canonical_path
            || checkout.physical_repository_id != entry.physical_repository_id
            || checkout.git_dir_identity != item.source_identity.git_dir_identity()
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "registration_batch_member_recovery",
                id: item.source_digest.clone(),
            });
        }
        Ok(true)
    }

    fn revalidate_batch_item(
        &self,
        project_id: &str,
        aggregate_root: &Path,
        item: &RegistrationBatchItem,
    ) -> Result<RegistrationCandidate, ProductStoreError> {
        let canonical_path = fs::canonicalize(&item.submitted_path).map_err(|_| {
            ProductStoreError::Conflict {
                kind: "registration_batch_candidate_identity_changed",
                id: item.source_digest.clone(),
            }
        })?;
        if !canonical_path.starts_with(aggregate_root) {
            return Err(ProductStoreError::Conflict {
                kind: "registration_batch_candidate_identity_changed",
                id: item.source_digest.clone(),
            });
        }
        let (candidate, evidence) = self.classify_git_candidate(
            project_id,
            item.submitted_path.clone(),
            canonical_path,
            &[],
        )?;
        let Some(evidence) = evidence else {
            return Err(ProductStoreError::Conflict {
                kind: "registration_batch_candidate_identity_changed",
                id: item.source_digest.clone(),
            });
        };
        if candidate.canonical_path.as_deref() != Some(item.canonical_path.as_path())
            || candidate.git_root.as_deref() != Some(item.git_root.as_path())
            || candidate.source_identity.as_ref() != Some(&item.source_identity)
            || evidence.source_key_digest != item.source_identity.key_digest
        {
            return Err(ProductStoreError::Conflict {
                kind: "registration_batch_candidate_identity_changed",
                id: item.source_digest.clone(),
            });
        }
        Ok(candidate)
    }

    #[cfg(test)]
    fn should_interrupt_after_completed_item(&self) -> bool {
        self.failure_after_completed_items
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                if remaining == usize::MAX || remaining == 0 {
                    None
                } else {
                    Some(remaining.saturating_sub(1))
                }
            })
            .is_ok_and(|previous| previous == 1)
    }

    /// Reads a submitted manifest (or discovers child Git directories when the
    /// manifest is empty) and classifies every candidate independently.
    /// This method only invokes read-only Git probes and never changes a
    /// checkout, index, ref, config, or branch.
    pub fn preflight(
        &self,
        input: RegistrationPreflightInput,
    ) -> Result<RegistrationPreflightResult, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        let submitted_paths = if input.paths.is_empty() {
            discover_git_directories(&input.aggregate_root.canonical_path)?
        } else {
            input.paths
        };
        let mut candidates = Vec::with_capacity(submitted_paths.len());
        let mut seen = Vec::new();

        for submitted_path in submitted_paths {
            let (candidate, evidence) = match fs::canonicalize(&submitted_path) {
                Err(_) => (RegistrationCandidate::missing(submitted_path), None),
                Ok(canonical_path)
                    if canonical_path == input.aggregate_root.canonical_path
                        || !canonical_path.starts_with(&input.aggregate_root.canonical_path) =>
                {
                    (
                        RegistrationCandidate::outside_root(submitted_path, canonical_path),
                        None,
                    )
                }
                Ok(canonical_path) => self.classify_git_candidate(
                    &input.project_id,
                    submitted_path,
                    canonical_path,
                    &seen,
                )?,
            };
            if let Some(evidence) = evidence {
                seen.push(evidence);
            }
            candidates.push(candidate);
        }

        Ok(RegistrationPreflightResult {
            project_id: input.project_id,
            aggregate_root: input.aggregate_root,
            candidates,
        })
    }

    fn classify_git_candidate(
        &self,
        project_id: &str,
        submitted_path: PathBuf,
        canonical_path: PathBuf,
        seen: &[GitCandidateEvidence],
    ) -> Result<(RegistrationCandidate, Option<GitCandidateEvidence>), ProductStoreError> {
        if !canonical_path.is_dir() {
            return Ok((
                RegistrationCandidate::new(
                    submitted_path,
                    Some(canonical_path),
                    None,
                    None,
                    RegistrationCandidateState::NonGit,
                    "not_git_repository",
                    None,
                    None,
                ),
                None,
            ));
        }

        let Some(git_root) = git_probe(&canonical_path, &["rev-parse", "--show-toplevel"])? else {
            return Ok((
                RegistrationCandidate::new(
                    submitted_path,
                    Some(canonical_path),
                    None,
                    None,
                    RegistrationCandidateState::NonGit,
                    "not_git_repository",
                    None,
                    None,
                ),
                None,
            ));
        };
        let git_root = fs::canonicalize(git_root.trim()).map_err(|error| {
            ProductStoreError::Io(format!(
                "canonicalize Git root reported for {}: {error}",
                canonical_path.display()
            ))
        })?;
        let git_dir = git_probe(&canonical_path, &["rev-parse", "--git-dir"])?
            .ok_or_else(|| git_probe_inconsistent(&canonical_path, "git_dir"))?;
        let git_dir = PathBuf::from(git_dir.trim());
        let git_dir = if git_dir.is_absolute() {
            git_dir
        } else {
            canonical_path.join(git_dir)
        };
        let canonical_git_dir = fs::canonicalize(&git_dir).map_err(|error| {
            ProductStoreError::Io(format!(
                "canonicalize Git directory {} reported for {}: {error}",
                git_dir.display(),
                canonical_path.display()
            ))
        })?;
        let canonical_origin =
            git_probe(&canonical_path, &["config", "--get", "remote.origin.url"])?.and_then(
                |origin| {
                    let origin = origin.trim();
                    (!origin.is_empty()).then(|| origin.to_string())
                },
            );
        let status = git_probe(&canonical_path, &["status", "--porcelain"])?
            .ok_or_else(|| git_probe_inconsistent(&canonical_path, "status"))?;
        // An unborn repository is still a Git repository. Its absent HEAD is
        // represented by an empty component in the revision digest.
        let head = git_probe(&canonical_path, &["rev-parse", "HEAD"])?;
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &canonical_path,
            canonical_git_dir.clone(),
            canonical_origin,
        );
        let evidence = GitCandidateEvidence {
            git_root: git_root.clone(),
            canonical_git_dir,
            source_key_digest: source_identity.key_digest.clone(),
        };

        let duplicate_reason = if seen.iter().any(|prior| {
            prior.canonical_git_dir == evidence.canonical_git_dir
                || prior.source_key_digest == evidence.source_key_digest
        }) {
            Some("duplicate_source_identity")
        } else if IdentityRegistryStore::new(self.paths.clone())
            .find_by_source(project_id, &source_identity)?
            .is_some()
        {
            Some("already_registered")
        } else {
            None
        };
        let linked_worktree = is_linked_worktree_git_dir(&evidence.canonical_git_dir);
        let nested = seen.iter().any(|prior| {
            git_root.starts_with(&prior.git_root) || prior.git_root.starts_with(&git_root)
        });

        let (state, reason) = if let Some(reason) = duplicate_reason {
            (RegistrationCandidateState::Duplicate, reason)
        } else if linked_worktree {
            (RegistrationCandidateState::Nested, "nested_worktree")
        } else if nested {
            (RegistrationCandidateState::Nested, "nested_repository")
        } else if !status.is_empty() {
            (RegistrationCandidateState::NeedsAttention, "dirty_worktree")
        } else {
            (RegistrationCandidateState::Eligible, "eligible")
        };

        Ok((
            RegistrationCandidate::new(
                submitted_path,
                Some(canonical_path),
                Some(git_root),
                Some(source_identity),
                state,
                reason,
                head.as_deref().map(str::trim),
                Some(&status),
            ),
            Some(evidence),
        ))
    }

    pub fn attach_member(
        &self,
        input: AttachOnlyRegistrationInput,
    ) -> Result<CodebaseMemberRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.idempotency_key)?;
        self.attach_member_inner(input)
    }

    fn attach_member_with_root(
        &self,
        input: AttachOnlyRegistrationInput,
        aggregate_root: &Path,
    ) -> Result<CodebaseMemberRecord, ProductStoreError> {
        let store = self.authority_store();
        let project_id = input.project_id.clone();
        store.with_registration_manifest_writer(
            &project_id,
            aggregate_root,
            || self.attach_member_inner(input),
        )
    }

    /// Attaches one confirmed member directly through the LC-scoped authority
    /// store (v1.3 concern③ 处置): the registration chain never routes through
    /// the feature-enabled `RepositoryStore::create`, which performs the lazy
    /// identity migration and rewrites `repos.json` as a compatibility
    /// projection. Identity allocation, member/checkout/registry records and
    /// the manifest membership advance land in the logical-codebase subtree;
    /// `repos.json` stays byte-identical across the whole chain.
    fn attach_member_inner(
        &self,
        input: AttachOnlyRegistrationInput,
    ) -> Result<CodebaseMemberRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.idempotency_key)?;
        let canonical_path = canonicalize_repo_path(&input.canonical_path)?;
        let source_identity = resolve_repository_source(&canonical_path)?;
        let store = self.authority_store();
        let registry = IdentityRegistryStore::new(self.paths.clone());
        let project_id = input.project_id.clone();

        // The registry is project-scoped while the manifest/member writer
        // lock (`attach_member_with_root`) is per-LC. Two codebases
        // registering the same physical repository would otherwise
        // read-modify-write identity-registry.json under different per-LC
        // locks and could both allocate an identity, breaking the cross-LC
        // duplicate-registration guard. Serialize the registry read-modify-
        // write (and the member/checkout allocation that depends on it) on
        // the project-scoped identity migration lock, which also serializes
        // against the lazy identity migration executor.
        with_exact_exclusive_lock(&self.paths.identity_migration_lock_path(&project_id), || {
            let mut member = match registry.find_by_source(&project_id, &source_identity)? {
                Some(entry) if entry.state == IdentityRegistryState::Active => {
                    if entry.created_by_key != input.idempotency_key {
                        return Err(ProductStoreError::Conflict {
                            kind: "repository_already_registered",
                            id: entry.physical_repository_id,
                        });
                    }
                    store
                        .load_member(&project_id, entry.logical_repository_id)?
                        .ok_or_else(|| ProductStoreError::IdentityMismatch {
                            kind: "logical_codebase_member",
                            id: entry.logical_repository_id.0.to_string(),
                        })?
                }
                Some(entry) => {
                    return Err(ProductStoreError::Conflict {
                        kind: "repository_source_tombstoned",
                        id: entry.physical_repository_id,
                    });
                }
                None => {
                    // Crash-window recovery: a member may exist without a registry
                    // entry. Reuse its identity instead of allocating a second one.
                    let matching = store
                        .list_members(&project_id)?
                        .into_iter()
                        .filter(|member| member.source_identity == source_identity)
                        .collect::<Vec<_>>();
                    if matching.len() > 1 {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "logical_codebase_member",
                            id: source_identity.key_digest.clone(),
                        });
                    }
                    match matching.into_iter().next() {
                        Some(member) => {
                            // Heal the crash window: the durable member exists
                            // without a registry entry; complete the authority
                            // records with the caller's idempotency key.
                            let [primary_checkout_id] = member.checkout_ids.as_slice() else {
                                return Err(ProductStoreError::IdentityMismatch {
                                    kind: "logical_codebase_member",
                                    id: member.logical_repository_id.0.to_string(),
                                });
                            };
                            registry.upsert_active(
                                &project_id,
                                IdentityRegistryEntry::active(
                                    source_identity.clone(),
                                    member.logical_repository_id,
                                    member.physical_repository_id.clone(),
                                    *primary_checkout_id,
                                    input.idempotency_key.clone(),
                                ),
                            )?;
                            member
                        }
                        None => self.allocate_and_attach_member(
                            &store,
                            &project_id,
                            &input,
                            &canonical_path,
                            &source_identity,
                        )?,
                    }
                }
            };

            member.alias = input.alias;
            member.role = input.role;
            member.repo_type = input.repo_type;
            member.tech_stack = input.tech_stack;
            member.updated_at = Utc::now().to_rfc3339();
            store.save_member(&project_id, &member)?;
            self.ensure_manifest_membership(&store, &project_id, member.logical_repository_id)?;
            Ok(member)
        })
    }

    fn allocate_and_attach_member(
        &self,
        store: &LogicalCodebaseStore,
        project_id: &str,
        input: &AttachOnlyRegistrationInput,
        canonical_path: &Path,
        source_identity: &RepositorySourceIdentity,
    ) -> Result<CodebaseMemberRecord, ProductStoreError> {
        let ordinal = store
            .list_members(project_id)?
            .iter()
            .map(|member| member.ordinal)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "logical_codebase_member",
                reason: "repository ordinal overflow".to_string(),
            })?;
        let now = Utc::now().to_rfc3339();
        let physical_repository_id = format!("repository_{}", Uuid::new_v4().simple());
        validate_relative_id(&physical_repository_id)?;
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let primary_checkout_id = RepositoryCheckoutId(Uuid::new_v4());

        let member = CodebaseMemberRecord {
            logical_repository_id,
            physical_repository_id,
            alias: input.alias.clone(),
            role: input.role.clone(),
            ordinal,
            source_identity: source_identity.clone(),
            repo_type: input.repo_type.clone(),
            tech_stack: input.tech_stack.clone(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![primary_checkout_id],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        store.save_member(project_id, &member)?;
        store.save_checkout(
            project_id,
            &RepositoryCheckoutRecord {
                checkout_id: primary_checkout_id,
                logical_repository_id,
                physical_repository_id: member.physical_repository_id.clone(),
                kind: CheckoutKind::Main,
                canonical_path: canonical_path.to_path_buf(),
                checkout_path_hash: repo_hash_for_path(canonical_path.to_string_lossy().as_ref()),
                git_dir_identity: source_identity.git_dir_identity(),
                revision: None,
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            },
        )?;
        IdentityRegistryStore::new(self.paths.clone()).upsert_active(
            project_id,
            IdentityRegistryEntry::active(
                source_identity.clone(),
                logical_repository_id,
                member.physical_repository_id.clone(),
                primary_checkout_id,
                input.idempotency_key.clone(),
            ),
        )?;
        Ok(member)
    }

    fn ensure_manifest_membership(
        &self,
        store: &LogicalCodebaseStore,
        project_id: &str,
        member_id: LogicalRepositoryId,
    ) -> Result<(), ProductStoreError> {
        let mut manifest = store
            .load_manifest(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "logical_codebase_manifest",
                id: project_id.to_string(),
            })?;
        if manifest.member_ids.contains(&member_id) {
            return Ok(());
        }
        manifest.member_ids.push(member_id);
        manifest.membership_revision = manifest
            .membership_revision
            .checked_add(1)
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "logical_codebase_manifest",
                reason: "membership_revision overflow".to_string(),
            })?;
        manifest.updated_at = Utc::now().to_rfc3339();
        store.save_manifest(project_id, &manifest)
    }
}

fn is_linked_worktree_git_dir(canonical_git_dir: &Path) -> bool {
    canonical_git_dir
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "worktrees")
}

fn batch_item_from_candidate(
    candidate: &RegistrationCandidate,
    include_needs_attention: bool,
) -> Result<Option<RegistrationBatchItem>, ProductStoreError> {
    let selected = match candidate.state {
        RegistrationCandidateState::Eligible => true,
        RegistrationCandidateState::NeedsAttention => include_needs_attention,
        _ => false,
    };
    if !selected {
        return Ok(None);
    }
    let canonical_path =
        candidate
            .canonical_path
            .clone()
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "confirmed_registration_candidate",
                reason: "selected candidate is missing a canonical path".to_string(),
            })?;
    let git_root = candidate
        .git_root
        .clone()
        .ok_or_else(|| ProductStoreError::InvalidRecord {
            kind: "confirmed_registration_candidate",
            reason: "selected candidate is missing a Git root".to_string(),
        })?;
    let source_identity =
        candidate
            .source_identity
            .clone()
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "confirmed_registration_candidate",
                reason: "selected candidate is missing a source identity".to_string(),
            })?;
    Ok(Some(RegistrationBatchItem {
        source_digest: source_identity.key_digest.clone(),
        submitted_path: candidate.submitted_path.clone(),
        canonical_path,
        git_root,
        source_identity,
        preflight_revision: candidate.preflight_revision.clone(),
        alias: stable_alias(candidate),
        role: "repository".to_string(),
        repo_type: RepositoryType::Unknown,
        tech_stack: Vec::new(),
        status: RegistrationItemStatus::Pending,
        failure_reason: None,
        retry_count: 0,
    }))
}

fn canonical_manifest_digest(
    aggregate_root: &CanonicalAggregateRoot,
    items: &[RegistrationBatchItem],
) -> String {
    let mut sources = items
        .iter()
        .map(|item| item.source_digest.as_str())
        .collect::<Vec<_>>();
    sources.sort_unstable();
    sha256_key(format!(
        "{}\0{}",
        aggregate_root.canonical_path.to_string_lossy(),
        sources.join("\0")
    ))
}

fn batch_idempotency_key(
    project_id: &str,
    canonical_manifest_digest: &str,
    sorted_revisions: &[&str],
) -> String {
    sha256_key(format!(
        "{}\0{}\0{}",
        project_id,
        canonical_manifest_digest,
        sorted_revisions.join("\0")
    ))
}

fn batch_item_idempotency_key(batch_id: &str, source_digest: &str) -> String {
    format!("batch:{batch_id}:item:{source_digest}")
}

fn batch_failure_reason(error: &ProductStoreError) -> String {
    match error {
        ProductStoreError::Conflict { kind, .. }
        | ProductStoreError::NotFound { kind, .. }
        | ProductStoreError::Ambiguous { kind, .. }
        | ProductStoreError::IdentityMismatch { kind, .. }
        | ProductStoreError::InvalidRecord { kind, .. } => (*kind).to_string(),
        ProductStoreError::Io(_) => "product_store_io".to_string(),
        ProductStoreError::Json(_) => "product_store_json".to_string(),
        ProductStoreError::PathEscape(_) => "product_store_path_escape".to_string(),
    }
}

fn replace_batch_item(
    batch: &mut RegistrationBatchRecord,
    replacement: RegistrationBatchItem,
) -> Result<(), ProductStoreError> {
    let Some(item) = batch
        .items
        .iter_mut()
        .find(|item| item.source_digest == replacement.source_digest)
    else {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "registration_batch_item",
            id: replacement.source_digest,
        });
    };
    *item = replacement;
    Ok(())
}

