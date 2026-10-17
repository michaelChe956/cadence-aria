use super::*;

impl CodingWorkspaceEngine {
    pub async fn execute_code_review(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<CodeReviewReport, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_code_review_with_commands(attempt, provider, &mut command_rx)
            .await
    }

    pub async fn execute_code_review_with_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodeReviewReport, CodingWorkspaceEngineError> {
        let attempt =
            self.admit_provider_run(attempt, &CodingExecutionStage::CodeReview, "execute")?;
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        // C2 Task 5（REQ-CRO-05）：reviewer 三值——进入 Code Reviewer 阶段而快照
        // 该角色为空时，在建 role run／timeline node 之前落缺配置门，绝不以 author 顶替。
        let reviewer = match self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .code_reviewer
        {
            Some(reviewer) => reviewer,
            None => {
                return Err(self
                    .block_missing_reviewer_configuration(
                        &attempt,
                        CodingExecutionStage::CodeReview,
                        CodingProviderRole::CodeReviewer,
                    )
                    .await);
            }
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )?;
        let node = self.create_code_review_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let role_run = match self.store.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
        )? {
            Some(run) if run.status == CodingRoleRunStatus::Running && run.node_id.is_none() => {
                self.store.attach_role_run_node(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &run.id,
                    node.id.clone(),
                )?
            }
            _ => self.store.create_role_run(
                &attempt,
                CodingExecutionStage::CodeReview,
                CodingProviderRole::CodeReviewer,
                CodingRoleRunTrigger::Initial,
                Some(node.id.clone()),
            )?,
        };
        let initial_resume_provider_session_id = attempt
            .provider_conversations
            .iter()
            .find(|conversation| {
                conversation.role == ProviderConversationRole::CodeReviewer
                    && conversation.provider == reviewer
            })
            .map(|conversation| conversation.provider_session_id.trim().to_string())
            .filter(|session_id| !session_id.is_empty());
        // T19:开窗必须在 reviewer_context/Git diff/material 采集之前——
        // 重试循环内的 prompt 构建/reviewer_context 渲染全部发生在窗口
        // Running 之后(材料与 baseline manifest 同指纹)。
        let initial_window_id = format!("{}-w01", role_run.id);
        let mut review_window = self
            .begin_reviewer_window_for_run(
                &initial_window_id,
                &attempt,
                &role_run.id,
                &reviewer,
                worktree_path,
            )
            .await?;
        let retry_success = match self
            .run_code_reviewer_with_retry_cycle(CodeReviewerRetryCycleInput {
                attempt: &attempt,
                initial_node: node,
                initial_role_run: role_run,
                provider,
                reviewer: &reviewer,
                worktree_path,
                initial_resume_provider_session_id,
                command_rx,
                review_window: &mut review_window,
            })
            .await
        {
            Ok(success) => success,
            Err(error) => {
                // T19:取消/技术失败逃逸——窗口收口恢复,锁保持到恢复完成
                // (干净恢复即释放;违规进人工门;停等交 T21/人工)。收口
                // 错误只记录,不掩盖原始错误。
                self.close_reviewer_window_after_failure(
                    &review_window,
                    "code_review_failed_or_cancelled",
                )
                .await;
                return Err(error);
            }
        };
        let outcome = retry_success.outcome;
        let role_run = retry_success.role_run;
        let node = retry_success.node;
        let full_output = outcome.full_output.clone();
        // F2(修复轮 1):成功后的落盘链(raw output/refs/收口/报告构建)
        // 任一失败都必须先收口窗口再返回——否则 Running/Quiescing 窗口
        // 携租约与四通道冻结永久残留(后续开窗/Coder/人工重试全被拒)。
        let persisted: Result<(CodeReviewReport, String), CodingWorkspaceEngineError> = async {
            let raw_provider_output_ref = self.store.save_provider_raw_output(
                &attempt,
                CodingExecutionStage::CodeReview,
                "code_review",
                &full_output,
            )?;
            self.store.update_role_run_refs(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &role_run.id,
                vec![raw_provider_output_ref.clone()],
                Vec::new(),
            )?;
            // T19:收口先于 save——Quiescing/落违规/恢复/采信完成后,
            // 保存的才是可消费完成事实;Pending/Invalidated 永不驱动
            // approve/交付。
            let window_outcome = self
                .finalize_reviewer_window_with_report(&review_window)
                .await?;
            let mut report = self.build_code_review_report(
                &attempt,
                outcome,
                Some(raw_provider_output_ref.clone()),
                &role_run,
            )?;
            report.review_window_id = Some(window_outcome.window_id.clone());
            report.acceptance = Some(wire_acceptance(window_outcome.acceptance));
            Ok((report, raw_provider_output_ref))
        }
        .await;
        let (report, raw_provider_output_ref) = match persisted {
            Ok(pair) => pair,
            Err(error) => {
                self.close_reviewer_window_after_failure(
                    &review_window,
                    "code_review_post_run_persist_failed",
                )
                .await;
                return Err(error);
            }
        };
        let plan_defect_source = PlanDefectSource::CodeReviewer;
        let reviewer_projection = self.reviewer_projection_for_attempt(&attempt)?;
        let plan_defect_route = code_review_flow_decision(&report, &reviewer_projection);
        // T19 采信门:窗口未采信(Pending/Invalidated)的报告即使内容
        // approve 也不驱动推进——复用既有 blocked gate 链落人工分诊门,
        // 不重建第二条关闭门链。
        let report_blocks_on_acceptance = report.verdict == ReviewVerdict::Approve
            && matches!(
                report.acceptance,
                Some(
                    crate::product::coding_models::ReviewReportAcceptance::PendingAcceptance
                        | crate::product::coding_models::ReviewReportAcceptance::Invalidated
                )
            );
        self.store.save_code_review_report(&attempt, &report)?;
        if let Err(error) =
            self.store
                .write_snapshot_for_code_review_report(&attempt, &report, &full_output)
        {
            if let Err(rollback_error) = self.store.delete_code_review_report(&attempt, &report.id)
            {
                return Err(CodingWorkspaceEngineError::Store(ProductStoreError::Io(
                    format!(
                        "code_review_snapshot_write_failed: {error}; report_rollback_failed: {rollback_error}"
                    ),
                )));
            }
            return Err(error.into());
        }
        // C2 Task 1（REQ-CRO-01）durable-first：完成事实（role run 状态、timeline
        // 完成、分诊门禁、chat entry）全部先经既有 store 写面落盘，之后才发射
        // `CodeReviewComplete` 观察事件——断连／观察通道失效不回滚业务事实，
        // 驾驶舱重连经 GET 补读到同一结果。范式对齐 coder 侧 coding.rs 的
        // save_provider_raw_output→role_run refs→status→timeline→emit 次序。
        let (node_status, summary, role_run_status, reason_code) = match report.verdict {
            ReviewVerdict::Approve => (
                CodingTimelineNodeStatus::Completed,
                Some("code review 通过".to_string()),
                CodingRoleRunStatus::Completed,
                None,
            ),
            ReviewVerdict::RequestChanges => (
                CodingTimelineNodeStatus::Failed,
                Some("code review 要求修改".to_string()),
                CodingRoleRunStatus::Completed,
                None,
            ),
            ReviewVerdict::Blocked => (
                CodingTimelineNodeStatus::Blocked,
                Some("code review 被阻塞".to_string()),
                CodingRoleRunStatus::Blocked,
                Some("code_review_blocked".to_string()),
            ),
        };
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status,
            summary,
        )
        .await?;
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            role_run_status,
            reason_code,
        )?;
        let lands_code_review_blocked = report.verdict == ReviewVerdict::Blocked
            && !code_review_report_has_actionable_findings(&report);
        if lands_code_review_blocked {
            self.create_review_blocked_gate(ReviewBlockedGateInput {
                attempt: &attempt,
                node_id: &node.id,
                stage: CodingExecutionStage::CodeReview,
                role: CodingProviderRole::CodeReviewer,
                title: "Code review blocked".to_string(),
                description: report.summary.clone(),
                reason_code: "code_review_blocked",
                evidence_refs: vec![report.id.clone()],
                raw_provider_output_ref: Some(raw_provider_output_ref.clone()),
            })
            .await?;
        }
        // 既有的 code_review_blocked 门禁已落地时，不再重复落地分诊门禁。
        // 当 lands_code_review_blocked 为真（verdict==Blocked && !actionable），该情形
        // 已被 plan_defect.rs 判定为 StopForHumanTriage（plan_defect.rs:177-178），
        // 由上面分支统一落 code_review_blocked gate；新分诊门禁必须跳过，避免 double-gate。
        if !lands_code_review_blocked {
            // 仅处理三个分诊决策；RunCoderFix / StartPlanRepair /
            // ContinueAfterApprove / StartStoryAmendment / StartDesignAmendment 由
            // runner 后续处理，不在此落门禁。门禁动作不触发 plan repair。
            let triage_reason_code: Option<(&'static str, &'static str)> =
                if report_blocks_on_acceptance {
                    Some((
                        "code_review_report_not_accepted",
                        "Code Review 报告未被窗口采信(Pending/Invalidated)",
                    ))
                } else {
                    match plan_defect_route {
                        CodeReviewFlowDecision::StopForHumanTriage => Some((
                            "code_review_output_human_triage",
                            "Code Review 结论需人工分诊",
                        )),
                        CodeReviewFlowDecision::RetryVerification => Some((
                            "code_review_verification_incomplete",
                            "Code Review 验证证据不完整",
                        )),
                        CodeReviewFlowDecision::OpenOperationalGate => Some((
                            "code_review_operational_blocker",
                            "Code Review 命中运维阻塞",
                        )),
                        _ => None,
                    }
                };
            if let Some((reason_code, title)) = triage_reason_code {
                self.create_review_blocked_gate(ReviewBlockedGateInput {
                    attempt: &attempt,
                    node_id: &node.id,
                    stage: CodingExecutionStage::CodeReview,
                    role: CodingProviderRole::CodeReviewer,
                    title: title.to_string(),
                    description: report.summary.clone(),
                    reason_code,
                    evidence_refs: vec![report.id.clone()],
                    raw_provider_output_ref: Some(raw_provider_output_ref.clone()),
                })
                .await?;
            }
        }
        self.emit_code_review_chat_entry(
            &attempt,
            &node.id,
            &report,
            plan_defect_source.label(),
            plan_defect_route.label(),
        )
        .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodeReviewComplete {
                report: Box::new(report.clone()),
            })
            .await;
        Ok(report)
    }
}

// ---------------------------------------------------------------------------
// T19:reviewer 窗口生命周期(受信产品服务入口;runner 与补偿共用)
// ---------------------------------------------------------------------------
//
// lock order(显式维护,防死锁):窗口 worktree lease → attempt mutation/
// recovery arbitration → 具体 store/Git。任何路径都不得在持有 attempt
// 仲裁锁后再等待窗口租约释放。
//
// 未实现边界(登记,不假装已覆盖——RevB2T18 ⚠1/⚠2):
// - 平台合法写入域的「精确 expected 变化/actor」观察方(T5/fs 监视)
//   尚无生产实现:窗口 platform_write_paths 传空(平台承诺冻结期间无
//   域外合法写),allowlist 内 actor 区分留给观察方接线;
// - `.aria` 直写(allowlist 外平台写)的检测观察方无生产实现:
//   违规事实仍由 T16/T18 恢复链观察(Git 域/受管文件),`.aria` 面
//   只有记录一致性对账兜底。

use crate::product::reviewer_reconciliation::reconcile_from_stores;
use crate::product::reviewer_snapshot::ReviewerSnapshotStore;
use crate::product::reviewer_snapshot::SnapshotBudget;
use crate::product::reviewer_window::{
    ReviewerWindowError, WindowExpectation, abandon_window, assert_review_window_current,
    begin_quiesce, begin_review_window, close_accepted, enter_platform_write, exit_platform_write,
    record_report_acceptance, resolve_worktree_git_identity, restore_window,
};
use crate::product::reviewer_window_store::{
    ChannelWriter, ReportAcceptance, ReviewerWindowState, ReviewerWindowStore, WriteChannelKind,
    wire_acceptance,
};

/// 窗口 store 生产根(`<aria>/reviewer-windows`;engine 与 coding_attempt_store
/// 侧通道守卫共用同一约定,不得另立第二处根)。
pub(crate) const REVIEWER_WINDOW_STORE_DIR: &str = "reviewer-windows";
/// 快照 store 生产根(`<aria>/reviewer-snapshots`)。
pub(crate) const REVIEWER_SNAPSHOT_STORE_DIR: &str = "reviewer-snapshots";
/// T5 资源登记生产根(`<aria>/provider-resources`;quiesce 协调 scope 枚举源)。
pub(crate) const PROVIDER_RESOURCE_STORE_DIR: &str = "provider-resources";

/// 窗口收口结果(T19;消费面只认这里/`reconcile_from_stores`,不复制判定)。
pub(crate) struct ReviewerWindowOutcome {
    pub(crate) window_id: String,
    /// 权威采信投影(同源窗口记录;报告 DTO 只是 wire 形状)。
    pub(crate) acceptance: ReportAcceptance,
}

fn reviewer_provider_wire_name(provider: &ProviderName) -> String {
    serde_json::to_string(provider)
        .map(|raw| raw.trim_matches('"').to_string())
        .unwrap_or_else(|_| format!("{provider:?}"))
}

fn reviewer_window_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 平台自身 git mutation 的通道 writer 身份(PID-start 防复用;崩溃后由
/// 排空侧收割——与 T5 同构的 unix 观察语义)。
pub(crate) fn platform_git_channel_writer(attempt_id: &str) -> ChannelWriter {
    ChannelWriter {
        writer_id: format!("platform-git-{attempt_id}"),
        boot_identity: format!("aria-platform-{}", std::process::id()),
        pid: std::process::id(),
        start_time_ticks: crate::cross_cutting::process_manager::process_start_time_ticks(
            std::process::id(),
        )
        .unwrap_or(0),
        entered_at: reviewer_window_rfc3339(),
    }
}

impl CodingWorkspaceEngine {
    /// 平台 git mutation 通道准入:窗口冻结期间拒绝(stage/commit/push/
    /// 补偿共用;受信产品服务入口调用,不是 REST/WS 按钮)。
    pub(crate) fn guard_platform_write_channel(
        &self,
        worktree: &Path,
        channel: WriteChannelKind,
        attempt_id: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        enter_platform_write(
            &self.reviewer_window_store(),
            worktree,
            channel,
            platform_git_channel_writer(attempt_id),
        )
        .map_err(|error| {
            CodingWorkspaceEngineError::Store(crate::product::json_store::ProductStoreError::Io(
                format!("reviewer_window_channel_blocked: {error}"),
            ))
        })
    }

    /// 通道退出(best-effort:退出允许于 Draining/Frozen;失败仅告警。
    /// 残留 writer 由窗口排空按 PID-start 收割/停等,fail-closed)。
    pub(crate) fn release_platform_write_channel(
        &self,
        worktree: &Path,
        channel: WriteChannelKind,
        attempt_id: &str,
    ) {
        if let Err(error) = exit_platform_write(
            &self.reviewer_window_store(),
            worktree,
            channel,
            &format!("platform-git-{attempt_id}"),
        ) {
            tracing::warn!(
                ?error,
                channel = channel.as_str(),
                "reviewer_window_channel_exit_failed"
            );
        }
    }
}

impl CodingWorkspaceEngine {
    pub(crate) fn reviewer_window_store(&self) -> ReviewerWindowStore {
        ReviewerWindowStore::new(self.store.paths().root().join(REVIEWER_WINDOW_STORE_DIR))
    }

    pub(crate) fn reviewer_snapshot_store(&self) -> ReviewerSnapshotStore {
        ReviewerSnapshotStore::new(self.store.paths().root().join(REVIEWER_SNAPSHOT_STORE_DIR))
    }

    /// T5 quiesce 目标:主 scope = reviewer run 自身(空登记的 Quiescent
    /// 即合法静止证明);协调 scope = 资源 store 中登记了 `aria.worktree`
    /// == 本树 tree_key 的兄弟 scope(RevB2T17 ⚠1 硬前置;T14/T20 落地
    /// Coder/terminal 登记时打上该标签即自动纳入静止——当前生产无登记,
    /// 集合恒为空,不误杀无关 scope)。
    fn reviewer_quiesce_targets(
        &self,
        window_id: &str,
        attempt: &CodingExecutionAttempt,
        run_id: &str,
        worktree: &Path,
    ) -> Result<
        Vec<crate::cross_cutting::provider_resources::ProviderRunResourceScope>,
        CodingWorkspaceEngineError,
    > {
        use crate::cross_cutting::provider_resources::{
            ProviderRunResourceScope, ResourceScopeKey,
        };
        let store_root = self.store.paths().root().join(PROVIDER_RESOURCE_STORE_DIR);
        let resource_store =
            crate::product::provider_resource_store::ProviderResourceStore::new(&store_root);
        // 主 scope:本 reviewer run(绑定窗口)。
        let private_root = self.store.provider_stream_log_root(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        );
        std::fs::create_dir_all(&private_root).map_err(|error| {
            CodingWorkspaceEngineError::Store(crate::product::json_store::ProductStoreError::Io(
                format!(
                    "reviewer_scope_private_root_create_failed: {}: {error}",
                    private_root.display()
                ),
            ))
        })?;
        let key = ResourceScopeKey {
            scope_id: format!("reviewer-window-{}", attempt.id),
            run_id: run_id.to_string(),
            boot_identity: format!("aria-engine-{}", std::process::id()),
        };
        let main_scope = ProviderRunResourceScope::open_for_recovery(
            &key,
            Some(window_id),
            &private_root,
            &store_root,
            None,
        )
        .map_err(|error| {
            CodingWorkspaceEngineError::Store(crate::product::json_store::ProductStoreError::Io(
                format!("reviewer_scope_open_failed: {error}"),
            ))
        })?;
        let mut targets = vec![main_scope];

        // 协调 scope:按 aria.worktree 归属标签过滤(未标注归属的不静止——
        // 归属注入是 T14/T20 的登记合同,本 Task 只消费)。
        let tree_key = crate::product::reviewer_window_store::tree_key_of(
            &worktree.canonicalize().map_err(|error| {
                CodingWorkspaceEngineError::Store(
                    crate::product::json_store::ProductStoreError::Io(format!(
                        "worktree canonicalize 失败: {error}"
                    )),
                )
            })?,
        );
        for sibling in resource_store
            .scope_keys()
            .map_err(CodingWorkspaceEngineError::Store)?
        {
            if sibling == key {
                continue;
            }
            let records = resource_store
                .list_scope(&sibling)
                .map_err(CodingWorkspaceEngineError::Store)?;
            let owned = records.iter().any(|record| {
                record
                    .reserved_labels
                    .get("aria.worktree")
                    .is_some_and(|labelled| labelled == &tree_key)
            });
            if !owned {
                continue;
            }
            let private = records
                .iter()
                .find_map(|record| record.canonical_private_path.as_ref().map(PathBuf::from))
                .unwrap_or_else(|| private_root.clone());
            let Ok(private) = private.canonicalize() else {
                continue;
            };
            if let Ok(scope) = ProviderRunResourceScope::open_for_recovery(
                &sibling,
                Some(window_id),
                &private,
                &store_root,
                None,
            ) {
                targets.push(scope);
            }
        }
        Ok(targets)
    }

    /// 开窗:必须在 reviewer_context/Git diff/material 采集之前调用
    /// (T17 冻结顺序:租约→通道锁→静止→baseline→Running)。
    pub(crate) async fn begin_reviewer_window_for_run(
        &self,
        window_id: &str,
        attempt: &CodingExecutionAttempt,
        run_id: &str,
        provider: &ProviderName,
        worktree: &Path,
    ) -> Result<crate::product::reviewer_window::ReviewerWindowHandle, CodingWorkspaceEngineError>
    {
        let targets = self.reviewer_quiesce_targets(window_id, attempt, run_id, worktree)?;
        let target_refs: Vec<&crate::cross_cutting::provider_resources::ProviderRunResourceScope> =
            targets.iter().collect();
        let request = crate::product::reviewer_window::ReviewerWindowBeginRequest {
            window_id,
            attempt_id: &attempt.id,
            run_id,
            provider: &reviewer_provider_wire_name(provider),
            worktree,
            quiesce_targets: &target_refs,
            // 平台承诺冻结期间无域外合法写(精确 expected 变化/actor 观察
            // 方未实现——见模块头未实现边界登记)。
            platform_write_paths: Vec::new(),
            budget: SnapshotBudget {
                max_entries: 200_000,
                max_total_bytes: 2 * 1024 * 1024 * 1024,
            },
            drain_timeout: std::time::Duration::from_secs(60),
        };
        begin_review_window(
            &self.reviewer_window_store(),
            &self.reviewer_snapshot_store(),
            &request,
        )
        .await
        .map_err(|error| {
            CodingWorkspaceEngineError::Store(crate::product::json_store::ProductStoreError::Io(
                format!("reviewer_window_begin_failed: {error}"),
            ))
        })
    }

    /// 材料采集前同源复验:窗口必须 Running 且材料与 baseline manifest
    /// 同指纹(T17 assert;gateway/spawn/材料共用)。
    pub(crate) fn assert_reviewer_window_running(
        &self,
        window: &crate::product::reviewer_window::ReviewerWindowHandle,
        attempt: &CodingExecutionAttempt,
        run_id: &str,
        provider: &ProviderName,
        worktree: &Path,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let expectation = WindowExpectation {
            window_id: &window.window_id,
            attempt_id: &attempt.id,
            run_id,
            provider: &reviewer_provider_wire_name(provider),
            worktree,
        };
        let proof = assert_review_window_current(
            &self.reviewer_window_store(),
            &self.reviewer_snapshot_store(),
            &expectation,
            &[ReviewerWindowState::Running],
        )
        .map_err(|error| {
            CodingWorkspaceEngineError::Store(crate::product::json_store::ProductStoreError::Io(
                format!("reviewer_window_not_current: {error}"),
            ))
        })?;
        if proof.manifest_sha256 != window.manifest_sha256 {
            return Err(CodingWorkspaceEngineError::Store(
                crate::product::json_store::ProductStoreError::Io(
                    "reviewer_window_material_fingerprint_drift".to_string(),
                ),
            ));
        }
        Ok(())
    }

    /// 干净回收:Running→Quiescing→恢复→(无违规)弃置释放。
    /// 前窗有违规(journal/ManualGate)→ Err(不重试、不释放,人工门)。
    /// 恢复停等(RecoveryRequired)→ Err(锁保留,T21 恢复驱动接管)。
    pub(crate) async fn recycle_reviewer_window(
        &self,
        window: &crate::product::reviewer_window::ReviewerWindowHandle,
        reason: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let window_store = self.reviewer_window_store();
        let snapshot_store = self.reviewer_snapshot_store();
        let record = begin_quiesce(&window_store, &window.window_id, window.generation, reason)
            .map_err(Self::window_lifecycle_error)?;
        let summary = restore_window(
            &window_store,
            &snapshot_store,
            &window.window_id,
            record.generation,
        )
        .await
        .map_err(Self::window_lifecycle_error)?;
        if summary.manual_gate {
            return Err(CodingWorkspaceEngineError::Store(
                crate::product::json_store::ProductStoreError::Io(format!(
                    "reviewer_window_violation_manual_gate: {}",
                    window.window_id
                )),
            ));
        }
        // 恢复完整且无违规:技术回收释放锁(旧报告无 / 已 Invalidated,
        // 弃置即 ClosedInvalidated,新窗重加锁/新 baseline)。
        abandon_window(
            &window_store,
            &window.window_id,
            summary.record.generation,
            reason,
        )
        .map_err(Self::window_lifecycle_error)?;
        Ok(())
    }

    /// 失败/取消路径收口:尽力恢复到 baseline 并保持锁到恢复完成——
    /// 干净恢复即弃置释放(取消本身是人工决策);违规进人工门、停等
    /// 保持 RecoveryRequired,均交 T21 恢复驱动/人工处置。错误只记录,
    /// 不掩盖调用方原始错误。
    pub(crate) async fn close_reviewer_window_after_failure(
        &self,
        window: &crate::product::reviewer_window::ReviewerWindowHandle,
        reason: &str,
    ) {
        if let Err(error) = self.recycle_reviewer_window(window, reason).await {
            tracing::warn!(?error, window = %window.window_id, "reviewer_window_failure_close_kept_lock");
        }
    }

    /// 报告收口(权威判定面 = `reconcile_from_stores`,不复制采信规则):
    /// Quiescing→落违规(恢复链观察)→恢复→三域对账→采信记录→
    /// (Accepted 才 close_accepted 释放锁)。必须发生在
    /// `save_code_review_report` 之前——保存的是可消费完成事实。
    pub(crate) async fn finalize_reviewer_window_with_report(
        &self,
        window: &crate::product::reviewer_window::ReviewerWindowHandle,
    ) -> Result<ReviewerWindowOutcome, CodingWorkspaceEngineError> {
        let window_store = self.reviewer_window_store();
        let snapshot_store = self.reviewer_snapshot_store();
        let record = begin_quiesce(
            &window_store,
            &window.window_id,
            window.generation,
            "review 完成:收口窗口(落违规/恢复/采信)",
        )
        .map_err(Self::window_lifecycle_error)?;
        let summary = restore_window(
            &window_store,
            &snapshot_store,
            &window.window_id,
            record.generation,
        )
        .await
        .map_err(Self::window_lifecycle_error)?;
        let mut record = summary.record;
        let outcome = reconcile_from_stores(&window_store, &snapshot_store, &window.window_id)
            .map_err(|error| {
                CodingWorkspaceEngineError::Store(
                    crate::product::json_store::ProductStoreError::Io(format!(
                        "reviewer_reconciliation_failed: {error}"
                    )),
                )
            })?;
        let acceptance = match outcome.recommendation {
            crate::product::reviewer_reconciliation::AcceptanceRecommendation::Accepted {
                ..
            } => {
                record = record_report_acceptance(
                    &window_store,
                    &snapshot_store,
                    &window.window_id,
                    record.generation,
                    ReportAcceptance::Accepted,
                )
                .map_err(Self::window_lifecycle_error)?;
                record = close_accepted(&window_store, &window.window_id, record.generation)
                    .map_err(Self::window_lifecycle_error)?;
                ReportAcceptance::Accepted
            }
            crate::product::reviewer_reconciliation::AcceptanceRecommendation::Invalidated {
                ..
            } => {
                // 整轮作废:采信投影落 Invalidated;窗口留人工门待裁决
                // (人工仅作废关闭/作废后新轮,不能洗白)。
                record = record_report_acceptance(
                    &window_store,
                    &snapshot_store,
                    &window.window_id,
                    record.generation,
                    ReportAcceptance::Invalidated,
                )
                .map_err(Self::window_lifecycle_error)?;
                ReportAcceptance::Invalidated
            }
            // 停等:保持 PendingAcceptance 与锁,恢复驱动重入后续裁。
            crate::product::reviewer_reconciliation::AcceptanceRecommendation::NotReady {
                ..
            } => ReportAcceptance::PendingAcceptance,
        };
        // ⚠5(修复轮 1):终态观察——人工门/停等窗留 durable 记录供
        // T21 恢复驱动与人工面分诊(记录状态由 store 持久,此处仅告警)。
        if summary.manual_gate || record.state != ReviewerWindowState::ClosedAccepted {
            tracing::warn!(
                window = %window.window_id,
                state = record.state.as_str(),
                manual_gate = summary.manual_gate,
                acceptance = ?acceptance,
                "reviewer_window_finalized_not_accepted_lock_or_gate_held"
            );
        }
        Ok(ReviewerWindowOutcome {
            window_id: window.window_id.clone(),
            acceptance,
        })
    }

    /// T20b（硬约束5/RevB2T19 ⚠1）：交付门回查——报告携带窗口 id 时以
    /// `reconcile_from_stores` 为权威复检（不信任 DTO 采信投影）；无窗口
    /// id/空引用 = 历史未核验报告，返回 `None`（调用方按历史放行语义）；
    /// 窗口 id 在场而记录缺失/对账失败 = 悬挂引用，fail-closed
    /// `Invalidated`（不默认 Accepted）。
    pub(crate) fn recheck_review_acceptance_for_delivery(
        &self,
        review_window_id: Option<&str>,
    ) -> Option<ReportAcceptance> {
        let window_id = review_window_id
            .map(str::trim)
            .filter(|id| !id.is_empty())?;
        let outcome = reconcile_from_stores(
            &self.reviewer_window_store(),
            &self.reviewer_snapshot_store(),
            window_id,
        );
        Some(match outcome {
            Ok(outcome) => match outcome.recommendation {
                crate::product::reviewer_reconciliation::AcceptanceRecommendation::Accepted {
                    ..
                } => ReportAcceptance::Accepted,
                crate::product::reviewer_reconciliation::AcceptanceRecommendation::Invalidated {
                    ..
                } => ReportAcceptance::Invalidated,
                // 停等（恢复未完成/瞬时错误）：交付面不得放行——按
                // Pending 处理（与 finalize 采信投影同口径）。
                crate::product::reviewer_reconciliation::AcceptanceRecommendation::NotReady {
                    ..
                } => ReportAcceptance::PendingAcceptance,
            },
            // 窗口记录缺失/读取失败：窗口 id 在场的悬挂引用 fail-closed。
            Err(error) => {
                tracing::warn!(
                    window = %window_id,
                    %error,
                    "review_delivery_recheck_failed_fail_closed_invalidated"
                );
                ReportAcceptance::Invalidated
            }
        })
    }

    /// 树上存在未终态 reviewer 窗口(租约被持有)→ fail-closed。
    /// Coder 交付/人工重试/plan repair 再入的窗重叠守卫(T14 合同预留:
    /// 不依赖其实现,由租约面直接判定)。
    pub(crate) fn ensure_no_open_reviewer_window(
        &self,
        worktree: &Path,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let Ok(canonical) = worktree.canonicalize() else {
            return Ok(());
        };
        // 无 Git 身份的 worktree 不可能存在 reviewer 窗口(T17 开窗硬
        // 前提是 git 身份解析)——按无窗放行,不把非 git 目录当阻塞。
        let Ok((_, _, common_dir)) = resolve_worktree_git_identity(&canonical) else {
            return Ok(());
        };
        let tree_key = crate::product::reviewer_window_store::tree_key_of(&canonical);
        let leases = self
            .reviewer_window_store()
            .list_leases()
            .map_err(CodingWorkspaceEngineError::Store)?;
        for lease in leases {
            if lease.tree_key == tree_key || Path::new(&lease.common_dir) == common_dir {
                return Err(CodingWorkspaceEngineError::Store(
                    crate::product::json_store::ProductStoreError::Conflict {
                        kind: "reviewer_window_lease",
                        id: lease.window_id.clone(),
                    },
                ));
            }
        }
        Ok(())
    }

    fn window_lifecycle_error(error: ReviewerWindowError) -> CodingWorkspaceEngineError {
        CodingWorkspaceEngineError::Store(crate::product::json_store::ProductStoreError::Io(
            format!("reviewer_window_lifecycle_failed: {error}"),
        ))
    }
}
