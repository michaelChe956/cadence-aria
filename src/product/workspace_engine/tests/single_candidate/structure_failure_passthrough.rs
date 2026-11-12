// 5.3（REQ-WSC-24 残留）：Final Compile 的依赖环失败必须以 typed 结构失败
// （`PlanningCompileFailureKind::Structure`，稳定码 `STRUCTURE_STABLE_CODE`）
// 透传到失败面，不得再被包成 operational 字符串（`final_compile_prepare_failed`
// + cause 文本）。
//
// fixture 与 `structure_parity.rs` 同构（多仓、跨仓逆序消费、change_order
// 展开自动边成环）；harness 同 `final_compile_edges`：IR adapter → 纯 prepare。
// 失败面分流经生产单源 `compile/prepare_failure::into_compile_failure`
// （`compile/single_candidate.rs` 的 prepare 包裹点同一实现）。
mod structure_failure_passthrough {
    use std::collections::BTreeMap;
    use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
    use crate::product::models::{
        IssueWorkItemPlan, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
        WorkItemDraftVerificationPlan,
    };
    use crate::product::planning_failure::PlanningFailureKind;
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
    use crate::product::workspace_engine::compile::prepare_failure::into_compile_failure;
    use crate::product::workspace_engine::compile_failure_surface::{
        CYCLE_FINGERPRINT_MARKER, CYCLE_IDS_MARKER,
    };
    use crate::product::workspace_engine::types::{
        PlanningCompileFailure, PlanningCompileFailureKind,
    };
    use crate::product::workspace_engine::{
        InitialPlanCompileDurableContext, InitialPlanCompileInput,
        prepare_initial_plan_compile,
    };

    const SOURCE_HASH: &str = "structure-failure-passthrough-source-hash";

    /// 稳定 UUID（同 structure_parity 纪律：禁止运行时随机，保证图节点序可复现）。
    fn repo(seed: u8) -> LogicalRepositoryId {
        let mut bytes = [0u8; 16];
        bytes[0] = seed;
        LogicalRepositoryId(uuid::Uuid::from_bytes(bytes))
    }

    /// 现场 #1 形态（4.1 fixture）：gateway 与 frontend 两个托管成员仓，
    /// `change_order` 为 gateway→frontend（gateway 是前置仓）。
    struct Repos {
        gateway: LogicalRepositoryId,
        frontend: LogicalRepositoryId,
    }

    impl Repos {
        fn new() -> Self {
            Self {
                gateway: repo(0x0A),
                frontend: repo(0x0B),
            }
        }

        /// 授权成员表（逻辑成员 → 物理仓），与 `physical_by_logical` 同向。
        fn table(&self) -> BTreeMap<LogicalRepositoryId, String> {
            BTreeMap::from([
                (self.gateway, "repository_gateway".to_string()),
                (self.frontend, "repository_frontend".to_string()),
            ])
        }

        /// confirmed design 的仓级执行顺序：gateway → frontend。
        fn change_order(&self) -> Vec<LogicalRepositoryId> {
            vec![self.gateway, self.frontend]
        }
    }

    fn contract(
        logical_id: &str,
        provider: Option<&str>,
        physical: &str,
    ) -> CanonicalWorkItemContract {
        let mut contract = canonical_contract_fixture(logical_id);
        // 呈现名与逻辑身份同值：终编译环链按 title 渲染，保持与
        // structure_parity 的逐字可比纪律。
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
            target: EnrollmentTarget::SingleRepository {
                repository_id: physical.to_string(),
            },
        });
        contract
    }

    /// 现场形态（D2）：每个 WI 都用 `single_repository` 声明自己托管的物理仓。
    fn item(logical_id: &str, physical: &str, provider: Option<&str>) -> PlanCandidateItemIr {
        PlanCandidateItemIr {
            target_repository_id: physical.to_string(),
            contract: contract(logical_id, provider, physical),
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

    fn previous_plan() -> IssueWorkItemPlan {
        IssueWorkItemPlan {
            id: "plan_structure_passthrough".to_string(),
            project_id: "project_structure_passthrough".to_string(),
            issue_id: "issue_structure_passthrough".to_string(),
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

    /// 终编译 prepare 边界（同 `final_compile_edges` 的 harness）：
    /// IR adapter → `initial_plan_compile_input_from_ir` → 纯 prepare。
    fn compile_input(
        table: &BTreeMap<LogicalRepositoryId, String>,
        change_order: &[LogicalRepositoryId],
        ir: &PlanCandidateIr,
    ) -> InitialPlanCompileInput {
        let context = IrCompileAdapterContext {
            project_id: "project_structure_passthrough".to_string(),
            issue_id: "issue_structure_passthrough".to_string(),
            plan_id: "plan_structure_passthrough".to_string(),
            previous_plan: previous_plan(),
            source_revision_id: "source_structure_passthrough".to_string(),
            source_revision_ref: "plan-source/structure-passthrough".to_string(),
            plan_candidate_ir_ref: "plan-ir/structure-passthrough".to_string(),
            mechanical_report_ref: "mechanical-report/structure-passthrough".to_string(),
            publication_provenance_ref: "publication-provenance/structure-passthrough".to_string(),
            logical_targets: Some(table.clone()),
            // LC 多仓：legacy 单仓 target 不参与（与生产 SC 链同口径）。
            repository_id: String::new(),
            change_order: change_order.to_vec(),
            compile_id: "compile_structure_passthrough".to_string(),
            now: "2026-10-31T00:00:00Z".to_string(),
        };
        let report = PlanCandidateMechanicalReport {
            source_revision_hash: SOURCE_HASH.to_string(),
            compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
            findings: Vec::new(),
        };
        initial_plan_compile_input_from_ir(&context, ir, &report)
            .expect("IR adapter 必须接受同构 fixture")
    }

    /// 多仓环 fixture（4.1 现场 #1 形态）：gateway 托管仓序第 1 的成员仓，
    /// 却消费 frontend（仓序第 2）的产出——canonical 边 frontend→gateway 与
    /// change_order 自动边 gateway→frontend 闭合出环。
    fn cycle_ir() -> PlanCandidateIr {
        ir_of(vec![
            item("WI-GATEWAY", "repository_gateway", Some("WI-FRONTEND")),
            item("WI-FRONTEND", "repository_frontend", None),
        ])
    }

    /// 5.3 透传：环失败以 typed 结构失败（Structure）到达失败面，稳定码 =
    /// `STRUCTURE_STABLE_CODE`，`final_compile_prepare_failed` 码不再出现于
    /// 该形态；结构指纹与机器 id 标记段（5.4 同环重复停判定的锚点）随渲染保留。
    #[test]
    fn cycle_failure_reaches_surface_as_structure_kind() {
        let repos = Repos::new();
        let table = repos.table();
        let change_order = repos.change_order();
        let ir = cycle_ir();
        let input = compile_input(&table, &change_order, &ir);
        let err = prepare_initial_plan_compile(input, InitialPlanCompileDurableContext::legacy())
            .expect_err("多仓环 fixture 必须被终编译 prepare 拒绝");

        // 失败面（compile/single_candidate 的 prepare 包裹点同一分流单源）。
        let surface = into_compile_failure(err);
        let PlanningCompileFailureKind::Structure(set) = &surface.kind else {
            panic!("环形态必须以 typed 结构失败（Structure）透传：{surface:?}");
        };
        assert_eq!(set.failures.len(), 1, "集合恰一条环失败：{set:?}");
        assert!(
            matches!(
                set.failures[0].kind,
                PlanningFailureKind::DependencyCycle { .. }
            ),
            "环失败的 kind 必须是 DependencyCycle：{:?}",
            set.failures[0].kind
        );
        assert!(
            !set.failures[0].evidence.affected_ids.is_empty(),
            "机器 id 标记段必须还原 affected_ids：{:?}",
            set.failures[0].evidence.affected_ids
        );
        assert_eq!(
            surface.stable_code(),
            PlanningCompileFailure::STRUCTURE_STABLE_CODE
        );
        let rendered = surface.render();
        assert!(
            !rendered.contains("final_compile_prepare_failed"),
            "环形态不得再出现 final_compile_prepare_failed 码：{rendered}"
        );
        // 5.4：结构指纹与机器 id 标记段随渲染保留（同环重复停判定不回退）。
        assert!(rendered.contains(CYCLE_FINGERPRINT_MARKER), "{rendered}");
        assert!(rendered.contains(CYCLE_IDS_MARKER), "{rendered}");
        assert!(
            rendered.contains("「WI-GATEWAY」 → 「WI-FRONTEND」 → 「WI-GATEWAY」"),
            "环链人话事实必须保留：{rendered}"
        );
    }

    /// 兼容面：prepare 的 operational 形态仍产
    /// `operational("final_compile_prepare_failed", cause)`（稳定码保留一个
    /// 版本周期的兼容面）。
    #[test]
    fn operational_failure_keeps_final_compile_prepare_failed_code() {
        let repos = Repos::new();
        let table = repos.table();
        let change_order = repos.change_order();
        let ir = cycle_ir();
        let mut input = compile_input(&table, &change_order, &ir);
        // 纯 prepare 的 operational 失败形态（非环）。
        input.outline_order = Vec::new();
        let err = prepare_initial_plan_compile(input, InitialPlanCompileDurableContext::legacy())
            .expect_err("空 outline_order 必须被 prepare 以 operational 拒绝");
        let surface = into_compile_failure(err);
        let PlanningCompileFailureKind::Operational { stable_code, cause } = &surface.kind else {
            panic!("非环失败必须保持 Operational 形态：{surface:?}");
        };
        assert_eq!(stable_code, "final_compile_prepare_failed");
        assert!(cause.contains("at least one outline"), "{cause}");
        assert_eq!(surface.stable_code(), "final_compile_prepare_failed");
        assert_eq!(
            surface.render(),
            format!("[final_compile_prepare_failed] {cause}")
        );
    }
}
