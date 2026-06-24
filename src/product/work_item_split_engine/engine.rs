use serde_json::json;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::product::lifecycle_store::LifecycleStore;
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
        let provider_type = provider_name_to_type(&author_provider);
        let worktree_path = repository.path.to_string_lossy().to_string();
        let adapter_input = AdapterInput {
            provider_type,
            role: AdapterRole::WorkItemSplitter,
            worktree_path: Some(worktree_path),
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
