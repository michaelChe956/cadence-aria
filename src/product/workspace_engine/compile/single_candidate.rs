use super::*;

#[cfg(test)]
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SingleCandidateCompileCheckpoint {
    ApprovalPersisted,
    ReservationPersisted,
    ProvenancePersisted,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SingleCandidateCompileFailpointKey {
    session_id: String,
    project_id: String,
    issue_id: String,
    plan_id: String,
    checkpoint: SingleCandidateCompileCheckpoint,
}

#[cfg(test)]
pub(crate) struct SingleCandidateCompileFailpointGuard {
    key: SingleCandidateCompileFailpointKey,
    registration_id: u64,
}

#[cfg(test)]
static SINGLE_CANDIDATE_COMPILE_FAILPOINTS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<SingleCandidateCompileFailpointKey, u64>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
static NEXT_SINGLE_CANDIDATE_COMPILE_FAILPOINT_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// SG3b（REQ-SG-03、REQ-PLN-09）：Final Compile 冻结结构上下文复验的纯
/// 判定。Some 冻结 payload 与当前 live context 必须同代——rule_version 或
/// context_digest 漂移均为 typed Operational 拒绝（拒不重 lower 最新
/// artifact 偷换发布物）；None（旧记录参考形态）不做冻结 digest 要求，
/// 同源纯评估（assess）另在调用侧执行。
// PlanningCompileFailure 承载完整 typed 失败上下文(诊断/seal),按值返回是契约。
#[allow(clippy::result_large_err)]
pub(super) fn verify_frozen_structure_context(
    frozen: Option<&crate::product::planning_failure::PersistedPlanningStructureReport>,
    current: &draft_batch::compile_support::PlanningStructureContext,
) -> Result<(), crate::product::workspace_engine::types::PlanningCompileFailure> {
    let Some(frozen) = frozen else {
        return Ok(());
    };
    let current_rule =
        crate::product::workspace_engine::plan_projection::PLANNING_STRUCTURE_RULE_VERSION;
    if frozen.rule_version != current_rule {
        return Err(
            crate::product::workspace_engine::types::PlanningCompileFailure::operational(
                "single_candidate_structure_rule_stale",
                format!(
                    "frozen rule_version {} != current {current_rule}",
                    frozen.rule_version
                ),
            ),
        );
    }
    if frozen.context_digest != current.context_digest {
        return Err(
            crate::product::workspace_engine::types::PlanningCompileFailure::operational(
                "single_candidate_structure_context_drift",
                format!(
                    "frozen context_digest {} != current {}",
                    frozen.context_digest, current.context_digest
                ),
            ),
        );
    }
    Ok(())
}

impl WorkspaceEngine {
    #[cfg(test)]
    pub(crate) fn register_single_candidate_compile_failpoint(
        &self,
        checkpoint: SingleCandidateCompileCheckpoint,
    ) -> SingleCandidateCompileFailpointGuard {
        let key = SingleCandidateCompileFailpointKey {
            session_id: self.session.session_id.clone(),
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
            checkpoint,
        };
        let registration_id = NEXT_SINGLE_CANDIDATE_COMPILE_FAILPOINT_ID
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let previous = single_candidate_compile_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.clone(), registration_id);
        assert!(
            previous.is_none(),
            "single candidate compile failpoint already registered"
        );
        SingleCandidateCompileFailpointGuard {
            key,
            registration_id,
        }
    }

    #[cfg(test)]
    fn maybe_fail_single_candidate_compile(
        &self,
        checkpoint: SingleCandidateCompileCheckpoint,
    ) -> Result<(), String> {
        let key = SingleCandidateCompileFailpointKey {
            session_id: self.session.session_id.clone(),
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
            checkpoint,
        };
        // D2.1b 探针：SC initial 各 checkpoint 位点（与 failpoint 同位，
        // 无 failpoint 注册也触发）。
        probe_work_item_plan_compile_admission(
            &key.project_id,
            &key.issue_id,
            &key.plan_id,
            &format!("{:?}", key.checkpoint),
        );
        if single_candidate_compile_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&key)
            .is_some()
        {
            return Err(format!("single_candidate_compile_failpoint:{checkpoint:?}"));
        }
        Ok(())
    }

    /// SG3b：SC Final Compile 的 typed 入口——inner 产
    /// `PlanningCompileFailure`，本边界统一 seal（内容寻址 failure_id +
    /// approval/source refs）、缓存 typed 失败供本次 close 的
    /// WS/REST 投影消费（`close_human_gate` 开始时清空，无跨 close 陈旧）。
    /// 未达 seal 的失败（plain `?` 站点）在此兜底 seal；diagnostic 落点
    /// 沿 `sg3b_failure` 站点原语义（非 closure 站点原本不落诊断，保持）。
    #[allow(clippy::result_large_err)] // 同上:typed 失败上下文按值上抛。
    pub(super) async fn run_single_candidate_initial_plan_compile(
        &mut self,
    ) -> Result<
        InitialPlanCompileOutcome,
        crate::product::workspace_engine::types::PlanningCompileFailure,
    > {
        let result = self.run_single_candidate_initial_plan_compile_inner().await;
        match result {
            Ok(outcome) => {
                // REQ-WSC-18：成功尝试覆盖候选编译事实——清除同候选的
                // 「编译不过」残留，否则一次瞬态失败恢复后确认动作会被
                // 过期失败永久挡死（新的死路）。
                self.record_candidate_compile_success();
                Ok(outcome)
            }
            Err(mut failure) => {
                if failure.failure_id.is_empty() {
                    let identity = self
                        .lifecycle_store
                        .clone()
                        .and_then(|lifecycle| {
                            lifecycle
                                .get_workspace_session(&self.session.session_id)
                                .ok()
                        })
                        .map(|record| {
                            (
                                record.approval_attempt_id,
                                record.work_item_plan_source_revision_ref,
                                record.plan_candidate_ir_ref,
                                record.mechanical_report_ref,
                            )
                        });
                    match identity {
                        Some((approval, source_ref, ir_ref, report_ref)) => {
                            failure.seal_with_identity(
                                &self.session.session_id,
                                approval,
                                source_ref,
                                ir_ref,
                                report_ref,
                            );
                        }
                        None => failure.seal_with_identity(
                            &self.session.session_id,
                            None,
                            None,
                            None,
                            None,
                        ),
                    }
                }
                self.last_gate_close_compile_failure = Some(
                    crate::product::workspace_engine::conversational_gate::HumanGateCloseCompileFailure {
                        failure: failure.clone(),
                        validator_findings: Vec::new(),
                    },
                );
                // REQ-WSC-18：完整诊断 durable 落盘（与门卡同族落点）——
                // 现场此前只有 timeline summary 与内存 stash，owner 在门卡上
                // 读不到任何报错内容；plain `?` 站点（如
                // `final_compile_prepare_failed`）原本连 policy_diagnostics
                // 都不落。这里是 SC initial compile 的唯一返回边界，失败与
                // 成功两侧都在此收口，无跨路径遗漏。
                // REQ-WSC-21/22：路由裁定（自动重写 / 提前停）随 durable 事实
                // 一并落盘并随门卡投影；本边界不改写既有失败返回语义。
                let _route = self.record_candidate_compile_failure(&failure);
                Err(failure)
            }
        }
    }

    /// REQ-WSC-18/21/22：把 typed 失败完整落成 durable 候选编译事实（不截断），
    /// 并返回自动重写路由裁定。写失败只降级为可观测静默——不得因诊断落盘失败
    /// 改写既有失败语义。
    /// REQ-WSC-25：同时向 append-only 尝试流水追加一行（历史序列，与上面的
    /// 「当前状态」分工）。
    pub(crate) fn record_candidate_compile_failure(
        &mut self,
        failure: &crate::product::workspace_engine::types::PlanningCompileFailure,
    ) -> crate::product::workspace_engine::compile_failure_surface::CandidateCompileRoute {
        use crate::product::workspace_engine::compile_failure_surface::CandidateCompileRoute;
        let Some(lifecycle) = self.lifecycle_store.clone() else {
            return CandidateCompileRoute::Gate;
        };
        let previous = lifecycle
            .load_candidate_compile_status(&self.session.session_id)
            .ok()
            .flatten();
        let now = chrono::Utc::now().to_rfc3339();
        // 工作包 1.5：余额事实源 = 统一权威余额（durable 读侧组装，单一来源）。
        let (status, route) =
            crate::product::workspace_engine::compile_failure_surface::candidate_compile_status(
                previous.as_ref(),
                failure,
                now.clone(),
                self.author_rewrite_balance(),
            );
        let _ = lifecycle.save_candidate_compile_status(&self.session.session_id, &status);
        // REQ-WSC-25：追加写尝试流水（历史序列；与上面的「当前状态」分工）。
        let attempt = crate::product::lifecycle_store::candidate_attempts::
            candidate_attempt_record_from_failure(failure, &status, now);
        self.append_candidate_attempt(&lifecycle, attempt);
        route
    }

    /// 工作包 1.5（Task 4）：author 重写**单一权威余额**的读侧组装——全部
    /// 执法点（`candidate_compile_status` 供参、`rescue_platform_compile_
    /// failure_repair` 守卫、三处门快照构造）MUST 经此同一事实源。事实源
    /// 一律 **durable 记录**（Task 3 评审 Minor-3：reconcile 退款只写
    /// durable、不刷 engine 内存镜像 ⇒ 严禁读 `self.session.author_rewrite_
    /// budget` 镜像，否则少算可用额度）。`None` = 余额未知 ⇒ 调用方保守处理。
    pub(crate) fn author_rewrite_balance(&self) -> Option<u32> {
        let store = self.lifecycle_store.as_ref()?;
        let record = store.get_workspace_session(&self.session.session_id).ok()?;
        crate::product::lifecycle_store::workspace_policy_route::author_rewrite_balance(
            store,
            &record,
            crate::product::workspace_engine::compile_failure_surface::max_auto_rewrites(),
        )
    }

    /// REQ-WSC-18：编译成功 → 以「无失败」覆盖同一会话的候选编译事实，避免
    /// 过期失败把确认动作永久挡死。
    /// REQ-WSC-25：同时向 append-only 尝试流水追加一行（成功也进历史序列，
    /// 否则首次通过率无从计算）。
    fn record_candidate_compile_success(&mut self) {
        let Some(lifecycle) = self.lifecycle_store.clone() else {
            return;
        };
        let Ok(record) = lifecycle.get_workspace_session(&self.session.session_id) else {
            return;
        };
        // Task 5（工作包 4.1/4.2）：编译成功 ⇒ 只清该候选编译链的预算域自动
        // 段，且**授权事实先落**（clear CAS 在诊断计数面之前）。两写非原子
        // （session 记录 / candidate compile status 各一文件）⇒ 崩溃窗内只留
        // 「诊断计数陈旧」（auto_rewrites_used 未随成功归零），不是授权事实
        // 失真——下次编译失败的读侧以预算域（统一权威余额）为准，诊断计数随
        // 下次成功落盘对齐。写失败只降级为静默（与下方 save 同口径）。
        let _ = lifecycle.clear_author_rewrite_auto_segment(&self.session.session_id);
        let now = chrono::Utc::now().to_rfc3339();
        let status =
            crate::product::workspace_engine::compile_failure_surface::cleared_candidate_compile_status(
                record.work_item_plan_source_revision_ref.clone(),
                record.plan_candidate_ir_ref.clone(),
                now.clone(),
            );
        let _ = lifecycle.save_candidate_compile_status(&self.session.session_id, &status);
        // REQ-WSC-25：成功尝试也进历史序列（否则首次通过率无从计算）。
        let attempt = crate::product::lifecycle_store::candidate_attempts::
            candidate_attempt_record_from_success(
                &status,
                record
                    .compile_reservation
                    .as_ref()
                    .map(|reservation| reservation.compile_id.as_str()),
                now,
            );
        self.append_candidate_attempt(&lifecycle, attempt);
    }

    /// REQ-WSC-25：把本次尝试追加进 append-only 流水
    /// （`issues/{issue}/candidate-attempts.jsonl`——issue 级落点：会话目录
    /// 会被「终止+重开」删掉，流水必须活过它）。
    ///
    /// 写失败只降级为静默：错误记入运行日志（policy diagnostics），**不得**因
    /// 此使候选生成或确认失败（REQ-WSC-25 场景 2）。
    fn append_candidate_attempt(
        &mut self,
        lifecycle: &LifecycleStore,
        attempt: crate::product::lifecycle_store::candidate_attempts::CandidateAttemptRecord,
    ) {
        if let Err(error) = lifecycle.append_candidate_attempt(&self.session.session_id, &attempt) {
            self.record_single_candidate_compile_failure(
                lifecycle,
                "candidate_attempt_log_write_failed",
                &format!("append candidate attempt log failed: {error}"),
            );
        }
    }

    /// SG3b typed 失败站点：构造（携 run/digest 身份）→ seal → durable
    /// diagnostic（failure_id + 渲染）→ 返回 typed 失败。替代旧
    /// `failure(engine, code, message) -> String` closure 的单点等价物。
    fn sg3b_failure(
        &mut self,
        lifecycle: &LifecycleStore,
        code: &str,
        message: String,
        compile_run_id: Option<&str>,
        context_digest: Option<&str>,
    ) -> crate::product::workspace_engine::types::PlanningCompileFailure {
        let mut failure =
            crate::product::workspace_engine::types::PlanningCompileFailure::operational(
                code, message,
            );
        failure.compile_run_id = compile_run_id.map(str::to_string);
        failure.context_digest = context_digest.map(str::to_string);
        self.sg3b_seal_with_diagnostic(lifecycle, failure)
    }

    /// SG3b：seal（durable record 身份）+ durable diagnostic 落盘的单源
    /// helper——Operational 与 Structure 两类 gate 失败共用。
    fn sg3b_seal_with_diagnostic(
        &mut self,
        lifecycle: &LifecycleStore,
        mut failure: crate::product::workspace_engine::types::PlanningCompileFailure,
    ) -> crate::product::workspace_engine::types::PlanningCompileFailure {
        match lifecycle.get_workspace_session(&self.session.session_id) {
            Ok(record) => failure.seal(&self.session.session_id, &record),
            Err(_) => failure.seal_with_identity(&self.session.session_id, None, None, None, None),
        }
        let diagnostic = format!("failure_id={}; {}", failure.failure_id, failure.render());
        self.record_single_candidate_compile_failure(lifecycle, failure.stable_code(), &diagnostic);
        failure
    }

    /// SG3b：Final Compile 结构复验门（initial 与 recovery resume 共用，
    /// 与候选门同源：同一 `build_single_candidate_structure_context` 装载 +
    /// SG1 纯评估 + 冻结 payload rule/context digest 同代检查）。返回 live
    /// context digest；非 Accepted 返回 typed 失败（未 seal——由调用边界统
    /// 一 seal/落诊断）。None 冻结 payload（旧记录参考形态）只做纯评估。
    #[allow(clippy::result_large_err)] // 同上:typed 失败上下文按值上抛。
    fn sg3b_final_compile_structure_gate(
        &self,
        _lifecycle: &LifecycleStore,
        ir: &crate::product::work_item_plan_compiler::PlanCandidateIr,
        source_markdown: &str,
        source_ref: &str,
        ir_ref: &str,
        frozen: Option<&crate::product::planning_failure::PersistedPlanningStructureReport>,
    ) -> Result<String, crate::product::workspace_engine::types::PlanningCompileFailure> {
        use crate::product::workspace_engine::types::PlanningCompileFailure;
        let structure_context = self
            .build_single_candidate_structure_context(ir, source_ref, ir_ref)
            .map_err(|cause| {
                PlanningCompileFailure::operational(
                    "single_candidate_structure_context_failed",
                    format!("single-candidate structure context unavailable: {cause}"),
                )
            })?;
        verify_frozen_structure_context(frozen, &structure_context).map_err(|mut failure| {
            failure.context_digest = Some(structure_context.context_digest.clone());
            failure
        })?;
        let assessment =
            crate::product::workspace_engine::plan_projection::assess_single_candidate_structure(
                source_markdown.to_string(),
                ir,
                &structure_context,
            );
        match assessment {
            crate::product::workspace_engine::types::StructuralAssessment::Accepted(_) => {
                Ok(structure_context.context_digest)
            }
            crate::product::workspace_engine::types::StructuralAssessment::Rejected(set, _) => {
                let mut failure = PlanningCompileFailure::structure(set);
                failure.context_digest = Some(structure_context.context_digest.clone());
                Err(failure)
            }
            other => {
                let rendered =
                    crate::product::workspace_engine::plan_projection::structural_assessment_stop_reason(&other)
                        .expect_err("non-accepted assessment must produce a typed stop");
                let mut failure = PlanningCompileFailure::operational(
                    "single_candidate_structure_operational",
                    rendered,
                );
                failure.context_digest = Some(structure_context.context_digest.clone());
                Err(failure)
            }
        }
    }

    #[allow(clippy::result_large_err)] // 同上:typed 失败上下文按值上抛。
    async fn run_single_candidate_initial_plan_compile_inner(
        &mut self,
    ) -> Result<
        InitialPlanCompileOutcome,
        crate::product::workspace_engine::types::PlanningCompileFailure,
    > {
        use crate::product::workspace_engine::types::PlanningCompileFailure;
        let lifecycle = self.lifecycle_store.clone().ok_or_else(|| {
            PlanningCompileFailure::operational(
                "lifecycle_store_unavailable",
                "lifecycle_store unavailable".to_string(),
            )
        })?;
        // SG3b：本运行的 compile 身份与结构门 digest（结构门通过后填充，
        // 供 post-reservation 失败站点随失败上行）。
        let record = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                PlanningCompileFailure::operational(
                    "single_candidate_session_load_failed",
                    format!("load single candidate session failed: {error}"),
                )
            })?;
        if record.project_id != self.session.project_id
            || record.issue_id != self.session.issue_id
            || record.entity_id != self.session.entity_id
            || record.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || record.single_candidate_phase
                != Some(crate::product::models::SingleCandidatePhase::Approval)
        {
            return Err(self.sg3b_failure(
                &lifecycle,
                "single_candidate_invalid_session",
                "single candidate compile session is not in Approval for this scope".to_string(),
                None,
                None,
            ));
        }
        let scope = SourceStoreScope {
            project_id: record.project_id.clone(),
            issue_id: record.issue_id.clone(),
            plan_id: record.entity_id.clone(),
        };
        let source_ref = record
            .work_item_plan_source_revision_ref
            .clone()
            .ok_or_else(|| {
                self.sg3b_failure(
                    &lifecycle,
                    "SOURCE_STORE_MALFORMED_REF",
                    "single candidate source revision ref is missing".to_string(),
                    None,
                    None,
                )
            })?;
        let ir_ref = record.plan_candidate_ir_ref.clone().ok_or_else(|| {
            self.sg3b_failure(
                &lifecycle,
                "SOURCE_STORE_MALFORMED_REF",
                "single candidate IR ref is missing".to_string(),
                None,
                None,
            )
        })?;
        let report_ref = record.mechanical_report_ref.clone().ok_or_else(|| {
            self.sg3b_failure(
                &lifecycle,
                "SOURCE_STORE_MALFORMED_REF",
                "single candidate mechanical report ref is missing".to_string(),
                None,
                None,
            )
        })?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source = source_store
            .get_source_revision(&scope, &source_ref)
            .map_err(|error| {
                self.sg3b_failure(
                    &lifecycle,
                    error.code(),
                    format!("single candidate source reload failed: {error:?}"),
                    None,
                    None,
                )
            })?;
        let ir = source_store
            .get_plan_candidate_ir(&scope, &ir_ref)
            .map_err(|error| {
                self.sg3b_failure(
                    &lifecycle,
                    error.code(),
                    format!("single candidate IR reload failed: {error:?}"),
                    None,
                    None,
                )
            })?;
        let report = source_store
            .get_mechanical_report(&scope, &report_ref)
            .map_err(|error| {
                self.sg3b_failure(
                    &lifecycle,
                    error.code(),
                    format!("single candidate mechanical report reload failed: {error:?}"),
                    None,
                    None,
                )
            })?;
        verify_publish_freshness(&source.source, &ir.ir, &report.report).map_err(|error| {
            self.sg3b_failure(
                &lifecycle,
                "single_candidate_freshness_failed",
                format!("single candidate publish freshness failed: {error:?}"),
                None,
                None,
            )
        })?;

        let approval_attempt_id = single_candidate_approval_attempt_id(
            &record.id,
            &record.entity_id,
            &source_ref,
            &ir_ref,
            &report_ref,
        );
        let (approved_at, approved) = match (
            record.approval_attempt_id.as_deref(),
            record.approved_at.as_deref(),
        ) {
            (None, None) => {
                let approved_at = chrono::Utc::now().to_rfc3339();
                let approved = lifecycle
                    .compare_and_save_single_candidate_approval(
                        &record,
                        &approval_attempt_id,
                        &approved_at,
                    )
                    .map_err(|error| {
                        self.sg3b_failure(
                            &lifecycle,
                            "persistence_failure",
                            format!("save single candidate Approval failed: {error}"),
                            None,
                            None,
                        )
                    })?;
                (approved_at, approved)
            }
            (Some(existing_id), Some(existing_at))
                if existing_id == approval_attempt_id
                    && record.approval_attempt_id.as_deref()
                        == Some(approval_attempt_id.as_str()) =>
            {
                (existing_at.to_string(), record)
            }
            _ => {
                return Err(self.sg3b_failure(
                    &lifecycle,
                    "single_candidate_approval_conflict",
                    "single candidate Approval tuple conflicts with current durable refs"
                        .to_string(),
                    None,
                    None,
                ));
            }
        };
        #[cfg(test)]
        self.maybe_fail_single_candidate_compile(
            SingleCandidateCompileCheckpoint::ApprovalPersisted,
        )
        .map_err(|error| {
            PlanningCompileFailure::operational("single_candidate_compile_failpoint", error)
        })?;

        // SG3b 结构复验门（REQ-SG-03、REQ-PLN-09，plan §0.6.2 同源）：与候选
        // 门同一 context 装载 + SG1 纯评估 + 冻结 payload 同代检查；非
        // Accepted 一律 typed 停点（seal + durable diagnostic），零
        // reservation、零 transaction、零发布，拒不重 lower。
        let gate_context_digest = Some(
            self.sg3b_final_compile_structure_gate(
                &lifecycle,
                &ir.ir,
                &source.source,
                &source_ref,
                &ir_ref,
                report.structure_report.as_ref(),
            )
            .map_err(|failure| self.sg3b_seal_with_diagnostic(&lifecycle, failure))?,
        );

        let reservation = crate::product::models::SingleCandidateCompileReservation {
            compile_id: single_candidate_compile_id(
                &approved.id,
                &approved.entity_id,
                &approval_attempt_id,
                &approved_at,
            ),
            now: approved_at.to_string(),
            publication_provenance_ref: String::new(),
        };
        let mut reservation = reservation;
        reservation.publication_provenance_ref = format!(
            "project/{}/issue/{}/plan/{}/publication_provenance/{}",
            approved.project_id, approved.issue_id, approved.entity_id, reservation.compile_id
        );
        let compile_run_id = Some(reservation.compile_id.clone());
        let reserved = lifecycle
            .put_compile_reservation_cas(
                &approved.project_id,
                &approved.issue_id,
                &approved.entity_id,
                &approved.id,
                &approved,
                &reservation,
            )
            .map_err(|error| match error {
                CompileReservationError::Conflict => self.sg3b_failure(
                    &lifecycle,
                    "SINGLE_CANDIDATE_COMPILE_RESERVATION_CONFLICT",
                    error.to_string(),
                    compile_run_id.as_deref(),
                    gate_context_digest.as_deref(),
                ),
                CompileReservationError::InvalidSession(message) => self.sg3b_failure(
                    &lifecycle,
                    "SINGLE_CANDIDATE_COMPILE_RESERVATION_INVALID_SESSION",
                    message,
                    compile_run_id.as_deref(),
                    gate_context_digest.as_deref(),
                ),
                CompileReservationError::PersistenceFailure(error) => self.sg3b_failure(
                    &lifecycle,
                    "persistence_failure",
                    format!("save compile reservation failed: {error}"),
                    compile_run_id.as_deref(),
                    gate_context_digest.as_deref(),
                ),
            })?;
        #[cfg(test)]
        self.maybe_fail_single_candidate_compile(
            SingleCandidateCompileCheckpoint::ReservationPersisted,
        )
        .map_err(|error| {
            PlanningCompileFailure::operational("single_candidate_compile_failpoint", error)
        })?;
        self.session.approval_attempt_id = reserved.approval_attempt_id.clone();
        self.session.approved_at = reserved.approved_at.clone();
        self.session.compile_reservation = reserved.compile_reservation.clone();
        self.session.publication_provenance_ref = reserved.publication_provenance_ref.clone();
        let reservation = reserved.compile_reservation.as_ref().ok_or_else(|| {
            PlanningCompileFailure::operational(
                "persistence_failure",
                "compile reservation disappeared after CAS".to_string(),
            )
        })?;
        let previous_plan = lifecycle
            .get_issue_work_item_plan(&scope.project_id, &scope.issue_id, &scope.plan_id)
            .map_err(|error| {
                PlanningCompileFailure::operational(
                    "single_candidate_plan_load_failed",
                    format!("load plan for single candidate compile failed: {error}"),
                )
            })?;
        let logical_targets = self
            .logical_work_item_plan_repository_targets(&lifecycle, &previous_plan)
            .map_err(|error| {
                self.sg3b_failure(
                    &lifecycle,
                    "single_candidate_target_failed",
                    error,
                    compile_run_id.as_deref(),
                    gate_context_digest.as_deref(),
                )
            })?;
        let repository_id = if logical_targets.is_none() {
            ir.ir
                .items
                .first()
                .map(|item| item.target_repository_id.clone())
                .ok_or_else(|| {
                    PlanningCompileFailure::operational(
                        "single_candidate_ir_empty",
                        "single candidate IR contains no items".to_string(),
                    )
                })?
        } else {
            String::new()
        };
        let change_order = draft_batch::compile_support::load_change_order_from_confirmed_design(
            &lifecycle,
            &previous_plan,
        )
        .map_err(|error| {
            self.sg3b_failure(
                &lifecycle,
                "single_candidate_change_order_failed",
                error,
                compile_run_id.as_deref(),
                gate_context_digest.as_deref(),
            )
        })?;
        let context = ir_adapter::IrCompileAdapterContext {
            project_id: scope.project_id.clone(),
            issue_id: scope.issue_id.clone(),
            plan_id: scope.plan_id.clone(),
            previous_plan: previous_plan.clone(),
            source_revision_id: source.id.clone(),
            source_revision_ref: source_ref.clone(),
            plan_candidate_ir_ref: ir_ref.clone(),
            mechanical_report_ref: report_ref.clone(),
            publication_provenance_ref: reservation.publication_provenance_ref.clone(),
            logical_targets,
            repository_id,
            change_order,
            compile_id: reservation.compile_id.clone(),
            now: reservation.now.clone(),
        };
        let logical_ids = ir
            .ir
            .items
            .iter()
            .map(|item| item.contract.identity.logical_work_item_id.clone())
            .collect::<Vec<_>>();
        let allocated_ids =
            crate::product::work_item_revision_store::allocate_initial_plan_publication_ids(
                &scope.project_id,
                &scope.issue_id,
                &scope.plan_id,
                &reservation.compile_id,
                &logical_ids,
            )
            .map_err(|error| {
                self.sg3b_failure(
                    &lifecycle,
                    "single_candidate_publication_ids_failed",
                    error.to_string(),
                    compile_run_id.as_deref(),
                    gate_context_digest.as_deref(),
                )
            })?;
        let mut provenance = PlanCandidatePublicationProvenance {
            id: reservation.compile_id.clone(),
            plan_id: scope.plan_id.clone(),
            plan_revision_id: allocated_ids.plan_revision_id,
            source_revision_ref: source_ref,
            plan_candidate_ir_ref: ir_ref,
            mechanical_report_ref: report_ref,
            source_revision_hash: source.source_revision_hash,
            compiler_version: ir.ir.compiler_version.clone(),
            published_at: reservation.now.clone(),
            content_hash: String::new(),
        };
        provenance.content_hash = provenance.content_hash().map_err(|error| {
            self.sg3b_failure(
                &lifecycle,
                error.code(),
                format!("hash provenance failed: {error:?}"),
                compile_run_id.as_deref(),
                gate_context_digest.as_deref(),
            )
        })?;
        let provenance_ref = source_store
            .put_publication_provenance(
                &scope.project_id,
                &scope.issue_id,
                &scope.plan_id,
                &provenance,
            )
            .map_err(|error| {
                self.sg3b_failure(
                    &lifecycle,
                    error.code(),
                    format!("persist provenance failed: {error:?}"),
                    compile_run_id.as_deref(),
                    gate_context_digest.as_deref(),
                )
            })?;
        if provenance_ref != reservation.publication_provenance_ref {
            return Err(self.sg3b_failure(
                &lifecycle,
                "single_candidate_provenance_ref_mismatch",
                "provenance canonical ref does not match reservation".to_string(),
                compile_run_id.as_deref(),
                gate_context_digest.as_deref(),
            ));
        }
        let provenance = source_store
            .get_publication_provenance(&scope, &provenance_ref)
            .map_err(|error| {
                self.sg3b_failure(
                    &lifecycle,
                    error.code(),
                    format!("reload provenance failed: {error:?}"),
                    compile_run_id.as_deref(),
                    gate_context_digest.as_deref(),
                )
            })?;
        #[cfg(test)]
        self.maybe_fail_single_candidate_compile(
            SingleCandidateCompileCheckpoint::ProvenancePersisted,
        )
        .map_err(|error| {
            PlanningCompileFailure::operational("single_candidate_compile_failpoint", error)
        })?;
        let durable_context = ir_adapter::durable_compile_context_from_ir(&context, &provenance)
            .map_err(|cause| {
                PlanningCompileFailure::operational(
                    "single_candidate_durable_context_failed",
                    cause,
                )
            })?;
        let input =
            ir_adapter::initial_plan_compile_input_from_ir(&context, &ir.ir, &report.report)
                .map_err(|cause| {
                    PlanningCompileFailure::operational("single_candidate_ir_adapter_failed", cause)
                })?;
        let prepared = prepare_initial_plan_compile(input, durable_context)
            .map_err(prepare_failure::into_compile_failure)?;
        let stores = CompileStores {
            plan_store: self.work_item_plan_store().map_err(|cause| {
                PlanningCompileFailure::operational("plan_store_unavailable", cause)
            })?,
            revision_store: self.revision_store(),
        };
        let (outcome, mut tx) =
            execute_initial_plan_compile(&stores, prepared).map_err(|cause| {
                let mut failure =
                    PlanningCompileFailure::operational("final_compile_execute_failed", cause);
                failure.compile_run_id = compile_run_id.clone();
                failure.context_digest = gate_context_digest.clone();
                failure
            })?;
        self.finalize_initial_plan_compile(&lifecycle, &stores.plan_store, &mut tx, &outcome)
            .await
            .map_err(|cause| {
                let mut failure =
                    PlanningCompileFailure::operational("final_compile_finalize_failed", cause);
                failure.compile_run_id = compile_run_id.clone();
                failure.context_digest = gate_context_digest.clone();
                failure
            })?;
        Ok(outcome)
    }

    pub(super) fn resume_single_candidate_compile_transaction(
        &mut self,
        store: &WorkItemPlanStore,
        tx: &mut WorkItemPlanCompileTransaction,
    ) -> Result<InitialPlanCompileOutcome, String> {
        let result = self.resume_single_candidate_compile_transaction_inner(store, tx);
        if let Err(error) = &result {
            let lifecycle = self.lifecycle_store.clone();
            if let Some(lifecycle) = lifecycle {
                self.record_single_candidate_compile_failure(
                    &lifecycle,
                    "single_candidate_recovery_failed",
                    error,
                );
            }
        }
        result
    }

    fn resume_single_candidate_compile_transaction_inner(
        &mut self,
        store: &WorkItemPlanStore,
        tx: &mut WorkItemPlanCompileTransaction,
    ) -> Result<InitialPlanCompileOutcome, String> {
        self.validate_single_candidate_transaction_refs_inner(tx)?;
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let scope = SourceStoreScope {
            project_id: tx.project_id.clone(),
            issue_id: tx.issue_id.clone(),
            plan_id: tx.plan_id.clone(),
        };
        let source_ref = tx
            .source_revision_ref
            .clone()
            .ok_or_else(|| "single candidate transaction source ref is missing".to_string())?;
        let ir_ref = tx
            .plan_candidate_ir_ref
            .clone()
            .ok_or_else(|| "single candidate transaction IR ref is missing".to_string())?;
        let report_ref = tx
            .mechanical_report_ref
            .clone()
            .ok_or_else(|| "single candidate transaction report ref is missing".to_string())?;
        let provenance_ref = tx
            .publication_provenance_ref
            .clone()
            .ok_or_else(|| "single candidate transaction provenance ref is missing".to_string())?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source = source_store
            .get_source_revision(&scope, &source_ref)
            .map_err(|error| format!("{}: transaction source reload failed", error.code()))?;
        let ir = source_store
            .get_plan_candidate_ir(&scope, &ir_ref)
            .map_err(|error| format!("{}: transaction IR reload failed", error.code()))?;
        let report = source_store
            .get_mechanical_report(&scope, &report_ref)
            .map_err(|error| format!("{}: transaction report reload failed", error.code()))?;
        verify_publish_freshness(&source.source, &ir.ir, &report.report)
            .map_err(|error| format!("single candidate freshness failed: {error:?}"))?;
        let provenance = source_store
            .get_publication_provenance(&scope, &provenance_ref)
            .map_err(|error| format!("{}: transaction provenance reload failed", error.code()))?;
        if tx.publication_provenance_content_hash.as_deref()
            != Some(provenance.content_hash.as_str())
            || provenance.plan_revision_id
                != crate::product::work_item_revision_store::allocate_initial_plan_publication_ids(
                    &scope.project_id,
                    &scope.issue_id,
                    &scope.plan_id,
                    &tx.compile_id,
                    &ir.ir
                        .items
                        .iter()
                        .map(|item| item.contract.identity.logical_work_item_id.clone())
                        .collect::<Vec<_>>(),
                )
                .map_err(|error| error.to_string())?
                .plan_revision_id
        {
            return Err("single candidate provenance identity mismatch".to_string());
        }

        // SG3b resume 复验门：恢复重放（重新 lower IR 构造 publication）前对
        // 冻结候选与当前 live 上下文做同源结构复验——漂移/规则过期/坏有效图
        // 均拒绝重放（拒不重 lower 偷换发布物）；typed 身份（seal 后
        // failure_id）落 durable diagnostic，边界只取人读渲染。
        if let Err(mut gate_failure) = self.sg3b_final_compile_structure_gate(
            &lifecycle,
            &ir.ir,
            &source.source,
            &source_ref,
            &ir_ref,
            report.structure_report.as_ref(),
        ) {
            gate_failure.compile_run_id = Some(tx.compile_id.clone());
            let sealed = self.sg3b_seal_with_diagnostic(&lifecycle, gate_failure);
            return Err(sealed.render());
        }
        let previous_plan = tx.previous_plan_snapshot.clone();
        let logical_targets =
            self.logical_work_item_plan_repository_targets(&lifecycle, &previous_plan)?;
        let repository_id = if logical_targets.is_none() {
            ir.ir
                .items
                .first()
                .map(|item| item.target_repository_id.clone())
                .ok_or_else(|| "single candidate IR contains no items".to_string())?
        } else {
            String::new()
        };
        let change_order = draft_batch::compile_support::load_change_order_from_confirmed_design(
            &lifecycle,
            &previous_plan,
        )?;
        let context = ir_adapter::IrCompileAdapterContext {
            project_id: scope.project_id,
            issue_id: scope.issue_id,
            plan_id: scope.plan_id,
            previous_plan,
            source_revision_id: source.id,
            source_revision_ref: source_ref,
            plan_candidate_ir_ref: ir_ref,
            mechanical_report_ref: report_ref,
            publication_provenance_ref: provenance_ref,
            logical_targets,
            repository_id,
            change_order,
            compile_id: tx.compile_id.clone(),
            now: tx.created_at.clone(),
        };
        let input =
            ir_adapter::initial_plan_compile_input_from_ir(&context, &ir.ir, &report.report)?;
        let durable_context = ir_adapter::durable_compile_context_from_ir(&context, &provenance)?;
        let prepared = prepare_initial_plan_compile(input, durable_context)?;
        // 恢复重放与首次执行经同一提交段语义:source draft 先于 publication
        // 重放落盘(同 draft_id 覆盖写,幂等)。见
        // persist_initial_plan_compile_draft_records 的说明。
        persist_initial_plan_compile_draft_records(store, &prepared.draft_records)?;
        let publication_input = prepared
            .publication_input
            .ok_or_else(|| "single candidate recovery publication input is missing".to_string())?;
        let journal = prepare_initial_plan_publication(publication_input)
            .map_err(|error| error.to_string())?;
        let outcome = publish_initial_plan_revision(&self.revision_store(), &journal)
            .map_err(|error| error.to_string())?;
        tx.outline_to_work_item_id = prepared
            .work_items
            .iter()
            .map(|work_item| {
                let outline_id = work_item.source_outline_id.clone().ok_or_else(|| {
                    format!(
                        "recovery work item `{}` has no source outline id",
                        work_item.id
                    )
                })?;
                Ok((outline_id, work_item.id.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        tx.outline_to_verification_plan_id = prepared
            .verification_plans
            .iter()
            .map(|verification_plan| {
                let outline_id = prepared
                    .work_items
                    .iter()
                    .find(|work_item| work_item.id == verification_plan.work_item_id)
                    .and_then(|work_item| work_item.source_outline_id.clone())
                    .ok_or_else(|| {
                        format!(
                            "recovery verification plan `{}` has no matching outline",
                            verification_plan.id
                        )
                    })?;
                Ok((outline_id, verification_plan.id.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        // D2.1b 探针：SC resume 恢复重放位点（resume 还在跑同样 busy）。
        #[cfg(test)]
        probe_work_item_plan_compile_admission(
            &tx.project_id,
            &tx.issue_id,
            &tx.plan_id,
            "single_candidate_publication_resumed",
        );
        tx.step_cursor = "publication_resumed".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save resumed publication cursor failed: {error}"))?;
        Ok(outcome)
    }

    pub(crate) fn validate_single_candidate_transaction_refs(
        &mut self,
        tx: &WorkItemPlanCompileTransaction,
    ) -> Result<(), String> {
        let result = self.validate_single_candidate_transaction_refs_inner(tx);
        if let Err(error) = &result {
            let lifecycle = self.lifecycle_store.clone();
            if let Some(lifecycle) = lifecycle {
                self.record_single_candidate_compile_failure(
                    &lifecycle,
                    "single_candidate_recovery_failed",
                    error,
                );
            }
        }
        result
    }

    fn validate_single_candidate_transaction_refs_inner(
        &self,
        tx: &WorkItemPlanCompileTransaction,
    ) -> Result<(), String> {
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let scope = SourceStoreScope {
            project_id: tx.project_id.clone(),
            issue_id: tx.issue_id.clone(),
            plan_id: tx.plan_id.clone(),
        };
        let source_ref = tx
            .source_revision_ref
            .as_deref()
            .ok_or_else(|| "single candidate transaction source ref is missing".to_string())?;
        let ir_ref = tx
            .plan_candidate_ir_ref
            .as_deref()
            .ok_or_else(|| "single candidate transaction IR ref is missing".to_string())?;
        let report_ref = tx
            .mechanical_report_ref
            .as_deref()
            .ok_or_else(|| "single candidate transaction report ref is missing".to_string())?;
        let provenance_ref = tx
            .publication_provenance_ref
            .as_deref()
            .ok_or_else(|| "single candidate transaction provenance ref is missing".to_string())?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        source_store
            .get_source_revision(&scope, source_ref)
            .map_err(|error| format!("{}: transaction source ref invalid", error.code()))?;
        source_store
            .get_plan_candidate_ir(&scope, ir_ref)
            .map_err(|error| format!("{}: transaction IR ref invalid", error.code()))?;
        source_store
            .get_mechanical_report(&scope, report_ref)
            .map_err(|error| format!("{}: transaction report ref invalid", error.code()))?;
        let provenance = source_store
            .get_publication_provenance(&scope, provenance_ref)
            .map_err(|error| format!("{}: transaction provenance ref invalid", error.code()))?;
        if tx.publication_provenance_content_hash.as_deref()
            != Some(provenance.content_hash.as_str())
        {
            return Err(
                "SOURCE_STORE_CONTENT_HASH_MISMATCH: transaction provenance hash differs"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// F-54 B2（诊断 §4.1）：SC Final Compile 的可恢复失败只追加 diagnostics，
    /// 不落 `status=Failed + phase=Failed`。三处调用点（initial compile 的
    /// failure() 通道、recovery Continue 的 validate/resume）失败后门都会保持/
    /// 重开在 `WaitingForHuman`，残留 Failed 相位会形成 `(waiting_for_human,
    /// failed)` 矛盾对——confirm 与 abandon 双拒（0009 全通路死锁）。相位与
    /// 状态保持调用前的值（compile 前置已保证 Approval），与 strict validator
    /// 深层失败「不落 durable 相位」的既有观测良好行为对齐；终态 Failed 保留
    /// 给 AbortFatal / provider 级终态（routing_scope / provider run 既有路径）。
    fn record_single_candidate_compile_failure(
        &mut self,
        lifecycle: &LifecycleStore,
        code: &str,
        message: &str,
    ) {
        let Ok(record) = lifecycle.get_workspace_session(&self.session.session_id) else {
            return;
        };
        let mut diagnostics = record.policy_diagnostics.clone();
        diagnostics.push(PolicyDiagnostic {
            code: code.to_string(),
            message: message.to_string(),
            field: None,
        });
        if let Ok(saved) = lifecycle.compare_and_save_policy_route(
            &record,
            PolicyRoutePersist {
                status: record.status.clone(),
                single_candidate_phase: record.single_candidate_phase.clone(),
                run_history: record.run_history.clone(),
                scope: record.review_invocation_scope.clone(),
                gate: record.human_gate_snapshot.clone(),
                diagnostics,
                repair_reservation: record.repair_reservation.clone(),
                provider_start_ledger: record.provider_start_ledger.clone(),
                author_rewrite_budget: record.author_rewrite_budget.clone(),
            },
        ) {
            self.session.session_status = saved.status;
            self.session.single_candidate_phase = saved.single_candidate_phase;
            self.session.policy_diagnostics = saved.policy_diagnostics;
        }
    }
}

#[cfg(test)]
fn single_candidate_compile_failpoints()
-> &'static std::sync::Mutex<std::collections::HashMap<SingleCandidateCompileFailpointKey, u64>> {
    SINGLE_CANDIDATE_COMPILE_FAILPOINTS
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
impl Drop for SingleCandidateCompileFailpointGuard {
    fn drop(&mut self) {
        let mut failpoints = single_candidate_compile_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoints.get(&self.key) == Some(&self.registration_id) {
            failpoints.remove(&self.key);
        }
    }
}
