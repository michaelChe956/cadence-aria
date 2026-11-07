//! PlanRevision 具名臂（治本 Task 14b，栈根修）。
//!
//! 修前形态：`ProviderRunKind::WorkItemPlanRevision` 分支在 drive body 里
//! **内联展开** `workspace_ws_work_item_plan_revision_arm!` 宏——宏体近 500 行
//! 且含大量 await，其构造临时区全部摊在 `provider_run_drive_body` 的 poll 帧里
//! （修前该帧序言预留 724,992 B ≈ 708 KiB，是 tokio worker 2 MiB 栈被吃穿的
//! 主因之一）。
//!
//! 修后形态：本文件承载该臂的**具名构造函数**：宏展开发生在本函数自己的栈帧
//! 内，完成后立即 `Box::pin` 装箱到堆——drive 分发点的 poll 帧里只剩 16 字节
//! 胖指针，不再搬运宏体临时区。
//!
//! 语义等价性（一条不许变）：
//! - 宏体尾是**无 break 的 loop** ⇒ 宏恒以 drive 级 `return` 收口（编译器已
//!   证明其后代码不可达）——等价 **ArmExit::Stop 语义**：父 drive 不再走
//!   followups、不补一次通用 finish、不补错误帧；engine guard 随宏体作用域
//!   结束释放（与原 drive 级 return 同一时点，其间无 await）。
//! - 宏体内 `drop($engine)` 的**先释放 engine 锁、再 await 收口帧**的次序逐字
//!   保留（锁内 await 会改变接替 run 的取锁时序）。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：engine 的
//! `MutexGuard` 按值属本臂；idle `provider_drive_guard` 仍是外层 `Option` 的
//! 可变借用；`entity_run_lease` 仍是独立外层 binding（不传给臂）。

use super::task_body::ArmDrive;
use super::*;
use std::pin::Pin;

/// PlanRevision 具名构造函数（治本）：构造在本函数自己的栈帧内完成、随即
/// 装箱到堆；drive 分发点的 poll 帧只留胖指针。
pub(crate) fn drive_plan_revision_arm<'e, 'd>(
    drive: ArmDrive<'e, 'd>,
    feedback: Option<String>,
) -> Pin<Box<dyn Future<Output = ()> + Send + 'd>>
where
    'e: 'd,
{
    Box::pin(async move {
        let ArmDrive {
            mut engine,
            command_rx,
            provider: provider_for_run,
            manager: manager_for_task,
            run_context: run_context_clone,
            outbound_tx: outbound_tx_for_task,
            run_label,
            run_token,
            provider_drive_guard,
            run_cancel,
            ..
        } = drive;
        // 宏体按值取用 engine（宏内 `drop($engine)` 的释放时点不变）；宏体恒以
        // drive 级 `return` 收口（尾是无 break 的 loop）——本块不会正常走完。
        workspace_ws_work_item_plan_revision_arm!(
            engine,
            run_context_clone,
            provider_for_run,
            run_cancel,
            command_rx,
            run_label,
            outbound_tx_for_task,
            manager_for_task,
            run_token,
            provider_drive_guard,
            feedback
        );
    })
}
