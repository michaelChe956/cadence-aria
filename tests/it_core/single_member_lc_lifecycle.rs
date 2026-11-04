//! 块3 片4（REQ-REG-17）：1 成员 LC 的**行为级证据**——单成员（1 成员）与
//! 多成员（N 成员）走同一条代码路径，并锁死以下不变量：
//!
//! - T4.4 登记 → 首建推进到 `PlanningReady`：真实登记链
//!   （preflight → confirmed batch → resume）+ 真实首建推进器（advance），
//!   1 成员 LC 全程无拒绝、无降级停点；
//! - T4.5 仓序自动边零条：`change_order_pairs` 是仓序自动边唯一规则源
//!   （REQ-PLN-09），规则体是「严格后继双重循环、零分支」——`n <= 1`
//!   （单成员 LC 的 change_order 至多 1 项）在数学上产出 0 对自动边；
//!   推进停点是 `PlanningReady` 而非 `WaitingForHuman`/`Failed`/迭代上限，
//!   即「不因无后仓报错或降级」；
//! - T4.6 成员/聚合索引各索引 1 个成员：成员索引（list_members）恰 1 条；
//!   聚合索引用真实 store/record 类型落盘恰 1 份成员快照（同一构造路径
//!   对 1 与 N 成员无差别）；
//! - T4.7 成员数 1↔N 变化无特殊流程：把同一登记链原样再跑一遍挂上第 2
//!   个成员仓——manifest 成员数/成员修订自然前进，bootstrap 投影自然
//!   反映（仍 planning_ready、五步 Completed），全程没有迁移步骤，也
//!   没有「切回单仓」路径介入。
//!
//! 夹具说明：全部走 `cadence_aria` 公共 API（登记协调器、bootstrap 服务、
//! 真实 store）。provider 长任务（跑根配方）与聚合索引重建是 web 层注入的
//! 派发面，本文件按 `bootstrap_advance_tests.inc.rs` 同构的 durable 结果面
//! 物化（政策 artifact + 五步 Completed operation + 四条 Allowed 命令审计
//! receipt），并在重建派发器里用真实 manifest/成员/checkout 构造成员快照。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::aggregate_index::{
    AggregateIndexError, AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
    AggregateIndexStore,
};
use cadence_aria::product::logical_codebase::aggregate_initialization::{
    AggregateInitializationOperation, AggregateInitializationOperationInput,
    AggregateInitializationStepKind,
};
use cadence_aria::product::logical_codebase::policy::{
    AggregatePolicyArtifact, AggregatePolicyArtifactStore,
};
use cadence_aria::product::logical_codebase::root_recipe_receipt::{
    ROOT_RULE_ENTRY_FILE, RootRecipeCommandVerdict, RootRecipeFilesystemAuditor,
    RootRecipeReceiptStore, root_rule_digest,
};
use cadence_aria::product::logical_codebase::{
    AggregateInitializationOperationStore, BootstrapAdvanceRequest, BootstrapAdvanceResult,
    BootstrapAdvanceStop, BootstrapPrepareOutcome, BootstrapPrepareRequest, CanonicalAggregateRoot,
    CheckoutKind, ConfirmedRegistrationBatchInput, LogicalCodebaseBootstrapProjector,
    LogicalCodebaseBootstrapService, LogicalCodebaseBootstrapStepStatus, LogicalCodebaseManifest,
    LogicalCodebaseRegistrationCoordinator, LogicalCodebaseStore, MemberStatus,
    RegistrationBatchStatus, RegistrationCandidateState, RegistrationItemStatus,
    RegistrationPreflightInput, RegistrationPreflightResult, root_recipe_command_index,
};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};

const PROJECT_ID: &str = "project_0001";
const OPERATION_ID: &str = "aggregate_initialization_single_member_0001";
const INDEX_ID: &str = "aggregate_index_single_member_0001";

// ---------------------------------------------------------------------------
// 基础夹具
// ---------------------------------------------------------------------------

fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repository)
        .args(args)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "git {} in {}: {error}",
                args.join(" "),
                repository.display()
            )
        });
    assert!(
        output.status.success(),
        "git {} in {} failed:\nstdout: {}\nstderr: {}",
        args.join(" "),
        repository.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// 干净成员仓（与 bootstrap 内部夹具同构）：README + 真实成员规则材料 +
/// 初始提交——rules/policy 步只读检查成员规则（`.claude/rules/language.md`）。
fn init_clean_member_repository(path: &Path) {
    std::fs::create_dir_all(path.join(".claude/rules")).expect("create member rules dir");
    git(path, &["init", "-b", "main"]);
    git(path, &["config", "user.email", "single-member@test.local"]);
    git(path, &["config", "user.name", "Single Member Test"]);
    std::fs::write(path.join("README.md"), "# member\n").expect("write README");
    std::fs::write(path.join(".claude/rules/language.md"), "# rule\n").expect("write rule");
    git(path, &["add", "."]);
    git(path, &["commit", "-m", "init"]);
}

/// 已登记的 LC 夹具：`_temp` 必须存活到测试结束（.aria 应用数据在其下）。
struct RegisteredLc {
    _temp: tempfile::TempDir,
    paths: ProductAppPaths,
    lc_id: String,
    aggregate_root: PathBuf,
    manifest: LogicalCodebaseManifest,
}

fn create_project_and_lc(paths: &ProductAppPaths, aggregate_root: &Path) -> String {
    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "single-member-lifecycle".to_string(),
            description: None,
        })
        .expect("create project");
    assert_eq!(
        project.id, PROJECT_ID,
        "空项目目录的首个项目恒为 project_0001"
    );
    let record = LogicalCodebaseStore::new(paths.clone())
        .create(
            PROJECT_ID,
            cadence_aria::product::logical_codebase::LogicalCodebaseCreateInput {
                name: "single-member-lc".to_string(),
                aggregate_root: aggregate_root.to_path_buf(),
            },
        )
        .expect("create logical codebase");
    record.id
}

fn registration_coordinator(
    paths: &ProductAppPaths,
    lc_id: &str,
) -> LogicalCodebaseRegistrationCoordinator {
    LogicalCodebaseRegistrationCoordinator::for_lc(paths.clone(), lc_id.to_string())
}

/// 真实登记链的发现→确认两步（执行 resume 由调用方显式做，便于逐段断言）。
fn preflight_and_confirm(
    paths: &ProductAppPaths,
    lc_id: &str,
    aggregate_root: &Path,
) -> (
    RegistrationPreflightResult,
    cadence_aria::product::logical_codebase::RegistrationBatchRecord,
) {
    let coordinator = registration_coordinator(paths, lc_id);
    let preflight = coordinator
        .preflight(RegistrationPreflightInput {
            project_id: PROJECT_ID.to_string(),
            aggregate_root: CanonicalAggregateRoot {
                canonical_path: std::fs::canonicalize(aggregate_root)
                    .expect("canonical aggregate root"),
            },
            paths: Vec::new(),
        })
        .expect("preflight must run on the single-member aggregate root");
    let batch = coordinator
        .submit_confirmed_batch(ConfirmedRegistrationBatchInput::from_preflight(
            &preflight, false,
        ))
        .expect("confirm the preflight batch");
    (preflight, batch)
}

/// 走完整真实登记链：项目 → LC → 1 个干净成员仓 → preflight（发现模式）
/// → confirmed batch → resume。返回登记后的 durable 现场。
async fn registered_single_member_lc() -> RegisteredLc {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(temp.path().to_path_buf());
    let aggregate_root = temp.path().join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
    let lc_id = create_project_and_lc(&paths, &aggregate_root);

    init_clean_member_repository(&aggregate_root.join("repo"));

    let (preflight, batch) = preflight_and_confirm(&paths, &lc_id, &aggregate_root);
    assert_eq!(
        preflight.candidates.len(),
        1,
        "发现模式必须恰好找到 1 个成员仓候选"
    );
    assert_eq!(preflight.count(RegistrationCandidateState::Eligible), 1);

    let coordinator = registration_coordinator(&paths, &lc_id);
    let resumed = coordinator
        .resume_batch(PROJECT_ID, &batch.id)
        .expect("resume must attach the single member");
    assert_eq!(resumed.status, RegistrationBatchStatus::Completed);
    assert!(
        resumed
            .items
            .iter()
            .all(|item| item.status == RegistrationItemStatus::Completed)
    );

    let store = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone());
    let manifest = store
        .load_manifest(PROJECT_ID)
        .expect("load manifest")
        .expect("registration must have created the manifest");
    assert_eq!(
        manifest.member_ids.len(),
        1,
        "登记后 manifest 恰有 1 个成员"
    );
    RegisteredLc {
        _temp: temp,
        paths,
        lc_id,
        aggregate_root,
        manifest,
    }
}

// ---------------------------------------------------------------------------
// 首建推进材料（与 bootstrap_advance_tests.inc.rs 的 durable 结果面同构）
// ---------------------------------------------------------------------------

fn policy_revision_one(manifest: &LogicalCodebaseManifest) -> AggregatePolicyArtifact {
    use sha2::{Digest, Sha256};
    let policy_text = "# Aggregate policy (root recipe)\n\nRoot-recipe aggregated policy for planning read-only and coding target-write sessions.\n"
        .to_string();
    AggregatePolicyArtifact {
        policy_id: format!(
            "policy/{}/{}/1",
            manifest.project_id, manifest.logical_codebase_id
        ),
        project_id: manifest.project_id.clone(),
        logical_codebase_id: manifest.logical_codebase_id.to_string(),
        revision: 1,
        digest: format!("sha256:{:x}", Sha256::digest(policy_text.as_bytes())),
        policy_text,
        created_at: "2026-11-03T00:00:00Z".to_string(),
    }
}

fn write_root_rule_and_freeze_digest(aggregate_root: &Path) -> String {
    std::fs::write(
        aggregate_root.join(ROOT_RULE_ENTRY_FILE),
        "# aggregate root rules\n",
    )
    .expect("write root rule entry");
    let canonical = std::fs::canonicalize(aggregate_root).expect("canonical aggregate root");
    root_rule_digest(&canonical)
        .expect("compute root rule digest")
        .expect("root rule digest must resolve after writing the entry")
}

fn materialize_root_recipe(
    paths: &ProductAppPaths,
    lc_id: &str,
    aggregate_root: &Path,
    membership_revision: u64,
    policy: &AggregatePolicyArtifact,
    rule_digest: &str,
) {
    AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id.to_string())
        .save(PROJECT_ID, policy)
        .expect("publish policy artifact");
    let store = AggregateInitializationOperationStore::for_lc(paths.clone(), lc_id.to_string());
    let input = AggregateInitializationOperationInput {
        idempotency_key: format!("single-member-{OPERATION_ID}"),
        manifest_revision: membership_revision,
        policy_digest: policy.digest.clone(),
        profile_evidence_digest: Some("sha256:profile-evidence".to_string()),
        provider_context_root: aggregate_root.to_path_buf(),
        provider: "claude_code".to_string(),
    };
    store
        .create_idempotent(AggregateInitializationOperation::new(
            OPERATION_ID.to_string(),
            PROJECT_ID.to_string(),
            input,
            "2026-11-03T00:01:00Z".to_string(),
        ))
        .expect("create root recipe operation");
    store
        .mark_running(PROJECT_ID, OPERATION_ID, "2026-11-03T00:02:00Z".to_string())
        .expect("mark operation running");
    for step in AggregateInitializationStepKind::V1 {
        store
            .mark_step_running(
                PROJECT_ID,
                OPERATION_ID,
                step,
                format!("single-member:{OPERATION_ID}:{}", step.as_str()),
                "2026-11-03T00:03:00Z".to_string(),
            )
            .expect("mark step running");
        store
            .checkpoint_step_output(
                PROJECT_ID,
                OPERATION_ID,
                step,
                format!("aggregate-initializations/op/{}.json", step.as_str()),
                "2026-11-03T00:04:00Z".to_string(),
            )
            .expect("checkpoint step output");
        store
            .mark_step_completed(
                PROJECT_ID,
                OPERATION_ID,
                step,
                "2026-11-03T00:05:00Z".to_string(),
            )
            .expect("mark step completed");
    }
    store
        .finish_completed(PROJECT_ID, OPERATION_ID, "2026-11-03T00:06:00Z".to_string())
        .expect("finish operation");

    let receipt_store = RootRecipeReceiptStore::for_lc(paths.clone(), lc_id.to_string());
    let auditor = RootRecipeFilesystemAuditor::new();
    let canonical_root = std::fs::canonicalize(aggregate_root).expect("canonical root");
    for (index, (step, command_index, command)) in
        root_recipe_command_index().into_iter().enumerate()
    {
        let watch = auditor
            .before_command(OPERATION_ID, &canonical_root, step, command_index, command)
            .expect("audit before command");
        let receipt = auditor
            .after_command(watch, format!("2026-11-03T00:10:{index:02}Z"))
            .expect("audit after command");
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
        receipt_store
            .append_command(PROJECT_ID, receipt)
            .expect("append command receipt");
    }
    receipt_store
        .finalize(
            PROJECT_ID,
            OPERATION_ID,
            &policy.digest,
            rule_digest,
            "2026-11-03T00:20:00Z".to_string(),
        )
        .expect("finalize root receipt");
}

/// 重建派发器形态：真实 manifest/成员/checkout 构造成员快照并经
/// `AggregateIndexStore` 落盘激活。
type IndexRebuildDispatcher =
    Arc<dyn Fn(&str, &str, u64) -> Result<AggregateIndexRecord, AggregateIndexError> + Send + Sync>;

/// 重建派发器：用真实 manifest/成员/Main checkout 构造成员快照并经真实
/// `AggregateIndexStore` 落盘激活——成员数只是循环上界，1 与 N 无差别。
fn index_rebuild_dispatcher(lc: &RegisteredLc) -> IndexRebuildDispatcher {
    let paths = lc.paths.clone();
    let lc_id = lc.lc_id.clone();
    Arc::new(
        move |project_id: &str, command_id: &str, expected_revision: u64| {
            let store = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone());
            let manifest = store
                .load_manifest(project_id)
                .expect("load manifest for rebuild")
                .expect("manifest exists");
            let checkouts = store.list_checkouts(project_id).expect("list checkouts");
            let mut snapshots = Vec::new();
            for member_id in &manifest.member_ids {
                let member = store
                    .load_member(project_id, *member_id)
                    .expect("load member")
                    .expect("member registered");
                if member.status != MemberStatus::Active {
                    continue;
                }
                let checkout = checkouts
                    .iter()
                    .find(|checkout| {
                        checkout.logical_repository_id == *member_id
                            && checkout.kind == CheckoutKind::Main
                    })
                    .expect("each active member has one main checkout");
                let revision = checkout
                    .revision
                    .clone()
                    .unwrap_or_else(|| git(&checkout.canonical_path, &["rev-parse", "HEAD"]));
                snapshots.push(AggregateIndexMemberSnapshot::indexed(
                    *member_id,
                    checkout.checkout_id,
                    revision,
                    false,
                    "2026-11-03T00:07:00Z".to_string(),
                ));
            }
            let index_store = AggregateIndexStore::for_lc(paths.clone(), lc_id.clone());
            let mut record = AggregateIndexRecord::building(
                INDEX_ID.to_string(),
                project_id.to_string(),
                expected_revision,
                snapshots,
                "2026-11-03T00:07:00Z".to_string(),
            );
            record.command_id = Some(command_id.to_string());
            record.config_digest = "sha256:single-member-lifecycle".to_string();
            index_store
                .create(project_id, record)
                .expect("create aggregate index generation");
            index_store.mark_status(project_id, INDEX_ID, AggregateIndexStatus::Active, None)
        },
    )
}

fn bootstrap_service(
    lc: &RegisteredLc,
    policy: &AggregatePolicyArtifact,
    rule_digest: &str,
) -> LogicalCodebaseBootstrapService {
    let prepare = {
        let paths = lc.paths.clone();
        let lc_id = lc.lc_id.clone();
        let aggregate_root = lc.aggregate_root.clone();
        let membership_revision = lc.manifest.membership_revision;
        let policy = policy.clone();
        let rule_digest = rule_digest.to_string();
        Arc::new(move |_request: &BootstrapPrepareRequest| {
            materialize_root_recipe(
                &paths,
                &lc_id,
                &aggregate_root,
                membership_revision,
                &policy,
                &rule_digest,
            );
            BootstrapPrepareOutcome::Started
        })
    };
    LogicalCodebaseBootstrapService::new(lc.paths.clone())
        .with_root_recipe_prepare(prepare)
        .with_aggregate_index_rebuild(index_rebuild_dispatcher(lc))
}

/// 登记后推进到 PlanningReady（T4.4/T4.5/T4.6 共用的完整链）。
async fn advanced_single_member_lc() -> (RegisteredLc, BootstrapAdvanceResult) {
    let lc = registered_single_member_lc().await;
    let rule_digest = write_root_rule_and_freeze_digest(&lc.aggregate_root);
    let policy = policy_revision_one(&lc.manifest);
    let service = bootstrap_service(&lc, &policy, &rule_digest);
    let result = service
        .advance(BootstrapAdvanceRequest {
            project_id: PROJECT_ID.to_string(),
            logical_codebase_id: lc.lc_id.clone(),
        })
        .await
        .expect("single-member LC first build must advance without failure");
    (lc, result)
}

fn assert_all_steps_completed(result: &BootstrapAdvanceResult) {
    assert_eq!(
        result.projection.steps.len(),
        5,
        "首建恒为五步（identity → manifest/checkout → rules/policy → member index → aggregate index）"
    );
    for step in &result.projection.steps {
        assert_eq!(
            step.status,
            LogicalCodebaseBootstrapStepStatus::Completed,
            "{} 必须完成（当前 {:?}）",
            step.step.as_str(),
            step.failure
                .as_ref()
                .map(|failure| failure.reason_code.as_str())
        );
    }
}

// ---------------------------------------------------------------------------
// T4.4~T4.7
// ---------------------------------------------------------------------------

/// T4.4：1 成员 LC 从真实登记链到首建 PlanningReady——无拒绝、无降级。
#[tokio::test]
async fn single_member_lc_registration_and_first_build_reaches_planning_ready() {
    let (lc, result) = advanced_single_member_lc().await;

    assert_eq!(result.stopped, BootstrapAdvanceStop::PlanningReady);
    assert!(result.projection.planning_ready);
    assert_all_steps_completed(&result);

    // identity 步的完成判据本身是「active 成员 ≥1」（bootstrap_projector：
    // active_members == 0 → NotStarted；≥1 → Completed）——1 成员即完成，
    // 是「同路径」最直接的判据级证据。
    let members = LogicalCodebaseStore::for_lc(lc.paths.clone(), lc.lc_id.clone())
        .list_members(PROJECT_ID)
        .expect("list members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].status, MemberStatus::Active);

    // manifest/checkout 步的事实面：1 条 Main checkout。
    let checkouts = LogicalCodebaseStore::for_lc(lc.paths.clone(), lc.lc_id.clone())
        .list_checkouts(PROJECT_ID)
        .expect("list checkouts");
    assert_eq!(checkouts.len(), 1);
    assert_eq!(checkouts[0].kind, CheckoutKind::Main);
}

/// T4.5：单成员仓序自动边零条——规则源锁定 + 推进无「无后仓」降级。
#[tokio::test]
async fn single_member_change_order_has_zero_auto_edges() {
    let (lc, result) = advanced_single_member_lc().await;

    // 行为面：推进停在 PlanningReady（WaitingForHuman/Failed/迭代上限才是
    // 「因无后仓报错或降级」的停点）——单成员没有任何降级停点。
    assert_eq!(result.stopped, BootstrapAdvanceStop::PlanningReady);
    assert!(result.projection.planning_ready);

    // 数据面：manifest 恰 1 个成员 ⇒ 本 LC 任何 confirmed Design 的
    // change_order（成员子集）至多 1 项。
    assert_eq!(lc.manifest.member_ids.len(), 1);

    // 规则源锁定：仓序自动边唯一规则源 `change_order_pairs`（REQ-PLN-09，
    // compile_support.rs:828 注释自证「同仓/单仓/空顺序无对，单仓 compile
    // 行为不变」）。锁死三件事：
    // 1) 全 src/** 只有这一个定义（算法不得被复制出第二份规则源）；
    // 2) 规则体是严格后继双重循环且零分支（无 if、无 `== 1`、无
    //    is_empty）——`for i in 0..n { for j in (i+1)..n }` 在 n <= 1 时
    //    迭代 0 次，单成员 ⇒ 自动边集合为空是规则的结构性质；
    // 3) Final Compile 合并路径与 SG1 纯投影两个消费点都调用它（不复制）。
    change_order_auto_edge_rule_source_lock();
}

/// 仓序自动边唯一规则源的结构锁（见 T4.5 注释）。任何把规则改成带成员数
/// 分支、或把边算法复制到第二个定义点的改动都会在此报红。
fn change_order_auto_edge_rule_source_lock() {
    let rule_file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/product/workspace_engine/draft_batch/compile_support.rs");
    let text = std::fs::read_to_string(&rule_file)
        .unwrap_or_else(|error| panic!("read {}: {error}", rule_file.display()));
    let lines: Vec<&str> = text.lines().collect();

    let definitions: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("fn change_order_pairs"))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        definitions.len(),
        1,
        "仓序自动边规则源必须唯一：compile_support.rs 内只允许一个 change_order_pairs 定义"
    );
    assert_eq!(
        count_change_order_pairs_definitions(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
        ),
        1,
        "仓序自动边规则源必须唯一：全 src/** 只允许 compile_support.rs::change_order_pairs 一个定义"
    );
    let definition = definitions[0];
    assert!(
        lines[definition]
            .trim_start()
            .starts_with("fn change_order_pairs"),
        "规则定义必须位于生产顶层（不得移入测试/嵌套模块）"
    );

    let body_end = block_end_line(&lines, definition)
        .unwrap_or_else(|| panic!("规则体花括号失衡：line {}", definition + 1));
    let body: String = lines[definition..=body_end].join("\n");
    let squashed: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let has_if_token = lines[definition..=body_end].iter().any(|line| {
        line.split_whitespace().any(|token| token == "if")
            // 无空格形态 `if(`（squash 后判定）：
            || line.contains("if(")
    });
    assert!(
        !has_if_token,
        "规则体不得含 if 分支：单成员/多成员必须由同一无分支枚举覆盖；当前规则体被改动过。规则体原文：\n{body}"
    );
    assert!(
        !squashed.contains("==1") && !squashed.contains("is_empty"),
        "规则体不得含 `== 1` / `is_empty` 一类成员数特判"
    );
    let loop_headers = lines[definition..=body_end]
        .iter()
        .filter(|line| line.trim_start().starts_with("for "))
        .count();
    assert_eq!(
        loop_headers, 2,
        "规则体必须是严格后继双重循环（外层 0..n、内层 (i+1)..n）——n <= 1 时 0 次迭代"
    );

    let call_sites = lines
        .iter()
        .filter(|line| {
            line.contains("change_order_pairs(")
                && !line.contains("fn change_order_pairs")
                && !line.trim_start().starts_with("//")
        })
        .count();
    assert_eq!(
        call_sites, 2,
        "Final Compile 合并路径与 SG1 纯投影两个消费点必须共用同一规则源（不得复制边算法）"
    );
}

/// 从 item 头行起找到 item 体的配对右花括号行号（0-based）。
fn block_end_line(lines: &[&str], header: usize) -> Option<usize> {
    let mut depth: i64 = 0;
    let mut seen_open = false;
    for (offset, line) in lines.iter().enumerate().skip(header) {
        for character in line.chars() {
            match character {
                '{' => {
                    depth += 1;
                    seen_open = true;
                }
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        return None;
                    }
                    if seen_open && depth == 0 {
                        // enumerate().skip(header) 的 offset 已是绝对行号。
                        return Some(offset);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// 全 src/** 里 `fn change_order_pairs` 定义的个数（生产与测试一起数：
/// 规则源唯一性是硬约束，复制到任何地方——包括测试替身——都算破坏）。
fn count_change_order_pairs_definitions(directory: &Path) -> usize {
    let mut count = 0usize;
    let entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name == ".git" || name == "target" {
                continue;
            }
            count += count_change_order_pairs_definitions(&path);
        } else if name.ends_with(".rs") {
            let raw = std::fs::read_to_string(&path).unwrap_or_default();
            count += raw
                .lines()
                .filter(|line| line.contains("fn change_order_pairs"))
                .count();
        }
    }
    count
}

/// T4.6：成员索引与聚合索引各索引 1 个成员（同一构造路径对 1/N 无差别）。
#[tokio::test]
async fn single_member_indexes_one_member() {
    let lc = registered_single_member_lc().await;

    // 成员索引：登记链落盘的成员表恰 1 条 Active。
    let store = LogicalCodebaseStore::for_lc(lc.paths.clone(), lc.lc_id.clone());
    let members = store.list_members(PROJECT_ID).expect("list members");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].status, MemberStatus::Active);
    assert_eq!(members[0].logical_repository_id, lc.manifest.member_ids[0]);

    // 推进到 ready（重建派发器用真实成员构造快照）。
    let rule_digest = write_root_rule_and_freeze_digest(&lc.aggregate_root);
    let policy = policy_revision_one(&lc.manifest);
    let service = bootstrap_service(&lc, &policy, &rule_digest);
    let result = service
        .advance(BootstrapAdvanceRequest {
            project_id: PROJECT_ID.to_string(),
            logical_codebase_id: lc.lc_id.clone(),
        })
        .await
        .expect("advance");
    assert_eq!(result.stopped, BootstrapAdvanceStop::PlanningReady);

    // 聚合索引：真实 store 里的活跃世代恰索引 1 个成员（快照指向登记成员
    // 与其 Main checkout，含 revision，included=true）。
    let active = AggregateIndexStore::for_lc(lc.paths.clone(), lc.lc_id.clone())
        .active(PROJECT_ID)
        .expect("load active aggregate index")
        .expect("aggregate index must be active after first build");
    assert_eq!(active.member_snapshots.len(), 1);
    let snapshot = &active.member_snapshots[0];
    assert_eq!(snapshot.logical_repository_id, lc.manifest.member_ids[0]);
    assert_eq!(snapshot.checkout_id, members[0].checkout_ids[0]);
    assert!(snapshot.included);
    assert!(!snapshot.dirty);
    assert_eq!(
        active.membership_revision, lc.manifest.membership_revision,
        "聚合索引世代绑定登记后的成员修订"
    );
}

/// T4.7：成员数 1↔N 变化无特殊流程——同一登记链再挂第 2 个成员仓，
/// manifest/成员表/bootstrap 投影自然反映，无迁移步骤、无「切回单仓」路径。
#[tokio::test]
async fn single_member_count_change_needs_no_special_flow() {
    let (lc, result) = advanced_single_member_lc().await;
    assert_eq!(result.stopped, BootstrapAdvanceStop::PlanningReady);
    let revision_after_first_member = lc.manifest.membership_revision;
    assert_eq!(
        revision_after_first_member, 2,
        "空 manifest(rev 1) + 首成员挂载"
    );

    // 1 → N：同一登记链原样再跑（发现模式 preflight → confirm → resume）。
    init_clean_member_repository(&lc.aggregate_root.join("repo2"));
    let (preflight, batch) = preflight_and_confirm(&lc.paths, &lc.lc_id, &lc.aggregate_root);
    assert_eq!(preflight.candidates.len(), 2, "两个成员仓都被发现");
    // 同一分类器：已登记成员 → Duplicate(already_registered)；新成员 → Eligible。
    let first = preflight
        .candidates
        .iter()
        .find(|candidate| {
            candidate
                .canonical_path
                .as_deref()
                .is_some_and(|path| path.ends_with("repo"))
        })
        .expect("first member candidate");
    assert_eq!(first.state, RegistrationCandidateState::Duplicate);
    assert_eq!(first.reason, "already_registered");
    let second = preflight
        .candidates
        .iter()
        .find(|candidate| {
            candidate
                .canonical_path
                .as_deref()
                .is_some_and(|path| path.ends_with("repo2"))
        })
        .expect("second member candidate");
    assert_eq!(second.state, RegistrationCandidateState::Eligible);
    assert_eq!(
        batch.items.len(),
        1,
        "确认批次只携带新成员（已登记成员按既有分类跳过，无特殊流程）"
    );

    let coordinator = registration_coordinator(&lc.paths, &lc.lc_id);
    let resumed = coordinator
        .resume_batch(PROJECT_ID, &batch.id)
        .expect("resume the second member batch");
    assert_eq!(resumed.status, RegistrationBatchStatus::Completed);

    // durable 投影自然反映：成员 2 条、manifest 修订 +1，全程零迁移步骤。
    let store = LogicalCodebaseStore::for_lc(lc.paths.clone(), lc.lc_id.clone());
    let manifest = store
        .load_manifest(PROJECT_ID)
        .expect("reload manifest")
        .expect("manifest exists");
    assert_eq!(manifest.member_ids.len(), 2);
    assert_eq!(
        manifest.membership_revision,
        revision_after_first_member + 1,
        "成员修订按既有 ensure_manifest_membership 前进，无手动迁移"
    );
    let members = store.list_members(PROJECT_ID).expect("list members");
    assert_eq!(members.len(), 2);
    assert!(
        members
            .iter()
            .all(|member| member.status == MemberStatus::Active)
    );

    // bootstrap 投影自然反映：仍 planning_ready、五步 Completed——1 ↔ N
    // 不触发任何「切回单仓」/重置/迁移动作（聚合索引旧世代原样保留）。
    let projection = LogicalCodebaseBootstrapProjector::new(lc.paths.clone())
        .project(PROJECT_ID, &lc.lc_id)
        .expect("re-project after member growth");
    assert!(
        projection.planning_ready,
        "成员 1→N 后投影必须保持就绪（无迁移/重跑/降级）"
    );
    assert_eq!(projection.steps.len(), 5);
    for step in &projection.steps {
        assert_eq!(
            step.status,
            LogicalCodebaseBootstrapStepStatus::Completed,
            "{} 在成员 1→N 后保持完成",
            step.step.as_str()
        );
    }
    assert_eq!(
        projection.membership_revision,
        Some(manifest.membership_revision)
    );

    let active = AggregateIndexStore::for_lc(lc.paths.clone(), lc.lc_id.clone())
        .active(PROJECT_ID)
        .expect("load active aggregate index")
        .expect("active aggregate index preserved");
    assert_eq!(
        active.membership_revision, revision_after_first_member,
        "旧索引世代原样保留（无自动重建/切回动作）——重建属于显式流程"
    );
    assert_eq!(active.member_snapshots.len(), 1);
}
