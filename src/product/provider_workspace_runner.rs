use crate::cross_cutting::provider_adapter::{
    DEFAULT_PROVIDER_TIMEOUT_SECS, ProviderAdapter, ProviderAdapterError,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
use crate::product::entity_operation_store::{
    EntityOperationError, EntityOperationStore, EntityRunLease,
};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::{
    AppendProviderReviewRoundInput, AppendSpecVersionInput, LifecycleStore,
};
use crate::product::models::{
    ProviderName, ProviderReviewRoundRecord, SpecVersionRecord, WorkspaceSessionRecord,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::workspace_repository::workspace_repository_for_session;
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

/// D2.1a（fix-cascade-deletion Task 3.1；REQ-DEL-06，plan §0.2/§0.4）：运行
/// 准入 scope 归约。归约链（真实归属，不以 prompt/run_ref 猜 owner）：
/// ① D1.0 会话身份索引（显式 id 创建路径落盘；墓碑 fail-closed）；
/// ② manager/REST 绑定的 session record：Story/Design/WorkItemPlan 直接映射，
///    WorkItem 先 `work_item_child_binding.plan_id`，再按 issue 计划
///    `work_item_ids` refs 反查拥有 Plan（D1.3a `owning_plan_scope` 同链）。
/// 全部来源耗尽（旧无 owner 档案）→ None（保持原行为：不误伤、也不当
/// live 授权）。Conflict（身份索引命中墓碑）同样回落记录归约——scope 一致，
/// 阶段分类交给 acquire_run 在锁内精确判定。
pub(crate) fn resolve_session_run_scope(
    paths: &ProductAppPaths,
    session: &WorkspaceSessionRecord,
) -> Result<Option<EntityScope>, ProductStoreError> {
    let operations = EntityOperationStore::new(paths.clone());
    match operations.scope_for_workspace_session(&session.id) {
        Ok(scope) => Ok(Some(scope)),
        Err(ProductStoreError::NotFound { .. }) | Err(ProductStoreError::Conflict { .. }) => {
            resolve_session_scope_from_record(paths, session)
        }
        Err(other) => Err(other),
    }
}

/// 记录面归约（②）：只读、不持锁。
fn resolve_session_scope_from_record(
    paths: &ProductAppPaths,
    session: &WorkspaceSessionRecord,
) -> Result<Option<EntityScope>, ProductStoreError> {
    match session.workspace_type {
        WorkspaceType::Story => Ok(Some(EntityScope::new(
            &session.project_id,
            &session.issue_id,
            DeletionEntityKind::Story,
            &session.entity_id,
        ))),
        WorkspaceType::Design => Ok(Some(EntityScope::new(
            &session.project_id,
            &session.issue_id,
            DeletionEntityKind::Design,
            &session.entity_id,
        ))),
        WorkspaceType::WorkItemPlan => Ok(Some(EntityScope::new(
            &session.project_id,
            &session.issue_id,
            DeletionEntityKind::Plan,
            &session.entity_id,
        ))),
        WorkspaceType::WorkItem => {
            if let Some(binding) = &session.work_item_child_binding {
                return Ok(Some(EntityScope::new(
                    &session.project_id,
                    &session.issue_id,
                    DeletionEntityKind::Plan,
                    &binding.plan_id,
                )));
            }
            for plan in LifecycleStore::new(paths.clone())
                .list_issue_work_item_plans(&session.project_id, &session.issue_id)?
            {
                if plan.work_item_ids.iter().any(|id| id == &session.entity_id) {
                    return Ok(Some(EntityScope::new(
                        &session.project_id,
                        &session.issue_id,
                        DeletionEntityKind::Plan,
                        &plan.id,
                    )));
                }
            }
            Ok(None)
        }
    }
}

/// D2.1a：run lease 获取（manager 与 REST 共用）。最外层取得、任务全生命
/// 周期持有；Deleting/Deleted/Busy 由 acquire_run 在 scope 锁内精确分类
/// （REQ-DEL-06 稳定码）；None（旧无 owner 档案）→ 无 lease 运行（原行为）。
pub(crate) fn acquire_run_lease_for_session(
    paths: &ProductAppPaths,
    session: &WorkspaceSessionRecord,
) -> Result<Option<EntityRunLease>, EntityOperationError> {
    let scope = resolve_session_run_scope(paths, session).map_err(EntityOperationError::Store)?;
    match scope {
        Some(scope) => EntityOperationStore::new(paths.clone())
            .acquire_run(&scope)
            .map(Some),
        None => Ok(None),
    }
}

/// D2.1a：准入失败的稳定错误（REST 面以 message 前缀携带稳定码）。
fn run_admission_error(error: EntityOperationError) -> ProviderAdapterError {
    let code = match &error {
        EntityOperationError::Deleting => "entity_deleting",
        EntityOperationError::Deleted => "entity_deleted",
        EntityOperationError::Busy => "entity_busy",
        EntityOperationError::Store(_) => "entity_admission_failed",
    };
    ProviderAdapterError::incompatible_output(
        format!("{code}: workspace session owner entity admission rejected: {error}"),
        String::new(),
        String::new(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderRunInput {
    pub session_id: String,
    pub user_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderRunOutput {
    pub session: WorkspaceSessionRecord,
    pub version: SpecVersionRecord,
    pub review_rounds: Vec<ProviderReviewRoundRecord>,
}

#[derive(Debug, Clone)]
pub struct ProviderWorkspaceRunner {
    paths: ProductAppPaths,
}

impl ProviderWorkspaceRunner {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn run_next(
        &self,
        input: WorkspaceProviderRunInput,
        provider: &dyn ProviderAdapter,
    ) -> Result<WorkspaceProviderRunOutput, ProviderAdapterError> {
        let store = LifecycleStore::new(self.paths.clone());
        let session = store
            .get_workspace_session(&input.session_id)
            .map_err(store_error)?;
        // D2.1a（plan Interfaces）：REST 面启动前 acquire_run——lease 持有到
        // 本函数尾部全部 store writes（review/version/messages/status）完成；
        // purging/deleted 稳定拒、争用 entity_busy fail-fast（REQ-DEL-06）。
        let _run_lease =
            acquire_run_lease_for_session(&self.paths, &session).map_err(run_admission_error)?;
        let repository =
            workspace_repository_for_session(&self.paths, &store, &session).map_err(store_error)?;
        let prompt = build_prompt(&session, &input.user_prompt);
        let adapter_input = AdapterInput {
            env_vars: Default::default(),
            working_directory: None,
            provider_type: provider_type_for_name(&session.author_provider)?,
            role: AdapterRole::Orchestrator,
            worktree_path: Some(repository.path.to_string_lossy().to_string()),
            // workspace session 无 coding attempt 上下文，按契约缺省不写流日志。
            provider_stream_log_dir: None,
            prompt,
            context_files: Vec::new(),
            output_schema: "provider_workspace_markdown".to_string(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let adapter_output = provider.run(&adapter_input)?;
        let structured = adapter_output.structured_output.unwrap_or_default();
        let markdown = structured
            .get("markdown")
            .and_then(|value| value.as_str())
            .unwrap_or(adapter_output.stdout.as_str())
            .to_string();
        let review_result = structured
            .get("review_result")
            .and_then(|value| value.as_str())
            .unwrap_or("review completed")
            .to_string();
        let revision_result = structured
            .get("revision_result")
            .and_then(|value| value.as_str())
            .unwrap_or("revision completed")
            .to_string();

        let mut review_rounds = Vec::new();
        for round_index in 1..=session.review_rounds.max(1) {
            review_rounds.push(
                store
                    .append_provider_review_round(AppendProviderReviewRoundInput {
                        project_id: session.project_id.clone(),
                        issue_id: session.issue_id.clone(),
                        session_id: session.id.clone(),
                        round_index,
                        author_provider: session.author_provider.clone(),
                        reviewer_provider: session.reviewer_provider.clone(),
                        review_result: review_result.clone(),
                        revision_result: revision_result.clone(),
                    })
                    .map_err(store_error)?,
            );
        }
        let version = store
            .append_version(AppendSpecVersionInput {
                project_id: session.project_id.clone(),
                issue_id: session.issue_id.clone(),
                entity_id: session.entity_id.clone(),
                markdown: markdown.clone(),
                provider_run_refs: vec![format!("provider_run_{}", session.id)],
                review_refs: review_rounds.iter().map(|round| round.id.clone()).collect(),
                confirmed_by: None,
            })
            .map_err(store_error)?;
        store
            .append_workspace_message(&session.id, "provider".to_string(), markdown)
            .map_err(store_error)?;
        store
            .append_workspace_message(
                &session.id,
                "reviewer".to_string(),
                format!("{review_result}\n\n{revision_result}"),
            )
            .map_err(store_error)?;
        let session = store
            .update_workspace_session_status(&session.id, WorkspaceSessionStatus::WaitingForHuman)
            .map_err(store_error)?;

        Ok(WorkspaceProviderRunOutput {
            session,
            version,
            review_rounds,
        })
    }
}

fn provider_type_for_name(provider: &ProviderName) -> Result<ProviderType, ProviderAdapterError> {
    match provider {
        ProviderName::ClaudeCode => Ok(ProviderType::ClaudeCode),
        ProviderName::Codex => Ok(ProviderType::Codex),
        // F3：真实 provider 会话走本桥时显式拒绝（handler 映射为可诊断错误），
        // 不得 unreachable panic 造成连接空回复；真实 provider 的启动
        // 由 workspace session manager 的 provider run 面负责。
        ProviderName::Pi => Err(ProviderAdapterError::incompatible_output(
            "legacy fake runner does not support pi",
            String::new(),
            String::new(),
        )),
        ProviderName::KimiCode => Err(ProviderAdapterError::incompatible_output(
            "legacy fake runner does not support kimi_code",
            String::new(),
            String::new(),
        )),
        ProviderName::Fake => Ok(ProviderType::Fake),
    }
}

fn build_prompt(session: &WorkspaceSessionRecord, user_prompt: &str) -> String {
    let mut prompt = String::new();
    for message in &session.messages {
        prompt.push_str(&format!("[{}]: {}\n", message.role, message.content));
    }
    prompt.push_str(user_prompt);
    prompt
}

fn store_error(error: impl std::fmt::Display) -> ProviderAdapterError {
    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
}
#[cfg(test)]
mod tests {
    use super::*;

    /// F3（P1 前置必修）：run-next 桥对真实 provider 会话必须显式拒绝
    /// （返回可诊断错误），禁止 unreachable panic 导致连接空回复。
    #[test]
    fn legacy_runner_rejects_real_providers_explicitly_instead_of_panicking() {
        assert_eq!(
            provider_type_for_name(&ProviderName::ClaudeCode).unwrap(),
            ProviderType::ClaudeCode
        );
        assert_eq!(
            provider_type_for_name(&ProviderName::Codex).unwrap(),
            ProviderType::Codex
        );
        assert_eq!(
            provider_type_for_name(&ProviderName::Fake).unwrap(),
            ProviderType::Fake
        );
        let pi_error =
            provider_type_for_name(&ProviderName::Pi).expect_err("pi must be rejected explicitly");
        assert!(pi_error.details.contains("does not support pi"));
        assert!(
            provider_type_for_name(&ProviderName::KimiCode)
                .expect_err("kimi_code must stay rejected")
                .details
                .contains("does not support kimi_code")
        );
    }
}
