use chrono::Utc;

use crate::product::coding_models::{
    CodingAgentRole, CodingChatEntry, CodingContextNote, CodingEntryType, CodingExecutionAttempt,
    CodingReworkInstruction,
};
use crate::product::id::next_sequential_id_in_directory;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

impl super::CodingAttemptStore {
    pub fn create_context_note(
        &self,
        attempt: &CodingExecutionAttempt,
        content: String,
    ) -> Result<CodingContextNote, ProductStoreError> {
        self.validate_scoped_attempt_record(
            attempt,
            &attempt.id,
            "coding_context_note",
            &attempt.id,
        )?;
        let notes_root = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("context-notes");
        let id = next_sequential_id_in_directory("coding_context_note", &notes_root).map_err(
            |error| ProductStoreError::Io(format!("read {}: {error}", notes_root.display())),
        )?;
        let note = CodingContextNote {
            id: id.clone(),
            attempt_id: attempt.id.clone(),
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
        super::list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("context-notes"),
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

    pub fn save_chat_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        entry: &CodingChatEntry,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&entry.id)?;
        self.validate_scoped_attempt_record(
            attempt,
            &entry.attempt_id,
            "coding_chat_entry",
            &entry.id,
        )?;
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
        let mut entries: Vec<CodingChatEntry> = super::list_json_records(
            &self
                .attempt_dir(project_id, issue_id, attempt_id)
                .join("chat-entries"),
        )?;
        entries.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(entries)
    }

    /// F-16/§3#5 死因可考：runner 死亡转人工恢复时，把稳定 reason 码与原始
    /// 错误串作为 System/SystemEvent 尾帧落入 attempt chat-entries——WS 瞬时帧
    /// 与服务 tty 均不可回收，durable 层此前零痕迹。
    pub fn append_manual_recovery_diagnostic(
        &self,
        attempt: &CodingExecutionAttempt,
        reason_stable_code: &str,
        failure_detail: &str,
    ) -> Result<CodingChatEntry, ProductStoreError> {
        let entries_root = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("chat-entries");
        let id = next_sequential_id_in_directory("coding_chat_entry", &entries_root).map_err(
            |error| ProductStoreError::Io(format!("read {}: {error}", entries_root.display())),
        )?;
        let entry = CodingChatEntry {
            id,
            attempt_id: attempt.id.clone(),
            node_id: None,
            role: CodingAgentRole::System,
            entry_type: CodingEntryType::SystemEvent {
                event_type: "manual_recovery_transition".to_string(),
                message: format!("{reason_stable_code}: {failure_detail}"),
            },
            content: None,
            metadata: Some(serde_json::json!({
                "manual_recovery_reason": reason_stable_code,
                "failure_detail": failure_detail,
            })),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_chat_entry(attempt, &entry)?;
        Ok(entry)
    }

    pub fn save_rework_instruction(
        &self,
        attempt: &CodingExecutionAttempt,
        instruction: &CodingReworkInstruction,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&instruction.id)?;
        self.validate_scoped_attempt_record(
            attempt,
            &instruction.attempt_id,
            "coding_rework_instruction",
            &instruction.id,
        )?;
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
        super::list_json_records(&self.rework_instructions_root(project_id, issue_id, attempt_id))
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
}
