// session 定位与 timeline 根解析（从 workspace.rs 整块搬移，逐字未改；
// 主模块已近行数上限，按职责搬移不压行）。
//
// `include!` 复用 workspace.rs 既有搬移惯例（见同目录
// `workspace_timeline_artifacts.inc.rs`）：展开于同一模块，可见性/签名零变化。
impl LifecycleStore {
    fn next_workspace_session_id(&self) -> Result<String, ProductStoreError> {
        // D1.3a：全项目 session 最大序号与耐久高水位取 max 再增（D1.0
        // `allocate_workspace_session_id`）；已分配（含已删实体）ID 不复用。
        self.entity_operation_store()
            .allocate_workspace_session_id()
    }

    pub(super) fn find_workspace_session_path(
        &self,
        session_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_file_name = format!("{session_id}.json");
        let mut matched_path = None;
        for project_path in child_directories(&self.paths.projects_root())? {
            let issues_root = project_path.join("issues");
            for issue_path in child_directories(&issues_root)? {
                let session_path = issue_path
                    .join("workspace-sessions")
                    .join(&session_file_name);
                if !path_is_regular_file(&session_path)? {
                    continue;
                }
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

    pub(crate) fn workspace_timeline_root_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(session_id)?;
        Ok(self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-timelines")
            .join(session_id))
    }
}
