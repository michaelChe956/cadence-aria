impl super::CodingAttemptStore {
    /// C2 Task 4（REQ-CRO-04）：指定 stage 的已解决门中，是否存在以
    /// manual_continue／accept_risk 解决且证据引用了给定报告 id 的记录
    /// ——续跑 runner 跳过已完成 reviewer 的唯一判据（retry_* 解决的门
    /// 不构成跳过：重跑评审是预期行为）。
    pub fn has_resolved_gate_confirming_report(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        stage: CodingExecutionStage,
        report_id: &str,
    ) -> Result<bool, ProductStoreError> {
        let resolved_root = self
            .blocked_gates_root(project_id, issue_id, attempt_id)
            .join("resolved");
        let records: Vec<BlockedGateRecord> = super::list_json_records(&resolved_root)?;
        let confirmed_stage = Some(stage);
        Ok(records.iter().any(|record| {
            record.gate.stage == confirmed_stage
                && matches!(
                    record.resolved_action.as_deref(),
                    Some("manual_continue") | Some("accept_risk")
                )
                && record
                    .gate
                    .evidence_refs
                    .iter()
                    .any(|evidence| evidence == report_id)
        }))
    }

    /// Bug2-A(Task 3):重开(resolved→open)在同一 blocked-gate-mutations
    /// 锁内递增 resolution_generation、清空关闭字段并移除归档——旧 run
    /// 的 gate_origin 因代次失配全部失效,迟到终态不会误关新一轮门。
    /// 根记录已 open 时为幂等清理(仅移除崩溃残留的归档,不动代次)。
    pub(crate) fn reopen_failed_code_review_gate_for_plan_repair(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(gate_id)?;
        let gates_root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        let lock_target = gates_root.join("blocked-gate-mutations");
        with_exclusive_lock(&lock_target, || {
            let open_path = gates_root.join(format!("{gate_id}.json"));
            let resolved_path = gates_root.join("resolved").join(format!("{gate_id}.json"));
            if super::path_is_regular_file(&open_path)? {
                return super::remove_file_if_exists(&resolved_path);
            }
            if !super::path_is_regular_file(&resolved_path)? {
                return Err(ProductStoreError::NotFound {
                    kind: "coding_blocked_gate",
                    id: gate_id.to_string(),
                });
            }
            let mut record: BlockedGateRecord = read_json(&resolved_path)?;
            record.status = BlockedGateStatus::Open;
            record.resolution_generation = record.resolution_generation.saturating_add(1);
            record.resolved_action = None;
            record.resolved_at = None;
            record.updated_at = Utc::now().to_rfc3339();
            write_json(&open_path, &record)?;
            super::remove_file_if_exists(&resolved_path)
        })
    }

    pub fn create_choice_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        input: CreateChoiceGateInput,
    ) -> Result<CodingChoiceGate, ProductStoreError> {
        validate_relative_id(&input.attempt_id)?;
        if let Some(node_id) = &input.node_id {
            validate_relative_id(node_id)?;
        }
        self.validate_scoped_attempt_record(
            attempt,
            &input.attempt_id,
            "coding_choice_gate",
            &input.choice_id,
        )?;
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
            gate.questions = input.questions;
            gate.updated_at = Utc::now().to_rfc3339();
            write_json(&existing_path, &gate)?;
            return Ok(gate);
        }

        let gate_id = next_sequential_id_in_gate_directories("coding_choice_gate", &gates_root)?;
        let now = Utc::now().to_rfc3339();
        let gate = CodingChoiceGate {
            gate_id: gate_id.clone(),
            choice_id: input.choice_id,
            attempt_id: attempt.id.clone(),
            node_id: input.node_id,
            stage: input.stage,
            role: input.role,
            provider: input.provider,
            source: input.source,
            prompt: input.prompt,
            options: input.options,
            allow_multiple: input.allow_multiple,
            allow_free_text: input.allow_free_text,
            questions: input.questions,
            // Task 11：run 化身是 handler 层投影注入，durable 记录恒 None。
            expected_run_id: None,
            response: None,
            status: CodingChoiceGateStatus::Open,
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
        input: ResolveChoiceGateInput,
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
            selected_option_ids: input.selected_option_ids,
            free_text: input.free_text,
            responded_at: Utc::now().to_rfc3339(),
            answers: input.answers,
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

    /// 批 2 T2（REQ-WIGA-09）：choice 门全量读（含终态 Cancelled/Resolved
    /// 记录；与 `list_stage_gates` 对称）——终止收口/审计校验用。
    pub fn list_choice_gates(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingChoiceGate>, ProductStoreError> {
        super::list_json_records(&self.choice_gates_root(project_id, issue_id, attempt_id))
    }

    /// 批 2 T2（C5，REQ-WIGA-09）：取消 open choice 门（终止收口专用）——
    /// 置 `Cancelled`、**不写 response**（不伪造答案）；按 open 门集合调用，
    /// 幂等由调用方先过滤保证（非 open 门按 NotFound 上抛）。
    pub fn cancel_choice_gate(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        choice_id: &str,
    ) -> Result<CodingChoiceGate, ProductStoreError> {
        let gates_root = self.choice_gates_root(project_id, issue_id, attempt_id);
        let Some(path) = matching_open_choice_gate_path(&gates_root, choice_id)? else {
            return Err(ProductStoreError::NotFound {
                kind: "coding_choice_gate",
                id: choice_id.to_string(),
            });
        };
        let mut gate: CodingChoiceGate = read_json(&path)?;
        gate.status = CodingChoiceGateStatus::Cancelled;
        gate.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &gate)?;
        Ok(gate)
    }

    /// 批 2 T2（C5，REQ-WIGA-09）：终止收口关门——关闭该 attempt 全部
    /// open blocked/choice/stage 门：stage→Cancelled、choice→Cancelled
    ///（无 response，不伪造答案）、blocked→Resolved（resolved_action=
    /// `plan_terminated` 审计标记）。幂等（无 open 门＝0）；返回关闭门数。
    pub fn close_all_open_gates_for_termination(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<usize, ProductStoreError> {
        let mut closed = 0;
        for gate in self.list_open_stage_gates(project_id, issue_id, attempt_id)? {
            self.update_stage_gate_status(
                project_id,
                issue_id,
                attempt_id,
                &gate.gate_id,
                CodingStageGateStatus::Cancelled,
            )?;
            closed += 1;
        }
        for gate in self.list_open_choice_gates(project_id, issue_id, attempt_id)? {
            self.cancel_choice_gate(project_id, issue_id, attempt_id, &gate.choice_id)?;
            closed += 1;
        }
        for gate in self.list_open_blocked_gates(project_id, issue_id, attempt_id)? {
            self.resolve_blocked_gate_with_action(
                project_id,
                issue_id,
                attempt_id,
                &gate.gate_id,
                Some("plan_terminated"),
            )?;
            closed += 1;
        }
        Ok(closed)
    }

    pub fn create_quality_bypass_audit(
        &self,
        attempt: &CodingExecutionAttempt,
        input: CreateQualityBypassAuditInput,
    ) -> Result<QualityGateBypassAudit, ProductStoreError> {
        validate_relative_id(&input.attempt_id)?;
        validate_relative_id(&input.gate_id)?;
        self.validate_scoped_attempt_record(
            attempt,
            &input.attempt_id,
            "quality_bypass_audit",
            &input.gate_id,
        )?;
        let root =
            self.quality_bypass_audits_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let id = next_sequential_id_in_directory("quality_bypass_audit", &root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?;
        let audit = QualityGateBypassAudit {
            id: id.clone(),
            attempt_id: attempt.id.clone(),
            gate_id: input.gate_id,
            stage: input.stage,
            reason_code: input.reason_code,
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
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        expires_at: String,
        provider_snapshot: CodingRoleProviderConfigSnapshot,
    ) -> Result<CodingStageGateState, ProductStoreError> {
        self.validate_scoped_attempt_record(
            attempt,
            &attempt.id,
            "coding_stage_gate",
            &attempt.id,
        )?;
        let gates_root = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("stage-gates");
        let gate_id =
            next_sequential_id_in_directory("coding_stage_gate", &gates_root).map_err(|error| {
                ProductStoreError::Io(format!("read {}: {error}", gates_root.display()))
            })?;
        let now = Utc::now().to_rfc3339();
        let gate = CodingStageGateState {
            gate_id: gate_id.clone(),
            attempt_id: attempt.id.clone(),
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
