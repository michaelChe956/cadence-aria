use serde_json::json;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::session_launch::ValidatedAdapterInput;
use crate::product::cadence_skills::routing_reference::RoutingReferenceContext;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::policy::{PolicyTarget, SessionPolicyAction};
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
            // 同步栈无 policy 来源；逻辑代码库仓库在 invoke_provider 前 fail-closed，
            // prompt 构建始终注入 Legacy（与改造前字节一致）。
            &RoutingReferenceContext::Legacy,
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
            // 同步栈无 policy 来源；逻辑代码库仓库在 invoke_provider 前 fail-closed，
            // prompt 构建始终注入 Legacy（与改造前字节一致）。
            &RoutingReferenceContext::Legacy,
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
            // Legacy 单仓直连路径：不注入独立 cwd（None → 沿用 worktree_path，
            // Task 2.5 单仓两字段映射旧目录，零行为变化）。
            working_directory: None,
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
        workspace_session_id: &str,
    ) -> ApiResult<ProviderInvocationResult> {
        // Task 1b 段①尾:sync split 的真实 caller 收口流——
        // begin handle → bind sink(prepare 冻结 audit 上下文)→ start(gateway
        // run_sync,prepared launch 只走 validated trait)→ parse(complete
        // 消费已有 handle)→ complete/fail。
        let handle = lifecycle
            .begin_work_item_split_provider_run(
                &issue.project_id,
                &issue.id,
                &author_provider,
                workspace_session_id,
            )
            .map_err(product_store_api_error)?;
        let provider_type = provider_name_to_type(&author_provider);
        let worktree_path = repository.path.to_string_lossy().to_string();
        let adapter_input = AdapterInput {
            provider_type,
            role: AdapterRole::WorkItemSplitter,
            // Task 2.6(REQ-ENV-10,split sync 行):cwd 重绑 canonical root——
            // gateway 冻结的 manifest `provider_context_root`;worktree_path 仍
            // 是 target 成员路径(Task 2.5 字段合同:两字段分离,单仓回填不变)。
            working_directory: Some(gateway.authority_root().to_path_buf()),
            worktree_path: Some(worktree_path),
            provider_stream_log_dir: None,
            prompt: prompt.to_string(),
            context_files: Vec::new(),
            output_schema: WORK_ITEM_SPLIT_OUTPUT_SCHEMA.to_string(),
            timeout: 3 * 60 * 60,
            max_retries: 1,
        };
        // prepare 前绑定 run-bound sink:所有 LC 角色(含无通用 tool_policy)
        // 统一写 launch audit;audit_sink 即 LifecycleStore(生产 sink)。
        let context =
            crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext {
                workspace_session_id: workspace_session_id.to_string(),
                role_run_seq: handle.role_run_seq,
                audit_sink: std::sync::Arc::new(lifecycle.clone()),
            };

        let launch = prepare_sync_launch(
            gateway,
            &issue.project_id,
            repository,
            &author_provider,
            adapter_input,
            context,
        );
        // gateway 持有的 sync_adapter 不是 `Send`(registry 内的真实 adapter 未约束
        // Send+Sync),因此无法 `spawn_blocking` 出当前线程;改为在当前 async 任务内
        // 同步调用 `run_sync`。这与同步 adapter run 的阻塞语义一致,调用方负责确保
        // gateway 已在该 runtime 构造。
        let run_result =
            launch.and_then(|launch| gateway.run_sync(launch).map_err(map_provider_gateway_error));
        let output = match run_result {
            Ok(output) => output,
            Err(error) => {
                let _ = lifecycle.fail_work_item_split_provider_run(&handle, &error.message);
                return Err(error);
            }
        };

        let structured_output = match output.structured_output {
            Some(structured_output) => structured_output,
            None => {
                let message = "provider did not return structured output".to_string();
                let _ = lifecycle.fail_work_item_split_provider_run(&handle, &message);
                return Err(ApiError::runtime(
                    "work_item_split_provider_output_invalid",
                    message,
                    json!({}),
                ));
            }
        };

        // parse.rs 的 complete 函数消费已有 handle,不再重新
        // `save_work_item_split_provider_run`。
        super::parse::complete_split_provider_run(lifecycle, &handle, prompt, &structured_output)
            .map_err(|error| {
            let _ = lifecycle.fail_work_item_split_provider_run(&handle, &error.message);
            error
        })?;

        Ok(ProviderInvocationResult {
            structured_output,
            run_ref: handle.run_ref,
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
    context: crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext,
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
    let provider_ref =
        provider_ref_for_name(author_provider).map_err(map_provider_gateway_error)?;
    let root = gateway.authority_root().to_path_buf();
    let request = SessionLaunchRequest {
        project_id: project_id.to_string(),
        provider: provider_ref,
        action: SessionPolicyAction::PlanningReadOnly,
        target,
        // Task 2.6（REQ-ENV-10）：cwd=canonical root（与 invoke_provider_via_gateway
        // 的 AdapterInput.working_directory 同源），envelope 冻结后由 run_sync 的
        // spawn 前复验消费；readable roots 随 cwd root 化（成员位于 root 子树内的
        // 聚合布局），writable roots 为空（planning 只读）。
        working_directory: root.clone(),
        readable_roots: vec![root],
        writable_roots: Vec::new(),
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    };
    gateway
        .prepare_sync_launch(adapter_input, request, context)
        .map_err(map_provider_gateway_error)
}

/// 把 registry/availability gate 使用的 `ProviderName` 映射到 gateway 的 `ProviderRef`。
/// 经 `ProviderRef::from_provider_name` 集中 fail-closed:仅 ClaudeCode/Codex 有
/// gateway 真实 dialect;Pi/KimiCode/Fake(及未来 provider)返回显式
/// unsupported 错误,🔴 禁止静默回退 ClaudeCode——配置的 provider 不允许被
/// 悄悄换成 Claude 启动(C-2)。Codex 的 danger-full-access 路由阻断(REQ-ENV-05)
/// 由 gateway 路由级硬门施加,与本映射正交。
pub(crate) fn provider_ref_for_name(
    provider: &ProviderName,
) -> Result<crate::product::logical_codebase::provider_gateway::ProviderRef, ProviderGatewayError> {
    crate::product::logical_codebase::provider_gateway::ProviderRef::from_provider_name(
        provider,
        "cap_managed_snapshot",
    )
}
