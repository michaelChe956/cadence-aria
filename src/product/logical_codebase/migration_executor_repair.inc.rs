impl IdentityMigrationExecutor {
    /// C4 Task 7：repair-only 只读事实——直接读 repos.json（legacy 布局）
    /// 并重算 source digest。绕过 `ensure_identity_schema` 与普通成员/
    /// 身份解析路径；失败 journal 期间普通链路不可用时仍可诊断。
    pub fn read_repair_sources(
        &self,
        project_id: &str,
    ) -> Result<(Vec<RepositoryRecord>, String), ProductStoreError> {
        validate_relative_id(project_id)?;
        let repositories = self.legacy_repositories(project_id)?;
        let digest = source_repositories_digest(&repositories)?;
        Ok((repositories, digest))
    }

    /// C4 Task 7：用户确认后的显式 repair——在迁移锁内把 Failed journal
    /// 拨回 Scanning（audit 追加、原失败原因保留在 audit detail，journal
    /// 文件不删除），锁外复用既有 ensure 链按 `continuation` 推进。
    pub fn repair_from_failed(
        &self,
        project_id: &str,
        audit: crate::product::logical_codebase::migration::IdentityRepairAuditEntry,
        continuation: crate::product::logical_codebase::identity_repair::RepairContinuation,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        let lock_path = self.paths.identity_migration_lock_path(project_id);
        with_exact_exclusive_lock(&lock_path, || {
            let mut journal = self.load_or_begin_scanning(project_id)?;
            if journal.phase != IdentityMigrationPhase::Failed {
                return Err(ProductStoreError::Conflict {
                    kind: "identity_repair_not_failed",
                    id: journal.migration_id,
                });
            }
            if journal
                .repair_audit
                .iter()
                .any(|entry| entry.command_id == audit.command_id)
            {
                return Ok(());
            }
            journal.repair_audit.push(audit);
            journal.phase = IdentityMigrationPhase::Scanning;
            journal.last_error = None;
            touch(&mut journal);
            self.journals.save(project_id, &journal)
        })?;
        match continuation {
            crate::product::logical_codebase::identity_repair::RepairContinuation::ThroughAuthority => {
                self.ensure_through_authority(project_id)
            }
            crate::product::logical_codebase::identity_repair::RepairContinuation::FullSchema => {
                self.ensure_identity_schema(project_id)
            }
        }
    }

    /// C4 Task 7：mapping 裁决核验（失败零写入）——digest/physical/canonical
    /// 幂等键/registry 候选完全一致才放行；tombstone 候选在用户提交了完全
    /// 匹配的 mapping 后经原链 `reactivate_tombstoned_source` 复活。
    pub fn repair_validate_mapping(
        &self,
        project_id: &str,
        submission: &crate::product::logical_codebase::identity_repair::IdentityMappingSubmission,
    ) -> Result<RepositoryIdentityMapping, ProductStoreError> {
        validate_relative_id(project_id)?;
        let repositories = self.legacy_repositories(project_id)?;
        let repository = repositories
            .iter()
            .find(|repository| repository.id == submission.legacy_repository_id)
            .ok_or_else(|| ProductStoreError::Conflict {
                kind: "identity_repair_mapping_unknown_repository",
                id: submission.legacy_repository_id.clone(),
            })?;
        let source = repository_source_identity(repository)?;
        if source.key_digest != submission.source_identity_digest {
            return Err(ProductStoreError::Conflict {
                kind: "identity_repair_mapping_digest_mismatch",
                id: submission.legacy_repository_id.clone(),
            });
        }
        if repository.id != submission.physical_repository_id {
            return Err(ProductStoreError::Conflict {
                kind: "identity_repair_mapping_physical_mismatch",
                id: submission.physical_repository_id.clone(),
            });
        }
        let idempotency_key = mapping_idempotency_key(project_id, &repository.id, &source.key_digest);
        if !submission.idempotency_key.is_empty() && submission.idempotency_key != idempotency_key {
            return Err(ProductStoreError::Conflict {
                kind: "identity_repair_mapping_key_mismatch",
                id: submission.legacy_repository_id.clone(),
            });
        }
        match self.registry.find_by_source(project_id, &source)? {
            None => Err(ProductStoreError::Conflict {
                kind: "identity_repair_mapping_no_candidate",
                id: submission.legacy_repository_id.clone(),
            }),
            Some(entry) => {
                if entry.logical_repository_id != submission.logical_repository_id
                    || entry.primary_checkout_id != submission.primary_checkout_id
                {
                    return Err(ProductStoreError::Conflict {
                        kind: "identity_repair_mapping_candidate_mismatch",
                        id: submission.legacy_repository_id.clone(),
                    });
                }
                if entry.state == IdentityRegistryState::Tombstoned {
                    self.registry.reactivate_tombstoned_source(
                        project_id,
                        &source,
                        &format!("identity-repair:{project_id}"),
                        &Utc::now().to_rfc3339(),
                    )?;
                }
                Ok(RepositoryIdentityMapping {
                    legacy_repository_id: repository.id.clone(),
                    source_identity_digest: source.key_digest,
                    logical_repository_id: entry.logical_repository_id,
                    primary_checkout_id: entry.primary_checkout_id,
                    physical_repository_id: repository.id.clone(),
                    idempotency_key,
                    authority_written: false,
                    compatibility_backfilled: false,
                })
            }
        }
    }

    /// C4 Task 7：把用户裁决的 mapping 以 staging 事实写入 journal（替换同
    /// legacy id 的既有条目或追加），phase 保持 Failed——推进只能由后续
    /// Revalidate 走原链完成。
    pub fn repair_stage_mapping(
        &self,
        project_id: &str,
        mapping: RepositoryIdentityMapping,
        audit: crate::product::logical_codebase::migration::IdentityRepairAuditEntry,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        let lock_path = self.paths.identity_migration_lock_path(project_id);
        with_exact_exclusive_lock(&lock_path, || {
            let mut journal = self.load_or_begin_scanning(project_id)?;
            if journal.phase != IdentityMigrationPhase::Failed {
                return Err(ProductStoreError::Conflict {
                    kind: "identity_repair_not_failed",
                    id: journal.migration_id,
                });
            }
            if journal
                .repair_audit
                .iter()
                .any(|entry| entry.command_id == audit.command_id)
            {
                return Ok(());
            }
            match journal
                .mappings
                .iter_mut()
                .find(|existing| existing.legacy_repository_id == mapping.legacy_repository_id)
            {
                Some(existing) => *existing = mapping,
                None => journal.mappings.push(mapping),
            }
            journal.repair_audit.push(audit);
            touch(&mut journal);
            self.journals.save(project_id, &journal)
        })
    }
}
