// 本片段经 `include!` 进入 `knowledge_base` handler 模块（Task 7）。
//
// 请求期装配说明：`WebAppState` 没有 per-KB 依赖槽（Task 7 不改 state 面），
// coordinator 按聚合初始化同款生产口径在请求期组装；run registry 与 trust
// 前置门**复用聚合初始化依赖图的同一实例**——HTTP cancel 才能命中在跑
// worker 的取消令牌，trust 门与聚合面同源（fake runtime 下 home 指向
// workspace root，不触碰真实用户 home）。

/// 请求期解析的知识库执行依赖。
#[derive(Clone)]
pub(crate) struct KnowledgeBaseDependencies {
    pub(crate) coordinator: KnowledgeBaseCoordinator,
    pub(crate) runs: InitializationRunRegistry,
}

/// 依赖图未装配 trust 门（测试注入聚合夹具后 `trust == None`）时的 no-op 实现：
/// 与聚合面「未装配 gate 保持旧行为」同一语义。
struct NoopTrustPrecondition;

impl ProviderTrustPrecondition for NoopTrustPrecondition {
    fn ensure_ready(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _lc_id: &str,
        _canonical_root: &StdPath,
        _providers: &[ProviderName],
        _scenario: crate::product::logical_codebase::ProviderTrustScenario,
    ) -> ProviderTrustPreparationResult {
        ProviderTrustPreparationResult::Ready {
            registrations: Vec::new(),
        }
    }
}

/// owner HOME → Cadence-skills 插件源根（与 Task 4 预检②/Task 5 模板同一解析口径）。
fn knowledge_base_skills_source_root() -> ApiResult<PathBuf> {
    let home =
        crate::product::logical_codebase::provider_trust_adapters::resolve_owner_home_from_env()
            .ok_or_else(|| {
                ApiError::runtime(
                    CODE_SKILLS_UNAVAILABLE,
                    "owner home is not resolvable; knowledge-base skills cannot be located",
                    json!({ "missing": [] }),
                )
            })?;
    Ok(CadenceSkillsPaths::from_home(&home)
        .source_root()
        .to_path_buf())
}

/// 变更包 lint（P7 提交解析）的成员仓路径：LC 成员 → checkout 的 canonical path。
/// 读不到 checkout 的成员直接跳过（lint 对空成员列表按「全部失败」判定，与
/// Task 3 门禁语义一致，不在此放大失败面）。
fn knowledge_base_member_repos(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
) -> ApiResult<Vec<MemberRepoPaths>> {
    let store = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.to_string());
    let members = store
        .list_members(project_id)
        .map_err(product_store_api_error)?;
    let mut repos = Vec::new();
    for member in members {
        for checkout_id in &member.checkout_ids {
            if let Some(checkout) = store
                .load_checkout(project_id, *checkout_id)
                .map_err(product_store_api_error)?
            {
                repos.push(MemberRepoPaths {
                    repo_root: checkout.canonical_path,
                });
            }
        }
    }
    Ok(repos)
}

/// 组装一个 LC 作用域的知识库依赖图（生产口径：gateway turn + 共享 run registry）。
fn knowledge_base_dependencies(
    state: &WebAppState,
    project_id: &str,
    lc_id: &str,
) -> ApiResult<KnowledgeBaseDependencies> {
    let paths = product_app_paths(state);
    let aggregate = state.aggregate_initialization_dependencies();
    let trust: Arc<dyn ProviderTrustPrecondition> = aggregate
        .trust()
        .cloned()
        .unwrap_or_else(|| Arc::new(NoopTrustPrecondition));
    let gateway = state
        .gateway_factory()
        .cloned()
        .ok_or_else(|| {
            ApiError::runtime(
                "logical_provider_gateway_required",
                "logical codebase provider gateway factory is not configured",
                json!({}),
            )
        })?
        .build_for_lc(project_id, Some(lc_id))
        .map_err(|error| {
            ApiError::runtime(
                "logical_provider_gateway_required",
                format!("build knowledge base provider gateway failed: {error}"),
                json!({}),
            )
        })?;
    let clock: Arc<KnowledgeBaseClock> = Arc::new(|| chrono::Utc::now().to_rfc3339());
    let coordinator =
        KnowledgeBaseCoordinator::with_gateway(paths.clone(), Arc::new(gateway), trust, clock)
            .for_lc(lc_id.to_string());
    Ok(KnowledgeBaseDependencies {
        coordinator,
        runs: aggregate.runs().clone(),
    })
}

/// 只读端点（status/files/change-packages 等）无需 gateway：LC 解析 + durable store。
fn knowledge_base_read_context(
    state: &WebAppState,
    project_id: &str,
    lc_id: &str,
) -> ApiResult<(ProductAppPaths, PathBuf, KnowledgeBaseOperationStore)> {
    validate_knowledge_base_ids(project_id, lc_id)?;
    let paths = product_app_paths(state);
    let record = require_logical_codebase(&paths, project_id, lc_id)?;
    let canonical_root = fs::canonicalize(&record.aggregate_root).map_err(|error| {
        ApiError::runtime(
            "aggregate_root_missing",
            format!(
                "canonicalize aggregate root {}: {error}",
                record.aggregate_root.display()
            ),
            json!({}),
        )
    })?;
    require_knowledge_base_aggregate_gate(&paths, project_id, lc_id)?;
    let store = KnowledgeBaseOperationStore::for_lc(paths.clone(), lc_id.to_string());
    Ok((paths, canonical_root, store))
}

/// REQ-KB-13 知识库全域统一前置门：冷启动投影的「规则/政策」步非
/// `Completed` ⇒ 拒绝。判定**复用** `preflight::aggregate_initialization_completed`
/// （唯一口径；投影读不出 / IO 故障 / LC 归属不明一律按未完成 fail-closed），
/// 不在 handler 层另写判据。门集中在两处调用：本模块的
/// [`knowledge_base_read_context`]（覆盖 status / files / file 读写 /
/// change-packages×3 / bootstrap-inputs / initializations 查询与取消共 11 个
/// 端点）与创建端点 canonicalize 之后（见 handlers 片段）；未满足时在进入
/// 任何业务逻辑前返回 422，保证零写副作用（尤其 file 写入不落盘）。
fn require_knowledge_base_aggregate_gate(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
) -> ApiResult<()> {
    if crate::product::logical_codebase::knowledge_base::preflight::aggregate_initialization_completed(
        paths, project_id, lc_id,
    ) {
        return Ok(());
    }
    // message 走失败文案单一来源 failure_copy 的中文成品标题（码表已登记
    // 该码三段），handler 不手写文案；`details.bootstrap_step` 保留供前端
    // 定位冷启动的具体步。
    use crate::product::logical_codebase::failure_copy::{describe, FailureInput};
    Err(ApiError::runtime(
        CODE_REQUIRES_AGGREGATE_INITIALIZATION,
        describe(FailureInput::new(CODE_REQUIRES_AGGREGATE_INITIALIZATION)).title_zh,
        json!({ "bootstrap_step": "rules_policy" }),
    ))
}

/// 校验路径段形式的 project/lc 标识（与聚合初始化端点同一拒绝面）。
fn validate_knowledge_base_ids(project_id: &str, lc_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })?;
    validate_relative_id(lc_id).map_err(|error| {
        ApiError::validation(
            "invalid_logical_codebase_id",
            format!("invalid logical codebase id: {error}"),
        )
    })?;
    Ok(())
}
