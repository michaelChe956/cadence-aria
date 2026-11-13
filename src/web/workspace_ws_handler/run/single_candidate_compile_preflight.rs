//! SC compile 失败重驱的资格判定、诊断格式化与预检辅助面，自 single_candidate.rs 拆出以守 1200 行文件守护（先例：single_candidate_baseline_tree.rs / single_candidate_delivery.rs / single_candidate_structure_reredrive.rs）。
use super::*;
// 指令行条数上限与截断函数的唯一定义在 contract_autorepair（handler 终态
// error / 指令事件 / completion message / 教学重驱 prompt 共用同一口径）。
use crate::product::workspace_engine::contract_autorepair::cap_autorepair_instruction_lines;

/// 可教学重驱的 compile parse 语法类诊断码（码表保留，供
/// [`teaching_verdict`] 收集 `parse_reasons`；资格判定整体见该函数）。
///
/// 集合来源（Task 0 实查）：`work_item_plan_compiler::parse` 的语法类诊断码全集
/// 即 `grammar::DIAGNOSTIC_CODES`（missing_section / unknown_structured_key /
/// invalid_ears / invalid_work_item_id）。其余 parse 码在编译 loop 现状下
/// 无任何自动返修面（F-48：invalid_ears 因此直达终态），教学重驱是其
/// 唯一一次自愈机会。
pub(super) const TEACHABLE_PARSE_FAILURE_CODES: [&str; 4] = [
    "missing_section",
    "invalid_ears",
    "invalid_work_item_id",
    "unknown_structured_key",
];

/// 该诊断码是否属于可教学重驱的 parse 语法类（parse_reasons 收集用）。
pub(crate) fn is_teachable_parse_failure(code: &str) -> bool {
    TEACHABLE_PARSE_FAILURE_CODES.contains(&code)
}

/// Task 4（MODIFIED REQ-WSC-09）：教学重驱资格的 typed 载荷集合——只有
/// 失败集合里存在可定位、可回喂的 typed 事实（parse 码表 / lowering
/// fault / IR fault）才授予恰一次教学重驱。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TeachingPayload {
    /// parse 语法类（码表 4 项）事实块回灌（首行 code:line:message 逐字
    /// + 精确 field/repair_example 行；有界投影在渲染侧）。
    pub parse_reasons: Vec<String>,
    /// lowering typed fault（构造点显式写入、事实齐全）。
    pub lowering_faults: Vec<crate::product::work_item_plan_compiler::LoweringFault>,
    /// IR typed 载荷（行在场 + 合法集合非空）。
    pub ir_faults: Vec<crate::product::work_item_plan_compiler::IrFault>,
}

/// Task 4：教学资格判定结果；`reason` 为稳定判据码
/// （mixed_operational_set / no_typed_fault / ir_fault_not_locatable）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TeachingVerdict {
    Eligible { payload: TeachingPayload },
    NotEligible { reason: &'static str },
}

/// 教学资格判定（钉死规则，测试逐条对应）：
/// - `preflight_operational_failures` 非空 ⇒ `NotEligible{mixed_operational_set}`
///   （禁止 `any(可教)` 放行）；
/// - parse 码表命中 ⇒ 收 `parse_reasons`（首行三段逐字 + field/example 行）；
/// - `lowering_fault==Some` 且 field 非空、行>0、期望形态非空 ⇒ 收集
///   （`line==0` 视为不可定位，不收）；
/// - `ir_fault==Some` 且 `source_line==Some`、`legal_refs` 非空 ⇒ 收集
///   （否则该条不授资格）；
/// - 三者并集非空 ⇒ `Eligible`。
///
/// 「恰好一次」仍以前置授权、合法 provider 与剩余机会为条件；槽位机制零
/// 改动（SG6a.2 的 `is_lowering_absence_message` 措辞旁路已按 F1.2 删除，
/// 本函数不得以任何形式恢复措辞/前缀获权）。
pub(super) fn teaching_verdict(
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
    preflight_operational_failures: &[String],
) -> TeachingVerdict {
    if !preflight_operational_failures.is_empty() {
        return TeachingVerdict::NotEligible {
            reason: "mixed_operational_set",
        };
    }
    let mut payload = TeachingPayload {
        parse_reasons: diagnostics
            .iter()
            .filter(|diagnostic| is_teachable_parse_failure(&diagnostic.code))
            .map(|diagnostic| {
                let mut block = format!(
                    "{}:{}:{}",
                    diagnostic.code, diagnostic.line, diagnostic.message
                );
                if !diagnostic.field.is_empty() {
                    block.push_str(&format!("\n  缺失字段: {}", diagnostic.field));
                }
                if !diagnostic.repair_example.is_empty() {
                    block.push_str(&format!("\n  正确形状: {}", diagnostic.repair_example));
                }
                block
            })
            .collect(),
        lowering_faults: Vec::new(),
        ir_faults: Vec::new(),
    };
    let mut saw_unlocatable_ir_fault = false;
    for diagnostic in diagnostics {
        if let Some(fault) = locatable_lowering_fault(diagnostic) {
            payload.lowering_faults.push(fault.clone());
        }
        if let Some(fault) = diagnostic.ir_fault.as_ref() {
            if fault.source_line.is_some() && !fault.legal_refs.is_empty() {
                payload.ir_faults.push(fault.clone());
            } else {
                saw_unlocatable_ir_fault = true;
            }
        }
    }
    if payload.parse_reasons.is_empty()
        && payload.lowering_faults.is_empty()
        && payload.ir_faults.is_empty()
    {
        return TeachingVerdict::NotEligible {
            reason: if saw_unlocatable_ir_fault {
                "ir_fault_not_locatable"
            } else {
                "no_typed_fault"
            },
        };
    }
    TeachingVerdict::Eligible { payload }
}

/// lowering typed fault 的可收集判据：field 非空、行>0（BranchKeyMismatch
/// 取 fault 自带行；RequiredFieldAbsent 取诊断行——字段完全缺席时无行可
/// 定位即为 0，不收）、期望形态非空。
fn locatable_lowering_fault(
    diagnostic: &crate::product::work_item_plan_compiler::CompilerDiagnostic,
) -> Option<&crate::product::work_item_plan_compiler::LoweringFault> {
    use crate::product::work_item_plan_compiler::LoweringFault;

    let fault = diagnostic.lowering_fault.as_ref()?;
    let locatable = match fault {
        LoweringFault::BranchKeyMismatch {
            expected_field,
            line,
            expected_shape,
            ..
        } => !expected_field.is_empty() && *line > 0 && !expected_shape.is_empty(),
        LoweringFault::RequiredFieldAbsent {
            field,
            expected_shape,
        } => !field.is_empty() && diagnostic.line > 0 && !expected_shape.is_empty(),
    };
    locatable.then_some(fault)
}

// 丢弃 provider 在 markdown 文档标题前输出的前言（实现见 single_candidate_delivery）。

/// SC author 交付进入 compiler 前的确定性净化（结构标题归一化 + EARS 关键字
/// 空白归一化 + 前言修剪）。
///
/// 归一化先于修剪：前言修剪锚定的是规范英文文档标题，provider 输出「前言 +
/// 中文标题」时必须先把标题归一化才能锚定修剪。两处归一化都只吸收确定性
/// 抖动（固定词表标题翻译 / 关键字邻位空白），表外未知标题与正文不改，由
/// compiler fail-closed。
pub(super) fn prepare_author_delivery_for_compile(
    raw: &str,
) -> crate::product::work_item_plan_compiler::NormalizedPlanSource {
    let normalized =
        crate::product::work_item_plan_compiler::normalize_delivery_before_compile(raw);
    crate::product::work_item_plan_compiler::NormalizedPlanSource {
        source: single_candidate_delivery::trim_provider_preamble(&normalized.source).to_string(),
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
pub(super) async fn emit_author_heading_normalized_event(
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
pub(super) async fn emit_author_ears_normalized_event(
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
pub(super) fn format_compile_failure_reasons(
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

/// O1（裁决二）× Task 3 评审 Minor-3 × Task 4 评审 Important-1：compile
/// 失败路径的失败面拼装——诊断原文 + 收窄指令（未知 key 迁移/重复
/// trusted command）有界渲染，随教学重驱 prompt 与终态错误同源呈现，
/// 平台不应用、不丢弃。
///
/// 返回 `(reasons, instruction_lines)`：`instruction_lines` 为**未截断**的
/// 指令行（教学重驱 prompt 侧由 `cap_autorepair_instruction_lines` 单点
/// 截断，摘要写真总数）；终态 reasons 面在此处截断（同函数同口径）。
pub(super) fn compile_failure_surface(
    source: &str,
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
    context: &crate::product::work_item_plan_compiler::WorkItemPlanSourceContext,
) -> (Vec<String>, Vec<String>) {
    let instruction_lines =
        crate::product::workspace_engine::contract_autorepair::format_contract_autorepair_instructions(
            &crate::product::workspace_engine::contract_autorepair::build_compile_failure_repair_instructions(
                source, diagnostics, context,
            ),
        );
    let mut reasons = format_compile_failure_reasons(diagnostics);
    reasons.extend(cap_autorepair_instruction_lines(instruction_lines.clone()));
    (reasons, instruction_lines)
}

/// O1（裁决二）：缺口机器指令不应用、不丢弃——渲染进 owner 可见执行
/// 事件随失败面呈现（与教学重驱 prompt 同源的呈现面）；缺口本体沿
/// 引擎既有机械 verdict 通道（F5-A）交原授权 author 返修。prompt 携带
/// 指令文本驱动教学重驱的完整接线在 Task 4。
pub(super) async fn emit_contract_autorepair_instructions_event(
    engine: &mut WorkspaceEngine,
    node_id: &str,
    author_provider: &ProviderName,
    instructions: &[crate::product::workspace_engine::contract_prerevision::StructuralRepairInstruction],
) {
    if instructions.is_empty() {
        return;
    }
    engine
        .emit_execution_event(
            ProviderExecutionEvent {
                event_id: format!("{node_id}_contract_autorepair_instructions"),
                kind: ProviderExecutionEventKind::Output,
                status: ProviderExecutionEventStatus::Started,
                title: "SC 契约缺口 author 修复指令（O1：平台不改写 source）".to_string(),
                detail: Some(format!(
                    "确定性契约缺口产出 {} 条机器指令，交原授权 author 在保护闭包内产出新 revision",
                    instructions.len()
                )),
                command: None,
                cwd: None,
                output: Some(
                    cap_autorepair_instruction_lines(
                        crate::product::workspace_engine::contract_autorepair::format_contract_autorepair_instructions(
                            instructions,
                        ),
                    )
                    .join("\n"),
                ),
                exit_code: None,
            },
            Some(node_id.to_string()),
            Some(author_provider.clone()),
        )
        .await;
}

/// Task 4（MODIFIED REQ-WSC-09）：`prevalidate_plan_candidate_ir` 的 typed
/// 分类返回（替代旧 `Option<Vec<String>>`「Some 即重驱」口径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreflightVerdict {
    /// 校验失败诊断携带的 typed IR 载荷（已过 `attach_source_lines` 经
    /// AST span 补行；不可定位者保持 `None`，不授教学资格）。
    pub ir_faults: Vec<crate::product::work_item_plan_compiler::IrFault>,
    /// plan 读取/基线/成员解析失败（2.7 判 Operational；非空 ⇒ 零重驱，
    /// 禁止 `any(可教)` 放行）。
    pub operational_failures: Vec<String>,
    /// 校验失败原文（code:line:message 呈现回灌，供终态消息与 IR 重驱
    /// prompt；**非资格判据**，资格只认 typed 事实）。
    pub failure_reasons: Vec<String>,
}

impl PreflightVerdict {
    /// 是否存在任何失败（typed / operational / 呈现原文）。
    pub(super) fn has_failures(&self) -> bool {
        !(self.failure_reasons.is_empty()
            && self.operational_failures.is_empty()
            && self.ir_faults.is_empty())
    }

    /// 终态/呈现面：operational 与校验失败原文同序回显（非资格判据）。
    pub(super) fn presentation_reasons(&self) -> Vec<String> {
        let mut reasons = self.operational_failures.clone();
        reasons.extend(self.failure_reasons.iter().cloned());
        reasons
    }

    /// 教学资格（[`teaching_verdict`] 同一钉死规则在预检侧的投影）：
    /// operational 非空 ⇒ 零资格；可定位 IR fault（行在场 + 合法集合
    /// 非空）非空 ⇒ Eligible；否则 NotEligible（存在 IR fault 但全部
    /// 不可定位时报 `ir_fault_not_locatable`）。
    pub(super) fn to_teaching_verdict(&self) -> TeachingVerdict {
        if !self.operational_failures.is_empty() {
            return TeachingVerdict::NotEligible {
                reason: "mixed_operational_set",
            };
        }
        let mut saw_ir_fault = false;
        let mut ir_faults = Vec::new();
        for fault in &self.ir_faults {
            saw_ir_fault = true;
            if fault.source_line.is_some() && !fault.legal_refs.is_empty() {
                ir_faults.push(fault.clone());
            }
        }
        if ir_faults.is_empty() {
            return TeachingVerdict::NotEligible {
                reason: if saw_ir_fault {
                    "ir_fault_not_locatable"
                } else {
                    "no_typed_fault"
                },
            };
        }
        TeachingVerdict::Eligible {
            payload: TeachingPayload {
                parse_reasons: Vec::new(),
                lowering_faults: Vec::new(),
                ir_faults,
            },
        }
    }
}

/// 3.6 弱模型基线加固 × Task 4 typed 化：IR 预检——在持久化前跑与
/// `complete_single_candidate_work_item_plan_author` 内部同源的
/// `validate_plan_candidate_ir`（上下文组装同源：plan 的 source spec ids 与
/// repository profile），失败分类为 [`PreflightVerdict`]：typed IR 载荷
/// （经 `attach_source_lines` 补行）、operational 失败（plan 读取/基线/
/// 成员解析——原 `.ok()?` 静默吞掉的路径显式记录，不触发任何重驱）与
/// 呈现原文（非资格判据）。资格判定见 `to_teaching_verdict`；权威路径
/// 仍以既有 fail-closed 终态兜底。
pub(super) fn prevalidate_plan_candidate_ir(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    request: &crate::web::types::GenerateWorkItemsRequest,
    ir: &crate::product::work_item_plan_compiler::PlanCandidateIr,
    source: &str,
) -> PreflightVerdict {
    let session = engine.session();
    let operational_only = |failures: Vec<String>| PreflightVerdict {
        ir_faults: Vec::new(),
        operational_failures: failures,
        failure_reasons: Vec::new(),
    };
    let Ok(plan) = lifecycle.get_issue_work_item_plan(
        &session.project_id,
        &session.issue_id,
        &session.entity_id,
    ) else {
        return operational_only(vec!["load plan for prevalidation failed".to_string()]);
    };
    let mut operational_failures = Vec::new();
    let repository_profile = plan
        .repository_profile_ref
        .as_deref()
        .and_then(|profile_id| {
            match lifecycle.get_repository_profile(
                &session.project_id,
                &session.issue_id,
                profile_id,
            ) {
                Ok(profile) => Some(profile),
                Err(error) => {
                    operational_failures.push(format!("load repository profile failed: {error}"));
                    None
                }
            }
        });
    // F-56→REQ-PIB-03：运行期预校验与权威路径共用基线加载（按分支名取树）。
    // Task 4（2.7）：基线不可解析 → 记 operational（混有 operational ⇒ 零
    // 重驱）；预检以无基线形态继续收集其余失败，使失败集合完整可见，
    // 权威路径仍以 fail-closed 终态兜底。
    let baseline_tree = match crate::product::workspace_engine::plan_preflight::plan_baseline_tree(
        lifecycle,
        &session.project_id,
        &session.issue_id,
    ) {
        Ok(baseline_tree) => baseline_tree,
        Err(message) => {
            operational_failures.push(message);
            None
        }
    };
    // C1 Task 5（REQ-C1-PLAN-01）：existing 意图的授权解析面（读失败记
    // operational；existing 集缺席时校验上下文不完整，不跑校验）。
    let existing_work_item_ids =
        match lifecycle.list_work_items(&session.project_id, &session.issue_id) {
            Ok(records) => records
                .into_iter()
                .map(|record| record.id)
                .collect::<Vec<_>>(),
            Err(error) => {
                operational_failures.push(format!("list existing work items failed: {error}"));
                return PreflightVerdict {
                    ir_faults: Vec::new(),
                    operational_failures,
                    failure_reasons: Vec::new(),
                };
            }
        };
    // 批 2 R1（REQ-WIGA-01）：enrolled 会话传真实 binding+成员映射（装载
    // 失败记 operational，不触发任何重驱）；Manual/非 enrolled 保持
    // None 零回归。
    let enrollment_binding =
        match engine.current_enrollment_binding_for_compile(lifecycle, &session.entity_id) {
            Ok(binding) => binding,
            Err(error) => {
                operational_failures.push(format!(
                    "load enrollment binding for prevalidation failed: {error}"
                ));
                return PreflightVerdict {
                    ir_faults: Vec::new(),
                    operational_failures,
                    failure_reasons: Vec::new(),
                };
            }
        };
    let validation_member_targets = match &enrollment_binding {
        Some(_) => {
            let session_record = match lifecycle.get_workspace_session(&session.session_id) {
                Ok(record) => record,
                Err(error) => {
                    operational_failures.push(format!(
                        "load plan session for member targets failed: {error}"
                    ));
                    return PreflightVerdict {
                        ir_faults: Vec::new(),
                        operational_failures,
                        failure_reasons: Vec::new(),
                    };
                }
            };
            let repository = match workspace_repository_for_session(
                &lifecycle.app_paths(),
                lifecycle,
                &session_record,
            ) {
                Ok(resolved) => resolved,
                Err(error) => {
                    operational_failures.push(format!(
                        "resolve plan session repository for member targets failed: {error}"
                    ));
                    return PreflightVerdict {
                        ir_faults: Vec::new(),
                        operational_failures,
                        failure_reasons: Vec::new(),
                    };
                }
            };
            match crate::product::workspace_engine::draft_batch::compile_support::plan_compile_member_targets(
                lifecycle, &plan, &repository,
            ) {
                Ok(targets) => targets,
                Err(error) => {
                    operational_failures.push(format!(
                        "resolve plan member targets for prevalidation failed: {error}"
                    ));
                    return PreflightVerdict {
                        ir_faults: Vec::new(),
                        operational_failures,
                        failure_reasons: Vec::new(),
                    };
                }
            }
        }
        None => None,
    };
    // REQ-WSC-10（写范围×被授权仓归属）：enrolled 装载成员 checkout 前缀表（失败记 operational；单仓保持 None 零回归）。
    let eb = enrollment_binding.as_ref();
    let mc_prefixes = match member_checkout_prefixes::for_validation(lifecycle, &plan, eb) {
        Ok(prefixes) => prefixes,
        Err(e) => {
            operational_failures.push(format!("member checkout prefixes: {e}"));
            return PreflightVerdict {
                ir_faults: Vec::new(),
                operational_failures,
                failure_reasons: Vec::new(),
            };
        }
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
            member_checkout_prefixes: mc_prefixes.as_ref(),
            now: &validation_now,
        },
    );
    let mut ir_faults = Vec::new();
    let mut failure_reasons = Vec::new();
    match validation {
        Ok(_) => {}
        Err(diagnostics) => {
            ir_faults = diagnostics
                .iter()
                .filter_map(|diagnostic| diagnostic.ir_fault.clone())
                .collect();
            // Task 4：行号经编译器同一解析器的 AST span 补全（不是 message
            // 反解析）；source 中无该行保持 None，不授资格。
            crate::product::work_item_plan_compiler::attach_source_lines(&mut ir_faults, source);
            failure_reasons = format_compile_failure_reasons(&diagnostics);
        }
    }
    // r41 B2(Max 补充):伪编号 fail-closed——contract 校验的 requirement 域
    // 只查 WI 内自洽(design_traceability 自报即过,AI 可编造+登记逃逸);
    // 此处按已确认 story/design 真实 ID 集(story+design 双源提取)比对
    // IR 的 traceability requirement_id,清单外即拒绝。Task 4 typed 口径:
    // 该分支无 typed 事实(不伪造 IrFault),只进呈现原文——不构成教学
    // 资格,由既有终态面收口。
    if failure_reasons.is_empty() {
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
            // 此处按已确认 story 的真实 AC 集比对,清单外拒绝。
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
                failure_reasons.push(format!(
                    "unknown_requirement_ref/unknown_done_when_ref: plan 引用了已确认 spec 之外的编号 [{}];合法编号仅 [{}]",
                    offenders.join("、"),
                    registered.join("、")
                ));
            }
        }
    }
    PreflightVerdict {
        ir_faults,
        operational_failures,
        failure_reasons,
    }
}

/// 3.3（REQ-WSC-25 序列）：等价规范化审计落库——generate 段 handler 是
/// 唯一写入点（completion 复验侧不写）。幂等键 = node + canonicalization
/// 内容摘要（store 按会话+attempt_key 判重，同内容重放零膨胀）；写失败
/// 只降级记日志，不得使候选生成失败（与流水 store 的失败语义一致）。
pub(super) fn record_equivalence_canonicalizations(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    node_id: &str,
    canonicalizations: &[crate::product::lifecycle_store::candidate_attempts::EquivalenceCanonicalization],
) {
    use crate::product::lifecycle_store::candidate_attempts::{
        CandidateAttemptOutcome, CandidateAttemptRecord, CandidateAttemptStage,
    };
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    for entry in canonicalizations {
        digest.update(entry.kind.as_bytes());
        digest.update(entry.original_key.as_bytes());
        digest.update(entry.value.as_bytes());
        digest.update(entry.resolution_basis.as_bytes());
    }
    let digest = hex::encode(digest.finalize());
    let record = CandidateAttemptRecord {
        attempt_seq: 0,
        attempt_key: format!("equiv/{node_id}/{}", &digest[..16]),
        session_id: String::new(),
        issue_id: String::new(),
        recorded_at: chrono::Utc::now().to_rfc3339(),
        stage: CandidateAttemptStage::Final,
        outcome: CandidateAttemptOutcome::Passed,
        source_revision_ref: None,
        plan_candidate_ir_ref: None,
        failure: None,
        auto_rewrite: None,
        auto_repairs_applied: Vec::new(),
        equivalence_canonicalizations: canonicalizations.to_vec(),
        provider_attempt: None,
        source_missing_reason: None,
    };
    let session_id = engine.session().session_id.clone();
    if let Err(error) = lifecycle.append_candidate_attempt(&session_id, &record) {
        tracing::warn!(
            session_id = %session_id,
            %error,
            "append equivalence canonicalization audit record failed (degraded, flow continues)"
        );
    }
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
/// 补齐字段后重新输出完整 plan」的 IR 修复指令（签名不变，供既有单测与
/// `ir_prompt_tests` 复用）。Task 4 后生产调用点已切
/// `build_preflight_ir_reredrive_prompt`，本入口仅测试面消费。
#[cfg(test)]
pub(super) fn build_work_item_plan_ir_reredrive_prompt(
    blocking_reasons: &[String],
    registered_requirement_ids: &[String],
) -> String {
    append_ir_repair_instructions(
        crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt(
            blocking_reasons,
            &[],
            &[],
            &[],
            registered_requirement_ids,
        ),
        registered_requirement_ids,
    )
}

/// Task 4：preflight typed 判定面的 IR 重驱 prompt——校验失败原文
/// （code:line:message 呈现回灌）+ typed IR 载荷（引用/合法集合/期望形态）
/// 并入同一模板，再附既有 IR 修复指令。
pub(super) fn build_preflight_ir_reredrive_prompt(
    preflight: &PreflightVerdict,
    registered_requirement_ids: &[String],
) -> String {
    append_ir_repair_instructions(
        crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt(
            &preflight.failure_reasons,
            &[],
            &preflight.ir_faults,
            &[],
            registered_requirement_ids,
        ),
        registered_requirement_ids,
    )
}

/// IR 重驱 prompt 的既有修复指令尾段（两个入口共用，措辞逐字保持）。
fn append_ir_repair_instructions(
    mut prompt: String,
    registered_requirement_ids: &[String],
) -> String {
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

/// Task 4：compile 失败路径的 typed 重驱 prompt——parse 码表回灌 +
/// lowering/IR typed 载荷 + 有界指令行，经同一模板渲染（typed 事实
/// 四元组直入，不经过 message 文本）。
pub(super) fn build_typed_compile_reredrive_prompt(
    payload: &TeachingPayload,
    autorepair_instructions: &[String],
    registered_requirement_ids: &[String],
) -> String {
    crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt(
        &payload.parse_reasons,
        &payload.lowering_faults,
        &payload.ir_faults,
        autorepair_instructions,
        registered_requirement_ids,
    )
}

#[cfg(test)]
mod tests {
    use super::prepare_author_delivery_for_compile;
    use crate::product::work_item_plan_compiler::WorkItemPlanSourceContext;

    const FIELD_REP1: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/field-pi-zh-headings-rep1.md"
    ));

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

    /// Task 4（MODIFIED REQ-WSC-09，改写自旧「lowering 零资格」口径）：
    /// lowering 类资格只认 typed fault——无 fault（message 含「缺少」措辞）
    /// 或 fault 不可定位（诊断行 0）都不收集；事实齐全才 Eligible。
    #[test]
    fn teaching_verdict_collects_only_locatable_typed_lowering_faults() {
        use super::{TeachingVerdict, teaching_verdict};
        use crate::product::work_item_plan_compiler::types::{CompilerDiagnostic, LoweringFault};

        fn diagnostic(line: usize, fault: Option<LoweringFault>) -> CompilerDiagnostic {
            CompilerDiagnostic {
                code: "lowering_error".to_string(),
                line,
                field: "contract.intent.intent_target_codebase".to_string(),
                message: "Plan Intent 声明在场时该字段必填。".to_string(),
                repair_example: "- intent_target_codebase: value".to_string(),
                kind: None,
                lowering_fault: fault,
                ir_fault: None,
            }
        }
        // 措辞近似但无 typed fault：不获资格（RF② 判定函数面）。
        assert!(matches!(
            teaching_verdict(&[diagnostic(3, None)], &[]),
            TeachingVerdict::NotEligible {
                reason: "no_typed_fault"
            }
        ));
        // fault 在场但诊断行 0（不可定位）：不收集。
        assert!(matches!(
            teaching_verdict(
                &[diagnostic(
                    0,
                    Some(LoweringFault::RequiredFieldAbsent {
                        field: "intent_target_codebase",
                        expected_shape: "- intent_target_codebase: value",
                    })
                )],
                &[]
            ),
            TeachingVerdict::NotEligible {
                reason: "no_typed_fault"
            }
        ));
        // 事实齐全（field 非空、行>0、期望形态非空）：恰收集该 fault。
        match teaching_verdict(
            &[diagnostic(
                7,
                Some(LoweringFault::RequiredFieldAbsent {
                    field: "intent_target_codebase",
                    expected_shape: "- intent_target_codebase: value",
                }),
            )],
            &[],
        ) {
            TeachingVerdict::Eligible { payload } => {
                assert_eq!(payload.parse_reasons.len(), 0);
                assert_eq!(payload.lowering_faults.len(), 1);
                assert_eq!(payload.ir_faults.len(), 0);
            }
            TeachingVerdict::NotEligible { reason } => {
                panic!("事实齐全的 typed lowering fault 必须获恰一次资格: {reason}")
            }
        }
    }

    /// Task 4（RF② 判定函数面）：code 带可教前缀但不在码表、message 措辞
    /// 近似——都不构成资格源，零 typed fault ⇒ NotEligible。
    #[test]
    fn teaching_verdict_rejects_wording_or_prefix_without_typed_fault() {
        use super::{TeachingVerdict, teaching_verdict};
        use crate::product::work_item_plan_compiler::CompilerDiagnostic;

        fn diagnostic(code: &str, message: &str) -> CompilerDiagnostic {
            CompilerDiagnostic {
                code: code.to_string(),
                line: 3,
                field: "Traceability.requirement_id".to_string(),
                message: message.to_string(),
                repair_example: "- requirement_id: REQ-001".to_string(),
                kind: None,
                lowering_fault: None,
                ir_fault: None,
            }
        }
        let diagnostics = [
            diagnostic("lowering_error", "traceability 缺少 requirement_id。"),
            diagnostic("missing_section_extra", "前缀近似码不在 parse 码表。"),
        ];
        assert!(matches!(
            teaching_verdict(&diagnostics, &[]),
            TeachingVerdict::NotEligible {
                reason: "no_typed_fault"
            }
        ));
        // 对照：parse 码表码仍在（parse 语法类通道不受 typed 化影响）。
        assert!(matches!(
            teaching_verdict(
                &[diagnostic("invalid_ears", "statement 必须符合 EARS 模板。")],
                &[]
            ),
            TeachingVerdict::Eligible { .. }
        ));
    }

    /// Task 4（RF① 判定函数面）：operational 混入 ⇒ 零资格，禁止 any(可教)
    /// 放行——即便 parse 码表码同时在场。
    #[test]
    fn teaching_verdict_rejects_mixed_operational_set() {
        use super::{TeachingVerdict, teaching_verdict};
        use crate::product::work_item_plan_compiler::CompilerDiagnostic;

        let diagnostics = [CompilerDiagnostic {
            code: "missing_section".to_string(),
            line: 2,
            field: "Goal".to_string(),
            message: "Work Item 缺少必需 section。".to_string(),
            repair_example: "### Goal".to_string(),
            kind: None,
            lowering_fault: None,
            ir_fault: None,
        }];
        assert!(matches!(
            teaching_verdict(
                &diagnostics,
                &["无法推断默认基准分支：仓库无 main/master 本地分支".to_string()]
            ),
            TeachingVerdict::NotEligible {
                reason: "mixed_operational_set"
            }
        ));
    }

    /// Task 4：IR fault 不可定位（source_line=None 或 legal_refs 空）不授
    /// 资格；可定位（行在场 + 合法集合非空）才进入 payload。
    #[test]
    fn teaching_verdict_marks_unlocatable_ir_fault_not_eligible() {
        use super::{TeachingVerdict, teaching_verdict};
        use crate::product::work_item_plan_compiler::{
            CompilerDiagnostic, IrFault, IrFaultKind, IrFaultLocator,
        };

        fn ir_diagnostic(fault: IrFault) -> CompilerDiagnostic {
            CompilerDiagnostic {
                code: "unknown_requirement_ref".to_string(),
                line: fault.source_line.unwrap_or(0),
                field: "Tasks.requirement_refs".to_string(),
                message: "task references unknown design requirement REQ-002".to_string(),
                repair_example: "- requirement_refs: REQ-001".to_string(),
                kind: None,
                lowering_fault: None,
                ir_fault: Some(fault),
            }
        }
        fn fault(source_line: Option<usize>, legal_refs: Vec<&str>) -> IrFault {
            IrFault {
                kind: IrFaultKind::UnknownRequirementRef,
                work_item_id: "WI-001".to_string(),
                locator: IrFaultLocator::Task {
                    task_id: "TASK-001".to_string(),
                },
                offending_ref: "REQ-002".to_string(),
                legal_refs: legal_refs.into_iter().map(str::to_string).collect(),
                source_line,
                expected_shape: "- requirement_refs: REQ-001".to_string(),
            }
        }
        // 无行：不可定位，不授资格。
        assert!(matches!(
            teaching_verdict(&[ir_diagnostic(fault(None, vec!["REQ-001"]))], &[]),
            TeachingVerdict::NotEligible {
                reason: "ir_fault_not_locatable"
            }
        ));
        // 有行但合法集合空：同样不可回喂。
        assert!(matches!(
            teaching_verdict(&[ir_diagnostic(fault(Some(30), vec![]))], &[]),
            TeachingVerdict::NotEligible {
                reason: "ir_fault_not_locatable"
            }
        ));
        // 行 + 合法集合齐全：收集进 payload。
        match teaching_verdict(&[ir_diagnostic(fault(Some(30), vec!["REQ-001"]))], &[]) {
            TeachingVerdict::Eligible { payload } => assert_eq!(payload.ir_faults.len(), 1),
            TeachingVerdict::NotEligible { reason } => {
                panic!("可定位 IR fault 必须获恰一次资格: {reason}")
            }
        }
    }

    /// Task 3 评审 Minor-3：指令行进 owner 可见面（终态错误/指令事件/
    /// typed 重驱 prompt）前必须截断——条数上限 + 计数摘要收口。
    #[test]
    fn instruction_lines_are_capped_before_owner_visible_surfaces() {
        let lines: Vec<String> = (0..10)
            .map(|index| format!("rule-{index}: 修复动作"))
            .collect();
        let capped = super::cap_autorepair_instruction_lines(lines);
        assert_eq!(
            capped.len(),
            crate::product::workspace_engine::contract_autorepair::MAX_AUTOREPAIR_INSTRUCTION_LINES
                + 1,
            "上限条数 + 一行计数摘要: {capped:?}"
        );
        assert!(capped[3].contains("rule-3"), "上限内的指令必须在场");
        assert!(
            capped.last().expect("summary line").contains("共 10 条"),
            "截断必须以计数摘要显式收口: {capped:?}"
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
    /// A1（REQ-WSC-32）：同 message、不同 field 的 parse 诊断经资格判定与
    /// typed 重驱入口后，最终 prompt 逐条含原三段（逐字）+ 精确 field/形状。
    #[test]
    fn typed_reredrive_prompt_keeps_parse_field_and_example_facts() {
        use super::{TeachingVerdict, build_typed_compile_reredrive_prompt, teaching_verdict};
        use crate::product::work_item_plan_compiler::CompilerDiagnostic;
        fn missing(field: &str) -> CompilerDiagnostic {
            CompilerDiagnostic {
                code: "missing_section".into(),
                line: 5,
                field: field.into(),
                message: "结构化 section 缺少必需字段。".into(),
                repair_example: format!("- {field}: value"),
                kind: None,
                lowering_fault: None,
                ir_fault: None,
            }
        }
        let TeachingVerdict::Eligible { payload } =
            teaching_verdict(&[missing("Goal"), missing("role")], &[])
        else {
            panic!("载荷变化不得改教学资格");
        };
        let prompt = build_typed_compile_reredrive_prompt(&payload, &[], &[]);
        for field in ["Goal", "role"] {
            let block = format!(
                "missing_section:5:结构化 section 缺少必需字段。\n  缺失字段: {field}\n  正确形状: - {field}: value"
            );
            assert!(prompt.contains(&block), "{field} 逐条可区分: {prompt}");
        }
    }

    /// A1：资格判定的 diagnostics→parse_reasons 投影必须逐字携带 field/repair_example。
    #[test]
    fn teaching_verdict_projects_parse_field_and_example_facts() {
        use super::{TeachingVerdict, teaching_verdict};
        use crate::product::work_item_plan_compiler::CompilerDiagnostic;
        fn missing(field: &str) -> CompilerDiagnostic {
            CompilerDiagnostic {
                code: "missing_section".into(),
                line: 5,
                field: field.into(),
                message: "结构化 section 缺少必需字段。".into(),
                repair_example: format!("- {field}: value"),
                kind: None,
                lowering_fault: None,
                ir_fault: None,
            }
        }
        let TeachingVerdict::Eligible { payload } =
            teaching_verdict(&[missing("Goal"), missing("role")], &[])
        else {
            panic!("载荷变化不得改教学资格");
        };
        for field in ["Goal", "role"] {
            let block = format!(
                "missing_section:5:结构化 section 缺少必需字段。\n  缺失字段: {field}\n  正确形状: - {field}: value"
            );
            assert!(payload.parse_reasons.contains(&block), "{field} 投影逐条可区分");
        }
    }
}
