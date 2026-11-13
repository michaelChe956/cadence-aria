//! P09：SC 失败证据 blocking reasons 三通道纯拼装（compile / IR / structure）。
//!
//! 本模块只做**事实投影**：不解析 message/code 文本、不重建 typed payload、
//! 不改写来源集合、不授予任何资格。教学资格只由
//! `single_candidate_compile_preflight::PreflightVerdict::to_teaching_verdict`
//! 判定——本模块产出 facts，资格由调用侧既有判据决定。
use super::single_candidate_compile_preflight::{PreflightVerdict, format_compile_failure_reasons};
use crate::product::planning_failure::{PlanningFailure, PlanningFailureSet};
use crate::product::work_item_plan_compiler::{
    CompilerDiagnostic, IrFault, IrFaultKind, IrFaultLocator,
};

/// compile 失败面：逐条 `code:line:message`（复用既有单源形态；顺序即
/// diagnostics 顺序，不去重、不排序）后**全量**追加修复指令行——不设呈现
/// 上限、不折叠计数，第 N 条指令逐字在场（呈现截断仍由
/// `compile_failure_surface` 单独负责，本通道不截断）。
pub(super) fn compile_blocking_reasons(
    diagnostics: &[CompilerDiagnostic],
    instructions: &[String],
) -> Vec<String> {
    let mut reasons = format_compile_failure_reasons(diagnostics);
    reasons.extend(instructions.iter().cloned());
    reasons
}

/// IR 预检失败面：operational 在前、呈现原文在后（与
/// `PreflightVerdict::presentation_reasons` 同序）。两文本面皆空且 typed
/// 事实非空时，才回落到 typed 事实的稳定渲染——**有事实 ≠ 有资格**：
/// 不可定位的 typed 事实仍由 `to_teaching_verdict` 判 `ir_fault_not_locatable`。
pub(super) fn ir_blocking_reasons(preflight: &PreflightVerdict) -> Vec<String> {
    let mut reasons = preflight.operational_failures.clone();
    reasons.extend(preflight.failure_reasons.iter().cloned());
    if reasons.is_empty() && !preflight.ir_faults.is_empty() {
        reasons = preflight
            .ir_faults
            .iter()
            .map(render_ir_fault_fact)
            .collect();
    }
    reasons
}

/// 结构失败面：按 producer 顺序逐条渲染（精确重复条目重复输出，不去重、
/// 不排序），只投影 typed 事实，集合本体零改写。
pub(super) fn structure_blocking_reasons(set: &PlanningFailureSet) -> Vec<String> {
    set.failures.iter().map(render_structure_failure).collect()
}

/// typed IR 事实的单行渲染：保留 work item 身份、定位身份、出错引用、
/// 合法集合（原序）、期望形态；行号**仅在 `source_line` 在场时**写出，
/// 缺行不伪造 0/占位。渲染确定：同一事实两次渲染逐字相等。
fn render_ir_fault_fact(fault: &IrFault) -> String {
    let locator = match &fault.locator {
        IrFaultLocator::Task { task_id } => format!("task {task_id}"),
        IrFaultLocator::Handoff => "handoff".to_string(),
        IrFaultLocator::Criterion { criterion_id } => format!("criterion {criterion_id}"),
    };
    let mut fact = format!(
        "ir_fault {} work_item={} locator={} offending_ref={} legal_refs=[{}]",
        ir_fault_kind_code(&fault.kind),
        fault.work_item_id,
        locator,
        fault.offending_ref,
        fault.legal_refs.join("、"),
    );
    if let Some(line) = fault.source_line {
        fact.push_str(&format!(" source_line={line}"));
    }
    fact.push_str(&format!(" expected_shape={}", fault.expected_shape));
    fact
}

/// 四类 IR 故障的稳定码名（与 producer 写入的 diagnostic code 同名；枚举
/// 变体 → 码名的显式映射，不从 message 反解析）。既有同名单源在
/// `workspace_engine::prompts::sc_compile_teaching`（模块私有），P09 三文件
/// 闭合内不得改它，故此处显式映射同名字符串。
fn ir_fault_kind_code(kind: &IrFaultKind) -> &'static str {
    match kind {
        IrFaultKind::UnknownDoneWhenRef => "unknown_done_when_ref",
        IrFaultKind::UnknownReviewerCheckRef => "unknown_reviewer_check_ref",
        IrFaultKind::AcceptanceCriterionWithoutReviewerCheck => {
            "acceptance_criterion_without_reviewer_check"
        }
        IrFaultKind::UnknownRequirementRef => "unknown_requirement_ref",
    }
}

/// 结构失败的单行渲染：稳定码 + origin/stage + 诊断引用 + 受影响 ID
/// （producer 原序，重复保留）+ details 每对 key/value（BTreeMap 键序）。
/// 不解析 `Operational { stable_code, cause }` 的 cause 文本、不从 prose 造
/// id、不重封集合——渲染是来源集合的纯函数。
fn render_structure_failure(failure: &PlanningFailure) -> String {
    let mut line = format!(
        "planning_failure {} origin={:?} stage={:?} diagnostic_ref={}",
        failure.kind.stable_code(),
        failure.origin,
        failure.stage,
        failure.diagnostic_ref,
    );
    if !failure.evidence.affected_ids.is_empty() {
        line.push_str(&format!(
            " affected_ids=[{}]",
            failure.evidence.affected_ids.join("、")
        ));
    }
    if !failure.evidence.details.is_empty() {
        let details = failure
            .evidence
            .details
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(", ");
        line.push_str(&format!(" details=[{details}]"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::super::single_candidate_compile_preflight::{PreflightVerdict, TeachingVerdict};
    use super::{compile_blocking_reasons, ir_blocking_reasons, structure_blocking_reasons};
    use crate::product::planning_failure::{
        PlanningCycleWitnessHop, PlanningEdgeSource, PlanningFailure, PlanningFailureEvidence,
        PlanningFailureKind, PlanningFailureOrigin, PlanningFailureSet, PlanningStage,
    };
    use crate::product::work_item_plan_compiler::{
        CompilerDiagnostic, IrFault, IrFaultKind, IrFaultLocator,
    };

    fn diag(code: &str, line: usize, message: &str) -> CompilerDiagnostic {
        CompilerDiagnostic {
            code: code.to_string(),
            line,
            field: format!("{code}.field"),
            message: message.to_string(),
            repair_example: format!("{code}=example"),
            kind: None,
            lowering_fault: None,
            ir_fault: None,
        }
    }

    fn ir_fault(work_item_id: &str, source_line: Option<usize>) -> IrFault {
        IrFault {
            kind: IrFaultKind::UnknownRequirementRef,
            work_item_id: work_item_id.to_string(),
            locator: IrFaultLocator::Task {
                task_id: format!("{work_item_id}-task"),
            },
            offending_ref: format!("REQ-UNKNOWN-{work_item_id}"),
            legal_refs: vec![format!("{work_item_id}-AC-002")],
            source_line,
            expected_shape: IrFaultKind::UnknownRequirementRef
                .expected_shape()
                .to_string(),
        }
    }

    fn failure(tag: &str, kind: PlanningFailureKind) -> PlanningFailure {
        PlanningFailure {
            kind,
            origin: PlanningFailureOrigin::EffectiveGraph,
            stage: PlanningStage::Plan,
            diagnostic_ref: format!("diag/{tag}"),
            source_ref: "source/rev-1".to_string(),
            source_hash: "sha256:fixture".to_string(),
            evidence: PlanningFailureEvidence::default(),
        }
    }

    fn hop(from: &str, to: &str, sources: &[PlanningEdgeSource]) -> PlanningCycleWitnessHop {
        PlanningCycleWitnessHop {
            from: from.to_string(),
            to: to.to_string(),
            sources: sources.to_vec(),
        }
    }

    /// 案例 1：逐条完整 code:line:message 且顺序不动；指令行全量追加、不受呈现上限截断（第 5、6 条保留）。
    #[test]
    fn compile_blocking_reasons_keep_ordered_reasons_and_all_six_instruction_lines() {
        let diagnostics = vec![
            diag("missing_section", 0, "first reason"),
            diag("unknown_structured_key", 12, "second reason"),
            diag("invalid_ears", 31, "third reason"),
        ];
        let instructions: Vec<String> = (1..=6)
            .map(|n| format!("instruction-{n}: keep verbatim"))
            .collect();
        let blocking = compile_blocking_reasons(&diagnostics, &instructions);
        assert_eq!(blocking.len(), diagnostics.len() + instructions.len());
        for (index, diagnostic) in diagnostics.iter().enumerate() {
            let expected = format!(
                "{}:{}:{}",
                diagnostic.code, diagnostic.line, diagnostic.message
            );
            assert_eq!(blocking[index], expected);
        }
        assert_eq!(&blocking[diagnostics.len()..], instructions.as_slice());
        assert_eq!(
            blocking[blocking.len() - 1],
            "instruction-6: keep verbatim",
            "第 6 条指令不得被截断或折叠成计数摘要"
        );
        assert_eq!(
            instructions.len(),
            6,
            "指令全集仍原样保留给 repair_instructions"
        );
    }

    /// 案例 2：完全相同的事实重复保留；不排序、不去重。
    #[test]
    fn compile_blocking_reasons_do_not_dedup_identical_entries() {
        let duplicated = diag("unknown_structured_key", 7, "same reason");
        let diagnostics = vec![duplicated.clone(), duplicated.clone()];
        let instructions = vec![
            "same instruction".to_string(),
            "same instruction".to_string(),
        ];
        let blocking = compile_blocking_reasons(&diagnostics, &instructions);
        assert_eq!(blocking.len(), 4);
        assert_eq!(blocking[0], blocking[1]);
        assert_eq!(blocking[2], "same instruction");
        assert_eq!(blocking[3], "same instruction");
    }

    /// 案例 3：operational 在前、failure_reasons 在后；文本数组非空时 typed 事实不得再被追加。
    #[test]
    fn ir_blocking_reasons_emit_operational_then_raw_reasons_without_typed_echo() {
        let verdict = PreflightVerdict {
            ir_faults: vec![ir_fault("WI-typed", Some(4))],
            operational_failures: vec!["plan source unreadable".to_string()],
            failure_reasons: vec![
                "unknown_requirement_ref:4:first".to_string(),
                "unknown_done_when_ref:9:second".to_string(),
            ],
        };
        assert_eq!(
            ir_blocking_reasons(&verdict),
            vec![
                "plan source unreadable",
                "unknown_requirement_ref:4:first",
                "unknown_done_when_ref:9:second"
            ]
        );
    }

    /// 案例 4：两文本数组皆空但 typed 事实非空 ⇒ 稳定渲染非空 facts（保留 work_item_id/
    /// locator/legal_refs/期望形态）；且不授予教学资格。
    #[test]
    fn ir_typed_only_facts_render_stably_without_granting_eligibility() {
        let unlocatable = ir_fault("WI-typed", None);
        let locatable = ir_fault("WI-typed", Some(4237));
        let verdict_for = |fault: IrFault| PreflightVerdict {
            ir_faults: vec![fault],
            operational_failures: Vec::new(),
            failure_reasons: Vec::new(),
        };
        let first = ir_blocking_reasons(&verdict_for(unlocatable.clone()));
        let second = ir_blocking_reasons(&verdict_for(unlocatable.clone()));
        assert!(!first.is_empty(), "typed-only 失败面不得是空 facts");
        assert_eq!(first, second, "同一 typed 集合必须稳定渲染");
        let rendered = first.join("\n");
        assert!(rendered.contains(&unlocatable.work_item_id));
        assert!(rendered.contains(&unlocatable.expected_shape));
        assert!(rendered.contains("WI-typed-task"), "locator 身份必须保留");
        for legal_ref in &unlocatable.legal_refs {
            assert!(rendered.contains(legal_ref));
        }
        let with_line = ir_blocking_reasons(&verdict_for(locatable)).join("\n");
        assert_ne!(rendered, with_line);
        assert!(with_line.contains("4237"));
        assert!(!rendered.contains("4237"), "source_line=None 不得伪造行号");
        let verdict = verdict_for(unlocatable);
        assert!(verdict.has_failures());
        assert_eq!(verdict.presentation_reasons(), Vec::<String>::new());
        assert_eq!(
            verdict.to_teaching_verdict(),
            TeachingVerdict::NotEligible {
                reason: "ir_fault_not_locatable"
            },
            "有 typed 事实 ≠ 有教学资格"
        );
    }

    /// 案例 5：多失败按 producer 顺序全量保留，精确重复条目不去重；投影不改写集合。
    #[test]
    fn structure_blocking_reasons_keep_every_item_in_order_including_duplicates() {
        let duplicated = failure(
            "dup",
            PlanningFailureKind::TargetMismatch {
                logical_work_item_id: "WI-1".to_string(),
                declared_target: Some("repo-a".to_string()),
                resolved_target: "repo-b".to_string(),
            },
        );
        let cycle = failure(
            "cycle",
            PlanningFailureKind::DependencyCycle {
                witness: vec![
                    hop(
                        "WI-1",
                        "WI-2",
                        &[
                            PlanningEdgeSource::CanonicalInput,
                            PlanningEdgeSource::ChangeOrder,
                        ],
                    ),
                    hop("WI-2", "WI-1", &[PlanningEdgeSource::CanonicalInput]),
                ],
            },
        );
        let set = PlanningFailureSet {
            failures: vec![
                duplicated.clone(),
                cycle,
                duplicated.clone(),
                failure(
                    "missing",
                    PlanningFailureKind::RequiredContractMissing {
                        provider: "WI-1".to_string(),
                        consumer: "WI-2".to_string(),
                        contract_id: "CT-1".to_string(),
                    },
                ),
            ],
        };
        let before = set.clone();
        let blocking = structure_blocking_reasons(&set);
        assert_eq!(blocking.len(), 4);
        assert_eq!(blocking[0], blocking[2], "重复的 typed 条目必须保留");
        assert_ne!(blocking[0], blocking[1]);
        assert_ne!(blocking[1], blocking[3]);
        for (line, original) in blocking.iter().zip(set.failures.iter()) {
            assert!(line.contains(original.kind.stable_code()));
            assert!(line.contains(&original.diagnostic_ref));
        }
        assert_eq!(set, before, "投影是纯函数：集合本体零改写");
    }

    /// 案例 6：affected_ids 保持 producer 顺序且重复 ID 保留；details 按 BTreeMap 稳定键序展开。
    #[test]
    fn structure_blocking_reasons_keep_affected_id_order_and_details_key_order() {
        let mut ordered = failure(
            "ordered",
            PlanningFailureKind::Unknown {
                code: "unknown_fixture".to_string(),
            },
        );
        ordered.evidence = PlanningFailureEvidence {
            affected_ids: vec!["WI-9".to_string(), "WI-2".to_string(), "WI-9".to_string()],
            details: Default::default(),
        };
        ordered
            .evidence
            .details
            .insert("zeta-key".into(), "detail-zeta".into());
        ordered
            .evidence
            .details
            .insert("alpha-key".into(), "detail-alpha".into());
        ordered
            .evidence
            .details
            .insert("mid-key".into(), "detail-mid".into());
        let set = PlanningFailureSet {
            failures: vec![ordered],
        };
        let line = structure_blocking_reasons(&set).remove(0);
        let at = |needle: &str| {
            line.find(needle)
                .unwrap_or_else(|| panic!("rendered fact missing {needle}"))
        };
        assert!(at("WI-9") < at("WI-2"), "affected_ids 不得排序");
        assert!(
            at("WI-2") < line.rfind("WI-9").expect("second WI-9 missing"),
            "重复 affected_id 不得去重"
        );
        assert_eq!(line.matches("WI-9").count(), 2);
        assert!(at("alpha-key") < at("mid-key"), "details 按键序展开");
        assert!(at("mid-key") < at("zeta-key"));
        for value in ["detail-alpha", "detail-mid", "detail-zeta"] {
            assert!(line.contains(value));
        }
    }
}

/// P09 runtime consumer RED（真实 store 边界）：此刻生产 helper
/// （`compile_failure_facts` / `observe_sc_failure` / `failure_observed_key`）
/// 尚不存在 ⇒ 本模块编译失败即 RED。端到端 gate 顺序（FailureObserved 先于
/// claim、真实判定先于 provider start）由 P27 真实 adapter 验收，不在此用
/// 恒定零计数冒充（无 gate 调用时 claim/start 恒为 0，是 tautology）。
#[cfg(test)]
mod consumer_tests {
    use super::super::sc_failure_transport::{ScBoundProviderRound, begin_sc_failure_round};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::lifecycle_store::inputs::{
        CreateWorkspaceSessionInput, WorkItemPlanSessionOptions,
    };
    use crate::product::lifecycle_store::sc_failure_evidence::ScFailureEvidenceEvent;
    use crate::product::models::{ProviderName, WorkspaceType};
    use crate::product::work_item_plan_compiler::CompilerDiagnostic;
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
    use crate::web::workspace_ws_types::{ScEvidenceRef, ScFailureClass, ScFailureScope};

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const PLAN_ID: &str = "plan_0001";
    const NODE_ID: &str = "node_sc_0001";
    const BUSINESS_KEY: &str = "workspace_author:session:0";

    /// SC single-candidate WorkItemPlan session（store 准入要求
    /// flow_kind == SingleCandidate 且 scope.plan_id == session.entity_id）。
    fn fixture() -> (tempfile::TempDir, LifecycleStore, ScFailureScope) {
        let tmp = tempfile::tempdir().expect("temp root");
        let lifecycle = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.into(),
                issue_id: ISSUE_ID.into(),
                entity_id: PLAN_ID.into(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    ..Default::default()
                }),
            })
            .expect("create SC session");
        let scope = ScFailureScope {
            session_id: record.id,
            project_id: PROJECT_ID.into(),
            issue_id: ISSUE_ID.into(),
            plan_id: PLAN_ID.into(),
        };
        (tmp, lifecycle, scope)
    }

    fn bind_round(lifecycle: &LifecycleStore, scope: &ScFailureScope) -> ScBoundProviderRound {
        begin_sc_failure_round(
            lifecycle,
            scope,
            BUSINESS_KEY,
            None,
            NODE_ID,
            &ProviderName::Fake,
            0,
        )
        .expect("round binds a real handle")
    }

    /// 独立摘要（绝不复用生产 `sc_sha256` 作期望值，避免同错自证）。
    fn independent_sha256(text: &str) -> String {
        use sha2::Digest as _;
        hex::encode(sha2::Sha256::digest(text.as_bytes()))
    }

    fn diagnostic(code: &str, line: usize, message: &str) -> CompilerDiagnostic {
        CompilerDiagnostic {
            code: code.to_string(),
            line,
            field: format!("{code}.field"),
            message: message.to_string(),
            repair_example: format!("{code}=example"),
            kind: None,
            lowering_fault: None,
            ir_fault: None,
        }
    }

    /// ① 真实 store 完整读回：FailureObserved 一轮写读即返回**未截断**事实
    /// （诊断原文顺序、6 条指令逐字、source/raw 各自 ref 且同 locator），
    /// 且 RoundBound 与本轮失败事实同链保留、读回行逐字等于返回行。
    #[test]
    fn observe_sc_failure_persists_full_untruncated_facts_in_real_store() {
        let (_tmp, lifecycle, scope) = fixture();
        let session_id = scope.session_id.clone();
        let bound = bind_round(&lifecycle, &scope);

        let raw_body = "raw provider output (verbatim)";
        let raw = ScEvidenceRef::Available {
            locator: format!("sc-split-output/v1/{session_id}/0"),
            sha256: independent_sha256(raw_body),
            bytes: raw_body.len() as u64,
        };
        let compile_source = "# plan\nnormalized compile source (verbatim)\n";
        let diagnostics = vec![
            diagnostic("missing_section", 0, "first reason"),
            diagnostic("unknown_structured_key", 12, "second reason"),
        ];
        let instructions: Vec<String> = (1..=6)
            .map(|n| format!("instruction-{n}: keep verbatim"))
            .collect();

        let facts = super::compile_failure_facts(
            &diagnostics,
            instructions.clone(),
            &raw,
            compile_source,
            &bound.round,
        );
        let observed = super::observe_sc_failure(&lifecycle, &bound.chain, &bound.round, facts)
            .expect("FailureObserved appends and strictly reads back");

        let ScFailureEvidenceEvent::FailureObserved {
            round_id,
            failure_class,
            blocking_reasons,
            repair_instructions,
            raw: read_raw,
            source: read_source,
            existing_failure_id,
            attempt_key,
        } = &observed.entry.event
        else {
            unreachable!("observe_sc_failure only appends FailureObserved rows");
        };
        assert_eq!(round_id, &bound.round.round_id);
        assert_eq!(failure_class, &Some(ScFailureClass::Compile));
        assert_eq!(blocking_reasons.len(), diagnostics.len() + instructions.len());
        assert_eq!(blocking_reasons[0], "missing_section:0:first reason");
        assert_eq!(blocking_reasons[1], "unknown_structured_key:12:second reason");
        assert_eq!(
            &blocking_reasons[diagnostics.len()..],
            instructions.as_slice(),
            "指令全集必须逐字、按序、不截断"
        );
        assert_eq!(repair_instructions, &instructions);
        assert_eq!(read_raw, &raw, "raw 必须是本轮真实引用");
        assert_eq!(
            read_source,
            &ScEvidenceRef::Available {
                locator: format!("sc-split-output/v1/{session_id}/0"),
                sha256: independent_sha256(compile_source),
                bytes: compile_source.len() as u64,
            },
            "source 与 raw 同 locator、摘要/字节数取归一化正文真值"
        );
        assert!(existing_failure_id.is_none());
        assert!(attempt_key.is_none());

        let read = lifecycle
            .read_sc_failure_evidence(&bound.chain)
            .expect("strict evidence log readable");
        assert!(
            read.entries.iter().any(|entry| {
                matches!(&entry.event, ScFailureEvidenceEvent::RoundBound { round } if round == &bound.round)
            }),
            "本轮绑定事实必须与失败事实同链保留"
        );
        let stored = read
            .entries
            .iter()
            .find(|entry| entry.entry_key == super::failure_observed_key(&bound.round.round_id))
            .expect("FailureObserved row is durable");
        assert_eq!(stored.event, observed.entry.event);
    }

    /// ② 真实写准入失败（chain 身份与四元组不一致）：稳定码/step 上浮，
    /// 且真实日志零 FailureObserved（不留半截失败事实）。
    #[test]
    fn evidence_write_failure_leaves_no_failure_observed_rows() {
        let (_tmp, lifecycle, scope) = fixture();
        let session_id = scope.session_id.clone();
        let bound = bind_round(&lifecycle, &scope);

        // 合法 64 位小写 hex，但与本轮身份四元组不一致 ⇒ 真实 store 写准入失败。
        let mut tampered = bound.chain.clone();
        tampered.chain_id = format!("chain-v1-{}", "0".repeat(64));

        let raw = ScEvidenceRef::Available {
            locator: format!("sc-split-output/v1/{session_id}/0"),
            sha256: independent_sha256("raw body"),
            bytes: "raw body".len() as u64,
        };
        let facts = super::compile_failure_facts(
            &[diagnostic("missing_section", 0, "first reason")],
            vec!["instruction-1: keep verbatim".to_string()],
            &raw,
            "compile source",
            &bound.round,
        );
        let error = super::observe_sc_failure(&lifecycle, &tampered, &bound.round, facts)
            .expect_err("chain 身份不一致时证据写准入必须失败");
        assert_eq!(error.code, "sc_failure_chain_invalid");
        assert_eq!(error.step, "append_sc_failure_evidence");

        let read = lifecycle
            .read_sc_failure_evidence(&bound.chain)
            .expect("strict evidence log readable");
        assert!(
            !read.entries.iter().any(|entry| matches!(
                &entry.event,
                ScFailureEvidenceEvent::FailureObserved { .. }
            )),
            "写失败不得留下半截失败事实"
        );
    }
}
