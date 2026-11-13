use super::*;
use crate::product::workspace_engine::PlanAuthorOutputContract;
use crate::product::workspace_engine::SingleCandidateAuthorCompletion;
use crate::product::workspace_engine::draft_batch::member_checkout_prefixes;
use crate::web::workspace_ws_types::{ScEvidenceRef, ScRoundIdentity};
use sc_failure_transport::ScAuthorRoundDelivery;

pub(crate) enum SingleCandidateProviderRunOutcome {
    Completed,
}

// 停等臂携带完整预检/信任材料;按值传递避免全链路 Box 化。
#[derive(Debug)]
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

/// SC 物理轮绑定与教学驱动；保留子模块位置以共享编译规范化辅助。
#[path = "sc_failure_transport.rs"]
pub(super) mod sc_failure_transport;

/// P08：SC 轮入场编排（Ready 才真实 start / Completed 复用 durable raw /
/// Waiting 走既有 CAS / 显式 fresh 的当前 operation 绑定）——自 transport 物理
/// 拆出以守 1200 行硬线。
#[path = "sc_failure_round_entry.rs"]
pub(super) mod sc_failure_round_entry;

/// P09：SC 失败证据 blocking reasons 纯拼装（compile/IR/structure 三通道共用）。
#[path = "sc_failure_evidence.rs"]
pub(super) mod sc_failure_evidence;

/// P20：受控完整失败 source resolver（compile/IR/structure 三通道共用的真实
/// source 归一化读回）。模块本体声明在 single_candidate.rs 内——它复用既有
/// 生产归一化 helper，不另立 normalizer。
#[path = "sc_teaching_source.rs"]
pub(super) mod sc_teaching_source;

/// P22：SC 教学重驱 prompt 的最终装配 facade（纯函数面：零 I/O、零 provider、
/// 零 store）。模块本体声明在 single_candidate.rs 内——它消费同子树的
/// `single_candidate_compile_preflight` typed 事实与 P20 的 source 解析结果。
#[path = "sc_teaching_prompt.rs"]
pub(super) mod sc_teaching_prompt;

/// P25：SC 教学重驱 prompt 的审计面（全文不可变落盘 + 严格读回、缺提示的真实
/// 记录）——三臂共用一份实现，模块本体声明在 single_candidate.rs 内。
#[path = "sc_teaching_audit.rs"]
pub(super) mod sc_teaching_audit;

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
pub(super) mod single_candidate_compile_preflight;
mod single_candidate_delivery;

use single_candidate_compile_preflight::{
    compile_failure_surface, emit_contract_autorepair_instructions_event,
    prevalidate_plan_candidate_ir, record_equivalence_canonicalizations,
};
use single_candidate_structure_reredrive::{
    close_sc_teaching_delivered, drive_compile_teaching_reredrive,
    record_generate_segment_compile_failure, record_generate_segment_ir_validation_failure,
    sc_teaching_chain_binding,
};
// 终局面的类型/常量别名：真实组包与发布在
// `sc_failure_evidence::sc_failure_terminal` 子模块内实现，本层只取用出口。
use sc_failure_evidence::sc_failure_terminal::{
    SC_TERMINAL_CLASS_COMPILE, SC_TERMINAL_CLASS_IR_PREFLIGHT, SC_TERMINAL_CLASS_STRUCTURE_GATE,
    terminal_exit_error,
};
// 门结果的终局收口（三臂共用一份）：失败源/预算/审计失败 ⇒ 交终局处理器
// 组包（判定与缺提示记录已 durable），不在主文件重复终局逻辑。
use sc_teaching_audit::sc_teaching_terminal_grant;
// Task 4：`is_teachable_parse_failure` 判据已由 `teaching_verdict` 接管；
// IR 重驱 prompt 的构造已下沉 P22 facade（生产与测试同一入口，旧 builder
// 别名不再保名）。
#[cfg(test)]
use single_candidate_compile_preflight::is_teachable_parse_failure;

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
    // Task 6（REQ-WSC-28）：教学窗口的 run 级冻结绑定（业务请求/材料/上下文
    // digest，链内恒定——四类失败共用同一窗口的漂移核验基准）。
    let teaching_binding = sc_teaching_chain_binding(
        &request,
        &story_context_blocks,
        &design_context_blocks,
        &language_rules,
    );
    // C2（REQ-WSC-06）：SC 首轮 `structure:` 段成员感知装配——聚合根取
    // launch 的 working_dir（gateway 冻结 authority_root；focus 可能是成员
    // checkout，不得用 repository.path）；成员缺席 ⇒ 逐字节退回旧单仓摘要。
    use crate::product::work_item_split_engine::context as sc_structure;
    let is_lc_issue = sc_structure::is_lc_issue(&issue);
    let repository_structure = match (member_targets.as_ref(), lc_plan_launch.as_ref()) {
        (Some(sc_members), Some(sc_launch)) => {
            let sc_prefixes = member_checkout_prefixes::plan_member_checkout_prefixes(
                &lifecycle,
                &member_targets_plan,
            )
            .unwrap_or_else(|reason| {
                tracing::warn!("SC 成员 checkout 前缀装载失败，退回通用不可用面：{reason}");
                None
            });
            sc_structure::summarize_repository_structure_sc_members(
                &sc_launch.working_dir,
                is_lc_issue,
                Some(sc_members),
                sc_prefixes.as_ref(),
            )
        }
        _ => sc_structure::summarize_repository_structure(&repository.path, is_lc_issue),
    };
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
    // C2（REQ-WSC-06）：三分支共享同一 author 上下文——三处逐字重复的字面量
    // 收敛为单点纯移动（字段与取值逐字不变），为成员感知装配腾行守净零。
    let routing_context = launch.routing_context();
    let author_context =
        crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
            story_context: &story_context,
            design_context: &design_context,
            design_requirement_ids: &design_requirement_ids,
            repository_structure: &repository_structure,
            plan_options: &plan_options,
            language_rules: &language_rules,
            routing_context: &routing_context,
            member_targets: member_targets.as_ref(),
            change_order: &confirmed_change_order,
            baseline_tree: baseline_tree_section.as_deref(),
        };
    let full_prompt = match (revision_verdict.as_ref(), previous_source_for_fresh_revision.as_deref())
    {
        (Some(review), Some(previous_source)) => {
            crate::product::work_item_split_engine::prompts::
                build_work_item_plan_markdown_fresh_revision_prompt(
                    &request,
                    &issue,
                    &repository,
                    author_context,
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
                    author_context,
                    review,
                )
        }
        (None, _) => crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            author_context,
        ),
    }
    .map_err(SingleCandidateProviderRunError::Message)?;
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
    // P08a（计划 §2.4）：原轮 begin 前的失败链身份前置——durable 会话缺原
    // 业务键（或 lifecycle 不可读）时，在真实 split begin／provider 启动之前
    // 给出可见失败（不再空键启动、不由 ledger 形态猜身份）；返回值由 P08 首
    // consumer 接管构造失败链作用域，此处不预分配日志/handle，也不留假绑定。
    // P08a：真实业务键（失败链身份的原业务键）——不再丢弃返回值，由 P08 构造
    // 失败链作用域时消费。
    let business_key = engine
        .sc_author_business_key_for_failure_chain()
        .map_err(SingleCandidateProviderRunError::Message)?;
    // P08：SC 物理轮绑定（真实 handle → chain → durable RoundBound，启动前落盘）。
    // Case 5/6：显式 fresh operation 的物理身份由**当前 operation item** 决定——
    // 已绑定则只分类既有轮（不新 node/seq）；未绑定则沿旧链锚新建 AuthorRun node
    // 并以 max(ordinal)+1 绑定。非 fresh 保持「active 复用，否则新建」原样。
    let entry = sc_failure_round_entry::begin_sc_author_round_entry(
        engine,
        &lifecycle,
        &sc_failure_transport::sc_failure_scope_for(engine),
        &business_key,
        &author_provider,
        is_fresh_context,
    )
    .await?;
    // 只有 `Ready` 可真实启动；`Completed` 复用严格 durable raw；`Waiting` 走既有
    // `LcResumeRecoveryState::Waiting` CAS（不第二 start、不落 Failed/A4 终局）。
    let (bound, replayed_round) = match entry {
        sc_failure_transport::ScRoundEntry::Ready(bound) => (bound, None),
        sc_failure_transport::ScRoundEntry::Completed {
            bound,
            raw_text,
            raw,
        } => (bound, Some((raw_text, raw))),
        sc_failure_transport::ScRoundEntry::Waiting {
            bound,
            reason_code,
            detail,
        } => {
            return Err(sc_failure_round_entry::wait_sc_round_launch(
                engine,
                &bound,
                reason_code,
                &detail,
            )
            .await);
        }
    };
    let node_id = bound.round.node_id.clone();
    let mut current_round: Option<(ScEvidenceRef, ScRoundIdentity)>;
    let full_output = match replayed_round {
        // 已完成轮严格重放：同一 normalizer 复用 durable raw（零 start/claim、不改
        // 完成时间、不重发提示词事件）。
        Some((raw_text, raw)) => {
            let delivery = sc_failure_transport::replayed_sc_round_delivery(
                raw_text,
                raw,
                bound.round.clone(),
            );
            current_round = Some((delivery.raw, bound.round.clone()));
            delivery.source
        }
        None => {
            engine
                .emit_provider_prompt_event(
                    &node_id,
                    full_prompt.clone(),
                    prompt_event_detail,
                    Some(author_provider.clone()),
                )
                .await;
            // T6 fix-loop r2：trust 分类已下沉 start_work_item_plan_author 的
            // Logical 分支共享口（preflight 之后的撤销窗口残余同样以分类面呈现）；
            // 真实 start/驱动/严格收口整体在 `drive_ready_sc_round`（同出口语义）。
            let delivery = match sc_failure_round_entry::drive_ready_sc_round(
                engine,
                &lifecycle,
                &launch,
                Arc::clone(&provider_for_run),
                provider_input,
                &run_cancel,
                command_rx,
                &node_id,
                &author_provider,
                &full_prompt,
                &bound,
            )
            .await
            {
                Ok(delivery) => delivery,
                Err(error) => return Err(error),
            };
            current_round = Some((delivery.raw, bound.round.clone()));
            delivery.source
        }
    };
    // F2-B/SG6a.2（REQ-SG-05）/O1（裁决二 §2.2/§2.4）：教学重驱资格只认
    // typed 事实（WSC-09 判据面）；converge 单次编译不本地改写 source，
    // 机器指令渲染进 owner 可见呈现面、缺口沿机械 verdict 通道（F5-A）
    // 交原授权 author。重驱产物重新进入本循环；重驱机会由单一耐久教学
    // 窗口承载（Task 6，REQ-WSC-28）——四类失败共用，原调用在内合计
    // 两次（有界，不是无限自愈）。
    let compile_context = crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
        target_repository_id: repository.id.clone(),
        member_targets: member_targets.clone(),
    };
    let mut compile_source = full_output;
    // `first_round_failure`：首个失败类的描述（只用于终态错误文案，不再兼作槽位标记）。
    let mut first_round_failure: Option<String> = None;
    let candidate_item_count = loop {
        let crate::product::workspace_engine::contract_autorepair::ContractConvergeOutcome {
            ir,
            instructions: contract_autorepair_instructions,
            audit,
        } = match crate::product::workspace_engine::contract_autorepair::converge_work_item_plan_source(
            &compile_source,
            compile_context.clone(),
        ) {
            Ok(outcome) => outcome,
            Err(diagnostics) => {
                let (reasons, instruction_lines) =
                    compile_failure_surface(&compile_source, &diagnostics, &compile_context);
                let detail = reasons.join("; ");
                // Task 6：事实写读 → 既有资格/窗口 claim → 真实判定写读，
                // 全在 grant 之前（原窗口 establish/漂移/claim 顺序不变）。
                let outcome = sc_failure_evidence::gate_compile_failure(
                    engine,
                    &lifecycle,
                    &bound,
                    current_round.as_ref(),
                    &compile_source,
                    &diagnostics,
                    instruction_lines,
                    &teaching_binding,
                    &design_requirement_ids,
                    &detail,
                );
                // 失败源/预算/审计失败：判定与缺提示记录已 durable ⇒ 交终局处理器
                // 组包（不只丢一条 Message 丢掉终局）。
                let grant = sc_teaching_terminal_grant(
                    engine,
                    &lifecycle,
                    &bound.chain,
                    SC_TERMINAL_CLASS_COMPILE,
                    outcome,
                )
                .await?;
                // Task 8（5.1/5.2，REQ-WSC-25/28D）：generate 段失败流水行
                // （阶段=Parse/TypedLowering/TypedIrValidation；non-blocking）。
                record_generate_segment_compile_failure(engine, &lifecycle, &diagnostics, &detail);
                if let Some(grant) = grant {
                    first_round_failure.get_or_insert(detail.clone());
                    let ScAuthorRoundDelivery { source, raw, round } =
                        drive_compile_teaching_reredrive(
                            engine,
                            &lifecycle,
                            &launch,
                            &provider_for_run,
                            &run_cancel,
                            command_rx,
                            &node_id,
                            &author_provider,
                            "compile_reredrive",
                            "SC compile 失败教学重驱提示词",
                            "typed 完备判据（parse 码表 / lowering fault / IR fault，事实齐全且可定位）compile 失败的教学重驱（含 typed 载荷与错误原文），恰一次",
                            &grant.reredrive_prompt,
                            &repository.path,
                            &grant.attempt,
                            &grant.verify,
                            &bound,
                            current_round.as_ref(),
                        )
                        .await?;
                    compile_source = source;
                    current_round = Some((raw, round));
                    continue;
                }
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
                return Err(terminal_exit_error(
                    engine,
                    &lifecycle,
                    &bound.chain,
                    SC_TERMINAL_CLASS_COMPILE,
                    Vec::new(),
                    message,
                )
                .await);
            }
            };
        // O1（裁决二）：缺口机器指令不应用、不丢弃——渲染进 owner 可见
        // 执行事件随失败面呈现（与教学重驱 prompt 同源的呈现面）；缺口
        // 本体沿引擎既有机械 verdict 通道（F5-A）交原授权 author 返修。
        // prompt 携带指令文本驱动教学重驱的完整接线在 Task 4。
        emit_contract_autorepair_instructions_event(
            engine,
            &node_id,
            &author_provider,
            &contract_autorepair_instructions,
        )
        .await;
        // 3.3（REQ-WSC-25 序列）：等价规范化审计落库——generate 段 handler 是
        // 唯一写入点（幂等，键=node+内容摘要）；completion 复验侧不重复写。
        if !audit.equivalence_canonicalizations.is_empty() {
            record_equivalence_canonicalizations(
                engine,
                &lifecycle,
                &node_id,
                &audit.equivalence_canonicalizations,
            );
        }
        let preflight =
            prevalidate_plan_candidate_ir(engine, &lifecycle, &request, &ir, &compile_source);
        if preflight.has_failures() {
            // Task 6：与 compile 类共用同一耐久教学窗口（无独立槽位）。
            // Task 6：与 compile 类共用同一耐久教学窗口（无独立槽位）。
            let detail = preflight.presentation_reasons().join("; ");
            let outcome = sc_failure_evidence::gate_ir_failure(
                engine,
                &lifecycle,
                &bound,
                current_round.as_ref(),
                &compile_source,
                &preflight,
                &teaching_binding,
                &design_requirement_ids,
                &detail,
            );
            // 失败源/预算/审计失败：判定与缺提示记录已 durable ⇒ 交终局处理器
            // 组包（不只丢一条 Message 丢掉终局）。
            let grant = sc_teaching_terminal_grant(
                engine,
                &lifecycle,
                &bound.chain,
                SC_TERMINAL_CLASS_IR_PREFLIGHT,
                outcome,
            )
            .await?;
            // Task 8（5.1）：typed IR 校验失败流水行（stage=TypedIrValidation）。
            record_generate_segment_ir_validation_failure(engine, &lifecycle, &detail);
            if let Some(grant) = grant {
                first_round_failure.get_or_insert(detail.clone());
                let ScAuthorRoundDelivery { source, raw, round } =
                    drive_compile_teaching_reredrive(
                        engine,
                        &lifecycle,
                        &launch,
                        &provider_for_run,
                        &run_cancel,
                        command_rx,
                        &node_id,
                        &author_provider,
                        "ir_reredrive",
                        "SC IR 校验失败教学重驱提示词",
                        "IR 校验失败（unknown_requirement_ref / acceptance_criterion_without_reviewer_check 等）的教学重驱（含错误原文与修正指令），恰一次",
                        &grant.reredrive_prompt,
                        &repository.path,
                        &grant.attempt,
                        &grant.verify,
                        &bound,
                        current_round.as_ref(),
                    )
                    .await?;
                compile_source = source;
                current_round = Some((raw, round));
                continue;
            }
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
            return Err(terminal_exit_error(
                engine,
                &lifecycle,
                &bound.chain,
                SC_TERMINAL_CLASS_IR_PREFLIGHT,
                Vec::new(),
                message,
            )
            .await);
        }
        // Task 6：结构门类不再有独立槽位——拒收统一交回，重驱资格由同一
        // 教学窗口的 claim 结果决定（Denied ⇒ 停止该生成自动链）。
        break match engine
            .complete_single_candidate_work_item_plan_author(
                compile_source,
                repository.id.clone(),
                member_targets.clone(),
                true,
            )
            .await
        {
            Ok(SingleCandidateAuthorCompletion::Candidate { item_count }) => {
                close_sc_teaching_delivered(&lifecycle, &engine.session().session_id);
                item_count
            }
            // SG3a.1 拒收：消费同一教学窗口后带着可执行修复指令重驱作者
            // 一次（provider 通道复用，不新造 attempt 通道）。
            Ok(SingleCandidateAuthorCompletion::Rejected {
                failure,
                structure_failures,
            }) => {
                let detail =
                    super::single_candidate_structure_reredrive::structure_gate_first_round_failure();
                // 结构臂不重算 typed 集：原 sealed failure + 原 typed 失败集
                // 原样进事实写读，重驱资格仍由同一窗口 claim 决定。
                let outcome = sc_failure_evidence::gate_structure_failure(
                    engine,
                    &lifecycle,
                    &bound,
                    current_round.as_ref(),
                    &failure,
                    &structure_failures,
                    &teaching_binding,
                    &detail,
                );
                // 失败源/预算/审计失败：判定与缺提示记录已 durable ⇒ 交终局处理器
                // 组包（不只丢一条 Message 丢掉终局）。
                let grant = sc_teaching_terminal_grant(
                    engine,
                    &lifecycle,
                    &bound.chain,
                    SC_TERMINAL_CLASS_STRUCTURE_GATE,
                    outcome,
                )
                .await?;
                if let Some(grant) = grant {
                    first_round_failure.get_or_insert(detail.clone());
                    let ScAuthorRoundDelivery { source, raw, round } =
                        super::single_candidate_structure_reredrive::drive_structure_gate_reredrive(
                            engine,
                            &lifecycle,
                            &launch,
                            Arc::clone(&provider_for_run),
                            &run_cancel,
                            command_rx,
                            &node_id,
                            &author_provider,
                            &grant.reredrive_prompt,
                            &repository.path,
                            Some(&grant.attempt),
                            &grant.verify,
                            &bound,
                            current_round.as_ref(),
                        )
                        .await?;
                    compile_source = source;
                    current_round = Some((raw, round));
                    continue;
                }
                // Denied/Drift ⇒ 停止该生成自动链：保留 last-valid refs 与
                // 失败历史、不提升坏 candidate、不开人工门、不留 A 通道
                // 预领，按既有失败面回显两轮真实原因。
                let message = match first_round_failure.as_deref() {
                    Some(first_round) => format!(
                        "candidate structure gate rejected (after one teaching re-drive): \
                         first round: {first_round}; re-drive round: {detail}\
                         {SINGLE_CANDIDATE_TERMINAL_REOPEN_GUIDANCE}"
                    ),
                    None => format!(
                        "candidate structure gate rejected: {detail}\
                         {SINGLE_CANDIDATE_TERMINAL_REOPEN_GUIDANCE}"
                    ),
                };
                return Err(terminal_exit_error(
                    engine,
                    &lifecycle,
                    &bound.chain,
                    SC_TERMINAL_CLASS_STRUCTURE_GATE,
                    Vec::new(),
                    message,
                )
                .await);
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
