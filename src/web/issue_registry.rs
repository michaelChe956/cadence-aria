use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct IssueRegistryError {
    code: &'static str,
    message: String,
}

impl IssueRegistryError {
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Draft,
    Started,
    Running,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueRecord {
    pub issue_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: IssueStatus,
    pub workspace_id: Option<String>,
    pub task_id: Option<String>,
    pub session_id: Option<String>,
    pub change_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateIssueInput {
    pub title: String,
    pub description: Option<String>,
    pub change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskWorkspaceLink {
    pub issue_id: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone)]
pub struct IssueRegistry {
    app_root: PathBuf,
}

impl IssueRegistry {
    pub fn new(app_root: PathBuf) -> Self {
        Self { app_root }
    }

    pub fn list(&self) -> Result<Vec<IssueRecord>, IssueRegistryError> {
        let path = self.path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(path).map_err(io_error)?;
        serde_json::from_reader(file).map_err(json_error)
    }

    pub fn create(&self, input: CreateIssueInput) -> Result<IssueRecord, IssueRegistryError> {
        let title = input.title.trim().to_string();
        if title.is_empty() {
            return Err(registry_error("issue_title_required", "title is required"));
        }
        let mut issues = self.list()?;
        let now = Utc::now();
        let issue = IssueRecord {
            issue_id: format!("issue_{:04}", issues.len() + 1),
            title: title.clone(),
            description: input.description.filter(|value| !value.trim().is_empty()),
            status: IssueStatus::Draft,
            workspace_id: None,
            task_id: None,
            session_id: None,
            change_id: input.change_id.unwrap_or_else(|| slugify(&title)),
            created_at: now,
            updated_at: now,
        };
        issues.push(issue.clone());
        self.write(&issues)?;
        Ok(issue)
    }

    pub fn get(&self, issue_id: &str) -> Result<IssueRecord, IssueRegistryError> {
        self.list()?
            .into_iter()
            .find(|issue| issue.issue_id == issue_id)
            .ok_or_else(|| registry_error("issue_not_found", issue_id))
    }

    pub fn mark_started(
        &self,
        issue_id: &str,
        workspace_id: &str,
        task_id: &str,
        session_id: &str,
    ) -> Result<IssueRecord, IssueRegistryError> {
        let mut issues = self.list()?;
        let issue = issues
            .iter_mut()
            .find(|issue| issue.issue_id == issue_id)
            .ok_or_else(|| registry_error("issue_not_found", issue_id))?;
        if issue.task_id.is_none() {
            issue.status = IssueStatus::Started;
            issue.workspace_id = Some(workspace_id.to_string());
            issue.task_id = Some(task_id.to_string());
            issue.session_id = Some(session_id.to_string());
            issue.updated_at = Utc::now();
        }
        let cloned = issue.clone();
        self.write(&issues)?;
        Ok(cloned)
    }

    pub fn find_by_task(&self, task_id: &str) -> Result<TaskWorkspaceLink, IssueRegistryError> {
        let issue = self
            .list()?
            .into_iter()
            .find(|issue| issue.task_id.as_deref() == Some(task_id))
            .ok_or_else(|| registry_error("task_workspace_not_found", task_id))?;
        Ok(TaskWorkspaceLink {
            issue_id: issue.issue_id,
            workspace_id: issue
                .workspace_id
                .ok_or_else(|| registry_error("issue_missing_workspace", task_id))?,
        })
    }

    fn path(&self) -> PathBuf {
        self.app_root.join(".aria/runtime/web/issues.json")
    }

    fn write(&self, issues: &[IssueRecord]) -> Result<(), IssueRegistryError> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let file = fs::File::create(path).map_err(io_error)?;
        serde_json::to_writer_pretty(file, issues).map_err(json_error)
    }
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

fn registry_error(code: &'static str, value: &str) -> IssueRegistryError {
    IssueRegistryError {
        code,
        message: value.to_string(),
    }
}

fn io_error(error: std::io::Error) -> IssueRegistryError {
    IssueRegistryError {
        code: "issue_registry_io",
        message: error.to_string(),
    }
}

fn json_error(error: serde_json::Error) -> IssueRegistryError {
    IssueRegistryError {
        code: "issue_registry_json",
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_lists_and_persists_issues() {
        let app = tempdir().expect("tempdir");
        let registry = IssueRegistry::new(app.path().to_path_buf());

        let issue = registry
            .create(CreateIssueInput {
                title: "Add workspace picker".to_string(),
                description: Some("Choose a code repo before start".to_string()),
                change_id: None,
            })
            .expect("create issue");

        assert_eq!(issue.issue_id, "issue_0001");
        assert_eq!(issue.status, IssueStatus::Draft);
        assert_eq!(issue.change_id, "add-workspace-picker");

        let reloaded = IssueRegistry::new(app.path().to_path_buf())
            .list()
            .expect("list issues");
        assert_eq!(reloaded, vec![issue]);
    }

    #[test]
    fn start_links_issue_to_workspace_and_task_once() {
        let app = tempdir().expect("tempdir");
        let registry = IssueRegistry::new(app.path().to_path_buf());
        let issue = registry
            .create(CreateIssueInput {
                title: "Run selected repo".to_string(),
                description: None,
                change_id: None,
            })
            .expect("create issue");

        let started = registry
            .mark_started(
                &issue.issue_id,
                "workspace_0001",
                "task_0001",
                "sess_task_0001",
            )
            .expect("mark started");
        let started_again = registry
            .mark_started(
                &issue.issue_id,
                "workspace_9999",
                "task_9999",
                "sess_task_9999",
            )
            .expect("mark started again");

        assert_eq!(started.workspace_id.as_deref(), Some("workspace_0001"));
        assert_eq!(started.task_id.as_deref(), Some("task_0001"));
        assert_eq!(
            started_again.workspace_id.as_deref(),
            Some("workspace_0001")
        );
        assert_eq!(started_again.task_id.as_deref(), Some("task_0001"));
    }

    #[test]
    fn finds_workspace_for_task() {
        let app = tempdir().expect("tempdir");
        let registry = IssueRegistry::new(app.path().to_path_buf());
        let issue = registry
            .create(CreateIssueInput {
                title: "Lookup task".to_string(),
                description: None,
                change_id: None,
            })
            .expect("create issue");
        registry
            .mark_started(
                &issue.issue_id,
                "workspace_0001",
                "task_0001",
                "sess_task_0001",
            )
            .expect("start");

        let link = registry.find_by_task("task_0001").expect("find task");

        assert_eq!(link.issue_id, "issue_0001");
        assert_eq!(link.workspace_id, "workspace_0001");
    }
}
