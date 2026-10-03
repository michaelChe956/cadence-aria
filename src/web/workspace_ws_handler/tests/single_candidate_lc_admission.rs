//! C-1：LC 会话 SingleCandidate author 运行前的 admission 预检测试。
//!
//! 覆盖：
//! - LC 会话成员缺 `.claude/rules/language.md` → admission 预检在规则读取前
//!   转 waiting/Prepare 面（durable phase=Prepare、非终态 Failed、provider 零启动），
//!   而不是现行（修复前）的运行时终态 Failed；
//! - waiting 面可操作：补齐成员规则后同一会话重新 StartGeneration 可继续启动
//!   author，且 prompt 消费的是同一实际文件（不新增 fallback）；
//! - 单仓 legacy 路径行为不变由既有
//!   `single_candidate_author_rejects_missing_language_rules_before_provider_start`
//!   回归锁定（终态 Failed 语义保留）。

use super::single_candidate_provider_run::{
    ProviderRunFixture, single_candidate_context, single_candidate_markdown,
};
use super::*;

use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession,
};
use crate::product::logical_codebase::issue_selection::IssueCodebaseSelection;
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::provider_gateway::{
    PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource,
};
use crate::product::logical_codebase::store::LogicalCodebaseRecord;
use crate::product::logical_codebase::types::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId, MemberStatus,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use crate::product::logical_codebase::{
    GatewayRunAudit, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
    LogicalCodebaseProviderGateway, LogicalCodebaseStore, ProviderDialect, ProviderRef,
    SessionLaunchRequest, SessionPolicyAction,
};
use crate::product::models::RepositoryRecord;
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus};
use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};

// ---- 测试 doubles（与 provider_admission_preflight 测试同型，作用域隔离） ----

/// 恒放行 capability source：ClaudeCode→ClaudeCodeCliV1，snapshot ref 透传。
struct LcStaticCapabilitySource;

impl ProviderCapabilitySource for LcStaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.4.0".to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            resume_evidence:
                crate::product::logical_codebase::provider_gateway::ResumeEvidenceState::Confirmed,
        })
    }
}

/// 透传 target resolver：canonicalize worktree 后按请求形态重建 target。
struct LcPassThroughTargetResolver;

impl PolicyTargetResolver for LcPassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<
        crate::product::logical_codebase::policy::PolicyTarget,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        let canonical = std::fs::canonicalize(&request.target.worktree).map_err(|_| {
            crate::product::logical_codebase::ProviderGatewayError::Target(
                "worktree missing".to_string(),
            )
        })?;
        if request.target.logical_repository_id.is_empty() {
            Ok(crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(canonical))
        } else {
            Ok(
                crate::product::logical_codebase::policy::PolicyTarget::checkout(
                    request.target.logical_repository_id.clone(),
                    request.target.checkout_id.clone(),
                    canonical,
                ),
            )
        }
    }
}

struct LcStubSyncAdapter;

impl ProviderAdapter for LcStubSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
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

fn lc_always_available_gate() -> Arc<ProviderAvailabilityGate> {
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

/// 记录输入并输出预置 markdown 的 author provider（gateway registry 与
/// run-context registry 共用同一实例）。output 的 story/design 引用使用确定
/// 字面量 story_0001/design_0001：provider 必须先于夹具构造（gateway registry
/// 接线），而夹具 issue/spec 目录为空时顺序 id 恒定。
struct LcRecordingAuthorProvider {
    output: String,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
    for LcRecordingAuthorProvider
{
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        let _ = self.inputs.send(input);
        let output = self.output.clone();
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
}

// ---- LC 别名登记 ----

pub(super) struct AliasLcFixture {
    pub(super) lc_id: String,
    pub(super) member_id: LogicalRepositoryId,
    pub(super) checkout_id: RepositoryCheckoutId,
    pub(super) gateway: Arc<LogicalCodebaseProviderGateway>,
}

/// 在 legacy 别名位置登记单成员逻辑代码库（record/manifest/member/checkout/
/// selection/index/policy 权威记录）并组装 gateway。成员 checkout 锚定
/// `member_root`（即夹具 repository_root，聚合根的直接子目录）；聚合根取
/// `member_root` 的父目录。
pub(super) fn register_alias_logical_codebase(
    app_paths: &ProductAppPaths,
    member_root: &std::path::Path,
    physical_repository: &RepositoryRecord,
    gateway_author: Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter>,
) -> AliasLcFixture {
    let project_id = "project_0001";
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id);
    let aggregate_root = member_root
        .parent()
        .expect("member root must live under the aggregate root")
        .to_path_buf();

    // authority resolver 的 LC 存在性凭证：record.json。
    let record = LogicalCodebaseRecord {
        id: lc_id.clone(),
        name: "sc admission alias lc".to_string(),
        aggregate_root: aggregate_root.clone(),
        created_at: "2026-09-30T00:00:00Z".to_string(),
    };
    let record_root = app_paths.logical_codebase_record_root(project_id, &lc_id);
    std::fs::create_dir_all(&record_root).expect("create lc record root");
    let record_path = record_root.join("record.json");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&record).expect("serialize lc record"),
    )
    .expect("write lc record.json");

    let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    let canonical_member = std::fs::canonicalize(member_root).expect("canonical member root");
    let authority = LogicalCodebaseStore::new(app_paths.clone());
    let manifest =
        LogicalCodebaseManifest::new(project_id, aggregate_root.clone(), vec![member_id]);
    authority
        .save_manifest(project_id, &manifest)
        .expect("save manifest");
    let now = "2026-09-30T00:00:00Z".to_string();
    authority
        .save_member(
            project_id,
            &CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: physical_repository.id.clone(),
                alias: "sc-member".to_string(),
                role: "member".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    member_root,
                    member_root.join(".git"),
                    Some("ssh://git@example.test/sc/member.git".to_string()),
                ),
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
        .expect("save member");
    authority
        .save_checkout(
            project_id,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: physical_repository.id.clone(),
                kind: CheckoutKind::Main,
                canonical_path: canonical_member,
                checkout_path_hash: "sha256:sc-admission-checkout".to_string(),
                git_dir_identity: "sha256:sc-admission-git-dir".to_string(),
                revision: Some("abc123".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save checkout");

    // 聚合索引（Logical WorkItemPlan 的 workspace context 聚合视野输入）。
    let index = crate::product::logical_codebase::aggregate_index::AggregateIndexRecord::building(
        "aggregate_index_0001".to_string(),
        project_id.to_string(),
        1,
        vec![
            crate::product::logical_codebase::aggregate_index::AggregateIndexMemberSnapshot::indexed(
                member_id,
                checkout_id,
                "abc123".to_string(),
                false,
                now.clone(),
            ),
        ],
        now.clone(),
    );
    let index_store = crate::product::logical_codebase::aggregate_index::AggregateIndexStore::new(
        app_paths.clone(),
    );
    index_store
        .create(project_id, index.clone())
        .expect("create aggregate index");
    let mut activated = index;
    activated.status =
        crate::product::logical_codebase::aggregate_index::AggregateIndexStatus::Active;
    index_store
        .replace_active(project_id, activated)
        .expect("activate aggregate index");
    // 测试夹具不依赖真实 codegraph CLI：标记 Degraded（last-known-good），
    // freshness 评估短路（不探测 git、不触发 codegraph sync），聚合视野照常可读。
    index_store
        .mark_status(
            project_id,
            "aggregate_index_0001",
            crate::product::logical_codebase::aggregate_index::AggregateIndexStatus::Degraded,
            Some("test fixture degraded last-known-good index".to_string()),
        )
        .expect("mark aggregate index degraded");
    // issue 显式单成员选择（WorkItemPlan 路由面 target 解析输入）。
    let selection = IssueCodebaseSelection::explicit(
        project_id,
        "issue_0001",
        vec![member_id],
        Vec::new(),
        Vec::new(),
        None,
    );
    IssueCodebaseSelectionStore::new(app_paths.clone())
        .save(&selection)
        .expect("save issue selection");

    // 聚合 policy artifact（admission 预检步骤 3）。
    let policy_store = AggregatePolicyArtifactStore::new(app_paths.clone());
    policy_store
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");

    // gateway：测试 double capability/target resolver；registry 携带真实
    // author provider（LC author 经 gateway 启动）。
    let mut gateway_registry = ProviderRegistry::new();
    gateway_registry.register(ProviderName::ClaudeCode, gateway_author);
    let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
        policy_store,
        Arc::new(LcStaticCapabilitySource),
        Arc::new(LcPassThroughTargetResolver),
        Arc::new(gateway_registry),
        Arc::new(LcStubSyncAdapter),
        lc_always_available_gate(),
        Arc::new(GatewayRunAudit::new()),
        std::fs::canonicalize(&aggregate_root).expect("canonical authority root"),
    ));

    AliasLcFixture {
        lc_id,
        member_id,
        checkout_id,
        gateway,
    }
}

/// 等待下一条 type=="error" 的出站帧并返回其 JSON。
async fn next_error_frame(outbound_rx: &mut mpsc::Receiver<OutboundControl>) -> serde_json::Value {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let outbound = outbound_rx
                .recv()
                .await
                .expect("admission waiting must emit an outbound frame");
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
    .expect("error frame")
}

fn durable_session(fixture: &ProviderRunFixture) -> crate::product::models::WorkspaceSessionRecord {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload single-candidate session")
}

/// C-1 红→绿主用例：LC 会话成员缺 language rules → waiting/Prepare 面，
/// 不是终态 Failed；provider 保持零启动。
#[tokio::test]
async fn lc_missing_member_language_rules_converts_to_waiting_prepare_face() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    // 成员缺 .claude/rules/language.md（write_member_language_rules=false）。
    let fixture = ProviderRunFixture::new_logical(false, provider.clone());
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let error = next_error_frame(&mut outbound_rx).await;
    let message = error["message"].as_str().expect("error message");
    assert!(
        message.contains("member_language_rules_missing"),
        "waiting 消息必须携带预检判别码，got: {message}"
    );
    let canonical_member_root =
        std::fs::canonicalize(fixture.member_checkout_root()).expect("canonical member root");
    assert!(
        message.contains(&canonical_member_root.display().to_string()),
        "waiting 消息必须指向缺失的成员材料路径，got: {message}"
    );

    // waiting/Prepare 面：durable phase 回到 Prepare、状态非 Failed（可操作等待，
    // 修复前的现行行为是终态 Failed + status=Failed）。
    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "LC 缺成员规则必须转 waiting/Prepare 面，got status={:?}",
        durable.status
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);

    // provider 零启动。
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "LC admission waiting 必须 reject before provider startup"
    );
}

/// waiting 面可操作：补齐成员规则后，同一会话重新 StartGeneration 能继续启动
/// author，且 prompt 消费同一实际文件（成员 checkout 的 language.md 原文）。
#[tokio::test]
async fn lc_waiting_face_resumes_after_member_rules_restored() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical(false, provider.clone());
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider.clone());
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;
    let error = next_error_frame(&mut outbound_rx).await;
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("member_language_rules_missing")
    );
    assert_eq!(
        durable_session(&fixture).single_candidate_phase,
        Some(SingleCandidatePhase::Prepare)
    );

    // 补齐成员规则（同一实际文件），再次发起生成。
    let rules_path = fixture
        .member_checkout_root()
        .join(".claude/rules/language.md");
    std::fs::write(
        &rules_path,
        "## 语言规则\n\n- **必须使用中文** - LC waiting 恢复后的成员规则原文。\n",
    )
    .expect("restore member language rules");
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(2), input_rx.recv())
        .await
        .expect("restored rules must let the author provider start")
        .expect("author provider input");
    assert!(
        full_input
            .prompt
            .contains("LC waiting 恢复后的成员规则原文"),
        "author prompt 必须消费成员 checkout 的同一实际文件，got prompt tail: {}",
        &full_input.prompt[full_input.prompt.len().saturating_sub(400)..]
    );
    // 恢复轮不再产生 waiting 错误帧（run 健康推进，出站保持静默或仅非 error 帧）。
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(200), outbound_rx.recv()).await,
            Ok(Some(OutboundControl::Text(json)))
                if serde_json::from_str::<serde_json::Value>(&json)
                    .map(|value| value["type"] == "error")
                    .unwrap_or(false)
        ),
        "restored rules run must not emit another waiting error"
    );
}

/// 成员规则齐备的 LC 会话：预检通过后零额外行为差异，author 正常启动并消费
/// 同一实际文件（首启即达，不先经 waiting）。
#[tokio::test]
async fn lc_run_with_member_rules_present_starts_author_directly() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical(true, provider.clone());
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(2), input_rx.recv())
        .await
        .expect("rules-present LC session must start the author provider")
        .expect("author provider input");
    assert!(
        full_input.prompt.contains("必须使用中文"),
        "author prompt must consume the same on-disk language rules"
    );
}

/// Task 2.8（REQ-PLN-01/07，planning snapshot 贯穿）：LC plan author（SC
/// markdown 链与 WorkItemPlan outline 链共用同一 launch/start 通道）provider
/// spawn cwd 必须是 canonical 聚合根（manifest `provider_context_root`），
/// target worktree 保持成员 checkout——cwd/target 分离贯穿 prompt→launch→
/// spawn 全链，不得以成员 checkout 兼任 cwd。
#[tokio::test]
async fn lc_plan_author_run_spawns_from_root_cwd_with_member_target() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical(true, provider.clone());
    let (context, _outbound_rx) = single_candidate_context(&fixture, provider);

    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
        },
    )
    .await;

    let full_input = tokio::time::timeout(std::time::Duration::from_secs(2), input_rx.recv())
        .await
        .expect("LC plan author run must start the provider")
        .expect("author provider input");
    let canonical_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    let canonical_member =
        std::fs::canonicalize(fixture.member_checkout_root()).expect("canonical member root");
    assert_eq!(
        full_input.working_directory.as_deref(),
        Some(canonical_root.as_path()),
        "LC plan author spawn cwd 必须是 canonical 聚合根（root cwd），got: {:?}",
        full_input.working_directory
    );
    assert_eq!(
        full_input.working_dir, canonical_member,
        "target worktree 保持成员 checkout（cwd/target 分离）"
    );
}
