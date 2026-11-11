//! SC compile 失败重驱的资格判定、诊断格式化与预检辅助面，自 single_candidate.rs 拆出以守 1200 行文件守护（先例：single_candidate_baseline_tree.rs / single_candidate_delivery.rs / single_candidate_structure_reredrive.rs）。
use super::*;

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
pub(super) const TEACHABLE_PARSE_FAILURE_CODES: [&str; 4] = [
    "missing_section",
    "invalid_ears",
    "invalid_work_item_id",
    "unknown_structured_key",
];

/// 该诊断码是否属于可教学重驱的 parse 语法类。
pub(super) fn is_teachable_parse_failure(code: &str) -> bool {
    TEACHABLE_PARSE_FAILURE_CODES.contains(&code)
}

// SG6a.2（REQ-SG-05 SC 初轮行×F1.2 typed 口径）：SC 初轮教学重驱资格
// 只认共享 parse 语法码白名单（TEACHABLE_PARSE_FAILURE_CODES）；原
// `is_lowering_absence_message`（message 含「缺少/缺失」措辞即授重驱
// 资格）属字符串资格旁路，已删除——lowering 类（缺席与值域统一）零
// 教学重驱，确定性终态失败由 typed 诊断回显承载。

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

/// 3.6 弱模型基线加固：IR 预校验——在持久化前跑与
/// `complete_single_candidate_work_item_plan_author` 内部同源的
/// `validate_plan_candidate_ir`（上下文组装同源：plan 的 source spec ids 与
/// repository profile），使 IR 校验失败（unknown_requirement_ref /
/// acceptance_criterion_without_reviewer_check 等）可在 handler 分类并享有
/// 恰一次教学重驱。仅当预校验确定性失败时返回 Some(reasons)；上下文装载
/// 失败返回 None（不阻断流程），由 complete_... 权威路径以既有终态形态兜底。
pub(super) fn prevalidate_plan_candidate_ir(
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
    // REQ-WSC-10（写范围×被授权仓归属）：enrolled 装载成员 checkout 前缀表（失败转预检诊断；单仓保持 None 零回归）。
    let eb = enrollment_binding.as_ref();
    let mc_prefixes = match member_checkout_prefixes::for_validation(lifecycle, &plan, eb) {
        Ok(prefixes) => prefixes,
        Err(e) => return Some(vec![format!("member checkout prefixes: {e}")]),
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
pub(super) fn build_work_item_plan_ir_reredrive_prompt(
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
