pub struct RegistrationBatchStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl RegistrationBatchStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// Scopes batches to one logical codebase subtree (`registration-batches/`
    /// under the v1.3 per-LC layout; the legacy alias keeps the legacy root).
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
        }
    }

    fn scope_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        crate::product::logical_codebase::lc_scope_root(&self.paths, project_id, &self.lc_id)
    }

    pub fn create_or_get(
        &self,
        batch: RegistrationBatchRecord,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        validate_batch_record(&batch)?;
        let project_id = batch.project_id.clone();
        self.with_project_lock(&project_id, || {
            let existing =
                self.find_by_idempotency_key_unlocked(&project_id, &batch.idempotency_key)?;
            match existing {
                Some(existing) => {
                    validate_batch_record(&existing)?;
                    if existing.aggregate_root != batch.aggregate_root
                        || existing.items != batch.items
                    {
                        return Err(ProductStoreError::Conflict {
                            kind: "registration_batch_idempotency_key_reused",
                            id: batch.idempotency_key.clone(),
                        });
                    }
                    Ok(existing)
                }
                None => {
                    let path = self.batch_path(&project_id, &batch.id)?;
                    if path.exists() {
                        return Err(ProductStoreError::Conflict {
                            kind: "registration_batch_id_collision",
                            id: batch.id.clone(),
                        });
                    }
                    write_json(&path, &batch)?;
                    Ok(batch)
                }
            }
        })
    }

    pub fn load(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        let path = self.batch_path(project_id, batch_id)?;
        if !path.exists() {
            return Err(ProductStoreError::NotFound {
                kind: "registration_batch",
                id: batch_id.to_string(),
            });
        }
        let batch: RegistrationBatchRecord = read_json(&path)?;
        validate_batch_record_for(&batch, project_id, batch_id)?;
        Ok(batch)
    }

    pub fn resume(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        self.load(project_id, batch_id)
    }

    pub fn cancel(
        &self,
        project_id: &str,
        batch_id: &str,
    ) -> Result<RegistrationBatchRecord, ProductStoreError> {
        self.with_batch_mutation(project_id, batch_id, |batch| {
            if batch.status != RegistrationBatchStatus::Completed {
                batch.status = RegistrationBatchStatus::Cancelled;
                batch.updated_at = Utc::now().to_rfc3339();
            }
            Ok(())
        })
        .map(|(batch, ())| batch)
    }

    fn with_batch_mutation<T>(
        &self,
        project_id: &str,
        batch_id: &str,
        mutation: impl FnOnce(&mut RegistrationBatchRecord) -> Result<T, ProductStoreError>,
    ) -> Result<(RegistrationBatchRecord, T), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(batch_id)?;
        self.with_project_lock(project_id, || {
            let mut batch = self.load(project_id, batch_id)?;
            let output = mutation(&mut batch)?;
            validate_batch_record(&batch)?;
            write_json(&self.batch_path(project_id, batch_id)?, &batch)?;
            Ok((batch, output))
        })
    }

    fn save_unlocked(&self, batch: &RegistrationBatchRecord) -> Result<(), ProductStoreError> {
        validate_batch_record(batch)?;
        write_json(&self.batch_path(&batch.project_id, &batch.id)?, batch)
    }

    fn with_project_lock<T>(
        &self,
        project_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        validate_relative_id(project_id)?;
        let lock_path = self.scope_root(project_id)?.join(".registration-batches.lock");
        with_exact_exclusive_lock(&lock_path, operation)
    }

    fn find_by_idempotency_key_unlocked(
        &self,
        project_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<RegistrationBatchRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(idempotency_key)?;
        let root = self.scope_root(project_id)?.join("registration-batches");
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read registration batches {}: {error}",
                    root.display()
                )));
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read registration batch entry: {error}"))
            })?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let batch: RegistrationBatchRecord = read_json(&path)?;
            validate_batch_record(&batch)?;
            if batch.idempotency_key == idempotency_key {
                return Ok(Some(batch));
            }
        }
        Ok(None)
    }

    fn batch_path(&self, project_id: &str, batch_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(batch_id)?;
        Ok(self
            .scope_root(project_id)?
            .join("registration-batches")
            .join(format!("{batch_id}.json")))
    }
}

fn validate_batch_record(batch: &RegistrationBatchRecord) -> Result<(), ProductStoreError> {
    validate_batch_record_for(batch, &batch.project_id, &batch.id)
}

fn validate_batch_record_for(
    batch: &RegistrationBatchRecord,
    project_id: &str,
    batch_id: &str,
) -> Result<(), ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(batch_id)?;
    validate_relative_id(&batch.project_id)?;
    validate_relative_id(&batch.id)?;
    validate_relative_id(&batch.idempotency_key)?;
    if batch.project_id != project_id || batch.id != batch_id || batch.items.is_empty() {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "registration_batch",
            id: batch_id.to_string(),
        });
    }
    let mut source_digests = std::collections::BTreeSet::new();
    for item in &batch.items {
        if item.source_digest.is_empty()
            || !source_digests.insert(item.source_digest.clone())
            || item.canonical_path.as_os_str().is_empty()
            || item.git_root.as_os_str().is_empty()
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "registration_batch_item",
                id: batch_id.to_string(),
            });
        }
    }
    Ok(())
}

fn aggregate_batch_status(items: &[RegistrationBatchItem]) -> RegistrationBatchStatus {
    if items.iter().all(|item| {
        matches!(
            item.status,
            RegistrationItemStatus::Completed | RegistrationItemStatus::Skipped
        )
    }) {
        RegistrationBatchStatus::Completed
    } else {
        RegistrationBatchStatus::PartialFailed
    }
}

fn sha256_key(payload: impl AsRef<[u8]>) -> String {
    format!("sha256:{:x}", Sha256::digest(payload.as_ref()))
}

fn stable_alias(candidate: &RegistrationCandidate) -> String {
    candidate
        .canonical_path
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("repository")
        .to_string()
}

