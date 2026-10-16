impl super::WorkspaceEngine {
    pub(crate) async fn run_sc_manual_revision_turn(
        &mut self,
        turn_id: &str,
        provider_output: String,
    ) -> Result<ScManualRevisionResult, String> {
        use crate::product::models::HumanGateTurnStatus;
        use crate::product::work_item_plan_source_store::{
            PlanCandidateMechanicalReportRecord, WorkItemPlanSourceStore,
        };

        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let turn = lifecycle
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| error.to_string())?;
        if turn.status != HumanGateTurnStatus::Running
            && turn.status != HumanGateTurnStatus::Reserved
        {
            return Err(format!("human gate turn {turn_id} is not active"));
        }
        let delivery = prepare_revision_delivery_for_compile(&provider_output);
        if delivery.normalized_heading_lines > 0 {
            let node_id = self
                .active_node_id
                .clone()
                .unwrap_or_else(|| "timeline_node_unknown".to_string());
            tracing::info!(
                session_id = %self.session.session_id,
                turn_id = %turn_id,
                node_id = %node_id,
                diagnostic = crate::product::work_item_plan_compiler::PLAN_HEADING_NORMALIZATION_DIAGNOSTIC,
                normalized_heading_lines = delivery.normalized_heading_lines,
                "人工修订 markdown 结构标题已确定性归一化后再编译"
            );
            self.emit_execution_event(
                crate::cross_cutting::streaming_provider::ProviderExecutionEvent {
                    event_id: format!(
                        "human_gate_revision_heading_normalized_{node_id}_{turn_id}"
                    ),
                    kind: crate::cross_cutting::streaming_provider::ProviderExecutionEventKind::Provider,
                    status: crate::cross_cutting::streaming_provider::ProviderExecutionEventStatus::Completed,
                    title: "人工修订结构标题确定性归一化".to_string(),
                    detail: Some(format!(
                        "normalized {} structural heading lines via the fixed zh→en table before compile",
                        delivery.normalized_heading_lines
                    )),
                    command: None,
                    cwd: None,
                    output: None,
                    exit_code: None,
                },
                self.active_node_id.clone(),
                Some(self.session.author_provider.clone()),
            )
            .await;
        }
        if delivery.normalized_ears_lines > 0 {
            let node_id = self
                .active_node_id
                .clone()
                .unwrap_or_else(|| "timeline_node_unknown".to_string());
            tracing::info!(
                session_id = %self.session.session_id,
                turn_id = %turn_id,
                node_id = %node_id,
                diagnostic = crate::product::work_item_plan_compiler::PLAN_EARS_SPACING_NORMALIZATION_DIAGNOSTIC,
                normalized_ears_lines = delivery.normalized_ears_lines,
                "人工修订 markdown EARS 关键字空白已确定性归一化后再编译"
            );
            self.emit_execution_event(
                crate::cross_cutting::streaming_provider::ProviderExecutionEvent {
                    event_id: format!(
                        "human_gate_revision_ears_spacing_normalized_{node_id}_{turn_id}"
                    ),
                    kind: crate::cross_cutting::streaming_provider::ProviderExecutionEventKind::Provider,
                    status: crate::cross_cutting::streaming_provider::ProviderExecutionEventStatus::Completed,
                    title: "人工修订 EARS 关键字空白确定性归一化".to_string(),
                    detail: Some(format!(
                        "normalized {} EARS statement lines (keyword-adjacent whitespace only; WHEN / THE SYSTEM SHALL spacing) before compile",
                        delivery.normalized_ears_lines
                    )),
                    command: None,
                    cwd: None,
                    output: None,
                    exit_code: None,
                },
                self.active_node_id.clone(),
                Some(self.session.author_provider.clone()),
            )
            .await;
        }

        // SG6a.1（REQ-CG-03「修订不过校验」/REQ-SG-02）：判定先于任何终态
        // 写。原「compile/validate 失败 → 立即把 turn 写成终态 Failed」的
        // 单段驱动拆为判定/成功提交/终结三段——typed 结构失败（机器事实
        // 完整）在同 turn 冻结机器指令 prompt 并 claim attempt2（恰一
        // 次）；业务事实缺少/未 typed 失败回原门；operational 沿等待；
        // 物理 attempt 耗尽是 typed 系统故障停点。坏 source 不提升
        // current refs、不评审、零 provider 启动。
        let outcome = self.assess_sc_revision_output_full(turn_id, &provider_output);
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(failure) => {
                // attempt 耗尽是候选命运的 typed 系统故障（in-band 回显；
                // 终态收口/release hold 归 SG6a.3）。其余是判定管道自身的
                // operational 错——沿原 Err(String) 语义上抛，不吞错。
                if failure.stable_code() == "STRUCTURAL_ATTEMPTS_EXHAUSTED" {
                    // SG6a.3（复审②③）：Running+claim/上限停点的终结写
                    //（recovery AwaitingStructuralClosure 的对端）——
                    // Failed(validation_reject)+released_structural 恰一次
                    // +grant SystemFault；CAS 失败上抛（release CAS 失败
                    // 不能假通知成功）。
                    self.close_sc_revision_turn_after_rejection(
                        turn_id,
                        crate::product::models::UserBudgetReceiptKind::ReleasedStructural,
                        "structural_exhausted_before_delivery",
                        true,
                    )
                    .await?;
                    return Ok(ScManualRevisionResult::ValidationRejected {
                        diagnostics: vec![render_sc_revision_compile_failure(&failure)],
                        claim: None,
                    });
                }
                return Err(render_sc_revision_compile_failure(&failure));
            }
        };
        match outcome.assessment {
            crate::product::workspace_engine::types::StructuralAssessment::Accepted(prepared) => {
                let accepted = outcome
                    .accepted
                    .expect("accepted assessment carries same-generation commit facts");
                // 成功提交段：accepted 新 refs 必须携 SG3a.0 冻结 payload
                //（plan §0.6.2：只能提交已含 Some 合法 payload 的同代
                // refs，与 author 路径 `complete_single_candidate_work_
                // item_plan_author` 同源）。
                let frozen_structure_report =
                    super::plan_projection::freeze_persisted_structure_report(
                        &prepared.report,
                        &accepted.source_id,
                        &accepted.ir_id,
                    )
                    .map_err(|error| format!("freeze structure report failed: {error}"))?;
                // report_id 全量内容寻址（对齐 single_candidate author 路径与
                // provenance 全 sha256 惯例）：id 由全量记录体派生，同 source 上
                // validator 语义演进产生新 id 追加写入，杜绝同 id 不同 body 撞
                // put_immutable 的确定性 IdentityMismatch。
                let mut report_record = PlanCandidateMechanicalReportRecord {
                    id: String::new(),
                    source_revision_id: accepted.source_id.clone(),
                    ir_id: accepted.ir_id.clone(),
                    report: accepted.report,
                    structure_report: Some(frozen_structure_report),
                    content_hash: String::new(),
                };
                report_record.id = report_record
                    .content_addressed_id()
                    .map_err(|error| error.code().to_string())?;
                report_record.content_hash = report_record
                    .content_hash()
                    .map_err(|error| error.code().to_string())?;
                let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
                let report_ref = source_store
                    .put_mechanical_report(
                        &self.session.project_id,
                        &self.session.issue_id,
                        &self.session.entity_id,
                        &report_record,
                    )
                    .map_err(|error| error.code().to_string())?;
                let artifact_versions = {
                    let mut versions = self.artifact_versions.clone();
                    for version in &mut versions {
                        version.is_current = false;
                    }
                    let version = versions.len() as u32 + 1;
                    versions.push(crate::web::workspace_ws_types::ArtifactVersion {
                        version,
                        payload: crate::web::workspace_ws_types::ArtifactPayload::Markdown {
                            markdown: prepared.source.clone(),
                            diff: None,
                        },
                        generated_by: self.session.author_provider.clone(),
                        reviewed_by: None,
                        review_verdict: None,
                        confirmed_by: None,
                        is_current: true,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        source_node_id: self
                            .active_node_id
                            .clone()
                            .unwrap_or_else(|| "timeline_node_unknown".to_string()),
                    });
                    versions
                };
                let artifact_id = format!(
                    "artifact_version_{:03}",
                    artifact_versions
                        .last()
                        .map(|version| version.version)
                        .unwrap_or(0)
                );
                // SG6a.3（REQ-CG-03「修订成功回呈」/REQ-CG-02/05）：合法
                // refs 对应原业务才 deliver——deliver settlement 与终态分
                // 离：commit 在 turn Running 上同 CAS settle
                // DeliveredCharge 恰一次、轮换三 refs、grant→Reviewing、
                // 落 checkpoint（SG4b store 单源）；终态写与 review 收口
                // 在路由后分离落账（复评仍 in-flight）。
                self.mark_human_gate_turn_running(turn_id)?;
                let expected = lifecycle
                    .get_workspace_session(&self.session.session_id)
                    .map_err(|error| error.to_string())?;
                let running_turn = lifecycle
                    .get_human_gate_turn(&self.session.session_id, turn_id)
                    .map_err(|error| error.to_string())?;
                let saved = lifecycle
                    .commit_structurally_valid_revision(
                        &expected,
                        running_turn,
                        &accepted.source_ref,
                        &accepted.ir_ref,
                        &report_ref,
                        &artifact_versions,
                        &artifact_id,
                        &prepared.report,
                    )
                    .map_err(|error| error.to_string())?;
                self.artifact_versions = artifact_versions;
                self.session.artifact =
                    Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
                        markdown: prepared.source,
                        diff: None,
                    });
                if let Some(node_id) = self.active_node_id.clone() {
                    let _ = self
                        .persist_artifact_ref(
                            &node_id,
                            crate::product::models::ArtifactRef {
                                artifact_id: artifact_id.clone(),
                                version: self
                                    .artifact_versions
                                    .last()
                                    .map(|version| version.version)
                                    .unwrap_or(0),
                            },
                        )
                        .await;
                }
                let _ = self
                    .event_tx
                    .send(super::EngineEvent::ArtifactUpdate {
                        version: self
                            .artifact_versions
                            .last()
                            .map(|version| version.version)
                            .unwrap_or(0),
                        payload: self.session.artifact.clone().expect("artifact set above"),
                    })
                    .await;
                self.reload_session_from_record(saved);
                if let Some(current_version) = self
                    .artifact_versions
                    .iter()
                    .find(|version| version.is_current)
                {
                    self.session.artifact = Some(current_version.payload.clone());
                }
                // F-49/A6：修订轮载体是 author 节点时在路由前收口该节点——与普通 SC 修订
                // 同构（`single_candidate.rs` 的 complete_single_candidate_work_item_plan_author
                // 同样先 complete_active_node 再 route，实测 workspace_session_0009 的
                // node_004/node_008 均 Completed）。不收口会留下永久 Active 的「修订中」author
                // 节点（时间线 UI 持续显示进行中）。门节点载体（历史直驱/引擎级调用）保持
                // Active：门仍开着，其收口由 enter_human_confirm 与终态 compile 负责。
                if self.active_node_type() == Some(super::TimelineNodeType::AuthorRun) {
                    self.complete_active_node(Some(
                        "已按人工反馈完成修订并持久化，等待复评".to_string(),
                    ))
                    .await;
                }
                // SG6a.3（REQ-CG-03「修订成功回呈」）：复评收口分流——
                // 无 reviewer 走本地 synthetic Pass（同步收口），有 reviewer
                // 重启评审（复评仍 in-flight，turn 保持 Running 至 verdict）。
                // 收口后（synthetic 分支在路由返回处同步成立）turn 终态化
                // Completed；grant 携 review 证据终态 Resolved（finalize 幂
                // 等）。有 reviewer 分支的 verdict 面收口归 SG6b（routing
                // 面），本驱动面只完成 turn 终态写（保持既有可 confirm 面）。
                let review_node_id = self
                    .active_node_id
                    .clone()
                    .unwrap_or_else(|| "single_candidate_evaluate".to_string());
                let review_closed_synchronously =
                    self.session.review_rounds == 0 || self.session.reviewer_provider.is_none();
                if review_closed_synchronously {
                    self.route_single_candidate_evaluate_without_reviewer()
                        .await;
                } else {
                    self.start_review().await;
                    if self.session.stage == super::WorkspaceStage::CrossReview {
                        self.request_provider_run(super::ProviderRunKind::ReviewOnly)
                            .await;
                    }
                }
                // 终态写与 grant 终态化（断点幂等：已终态零写）。SG6b
                //（REQ-CG-03:74 / SG6a.3⑧1 移交）：verdict 面时序——
                // synthetic 收口（无 reviewer）在本驱动面同步终态化
                // turn+grant；有 reviewer 分支 turn 保持 Running/gate_busy
                // 至原 reviewer verdict 收口（complete_review 面：
                // turn Completed + grant Resolved finalize），此处不再
                // 提前终态化。
                if review_closed_synchronously {
                    {
                        let expected = lifecycle
                            .get_workspace_session(&self.session.session_id)
                            .map_err(|error| error.to_string())?;
                        let mut completed_turn = lifecycle
                            .get_human_gate_turn(&self.session.session_id, turn_id)
                            .map_err(|error| error.to_string())?;
                        if completed_turn.status
                            != crate::product::models::HumanGateTurnStatus::Completed
                        {
                            completed_turn.status =
                                crate::product::models::HumanGateTurnStatus::Completed;
                            completed_turn.failure_class = None;
                            completed_turn.updated_at = chrono::Utc::now().to_rfc3339();
                            lifecycle
                                .update_human_gate_turn(&expected, completed_turn)
                                .map_err(|error| error.to_string())?;
                        }
                    }
                    let expected = lifecycle
                        .get_workspace_session(&self.session.session_id)
                        .map_err(|error| error.to_string())?;
                    let grant_binding =
                        expected
                            .structural_governance_state
                            .as_ref()
                            .and_then(|state| {
                                let grant = state.current_grant.as_ref()?;
                                (grant.turn_id.as_deref() == Some(turn_id)).then(|| {
                                    (
                                        grant.grant_id.clone(),
                                        state.current_context_digest.clone().unwrap_or_default(),
                                    )
                                })
                            });
                    if let Some((grant_id, context_digest)) = grant_binding {
                        let turn_final = lifecycle
                            .get_human_gate_turn(&self.session.session_id, turn_id)
                            .map_err(|error| error.to_string())?;
                        let scope_digest = sc_revision_review_scope_digest(
                            &accepted.source_ref,
                            &accepted.ir_ref,
                            &report_ref,
                            &context_digest,
                        );
                        lifecycle
                            .finalize_structural_revision_grant(
                                &expected,
                                turn_final,
                                &grant_id,
                                Some(&review_node_id),
                                Some(&scope_digest),
                            )
                            .map_err(|error| error.to_string())?;
                    }
                }
                Ok(ScManualRevisionResult::Accepted {
                    artifact_ref: artifact_id,
                })
            }
            crate::product::workspace_engine::types::StructuralAssessment::Rejected(
                set,
                instructions,
            ) => {
                // L1（机器事实完整的确定性结构失败）：在任何终态写之前冻结
                // 机器指令 prompt 并沿同 grant 同 turn claim attempt2
                //（REQ-CG-03：不新反馈、不扣另一轮；REQ-SG-05：claim 与
                // 真实 start 分账——本驱动不启动 provider，实际 attempt2
                // 启动由 SG6a.2 handler 消费唯一 claim/冻结 prompt）。
                let mut diagnostics = render_sc_revision_failure_set(&set);
                let inputs = outcome
                    .rejection
                    .expect("rejected assessment carries claim facts");
                // SG6a.2：claim 成功时冻结件随案返回（handler 消费唯一
                // claim/冻结 prompt 驱动 attempt2；Replayed/Waiting/Exhausted
                // 均不携案——唯一 claim 已归首次返回）。
                let mut claim_carry: Option<ScRevisionClaimCarry> = None;
                match instructions.first() {
                    Some(instruction) => {
                        let resolved = expected
                            .structural_governance_state
                            .as_ref()
                            .map(|state| state.resolved_structure_facts.clone())
                            .unwrap_or_default();
                        match super::prompts::build_structural_revision_prompt(
                            &turn.feedback_text,
                            &inputs.failed_source,
                            instruction,
                            &inputs.context,
                            &resolved,
                        ) {
                            Ok(frozen) => {
                                // provider 预算 fail-closed（SG5.2 裁决）：
                                // Operational 不驱 author、不 claim（零
                                // hold/零 start）。
                                let budget = super::prompts::sc_provider_input_budget(
                                    &self.session.author_provider,
                                );
                                if let Err(rejection) = frozen.adjudicate_provider_budget(&budget) {
                                    diagnostics
                                        .push(render_structural_prompt_rejection(&rejection));
                                    // SG6a.3（复审①）：预算拒收口——
                                    // Failed(validation_reject)+nonstructural_
                                    // failure_charge（非结构终态不退，
                                    // REQ-CG-02），Busy 面解锁。
                                    self.close_sc_revision_turn_after_rejection(
                                        turn_id,
                                        crate::product::models::UserBudgetReceiptKind::NonstructuralFailureCharge,
                                        "structural_prompt_budget_rejected",
                                        false,
                                    )
                                    .await?;
                                    return Ok(ScManualRevisionResult::ValidationRejected {
                                        diagnostics,
                                        claim: None,
                                    });
                                }
                                let expected_for_claim = lifecycle
                                    .get_workspace_session(&self.session.session_id)
                                    .map_err(|error| error.to_string())?;
                                // SG6b P2 修复轮（Main 审查派回）：冻结件
                                // 内容寻址落盘先于 durable claim（幂等覆
                                // 写）——claim 与落盘两 durable 动作间崩溃
                                // 时重试链恒有冻结证据可读，fresh 恢复不
                                // 至 CANDIDATE_DRIFTED 0 start、attempt2
                                // 通道不静默丢失（REQ-SG-05「显式 fresh
                                // 同 attempt None-resume/子 prompt」）。
                                Self::persist_frozen_structural_prompt(
                                    &lifecycle.app_paths(),
                                    &self.session.project_id,
                                    &self.session.issue_id,
                                    &self.session.session_id,
                                    &frozen,
                                )?;
                                match lifecycle.claim_structural_attempt(
                                    &expected_for_claim,
                                    turn_id,
                                    instruction,
                                    &frozen.prompt_ref,
                                    &frozen.prompt_digest,
                                ) {
                                    Ok(
                                        crate::product::models::workspace::StructuralClaimOutcome::Claimed {
                                            attempt_no,
                                            prompt_ref,
                                            prompt_digest: _,
                                            provider_start_key,
                                        },
                                    ) => {
                                        #[cfg(test)]
                                        if SG6B_CLAIM_PERSIST_CRASH.swap(false, std::sync::atomic::Ordering::SeqCst) {
                                            // P2 复审（Main 审查 suggestion）：崩溃窗读回判别
                                            //——claim 已落而冻结件缺失（旧序：persist 在 claim 后）
                                            // 在此 panic；新序（persist 先于 durable claim）恒有
                                            // 内容寻址证据可读回。
                                            Self::load_frozen_structural_prompt(
                                                &lifecycle.app_paths(),
                                                &self.session.project_id,
                                                &self.session.issue_id,
                                                &self.session.session_id,
                                                &prompt_ref,
                                            )
                                            .expect("sg6b_claim_persist_crash_window: frozen evidence must be readable after the durable claim (persist must precede claim)");
                                        }
                                        diagnostics.push(format!(
                                            "STRUCTURAL_ATTEMPT_CLAIMED: attempt {attempt_no} binds frozen prompt {prompt_ref} on the same turn"
                                        ));
                                        claim_carry = Some(ScRevisionClaimCarry {
                                            frozen,
                                            attempt_no,
                                            provider_start_key,
                                        });
                                    }
                                    Ok(crate::product::models::workspace::StructuralClaimOutcome::Replayed) => {
                                        diagnostics.push(
                                            "STRUCTURAL_ATTEMPT_CLAIM_REPLAYED: same prompt identity claimed exactly once; no third attempt"
                                                .to_string(),
                                        );
                                    }
                                    Ok(crate::product::models::workspace::StructuralClaimOutcome::Waiting { reason }) => {
                                        diagnostics.push(format!(
                                            "STRUCTURAL_CLAIM_WAITING: {reason}"
                                        ));
                                    }
                                    Ok(crate::product::models::workspace::StructuralClaimOutcome::Exhausted) => {
                                    // SG6a.3（复审②③）：claim 面耗尽的同款
                                    // 终结写（release 恰 1+grant SystemFault）。
                                    self.close_sc_revision_turn_after_rejection(
                                        turn_id,
                                        crate::product::models::UserBudgetReceiptKind::ReleasedStructural,
                                        "structural_exhausted_before_delivery",
                                        true,
                                    )
                                    .await?;
                                    return Ok(ScManualRevisionResult::ValidationRejected {
                                        diagnostics: vec![
                                            "STRUCTURAL_ATTEMPTS_EXHAUSTED: physical attempt cap consumed (L0/L1 share the same turn attempt key); typed system fault, no third start"
                                                .to_string(),
                                        ],
                                        claim: None,
                                    });
                                    }
                                    Err(error) => return Err(error.to_string()),
                                }
                            }
                            Err(prompt_set) => {
                                // SG5.2 builder fail-closed（Operational，
                                // 如反馈超限/候选漂移）：等待面，不 claim。
                                diagnostics.push(render_structural_prompt_rejection(&prompt_set));
                            }
                        }
                    }
                    None => {
                        // classify 判 L1 但 producer 零指令（保护事实缺
                        // 失/producer fail-closed）：不驱 author，回原门。
                        // SG6a.3（复审①）：零指令收口（charge 不退）。
                        diagnostics.push(
                            "STRUCTURAL_REPAIR_UNPRODUCIBLE: L1 facts classified but no machine instruction could be produced; the failure stays with the original human gate"
                                .to_string(),
                        );
                        self.close_sc_revision_turn_after_rejection(
                            turn_id,
                            crate::product::models::UserBudgetReceiptKind::NonstructuralFailureCharge,
                            "structural_repair_unproducible",
                            false,
                        )
                        .await?;
                    }
                }
                Ok(ScManualRevisionResult::ValidationRejected {
                    diagnostics,
                    claim: claim_carry,
                })
            }
            crate::product::workspace_engine::types::StructuralAssessment::ExistingGate(set) => {
                // 业务事实缺少/未 typed 失败：回原门（REQ-SG-01「真实业务
                // 矛盾」由用户裁决；类别/class_hint 不构成 L1 许可）。
                // SG6a.3（复审①）：无 claim 的原门拒收收口——
                // Failed(validation_reject)+nonstructural_failure_charge，
                // `handle_human_gate_feedback` 的 Busy 面解锁（REQ-CG-02
                //「其他非结构终态维持原预约扣次不退」）。
                let mut diagnostics = render_sc_revision_failure_set(&set);
                diagnostics.push(
                    "STRUCTURAL_EXISTING_GATE: failure requires the original human gate; no machine repair is licensed"
                        .to_string(),
                );
                self.close_sc_revision_turn_after_rejection(
                    turn_id,
                    crate::product::models::UserBudgetReceiptKind::NonstructuralFailureCharge,
                    "validation_rejected_existing_gate",
                    false,
                )
                .await?;
                Ok(ScManualRevisionResult::ValidationRejected {
                    diagnostics,
                    claim: None,
                })
            }
            crate::product::workspace_engine::types::StructuralAssessment::Operational(set) => {
                // operational/授权等待：沿原安全等待，不驱 author；等待
                // 不构成 claim（复审①同款收口，charge 不退——机器窗口无
                // 授权事实时不留 Running 死锁）。
                let mut diagnostics = render_sc_revision_failure_set(&set);
                diagnostics.push(
                    "STRUCTURAL_OPERATIONAL_WAIT: operational/authorization facts missing; no author attempt is driven"
                        .to_string(),
                );
                self.close_sc_revision_turn_after_rejection(
                    turn_id,
                    crate::product::models::UserBudgetReceiptKind::NonstructuralFailureCharge,
                    "validation_rejected_operational_wait",
                    false,
                )
                .await?;
                Ok(ScManualRevisionResult::ValidationRejected {
                    diagnostics,
                    claim: None,
                })
            }
        }
    }

    /// SG6a.3（复审①②③，plan v1.1 Task SG6a.3）：无 claim 拒收 arm 与
    /// 耗尽停点的 turn 终态收口——Failed(validation_reject) 落账（幂等：
    /// 已终态零写）+ 用户预算 hold→终态 receipt 结算
    /// （`settle_structural_user_budget` 单源幂等：同 receipt 重放零写；
    /// released_structural 恢复快照可用预算、nonstructural_failure_charge
    /// 不退）+ 耗尽证据时 grant 终态化 SystemFault（`finalize_structural_
    /// revision_grant` 幂等）。断点幂等：receipt/grant 按 durable 证据重
    /// 放零二次；settle/finalize 的 CAS 失败上抛（release CAS 失败不能
    /// 假通知成功）；旧记录无聚合状态/无 hold 不凭空造账（零结算跳过，
    /// 不追溯，`settle` 的 InvalidRecord 容忍为已结算/legacy 形态）。
    async fn close_sc_revision_turn_after_rejection(
        &mut self,
        turn_id: &str,
        receipt_kind: crate::product::models::UserBudgetReceiptKind,
        reason: &str,
        finalize_exhausted: bool,
    ) -> Result<(), String> {
        use crate::product::models::HumanGateTurnFailureClass;

        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        // 失败分类落账：唯一一次终态 failure 写（同 turn 幂等，二次调用
        // 零写），并收口修订 author 载体节点。
        self.fail_human_gate_turn(turn_id, HumanGateTurnFailureClass::ValidationReject)
            .await?;
        // 预算结算：fresh expected 上 CAS；无聚合状态的旧记录零结算跳过。
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        if expected.structural_governance_state.is_some() {
            let turn = store
                .get_human_gate_turn(&self.session.session_id, turn_id)
                .map_err(|error| error.to_string())?;
            let receipt = crate::product::models::UserBudgetReceipt {
                kind: receipt_kind,
                turn_id: turn.turn_id.clone(),
                command_id: turn.command_id.clone(),
                attempt_reason: Some(
                    crate::product::models::StructuralAttemptReason::HumanRevision,
                ),
                source_revision_ref: None,
                plan_candidate_ir_ref: None,
                structure_report_ref: None,
                context_digest: None,
                reason: Some(reason.to_string()),
                settled_at: chrono::Utc::now().to_rfc3339(),
            };
            match store.settle_structural_user_budget(&expected, turn_id, receipt) {
                Ok(_) => {}
                Err(crate::product::json_store::ProductStoreError::InvalidRecord { .. }) => {
                    // 无活动 hold（他路已结算/legacy hold 缺席）：durable
                    // 账本为准，不凭空造账也不上抛假错。
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        if !finalize_exhausted {
            return Ok(());
        }
        // 耗尽证据终态化 grant（同 grant 幂等；无 grant/未绑本 turn 跳过）。
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let Some(grant_id) = expected
            .structural_governance_state
            .as_ref()
            .and_then(|state| state.current_grant.as_ref())
            .filter(|grant| grant.turn_id.as_deref() == Some(turn_id))
            .map(|grant| grant.grant_id.clone())
        else {
            return Ok(());
        };
        let turn = store
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| error.to_string())?;
        store
            .finalize_structural_revision_grant(&expected, turn, &grant_id, None, None)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

// ============================================================================

// SG6a.1：SC 修订轮判定面（assess）——四分支分类 + 同 turn claim 驱动事实。
// ============================================================================

/// SG6a.2（Main 裁定(a)）：结构拒收后同 turn 已 claim 的 attempt2 交付
/// 载体——`run_sc_manual_revision_turn` 返回面的最小扩展，handler 消费
/// 唯一 claim/冻结 prompt 的全部事实（claim 与真实 start 分账的 claim 侧；
/// `provider_start_key` 由真实启动路径登记 provider_start_ledger）。
/// 纯数据：不启动 provider、不做 turn CAS；判定/claim 内部零改动。
#[derive(Debug, Clone)]
pub(crate) struct ScRevisionClaimCarry {
    /// claim 绑定的冻结机器指令 prompt（完整内容寻址身份 + 全文）。
    pub(crate) frozen: super::prompts::FrozenStructuralPrompt,
    /// claim 分配的同 turn 物理 attempt 号（L0/L1 共用同一 turn attempt
    /// key 下的下一个 attempt）。生产读者为审计/终结收口面（SG6a.3）；
    /// 当前合同消费见 sg_06a_2 用例——沿 SG6a.1「尚未接线」先例门控。
    #[allow(dead_code)]
    pub(crate) attempt_no: u32,
    /// 真实启动路径登记 provider_start_ledger 的幂等键
    /// （`human_gate:{turn_id}:attempt:{n}`；claim 已保证其缺席）。
    pub(crate) provider_start_key: String,
}

/// 判定面的富载体：四分支评估 + 驱动面所需同代事实。公开签名
/// `assess_sc_revision_output`（plan §0.5）只投影 `assessment`；`accepted`
/// 仅在 `Accepted` 分支、`rejection` 仅在 `Rejected` 分支携带。
pub(crate) struct ScRevisionAssessment {
    pub(crate) assessment: crate::product::workspace_engine::types::StructuralAssessment,
    /// `Accepted`：三 refs 同代提交事实（source/IR 证据已 immutable 落
    /// 盘，validator 同源报告随提交段写 report record）。
    pub(crate) accepted: Option<ScRevisionAcceptedFacts>,
    /// `Rejected`：claim 段事实（同代结构上下文 + 失败候选全文）。
    pub(crate) rejection: Option<ScRevisionRejectionFacts>,
}

pub(crate) struct ScRevisionAcceptedFacts {
    pub(crate) source_id: String,
    pub(crate) source_ref: String,
    pub(crate) ir_id: String,
    pub(crate) ir_ref: String,
    pub(crate) report: crate::product::work_item_plan_compiler::PlanCandidateMechanicalReport,
}

pub(crate) struct ScRevisionRejectionFacts {
    pub(crate) context:
        crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureContext,
    pub(crate) failed_source: String,
}

/// 判定管道 typed 停点 → 人读单行（稳定码 + render；不 parse message）。
fn render_sc_revision_compile_failure(
    failure: &crate::product::workspace_engine::types::PlanningCompileFailure,
) -> String {
    format!("[{}] {}", failure.stable_code(), failure.render())
}

/// typed 失败集合 → 诊断行（stable code + 受影响 ID；message 入 details
/// 时附带，供 WS 回显拼接）。
fn render_sc_revision_failure_set(
    set: &crate::product::planning_failure::PlanningFailureSet,
) -> Vec<String> {
    set.failures
        .iter()
        .map(|failure| {
            let ids = failure.evidence.affected_ids.join("->");
            let code = failure.kind.stable_code();
            match failure.evidence.details.get("message") {
                Some(message) => format!("{code}[{ids}]: {message}"),
                None => format!("{code}[{ids}]"),
            }
        })
        .collect()
}

/// 未解析候选（parser 拒收，无 IR）的结构上下文 IR 引用占位——不伪 IR
/// 图事实，渲染面如实呈现「未解析」。
const SC_REVISION_PRE_COMPILE_IR_REF: &str = "plan-ir:none-unparsed-source";

/// 与 `build_single_candidate_structure_context` 内 `digest_json` 同配方
///（serde_json→sha256；序列化失败按空串，与 digest 单源一致）。
fn sc_revision_digest_json<T: serde::Serialize>(value: &T) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(
        serde_json::to_string(value).unwrap_or_default().as_bytes(),
    ))
}

/// SG6a.3（REQ-SG-04/REQ-CG-03「修订成功回呈」）：交付代次的评审
/// scope digest——绑定同代 source/IR/report refs 与 context digest，作
/// 为 `finalize_structural_revision_grant`（Completed→Resolved）的
/// review 证据与 checkpoint `review_scope` 回填值（与
/// `structural_checkpoint_id` 同代身份域，不另立第二候选权威）。
pub(crate) fn sc_revision_review_scope_digest(
    source_revision_ref: &str,
    plan_candidate_ir_ref: &str,
    structure_report_ref: &str,
    context_digest: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let canonical = [
        "structural_review_scope",
        source_revision_ref,
        plan_candidate_ir_ref,
        structure_report_ref,
        context_digest,
    ]
    .join("\0");
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(canonical.as_bytes()))
    )
}

/// 未解析候选的结构上下文（「未解析先修格式，不猜 IR 图」）：图事实
///（Design/成员/仓序/基线/intent）一律 None/空，仅绑定 source 身份、
/// 编译器/规则版本与 plan options 摘要；digest 沿
/// `compute_planning_context_digest` 单源配方。与 IR 后权威
/// `build_single_candidate_structure_context` 输入类不相交（后者要求真
/// 实 IR），不构成同一输入的第二上下文权威。
fn sc_revision_pre_compile_context(
    source_revision_ref: &str,
    source_hash: &str,
    options_digest: &str,
) -> crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureContext {
    let mut context =
        crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureContext {
            source_revision_ref: source_revision_ref.to_string(),
            plan_candidate_ir_ref: SC_REVISION_PRE_COMPILE_IR_REF.to_string(),
            source_hash: source_hash.to_string(),
            compiler_version:
                crate::product::work_item_plan_compiler::grammar::WORK_ITEM_PLAN_COMPILER_VERSION
                    .to_string(),
            rule_version: super::plan_projection::PLANNING_STRUCTURE_RULE_VERSION.to_string(),
            context_digest: String::new(),
            confirmed_design_id: None,
            confirmed_design_digest: None,
            member_table_digest: None,
            selection_digest: None,
            physical_by_logical: Default::default(),
            change_order: Vec::new(),
            options_digest: options_digest.to_string(),
            baseline_tree_digest: None,
            intent_digest: None,
        };
    context.context_digest = super::plan_projection::compute_planning_context_digest(&context);
    context
}

/// 未解析候选的指令身份载体：零有效边/零拓扑事实（heading 类修复不需
/// 图事实；witness 类失败不会出现在未解析候选上——编译已失败）。
fn sc_revision_pre_compile_report(
    context: &crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureContext,
    failures: crate::product::planning_failure::PlanningFailureSet,
) -> crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureReport {
    crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureReport {
        source_hash: context.source_hash.clone(),
        context_digest: context.context_digest.clone(),
        rule_version: context.rule_version.clone(),
        compiler_version: context.compiler_version.clone(),
        effective_edges: Vec::new(),
        topological_order: None,
        residual_nodes: Vec::new(),
        physical_targets: Default::default(),
        declared_dependency_facts: Vec::new(),
        failures,
        structurally_valid: false,
    }
}

/// compiler/validator 诊断 → typed 失败集合。只有源分支显式写的
/// `Some(kind)` 才携带 typed payload；其余折叠为 `Unknown{code}`（缺事
/// 实不猜、严禁从 code/message/class_hint 反推——F1.2 口径）。message/
/// repair_example 入 evidence details 供回显。
fn sc_revision_failure_set_from_diagnostics(
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
    source_ref: &str,
    source_hash: &str,
) -> crate::product::planning_failure::PlanningFailureSet {
    crate::product::planning_failure::PlanningFailureSet {
        failures: diagnostics
            .iter()
            .map(|diagnostic| {
                let mut details = std::collections::BTreeMap::new();
                details.insert("line".to_string(), diagnostic.line.to_string());
                details.insert("field".to_string(), diagnostic.field.clone());
                details.insert("message".to_string(), diagnostic.message.clone());
                details.insert(
                    "repair_example".to_string(),
                    diagnostic.repair_example.clone(),
                );
                crate::product::planning_failure::PlanningFailure {
                    kind: diagnostic.kind.clone().unwrap_or(
                        crate::product::planning_failure::PlanningFailureKind::Unknown {
                            code: diagnostic.code.clone(),
                        },
                    ),
                    origin: crate::product::planning_failure::PlanningFailureOrigin::Compiler,
                    stage: crate::product::planning_failure::PlanningStage::Plan,
                    diagnostic_ref: format!(
                        "plan-compile/{}/{}:{}",
                        diagnostic.code, diagnostic.line, diagnostic.field
                    ),
                    source_ref: source_ref.to_string(),
                    source_hash: source_hash.to_string(),
                    evidence: crate::product::planning_failure::PlanningFailureEvidence {
                        affected_ids: Vec::new(),
                        details,
                    },
                }
            })
            .collect(),
    }
}

/// 判定面用的耐久授权事实（REQ-SG-05：外层从既有 grant/turn 读取，本
/// 函数不新增入站许可）：SC 修订授权=已接受 typed feedback 的
/// turn/command；物理 attempt 上限沿 `HUMAN_GATE_PROVIDER_MAX_ATTEMPTS`
///（L0/L1 共用同一 turn attempt key）。
fn sc_revision_authorization_facts(
    expected: &crate::product::models::WorkspaceSessionRecord,
    turn: &crate::product::models::HumanGateTurn,
) -> crate::product::planning_failure::StructuralAuthorizationFacts {
    let state = expected.structural_governance_state.as_ref();
    let grant = state.and_then(|state| state.current_grant.as_ref());
    crate::product::planning_failure::StructuralAuthorizationFacts {
        authorization_id: turn.turn_id.clone(),
        authorization_kind: crate::product::planning_failure::StructuralAuthorizationKind::ScTurn,
        parent_id: turn.command_id.clone(),
        source_hash: turn.source_hash.clone(),
        context_digest: state
            .and_then(|state| state.current_context_digest.clone())
            .unwrap_or_default(),
        active: matches!(
            turn.status,
            crate::product::models::HumanGateTurnStatus::Reserved
                | crate::product::models::HumanGateTurnStatus::Running
        ),
        anchor_waiting: grant.and_then(|grant| grant.phase).is_some_and(|phase| {
            phase == crate::product::models::workspace::StructuralGrantPhase::BlockedByLcRecovery
        }),
        launch_unknown: false,
        attempt_no: turn
            .attempt_no
            .max(grant.map(|grant| grant.attempt_no).unwrap_or(0)),
        max_attempts: HUMAN_GATE_PROVIDER_MAX_ATTEMPTS,
    }
}

/// 修订轮受保护业务事实（SG2 契约「值全从已确认/原授权读取」）：
/// - `business_request_digest`：本轮已接受人工反馈（原业务请求）的内容
///   摘要——修订必须保真的业务基线；
/// - `confirmed_design_digest`：plan 绑定的 Confirmed Design（与结构上
///   下文同读取口径，未绑定为 None 不猜）；
/// - `protected_field_refs`：REQ-SG-02A 点名的业务字段域（Inputs/Ac）；
/// - `protected_contract_refs`：候选 IR 声明的真实供需契约（未解析候选
///   无图事实，为空）；
/// - `authorized_member_ids`：结构上下文成员表的授权逻辑成员。
#[allow(clippy::too_many_arguments)]
fn sc_revision_protected_business_facts(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    plan: &crate::product::models::IssueWorkItemPlan,
    turn: &crate::product::models::HumanGateTurn,
    ir: Option<&crate::product::work_item_plan_compiler::PlanCandidateIr>,
    context: &crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureContext,
) -> crate::product::workspace_engine::contract_prerevision::ProtectedBusinessFacts {
    use sha2::{Digest, Sha256};

    let business_request_digest = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(turn.feedback_text.as_bytes()))
    );
    let confirmed = lifecycle
        .list_design_specs(&plan.project_id, &plan.issue_id)
        .ok()
        .and_then(|specs| {
            specs.into_iter().find(|design| {
                plan.source_design_spec_ids.contains(&design.id)
                    && design.confirmation_status
                        == crate::product::models::LifecycleConfirmationStatus::Confirmed
            })
        });
    let protected_contract_refs = ir
        .map(|ir| {
            ir.items
                .iter()
                .flat_map(|item| {
                    item.contract
                        .input_contracts
                        .iter()
                        .map(|contract| contract.contract_id.clone())
                })
                .collect()
        })
        .unwrap_or_default();
    let authorized_member_ids = context
        .physical_by_logical
        .keys()
        .map(|logical| logical.0.to_string())
        .collect();
    crate::product::workspace_engine::contract_prerevision::ProtectedBusinessFacts {
        business_request_digest,
        confirmed_design_digest: confirmed
            .map(|design| format!("sha256:{}", sc_revision_digest_json(&design))),
        protected_field_refs: vec!["Inputs".to_string(), "Acceptance Criteria".to_string()],
        protected_contract_refs,
        authorized_member_ids,
    }
}

impl super::WorkspaceEngine {
    /// SG6a.1（plan v1.1 §0.5 接口字典）：SC 修订轮输出判定面——对
    /// `output` 执行与候选系统门同源的全量结构评估（归一化 → compile →
    /// 结构门 → canonical/split/preflight），返回
    /// `Accepted/Rejected/ExistingGate/Operational` 四分支。判定面不写
    /// 终态（turn 不因拒收提前 Failed/Completed）、不做任何 provider
    /// I/O；`Accepted` 携 SG3a.0 冻结 payload 所需的同代
    ///
    /// 尚未接线生产 caller（SG6a.2 handler/SG6b recovery 消费）：沿
    /// SG5.2「尚未接线」先例门控 dead_code，不发布自动能力。
    #[allow(dead_code)]
    #[allow(clippy::result_large_err)]
    pub(crate) fn assess_sc_revision_output(
        &mut self,
        turn_id: &str,
        output: &str,
    ) -> Result<
        crate::product::workspace_engine::types::StructuralAssessment,
        crate::product::workspace_engine::types::PlanningCompileFailure,
    > {
        self.assess_sc_revision_output_full(turn_id, output)
            .map(|outcome| outcome.assessment)
    }

    /// 判定面实现（驱动面消费富载体）：失败原始输出留 immutable 尝试证
    /// 据（REQ-SG-02），分类经 `classify_failure_set` 纯函数（不读
    /// message/category/class_hint）。
    #[allow(clippy::result_large_err)]
    fn assess_sc_revision_output_full(
        &mut self,
        turn_id: &str,
        output: &str,
    ) -> Result<ScRevisionAssessment, crate::product::workspace_engine::types::PlanningCompileFailure>
    {
        use crate::product::models::HumanGateTurnStatus;
        use crate::product::planning_failure::PlanningRecoveryDisposition;
        use crate::product::work_item_plan_compiler::{
            PlanCandidateValidationContext, WorkItemPlanSourceContext, compile_work_item_plan,
            validate_plan_candidate_ir,
        };
        use crate::product::work_item_plan_source_store::{
            PlanCandidateIrRecord, SourceRevisionRecord, WorkItemPlanSourceStore,
        };
        use crate::product::workspace_engine::types::{
            PlanningCompileFailure, StructuralAssessment,
        };
        use sha2::{Digest, Sha256};

        let operational =
            |code: &str, cause: String| PlanningCompileFailure::operational(code, cause);
        let lifecycle = self.lifecycle_store.clone().ok_or_else(|| {
            operational(
                "lifecycle_store_unavailable",
                "lifecycle_store unavailable".to_string(),
            )
        })?;
        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                operational(
                    "sc_revision_session_load_failed",
                    format!("load session for revision assessment failed: {error}"),
                )
            })?;
        let turn = lifecycle
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| {
                operational(
                    "sc_revision_turn_load_failed",
                    format!(
                        "load human gate turn {turn_id} for revision assessment failed: {error}"
                    ),
                )
            })?;
        if turn.status != HumanGateTurnStatus::Running
            && turn.status != HumanGateTurnStatus::Reserved
        {
            return Err(operational(
                "human_gate_turn_not_active",
                format!("human gate turn {turn_id} is not active"),
            ));
        }
        let delivery = prepare_revision_delivery_for_compile(output);
        // SG6a.3（SG6a.2 复审范围外登记项处置）：失败候选身份绑定原始
        // provider 输出（REQ-SG-02「失败原始输出」）——claim 的
        // grant.failing_source_hash 与冻结 prompt 内嵌候选以 raw 为单源，
        // join 面的「基线==绑定」恒等成立（不再依赖「原始==归一化」前
        // 提）；编译/IR/结构上下文证据仍以归一化基（SG6a.1 单源不变），
        // 逐行归一化不改行号，typed 行引用对原始文本仍有效。
        let raw_source_hash = {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(output.as_bytes()))
        };
        let source = delivery.source;
        let plan = lifecycle
            .get_issue_work_item_plan(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| {
                operational(
                    "sc_revision_plan_load_failed",
                    format!("load plan for human gate revision failed: {error}"),
                )
            })?;
        // r30 根修(r29 现场):LC 会话的 story.repository_id 为空(Logical
        // story 生成落空串),直取 story 字段令 compile lowering_error
        // 「session context 缺少 target repository」→ turn validation_reject,
        // 门上修订对 LC 会话恒不可用。与 fresh 生成链(single_candidate.rs
        // 经 workspace_repository_for_session 取 repository.id)同源解析;
        // 解析失败回退 legacy story 字段口径。
        let (repository_id, member_targets) =
            match crate::product::workspace_repository::workspace_repository_for_session(
                &lifecycle.app_paths(),
                &lifecycle,
                &expected,
            ) {
                Ok(resolved) => {
                    // B1 铁律（多仓 plan 聚合视图缺口）：聚合视图会话的
                    // IR/WI 目标按每 WI 成员归属落成员物理 ID，聚合视图
                    // ID 不得作 compile target（Final Compile 拒收）。
                    let member_targets =
                        crate::product::workspace_engine::draft_batch::compile_support::plan_compile_member_targets(
                            &lifecycle, &plan, &resolved,
                        )
                        .map_err(|error| {
                            operational(
                                "sc_revision_member_targets_failed",
                                format!("resolve plan compile member targets failed: {error}"),
                            )
                        })?;
                    (resolved.id, member_targets)
                }
                Err(_) => (
                    self.work_item_plan_repository_id(&lifecycle, &plan)
                        .map_err(|error| {
                            operational(
                                "sc_revision_repository_resolve_failed",
                                format!("resolve revision target repository failed: {error}"),
                            )
                        })?,
                    None,
                ),
            };
        let repository_profile = plan
            .repository_profile_ref
            .as_deref()
            .map(|profile_id| {
                lifecycle.get_repository_profile(
                    &self.session.project_id,
                    &self.session.issue_id,
                    profile_id,
                )
            })
            .transpose()
            .map_err(|error| {
                operational(
                    "sc_revision_profile_load_failed",
                    format!("load repository profile for human gate revision failed: {error}"),
                )
            })?;

        // 尝试证据（REQ-SG-02「失败原始输出 SHALL 留作 attempt evidence，
        // 不成为可审核 current artifact」）：失败输出也 immutable 落
        // source store（内容寻址幂等，不提升 current refs）。
        let source_hash = hex::encode(Sha256::digest(source.as_bytes()));
        let source_id = format!("source-{}", &source_hash[..16]);
        let mut source_record = SourceRevisionRecord {
            id: source_id.clone(),
            source: source.clone(),
            source_revision_hash: source_hash.clone(),
            content_hash: String::new(),
        };
        source_record.content_hash = source_record.content_hash().map_err(|error| {
            operational(
                "SC_REVISION_SOURCE_HASH_FAILED",
                format!("hash revision attempt evidence failed: {}", error.code()),
            )
        })?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source_ref = source_store
            .put_source_revision(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
                &source_record,
            )
            .map_err(|error| {
                operational(
                    error.code(),
                    format!("persist revision attempt evidence failed: {error:?}"),
                )
            })?;

        // —— 编译与结构门（与 author 系统门同源：SG3a.1 单源装载/纯评估）——
        let mut failure_set;
        let mut rejection_context;
        let mut ir_for_instructions: Option<
            crate::product::work_item_plan_compiler::PlanCandidateIr,
        > = None;
        match compile_work_item_plan(
            &source,
            &WorkItemPlanSourceContext {
                target_repository_id: repository_id,
                member_targets: member_targets.clone(),
            },
        ) {
            Err(diagnostics) => {
                // parser 拒收：typed 诊断集合 + 未解析（零图事实）上下文。
                failure_set = sc_revision_failure_set_from_diagnostics(
                    &diagnostics,
                    &source_ref,
                    &source_hash,
                );
                rejection_context = sc_revision_pre_compile_context(
                    &source_ref,
                    &source_hash,
                    &sc_revision_digest_json(&plan.options),
                );
            }
            Ok(ir) => {
                let ir_id = format!("ir-{}", &source_hash[..16]);
                let mut ir_record = PlanCandidateIrRecord {
                    id: ir_id.clone(),
                    source_revision_id: source_id.clone(),
                    ir: ir.clone(),
                    content_hash: String::new(),
                };
                ir_record.content_hash = ir_record.content_hash().map_err(|error| {
                    operational(
                        "SC_REVISION_IR_HASH_FAILED",
                        format!("hash revision attempt IR failed: {}", error.code()),
                    )
                })?;
                let ir_ref = source_store
                    .put_plan_candidate_ir(
                        &self.session.project_id,
                        &self.session.issue_id,
                        &self.session.entity_id,
                        &ir_record,
                    )
                    .map_err(|error| {
                        operational(
                            error.code(),
                            format!("persist revision attempt IR failed: {error:?}"),
                        )
                    })?;
                let context = self
                    .build_single_candidate_structure_context(&ir, &source_ref, &ir_ref)
                    .map_err(|cause| {
                        operational(
                            "sc_revision_structure_context_failed",
                            format!("single-candidate structure context unavailable: {cause}"),
                        )
                    })?;
                match super::plan_projection::assess_single_candidate_structure(
                    source.clone(),
                    &ir,
                    &context,
                ) {
                    StructuralAssessment::Accepted(prepared) => {
                        // 结构门通过：canonical/split/preflight 全量同源
                        // 校验仍须通过（REQ-CG-03「全量同源结构门」）。
                        // F-56→REQ-PIB-03：plan 基线=issue 基线分支树。
                        let baseline_tree = super::plan_preflight::plan_baseline_tree(
                            &lifecycle,
                            &self.session.project_id,
                            &self.session.issue_id,
                        )
                        .map_err(|error| {
                            operational(
                                "sc_revision_baseline_load_failed",
                                format!("load plan baseline tree failed: {error}"),
                            )
                        })?;
                        // C1 Task 5（REQ-C1-PLAN-01）：existing 意图的授权
                        // 解析面——issue 下 durable 既有 work item id 集。
                        let existing_work_item_ids = lifecycle
                            .list_work_items(&self.session.project_id, &self.session.issue_id)
                            .map_err(|error| {
                                operational(
                                    "sc_revision_existing_items_failed",
                                    format!("list existing work items failed: {error}"),
                                )
                            })?
                            .into_iter()
                            .map(|record| record.id)
                            .collect::<Vec<_>>();
                        // 批 2 R1（REQ-WIGA-01）：enrolled 会话传真实
                        // binding+成员映射（装载失败 operational 不以 None
                        // 吞错）；Manual/非 enrolled 保持 None 零回归。
                        let enrollment_binding = self
                            .current_enrollment_binding_for_compile(
                                &lifecycle,
                                &self.session.entity_id,
                            )
                            .map_err(|error| {
                                operational(
                                    "sc_revision_enrollment_binding_failed",
                                    format!(
                                        "load enrollment binding for revision validation \
                                         failed: {error}"
                                    ),
                                )
                            })?;
                        let validation_member_targets = match &enrollment_binding {
                            Some(_) => member_targets.as_ref(),
                            None => None,
                        };
                        let validation_now = chrono::Utc::now().to_rfc3339();
                        match validate_plan_candidate_ir(
                            &ir,
                            &PlanCandidateValidationContext {
                                project_id: &self.session.project_id,
                                issue_id: &self.session.issue_id,
                                plan_id: &self.session.entity_id,
                                source_story_spec_ids: &plan.source_story_spec_ids,
                                source_design_spec_ids: &plan.source_design_spec_ids,
                                repository_profile: repository_profile.as_ref(),
                                // F-51：三生产路径之二（门内人工修订）——
                                // 存储 options 显式提供。
                                plan_options: &plan.options,
                                baseline_tree: baseline_tree.as_ref(),
                                existing_work_item_ids: &existing_work_item_ids,
                                enrollment_target: enrollment_binding
                                    .as_ref()
                                    .map(|binding| &binding.target),
                                member_targets: validation_member_targets,
                                now: &validation_now,
                            },
                        ) {
                            Ok(report) => {
                                return Ok(ScRevisionAssessment {
                                    assessment: StructuralAssessment::Accepted(prepared),
                                    accepted: Some(ScRevisionAcceptedFacts {
                                        source_id,
                                        source_ref,
                                        ir_id,
                                        ir_ref,
                                        report,
                                    }),
                                    rejection: None,
                                });
                            }
                            Err(diagnostics) => {
                                failure_set = sc_revision_failure_set_from_diagnostics(
                                    &diagnostics,
                                    &source_ref,
                                    &source_hash,
                                );
                                rejection_context = context;
                                ir_for_instructions = Some(ir);
                            }
                        }
                    }
                    structure_rejection => {
                        // 结构门失败（typed 集合已在评估分支内）。
                        failure_set = match &structure_rejection {
                            StructuralAssessment::Rejected(set, _)
                            | StructuralAssessment::ExistingGate(set)
                            | StructuralAssessment::Operational(set) => set.clone(),
                            StructuralAssessment::Accepted(_) => unreachable!(),
                        };
                        rejection_context = context;
                        ir_for_instructions = Some(ir);
                    }
                }
            }
        }

        // —— 分类（classify_failure_set 纯函数：不读 message/category/
        // class_hint；同 (set, facts) 恒等输出）——
        let facts = sc_revision_authorization_facts(&expected, &turn);
        match crate::product::planning_failure::classify_failure_set(&failure_set, &facts) {
            PlanningRecoveryDisposition::StructuralAuthor => {
                // L1：机器事实完整 + 活动授权 + 剩余物理 attempt → 生成
                // 精确修复指令（SG2 纯 producer）。指令身份载体：编译失
                // 败用未解析 shell（零图事实）；结构门失败用同输入 SG1
                // 单源重投影（纯确定性，与评估报告恒等——复用
                // planning_structure_items_from_ir/project_effective_
                // planning_graph 同源函数，不建第二图权威）。
                // SG6a.3：失败/指令的 source_hash 统一重绑原始输出 hash
                //（claim 绑定与 join 前提单源，见 assess 头注）；typed
                // kind/witness/行号载荷不动。
                for failure in failure_set.failures.iter_mut() {
                    failure.source_hash = raw_source_hash.clone();
                }
                // SG6a.3 P2（续）：指令身份载体是（context,report）对——
                // prompt 构建器 join instruction.source_hash 与
                // context.source_hash（SG5.2 失配分支），context 同步重绑
                // raw 并按单源配方重算 context_digest，指令/上下文/claim
                // 绑定三者同源；编译/IR/refs 证据仍归一化基。
                rejection_context.source_hash = raw_source_hash.clone();
                rejection_context.context_digest =
                    super::plan_projection::compute_planning_context_digest(&rejection_context);
                let mut instruction_report = match &ir_for_instructions {
                    None => sc_revision_pre_compile_report(&rejection_context, failure_set.clone()),
                    Some(ir) => {
                        let mut items =
                            super::plan_projection::planning_structure_items_from_ir(ir);
                        if rejection_context.physical_by_logical.is_empty() {
                            for item in &mut items {
                                item.declared_member = None;
                                item.logical_target = None;
                            }
                        }
                        crate::product::workspace_engine::draft_batch::compile_support::project_effective_planning_graph(
                            &items, &rejection_context,
                        )
                    }
                };
                // SG6a.3 P2（Main 审查）：指令身份载体（report→instruction
                // →claim 的 grant.failing_source_hash）同样重绑原始输出
                // hash——否则非 canonical 候选（前言/归一化差异）join 以
                // raw 基线对归一化绑定恒 CANDIDATE_DRIFTED 假拒。report
                // 仅作本指令构建的纯输入，不落盘、不影响 refs/freshness
                //（编译/IR/结构上下文证据仍归一化基）。
                instruction_report.source_hash = raw_source_hash.clone();
                let protected = sc_revision_protected_business_facts(
                    &lifecycle,
                    &plan,
                    &turn,
                    ir_for_instructions.as_ref(),
                    &rejection_context,
                );
                let instructions =
                    crate::product::workspace_engine::contract_prerevision::build_structural_repair_instructions(
                        &failure_set,
                        &instruction_report,
                        &protected,
                    );
                if instructions.is_empty() {
                    // classify 判 L1 但 producer 零指令（保护事实缺失/
                    // fail-closed）：回原门，不驱 author。
                    return Ok(ScRevisionAssessment {
                        assessment: StructuralAssessment::ExistingGate(failure_set),
                        accepted: None,
                        rejection: None,
                    });
                }
                Ok(ScRevisionAssessment {
                    assessment: StructuralAssessment::Rejected(failure_set, instructions),
                    accepted: None,
                    rejection: Some(ScRevisionRejectionFacts {
                        context: rejection_context,
                        failed_source: output.to_string(),
                    }),
                })
            }
            PlanningRecoveryDisposition::OperationalWait => Ok(ScRevisionAssessment {
                assessment: StructuralAssessment::Operational(failure_set),
                accepted: None,
                rejection: None,
            }),
            PlanningRecoveryDisposition::UnknownStop => {
                if facts.attempt_no >= facts.max_attempts {
                    // 超限 typed 系统故障（REQ-SG-05：L0/L1 共用同 turn
                    // attempt key，无第 3 次；终态收口/release hold 归
                    // SG6a.3）。
                    let rendered = render_sc_revision_failure_set(&failure_set).join("; ");
                    Err(operational(
                        "STRUCTURAL_ATTEMPTS_EXHAUSTED",
                        format!(
                            "physical attempts exhausted (attempt {} of {}); typed system fault, recent failures: {rendered}",
                            facts.attempt_no, facts.max_attempts
                        ),
                    ))
                } else {
                    // 未 typed/事实不完整失败：机器不可证明可修——回原
                    // 门（类别与 class_hint 不构成许可）。
                    Ok(ScRevisionAssessment {
                        assessment: StructuralAssessment::ExistingGate(failure_set),
                        accepted: None,
                        rejection: None,
                    })
                }
            }
            // classify 对非空集合不产生 TechnicalRoute/BusinessGate（空集
            // 合已在 Accepted 早退；防御分支同走原门）。
            PlanningRecoveryDisposition::TechnicalRoute
            | PlanningRecoveryDisposition::BusinessGate => Ok(ScRevisionAssessment {
                assessment: StructuralAssessment::ExistingGate(failure_set),
                accepted: None,
                rejection: None,
            }),
        }
    }
}

/// SG6b P2 修复轮（Main 审查派回）：claim 成功后、冻结件落盘前的崩溃窗
/// 模拟开关（仅测试）。开启时该窗内 panic，消费方断言冻结证据在 claim
/// 前已持久化（内容寻址幂等）。
#[cfg(test)]
pub(crate) static SG6B_CLAIM_PERSIST_CRASH: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
