//! followups 具名臂（治本 Task 14b，栈根修）。
//!
//! 修前形态：`workspace_ws_provider_run_followups!` 宏在 drive body 里**内联
//! 展开**——宏体含大量 await 与 drive 级 `return`，其构造临时区全部摊在
//! `provider_run_drive_body` 的 poll 帧里（修前该帧序言预留 724,992 B）。
//!
//! 修后形态：本文件承载 followups 的**具名构造函数**：宏展开发生在本函数
//! 自己的栈帧内，完成后立即 `Box::pin` 装箱到堆——drive 分发点的 poll 帧里
//! 只剩胖指针。
//!
//! 语义等价性（一条不许变）：
//! - 宏内 `return` = 原 drive 级 `return`：**父层不再补一次通用 finish、不补
//!   错误帧**（宏内各错误路径自身已按原顺序完成 mark/drop/回显/finish_run
//!   收口）——本臂以「engine 未交还」（`engine_out` 保持 `None`）表达该出口；
//! - 宏体正常穿过时 engine 按值交还父层（`engine_out = Some(guard)`），父层
//!   照旧执行 drive 正常尾部（mark_active_run_finished → drop engine → 释放
//!   idle guard → finish_run），次序不变；
//! - 宏体内 `drop($engine)` 的**先释放 engine 锁、再 await 收口帧/finish_run**
//!   的次序逐字保留。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：engine 的
//! `MutexGuard` 按值属本臂；idle `provider_drive_guard` 仍是外层 `Option` 的
//! 可变借用（臂仅既有分支 `.take()`）；`entity_run_lease` 仍是独立外层 binding。

use super::*;
use std::pin::Pin;

/// followups 具名构造函数（治本）：构造在本函数自己的栈帧内完成、随即装箱
/// 到堆；drive 分发点的 poll 帧只留胖指针。
///
/// `engine_out` 是 engine 的交还槽：`Some(guard)` = 宏体正常穿过（父层继续
/// 正常尾部）；保持 `None` = 宏内提前 `return`（父层立即返回，不补 finish/
/// 错误帧）。
#[allow(clippy::too_many_arguments)] // 与宏入参逐项同源，不合并包一层。
pub(crate) fn drive_provider_run_followups<'e, 'd, 'a>(
    engine: tokio::sync::MutexGuard<'e, WorkspaceEngine>,
    provider_registry_for_run: &'d Arc<ProviderRegistry>,
    manager_for_task: &'d Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    run_token: u64,
    run_label: &'d str,
    outbound_tx_for_task: &'d mpsc::Sender<OutboundControl>,
    run_cancel: &'d CancellationToken,
    run_context_clone: &'d ProviderRunContext,
    provider_drive_guard: &'d mut Option<crate::web::state::WorkspaceProviderDriveGuard>,
    engine_out: &'a mut Option<tokio::sync::MutexGuard<'e, WorkspaceEngine>>,
) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
where
    'e: 'a,
    'd: 'a,
{
    Box::pin(async move {
        // 宏体按值取用 engine（宏内 `drop($engine)` 的释放时点不变）；宏体正常
        // 穿过时把 engine 原样交还槽位，宏内提前 `return` 时不交还。
        let mut engine_slot = Some(engine);
        {
            let mut engine = engine_slot
                .take()
                .expect("engine guard present at run followups entry");
            workspace_ws_provider_run_followups!(
                engine,
                provider_registry_for_run,
                manager_for_task,
                run_token,
                run_label,
                outbound_tx_for_task,
                run_cancel,
                run_context_clone,
                provider_drive_guard
            );
            engine_slot = Some(engine);
        }
        *engine_out = engine_slot;
    })
}
