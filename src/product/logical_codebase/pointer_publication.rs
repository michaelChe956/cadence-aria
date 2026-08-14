//! Pointer publication store + pointer content template.
//!
//! 记录落盘于
//! `.aria/projects/{project}/logical-codebase/pointer-publications/{publication_id}.json`
//! （单文件原子写 + 单写者状态推进）。本模块还提供指针块渲染/幂等匹配/冲突检测纯函数。

use std::io::ErrorKind;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

const POINTER_PUBLICATION_KIND: &str = "pointer_publication";
const POINTER_PUBLICATION_ENTRY_KIND: &str = "pointer_publication_entry";

/// 一次「指针发布操作」的聚合记录。发布进度的唯一事实来源，与 coding attempt
/// 生命周期完全解耦。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PointerPublication {
    pub id: String,
    pub project_id: String,
    pub logical_codebase_id: String,
    pub batch_kind: PointerPublicationBatchKind,
    pub entries: Vec<PointerPublicationEntry>,
    pub status: PointerPublicationStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerPublicationBatchKind {
    Full,
    Incremental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerPublicationStatus {
    InProgress,
    CompletedAll,
    CompletedPartial,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PointerPublicationEntry {
    pub member_repo_id: String,
    pub state: PointerPublicationEntryState,
    pub branch_name: Option<String>,
    pub commit_sha: Option<String>,
    pub push_error: Option<String>,
    pub conflict_detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerPublicationEntryState {
    Pending,
    Skipped,
    Conflict,
    Committed,
    Pushed,
    ReviewCreated,
    Failed,
    Revoked,
}

/// 指针块渲染输入。字段即指针块正文内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerBlockFields {
    pub logical_codebase_id: String,
    pub repo_id: String,
    pub canonical_policy_locator: String,
    pub pointer_version: u32,
}

/// 已有文件与期望指针块的三态合并判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerMergeVerdict {
    /// 已有文件无指针块，应末尾追加。
    Append,
    /// 已有指针块规范化后与期望一致，幂等跳过。
    Skip,
    /// 已有指针块与期望不一致，降级人工，携带差异摘要。
    Conflict { summary: String },
}

#[derive(Debug, Clone)]
pub struct PointerPublicationStore {
    paths: ProductAppPaths,
}

impl PointerPublicationStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// 创建发布批次（含发布锁：同 logical_codebase 已有 InProgress 批次即拒绝，
    /// 单写者语义，非乐观锁 CAS）。同一 id 幂等重发返回既有记录。
    pub fn create_publication(
        &self,
        publication: PointerPublication,
    ) -> Result<PointerPublication, ProductStoreError> {
        validate_initial_publication(&publication)?;
        let path = self.publication_path(&publication.project_id, &publication.id)?;
        if path.exists() {
            let existing: PointerPublication = read_json(&path)?;
            ensure_publication_identity(&existing, &publication.project_id, &publication.id)?;
            if existing == publication {
                return Ok(existing);
            }
            return Err(conflict(&publication.id));
        }

        if self
            .find_in_progress_publication(
                &publication.project_id,
                &publication.logical_codebase_id,
            )?
            .is_some()
        {
            return Err(conflict(&format!(
                "logical_codebase:{}",
                publication.logical_codebase_id
            )));
        }

        write_json(&path, &publication)?;
        Ok(publication)
    }

    pub fn load_publication(
        &self,
        project_id: &str,
        publication_id: &str,
    ) -> Result<PointerPublication, ProductStoreError> {
        let path = self.publication_path(project_id, publication_id)?;
        if !path.exists() {
            return Err(not_found(publication_id));
        }
        let publication: PointerPublication = read_json(&path)?;
        ensure_publication_identity(&publication, project_id, publication_id)?;
        validate_record_shape(&publication)?;
        Ok(publication)
    }

    /// 单文件原子写（先写临时文件再 rename，见 `write_json`）。
    pub fn save_publication(
        &self,
        publication: &PointerPublication,
    ) -> Result<(), ProductStoreError> {
        validate_record_shape(publication)?;
        write_json(
            &self.publication_path(&publication.project_id, &publication.id)?,
            publication,
        )
    }

    /// 列出某 project 的全部发布批次（按 created_at 升序）。只读顶层
    /// `pointer-publications/{id}.json`，不触碰 `{id}/` 子目录（git-operations /
    /// review-requests 分区）。
    pub fn list_publications(
        &self,
        project_id: &str,
    ) -> Result<Vec<PointerPublication>, ProductStoreError> {
        let root = self.publications_root(project_id)?;
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };

        let mut publications = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            if !entry
                .file_type()
                .map_err(|error| {
                    ProductStoreError::Io(format!("stat {}: {error}", path.display()))
                })?
                .is_file()
            {
                continue;
            }
            let publication: PointerPublication = read_json(&path)?;
            ensure_publication_identity(&publication, project_id, &publication.id)?;
            validate_record_shape(&publication)?;
            publications.push(publication);
        }
        publications.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        Ok(publications)
    }

    /// 单写者推进条目状态。校验 publication 归属（条目必须属于该批次）、合法状态
    /// 转换（禁止回退）以及批次仍处于 InProgress。
    pub fn advance_entry_state(
        &self,
        project_id: &str,
        publication_id: &str,
        member_repo_id: &str,
        target: PointerPublicationEntryState,
    ) -> Result<PointerPublication, ProductStoreError> {
        self.update(project_id, publication_id, |publication| {
            if publication.status != PointerPublicationStatus::InProgress {
                return Err(invalid_record(format!(
                    "publication {publication_id} is not InProgress: {:?}",
                    publication.status
                )));
            }
            let entry = publication
                .entries
                .iter_mut()
                .find(|entry| entry.member_repo_id == member_repo_id)
                .ok_or_else(|| entry_not_found(member_repo_id))?;
            if !valid_entry_transition(&entry.state, &target) {
                return Err(invalid_record(format!(
                    "invalid entry state transition for {member_repo_id}: {:?} -> {target:?}",
                    entry.state
                )));
            }
            entry.state = target;
            publication.updated_at = Utc::now().to_rfc3339();
            Ok(())
        })
    }

    /// 撤回发布：批次置 `Revoked`、全部条目置 `Revoked`。重复 revoke 幂等返回当前态。
    /// 远端分支删除与 ReviewRequest 标记属 T10 编排职责，本方法只推进记录状态。
    pub fn mark_revoked(
        &self,
        project_id: &str,
        publication_id: &str,
    ) -> Result<PointerPublication, ProductStoreError> {
        self.update(project_id, publication_id, |publication| {
            if publication.status == PointerPublicationStatus::Revoked {
                return Ok(());
            }
            publication.status = PointerPublicationStatus::Revoked;
            for entry in &mut publication.entries {
                entry.state = PointerPublicationEntryState::Revoked;
            }
            publication.updated_at = Utc::now().to_rfc3339();
            Ok(())
        })
    }

    /// 单写者记录条目结局（推进状态 + 写详情，单次原子写）。校验 publication 归属、
    /// 批次仍 InProgress、合法状态转换（同 `advance_entry_state`）。
    /// 由 `PointerPublishCoordinator` 每仓流水结束时调用，避免状态与详情两步写产生
    /// 部分窗口。
    #[allow(clippy::too_many_arguments)]
    pub fn record_entry_outcome(
        &self,
        project_id: &str,
        publication_id: &str,
        member_repo_id: &str,
        target: PointerPublicationEntryState,
        branch_name: Option<String>,
        commit_sha: Option<String>,
        push_error: Option<String>,
        conflict_detail: Option<String>,
    ) -> Result<PointerPublication, ProductStoreError> {
        self.update(project_id, publication_id, |publication| {
            if publication.status != PointerPublicationStatus::InProgress {
                return Err(invalid_record(format!(
                    "publication {publication_id} is not InProgress: {:?}",
                    publication.status
                )));
            }
            let entry = publication
                .entries
                .iter_mut()
                .find(|entry| entry.member_repo_id == member_repo_id)
                .ok_or_else(|| entry_not_found(member_repo_id))?;
            if !valid_entry_transition(&entry.state, &target) {
                return Err(invalid_record(format!(
                    "invalid entry state transition for {member_repo_id}: {:?} -> {target:?}",
                    entry.state
                )));
            }
            entry.state = target;
            entry.branch_name = branch_name;
            entry.commit_sha = commit_sha;
            entry.push_error = push_error;
            entry.conflict_detail = conflict_detail;
            publication.updated_at = Utc::now().to_rfc3339();
            Ok(())
        })
    }

    /// 崩溃恢复专用：把条目复位到 `Pending`（清空详情，供全量重跑）或推进到
    /// `Pushed`（远端分支已存在，供补写 ReviewRequest）。与 `advance_entry_state`
    /// 不同，本方法允许跨恢复的状态跳跃（如 `Committed→Pending`、`Failed→Pushed`），
    /// 但要求批次仍 InProgress 且条目处于可重试的非终态集合。
    pub fn reset_entry_for_retry(
        &self,
        project_id: &str,
        publication_id: &str,
        member_repo_id: &str,
        target: PointerPublicationEntryState,
    ) -> Result<PointerPublication, ProductStoreError> {
        assert!(
            matches!(
                target,
                PointerPublicationEntryState::Pending | PointerPublicationEntryState::Pushed
            ),
            "reset_entry_for_retry only supports Pending or Pushed"
        );
        self.update(project_id, publication_id, |publication| {
            if publication.status != PointerPublicationStatus::InProgress {
                return Err(invalid_record(format!(
                    "publication {publication_id} is not InProgress: {:?}",
                    publication.status
                )));
            }
            let entry = publication
                .entries
                .iter_mut()
                .find(|entry| entry.member_repo_id == member_repo_id)
                .ok_or_else(|| entry_not_found(member_repo_id))?;
            match entry.state {
                PointerPublicationEntryState::Pending
                | PointerPublicationEntryState::Committed
                | PointerPublicationEntryState::Pushed
                | PointerPublicationEntryState::Failed
                | PointerPublicationEntryState::Conflict => {}
                other => {
                    return Err(invalid_record(format!(
                        "entry {member_repo_id} is {other:?} and cannot be reset for retry"
                    )));
                }
            }
            entry.state = target;
            if target == PointerPublicationEntryState::Pending {
                entry.branch_name = None;
                entry.commit_sha = None;
                entry.push_error = None;
                entry.conflict_detail = None;
            }
            publication.updated_at = Utc::now().to_rfc3339();
            Ok(())
        })
    }

    fn publications_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(self
            .paths
            .logical_codebase_root(project_id)
            .join("pointer-publications"))
    }

    fn publication_path(
        &self,
        project_id: &str,
        publication_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(publication_id)?;
        Ok(self
            .publications_root(project_id)?
            .join(format!("{publication_id}.json")))
    }

    fn find_in_progress_publication(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<Option<PointerPublication>, ProductStoreError> {
        let root = self.publications_root(project_id)?;
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };

        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let publication: PointerPublication = read_json(&path)?;
            if publication.logical_codebase_id == logical_codebase_id
                && publication.status == PointerPublicationStatus::InProgress
            {
                return Ok(Some(publication));
            }
        }
        Ok(None)
    }

    fn update(
        &self,
        project_id: &str,
        publication_id: &str,
        update: impl FnOnce(&mut PointerPublication) -> Result<(), ProductStoreError>,
    ) -> Result<PointerPublication, ProductStoreError> {
        let path = self.publication_path(project_id, publication_id)?;
        if !path.exists() {
            return Err(not_found(publication_id));
        }
        let mut publication: PointerPublication = read_json(&path)?;
        ensure_publication_identity(&publication, project_id, publication_id)?;
        validate_record_shape(&publication)?;
        update(&mut publication)?;
        validate_record_shape(&publication)?;
        write_json(&path, &publication)?;
        Ok(publication)
    }
}

fn validate_initial_publication(publication: &PointerPublication) -> Result<(), ProductStoreError> {
    validate_record_shape(publication)?;
    if publication.status != PointerPublicationStatus::InProgress {
        return Err(invalid_record(format!(
            "create_publication requires InProgress status, got {:?}",
            publication.status
        )));
    }
    if publication.entries.iter().any(|entry| {
        entry.state != PointerPublicationEntryState::Pending
            || entry.branch_name.is_some()
            || entry.commit_sha.is_some()
            || entry.push_error.is_some()
            || entry.conflict_detail.is_some()
    }) {
        return Err(invalid_record(
            "create_publication requires every entry to be Pending with no details",
        ));
    }
    Ok(())
}

fn validate_record_shape(publication: &PointerPublication) -> Result<(), ProductStoreError> {
    validate_relative_id(&publication.id)?;
    validate_relative_id(&publication.project_id)?;
    validate_relative_id(&publication.logical_codebase_id)?;
    if publication.entries.is_empty() {
        return Err(invalid_record("publication entries must not be empty"));
    }
    let mut seen = std::collections::HashSet::new();
    for entry in &publication.entries {
        validate_relative_id(&entry.member_repo_id)?;
        if !seen.insert(entry.member_repo_id.as_str()) {
            return Err(invalid_record(format!(
                "duplicate member_repo_id: {}",
                entry.member_repo_id
            )));
        }
    }
    Ok(())
}

fn ensure_publication_identity(
    publication: &PointerPublication,
    project_id: &str,
    publication_id: &str,
) -> Result<(), ProductStoreError> {
    if publication.project_id != project_id || publication.id != publication_id {
        return Err(identity_mismatch(publication_id));
    }
    Ok(())
}

fn valid_entry_transition(
    from: &PointerPublicationEntryState,
    to: &PointerPublicationEntryState,
) -> bool {
    matches!(
        (from, to),
        (
            PointerPublicationEntryState::Pending,
            PointerPublicationEntryState::Skipped
        ) | (
            PointerPublicationEntryState::Pending,
            PointerPublicationEntryState::Conflict
        ) | (
            PointerPublicationEntryState::Pending,
            PointerPublicationEntryState::Committed
        ) | (
            PointerPublicationEntryState::Pending,
            PointerPublicationEntryState::Failed
        ) | (
            PointerPublicationEntryState::Conflict,
            PointerPublicationEntryState::Pending
        ) | (
            PointerPublicationEntryState::Committed,
            PointerPublicationEntryState::Pushed
        ) | (
            PointerPublicationEntryState::Committed,
            PointerPublicationEntryState::Failed
        ) | (
            PointerPublicationEntryState::Pushed,
            PointerPublicationEntryState::ReviewCreated
        ) | (
            PointerPublicationEntryState::Pushed,
            PointerPublicationEntryState::Failed
        ) | (
            PointerPublicationEntryState::Failed,
            PointerPublicationEntryState::Pending
        )
    )
}

fn not_found(id: &str) -> ProductStoreError {
    ProductStoreError::NotFound {
        kind: POINTER_PUBLICATION_KIND,
        id: id.to_string(),
    }
}

fn entry_not_found(id: &str) -> ProductStoreError {
    ProductStoreError::NotFound {
        kind: POINTER_PUBLICATION_ENTRY_KIND,
        id: id.to_string(),
    }
}

fn conflict(id: &str) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: POINTER_PUBLICATION_KIND,
        id: id.to_string(),
    }
}

fn identity_mismatch(id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: POINTER_PUBLICATION_KIND,
        id: id.to_string(),
    }
}

fn invalid_record(reason: impl Into<String>) -> ProductStoreError {
    ProductStoreError::InvalidRecord {
        kind: POINTER_PUBLICATION_KIND,
        reason: reason.into(),
    }
}

const POINTER_BLOCK_START: &str = "aria-logical-codebase-pointer:start";
const POINTER_BLOCK_END: &str = "aria-logical-codebase-pointer:end";

/// 渲染指针标记块（含末尾换行）。纯函数，字节稳定。
pub fn render_pointer_block(fields: &PointerBlockFields) -> String {
    format!(
        "<!-- {POINTER_BLOCK_START}\n  logical_codebase_id: {}\n  repo_id: {}\n  canonical_policy_locator: {}\n  声明：未加载集中政策前禁止写；本块仅用于发现，不作为政策正文\n  pointer_version: {}\n{POINTER_BLOCK_END} -->\n",
        fields.logical_codebase_id,
        fields.repo_id,
        fields.canonical_policy_locator,
        fields.pointer_version
    )
}

/// 已有文件与期望指针块的三态合并判定：无标记块 → `Append`；标记块规范化内容
/// 一致（忽略行尾空白差异）→ `Skip`；不一致 → `Conflict`（携带差异摘要）。
pub fn classify_merge(existing_file: &str, expected_block: &str) -> PointerMergeVerdict {
    let Some(existing_block) = extract_pointer_block(existing_file) else {
        return PointerMergeVerdict::Append;
    };
    if normalize_block(existing_block) == normalize_block(expected_block) {
        PointerMergeVerdict::Skip
    } else {
        PointerMergeVerdict::Conflict {
            summary: pointer_diff_summary(expected_block, existing_block),
        }
    }
}

/// 在既有文件末尾追加指针块，原内容零改动。
pub fn apply_append(existing_file: &str, block: &str) -> String {
    let mut out = existing_file.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    out
}

/// 从既有文件提取指针块（起止标记所在行之间，含标记行）。无起始标记返回 `None`；
/// 有起始标记但缺结束标记时取到文件末尾，使比较落入 `Conflict`（不吞掉残缺块）。
fn extract_pointer_block(content: &str) -> Option<&str> {
    let start = content.find(POINTER_BLOCK_START)?;
    let block_start = content[..start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let end = match content[start..].find(POINTER_BLOCK_END) {
        Some(offset) => start + offset,
        None => content.len(),
    };
    let end_line_end = content[end..]
        .find('\n')
        .map(|offset| end + offset + 1)
        .unwrap_or(content.len());
    Some(&content[block_start..end_line_end])
}

fn normalize_block(block: &str) -> Vec<String> {
    block
        .lines()
        .map(str::trim_end)
        .map(str::to_string)
        .collect()
}

fn pointer_diff_summary(expected: &str, existing: &str) -> String {
    let expected_lines = normalize_block(expected);
    let existing_lines = normalize_block(existing);
    let mut parts = vec![format!(
        "pointer block differs: expected {} line(s), existing {} line(s)",
        expected_lines.len(),
        existing_lines.len()
    )];
    if let Some((index, (expected_line, existing_line))) = expected_lines
        .iter()
        .zip(&existing_lines)
        .enumerate()
        .find(|(_, (expected_line, existing_line))| expected_line != existing_line)
    {
        parts.push(format!(
            "first differing line {}: expected {expected_line:?}, existing {existing_line:?}",
            index + 1
        ));
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::json_store::ProductStoreError;
    use uuid::Uuid;

    const CREATED_AT: &str = "2026-08-14T00:00:00Z";

    fn block_fields() -> PointerBlockFields {
        PointerBlockFields {
            logical_codebase_id: "lc_1".into(),
            repo_id: "repo_1".into(),
            canonical_policy_locator: "/data/aria/aggregate/policy".into(),
            pointer_version: 1,
        }
    }

    fn block_fixture() -> String {
        render_pointer_block(&block_fields())
    }

    fn publication_fixture(
        project_id: &str,
        logical_codebase_id: &str,
        repo_ids: &[&str],
    ) -> PointerPublication {
        PointerPublication {
            id: Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            logical_codebase_id: logical_codebase_id.to_string(),
            batch_kind: PointerPublicationBatchKind::Full,
            entries: repo_ids
                .iter()
                .map(|repo_id| PointerPublicationEntry {
                    member_repo_id: repo_id.to_string(),
                    state: PointerPublicationEntryState::Pending,
                    branch_name: None,
                    commit_sha: None,
                    push_error: None,
                    conflict_detail: None,
                })
                .collect(),
            status: PointerPublicationStatus::InProgress,
            created_at: CREATED_AT.to_string(),
            updated_at: CREATED_AT.to_string(),
        }
    }

    #[test]
    fn create_publication_rejects_concurrent_in_progress_batch() {
        let temp = tempfile::tempdir().unwrap();
        let store = PointerPublicationStore::new(ProductAppPaths::new(temp.path()));
        let first = publication_fixture("project_0001", "lc_0001", &["repo_a"]);
        let second = publication_fixture("project_0001", "lc_0001", &["repo_b"]);

        store.create_publication(first).unwrap();
        let err = store.create_publication(second).unwrap_err();
        assert!(matches!(err, ProductStoreError::Conflict { .. }));
    }

    #[test]
    fn advance_entry_enforces_valid_transitions() {
        let temp = tempfile::tempdir().unwrap();
        let store = PointerPublicationStore::new(ProductAppPaths::new(temp.path()));
        let publication = publication_fixture("project_0001", "lc_0001", &["repo_a"]);
        let created = store.create_publication(publication).unwrap();

        // Pending → Committed 合法
        let advanced = store
            .advance_entry_state(
                "project_0001",
                &created.id,
                "repo_a",
                PointerPublicationEntryState::Committed,
            )
            .unwrap();
        assert_eq!(
            advanced.entries[0].state,
            PointerPublicationEntryState::Committed
        );

        // Committed → Pushed 合法（为 Pushed→Pending 拒绝做铺垫）
        store
            .advance_entry_state(
                "project_0001",
                &created.id,
                "repo_a",
                PointerPublicationEntryState::Pushed,
            )
            .unwrap();

        // Pushed → Pending 拒绝
        let err = store
            .advance_entry_state(
                "project_0001",
                &created.id,
                "repo_a",
                PointerPublicationEntryState::Pending,
            )
            .unwrap_err();
        assert!(matches!(err, ProductStoreError::InvalidRecord { .. }));
    }

    #[test]
    fn save_publication_is_single_writer_atomic() {
        let temp = tempfile::tempdir().unwrap();
        let store = PointerPublicationStore::new(ProductAppPaths::new(temp.path()));
        let publication = publication_fixture("project_0001", "lc_0001", &["repo_a"]);

        store.save_publication(&publication).unwrap();
        let loaded = store
            .load_publication("project_0001", &publication.id)
            .unwrap();
        assert_eq!(loaded, publication);

        // 坏 JSON 文件 → Err 不 panic
        let bad_path = temp
            .path()
            .join("projects/project_0001/logical-codebase/pointer-publications")
            .join(format!("{}.json", publication.id));
        std::fs::write(&bad_path, b"{ not valid json").unwrap();
        let err = store
            .load_publication("project_0001", &publication.id)
            .unwrap_err();
        assert!(matches!(err, ProductStoreError::Json(_)));
    }

    #[test]
    fn mark_revoked_marks_publication_and_entries_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let store = PointerPublicationStore::new(ProductAppPaths::new(temp.path()));
        let publication = publication_fixture("project_0001", "lc_0001", &["repo_a", "repo_b"]);
        let created = store.create_publication(publication).unwrap();

        let revoked = store.mark_revoked("project_0001", &created.id).unwrap();
        assert_eq!(revoked.status, PointerPublicationStatus::Revoked);
        assert!(
            revoked
                .entries
                .iter()
                .all(|entry| entry.state == PointerPublicationEntryState::Revoked)
        );

        // 重复 revoke 幂等返回当前态
        let again = store.mark_revoked("project_0001", &created.id).unwrap();
        assert_eq!(again.status, PointerPublicationStatus::Revoked);
    }

    #[test]
    fn render_pointer_block_contains_all_fields() {
        let block = render_pointer_block(&PointerBlockFields {
            logical_codebase_id: "lc_1".into(),
            repo_id: "repo_1".into(),
            canonical_policy_locator: "/data/aria/aggregate/policy".into(),
            pointer_version: 1,
        });
        for needle in [
            "aria-logical-codebase-pointer:start",
            "lc_1",
            "repo_1",
            "/data/aria/aggregate/policy",
            "未加载集中政策前禁止写",
            "pointer_version: 1",
            "aria-logical-codebase-pointer:end",
        ] {
            assert!(block.contains(needle), "missing {needle}: {block}");
        }
    }

    #[test]
    fn classify_merge_three_way() {
        let existing_without = "# 既有规则\n保留内容\n";

        // 无标记块 → Append
        assert_eq!(
            classify_merge(existing_without, &block_fixture()),
            PointerMergeVerdict::Append
        );

        // 标记块内容一致（规范化比较：既有块带行尾空格）→ Skip
        let mut existing_with = existing_without.to_string();
        existing_with.push_str(&block_fixture());
        let normalized_existing =
            existing_with.replace("  pointer_version: 1\n", "  pointer_version: 1  \n");
        assert_eq!(
            classify_merge(&normalized_existing, &block_fixture()),
            PointerMergeVerdict::Skip
        );

        // 标记块不一致 → Conflict
        let mut conflicting = existing_without.to_string();
        conflicting.push_str(
            "<!-- aria-logical-codebase-pointer:start\n  logical_codebase_id: lc_other\n  repo_id: repo_1\n  canonical_policy_locator: /data/aria/aggregate/policy\n  声明：未加载集中政策前禁止写；本块仅用于发现，不作为政策正文\n  pointer_version: 1\naria-logical-codebase-pointer:end -->\n",
        );
        match classify_merge(&conflicting, &block_fixture()) {
            PointerMergeVerdict::Conflict { summary } => assert!(!summary.is_empty()),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn classify_merge_conflict_summary_describes_diff() {
        let different = render_pointer_block(&PointerBlockFields {
            logical_codebase_id: "lc_other".into(),
            repo_id: "repo_1".into(),
            canonical_policy_locator: "/data/aria/aggregate/policy".into(),
            pointer_version: 1,
        });
        let mut existing = "# 既有规则\n".to_string();
        existing.push_str(&different);
        match classify_merge(&existing, &block_fixture()) {
            PointerMergeVerdict::Conflict { summary } => {
                assert!(summary.contains("differs"), "summary: {summary}");
                assert!(summary.contains("line"), "summary: {summary}");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn apply_append_preserves_existing_content_bytes() {
        let existing = "# 既有规则\n保留内容\n";
        let out = apply_append(existing, &block_fixture());
        assert!(out.starts_with("# 既有规则\n保留内容\n"));
        assert!(out.ends_with("aria-logical-codebase-pointer:end -->\n"));
    }
}
