//! REQ-ROUTE-01 两态化（单仓入口删除）后的本地测试夹具支持：
//! Legacy 直连路由的夹具同样需要「LC 权威全链」（record/manifest/member/
//! checkout/selection/aggregate-index/policy），路由解析（`resolve_for_issue`
//! → `RepositoryRouting::classify`）才得到 Logical；已有权威的基座夹具则只
//! 缺「active aggregate index + 聚合 policy artifact」这一段规划链前置。
//!
//! 本模块只做登记与前置补齐，不改任何权威语义；LC 权威登记复用
//! `single_candidate_lc_admission::register_alias_logical_codebase_with_extras`
//! （record.json 落别名子树、其余落项目级 legacy 根，与 resolver 期望一致），
//! 不另起第二套。Legacy 直连路由**不把**返回的 gateway 注入 engine（保持
//! legacy 直连 provider 语义：`resolve_author_root_launch` 无 gateway 时
//! 返回 None）。

use std::path::Path;
use std::sync::Arc;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::logical_codebase::LogicalCodebaseStore;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::types::MemberStatus;
use crate::product::models::RepositoryRecord;
use crate::product::project_store::{CreateProjectInput, ProjectStore};

/// 占位 gateway author：`register_alias_logical_codebase_with_extras` 的必填
/// 参数。Legacy 直连路由不把 gateway 注入 engine ⇒ 本 adapter 永不会被调用；
/// 一旦被调用即以可观测错误终止（不静默伪造 provider 会话）。
pub(super) fn stub_gateway_author() -> Arc<dyn StreamingProviderAdapter> {
    Arc::new(NeverInvokedAdapter)
}

struct NeverInvokedAdapter;

#[async_trait::async_trait]
impl StreamingProviderAdapter for NeverInvokedAdapter {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            "",
            "legacy direct fixture never launches the gateway author",
            0,
        ))
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        tokio::sync::mpsc::Receiver<crate::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        Err(ProviderAdapterError::execution_failed(
            None,
            "",
            "legacy direct fixture never launches the gateway author",
            0,
        ))
    }
}

/// `register_alias_authority_and_upgrade_projection` 的打包参数
/// （clippy 参数上限；字段与原参数一一对应，纯机械搬迁）。
pub(super) struct AliasAuthorityRequest<'a> {
    pub(super) app_paths: &'a ProductAppPaths,
    pub(super) repository: &'a RepositoryRecord,
    pub(super) repository_root: &'a Path,
    pub(super) extra_member_roots: &'a [tempfile::TempDir],
    pub(super) extra_repositories: &'a [RepositoryRecord],
    pub(super) focus: &'a super::single_candidate_provider_run::LogicalAliasFocus,
    pub(super) gateway_author: Arc<dyn StreamingProviderAdapter>,
    pub(super) gateway_extras: super::single_candidate_lc_admission::AliasLcGatewayExtras,
}

/// Legacy 直连路由的权威登记：复用 lc_admission 的别名登记（record/manifest/
/// member/checkout/selection/aggregate-index/policy）并把兼容投影升级为已登记
/// 形态（strict 三层身份解析要求 logical_repository_id/primary_checkout_id/
/// identity_schema_version 对齐）。成员 0 = `repository_root`，其余成员取
/// `extra_member_roots`（顺序即 ordinal，与物理仓一一配对）。
///
/// 返回的 `AliasLcFixture.gateway` 仅供注入 gateway 的路由消费；Legacy 直连
/// 路由忽略该字段（不注入 engine）。
pub(super) fn register_alias_authority_and_upgrade_projection(
    request: AliasAuthorityRequest<'_>,
) -> super::single_candidate_lc_admission::AliasLcFixture {
    let AliasAuthorityRequest {
        app_paths,
        repository,
        repository_root,
        extra_member_roots,
        extra_repositories,
        focus,
        gateway_author,
        gateway_extras,
    } = request;
    let member_roots = std::iter::once(repository_root.to_path_buf())
        .chain(
            extra_member_roots
                .iter()
                .map(|member_root| member_root.path().to_path_buf()),
        )
        .collect::<Vec<_>>();
    let physical_repositories = std::iter::once(repository.clone())
        .chain(extra_repositories.iter().cloned())
        .collect::<Vec<_>>();
    let registered =
        super::single_candidate_lc_admission::register_alias_logical_codebase_with_extras(
            app_paths,
            &member_roots,
            &physical_repositories,
            focus,
            gateway_author,
            gateway_extras,
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
    registered
}

/// 既有权威（manifest/member/checkout/selection 已由基座夹具建成，如
/// workspace_engine 的 `make_work_item_plan_engine_with_accepted_contract_drafts`）
/// 补规划链前置：active(→Degraded) aggregate index + 聚合 policy artifact。
/// 不重写权威记录、不重写兼容投影。index 标记 Degraded（last-known-good），
/// freshness 评估短路（不探测 git、不触发 codegraph sync），与
/// `register_alias_logical_codebase_with_extras` 的夹具口径一致。
pub(super) fn ensure_planning_index_and_policy(app_paths: &ProductAppPaths, project_id: &str) {
    let authority = LogicalCodebaseStore::new(app_paths.clone());
    let manifest = authority
        .load_manifest(project_id)
        .expect("load fixture manifest")
        .expect("fixture manifest must exist");
    let members = authority
        .list_members(project_id)
        .expect("list fixture members");
    let checkouts = authority
        .list_checkouts(project_id)
        .expect("list fixture checkouts");
    let now = chrono::Utc::now().to_rfc3339();
    let snapshots = members
        .iter()
        .filter(|member| member.status == MemberStatus::Active)
        .map(|member| {
            let checkout = checkouts
                .iter()
                .find(|checkout| member.checkout_ids.contains(&checkout.checkout_id))
                .unwrap_or_else(|| panic!("member {} lacks a recorded checkout", member.alias));
            AggregateIndexMemberSnapshot::indexed(
                member.logical_repository_id,
                checkout.checkout_id,
                checkout
                    .revision
                    .clone()
                    .unwrap_or_else(|| "abc123".to_string()),
                false,
                now.clone(),
            )
        })
        .collect::<Vec<_>>();
    let index = AggregateIndexRecord::building(
        "aggregate_index_0001".to_string(),
        project_id.to_string(),
        manifest.membership_revision,
        snapshots,
        now.clone(),
    );
    let index_store = AggregateIndexStore::new(app_paths.clone());
    index_store
        .create(project_id, index.clone())
        .expect("create aggregate index");
    let mut activated = index;
    activated.status = AggregateIndexStatus::Active;
    index_store
        .replace_active(project_id, activated)
        .expect("activate aggregate index");
    index_store
        .mark_status(
            project_id,
            "aggregate_index_0001",
            AggregateIndexStatus::Degraded,
            Some("test fixture degraded last-known-good index".to_string()),
        )
        .expect("mark aggregate index degraded");
    AggregatePolicyArtifactStore::new(app_paths.clone())
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");
    // manager 生产 gateway 的 launch validate 首关消费 durable capability
    // 行（bootstrap 行全 Unknown ⇒ PlanningReadOnly launch cell unknown ⇒
    // 修订 author 0 启动）。夹具口径：ClaudeCode 行三格全 Confirmed
    //（launch/resume/write_boundary；execution_semantics=host-open-v1，
    // schema v3 语义）。
    let capability_store =
        crate::product::logical_codebase::ProviderCapabilityStore::new(app_paths.clone());
    capability_store
        .ensure_bootstrap(project_id)
        .expect("bootstrap provider capability rows");
    if let Some(mut record) = capability_store
        .get(
            project_id,
            crate::product::logical_codebase::ProviderRefType::ClaudeCode,
        )
        .expect("read claude code capability record")
    {
        use crate::product::logical_codebase::SessionPolicyAction;
        for action in [
            SessionPolicyAction::PlanningReadOnly,
            SessionPolicyAction::CodingTargetWrite,
            SessionPolicyAction::ReviewReadOnly,
        ] {
            let mut row = record.action_matrix.row(&action);
            row.launch =
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed;
            row.resume =
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed;
            row.write_boundary =
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed;
            record.action_matrix.replace_row(row);
        }
        record.execution_semantics = "host-open-v1".to_string();
        capability_store
            .upsert(project_id, &record)
            .expect("upsert confirmed claude code capability rows");
    }
}

/// SC author 修订链的 admission 预检消费成员 `.claude/rules/language.md`
///（缺失 ⇒ 运行停等在 provider 启动之前）；基座夹具只 git-init 成员
/// checkout，这里按 ProviderRunFixture 的默认正文补齐（已存在则不覆写）。
/// coding-runtime 夹具（plan_repair 等）不做 SC admission ⇒ 不调用本函数。
pub(super) fn ensure_member_language_rules(app_paths: &ProductAppPaths, project_id: &str) {
    let authority = LogicalCodebaseStore::new(app_paths.clone());
    let members = authority
        .list_members(project_id)
        .expect("list fixture members");
    let checkouts = authority
        .list_checkouts(project_id)
        .expect("list fixture checkouts");
    for member in members
        .iter()
        .filter(|member| member.status == MemberStatus::Active)
    {
        let Some(checkout) = checkouts
            .iter()
            .find(|checkout| member.checkout_ids.contains(&checkout.checkout_id))
        else {
            continue;
        };
        let rules = checkout.canonical_path.join(".claude/rules/language.md");
        if !rules.exists() {
            let _ = std::fs::create_dir_all(checkout.canonical_path.join(".claude/rules"));
            let _ = std::fs::write(
                &rules,
                "## 语言规则\n\n- **必须使用中文** - 所有响应、解释、注释和文档必须使用中文。\n",
            );
        }
    }
}

/// 单成员 Legacy 直连场景的权威登记便捷入口（tests.rs 内联夹具用）：
/// 登记 LC 权威全链 + 投影升级；返回的 gateway 不注入 engine（保持 legacy
/// 直连 provider 语义）。
pub(super) fn register_legacy_direct_authority(
    app_paths: &ProductAppPaths,
    repository: &RepositoryRecord,
    member_root: &Path,
) -> super::single_candidate_lc_admission::AliasLcFixture {
    register_alias_authority_and_upgrade_projection(AliasAuthorityRequest {
        app_paths,
        repository,
        repository_root: member_root,
        extra_member_roots: &[],
        extra_repositories: &[],
        focus: &super::single_candidate_provider_run::LogicalAliasFocus::FirstMember,
        gateway_author: stub_gateway_author(),
        gateway_extras: super::single_candidate_lc_admission::AliasLcGatewayExtras::default(),
    })
}

/// 「有 manifest、无 selection」的不成对现场（fail-closed 用例前置）：
/// project + 别名归属 issue + 空 manifest，无 selection。
pub(super) fn seed_unpaired_manifest_with_attributed_issue(app_paths: &ProductAppPaths) {
    seed_attributed_issue(app_paths);
    LogicalCodebaseStore::new(app_paths.clone())
        .save_manifest(
            "project_0001",
            &crate::product::logical_codebase::LogicalCodebaseManifest::new(
                "project_0001",
                app_paths.root().join("aggregate-root"),
                Vec::new(),
            ),
        )
        .expect("save unpaired fixture manifest");
}

/// 建（缺失时）project_0001 + 归属 legacy 别名代码库的 issue_0001。
/// ⚠️ 必须先于任何 selection 写入调用（`IssueStore::create` 的顺序 id 按
/// `issues/` 目录数分配，selection 目录先建会让 issue id 漂移）。
pub(super) fn seed_attributed_issue(app_paths: &ProductAppPaths) {
    let project_store = ProjectStore::new(app_paths.clone());
    if project_store.get("project_0001").is_err() {
        project_store
            .create(CreateProjectInput {
                name: "lc fixture support project".to_string(),
                description: None,
            })
            .expect("create project");
    }
    // 幂等：issue 已在场（如 anchor_t13 家族自建 issue 后调用
    // write_logical_codebase）则不重复创建，避免顺序 id 漂移。
    if IssueStore::new(app_paths.clone())
        .get("project_0001", "issue_0001")
        .is_ok()
    {
        return;
    }
    let alias_lc_id =
        crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: alias_lc_id,
            title: "LC fixture support issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("create attributed issue");
    assert_eq!(
        issue.id, "issue_0001",
        "attributed issue must keep the fixture-hardcoded id (selection ordering contract)"
    );
}
