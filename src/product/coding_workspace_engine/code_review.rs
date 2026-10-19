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
    ManualGateDecision, ReviewerWindowError, WindowExpectation, abandon_window,
    assert_review_window_current, begin_quiesce, begin_review_window, close_accepted,
    enter_platform_write, exit_platform_write, park_window_recovery, record_report_acceptance,
    resolve_manual_gate, resolve_worktree_git_identity, restore_window,
};
use crate::product::reviewer_window_store::{
    ChannelWriter, LeaseOwnerLiveness, REVIEWER_WINDOW_STORE_DIR, ReportAcceptance,
    ReviewerWindowRecord, ReviewerWindowState, ReviewerWindowStore, WriteChannelKind,
    lease_owner_liveness, tree_key_of, wire_acceptance,
};

/// F13(波3):窗口 store 根目录名已上收为
/// `reviewer_window_store::REVIEWER_WINDOW_STORE_DIR` 单一常量(本模块
/// 经 use 引入;`review/drive.rs`/`coding_attempt_store/git_operation.rs`
/// 同源引用,不再各持字面量副本)。
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
    /// 集合恒为空,不误杀无关 scope)。B1-1:主 scope 同样挂 worktree
    /// 标签——窗口主 scope 与兄弟 scope 的两类恢复路径都能枚举 reviewer
    /// 进程登记(下一窗/恢复驱动的 quiesce_targets 均可收割)。
    pub(crate) fn reviewer_quiesce_targets(
        &self,
        window_id: &str,
        attempt: &CodingExecutionAttempt,
        run_id: &str,
        worktree: &Path,
    ) -> Result<
        Vec<crate::cross_cutting::provider_resources::ProviderRunResourceScope>,
        CodingWorkspaceEngineError,
    > {
        use crate::cross_cutting::provider_resources::ProviderRunResourceScope;
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
        // 协调归属 worktree 标签(主 scope 与兄弟枚举共用同一 tree_key)。
        let tree_key = crate::product::reviewer_window_store::tree_key_of(
            &worktree.canonicalize().map_err(|error| {
                CodingWorkspaceEngineError::Store(
                    crate::product::json_store::ProductStoreError::Io(format!(
                        "worktree canonicalize 失败: {error}"
                    )),
                )
            })?,
        );
        let key = Self::reviewer_main_scope_key(attempt, run_id);
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
        })?
        // B1-1:主 scope 挂 worktree 标签(reserve 前挂,aria.worktree 由
        // scope 注入)——本窗与后续窗口的兄弟枚举路径均可发现本窗进程。
        .with_worktree_key(&tree_key);
        let mut targets = vec![main_scope];

        // 协调 scope:按 aria.worktree 归属标签过滤(未标注归属的不静止——
        // 归属注入是 T14/T20 的登记合同,本 Task 只消费)。
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

    /// B1-1:编码 reviewer launch 的受信归属主键(与开窗 quiesce 主 scope
    /// 同源——`reviewer-window-{attempt}` × 开窗 run_id × 本平台 boot)。
    fn reviewer_main_scope_key(
        attempt: &CodingExecutionAttempt,
        run_id: &str,
    ) -> crate::cross_cutting::provider_resources::ResourceScopeKey {
        crate::cross_cutting::provider_resources::ResourceScopeKey {
            scope_id: format!("reviewer-window-{}", attempt.id),
            run_id: run_id.to_string(),
            boot_identity: format!("aria-engine-{}", std::process::id()),
        }
    }

    /// B1-1(波 2):编码 reviewer 本次 launch 的受信资源归属铸造——按
    /// 开窗实际归属(同 key/run/boot,挂 worktree 标签)打开主 scope 并
    /// 生成唯一资源 ID 的载体;调用点在窗口 assert 通过后(reviewer
    /// retry cycle),与 `reviewer_quiesce_targets` 主 scope 完全同源。
    pub(crate) fn reviewer_launch_ownership(
        &self,
        window_id: &str,
        attempt: &CodingExecutionAttempt,
        run_id: &str,
        worktree: &Path,
    ) -> Result<
        crate::cross_cutting::provider_resources::TrustedLaunchOwnership,
        CodingWorkspaceEngineError,
    > {
        use crate::cross_cutting::provider_resources::{
            ProviderRunResourceScope, TrustedLaunchOwnership,
        };
        let store_root = self.store.paths().root().join(PROVIDER_RESOURCE_STORE_DIR);
        let private_root = self.store.provider_stream_log_root(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        );
        std::fs::create_dir_all(&private_root).map_err(|error| {
            CodingWorkspaceEngineError::Store(crate::product::json_store::ProductStoreError::Io(
                format!(
                    "reviewer_launch_scope_private_root_create_failed: {}: {error}",
                    private_root.display()
                ),
            ))
        })?;
        let tree_key = crate::product::reviewer_window_store::tree_key_of(
            &worktree.canonicalize().map_err(|error| {
                CodingWorkspaceEngineError::Store(
                    crate::product::json_store::ProductStoreError::Io(format!(
                        "reviewer_launch_worktree_canonicalize 失败: {error}"
                    )),
                )
            })?,
        );
        let key = Self::reviewer_main_scope_key(attempt, run_id);
        let scope = ProviderRunResourceScope::open_for_recovery(
            &key,
            Some(window_id),
            &private_root,
            &store_root,
            None,
        )
        .map_err(|error| {
            CodingWorkspaceEngineError::Store(crate::product::json_store::ProductStoreError::Io(
                format!("reviewer_launch_scope_open_failed: {error}"),
            ))
        })?
        .with_worktree_key(&tree_key);
        Ok(TrustedLaunchOwnership::mint(
            std::sync::Arc::new(scope),
            &format!("code_review provider process: window={window_id}"),
        ))
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

// ---------------------------------------------------------------------------
// T21:取消/崩溃启动补偿与人工恢复续接
// ---------------------------------------------------------------------------

/// 单窗启动补偿结局(消费面:启动扫描前置、人工恢复续接)。
#[derive(Debug)]
pub(crate) struct StartupWindowCompensation {
    pub(crate) window_id: String,
    pub(crate) attempt_id: String,
    /// 补偿后的窗口状态(终态=已释放;ManualGate/RecoveryRequired=保持锁)。
    pub(crate) state: ReviewerWindowState,
    /// 树已释放(终态关闭;租约释放、四通道解冻)。
    pub(crate) released: bool,
    /// B2/F3:租约闸门跳过(活/未知/异常 owner)——零副作用,未动树,
    /// 不动 generation/租约/attempt 投影;与「补偿后保持锁」语义分离
    /// (调用方据此区分跳过日志与停等告警)。
    pub(crate) skipped_live_owner: bool,
    /// 保持锁时的人工处置原因(人工门/停等投影)。
    pub(crate) manual_attention: Option<String>,
}

/// attempt 删除/终止时的窗口收口模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttemptWindowCloseMode {
    /// 终止:恢复失败 → 保持锁 fail-closed(终止结果可见,人工/重试续接)。
    Terminate,
    /// 删除:工作树随之删除,恢复 best-effort 后无条件弃置关闭(通道/
    /// 租约必须收口,不留冻结残锁卡共享 common-dir 的主仓 git mutation)。
    Delete,
}

/// B2/F3:崩溃补偿的驱动来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrashCompensationDrive {
    /// 启动扫描(serve_web 启动序列):owner 存活即零副作用跳过——
    /// owner==self 也不是启动扫描动树的通行证(窗口可能仍在正常推进)。
    StartupScan,
    /// 人工显式重驱(socket RecoverCoding 前置):owner=本进程(存活)
    /// 的窗口放行续接(同进程自持窗口的恢复不被永久挡)。
    ExplicitRedrive,
}

/// B2/F3:补偿破坏单元(停资源/恢复/弃置)前的租约闸门——启动扫描
/// 与显式重驱共用。分类不洗白:
/// - owner 存活 → 跳过(不同端口并发实例持窗,零副作用);
/// - owner 未知(owner_pid=0//proc 取证失败)→ 停等跳过;
/// - 租约不可读(缺失损坏)/租约属他窗 → 显式分类跳过,不当死 owner;
/// - 租约缺失:Preparing/无 baseline(开窗在租约持久化前崩溃,本就
///   无主可查)与非 Running 停等/恢复中态(finalize_terminal「先释放
///   租约后落终态」的重入前缀,幂等收口是既有语义)均放行;Running
///   却无租约属异常,停等;
/// - 死 owner → 借现有租约锁域原子认领(acquire_lease 域锁内读-判-写
///   +身份比对;两恢复者竞争=一赢一停等),认领成功才放行。
///
/// `Ok(())` = 可进入破坏单元;`Err(reason)` = 零副作用跳过(原因可见)。
fn crash_compensation_lease_gate(
    window_store: &ReviewerWindowStore,
    record: &ReviewerWindowRecord,
    drive: CrashCompensationDrive,
) -> Result<(), String> {
    let tree_key = tree_key_of(&record.identity.canonical_worktree);
    let lease = match window_store.load_lease(&tree_key) {
        Ok(Some(lease)) => lease,
        Ok(None) => {
            if matches!(record.state, ReviewerWindowState::Preparing)
                || record.baseline.is_none()
                || record.state != ReviewerWindowState::Running
            {
                return Ok(());
            }
            return Err(
                "lease_missing_for_running_window(异常:Running 窗无租约,显式停等不动树)"
                    .to_string(),
            );
        }
        Err(error) => {
            return Err(format!(
                "lease_unreadable_fail_closed(租约缺失损坏,不当死 owner): {error}"
            ));
        }
    };
    if lease.window_id != record.window_id || lease.attempt_id != record.attempt_id {
        return Err(format!(
            "lease_owner_mismatch(租约属窗口 {}/{},本窗 {}/{}):不改写他人租约",
            lease.window_id, lease.attempt_id, record.window_id, record.attempt_id
        ));
    }
    let self_pid = std::process::id();
    match lease_owner_liveness(&lease) {
        LeaseOwnerLiveness::Alive => {
            let self_owned = lease.owner_pid == self_pid
                && lease.owner_start_ticks
                    == crate::cross_cutting::process_manager::process_start_time_ticks(self_pid)
                        .unwrap_or(0);
            if drive == CrashCompensationDrive::ExplicitRedrive && self_owned {
                return Ok(());
            }
            Err(format!(
                "lease_owner_alive(owner pid {}):并发实例持窗,跳过(零副作用)",
                lease.owner_pid
            ))
        }
        LeaseOwnerLiveness::Unknown => Err(format!(
            "lease_owner_unknown(owner_pid={} 或取证失败):fail-closed 停等,不当死 owner 处理",
            lease.owner_pid
        )),
        LeaseOwnerLiveness::Dead => {
            // 死 owner:现有租约锁域内原子认领——身份字段不变,owner 换成
            // 本恢复者;并发恢复者一赢(Ok)一停等(Conflict:他者已认领
            // 且存活)。
            let mut takeover = lease.clone();
            takeover.owner_pid = self_pid;
            takeover.owner_start_ticks =
                crate::cross_cutting::process_manager::process_start_time_ticks(self_pid)
                    .unwrap_or(0);
            window_store.acquire_lease(&takeover).map_err(|error| {
                format!("lease_takeover_lost(他恢复者已认领或租约冲突,停等): {error}")
            })
        }
    }
}

/// F12(波3):半成品快照清理——仅清「manifest 未发布」的**本窗** snapshot
/// 目录(`<snapshot-root>/windows/<window_id>`)。
///
/// 语义边界(design-review-fixes D-F12):
/// - `record.baseline == None` **不等于** manifest 未发布:`begin_review_window`
///   的 F4 采用路径特意支持「manifest 已发布、Running CAS 未落」的同窗采用,
///   已发布 manifest 是可恢复证据——manifest.json 存在即一律不动;
/// - 归属校验:目录按本窗 window_id 键控(单一相对段,`validate_relative_id`
///   拒绝 `..`/路径分隔符),邻窗目录零接触;符号链接目录拒绝穿透删除;
/// - 幂等:目录不存在=无事可做;同窗重复清理安全;
/// - 返回 Ok(true)=已清;Ok(false)=无需清理(未残留/已发布/幂等重入);
///   Err(reason)=拒绝或失败原因(调用方仅告警,不阻塞弃置收口——残留
///   目录由同窗重采的 `capture_baseline` 自清)。
fn cleanup_unpublished_snapshot_window_dir(
    snapshot_root: &std::path::Path,
    window_id: &str,
) -> Result<bool, String> {
    if let Err(error) = crate::product::json_store::validate_relative_id(window_id) {
        return Err(format!("window_id 非法,拒绝清理(fail-closed): {error}"));
    }
    let window_dir = snapshot_root.join("windows").join(window_id);
    let metadata = match std::fs::symlink_metadata(&window_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "读取窗口目录 {} 失败: {error}",
                window_dir.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "窗口目录 {} 是符号链接,拒绝穿透删除",
            window_dir.display()
        ));
    }
    if window_dir.join("manifest.json").is_file() {
        // 已发布 = F4 同窗采用路径的可恢复证据,不删。
        return Ok(false);
    }
    match std::fs::remove_dir_all(&window_dir) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "清理未发布窗口目录 {} 失败: {error}",
            window_dir.display()
        )),
    }
}

impl CodingWorkspaceEngine {
    /// T21 启动补偿扫描:枚举全部未终态 reviewer 窗口逐一补偿——
    /// 停旧登记资源且确认静止 → 按原 manifest 续恢复(不重冻/不重采
    /// 污染基线)→ 无违规则弃置释放,有违规/停等保持锁并投影人工门。
    /// 局部坏窗不停扫:单窗失败记入结局,其余无冲突树继续(fail-closed
    /// 仅作用于该树)。
    pub(crate) async fn compensate_open_reviewer_windows_at_startup(
        &self,
    ) -> Vec<StartupWindowCompensation> {
        let window_store = self.reviewer_window_store();
        let snapshot_store = self.reviewer_snapshot_store();
        let records = match window_store.list_windows() {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(%error, "startup window scan: window records unreadable");
                return Vec::new();
            }
        };
        let mut outcomes = Vec::new();
        for record in records {
            if record.state.is_terminal() {
                continue;
            }
            let outcome = self
                .compensate_window_after_crash(
                    &window_store,
                    &snapshot_store,
                    record,
                    CrashCompensationDrive::StartupScan,
                )
                .await;
            if outcome.skipped_live_owner {
                // B2/F3:活/未知/异常 owner 跳过——零副作用,非失败,
                // 与停等告警分流。
                tracing::info!(
                    window = %outcome.window_id,
                    attempt = %outcome.attempt_id,
                    state = outcome.state.as_str(),
                    reason = outcome.manual_attention.as_deref().unwrap_or("unresolved"),
                    "startup reviewer window compensation skipped (lease owner gate)"
                );
            } else if !outcome.released {
                tracing::warn!(
                    window = %outcome.window_id,
                    attempt = %outcome.attempt_id,
                    state = outcome.state.as_str(),
                    reason = outcome.manual_attention.as_deref().unwrap_or("unresolved"),
                    "startup reviewer window compensation kept lock (fail-closed)"
                );
            }
            outcomes.push(outcome);
        }
        outcomes
    }

    /// 单窗补偿单元(启动驱动与人工恢复续接共用)。锁序沿 T19:窗口
    /// 租约/冻结在前,attempt git 补偿仅在窗口终态释放后续接。
    /// B2/F3:破坏单元(停资源/恢复/弃置)之前先过 `crash_compensation_
    /// lease_gate`——owner 存活/未知/租约异常零副作用跳过;死 owner
    /// 经租约锁域原子认领后才放行。
    async fn compensate_window_after_crash(
        &self,
        window_store: &ReviewerWindowStore,
        snapshot_store: &ReviewerSnapshotStore,
        record: ReviewerWindowRecord,
        drive: CrashCompensationDrive,
    ) -> StartupWindowCompensation {
        let window_id = record.window_id.clone();
        let attempt_id = record.attempt_id.clone();
        if let Err(reason) = crash_compensation_lease_gate(window_store, &record, drive) {
            tracing::warn!(
                window = %window_id,
                attempt = %attempt_id,
                state = record.state.as_str(),
                %reason,
                "crash compensation skipped: lease owner gate (zero side effects)"
            );
            return StartupWindowCompensation {
                state: record.state,
                released: false,
                skipped_live_owner: true,
                manual_attention: Some(reason),
                window_id,
                attempt_id,
            };
        }
        // ManualGate:人工裁决待定——不动状态、保持锁,投影人工面。
        if record.state == ReviewerWindowState::ManualGate {
            self.project_window_manual_attention(&record, "reviewer_window_manual_gate");
            return StartupWindowCompensation {
                state: record.state,
                released: false,
                skipped_live_owner: false,
                manual_attention: Some(
                    "违规人工门:仅人工裁决(Invalidate/RequireNewReview)可收口".to_string(),
                ),
                window_id,
                attempt_id,
            };
        }
        // 1) 停旧登记资源且确认静止(killpg/Drop 只是步骤,静止以真实
        //    身份观察为准;未知 writer/容器不可证静止 → 停等保持锁)。
        if let Err(reason) = self.quiesce_window_scopes_for_recovery(&record).await {
            let state = park_window_recovery(
                window_store,
                &record,
                "startup_compensation_quiesce_not_proven",
                &reason,
            );
            self.project_window_manual_attention(&record, "reviewer_window_recovery_required");
            return StartupWindowCompensation {
                state,
                released: false,
                skipped_live_owner: false,
                manual_attention: Some(reason),
                window_id,
                attempt_id,
            };
        }
        // 2) 重读记录(防御并发推进)后按崩溃前缀分派。
        let mut current = match window_store.load_window(&window_id) {
            Ok(record) => record,
            Err(error) => {
                let reason = format!("window record unreadable: {error}");
                self.project_window_manual_attention(&record, "reviewer_window_recovery_required");
                return StartupWindowCompensation {
                    state: record.state,
                    released: false,
                    skipped_live_owner: false,
                    manual_attention: Some(reason),
                    window_id,
                    attempt_id,
                };
            }
        };
        if current.state == ReviewerWindowState::Running {
            // run 已死亡(进程崩溃),材料面停:Running → Quiescing 后走恢复。
            match begin_quiesce(
                window_store,
                &window_id,
                current.generation,
                "启动补偿:run 已死亡,收口窗口(落违规→恢复)",
            ) {
                Ok(quiescing) => current = quiescing,
                Err(error) => {
                    let reason = format!("begin_quiesce failed: {error}");
                    self.project_window_manual_attention(
                        &current,
                        "reviewer_window_recovery_required",
                    );
                    return StartupWindowCompensation {
                        state: current.state,
                        released: false,
                        skipped_live_owner: false,
                        manual_attention: Some(reason),
                        window_id,
                        attempt_id,
                    };
                }
            }
        }
        if matches!(current.state, ReviewerWindowState::Preparing) || current.baseline.is_none() {
            // 崩溃于开窗中途(租约已持久/manifest 未全):无 baseline ⇒
            // reviewer 无材料无写入,树无污染可恢复——弃置释放(通道解冻
            // +租约释放;manifest 采用点属同窗重入路径,启动面无重入者)。
            // F12(波3):弃置前清理未发布的本窗 snapshot 残留目录
            // (baseline=None ≠ manifest 未发布:已发布 manifest 保留);
            // 仅磁盘卫生,best-effort 不阻塞弃置收口(同窗重采自清)。
            let snapshot_root = self.store.paths().root().join(REVIEWER_SNAPSHOT_STORE_DIR);
            match cleanup_unpublished_snapshot_window_dir(&snapshot_root, &window_id) {
                Ok(true) => tracing::info!(
                    window = %window_id,
                    "startup compensation: unpublished snapshot residue cleaned"
                ),
                Ok(false) => {}
                Err(reason) => tracing::warn!(
                    window = %window_id,
                    %reason,
                    "startup compensation: unpublished snapshot residue cleanup skipped/failed (abandon continues)"
                ),
            }
            return match abandon_window(
                window_store,
                &window_id,
                current.generation,
                "启动补偿:开窗未完成(无 baseline),弃置释放",
            ) {
                Ok(closed) => {
                    self.after_window_released(window_id, attempt_id, closed.state)
                        .await
                }
                Err(error) => StartupWindowCompensation {
                    state: current.state,
                    released: false,
                    skipped_live_owner: false,
                    manual_attention: Some(format!("abandon failed: {error}")),
                    window_id,
                    attempt_id,
                },
            };
        }
        // 3) 按原 manifest 续恢复(Quiescing/ViolationRecorded/Restoring/
        //    RecoveryRequired 停等重入;T16 游标幂等续接,不重冻基线)。
        let generation = current.generation;
        match restore_window(window_store, snapshot_store, &window_id, generation).await {
            Ok(summary) if summary.manual_gate => {
                // 违规事实已 journal-first 落盘:人工门保持锁(仅人工裁决)。
                self.project_window_manual_attention(
                    &summary.record,
                    "reviewer_window_manual_gate",
                );
                StartupWindowCompensation {
                    state: summary.record.state,
                    released: false,
                    skipped_live_owner: false,
                    manual_attention: Some("恢复完整但违规事实落 journal:人工门待裁决".to_string()),
                    window_id,
                    attempt_id,
                }
            }
            Ok(summary) => {
                // 恢复完整且无违规:run 已死亡、无报告可采——按失败收口
                // 语义弃置释放(旧报告无/Invalidated;新轮重开新窗/新基线)。
                match abandon_window(
                    window_store,
                    &window_id,
                    summary.record.generation,
                    "启动补偿:恢复完整且无违规,run 已死亡,弃置释放",
                ) {
                    Ok(closed) => {
                        self.after_window_released(window_id, attempt_id, closed.state)
                            .await
                    }
                    Err(error) => StartupWindowCompensation {
                        state: summary.record.state,
                        released: false,
                        skipped_live_owner: false,
                        manual_attention: Some(format!("abandon failed: {error}")),
                        window_id,
                        attempt_id,
                    },
                }
            }
            Err(ReviewerWindowError::RecoveryStopped { reason, .. }) => {
                // 恢复失败:restore_window 已持久 RecoveryRequired(游标入
                // 记录,锁保留)——投影人工门面,重入续接。
                let state = window_store
                    .load_window(&window_id)
                    .map(|reloaded| reloaded.state)
                    .unwrap_or(ReviewerWindowState::RecoveryRequired);
                self.project_window_manual_attention(&current, "reviewer_window_recovery_required");
                StartupWindowCompensation {
                    state,
                    released: false,
                    skipped_live_owner: false,
                    manual_attention: Some(format!("恢复停等: {reason}")),
                    window_id,
                    attempt_id,
                }
            }
            Err(error) => {
                // 瞬时 Err(状态不动)或违规 journal pending 停等:保持锁,
                // 可见原因,重试/人工续接。
                let state = window_store
                    .load_window(&window_id)
                    .map(|reloaded| reloaded.state)
                    .unwrap_or(current.state);
                self.project_window_manual_attention(&current, "reviewer_window_recovery_required");
                StartupWindowCompensation {
                    state,
                    released: false,
                    skipped_live_owner: false,
                    manual_attention: Some(error.to_string()),
                    window_id,
                    attempt_id,
                }
            }
        }
    }

    /// 窗口释放后动作:续接被冻结拒绝的 Git 补偿(取消语义;仅非在途
    /// attempt——Running/Created 的 journal 归 runner 恢复/终止收敛面)。
    async fn after_window_released(
        &self,
        window_id: String,
        attempt_id: String,
        state: ReviewerWindowState,
    ) -> StartupWindowCompensation {
        self.resume_pending_review_compensation(&attempt_id).await;
        StartupWindowCompensation {
            window_id,
            attempt_id,
            state,
            released: true,
            skipped_live_owner: false,
            manual_attention: None,
        }
    }

    /// 停止并清理窗口锁集内的全部登记资源(先静止证明再清理;killpg/
    /// Drop 只是步骤,静止以真实身份观察为准):
    /// - 窗口自身的 quiesce scope(记录在窗口记录 `quiesced_scopes`);
    /// - 资源 store 中登记了 `aria.worktree == 本树 tree_key` 的兄弟
    ///   scope(T14/T20 归属标签合同;未标注归属的不静止,不误杀无关
    ///   scope)。
    ///
    /// 任何 scope 静止未证明/清理失败 → Err(停等,不伪称静止)。
    async fn quiesce_window_scopes_for_recovery(
        &self,
        record: &ReviewerWindowRecord,
    ) -> Result<(), String> {
        use crate::cross_cutting::provider_resources::QuiesceOutcome;
        let store_root = self.store.paths().root().join(PROVIDER_RESOURCE_STORE_DIR);
        let resource_store =
            crate::product::provider_resource_store::ProviderResourceStore::new(&store_root);
        let tree_key = tree_key_of(&record.identity.canonical_worktree);
        let all_keys = resource_store
            .scope_keys()
            .map_err(|error| format!("resource store unreadable: {error}"))?;
        let mut targets: Vec<(
            crate::cross_cutting::provider_resources::ResourceScopeKey,
            Vec<crate::cross_cutting::provider_resources::ProviderResourceRecord>,
        )> = Vec::new();
        for key in all_keys {
            if record.quiesced_scopes.contains(&key) || targets.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let records = resource_store
                .list_scope(&key)
                .map_err(|error| format!("scope {} unreadable: {error}", key.scope_id))?;
            let owned = records.iter().any(|resource| {
                resource
                    .reserved_labels
                    .get("aria.worktree")
                    .is_some_and(|labelled| labelled == &tree_key)
            });
            if owned {
                targets.push((key, records));
            }
        }
        for key in &record.quiesced_scopes {
            if targets.iter().any(|(known, _)| known == key) {
                continue;
            }
            let records = resource_store
                .list_scope(key)
                .map_err(|error| format!("scope {} unreadable: {error}", key.scope_id))?;
            targets.push((key.clone(), records));
        }
        if targets.is_empty() {
            return Ok(());
        }
        let scratch = self
            .store
            .paths()
            .root()
            .join("startup-window-compensation");
        std::fs::create_dir_all(&scratch)
            .map_err(|error| format!("scratch private root create failed: {error}"))?;
        let cleanup_scratch = scratch.clone();
        for (key, records) in targets {
            let private = self.restore_scope_private_root(&key, &records, &scratch);
            let scope =
                crate::cross_cutting::provider_resources::ProviderRunResourceScope::open_for_recovery(
                    &key,
                    Some(&record.window_id),
                    &private,
                    &store_root,
                    None,
                )
                .map_err(|error| format!("scope {} open failed: {error}", key.scope_id))?;
            match scope.quiesce_scope().await {
                Ok(QuiesceOutcome::Quiescent(proof)) => scope
                    .cleanup_owned_scope(&proof)
                    .map_err(|error| format!("scope {} cleanup failed: {error}", key.scope_id))?,
                Ok(QuiesceOutcome::RecoveryRequired(unknown)) => {
                    // ⚠9:scratch 兜底目录收口(best-effort,空目录才可删)。
                    let _ = std::fs::remove_dir(&cleanup_scratch);
                    return Err(format!("{}: {}", unknown.reason, unknown.detail));
                }
                Err(error) => {
                    let _ = std::fs::remove_dir(&cleanup_scratch);
                    return Err(format!("quiesce failed: {error}"));
                }
            }
        }
        let _ = std::fs::remove_dir(&cleanup_scratch);
        Ok(())
    }

    /// F1(审查修复轮 1):还原 scope **开窗时的私有根**——Directory 资源的
    /// `canonical_private_path` 是私有根下的资源目录本身,直接当私有根会让
    /// cleanup 的「严格子路径」守卫命中 outside_private_root → 清理必败 →
    /// 窗口永久停等。按 scope_id 前缀还原:
    /// - `code-delivery-{attempt}` → `coder_delivery_private_root(attempt)`;
    /// - `reviewer-window-{attempt}` → attempt 的 provider-stream 根;
    /// - 其余:资源私有路径的父目录(兜底 scratch)。
    fn restore_scope_private_root(
        &self,
        key: &crate::cross_cutting::provider_resources::ResourceScopeKey,
        records: &[crate::cross_cutting::provider_resources::ProviderResourceRecord],
        scratch: &std::path::Path,
    ) -> std::path::PathBuf {
        let attempt_scoped_root = |prefix: &str| -> Option<std::path::PathBuf> {
            let attempt_id = key.scope_id.strip_prefix(prefix)?;
            let attempt = self.store.get_attempt_by_id(attempt_id).ok()?;
            let stream_root = self.store.provider_stream_log_root(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            );
            let root = if prefix == "code-delivery-" {
                stream_root.join("code-delivery")
            } else {
                stream_root
            };
            std::fs::create_dir_all(&root).ok()?;
            Some(root)
        };
        if let Some(root) = attempt_scoped_root("code-delivery-") {
            return root;
        }
        if let Some(root) = attempt_scoped_root("reviewer-window-") {
            return root;
        }
        records
            .iter()
            .find_map(|resource| {
                resource.canonical_private_path.as_ref().and_then(|path| {
                    std::path::Path::new(path)
                        .parent()
                        .map(std::path::Path::to_path_buf)
                })
            })
            .unwrap_or_else(|| scratch.to_path_buf())
    }

    /// 窗口保持锁时的人工面投影:attempt 活跃则转 AwaitingManualRecovery
    /// (幂等;启动分诊/attach 恢复据此 fail-closed,人工恢复续接为唯一出口)。
    fn project_window_manual_attention(&self, record: &ReviewerWindowRecord, reason: &str) {
        match self.store.get_attempt_by_id(&record.attempt_id) {
            Ok(attempt) if attempt.status.is_active() => {
                if let Err(error) = self
                    .store
                    .transition_to_awaiting_manual_recovery(&record.attempt_id, reason)
                {
                    tracing::warn!(
                        attempt_id = %record.attempt_id,
                        ?error,
                        "startup window compensation: manual-recovery projection failed"
                    );
                }
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(
                attempt_id = %record.attempt_id,
                %error,
                "startup window compensation: attempt lookup failed"
            ),
        }
    }

    /// 释放后续接被窗口冻结拒绝的 ReviewRequest Git 补偿(取消已决定的
    /// 形态:终态/人工恢复/阻塞 attempt;在途 Running/Created 归 runner
    /// 恢复与终止收敛面,不在此抢跑)。失败仅告警(journal durable 保守)。
    async fn resume_pending_review_compensation(&self, attempt_id: &str) {
        use crate::product::coding_attempt_store::{
            CodingGitOperationKind, CodingGitOperationPhase,
        };
        let Ok(attempt) = self.store.get_attempt_by_id(attempt_id) else {
            return;
        };
        if matches!(
            attempt.status,
            crate::product::coding_models::CodingAttemptStatus::Running
                | crate::product::coding_models::CodingAttemptStatus::Created
        ) {
            return;
        }
        let Ok(Some(journal)) = self.store.get_coding_git_operation(&attempt) else {
            return;
        };
        if journal.kind != CodingGitOperationKind::ReviewRequest
            || !matches!(
                journal.phase,
                CodingGitOperationPhase::Before
                    | CodingGitOperationPhase::CommitStarted
                    | CodingGitOperationPhase::CommitCreated
            )
        {
            return;
        }
        if let Err(error) = self
            .compensate_cancelled_review_commit(&attempt, &journal)
            .await
        {
            tracing::warn!(
                attempt_id,
                %error,
                "window released: git compensation resume failed (durable journal kept)"
            );
        }
    }

    /// 人工恢复续接(socket RecoverCoding 前置):attempt 树上的未终态
    /// 窗口先驱动补偿(停资源→续恢复→释放);ManualGate 需显式裁决
    /// (REST reviewer-window:resolve),redrive 不代作废。全部收口
    /// (终态)才放行 runner 恢复——带窗重启 runner 恒拒。
    pub(crate) async fn redrive_reviewer_window_recovery_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let window_store = self.reviewer_window_store();
        let snapshot_store = self.reviewer_snapshot_store();
        let records = window_store
            .list_windows()
            .map_err(CodingWorkspaceEngineError::Store)?;
        let attempt_tree_key = attempt
            .worktree_path
            .as_deref()
            .and_then(|worktree| worktree.canonicalize().ok())
            .map(|canonical| tree_key_of(&canonical));
        for record in records {
            if record.state.is_terminal() {
                continue;
            }
            let on_tree = attempt_tree_key
                .as_ref()
                .is_some_and(|key| key == &tree_key_of(&record.identity.canonical_worktree));
            if record.attempt_id != attempt.id && !on_tree {
                continue;
            }
            if record.state == ReviewerWindowState::ManualGate {
                return Err(CodingWorkspaceEngineError::Store(
                    crate::product::json_store::ProductStoreError::Io(format!(
                        "reviewer_window_manual_gate_requires_decision: window={}",
                        record.window_id
                    )),
                ));
            }
            let outcome = self
                .compensate_window_after_crash(
                    &window_store,
                    &snapshot_store,
                    record,
                    CrashCompensationDrive::ExplicitRedrive,
                )
                .await;
            if !outcome.released {
                return Err(CodingWorkspaceEngineError::Store(
                    crate::product::json_store::ProductStoreError::Io(format!(
                        "reviewer_window_recovery_unresolved: window={} state={} reason={}",
                        outcome.window_id,
                        outcome.state.as_str(),
                        outcome.manual_attention.as_deref().unwrap_or("unresolved")
                    )),
                ));
            }
        }
        Ok(())
    }

    /// 人工门裁决生产入口(REST reviewer-window:resolve 消费):
    /// ManualGate → ClosedInvalidated|NewReviewRequired(锁释放,旧报告
    /// 永久 Invalidated)。人工许可新轮=另取租约/新基线(新 run 重开新
    /// 窗);人工拒绝补偿(Invalidate)不解锁 attempt——不自动复活
    /// runner,续接只经人工恢复面/新轮,旧违规报告不可 resume。
    ///
    /// ⚠5(审查顺带):本入口不抢跑 attempt 的取消 git 补偿——窗口已终态
    /// 释放,补偿续接统一归启动驱动/终止收敛面(`resume_pending_review_
    /// compensation` 的单一归属),人工裁决面保持零 git mutation。
    pub(crate) fn resolve_reviewer_manual_gate_for_attempt(
        &self,
        attempt_id: &str,
        decision: ManualGateDecision,
    ) -> Result<ReviewerWindowRecord, CodingWorkspaceEngineError> {
        let window_store = self.reviewer_window_store();
        let record = window_store
            .list_windows()
            .map_err(CodingWorkspaceEngineError::Store)?
            .into_iter()
            .find(|record| !record.state.is_terminal() && record.attempt_id == attempt_id)
            .ok_or_else(|| {
                CodingWorkspaceEngineError::Store(
                    crate::product::json_store::ProductStoreError::NotFound {
                        kind: "reviewer_window_open",
                        id: attempt_id.to_string(),
                    },
                )
            })?;
        if record.state != ReviewerWindowState::ManualGate {
            return Err(CodingWorkspaceEngineError::Store(
                crate::product::json_store::ProductStoreError::Io(format!(
                    "reviewer_window_not_manual_gate: window={} state={}(停等窗先经恢复续接)",
                    record.window_id,
                    record.state.as_str()
                )),
            ));
        }
        let generation = record.generation;
        let resolved = resolve_manual_gate(&window_store, &record.window_id, generation, decision)
            .map_err(Self::window_lifecycle_error)?;
        // 投影(幂等):attempt 保持人工恢复面,不自动 resume(人工拒绝
        // 补偿不解锁;新轮/人工恢复续接是唯一出口)。
        self.project_window_manual_attention(&resolved, "reviewer_window_manual_resolved");
        Ok(resolved)
    }

    /// attempt 删除/终止时的 reviewer 窗口与租约收口:逐窗先恢复
    /// baseline(保 Coder dirty)再弃置关闭;Terminate 恢复失败 → 保持
    /// 锁 fail-closed(Err 上抛,终止结果可见);Delete 无条件收口
    /// (工作树随之删除,不留冻结残锁)。违规 journal 随记录保留审计。
    pub(crate) async fn close_reviewer_windows_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        mode: AttemptWindowCloseMode,
    ) -> Result<usize, CodingWorkspaceEngineError> {
        let window_store = self.reviewer_window_store();
        let snapshot_store = self.reviewer_snapshot_store();
        let records: Vec<ReviewerWindowRecord> = window_store
            .list_windows()
            .map_err(CodingWorkspaceEngineError::Store)?
            .into_iter()
            .filter(|record| !record.state.is_terminal() && record.attempt_id == attempt.id)
            .collect();
        let mut closed = 0usize;
        for record in records {
            let window_id = record.window_id.clone();
            let mut current = record;
            if current.state == ReviewerWindowState::Running {
                current = begin_quiesce(
                    &window_store,
                    &window_id,
                    current.generation,
                    "attempt 终止/删除:收口窗口(落违规→恢复)",
                )
                .map_err(Self::window_lifecycle_error)?;
            }
            // 恢复前先停旧登记资源且确认静止(与启动补偿同款锁序;writer
            // 未静止时不得动树)。Terminate 停等 fail-closed;Delete 跳过
            // 恢复直接弃置(工作树随之删除)。
            let quiesced = self.quiesce_window_scopes_for_recovery(&current).await;
            if let Err(reason) = &quiesced
                && matches!(mode, AttemptWindowCloseMode::Terminate)
            {
                park_window_recovery(
                    &window_store,
                    &current,
                    "attempt_close_quiesce_not_proven",
                    reason,
                );
                return Err(CodingWorkspaceEngineError::Store(
                    crate::product::json_store::ProductStoreError::Io(format!(
                        "reviewer_window_close_quiesce_not_proven: window={} {reason}",
                        window_id
                    )),
                ));
            }
            if quiesced.is_ok()
                && current.baseline.is_some()
                && matches!(
                    current.state,
                    ReviewerWindowState::Quiescing
                        | ReviewerWindowState::ViolationRecorded
                        | ReviewerWindowState::Restoring
                        | ReviewerWindowState::RecoveryRequired
                )
            {
                match restore_window(
                    &window_store,
                    &snapshot_store,
                    &window_id,
                    current.generation,
                )
                .await
                {
                    Ok(summary) => current = summary.record,
                    Err(error) => {
                        if matches!(mode, AttemptWindowCloseMode::Terminate) {
                            // fail-closed:树保持锁,终止结果可见,
                            // 人工恢复续接/重试终止收敛。
                            return Err(Self::window_lifecycle_error(error));
                        }
                        tracing::warn!(
                            window = %window_id,
                            %error,
                            "attempt delete window close: restore best-effort failed, abandoning anyway (worktree being deleted)"
                        );
                    }
                }
            }
            let note = match mode {
                AttemptWindowCloseMode::Terminate => "attempt 终止:恢复后弃置关闭",
                AttemptWindowCloseMode::Delete => "attempt 删除:弃置关闭(工作树随之删除)",
            };
            abandon_window(&window_store, &window_id, current.generation, note)
                .map_err(Self::window_lifecycle_error)?;
            closed += 1;
            self.resume_pending_review_compensation(&attempt.id).await;
        }
        Ok(closed)
    }
}
#[cfg(test)]
mod hygiene_tests {
    /// F13(波3):`"reviewer-windows"` 目录名单一常量归 reviewer_window_store
    /// 持有,三处消费(coding 引擎/coding_attempt_store 通道守卫/plan 窗口)
    /// 同根同值;字面值保持不变,不留模块私有别名。
    #[test]
    fn f13_reviewer_windows_dir_is_single_shared_constant() {
        assert_eq!(
            crate::product::reviewer_window_store::REVIEWER_WINDOW_STORE_DIR,
            "reviewer-windows",
            "单一常量必须持原目录字面值"
        );
    }
    mod f12 {
        use super::super::*;
        use crate::product::app_paths::ProductAppPaths;
        use crate::product::reviewer_snapshot::{
            GitObservationBaseline, REVIEWER_BASELINE_SCHEMA, RestoreCursor,
            ReviewWorktreeLockIdentity, ReviewerBaselineManifest, ReviewerSnapshotStore,
        };
        use crate::product::reviewer_window_store::{
            REVIEWER_WINDOW_STORE_DIR, ReviewerWindowRecord, ReviewerWindowState,
            ReviewerWindowStore,
        };

        /// 每用例独立临时根(避免用例间残留互染)。
        fn f12_root(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
            let root = tempfile::tempdir().expect("f12 root");
            let snap = root.path().join(format!("snap-{tag}"));
            std::fs::create_dir_all(&snap).expect("snap root");
            (root, snap)
        }

        fn make_residue(snap: &std::path::Path, window_id: &str) -> std::path::PathBuf {
            let dir = snap.join("windows").join(window_id);
            std::fs::create_dir_all(dir.join("blobs")).expect("residue dir");
            std::fs::write(dir.join("blobs/aa"), b"half-written").expect("blob");
            dir
        }

        /// 红线①:崩溃半成品(未发布本窗目录)必须清理,且幂等。
        #[test]
        fn f12_cleanup_removes_unpublished_residue_and_is_idempotent() {
            let (_keep, snap) = f12_root("unpub");
            let dir = make_residue(&snap, "w-f12");

            assert_eq!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-f12"),
                Ok(true),
                "F12 红:未发布本窗残留必须被清理"
            );
            assert!(!dir.exists(), "清理后目录消失");
            assert_eq!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-f12"),
                Ok(false),
                "重复清理幂等:目录已不存在=无事可做"
            );
        }

        /// 红线②:baseline=None ≠ manifest 未发布——已发布 manifest 是
        /// F4 同窗采用路径的可恢复证据,不得删除。
        #[test]
        fn f12_cleanup_keeps_published_manifest_dir() {
            let (_keep, snap) = f12_root("pub");
            let dir = make_residue(&snap, "w-pub");
            std::fs::write(dir.join("manifest.json"), b"{}").expect("published manifest");

            assert_eq!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-pub"),
                Ok(false),
                "manifest 已发布:不清理"
            );
            assert!(dir.join("manifest.json").is_file(), "可恢复证据保留");
        }

        /// 红线③:邻窗目录零接触。
        #[test]
        fn f12_cleanup_keeps_neighbor_windows() {
            let (_keep, snap) = f12_root("nb");
            let own = make_residue(&snap, "w-own");
            let neighbor = make_residue(&snap, "w-neighbor");

            assert_eq!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-own"),
                Ok(true)
            );
            assert!(!own.exists());
            assert!(
                neighbor.join("blobs/aa").exists(),
                "邻窗未发布目录也不得误删"
            );
        }

        /// 红线④:恶意 window_id(路径穿越/分隔符/空)拒绝清理,目标零副作用。
        #[test]
        fn f12_cleanup_rejects_malicious_window_ids() {
            let (_keep, snap) = f12_root("evil");
            let escape = _keep.path().join("escape-target");
            std::fs::create_dir_all(&escape).expect("escape target");

            for bad in ["../escape-target", "a/b", ""] {
                assert!(
                    cleanup_unpublished_snapshot_window_dir(&snap, bad).is_err(),
                    "恶意 window_id {bad:?} 必须拒绝"
                );
            }
            assert!(escape.exists(), "越界目标零副作用");
        }

        /// 红线⑤:符号链接窗口目录拒绝穿透删除。
        #[cfg(unix)]
        #[test]
        fn f12_cleanup_refuses_symlinked_window_dir() {
            let (keep, snap) = f12_root("link");
            let target = make_residue(&keep.path().to_path_buf(), "real-target");
            std::fs::create_dir_all(snap.join("windows")).expect("windows dir");
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, snap.join("windows").join("w-link"))
                .expect("symlink");

            assert!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-link").is_err(),
                "符号链接目录必须拒绝穿透删除"
            );
            assert!(target.exists(), "链接目标不被删");
        }

        /// 布局钉:本模块重建的 `windows/<window_id>` 布局必须与
        /// `ReviewerSnapshotStore` 真实布局一致(store 能读到我们写的 manifest)。
        #[test]
        fn f12_snapshot_layout_pin_matches_store() {
            let (_keep, snap) = f12_root("pin");
            let identity = ReviewWorktreeLockIdentity {
                canonical_worktree: std::path::PathBuf::from("/tmp/f12-pin-worktree"),
                gitdir: std::path::PathBuf::from("/tmp/f12-pin-worktree/.git"),
                common_dir: std::path::PathBuf::from("/tmp/f12-pin-worktree/.git"),
                lease_id: "f12-pin-lease".to_string(),
            };
            let manifest = ReviewerBaselineManifest {
                schema: REVIEWER_BASELINE_SCHEMA.to_string(),
                window_id: "w-f12-pin".to_string(),
                attempt_id: "attempt-pin".to_string(),
                run_id: "run-pin".to_string(),
                lock_identity: identity,
                entries: Vec::new(),
                git_observation: GitObservationBaseline {
                    head_symref: None,
                    head_sha: "0".repeat(40),
                    index_blob: "blobs/0".repeat(1),
                    index_sha256: "0".repeat(64),
                    managed_refs: Vec::new(),
                    refs_digest: "0".repeat(64),
                    index_semantic_sha256: None,
                },
                platform_write_baseline: Vec::new(),
                git_annex_observation: None,
                scope_key: crate::cross_cutting::provider_resources::ResourceScopeKey {
                    scope_id: "f12-pin-scope".to_string(),
                    run_id: "run-pin".to_string(),
                    boot_identity: "f12-pin-boot".to_string(),
                },
                scope_manifest: Vec::new(),
                quiesced_at: "2026-10-19T00:00:00Z".to_string(),
                captured_at: "2026-10-19T00:00:00Z".to_string(),
                restore_cursor: RestoreCursor::start(),
            };
            let dir = snap.join("windows").join("w-f12-pin");
            std::fs::create_dir_all(&dir).expect("pin dir");
            std::fs::write(
                dir.join("manifest.json"),
                serde_json::to_string(&manifest).expect("serialize manifest"),
            )
            .expect("write manifest");

            let loaded = ReviewerSnapshotStore::new(&snap)
                .load_manifest("w-f12-pin")
                .expect("store 必须按同布局读到本模块写的 manifest");
            assert_eq!(loaded.window_id, "w-f12-pin");
        }

        fn f12_engine(root: &std::path::Path) -> CodingWorkspaceEngine {
            let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
            CodingWorkspaceEngine::new(
                crate::product::coding_attempt_store::CodingAttemptStore::new(
                    ProductAppPaths::new(root.join(".aria")),
                ),
                crate::product::git_workspace_service::GitWorkspaceService::new(),
                event_tx,
            )
        }

        fn f12_preparing_record(root: &std::path::Path, window_id: &str) {
            let store = ReviewerWindowStore::new(
                ProductAppPaths::new(root.join(".aria"))
                    .root()
                    .join(REVIEWER_WINDOW_STORE_DIR),
            );
            let identity = ReviewWorktreeLockIdentity {
                canonical_worktree: root.join("wt"),
                gitdir: root.join("wt/.git"),
                common_dir: root.join("wt/.git"),
                lease_id: format!("f12-lease-{window_id}"),
            };
            store
                .create_or_resume_window(&ReviewerWindowRecord::new(
                    window_id,
                    identity,
                    "attempt-f12",
                    "run-f12",
                    "claude_code",
                    Vec::new(),
                ))
                .expect("preparing record");
        }

        /// 集成红线:启动补偿弃置无 baseline 窗前,清理未发布本窗 snapshot
        /// 残留;邻窗不误删。
        #[tokio::test]
        async fn f12_startup_compensation_cleans_unpublished_residue() {
            let root = tempfile::tempdir().expect("root");
            f12_preparing_record(root.path(), "f12-win");
            let snapshot_root = ProductAppPaths::new(root.path().join(".aria"))
                .root()
                .join(REVIEWER_SNAPSHOT_STORE_DIR);
            let own = make_residue(&snapshot_root, "f12-win");
            let neighbor = make_residue(&snapshot_root, "f12-neighbor");

            let outcomes = f12_engine(root.path())
                .compensate_open_reviewer_windows_at_startup()
                .await;

            assert_eq!(outcomes.len(), 1);
            assert!(
                outcomes[0].released,
                "无 baseline Preparing 窗仍必须弃置释放"
            );
            assert!(!own.exists(), "F12 红:未发布本窗残留必须在弃置前清理");
            assert!(neighbor.exists(), "邻窗残留不误删");
            let record = ReviewerWindowStore::new(
                ProductAppPaths::new(root.path().join(".aria"))
                    .root()
                    .join(REVIEWER_WINDOW_STORE_DIR),
            )
            .load_window("f12-win")
            .expect("record");
            assert_eq!(record.state, ReviewerWindowState::ClosedInvalidated);
        }

        /// 集成对照:manifest 已发布的目录在弃置后仍保留(F4 可恢复证据)。
        #[tokio::test]
        async fn f12_startup_compensation_keeps_published_manifest() {
            let root = tempfile::tempdir().expect("root");
            f12_preparing_record(root.path(), "f12-pub");
            let snapshot_root = ProductAppPaths::new(root.path().join(".aria"))
                .root()
                .join(REVIEWER_SNAPSHOT_STORE_DIR);
            let own = make_residue(&snapshot_root, "f12-pub");
            std::fs::write(own.join("manifest.json"), b"{}").expect("published manifest");

            let outcomes = f12_engine(root.path())
                .compensate_open_reviewer_windows_at_startup()
                .await;

            assert_eq!(outcomes.len(), 1);
            assert!(outcomes[0].released, "已发布 manifest 不阻塞弃置收口");
            assert!(
                own.join("manifest.json").is_file(),
                "已发布 manifest 是可恢复证据,不得误删"
            );
        }
    }
}
