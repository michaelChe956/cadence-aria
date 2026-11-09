//! 知识库创建期前置检查（Task 4，只读、零副作用、不产生 operation）。
//!
//! 对应技术方案 §5.3 的预检 ①-⑥；预检 ⑦（provider 选择，按
//! `CodingTargetWrite` 分格）与 ⑧（同 LC 单飞检查）需要 health 快照与
//! project 作用域，由 HTTP 层经 [`crate::product::logical_codebase::provider_selection::select_aggregate_provider_for_write`]
//! 与 `KnowledgeBaseOperationStore::list_running` 完成后，映射进本模块的
//! `KnowledgeBasePreflightFailure` 统一失败面；`preflight_*` 本身只产出
//! 同步可判定的 ①-⑥。

use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::cadence_skills::CadenceSkillsPaths;
use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::provider_selection::ProviderSelectionOutcome;
use crate::product::logical_codebase::provider_trust_adapters::resolve_owner_home_from_env;
use crate::product::logical_codebase::root_recipe_receipt::root_rule_digest;
use crate::product::logical_codebase::store::LogicalCodebaseStore;
use crate::product::models::ProviderName;

use super::change_package::{ChangePackageViolation, MemberRepoPaths, lint_change_package};
use super::manifest::read_manifest;

/// 知识库创建期前置检查的统一失败面（稳定码登记在 Task 7，本模块只产出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeBasePreflightFailure {
    /// 预检①：聚合初始化未完成（根规则入口或已发布政策 artifact 缺失）。
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
    /// 预检⑦：候选的 `CodingTargetWrite` 分格被真实拒绝（HTTP 层从
    /// `ProviderSelectionOutcome::Denied` 构造）。
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

/// bootstrap 前置检查（预检①②③，只读、不产生 operation）。
pub fn preflight_bootstrap(
    paths: &ProductAppPaths,
    lc_id: &str,
    aggregate_root: &Path,
    provider: &ProviderName,
) -> Result<(), KnowledgeBasePreflightFailure> {
    preflight_common(paths, lc_id, aggregate_root, provider)?;
    let missing = missing_bootstrap_inputs(aggregate_root);
    if !missing.is_empty() {
        return Err(KnowledgeBasePreflightFailure::InputsIncomplete { missing });
    }
    Ok(())
}

/// update 前置检查（预检①②④⑤⑥，只读、不产生 operation）。
pub fn preflight_update(
    paths: &ProductAppPaths,
    lc_id: &str,
    aggregate_root: &Path,
    provider: &ProviderName,
    package_dir: &Path,
    members: &[MemberRepoPaths],
) -> Result<(), KnowledgeBasePreflightFailure> {
    preflight_common(paths, lc_id, aggregate_root, provider)?;
    let snapshot = read_manifest(&knowledge_base_root(aggregate_root))
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
    let lint = lint_change_package(package_dir, &snapshot, members);
    if !lint.violations.is_empty() {
        return Err(KnowledgeBasePreflightFailure::ChangePackageInvalid {
            violations: lint.violations,
        });
    }
    Ok(())
}

/// 预检①②（bootstrap 与 update 共用，顺序固定：先聚合初始化，后 skills）。
fn preflight_common(
    paths: &ProductAppPaths,
    lc_id: &str,
    aggregate_root: &Path,
    provider: &ProviderName,
) -> Result<(), KnowledgeBasePreflightFailure> {
    if !aggregate_initialization_completed(paths, lc_id, aggregate_root) {
        return Err(KnowledgeBasePreflightFailure::RequiresAggregateInitialization);
    }
    let missing = missing_knowledge_base_skills(provider);
    if !missing.is_empty() {
        return Err(KnowledgeBasePreflightFailure::SkillsUnavailable { missing });
    }
    Ok(())
}

/// 「聚合初始化已完成」= canonical 聚合根下根规则入口文件存在 **且** 该 LC
/// scope 下已发布政策 artifact 存在。两个事实分别复用既有只读查询
/// （`root_recipe_receipt::root_rule_digest` 与
/// `AggregatePolicyArtifactStore::get`），不新造判定；任一事实证明不了
/// （缺失、IO 故障、LC 归属找不到）都判未完成（fail-closed）。
fn aggregate_initialization_completed(
    paths: &ProductAppPaths,
    lc_id: &str,
    aggregate_root: &Path,
) -> bool {
    match root_rule_digest(aggregate_root) {
        Ok(Some(_)) => {}
        // Ok(None) = 入口文件缺失；Err = IO 故障——都证明不了「已完成」。
        _ => return false,
    }
    let Some(project_id) = owning_project(paths, lc_id) else {
        return false;
    };
    matches!(
        AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id.to_string()).get(&project_id),
        Ok(Some(_))
    )
}

/// 由 lc_id 反查所属 project：本模块签名不带 project_id，只能只读扫描
/// `projects_root` 并逐项目用既有 `load_lc_manifest` 判归属（读不到或坏
/// manifest 都视为不属于该项目）；LC manifest 找不到 ⇒ None（fail-closed）。
fn owning_project(paths: &ProductAppPaths, lc_id: &str) -> Option<String> {
    let store = LogicalCodebaseStore::new(paths.clone());
    let entries = std::fs::read_dir(paths.projects_root()).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let Ok(project_id) = entry.file_name().into_string() else {
            continue;
        };
        if validate_relative_id(&project_id).is_err() {
            continue;
        }
        if matches!(store.load_lc_manifest(&project_id, lc_id), Ok(Some(_))) {
            return Some(project_id);
        }
    }
    None
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
    let user_input = knowledge_base_root(aggregate_root).join("user-input");
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
            if let Some(rest) = trimmed.strip_prefix("状态") {
                if let Some(value) = rest.strip_prefix('：').or_else(|| rest.strip_prefix(':')) {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}

/// 知识库根目录：聚合根下 `cadence/knowledge-base`（与 Task 5 `files.rs`
/// 的 `kb_root` 同口径；files.rs 并行落地中，先内联同一路径，落地后收敛）。
fn knowledge_base_root(aggregate_root: &Path) -> PathBuf {
    aggregate_root.join("cadence").join("knowledge-base")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::cadence_skills::CadenceSkillsPaths;
    use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::store::LogicalCodebaseManifest;
    use crate::product::models::ProviderName;

    const PROJECT_ID: &str = "project_0001";
    const LC_ID: &str = "logical_codebase_0001";

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
    }

    impl PreflightFixture {
        fn new() -> Self {
            let workspace = TempDir::new().expect("workspace root");
            let home = TempDir::new().expect("isolated home");
            let paths = ProductAppPaths::new(workspace.path().join(".aria"));
            let aggregate_root = workspace.path().join("aggregate");
            fs::create_dir_all(&aggregate_root).expect("create aggregate root");
            Self {
                _workspace: workspace,
                _home: home,
                paths,
                aggregate_root,
            }
        }

        fn home_path(&self) -> &std::path::Path {
            self._home.path()
        }

        fn kb_root(&self) -> PathBuf {
            self.aggregate_root.join("cadence").join("knowledge-base")
        }

        /// 预检①：canonical 聚合根下根规则入口 + per-LC manifest +
        /// 已发布政策 artifact（经既有 store 写入，保证形态合法）。
        fn seed_aggregate_initialization(&self) {
            fs::write(
                self.aggregate_root.join(
                    crate::product::logical_codebase::root_recipe_receipt::ROOT_RULE_ENTRY_FILE,
                ),
                "# aggregate root rules\n",
            )
            .expect("write root rule entry");
            let manifest =
                LogicalCodebaseManifest::new(PROJECT_ID, self.aggregate_root.clone(), vec![]);
            let lc_dir = self.paths.logical_codebases_root(PROJECT_ID).join(LC_ID);
            fs::create_dir_all(&lc_dir).expect("create lc scope root");
            fs::write(
                lc_dir.join("manifest.json"),
                serde_json::to_string(&manifest).expect("serialize lc manifest"),
            )
            .expect("write lc manifest");
            AggregatePolicyArtifactStore::for_lc(self.paths.clone(), LC_ID.to_string())
                .ensure_bootstrap(&manifest)
                .expect("install lc policy artifact");
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
    // 预检①：聚合初始化已完成
    // ------------------------------------------------------------------

    #[test]
    fn preflight_bootstrap_requires_aggregate_initialization() {
        let fixture = PreflightFixture::new();
        // 什么都不种：没有根规则、没有政策 artifact。
        let failure =
            preflight_bootstrap(&fixture.paths, LC_ID, &fixture.aggregate_root, &provider())
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

        let failure =
            preflight_bootstrap(&fixture.paths, LC_ID, &fixture.aggregate_root, &provider())
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
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        preflight_bootstrap(&fixture.paths, LC_ID, &fixture.aggregate_root, &provider())
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

        let failure =
            preflight_bootstrap(&fixture.paths, LC_ID, &fixture.aggregate_root, &provider())
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
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        preflight_bootstrap(&fixture.paths, LC_ID, &fixture.aggregate_root, &provider())
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
        let _guard = HomeEnvGuard::isolate(fixture.home_path());

        preflight_bootstrap(&fixture.paths, LC_ID, &fixture.aggregate_root, &provider())
            .expect("database-ddl.sql is optional");
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
            LC_ID,
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
            LC_ID,
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
}
