use chrono::Utc;

use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, StructuralClaimOutcome,
    StructuralGovernanceState, StructuralGrantPhase, StructuralSettlement,
    StructuralSettlementError, UserBudgetHold, UserBudgetReceipt, WorkspaceSessionRecord,
};

use crate::product::workspace_engine::HUMAN_GATE_PROVIDER_MAX_ATTEMPTS;

use super::LifecycleStore;

impl LifecycleStore {
    pub(super) fn human_gate_turn_path(
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

    pub(super) fn validate_human_gate_turn(
        turn: &HumanGateTurn,
        session_id: &str,
        turn_id: &str,
    ) -> Result<(), ProductStoreError> {
        if turn.attempt_no == 0 || turn.attempt_no > HUMAN_GATE_PROVIDER_MAX_ATTEMPTS {
            return Err(ProductStoreError::InvalidRecord {
                kind: "human_gate_turn",
                reason: format!(
                    "attempt_no must be between 1 and {HUMAN_GATE_PROVIDER_MAX_ATTEMPTS}"
                ),
            });
        }
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
        if !turn.source_hash.is_empty()
            && !crate::product::work_item_plan_policy::is_lowercase_sha256_hex(&turn.source_hash)
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "human_gate_turn",
                reason: "source_hash must be a lowercase SHA-256 hex digest".to_string(),
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

        // D1.3b（fix-cascade-deletion plan §0.4 human_gate 行）：最外层 scope
        // 准入——expected 记录域在 session flock 外先取 scope；锁内 CAS/预算/
        // nonce 与失败回滚零变化，回滚不重取锁。
        let scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            self.compare_and_reserve_human_gate_turn_in_admission(expected, turn, reservation)
        })
    }

    /// `compare_and_reserve_human_gate_turn` 的已准入内层（D1.3b：外层已持
    /// scope 共享准入/已按删除事实拒；session flock 内原 CAS 语义不变）。
    fn compare_and_reserve_human_gate_turn_in_admission(
        &self,
        expected: &WorkspaceSessionRecord,
        turn: HumanGateTurn,
        reservation: HumanGateReservation,
    ) -> Result<(WorkspaceSessionRecord, HumanGateTurn), ProductStoreError> {
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
                    return Err(ProductStoreError::Conflict {
                        kind: "human_gate_reservation",
                        id: reservation.command_id.clone(),
                    });
                }
                let previous_turn_path =
                    self.human_gate_turn_path(&stored, &existing_reservation.turn_id)?;
                if !super::path_exists(&previous_turn_path)? {
                    return Err(ProductStoreError::Conflict {
                        kind: "human_gate_reservation",
                        id: reservation.command_id.clone(),
                    });
                }
                let previous_turn: HumanGateTurn = read_json(&previous_turn_path)?;
                Self::validate_human_gate_turn(
                    &previous_turn,
                    &stored.id,
                    &existing_reservation.turn_id,
                )?;
                if matches!(
                    previous_turn.status,
                    HumanGateTurnStatus::Reserved | HumanGateTurnStatus::Running
                ) {
                    return Err(ProductStoreError::Conflict {
                        kind: "human_gate_reservation",
                        id: reservation.command_id.clone(),
                    });
                }
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
            {
                let snapshot = next
                    .human_gate_snapshot
                    .as_mut()
                    .expect("snapshot checked above");
                snapshot.manual_repairs_remaining -= 1;
                // C2（REQ-CG-02/REQ-HGC-01）：被接受为修订 turn 的反馈与预算
                // 预留同 CAS 原子计数；旧会话缺 gate-local 事实（None）保持
                // unknown，不补计数不改历史。
                if let Some(turns) = snapshot.accepted_feedback_turns {
                    snapshot.accepted_feedback_turns = Some(turns.saturating_add(1));
                }
            }
            // SG4a（REQ-CG-02）：接受真实反馈＝以 durable hold 立即占用一轮
            // 可用预算（不标已交付）；与预算扣减、provider-start ledger 同
            // session CAS 原子落盘。按 turn 幂等：重放路径不重复占位。
            {
                let hold = UserBudgetHold {
                    turn_id: turn.turn_id.clone(),
                    command_id: turn.command_id.clone(),
                    source_hash: turn.source_hash.clone(),
                    held_at: Utc::now().to_rfc3339(),
                };
                let state = next
                    .structural_governance_state
                    .get_or_insert_with(StructuralGovernanceState::default);
                if !state
                    .user_holds
                    .iter()
                    .any(|held| held.turn_id == hold.turn_id)
                {
                    state.user_holds.push(hold);
                }
                // SG6a.3（复审④/REQ-SG-05「…grant phase SHALL 同 session
                // CAS 绑定」）：turn reserve 是结构授权窗口（current_grant）
                // 的唯一生产 producer——接受真实反馈的同一 CAS 原子创建
                // 绑定本 turn/command 的 grant（attempt 0，机器 claim 推
                // 进）。前一窗口绑定的 turn 必已终态（reservation 单飞已
                // 拒非终态前驱），轮换不清理 history（receipt/checkpoint/
                // resolved facts 保留）；已绑本 turn 的既有 grant（测试
                // 注入/claim 进度）原样保留，不被 reserve 重放覆盖。
                if state
                    .current_grant
                    .as_ref()
                    .and_then(|grant| grant.turn_id.as_deref())
                    != Some(turn.turn_id.as_str())
                {
                    state.current_grant = Some(structural_revision_grant_for_turn(&turn));
                }
            }
            next.human_gate_reservation = Some(reservation.clone());
            next.provider_start_ledger.push(
                crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                    provider_start_idempotency_key: reservation.provider_start_idempotency_key,
                    started: true,
                    provider: None,
                    started_at: None,
                    provider_attempt_key: None,
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

    pub fn list_human_gate_turns(
        &self,
        session_id: &str,
    ) -> Result<Vec<HumanGateTurn>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session = self.get_workspace_session(session_id)?;
        let root = self
            .paths
            .issue_lifecycle_root(&session.project_id, &session.issue_id)
            .join("workspace-sessions")
            .join(&session.id)
            .join("human-gate-turns");
        if !super::path_exists(&root)? {
            return Ok(Vec::new());
        }
        let mut entries = std::fs::read_dir(&root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
        entries.sort_by_key(|entry| entry.file_name());
        let mut turns = Vec::new();
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(turn_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "human_gate_turn",
                    id: session_id.to_string(),
                });
            };
            let turn: HumanGateTurn = read_json(&path)?;
            Self::validate_human_gate_turn(&turn, session_id, turn_id)?;
            turns.push(turn);
        }
        Ok(turns)
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

        // D1.3b（fix-cascade-deletion plan §0.4 human_gate 行）：最外层 scope
        // 准入——expected 记录域在 session flock 外先取 scope；锁内 CAS/回滚
        // 零变化，回滚不重取锁。
        let scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            self.update_human_gate_turn_in_admission(expected, turn)
        })
    }

    /// `update_human_gate_turn` 的已准入内层（D1.3b：外层已持 scope 共享准入/
    /// 已按删除事实拒；session flock 内原 CAS 语义不变）。
    fn update_human_gate_turn_in_admission(
        &self,
        expected: &WorkspaceSessionRecord,
        turn: HumanGateTurn,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
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
                || existing.budget_reserved != turn.budget_reserved
                || existing.source_hash != turn.source_hash
                || existing.created_at != turn.created_at
            {
                return Err(ProductStoreError::Conflict {
                    kind: "human_gate_turn",
                    id: turn.turn_id.clone(),
                });
            }
            if existing.attempt_no != turn.attempt_no
                && !(existing.status == HumanGateTurnStatus::Running
                    && turn.status == HumanGateTurnStatus::Running
                    && turn.attempt_no == existing.attempt_no.saturating_add(1))
            {
                return Err(ProductStoreError::Conflict {
                    kind: "human_gate_turn",
                    id: turn.turn_id.clone(),
                });
            }
            if existing == turn {
                return Ok(stored);
            }
            let valid_status_progression = matches!(
                (&existing.status, &turn.status),
                (HumanGateTurnStatus::Reserved, HumanGateTurnStatus::Running)
                    | (
                        HumanGateTurnStatus::Reserved,
                        HumanGateTurnStatus::Completed
                    )
                    | (HumanGateTurnStatus::Reserved, HumanGateTurnStatus::Failed)
                    | (HumanGateTurnStatus::Running, HumanGateTurnStatus::Completed)
                    | (HumanGateTurnStatus::Running, HumanGateTurnStatus::Failed)
            ) || (existing.status == HumanGateTurnStatus::Running
                && turn.status == HumanGateTurnStatus::Running
                && turn.attempt_no == existing.attempt_no.saturating_add(1));
            if !valid_status_progression {
                return Err(ProductStoreError::Conflict {
                    kind: "human_gate_turn",
                    id: turn.turn_id.clone(),
                });
            }
            let original = stored.clone();
            let mut next = stored;
            if turn.attempt_no != existing.attempt_no {
                let provider_start_idempotency_key =
                    format!("human_gate:{}:attempt:{}", turn.turn_id, turn.attempt_no);
                if next.provider_start_ledger.iter().any(|entry| {
                    entry.provider_start_idempotency_key == provider_start_idempotency_key
                }) {
                    return Err(ProductStoreError::Conflict {
                        kind: "provider_start_ledger",
                        id: provider_start_idempotency_key,
                    });
                }
                next.provider_start_ledger.push(
                    crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                        provider_start_idempotency_key,
                        started: true,
                        provider: None,
                        started_at: None,
                        provider_attempt_key: None,
                    },
                );
                next.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &next)?;
            }
            if let Err(error) = write_json(&turn_path, &turn) {
                if turn.attempt_no != existing.attempt_no {
                    let _ = write_json(&session_path, &original);
                }
                return Err(error);
            }
            Ok(next)
        })
    }

    /// SG4a（plan v1.1 §0.5 / REQ-SG-05）：在活动 grant 下为同 turn 分配
    /// 结构修复物理 attempt。前置：聚合状态与 grant 存在且绑定同
    /// turn/command、gate turn 存在且 Running；物理 attempt 上限沿
    /// `HUMAN_GATE_PROVIDER_MAX_ATTEMPTS`（含原调用最多 2，L0/L1 共用同
    /// 一 turn attempt key，无第 3 次）。claim 与真实 start 分账：只推进
    /// grant phase/attempt 并绑定 prompt/instruction/失败 source，不写
    /// provider_start_ledger——返回的 `provider_start_key` 由真实启动路径
    /// 登记。同 prompt 重复 claim → `Replayed`（恰一次，不加 attempt）；
    /// 上限耗尽 → `Exhausted`（零写零新授权）；无状态/无 grant/无 turn
    /// 绑定/BlockedByLcRecovery → `Waiting`（旧记录零回填）。
    pub(crate) fn claim_structural_attempt(
        &self,
        expected: &WorkspaceSessionRecord,
        turn_id: &str,
        instruction: &crate::product::workspace_engine::contract_prerevision::StructuralRepairInstruction,
        prompt_ref: &str,
        prompt_digest: &str,
    ) -> Result<StructuralClaimOutcome, ProductStoreError> {
        validate_relative_id(&expected.id)?;
        validate_relative_id(turn_id)?;
        if instruction.instruction_digest.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "structural_repair_instruction",
                reason: "instruction_digest must not be empty".to_string(),
            });
        }
        if prompt_ref.trim().is_empty() || prompt_digest.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "structural_repair_instruction",
                reason: "prompt ref/digest must not be empty".to_string(),
            });
        }

        // D1.3b（fix-cascade-deletion plan §0.4）：最外层 scope 准入——expected
        // 记录域在 session flock 外先取 scope；锁内 grant/attempt 语义零变化。
        let scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            self.claim_structural_attempt_in_admission(
                expected,
                turn_id,
                instruction,
                prompt_ref,
                prompt_digest,
            )
        })
    }

    /// `claim_structural_attempt` 的已准入内层（D1.3b：外层已持 scope 共享
    /// 准入/已按删除事实拒；session flock 内原语义不变）。
    fn claim_structural_attempt_in_admission(
        &self,
        expected: &WorkspaceSessionRecord,
        turn_id: &str,
        instruction: &crate::product::workspace_engine::contract_prerevision::StructuralRepairInstruction,
        prompt_ref: &str,
        prompt_digest: &str,
    ) -> Result<StructuralClaimOutcome, ProductStoreError> {
        let session_path = self.find_workspace_session_path(&expected.id)?;
        let turn_path = self.human_gate_turn_path(expected, turn_id)?;
        with_exclusive_lock(&session_path, || {
            let stored: WorkspaceSessionRecord = read_json(&session_path)?;
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: expected.id.clone(),
                });
            }
            // 旧记录零回填：无聚合状态/无 grant → Waiting，不补授权。
            let Some(state) = stored.structural_governance_state.as_ref() else {
                return Ok(StructuralClaimOutcome::Waiting {
                    reason: "structural governance state missing".to_string(),
                });
            };
            let Some(grant) = state.current_grant.as_ref() else {
                return Ok(StructuralClaimOutcome::Waiting {
                    reason: "no active structural revision grant".to_string(),
                });
            };
            let Some(grant_turn_id) = grant.turn_id.as_deref() else {
                return Ok(StructuralClaimOutcome::Waiting {
                    reason: "structural grant has no bound turn".to_string(),
                });
            };
            if grant_turn_id != turn_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "structural_revision_grant",
                    id: turn_id.to_string(),
                });
            }
            if let Some(phase) = grant.phase {
                match phase {
                    // anchor waiting 优先于同 turn 续跑（设计 §7.3）。
                    StructuralGrantPhase::BlockedByLcRecovery => {
                        return Ok(StructuralClaimOutcome::Waiting {
                            reason: "blocked by lc recovery".to_string(),
                        });
                    }
                    // 评审/终态/业务门阶段无新 author attempt。
                    StructuralGrantPhase::Reviewing
                    | StructuralGrantPhase::Resolved
                    | StructuralGrantPhase::SystemFault
                    | StructuralGrantPhase::BusinessGate => {
                        return Err(ProductStoreError::Conflict {
                            kind: "structural_revision_grant",
                            id: grant.grant_id.clone(),
                        });
                    }
                    _ => {}
                }
            }
            let turn: HumanGateTurn = read_json(&turn_path)?;
            Self::validate_human_gate_turn(&turn, &stored.id, turn_id)?;
            if let Some(grant_command_id) = grant.command_id.as_deref()
                && grant_command_id != turn.command_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "structural_revision_grant",
                    id: turn.command_id.clone(),
                });
            }
            // 重复 claim（同 prompt 的当前 attempt）恰一次：不加 attempt、
            // 零写（REQ-SG-05：Attempt2Claimed 续同 key）。
            if grant.attempt_no >= 1
                && grant.phase == Some(StructuralGrantPhase::Attempt2Claimed)
                && grant.prompt_ref.as_deref() == Some(prompt_ref)
                && grant.prompt_digest.as_deref() == Some(prompt_digest)
            {
                return Ok(StructuralClaimOutcome::Replayed);
            }
            // attempt 推进仅 Running（L0 恢复/终态 turn 无新 claim）。
            if turn.status != HumanGateTurnStatus::Running {
                return Err(ProductStoreError::Conflict {
                    kind: "human_gate_turn",
                    id: turn_id.to_string(),
                });
            }
            // 物理上限：L0（turn.attempt_no）与 L1（grant.attempt_no）共用
            // 同一 turn attempt key；已耗尽零写零新授权（无第 3 次）。
            let used = turn.attempt_no.max(grant.attempt_no);
            if used >= HUMAN_GATE_PROVIDER_MAX_ATTEMPTS {
                return Ok(StructuralClaimOutcome::Exhausted);
            }
            let next_attempt = used + 1;
            let provider_start_key = format!("human_gate:{turn_id}:attempt:{next_attempt}");
            if stored
                .provider_start_ledger
                .iter()
                .any(|entry| entry.provider_start_idempotency_key == provider_start_key)
            {
                return Err(ProductStoreError::Conflict {
                    kind: "provider_start_ledger",
                    id: provider_start_key,
                });
            }
            let mut next = stored.clone();
            {
                let state = next
                    .structural_governance_state
                    .as_mut()
                    .expect("state checked above");
                let grant = state.current_grant.as_mut().expect("grant checked above");
                grant.attempt_no = next_attempt;
                grant.phase = Some(StructuralGrantPhase::Attempt2Claimed);
                grant.prompt_ref = Some(prompt_ref.to_string());
                grant.prompt_digest = Some(prompt_digest.to_string());
                grant.instruction_digest = Some(instruction.instruction_digest.clone());
                grant.failing_source_hash = Some(instruction.source_hash.clone());
                if !instruction.context_digest.trim().is_empty() {
                    state.current_context_digest = Some(instruction.context_digest.clone());
                }
            }
            next.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &next)?;
            Ok(StructuralClaimOutcome::Claimed {
                attempt_no: next_attempt,
                prompt_ref: prompt_ref.to_string(),
                prompt_digest: prompt_digest.to_string(),
                provider_start_key,
            })
        })
    }

    /// SG4a（plan v1.1 §0.5 / REQ-CG-02）：同 turn 的用户预算终态结算，
    /// 与 session CAS 同权威。终态互斥恰一次：同 receipt 重放返回当前
    /// 记录（零写）；不同 receipt（含已 deliver 后复评失败）→ `Conflict`
    /// 不退款。成功结算移除活动 hold、追加 receipt（released_structural
    /// 不实扣，快照可用预算同 CAS 恢复且不超过总额）。旧记录无聚合状态/
    /// 无活动 hold → `InvalidRecord`（不凭空造账）。
    pub fn settle_structural_user_budget(
        &self,
        expected: &WorkspaceSessionRecord,
        turn_id: &str,
        receipt: UserBudgetReceipt,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(&expected.id)?;
        validate_relative_id(turn_id)?;
        validate_relative_id(&receipt.turn_id)?;
        validate_relative_id(&receipt.command_id)?;
        if receipt.turn_id != turn_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "user_budget_receipt",
                id: turn_id.to_string(),
            });
        }

        // D1.3b（fix-cascade-deletion plan §0.4）：最外层 scope 准入——expected
        // 记录域在 session flock 外先取 scope；锁内结算/重放语义零变化。
        let scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            self.settle_structural_user_budget_in_admission(expected, turn_id, receipt)
        })
    }

    /// `settle_structural_user_budget` 的已准入内层（D1.3b：外层已持 scope
    /// 共享准入/已按删除事实拒；session flock 内原结算语义不变）。
    fn settle_structural_user_budget_in_admission(
        &self,
        expected: &WorkspaceSessionRecord,
        turn_id: &str,
        receipt: UserBudgetReceipt,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let session_path = self.find_workspace_session_path(&expected.id)?;
        with_exclusive_lock(&session_path, || {
            let stored: WorkspaceSessionRecord = read_json(&session_path)?;
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: expected.id.clone(),
                });
            }
            // 旧记录无聚合状态 → 不凭空造账。
            if stored.structural_governance_state.is_none() {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "structural_governance_state",
                    reason: "structural governance state missing".to_string(),
                });
            }
            let mut next = stored.clone();
            {
                // 结算数学单源（SG4b）：`commit_structurally_valid_revision`
                // 的同 CAS deliver 与本入口共用 `settle_turn_receipt` 核心
                // ——三终态互斥恰一次、同 receipt 重放零写、无 hold 不造账、
                // released_structural 快照恢复（不实扣）。
                let snapshot = next.human_gate_snapshot.as_mut();
                let outcome = next
                    .structural_governance_state
                    .as_mut()
                    .expect("state checked above")
                    .settle_turn_receipt(snapshot, turn_id, &receipt)
                    .map_err(|error| match error {
                        StructuralSettlementError::TerminalConflict => {
                            ProductStoreError::Conflict {
                                kind: "user_budget_receipt",
                                id: turn_id.to_string(),
                            }
                        }
                        StructuralSettlementError::CommandMismatch => {
                            ProductStoreError::IdentityMismatch {
                                kind: "user_budget_hold",
                                id: receipt.command_id.clone(),
                            }
                        }
                        StructuralSettlementError::NoActiveHold => {
                            ProductStoreError::InvalidRecord {
                                kind: "user_budget_hold",
                                reason: "no active hold for turn".to_string(),
                            }
                        }
                    })?;
                if matches!(outcome, StructuralSettlement::Replayed) {
                    // 恰一次：同 receipt 重放零写。
                    return Ok(stored);
                }
            }
            next.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &next)?;
            Ok(next)
        })
    }
}

/// SG6a.3（plan v1.1 Task SG6a.3 / REQ-SG-05）：reserve 原子创建的结构
/// 修订授权窗口。授权摘要对已接受 typed feedback 的 turn 身份全量内容
/// 寻址（turn/command/feedback/基线 source hash）；attempt 从 0 起，物
/// 理 attempt 消费由 `claim_structural_attempt` 推进（claim 与真实 start
/// 分账）；phase=AuthorRunning 是唯一合法初相（无评审/终态授权）。
fn structural_revision_grant_for_turn(
    turn: &HumanGateTurn,
) -> crate::product::models::workspace::StructuralRevisionGrant {
    use sha2::{Digest, Sha256};
    let authorization_hash = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(
            [
                turn.turn_id.as_str(),
                turn.command_id.as_str(),
                turn.feedback_text.as_str(),
                turn.source_hash.as_str(),
            ]
            .join("\0")
            .as_bytes(),
        ))
    );
    crate::product::models::workspace::StructuralRevisionGrant {
        grant_id: format!("structural_grant_{}", turn.turn_id),
        authorization_hash,
        turn_id: Some(turn.turn_id.clone()),
        command_id: Some(turn.command_id.clone()),
        candidate_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        phase: Some(StructuralGrantPhase::AuthorRunning),
        attempt_no: 0,
        prompt_ref: None,
        prompt_digest: None,
        instruction_digest: None,
        failing_source_hash: None,
        review_identity: None,
        granted_at: Utc::now().to_rfc3339(),
    }
}
