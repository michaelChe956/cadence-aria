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

/// 6.1（方案 A 附带）：Alias LC 夹具的 gateway 组装覆盖项。默认全空——
/// 既有夹具行为零变化；trust 首次失败只读指导用例按需注入。
#[derive(Default, Clone)]
pub(super) struct AliasLcGatewayExtras {
    /// 覆盖 capability source（如按 provider 报真实 dialect 的
    /// `LcDialectCapabilitySource`，让 gateway 的只读 trust 复验按
    /// `provider_name_for_dialect` 命中 Codex/Kimi 的 workspace trust 面）。
    pub(super) capability: Option<Arc<dyn ProviderCapabilitySource>>,
    /// 隔离 home（空 tempdir）——组装只读 trust source（真实 Codex/Kimi
    /// adapter `for_home`，绝不读写真实用户 home）。
    pub(super) trust_home: Option<std::path::PathBuf>,
    /// 覆盖 availability gate（负例：provider 不健康时其它
    /// ProviderUnavailable 不得误当 trust 缺失）。
    pub(super) availability: Option<Arc<ProviderAvailabilityGate>>,
}

/// 在 legacy 别名位置登记逻辑代码库（record/manifest/member/checkout/
/// selection/index/policy 权威记录）并组装 gateway。既有行为入口——
/// 零覆盖项；细节见 `register_alias_logical_codebase_with_extras`。
#[allow(dead_code)] // 既有行为入口(零覆盖项);矩阵已转用 _with_extras 变体。
pub(super) fn register_alias_logical_codebase(
    app_paths: &ProductAppPaths,
    member_roots: &[std::path::PathBuf],
    physical_repositories: &[RepositoryRecord],
    focus: &super::single_candidate_provider_run::LogicalAliasFocus,
    gateway_author: Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter>,
) -> AliasLcFixture {
    register_alias_logical_codebase_with_extras(
        app_paths,
        member_roots,
        physical_repositories,
        focus,
        gateway_author,
        AliasLcGatewayExtras::default(),
    )
}

/// [`register_alias_logical_codebase`] 的 gateway 覆盖项变体（6.1）：
/// `extras.trust_home` 注入只读 trust source（隔离 home 的真实
/// Codex/Kimi adapter + LC 作用域 receipt store），capability/availability
/// 可换可控 double。`member_roots` 顺序即 ordinal（成员 0 即夹具
/// repository_root）；各成员 checkout 必须是聚合根的直接子目录，聚合根取
/// 成员 0 的父目录。`focus` 决定 issue 显式选择的 focus 形态
/// （WorkItemPlan 路由面 target 解析输入：FirstMember+多成员=唯一 focus
/// 解析到成员 0；NonUnique=无唯一 focus 解析聚合根视图）。
pub(super) fn register_alias_logical_codebase_with_extras(
    app_paths: &ProductAppPaths,
    member_roots: &[std::path::PathBuf],
    physical_repositories: &[RepositoryRecord],
    focus: &super::single_candidate_provider_run::LogicalAliasFocus,
    gateway_author: Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter>,
    extras: AliasLcGatewayExtras,
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
    // author provider（LC author 经 gateway 启动）。6.1 extras：dialect
    // capability/可控 availability/隔离 home 只读 trust source 可覆盖。
    let capability_source: Arc<dyn ProviderCapabilitySource> = extras
        .capability
        .unwrap_or_else(|| Arc::new(LcStaticCapabilitySource));
    let availability_gate = extras.availability.unwrap_or_else(
        crate::web::handlers::automation_enrollment_test_support::always_available_gate,
    );
    let mut gateway_registry = ProviderRegistry::new();
    gateway_registry.register(ProviderName::ClaudeCode, gateway_author);
    let gateway = LogicalCodebaseProviderGateway::with_audit(
        policy_store,
        capability_source,
        Arc::new(LcPassThroughTargetResolver),
        Arc::new(gateway_registry),
        Arc::new(LcStubSyncAdapter),
        availability_gate,
        Arc::new(GatewayRunAudit::new()),
        std::fs::canonicalize(&aggregate_root).expect("canonical authority root"),
    );
    // 6.1：隔离 home 的只读 trust source（真实 Codex/Kimi adapter for_home，
    // 绝不触碰真实用户 home）；LC 作用域与 gateway lc_scope 同源。
    let gateway = match extras.trust_home {
        Some(trust_home) => {
            let source =
                crate::product::logical_codebase::provider_trust::ReadonlyProviderTrustSource::new(
                    app_paths.clone(),
                    Some(lc_id.clone()),
                    vec![
                        Arc::new(
                            crate::product::logical_codebase::provider_trust_adapters::CodexTrustAdapter::for_home(&trust_home),
                        ),
                        Arc::new(
                            crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::for_home(&trust_home),
                        ),
                    ],
                );
            gateway.with_readonly_lc_facts(
                crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                    app_paths.clone(),
                    &lc_id,
                ),
                Arc::new(source),
                Some(lc_id.clone()),
            )
        }
        None => gateway,
    };
    let gateway = Arc::new(gateway);

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

/// 读取下一条指定 type 的出站帧（跳过其它帧；2s 超时 fail-closed）。
async fn next_frame_of_type(
    outbound_rx: &mut mpsc::Receiver<OutboundControl>,
    frame_type: &str,
) -> serde_json::Value {
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
            if value["type"] == frame_type {
                return value;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{frame_type} frame"))
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
            resume_recovery: None,
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
            resume_recovery: None,
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
            resume_recovery: None,
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
            resume_recovery: None,
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
            resume_recovery: None,
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
            resume_recovery: None,
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
            resume_recovery: None,
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
            resume_recovery: None,
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
            resume_recovery: None,
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
            resume_recovery: None,
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
            resume_recovery: None,
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

// ---- 6.1（方案 A 附带，REQ-REG-15「新根首次生成带指导但不自动登记」）----

/// 按 provider 报真实 adapter/wire dialect 的 capability source double：
/// Codex→CodexCliV1、KimiCode→KimiAcpV1——gateway 的只读 trust 复验
/// （#4b）按 `provider_name_for_dialect` 命中对应 provider 的用户级
/// workspace trust 面（默认 double 恒 ClaudeCodeCliV1，不会触发 trust）。
pub(super) struct LcDialectCapabilitySource;

impl LcDialectCapabilitySource {
    fn capability(provider: &ProviderRef, action: SessionPolicyAction) -> ProviderCapability {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
        let (adapter_dialect, wire_dialect) = match provider.provider_type {
            crate::product::logical_codebase::ProviderRefType::ClaudeCode => (
                ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            crate::product::logical_codebase::ProviderRefType::Codex => (
                ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            crate::product::logical_codebase::ProviderRefType::Pi => {
                (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc)
            }
            crate::product::logical_codebase::ProviderRefType::KimiCode => {
                (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp)
            }
        };
        ProviderCapability {
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
        }
    }
}

impl ProviderCapabilitySource for LcDialectCapabilitySource {
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
        Ok(Self::capability(
            provider,
            SessionPolicyAction::PlanningReadOnly,
        ))
    }
}

/// 可控 availability gate：ClaudeCode 恒健康，Codex/Kimi 按参数。
pub(super) fn lc_scoped_availability_gate(
    codex_available: bool,
    kimi_available: bool,
) -> Arc<ProviderAvailabilityGate> {
    struct Static(Arc<ProviderHealthSnapshot>);
    impl ProviderHealthSource for Static {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }
        fn degraded(&self) -> bool {
            false
        }
    }
    let checked_at = chrono::Utc::now();
    let entry = |provider, available| ProviderHealthEntry {
        provider,
        command: "stub".to_string(),
        available,
        version: Some("1.0".to_string()),
        reason_code: None,
        reason: None,
        checked_at,
    };
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [
            entry(ProviderName::ClaudeCode, true),
            entry(ProviderName::Codex, codex_available),
            entry(ProviderName::KimiCode, kimi_available),
        ]
        .into_iter()
        .collect(),
    });
    Arc::new(ProviderAvailabilityGate::new(Arc::new(Static(snapshot))))
}

/// 递归快照（路径 → 文件内容 sha256）：断言「零 trust 写」用——隔离
/// home 与 LC trust fact 目录在首次生成前后逐字节不变。
fn snapshot_tree(root: &std::path::Path) -> Vec<(String, String)> {
    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("sha256:{:x}", Sha256::digest(bytes))
    }
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                let digest = std::fs::read(&path)
                    .map(|bytes| sha256_hex(&bytes))
                    .unwrap_or_else(|error| format!("read-error:{error}"));
                out.push((path.display().to_string(), digest));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

/// LC trust durable 事实目录（legacy 别名 LC 作用域 = project 级
/// `logical-codebase/`，含 provider-trust 的 waiting/ownership/audit）。
fn trust_facts_root(fixture: &ProviderRunFixture) -> std::path::PathBuf {
    fixture.app_paths.logical_codebase_root("project_0001")
}

/// 指定 author 的会话上下文（run-context registry 注册该 provider，
/// StartGeneration 的 provider_config 锁定同一 author）。
fn single_candidate_context_for_author(
    fixture: &ProviderRunFixture,
    author: ProviderName,
    provider: Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter>,
) -> (WorkspaceInboundContext, mpsc::Receiver<OutboundControl>) {
    let mut registry = ProviderRegistry::new();
    registry.register(author, provider);
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
                fixture.root_path(),
                crate::web::runtime::WebRuntime::new_fake(fixture.root_path()),
            ),
            engine: fixture.engine.clone(),
            run_context,
            outbound_tx,
            session_id: fixture.record.id.clone(),
        },
        outbound_rx,
    )
}

fn author_provider_config(author: ProviderName) -> ProviderConfigSnapshot {
    ProviderConfigSnapshot {
        author,
        reviewer: None,
        review_rounds: 0,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    }
}

/// 6.1 红→绿主用例（Codex）：全新 root、其它准入门满足且隔离 home 无
/// 预登记 → 首次生成仍 fail-closed（provider 零启动、真实 home 与 trust
/// durable 事实快照不变），但 waiting 消息携带同源 manual_guidance（真实
/// 工件路径／本根 trust 键／人工登记形状）与生成场景 retry_action
/// 「人工登记…显式重新生成」，而不是裸 ProviderUnavailable 折叠。
#[tokio::test]
async fn lc_first_generation_codex_missing_trust_shows_manual_registration_guidance() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    // 隔离 home：真实 Codex/Kimi adapter 指向空 tempdir，无预登记。
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider.clone(),
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(lc_scoped_availability_gate(true, true)),
        },
    );
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    let home_before = snapshot_tree(trust_home.path());
    let trust_facts_before = snapshot_tree(&trust_facts_root(&fixture));

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::Codex, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::Codex),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let error = next_error_frame(&mut outbound_rx).await;
    let message = error["message"].as_str().expect("error message");
    assert!(
        message.contains("provider_trust_missing"),
        "trust 缺失必须以稳定判别码 provider_trust_missing 上浮，got: {message}"
    );
    assert!(
        !message.contains("provider_gateway_unavailable"),
        "trust 缺失不得再以裸 ProviderUnavailable 折叠呈现，got: {message}"
    );
    // 同源 manual_guidance：真实 adapter 工件路径＋本根 trust 键＋人工形状。
    assert!(
        message.contains(
            &trust_home
                .path()
                .join(".codex")
                .join("config.toml")
                .display()
                .to_string()
        ),
        "指导必须指向隔离 home 的真实 Codex config.toml 路径，got: {message}"
    );
    assert!(
        message.contains("[projects.\"")
            && message.contains(&canonical_aggregate_root.display().to_string()),
        "指导必须携带本 canonical root 的 trust 键（[projects.\"<root>\"]），got: {message}"
    );
    assert!(
        message.contains("trust_level = \"trusted\""),
        "指导必须给出人工登记形状（trust_level = \"trusted\"），got: {message}"
    );
    // 生成场景 retry_action：人工登记后显式重新生成，不混其它场景动作。
    assert!(
        message.contains("人工登记") && message.contains("显式重新生成"),
        "waiting 必须携带生成场景 retry_action（人工登记后显式重新生成），got: {message}"
    );
    assert!(
        !message.contains("重新确认") && !message.contains("重试初始化"),
        "generation 指导不得混入 recipe/confirmation 场景动作，got: {message}"
    );

    // durable waiting/Prepare 面（复用既有 AdmissionWaiting 回落），非终态 Failed。
    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "trust 缺失首次生成必须是 waiting/Prepare 面，got status={:?}",
        durable.status
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);

    // provider 零启动。
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "trust 缺失必须 reject before provider startup"
    );

    // 真实 home 与 trust durable 事实（waiting/ownership/audit）快照不变：
    // 零登记、零 trust 写（start_generation 不自动登记）。
    assert_eq!(
        home_before,
        snapshot_tree(trust_home.path()),
        "首次 generation trust 失败不得写隔离 home"
    );
    assert_eq!(
        trust_facts_before,
        snapshot_tree(&trust_facts_root(&fixture)),
        "首次 generation trust 失败不得写 trust durable 事实"
    );
}

/// 6.1（Kimi 变体）：同源指导来自 Kimi 真实 adapter——workspace-trust
/// 记录路径与稳定键 `wd_<basename>_<sha256[:12]>`（与 adapter 算法同源，
/// 不手抄第二份），人工形状为真实 JSON 记录形。
#[tokio::test]
async fn lc_first_generation_kimi_missing_trust_shows_manual_registration_guidance() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider.clone(),
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(lc_scoped_availability_gate(true, true)),
        },
    );
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    let home_before = snapshot_tree(trust_home.path());
    let trust_facts_before = snapshot_tree(&trust_facts_root(&fixture));

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::KimiCode, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::KimiCode),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let error = next_error_frame(&mut outbound_rx).await;
    let message = error["message"].as_str().expect("error message");
    assert!(
        message.contains("provider_trust_missing"),
        "Kimi 首次 trust 失败同样以 provider_trust_missing 上浮，got: {message}"
    );
    assert!(
        message.contains(
            &trust_home
                .path()
                .join(".kimi-code")
                .join("workspace-trust")
                .display()
                .to_string()
        ),
        "指导必须指向隔离 home 的 Kimi workspace-trust 目录，got: {message}"
    );
    assert!(
        message.contains("wd_aggregate-root_"),
        "指导必须携带 Kimi 稳定键形（wd_<basename>_<sha256[:12]>，同源生成），got: {message}"
    );
    assert!(
        message.contains("trustedAt")
            && message.contains(&canonical_aggregate_root.display().to_string()),
        "指导必须给出真实 JSON 记录形状（root/trustedAt），got: {message}"
    );
    assert!(
        message.contains("人工登记") && message.contains("显式重新生成"),
        "Kimi waiting 同样携带生成场景 retry_action，got: {message}"
    );

    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "Kimi trust 缺失首次生成必须是 waiting/Prepare 面，got status={:?}",
        durable.status
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "Kimi trust 缺失必须 reject before provider startup"
    );
    assert_eq!(
        home_before,
        snapshot_tree(trust_home.path()),
        "Kimi 首次 generation trust 失败不得写隔离 home"
    );
    assert_eq!(
        trust_facts_before,
        snapshot_tree(&trust_facts_root(&fixture)),
        "Kimi 首次 generation trust 失败不得写 trust durable 事实"
    );
}

/// 指引卡简化（owner 2026-10-12 直批）：trust 缺失首次生成的上浮不
/// 只有平铺 error message——紧随其后携带结构化 `trust_missing_guidance`
/// 帧（providers/canonical_root/reason_code/retry_action/command_markdown
/// 全部同源只读等待投影），前端「两行说明＋可复制命令窗」直渲，不再
/// 解析平铺串。entry 缺失给命令；用户值冲突不给（一键命令不得弱化
/// 「不覆盖用户条目」纪律）。
#[tokio::test]
async fn lc_first_generation_trust_missing_emits_structured_guidance_frame() {
    let (input_tx, _input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider.clone(),
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(lc_scoped_availability_gate(true, true)),
        },
    );
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::KimiCode, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::KimiCode),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    // 平铺 error 帧保留（旧客户端兼容，稳定判别码不变）。
    let error = next_error_frame(&mut outbound_rx).await;
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("provider_trust_missing"),
        "flat error frame keeps the stable code, got: {error}"
    );
    // 紧随其后的结构化指导帧。
    let guidance = next_frame_of_type(&mut outbound_rx, "trust_missing_guidance").await;
    assert_eq!(
        guidance["providers"],
        serde_json::json!(["kimi_code"]),
        "providers carry wire labels for the card headline: {guidance}"
    );
    assert_eq!(
        guidance["canonical_root"],
        canonical_aggregate_root.display().to_string(),
        "canonical root is the frozen authority root, got: {guidance}"
    );
    assert_eq!(guidance["reason_code"], "trust_entry_missing");
    assert!(
        guidance["retry_action"]
            .as_str()
            .expect("retry action")
            .contains("显式重新生成"),
        "retry action stays the generation-scoped wording: {guidance}"
    );
    let command = guidance["command_markdown"]
        .as_str()
        .expect("entry missing carries the copyable command");
    assert!(
        command.contains("mkdir -p")
            && command.contains(
                &trust_home
                    .path()
                    .join(".kimi-code")
                    .join("workspace-trust")
                    .display()
                    .to_string()
            ),
        "command markdown is the same-source kimi one-line shell: {command}"
    );

    // 用户值冲突变体：结构化帧同样上浮，但 command_markdown 缺席。
    let (input_tx, _input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let conflict_home = tempfile::tempdir().expect("isolated conflict home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider.clone(),
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(conflict_home.path().to_path_buf()),
            availability: Some(lc_scoped_availability_gate(true, true)),
        },
    );
    let conflict_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    let codex_config = conflict_home.path().join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_config.parent().expect("codex config dir"))
        .expect("create codex config dir");
    std::fs::write(
        &codex_config,
        format!(
            "[projects.\"{}\"]\ntrust_level = \"untrusted\"\n",
            conflict_root.display()
        ),
    )
    .expect("seed codex conflict");

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::Codex, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::Codex),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let _ = next_error_frame(&mut outbound_rx).await;
    let guidance = next_frame_of_type(&mut outbound_rx, "trust_missing_guidance").await;
    assert_eq!(guidance["providers"], serde_json::json!(["codex"]));
    assert_eq!(guidance["reason_code"], "codex_trust_key_conflict");
    assert!(
        guidance.get("command_markdown").is_none() || guidance["command_markdown"].is_null(),
        "conflict guidance must not carry a one-shot command: {guidance}"
    );
}

/// 6.1 负例（不误当）：Codex trust 已人工登记、但 availability gate 不
/// 健康——revalidate 的 ProviderUnavailable 来自 #9（availability）而非
/// #4b（trust）：保持 spawn_revalidation_drift 折叠，不附 trust 指导、
/// 不误标 provider_trust_missing。
#[tokio::test]
async fn lc_first_generation_other_unavailable_not_misattributed_as_trust_missing() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider.clone(),
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            // Codex 不健康：ProviderUnavailable 来自 availability gate。
            availability: Some(lc_scoped_availability_gate(false, true)),
        },
    );
    // 人工预登记 Codex trust（直接落用户级工件，模拟用户已手工登记）。
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    let codex_config = trust_home.path().join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_config.parent().expect("codex config dir"))
        .expect("create codex config dir");
    std::fs::write(
        &codex_config,
        format!(
            "[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
            canonical_aggregate_root.display()
        ),
    )
    .expect("pre-register codex trust");

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::Codex, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::Codex),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let error = next_error_frame(&mut outbound_rx).await;
    let message = error["message"].as_str().expect("error message");
    assert!(
        message.contains("spawn_revalidation_drift"),
        "非 trust 维度的 ProviderUnavailable 必须保持原判别码，got: {message}"
    );
    assert!(
        !message.contains("provider_trust_missing"),
        "availability 拒绝不得误标为 trust 缺失，got: {message}"
    );
    assert!(
        !message.contains("人工登记") && !message.contains("显式重新生成"),
        "非 trust 失败不得附人工登记指导，got: {message}"
    );
    // 同一 waiting 面（Prepare）与零启动不变。
    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare)
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "availability 拒绝同样必须 reject before provider startup"
    );
}

/// 6.1 负例（其它门原样）：Codex trust 已登记、availability 健康、但成员
/// language rules 缺失——member_language_rules_missing waiting 原样（判别
/// 码/材料路径），不携带 trust 指导（指导只绑定 trust 维度失败）。
#[tokio::test]
async fn lc_first_generation_member_rules_missing_keeps_waiting_without_trust_guidance() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        // 成员缺 .claude/rules/language.md（其它材料门）。
        false,
        provider.clone(),
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(lc_scoped_availability_gate(true, true)),
        },
    );
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    let codex_config = trust_home.path().join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_config.parent().expect("codex config dir"))
        .expect("create codex config dir");
    std::fs::write(
        &codex_config,
        format!(
            "[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
            canonical_aggregate_root.display()
        ),
    )
    .expect("pre-register codex trust");

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::Codex, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::Codex),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let error = next_error_frame(&mut outbound_rx).await;
    let message = error["message"].as_str().expect("error message");
    assert!(
        message.contains("member_language_rules_missing"),
        "成员规则缺失保持原判别码（其它材料门原样），got: {message}"
    );
    assert!(
        !message.contains("provider_trust_missing") && !message.contains("人工登记指导"),
        "成员规则 waiting 不携带 trust 指导，got: {message}"
    );
    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare)
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "成员规则缺失必须 reject before provider startup"
    );
}

/// 7.1 接线锚（conflict 态）：用户在隔离 home 预置非 trusted 冲突条目——
/// 首次生成经真实 admission/spawn 复验链拒收（`provider_trust_missing`
/// 同源判别+冲突 reason），带同源人工核验指导；用户冲突条目字节保持、
/// home 与 durable trust 事实快照不变（不覆盖、不补写、不伪造 capability），
/// provider 零启动。
#[tokio::test]
async fn lc_first_generation_conflict_trust_preserves_user_entry_without_writes() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider.clone(),
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(lc_scoped_availability_gate(true, true)),
        },
    );
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    // 用户冲突条目：目标根 projects 表持非 trusted 值。
    let codex_config = trust_home.path().join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_config.parent().expect("codex config dir"))
        .expect("create codex config dir");
    std::fs::write(
        &codex_config,
        format!(
            "[projects.\"{}\"]\ntrust_level = \"untrusted\"\n",
            canonical_aggregate_root.display()
        ),
    )
    .expect("seed codex conflict");
    let conflict_bytes = std::fs::read(&codex_config).expect("conflict bytes");
    let home_before = snapshot_tree(trust_home.path());
    let trust_facts_before = snapshot_tree(&trust_facts_root(&fixture));

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::Codex, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::Codex),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let error = next_error_frame(&mut outbound_rx).await;
    let message = error["message"].as_str().expect("error message");
    assert!(
        message.contains("provider_trust_missing"),
        "冲突态同样以稳定判别码上浮，got: {message}"
    );
    assert!(
        message.contains("codex_trust_key_conflict"),
        "原始冲突 reason（codex_trust_key_conflict）保留，got: {message}"
    );
    assert!(
        message.contains(&codex_config.display().to_string()),
        "冲突指导指向隔离 home 真实 Codex 工件路径，got: {message}"
    );

    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "冲突拒收保持 waiting/Prepare 面，got status={:?}",
        durable.status
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "冲突态必须 reject before provider startup"
    );
    // 用户冲突条目字节保持；home/durable trust 事实零写入。
    assert_eq!(
        std::fs::read(&codex_config).expect("conflict bytes after"),
        conflict_bytes,
        "用户冲突条目不得被覆盖"
    );
    assert_eq!(
        home_before,
        snapshot_tree(trust_home.path()),
        "冲突拒收不得写隔离 home"
    );
    assert_eq!(
        trust_facts_before,
        snapshot_tree(&trust_facts_root(&fixture)),
        "冲突拒收不得写 trust durable 事实"
    );
}

/// 7.1 接线锚（登记后删除态）：真实 registry 曾登记本根（durable
/// ownership/audit 在场），用户删除 home 侧工件后首次生成——admission/
/// spawn 复验按只读事实拒收（entry absent），不复活条目、不改写
/// ownership、不追加 audit；durable trust 事实快照冻结。
#[tokio::test]
async fn lc_first_generation_deleted_entry_does_not_resurrect_registration() {
    use crate::product::logical_codebase::ProviderTrustRegistry;

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let fixture = ProviderRunFixture::new_logical_with_gateway_extras(
        true,
        provider.clone(),
        AliasLcGatewayExtras {
            capability: Some(Arc::new(LcDialectCapabilitySource)),
            trust_home: Some(trust_home.path().to_path_buf()),
            availability: Some(lc_scoped_availability_gate(true, true)),
        },
    );
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    // 真实 registry 登记两 provider（legacy 别名 LC 作用域=本夹具 durable
    // 布局），随后删除 home 侧用户工件——「登记后删除」。
    let registry = crate::product::logical_codebase::HomeBackedProviderTrustRegistry::new(
        fixture.app_paths.clone(),
        vec![
            Arc::new(
                crate::product::logical_codebase::CodexTrustAdapter::for_home(trust_home.path()),
            ),
            Arc::new(
                crate::product::logical_codebase::KimiTrustAdapter::for_home(trust_home.path()),
            ),
        ],
    );
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    for trust_provider in [ProviderName::Codex, ProviderName::KimiCode] {
        registry
            .ensure_trusted(
                "project_0001",
                "aggregate_initialization_0001",
                &lc_id,
                &trust_provider,
                &canonical_aggregate_root,
            )
            .unwrap_or_else(|error| panic!("seed ensure {trust_provider:?}: {error:?}"));
    }
    std::fs::remove_file(trust_home.path().join(".codex").join("config.toml"))
        .expect("user deletes codex entry");
    std::fs::remove_file(
        crate::product::logical_codebase::KimiTrustAdapter::for_home(trust_home.path())
            .record_path(&canonical_aggregate_root),
    )
    .expect("user deletes kimi entry");
    let home_before = snapshot_tree(trust_home.path());
    let trust_facts_before = snapshot_tree(&trust_facts_root(&fixture));
    let audits_before = crate::product::logical_codebase::ProviderTrustStore::for_lc(
        fixture.app_paths.clone(),
        &lc_id,
    )
    .list_audit("project_0001")
    .expect("audit baseline")
    .len();
    assert_eq!(audits_before, 2, "布置期恰好两条 ensure/registered 审计");

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::Codex, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::Codex),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let error = next_error_frame(&mut outbound_rx).await;
    let message = error["message"].as_str().expect("error message");
    assert!(
        message.contains("provider_trust_missing"),
        "删除态以 entry absent 判据拒收，got: {message}"
    );
    assert!(
        message.contains(&canonical_aggregate_root.display().to_string()),
        "指导携带本根 trust 键（同源），got: {message}"
    );

    let durable = durable_session(&fixture);
    assert_eq!(
        durable.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare),
        "删除态保持 waiting/Prepare 面，got status={:?}",
        durable.status
    );
    assert_ne!(durable.status, WorkspaceSessionStatus::Failed);
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "删除态必须 reject before provider startup"
    );
    assert!(
        !trust_home
            .path()
            .join(".codex")
            .join("config.toml")
            .exists(),
        "生成路径不得复活被用户删除的条目"
    );
    assert_eq!(
        home_before,
        snapshot_tree(trust_home.path()),
        "删除态拒收不得写隔离 home"
    );
    assert_eq!(
        trust_facts_before,
        snapshot_tree(&trust_facts_root(&fixture)),
        "删除态拒收不得改写 durable trust 事实（ownership/audit 冻结）"
    );
    assert_eq!(
        crate::product::logical_codebase::ProviderTrustStore::for_lc(
            fixture.app_paths.clone(),
            &lc_id,
        )
        .list_audit("project_0001")
        .expect("audit after")
        .len(),
        audits_before,
        "只读拒收不得追加 trust 审计"
    );
}

/// 6.1 fix-loop（owner 验收回归）：多仓 WorkItemPlan（Legacy flow）会话
/// 首生成——outline Author 经 gateway spawn 的 trust 拒绝走**裸
/// ProviderUnavailable 臂**（无 admission 预检/无 drift 包装，经
/// `EngineEvent::Error` 上浮）——必须提升分类：错误携带
/// `provider_trust_missing`＋同源 manual_guidance（真实工件路径/键形/
/// 人工形状）＋生成场景 retry_action「人工登记…显式重新生成」，原诊断
/// 原文保留；provider 零启动、home/trust durable 快照不变。
#[tokio::test]
async fn lc_legacy_generation_spawn_trust_missing_classifies_with_manual_guidance() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let (fixture, mut engine_rx) =
        ProviderRunFixture::new_logical_legacy_with_gateway_extras_and_engine_rx(
            true,
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                trust_home: Some(trust_home.path().to_path_buf()),
                availability: Some(lc_scoped_availability_gate(true, true)),
            },
        );
    let home_before = snapshot_tree(trust_home.path());
    let trust_facts_before = snapshot_tree(&trust_facts_root(&fixture));

    let (context, _outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::KimiCode, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::KimiCode),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    // 裸臂上浮面：spawn 失败经 EngineEvent::Error → WS error 消息
    //（owner 现场同源观察点）。
    let message = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match engine_rx.recv().await {
                Some(EngineEvent::Error { message }) => return message,
                Some(_) => continue,
                None => panic!("engine event channel closed before spawn error surfaced"),
            }
        }
    })
    .await
    .expect("legacy outline spawn trust failure must surface via EngineEvent::Error");

    assert!(
        message.contains("provider_trust_missing"),
        "spawn 臂 trust 拒绝必须提升 provider_trust_missing 分类，got: {message}"
    );
    assert!(
        message.contains("provider trust not established for KimiCode"),
        "原诊断原文必须保留（不吞 gateway 拒绝事实），got: {message}"
    );
    // 同源 manual_guidance：Kimi 真实工件目录+稳定键+记录形状。
    assert!(
        message.contains(
            &trust_home
                .path()
                .join(".kimi-code")
                .join("workspace-trust")
                .display()
                .to_string()
        ) && message.contains("wd_aggregate-root_")
            && message.contains("trustedAt"),
        "spawn 臂指导必须同源（真实工件路径/键形/JSON 形状），got: {message}"
    );
    assert!(
        message.contains("人工登记") && message.contains("显式重新生成"),
        "spawn 臂必须携带生成场景 retry_action，got: {message}"
    );

    // provider 零启动；home/trust durable 快照不变（零 trust 写）。
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "spawn 臂 trust 拒绝必须 reject before provider startup"
    );
    assert_eq!(
        home_before,
        snapshot_tree(trust_home.path()),
        "spawn 臂拒收不得写隔离 home"
    );
    assert_eq!(
        trust_facts_before,
        snapshot_tree(&trust_facts_root(&fixture)),
        "spawn 臂拒收不得写 trust durable 事实"
    );
}

/// T6 fix-loop r2（owner 现场根因回归）：**Story 工作区**首生成走通用
/// Author 链（StartGeneration → Author run → `handle_user_message_from_run`
/// → `drive_author_provider_session_via_gateway` → gateway.start_streaming
/// 复验），不经 WIP 七臂的任何一行。Kimi 条目缺失时 spawn trust 拒绝
/// 必须以 `provider_trust_missing` 分类＋同源 manual_guidance＋生成场景
/// retry_action 上浮（owner 现场裸串
/// `provider_gateway_unavailable: provider trust not established for
/// KimiCode: entry absent` 保留为原拒绝原文）；provider 零启动、
/// home/trust durable 快照不变。Design/传统 WorkItem 首生成与
/// ChoiceFollowup 在同口合流，由该臂一次覆盖。
#[tokio::test]
async fn story_generation_spawn_trust_missing_classifies_with_manual_guidance() {
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let (fixture, mut engine_rx) =
        ProviderRunFixture::new_logical_story_with_gateway_extras_and_engine_rx(
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                trust_home: Some(trust_home.path().to_path_buf()),
                availability: Some(lc_scoped_availability_gate(true, true)),
            },
            ProviderName::KimiCode,
        );
    let home_before = snapshot_tree(trust_home.path());
    let trust_facts_before = snapshot_tree(&trust_facts_root(&fixture));

    let (context, _outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::KimiCode, provider);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::KimiCode),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    // 上浮面：spawn 失败经 EngineEvent::Error → WS error 消息（owner 现场
    // 跨会话收件箱 HardErrorInboxRow 的同源观察点）。
    let message = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match engine_rx.recv().await {
                Some(EngineEvent::Error { message }) => return message,
                Some(_) => continue,
                None => panic!("engine event channel closed before spawn error surfaced"),
            }
        }
    })
    .await
    .expect("story author spawn trust failure must surface via EngineEvent::Error");

    assert!(
        message.contains("provider_trust_missing"),
        "Story 通用 Author 臂 trust 拒绝必须提升 provider_trust_missing 分类，got: {message}"
    );
    assert!(
        message.contains("provider trust not established for KimiCode"),
        "原诊断原文（owner 现场同源裸串）必须保留，got: {message}"
    );
    // 同源 manual_guidance：Kimi 真实工件目录+稳定键+记录形状。
    assert!(
        message.contains(
            &trust_home
                .path()
                .join(".kimi-code")
                .join("workspace-trust")
                .display()
                .to_string()
        ) && message.contains("wd_aggregate-root_")
            && message.contains("trustedAt"),
        "Story 生成臂指导必须同源（真实工件路径/键形/JSON 形状），got: {message}"
    );
    assert!(
        message.contains("人工登记") && message.contains("显式重新生成"),
        "Story 生成臂必须携带生成场景 retry_action，got: {message}"
    );

    // provider 零启动；home/trust durable 快照不变（零 trust 写）。
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), input_rx.recv()).await,
            Ok(Some(_))
        ),
        "Story 生成臂 trust 拒绝必须 reject before provider startup"
    );
    assert_eq!(
        home_before,
        snapshot_tree(trust_home.path()),
        "Story 生成臂拒收不得写隔离 home"
    );
    assert_eq!(
        trust_facts_before,
        snapshot_tree(&trust_facts_root(&fixture)),
        "Story 生成臂拒收不得写 trust durable 事实"
    );
}

/// 指引卡简化补缺（owner 2026-10-12 多仓 story 生成现场）：trust 缺失
/// 的 **Story 通用 Author 启动臂**（StartGeneration → Author run →
/// `drive_author_provider_session_via_gateway`）上浮不只有平铺 error
/// message——紧随其后携带结构化 `trust_missing_guidance` 帧
/// （providers/canonical_root/reason_code/retry_action/command_markdown
/// 全部同源只读等待投影），与 SC admission waiting 路径同 pattern；前端
/// 「两行说明＋可复制命令窗」直渲，不再解析平铺串。
#[tokio::test]
async fn story_generation_trust_missing_emits_structured_guidance_frame() {
    let (input_tx, _input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let (fixture, engine_rx) =
        ProviderRunFixture::new_logical_story_with_gateway_extras_and_engine_rx(
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                trust_home: Some(trust_home.path().to_path_buf()),
                availability: Some(lc_scoped_availability_gate(true, true)),
            },
            ProviderName::KimiCode,
        );
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::KimiCode, provider);
    // 引擎事件经生产同构 forward 任务（map_engine_event）落 outbound——
    // author 臂错误/指导均走 EngineEvent 通道上浮。
    let forward = spawn_engine_event_forward_task(engine_rx, context.outbound_tx.clone(), None);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::KimiCode),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    // 平铺 error 帧保留（旧客户端兼容，稳定判别码不变）。
    let error = next_error_frame(&mut outbound_rx).await;
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("provider_trust_missing"),
        "flat error frame keeps the stable code, got: {error}"
    );
    // 紧随其后的结构化指导帧（entry 缺失携带可复制命令窗）。
    let guidance = next_frame_of_type(&mut outbound_rx, "trust_missing_guidance").await;
    assert_eq!(
        guidance["providers"],
        serde_json::json!(["kimi_code"]),
        "providers carry wire labels for the card headline: {guidance}"
    );
    assert_eq!(
        guidance["canonical_root"],
        canonical_aggregate_root.display().to_string(),
        "canonical root is the frozen authority root, got: {guidance}"
    );
    assert_eq!(guidance["reason_code"], "trust_entry_missing");
    assert!(
        guidance["retry_action"]
            .as_str()
            .expect("retry action")
            .contains("显式重新生成"),
        "retry action stays the generation-scoped wording: {guidance}"
    );
    let command = guidance["command_markdown"]
        .as_str()
        .expect("entry missing carries the copyable command");
    assert!(
        command.contains("mkdir -p")
            && command.contains(
                &trust_home
                    .path()
                    .join(".kimi-code")
                    .join("workspace-trust")
                    .display()
                    .to_string()
            ),
        "command markdown is the same-source kimi one-line shell: {command}"
    );
    forward.abort();
}

/// 指引卡简化补缺（reviewer 臂同型）：Reviewer 首发 gateway 启动臂
/// （`drive_review_session_via_gateway` → `start_review_session_via_gateway`）
/// trust 拒绝同样以结构化 `trust_missing_guidance` 帧紧随 error 帧上浮
/// （Codex entry 缺失：TOML 片段命令窗）；provider 零启动。
#[tokio::test]
async fn reviewer_launch_trust_missing_emits_structured_guidance_frame() {
    let (input_tx, _input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let (fixture, engine_rx) =
        ProviderRunFixture::new_logical_story_with_gateway_extras_and_engine_rx(
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                trust_home: Some(trust_home.path().to_path_buf()),
                availability: Some(lc_scoped_availability_gate(true, true)),
            },
            ProviderName::KimiCode,
        );
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    // reviewer 臂就位：夹具会话 reviewer 固定 Codex（story 夹具形态），
    // 空隔离 home → Codex entry 缺失。
    {
        let mut engine = fixture.engine.lock().await;
        engine.session.review_rounds = 2;
        engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
            markdown: "# Artifact\n\n可以确认".to_string(),
            diff: None,
        });
    }
    let (outbound_tx, mut outbound_rx) = mpsc::channel(64);
    let forward = spawn_engine_event_forward_task(engine_rx, outbound_tx, None);
    {
        let mut engine = fixture.engine.lock().await;
        // T20b(决策项⑦):真实 gateway 链的 reviewer 会话必须携窗——内层
        // 直调前开规划审查窗(生产入口同 helper 面),使 trust 分型错误
        // 形态保持原判别面。
        let reviewer = engine
            .session
            .reviewer_provider
            .clone()
            .expect("reviewer provider");
        engine
            .begin_plan_review_window(&reviewer, fixture.member_checkout_root())
            .await
            .expect("plan review window");
        engine.start_review_or_skip().await;
        let (_command_tx, command_rx) =
            mpsc::channel::<crate::cross_cutting::streaming_provider::ProviderCommand>(8);
        engine
            .drive_review_session_via_gateway_with_window(command_rx)
            .await;
    }

    // 平铺 error 帧保留（旧客户端兼容，稳定判别码不变）。
    let error = next_error_frame(&mut outbound_rx).await;
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("provider_trust_missing"),
        "reviewer arm flat error keeps the stable code, got: {error}"
    );
    // 紧随其后的结构化指导帧（Codex entry 缺失 → TOML 片段命令窗）。
    let guidance = next_frame_of_type(&mut outbound_rx, "trust_missing_guidance").await;
    assert_eq!(
        guidance["providers"],
        serde_json::json!(["codex"]),
        "reviewer guidance names the reviewer provider: {guidance}"
    );
    assert_eq!(
        guidance["canonical_root"],
        canonical_aggregate_root.display().to_string(),
        "canonical root is the frozen authority root, got: {guidance}"
    );
    assert_eq!(guidance["reason_code"], "trust_entry_missing");
    let command = guidance["command_markdown"]
        .as_str()
        .expect("codex entry missing carries the copyable command");
    assert!(
        command.contains("trust_level")
            && command.contains(
                &trust_home
                    .path()
                    .join(".codex")
                    .join("config.toml")
                    .display()
                    .to_string()
            ),
        "command markdown is the same-source codex TOML snippet: {command}"
    );
    forward.abort();
}

/// 负例（不误发）：Story Author 启动臂的非 trust ProviderUnavailable 不得
/// 发结构化指导帧——Codex trust 已人工预登记（waiting=None）而 availability
/// gate 不健康时，拒绝来自 availability 而非 trust 面，分类桥不命中
/// trust 分型、零结构化帧（与 6.1 SC 负例
/// `lc_first_generation_other_unavailable_not_misattributed_as_trust_missing`
/// 同判据，Story 通用 Author 臂面）。
#[tokio::test]
async fn story_generation_other_unavailable_emits_no_guidance_frame() {
    let (input_tx, _input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(LcRecordingAuthorProvider {
        output: single_candidate_markdown("story_0001", "design_0001"),
        inputs: input_tx,
    });
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let (fixture, engine_rx) =
        ProviderRunFixture::new_logical_story_with_gateway_extras_and_engine_rx(
            provider.clone(),
            AliasLcGatewayExtras {
                capability: Some(Arc::new(LcDialectCapabilitySource)),
                trust_home: Some(trust_home.path().to_path_buf()),
                // Codex 不健康：ProviderUnavailable 来自 availability gate
                // 而非 trust 面。
                availability: Some(lc_scoped_availability_gate(false, true)),
            },
            ProviderName::Codex,
        );
    // 人工预登记 Codex trust（用户级工件已登记 → trust 复验通过，
    // 拒绝只能来自 availability）。
    let canonical_aggregate_root = std::fs::canonicalize(
        fixture
            .member_checkout_root()
            .parent()
            .expect("member checkout lives under the aggregate root"),
    )
    .expect("canonical aggregate root");
    let codex_config = trust_home.path().join(".codex").join("config.toml");
    std::fs::create_dir_all(codex_config.parent().expect("codex config dir"))
        .expect("create codex config dir");
    std::fs::write(
        &codex_config,
        format!(
            "[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
            canonical_aggregate_root.display()
        ),
    )
    .expect("pre-register codex trust");

    let (context, mut outbound_rx) =
        single_candidate_context_for_author(&fixture, ProviderName::Codex, provider);
    let forward = spawn_engine_event_forward_task(engine_rx, context.outbound_tx.clone(), None);
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: author_provider_config(ProviderName::Codex),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;

    let error = next_error_frame(&mut outbound_rx).await;
    let message = error["message"].as_str().expect("error message");
    assert!(
        !message.contains("provider_trust_missing"),
        "availability rejection must not be misattributed as trust missing, got: {message}"
    );
    // 有界窗口内不得出现任何 trust_missing_guidance 帧。
    let guidance_leak = tokio::time::timeout(std::time::Duration::from_millis(400), async {
        loop {
            let control = outbound_rx
                .recv()
                .await
                .expect("outbound channel must stay open");
            let OutboundControl::Text(text) = control else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&text).expect("frame json");
            if value["type"] == "trust_missing_guidance" {
                return true;
            }
        }
    })
    .await;
    assert!(
        guidance_leak.is_err(),
        "non-trust launch failure must not emit a guidance frame, got: {guidance_leak:?}"
    );
    forward.abort();
}
