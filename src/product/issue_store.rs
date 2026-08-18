use std::fs;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueDescriptionRevision {
    pub version: u32,
    pub description: String,
    pub revised_by: String,
    pub created_at: String,
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

    pub fn update_description(
        &self,
        project_id: &str,
        issue_id: &str,
        description: String,
        revised_by: &str,
    ) -> Result<IssueRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let mut issue = self.get(project_id, issue_id)?;
        let revisions_path = self.description_revisions_path(project_id, issue_id);
        let mut revisions: Vec<IssueDescriptionRevision> = if revisions_path.exists() {
            read_json(&revisions_path)?
        } else {
            Vec::new()
        };
        let now = Utc::now().to_rfc3339();
        let version = revisions.last().map_or(1, |revision| revision.version + 1);
        revisions.push(IssueDescriptionRevision {
            version,
            description: issue.description.clone().unwrap_or_default(),
            revised_by: revised_by.to_string(),
            created_at: now.clone(),
        });
        write_json(&revisions_path, &revisions)?;

        issue.description = Some(description);
        issue.updated_at = now;
        write_json(&self.issue_path(project_id, issue_id), &issue)?;
        Ok(issue)
    }

    pub fn list_description_revisions(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<IssueDescriptionRevision>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let revisions_path = self.description_revisions_path(project_id, issue_id);
        if !revisions_path.exists() {
            return Ok(Vec::new());
        }
        read_json(&revisions_path)
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

    fn description_revisions_path(&self, project_id: &str, issue_id: &str) -> std::path::PathBuf {
        self.paths
            .issue_root(project_id, issue_id)
            .join("description_revisions.json")
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
    fn update_description_persists_and_appends_revision() {
        let (_tmp, store) = setup_store();
        let issue = seed_issue(&store);
        let updated = store
            .update_description(PROJECT_ID, &issue.id, "细化后的描述".into(), "user")
            .unwrap();
        assert_eq!(updated.description.as_deref(), Some("细化后的描述"));
        let revs = store
            .list_description_revisions(PROJECT_ID, &issue.id)
            .unwrap();
        assert_eq!(revs.len(), 1);
        assert_eq!(
            revs[0].description,
            issue.description.clone().unwrap_or_default()
        );
    }

    #[test]
    fn update_description_appends_revisions_with_incrementing_version() {
        let (_tmp, store) = setup_store();
        let issue = seed_issue(&store);
        store
            .update_description(PROJECT_ID, &issue.id, "第一版描述".into(), "user")
            .unwrap();
        let updated = store
            .update_description(PROJECT_ID, &issue.id, "第二版描述".into(), "assistant")
            .unwrap();
        assert_eq!(updated.description.as_deref(), Some("第二版描述"));

        // 写后 get 读到新 description（验证真实落盘而非内存返回）。
        let read = store.get(PROJECT_ID, &issue.id).unwrap();
        assert_eq!(read.description.as_deref(), Some("第二版描述"));

        let revs = store
            .list_description_revisions(PROJECT_ID, &issue.id)
            .unwrap();
        assert_eq!(revs.len(), 2);
        assert_eq!(revs[0].version, 1);
        assert_eq!(revs[0].description, "");
        assert_eq!(revs[0].revised_by, "user");
        assert_eq!(revs[1].version, 2);
        assert_eq!(revs[1].description, "第一版描述");
        assert_eq!(revs[1].revised_by, "assistant");
    }

    #[test]
    fn update_description_missing_issue_returns_not_found() {
        let (_tmp, store) = setup_store();
        let result = store.update_description(PROJECT_ID, "issue_9999", "描述".into(), "user");
        assert!(matches!(
            result,
            Err(ProductStoreError::NotFound { kind: "issue", .. })
        ));
    }
}
