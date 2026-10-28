impl super::CodingAttemptStore {
    pub fn ensure_provider_run_allowed(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let authoritative = self.validate_attempt_lineage(attempt)?;
        let current = match self.reconcile_linked_plan_repair_pause(&authoritative) {
            Ok(reconciliation) => reconciliation.attempt,
            Err(_)
                if matches!(
                    authoritative.status,
                    CodingAttemptStatus::AwaitingPlanAmendment
                        | CodingAttemptStatus::ApplyingPlanAmendment
                        | CodingAttemptStatus::AmendmentApplyFailed
                ) =>
            {
                return Err(ProductStoreError::Io(
                    "plan_amendment_blocks_provider_run".to_string(),
                ));
            }
            Err(error) => return Err(error),
        };
        if matches!(
            current.status,
            CodingAttemptStatus::AwaitingPlanAmendment
                | CodingAttemptStatus::ApplyingPlanAmendment
                | CodingAttemptStatus::AmendmentApplyFailed
        ) {
            return Err(ProductStoreError::Io(
                "plan_amendment_blocks_provider_run".to_string(),
            ));
        }
        // 第二道防线：可执行状态必须经 admission 进入并留有会话标记。即使有人绕过
        // 接线直接写盘把 attempt 置为 Running，marker 缺失时 provider run 仍被拒。
        // Legacy/Logical attempt 一视同仁——两者都必须经 admit（Legacy 走 legacy
        // digest 路径）消费 ticket 后才能启动 provider run。
        if current.status == CodingAttemptStatus::Running
            && current.admission_ticket_consumed_at.is_none()
        {
            return Err(ProductStoreError::Io(
                "admission_missing_blocks_provider_run".to_string(),
            ));
        }
        // N2/S2b（REQ-CS-05）：plan-coding first_ready_unit 的授权边界——
        // Paused（legacy 步暂停，恢复走入口面 resume 语义）与 Invalidated
        // run 拒绝 provider 进入；冻结 selected unit 已 Completed 的窗口
        // 不再默认暂停（P5 撤除）：完成链自动推进在同一 run 授权下启动
        // 后续 unit 的 provider run。
        let plan_run_store = crate::product::coding_plan_run::CodingPlanRunStore::new(self.paths());
        if let Some(run) = plan_run_store.matching_first_ready_run(&current)? {
            match run.phase {
                crate::product::coding_plan_run::CodingPlanRunPhase::Paused => {
                    return Err(ProductStoreError::Io(format!(
                        "plan_coding_paused: {}",
                        current.id
                    )));
                }
                crate::product::coding_plan_run::CodingPlanRunPhase::Invalidated => {
                    return Err(ProductStoreError::Io(format!(
                        "plan_coding_invalidated: {}",
                        current.id
                    )));
                }
                _ => {}
            }
        }
        // 批 2 T1（REQ-WIGA-09）：精确绑定已 terminated 的 plan 拒绝 provider
        // 驱动——无 run 文件也关闸（终止事实先于 run 失效可见，不依赖
        // matching_first_ready_run 命中）；manual/无绑定/异 plan 不拦。
        let paths = self.paths();
        if let Some(terminated_at) = automation_plan_terminated_for_attempt(&paths, &current)? {
            return Err(ProductStoreError::Io(format!(
                "automation_plan_terminated: {terminated_at}"
            )));
        }
        Ok(current)
    }

    pub fn block_active_unit_run_for_plan_repair(
        &self,
        attempt: &CodingExecutionAttempt,
        increment_plan_repair_count: bool,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let active = self.get_active_unit_run(&current)?;
        self.block_unit_run_for_plan_repair(
            &current,
            &active.unit_id,
            &active.id,
            increment_plan_repair_count,
        )
    }

    pub fn block_unit_run_for_plan_repair(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_id: &str,
        unit_run_id: &str,
        increment_plan_repair_count: bool,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        if !matches!(
            current.status,
            CodingAttemptStatus::Running | CodingAttemptStatus::AwaitingPlanAmendment
        ) {
            return Err(ProductStoreError::Io(format!(
                "plan_repair_invalid_attempt_status: {:?}",
                current.status
            )));
        }
        validate_relative_id(unit_id)?;
        validate_relative_id(unit_run_id)?;
        let latest = self
            .list_coding_unit_runs(&current, unit_id)?
            .into_iter()
            .max_by_key(|run| run.execution_no)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_unit_run",
                id: unit_id.to_string(),
            })?;
        if latest.id != unit_run_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_unit_run_plan_repair_authority",
                id: unit_run_id.to_string(),
            });
        }
        let path = self.coding_unit_run_path(
            &current.project_id,
            &current.issue_id,
            &current.id,
            unit_id,
            unit_run_id,
        );
        with_exclusive_lock(&path, || {
            let mut run: CodingUnitRun = read_json(&path)?;
            if run.id != unit_run_id || run.unit_id != unit_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_unit_run",
                    id: unit_run_id.to_string(),
                });
            }
            if run.status == CodingUnitRunStatus::BlockedByPlanDefect {
                return Ok(run);
            }
            if !matches!(
                run.status,
                CodingUnitRunStatus::Pending
                    | CodingUnitRunStatus::Running
                    | CodingUnitRunStatus::Completed
                    | CodingUnitRunStatus::Blocked
            ) {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_unit_run_plan_repair_transition",
                    id: run.id,
                });
            }
            run.status = CodingUnitRunStatus::BlockedByPlanDefect;
            if increment_plan_repair_count {
                run.plan_repair_count = run.plan_repair_count.saturating_add(1);
            }
            run.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &run)?;
            Ok(run)
        })
    }

    pub fn create_blocked_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        input: CreateBlockedGateInput,
    ) -> Result<CodingGateRequired, ProductStoreError> {
        self.create_blocked_gate_with_diagnostic(attempt, input, None)
    }

    pub fn create_blocked_gate_with_diagnostic(
        &self,
        attempt: &CodingExecutionAttempt,
        input: CreateBlockedGateInput,
        diagnostic: Option<crate::product::coding_models::CodingGateDiagnostic>,
    ) -> Result<CodingGateRequired, ProductStoreError> {
        validate_relative_id(&input.attempt_id)?;
        if let Some(node_id) = &input.node_id {
            validate_relative_id(node_id)?;
        }
        self.validate_scoped_attempt_record(
            attempt,
            &input.attempt_id,
            "coding_blocked_gate",
            &input.attempt_id,
        )?;
        let gates_root =
            self.blocked_gates_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let lock_target = gates_root.join("blocked-gate-mutations");
        with_exclusive_lock(&lock_target, || {
            if is_group_review_conclusion_reason(input.reason_code.as_deref()) {
                resolve_open_group_review_failure_gates(&gates_root, &input.attempt_id)?;
            }
            create_blocked_gate_unlocked(&gates_root, attempt, input, diagnostic)
        })
    }

    /// P2-c（REQ-CRO-09，C1 裁定）：`triage_summary` 的独立落盘入口。不给
    /// `CreateBlockedGateInput` 加字段（其全仓字面量众多且多数不在本包清单），
    /// 引擎建门后按 gate_id 回写摘要；回写失败不影响已落盘的门（调用方按
    /// F-7 忽略错误）。
    pub fn attach_blocked_gate_triage_summary(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
        summary: crate::product::coding_models::CodingTriageSummary,
    ) -> Result<CodingGateRequired, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let gates_root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        let lock_target = gates_root.join("blocked-gate-mutations");
        with_exclusive_lock(&lock_target, || {
            let path = gates_root.join(format!("{gate_id}.json"));
            if !super::path_is_regular_file(&path)? {
                return Err(ProductStoreError::NotFound {
                    kind: "coding_blocked_gate",
                    id: gate_id.to_string(),
                });
            }
            let mut record: BlockedGateRecord = read_json(&path)?;
            record.gate.triage_summary = Some(summary);
            record.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &record)?;
            Ok(record.gate)
        })
    }

    /// change `triage-gate-actions-by-verdict`：分诊门派生事实的独立落盘入口
    /// （仿 `attach_blocked_gate_triage_summary`）。把建门时派生
    /// `available_actions` 所用的最新 Code Review verdict 写到门记录上，供呈现面
    /// 生成「为什么没有送回 Coder」文案；`available_actions` 仍是权威动作集合，
    /// 本入口 MUST NOT 改动动作。
    ///
    /// `verdict` 为 `None` 表示「读不到事实」（fail-closed 三项派生），写入时显式
    /// 置 `None`：去重复用同门实例（同 attempt/node/stage/reason_code）时不会残留
    /// 上一轮的 verdict。回写失败不影响已落盘的门（调用方按 F-7 忽略错误）。
    pub fn attach_blocked_gate_latest_review_verdict(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
        verdict: Option<crate::product::coding_models::ReviewVerdict>,
    ) -> Result<CodingGateRequired, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let gates_root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        let lock_target = gates_root.join("blocked-gate-mutations");
        with_exclusive_lock(&lock_target, || {
            let path = gates_root.join(format!("{gate_id}.json"));
            if !super::path_is_regular_file(&path)? {
                return Err(ProductStoreError::NotFound {
                    kind: "coding_blocked_gate",
                    id: gate_id.to_string(),
                });
            }
            let mut record: BlockedGateRecord = read_json(&path)?;
            record.gate.latest_review_verdict = verdict;
            record.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &record)?;
            Ok(record.gate)
        })
    }

    /// P2-c：按 `save_provider_raw_output` 生成的引用读回原始输出。宽容语义：
    /// 引用形状不符或文件不存在返回 `Ok(None)`（F-7：读不到绝不影响开门）。
    pub fn read_provider_raw_output_by_ref(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        reference: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        let Some(relative) = reference.strip_prefix("provider-raw/") else {
            return Ok(None);
        };
        if relative.is_empty()
            || relative
                .split('/')
                .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Ok(None);
        }
        let path = self
            .provider_raw_output_root(project_id, issue_id, attempt_id)
            .join(relative);
        match std::fs::read_to_string(&path) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                path.display()
            ))),
        }
    }

    pub fn create_group_review_failure_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        mut input: CreateBlockedGateInput,
        diagnostic: crate::product::coding_models::CodingGateDiagnostic,
    ) -> Result<CodingGateRequired, ProductStoreError> {
        input.available_actions =
            group_review_failure_actions(input.reason_code.as_deref().unwrap_or_default());
        self.validate_scoped_attempt_record(
            attempt,
            &input.attempt_id,
            "group_review_failure_gate",
            &input.attempt_id,
        )?;
        let gates_root =
            self.blocked_gates_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let lock_target = gates_root.join("blocked-gate-mutations");
        with_exclusive_lock(&lock_target, || {
            let mut active = super::json_file_paths(&gates_root)?
                .into_iter()
                .map(|path| Ok((path.clone(), read_json::<BlockedGateRecord>(&path)?)))
                .collect::<Result<Vec<_>, ProductStoreError>>()?
                .into_iter()
                .filter(|(_, record)| {
                    record.status == BlockedGateStatus::Open
                        && record.attempt_id == input.attempt_id
                        && is_group_review_failure_reason(
                            record.gate.reason_code.as_deref().unwrap_or_default(),
                        )
                })
                .collect::<Vec<_>>();
            active.sort_by_key(|(_, record)| {
                std::cmp::Reverse(group_review_failure_priority(
                    record.gate.reason_code.as_deref().unwrap_or_default(),
                ))
            });

            if let Some((path, mut existing)) = active.into_iter().next() {
                let decision = group_review_failure_gate_decision(
                    existing.gate.reason_code.as_deref().unwrap_or_default(),
                    input.reason_code.as_deref().unwrap_or_default(),
                );
                if decision != GroupReviewFailureGateDecision::ReplaceExisting {
                    super::merge_unique_strings(
                        &mut existing.gate.evidence_refs,
                        input.evidence_refs,
                    );
                    existing.gate.raw_provider_output_ref = input
                        .raw_provider_output_ref
                        .or(existing.gate.raw_provider_output_ref);
                    existing.gate.diagnostic = Some(diagnostic);
                    existing.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &existing)?;
                    return Ok(existing.gate);
                }
                // Bug2-A(Task 3):组门被替代走统一关闭原语,机器原因用
                // 明确审计值 system_superseded(归档+删根,不再散落赋值)。
                resolve_blocked_gate_record_unlocked(
                    &gates_root,
                    &existing.gate.gate_id,
                    Some("system_superseded"),
                )?;
            }

            for path in super::json_file_paths(&gates_root)? {
                let record: BlockedGateRecord = read_json(&path)?;
                if record.status == BlockedGateStatus::Open
                    && record.attempt_id == input.attempt_id
                    && is_group_review_conclusion_reason(record.gate.reason_code.as_deref())
                {
                    let Some(gate_id) = path.file_stem().and_then(|id| id.to_str()) else {
                        continue;
                    };
                    resolve_blocked_gate_record_unlocked(
                        &gates_root,
                        gate_id,
                        Some("system_superseded"),
                    )?;
                }
            }
            let gate = create_blocked_gate_unlocked(&gates_root, attempt, input, Some(diagnostic))?;
            Ok(gate)
        })
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
        Ok(records
            .into_iter()
            .map(|record| normalize_blocked_gate(record.gate))
            .collect())
    }

    /// P4-c（REQ-CRO-12）：open blocked 门的只读事实面——`(gate, created_at)`
    /// 元组。`created_at` 是 store 私有 `BlockedGateRecord` 字段，经此入口
    /// 只读透出，供 web 层 attention 投影作 `occurred_at`（事实时间，不用
    /// GET now）；store 不依赖 web DTO。
    pub fn list_open_blocked_gate_facts(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<(CodingGateRequired, String)>, ProductStoreError> {
        let mut records: Vec<BlockedGateRecord> =
            super::list_json_records(&self.blocked_gates_root(project_id, issue_id, attempt_id))?;
        records.retain(|record| record.status == BlockedGateStatus::Open);
        Ok(records
            .into_iter()
            .map(|record| {
                let created_at = record.created_at;
                (normalize_blocked_gate(record.gate), created_at)
            })
            .collect())
    }

    pub fn open_blocked_gate_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let path = self
            .blocked_gates_root(project_id, issue_id, attempt_id)
            .join(format!("{gate_id}.json"));
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        let record: BlockedGateRecord = read_json(&path)?;
        Ok((record.status == BlockedGateStatus::Open)
            .then_some(record.node_id)
            .flatten())
    }

    pub fn resolve_blocked_gate(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<ResolveOutcome, ProductStoreError> {
        self.resolve_blocked_gate_with_action(project_id, issue_id, attempt_id, gate_id, None)
    }

    /// C2 Task 4（REQ-CRO-04）＋Bug2-A(Task 3)：统一持久化关闭原语——
    /// blocked-gate-mutations 锁内执行「校验→根记录写完整 resolved 事实
    /// （resolved_action+resolved_at）→写归档→删根记录」。manual_continue／
    /// accept_risk 等续跑动作被持久化为「已确认结论」的证据，续跑 runner
    /// 据此跳过已完成的 reviewer（复用持久化结论，不重跑）。
    ///
    /// 幂等与崩溃收敛（四种前缀）：
    /// - 根已关、归档未完→按根记录首关事实补齐归档并删根；
    /// - 归档完、根仍在→清理残留根记录，返回归档首关结果；
    /// - 都全（仅归档）→同代次同动作重放幂等返回首关结果；
    /// - 都不存在→NotFound。
    ///
    /// 异动作冲突不覆盖：已关闭门的既有动作与传入动作不同时返回
    /// Conflict，审计保持首关事实（None 传入视作「不指定」，仅收敛）。
    pub fn resolve_blocked_gate_with_action(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
        resolved_action: Option<&str>,
    ) -> Result<ResolveOutcome, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let gates_root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        let lock_target = gates_root.join("blocked-gate-mutations");
        with_exclusive_lock(&lock_target, || {
            resolve_blocked_gate_record_unlocked(&gates_root, gate_id, resolved_action)
        })
    }

    /// Bug2-A(Task 3)：run 终态迟到的 origin 精确匹配关闭——仅当 open
    /// 根记录的 gate_id+generation 与 origin 完全一致才关（动作=
    /// origin.action_id）。重开（代次已增）后的新门对旧 origin 一律
    /// Ok(None)，不误关；已按同代次同动作关闭则幂等返回首关结果；
    /// 其余（异动作已关/门不存在）返回 None，零写入。
    pub fn resolve_blocked_gate_for_origin(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        origin: &GateOrigin,
    ) -> Result<Option<ResolveOutcome>, ProductStoreError> {
        validate_relative_id(&origin.gate_id)?;
        let gates_root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        let lock_target = gates_root.join("blocked-gate-mutations");
        with_exclusive_lock(&lock_target, || {
            let root_path = gates_root.join(format!("{}.json", origin.gate_id));
            if super::path_is_regular_file(&root_path)? {
                let record: BlockedGateRecord = read_json(&root_path)?;
                if record.status == BlockedGateStatus::Open {
                    if record.resolution_generation != origin.generation {
                        // 重开后的新门:旧 origin 代次失配,不关。
                        return Ok(None);
                    }
                    let outcome = resolve_blocked_gate_record_unlocked(
                        &gates_root,
                        &origin.gate_id,
                        Some(origin.action_id.as_str()),
                    )?;
                    return Ok(Some(outcome));
                }
                // 根已带首关事实(崩溃前缀):仅同动作才补齐归档并幂等
                // 返回;异动作已关不碰(补偿绝不与人工决策互抢审计)。
                if record.resolved_action.as_deref() != Some(origin.action_id.as_str()) {
                    return Ok(None);
                }
                let outcome = resolve_blocked_gate_record_unlocked(
                    &gates_root,
                    &origin.gate_id,
                    Some(origin.action_id.as_str()),
                )?;
                return Ok(Some(outcome));
            }
            let Some(record) = read_archived_blocked_gate_record(&gates_root, &origin.gate_id)?
            else {
                return Ok(None);
            };
            let matches = record.resolution_generation == origin.generation
                && record.resolved_action.as_deref() == Some(origin.action_id.as_str());
            Ok(matches.then(|| resolve_outcome_from_record(record)))
        })
    }

    /// Bug2-A(Task 3)：open blocked 门根记录的当前解决代次——gate-retry
    /// 入口构造 GateOrigin 用；门非 open（已关/不存在）返回 None。
    pub fn open_blocked_gate_generation(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<Option<u64>, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let path = self
            .blocked_gates_root(project_id, issue_id, attempt_id)
            .join(format!("{gate_id}.json"));
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        let record: BlockedGateRecord = read_json(&path)?;
        Ok((record.status == BlockedGateStatus::Open).then_some(record.resolution_generation))
    }

    /// Bug2-A(Task 3)：resolved 归档记录的只读事实面——补偿对账/测试
    /// 断言消费；归档不存在返回 None。
    pub fn get_archived_blocked_gate_facts(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
    ) -> Result<Option<ArchivedBlockedGateFacts>, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let Some(record) = read_archived_blocked_gate_record(
            &self.blocked_gates_root(project_id, issue_id, attempt_id),
            gate_id,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(ArchivedBlockedGateFacts {
            gate: normalize_blocked_gate(record.gate),
            resolved_action: record.resolved_action,
            resolved_at: record.resolved_at,
            resolution_generation: record.resolution_generation,
        }))
    }

    /// Bug2-B(Task 4)：启动 gate-only 对账的根记录视图——open 门（含当前
    /// 代次）与「已写完整关闭事实但归档未落」的崩溃前缀记录；归档完整的
    /// 门不在列（另经 `archived_blocked_gate_facts_for_attempt` 枚举）。
    /// 只读视图，行动一律经统一关闭/reopen 原语在同一 mutations 锁内复核。
    pub fn blocked_gate_states_for_startup_reconcile(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<StartupBlockedGateState>, ProductStoreError> {
        let gates_root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        let mut states = Vec::new();
        for path in super::json_file_paths(&gates_root)? {
            let record: BlockedGateRecord = read_json(&path)?;
            let pending_resolution =
                (record.status == BlockedGateStatus::Resolved).then(|| ArchivedBlockedGateFacts {
                    gate: normalize_blocked_gate(record.gate.clone()),
                    resolved_action: record.resolved_action.clone(),
                    resolved_at: record.resolved_at.clone(),
                    resolution_generation: record.resolution_generation,
                });
            states.push(StartupBlockedGateState {
                gate: normalize_blocked_gate(record.gate),
                generation: record.resolution_generation,
                pending_resolution,
            });
        }
        Ok(states)
    }

    /// Bug2-B(Task 4)：resolved 归档清单（含缺 action 的旧档）——启动对账
    /// 的证据回填扫描面。
    pub fn archived_blocked_gate_facts_for_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<ArchivedBlockedGateFacts>, ProductStoreError> {
        let resolved_root = self
            .blocked_gates_root(project_id, issue_id, attempt_id)
            .join("resolved");
        let mut facts = Vec::new();
        for path in super::json_file_paths(&resolved_root)? {
            let record: BlockedGateRecord = read_json(&path)?;
            facts.push(ArchivedBlockedGateFacts {
                gate: normalize_blocked_gate(record.gate),
                resolved_action: record.resolved_action,
                resolved_at: record.resolved_at,
                resolution_generation: record.resolution_generation,
            });
        }
        Ok(facts)
    }

    /// Bug2-B(Task 4)：按证据回填归档动作——仅当归档存在且
    /// `resolved_action` 缺失（旧数据）时写入动作；已有动作（含异动作）
    /// 一律不覆盖，`resolved_at` 历史不伪造（保持缺失）。
    /// blocked-gate-mutations 锁内执行；返回回填后事实（未命中返回 None）。
    pub fn backfill_archived_blocked_gate_action(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        gate_id: &str,
        action: &str,
    ) -> Result<Option<ArchivedBlockedGateFacts>, ProductStoreError> {
        validate_relative_id(gate_id)?;
        let gates_root = self.blocked_gates_root(project_id, issue_id, attempt_id);
        let lock_target = gates_root.join("blocked-gate-mutations");
        with_exclusive_lock(&lock_target, || {
            let archive_path = gates_root.join("resolved").join(format!("{gate_id}.json"));
            if !super::path_is_regular_file(&archive_path)? {
                return Ok(None);
            }
            let mut record: BlockedGateRecord = read_json(&archive_path)?;
            if record.resolved_action.is_some() {
                return Ok(None);
            }
            record.resolved_action = Some(action.to_string());
            write_json(&archive_path, &record)?;
            Ok(Some(ArchivedBlockedGateFacts {
                gate: normalize_blocked_gate(record.gate),
                resolved_action: record.resolved_action,
                resolved_at: record.resolved_at,
                resolution_generation: record.resolution_generation,
            }))
        })
    }
}
