// -------------------------------------------------------------------
// Task 3.2（D2/D3，BOOT-03）：生产接线的 root recipe 安全回归锁。
//
// 与 `trust_route_fixture` 同形的生产依赖图（gateway factory 驱动携带
// `Some(paths)`：admission 预检 + per-LC root recipe 命令审计 + Task 4
// 末命令发布收口），但聚合根内含真实成员 Git 仓，供两类断言消费：
// - D2 `root_receipt_rejects_member_git_and_unknown_changes`：provider 在
//   pre_check 命令窗口内写成员 `.git` 与根级未知路径 → 命令 receipt 拒绝
//   （证据 durable 保留）；Task 4 起被拒命令使最终 receipt 无法签发，
//   末命令收口失败传播（operation Failed，readiness 以 provider-turn
//   失败面呈现），且 auditor 只读不回滚用户/成员字节。
// - D3 `shared_executor_cannot_reach_repository_registration_or_git_finalize`：
//   共享四命令 executor（root recipe 命令源 + gateway 驱动）完整跑通五步
//   operation 的同时，成员 Git 状态、根清单变化=精确 recipe 自有产物 +
//   发布 locator、单仓 registration/GitFinalize 持久化指纹全部零变化
//   ——以可观测调用图边界（文件系统、进程审计、store 记录）证明不可达
//   单仓调用图，替代源码 token 扫描。
// -------------------------------------------------------------------

/// 聚合根内真实成员 Git 仓的不可变指纹。`git add -A` + `git commit`
/// （GitFinalize 的调用图指纹）会改变 head_revision 或 porcelain。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RootSafetyMemberFingerprint {
    head_revision: String,
    porcelain: String,
    tracked_files: Vec<String>,
}

fn root_safety_member_fingerprint(repo: &std::path::Path) -> RootSafetyMemberFingerprint {
    RootSafetyMemberFingerprint {
        head_revision: root_safety_git(repo, &["rev-parse", "HEAD"])
            .trim()
            .to_string(),
        porcelain: root_safety_git(repo, &["status", "--porcelain"]),
        tracked_files: root_safety_git(repo, &["ls-files"])
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
    }
}

fn root_safety_git(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-c")
        .arg("user.name=Aria Root Safety Test")
        .arg("-c")
        .arg("user.email=root-safety@example.com")
        .arg("-c")
        .arg("commit.gpgsign=false")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("spawn git for root safety fixture");
    assert!(
        output.status.success(),
        "git {args:?} in {} failed: {}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// canonical 聚合根的递归文件清单（相对路径 → 文件内容 digest / symlink
/// 目标 / 目录标记）。任何根级写入（未知路径、git_finalize 于根产生的
/// `.git` 树变化、allowlist 外 artifact）都会使前后清单不等。
fn aggregate_root_inventory(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    fn walk(
        inventory: &mut std::collections::BTreeMap<String, String>,
        root: &std::path::Path,
        dir: &std::path::Path,
    ) {
        let mut entries = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
            .collect::<Result<Vec<_>, _>>()
            .expect("inventory entries");
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("relative to root");
            let key = relative.to_string_lossy().replace('\\', "/");
            let metadata = std::fs::symlink_metadata(&path).expect("inventory metadata");
            if metadata.file_type().is_symlink() {
                let target = std::fs::read_link(&path).expect("link target");
                inventory.insert(key, format!("symlink:{}", target.to_string_lossy()));
            } else if metadata.is_dir() {
                inventory.insert(key, "dir".to_string());
                walk(inventory, root, &path);
            } else {
                let bytes = std::fs::read(&path).expect("inventory file bytes");
                let digest = {
                    use sha2::Digest;
                    let mut hasher = sha2::Sha256::new();
                    hasher.update(&bytes);
                    format!("sha256:{:x}", hasher.finalize())
                };
                inventory.insert(key, digest);
            }
        }
    }
    let mut inventory = std::collections::BTreeMap::new();
    walk(&mut inventory, root, root);
    inventory
}

/// D2 探针：首个 pre_check turn（命令 1 审计窗口内）向成员 `.git/HEAD`
/// 写入篡改字节、向根级写一个 allowlist 外未知文件——模拟 provider 越界
/// 写；其余命令行为与真实 recipe 同构（RuleAndMcpConfig 时机生成根规则
/// 材料，Task 4 生产同构），随后委托 recipe provider 正常完成会话
/// （turn 本身成功，receipt 层拒绝）。
struct RogueRootWriteStreamingProvider {
    fired: std::sync::atomic::AtomicBool,
    member_git_head: PathBuf,
    rogue_path: PathBuf,
    recipe: RootPolicyRecipeStreamingProvider,
}

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
    for RogueRootWriteStreamingProvider
{
    async fn start(
        &self,
        input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        let pre_check =
            crate::product::repository_store::RepositoryInitializationStepKind::PreCheck
                .command()
                .expect("pre_check command");
        if input.prompt.contains(pre_check)
            && !self.fired.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            std::fs::write(&self.member_git_head, "ref: refs/heads/rogue-mutation\n")
                .expect("simulated member git mutation");
            std::fs::write(&self.rogue_path, "out of root recipe write\n")
                .expect("simulated unknown root path write");
        }
        self.recipe.start(input, cancel).await
    }
}

struct RootSafetyFixture {
    app: axum::Router,
    lc_id: String,
    paths: ProductAppPaths,
    aggregate_root: std::path::PathBuf,
    member_repo: std::path::PathBuf,
    factory: Arc<LogicalCodebaseGatewayFactory>,
    /// 路由 trust 门；当前无读取点，保留 fixture 形状。
    #[allow(dead_code)]
    gate: Arc<RouteTrustGate>,
    /// 生产形依赖图；当前无读取点，保留 fixture 形状。
    #[allow(dead_code)]
    dependencies: AggregateInitializationDependencies,
}

/// 生产形依赖图 fixture：聚合根（非 Git）内含真实成员 Git 仓
/// `member-a`（一次初始提交，工作区干净）。`rogue=true` 时 provider 换为
/// D2 越界写探针，否则为普通 `FakeStreamingProvider`（D3 干净基线）。
fn root_safety_fixture(rogue: bool) -> RootSafetyFixture {
    let root = tempdir().expect("root");
    let root_path = root.path().to_path_buf();
    let paths = ProductAppPaths::new(root_path.join(".aria"));
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "Root safety wiring test".to_string(),
            description: None,
        })
        .unwrap();
    let aggregate_root = root_path.join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).unwrap();
    let member_repo = aggregate_root.join("member-a");
    std::fs::create_dir_all(&member_repo).unwrap();
    root_safety_git(&member_repo, &["init", "--initial-branch=main"]);
    std::fs::write(member_repo.join("README.md"), "# member a\n").unwrap();
    root_safety_git(&member_repo, &["add", "README.md"]);
    root_safety_git(&member_repo, &["commit", "-m", "member a baseline"]);

    let lc_id = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
        .create(
            "project_0001",
            crate::product::logical_codebase::LogicalCodebaseCreateInput {
                name: "root-safety".to_string(),
                aggregate_root: aggregate_root.clone(),
            },
        )
        .unwrap()
        .id;
    crate::product::logical_codebase::LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone())
        .save_manifest(
            "project_0001",
            &crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
                "project_0001",
                aggregate_root.clone(),
                Vec::new(),
            ),
        )
        .unwrap();

    let streaming: Arc<dyn crate::cross_cutting::streaming_provider::StreamingProviderAdapter> =
        if rogue {
            Arc::new(RogueRootWriteStreamingProvider {
                fired: std::sync::atomic::AtomicBool::new(false),
                member_git_head: member_repo.join(".git").join("HEAD"),
                rogue_path: aggregate_root.join("rogue.txt"),
                recipe: RootPolicyRecipeStreamingProvider::new(RecipeFixtureOptions::default()),
            })
        } else {
            // Task 4：干净基线同样由 provider 在 RuleAndMcpConfig 时机生成
            // 根规则材料（生产同构），使末命令发布收口有真实材料。
            Arc::new(RootPolicyRecipeStreamingProvider::new(
                RecipeFixtureOptions::default(),
            ))
        };
    let factory = fake_registry_gateway_factory_with(streaming, paths.clone());
    let gate = Arc::new(RouteTrustGate::new(false));
    let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(
        GatewayFactoryProviderTurnDriver::new(Some(factory.clone()), Some(paths.clone())),
    );
    let clock: Arc<dyn Fn() -> String + Send + Sync> =
        Arc::new(|| "2026-10-02T00:00:00Z".to_string());
    let coordinator = AggregateInitializationCoordinator::new(
        paths.clone(),
        AggregateInitializationOperationStore::new(paths.clone()),
        Arc::new(RouteTestSkills),
        Arc::new(DeterministicAggregatePreflightService::new(paths.clone())),
        provider,
        clock,
    )
    .with_trust(gate.clone());
    // Task 4：index op 指向本 fixture 的 LC scope 并以必缺 codegraph 二进制
    // 确定性失败——不与并行测试共享 temp 根，也不让真实 codegraph 在
    // 聚合根写 `.codegraph/**` 破坏根清单对照。
    let index = Arc::new(AggregateIndexOperation::new(
        paths.clone(),
        CodeGraphCli::new(
            Arc::new(TokioBoundedCommandRunner),
            "codegraph-missing-binary-for-root-policy-tests".to_string(),
        ),
        CodeGraphExcludeGenerator,
    ));
    let dependencies = AggregateInitializationDependencies::with_index(
        Arc::new(coordinator),
        InitializationRunRegistry::default(),
        index,
    )
    .with_trust(gate.clone());
    let state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(root_path.clone()))
        .with_aggregate_initialization_dependencies(dependencies.clone());
    std::mem::forget(root);
    RootSafetyFixture {
        app: build_web_router(state),
        lc_id,
        paths,
        aggregate_root,
        member_repo,
        factory,
        gate,
        dependencies,
    }
}

/// POST 创建五步 operation 并轮询到终态（trust gate 初始即 Ready）。
async fn root_safety_run_recipe_to_terminal(
    fixture: &RootSafetyFixture,
    idempotency_key: &str,
) -> crate::product::logical_codebase::AggregateInitializationOperation {
    let uri = format!(
        "/api/projects/project_0001/logical-codebases/{}/initializations",
        fixture.lc_id
    );
    let (status, body) = response_json(
        post_json(
            &fixture.app,
            &uri,
            serde_json::json!({"idempotency_key": idempotency_key}),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let operation_id = body["operation_id"]
        .as_str()
        .expect("operation id")
        .to_string();

    let operations =
        AggregateInitializationOperationStore::for_lc(fixture.paths.clone(), fixture.lc_id.clone());
    loop {
        let operation = operations
            .get("project_0001", &operation_id)
            .expect("operation is durable");
        match operation.status {
            crate::product::logical_codebase::AggregateInitializationOperationStatus::Created
            | crate::product::logical_codebase::AggregateInitializationOperationStatus::Running => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            _ => return operation,
        }
    }
}

/// Task 3.2（D2/REQ-BOOT-03）：生产接线的 receipt 拒绝面。provider 在
/// pre_check 命令窗口内篡改成员 `.git/HEAD` 并写根级未知路径：
/// - 命令 receipt `Rejected`，变更证据按 MemberGit/UnknownPath 分类
///   durable 保留（允许记录，不允许静默）；
/// - 后续命令窗口无越界新写入 → `Allowed`（窗口按命令隔离；命令 2/3 的
///   规则材料生成属 allowlisted 变更）；
/// - Task 4（REQ-BOOT-05）起被拒命令使最终 receipt 无法签发 → 末命令
///   收口失败传播：operation Failed 于 OpenspecAndExamples、最终 receipt
///   缺席、readiness 以 provider-turn 失败面呈现（planning_ready=false）；
/// - auditor 只读：越界字节与成员 Git 篡改原样保留，绝不回滚用户文件。
#[tokio::test]
async fn root_receipt_rejects_member_git_and_unknown_changes() {
    let fixture = root_safety_fixture(true);
    let operation = root_safety_run_recipe_to_terminal(&fixture, "d2-reject-1").await;
    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed,
        "a rejected command must fail the final-command publication closure: {:?}",
        operation.error
    );
    assert_eq!(
        operation.failed_step,
        Some(
            crate::product::logical_codebase::AggregateInitializationStepKind::OpenspecAndExamples
        ),
        "the closure failure lands on the final command's step"
    );

    let receipts = crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        fixture.paths.clone(),
        fixture.lc_id.clone(),
    );
    let mut commands = receipts
        .list_commands("project_0001", &operation.operation_id)
        .expect("command receipts durable");
    commands.sort_by_key(|receipt| receipt.command_index);
    assert_eq!(commands.len(), 4, "one audit receipt per frozen command");

    let (pre_check, rest) = commands.split_first().expect("pre_check receipt");
    assert_eq!(pre_check.command_index, 1);
    assert_eq!(
        pre_check.verdict,
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandVerdict::Rejected,
        "member git mutation + unknown path must reject the command receipt"
    );
    let rejection_reason = pre_check
        .rejection_reason
        .as_deref()
        .expect("rejection carries a reason");
    assert!(!rejection_reason.trim().is_empty());
    let changes = &pre_check.observed_changes;
    let member_git_change = changes
        .iter()
        .find(|change| change.path == "member-a/.git/HEAD")
        .expect("member git mutation is durable evidence");
    assert_eq!(
        member_git_change.classification,
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeChangeClass::MemberGit
    );
    assert!(!member_git_change.allowed);
    let unknown_change = changes
        .iter()
        .find(|change| change.path == "rogue.txt")
        .expect("unknown root path write is durable evidence");
    assert_eq!(
        unknown_change.classification,
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeChangeClass::UnknownPath
    );
    assert!(!unknown_change.allowed);

    for receipt in rest {
        assert_eq!(
            receipt.verdict,
            crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandVerdict::Allowed,
            "command {} has no further writes: its window must stay allowed",
            receipt.command_index
        );
    }

    // finalize fail-closed：任一命令未 Allowed 即无最终 receipt。
    assert!(
        receipts
            .get("project_0001", &operation.operation_id)
            .expect("final receipt read")
            .is_none(),
        "finalize must refuse when any command receipt is rejected"
    );

    // readiness 失败面：planning_ready=false，末命令收口失败以
    // provider-turn 失败 reason 呈现（Task 4 契约替代旧 root_receipt_missing
    // 等待面——被拒命令不再允许 warn-only 补发）。
    let (status, projection) = get_json(
        &fixture.app,
        &format!(
            "/api/projects/project_0001/logical-codebases/{}/bootstrap",
            fixture.lc_id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projection}");
    assert_eq!(projection["planning_ready"], false, "{projection}");
    let reason_codes: Vec<&str> = projection["notices"]
        .as_array()
        .expect("notices array")
        .iter()
        .filter_map(|notice| notice["reason_code"].as_str())
        .collect();
    assert!(
        reason_codes.contains(&"aggregate_openspec_and_examples_failed"),
        "readiness must surface the final-command closure failure, got {reason_codes:?}"
    );

    // auditor 只读：越界字节与成员 Git 篡改原样在盘（不覆盖、不回滚）。
    assert_eq!(
        std::fs::read_to_string(fixture.aggregate_root.join("rogue.txt")).expect("rogue bytes"),
        "out of root recipe write\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.member_repo.join(".git").join("HEAD"))
            .expect("member HEAD bytes"),
        "ref: refs/heads/rogue-mutation\n"
    );
}

/// Task 3.2（D3/BOOT-01 后半句）：调用图/可达性回归锁——替代
/// `coordinator_tests_contract` 的源码 token 扫描（Task 1.1 形态）。共享
/// 四命令 executor（`root_recipe_command_index` 命令源 + gateway 驱动的
/// provider turn）完整执行五步 root recipe 时，以四条可观测调用图边界
/// 证明单仓 registration/GitFinalize 不可达：
/// 1. 成员 Git 状态指纹零变化（`git add -A`/`git commit` 会改写
///    head_revision/porcelain/tracked）；
/// 2. canonical 根文件清单变化恰为 recipe 自有产物（allowlisted 规则
///    材料）+ Task 4 发布 locator——根级 git_finalize/未知写入暴露为
///    `.git` 树或清单外新路径；
/// 3. 单仓持久化指纹零残留：`RepositoryStore` 无 Repository 记录、
///    `repository-initializations/` 无单仓六步 operation 记录
///    （`RepositoryRegistrationCoordinator` 的两条落盘边）；
/// 4. 三个 provider turn 全部经 gateway 启动且携带 policy digest
///    （`ClaudeRepositoryInitializer` 的 registry 直连启动不留审计），
///    四条命令 receipt 的命令文本与冻结命令索引一致（共享命令源真实
///    执行），最终 receipt 冻结真正文 digest。
#[tokio::test]
async fn shared_executor_cannot_reach_repository_registration_or_git_finalize() {
    let fixture = root_safety_fixture(false);
    let member_before = root_safety_member_fingerprint(&fixture.member_repo);
    let root_before = aggregate_root_inventory(&fixture.aggregate_root);

    let operation = root_safety_run_recipe_to_terminal(&fixture, "d3-isolation-1").await;
    assert_eq!(
        operation.status,
        crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
        "shared executor must finish the five-step recipe: {:?}",
        operation.error
    );

    // 1. 成员 Git 调用图指纹：git_finalize（add -A/commit）零痕迹。
    assert_eq!(
        root_safety_member_fingerprint(&fixture.member_repo),
        member_before,
        "member git state must stay byte-identical (no git_finalize edge)"
    );

    // 2. 根清单：新增路径必须恰为 recipe 自有产物（AGENTS/.claude 规则
    //    材料）与发布 locator 树（Task 4），无任何删除/改写——根级
    //    git_finalize（`.git` 树）或未知写入会暴露为清单外路径。
    let policy_artifact =
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            fixture.paths.clone(),
            fixture.lc_id.clone(),
        )
        .get("project_0001")
        .expect("policy artifact read")
        .expect("published policy artifact");
    let mut recipe_owned: Vec<String> = vec![
        "AGENTS.md".to_string(),
        ".claude".to_string(),
        ".claude/rules".to_string(),
        ".claude/rules/language.md".to_string(),
    ];
    let mut prefix = String::new();
    for segment in policy_artifact.policy_id.split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        recipe_owned.push(prefix.clone());
    }
    let root_after = aggregate_root_inventory(&fixture.aggregate_root);
    for (path, before) in &root_before {
        assert_eq!(
            root_after.get(path),
            Some(before),
            "pre-existing root entry {path} must stay byte-identical"
        );
    }
    let added: std::collections::BTreeSet<&String> = root_after
        .keys()
        .filter(|path| !root_before.contains_key(*path))
        .collect();
    let expected_added: std::collections::BTreeSet<String> = recipe_owned.into_iter().collect();
    assert_eq!(
        added
            .into_iter()
            .map(|path| (*path).clone())
            .collect::<std::collections::BTreeSet<String>>(),
        expected_added,
        "root inventory delta must be exactly the recipe-owned artifacts plus the published locator"
    );

    // 3. 单仓 registration 持久化指纹：零 Repository 记录、零单仓 operation。
    let repositories =
        crate::product::repository_store::RepositoryStore::new(fixture.paths.clone())
            .list("project_0001")
            .expect("repository records readable");
    assert!(
        repositories.is_empty(),
        "aggregate root recipe must not persist single-repository records: {repositories:?}"
    );
    let single_repo_operations = fixture
        .paths
        .repository_initializations_root("project_0001");
    let operation_files = std::fs::read_dir(&single_repo_operations)
        .map(|entries| entries.filter_map(|entry| entry.ok()).count())
        .unwrap_or(0);
    assert_eq!(
        operation_files, 0,
        "aggregate root recipe must not create single-repository initialization operations"
    );

    // 4. 共享 executor 的 gateway 启动边 + 冻结命令源。
    assert_eq!(
        fixture.factory.audit().stream_launches(),
        3,
        "three provider turns must launch through the LC gateway"
    );
    assert!(
        fixture.factory.audit().all_have_policy_digest(),
        "every shared-executor launch must carry a policy digest (registry-direct initializer would not)"
    );
    let receipts = crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        fixture.paths.clone(),
        fixture.lc_id.clone(),
    );
    let mut commands = receipts
        .list_commands("project_0001", &operation.operation_id)
        .expect("command receipts");
    commands.sort_by_key(|receipt| receipt.command_index);
    let frozen_index =
        crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index(
        );
    assert_eq!(commands.len(), frozen_index.len());
    for (receipt, (_, command_index, command)) in commands.iter().zip(frozen_index.iter()) {
        assert_eq!(receipt.command_index, *command_index);
        assert_eq!(receipt.command, *command);
        assert_eq!(
            receipt.verdict,
            crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandVerdict::Allowed
        );
        match receipt.command_index {
            // 命令 2/3 共享同一 RuleAndMcpConfig turn（两条 watch 都在 turn
            // 前开启）——窗口包含规则材料生成（AGENTS/.claude 规则），全部
            // allowlisted；命令 1/4 窗口零写入（发布发生在全部审计窗口
            // 关闭之后，不进任何窗口）。
            2 | 3 => assert!(
                receipt.observed_changes.iter().all(|change| change.allowed),
                "rule-material generation must be allowlisted evidence: {:?}",
                receipt.observed_changes
            ),
            _ => assert!(
                receipt.observed_changes.is_empty(),
                "command {} must not touch anything outside its own audit window",
                receipt.command_index
            ),
        }
    }
    // 最终 receipt 冻结真正文 digest（Task 4 末命令收口）。
    let receipt = receipts
        .get("project_0001", &operation.operation_id)
        .expect("final receipt read")
        .expect("finalized root receipt");
    assert_eq!(receipt.policy_digest, policy_artifact.digest);
    assert!(!policy_artifact.is_bootstrap_placeholder());
}
