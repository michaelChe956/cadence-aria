impl RepositoryStore {
    pub fn resolve_logical_repository(
        &self,
        project_id: &str,
        logical_id: LogicalRepositoryId,
    ) -> Result<
        (
            CodebaseMemberRecord,
            RepositoryCheckoutRecord,
            RepositoryRecord,
        ),
        ProductStoreError,
    > {
        self.resolve_logical_repository_with_source(project_id, logical_id)
            .map(|(member, checkout, repository, _)| (member, checkout, repository))
    }

    /// Resolves a logical repository exclusively from logical-codebase authority.
    ///
    /// This is for callers that have already entered the logical routing state:
    /// any absent or inconsistent authority record fails closed and must never
    /// fall back to the dual-read legacy projection.
    pub fn resolve_logical_repository_strict(
        &self,
        project_id: &str,
        logical_id: LogicalRepositoryId,
    ) -> Result<
        (
            CodebaseMemberRecord,
            RepositoryCheckoutRecord,
            RepositoryRecord,
        ),
        ProductStoreError,
    > {
        self.ensure_identity_schema(project_id)?;
        validate_relative_id(project_id)?;
        let authority = LogicalCodebaseStore::new(self.paths.clone());
        let manifest = authority.load_manifest(project_id)?.ok_or_else(|| {
            ProductStoreError::NotFound {
                kind: "logical_repository_manifest",
                id: project_id.to_string(),
            }
        })?;
        if !manifest.member_ids.contains(&logical_id) {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            });
        }

        let member = authority
            .load_member(project_id, logical_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            })?;
        validate_relative_id(&member.physical_repository_id)?;
        let repository = self
            .list_compatibility_projection(project_id)?
            .into_iter()
            .find(|record| record.id == member.physical_repository_id)
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            })?;
        let checkout_id =
            repository
                .primary_checkout_id
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "logical_repository",
                    id: logical_id.0.to_string(),
                })?;
        let checkout = authority
            .load_checkout(project_id, checkout_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            })?;
        if member.logical_repository_id != logical_id
            || member.status != MemberStatus::Active
            || !member.checkout_ids.contains(&checkout_id)
            || checkout.logical_repository_id != logical_id
            || checkout.physical_repository_id != member.physical_repository_id
            || repository.project_id != project_id
            || repository.logical_repository_id != Some(logical_id)
            || repository.identity_schema_version != 1
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            });
        }
        Ok((member, checkout, repository))
    }

    /// Resolves a legacy physical repository ID only during the persisted
    /// dual-read migration window. This is deliberately separate from the
    /// logical-ID reader so callers cannot accidentally treat a physical ID
    /// as an authority identifier after cutover.
    pub fn resolve_legacy_physical_repository_if_dual(
        &self,
        project_id: &str,
        physical_repository_id: &str,
    ) -> Result<
        (
            CodebaseMemberRecord,
            RepositoryCheckoutRecord,
            RepositoryRecord,
        ),
        ProductStoreError,
    > {
        validate_relative_id(project_id)?;
        validate_relative_id(physical_repository_id)?;
        let journal = IdentityMigrationJournalStore::new(self.paths.clone()).load(project_id)?;
        if !journal
            .as_ref()
            .is_some_and(IdentityMigrationJournal::permits_legacy_projection)
        {
            return Err(ProductStoreError::NotFound {
                kind: "logical_repository",
                id: physical_repository_id.to_string(),
            });
        }
        let matches = IdentityRegistryStore::new(self.paths.clone())
            .find_by_physical_id(project_id, physical_repository_id)?;
        let [entry] = matches.as_slice() else {
            return Err(identity_resolution_error(
                physical_repository_id,
                matches.is_empty(),
            ));
        };
        let (member, checkout, repository, source) =
            self.resolve_logical_repository_with_source(project_id, entry.logical_repository_id)?;
        tracing::warn!(
            project_id,
            logical_repository_id = %entry.logical_repository_id.0,
            physical_repository_id,
            resolution_source = ?source,
            metric = "identity_resolution_legacy_projection",
            "resolved legacy physical repository through unique registry mapping"
        );
        Ok((member, checkout, repository))
    }

    pub fn resolve_logical_repository_with_source(
        &self,
        project_id: &str,
        logical_id: LogicalRepositoryId,
    ) -> Result<
        (
            CodebaseMemberRecord,
            RepositoryCheckoutRecord,
            RepositoryRecord,
            ResolutionSource,
        ),
        ProductStoreError,
    > {
        self.ensure_identity_schema(project_id)?;
        validate_relative_id(project_id)?;
        let authority = LogicalCodebaseStore::new(self.paths.clone());
        let Some(manifest) = authority.load_manifest(project_id)? else {
            return self.resolve_legacy_projection_if_dual(project_id, logical_id);
        };
        if !manifest.member_ids.contains(&logical_id) {
            return self.resolve_legacy_projection_if_dual(project_id, logical_id);
        }

        let Some(member) = authority.load_member(project_id, logical_id)? else {
            return self.resolve_legacy_projection_if_dual(project_id, logical_id);
        };
        validate_relative_id(&member.physical_repository_id)?;
        let repository = self
            .list_compatibility_projection(project_id)?
            .into_iter()
            .find(|record| record.id == member.physical_repository_id)
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            })?;
        let checkout_id =
            repository
                .primary_checkout_id
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "logical_repository",
                    id: logical_id.0.to_string(),
                })?;
        let checkout = authority
            .load_checkout(project_id, checkout_id)?
            .ok_or_else(|| ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            })?;
        if member.logical_repository_id != logical_id
            || member.status != MemberStatus::Active
            || !member.checkout_ids.contains(&checkout_id)
            || checkout.logical_repository_id != logical_id
            || checkout.physical_repository_id != member.physical_repository_id
            || repository.project_id != project_id
            || repository.logical_repository_id != Some(logical_id)
            || repository.identity_schema_version != 1
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            });
        }
        Ok((
            member,
            checkout,
            repository,
            ResolutionSource::LogicalAuthority,
        ))
    }

    fn resolve_legacy_projection_if_dual(
        &self,
        project_id: &str,
        logical_id: LogicalRepositoryId,
    ) -> Result<
        (
            CodebaseMemberRecord,
            RepositoryCheckoutRecord,
            RepositoryRecord,
            ResolutionSource,
        ),
        ProductStoreError,
    > {
        let journal = IdentityMigrationJournalStore::new(self.paths.clone()).load(project_id)?;
        if !journal
            .as_ref()
            .is_some_and(IdentityMigrationJournal::permits_legacy_projection)
        {
            return Err(ProductStoreError::NotFound {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            });
        }

        let matches = IdentityRegistryStore::new(self.paths.clone())
            .find_by_logical_id(project_id, logical_id)?;
        let [entry] = matches.as_slice() else {
            return Err(identity_resolution_error(
                &logical_id.0.to_string(),
                matches.is_empty(),
            ));
        };
        validate_relative_id(&entry.physical_repository_id)?;
        let repository = self
            .list_compatibility_projection(project_id)?
            .into_iter()
            .find(|record| record.id == entry.physical_repository_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository",
                id: entry.physical_repository_id.clone(),
            })?;
        if repository.project_id != project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "legacy_projection",
                id: logical_id.0.to_string(),
            });
        }

        let member = CodebaseMemberRecord {
            logical_repository_id: logical_id,
            physical_repository_id: entry.physical_repository_id.clone(),
            alias: repository.name.clone(),
            role: "legacy_projection".to_string(),
            ordinal: 0,
            source_identity: entry.source_identity.clone(),
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![entry.primary_checkout_id],
            status: MemberStatus::Active,
            created_at: repository.created_at.clone(),
            updated_at: repository.updated_at.clone(),
        };
        let checkout = RepositoryCheckoutRecord {
            checkout_id: entry.primary_checkout_id,
            logical_repository_id: logical_id,
            physical_repository_id: entry.physical_repository_id.clone(),
            kind: CheckoutKind::Main,
            canonical_path: repository.path.clone(),
            checkout_path_hash: repository.repo_hash.clone(),
            git_dir_identity: entry.source_identity.git_dir_identity(),
            revision: None,
            availability: CheckoutAvailability::Unresolved,
            observed_at: repository.updated_at.clone(),
            created_at: repository.created_at.clone(),
            updated_at: repository.updated_at.clone(),
        };
        tracing::warn!(
            project_id,
            logical_repository_id = %logical_id.0,
            physical_repository_id = %entry.physical_repository_id,
            resolution_source = "legacy_projection",
            metric = "identity_resolution_legacy_projection",
            "using controlled legacy repository projection"
        );
        Ok((
            member,
            checkout,
            repository,
            ResolutionSource::LegacyProjection,
        ))
    }

    fn resolve_logical_by_physical(
        &self,
        project_id: &str,
        physical_repository_id: &str,
    ) -> Result<
        (
            CodebaseMemberRecord,
            RepositoryCheckoutRecord,
            RepositoryRecord,
        ),
        ProductStoreError,
    > {
        validate_relative_id(project_id)?;
        validate_relative_id(physical_repository_id)?;
        let repository = self
            .list_compatibility_projection(project_id)?
            .into_iter()
            .find(|record| record.id == physical_repository_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository",
                id: physical_repository_id.to_string(),
            })?;
        let logical_repository_id = repository.logical_repository_id.ok_or_else(|| {
            ProductStoreError::IdentityMismatch {
                kind: "repository_projection",
                id: physical_repository_id.to_string(),
            }
        })?;
        let checkout_id =
            repository
                .primary_checkout_id
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "repository_projection",
                    id: physical_repository_id.to_string(),
                })?;
        if repository.identity_schema_version != 1 {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_projection",
                id: physical_repository_id.to_string(),
            });
        }
        let authority = LogicalCodebaseStore::new(self.paths.clone());
        let member = authority
            .load_member(project_id, logical_repository_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "logical_codebase_member",
                id: logical_repository_id.0.to_string(),
            })?;
        let checkout = authority
            .load_checkout(project_id, checkout_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository_checkout",
                id: checkout_id.0.to_string(),
            })?;
        if member.physical_repository_id != physical_repository_id
            || member.logical_repository_id != logical_repository_id
            || !member.checkout_ids.contains(&checkout_id)
            || checkout.physical_repository_id != physical_repository_id
            || checkout.logical_repository_id != logical_repository_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_repository_resolution",
                id: physical_repository_id.to_string(),
            });
        }
        Ok((member, checkout, repository))
    }


}
