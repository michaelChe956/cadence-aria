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

// ─── C2 Task 9（#2/#9，REQ-CVT-01/02/05）：计划命令与实际命令并列证据 ───

/// 计划命令与实际命令并列证据（DTO）。actual_* 全部来自 role-run JSONL 的
/// ExecutionEvent；缺失字段保持 None（前端显示"未记录"），不推断补写。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VerificationCommandEvidence {
    pub check_id: String,
    /// 计划合同字面命令。
    pub planned_command: Option<String>,
    /// 无命令 check 的人工说明。
    pub planned_manual_instruction: Option<String>,
    /// role-run JSONL 派生的实际执行命令；None → 前端显示"未记录"。
    pub actual_command: Option<String>,
    pub actual_cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub test_execution_count: Option<u64>,
    pub environment_summary: Option<String>,
    /// "实际执行命令与计划不一致"（同一可执行族下参数差异也算）。
    pub mismatch: bool,
}

/// C2 Task 9：三类文案 reason code（验证类等待面与前端共用）。
pub const VERIFICATION_EVIDENCE_ACTUAL_COMMAND_MISMATCH: &str = "actual_command_mismatch";
pub const VERIFICATION_EVIDENCE_PLAN_UNDECLARED: &str = "plan_undeclared_path_or_command";
pub const VERIFICATION_EVIDENCE_PLAN_PATH_UNEXECUTABLE: &str = "plan_path_unexecutable";

/// 三类文案（全新中文口径）：不一致→指向重跑原计划命令；计划未声明→
/// 指向计划反馈；计划路径不可执行→指向计划修订或验证处理。文案不建议
/// 升级运行时版本、泛化重试或"忽略 finding"。
pub fn verification_evidence_copy(reason_code: &str) -> Option<&'static str> {
    match reason_code {
        VERIFICATION_EVIDENCE_ACTUAL_COMMAND_MISMATCH => Some(
            "实际执行命令与计划不一致：coder 未按计划命令执行。可在并列证据下方点击\"重跑原计划命令\"，以计划合同字面命令发起返修。",
        ),
        VERIFICATION_EVIDENCE_PLAN_UNDECLARED => Some(
            "计划未声明该路径或命令：coder 执行了计划合同之外的路径或命令。请通过计划反馈修订计划，使合同声明与实际执行面一致。",
        ),
        VERIFICATION_EVIDENCE_PLAN_PATH_UNEXECUTABLE => Some(
            "计划路径不可执行：计划命令引用的路径在仓库中不存在或不可执行。请发起计划修订，或转入验证处理记录受限豁免。",
        ),
        _ => None,
    }
}

impl super::CodingAttemptStore {
    /// 派生并列证据：planned 来自计划合同 VerificationCheck；actual 取
    /// attempt 全部 role-run JSONL 中与计划命令同可执行族（首 token 相同）
    /// 的最新一条 ExecutionEvent；无匹配 → actual 为 None（"未记录"），
    /// 不推断补写、不归类计划缺陷。
    pub fn verification_command_evidence(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        check: &crate::product::work_item_contract::VerificationCheck,
    ) -> Result<VerificationCommandEvidence, ProductStoreError> {
        let mut evidence = VerificationCommandEvidence {
            check_id: check.check_id.clone(),
            planned_command: check.command.clone(),
            planned_manual_instruction: check.manual_instruction.clone(),
            actual_command: None,
            actual_cwd: None,
            exit_code: None,
            test_execution_count: None,
            environment_summary: None,
            mismatch: false,
        };
        let Some(planned_command) = check.command.as_deref() else {
            return Ok(evidence);
        };
        let planned_head = first_command_token(planned_command);
        let mut matched: Option<(String, serde_json::Value)> = None;
        for role_run in self.list_role_runs(project_id, issue_id, attempt_id)? {
            for event in
                self.list_role_run_events(project_id, issue_id, attempt_id, &role_run.id)?
            {
                if event.event_type != CodingRoleRunEventType::ExecutionEvent {
                    continue;
                }
                let Some(command) = event
                    .payload
                    .get("command")
                    .and_then(|value| value.as_str())
                else {
                    continue;
                };
                if first_command_token(command) != planned_head {
                    continue;
                }
                // list_role_run_events 按 sequence 升序，role_runs 按 id 升序：
                // 后写者覆盖，最终保留同族最新一条。
                matched = Some((command.to_string(), event.payload.clone()));
            }
        }
        if let Some((command, payload)) = matched {
            evidence.actual_command = Some(command.clone());
            evidence.actual_cwd = payload
                .get("cwd")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            evidence.exit_code = payload
                .get("exit_code")
                .and_then(|value| value.as_i64())
                .map(|code| code as i32);
            evidence.mismatch = command != planned_command;
        }
        Ok(evidence)
    }
}

fn first_command_token(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or_default()
}
