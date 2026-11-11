use super::*;
use crate::product::workspace_engine::PlanAuthorOutputContract;
use crate::product::workspace_engine::SingleCandidateAuthorCompletion;
use crate::product::workspace_engine::draft_batch::member_checkout_prefixes;

pub(crate) enum SingleCandidateProviderRunOutcome {
    Completed,
}

// 停等臂携带完整预检/信任材料;按值传递避免全链路 Box 化。
#[allow(clippy::large_enum_variant)]
pub(crate) enum SingleCandidateProviderRunError {
    AlreadyFinished,
    /// 败者让位（k3 P2 / RCA 修法 3）：本次启动的 ledger 键已被健康持有者
    /// 领走（在途 run 或已完成的会话）。迟到 run 必须静默退场——不落 failed
    /// 节点、不广播 Error、不改写 durable phase。
    Superseded,
    Message(String),
    /// C-1：LC admission waiting——成员规则缺失/政策漂移是可操作的持久等待
    /// 事实（durable phase 已回落 Prepare 面），不是终态失败。携带预检的
    /// 判别码、缺失材料与允许动作供上浮投影；运行以无 failed 节点收尾。
    AdmissionWaiting {
        reason_code: String,
        detail: String,
        missing_materials: Vec<String>,
        allowed_actions: Vec<crate::product::logical_codebase::BootstrapActionKind>,
        /// 6.1（方案 A 附带）：trust 缺失（reason_code ==
        /// `PROVIDER_TRUST_MISSING_WAITING_REASON`）时的同源只读人工登记
        /// 指导——真实 adapter 工件路径/trust 键/人工形状＋生成场景
        /// retry_action；其它 waiting 恒 `None`（原投影不变）。
        manual_guidance: Option<String>,
        /// 指引卡简化（owner 2026-10-12 直批）：trust 缺失时的同源只读
        /// 等待投影原样随行——供上层紧随 error 帧发送结构化
        /// `trust_missing_guidance` 帧（providers/canonical_root/
        /// command_markdown），前端不再解析平铺串；其它 waiting 恒 None。
        /// Box 化：`ProviderTrustWaiting` 本体 ≥320 字节，不 Box 会让整个
        /// 错误枚举过大（clippy::result_large_err ⇒ 每个返回该错误的函数都
        /// 报红）。等待载荷只做「原样随行」的只读投影，Box 不改语义。
        trust_waiting:
            Option<Box<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting>>,
    },
    /// B2c（Task 5.5，设计 §5.3 SC 纪律①）：SC author native mismatch
    /// （旧 native 在而冻结绑定不可用/已吊销）的独立 typed 等待——不落
    /// Message/AlreadyFinished 的 generic Failed，不 rearm。durable 已由
    /// `persist_single_candidate_author_superseded_wait` 收口为 Waiting
    /// recovery（原 reservation key/phase 保留，ledger 不追加），等待
    /// 显式 fresh（sc_author → LcFreshContext）接管。
    NativeSupersededWaiting {
        reason_code: String,
        superseded_native_session_id: Option<String>,
        recovery_id: String,
    },
}

/// B2c（Task 5.5，设计 §5.3 sc_author 行 / plan 接口字典 recovery entry
/// mode）：`run_single_candidate_author` 的重入形态。
pub(crate) enum SingleCandidateAuthorRecoveryEntry {
    /// 常规入口：native mismatch precheck 先于新业务 reserve
    /// （FrozenBindingUnavailable → typed `NativeSupersededWaiting`，
    /// 不 reserve 第二次、不 rearm）。
    Ordinary,
    /// sc_author 显式 fresh 重入（FreshRunning recovery）：绕过
    /// reserve/rearm——同 logical reservation 重入不 reserve 第二次；
    /// `resume=None` 全新物理 native；verdict 在场时走 fresh 返修 prompt
    /// （按 payload source ref 装载上一版 source 全文，自足不依赖旧会话）。
    LcFreshContext { source_revision_ref: Option<String> },
}

/// SC author 终态失败消息的显式重开指引后缀：Failed 相位有显式恢复入口
/// （`start_generation` 对 Failed 会话 re-arm），终态消息不能只留诊断原文，
/// 必须告诉用户下一步怎么做。措辞与 run_single_candidate_author 内
/// 「已终态失败……请显式重新开始生成」恢复入口消息同族。
const SINGLE_CANDIDATE_TERMINAL_REOPEN_GUIDANCE: &str =
    "；SingleCandidate session 已终态失败，请显式重新开始生成";

/// F2-B/3.6：教学重驱共用驱动——同 node 再驱一次 provider，返回归一化后的
/// markdown source。驱动中断（会话失败/取消）时已按既有终态形态持久化 Failed
/// 并返回 AlreadyFinished，与首轮驱动失败处理一致。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 上述枚举是运行收尾的唯一 typed 出口。
pub(super) async fn drive_single_candidate_reredrive(
    engine: &mut WorkspaceEngine,
    launch: &crate::web::workspace_ws_handler::run::gateway_start::PlanAuthorLaunch,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    node_id: &str,
    author_provider: &crate::product::models::ProviderName,
    reredrive_prompt: &str,
    repository_path: &std::path::Path,
    // Task 1b 段②生产臂:重驱是独立 provider run,调用方每轮 begin 新
    // handle 后传入。
    plan_split_run: Option<&crate::web::workspace_ws_handler::run::PlanSplitRunContext>,
) -> Result<String, SingleCandidateProviderRunError> {
    let reredrive_input = engine
        .build_work_item_plan_streaming_input(
            crate::product::work_item_split_engine::types::provider_name_to_type(author_provider),
            reredrive_prompt.to_string(),
            repository_path.to_string_lossy().to_string(),
            author_provider.clone(),
            PlanAuthorOutputContract::Structured,
        )
        .map_err(SingleCandidateProviderRunError::Message)?;
    let reredrive_input = engine.attach_tool_policy_audit(reredrive_input);
    // T6 fix-loop r2：trust 分类已下沉 start_work_item_plan_author 的
    // Logical 分支共享口（教学重驱同臂）。
    let reredrive_session = start_work_item_plan_author(
        launch.clone(),
        Arc::clone(&provider_for_run),
        reredrive_input,
        run_cancel.clone(),
        plan_split_run,
    )
    .await;
    let reredrive_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            reredrive_session,
            command_rx,
            node_id.to_string(),
            author_provider.clone(),
        )
        .await
    {
        Ok(output) => output,
        Err(message) => {
            crate::web::workspace_ws_handler::run::fail_plan_split_run(
                plan_split_run,
                &format!("provider session drive failed: {message}"),
            );
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::AlreadyFinished);
        }
    };
    crate::web::workspace_ws_handler::run::close_plan_split_run_with_markdown(
        plan_split_run,
        reredrive_prompt,
        &reredrive_output,
    );
    let reredrive_delivery = prepare_author_delivery_for_compile(&reredrive_output);
    emit_author_heading_normalized_event(
        engine,
        node_id,
        &reredrive_output,
        &reredrive_delivery,
        author_provider,
    )
    .await;
    emit_author_ears_normalized_event(
        engine,
        node_id,
        &reredrive_output,
        &reredrive_delivery,
        author_provider,
    )
    .await;
    Ok(reredrive_delivery.source)
}

/// 公共 admission 预检「成员语言规则缺失」等待项的判别码（路 C，Task 4）：
/// SC 消费链对成员文件的必需性判定收敛到实际消费点——该等待项在 SC 侧不
/// 直接阻断（其他 active 成员缺失不阻断 SC），由消费点对已解析
/// `repository.path` 的同一份读取重判（读取失败才停等）。
const PUBLIC_MEMBER_RULES_WAITING_REASON: &str = "member_language_rules_missing";

/// 6.1（方案 A 附带，REQ-REG-15「新根首次生成带指导但不自动登记」）：
/// admission 预检「provider trust 缺失」等待项判别码（spawn 复验 trust
/// 维度拒绝的同源只读分类）。该 reason 的 generation waiting 桥接同源
/// 只读 manual_guidance；其它 waiting 原样上浮。
const PROVIDER_TRUST_MISSING_WAITING_REASON: &str = "provider_trust_missing";

/// C-1/路 C（Task 4，REQ-REG-12）：LC 会话的 admission 预检——在读取
/// `.claude/rules/language.md` 之前经 `LogicalCodebaseProviderAdmissionPreflight`
/// 校验聚合 policy、发布链与 gateway capability 等公共准入条件。
///
/// - waiting（根规则/发布链/capability/cwd 漂移等公共门）：durable phase 回落
///   Prepare/Open 等待面，返回 `AdmissionWaiting`（不落终态 Failed、
///   不产生 failed 节点）；
/// - 成员语言规则等待（`member_language_rules_missing`）：路 C 起 SC 不直接
///   阻断——SC 必需成员材料只由实际消费点对已解析 `repository.path` 的
///   同一份读取判定（其他成员缺失仅是审计事实，不产生成员准备动作，
///   不枚举全部 active 成员作 SC 文件门）；
/// - 存储/路由错误：与其它 pre-run 装载失败同形（`Message`，不改写
///   durable phase）；
/// - LC 归属读取失败（含老记录缺归属——读取层显式报
///   `issue_not_bound_to_logical_codebase`）：持久状态不一致，fail-closed
///   `Message`，绝不静默跳过预检回落 legacy 行为。
///
/// 预检通过返回 `Ok(())`：后续仍由调用方读取同一实际文件（无 fallback）。
#[allow(clippy::result_large_err)] // 同上:运行收尾 typed 出口。
/// D5 事前基线树装载面（拆出以守住单文件行数红线）。
mod single_candidate_baseline_tree;
mod single_candidate_compile_preflight;
mod single_candidate_delivery;

use single_candidate_compile_preflight::{
    build_work_item_plan_ir_reredrive_prompt, emit_author_ears_normalized_event,
    emit_author_heading_normalized_event, format_compile_failure_reasons,
    is_teachable_parse_failure, prepare_author_delivery_for_compile, prevalidate_plan_candidate_ir,
};

fn preflight_lc_member_rules_before_author_run(
    engine: &mut WorkspaceEngine,
    run_context: &ProviderRunContext,
    launch: &super::gateway_start::LogicalPlanLaunch,
) -> Result<(), Box<SingleCandidateProviderRunError>> {
    // 错误载荷 Box 化：clippy::result_large_err（该枚举 ≥128 字节）。`?` 经
    // `From<E> for Box<E>` 自动装箱，调用点用 `map_err(|e| *e)` 解箱。
    let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        &run_context.app_paths,
        &run_context.session_record.project_id,
        &run_context.session_record.issue_id,
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!(
            "resolve logical codebase for admission preflight failed: {error}"
        ))
    })?;
    let request = launch.planning_request().map_err(|error| {
        SingleCandidateProviderRunError::Message(format!(
            "build admission preflight request failed: {error}"
        ))
    })?;
    let preflight =
        crate::product::logical_codebase::LogicalCodebaseProviderAdmissionPreflight::new(
            run_context.app_paths.clone(),
            lc_id,
            launch.gateway.clone(),
        );
    // Task 1.2：LC 作者会话是普通 session（根规则必须存在）——Normal 相位；
    // AggregateBootstrap 仅由聚合 root recipe 的凭据流程使用。
    match preflight.check(&request, &crate::product::logical_codebase::provider_admission_preflight::ProviderAdmissionPhase::Normal) {
        Ok(_) => Ok(()),
        Err(
            crate::product::logical_codebase::ProviderAdmissionError::Waiting {
                reason_code, ..
            },
        ) if reason_code == PUBLIC_MEMBER_RULES_WAITING_REASON => {
            // 路 C（Task 4，REQ-REG-12）：公共预检的成员语言规则事实不直接
            // 阻断 SC——必需成员材料只由下方实际消费点对已解析
            // `repository.path` 的同一份读取判定：读取失败在 provider 前停等
            // （detail 区分实际读取错误并指明来源）；聚焦/根文件可读则继续。
            // 审计事实（含成员无 recorded checkout）留在公共预检结果侧。
            Ok(())
        }
        Err(
            crate::product::logical_codebase::ProviderAdmissionError::Waiting {
                reason_code,
                detail,
                missing_materials,
                allowed_actions,
            },
        ) => {
            engine.persist_single_candidate_waiting_phase();
            // 6.1（方案 A 附带，D5）：trust 缺失的 generation waiting 桥接
            // 同源只读 manual_guidance——同一 gateway 注入的只读 trust
            // source 与冻结 authority root 生成（provider 特定工件路径/
            // trust 键/人工形状＋生成场景 retry_action「人工登记后显式
            // 重新生成」）；只读零登记/零 trust 写（start_generation 不
            // 自动登记），指导缺席（无 source/无 adapter）时保持原投影，
            // 不伪造 capability。
            // 指引卡简化（owner 2026-10-12 直批）：除平铺 manual_guidance
            // 外，同源等待投影原样随行（结构化 command_markdown 供上层
            // trust_missing_guidance 帧；旧客户端只读平铺串，不受影响）。
            let trust_waiting = if reason_code == PROVIDER_TRUST_MISSING_WAITING_REASON {
                let author_provider = engine.session().author_provider.clone();
                launch
                    .gateway
                    .read_only_generation_trust_waiting(&author_provider)
            } else {
                None
            };
            let manual_guidance = trust_waiting.as_ref().map(|waiting| {
                format!(
                    "{}（retry_action：{}）",
                    waiting.manual_guidance, waiting.retry_action
                )
            });
            Err(Box::new(SingleCandidateProviderRunError::AdmissionWaiting {
                reason_code,
                detail,
                missing_materials,
                allowed_actions,
                manual_guidance,
                trust_waiting: trust_waiting.map(Box::new),
            }))
        }
        Err(crate::product::logical_codebase::ProviderAdmissionError::Store(error)) => {
            Err(Box::new(SingleCandidateProviderRunError::Message(format!(
                "provider admission preflight store error: {error}"
            ))))
        }
    }
}

#[allow(clippy::result_large_err)] // 同上:运行收尾 typed 出口。
pub(crate) async fn run_single_candidate_author(
    engine: &mut WorkspaceEngine,
    provider_for_run: Arc<dyn StreamingProviderAdapter>,
    run_cancel: CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    recovery_entry: SingleCandidateAuthorRecoveryEntry,
    run_context: &ProviderRunContext,
) -> Result<SingleCandidateProviderRunOutcome, SingleCandidateProviderRunError> {
    // B2c（Task 5.5，设计 §5.3 SC 纪律①）：显式 fresh（LcFreshContext）不经
    // precheck/reserve——同 logical reservation 重入不 reserve 第二次（trap：
    // phase=Generate 时同 key 重放返回 false 会误判 Superseded）；claim CAS
    // 已 single-flight。
    let is_fresh_context = matches!(
        recovery_entry,
        SingleCandidateAuthorRecoveryEntry::LcFreshContext { .. }
    );
    // Ordinary 入口的 full resume precheck 先于新业务 reserve——mismatch
    // 家族（旧 native 在而冻结绑定不可用/已吊销）转 typed
    // NativeSupersededWaiting（durable 等待收口保留原 key/phase），不再落
    // reserve 让位静默 Superseded / generic Failed。
    if !is_fresh_context
        && let Some(reason_code) = engine
            .precheck_single_candidate_author_resume()
            .map_err(SingleCandidateProviderRunError::Message)?
    {
        return match engine
            .persist_single_candidate_author_superseded_wait(reason_code)
            .await
        {
            Ok(recovery) => Err(SingleCandidateProviderRunError::NativeSupersededWaiting {
                reason_code: reason_code.to_string(),
                superseded_native_session_id: recovery.superseded_native_session_id.clone(),
                recovery_id: recovery.recovery_id,
            }),
            Err(message) => Err(SingleCandidateProviderRunError::Message(format!(
                "persist single candidate author superseded wait failed: {message}"
            ))),
        };
    }
    let should_start = if is_fresh_context {
        true
    } else {
        match engine.reserve_single_candidate_author_start() {
            Ok(should_start) => should_start,
            Err(message) => {
                engine.persist_single_candidate_terminal_phase(
                    crate::product::models::SingleCandidatePhase::Failed,
                );
                return Err(SingleCandidateProviderRunError::Message(message));
            }
        }
    };
    if !should_start {
        // 败者让位 vs 无法启动（k3 P2 / RCA 修法 3）：键已被领走时按重载后的
        // durable phase 区分——Failed 表示终态失败且无他者在跑（唯一真实失败
        // 已由失败方自己落盘），保留可见 Message 失败路径作为恢复入口（回滚
        // PrepareContext + Error 广播）；Completed/Generate 表示键的持有者是
        // 健康在途 run 或已完结会话，迟到 run 静默让位，不产生虚假 failed
        // 节点与误导性错误。
        return match engine.session().single_candidate_phase.as_ref() {
            Some(crate::product::models::SingleCandidatePhase::Failed) => {
                Err(SingleCandidateProviderRunError::Message(
                    "SingleCandidate session 已终态失败，本次运行无法启动；请显式重新开始生成"
                        .to_string(),
                ))
            }
            _ => Err(SingleCandidateProviderRunError::Superseded),
        };
    }

    let lifecycle = LifecycleStore::new(run_context.app_paths.clone());
    let request = build_work_item_plan_generate_request(engine, &lifecycle).map_err(|error| {
        SingleCandidateProviderRunError::Message(format!(
            "build single candidate request failed: {error}"
        ))
    })?;
    let repository = workspace_repository_for_session(
        &run_context.app_paths,
        &lifecycle,
        &run_context.session_record,
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!("load repository failed: {error}"))
    })?;
    // B1 铁律（多仓 plan 聚合视图缺口，session_0003 现场）：聚合视图会话
    // 的 IR/WI 目标按每 WI Plan Intent 成员归属落成员物理 ID——聚合视图
    // ID 不得作 compile target（Final Compile 以 work_item_target_missing
    // 拒收，owner 验收卡死）。业务 target/材料来源仍用上方 `repository`。
    let member_targets_plan = lifecycle
        .get_issue_work_item_plan(
            &run_context.session_record.project_id,
            &run_context.session_record.issue_id,
            &run_context.session_record.entity_id,
        )
        .map_err(|error| {
            SingleCandidateProviderRunError::Message(format!(
                "load plan for member targets failed: {error}"
            ))
        })?;
    let member_targets =
        crate::product::workspace_engine::draft_batch::compile_support::plan_compile_member_targets(
            &lifecycle,
            &member_targets_plan,
            &repository,
        )
        .map_err(|error| {
            SingleCandidateProviderRunError::Message(format!(
                "resolve plan member targets failed: {error}"
            ))
        })?;
    // SG5.1（REQ-CG-03、REQ-SG-02）：author prompt 的已确认仓序事实——与
    // Final Compile / SG1 有效图的 change_order 同一规则源（confirmed design
    // 单源装载，只读消费、不改 admission）；LC 多仓 plan 才有成员/仓序教学，
    // legacy 单仓 member_targets=None → 教学段缺席、首轮 prompt 逐字节不变。
    let confirmed_change_order = crate::product::workspace_engine::draft_batch::compile_support::load_change_order_from_confirmed_design(
        &lifecycle,
        &member_targets_plan,
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!(
            "load confirmed change order for author prompt failed: {error}"
        ))
    })?;
    let issue = IssueStore::new(run_context.app_paths.clone())
        .get(
            &run_context.session_record.project_id,
            &run_context.session_record.issue_id,
        )
        .map_err(|error| {
            SingleCandidateProviderRunError::Message(format!("load issue failed: {error}"))
        })?;
    // C-1/路 C（Task 4，REQ-REG-12）：LC 会话在消费 language rules 之前先过
    // provider admission 公共预检——根规则/发布链/policy/capability 等公共门
    // 漂移时转 waiting/Prepare 面（可操作等待事实）；成员语言规则等待不在此
    // 阻断（见 `preflight_lc_member_rules_before_author_run` 路 C 说明）。
    // 单仓 legacy 会话（无 gateway 注入或无 planning launch，判定与下方
    // `resolve_plan_author_launch` 同谓词）零变化。
    let lc_plan_launch = super::gateway_start::logical_plan_launch_for(engine);
    if let Some(lc_plan_launch) = lc_plan_launch.as_ref() {
        preflight_lc_member_rules_before_author_run(engine, run_context, lc_plan_launch)
            .map_err(|error| *error)?;
    }
    // 路 C（Task 4，REQ-REG-12）：LC SC 的必需成员材料在真实读取点判定并
    // 同源消费——对已解析 `repository.path`（`workspace_repository_for_session`
    // 的身份解析 primary checkout；多 target 无唯一 focus 时为聚合根视图）
    // 做唯一一次 UTF-8 读取：成功的同一份 `String` 直接构造 prompt（不预检
    // 一份、消费另一份）；NotFound/权限/目录/非法 UTF-8 在 provider 启动前
    // 转 waiting/Prepare 面（reason 保持 member_language_rules_missing 语义，
    // detail 区分实际读取错误并指明来源；Prepare/Retry、零启动），恢复文件后
    // 原会话重新发起生成即按同一来源重读。非 LC 会话保持旧终态 Failed 行为
    // 零变化。
    let language_rules_path = repository.path.join(".claude/rules/language.md");
    let language_rules = match std::fs::read_to_string(&language_rules_path) {
        Ok(rules) => rules,
        Err(error) if lc_plan_launch.is_some() => {
            engine.persist_single_candidate_waiting_phase();
            return Err(SingleCandidateProviderRunError::AdmissionWaiting {
                reason_code: PUBLIC_MEMBER_RULES_WAITING_REASON.to_string(),
                detail: format!(
                    "read required SingleCandidate language rules failed for repository {} at {}: {error}",
                    repository.path.display(),
                    language_rules_path.display(),
                ),
                missing_materials: vec![format!(
                    "actual SC consumption path {} (unreadable: {error})",
                    language_rules_path.display()
                )],
                allowed_actions: vec![
                    crate::product::logical_codebase::BootstrapActionKind::Prepare,
                    crate::product::logical_codebase::BootstrapActionKind::Retry,
                ],
                // 成员规则等待不携带 trust 指导（指导只绑定 trust 维度失败）。
                manual_guidance: None,
                trust_waiting: None,
            });
        }
        Err(error) => {
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::Message(format!(
                "load required SingleCandidate language rules failed for repository {} at {}: {error}",
                repository.path.display(),
                language_rules_path.display(),
            )));
        }
    };
    let story_context_blocks =
        crate::product::work_item_split_engine::context::collect_story_context(
            &lifecycle, &request, &issue,
        )
        .map_err(|error| {
            SingleCandidateProviderRunError::Message(format!(
                "load story context failed: {}",
                error.message
            ))
        })?;
    let story_context = story_context_blocks.join("\n\n");
    let design_context_blocks =
        crate::product::work_item_split_engine::context::collect_design_context(
            &lifecycle, &request, &issue,
        )
        .map_err(|error| {
            SingleCandidateProviderRunError::Message(format!(
                "load design context failed: {}",
                error.message
            ))
        })?;
    // r41 A1:story+design 双源提取已登记需求 ID——仅扫 design 时,design
    // 正文缺 NFR token 会让 [design_requirements] 白名单缺员,反而逼 AI
    // 编造编号(r33/r37 三轮现场);story 正文的 REQ-/NFR- 同属登记域。
    let registered_requirement_ids = {
        let mut contexts = story_context_blocks.clone();
        contexts.extend(design_context_blocks.iter().cloned());
        crate::product::work_item_split_engine::context::extract_registered_requirement_ids(
            &contexts,
        )
    };
    let design_requirement_ids = registered_requirement_ids;
    let design_context = design_context_blocks.join("\n\n");
    let repository_structure =
        crate::product::work_item_split_engine::context::summarize_repository_structure(
            &repository.path,
            // REQ-ROUTE-01：归属必填后「真多仓 LC」以非 legacy 别名判定（同
            // issue_is_cross_repo_shape 口径）。
            issue.logical_codebase_id
                != crate::product::logical_codebase::legacy_logical_codebase_id(&issue.project_id),
        );
    let author_provider = engine.session().author_provider.clone();
    // B2b1(Task 5.2,REQ-PLN-08):envelope target 恒聚合根锚(E native
    // anchor 消费收敛进 `resolve_plan_author_launch`);r27 成员锚同源机制
    // 在 root 锚下结构性消失,fresh 轮与修订轮 envelope 恒一致。
    let launch = resolve_plan_author_launch(engine).map_err(|error| {
        SingleCandidateProviderRunError::Message(format!("logical plan launch failed: {error}"))
    })?;
    // F5-A：SC 修订轮 findings 回灌——最近 verdict 要求返修时，本轮 author 重跑
    // 使用返修 prompt（在首轮完整 prompt 的尾部输出指令前注入 reviewer findings 与
    // 硬性修复指令）；首轮（无 verdict）prompt 逐字节不变。
    // F-51（REQ-WSC-06）：author prompt 镜像教学使用与 prevalidate 同源的 plan record
    // options；记录缺失时 fail-safe 全 false（教学缺席，权威校验仍由编译前预检承担）。
    let plan_options = {
        let session = engine.session();
        lifecycle
            .get_issue_work_item_plan(&session.project_id, &session.issue_id, &session.entity_id)
            .map(|plan| plan.options)
            .unwrap_or(crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            })
    };
    let revision_verdict = engine.single_candidate_pending_revision_verdict();
    // B2c（Task 5.5，设计 §5.3 sc_author 行）：fresh 重入的返修轮 prompt
    // 自足——全新 provider 会话（resume=None）无旧会话历史可依赖，按
    // payload source ref 装载 durable 上一版 candidate source 全文，走
    // fresh 返修 builder（[previous_source] 先于 [review_revision]）；正常
    // resume 的修订 prompt 与首轮 prompt 逐字节零变化。
    let previous_source_for_fresh_revision: Option<String> = if is_fresh_context
        && revision_verdict.is_some()
    {
        let source_ref = match &recovery_entry {
            SingleCandidateAuthorRecoveryEntry::LcFreshContext {
                source_revision_ref: Some(source_ref),
            } => source_ref.clone(),
            _ => {
                return Err(SingleCandidateProviderRunError::Message(
                    "sc_author fresh revision round requires the payload source revision ref to carry the previous candidate source"
                        .to_string(),
                ));
            }
        };
        let (project_id, issue_id, plan_id) = {
            let session = engine.session();
            (
                session.project_id.clone(),
                session.issue_id.clone(),
                session.entity_id.clone(),
            )
        };
        let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
            project_id,
            issue_id,
            plan_id,
        };
        Some(
            crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
                lifecycle.app_paths(),
            )
            .get_source_revision(&scope, &source_ref)
            .map_err(|error| {
                SingleCandidateProviderRunError::Message(format!(
                    "load previous source for sc fresh revision failed: {error:?}"
                ))
            })?
            .source,
        )
    } else {
        None
    };

    // D5 事前：装载基线树注入 `baseline:` 段（装载面见 single_candidate_baseline_tree）。
    let baseline_tree_section = if cfg!(feature = "wig-d5-before") {
        single_candidate_baseline_tree::baseline_tree_section(
            &lifecycle,
            &run_context.session_record.project_id,
            &run_context.session_record.issue_id,
            &repository.path,
        )
    } else {
        None
    };
    let full_prompt = match (revision_verdict.as_ref(), previous_source_for_fresh_revision.as_deref())
    {
        (Some(review), Some(previous_source)) => {
            crate::product::work_item_split_engine::prompts::
                build_work_item_plan_markdown_fresh_revision_prompt(
                    &request,
                    &issue,
                    &repository,
                    crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                        story_context: &story_context,
                        design_context: &design_context,
                        design_requirement_ids: &design_requirement_ids,
                        repository_structure: &repository_structure,
                        plan_options: &plan_options,
                        language_rules: &language_rules,
                        routing_context: &launch.routing_context(),
                        member_targets: member_targets.as_ref(),
                        change_order: &confirmed_change_order,
                        baseline_tree: baseline_tree_section.as_deref(),
                    },
                    review,
                    previous_source,
                )
        }
        (Some(review), None) => {
            crate::product::work_item_split_engine::prompts::
                build_work_item_plan_markdown_revision_prompt(
                    &request,
                    &issue,
                    &repository,
                    crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                        story_context: &story_context,
                        design_context: &design_context,
                        design_requirement_ids: &design_requirement_ids,
                        repository_structure: &repository_structure,
                        plan_options: &plan_options,
                        language_rules: &language_rules,
                        routing_context: &launch.routing_context(),
                        member_targets: member_targets.as_ref(),
                        change_order: &confirmed_change_order,
                        baseline_tree: baseline_tree_section.as_deref(),
                    },
                    review,
                )
        }
        (None, _) => crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: &story_context,
                design_context: &design_context,
                design_requirement_ids: &design_requirement_ids,
                repository_structure: &repository_structure,
                plan_options: &plan_options,
                language_rules: &language_rules,
                routing_context: &launch.routing_context(),
                member_targets: member_targets.as_ref(),
                change_order: &confirmed_change_order,
                baseline_tree: baseline_tree_section.as_deref(),
            },
        ),
    }
    .map_err(SingleCandidateProviderRunError::Message)?;
    let node_id = if engine.active_node_type()
        == Some(crate::web::workspace_ws_types::TimelineNodeType::AuthorRun)
    {
        engine.active_timeline_node_id().ok_or_else(|| {
            SingleCandidateProviderRunError::Message(
                "single candidate author run node unavailable".to_string(),
            )
        })?
    } else {
        engine.begin_work_item_plan_author_run().await
    };

    #[cfg(test)]
    super::record_single_candidate_generation_step(
        &engine.session().session_id,
        "full_markdown_author",
    );
    let prompt_event_detail = match (&revision_verdict, is_fresh_context) {
        (Some(_), true) => {
            "发送给 SingleCandidate markdown author 的显式 fresh 返修提示词（全新会话自足：上一版 source 全文＋reviewer findings 回灌）"
        }
        (Some(_), false) => {
            "发送给 SingleCandidate markdown author 的返修提示词（已回灌 reviewer findings）"
        }
        (None, _) => "发送给 SingleCandidate markdown author 的完整提示词",
    };
    engine
        .emit_provider_prompt_event(
            &node_id,
            full_prompt.clone(),
            prompt_event_detail,
            Some(author_provider.clone()),
        )
        .await;
    // B2c（Task 5.5，设计 §5.3 sc_author 行）：fresh 重入走 fresh input
    // builder——resume_provider_session_id 结构性为 None（全新物理 native，
    // 不回捞旧会话）；正常 run 维持 resume 候选路径零变化。
    let provider_input = if is_fresh_context {
        engine.build_work_item_plan_streaming_input_fresh(
            crate::product::work_item_split_engine::types::provider_name_to_type(&author_provider),
            full_prompt.clone(),
            repository.path.to_string_lossy().to_string(),
            author_provider.clone(),
            PlanAuthorOutputContract::Structured,
        )
    } else {
        engine.build_work_item_plan_streaming_input(
            crate::product::work_item_split_engine::types::provider_name_to_type(&author_provider),
            full_prompt.clone(),
            repository.path.to_string_lossy().to_string(),
            author_provider.clone(),
            PlanAuthorOutputContract::Structured,
        )
    }
    .map_err(SingleCandidateProviderRunError::Message)?;
    let provider_input = engine.attach_tool_policy_audit(provider_input);
    // Task 1b 段②生产臂:SC markdown author 单次 provider run 绑定
    // split run 身份;Markdown plan 产物按原文存档收口。
    let plan_split_run =
        match crate::web::workspace_ws_handler::run::begin_plan_split_run_if_logical(
            &launch,
            &lifecycle,
            &engine.session().project_id,
            &engine.session().issue_id,
            &author_provider,
            &engine.session().session_id,
        ) {
            Ok(run) => run,
            Err(message) => {
                engine.persist_single_candidate_terminal_phase(
                    crate::product::models::SingleCandidatePhase::Failed,
                );
                return Err(SingleCandidateProviderRunError::Message(message));
            }
        };
    // T6 fix-loop r2：trust 分类已下沉 start_work_item_plan_author 的
    // Logical 分支共享口（preflight 之后的撤销窗口残余同样以分类面呈现）。
    let provider_session = start_work_item_plan_author(
        launch.clone(),
        Arc::clone(&provider_for_run),
        provider_input,
        run_cancel.clone(),
        plan_split_run.as_ref(),
    )
    .await;
    let full_output = match engine
        .drive_work_item_plan_provider_session_to_output(
            provider_session,
            command_rx,
            node_id.clone(),
            author_provider.clone(),
        )
        .await
    {
        Ok(output) => output,
        Err(message) => {
            crate::web::workspace_ws_handler::run::fail_plan_split_run(
                plan_split_run.as_ref(),
                &format!("provider session drive failed: {message}"),
            );
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::AlreadyFinished);
        }
    };
    crate::web::workspace_ws_handler::run::close_plan_split_run_with_markdown(
        plan_split_run.as_ref(),
        &full_prompt,
        &full_output,
    );
    let delivery = prepare_author_delivery_for_compile(&full_output);
    emit_author_heading_normalized_event(
        engine,
        &node_id,
        &full_output,
        &delivery,
        &author_provider,
    )
    .await;
    emit_author_ears_normalized_event(engine, &node_id, &full_output, &delivery, &author_provider)
        .await;
    let full_output = delivery.source;
    // F2-B：SC compile 失败教学重驱（同 candidate 恰一次）。SG6a.2（REQ-SG-05
    // SC 初轮行）：教学重驱资格只认 parse 语法类共享码白名单
    // （TEACHABLE_PARSE_FAILURE_CODES：missing_section / invalid_ears /
    // invalid_work_item_id）——lowering 类（缺席/值域统一）零重驱资格，
    // 原 message「缺少/缺失」措辞分流（is_lowering_absence_message）已按
    // F1.2 typed 口径删除。compile 失败不再直接终态：先在 handler 预检查
    // 中复用与引擎落盘路径同源的 contract_autorepair 收敛器；收敛成功的
    // 机械 lowering 缺口（当前为重复 trusted command）直接带着修复后的
    // source 进入 complete，后者再次经过同一收敛入口并落盘。收敛失败则
    // 保留原诊断，parse 语法类继续教学重驱。重驱产物重新进入本循环，因此
    // 同样不能绕过收敛器；重驱槽与收敛轮彼此独立，每 candidate 仍至多
    // 一次额外 provider 驱动。
    // 3.6 弱模型基线加固：IR 校验失败（validate plan candidate IR failed 类，
    // 如 unknown_requirement_ref / acceptance_criterion_without_reviewer_check）
    // 同样享有恰一次教学重驱——在持久化前预跑同源 validate_plan_candidate_ir
    // 分类，重驱 prompt 附错误原文与修正引用/补齐字段指令；重驱再败→终态含
    // 两轮；非 IR/非白名单的其他失败（如内部错误/lowering 类）不触发重驱。
    // 重驱机会在 compile/IR 两类间共享（`compile_ir_rerun_used` 单槽）。
    // 4. 分槽（2026-10-31 真机修订）：结构门重驱有**自己的**槽位
    // （`structure_rerun_used`）——两类槽互不挤占、每类至多一次 → 额外
    // provider 驱动总上限 2 次（有界，不是无限自愈）。
    let compile_context = crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
        target_repository_id: repository.id.clone(),
        member_targets: member_targets.clone(),
    };
    let mut compile_source = full_output;
    // `first_round_failure`：首个失败类的描述（只用于终态错误文案，不再兼作槽位标记）。
    let mut first_round_failure: Option<String> = None;
    // 槽位：compile/IR 两类共享一个，结构门类独立一个——互不挤占、每类至多一次。
    let mut compile_ir_rerun_used = false;
    let mut structure_rerun_used = false;
    let candidate_item_count = loop {
        let (ir, repaired_source, _autorepair_log) =
            match crate::product::workspace_engine::contract_autorepair::converge_work_item_plan_source(
                &compile_source,
                compile_context.clone(),
            ) {
                Ok(outcome) => outcome,
                Err(diagnostics) => {
                    let reasons = format_compile_failure_reasons(&diagnostics);
                    if !compile_ir_rerun_used
                        && diagnostics
                            .iter()
                            .any(|diagnostic| is_teachable_parse_failure(&diagnostic.code))
                    {
                        compile_ir_rerun_used = true;
                        first_round_failure.get_or_insert_with(|| reasons.join("; "));
                        let reredrive_prompt =
                            crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt(
                                &reasons,
                                &design_requirement_ids,
                            );
                        engine
                            .emit_execution_event(
                                ProviderExecutionEvent {
                                    event_id: format!("{node_id}_prompt_compile_reredrive"),
                                    kind: ProviderExecutionEventKind::Output,
                                    status: ProviderExecutionEventStatus::Started,
                                    title: "SC compile 失败教学重驱提示词".to_string(),
                                    detail: Some(
                                        "parse 语法类（missing_section / invalid_ears / invalid_work_item_id）compile 失败的教学重驱（含 compile 错误原文），恰一次"
                                            .to_string(),
                                    ),
                                    command: None,
                                    cwd: None,
                                    output: Some(reredrive_prompt.clone()),
                                    exit_code: None,
                                },
                                Some(node_id.clone()),
                                Some(author_provider.clone()),
                            )
                            .await;
                        let reredrive_run = match crate::web::workspace_ws_handler::run::begin_plan_split_run_if_logical(
                            &launch,
                            &lifecycle,
                            &engine.session().project_id,
                            &engine.session().issue_id,
                            &author_provider,
                            &engine.session().session_id,
                        ) {
                            Ok(run) => run,
                            Err(message) => {
                                engine.persist_single_candidate_terminal_phase(
                                    crate::product::models::SingleCandidatePhase::Failed,
                                );
                                return Err(SingleCandidateProviderRunError::Message(message));
                            }
                        };
                        compile_source = drive_single_candidate_reredrive(
                            engine,
                            &launch,
                            Arc::clone(&provider_for_run),
                            &run_cancel,
                            command_rx,
                            &node_id,
                            &author_provider,
                            &reredrive_prompt,
                            &repository.path,
                            reredrive_run.as_ref(),
                        )
                        .await?;
                        continue;
                    }
                    let detail = reasons.join("; ");
                    let message = match first_round_failure.as_deref() {
                        Some(first_round) => format!(
                            "compile markdown source failed (after one teaching re-drive): \
                             first round: {first_round}; re-drive round: {detail}\
                             {SINGLE_CANDIDATE_TERMINAL_REOPEN_GUIDANCE}"
                        ),
                        None => format!(
                            "compile markdown source failed: {detail}\
                             {SINGLE_CANDIDATE_TERMINAL_REOPEN_GUIDANCE}"
                        ),
                    };
                    engine.persist_single_candidate_terminal_phase(
                        crate::product::models::SingleCandidatePhase::Failed,
                    );
                    return Err(SingleCandidateProviderRunError::Message(message));
                }
            };
        compile_source = repaired_source;

        if let Some(reasons) = prevalidate_plan_candidate_ir(engine, &lifecycle, &request, &ir) {
            if !compile_ir_rerun_used {
                compile_ir_rerun_used = true;
                first_round_failure.get_or_insert_with(|| reasons.join("; "));
                let reredrive_prompt =
                    build_work_item_plan_ir_reredrive_prompt(&reasons, &design_requirement_ids);
                engine
                    .emit_execution_event(
                        ProviderExecutionEvent {
                            event_id: format!("{node_id}_prompt_ir_reredrive"),
                            kind: ProviderExecutionEventKind::Output,
                            status: ProviderExecutionEventStatus::Started,
                            title: "SC IR 校验失败教学重驱提示词".to_string(),
                            detail: Some(
                                "IR 校验失败（unknown_requirement_ref / acceptance_criterion_without_reviewer_check 等）的教学重驱（含错误原文与修正指令），恰一次"
                                    .to_string(),
                            ),
                            command: None,
                            cwd: None,
                            output: Some(reredrive_prompt.clone()),
                            exit_code: None,
                        },
                        Some(node_id.clone()),
                        Some(author_provider.clone()),
                    )
                    .await;
                let reredrive_run =
                    match crate::web::workspace_ws_handler::run::begin_plan_split_run_if_logical(
                        &launch,
                        &lifecycle,
                        &engine.session().project_id,
                        &engine.session().issue_id,
                        &author_provider,
                        &engine.session().session_id,
                    ) {
                        Ok(run) => run,
                        Err(message) => {
                            engine.persist_single_candidate_terminal_phase(
                                crate::product::models::SingleCandidatePhase::Failed,
                            );
                            return Err(SingleCandidateProviderRunError::Message(message));
                        }
                    };
                compile_source = drive_single_candidate_reredrive(
                    engine,
                    &launch,
                    Arc::clone(&provider_for_run),
                    &run_cancel,
                    command_rx,
                    &node_id,
                    &author_provider,
                    &reredrive_prompt,
                    &repository.path,
                    reredrive_run.as_ref(),
                )
                .await?;
                continue;
            }
            let detail = reasons.join("; ");
            let message = match first_round_failure.as_deref() {
                Some(first_round) => format!(
                    "validate plan candidate IR failed (after one teaching re-drive): \
                     first round: {first_round}; re-drive round: {detail}\
                     {SINGLE_CANDIDATE_TERMINAL_REOPEN_GUIDANCE}"
                ),
                None => format!(
                    "validate plan candidate IR failed: {detail}\
                     {SINGLE_CANDIDATE_TERMINAL_REOPEN_GUIDANCE}"
                ),
            };
            engine.persist_single_candidate_terminal_phase(
                crate::product::models::SingleCandidatePhase::Failed,
            );
            return Err(SingleCandidateProviderRunError::Message(message));
        }

        break match engine
            .complete_single_candidate_work_item_plan_author(
                compile_source,
                repository.id.clone(),
                member_targets.clone(),
                // 结构门类重驱槽（**独立槽**，与 compile/IR 槽互不挤占）：
                // 空闲时结构门拒收先教学重驱作者一次，占用后直接落人工门
                // （引擎侧收口，runner 不再判）。
                !structure_rerun_used,
            )
            .await
        {
            Ok(SingleCandidateAuthorCompletion::Candidate { item_count }) => item_count,
            // SG3a.1 拒收 + 结构门槽空闲：占用**结构门自己的**槽并带着可执行
            // 修复指令重驱作者一次（provider 通道复用，不新造 attempt 通道）。
            Ok(SingleCandidateAuthorCompletion::Rejected { repair_prompt }) => {
                structure_rerun_used = true;
                first_round_failure.get_or_insert_with(
                    super::single_candidate_structure_reredrive::structure_gate_first_round_failure,
                );
                compile_source =
                    super::single_candidate_structure_reredrive::drive_structure_gate_reredrive(
                        engine,
                        &lifecycle,
                        &launch,
                        Arc::clone(&provider_for_run),
                        &run_cancel,
                        command_rx,
                        &node_id,
                        &author_provider,
                        &repair_prompt,
                        &repository.path,
                    )
                    .await?;
                continue;
            }
            // SG3a.1 拒收已按 typed 失败收口（durable + 尝试流水 + 重写/人工门）：
            // 引擎已在重写或人工门态，本次 provider run 正常结束，**不落 failed**。
            Ok(SingleCandidateAuthorCompletion::Routed) => {
                return Ok(SingleCandidateProviderRunOutcome::Completed);
            }
            Err(message) => {
                engine.persist_single_candidate_terminal_phase(
                    crate::product::models::SingleCandidatePhase::Failed,
                );
                return Err(SingleCandidateProviderRunError::Message(message));
            }
        };
    };
    #[cfg(test)]
    {
        super::record_work_item_plan_parser_path(
            &engine.session().session_id,
            "single_candidate_markdown",
        );
        super::record_single_candidate_generation_step(
            &engine.session().session_id,
            "parse_source_revision",
        );
    }

    let decision_input = crate::product::workspace_engine::SingleCandidateGenerationDecisionInput {
        provider: author_provider.clone(),
        candidate_item_count,
    };
    let generation_mode =
        crate::product::workspace_engine::select_internal_generation_mode(&decision_input);
    #[cfg(test)]
    super::record_single_candidate_generation_step(&engine.session().session_id, "selector");
    let generation_diagnostic = format!(
        "internal generation mode={generation_mode:?}; provider={:?}; compiled_item_count={}",
        decision_input.provider, decision_input.candidate_item_count,
    );
    tracing::info!(
        session_id = %engine.session().session_id,
        provider = ?decision_input.provider,
        compiled_item_count = decision_input.candidate_item_count,
        generation_mode = ?generation_mode,
        "single-candidate internal generation mode diagnosed after compilation"
    );
    engine
        .emit_execution_event(
            ProviderExecutionEvent {
                event_id: format!("single_candidate_generation_mode_{node_id}"),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Completed,
                title: "SingleCandidate 内部生成模式已诊断".to_string(),
                detail: Some(generation_diagnostic),
                command: None,
                cwd: None,
                output: None,
                exit_code: None,
            },
            Some(node_id),
            Some(author_provider),
        )
        .await;
    Ok(SingleCandidateProviderRunOutcome::Completed)
}

#[cfg(test)]
mod tests {

    #[test]
    fn trims_provider_preamble_before_document_heading() {
        let source = "我会先读取上下文，再生成计划。\n\n# Work Item Plan\n## Work Item WI-001: x\n";

        assert_eq!(
            super::single_candidate_delivery::trim_provider_preamble(source),
            "# Work Item Plan\n## Work Item WI-001: x\n"
        );
    }

    #[test]
    fn trims_glued_preamble_before_document_heading() {
        let source = "我会先读取上下文，再生成计划。# Work Item Plan\n## Work Item WI-001: x\n";

        assert_eq!(
            super::single_candidate_delivery::trim_provider_preamble(source),
            "# Work Item Plan\n## Work Item WI-001: x\n"
        );
    }

    #[test]
    fn leaves_source_without_preamble_unchanged() {
        let source = "# Work Item Plan\n## Work Item WI-001: x\n";

        assert_eq!(
            super::single_candidate_delivery::trim_provider_preamble(source),
            source
        );
    }

    #[test]
    fn leaves_source_without_document_heading_unchanged() {
        let source = "我会先读取上下文，再生成计划。\n## Work Item WI-001: x\n";

        assert_eq!(
            super::single_candidate_delivery::trim_provider_preamble(source),
            source
        );
    }

    #[test]
    fn trims_code_fence_before_document_heading() {
        let source = "```markdown\n# Work Item Plan\n## Work Item WI-001: x\n```\n";

        assert_eq!(
            super::single_candidate_delivery::trim_provider_preamble(source),
            "# Work Item Plan\n## Work Item WI-001: x\n```\n"
        );
    }
}

#[cfg(test)]
#[path = "single_candidate_ir_prompt_tests.rs"]
mod ir_prompt_tests;
