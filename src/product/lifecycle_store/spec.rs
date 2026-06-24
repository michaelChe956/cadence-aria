use chrono::Utc;
use std::path::PathBuf;

use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    DesignSpecRecord, LifecycleConfirmationStatus, ProjectProviderDefaultsRecord,
    SpecVersionRecord, StorySpecRecord, WorkspaceType,
};

use super::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateProjectProviderDefaultsInput,
    CreateStorySpecInput, LifecycleStore, count_json_files, delete_required_file,
    ensure_target_absent, list_json_records, path_is_regular_file, remove_dir_all_if_exists,
    validate_relative_ids,
};

pub(crate) enum ExistingSpecRecord {
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

    pub fn delete_story_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        story_spec_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(story_spec_id)?;

        delete_required_file(
            &self
                .story_specs_root(project_id, issue_id)
                .join(format!("{story_spec_id}.json")),
            "story_spec",
            story_spec_id,
        )?;
        remove_dir_all_if_exists(&self.versions_root(project_id, issue_id, story_spec_id))?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            story_spec_id,
            WorkspaceType::Story,
        )
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

    pub fn delete_design_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        design_spec_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(design_spec_id)?;

        delete_required_file(
            &self
                .design_specs_root(project_id, issue_id)
                .join(format!("{design_spec_id}.json")),
            "design_spec",
            design_spec_id,
        )?;
        remove_dir_all_if_exists(&self.versions_root(project_id, issue_id, design_spec_id))?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            design_spec_id,
            WorkspaceType::Design,
        )
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
        let version = super::next_version_number(&versions)?;
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

    pub fn update_spec_confirmation_status(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
        status: LifecycleConfirmationStatus,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(entity_id)?;

        let spec = self.load_existing_spec(project_id, issue_id, entity_id)?;
        let updated_at = Utc::now().to_rfc3339();
        match spec {
            ExistingSpecRecord::Story {
                path,
                record: mut story,
            } => {
                story.confirmation_status = status;
                story.updated_at = updated_at;
                write_json(&path, &story)
            }
            ExistingSpecRecord::Design {
                path,
                record: mut design,
            } => {
                design.confirmation_status = status;
                design.updated_at = updated_at;
                write_json(&path, &design)
            }
        }
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

    pub(crate) fn load_existing_spec(
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

    pub(crate) fn update_spec_current_version(
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
}
