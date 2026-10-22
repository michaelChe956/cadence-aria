use super::*;
use crate::product::work_item_plan_compiler::grammar;
use crate::product::workspace_engine::PlanAuthorOutputContract;

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
        trust_waiting:
            Option<crate::product::logical_codebase::provider_trust::ProviderTrustWaiting>,
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

/// 可教学重驱的 compile parse 语法类诊断码。
///
/// 集合来源（Task 0 实查）：`work_item_plan_compiler::parse` 的语法类诊断码全集
/// 即 `grammar::DIAGNOSTIC_CODES`（missing_section / unknown_structured_key /
/// invalid_work_item_id / invalid_ears），减去 `contract_autorepair` 已确定性收敛
/// 的码——`unknown_structured_key` 在收敛器内按诊断行号确定性删行（F-41）、
/// 重复 trusted command 亦有确定性收敛轮，先确定性后模型，故不进教学重驱。
/// 其余三个码在编译 loop 现状下无任何自动返修面（F-48：invalid_ears 因此直达
/// 终态），教学重驱是其唯一一次自愈机会。
// r41 B2:unknown_structured_key 入可教面(F-48 先例:一次自愈机会;
// contract_autorepair 可收敛多数,残余经教学重驱带 B1 白名单自纠)。
const TEACHABLE_PARSE_FAILURE_CODES: [&str; 4] = [
    "missing_section",
    "invalid_ears",
    "invalid_work_item_id",
    "unknown_structured_key",
];

/// 该诊断码是否属于可教学重驱的 parse 语法类。
fn is_teachable_parse_failure(code: &str) -> bool {
    TEACHABLE_PARSE_FAILURE_CODES.contains(&code)
}

// SG6a.2（REQ-SG-05 SC 初轮行×F1.2 typed 口径）：SC 初轮教学重驱资格
// 只认共享 parse 语法码白名单（TEACHABLE_PARSE_FAILURE_CODES）；原
// `is_lowering_absence_message`（message 含「缺少/缺失」措辞即授重驱
// 资格）属字符串资格旁路，已删除——lowering 类（缺席与值域统一）零
// 教学重驱，确定性终态失败由 typed 诊断回显承载。

/// 丢弃 provider 在 markdown 文档标题前输出的前言，保留既有 parser 的失败语义。
///
/// 定位首个固定文档标题的字节偏移并从该处修剪；找不到标题时原样返回，避免把
/// 缺少标题的输出静默转换成另一种错误。
fn trim_provider_preamble(source: &str) -> &str {
    let document_heading = format!("{}\n", grammar::DOCUMENT_HEADING);
    source
        .find(&document_heading)
        .map(|offset| &source[offset..])
        .unwrap_or(source)
}

/// SC author 交付进入 compiler 前的确定性净化（结构标题归一化 + EARS 关键字
/// 空白归一化 + 前言修剪）。
///
/// 归一化先于修剪：前言修剪锚定的是规范英文文档标题，provider 输出「前言 +
/// 中文标题」时必须先把标题归一化才能锚定修剪。两处归一化都只吸收确定性
/// 抖动（固定词表标题翻译 / 关键字邻位空白），表外未知标题与正文不改，由
/// compiler fail-closed。
fn prepare_author_delivery_for_compile(
    raw: &str,
) -> crate::product::work_item_plan_compiler::NormalizedPlanSource {
    let normalized =
        crate::product::work_item_plan_compiler::normalize_delivery_before_compile(raw);
    crate::product::work_item_plan_compiler::NormalizedPlanSource {
        source: trim_provider_preamble(&normalized.source).to_string(),
        normalized_heading_lines: normalized.normalized_heading_lines,
        normalized_ears_lines: normalized.normalized_ears_lines,
    }
}

/// SingleCandidate 的单次 markdown author 链路。
///
/// Provider 完整输出直接进入 source revision 与 compiler；内部 selector 只在编译成功后
/// 基于 IR item 数和 provider profile 记录诊断，不触碰 legacy outline/draft/batch 链路。
/// SingleCandidate author 归一化审计事件的稳定 event_id。
///
/// execution event 按 event_id upsert:同一 AuthorRun 节点被重驱(中断恢复、
/// 重试)时若 event_id 只含 node_id,后一次归一化会覆盖前一次的审计记录。
/// 以 provider 原文内容摘要为去重键(与修订链 report id 的 `source_hash[..16]`
/// 模式一致):不同尝试内容各留一条审计,同内容重放保持同 ID 幂等去重。
fn author_normalization_event_id(kind: &str, node_id: &str, raw_output: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(raw_output.as_bytes()));
    format!(
        "single_candidate_{kind}_normalized_{node_id}_{}",
        &digest[..16]
    )
}

fn author_heading_normalized_event_id(node_id: &str, raw_output: &str) -> String {
    author_normalization_event_id("heading", node_id, raw_output)
}

fn author_ears_normalized_event_id(node_id: &str, raw_output: &str) -> String {
    author_normalization_event_id("ears_spacing", node_id, raw_output)
}

/// 归一化审计事件发射（首轮与 F2-B 教学重驱轮共用）：归一化行数>0 时才发。
async fn emit_author_heading_normalized_event(
    engine: &mut WorkspaceEngine,
    node_id: &str,
    raw_output: &str,
    delivery: &crate::product::work_item_plan_compiler::NormalizedPlanSource,
    author_provider: &ProviderName,
) {
    if delivery.normalized_heading_lines == 0 {
        return;
    }
    tracing::info!(
        session_id = %engine.session().session_id,
        node_id = %node_id,
        diagnostic = crate::product::work_item_plan_compiler::PLAN_HEADING_NORMALIZATION_DIAGNOSTIC,
        normalized_heading_lines = delivery.normalized_heading_lines,
        "single-candidate author markdown 结构标题已确定性归一化后再编译"
    );
    engine
        .emit_execution_event(
            ProviderExecutionEvent {
                event_id: author_heading_normalized_event_id(node_id, raw_output),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Completed,
                title: "SingleCandidate 结构标题确定性归一化".to_string(),
                detail: Some(format!(
                    "normalized {} structural heading lines via the fixed zh→en table before compile",
                    delivery.normalized_heading_lines
                )),
                command: None,
                cwd: None,
                output: None,
                exit_code: None,
            },
            Some(node_id.to_string()),
            Some(author_provider.clone()),
        )
        .await;
}

/// EARS 关键字空白归一化审计事件发射（首轮与 F2-B 教学重驱轮共用）：
/// 救回行数>0 时才发，保证「本次交付被确定性救回」可判定（REQ-WSC-02）。
async fn emit_author_ears_normalized_event(
    engine: &mut WorkspaceEngine,
    node_id: &str,
    raw_output: &str,
    delivery: &crate::product::work_item_plan_compiler::NormalizedPlanSource,
    author_provider: &ProviderName,
) {
    if delivery.normalized_ears_lines == 0 {
        return;
    }
    tracing::info!(
        session_id = %engine.session().session_id,
        node_id = %node_id,
        diagnostic =
            crate::product::work_item_plan_compiler::PLAN_EARS_SPACING_NORMALIZATION_DIAGNOSTIC,
        normalized_ears_lines = delivery.normalized_ears_lines,
        "single-candidate author markdown EARS 关键字空白已确定性归一化后再编译"
    );
    engine
        .emit_execution_event(
            ProviderExecutionEvent {
                event_id: author_ears_normalized_event_id(node_id, raw_output),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Completed,
                title: "SingleCandidate EARS 关键字空白确定性归一化".to_string(),
                detail: Some(format!(
                    "normalized {} EARS statement lines (keyword-adjacent whitespace only; \
                     WHEN / THE SYSTEM SHALL spacing) before compile",
                    delivery.normalized_ears_lines
                )),
                command: None,
                cwd: None,
                output: None,
                exit_code: None,
            },
            Some(node_id.to_string()),
            Some(author_provider.clone()),
        )
        .await;
}

/// compile 失败原因原文（与 single_candidate.rs `format_compiler_diagnostics`
/// 同源的 code:line:message 形态，供教学重驱 prompt 回灌与终态错误拼装）。
fn format_compile_failure_reasons(
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
) -> Vec<String> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{}:{}:{}",
                diagnostic.code, diagnostic.line, diagnostic.message
            )
        })
        .collect()
}

/// 3.6 弱模型基线加固：IR 预校验——在持久化前跑与
/// `complete_single_candidate_work_item_plan_author` 内部同源的
/// `validate_plan_candidate_ir`（上下文组装同源：plan 的 source spec ids 与
/// repository profile），使 IR 校验失败（unknown_requirement_ref /
/// acceptance_criterion_without_reviewer_check 等）可在 handler 分类并享有
/// 恰一次教学重驱。仅当预校验确定性失败时返回 Some(reasons)；上下文装载
/// 失败返回 None（不阻断流程），由 complete_... 权威路径以既有终态形态兜底。
fn prevalidate_plan_candidate_ir(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    request: &crate::web::types::GenerateWorkItemsRequest,
    ir: &crate::product::work_item_plan_compiler::PlanCandidateIr,
) -> Option<Vec<String>> {
    let session = engine.session();
    let plan = lifecycle
        .get_issue_work_item_plan(&session.project_id, &session.issue_id, &session.entity_id)
        .ok()?;
    let repository_profile = plan
        .repository_profile_ref
        .as_deref()
        .and_then(|profile_id| {
            lifecycle
                .get_repository_profile(&session.project_id, &session.issue_id, profile_id)
                .ok()
        });
    // F-56→REQ-PIB-03：运行期预校验与权威路径共用基线加载（按分支名取树）。
    // 基线不可解析 → Err 转预检诊断（经教学重驱可观测，重驱不治本，
    // 权威路径仍以 fail-closed 终态兜底）。
    let baseline_tree = match crate::product::workspace_engine::plan_preflight::plan_baseline_tree(
        lifecycle,
        &session.project_id,
        &session.issue_id,
    ) {
        Ok(baseline_tree) => baseline_tree,
        Err(message) => return Some(vec![message]),
    };
    // C1 Task 5（REQ-C1-PLAN-01）：existing 意图的授权解析面（读失败转预检
    // 诊断，权威路径仍 fail-closed）。
    let existing_work_item_ids =
        match lifecycle.list_work_items(&session.project_id, &session.issue_id) {
            Ok(records) => records
                .into_iter()
                .map(|record| record.id)
                .collect::<Vec<_>>(),
            Err(error) => return Some(vec![format!("list existing work items failed: {error}")]),
        };
    // 批 2 R1（REQ-WIGA-01）：enrolled 会话传真实 binding+成员映射（装载
    // 失败转预检诊断，权威路径仍 fail-closed）；Manual/非 enrolled 保持
    // None 零回归。
    let enrollment_binding =
        match engine.current_enrollment_binding_for_compile(lifecycle, &session.entity_id) {
            Ok(binding) => binding,
            Err(error) => {
                return Some(vec![format!(
                    "load enrollment binding for prevalidation failed: {error}"
                )]);
            }
        };
    let validation_member_targets = match &enrollment_binding {
        Some(_) => {
            let session_record = match lifecycle.get_workspace_session(&session.session_id) {
                Ok(record) => record,
                Err(error) => {
                    return Some(vec![format!(
                        "load plan session for member targets failed: {error}"
                    )]);
                }
            };
            let repository = match workspace_repository_for_session(
                &lifecycle.app_paths(),
                lifecycle,
                &session_record,
            ) {
                Ok(resolved) => resolved,
                Err(error) => {
                    return Some(vec![format!(
                        "resolve plan session repository for member targets failed: {error}"
                    )]);
                }
            };
            match crate::product::workspace_engine::draft_batch::compile_support::plan_compile_member_targets(
                lifecycle, &plan, &repository,
            ) {
                Ok(targets) => targets,
                Err(error) => {
                    return Some(vec![format!(
                        "resolve plan member targets for prevalidation failed: {error}"
                    )])
                }
            }
        }
        None => None,
    };
    let validation_now = chrono::Utc::now().to_rfc3339();
    let validation = crate::product::work_item_plan_compiler::validate_plan_candidate_ir(
        ir,
        &crate::product::work_item_plan_compiler::PlanCandidateValidationContext {
            project_id: &session.project_id,
            issue_id: &session.issue_id,
            plan_id: &session.entity_id,
            source_story_spec_ids: &request.story_spec_ids,
            source_design_spec_ids: &request.design_spec_ids,
            repository_profile: repository_profile.as_ref(),
            // F-51：三生产路径之三（运行期预校验）——同一 plan record options。
            plan_options: &plan.options,
            baseline_tree: baseline_tree.as_ref(),
            existing_work_item_ids: &existing_work_item_ids,
            enrollment_target: enrollment_binding.as_ref().map(|binding| &binding.target),
            member_targets: validation_member_targets.as_ref(),
            now: &validation_now,
        },
    );
    match validation {
        Ok(_) => {}
        Err(diagnostics) => return Some(format_compile_failure_reasons(&diagnostics)),
    }
    // r41 B2(Max 补充):伪编号 fail-closed——contract 校验的 requirement 域
    // 只查 WI 内自洽(design_traceability 自报即过,AI 可编造+登记逃逸);
    // 此处按已确认 story/design 真实 ID 集(story+design 双源提取)比对
    // IR 的 traceability requirement_id,清单外即拒绝(经 A2 教学重驱可
    // 纠一次,再败终态可观测)。
    let mut spec_contexts: Vec<String> = request
        .story_spec_ids
        .iter()
        .filter_map(|id| {
            latest_spec_markdown(lifecycle, &session.project_id, &session.issue_id, id)
        })
        .collect();
    spec_contexts.extend(request.design_spec_ids.iter().filter_map(|id| {
        latest_spec_markdown(lifecycle, &session.project_id, &session.issue_id, id)
    }));
    let registered =
        crate::product::work_item_split_engine::context::extract_registered_requirement_ids(
            &spec_contexts,
        );
    if !registered.is_empty() {
        // r41 B2:traceability requirement_id 伪编号。
        let forged: Vec<String> = ir
            .items
            .iter()
            .flat_map(|item| item.contract.design_traceability.iter())
            .map(|entry| entry.requirement_id.as_str())
            .filter(|id| !registered.iter().any(|registered| registered == id))
            .map(str::to_string)
            .collect();
        // r45 扩 AC:task done_when 的 AC- 引用同域 fail-closed——r45 现场
        // 第五变体(done_when 编造 AC-002/AC-003)证明 AC 是独立命名空间
        // 漏口;产品的 unknown_done_when_ref 只查 WI 内自洽(自报即过),
        // 此处按已确认 story 的真实 AC 集比对,清单外经教学重驱可救一次。
        let forged_done_when: Vec<String> = ir
            .items
            .iter()
            .flat_map(|item| item.contract.tasks.iter())
            .flat_map(|task| task.done_when_refs.iter())
            .filter(|reference| {
                reference.starts_with("AC-")
                    && !registered
                        .iter()
                        .any(|registered| registered == reference.as_str())
            })
            .cloned()
            .collect();
        if !forged.is_empty() || !forged_done_when.is_empty() {
            let mut offenders = forged;
            offenders.extend(forged_done_when);
            return Some(vec![format!(
                "unknown_requirement_ref/unknown_done_when_ref: plan 引用了已确认 spec 之外的编号 [{}];合法编号仅 [{}]",
                offenders.join("、"),
                registered.join("、")
            )]);
        }
    }
    None
}

/// r41 B2:spec 最新版本正文(读失败/无版本返回 None,该校验面不阻断
/// 既有 IR 校验语义)。
fn latest_spec_markdown(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_id: &str,
) -> Option<String> {
    lifecycle
        .list_versions(project_id, issue_id, entity_id)
        .ok()?
        .into_iter()
        .max_by_key(|version| version.version)
        .map(|version| version.markdown)
}

/// 3.6 弱模型基线加固：IR 校验失败的教学重驱 prompt。复用 F2-B 既有 compile
/// 重驱模板（错误原文已逐条回灌 + 立即输出完整 source 指令），附加「修正引用/
/// 补齐字段后重新输出完整 plan」的 IR 修复指令。
fn build_work_item_plan_ir_reredrive_prompt(
    blocking_reasons: &[String],
    registered_requirement_ids: &[String],
) -> String {
    let mut prompt =
        crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt(
            blocking_reasons,
            registered_requirement_ids,
        );
    prompt.push_str(
        "上述为 IR 校验失败（引用或字段不符合契约）。\n\
         修正引用/补齐字段：requirement_refs、done_when_refs、reviewer_check_refs 只能逐字引用本计划已定义 id；\
         每个 criterion_id 必须有配对的 reviewer_check_refs 行。\n",
    );
    // r41 A2/r45 扩 AC:附已登记编号清单(产品侧与 harness 反馈轮对称;
    // story+design 双源 REQ-/NFR-/AC-,与 fresh prompt 的
    // [design_requirements] 同集)。
    if !registered_requirement_ids.is_empty() {
        prompt.push_str(&format!(
            "[design_requirements] {}\n\
             requirement_refs 与 done_when 的引用只能取自上列清单(REQ-*/NFR-*/AC-*),\
             清单外编号一律删除或改为清单内编号。\n",
            registered_requirement_ids.join("、")
        ));
    }
    prompt.push_str("修正后重新输出完整 plan，第一行即 `# Work Item Plan`，不要输出解释。\n");
    prompt
}

/// F2-B/3.6：教学重驱共用驱动——同 node 再驱一次 provider，返回归一化后的
/// markdown source。驱动中断（会话失败/取消）时已按既有终态形态持久化 Failed
/// 并返回 AlreadyFinished，与首轮驱动失败处理一致。
#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)] // 上述枚举是运行收尾的唯一 typed 出口。
async fn drive_single_candidate_reredrive(
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
/// - LC 归属缺失（gateway 已注入但 issue 无 logical_codebase_id）：持久
///   状态不一致，fail-closed `Message`，绝不静默跳过预检回落 legacy 行为。
///
/// 预检通过返回 `Ok(())`：后续仍由调用方读取同一实际文件（无 fallback）。
#[allow(clippy::result_large_err)] // 同上:运行收尾 typed 出口。
fn preflight_lc_member_rules_before_author_run(
    engine: &mut WorkspaceEngine,
    run_context: &ProviderRunContext,
    launch: &super::gateway_start::LogicalPlanLaunch,
) -> Result<(), SingleCandidateProviderRunError> {
    let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        &run_context.app_paths,
        &run_context.session_record.project_id,
        &run_context.session_record.issue_id,
    )
    .map_err(|error| {
        SingleCandidateProviderRunError::Message(format!(
            "resolve logical codebase for admission preflight failed: {error}"
        ))
    })?
    .ok_or_else(|| {
        SingleCandidateProviderRunError::Message(
            "logical session has no logical codebase attribution; admission preflight cannot run"
                .to_string(),
        )
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
            Err(SingleCandidateProviderRunError::AdmissionWaiting {
                reason_code,
                detail,
                missing_materials,
                allowed_actions,
                manual_guidance,
                trust_waiting,
            })
        }
        Err(crate::product::logical_codebase::ProviderAdmissionError::Store(error)) => {
            Err(SingleCandidateProviderRunError::Message(format!(
                "provider admission preflight store error: {error}"
            )))
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
        preflight_lc_member_rules_before_author_run(engine, run_context, lc_plan_launch)?;
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
            issue.logical_codebase_id.is_some(),
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
    // 重驱机会在 compile/IR 两类间共享（first_round_failure 单槽）。
    let compile_context = crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
        target_repository_id: repository.id.clone(),
        member_targets: member_targets.clone(),
    };
    let mut compile_source = full_output;
    let mut first_round_failure: Option<String> = None;
    let candidate_item_count = loop {
        let (ir, repaired_source, _autorepair_log) =
            match crate::product::workspace_engine::contract_autorepair::converge_work_item_plan_source(
                &compile_source,
                compile_context.clone(),
            ) {
                Ok(outcome) => outcome,
                Err(diagnostics) => {
                    let reasons = format_compile_failure_reasons(&diagnostics);
                    if first_round_failure.is_none()
                        && diagnostics
                            .iter()
                            .any(|diagnostic| is_teachable_parse_failure(&diagnostic.code))
                    {
                        first_round_failure = Some(reasons.join("; "));
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
            if first_round_failure.is_none() {
                first_round_failure = Some(reasons.join("; "));
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
                member_targets,
            )
            .await
        {
            Ok(candidate_item_count) => candidate_item_count,
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
    use super::{prepare_author_delivery_for_compile, trim_provider_preamble};
    use crate::product::work_item_plan_compiler::WorkItemPlanSourceContext;

    const FIELD_REP1: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/field-pi-zh-headings-rep1.md"
    ));

    #[test]
    fn trims_provider_preamble_before_document_heading() {
        let source = "我会先读取上下文，再生成计划。\n\n# Work Item Plan\n## Work Item WI-001: x\n";

        assert_eq!(
            trim_provider_preamble(source),
            "# Work Item Plan\n## Work Item WI-001: x\n"
        );
    }

    #[test]
    fn trims_glued_preamble_before_document_heading() {
        let source = "我会先读取上下文，再生成计划。# Work Item Plan\n## Work Item WI-001: x\n";

        assert_eq!(
            trim_provider_preamble(source),
            "# Work Item Plan\n## Work Item WI-001: x\n"
        );
    }

    #[test]
    fn leaves_source_without_preamble_unchanged() {
        let source = "# Work Item Plan\n## Work Item WI-001: x\n";

        assert_eq!(trim_provider_preamble(source), source);
    }

    #[test]
    fn leaves_source_without_document_heading_unchanged() {
        let source = "我会先读取上下文，再生成计划。\n## Work Item WI-001: x\n";

        assert_eq!(trim_provider_preamble(source), source);
    }

    #[test]
    fn trims_code_fence_before_document_heading() {
        let source = "```markdown\n# Work Item Plan\n## Work Item WI-001: x\n```\n";

        assert_eq!(
            trim_provider_preamble(source),
            "# Work Item Plan\n## Work Item WI-001: x\n```\n"
        );
    }

    /// Task 0 实查结论的代码化(修订于 r41 B2):parse 语法类白名单 =
    /// parse 语法类全集(`grammar::DIAGNOSTIC_CODES`)。`unknown_structured_key`
    /// 此前由 contract_autorepair 确定性收敛(F-41)而排除;r41 B2 裁决:
    /// 收敛器对「未知 section 标题」形态 fail-closed 后残余失败应有恰一次
    /// 教学重驱自愈机会(F-48 先例,重驱 prompt 携带 B1 白名单可自纠)。
    /// 语法层新增诊断码时本断言会失败,迫使同步审视白名单。
    #[test]
    fn teachable_parse_failure_codes_are_parse_grammar_minus_converged_codes() {
        use super::{TEACHABLE_PARSE_FAILURE_CODES, is_teachable_parse_failure};
        use crate::product::work_item_plan_compiler::grammar;

        for code in grammar::DIAGNOSTIC_CODES {
            assert!(
                is_teachable_parse_failure(code),
                "parse 语法类全集都应可教学重驱: {code}"
            );
        }
        assert_eq!(
            TEACHABLE_PARSE_FAILURE_CODES.len(),
            grammar::DIAGNOSTIC_CODES.len()
        );
    }

    /// SG6a.2（原 Task 3.5 分流契约迁移）：lowering 类教学资格整体取消
    /// ——缺席/值域统一零重驱（资格只认 parse 白名单码；message 措辞
    /// 不构成资格源，F1.2 口径）。
    #[test]
    fn teachable_lowering_failures_have_zero_repair_once_eligibility() {
        use super::is_teachable_parse_failure;

        for (code, message) in [
            ("lowering_error", "traceability 缺少 requirement_id。"),
            ("lowering_error", "必填 markdown 字段缺失。"),
            ("lowering_error", "intent 必须是 create 或 existing。"),
        ] {
            assert!(
                !is_teachable_parse_failure(code),
                "lowering 类不再有教学重驱资格（message 措辞不构成资格源）: {message}"
            );
        }
        assert!(
            is_teachable_parse_failure("missing_section"),
            "parse 语法类白名单资格不受影响"
        );
    }

    fn compile_item_count(source: &str) -> Result<usize, String> {
        crate::product::work_item_plan_compiler::compile_work_item_plan(
            source,
            &WorkItemPlanSourceContext {
                target_repository_id: "repository_0001".to_string(),
                member_targets: None,
            },
        )
        .map(|ir| ir.items.len())
        .map_err(|diagnostics| {
            diagnostics
                .first()
                .map(|diagnostic| diagnostic.message.clone())
                .unwrap_or_default()
        })
    }

    #[test]
    fn author_delivery_normalizes_field_chinese_headings_and_compiles() {
        // 现场 pi-full rep1 原件 + 前言:author 链路净化后必须可编译。
        let raw = format!("provider preamble\n{FIELD_REP1}");

        let delivery = prepare_author_delivery_for_compile(&raw);

        assert_eq!(delivery.normalized_heading_lines, 15);
        assert!(delivery.source.starts_with("# Work Item Plan\n"));
        assert_eq!(
            compile_item_count(&delivery.source).expect("归一化后必须编译通过"),
            1
        );
    }

    #[test]
    fn author_delivery_leaves_english_source_unchanged() {
        let source =
            "# Work Item Plan\n## Work Item WI-001: x\n### Identity\n- schema_version: 1\n";

        let delivery = prepare_author_delivery_for_compile(source);

        assert_eq!(delivery.normalized_heading_lines, 0);
        assert_eq!(delivery.source, source);
    }

    #[test]
    fn author_delivery_keeps_unknown_headings_fail_closed() {
        // 表外中文标题不改写,仍由 compiler fail-closed 拒绝。
        let source = "# 工作项计划\n## 工作项 WI-001: x\n### 溯源清单\n";

        let delivery = prepare_author_delivery_for_compile(source);

        assert_eq!(delivery.normalized_heading_lines, 2);
        assert!(delivery.source.contains("### 溯源清单\n"));
        assert!(compile_item_count(&delivery.source).is_err());
    }

    #[test]
    fn author_heading_normalized_event_id_keeps_attempt_history_on_same_node() {
        // 同一 AuthorRun 节点被重驱(中断恢复/重试)时,两次不同尝试的归一化
        // 审计不得共用 event_id——execution event 按 event_id upsert,共用会让
        // 后一次覆盖前一次,丢审计历史。
        let node_id = "timeline_node_001";
        let first = super::author_heading_normalized_event_id(node_id, "attempt one output");
        let second = super::author_heading_normalized_event_id(node_id, "attempt two output");

        assert_ne!(
            first, second,
            "同节点不同尝试共用 event_id 会让 upsert 覆盖审计历史"
        );
        assert_eq!(
            first,
            super::author_heading_normalized_event_id(node_id, "attempt one output"),
            "同内容重放必须保持 event_id 稳定以幂等去重"
        );
        assert!(first.starts_with("single_candidate_heading_normalized_timeline_node_001"));
    }
}

#[cfg(test)]
mod r41_teachable_and_ir_prompt_tests {
    use super::*;

    /// r41 B2:unknown_structured_key 属可教面(一次自愈机会,F-48 先例)。
    #[test]
    fn unknown_structured_key_is_teachable() {
        assert!(is_teachable_parse_failure("unknown_structured_key"));
        assert!(is_teachable_parse_failure("missing_section"));
        assert!(!is_teachable_parse_failure("internal_error"));
    }

    /// r41 A2:IR 重驱 prompt 附已登记需求编号清单与约束指令。
    #[test]
    fn ir_reredrive_prompt_appends_registered_requirement_ids() {
        let reasons = vec![
            "unknown_requirement_ref: task references unknown design requirement NFR-1".to_string(),
        ];
        let ids = vec!["REQ-ENV-01".to_string(), "NFR-PERF-02".to_string()];
        let prompt = build_work_item_plan_ir_reredrive_prompt(&reasons, &ids);
        assert!(prompt.contains("[design_requirements] REQ-ENV-01、NFR-PERF-02"));
        assert!(prompt.contains("清单外编号一律删除或改为清单内编号"));
    }

    /// r45 扩 AC:done_when 的 AC 引用清单外被拒(第五变体锚点:
    /// AC-002/AC-003 编造;产品 unknown_done_when_ref 只查 WI 内自洽)。
    #[test]
    fn ir_reredrive_prompt_lists_ac_namespace() {
        let reasons = vec![
            "unknown_done_when_ref: task references unknown acceptance criterion AC-002"
                .to_string(),
        ];
        let ids = vec!["REQ-ENV-01".to_string(), "AC-001".to_string()];
        let prompt = build_work_item_plan_ir_reredrive_prompt(&reasons, &ids);
        assert!(prompt.contains("AC-001"));
        assert!(
            prompt.contains("done_when 的引用只能取自上列清单"),
            "清单约束必须覆盖 done_when/AC 命名空间: {prompt}"
        );
    }

    /// r41 A2:清单空时不注入(无 spec 上下文的老会话零变化)。
    #[test]
    fn ir_reredrive_prompt_omits_empty_requirement_list() {
        let reasons = vec!["acceptance_criterion_without_reviewer_check: WI-001 AC-1".to_string()];
        let prompt = build_work_item_plan_ir_reredrive_prompt(&reasons, &[]);
        assert!(!prompt.contains("[design_requirements]"));
    }
}
