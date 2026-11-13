//! P09 前置 RED（纯行为面）：SC 失败证据三通道 blocking reasons 拼装契约。
//! 落点：`run/sc_failure_evidence.rs` 末尾 `#[cfg(test)] mod tests`，原样粘贴；
//! helpers 在 `super::`（文件尚不存在）⇒ 当前整体不可编译 = RED。只钉数据事实
//! （顺序/存在/重复/未截断），不钉实现措辞；仅 ticket 明文的 `code:line:message` 比对整串。
//!
//! v2 校正（2026-11-13，FullFailureFacts）：`IrFault` 实测 7 字段（`ir_fault.rs:21-40`
//! 含 `offending_ref: String`），v1 fixture 缺该字段会额外产生 E0063，使 RED 失真；
//! 此处补齐真实字段，其余断言逐字未改。
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
