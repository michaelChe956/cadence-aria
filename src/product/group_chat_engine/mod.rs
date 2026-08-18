pub mod agent_turn;
pub mod claims;
pub mod context;
pub mod coordinator;
pub mod finalize;
pub mod prompts;
pub mod roles;
pub mod settings;
pub mod timeline;
pub mod triage;
pub mod types;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;

use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::app_paths::ProductAppPaths;
use crate::product::group_chat_store::GroupChatStore;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::ProviderName;

use self::coordinator::{Coordinator, CoordinatorError, CoordinatorRunSummary};
use self::finalize::{FinalizeError, FinalizeInput, FinalizeService};
use self::roles::{
    DESIGN_BACKEND_SLOT, DESIGN_FRONTEND_SLOT, DESIGN_SUMMARY_SLOT, ISSUE_FULL_SLOT,
    STORY_FULL_SLOT, default_lineup,
};
use self::triage::RuleRouter;
use self::types::{
    ArtifactLine, ArtifactLineKind, DraftSlot, DraftSlotKey, GroupChatRoleKey,
    GroupChatSessionRecord, GroupChatSessionStatus, RoleInstance,
};

/// 群聊 HTTP/WS 共享的薄引擎门面。
///
/// 会话与时间线由 `GroupChatStore` 持久化，Coordinator 负责消息闭环，FinalizeService
/// 负责实体定稿和看板桥接。当前 triage provider 配置仅落盘并回显；实际 LLM triage
/// 接线待后续任务，运行时仍使用确定性的 `RuleRouter`。
pub struct GroupChatEngine {
    pub store: GroupChatStore,
    coordinator: Arc<Mutex<Coordinator>>,
    session_creation_lock: std::sync::Mutex<()>,
    pub finalize: FinalizeService,
    pub providers: Arc<ProviderRegistry>,
}

impl GroupChatEngine {
    pub fn new(paths: ProductAppPaths, providers: Arc<ProviderRegistry>) -> Self {
        let store = GroupChatStore::new(paths.clone());
        let adapters = providers
            .available_names()
            .into_iter()
            .filter_map(|provider| providers.get(&provider).map(|adapter| (provider, adapter)))
            .collect::<HashMap<_, _>>();
        let coordinator = Coordinator::new(store.clone(), adapters, Box::new(RuleRouter::new()));
        let finalize = FinalizeService::new(
            LifecycleStore::new(paths.clone()),
            IssueStore::new(paths.clone()),
            store.clone(),
        );
        Self {
            store,
            coordinator: Arc::new(Mutex::new(coordinator)),
            session_creation_lock: std::sync::Mutex::new(()),
            finalize,
            providers,
        }
    }

    pub fn create_or_get_session(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<(GroupChatSessionRecord, bool), ProductStoreError> {
        let _creation_guard = self
            .session_creation_lock
            .lock()
            .expect("群聊会话创建锁可用");
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let issues = IssueStore::new(self.finalize.lifecycle.app_paths());
        issues.get(project_id, issue_id)?;
        if let Some(session) = self.store.find_session_for_issue(project_id, issue_id)? {
            return Ok((session, false));
        }

        let session_id = format!("group_chat_session_{}", uuid::Uuid::new_v4().simple());
        let now = Utc::now().to_rfc3339();
        let provider = self.default_provider();
        let roles = default_lineup()
            .into_iter()
            .enumerate()
            .map(|(index, (role_key, permission_mode))| RoleInstance {
                id: format!("role_{}", index + 1),
                role_key,
                provider: provider.clone(),
                display_name: role_display_name(role_key).to_owned(),
                permission_mode,
                seen_cursor: 0,
                injection_watermark: 0,
            })
            .collect();
        let session = GroupChatSessionRecord {
            id: session_id,
            project_id: project_id.to_owned(),
            issue_id: issue_id.to_owned(),
            status: GroupChatSessionStatus::Active,
            roles,
            artifact_lines: default_artifact_lines(),
            triage_provider: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.store.save_session_snapshot(&session)?;
        Ok((session, true))
    }

    pub fn load_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<GroupChatSessionRecord, ProductStoreError> {
        let session = self.store.load_session(project_id, issue_id, session_id)?;
        for line in &session.artifact_lines {
            if line.kind != ArtifactLineKind::IssueRefinement && !line.finalized_versions.is_empty()
            {
                self.finalize
                    .repair_bridge_if_missing(project_id, issue_id, session_id, line.kind)
                    .map_err(|error| ProductStoreError::Io(error.to_string()))?;
            }
        }
        self.store.load_session(project_id, issue_id, session_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_role(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        role_key: GroupChatRoleKey,
        provider: ProviderName,
        display_name: Option<String>,
        permission_mode: Option<ProviderPermissionMode>,
    ) -> Result<GroupChatSessionRecord, ProductStoreError> {
        if self.providers.get(&provider).is_none() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "group_chat_role",
                reason: format!("provider adapter is unavailable: {provider:?}"),
            });
        }
        let mut session = self.store.load_session(project_id, issue_id, session_id)?;
        let role_id = format!("role_{}", session.roles.len() + 1);
        session.roles.push(RoleInstance {
            id: role_id,
            role_key,
            provider,
            display_name: display_name.unwrap_or_else(|| role_display_name(role_key).to_owned()),
            permission_mode: permission_mode.unwrap_or({
                if matches!(
                    role_key,
                    GroupChatRoleKey::Author
                        | GroupChatRoleKey::FrontendDesign
                        | GroupChatRoleKey::BackendDesign
                ) {
                    ProviderPermissionMode::Auto
                } else {
                    ProviderPermissionMode::Supervised
                }
            }),
            seen_cursor: 0,
            injection_watermark: 0,
        });
        session.updated_at = Utc::now().to_rfc3339();
        self.store.save_session_snapshot(&session)?;
        Ok(session)
    }

    pub async fn on_user_message(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        text: &str,
        mentions: Vec<String>,
        draft_slot: Option<DraftSlotKey>,
    ) -> Result<CoordinatorRunSummary, CoordinatorError> {
        let coordinator = self.coordinator.clone();
        let project_id = project_id.to_owned();
        let issue_id = issue_id.to_owned();
        let session_id = session_id.to_owned();
        let text = text.to_owned();
        tokio::spawn(async move {
            coordinator
                .lock()
                .await
                .on_user_message_with_draft(
                    &project_id,
                    &issue_id,
                    &session_id,
                    &text,
                    mentions,
                    draft_slot,
                )
                .await
        })
        .await
        .map_err(|error| {
            CoordinatorError::Store(ProductStoreError::Io(format!("群聊协调任务失败：{error}")))
        })?
    }

    pub fn finalize_line(&self, input: FinalizeInput) -> Result<types::RoomEvent, FinalizeError> {
        self.finalize.finalize_line(input)
    }

    fn default_provider(&self) -> ProviderName {
        self.providers
            .available_names()
            .into_iter()
            .find(|provider| *provider == ProviderName::Fake)
            .or_else(|| self.providers.available_names().into_iter().next())
            .unwrap_or(ProviderName::Fake)
    }
}

fn role_display_name(role_key: GroupChatRoleKey) -> &'static str {
    match role_key {
        GroupChatRoleKey::Author => "作者",
        GroupChatRoleKey::FrontendDesign => "前端设计",
        GroupChatRoleKey::BackendDesign => "后端设计",
        GroupChatRoleKey::Reviewer => "审稿人",
        GroupChatRoleKey::Researcher => "研究员",
    }
}

fn default_artifact_lines() -> Vec<ArtifactLine> {
    vec![
        line(ArtifactLineKind::IssueRefinement, &[ISSUE_FULL_SLOT]),
        line(ArtifactLineKind::StorySpec, &[STORY_FULL_SLOT]),
        line(
            ArtifactLineKind::DesignSpec,
            &[
                DESIGN_FRONTEND_SLOT,
                DESIGN_BACKEND_SLOT,
                DESIGN_SUMMARY_SLOT,
            ],
        ),
    ]
}

fn line(kind: ArtifactLineKind, slots: &[&str]) -> ArtifactLine {
    ArtifactLine {
        kind,
        drafts: slots
            .iter()
            .map(|slot| DraftSlot {
                slot_key: DraftSlotKey((*slot).to_owned()),
                current: None,
                claim: None,
            })
            .collect(),
        finalized_versions: Vec::new(),
        entity_id: None,
        bridge_session_id: None,
    }
}

#[cfg(test)]
mod tests;
