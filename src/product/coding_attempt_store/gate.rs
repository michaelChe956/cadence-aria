use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_attempt_store::{
    CreateBlockedGateInput, CreateChoiceGateInput, CreateQualityBypassAuditInput,
    ResolveChoiceGateInput,
};
use crate::product::coding_models::{
    CodingAttemptStatus, CodingChoiceGate, CodingChoiceGateResponse, CodingChoiceGateStatus,
    CodingExecutionAttempt, CodingExecutionStage, CodingGateAction, CodingGateActionType,
    CodingGateKind, CodingGateRequired, CodingProviderRole, CodingRoleProviderConfigSnapshot,
    CodingStageGateState, CodingStageGateStatus, CodingUnitRun, CodingUnitRunStatus, GateOrigin,
    QualityGateBypassAudit,
};
use crate::product::id::{next_sequential_id_from_existing, next_sequential_id_in_directory};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::locking::with_exclusive_lock;

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
    /// C2 Task 4（REQ-CRO-04）：解决该门的动作 id（manual_continue／
    /// accept_risk／retry_review 等）。旧记录缺字段按 None 解释。
    #[serde(default)]
    resolved_action: Option<String>,
    /// Bug2-A(Task 3)：首关时间——首次落 Resolved 事实时写入一次,重放
    /// 与崩溃收敛不重写;旧归档缺字段按 None 解释(历史时间不伪造)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_at: Option<String>,
    /// Bug2-A(Task 3)：解决代次——重开(resolved→open)递增;旧记录缺
    /// 字段按 0 解释。run 的 gate_origin.generation 与此比对,失配即拒关。
    #[serde(default)]
    resolution_generation: u64,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupReviewFailureGateDecision {
    ReplaceExisting,
    KeepExistingAppendEvidence,
    MergeEvidence,
}

pub fn group_review_failure_gate_decision(
    existing_reason_code: &str,
    incoming_reason_code: &str,
) -> GroupReviewFailureGateDecision {
    let existing_priority = group_review_failure_priority(existing_reason_code);
    let incoming_priority = group_review_failure_priority(incoming_reason_code);
    if incoming_priority > existing_priority {
        GroupReviewFailureGateDecision::ReplaceExisting
    } else if incoming_reason_code == existing_reason_code {
        GroupReviewFailureGateDecision::MergeEvidence
    } else {
        GroupReviewFailureGateDecision::KeepExistingAppendEvidence
    }
}

fn group_review_failure_priority(reason_code: &str) -> u8 {
    match reason_code {
        "capacity_exceeded" => 7,
        "material_overflow" => 6,
        "identity_missing" => 5,
        "reduction_output_invalid" => 4,
        "reduction_transport_exhausted" => 3,
        "shard_output_invalid" => 2,
        "shard_transport_exhausted" => 1,
        _ => 0,
    }
}

fn group_review_failure_actions(reason_code: &str) -> Vec<CodingGateAction> {
    let retry = match reason_code {
        "reduction_output_invalid" | "reduction_transport_exhausted" => CodingGateAction {
            action_id: "retry_group_reduction".to_string(),
            label: "重试组审查归约".to_string(),
            action_type: CodingGateActionType::RetryGroupReduction,
        },
        _ => CodingGateAction {
            action_id: "retry_group_review_shard".to_string(),
            label: "重试组审查分片".to_string(),
            action_type: CodingGateActionType::RetryGroupReviewShard,
        },
    };
    vec![
        retry,
        CodingGateAction {
            action_id: "abort".to_string(),
            label: "终止".to_string(),
            action_type: CodingGateActionType::Abort,
        },
    ]
}

fn is_group_review_failure_reason(reason_code: &str) -> bool {
    matches!(
        reason_code,
        "capacity_exceeded"
            | "material_overflow"
            | "identity_missing"
            | "reduction_output_invalid"
            | "reduction_transport_exhausted"
            | "shard_output_invalid"
            | "shard_transport_exhausted"
    )
}

fn is_group_review_conclusion_reason(reason_code: Option<&str>) -> bool {
    matches!(
        reason_code,
        Some(
            "group_final_review_blocked"
                | "internal_review_change_requested"
                | "internal_review_verification_incomplete"
                | "internal_review_operational_blocker"
                | "internal_review_human_triage"
        )
    )
}

/// Bug2-A(Task 3):open 组审查失败门批量关闭——逐门走统一关闭原语
/// (机器原因 system_superseded:归档+删根+完整 resolved 事实),不再
/// 散落状态赋值。仅在 blocked-gate-mutations 锁内调用。
fn resolve_open_group_review_failure_gates(
    gates_root: &Path,
    attempt_id: &str,
) -> Result<(), ProductStoreError> {
    for path in super::json_file_paths(gates_root)? {
        let record: BlockedGateRecord = read_json(&path)?;
        if record.status == BlockedGateStatus::Open
            && record.attempt_id == attempt_id
            && is_group_review_failure_reason(
                record.gate.reason_code.as_deref().unwrap_or_default(),
            )
        {
            let Some(gate_id) = path.file_stem().and_then(|id| id.to_str()) else {
                continue;
            };
            resolve_blocked_gate_record_unlocked(gates_root, gate_id, Some("system_superseded"))?;
        }
    }
    Ok(())
}

/// Bug2-A(Task 3):统一关闭原语的返回事实——携带首关动作/时间与代次;
/// 同代次同动作重放返回首关结果(不因重放改写)。resolved_at 为 None
/// 仅见于缺该字段的旧归档(历史时间不伪造)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveOutcome {
    pub gate: CodingGateRequired,
    pub resolved_action: Option<String>,
    pub resolved_at: Option<String>,
    pub resolution_generation: u64,
}

/// Bug2-A(Task 3):resolved 归档记录的只读事实面——启动对账/补偿/测试
/// 断言消费(Task 4 接续)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedBlockedGateFacts {
    pub gate: CodingGateRequired,
    pub resolved_action: Option<String>,
    pub resolved_at: Option<String>,
    pub resolution_generation: u64,
}

/// 统一关闭原语核心(调用方必须已持有 blocked-gate-mutations 锁):
/// 校验→根记录写完整 resolved 事实→写归档→删根记录。四种崩溃前缀
/// 幂等收敛;异动作冲突返回 Conflict 不覆盖;都不存在返回 NotFound。
fn resolve_blocked_gate_record_unlocked(
    gates_root: &Path,
    gate_id: &str,
    resolved_action: Option<&str>,
) -> Result<ResolveOutcome, ProductStoreError> {
    let root_path = gates_root.join(format!("{gate_id}.json"));
    let archive_path = gates_root.join("resolved").join(format!("{gate_id}.json"));
    let root_exists = super::path_is_regular_file(&root_path)?;
    let archive_exists = super::path_is_regular_file(&archive_path)?;
    if !root_exists && !archive_exists {
        return Err(ProductStoreError::NotFound {
            kind: "coding_blocked_gate",
            id: gate_id.to_string(),
        });
    }

    if root_exists && archive_exists {
        // 崩溃前缀「归档完根仍在」:归档是首关事实,清理残留根记录。
        let record: BlockedGateRecord = read_json(&archive_path)?;
        ensure_resolved_action_consistent(&record, resolved_action, gate_id)?;
        super::remove_file_if_exists(&root_path)?;
        return Ok(resolve_outcome_from_record(record));
    }

    if root_exists {
        let mut record: BlockedGateRecord = read_json(&root_path)?;
        if record.status == BlockedGateStatus::Open {
            // 首次关闭:根记录先落完整 resolved 事实(list_open 不再投影),
            // 再写归档,最后删根——任意一步崩溃都可由本原语重放收敛。
            record.status = BlockedGateStatus::Resolved;
            record.resolved_action = resolved_action.map(str::to_string);
            record.resolved_at = Some(Utc::now().to_rfc3339());
            record.updated_at = record.resolved_at.clone().unwrap_or_default();
            write_json(&root_path, &record)?;
        } else {
            // 崩溃前缀「根已关归档未完」:以根记录首关事实补齐归档。
            ensure_resolved_action_consistent(&record, resolved_action, gate_id)?;
        }
        write_json(&archive_path, &record)?;
        super::remove_file_if_exists(&root_path)?;
        return Ok(resolve_outcome_from_record(record));
    }

    // 崩溃前缀「都全」(仅归档):同动作重放幂等返回首关结果。
    let record: BlockedGateRecord = read_json(&archive_path)?;
    ensure_resolved_action_consistent(&record, resolved_action, gate_id)?;
    Ok(resolve_outcome_from_record(record))
}

/// 已关闭记录的既有动作与传入动作均为 Some 且不同→Conflict(不覆盖审计);
/// 其余(None 传入或一致)放行收敛。
fn ensure_resolved_action_consistent(
    record: &BlockedGateRecord,
    resolved_action: Option<&str>,
    gate_id: &str,
) -> Result<(), ProductStoreError> {
    if let (Some(existing), Some(incoming)) = (record.resolved_action.as_deref(), resolved_action)
        && existing != incoming
    {
        return Err(ProductStoreError::Conflict {
            kind: "coding_blocked_gate_resolved_action",
            id: gate_id.to_string(),
        });
    }
    Ok(())
}

fn read_archived_blocked_gate_record(
    gates_root: &Path,
    gate_id: &str,
) -> Result<Option<BlockedGateRecord>, ProductStoreError> {
    let archive_path = gates_root.join("resolved").join(format!("{gate_id}.json"));
    if !super::path_is_regular_file(&archive_path)? {
        return Ok(None);
    }
    read_json(&archive_path).map(Some)
}

fn resolve_outcome_from_record(record: BlockedGateRecord) -> ResolveOutcome {
    ResolveOutcome {
        gate: normalize_blocked_gate(record.gate),
        resolved_action: record.resolved_action,
        resolved_at: record.resolved_at,
        resolution_generation: record.resolution_generation,
    }
}

fn create_blocked_gate_unlocked(
    gates_root: &Path,
    attempt: &CodingExecutionAttempt,
    input: CreateBlockedGateInput,
    diagnostic: Option<crate::product::coding_models::CodingGateDiagnostic>,
) -> Result<CodingGateRequired, ProductStoreError> {
    if let Some(existing_path) = matching_open_blocked_gate_path(gates_root, &input)? {
        let mut record: BlockedGateRecord = read_json(&existing_path)?;
        record.gate.title = input.title;
        record.gate.description = input.description;
        record.gate.role = input.role;
        record.gate.available_actions = input.available_actions;
        record.gate.raw_provider_output_ref = input
            .raw_provider_output_ref
            .or(record.gate.raw_provider_output_ref);
        record.gate.diagnostic = diagnostic.or(record.gate.diagnostic);
        super::merge_unique_strings(&mut record.gate.evidence_refs, input.evidence_refs);
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&existing_path, &record)?;
        return Ok(record.gate);
    }
    let gate_id = next_sequential_id_in_gate_directories("coding_blocked_gate", gates_root)?;
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
        diagnostic,
        // P2-c（C1 裁定）：摘要不经建门输入——建门后由
        // `attach_blocked_gate_triage_summary` 独立入口回写。
        triage_summary: None,
    };
    let record = BlockedGateRecord {
        gate: gate.clone(),
        attempt_id: attempt.id.clone(),
        node_id: input.node_id,
        status: BlockedGateStatus::Open,
        resolved_action: None,
        resolved_at: None,
        resolution_generation: 0,
        created_at: now.clone(),
        updated_at: now,
    };
    write_json(&gates_root.join(format!("{gate_id}.json")), &record)?;
    Ok(gate)
}

fn next_sequential_id_in_gate_directories(
    prefix: &str,
    gates_root: &Path,
) -> Result<String, ProductStoreError> {
    let resolved_root = gates_root.join("resolved");
    let paths = [
        super::json_file_paths(gates_root)?,
        super::json_file_paths(&resolved_root)?,
    ];
    Ok(next_sequential_id_from_existing(
        prefix,
        paths
            .iter()
            .flatten()
            .filter_map(|path| path.file_stem()?.to_str()),
    ))
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

fn normalize_blocked_gate(mut gate: CodingGateRequired) -> CodingGateRequired {
    if gate.reason_code.as_deref() != Some("code_review_blocked")
        || gate.stage != Some(CodingExecutionStage::CodeReview)
        || gate.role != Some(CodingProviderRole::CodeReviewer)
        || gate
            .available_actions
            .iter()
            .any(|action| action.action_type == CodingGateActionType::SendToCoder)
    {
        return gate;
    }

    let insert_index = gate
        .available_actions
        .iter()
        .position(|action| action.action_type == CodingGateActionType::Abort)
        .unwrap_or(gate.available_actions.len());
    gate.available_actions.insert(
        insert_index,
        CodingGateAction {
            action_id: "send_to_coder".to_string(),
            label: "提交给 Coder 修复".to_string(),
            action_type: CodingGateActionType::SendToCoder,
        },
    );
    gate
}

/// 批 2 T1（REQ-WIGA-09）：attempt 所属 plan 是否已被精确绑定的 enrollment
/// 终止。仅当 enrollment 显式绑定该 attempt 的 `work_item_group_id` 且
/// `terminated_at` 非空才返回 `Some(terminated_at)`——manual/无绑定/异 plan
/// 一律 `None`（手动无绑定路径保持原语义）。只读；损坏 JSON fail-closed
/// 上抛，不当 None 吞错。
pub(crate) fn automation_plan_terminated_for_attempt(
    paths: &crate::product::app_paths::ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<Option<String>, ProductStoreError> {
    let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
        return Ok(None);
    };
    let enrollment =
        crate::product::issue_automation_store::IssueAutomationStore::new(paths.clone())
            .get(&attempt.project_id, &attempt.issue_id)?;
    Ok(enrollment.and_then(|enrollment| {
        if enrollment.plan_id.as_deref() == Some(plan_id) {
            enrollment.terminated_at
        } else {
            None
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::{
        CodingAttemptStore, CreateCodingAttemptInput, CreateGroupCodingAttemptInput,
    };
    use crate::product::coding_models::{CodingChoiceOption, CodingChoiceQuestion};
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    fn fixture_attempt() -> (
        tempfile::TempDir,
        CodingAttemptStore,
        CodingExecutionAttempt,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: Some(ProviderName::Fake),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
            })
            .unwrap();
        (tmp, store, attempt)
    }

    fn coding_choice_option(id: &str, label: &str) -> CodingChoiceOption {
        CodingChoiceOption {
            id: id.to_string(),
            label: label.to_string(),
            description: None,
        }
    }

    fn two_questions() -> Vec<CodingChoiceQuestion> {
        vec![
            CodingChoiceQuestion {
                id: "q-1".to_string(),
                prompt: "是否包含集成测试".to_string(),
                options: vec![
                    coding_choice_option("yes", "包含"),
                    coding_choice_option("no", "不包含"),
                ],
                allow_multiple: false,
                allow_free_text: false,
            },
            CodingChoiceQuestion {
                id: "q-2".to_string(),
                prompt: "评审轮数".to_string(),
                options: vec![
                    coding_choice_option("one", "一轮"),
                    coding_choice_option("two", "两轮"),
                ],
                allow_multiple: false,
                allow_free_text: true,
            },
        ]
    }

    fn choice_input(attempt: &CodingExecutionAttempt) -> CreateChoiceGateInput {
        CreateChoiceGateInput {
            attempt_id: attempt.id.clone(),
            choice_id: "choice-question-1".to_string(),
            stage: CodingExecutionStage::Coding,
            node_id: None,
            role: CodingProviderRole::Coder,
            provider: ProviderName::Fake,
            source: "provider".to_string(),
            prompt: "拆分方案确认".to_string(),
            options: vec![
                coding_choice_option("a", "方案 A"),
                coding_choice_option("b", "方案 B"),
            ],
            allow_multiple: false,
            allow_free_text: false,
            questions: two_questions(),
        }
    }

    #[test]
    fn coding_choice_reply_gate_persists_multi_question_and_full_answers() {
        let (_tmp, store, attempt) = fixture_attempt();
        let gate = store
            .create_choice_gate(&attempt, choice_input(&attempt))
            .unwrap();
        assert_eq!(gate.questions.len(), 2);
        assert_eq!(gate.questions[0].id, "q-1");
        assert!(gate.questions[1].allow_free_text);

        // durable 读回同样含 questions；open 列表投影不丢题。
        let listed = store
            .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].questions.len(), 2);

        let answers = vec![
            crate::cross_cutting::streaming_provider::ChoiceAnswerData {
                question_id: "q-1".to_string(),
                selected_option_ids: vec!["yes".to_string()],
                free_text: None,
            },
            crate::cross_cutting::streaming_provider::ChoiceAnswerData {
                question_id: "q-2".to_string(),
                selected_option_ids: vec!["one".to_string()],
                free_text: Some("按一轮即可".to_string()),
            },
        ];
        let resolved = store
            .resolve_choice_gate(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                "choice-question-1",
                ResolveChoiceGateInput {
                    selected_option_ids: vec!["yes".to_string()],
                    free_text: None,
                    answers,
                },
            )
            .unwrap();
        assert_eq!(resolved.status, CodingChoiceGateStatus::Resolved);
        let response = resolved.response.unwrap();
        assert_eq!(response.answers.len(), 2);
        assert_eq!(response.answers[1].question_id, "q-2");
        assert_eq!(response.answers[1].free_text.as_deref(), Some("按一轮即可"));

        // resolve 后 open 列表收敛为空。
        assert!(
            store
                .list_open_choice_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn coding_choice_reply_gate_legacy_without_questions_defaults_single_question() {
        let (_tmp, store, attempt) = fixture_attempt();
        // 旧 gate：无 questions 字段（serde default 空）。
        let mut input = choice_input(&attempt);
        input.questions = Vec::new();
        let gate = store.create_choice_gate(&attempt, input).unwrap();
        assert!(gate.questions.is_empty());

        // 兼容投影：缺 questions 的旧 gate 按单一 default 题呈现，不猜多题。
        let effective = gate.effective_questions();
        assert_eq!(effective.len(), 1);
        assert_eq!(effective[0].id, "default");
        assert_eq!(effective[0].prompt, gate.prompt);
        assert_eq!(effective[0].options.len(), gate.options.len());

        // 旧答案形态（仅 selected/free_text）仍可 resolve（answers 缺省空）。
        let resolved = store
            .resolve_choice_gate(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                "choice-question-1",
                ResolveChoiceGateInput {
                    selected_option_ids: vec!["a".to_string()],
                    free_text: None,
                    answers: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(resolved.status, CodingChoiceGateStatus::Resolved);
        assert!(resolved.response.unwrap().answers.is_empty());
    }

    /// P2-c（REQ-CRO-09，C1 裁定）：triage_summary 独立落盘入口——不经过
    /// CreateBlockedGateInput，建门后按 gate_id 回写；durable 读回不丢；
    /// 未知 gate_id 报 NotFound。
    #[test]
    fn attach_blocked_gate_triage_summary_writes_on_open_gate() {
        let (_tmp, store, attempt) = fixture_attempt();
        let gate = store
            .create_blocked_gate(
                &attempt,
                CreateBlockedGateInput {
                    attempt_id: attempt.id.clone(),
                    stage: CodingExecutionStage::Coding,
                    node_id: Some("node_coding_0001".to_string()),
                    role: Some(CodingProviderRole::Coder),
                    title: "Coder 输出需要人工分诊".to_string(),
                    description: "Coder 输出需要人工分诊，流程已暂停".to_string(),
                    reason_code: Some("coding_output_human_triage".to_string()),
                    evidence_refs: Vec::new(),
                    raw_provider_output_ref: Some(
                        "provider-raw/coding/coder_output_0001.txt".to_string(),
                    ),
                    available_actions: Vec::new(),
                },
            )
            .unwrap();
        assert!(gate.triage_summary.is_none(), "新建门默认无摘要");

        let summary = crate::product::coding_models::CodingTriageSummary {
            problem: "CT-001 上游交付物缺失：busi 成员仓（WI-001）在运行环境中完全不存在"
                .to_string(),
            findings: vec![crate::product::coding_models::CodingTriageFinding {
                reason_code: "upstream_deliverable_absent_in_runtime".to_string(),
                message: "busi 成员仓不存在".to_string(),
            }],
            did: None,
            unverified: vec!["聚合根仅含 api/.git 骨架；busi 成员仓不存在".to_string()],
        };
        let updated = store
            .attach_blocked_gate_triage_summary(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &gate.gate_id,
                summary,
            )
            .unwrap();
        assert!(updated.triage_summary.is_some(), "回写后必须能读到摘要");
        assert_eq!(
            updated
                .triage_summary
                .as_ref()
                .map(|summary| summary.did.is_none()),
            Some(true)
        );

        // durable：open 列表读回不丢摘要。
        let listed = store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].triage_summary.is_some());

        // 未知 gate_id：NotFound（调用方按 F-7 忽略，不影响门本身）。
        let missing = store.attach_blocked_gate_triage_summary(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            "coding_blocked_gate_9999",
            crate::product::coding_models::CodingTriageSummary::default(),
        );
        assert!(matches!(missing, Err(ProductStoreError::NotFound { .. })));
    }

    /// 批 2 T1（REQ-WIGA-09）：ensure_provider_run_allowed 对精确绑定已
    /// terminated 的 plan 拒绝 provider 驱动——无 CodingPlanRun 也关闸（准入
    /// 以 enrollment terminated 事实为准）；manual 无绑定 attempt 不拦。
    #[test]
    fn ensure_provider_run_allowed_rejects_terminated_plan_even_without_run() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_group_attempt(CreateGroupCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                current_work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: Some(ProviderName::Fake),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
                start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            })
            .unwrap();
        assert_eq!(
            attempt.work_item_group_id.as_deref(),
            Some("work_item_plan_0001")
        );
        // 生产中 group attempt 创建时 plan lineage 已由 plan 流程落盘；
        // ensure_provider_run_allowed 的 plan-repair reconcile 需读它
        //（linked_active_plan_repair_snapshot → get_plan_lineage）。
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(paths.clone())
            .put_plan_lineage(
                &crate::product::models::work_item_revision::WorkItemPlanLineage {
                    id: "work_item_plan_0001".to_string(),
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    story_spec_refs: Vec::new(),
                    design_spec_refs: Vec::new(),
                    active_revision_id: None,
                    active_amendment_id: None,
                    created_at: "2026-10-16T00:00:00Z".to_string(),
                    updated_at: "2026-10-16T00:00:00Z".to_string(),
                },
            )
            .unwrap();

        // manual：无 enrollment 绑定，ensure_provider_run_allowed 不拦（回归）。
        store
            .ensure_provider_run_allowed(&attempt)
            .expect("manual attempt without enrollment is not gated");

        // 绑定该 plan 的 enrollment 终止后：无 run 文件也拒绝 provider 驱动。
        let enrollment_store =
            crate::product::issue_automation_store::IssueAutomationStore::new(paths.clone());
        let enrollment_id = {
            let created = enrollment_store
                .compare_and_set(
                    "project_0001",
                    "issue_0001",
                    None,
                    crate::product::models::automation::EnrollmentWriteCommand::Enable {
                        selection_key: "human-choice-1".to_string(),
                        source: automation_source(),
                        options: automation_options(),
                        target: automation_target(),
                    },
                )
                .unwrap();
            let bound = enrollment_store
                .bind_plan(
                    "project_0001",
                    "issue_0001",
                    created.policy_revision,
                    "work_item_plan_0001",
                    "session_0001",
                )
                .unwrap();
            bound.enrollment_id
        };
        enrollment_store
            .terminate_plan(
                "project_0001",
                "issue_0001",
                crate::product::models::automation::EnrollmentTerminationRequest {
                    command_id: "terminate-command-1".to_string(),
                    expected_enrollment_id: enrollment_id,
                    expected_policy_revision: 2,
                    reason: "user requested termination".to_string(),
                },
            )
            .unwrap();

        let error = store.ensure_provider_run_allowed(&attempt).unwrap_err();
        assert!(
            error.to_string().contains("automation_plan_terminated"),
            "unexpected error: {error}"
        );
    }

    fn automation_source() -> crate::product::models::automation::EnrollmentSource {
        crate::product::models::automation::EnrollmentSource {
            stories: vec![crate::product::models::automation::SourceRevisionRef {
                id: "story_spec_0001".into(),
                version: 1,
            }],
            designs: vec![],
        }
    }

    fn automation_options() -> crate::product::models::automation::EnrollmentOptions {
        crate::product::models::automation::EnrollmentOptions {
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            plan_approval: crate::product::models::automation::PlanApprovalPolicy::Manual,
            plan_options: crate::product::models::lifecycle::IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
        }
    }

    fn automation_target() -> crate::product::logical_codebase::EnrollmentTarget {
        use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
        EnrollmentTarget::LogicalCodebase {
            logical_codebase_id: "logical_codebase_0001".into(),
            logical_repository_id: LogicalRepositoryId(uuid::Uuid::new_v4()),
        }
    }

    /// Bug2-A(Task 3 Step 2):统一关闭原语的四种崩溃前缀收敛——
    /// 根已关归档未完→补齐;归档完根仍在→清理;都全→幂等;都不存在→NotFound。
    #[test]
    fn resolve_blocked_gate_converges_crash_prefixes_and_preserves_first_close() {
        let (_tmp, store, attempt) = fixture_attempt();
        let gate = store
            .create_blocked_gate(&attempt, interrupted_gate_input(&attempt))
            .unwrap();
        let project_id = attempt.project_id.as_str();
        let issue_id = attempt.issue_id.as_str();
        let attempt_id = attempt.id.as_str();
        let gates_root = store.blocked_gates_root(project_id, issue_id, attempt_id);
        let root_path = gates_root.join(format!("{}.json", gate.gate_id));
        let archive_path = gates_root
            .join("resolved")
            .join(format!("{}.json", gate.gate_id));
        let read_json_at = |path: &std::path::Path| {
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(path).unwrap())
                .unwrap()
        };

        // 前缀一:根已写完整 resolved 事实、归档未写(崩溃于根写之后)——
        // 收敛必须以根记录的首关事实补齐归档,不重写时间。
        {
            let mut record = read_json_at(&root_path);
            record["status"] = serde_json::json!("resolved");
            record["resolved_action"] = serde_json::json!("retry_review");
            record["resolved_at"] = serde_json::json!("2026-10-17T00:00:00Z");
            std::fs::write(&root_path, serde_json::to_string_pretty(&record).unwrap()).unwrap();
            store
                .resolve_blocked_gate_with_action(
                    project_id,
                    issue_id,
                    attempt_id,
                    &gate.gate_id,
                    Some("retry_review"),
                )
                .expect("complete archive from resolved root prefix");
            let archived = read_json_at(&archive_path);
            assert_eq!(archived["status"], serde_json::json!("resolved"));
            assert_eq!(
                archived["resolved_action"],
                serde_json::json!("retry_review")
            );
            assert_eq!(
                archived["resolved_at"],
                serde_json::json!("2026-10-17T00:00:00Z"),
                "first-close facts must be preserved, not rewritten"
            );
            assert!(!root_path.exists(), "root record removed after convergence");
        }

        // 前缀二:归档完、根仍在(崩溃于删根之前)——清理根记录,首关事实不变。
        {
            std::fs::copy(&archive_path, &root_path).unwrap();
            store
                .resolve_blocked_gate_with_action(
                    project_id,
                    issue_id,
                    attempt_id,
                    &gate.gate_id,
                    Some("retry_review"),
                )
                .expect("cleanup leftover root with complete archive");
            let archived = read_json_at(&archive_path);
            assert_eq!(
                archived["resolved_at"],
                serde_json::json!("2026-10-17T00:00:00Z"),
                "archive wins over leftover root"
            );
            assert!(!root_path.exists());
        }

        // 前缀三:都全(仅归档)——同动作重放幂等返回首关结果,异动作冲突不覆盖。
        {
            store
                .resolve_blocked_gate_with_action(
                    project_id,
                    issue_id,
                    attempt_id,
                    &gate.gate_id,
                    Some("retry_review"),
                )
                .expect("idempotent replay returns first-close outcome");
            let archived = read_json_at(&archive_path);
            assert_eq!(
                archived["resolved_at"],
                serde_json::json!("2026-10-17T00:00:00Z")
            );
            let conflict = store
                .resolve_blocked_gate_with_action(
                    project_id,
                    issue_id,
                    attempt_id,
                    &gate.gate_id,
                    Some("abort"),
                )
                .expect_err("different action must not overwrite the audit");
            assert!(
                conflict.to_string().contains("product_store_conflict"),
                "{conflict}"
            );
            let after = read_json_at(&archive_path);
            assert_eq!(after["resolved_action"], serde_json::json!("retry_review"));
        }

        // 前缀四:都不存在——NotFound,零写入。
        {
            let missing = store
                .resolve_blocked_gate_with_action(
                    project_id,
                    issue_id,
                    attempt_id,
                    "coding_blocked_gate_0999",
                    Some("retry_review"),
                )
                .expect_err("missing gate must be NotFound");
            assert!(
                missing.to_string().contains("product_store_not_found"),
                "{missing}"
            );
        }
    }

    fn interrupted_gate_input(attempt: &CodingExecutionAttempt) -> CreateBlockedGateInput {
        CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            node_id: None,
            role: Some(CodingProviderRole::CodeReviewer),
            title: "代码审查中断".to_string(),
            description: "provider interrupted".to_string(),
            reason_code: Some("code_review_provider_interrupted".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
            available_actions: Vec::new(),
        }
    }
}
