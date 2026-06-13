use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{
    AnalystDecisionRecord, CodeReviewReport, CodingAttemptStatus, CodingChatEntry,
    CodingContextNote, CodingExecutionAttempt, CodingExecutionStage, CodingGateAction,
    CodingGateKind, CodingGateRequired, CodingProviderRole, CodingReworkInstruction,
    CodingRoleProviderConfigSnapshot, CodingRoleRun, CodingRoleRunEvent, CodingRoleRunEventType,
    CodingRoleRunStatus, CodingRoleRunTrigger, CodingStageGateState, CodingStageGateStatus,
    CodingTimelineNode, CodingTimelineNodeStatus, InternalPrReview, QualityGateBypassAudit,
    ReviewRequest, TestPlan, TestingReport,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{
    ProductStoreError, read_json, validate_relative_artifact_ref, validate_relative_id, write_json,
};
use crate::product::models::ProviderConversationRef;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBlockedGateInput {
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub node_id: Option<String>,
    pub role: Option<CodingProviderRole>,
    pub title: String,
    pub description: String,
    pub reason_code: Option<String>,
    pub evidence_refs: Vec<String>,
    pub raw_provider_output_ref: Option<String>,
    pub available_actions: Vec<CodingGateAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateQualityBypassAuditInput {
    pub attempt_id: String,
    pub gate_id: String,
    pub stage: CodingExecutionStage,
    pub reason_code: Option<String>,
    pub skipped_required_steps: Vec<String>,
    pub operator_context: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BlockedGateStatus {
    Open,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BlockedGateRecord {
    gate: CodingGateRequired,
    attempt_id: String,
    node_id: Option<String>,
    status: BlockedGateStatus,
    created_at: String,
    updated_at: String,
}

const ROLE_RUN_EVENT_INLINE_STRING_LIMIT: usize = 16_384;
static ROLE_RUN_EVENT_LOG_MUTEX: Mutex<()> = Mutex::new(());

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
            provider_conversations: Vec::new(),
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

    pub fn save_coding_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        write_json(
            &self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            attempt,
        )?;
        write_json(
            &self.role_provider_config_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            &CodingRoleProviderConfigSnapshot::from(&attempt.provider_config_snapshot),
        )?;
        Ok(())
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

    pub fn delete_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        remove_file_if_exists(&self.attempt_path(project_id, issue_id, attempt_id))?;
        remove_dir_all_if_exists(&self.attempt_dir(project_id, issue_id, attempt_id))?;
        Ok(attempt)
    }

    pub fn delete_attempts_for_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
        let attempts = self.list_attempts_for_work_item(project_id, issue_id, work_item_id)?;
        for attempt in &attempts {
            self.delete_attempt(project_id, issue_id, &attempt.id)?;
        }
        Ok(attempts)
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

    pub fn create_role_run(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        trigger: CodingRoleRunTrigger,
        node_id: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        if let Some(node_id) = &node_id {
            validate_relative_id(node_id)?;
        }
        let existing = self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let id = next_sequential_id("coding_role_run", existing.len());
        let run_no = existing
            .iter()
            .filter(|run| run.stage == stage && run.role == role)
            .map(|run| run.run_no)
            .max()
            .unwrap_or(0)
            + 1;
        let run = CodingRoleRun {
            id,
            attempt_id: attempt.id.clone(),
            stage,
            role,
            run_no,
            status: CodingRoleRunStatus::Running,
            trigger,
            node_id,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            supersedes_run_id: None,
            superseded_by_run_id: None,
            reason_code: None,
            raw_provider_output_refs: Vec::new(),
            artifact_refs: Vec::new(),
        };
        self.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
        Ok(run)
    }

    pub fn list_role_runs(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingRoleRun>, ProductStoreError> {
        let mut runs: Vec<CodingRoleRun> =
            list_json_records(&self.role_runs_root(project_id, issue_id, attempt_id))?;
        runs.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(runs)
    }

    pub fn latest_role_run(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
    ) -> Result<Option<CodingRoleRun>, ProductStoreError> {
        Ok(self
            .list_role_runs(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|run| run.stage == stage && run.role == role))
    }

    pub fn update_role_run_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        status: CodingRoleRunStatus,
        reason_code: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        run.status = status;
        run.reason_code = reason_code;
        run.completed_at = Some(Utc::now().to_rfc3339());
        self.save_role_run(project_id, issue_id, &run)?;
        Ok(run)
    }

    pub fn attach_role_run_node(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        node_id: String,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(&node_id)?;
        let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        run.node_id = Some(node_id);
        self.save_role_run(project_id, issue_id, &run)?;
        Ok(run)
    }

    pub fn supersede_latest_role_run_and_create(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        trigger: CodingRoleRunTrigger,
        node_id: Option<String>,
        reason_code: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        let previous = self.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            stage.clone(),
            role.clone(),
        )?;
        let mut next = self.create_role_run(attempt, stage, role, trigger, node_id)?;
        next.supersedes_run_id = previous.as_ref().map(|run| run.id.clone());
        next.reason_code = reason_code;
        self.save_role_run(&attempt.project_id, &attempt.issue_id, &next)?;
        if let Some(mut previous_run) = previous {
            previous_run.status = CodingRoleRunStatus::Superseded;
            previous_run.superseded_by_run_id = Some(next.id.clone());
            previous_run.completed_at = Some(Utc::now().to_rfc3339());
            self.save_role_run(&attempt.project_id, &attempt.issue_id, &previous_run)?;
        }
        Ok(next)
    }

    pub fn update_role_run_refs(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        raw_provider_output_refs: Vec<String>,
        artifact_refs: Vec<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(role_run_id)?;
        let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        for reference in raw_provider_output_refs {
            validate_relative_artifact_ref(&reference)?;
            if !run
                .raw_provider_output_refs
                .iter()
                .any(|existing| existing == &reference)
            {
                run.raw_provider_output_refs.push(reference);
            }
        }
        for reference in artifact_refs {
            validate_relative_artifact_ref(&reference)?;
            if !run
                .artifact_refs
                .iter()
                .any(|existing| existing == &reference)
            {
                run.artifact_refs.push(reference);
            }
        }
        self.save_role_run(project_id, issue_id, &run)?;
        Ok(run)
    }

    pub fn append_role_run_event(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: &CodingRoleRun,
        event_type: CodingRoleRunEventType,
        payload: serde_json::Value,
    ) -> Result<CodingRoleRunEvent, ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        validate_relative_id(&role_run.id)?;
        if attempt.id != role_run.attempt_id {
            return Err(ProductStoreError::NotFound {
                kind: "coding_role_run_attempt",
                id: role_run.id.clone(),
            });
        }

        let path = self.role_run_event_log_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        );
        let _event_log_guard = ROLE_RUN_EVENT_LOG_MUTEX
            .lock()
            .map_err(|error| ProductStoreError::Io(format!("lock role run event log: {error}")))?;
        let sequence = next_jsonl_sequence(&path)?;
        let (payload, truncated, artifact_ref) = self.normalize_role_run_event_payload(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            sequence,
            payload,
        )?;
        let event = CodingRoleRunEvent {
            attempt_id: attempt.id.clone(),
            role_run_id: role_run.id.clone(),
            node_id: role_run.node_id.clone(),
            stage: role_run.stage.clone(),
            role: role_run.role.clone(),
            sequence,
            event_type,
            created_at: Utc::now().to_rfc3339(),
            payload,
            truncated,
            artifact_ref,
        };
        append_jsonl(&path, &event)?;
        Ok(event)
    }

    pub fn list_role_run_events(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> Result<Vec<CodingRoleRunEvent>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(role_run_id)?;
        let path = self.role_run_event_log_path(project_id, issue_id, attempt_id, role_run_id);
        let mut events: Vec<CodingRoleRunEvent> = read_jsonl_records(&path)?;
        events.sort_by_key(|event| event.sequence);
        Ok(events)
    }

    pub fn role_run_retry_diagnostic_summary(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(role_run_id)?;
        let run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        let events = self.list_role_run_events(project_id, issue_id, attempt_id, role_run_id)?;
        if events.is_empty()
            && run.reason_code.is_none()
            && run.raw_provider_output_refs.is_empty()
            && run.artifact_refs.is_empty()
        {
            return Ok(None);
        }

        let terminal = events.iter().rev().find(|event| {
            matches!(
                event.event_type,
                CodingRoleRunEventType::MessageComplete
                    | CodingRoleRunEventType::ProviderFailed
                    | CodingRoleRunEventType::Timeout
                    | CodingRoleRunEventType::Aborted
            )
        });
        let mut lines = Vec::new();
        lines.push("[previous_role_run_diagnostic]".to_string());
        lines.push(format!("role_run_id: {}", run.id));
        lines.push(format!("stage: {:?}", run.stage));
        lines.push(format!("role: {:?}", run.role));
        lines.push(format!("status: {:?}", run.status));
        if let Some(reason_code) = run.reason_code.as_deref() {
            lines.push(format!("reason_code: {reason_code}"));
        }
        if let Some(event) = terminal {
            lines.push(format!(
                "terminal_event: {}",
                coding_role_run_event_type_name(event.event_type)
            ));
            if let Some(reason) = role_run_event_payload_reason(event) {
                lines.push(format!("terminal_reason: {reason}"));
            }
        }
        lines.push("recent_events:".to_string());
        for event in events
            .iter()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            lines.push(format!(
                "- #{} {} title={} status={} detail={}",
                event.sequence,
                coding_role_run_event_type_name(event.event_type),
                role_run_event_payload_text(event, "title").unwrap_or("-"),
                role_run_event_payload_text(event, "status").unwrap_or("-"),
                role_run_event_payload_text(event, "detail").unwrap_or("-")
            ));
            for artifact_ref in role_run_event_artifact_refs(event) {
                lines.push(format!("  event_artifact_ref: {artifact_ref}"));
            }
        }
        if !run.raw_provider_output_refs.is_empty() {
            lines.push(format!(
                "raw_provider_output_refs: {}",
                run.raw_provider_output_refs.join(", ")
            ));
        }
        if !run.artifact_refs.is_empty() {
            lines.push(format!("artifact_refs: {}", run.artifact_refs.join(", ")));
        }
        let summary = truncate_utf8(&lines.join("\n"), 8_000);
        Ok(Some(summary))
    }

    pub fn read_attempt_artifact_text(
        &self,
        attempt_id: &str,
        artifact_ref: &str,
    ) -> Result<String, ProductStoreError> {
        validate_relative_artifact_ref(artifact_ref)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let path = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join(artifact_ref);
        fs::read_to_string(&path)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))
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

    pub fn replace_attempt_provider_conversations(
        &self,
        attempt_id: &str,
        provider_conversations: Vec<ProviderConversationRef>,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        let mut attempt = self.find_attempt_by_id(attempt_id)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        attempt.provider_conversations = provider_conversations;
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

    pub fn save_analyst_decision(
        &self,
        decision: &AnalystDecisionRecord,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&decision.id)?;
        let attempt = self.find_attempt_by_id(&decision.attempt_id)?;
        write_json(
            &self
                .analyst_decisions_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join(format!("{}.json", decision.id)),
            decision,
        )
    }

    pub fn list_analyst_decisions(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<AnalystDecisionRecord>, ProductStoreError> {
        list_json_records(&self.analyst_decisions_root(project_id, issue_id, attempt_id))
    }

    pub fn latest_analyst_decision(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<AnalystDecisionRecord>, ProductStoreError> {
        Ok(self
            .list_analyst_decisions(project_id, issue_id, attempt_id)?
            .into_iter()
            .last())
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

    pub fn save_provider_raw_output(
        &self,
        attempt_id: &str,
        stage: CodingExecutionStage,
        purpose: &str,
        output: &str,
    ) -> Result<String, ProductStoreError> {
        validate_relative_id(purpose)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let stage_dir_name = coding_stage_dir_name(&stage);
        let raw_root = self
            .provider_raw_output_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join(stage_dir_name);
        fs::create_dir_all(&raw_root).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", raw_root.display()))
        })?;

        let sequence = next_text_file_sequence(&raw_root, purpose)?;
        let file_name = format!("{purpose}_{sequence:04}.txt");
        let path = raw_root.join(&file_name);
        fs::write(&path, output)
            .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;

        Ok(format!(
            "provider-raw/{}/{}",
            coding_stage_dir_name(&stage),
            file_name
        ))
    }

    pub fn save_analyst_evidence(
        &self,
        attempt_id: &str,
        evidence: &str,
    ) -> Result<String, ProductStoreError> {
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let evidence_root = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("artifacts")
            .join("rework");
        fs::create_dir_all(&evidence_root).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", evidence_root.display()))
        })?;

        let sequence = next_text_file_sequence(&evidence_root, "analyst_evidence")?;
        let file_name = format!("analyst_evidence_{sequence:04}.txt");
        let path = evidence_root.join(&file_name);
        fs::write(&path, evidence)
            .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;

        Ok(format!("artifacts/rework/{}", file_name))
    }

    pub fn save_test_plan(&self, plan: &TestPlan) -> Result<(), ProductStoreError> {
        validate_relative_id(&plan.id)?;
        let attempt = self.find_attempt_by_id(&plan.attempt_id)?;
        write_json(
            &self
                .test_plans_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .join(format!("{}.json", plan.id)),
            plan,
        )
    }

    pub fn list_test_plans(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<TestPlan>, ProductStoreError> {
        list_json_records(&self.test_plans_root(project_id, issue_id, attempt_id))
    }

    pub fn create_blocked_gate(
        &self,
        input: CreateBlockedGateInput,
    ) -> Result<CodingGateRequired, ProductStoreError> {
        validate_relative_id(&input.attempt_id)?;
        if let Some(node_id) = &input.node_id {
            validate_relative_id(node_id)?;
        }
        let attempt = self.find_attempt_by_id(&input.attempt_id)?;
        let gates_root =
            self.blocked_gates_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        if let Some(existing_path) = matching_open_blocked_gate_path(&gates_root, &input)? {
            let mut record: BlockedGateRecord = read_json(&existing_path)?;
            record.gate.title = input.title;
            record.gate.description = input.description;
            record.gate.role = input.role;
            record.gate.available_actions = input.available_actions;
            record.gate.raw_provider_output_ref = input
                .raw_provider_output_ref
                .or(record.gate.raw_provider_output_ref);
            merge_unique_strings(&mut record.gate.evidence_refs, input.evidence_refs);
            record.updated_at = Utc::now().to_rfc3339();
            write_json(&existing_path, &record)?;
            return Ok(record.gate);
        }
        let gate_count =
            count_json_files(&gates_root)? + count_json_files(&gates_root.join("resolved"))?;
        let gate_id = next_sequential_id("coding_blocked_gate", gate_count);
        let now = Utc::now().to_rfc3339();
        let gate = CodingGateRequired {
            gate_id: gate_id.clone(),
            kind: CodingGateKind::Blocked,
            title: input.title,
            description: input.description,
            stage: Some(input.stage),
            role: input.role,
            expires_at: None,
            provider_snapshot: None,
            available_actions: input.available_actions,
            reason_code: input.reason_code,
            evidence_refs: input.evidence_refs,
            raw_provider_output_ref: input.raw_provider_output_ref,
        };
        let record = BlockedGateRecord {
            gate: gate.clone(),
            attempt_id: attempt.id,
            node_id: input.node_id,
            status: BlockedGateStatus::Open,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(&gates_root.join(format!("{gate_id}.json")), &record)?;
        Ok(gate)
    }

    pub fn list_open_blocked_gates(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingGateRequired>, ProductStoreError> {
        let mut records: Vec<BlockedGateRecord> =
            list_json_records(&self.blocked_gates_root(project_id, issue_id, attempt_id))?;
        records.retain(|record| record.status == BlockedGateStatus::Open);
        Ok(records.into_iter().map(|record| record.gate).collect())
    }

    pub fn resolve_blocked_gate(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<CodingGateRequired, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let gates_root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        let path = gates_root.join(format!("{gate_id}.json"));
        if !path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_blocked_gate",
                id: gate_id.to_string(),
            });
        }

        let mut record: BlockedGateRecord = read_json(&path)?;
        record.status = BlockedGateStatus::Resolved;
        record.updated_at = Utc::now().to_rfc3339();
        let gate = record.gate.clone();
        write_json(
            &gates_root.join("resolved").join(format!("{gate_id}.json")),
            &record,
        )?;
        remove_file_if_exists(&path)?;
        Ok(gate)
    }

    pub fn create_quality_bypass_audit(
        &self,
        input: CreateQualityBypassAuditInput,
    ) -> Result<QualityGateBypassAudit, ProductStoreError> {
        validate_relative_id(&input.attempt_id)?;
        validate_relative_id(&input.gate_id)?;
        let attempt = self.find_attempt_by_id(&input.attempt_id)?;
        let root =
            self.quality_bypass_audits_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let id = next_sequential_id("quality_bypass_audit", count_json_files(&root)?);
        let audit = QualityGateBypassAudit {
            id: id.clone(),
            attempt_id: attempt.id,
            gate_id: input.gate_id,
            stage: input.stage,
            reason_code: input.reason_code,
            skipped_required_steps: input.skipped_required_steps,
            operator_context: input.operator_context,
            created_at: Utc::now().to_rfc3339(),
        };
        write_json(&root.join(format!("{id}.json")), &audit)?;
        Ok(audit)
    }

    pub fn list_quality_bypass_audits(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<QualityGateBypassAudit>, ProductStoreError> {
        list_json_records(&self.quality_bypass_audits_root(project_id, issue_id, attempt_id))
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

    fn analyst_decisions_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("analyst-decisions")
    }

    fn test_plans_root(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("test-plans")
    }

    fn role_runs_root(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("role-runs")
    }

    fn role_run_events_root(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("role-run-events")
    }

    fn role_run_event_log_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> PathBuf {
        self.role_run_events_root(project_id, issue_id, attempt_id)
            .join(format!("{role_run_id}.jsonl"))
    }

    fn role_run_event_artifact_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("artifacts")
            .join("role-run-events")
            .join(role_run_id)
    }

    fn normalize_role_run_event_payload(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        sequence: u64,
        payload: serde_json::Value,
    ) -> Result<(serde_json::Value, bool, Option<String>), ProductStoreError> {
        let mut payload = payload;
        let Some(object) = payload.as_object_mut() else {
            return Ok((payload, false, None));
        };

        let mut first_artifact_ref = None;
        for field in [
            "prompt", "content", "output", "stdout", "stderr", "detail", "message",
        ] {
            let Some(value) = object.get_mut(field) else {
                continue;
            };
            let Some(text) = value.as_str() else {
                continue;
            };
            if text.len() <= ROLE_RUN_EVENT_INLINE_STRING_LIMIT {
                continue;
            }

            let artifact_root =
                self.role_run_event_artifact_root(project_id, issue_id, attempt_id, role_run_id);
            let artifact_ref = self.save_role_run_event_artifact(
                &artifact_root,
                role_run_id,
                sequence,
                field,
                text,
            )?;
            let preview = truncate_utf8(text, ROLE_RUN_EVENT_INLINE_STRING_LIMIT);
            if first_artifact_ref.is_none() {
                first_artifact_ref = Some(artifact_ref.clone());
            }
            *value = serde_json::json!({
                "preview": preview,
                "artifact_ref": artifact_ref,
                "truncated": true
            });
        }

        let truncated = first_artifact_ref.is_some();
        Ok((payload, truncated, first_artifact_ref))
    }

    fn save_role_run_event_artifact(
        &self,
        root: &Path,
        role_run_id: &str,
        sequence: u64,
        field: &str,
        content: &str,
    ) -> Result<String, ProductStoreError> {
        validate_relative_id(role_run_id)?;
        validate_relative_id(field)?;
        fs::create_dir_all(root).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", root.display()))
        })?;
        let file_name = format!("{sequence:04}_{field}.txt");
        let path = root.join(&file_name);
        fs::write(&path, content)
            .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;
        let artifact_ref = format!("artifacts/role-run-events/{role_run_id}/{file_name}");
        validate_relative_artifact_ref(&artifact_ref)?;
        Ok(artifact_ref)
    }

    fn role_run_path(&self, project_id: &str, issue_id: &str, run: &CodingRoleRun) -> PathBuf {
        self.role_runs_root(project_id, issue_id, &run.attempt_id)
            .join(format!("{}.json", run.id))
    }

    fn save_role_run(
        &self,
        project_id: &str,
        issue_id: &str,
        run: &CodingRoleRun,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&run.id)?;
        write_json(&self.role_run_path(project_id, issue_id, run), run)
    }

    fn get_role_run(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(role_run_id)?;
        read_json(
            &self
                .role_runs_root(project_id, issue_id, attempt_id)
                .join(format!("{role_run_id}.json")),
        )
    }

    fn provider_raw_output_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("provider-raw")
    }

    fn blocked_gates_root(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("blocked-gates")
    }

    fn quality_bypass_audits_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("quality-bypass-audits")
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

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
    let mut line =
        serde_json::to_vec(value).map_err(|error| ProductStoreError::Json(error.to_string()))?;
    line.push(b'\n');
    file.write_all(&line)
        .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;
    file.flush()
        .map_err(|error| ProductStoreError::Io(format!("flush {}: {error}", path.display())))?;
    Ok(())
}

fn read_jsonl_records<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?;
    let mut records = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        records.push(
            serde_json::from_str(line)
                .map_err(|error| ProductStoreError::Json(error.to_string()))?,
        );
    }
    Ok(records)
}

fn next_jsonl_sequence(path: &Path) -> Result<u64, ProductStoreError> {
    Ok(read_jsonl_records::<serde_json::Value>(path)?.len() as u64 + 1)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn coding_role_run_event_type_name(event_type: CodingRoleRunEventType) -> &'static str {
    match event_type {
        CodingRoleRunEventType::ProviderPrompt => "provider_prompt",
        CodingRoleRunEventType::ProviderStart => "provider_start",
        CodingRoleRunEventType::TextDelta => "text_delta",
        CodingRoleRunEventType::ExecutionEvent => "execution_event",
        CodingRoleRunEventType::ToolCall => "tool_call",
        CodingRoleRunEventType::ToolResult => "tool_result",
        CodingRoleRunEventType::StatusChanged => "status_changed",
        CodingRoleRunEventType::PermissionRequest => "permission_request",
        CodingRoleRunEventType::ChoiceRequest => "choice_request",
        CodingRoleRunEventType::MessageComplete => "message_complete",
        CodingRoleRunEventType::ProviderFailed => "provider_failed",
        CodingRoleRunEventType::Timeout => "timeout",
        CodingRoleRunEventType::Aborted => "aborted",
        CodingRoleRunEventType::PersistenceWarning => "persistence_warning",
    }
}

fn role_run_event_payload_text<'a>(event: &'a CodingRoleRunEvent, field: &str) -> Option<&'a str> {
    event.payload.get(field).and_then(|value| value.as_str())
}

fn role_run_event_payload_reason(event: &CodingRoleRunEvent) -> Option<&str> {
    role_run_event_payload_text(event, "reason_code")
        .or_else(|| role_run_event_payload_text(event, "message"))
}

fn role_run_event_artifact_refs(event: &CodingRoleRunEvent) -> Vec<String> {
    let mut artifact_refs = Vec::new();
    if let Some(artifact_ref) = event.artifact_ref.as_deref() {
        push_unique_artifact_ref(&mut artifact_refs, artifact_ref);
    }
    collect_payload_artifact_refs(&event.payload, &mut artifact_refs);
    artifact_refs
}

fn collect_payload_artifact_refs(value: &serde_json::Value, artifact_refs: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(artifact_ref) = object.get("artifact_ref").and_then(|value| value.as_str())
            {
                push_unique_artifact_ref(artifact_refs, artifact_ref);
            }
            for nested in object.values() {
                collect_payload_artifact_refs(nested, artifact_refs);
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                collect_payload_artifact_refs(nested, artifact_refs);
            }
        }
        _ => {}
    }
}

fn push_unique_artifact_ref(artifact_refs: &mut Vec<String>, artifact_ref: &str) {
    if !artifact_refs
        .iter()
        .any(|existing| existing == artifact_ref)
    {
        artifact_refs.push(artifact_ref.to_string());
    }
}

fn count_json_files(path: &Path) -> Result<usize, ProductStoreError> {
    Ok(json_file_paths(path)?.len())
}

fn next_text_file_sequence(path: &Path, purpose: &str) -> Result<usize, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(1);
    }
    let prefix = format!("{purpose}_");
    let mut count = 0;
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
        if !file_type.is_file() {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if file_name.starts_with(&prefix) && file_name.ends_with(".txt") {
            count += 1;
        }
    }
    Ok(count + 1)
}

fn coding_stage_dir_name(stage: &CodingExecutionStage) -> &'static str {
    match stage {
        CodingExecutionStage::PrepareContext => "prepare_context",
        CodingExecutionStage::WorktreePrepare => "worktree_prepare",
        CodingExecutionStage::Coding => "coding",
        CodingExecutionStage::Testing => "testing",
        CodingExecutionStage::CodeReview => "code_review",
        CodingExecutionStage::Rework => "rework",
        CodingExecutionStage::ReviewRequest => "review_request",
        CodingExecutionStage::InternalPrReview => "internal_pr_review",
        CodingExecutionStage::FinalConfirm => "final_confirm",
    }
}

fn matching_open_blocked_gate_path(
    gates_root: &Path,
    input: &CreateBlockedGateInput,
) -> Result<Option<PathBuf>, ProductStoreError> {
    for path in json_file_paths(gates_root)? {
        let record: BlockedGateRecord = read_json(&path)?;
        if record.status == BlockedGateStatus::Open
            && record.attempt_id == input.attempt_id
            && record.node_id.as_ref() == input.node_id.as_ref()
            && record.gate.stage.as_ref() == Some(&input.stage)
            && record.gate.reason_code.as_ref() == input.reason_code.as_ref()
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn merge_unique_strings(target: &mut Vec<String>, source: Vec<String>) {
    for value in source {
        if !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
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

fn remove_file_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

fn remove_dir_all_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::product::coding_models::{
        CodingGateAction, CodingGateActionType, CodingProviderRole, TestPlan, TestPlanRiskLevel,
        TestPlanStep, TestPlanTool,
    };
    use crate::product::models::ProviderName;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const WORK_ITEM_ID: &str = "work_item_0001";

    fn setup() -> (TempDir, CodingAttemptStore, CodingExecutionAttempt) {
        let tmp = TempDir::new().unwrap();
        let store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                },
                max_auto_rework: 2,
            })
            .unwrap();
        (tmp, store, attempt)
    }

    #[test]
    fn persists_test_plan_raw_output_and_blocked_gate() {
        let (_tmp, store, attempt) = setup();

        let raw_ref = store
            .save_provider_raw_output(
                &attempt.id,
                CodingExecutionStage::Testing,
                "plan_tests",
                "raw test plan output",
            )
            .unwrap();
        assert_eq!(raw_ref, "provider-raw/testing/plan_tests_0001.txt");

        let plan = TestPlan {
            id: "test_plan_0001".to_string(),
            attempt_id: attempt.id.clone(),
            role_run_id: None,
            run_no: None,
            summary: "unit tests".to_string(),
            context_warnings: Vec::new(),
            assumptions: Vec::new(),
            steps: vec![TestPlanStep {
                id: "unit".to_string(),
                title: "Unit tests".to_string(),
                intent: "verify unit behavior".to_string(),
                required: true,
                tool: TestPlanTool::RunCommand,
                risk_level: TestPlanRiskLevel::Low,
                command_or_tool_input: serde_json::json!({"command": ["true"]}),
                evidence_expectation: "exit 0".to_string(),
                related_requirements: Vec::new(),
                related_design_constraints: Vec::new(),
                related_work_item_tasks: Vec::new(),
            }],
            created_at: "2026-06-10T00:00:00Z".to_string(),
            raw_provider_output_ref: Some(raw_ref.clone()),
        };
        store.save_test_plan(&plan).unwrap();
        let plans = store
            .list_test_plans(PROJECT_ID, ISSUE_ID, &attempt.id)
            .unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(
            plans[0].raw_provider_output_ref.as_deref(),
            Some(raw_ref.as_str())
        );

        let gate = store
            .create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Testing,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::Tester),
                title: "Testing blocked".to_string(),
                description: "required step missing".to_string(),
                reason_code: Some("missing_required_steps".to_string()),
                evidence_refs: vec!["testing_report_0001.json".to_string()],
                raw_provider_output_ref: Some(raw_ref),
                available_actions: vec![CodingGateAction {
                    action_id: "retry_test_plan".to_string(),
                    label: "重试测试计划".to_string(),
                    action_type: CodingGateActionType::RetryTestPlan,
                }],
            })
            .unwrap();
        let open = store
            .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(
            open[0].reason_code.as_deref(),
            Some("missing_required_steps")
        );
        assert_eq!(open[0].evidence_refs, vec!["testing_report_0001.json"]);
        assert_eq!(
            open[0].available_actions[0].action_type,
            CodingGateActionType::RetryTestPlan
        );

        store
            .resolve_blocked_gate(PROJECT_ID, ISSUE_ID, &attempt.id, &gate.gate_id)
            .unwrap();
        let open = store
            .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
            .unwrap();
        assert!(open.is_empty());
    }

    #[test]
    fn blocked_gate_creation_is_idempotent_for_same_node_and_reason() {
        let (_tmp, store, attempt) = setup();
        let first = store
            .create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Testing,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::Tester),
                title: "Testing blocked".to_string(),
                description: "required step missing".to_string(),
                reason_code: Some("missing_required_steps".to_string()),
                evidence_refs: vec!["testing_report_0001.json".to_string()],
                raw_provider_output_ref: None,
                available_actions: vec![CodingGateAction {
                    action_id: "retry_test_plan".to_string(),
                    label: "重试测试计划".to_string(),
                    action_type: CodingGateActionType::RetryTestPlan,
                }],
            })
            .unwrap();

        let second = store
            .create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Testing,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::Tester),
                title: "Testing still blocked".to_string(),
                description: "required step still missing".to_string(),
                reason_code: Some("missing_required_steps".to_string()),
                evidence_refs: vec![
                    "testing_report_0001.json".to_string(),
                    "testing_report_0002.json".to_string(),
                ],
                raw_provider_output_ref: None,
                available_actions: vec![CodingGateAction {
                    action_id: "rerun_missing_steps".to_string(),
                    label: "补跑缺失步骤".to_string(),
                    action_type: CodingGateActionType::RerunMissingSteps,
                }],
            })
            .unwrap();

        assert_eq!(second.gate_id, first.gate_id);
        let open = store
            .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(
            open[0].evidence_refs,
            vec!["testing_report_0001.json", "testing_report_0002.json"]
        );
        assert_eq!(
            open[0].available_actions[0].action_type,
            CodingGateActionType::RerunMissingSteps
        );
    }
}
