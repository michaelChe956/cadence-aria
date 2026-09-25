//! C1 Task 1/2：SC 候选 preflight 族的机械 ReviewVerdict 适配（F-51/F-56）。
//!
//! preflight 族 = options×items 三族缺口 + AC 引用路径×基线树缺口。此类缺口
//! 是「用户创建意图 × 候选实际交付」的确定性事实，与 canonical 契约缺口
//! （`contract_prerevision`）同族：SHALL 在 generate/evaluate 期经既有机械
//! ReviewVerdict 回灌修订循环（`complete_review` ingestion → F5-A 修订轮回灌
//! → policy TriggerAggregateRepair 重驱 author），SHALL NOT 把 author 轮次硬
//! 失败，更不得延迟至 Approval/Final Compile 才首次出现。
//!
//! finding 消息由 `WorkItemSplitValidator::validate` 三族语义产出（禁复制
//! 规则）；`required_action`/`contract_field` 从共享常量与 code 确定性映射，
//! 同输入同文本——保证跨轮指纹稳定（REQ-TOP-04 结构化 identity 消费同一
//! `classify_finding` 口径）。skipped-risk Warning 仅在已有 Error verdict 时
//! 搭车为 Suggestion+Advisory（可见不参与闸门/预算），与 contract_prerevision
//! 行为一致。

use crate::product::models::WorkItemSplitFinding;
use crate::product::work_item_plan_compiler::PlanCandidateMechanicalReport;
use crate::product::work_item_split_validator::{
    E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION, FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION,
    INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION,
};
use crate::web::workspace_ws_types::{
    ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
};
/// 机械 preflight verdict 的确定性可读全文（`complete_review` 的
/// `record_review_message` 载体，进 timeline 消息流供人审阅）。
pub(crate) fn preflight_readable_output(verdict: &ReviewVerdict) -> String {
    let mut output = String::new();
    output.push_str("[plan_preflight] 机械 options/基线路径预检未通过。\n");
    for finding in &verdict.findings {
        output.push_str(&format!(
            "- [{}] {}\n",
            match finding.severity {
                ReviewFindingSeverity::Blocking => "blocking",
                ReviewFindingSeverity::MustFix => "must_fix",
                ReviewFindingSeverity::Suggestion => "suggestion",
            },
            finding.message
        ));
    }
    output
}

/// 从候选机械报告中收集 preflight 族 findings 并适配为机械返修 verdict；
/// 无 Error 级 preflight 缺口时返回 None（干净/仅告警候选零变化）。
pub(crate) fn preflight_review_verdict(
    report: &PlanCandidateMechanicalReport,
) -> Option<ReviewVerdict> {
    let preflight: Vec<&WorkItemSplitFinding> = report
        .findings
        .iter()
        .filter(|finding| {
            crate::product::work_item_plan_compiler::PREFLIGHT_FINDING_CODES
                .contains(&finding.code.as_str())
        })
        .collect();
    let has_error = preflight.iter().any(|finding| {
        finding.severity == crate::product::models::WorkItemSplitFindingSeverity::Error
    });
    if !has_error {
        return None;
    }
    let findings = preflight
        .iter()
        .map(|finding| preflight_finding_to_review_finding(finding))
        .collect::<Vec<_>>();
    let error_count = findings
        .iter()
        .filter(|finding| finding.severity == ReviewFindingSeverity::MustFix)
        .count();
    let summary = format!(
        "计划 preflight 机械校验发现 {error_count} 项 Error 级缺口（options×items / AC 路径×基线树），候选必须返修"
    );
    let mut comments = String::new();
    comments.push_str(&format!(
        "计划 preflight 机械校验（存储 plan options 与基线树 × 候选交付交叉核对）发现 {error_count} 项 Error 级缺口：\n"
    ));
    for finding in &findings {
        if finding.severity == ReviewFindingSeverity::MustFix {
            comments.push_str(&format!("- {}\n", finding.message));
        }
    }
    comments
        .push_str("以上缺口均为机械比对结论（非 reviewer 判断）；逐条按 required_action 修复。");
    Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments,
        summary,
        findings,
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    })
}

/// 合并两路机械返修 verdict（canonical 契约缺口 × preflight 族）共用一次
/// `complete_review` ingestion：findings 顺序拼接（契约缺口在前，preflight 在
/// 后），verdict/gate 保持 Revise/RequiresRevision，comments 确定性拼接。
pub(crate) fn merge_revision_verdicts(
    contract: ReviewVerdict,
    preflight: ReviewVerdict,
) -> ReviewVerdict {
    let mut findings = contract.findings;
    findings.extend(preflight.findings);
    let comments = format!("{}\n{}", contract.comments, preflight.comments);
    let summary = format!("{}；{}", contract.summary, preflight.summary);
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments,
        summary,
        findings,
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

/// 加载 plan 基线树（REQ-PIB-03，软限制 pivot 后的**唯一硬兜底**）：按
/// issue 基线分支名直接取树——`git -C <repo> ls-tree -r --name-only
/// refs/heads/<base>`，不依赖共享 coding worktree（author 期首用场景），
/// 不 checkout、不触工作区。三生产路径（author 权威/门内修订/运行期预校验）
/// 共用本实现，与 author 所见（prompts 基线解析）、coding fork 同一解析链
///（`resolve_effective_base_branch` 三面同源）。
///
/// - `Ok(None)`：issue 无仓（repo_id=None，逻辑代码库 Non-Goal 面）——AC
///   路径核对不触发（与现状一致）；
/// - `Err(diagnosis)`：基线不可解析（分支被删/存量皆无/仓库不可用）——
///   **fail-closed 不再跳过**（废弃 fail-safe：provider 原生通道软限制的
///   残余风险以本核对为唯一硬兜底，跳过=兜底失效）。
pub(crate) fn plan_baseline_tree(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
) -> Result<Option<std::collections::BTreeSet<String>>, String> {
    let paths = lifecycle.app_paths();
    let issue = crate::product::issue_store::IssueStore::new(paths.clone())
        .get(project_id, issue_id)
        .map_err(|error| format!("load issue for plan baseline failed: {error}"))?;
    let Some(repo_id) = issue.repo_id.as_deref() else {
        return Ok(None);
    };
    let repo_path = plan_baseline_repository_path(&paths, project_id, repo_id)?;
    let branch = crate::product::issue_baseline::resolve_effective_base_branch(
        &repo_path,
        issue.base_branch.as_deref(),
    )
    .map_err(|error| error.diagnosis())?;
    let reference = format!("refs/heads/{branch}");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo_path)
        .args(["ls-tree", "-r", "--name-only"])
        .arg(&reference)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| {
            format!(
                "git ls-tree {reference} in {}: {error}",
                repo_path.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "git ls-tree {reference} in {} failed: {}",
            repo_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
    ))
}

/// issue 默认仓 → 主检出路径（与 advance 的 `resolve_advance_repository`
/// 同口径：dual 物理仓优先解析，回退 id 精确匹配）。
fn plan_baseline_repository_path(
    paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    repo_id: &str,
) -> Result<std::path::PathBuf, String> {
    let project = crate::product::project_store::ProjectStore::new(paths.clone())
        .get(project_id)
        .map_err(|error| format!("load project for plan baseline failed: {error}"))?;
    let store =
        crate::product::repository_store::RepositoryStore::for_project(paths.clone(), &project);
    store
        .resolve_legacy_physical_repository_if_dual(project_id, repo_id)
        .map(|(_, _, repository)| repository.path)
        .or_else(|_| {
            store
                .list(project_id)
                .map_err(|error| format!("list repositories for plan baseline failed: {error}"))?
                .into_iter()
                .find(|repository| repository.id == repo_id)
                .map(|repository| repository.path)
                .ok_or_else(|| format!("plan baseline repository not found: {repo_id}"))
        })
}

/// preflight code → 确定性身份定位器（跨轮指纹稳定：category+contract_field）。
fn preflight_contract_field(code: &str) -> String {
    match code {
        "integration_work_item_required" => "plan_options.include_integration_tests".to_string(),
        "e2e_work_item_required" => "plan_options.include_e2e_tests".to_string(),
        "frontend_backend_split_required" => {
            "plan_options.force_frontend_backend_split".to_string()
        }
        other => format!("plan_preflight.{other}"),
    }
}

/// preflight code → 与校验器消息同源的修复路径（REQ-WSC-06 口径一致纪律）。
fn preflight_required_action(code: &str) -> String {
    match code {
        "integration_work_item_required" => {
            INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION.to_string()
        }
        "e2e_work_item_required" => E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION.to_string(),
        "frontend_backend_split_required" => {
            FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION.to_string()
        }
        "acceptance_path_not_in_baseline" => {
            crate::product::work_item_plan_compiler::ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION
                .to_string()
        }
        other => format!("按 finding 消息修复 {other}"),
    }
}

/// `WorkItemSplitFinding` → `ReviewFinding` 适配：
/// - Error → MustFix + category=ContractGap + class_hint=Repairable（policy 侧
///   归入自动返修通道，与 reviewer 契约缺口同池消费预算）；
/// - Warning → Suggestion + class_hint=Advisory（仅在已有 verdict 时搭车）。
fn preflight_finding_to_review_finding(finding: &WorkItemSplitFinding) -> ReviewFinding {
    use crate::product::models::WorkItemSplitFindingSeverity;
    let is_error = finding.severity == WorkItemSplitFindingSeverity::Error;
    ReviewFinding {
        severity: if is_error {
            ReviewFindingSeverity::MustFix
        } else {
            ReviewFindingSeverity::Suggestion
        },
        message: format!("[{}] {}", finding.code, finding.message),
        evidence: format!(
            "plan preflight mechanical finding；work_items: [{}]",
            finding.work_item_ids.join(", ")
        ),
        required_action: if is_error {
            preflight_required_action(&finding.code)
        } else {
            String::new()
        },
        category: Some(if is_error {
            crate::product::work_item_plan_policy::ReviewFindingCategory::ContractGap
        } else {
            crate::product::work_item_plan_policy::ReviewFindingCategory::Completeness
        }),
        class_hint: Some(if is_error {
            crate::product::work_item_plan_policy::FindingClassHint::Repairable
        } else {
            crate::product::work_item_plan_policy::FindingClassHint::Advisory
        }),
        contract_field: Some(preflight_contract_field(&finding.code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with(code: &str, message: &str) -> PlanCandidateMechanicalReport {
        PlanCandidateMechanicalReport {
            source_revision_hash: "a".repeat(64),
            compiler_version: "work_item_plan_compiler/v1".to_string(),
            findings: vec![WorkItemSplitFinding {
                code: code.to_string(),
                message: message.to_string(),
                work_item_ids: Vec::new(),
                severity: crate::product::models::WorkItemSplitFindingSeverity::Error,
            }],
        }
    }

    #[test]
    fn clean_report_produces_no_verdict() {
        let report = PlanCandidateMechanicalReport {
            source_revision_hash: "a".repeat(64),
            compiler_version: "work_item_plan_compiler/v1".to_string(),
            findings: Vec::new(),
        };
        assert!(preflight_review_verdict(&report).is_none());
    }

    #[test]
    fn options_gap_adapts_to_mechanical_revision_verdict() {
        let report = report_with(
            "integration_work_item_required",
            "include_integration_tests is enabled but the plan does not contain an integration work item；修复动作：新增一个 kind=integration 的 Work Item",
        );
        let verdict = preflight_review_verdict(&report).expect("gap must produce verdict");
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        let finding = &verdict.findings[0];
        assert_eq!(finding.severity, ReviewFindingSeverity::MustFix);
        assert_eq!(
            finding.contract_field.as_deref(),
            Some("plan_options.include_integration_tests")
        );
        assert!(
            finding.required_action.contains("kind=integration"),
            "required_action 与校验器消息同源：{}",
            finding.required_action
        );
        assert!(preflight_readable_output(&verdict).contains("[plan_preflight]"));
    }

    #[test]
    fn acceptance_path_gap_adapts_to_mechanical_revision_verdict() {
        let report = report_with(
            "acceptance_path_not_in_baseline",
            "work item WI-001 的验收标准/验证计划引用路径 [status.html] 不存在于 plan 基线树",
        );
        let verdict = preflight_review_verdict(&report).expect("gap must produce verdict");
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        let finding = &verdict.findings[0];
        assert_eq!(finding.severity, ReviewFindingSeverity::MustFix);
        assert_eq!(
            finding.contract_field.as_deref(),
            Some("plan_preflight.acceptance_path_not_in_baseline")
        );
        assert_eq!(
            finding.required_action,
            crate::product::work_item_plan_compiler::ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION,
            "required_action 与共享常量逐字同源（REQ-WSC-06 口径一致）"
        );
    }

    #[test]
    fn non_preflight_errors_do_not_produce_verdict() {
        let report = report_with("traceability_refs_required", "unrelated structural error");
        assert!(preflight_review_verdict(&report).is_none());
    }

    // ---- REQ-PIB-03 T3.2：plan_baseline_tree 按分支名取树 + fail-closed ----

    struct PlanBaselineFixture {
        // TempDir 保活：drop 即删除（.aria 记录树与 git 仓都必须活过断言期）。
        _aria_root: tempfile::TempDir,
        _repo: tempfile::TempDir,
        lifecycle: crate::product::lifecycle_store::LifecycleStore,
        repo_path: std::path::PathBuf,
        project_id: String,
        issue_id: String,
    }

    fn git_at(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .status()
            .expect("git fixture command");
        assert!(status.success(), "git {args:?} in {}", dir.display());
    }

    /// 真实 git 仓 + 项目/仓库/issue 记录。`extra_branch=(branch, file)` 在
    /// 主提交后追加分支专属提交；`sibling_worktree=true` 建兄弟 worktree 并在
    /// 其分支上提交 status.html（F-56 现场形态：磁盘存在、main 树内不存在）。
    fn plan_baseline_fixture(
        initial_branch: &str,
        base_branch: Option<&str>,
        extra_branch: Option<(&str, &str)>,
        sibling_worktree: bool,
    ) -> PlanBaselineFixture {
        let aria_root = tempfile::tempdir().expect("aria root");
        let repo = tempfile::tempdir().expect("repo dir");
        let repo_path = repo.path().to_path_buf();
        git_at(repo.path(), &["init", "-b", initial_branch]);
        git_at(repo.path(), &["config", "user.email", "test@example.com"]);
        git_at(repo.path(), &["config", "user.name", "Test User"]);
        std::fs::write(repo.path().join("package.json"), "{}\n").expect("package.json");
        git_at(repo.path(), &["add", "package.json"]);
        git_at(repo.path(), &["commit", "-m", "baseline"]);
        if let Some((branch, file)) = extra_branch {
            git_at(repo.path(), &["checkout", "-b", branch]);
            std::fs::write(repo.path().join(file), "feature\n").expect("feature file");
            git_at(repo.path(), &["add", file]);
            git_at(repo.path(), &["commit", "-m", "feature"]);
            git_at(repo.path(), &["checkout", initial_branch]);
        }
        if sibling_worktree {
            let sibling = repo.path().join(".worktrees/aria-issues/issue_0001");
            git_at(
                repo.path(),
                &[
                    "worktree",
                    "add",
                    ".worktrees/aria-issues/issue_0001",
                    "-b",
                    "sibling",
                ],
            );
            std::fs::write(sibling.join("status.html"), "sibling\n").expect("sibling file");
            git_at(&sibling, &["add", "status.html"]);
            git_at(&sibling, &["commit", "-m", "sibling"]);
        }
        let app_paths =
            crate::product::app_paths::ProductAppPaths::new(aria_root.path().join(".aria"));
        crate::product::project_store::ProjectStore::new(app_paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "plan baseline fixture".to_string(),
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
                idempotency_key: format!(
                    "plan-baseline-fixture-{initial_branch}-{}",
                    extra_branch.map(|(branch, _)| branch).unwrap_or("none")
                ),
            })
            .expect("create repository");
        crate::product::issue_store::IssueStore::new(app_paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                logical_codebase_id: None,
                base_branch: base_branch.map(str::to_string),
                title: "Plan baseline".to_string(),
                description: None,
                change_id: None,
            })
            .expect("create issue");
        PlanBaselineFixture {
            _aria_root: aria_root,
            _repo: repo,
            lifecycle: crate::product::lifecycle_store::LifecycleStore::new(app_paths),
            repo_path,
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
        }
    }

    /// 场景：author 期（无任何共享 coding worktree 记录）也能取树；默认链
    /// None→main；树=分支全量文件清单（三面同源同一解析链）。
    #[test]
    fn plan_baseline_tree_loads_default_branch_without_shared_worktree() {
        let fixture = plan_baseline_fixture("main", None, None, false);
        // 无共享 worktree 记录（author 期首用场景——不依赖共享 worktree）。
        assert!(
            fixture
                .lifecycle
                .get_issue_shared_worktree(&fixture.project_id, &fixture.issue_id)
                .ok()
                .flatten()
                .is_none()
        );
        let tree = plan_baseline_tree(&fixture.lifecycle, &fixture.project_id, &fixture.issue_id)
            .expect("default chain must resolve")
            .expect("repo-backed issue must yield a tree");
        assert!(tree.contains("package.json"));
        assert!(!tree.contains(".worktrees"));
    }

    /// 场景：锁定分支 feature/x → 树含该分支专属文件（fork/核对/所见同源）。
    #[test]
    fn plan_baseline_tree_reads_locked_feature_branch_tree() {
        let fixture = plan_baseline_fixture(
            "main",
            Some("feature/x"),
            Some(("feature/x", "flag.css")),
            false,
        );
        let tree = plan_baseline_tree(&fixture.lifecycle, &fixture.project_id, &fixture.issue_id)
            .expect("locked branch must resolve")
            .expect("repo-backed issue must yield a tree");
        assert!(tree.contains("flag.css"));
        assert!(tree.contains("package.json"));
    }

    /// 场景（REQ-PIB-02 场景 5 / F-56+F-57 谓词回放）：兄弟 worktree 独有
    /// 文件（磁盘存在、基线树内不存在）与工作区脏文件都不得进入基线树——
    /// 该谓词（!tree.contains(path)）正是 C1 拦截软限制漏网 AC 引用的
    /// acceptance_path_not_in_baseline 判定（机制见 compiler
    /// preflight_baseline_paths 测试）。
    #[test]
    fn plan_baseline_tree_excludes_sibling_and_disk_only_paths() {
        let fixture = plan_baseline_fixture("main", None, None, true);
        // 工作区脏文件（磁盘存在、main 树内不存在）。
        std::fs::write(fixture.repo_path.join("notes.txt"), "dirty\n")
            .expect("dirty working tree file");
        let tree = plan_baseline_tree(&fixture.lifecycle, &fixture.project_id, &fixture.issue_id)
            .expect("baseline must resolve")
            .expect("repo-backed issue must yield a tree");
        assert_eq!(
            tree.iter().cloned().collect::<Vec<_>>(),
            vec!["package.json".to_string()],
            "基线树=main 树全量：兄弟件 status.html / .worktrees 路径 / 工作区脏件一律不可见"
        );
        assert!(!tree.contains("status.html"));
        assert!(!tree.contains(".worktrees/aria-issues/issue_0001/status.html"));
    }

    /// 场景（REQ-PIB-03 场景 3）：基线分支被删 → Err fail-closed 不跳过。
    #[test]
    fn plan_baseline_tree_fails_closed_when_branch_missing() {
        let fixture = plan_baseline_fixture("main", Some("gone"), None, false);
        let error = plan_baseline_tree(&fixture.lifecycle, &fixture.project_id, &fixture.issue_id)
            .expect_err("missing branch must fail closed");
        assert!(error.contains("基准分支不存在"), "{error}");
        assert!(error.contains("gone"), "{error}");
    }

    /// 场景（存量皆无仓）：repo 无 main/master 且 issue 无显式基线 → Err
    /// 诊断「无法推断默认基准分支」（不回退、不猜）。
    #[test]
    fn plan_baseline_tree_fails_closed_without_default_branch() {
        let fixture = plan_baseline_fixture("trunk", None, None, false);
        let error = plan_baseline_tree(&fixture.lifecycle, &fixture.project_id, &fixture.issue_id)
            .expect_err("no default branch must fail closed");
        assert!(error.contains("无法推断默认基准分支"), "{error}");
    }

    /// 无仓 issue（repo_id=None，逻辑代码库 Non-Goal 面）→ Ok(None) 核对不触发。
    #[test]
    fn plan_baseline_tree_returns_none_for_repoless_issue() {
        let aria_root = tempfile::tempdir().expect("aria root");
        let app_paths =
            crate::product::app_paths::ProductAppPaths::new(aria_root.path().join(".aria"));
        crate::product::project_store::ProjectStore::new(app_paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "repoless fixture".to_string(),
                description: None,
            })
            .expect("create project");
        crate::product::issue_store::IssueStore::new(app_paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: None,
                logical_codebase_id: Some("lc_0001".to_string()),
                base_branch: None,
                title: "Repoless".to_string(),
                description: None,
                change_id: None,
            })
            .expect("create repoless issue");
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(app_paths);
        let tree = plan_baseline_tree(&lifecycle, "project_0001", "issue_0001")
            .expect("repoless issue must not fail");
        assert!(tree.is_none());
    }
}
