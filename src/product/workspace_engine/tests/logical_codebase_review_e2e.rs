//! T19 规划线端到端夹具验收（REQ-PLN-11…17 / REQ-ENV-13/14 的 e2e 面）。
//!
//! 多仓 issue 选 2 个成员仓 → Story / Design / WorkItemPlan 三环节各跑通
//! 「author 输入构建 → reviewer 评审 → verdict」：
//! - 材料经生产入口 `ensure_workspace_context_message` 真实注入（成员清单全量
//!   + 逐仓实况 + 逐仓 HEAD 分支基线），不是手工拼的消息；
//! - reviewer prompt 每轮含 2 仓完整清单与逐仓实况（`基线:` 行取真实 git 分支）；
//! - 无 `primary` / 主成员字样、无评审窗记录、无 `primary-member anchored` 事件、
//!   无 `plan_review_window_*` 错误码（REQ-PLN-11 / REQ-ENV-14）；
//! - 评审身份为会话 UUID（REQ-ENV-13）。
//!
//! 夹具仓根 / 日志 / 快照全部落 `/tmp`（`tempfile::tempdir`）。

use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::{
    AggregateDesignSpecScope, AggregateStorySpecScope, CreateDesignSpecInput,
    CreateIssueWorkItemPlanInput, CreateStorySpecInput,
};
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::logical_codebase::{
    self, AggregatePolicyArtifactStore, IssueCodebaseSelection, IssueCodebaseSelectionStore,
    LogicalCodebaseFeature, LogicalCodebaseProviderGateway, LogicalCodebaseStore,
    LogicalRepositoryId, PolicyTarget, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderGatewayError, ProviderRef, SessionLaunchRequest,
    SessionPolicyAction,
};
use crate::product::models::IssueWorkItemPlanStatus;
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};
use std::path::Path;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// 最小 lc gateway 桩（与 review/drive_tests.rs t2_lc_no_review_window 同构）：
// 形态门禁只读 `Option::is_some()`，不 bootstrap 政策，不触 validate 链。
// ---------------------------------------------------------------------------

struct UnusedCapabilitySource;

impl ProviderCapabilitySource for UnusedCapabilitySource {
    fn require_supported(
        &self,
        _provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "t19 e2e fixture capability source is never queried".to_string(),
        ))
    }

    fn require_resume_supported(
        &self,
        _provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "t19 e2e fixture capability source is never queried".to_string(),
        ))
    }

    fn require_write_boundary(
        &self,
        _provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "t19 e2e fixture capability source is never queried".to_string(),
        ))
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "t19 e2e fixture capability source is never queried".to_string(),
        ))
    }
}

struct UnusedTargetResolver;

impl PolicyTargetResolver for UnusedTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

struct UnusedSyncAdapter;

impl ProviderAdapter for UnusedSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = chrono::Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [ProviderName::ClaudeCode]
            .into_iter()
            .map(|provider| ProviderHealthEntry {
                provider,
                command: "stub".to_string(),
                available: true,
                version: Some("1.0".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            })
            .collect(),
    });
    Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
        snapshot,
    ))))
}

// ---------------------------------------------------------------------------
// verdict provider：按 stage 回结构化 verdict（approve / plan pass）。
// ---------------------------------------------------------------------------

struct E2EVerdictProvider {
    payload: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for E2EVerdictProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        let output = "两仓协同已核对，可以确认。".to_string();
        let contract = input.structured_output_contract;
        let payload = self.payload.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let full_output = if let Some(contract) = contract.as_ref() {
                // 结构化合同双校验：信封属性与 JSON 体内的 nonce 字段都要回。
                let mut json: serde_json::Value =
                    serde_json::from_str(&payload).expect("e2e verdict JSON");
                json.as_object_mut()
                    .expect("e2e verdict JSON must be an object")
                    .insert("nonce".to_string(), serde_json::json!(contract.nonce));
                format!(
                    "{output}\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">{json}</ARIA_STRUCTURED_OUTPUT>",
                    contract.nonce
                )
            } else {
                output
            };
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::from_output(
                    full_output,
                    contract.as_ref(),
                    None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

// ---------------------------------------------------------------------------
// 夹具：2 个真实 git 成员仓（各自 checkout 到 feature 分支）+ lc manifest +
// 多仓 issue + 2 仓 selection + story/design/work item plan 三类实体。
// ---------------------------------------------------------------------------

struct TwoRepoLcFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    lifecycle: LifecycleStore,
    _authority_kept: tempfile::TempDir,
    gateway: Arc<LogicalCodebaseProviderGateway>,
    project_id: String,
    issue_id: String,
    story_id: String,
    design_id: String,
    plan_id: String,
    members: [MemberExpectation; 2],
}

struct MemberExpectation {
    logical_id: LogicalRepositoryId,
    checkout_id: crate::product::logical_codebase::RepositoryCheckoutId,
    alias: String,
    branch: String,
}

fn git_step(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git fixture command");
    assert!(status.success(), "git fixture {args:?} must succeed");
}

fn git_repo_on_branch(dir: &Path, branch: &str, code_path: &str, code_content: &str) {
    git_step(dir, &["init", "--quiet"]);
    git_step(dir, &["config", "user.email", "t19-e2e@example.test"]);
    git_step(dir, &["config", "user.name", "T19 E2E"]);
    // aggregate index 要求每个成员仓至少含 1 个可索引代码文件。
    let code_file = dir.join(code_path);
    if let Some(parent) = code_file.parent() {
        std::fs::create_dir_all(parent).expect("member code dir");
    }
    std::fs::write(&code_file, code_content).expect("member code file");
    git_step(dir, &["add", "."]);
    git_step(dir, &["commit", "-m", "init", "--quiet"]);
    git_step(dir, &["checkout", "-b", branch]);
}

fn two_repo_lc_fixture() -> TwoRepoLcFixture {
    let root = tempfile::tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "T19-LC-E2E".to_string(),
            description: None,
        })
        .expect("project");
    let project_id = project.id.clone();

    let member_specs = [
        (
            "member-a",
            "feature/e2e-member-a",
            "src/lib.rs",
            "pub fn announcement_api() -> &'static str {\n    \"/api/announcements\"\n}\n",
        ),
        (
            "member-b",
            "feature/e2e-member-b",
            "src/index.ts",
            "export function renderAnnouncements(): string {\n  return \"announcements\";\n}\n",
        ),
    ];
    let mut logical_ids: Vec<LogicalRepositoryId> = Vec::new();
    let mut members: Vec<MemberExpectation> = Vec::new();
    for (name, branch, code_path, code_content) in member_specs {
        let dir = root.path().join(name);
        std::fs::create_dir_all(&dir).expect("member dir");
        git_repo_on_branch(&dir, branch, code_path, code_content);
        let record = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        )
        .create(CreateRepositoryInput {
            project_id: project_id.clone(),
            name: name.to_string(),
            path: dir,
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: format!("t19-lc-e2e-{name}"),
        })
        .expect("member repository");
        let logical_id = record.logical_repository_id.expect("logical id");
        let checkout_id = record.primary_checkout_id.expect("primary checkout id");
        logical_ids.push(logical_id);
        members.push(MemberExpectation {
            logical_id,
            checkout_id,
            alias: name.to_string(),
            branch: branch.to_string(),
        });
    }

    // active aggregate index（onboarding 落盘形态）：成员快照与两成员对齐。
    let indexed_at = "2026-10-23T00:00:00Z".to_string();
    let snapshots = members
        .iter()
        .map(|member| {
            AggregateIndexMemberSnapshot::indexed(
                member.logical_id,
                member.checkout_id,
                "abc123".to_string(),
                false,
                indexed_at.clone(),
            )
        })
        .collect::<Vec<_>>();
    let index = AggregateIndexRecord::building(
        "aggregate_index_0001".to_string(),
        project_id.clone(),
        1,
        snapshots,
        indexed_at,
    );
    AggregateIndexStore::new(paths.clone())
        .create(&project_id, index.clone())
        .expect("create aggregate index");
    let mut activated = index;
    activated.status = AggregateIndexStatus::Active;
    AggregateIndexStore::new(paths.clone())
        .replace_active(&project_id, activated)
        .expect("activate aggregate index");
    let manifest = LogicalCodebaseStore::new(paths.clone())
        .load_manifest(&project_id)
        .expect("manifest io")
        .expect("manifest seeded by member creation");
    AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .expect("bootstrap policy");

    let legacy_lc = logical_codebase::legacy_logical_codebase_id(&project_id);
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: project_id.clone(),
            repo_id: None,
            logical_codebase_id: Some(legacy_lc),
            base_branch: None,
            title: "公告管理升级（跨 member-a / member-b）".to_string(),
            description: Some("两仓协同的公告管理升级需求".to_string()),
            change_id: None,
        })
        .expect("issue");
    let issue_id = issue.id.clone();
    // issue 选定 2 仓（评审范围 = 该集合，REQ-PLN-11）。
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            &project_id,
            &issue_id,
            logical_ids.clone(),
            Vec::new(),
            Vec::new(),
            None,
        ))
        .expect("selection");

    let lifecycle = LifecycleStore::new(paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id: String::new(),
            title: "公告管理升级 story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                logical_codebase_ref: manifest.logical_codebase_id,
                effective_member_ids: logical_ids.clone(),
                involved_repository_ids: logical_ids.clone(),
                focus_repository_id: None,
            }),
        })
        .expect("story spec");
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            story_spec_ids: vec![story.id.clone()],
            title: "公告管理升级 design".to_string(),
            aggregate_codebase: Some(AggregateDesignSpecScope {
                logical_codebase_ref: manifest.logical_codebase_id,
                effective_member_ids: logical_ids.clone(),
                involved_repository_ids: logical_ids.clone(),
                change_order: Vec::new(),
            }),
        })
        .expect("design spec");
    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: None,
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            source_story_spec_ids: vec![story.id.clone()],
            source_design_spec_ids: vec![design.id.clone()],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("work item plan");

    // 最小 lc gateway（形态判据 + reviewer cwd=authority root）。
    let authority_root_dir = tempfile::tempdir().expect("authority root");
    let authority_root = authority_root_dir.path().join("authority");
    std::fs::create_dir_all(&authority_root).expect("create authority root");
    let gateway = Arc::new(LogicalCodebaseProviderGateway::new(
        AggregatePolicyArtifactStore::new(paths.clone()),
        Arc::new(UnusedCapabilitySource),
        Arc::new(UnusedTargetResolver),
        Arc::new(ProviderRegistry::new()),
        Arc::new(UnusedSyncAdapter),
        always_available_gate(),
        authority_root,
    ));

    TwoRepoLcFixture {
        _root: root,
        paths,
        lifecycle,
        _authority_kept: authority_root_dir,
        gateway,
        project_id,
        issue_id,
        story_id: story.id,
        design_id: design.id,
        plan_id: plan.id,
        members: [members.remove(0), members.remove(0)],
    }
}

/// 材料断言（REQ-PLN-11/12/14）：2 仓完整清单 + 逐仓实况（含真实分支基线行），
/// 无 primary / 主成员 / 缺员标记。
fn assert_material_covers_both_members(prompt: &str, surface: &str, fixture: &TwoRepoLcFixture) {
    assert!(
        prompt.contains("## 聚合代码库成员清单"),
        "{surface} 必须含成员清单 marker"
    );
    assert!(prompt.contains("## 逐仓实况"), "{surface} 必须含逐仓实况段");
    for member in &fixture.members {
        let id = member.logical_id.0.to_string();
        assert!(
            prompt.contains(&format!("{} | {}", id, member.alias)),
            "{surface} 清单必须全量含成员 {}（{}）：\n{}",
            member.alias,
            id,
            prompt
        );
        assert!(
            prompt.contains(&format!("基线: {}（成员 {}", member.branch, id)),
            "{surface} 逐仓实况必须含成员 {} 的 HEAD 分支基线行（{}）：\n{}",
            member.alias,
            member.branch,
            prompt
        );
    }
    assert!(
        !prompt.to_lowercase().contains("primary") && !prompt.contains("主成员"),
        "{surface} 不得出现 primary / 主成员字样"
    );
    assert!(
        !prompt.contains("omitted_member") && !prompt.contains("truncated"),
        "{surface} 不得出现缺员 / 截断标记（REQ-PLN-12 全量清单）"
    );
}

struct StageRunResult {
    verdict: ReviewVerdictType,
    review_identity_id: String,
}

/// 单环节 e2e：author 输入构建 → reviewer 评审 → verdict，附全部负例断言。
async fn run_stage_end_to_end(
    fixture: &TwoRepoLcFixture,
    workspace_type: WorkspaceType,
) -> StageRunResult {
    let entity_id = match workspace_type {
        WorkspaceType::Story => fixture.story_id.clone(),
        WorkspaceType::Design => fixture.design_id.clone(),
        WorkspaceType::WorkItemPlan => fixture.plan_id.clone(),
        WorkspaceType::WorkItem => unreachable!("T19 不驱动 WorkItem 执行面"),
    };
    let record = fixture
        .lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: fixture.project_id.clone(),
            issue_id: fixture.issue_id.clone(),
            entity_id,
            workspace_type: workspace_type.clone(),
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 2,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");

    // 生产入口注入真实材料（成员清单 + 逐仓实况 + 基线行）。
    let record = crate::web::workspace_context::ensure_workspace_context_message(
        &fixture.paths,
        &fixture.lifecycle,
        record,
    )
    .await
    .expect("workspace context message");

    let mut session = WorkspaceSession::from_record(record);
    session.artifact = Some(artifact_payload(&match &workspace_type {
        WorkspaceType::Story => complete_story_artifact(
            "两仓公告管理升级：member-a 提供接口、member-b 提供前端页面",
            "两仓联调用例全部通过",
        ),
        WorkspaceType::Design => complete_design_artifact(
            "member-a 提供 REST 接口、member-b 消费渲染",
            "GET /api/announcements 返回分页公告列表",
        ),
        _ => "# Work Item 拆分计划\n\n## 拆分\n- WI-1: member-a 接口改造\n- WI-2: member-b 前端改造\n"
            .to_string(),
    }));
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let checkpoint = Arc::new(CheckpointStore::new(
        fixture
            .paths
            .issue_lifecycle_root(&fixture.project_id, &fixture.issue_id),
    ));
    let engine =
        WorkspaceEngine::new_persistent(checkpoint, fixture.lifecycle.clone(), event_tx, session);
    let mut engine = engine.with_logical_provider_gateway(fixture.gateway.clone());
    let window_root = fixture.paths.root().join("reviewer-windows");

    // ---- author 面：author 输入携带真实材料（author→reviewer→verdict 的第一环）----
    let author_input = engine
        .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
        .expect("author input");
    assert_material_covers_both_members(&author_input.prompt, "author prompt", fixture);
    match workspace_type {
        WorkspaceType::Story | WorkspaceType::Design => assert!(
            author_input.structured_output_contract.is_some(),
            "{workspace_type:?} 聚合 author 输入必须带结构化输出合同"
        ),
        WorkspaceType::WorkItemPlan => assert!(
            author_input.structured_output_contract.is_none(),
            "WorkItemPlan author 输入必须保持无结构化输出合同"
        ),
        WorkspaceType::WorkItem => unreachable!(),
    }

    // ---- reviewer 面：prompt 含材料、身份为会话 UUID、无窗口 ----
    let review_identity_id = engine.session.review_identity_id.clone();
    assert!(
        !review_identity_id.is_empty(),
        "lc 会话必须有评审身份（REQ-ENV-13）"
    );
    assert!(
        engine.reviewer_verify_policy().is_some(),
        "lc 会话 reviewer_verify_policy 必须由会话 UUID 身份给出"
    );
    engine.start_review_or_skip().await;
    let review_input = engine.build_review_input().expect("reviewer prompt");
    assert_material_covers_both_members(&review_input.prompt, "reviewer prompt", fixture);

    let payload = match workspace_type {
        WorkspaceType::Story | WorkspaceType::Design => {
            "{\"verdict\":\"pass\",\"summary\":\"两仓协同已核对，可以确认\",\"findings\":[]}"
                .to_string()
        }
        WorkspaceType::WorkItemPlan => {
            "{\"review_scope\":\"outline\",\"verdict\":\"pass\",\"summary\":\"两仓拆分合理，可以确认\",\"generation_round_id\":\"t19_e2e_round\",\"findings\":[]}".to_string()
        }
        WorkspaceType::WorkItem => unreachable!(),
    };
    let (_command_tx, command_rx) = mpsc::channel(8);
    engine
        .drive_review_session(Arc::new(E2EVerdictProvider { payload }), command_rx)
        .await;

    // ---- verdict 面 ----
    let verdict = engine
        .latest_review_verdict
        .clone()
        .unwrap_or_else(|| panic!("{workspace_type:?} 评审完成必须落 verdict"));
    let expected = match workspace_type {
        WorkspaceType::Story | WorkspaceType::Design => ReviewVerdictType::Pass,
        WorkspaceType::WorkItemPlan => ReviewVerdictType::Pass,
        WorkspaceType::WorkItem => unreachable!(),
    };
    assert_eq!(
        verdict.verdict, expected,
        "{workspace_type:?} verdict 必须为结构化结论：{:?}",
        verdict.structured_output_diagnostic
    );

    // ---- 负例：无错误事件、无窗口噪声、无 primary 事件（REQ-ENV-14/PLN-11）----
    let mut errors = Vec::new();
    let mut window_noise = Vec::new();
    let mut primary_noise = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        match event {
            EngineEvent::Error { message } => errors.push(message),
            EngineEvent::ExecutionEvent { event, .. } => {
                let text = format!("{} {}", event.title, event.detail.unwrap_or_default());
                if text.contains("plan_review_window") || text.contains("评审窗") {
                    window_noise.push(text.clone());
                }
                if text.to_lowercase().contains("primary")
                    || text.contains("主成员")
                    || text.contains("primary-member anchored")
                {
                    primary_noise.push(text);
                }
            }
            _ => {}
        }
    }
    assert!(
        errors.is_empty(),
        "{workspace_type:?} lc 评审必须无错误收尾: {errors:?}"
    );
    assert!(
        window_noise.is_empty(),
        "{workspace_type:?} 不得出现窗口错误码 / 窗口注意力提示: {window_noise:?}"
    );
    assert!(
        primary_noise.is_empty(),
        "{workspace_type:?} 不得出现 primary / 主成员 / 锚定附注事件: {primary_noise:?}"
    );
    assert!(
        !window_root.exists()
            || std::fs::read_dir(&window_root)
                .map(|entries| entries.count())
                .unwrap_or(0)
                == 0,
        "{workspace_type:?} lc 评审不得留下评审窗记录（{}）",
        window_root.display()
    );

    StageRunResult {
        verdict: verdict.verdict,
        review_identity_id,
    }
}

/// T19 规划线端到端：多仓 issue 选 2 仓，story/design/work item 三环节各跑通
/// author→reviewer→verdict；reviewer prompt 每轮含 2 仓完整清单 + 逐仓实况
/// （含 `基线:` 行）、无 `primary` 字样、无窗口记录、无 `primary-member
/// anchored` 事件、无 `plan_review_window_*` 错误码。
#[tokio::test]
async fn logical_codebase_review_runs_story_design_workitem_end_to_end() {
    let fixture = two_repo_lc_fixture();

    let story = run_stage_end_to_end(&fixture, WorkspaceType::Story).await;
    let design = run_stage_end_to_end(&fixture, WorkspaceType::Design).await;
    let work_item = run_stage_end_to_end(&fixture, WorkspaceType::WorkItemPlan).await;

    assert_eq!(story.verdict, ReviewVerdictType::Pass);
    assert_eq!(design.verdict, ReviewVerdictType::Pass);
    assert_eq!(work_item.verdict, ReviewVerdictType::Pass);

    // REQ-ENV-13：评审身份为每会话 UUID——两会话不得共享同一身份。
    assert_ne!(
        story.review_identity_id, design.review_identity_id,
        "不同会话必须各自生成唯一评审身份"
    );
    assert_ne!(design.review_identity_id, work_item.review_identity_id);
}
