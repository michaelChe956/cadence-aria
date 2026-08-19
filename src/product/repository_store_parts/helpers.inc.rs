fn identity_resolution_error(id: &str, missing: bool) -> ProductStoreError {
    if missing {
        ProductStoreError::NotFound {
            kind: "identity_resolution_missing",
            id: id.to_string(),
        }
    } else {
        ProductStoreError::Ambiguous {
            kind: "identity_resolution_ambiguous",
            id: id.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct RepositoryIdentityAllocation {
    physical_repository_id: String,
    logical_repository_id: LogicalRepositoryId,
    primary_checkout_id: RepositoryCheckoutId,
    created_at: String,
}

impl RepositoryIdentityAllocation {
    fn new() -> Self {
        let physical_repository_id = format!("repository_{}", Uuid::new_v4().simple());
        // This physical ID is later persisted in the authority records before it
        // is used by any compatibility-projection path.
        debug_assert!(validate_relative_id(&physical_repository_id).is_ok());
        Self {
            physical_repository_id,
            logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
            primary_checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn repository_record(
        &self,
        input: &CreateRepositoryInput,
        canonical_path: PathBuf,
    ) -> RepositoryRecord {
        let repo_path_text = canonical_path.to_string_lossy();
        RepositoryRecord {
            id: self.physical_repository_id.clone(),
            project_id: input.project_id.clone(),
            name: input.name.clone(),
            repo_hash: repo_hash_for_path(repo_path_text.as_ref()),
            runtime_root: canonical_path.join(".aria/runtime"),
            path: canonical_path,
            default_policy_preset: input
                .default_policy_preset
                .clone()
                .unwrap_or_else(|| "manual-write".to_string()),
            default_provider_mode: input
                .default_provider_mode
                .clone()
                .unwrap_or_else(|| "fake".to_string()),
            created_at: self.created_at.clone(),
            updated_at: self.created_at.clone(),
            logical_repository_id: Some(self.logical_repository_id),
            primary_checkout_id: Some(self.primary_checkout_id),
            identity_schema_version: 1,
        }
    }

    fn member_record(
        &self,
        input: &CreateRepositoryInput,
        source_identity: &RepositorySourceIdentity,
        ordinal: u32,
        created_at: &str,
    ) -> CodebaseMemberRecord {
        CodebaseMemberRecord {
            logical_repository_id: self.logical_repository_id,
            physical_repository_id: self.physical_repository_id.clone(),
            alias: input.name.clone(),
            role: "repository".to_string(),
            ordinal,
            source_identity: source_identity.clone(),
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![self.primary_checkout_id],
            status: MemberStatus::Active,
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
        }
    }

    fn checkout_record(
        &self,
        canonical_path: &Path,
        source_identity: &RepositorySourceIdentity,
        created_at: &str,
    ) -> RepositoryCheckoutRecord {
        RepositoryCheckoutRecord {
            checkout_id: self.primary_checkout_id,
            logical_repository_id: self.logical_repository_id,
            physical_repository_id: self.physical_repository_id.clone(),
            kind: CheckoutKind::Main,
            canonical_path: canonical_path.to_path_buf(),
            checkout_path_hash: repo_hash_for_path(canonical_path.to_string_lossy().as_ref()),
            git_dir_identity: source_identity.git_dir_identity(),
            revision: None,
            availability: CheckoutAvailability::Available,
            observed_at: created_at.to_string(),
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RepositoryDeleteReceipt {
    project_id: String,
    command: DeleteRepositoryCommandReceipt,
    public: RepositoryDeletionReceipt,
    completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DeleteRepositoryCommandReceipt {
    operation_id: String,
    expected_updated_at: Option<String>,
    physical_repository_id: String,
    original_updated_at: String,
}

impl RepositoryDeleteReceipt {
    fn intent(
        project_id: &str,
        repository: &RepositoryRecord,
        member: &CodebaseMemberRecord,
        checkout: &RepositoryCheckoutRecord,
        command: &DeleteRepositoryCommand,
        deleted_at: String,
    ) -> Self {
        Self {
            project_id: project_id.to_string(),
            command: DeleteRepositoryCommandReceipt {
                operation_id: command.operation_id.clone(),
                expected_updated_at: command.expected_updated_at.clone(),
                physical_repository_id: repository.id.clone(),
                original_updated_at: repository.updated_at.clone(),
            },
            public: RepositoryDeletionReceipt {
                physical_repository_id: repository.id.clone(),
                logical_repository_id: Some(member.logical_repository_id),
                checkout_id: Some(checkout.checkout_id),
                tombstone_operation_id: Some(command.operation_id.clone()),
                deleted_at,
                legacy_delete: false,
            },
            completed: false,
        }
    }

    fn legacy_intent(
        project_id: &str,
        repository: &RepositoryRecord,
        command: &DeleteRepositoryCommand,
        deleted_at: String,
    ) -> Self {
        Self {
            project_id: project_id.to_string(),
            command: DeleteRepositoryCommandReceipt {
                operation_id: command.operation_id.clone(),
                expected_updated_at: command.expected_updated_at.clone(),
                physical_repository_id: repository.id.clone(),
                original_updated_at: repository.updated_at.clone(),
            },
            public: RepositoryDeletionReceipt {
                physical_repository_id: repository.id.clone(),
                logical_repository_id: None,
                checkout_id: None,
                tombstone_operation_id: None,
                deleted_at,
                legacy_delete: true,
            },
            completed: false,
        }
    }

    fn validate_path_identity(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<(), ProductStoreError> {
        for id in [
            self.project_id.as_str(),
            self.command.operation_id.as_str(),
            self.command.physical_repository_id.as_str(),
            self.public.physical_repository_id.as_str(),
            project_id,
            operation_id,
        ] {
            validate_relative_id(id)?;
        }
        let public_identity_is_valid = if self.public.legacy_delete {
            self.public.logical_repository_id.is_none()
                && self.public.checkout_id.is_none()
                && self.public.tombstone_operation_id.is_none()
        } else {
            self.public.logical_repository_id.is_some()
                && self.public.checkout_id.is_some()
                && self.public.tombstone_operation_id.as_deref() == Some(operation_id)
        };
        if self.project_id != project_id
            || self.command.operation_id != operation_id
            || self.command.physical_repository_id != self.public.physical_repository_id
            || !public_identity_is_valid
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_delete_receipt",
                id: operation_id.to_string(),
            });
        }
        Ok(())
    }

    fn validate_delete_command(
        &self,
        project_id: &str,
        physical_repository_id: &str,
        command: &DeleteRepositoryCommand,
    ) -> Result<(), ProductStoreError> {
        self.validate_path_identity(project_id, &command.operation_id)?;
        if self.command.physical_repository_id != physical_repository_id
            || self.command.expected_updated_at != command.expected_updated_at
        {
            return Err(ProductStoreError::Conflict {
                kind: "idempotency_key_reused",
                id: command.operation_id.clone(),
            });
        }
        Ok(())
    }

    fn into_public(self) -> RepositoryDeletionReceipt {
        self.public
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RepositoryCreateReceipt {
    idempotency_key: String,
    input_digest: String,
    repository: RepositoryRecord,
}

impl RepositoryCreateReceipt {
    fn new(
        input: &CreateRepositoryInput,
        canonical_path: &Path,
        repository: RepositoryRecord,
    ) -> Self {
        Self {
            idempotency_key: input.idempotency_key.clone(),
            input_digest: create_input_digest(input, canonical_path),
            repository,
        }
    }

    fn validate_input(
        &self,
        input: &CreateRepositoryInput,
        canonical_path: &Path,
    ) -> Result<(), ProductStoreError> {
        if self.input_digest != create_input_digest(input, canonical_path) {
            return Err(ProductStoreError::Conflict {
                kind: "idempotency_key_reused",
                id: input.idempotency_key.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct RepositoryCommandReceiptStore {
    paths: ProductAppPaths,
}

impl RepositoryCommandReceiptStore {
    fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    fn find_create(
        &self,
        project_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<RepositoryCreateReceipt>, ProductStoreError> {
        let path = self.create_receipt_path(project_id, idempotency_key)?;
        if !path.exists() {
            return Ok(None);
        }
        let receipt: RepositoryCreateReceipt = read_json(&path)?;
        if receipt.idempotency_key != idempotency_key {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_create_receipt",
                id: idempotency_key.to_string(),
            });
        }
        Ok(Some(receipt))
    }

    fn save_create(
        &self,
        project_id: &str,
        receipt: &RepositoryCreateReceipt,
    ) -> Result<(), ProductStoreError> {
        let path = self.create_receipt_path(project_id, &receipt.idempotency_key)?;
        if path.exists() {
            let existing: RepositoryCreateReceipt = read_json(&path)?;
            if existing == *receipt {
                return Ok(());
            }
            return Err(ProductStoreError::Conflict {
                kind: "idempotency_key_reused",
                id: receipt.idempotency_key.clone(),
            });
        }
        write_json(&path, receipt)
    }

    fn find_delete(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<Option<RepositoryDeleteReceipt>, ProductStoreError> {
        let path = self.delete_receipt_path(project_id, operation_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let receipt: RepositoryDeleteReceipt = read_json(&path)?;
        receipt.validate_path_identity(project_id, operation_id)?;
        Ok(Some(receipt))
    }

    fn save_delete(
        &self,
        project_id: &str,
        receipt: &RepositoryDeleteReceipt,
    ) -> Result<(), ProductStoreError> {
        receipt.validate_path_identity(project_id, &receipt.command.operation_id)?;
        let path = self.delete_receipt_path(project_id, &receipt.command.operation_id)?;
        if path.exists() {
            let existing: RepositoryDeleteReceipt = read_json(&path)?;
            if existing == *receipt {
                return Ok(());
            }
            return Err(ProductStoreError::Conflict {
                kind: "idempotency_key_reused",
                id: receipt.command.operation_id.clone(),
            });
        }
        write_json(&path, receipt)
    }

    fn mark_delete_completed(
        &self,
        project_id: &str,
        receipt: &RepositoryDeleteReceipt,
    ) -> Result<RepositoryDeletionReceipt, ProductStoreError> {
        let path = self.delete_receipt_path(project_id, &receipt.command.operation_id)?;
        let mut stored: RepositoryDeleteReceipt = read_json(&path)?;
        stored.validate_path_identity(project_id, &receipt.command.operation_id)?;
        if stored != *receipt && !stored.completed {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "repository_delete_receipt",
                id: receipt.command.operation_id.clone(),
            });
        }
        if !stored.completed {
            stored.completed = true;
            write_json(&path, &stored)?;
        }
        Ok(stored.into_public())
    }

    fn delete_receipt_path(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        Ok(self
            .paths
            .logical_codebase_root(project_id)
            .join("command-receipts")
            .join(format!("delete-{operation_id}.json")))
    }

    fn create_receipt_path(
        &self,
        project_id: &str,
        idempotency_key: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(idempotency_key)?;
        Ok(self
            .paths
            .logical_codebase_root(project_id)
            .join("command-receipts")
            .join(format!("create-{idempotency_key}.json")))
    }
}

struct RepositorySourceResolver;

impl RepositorySourceResolver {
    fn resolve(canonical_path: &Path) -> Result<RepositorySourceIdentity, ProductStoreError> {
        let git_dir_output = run_git(canonical_path, &["rev-parse", "--git-dir"])?;
        let git_dir = PathBuf::from(git_dir_output.trim());
        let git_dir = if git_dir.is_absolute() {
            git_dir
        } else {
            canonical_path.join(git_dir)
        };
        let canonical_git_dir = canonicalize_repo_path(&git_dir)?;
        let canonical_origin = match run_git(canonical_path, &["remote", "get-url", "origin"]) {
            Ok(value) => {
                let origin = value.trim();
                (!origin.is_empty()).then(|| origin.to_string())
            }
            Err(ProductStoreError::Io(message)) if message.contains("git exited") => None,
            Err(error) => return Err(error),
        };
        Ok(RepositorySourceIdentity::from_git_parts(
            canonical_path,
            canonical_git_dir,
            canonical_origin,
        ))
    }
}

pub(crate) fn resolve_repository_source(
    canonical_path: &Path,
) -> Result<RepositorySourceIdentity, ProductStoreError> {
    RepositorySourceResolver::resolve(canonical_path)
}

fn run_git(repository_path: &Path, arguments: &[&str]) -> Result<String, ProductStoreError> {
    let output = Command::new("git")
        .current_dir(repository_path)
        .args(arguments)
        .output()
        .map_err(|error| {
            ProductStoreError::Io(format!("run git in {}: {error}", repository_path.display()))
        })?;
    if !output.status.success() {
        return Err(ProductStoreError::Io(format!(
            "git exited {:?} in {}: {}",
            output.status.code(),
            repository_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| ProductStoreError::Io(format!("git output was not UTF-8: {error}")))
}

fn create_input_digest(input: &CreateRepositoryInput, canonical_path: &Path) -> String {
    let payload = format!(
        "{}\\0{}\\0{}\\0{}\\0{}\\0{}",
        input.project_id,
        input.name,
        canonical_path.to_string_lossy(),
        input.default_policy_preset.as_deref().unwrap_or("<none>"),
        input.default_provider_mode.as_deref().unwrap_or("<none>"),
        "repository_create_v1",
    );
    format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
}

pub(crate) fn canonicalize_repo_path(path: &Path) -> Result<PathBuf, ProductStoreError> {
    fs::canonicalize(path)
        .map_err(|error| ProductStoreError::Io(format!("canonicalize {}: {error}", path.display())))
}

