use super::*;

#[test]
fn only_pi_is_excluded_from_review_repair() {
    assert!(provider_allows_review_repair(&ProviderName::KimiCode));
    assert!(!provider_allows_review_repair(&ProviderName::Pi));
    assert!(provider_allows_review_repair(&ProviderName::Codex));
}

/// 诊断直通（claude×轻 握手谜团第 2 轮）：gateway 错误映射不得丢弃 adapter
/// error 的 stderr——Adapter 变体的 stderr 尾部（有界）需并入新错误的
/// details（驱动 result/WS error 消息的上游），stderr 字段同源保留。
#[test]
fn gateway_adapter_error_mapping_carries_bounded_stderr_tail() {
    let stderr_head = "NOISE_HEAD_MARKER".to_string() + &"b".repeat(700);
    let stderr = format!("{stderr_head}\nSENTINEL_GW_STDERR_TAIL");
    let inner = ProviderAdapterError::parse_error(
        "claude policy session: handshake failed: claude policy handshake cancelled",
        String::new(),
        stderr,
    );
    let mapped = map_gateway_error_to_adapter(&ProviderGatewayError::Adapter(inner));
    assert!(
        mapped.details.contains("provider_gateway_adapter"),
        "gateway 映射应保留既有 Display 前缀文案，got: {}",
        mapped.details
    );
    assert!(
        mapped.details.contains("SENTINEL_GW_STDERR_TAIL"),
        "gateway 映射应携带 adapter stderr 尾部，got: {}",
        mapped.details
    );
    assert!(
        !mapped.details.contains("NOISE_HEAD_MARKER"),
        "stderr 尾部应有界（丢头保尾），got: {}",
        mapped.details
    );
    assert!(
        mapped.stderr.contains("SENTINEL_GW_STDERR_TAIL"),
        "stderr 字段应同源保留，got: {}",
        mapped.stderr
    );
}

// ---------------------------------------------------------------------------
// T6 fix-loop r2：reviewer 首发 gateway 启动口（U3 位点）trust 分类。
// ---------------------------------------------------------------------------

/// U3 夹具 capability double：任意 provider×action 恒 Confirmed（真实
/// dialect 映射，Kimi/Codex/Claude/Pi 全支持）。
struct TrustScenarioCapabilitySource;

impl crate::product::logical_codebase::ProviderCapabilitySource for TrustScenarioCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError> {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
        let (adapter_dialect, wire_dialect) = match provider.provider_type {
            crate::product::logical_codebase::ProviderRefType::ClaudeCode => (
                crate::product::logical_codebase::policy::ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            crate::product::logical_codebase::ProviderRefType::Codex => (
                crate::product::logical_codebase::policy::ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            crate::product::logical_codebase::ProviderRefType::Pi => (
                crate::product::logical_codebase::policy::ProviderDialect::PiRpcV1,
                ProviderWireDialect::PiRpc,
            ),
            crate::product::logical_codebase::ProviderRefType::KimiCode => (
                crate::product::logical_codebase::policy::ProviderDialect::KimiAcpV1,
                ProviderWireDialect::KimiAcp,
            ),
        };
        Ok(crate::product::logical_codebase::ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.4.0".to_string(),
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability: ProviderActionCapability {
                action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: format!("projection-digest-{action:?}"),
                evidence_ref: format!("probe://{action:?}"),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        })
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError> {
        self.require_supported(provider, action)
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError> {
        self.require_supported(provider, action)
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<crate::product::logical_codebase::ProviderCapability, ProviderGatewayError> {
        self.require_supported(
            provider,
            crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
        )
    }
}

/// U3 夹具 target resolver：透传 request target。
struct TrustScenarioTargetResolver;

impl crate::product::logical_codebase::PolicyTargetResolver for TrustScenarioTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// U3 夹具 sync adapter stub。
struct TrustScenarioSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for TrustScenarioSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<
        crate::protocol::contracts::AdapterOutput,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        Ok(crate::protocol::contracts::AdapterOutput {
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
        })
    }
}

/// U3 夹具：bootstrap 政策就绪 + Kimi/Codex 隔离 home 只读 trust source
///（条目缺失）的 gateway；provider adapter 不注册（trust 拒绝先于
/// registry lookup，零启动由断言保证）。
fn reviewer_trust_gateway(
    paths: &crate::product::app_paths::ProductAppPaths,
    trust_home: &std::path::Path,
) -> std::sync::Arc<LogicalCodebaseProviderGateway> {
    let manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
        "project_0001",
        paths.root().to_path_buf(),
        vec![],
    );
    crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");
    let lc_id = "logical_codebase_0001".to_string();
    let gateway = LogicalCodebaseProviderGateway::with_audit(
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::new(paths.clone()),
        std::sync::Arc::new(TrustScenarioCapabilitySource),
        std::sync::Arc::new(TrustScenarioTargetResolver),
        std::sync::Arc::new(crate::cross_cutting::provider_registry::ProviderRegistry::new()),
        std::sync::Arc::new(TrustScenarioSyncAdapter),
        std::sync::Arc::new(crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(
            std::sync::Arc::new(TrustScenarioAlwaysHealthy),
        )),
        std::sync::Arc::new(
            crate::product::logical_codebase::provider_gateway::GatewayRunAudit::new(),
        ),
        std::fs::canonicalize(&manifest.provider_context_root).expect("authority root"),
    )
    .with_readonly_lc_facts(
        crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(paths.clone(), &lc_id),
        std::sync::Arc::new(
            crate::product::logical_codebase::provider_trust::ReadonlyProviderTrustSource::new(
                paths.clone(),
                Some(lc_id.clone()),
                vec![
                    std::sync::Arc::new(
                        crate::product::logical_codebase::provider_trust_adapters::CodexTrustAdapter::for_home(trust_home),
                    ),
                    std::sync::Arc::new(
                        crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::for_home(trust_home),
                    ),
                ],
            ),
        ),
        Some(lc_id),
    );
    std::sync::Arc::new(gateway)
}

/// U3 夹具 availability gate：恒健康。
struct TrustScenarioAlwaysHealthy;

impl crate::cross_cutting::provider_availability_gate::ProviderHealthSource
    for TrustScenarioAlwaysHealthy
{
    fn snapshot(
        &self,
    ) -> std::sync::Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot> {
        let checked_at = chrono::Utc::now();
        std::sync::Arc::new(
            crate::cross_cutting::provider_health::ProviderHealthSnapshot {
                schema_version: 1,
                generation: 1,
                checked_at,
                providers: [
                    crate::product::models::ProviderName::ClaudeCode,
                    crate::product::models::ProviderName::Codex,
                    crate::product::models::ProviderName::KimiCode,
                ]
                .into_iter()
                .map(
                    |provider| crate::cross_cutting::provider_health::ProviderHealthEntry {
                        provider,
                        command: "stub".to_string(),
                        available: true,
                        version: Some("1.0".to_string()),
                        reason_code: None,
                        reason: None,
                        checked_at,
                    },
                )
                .collect(),
            },
        )
    }

    fn degraded(&self) -> bool {
        false
    }
}

fn reviewer_trust_streaming_input(
    working_dir: std::path::PathBuf,
) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
    crate::cross_cutting::streaming_provider::StreamingProviderInput {
        provider_type: crate::protocol::contracts::ProviderType::KimiCode,
        role: crate::protocol::contracts::AdapterRole::Reviewer,
        prompt: "review the change".to_string(),
        working_dir,
        working_directory: None,
        workspace_session_id: Some("session_0001".to_string()),
        resume_provider_session_id: None,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        // W0-3 N5-1(T20a 守卫对齐):Reviewer 启动必须携带非空
        // ReviewerVerify 窗口合同(镜像生产 reviewer_verify_policy 铸造),
        // 否则网关 UnsupportedCapability 先拒、到不了 trust 复验。
        tool_policy: Some(
            crate::cross_cutting::streaming_provider::ProviderToolPolicy::reviewer_verify(
                "review_window_0001",
            ),
        ),
        audit_sink: None,
        structured_output_contract: None,
        env_vars: Default::default(),
        timeout_secs: 30,
        baseline_tree: None,
    }
}

/// T6 fix-loop r2：reviewer 首发 gateway 启动口（`start_review_session_via_
/// gateway`，U3 位点）Kimi reviewer 条目缺失 → spawn trust 拒绝以
/// `provider_trust_missing` 分类＋同源指导＋生成场景 retry_action 拒收
///（`ReviewProviderRunFailure::Start` 形态，经 finish_review_provider_run_
/// failure → EngineEvent::Error 上浮）；provider 零启动。负例：Claude
/// reviewer 无 workspace trust 面——validate/复验后错误不携带 trust 分类。
#[tokio::test]
async fn reviewer_gateway_spawn_trust_missing_classifies_with_manual_guidance() {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let gateway = reviewer_trust_gateway(&paths, trust_home.path());
    let target_dir = tempfile::tempdir().expect("review target anchor");

    let (result, trust_waiting) = start_review_session_via_gateway(
        &gateway,
        &crate::product::models::ProviderName::KimiCode,
        reviewer_trust_streaming_input(target_dir.path().to_path_buf()),
        "project_0001".to_string(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await;
    let error = match result {
        Ok(_) => panic!("reviewer spawn must reject on missing trust"),
        Err(error) => error,
    };

    // 指引卡简化补缺：结构化等待投影同源随行（供上层紧随 error 帧发出
    // trust_missing_guidance）——entry 缺失携带可复制命令。
    let waiting = trust_waiting.expect("trust-classified start failure carries the waiting");
    assert_eq!(waiting.reason_code, "trust_entry_missing");
    assert!(
        waiting
            .command_markdown
            .as_deref()
            .is_some_and(|command| command.contains("mkdir -p")),
        "entry missing carries the copyable kimi command: {:?}",
        waiting.command_markdown
    );

    assert!(
        error.details.contains("provider_trust_missing"),
        "reviewer gateway 启动臂 trust 拒绝必须提升 provider_trust_missing 分类，got: {}",
        error.details
    );
    assert!(
        error
            .details
            .contains("provider trust not established for KimiCode"),
        "原拒绝原文必须保留，got: {}",
        error.details
    );
    assert!(
        error.details.contains("人工登记") && error.details.contains("显式重新生成"),
        "reviewer 臂必须携带生成场景 retry_action，got: {}",
        error.details
    );
    assert!(
        error.details.contains(
            &trust_home
                .path()
                .join(".kimi-code")
                .join("workspace-trust")
                .display()
                .to_string()
        ),
        "reviewer 臂指导必须同源（真实工件路径），got: {}",
        error.details
    );
}

/// U3 负例：Claude reviewer 无用户级 workspace trust 面——gateway 拒绝
///（registry lookup 等）不得携带 trust 分类/指导。
#[tokio::test]
async fn reviewer_gateway_rejection_without_trust_face_stays_bare() {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let gateway = reviewer_trust_gateway(&paths, trust_home.path());
    let target_dir = tempfile::tempdir().expect("review target anchor");

    let (result, trust_waiting) = start_review_session_via_gateway(
        &gateway,
        &crate::product::models::ProviderName::ClaudeCode,
        reviewer_trust_streaming_input(target_dir.path().to_path_buf()),
        "project_0001".to_string(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await;
    let error = match result {
        Ok(_) => panic!("claude reviewer launch must fail without registered adapter"),
        Err(error) => error,
    };
    assert!(
        trust_waiting.is_none(),
        "无 workspace trust 面的拒绝不得携带结构化等待投影，got: {trust_waiting:?}"
    );

    assert!(
        !error.details.contains("provider_trust_missing")
            && !error.details.contains("人工登记指导"),
        "无 workspace trust 面的 provider 拒绝不得误附 trust 指导，got: {}",
        error.details
    );
}

// ---- 批2 Task 20b 修复轮 1(F1,Main 裁定 b):聚合形态窗口目标钉测 ----
// 形态枚举(报告同步):
// A. Story focus=Some:业务解析→focus 成员 checkout→窗口锚成员(直选);
// B. 聚合形态(Story focus=None/Design involved 空/Plan 多 target):主成员锚
//    (manifest member_ids 首成员,Main 裁定 b)——评审照常走,窗口附注披露
//    「primary-member anchored;非主成员面未对账」(R8 类已知边界);
// C. 真零成员(空 manifest):专用码 plan_review_window_target_unresolved 停等。
mod t20b_f1_plan_window_target {
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use crate::cross_cutting::streaming_provider::{
        ProviderCompletion, ProviderEvent, ProviderSession, StreamingProviderAdapter,
        StreamingProviderInput,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::logical_codebase::{
        IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId,
    };
    use crate::product::workspace_engine::types::{EngineEvent, WorkspaceEngine, WorkspaceSession};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// 最小 verdict provider:approve JSON 按结构化合同回 nonce。
    pub(super) struct ApproveReviewProvider;
    #[async_trait::async_trait]
    impl StreamingProviderAdapter for ApproveReviewProvider {
        async fn start(
            &self,
            input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (event_tx, event_rx) = mpsc::channel(8);
            let (command_tx, _command_rx) = mpsc::channel(8);
            let output = "可以确认。".to_string();
            let contract = input.structured_output_contract;
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::TextDelta {
                        content: output.clone(),
                    })
                    .await;
                let full_output = if let Some(contract) = contract.as_ref() {
                    format!(
                        "{output}\n<ARIA_STRUCTURED_OUTPUT nonce=\"{}\">{{\"verdict\":\"approve\",\"summary\":\"可以确认\",\"findings\":[]}}</ARIA_STRUCTURED_OUTPUT>",
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

    pub(super) enum FixtureEntity {
        /// Story:focus=首成员(唯一成员面,直选)。
        StoryFocusFirstMember,
        /// Story:focus=None(聚合形态→主成员锚)。
        StoryNoFocus,
        /// Design:involved 空(聚合形态→主成员锚)。
        DesignEmptyInvolved,
        /// Story:真零成员(manifest 空→专用码停等)。
        StoryZeroMembers,
    }

    pub(super) struct PlanWindowFixture {
        _root: tempfile::TempDir,
        pub(super) paths: ProductAppPaths,
        lifecycle: LifecycleStore,
        member_a: Option<std::path::PathBuf>,
        project_id: String,
        issue_id: String,
        entity_id: String,
    }

    fn git_init(dir: &std::path::Path) {
        use std::process::Command;
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "t20b@example.test"],
            vec!["config", "user.name", "T20B F1"],
            vec!["commit", "-m", "init", "--quiet", "--allow-empty"],
        ] {
            let status = Command::new("git")
                .current_dir(dir)
                .args(&args)
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git fixture {args:?} must succeed");
        }
    }

    pub(super) fn logical_fixture(entity: FixtureEntity) -> PlanWindowFixture {
        use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
        use crate::product::logical_codebase::{
            AggregatePolicyArtifactStore, LogicalCodebaseFeature,
        };
        use crate::product::project_store::{CreateProjectInput, ProjectStore};
        use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};

        let root = tempfile::tempdir().expect("root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "T20B-F1".to_string(),
                description: None,
            })
            .expect("project");
        let project_id = project.id.clone();

        let zero_members = matches!(entity, FixtureEntity::StoryZeroMembers);
        let mut logical_ids: Vec<LogicalRepositoryId> = Vec::new();
        let mut member_paths: Vec<std::path::PathBuf> = Vec::new();
        let mut physical_ids: Vec<String> = Vec::new();
        if !zero_members {
            for name in ["member-a", "member-b"] {
                let dir = root.path().join(name);
                std::fs::create_dir_all(&dir).expect("member dir");
                git_init(&dir);
                let record = RepositoryStore::with_logical_codebase_feature(
                    paths.clone(),
                    LogicalCodebaseFeature::enabled(),
                )
                .create(CreateRepositoryInput {
                    project_id: project_id.clone(),
                    name: name.to_string(),
                    path: dir.clone(),
                    default_policy_preset: None,
                    default_provider_mode: None,
                    idempotency_key: format!("t20b-f1-{name}"),
                })
                .expect("member repository");
                logical_ids.push(record.logical_repository_id.expect("logical id"));
                physical_ids.push(record.id);
                member_paths.push(dir);
            }
        }

        let manifest = LogicalCodebaseStore::new(paths.clone())
            .load_manifest(&project_id)
            .expect("manifest io")
            .unwrap_or_else(|| {
                // 零成员形态:显式落空 manifest(member_ids 空)。
                let empty = LogicalCodebaseManifest::new(
                    &project_id,
                    root.path().to_path_buf(),
                    Vec::new(),
                );
                LogicalCodebaseStore::new(paths.clone())
                    .save_manifest(&project_id, &empty)
                    .expect("save empty manifest");
                empty
            });
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("bootstrap policy");
        let legacy_lc = crate::product::logical_codebase::legacy_logical_codebase_id(&project_id);
        let issue = IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(legacy_lc.to_string()),
                base_branch: None,
                title: "t20b f1".to_string(),
                description: None,
                change_id: None,
            })
            .expect("issue");
        let issue_id = issue.id.clone();
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
        let entity_id = match entity {
            FixtureEntity::DesignEmptyInvolved => {
                use crate::product::lifecycle_store::{
                    AggregateDesignSpecScope, CreateDesignSpecInput,
                };
                let design = lifecycle
                    .create_design_spec(CreateDesignSpecInput {
                        project_id: project_id.clone(),
                        issue_id: issue_id.clone(),
                        story_spec_ids: Vec::new(),
                        title: "t20b f1 design".to_string(),
                        aggregate_codebase: Some(AggregateDesignSpecScope {
                            logical_codebase_ref: manifest.logical_codebase_id,
                            effective_member_ids: logical_ids.clone(),
                            involved_repository_ids: Vec::new(),
                            change_order: Vec::new(),
                        }),
                    })
                    .expect("design spec");
                design.id
            }
            FixtureEntity::StoryFocusFirstMember
            | FixtureEntity::StoryNoFocus
            | FixtureEntity::StoryZeroMembers => {
                use crate::product::lifecycle_store::{
                    AggregateStorySpecScope, CreateStorySpecInput,
                };
                let story = lifecycle
                    .create_story_spec(CreateStorySpecInput {
                        project_id: project_id.clone(),
                        issue_id: issue_id.clone(),
                        repository_id: physical_ids.first().cloned().unwrap_or_default(),
                        title: "t20b f1 story".to_string(),
                        aggregate_codebase: Some(AggregateStorySpecScope {
                            logical_codebase_ref: manifest.logical_codebase_id,
                            effective_member_ids: logical_ids.clone(),
                            involved_repository_ids: logical_ids.clone(),
                            focus_repository_id: match entity {
                                FixtureEntity::StoryFocusFirstMember => {
                                    logical_ids.first().copied()
                                }
                                _ => None,
                            },
                        }),
                    })
                    .expect("story spec");
                story.id
            }
        };
        PlanWindowFixture {
            _root: root,
            paths,
            lifecycle,
            member_a: member_paths.first().cloned(),
            project_id,
            issue_id,
            entity_id,
        }
    }

    pub(super) fn engine_with_aggregate_view(
        fixture: &PlanWindowFixture,
        workspace_type: crate::product::models::WorkspaceType,
    ) -> (
        tempfile::TempDir,
        mpsc::Receiver<EngineEvent>,
        WorkspaceEngine,
    ) {
        use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
        use crate::product::models::ProviderName;
        let aggregate_view = tempfile::tempdir().expect("aggregate view");
        std::fs::create_dir_all(aggregate_view.path()).expect("aggregate view dir");
        let record = fixture
            .lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: fixture.project_id.clone(),
                issue_id: fixture.issue_id.clone(),
                entity_id: fixture.entity_id.clone(),
                workspace_type,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::ClaudeCode),
                review_rounds: 2,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("workspace session");
        let checkpoint = Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            fixture
                .paths
                .issue_lifecycle_root(&fixture.project_id, &fixture.issue_id),
        ));
        let (tx, rx) = mpsc::channel(64);
        let mut engine = WorkspaceEngine::new_persistent(
            checkpoint,
            LifecycleStore::new(fixture.paths.clone()),
            tx,
            WorkspaceSession::from_record(record),
        );
        // 生产 LC 聚合根视图形态:仓库路径=非 git provider_context_root;
        // 系统上下文携带聚合 scope marker(baseline 链据此跳过)。
        engine.session.repository_path = Some(aggregate_view.path().to_path_buf());
        engine
            .session
            .messages
            .push(crate::product::workspace_engine::types::SessionMessage {
                id: "t20b_f1_aggregate_scope".to_string(),
                role: "system".to_string(),
                content: "## 聚合代码库成员清单\n- member-a\n- member-b".to_string(),
                checkpoint_id: None,
                created_at: "2026-10-17T00:00:00Z".to_string(),
            });
        engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
            markdown: "# Artifact\n\n可以确认".to_string(),
            diff: None,
        });
        (aggregate_view, rx, engine)
    }

    /// 形态 A:Story focus=Some → 窗口锚 focus 成员(直选),无附注。
    #[tokio::test]
    async fn story_focus_aggregate_view_windows_on_focus_member_and_completes() {
        let fixture = logical_fixture(FixtureEntity::StoryFocusFirstMember);
        let (_view, mut rx, mut engine) =
            engine_with_aggregate_view(&fixture, crate::product::models::WorkspaceType::Story);
        let session_id = engine.session().session_id.to_string();
        let member_a = fixture.member_a.clone().expect("member a");
        let window_root = fixture.paths.root().join("reviewer-windows");
        let _kept = fixture._root.keep();

        engine.start_review_or_skip().await;
        let (_command_tx, command_rx) = mpsc::channel(8);
        engine
            .drive_review_session(Arc::new(ApproveReviewProvider), command_rx)
            .await;

        let mut saw_error = None;
        let mut saw_anchor_note = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                EngineEvent::Error { message } => saw_error = Some(message),
                EngineEvent::ExecutionEvent { event, .. }
                    if event.title.contains("primary-member anchored") =>
                {
                    saw_anchor_note = true;
                }
                _ => {}
            }
        }
        assert!(
            saw_error.is_none(),
            "story focus form must review on the focus member: {saw_error:?}"
        );
        assert!(
            !saw_anchor_note,
            "focus-member direct selection must not carry the primary-anchor note"
        );
        let window_store =
            crate::product::reviewer_window_store::ReviewerWindowStore::new(window_root);
        let record = window_store
            .load_window(&crate::product::reviewer_window_store::role_run_window_id(
                &session_id,
                engine.active_run_id().unwrap_or(session_id.as_str()),
            ))
            .expect("window record must exist");
        let expected = member_a.canonicalize().expect("member a canonical");
        assert_eq!(
            record.identity.canonical_worktree, expected,
            "window must anchor the focus member checkout, not the aggregate view"
        );
    }

    /// 形态 B(Story focus=None):主成员锚——评审照常走+窗口附注披露。
    #[tokio::test]
    async fn story_without_focus_anchors_primary_member_and_completes_with_note() {
        let fixture = logical_fixture(FixtureEntity::StoryNoFocus);
        let (_view, mut rx, mut engine) =
            engine_with_aggregate_view(&fixture, crate::product::models::WorkspaceType::Story);
        let session_id = engine.session().session_id.to_string();
        let member_a = fixture.member_a.clone().expect("member a");
        let window_root = fixture.paths.root().join("reviewer-windows");
        let _kept = fixture._root.keep();

        engine.start_review_or_skip().await;
        let (_command_tx, command_rx) = mpsc::channel(8);
        engine
            .drive_review_session(Arc::new(ApproveReviewProvider), command_rx)
            .await;

        let mut saw_error = None;
        let mut saw_anchor_note = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                EngineEvent::Error { message } => saw_error = Some(message),
                EngineEvent::ExecutionEvent { event, .. }
                    if event.title.contains("primary-member anchored")
                        && event.title.contains("非主成员面未对账") =>
                {
                    saw_anchor_note = true;
                }
                _ => {}
            }
        }
        assert!(
            saw_error.is_none(),
            "no-focus aggregate form must review via the primary-member anchor: {saw_error:?}"
        );
        assert!(
            saw_anchor_note,
            "primary-member anchoring must carry the R8-class disclosure note"
        );
        let window_store =
            crate::product::reviewer_window_store::ReviewerWindowStore::new(window_root);
        let record = window_store
            .load_window(&crate::product::reviewer_window_store::role_run_window_id(
                &session_id,
                engine.active_run_id().unwrap_or(session_id.as_str()),
            ))
            .expect("window record must exist");
        let expected = member_a.canonicalize().expect("member a canonical");
        assert_eq!(
            record.identity.canonical_worktree, expected,
            "window must anchor the primary member checkout (member_ids first)"
        );
    }

    /// 形态 B(Design involved 空):主成员锚——评审照常走+附注披露。
    #[tokio::test]
    async fn design_empty_involved_anchors_primary_member_and_completes() {
        let fixture = logical_fixture(FixtureEntity::DesignEmptyInvolved);
        let (_view, mut rx, mut engine) =
            engine_with_aggregate_view(&fixture, crate::product::models::WorkspaceType::Design);
        let member_a = fixture.member_a.clone().expect("member a");
        let window_root = fixture.paths.root().join("reviewer-windows");
        let _kept = fixture._root.keep();

        engine.start_review_or_skip().await;
        let (_command_tx, command_rx) = mpsc::channel(8);
        engine
            .drive_review_session(Arc::new(ApproveReviewProvider), command_rx)
            .await;

        let mut saw_error = None;
        let mut saw_anchor_note = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                EngineEvent::Error { message } => saw_error = Some(message),
                EngineEvent::ExecutionEvent { event, .. }
                    if event.title.contains("primary-member anchored") =>
                {
                    saw_anchor_note = true;
                }
                _ => {}
            }
        }
        assert!(
            saw_error.is_none(),
            "design empty-involved form must review via the primary-member anchor: {saw_error:?}"
        );
        assert!(
            saw_anchor_note,
            "must carry the primary-anchor disclosure note"
        );
        // 窗口记录目录扫描:存在锚定主成员的窗口记录(记录文件含成员路径)。
        let expected = member_a.canonicalize().expect("member a canonical");
        let mut anchored = false;
        fn scan(dir: &std::path::Path, expected: &std::path::Path, out: &mut bool) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    scan(&path, expected, out);
                } else if let Ok(content) = std::fs::read_to_string(&path)
                    && content.contains(&expected.display().to_string())
                {
                    *out = true;
                }
            }
        }
        scan(&window_root, &expected, &mut anchored);
        assert!(
            anchored,
            "a window record anchored on the primary member must exist under {}",
            window_root.display()
        );
    }

    /// 形态 C:真零成员(空 manifest)→专用码停等,非静默/非 IdentityDrift。
    #[tokio::test]
    async fn zero_member_aggregate_reports_dedicated_unresolved_code() {
        let fixture = logical_fixture(FixtureEntity::StoryZeroMembers);
        let (_view, mut rx, mut engine) =
            engine_with_aggregate_view(&fixture, crate::product::models::WorkspaceType::Story);

        engine.start_review_or_skip().await;
        let (_command_tx, command_rx) = mpsc::channel(8);
        engine
            .drive_review_session(Arc::new(ApproveReviewProvider), command_rx)
            .await;

        let mut error_message = None;
        while let Ok(event) = rx.try_recv() {
            if let EngineEvent::Error { message } = event {
                error_message = Some(message);
            }
        }
        let message =
            error_message.expect("zero-member aggregate must stop with an explicit error");
        assert!(
            message.contains("plan_review_window_target_unresolved"),
            "must use the dedicated owner-ruling error code, got: {message}"
        );
        assert!(
            !message.contains("IdentityDrift") && !message.contains("identity_drift"),
            "must not misreport as git identity drift, got: {message}"
        );
    }
}

// ---- Task 2(REQ-ENV-14/REQ-PLN-11):多仓不开评审窗 + lc 评审目标降级 LC 根 ----
// lc 形态以注入 logical gateway 表达(T1 门禁唯一判据);夹具复用 t20b_f1
// 的 logical_fixture/engine_with_aggregate_view,authority root 用独立临时目录。
mod t2_lc_no_review_window {
    use super::t20b_f1_plan_window_target::{
        ApproveReviewProvider, FixtureEntity, engine_with_aggregate_view, logical_fixture,
    };
    use crate::cross_cutting::provider_adapter::ProviderAdapter;
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, LogicalCodebaseProviderGateway, PolicyTarget,
        PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource, ProviderGatewayError,
        ProviderRef, SessionLaunchRequest, SessionPolicyAction,
    };
    use crate::product::models::ProviderName;
    use crate::product::workspace_engine::types::EngineEvent;
    use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    // ---- 最小 gateway 桩(与 prompts.rs T1 门禁测试同构):形态门禁只读
    // `Option::is_some()`,不 bootstrap 政策,不触 validate 链。----

    /// 占位 capability source:恒拒即可(不会被调用)。
    struct UnusedCapabilitySource;

    impl ProviderCapabilitySource for UnusedCapabilitySource {
        fn require_supported(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "t2 fixture capability source is never queried".to_string(),
            ))
        }

        fn require_resume_supported(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "t2 fixture capability source is never queried".to_string(),
            ))
        }

        fn require_write_boundary(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "t2 fixture capability source is never queried".to_string(),
            ))
        }

        fn require_root_recipe_supported(
            &self,
            _provider: &ProviderRef,
            _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "t2 fixture capability source is never queried".to_string(),
            ))
        }
    }

    /// 占位 target resolver:透传请求 target(不会被调用)。
    struct UnusedTargetResolver;

    impl PolicyTargetResolver for UnusedTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<PolicyTarget, ProviderGatewayError> {
            Ok(request.target.clone())
        }
    }

    /// 占位同步 adapter:gateway 构造参数,不会被调用。
    struct UnusedSyncAdapter;

    impl ProviderAdapter for UnusedSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
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

    /// 构造仅满足形态判据的最小 gateway;authority root 用独立临时目录,
    /// 返回 (gateway, authority_root, 临时目录锚)。
    fn lc_gateway_fixture() -> (
        Arc<LogicalCodebaseProviderGateway>,
        PathBuf,
        tempfile::TempDir,
    ) {
        let root = tempfile::tempdir().expect("gateway authority root");
        let authority_root = root.path().join("authority");
        std::fs::create_dir_all(&authority_root).expect("create authority root");
        (
            Arc::new(LogicalCodebaseProviderGateway::new(
                AggregatePolicyArtifactStore::new(ProductAppPaths::new(root.path().join(".aria"))),
                Arc::new(UnusedCapabilitySource),
                Arc::new(UnusedTargetResolver),
                Arc::new(ProviderRegistry::new()),
                Arc::new(UnusedSyncAdapter),
                always_available_gate(),
                authority_root.clone(),
            )),
            authority_root,
            root,
        )
    }

    /// REQ-ENV-14:lc 形态评审会话不开评审窗——无 plan_review_window_* 错误码、
    /// 无窗口注意力提示、评审窗口 store 无新记录;材料齐备(2 成员)时照常走完。
    #[tokio::test]
    async fn lc_review_session_opens_no_plan_review_window() {
        let fixture = logical_fixture(FixtureEntity::StoryFocusFirstMember);
        let (gateway, _authority_root, _authority_kept) = lc_gateway_fixture();
        let (_view, mut rx, engine) =
            engine_with_aggregate_view(&fixture, crate::product::models::WorkspaceType::Story);
        let mut engine = engine.with_logical_provider_gateway(gateway);
        let session_id = engine.session().session_id.to_string();
        let window_root = fixture.paths.root().join("reviewer-windows");

        engine.start_review_or_skip().await;
        let (_command_tx, command_rx) = mpsc::channel(8);
        engine
            .drive_review_session(Arc::new(ApproveReviewProvider), command_rx)
            .await;

        let mut errors = Vec::new();
        let mut window_noise = Vec::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                EngineEvent::Error { message } => errors.push(message),
                EngineEvent::ExecutionEvent { event, .. } => {
                    let text = format!("{} {}", event.title, event.detail.unwrap_or_default());
                    if text.contains("plan_review_window") || text.contains("评审窗") {
                        window_noise.push(text);
                    }
                }
                _ => {}
            }
        }
        assert!(errors.is_empty(), "lc 形态评审必须无错误收尾: {errors:?}");
        assert!(
            window_noise.is_empty(),
            "lc 形态不得出现窗口错误码/窗口注意力提示: {window_noise:?}"
        );
        let window_store =
            crate::product::reviewer_window_store::ReviewerWindowStore::new(window_root.clone());
        let window_id = crate::product::reviewer_window_store::role_run_window_id(
            &session_id,
            engine.active_run_id().unwrap_or(session_id.as_str()),
        );
        assert!(
            window_store.load_window(&window_id).is_err(),
            "lc 形态评审不得产生评审窗记录({window_id})"
        );
        if window_root.exists() {
            let leftover = std::fs::read_dir(&window_root)
                .map(|entries| entries.count())
                .unwrap_or(0);
            assert_eq!(
                leftover, 0,
                "lc 形态评审后 reviewer-windows 树不得留任何记录"
            );
        }
    }

    /// REQ-ENV-14:lc 空成员集合(空 manifest)→ 材料未解析显式阻断,
    /// 不启动 provider,错误码前缀 plan_review_material_unresolved:。
    #[tokio::test]
    async fn lc_review_with_empty_member_set_is_blocked_as_material_unresolved() {
        let fixture = logical_fixture(FixtureEntity::StoryZeroMembers);
        let (gateway, _authority_root, _authority_kept) = lc_gateway_fixture();
        let (_view, mut rx, engine) =
            engine_with_aggregate_view(&fixture, crate::product::models::WorkspaceType::Story);
        let mut engine = engine.with_logical_provider_gateway(gateway);

        engine.start_review_or_skip().await;
        let (_command_tx, command_rx) = mpsc::channel(8);
        engine
            .drive_review_session(Arc::new(ApproveReviewProvider), command_rx)
            .await;

        let mut error_message = None;
        while let Ok(event) = rx.try_recv() {
            if let EngineEvent::Error { message } = event {
                error_message = Some(message);
            }
        }
        let message = error_message.expect("空成员集合必须以显式错误阻断评审");
        assert!(
            message.starts_with("plan_review_material_unresolved:"),
            "必须以材料未解析错误码阻断, got: {message}"
        );
        assert!(
            !message.contains("plan_review_window"),
            "lc 形态不得产 plan_review_window_* 错误码, got: {message}"
        );
    }

    /// REQ-ENV-14:单仓(未注入 gateway)仍开评审窗,窗口 id 形如
    /// {session}-{run}-w01。
    #[tokio::test]
    async fn single_repo_review_session_still_opens_window() {
        let fixture = logical_fixture(FixtureEntity::StoryFocusFirstMember);
        let (_view, mut rx, mut engine) =
            engine_with_aggregate_view(&fixture, crate::product::models::WorkspaceType::Story);
        let session_id = engine.session().session_id.to_string();

        engine.start_review_or_skip().await;
        let (_command_tx, command_rx) = mpsc::channel(8);
        engine
            .drive_review_session(Arc::new(ApproveReviewProvider), command_rx)
            .await;

        let mut saw_error = None;
        while let Ok(event) = rx.try_recv() {
            if let EngineEvent::Error { message } = event {
                saw_error = Some(message);
            }
        }
        assert!(saw_error.is_none(), "单仓评审照常开窗完成: {saw_error:?}");
        let run_id = engine
            .active_run_id()
            .unwrap_or(session_id.as_str())
            .to_string();
        let window_store = crate::product::reviewer_window_store::ReviewerWindowStore::new(
            fixture.paths.root().join("reviewer-windows"),
        );
        let record = window_store
            .load_window(&crate::product::reviewer_window_store::role_run_window_id(
                &session_id,
                &run_id,
            ))
            .expect("单仓形态必须仍产窗口记录");
        assert_eq!(
            record.window_id,
            format!("{session_id}-{run_id}-w01"),
            "窗口 id 必须保持 {{session}}-{{run}}-w01 形态"
        );
    }

    /// REQ-PLN-11(设计 A 条目 2):lc 评审目标降级为 LC 根——target 与 cwd
    /// 均为 authority root,不再要求 repository_path 是成员 worktree。
    #[test]
    fn lc_review_target_is_logical_root() {
        let fixture = logical_fixture(FixtureEntity::StoryFocusFirstMember);
        let (gateway, authority_root, _authority_kept) = lc_gateway_fixture();
        let (_view, _rx, mut engine) =
            engine_with_aggregate_view(&fixture, crate::product::models::WorkspaceType::Story);
        engine.session.repository_path = None;
        let engine = engine.with_logical_provider_gateway(gateway);

        let (target, cwd) = engine
            .review_launch_target_and_cwd()
            .expect("lc 评审目标必须解析为 LC 根");
        assert_eq!(
            target, authority_root,
            "lc 评审 target 必须降级为 LC 根(authority root)"
        );
        assert_eq!(
            cwd.as_deref(),
            Some(authority_root.as_path()),
            "lc 评审 cwd 必须同为 LC 根"
        );
    }
}
