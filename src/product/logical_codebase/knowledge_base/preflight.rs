//! 知识库创建期前置检查（Task 4，只读、零副作用、不产生 operation）。
//!
//! 对应技术方案 §5.3 的预检 ①-⑥；预检 ⑦（provider 选择，按
//! `CodingTargetWrite` 分格）与 ⑧（同 LC 单飞检查）需要 health 快照与
//! project 作用域，由 HTTP 层经 [`crate::product::logical_codebase::provider_selection::select_aggregate_provider_for_write`]
//! 与 `KnowledgeBaseOperationStore::list_running` 完成后，映射进本模块的
//! `KnowledgeBasePreflightFailure` 统一失败面；`preflight_*` 本身只产出
//! 同步可判定的 ①-⑥。

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
    use crate::product::logical_codebase::store::{LogicalCodebaseManifest, LogicalCodebaseStore};
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
            let layer = CadenceSkillsPaths::from_home(self.home_path()).shared_skills_root();
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
                .find(|(name, _)| name == domain)
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
