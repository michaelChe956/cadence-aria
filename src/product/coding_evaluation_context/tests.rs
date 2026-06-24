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
use crate::product::models::{ProviderName, WorkspaceType};
use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion, ProviderConfigSnapshot};

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
                payload: ArtifactPayload::Markdown {
                    markdown: "# Work Item\n\n## 验证命令\n- cargo test --locked".to_string(),
                    diff: None,
                },
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
                payload: ArtifactPayload::Markdown {
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
                    diff: None,
                },
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
