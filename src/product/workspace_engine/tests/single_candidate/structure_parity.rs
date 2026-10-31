// A1（REQ-WG-01 / REQ-WG-02）：候选结构门与 Final Compile 的依赖图奇偶性。
//
// 同一个 fixture 必须**真的跑两处**：候选门 `assess_single_candidate_structure`
// （SG1 有效图）与终编译 `prepare_initial_plan_compile`（内部走
// `apply_change_order_cross_repo_depends_on`）。只测抽出来的反查函数抓不到
// 「两套图不一致」——本模块因此钉住三件事：
//   ① 有效边集合逐条一致（含 change_order 自动边的两端与来源）；
//   ② 成环结论一致；
//   ③ 环 witness 首跳（含来源）与链的延续逐字一致。
// 反例（本文件存在即证明）：SingleRepository 归属落 (None, None) 时，候选门
// 一条仓序边都不加 → 判无环，终编译反查到成员 → 判成环。
mod structure_parity {
    use super::*;

    use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
    use crate::product::models::{
        IssueWorkItemPlan, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
        WorkItemDraftVerificationPlan,
    };
    use crate::product::planning_failure::{PlanningEdgeSource, PlanningFailureKind};
    use crate::product::work_item_contract::{
        CanonicalWorkItemContract, ContractCompatibilityPolicy, RequiredInputContract,
        WorkItemIntent, WorkItemIntentContract, canonical_contract_fixture,
    };
    use crate::product::work_item_plan_compiler::{
        PlanCandidateIr, PlanCandidateItemIr, PlanCandidateMechanicalReport,
        WORK_ITEM_PLAN_COMPILER_VERSION,
    };
    use crate::product::workspace_engine::compile::ir_adapter::{
        IrCompileAdapterContext, initial_plan_compile_input_from_ir,
    };
    use crate::product::workspace_engine::draft_batch::compile_support::{
        PlanningStructureContext, PlanningStructureReport,
    };
    use crate::product::workspace_engine::plan_projection::{
        PLANNING_STRUCTURE_RULE_VERSION, assess_single_candidate_structure,
    };
    use crate::product::workspace_engine::types::StructuralAssessment;
    use crate::product::workspace_engine::{
        InitialPlanCompileDurableContext, prepare_initial_plan_compile,
    };

    const SOURCE_HASH: &str = "structure-parity-source-hash";

    /// 稳定 UUID：禁止运行时随机，保证两处图的节点序可复现。
    fn repo(seed: u8) -> LogicalRepositoryId {
        let mut bytes = [0u8; 16];
        bytes[0] = seed;
        LogicalRepositoryId(uuid::Uuid::from_bytes(bytes))
    }

    struct Repos {
        a: LogicalRepositoryId,
        b: LogicalRepositoryId,
        c: LogicalRepositoryId,
    }

    impl Repos {
        fn new() -> Self {
            Self {
                a: repo(0x0A),
                b: repo(0x0B),
                c: repo(0x0C),
            }
        }

        /// 授权成员表（逻辑成员 → 物理仓），与 `physical_by_logical` 同向。
        fn table(&self) -> BTreeMap<LogicalRepositoryId, String> {
            BTreeMap::from([
                (self.a, "repository_a".to_string()),
                (self.b, "repository_b".to_string()),
                (self.c, "repository_c".to_string()),
            ])
        }

        /// confirmed design 的仓级执行顺序：A → B → C。
        fn change_order(&self) -> Vec<LogicalRepositoryId> {
            vec![self.a, self.b, self.c]
        }
    }

    fn contract(
        logical_id: &str,
        provider: Option<&str>,
        target: EnrollmentTarget,
    ) -> CanonicalWorkItemContract {
        let mut contract = canonical_contract_fixture(logical_id);
        // 呈现名与逻辑身份同值：终编译环链按 title 渲染，候选门用逻辑 id，
        // 两者同值才能逐字比对 witness。
        contract.identity.title = logical_id.to_string();
        contract.input_contracts = provider
            .map(|provider| RequiredInputContract {
                contract_id: format!("contract.{logical_id}"),
                provider_logical_work_item_id: provider.to_string(),
                required_capabilities: vec!["capability".to_string()],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            })
            .into_iter()
            .collect();
        contract.intent_contract = Some(WorkItemIntentContract {
            intent: WorkItemIntent::Create,
            provider_work_item_id: provider.unwrap_or_default().to_string(),
            depends_on: Vec::new(),
            exclusive_scopes: Vec::new(),
            forbidden_scopes: Vec::new(),
            target,
        });
        contract
    }

    /// 现场形态（D2）：每个 WI 都用 `single_repository` 声明自己托管的物理仓。
    fn item(logical_id: &str, physical: &str, provider: Option<&str>) -> PlanCandidateItemIr {
        PlanCandidateItemIr {
            target_repository_id: physical.to_string(),
            contract: contract(
                logical_id,
                provider,
                EnrollmentTarget::SingleRepository {
                    repository_id: physical.to_string(),
                },
            ),
            verification_plan: WorkItemDraftVerificationPlan { checks: Vec::new() },
            trusted_commands: Vec::new(),
        }
    }

    fn ir_of(items: Vec<PlanCandidateItemIr>) -> PlanCandidateIr {
        PlanCandidateIr {
            source_revision_hash: SOURCE_HASH.to_string(),
            compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
            items,
        }
    }

    fn structure_context(
        table: BTreeMap<LogicalRepositoryId, String>,
        change_order: Vec<LogicalRepositoryId>,
    ) -> PlanningStructureContext {
        PlanningStructureContext {
            source_revision_ref: "plan-source/structure-parity".to_string(),
            plan_candidate_ir_ref: "plan-ir/structure-parity".to_string(),
            source_hash: SOURCE_HASH.to_string(),
            compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
            rule_version: PLANNING_STRUCTURE_RULE_VERSION.to_string(),
            context_digest: "structure-parity-context-digest".to_string(),
            confirmed_design_id: None,
            confirmed_design_digest: None,
            member_table_digest: None,
            selection_digest: None,
            physical_by_logical: table,
            change_order,
            options_digest: "structure-parity-options-digest".to_string(),
            baseline_tree_digest: None,
            intent_digest: None,
        }
    }

    fn previous_plan() -> IssueWorkItemPlan {
        IssueWorkItemPlan {
            id: "plan_structure_parity".to_string(),
            project_id: "project_structure_parity".to_string(),
            issue_id: "issue_structure_parity".to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
            review_summary: None,
            ever_confirmed: None,
            created_at: "2026-10-31T00:00:00Z".to_string(),
            updated_at: "2026-10-31T00:00:00Z".to_string(),
        }
    }

    /// 终编译侧的**真实**边集合：IR adapter → 纯 prepare（内部把 change_order
    /// 展开成跨仓 depends_on 并做环检测）。成功时把 work item id 反投影回
    /// 逻辑 id，产出与候选门同构的 `(provider, consumer)` 边集合。
    fn final_compile_edges(
        table: &BTreeMap<LogicalRepositoryId, String>,
        change_order: &[LogicalRepositoryId],
        ir: &PlanCandidateIr,
    ) -> Result<Vec<(String, String)>, String> {
        let context = IrCompileAdapterContext {
            project_id: "project_structure_parity".to_string(),
            issue_id: "issue_structure_parity".to_string(),
            plan_id: "plan_structure_parity".to_string(),
            previous_plan: previous_plan(),
            source_revision_id: "source_structure_parity".to_string(),
            source_revision_ref: "plan-source/structure-parity".to_string(),
            plan_candidate_ir_ref: "plan-ir/structure-parity".to_string(),
            mechanical_report_ref: "mechanical-report/structure-parity".to_string(),
            publication_provenance_ref: "publication-provenance/structure-parity".to_string(),
            logical_targets: Some(table.clone()),
            // LC 多仓：legacy 单仓 target 不参与（与生产 SC 链同口径）。
            repository_id: String::new(),
            change_order: change_order.to_vec(),
            compile_id: "compile_structure_parity".to_string(),
            now: "2026-10-31T00:00:00Z".to_string(),
        };
        let report = PlanCandidateMechanicalReport {
            source_revision_hash: SOURCE_HASH.to_string(),
            compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
            findings: Vec::new(),
        };
        let input = initial_plan_compile_input_from_ir(&context, ir, &report)?;
        let prepared =
            prepare_initial_plan_compile(input, InitialPlanCompileDurableContext::legacy())?;
        let wi_to_logical: BTreeMap<&str, &str> = prepared
            .work_items
            .iter()
            .zip(ir.items.iter())
            .map(|(record, item)| {
                (
                    record.id.as_str(),
                    item.contract.identity.logical_work_item_id.as_str(),
                )
            })
            .collect();
        let mut edges = Vec::new();
        for record in &prepared.work_items {
            let consumer = *wi_to_logical
                .get(record.id.as_str())
                .expect("work item record must map back to a fixture item");
            for dependency in &record.depends_on {
                let provider = *wi_to_logical
                    .get(dependency.as_str())
                    .unwrap_or_else(|| panic!("dependency `{dependency}` must be in the batch"));
                edges.push((provider.to_string(), consumer.to_string()));
            }
        }
        edges.sort();
        edges.dedup();
        Ok(edges)
    }

    fn gate_edges(report: &PlanningStructureReport) -> Vec<(String, String)> {
        let mut edges: Vec<(String, String)> = report
            .effective_edges
            .iter()
            .map(|edge| (edge.from.clone(), edge.to.clone()))
            .collect();
        edges.sort();
        edges
    }

    #[test]
    fn candidate_gate_and_final_compile_agree_on_effective_edges() {
        let repos = Repos::new();
        let table = repos.table();
        let change_order = repos.change_order();
        let ir = ir_of(vec![
            item("WI-001", "repository_a", None),
            item("WI-002", "repository_b", Some("WI-001")),
            item("WI-003", "repository_c", Some("WI-002")),
            item("WI-004", "repository_c", Some("WI-003")),
        ]);
        let context = structure_context(table.clone(), change_order.clone());
        let assessment = assess_single_candidate_structure(String::new(), &ir, &context);
        let StructuralAssessment::Accepted(prepared) = assessment else {
            panic!("无环候选必须在候选门通过（同仓消费 + 顺仓序）");
        };
        // 仓序自动边确实在两处都生效：WI-004 托管仓序第 3，与 WI-001 之间
        // 只有 change_order 一条来源。
        let cross_repo = prepared
            .report
            .effective_edges
            .iter()
            .find(|edge| edge.from == "WI-001" && edge.to == "WI-004")
            .expect("候选门必须产生 change_order 自动边 WI-001→WI-004");
        assert_eq!(cross_repo.sources, vec![PlanningEdgeSource::ChangeOrder]);
        assert_eq!(cross_repo.order_indices, vec![0, 2]);

        let final_edges = final_compile_edges(&table, &change_order, &ir)
            .expect("同一 fixture 在终编译必须同样通过");
        assert_eq!(
            gate_edges(&prepared.report),
            final_edges,
            "候选门与终编译的有效边集合必须逐条一致",
        );
        // 3 条 canonical + 3 条仓序自动边（A→B、A→C、B→C）。
        assert_eq!(final_edges.len(), 6);
    }

    #[test]
    fn candidate_gate_and_final_compile_agree_on_dependency_cycle_witness() {
        let repos = Repos::new();
        let table = repos.table();
        let change_order = repos.change_order();
        let ir = ir_of(vec![
            item("WI-001", "repository_a", None),
            item("WI-002", "repository_b", Some("WI-001")),
            item("WI-003", "repository_c", Some("WI-002")),
            // 现场形态（D2）：托管仓序第 2，却消费仓序第 3 的产出。
            item("WI-004", "repository_b", Some("WI-003")),
        ]);
        let context = structure_context(table.clone(), change_order.clone());
        let assessment = assess_single_candidate_structure(String::new(), &ir, &context);
        let StructuralAssessment::Rejected(failures, _) = assessment else {
            panic!("逆仓序的跨仓消费必须在候选门被拦下（review 前）");
        };
        let witness = failures
            .failures
            .iter()
            .find_map(|failure| match &failure.kind {
                PlanningFailureKind::DependencyCycle { witness } => Some(witness.clone()),
                _ => None,
            })
            .expect("候选门必须给出 typed 依赖环");
        assert_eq!(witness.len(), 2, "witness 必须是真实二跳环：{witness:?}");
        assert_eq!(
            (witness[0].from.as_str(), witness[0].to.as_str()),
            ("WI-003", "WI-004"),
            "首跳必须是 WI-003→WI-004（provider→consumer）",
        );
        assert_eq!(
            witness[0].sources,
            vec![PlanningEdgeSource::CanonicalInput],
            "首跳来源必须是 canonical Inputs",
        );
        assert_eq!(
            (witness[1].from.as_str(), witness[1].to.as_str()),
            ("WI-004", "WI-003"),
            "回边必须是 WI-004→WI-003",
        );
        assert_eq!(
            witness[1].sources,
            vec![PlanningEdgeSource::ChangeOrder],
            "回边来源必须是 change_order 自动边",
        );

        let error = final_compile_edges(&table, &change_order, &ir)
            .expect_err("终编译必须同样判依赖环");
        assert!(error.contains("dependency_cycle"), "{error}");
        // 终编译环链由同一 witness 提取器渲染（固定 ID/邻接序）——首跳与
        // 环的延续必须与候选门逐字一致。
        let rendered = format!(
            "「{}」 → 「{}」 → 「{}」",
            witness[0].from, witness[0].to, witness[1].to,
        );
        assert!(
            error.contains(&rendered),
            "终编译环链必须与候选门 witness 逐字一致（期望含 {rendered}）：{error}",
        );
    }

    #[test]
    fn ambiguous_physical_member_is_never_guessed_in_either_graph() {
        let repos = Repos::new();
        let mut table = repos.table();
        // 两个逻辑成员共用同一物理仓：配置矛盾 → 多命中。
        table.insert(repo(0xEE), "repository_b".to_string());
        let change_order = repos.change_order();
        let ir = ir_of(vec![
            item("WI-001", "repository_a", None),
            item("WI-002", "repository_b", Some("WI-001")),
        ]);
        let context = structure_context(table.clone(), change_order.clone());
        let assessment = assess_single_candidate_structure(String::new(), &ir, &context);
        let StructuralAssessment::Accepted(prepared) = assessment else {
            panic!("多命中不得在候选门编造结构失败（不猜、也不造假证据）");
        };
        assert!(
            !prepared.report.physical_targets.contains_key("WI-002"),
            "多命中不得任选一个逻辑成员并登记物理 target",
        );
        assert!(
            prepared.report.effective_edges.iter().all(|edge| {
                !(edge.to == "WI-002" && edge.sources.contains(&PlanningEdgeSource::ChangeOrder))
            }),
            "多命中不得产生该 WI 的仓序自动边",
        );

        let error = final_compile_edges(&table, &change_order, &ir)
            .expect_err("终编译必须对多命中 fail-closed");
        assert!(
            error.contains("maps to multiple logical repositories"),
            "多命中的既有失败语义必须保留：{error}",
        );
    }
}
