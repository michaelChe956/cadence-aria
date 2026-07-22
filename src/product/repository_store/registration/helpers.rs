use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use super::{BoundedCommandResult, CadenceSkillsError, CadenceSkillsPreparationResult};
use crate::product::repository_store::{
    CadenceSkillsPreparationSummary, RepositoryRegistrationError,
};

pub(super) fn command_succeeded(result: &BoundedCommandResult) -> bool {
    result.exit_code == Some(0) && !result.timed_out && !result.cancelled
}

pub(super) fn command_diagnostic(result: &BoundedCommandResult) -> String {
    let message = if result.stderr.trim().is_empty() {
        result.stdout.trim()
    } else {
        result.stderr.trim()
    };
    if result.timed_out {
        format!("git command timed out: {message}")
    } else if result.cancelled {
        format!("git command cancelled: {message}")
    } else {
        format!("git command exited {:?}: {message}", result.exit_code)
    }
}

pub(super) fn parse_porcelain_status(output: &str) -> BTreeMap<String, String> {
    let records = output.split('\0').collect::<Vec<_>>();
    let mut status = BTreeMap::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        if record.len() < 4 {
            index += 1;
            continue;
        }
        let code = record[..2].to_string();
        let path = record[3..].to_string();
        status.insert(path, code.clone());
        if code.contains('R') || code.contains('C') {
            index += 1;
            if let Some(original) = records.get(index)
                && !original.is_empty()
            {
                status.insert((*original).to_string(), code);
            }
        }
        index += 1;
    }
    status
}

pub(super) fn changed_paths(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn preparation_summary(
    prepared: &CadenceSkillsPreparationResult,
) -> CadenceSkillsPreparationSummary {
    CadenceSkillsPreparationSummary {
        source_mode: prepared.source_mode.as_str().to_string(),
        source_root: prepared.source_root.clone(),
        skills_root: prepared.skills_root.clone(),
        git_updated: prepared.git_updated,
        link_sync_status: prepared.link_sync_status.as_str().to_string(),
        warnings: prepared.warnings.clone(),
    }
}

pub(super) fn cadence_error(error: CadenceSkillsError) -> RepositoryRegistrationError {
    claude_error(
        "cadence_skills_prepare",
        error.code(),
        &error.to_string(),
        true,
        "Repair Cadence-skills availability or synchronization, then retry.",
    )
}

pub(super) fn claude_error(
    stage: &str,
    reason_code: &str,
    reason: &str,
    retryable: bool,
    action: &str,
) -> RepositoryRegistrationError {
    let mut error = registration_error(stage, reason_code, reason, retryable, action);
    error.provider = Some("claude_code".to_string());
    error
}

pub(super) fn git_state_error(stage: &str, reason: &str) -> RepositoryRegistrationError {
    registration_error(
        stage,
        "repository_git_state_failed",
        reason,
        true,
        "Git state could not be determined; inspect the repository manually before retrying.",
    )
}

pub(super) fn registration_error(
    stage: &str,
    reason_code: &str,
    reason: &str,
    retryable: bool,
    action: &str,
) -> RepositoryRegistrationError {
    RepositoryRegistrationError::new(
        stage,
        reason_code,
        Some(sanitize_summary(reason, 4 * 1024)),
        retryable,
        action,
    )
}

pub(super) fn sanitize_summary(value: &str, limit: usize) -> String {
    let mut summary = String::new();
    let mut in_escape = false;
    for character in value.chars() {
        if in_escape {
            if character.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if character == '\u{1b}' {
            in_escape = true;
            continue;
        }
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            continue;
        }
        if summary.len() + character.len_utf8() > limit {
            summary.push_str("…[truncated]");
            break;
        }
        summary.push(character);
    }
    summary
        .split_whitespace()
        .map(|token| {
            let Some((key, _)) = token.split_once('=') else {
                return token.to_string();
            };
            let upper = key.to_ascii_uppercase();
            if ["KEY", "TOKEN", "SECRET", "PASSWORD"]
                .iter()
                .any(|marker| upper.contains(marker))
            {
                format!("{key}=[REDACTED]")
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

static INITIALIZING_PATHS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

pub(super) struct InitializationGuard {
    path: PathBuf,
}

impl InitializationGuard {
    pub(super) fn try_acquire(path: PathBuf) -> Result<Self, Box<RepositoryRegistrationError>> {
        let paths = INITIALIZING_PATHS.get_or_init(|| Mutex::new(HashSet::new()));
        let mut paths = paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !paths.insert(path.clone()) {
            return Err(Box::new(registration_error(
                "repository_initialization_lock",
                "repository_initialization_in_progress",
                "repository initialization is already in progress for this canonical Git root",
                true,
                "Wait for the active initialization to finish, then retry.",
            )));
        }
        Ok(Self { path })
    }
}

impl Drop for InitializationGuard {
    fn drop(&mut self) {
        if let Some(paths) = INITIALIZING_PATHS.get() {
            paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&self.path);
        }
    }
}
