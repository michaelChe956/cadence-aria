// D2.2（fix-cascade-deletion Task 3.2）：三类 DELETE 统一编排测试。经 lifecycle.rs
// 的 tests mod include!（large_file_guard 1200 行红线拆分，沿 lifecycle_tests_n7.inc.rs
// 先例）。PROJECT_ID/ISSUE_ID/REPOSITORY_ID、build_test_router、set_target_repository_id
// 与 N7 夹具（n7_plan_run/n7_write_run/n7_run_file/n7_run_lock/n7_repo_worktree_input）
// 复用同 mod 已有绑定。

use crate::cross_cutting::tool_policy_audit::{ProviderStartAudit, ToolPolicyAuditSink};
use crate::product::advance_store::{
    AdvanceInitializationJournal, AdvanceInitializationPhase, AdvanceRecord, AdvanceStatus,
    AdvanceStore,
};
use crate::product::entity_deletion_store::{
    DeletionPhase, EntityDeletionRecord, EntityDeletionStore, EntityScope,
};
use crate::product::entity_operation_store::{EntityOperationStore, SharedResourceKey};
use crate::product::models::automation::{OperationState, RetryInitializationRecord};
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateMechanicalReport, PlanCandidatePublicationProvenance,
};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    WorkItemPlanSourceStore,
};
use sha2::{Digest, Sha256};

const D2_2_PLAN_A: &str = "work_item_plan_0001";
const D2_2_PLAN_B: &str = "work_item_plan_0002";
const D2_2_COMPILER_VERSION: &str = "work-item-plan-compiler@d2_2";

fn d2_2_seed_project_issue(paths: &ProductAppPaths) -> LifecycleStore {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "d2_2 cascade deletion project".to_string(),
            description: None,
        })
        .expect("create project");
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(REPOSITORY_ID.to_string()),
            logical_codebase_id: None,
            title: "D2.2 级联删除".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("create issue");
    assert_eq!(issue.id, ISSUE_ID);
    LifecycleStore::new(paths.clone())
}

fn d2_2_seed_plan(
    lifecycle: &LifecycleStore,
    plan_id: &str,
    story_refs: Vec<String>,
    profile_ref: Option<String>,
    run_ref: Option<String>,
    work_item_ids: Vec<String>,
    status: crate::product::models::IssueWorkItemPlanStatus,
) {
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(plan_id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: story_refs,
            source_design_spec_ids: Vec::new(),
            options: crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status,
            work_item_ids,
            repository_profile_ref: profile_ref,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: run_ref,
            validator_findings: Vec::new(),
        })
        .expect("seed plan");
}

fn d2_2_wi(lifecycle: &LifecycleStore, id: &str, plan_id: &str) {
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "d2_2 wi".to_string(),
            work_item_set_id: None,
            source_work_item_plan_id: Some(plan_id.to_string()),
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            kind: crate::product::models::WorkItemKind::Backend,
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: Default::default(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            plan_status: crate::product::models::WorkItemPlanStatus::Draft,
        })
        .expect("seed work item");
}

fn d2_2_session(
    lifecycle: &LifecycleStore,
    entity_id: &str,
    workspace_type: WorkspaceType,
) -> crate::product::models::WorkspaceSessionRecord {
    lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: entity_id.to_string(),
            workspace_type,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("seed session")
}

/// 注册 REPOSITORY_ID 的物理仓库记录（独立 DELETE /work-items 路径的
/// `find_repository` 需要 repos.json 在档）。
fn d2_2_seed_repository(paths: &ProductAppPaths) {
    crate::product::json_store::write_json(
        &paths.project_root(PROJECT_ID).join("repos.json"),
        &[crate::product::models::RepositoryRecord {
            id: REPOSITORY_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            name: REPOSITORY_ID.to_string(),
            path: paths.root().join("repo-1"),
            repo_hash: "sha256:d2_2-repo".to_string(),
            runtime_root: paths.root().join("repo-1").join(".aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-10-14T00:00:00Z".to_string(),
            logical_repository_id: None,
            primary_checkout_id: None,
            identity_schema_version: 1,
            updated_at: "2026-10-14T00:00:00Z".to_string(),
        }],
    )
    .unwrap();
}

/// 真实 source store writer 落档（source/IR/机械报告/provenance），返回 plan 子树路径。
fn d2_2_source_objects(paths: &ProductAppPaths, plan_id: &str) -> std::path::PathBuf {
    fn source_hash(source: &str) -> String {
        format!("{:x}", Sha256::digest(source.as_bytes()))
    }
    let store = WorkItemPlanSourceStore::new(paths.clone());
    let source = "# Work Item Plan\n";
    let mut revision = SourceRevisionRecord {
        id: "source-001".to_string(),
        source: source.to_string(),
        source_revision_hash: source_hash(source),
        content_hash: String::new(),
    };
    revision.content_hash = revision.content_hash().unwrap();
    let source_ref = store
        .put_source_revision(PROJECT_ID, ISSUE_ID, plan_id, &revision)
        .unwrap();

    let mut ir = PlanCandidateIrRecord {
        id: "ir-001".to_string(),
        source_revision_id: revision.id.clone(),
        ir: PlanCandidateIr {
            source_revision_hash: revision.source_revision_hash.clone(),
            compiler_version: D2_2_COMPILER_VERSION.to_string(),
            items: Vec::new(),
        },
        content_hash: String::new(),
    };
    ir.content_hash = ir.content_hash().unwrap();
    let ir_ref = store
        .put_plan_candidate_ir(PROJECT_ID, ISSUE_ID, plan_id, &ir)
        .unwrap();

    let mut mechanical = PlanCandidateMechanicalReportRecord {
        id: "report-001".to_string(),
        source_revision_id: revision.id.clone(),
        ir_id: ir.id.clone(),
        report: PlanCandidateMechanicalReport {
            source_revision_hash: revision.source_revision_hash.clone(),
            compiler_version: D2_2_COMPILER_VERSION.to_string(),
            findings: Vec::new(),
        },
        structure_report: None,
        content_hash: String::new(),
    };
    mechanical.content_hash = mechanical.content_hash().unwrap();
    let report_ref = store
        .put_mechanical_report(PROJECT_ID, ISSUE_ID, plan_id, &mechanical)
        .unwrap();

    let mut provenance = PlanCandidatePublicationProvenance {
        id: "publication-001".to_string(),
        plan_id: plan_id.to_string(),
        plan_revision_id: "plan-revision-0001".to_string(),
        source_revision_ref: source_ref,
        plan_candidate_ir_ref: ir_ref,
        mechanical_report_ref: report_ref,
        source_revision_hash: revision.source_revision_hash.clone(),
        compiler_version: D2_2_COMPILER_VERSION.to_string(),
        published_at: "2026-10-14T00:00:00Z".to_string(),
        content_hash: String::new(),
    };
    provenance.content_hash = provenance.content_hash().unwrap();
    store
        .put_publication_provenance(PROJECT_ID, ISSUE_ID, plan_id, &provenance)
        .unwrap();

    paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("work-item-plan-sources")
        .join(plan_id)
}

/// advance record/初始化 journal/retry（D1.2c 真实接口；retry 沿 §0.5 固定路径）。
fn d2_2_advance_closure(paths: &ProductAppPaths, plan_id: &str) -> (String, String) {
    let advance = AdvanceStore::new(paths.clone());
    let record = AdvanceRecord {
        id: format!("advance_command_{plan_id}"),
        command_id: format!("command_{plan_id}"),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        plan_revision_id: "revision_0001".to_string(),
        attempt_id: Some("attempt_0001".to_string()),
        start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        target_attempts: Vec::new(),
        status: AdvanceStatus::Ready,
        workspace_entry: Some(format!("/workspaces/attempt_{plan_id}")),
        error: None,
        created_at: "2026-10-14T00:00:00Z".to_string(),
        updated_at: "2026-10-14T00:00:01Z".to_string(),
    };
    advance.put_record(&record).unwrap();
    advance
        .save_advance_initialization(
            &record,
            &AdvanceInitializationJournal {
                advance_id: record.id.clone(),
                plan_id: plan_id.to_string(),
                attempt_id: "attempt_0001".to_string(),
                target_attempt_ids: Vec::new(),
                phase: AdvanceInitializationPhase::JournalPrepared,
                error: None,
                created_at: "2026-10-14T00:00:00Z".to_string(),
                updated_at: "2026-10-14T00:00:01Z".to_string(),
            },
        )
        .unwrap();
    let retry_command_id = format!("retry_cmd_{plan_id}");
    crate::product::json_store::write_json(
        &paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("advance-retries")
            .join(format!("{retry_command_id}.json")),
        &RetryInitializationRecord {
            retry_id: format!("advance_retry_{retry_command_id}"),
            command_id: retry_command_id.clone(),
            advance_id: record.id.clone(),
            attempt_id: "attempt_0001".to_string(),
            state: OperationState::Accepted,
            checkpoint: AdvanceInitializationPhase::JournalPrepared,
            created_at: "2026-10-14T00:00:00Z".to_string(),
            updated_at: "2026-10-14T00:00:01Z".to_string(),
        },
    )
    .unwrap();
    (record.id, retry_command_id)
}

fn d2_2_review_for_session(
    lifecycle: &LifecycleStore,
    session_id: &str,
) -> crate::product::models::ProviderReviewRoundRecord {
    lifecycle
        .append_provider_review_round(
            crate::product::lifecycle_store::AppendProviderReviewRoundInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                session_id: session_id.to_string(),
                round_index: 1,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_result: "approved".to_string(),
                revision_result: "none".to_string(),
            },
        )
        .expect("seed review")
}

/// 业务字节快照：排除删除机制自身的稳定元数据（entity-operations/ 咨询锁与
/// entity-deletions/ 事实目录）与 `.lock` 咨询锁文件——零破坏断言针对业务文件。
fn d2_2_business_snapshot(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    fn walk(
        dir: &std::path::Path,
        root: &std::path::Path,
        out: &mut std::collections::BTreeMap<String, String>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
                continue;
            }
            let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            if name.ends_with(".lock") {
                continue;
            }
            let mut metadata_excluded = false;
            let mut cursor: &std::path::Path = path.as_path();
            while let Some(parent) = cursor.parent() {
                if let Some(segment) = parent.file_name().and_then(|v| v.to_str())
                    && (segment == "entity-operations" || segment == "entity-deletions")
                {
                    metadata_excluded = true;
                    break;
                }
                cursor = parent;
            }
            if metadata_excluded {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap_or_default();
            out.insert(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                format!("{:x}", Sha256::digest(&bytes)),
            );
        }
    }
    let mut snapshot = std::collections::BTreeMap::new();
    walk(root, root, &mut snapshot);
    snapshot
}

fn d2_2_deletion_dir(paths: &ProductAppPaths, kind: &str) -> std::path::PathBuf {
    paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("entity-deletions")
        .join(kind)
}

fn d2_2_pending_path(paths: &ProductAppPaths, kind: &str, id: &str) -> std::path::PathBuf {
    paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("entity-operations")
        .join("pending")
        .join(kind)
        .join(format!("{id}.json"))
}

async fn d2_2_delete(
    app: &axum::Router,
    segment: &str,
    id: &str,
    confirmed: Option<bool>,
) -> (StatusCode, serde_json::Value) {
    let uri = match confirmed {
        Some(value) => {
            format!("/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/{segment}/{id}?confirmed={value}")
        }
        None => format!("/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/{segment}/{id}"),
    };
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
        .expect("send delete");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read delete body");
    let value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// Plan A 完整闭包清除（REQ-GRP-01/REQ-DEL-01/02/07）：source/IR/report/provenance/
/// drafts·compiles·outlines/revision·publication/WI/session/review/run/审计分区/
/// advance·retry/独立 checkpoint/run 文件全清；上游 story、他 Plan、他仓、异 session
/// 保留；恰一条无内容删除记录。
#[tokio::test]
async fn cascade_d2_2_full_closure() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);

    // 上游 story（A 引用它；Story 不随下游 Plan 级联）。
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "上游 story".to_string(),
            aggregate_codebase: None,
        })
        .expect("seed story");

    // A：Draft（ever_confirmed=Some(false) → 免确认）＋真实闭包产物。
    let run_id = lifecycle
        .save_work_item_split_provider_run(
            PROJECT_ID,
            ISSUE_ID,
            &ProviderName::Codex,
            "d2_2 split prompt",
            &serde_json::json!({"plan": D2_2_PLAN_A}),
        )
        .unwrap();
    let profile = lifecycle
        .create_repository_profile(
            crate::product::lifecycle_store::CreateRepositoryProfileInput {
                id: Some("repository_profile_0001".to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                provider_run_ref: Some(run_id.clone()),
                languages: Vec::new(),
                frameworks: Vec::new(),
                package_managers: Vec::new(),
                test_frameworks: Vec::new(),
                build_systems: Vec::new(),
                verification_capabilities: Vec::new(),
                detected_layers: Vec::new(),
                split_recommendation: "single".to_string(),
                confidence: crate::product::models::RepositoryProfileConfidence::Low,
                uncertainties: Vec::new(),
            },
        )
        .unwrap();
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_A,
        vec![story.id.clone()],
        Some(profile.id.clone()),
        Some(run_id.clone()),
        vec!["wi-a-1".to_string()],
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );
    d2_2_wi(&lifecycle, "wi-a-1", D2_2_PLAN_A);
    let repo_target = LogicalRepositoryId(Uuid::new_v4());
    set_target_repository_id(&lifecycle, "wi-a-1", Some(repo_target));
    let vp = lifecycle
        .create_verification_plan(
            crate::product::lifecycle_store::CreateVerificationPlanInput {
                id: Some("verification_plan_0001".to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "wi-a-1".to_string(),
                repository_profile_ref: Some(profile.id.clone()),
                provider_run_ref: Some(run_id.clone()),
                scope: crate::product::models::VerificationScope::Unit,
                commands: Vec::new(),
                manual_checks: Vec::new(),
                required_gates: Vec::new(),
                risk_notes: Vec::new(),
                confidence: crate::product::models::RepositoryProfileConfidence::Low,
                fallback_policy: crate::product::models::VerificationFallbackPolicy::ManualGate,
            },
        )
        .unwrap();
    let plan_session = d2_2_session(&lifecycle, D2_2_PLAN_A, WorkspaceType::WorkItemPlan);
    let review = d2_2_review_for_session(&lifecycle, &plan_session.id);
    // 两个全局 session 审计分区。
    lifecycle
        .append(
            &plan_session.id,
            1,
            crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent::ProviderStart(
                ProviderStartAudit {
                    provider: "codex".to_string(),
                    role: "orchestrator".to_string(),
                    workspace_session_id: plan_session.id.clone(),
                    provider_session_id: "thread-d22".to_string(),
                    tool_policy_canonical_digest: "sha256:d22-digest".to_string(),
                    argv: vec!["--mode".to_string(), "rpc".to_string()],
                    sandbox: None,
                    approval_policy: None,
                    provider_version: "provider 1.2.3".to_string(),
                    adapter_dialect: "codex-app-server-rpc".to_string(),
                    lc_projection: None,
                },
            ),
        )
        .unwrap();
    let (advance_id, retry_command_id) = d2_2_advance_closure(&paths, D2_2_PLAN_A);
    let source_tree = d2_2_source_objects(&paths, D2_2_PLAN_A);
    // drafts/compiles/outlines 产物目录。
    for dir in [
        "work_item_plan_drafts",
        "work_item_plan_compiles",
        "work_item_plan_outlines",
    ] {
        let target = paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join(dir)
            .join(D2_2_PLAN_A);
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("artifact.json"), b"{\"plan\":\"a\"}").unwrap();
    }
    // revision/publication 子树。
    WorkItemRevisionStore::new(paths.clone())
        .put_plan_lineage(&WorkItemPlanLineage {
            id: D2_2_PLAN_A.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_refs: vec![story.id.clone()],
            design_spec_refs: Vec::new(),
            active_revision_id: None,
            active_amendment_id: None,
            created_at: "2026-10-14T00:00:00Z".to_string(),
            updated_at: "2026-10-14T00:00:00Z".to_string(),
        })
        .unwrap();
    // plan run＋前导点锁＋仓维 worktree（无他 owner → 随闭包清）。
    let run = n7_plan_run(
        D2_2_PLAN_A,
        &[(repo_target, "attempt_of_plan_a".to_string())],
        None,
        crate::product::coding_plan_run::CodingPlanRunPhase::ReadyBound,
    );
    n7_write_run(&paths, &run);
    std::fs::write(n7_run_lock(&paths, D2_2_PLAN_A), b"").unwrap();
    lifecycle
        .upsert_repo_shared_worktree(n7_repo_worktree_input(repo_target, "/tmp/d2_2-wt-a"))
        .unwrap();

    // B：旁观 Plan（自有 WI/session/source/advance/run/异仓 worktree，全部保留）。
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_B,
        Vec::new(),
        None,
        None,
        vec!["wi-b-1".to_string()],
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );
    d2_2_wi(&lifecycle, "wi-b-1", D2_2_PLAN_B);
    let repo_other = LogicalRepositoryId(Uuid::new_v4());
    set_target_repository_id(&lifecycle, "wi-b-1", Some(repo_other));
    let plan_session_b = d2_2_session(&lifecycle, D2_2_PLAN_B, WorkspaceType::WorkItemPlan);
    let review_b = d2_2_review_for_session(&lifecycle, &plan_session_b.id);
    let (advance_id_b, retry_command_id_b) = d2_2_advance_closure(&paths, D2_2_PLAN_B);
    let source_tree_b = d2_2_source_objects(&paths, D2_2_PLAN_B);
    let run_b = n7_plan_run(
        D2_2_PLAN_B,
        &[(repo_other, "attempt_of_plan_b".to_string())],
        None,
        crate::product::coding_plan_run::CodingPlanRunPhase::Prepared,
    );
    n7_write_run(&paths, &run_b);
    lifecycle
        .upsert_repo_shared_worktree(n7_repo_worktree_input(repo_other, "/tmp/d2_2-wt-b"))
        .unwrap();

    // advance 独立 engine checkpoint：A session（清）＋他 session（留）。
    let checkpoints_root = root
        .path()
        .join("advance-engine-checkpoints")
        .join("workspace-sessions");
    let checkpoint_a = checkpoints_root.join(&plan_session.id).join("checkpoints");
    std::fs::create_dir_all(&checkpoint_a).unwrap();
    std::fs::write(checkpoint_a.join("cp-1.json"), b"{}").unwrap();
    std::fs::create_dir_all(checkpoints_root.join("session_of_other_entity")).unwrap();

    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    assert_eq!(body["status"], "deleted");

    // A 闭包全清。
    assert!(
        !lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
            .is_ok()
    );
    assert!(
        !source_tree.exists(),
        "source/IR/report/provenance 子树全清"
    );
    for dir in [
        "work_item_plan_drafts",
        "work_item_plan_compiles",
        "work_item_plan_outlines",
    ] {
        assert!(
            !paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join(dir)
                .join(D2_2_PLAN_A)
                .exists(),
            "{dir} 产物全清"
        );
    }
    assert!(
        !paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("work-item-revisions")
            .join(D2_2_PLAN_A)
            .exists(),
        "revision 子树全清"
    );
    assert!(
        !paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("work-item-revision-publications")
            .join(D2_2_PLAN_A)
            .exists(),
        "publication 子树全清"
    );
    let issue_root = paths.issue_root(PROJECT_ID, ISSUE_ID);
    assert!(!issue_root.join("work-items").join("wi-a-1.json").exists());
    let sessions_root = paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("workspace-sessions");
    assert!(
        !sessions_root
            .join(format!("{}.json", plan_session.id))
            .exists()
    );
    assert!(
        !paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-timelines")
            .join(&plan_session.id)
            .exists(),
        "timeline 随 session 清"
    );
    assert!(
        !paths
            .root()
            .join("work-item-split-runs")
            .join(&plan_session.id)
            .exists()
            && !paths
                .root()
                .join("tool-policy-run-audit")
                .join(&plan_session.id)
                .exists(),
        "两个全局 session 审计分区清"
    );
    assert!(
        !paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("provider-review-rounds")
            .join(format!("{}.json", review.id))
            .exists(),
        "review 随 session 清"
    );
    assert!(
        !paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("provider-runs")
            .join(&run_id)
            .exists(),
        "归属 run 档案清"
    );
    assert!(
        !paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("verification-plans")
            .join(format!("{}.json", vp.id))
            .exists()
            && !paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("repository-profiles")
                .join(format!("{}.json", profile.id))
                .exists(),
        "VP/profile 随闭包清"
    );
    let advance_root = paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("advance-records");
    assert!(
        !advance_root.join(format!("{advance_id}.json")).exists()
            && !advance_root
                .join(format!("{D2_2_PLAN_A}.initialization.json"))
                .exists(),
        "advance record/journal 清"
    );
    assert!(
        !paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("advance-retries")
            .join(format!("{retry_command_id}.json"))
            .exists(),
        "advance retry 清"
    );
    assert!(
        AdvanceStore::new(paths.clone())
            .list_plan_artifacts(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
            .unwrap()
            .advance_ids
            .is_empty(),
        "advance list 复核空"
    );
    assert!(!n7_run_file(&paths, D2_2_PLAN_A).exists());
    assert!(!n7_run_lock(&paths, D2_2_PLAN_A).exists());
    assert!(
        lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repo_target)
            .unwrap()
            .is_none(),
        "目标仓 worktree 无他 owner 随闭包清"
    );
    assert!(
        !checkpoints_root.join(&plan_session.id).exists(),
        "advance 独立 checkpoint 按 session 清"
    );
    assert!(
        checkpoints_root.join("session_of_other_entity").exists(),
        "不整删 checkpoint 全局根"
    );

    // B／上游／基础设施保留。
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_B)
            .is_ok()
    );
    assert!(source_tree_b.exists(), "他 Plan source 不误删");
    assert!(
        issue_root.join("work-items").join("wi-b-1.json").exists()
            && sessions_root
                .join(format!("{}.json", plan_session_b.id))
                .exists()
    );
    assert!(
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("provider-review-rounds")
            .join(format!("{}.json", review_b.id))
            .exists()
    );
    assert!(
        advance_root.join(format!("{advance_id_b}.json")).exists()
            && paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("advance-retries")
                .join(format!("{retry_command_id_b}.json"))
                .exists(),
        "他 Plan advance/retry 保留"
    );
    assert!(n7_run_file(&paths, D2_2_PLAN_B).exists());
    assert!(
        lifecycle
            .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repo_other)
            .unwrap()
            .is_some(),
        "异仓 worktree 不被 issue-wide 扫除"
    );
    assert!(
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("story-specs")
            .join(format!("{}.json", story.id))
            .exists(),
        "上游 story 不随下游删除"
    );

    // 恰一条无内容删除记录；pending 已移除。
    let deletion_dir = d2_2_deletion_dir(&paths, "plan");
    let records: Vec<_> = std::fs::read_dir(&deletion_dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(records.len(), 1, "恰一条删除记录");
    let record: EntityDeletionRecord = crate::product::json_store::read_json(&records[0]).unwrap();
    assert_eq!(record.phase, DeletionPhase::Deleted);
    assert_eq!(record.scope.entity_id, D2_2_PLAN_A);
    assert!(record.deleted_at.is_some());
    assert!(
        !d2_2_pending_path(&paths, "plan", D2_2_PLAN_A).exists(),
        "成功删除后 pending 移除"
    );
}

/// 拒删路径零破坏（REQ-DEL-03/04/05/06）：迟到子 attempt、repo 资源 writer 竞态、
/// 共享 profile 外部引用、未确认——各 409 拒绝后业务字节与记录数不变、无墓碑。
#[tokio::test]
async fn cascade_d2_2_late_child_attempt_zero_mutation() {
    // A) legacy plan 后一个 WI 有 attempt（前一个已独立删除）→ coding 拒删零破坏。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        d2_2_seed_plan(
            &lifecycle,
            D2_2_PLAN_A,
            Vec::new(),
            None,
            None,
            vec!["wi-a-1".to_string(), "wi-a-2".to_string()],
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );
        d2_2_wi(&lifecycle, "wi-a-1", D2_2_PLAN_A);
        d2_2_wi(&lifecycle, "wi-a-2", D2_2_PLAN_A);
        d2_2_seed_repository(&paths);
        // 前一个 WI 先被独立删除（合法路径）。
        let (status, _) = d2_2_delete(&app, "work-items", "wi-a-1", None).await;
        assert_eq!(status, StatusCode::OK);
        // 后一个 WI 存在 coding attempt 记录（未运行也拒）。
        let coding_store = CodingAttemptStore::new(paths.clone());
        let attempt = coding_store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "wi-a-2".to_string(),
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
            .expect("seed attempt");

        let before = d2_2_business_snapshot(root.path());
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::CONFLICT, "gate body: {body}");
        assert_eq!(body["code"], "coding_workspace_exists");
        assert_eq!(body["details"]["work_item_id"], "wi-a-2");
        let after = d2_2_business_snapshot(root.path());
        assert_eq!(before, after, "拒删路径所有业务文件字节与记录数不变");
        assert!(
            lifecycle
                .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
                .is_ok(),
            "plan 记录保留"
        );
        assert!(
            coding_store
                .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
                .is_ok(),
            "attempt 保留"
        );
        assert!(
            EntityDeletionStore::new(paths.clone())
                .get(&EntityScope::new(
                    PROJECT_ID,
                    ISSUE_ID,
                    crate::product::entity_deletion_store::DeletionEntityKind::Plan,
                    D2_2_PLAN_A
                ))
                .unwrap()
                .is_none(),
            "拒删不立墓碑"
        );
    }

    // B) repo 资源 writer 竞态：删除先取 cleanup lease，writer 持写 lease → entity_busy。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        d2_2_seed_plan(
            &lifecycle,
            D2_2_PLAN_A,
            Vec::new(),
            None,
            None,
            vec!["wi-a-1".to_string()],
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );
        d2_2_wi(&lifecycle, "wi-a-1", D2_2_PLAN_A);
        let repo_target = LogicalRepositoryId(Uuid::new_v4());
        set_target_repository_id(&lifecycle, "wi-a-1", Some(repo_target));
        lifecycle
            .upsert_repo_shared_worktree(n7_repo_worktree_input(repo_target, "/tmp/d2_2-race"))
            .unwrap();
        let operations = EntityOperationStore::new(paths.clone());
        let writer_lease = operations
            .try_acquire_shared_resource_write(&SharedResourceKey {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: Some(repo_target),
            })
            .expect("hold resource writer lease");

        let before = d2_2_business_snapshot(root.path());
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::CONFLICT, "busy body: {body}");
        assert_eq!(body["code"], "entity_busy");
        assert_eq!(
            d2_2_business_snapshot(root.path()),
            before,
            "busy 拒绝零破坏"
        );
        drop(writer_lease);

        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "writer 释放后删除成功: {body}");
        assert!(
            lifecycle
                .get_repo_shared_worktree(PROJECT_ID, ISSUE_ID, repo_target)
                .unwrap()
                .is_none(),
            "无在途 writer 后目标仓资源清"
        );
    }

    // C) 共享业务对象（repository profile）被他 Plan 配置引用 → entity_referenced；
    //    解除引用后放行（与基础设施 shared-worktree 保留语义分开）。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        let profile = lifecycle
            .create_repository_profile(
                crate::product::lifecycle_store::CreateRepositoryProfileInput {
                    id: Some("repository_profile_0001".to_string()),
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    repository_id: REPOSITORY_ID.to_string(),
                    provider_run_ref: None,
                    languages: Vec::new(),
                    frameworks: Vec::new(),
                    package_managers: Vec::new(),
                    test_frameworks: Vec::new(),
                    build_systems: Vec::new(),
                    verification_capabilities: Vec::new(),
                    detected_layers: Vec::new(),
                    split_recommendation: "single".to_string(),
                    confidence: crate::product::models::RepositoryProfileConfidence::Low,
                    uncertainties: Vec::new(),
                },
            )
            .unwrap();
        d2_2_seed_plan(
            &lifecycle,
            D2_2_PLAN_A,
            Vec::new(),
            Some(profile.id.clone()),
            None,
            Vec::new(),
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );
        d2_2_seed_plan(
            &lifecycle,
            D2_2_PLAN_B,
            Vec::new(),
            Some(profile.id.clone()),
            None,
            Vec::new(),
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );

        let before = d2_2_business_snapshot(root.path());
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::CONFLICT, "referenced body: {body}");
        assert_eq!(body["code"], "entity_referenced");
        assert_eq!(
            body["details"]["references"][0]["id"], D2_2_PLAN_B,
            "列出实际引用者"
        );
        assert_eq!(d2_2_business_snapshot(root.path()), before);
        // 解除 B 的引用后放行。
        let plan_b_path = paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("issue-work-item-plans")
            .join(format!("{D2_2_PLAN_B}.json"));
        let mut plan_b: crate::product::models::IssueWorkItemPlan =
            crate::product::json_store::read_json(&plan_b_path).unwrap();
        plan_b.repository_profile_ref = None;
        crate::product::json_store::write_json(&plan_b_path, &plan_b).unwrap();
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "解除引用后放行: {body}");
        assert!(
            !paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("repository-profiles")
                .join(format!("{}.json", profile.id))
                .exists(),
            "A 专属 profile 随闭包清"
        );
    }

    // D) 服务端分级确认（REQ-DEL-04）：Confirmed plan 未带 confirmed → 稳定 409 拒绝；
    //    同意后一次删除完成。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        d2_2_seed_plan(
            &lifecycle,
            D2_2_PLAN_A,
            Vec::new(),
            None,
            None,
            Vec::new(),
            crate::product::models::IssueWorkItemPlanStatus::Confirmed,
        );

        let before = d2_2_business_snapshot(root.path());
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::CONFLICT, "confirm body: {body}");
        assert_eq!(body["code"], "deletion_confirmation_required");
        assert_eq!(body["details"]["reason"], "previously_confirmed");
        assert_eq!(
            d2_2_business_snapshot(root.path()),
            before,
            "未确认拒绝零破坏"
        );

        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, Some(true)).await;
        assert_eq!(status, StatusCode::OK, "确认后删除: {body}");
        assert_eq!(body["status"], "deleted");
        let record: EntityDeletionRecord = crate::product::json_store::read_json(
            &d2_2_deletion_dir(&paths, "plan").join(format!("{D2_2_PLAN_A}.json")),
        )
        .unwrap();
        assert_eq!(record.phase, DeletionPhase::Deleted);
    }
}

/// 失败重入与根/pending 缺失后的 scope 反查（REQ-DEL-07/02）：子依赖 failpoint 中断
/// 保留重试入口；重入续清；根删后 complete 前中断凭固定来源收口；Deleted 幂等。
#[tokio::test]
async fn cascade_d2_2_failure_reentry() {
    // A) 子依赖损坏 JSON → 删除失败上抛；墓碑+pending 落盘、根与未清子依赖保留；
    //    修复后重入一次收口、恰一条记录。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        d2_2_seed_plan(
            &lifecycle,
            D2_2_PLAN_A,
            Vec::new(),
            None,
            None,
            vec!["wi-a-1".to_string()],
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );
        d2_2_wi(&lifecycle, "wi-a-1", D2_2_PLAN_A);
        let plan_session = d2_2_session(&lifecycle, D2_2_PLAN_A, WorkspaceType::WorkItemPlan);
        let review = d2_2_review_for_session(&lifecycle, &plan_session.id);
        d2_2_advance_closure(&paths, D2_2_PLAN_A);
        let review_dir = paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("provider-review-rounds");
        let review_path = review_dir.join(format!("{}.json", review.id));
        // failpoint：review 目录置只读——采集（只读）可过、purge ①（删除）必败，
        // 真实 IO 错误在墓碑之后上抛（不当空、不假记完成）。
        std::fs::set_permissions(
            &review_dir,
            std::os::unix::fs::PermissionsExt::from_mode(0o555),
        )
        .unwrap();

        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_ne!(status, StatusCode::OK, "failpoint 必须上抛: {body}");
        let scope = EntityScope::new(
            PROJECT_ID,
            ISSUE_ID,
            crate::product::entity_deletion_store::DeletionEntityKind::Plan,
            D2_2_PLAN_A,
        );
        let deletion_store = EntityDeletionStore::new(paths.clone());
        let record = deletion_store.get(&scope).unwrap().expect("墓碑在位");
        assert_eq!(record.phase, DeletionPhase::Purging);
        assert!(
            d2_2_pending_path(&paths, "plan", D2_2_PLAN_A).exists(),
            "pending manifest 落盘（重入入口）"
        );
        assert!(
            lifecycle
                .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
                .is_ok(),
            "根最后：失败时根保留"
        );
        assert!(
            paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("work-items")
                .join("wi-a-1.json")
                .exists(),
            "未清到的子依赖保留"
        );

        // 修复 failpoint 后重入：pending+owner 索引续清，一次收口。
        std::fs::set_permissions(
            &review_dir,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "重入收口: {body}");
        assert!(
            !review_path.exists()
                && !paths
                    .issue_root(PROJECT_ID, ISSUE_ID)
                    .join("work-items")
                    .join("wi-a-1.json")
                    .exists()
                && !lifecycle
                    .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
                    .is_ok(),
            "重入清完整闭包"
        );
        let record: EntityDeletionRecord = crate::product::json_store::read_json(
            &d2_2_deletion_dir(&paths, "plan").join(format!("{D2_2_PLAN_A}.json")),
        )
        .unwrap();
        assert_eq!(record.phase, DeletionPhase::Deleted);
        assert!(
            !d2_2_pending_path(&paths, "plan", D2_2_PLAN_A).exists(),
            "成功后 pending 移除"
        );
        let records: Vec<_> = std::fs::read_dir(d2_2_deletion_dir(&paths, "plan"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(records.len(), 1, "重试不重复留痕");
    }

    // B) 根已删、pending 已移除、complete 前中断（模拟崩溃）→ 重入凭 §0.5 固定来源
    //    反查收敛，根缺不 404；Deleted 幂等 200。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        d2_2_seed_plan(
            &lifecycle,
            D2_2_PLAN_A,
            Vec::new(),
            None,
            None,
            vec!["wi-a-1".to_string()],
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );
        d2_2_wi(&lifecycle, "wi-a-1", D2_2_PLAN_A);
        let (status, _) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK);

        // 模拟 complete 前崩溃：记录回置 purging（pending 已移除、根已删）。
        let record_path = d2_2_deletion_dir(&paths, "plan").join(format!("{D2_2_PLAN_A}.json"));
        let mut record: EntityDeletionRecord =
            crate::product::json_store::read_json(&record_path).unwrap();
        record.phase = DeletionPhase::Purging;
        record.deleted_at = None;
        crate::product::json_store::write_json(&record_path, &record).unwrap();

        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "根缺不 404，重入收口: {body}");
        let record: EntityDeletionRecord =
            crate::product::json_store::read_json(&record_path).unwrap();
        assert_eq!(record.phase, DeletionPhase::Deleted, "完成单条事实");
        assert!(record.deleted_at.is_some());

        // Deleted 幂等：重复请求 200、记录不再变化。
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "Deleted 幂等: {body}");
        assert_eq!(body["status"], "deleted");
        let after: EntityDeletionRecord =
            crate::product::json_store::read_json(&record_path).unwrap();
        assert_eq!(after, record, "幂等重试不新增记录、不改完成时间");
    }

    // C) Story 闭包：被 Design 引用拒删 → 先删后置 → 上游放行全清（REQ-DEL-01/03）。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                title: "上游 story".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "后置 design".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        let story_session = d2_2_session(&lifecycle, &story.id, WorkspaceType::Story);
        let story_review = d2_2_review_for_session(&lifecycle, &story_session.id);
        lifecycle
            .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: story.id.clone(),
                markdown: "# story v1".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            })
            .unwrap();

        let (status, body) = d2_2_delete(&app, "story-specs", &story.id, None).await;
        assert_eq!(status, StatusCode::CONFLICT, "story body: {body}");
        assert_eq!(body["code"], "entity_referenced");
        assert_eq!(body["details"]["references"][0]["kind"], "design_spec");

        let (status, _) = d2_2_delete(&app, "design-specs", &design.id, None).await;
        assert_eq!(status, StatusCode::OK, "先删后置 design");
        let (status, body) = d2_2_delete(&app, "story-specs", &story.id, None).await;
        assert_eq!(status, StatusCode::OK, "后删上游: {body}");

        assert!(
            !paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("story-specs")
                .join(format!("{}.json", story.id))
                .exists()
                && !paths
                    .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                    .join("design-specs")
                    .join(format!("{}.json", design.id))
                    .exists()
        );
        assert!(
            !paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("versions")
                .join(&story.id)
                .exists(),
            "story versions 全清"
        );
        let sessions_root = paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions");
        assert!(
            !sessions_root
                .join(format!("{}.json", story_session.id))
                .exists()
        );
        assert!(
            !paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("provider-review-rounds")
                .join(format!("{}.json", story_review.id))
                .exists(),
            "story review 随闭包清"
        );
        let record: EntityDeletionRecord = crate::product::json_store::read_json(
            &d2_2_deletion_dir(&paths, "story").join(format!("{}.json", story.id)),
        )
        .unwrap();
        assert_eq!(record.phase, DeletionPhase::Deleted);
    }
}
