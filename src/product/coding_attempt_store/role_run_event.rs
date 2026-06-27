use std::fs;
use std::path::Path;
use std::sync::Mutex;

use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingRoleRun, CodingRoleRunEvent, CodingRoleRunEventType,
};
use crate::product::json_store::{
    ProductStoreError, validate_relative_artifact_ref, validate_relative_id,
};

static ROLE_RUN_EVENT_LOG_MUTEX: Mutex<()> = Mutex::new(());

impl super::CodingAttemptStore {
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
        let sequence = super::next_jsonl_sequence(&path)?;
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
        super::append_jsonl(&path, &event)?;
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
        let mut events: Vec<CodingRoleRunEvent> = super::read_jsonl_records(&path)?;
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
                super::coding_role_run_event_type_name(event.event_type)
            ));
            if let Some(reason) = super::role_run_event_payload_reason_summary(event) {
                lines.push(format!("terminal_reason: {reason}"));
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
        let recent_events = events
            .iter()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        let mut event_artifact_refs = Vec::new();
        for event in &recent_events {
            for artifact_ref in super::role_run_event_artifact_refs(event) {
                super::push_unique_artifact_ref(&mut event_artifact_refs, &artifact_ref);
            }
        }
        if !event_artifact_refs.is_empty() {
            lines.push(format!(
                "event_artifact_refs: {}",
                event_artifact_refs.join(", ")
            ));
        }
        lines.push("recent_events:".to_string());
        for event in recent_events {
            lines.push(format!(
                "- #{} {} title={} status={} detail={}",
                event.sequence,
                super::coding_role_run_event_type_name(event.event_type),
                super::role_run_event_payload_summary_text(event, "title"),
                super::role_run_event_payload_summary_text(event, "status"),
                super::role_run_event_payload_summary_text(event, "detail")
            ));
        }
        let summary =
            super::truncate_utf8(&lines.join("\n"), super::ROLE_RUN_RETRY_DIAGNOSTIC_LIMIT);
        Ok(Some(summary))
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
            if text.len() <= super::ROLE_RUN_EVENT_INLINE_STRING_LIMIT {
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
            let preview = super::truncate_utf8(text, super::ROLE_RUN_EVENT_INLINE_STRING_LIMIT);
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
}
