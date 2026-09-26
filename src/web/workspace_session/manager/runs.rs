//! run 生命周期：启动/中止/完成与活跃 run 的命令通道管理。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::mpsc;
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
            {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                super::choices::retire_claims_for_run(&mut state, &run.run_incarnation);
            }
            Self::cancel_run(&run);
        }
        Ok(run)
    }

    pub async fn finish_run(self: &Arc<Self>, token: u64) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(run) = state.active_run.take() {
                if run.token == token {
                    super::choices::retire_claims_for_run(&mut state, &run.run_incarnation);
                    state.journal.mark_run_terminal();
                } else {
                    // 非本人 token：恢复原 run，不误伤。
                    state.active_run = Some(run);
                }
            }
        }
        self.maybe_recycle().await;
    }

    pub async fn abort_active_run(&self) -> bool {
        let run = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let run = state.active_run.take();
            if let Some(run) = run.as_ref() {
                super::choices::retire_claims_for_run(&mut state, &run.run_incarnation);
                state.journal.mark_run_terminal();
            }
            run
        };
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
