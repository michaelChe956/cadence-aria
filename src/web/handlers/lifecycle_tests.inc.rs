// lifecycle handler 测试（T4 分组视图 + T7 delivery_summary）：拆分到独立文件，
// 经 lifecycle.rs 的 `include!` 引入（large_file_guard 1200 行红线）。共享 `mod tests`
// 作用域内 `use super::*` 的导入。

use crate::product::advance_store::AdvanceTargetAttemptBinding;
use crate::product::coding_models::{
    RemoteKind, ReviewRequest, ReviewRequestKind, ReviewRequestOwnerKind,
};
use crate::product::coding_plan_run::{
    CodingPlanRun, CodingPlanRunPhase, CodingPlanRunStore, CodingPlanSelection,
    PLAN_CODING_MODE_FIRST_READY_UNIT,
};
use crate::product::issue_store::CreateProductIssueInput;
use crate::product::lifecycle_store::{
    CreateDesignSpecInput, CreateStorySpecInput, CreateWorkItemInput,
};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
    RepositorySourceIdentity, RepositoryType,
};
use crate::product::models::{LifecycleConfirmationStatus, RepositoryRecord, WorkItemPlanLineage};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::app::build_web_router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const REPOSITORY_ID: &str = "repo-1";

/// 建 issue + 3 个 work item：wi-a/wi-b 带不同 target_repository_id（有 logical codebase
/// 权威记录支撑），wi-c 无 target。
fn seed_issue_and_work_items(lifecycle: &LifecycleStore, paths: &ProductAppPaths) {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "lifecycle test project".to_string(),
            description: None,
        })
        .unwrap();
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(REPOSITORY_ID.to_string()),
            logical_codebase_id: None,
            title: "分组视图测试".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
    assert_eq!(issue.id, ISSUE_ID);

    let logical_a = LogicalRepositoryId(Uuid::new_v4());
    let logical_b = LogicalRepositoryId(Uuid::new_v4());
    seed_logical_codebase(
        paths,
        &[(logical_a, "checkout-wi-a"), (logical_b, "checkout-wi-b")],
    );

    for (index, (id, target)) in [
        ("wi-a", Some(logical_a)),
        ("wi-b", Some(logical_b)),
        ("wi-c", None),
    ]
    .into_iter()
    .enumerate()
    {
        let work_item = lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(id.to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                title: format!("工作项 {index}"),
                kind: WorkItemKind::Backend,
                plan_status: WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(work_item.id, id);
        set_target_repository_id(lifecycle, id, target);
    }
}

/// 播种 logical codebase 权威记录（manifest + selection + member + checkout + repos.json
/// 兼容投影），使 `resolve_logical_repository_strict` / `PlanningContextSetResolver` 都能解析。
/// 成员 alias 与 repository.name 统一为 `REPOSITORY_ID`，保持分组视图 alias 断言不变。
fn seed_logical_codebase(paths: &ProductAppPaths, members: &[(LogicalRepositoryId, &str)]) {
    let manifest = LogicalCodebaseManifest::new(
        PROJECT_ID,
        paths.root().join("aggregate-root"),
        members.iter().map(|(id, _)| *id).collect(),
    );
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest(PROJECT_ID, &manifest)
        .unwrap();
    let now = "2026-08-13T00:00:00Z".to_string();
    let mut repositories = Vec::new();
    for (index, (logical_id, checkout_name)) in members.iter().enumerate() {
        let physical_repository_id = format!("physical-{checkout_name}");
        let checkout_path = paths.root().join(checkout_name);
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &checkout_path,
            checkout_path.join(".git"),
            None,
        );
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        LogicalCodebaseStore::new(paths.clone())
            .save_member(
                PROJECT_ID,
                &CodebaseMemberRecord {
                    logical_repository_id: *logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    alias: REPOSITORY_ID.to_string(),
                    role: "repository".to_string(),
                    ordinal: (index + 1) as u32,
                    source_identity: source_identity.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        LogicalCodebaseStore::new(paths.clone())
            .save_checkout(
                PROJECT_ID,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: *logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    kind: CheckoutKind::Main,
                    canonical_path: checkout_path.clone(),
                    checkout_path_hash: format!("sha256:{checkout_name}"),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        repositories.push(RepositoryRecord {
            id: physical_repository_id,
            project_id: PROJECT_ID.to_string(),
            name: REPOSITORY_ID.to_string(),
            path: checkout_path,
            repo_hash: format!("sha256:{checkout_name}"),
            runtime_root: paths.root().join(checkout_name).join(".aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: now.clone(),
            logical_repository_id: Some(*logical_id),
            primary_checkout_id: Some(checkout_id),
            identity_schema_version: 1,
            updated_at: now.clone(),
        });
    }
    crate::product::json_store::write_json(
        &paths.project_root(PROJECT_ID).join("repos.json"),
        &repositories,
    )
    .unwrap();
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            PROJECT_ID,
            ISSUE_ID,
            members.iter().map(|(id, _)| *id).collect(),
            Vec::new(),
            Vec::new(),
            None,
        ))
        .unwrap();
}

fn seed_completed_attempt(
    store: &CodingAttemptStore,
    work_item_id: &str,
    branch_name: &str,
    commit_sha: &str,
) -> CodingExecutionAttempt {
    let created = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: branch_name.to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .unwrap();
    let attempt = CodingExecutionAttempt {
        status: CodingAttemptStatus::Completed,
        head_commit: Some(commit_sha.to_string()),
        ..created
    };
    store.write_coding_attempt_for_test(&attempt).unwrap();
    attempt
}

fn seed_review_request(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    push_status: PushStatus,
    push_error: Option<String>,
) {
    let request = ReviewRequest {
        id: format!("review_request_{}", Uuid::new_v4().simple()),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: attempt.branch_name.clone(),
        commit_sha: attempt.head_commit.clone().unwrap_or_default(),
        push_status,
        external_url: None,
        manual_instructions: Vec::new(),
        push_error,
        owner_kind: ReviewRequestOwnerKind::Attempt,
        pointer_publication_id: None,
        revoked: false,
        created_at: "2026-08-13T00:00:00Z".to_string(),
        updated_at: "2026-08-13T00:00:00Z".to_string(),
    };
    store.save_review_request(attempt, &request).unwrap();
}

/// create_work_item 恒置 target_repository_id=None，测试直接改写落盘 JSON。
fn set_target_repository_id(
    lifecycle: &LifecycleStore,
    work_item_id: &str,
    target: Option<LogicalRepositoryId>,
) {
    let path = lifecycle
        .work_items_root(PROJECT_ID, ISSUE_ID)
        .join(format!("{work_item_id}.json"));
    let mut record: LifecycleWorkItemRecord = crate::product::json_store::read_json(&path).unwrap();
    record.target_repository_id = target;
    crate::product::json_store::write_json(&path, &record).unwrap();
}

fn build_test_router(root: &std::path::Path) -> axum::Router {
    build_web_router(WebAppState::new(
        root.to_path_buf(),
        WebRuntime::new_fake(root.to_path_buf()),
    ))
}

async fn get_issue_lifecycle(app: &axum::Router) -> axum::http::Response<Body> {
    let uri = format!("/api/issues/{ISSUE_ID}/lifecycle?project_id={PROJECT_ID}");
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[test]
fn single_candidate_preflight_is_deterministic_and_only_allows_one_repository() {
    assert_eq!(
        preflight_single_repository_candidate(&["logical-repository-1".to_string()]),
        SingleCandidatePreflightDecision::Eligible {
            repository_id: "logical-repository-1".to_string(),
        }
    );

    for repository_ids in [
        Vec::<String>::new(),
        vec![
            "logical-repository-1".to_string(),
            "logical-repository-2".to_string(),
        ],
    ] {
        // L2 重钉（T5/REQ-WSC-08）：legacy fallback 变体已删除——preflight 失败
        // 输出新路径终态原因（Ineligible），无回落语义。
        assert_eq!(
            preflight_single_repository_candidate(&repository_ids),
            SingleCandidatePreflightDecision::Ineligible {
                reason: format!(
                    "single-candidate preflight requires exactly one logical repository; found {}",
                    repository_ids.len()
                ),
            }
        );
    }
}

fn seed_prepare_work_item_plan_fixture(
    paths: &ProductAppPaths,
    repository_count: usize,
) -> (LifecycleStore, String, String) {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "preflight project".to_string(),
            description: None,
        })
        .unwrap();
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(REPOSITORY_ID.to_string()),
            logical_codebase_id: None,
            title: "preflight issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
    let repository_names = ["checkout-preflight-a", "checkout-preflight-b"];
    let members = repository_names
        .into_iter()
        .take(repository_count)
        .enumerate()
        .map(|(index, name)| {
            (
                LogicalRepositoryId(Uuid::from_u128((index + 1) as u128)),
                name,
            )
        })
        .collect::<Vec<_>>();
    seed_logical_codebase(paths, &members);

    let lifecycle = LifecycleStore::new(paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "preflight story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    lifecycle
        .update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .unwrap();
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "preflight design".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    lifecycle
        .update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &design.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .unwrap();
    (lifecycle, story.id, design.id)
}

/// 播种普通单物理仓项目：没有 logical codebase manifest/selection，issue 只通过
/// 唯一的 repo_id 归属物理 RepositoryRecord。
fn seed_single_physical_prepare_work_item_plan_fixture(
    paths: &ProductAppPaths,
) -> (LifecycleStore, String, String) {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "single physical preflight project".to_string(),
            description: None,
        })
        .unwrap();
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(REPOSITORY_ID.to_string()),
            logical_codebase_id: None,
            title: "single physical preflight issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
    let now = "2026-08-27T00:00:00Z".to_string();
    crate::product::json_store::write_json(
        &paths.project_root(PROJECT_ID).join("repos.json"),
        &vec![RepositoryRecord {
            id: REPOSITORY_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            name: "single physical repository".to_string(),
            path: paths.root().join("single-physical-repository"),
            repo_hash: "sha256:single-physical-repository".to_string(),
            runtime_root: paths
                .root()
                .join("single-physical-repository/.aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: now.clone(),
            logical_repository_id: None,
            primary_checkout_id: None,
            identity_schema_version: 0,
            updated_at: now,
        }],
    )
    .unwrap();

    let lifecycle = LifecycleStore::new(paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "single physical preflight story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    lifecycle
        .update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .unwrap();
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "single physical preflight design".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    lifecycle
        .update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &design.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .unwrap();
    (lifecycle, story.id, design.id)
}

async fn post_prepare_work_item_plan(
    app: &axum::Router,
    story_spec_id: String,
    design_spec_id: String,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/work-item-plans:prepare"
                ))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "title": "preflight plan",
                        "story_spec_ids": [story_spec_id],
                        "design_spec_ids": [design_spec_id],
                        "author_provider": "claude_code",
                        "reviewer_provider": "codex",
                        "review_rounds": 1,
                        "superpowers_enabled": false,
                        "openspec_enabled": false,
                        "include_integration_tests": true,
                        "include_e2e_tests": false,
                        "force_frontend_backend_split": false,
                        "require_execution_plan_confirm": false
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

fn data_root_files(root: &std::path::Path) -> std::collections::BTreeSet<String> {
    fn collect(
        root: &std::path::Path,
        current: &std::path::Path,
        files: &mut std::collections::BTreeSet<String>,
    ) {
        let entries = match std::fs::read_dir(current) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("read {}: {error}", current.display()),
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, files);
            } else {
                files.insert(path.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }

    let mut files = std::collections::BTreeSet::new();
    collect(root, root, &mut files);
    files
}

#[tokio::test]
async fn prepare_single_physical_repository_uses_rollout_flow_snapshot_without_logical_manifest() {
    // L2 重钉（T5/REQ-WSC-08）：新会话一律 SingleCandidate——rollout 快照
    // 不再切换 flow，也不门控 preflight（fix round 1 后 flag 仅随会话
    // 持久化作历史记录，无行为分支）。
    for rollout_enabled in [true, false] {
        let expected_flow_kind = WorkItemPlanFlowKind::SingleCandidate;
        let root = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let (lifecycle, story_spec_id, design_spec_id) =
            seed_single_physical_prepare_work_item_plan_fixture(&paths);
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        )
        .with_work_item_plan_single_candidate(rollout_enabled);
        let app = build_web_router(state);

        let response = post_prepare_work_item_plan(&app, story_spec_id, design_spec_id).await;

        assert_eq!(response.status(), StatusCode::OK);
        let sessions = lifecycle
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].flow_kind, expected_flow_kind,
            "rollout_enabled={rollout_enabled}"
        );
    }
}

#[tokio::test]
async fn prepare_is_single_candidate_even_when_rollout_flag_is_disabled() {
    let root = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let (lifecycle, story_spec_id, design_spec_id) = seed_prepare_work_item_plan_fixture(&paths, 1);
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let _response = post_prepare_work_item_plan(&app, story_spec_id, design_spec_id).await;

    // L2 重钉（T5/REQ-WSC-08）：rollout flag 不再把新会话切到 legacy——
    // flow_kind 恒 SingleCandidate（该测试原断言 Legacy 随 legacy flow 删除退役）。
    let sessions = lifecycle
        .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].flow_kind, WorkItemPlanFlowKind::SingleCandidate);
}

#[tokio::test]
async fn prepare_preflight_failure_converges_to_new_path_terminal_without_legacy_fallback() {
    for repository_count in [0, 2] {
        let root = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let (lifecycle, story_spec_id, design_spec_id) =
            seed_prepare_work_item_plan_fixture(&paths, repository_count);
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        )
        .with_work_item_plan_single_candidate(true);
        let app = build_web_router(state);
        let before_files = data_root_files(paths.root());

        let response = post_prepare_work_item_plan(&app, story_spec_id, design_spec_id).await;
        assert!(response.status().is_server_error());
        drop(response);
        // L2 重钉（T5/REQ-WSC-08）：preflight 失败=新路径 durable Failed 终态
        //（含原因），无 legacy 回落、无 flow_kind 切换。
        let sessions = lifecycle
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].flow_kind, WorkItemPlanFlowKind::SingleCandidate);
        assert_eq!(sessions[0].status, WorkspaceSessionStatus::Failed);
        assert!(sessions[0].messages.iter().any(|message| {
            message
                .content
                .contains("single-candidate preflight requires exactly one logical repository")
        }));
        assert!(sessions[0].provider_start_ledger.is_empty());
        assert!(sessions[0].work_item_plan_source_revision_ref.is_none());
        assert!(sessions[0].plan_candidate_ir_ref.is_none());
        assert!(sessions[0].mechanical_report_ref.is_none());
        let after_files = data_root_files(paths.root());
        let new_files = after_files.difference(&before_files).collect::<Vec<_>>();
        assert!(
            new_files.iter().all(|path| {
                !path.contains("work-item-plan-sources/")
                    && !path.contains("source_revision/")
                    && !path.contains("plan_candidate_ir/")
                    && !path.contains("mechanical_report/")
                    && !path.contains("work_item_plan_compiles")
            }),
            "preflight fallback must not create single-candidate artifacts: {new_files:?}"
        );
    }

    let root = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let (lifecycle, story_spec_id, design_spec_id) = seed_prepare_work_item_plan_fixture(&paths, 1);
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    )
    .with_work_item_plan_single_candidate(true);
    let app = build_web_router(state);

    let response = post_prepare_work_item_plan(&app, story_spec_id, design_spec_id).await;
    assert!(response.status().is_server_error());
    let sessions = lifecycle
        .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].flow_kind,
        WorkItemPlanFlowKind::SingleCandidate,
        "a post-session error must not silently recreate a legacy session"
    );
    assert_eq!(sessions[0].status, WorkspaceSessionStatus::Failed);
    assert!(
        sessions[0]
            .messages
            .iter()
            .any(|message| message.content.contains("single-candidate prepare failed"))
    );
    assert!(sessions[0].provider_start_ledger.is_empty());
    assert!(sessions[0].work_item_plan_source_revision_ref.is_none());
    assert!(sessions[0].plan_candidate_ir_ref.is_none());
    assert!(sessions[0].mechanical_report_ref.is_none());
}

#[tokio::test]
async fn prepare_preflight_failure_converges_to_new_path_terminal_even_when_rollout_flag_is_disabled()
 {
    // fix round 1（k3 F1/REQ-WSC-08）：preflight 恒评估——flag off（CLI
    // 默认态）下多仓 logical Issue 同样在 prepare 期收敛 durable Failed
    // 终态（含原因），而非生成期 TargetAmbiguous/TargetMissing。
    let root = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let (lifecycle, story_spec_id, design_spec_id) = seed_prepare_work_item_plan_fixture(&paths, 2);
    // 默认 WebAppState：work_item_plan_single_candidate=false（flag off）。
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let response = post_prepare_work_item_plan(&app, story_spec_id, design_spec_id).await;
    assert!(response.status().is_server_error());
    drop(response);
    let sessions = lifecycle
        .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].flow_kind, WorkItemPlanFlowKind::SingleCandidate);
    assert_eq!(sessions[0].status, WorkspaceSessionStatus::Failed);
    assert!(sessions[0].messages.iter().any(|message| {
        message
            .content
            .contains("single-candidate preflight requires exactly one logical repository")
    }));
}

#[tokio::test]
async fn grouped_view_returns_work_item_repository_groups_with_dto_shape() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    seed_issue_and_work_items(&lifecycle, &paths);
    let app = build_test_router(temp.path());

    let response = get_issue_lifecycle(&app).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // 扁平 work_items 与分组视图同源（都来自已持久化记录）。
    let flat = value["work_items"].as_array().expect("work_items");
    assert_eq!(flat.len(), 3);
    let groups = value["work_item_repository_groups"]
        .as_array()
        .expect("work_item_repository_groups");
    assert_eq!(groups.len(), 3, "target A / target B / 未指定仓库 三组");

    // 分组 DTO 形状：target_repository_id / alias / status / compatibility_projection / items。
    for group in groups {
        let obj = group.as_object().expect("group object");
        for field in [
            "target_repository_id",
            "alias",
            "status",
            "compatibility_projection",
            "items",
        ] {
            assert!(obj.contains_key(field), "分组缺少字段 {field}");
        }
        let items = obj["items"].as_array().expect("items array");
        assert!(!items.is_empty(), "分组 items 不应为空");
        for item in items {
            assert!(item.get("work_item_id").is_some(), "item 缺 work_item_id");
            assert!(item.get("title").is_some(), "item 缺 title");
        }
    }

    // 未指定仓库组 compatibility_projection = true 且恒置末。
    let unassigned = groups.last().expect("unassigned group");
    assert!(
        unassigned["compatibility_projection"]
            .as_bool()
            .expect("compatibility_projection")
    );
    assert!(unassigned["target_repository_id"].is_null());

    // 指定仓库组：target_repository_id 为 UUID 字符串，alias 回落到物理投影名 repo-1。
    let assigned: Vec<_> = groups
        .iter()
        .filter(|g| !g["compatibility_projection"].as_bool().unwrap())
        .collect();
    assert_eq!(assigned.len(), 2);
    for group in assigned {
        assert!(group["target_repository_id"].is_string());
        assert_eq!(group["alias"].as_str(), Some(REPOSITORY_ID));
    }
}

#[tokio::test]
async fn issue_lifecycle_returns_delivery_summary_with_partial_entries() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    seed_issue_and_work_items(&lifecycle, &paths);

    // 多仓 partial 场景：wi-a 已推（Pushed）、wi-b push 失败（Failed）、wi-c 无 attempt。
    let coding_store = CodingAttemptStore::new(paths.clone());
    let attempt_a = seed_completed_attempt(&coding_store, "wi-a", "aria/wi-a/attempt-1", "sha111");
    seed_review_request(&coding_store, &attempt_a, PushStatus::Pushed, None);
    let attempt_b = seed_completed_attempt(&coding_store, "wi-b", "aria/wi-b/attempt-1", "sha222");
    seed_review_request(
        &coding_store,
        &attempt_b,
        PushStatus::Failed,
        Some("push rejected".to_string()),
    );

    let app = build_test_router(temp.path());
    let response = get_issue_lifecycle(&app).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let summary = &value["delivery_summary"];
    assert_eq!(summary["project_id"].as_str(), Some(PROJECT_ID));
    assert_eq!(summary["issue_id"].as_str(), Some(ISSUE_ID));
    assert_eq!(summary["overall"].as_str(), Some("partial"));

    let entries = summary["entries"].as_array().expect("delivery entries");
    assert_eq!(entries.len(), 3);

    let entry = |work_item_id: &str| -> &serde_json::Value {
        entries
            .iter()
            .find(|entry| entry["work_item_id"].as_str() == Some(work_item_id))
            .unwrap_or_else(|| panic!("missing delivery entry for {work_item_id}"))
    };

    let wi_a = entry("wi-a");
    assert_eq!(wi_a["repository_name"].as_str(), Some("checkout-wi-a"));
    assert_eq!(wi_a["attempt_status"].as_str(), Some("completed"));
    assert_eq!(wi_a["push_status"].as_str(), Some("pushed"));
    assert!(wi_a["push_error"].is_null());

    let wi_b = entry("wi-b");
    assert_eq!(wi_b["repository_name"].as_str(), Some("checkout-wi-b"));
    assert_eq!(wi_b["attempt_status"].as_str(), Some("completed"));
    assert_eq!(wi_b["push_status"].as_str(), Some("failed"));
    assert_eq!(wi_b["push_error"].as_str(), Some("push rejected"));

    let wi_c = entry("wi-c");
    assert!(wi_c["attempt_status"].is_null());
    assert!(wi_c["push_status"].is_null());
    assert!(wi_c["push_error"].is_null());
}

#[tokio::test]
async fn grouped_view_propagates_list_work_items_error_instead_of_silent_empty_groups() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    seed_issue_and_work_items(&lifecycle, &paths);

    // 构造 schema-v2 plan lineage：让 legacy 路径跳过 list_work_items，
    // 从而使分组视图成为唯一调用 list_work_items 的地方——失败必须传播为 handler 错误，
    // 而不是静默返回空 work_item_repository_groups（回归 Task10 fix）。
    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("plan-1".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec!["wi-a".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .unwrap();
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    revision_store
        .put_plan_lineage(&WorkItemPlanLineage {
            id: plan.id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_refs: Vec::new(),
            design_spec_refs: Vec::new(),
            active_revision_id: None,
            active_amendment_id: None,
            created_at: "2026-08-09T00:00:00Z".to_string(),
            updated_at: "2026-08-09T00:00:00Z".to_string(),
        })
        .unwrap();

    // 破坏 work_items 数据文件：list_work_items（分组路径）必然失败。
    let corrupt_path = lifecycle
        .work_items_root(PROJECT_ID, ISSUE_ID)
        .join("wi-a.json");
    std::fs::write(&corrupt_path, "{ not valid json").unwrap();

    let app = build_test_router(temp.path());
    let response = get_issue_lifecycle(&app).await;
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "list_work_items 失败必须传播为 500，而不是静默返回空分组"
    );
}

/// REQ-MTG-04（WP3）：issue_lifecycle 的 plan DTO additive 携带
/// group_projection——per-target 投影 + 三值终态（"partial" 不伪装全局成功）。
#[tokio::test]
async fn issue_lifecycle_returns_plan_group_projection_with_three_valued_overall() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "group projection test project".to_string(),
            description: None,
        })
        .unwrap();
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(REPOSITORY_ID.to_string()),
            logical_codebase_id: None,
            title: "聚合投影测试".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
    let alpha = LogicalRepositoryId(Uuid::new_v4());
    let beta = LogicalRepositoryId(Uuid::new_v4());
    seed_logical_codebase(
        &paths,
        &[(alpha, "checkout-alpha"), (beta, "checkout-beta")],
    );
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("plan-projection-1".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec!["wi-a".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .unwrap();

    let target_snapshot =
        |logical_id: LogicalRepositoryId| crate::product::coding_models::AttemptTargetSnapshot {
            logical_repository_id: logical_id,
            checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
            physical_repository_id: "physical-projection".to_string(),
            canonical_path: temp.path().join("checkout-projection"),
            git_dir_identity: "projection-git-dir".to_string(),
            revision: Some("projection-revision".to_string()),
            policy_digest: "projection-policy".to_string(),
            membership_revision: 1,
            captured_at: "2026-09-19T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        };
    let group_input = |plan_id: &str, target: Option<LogicalRepositoryId>, branch_name: &str| {
        crate::product::coding_attempt_store::CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: plan_id.to_string(),
            current_work_item_id: "wi-a".to_string(),
            base_branch: "main".to_string(),
            branch_name: branch_name.to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: target.map(target_snapshot),
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        }
    };

    let coding_store = CodingAttemptStore::new(paths.clone());
    let delivered = coding_store
        .create_group_attempt(group_input(
            "plan-projection-1",
            Some(alpha),
            "aria/issues/issue_0001/checkout-alpha",
        ))
        .unwrap();
    let delivered = CodingExecutionAttempt {
        status: CodingAttemptStatus::Completed,
        head_commit: Some("sha111".to_string()),
        ..delivered
    };
    coding_store
        .write_coding_attempt_for_test(&delivered)
        .unwrap();
    seed_review_request(&coding_store, &delivered, PushStatus::Pushed, None);
    // beta target 停留初始二元组 (Created, PrepareContext)——部分交付+部分未启 → Partial。
    coding_store
        .create_group_attempt(group_input(
            "plan-projection-1",
            Some(beta),
            "aria/issues/issue_0001/checkout-beta",
        ))
        .unwrap();

    let app = build_test_router(temp.path());
    let response = get_issue_lifecycle(&app).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let plans = value["work_item_plans"]
        .as_array()
        .expect("work_item_plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0]["id"].as_str(), Some("plan-projection-1"));
    let projection = &plans[0]["group_projection"];
    assert_eq!(projection["plan_id"].as_str(), Some("plan-projection-1"));
    assert_eq!(projection["overall"].as_str(), Some("partial"));

    let entries = projection["entries"]
        .as_array()
        .expect("projection entries");
    assert_eq!(entries.len(), 2);
    let entry = |name: &str| -> &serde_json::Value {
        entries
            .iter()
            .find(|entry| entry["repository_name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("missing projection entry for {name}"))
    };
    let delivered_entry = entry("checkout-alpha");
    assert_eq!(
        delivered_entry["attempt_status"].as_str(),
        Some("completed")
    );
    assert_eq!(delivered_entry["push_status"].as_str(), Some("pushed"));
    assert_eq!(
        delivered_entry["branch_name"].as_str(),
        Some("aria/issues/issue_0001/checkout-alpha")
    );
    assert_eq!(delivered_entry["head_commit"].as_str(), Some("sha111"));
    assert!(delivered_entry["review_request_id"].is_string());
    assert!(delivered_entry["blocked_reason"].is_null());

    let unstarted_entry = entry("checkout-beta");
    assert_eq!(unstarted_entry["attempt_status"].as_str(), Some("created"));
    assert_eq!(unstarted_entry["stage"].as_str(), Some("prepare_context"));
    assert!(unstarted_entry["push_status"].is_null());
    assert!(unstarted_entry["blocked_reason"].is_null());
}

/// P1 WIGA Task 3：构造与人工 prepare 同形的请求（逐项填写
/// `src/web/types.rs` 的 PrepareWorkItemPlanRequest 字段）。
fn plan_prepare_request(story_id: String, design_id: String) -> PrepareWorkItemPlanRequest {
    PrepareWorkItemPlanRequest {
        title: "preflight design Work Item".to_string(),
        story_spec_ids: vec![story_id],
        design_spec_ids: vec![design_id],
        author_provider: Some("fake".to_string()),
        reviewer_provider: Some("fake".to_string()),
        review_rounds: Some(1),
        superpowers_enabled: Some(false),
        openspec_enabled: Some(false),
        run_policy: Some(RunPolicy::Interactive),
        include_integration_tests: Some(true),
        include_e2e_tests: Some(false),
        force_frontend_backend_split: Some(false),
        require_execution_plan_confirm: Some(false),
    }
}

#[test]
fn prepare_bound_plan_reuses_identity_without_overwriting_content() {
    let root = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let (lifecycle, story_id, design_id) = seed_prepare_work_item_plan_fixture(&paths, 1);
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let request = plan_prepare_request(story_id, design_id);
    let ids = PreparedPlanIds {
        plan_id: "issue_work_item_plan_auto_fixture".into(),
        session_id: "workspace_session_auto_fixture".into(),
    };
    let first = prepare_plan_records(
        &state,
        PROJECT_ID,
        ISSUE_ID,
        request.clone(),
        Some(ids.clone()),
    )
    .unwrap();
    assert_eq!(first.plan.id, "issue_work_item_plan_auto_fixture");
    assert_eq!(first.session.id, "workspace_session_auto_fixture");
    assert_eq!(first.session.run_policy, RunPolicy::Interactive);

    // 同稳定 ID 但请求身份漂移（options 变化）→ IdentityMismatch 拒绝，不覆盖。
    let mut changed = request.clone();
    changed.include_e2e_tests = Some(true);
    assert!(
        prepare_plan_records(&state, PROJECT_ID, ISSUE_ID, changed, Some(ids.clone())).is_err()
    );

    // 原请求重试：复用同一 plan/session，创建时间不变，plan 总数仍为 1。
    let repeated = prepare_plan_records(&state, PROJECT_ID, ISSUE_ID, request, Some(ids)).unwrap();
    assert_eq!(repeated.plan.id, first.plan.id);
    assert_eq!(repeated.plan.created_at, first.plan.created_at);
    assert_eq!(repeated.session.id, first.session.id);
    assert_eq!(repeated.session.created_at, first.session.created_at);
    assert_eq!(
        lifecycle
            .list_issue_work_item_plans(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        lifecycle
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .len(),
        1
    );
}

/// P1 WIGA Task 8（REQ-WIGA-07）：issue lifecycle HTTP 投影只读携带
/// `plan_confirmed_info`——未确认时空数组；finalizer 崩溃经人工
/// CompileRecovery Continue + Approve 确认后有且仅有一条，重复 GET
/// 同 key 同 `committed_at` 时间（幂等，不取会变的 updated_at）。
#[tokio::test]
async fn issue_lifecycle_projects_plan_confirmed_info_with_stable_key_and_time() {
    let mut fixture = Box::new(crate::web::wiga_gate_fixture::EnrolledGateFixture::new().await);
    let app = crate::web::app::build_web_router(fixture.state.clone());

    let fetch_info = |app: axum::Router| async move {
        let uri = format!("/api/issues/{ISSUE_ID}/lifecycle?project_id={PROJECT_ID}");
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        value["plan_confirmed_info"]
            .as_array()
            .expect("plan_confirmed_info array (serde default)")
            .clone()
    };

    // 未编译/未确认：additive 字段存在且为空。
    let before = fetch_info(app.clone()).await;
    assert!(before.is_empty(), "未确认不得投影成功信息: {before:?}");

    fixture.fail_compile_after_human_approve().await;
    let during = fetch_info(app.clone()).await;
    assert!(
        during.is_empty(),
        "failpoint 失败 compile 不得投影成功信息: {during:?}"
    );

    fixture.recover_and_confirm_compile().await;
    let first = fetch_info(app.clone()).await;
    assert_eq!(first.len(), 1, "恢复并确认后有且仅有一条: {first:?}");
    let info = &first[0];
    assert_eq!(info["title"].as_str(), Some("Work Item Plan 已确认"));
    assert_eq!(
        info["session_id"].as_str(),
        Some(fixture.session_id.as_str())
    );
    let key = info["key"].as_str().expect("key");
    assert!(key.starts_with("plan_confirmed:"), "{key}");
    assert!(key.contains(&fixture.durable().entity_id), "{key}");
    assert!(
        !info["occurred_at"].as_str().unwrap_or_default().is_empty(),
        "occurred_at 取 tx.committed_at"
    );

    // 重复 GET：同 key 同时间（幂等投影，不随读取变化）。
    let second = fetch_info(app).await;
    assert_eq!(first, second, "重复 GET 必须得到相同投影");
}

// ───────────────────────────────────────────────────────────────
// N7（REQ-CS-07）：新 plan-run 删除闭包接线测试
// ───────────────────────────────────────────────────────────────

const N7_PLAN_A: &str = "work_item_plan_0001";
const N7_PLAN_B: &str = "work_item_plan_0002";

/// 最小 project + issue 种子（N7 只关心 plan 删除闭包，不建 work item）。
fn n7_seed_project_issue(paths: &ProductAppPaths) -> LifecycleStore {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "n7 deletion closure project".to_string(),
            description: None,
        })
        .expect("create project");
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(REPOSITORY_ID.to_string()),
            logical_codebase_id: None,
            title: "N7 删除闭包".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("create issue");
    assert_eq!(issue.id, ISSUE_ID);
    LifecycleStore::new(paths.clone())
}

fn n7_seed_plan(lifecycle: &LifecycleStore, plan_id: &str) {
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(plan_id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("seed plan");
}

/// group 绑定 attempt（work_item_group_id=plan）：plan 级门禁按它拒删。
fn n7_group_attempt(coding_store: &CodingAttemptStore, plan_id: &str) -> CodingExecutionAttempt {
    let created = coding_store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let mut attempt = created;
    attempt.work_item_group_id = Some(plan_id.to_string());
    coding_store
        .write_coding_attempt_for_test(&attempt)
        .expect("bind attempt to plan");
    attempt
}

/// 直写冻结地址 `{issue_root}/coding-plan-runs/{plan_id}.json` 的 run 记录。
fn n7_plan_run(
    plan_id: &str,
    targets: &[(LogicalRepositoryId, String)],
    selection_attempt: Option<&str>,
    phase: CodingPlanRunPhase,
) -> CodingPlanRun {
    let selection = selection_attempt.map(|attempt_id| CodingPlanSelection {
        unit_id: "unit_0001".to_string(),
        logical_work_item_id: "logical_work_item_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        target_repository_id: targets
            .first()
            .map(|(repository, _)| repository.0.to_string())
            .unwrap_or_default(),
        start_command_id: "plan_start_deterministic_0001".to_string(),
    });
    CodingPlanRun {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        command_id: "plan_command_0001".to_string(),
        bound_plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "graph_revision_0001".to_string(),
        target_attempts: targets
            .iter()
            .map(|(repository, attempt_id)| AdvanceTargetAttemptBinding {
                target_repository_id: repository.0.to_string(),
                attempt_id: attempt_id.clone(),
            })
            .collect(),
        mode: PLAN_CODING_MODE_FIRST_READY_UNIT.to_string(),
        selection,
        phase,
        start_outcome: None,
        pause_reason: None,
        created_at: "2026-10-14T00:00:00Z".to_string(),
        updated_at: "2026-10-14T00:00:00Z".to_string(),
    }
}

fn n7_runs_root(paths: &ProductAppPaths) -> std::path::PathBuf {
    paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("coding-plan-runs")
}

fn n7_run_file(paths: &ProductAppPaths, plan_id: &str) -> std::path::PathBuf {
    n7_runs_root(paths).join(format!("{plan_id}.json"))
}

fn n7_run_lock(paths: &ProductAppPaths, plan_id: &str) -> std::path::PathBuf {
    n7_runs_root(paths).join(format!(".{plan_id}.json.lock"))
}

fn n7_write_run(paths: &ProductAppPaths, run: &CodingPlanRun) {
    std::fs::create_dir_all(n7_runs_root(paths)).expect("create runs root");
    crate::product::json_store::write_json(&n7_run_file(paths, &run.plan_id), run)
        .expect("seed plan run");
}

fn n7_repo_worktree_input(
    repository_id: LogicalRepositoryId,
    worktree_path: &str,
) -> UpsertRepoSharedWorktreeInput {
    UpsertRepoSharedWorktreeInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        repository_id,
        branch_name: format!("worktree-{}", repository_id.0),
        worktree_path: std::path::PathBuf::from(worktree_path),
        base_branch: "main".to_string(),
    }
}

/// 直接改写仓维 worktree 记录，模拟另一 plan attempt 持有活动锁。
fn n7_set_repo_worktree_owner(
    paths: &ProductAppPaths,
    repository_id: LogicalRepositoryId,
    owner_attempt_id: &str,
) {
    let path = paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("shared-worktrees")
        .join(format!("{}.json", repository_id.0));
    let mut record: crate::product::models::IssueSharedWorktree =
        crate::product::json_store::read_json(&path).expect("read repo worktree");
    record.current_active_work_item_id = Some("work_item_0009".to_string());
    record.current_lock_owner_id = Some(owner_attempt_id.to_string());
    record.status = crate::product::models::IssueSharedWorktreeStatus::Running;
    crate::product::json_store::write_json(&path, &record).expect("seed other owner");
}

async fn delete_work_item_plan_request(
    app: &axum::Router,
    plan_id: &str,
) -> (StatusCode, serde_json::Value) {
    let uri = format!("/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/work-item-plans/{plan_id}");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("send delete plan");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read delete body");
    let value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// 有 coding workspace 时拒删 plan：run/plan/attempt 全部零改动（REQ-CS-07
/// 「先删 workspace 的门禁 MUST 保留」）。基线保护（门禁既有，防回归）。
#[tokio::test]
async fn plan_coding_n7_workspace_gate_zero_mutation() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = n7_seed_project_issue(&paths);
    n7_seed_plan(&lifecycle, N7_PLAN_A);
    let coding_store = CodingAttemptStore::new(paths.clone());
    let attempt = n7_group_attempt(&coding_store, N7_PLAN_A);
    let run = n7_plan_run(
        N7_PLAN_A,
        &[(LogicalRepositoryId(Uuid::new_v4()), attempt.id.clone())],
        Some(&attempt.id),
        CodingPlanRunPhase::StartRequested,
    );
    n7_write_run(&paths, &run);
    let run_before = std::fs::read(n7_run_file(&paths, N7_PLAN_A)).expect("run seeded");

    let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;

    assert_eq!(status, StatusCode::CONFLICT, "gate must reject: {body}");
    assert_eq!(body["code"], "coding_workspace_exists");
    // run/plan/attempt 零改动。
    let run_after = std::fs::read(n7_run_file(&paths, N7_PLAN_A)).expect("run untouched");
    assert_eq!(
        run_before, run_after,
        "拒绝路径不得动 run 记录（早拒不 purge/不失效）"
    );
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, N7_PLAN_A)
            .is_ok(),
        "plan record must remain after rejection"
    );
    assert!(
        coding_store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .is_ok(),
        "attempt record must remain after rejection"
    );
}

/// 先删本 workspaces 再删 plan（schema-v2 闭包）：本 plan run 文件+前导点锁清、
/// 冻结 target 资源精确清理；他 Plan run / 他 target / 他人持锁 owner 不误删。
#[tokio::test]
async fn plan_coding_n7_plan_purge_removes_run_and_preserves_other_owner() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = n7_seed_project_issue(&paths);
    n7_seed_plan(&lifecycle, N7_PLAN_A);
    n7_seed_plan(&lifecycle, N7_PLAN_B);
    // plan A 走 schema-v2 清理闭包。
    WorkItemRevisionStore::new(paths.clone())
        .put_plan_lineage(&WorkItemPlanLineage {
            id: N7_PLAN_A.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_refs: Vec::new(),
            design_spec_refs: Vec::new(),
            active_revision_id: None,
            active_amendment_id: None,
            created_at: "2026-10-14T00:00:00Z".to_string(),
            updated_at: "2026-10-14T00:00:00Z".to_string(),
        })
        .expect("seed plan lineage");

    let repo_a = LogicalRepositoryId(Uuid::new_v4());
    let repo_b = LogicalRepositoryId(Uuid::new_v4());
    let repo_c = LogicalRepositoryId(Uuid::new_v4());
    lifecycle
        .upsert_repo_shared_worktree(n7_repo_worktree_input(repo_a, "/tmp/n7-wt-a"))
        .expect("seed repo a worktree");
    lifecycle
        .upsert_repo_shared_worktree(n7_repo_worktree_input(repo_b, "/tmp/n7-wt-b"))
        .expect("seed repo b worktree");
    lifecycle
        .upsert_repo_shared_worktree(n7_repo_worktree_input(repo_c, "/tmp/n7-wt-c"))
        .expect("seed repo c worktree");
    // repo_c 在冻结集内但被其他 plan 的 attempt 持有活动锁（真实 owner 复核）。
    n7_set_repo_worktree_owner(&paths, repo_c, "other_plan_attempt_0001");

    let coding_store = CodingAttemptStore::new(paths.clone());
    let attempt_a = n7_group_attempt(&coding_store, N7_PLAN_A);
    let run_a = n7_plan_run(
        N7_PLAN_A,
        &[
            (repo_a, attempt_a.id.clone()),
            (repo_c, "attempt_c_of_plan_a".to_string()),
        ],
        None,
        CodingPlanRunPhase::ReadyBound,
    );
    n7_write_run(&paths, &run_a);
    let run_b = n7_plan_run(
        N7_PLAN_B,
        &[(repo_b, "attempt_b_of_plan_b".to_string())],
        None,
        CodingPlanRunPhase::Prepared,
    );
    n7_write_run(&paths, &run_b);
    // 前导点锁残留（安全删除窗口必须一并清）。
    std::fs::write(n7_run_lock(&paths, N7_PLAN_A), b"").expect("seed run lock");

    // 门禁顺序：先删本 run 绑定的 coding workspace，再删 plan（invalidate 是
    // run 写准入——必须在墓碑立起之前完成）。
    finalize_coding_attempt_deletion(&coding_store, &paths, &attempt_a)
        .expect("delete member workspace first");

    // N2 修复轮语义：purge 的锁清理只在删除窗口执行——Plan 墓碑在位
    //（真实删除闭包形态：D1.2c 级联持 lease begin 后重入本删除）时
    // `purge_plan` 才清前导点锁。
    let scope = crate::product::entity_deletion_store::EntityScope::new(
        PROJECT_ID,
        ISSUE_ID,
        crate::product::entity_deletion_store::DeletionEntityKind::Plan,
        N7_PLAN_A,
    );
    let lease = crate::product::entity_operation_store::EntityOperationStore::new(paths.clone())
        .try_acquire_delete(&scope)
        .expect("acquire plan delete lease");
    crate::product::entity_deletion_store::EntityDeletionStore::new(paths.clone())
        .begin(&lease)
        .expect("begin plan deletion tombstone");
    drop(lease);
    let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    assert_eq!(body["status"], "deleted");

    // 本 plan run 文件 + 前导点安全锁清。
    assert!(
        !n7_run_file(&paths, N7_PLAN_A).exists(),
        "plan run file must be purged with plan deletion"
    );
    assert!(
        !n7_run_lock(&paths, N7_PLAN_A).exists(),
        "plan run leading-dot lock must be purged in the safe deletion window"
    );
    // 他 plan run 保留（不整目录清空）。
    assert!(
        n7_run_file(&paths, N7_PLAN_B).exists(),
        "other plan run must be preserved"
    );
    let kept_run_b = CodingPlanRunStore::new(paths.clone())
        .get(PROJECT_ID, ISSUE_ID, N7_PLAN_B)
        .expect("load other plan run")
        .expect("other plan run persists");
    assert!(
        matches!(kept_run_b.phase, CodingPlanRunPhase::Prepared),
        "other plan run phase must be untouched"
    );
    // 冻结 target 内：无主 repo_a 清；他人持锁 repo_c 保留（复核真实 owner）。
    assert!(
        lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repo_a)
            .expect("read repo a")
            .is_none(),
        "frozen target without another owner must be cleaned"
    );
    assert!(
        lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repo_c)
            .expect("read repo c")
            .is_some(),
        "frozen target actively locked by another owner must be preserved"
    );
    // 非冻结集的 repo_b（他 target）不得被 issue-wide 扫掉。
    assert!(
        lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repo_b)
            .expect("read repo b")
            .is_some(),
        "targets outside the frozen set must not be swept by issue-wide cleanup"
    );
    // plan 记录：A 删、B 留。
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, N7_PLAN_A)
            .is_err()
    );
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, N7_PLAN_B)
            .is_ok(),
        "other plan record must be preserved"
    );
}

/// 缺失幂等 OK、IO/JSON 错误不吞、删除后迟到写不复活 run 目录（legacy 闭包）。
#[tokio::test]
async fn plan_coding_n7_missing_resource_reentry_and_late_start() {
    // A) 无 run 的 plan：删除成功（缺失 OK）；重入不复活任何 run 产物。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = n7_seed_project_issue(&paths);
        n7_seed_plan(&lifecycle, N7_PLAN_A);

        let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
        assert_eq!(status, StatusCode::OK, "missing run is OK: {body}");
        assert!(!n7_run_file(&paths, N7_PLAN_A).exists());

        let (reentry_status, reentry_body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
        assert_ne!(
            reentry_status,
            StatusCode::OK,
            "re-entry on deleted plan must not report success: {reentry_body}"
        );
        assert!(
            !n7_run_file(&paths, N7_PLAN_A).exists() && !n7_run_lock(&paths, N7_PLAN_A).exists(),
            "re-entry must not resurrect run artifacts"
        );
    }

    // B) run JSON 损坏：IO/JSON 错误不得被吞——删除失败上抛且 plan 记录保持。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = n7_seed_project_issue(&paths);
        n7_seed_plan(&lifecycle, N7_PLAN_A);
        std::fs::create_dir_all(n7_runs_root(&paths)).expect("runs root");
        std::fs::write(n7_run_file(&paths, N7_PLAN_A), b"{ not valid json")
            .expect("seed corrupt run");

        let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
        assert_ne!(
            status,
            StatusCode::OK,
            "corrupt run json must surface (never swallowed): {body}"
        );
        assert!(
            lifecycle
                .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, N7_PLAN_A)
                .is_ok(),
            "failed closure before mutation must keep the plan record"
        );
    }

    // C) 删除成功后：迟到写（旧 command 的首启标记）必须拒绝且不复活 run 文件。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = n7_seed_project_issue(&paths);
        n7_seed_plan(&lifecycle, N7_PLAN_A);
        let coding_store = CodingAttemptStore::new(paths.clone());
        let attempt = n7_group_attempt(&coding_store, N7_PLAN_A);
        let run = n7_plan_run(
            N7_PLAN_A,
            &[(LogicalRepositoryId(Uuid::new_v4()), attempt.id.clone())],
            Some(&attempt.id),
            CodingPlanRunPhase::ReadyBound,
        );
        n7_write_run(&paths, &run);
        // 残留前导点锁：无墓碑（非删除窗口）时 purge 只清 run 文件、保留锁
        //（N2 修复轮语义：锁 unlink 只在墓碑/lease 保护的安全窗口执行）。
        std::fs::write(n7_run_lock(&paths, N7_PLAN_A), b"").expect("seed run lock");
        let run_store = CodingPlanRunStore::new(paths.clone());

        finalize_coding_attempt_deletion(&coding_store, &paths, &attempt)
            .expect("delete member workspace");
        let stale = run_store
            .get(PROJECT_ID, ISSUE_ID, N7_PLAN_A)
            .expect("load run")
            .expect("run invalidated by member deletion");
        let (status, body) = delete_work_item_plan_request(&app, N7_PLAN_A).await;
        assert_eq!(status, StatusCode::OK, "delete body: {body}");
        assert!(!n7_run_file(&paths, N7_PLAN_A).exists());
        assert!(
            n7_run_lock(&paths, N7_PLAN_A).exists(),
            "without a deletion-window tombstone purge_plan keeps the lock file (read-only exit)"
        );

        let late = run_store.mark_start_requested(&stale);
        assert!(
            late.is_err(),
            "late write on purged run must be rejected, got {late:?}"
        );
        assert!(
            !n7_run_file(&paths, N7_PLAN_A).exists(),
            "late write must not resurrect the run file"
        );
    }
}
