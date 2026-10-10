//! C-1/路 C（Task 4，REQ-REG-12）：LC 会话 SingleCandidate author 运行前的
//! admission 预检与实际消费路径测试。
//!
//! 覆盖：
//! - LC 会话成员缺 `.claude/rules/language.md` → admission 在规则消费前转
//!   waiting/Prepare 面（durable phase=Prepare、非终态 Failed、provider 零启动），
//!   而不是现行（修复前）的运行时终态 Failed；
//! - waiting 面可操作：补齐成员规则后同一会话重新 StartGeneration 可继续启动
//!   author，且 prompt 消费的是同一实际文件（不新增 fallback）；
//! - 路 C 矩阵（Task 4）：聚焦成员规则齐备而其他 active 成员缺失 → 继续，
//!   仅按实际消费路径判定；多 target 无唯一 focus（读根）→ 按聚合根文件
//!   判定；实际消费文件不可读/目录/非法 UTF-8 → provider 前停等；
//! - 单仓 legacy 路径行为不变由既有
//!   `single_candidate_author_rejects_missing_language_rules_before_provider_start`
//!   回归锁定（终态 Failed 语义保留）。

use super::single_candidate_provider_run::{
    LogicalAliasFocus, MemberRuleMaterial, ProviderRunFixture, single_candidate_context,
    single_candidate_markdown,
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

/// 恒放行 capability source:ClaudeCode→ClaudeCodeCliV1,snapshot ref 透传
/// (Task 2b 分格形状:launch/resume/write_boundary 恒 Confirmed)。
struct LcStaticCapabilitySource;

impl LcStaticCapabilitySource {
    fn capability(provider: &ProviderRef, action: SessionPolicyAction) -> ProviderCapability {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
        ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.4.0".to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            wire_dialect: ProviderWireDialect::ClaudeCodeStreamJson,
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
        }
    }
}

impl ProviderCapabilitySource for LcStaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, crate::product::logical_codebase::ProviderGatewayError> {
        use crate::product::logical_codebase::provider_gateway::ProviderGatewayError;
        if provider.provider_type
            != crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode
        {
            return Err(ProviderGatewayError::UnsupportedCapability(
                crate::product::logical_codebase::provider_gateway::PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE
                    .to_string(),
            ));
        }
        Ok(Self::capability(
            provider,
            SessionPolicyAction::PlanningReadOnly,
        ))
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
    /// Task 1b 段③:prepared launch 经 `start_validated` 分发——拆出 input
    /// 后与裸 `start` 同路径(记录/输出语义保持)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }
}

// ---- LC 别名登记 ----

pub(super) struct AliasLcFixture {
    /// 别名登记的 LC id；当前无读取点，保留 fixture 形状。
    #[allow(dead_code)]
    pub(super) lc_id: String,
    /// 成员逻辑身份（顺序=ordinal；成员 0 是 `session.repository_path` 锚定的
    /// 聚焦成员），与 `checkout_ids` 成对。
    pub(super) member_ids: Vec<LogicalRepositoryId>,
    pub(super) checkout_ids: Vec<RepositoryCheckoutId>,
    pub(super) gateway: Arc<LogicalCodebaseProviderGateway>,
}

/// 在 legacy 别名位置登记逻辑代码库（record/manifest/member/checkout/
/// selection/index/policy 权威记录）并组装 gateway。`member_roots` 顺序即
/// ordinal（成员 0 即夹具 repository_root）；各成员 checkout 必须是聚合根的
/// 直接子目录，聚合根取成员 0 的父目录。`focus` 决定 issue 显式选择的
/// focus 形态（WorkItemPlan 路由面 target 解析输入：FirstMember+多成员=
/// 唯一 focus 解析到成员 0；NonUnique=无唯一 focus 解析聚合根视图）。
pub(super) fn register_alias_logical_codebase(
    app_paths: &ProductAppPaths,
    member_roots: &[std::path::PathBuf],
    physical_repositories: &[RepositoryRecord],
    focus: &super::single_candidate_provider_run::LogicalAliasFocus,
    gateway_author: Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter>,
) -> AliasLcFixture {
    assert!(
        !member_roots.is_empty(),
        "logical alias registration needs at least one member"
    );
    assert_eq!(
        member_roots.len(),
        physical_repositories.len(),
        "logical alias members and physical repositories must pair"
    );
    let project_id = "project_0001";
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id);
    let aggregate_root = member_roots[0]
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

    let member_ids = member_roots
        .iter()
        .map(|_| LogicalRepositoryId(uuid::Uuid::new_v4()))
        .collect::<Vec<_>>();
    let checkout_ids = member_roots
        .iter()
        .map(|_| RepositoryCheckoutId(uuid::Uuid::new_v4()))
        .collect::<Vec<_>>();
    let authority = LogicalCodebaseStore::new(app_paths.clone());
    let manifest =
        LogicalCodebaseManifest::new(project_id, aggregate_root.clone(), member_ids.clone());
    authority
        .save_manifest(project_id, &manifest)
        .expect("save manifest");
    let now = "2026-09-30T00:00:00Z".to_string();
    for ((index, member_root), (member_id, checkout_id)) in member_roots
        .iter()
        .enumerate()
        .zip(member_ids.iter().zip(&checkout_ids))
    {
        authority
            .save_member(
                project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: *member_id,
                    physical_repository_id: physical_repositories[index].id.clone(),
                    alias: format!("sc-member-{index}"),
                    role: "member".to_string(),
                    ordinal: index as u32 + 1,
                    source_identity: RepositorySourceIdentity::from_git_parts(
                        member_root,
                        member_root.join(".git"),
                        Some(format!("ssh://git@example.test/sc/member-{index}.git")),
                    ),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![*checkout_id],
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
                    checkout_id: *checkout_id,
                    logical_repository_id: *member_id,
                    physical_repository_id: physical_repositories[index].id.clone(),
                    kind: CheckoutKind::Main,
                    canonical_path: std::fs::canonicalize(member_root)
                        .expect("canonical member root"),
                    checkout_path_hash: format!("sha256:sc-admission-checkout-{index}"),
                    git_dir_identity: format!("sha256:sc-admission-git-dir-{index}"),
                    revision: Some("abc123".to_string()),
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .expect("save checkout");
    }

    // 聚合索引（Logical WorkItemPlan 的 workspace context 聚合视野输入）。
    let index = crate::product::logical_codebase::aggregate_index::AggregateIndexRecord::building(
        "aggregate_index_0001".to_string(),
        project_id.to_string(),
        1,
        member_ids
            .iter()
            .zip(&checkout_ids)
            .map(|(member_id, checkout_id)| {
                crate::product::logical_codebase::aggregate_index::AggregateIndexMemberSnapshot::indexed(
                    *member_id,
                    *checkout_id,
                    "abc123".to_string(),
                    false,
                    now.clone(),
                )
            })
            .collect(),
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
    // issue 显式成员选择（WorkItemPlan 路由面 target 解析输入）。focus 形态：
    // FirstMember+多成员 → 唯一 focus 解析到成员 0；单成员与既有夹具一致
    // （focus 空 → 唯一 target）；NonUnique → 全员 focus，无唯一 focus 解析
    // 聚合根视图。
    let focus_ids = match focus {
        super::single_candidate_provider_run::LogicalAliasFocus::FirstMember
            if member_ids.len() >= 2 =>
        {
            vec![member_ids[0]]
        }
        super::single_candidate_provider_run::LogicalAliasFocus::FirstMember => Vec::new(),
        super::single_candidate_provider_run::LogicalAliasFocus::NonUnique => member_ids.clone(),
    };
    let selection = IssueCodebaseSelection::explicit(
        project_id,
        "issue_0001",
        member_ids.clone(),
        Vec::new(),
        focus_ids,
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
        member_ids,
        checkout_ids,
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

/// 路 C（Task 4，REQ-REG-12）：聚焦成员的规则材料齐备而其他 active 成员缺失
/// → SC 仅按实际消费路径判定必需材料——author 正常启动，prompt 消费聚焦
/// 成员 checkout 的同一实际文件；其他成员缺失不阻断、不产生成员准备动作。
#[tokio::test]
async fn lc_sc_focus_member_rules_present_other_member_missing_continues() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical_multi(
        vec![
            MemberRuleMaterial::Present(
                "## 语言规则\n\n- **必须使用中文** - 聚焦成员实际消费的规则原文（focus-member-marker）。\n"
                    .to_string(),
            ),
            MemberRuleMaterial::Absent,
        ],
        None,
        LogicalAliasFocus::FirstMember,
        provider.clone(),
    );
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
        .expect("其他成员缺失不得阻断聚焦成员规则齐备的 SC author 启动")
        .expect("author provider input");
    assert!(
        full_input
            .prompt
            .contains("聚焦成员实际消费的规则原文（focus-member-marker）"),
        "author prompt 必须消费聚焦成员 checkout 的同一实际文件，got prompt tail: {}",
        &full_input.prompt[full_input.prompt.len().saturating_sub(400)..]
    );
    // 不产生 waiting/错误帧（run 健康推进，其他成员缺失只是非阻断审计事实）。
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(200), outbound_rx.recv()).await,
            Ok(Some(OutboundControl::Text(json)))
                if serde_json::from_str::<serde_json::Value>(&json)
                    .map(|value| value["type"] == "error")
                    .unwrap_or(false)
        ),
        "聚焦成员可读时其他成员缺失不得产生 waiting 错误帧"
    );
}

/// 路 C：多 target 无唯一 focus（focus=全员）→ repository 解析为聚合根视图，
/// SC 必需正文=聚合根 `.claude/rules/language.md`——根文件可读时继续，不因
/// 任意成员规则缺失停等；prompt 消费根文件原文。
#[tokio::test]
async fn lc_sc_multi_target_without_unique_focus_consumes_root_rules() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical_multi(
        vec![MemberRuleMaterial::Absent, MemberRuleMaterial::Absent],
        Some(
            "## 语言规则\n\n- **必须使用中文** - 聚合根实际消费的规则原文（aggregate-root-marker）。\n"
                .to_string(),
        ),
        LogicalAliasFocus::NonUnique,
        provider.clone(),
    );
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
        .expect("根规则可读时多 target 聚合根视图 SC 必须继续启动 author")
        .expect("author provider input");
    assert!(
        full_input
            .prompt
            .contains("聚合根实际消费的规则原文（aggregate-root-marker）"),
        "聚合根视图的 author prompt 必须消费根 .claude/rules/language.md 原文，got prompt tail: {}",
        &full_input.prompt[full_input.prompt.len().saturating_sub(400)..]
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(200), outbound_rx.recv()).await,
            Ok(Some(OutboundControl::Text(json)))
                if serde_json::from_str::<serde_json::Value>(&json)
                    .map(|value| value["type"] == "error")
                    .unwrap_or(false)
        ),
        "根文件可读时不得因成员规则缺失停等"
    );
}

/// 路 C：聚合根视图下根规则文件缺失 → SC 实际消费文件 waiting 合同
/// （member_language_rules_missing 语义，detail 指明聚合根来源路径）——
/// provider 零启动、durable Prepare 面，不落终态 Failed。
#[tokio::test]
async fn lc_sc_multi_target_root_rules_missing_waits_before_provider() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical_multi(
        vec![MemberRuleMaterial::Absent, MemberRuleMaterial::Absent],
        None,
        LogicalAliasFocus::NonUnique,
        provider.clone(),
    );
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
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    assert!(
        message.contains(&canonical_aggregate_root.display().to_string()),
        "waiting 消息必须指向实际消费的聚合根来源路径，got: {message}"
    );
    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "聚合根规则缺失必须是 waiting/Prepare 面，got status={:?}",
        durable.status
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "聚合根规则缺失必须 reject before provider startup"
    );
}

/// 路 C：实际消费文件存在但非法 UTF-8 → provider 前停等（不是预检过后
/// 运行时终态 Failed）；detail 区分实际读取错误。
#[tokio::test]
async fn lc_sc_consumed_rule_file_invalid_utf8_waits_before_provider() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical_multi(
        vec![MemberRuleMaterial::InvalidUtf8],
        None,
        LogicalAliasFocus::FirstMember,
        provider.clone(),
    );
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
        "非法 UTF-8 必须按实际消费文件停等（判别码），got: {message}"
    );
    assert!(
        message.contains("UTF-8"),
        "detail 必须区分实际读取错误（非法 UTF-8），got: {message}"
    );
    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "非法 UTF-8 必须转 waiting/Prepare 面，got status={:?}",
        durable.status
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "非法 UTF-8 必须 reject before provider startup"
    );
}

/// 路 C：实际消费文件路径是目录 → provider 前停等（目录错误类归入实际
/// 消费文件 waiting 合同）。
#[tokio::test]
async fn lc_sc_consumed_rule_file_directory_waits_before_provider() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical_multi(
        vec![MemberRuleMaterial::Directory],
        None,
        LogicalAliasFocus::FirstMember,
        provider.clone(),
    );
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
        "目录错误必须按实际消费文件停等（判别码），got: {message}"
    );
    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "目录错误必须转 waiting/Prepare 面，got status={:?}",
        durable.status
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "目录错误必须 reject before provider startup"
    );
}

/// 路 C：实际消费文件存在但无读取权限 → provider 前停等（权限错误类归入
/// 实际消费文件 waiting 合同）。运行环境无法制造 EACCES（如 root）时跳过。
#[tokio::test]
async fn lc_sc_consumed_rule_file_unreadable_waits_before_provider() {
    use std::os::unix::fs::PermissionsExt;

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let fixture = ProviderRunFixture::new_logical_multi(
        vec![MemberRuleMaterial::Present(
            "## 语言规则\n\n- **必须使用中文** - 待收紧权限的成员规则。\n".to_string(),
        )],
        None,
        LogicalAliasFocus::FirstMember,
        provider.clone(),
    );
    let rules_path = fixture
        .member_checkout_root()
        .join(".claude/rules/language.md");
    std::fs::set_permissions(&rules_path, std::fs::Permissions::from_mode(0o000))
        .expect("revoke member language rules read permission");
    if std::fs::read_to_string(&rules_path).is_ok() {
        eprintln!(
            "skipping: environment cannot produce permission-denied reads (root?) at {}",
            rules_path.display()
        );
        return;
    }
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
        "权限错误必须按实际消费文件停等（判别码），got: {message}"
    );
    assert!(
        message.contains("Permission denied"),
        "detail 必须区分实际读取错误（权限拒绝），got: {message}"
    );
    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "权限错误必须转 waiting/Prepare 面，got status={:?}",
        durable.status
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "权限错误必须 reject before provider startup"
    );
}
