//! it_web 侧「已确认能力」播种助手。
//!
//! 生产语义（`4749a1fc` 起）：bootstrap 行保持 `Unknown`，只有真实 probe 才能
//! 写入 `Confirmed`（`Unknown + supported_actions` 迁移桥已退役）。因此正向
//! fixture 若要让会话经 gateway 启动，必须在 `ensure_bootstrap` 之后显式物化
//! 「已探测」事实，否则 launch 分格 fail-closed 为
//! `provider_capability_launch_not_confirmed`。
//!
//! 记录形状与 lib target 内的
//! `crate::web::handlers::automation_enrollment_test_support::confirmed_capability_record`
//! 一致（exact version + dialect + 逐 action 三态证据 + host-open 语义标识 +
//! snapshot ref 与 gateway `ProviderRef` 对齐）；该 helper 是 `#[cfg(test)]`
//! 的 lib 私有项，it_web（独立 target）取不到，故在此复刻。
//!
//! 负向场景（断言「能力未确认」被拒）**不得**调用本助手，保留 bootstrap
//! `Unknown` 行即可——本助手只用于正向路径，不放宽产品 fail-closed 面。

use cadence_aria::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN;
use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::policy::{
    ProviderDialect, ProviderWireDialect, SessionPolicyAction,
};
use cadence_aria::product::logical_codebase::provider_capability_store::{
    CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
    ProviderActionMatrix, ProviderCapabilityRecord, ProviderCapabilityStore, RootRecipeEvidence,
};
use cadence_aria::product::logical_codebase::provider_gateway::{
    ProviderRefType, ResumeEvidenceState,
};

/// 三个 canonical action 行全 `Confirmed` 的 capability 记录（trust 亦
/// `Confirmed`，执行语义 host-open，snapshot ref 与 gateway
/// `claude_code("cap_managed_snapshot")` 对齐）。
fn confirmed_capability_record(provider_type: ProviderRefType) -> ProviderCapabilityRecord {
    let (adapter_dialect, wire_dialect) = match provider_type {
        ProviderRefType::ClaudeCode => (
            ProviderDialect::ClaudeCodeCliV1,
            ProviderWireDialect::ClaudeCodeStreamJson,
        ),
        ProviderRefType::Codex => (
            ProviderDialect::CodexCliV1,
            ProviderWireDialect::CodexAppServerRpc,
        ),
        ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
        ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
    };
    let row = |action: SessionPolicyAction| ProviderActionCapability {
        action,
        launch: ProviderCapabilityEvidence::Confirmed,
        resume: ProviderCapabilityEvidence::Confirmed,
        write_boundary: ProviderCapabilityEvidence::Confirmed,
        projection_digest: format!("projection-digest-{action:?}"),
        evidence_ref: format!("probe://{action:?}"),
    };
    ProviderCapabilityRecord {
        provider_type,
        schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
        version: "1.4.0".to_string(),
        adapter_dialect,
        wire_dialect,
        capability_snapshot_ref: "cap_managed_snapshot".to_string(),
        evidence: CapabilityEvidence::ProductionVerified,
        resume_evidence: ResumeEvidenceState::Unsupported,
        supported_actions: Vec::new(),
        action_matrix: ProviderActionMatrix::from_rows(vec![
            row(SessionPolicyAction::PlanningReadOnly),
            row(SessionPolicyAction::CodingTargetWrite),
            row(SessionPolicyAction::ReviewReadOnly),
        ])
        .expect("three canonical capability rows"),
        trust: ProviderCapabilityEvidence::Confirmed,
        probed_at: None,
        probe_artifact_ref: None,
        root_recipe_evidence: RootRecipeEvidence::None,
        execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
    }
}

/// 先物化 bootstrap 行（保持生产「未探测」语义），再把指定 provider 覆盖写为
/// `Confirmed`。
///
/// capability 记录的**唯一作用域 = 项目级全局**（owner 2026-11-04 裁定：能力描述
/// 的是「本机上某家 provider CLI 能不能跑这个 action」，与逻辑代码库的代码内容
/// 无关 ⇒ 一次核验对项目内全部逻辑代码库同时生效）。⇒ 播种只认项目，不再按
/// LC 子树分叉（原 `lc_id` 参数随之退役）。
pub(crate) fn seed_confirmed_capability(
    paths: &ProductAppPaths,
    _project_id: &str,
    providers: &[ProviderRefType],
) {
    let store = ProviderCapabilityStore::new(paths.clone());
    store
        .ensure_bootstrap()
        .expect("bootstrap capability records");
    for provider_type in providers {
        store
            .upsert(&confirmed_capability_record(*provider_type))
            .expect("seed confirmed capability record");
    }
}

/// 常用形态：仅把 ClaudeCode（gateway 多仓/逻辑路径的默认 provider）覆盖写为
/// `Confirmed`。
pub(crate) fn seed_confirmed_claude(paths: &ProductAppPaths, project_id: &str) {
    seed_confirmed_capability(paths, project_id, &[ProviderRefType::ClaudeCode]);
}

/// 片4（REQ-BOOT-07）：为「期望创建成功」的 it_web 夹具注入全可用
/// provider health——stub runner 让四家版本探测全部成功（版本取各家
/// 最低兼容版本，与 web_provider_health_api 的探测断言同源），非降级、
/// 无真实探针。聚合初始化创建链的选择器（L1）据此确定性通过；负向
/// 场景（断言选择失败面）不经本助手，保留默认 health 语义。
pub(crate) fn with_available_provider_health(
    state: cadence_aria::web::state::WebAppState,
    root: &std::path::Path,
) -> cadence_aria::web::state::WebAppState {
    struct AllAvailableRunner;
    #[async_trait::async_trait]
    impl cadence_aria::cross_cutting::bounded_command_runner::BoundedCommandRunner
        for AllAvailableRunner
    {
        async fn run(
            &self,
            request: cadence_aria::cross_cutting::bounded_command_runner::BoundedCommandRequest,
        ) -> Result<
            cadence_aria::cross_cutting::bounded_command_runner::BoundedCommandResult,
            cadence_aria::cross_cutting::bounded_command_runner::BoundedCommandError,
        > {
            let version = match request.executable.as_str() {
                "codex" => "2.0".to_string(),
                "pi" => "0.83.0".to_string(),
                "kimi" => "0.34.0".to_string(),
                // 聚合冷启动环境探针（CodeGraphEnvironmentProbe，生产依赖
                // 装配复用 state.command_runner + executable "codegraph"）与
                // provider 健康探测共用本 runner：`codegraph --version` 必须
                // 答区间内版本，否则 member_index 步被生产版本门正确拒绝为
                // codegraph_version_mismatch。取生产声明的区间下界，区间
                // 上调时自动跟随（与 FakeCodeGraphCli 的 1.6.0 应答同源）。
                "codegraph" => format!(
                    "{}.{}.{}",
                    cadence_aria::product::logical_codebase::aggregate_index::CODEGRAPH_MIN_VERSION
                        .0,
                    cadence_aria::product::logical_codebase::aggregate_index::CODEGRAPH_MIN_VERSION
                        .1,
                    cadence_aria::product::logical_codebase::aggregate_index::CODEGRAPH_MIN_VERSION
                        .2,
                ),
                _ => "1.0".to_string(),
            };
            Ok(
                cadence_aria::cross_cutting::bounded_command_runner::BoundedCommandResult {
                    exit_code: Some(0),
                    stdout: version,
                    stderr: String::new(),
                    timed_out: false,
                    cancelled: false,
                    stdout_truncated: false,
                    stderr_truncated: false,
                    duration_ms: 1,
                },
            )
        }
    }
    let runner: std::sync::Arc<
        dyn cadence_aria::cross_cutting::bounded_command_runner::BoundedCommandRunner,
    > = std::sync::Arc::new(AllAvailableRunner);
    let health = std::sync::Arc::new(
        cadence_aria::cross_cutting::provider_health::ProviderHealthService::with_dependencies(
            cadence_aria::cross_cutting::aria_state_paths::AriaStatePaths::from_workspace_root(
                root,
            ),
            runner.clone(),
            std::sync::Arc::new(
                cadence_aria::cross_cutting::provider_health::SystemProviderHealthClock,
            ),
            std::time::Duration::from_secs(1),
            4096,
        ),
    );
    let gate = std::sync::Arc::new(
        cadence_aria::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(
            health.clone(),
        ),
    );
    state.with_provider_health(health, gate, runner)
}

/// 生产 machine_skills 步骤在 fake runtime 下以 workspace root 为 home——
/// 预置本地技能源目录，避免在线 clone（与 web 层生产依赖测试同构）。
/// 四条聚合命令必须一并预置：链接层校验（ensure_skills_commands_linked）
/// 要求 shared/codex/claude 三层都能解析 AGGREGATE_SKILL_COMMANDS，缺一即
/// skills_commands_missing 阻断 member_index，注入的 provider 中断不可达。
pub(crate) fn seed_local_skills_source(root: &std::path::Path) {
    let skills_root = root.join(".agents/Cadence-skills/cadence-init/skills");
    let seed_skill = |name: &str, body: String| {
        let dir = skills_root.join(name);
        std::fs::create_dir_all(&dir).expect("create skills source");
        std::fs::write(dir.join("SKILL.md"), body).expect("write SKILL.md");
    };
    seed_skill("demo", "# demo\n".to_string());
    for command in
        cadence_aria::product::logical_codebase::environment_probe::AGGREGATE_SKILL_COMMANDS
    {
        seed_skill(command, format!("# {command}\n"));
    }
}
