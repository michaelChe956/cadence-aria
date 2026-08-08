impl RepositoryStore {
    pub fn create(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        if !self.logical_codebase_feature.is_enabled() {
            return self.create_legacy(input);
        }

        self.ensure_identity_schema(&input.project_id)?;
        let project_id = input.project_id.clone();
        let lock_path = self.paths.identity_migration_lock_path(&project_id);
        with_exact_exclusive_lock(&lock_path, || self.create_logical_repository(input))
    }

    fn create_legacy(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        let project_id = input.project_id;
        let mut repositories = self.list(&project_id)?;
        let existing_len = repositories.len();
        let id = next_sequential_id("repository", existing_len);
        let now = Utc::now().to_rfc3339();
        let canonical_path = canonicalize_repo_path(&input.path)?;
        let repo_path_text = canonical_path.to_string_lossy();
        let repository = RepositoryRecord {
            id: id.clone(),
            project_id: project_id.clone(),
            name: input.name,
            repo_hash: repo_hash_for_path(repo_path_text.as_ref()),
            runtime_root: canonical_path.join(".aria/runtime"),
            path: canonical_path,
            default_policy_preset: input
                .default_policy_preset
                .unwrap_or_else(|| "manual-write".to_string()),
            default_provider_mode: input
                .default_provider_mode
                .unwrap_or_else(|| "fake".to_string()),
            created_at: now.clone(),
            updated_at: now,
            logical_repository_id: None,
            primary_checkout_id: None,
            identity_schema_version: 0,
        };

        repositories.push(repository.clone());
        write_json(&self.repos_path(&project_id), &repositories)?;
        Ok(repository)
    }

    fn create_logical_repository(
        &self,
        input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.idempotency_key)?;
        let canonical_path = canonicalize_repo_path(&input.path)?;
        let source_identity = resolve_repository_source(&canonical_path)?;
        let receipts = RepositoryCommandReceiptStore::new(self.paths.clone());
        let receipt = receipts.find_create(&input.project_id, &input.idempotency_key)?;
        if let Some(receipt) = receipt.as_ref() {
            receipt.validate_input(&input, &canonical_path)?;
        }

        let registry = IdentityRegistryStore::new(self.paths.clone());
        let authority = LogicalCodebaseStore::new(self.paths.clone());
        let identity = match registry.find_by_source(&input.project_id, &source_identity)? {
            Some(entry) if entry.state == IdentityRegistryState::Active => {
                if entry.created_by_key != input.idempotency_key {
                    return Err(ProductStoreError::Conflict {
                        kind: "repository_already_registered",
                        id: entry.physical_repository_id,
                    });
                }
                if let Some(receipt) = receipt {
                    return Ok(receipt.repository);
                }
                self.existing_authority_identity(
                    &authority,
                    &input,
                    &canonical_path,
                    &source_identity,
                    &entry,
                )?
            }
            Some(entry) => {
                return Err(ProductStoreError::Conflict {
                    kind: "repository_source_tombstoned",
                    id: entry.physical_repository_id,
                });
            }
            None => {
                if let Some(receipt) = receipt {
                    return Ok(receipt.repository);
                }
                match self.find_incomplete_authority_identity(
                    &authority,
                    &input,
                    &canonical_path,
                    &source_identity,
                )? {
                    Some(identity) => identity,
                    None => RepositoryIdentityAllocation::new(),
                }
            }
        };

        let repository = identity.repository_record(&input, canonical_path.clone());
        self.ensure_authority_records(
            &authority,
            &registry,
            &input,
            &source_identity,
            &identity,
            &repository,
        )?;
        self.ensure_compatibility_projection(&repository)?;

        let receipt = RepositoryCreateReceipt::new(&input, &canonical_path, repository.clone());
        receipts.save_create(&input.project_id, &receipt)?;
        Ok(repository)
    }

    fn existing_authority_identity(
        &self,
        authority: &LogicalCodebaseStore,
        input: &CreateRepositoryInput,
        canonical_path: &Path,
        source_identity: &RepositorySourceIdentity,
        entry: &IdentityRegistryEntry,
    ) -> Result<RepositoryIdentityAllocation, ProductStoreError> {
        let created_at = authority
            .load_member(&input.project_id, entry.logical_repository_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: entry.logical_repository_id.0.to_string(),
            })?
            .created_at;
        let identity = RepositoryIdentityAllocation {
            physical_repository_id: entry.physical_repository_id.clone(),
            logical_repository_id: entry.logical_repository_id,
            primary_checkout_id: entry.primary_checkout_id,
            created_at,
        };
        self.validate_authority_identity(
            authority,
            input,
            canonical_path,
            source_identity,
            &identity,
        )?;
        Ok(identity)
    }

    fn find_incomplete_authority_identity(
        &self,
        authority: &LogicalCodebaseStore,
        input: &CreateRepositoryInput,
        canonical_path: &Path,
        source_identity: &RepositorySourceIdentity,
    ) -> Result<Option<RepositoryIdentityAllocation>, ProductStoreError> {
        let matching_members = authority
            .list_members(&input.project_id)?
            .into_iter()
            .filter(|member| member.source_identity == *source_identity)
            .collect::<Vec<_>>();
        if matching_members.len() > 1 {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: source_identity.key_digest.clone(),
            });
        }
        let Some(member) = matching_members.into_iter().next() else {
            return Ok(None);
        };
        let [primary_checkout_id] = member.checkout_ids.as_slice() else {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: member.logical_repository_id.0.to_string(),
            });
        };
        let identity = RepositoryIdentityAllocation {
            physical_repository_id: member.physical_repository_id.clone(),
            logical_repository_id: member.logical_repository_id,
            primary_checkout_id: *primary_checkout_id,
            created_at: member.created_at.clone(),
        };
        validate_relative_id(&identity.physical_repository_id)?;
        let expected_member =
            identity.member_record(input, source_identity, member.ordinal, &member.created_at);
        if member != expected_member {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: identity.logical_repository_id.0.to_string(),
            });
        }
        if let Some(checkout) =
            authority.load_checkout(&input.project_id, identity.primary_checkout_id)?
        {
            let expected_checkout =
                identity.checkout_record(canonical_path, source_identity, &checkout.created_at);
            if checkout != expected_checkout {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_checkout",
                    id: identity.primary_checkout_id.0.to_string(),
                });
            }
        }
        Ok(Some(identity))
    }

    fn validate_authority_identity(
        &self,
        authority: &LogicalCodebaseStore,
        input: &CreateRepositoryInput,
        canonical_path: &Path,
        source_identity: &RepositorySourceIdentity,
        identity: &RepositoryIdentityAllocation,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&identity.physical_repository_id)?;
        let member = authority
            .load_member(&input.project_id, identity.logical_repository_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: identity.logical_repository_id.0.to_string(),
            })?;
        let expected_member =
            identity.member_record(input, source_identity, member.ordinal, &member.created_at);
        if member != expected_member {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: identity.logical_repository_id.0.to_string(),
            });
        }
        let checkout = authority
            .load_checkout(&input.project_id, identity.primary_checkout_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "repository_checkout",
                id: identity.primary_checkout_id.0.to_string(),
            })?;
        let expected_checkout =
            identity.checkout_record(canonical_path, source_identity, &checkout.created_at);
        if checkout != expected_checkout {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_checkout",
                id: identity.primary_checkout_id.0.to_string(),
            });
        }
        Ok(())
    }

    fn ensure_authority_records(
        &self,
        authority: &LogicalCodebaseStore,
        registry: &IdentityRegistryStore,
        input: &CreateRepositoryInput,
        source_identity: &RepositorySourceIdentity,
        identity: &RepositoryIdentityAllocation,
        repository: &RepositoryRecord,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&identity.physical_repository_id)?;
        let member = match authority
            .load_member(&input.project_id, identity.logical_repository_id)?
        {
            Some(member) => {
                let expected = identity.member_record(
                    input,
                    source_identity,
                    member.ordinal,
                    &member.created_at,
                );
                if member != expected {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "logical_codebase_member",
                        id: identity.logical_repository_id.0.to_string(),
                    });
                }
                member
            }
            None => {
                let ordinal = authority
                    .list_members(&input.project_id)?
                    .iter()
                    .map(|member| member.ordinal)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| ProductStoreError::InvalidRecord {
                        kind: "logical_codebase_member",
                        reason: "repository ordinal overflow".to_string(),
                    })?;
                let member =
                    identity.member_record(input, source_identity, ordinal, &repository.created_at);
                authority.save_member(&input.project_id, &member)?;
                member
            }
        };

        let checkout =
            match authority.load_checkout(&input.project_id, identity.primary_checkout_id)? {
                Some(checkout) => {
                    let expected = identity.checkout_record(
                        &repository.path,
                        source_identity,
                        &checkout.created_at,
                    );
                    if checkout != expected {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "repository_checkout",
                            id: identity.primary_checkout_id.0.to_string(),
                        });
                    }
                    checkout
                }
                None => {
                    let checkout = identity.checkout_record(
                        &repository.path,
                        source_identity,
                        &repository.created_at,
                    );
                    authority.save_checkout(&input.project_id, &checkout)?;
                    checkout
                }
            };

        let expected_registry = IdentityRegistryEntry::active(
            source_identity.clone(),
            identity.logical_repository_id,
            identity.physical_repository_id.clone(),
            identity.primary_checkout_id,
            input.idempotency_key.clone(),
        );
        match registry.find_by_source(&input.project_id, source_identity)? {
            Some(existing) if existing == expected_registry => {}
            Some(_) => {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "identity_registry",
                    id: identity.physical_repository_id.clone(),
                });
            }
            None => registry.upsert_active(&input.project_id, expected_registry)?,
        }

        self.ensure_manifest_membership(
            authority,
            &input.project_id,
            member.logical_repository_id,
        )?;
        debug_assert_eq!(checkout.checkout_id, identity.primary_checkout_id);
        Ok(())
    }

    fn ensure_manifest_membership(
        &self,
        authority: &LogicalCodebaseStore,
        project_id: &str,
        member_id: LogicalRepositoryId,
    ) -> Result<(), ProductStoreError> {
        let mut manifest =
            authority
                .load_manifest(project_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_codebase_manifest",
                    id: project_id.to_string(),
                })?;
        if manifest.member_ids.contains(&member_id) {
            return Ok(());
        }
        manifest.member_ids.push(member_id);
        manifest.membership_revision =
            manifest.membership_revision.checked_add(1).ok_or_else(|| {
                ProductStoreError::InvalidRecord {
                    kind: "logical_codebase_manifest",
                    reason: "membership_revision overflow".to_string(),
                }
            })?;
        manifest.updated_at = Utc::now().to_rfc3339();
        authority.save_manifest(project_id, &manifest)
    }

    fn ensure_compatibility_projection(
        &self,
        repository: &RepositoryRecord,
    ) -> Result<(), ProductStoreError> {
        let mut repositories = self.list_compatibility_projection(&repository.project_id)?;
        match repositories
            .iter()
            .find(|existing| existing.id == repository.id)
        {
            Some(existing) if existing == repository => return Ok(()),
            Some(_) => {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_projection",
                    id: repository.id.clone(),
                });
            }
            None => {}
        }
        repositories.push(repository.clone());
        write_json(&self.repos_path(&repository.project_id), &repositories)
    }

    fn list_compatibility_projection(
        &self,
        project_id: &str,
    ) -> Result<Vec<RepositoryRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let path = self.repos_path(project_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        read_json(&path)
    }


}
