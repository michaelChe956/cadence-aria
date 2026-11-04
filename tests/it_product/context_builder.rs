//! `it_product` 集成测试共享夹具：**LC 权威 + 冻结 `target_snapshot`**。
//!
//! REQ-ROUTE-01 两态化（去单仓入口）后，admission 的唯一凭证是「LC 权威 +
//! 冻结快照」：无归属/不成对一律 `FailClosed(TargetMissing)`
//! （`RepositoryRouting::unbound_issue()`），不再有 `Legacy` 变体回退物理仓。
//! 因此任何以 admission 行为（签票、CAS 转换、marker 清理）或 provider run
//! （租约三态）为不变量的集成用例，都必须先经本模块建出真实归属，否则测到的是
//! fail-closed（`target_snapshot_identity_drifted` /
//! `coding_run_lease_unknown: … product_store_not_found: issue issue_0001`）
//! 而不是原不变量。
//!
//! 与 `src/product/logical_codebase/logical_test_fixture.rs`（`pub(crate)`，仅 crate
//! 内可见）等价，但**只用公开 API 重写**——外部集成测试（独立 crate）调不到 crate
//! 内夹具，故此处按公开 API 复刻同一套权威现场：
//!
//! 1. `ProjectStore::create` → `project_0001`；
//! 2. `LogicalCodebaseStore::create` → 真多仓 LC（非 legacy 别名），落
//!    `logical-codebases/{lc_id}/record.json`；
//! 3. `IssueStore::create` → `issue_0001`（**必须先于 selection 落盘**：项目级
//!    selection store 会生成 `issues/{issue_id}/` 目录，而 issue id 由目录数分配，
//!    顺序颠倒会让硬编码 id 漂移）；
//! 4. `for_lc` 子树落 manifest / member / checkout / selection / policy 权威；
//! 5. 聚合索引（active→degraded last-known-good）补齐规划链前置；
//! 6. `repos.json` 兼容投影（物理仓 id 与权威 member 对齐）。
//!
//! 成员 checkout 必须是**真实 git 仓**且位于 manifest `provider_context_root`
//! 之下（否则 `work_item_scope_domain_unresolved` fail-closed）：本模块把聚合根
//! 放在 `{root}/aggregate-root`，成员放在其**直接子目录**
//! `{root}/aggregate-root/repository_0001`。
//!
//! Phase 2 铺开其余 `part_XX.rs` 时会引用本模块的全部导出项；本阶段只迁
//! `part_01.rs`，未被引用的导出项不视为死代码。
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cadence_aria::cross_cutting::provider_adapter::{
    DEFAULT_PROVIDER_TIMEOUT_SECS, ProviderAdapter, ProviderAdapterError,
};
use cadence_aria::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use cadence_aria::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderPermissionMode, ProviderSession, StreamChunk,
    StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
use cadence_aria::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot;
use cadence_aria::product::coding_models::{AttemptTargetSnapshot, CodingExecutionAttempt};
use cadence_aria::product::json_store::write_json;
use cadence_aria::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use cadence_aria::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential;
use cadence_aria::product::logical_codebase::provider_capability_store::ProviderActionCapability;
use cadence_aria::product::logical_codebase::provider_gateway::PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE;
use cadence_aria::product::logical_codebase::{
    AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
    GatewayRunAudit, IssueCodebaseSelection, IssueCodebaseSelectionStore,
    LogicalCodebaseCreateInput, LogicalCodebaseManifest, LogicalCodebaseProviderGateway,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, PolicyTarget, PolicyTargetResolver,
    ProviderCapability, ProviderCapabilitySource, ProviderDialect, ProviderGatewayError,
    ProviderRef, ProviderRefType, ProviderWireDialect, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType, SessionLaunchRequest,
    SessionPolicyAction,
};
use cadence_aria::product::models::{
    IssuePhase, IssueRecord, IssueStatus, ProviderName, RepositoryRecord,
};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, ProviderType, TimeoutStatus};
use tokio_util::sync::CancellationToken;

/// 夹具内唯一成员/物理仓 id（与 `repos.json` 投影、member/checkout 记录一致）。
pub(crate) const FIXTURE_PHYSICAL_REPOSITORY_ID: &str = "repository_0001";

/// 夹具内聚合索引固定 id。
pub(crate) const FIXTURE_AGGREGATE_INDEX_ID: &str = "aggregate_index_0001";

/// 一条 issue 的完整可准入现场（LC 权威 + 冻结快照的来源）。
pub(crate) struct LogicalAuthority {
    /// 项目 id（恒为 `project_0001`）。
    pub(crate) project_id: String,
    /// issue id（恒为 `issue_0001`）。
    pub(crate) issue_id: String,
    /// issue 持久归属的 LC id（真多仓子树，非 legacy 别名）。
    pub(crate) logical_codebase_id: String,
    /// 成员 logical id（`AttemptTargetSnapshot.logical_repository_id`）。
    pub(crate) member_id: LogicalRepositoryId,
    /// 主 checkout id（`AttemptTargetSnapshot.checkout_id`）。
    pub(crate) checkout_id: RepositoryCheckoutId,
    /// 成员 checkout 的真实路径（真实 git 仓，位于聚合根之下）。
    pub(crate) member_path: PathBuf,
}

impl LogicalAuthority {
    /// 按当前 LC 权威事实构造一份**与权威一致**的冻结快照。
    ///
    /// 直接复用生产快照构造器 `build_attempt_target_snapshot`（公开 API）：
    /// policy digest / membership revision / checkout 身份 / HEAD revision
    /// 全部按权威现场解析，故 `validate_snapshot_fields` 必然通过
    /// （不手搓摘要、不伪造 revision）。
    pub(crate) fn snapshot(&self, paths: &ProductAppPaths) -> AttemptTargetSnapshot {
        build_attempt_target_snapshot(
            paths,
            &self.project_id,
            self.member_id,
            Some(&self.logical_codebase_id),
        )
        .expect("build fixture attempt target snapshot")
    }
}

/// 建「project + 真多仓 LC 权威 + 归属该 LC 的 issue」的可准入现场。
///
/// * `root`：夹具根目录（通常是 `tempdir` 路径）；聚合根取 `{root}/aggregate-root`，
///   成员 checkout 取其直接子目录 `{root}/aggregate-root/repository_0001`。
/// * 幂等前提：同一 `paths` 下只应调用一次（project/issue id 按目录顺序分配）。
///
/// 返回的 [`LogicalAuthority`] 可直接喂给 [`LogicalAuthority::snapshot`]。
pub(crate) fn seed_logical_authority(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    root: &Path,
) -> LogicalAuthority {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "it-product fixture".to_string(),
            description: None,
        })
        .expect("fixture project");

    // 真多仓 LC（非 legacy 别名）：`create` 落 `logical-codebases/{lc_id}/record.json`
    // 与子树目录，authority resolver 直读该 record（不经 `migrate_legacy`）。
    let aggregate_root = root.join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("fixture aggregate root");
    let lc_record = LogicalCodebaseStore::new(paths.clone())
        .create(
            project_id,
            LogicalCodebaseCreateInput {
                name: "it-product-lc".to_string(),
                aggregate_root: aggregate_root.clone(),
            },
        )
        .expect("fixture logical codebase");
    let logical_codebase_id = lc_record.id;

    // issue 记录：**按固定 id 直接落盘**，不用 `IssueStore::create`。
    // 原因：`IssueStore::create` 的 id 按 `issues/` 目录数分配（`issue_0001` 是
    // 第一个目录），而 coding attempt 落盘本身就会创建 `issues/{issue_id}/`
    // 目录 ⇒ attempt 先建时再 create issue 会分配到 `issue_0002`，与夹具里
    // 硬编码的 `issue_0001` 全部错位。固定 id 落盘让「先建 attempt 还是先建
    // 权威」都无所谓。
    let now = chrono::Utc::now().to_rfc3339();
    write_json(
        &paths.issue_root(project_id, issue_id).join("issue.json"),
        &IssueRecord {
            id: issue_id.to_string(),
            project_id: project_id.to_string(),
            repo_id: Some(FIXTURE_PHYSICAL_REPOSITORY_ID.to_string()),
            base_branch: None,
            logical_codebase_id: logical_codebase_id.clone(),
            title: "it-product fixture issue".to_string(),
            description: None,
            change_id: "change_issue_0001".to_string(),
            phase: IssuePhase::Clarification,
            status: IssueStatus::Draft,
            active_binding_id: None,
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .expect("fixture issue record");

    let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    // 成员 checkout 位于聚合根**之下**（`scope_member_prefix` 的 strip 前提），
    // 且必须是真实 git 仓（跨目标基线采样按真仓读 HEAD）。
    let member_path = aggregate_root.join(FIXTURE_PHYSICAL_REPOSITORY_ID);
    let revision = init_member_git_repo(&member_path);
    let source_identity =
        RepositorySourceIdentity::from_git_parts(&member_path, member_path.join(".git"), None);

    let authority = LogicalCodebaseStore::for_lc(paths.clone(), &logical_codebase_id);
    let manifest = LogicalCodebaseManifest::new(project_id, aggregate_root, vec![member_id]);
    authority
        .save_manifest(project_id, &manifest)
        .expect("fixture manifest");
    authority
        .save_member(
            project_id,
            &CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
                alias: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
                role: "repository".to_string(),
                ordinal: 1,
                source_identity: source_identity.clone(),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: MemberStatus::Active,
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            },
        )
        .expect("fixture member");
    authority
        .save_checkout(
            project_id,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
                kind: CheckoutKind::Main,
                canonical_path: member_path.clone(),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: source_identity.git_dir_identity(),
                revision: Some(revision.clone()),
                availability: CheckoutAvailability::Available,
                observed_at: "2026-08-11T00:00:00Z".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            },
        )
        .expect("fixture checkout");
    IssueCodebaseSelectionStore::for_lc(paths.clone(), &logical_codebase_id)
        .save(
            &IssueCodebaseSelection::all_members(project_id, issue_id, None)
                .for_logical_codebase(&logical_codebase_id),
        )
        .expect("fixture issue selection");
    write_json(
        &paths.project_root(project_id).join("repos.json"),
        &[RepositoryRecord {
            id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
            project_id: project_id.to_string(),
            name: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
            path: member_path.clone(),
            repo_hash: "sha256:repository".to_string(),
            runtime_root: PathBuf::from("runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            logical_repository_id: Some(member_id),
            primary_checkout_id: Some(checkout_id),
            identity_schema_version: 1,
        }],
    )
    .expect("fixture repos projection");
    AggregatePolicyArtifactStore::for_lc(paths.clone(), &logical_codebase_id)
        .ensure_bootstrap(&manifest)
        .expect("fixture aggregate policy");
    ensure_aggregate_index(
        paths,
        project_id,
        &logical_codebase_id,
        &manifest,
        member_id,
        checkout_id,
        &revision,
    );

    LogicalAuthority {
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        logical_codebase_id,
        member_id,
        checkout_id,
        member_path,
    }
}

/// 补规划链前置：**active(→Degraded) 聚合索引**。
///
/// 规划链（workspace context / gateway / 计划聚合视野）以
/// `AggregateIndexStore::active_required` 为 fail-closed 前置：缺索引 ⇒
/// `aggregate_index_unavailable`。index 标 Degraded（last-known-good）以短路
/// freshness 的真实 codegraph 探测。
fn ensure_aggregate_index(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    manifest: &LogicalCodebaseManifest,
    member_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
    revision: &str,
) {
    let index_store = AggregateIndexStore::for_lc(paths.clone(), logical_codebase_id);
    if index_store
        .get(project_id, FIXTURE_AGGREGATE_INDEX_ID)
        .expect("read fixture aggregate index")
        .is_some()
    {
        return;
    }
    let now = chrono::Utc::now().to_rfc3339();
    let index = AggregateIndexRecord::building(
        FIXTURE_AGGREGATE_INDEX_ID.to_string(),
        project_id.to_string(),
        manifest.membership_revision,
        vec![AggregateIndexMemberSnapshot::indexed(
            member_id,
            checkout_id,
            revision.to_string(),
            false,
            now.clone(),
        )],
        now.clone(),
    );
    index_store
        .create(project_id, index.clone())
        .expect("create fixture aggregate index");
    let mut activated = index;
    activated.status = AggregateIndexStatus::Active;
    index_store
        .replace_active(project_id, activated)
        .expect("activate fixture aggregate index");
    index_store
        .mark_status(
            project_id,
            FIXTURE_AGGREGATE_INDEX_ID,
            AggregateIndexStatus::Degraded,
            Some("test fixture degraded last-known-good index".to_string()),
        )
        .expect("mark fixture aggregate index degraded");
}

/// 把冻结 `target_snapshot` 补挂到**已落盘**的 attempt 记录上。
///
/// 用于「attempt 由共享夹具先创建、LC 权威后补」的用例：attempt 的
/// `target_snapshot` 在创建时冻结，store 没有公开的补挂 API，故按
/// `seed_coding_attempt_running` 同款的 fixture 直写方式落盘（只用于集成
/// 测试初始状态，不代表生产写路径）。
pub(crate) fn attach_target_snapshot(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    snapshot: AttemptTargetSnapshot,
) -> CodingExecutionAttempt {
    let mut attempt = attempt.clone();
    attempt.target_snapshot = Some(snapshot);
    write_json(
        &store
            .paths()
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join(format!("{}.json", attempt.id)),
        &attempt,
    )
    .expect("attach attempt target snapshot");
    attempt
}

// ── provider gateway 夹具 ──────────────────────────────────────────────────
//
// 逻辑 target（`attempt.target_snapshot.is_some()`）的 provider run 唯一经
// `LogicalCodebaseProviderGateway::start_streaming` 启动（否则
// `logical_provider_gateway_required` fail-closed），而 gateway 从自己的
// registry 取 adapter ⇒ 测试 adapter 必须同时交给引擎方法（断言捕获面）与
// registry（真实 spawn 面），两处指向同一实例。
//
// gateway 本身**完全可以用公开 API 组装**（本节即证据）；唯一缺口在 adapter
// 侧：`StreamingProviderAdapter::start_validated` 的入参
// `ValidatedStreamingProviderInput::into_parts()` 是 `pub(crate)`（见报告
// 「不是 pub 的 API 清单」），外部集成测试无法把 validated 载体拆回
// `StreamingProviderInput` ⇒ 只能改用公开的 `launch_view()`（role /
// working_dir / prompt 三件）。

/// 把「只实现 legacy `run_streaming`」的测试 adapter 桥接到 LC gateway 的
/// validated 启动路径（`start_validated`）。
///
/// 手法与 `tests/it_web/web_coding_ws_handler/fixtures.rs` 的
/// `GatewayDialectStreamingStub` 同构（已验证手法）：validated 载体拆不开
/// （`ValidatedStreamingProviderInput::into_parts()` 是 crate 内 API），故从公开
/// 只读投影 `launch_view()`（role / spawn cwd / prompt）重建 `AdapterInput`
/// 驱动内层 `run_streaming`，并把 `StreamChunk` 桥接回 `ProviderEvent` 会话。
///
/// ⚠️ 只能桥接**行为与完整 `StreamingProviderInput` 无关**的替身：断言
/// `permission_mode` / `timeout_secs` / `resume_provider_session_id` 的替身
/// （如 `SessionInputCapturingProvider`）在 validated 路径下拿不到这些字段
/// （公开投影只有 role / working_dir / prompt），不能这样桥接。
pub(crate) struct GatewayBridgedAdapter {
    /// 内层替身（真实 spawn 面）。
    pub(crate) inner: Arc<dyn StreamingProviderAdapter>,
    /// 重建 `AdapterInput` 时的 provider 类型（与 registry 注册名对应）。
    pub(crate) provider_type: ProviderType,
    /// 重建 `AdapterInput` 时的 output_schema；`None` ⇒ 按 prompt 标记复原
    /// （it_web 同款口径，见 [`gateway_bridged_output_schema`]）。
    pub(crate) output_schema: Option<String>,
}

impl GatewayBridgedAdapter {
    /// 便捷构造：provider 类型显式给，output_schema 按 prompt 复原。
    pub(crate) fn new(
        inner: Arc<dyn StreamingProviderAdapter>,
        provider_type: ProviderType,
    ) -> Self {
        Self {
            inner,
            provider_type,
            output_schema: None,
        }
    }
}

/// 按 prompt 复原 output_schema（it_web 同款口径：
/// `tests/it_web/web_coding_ws_handler/fixtures.rs` 的 `gateway_bridged_output_schema`）。
///
/// LC 侧两条 reviewer 生产模板：legacy 单 Work Item 中文模板（prompts.rs
/// 「你是 CodeReviewer」）与 group(schema v2) 投影渲染模板
/// （work_item_projection/render.rs「# {provider} Reviewer Work Item Projection」）
/// ——两条都复原到 `coding_workspace_code_review_json`；group final review 模板
/// 复原到 `coding_workspace_internal_pr_review_json`；其余（coder 等）空串。
///
/// ⚠️ **判别顺序：group final review 必须排在 code review 之前**
/// （it_product 的 group final review prompt **同时**含
/// 「你是 WorkItemGroup GroupFinalReview reviewer」与
/// 「# {provider} Reviewer Work Item Projection」两段 ⇒ 若 code review 分支在前，
/// 会先命中并复原成 `coding_workspace_code_review_json`，
/// 与该模板的期望 `coding_workspace_internal_pr_review_json` 不符）。
/// it_web 侧的两条模板不重叠，故其原始顺序在那边无害；此处按 it_product 的
/// 实际 prompt 取更严的顺序（先判更具体的 group final review）。
pub(crate) fn gateway_bridged_output_schema(prompt: &str) -> &'static str {
    if prompt.contains("你是 WorkItemGroup GroupFinalReview reviewer") {
        "coding_workspace_internal_pr_review_json"
    } else if prompt.contains("你是 CodeReviewer")
        || prompt.contains(" Reviewer Work Item Projection")
    {
        "coding_workspace_code_review_json"
    } else {
        ""
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GatewayBridgedAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inner.start(input, cancel).await
    }

    /// 已自带 `start_validated` 的替身优先走其自身实现；仅当 inner 落到 trait
    /// 默认拒绝时才用 `run_streaming` 桥接。
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
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.inner.run_streaming(input, cancel).await
    }
}

impl GatewayBridgedAdapter {
    async fn bridged_validated_session(
        &self,
        validated: ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let view = validated.launch_view();
        let output_schema = self
            .output_schema
            .clone()
            .unwrap_or_else(|| gateway_bridged_output_schema(view.prompt).to_string());
        let input = AdapterInput {
            provider_type: self.provider_type.clone(),
            role: view.role.clone(),
            worktree_path: Some(view.working_dir.to_string_lossy().to_string()),
            working_directory: None,
            provider_stream_log_dir: None,
            prompt: view.prompt.to_string(),
            context_files: Vec::new(),
            output_schema,
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
            env_vars: Default::default(),
        };
        let mut chunk_rx = self.inner.run_streaming(&input, cancel).await?;
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(8);
        let (command_tx, _command_rx) = tokio::sync::mpsc::channel(8);
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

/// 把「只实现裸 `start`」的替身桥接到 LC gateway 的 validated 启动路径。
///
/// 与 [`GatewayBridgedAdapter`]（`run_streaming` 桥）**互补**：
/// - `run_streaming` 桥把 `StreamChunk` 桥成 `ProviderEvent`，但 trait 默认
///   `run_streaming` 会把 `ProviderEvent` **降级**成 `StreamChunk`
///   ⇒ `Execution` / `ToolCall` / `PermissionRequest` / `ChoiceRequest` 全被丢弃，
///   事件类断言测不到真实行为；
/// - 本桥改为驱动内层**裸 `start`**，把返回的 `ProviderSession` **原样透传**
///   ⇒ 事件流不丢。
///
/// validated 载体拆不开（`ValidatedStreamingProviderInput::into_parts` 是 crate 内
/// API），故从公开只读投影 `launch_view()` 重建 `StreamingProviderInput`
/// （role / spawn cwd / prompt 三个真值，其余取中性默认）。
///
/// ⚠️ 重建的 input 里 `permission_mode` / `timeout_secs` /
/// `resume_provider_session_id` 是**中性默认**，不是引擎真实下发的值 ⇒
/// 断言这三个字段的用例**不能用本桥**（会**假绿**：断言的是自己重建的值）。
/// A1（`ValidatedStreamingProviderInput::into_parts` 提为 `pub`）已解 ⇒
/// 需要真值的用例请改用 [`GatewayExactInputAdapter`]（逐字透传真实 input）。
pub(crate) struct GatewayValidatedStartAdapter {
    /// 内层替身（真实 spawn 面）。
    pub(crate) inner: Arc<dyn StreamingProviderAdapter>,
    /// 重建 input 时的 provider 类型（gateway 的 registry 槽位名与之对应）。
    pub(crate) provider_type: ProviderType,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GatewayValidatedStartAdapter {
    fn supports_tool_calls(&self) -> bool {
        self.inner.supports_tool_calls()
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inner.start(input, cancel).await
    }

    /// 已自带 `start_validated` 的替身优先走其自身实现；仅当 inner 落到 trait
    /// 默认拒绝时才用裸 `start` 桥接。
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
                let view = validated.launch_view();
                let input = StreamingProviderInput {
                    provider_type: self.provider_type.clone(),
                    role: view.role.clone(),
                    prompt: view.prompt.to_string(),
                    working_dir: view.working_dir.to_path_buf(),
                    working_directory: None,
                    workspace_session_id: None,
                    resume_provider_session_id: None,
                    permission_mode: ProviderPermissionMode::Auto,
                    tool_policy: None,
                    audit_sink: None,
                    structured_output_contract: None,
                    env_vars: BTreeMap::new(),
                    timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
                    baseline_tree: None,
                };
                self.inner.start(input, cancel).await
            }
            Err(error) => Err(error),
        }
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.inner.run_streaming(input, cancel).await
    }
}

/// validated 启动桥（A1 已解）：把 validated 载体**原样**拆回裸
/// `StreamingProviderInput`（`into_parts` 自 A1 起是 `pub`），驱动内层 `start`。
///
/// 与 [`GatewayValidatedStartAdapter`] 的关键差别：后者用 `launch_view()` **重建**
/// input（`permission_mode` / `timeout_secs` / `resume_provider_session_id` 是中性
/// 默认 ⇒ 断言这些字段会**假绿**），本桥**逐字透传引擎真实下发的 input**
/// ⇒ 断言 `permission_mode` / `provider_type` / `timeout_secs` /
/// `resume_provider_session_id` 的用例**必须**用本桥。
///
/// 与 [`GatewayBridgedAdapter`] 的差别：后者走 `run_streaming`（trait 默认实现会把
/// `ProviderEvent` 降级丢事件），本桥走裸 `start` ⇒ 事件流不丢。
pub(crate) struct GatewayExactInputAdapter {
    /// 内层替身（真实 spawn 面）。
    pub(crate) inner: Arc<dyn StreamingProviderAdapter>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GatewayExactInputAdapter {
    fn supports_tool_calls(&self) -> bool {
        self.inner.supports_tool_calls()
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inner.start(input, cancel).await
    }

    /// 已自带 `start_validated` 的替身优先走其自身实现；仅当 inner 落到 trait
    /// 默认拒绝时才拆载体驱动裸 `start`。
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
                let (input, _policy) = validated.into_parts();
                self.inner.start(input, cancel).await
            }
            Err(error) => Err(error),
        }
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.inner.run_streaming(input, cancel).await
    }
}

/// 组装逻辑 target 的 provider gateway（公开 API）。
///
/// * `registry`：真实 spawn 面取 adapter 的来源（测试 adapter 必须注册在此）。
/// * authority root 取夹具根（`.aria` 的父目录）：成员 checkout 与用例的
///   coding worktree 都建在其下，gateway 的 cwd 授权门才放行。
pub(crate) fn build_gateway(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    registry: Arc<ProviderRegistry>,
) -> LogicalCodebaseProviderGateway {
    let policies = AggregatePolicyArtifactStore::for_lc(paths.clone(), logical_codebase_id);
    let manifest = LogicalCodebaseStore::for_lc(paths.clone(), logical_codebase_id)
        .load_manifest(project_id)
        .expect("load fixture manifest")
        .expect("fixture manifest must exist");
    policies
        .ensure_bootstrap(&manifest)
        .expect("bootstrap fixture aggregate policy");
    LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(StaticCapabilitySource),
        Arc::new(PassThroughTargetResolver),
        registry,
        Arc::new(StubSyncAdapter),
        always_available_gate(),
        Arc::new(GatewayRunAudit::new()),
        paths
            .root()
            .parent()
            .expect("fixture workspace root")
            .to_path_buf(),
    )
}

/// pass-through target resolver：直接返回请求中冻结的 target。
struct PassThroughTargetResolver;

impl PolicyTargetResolver for PassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 测试 capability source：按 provider ref 返回对应 dialect 的 capability。
struct StaticCapabilitySource;

impl StaticCapabilitySource {
    fn capability(provider: &ProviderRef, action: SessionPolicyAction) -> ProviderCapability {
        let (adapter_dialect, wire_dialect) = match provider.provider_type {
            ProviderRefType::ClaudeCode => (
                ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            ProviderRefType::Codex => (
                ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
            ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
        };
        ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.0.0".to_string(),
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

impl ProviderCapabilitySource for StaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        if provider.provider_type != ProviderRefType::ClaudeCode {
            return Err(ProviderGatewayError::UnsupportedCapability(
                PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE.to_string(),
            ));
        }
        Ok(Self::capability(
            provider,
            SessionPolicyAction::PlanningReadOnly,
        ))
    }
}

/// 同步 adapter stub：`validate` 不触达 sync adapter，仅满足 gateway 构造签名。
struct StubSyncAdapter;

impl ProviderAdapter for StubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

/// 恒「健康」的 provider 可用性门（夹具不触达真实 provider 探活）。
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
        providers: [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ]
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

/// 自检：LC 权威 + 冻结快照 + provider gateway 三件套**可由公开 API 组装**。
///
/// 这是 Phase 1 的路径可行性证据：共享夹具不依赖任何 crate 内（`pub(crate)`）
/// API，集成测试（外部 crate）能独立建出「可准入 + 可启动 provider」的现场。
/// （provider 真正启动还差 adapter 侧支持 `start_validated`——见报告「不是 pub
/// 的 API 清单」中的 `ValidatedStreamingProviderInput::into_parts`。）
#[test]
fn logical_authority_snapshot_and_gateway_are_assemblable_from_public_api() {
    let root = tempfile::tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let authority = seed_logical_authority(&paths, "project_0001", "issue_0001", root.path());

    let snapshot = authority.snapshot(&paths);
    assert_eq!(snapshot.logical_repository_id, authority.member_id);
    assert_eq!(snapshot.checkout_id, authority.checkout_id);
    assert_eq!(snapshot.membership_revision, 1);
    assert!(!snapshot.policy_digest.is_empty());
    assert!(authority.member_path.join(".git").exists());

    let gateway = build_gateway(
        &paths,
        "project_0001",
        &authority.logical_codebase_id,
        Arc::new(ProviderRegistry::new()),
    );
    // gateway 可构造即证明公开 API 足够（启动面另需 adapter 支持 validated 启动）。
    let _ = gateway;
}

/// 在夹具成员 checkout 上初始化一个带初始提交的真实 git 仓（幂等），返回 HEAD sha。
fn init_member_git_repo(path: &Path) -> String {
    std::fs::create_dir_all(path).expect("fixture member dir");
    if !path.join(".git").exists() {
        for args in [
            ["init", "--quiet"].as_slice(),
            ["config", "user.email", "fixture@example.test"].as_slice(),
            ["config", "user.name", "Fixture"].as_slice(),
        ] {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(path)
                .status()
                .expect("fixture git command");
            assert!(status.success(), "git {args:?} failed");
        }
        std::fs::write(path.join("README.md"), "fixture\n").expect("fixture readme");
        for args in [
            ["add", "README.md"].as_slice(),
            ["commit", "--quiet", "-m", "fixture"].as_slice(),
        ] {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(path)
                .status()
                .expect("fixture git command");
            assert!(status.success(), "git {args:?} failed");
        }
    }
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .expect("fixture git rev-parse");
    assert!(output.status.success(), "git rev-parse HEAD failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
