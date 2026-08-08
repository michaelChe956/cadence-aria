
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityMigrationPhase {
    Scanning,
    Mapping,
    WritingAuthority,
    BackfillingCompatibility,
    DualReadWrite,
    SwitchingReads,
    LegacyFallbackRemoved,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryIdentityMapping {
    pub legacy_repository_id: String,
    pub source_identity_digest: String,
    pub logical_repository_id: LogicalRepositoryId,
    pub primary_checkout_id: RepositoryCheckoutId,
    pub physical_repository_id: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub authority_written: bool,
    #[serde(default)]
    pub compatibility_backfilled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityMigrationJournal {
    pub journal_version: u16,
    pub migration_id: String,
    pub project_id: String,
    pub target_schema_version: u16,
    pub phase: IdentityMigrationPhase,
    pub source_repos_digest: String,
    pub mappings: Vec<RepositoryIdentityMapping>,
    #[serde(default)]
    pub completed_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

impl IdentityMigrationJournal {
    pub fn permits_legacy_projection(&self) -> bool {
        self.read_mode.as_deref() == Some("dual")
    }

    pub fn new(project_id: &str, source_repos_digest: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            journal_version: 1,
            migration_id: format!("identity-migration:{project_id}:v1"),
            project_id: project_id.to_string(),
            target_schema_version: 1,
            phase: IdentityMigrationPhase::Scanning,
            source_repos_digest: source_repos_digest.to_string(),
            mappings: Vec::new(),
            completed_keys: Vec::new(),
            read_mode: None,
            last_error: None,
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IdentityMigrationJournalStore {
    paths: ProductAppPaths,
}

impl IdentityMigrationJournalStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn load(
        &self,
        project_id: &str,
    ) -> Result<Option<IdentityMigrationJournal>, ProductStoreError> {
        let path = self.path(project_id)?;
        if !path.exists() {
            return Ok(None);
        }

        let journal: IdentityMigrationJournal = read_json(&path)?;
        self.validate_project(project_id, &journal)?;
        Ok(Some(journal))
    }

    pub fn save(
        &self,
        project_id: &str,
        journal: &IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        self.validate_project(project_id, journal)?;
        write_json(&self.path(project_id)?, journal)
    }

    fn path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(self
            .paths
            .logical_codebase_root(project_id)
            .join(IDENTITY_MIGRATION_JOURNAL_FILE))
    }

    fn validate_project(
        &self,
        project_id: &str,
        journal: &IdentityMigrationJournal,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(&journal.project_id)?;
        if journal.project_id != project_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "identity_migration_journal",
                id: project_id.to_string(),
            });
        }
        Ok(())
    }
}

/// Test-only and embedding hook for simulating a process interruption after an
/// authority write. Production executors use its no-op implementation.
pub trait MigrationFaultInjector: Send + Sync {
    fn after_authority_write(
        &self,
        _project_id: &str,
        _mapping: &RepositoryIdentityMapping,
    ) -> Result<(), ProductStoreError> {
        Ok(())
    }
}

#[derive(Debug)]
struct NoopMigrationFaultInjector;

impl MigrationFaultInjector for NoopMigrationFaultInjector {}

#[derive(Debug, Clone)]
struct AuthorityInput {
    repository: RepositoryRecord,
    mapping: RepositoryIdentityMapping,
    source_identity: RepositorySourceIdentity,
    canonical_path: PathBuf,
    ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct IssueCodebaseSelection {
    included: Vec<LogicalRepositoryId>,
    focus: Vec<LogicalRepositoryId>,
    selection_policy: String,
}

