impl LifecycleStore {
    pub fn save_timeline_nodes(
        &self,
        session_id: &str,
        nodes: &[TimelineNode],
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        let (_, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let path = self
                .workspace_timeline_root_for_session(session_id)?
                .join("timeline_nodes.json");
            write_json(&path, &nodes)
        })
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

    pub fn load_timeline_nodes_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<Vec<TimelineNode>, ProductStoreError> {
        let path = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
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
        let (_, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let path = self
                .workspace_timeline_root_for_session(session_id)?
                .join("timeline_node_details")
                .join(format!("{node_id}.json"));
            write_json(&path, detail)
        })
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

    pub(crate) fn delete_node_detail(
        &self,
        session_id: &str,
        node_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_node_details")
            .join(format!("{node_id}.json"));
        remove_file_if_exists(&path)
    }

    pub fn load_node_detail_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        node_id: &str,
    ) -> Result<crate::product::models::NodeDetail, ProductStoreError> {
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
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

    pub fn list_node_detail_ids_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<Vec<String>, ProductStoreError> {
        let dir = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
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
        mut version: ArtifactVersion,
    ) -> Result<(), ProductStoreError> {
        self.with_artifact_versions_write(session_id, |ctx| {
            // D1：锁内分配 max(raw)+1，覆盖 caller 传入的 version。
            version.version = ctx.next_version()?;
            let mut proposed = ctx.raw().to_vec();
            proposed.push(version);
            ctx.save(&proposed).map(|_| ())
        })
    }

    pub fn list_artifact_versions(
        &self,
        session_id: &str,
    ) -> Result<Vec<ArtifactVersion>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("artifact_versions.json");
        Self::read_artifact_versions_at(&path)
    }

    pub fn list_artifact_versions_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<Vec<ArtifactVersion>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
            .join("artifact_versions.json");
        Self::read_artifact_versions_at(&path)
    }

    pub fn save_artifact_versions(
        &self,
        session_id: &str,
        versions: &[ArtifactVersion],
    ) -> Result<(), ProductStoreError> {
        self.with_artifact_versions_write(session_id, |ctx| ctx.save(versions).map(|_| ()))
    }

    fn read_artifact_versions_at(
        path: &std::path::Path,
    ) -> Result<Vec<ArtifactVersion>, ProductStoreError> {
        if !path_exists(path)? {
            return Ok(Vec::new());
        }
        read_json(path)
    }

    /// 在 `artifact_versions.json` sidecar 排它锁内运行同步 `operation`。
    /// 锁序：scope 准入 → artifact 锁（恒为最内层）。闭包同步，结构上不可能跨 await。
    /// 闭包内禁止调用 `save_artifact_versions` / `append_artifact_version`、嵌套本方法
    /// （同一 sidecar 的 flock 同线程二次获取会永久阻塞），也禁止写 session record
    /// （会形成 artifact → session 逆序）。
    pub(crate) fn with_artifact_versions_write<T>(
        &self,
        session_id: &str,
        operation: impl FnOnce(&mut ArtifactVersionsWrite<'_>) -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (_, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let path = self
                .workspace_timeline_root_for_session(session_id)?
                .join("artifact_versions.json");
            let lock =
                crate::product::coding_attempt_store::locking::ExclusiveFileLock::acquire(&path)?;
            let raw = Self::read_artifact_versions_at(&path)?;
            let mut ctx = ArtifactVersionsWrite {
                path: &path,
                _lock: lock,
                raw,
            };
            operation(&mut ctx)
        })
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
            // session 附属目录（checkpoints 等）与 flock lock 文件必须一并清理：
            // 残留 checkpoint 会被复用同 id 的新会话读到（v37 story spec 删除后
            // 重新生成仍呈旧状态）。lock 清理先例见 delete_issue_shared_worktree。
            remove_dir_all_if_exists(&sessions_root.join(&session.id))?;
            remove_file_if_exists(&sessions_root.join(format!(".{}.json.lock", session.id)))?;
            remove_file_if_exists(&sessions_root.join(format!("{}.json", session.id)))?;
        }
        Ok(())
    }
}

/// `artifact_versions.json` 的写事务上下文：仅由 `LifecycleStore::with_artifact_versions_write`
/// 构造，持有该文件的 sidecar 排它锁（`.artifact_versions.json.lock`，由现有
/// `ExclusiveFileLock` 原语在此处新建；此前不存在针对该文件的锁）直到 drop。
pub(crate) struct ArtifactVersionsWrite<'a> {
    path: &'a std::path::Path,
    _lock: crate::product::coding_attempt_store::locking::ExclusiveFileLock,
    raw: Vec<ArtifactVersion>,
}

impl ArtifactVersionsWrite<'_> {
    /// 进入事务时的磁盘真值（成功 `save` 后为最新落盘表），不做任何过滤。
    pub(crate) fn raw(&self) -> &[ArtifactVersion] {
        &self.raw
    }

    /// `max(raw.version)+1`（不是 `len()+1`）；溢出返回类型化错误，不回绕。
    pub(crate) fn next_version(&self) -> Result<u32, ProductStoreError> {
        match self.raw.iter().map(|entry| entry.version).max() {
            None => Ok(1),
            Some(max) => max
                .checked_add(1)
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "artifact_versions",
                    reason: "version_space_exhausted".to_string(),
                }),
        }
    }

    /// 以 `proposed` 为权威整表落盘，但 A4（SC 失败修复包）走集中协调：
    /// 磁盘已有而 `proposed` 缺席的 A4 逐字保留；同 version A4 必须逐字相等
    /// （异 payload/metadata 拒绝）；A4 恒 `is_current=false`；同 `package_id`
    /// 不得绑定两个不同 digest。普通条目仍以 `proposed` 为权威（删除/回滚
    /// 不复活）。version 重复在任何写之前拒绝。
    /// 禁止转调公共 `save_artifact_versions`：同一 sidecar 的 flock 按 open file
    /// description 计，同线程二次获取会永久阻塞。
    pub(crate) fn save(
        &mut self,
        proposed: &[ArtifactVersion],
    ) -> Result<Vec<ArtifactVersion>, ProductStoreError> {
        let mut seen = std::collections::HashSet::with_capacity(proposed.len());
        if !proposed.iter().all(|entry| seen.insert(entry.version)) {
            return Err(ProductStoreError::InvalidRecord {
                kind: "artifact_versions",
                reason: "duplicate_version".to_string(),
            });
        }
        let merged = self.merge_preserving_a4(proposed)?;
        write_json(self.path, &merged)?;
        self.raw = merged.clone();
        Ok(merged)
    }

    /// A4-only 集中协调（Plan §2.5 P14）。
    fn merge_preserving_a4(
        &self,
        proposed: &[ArtifactVersion],
    ) -> Result<Vec<ArtifactVersion>, ProductStoreError> {
        for entry in proposed {
            if is_a4_payload(&entry.payload) && entry.is_current {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "artifact_versions",
                    reason: "a4_current_forbidden".to_string(),
                });
            }
        }
        let mut merged = proposed.to_vec();
        for disk in &self.raw {
            if !is_a4_payload(&disk.payload) {
                continue;
            }
            match merged
                .iter()
                .position(|entry| entry.version == disk.version)
            {
                Some(index) => {
                    if merged[index] != *disk {
                        return Err(ProductStoreError::InvalidRecord {
                            kind: "artifact_versions",
                            reason: "a4_immutable_violation".to_string(),
                        });
                    }
                }
                None => merged.push(disk.clone()),
            }
        }
        let mut digest_by_package: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for entry in &merged {
            let crate::web::workspace_ws_types::ArtifactPayload::ScFailureRepairPackage {
                sc_failure_repair_package,
            } = &entry.payload
            else {
                continue;
            };
            let digest =
                crate::product::lifecycle_store::sc_failure_publication::sc_failure_package_digest(
                    sc_failure_repair_package,
                )
                .map_err(|error| ProductStoreError::InvalidRecord {
                    kind: "artifact_versions",
                    reason: format!("a4_package_digest_failed: {}", error.detail),
                })?;
            match digest_by_package.get(sc_failure_repair_package.package_id.as_str()) {
                Some(existing) if existing != &digest => {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "artifact_versions",
                        reason: "a4_package_digest_conflict".to_string(),
                    });
                }
                Some(_) => {}
                None => {
                    digest_by_package.insert(sc_failure_repair_package.package_id.clone(), digest);
                }
            }
        }
        Ok(merged)
    }

    /// 当前磁盘文件的逐字节内容；文件缺失与 `list_artifact_versions` 同口径返回空。
    pub(crate) fn read_bytes(&self) -> Result<Vec<u8>, ProductStoreError> {
        if !path_exists(self.path)? {
            return Ok(Vec::new());
        }
        std::fs::read(self.path).map_err(|error| {
            ProductStoreError::Io(format!("read {}: {error}", self.path.display()))
        })
    }
}

fn is_a4_payload(payload: &crate::web::workspace_ws_types::ArtifactPayload) -> bool {
    matches!(
        payload,
        crate::web::workspace_ws_types::ArtifactPayload::ScFailureRepairPackage { .. }
    )
}

#[cfg(test)]
#[path = "tests/sc_artifact_version_write.rs"]
mod sc_artifact_version_write;
