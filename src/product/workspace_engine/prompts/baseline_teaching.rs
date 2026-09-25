use super::*;

/// 基线限制教学块（REQ-PIB-02 provider 原生通道软限制，design D3）。
///
/// provider 无关（kimi 同注入，冗余无害）；**软约束非安全边界**——host-served
/// 通道的硬边界在 kimi client_services（fs 树路由/terminal 拒绝），provider
/// 原生通道物理可达集不受限（用户 2026-09-25 裁决接受残余风险）；plan 期
/// AC 路径形态由 REQ-PIB-03 核对 fail-closed 兜底（acceptance_path_not_in_
/// baseline）。
pub(crate) fn baseline_teaching_block(branch: &str) -> String {
    format!(
        "## 基准分支基线（issue 基线 = {branch}）\n\
         本 issue 的基准分支已锁定为 `{branch}`：你对仓库「既有内容」的一切判断只能以该分支的树内容为来源（`git show refs/heads/{branch}:<path>`、`git ls-tree -r --name-only refs/heads/{branch}`）。\n\
         - 不得把 `.worktrees/`（含 `.worktrees/aria-issues/*` 兄弟工作区）、未提交的工作区改动、或其他分支才存在的文件当作「既有事实」引用；\n\
         - 不得访问仓库工作区之外的路径；\n\
         - plan 的验收标准/验证计划引用的路径若不在基线树内，将在核对期被 MustFix 拦回（acceptance_path_not_in_baseline）。\n"
    )
}

/// reviewer 版基线限制教学块（REQ-PIB-02 覆盖补齐，F-58 现场：issue_0002 story
/// 会话 reviewer_run 的 pi reviewer 以 bash 扫到 `.worktrees/aria-issues/*`——
/// author 两族 builder 已注入，而 review 族与 author 修订面缺口）。
///
/// 与 author 版同一来源与同一禁令（基线树=「既有事实」唯一来源、`.worktrees/`
/// 与工作区外不可引用），按 reviewer 的产出面（finding 证据）措辞；同一 header
/// 标记保持两面可被同一断言识别。仍为软约束非安全边界（语义同
/// `baseline_teaching_block`，host-served 通道才具硬边界）。
pub(crate) fn reviewer_baseline_teaching_block(branch: &str) -> String {
    format!(
        "## 基准分支基线（issue 基线 = {branch}）\n\
         本 issue 的基准分支已锁定为 `{branch}`：你对仓库「既有内容」的一切判断与 finding 证据只能以该分支的树内容为来源（`git show refs/heads/{branch}:<path>`、`git ls-tree -r --name-only refs/heads/{branch}`）。\n\
         - 不得把 `.worktrees/`（含 `.worktrees/aria-issues/*` 兄弟工作区）、未提交的工作区改动、或其他分支才存在的文件当作「既有事实」或 finding 证据；\n\
         - 不得访问仓库工作区之外的路径；\n\
         - 候选产物引用的路径若不在基线树内，属 `acceptance_path_not_in_baseline` 缺口（MustFix 判定由核对期承载）。\n"
    )
}

impl WorkspaceEngine {
    /// issue 基线树解析（REQ-PIB-02，四面同链同源：author 两族 / reviewer 族 /
    /// author 修订面）。跳过面（`Ok(None)`，行为不变）：无持久 store 的内存态
    /// engine、无仓库路径会话、聚合 Logical Story/Design（Non-Goal 多仓差异基线）。
    /// 否则经 `IssueStore` 读 issue.base_branch → `resolve_effective_base_branch`
    ///（三面同源唯一解析链）：不可解析（分支被删/存量皆无/仓库不可用）→
    /// `Err(diagnosis)` fail-closed 终止该轮 provider 运行，不回退不猜替代分支。
    fn resolve_issue_baseline_tree(
        &self,
    ) -> Result<Option<crate::cross_cutting::streaming_provider::BaselineTreeRef>, String> {
        if self.is_aggregate_story_or_design() {
            return Ok(None);
        }
        let Some(repository_path) = self.session.repository_path.as_ref() else {
            return Ok(None);
        };
        let Some(store) = self.lifecycle_store.as_ref() else {
            return Ok(None);
        };
        let issue = crate::product::issue_store::IssueStore::new(store.app_paths())
            .get(&self.session.project_id, &self.session.issue_id)
            .map_err(|error| format!("load issue for author baseline failed: {error}"))?;
        let branch = crate::product::issue_baseline::resolve_effective_base_branch(
            repository_path,
            issue.base_branch.as_deref(),
        )
        .map_err(|error| error.diagnosis())?;
        Ok(Some(
            crate::cross_cutting::streaming_provider::BaselineTreeRef {
                repo_path: repository_path.clone(),
                branch,
            },
        ))
    }

    /// author 面（author 两族 + author 修订面，三处同构）：基线解析 fail-closed
    /// + 软限制教学注入，返回基线锚点供 `StreamingProviderInput` 携带。
    pub(super) fn append_author_baseline_teaching(
        &self,
        prompt: &mut String,
    ) -> Result<Option<crate::cross_cutting::streaming_provider::BaselineTreeRef>, String> {
        let baseline_tree = self.resolve_issue_baseline_tree()?;
        if let Some(baseline) = baseline_tree.as_ref() {
            prompt.push_str(&baseline_teaching_block(&baseline.branch));
        }
        Ok(baseline_tree)
    }

    /// reviewer 族（review.rs 各 review builder）：同一条解析链 + reviewer 版
    /// 教学块（F-58 覆盖补齐）。同链同源意味着基线不可解析时 reviewer 亦
    /// fail-closed——与 author 两族同语义，不留「基线已消失但审核照跑」的缺口。
    pub(super) fn append_reviewer_baseline_teaching(
        &self,
        prompt: &mut String,
    ) -> Result<Option<crate::cross_cutting::streaming_provider::BaselineTreeRef>, String> {
        let baseline_tree = self.resolve_issue_baseline_tree()?;
        if let Some(baseline) = baseline_tree.as_ref() {
            prompt.push_str(&reviewer_baseline_teaching_block(&baseline.branch));
        }
        Ok(baseline_tree)
    }
}

/// REQ-PIB-02（T2.2 软限制注入 + T2.3 fail-closed）基线面测试共用夹具：
/// author 两族 / reviewer 族 / 修订面三面同链同源（F-58 覆盖补齐后）。
#[cfg(test)]
mod baseline_teaching_fixture {
    use crate::product::models::{ProviderName, WorkspaceType};
    use crate::product::workspace_engine::types::WorkspaceEngine;

    pub(super) fn git_at(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .status()
            .expect("git fixture command");
        assert!(status.success(), "git {args:?} in {}", dir.display());
    }

    pub(super) struct BaselineEngineFixture {
        pub(super) _aria_root: tempfile::TempDir,
        pub(super) _repo: tempfile::TempDir,
        pub(super) engine: WorkspaceEngine,
    }

    /// 真实 git 仓（`initial_branch` 初始提交）+ issue（`base_branch`）+ story
    /// workspace session 的持久 engine（repository_path=主检出）。
    pub(super) fn baseline_engine(
        initial_branch: &str,
        base_branch: Option<&str>,
        workspace_type: WorkspaceType,
    ) -> BaselineEngineFixture {
        let aria_root = tempfile::tempdir().expect("aria root");
        let repo = tempfile::tempdir().expect("repo dir");
        git_at(repo.path(), &["init", "-b", initial_branch]);
        git_at(repo.path(), &["config", "user.email", "test@example.com"]);
        git_at(repo.path(), &["config", "user.name", "Test User"]);
        std::fs::write(repo.path().join("package.json"), "{}\n").expect("package.json");
        git_at(repo.path(), &["add", "package.json"]);
        git_at(repo.path(), &["commit", "-m", "baseline"]);
        let app_paths =
            crate::product::app_paths::ProductAppPaths::new(aria_root.path().join(".aria"));
        crate::product::project_store::ProjectStore::new(app_paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "baseline engine fixture".to_string(),
                description: None,
            })
            .expect("create project");
        let repository = crate::product::repository_store::RepositoryStore::new(app_paths.clone())
            .create(crate::product::repository_store::CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "Repo".to_string(),
                path: repo.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: format!("baseline-engine-{initial_branch}"),
            })
            .expect("create repository");
        crate::product::issue_store::IssueStore::new(app_paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                logical_codebase_id: None,
                base_branch: base_branch.map(str::to_string),
                title: "Baseline".to_string(),
                description: None,
                change_id: None,
            })
            .expect("create issue");
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(app_paths);
        let session_record = lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    entity_id: "story_spec_0001".to_string(),
                    workspace_type: workspace_type.clone(),
                    author_provider: ProviderName::ClaudeCode,
                    reviewer_provider: ProviderName::Codex,
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: None,
                },
            )
            .expect("create workspace session");
        let mut session =
            crate::product::workspace_engine::types::WorkspaceSession::from_record(session_record);
        session.repository_path = Some(repo.path().to_path_buf());
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        let engine = WorkspaceEngine::new_persistent(
            std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                aria_root.path().join("checkpoints"),
            )),
            lifecycle,
            event_tx,
            session,
        );
        BaselineEngineFixture {
            _aria_root: aria_root,
            _repo: repo,
            engine,
        }
    }
}

#[cfg(test)]
mod author_baseline_tests {
    use super::baseline_teaching_fixture::baseline_engine;
    use crate::product::models::{ProviderName, WorkspaceType};
    use crate::product::workspace_engine::types::AuthorPromptMode;
    use crate::product::workspace_engine::types::PlanAuthorOutputContract;

    /// REQ-PIB-02 场景 4（软限制注入）：native（ClaudeCode）会话正常构造（不
    /// 拒启），prompt 含基线教学块，input 携带基线锚点——story/design 与 plan
    /// 两族同注入。
    #[test]
    fn baseline_session_builds_with_teaching_block_for_native_provider() {
        let fixture = baseline_engine("main", None, WorkspaceType::Story);
        let input = fixture
            .engine
            .build_streaming_input("生成", AuthorPromptMode::FullConversation)
            .expect("native baseline session must start (soft restriction)");
        let baseline = input.baseline_tree.as_ref().expect("baseline anchor");
        assert_eq!(baseline.branch, "main");
        assert_eq!(
            baseline.repo_path,
            fixture.engine.session.repository_path.clone().unwrap()
        );
        assert!(
            input.prompt.contains("基准分支基线（issue 基线 = main）"),
            "{}",
            input.prompt
        );
        assert!(
            input.prompt.contains(".worktrees/"),
            "{prompt}",
            prompt = input.prompt
        );
        assert!(
            input.prompt.contains("refs/heads/main"),
            "{prompt}",
            prompt = input.prompt
        );

        let plan_input = fixture
            .engine
            .build_work_item_plan_streaming_input(
                crate::protocol::contracts::ProviderType::ClaudeCode,
                "plan prompt".to_string(),
                "/tmp/worktree".to_string(),
                ProviderName::ClaudeCode,
                PlanAuthorOutputContract::Structured,
            )
            .expect("native plan baseline session must start");
        assert!(
            plan_input
                .prompt
                .contains("基准分支基线（issue 基线 = main）")
        );
        assert!(plan_input.baseline_tree.is_some());
    }

    /// T2.3（REQ-PIB-02 场景 3）：基线分支被删 → 两族 builder Err fail-closed
    ///（终止生成，不回退不猜），诊断含「基准分支不存在」。
    #[test]
    fn baseline_branch_missing_fails_closed_for_both_builder_families() {
        let fixture = baseline_engine("main", Some("gone"), WorkspaceType::Story);
        let error = fixture
            .engine
            .build_streaming_input("生成", AuthorPromptMode::FullConversation)
            .expect_err("missing baseline branch must fail closed");
        assert!(error.contains("基准分支不存在"), "{error}");
        assert!(error.contains("gone"), "{error}");
        let plan_error = fixture
            .engine
            .build_work_item_plan_streaming_input(
                crate::protocol::contracts::ProviderType::ClaudeCode,
                "plan prompt".to_string(),
                "/tmp/worktree".to_string(),
                ProviderName::ClaudeCode,
                PlanAuthorOutputContract::Structured,
            )
            .expect_err("plan family must fail closed too");
        assert!(plan_error.contains("基准分支不存在"), "{plan_error}");
    }

    /// 存量 None=默认链：master-only 仓 → master；皆无仓（trunk）→ Err「无法
    /// 推断默认基准分支」（存量场景 fail-closed）。
    #[test]
    fn legacy_none_follows_default_chain_and_no_default_fails_closed() {
        let master_fixture = baseline_engine("master", None, WorkspaceType::Design);
        let input = master_fixture
            .engine
            .build_streaming_input("生成", AuthorPromptMode::FullConversation)
            .expect("master-only legacy issue resolves master");
        assert_eq!(
            input.baseline_tree.as_ref().expect("baseline").branch,
            "master"
        );
        assert!(input.prompt.contains("issue 基线 = master"));

        let trunk_fixture = baseline_engine("trunk", None, WorkspaceType::Design);
        let error = trunk_fixture
            .engine
            .build_streaming_input("生成", AuthorPromptMode::FullConversation)
            .expect_err("no default branch must fail closed");
        assert!(error.contains("无法推断默认基准分支"), "{error}");
    }
}

/// REQ-PIB-02 覆盖补齐（F-58）：reviewer 族与 author 修订面同链同源。
///
/// 现场：issue_0002 story 会话 reviewer_run 的 pi reviewer 以 bash 扫到
/// `.worktrees/aria-issues/*`——author 两族 builder 已注入基线教学块，而
/// review 族 builder 与 `build_revision_input` 未注入（间隙）。
#[cfg(test)]
mod reviewer_revision_baseline_tests {
    use super::baseline_teaching_fixture::baseline_engine;
    use crate::product::models::WorkspaceType;

    /// story/design reviewer（共享 `build_review_input`）与 author 修订面
    /// （`build_revision_input`）都注入基线教学块并携带同一基线锚点。
    #[test]
    fn reviewer_and_revision_faces_inject_baseline_teaching() {
        for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
            let fixture = baseline_engine("main", None, workspace_type.clone());
            let input = fixture
                .engine
                .build_review_input()
                .expect("reviewer baseline session must build");
            let baseline = input
                .baseline_tree
                .as_ref()
                .expect("reviewer baseline anchor");
            assert_eq!(baseline.branch, "main");
            assert!(
                input.prompt.contains("基准分支基线（issue 基线 = main）"),
                "{workspace_type:?}: {}",
                input.prompt
            );
            assert!(
                input.prompt.contains(".worktrees/"),
                "{workspace_type:?}: {prompt}",
                prompt = input.prompt
            );
            assert!(
                input.prompt.contains("refs/heads/main"),
                "{workspace_type:?}: {prompt}",
                prompt = input.prompt
            );
        }

        let mut revision = baseline_engine("master", None, WorkspaceType::Story);
        revision.engine.pending_revision_context = Some("补充异常场景".to_string());
        let input = revision
            .engine
            .build_revision_input()
            .expect("revision baseline session must build");
        assert_eq!(
            input.baseline_tree.as_ref().expect("baseline").branch,
            "master"
        );
        assert!(
            input.prompt.contains("基准分支基线（issue 基线 = master）"),
            "{}",
            input.prompt
        );
        assert!(
            input.prompt.contains(".worktrees/"),
            "{prompt}",
            prompt = input.prompt
        );
    }

    /// 同链 fail-closed：reviewer 与修订面在基线分支不可解析时同样终止
    ///（不回退不猜替代分支）——与 author 两族同语义。
    #[test]
    fn reviewer_and_revision_faces_fail_closed_when_baseline_branch_missing() {
        let fixture = baseline_engine("main", Some("gone"), WorkspaceType::Story);
        let error = fixture
            .engine
            .build_review_input()
            .expect_err("missing baseline branch must fail reviewer closed");
        assert!(error.contains("基准分支不存在"), "{error}");
        assert!(error.contains("gone"), "{error}");

        let mut revision = baseline_engine("main", Some("gone"), WorkspaceType::Story);
        revision.engine.pending_revision_context = Some("补充反馈".to_string());
        let error = revision
            .engine
            .build_revision_input()
            .expect_err("missing baseline branch must fail revision closed");
        assert!(error.contains("基准分支不存在"), "{error}");
        assert!(error.contains("gone"), "{error}");
    }
}
