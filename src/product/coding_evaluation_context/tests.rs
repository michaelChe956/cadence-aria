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
    AppendSpecVersionInput, CreateDesignSpecInput, CreateIssueWorkItemPlanInput,
    CreateStorySpecInput, CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::models::{
    IssueWorkItemPlan, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, ProviderName,
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus, WorkItemGenerationMode,
    WorkItemPlanCommitState, WorkItemPlanCompileStatus, WorkItemPlanCompileTransaction,
    WorkItemPlanStatus, WorkspaceType,
};
use crate::product::work_item_plan_store::WorkItemPlanStore;
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

#[test]
fn group_attempt_uses_current_work_item_as_execution_context() {
    let (_tmp, paths, attempt) = group_attempt_with_two_work_items(false);

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");

    assert_eq!(pack.work_item.artifact_id, "work_item_0001");
    assert_eq!(
        pack.group_context.as_ref().expect("group").plan_id,
        "work_item_plan_0001"
    );
    assert_eq!(
        pack.group_context
            .as_ref()
            .expect("group")
            .sibling_work_item_ids,
        vec!["work_item_0001".to_string(), "work_item_0002".to_string()]
    );
}

#[test]
fn group_context_warns_when_current_work_item_is_not_in_plan() {
    let (_tmp, paths, mut attempt) = group_attempt_with_two_work_items(false);
    attempt.current_work_item_id = Some("work_item_outside".to_string());

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");

    assert!(
        pack.context_warnings
            .contains(&"group_plan_mapping_mismatch".to_string())
    );
}

#[test]
fn group_context_includes_source_draft_mapping_when_compile_context_exists() {
    let (_tmp, paths, attempt) = group_attempt_with_two_work_items(true);

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");
    let group_context = pack.group_context.expect("group context");

    assert_eq!(
        group_context.source_outline_id.as_deref(),
        Some("outline_backend")
    );
    assert_eq!(
        group_context.source_draft_id.as_deref(),
        Some("draft_backend")
    );
    assert!(
        pack.context_warnings
            .contains(&"group_draft_context_loaded".to_string())
    );
}

#[test]
fn group_context_warns_when_compile_draft_mapping_is_unavailable() {
    let (_tmp, paths, attempt) = group_attempt_with_two_work_items(false);

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");
    let group_context = pack.group_context.expect("group context");

    assert_eq!(group_context.source_outline_id, None);
    assert_eq!(group_context.source_draft_id, None);
    assert!(
        pack.context_warnings
            .contains(&"group_draft_context_unavailable".to_string())
    );
}

fn group_attempt_with_two_work_items(
    with_compile_context: bool,
) -> (TempDir, ProductAppPaths, CodingExecutionAttempt) {
    let tmp = TempDir::new().expect("tmp");
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let plan = create_group_plan_fixture(&lifecycle);
    if with_compile_context {
        save_compile_context_fixture(&paths, &plan);
    }

    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: crate::product::coding_models::CodingAttemptScope::WorkItemGroup,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::Coding,
        base_branch: "main".to_string(),
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
        },
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: Some(plan.id),
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: Some("coding_execution_unit_0001".to_string()),
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
        completed_at: None,
    };
    (tmp, paths, attempt)
}

fn create_group_plan_fixture(lifecycle: &LifecycleStore) -> IssueWorkItemPlan {
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Work Item 1".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            kind: Default::default(),
            sequence_hint: Some(10),
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: Default::default(),
            required_handoff_from: Vec::new(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            plan_status: WorkItemPlanStatus::Confirmed,
        })
        .expect("create work item 1");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "Work Item 2".to_string(),
            work_item_set_id: Some("work_item_plan_0001".to_string()),
            kind: Default::default(),
            sequence_hint: Some(20),
            depends_on: vec!["work_item_0001".to_string()],
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: Default::default(),
            required_handoff_from: vec!["work_item_0001".to_string()],
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            plan_status: WorkItemPlanStatus::Confirmed,
        })
        .expect("create work item 2");
    lifecycle
        .update_work_item_handoff_summary(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            Some("handoff/work_item_0001.md".to_string()),
            None,
        )
        .expect("update handoff ref");
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("work_item_plan_0001".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create plan")
}

fn save_compile_context_fixture(paths: &ProductAppPaths, plan: &IssueWorkItemPlan) {
    let store = WorkItemPlanStore::new(paths.clone());
    let tx = WorkItemPlanCompileTransaction {
        compile_id: "work_item_plan_compile_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan.id.clone(),
        generation_round_id: "generation_round_0001".to_string(),
        outline_version_ref: "outline_version_0001".to_string(),
        active_draft_ids: vec!["draft_backend".to_string(), "draft_frontend".to_string()],
        status: WorkItemPlanCompileStatus::Committed,
        plan_commit_state: WorkItemPlanCommitState::Committed,
        step_cursor: "committed".to_string(),
        outline_to_work_item_id: std::collections::BTreeMap::from([
            ("outline_backend".to_string(), "work_item_0001".to_string()),
            ("outline_frontend".to_string(), "work_item_0002".to_string()),
        ]),
        outline_to_verification_plan_id: std::collections::BTreeMap::new(),
        created_work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
        created_verification_plan_ids: Vec::new(),
        child_session_ids: Vec::new(),
        validator_findings: Vec::new(),
        abort_requested_at: None,
        failure_reason: None,
        previous_plan_snapshot: plan.clone(),
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
        committed_at: Some("2026-06-10T00:01:00Z".to_string()),
    };
    store.put_compile_transaction(&tx).expect("put compile tx");
    store
        .put_draft_record(&draft_record(
            &plan.id,
            "draft_backend",
            "outline_backend",
            "generation_round_0001",
        ))
        .expect("put backend draft");
    store
        .put_draft_record(&draft_record(
            &plan.id,
            "draft_frontend",
            "outline_frontend",
            "generation_round_0001",
        ))
        .expect("put frontend draft");
}

fn draft_record(
    plan_id: &str,
    draft_id: &str,
    outline_id: &str,
    generation_round_id: &str,
) -> WorkItemDraftRecord {
    WorkItemDraftRecord {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        draft_id: draft_id.to_string(),
        outline_id: outline_id.to_string(),
        generation_round_id: generation_round_id.to_string(),
        batch_id: None,
        attempt_index: 1,
        outline_version_ref: "outline_version_0001".to_string(),
        generation_mode: WorkItemGenerationMode::Serial,
        candidate: WorkItemDraftCandidate {
            outline_id: outline_id.to_string(),
            title: format!("{outline_id} title"),
            kind: Default::default(),
            goal: format!("{outline_id} goal"),
            implementation_context: format!("{outline_id} context"),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            depends_on_outline_ids: Vec::new(),
            required_handoff_from_outline_ids: Vec::new(),
            handoff_summary: format!("{outline_id} handoff"),
            verification_plan: serde_json::json!({}),
        },
        status: WorkItemDraftStatus::Accepted,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "author_run_0001".to_string(),
        accepted_at: Some("2026-06-10T00:00:00Z".to_string()),
        superseded_at: None,
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
    }
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
