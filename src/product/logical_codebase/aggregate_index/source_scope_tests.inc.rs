// 片3「四态判定接入验收」红→绿测试（本文件是 `source_scope_tests` 模块体，
// 由 operation.rs 的 `#[cfg(test)] mod source_scope_tests { include!(...) }`
// 引入，故不写 `mod` 包裹、不写 `//!`）。
//
// 覆盖：四值判定接入验收入口、失败主码优先级、覆盖键缺失按「无法确认」、
// 空索引归一两分支（通知 + 合法空清单）、成功世代才落跳过事实、负向排除
// 查询不放松、预算/可见范围注入点直调。成员目录用 /tmp 临时目录；可见
// 范围默认走生产 `GitVisibleFiles`（非 git 目录 ⇒ 回落全部文件），git
// 可见范围语义用例显式 `git init` 真实仓（不提交、不需要身份配置）。
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use uuid::Uuid;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};

use super::super::codegraph_coverage::{AggregateCoverageProbe, CoverageOutcome};
use super::super::source_scan::{MemberSourceVisibility, SourceScanBudget};
use super::super::types::{AggregateIndexMemberSkipReason, AggregateIndexMemberSnapshot};
use super::*;

// ── 测试辅助（本模块私有，不进生产代码）──

/// 覆盖探针桩：脚本化 `member_covered`（可缺键制造「覆盖状态未知」）。
struct ScriptedCoverage {
    member_covered: BTreeMap<String, bool>,
}

impl AggregateCoverageProbe for ScriptedCoverage {
    fn coverage(
        &self,
        _root: &Path,
        _member_names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError> {
        Ok(CoverageOutcome {
            member_covered: self.member_covered.clone(),
            // 健康字段给健康值：本片只关心覆盖判定，不触发健康闸 warning。
            stats: Some(serde_json::json!({"fileCount": 2})),
            index: Some(serde_json::json!({"state": "complete", "pending": 0, "stale": false})),
        })
    }
}

/// 可见范围桩：直调 `cross_verify_uncovered` 用，不触真实 git。
struct ScriptedVisibility {
    files: Result<Vec<PathBuf>, AggregateIndexError>,
}

impl MemberSourceVisibility for ScriptedVisibility {
    fn visible_files(&self, _member_root: &Path, _budget: &SourceScanBudget<'_>) -> Result<Vec<PathBuf>, AggregateIndexError> {
        self.files.clone()
    }
}

/// /tmp 聚合根 + api/web 两个成员 + 脚本化 codegraph/git 桩。覆盖探针默认
/// 走 CLI files 回退（与既有 DI 构造一致），需要 SDK 判定时注入桩。
struct SourceScopeFixture {
    _temp: tempfile::TempDir,
    paths: ProductAppPaths,
    root: PathBuf,
    logical: LogicalCodebaseStore,
    store: AggregateIndexStore,
    cli: Arc<SourceScopeRunner>,
    api_checkout: RepositoryCheckoutId,
    web_checkout: RepositoryCheckoutId,
}

impl SourceScopeFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("aggregate");
        std::fs::create_dir_all(root.join("api/src")).unwrap();
        std::fs::create_dir_all(root.join("web/src")).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let logical = LogicalCodebaseStore::new(paths.clone());
        let store = AggregateIndexStore::new(paths.clone());
        let api = member_with_checkout(&root, "api", "a".repeat(40));
        let web = member_with_checkout(&root, "web", "b".repeat(40));
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            root.clone(),
            vec![api.0.logical_repository_id, web.0.logical_repository_id],
        );
        manifest.membership_revision = 3;
        logical.save_manifest("project_0001", &manifest).unwrap();
        logical.save_member("project_0001", &api.0).unwrap();
        logical.save_member("project_0001", &web.0).unwrap();
        logical.save_checkout("project_0001", &api.1).unwrap();
        logical.save_checkout("project_0001", &web.1).unwrap();
        Self {
            _temp: temp,
            root,
            paths,
            logical,
            store,
            cli: Arc::new(SourceScopeRunner::default()),
            api_checkout: api.1.checkout_id,
            web_checkout: web.1.checkout_id,
        }
    }

    fn operation(&self) -> AggregateIndexOperation {
        AggregateIndexOperation::with_snapshot_dependencies(
            self.logical.clone(),
            self.store.clone(),
            CodeGraphCli::new(self.cli.clone(), "codegraph".to_string()),
            CodeGraphExcludeGenerator,
            AggregateIndexSnapshotCollector::with_dependencies(
                self.logical.clone(),
                self.cli.clone(),
            ),
        )
    }

    /// 负向排除查询全部放行（默认空结果）；单测可再覆盖个别查询。
    fn allow_all_exclusion_queries(&self) {
        for query in EXCLUDED_QUERIES {
            self.cli.query_returns(query, serde_json::json!([]));
        }
    }

    fn records(&self) -> Vec<AggregateIndexRecord> {
        let root = self.paths.aggregate_indexes_root("project_0001");
        let mut records = std::fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .map(|bytes| serde_json::from_slice(&bytes).unwrap())
            .collect::<Vec<AggregateIndexRecord>>();
        records.sort_by(|left, right| left.aggregate_index_id.cmp(&right.aggregate_index_id));
        records
    }
}

/// 成员/主检出记录（与 operation_tests 的夹具同构，收窄为本模块私有）。
fn member_with_checkout(
    root: &Path,
    name: &str,
    revision: String,
) -> (CodebaseMemberRecord, RepositoryCheckoutRecord) {
    let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
    let canonical_path = root.join(name);
    let now = "2026-08-09T00:00:00Z".to_string();
    let source_identity = RepositorySourceIdentity {
        scheme: "test".to_string(),
        key_digest: format!("sha256:source-{name}"),
        canonical_git_dir: canonical_path.join(".git"),
        canonical_origin: None,
        first_seen_path_hash: format!("sha256:path-{name}"),
    };
    let member = CodebaseMemberRecord {
        logical_repository_id,
        physical_repository_id: format!("repository_{name}"),
        alias: name.to_string(),
        role: "repository".to_string(),
        ordinal: 1,
        source_identity: source_identity.clone(),
        repo_type: RepositoryType::Unknown,
        tech_stack: Vec::new(),
        owner: None,
        tags: Vec::new(),
        default_ref: None,
        checkout_ids: vec![checkout_id],
        status: MemberStatus::Active,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let checkout = RepositoryCheckoutRecord {
        checkout_id,
        logical_repository_id,
        physical_repository_id: member.physical_repository_id.clone(),
        kind: CheckoutKind::Main,
        canonical_path,
        checkout_path_hash: format!("sha256:checkout-{name}"),
        git_dir_identity: source_identity.git_dir_identity(),
        revision: Some(revision),
        availability: CheckoutAvailability::Available,
        observed_at: now.clone(),
        created_at: now.clone(),
        updated_at: now,
    };
    (member, checkout)
}

/// 按主检出 ID 取成员快照（快照顺序与 manifest 同序，但断言按身份定位）。
fn member_snapshot<'a>(
    record: &'a AggregateIndexRecord,
    checkout: &RepositoryCheckoutId,
) -> &'a AggregateIndexMemberSnapshot {
    record
        .member_snapshots
        .iter()
        .find(|snapshot| snapshot.checkout_id == *checkout)
        .expect("member snapshot must exist in the generation record")
}

/// 真实 git 成员仓（可见范围语义用例）：`git init` + `.gitignore`，不提交
/// （`ls-files --cached --others --exclude-standard` 对未提交仓同样成立）。
fn git_init_member(root: &Path, name: &str, gitignore: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".gitignore"), gitignore).unwrap();
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&dir)
        .status()
        .unwrap();
    assert!(status.success(), "git init failed for fixture member {name}");
    dir
}

/// codegraph/git 命令桩：files 清单与空索引通知可脚本化；快照采集的 git
/// 请求按成员目录名返回稳定 revision（不漂移）。
#[derive(Default)]
struct SourceScopeRunner {
    state: parking_lot::Mutex<SourceScopeState>,
}

#[derive(Default)]
struct SourceScopeState {
    files: Vec<PathBuf>,
    /// 复刻 codegraph 1.6 契约缺陷：空索引（0 文件）时 `files --json`
    /// 以 exit 0 输出人读提示而非 JSON 数组。
    files_empty_index_notice: bool,
    queries: BTreeMap<String, serde_json::Value>,
    /// `codegraph --version` 的实测输出（空串＝源码后缀清单的校准基线
    /// 1.6.3；单测可改写以模拟版本漂移）。
    version: String,
}

impl SourceScopeRunner {
    fn files_return<I, P>(&self, paths: I)
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut state = self.state.lock();
        state.files_empty_index_notice = false;
        state.files = paths
            .into_iter()
            .map(|path| path.as_ref().to_path_buf())
            .collect();
    }

    /// README-only 空仓形态：init 建出 0 文件库，`files --json` 输出
    /// 上游人读提示（非法 JSON）。
    fn files_return_empty_index_notice(&self) {
        let mut state = self.state.lock();
        state.files_empty_index_notice = true;
        state.files.clear();
    }

    fn query_returns(&self, query: &str, value: serde_json::Value) {
        self.state.lock().queries.insert(query.to_string(), value);
    }

    /// 改写 `codegraph --version` 的实测输出（模拟版本漂移）。
    fn set_version(&self, version: &str) {
        self.state.lock().version = version.to_string();
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for SourceScopeRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let state = self.state.lock();
        // 快照采集的 git 请求带受信禁桥前缀（见 snapshot.rs 生产 git()）：
        // 先断言并剥离，再按同一 argv 匹配面分发。
        let bridge = crate::cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS;
        let bridge_len = bridge.len();
        let argv: &[String] = if request.executable == "git" {
            assert!(
                request.argv.len() > bridge_len
                    && request.argv[..bridge_len]
                        .iter()
                        .zip(bridge.iter())
                        .all(|(actual, expected)| actual == expected),
                "git argv missing trusted bridge prefix: {:?}",
                request.argv
            );
            &request.argv[bridge_len..]
        } else {
            request.argv.as_slice()
        };
        let stdout = match argv {
            [version_arg] if version_arg == "--version" => {
                // 未显式改写时＝源码后缀清单的校准基线版本（无漂移）。
                let reported = match state.version.as_str() {
                    "" => "1.6.3",
                    other => other,
                };
                format!("{reported}\n")
            }
            [rev_parse, head] if rev_parse == "rev-parse" && head == "HEAD" => match request
                .working_dir
                .file_name()
                .and_then(|name| name.to_str())
            {
                Some("api") => "a".repeat(40),
                Some("web") => "b".repeat(40),
                Some(name) if name.starts_with("synthetic_") => "c".repeat(40),
                name => panic!("unexpected fake Git checkout: {name:?}"),
            },
            [status, porcelain] if status == "status" && porcelain == "--porcelain=v1" => {
                String::new()
            }
            [init, dot] if init == "init" && dot == "." => "Indexed 2 files\n".to_string(),
            [sync, dot] if sync == "sync" && dot == "." => "Synced 2 files\n".to_string(),
            [files, json] if files == "files" && json == "--json" => {
                if state.files_empty_index_notice {
                    "ℹ No files indexed. Run \"codegraph index\" first.".to_string()
                } else {
                    serde_json::to_string(
                        &state
                            .files
                            .iter()
                            .map(|path| serde_json::json!({"path": path}))
                            .collect::<Vec<_>>(),
                    )
                    .unwrap()
                }
            }
            [query, symbol, json] if query == "query" && json == "--json" => state
                .queries
                .get(symbol)
                .cloned()
                .unwrap_or_else(|| serde_json::json!([]))
                .to_string(),
            argv => panic!("unexpected fake CodeGraph argv: {argv:?}"),
        };
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }
}

// ── 红测试清单（对应 Plan §5.3）──

/// ① 混合世代：A 被覆盖、B 仅配置文件（盘检确认无源码）⇒ 激活，
/// B 在**本世代**快照上记跳过事实（included 语义不动）。
#[test]
fn mixed_lc_activates_and_records_skip_for_source_less_member() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::write(fixture.root.join("web/limits.json"), "{}\n").unwrap();
    fixture.cli.files_return(["api/src/A.rs"]);
    fixture.allow_all_exclusion_queries();

    let record = fixture.operation().build("project_0001", 3).unwrap();
    assert_eq!(record.status, AggregateIndexStatus::Active);
    let web = member_snapshot(&record, &fixture.web_checkout);
    assert_eq!(
        web.skip_reason,
        Some(AggregateIndexMemberSkipReason::NoSource)
    );
    assert!(web.included, "included 语义不动：跳过成员仍在快照里");
    assert_eq!(
        member_snapshot(&record, &fixture.api_checkout).skip_reason,
        None,
        "被覆盖成员不得有跳过标记"
    );
    // 构建期观测快照不是验收结论，不打标记。
    let observed = record
        .observed_after_member_snapshots
        .iter()
        .find(|snapshot| snapshot.checkout_id == fixture.web_checkout)
        .expect("observed-after snapshot must exist");
    assert_eq!(observed.skip_reason, None);
}

/// ② 普通路径有源码却未被收录 ⇒ 复用覆盖失败码的**磁盘事实变体**；
/// 文案不得说用户没放源码（不得出现旧事实句）。
#[test]
fn source_in_normal_path_fails_with_disk_source_variant_message() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::write(fixture.root.join("web/src/B.rs"), "fn b() {}\n").unwrap();
    fixture.cli.files_return(["api/src/A.rs"]);
    fixture.allow_all_exclusion_queries();

    let error = fixture.operation().build("project_0001", 3).unwrap_err();
    let AggregateIndexError::Failed { code, message } = &error else {
        panic!("expected failed error, got: {error}");
    };
    assert_eq!(code, &"aggregate_index_member_coverage_failed");
    assert!(
        message.starts_with("source found on disk; no indexed files under included members [web]"),
        "disk-source fact must name web, got: {message}"
    );
    assert!(
        !message.contains("no indexable code file"),
        "must not claim the member has no source on disk: {message}"
    );
}

/// ③ 源码只在被排除的 `.worktrees/` 子树 ⇒ 专属失败码（第二段诊断）。
#[test]
fn source_only_in_worktrees_fails_with_dedicated_code() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::create_dir_all(fixture.root.join("web/.worktrees/feat/src")).unwrap();
    std::fs::write(
        fixture.root.join("web/.worktrees/feat/src/b.rs"),
        "fn b() {}\n",
    )
    .unwrap();
    fixture.cli.files_return(["api/src/A.rs"]);
    fixture.allow_all_exclusion_queries();

    let error = fixture.operation().build("project_0001", 3).unwrap_err();
    let AggregateIndexError::Failed { code, message } = &error else {
        panic!("expected failed error, got: {error}");
    };
    assert_eq!(code, &"aggregate_index_member_source_excluded");
    assert!(
        message.contains("included members [web]"),
        "must name the worktree-only member, got: {message}"
    );
}

/// ④+⑨ 盘检无法确认 ⇒ 无法确认码（预算注入点直调，不依赖真实 sleep）；
/// 文案不得断言「无源码」，也不得复用覆盖失败码的旧事实句。
#[test]
fn scan_incomplete_fails_without_claiming_no_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("aggregate");
    let names = vec!["api".to_string(), "web".to_string()];
    let covered = BTreeMap::from([("api".to_string(), true), ("web".to_string(), false)]);
    // 注入口可用性：`verify` 是关联函数拿不到注入成员，测试经同模块直调
    // `cross_verify_uncovered` 注入耗尽预算（条目 0 ⇒ 入口即无法确认）。
    let mut exhausted = SourceScanBudget::for_tests(0, None);
    let error = super::cross_verify_uncovered(
        &root,
        &covered,
        &names,
        &ScriptedVisibility {
            files: Ok(Vec::new()),
        },
        &mut exhausted,
    )
    .1
    .unwrap_err();
    let AggregateIndexError::Failed { code, message } = &error else {
        panic!("expected failed error, got: {error}");
    };
    assert_eq!(code, &"aggregate_index_source_scan_incomplete");
    assert!(
        message.contains("cannot confirm absence of source"),
        "must state that absence is unconfirmed, got: {message}"
    );
    assert!(
        !message.contains("no indexable code file"),
        "must not claim the member has no source: {message}"
    );
    // git 侧失败（可见范围探测报错）同样落「无法确认」，不得当无源码。
    let mut fresh = SourceScanBudget::for_tests(100, None);
    let failing = ScriptedVisibility {
        files: Err(AggregateIndexError::Failed {
            code: "aggregate_index_source_scan_git_failed",
            message: "git unavailable".to_string(),
        }),
    };
    let error =
        super::cross_verify_uncovered(&root, &covered, &names, &failing, &mut fresh).1.unwrap_err();
    assert!(
        matches!(&error, AggregateIndexError::Failed { code, .. }
            if *code == "aggregate_index_source_scan_incomplete"),
        "git-side failure must stay unconfirmed, got: {error}"
    );
}

/// ⑤ 覆盖键缺失 ⇒ 按无法确认处理（R1）：不得当「覆盖不足」，也不得当
/// 「已确认无源码」；即便盘上有源码也不得进漏收桶。
#[test]
fn missing_coverage_key_is_not_treated_as_no_source() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    // web 盘上有源码：若实现把缺键当「未覆盖」去盘检会误报漏收——缺键
    // 连覆盖状态都不知道，必须先于盘检失败。
    std::fs::write(fixture.root.join("web/src/B.rs"), "fn b() {}\n").unwrap();
    fixture.allow_all_exclusion_queries();
    let operation = fixture.operation().with_coverage_probe(Arc::new(ScriptedCoverage {
        member_covered: BTreeMap::from([("api".to_string(), true)]),
    }));

    let error = operation.build("project_0001", 3).unwrap_err();
    let AggregateIndexError::Failed { code, message } = &error else {
        panic!("expected failed error, got: {error}");
    };
    assert_eq!(
        code, &"aggregate_index_source_scan_incomplete",
        "missing coverage key is unknown coverage state, not coverage failure"
    );
    assert!(
        message.contains("coverage state unknown for included members [web]"),
        "must report the unknown coverage state, got: {message}"
    );
}

/// ⑥ 全员确认无源码 ⇒ 全员无源码码；不发布 active、不落跳过事实。
#[test]
fn all_members_source_less_fails_with_all_members_code() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/README.md"), "config only\n").unwrap();
    std::fs::write(fixture.root.join("web/limits.json"), "{}\n").unwrap();
    fixture.allow_all_exclusion_queries();
    let operation = fixture.operation().with_coverage_probe(Arc::new(ScriptedCoverage {
        member_covered: BTreeMap::from([
            ("api".to_string(), false),
            ("web".to_string(), false),
        ]),
    }));

    let error = operation.build("project_0001", 3).unwrap_err();
    let AggregateIndexError::Failed { code, message } = &error else {
        panic!("expected failed error, got: {error}");
    };
    assert_eq!(code, &"aggregate_index_all_members_source_less");
    assert!(
        message.contains("included members [api, web]"),
        "must name every source-less member in manifest order, got: {message}"
    );
    assert_eq!(
        fixture.store.active("project_0001").unwrap(),
        None,
        "no active record may be published for an all-source-less project"
    );
    let records = fixture.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, AggregateIndexStatus::Failed);
    assert!(
        records[0]
            .member_snapshots
            .iter()
            .all(|snapshot| snapshot.skip_reason.is_none()),
        "failed generations never carry skip facts"
    );
}

/// ⑦a CLI 空索引**通知** + 磁盘无源码 ⇒ 归一为全员未覆盖后进交叉验收 ⇒
/// 全员无源码码（不得落覆盖失败码）。
#[test]
fn cli_empty_index_with_no_source_anywhere_fails_with_all_members_code() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/README.md"), "config only\n").unwrap();
    std::fs::write(fixture.root.join("web/limits.json"), "{}\n").unwrap();
    fixture.cli.files_return_empty_index_notice();
    fixture.allow_all_exclusion_queries();

    let error = fixture.operation().build("project_0001", 3).unwrap_err();
    let AggregateIndexError::Failed { code, message } = &error else {
        panic!("expected failed error, got: {error}");
    };
    assert_eq!(code, &"aggregate_index_all_members_source_less");
    assert!(
        message.contains("api") && message.contains("web"),
        "must name both members, got: {message}"
    );
    assert!(
        !message.contains("no indexable code file"),
        "must not use the legacy coverage-failure fact, got: {message}"
    );
    assert_eq!(fixture.store.active("project_0001").unwrap(), None);
}

/// ⑦b CLI 空索引**通知** + 某成员盘上有源码 ⇒ 磁盘事实变体（两分支
/// 分别断言，不得混淆成全员无源码）。
#[test]
fn cli_empty_index_with_source_on_disk_fails_with_disk_source_variant() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::write(fixture.root.join("web/limits.json"), "{}\n").unwrap();
    fixture.cli.files_return_empty_index_notice();
    fixture.allow_all_exclusion_queries();

    let error = fixture.operation().build("project_0001", 3).unwrap_err();
    let AggregateIndexError::Failed { code, message } = &error else {
        panic!("expected failed error, got: {error}");
    };
    assert_eq!(code, &"aggregate_index_member_coverage_failed");
    assert!(
        message.starts_with(
            "source found on disk; no indexed files under included members [api]"
        ),
        "empty index must not mask disk source, got: {message}"
    );
}

/// ⑦c CLI **合法空清单**（`[]`，非通知）同样进交叉验收，不早退。
#[test]
fn cli_legal_empty_file_list_enters_cross_verification() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::write(fixture.root.join("web/limits.json"), "{}\n").unwrap();
    fixture.cli.files_return(Vec::<&str>::new());
    fixture.allow_all_exclusion_queries();

    let error = fixture.operation().build("project_0001", 3).unwrap_err();
    let AggregateIndexError::Failed { code, message } = &error else {
        panic!("expected failed error, got: {error}");
    };
    assert_eq!(code, &"aggregate_index_member_coverage_failed");
    assert!(
        message.starts_with(
            "source found on disk; no indexed files under included members [api]"
        ),
        "legal empty list must reach cross verification, got: {message}"
    );
}

/// ⑧ 失败世代不产生跳过事实；上一代 LKG 的跳过事实不被改写；上一代
/// 跳过的成员本次可见范围内出现源码 ⇒ 重新判定（不继承上一代结论）。
#[test]
fn failed_generation_does_not_write_skip_reason_and_keeps_last_known_good() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::write(fixture.root.join("web/limits.json"), "{}\n").unwrap();
    fixture.cli.files_return(["api/src/A.rs"]);
    fixture.allow_all_exclusion_queries();
    let active = fixture.operation().build("project_0001", 3).unwrap();
    assert_eq!(active.status, AggregateIndexStatus::Active);
    assert_eq!(
        member_snapshot(&active, &fixture.web_checkout).skip_reason,
        Some(AggregateIndexMemberSkipReason::NoSource)
    );

    // 上一代跳过的成员在可见范围内新增源码、索引仍漏收 ⇒ 漏收失败。
    std::fs::write(fixture.root.join("web/src/B.rs"), "fn b() {}\n").unwrap();
    let error = fixture.operation().rebuild("project_0001").unwrap_err();
    assert!(
        matches!(&error, AggregateIndexError::Failed { code, .. }
            if *code == "aggregate_index_member_coverage_failed"),
        "new visible source must be rejudged, not inherited as skip: {error}"
    );

    let records = fixture.records();
    assert_eq!(records.len(), 2);
    assert_eq!(
        fixture.store.active("project_0001").unwrap(),
        None,
        "failed rebuild must not publish a new active record"
    );
    let failed = records
        .iter()
        .find(|record| record.status == AggregateIndexStatus::Stale)
        .expect("rebuild failure marks the fresh generation stale");
    assert!(
        failed
            .member_snapshots
            .iter()
            .all(|snapshot| snapshot.skip_reason.is_none()),
        "failed generations never carry skip facts"
    );
    // LKG（rebuild 失败被降级）保留它自己那一代的跳过事实。
    let lkg = records
        .iter()
        .find(|record| record.status == AggregateIndexStatus::Degraded)
        .expect("last-known-good is degraded, not deleted");
    assert_eq!(
        member_snapshot(lkg, &fixture.web_checkout).skip_reason,
        Some(AggregateIndexMemberSkipReason::NoSource)
    );
}

/// ⑩ 交叉验收通过（含跳过成员）不放松负向排除查询。
#[test]
fn exclusion_queries_not_relaxed_for_skipped_members() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::write(fixture.root.join("web/limits.json"), "{}\n").unwrap();
    fixture.cli.files_return(["api/src/A.rs"]);
    fixture.allow_all_exclusion_queries();
    fixture.cli.query_returns(
        "SHOULD_NOT_INDEX_WORKTREE",
        serde_json::json!([{"file": "web/.worktrees/feat/src/leak.rs"}]),
    );

    let error = fixture.operation().build("project_0001", 3).unwrap_err();
    let AggregateIndexError::Failed { code, message } = &error else {
        panic!("expected failed error, got: {error}");
    };
    assert_eq!(code, &"aggregate_index_exclusion_failed");
    assert!(
        message.contains("web/.worktrees/feat/src/leak.rs"),
        "exclusion evidence must surface the offending path, got: {message}"
    );
}

/// G3（真实 git 仓）：被 `.gitignore` 忽略的源码不算源码——仅有被忽略
/// 源码的成员按「确认无源码」跳过，不得报漏收。
#[test]
fn ignored_only_source_member_is_skipped_not_reported_missing() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    git_init_member(&fixture.root, "web", "sandbox/\n");
    std::fs::create_dir_all(fixture.root.join("web/sandbox")).unwrap();
    std::fs::write(
        fixture.root.join("web/sandbox/ignored.ts"),
        "export const ignored = 1;\n",
    )
    .unwrap();
    fixture.cli.files_return(["api/src/A.rs"]);
    fixture.allow_all_exclusion_queries();

    let record = fixture.operation().build("project_0001", 3).unwrap();
    assert_eq!(record.status, AggregateIndexStatus::Active);
    assert_eq!(
        member_snapshot(&record, &fixture.web_checkout).skip_reason,
        Some(AggregateIndexMemberSkipReason::NoSource),
        "ignored-only source member must be skipped, not reported as missing coverage"
    );
}

/// 混合激活走**非空覆盖**用例：SDK 探针报 A 已覆盖、B 未覆盖 ⇒ 激活 +
/// B 记跳过（激活不依赖空索引归一路径）。
#[test]
fn mixed_activation_uses_non_empty_coverage_case() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::write(fixture.root.join("web/limits.json"), "{}\n").unwrap();
    fixture.cli.files_return(["api/src/A.rs"]);
    fixture.allow_all_exclusion_queries();
    let operation = fixture.operation().with_coverage_probe(Arc::new(ScriptedCoverage {
        member_covered: BTreeMap::from([
            ("api".to_string(), true),
            ("web".to_string(), false),
        ]),
    }));

    let record = operation.build("project_0001", 3).unwrap();
    assert_eq!(record.status, AggregateIndexStatus::Active);
    assert_eq!(
        member_snapshot(&record, &fixture.web_checkout).skip_reason,
        Some(AggregateIndexMemberSkipReason::NoSource)
    );
}

/// 版本漂移软告警（校准基线一致）：实测 codegraph 版本 == 源码后缀清单的
/// 校准基线 1.6.3 ⇒ 不记漂移告警，世代照常 Active。
#[test]
fn calibrated_codegraph_version_records_no_drift_warning() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::write(fixture.root.join("web/src/B.rs"), "fn b() {}\n").unwrap();
    fixture.cli.files_return(["api/src/A.rs", "web/src/B.rs"]);
    fixture.allow_all_exclusion_queries();
    fixture.cli.set_version("1.6.3");

    let record = fixture.operation().build("project_0001", 3).unwrap();
    assert_eq!(record.status, AggregateIndexStatus::Active);
    let warning = record.warning.clone().unwrap_or_default();
    assert!(
        !warning.contains("版本漂移"),
        "校准基线版本不得触发漂移告警, got: {warning}"
    );
}

/// 版本漂移软告警（漂移）：实测 codegraph 版本 ≠ 校准基线 ⇒ 记一条可审计
/// warning（含实测版本、校准基线版本与复校准指引），**不阻断索引构建**
/// （世代仍 Active）。
#[test]
fn codegraph_version_drift_records_auditable_warning_without_blocking() {
    let fixture = SourceScopeFixture::new();
    std::fs::write(fixture.root.join("api/src/A.rs"), "fn a() {}\n").unwrap();
    std::fs::write(fixture.root.join("web/src/B.rs"), "fn b() {}\n").unwrap();
    fixture.cli.files_return(["api/src/A.rs", "web/src/B.rs"]);
    fixture.allow_all_exclusion_queries();
    fixture.cli.set_version("1.7.0");

    let record = fixture.operation().build("project_0001", 3).unwrap();
    assert_eq!(
        record.status,
        AggregateIndexStatus::Active,
        "版本漂移只记软告警，不得阻断索引构建"
    );
    let warning = record.warning.clone().unwrap_or_default();
    assert!(
        warning.contains("版本漂移"),
        "必须留下可审计的版本漂移告警, got: {warning}"
    );
    assert!(
        warning.contains("1.7.0"),
        "告警须含实测 codegraph 版本, got: {warning}"
    );
    assert!(
        warning.contains("1.6.3"),
        "告警须含源码后缀清单的校准基线版本, got: {warning}"
    );
    assert!(
        warning.contains("复校准"),
        "告警须含复校准指引, got: {warning}"
    );
}

include!("scan_timing_tests.inc.rs");
