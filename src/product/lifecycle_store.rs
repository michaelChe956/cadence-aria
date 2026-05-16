use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::de::DeserializeOwned;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    DesignKind, DesignSpecRecord, LifecycleConfirmationStatus, LifecycleWorkItemRecord,
    ProjectProviderDefaultsRecord, ProviderName, SpecVersionRecord, StorySpecRecord,
    WorkItemPlanStatus, WorkItemStatus, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceType,
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

        let root = self.versions_root(&input.project_id, &input.issue_id, &input.entity_id);
        let existing_len = count_json_files(&root)?;
        let id = next_sequential_id("version", existing_len);
        let version = existing_len as u32 + 1;
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

        write_json(&root.join(format!("{id}.json")), &record)?;
        self.update_spec_current_version(
            &record.project_id,
            &record.issue_id,
            &record.entity_id,
            version,
            now,
        )?;
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
        let id = next_sequential_id("workspace_session", count_json_files(&root)?);
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

        write_json(&root.join(format!("{id}.json")), &session)?;
        Ok(session)
    }

    pub fn list_workspace_sessions(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<WorkspaceSessionRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.workspace_sessions_root(project_id, issue_id))
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

    fn update_spec_current_version(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
        version: u32,
        updated_at: String,
    ) -> Result<(), ProductStoreError> {
        let story_path = self
            .story_specs_root(project_id, issue_id)
            .join(format!("{entity_id}.json"));
        if path_exists(&story_path)? {
            let mut story: StorySpecRecord = read_json(&story_path)?;
            story.current_version = Some(version);
            story.updated_at = updated_at;
            return write_json(&story_path, &story);
        }

        let design_path = self
            .design_specs_root(project_id, issue_id)
            .join(format!("{entity_id}.json"));
        if path_exists(&design_path)? {
            let mut design: DesignSpecRecord = read_json(&design_path)?;
            design.current_version = Some(version);
            design.updated_at = updated_at;
            return write_json(&design_path, &design);
        }

        Ok(())
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
        let entry_path = entry.path();
        if entry_path.extension().and_then(|value| value.to_str()) == Some("json") {
            entries.push(entry_path);
        }
    }
    entries.sort();

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        records.push(read_json(&entry)?);
    }
    Ok(records)
}

fn count_json_files(path: &Path) -> Result<usize, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(0);
    }

    fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
        .try_fold(0usize, |count, entry| {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
            })?;
            let is_json = entry.path().extension().and_then(|value| value.to_str()) == Some("json");
            Ok(if is_json { count + 1 } else { count })
        })
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
