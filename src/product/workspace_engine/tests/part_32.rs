// T10b:规划 review via-gateway 骨架的 lib 层单测。
//
// 覆盖:
// - 注入 fake-registry gateway 后调用 `drive_review_session_via_gateway` →
//   会话驱动正常(reviewer pass → HumanConfirm)+ gateway audit `stream_launches` +1。
// - 既有 `drive_review_session`(Legacy,未注入 gateway)路径在其余 part 中保持
//   零变化,本文件不重复覆盖。

use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseManifest,
    LogicalCodebaseProviderGateway, PolicyTarget, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderDialect, ProviderGatewayError, ProviderRef, ProviderRefType,
    SessionLaunchRequest, SessionPolicyAction,
};
use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};

/// 同步 adapter stub:`run` 返回最小成功输出,供 gateway 构造。
struct ReviewStubSyncAdapter;

impl ProviderAdapter for ReviewStubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
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

/// 测试用 capability source:返回固定 capability,resume 证据恒为 `Confirmed`,
/// 使 review repair 的 resume 启动也能通过 spawn 前复验。
struct ReviewStaticCapabilitySource;

impl ProviderCapabilitySource for ReviewStaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let adapter_dialect = match provider.provider_type {
            ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
            ProviderRefType::Codex => ProviderDialect::CodexCliV1,
        };
        Ok(ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.4.0".to_string(),
            adapter_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            resume_evidence: ResumeEvidenceState::Confirmed,
        })
    }
}

/// 测试用 target resolver:直接透传 request target。aggregate_root target 的
/// worktree 与 input.working_dir 相同,spawn 前 canonicalize 复验可通过对等比较。
struct ReviewPassThroughTargetResolver;

impl PolicyTargetResolver for ReviewPassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

fn review_always_available_gate() -> Arc<ProviderAvailabilityGate> {
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
        providers: [ProviderName::ClaudeCode, ProviderName::Codex]
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

struct ReviewGatewayFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    gateway: Arc<LogicalCodebaseProviderGateway>,
    audit: Arc<GatewayRunAudit>,
    worktree: std::path::PathBuf,
}

fn review_gateway_fixture() -> ReviewGatewayFixture {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("create review worktree");

    let manifest = LogicalCodebaseManifest::new("project_0001", worktree.clone(), Vec::new());
    let policy_store = AggregatePolicyArtifactStore::new(paths.clone());
    policy_store
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");

    // fake registry:ClaudeCode 映射到「输出 pass verdict」的 streaming adapter。
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ReviewVerdictStreamingProvider {
            output: "审核通过。\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"可以确认\"}\n```",
            provider_type: Arc::new(Mutex::new(None)),
            prompt: Arc::new(Mutex::new(None)),
        }),
    );

    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
        policy_store,
        Arc::new(ReviewStaticCapabilitySource),
        Arc::new(ReviewPassThroughTargetResolver),
        Arc::new(registry),
        Arc::new(ReviewStubSyncAdapter),
        review_always_available_gate(),
        audit.clone(),
        worktree.clone(),
    ));

    ReviewGatewayFixture {
        _root: root,
        paths,
        gateway,
        audit,
        worktree,
    }
}

#[tokio::test]
async fn drive_review_session_via_gateway_records_audit_and_completes() {
    let fixture = review_gateway_fixture();
    let audit = fixture.audit.clone();
    assert_eq!(audit.stream_launches(), 0);

    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_review_via_gateway");
    session.review_rounds = 2;
    session.reviewer_provider = Some(ProviderName::ClaudeCode);
    session.artifact = Some(artifact_payload("# Artifact\n\n可以确认"));
    session.repository_path = Some(fixture.worktree.clone());

    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(fixture.gateway.clone());

    engine.start_review_or_skip().await;
    engine
        .drive_review_session_via_gateway(empty_provider_commands())
        .await;

    assert_eq!(
        audit.stream_launches(),
        1,
        "logical review session must start via gateway once"
    );
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "Story/Design pass verdict must return its report to author confirmation"
    );
    assert!(
        engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::ReviewerRun
                && node.status == TimelineNodeStatus::Completed
                && node.summary.as_deref() == Some("Review 完成，报告已进入对话流")
        }),
        "Story/Design reviewer run must complete before its report returns to author confirmation"
    );
}

/// 测试用 target resolver：透传 request target 并记录之，用于验证
/// `routing_reference_context` 构造 aggregate_root target 时镜像 factory 的
/// canonicalize 语义。
#[derive(Default)]
struct RecordingTargetResolver {
    seen: Arc<Mutex<Option<PolicyTarget>>>,
}

impl PolicyTargetResolver for RecordingTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        *self.seen.lock().unwrap() = Some(request.target.clone());
        Ok(request.target.clone())
    }
}

/// 测试用 target resolver：恒失败，用于验证 gateway validate Err → Legacy 行为不变。
struct FailingTargetResolver;

impl PolicyTargetResolver for FailingTargetResolver {
    fn resolve_and_revalidate(
        &self,
        _request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Err(ProviderGatewayError::Target("stub failure".to_string()))
    }
}

/// 构造一个最小 gateway：静态 capability + 指定 target resolver + 真实 bootstrap 的
/// aggregate policy。`routing_reference_context` 只走 validate（政策 + capability +
/// target），不触发真实 provider run。
fn routing_context_gateway<T: PolicyTargetResolver + 'static>(
    root: &tempfile::TempDir,
    resolver: T,
) -> (Arc<LogicalCodebaseProviderGateway>, std::path::PathBuf) {
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("create worktree");

    let manifest = LogicalCodebaseManifest::new("project_0001", worktree.clone(), Vec::new());
    let policy_store = AggregatePolicyArtifactStore::new(paths.clone());
    policy_store
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");

    let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
        policy_store,
        Arc::new(ReviewStaticCapabilitySource),
        Arc::new(resolver),
        Arc::new(ProviderRegistry::new()),
        Arc::new(ReviewStubSyncAdapter),
        review_always_available_gate(),
        Arc::new(GatewayRunAudit::new()),
        worktree.clone(),
    ));
    (gateway, worktree)
}

#[test]
fn routing_reference_context_returns_legacy_without_gateway() {
    let root = tempfile::tempdir().expect("temporary product root");
    let (event_tx, _event_rx) = mpsc::channel(64);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        event_tx,
        make_session("sess_no_gateway"),
    );

    assert!(matches!(
        engine.routing_reference_context(),
        RoutingReferenceContext::Legacy
    ));
}

#[test]
fn routing_reference_context_canonicalizes_aggregate_root_target() {
    let root = tempfile::tempdir().expect("temporary product root");
    let resolver = RecordingTargetResolver::default();
    let seen = resolver.seen.clone();
    let (gateway, worktree) = routing_context_gateway(&root, resolver);

    // repository_path 故意带 `..`，验证 target worktree 在构造时被 canonicalize。
    let non_canonical = worktree.join("..").join("worktree");
    let canonical = std::fs::canonicalize(&non_canonical).expect("canonicalize non-canonical path");

    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_logical");
    session.repository_path = Some(non_canonical);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(gateway);

    assert!(matches!(
        engine.routing_reference_context(),
        RoutingReferenceContext::Logical(_)
    ));

    let recorded = seen
        .lock()
        .unwrap()
        .clone()
        .expect("resolver must record target");
    assert_eq!(recorded.worktree, canonical);
}

#[test]
fn routing_reference_context_returns_legacy_when_validate_fails() {
    let root = tempfile::tempdir().expect("temporary product root");
    let (gateway, worktree) = routing_context_gateway(&root, FailingTargetResolver);

    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_validate_fails");
    session.repository_path = Some(worktree);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(gateway);

    // validate Err → Legacy 行为不变。`tracing::warn!` 是日志级副作用，
    // 无稳定可捕获的断言方式，此处不测试日志输出本身。
    assert!(matches!(
        engine.routing_reference_context(),
        RoutingReferenceContext::Legacy
    ));
}


#[test]
fn prompt_sliding_window_applies_to_author_revision_and_reviewer_entrypoints() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_prompt_sliding_window");
    session.workspace_type = WorkspaceType::Story;
    let mut artifacts = Vec::new();
    session.messages = vec![SessionMessage {
        id: "msg_000".to_string(),
        role: "system".to_string(),
        content: "Workspace 生成任务已准备\n\n[canonical_inputs]\n完整 canonical inputs：不得裁剪。\n\n[constraint_summary]\n历史约束。".to_string(),
        checkpoint_id: None,
        created_at: "2026-08-21T00:00:00Z".to_string(),
    }];
    for round in 1..=4 {
        let markdown = format!(
            "# Story Artifact v{round}\n\n## 功能需求\n- [REQ-{round:03}] {}\n\n## 成功标准\n- [AC-{round:03}] {}\n",
            "早期 artifact 正文 ".repeat(30),
            "验收细节 ".repeat(30),
        );
        session.messages.push(SessionMessage {
            id: format!("msg_user_{round}"),
            role: "user".to_string(),
            content: format!("ROUND-{round}-USER-RAW {}", "需求细节 ".repeat(30)),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:00:{round:02}Z"),
        });
        session.messages.push(SessionMessage {
            id: format!("msg_author_{round}"),
            role: "assistant".to_string(),
            content: markdown.clone(),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:01:{round:02}Z"),
        });
        session.messages.push(SessionMessage {
            id: format!("msg_reviewer_{round}"),
            role: "reviewer".to_string(),
            content: format!(
                "[review_summary]\nround {round} verdict\n\n[review_findings]\n1. severity: suggestion\n   message: round {round} suggestion\n   evidence: artifact v{round}\n   required_action: optional"
            ),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:02:{round:02}Z"),
        });
        artifacts.push(ArtifactVersion {
            version: round,
            payload: ArtifactPayload::Markdown {
                markdown,
                diff: None,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: Some(ProviderName::Codex),
            review_verdict: Some(ReviewVerdictType::Revise),
            confirmed_by: None,
            is_current: round == 4,
            created_at: format!("2026-08-21T00:03:{round:02}Z"),
            source_node_id: format!("author_round_{round}"),
        });
    }
    session.messages.insert(
        4,
        SessionMessage {
            id: "msg_choice".to_string(),
            role: "system".to_string(),
            content: "结构化交互审计记录（daemon 捕获）\n- choice_id: choice_rollout\n- answers:\n  - question_id: q1\n    selected: gradual = 分批发布\n- impacts: REQ-001, AC-001".to_string(),
            checkpoint_id: None,
            created_at: "2026-08-21T00:00:03Z".to_string(),
        },
    );
    session.artifact = Some(artifacts[3].payload.clone());
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    engine.artifact_versions = artifacts;
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "第 1 轮必须修复仍未关闭。".to_string(),
        summary: "第 1 轮追踪关系未关闭".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "ROUND-1-MUST-FIX-FULL-TEXT：REQ-001 必须追踪到 AC-001。".to_string(),
            evidence: "artifact v1 / REQ-001".to_string(),
            required_action: "补齐 REQ-001 -> AC-001 trace。".to_string(),
        }],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    });

    let author = engine
        .build_streaming_input("新的 author 请求", AuthorPromptMode::FullConversation)
        .expect("author prompt")
        .prompt;
    let revision = engine.build_revision_full_prompt(
        engine
            .session
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown())
            .expect("current artifact"),
        engine.latest_review_verdict.as_ref().expect("review verdict"),
        &engine.routing_reference_context(),
    );
    let reviewer = engine.build_review_input().expect("review prompt").prompt;

    for (entrypoint, prompt) in [
        ("author", &author),
        ("revision", &revision),
        ("reviewer", &reviewer),
    ] {
        assert!(
            prompt.contains("[历史压缩摘要 round=1]"),
            "{entrypoint} must summarize early rounds: {prompt}"
        );
        assert!(
            !prompt.contains("ROUND-1-USER-RAW"),
            "{entrypoint} must not replay early raw rounds: {prompt}"
        );
        assert!(prompt.contains("ROUND-3-USER-RAW"), "{entrypoint}: {prompt}");
        assert!(prompt.contains("ROUND-4-USER-RAW"), "{entrypoint}: {prompt}");
        assert!(prompt.contains("choice_rollout"), "{entrypoint}: {prompt}");
        assert!(prompt.contains("gradual = 分批发布"), "{entrypoint}: {prompt}");
        assert!(prompt.contains("# Story Artifact v4"), "{entrypoint}: {prompt}");
    }
    assert!(reviewer.contains("完整 canonical inputs：不得裁剪。"));
    assert!(reviewer.contains("ROUND-1-MUST-FIX-FULL-TEXT"));
    assert!(reviewer.contains("artifact v1 -> v2 相邻版本差异摘要"));

    let full_replay_len = engine
        .session
        .messages
        .iter()
        .map(|message| message.role.len() + message.content.len() + 5)
        .sum::<usize>();
    assert!(
        author.len() < full_replay_len + 5_000,
        "prompt character-count decrease is a proxy metric only, not a release gate"
    );
}
