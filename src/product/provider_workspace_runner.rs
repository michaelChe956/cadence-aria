use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{
    AppendProviderReviewRoundInput, AppendSpecVersionInput, LifecycleStore,
};
use crate::product::models::{
    ProviderReviewRoundRecord, SpecVersionRecord, WorkspaceSessionRecord, WorkspaceSessionStatus,
};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderRunInput {
    pub session_id: String,
    pub user_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderRunOutput {
    pub session: WorkspaceSessionRecord,
    pub version: SpecVersionRecord,
    pub review_round: ProviderReviewRoundRecord,
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
        let adapter_input = AdapterInput {
            provider_type: ProviderType::Fake,
            role: AdapterRole::Orchestrator,
            worktree_path: None,
            prompt: input.user_prompt,
            context_files: Vec::new(),
            output_schema: "provider_workspace_markdown".to_string(),
            timeout: 2400,
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

        let review_round = store
            .append_provider_review_round(AppendProviderReviewRoundInput {
                project_id: session.project_id.clone(),
                issue_id: session.issue_id.clone(),
                session_id: session.id.clone(),
                round_index: 1,
                author_provider: session.author_provider.clone(),
                reviewer_provider: session.reviewer_provider.clone(),
                review_result,
                revision_result,
            })
            .map_err(store_error)?;
        let version = store
            .append_version(AppendSpecVersionInput {
                project_id: session.project_id.clone(),
                issue_id: session.issue_id.clone(),
                entity_id: session.entity_id.clone(),
                markdown,
                provider_run_refs: vec![format!("provider_run_{}", session.id)],
                review_refs: vec![review_round.id.clone()],
                confirmed_by: None,
            })
            .map_err(store_error)?;
        let session = store
            .update_workspace_session_status(&session.id, WorkspaceSessionStatus::WaitingForHuman)
            .map_err(store_error)?;

        Ok(WorkspaceProviderRunOutput {
            session,
            version,
            review_round,
        })
    }
}

fn store_error(error: impl std::fmt::Display) -> ProviderAdapterError {
    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
}
