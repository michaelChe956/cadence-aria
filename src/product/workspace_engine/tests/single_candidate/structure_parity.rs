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

    /// SG3a.1（作者阶段 typed 失败，REQ-WSC-18/21/25）：同一 LC 多仓环在作者
    /// 阶段产出的 durable 事实必须能被 owner 面解析出**逆仓序那一跳**的两位次，
    /// 并给出恰好两个带后果的修法——否则 owner 还是只能看到「生成失败」。
    #[test]
    fn author_stage_cycle_failure_carries_witness_facts_and_two_owner_options() {
        use crate::product::workspace_engine::compile_failure_surface::{
            cycle_witness_facts, owner_hint, owner_message, rewrite_guidance,
            cycle_owner_options,
        };
        use crate::product::workspace_engine::single_candidate_structure_failure::{
            STRUCTURE_STABLE_CODE, author_gate_summary, author_structure_gate_failure,
        };

        let repos = Repos::new();
        let table = repos.table();
        let change_order = repos.change_order();
        let ir = ir_of(vec![
            item("WI-001", "repository_a", None),
            item("WI-002", "repository_b", Some("WI-001")),
            item("WI-003", "repository_c", Some("WI-002")),
            // 现场形态：托管仓序第 2（repository_b），却消费仓序第 3 的产出。
            item("WI-004", "repository_b", Some("WI-003")),
        ]);
        let context = structure_context(table.clone(), change_order.clone());
        let assessment = assess_single_candidate_structure(String::new(), &ir, &context);
        let StructuralAssessment::Rejected(failures, _) = assessment else {
            panic!("逆仓序的跨仓消费必须在候选门被拦下");
        };

        let typed =
            author_structure_gate_failure(&ir, &context, &failures);
        assert_eq!(
            typed.stable_code(),
            STRUCTURE_STABLE_CODE,
            "作者阶段拒收一律用结构拒收码（owner 文案不得指向没跑过的终编译；\
             失败码统计要能区分阶段；不虚记自动重写预算）"
        );

        // durable 投影（只投影、不落盘）：message 必须能被既有解析面还原事实。
        let failure = crate::product::workspace_engine::compile_failure_surface::
            candidate_compile_status(None, &typed, "2026-10-31T00:00:00Z".to_string())
            .0
            .failure
            .expect("durable failure projection");
        let facts = cycle_witness_facts(&failure.message)
            .unwrap_or_else(|| panic!("环事实必须可解析：{}", failure.message));
        assert_eq!(facts.provider, "WI-003", "provider 是仓序在后的那个 WI");
        assert_eq!(facts.consumer, "WI-004", "consumer 是仓序在前的那个 WI");
        assert_eq!(facts.provider_order, 3, "provider 所在仓是仓序第 3 个");
        assert_eq!(facts.consumer_order, 2, "consumer 所在仓是仓序第 2 个");

        // owner 面：一句人话问题 + 恰好两个带后果的修法（不是「生成失败」）。
        let options = cycle_owner_options(&facts);
        assert_eq!(options.len(), 2, "必须给出恰好两个合法修法：{options:?}");
        assert!(
            options
                .iter()
                .all(|option| !option.label.trim().is_empty() && !option.consequence.trim().is_empty()),
            "每个修法都必须带后果（owner 才能取舍）：{options:?}"
        );
        // 平台合成 finding（owner 面真正的载体）：一句人话问题 + 恰好两个修法，
        // 且带单源前缀（策略层据此识别平台轮，红线：不按 kind/轮次豁免）。
        let verdict = crate::product::workspace_engine::compile_failure_surface::
            auto_rewrite_verdict(&failure);
        let finding = verdict.findings.first().expect("platform finding");
        assert!(
            finding.message.starts_with(
                crate::product::workspace_engine::compile_failure_surface::
                    PLATFORM_COMPILE_FAILURE_FINDING_PREFIX
            ),
            "平台 finding 必须带单源前缀：{}",
            finding.message
        );
        assert!(
            finding.owner_question.as_deref().is_some_and(|question| {
                question.contains("WI-004") && question.contains("WI-003")
            }),
            "平台 finding 必须带 owner 能答的一句话问题：{:?}",
            finding.owner_question
        );
        assert_eq!(
            finding.owner_options.as_ref().map(Vec::len),
            Some(2),
            "平台 finding 必须带恰好两个 owner 选项：{:?}",
            finding.owner_options
        );
        let hint = owner_hint(&failure);
        assert!(
            hint.contains("两个合法修法"),
            "owner 提示必须列出两个修法：{hint}"
        );
        assert!(
            rewrite_guidance(&failure).len() >= 3,
            "给 author 的要点必须含那一跳事实 + 两个消环方向 + 保护项"
        );
        assert!(
            !owner_message(&typed).contains(STRUCTURE_STABLE_CODE),
            "人话段不得把稳定码甩给 owner"
        );
        let summary = author_gate_summary(&typed);
        assert!(
            summary.contains("WI-003") && summary.contains("WI-004") && summary.contains("仓序第"),
            "人工门摘要必须带真实位次与两个修法：{summary}"
        );

        // 结构指纹跨「改标题」稳定（同环同指纹），且 detail 里逐字可见。
        let mut retitled = ir.clone();
        for item in &mut retitled.items {
            item.contract.identity.title = format!("{}（作者改写版）", item.contract.identity.title);
        }
        let retitled_assessment =
            assess_single_candidate_structure(String::new(), &retitled, &context);
        let StructuralAssessment::Rejected(retitled_failures, _) = retitled_assessment else {
            panic!("改标题不改变依赖结构，仍必须被拦下");
        };
        let retitled_typed =
            author_structure_gate_failure(&retitled, &context, &retitled_failures);
        let fingerprint = |failure: &crate::product::workspace_engine::types::PlanningCompileFailure| {
            failure
                .render()
                .split(crate::product::workspace_engine::compile_failure_surface::
                    CYCLE_FINGERPRINT_MARKER)
                .nth(1)
                .and_then(|rest| rest.lines().next())
                .map(str::to_string)
                .expect("fingerprint segment")
        };
        assert_eq!(
            fingerprint(&typed),
            fingerprint(&retitled_typed),
            "同一依赖环必须得到同一结构指纹（不靠文案/标题）"
        );
        assert_ne!(
            typed.render(),
            retitled_typed.render(),
            "人话段可以随标题变化——指纹稳定性不靠文案一致"
        );
    }

    /// 环存在但**没有**逆仓序一跳（同仓互等）时：只给真实环链，不给位次与
    /// 选项（不虚构环形状 / 不编造修法）。
    #[test]
    fn same_repo_cycle_never_fabricates_orders_or_options() {
        use crate::product::workspace_engine::compile_failure_surface::cycle_witness_facts;
        use crate::product::workspace_engine::single_candidate_structure_failure::
            author_structure_gate_failure;

        let repos = Repos::new();
        let table = repos.table();
        let change_order = repos.change_order();
        // 同仓（repository_a）内互相依赖：有效图成环，但没有仓序逆序那一跳。
        let mut first = item("WI-001", "repository_a", None);
        let mut second = item("WI-002", "repository_a", Some("WI-001"));
        first.contract.input_contracts = second.contract.input_contracts.clone();
        first.contract.input_contracts[0].provider_logical_work_item_id =
            "WI-002".to_string();
        second.contract.input_contracts[0].provider_logical_work_item_id =
            "WI-001".to_string();
        let ir = ir_of(vec![first, second]);
        let context = structure_context(table, change_order);
        let assessment = assess_single_candidate_structure(String::new(), &ir, &context);
        let StructuralAssessment::Rejected(failures, _) = assessment else {
            panic!("同仓互等必须成环");
        };
        let typed = author_structure_gate_failure(&ir, &context, &failures);
        let failure = crate::product::workspace_engine::compile_failure_surface::
            candidate_compile_status(None, &typed, "2026-10-31T00:00:00Z".to_string())
            .0
            .failure
            .expect("durable failure projection");
        assert!(
            cycle_witness_facts(&failure.message).is_none(),
            "没有逆仓序一跳时不得编造位次与选项：{}",
            failure.message
        );
        assert!(
            failure.message.contains("构成依赖环"),
            "仍必须说清真实环链与消环方向：{}",
            failure.message
        );
    }
}
