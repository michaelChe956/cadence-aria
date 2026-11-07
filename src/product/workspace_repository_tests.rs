use super::*;
use crate::product::logical_codebase::{
    IssueCodebaseSelection, LogicalCodebaseManifest, LogicalCodebaseStore,
};

fn write_manifest_fixture(paths: &ProductAppPaths, project_id: &str) {
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest(
            project_id,
            &LogicalCodebaseManifest::new(
                project_id,
                paths.root().join("aggregate-root"),
                Vec::new(),
            ),
        )
        .unwrap();
}

/// 落一条归属 legacy 别名逻辑代码库的 issue 记录。
///
/// REQ-ROUTE-01：routing 只从 issue 的持久归属解析 LC——issue 记录缺失时
/// `load_for_issue` 按 T1.2 契约返回 `NotFound`（不是静默路由/回退）。归属填
/// legacy 别名 id：这些夹具的 manifest/member/selection 写在项目级 legacy
/// 布局，而 `for_lc(legacy_logical_codebase_id(project))` 与该布局字节等价
///（`store::lc_scope_root` 明文契约），故解析命中的是同一份权威数据。
fn write_legacy_aliased_issue(paths: &ProductAppPaths, project_id: &str, title: &str) {
    crate::product::issue_store::IssueStore::new(paths.clone())
        .create(crate::product::issue_store::CreateProductIssueInput {
            project_id: project_id.to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id:
                crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id),
            title: title.to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
}

#[test]
fn routing_without_manifest_and_selection_fails_closed() {
    // REQ-ROUTE-01 两态中枢：issue 已归属 LC 但子树无 manifest/selection
    //（(None, None)）→ FailClosed + 稳定码 repository_routing_target_missing，
    // 不再回退 Legacy 单仓路由；issue 不存在则按 T1.2 契约返回 NotFound
    // 错误（非 Ok(Legacy)）。
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    let issue = crate::product::issue_store::IssueStore::new(paths.clone())
        .create(crate::product::issue_store::CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: lc_id,
            title: "unrouted issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
    let routing = RepositoryRouting::load_for_issue(&paths, "project_0001", &issue.id).unwrap();
    match routing {
        RepositoryRouting::FailClosed { code, .. } => {
            assert_eq!(code.stable_code(), "repository_routing_target_missing");
        }
        RepositoryRouting::Logical { .. } => {
            panic!("(None, None) must fail closed, got Logical")
        }
    }
    match RepositoryRouting::load_for_issue(&paths, "project_0001", "issue_9999") {
        Err(crate::product::json_store::ProductStoreError::NotFound { kind, .. }) => {
            assert_eq!(kind, "issue");
        }
        _ => panic!("missing issue must surface NotFound, not a silent route"),
    }
}

#[test]
fn load_routing_some_none_is_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write_manifest_fixture(&paths, "project_0001");
    // 不变量：(manifest, selection) 不成对（Some, None）⇒ FailClosed。
    // issue 记录须在场，否则先撞 T1.2 契约的 NotFound 而非本不变量。
    write_legacy_aliased_issue(&paths, "project_0001", "manifest-only issue");
    let routing = RepositoryRouting::load_for_issue(&paths, "project_0001", "issue_0001").unwrap();
    assert!(matches!(routing, RepositoryRouting::FailClosed { .. }));
}

#[test]
fn plan_session_target_zero_targets_stays_target_missing_even_with_unique_focus() {
    // 双审定案：WorkItemPlan 面 0-target 保持 TargetMissing fail-closed 红线，
    // 即使 focus 唯一也不回落（与创建面 0-target focus 语义不对称是定案）。
    let focus = LogicalRepositoryId(uuid::Uuid::new_v4());
    let selection = IssueCodebaseSelection::explicit(
        "project_0001",
        "issue_0001",
        vec![focus],
        Vec::new(),
        vec![focus],
        None,
    );

    let error = plan_session_repository_target(BTreeSet::new(), "work_item_plan_0001", &selection)
        .unwrap_err();

    assert_eq!(
        stable_routing_code(&error),
        "repository_routing_target_missing"
    );
}

#[test]
fn plan_session_target_single_target_resolution_unchanged() {
    let target = LogicalRepositoryId(uuid::Uuid::new_v4());
    let selection = IssueCodebaseSelection::explicit(
        "project_0001",
        "issue_0001",
        vec![target],
        Vec::new(),
        vec![target],
        None,
    );
    let targets = BTreeSet::from([target]);

    let resolved =
        plan_session_repository_target(targets, "work_item_plan_0001", &selection).unwrap();

    assert_eq!(resolved, PlanSessionRepositoryTarget::Member(target));
}

#[test]
fn plan_session_target_multi_target_falls_back_to_unique_focus() {
    // REQ-COD-04（WP1 分流化）：plan 会话路由面——多 target 回落 selection
    // focus 唯一解析（与创建面 0-target focus 语义对称），不再一律 TargetAmbiguous。
    let api = LogicalRepositoryId(uuid::Uuid::new_v4());
    let web = LogicalRepositoryId(uuid::Uuid::new_v4());
    let selection = IssueCodebaseSelection::explicit(
        "project_0001",
        "issue_0001",
        vec![api, web],
        Vec::new(),
        vec![api],
        None,
    );
    let targets = BTreeSet::from([api, web]);

    let resolved =
        plan_session_repository_target(targets, "work_item_plan_0001", &selection).unwrap();

    assert_eq!(resolved, PlanSessionRepositoryTarget::Member(api));
}

#[test]
fn plan_session_target_multi_target_with_multi_focus_anchors_aggregate_root() {
    // S6 方案 B（controller 2026-10-10）：多 target + 多 focus 不再
    // TargetAmbiguous——plan 会话=orchestrator 角色，锚定聚合根视图。
    let api = LogicalRepositoryId(uuid::Uuid::new_v4());
    let web = LogicalRepositoryId(uuid::Uuid::new_v4());
    let selection = IssueCodebaseSelection::explicit(
        "project_0001",
        "issue_0001",
        vec![api, web],
        Vec::new(),
        vec![api, web],
        None,
    );
    let targets = BTreeSet::from([api, web]);

    let resolved =
        plan_session_repository_target(targets, "work_item_plan_0001", &selection).unwrap();

    assert_eq!(resolved, PlanSessionRepositoryTarget::AggregateRoot);
}

#[test]
fn plan_session_target_four_targets_with_four_focus_no_longer_ambiguous() {
    // S6（2026-10-10 全旅程 E2E）红→绿主锁：四仓纵切 plan 会话
    // （durable 现场 selection_policy=explicit、included=4、focus=4）
    // 在路由面不得再 TargetAmbiguous——方案 B（controller 2026-10-10）：
    // plan 会话=orchestrator 角色，focus 非唯一时锚定聚合根视图。
    let targets: Vec<LogicalRepositoryId> = (0..4)
        .map(|_| LogicalRepositoryId(uuid::Uuid::new_v4()))
        .collect();
    let selection = IssueCodebaseSelection::explicit(
        "project_0001",
        "issue_0001",
        targets.clone(),
        Vec::new(),
        targets.clone(),
        None,
    );

    let outcome = plan_session_repository_target(
        targets.into_iter().collect(),
        "issue_work_item_plan_0001",
        &selection,
    );

    assert!(
        matches!(outcome, Ok(PlanSessionRepositoryTarget::AggregateRoot)),
        "multi-target multi-focus plan session must anchor at the aggregate root: {outcome:?}"
    );
}

#[test]
fn plan_session_target_multi_target_without_focus_anchors_aggregate_root() {
    let api = LogicalRepositoryId(uuid::Uuid::new_v4());
    let web = LogicalRepositoryId(uuid::Uuid::new_v4());
    let selection = IssueCodebaseSelection::explicit(
        "project_0001",
        "issue_0001",
        vec![api, web],
        Vec::new(),
        Vec::new(),
        None,
    );
    let targets = BTreeSet::from([api, web]);

    let resolved =
        plan_session_repository_target(targets, "work_item_plan_0001", &selection).unwrap();

    assert_eq!(resolved, PlanSessionRepositoryTarget::AggregateRoot);
}

fn stable_routing_code(error: &ProductStoreError) -> &str {
    let ProductStoreError::InvalidRecord { reason, .. } = error else {
        panic!("expected repository_routing InvalidRecord, got {error:?}");
    };
    reason.split(':').next().unwrap_or_default()
}

/// 缺陷 #2（2026-10-02 全链 E2E）回归夹具：LC 路由（manifest+selection+
/// 成员 git 仓）下的 Story 会话解析。
struct LcStoryRoutingFixture {
    _temp: tempfile::TempDir,
    paths: ProductAppPaths,
    manifest: LogicalCodebaseManifest,
    member_logical_id: LogicalRepositoryId,
    member_b_logical_id: LogicalRepositoryId,
    member_path: std::path::PathBuf,
}

impl LcStoryRoutingFixture {
    fn register_member(
        paths: &ProductAppPaths,
        root: &std::path::Path,
        name: &str,
    ) -> (LogicalRepositoryId, std::path::PathBuf) {
        let member_path = root.join(name);
        std::fs::create_dir_all(&member_path).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "routing@example.test"],
            vec!["config", "user.name", "Routing Fixture"],
        ] {
            let status = std::process::Command::new("git")
                .args(&args)
                .current_dir(&member_path)
                .status()
                .unwrap();
            assert!(status.success(), "git {:?} failed", args);
        }
        std::fs::write(member_path.join("README.md"), format!("# {name}\n")).unwrap();
        for args in [
            vec!["add", "README.md"],
            vec!["commit", "--quiet", "-m", "initial commit"],
        ] {
            let status = std::process::Command::new("git")
                .args(&args)
                .current_dir(&member_path)
                .status()
                .unwrap();
            assert!(status.success(), "git {:?} failed", args);
        }
        let repository =
            crate::product::repository_store::RepositoryStore::with_logical_codebase_feature(
                paths.clone(),
                crate::product::logical_codebase::LogicalCodebaseFeature::enabled(),
            )
            .create(crate::product::repository_store::CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: name.to_string(),
                path: member_path.clone(),
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: format!("lc-routing-{name}"),
            })
            .unwrap();
        (
            repository.logical_repository_id.expect("logical id"),
            member_path,
        )
    }

    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "lc-routing".to_string(),
                description: None,
            })
            .unwrap();
        let root = temp.path().join("aggregate-root");
        let (member_logical_id, member_path) = Self::register_member(&paths, &root, "member-a");
        let (member_b_logical_id, _) = Self::register_member(&paths, &root, "member-b");
        let manifest = LogicalCodebaseStore::new(paths.clone())
            .load_manifest("project_0001")
            .unwrap()
            .expect("manifest auto-created by member registration");
        // REQ-ROUTE-01：routing 只从 issue 的持久归属解析 LC——issue 记录缺失
        // 时 `load_for_issue` 按 T1.2 契约返回 NotFound（不是静默路由/回退）。
        // ⚠️ 必须在写 selection **之前**建 issue：selection store 会生成
        // `issues/issue_0001/` 目录，而 `next_sequential_id_in_directory` 据目录数
        // 分配序号，先写 selection 会把 issue 顶成 `issue_0002`，与夹具里到处
        // 硬编码的 `issue_0001` 全部错位。
        write_legacy_aliased_issue(&paths, "project_0001", "lc routing fixture issue");
        crate::product::logical_codebase::IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::all_members(
                "project_0001",
                "issue_0001",
                None,
            ))
            .unwrap();
        Self {
            _temp: temp,
            paths,
            manifest,
            member_logical_id,
            member_b_logical_id,
            member_path,
        }
    }

    fn story_session(
        &self,
        involved: Vec<LogicalRepositoryId>,
        focus: Option<LogicalRepositoryId>,
    ) -> crate::product::models::WorkspaceSessionRecord {
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(self.paths.clone());
        let story = lifecycle
            .create_story_spec(crate::product::lifecycle_store::CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: String::new(),
                title: "lc draft story".to_string(),
                aggregate_codebase: Some(
                    crate::product::lifecycle_store::AggregateStorySpecScope {
                        logical_codebase_ref: self.manifest.logical_codebase_id,
                        effective_member_ids: self.manifest.member_ids.clone(),
                        involved_repository_ids: involved,
                        focus_repository_id: focus,
                    },
                ),
            })
            .unwrap();
        lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    entity_id: story.id,
                    workspace_type: crate::product::models::WorkspaceType::Story,
                    author_provider: crate::product::models::ProviderName::ClaudeCode,
                    reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: None,
                },
            )
            .unwrap()
    }

    fn design_session(
        &self,
        involved: Vec<LogicalRepositoryId>,
    ) -> crate::product::models::WorkspaceSessionRecord {
        self.design_session_for("issue_0001", involved)
    }

    /// r62(codex design resume 现场):多仓 Design 的 change_order
    /// 参数化变体(REQ-PLN-05:involved>1 必须 change_order)。
    fn design_session_with_change_order(
        &self,
        involved: Vec<LogicalRepositoryId>,
        change_order: Vec<LogicalRepositoryId>,
    ) -> crate::product::models::WorkspaceSessionRecord {
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(self.paths.clone());
        let design = lifecycle
            .create_design_spec(crate::product::lifecycle_store::CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: Vec::new(),
                title: "lc multi-repo design".to_string(),
                aggregate_codebase: Some(
                    crate::product::lifecycle_store::AggregateDesignSpecScope {
                        logical_codebase_ref: self.manifest.logical_codebase_id,
                        effective_member_ids: self.manifest.member_ids.clone(),
                        involved_repository_ids: involved,
                        change_order,
                    },
                ),
            })
            .unwrap();
        lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    entity_id: design.id,
                    workspace_type: crate::product::models::WorkspaceType::Design,
                    author_provider: crate::product::models::ProviderName::ClaudeCode,
                    reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: None,
                },
            )
            .unwrap()
    }

    fn design_session_for(
        &self,
        issue_id: &str,
        involved: Vec<LogicalRepositoryId>,
    ) -> crate::product::models::WorkspaceSessionRecord {
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(self.paths.clone());
        let design = lifecycle
            .create_design_spec(crate::product::lifecycle_store::CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: issue_id.to_string(),
                story_spec_ids: Vec::new(),
                title: "lc draft design".to_string(),
                aggregate_codebase: Some(
                    crate::product::lifecycle_store::AggregateDesignSpecScope {
                        logical_codebase_ref: self.manifest.logical_codebase_id,
                        effective_member_ids: self.manifest.member_ids.clone(),
                        involved_repository_ids: involved,
                        change_order: Vec::new(),
                    },
                ),
            })
            .unwrap();
        lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: "project_0001".to_string(),
                    issue_id: issue_id.to_string(),
                    entity_id: design.id,
                    workspace_type: crate::product::models::WorkspaceType::Design,
                    author_provider: crate::product::models::ProviderName::ClaudeCode,
                    reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: None,
                },
            )
            .unwrap()
    }

    fn design_session_for_lc(
        &self,
        issue_id: &str,
        involved: Vec<LogicalRepositoryId>,
        effective_member_ids: Vec<LogicalRepositoryId>,
    ) -> crate::product::models::WorkspaceSessionRecord {
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(self.paths.clone());
        let design = lifecycle
            .create_design_spec(crate::product::lifecycle_store::CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: issue_id.to_string(),
                story_spec_ids: Vec::new(),
                title: "lc draft design".to_string(),
                aggregate_codebase: Some(
                    crate::product::lifecycle_store::AggregateDesignSpecScope {
                        logical_codebase_ref: self.manifest.logical_codebase_id,
                        effective_member_ids,
                        involved_repository_ids: involved,
                        change_order: Vec::new(),
                    },
                ),
            })
            .unwrap();
        lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: "project_0001".to_string(),
                    issue_id: issue_id.to_string(),
                    entity_id: design.id,
                    workspace_type: crate::product::models::WorkspaceType::Design,
                    author_provider: crate::product::models::ProviderName::ClaudeCode,
                    reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: None,
                },
            )
            .unwrap()
    }

    /// S6（2026-10-10 E2E）红→绿夹具：多仓 LC 下的 WorkItemPlan 会话
    /// （all_members selection → 双成员 target，无 confirmed design 过滤）。
    fn plan_session(&self) -> crate::product::models::WorkspaceSessionRecord {
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(self.paths.clone());
        let plan = lifecycle
            .create_issue_work_item_plan(
                crate::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                    id: None,
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    source_story_spec_ids: Vec::new(),
                    source_design_spec_ids: Vec::new(),
                    options: crate::product::models::IssueWorkItemPlanOptions {
                        include_integration_tests: false,
                        include_e2e_tests: false,
                        force_frontend_backend_split: false,
                        require_execution_plan_confirm: false,
                    },
                    status: crate::product::models::IssueWorkItemPlanStatus::Draft,
                    work_item_ids: Vec::new(),
                    repository_profile_ref: None,
                    verification_plan_ids: Vec::new(),
                    dependency_graph: Vec::new(),
                    created_from_provider_run: None,
                    validator_findings: Vec::new(),
                },
            )
            .unwrap();
        lifecycle
            .create_workspace_session(
                crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    entity_id: plan.id,
                    workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                    author_provider: crate::product::models::ProviderName::ClaudeCode,
                    reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: None,
                },
            )
            .unwrap()
    }
}

/// 红→绿主锁：草稿 LC Story（focus=None ∧ involved 空）attach 期解析不再
/// TargetMissing，锚定聚合根视图（path=provider_context_root、无 checkout、
/// logical 身份在场供 gateway 注入谓词）。
#[test]
fn draft_lc_story_anchors_at_aggregate_root_until_focus_determined() {
    let fixture = LcStoryRoutingFixture::new();
    let session = fixture.story_session(Vec::new(), None);
    let repository = workspace_repository_for_session(
        &fixture.paths,
        &LifecycleStore::new(fixture.paths.clone()),
        &session,
    )
    .expect("draft LC story must anchor at the aggregate root view");
    assert_eq!(repository.path, fixture.manifest.provider_context_root);
    assert!(repository.logical_repository_id.is_some());
    assert!(repository.primary_checkout_id.is_none());
}

/// 边界 1：focus 已定（回写后）→ 恢复既有成员解析，路径=成员 canonical
/// checkout、logical 身份=成员。
#[test]
fn lc_story_with_focus_resolves_member_unchanged() {
    let fixture = LcStoryRoutingFixture::new();
    let session = fixture.story_session(
        vec![fixture.member_logical_id],
        Some(fixture.member_logical_id),
    );
    let repository = workspace_repository_for_session(
        &fixture.paths,
        &LifecycleStore::new(fixture.paths.clone()),
        &session,
    )
    .expect("focused LC story resolves the member checkout");
    assert_eq!(repository.path, fixture.member_path);
    assert_eq!(
        repository.logical_repository_id,
        Some(fixture.member_logical_id)
    );
}

/// 边界 2（story 焦点仓路由修复,2026-10-11,story_spec_0001 现场）:focus=None 但
/// involved 非空——AI 自决流合法可达态（structured output schema 允许 focus=null,
/// 多仓均衡纵切无单一焦点）→ 锚定聚合根视图,不再 TargetMissing fail-closed。
///
/// 与 plan S6（多 target + focus 非唯一回落 AggregateRoot,5a217fa8）同构裁决:
/// ①首轮草稿态锚=aggregate_root,本态回落聚合根 → 锚连续 → 指纹可比对 →
/// 原生恢复可达（r23 根修目标）;②cwd 恒 canonical root（ScoutRootVsMember）,
/// author 视野零变化;③story 草稿态（缺陷#2）/design 草稿态（缺陷#5）/
/// design change_order 首仓（r62）家族先例同款。AI 显式给出 focus 时
/// 仍走上方成员锚臂（r23 语义零变化）。
#[test]
fn lc_story_without_focus_but_involved_anchors_aggregate_root() {
    let fixture = LcStoryRoutingFixture::new();
    let session = fixture.story_session(vec![fixture.member_logical_id], None);
    let repository = workspace_repository_for_session(
        &fixture.paths,
        &LifecycleStore::new(fixture.paths.clone()),
        &session,
    )
    .expect("involved-but-unfocused LC story must anchor at the aggregate root view");
    assert_eq!(repository.path, fixture.manifest.provider_context_root);
    assert!(repository.logical_repository_id.is_some());
    assert!(repository.primary_checkout_id.is_none());
}

/// S6（2026-10-10 全旅程 E2E）红→绿主锁：多仓 plan 会话（LC 双成员，
/// selection 恒 all_members/无唯一 focus）attach 期解析不再
/// TargetAmbiguous——方案 B：plan 会话=orchestrator 角色（durable system
/// 消息 adapter_role=orchestrator），锚定聚合根视图（与缺陷 #2 story
/// 草稿态 / 缺陷 #5 design 草稿态同款口径；S6 现场 WS create 即在本臂
/// 失败，前端停留「正在连接工作区…」60s 重连循环）。
#[test]
fn lc_plan_session_multi_target_anchors_aggregate_root_view() {
    let fixture = LcStoryRoutingFixture::new();
    let session = fixture.plan_session();
    let repository = workspace_repository_for_session(
        &fixture.paths,
        &LifecycleStore::new(fixture.paths.clone()),
        &session,
    )
    .expect("multi-repo plan session must anchor at the aggregate root view");
    assert_eq!(repository.path, fixture.manifest.provider_context_root);
    assert!(repository.logical_repository_id.is_some());
    assert!(repository.primary_checkout_id.is_none());
}

/// 缺陷 #5（2026-10-02 全链 E2E）红→绿主锁：草稿 LC Design（involved 空）
/// attach 期解析不再 TargetMissing，锚定聚合根视图（同缺陷 #2 Story 口径）。
#[test]
fn draft_lc_design_anchors_at_aggregate_root_until_involved_determined() {
    let fixture = LcStoryRoutingFixture::new();
    let session = fixture.design_session(Vec::new());
    let repository = workspace_repository_for_session(
        &fixture.paths,
        &LifecycleStore::new(fixture.paths.clone()),
        &session,
    )
    .expect("draft LC design must anchor at the aggregate root view");
    assert_eq!(repository.path, fixture.manifest.provider_context_root);
    assert!(repository.logical_repository_id.is_some());
    assert!(repository.primary_checkout_id.is_none());
}

/// 边界：involved 唯一（回写后）→ 恢复成员解析；≥2 保持 TargetAmbiguous。
#[test]
fn lc_design_with_single_involved_resolves_member_unchanged() {
    let fixture = LcStoryRoutingFixture::new();
    let session = fixture.design_session(vec![fixture.member_logical_id]);
    let repository = workspace_repository_for_session(
        &fixture.paths,
        &LifecycleStore::new(fixture.paths.clone()),
        &session,
    )
    .expect("single-involved LC design resolves the member checkout");
    assert_eq!(repository.path, fixture.member_path);
    assert_eq!(
        repository.logical_repository_id,
        Some(fixture.member_logical_id)
    );
}

#[test]
fn lc_design_with_multiple_involved_stays_target_ambiguous() {
    let fixture = LcStoryRoutingFixture::new();
    let session =
        fixture.design_session(vec![fixture.member_logical_id, fixture.member_b_logical_id]);
    let error = workspace_repository_for_session(
        &fixture.paths,
        &LifecycleStore::new(fixture.paths.clone()),
        &session,
    )
    .unwrap_err();
    assert_eq!(stable_routing_code(&error), "repository_routing_ambiguous");
}

/// r62(codex-6 design resume 现场)红→绿主锁:确认面合法化的多仓 Design
/// (REQ-PLN-05 决策 3b:involved>1 + change_order 通过确认 gate 落盘)在
/// 会话路由面不再 TargetAmbiguous——修订轮按 change_order 首成员确定性
/// 锚定(与 plan 会话 selection.focus 回落先例 REQ-COD-04 对称;现场:
/// design_spec_0001 involved=[alpha,beta] change_order=[alpha,beta],
/// 修订轮 resolve 直接拒绝)。
#[test]
fn lc_design_with_multiple_involved_and_change_order_anchors_first_member() {
    let fixture = LcStoryRoutingFixture::new();
    let session = fixture.design_session_with_change_order(
        vec![fixture.member_logical_id, fixture.member_b_logical_id],
        vec![fixture.member_logical_id, fixture.member_b_logical_id],
    );
    let repository = workspace_repository_for_session(
        &fixture.paths,
        &LifecycleStore::new(fixture.paths.clone()),
        &session,
    )
    .expect("multi-repo design with change order anchors the first member");
    assert_eq!(repository.path, fixture.member_path);
    assert_eq!(
        repository.logical_repository_id,
        Some(fixture.member_logical_id)
    );
}

/// 缺陷 #6（2026-10-02 全链 E2E）红→绿主锁：per-LC（v1.3）布局——成员/
/// checkouts/selection 全在 logical-codebases/{lc}/ 子树、无 legacy
/// repos.json 投影——involved 回写后的成员解析必须经 for_lc authority
/// 合成 RepositoryRecord（path=成员 canonical checkout）；修复前直连
/// strict 解析恒 IdentityMismatch（design_spec_0001 现场）。
#[test]
fn per_lc_issue_member_resolution_synthesizes_record_from_lc_authority() {
    let fixture = LcStoryRoutingFixture::new();
    // 显式 LC record + per-LC 子树权威数据（不复用 legacy manifest）。
    let record = LogicalCodebaseStore::new(fixture.paths.clone())
        .create(
            "project_0001",
            crate::product::logical_codebase::LogicalCodebaseCreateInput {
                name: "per-lc".to_string(),
                aggregate_root: fixture.manifest.provider_context_root.clone(),
            },
        )
        .unwrap();
    let authority = LogicalCodebaseStore::for_lc(fixture.paths.clone(), record.id.clone());
    // 隔离成员 member-c：仅存在于 per-LC 子树 + identity registry，
    // 不写 legacy manifest/repos.json 投影（真实 E2E 形态）。
    let member_c_path = fixture.manifest.provider_context_root.join("member-c");
    std::fs::create_dir_all(&member_c_path).unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "user.email", "per-lc@example.test"],
        vec!["config", "user.name", "Per LC Fixture"],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(&member_c_path)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }
    std::fs::write(member_c_path.join("README.md"), "# member-c\n").unwrap();
    for args in [
        vec!["add", "README.md"],
        vec!["commit", "--quiet", "-m", "init"],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(&member_c_path)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }
    let member_c = LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = crate::product::logical_codebase::RepositoryCheckoutId(uuid::Uuid::new_v4());
    let source_identity =
        crate::product::logical_codebase::RepositorySourceIdentity::from_git_parts(
            &member_c_path,
            member_c_path.join(".git"),
            None,
        );
    let manifest = LogicalCodebaseManifest::new(
        "project_0001",
        fixture.manifest.provider_context_root.clone(),
        vec![member_c],
    );
    authority.save_manifest("project_0001", &manifest).unwrap();
    authority
        .save_checkout(
            "project_0001",
            &crate::product::logical_codebase::RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_c,
                physical_repository_id: "repository_per_lc_member_c".to_string(),
                kind: crate::product::logical_codebase::CheckoutKind::Main,
                canonical_path: member_c_path.clone(),
                checkout_path_hash: String::new(),
                git_dir_identity: String::new(),
                revision: None,
                availability: crate::product::logical_codebase::CheckoutAvailability::Available,
                observed_at: "2026-10-02T00:00:00Z".to_string(),
                created_at: "2026-10-02T00:00:00Z".to_string(),
                updated_at: "2026-10-02T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    authority
        .save_member(
            "project_0001",
            &crate::product::logical_codebase::CodebaseMemberRecord {
                logical_repository_id: member_c,
                physical_repository_id: "repository_per_lc_member_c".to_string(),
                alias: "member-c".to_string(),
                role: "repository".to_string(),
                ordinal: 0,
                source_identity: source_identity.clone(),
                repo_type: crate::product::logical_codebase::RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: crate::product::logical_codebase::MemberStatus::Active,
                created_at: "2026-10-02T00:00:00Z".to_string(),
                updated_at: "2026-10-02T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    crate::product::logical_codebase::IdentityRegistryStore::new(fixture.paths.clone())
        .upsert_active(
            "project_0001",
            crate::product::logical_codebase::IdentityRegistryEntry {
                source_identity: source_identity.clone(),
                logical_repository_id: member_c,
                physical_repository_id: "repository_per_lc_member_c".to_string(),
                primary_checkout_id: checkout_id,
                state: crate::product::logical_codebase::IdentityRegistryState::Active,
                created_by_key: "test:per-lc-member-c".to_string(),
                deleted_at: None,
                delete_operation_id: None,
                reactivated_at: None,
                // Bug3 清单外单行补丁：IdentityRegistryEntry 新增 released_at
                //（serde default 兼容旧数据）后的编译强制补齐。
                released_at: None,
            },
        )
        .unwrap();
    // issue 持久化 lc 归属 + per-LC selection。
    let issue = crate::product::issue_store::IssueStore::new(fixture.paths.clone())
        .create(crate::product::issue_store::CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: None,
            logical_codebase_id: record.id.clone(),
            title: "per-lc issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
    crate::product::logical_codebase::IssueCodebaseSelectionStore::for_lc(
        fixture.paths.clone(),
        record.id.clone(),
    )
    .save(&IssueCodebaseSelection::all_members(
        "project_0001",
        &issue.id,
        Some(record.id.clone()),
    ))
    .unwrap();

    let session = fixture.design_session_for_lc(&issue.id, vec![member_c], vec![member_c]);
    let repository = workspace_repository_for_session(
        &fixture.paths,
        &LifecycleStore::new(fixture.paths.clone()),
        &session,
    )
    .expect("per-LC design must resolve the member from LC authority");
    assert_eq!(repository.path, member_c_path);
    assert_eq!(repository.logical_repository_id, Some(member_c));
    assert_eq!(repository.primary_checkout_id, Some(checkout_id));
}

/// E 包（fix-planning-session-anchor-recovery REQ-PLN-08）红→绿主锁：
/// 三类 LC 规划会话（Story/Design/WorkItemPlan，PlanningReadOnly）的 native
/// launch anchor 恒聚合根（path=canonical root、primary_checkout_id=None），
/// scope 回写（focus/involved）不迁移锚；聚合视图逻辑身份不与成员混同；
/// 单仓形态（单成员 LC）与 WorkItem/Coding 的 native 与业务同锚同错。
///
/// ⚠️ REQ-ROUTE-01 两态化后「无 LC manifest 的单仓项目」不再是可路由形态
/// （(manifest, selection) 不成对 ⇒ FailClosed，(None,None) 亦不回退物理仓），
/// 故单仓场景改由「单成员 LC」表达——见本文件 direct 块的说明。
#[test]
fn anchor_t01_native_anchor_stays_root_after_scope_writeback() {
    let fixture = LcStoryRoutingFixture::new();
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone());
    let canonical_root = fixture.manifest.provider_context_root.clone();

    // 三类规划会话 native anchor 恒聚合根（两 checkout/physical 身份为空）。
    let story_session = fixture.story_session(Vec::new(), None);
    let design_session = fixture.design_session(Vec::new());
    let plan_session = fixture.plan_session();
    for session in [&story_session, &design_session, &plan_session] {
        let native = workspace_native_launch_anchor_for_session(
            &fixture.paths,
            &lifecycle,
            session,
        )
        .expect("LC planning native launch anchor must stay at the aggregate root");
        assert_eq!(native.path, canonical_root);
        assert!(native.primary_checkout_id.is_none());
        // 聚合视图逻辑身份=LC 身份，不与任何成员逻辑身份混同（更不等于成员物理 ID）。
        assert_ne!(
            native.logical_repository_id,
            Some(fixture.member_logical_id)
        );
        assert_ne!(
            native.logical_repository_id,
            Some(fixture.member_b_logical_id)
        );
    }

    // scope 回写（story focus: None→member）前后 native anchor 逐字段不变；
    // 同一会话业务面恢复成员解析（锚迁移只允许发生在 business 输出）。
    let before_scope = workspace_native_launch_anchor_for_session(
        &fixture.paths,
        &lifecycle,
        &story_session,
    )
    .expect("native anchor before scope writeback");
    lifecycle
        .update_story_spec_aggregate(
            "project_0001",
            "issue_0001",
            &story_session.entity_id,
            &crate::product::lifecycle_store::AggregateStorySpecScope {
                logical_codebase_ref: fixture.manifest.logical_codebase_id,
                effective_member_ids: fixture.manifest.member_ids.clone(),
                involved_repository_ids: vec![fixture.member_logical_id],
                focus_repository_id: Some(fixture.member_logical_id),
            },
        )
        .unwrap();
    let after_scope = workspace_native_launch_anchor_for_session(
        &fixture.paths,
        &lifecycle,
        &story_session,
    )
    .expect("native anchor after scope writeback");
    assert_eq!(before_scope, after_scope);
    let business = workspace_repository_for_session(&fixture.paths, &lifecycle, &story_session)
        .expect("focused story business repository resolves the member");
    assert_eq!(business.path, fixture.member_path);

    // direct 形态（单仓项目）：native 与业务同锚=原目录。
    //
    // ⚠️ REQ-ROUTE-01 两态化后「无 LC manifest 的单仓项目」不再是可路由形态：
    // (manifest, selection) 不成对 ⇒ FailClosed（(None,None) 也不再回退物理
    // 仓）。故本块把同一不变量落到当前唯一合法的单仓形态上——**单成员 LC、
    // 聚合根=原目录**：native 与业务仍同锚于该原目录（聚合根视图），断言逐
    // 字不变。夹具用共享夹具 seed_complete 建权威（issue 先建、selection
    // 后写由其内部保证）。
    let legacy_root = fixture._temp.path().join("legacy-root");
    let legacy_dir = legacy_root.join("aggregate-root");
    crate::product::logical_codebase::logical_test_fixture::seed_complete(
        &fixture.paths,
        &legacy_root,
        "project_0002",
        "issue_0001",
    );
    let legacy_story = lifecycle
        .create_story_spec(crate::product::lifecycle_store::CreateStorySpecInput {
            project_id: "project_0002".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: crate::product::logical_codebase::logical_test_fixture::FIXTURE_PHYSICAL_REPOSITORY_ID
                .to_string(),
            title: "legacy direct story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    let legacy_session = lifecycle
        .create_workspace_session(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0002".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: legacy_story.id,
                workspace_type: crate::product::models::WorkspaceType::Story,
                author_provider: crate::product::models::ProviderName::ClaudeCode,
                reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
        )
        .unwrap();
    let legacy_native = workspace_native_launch_anchor_for_session(
        &fixture.paths,
        &lifecycle,
        &legacy_session,
    )
    .expect("legacy direct native anchor keeps the original directory");
    let legacy_business = workspace_repository_for_session(
        &fixture.paths,
        &lifecycle,
        &legacy_session,
    )
    .expect("legacy direct business repository keeps the original directory");
    assert_eq!(legacy_native, legacy_business);
    assert_eq!(legacy_native.path, legacy_dir);

    // WorkItem/Coding：无 runtime binding 的会话两 getter 一致 fail-closed
    // （native 不引入任何分流/旁路；成功路径同理同锚=原 member）。
    let work_item_session = lifecycle
        .create_workspace_session(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "work_item_missing_runtime_binding".to_string(),
                workspace_type: crate::product::models::WorkspaceType::WorkItem,
                author_provider: crate::product::models::ProviderName::ClaudeCode,
                reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
        )
        .unwrap();
    let work_item_native_error = workspace_native_launch_anchor_for_session(
        &fixture.paths,
        &lifecycle,
        &work_item_session,
    )
    .unwrap_err();
    let work_item_business_error = workspace_repository_for_session(
        &fixture.paths,
        &lifecycle,
        &work_item_session,
    )
    .unwrap_err();
    assert_eq!(
        format!("{work_item_native_error:?}"),
        format!("{work_item_business_error:?}"),
        "work item native anchor must fail identically to business resolution"
    );
}

/// E 包边界锁：business_repository 逐项保留现行 `workspace_repository_for_session`
/// 语义（Story focus 成员／Design 首 change_order 成员／Plan 多 target 无唯一
/// focus 的聚合视图材料行为）；selection 失效与 focus 越界共同守卫两用途一致
/// 拒收（native anchor 不得绕过）；聚合视图 ID 不等于成员物理 ID。
#[test]
fn anchor_t01_business_target_and_selection_guards_unchanged() {
    let fixture = LcStoryRoutingFixture::new();
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone());

    // Story focus=成员：business=成员（r23 原断言），native=聚合根，两身份不混同。
    let story_session = fixture.story_session(
        vec![fixture.member_logical_id],
        Some(fixture.member_logical_id),
    );
    let original_member =
        workspace_repository_for_session(&fixture.paths, &lifecycle, &story_session)
            .expect("focused story keeps member business repository");
    let context = resolve_workspace_repository_context(
        &fixture.paths,
        &lifecycle,
        &story_session,
    )
    .expect("context resolves for focused story");
    let business = context
        .business_repository
        .expect("focused story business output is the member");
    let native = context
        .native_launch_anchor
        .expect("focused story native output is the aggregate root");
    assert_eq!(business.id, original_member.id);
    assert_eq!(business.path, original_member.path);
    assert_eq!(business.path, fixture.member_path);
    assert_eq!(
        business.logical_repository_id,
        Some(fixture.member_logical_id)
    );
    assert_ne!(native.id, business.id);
    assert_eq!(native.path, fixture.manifest.provider_context_root);
    assert!(native.primary_checkout_id.is_none());

    // Design ≥2 involved + change_order：business=首成员（r62 原断言），native=聚合根。
    let design_session = fixture.design_session_with_change_order(
        vec![fixture.member_logical_id, fixture.member_b_logical_id],
        vec![fixture.member_logical_id, fixture.member_b_logical_id],
    );
    let context = resolve_workspace_repository_context(
        &fixture.paths,
        &lifecycle,
        &design_session,
    )
    .expect("context resolves for multi-repo design");
    let business = context
        .business_repository
        .expect("multi-repo design business output is the first change_order member");
    assert_eq!(business.path, fixture.member_path);
    assert_eq!(
        business.logical_repository_id,
        Some(fixture.member_logical_id)
    );
    let native = context
        .native_launch_anchor
        .expect("multi-repo design native output is the aggregate root");
    assert_eq!(native.path, fixture.manifest.provider_context_root);
    assert_ne!(native.id, business.id);

    // Plan 多 target 无唯一 focus：business=聚合视图（S6 原材料行为保留），
    // native=聚合根。
    let plan_session = fixture.plan_session();
    let context =
        resolve_workspace_repository_context(&fixture.paths, &lifecycle, &plan_session)
            .expect("context resolves for multi-target plan");
    let business = context
        .business_repository
        .expect("multi-target plan business output stays the aggregate view");
    assert_eq!(business.path, fixture.manifest.provider_context_root);
    assert!(business.primary_checkout_id.is_none());
    let native = context
        .native_launch_anchor
        .expect("multi-target plan native output is the aggregate root");
    assert_eq!(native.path, fixture.manifest.provider_context_root);
    assert!(native.primary_checkout_id.is_none());

    // focus 越界（focus ⊄ include 的持久化脏态，绕过 store save 校验直接落盘，
    // 模拟旧版本/外部写入）：两输出一致拒收（现行守卫不变，native 不绕过）。
    let out_of_bounds = LogicalRepositoryId(uuid::Uuid::new_v4());
    crate::product::json_store::write_json(
        &fixture
            .paths
            .codebase_selection_path("project_0001", "issue_0001"),
        &IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![fixture.member_logical_id],
            Vec::new(),
            vec![out_of_bounds],
            None,
        ),
    )
    .unwrap();
    let focus_overflow_native = workspace_native_launch_anchor_for_session(
        &fixture.paths,
        &lifecycle,
        &story_session,
    );
    let focus_overflow_business = workspace_repository_for_session(
        &fixture.paths,
        &lifecycle,
        &story_session,
    );
    assert!(focus_overflow_native.is_err());
    assert!(focus_overflow_business.is_err());

    // 恢复合法 selection 后标记失效（成员删除/停用）：两输出仍一致拒收
    // （SelectionInvalidated；native anchor 不得绕过共同守卫）。
    let selection_store =
        crate::product::logical_codebase::IssueCodebaseSelectionStore::new(fixture.paths.clone());
    selection_store
        .save(&IssueCodebaseSelection::all_members(
            "project_0001",
            "issue_0001",
            None,
        ))
        .unwrap();
    selection_store
        .mark_invalidated("project_0001", "issue_0001", "anchor_t01 member removed")
        .unwrap();
    let invalid_selection_result = workspace_native_launch_anchor_for_session(
        &fixture.paths,
        &lifecycle,
        &story_session,
    );
    assert!(invalid_selection_result.is_err());
    let invalid_business = workspace_repository_for_session(
        &fixture.paths,
        &lifecycle,
        &story_session,
    );
    assert!(invalid_business.is_err());
    assert_eq!(
        stable_routing_code(invalid_selection_result.as_ref().unwrap_err()),
        "repository_routing_inconsistent"
    );
}
