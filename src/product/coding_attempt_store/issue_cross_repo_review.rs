use std::fs;
use std::path::{Path, PathBuf};

use crate::product::coding_models::{IssueCrossRepoReview, IssueCrossRepoRiskAcknowledgement};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::locking::with_exclusive_lock;

/// issue 级跨仓最终评审工件根目录名（与 `coding-attempts/` 同级，属 issue 维度）。
const CROSS_REPO_REVIEWS_DIR: &str = "coding-cross-repo-reviews";
/// 评审本体文件名（同 issue 唯一）。
const CROSS_REPO_REVIEW_FILE: &str = "review.json";
/// 留痕目录名。
const CROSS_REPO_ACKNOWLEDGEMENTS_DIR: &str = "acknowledgements";

impl super::CodingAttemptStore {
    fn issue_cross_repo_reviews_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_root(project_id, issue_id)
            .join(CROSS_REPO_REVIEWS_DIR)
    }

    fn issue_cross_repo_acknowledgements_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.issue_cross_repo_reviews_root(project_id, issue_id)
            .join(CROSS_REPO_ACKNOWLEDGEMENTS_DIR)
    }

    /// 读取 issue 级跨仓最终评审记录；不存在时返回 `Ok(None)`。
    ///
    /// REQ-GFR-CR-03：评审记录**按 issue 唯一**——同一 issue 永远只有一份评审本体。
    pub fn load_issue_cross_repo_review(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<IssueCrossRepoReview>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let path = self
            .issue_cross_repo_reviews_root(project_id, issue_id)
            .join(CROSS_REPO_REVIEW_FILE);
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        Ok(Some(read_json::<IssueCrossRepoReview>(&path)?))
    }

    /// 写入 issue 级跨仓最终评审记录（**覆盖式**：同 issue 唯一）。
    pub fn save_issue_cross_repo_review(
        &self,
        review: &IssueCrossRepoReview,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&review.project_id)?;
        validate_relative_id(&review.issue_id)?;
        validate_relative_id(&review.id)?;
        let root = self.issue_cross_repo_reviews_root(&review.project_id, &review.issue_id);
        with_exclusive_lock(&root, || {
            write_json(&root.join(CROSS_REPO_REVIEW_FILE), review)
        })
    }

    /// 追加一条「已知风险，仍然确认」留痕（**追加式**）。
    ///
    /// REQ-GFR-CR-06：同一 `(cross_repo_review_id, attempt_id)` 至多留痕一条——
    /// 已存在时**不重复写**，返回 `Ok(false)`；真正写入时返回 `Ok(true)`。
    pub fn append_issue_cross_repo_acknowledgement(
        &self,
        acknowledgement: &IssueCrossRepoRiskAcknowledgement,
    ) -> Result<bool, ProductStoreError> {
        validate_relative_id(&acknowledgement.project_id)?;
        validate_relative_id(&acknowledgement.issue_id)?;
        validate_relative_id(&acknowledgement.id)?;
        validate_relative_id(&acknowledgement.attempt_id)?;
        validate_relative_id(&acknowledgement.cross_repo_review_id)?;
        let root = self
            .issue_cross_repo_reviews_root(&acknowledgement.project_id, &acknowledgement.issue_id);
        let acknowledgements_root = root.join(CROSS_REPO_ACKNOWLEDGEMENTS_DIR);
        let path = acknowledgements_root.join(format!("{}.json", acknowledgement.id));
        with_exclusive_lock(&root, || {
            if self
                .find_issue_cross_repo_acknowledgement(
                    &acknowledgements_root,
                    &acknowledgement.cross_repo_review_id,
                    &acknowledgement.attempt_id,
                )?
                .is_some()
            {
                return Ok(false);
            }
            write_json(&path, acknowledgement)?;
            Ok(true)
        })
    }

    /// 列出该 issue 的全部「已知风险，仍然确认」留痕（按确认时间、再按 id 稳定排序）。
    pub fn list_issue_cross_repo_acknowledgements(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<IssueCrossRepoRiskAcknowledgement>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self.issue_cross_repo_acknowledgements_root(project_id, issue_id);
        let mut records = Vec::new();
        for path in acknowledgement_files(&root)? {
            records.push(read_json::<IssueCrossRepoRiskAcknowledgement>(&path)?);
        }
        records.sort_by(|left, right| {
            left.acknowledged_at
                .cmp(&right.acknowledged_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(records)
    }

    /// 判定该 issue 是否已对同一评审记录、同一 attempt 留痕。
    pub fn has_issue_cross_repo_acknowledgement(
        &self,
        project_id: &str,
        issue_id: &str,
        cross_repo_review_id: &str,
        attempt_id: &str,
    ) -> Result<bool, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(cross_repo_review_id)?;
        validate_relative_id(attempt_id)?;
        let root = self.issue_cross_repo_acknowledgements_root(project_id, issue_id);
        Ok(self
            .find_issue_cross_repo_acknowledgement(&root, cross_repo_review_id, attempt_id)?
            .is_some())
    }

    fn find_issue_cross_repo_acknowledgement(
        &self,
        acknowledgements_root: &Path,
        cross_repo_review_id: &str,
        attempt_id: &str,
    ) -> Result<Option<IssueCrossRepoRiskAcknowledgement>, ProductStoreError> {
        for path in acknowledgement_files(acknowledgements_root)? {
            let record = read_json::<IssueCrossRepoRiskAcknowledgement>(&path)?;
            if record.cross_repo_review_id == cross_repo_review_id
                && record.attempt_id == attempt_id
            {
                return Ok(Some(record));
            }
        }
        Ok(None)
    }
}

/// 列出留痕目录下的全部 `*.json` 文件（稳定排序）。
fn acknowledgement_files(root: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    let mut paths = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(paths),
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
        if !super::path_is_regular_file(&path)? {
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    Ok(paths)
}
