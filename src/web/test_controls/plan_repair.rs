use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod fault;
mod linked_workspace;
mod provider_matrix;
mod recovery;
mod seed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRepairFaultPoint {
    AfterDraftSaved,
    AfterProjectionGenerated,
    AfterPlanReview,
    AfterAmendmentPrepared,
    AfterPlanPublished,
    AfterPlanBindingWritten,
    AfterUnitRunsWritten,
    AfterResumeTargetWritten,
    AfterHandoffPublished,
}

impl PlanRepairFaultPoint {
    pub const ALL: [Self; 9] = [
        Self::AfterDraftSaved,
        Self::AfterProjectionGenerated,
        Self::AfterPlanReview,
        Self::AfterAmendmentPrepared,
        Self::AfterPlanPublished,
        Self::AfterPlanBindingWritten,
        Self::AfterUnitRunsWritten,
        Self::AfterResumeTargetWritten,
        Self::AfterHandoffPublished,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlanRepairFixtureControl {
    pub fault_point: Option<PlanRepairFaultPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRepairFixtureWaiting {
    pub attempt_status: String,
    pub active_logical_work_item_id: String,
    pub active_unit_rework_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRepairFixtureRecovered {
    pub bound_plan_revision_id: String,
    pub active_plan_revision_id: String,
    pub active_amendment_id: Option<String>,
    pub logical_active_revision_ids: BTreeMap<String, String>,
    pub current_work_item_revision_id: String,
    pub current_resolved_handoff_revision_ids: Vec<String>,
    pub rewritten_logical_work_item_ids: Vec<String>,
    pub revalidated_logical_work_item_ids: Vec<String>,
    pub repair_request_count: usize,
    pub amendment_reference_ids: Vec<String>,
    pub unique_amendment_reference_ids: usize,
    pub amendment_artifact_ids: Vec<String>,
    pub unique_amendment_artifact_ids: usize,
    pub unit_run_ids: Vec<String>,
    pub unique_unit_run_ids: usize,
    pub handoff_revision_ids: Vec<String>,
    pub unique_handoff_revision_ids: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRepairDirtyGateSnapshot {
    pub open_gate_count: usize,
    pub open_gate_reason_codes: Vec<String>,
    pub application_journal_count: usize,
    pub bound_plan_revision_id: String,
    pub applied_amendment_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRepairIdentitySnapshot {
    pub request_id: String,
    pub amendment_id: String,
    pub child_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRepairProviderMatrixResult {
    pub provider: crate::product::models::ProviderName,
    pub rendered_contract_ids_preserved: bool,
    pub author_contract_ids: Vec<String>,
    pub plan_review_passed: bool,
    pub coder_defect_class: crate::product::models::PlanDefectClass,
    pub code_review_defect_class: crate::product::models::PlanDefectClass,
    pub code_review_route: crate::product::models::PlanDefectRoute,
    pub author_draft_artifact_persisted: bool,
    pub plan_review_complete_event_observed: bool,
    pub coding_role_run_count: usize,
    pub coding_raw_output_ref_count: usize,
}

#[derive(Debug)]
pub struct PlanRepairFixtureError {
    message: String,
    fault_point: Option<PlanRepairFaultPoint>,
}

impl PlanRepairFixtureError {
    fn not_implemented(operation: &str) -> Self {
        Self {
            message: format!("plan_repair_fixture_not_implemented: {operation}"),
            fault_point: None,
        }
    }

    fn injected(fault_point: PlanRepairFaultPoint) -> Self {
        Self {
            message: format!("plan_repair_fixture_fault: {fault_point:?}"),
            fault_point: Some(fault_point),
        }
    }

    pub fn fault_point(&self) -> Option<PlanRepairFaultPoint> {
        self.fault_point
    }
}

impl fmt::Display for PlanRepairFixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PlanRepairFixtureError {}

#[derive(Debug, Clone)]
pub struct PlanRepairFixtureRuntime {
    root: PathBuf,
    control: PlanRepairFixtureControl,
}

impl PlanRepairFixtureRuntime {
    pub async fn seed(
        root: &Path,
        control: PlanRepairFixtureControl,
    ) -> Result<Self, PlanRepairFixtureError> {
        std::fs::create_dir_all(root).map_err(|error| PlanRepairFixtureError {
            message: format!("plan_repair_fixture_root_failed: {error}"),
            fault_point: None,
        })?;
        seed::seed_initial_fixture(root)?;
        Ok(Self {
            root: root.to_path_buf(),
            control,
        })
    }

    pub async fn reopen(root: &Path) -> Result<Self, PlanRepairFixtureError> {
        if !root.is_dir() {
            return Err(PlanRepairFixtureError {
                message: format!("plan_repair_fixture_root_missing: {}", root.display()),
                fault_point: None,
            });
        }
        Ok(Self {
            root: root.to_path_buf(),
            control: PlanRepairFixtureControl::default(),
        })
    }

    pub async fn drive_until_review_finds_upstream_contract_invalid(
        &self,
    ) -> Result<PlanRepairFixtureWaiting, PlanRepairFixtureError> {
        seed::route_upstream_contract_invalid(&self.root).await
    }

    pub async fn drive_until_awaiting_confirmation(
        &self,
    ) -> Result<PlanRepairIdentitySnapshot, PlanRepairFixtureError> {
        recovery::ensure_review_is_routed(&self.root).await?;
        let _ = recovery::prepare_review_and_awaiting(&self.root).await?;
        let identity = recovery::plan_repair_identity(&self.root)?;
        let store = crate::product::coding_attempt_store::CodingAttemptStore::new(
            seed::fixture_paths(&self.root),
        );
        let attempt = recovery::fixture_attempt(&store)?;
        crate::web::workspace_ws_handler::refresh_coding_runtime_revision_history(
            &store.paths(),
            &attempt,
            Some(&identity.child_session_id),
        )
        .map_err(recovery::fixture_error)?;
        Ok(identity)
    }

    pub async fn confirm_publish_apply_and_resume(
        &self,
    ) -> Result<PlanRepairFixtureRecovered, PlanRepairFixtureError> {
        recovery::confirm_publish_apply_and_resume(&self.root).await
    }

    pub async fn drive_until_fault(&self) -> Result<(), PlanRepairFixtureError> {
        let fault_point = self
            .control
            .fault_point
            .ok_or_else(|| PlanRepairFixtureError::not_implemented("fault_drive"))?;
        fault::drive_until_fault(&self.root, fault_point).await
    }

    pub async fn recover_to_completion(
        &self,
    ) -> Result<PlanRepairFixtureRecovered, PlanRepairFixtureError> {
        fault::recover_to_completion(&self.root).await
    }

    pub async fn replay_duplicate_plan_defect_finding(&self) -> Result<(), PlanRepairFixtureError> {
        self.replay_plan_defect_finding("code_review_report_0001_finding_0001")
            .await
    }

    pub(crate) async fn replay_plan_defect_finding(
        &self,
        finding_id: &str,
    ) -> Result<(), PlanRepairFixtureError> {
        seed::replay_plan_defect_finding(&self.root, finding_id).await
    }

    pub async fn start_overlapping_plan_defect_findings(
        &self,
    ) -> Result<[PlanRepairIdentitySnapshot; 2], PlanRepairFixtureError> {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let start = |finding_id: &'static str| {
            let root = self.root.clone();
            let barrier = barrier.clone();
            tokio::task::spawn_blocking(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| PlanRepairFixtureError {
                        message: format!("plan_repair_fixture_runtime_failed: {error}"),
                        fault_point: None,
                    })?;
                barrier.wait();
                runtime.block_on(seed::replay_plan_defect_finding(&root, finding_id))?;
                recovery::plan_repair_identity(&root)
            })
        };
        let first = start("code_review_report_0001_finding_0001");
        let second = start("code_review_report_0001_finding_0002");
        let (first, second) =
            tokio::try_join!(first, second).map_err(|error| PlanRepairFixtureError {
                message: format!("plan_repair_fixture_join_failed: {error}"),
                fault_point: None,
            })?;
        Ok([first?, second?])
    }

    pub fn plan_repair_identity(
        &self,
    ) -> Result<PlanRepairIdentitySnapshot, PlanRepairFixtureError> {
        recovery::plan_repair_identity(&self.root)
    }

    #[cfg(test)]
    pub(crate) fn authoritative_plan_repair_request(
        &self,
    ) -> Result<crate::product::models::PlanRepairRequest, PlanRepairFixtureError> {
        recovery::authoritative_plan_repair_request(&self.root)
    }

    pub async fn start_stale_base_plan_repair(
        &self,
    ) -> Result<(), crate::product::plan_repair::PlanRepairError> {
        recovery::start_stale_base_plan_repair(&self.root).await
    }

    pub fn plan_repair_request_count(&self) -> Result<usize, PlanRepairFixtureError> {
        recovery::plan_repair_request_count(&self.root)
    }

    pub async fn publish_then_attempt_dirty_worktree_apply(
        &self,
    ) -> Result<PlanRepairDirtyGateSnapshot, PlanRepairFixtureError> {
        recovery::publish_then_attempt_dirty_worktree_apply(&self.root).await
    }

    pub async fn restore_linked_workspace_matrix(
        &self,
    ) -> Result<
        Vec<crate::product::workspace_engine::LinkedWorkspaceSessionSnapshot>,
        PlanRepairFixtureError,
    > {
        linked_workspace::restore_linked_workspace_matrix(&self.root).await
    }

    pub async fn run_provider_matrix(
        &self,
        provider: crate::product::models::ProviderName,
    ) -> Result<PlanRepairProviderMatrixResult, PlanRepairFixtureError> {
        provider_matrix::run_provider_matrix(&self.root, provider).await
    }
}
