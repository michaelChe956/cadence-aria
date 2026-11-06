//! S2（任务2）小外壳 + boxed body/drive：provider run 任务的三段边界。
//!
//! 1. `provider_run_task`（小外壳）：spawn 捕获面＝任务参数本身；task
//!    body 的构造表达式位于外壳 future **首次 poll 内**并立即
//!    `Box::pin`——深 inbound 链上只构造/搬移外壳（≤32 KiB 门禁），不再
//!    构造巨型 body（任务1 是在 spawn 调用侧直接构造具名 body 的
//!    S0 过渡形态）。
//! 2. `provider_run_task_body`（任务 body）：持三守卫外层与准入后装配——
//!    engine 锁获取 + token 安装、boxed drive 的唯一构造点、取消兜底
//!    参数交既有 settle helper。
//! 3. `provider_run_drive_body`（drive）：engine MutexGuard 按值随 A 属于
//!    drive；run_kind 只分发实际选中的臂（Continue 才 followups+正常
//!    尾部，Stop 立即返回）。P3（任务4）起 legacy outline 三族、P4（任务5）
//!    起 draft/batch 两族均拆入各自具名臂文件；revision/SC 手工修订等
//!    后续族按任务 6-8 逐族迁移后再冻结体积门禁。
//!
//! 三守卫不可混用（r16 现场死锁根修，缺陷#10家族——设计 §3.4）：
//! - engine 的 `MutexGuard` 按值属 drive/当前臂：drop 取消 drive 必须释放它，
//!   且先于兜底重取锁；
//! - idle `provider_drive_guard` 是外层 `Option<WorkspaceProviderDriveGuard>`
//!   （臂借 `&mut`，drop drive **不等于**释放 idle）；
//! - `entity_run_lease` 是独立外层 binding（覆盖 drive、取消兜底与最终
//!   store 写，接替 run 自行另 acquire 新 lease）。
//!
//! 不得用「provider_drive_guard 随 drive 一律释放」的歧义说法代替三者合同。

use super::*;

use super::basic_arms::{
    drive_author_arm, drive_author_choice_followup_arm, drive_review_only_arm, drive_revision_arm,
};
use super::legacy_outline::drive_legacy_outline_arm;
use super::plan_batch::drive_plan_batch_arm;
use super::plan_draft::drive_plan_draft_arm;
use super::plan_revision::drive_plan_revision_arm;
use super::run_followups::drive_provider_run_followups;
use super::sc_manual_revision::drive_human_gate_sc_revision_arm;
use super::single_candidate_arm::drive_single_candidate_author_arm;
use std::pin::Pin;

/// 臂级控制流合同（设计 §3.3）：不是新的业务错误抽象。`Continue` 携带按值
/// 交还的 engine MutexGuard（原正常穿过 match 末尾的路径）；`Stop` 表示臂内
/// 已按原顺序完成该分支收口并退出了原 drive 级 `return` 的路径——父 drive
/// 遇 Stop 立即返回：不继续 followups、不补一次通用 finish、不补错误帧。
/// P2（任务3）冻结：本合同即基础族/SC/后续各族臂的唯一控制流出口，后续
/// 任务不得另造错误枚举或第二套 Stop 语义。
pub(crate) enum ArmExit<'e> {
    /// 正常穿过 match：guard 交给下一段（followups + 正常尾部）。
    Continue(tokio::sync::MutexGuard<'e, WorkspaceEngine>),
    /// 原 drive 级 return：臂内收口已完成，drive 立即结束。
    Stop,
}

/// 重子调用装箱助手（治本 Task 14b，栈根修）：被包的 future 在**本助手
/// 自己的栈帧**内构造，随即装箱到堆——调用方（各臂）的 poll 帧里只剩胖
/// 指针，不再摊住子 future 的构造临时区。
///
/// 修前基础族各臂的 poll 帧预留最高 159,744 B（SC 人工修订臂 344,064 B）；
/// 逐个装箱后降到目标量级。语义零变化：只是把「构造」挪进助手的栈帧、
/// 把 future 状态搬到堆。
#[inline(never)]
pub(crate) fn boxed_child<'a, F, Fut>(
    make: F,
) -> Pin<Box<dyn Future<Output = Fut::Output> + Send + 'a>>
where
    F: FnOnce() -> Fut + Send + 'a,
    Fut: Future + Send + 'a,
    Fut::Output: Send + 'a,
{
    Box::pin(make())
}

/// 具名臂共享入参（设计 §3.3 窄参数合同，不克隆整份 ProviderRunContext）：
/// engine guard 与单一 owned `command_rx` 按值进入当前臂；其余共享量按引用
/// （原 provider_for_run / manager_for_task / run_context_clone /
/// outbound_tx_for_task / run_label / run_token / provider_drive_guard /
/// run_cancel）。
pub(crate) struct ArmDrive<'e, 'd> {
    /// engine MutexGuard：按值进臂（见文件头三守卫合同）。`'e` 与
    /// `ArmExit<'e>` 同源——Continue 交还 guard 只携带 `'e`，不把 `'d`
    /// 借用延长到臂返回之后。
    pub(crate) engine: tokio::sync::MutexGuard<'e, WorkspaceEngine>,
    /// 单一 owned Receiver：进入当前臂，不 clone/重建第二消费者。
    pub(crate) command_rx: mpsc::Receiver<ProviderCommand>,
    /// 本 run 选定的 provider。
    pub(crate) provider: Arc<dyn StreamingProviderAdapter>,
    /// manager（原 manager_for_task）。
    pub(crate) manager: &'d Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    /// 运行上下文（原 run_context_clone）。
    pub(crate) run_context: &'d ProviderRunContext,
    /// 出站帧通道（原 outbound_tx_for_task）。
    pub(crate) outbound_tx: &'d mpsc::Sender<OutboundControl>,
    /// run 标签（原 run_label）。
    pub(crate) run_label: &'d str,
    /// run token。
    pub(crate) run_token: u64,
    /// idle 守卫：外层 Option 的可变借用（臂仅既有分支 `.take()`）。
    pub(crate) provider_drive_guard: &'d mut Option<crate::web::state::WorkspaceProviderDriveGuard>,
    /// 取消 token：安装 engine token 与按值 provider API 处才 clone。
    pub(crate) run_cancel: &'d CancellationToken,
    /// provider 注册表（followups 查 reviewer/outline/draft provider 用；
    /// S2 随 A 传入 drive，P7 followups 具名化时随之下沉）。
    pub(crate) provider_registry: &'d Arc<ProviderRegistry>,
}

/// S2 小外壳（设计 §3.1.2）：spawn 捕获面＝任务参数本身；task body 的构造
/// 表达式位于本 future **首次 poll 内**并立即装箱——深 inbound 链上只构造/
/// 搬移本外壳（≤32 KiB 门禁，见 stack_budget `raw_wrapper_is_at_most_32768_bytes`），
/// 不构造/搬移巨型 body。外壳不额外 clone 上下文/enum。
#[allow(clippy::manual_async_fn)]
// S2 冻结接口：显式 `-> impl Future + Send + 'static`
// 边界供任务 9/10 直接依赖（async fn 无法在签名声明 Send）。
#[allow(clippy::too_many_arguments)] // 与 provider_run_task_body 同源逐参数，不合并包一层
pub(crate) fn provider_run_task(
    run_kind: ProviderRunKind,
    run_context_clone: ProviderRunContext,
    engine_for_run: Arc<Mutex<WorkspaceEngine>>,
    manager_for_task: Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    provider_registry_for_run: Arc<ProviderRegistry>,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    outbound_tx_for_task: mpsc::Sender<OutboundControl>,
    cancel_arm_outbound_tx: mpsc::Sender<OutboundControl>,
    session_id: String,
    run_label: String,
    run_token: u64,
    run_cancel: CancellationToken,
    command_rx: mpsc::Receiver<ProviderCommand>,
    provider_drive_guard: crate::web::state::WorkspaceProviderDriveGuard,
    entity_run_lease: Option<crate::product::entity_operation_store::EntityRunLease>,
    cancelled_revision_turn_id: Option<String>,
) -> impl Future<Output = ()> + Send + 'static {
    async move {
        // 仅测试构造探针（S2）：async 块体只在被 poll 时执行——探针因此
        // 证明 body 构造发生在首次 poll 内（spawn 调用侧恒为 0，见
        // stack_budget `body_is_not_constructed_before_wrapper_first_poll`）。
        #[cfg(test)]
        PROVIDER_RUN_TASK_BODY_CONSTRUCTIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // 首次 poll 内构造并装箱 task body（设计 §3.1.2 显式 Box 边界）：
        // 巨型 body 状态不再内嵌外壳；构造/poll 的同步临时值由本任务
        // 线程在首 poll 时自行承担，与深 inbound 调用链解耦。
        Box::pin(provider_run_task_body(
            run_kind,
            run_context_clone,
            engine_for_run,
            manager_for_task,
            provider_registry_for_run,
            provider_for_run,
            outbound_tx_for_task,
            cancel_arm_outbound_tx,
            session_id,
            run_label,
            run_token,
            run_cancel,
            command_rx,
            provider_drive_guard,
            entity_run_lease,
            cancelled_revision_turn_id,
        ))
        .await
    }
}

/// 仅测试构造探针（S2）：`provider_run_task` 首次 poll 内 +1（隔离子进程
/// 断言，进程内计数不受其它用例并发 spawn 干扰）。
#[cfg(test)]
pub(crate) static PROVIDER_RUN_TASK_BODY_CONSTRUCTIONS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// 具名任务 body：参数即原匿名块被 move 的逐捕获（顺序对齐原绑定出现次序）。
/// S2：body 只持三守卫外层（实体 lease、idle Option）、engine 锁获取与
/// token 安装、boxed drive 唯一构造点与取消兜底参数——实际分发/followups/
/// 正常尾部在 `provider_run_drive_body`。
#[allow(clippy::too_many_arguments)] // S0 机械提取：原 spawn 块捕获逐参数化，不合并包一层
pub(crate) async fn provider_run_task_body(
    run_kind: ProviderRunKind,
    run_context_clone: ProviderRunContext,
    engine_for_run: Arc<Mutex<WorkspaceEngine>>,
    manager_for_task: Arc<crate::web::workspace_session::WorkspaceSessionManager>,
    provider_registry_for_run: Arc<ProviderRegistry>,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    outbound_tx_for_task: mpsc::Sender<OutboundControl>,
    cancel_arm_outbound_tx: mpsc::Sender<OutboundControl>,
    session_id: String,
    run_label: String,
    run_token: u64,
    run_cancel: CancellationToken,
    command_rx: mpsc::Receiver<ProviderCommand>,
    provider_drive_guard: crate::web::state::WorkspaceProviderDriveGuard,
    entity_run_lease: Option<crate::product::entity_operation_store::EntityRunLease>,
    cancelled_revision_turn_id: Option<String>,
) {
    // D2.1a：run lease 随任务全生命周期持有——drive/取消臂收口/最终
    // store writes 完成前不释放（supersede 接力 run 自行另持新 lease）。
    let _entity_run_lease = entity_run_lease;
    let mut provider_drive_guard = Some(provider_drive_guard);
    let mut engine = engine_for_run.lock().await;
    engine.use_run_token(run_cancel.clone());
    // P3（任务4）：StaleContext 重建的 `Box<ResolvedPlanningContext>` 不再在
    // 共通前奏裸 clone——run_kind 匹配 OutlineRebuild 时由 drive 分发点把
    // 原 Box 载荷按值移交 legacy 臂（设计 §3.3 rebuild_context 行）；
    // outline feedback / revision 标记同理按 variant 精确移动，不再提前复制。
    // r16 现场死锁根修(缺陷 #10 家族,授权:run 注册/取消链):驱动体可能
    // 悬停在 select 之外的裸 await(现场形态:provider 启动链/SC 臂体内
    // 链),engine MutexGuard 随任务体永驻,supersede 取消 token 后新 run
    // 的 engine.lock().await 永久排队,WS/HTTP 全冻。取消臂兜底:drive
    // 体被 drop 时 guard 随之释放,run 注册由 finish_run 幂等回收(非本人
    // token 自动复位,不误伤接替 run)。主 select 臂的既有取消观察
    //(provider_drive.rs cancel.cancelled)保持不变——本臂覆盖其盲区。
    let supersede_reclaim_manager = manager_for_task.clone();
    // S2 boxed drive 边界（设计 §3.1.3）+ 治本（Task 14b）：engine MutexGuard
    // 按值随 A 进入 drive；drive 整体只以 `Pin<Box<_>>` 指针进入 settle
    // helper 的 pin/select 作用域——helper 不再把完整 drive 状态机放回栈上。
    // 且 drive future 的**构造**发生在具名构造函数自己的栈帧内（随即装箱到
    // 堆）——本 body 的 poll 帧不再承载 drive 状态机的构造临时区。
    let drive = make_provider_run_drive_body(
        ArmDrive {
            engine,
            command_rx,
            provider: provider_for_run,
            manager: &manager_for_task,
            provider_registry: &provider_registry_for_run,
            run_context: &run_context_clone,
            outbound_tx: &outbound_tx_for_task,
            run_label: &run_label,
            run_token,
            provider_drive_guard: &mut provider_drive_guard,
            run_cancel: &run_cancel,
        },
        run_kind,
    );
    // 取消观察与有界收口窗口（含 drive 被 drop 的原兜底）见 cancel_settle.inc.rs。
    drive_until_cancelled_settles(
        drive,
        &run_cancel,
        cancelled_revision_turn_id,
        &engine_for_run,
        &cancel_arm_outbound_tx,
        &session_id,
        &run_label,
        &supersede_reclaim_manager,
        run_token,
    )
    .await;
}

/// 具名 drive body（S2，设计 §3.1.3/§3.1.4）：A（`ArmDrive`）按值进入，
/// engine MutexGuard 按值随 A 按值属于本 drive——取消强制 drop 时连同释放且
/// **先于**兜底重取锁（三守卫合同见文件头）。`run_kind` 移动进本函数后
/// 只分发到实际选中的臂：Continue 才 followups+正常尾部，Stop 立即返回。
/// 唯一构造点在 `provider_run_task_body`（boxed）。
///
/// 治本（Task 14b，栈根修）：本函数**只分发具名构造函数**——各臂、PlanRevision
/// 臂与 followups 均为「具名构造函数 + `Pin<Box<dyn Future>>`」：构造发生在
/// 构造函数自己的栈帧内、随即装箱到堆，本函数的 poll 帧里只剩胖指针，不再
/// 搬运任何臂/宏体的构造临时区（修前本函数 poll 帧序言预留 724,992 B）。
pub(crate) async fn provider_run_drive_body<'e, 'd>(
    drive: ArmDrive<'e, 'd>,
    run_kind: ProviderRunKind,
) where
    'e: 'd,
{
    // S2：A 展开为原 drive 块的逐字段绑定（机械改名：引用型字段即原匿名
    // 块的借用捕获；engine guard 与 command_rx 按值。具名臂构造从本绑定
    // 取字段，宏臂继续直接引用同名绑定）。
    // P2/P3/P4 + 治本（Task 14b）：基础族四臂、SC author 臂、legacy outline
    // 三族、draft/batch 两族、PlanRevision 臂与 followups 全部经**具名构造函数**
    // 的胖指针分发（只为实际选中的分支构造；构造临时区不落在本帧）。
    let ArmDrive {
        mut engine,
        command_rx,
        provider: provider_for_run,
        manager: manager_for_task,
        provider_registry: provider_registry_for_run,
        run_context: run_context_clone,
        outbound_tx: outbound_tx_for_task,
        run_label,
        run_token,
        provider_drive_guard,
        run_cancel,
    } = drive;
    match run_kind {
        // S2 分发（原 S0 机械提取）：engine MutexGuard 按值进当前臂
        // （ArmDrive），Continue 交还、Stop 即 drive 级 return（见
        // ArmExit 合同）。
        ProviderRunKind::Author { content } => {
            // 治本（Task 14b）：真实 Box 间接边界（设计 §3.1.5）——臂的构造
            // 临时区落在臂构造函数的栈帧内，本 drive 帧只留胖指针。
            engine = match drive_author_arm(
                ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: manager_for_task,
                    provider_registry: provider_registry_for_run,
                    run_context: run_context_clone,
                    outbound_tx: outbound_tx_for_task,
                    run_label,
                    run_token,
                    provider_drive_guard: &mut *provider_drive_guard,
                    run_cancel,
                },
                content,
            )
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::AuthorChoiceFollowup { content } => {
            engine = match drive_author_choice_followup_arm(
                ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: manager_for_task,
                    provider_registry: provider_registry_for_run,
                    run_context: run_context_clone,
                    outbound_tx: outbound_tx_for_task,
                    run_label,
                    run_token,
                    provider_drive_guard: &mut *provider_drive_guard,
                    run_cancel,
                },
                content,
            )
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::Revision => {
            engine = match drive_revision_arm(ArmDrive {
                engine,
                command_rx,
                provider: provider_for_run.clone(),
                manager: manager_for_task,
                provider_registry: provider_registry_for_run,
                run_context: run_context_clone,
                outbound_tx: outbound_tx_for_task,
                run_label,
                run_token,
                provider_drive_guard: &mut *provider_drive_guard,
                run_cancel,
            })
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::ReviewOnly => {
            engine = match drive_review_only_arm(ArmDrive {
                engine,
                command_rx,
                provider: provider_for_run.clone(),
                manager: manager_for_task,
                provider_registry: provider_registry_for_run,
                run_context: run_context_clone,
                outbound_tx: outbound_tx_for_task,
                run_label,
                run_token,
                provider_drive_guard: &mut *provider_drive_guard,
                run_cancel,
            })
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        // P3（任务4）：legacy outline 三族经具名构造函数的胖指针分发；
        // 三 variant 移入精确载荷（Legacy=(None,false,None)、
        // Revision=(feedback,true,None)、Rebuild=(None,false,Some(rebuilt))，
        // 设计 §3.3——不在共通前奏 clone rebuilt/feedback）。
        ProviderRunKind::WorkItemPlanLegacyAuthor => {
            engine = match drive_legacy_outline_arm(
                ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: manager_for_task,
                    provider_registry: provider_registry_for_run,
                    run_context: run_context_clone,
                    outbound_tx: outbound_tx_for_task,
                    run_label,
                    run_token,
                    provider_drive_guard: &mut *provider_drive_guard,
                    run_cancel,
                },
                None,
                false,
                None,
            )
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanOutlineRevision { feedback } => {
            engine = match drive_legacy_outline_arm(
                ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: manager_for_task,
                    provider_registry: provider_registry_for_run,
                    run_context: run_context_clone,
                    outbound_tx: outbound_tx_for_task,
                    run_label,
                    run_token,
                    provider_drive_guard: &mut *provider_drive_guard,
                    run_cancel,
                },
                feedback,
                true,
                None,
            )
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanOutlineRebuild { rebuilt } => {
            engine = match drive_legacy_outline_arm(
                ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: manager_for_task,
                    provider_registry: provider_registry_for_run,
                    run_context: run_context_clone,
                    outbound_tx: outbound_tx_for_task,
                    run_label,
                    run_token,
                    provider_drive_guard: &mut *provider_drive_guard,
                    run_cancel,
                },
                None,
                false,
                Some(rebuilt),
            )
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor => {
            engine = match drive_single_candidate_author_arm(ArmDrive {
                engine,
                command_rx,
                provider: provider_for_run.clone(),
                manager: manager_for_task,
                provider_registry: provider_registry_for_run,
                run_context: run_context_clone,
                outbound_tx: outbound_tx_for_task,
                run_label,
                run_token,
                provider_drive_guard: &mut *provider_drive_guard,
                run_cancel,
            })
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanDraft { feedback } => {
            // P4（任务5）+ 治本（Task 14b）真实 Box 间接边界（设计 §3.1.5）：
            // 只为实际选中的臂分配，drive 状态机不再内嵌完整臂状态。fresh
            // 冻结载荷/RetryOnce 循环/每轮新 split handle/AwaitConfirmation
            // 的 break 语义随臂整体迁移（plan_draft.rs，机械迁移零差异）。
            engine = match drive_plan_draft_arm(
                ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: manager_for_task,
                    provider_registry: provider_registry_for_run,
                    run_context: run_context_clone,
                    outbound_tx: outbound_tx_for_task,
                    run_label,
                    run_token,
                    provider_drive_guard: &mut *provider_drive_guard,
                    run_cancel,
                },
                feedback,
            )
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanBatch => {
            // P4（任务5）+ 治本（Task 14b）真实 Box 间接边界（设计 §3.1.5）：
            // batch fresh 不另建 batch/不重置 repair 预算；AwaitConfirmation
            // 仍 feedback=None 继续原 loop——与 draft 的 break 差异不统一
            // （plan_batch.rs）。
            engine = match drive_plan_batch_arm(ArmDrive {
                engine,
                command_rx,
                provider: provider_for_run.clone(),
                manager: manager_for_task,
                provider_registry: provider_registry_for_run,
                run_context: run_context_clone,
                outbound_tx: outbound_tx_for_task,
                run_label,
                run_token,
                provider_drive_guard: &mut *provider_drive_guard,
                run_cancel,
            })
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanRevision { feedback } => {
            // 治本（Task 14b）：PlanRevision 具名臂（宏体构造临时区落在臂
            // 构造函数的栈帧内 → 装箱到堆；本 drive 帧只留胖指针）。
            // 宏体恒以 drive 级 `return` 收口（尾是无 break 的 loop）⇒ 等价
            // ArmExit::Stop：父 drive 不再走 followups、不补一次通用 finish、
            // 不补错误帧；engine guard 随臂作用域结束释放（其间无 await）。
            drive_plan_revision_arm(
                ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: manager_for_task,
                    provider_registry: provider_registry_for_run,
                    run_context: run_context_clone,
                    outbound_tx: outbound_tx_for_task,
                    run_label,
                    run_token,
                    provider_drive_guard: &mut *provider_drive_guard,
                    run_cancel,
                },
                feedback,
            )
            .await;
            return;
        }
        ProviderRunKind::HumanGateScManualRevision { turn_id, prompt } => {
            engine = match drive_human_gate_sc_revision_arm(
                ArmDrive {
                    engine,
                    command_rx,
                    provider: provider_for_run.clone(),
                    manager: manager_for_task,
                    provider_registry: provider_registry_for_run,
                    run_context: run_context_clone,
                    outbound_tx: outbound_tx_for_task,
                    run_label,
                    run_token,
                    provider_drive_guard: &mut *provider_drive_guard,
                    run_cancel,
                },
                turn_id,
                prompt,
            )
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
    }
    // 治本（Task 14b）：followups 具名构造函数——宏体构造临时区落在构造函数
    // 自己的栈帧内 → 装箱到堆；本 drive 帧只留胖指针。宏内提前 `return` =
    // 原 drive 级 return：父层不补一次通用 finish、不补错误帧（宏内各错误
    // 路径自身已按原顺序完成收口）。
    let mut followups_engine: Option<tokio::sync::MutexGuard<'e, WorkspaceEngine>> = None;
    drive_provider_run_followups(
        engine,
        provider_registry_for_run,
        manager_for_task,
        run_token,
        run_label,
        outbound_tx_for_task,
        run_cancel,
        run_context_clone,
        &mut *provider_drive_guard,
        &mut followups_engine,
    )
    .await;
    let Some(mut engine) = followups_engine else {
        return;
    };
    engine.mark_active_run_finished(run_label);
    drop(engine);
    drop(provider_drive_guard.take());
    manager_for_task.finish_run(run_token).await;
}

/// 具名 drive body 的**装箱构造函数**（治本 Task 14b）：drive future 的构造
/// 发生在本函数自己的栈帧内、随即装箱到堆——调用方（`provider_run_task_body`）
/// 的 poll 帧不再承载 drive 状态机的构造临时区（修前该帧预留 114,688 B）。
pub(crate) fn make_provider_run_drive_body<'e, 'd>(
    drive: ArmDrive<'e, 'd>,
    run_kind: ProviderRunKind,
) -> Pin<Box<dyn Future<Output = ()> + Send + 'd>>
where
    'e: 'd,
{
    Box::pin(provider_run_drive_body(drive, run_kind))
}
