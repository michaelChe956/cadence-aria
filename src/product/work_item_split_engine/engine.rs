use serde_json::json;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::session_launch::ValidatedAdapterInput;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::policy::PolicyTarget;
use crate::product::logical_codebase::provider_gateway::{
    LogicalCodebaseProviderGateway, ProviderGatewayError, SessionLaunchRequest,
};
use crate::product::models::{
    IssueRecord, LifecycleWorkItemRecord, ProviderName, RepositoryRecord,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole};
use crate::web::error::{ApiError, ApiResult};
use crate::web::types::GenerateWorkItemsRequest;

use super::WorkItemSplitEngine;
use super::schema::WORK_ITEM_SPLIT_OUTPUT_SCHEMA;
use super::types::{
    ProviderInvocationResult, WorkItemSplitProviderOutput, product_store_api_error,
    provider_name_to_type,
};

impl WorkItemSplitEngine {
    pub async fn generate(
        &self,
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
    ) -> ApiResult<WorkItemSplitProviderOutput> {
        let invocation = Self::build_generate_invocation(
            request,
            lifecycle,
            issue,
            repository,
            author_provider,
        )?;

        let provider_output = self
            .invoke_provider(
                &invocation.prompt,
                repository,
                invocation.author_provider.clone(),
                lifecycle,
                issue,
            )
            .await?;

        super::parse::parse_provider_output(
            lifecycle,
            request,
            issue,
            repository,
            provider_output.run_ref,
            &provider_output.structured_output,
        )
    }

    /// Revision：保留项 + redo-only 重做项 + DAG repatch。
    ///
    /// 局部重做时，prompt 注入"保留项清单（只作上下文，不允许重写）+ 重做项及反馈"，
    /// provider 只输出 redo 项。后端负责：
    /// 1. retained 原记录直接合并；
    /// 2. 为 redo 输出分配新 id / verification_plan id；
    /// 3. 用 redo_specs 顺序建立 old_id -> new_id 映射；
    /// 4. `repatch_dependencies` 把 dependency_graph 与 retained/redo 的 depends_on 中旧 id 改成新 id。
    ///
    /// retained/redo_specs 均空时表示整组 review/AutoRevision，退化为完整 split 输出解析。
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_revision(
        &self,
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        retained: &[LifecycleWorkItemRecord],
        redo_specs: &[super::types::RedoSpec],
    ) -> ApiResult<WorkItemSplitProviderOutput> {
        let invocation = Self::build_revision_invocation(
            request,
            lifecycle,
            issue,
            repository,
            author_provider,
            retained,
            redo_specs,
        )?;

        let provider_output = self
            .invoke_provider(
                &invocation.prompt,
                repository,
                invocation.author_provider,
                lifecycle,
                issue,
            )
            .await?;
        let structured = &provider_output.structured_output;

        if retained.is_empty() && redo_specs.is_empty() {
            return super::parse::parse_provider_output(
                lifecycle,
                request,
                issue,
                repository,
                provider_output.run_ref,
                structured,
            );
        }

        super::revision::materialize_revision_output(
            lifecycle,
            request,
            issue,
            repository,
            provider_output.run_ref,
            structured,
            retained,
            redo_specs,
        )
    }

    async fn invoke_provider(
        &self,
        prompt: &str,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
    ) -> ApiResult<ProviderInvocationResult> {
        // 防线：逻辑代码库仓库（logical_repository_id 为 Some）必须经
        // `LogicalCodebaseProviderGateway` 启动（REQ-ENV-01/02「无政策不得启动」）。
        // 本同步路径当前为死代码，但若未来被复活用于逻辑代码库仓库，会绕过
        // gateway 直连真实 provider——因此在此 fail-closed，不执行 adapter.run。
        if repository.logical_repository_id.is_some() {
            return Err(ApiError::runtime(
                "logical_provider_gateway_required",
                "logical codebase work item split must launch through LogicalCodebaseProviderGateway",
                json!({}),
            ));
        }

        let provider_type = provider_name_to_type(&author_provider);
        let worktree_path = repository.path.to_string_lossy().to_string();
        let adapter_input = AdapterInput {
            provider_type,
            role: AdapterRole::WorkItemSplitter,
            worktree_path: Some(worktree_path),
            // work item split 阶段无 coding attempt 上下文，按契约缺省不写流日志。
            provider_stream_log_dir: None,
            prompt: prompt.to_string(),
            context_files: Vec::new(),
            output_schema: WORK_ITEM_SPLIT_OUTPUT_SCHEMA.to_string(),
            timeout: 3 * 60 * 60,
            max_retries: 1,
        };

        let adapter = self.provider_adapter.clone();
        let output = tokio::task::spawn_blocking(move || adapter.run(&adapter_input))
            .await
            .map_err(|error| {
                ApiError::runtime(
                    "work_item_split_provider_panic",
                    "provider adapter panicked",
                    json!({"details": error.to_string()}),
                )
            })?
            .map_err(map_provider_adapter_error)?;

        let structured_output = output.structured_output.ok_or_else(|| {
            ApiError::runtime(
                "work_item_split_provider_output_invalid",
                "provider did not return structured output",
                json!({}),
            )
        })?;

        let run_ref = lifecycle
            .save_work_item_split_provider_run(
                &issue.project_id,
                &issue.id,
                &author_provider,
                prompt,
                &structured_output,
            )
            .map_err(product_store_api_error)?;

        Ok(ProviderInvocationResult {
            structured_output,
            run_ref,
        })
    }

    /// 逻辑代码库同步入口(Task 11):把 work item split 的同步 provider 调用从直接
    /// `provider_adapter.run` 改为经 `LogicalCodebaseProviderGateway::run_sync`,使
    /// 真实启动唯一由 gateway 产出并留 audit。
    ///
    /// 与 `invoke_provider` 对称:同一个 prompt、同一份 `AdapterInput`(经
    /// `WORK_ITEM_SPLIT_OUTPUT_SCHEMA`),但启动前的政策校验、canonical 复验、resume
    /// fail-closed 都由 gateway 在 spawn 前完成;调用方负责构造一个已注入 policy
    /// store/capability/target resolver/真实 registry 的 gateway。
    ///
    /// 该方法在 Web 层为逻辑代码库 issue 选定 gateway 后接入;传统单仓/非逻辑 issue
    /// 仍走 `invoke_provider` 的直接 adapter 路径,防止本工作包扩大旧 API 行为。
    #[allow(dead_code)]
    pub(crate) async fn invoke_provider_via_gateway(
        &self,
        prompt: &str,
        repository: &RepositoryRecord,
        author_provider: ProviderName,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        gateway: &LogicalCodebaseProviderGateway,
    ) -> ApiResult<ProviderInvocationResult> {
        let provider_type = provider_name_to_type(&author_provider);
        let worktree_path = repository.path.to_string_lossy().to_string();
        let adapter_input = AdapterInput {
            provider_type,
            role: AdapterRole::WorkItemSplitter,
            worktree_path: Some(worktree_path),
            provider_stream_log_dir: None,
            prompt: prompt.to_string(),
            context_files: Vec::new(),
            output_schema: WORK_ITEM_SPLIT_OUTPUT_SCHEMA.to_string(),
            timeout: 3 * 60 * 60,
            max_retries: 1,
        };

        let launch = prepare_sync_launch(
            gateway,
            &issue.project_id,
            repository,
            &author_provider,
            adapter_input,
        )?;
        // gateway 持有的 sync_adapter 不是 `Send`(registry 内的真实 adapter 未约束
        // Send+Sync),因此无法 `spawn_blocking` 出当前线程;改为在当前 async 任务内
        // 同步调用 `run_sync`。这与同步 adapter run 的阻塞语义一致,调用方负责确保
        // gateway 已在该 runtime 构造。
        let output = gateway
            .run_sync(launch)
            .map_err(map_provider_gateway_error)?;

        let structured_output = output.structured_output.ok_or_else(|| {
            ApiError::runtime(
                "work_item_split_provider_output_invalid",
                "provider did not return structured output",
                json!({}),
            )
        })?;

        let run_ref = lifecycle
            .save_work_item_split_provider_run(
                &issue.project_id,
                &issue.id,
                &author_provider,
                prompt,
                &structured_output,
            )
            .map_err(product_store_api_error)?;

        Ok(ProviderInvocationResult {
            structured_output,
            run_ref,
        })
    }
}

pub(crate) fn map_provider_adapter_error(error: ProviderAdapterError) -> ApiError {
    ApiError::runtime(
        "work_item_split_provider_error",
        &error.details,
        json!({
            "provider_error_code": error.code,
            "stdout": error.stdout,
            "stderr": error.stderr,
            "exit_code": error.exit_code,
        }),
    )
}

/// 把 `ProviderGatewayError` 映射为 web 层 `ApiError`。fail-closed 的政策/复验错误
/// 保持独立稳定码(经 `provider_gateway_error_code` 前缀表),使上游能区分「政策门拒绝」
/// 与「adapter 运行失败」。
pub(crate) fn map_provider_gateway_error(error: ProviderGatewayError) -> ApiError {
    let code = crate::web::handlers::provider_gateway_error_code(&error);
    ApiError::runtime(
        code,
        error.to_string(),
        json!({
            "error_kind": format!("{error:?}"),
        }),
    )
}

/// 构造逻辑代码库 work item split 的同步启动请求并验证政策,产出绑定了 validated
/// policy 的 `ValidatedAdapterInput`。逻辑 identity 缺失时 fail-closed:没有
/// `logical_repository_id`/`primary_checkout_id` 的仓库不是逻辑代码库,不应进入此路径。
///
/// work item split 为只读规划动作(`PlanningReadOnly`),readable root 覆盖仓库
/// 根,writable root 为空。配置 artifact 引用由调用方托管(Task 16 接 Web 时从
/// 集中配置仓库取得)。
pub(crate) fn prepare_sync_launch(
    gateway: &LogicalCodebaseProviderGateway,
    project_id: &str,
    repository: &RepositoryRecord,
    author_provider: &ProviderName,
    adapter_input: AdapterInput,
) -> ApiResult<ValidatedAdapterInput> {
    let logical_repository_id = repository
        .logical_repository_id
        .ok_or_else(|| {
            ApiError::runtime(
                "work_item_split_logical_identity_missing",
                "logical codebase repository identity is required for gateway launch",
                json!({"repository_id": repository.id}),
            )
        })?
        .0
        .to_string();
    let checkout_id = repository.primary_checkout_id.ok_or_else(|| {
        ApiError::runtime(
            "work_item_split_logical_checkout_missing",
            "logical codebase primary checkout id is required for gateway launch",
            json!({"repository_id": repository.id}),
        )
    })?;
    let target = PolicyTarget::checkout(
        logical_repository_id,
        checkout_id.0.to_string(),
        repository.path.clone(),
    );
    let provider_ref = provider_ref_for_name(author_provider);
    let request = SessionLaunchRequest::planning(
        project_id.to_string(),
        provider_ref,
        target,
        vec![repository.path.clone()],
        "sha256:managed-config-artifact",
    );
    let validated = gateway
        .validate(request)
        .map_err(map_provider_gateway_error)?;
    Ok(ValidatedAdapterInput::new(adapter_input, validated))
}

/// 把 registry/availability gate 使用的 `ProviderName` 映射到 gateway 的 `ProviderRef`。
/// `Fake`/`Pi` 不经 gateway(测试/内置路径不走逻辑代码库真实启动),这里仅在
/// ClaudeCode/Codex 间分发;其它 provider 映射到 ClaudeCode 占位(逻辑代码库只
/// 支持这两个真实 dialect,CapabilitySource 会在 validate 阶段拒掉不支持者)。
pub(crate) fn provider_ref_for_name(
    provider: &ProviderName,
) -> crate::product::logical_codebase::provider_gateway::ProviderRef {
    use crate::product::logical_codebase::provider_gateway::ProviderRef;
    match provider {
        ProviderName::Codex => ProviderRef::codex("cap_managed_snapshot"),
        _ => ProviderRef::claude_code("cap_managed_snapshot"),
    }
}
