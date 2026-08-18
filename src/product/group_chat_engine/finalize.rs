use chrono::Utc;
use thiserror::Error;

use crate::product::group_chat_store::GroupChatStore;
use crate::product::issue_store::IssueStore;
use crate::product::lifecycle_store::derivation_guard::{
    DerivationGuardError, validate_design_finalize_allowed,
};
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateStorySpecInput,
    CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::models::{
    LifecycleConfirmationStatus, SessionOrigin, WorkspaceSessionStatus, WorkspaceType,
};
use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion};

use super::types::{
    ArtifactLine, ArtifactLineKind, DraftSlotKey, GroupChatSessionRecord, RoleInstance, RoomEvent,
};

/// 群聊定稿所需的存储与业务依赖。
#[derive(Debug, Clone)]
pub struct FinalizeService {
    pub lifecycle: LifecycleStore,
    pub issue_store: IssueStore,
    pub group_chat: GroupChatStore,
}

impl FinalizeService {
    pub fn new(
        lifecycle: LifecycleStore,
        issue_store: IssueStore,
        group_chat: GroupChatStore,
    ) -> Self {
        Self {
            lifecycle,
            issue_store,
            group_chat,
        }
    }

    /// 将一个产物线定稿到生命周期实体，并同步看板桥接会话。
    pub fn finalize_line(&self, input: FinalizeInput) -> Result<RoomEvent, FinalizeError> {
        let mut session =
            self.group_chat
                .load_session(&input.project_id, &input.issue_id, &input.session_id)?;
        let line_index = session
            .artifact_lines
            .iter()
            .position(|line| line.kind == input.line_kind)
            .ok_or(FinalizeError::LineNotFound(input.line_kind))?;
        let mut line = session.artifact_lines[line_index].clone();

        let story_entity_id = if input.line_kind == ArtifactLineKind::DesignSpec {
            Some(self.resolve_story_entity_id(&session, &line, &input)?)
        } else {
            None
        };
        if let Some(story_entity_id) = story_entity_id.as_deref() {
            validate_design_finalize_allowed(
                &self.lifecycle,
                &input.project_id,
                &input.issue_id,
                story_entity_id,
            )
            .map_err(FinalizeError::DerivationGuard)?;
        }

        let included_slots = selected_slots(&line, input.included_slots_override.as_ref())?;
        let markdown = markdown_for_line(&line, &included_slots)?;
        let (entity_id, spec_version, provider) = match input.line_kind {
            ArtifactLineKind::IssueRefinement => {
                let issue = self.issue_store.update_description(
                    &input.project_id,
                    &input.issue_id,
                    markdown.clone(),
                    input.confirmed_by.as_deref().unwrap_or("group_chat"),
                )?;
                (issue.id, None, author_provider(&session.roles)?)
            }
            ArtifactLineKind::StorySpec => {
                let entity_id = self.ensure_story_entity(&input, &line)?;
                let provider = author_provider(&session.roles)?;
                let version = self.lifecycle.append_version(AppendSpecVersionInput {
                    project_id: input.project_id.clone(),
                    issue_id: input.issue_id.clone(),
                    entity_id: entity_id.clone(),
                    markdown: markdown.clone(),
                    provider_run_refs: input.provider_run_refs.clone(),
                    review_refs: input.review_refs.clone(),
                    confirmed_by: input.confirmed_by.clone(),
                })?;
                self.lifecycle.update_spec_confirmation_status(
                    &input.project_id,
                    &input.issue_id,
                    &entity_id,
                    LifecycleConfirmationStatus::Confirmed,
                )?;
                (entity_id, Some(version), provider)
            }
            ArtifactLineKind::DesignSpec => {
                let entity_id =
                    self.ensure_design_entity(&input, &line, story_entity_id.as_deref())?;
                let provider = author_provider(&session.roles)?;
                let version = self.lifecycle.append_version(AppendSpecVersionInput {
                    project_id: input.project_id.clone(),
                    issue_id: input.issue_id.clone(),
                    entity_id: entity_id.clone(),
                    markdown: markdown.clone(),
                    provider_run_refs: input.provider_run_refs.clone(),
                    review_refs: input.review_refs.clone(),
                    confirmed_by: input.confirmed_by.clone(),
                })?;
                self.lifecycle.update_spec_confirmation_status(
                    &input.project_id,
                    &input.issue_id,
                    &entity_id,
                    LifecycleConfirmationStatus::Confirmed,
                )?;
                (entity_id, Some(version), provider)
            }
        };

        // Issue 澄清只更新 IssueStore，不创建看板桥接。
        let version_id = spec_version
            .as_ref()
            .map(|version| version.id.clone())
            .unwrap_or_else(|| {
                format!(
                    "issue-revision-{}",
                    Utc::now().timestamp_nanos_opt().unwrap_or_default()
                )
            });
        let event_seq = self.group_chat.next_event_seq(
            &input.project_id,
            &input.issue_id,
            &input.session_id,
        )?;

        if let Some(version) = spec_version.as_ref() {
            line.entity_id = Some(entity_id.clone());
            let bridge_session_id = self.ensure_bridge_session(
                &input,
                &line,
                &entity_id,
                &provider,
                story_entity_id.as_deref(),
            )?;
            let reviewer = if input.review_refs.is_empty() {
                None
            } else {
                reviewer_provider(&session.roles)
            };
            let artifact = ArtifactVersion {
                version: version.version,
                payload: ArtifactPayload::Markdown {
                    markdown,
                    diff: None,
                },
                generated_by: provider,
                reviewed_by: reviewer,
                review_verdict: None,
                confirmed_by: input.confirmed_by.clone(),
                is_current: true,
                created_at: Utc::now().to_rfc3339(),
                source_node_id: event_seq.to_string(),
            };
            self.append_current_artifact(&bridge_session_id, artifact)?;
            line.bridge_session_id = Some(bridge_session_id);
        }

        line.finalized_versions.push(version_id.clone());
        session.artifact_lines[line_index] = line;
        let event = RoomEvent::FinalizeEvent {
            artifact_line: input.line_kind,
            version: version_id,
            included_slots,
        };
        self.group_chat.append_event(
            &input.project_id,
            &input.issue_id,
            &input.session_id,
            event.clone(),
        )?;
        session.status = super::types::GroupChatSessionStatus::Finalized;
        session.updated_at = Utc::now().to_rfc3339();
        self.group_chat.save_session_snapshot(&session)?;
        Ok(event)
    }

    /// 修复实体已有版本但群聊桥接会话被删除的异常状态。
    pub fn repair_bridge_if_missing(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        line_kind: ArtifactLineKind,
    ) -> Result<bool, FinalizeError> {
        if line_kind == ArtifactLineKind::IssueRefinement {
            return Ok(false);
        }
        let mut session = self
            .group_chat
            .load_session(project_id, issue_id, session_id)?;
        let index = session
            .artifact_lines
            .iter()
            .position(|line| line.kind == line_kind)
            .ok_or(FinalizeError::LineNotFound(line_kind))?;
        let line = session.artifact_lines[index].clone();
        let entity_id = line
            .entity_id
            .clone()
            .ok_or(FinalizeError::MissingEntity(line_kind))?;
        if line.finalized_versions.is_empty() {
            return Ok(false);
        }
        if let Some(bridge_id) = line.bridge_session_id.as_deref()
            && self.lifecycle.get_workspace_session(bridge_id).is_ok()
        {
            return Ok(false);
        }
        let provider = author_provider(&session.roles)?;
        let bridge_id = self.ensure_bridge_session(
            &FinalizeInput {
                project_id: project_id.to_owned(),
                issue_id: issue_id.to_owned(),
                session_id: session_id.to_owned(),
                line_kind,
                included_slots_override: None,
                confirmed_by: None,
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
            },
            &line,
            &entity_id,
            &provider,
            None,
        )?;
        let versions = self
            .lifecycle
            .list_versions(project_id, issue_id, &entity_id)?;
        let existing_artifacts = self.lifecycle.list_artifact_versions(&bridge_id)?;
        for version in versions
            .into_iter()
            .filter(|version| line.finalized_versions.contains(&version.id))
            .filter(|version| {
                !existing_artifacts
                    .iter()
                    .any(|artifact| artifact.version == version.version)
            })
        {
            self.append_current_artifact(
                &bridge_id,
                ArtifactVersion {
                    version: version.version,
                    payload: ArtifactPayload::Markdown {
                        markdown: version.markdown,
                        diff: None,
                    },
                    generated_by: provider.clone(),
                    reviewed_by: None,
                    review_verdict: None,
                    confirmed_by: version.confirmed_by,
                    is_current: true,
                    created_at: version.created_at,
                    source_node_id: format!("repair:{}", version.id),
                },
            )?;
        }
        session.artifact_lines[index].bridge_session_id = Some(bridge_id);
        self.group_chat.save_session_snapshot(&session)?;
        Ok(true)
    }

    fn resolve_story_entity_id(
        &self,
        session: &GroupChatSessionRecord,
        line: &ArtifactLine,
        input: &FinalizeInput,
    ) -> Result<String, FinalizeError> {
        if let Some(design_id) = line.entity_id.as_deref()
            && let Some(design) = self
                .lifecycle
                .list_design_specs(&input.project_id, &input.issue_id)?
                .into_iter()
                .find(|design| design.id == design_id)
            && let Some(story_id) = design.story_spec_ids.first()
        {
            return Ok(story_id.clone());
        }
        self.lifecycle
            .list_story_specs(&input.project_id, &input.issue_id)?
            .into_iter()
            .next()
            .map(|story| story.id)
            .or_else(|| {
                session
                    .artifact_lines
                    .iter()
                    .find(|candidate| candidate.kind == ArtifactLineKind::StorySpec)
                    .and_then(|candidate| candidate.entity_id.clone())
            })
            .ok_or(FinalizeError::MissingStoryDependency)
    }

    fn ensure_story_entity(
        &self,
        input: &FinalizeInput,
        line: &ArtifactLine,
    ) -> Result<String, FinalizeError> {
        if let Some(id) = line.entity_id.as_deref() {
            return Ok(id.to_owned());
        }
        if let Some(existing) = self
            .lifecycle
            .list_story_specs(&input.project_id, &input.issue_id)?
            .into_iter()
            .next()
        {
            return Ok(existing.id);
        }
        let issue = self.issue_store.get(&input.project_id, &input.issue_id)?;
        Ok(self
            .lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: input.project_id.clone(),
                issue_id: input.issue_id.clone(),
                repository_id: issue.repo_id.unwrap_or_else(|| "group-chat".to_owned()),
                title: issue.title,
            })?
            .id)
    }

    fn ensure_design_entity(
        &self,
        input: &FinalizeInput,
        line: &ArtifactLine,
        story_id: Option<&str>,
    ) -> Result<String, FinalizeError> {
        if let Some(id) = line.entity_id.as_deref() {
            return Ok(id.to_owned());
        }
        if let Some(existing) = self
            .lifecycle
            .list_design_specs(&input.project_id, &input.issue_id)?
            .into_iter()
            .next()
        {
            return Ok(existing.id);
        }
        let story_id = story_id.ok_or(FinalizeError::MissingStoryDependency)?;
        Ok(self
            .lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: input.project_id.clone(),
                issue_id: input.issue_id.clone(),
                story_spec_ids: vec![story_id.to_owned()],
                title: "群聊 Design Spec".to_owned(),
            })?
            .id)
    }

    fn ensure_bridge_session(
        &self,
        input: &FinalizeInput,
        line: &ArtifactLine,
        entity_id: &str,
        provider: &crate::product::models::ProviderName,
        _story_id: Option<&str>,
    ) -> Result<String, FinalizeError> {
        if let Some(id) = line.bridge_session_id.as_deref()
            && self.lifecycle.get_workspace_session(id).is_ok()
        {
            return Ok(id.to_owned());
        }
        if let Some(existing) = self
            .lifecycle
            .list_workspace_sessions(&input.project_id, &input.issue_id)?
            .into_iter()
            .find(|session| {
                session.entity_id == entity_id
                    && session.origin == Some(SessionOrigin::GroupChat)
                    && session.workspace_type == workspace_type(input.line_kind)
            })
        {
            return Ok(existing.id);
        }
        let created = self
            .lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: input.project_id.clone(),
                issue_id: input.issue_id.clone(),
                entity_id: entity_id.to_owned(),
                workspace_type: workspace_type(input.line_kind),
                author_provider: provider.clone(),
                reviewer_provider: provider.clone(),
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
            })?;
        self.lifecycle
            .update_workspace_session_origin(&created.id, SessionOrigin::GroupChat)?;
        self.lifecycle
            .update_workspace_session_status(&created.id, WorkspaceSessionStatus::Confirmed)?;
        Ok(created.id)
    }

    fn append_current_artifact(
        &self,
        session_id: &str,
        artifact: ArtifactVersion,
    ) -> Result<(), FinalizeError> {
        self.lifecycle
            .append_artifact_version(session_id, artifact)?;
        Ok(())
    }
}

/// 一次定稿请求。草稿内容和角色信息从 `session_id` 对应的群聊快照读取。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeInput {
    pub project_id: String,
    pub issue_id: String,
    pub session_id: String,
    pub line_kind: ArtifactLineKind,
    pub included_slots_override: Option<Vec<DraftSlotKey>>,
    pub confirmed_by: Option<String>,
    pub provider_run_refs: Vec<String>,
    pub review_refs: Vec<String>,
}

#[derive(Debug, Error)]
pub enum FinalizeError {
    #[error("产物线不存在：{0:?}")]
    LineNotFound(ArtifactLineKind),
    #[error("缺少可定稿草稿槽")]
    NoDraft,
    #[error("草稿槽不存在：{0}")]
    SlotNotFound(String),
    #[error("缺少生命周期实体：{0:?}")]
    MissingEntity(ArtifactLineKind),
    #[error("缺少 Story Spec 前置实体")]
    MissingStoryDependency,
    #[error("派生约束失败：{0}")]
    DerivationGuard(#[source] DerivationGuardError),
    #[error("产品存储错误：{0}")]
    Store(String),
}

impl From<crate::product::json_store::ProductStoreError> for FinalizeError {
    fn from(error: crate::product::json_store::ProductStoreError) -> Self {
        Self::Store(error.to_string())
    }
}

fn selected_slots(
    line: &ArtifactLine,
    override_slots: Option<&Vec<DraftSlotKey>>,
) -> Result<Vec<DraftSlotKey>, FinalizeError> {
    let slots = override_slots.cloned().unwrap_or_else(|| {
        line.drafts
            .iter()
            .filter(|slot| slot.current.is_some())
            .map(|slot| slot.slot_key.clone())
            .collect()
    });
    for key in &slots {
        if !line.drafts.iter().any(|slot| slot.slot_key == *key) {
            return Err(FinalizeError::SlotNotFound(key.0.clone()));
        }
    }
    if slots.is_empty() {
        return Err(FinalizeError::NoDraft);
    }
    Ok(slots)
}

fn markdown_for_line(
    line: &ArtifactLine,
    included_slots: &[DraftSlotKey],
) -> Result<String, FinalizeError> {
    let mut parts = Vec::new();
    for key in included_slots {
        if let Some(slot) = line.drafts.iter().find(|slot| slot.slot_key == *key)
            && let Some(draft) = &slot.current
        {
            parts.push(draft.markdown.clone());
        }
    }
    if parts.is_empty() {
        return Err(FinalizeError::NoDraft);
    }
    Ok(parts.join("\n\n"))
}

fn author_provider(
    roles: &[RoleInstance],
) -> Result<crate::product::models::ProviderName, FinalizeError> {
    roles
        .iter()
        .find(|role| role.role_key == super::types::GroupChatRoleKey::Author)
        .or_else(|| roles.first())
        .map(|role| role.provider.clone())
        .ok_or_else(|| FinalizeError::Store("群聊没有执笔角色".to_owned()))
}

fn reviewer_provider(roles: &[RoleInstance]) -> Option<crate::product::models::ProviderName> {
    roles
        .iter()
        .find(|role| role.role_key == super::types::GroupChatRoleKey::Reviewer)
        .map(|role| role.provider.clone())
}

fn workspace_type(line_kind: ArtifactLineKind) -> WorkspaceType {
    match line_kind {
        ArtifactLineKind::StorySpec => WorkspaceType::Story,
        ArtifactLineKind::DesignSpec => WorkspaceType::Design,
        ArtifactLineKind::IssueRefinement => WorkspaceType::Story,
    }
}
