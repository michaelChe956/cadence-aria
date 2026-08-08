impl RepositoryStore {
    pub fn delete(
        &self,
        project_id: &str,
        physical_repository_id: &str,
        command: DeleteRepositoryCommand,
    ) -> Result<RepositoryDeletionReceipt, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(physical_repository_id)?;
        validate_relative_id(&command.operation_id)?;
        if command.allow_tombstone_reactivation {
            return Err(ProductStoreError::InvalidRecord {
                kind: "delete_repository",
                reason: "delete command cannot bypass references".to_string(),
            });
        }
        let receipts = RepositoryCommandReceiptStore::new(self.paths.clone());
        if let Some(receipt) = receipts.find_delete(project_id, &command.operation_id)? {
            receipt.validate_delete_command(project_id, physical_repository_id, &command)?;
            return self.replay_delete_receipt(project_id, receipt);
        }
        if !self.logical_codebase_feature.is_enabled() {
            return self.delete_legacy(project_id, physical_repository_id, command);
        }

        self.ensure_identity_schema(project_id)?;
        let (member, checkout, repository) =
            self.resolve_logical_by_physical(project_id, physical_repository_id)?;
        if command
            .expected_updated_at
            .as_deref()
            .is_some_and(|expected| expected != repository.updated_at)
        {
            return Err(ProductStoreError::Conflict {
                kind: "repository_updated_at",
                id: physical_repository_id.to_string(),
            });
        }
        let report = RepositoryReferenceScanner::new(self.paths.clone()).scan(
            project_id,
            physical_repository_id,
            checkout.logical_repository_id,
        )?;
        if !report.is_empty() {
            return Err(ProductStoreError::Conflict {
                kind: "repository_references",
                id: physical_repository_id.to_string(),
            });
        }
        self.delete_with_tombstone(project_id, repository, member, checkout, command)
    }

    fn delete_legacy(
        &self,
        project_id: &str,
        repository_id: &str,
        command: DeleteRepositoryCommand,
    ) -> Result<RepositoryDeletionReceipt, ProductStoreError> {
        let mut repositories = self.list_compatibility_projection(project_id)?;
        let repository = repositories
            .iter()
            .find(|record| record.id == repository_id)
            .cloned()
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository",
                id: repository_id.to_string(),
            })?;
        if command
            .expected_updated_at
            .as_deref()
            .is_some_and(|expected| expected != repository.updated_at)
        {
            return Err(ProductStoreError::Conflict {
                kind: "repository_updated_at",
                id: repository_id.to_string(),
            });
        }
        let deleted_at = Utc::now().to_rfc3339();
        let receipt =
            RepositoryDeleteReceipt::legacy_intent(project_id, &repository, &command, deleted_at);
        RepositoryCommandReceiptStore::new(self.paths.clone()).save_delete(project_id, &receipt)?;
        repositories.retain(|record| record.id != repository_id);
        write_json(&self.repos_path(project_id), &repositories)?;
        RepositoryCommandReceiptStore::new(self.paths.clone())
            .mark_delete_completed(project_id, &receipt)
    }

    fn delete_with_tombstone(
        &self,
        project_id: &str,
        repository: RepositoryRecord,
        mut member: CodebaseMemberRecord,
        mut checkout: RepositoryCheckoutRecord,
        command: DeleteRepositoryCommand,
    ) -> Result<RepositoryDeletionReceipt, ProductStoreError> {
        let receipts = RepositoryCommandReceiptStore::new(self.paths.clone());
        debug_assert!(
            receipts
                .find_delete(project_id, &command.operation_id)?
                .is_none()
        );
        let deleted_at = Utc::now().to_rfc3339();
        let receipt = RepositoryDeleteReceipt::intent(
            project_id,
            &repository,
            &member,
            &checkout,
            &command,
            deleted_at,
        );
        receipts.save_delete(project_id, &receipt)?;
        self.apply_delete_tombstone(
            project_id,
            &repository.id,
            &mut member,
            &mut checkout,
            &receipt,
        )?;
        receipts.mark_delete_completed(project_id, &receipt)
    }

    fn replay_delete_receipt(
        &self,
        project_id: &str,
        receipt: RepositoryDeleteReceipt,
    ) -> Result<RepositoryDeletionReceipt, ProductStoreError> {
        if receipt.completed {
            return Ok(receipt.into_public());
        }
        if receipt.public.legacy_delete {
            let mut repositories = self.list_compatibility_projection(project_id)?;
            if repositories
                .iter()
                .any(|record| record.id == receipt.command.physical_repository_id)
            {
                repositories.retain(|record| record.id != receipt.command.physical_repository_id);
                write_json(&self.repos_path(project_id), &repositories)?;
            }
            return RepositoryCommandReceiptStore::new(self.paths.clone())
                .mark_delete_completed(project_id, &receipt);
        }
        let logical_repository_id = receipt.public.logical_repository_id.ok_or_else(|| {
            ProductStoreError::IdentityMismatch {
                kind: "repository_delete_receipt",
                id: receipt.command.operation_id.clone(),
            }
        })?;
        let checkout_id =
            receipt
                .public
                .checkout_id
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "repository_delete_receipt",
                    id: receipt.command.operation_id.clone(),
                })?;
        let authority = LogicalCodebaseStore::new(self.paths.clone());
        let mut member = authority
            .load_member(project_id, logical_repository_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "logical_codebase_member",
                id: logical_repository_id.0.to_string(),
            })?;
        let mut checkout = authority
            .load_checkout(project_id, checkout_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository_checkout",
                id: checkout_id.0.to_string(),
            })?;
        self.apply_delete_tombstone(
            project_id,
            &receipt.command.physical_repository_id,
            &mut member,
            &mut checkout,
            &receipt,
        )?;
        RepositoryCommandReceiptStore::new(self.paths.clone())
            .mark_delete_completed(project_id, &receipt)
    }

    fn apply_delete_tombstone(
        &self,
        project_id: &str,
        physical_repository_id: &str,
        member: &mut CodebaseMemberRecord,
        checkout: &mut RepositoryCheckoutRecord,
        receipt: &RepositoryDeleteReceipt,
    ) -> Result<(), ProductStoreError> {
        let registry = IdentityRegistryStore::new(self.paths.clone());
        let entry = registry
            .find_by_source(project_id, &member.source_identity)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "identity_registry",
                id: physical_repository_id.to_string(),
            })?;
        if entry.physical_repository_id != physical_repository_id
            || entry.logical_repository_id != member.logical_repository_id
            || entry.primary_checkout_id != checkout.checkout_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "identity_registry",
                id: physical_repository_id.to_string(),
            });
        }
        match entry.state {
            IdentityRegistryState::Active => registry.tombstone(
                project_id,
                &member.source_identity,
                &receipt.command.operation_id,
                &receipt.public.deleted_at,
            )?,
            IdentityRegistryState::Tombstoned => {
                if entry.delete_operation_id.as_deref()
                    != Some(receipt.command.operation_id.as_str())
                    || entry.deleted_at.as_deref() != Some(receipt.public.deleted_at.as_str())
                {
                    return Err(ProductStoreError::Conflict {
                        kind: "repository_source_tombstoned",
                        id: physical_repository_id.to_string(),
                    });
                }
            }
        }

        let authority = LogicalCodebaseStore::new(self.paths.clone());
        if member.status == MemberStatus::Active {
            member.status = MemberStatus::Tombstoned;
            member.updated_at = receipt.public.deleted_at.clone();
            authority.save_member(project_id, member)?;
        } else if member.status != MemberStatus::Tombstoned {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: member.logical_repository_id.0.to_string(),
            });
        }
        if checkout.availability == CheckoutAvailability::Available {
            checkout.availability = CheckoutAvailability::Unresolved;
            checkout.updated_at = receipt.public.deleted_at.clone();
            authority.save_checkout(project_id, checkout)?;
        } else if checkout.availability != CheckoutAvailability::Unresolved {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_checkout",
                id: checkout.checkout_id.0.to_string(),
            });
        }

        let mut repositories = self.list_compatibility_projection(project_id)?;
        if repositories
            .iter()
            .any(|record| record.id == physical_repository_id)
        {
            repositories.retain(|record| record.id != physical_repository_id);
            write_json(&self.repos_path(project_id), &repositories)?;
        }
        Ok(())
    }



}
