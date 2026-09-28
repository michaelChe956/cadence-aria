//! C4 Task 7：identity journal repair 的 HTTP DTO。
//!
//! 只投影 `IdentityJournalDiagnostic`/repair action 的 durable 事实；字段
//! 与 product 类型一一对应（snake_case），不另造状态机。

use serde::{Deserialize, Serialize};

use crate::product::logical_codebase::IdentityJournalDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepositoryIdentityMappingDto {
    pub legacy_repository_id: String,
    pub source_identity_digest: String,
    pub logical_repository_id: String,
    pub primary_checkout_id: String,
    pub physical_repository_id: String,
    pub idempotency_key: String,
    pub authority_written: bool,
    pub compatibility_backfilled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IdentityJournalDiagnosticDto {
    pub project_id: String,
    pub logical_codebase_id: String,
    pub migration_id: String,
    pub phase: String,
    pub source_repos_digest: String,
    pub observed_source_repos_digest: Option<String>,
    pub read_mode: Option<String>,
    pub completed_keys: Vec<String>,
    pub mappings: Vec<RepositoryIdentityMappingDto>,
    pub candidates: Vec<RepositoryIdentityMappingDto>,
    pub conflicts: Vec<String>,
    pub impact: Vec<String>,
    pub allowed_actions: Vec<String>,
}

impl From<&IdentityJournalDiagnostic> for IdentityJournalDiagnosticDto {
    fn from(diagnostic: &IdentityJournalDiagnostic) -> Self {
        fn mapping_dto(
            mapping: &crate::product::logical_codebase::RepositoryIdentityMapping,
        ) -> RepositoryIdentityMappingDto {
            RepositoryIdentityMappingDto {
                legacy_repository_id: mapping.legacy_repository_id.clone(),
                source_identity_digest: mapping.source_identity_digest.clone(),
                logical_repository_id: mapping.logical_repository_id.0.to_string(),
                primary_checkout_id: mapping.primary_checkout_id.0.to_string(),
                physical_repository_id: mapping.physical_repository_id.clone(),
                idempotency_key: mapping.idempotency_key.clone(),
                authority_written: mapping.authority_written,
                compatibility_backfilled: mapping.compatibility_backfilled,
            }
        }
        Self {
            project_id: diagnostic.project_id.clone(),
            logical_codebase_id: diagnostic.logical_codebase_id.clone(),
            migration_id: diagnostic.migration_id.clone(),
            phase: phase_str(&diagnostic.phase),
            source_repos_digest: diagnostic.source_repos_digest.clone(),
            observed_source_repos_digest: diagnostic.observed_source_repos_digest.clone(),
            read_mode: diagnostic.read_mode.clone(),
            completed_keys: diagnostic.completed_keys.clone(),
            mappings: diagnostic
                .mappings
                .iter()
                .map(mapping_dto)
                .collect(),
            candidates: diagnostic
                .candidates
                .iter()
                .map(mapping_dto)
                .collect(),
            conflicts: diagnostic.conflicts.clone(),
            impact: diagnostic.impact.clone(),
            allowed_actions: diagnostic
                .allowed_actions
                .iter()
                .map(|action| match action {
                    crate::product::logical_codebase::IdentityRepairActionKind::ContinueSafePrefix => {
                        "continue_safe_prefix"
                    }
                    crate::product::logical_codebase::IdentityRepairActionKind::SubmitMapping => {
                        "submit_mapping"
                    }
                    crate::product::logical_codebase::IdentityRepairActionKind::Revalidate => {
                        "revalidate"
                    }
                })
                .map(str::to_string)
                .collect(),
        }
    }
}

fn phase_str(phase: &crate::product::logical_codebase::IdentityMigrationPhase) -> String {
    match phase {
        crate::product::logical_codebase::IdentityMigrationPhase::Scanning => "scanning",
        crate::product::logical_codebase::IdentityMigrationPhase::Mapping => "mapping",
        crate::product::logical_codebase::IdentityMigrationPhase::WritingAuthority => {
            "writing_authority"
        }
        crate::product::logical_codebase::IdentityMigrationPhase::BackfillingCompatibility => {
            "backfilling_compatibility"
        }
        crate::product::logical_codebase::IdentityMigrationPhase::DualReadWrite => "dual_read_write",
        crate::product::logical_codebase::IdentityMigrationPhase::SwitchingReads => "switching_reads",
        crate::product::logical_codebase::IdentityMigrationPhase::LegacyFallbackRemoved => {
            "legacy_fallback_removed"
        }
        crate::product::logical_codebase::IdentityMigrationPhase::Completed => "completed",
        crate::product::logical_codebase::IdentityMigrationPhase::Failed => "failed",
    }
    .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IdentityMappingSubmissionDto {
    pub legacy_repository_id: String,
    pub source_identity_digest: String,
    pub logical_repository_id: String,
    pub primary_checkout_id: String,
    pub physical_repository_id: String,
    #[serde(default)]
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IdentityRepairActionRequestDto {
    pub command_id: String,
    pub expected_journal_updated_at: String,
    pub action: String,
    #[serde(default)]
    pub mapping: Option<IdentityMappingSubmissionDto>,
}
