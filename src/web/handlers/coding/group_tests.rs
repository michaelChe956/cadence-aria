use std::collections::BTreeMap;
use std::fs;
use std::process::Command;

use super::*;
use crate::product::coding_attempt_store::AuthoritativeCodingUnitBinding;
use crate::product::issue_store::CreateProductIssueInput;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, IssueCodebaseSelection, IssueCodebaseSelectionStore,
    LogicalCodebaseFeature, LogicalCodebaseStore,
};
use crate::product::project_store::CreateProjectInput;
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};

struct SplitResolutionFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    project_id: String,
    issue_id: String,
    targets: Vec<LogicalRepositoryId>,
}

impl SplitResolutionFixture {
    /// 重写 issue selection（focus 形态由用例自定）。
    fn save_selection(
        &self,
        included: Vec<LogicalRepositoryId>,
        focus: Vec<LogicalRepositoryId>,
    ) {
        IssueCodebaseSelectionStore::new(self.paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                &self.project_id,
                &self.issue_id,
                included,
                Vec::new(),
                focus,
                None,
            ))
            .unwrap();
    }
}

fn split_resolution_fixture() -> SplitResolutionFixture {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "split resolution".to_string(),
            description: None,
        })
        .unwrap();
    let mut targets = Vec::new();
    for name in ["api", "web"] {
        let canonical_path = root.path().join(name);
        fs::create_dir_all(&canonical_path).unwrap();
        run_git(&canonical_path, &["init", "--quiet"]);
        run_git(
            &canonical_path,
            &["config", "user.email", "split@example.test"],
        );
        run_git(
            &canonical_path,
            &["config", "user.name", "Split Resolution"],
        );
        fs::write(canonical_path.join("README.md"), format!("# {name}\n")).unwrap();
        run_git(&canonical_path, &["add", "README.md"]);
        run_git(
            &canonical_path,
            &["commit", "--quiet", "-m", "initial commit"],
        );
        let repository = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        )
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: name.to_string(),
            path: canonical_path,
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: format!("split-resolution-{name}"),
        })
        .unwrap();
        targets.push(
            repository
                .logical_repository_id
                .expect("logical repository ID"),
        );
    }
    let manifest = LogicalCodebaseStore::new(paths.clone())
        .load_manifest(&project.id)
        .unwrap()
        .expect("manifest");
    AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .unwrap();
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: None,
            logical_codebase_id: None,
            title: "mixed-target group".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
    let fixture = SplitResolutionFixture {
        _root: root,
        paths,
        project_id: project.id,
        issue_id: issue.id,
        targets,
    };
    fixture.save_selection(fixture.targets.clone(), vec![fixture.targets[0]]);
    fixture
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("start git");
    assert!(status.success(), "git {} failed", args.join(" "));
}

fn unit_binding(target: Option<LogicalRepositoryId>) -> AuthoritativeCodingUnitBinding {
    AuthoritativeCodingUnitBinding {
        logical_work_item_id: "work_item_0001".to_string(),
        work_item_revision_id: "revision_0001".to_string(),
        verification_plan_revision_id: "verification_0001".to_string(),
        projection_bundle_id: "projection_0001".to_string(),
        target_repository_id: target,
        source_draft_error: None,
        dependency_logical_work_item_ids: Vec::new(),
    }
}

fn authoritative(units: Vec<AuthoritativeCodingUnitBinding>) -> AuthoritativeGroupPlanBinding {
    AuthoritativeGroupPlanBinding {
        plan_revision_id: "plan_revision_0001".to_string(),
        dependency_graph_revision_id: "dependency_graph_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_0001".to_string(),
        units,
    }
}

#[test]
fn group_target_snapshots_resolves_mixed_targets_per_target() {
    // REQ-COD-04（WP1 分流化）：mixed-target 不再构成拒绝理由——按 unit target
    // 分组逐仓产出冻结快照（REQ-MTG-01 scenario「尝试 mixed-target group」解析面）。
    let fixture = split_resolution_fixture();
    let [api, web] = fixture.targets.as_slice() else {
        panic!("fixture must register two targets");
    };
    let binding = authoritative(vec![unit_binding(Some(*api)), unit_binding(Some(*web))]);

    let snapshots: BTreeMap<LogicalRepositoryId, AttemptTargetSnapshot> =
        group_target_snapshots(
            &fixture.paths,
            &fixture.project_id,
            &fixture.issue_id,
            &binding,
        )
        .expect("mixed-target split resolution must succeed")
        .expect("logical routing must yield a target map");

    assert_eq!(snapshots.len(), 2, "one snapshot per target");
    assert_eq!(snapshots[api].logical_repository_id, *api);
    assert_eq!(snapshots[web].logical_repository_id, *web);
    assert!(snapshots[api].revision.is_some(), "git HEAD captured");
    assert!(snapshots[web].revision.is_some(), "git HEAD captured");
}

#[test]
fn resolve_group_repositories_resolves_mixed_targets_per_target() {
    let fixture = split_resolution_fixture();
    let [api, web] = fixture.targets.as_slice() else {
        panic!("fixture must register two targets");
    };
    let binding = authoritative(vec![unit_binding(Some(*api)), unit_binding(Some(*web))]);

    let repositories = resolve_group_repositories(
        &fixture.paths,
        &fixture.project_id,
        &fixture.issue_id,
        &binding,
    )
    .expect("mixed-target split resolution must succeed")
    .expect("logical routing must yield a repository map");

    assert_eq!(repositories.len(), 2, "one repository per target");
    assert_eq!(repositories[api].logical_repository_id, Some(*api));
    assert_eq!(repositories[web].logical_repository_id, Some(*web));
}

#[test]
fn group_target_snapshots_single_target_keeps_single_entry() {
    // 单 target 语义零变化：多值面退化为单条目，与现行单值面一致。
    let fixture = split_resolution_fixture();
    let [api, _] = fixture.targets.as_slice() else {
        panic!("fixture must register two targets");
    };
    let binding = authoritative(vec![unit_binding(Some(*api)), unit_binding(Some(*api))]);

    let snapshots = group_target_snapshots(
        &fixture.paths,
        &fixture.project_id,
        &fixture.issue_id,
        &binding,
    )
    .unwrap()
    .expect("logical routing");

    assert_eq!(snapshots.len(), 1);
    assert!(snapshots.contains_key(api));
}

#[test]
fn group_target_snapshots_zero_target_unique_focus_falls_back_to_focus() {
    // 0-target focus 唯一回落语义原样保留（「单 target 路径」的一部分）。
    let fixture = split_resolution_fixture();
    let [api, _] = fixture.targets.as_slice() else {
        panic!("fixture must register two targets");
    };
    let binding = authoritative(vec![unit_binding(None), unit_binding(None)]);

    let snapshots = group_target_snapshots(
        &fixture.paths,
        &fixture.project_id,
        &fixture.issue_id,
        &binding,
    )
    .unwrap()
    .expect("logical routing");

    assert_eq!(snapshots.len(), 1);
    assert!(snapshots.contains_key(api));
}

#[test]
fn group_target_snapshots_zero_target_without_unique_focus_fails_closed() {
    // REQ-COD-04 scenario「无唯一 target 归属仍 fail-closed」：units 均无
    // target 且 focus 多点 → TargetMissing 稳定码，不静默选 target。
    let fixture = split_resolution_fixture();
    fixture.save_selection(fixture.targets.clone(), fixture.targets.clone());
    let binding = authoritative(vec![unit_binding(None), unit_binding(None)]);

    let error = group_target_snapshots(
        &fixture.paths,
        &fixture.project_id,
        &fixture.issue_id,
        &binding,
    )
    .unwrap_err();

    assert_eq!(error.code, "repository_routing_target_missing");
}

#[test]
fn group_target_snapshots_target_outside_selection_fails_closed() {
    // 有效 selection 之外的 target 保持 TargetUnknown fail-closed。
    let fixture = split_resolution_fixture();
    let [api, web] = fixture.targets.as_slice() else {
        panic!("fixture must register two targets");
    };
    fixture.save_selection(vec![*api], vec![*api]);
    let binding = authoritative(vec![unit_binding(Some(*api)), unit_binding(Some(*web))]);

    let error = group_target_snapshots(
        &fixture.paths,
        &fixture.project_id,
        &fixture.issue_id,
        &binding,
    )
    .unwrap_err();

    assert_eq!(error.code, "repository_routing_target_unknown");
}

#[test]
fn group_target_snapshots_source_draft_error_stays_inconsistent() {
    // source_draft_error fail-closed 前置保留（溯源断链不进分流解析）。
    let fixture = split_resolution_fixture();
    let [api, web] = fixture.targets.as_slice() else {
        panic!("fixture must register two targets");
    };
    let mut broken = unit_binding(Some(*api));
    broken.source_draft_error = Some("source draft missing".to_string());
    let binding = authoritative(vec![broken, unit_binding(Some(*web))]);

    let error = group_target_snapshots(
        &fixture.paths,
        &fixture.project_id,
        &fixture.issue_id,
        &binding,
    )
    .unwrap_err();

    assert_eq!(error.code, "repository_routing_inconsistent");
}

#[test]
fn group_worktree_layout_repository_route_writes_repo_scoped_record() {
    // 缺陷 #13 层2（组入口路由分流缺口）：Logical 路由（target_snapshot
    // 存在）的组 worktree 布局必须写仓维 record，不得写 issue 维 legacy
    // 布局——后者与运行期 preflight_repo_shared_worktree_absent 契约相反
    // （恢复 / handoff / 完成门均 legacy_shared_worktree_present fail-closed）。
    let fixture = split_resolution_fixture();
    let [api, _] = fixture.targets.as_slice() else {
        panic!("fixture must register two targets");
    };
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone());
    let route = IssueWorktreeRoute::Repository { repository_id: *api };

    let lease = upsert_worktree_and_acquire_lease(
        &fixture.paths,
        &lifecycle,
        &route,
        &fixture.project_id,
        &fixture.issue_id,
        "repository_physical_0001",
        "work_item_0001",
        "issue_worktree_lease_layout_test",
        "aria/issues/issue_0001",
        "main",
        fixture.paths.issue_root(&fixture.project_id, &fixture.issue_id).join("wt"),
    )
    .expect("repository route worktree layout binding");

    assert!(lease.acquired, "first acquisition on a fresh repo record");
    let issue_root = fixture
        .paths
        .issue_root(&fixture.project_id, &fixture.issue_id);
    assert!(
        !issue_root.join("issue-shared-worktree.json").exists(),
        "repository route must not write the legacy issue-scoped layout"
    );
    let repo_record = issue_root
        .join("shared-worktrees")
        .join(format!("{}.json", api.0));
    assert!(
        repo_record.exists(),
        "repository route must write the repo-scoped shared worktree record"
    );
}

#[test]
fn group_worktree_layout_repository_route_fails_closed_on_legacy_record() {
    // 迁移契约 §4.2.6 在组入口同样生效：旧 issue 维 record 存在 →
    // 422 legacy_shared_worktree_present，绝不静默覆盖、绝不从旧文件推导。
    let fixture = split_resolution_fixture();
    let [api, _] = fixture.targets.as_slice() else {
        panic!("fixture must register two targets");
    };
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: fixture.project_id.clone(),
            issue_id: fixture.issue_id.clone(),
            repository_id: "repository_physical_legacy".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: fixture
                .paths
                .issue_root(&fixture.project_id, &fixture.issue_id)
                .join("legacy-wt"),
            base_branch: "main".to_string(),
        })
        .unwrap();

    let error = upsert_worktree_and_acquire_lease(
        &fixture.paths,
        &lifecycle,
        &IssueWorktreeRoute::Repository { repository_id: *api },
        &fixture.project_id,
        &fixture.issue_id,
        "repository_physical_0001",
        "work_item_0001",
        "issue_worktree_lease_layout_test",
        "aria/issues/issue_0001",
        "main",
        fixture
            .paths
            .issue_root(&fixture.project_id, &fixture.issue_id)
            .join("wt"),
    )
    .unwrap_err();

    assert_eq!(error.code, "legacy_shared_worktree_present");
}

#[test]
fn group_worktree_layout_legacy_route_keeps_issue_scoped_record() {
    // Legacy 路由（无 target_snapshot）回归红线：issue 维布局行为不变。
    let fixture = split_resolution_fixture();
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone());

    let lease = upsert_worktree_and_acquire_lease(
        &fixture.paths,
        &lifecycle,
        &IssueWorktreeRoute::Legacy,
        &fixture.project_id,
        &fixture.issue_id,
        "repository_physical_legacy",
        "work_item_0001",
        "issue_worktree_lease_legacy_test",
        "aria/issues/issue_0001",
        "main",
        fixture
            .paths
            .issue_root(&fixture.project_id, &fixture.issue_id)
            .join("legacy-wt"),
    )
    .expect("legacy route worktree layout binding");

    assert!(lease.acquired);
    assert!(
        fixture
            .paths
            .issue_root(&fixture.project_id, &fixture.issue_id)
            .join("issue-shared-worktree.json")
            .exists(),
        "legacy route keeps the issue-scoped layout"
    );
}
