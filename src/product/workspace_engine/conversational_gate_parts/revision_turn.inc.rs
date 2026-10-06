impl super::WorkspaceEngine {
    pub(crate) async fn run_sc_manual_revision_turn(
        &mut self,
        turn_id: &str,
        provider_output: String,
    ) -> Result<ScManualRevisionResult, String> {
        use crate::product::models::{HumanGateTurnFailureClass, HumanGateTurnStatus};
        use crate::product::work_item_plan_compiler::{
            PlanCandidateValidationContext, WorkItemPlanSourceContext, compile_work_item_plan,
            validate_plan_candidate_ir,
        };
        use crate::product::work_item_plan_source_store::{
            PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
            WorkItemPlanSourceStore,
        };
        use sha2::{Digest, Sha256};

        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let mut expected = lifecycle
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
        let source = delivery.source;
        let plan = lifecycle
            .get_issue_work_item_plan(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load plan for human gate revision failed: {error}"))?;
        // r30 根修(r29 现场):LC 会话的 story.repository_id 为空(Logical
        // story 生成落空串),直取 story 字段令 compile lowering_error
        // 「session context 缺少 target repository」→ turn validation_reject,
        // 门上修订对 LC 会话恒不可用。与 fresh 生成链(single_candidate.rs
        // 经 workspace_repository_for_session 取 repository.id)同源解析;
        // 解析失败回退 legacy story 字段口径。
        let repository_id =
            match crate::product::workspace_repository::workspace_repository_for_session(
                &lifecycle.app_paths(),
                &lifecycle,
                &expected,
            ) {
                Ok(resolved) => resolved.id,
                Err(_) => self.work_item_plan_repository_id(&lifecycle, &plan)?,
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
                format!("load repository profile for human gate revision failed: {error}")
            })?;

        let ir = match compile_work_item_plan(
            &source,
            &WorkItemPlanSourceContext {
                target_repository_id: repository_id,
            },
        ) {
            Ok(ir) => ir,
            Err(diagnostics) => {
                let messages = diagnostics
                    .iter()
                    .map(|diagnostic| {
                        format!(
                            "{}:{}:{}",
                            diagnostic.code, diagnostic.line, diagnostic.message
                        )
                    })
                    .collect();
                let mut failed = turn.clone();
                failed.status = HumanGateTurnStatus::Failed;
                failed.failure_class = Some(HumanGateTurnFailureClass::ValidationReject);
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                expected = lifecycle
                    .update_human_gate_turn(&expected, failed)
                    .map_err(|error| error.to_string())?;
                self.session.provider_start_ledger = expected.provider_start_ledger;
                return Ok(ScManualRevisionResult::ValidationRejected {
                    diagnostics: messages,
                });
            }
        };
        // F-56→REQ-PIB-03：门内人工修订与 author 路径共用基线加载（按分支名
        // 取树）；不可解析 → Err 直通 fail-closed（废弃跳过）。
        let baseline_tree = super::plan_preflight::plan_baseline_tree(
            &lifecycle,
            &self.session.project_id,
            &self.session.issue_id,
        )?;
        // C1 Task 5（REQ-C1-PLAN-01）：existing 意图的授权解析面。
        let existing_work_item_ids = lifecycle
            .list_work_items(&self.session.project_id, &self.session.issue_id)
            .map_err(|error| format!("list existing work items failed: {error}"))?
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>();
        let validation_now = chrono::Utc::now().to_rfc3339();
        let report = match validate_plan_candidate_ir(
            &ir,
            &PlanCandidateValidationContext {
                project_id: &self.session.project_id,
                issue_id: &self.session.issue_id,
                plan_id: &self.session.entity_id,
                source_story_spec_ids: &plan.source_story_spec_ids,
                source_design_spec_ids: &plan.source_design_spec_ids,
                repository_profile: repository_profile.as_ref(),
                // F-51：三生产路径之二（门内人工修订）——存储 options 显式提供。
                plan_options: &plan.options,
                baseline_tree: baseline_tree.as_ref(),
                existing_work_item_ids: &existing_work_item_ids,
            enrollment_target: None,
            now: &validation_now,
            },
        ) {
            Ok(report) => report,
            Err(diagnostics) => {
                let messages = diagnostics
                    .iter()
                    .map(|diagnostic| {
                        format!(
                            "{}:{}:{}",
                            diagnostic.code, diagnostic.line, diagnostic.message
                        )
                    })
                    .collect();
                let mut failed = turn.clone();
                failed.status = HumanGateTurnStatus::Failed;
                failed.failure_class = Some(HumanGateTurnFailureClass::ValidationReject);
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                expected = lifecycle
                    .update_human_gate_turn(&expected, failed)
                    .map_err(|error| error.to_string())?;
                self.session.provider_start_ledger = expected.provider_start_ledger;
                return Ok(ScManualRevisionResult::ValidationRejected {
                    diagnostics: messages,
                });
            }
        };

        let source_hash = hex::encode(Sha256::digest(source.as_bytes()));
        let source_id = format!("source-{}", &source_hash[..16]);
        let mut source_record = SourceRevisionRecord {
            id: source_id.clone(),
            source: source.clone(),
            source_revision_hash: source_hash.clone(),
            content_hash: String::new(),
        };
        source_record.content_hash = source_record
            .content_hash()
            .map_err(|error| error.code().to_string())?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source_ref = source_store
            .put_source_revision(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
                &source_record,
            )
            .map_err(|error| error.code().to_string())?;
        let ir_id = format!("ir-{}", &source_hash[..16]);
        let mut ir_record = PlanCandidateIrRecord {
            id: ir_id.clone(),
            source_revision_id: source_id.clone(),
            ir,
            content_hash: String::new(),
        };
        ir_record.content_hash = ir_record
            .content_hash()
            .map_err(|error| error.code().to_string())?;
        let ir_ref = source_store
            .put_plan_candidate_ir(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
                &ir_record,
            )
            .map_err(|error| error.code().to_string())?;
        let report_id = format!("report-{}", &source_hash[..16]);
        let mut report_record = PlanCandidateMechanicalReportRecord {
            id: report_id,
            source_revision_id: source_id,
            ir_id,
            report,
            content_hash: String::new(),
        };
        report_record.content_hash = report_record
            .content_hash()
            .map_err(|error| error.code().to_string())?;
        let report_ref = source_store
            .put_mechanical_report(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
                &report_record,
            )
            .map_err(|error| error.code().to_string())?;
        let mut completed = turn;
        completed.status = HumanGateTurnStatus::Completed;
        completed.failure_class = None;
        let artifact_versions = {
            let mut versions = self.artifact_versions.clone();
            for version in &mut versions {
                version.is_current = false;
            }
            let version = versions.len() as u32 + 1;
            versions.push(crate::web::workspace_ws_types::ArtifactVersion {
                version,
                payload: crate::web::workspace_ws_types::ArtifactPayload::Markdown {
                    markdown: source.clone(),
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
        let saved = lifecycle
            .complete_human_gate_revision(
                &expected,
                completed,
                &source_ref,
                &ir_ref,
                &report_ref,
                &artifact_versions,
                &artifact_id,
            )
            .map_err(|error| error.to_string())?;
        self.artifact_versions = artifact_versions;
        self.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
            markdown: source,
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
            self.complete_active_node(Some("已按人工反馈完成修订并持久化，等待复评".to_string()))
                .await;
        }
        // 人工修订与初始 author 同构：候选落盘后必须重走 Evaluate policy route。
        // Evaluate 路由仍是进 Approval 的主路径；close 时的人工权威升级（confirm
        // 视为批准权威，在 close CAS 内原子提升 phase→Approval）是兜底，二者不
        // 互相替代：路由负责让门以 Approval 相位等待，升级负责解开路由缺席/评审
        // 再次 must_fix 时 Evaluate 进门 × Approval 关门的死锁。无 reviewer 走本地
        // synthetic Pass 路由进 Approval；有 reviewer 重启评审，不让 close 绕过评审。
        if self.session.review_rounds == 0 || self.session.reviewer_provider.is_none() {
            self.route_single_candidate_evaluate_without_reviewer()
                .await;
        } else {
            self.start_review().await;
            if self.session.stage == super::WorkspaceStage::CrossReview {
                self.request_provider_run(super::ProviderRunKind::ReviewOnly)
                    .await;
            }
        }
        Ok(ScManualRevisionResult::Accepted {
            artifact_ref: artifact_id,
        })
    }
}
