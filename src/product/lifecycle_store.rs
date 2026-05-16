use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::de::DeserializeOwned;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    DesignKind, DesignSpecRecord, LifecycleConfirmationStatus, LifecycleWorkItemRecord,
    ProjectProviderDefaultsRecord, ProviderName, SpecVersionRecord, StorySpecRecord,
    WorkItemPlanStatus, WorkItemStatus, WorkspaceMessageRecord, WorkspaceSessionRecord,
    WorkspaceSessionStatus, WorkspaceType,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateStorySpecInput {
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDesignSpecInput {
    pub project_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_kind: DesignKind,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkItemInput {
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendSpecVersionInput {
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub markdown: String,
    pub provider_run_refs: Vec<String>,
    pub review_refs: Vec<String>,
    pub confirmed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkspaceSessionInput {
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectProviderDefaultsInput {
    pub project_id: String,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct LifecycleStore {
    paths: ProductAppPaths,
}

enum ExistingSpecRecord {
    Story {
        path: PathBuf,
        record: StorySpecRecord,
    },
    Design {
        path: PathBuf,
        record: DesignSpecRecord,
    },
}

impl LifecycleStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn create_story_spec(
        &self,
        input: CreateStorySpecInput,
    ) -> Result<StorySpecRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;

        let root = self.story_specs_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("story_spec", count_json_files(&root)?);
        let now = Utc::now().to_rfc3339();
        let story = StorySpecRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            repository_id: input.repository_id,
            title: input.title,
            current_version: None,
            confirmation_status: LifecycleConfirmationStatus::Draft,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &story)?;
        Ok(story)
    }

    pub fn list_story_specs(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<StorySpecRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.story_specs_root(project_id, issue_id))
    }

    pub fn create_design_spec(
        &self,
        input: CreateDesignSpecInput,
    ) -> Result<DesignSpecRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_ids(&input.story_spec_ids)?;

        let root = self.design_specs_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("design_spec", count_json_files(&root)?);
        let now = Utc::now().to_rfc3339();
        let design = DesignSpecRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            story_spec_ids: input.story_spec_ids,
            design_kind: input.design_kind,
            title: input.title,
            current_version: None,
            confirmation_status: LifecycleConfirmationStatus::Draft,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &design)?;
        Ok(design)
    }

    pub fn list_design_specs(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<DesignSpecRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.design_specs_root(project_id, issue_id))
    }

    pub fn create_work_item(
        &self,
        input: CreateWorkItemInput,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;
        validate_relative_ids(&input.story_spec_ids)?;
        validate_relative_ids(&input.design_spec_ids)?;

        let root = self.work_items_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("work_item", count_json_files(&root)?);
        let now = Utc::now().to_rfc3339();
        let work_item = LifecycleWorkItemRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            repository_id: input.repository_id,
            story_spec_ids: input.story_spec_ids,
            design_spec_ids: input.design_spec_ids,
            title: input.title,
            plan_status: WorkItemPlanStatus::NotStarted,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &work_item)?;
        Ok(work_item)
    }

    pub fn list_work_items(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<LifecycleWorkItemRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.work_items_root(project_id, issue_id))
    }

    pub fn append_version(
        &self,
        input: AppendSpecVersionInput,
    ) -> Result<SpecVersionRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.entity_id)?;

        let spec = self.load_existing_spec(&input.project_id, &input.issue_id, &input.entity_id)?;
        let root = self.versions_root(&input.project_id, &input.issue_id, &input.entity_id);
        let versions: Vec<SpecVersionRecord> = list_json_records(&root)?;
        let version = next_version_number(&versions)?;
        let id = next_sequential_id(
            "version",
            usize::try_from(version - 1).map_err(|_| {
                ProductStoreError::Io(format!("version sequence overflow: {version}"))
            })?,
        );
        let now = Utc::now().to_rfc3339();
        let record = SpecVersionRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            entity_id: input.entity_id,
            version,
            markdown: input.markdown,
            provider_run_refs: input.provider_run_refs,
            review_refs: input.review_refs,
            confirmed_by: input.confirmed_by,
            created_at: now.clone(),
        };

        let target_path = root.join(format!("{id}.json"));
        ensure_target_absent(&target_path)?;
        write_json(&target_path, &record)?;
        self.update_spec_current_version(spec, version, now)?;
        Ok(record)
    }

    pub fn list_versions(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
    ) -> Result<Vec<SpecVersionRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(entity_id)?;
        list_json_records(&self.versions_root(project_id, issue_id, entity_id))
    }

    pub fn create_workspace_session(
        &self,
        input: CreateWorkspaceSessionInput,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.entity_id)?;

        let root = self.workspace_sessions_root(&input.project_id, &input.issue_id);
        let id = self.next_workspace_session_id()?;
        let now = Utc::now().to_rfc3339();
        let session = WorkspaceSessionRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            entity_id: input.entity_id,
            workspace_type: input.workspace_type,
            status: WorkspaceSessionStatus::Open,
            author_provider: input.author_provider,
            reviewer_provider: input.reviewer_provider,
            review_rounds: input.review_rounds,
            superpowers_enabled: input.superpowers_enabled,
            openspec_enabled: input.openspec_enabled,
            messages: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        };

        let target_path = root.join(format!("{id}.json"));
        ensure_target_absent(&target_path)?;
        write_json(&target_path, &session)?;
        Ok(session)
    }

    pub fn list_workspace_sessions(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<WorkspaceSessionRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_workspace_session_records(&self.workspace_sessions_root(project_id, issue_id))
    }

    pub fn get_workspace_session(
        &self,
        session_id: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        read_json(&self.find_workspace_session_path(session_id)?)
    }

    pub fn append_workspace_message(
        &self,
        session_id: &str,
        role: String,
        content: String,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        let now = Utc::now().to_rfc3339();
        session.messages.push(WorkspaceMessageRecord {
            role,
            content,
            created_at: now.clone(),
        });
        session.updated_at = now;
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn update_workspace_session_status(
        &self,
        session_id: &str,
        status: WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.status = status;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn upsert_project_provider_defaults(
        &self,
        input: CreateProjectProviderDefaultsInput,
    ) -> Result<ProjectProviderDefaultsRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;

        let defaults = ProjectProviderDefaultsRecord {
            project_id: input.project_id,
            author_provider: input.author_provider,
            reviewer_provider: input.reviewer_provider,
            review_rounds: input.review_rounds,
            superpowers_enabled: input.superpowers_enabled,
            openspec_enabled: input.openspec_enabled,
            updated_at: Utc::now().to_rfc3339(),
        };

        write_json(
            &self
                .paths
                .project_provider_defaults_path(&defaults.project_id),
            &defaults,
        )?;
        Ok(defaults)
    }

    fn load_existing_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
    ) -> Result<ExistingSpecRecord, ProductStoreError> {
        let story_path = self
            .story_specs_root(project_id, issue_id)
            .join(format!("{entity_id}.json"));
        if path_is_regular_file(&story_path)? {
            let record = read_json(&story_path)?;
            return Ok(ExistingSpecRecord::Story {
                path: story_path,
                record,
            });
        }

        let design_path = self
            .design_specs_root(project_id, issue_id)
            .join(format!("{entity_id}.json"));
        if path_is_regular_file(&design_path)? {
            let record = read_json(&design_path)?;
            return Ok(ExistingSpecRecord::Design {
                path: design_path,
                record,
            });
        }

        Err(ProductStoreError::NotFound {
            kind: "spec",
            id: entity_id.to_string(),
        })
    }

    fn update_spec_current_version(
        &self,
        spec: ExistingSpecRecord,
        version: u32,
        updated_at: String,
    ) -> Result<(), ProductStoreError> {
        match spec {
            ExistingSpecRecord::Story {
                path,
                record: mut story,
            } => {
                story.current_version = Some(version);
                story.updated_at = updated_at;
                write_json(&path, &story)
            }
            ExistingSpecRecord::Design {
                path,
                record: mut design,
            } => {
                design.current_version = Some(version);
                design.updated_at = updated_at;
                write_json(&path, &design)
            }
        }
    }

    fn next_workspace_session_id(&self) -> Result<String, ProductStoreError> {
        let max_sequence = max_workspace_session_sequence(&self.paths.projects_root())?;
        Ok(next_sequential_id("workspace_session", max_sequence))
    }

    fn find_workspace_session_path(&self, session_id: &str) -> Result<PathBuf, ProductStoreError> {
        let mut matched_path = None;
        for project_path in child_directories(&self.paths.projects_root())? {
            let issues_root = project_path.join("issues");
            for issue_path in child_directories(&issues_root)? {
                let workspace_sessions_root = issue_path.join("workspace-sessions");
                for session_path in workspace_session_file_paths(&workspace_sessions_root)? {
                    let Some(session) = read_workspace_session_record(&session_path)? else {
                        continue;
                    };
                    if session.id != session_id {
                        continue;
                    }
                    if matched_path.is_some() {
                        return Err(ProductStoreError::Io(
                            "workspace_session_ambiguous".to_string(),
                        ));
                    }
                    matched_path = Some(session_path);
                }
            }
        }

        matched_path.ok_or_else(|| ProductStoreError::NotFound {
            kind: "workspace_session",
            id: session_id.to_string(),
        })
    }

    fn story_specs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("story-specs")
    }

    fn design_specs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("design-specs")
    }

    fn work_items_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("work-items")
    }

    fn versions_root(&self, project_id: &str, issue_id: &str, entity_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("versions")
            .join(entity_id)
    }

    fn workspace_sessions_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-sessions")
    }
}

fn list_json_records<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, ProductStoreError> {
    let entries = json_file_paths(path)?;

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        records.push(read_json(&entry)?);
    }
    Ok(records)
}

fn count_json_files(path: &Path) -> Result<usize, ProductStoreError> {
    Ok(json_file_paths(path)?.len())
}

fn json_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        let entry_path = entry.path();
        if file_type.is_file()
            && entry_path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

fn list_workspace_session_records(
    path: &Path,
) -> Result<Vec<WorkspaceSessionRecord>, ProductStoreError> {
    let entries = workspace_session_file_paths(path)?;

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Some(record) = read_workspace_session_record(&entry)? {
            records.push(record);
        }
    }
    Ok(records)
}

fn workspace_session_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    Ok(json_file_paths(path)?
        .into_iter()
        .filter(|path| workspace_session_file_stem(path).is_some())
        .collect())
}

fn read_workspace_session_record(
    path: &Path,
) -> Result<Option<WorkspaceSessionRecord>, ProductStoreError> {
    let Some(file_id) = workspace_session_file_stem(path) else {
        return Ok(None);
    };
    let session: WorkspaceSessionRecord = read_json(path)?;
    if session.id == file_id {
        Ok(Some(session))
    } else {
        Ok(None)
    }
}

fn workspace_session_file_stem(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let suffix = stem.strip_prefix("workspace_session_")?;
    if suffix.is_empty() {
        return None;
    }
    Some(stem)
}

fn child_directories(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        if file_type.is_dir() {
            entries.push(entry.path());
        }
    }
    entries.sort();
    Ok(entries)
}

fn next_version_number(records: &[SpecVersionRecord]) -> Result<u32, ProductStoreError> {
    records
        .iter()
        .map(|record| record.version)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| ProductStoreError::Io("version sequence overflow".to_string()))
}

fn max_workspace_session_sequence(projects_root: &Path) -> Result<usize, ProductStoreError> {
    let mut max_sequence = 0usize;
    for project_path in child_directories(projects_root)? {
        let issues_root = project_path.join("issues");
        for issue_path in child_directories(&issues_root)? {
            let workspace_sessions_root = issue_path.join("workspace-sessions");
            for session_path in workspace_session_file_paths(&workspace_sessions_root)? {
                let Some(session) = read_workspace_session_record(&session_path)? else {
                    continue;
                };
                if let Some(sequence) = parse_sequential_id(&session.id, "workspace_session") {
                    max_sequence = max_sequence.max(sequence);
                }
            }
        }
    }
    Ok(max_sequence)
}

fn parse_sequential_id(value: &str, prefix: &str) -> Option<usize> {
    value
        .strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_prefix('_'))
        .and_then(|suffix| suffix.parse().ok())
}

fn ensure_target_absent(path: &Path) -> Result<(), ProductStoreError> {
    if path_exists(path)? {
        return Err(ProductStoreError::Io(format!(
            "refuse to overwrite {}",
            path.display()
        )));
    }
    Ok(())
}

fn path_is_regular_file(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

fn validate_relative_ids(values: &[String]) -> Result<(), ProductStoreError> {
    for value in values {
        validate_relative_id(value)?;
    }
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("try_exists {}: {error}", path.display())))
}
