/// 旧单仓 coding fixture 的生产路径现在会通过 `RepositoryStore::for_project`
/// 读取项目记录。各 fixture 会先写入 repository / issue 等旧数据，故不能依赖
/// `ProjectStore::create` 的目录计数；这里直接为既有 `project_0001` 路径持久化
/// 最小 ProjectRecord，确保旧数据路径仍明确为单仓。
fn seed_legacy_project_fixture(paths: &ProductAppPaths) {
    let project_path = paths.project_root("project_0001").join("project.json");
    if project_path.exists() {
        // 迁移档夹具（migrate_and_freeze_coding_ws_fixture）在建 attempt 之前把
        // legacy 布局升成 LC 权威——此后 LC 存储在场属预期；旧「必须仍是单仓」
        // 断言只描述迁移前状态，迁移后的二次进入不再成立。
        let _project: cadence_aria::product::models::ProjectRecord =
            cadence_aria::product::json_store::read_json(&project_path)
                .expect("read legacy coding fixture project");
        return;
    }

    cadence_aria::product::json_store::write_json(
        &project_path,
        &cadence_aria::product::models::ProjectRecord {
            id: "project_0001".to_string(),
            name: "legacy coding websocket fixture".to_string(),
            description: None,
            created_at: "2026-08-18T00:00:00Z".to_string(),
            updated_at: "2026-08-18T00:00:00Z".to_string(),
            last_opened_at: None,
        },
    )
    .expect("persist legacy coding fixture project");
}

fn seed_group_revision_history_fixture(lifecycle: &LifecycleStore, session_id: &str) {
    let entries = [
        ("work_item_0001", "work_item_revision_0001"),
        ("work_item_0002", "work_item_revision_0002"),
    ]
    .into_iter()
    .map(
        |(logical_work_item_id, revision_id)| WorkItemHistoryEntryDto {
            kind: WorkItemHistoryEntryKind::WorkItemRevision,
            id: revision_id.to_string(),
            logical_work_item_id: logical_work_item_id.to_string(),
            related_revision_id: Some(format!("draft_{revision_id}")),
            summary: format!("Compiled WorkItem revision from draft_{revision_id}"),
            created_at: "2026-07-18T00:00:00Z".to_string(),
        },
    )
    .collect();
    lifecycle
        .save_artifact_versions(
            session_id,
            &[ArtifactVersion {
                version: 1,
                payload: ArtifactPayload::WorkItemRevisionHistory {
                    history: Box::new(WorkItemRevisionHistoryDto { entries }),
                },
                generated_by: ProviderName::Fake,
                reviewed_by: None,
                review_verdict: None,
                confirmed_by: None,
                is_current: true,
                created_at: "2026-07-18T00:00:00Z".to_string(),
                source_node_id: "timeline_node_compile".to_string(),
            }],
        )
        .expect("save work item revision history");
}

// ============================================================================
// 块3 模板①（同 it_web 全量失败收尾交接 §3）：coding WS 夹具的 LC 权威迁移段。
//
// legacy 单仓记录（project/repository/issue/work item）经生产迁移器
// `IdentityMigrationExecutor::ensure_identity_schema` 升成真 LC 权威：
// LC record/manifest/member/checkout + issue selection 与 work item
// target 回填 + bootstrap policy artifact。coding 链的路由
// （`RepositoryRouting::load_for_issue` / `resolve_coding_attempt_repository`）
// 与 admission（REQ-COD-03「LC 权威 + 冻结快照」唯一凭证）从此有权威可寻。
// 迁移为纯 JSON 落盘（不触发 codegraph），无需 CODEGRAPH_FIXTURE_LOCK。
// ============================================================================

/// 夹具缺 issue 记录时补建（带确定性 legacy 别名 LC 归属 + repo_id——
/// 迁移回填 issue selection 的输入；已有 issue 的夹具不重复建）。
fn ensure_legacy_issue_fixture(app_paths: &ProductAppPaths, repository_id: &str) {
    if app_paths
        .issue_root("project_0001", "issue_0001")
        .join("issue.json")
        .exists()
    {
        return;
    }
    cadence_aria::product::issue_store::IssueStore::new(app_paths.clone())
        .create(
            cadence_aria::product::issue_store::CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository_id.to_string()),
                logical_codebase_id: legacy_logical_codebase_id("project_0001"),
                title: "coding ws fixture issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            },
        )
        .expect("seed coding ws fixture issue");
}

/// 夹具补件：测试手工播种的 provider role run（不经引擎 provider_stream 启动，
/// 因而没有 Task 14 采集的越界基线）会被交付前统一门（Task 15）以
/// `cross_target_baseline_missing` fail-closed 阻断。按生产同款 JSON 形状为该
/// run 补采基线：窗口取 attempt 冻结快照的成员（单仓迁移夹具的唯一 active
/// 成员），HEAD 与 porcelain 均为该 checkout 的真实 git 采样，不伪造。
fn seed_cross_target_baseline_for_role_run(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    run_id: &str,
) {
    let snapshot = attempt
        .target_snapshot
        .as_ref()
        .expect("baseline seed requires frozen target snapshot");
    let checkout = &snapshot.canonical_path;
    let head = String::from_utf8_lossy(
        &Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(checkout)
            .output()
            .expect("sample baseline HEAD")
            .stdout,
    )
    .trim()
    .to_string();
    // 生产采样不做 trim（git_status 原样返回 stdout），冻结值保持字节一致。
    let porcelain = String::from_utf8_lossy(
        &Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(checkout)
            .output()
            .expect("sample baseline porcelain")
            .stdout,
    )
    .to_string();
    let baseline = json!({
        "run_id": run_id,
        "captured_at": chrono::Utc::now().to_rfc3339(),
        "member_checkouts": [{
            "logical_repository_id": serde_json::to_value(snapshot.logical_repository_id)
                .expect("serialize baseline member id"),
            "canonical_path": checkout.to_string_lossy(),
            "head_revision": head,
            "porcelain_status": porcelain,
        }],
    });
    let path = app_paths
        .issue_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join("cross-target-baselines")
        .join(format!("{run_id}.json"));
    std::fs::create_dir_all(path.parent().expect("baseline dir")).expect("create baseline dir");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&baseline).expect("serialize baseline"),
    )
    .expect("seed cross target baseline");
}

/// lifecycle 写入（work item/plan/session）会先建出 `issues/issue_0001/`
/// 目录（不含 issue.json），而 `IssueStore::create` 按 issues/ 目录数分配
/// id——issue 必须在任何 lifecycle 写入之前创建，否则 id 漂成 issue_0002、
/// 迁移的 selection 回填错位，admission 即 FailClosed(TargetMissing)。
/// 本助手用于「repository 记录之后、work item 创建之前」的夹具顺序收口。
fn ensure_legacy_issue_before_lifecycle_writes(app_paths: &ProductAppPaths, repository_id: &str) {
    seed_legacy_project_fixture(app_paths);
    ensure_legacy_issue_fixture(app_paths, repository_id);
}

/// 迁移段：把 legacy 别名布局升成真 LC 权威（同步幂等）。
fn migrate_legacy_coding_ws_fixture(app_paths: &ProductAppPaths) {
    cadence_aria::product::logical_codebase::IdentityMigrationExecutor::new(app_paths.clone())
        .ensure_identity_schema("project_0001")
        .expect("migrate legacy coding ws fixture into logical codebase authority");
}

/// 迁移段收口：从权威事实源冻结 attempt target snapshot（唯一成员；经
/// `build_attempt_target_snapshot` 解析 member/checkout/repos 投影 + 政策
/// digest + checkout HEAD，不自造字段——admission 的
/// `validate_snapshot_fields` 三层身份复验与权威逐字一致）。
fn frozen_member_target_snapshot(app_paths: &ProductAppPaths) -> AttemptTargetSnapshot {
    let lc_id = legacy_logical_codebase_id("project_0001");
    let mut members = LogicalCodebaseStore::new(app_paths.clone())
        .list_members("project_0001")
        .expect("list migrated coding ws members");
    let member = members
        .pop()
        .expect("migrated coding ws fixture has a logical member");
    assert!(
        members.is_empty(),
        "coding ws single-repo fixture must migrate to exactly one member"
    );
    cadence_aria::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot(
        app_paths,
        "project_0001",
        member.logical_repository_id,
        Some(&lc_id),
    )
    .expect("freeze coding ws fixture target snapshot")
}

/// 迁移 + 冻结快照的组合段（单仓夹具共用收口）。
///
/// 调用时机：git 仓、repository 记录、work item 落盘之后、
/// `create_legacy_*_coding_attempt_fixture` 之前——迁移要回填 issue selection 与
/// work item target（需要它们在场），冻结快照要填进 attempt 创建输入。
fn migrate_and_freeze_coding_ws_fixture(
    app_paths: &ProductAppPaths,
    repository_id: &str,
) -> AttemptTargetSnapshot {
    seed_legacy_project_fixture(app_paths);
    ensure_legacy_issue_fixture(app_paths, repository_id);
    migrate_legacy_coding_ws_fixture(app_paths);
    frozen_member_target_snapshot(app_paths)
}

/// 报告 ClaudeCode 恒健康的 availability gate：LC（gateway）路径在 spawn 前
/// 复验 provider 可用性，测试环境无真实 provider 二进制，直挂恒健康源
/// （同 issue_delivery_summary 先例；Fake 不经 gateway，无需槽位）。
fn always_available_provider_gate() -> Arc<ProviderAvailabilityGate> {
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

/// 同步 adapter stub：gateway 工厂构造签名需要，coding WS 只走 streaming 路径。
struct CodingWsStubSyncAdapter;

impl ProviderAdapter for CodingWsStubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Err(ProviderAdapterError::provider_unavailable(
            "coding ws fixture sync adapter is not expected to run",
        ))
    }
}

/// 模板⑤收口：带冻结快照的 attempt 恒经 LC gateway 启动 provider——
/// ① durable capability 行 Confirmed（bootstrap 行恒 Unknown，缺记录即
/// `provider_capability_launch_not_confirmed`）；② gateway 直挂
/// always-available gate。registry 的 ClaudeCode 槽位承载脚本化 provider
/// （gateway 方言槽位；Fake 保留给不经 gateway 的既有路径）。
fn coding_ws_state_with_gateway(
    root_path: &Path,
    app_paths: &ProductAppPaths,
    registry: ProviderRegistry,
) -> WebAppState {
    crate::provider_capability_fixture::seed_confirmed_claude(app_paths, "project_0001");
    let registry = Arc::new(registry);
    WebAppState::with_events_and_provider_registry(
        root_path.to_path_buf(),
        WebRuntime::new_fake(root_path.to_path_buf()),
        cadence_aria::web::events::EventHub::new(),
        registry.clone(),
    )
    .with_gateway_factory(Arc::new(LogicalCodebaseGatewayFactory::new(
        app_paths.clone(),
        registry,
        Arc::new(CodingWsStubSyncAdapter),
        always_available_provider_gate(),
    )))
}

// ============================================================================
// 模板⑤补件：gateway 方言槽位的替身包装。
//
// 带冻结快照的 attempt 恒经 LC gateway validated 启动（`start_validated`
// 分发；raw `start` 仅保留单仓 direct），而 coding WS 的脚本化替身只实现了
// legacy `run_streaming` 行为（按 AdapterInput 的 role / output_schema 分支）。
// 本包装从 validated 只读投影 `launch_view`（role / spawn cwd / prompt）重建
// AdapterInput 驱动内层 `run_streaming`，并把 StreamChunk 桥接为 ProviderEvent
// 会话；output_schema 用 prompt 的生产模板标记复原（与
// `code_reviewer_retry_cycle_streaming_input` /
// `internal_pr_review_streaming_input` 的构造取值逐字对齐）。CodeReviewer 有
// 两条生产模板：legacy 单 WorkItem 中文模板（prompts.rs「你是 CodeReviewer」）
// 与 group(schema v2) 投影渲染模板（work_item_projection/render.rs 的
// 「# {provider} Reviewer Work Item Projection」）——两条都复原到
// `coding_workspace_code_review_json`。脚本状态（计数/捕获）
// 全留在内层替身，桥接不复制脚本逻辑。
// ============================================================================

fn gateway_bridged_output_schema(prompt: &str) -> &'static str {
    if prompt.contains("你是 CodeReviewer") || prompt.contains(" Reviewer Work Item Projection") {
        "coding_workspace_code_review_json"
    } else if prompt.contains("你是 WorkItemGroup GroupFinalReview reviewer") {
        "coding_workspace_internal_pr_review_json"
    } else {
        ""
    }
}

struct GatewayDialectStreamingStub {
    inner: Arc<dyn StreamingProviderAdapter>,
}

impl GatewayDialectStreamingStub {
    fn wrapping(inner: Arc<dyn StreamingProviderAdapter>) -> Arc<Self> {
        Arc::new(Self { inner })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GatewayDialectStreamingStub {
    /// 已自带 `start_validated` 的替身（如 ScopedSessionProvider）优先走其
    /// 自身实现；仅当 inner 落到 trait 默认拒绝（错误串为 trait 默认实现
    /// `streaming_provider/mod.rs` 的逐字常量）时才用 run_streaming 桥接。
    async fn start_validated(
        &self,
        validated: ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        match self
            .inner
            .start_validated(validated.clone(), cancel.clone())
            .await
        {
            Ok(session) => Ok(session),
            Err(error)
                if error
                    .stderr
                    .contains("validated start is not supported by this adapter") =>
            {
                self.bridged_validated_session(validated, cancel).await
            }
            Err(error) => Err(error),
        }
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.inner.run_streaming(input, cancel).await
    }
}

impl GatewayDialectStreamingStub {
    async fn bridged_validated_session(
        &self,
        validated: ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let view = validated.launch_view();
        let input = AdapterInput {
            provider_type: ProviderType::ClaudeCode,
            role: view.role.clone(),
            worktree_path: Some(view.working_dir.to_string_lossy().to_string()),
            working_directory: None,
            provider_stream_log_dir: None,
            prompt: view.prompt.to_string(),
            context_files: Vec::new(),
            output_schema: gateway_bridged_output_schema(view.prompt).to_string(),
            timeout: cadence_aria::cross_cutting::provider_adapter::DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
            env_vars: Default::default(),
        };
        let mut chunk_rx = self.inner.run_streaming(&input, cancel).await?;
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            while let Some(chunk) = chunk_rx.recv().await {
                let event = match chunk {
                    StreamChunk::Text(text) => ProviderEvent::TextDelta { content: text },
                    StreamChunk::Done { full_output } => {
                        ProviderEvent::Completed(ProviderCompletion::plain(full_output, None))
                    }
                    StreamChunk::Error(message) => ProviderEvent::Failed { message },
                };
                if event_tx.send(event).await.is_err() {
                    return;
                }
            }
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}
