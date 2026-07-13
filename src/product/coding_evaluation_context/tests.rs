use super::*;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use tempfile::TempDir;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAgentRole, CodingAttemptStatus, CodingChatEntry, CodingEntryType, CodingExecutionAttempt,
    CodingExecutionStage, CodingProviderRole, CodingRoleRunStatus, CodingRoleRunTrigger,
    WorkItemHandoff,
};
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateIssueWorkItemPlanInput,
    CreateStorySpecInput, CreateVerificationPlanInput, CreateWorkItemInput,
    CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::models::{
    IssueWorkItemPlan, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName,
    RepositoryProfileConfidence, VerificationCommand, VerificationCommandSafety,
    VerificationCommandSource, VerificationFallbackPolicy, VerificationScope,
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus, WorkItemGenerationMode,
    WorkItemKind, WorkItemPlanCommitState, WorkItemPlanCompileStatus,
    WorkItemPlanCompileTransaction, WorkItemPlanStatus, WorkspaceType,
};
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion, ProviderConfigSnapshot};

mod group_context;
mod tester_execution;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const REPOSITORY_ID: &str = "repository_0001";

#[test]
fn evaluation_context_uses_compiled_work_item_without_artifact_version() {
    let tmp = TempDir::new().expect("tempdir");
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let verification_plan_id = "verification_plan_0001";

    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "Compiled evaluation context".to_string(),
            planned_implementation_context: Some(
                "compiled evaluation implementation context".to_string(),
            ),
            planned_handoff_summary: Some("compiled evaluation handoff".to_string()),
            kind: WorkItemKind::Backend,
            forbidden_write_scopes: vec!["tests/**".to_string()],
            verification_plan_ref: Some(verification_plan_id.to_string()),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item");
    lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some(verification_plan_id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_0001".to_string(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_evaluation_context".to_string(),
                label: "evaluation context test".to_string(),
                command: "cargo test --locked --lib evaluation_context".to_string(),
                cwd: ".".to_string(),
                purpose: "prove evaluation context uses compiled work item".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: vec!["cmd_evaluation_context".to_string()],
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("create verification plan");

    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::CodeReview,
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
        },
        provider_conversations: Vec::new(),
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-07-14T00:00:00Z".to_string(),
        updated_at: "2026-07-14T00:00:00Z".to_string(),
        completed_at: None,
    };

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::CodeReviewer)
        .expect("evaluation context");

    assert!(
        pack.work_item
            .raw_markdown_or_sections
            .contains("compiled evaluation implementation context")
    );
    assert!(pack.work_item.raw_markdown_or_sections.contains("tests/**"));
    assert!(
        pack.work_item
            .raw_markdown_or_sections
            .contains("cargo test --locked --lib evaluation_context")
    );
}

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
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
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
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
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
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
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
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
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
fn code_reviewer_context_pack_includes_coder_evidence() {
    let tmp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let work_item = lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "Evidence Work Item".to_string(),
            ..Default::default()
        })
        .unwrap();
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        work_item_id: work_item.id.clone(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::CodeReview,
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
        work_item_group_id: None,
        current_work_item_id: Some(work_item.id.clone()),
        active_unit_id: None,
        head_commit: Some("abc123".to_string()),
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
        completed_at: None,
    };
    store.save_coding_attempt(&attempt).expect("save attempt");
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("create role run");
    let raw_ref = store
        .save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Coding,
            "coder_output",
            "完整 coder 输出",
        )
        .expect("raw output");
    store
        .update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            vec![raw_ref.clone()],
            vec!["artifacts/coder/diff-stat.txt".to_string()],
        )
        .expect("role run refs");
    store
        .update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            CodingRoleRunStatus::Completed,
            None,
        )
        .expect("complete role run");
    store
        .save_chat_entry(&CodingChatEntry {
            id: "coding_node_0001_coder_output".to_string(),
            attempt_id: attempt.id.clone(),
            node_id: Some("coding_node_0001".to_string()),
            role: CodingAgentRole::Author,
            entry_type: CodingEntryType::AssistantMessage,
            content: Some("执行清单\n验证命令输出: all checks passed".to_string()),
            metadata: Some(serde_json::json!({
                "role_run_id": role_run.id,
                "raw_provider_output_ref": raw_ref,
            })),
            created_at: "2026-06-10T00:00:01Z".to_string(),
        })
        .expect("chat entry");
    store
        .save_work_item_handoff(&WorkItemHandoff {
            id: "work_item_handoff_0001".to_string(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            work_item_id: work_item.id.clone(),
            attempt_id: attempt.id.clone(),
            provider_run_ref: None,
            summary: "handoff".to_string(),
            files_changed: Vec::new(),
            commit_sha: Some("abc123".to_string()),
            diff_summary: String::new(),
            tests_run: vec!["./verify".to_string()],
            test_result_summary: "passed".to_string(),
            review_summary: None,
            api_or_contract_changes: Vec::new(),
            open_risks: Vec::new(),
            next_work_item_notes: Vec::new(),
            created_at: "2026-06-10T00:00:02Z".to_string(),
        })
        .expect("handoff");

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::CodeReviewer)
        .expect("context pack");
    let evidence = pack.coder_evidence.expect("coder evidence");

    assert_eq!(
        evidence.latest_role_run_id.as_deref(),
        Some(role_run.id.as_str())
    );
    assert_eq!(evidence.run_no, Some(1));
    assert_eq!(evidence.raw_provider_output_refs, vec![raw_ref]);
    assert_eq!(
        evidence.artifact_refs,
        vec!["artifacts/coder/diff-stat.txt".to_string()]
    );
    assert!(
        evidence
            .completion_report_excerpt
            .as_deref()
            .is_some_and(|excerpt| excerpt.contains("验证命令输出"))
    );
    assert_eq!(evidence.handoff_tests_run, vec!["./verify"]);
    assert_eq!(
        evidence.handoff_test_result_summary.as_deref(),
        Some("passed")
    );
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
        scope: crate::product::coding_models::CodingAttemptScope::WorkItem,
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
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
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
