use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::de::DeserializeOwned;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{
    CodeReviewReport, CodingAttemptStatus, CodingChatEntry, CodingContextNote,
    CodingExecutionAttempt, CodingExecutionStage, CodingProviderRole, CodingReworkInstruction,
    CodingRoleProviderConfigSnapshot, CodingStageGateState, CodingStageGateStatus,
    CodingTimelineNode, CodingTimelineNodeStatus, InternalPrReview, ReviewRequest, TestingReport,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCodingAttemptInput {
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub base_branch: String,
    pub branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub max_auto_rework: u32,
}

#[derive(Debug, Clone)]
pub struct CodingAttemptStore {
    paths: ProductAppPaths,
}

impl CodingAttemptStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> ProductAppPaths {
        self.paths.clone()
    }

    pub fn create_attempt(
        &self,
        input: CreateCodingAttemptInput,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.work_item_id)?;

        if let Some(active) =
            self.get_active_attempt(&input.project_id, &input.issue_id, &input.work_item_id)?
        {
            return Err(ProductStoreError::Io(format!(
                "active_coding_attempt_exists: {}",
                active.id
            )));
        }

        let root = self.coding_attempts_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("coding_attempt", count_json_files(&root)?);
        let attempt_no = self
            .list_attempts_for_work_item(&input.project_id, &input.issue_id, &input.work_item_id)?
            .iter()
            .map(|attempt| attempt.attempt_no)
            .max()
            .unwrap_or(0)
            + 1;
        let now = Utc::now().to_rfc3339();
        let attempt = CodingExecutionAttempt {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            work_item_id: input.work_item_id,
            attempt_no,
            status: CodingAttemptStatus::Created,
            stage: CodingExecutionStage::PrepareContext,
            base_branch: input.base_branch,
            branch_name: input.branch_name,
            worktree_path: input.worktree_path,
            provider_config_snapshot: input.provider_config_snapshot,
            rework_count: 0,
            max_auto_rework: input.max_auto_rework,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        };

        write_json(
            &self.attempt_path(&attempt.project_id, &attempt.issue_id, &id),
            &attempt,
        )?;
        write_json(
            &self.role_provider_config_path(&attempt.project_id, &attempt.issue_id, &id),
            &CodingRoleProviderConfigSnapshot::from(&attempt.provider_config_snapshot),
        )?;
        Ok(attempt)
    }

    pub fn get_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        if !path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_attempt",
                id: attempt_id.to_string(),
            });
        }
        read_json(&path)
    }

    pub fn get_attempt_by_id(
        &self,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        self.find_attempt_by_id(attempt_id)
    }

    pub fn list_attempts_for_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        let mut attempts: Vec<CodingExecutionAttempt> =
            list_json_records(&self.coding_attempts_root(project_id, issue_id))?
                .into_iter()
                .filter(|attempt: &CodingExecutionAttempt| attempt.work_item_id == work_item_id)
                .collect();
        attempts.sort_by(|left, right| {
            left.attempt_no
                .cmp(&right.attempt_no)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(attempts)
    }

    pub fn get_active_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Option<CodingExecutionAttempt>, ProductStoreError> {
        let active = self
            .list_attempts_for_work_item(project_id, issue_id, work_item_id)?
            .into_iter()
            .find(|attempt| attempt.status.is_active());
        Ok(active)
    }

    pub fn update_attempt_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        status: CodingAttemptStatus,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        if !valid_status_transition(&attempt.status, &status) {
            return Err(ProductStoreError::Io(format!(
                "invalid_coding_attempt_status_transition: {:?} -> {:?}",
                attempt.status, status
            )));
        }
        let now = Utc::now().to_rfc3339();
        if matches!(
            status,
            CodingAttemptStatus::Completed
                | CodingAttemptStatus::Failed
                | CodingAttemptStatus::Aborted
        ) {
            attempt.completed_at = Some(now.clone());
        }
        attempt.status = status;
        attempt.updated_at = now;
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn update_attempt_stage(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        stage: CodingExecutionStage,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        if !valid_stage_transition(&attempt.stage, &stage) {
            return Err(ProductStoreError::Io(format!(
                "invalid_coding_attempt_stage_transition: {:?} -> {:?}",
                attempt.stage, stage
            )));
        }
        attempt.stage = stage;
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn update_attempt_worktree_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        worktree_path: PathBuf,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.worktree_path = Some(worktree_path);
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn update_attempt_review_request_state(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        head_commit: String,
        pushed_remote: String,
        review_request_id: String,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.head_commit = Some(head_commit);
        attempt.pushed_remote = Some(pushed_remote);
        attempt.review_request_id = Some(review_request_id);
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn update_attempt_provider_config_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        provider_config_snapshot: ProviderConfigSnapshot,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.provider_config_snapshot = provider_config_snapshot;
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn get_role_provider_config_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingRoleProviderConfigSnapshot, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let path = self.role_provider_config_path(project_id, issue_id, attempt_id);
        if path_is_regular_file(&path)? {
            return read_json(&path);
        }
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        Ok(CodingRoleProviderConfigSnapshot::from(
            &attempt.provider_config_snapshot,
        ))
    }

    pub fn update_role_provider_config_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_provider_config_snapshot: CodingRoleProviderConfigSnapshot,
    ) -> Result<CodingRoleProviderConfigSnapshot, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        write_json(
            &self.role_provider_config_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            &role_provider_config_snapshot,
        )?;
        Ok(role_provider_config_snapshot)
    }

    pub fn increment_attempt_rework_count(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.rework_count += 1;
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn create_context_note(
        &self,
        attempt_id: &str,
        content: String,
    ) -> Result<CodingContextNote, ProductStoreError> {
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let notes_root = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("context-notes");
        let id = next_sequential_id("coding_context_note", count_json_files(&notes_root)?);
        let note = CodingContextNote {
            id: id.clone(),
            attempt_id: attempt.id,
            content,
            created_at: Utc::now().to_rfc3339(),
            consumed_by_rework_round: None,
        };
        write_json(&notes_root.join(format!("{id}.json")), &note)?;
        Ok(note)
    }

    pub fn list_context_notes(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingContextNote>, ProductStoreError> {
        list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("context-notes"),
        )
    }

    pub fn save_chat_entry(&self, entry: &CodingChatEntry) -> Result<(), ProductStoreError> {
        validate_relative_id(&entry.id)?;
        let attempt = self.find_attempt_by_id(&entry.attempt_id)?;
        write_json(
            &self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("chat-entries")
                .join(format!("{}.json", entry.id)),
            entry,
        )
    }

    pub fn list_chat_entries(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingChatEntry>, ProductStoreError> {
        list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("chat-entries"),
        )
    }

    pub fn list_unconsumed_context_notes(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingContextNote>, ProductStoreError> {
        Ok(self
            .list_context_notes(project_id, issue_id, attempt_id)?
            .into_iter()
            .filter(|note| note.consumed_by_rework_round.is_none())
            .collect())
    }

    pub fn mark_context_notes_consumed(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        note_ids: &[String],
        rework_round: u32,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let notes_root = self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("context-notes");
        for note_id in note_ids {
            validate_relative_id(note_id)?;
            let path = notes_root.join(format!("{note_id}.json"));
            let mut note: CodingContextNote = read_json(&path)?;
            note.consumed_by_rework_round = Some(rework_round);
            write_json(&path, &note)?;
        }
        Ok(())
    }

    pub fn save_rework_instruction(
        &self,
        instruction: &CodingReworkInstruction,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&instruction.id)?;
        let attempt = self.find_attempt_by_id(&instruction.attempt_id)?;
        write_json(
            &self
                .rework_instructions_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join(format!("{}.json", instruction.id)),
            instruction,
        )
    }

    pub fn list_rework_instructions(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingReworkInstruction>, ProductStoreError> {
        list_json_records(&self.rework_instructions_root(project_id, issue_id, attempt_id))
    }

    pub fn latest_unconsumed_rework_instruction(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<CodingReworkInstruction>, ProductStoreError> {
        Ok(self
            .list_rework_instructions(project_id, issue_id, attempt_id)?
            .into_iter()
            .rfind(|instruction| instruction.consumed_by_node_id.is_none()))
    }

    pub fn mark_rework_instruction_consumed(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        instruction_id: &str,
        node_id: &str,
    ) -> Result<CodingReworkInstruction, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(instruction_id)?;
        validate_relative_id(node_id)?;
        let path = self
            .rework_instructions_root(project_id, issue_id, attempt_id)
            .join(format!("{instruction_id}.json"));
        let mut instruction: CodingReworkInstruction = read_json(&path)?;
        instruction.consumed_by_node_id = Some(node_id.to_string());
        instruction.consumed_at = Some(Utc::now().to_rfc3339());
        write_json(&path, &instruction)?;
        Ok(instruction)
    }

    pub fn attempt_artifact_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("artifacts")
    }

    pub fn attempt_test_output_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_artifact_root(project_id, issue_id, attempt_id)
            .join("test-output")
    }

    pub fn attempt_test_output_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        artifact_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        let file_name = Path::new(artifact_id)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| ProductStoreError::PathEscape(artifact_id.to_string()))?;
        validate_relative_id(file_name)?;
        Ok(self
            .attempt_test_output_root(project_id, issue_id, attempt_id)
            .join(file_name))
    }

    pub fn create_stage_gate(
        &self,
        attempt_id: &str,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        expires_at: String,
        provider_snapshot: CodingRoleProviderConfigSnapshot,
    ) -> Result<CodingStageGateState, ProductStoreError> {
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let gates_root = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("stage-gates");
        let gate_id = next_sequential_id("coding_stage_gate", count_json_files(&gates_root)?);
        let now = Utc::now().to_rfc3339();
        let gate = CodingStageGateState {
            gate_id: gate_id.clone(),
            attempt_id: attempt.id,
            stage,
            role,
            expires_at,
            provider_snapshot,
            status: CodingStageGateStatus::Open,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(&gates_root.join(format!("{gate_id}.json")), &gate)?;
        Ok(gate)
    }

    pub fn list_stage_gates(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingStageGateState>, ProductStoreError> {
        list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("stage-gates"),
        )
    }

    pub fn list_open_stage_gates(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingStageGateState>, ProductStoreError> {
        Ok(self
            .list_stage_gates(project_id, issue_id, attempt_id)?
            .into_iter()
            .filter(|gate| gate.status == CodingStageGateStatus::Open)
            .collect())
    }

    pub fn update_stage_gate_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
        status: CodingStageGateStatus,
    ) -> Result<CodingStageGateState, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let path = self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("stage-gates")
            .join(format!("{gate_id}.json"));
        if !path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_stage_gate",
                id: gate_id.to_string(),
            });
        }
        let mut gate: CodingStageGateState = read_json(&path)?;
        gate.status = status;
        gate.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &gate)?;
        Ok(gate)
    }

    pub fn refresh_stage_gate(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
        expires_at: String,
        provider_snapshot: CodingRoleProviderConfigSnapshot,
    ) -> Result<CodingStageGateState, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let path = self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("stage-gates")
            .join(format!("{gate_id}.json"));
        if !path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_stage_gate",
                id: gate_id.to_string(),
            });
        }
        let mut gate: CodingStageGateState = read_json(&path)?;
        gate.expires_at = expires_at;
        gate.provider_snapshot = provider_snapshot;
        gate.status = CodingStageGateStatus::Open;
        gate.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &gate)?;
        Ok(gate)
    }

    pub fn save_testing_report(&self, report: &TestingReport) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&report.attempt_id)?;
        write_json(
            &self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("testing-reports")
                .join(format!("{}.json", report.id)),
            report,
        )
    }

    pub fn get_testing_report(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        report_id: &str,
    ) -> Result<TestingReport, ProductStoreError> {
        validate_relative_id(report_id)?;
        read_json(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("testing-reports")
                .join(format!("{report_id}.json")),
        )
    }

    pub fn list_testing_reports(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<TestingReport>, ProductStoreError> {
        list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("testing-reports"),
        )
    }

    pub fn save_code_review_report(
        &self,
        report: &CodeReviewReport,
    ) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&report.attempt_id)?;
        write_json(
            &self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("code-reviews")
                .join(format!("{}.json", report.id)),
            report,
        )
    }

    pub fn list_code_review_reports(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodeReviewReport>, ProductStoreError> {
        list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("code-reviews"),
        )
    }

    pub fn save_review_request(&self, request: &ReviewRequest) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&request.attempt_id)?;
        write_json(
            &self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("review-requests")
                .join(format!("{}.json", request.id)),
            request,
        )
    }

    pub fn get_review_request(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        request_id: &str,
    ) -> Result<ReviewRequest, ProductStoreError> {
        validate_relative_id(request_id)?;
        read_json(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("review-requests")
                .join(format!("{request_id}.json")),
        )
    }

    pub fn list_review_requests(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<ReviewRequest>, ProductStoreError> {
        list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("review-requests"),
        )
    }

    pub fn save_internal_pr_review(
        &self,
        review: &InternalPrReview,
    ) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&review.attempt_id)?;
        write_json(
            &self
                .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join("internal-reviews")
                .join(format!("{}.json", review.id)),
            review,
        )
    }

    pub fn get_internal_pr_review(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        review_id: &str,
    ) -> Result<InternalPrReview, ProductStoreError> {
        validate_relative_id(review_id)?;
        read_json(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("internal-reviews")
                .join(format!("{review_id}.json")),
        )
    }

    pub fn list_internal_pr_reviews(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<InternalPrReview>, ProductStoreError> {
        list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("internal-reviews"),
        )
    }

    pub fn save_timeline_node(&self, node: CodingTimelineNode) -> Result<(), ProductStoreError> {
        let attempt = self.find_attempt_by_id(&node.attempt_id)?;
        let path = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("timeline-nodes.json");
        let mut nodes: Vec<CodingTimelineNode> = if path_is_regular_file(&path)? {
            read_json(&path)?
        } else {
            Vec::new()
        };
        if let Some(existing) = nodes.iter_mut().find(|existing| existing.id == node.id) {
            *existing = node;
        } else {
            nodes.push(node);
        }
        write_json(&path, &nodes)
    }

    pub fn get_timeline_nodes(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingTimelineNode>, ProductStoreError> {
        let path = self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("timeline-nodes.json");
        if !path_is_regular_file(&path)? {
            return Ok(Vec::new());
        }
        read_json(&path)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_timeline_node_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        node_id: &str,
        status: CodingTimelineNodeStatus,
        summary: Option<String>,
        completed_at: Option<String>,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(node_id)?;
        let path = self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("timeline-nodes.json");
        let mut nodes: Vec<CodingTimelineNode> = if path_is_regular_file(&path)? {
            read_json(&path)?
        } else {
            return Err(ProductStoreError::NotFound {
                kind: "coding_timeline_node",
                id: node_id.to_string(),
            });
        };
        let Some(node) = nodes.iter_mut().find(|node| node.id == node_id) else {
            return Err(ProductStoreError::NotFound {
                kind: "coding_timeline_node",
                id: node_id.to_string(),
            });
        };
        node.status = status;
        node.summary = summary;
        node.completed_at = completed_at;
        write_json(&path, &nodes)
    }

    fn find_attempt_by_id(
        &self,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        let mut found = None;
        for project_path in child_directories(&self.paths.projects_root())? {
            let Some(project_id) = project_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let issues_root = project_path.join("issues");
            for issue_path in child_directories(&issues_root)? {
                let Some(issue_id) = issue_path.file_name().and_then(|value| value.to_str()) else {
                    continue;
                };
                let path = self.attempt_path(project_id, issue_id, attempt_id);
                if !path_is_regular_file(&path)? {
                    continue;
                }
                if found.is_some() {
                    return Err(ProductStoreError::Io(format!(
                        "coding_attempt_ambiguous: {attempt_id}"
                    )));
                }
                found = Some(read_json(&path)?);
            }
        }
        found.ok_or_else(|| ProductStoreError::NotFound {
            kind: "coding_attempt",
            id: attempt_id.to_string(),
        })
    }

    fn attempt_path(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf {
        self.coding_attempts_root(project_id, issue_id)
            .join(format!("{attempt_id}.json"))
    }

    fn attempt_dir(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf {
        self.coding_attempts_root(project_id, issue_id)
            .join(attempt_id)
    }

    fn role_provider_config_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("role-provider-config.json")
    }

    fn rework_instructions_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("rework-instructions")
    }

    fn coding_attempts_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("coding-attempts")
    }
}

fn valid_status_transition(current: &CodingAttemptStatus, next: &CodingAttemptStatus) -> bool {
    if current == next {
        return true;
    }
    match current {
        CodingAttemptStatus::Created => {
            matches!(
                next,
                CodingAttemptStatus::Running | CodingAttemptStatus::Aborted
            )
        }
        CodingAttemptStatus::Running => matches!(
            next,
            CodingAttemptStatus::WaitingForHuman
                | CodingAttemptStatus::Blocked
                | CodingAttemptStatus::Completed
                | CodingAttemptStatus::Failed
                | CodingAttemptStatus::Aborted
        ),
        CodingAttemptStatus::WaitingForHuman => {
            matches!(
                next,
                CodingAttemptStatus::Running
                    | CodingAttemptStatus::Completed
                    | CodingAttemptStatus::Aborted
            )
        }
        CodingAttemptStatus::Blocked => {
            matches!(
                next,
                CodingAttemptStatus::Running | CodingAttemptStatus::Aborted
            )
        }
        CodingAttemptStatus::Completed
        | CodingAttemptStatus::Failed
        | CodingAttemptStatus::Aborted => false,
    }
}

fn valid_stage_transition(current: &CodingExecutionStage, next: &CodingExecutionStage) -> bool {
    if current == next {
        return true;
    }
    if matches!(next, CodingExecutionStage::Rework) {
        return true;
    }
    if matches!(current, CodingExecutionStage::Rework) {
        return matches!(
            next,
            CodingExecutionStage::Coding
                | CodingExecutionStage::Testing
                | CodingExecutionStage::CodeReview
                | CodingExecutionStage::ReviewRequest
                | CodingExecutionStage::InternalPrReview
                | CodingExecutionStage::FinalConfirm
        );
    }
    next.order() >= current.order()
}

fn list_json_records<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, ProductStoreError> {
    let entries = json_file_paths(path)?;
    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        records.push(read_json(&entry)?);
    }
    Ok(records)
}

fn count_json_files(path: &Path) -> Result<usize, ProductStoreError> {
    Ok(json_file_paths(path)?.len())
}

fn json_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        let entry_path = entry.path();
        if file_type.is_file()
            && entry_path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

fn child_directories(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        if file_type.is_dir() {
            entries.push(entry.path());
        }
    }
    entries.sort();
    Ok(entries)
}

fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

fn path_is_regular_file(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}
