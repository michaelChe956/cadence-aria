use serde::{Deserialize, Serialize};

use crate::product::coding_models::{
    CodingExecutionStage, CodingProviderRole, CodingRoleProviderConfigSnapshot,
};
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodingRunnerCommand {
    ProviderSelect {
        role: String,
        provider: ProviderName,
    },
    StageGateConfirm {
        stage: CodingExecutionStage,
    },
    PermissionResponse {
        id: String,
        approved: bool,
        reason: Option<String>,
    },
    ChoiceResponse {
        id: String,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
    },
    AbortAttempt,
}

pub fn coding_provider_role_for_stage(stage: &CodingExecutionStage) -> Option<CodingProviderRole> {
    match stage {
        CodingExecutionStage::Coding => Some(CodingProviderRole::Coder),
        CodingExecutionStage::Testing => Some(CodingProviderRole::Tester),
        CodingExecutionStage::Rework => Some(CodingProviderRole::Analyst),
        CodingExecutionStage::CodeReview => Some(CodingProviderRole::CodeReviewer),
        CodingExecutionStage::InternalPrReview => Some(CodingProviderRole::InternalReviewer),
        CodingExecutionStage::PrepareContext
        | CodingExecutionStage::WorktreePrepare
        | CodingExecutionStage::ReviewRequest
        | CodingExecutionStage::FinalConfirm => None,
    }
}

pub fn parse_coding_provider_role(role: &str) -> Option<CodingProviderRole> {
    match role {
        "author" | "coder" => Some(CodingProviderRole::Coder),
        "tester" => Some(CodingProviderRole::Tester),
        "analyst" => Some(CodingProviderRole::Analyst),
        "reviewer" | "code_reviewer" => Some(CodingProviderRole::CodeReviewer),
        "internal_reviewer" => Some(CodingProviderRole::InternalReviewer),
        _ => None,
    }
}

pub fn apply_provider_selection_to_snapshots(
    role: &str,
    provider: ProviderName,
    legacy_snapshot: &mut ProviderConfigSnapshot,
    role_snapshot: &mut CodingRoleProviderConfigSnapshot,
) -> Result<CodingProviderRole, String> {
    match role {
        "author" => {
            legacy_snapshot.author = provider.clone();
            role_snapshot.coder = provider.clone();
            role_snapshot.tester = provider.clone();
            role_snapshot.analyst = provider;
            Ok(CodingProviderRole::Coder)
        }
        "reviewer" => {
            legacy_snapshot.reviewer = Some(provider.clone());
            role_snapshot.code_reviewer = provider.clone();
            role_snapshot.internal_reviewer = provider;
            Ok(CodingProviderRole::CodeReviewer)
        }
        "coder" => {
            legacy_snapshot.author = provider.clone();
            role_snapshot.coder = provider;
            Ok(CodingProviderRole::Coder)
        }
        "tester" => {
            role_snapshot.tester = provider;
            Ok(CodingProviderRole::Tester)
        }
        "analyst" => {
            role_snapshot.analyst = provider;
            Ok(CodingProviderRole::Analyst)
        }
        "code_reviewer" => {
            legacy_snapshot.reviewer = Some(provider.clone());
            role_snapshot.code_reviewer = provider;
            Ok(CodingProviderRole::CodeReviewer)
        }
        "internal_reviewer" => {
            role_snapshot.internal_reviewer = provider;
            Ok(CodingProviderRole::InternalReviewer)
        }
        _ => Err(format!("unsupported_coding_provider_role: {role}")),
    }
}
