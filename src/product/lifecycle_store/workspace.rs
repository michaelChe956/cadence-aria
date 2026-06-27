use std::path::PathBuf;

use chrono::Utc;

use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    ProviderConversationRef, WorkspaceMessageRecord, WorkspaceSessionRecord,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::web::workspace_ws_types::{ArtifactVersion, TimelineNode};

use super::{
    CreateWorkspaceSessionInput, LifecycleStore, child_directories, json_file_paths,
    list_workspace_session_records, path_exists, read_workspace_session_record,
    remove_dir_all_if_exists, remove_file_if_exists, workspace_session_file_paths,
};

impl LifecycleStore {
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
            provider_conversations: Vec::new(),
            messages: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        };

        let target_path = root.join(format!("{id}.json"));
        super::ensure_target_absent(&target_path)?;
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

    pub fn replace_workspace_messages(
        &self,
        session_id: &str,
        messages: Vec<WorkspaceMessageRecord>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.messages = messages;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn replace_workspace_provider_conversations(
        &self,
        session_id: &str,
        provider_conversations: Vec<ProviderConversationRef>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.provider_conversations = provider_conversations;
        session.updated_at = Utc::now().to_rfc3339();
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

    pub fn update_workspace_session_providers(
        &self,
        session_id: &str,
        author_provider: crate::product::models::ProviderName,
        reviewer_provider: crate::product::models::ProviderName,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.author_provider = author_provider;
        session.reviewer_provider = reviewer_provider;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn truncate_workspace_session_messages(
        &self,
        session_id: &str,
        keep_count: usize,
        status: WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.messages.truncate(keep_count);
        session.status = status;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn save_timeline_nodes(
        &self,
        session_id: &str,
        nodes: &[TimelineNode],
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_nodes.json");
        write_json(&path, &nodes)
    }

    pub fn load_timeline_nodes(
        &self,
        session_id: &str,
    ) -> Result<Vec<TimelineNode>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_nodes.json");
        if !path_exists(&path)? {
            return Ok(Vec::new());
        }
        read_json(&path)
    }

    pub fn save_node_detail(
        &self,
        session_id: &str,
        node_id: &str,
        detail: &crate::product::models::NodeDetail,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_node_details")
            .join(format!("{node_id}.json"));
        write_json(&path, detail)
    }

    pub fn load_node_detail(
        &self,
        session_id: &str,
        node_id: &str,
    ) -> Result<crate::product::models::NodeDetail, ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_node_details")
            .join(format!("{node_id}.json"));
        if !path_exists(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "node_detail",
                id: format!("{session_id}/{node_id}"),
            });
        }
        read_json(&path)
    }

    pub fn list_node_detail_ids(&self, session_id: &str) -> Result<Vec<String>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let dir = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_node_details");
        let entries = json_file_paths(&dir)?;
        let mut ids = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Some(stem) = entry.file_stem() {
                ids.push(stem.to_string_lossy().to_string());
            }
        }
        Ok(ids)
    }

    pub fn append_artifact_version(
        &self,
        session_id: &str,
        version: ArtifactVersion,
    ) -> Result<(), ProductStoreError> {
        let mut versions = self.list_artifact_versions(session_id)?;
        versions.push(version);
        self.save_artifact_versions(session_id, &versions)
    }

    pub fn list_artifact_versions(
        &self,
        session_id: &str,
    ) -> Result<Vec<ArtifactVersion>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("artifact_versions.json");
        if !path_exists(&path)? {
            return Ok(Vec::new());
        }
        read_json(&path)
    }

    pub fn save_artifact_versions(
        &self,
        session_id: &str,
        versions: &[ArtifactVersion],
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("artifact_versions.json");
        write_json(&path, &versions)
    }

    pub(crate) fn delete_workspace_sessions_for_entity(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
        workspace_type: WorkspaceType,
    ) -> Result<(), ProductStoreError> {
        let sessions_root = self.workspace_sessions_root(project_id, issue_id);
        let timeline_root = self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-timelines");
        for session in self
            .list_workspace_sessions(project_id, issue_id)?
            .into_iter()
            .filter(|session| {
                session.entity_id == entity_id && session.workspace_type == workspace_type
            })
        {
            remove_dir_all_if_exists(&timeline_root.join(&session.id))?;
            remove_file_if_exists(&sessions_root.join(format!("{}.json", session.id)))?;
        }
        Ok(())
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

    pub(crate) fn workspace_timeline_root_for_session(
        &self,
        session_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        let session_path = self.find_workspace_session_path(session_id)?;
        let sessions_root = session_path.parent().ok_or_else(|| {
            ProductStoreError::Io(format!(
                "workspace session path has no parent: {}",
                session_path.display()
            ))
        })?;
        let issue_root = sessions_root.parent().ok_or_else(|| {
            ProductStoreError::Io(format!(
                "workspace sessions path has no issue parent: {}",
                sessions_root.display()
            ))
        })?;
        Ok(issue_root.join("workspace-timelines").join(session_id))
    }
}

fn max_workspace_session_sequence(
    projects_root: &std::path::Path,
) -> Result<usize, ProductStoreError> {
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
