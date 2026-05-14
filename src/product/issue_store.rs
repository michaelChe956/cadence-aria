use std::fs;
use std::path::Path;

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, validate_relative_id, write_json};
use crate::product::models::{IssuePhase, IssueRecord, IssueStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProductIssueInput {
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

    pub fn create(&self, input: CreateProductIssueInput) -> Result<IssueRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.repo_id)?;
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

        write_json(
            &self
                .paths
                .issue_root(&issue.project_id, &id)
                .join("issue.json"),
            &issue,
        )?;
        Ok(issue)
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
