//! run 生命周期：启动/中止/完成与活跃 run 的命令通道管理。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::{ActiveRun, WorkspaceSessionManager};
use crate::web::workspace_ws_handler::{ProviderCommand, ProviderRunKind};

impl WorkspaceSessionManager {
    /// REQ-DLS-01：run 层的 lease 复核在状态锁内重读 attachment 当前 epoch，
    /// 不使用 socket 建立时刻的 epoch 快照——自愈授予或重新接管都会推进
    /// epoch，快照比对会把紧随其后的 run 写以旧 epoch 误拒。
    fn attachment_holds_lease(state: &super::ManagerState, connection_id: Option<&str>) -> bool {
        let Some(connection_id) = connection_id else {
            return true;
        };
        let attachment_epoch = state
            .attachments
            .get(connection_id)
            .or_else(|| state.pending_attachments.get(connection_id))
            .map(|attachment| attachment.lease_epoch);
        state.lease.holder.as_deref() == Some(connection_id)
            && attachment_epoch == Some(state.lease.epoch)
    }

    /// 每次启动都在 `start_run_from_attachment` 的同一 manager 临界区内完成
    /// lease epoch 校验、旧 run 取出与新 run 登记。socket 路径额外在取得 engine 锁前
    /// 通过 `abort_active_run_from_attachment` 低延迟中止当前 run；启动前仍复检 epoch。
    pub async fn start_run(
        &self,
        kind: ProviderRunKind,
        requested_node_id: Option<String>,
    ) -> Result<
        (
            u64,
            u64,
            CancellationToken,
            mpsc::Receiver<ProviderCommand>,
            Option<String>,
        ),
        String,
    > {
        self.start_run_from_attachment(None, kind, requested_node_id)
            .await
    }

    /// 在取得 engine 锁前，以 attachment epoch 核验 lease 并立即 supersede 当前
    /// run。故已被接管的迟到写永远不会取消 run，同时保留同一 holder 覆盖流式 run 的
    /// 既有低延迟时序。
    pub async fn abort_active_run_from_attachment(
        &self,
        connection_id: Option<&str>,
    ) -> Result<(), String> {
        let run = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !Self::attachment_holds_lease(&state, connection_id) {
                return Err("STALE_DRIVER_LEASE".to_string());
            }
            let run = state.active_run.take();
            if run.is_some() {
                state.journal.mark_run_terminal();
            }
            run
        };
        if let Some(run) = run {
            Self::cancel_run(&run);
        }
        Ok(())
    }

    /// 在 provider 启动前再次检查 socket attachment 的 lease epoch，并在同一 manager
    /// 临界区内取出被覆盖的 run、登记新 run，避免并发 relay/socket 启动覆盖 run 时
    /// 遗留未受 manager 管理且未取消的 provider task。
    pub async fn start_run_from_attachment(
        &self,
        connection_id: Option<&str>,
        kind: ProviderRunKind,
        requested_node_id: Option<String>,
    ) -> Result<
        (
            u64,
            u64,
            CancellationToken,
            mpsc::Receiver<ProviderCommand>,
            Option<String>,
        ),
        String,
    > {
        // D2.1a（fix-cascade-deletion）：实体删除后的旧 Arc dispatch 检查——
        // 失效 manager 不再登记任何新 run（不 cancel 在途 run，无副作用）。
        if self.invalidated_for_entity_deletion() {
            return Err(super::ENTITY_DELETED_STABLE_MESSAGE.to_string());
        }
        let (replaced_run, run) = {
            let (command_tx, command_rx) = mpsc::channel(8);
            let cancel = CancellationToken::new();
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !Self::attachment_holds_lease(&state, connection_id) {
                return Err("STALE_DRIVER_LEASE".to_string());
            }
            let replaced_run = state.active_run.take();
            state.next_run_id += 1;
            let run_id = state.next_run_id;
            let token = crate::web::workspace_ws_handler::NEXT_ACTIVE_RUN_TOKEN
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // 批 2 T2：completion watch 随 run 登记——finish_run 按 token 发
            // 真实收口信号（abort 等待者在锁外订阅）。
            let (completion_tx, _completion_rx) = watch::channel(false);
            state.run_completions.insert(token, completion_tx);
            let node_id = requested_node_id;
            let lease_epoch = state.lease.epoch;
            state.active_run = Some(ActiveRun {
                id: run_id,
                token,
                node_id: node_id.clone(),
                kind,
                cancel: cancel.clone(),
                command_tx,
                pending_choices: Arc::new(StdMutex::new(Vec::new())),
                lease_epoch,
                run_incarnation: uuid::Uuid::new_v4().to_string(),
            });
            state
                .journal
                .mark_run_started(self.next_event_seq.load(Ordering::Relaxed));
            (replaced_run, (run_id, token, cancel, command_rx, node_id))
        };
        if let Some(run) = replaced_run {
            let stale_choice_ids = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                super::choices::retire_claims_for_run(&mut state, &run.run_incarnation);
                // 批 2 N1（REQ-WIGA-12）：被顶替 run 未答挂起 choice 的失效
                // 题锁外清（原问题随进程丢失，不能伪造成可答题）。
                super::choices::unanswered_choice_ids(&run)
            };
            if !stale_choice_ids.is_empty() {
                self.resolve_stale_plan_choice_facts(&stale_choice_ids);
            }
            Self::cancel_run(&run);
        }
        // REQ-PRB-05：登记后尽力准备 scratch（清空重建 + 0700），失败只 warn。
        let store = crate::product::lifecycle_store::LifecycleStore::new(self.app_paths.clone());
        if let Err(error) = store.prepare_run_scratch(&self.session_id) {
            tracing::warn!(session_id = %self.session_id, error = %error, "run scratch prepare failed");
        }
        Ok(run)
    }

    pub async fn finish_run(self: &Arc<Self>, token: u64) {
        // 批 2 N1（REQ-WIGA-12）：本 run 未答挂起 choice 的失效题在状态锁
        // 外清（enrollment 文件锁不得与 manager 状态锁互嵌）。
        let mut stale_choice_ids = Vec::new();
        // REQ-PRB-06：判定 MUST 在状态锁内作出（见下方赋值），删除在锁外。
        let no_active_run;
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(run) = state.active_run.take() {
                if run.token == token {
                    super::choices::retire_claims_for_run(&mut state, &run.run_incarnation);
                    state.journal.mark_run_terminal();
                    stale_choice_ids = super::choices::unanswered_choice_ids(&run);
                } else {
                    // 非本人 token：恢复原 run，不误伤。
                    state.active_run = Some(run);
                }
            }
            // 批 2 T2（REQ-WIGA-09）：真实 completion——任务收口点按 token
            // 发信号（含 abort 已摘除活跃 run 的迟到收口；等待者锁外解除）。
            if let Some(completion_tx) = state.run_completions.remove(&token) {
                completion_tx.send_replace(true);
            }
            // 锁内取值："此刻已无活动 run"。旧 token 的迟到收口会看到新 run
            // 仍在位 → false，故不会删掉新 run 的 scratch（supersede 竞态）。
            no_active_run = state.active_run.is_none();
        }
        if !stale_choice_ids.is_empty() {
            self.resolve_stale_plan_choice_facts(&stale_choice_ids);
        }
        // REQ-PRB-06：唯一收口点（正常结束/取消收口/abort 迟到收口共用）。
        // 仅在此时已无活动 run 才删除；发生在 drop(engine) 之后（provider_run.rs
        // 先 drop 再 finish_run），删除失败只 warn——残留由下次注册的清空与启动
        // sweep 回收，不影响收口。
        if no_active_run {
            let store =
                crate::product::lifecycle_store::LifecycleStore::new(self.app_paths.clone());
            if let Err(error) = store.cleanup_run_scratch(&self.session_id) {
                tracing::warn!(session_id = %self.session_id, error = %error, "run scratch cleanup failed");
            }
        }
        self.maybe_recycle().await;
    }

    /// 批 2 T2（REQ-WIGA-09）：取消当前活跃 run 并等待**真实 completion**——
    /// 只有该 run 任务自身的收口点（`finish_run` 按 token 发信号）才解除
    /// 等待；`is_active_run()==false` 只是状态摘除，不构成完成证据。取消在
    /// 状态锁内完成、等待在锁外（不持 engine 锁）；无活跃 run 直接返回
    /// false。30s 兜底超时防任务失联（打点后放行，终止收口继续）。
    pub async fn abort_active_run_and_wait_completion(&self) -> bool {
        let (completion, stale_choice_ids) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let run = state.active_run.take();
            if let Some(run) = run.as_ref() {
                super::choices::retire_claims_for_run(&mut state, &run.run_incarnation);
                state.journal.mark_run_terminal();
            }
            // 批 2 N1（REQ-WIGA-12）：消亡 run 未答挂起 choice 的失效题
            // 锁外清（原问题随进程丢失）。
            let stale_choice_ids = run
                .as_ref()
                .map(super::choices::unanswered_choice_ids)
                .unwrap_or_default();
            let completion = run.map(|run| {
                let token = run.token;
                let completion_rx = state.run_completions.get(&token).map(|tx| tx.subscribe());
                Self::cancel_run(&run);
                (token, completion_rx)
            });
            (completion, stale_choice_ids)
        };
        if !stale_choice_ids.is_empty() {
            self.resolve_stale_plan_choice_facts(&stale_choice_ids);
        }
        let Some((token, completion_rx)) = completion else {
            return false;
        };
        let Some(mut completion_rx) = completion_rx else {
            // 无 completion 登记（旧 fixture/已被收口）：视作已完成。
            return true;
        };
        let waited = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                if *completion_rx.borrow() {
                    break;
                }
                if completion_rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
        if waited.is_err() {
            eprintln!(
                "[aria-termination] workspace run completion wait timed out run_token={token}"
            );
        }
        true
    }

    pub async fn abort_active_run(&self) -> bool {
        let (run, stale_choice_ids) = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let run = state.active_run.take();
            if let Some(run) = run.as_ref() {
                super::choices::retire_claims_for_run(&mut state, &run.run_incarnation);
                state.journal.mark_run_terminal();
            }
            // 批 2 N1（REQ-WIGA-12）：同 abort_active_run_and_wait_completion。
            let stale_choice_ids = run
                .as_ref()
                .map(super::choices::unanswered_choice_ids)
                .unwrap_or_default();
            (run, stale_choice_ids)
        };
        if !stale_choice_ids.is_empty() {
            self.resolve_stale_plan_choice_facts(&stale_choice_ids);
        }
        if let Some(run) = run {
            Self::cancel_run(&run);
            true
        } else {
            false
        }
    }

    pub async fn active_run(&self) -> Option<ActiveRun> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_run
            .clone()
    }

    pub async fn active_run_command_tx(&self) -> Option<mpsc::Sender<ProviderCommand>> {
        self.active_run().await.map(|run| run.command_tx)
    }

    pub async fn replace_command_tx_if_token(
        &self,
        token: u64,
        command_tx: mpsc::Sender<ProviderCommand>,
    ) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(run) = state.active_run.as_mut()
            && run.token == token
        {
            run.command_tx = command_tx;
        }
    }

    /// P1 WIGA Task 5（REQ-WIGA-03）：auto 认领专用非 superseding 启动。
    /// 同一 manager 临界区内复核 active run：已有活 run（任何来源）返回
    /// `None` 只观察，绝不取消既有 run/claims；空闲才登记新 run。与
    /// `start_run_from_attachment` 的显式 supersede 语义互不影响。
    pub async fn try_start_run_if_idle(
        &self,
        kind: ProviderRunKind,
        requested_node_id: Option<String>,
    ) -> Result<
        Option<(
            u64,
            u64,
            CancellationToken,
            mpsc::Receiver<ProviderCommand>,
            Option<String>,
        )>,
        String,
    > {
        // D2.1a：同 start_run_from_attachment 的旧 Arc dispatch 检查。
        if self.invalidated_for_entity_deletion() {
            return Err(super::ENTITY_DELETED_STABLE_MESSAGE.to_string());
        }
        let (command_tx, command_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let run = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.active_run.is_some() {
                return Ok(None);
            }
            state.next_run_id += 1;
            let run_id = state.next_run_id;
            let token = crate::web::workspace_ws_handler::NEXT_ACTIVE_RUN_TOKEN
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // 批 2 T2：同 start_run_from_attachment——completion watch 随 run 登记。
            let (completion_tx, _completion_rx) = watch::channel(false);
            state.run_completions.insert(token, completion_tx);
            let node_id = requested_node_id;
            let lease_epoch = state.lease.epoch;
            state.active_run = Some(ActiveRun {
                id: run_id,
                token,
                node_id: node_id.clone(),
                kind,
                cancel: cancel.clone(),
                command_tx,
                pending_choices: Arc::new(StdMutex::new(Vec::new())),
                lease_epoch,
                run_incarnation: uuid::Uuid::new_v4().to_string(),
            });
            state
                .journal
                .mark_run_started(self.next_event_seq.load(Ordering::Relaxed));
            (run_id, token, cancel, command_rx, node_id)
        };
        // REQ-PRB-05：登记后尽力准备 scratch（清空重建 + 0700），失败只 warn。
        let store = crate::product::lifecycle_store::LifecycleStore::new(self.app_paths.clone());
        if let Err(error) = store.prepare_run_scratch(&self.session_id) {
            tracing::warn!(session_id = %self.session_id, error = %error, "run scratch prepare failed");
        }
        Ok(Some(run))
    }

    /// test-only：活跃 run 的可比对快照（token/run_incarnation），供自动认领
    /// 不 supersede 断言；不进入生产接口。
    #[cfg(test)]
    pub fn active_run_ref_for_test(&self) -> Option<super::ActiveRunRefForTest> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .active_run
            .as_ref()
            .map(|run| super::ActiveRunRefForTest {
                token: run.token,
                run_incarnation: run.run_incarnation.clone(),
            })
    }

    pub fn is_active_run(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.active_run.is_some())
            .unwrap_or(true)
    }

    fn cancel_run(run: &ActiveRun) {
        eprintln!(
            "[aria-cancellation] workspace abort_workspace_run cancelling runner token trigger=abort_workspace_run run_id=run-{} run_token={} node_id={:?}",
            run.id, run.token, run.node_id
        );
        let _ = run.command_tx.try_send(ProviderCommand::Abort);
        run.cancel.cancel();
    }
}

// ---------------------------------------------------------------------------
// 批 2 T2（REQ-WIGA-09）：run completion 真等待与 plan 终止引擎侧收口。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod termination_tests {
    use std::time::Duration;

    use super::WorkspaceSessionManager;
    use crate::web::workspace_ws_handler::ProviderRunKind;

    /// 红测①：completion 真等待——abort 摘除活跃 run 后，任务真实收口点
    /// `finish_run`（按 token 发 completion）未发生前等待不得返回；
    /// `is_active_run()==false` 只是状态摘除，不构成完成证据。
    #[tokio::test]
    async fn workspace_run_completion_waits_for_real_finish_run() {
        let manager = WorkspaceSessionManager::test_fixture("session_completion_wait_t2");
        let (_run_id, token, _cancel, _command_rx, _node) = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");
        assert!(manager.is_active_run());

        let wait = manager.abort_active_run_and_wait_completion();
        tokio::pin!(wait);
        // 任务尚未收口（无人调 finish_run）：100ms 内不得返回。
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            _ = &mut wait => panic!(
                "completion must wait for the real finish_run, not is_active_run()==false"
            ),
        }
        // 真实收口点 finish_run 发 completion：等待立即解除。
        manager.finish_run(token).await;
        tokio::time::timeout(Duration::from_secs(5), &mut wait)
            .await
            .expect("completion must be signalled by finish_run");
    }

    /// 取消等待不持 engine 锁：provider task 长持 engine 锁时，取消+等待
    /// 仍能推进（等待点在状态锁外）。
    #[tokio::test]
    async fn workspace_run_completion_wait_does_not_hold_engine_lock() {
        let manager = WorkspaceSessionManager::test_fixture("session_completion_lock_t2");
        let (_run_id, token, _cancel, _command_rx, _node) = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");
        // 模拟 provider run 持有 engine 锁直到收口。
        let engine_for_holder = manager.engine();
        let holder_token = token;
        let holder = tokio::spawn(async move {
            let _guard = engine_for_holder.lock().await;
            tokio::time::sleep(Duration::from_millis(150)).await;
        });
        // 取消等待与 engine 锁持有并行：不得死锁。
        let manager_for_wait = manager.clone();
        let wait = tokio::spawn(async move {
            manager_for_wait
                .abort_active_run_and_wait_completion()
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        manager.finish_run(holder_token).await;
        let had_run = tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .expect("completion wait must not deadlock on the engine lock")
            .expect("wait task alive");
        assert!(had_run);
        let _ = holder.await;
    }
}

// ---------------------------------------------------------------------------
// 批 2 N1（REQ-WIGA-12）：run 终态按 incarnation 清失效 plan choice 题。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod n1_tests {
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::logical_codebase::EnrollmentTarget;
    use crate::product::models::automation::{
        AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
        EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand,
    };
    use crate::product::models::{IssueWorkItemPlanOptions, ProviderName};
    use crate::web::workspace_ws_handler::ProviderRunKind;
    use crate::web::workspace_ws_types::{ChoiceOption, WsOutMessage};

    const N1_PROJECT: &str = "project_test";
    const N1_ISSUE: &str = "issue_test";

    fn n1_choice_frame(id: &str) -> WsOutMessage {
        WsOutMessage::ChoiceRequest {
            id: id.to_string(),
            prompt: "验收口径需要裁定".to_string(),
            options: vec![ChoiceOption {
                id: "option-a".to_string(),
                label: "按全局口径".to_string(),
                description: None,
            }],
            allow_multiple: false,
            allow_free_text: false,
            questions: Vec::new(),
            source: "ask_user_question".to_string(),
        }
    }

    fn bound_manager(
        session_id: &str,
        plan_id: &str,
    ) -> (
        std::sync::Arc<super::WorkspaceSessionManager>,
        IssueAutomationStore,
        crate::product::models::automation::EnrollmentBindingIdentity,
    ) {
        // test_fixture 的 app_paths 固定在 /tmp/<session_id>——pid 后缀隔离
        // 历史 cargo test 进程残留（enrollment 幂等重放会冻结既有 revision）。
        let session_id = format!("{session_id}_{}", std::process::id());
        let manager = super::WorkspaceSessionManager::test_fixture(&session_id);
        let store = IssueAutomationStore::new(manager.app_paths.clone());
        store
            .compare_and_set(
                N1_PROJECT,
                N1_ISSUE,
                None,
                EnrollmentWriteCommand::Enable {
                    command_id: None,
                    selection_key: format!("n1-runs-{session_id}"),
                    source: EnrollmentSource {
                        stories: Vec::new(),
                        designs: Vec::new(),
                    },
                    options: EnrollmentOptions {
                        author_provider: ProviderName::Fake,
                        reviewer_provider: ProviderName::Fake,
                        review_rounds: 0,
                        superpowers_enabled: false,
                        openspec_enabled: false,
                        plan_approval: crate::product::models::automation::PlanApprovalPolicy::Auto,
                        plan_options: IssueWorkItemPlanOptions {
                            include_integration_tests: false,
                            include_e2e_tests: false,
                            force_frontend_backend_split: false,
                            require_execution_plan_confirm: false,
                        },
                    },
                    target: EnrollmentTarget::SingleRepository {
                        repository_id: "repo_n1".to_string(),
                    },
                },
            )
            .expect("enable enrollment");
        let revision = store
            .get(N1_PROJECT, N1_ISSUE)
            .expect("read enrollment")
            .expect("enrollment present")
            .policy_revision;
        store
            .bind_plan(N1_PROJECT, N1_ISSUE, revision, plan_id, &manager.session_id)
            .expect("bind plan");
        let enrollment = store
            .get(N1_PROJECT, N1_ISSUE)
            .expect("read enrollment")
            .expect("enrollment present");
        let binding = crate::web::advance_plan::load_current_enrollment_binding(&enrollment)
            .expect("current binding");
        (manager, store, binding)
    }

    fn fact(
        manager: &super::WorkspaceSessionManager,
        plan_id: &str,
        key: String,
        kind: AutomationAttentionKind,
        choice_id: Option<&str>,
    ) -> AutomationAttentionFact {
        AutomationAttentionFact {
            key,
            kind,
            stage: AutomationAttentionStage::Plan,
            plan_id: Some(plan_id.to_string()),
            session_id: Some(manager.session_id.clone()),
            attempt_id: None,
            reason_code: if choice_id.is_some() {
                "plan_choice_pending".to_string()
            } else {
                "plan_provider_failed".to_string()
            },
            occurred_at: "2026-10-16T00:00:00Z".to_string(),
            choice_id: choice_id.map(str::to_string),
            gate_id: None,
        }
    }

    fn fact_keys(store: &IssueAutomationStore) -> Vec<String> {
        let mut keys = store
            .get(N1_PROJECT, N1_ISSUE)
            .unwrap()
            .expect("enrollment present")
            .attention_facts
            .into_iter()
            .map(|fact| fact.key)
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    /// 红测③：finish_run 后消亡 run 未答挂起 choice 的题消失；durable
    /// provider 失败事实（P4 写者）不误清。
    #[tokio::test]
    async fn finish_run_clears_stale_plan_choice_facts_but_keeps_durable_failures() {
        let (manager, store, binding) =
            bound_manager("workspace_session_n1_finish_run", "plan_n1_finish_run");
        let stale_key =
            crate::web::plan_generation::plan_choice_fact_key(&manager.session_id, "choice_stale");
        let failed_key =
            crate::web::plan_generation::plan_failed_node_fact_key(&manager.session_id);
        store
            .upsert_automation_attention_fact(
                N1_PROJECT,
                N1_ISSUE,
                &binding,
                fact(
                    &manager,
                    "plan_n1_finish_run",
                    stale_key.clone(),
                    AutomationAttentionKind::PlanQuestion,
                    Some("choice_stale"),
                ),
            )
            .expect("seed choice fact");
        store
            .upsert_automation_attention_fact(
                N1_PROJECT,
                N1_ISSUE,
                &binding,
                fact(
                    &manager,
                    "plan_n1_finish_run",
                    failed_key.clone(),
                    AutomationAttentionKind::AutomationFailed,
                    None,
                ),
            )
            .expect("seed failed fact");

        let (_run_id, token, _cancel, _command_rx, _node) = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");
        manager.register_pending_choice_frame(n1_choice_frame("choice_stale"));
        assert_eq!(
            fact_keys(&store),
            vec![stale_key.clone(), failed_key.clone()],
            "run 存活期事实保持"
        );

        manager.finish_run(token).await;
        assert_eq!(
            fact_keys(&store),
            vec![failed_key],
            "run 终态清失效 choice 题，durable Failed 保留"
        );
    }

    /// 红测③：被顶替（supersede）的 run 携带未答挂起 choice 消亡时，其
    /// 失效题同样按 incarnation 清除。
    #[tokio::test]
    async fn superseded_run_stale_choice_facts_resolve_on_replacement() {
        let (manager, store, binding) =
            bound_manager("workspace_session_n1_supersede", "plan_n1_supersede");
        let stale_key = crate::web::plan_generation::plan_choice_fact_key(
            &manager.session_id,
            "choice_superseded",
        );
        store
            .upsert_automation_attention_fact(
                N1_PROJECT,
                N1_ISSUE,
                &binding,
                fact(
                    &manager,
                    "plan_n1_supersede",
                    stale_key,
                    AutomationAttentionKind::PlanQuestion,
                    Some("choice_superseded"),
                ),
            )
            .expect("seed choice fact");

        let _run_a = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run a");
        manager.register_pending_choice_frame(n1_choice_frame("choice_superseded"));
        // 新 run 顶替旧 run：旧 run 的失效题随顶替清除。
        let _run_b = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run b");
        assert!(
            fact_keys(&store).is_empty(),
            "被顶替 run 的失效 choice 题已清"
        );
    }
}

#[cfg(test)]
mod manager_run_scratch_tests {
    use super::WorkspaceSessionManager;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::web::workspace_ws_handler::ProviderRunKind;

    fn scratch_of(manager: &WorkspaceSessionManager) -> std::path::PathBuf {
        LifecycleStore::new(manager.app_paths.clone()).run_scratch_dir(&manager.session_id)
    }

    #[tokio::test]
    async fn start_run_from_attachment_prepares_run_scratch() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-supersede");
        let scratch = scratch_of(&manager);
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("old.md"), "前序 run 残留").unwrap();

        let _ = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");

        assert!(scratch.is_dir());
        assert!(!scratch.join("old.md").exists(), "新 run 不得看见前序内容");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[tokio::test]
    async fn try_start_run_if_idle_prepares_run_scratch() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-idle");
        let scratch = scratch_of(&manager);
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("old.md"), "前序 run 残留").unwrap();

        let started = manager
            .try_start_run_if_idle(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("claim if idle")
            .is_some();
        assert!(started);
        assert!(!scratch.join("old.md").exists());
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[tokio::test]
    async fn restart_within_run_keeps_scratch_content() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-restart");
        let (_, token, _, _rx, _) = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");
        let scratch = scratch_of(&manager);
        std::fs::write(scratch.join("keep.md"), "同 run 暂存").unwrap();

        let (tx, _rx2) = tokio::sync::mpsc::channel(8);
        manager.replace_command_tx_if_token(token, tx).await;

        assert!(
            scratch.join("keep.md").exists(),
            "同 run 内重启不得清空 scratch"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[tokio::test]
    async fn finish_run_removes_scratch_when_no_active_run() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-finish");
        let (_, token, _, _rx, _) = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");
        let scratch = scratch_of(&manager);
        std::fs::write(scratch.join("draft.md"), "x").unwrap();

        manager.finish_run(token).await;

        assert!(!scratch.exists(), "唯一活动 run 收口后删除 scratch");
    }

    /// 契约场景「取消与失败路径不漏清」：取消收口走 abort 摘除活跃 run 后由
    /// 任务自身迟到 `finish_run` 收口（`cancel_settle.inc.rs` 的同一路径），
    /// 此时状态锁内已无活动 run → 仍须删除 scratch。
    #[tokio::test]
    async fn aborted_run_late_finish_run_removes_scratch() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-abort");
        let (_, token, _, _rx, _) = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");
        let scratch = scratch_of(&manager);
        std::fs::write(scratch.join("draft.md"), "x").unwrap();

        assert!(manager.abort_active_run().await, "取消收口摘除活跃 run");
        assert!(manager.active_run().await.is_none());
        manager.finish_run(token).await;

        assert!(!scratch.exists(), "abort 迟到收口不得漏清 scratch");
    }

    #[tokio::test]
    async fn late_finish_run_of_superseded_token_keeps_new_scratch() {
        // Review Focus #3：旧 token 的迟到收口不得删掉新 run 的 scratch。
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-supersede-race");
        let (_, stale_token, _, _rx, _) = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run stale");
        let _ = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run live");
        let scratch = scratch_of(&manager);
        std::fs::write(scratch.join("keep.md"), "新 run 暂存").unwrap();

        manager.finish_run(stale_token).await;

        assert!(
            scratch.join("keep.md").exists(),
            "旧 token 迟到收口必须跳过清理"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[tokio::test]
    async fn finish_run_cleanup_failure_does_not_block_completion() {
        let manager = WorkspaceSessionManager::test_fixture("n1-run-scratch-guard");
        let (_, token, _, _rx, _) = manager
            .start_run(ProviderRunKind::WorkItemPlanBatch, None)
            .await
            .expect("start run");
        let scratch = scratch_of(&manager);
        let external = manager.app_paths.root().join("external-target");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("keep.md"), "keep").unwrap();
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&external, &scratch).unwrap();

        manager.finish_run(token).await;

        assert!(manager.active_run().await.is_none(), "收口照常完成");
        assert!(
            external.join("keep.md").exists(),
            "护栏拒绝删除符号链接目标"
        );
        let _ = std::fs::remove_file(&scratch);
        let _ = std::fs::remove_dir_all(&external);
    }
}
