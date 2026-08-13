use std::fs;
use std::path::Path;

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{IssuePhase, IssueRecord, IssueStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProductIssueInput {
    pub project_id: String,
    pub repo_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProductIssueWithRepositoryInput {
    pub project_id: String,
    pub repo_id: String,
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IssueStore {
    paths: ProductAppPaths,
}

impl IssueStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn list(&self, project_id: &str) -> Result<Vec<IssueRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        let issues_root = self.paths.project_root(project_id).join("issues");
        if !issues_root.exists() {
            return Ok(Vec::new());
        }

        let mut issue_files = Vec::new();
        for entry in fs::read_dir(&issues_root).map_err(|error| {
            ProductStoreError::Io(format!("read {}: {error}", issues_root.display()))
        })? {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", issues_root.display()))
            })?;
            let issue_path = entry.path().join("issue.json");
            if issue_path.exists() {
                issue_files.push(issue_path);
            }
        }
        issue_files.sort();

        let mut issues = Vec::with_capacity(issue_files.len());
        for issue_file in issue_files {
            issues.push(read_json(&issue_file)?);
        }
        Ok(issues)
    }

    pub fn get(&self, project_id: &str, issue_id: &str) -> Result<IssueRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let issue_path = self.issue_path(project_id, issue_id);
        if !issue_path.exists() {
            return Err(ProductStoreError::NotFound {
                kind: "issue",
                id: issue_id.to_string(),
            });
        }
        read_json(&issue_path)
    }

    /// 更新 issue 状态并落盘。
    ///
    /// 幂等：若当前状态已是目标状态，直接返回内存记录而不重复写盘
    /// （避免 completed 接线重复触发时反复改写 `updated_at`）。
    pub fn update_status(
        &self,
        project_id: &str,
        issue_id: &str,
        status: IssueStatus,
    ) -> Result<IssueRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let mut issue = self.get(project_id, issue_id)?;
        if issue.status == status {
            return Ok(issue);
        }
        issue.status = status;
        issue.updated_at = Utc::now().to_rfc3339();
        write_json(&self.issue_path(project_id, issue_id), &issue)?;
        Ok(issue)
    }

    pub fn create(&self, input: CreateProductIssueInput) -> Result<IssueRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        if let Some(repo_id) = input.repo_id.as_deref() {
            validate_relative_id(repo_id)?;
        }
        let issues_root = self.paths.project_root(&input.project_id).join("issues");
        let existing_len = count_entries(&issues_root)?;
        let id = next_sequential_id("issue", existing_len);
        let now = Utc::now().to_rfc3339();
        let change_id = input.change_id.unwrap_or_else(|| {
            let slug = slugify(&input.title);
            if slug.is_empty() {
                format!("change_{id}")
            } else {
                slug
            }
        });
        let issue = IssueRecord {
            id: id.clone(),
            project_id: input.project_id,
            repo_id: input.repo_id,
            title: input.title,
            description: input.description,
            change_id,
            phase: IssuePhase::Clarification,
            status: IssueStatus::Draft,
            active_binding_id: None,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&self.issue_path(&issue.project_id, &id), &issue)?;
        Ok(issue)
    }

    pub fn create_with_repository(
        &self,
        input: CreateProductIssueWithRepositoryInput,
    ) -> Result<IssueRecord, ProductStoreError> {
        self.create(CreateProductIssueInput {
            project_id: input.project_id,
            repo_id: Some(input.repo_id),
            title: input.title,
            description: input.description,
            change_id: input.change_id,
        })
    }

    pub fn delete(&self, project_id: &str, issue_id: &str) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let issue_root = self.paths.issue_root(project_id, issue_id);
        if !issue_root.exists() {
            return Err(ProductStoreError::NotFound {
                kind: "issue",
                id: issue_id.to_string(),
            });
        }
        fs::remove_dir_all(&issue_root).map_err(|error| {
            ProductStoreError::Io(format!("remove {}: {error}", issue_root.display()))
        })
    }

    fn issue_path(&self, project_id: &str, issue_id: &str) -> std::path::PathBuf {
        self.paths
            .issue_root(project_id, issue_id)
            .join("issue.json")
    }
}

fn count_entries(path: &Path) -> Result<usize, ProductStoreError> {
    if !path.exists() {
        return Ok(0);
    }

    fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
        .try_fold(0usize, |count, entry| {
            entry.map(|_| count + 1).map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
            })
        })
}

fn slugify(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const PROJECT_ID: &str = "project_0001";

    fn setup_store() -> (TempDir, IssueStore) {
        let tmp = TempDir::new().unwrap();
        let store = IssueStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        (tmp, store)
    }

    fn seed_issue(store: &IssueStore) -> IssueRecord {
        store
            .create(CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some("repository_0001".to_string()),
                title: "issue".to_string(),
                description: None,
                change_id: None,
            })
            .unwrap()
    }

    #[test]
    fn update_status_persists_new_status_and_is_idempotent() {
        let (_tmp, store) = setup_store();
        let created = seed_issue(&store);
        assert_eq!(created.status, IssueStatus::Draft);

        let updated = store
            .update_status(PROJECT_ID, &created.id, IssueStatus::Completed)
            .unwrap();
        assert_eq!(updated.status, IssueStatus::Completed);

        // 写后 get 读到新 status（验证真实落盘而非内存返回）。
        let read = store.get(PROJECT_ID, &created.id).unwrap();
        assert_eq!(read.status, IssueStatus::Completed);

        // 幂等重复写：同 status 再写不报错，落盘 status 保持 Completed。
        let again = store
            .update_status(PROJECT_ID, &created.id, IssueStatus::Completed)
            .unwrap();
        assert_eq!(again.status, IssueStatus::Completed);
        let read_again = store.get(PROJECT_ID, &created.id).unwrap();
        assert_eq!(read_again.status, IssueStatus::Completed);
    }
}
