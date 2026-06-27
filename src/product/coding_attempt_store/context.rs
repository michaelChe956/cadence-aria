use chrono::Utc;

use crate::product::coding_models::{
    AnalystDecisionRecord, CodingChatEntry, CodingContextNote, CodingReworkInstruction,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

impl super::CodingAttemptStore {
    pub fn create_context_note(
        &self,
        attempt_id: &str,
        content: String,
    ) -> Result<CodingContextNote, ProductStoreError> {
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let notes_root = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("context-notes");
        let id = next_sequential_id("coding_context_note", super::count_json_files(&notes_root)?);
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
        super::list_json_records(&self.analyst_decisions_root(project_id, issue_id, attempt_id))
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
}
