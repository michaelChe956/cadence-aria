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
//!    尾部，Stop 立即返回）。P3（任务4）起 legacy outline 三族拆入
//!    `legacy_outline.rs` 具名臂；draft/batch/revision 仍留本体（宏原样），
//!    后续任务逐族迁移后再按族冻结体积门禁。
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
use super::sc_manual_revision::drive_human_gate_sc_revision_arm;
use super::single_candidate_arm::drive_single_candidate_author_arm;

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
    // S2 boxed drive 边界（设计 §3.1.3）：engine MutexGuard 按值随 A 进入
    // drive；drive 整体只以 `Pin<Box<_>>` 指针进入 settle helper 的
    // pin/select 作用域——helper 不再把完整 drive 状态机放回栈上。
    let drive = Box::pin(provider_run_drive_body(
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
    ));
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
/// engine MutexGuard 随 A 按值属于本 drive——取消强制 drop 时连同释放且
/// **先于**兜底重取锁（三守卫合同见文件头）。`run_kind` 移动进本函数后
/// 只分发到实际选中的臂：Continue 才 followups+正常尾部，Stop 立即返回。
/// 唯一构造点在 `provider_run_task_body`（boxed）。P2（任务3）起基础族
/// 四臂与 SC author 臂、P3（任务4）起 legacy outline 三族均以 `Box::pin`
/// 间接边界分发；尚未拆族的 draft/batch/revision 仍以原语句留在本体内。
pub(crate) async fn provider_run_drive_body<'e, 'd>(
    drive: ArmDrive<'e, 'd>,
    run_kind: ProviderRunKind,
) {
    // S2：A 展开为原 drive 块的逐字段绑定（机械改名：引用型字段即原匿名
    // 块的借用捕获；engine guard 与 command_rx 按值。具名臂构造从本绑定
    // 取字段，宏臂继续直接引用同名绑定）。
    // P2/P3：基础族四臂、SC author 臂与 legacy outline 三族经
    // `Box::pin(...).await` 真实间接边界分发（只为实际选中的臂分配）；
    // 尚未拆族的 draft/batch/revision 仍以原语句留在本体内。
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
            // P2（任务3）真实 Box 间接边界（设计 §3.1.5）：只为实际选中的臂
            // 分配，drive 状态机不再内嵌完整臂状态。
            engine = match Box::pin(drive_author_arm(
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
            ))
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::AuthorChoiceFollowup { content } => {
            engine = match Box::pin(drive_author_choice_followup_arm(
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
            ))
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::Revision => {
            engine = match Box::pin(drive_revision_arm(ArmDrive {
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
            }))
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::ReviewOnly => {
            engine = match Box::pin(drive_review_only_arm(ArmDrive {
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
            }))
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        // P3（任务4）：legacy outline 三族经 `Box::pin(...).await` 真实间接
        // 边界分发；三 variant 移入精确载荷（Legacy=(None,false,None)、
        // Revision=(feedback,true,None)、Rebuild=(None,false,Some(rebuilt))，
        // 设计 §3.3——不在共通前奏 clone rebuilt/feedback）。
        ProviderRunKind::WorkItemPlanLegacyAuthor => {
            engine = match Box::pin(drive_legacy_outline_arm(
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
            ))
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanOutlineRevision { feedback } => {
            engine = match Box::pin(drive_legacy_outline_arm(
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
            ))
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanOutlineRebuild { rebuilt } => {
            engine = match Box::pin(drive_legacy_outline_arm(
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
            ))
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor => {
            engine = match Box::pin(drive_single_candidate_author_arm(ArmDrive {
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
            }))
            .await
            {
                ArmExit::Continue(engine) => engine,
                ArmExit::Stop => return,
            };
        }
        ProviderRunKind::WorkItemPlanDraft { feedback } => {
            let mut command_rx = command_rx;
            // B2b3（Task 5.4，设计 §5.3 legacy_draft 行）：显式
            // fresh 接管——feedback 消费 durable 冻结 payload（take
            // 前；不再借内存 pending），stale 收口后活动 draft
            // provider 节点可重建（业务 generation 不重建），构建走
            // explicit fresh builder（resume=None，不回捞旧 native）。
            let mut draft_fresh_payload: Option<Option<String>> = None;
            let draft_fresh_outline = match lc_fresh_recovery_record_if_running(
                &run_context_clone.app_paths,
                &engine.session().session_id,
            ) {
                Ok(Some(recovery)) => match &recovery.run_context {
                    crate::product::models::LcRecoveryRunContext::LegacyDraft {
                        pending_feedback,
                        active_outline_id,
                        ..
                    } => {
                        draft_fresh_payload = Some(pending_feedback.clone());
                        Some(active_outline_id.clone())
                    }
                    _ => None,
                },
                Ok(None) => None,
                Err(message) => {
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                    return;
                }
            };
            let is_draft_fresh = draft_fresh_payload.is_some();
            let mut feedback = match draft_fresh_payload {
                Some(payload) => payload,
                None => feedback.or_else(|| engine.pending_revision_context.clone()),
            };
            if let Some(fresh_outline_id) = &draft_fresh_outline
                && engine.active_node_type()
                    != Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemDraftRun)
                && let Err(message) = engine
                    .start_serial_work_item_draft_run_for(fresh_outline_id)
                    .await
            {
                engine.mark_active_run_finished(run_label);
                drop(engine);
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                return;
            }
            while engine.active_node_type()
                == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemDraftRun)
            {
                let Some(node_id) = engine.active_timeline_node_id() else {
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error {
                        message: "work item draft run node unavailable".to_string(),
                    };
                    let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                    return;
                };
                let plan_launch = match resolve_plan_author_launch(&engine) {
                    Ok(launch) => launch,
                    Err(error) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("logical plan launch failed: {error}"),
                        };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let routing_context = plan_launch.routing_context();
                // B2b3：fresh 全程走 explicit fresh builder（RetryOnce
                // 循环内同纪律——每轮 native 启动都不回捞旧会话）。
                let provider_input = if is_draft_fresh {
                    engine.build_current_work_item_draft_streaming_input_explicit_fresh(
                        feedback.as_deref(),
                        &routing_context,
                    )
                } else {
                    engine.build_current_work_item_draft_streaming_input(
                        feedback.as_deref(),
                        &routing_context,
                    )
                };
                let provider_input = match provider_input {
                    Ok(input) => input,
                    Err(message) => {
                        engine
                            .finish_active_run_with_failed_node(message.clone())
                            .await;
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let author_provider = engine.session().author_provider.clone();
                engine
                    .emit_provider_prompt_event(
                        &node_id,
                        provider_input.prompt.clone(),
                        if feedback.is_some() {
                            "发送给 WorkItemDraft provider 的增量返修提示词"
                        } else {
                            "发送给 WorkItemDraft provider 的完整提示词"
                        },
                        Some(author_provider.clone()),
                    )
                    .await;
                let provider_input = engine.attach_tool_policy_audit(provider_input);
                // Task 1b 段②生产臂:RetryOnce 循环每轮 begin 新 handle。
                let draft_prompt = provider_input.prompt.clone();
                let plan_split_run = match begin_plan_split_run_if_logical(
                    &plan_launch,
                    &crate::product::lifecycle_store::LifecycleStore::new(
                        run_context_clone.app_paths.clone(),
                    ),
                    &run_context_clone.session_record.project_id,
                    &run_context_clone.session_record.issue_id,
                    &author_provider,
                    &engine.session().session_id,
                ) {
                    Ok(run) => run,
                    Err(message) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                // T6 fix-loop r2：trust 分类已下沉
                // start_work_item_plan_author 的 Logical 分支共享口。
                let provider_session = start_work_item_plan_author(
                    plan_launch,
                    provider_for_run.clone(),
                    provider_input,
                    run_cancel.clone(),
                    plan_split_run.as_ref(),
                )
                .await;
                let full_output = match engine
                    .drive_work_item_plan_provider_session_to_output(
                        provider_session,
                        &mut command_rx,
                        node_id,
                        author_provider,
                    )
                    .await
                {
                    Ok(output) => output,
                    Err(message) => {
                        fail_plan_split_run(
                            plan_split_run.as_ref(),
                            &format!("provider session drive failed: {message}"),
                        );
                        engine.mark_active_run_finished(run_label);
                        return;
                    }
                };
                let structured_output = match parse_work_item_split_structured_output(&full_output)
                {
                    Ok(output) => output,
                    Err(message) => {
                        fail_plan_split_run(
                            plan_split_run.as_ref(),
                            &format!("structured output parse failed: {message}"),
                        );
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("work item draft generate failed: {message}"),
                        };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                close_plan_split_run(plan_split_run.as_ref(), &draft_prompt, &full_output);
                let candidate = match parse_work_item_draft_output(structured_output) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("work item draft parse failed: {}", error.message),
                        };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                match engine
                    .complete_work_item_draft_author(candidate, feedback.as_deref())
                    .await
                {
                    Ok(WorkItemDraftAuthorOutcome::RetryOnce {
                        feedback: repair_feedback,
                        ..
                    }) => feedback = Some(repair_feedback),
                    Ok(WorkItemDraftAuthorOutcome::AwaitConfirmation) => break,
                    Err(message) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                }
            }
        }
        ProviderRunKind::WorkItemPlanBatch => {
            let mut command_rx = command_rx;
            // B2b3（Task 5.4，设计 §5.3 legacy_batch 行）：显式
            // fresh 接管——feedback 消费 durable 冻结 payload（take
            // 前；不借内存 pending），stale 收口后活动 batch provider
            // 节点可重建（不另建 batch、不重置 repair 预算，同 batch
            // 完成器收口），构建走 explicit fresh builder（resume=None）。
            let mut batch_fresh_payload: Option<Option<String>> = None;
            match lc_fresh_recovery_record_if_running(
                &run_context_clone.app_paths,
                &engine.session().session_id,
            ) {
                Ok(Some(recovery)) => {
                    if let crate::product::models::LcRecoveryRunContext::LegacyBatch {
                        pending_feedback,
                        ..
                    } = &recovery.run_context
                    {
                        batch_fresh_payload = Some(pending_feedback.clone());
                    }
                }
                Ok(None) => {}
                Err(message) => {
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error { message };
                    let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                    return;
                }
            }
            let is_batch_fresh = batch_fresh_payload.is_some();
            let mut feedback = match batch_fresh_payload {
                Some(payload) => payload,
                None => engine.pending_revision_context.clone(),
            };
            if is_batch_fresh
                && engine.active_node_type()
                    != Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemBatchRun)
            {
                engine.begin_work_item_batch_fresh_recovery_run().await;
            }
            while engine.active_node_type()
                == Some(crate::web::workspace_ws_types::TimelineNodeType::WorkItemBatchRun)
            {
                let Some(node_id) = engine.active_timeline_node_id() else {
                    engine.mark_active_run_finished(run_label);
                    drop(engine);
                    let err = WsOutMessage::Error {
                        message: "work item batch run node unavailable".to_string(),
                    };
                    let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                    return;
                };
                let plan_launch = match resolve_plan_author_launch(&engine) {
                    Ok(launch) => launch,
                    Err(error) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("logical plan launch failed: {error}"),
                        };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let routing_context = plan_launch.routing_context();
                // B2b3：fresh 全程走 explicit fresh builder（循环内
                // 每轮 native 启动都不回捞旧会话，同完整门）。
                let provider_input = if is_batch_fresh {
                    engine.build_current_work_item_batch_draft_streaming_input_explicit_fresh(
                        feedback.as_deref(),
                        &routing_context,
                    )
                } else {
                    engine.build_current_work_item_batch_draft_streaming_input(
                        feedback.as_deref(),
                        &routing_context,
                    )
                };
                let provider_input = match provider_input {
                    Ok(input) => input,
                    Err(message) => {
                        engine
                            .finish_active_run_with_failed_node(message.clone())
                            .await;
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                let author_provider = engine.session().author_provider.clone();
                engine
                    .emit_provider_prompt_event(
                        &node_id,
                        provider_input.prompt.clone(),
                        if feedback.is_some() {
                            "发送给 WorkItemBatch provider 的本地校验修复提示词"
                        } else {
                            "发送给 WorkItemBatch provider 的完整提示词"
                        },
                        Some(author_provider.clone()),
                    )
                    .await;
                let provider_input = engine.attach_tool_policy_audit(provider_input);
                // Task 1b 段②生产臂:RetryOnce 循环每轮 begin 新 handle。
                let batch_prompt = provider_input.prompt.clone();
                let plan_split_run = match begin_plan_split_run_if_logical(
                    &plan_launch,
                    &crate::product::lifecycle_store::LifecycleStore::new(
                        run_context_clone.app_paths.clone(),
                    ),
                    &run_context_clone.session_record.project_id,
                    &run_context_clone.session_record.issue_id,
                    &author_provider,
                    &engine.session().session_id,
                ) {
                    Ok(run) => run,
                    Err(message) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                // T6 fix-loop r2：trust 分类已下沉
                // start_work_item_plan_author 的 Logical 分支共享口。
                let provider_session = start_work_item_plan_author(
                    plan_launch,
                    provider_for_run.clone(),
                    provider_input,
                    run_cancel.clone(),
                    plan_split_run.as_ref(),
                )
                .await;
                let full_output = match engine
                    .drive_work_item_plan_provider_session_to_output(
                        provider_session,
                        &mut command_rx,
                        node_id,
                        author_provider,
                    )
                    .await
                {
                    Ok(output) => output,
                    Err(message) => {
                        fail_plan_split_run(
                            plan_split_run.as_ref(),
                            &format!("provider session drive failed: {message}"),
                        );
                        engine.mark_active_run_finished(run_label);
                        return;
                    }
                };
                let structured_output = match parse_work_item_split_structured_output(&full_output)
                {
                    Ok(output) => output,
                    Err(message) => {
                        fail_plan_split_run(
                            plan_split_run.as_ref(),
                            &format!("structured output parse failed: {message}"),
                        );
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!("work item batch draft generate failed: {message}"),
                        };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                close_plan_split_run(plan_split_run.as_ref(), &batch_prompt, &full_output);
                let candidate = match parse_work_item_draft_output(structured_output) {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error {
                            message: format!(
                                "work item batch draft parse failed: {}",
                                error.message
                            ),
                        };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                };
                match engine
                    .complete_work_item_batch_draft_author(candidate, feedback.as_deref())
                    .await
                {
                    Ok(WorkItemDraftAuthorOutcome::RetryOnce {
                        feedback: repair_feedback,
                        ..
                    }) => feedback = Some(repair_feedback),
                    Ok(WorkItemDraftAuthorOutcome::AwaitConfirmation) => feedback = None,
                    Err(message) => {
                        engine.mark_active_run_finished(run_label);
                        drop(engine);
                        let err = WsOutMessage::Error { message };
                        let _ = send_json_outbound(outbound_tx_for_task, &err).await;
                        return;
                    }
                }
            }
        }
        ProviderRunKind::WorkItemPlanRevision { feedback } => {
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
    engine.mark_active_run_finished(run_label);
    drop(engine);
    drop(provider_drive_guard.take());
    manager_for_task.finish_run(run_token).await;
}
