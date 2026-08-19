impl RepositoryStore {
    /// v1.3：按 issue 所属 codebase 解析逻辑仓三层身份（R9 编码/交付链切换点）。
    ///
    /// - `lc_id = None` 或 legacy 别名 lc：保持既有 strict 语义（legacy 根权威 +
    ///   repos.json 兼容投影），单仓/旧数据字节级不变；
    /// - 非 legacy `lc_id`：从 `logical-codebases/{lc_id}/` 子树权威记录解析
    ///   member/checkout；主 checkout 以 project 级 identity registry 的
    ///   `primary_checkout_id` 为权威（缺失时回退唯一的 Main kind checkout）。
    ///   新 LC 登记不写 repos.json，物理 `RepositoryRecord` 投影缺失时由
    ///   member/checkout 权威记录合成（与登记时 `RepositoryIdentityAllocation`
    ///   的构造一致）。
    pub fn resolve_logical_repository_for_issue_codebase(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
        logical_id: LogicalRepositoryId,
    ) -> Result<
        (
            CodebaseMemberRecord,
            RepositoryCheckoutRecord,
            RepositoryRecord,
        ),
        ProductStoreError,
    > {
        match lc_id {
            None => self.resolve_logical_repository_strict(project_id, logical_id),
            Some(lc_id) => {
                if lc_id == crate::product::logical_codebase::legacy_logical_codebase_id(project_id)
                {
                    return self.resolve_logical_repository_strict(project_id, logical_id);
                }
                self.resolve_logical_repository_in_lc(project_id, lc_id, logical_id)
            }
        }
    }

    fn resolve_logical_repository_in_lc(
        &self,
        project_id: &str,
        lc_id: &str,
        logical_id: LogicalRepositoryId,
    ) -> Result<
        (
            CodebaseMemberRecord,
            RepositoryCheckoutRecord,
            RepositoryRecord,
        ),
        ProductStoreError,
    > {
        validate_relative_id(project_id)?;
        validate_relative_id(lc_id)?;
        let authority = LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id);
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

        // 主 checkout：project 级 identity registry 权威；崩溃窗口（member 存在、
        // registry 缺失）回退 member.checkout_ids 中唯一的 Main kind checkout。
        let registry_matches = IdentityRegistryStore::new(self.paths.clone())
            .find_by_logical_id(project_id, logical_id)?;
        let checkout_id = match registry_matches.as_slice() {
            [entry] => entry.primary_checkout_id,
            _ => {
                let main_checkouts = member
                    .checkout_ids
                    .iter()
                    .filter_map(|id| authority.load_checkout(project_id, *id).ok().flatten())
                    .filter(|checkout| checkout.kind == CheckoutKind::Main)
                    .collect::<Vec<_>>();
                let [checkout] = main_checkouts.as_slice() else {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "logical_repository",
                        id: logical_id.0.to_string(),
                    });
                };
                checkout.checkout_id
            }
        };
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
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_repository",
                id: logical_id.0.to_string(),
            });
        }

        // 物理投影：legacy 双读投影存在且身份一致时优先；新 LC（repos.json 无记录）
        // 由权威记录合成。
        let repository = self
            .list_compatibility_projection(project_id)?
            .into_iter()
            .find(|record| {
                record.id == member.physical_repository_id
                    && record.project_id == project_id
                    && record.logical_repository_id == Some(logical_id)
                    && record.primary_checkout_id == Some(checkout_id)
                    && record.identity_schema_version == 1
            })
            .unwrap_or_else(|| {
                synthesize_repository_record_from_authority(project_id, &member, &checkout)
            });
        Ok((member, checkout, repository))
    }

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

/// 新 LC（repos.json 无投影）时由 LC 权威 member/checkout 记录合成物理
/// `RepositoryRecord`。字段构造与登记路径 `RepositoryIdentityAllocation::
/// repository_record` 一致：repo_hash/runtime_root 由 canonical path 派生，
/// 默认策略沿用登记缺省值。
fn synthesize_repository_record_from_authority(
    project_id: &str,
    member: &CodebaseMemberRecord,
    checkout: &RepositoryCheckoutRecord,
) -> RepositoryRecord {
    let canonical_path = checkout.canonical_path.clone();
    let repo_hash = repo_hash_for_path(canonical_path.to_string_lossy().as_ref());
    RepositoryRecord {
        id: member.physical_repository_id.clone(),
        project_id: project_id.to_string(),
        name: member.alias.clone(),
        repo_hash,
        runtime_root: canonical_path.join(".aria/runtime"),
        path: canonical_path,
        default_policy_preset: "manual-write".to_string(),
        default_provider_mode: "fake".to_string(),
        created_at: member.created_at.clone(),
        updated_at: member.updated_at.clone(),
        logical_repository_id: Some(member.logical_repository_id),
        primary_checkout_id: Some(checkout.checkout_id),
        identity_schema_version: 1,
    }
}
