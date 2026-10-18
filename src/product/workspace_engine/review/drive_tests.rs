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
        None,
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
        None,
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
    struct ApproveReviewProvider;
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

    enum FixtureEntity {
        /// Story:focus=首成员(唯一成员面,直选)。
        StoryFocusFirstMember,
        /// Story:focus=None(聚合形态→主成员锚)。
        StoryNoFocus,
        /// Design:involved 空(聚合形态→主成员锚)。
        DesignEmptyInvolved,
        /// Story:真零成员(manifest 空→专用码停等)。
        StoryZeroMembers,
    }

    struct PlanWindowFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
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

    fn logical_fixture(entity: FixtureEntity) -> PlanWindowFixture {
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

    fn engine_with_aggregate_view(
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
            .load_window(&format!("plan-{session_id}-w01"))
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
            .load_window(&format!("plan-{session_id}-w01"))
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

/// T22 b45 现场 r1(plan SC 复评臂 ProviderCommandMissing kimi):
/// reviewer gateway 启动环境的受信构造钉测——有活动审查窗时必须产出
/// 白名单 env(PATH 精确继承、HOME 冻结受信 owner home),裸名 CLI 靠
/// 最终 PATH 可解析;fail-closed 形态=空窗 id 被 freeze 拒绝。
#[test]
fn reviewer_launch_environment_carries_frozen_path_with_window() {
    let input = reviewer_trust_streaming_input(std::path::PathBuf::from("/tmp/review-target"));
    let environment = reviewer_launch_environment(&input, "review_window_0001")
        .expect("windowed reviewer launch must build the trusted environment");
    let owner_home =
        crate::cross_cutting::provider_environment::resolve_trusted_owner_home_from_env()
            .expect("test host must provide an absolute HOME for the frozen environment");
    assert_eq!(
        environment.get("PATH"),
        std::env::var_os("PATH")
            .map(|value| value.to_string_lossy().into_owned())
            .as_ref(),
        "reviewer 启动必须携带精确继承的宿主 PATH(裸名 CLI 可解析的充要条件)"
    );
    assert_eq!(
        environment.get("HOME").map(String::as_str),
        Some(owner_home.display().to_string().as_str()),
        "HOME 必须冻结受信 owner home"
    );
}

#[test]
fn reviewer_launch_environment_blank_window_fails_closed() {
    let input = reviewer_trust_streaming_input(std::path::PathBuf::from("/tmp/review-target"));
    let error = reviewer_launch_environment(&input, "  ")
        .expect_err("blank window id must fail closed, not fall back to an empty env");
    assert!(
        error.contains("plan reviewer"),
        "fail-closed 原因码必须可诊断,got: {error}"
    );
}
