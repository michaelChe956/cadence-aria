use chrono::Utc;

use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, WorkspaceSessionRecord,
};

use super::LifecycleStore;

impl LifecycleStore {
    fn human_gate_turn_path(
        &self,
        session: &WorkspaceSessionRecord,
        turn_id: &str,
    ) -> Result<std::path::PathBuf, ProductStoreError> {
        validate_relative_id(turn_id)?;
        Ok(self
            .paths
            .issue_lifecycle_root(&session.project_id, &session.issue_id)
            .join("workspace-sessions")
            .join(&session.id)
            .join("human-gate-turns")
            .join(format!("{turn_id}.json")))
    }

    fn validate_human_gate_turn(
        turn: &HumanGateTurn,
        session_id: &str,
        turn_id: &str,
    ) -> Result<(), ProductStoreError> {
        if turn.session_id != session_id || turn.turn_id != turn_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "human_gate_turn",
                id: turn_id.to_string(),
            });
        }
        if turn.budget_reserved != 1 {
            return Err(ProductStoreError::InvalidRecord {
                kind: "human_gate_turn",
                reason: "budget_reserved must be exactly 1".to_string(),
            });
        }
        if turn.status != HumanGateTurnStatus::Failed && turn.failure_class.is_some() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "human_gate_turn",
                reason: "failure_class is only valid for failed turns".to_string(),
            });
        }
        Ok(())
    }

    pub fn compare_and_reserve_human_gate_turn(
        &self,
        expected: &WorkspaceSessionRecord,
        turn: HumanGateTurn,
        reservation: HumanGateReservation,
    ) -> Result<(WorkspaceSessionRecord, HumanGateTurn), ProductStoreError> {
        validate_relative_id(&expected.id)?;
        validate_relative_id(&turn.turn_id)?;
        validate_relative_id(&turn.session_id)?;
        validate_relative_id(&turn.command_id)?;
        validate_relative_id(&reservation.command_id)?;
        validate_relative_id(&reservation.turn_id)?;
        if turn.session_id != expected.id
            || reservation.command_id != turn.command_id
            || reservation.turn_id != turn.turn_id
            || reservation.provider_start_idempotency_key.trim().is_empty()
            || turn.status != HumanGateTurnStatus::Reserved
            || turn.failure_class.is_some()
            || turn.budget_reserved != 1
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "human_gate_reservation",
                reason: "reservation identity or initial turn state is invalid".to_string(),
            });
        }

        let session_path = self.find_workspace_session_path(&expected.id)?;
        let turn_path = self.human_gate_turn_path(expected, &turn.turn_id)?;
        with_exclusive_lock(&session_path, || {
            let stored: WorkspaceSessionRecord = read_json(&session_path)?;
            if let Some(existing_reservation) = &stored.human_gate_reservation {
                if existing_reservation == &reservation {
                    if !super::path_exists(&turn_path)? {
                        write_json(&turn_path, &turn)?;
                        return Ok((stored, turn));
                    }
                    let existing_turn: HumanGateTurn = read_json(&turn_path)?;
                    Self::validate_human_gate_turn(&existing_turn, &stored.id, &turn.turn_id)?;
                    if existing_turn == turn {
                        return Ok((stored, existing_turn));
                    }
                }
                return Err(ProductStoreError::Conflict {
                    kind: "human_gate_reservation",
                    id: reservation.command_id.clone(),
                });
            }
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: expected.id.clone(),
                });
            }
            if stored.provider_start_ledger.iter().any(|entry| {
                entry.provider_start_idempotency_key == reservation.provider_start_idempotency_key
            }) {
                return Err(ProductStoreError::Conflict {
                    kind: "provider_start_ledger",
                    id: reservation.provider_start_idempotency_key.clone(),
                });
            }
            let snapshot = stored.human_gate_snapshot.as_ref().ok_or_else(|| {
                ProductStoreError::InvalidRecord {
                    kind: "human_gate_reservation",
                    reason: "human gate snapshot is required".to_string(),
                }
            })?;
            if snapshot.manual_repairs_remaining == 0 {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "human_gate_reservation",
                    reason: "human gate budget exhausted".to_string(),
                });
            }

            let original = stored.clone();
            let mut next = stored;
            next.human_gate_snapshot
                .as_mut()
                .expect("snapshot checked above")
                .manual_repairs_remaining -= 1;
            next.human_gate_reservation = Some(reservation.clone());
            next.provider_start_ledger.push(
                crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                    provider_start_idempotency_key: reservation.provider_start_idempotency_key,
                    started: true,
                },
            );
            next.updated_at = Utc::now().to_rfc3339();

            write_json(&session_path, &next)?;
            if let Err(error) = write_json(&turn_path, &turn) {
                let _ = write_json(&session_path, &original);
                return Err(error);
            }
            Ok((next, turn))
        })
    }

    pub fn get_human_gate_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<HumanGateTurn, ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(turn_id)?;
        let session = self.get_workspace_session(session_id)?;
        let path = self.human_gate_turn_path(&session, turn_id)?;
        let turn: HumanGateTurn = read_json(&path)?;
        Self::validate_human_gate_turn(&turn, session_id, turn_id)?;
        Ok(turn)
    }

    pub fn get_human_gate_turn_by_command_id(
        &self,
        session_id: &str,
        command_id: &str,
    ) -> Result<Option<HumanGateTurn>, ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(command_id)?;
        let session = self.get_workspace_session(session_id)?;
        let root = self
            .paths
            .issue_lifecycle_root(&session.project_id, &session.issue_id)
            .join("workspace-sessions")
            .join(&session.id)
            .join("human-gate-turns");
        if !super::path_exists(&root)? {
            return Ok(None);
        }
        let mut entries = std::fs::read_dir(&root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
        entries.sort_by_key(|entry| entry.file_name());
        let mut matching_turn = None;
        for entry in entries {
            let entry_path = entry.path();
            if entry_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(file_turn_id) = entry_path.file_stem().and_then(|stem| stem.to_str()) else {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "human_gate_turn",
                    id: session_id.to_string(),
                });
            };
            let turn: HumanGateTurn = read_json(&entry_path)?;
            Self::validate_human_gate_turn(&turn, session_id, file_turn_id)?;
            if turn.command_id == command_id {
                if matching_turn.is_some() {
                    return Err(ProductStoreError::Conflict {
                        kind: "human_gate_turn_command",
                        id: command_id.to_string(),
                    });
                }
                matching_turn = Some(turn);
            }
        }
        Ok(matching_turn)
    }

    pub fn update_human_gate_turn(
        &self,
        expected: &WorkspaceSessionRecord,
        turn: HumanGateTurn,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(&expected.id)?;
        validate_relative_id(&turn.turn_id)?;
        Self::validate_human_gate_turn(&turn, &expected.id, &turn.turn_id)?;
        let session_path = self.find_workspace_session_path(&expected.id)?;
        let turn_path = self.human_gate_turn_path(expected, &turn.turn_id)?;
        with_exclusive_lock(&session_path, || {
            let stored: WorkspaceSessionRecord = read_json(&session_path)?;
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: expected.id.clone(),
                });
            }
            let existing: HumanGateTurn = read_json(&turn_path)?;
            Self::validate_human_gate_turn(&existing, &stored.id, &turn.turn_id)?;
            if existing.command_id != turn.command_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "human_gate_turn",
                    id: turn.turn_id.clone(),
                });
            }
            if existing.session_id != turn.session_id
                || existing.feedback_text != turn.feedback_text
                || existing.attempt_no != turn.attempt_no
                || existing.budget_reserved != turn.budget_reserved
                || existing.created_at != turn.created_at
            {
                return Err(ProductStoreError::Conflict {
                    kind: "human_gate_turn",
                    id: turn.turn_id.clone(),
                });
            }
            if existing == turn {
                return Ok(stored);
            }
            let valid_progression = matches!(
                (&existing.status, &turn.status),
                (HumanGateTurnStatus::Reserved, HumanGateTurnStatus::Running)
                    | (
                        HumanGateTurnStatus::Reserved,
                        HumanGateTurnStatus::Completed
                    )
                    | (HumanGateTurnStatus::Reserved, HumanGateTurnStatus::Failed)
                    | (HumanGateTurnStatus::Running, HumanGateTurnStatus::Completed)
                    | (HumanGateTurnStatus::Running, HumanGateTurnStatus::Failed)
            );
            if !valid_progression {
                return Err(ProductStoreError::Conflict {
                    kind: "human_gate_turn",
                    id: turn.turn_id.clone(),
                });
            }
            write_json(&turn_path, &turn)?;
            Ok(stored)
        })
    }
}
