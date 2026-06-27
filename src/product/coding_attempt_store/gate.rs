use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_attempt_store::{
    CreateBlockedGateInput, CreateChoiceGateInput, CreateQualityBypassAuditInput,
};
use crate::product::coding_models::{
    CodingChoiceGate, CodingChoiceGateResponse, CodingChoiceGateStatus, CodingExecutionStage,
    CodingGateKind, CodingGateRequired, CodingProviderRole, CodingRoleProviderConfigSnapshot,
    CodingStageGateState, CodingStageGateStatus, QualityGateBypassAudit,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

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

impl super::CodingAttemptStore {
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
            super::merge_unique_strings(&mut record.gate.evidence_refs, input.evidence_refs);
            record.updated_at = Utc::now().to_rfc3339();
            write_json(&existing_path, &record)?;
            return Ok(record.gate);
        }
        let gate_count = super::count_json_files(&gates_root)?
            + super::count_json_files(&gates_root.join("resolved"))?;
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
            super::list_json_records(&self.blocked_gates_root(project_id, issue_id, attempt_id))?;
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
        if !super::path_is_regular_file(&path)? {
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
        super::remove_file_if_exists(&path)?;
        Ok(gate)
    }

    pub fn create_choice_gate(
        &self,
        input: CreateChoiceGateInput,
    ) -> Result<CodingChoiceGate, ProductStoreError> {
        validate_relative_id(&input.attempt_id)?;
        if let Some(node_id) = &input.node_id {
            validate_relative_id(node_id)?;
        }
        let attempt = self.find_attempt_by_id(&input.attempt_id)?;
        let gates_root =
            self.choice_gates_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        if let Some(existing_path) = matching_open_choice_gate_path(&gates_root, &input.choice_id)?
        {
            let mut gate: CodingChoiceGate = read_json(&existing_path)?;
            gate.stage = input.stage;
            gate.node_id = input.node_id;
            gate.role = input.role;
            gate.provider = input.provider;
            gate.source = input.source;
            gate.prompt = input.prompt;
            gate.options = input.options;
            gate.allow_multiple = input.allow_multiple;
            gate.allow_free_text = input.allow_free_text;
            gate.updated_at = Utc::now().to_rfc3339();
            write_json(&existing_path, &gate)?;
            return Ok(gate);
        }

        let gate_count = super::count_json_files(&gates_root)?
            + super::count_json_files(&gates_root.join("resolved"))?;
        let gate_id = next_sequential_id("coding_choice_gate", gate_count);
        let now = Utc::now().to_rfc3339();
        let gate = CodingChoiceGate {
            gate_id: gate_id.clone(),
            choice_id: input.choice_id,
            attempt_id: attempt.id,
            node_id: input.node_id,
            stage: input.stage,
            role: input.role,
            provider: input.provider,
            source: input.source,
            prompt: input.prompt,
            options: input.options,
            allow_multiple: input.allow_multiple,
            allow_free_text: input.allow_free_text,
            status: CodingChoiceGateStatus::Open,
            response: None,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(&gates_root.join(format!("{gate_id}.json")), &gate)?;
        Ok(gate)
    }

    pub fn list_open_choice_gates(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingChoiceGate>, ProductStoreError> {
        let mut gates: Vec<CodingChoiceGate> =
            super::list_json_records(&self.choice_gates_root(project_id, issue_id, attempt_id))?;
        gates.retain(|gate| gate.status == CodingChoiceGateStatus::Open);
        Ok(gates)
    }

    pub fn resolve_choice_gate(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        choice_id: &str,
        selected_option_ids: Vec<String>,
        free_text: Option<String>,
    ) -> Result<CodingChoiceGate, ProductStoreError> {
        let gates_root = self.choice_gates_root(project_id, issue_id, attempt_id);
        let Some(path) = matching_open_choice_gate_path(&gates_root, choice_id)? else {
            return Err(ProductStoreError::NotFound {
                kind: "coding_choice_gate",
                id: choice_id.to_string(),
            });
        };

        let mut gate: CodingChoiceGate = read_json(&path)?;
        gate.status = CodingChoiceGateStatus::Resolved;
        gate.response = Some(CodingChoiceGateResponse {
            selected_option_ids,
            free_text,
            responded_at: Utc::now().to_rfc3339(),
        });
        gate.updated_at = Utc::now().to_rfc3339();
        write_json(
            &gates_root
                .join("resolved")
                .join(format!("{}.json", gate.gate_id)),
            &gate,
        )?;
        super::remove_file_if_exists(&path)?;
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
        let id = next_sequential_id("quality_bypass_audit", super::count_json_files(&root)?);
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
        super::list_json_records(&self.quality_bypass_audits_root(project_id, issue_id, attempt_id))
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
        let gate_id =
            next_sequential_id("coding_stage_gate", super::count_json_files(&gates_root)?);
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
        super::list_json_records(
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
        if !super::path_is_regular_file(&path)? {
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
        if !super::path_is_regular_file(&path)? {
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
}

fn matching_open_blocked_gate_path(
    gates_root: &Path,
    input: &CreateBlockedGateInput,
) -> Result<Option<std::path::PathBuf>, ProductStoreError> {
    for path in super::json_file_paths(gates_root)? {
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

fn matching_open_choice_gate_path(
    gates_root: &Path,
    choice_id: &str,
) -> Result<Option<std::path::PathBuf>, ProductStoreError> {
    for path in super::json_file_paths(gates_root)? {
        let gate: CodingChoiceGate = read_json(&path)?;
        if gate.status == CodingChoiceGateStatus::Open && gate.choice_id == choice_id {
            return Ok(Some(path));
        }
    }
    Ok(None)
}
