use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommandRecord {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitCommandError {
    #[error("git command io error: {0}")]
    Io(String),
    #[error("git command failed: {record:?}")]
    Failed { record: GitCommandRecord },
}

pub fn run_git(cwd: &Path, args: &[String]) -> Result<GitCommandRecord, GitCommandError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| GitCommandError::Io(error.to_string()))?;
    let record = GitCommandRecord {
        cwd: cwd.to_path_buf(),
        args: args.to_vec(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    };
    if output.status.success() {
        Ok(record)
    } else {
        Err(GitCommandError::Failed { record })
    }
}

pub fn git_stdout(cwd: &Path, args: &[String]) -> Result<String, GitCommandError> {
    run_git(cwd, args).map(|record| record.stdout.trim().to_string())
}

pub fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}
