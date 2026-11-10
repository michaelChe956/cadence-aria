//! 知识库创建期前置检查（Task 4，只读、零副作用、不产生 operation）。
//!
//! 对应技术方案 §5.3 的预检 ①-⑥ 与 ⑦ 的**能力格半边**：预检 ⑦ 的
//! provider 选择（按 `CodingTargetWrite` 分格挑候选）与 ⑧（同 LC 单飞
//! 检查）需要 health 快照与 project 作用域，由 HTTP 层经
//! [`crate::product::logical_codebase::provider_selection::select_aggregate_provider_for_write`]
//! 与 `KnowledgeBaseOperationStore::list_running` 完成后，映射进本模块的
//! `KnowledgeBasePreflightFailure` 统一失败面；但被选 provider 的
//! `CodingTargetWrite` 分格**是否 `launch` 与 `write_boundary` 双
//! `Confirmed`** 是机器级 capability 记录上的同步可判定事实，由本模块的
//! [`preflight_write_capability`] 产出（见 [`KnowledgeBasePreflightFailure::CapabilityDenied`]）。

use std::path::Path;

use crate::product::app_paths::ProductAppPaths;
use crate::product::cadence_skills::CadenceSkillsPaths;
use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::production_policy_resolvers::StoreBackedProviderCapabilitySource;
use crate::product::logical_codebase::provider_capability_store::{
    MANAGED_CAPABILITY_SNAPSHOT_REF, ProviderCapabilityStore,
};
use crate::product::logical_codebase::provider_gateway::ProviderRef;
use crate::product::logical_codebase::provider_selection::ProviderSelectionOutcome;
use crate::product::logical_codebase::provider_trust::provider_wire_label;
use crate::product::logical_codebase::provider_trust_adapters::resolve_owner_home_from_env;
use crate::product::logical_codebase::{
    LogicalCodebaseBootstrapProjector, LogicalCodebaseBootstrapStep,
    LogicalCodebaseBootstrapStepStatus,
};
use crate::product::models::ProviderName;

use super::change_package::{ChangePackageViolation, MemberRepoPaths, lint_change_package};
use super::files::kb_root;
use super::manifest::{manifest_invariants, read_manifest};

/// 知识库创建期前置检查的统一失败面（稳定码登记在 Task 7，本模块只产出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeBasePreflightFailure {
    /// 预检①：聚合初始化未完成（冷启动投影 `rules_policy` 步未 Completed）。
    RequiresAggregateInitialization,
    /// 预检②：被选 provider 对应 skill 层缺 `knowledge-base-*` skill。
    SkillsUnavailable { missing: Vec<String> },
    /// 预检③：六领域输入按状态条件仍有缺口（`database-ddl.sql` / `product.md` 可选）。
    InputsIncomplete { missing: Vec<String> },
    /// 预检④：`manifest.yaml` 不存在（读取时的环境级 IO 故障同落此处：证明不了在场）。
    ManifestMissing,
    /// 预检④：`schema_version` 不是 "4.0"（文件在但版本未知记 `"unknown"`）。
    ManifestVersionUnsupported { found: String },
    /// 预检⑤：初始化未完成（`status != complete` 或 `global_validation != passed`）。
    NotInitialized,
    /// 预检⑥：变更包平台侧机械门禁违规（逐条透出）。
    ChangePackageInvalid {
        violations: Vec<ChangePackageViolation>,
    },
    /// 预检⑦：写分格 provider 选择结论非 `Selected`（四枚选择期失败码原样透出）。
    /// 由 HTTP 层在调用 [`crate::product::logical_codebase::provider_selection::select_aggregate_provider_for_write`]
    /// 之后构造；`preflight_*` 本身不产出（选择需要 health 快照，是异步面）。
    ProviderSelection(ProviderSelectionOutcome),
    /// 预检⑦：被选 provider 的 `CodingTargetWrite` 分格未同时满足
    /// `launch` 与 `write_boundary` 双 `Confirmed`（由
    /// [`preflight_write_capability`] 产出：机器级 capability 记录上的
    /// 同步可判定事实；缺记录/快照漂移同样 fail-closed 落此变体）。
    CapabilityDenied { provider: String },
    /// 预检⑧：同 LC 已有知识库 operation 在跑（durable 单飞；HTTP 层经
    /// `KnowledgeBaseOperationStore::list_running` 构造）。
    OtherKnowledgeBaseRunning { operation_id: String },
}

/// 知识库 skills 可用性判定的固定名单（7 个；源根缺任一 ⇒ Cadence-skills
/// 未装或版本过旧，失败面指向「先在 Provider 管理里安装/更新」）。
const KNOWLEDGE_BASE_SKILLS: [&str; 7] = [
    "knowledge-base-bootstrap",
    "knowledge-base-base-info",
    "knowledge-base-api",
    "knowledge-base-pages",
    "knowledge-base-overview",
    "knowledge-base-context",
    "knowledge-base-update",
];

/// 六领域 → scope 文件（顺序即 base-info.md 模板的领域顺序）。
const SIX_DOMAINS: [(&str, &str); 6] = [
    ("工程信息", "project-scope.md"),
    ("数据模型", "data-model-scope.md"),
    ("配置", "configuration-scope.md"),
    ("中间件", "middleware-scope.md"),
    ("接口", "api-scope.md"),
    ("页面", "page-scope.md"),
];

/// 知识库 Manifest 唯一支持的 schema 版本（skill 契约）。
const SCHEMA_VERSION_4: &str = "4.0";

/// bootstrap 前置检查（预检①②③⑦，只读、不产生 operation）。
pub fn preflight_bootstrap(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
    aggregate_root: &Path,
    provider: &ProviderName,
) -> Result<(), KnowledgeBasePreflightFailure> {
    preflight_common(paths, project_id, lc_id, provider)?;
    let missing = missing_bootstrap_inputs(aggregate_root);
    if !missing.is_empty() {
        return Err(KnowledgeBasePreflightFailure::InputsIncomplete { missing });
    }
    preflight_write_capability(paths, provider)?;
    Ok(())
}

/// update 前置检查（预检①②④⑤⑥⑦，只读、不产生 operation）。
pub fn preflight_update(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
    aggregate_root: &Path,
    provider: &ProviderName,
    package_dir: &Path,
    members: &[MemberRepoPaths],
) -> Result<(), KnowledgeBasePreflightFailure> {
    preflight_common(paths, project_id, lc_id, provider)?;
    let snapshot = read_manifest(&kb_root(aggregate_root))
        .map_err(|_| KnowledgeBasePreflightFailure::ManifestMissing)?;
    if !snapshot.exists {
        return Err(KnowledgeBasePreflightFailure::ManifestMissing);
    }
    let found = snapshot
        .schema_version
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    if found != SCHEMA_VERSION_4 {
        return Err(KnowledgeBasePreflightFailure::ManifestVersionUnsupported { found });
    }
    if snapshot.initialization_status.as_deref() != Some("complete")
        || snapshot.global_validation.as_deref() != Some("passed")
    {
        return Err(KnowledgeBasePreflightFailure::NotInitialized);
    }
    // 状态字面写着 complete/passed 不等于初始化有效完成：必需阶段缺失、阶段重复或
    // 乱序、completed_at 为空、适用领域被错误跳过等不变量违规，一律按未完成拒绝
    // （lint 消费端不再校验这些，不能指望下游兜底）。
    if !manifest_invariants(&snapshot).is_empty() {
        return Err(KnowledgeBasePreflightFailure::NotInitialized);
    }
    let lint = lint_change_package(package_dir, &snapshot, members);
    if !lint.violations.is_empty() {
        return Err(KnowledgeBasePreflightFailure::ChangePackageInvalid {
            violations: lint.violations,
        });
    }
    preflight_write_capability(paths, provider)?;
    Ok(())
}

/// 预检①②（bootstrap 与 update 共用，顺序固定：先聚合初始化，后 skills）。
fn preflight_common(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
    provider: &ProviderName,
) -> Result<(), KnowledgeBasePreflightFailure> {
    if !aggregate_initialization_completed(paths, project_id, lc_id) {
        return Err(KnowledgeBasePreflightFailure::RequiresAggregateInitialization);
    }
    let missing = missing_knowledge_base_skills(provider);
    if !missing.is_empty() {
        return Err(KnowledgeBasePreflightFailure::SkillsUnavailable { missing });
    }
    Ok(())
}

/// 「聚合初始化已完成」= 冷启动投影的 `rules_policy` 步 `status == Completed`
/// （唯一口径，与 readiness 页面 / advance 推进器同源：权威冻结的政策引用、
/// 已发布政策 artifact、Completed 根配方 operation 与最终 receipt 三源全一致
/// 才投影 Completed，见 [`LogicalCodebaseBootstrapProjector::project`]）。旧的
/// 「根规则文件 + 政策 artifact」两文件判定已删除，不并存两套口径；投影读不出
/// （LC/项目不存在、身份冲突）或 IO 故障都证明不了「已完成」，一律判 false
/// （fail-closed）。
pub fn aggregate_initialization_completed(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> bool {
    let Ok(projection) = LogicalCodebaseBootstrapProjector::new(paths.clone())
        .project(project_id, logical_codebase_id)
    else {
        return false;
    };
    projection.steps.iter().any(|step| {
        step.step == LogicalCodebaseBootstrapStep::RulesPolicy
            && step.status == LogicalCodebaseBootstrapStepStatus::Completed
    })
}

/// 预检②：被选 provider 对应 skill 层缺哪些 knowledge-base skill。
/// home 解析不出、或 provider 没有对应层（Kimi/Fake 不在知识库候选内）
/// ⇒ 七个全缺（fail-closed，不猜层）。
fn missing_knowledge_base_skills(provider: &ProviderName) -> Vec<String> {
    let Some(home) = resolve_owner_home_from_env() else {
        return all_knowledge_base_skills_missing();
    };
    let skills_paths = CadenceSkillsPaths::from_home(&home);
    let Some(layer) = provider_skill_layer(&skills_paths, provider) else {
        return all_knowledge_base_skills_missing();
    };
    KNOWLEDGE_BASE_SKILLS
        .iter()
        .filter(|skill| !layer.join(skill).join("SKILL.md").is_file())
        .map(|skill| skill.to_string())
        .collect()
}

fn all_knowledge_base_skills_missing() -> Vec<String> {
    KNOWLEDGE_BASE_SKILLS
        .iter()
        .map(|skill| skill.to_string())
        .collect()
}

/// provider → skill 层（Claude → claude 层、Pi → shared 层、Codex → codex 层）。
fn provider_skill_layer<'a>(
    paths: &'a CadenceSkillsPaths,
    provider: &ProviderName,
) -> Option<&'a Path> {
    match provider {
        ProviderName::ClaudeCode => Some(paths.claude_skills_root()),
        ProviderName::Pi => Some(paths.shared_skills_root()),
        ProviderName::Codex => Some(paths.codex_skills_root()),
        _ => None,
    }
}

/// 预检③：`user-input/base-info.md`（唯一入口，必需）+ 按领域「状态：」
/// 条件判定六个 scope 文件；`database-ddl.sql` / `product.md` 可选，不判。
fn missing_bootstrap_inputs(aggregate_root: &Path) -> Vec<String> {
    let user_input = kb_root(aggregate_root).join("user-input");
    let Ok(text) = std::fs::read_to_string(user_input.join("base-info.md")) else {
        return vec!["base-info.md".to_string()];
    };
    let mut missing = Vec::new();
    for (domain, file) in SIX_DOMAINS {
        if domain_requires_scope_file(domain, &text) && !user_input.join(file).is_file() {
            missing.push(file.to_string());
        }
    }
    missing
}

/// 领域状态含「不适用」（含带括号原因的形态）⇒ 不要求该文件；其余
/// （`全量` / `指定` / 空 / 未知）一律要求——证明不了「不适用」就当必填。
fn domain_requires_scope_file(domain: &str, text: &str) -> bool {
    !matches!(domain_status(domain, text), Some(status) if status.contains("不适用"))
}

/// 取领域章节（`## <领域>` 起、到下一个 `##` 止）里的首个「- 状态：」值；
/// 章节缺失或没有状态行 ⇒ `None`（按必填处理）。
fn domain_status(domain: &str, text: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            in_section = heading.starts_with(domain);
        } else if in_section {
            let trimmed = line.trim_start_matches('-').trim();
            if let Some(rest) = trimmed.strip_prefix("状态")
                && let Some(value) = rest.strip_prefix('：').or_else(|| rest.strip_prefix(':'))
            {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// 预检⑦：被选 provider 的 `CodingTargetWrite` 分格必须 `launch` 与
/// `write_boundary` **双 `Confirmed`**（fresh 门两半，D1）——创建 operation
/// 之前确认，缺任一半即按能力拒绝失败面返回，不产生 operation。
///
/// 判定与 gateway / 写分格选择器**同源**：capability 记录（机器级）经
/// [`StoreBackedProviderCapabilitySource::load_record`] 载入——缺记录、快照
/// 引用不一致、IO 故障一律 fail-closed 拒绝；分格门复用既有
/// [`ProviderActionCapability::fresh_gate`]（`launch` + `write_boundary`
/// 两半，与 gateway `require_supported` + `require_write_boundary` 同一对
/// 分格）。任一不是 `Confirmed` ⇒
/// [`KnowledgeBasePreflightFailure::CapabilityDenied`]。
pub fn preflight_write_capability(
    paths: &ProductAppPaths,
    provider: &ProviderName,
) -> Result<(), KnowledgeBasePreflightFailure> {
    let denied = || KnowledgeBasePreflightFailure::CapabilityDenied {
        provider: provider_wire_label(provider).to_string(),
    };
    let provider_ref = ProviderRef::from_provider_name(provider, MANAGED_CAPABILITY_SNAPSHOT_REF)
        .map_err(|_| denied())?;
    let source = StoreBackedProviderCapabilitySource::with_store(ProviderCapabilityStore::new(
        paths.clone(),
    ));
    let record = source.load_record(&provider_ref).map_err(|_| denied())?;
    record
        .action_matrix
        .row(&SessionPolicyAction::CodingTargetWrite)
        .fresh_gate()
        .map_err(|_| denied())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;
    use crate::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN;
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::cadence_skills::CadenceSkillsPaths;
    use crate::product::logical_codebase::policy::{
        AggregatePolicyArtifact, AggregatePolicyArtifactStore, ProviderDialect, ProviderWireDialect,
    };
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord,
    };
    use crate::product::logical_codebase::provider_gateway::{
        ProviderRefType, ResumeEvidenceState,
    };
    use crate::product::logical_codebase::root_recipe_receipt::{
        ROOT_RULE_ENTRY_FILE, RootRecipeCommandVerdict, RootRecipeFilesystemAuditor,
        RootRecipeReceiptStore, root_rule_digest,
    };
    use crate::product::logical_codebase::store::{
        LogicalCodebaseCreateInput, LogicalCodebaseStore,
    };
    use crate::product::logical_codebase::{
        AggregateInitializationOperation, AggregateInitializationOperationInput,
        AggregateInitializationOperationStore, AggregateInitializationStepKind,
        root_recipe_command_index,
    };
    use crate::product::models::ProviderName;
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    use sha2::{Digest, Sha256};

    const PROJECT_ID: &str = "project_0001";

    // ------------------------------------------------------------------
    // HOME 互斥锚：与本测试二进制内其它改写 HOME 的用例（gateway_factory
    // 的 T7HomeEnvGuard、workspace_session 的 part_01 等）共用同一 sidecar
    // 文件锁，串行改写，避免并行用例读到别人的临时 home。
    // ------------------------------------------------------------------

    fn home_env_lock_target() -> PathBuf {
        std::env::temp_dir().join("aria-t7-home-env")
    }

    struct HomeEnvGuard {
        _lock: crate::product::coding_attempt_store::locking::ExclusiveFileLock,
        previous: Option<OsString>,
    }

    impl HomeEnvGuard {
        fn isolate(home: &std::path::Path) -> Self {
            let _lock = crate::product::coding_attempt_store::locking::ExclusiveFileLock::acquire(
                &home_env_lock_target(),
            )
            .expect("serialize HOME mutation across the test binary");
            let previous = std::env::var_os("HOME");
            // SAFETY: 与其它改写 HOME 的测试经同一文件锁串行；持有期间
            // 本用例对 HOME 的消费只有 skill 层路径拼接。
            unsafe { std::env::set_var("HOME", home) };
            Self { _lock, previous }
        }
    }

    impl Drop for HomeEnvGuard {
        fn drop(&mut self) {
            // SAFETY: 同上，文件锁串行下的恢复写。
            unsafe {
                if let Some(previous) = self.previous.clone() {
                    std::env::set_var("HOME", previous);
                } else {
                    std::env::remove_var("HOME");
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // 夹具
    // ------------------------------------------------------------------

    struct PreflightFixture {
        _workspace: TempDir,
        _home: TempDir,
        paths: ProductAppPaths,
        aggregate_root: PathBuf,
        lc_id: String,
    }

    impl PreflightFixture {
        fn new() -> Self {
            let workspace = TempDir::new().expect("workspace root");
            let home = TempDir::new().expect("isolated home");
            let paths = ProductAppPaths::new(workspace.path().join(".aria"));
            let aggregate_root = workspace.path().join("aggregate");
            fs::create_dir_all(&aggregate_root).expect("create aggregate root");
            // 预检①的新口径直接消费冷启动投影：LC 身份必须先经既有 store
            // 正规登记（record.json），不再手工摆放 manifest.json 反查归属。
            ProjectStore::new(paths.clone())
                .create(CreateProjectInput {
                    name: "knowledge-base-preflight".to_string(),
                    description: None,
                })
                .expect("create project");
            let lc_id = LogicalCodebaseStore::new(paths.clone())
                .create(
                    PROJECT_ID,
                    LogicalCodebaseCreateInput {
                        name: "knowledge-base-lc".to_string(),
                        aggregate_root: aggregate_root.clone(),
                    },
                )
                .expect("create logical codebase")
                .id;
            Self {
                _workspace: workspace,
                _home: home,
                paths,
                aggregate_root,
                lc_id,
            }
        }

        fn home_path(&self) -> &std::path::Path {
            self._home.path()
        }

        fn kb_root(&self) -> PathBuf {
            kb_root(&self.aggregate_root)
        }

        /// 预检①：种齐冷启动投影 `rules_policy` 步 Completed 的全部 durable
        /// 事实——真实（非自举桩）政策 artifact、五步全 Completed 的根配方
        /// operation、AGENTS.md 根规则与四命令审计全 Allowed 的最终
        /// receipt（policy/rule digest 与当前事实一致）。
        fn seed_aggregate_initialization(&self) {
            let policy = self.real_policy();
            AggregatePolicyArtifactStore::for_lc(self.paths.clone(), self.lc_id.clone())
                .save(PROJECT_ID, &policy)
                .expect("save policy artifact");

            let operation_id = "aggregate_initialization_preflight_0001".to_string();
            let init_store = AggregateInitializationOperationStore::for_lc(
                self.paths.clone(),
                self.lc_id.clone(),
            );
            init_store
                .create_idempotent(AggregateInitializationOperation::new(
                    operation_id.clone(),
                    PROJECT_ID.to_string(),
                    AggregateInitializationOperationInput {
                        idempotency_key: format!("preflight-{operation_id}"),
                        manifest_revision: 1,
                        policy_digest: policy.digest.clone(),
                        profile_evidence_digest: Some("sha256:profile".to_string()),
                        provider_context_root: self.aggregate_root.clone(),
                        provider: "claude_code".to_string(),
                    },
                    "2026-11-10T00:01:00Z".to_string(),
                ))
                .expect("create init operation");
            init_store
                .mark_running(
                    PROJECT_ID,
                    &operation_id,
                    "2026-11-10T00:02:00Z".to_string(),
                )
                .expect("mark operation running");
            for step in AggregateInitializationStepKind::V1 {
                init_store
                    .mark_step_running(
                        PROJECT_ID,
                        &operation_id,
                        step,
                        format!("preflight:{operation_id}:{}", step.as_str()),
                        "2026-11-10T00:03:00Z".to_string(),
                    )
                    .expect("mark step running");
                init_store
                    .checkpoint_step_output(
                        PROJECT_ID,
                        &operation_id,
                        step,
                        format!("aggregate-initializations/op/{}.json", step.as_str()),
                        "2026-11-10T00:04:00Z".to_string(),
                    )
                    .expect("checkpoint step output");
                init_store
                    .mark_step_completed(
                        PROJECT_ID,
                        &operation_id,
                        step,
                        "2026-11-10T00:05:00Z".to_string(),
                    )
                    .expect("mark step completed");
            }
            init_store
                .finish_completed(
                    PROJECT_ID,
                    &operation_id,
                    "2026-11-10T00:06:00Z".to_string(),
                )
                .expect("finish operation completed");

            fs::write(
                self.aggregate_root.join(ROOT_RULE_ENTRY_FILE),
                "# aggregate root rules\n",
            )
            .expect("write root rule entry");
            let canonical_root =
                fs::canonicalize(&self.aggregate_root).expect("canonicalize aggregate root");
            let rule_digest = root_rule_digest(&canonical_root)
                .expect("read root rule digest")
                .expect("root rule entry present");
            let receipt_store =
                RootRecipeReceiptStore::for_lc(self.paths.clone(), self.lc_id.clone());
            let auditor = RootRecipeFilesystemAuditor::new();
            for (index, (step, command_index, command)) in
                root_recipe_command_index().into_iter().enumerate()
            {
                let watch = auditor
                    .before_command(&operation_id, &canonical_root, step, command_index, command)
                    .expect("freeze command watch");
                let receipt = auditor
                    .after_command(watch, format!("2026-11-10T00:10:{index:02}Z"))
                    .expect("audit command");
                assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
                receipt_store
                    .append_command(PROJECT_ID, receipt)
                    .expect("append command receipt");
            }
            receipt_store
                .finalize(
                    PROJECT_ID,
                    &operation_id,
                    &policy.digest,
                    &rule_digest,
                    "2026-11-10T00:20:00Z".to_string(),
                )
                .expect("finalize root receipt");
        }

        /// 旧口径的「两文件」事实：canonical 聚合根下根规则入口 + 已发布
        /// 政策 artifact——故意不种根配方 operation 与最终 receipt，验证
        /// 两文件在新口径下不再单独构成「已完成」。
        fn seed_legacy_two_file_facts(&self) {
            fs::write(
                self.aggregate_root.join(ROOT_RULE_ENTRY_FILE),
                "# aggregate root rules\n",
            )
            .expect("write root rule entry");
            let policy = self.real_policy();
            AggregatePolicyArtifactStore::for_lc(self.paths.clone(), self.lc_id.clone())
                .save(PROJECT_ID, &policy)
                .expect("save policy artifact");
        }

        /// revision 1 的真实（非自举桩）政策 artifact：digest 按 store 的
        /// canonical 规则计算（policy_text 原字节 SHA-256、`sha256:` 前缀）。
        fn real_policy(&self) -> AggregatePolicyArtifact {
            let policy_text = "# Aggregate policy (root recipe)\n\nRoot-recipe aggregated policy for planning read-only and coding target-write sessions.\n".to_string();
            AggregatePolicyArtifact {
                policy_id: format!("policy/{PROJECT_ID}/{}/1", self.lc_id),
                project_id: PROJECT_ID.to_string(),
                logical_codebase_id: self.lc_id.clone(),
                revision: 1,
                digest: format!("sha256:{:x}", Sha256::digest(policy_text.as_bytes())),
                policy_text,
                created_at: "2026-11-10T00:00:00Z".to_string(),
            }
        }

        /// 预检②：在 Pi 的 shared 层放 skill（provider 固定用 Pi）。
        fn seed_skills(&self, skills: &[&str]) {
            let skills_paths = CadenceSkillsPaths::from_home(self.home_path());
            let layer = skills_paths.shared_skills_root();
            for skill in skills {
                fs::create_dir_all(layer.join(skill)).expect("create skill dir");
                fs::write(layer.join(skill).join("SKILL.md"), "# skill\n").expect("write SKILL.md");
            }
        }

        fn seed_all_seven_skills(&self) {
            self.seed_skills(&KNOWLEDGE_BASE_SKILLS);
        }

        /// 预检⑦：种一条 Pi 的机器级 capability 记录（`CodingTargetWrite`
        /// 行的 `launch` / `write_boundary` 两分格由入参决定）。
        /// `execution_semantics` 恒 `host-open-v1`——缺执行语义时记录读回
        /// 矩阵整体读作 Unknown（不得成为当前 Confirmed）。
        fn seed_write_capability(
            &self,
            launch: ProviderCapabilityEvidence,
            write_boundary: ProviderCapabilityEvidence,
        ) {
            let row = ProviderActionCapability {
                action: SessionPolicyAction::CodingTargetWrite,
                launch,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary,
                projection_digest: "projection-digest-CodingTargetWrite".to_string(),
                evidence_ref: "probe://boundary/pi/0001".to_string(),
            };
            let record = ProviderCapabilityRecord {
                provider_type: ProviderRefType::Pi,
                schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
                version: "1.0.0".to_string(),
                adapter_dialect: ProviderDialect::PiRpcV1,
                wire_dialect: ProviderWireDialect::PiRpc,
                capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
                evidence: CapabilityEvidence::ProductionVerified,
                resume_evidence: ResumeEvidenceState::Confirmed,
                supported_actions: Vec::new(),
                action_matrix: ProviderActionMatrix::from_rows(vec![row])
                    .expect("capability matrix rows are unique"),
                trust: ProviderCapabilityEvidence::Confirmed,
                probed_at: Some("2026-11-09T00:00:00Z".to_string()),
                probe_artifact_ref: Some("probe://artifact-0001".to_string()),
                execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
            };
            ProviderCapabilityStore::new(self.paths.clone())
                .upsert(&record)
                .expect("upsert capability record");
        }

        /// 预检⑦：写分格两半全 `Confirmed`（放行基线）。
        fn seed_confirmed_write_capability(&self) {
            self.seed_write_capability(
                ProviderCapabilityEvidence::Confirmed,
                ProviderCapabilityEvidence::Confirmed,
            );
        }

        /// 预检③：base-info.md + 六领域 scope 文件（ddl/product 可选，不放）。
        fn seed_bootstrap_inputs(&self, overrides: &[(&str, &str)]) {
            let user_input = self.kb_root().join("user-input");
            fs::create_dir_all(&user_input).expect("create user-input");
            fs::write(user_input.join("base-info.md"), base_info_text(overrides))
                .expect("write base-info.md");
            for (_, file) in SIX_DOMAINS {
                fs::write(user_input.join(file), "# scope\n").expect("write scope file");
            }
        }

        fn write_manifest_yaml(&self, yaml: &str) {
            let kb_root = self.kb_root();
            fs::create_dir_all(&kb_root).expect("create kb root");
            fs::write(kb_root.join("manifest.yaml"), yaml).expect("write manifest.yaml");
        }
    }

    /// 按六领域生成 base-info.md；`overrides` 覆盖领域状态（默认 `全量`）。
    fn base_info_text(overrides: &[(&str, &str)]) -> String {
        let mut text =
            String::from("# KnowledgeBase 用户输入入口\n\n## 基线信息\n\n- 项目名称：demo\n");
        for (domain, default_file) in SIX_DOMAINS {
            let status = overrides
                .iter()
                .find(|(name, _)| *name == domain)
                .map(|(_, status)| *status)
                .unwrap_or("全量");
            text.push_str(&format!(
                "\n## {domain}\n\n- 状态：{status}\n- 资料：[{domain}范围](./{default_file})\n- 不适用原因：\n"
            ));
        }
        text
    }

    fn provider() -> ProviderName {
        ProviderName::Pi
    }

    // ------------------------------------------------------------------
    // 预检①：聚合初始化已完成（口径 = 冷启动投影 rules_policy 步 Completed）
    // ------------------------------------------------------------------

    #[test]
    fn aggregate_initialization_completed_true_when_rules_policy_step_completed() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        assert!(
            aggregate_initialization_completed(&fixture.paths, PROJECT_ID, &fixture.lc_id),
            "rules_policy 步投影 Completed 必须判已完成"
        );
    }

    #[test]
    fn aggregate_initialization_completed_false_when_rules_policy_step_not_completed() {
        let fixture = PreflightFixture::new();
        // 只登记 LC，不种政策/operation/receipt：rules_policy 步未开始。
        assert!(!aggregate_initialization_completed(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id
        ));
    }

    #[test]
    fn aggregate_initialization_completed_false_when_projection_unavailable() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        // LC 不存在 / project 不存在 ⇒ 投影读不出 ⇒ fail-closed false。
        assert!(!aggregate_initialization_completed(
            &fixture.paths,
            PROJECT_ID,
            "logical_codebase_missing"
        ));
        assert!(!aggregate_initialization_completed(
            &fixture.paths,
            "project_missing",
            &fixture.lc_id
        ));
    }

    #[test]
    fn aggregate_initialization_completed_false_for_legacy_two_file_facts() {
        let fixture = PreflightFixture::new();
        // 旧口径的「根规则 + 政策 artifact」两文件在场，但没有根配方
        // operation 与最终 receipt：新口径下不再单独构成「已完成」。
        fixture.seed_legacy_two_file_facts();
        assert!(!aggregate_initialization_completed(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id
        ));
    }

    #[test]
    fn preflight_bootstrap_requires_aggregate_initialization() {
        let fixture = PreflightFixture::new();
        // 什么都不种：rules_policy 步未完成。
        let failure = preflight_bootstrap(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
        )
        .expect_err("aggregate initialization is missing");
        assert_eq!(
            failure,
            KnowledgeBasePreflightFailure::RequiresAggregateInitialization
        );
    }

    // ------------------------------------------------------------------
    // 预检②：七个 knowledge-base-* skill
    // ------------------------------------------------------------------

    #[test]
    fn preflight_bootstrap_reports_missing_knowledge_base_skill() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        // 只放三个，缺 knowledge-base-overview / -context / -update / -base-info。
        fixture.seed_skills(&[
            "knowledge-base-bootstrap",
            "knowledge-base-api",
            "knowledge-base-pages",
        ]);
        fixture.seed_bootstrap_inputs(&[]);
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        let failure = preflight_bootstrap(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
        )
        .expect_err("knowledge base skills are incomplete");
        match failure {
            KnowledgeBasePreflightFailure::SkillsUnavailable { missing } => {
                assert!(
                    missing.contains(&"knowledge-base-overview".to_string()),
                    "missing: {missing:?}"
                );
            }
            other => panic!("expected SkillsUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn preflight_bootstrap_accepts_all_seven_knowledge_base_skills() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        fixture.seed_bootstrap_inputs(&[]);
        fixture.seed_confirmed_write_capability();
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        preflight_bootstrap(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
        )
        .expect("all seven skills present: preflight must pass");
    }

    // ------------------------------------------------------------------
    // 预检③：六领域输入按状态条件必需
    // ------------------------------------------------------------------

    #[test]
    fn preflight_bootstrap_reports_missing_scope_file_for_applicable_domain() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        fixture.seed_bootstrap_inputs(&[]);
        // 数据模型声明「全量」但删掉对应 scope 文件。
        fs::remove_file(
            fixture
                .kb_root()
                .join("user-input")
                .join("data-model-scope.md"),
        )
        .expect("remove data model scope");
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        let failure = preflight_bootstrap(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
        )
        .expect_err("data model scope file is required");
        match failure {
            KnowledgeBasePreflightFailure::InputsIncomplete { missing } => {
                assert!(
                    missing.contains(&"data-model-scope.md".to_string()),
                    "missing: {missing:?}"
                );
            }
            other => panic!("expected InputsIncomplete, got {other:?}"),
        }
    }

    #[test]
    fn preflight_bootstrap_accepts_not_applicable_domain_without_scope_file() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        // 接口声明「不适用」，且从始至终不创建 api-scope.md。
        fixture.seed_bootstrap_inputs(&[("接口", "不适用")]);
        fs::remove_file(fixture.kb_root().join("user-input").join("api-scope.md"))
            .expect("remove api scope");
        fixture.seed_confirmed_write_capability();
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        preflight_bootstrap(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
        )
        .expect("not-applicable domain must not require its scope file");
    }

    #[test]
    fn preflight_bootstrap_accepts_missing_optional_ddl() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        fixture.seed_bootstrap_inputs(&[]);
        assert!(
            !fixture
                .kb_root()
                .join("user-input")
                .join("database-ddl.sql")
                .exists(),
            "precondition: no ddl in fixture"
        );
        fixture.seed_confirmed_write_capability();
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        preflight_bootstrap(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
        )
        .expect("database-ddl.sql is optional");
    }

    // ------------------------------------------------------------------
    // 预检⑦：写分格能力门（`launch` 与 `write_boundary` 双 Confirmed）
    // ------------------------------------------------------------------

    #[test]
    fn preflight_rejects_write_cell_when_write_boundary_not_confirmed() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        fixture.seed_bootstrap_inputs(&[]);
        // launch 半边已确认，write_boundary 半边未确认 ⇒ 只查 launch 半边会
        // 误放行；双 Confirmed 门必须判能力拒绝（`CapabilityDenied` 变体
        // 不再是死码）。
        fixture.seed_write_capability(
            ProviderCapabilityEvidence::Confirmed,
            ProviderCapabilityEvidence::Unknown,
        );
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        let failure = preflight_bootstrap(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
        )
        .expect_err("write_boundary cell is not confirmed");
        assert_eq!(
            failure,
            KnowledgeBasePreflightFailure::CapabilityDenied {
                provider: "pi".to_string()
            }
        );
    }

    #[test]
    fn preflight_rejects_write_cell_when_launch_cell_not_confirmed() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        fixture.seed_bootstrap_inputs(&[]);
        fixture.seed_write_capability(
            ProviderCapabilityEvidence::Unknown,
            ProviderCapabilityEvidence::Confirmed,
        );
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        let failure = preflight_bootstrap(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
        )
        .expect_err("launch cell is not confirmed");
        assert!(matches!(
            failure,
            KnowledgeBasePreflightFailure::CapabilityDenied { .. }
        ));
    }

    #[test]
    fn preflight_rejects_write_cell_without_capability_record() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        fixture.seed_bootstrap_inputs(&[]);
        // 缺 capability 记录 ⇒ fail-closed（证明不了已核验即拒绝）。
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        let failure = preflight_bootstrap(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
        )
        .expect_err("capability record is missing");
        assert!(matches!(
            failure,
            KnowledgeBasePreflightFailure::CapabilityDenied { .. }
        ));
    }

    // ------------------------------------------------------------------
    // 预检④⑤：update 的 manifest 门禁
    // ------------------------------------------------------------------

    #[test]
    fn preflight_update_rejects_incomplete_initialization() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        fixture.write_manifest_yaml(
            "schema_version: \"4.0\"\ncoverage:\n  initialization:\n    status: in_progress\n    global_validation: pending\n    completed_stages: [base-info]\n",
        );
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        let failure = preflight_update(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
            &fixture.kb_root().join("user-input/updates/CHANGE-demo"),
            &[],
        )
        .expect_err("manifest is still in_progress");
        assert_eq!(failure, KnowledgeBasePreflightFailure::NotInitialized);
    }

    #[test]
    fn preflight_update_rejects_manifest_version_mismatch() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        fixture.write_manifest_yaml(
            "schema_version: \"3.0\"\ncoverage:\n  initialization:\n    status: complete\n    global_validation: passed\n",
        );
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        let failure = preflight_update(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
            &fixture.kb_root().join("user-input/updates/CHANGE-demo"),
            &[],
        )
        .expect_err("schema version 3.0 is unsupported");
        assert_eq!(
            failure,
            KnowledgeBasePreflightFailure::ManifestVersionUnsupported {
                found: "3.0".to_string()
            }
        );
    }

    #[test]
    fn preflight_update_rejects_complete_passed_manifest_with_broken_invariants() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        // status/global_validation 字面合法，但必需阶段一个都没完成、completed_at 为空：
        // 不变量非法 ⇒ 必须在进入变更包 lint 之前被拒。
        fixture.write_manifest_yaml(
            "schema_version: \"4.0\"\ncoverage:\n  initialization:\n    status: complete\n    global_validation: passed\n    completed_stages: []\n    skipped_stages: []\n    completed_at: \"\"\n",
        );
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        let failure = preflight_update(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
            &fixture.kb_root().join("user-input/updates/CHANGE-demo"),
            &[],
        )
        .expect_err("complete + passed with broken invariants must be rejected");
        assert_eq!(failure, KnowledgeBasePreflightFailure::NotInitialized);
    }

    #[test]
    fn preflight_update_legal_complete_manifest_passes_invariant_gate() {
        let fixture = PreflightFixture::new();
        fixture.seed_aggregate_initialization();
        fixture.seed_all_seven_skills();
        // 合法 complete manifest（与 manifest.rs 的 COMPLETE_FIXTURE 同形）：
        // 不变量门禁必须放行，失败只能来自后续的变更包 lint（目录不存在）。
        fixture.write_manifest_yaml(
            "schema_version: \"4.0\"\ncoverage:\n  initialization:\n    status: complete\n    global_validation: passed\n    completed_stages: [base-info, overview, global-validation]\n    skipped_stages:\n      - stage: api\n        reason: 接口领域不适用\n      - stage: pages\n        reason: 页面领域不适用\n    completed_at: \"2026-11-09T00:05:00Z\"\nscope:\n  configurations:\n    status: 全量\n  api:\n    status: 不适用\n  pages:\n    status: 不适用\n",
        );
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        let failure = preflight_update(
            &fixture.paths,
            PROJECT_ID,
            &fixture.lc_id,
            &fixture.aggregate_root,
            &provider(),
            &fixture.kb_root().join("user-input/updates/CHANGE-demo"),
            &[],
        );
        assert_ne!(failure, Err(KnowledgeBasePreflightFailure::NotInitialized));
    }
}
