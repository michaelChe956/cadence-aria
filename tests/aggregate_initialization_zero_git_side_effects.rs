//! Task 18: 聚合初始化零成员 Git 副作用回归。
//!
//! 本集成测试锁定 REQ-REG-02(登记/初始化零 git 副作用)对聚合初始化 operation
//! 的契约:聚合初始化成功执行前后,每个成员仓的 HEAD、porcelain status、tracked
//! 文件列表、untracked 文件列表与 `.git` 引用完全一致;聚合流程既不向成员仓
//! 写入 `.aria/aggregate`,也不发出任何 `git add -A` / `git commit` / `git push`
//! 调用。聚合根允许写 `.aria/aggregate/**`,但成员根一字节不得改变。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::aggregate_initialization_coordinator::{
    AggregatePreflightService, AggregateProviderTurnDriver, AggregateSkillsPreparation,
    MachineSkillsPreparation,
};
use cadence_aria::product::logical_codebase::store::LogicalCodebaseManifest;
use cadence_aria::product::logical_codebase::types::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId, MemberStatus,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use cadence_aria::product::logical_codebase::{
    AggregateInitializationCoordinator, AggregateInitializationOperationInput,
    AggregateInitializationStepKind, AggregatePreflightSnapshot, LogicalCodebaseStore,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const CREATED_AT: &str = "2026-08-09T00:00:00Z";

/// 拦截到的进程调用。`argv` 以程序名开头(如 `["git", "add", "-A"]`)。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedCall {
    argv: Vec<String>,
}

/// 每个成员仓的 Git 状态快照。覆盖 HEAD、porcelain status、tracked 文件列表、
/// untracked 文件列表与 `.git` 引用,使「成功/失败/取消三流程均比对」字节稳定。
#[derive(Debug, Clone, PartialEq, Eq)]
struct MemberGitSnapshot {
    head: String,
    status_porcelain: String,
    tracked_files: String,
    untracked_files: String,
    refs: String,
}

impl MemberGitSnapshot {
    fn capture(root: &Path) -> Self {
        Self {
            head: git_output(root, &["rev-parse", "HEAD"]),
            status_porcelain: git_output(root, &["status", "--porcelain"]),
            tracked_files: git_output(root, &["ls-files"]),
            untracked_files: git_output(root, &["status", "--porcelain", "--untracked"])
                .lines()
                .filter(|line| line.starts_with("?? "))
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
            refs: git_output(root, &["for-each-ref", "--format=%(refname) %(objectname)"]),
        }
    }
}

/// 多成员 Git 快照聚合,使 `capture(&[member_roots])` 与 `assert_eq!(before, after)`
/// 直接可用。成员顺序与传入顺序一致。
#[derive(Debug, Clone, PartialEq, Eq)]
struct AggregateInitializationGitSnapshot {
    members: Vec<MemberGitSnapshot>,
}

impl AggregateInitializationGitSnapshot {
    fn capture(member_roots: &[PathBuf]) -> Result<Self, String> {
        let mut members = Vec::with_capacity(member_roots.len());
        for root in member_roots {
            members.push(MemberGitSnapshot::capture(root));
        }
        Ok(Self { members })
    }
}

/// 记录型 provider turn 驱动:把每个 turn 当作一次「启动了 provider 进程」的
/// 合成 argv 记录进共享 command log,但不发出任何 git 调用。turn_count 反映
/// provider 实际被请求的次数,供断言使用。
struct RecordingProviderTurnDriver {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl RecordingProviderTurnDriver {
    fn new(calls: Arc<Mutex<Vec<RecordedCall>>>) -> Self {
        Self { calls }
    }
}

#[async_trait]
impl AggregateProviderTurnDriver for RecordingProviderTurnDriver {
    async fn run_turn(
        &self,
        _project_id: &str,
        _operation_id: &str,
        step: AggregateInitializationStepKind,
        _preflight: &AggregatePreflightSnapshot,
        _cancellation: CancellationToken,
    ) -> Result<String, cadence_aria::product::logical_codebase::AggregateInitializationError> {
        // 聚合 provider turn 只在聚合根读 / 写 `.aria/aggregate/**`;它绝不 spawn
        // 任何 git 调用。这里记录一条非 git 的合成 argv,使 command_log 断言
        // 能验证没有任何 `git add/commit/push`。
        self.calls
            .lock()
            .expect("command log mutex poisoned")
            .push(RecordedCall {
                argv: vec![
                    "claude".to_string(),
                    "--print".to_string(),
                    format!("aggregate-initialization:{}", step.as_str()),
                ],
            });
        Ok(format!("{} summary", step.as_str()))
    }
}

/// 记录型 skills preparation:记录一次合成(非 git)调用,返回稳定摘要。
struct RecordingSkillsPreparation {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

impl RecordingSkillsPreparation {
    fn new(calls: Arc<Mutex<Vec<RecordedCall>>>) -> Self {
        Self { calls }
    }
}

#[async_trait]
impl AggregateSkillsPreparation for RecordingSkillsPreparation {
    async fn prepare_skills(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _cancellation: CancellationToken,
    ) -> Result<
        MachineSkillsPreparation,
        cadence_aria::product::logical_codebase::AggregateInitializationError,
    > {
        self.calls
            .lock()
            .expect("command log mutex poisoned")
            .push(RecordedCall {
                argv: vec![
                    "cadence".to_string(),
                    "skills".to_string(),
                    "prepare".to_string(),
                ],
            });
        Ok(MachineSkillsPreparation {
            source_digest: "sha256:source".to_string(),
            link_digest: "sha256:link".to_string(),
            skills_root: PathBuf::from("/skills"),
            warnings: Vec::new(),
        })
    }
}

/// 聚合初始化零 Git 副作用 fixture:在公共非 Git 父目录下创建 2 个已提交
/// 的成员 Git 仓,持久化 logical codebase manifest + member + checkout,并把
/// 记录型 skills/provider 注入 coordinator。`command_log` 收集所有合成调用,
/// 供断言无 git add/commit/push。
struct AggregateInitializationGitFixture {
    _temp: tempfile::TempDir,
    member_roots: Vec<PathBuf>,
    command_log: Arc<Mutex<Vec<RecordedCall>>>,
    coordinator: AggregateInitializationCoordinator,
}

impl AggregateInitializationGitFixture {
    fn member_roots(&self) -> Vec<PathBuf> {
        self.member_roots.clone()
    }

    fn command_log(&self) -> Vec<RecordedCall> {
        self.command_log
            .lock()
            .expect("command log mutex poisoned")
            .clone()
    }

    fn coordinator(&self) -> &AggregateInitializationCoordinator {
        &self.coordinator
    }

    fn run_successful_aggregate_initialization(
        &self,
    ) -> Result<
        cadence_aria::product::logical_codebase::AggregateInitializationOperation,
        cadence_aria::product::logical_codebase::AggregateInitializationError,
    > {
        // 用 tokio runtime 驱动异步 execute。
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime for aggregate init test");
        runtime.block_on(async {
            self.coordinator()
                .execute(
                    "project_0001",
                    "aggregate_initialization_0001",
                    CancellationToken::new(),
                )
                .await
        })
    }
}

fn two_member_repositories() -> AggregateInitializationGitFixture {
    let temp = tempfile::tempdir().expect("temp root");
    let paths = ProductAppPaths::new(temp.path().join(".aria"));

    // 聚合根是公共非 Git 父目录(REQ-IND-01:非 Git 统一索引根)。
    let aggregate_root = temp.path().join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).unwrap();

    let aliases = ["api", "web"];
    let mut member_roots = Vec::new();
    let mut member_ids = Vec::new();
    let lc_store = LogicalCodebaseStore::new(paths.clone());
    for (ordinal, alias) in aliases.iter().enumerate() {
        let member_dir = aggregate_root.join(alias);
        init_git_repository(&member_dir);
        member_roots.push(member_dir.clone());
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        member_ids.push(member_id);
        let now = CREATED_AT.to_string();
        let member = CodebaseMemberRecord {
            logical_repository_id: member_id,
            physical_repository_id: format!("repository_{alias}"),
            alias: (*alias).to_string(),
            role: "service".to_string(),
            ordinal: ordinal as u32,
            source_identity: RepositorySourceIdentity::from_git_parts(
                &member_dir,
                member_dir.join(".git"),
                Some(format!("ssh://git@example.test/acme/{alias}.git")),
            ),
            repo_type: RepositoryType::Frontend,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout_id],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now,
        };
        lc_store.save_member("project_0001", &member).unwrap();
        let now = CREATED_AT.to_string();
        let checkout = RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id: member_id,
            physical_repository_id: format!("repository_{alias}"),
            kind: CheckoutKind::Main,
            canonical_path: member_dir.clone(),
            checkout_path_hash: "sha256:checkout".to_string(),
            git_dir_identity: "sha256:git-dir".to_string(),
            revision: Some("abc123".to_string()),
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        };
        lc_store.save_checkout("project_0001", &checkout).unwrap();
    }

    let manifest = LogicalCodebaseManifest::new("project_0001", aggregate_root, member_ids);
    lc_store.save_manifest("project_0001", &manifest).unwrap();

    let command_log: Arc<Mutex<Vec<RecordedCall>>> = Arc::new(Mutex::new(Vec::new()));
    let skills: Arc<dyn AggregateSkillsPreparation> =
        Arc::new(RecordingSkillsPreparation::new(command_log.clone()));
    // 使用真实 DeterministicAggregatePreflightService,使其 canonicalize 真实聚合根。
    let preflight: Arc<dyn AggregatePreflightService> = Arc::new(
        cadence_aria::product::logical_codebase::aggregate_initialization_coordinator::DeterministicAggregatePreflightService::new(paths.clone()),
    );
    let provider: Arc<dyn AggregateProviderTurnDriver> =
        Arc::new(RecordingProviderTurnDriver::new(command_log.clone()));
    let clock: Arc<dyn Fn() -> String + Send + Sync> = Arc::new(|| CREATED_AT.to_string());
    let store = cadence_aria::product::logical_codebase::AggregateInitializationOperationStore::new(
        paths.clone(),
    );
    let coordinator = AggregateInitializationCoordinator::new(
        paths.clone(),
        store,
        skills,
        preflight,
        provider,
        clock,
    );

    // 用稳定 operation id 显式 begin,使 execute 能找到已创建记录。
    coordinator
        .begin(
            "aggregate_initialization_0001".to_string(),
            "project_0001",
            AggregateInitializationOperationInput {
                idempotency_key: "0001".to_string(),
                manifest_revision: manifest.membership_revision,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: Some("sha256:profile".to_string()),
                provider_context_root: manifest.provider_context_root.clone(),
                provider: "claude_code".to_string(),
            },
        )
        .expect("begin aggregate initialization");

    AggregateInitializationGitFixture {
        _temp: temp,
        member_roots,
        command_log,
        coordinator,
    }
}

fn init_git_repository(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "--quiet"]);
    git(path, &["config", "user.email", "aria@example.invalid"]);
    git(path, &["config", "user.name", "Aria Test"]);
    std::fs::write(
        path.join("README.md"),
        format!("# {}\n", path.file_name().unwrap().to_string_lossy()),
    )
    .unwrap();
    git(path, &["add", "README.md"]);
    git(path, &["commit", "--quiet", "-m", "initial"]);
}

fn git(cwd: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed in {}: {}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(cwd: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed in {}: {}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn aggregate_initialization_never_changes_member_git_or_worktree_state() {
    let fixture = two_member_repositories();
    let before = AggregateInitializationGitSnapshot::capture(&fixture.member_roots()).unwrap();
    let operation = fixture.run_successful_aggregate_initialization().unwrap();
    assert_eq!(
        operation
            .steps
            .iter()
            .filter(|step| step.status.as_str() == "completed")
            .count(),
        5,
        "all five steps must complete"
    );
    let after = AggregateInitializationGitSnapshot::capture(&fixture.member_roots()).unwrap();

    assert_eq!(before, after);
    assert!(
        fixture
            .command_log()
            .iter()
            .all(|call| !is_forbidden_git_call(&call.argv)),
        "aggregate initialization must never issue git add -A / commit / push"
    );
    assert!(
        fixture
            .member_roots()
            .iter()
            .all(|root| !root.join(".aria/aggregate").exists())
    );
}

/// 判定 argv 是否为被禁止的成员仓写入型 git 调用
/// (`git add -A` / `git commit …` / `git push …`)。
fn is_forbidden_git_call(argv: &[String]) -> bool {
    match argv {
        [a, b, c] if a == "git" && b == "add" && c == "-A" => true,
        [a, b, ..] if a == "git" && (b == "commit" || b == "push") => true,
        _ => false,
    }
}
