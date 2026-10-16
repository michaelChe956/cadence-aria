use super::*;
use crate::cross_cutting::provider_adapter::{ProviderAdapterError, structured_output_sentinel};
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamChunk, StreamingProviderInput,
};
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::{
    CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
    CreateWorkspaceSessionInput, WorkItemPlanSessionOptions,
};
use crate::product::models::{IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, WorkspaceType};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
use crate::product::logical_codebase::{
    ProviderTrustPrecondition, ProviderTrustPreparationResult,
};
use crate::product::workspace_engine::conversational_gate::ConfirmationTrustContext;
use crate::web::workspace_session::WorkspaceSessionManager;

use super::single_candidate_lc_admission::AliasLcGatewayExtras;

/// 批 2 P3 夹具对齐：恒 Ready 的确认 trust precondition（生产注入
/// home-backed precondition；这里只对齐「上下文在场」的装配面，不掩盖
/// trust 停等语义——后者由 workspace_engine 侧 b2_p3_* 测试覆盖）。
struct AlwaysReadyTrustPrecondition;

impl ProviderTrustPrecondition for AlwaysReadyTrustPrecondition {
    fn ensure_ready(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _lc_id: &str,
        _canonical_root: &std::path::Path,
        _providers: &[crate::product::models::provider::ProviderName],
        _scenario: crate::product::logical_codebase::provider_trust::ProviderTrustScenario,
    ) -> ProviderTrustPreparationResult {
        ProviderTrustPreparationResult::Ready {
            registrations: Vec::new(),
        }
    }
}

struct RecordingOutputProvider {
    output: String,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RecordingOutputProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.inputs.send(input);
        provider_session_with_output(self.output.clone()).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        unreachable!("workspace provider-run tests use start")
    }
}

async fn provider_session_with_output(
    output: String,
) -> Result<ProviderSession, ProviderAdapterError> {
    let (event_tx, event_rx) = mpsc::channel(4);
    let (command_tx, _command_rx) = mpsc::channel(1);
    tokio::spawn(async move {
        let _ = event_tx
            .send(ProviderEvent::Completed(ProviderCompletion::plain(
                output, None,
            )))
            .await;
    });
    Ok(ProviderSession {
        native_session_id: None,
        events: event_rx,
        commands: command_tx,
    })
}

pub(super) struct ProviderRunFixture {
    root: tempfile::TempDir,
    repository_root: tempfile::TempDir,
    /// Task 4（路 C）：多成员 LogicalAlias 夹具的附加成员 checkout——仅持有
    /// 保活（TempDir Drop 清理），成员 0 沿用 `repository_root`。
    #[allow(dead_code)]
    extra_member_roots: Vec<tempfile::TempDir>,
    pub(super) app_paths: ProductAppPaths,
    pub(super) lifecycle: LifecycleStore,
    pub(super) record: WorkspaceSessionRecord,
    pub(super) engine: Arc<Mutex<WorkspaceEngine>>,
    pub(super) engine_tx: mpsc::Sender<EngineEvent>,
    pub(super) manager: Arc<WorkspaceSessionManager>,
    pub(super) workspace_runs: WorkspaceRunRegistry,
    pub(super) story_id: String,
    pub(super) design_id: String,
}

/// 单成员/legacy 夹具的成员规则默认正文（与 C-1 既有夹具字节一致）。
const DEFAULT_MEMBER_LANGUAGE_RULES: &str =
    "## 语言规则\n\n- **必须使用中文** - 所有响应、解释、注释和文档必须使用中文。\n";

/// LC 夹具成员的 `.claude/rules/language.md` 材料状态（Task 4 路 C SC 消费
/// 点矩阵：聚焦成员/聚合根/缺失/目录/非法 UTF-8）。
#[derive(Clone)]
pub(super) enum MemberRuleMaterial {
    /// 写入指定正文。
    Present(String),
    /// 文件缺失。
    Absent,
    /// `language.md` 路径是目录（目录错误类）。
    Directory,
    /// 文件存在但非法 UTF-8。
    InvalidUtf8,
}

/// LC 夹具 focus 形态（plan 会话路由面 target 解析输入）。
pub(super) enum LogicalAliasFocus {
    /// focus=首成员：多 target 唯一 focus → 解析为首成员 checkout；单成员
    /// 等价 focus 空（唯一 target，与既有夹具一致）。
    FirstMember,
    /// focus=全员：多 target 无唯一 focus → 聚合根视图。
    NonUnique,
}

/// LC 夹具规格：逐成员材料 + 聚合根规则正文 + focus 形态。
struct LogicalAliasSpec {
    members: Vec<MemberRuleMaterial>,
    root_rules: Option<String>,
    focus: LogicalAliasFocus,
    gateway_author: Arc<dyn StreamingProviderAdapter>,
    /// 6.1（方案 A 附带）：gateway 组装覆盖项（只读 trust source 隔离
    /// home／dialect capability source／可控 availability gate）；既有
    /// 调用方恒 `Default`（零覆盖，行为不变）。
    gateway_extras: super::single_candidate_lc_admission::AliasLcGatewayExtras,
}

/// 夹具路由形态：Legacy=单仓物理仓；LogicalAlias=legacy 别名逻辑代码库
/// （成员 0 checkout 锚定 `repository_root`，其余成员在其同级目录——C-1
/// admission 预检与 Task 4 路 C SC 消费点测试用）。
enum FixtureRoute {
    Legacy,
    LogicalAlias(LogicalAliasSpec),
    /// T6 fix-loop r2：LogicalAlias 拓扑 + Story 工作区会话（指定 author
    /// provider）——通用 Author 链（StartGeneration → Author run →
    /// drive_author_provider_session_via_gateway）验收夹具。
    LogicalAliasStory(LogicalAliasSpec, ProviderName),
}

impl ProviderRunFixture {
    pub(super) fn new(flow_kind: WorkItemPlanFlowKind) -> Self {
        // 既有语义：engine 事件接收端在 fixture 返回前释放，engine 侧 `send` 立即
        // 失败且不阻塞。需要观测 engine 事件的用例改用 `new_with_engine_rx`。
        let (engine_tx, engine_rx) = mpsc::channel(64);
        let fixture = Self::build_routed(flow_kind, FixtureRoute::Legacy, engine_tx);
        drop(engine_rx);
        fixture
    }

    /// C-1：LC（legacy 别名逻辑代码库）单成员 SingleCandidate 夹具——成员
    /// checkout 即 `repository_root`，engine 注入 gateway。
    /// `write_member_language_rules=false` 构造成员缺
    /// `.claude/rules/language.md` 的 admission waiting 场景；
    /// `gateway_author` 注册进 gateway registry（LC author 经 gateway 启动）。
    /// 权威记录登记细节见 `single_candidate_lc_admission.rs`。
    pub(super) fn new_logical(
        write_member_language_rules: bool,
        gateway_author: Arc<dyn StreamingProviderAdapter>,
    ) -> Self {
        Self::new_logical_with_gateway_extras(
            write_member_language_rules,
            gateway_author,
            AliasLcGatewayExtras::default(),
        )
    }

    /// 6.1（方案 A 附带）：`new_logical` 的 gateway 覆盖项变体——trust
    /// 首次失败只读指导用例注入隔离 home 只读 trust source／dialect
    /// capability source／可控 availability gate。
    pub(super) fn new_logical_with_gateway_extras(
        write_member_language_rules: bool,
        gateway_author: Arc<dyn StreamingProviderAdapter>,
        gateway_extras: AliasLcGatewayExtras,
    ) -> Self {
        Self::new_logical_multi_with_gateway_extras(
            vec![if write_member_language_rules {
                MemberRuleMaterial::Present(DEFAULT_MEMBER_LANGUAGE_RULES.to_string())
            } else {
                MemberRuleMaterial::Absent
            }],
            None,
            LogicalAliasFocus::FirstMember,
            gateway_author,
            gateway_extras,
        )
    }

    /// Task 4（路 C，REQ-REG-12）：多成员 LC 夹具——`members` 顺序即 ordinal
    /// （成员 0 是 `session.repository_path` 锚定的聚焦成员，其余成员 checkout
    /// 在其同级目录）；`root_rules` 写聚合根 `.claude/rules/language.md`
    /// （聚合根视图消费用，None=不写）；`focus` 决定 plan 路由 target 解析
    /// 形态（唯一 focus→成员 checkout；无唯一 focus→聚合根视图）。
    pub(super) fn new_logical_multi(
        members: Vec<MemberRuleMaterial>,
        root_rules: Option<String>,
        focus: LogicalAliasFocus,
        gateway_author: Arc<dyn StreamingProviderAdapter>,
    ) -> Self {
        Self::new_logical_multi_with_gateway_extras(
            members,
            root_rules,
            focus,
            gateway_author,
            AliasLcGatewayExtras::default(),
        )
    }

    /// 6.1（方案 A 附带）：`new_logical_multi` 的 gateway 覆盖项变体。
    pub(super) fn new_logical_multi_with_gateway_extras(
        members: Vec<MemberRuleMaterial>,
        root_rules: Option<String>,
        focus: LogicalAliasFocus,
        gateway_author: Arc<dyn StreamingProviderAdapter>,
        gateway_extras: AliasLcGatewayExtras,
    ) -> Self {
        assert!(
            !members.is_empty(),
            "logical alias fixture needs at least one member"
        );
        let (engine_tx, engine_rx) = mpsc::channel(64);
        let fixture = Self::build_routed(
            WorkItemPlanFlowKind::SingleCandidate,
            FixtureRoute::LogicalAlias(LogicalAliasSpec {
                members,
                root_rules,
                focus,
                gateway_author,
                gateway_extras,
            }),
            engine_tx,
        );
        drop(engine_rx);
        fixture
    }

    /// 6.1 fix-loop（owner 验收回归）：多仓 WorkItemPlan（Legacy flow）
    /// + LogicalAlias 路由 + gateway 覆盖项，且保留 engine 事件接收端
    /// （观察 outline author 经 gateway spawn 失败的 `EngineEvent::Error`
    /// 上浮面）。调用方 MUST 持续 drain engine_rx。
    pub(super) fn new_logical_legacy_with_gateway_extras_and_engine_rx(
        write_member_language_rules: bool,
        gateway_author: Arc<dyn StreamingProviderAdapter>,
        gateway_extras: AliasLcGatewayExtras,
    ) -> (Self, mpsc::Receiver<EngineEvent>) {
        let (engine_tx, engine_rx) = mpsc::channel(64);
        let fixture = Self::build_routed(
            WorkItemPlanFlowKind::Legacy,
            FixtureRoute::LogicalAlias(LogicalAliasSpec {
                members: vec![if write_member_language_rules {
                    MemberRuleMaterial::Present(DEFAULT_MEMBER_LANGUAGE_RULES.to_string())
                } else {
                    MemberRuleMaterial::Absent
                }],
                root_rules: None,
                focus: LogicalAliasFocus::FirstMember,
                gateway_author,
                gateway_extras,
            }),
            engine_tx,
        );
        (fixture, engine_rx)
    }

    /// T6 fix-loop r2（owner 现场回归）：LogicalAlias 拓扑 + **Story 工作区
    /// 会话**（author 由参数指定，通常 KimiCode）+ gateway 覆盖项，保留
    /// engine 事件接收端——观察 Story 首生成经 gateway spawn 失败的
    /// `EngineEvent::Error` 上浮面（StartGeneration → 通用 Author run →
    /// `drive_author_provider_session_via_gateway`）。调用方 MUST 持续
    /// drain engine_rx。
    pub(super) fn new_logical_story_with_gateway_extras_and_engine_rx(
        gateway_author: Arc<dyn StreamingProviderAdapter>,
        gateway_extras: AliasLcGatewayExtras,
        author_provider: ProviderName,
    ) -> (Self, mpsc::Receiver<EngineEvent>) {
        let (engine_tx, engine_rx) = mpsc::channel(64);
        let fixture = Self::build_routed(
            WorkItemPlanFlowKind::Legacy,
            FixtureRoute::LogicalAliasStory(
                LogicalAliasSpec {
                    members: vec![MemberRuleMaterial::Present(
                        DEFAULT_MEMBER_LANGUAGE_RULES.to_string(),
                    )],
                    root_rules: None,
                    focus: LogicalAliasFocus::FirstMember,
                    gateway_author,
                    gateway_extras,
                },
                author_provider,
            ),
            engine_tx,
        );
        (fixture, engine_rx)
    }

    /// 保留 engine 事件接收端：仅用于必须观测内部接力事件（如
    /// `ProviderRunRequested{WorkItemPlanSingleCandidateAuthor}`）的用例。调用方
    /// MUST 持续 drain，否则 engine 事件发送端在缓冲区满后阻塞。
    pub(super) fn new_with_engine_rx(
        flow_kind: WorkItemPlanFlowKind,
    ) -> (Self, mpsc::Receiver<EngineEvent>) {
        let (engine_tx, engine_rx) = mpsc::channel(64);
        (Self::build(flow_kind, engine_tx), engine_rx)
    }

    pub(super) fn root_path(&self) -> std::path::PathBuf {
        self.root.path().to_path_buf()
    }

    /// C-1：LC 成员 checkout 根（=repository_root），供测试写入/删除
    /// `.claude/rules/language.md`。
    pub(super) fn member_checkout_root(&self) -> std::path::PathBuf {
        self.repository_root.path().to_path_buf()
    }

    fn build(flow_kind: WorkItemPlanFlowKind, engine_tx: mpsc::Sender<EngineEvent>) -> Self {
        Self::build_routed(flow_kind, FixtureRoute::Legacy, engine_tx)
    }

    fn build_routed(
        flow_kind: WorkItemPlanFlowKind,
        route: FixtureRoute,
        engine_tx: mpsc::Sender<EngineEvent>,
    ) -> Self {
        let root = tempfile::tempdir().expect("temporary workspace root");
        // LogicalAlias：成员 checkout 必须是聚合根的直接子目录（聚合索引布局
        // 校验），故各成员 checkout 落在 root/aggregate-root/ 之下；成员 0 沿用
        // `repository_root`（session.repository_path 锚）。
        let repository_root = match &route {
            FixtureRoute::Legacy => tempfile::tempdir().expect("temporary repository root"),
            FixtureRoute::LogicalAlias(_) | FixtureRoute::LogicalAliasStory(..) => {
                let aggregate_root = root.path().join("aggregate-root");
                std::fs::create_dir_all(&aggregate_root).expect("create aggregate root directory");
                tempfile::tempdir_in(&aggregate_root)
                    .expect("temporary member checkout under aggregate root")
            }
        };
        // Task 4（路 C）：逐成员按矩阵落 `.claude/rules/language.md` 材料；
        // Legacy 恒默认正文（与 C-1 夹具字节一致）。
        let member_materials: Vec<MemberRuleMaterial> = match &route {
            FixtureRoute::Legacy => {
                vec![MemberRuleMaterial::Present(
                    DEFAULT_MEMBER_LANGUAGE_RULES.to_string(),
                )]
            }
            FixtureRoute::LogicalAlias(spec) | FixtureRoute::LogicalAliasStory(spec, _) => {
                spec.members.clone()
            }
        };
        apply_member_rule_material(repository_root.path(), &member_materials[0]);
        // REQ-PIB-02：夹具对齐生产不变量（main 分支+初始提交，裸目录无 git
        // 仓库会令基线解析 fail-closed，provider run 起步即终止）。
        super::init_ws_test_git_repo(repository_root.path());
        let mut extra_member_roots = Vec::new();
        for material in &member_materials[1..] {
            let member_root = tempfile::tempdir_in(
                repository_root
                    .path()
                    .parent()
                    .expect("member checkouts live under the aggregate root"),
            )
            .expect("extra member checkout under aggregate root");
            apply_member_rule_material(member_root.path(), material);
            super::init_ws_test_git_repo(member_root.path());
            extra_member_roots.push(member_root);
        }
        if let (FixtureRoute::LogicalAlias(spec) | FixtureRoute::LogicalAliasStory(spec, _)) =
            &route
            && let Some(root_rules) = spec.root_rules.as_ref()
        {
            let aggregate_root = repository_root
                .path()
                .parent()
                .expect("logical alias fixture lives under the aggregate root");
            std::fs::create_dir_all(aggregate_root.join(".claude/rules"))
                .expect("create aggregate root rules directory");
            std::fs::write(aggregate_root.join(".claude/rules/language.md"), root_rules)
                .expect("write aggregate root language rules");
        }
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        seed_legacy_project(&app_paths);
        let repository = RepositoryStore::new(app_paths.clone())
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "Provider run fixture repository".to_string(),
                path: repository_root.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: format!("provider-run-fixture-{flow_kind:?}"),
            })
            .expect("create repository");
        let extra_repositories = extra_member_roots
            .iter()
            .enumerate()
            .map(|(index, member_root)| {
                RepositoryStore::new(app_paths.clone())
                    .create(CreateRepositoryInput {
                        project_id: "project_0001".to_string(),
                        name: format!("Provider run fixture member repository {index}"),
                        path: member_root.path().to_path_buf(),
                        default_policy_preset: None,
                        default_provider_mode: None,
                        idempotency_key: format!(
                            "provider-run-fixture-{flow_kind:?}-member-{index}"
                        ),
                    })
                    .expect("create extra member repository")
            })
            .collect::<Vec<_>>();
        // C-1：LogicalAlias 路由的 issue 归属用确定性 legacy 别名 id（登记前后
        // 同值）；LC 权威记录在 issue 之后登记——selection 写入
        // issues/issue_0001/ 会占用顺序 id 目录名，必须让 issue 先落 issue_0001。
        let alias_lc_id = match &route {
            FixtureRoute::Legacy => None,
            FixtureRoute::LogicalAlias(_) | FixtureRoute::LogicalAliasStory(..) => Some(
                crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001"),
            ),
        };
        IssueStore::new(app_paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                logical_codebase_id: alias_lc_id,
                title: "Provider run flow dispatch".to_string(),
                description: Some("durable flow_kind must select one provider chain".to_string()),
                change_id: None,
                base_branch: None,
            })
            .expect("create issue");
        // C-1：LogicalAlias 路由在物理仓之上登记 LC 权威记录（record/manifest/
        // member/checkout/selection/policy + gateway），并把兼容投影升级为已登记
        // 形态（strict 三层身份解析要求 logical_repository_id/primary_checkout_id/
        // identity_schema_version 对齐）。
        let logical = match &route {
            FixtureRoute::Legacy => None,
            FixtureRoute::LogicalAlias(spec) | FixtureRoute::LogicalAliasStory(spec, _) => {
                let member_roots = std::iter::once(repository_root.path().to_path_buf())
                    .chain(
                        extra_member_roots
                            .iter()
                            .map(|member_root| member_root.path().to_path_buf()),
                    )
                    .collect::<Vec<_>>();
                let physical_repositories = std::iter::once(repository.clone())
                    .chain(extra_repositories.iter().cloned())
                    .collect::<Vec<_>>();
                let registered = super::single_candidate_lc_admission::
                    register_alias_logical_codebase_with_extras(
                        &app_paths,
                        &member_roots,
                        &physical_repositories,
                        &spec.focus,
                        Arc::clone(&spec.gateway_author),
                        spec.gateway_extras.clone(),
                    );
                let projections = physical_repositories
                    .iter()
                    .cloned()
                    .zip(&registered.member_ids)
                    .zip(&registered.checkout_ids)
                    .map(|((mut record, member_id), checkout_id)| {
                        record.logical_repository_id = Some(*member_id);
                        record.primary_checkout_id = Some(*checkout_id);
                        record.identity_schema_version = 1;
                        record
                    })
                    .collect::<Vec<_>>();
                let repos_json = app_paths.project_root("project_0001").join("repos.json");
                std::fs::write(
                    &repos_json,
                    serde_json::to_vec_pretty(&projections)
                        .expect("serialize registered repository projections"),
                )
                .expect("rewrite repos.json with registered projections");
                Some(registered)
            }
        };
        let lifecycle = LifecycleStore::new(app_paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository.id.clone(),
                title: "Provider run Story".to_string(),
                aggregate_codebase: None,
            })
            .expect("create story");
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "Provider run Design".to_string(),
                aggregate_codebase: None,
            })
            .expect("create design");
        let plan = lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some("plan_0001".to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
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
            .expect("create plan");
        let session_id = format!(
            "workspace_session_{}_provider_run_{}",
            match flow_kind {
                WorkItemPlanFlowKind::Legacy => "legacy",
                WorkItemPlanFlowKind::SingleCandidate => "single_candidate",
            },
            uuid::Uuid::new_v4().simple(),
        );
        let record = lifecycle
            .create_workspace_session_with_id(
                match &route {
                    // T6 fix-loop r2：Story 工作区会话——entity 是 story、无
                    // WIP options，author 由路由变体指定（通用 Author 链）。
                    FixtureRoute::LogicalAliasStory(_, story_author) => {
                        CreateWorkspaceSessionInput {
                            project_id: "project_0001".to_string(),
                            issue_id: "issue_0001".to_string(),
                            entity_id: story.id.clone(),
                            workspace_type: WorkspaceType::Story,
                            author_provider: story_author.clone(),
                            reviewer_provider: Some(ProviderName::Codex),

                            review_rounds: 0,
                            superpowers_enabled: false,
                            openspec_enabled: false,
                            work_item_plan_options: None,
                        }
                    }
                    FixtureRoute::Legacy | FixtureRoute::LogicalAlias(_) => {
                        CreateWorkspaceSessionInput {
                            project_id: "project_0001".to_string(),
                            issue_id: "issue_0001".to_string(),
                            entity_id: plan.id,
                            workspace_type: WorkspaceType::WorkItemPlan,
                            author_provider: ProviderName::ClaudeCode,
                            reviewer_provider: Some(ProviderName::Codex),

                            review_rounds: 0,
                            superpowers_enabled: false,
                            openspec_enabled: false,
                            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                                flow_kind,
                                run_policy: RunPolicy::Interactive,
                                rollout_snapshot: flow_kind
                                    == WorkItemPlanFlowKind::SingleCandidate,
                            }),
                        }
                    }
                },
                session_id,
            )
            .expect("create workspace session");
        let session_root = app_paths
            .issue_root("project_0001", "issue_0001")
            .join("workspace-sessions");
        assert!(
            session_root.join(format!("{}.json", record.id)).exists(),
            "created session file must exist at {}",
            session_root.display(),
        );
        assert_eq!(
            lifecycle
                .get_workspace_session(&record.id)
                .unwrap_or_else(|error| {
                    panic!(
                        "new fixture workspace session must be discoverable ({error:?}); root={}; entries={:?}",
                        session_root.display(),
                        std::fs::read_dir(&session_root)
                            .ok()
                            .into_iter()
                            .flatten()
                            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                            .collect::<Vec<_>>(),
                    )
                })
                .id,
            record.id,
        );
        let mut session = WorkspaceSession::from_record(record.clone());
        session.repository_path = Some(repository_root.path().to_path_buf());
        let engine = WorkspaceEngine::new_persistent(
            Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
            lifecycle.clone(),
            engine_tx.clone(),
            session,
        );
        // C-1：LogicalAlias 会话注入 gateway（与生产 manager 的注入谓词同源）。
        // 批 2 P3（REQ-WIGA-02/12）夹具对齐：生产 manager 在 gateway 装配段
        // 同步注入确认 trust 冻结上下文（T4.3/D2，缺 precondition 才
        // fail-closed 延后）；本夹具此前只注入 gateway，自动 compile 臂的
        // P3 trust 门会把「缺上下文」的 LC 会话停等在 compile 之前。此处
        // 注入恒 Ready 的 precondition 对齐生产装配面（冻结 issue 原始
        // alias LC 归属＋聚合根）；trust 停等语义由 workspace_engine 侧
        // b2_p3_* 测试覆盖。
        let engine = match logical {
            Some(registered) => {
                let alias_lc_id =
                    crate::product::logical_codebase::store::legacy_logical_codebase_id(
                        "project_0001",
                    );
                let aggregate_root = repository_root
                    .path()
                    .parent()
                    .expect("fixture aggregate root")
                    .to_path_buf();
                engine
                    .with_logical_provider_gateway(registered.gateway)
                    .with_confirmation_trust_context(ConfirmationTrustContext {
                        precondition: std::sync::Arc::new(AlwaysReadyTrustPrecondition),
                        issue_lc_id: Some(alias_lc_id),
                        frozen_root: aggregate_root,
                    })
            }
            None => engine,
        };
        let engine = Arc::new(Mutex::new(engine));
        assert_eq!(
            lifecycle
                .get_workspace_session(&record.id)
                .expect("persistent engine construction must retain workspace session")
                .id,
            record.id,
        );
        Self {
            root,
            repository_root,
            extra_member_roots,
            app_paths: app_paths.clone(),
            lifecycle,
            record: record.clone(),
            engine: engine.clone(),
            engine_tx,
            manager: WorkspaceSessionManager::test_fixture_with_parts(
                &record.id,
                engine.clone(),
                Arc::new(ProviderRegistry::new()),
                app_paths.clone(),
                record.clone(),
            ),
            workspace_runs: WorkspaceRunRegistry::default(),
            story_id: story.id,
            design_id: design.id,
        }
    }
}

/// Task 4（路 C）：按矩阵状态落成员 `.claude/rules/language.md` 材料
/// （Present/Absent/Directory/InvalidUtf8），目录骨架与其他夹具一致。
fn apply_member_rule_material(member_root: &std::path::Path, material: &MemberRuleMaterial) {
    let rules_dir = member_root.join(".claude/rules");
    std::fs::create_dir_all(&rules_dir).expect("create language rules directory");
    match material {
        MemberRuleMaterial::Present(content) => {
            std::fs::write(rules_dir.join("language.md"), content)
                .expect("write member language rules");
        }
        MemberRuleMaterial::Absent => {}
        MemberRuleMaterial::Directory => {
            std::fs::create_dir_all(rules_dir.join("language.md"))
                .expect("create directory at member language.md path");
        }
        MemberRuleMaterial::InvalidUtf8 => {
            std::fs::write(
                rules_dir.join("language.md"),
                b"\xef\xbb\xbf\xff\xff broken utf8 \xff",
            )
            .expect("write invalid utf-8 member language rules");
        }
    }
}

fn legacy_outline_output(story_id: &str, design_id: &str) -> String {
    let output = serde_json::json!({
        "outline": {
            "id": "outline_001",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "source_story_spec_ids": [story_id],
            "source_design_spec_ids": [design_id],
            "strategy_summary": "one backend owner",
            "work_item_outlines": [{
                "outline_id": "outline_backend",
                "logical_work_item_id": "WI-001",
                "title": "Backend API",
                "kind": "backend",
                "goal": "provide an API",
                "scope": ["src/backend/**"],
                "non_goals": ["frontend"],
                "estimated_context_tokens": 12000,
                "session_fit": "fits_single_agent_session",
                "source_story_spec_ids": [story_id],
                "source_design_spec_ids": [design_id],
                "exclusive_write_scopes": ["src/backend/**"],
                "forbidden_write_scopes": ["web/**"],
                "depends_on": [],
                "verification_intent": ["cargo test --locked --lib backend"],
                "trusted_verification_commands": [{
                    "command": "cargo test --locked --lib backend",
                    "cwd": ".",
                    "purpose": "backend test",
                    "source_ref": "design#verification"
                }],
                "handoff_notes": "provide API contract"
            }],
            "risks": [],
            "handoff_strategy": "one owner",
            "status": "draft"
        },
        "context_blockers": []
    });
    structured_output_sentinel("legacy-flow", &output)
}

pub(super) fn single_candidate_context(
    fixture: &ProviderRunFixture,
    provider: Arc<dyn StreamingProviderAdapter>,
) -> (WorkspaceInboundContext, mpsc::Receiver<OutboundControl>) {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    let (outbound_tx, outbound_rx) = mpsc::channel(64);
    (
        WorkspaceInboundContext {
            app_state: WebAppState::new(
                fixture.root.path().to_path_buf(),
                crate::web::runtime::WebRuntime::new_fake(fixture.root.path().to_path_buf()),
            ),
            engine: fixture.engine.clone(),
            run_context,
            outbound_tx,
            session_id: fixture.record.id.clone(),
        },
        outbound_rx,
    )
}

pub(super) fn single_candidate_markdown(story_id: &str, design_id: &str) -> String {
    format!(
        "# Work Item Plan\n\
         ## Work Item WI-001: Backend API\n\n\
         ### Identity\n- schema_version: 1\n- logical_work_item_id: WI-001\n- title: Backend API\n- kind: backend\n\n\
         ### Goal\n- summary: WHEN a request arrives THE SYSTEM SHALL return the planned API response.\n\n\
         ### Non Goals\n- non_goals: Frontend rendering is out of scope.\n\n\
         ### Dependencies\n- depends_on: []\n\n\
         ### Inputs\n\n\
         ### Outputs\n- contract_id: contract.backend-api\n- capabilities: api.backend.read\n\n\
         ### Tasks\n- task_id: TASK-001\n- statement: WHEN a request arrives THE SYSTEM SHALL return the planned API response.\n- requirement_refs: REQ-001\n- done_when_refs: AC-001\n\n\
         ### Write Policy\n- exclusive_scopes: src/backend/**\n- forbidden_scopes: web/**\n\n\
         ### Acceptance Criteria\n- criterion_id: AC-001\n- statement: WHEN a request arrives THE SYSTEM SHALL expose the backend API response.\n- required_evidence: source_diff\n- required_evidence: manual_check\n\n\
         ### Verification\n- check_id: CHECK-001\n- manual_instruction: Inspect the backend API response manually.\n- required: true\n- non_zero_test_execution_required: false\n\n\
         ### Handoff Schema\n- required_fields: commit_sha\n- provided_contract_refs: []\n- reviewer_check_refs: AC-001\n\n\
         ### Blockers\n- reason_code: no_trusted_command_catalog\n- route: operational_gate\n- target_contract_refs: contract.backend-api\n\n\
         ### Traceability\n- source_type: design_spec\n- source_id: {design_id}\n- requirement_id: REQ-001\n\n\
         ### Notes\nGenerated from Story {story_id}.\n\n\
         ### Rationale\nA single backend item owns the API boundary.\n"
    )
}

fn single_candidate_markdown_with_command(
    story_id: &str,
    design_id: &str,
    command: &str,
) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- check_id: CHECK-001\n- manual_instruction: Inspect the backend API response manually.",
        &format!(
            "- check_id: CHECK-001\n- command: {command}\n- manual_instruction: Inspect the backend API response manually."
        ),
        1,
    )
}
fn single_candidate_markdown_with_duplicate_command(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        "- check_id: CHECK-001\n- manual_instruction: Inspect the backend API response manually.",
        "- check_id: CHECK-001\n- command: cargo test --locked --lib backend_api\n- manual_instruction: Inspect the backend API response manually.\n- required: true\n- non_zero_test_execution_required: false\n- check_id: CHECK-002\n- command: cargo test --locked --lib backend_api\n- manual_instruction: Confirm the backend API response manually.\n- required: true\n- non_zero_test_execution_required: false",
        1,
    )
}

/// Task 3.5 E2E 现场（session_0004 node_004/node_006，真实 claude_code 稳定复现）：
/// author 输出「多 source_type + 批量 requirement_id」的 Traceability 形态——
/// parse 全过、lower 在首个 source flush 时报
/// `lowering_error:traceability 缺少 requirement_id`。
fn single_candidate_markdown_with_batched_traceability(story_id: &str, design_id: &str) -> String {
    single_candidate_markdown(story_id, design_id).replacen(
        &format!(
            "### Traceability\n- source_type: design_spec\n- source_id: {design_id}\n- requirement_id: REQ-001"
        ),
        &format!(
            "### Traceability\n- source_type: story_spec\n- source_id: {story_id}\n- source_type: design_spec\n- source_id: {design_id}\n- requirement_id: REQ-001"
        ),
        1,
    )
}

#[tokio::test]
async fn single_candidate_repairs_duplicate_trusted_command_before_terminal_failure() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output =
        single_candidate_markdown_with_duplicate_command(&fixture.story_id, &fixture.design_id);
    let provider = Arc::new(RecordingOutputProvider {
        output,
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let _full_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("full author provider must receive input")
        .expect("full author provider input");
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "mechanically repairable duplicate command must not trigger a provider re-drive"
    );

    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload repaired single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
        "duplicate trusted command must reach the engine approval path instead of terminal failure"
    );

    let source_ref = durable
        .work_item_plan_source_revision_ref
        .as_deref()
        .expect("repaired source revision ref");
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: durable.project_id.clone(),
        issue_id: durable.issue_id.clone(),
        plan_id: durable.entity_id.clone(),
    };
    let source_store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    );
    let repaired_source = source_store
        .get_source_revision(&scope, source_ref)
        .expect("repaired source must be persisted")
        .source;
    assert_eq!(
        repaired_source
            .matches("- command: cargo test --locked --lib backend_api")
            .count(),
        1,
        "engine authoritative convergence must persist source with one trusted command"
    );

    while let Ok(Some(outbound)) =
        tokio::time::timeout(std::time::Duration::from_millis(20), outbound_rx.recv()).await
    {
        if let OutboundControl::Text(json) = outbound {
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            assert_ne!(
                value["type"], "error",
                "repair path must not emit terminal error"
            );
            assert!(
                !value["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("compile markdown source failed")
            );
        }
    }
}

#[tokio::test]
async fn single_candidate_mixed_duplicate_and_missing_section_uses_one_teaching_reredrive() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output =
        single_candidate_markdown_with_duplicate_command(&fixture.story_id, &fixture.design_id)
            .replacen("### Inputs\n\n", "", 1);
    let provider = Arc::new(RecordingOutputProvider {
        output,
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let _full_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("full author provider must receive input")
        .expect("full author provider input");
    let reredrive_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("mixed diagnostics must receive one teaching re-drive")
        .expect("teaching re-drive input");
    assert!(
        reredrive_input
            .prompt
            .contains("立即输出完整 work-item-plan markdown source")
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "mixed diagnostics must consume at most one teaching re-drive"
    );

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let outbound = outbound_rx
                .recv()
                .await
                .expect("mixed compile failure outbound");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value;
            }
        }
    })
    .await
    .expect("mixed compile failure error");
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("compile markdown source failed")
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

/// SG6a.2（原 Task 3.5 现场回归网迁移）：lowering 结构错误（Traceability
/// 多源批量 requirement_id 的「缺席类」）不再有教学重驱资格——按
/// REQ-SG-05 SC 初轮行与 F1.2 typed 口径，资格只认 parse 语法类共享码
/// 白名单，message 措辞（缺少/缺失）不构成资格源（原
/// `is_lowering_absence_message` 分流已删除）。本用例锁新契约：零重驱、
/// 一轮确定性终态失败、失败原因原文回显。
#[tokio::test]
async fn single_candidate_batched_traceability_lowering_error_fails_terminal_without_reredrive() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output =
        single_candidate_markdown_with_batched_traceability(&fixture.story_id, &fixture.design_id);
    let provider = Arc::new(RecordingOutputProvider {
        output,
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let _full_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("full author provider must receive input")
        .expect("full author provider input");
    // 零重驱：lowering 类（缺席/值域统一）资格取消——不再有第二次
    // provider 输入（初轮机会纪律归 L1 同池，非独立教学池）。
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "lowering_error must not trigger any teaching re-drive (eligibility is the parse-code whitelist; message wording is not a license source)"
    );

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let outbound = outbound_rx.recv().await.expect("lowering failure outbound");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value;
            }
        }
    })
    .await
    .expect("lowering failure error");
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("compile markdown source failed")
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

#[tokio::test]
async fn legacy_provider_run_uses_outline_builder_and_legacy_parser_only() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::Legacy);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output = legacy_outline_output(&fixture.story_id, &fixture.design_id);
    let provider = Arc::new(RecordingOutputProvider {
        output,
        inputs: input_tx,
    });
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let mut run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        fixture.engine.clone(),
        fixture.workspace_runs.clone(),
        fixture.record.id.clone(),
        fixture.app_paths.clone(),
        fixture.record.clone(),
    );
    run_context.manager = fixture.manager.clone();
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);
    let context = WorkspaceInboundContext {
        app_state: WebAppState::new(
            fixture.root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(fixture.root.path().to_path_buf()),
        ),
        engine: fixture.engine.clone(),
        run_context,
        outbound_tx,
        session_id: fixture.record.id.clone(),
    };

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let input = match tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("legacy provider must receive input")
    {
        Some(input) => input,
        None => {
            let engine = fixture.engine.lock().await;
            let mut outbound = Vec::new();
            while let Ok(Some(control)) =
                tokio::time::timeout(std::time::Duration::from_millis(20), outbound_rx.recv()).await
            {
                outbound.push(format!("{control:?}"));
            }
            panic!(
                "legacy provider input channel closed; stage={:?}, active={:?}, outbound={outbound:?}",
                engine.session().stage,
                engine.active_run_id(),
            );
        }
    };
    assert!(input.prompt.contains("WorkItemPlan Outline"));
    assert!(!input.prompt.contains("[markdown_grammar]"));
    // F3 修复轮 P1-3：WorkItemPlan author（WorkItemSplitter）是策略角色——真实
    // web 启动路径必须在 provider.start 前绑定 run-bound durable sink（Legacy
    // 直连不得 policy+缺 sink 运行时 fail-closed）。
    assert!(
        input.tool_policy.is_some(),
        "WorkItemPlan author input must carry the deny policy"
    );
    assert!(
        input.audit_sink.is_some(),
        "WorkItemPlan author legacy web path must carry the run-bound durable audit sink"
    );
    wait_for_stage(&fixture.engine, WorkspaceStage::AuthorConfirm).await;
    assert_eq!(
        work_item_plan_parser_paths_for_session(&fixture.record.id),
        vec!["legacy_outline"],
        "legacy run must not reach the markdown compiler path",
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload legacy session");
    assert!(durable.work_item_plan_source_revision_ref.is_none());
}

#[tokio::test]
async fn single_candidate_provider_run_uses_markdown_builder_and_source_store_only() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let output = single_candidate_markdown(&fixture.story_id, &fixture.design_id);
    let provider = Arc::new(RecordingOutputProvider {
        output,
        inputs: input_tx,
    });
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("single-candidate full author provider must receive exactly one input")
        .expect("single-candidate full author provider input");
    assert!(full_input.prompt.contains("[markdown_grammar]"));
    assert!(full_input.prompt.contains("[routing_reference]"));
    assert!(full_input.prompt.contains("[cadence_project_rules]"));
    assert!(full_input.prompt.contains("必须使用中文"));
    assert!(full_input.prompt.contains("保持 grammar 指定的英文原样"));
    assert!(
        full_input
            .prompt
            .contains("任务拆分与验证设计遵循测试先行纪律")
    );
    assert!(full_input.prompt.contains("大范围定位优先检索工具"));
    assert!(!full_input.prompt.contains("按需查阅其中适用章节即可"));
    assert!(full_input.prompt.contains("[real_finding_few_shot]"));
    assert!(!full_input.prompt.contains("[outline_commands]"));
    assert!(!full_input.prompt.contains("<ARIA_STRUCTURED_OUTPUT"));
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "single-candidate must not invoke an outline provider before the full author"
    );
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;
    assert_eq!(
        single_candidate_generation_steps_for_session(&fixture.record.id),
        vec!["full_markdown_author", "parse_source_revision", "selector"],
        "single-candidate invocation order must stay full author → compile/source revision → internal selector diagnostic",
    );
    assert_eq!(
        work_item_plan_parser_paths_for_session(&fixture.record.id),
        vec!["single_candidate_markdown"],
        "single-candidate run must compile only the full markdown source and never reach the legacy or outline parser",
    );
    assert!(
        !fixture
            .engine
            .lock()
            .await
            .timeline_nodes
            .iter()
            .any(|node| {
                node.node_type
                    == crate::web::workspace_ws_types::TimelineNodeType::WorkItemGenerationMode
            }),
        "internal selection must not create a generation decision request node"
    );
    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Approval),
    );
    let source_ref = durable
        .work_item_plan_source_revision_ref
        .as_deref()
        .expect("source revision ref");
    let ir_ref = durable.plan_candidate_ir_ref.as_deref().expect("IR ref");
    let report_ref = durable
        .mechanical_report_ref
        .as_deref()
        .expect("mechanical report ref");
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: durable.project_id.clone(),
        issue_id: durable.issue_id.clone(),
        plan_id: durable.entity_id.clone(),
    };
    let source_store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    );
    assert_eq!(
        source_store
            .get_source_revision(&scope, source_ref)
            .expect("stored source")
            .source,
        single_candidate_markdown(&fixture.story_id, &fixture.design_id),
    );
    source_store
        .get_plan_candidate_ir(&scope, ir_ref)
        .expect("stored IR");
    source_store
        .get_mechanical_report(&scope, report_ref)
        .expect("stored mechanical report");
}

#[tokio::test]
async fn single_candidate_author_rejects_missing_language_rules_before_provider_start() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let language_rules_path = fixture
        .repository_root
        .path()
        .join(".claude/rules/language.md");
    std::fs::remove_file(&language_rules_path).expect("remove fixture language rules");
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(RecordingOutputProvider {
        output: single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let error = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let outbound = outbound_rx
                .recv()
                .await
                .expect("missing language rules must emit an error");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value;
            }
        }
    })
    .await
    .expect("missing language rules rejection");
    let message = error["message"].as_str().expect("error message");
    assert!(message.contains(&fixture.repository_root.path().display().to_string()));
    assert!(message.contains(&language_rules_path.display().to_string()));
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "missing language rules must reject before provider startup"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

#[tokio::test]
async fn single_candidate_projects_declared_verification_command_without_outline_catalog() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let markdown = single_candidate_markdown_with_command(
        &fixture.story_id,
        &fixture.design_id,
        "node --test tests/backend/",
    );
    let provider = Arc::new(RecordingOutputProvider {
        output: markdown,
        inputs: input_tx,
    });
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("full author provider must receive input")
        .expect("full author provider input");
    assert!(
        full_input
            .prompt
            .contains("Verification.command 直接声明，将按声明执行")
    );
    assert!(!full_input.prompt.contains("outline 阶段登记"));
    wait_for_stage(&fixture.engine, WorkspaceStage::HumanConfirm).await;

    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session");
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: durable.project_id.clone(),
        issue_id: durable.issue_id.clone(),
        plan_id: durable.entity_id.clone(),
    };
    let source_store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
        fixture.app_paths.clone(),
    );
    let ir = source_store
        .get_plan_candidate_ir(
            &scope,
            durable.plan_candidate_ir_ref.as_deref().expect("IR ref"),
        )
        .expect("full-plan declared command 必须通过 lowering");
    let trusted = &ir.ir.items[0].trusted_commands[0];
    assert_eq!(trusted.command, "node --test tests/backend/");
    assert_eq!(trusted.cwd, ".");
    assert_eq!(trusted.purpose, "Inspect the backend API response");
    assert!(trusted.source_ref.starts_with("plan-"));
}

#[tokio::test]
async fn single_candidate_full_plan_parse_failure_is_fatal_after_one_teaching_reredrive() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(RecordingOutputProvider {
        output: "# Work Item Plan\n\n## Work Item WI-001: malformed\n".to_string(),
        inputs: input_tx,
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("full author provider must receive input")
        .expect("full author provider input");
    assert!(
        full_input
            .prompt
            .contains("完整 `work-item-plan.md` source")
    );
    // F2-B：missing_section 类失败给恰一次教学重驱；重驱仍败则终态。
    let reredrive_input = tokio::time::timeout(std::time::Duration::from_secs(1), input_rx.recv())
        .await
        .expect("teaching re-drive must invoke the provider exactly once more")
        .expect("teaching re-drive input");
    assert!(
        reredrive_input
            .prompt
            .contains("立即输出完整 work-item-plan markdown source"),
        "re-drive prompt must carry the immediate-output teaching: {}",
        reredrive_input.prompt
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "teaching re-drive must be bounded to exactly one extra provider invocation"
    );
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let outbound = outbound_rx
                .recv()
                .await
                .expect("full-plan parse failure outbound");
            let OutboundControl::Text(json) = outbound else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&json).expect("outbound json");
            if value["type"] == "error" {
                return value;
            }
        }
    })
    .await
    .expect("full-plan parse failure error");
    let message = error["message"].as_str().expect("error message");
    assert!(message.contains("compile markdown source failed"));
    assert!(
        message.contains("first round") && message.contains("re-drive round"),
        "terminal failure must carry both rounds' diagnostics: {message}"
    );
    wait_for_single_candidate_phase(
        &fixture,
        crate::product::models::SingleCandidatePhase::Failed,
    )
    .await;
}

pub(super) async fn wait_for_single_candidate_phase(
    fixture: &ProviderRunFixture,
    expected: crate::product::models::SingleCandidatePhase,
) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if fixture
                .lifecycle
                .get_workspace_session(&fixture.record.id)
                .expect("reload session")
                .single_candidate_phase
                == Some(expected.clone())
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider run must reach expected single-candidate phase");
}

pub(super) async fn wait_for_stage(engine: &Arc<Mutex<WorkspaceEngine>>, expected: WorkspaceStage) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if engine.lock().await.session().stage == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider run must reach expected stage");
}

// 大文件守卫(>1200 行)按 include! 拆分:reredrive_and_reopen=SC compile 教学重驱/
// failed-completed 重开/迟到 run 让位/终态恢复错误用例与其专属 provider 夹具。
include!("single_candidate_provider_run/reredrive_and_reopen.rs");
// Task 2（REQ-WSC-09）：教学重驱白名单扩到 parse 语法类集合的端到端用例。
include!("single_candidate_provider_run/ears_reredrive.rs");
// B2c（Task 5.5）：SC author 真链回归（成员材料/compile target 不跟随 native
// 锚）与显式 fresh 同 reservation 重入用例（大文件守卫拆分）。
include!("single_candidate_provider_run/anchor_t17.rs");
// B1（多仓 plan 聚合视图缺口）：NonUnique focus 聚合会话真链——IR target
// 按 WI 成员归属落成员物理仓（大文件守卫拆分）。
include!("single_candidate_provider_run/aggregate_ir_target.rs");
