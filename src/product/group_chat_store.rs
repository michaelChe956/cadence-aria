use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::group_chat_engine::timeline;
use crate::product::group_chat_engine::types::{GroupChatSessionRecord, RoomEvent};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

/// 群聊会话的时间线与快照存储。
///
/// 时间线事件是唯一事实来源，快照仅用于减少恢复时的基础读取成本。
#[derive(Debug, Clone)]
pub struct GroupChatStore {
    paths: ProductAppPaths,
}

impl GroupChatStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// 将事件追加到 timeline.jsonl，并返回本次事件的单调递增序号。
    ///
    /// 时间线 fsync 成功后更新已有快照；两步之间崩溃时，时间线仍可完整恢复。
    pub fn append_event(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        event: RoomEvent,
    ) -> Result<u64, ProductStoreError> {
        validate_ids(project_id, issue_id, session_id)?;
        let timeline_path = self.timeline_path(project_id, issue_id, session_id);
        let entries = timeline::read_entries(&timeline_path)?;
        let seq = timeline::next_seq(&entries);
        timeline::append_event(&timeline_path, seq, &event)?;

        // 事件 fsync 成功后才更新快照；两步之间崩溃时，时间线仍可完整恢复。
        let session_path = self.session_path(project_id, issue_id, session_id);
        if session_path.exists() {
            let mut session: GroupChatSessionRecord = read_json(&session_path)?;
            if session.project_id != project_id
                || session.issue_id != issue_id
                || session.id != session_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "group_chat_session",
                    id: session_id.to_string(),
                });
            }
            let updated_entries = timeline::read_entries(&timeline_path)?;
            timeline::update_role_cursors(&mut session.roles, &updated_entries);
            write_json(&session_path, &session)?;
        }

        Ok(seq)
    }

    /// 读取快照后重放完整事件流，恢复事件权威的 seen_cursor。
    pub fn load_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<GroupChatSessionRecord, ProductStoreError> {
        validate_ids(project_id, issue_id, session_id)?;
        let session_path = self.session_path(project_id, issue_id, session_id);
        let mut session: GroupChatSessionRecord = read_json(&session_path)?;
        if session.project_id != project_id
            || session.issue_id != issue_id
            || session.id != session_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "group_chat_session",
                id: session_id.to_string(),
            });
        }

        let entries =
            timeline::read_entries(&self.timeline_path(project_id, issue_id, session_id))?;
        timeline::replay_role_cursors(&mut session.roles, &entries);
        Ok(session)
    }

    /// 返回下一条时间线事件将使用的序号。
    pub fn next_event_seq(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<u64, ProductStoreError> {
        validate_ids(project_id, issue_id, session_id)?;
        Ok(timeline::next_seq(&timeline::read_entries(
            &self.timeline_path(project_id, issue_id, session_id),
        )?))
    }

    /// 读取会话的完整追加式时间线，按事件序号升序返回。
    ///
    /// coordinator 在每个 agent turn 前以此建立 freshness 快照；具体序号仍由
    /// 存储层管理，因此此接口只暴露业务事件本身。
    pub fn load_events(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<Vec<RoomEvent>, ProductStoreError> {
        validate_ids(project_id, issue_id, session_id)?;
        Ok(
            timeline::read_entries(&self.timeline_path(project_id, issue_id, session_id))?
                .into_iter()
                .map(|(_, event)| event)
                .collect(),
        )
    }

    /// 原子替换 session.json 快照。调用方应只在时间线追加成功后调用。
    pub fn save_session_snapshot(
        &self,
        session: &GroupChatSessionRecord,
    ) -> Result<(), ProductStoreError> {
        validate_ids(&session.project_id, &session.issue_id, &session.id)?;
        write_json(
            &self.session_path(&session.project_id, &session.issue_id, &session.id),
            session,
        )
    }

    fn session_root(&self, project_id: &str, issue_id: &str, session_id: &str) -> PathBuf {
        self.paths
            .group_chat_session_root(project_id, issue_id, session_id)
    }

    fn timeline_path(&self, project_id: &str, issue_id: &str, session_id: &str) -> PathBuf {
        self.session_root(project_id, issue_id, session_id)
            .join("timeline.jsonl")
    }

    fn session_path(&self, project_id: &str, issue_id: &str, session_id: &str) -> PathBuf {
        self.session_root(project_id, issue_id, session_id)
            .join("session.json")
    }
}

fn validate_ids(
    project_id: &str,
    issue_id: &str,
    session_id: &str,
) -> Result<(), ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(issue_id)?;
    validate_relative_id(session_id)
}
