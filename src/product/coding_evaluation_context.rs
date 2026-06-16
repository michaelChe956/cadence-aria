use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::QualityGateBypassAudit;
use crate::product::coding_models::{CodingExecutionAttempt, CodingProviderRole};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    DesignSpecRecord, LifecycleWorkItemRecord, SpecVersionRecord, StorySpecRecord,
    WorkspaceSessionRecord, WorkspaceType,
};
use crate::web::workspace_ws_types::ArtifactVersion;

const MAX_CONTEXT_SECTION_CHARS: usize = 30_000;
const MAX_DIFF_CONTEXT_CHARS: usize = 12_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationContextRole {
    Tester,
    Analyst,
    CodeReviewer,
    InternalReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationContextPack {
    pub issue_id: String,
    pub attempt_id: String,
    pub provider_role: EvaluationContextRole,
    pub story_specs: Vec<EvaluationSpecContext>,
    pub design_specs: Vec<EvaluationSpecContext>,
    pub work_item: EvaluationWorkItemContext,
    pub repo_context: EvaluationRepoContext,
    pub openspec_context: OpenSpecContext,
    pub superpowers_context: SuperpowersContext,
    pub quality_bypass_audits: Vec<QualityGateBypassAudit>,
    pub context_warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationSpecContext {
    pub artifact_id: String,
    pub version_id: Option<String>,
    pub version: Option<u32>,
    pub title: String,
    pub raw_markdown_or_sections: String,
    pub workspace_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationWorkItemContext {
    pub artifact_id: String,
    pub version_id: Option<String>,
    pub version: Option<u32>,
    pub title: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub raw_markdown_or_sections: String,
    pub workspace_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationRepoContext {
    pub repository_id: Option<String>,
    pub branch_name: String,
    pub base_branch: String,
    pub worktree_path: Option<String>,
    pub changed_files: Vec<String>,
    pub diff_stat: String,
    pub diff_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenSpecContext {
    pub enabled: bool,
    pub active_change_id: Option<String>,
    pub relevant_requirements: Vec<String>,
    pub traceability_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuperpowersContext {
    pub enabled: bool,
    pub required_methods_by_role: BTreeMap<String, Vec<String>>,
}

pub fn build_evaluation_context_pack(
    paths: ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    provider_role: EvaluationContextRole,
) -> Result<EvaluationContextPack, ProductStoreError> {
    let coding_store = CodingAttemptStore::new(paths.clone());
    let quality_bypass_audits = coding_store.list_quality_bypass_audits(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let lifecycle = LifecycleStore::new(paths);
    let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
    let work_item = lifecycle
        .list_work_items(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|record| record.id == attempt.work_item_id);

    let mut context_warnings = Vec::new();
    let Some(work_item) = work_item else {
        context_warnings.push("missing_work_item".to_string());
        return Ok(EvaluationContextPack {
            issue_id: attempt.issue_id.clone(),
            attempt_id: attempt.id.clone(),
            provider_role,
            story_specs: Vec::new(),
            design_specs: Vec::new(),
            work_item: EvaluationWorkItemContext {
                artifact_id: attempt.work_item_id.clone(),
                version_id: None,
                version: None,
                title: String::new(),
                repository_id: String::new(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                raw_markdown_or_sections: String::new(),
                workspace_session_id: None,
            },
            repo_context: repo_context(attempt, None, &mut context_warnings),
            openspec_context: OpenSpecContext {
                enabled: false,
                active_change_id: None,
                relevant_requirements: Vec::new(),
                traceability_notes: Vec::new(),
            },
            superpowers_context: SuperpowersContext {
                enabled: false,
                required_methods_by_role: required_methods_by_role(),
            },
            quality_bypass_audits,
            context_warnings,
        });
    };

    let stories = lifecycle.list_story_specs(&attempt.project_id, &attempt.issue_id)?;
    let designs = lifecycle.list_design_specs(&attempt.project_id, &attempt.issue_id)?;
    let story_specs = contexts_for_story_specs(
        &lifecycle,
        &attempt.project_id,
        &attempt.issue_id,
        &work_item.story_spec_ids,
        &stories,
        &sessions,
        &mut context_warnings,
    )?;
    let design_specs = contexts_for_design_specs(
        &lifecycle,
        &attempt.project_id,
        &attempt.issue_id,
        &work_item.design_spec_ids,
        &designs,
        &sessions,
        &mut context_warnings,
    )?;
    let work_item_session = latest_session_for(&sessions, &work_item.id, &WorkspaceType::WorkItem);
    let work_item_version = latest_artifact_version_for_session(&lifecycle, work_item_session)?;
    let work_item_context = work_item_context(
        &work_item,
        work_item_version.as_ref(),
        work_item_session,
        &mut context_warnings,
    );
    let openspec_enabled = sessions.iter().any(|session| session.openspec_enabled);
    let superpowers_enabled = sessions.iter().any(|session| session.superpowers_enabled);

    Ok(EvaluationContextPack {
        issue_id: attempt.issue_id.clone(),
        attempt_id: attempt.id.clone(),
        provider_role,
        story_specs,
        design_specs,
        work_item: work_item_context,
        repo_context: repo_context(attempt, Some(&work_item), &mut context_warnings),
        openspec_context: OpenSpecContext {
            enabled: openspec_enabled,
            active_change_id: None,
            relevant_requirements: Vec::new(),
            traceability_notes: Vec::new(),
        },
        superpowers_context: SuperpowersContext {
            enabled: superpowers_enabled,
            required_methods_by_role: required_methods_by_role(),
        },
        quality_bypass_audits,
        context_warnings,
    })
}

fn contexts_for_story_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    ids: &[String],
    stories: &[StorySpecRecord],
    sessions: &[WorkspaceSessionRecord],
    warnings: &mut Vec<String>,
) -> Result<Vec<EvaluationSpecContext>, ProductStoreError> {
    let mut contexts = Vec::new();
    for id in ids {
        let Some(story) = stories.iter().find(|story| &story.id == id) else {
            warnings.push(format!("missing_story_spec:{id}"));
            continue;
        };
        let version = latest_version(lifecycle, project_id, issue_id, id)?;
        let session = latest_session_for(sessions, id, &WorkspaceType::Story);
        contexts.push(spec_context(
            &story.id,
            &story.title,
            version.as_ref(),
            session,
            warnings,
        ));
    }
    Ok(contexts)
}

fn contexts_for_design_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    ids: &[String],
    designs: &[DesignSpecRecord],
    sessions: &[WorkspaceSessionRecord],
    warnings: &mut Vec<String>,
) -> Result<Vec<EvaluationSpecContext>, ProductStoreError> {
    let mut contexts = Vec::new();
    for id in ids {
        let Some(design) = designs.iter().find(|design| &design.id == id) else {
            warnings.push(format!("missing_design_spec:{id}"));
            continue;
        };
        let version = latest_version(lifecycle, project_id, issue_id, id)?;
        let session = latest_session_for(sessions, id, &WorkspaceType::Design);
        contexts.push(spec_context(
            &design.id,
            &design.title,
            version.as_ref(),
            session,
            warnings,
        ));
    }
    Ok(contexts)
}

fn latest_version(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_id: &str,
) -> Result<Option<SpecVersionRecord>, ProductStoreError> {
    Ok(lifecycle
        .list_versions(project_id, issue_id, entity_id)?
        .into_iter()
        .max_by_key(|version| version.version))
}

fn latest_artifact_version_for_session(
    lifecycle: &LifecycleStore,
    session: Option<&WorkspaceSessionRecord>,
) -> Result<Option<ArtifactVersion>, ProductStoreError> {
    let Some(session) = session else {
        return Ok(None);
    };
    Ok(lifecycle
        .list_artifact_versions(&session.id)?
        .into_iter()
        .filter(|version| version.is_current)
        .max_by_key(|version| version.version))
}

fn latest_session_for<'a>(
    sessions: &'a [WorkspaceSessionRecord],
    entity_id: &str,
    workspace_type: &WorkspaceType,
) -> Option<&'a WorkspaceSessionRecord> {
    sessions
        .iter()
        .filter(|session| {
            session.entity_id == entity_id && &session.workspace_type == workspace_type
        })
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.created_at.cmp(&right.created_at))
        })
}

fn spec_context(
    artifact_id: &str,
    title: &str,
    version: Option<&SpecVersionRecord>,
    session: Option<&WorkspaceSessionRecord>,
    warnings: &mut Vec<String>,
) -> EvaluationSpecContext {
    let (raw_markdown_or_sections, truncated) = sanitize_context_text(
        &version
            .map(|version| version.markdown.clone())
            .unwrap_or_default(),
    );
    if truncated {
        push_warning_once(warnings, "context_truncated");
    }
    EvaluationSpecContext {
        artifact_id: artifact_id.to_string(),
        version_id: version.map(|version| version.id.clone()),
        version: version.map(|version| version.version),
        title: title.to_string(),
        raw_markdown_or_sections,
        workspace_session_id: session.map(|session| session.id.clone()),
    }
}

fn work_item_context(
    work_item: &LifecycleWorkItemRecord,
    version: Option<&ArtifactVersion>,
    session: Option<&WorkspaceSessionRecord>,
    warnings: &mut Vec<String>,
) -> EvaluationWorkItemContext {
    let (raw_markdown_or_sections, truncated) = sanitize_context_text(
        &version
            .map(|version| version.markdown.clone())
            .unwrap_or_default(),
    );
    if truncated {
        push_warning_once(warnings, "context_truncated");
    }
    EvaluationWorkItemContext {
        artifact_id: work_item.id.clone(),
        version_id: version.map(|version| format!("artifact_version_{:04}", version.version)),
        version: version.map(|version| version.version),
        title: work_item.title.clone(),
        repository_id: work_item.repository_id.clone(),
        story_spec_ids: work_item.story_spec_ids.clone(),
        design_spec_ids: work_item.design_spec_ids.clone(),
        raw_markdown_or_sections,
        workspace_session_id: session.map(|session| session.id.clone()),
    }
}

fn sanitize_context_text(input: &str) -> (String, bool) {
    let mut lines = Vec::new();
    let mut in_private_key_block = false;
    for line in input.lines() {
        let lower = line.to_ascii_lowercase();
        if in_private_key_block {
            if lower.contains("-----end") && lower.contains("private key") {
                in_private_key_block = false;
            }
            continue;
        }
        if lower.contains("-----begin") && lower.contains("private key") {
            lines.push("[REDACTED_PRIVATE_KEY]".to_string());
            in_private_key_block = true;
            continue;
        }
        if contains_sensitive_keyword(&lower) {
            lines.push("[REDACTED]".to_string());
        } else {
            lines.push(line.to_string());
        }
    }

    let sanitized = lines.join("\n");
    if sanitized.len() <= MAX_CONTEXT_SECTION_CHARS {
        return (sanitized, false);
    }
    (
        truncate_to_char_boundary(&sanitized, MAX_CONTEXT_SECTION_CHARS),
        true,
    )
}

fn contains_sensitive_keyword(lower_line: &str) -> bool {
    [
        "api_key",
        "token",
        "secret",
        "password",
        "authorization",
        "private key",
    ]
    .iter()
    .any(|keyword| lower_line.contains(keyword))
}

fn truncate_to_char_boundary(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }
    let mut end = max_len;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn push_warning_once(warnings: &mut Vec<String>, warning: &str) {
    if !warnings.iter().any(|existing| existing == warning) {
        warnings.push(warning.to_string());
    }
}

fn repo_context(
    attempt: &CodingExecutionAttempt,
    work_item: Option<&LifecycleWorkItemRecord>,
    warnings: &mut Vec<String>,
) -> EvaluationRepoContext {
    let (changed_files, diff_stat, diff_truncated) = attempt
        .worktree_path
        .as_ref()
        .map_or((Vec::new(), String::new(), false), |worktree_path| {
            diff_context(worktree_path, &attempt.base_branch, warnings)
        });
    EvaluationRepoContext {
        repository_id: work_item.map(|work_item| work_item.repository_id.clone()),
        branch_name: attempt.branch_name.clone(),
        base_branch: attempt.base_branch.clone(),
        worktree_path: attempt
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string()),
        changed_files,
        diff_stat,
        diff_truncated,
    }
}

fn diff_context(
    worktree_path: &Path,
    base_branch: &str,
    warnings: &mut Vec<String>,
) -> (Vec<String>, String, bool) {
    let Some(name_only) = git_stdout(worktree_path, &["diff", "--name-only", base_branch]) else {
        push_warning_once(warnings, "diff_unavailable");
        return (Vec::new(), String::new(), false);
    };
    let stat = git_stdout(worktree_path, &["diff", "--stat", base_branch]).unwrap_or_default();
    let untracked = git_stdout(
        worktree_path,
        &["ls-files", "--others", "--exclude-standard"],
    )
    .unwrap_or_default();

    let mut changed_files = name_only
        .lines()
        .chain(untracked.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    changed_files.sort();
    changed_files.dedup();

    let combined_stat = if untracked.trim().is_empty() {
        stat
    } else {
        format!("{stat}\nUntracked files:\n{untracked}")
    };
    let (diff_stat, diff_truncated) = sanitize_diff_text(&combined_stat);
    (changed_files, diff_stat, diff_truncated)
}

fn git_stdout(worktree_path: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

fn sanitize_diff_text(input: &str) -> (String, bool) {
    let (sanitized, redaction_truncated) = sanitize_context_text(input);
    if sanitized.len() <= MAX_DIFF_CONTEXT_CHARS {
        return (sanitized, redaction_truncated);
    }
    (
        truncate_to_char_boundary(&sanitized, MAX_DIFF_CONTEXT_CHARS),
        true,
    )
}

fn required_methods_by_role() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        (
            role_key(&CodingProviderRole::Tester),
            vec![
                "systematic_debugging".to_string(),
                "verification_before_completion".to_string(),
            ],
        ),
        (
            role_key(&CodingProviderRole::Analyst),
            vec![
                "systematic_debugging".to_string(),
                "receiving_code_review".to_string(),
            ],
        ),
        (
            role_key(&CodingProviderRole::CodeReviewer),
            vec![
                "requesting_code_review".to_string(),
                "verification_before_completion".to_string(),
            ],
        ),
        (
            role_key(&CodingProviderRole::InternalReviewer),
            vec![
                "requesting_code_review".to_string(),
                "verification_before_completion".to_string(),
            ],
        ),
    ])
}

fn role_key(role: &CodingProviderRole) -> String {
    match role {
        CodingProviderRole::Coder => "coder",
        CodingProviderRole::Tester => "tester",
        CodingProviderRole::Analyst => "analyst",
        CodingProviderRole::CodeReviewer => "code_reviewer",
        CodingProviderRole::InternalReviewer => "internal_reviewer",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::Command as StdCommand;

    use tempfile::TempDir;

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_models::{
        CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    };
    use crate::product::lifecycle_store::{
        AppendSpecVersionInput, CreateDesignSpecInput, CreateStorySpecInput, CreateWorkItemInput,
        CreateWorkspaceSessionInput, LifecycleStore,
    };
    use crate::product::models::{DesignKind, ProviderName, WorkspaceType};
    use crate::web::workspace_ws_types::{ArtifactVersion, ProviderConfigSnapshot};

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const REPOSITORY_ID: &str = "repository_0001";

    #[test]
    fn evaluation_context_pack_includes_story_design_work_item_and_contracts() {
        let tmp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());

        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                title: "Story".to_string(),
            })
            .unwrap();
        lifecycle
            .append_version(AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: story.id.clone(),
                markdown: "# Story\n\n## Acceptance Criteria\n- Works".to_string(),
                provider_run_refs: vec!["author_run_story".to_string()],
                review_refs: Vec::new(),
                confirmed_by: Some("user".to_string()),
            })
            .unwrap();
        lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: story.id.clone(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap();

        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                design_kind: DesignKind::Backend,
                title: "Design".to_string(),
            })
            .unwrap();
        lifecycle
            .append_version(AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: design.id.clone(),
                markdown: "# Design\n\n## Security\n- Validate input".to_string(),
                provider_run_refs: vec!["author_run_design".to_string()],
                review_refs: Vec::new(),
                confirmed_by: Some("user".to_string()),
            })
            .unwrap();
        lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: design.id.clone(),
                workspace_type: WorkspaceType::Design,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap();

        let work_item = lifecycle
            .create_work_item(CreateWorkItemInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                design_spec_ids: vec![design.id.clone()],
                title: "Work Item".to_string(),
                ..Default::default()
            })
            .unwrap();
        let work_item_session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: work_item.id.clone(),
                workspace_type: WorkspaceType::WorkItem,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap();
        lifecycle
            .append_artifact_version(
                &work_item_session.id,
                ArtifactVersion {
                    version: 1,
                    markdown: "# Work Item\n\n## 验证命令\n- cargo test --locked".to_string(),
                    generated_by: ProviderName::Codex,
                    reviewed_by: Some(ProviderName::ClaudeCode),
                    review_verdict: None,
                    confirmed_by: Some("user".to_string()),
                    is_current: true,
                    created_at: "2026-06-10T00:00:00Z".to_string(),
                    source_node_id: "author_run_work_item".to_string(),
                },
            )
            .unwrap();

        let attempt = CodingExecutionAttempt {
            id: "coding_attempt_0001".to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: work_item.id,
            attempt_no: 1,
            status: CodingAttemptStatus::Running,
            stage: CodingExecutionStage::Testing,
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            rework_count: 0,
            max_auto_rework: 2,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-06-10T00:00:00Z".to_string(),
            updated_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: None,
        };

        let pack =
            build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Tester).unwrap();

        assert!(
            pack.story_specs[0]
                .raw_markdown_or_sections
                .contains("Acceptance Criteria")
        );
        assert!(
            pack.design_specs[0]
                .raw_markdown_or_sections
                .contains("Security")
        );
        assert!(pack.work_item.raw_markdown_or_sections.contains("验证命令"));
        assert!(pack.openspec_context.enabled);
        assert!(pack.superpowers_context.enabled);
        assert!(
            pack.superpowers_context
                .required_methods_by_role
                .contains_key("tester")
        );
        assert!(
            pack.superpowers_context
                .required_methods_by_role
                .contains_key("analyst")
        );
        assert!(
            pack.superpowers_context
                .required_methods_by_role
                .contains_key("code_reviewer")
        );
        assert!(
            pack.superpowers_context
                .required_methods_by_role
                .contains_key("internal_reviewer")
        );
    }

    #[test]
    fn evaluation_context_pack_includes_attempt_diff_context() {
        let tmp = TempDir::new().unwrap();
        let worktree = tmp.path().join("worktree");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(worktree.join("src.txt"), "before\n").unwrap();
        init_repo(&worktree);
        fs::write(worktree.join("src.txt"), "before\nafter\n").unwrap();
        fs::write(worktree.join("new.txt"), "new file\n").unwrap();

        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let work_item = lifecycle
            .create_work_item(CreateWorkItemInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: "Diff Work Item".to_string(),
                ..Default::default()
            })
            .unwrap();

        let attempt = CodingExecutionAttempt {
            id: "coding_attempt_0001".to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: work_item.id,
            attempt_no: 1,
            status: CodingAttemptStatus::Running,
            stage: CodingExecutionStage::Testing,
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            rework_count: 0,
            max_auto_rework: 2,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-06-10T00:00:00Z".to_string(),
            updated_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: None,
        };

        let pack =
            build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Tester).unwrap();

        assert_eq!(pack.repo_context.changed_files, vec!["new.txt", "src.txt"]);
        assert!(pack.repo_context.diff_stat.contains("src.txt"));
        assert!(pack.repo_context.diff_stat.contains("Untracked files"));
        assert!(pack.repo_context.diff_stat.contains("new.txt"));
        assert!(!pack.repo_context.diff_truncated);
    }

    #[test]
    fn evaluation_context_pack_truncates_and_redacts_sensitive_lines() {
        let tmp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let work_item = lifecycle
            .create_work_item(CreateWorkItemInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: "Sensitive Work Item".to_string(),
                ..Default::default()
            })
            .unwrap();
        let work_item_session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: work_item.id.clone(),
                workspace_type: WorkspaceType::WorkItem,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
            })
            .unwrap();
        lifecycle
            .append_artifact_version(
                &work_item_session.id,
                ArtifactVersion {
                    version: 1,
                    markdown: format!(
                        "## Acceptance Criteria\n\
                         normal requirement\n\
                         api_key = \"should-not-leak\"\n\
                         Authorization: Bearer should-not-leak\n\
                         -----BEGIN PRIVATE KEY-----\n\
                         should-not-leak\n\
                         -----END PRIVATE KEY-----\n\
                         {}",
                        "x".repeat(30_200)
                    ),
                    generated_by: ProviderName::Codex,
                    reviewed_by: Some(ProviderName::ClaudeCode),
                    review_verdict: None,
                    confirmed_by: Some("user".to_string()),
                    is_current: true,
                    created_at: "2026-06-10T00:00:00Z".to_string(),
                    source_node_id: "author_run_work_item".to_string(),
                },
            )
            .unwrap();

        let attempt = CodingExecutionAttempt {
            id: "coding_attempt_0001".to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: work_item.id,
            attempt_no: 1,
            status: CodingAttemptStatus::Running,
            stage: CodingExecutionStage::Testing,
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            rework_count: 0,
            max_auto_rework: 2,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-06-10T00:00:00Z".to_string(),
            updated_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: None,
        };

        let pack =
            build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Tester).unwrap();
        let markdown = &pack.work_item.raw_markdown_or_sections;
        assert!(markdown.contains("normal requirement"));
        assert!(!markdown.contains("should-not-leak"));
        assert!(markdown.contains("[REDACTED]"));
        assert!(markdown.len() <= 30_000);
        assert!(
            pack.context_warnings
                .iter()
                .any(|warning| warning == "context_truncated")
        );
    }

    fn init_repo(repo: &Path) {
        run_git(repo, &["init"]);
        run_git(repo, &["config", "user.email", "aria@example.com"]);
        run_git(repo, &["config", "user.name", "Aria Test"]);
        run_git(repo, &["add", "."]);
        run_git(repo, &["commit", "-m", "initial"]);
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
        if !output.status.success() {
            panic!(
                "git {} failed\nstdout:\n{}\nstderr:\n{}",
                args.join(" "),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
