//! 片12（Plan §18）：聚合索引源码范围判定 + Story 前刷新的端到端集成。
//!
//! 只走真实 HTTP 路由 + 真实注入点（片3 `with_coverage_probe`、片7W
//! `with_aggregate_index_operation` / `with_evidence_command_runner`）；夹具为
//! `/tmp` 下真实 git 混合仓。全部走假 CodeGraph CLI，不触碰
//! `CODEGRAPH_FIXTURE_LOCK`。断言只用机器码与既有 DTO 字段，不自造文案。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::http::{Method, StatusCode};
use cadence_aria::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_models::{
    AttemptTargetSnapshot, CodingAdmissionKind, CodingAttemptScope, CodingAttemptStatus,
    CodingExecutionAttempt, CodingExecutionStage,
};
use cadence_aria::product::json_store::write_json;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::logical_codebase::LogicalCodebaseStore;
use cadence_aria::product::logical_codebase::aggregate_index::{
    AggregateCoverageProbe, AggregateIndexError, AggregateIndexOperation, AggregateIndexRecord,
    AggregateIndexSnapshotCollector, AggregateIndexStatus, AggregateIndexStore, CodeGraphCli,
    CodeGraphExcludeGenerator, CoverageOutcome,
};
use cadence_aria::product::logical_codebase::evidence_token::issue_evidence_token;
use cadence_aria::product::logical_codebase::issue_selection::IssueCodebaseSelectionStore;
use cadence_aria::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use cadence_aria::product::logical_codebase::store::legacy_logical_codebase_id;
use cadence_aria::product::models::ProviderName;
use cadence_aria::web::app::{build_web_router, build_web_router_with_evidence};
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

use crate::aggregate_index_fixture::{FAKE_CODEGRAPH_EXECUTABLE, FakeCodeGraphCli};
use crate::web_coding_attempt_api::{register_repository_and_wait, request_json, run_git};

const PROJECT_ID: &str = "project_0001";
const NOW: &str = "2026-11-18T00:00:00Z";

// —— 夹具：可翻转覆盖探针 + 真实 git 混合仓 LC（HTTP 注册 + 身份迁移）——

/// 成员规格：聚合根下目录名、仓库名（登记后即成员别名）、是否含 Aria 源码。
struct MemberSpec {
    dir: &'static str,
    alias: &'static str,
    source: bool,
}

/// 有源码成员（普通路径提交 `src/main.rs`）。
const SOURCE_MEMBER: MemberSpec = MemberSpec {
    dir: "source",
    alias: "source-app",
    source: true,
};
/// 配置成员：只有 `.json` / `.md`，不含任何 Aria 源码后缀。
const CONFIG_MEMBER: MemberSpec = MemberSpec {
    dir: "config",
    alias: "config-only",
    source: false,
};

/// 可翻转覆盖探针（片3 `with_coverage_probe` 注入点）：按磁盘事实报告覆盖
/// （有 `src/main.rs` ⇒ 覆盖）。`force_uncovered` 置位后把有源码成员报为
/// 未覆盖 ⇒ 构造 `aggregate_index_member_coverage_failed`（有源码却未收录）。
#[derive(Default)]
struct FlipCoverageProbe {
    force_uncovered: AtomicBool,
}

impl FlipCoverageProbe {
    fn force(&self) {
        self.force_uncovered.store(true, Ordering::SeqCst);
    }
}

impl AggregateCoverageProbe for FlipCoverageProbe {
    fn coverage(
        &self,
        root: &Path,
        member_names: &[String],
    ) -> Result<CoverageOutcome, AggregateIndexError> {
        let member_covered = member_names
            .iter()
            .map(|name| {
                let covered = root.join(name).join("src").join("main.rs").exists()
                    && !self.force_uncovered.load(Ordering::SeqCst);
                (name.clone(), covered)
            })
            .collect::<BTreeMap<_, _>>();
        Ok(CoverageOutcome {
            member_covered,
            stats: None,
            index: None,
        })
    }
}

/// 真实 git 成员仓：源码成员提交 `src/main.rs`；配置成员只提交配置与文档。
fn seed_member_repo(aggregate_root: &Path, spec: &MemberSpec) -> PathBuf {
    let repo = aggregate_root.join(spec.dir);
    fs::create_dir_all(&repo).expect("member repo dir");
    run_git(&repo, &["init"]);
    if spec.source {
        fs::create_dir_all(repo.join("src")).expect("member src dir");
        fs::write(
            repo.join("src").join("main.rs"),
            "pub fn member_symbol() -> u32 { 1 }\n",
        )
        .expect("member source");
    } else {
        fs::write(repo.join("limits.json"), "{\n  \"max_members\": 2\n}\n").expect("member config");
        fs::write(repo.join("NOTES.md"), "# 配置仓\n").expect("member notes");
    }
    run_git(&repo, &["add", "."]);
    run_git(
        &repo,
        &[
            "-c",
            "user.name=Aria Test",
            "-c",
            "user.email=aria@example.com",
            "commit",
            "-m",
            "initial",
        ],
    );
    repo
}

/// 混合仓 LC 夹具：真实 HTTP 注册 + 身份迁移 + 注入探针的聚合索引操作。
struct LcFixture {
    root: TempDir,
    paths: ProductAppPaths,
    lc_id: String,
    aggregate_root: PathBuf,
    probe: Arc<FlipCoverageProbe>,
    issue_id: String,
}

impl LcFixture {
    /// 每次装配新 state：假 CLI + 共享探针（片3/片7W 注入点）。
    fn state(&self) -> WebAppState {
        let operation = AggregateIndexOperation::with_snapshot_dependencies(
            LogicalCodebaseStore::new(self.paths.clone()),
            AggregateIndexStore::new(self.paths.clone()),
            CodeGraphCli::new(
                Arc::new(FakeCodeGraphCli::new(
                    self.paths.clone(),
                    PROJECT_ID,
                    &self.lc_id,
                )),
                FAKE_CODEGRAPH_EXECUTABLE.to_string(),
            ),
            CodeGraphExcludeGenerator,
            AggregateIndexSnapshotCollector::for_paths(self.paths.clone()),
        )
        .with_coverage_probe(self.probe.clone());
        WebAppState::new(
            self.root.path().to_path_buf(),
            WebRuntime::new_fake(self.root.path().to_path_buf()),
        )
        .with_aggregate_index_operation(Arc::new(operation))
    }

    fn app(&self) -> axum::Router {
        build_web_router(self.state())
    }

    /// 证据路由装配：同 state + 注入计数 runner（片7W/8A 注入点）。
    fn evidence_app(&self, runner: Arc<CountingQueryRunner>) -> axum::Router {
        build_web_router_with_evidence(self.state().with_evidence_command_runner(runner), true)
    }

    fn rebuild_uri(&self) -> String {
        self.lc_uri("/aggregate-indexes/rebuild")
    }
    fn active_uri(&self) -> String {
        self.lc_uri("/aggregate-indexes/active")
    }
    fn bootstrap_uri(&self) -> String {
        self.lc_uri("/bootstrap")
    }
    fn members_uri(&self) -> String {
        self.lc_uri("/members")
    }
    fn lc_uri(&self, suffix: &str) -> String {
        format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{}{suffix}",
            self.lc_id
        )
    }

    async fn rebuild(&self, app: &axum::Router) -> (StatusCode, Value) {
        request_json(app.clone(), Method::POST, &self.rebuild_uri(), json!({})).await
    }

    /// 读 active 投影（恒断言 200，返回体）。
    async fn get_active(&self, app: &axum::Router) -> Value {
        let (status, projection) =
            request_json(app.clone(), Method::GET, &self.active_uri(), json!({})).await;
        assert_eq!(status, StatusCode::OK, "{projection}");
        projection
    }

    /// 当前 active 世代 id（无 ⇒ None）。
    fn active_generation_id(&self) -> Option<String> {
        AggregateIndexStore::for_lc(self.paths.clone(), self.lc_id.clone())
            .active(PROJECT_ID)
            .expect("read durable aggregate-index generations")
            .map(|record| record.aggregate_index_id)
    }

    fn durable_records(&self) -> Vec<AggregateIndexRecord> {
        AggregateIndexStore::for_lc(self.paths.clone(), self.lc_id.clone())
            .records(PROJECT_ID)
            .expect("read durable records")
    }

    /// 经真实 HTTP 生成 Story（请求体照抄既有 web_lifecycle_api 用例）。
    async fn generate_story(
        &self,
        app: &axum::Router,
        issue_id: &str,
        involved: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut body: Value = serde_json::from_str(
            r#"{"title":"聚合 Story Spec","author_provider":"fake","reviewer_provider":"codex","review_rounds":3,"superpowers_enabled":false,"openspec_enabled":false}"#,
        )
        .expect("story request body");
        if let Some(member_id) = involved {
            body["involved_repository_ids"] = json!([member_id]);
        }
        request_json(
            app.clone(),
            Method::POST,
            &format!("/api/projects/{PROJECT_ID}/issues/{issue_id}/story-specs:generate"),
            body,
        )
        .await
    }
}

/// 播种 LC：真实 git 成员仓 → HTTP 注册 → 身份迁移 → bootstrap 政策产物 →
/// 前置多仓 issue（primary=repository_0001 ⇒ all_members selection）。
async fn seed_lc(members: &[MemberSpec]) -> LcFixture {
    let root = tempdir().expect("root");
    let aggregate_root = root.path().join("members");
    fs::create_dir_all(&aggregate_root).expect("aggregate root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lc_id = legacy_logical_codebase_id(PROJECT_ID);
    let fixture = LcFixture {
        root,
        paths: paths.clone(),
        lc_id: lc_id.clone(),
        aggregate_root: aggregate_root.clone(),
        probe: Arc::new(FlipCoverageProbe::default()),
        issue_id: String::new(),
    };
    let app = fixture.app();

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name": "SourceScope", "description": null}),
    )
    .await;
    // 注册顺序即 repository_NNNN 序号（primary=0001、配置仓=0002）。
    for spec in members {
        let repo = seed_member_repo(&aggregate_root, spec);
        register_repository_and_wait(app.clone(), json!({"name": spec.alias, "path": repo})).await;
    }

    // 身份迁移：物理仓 → 同一聚合根下的 LC 权威。
    cadence_aria::product::logical_codebase::IdentityMigrationExecutor::new(paths.clone())
        .ensure_identity_schema(PROJECT_ID)
        .expect("migrate member repositories into logical codebase authority");

    // 规划上下文解析（Story/Design）要求政策 artifact 可解析（缺失即 blocker）。
    let manifest = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone())
        .load_manifest(PROJECT_ID)
        .expect("load manifest")
        .expect("migration wrote the manifest");
    AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id.clone())
        .ensure_bootstrap(&manifest)
        .expect("ensure bootstrap policy artifact");

    // 前置 issue：经真实 HTTP 建（primary=repository_0001）⇒ issue_0001 + all_members。
    let (status, issue) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues"),
        serde_json::from_str(&format!(
            r#"{{"title":"聚合 Issue","description":null,"logical_codebase_id":"{lc_id}","repository_id":"repository_0001"}}"#
        ))
        .expect("issue body"),
    )
    .await;
    assert!(
        (200..300).contains(&status.as_u16()),
        "create primary issue: {status} {issue}"
    );
    LcFixture {
        issue_id: issue["issue_id"].as_str().expect("issue id").to_string(),
        ..fixture
    }
}

/// 读 bootstrap 投影里 aggregate_index_active 步骤。
async fn bootstrap_aggregate_step(app: &axum::Router, uri: &str) -> Value {
    let (status, projection) = request_json(app.clone(), Method::GET, uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{projection}");
    projection["steps"]
        .as_array()
        .expect("bootstrap steps")
        .iter()
        .find(|step| step["step"] == "aggregate_index_active")
        .cloned()
        .expect("aggregate_index_active step")
}

/// 经成员列表端点取某物理仓的 logical_repository_id。
async fn member_logical_id(app: &axum::Router, fixture: &LcFixture, physical_id: &str) -> String {
    let (status, members) =
        request_json(app.clone(), Method::GET, &fixture.members_uri(), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{members}");
    members["members"]
        .as_array()
        .expect("member rows")
        .iter()
        .find(|row| row["physical_repository_id"] == physical_id)
        .expect("member row")["logical_repository_id"]
        .as_str()
        .expect("logical id")
        .to_string()
}

// —— 用例 1：混合仓冷启动激活 + 卡片跳过告警点名配置成员 ——

#[tokio::test]
async fn mixed_lc_cold_start_activates_and_card_reports_skipped_member() {
    let fixture = seed_lc(&[SOURCE_MEMBER, CONFIG_MEMBER]).await;
    let app = fixture.app();

    let (status, rebuilt) = fixture.rebuild(&app).await;
    assert_eq!(status, StatusCode::OK, "{rebuilt}");
    assert_eq!(rebuilt["state"], "active", "{rebuilt}");
    assert_eq!(
        rebuilt["source_less_alert"]["code"], "aggregate_index_member_source_less",
        "{rebuilt}"
    );
    let message = rebuilt["source_less_alert"]["message_zh"]
        .as_str()
        .expect("alert copy");
    assert!(
        message.contains("已跳过无源码成员"),
        "卡片一行措辞：{message}"
    );
    assert!(
        message.contains("config-only"),
        "告警须点名配置成员：{message}"
    );
    // R8：结构化告警承载跳过事实，响应不得再出现裸清单键。
    assert!(rebuilt.get("skipped_members").is_none(), "{rebuilt}");

    // active 投影同读：state=active + 同一结构化告警。
    let projection = fixture.get_active(&app).await;
    assert_eq!(projection["state"], "active", "{projection}");
    assert_eq!(
        projection["source_less_alert"]["code"], "aggregate_index_member_source_less",
        "{projection}"
    );
    assert!(
        projection["source_less_alert"]["message_zh"]
            .as_str()
            .expect("alert copy")
            .contains("config-only"),
        "{}",
        projection
    );
}

// —— 用例 2（对照，必须失败）：普通路径有源码却未收录 ⇒ 422、不发 active、LKG 保留 ——

#[tokio::test]
async fn source_in_normal_path_fails_and_keeps_last_known_good() {
    let fixture = seed_lc(&[SOURCE_MEMBER, CONFIG_MEMBER]).await;
    let app = fixture.app();

    // 先建出 Active（含跳过配置成员）= LKG。
    let (status, first) = fixture.rebuild(&app).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let lkg_id = fixture
        .active_generation_id()
        .expect("active LKG generation");

    // 探针翻转：source 有源码却报未收录 ⇒ 漏收失败。
    fixture.probe.force();
    let (status, failed) = fixture.rebuild(&app).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{failed}");
    assert_eq!(failed["code"], "aggregate_index_unavailable", "{failed}");
    assert_eq!(
        failed["details"]["reason_code"], "aggregate_index_member_coverage_failed",
        "{failed}"
    );
    assert!(failed["details"]["copy"].is_object(), "{failed}");
    let message = failed["message"].as_str().expect("rendered copy");
    assert!(
        message.contains("怎么做") && message.contains("机器码"),
        "三段中文成品：{message}"
    );
    // 失败面点名该仓（成员名进失败事实与中文成品）。
    assert!(
        failed["details"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("source"),
        "{failed}"
    );

    // 随后单独 GET：degraded + failure_copy 存在（LKG 投影）。
    let projection = fixture.get_active(&app).await;
    assert_eq!(projection["state"], "degraded", "{projection}");
    assert!(projection["failure_copy"].is_object(), "{projection}");
    assert!(
        projection["failure_copy"]["reason_zh"]
            .as_str()
            .unwrap_or_default()
            .contains("source"),
        "失败面须点名漏收成员：{projection}"
    );
    // LKG 世代保留（不发新 active）。
    assert!(
        fixture
            .durable_records()
            .iter()
            .any(|record| record.aggregate_index_id == lkg_id),
        "LKG 世代必须保留"
    );
}

// —— 用例 3：全员无源码 ⇒ 不发布 active（专用码 + 空索引归一投影）——

#[tokio::test]
async fn all_members_source_less_never_becomes_ready() {
    let fixture = seed_lc(&[
        CONFIG_MEMBER,
        MemberSpec {
            dir: "config2",
            alias: "config-two",
            source: false,
        },
    ])
    .await;
    let app = fixture.app();

    let (status, failed) = fixture.rebuild(&app).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{failed}");
    assert_eq!(
        failed["details"]["reason_code"], "aggregate_index_all_members_source_less",
        "{failed}"
    );

    // 投影不 ready：空索引归一（片3）发布带失败封套的 Stale 世代（非 Failed），
    // warning 含专用码、failure_copy 同码，且不发布任何 active 世代。
    let projection = fixture.get_active(&app).await;
    assert_eq!(projection["state"], "stale", "{projection}");
    assert!(
        projection["warning"]
            .as_str()
            .unwrap_or_default()
            .contains("aggregate_index_all_members_source_less"),
        "{}",
        projection
    );
    assert_eq!(
        projection["failure_copy"]["machine_line"], "aggregate_index_all_members_source_less",
        "{}",
        projection
    );
    assert!(
        fixture.active_generation_id().is_none(),
        "不得发布 active 世代"
    );
}

// —— 用例 4：成员列表标注配置仓，且不改选择资格（双成员都 active）——

#[tokio::test]
async fn members_endpoint_annotates_skipped_member_without_changing_eligibility() {
    let fixture = seed_lc(&[SOURCE_MEMBER, CONFIG_MEMBER]).await;
    let app = fixture.app();
    // durable 跳过事实就位（成员列表只读投影，不触发同步）。
    let (status, rebuilt) = fixture.rebuild(&app).await;
    assert_eq!(status, StatusCode::OK, "{rebuilt}");

    let (status, members) =
        request_json(app.clone(), Method::GET, &fixture.members_uri(), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{members}");
    let rows = members["members"].as_array().expect("member rows");
    let find_row = |physical: &str| {
        rows.iter()
            .find(|row| row["physical_repository_id"] == physical)
            .unwrap_or_else(|| panic!("member row {physical}: {members}"))
    };
    let config_row = find_row("repository_0002");
    let source_row = find_row("repository_0001");
    // 配置仓：结构化标注 + 资格不变（status 仍 active）。
    assert_eq!(
        config_row["index_alert"]["code"], "aggregate_index_member_source_less",
        "{config_row}"
    );
    assert!(
        config_row["index_alert"]["message_zh"]
            .as_str()
            .expect("member copy")
            .contains("未纳入代码搜索"),
        "{config_row}"
    );
    assert_eq!(config_row["status"], "active", "{config_row}");
    // 有源码成员不串告警（告警只贴跳过事实本身）且资格同为 active。
    assert!(source_row.get("index_alert").is_none(), "{source_row}");
    assert_eq!(source_row["status"], "active", "{source_row}");
}

// —— 用例 5：只选配置仓建 Issue ⇒ 落库 focus 恰含该成员；规划仍视其为可用成员 ——

#[tokio::test]
async fn create_issue_with_only_config_repository_succeeds() {
    let fixture = seed_lc(&[SOURCE_MEMBER, CONFIG_MEMBER]).await;
    let app = fixture.app();
    let config_logical_id = member_logical_id(&app, &fixture, "repository_0002").await;

    let focus_body = format!(
        r#"{{"title":"只改配置","description":null,"logical_codebase_id":"{}","repository_id":"repository_0001","focus_repository_ids":["{config_logical_id}"]}}"#,
        fixture.lc_id
    );
    let (status, issue) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/issues"),
        serde_json::from_str(&focus_body).expect("focus issue body"),
    )
    .await;
    assert!(
        (200..300).contains(&status.as_u16()),
        "focus issue: {status} {issue}"
    );
    let issue_id = issue["issue_id"].as_str().expect("issue id").to_string();

    // 落库 selection：Explicit，focus 恰含配置成员（没被当成不可用成员拒绝）。
    let selection =
        IssueCodebaseSelectionStore::for_lc(fixture.paths.clone(), fixture.lc_id.clone())
            .load(PROJECT_ID, &issue_id)
            .expect("load selection")
            .expect("selection persisted");
    assert_eq!(
        selection.focus_repository_ids[0].0.to_string(),
        config_logical_id,
        "{selection:?}"
    );

    // 规划上下文仍把该成员留在有效成员集合：involved 钉定配置成员生成 Story 成功。
    let (status, story) = fixture
        .generate_story(&app, &issue_id, Some(&config_logical_id))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "配置成员必须在规划有效成员集合内：{story}"
    );
}

// —— 用例 6/7：跨仓证据——跳过目标未检索（零 CodeGraph 调用）；普通目标照旧 ——

/// 计数 runner：`query` 派发计数 + 可配置命中（filePath 列表）。
struct CountingQueryRunner {
    query_dispatches: AtomicUsize,
    hit_paths: Vec<String>,
}

#[async_trait::async_trait]
impl BoundedCommandRunner for CountingQueryRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        let ok = |stdout: String| BoundedCommandResult {
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 0,
        };
        match request.argv.first().map(String::as_str).unwrap_or_default() {
            "query" => {
                self.query_dispatches.fetch_add(1, Ordering::SeqCst);
                // 命中载荷与 codegraph query --json 同形（node.filePath/startLine/name）。
                let hits = self
                    .hit_paths
                    .iter()
                    .map(|path| {
                        format!(
                            r#"{{"node":{{"filePath":"{path}","startLine":1,"name":"cross_repo_symbol"}}}}"#
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                Ok(ok(format!("[{hits}]")))
            }
            _ => Ok(ok("[]".to_string())),
        }
    }
}

/// 证据夹具：复用 `seed_lc` 的三成员真实注册（app/svc 有源码 + config 只有
/// 配置），经注入探针的**真实 rebuild** 发布「config 记 skip_reason」的 Active
/// 世代（跳过事实走生产链路，非合成落盘）；attempt 与令牌照 evidence_query
/// 夹具形态播种；查询经计数/命中可控假 runner（片7W 注入点）。
struct EvidenceFixture {
    lc: LcFixture,
    token: String,
    runner: Arc<CountingQueryRunner>,
}

/// 目标成员别名（app = 普通源码成员；config = 被跳过的配置成员）。
async fn seed_evidence_fixture(target_alias: &str, hit_paths: Vec<String>) -> EvidenceFixture {
    let lc = seed_lc(&[
        MemberSpec {
            dir: "app",
            alias: "app",
            source: true,
        },
        MemberSpec {
            dir: "svc",
            alias: "svc",
            source: true,
        },
        MemberSpec {
            dir: "config",
            alias: "config",
            source: false,
        },
    ])
    .await;
    let runner = Arc::new(CountingQueryRunner {
        query_dispatches: AtomicUsize::new(0),
        hit_paths,
    });
    let app = lc.evidence_app(runner.clone());
    // 真实 rebuild：app/svc 覆盖、config 经盘检确认无源码 ⇒ skip_reason 落 durable。
    let (status, rebuilt) = lc.rebuild(&app).await;
    assert_eq!(status, StatusCode::OK, "{rebuilt}");
    assert_eq!(rebuilt["state"], "active", "{rebuilt}");
    assert_eq!(
        rebuilt["source_less_alert"]["code"], "aggregate_index_member_source_less",
        "{rebuilt}"
    );

    // 目标成员身份从 LC 权威反查（与 mediator 同源）；政策 digest 同源冻结。
    let logical = LogicalCodebaseStore::for_lc(lc.paths.clone(), lc.lc_id.clone());
    let target = logical
        .list_members(PROJECT_ID)
        .expect("list members")
        .into_iter()
        .find(|member| member.alias == target_alias)
        .expect("target member");
    let checkout = logical
        .load_checkout(PROJECT_ID, target.checkout_ids[0])
        .expect("load checkout")
        .expect("target checkout record");
    let policy_digest = AggregatePolicyArtifactStore::for_lc(lc.paths.clone(), lc.lc_id.clone())
        .get(PROJECT_ID)
        .expect("load policy store")
        .expect("policy artifact")
        .digest;
    let snapshot = AttemptTargetSnapshot {
        logical_repository_id: target.logical_repository_id,
        checkout_id: checkout.checkout_id,
        physical_repository_id: target.physical_repository_id.clone(),
        canonical_path: checkout.canonical_path.clone(),
        git_dir_identity: "git-dir-id".to_string(),
        revision: checkout.revision.clone(),
        policy_digest,
        membership_revision: 1,
        captured_at: NOW.to_string(),
        capture_source: "test".to_string(),
    };
    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: lc.issue_id.clone(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Running,
        version: 0,
        manual_recovery_reason: None,
        admission_ticket_consumed_at: None,
        admission_kind: CodingAdmissionKind::LegacyGroup,
        stage: CodingExecutionStage::Coding,
        base_branch: "main".to_string(),
        branch_name: format!("aria/issues/{}", lc.issue_id),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 0,
            permission_modes: Default::default(),
        },
        rework_count: 0,
        max_auto_rework: 0,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
        target_snapshot: Some(snapshot),
        completed_at: None,
        start_claim: None,
        start_run_policy: cadence_aria::product::coding_models::CodingStartRunPolicy::Manual,
    };
    let attempt_path = lc
        .paths
        .issue_lifecycle_root(PROJECT_ID, &lc.issue_id)
        .join("coding-attempts")
        .join("coding_attempt_0001.json");
    write_json(&attempt_path, &attempt).expect("write attempt record");

    // 令牌签发仓（真实 git + issue 级 worktree），走生产令牌链。
    let repo = lc.root.path().join("token-repo");
    fs::create_dir_all(&repo).expect("token repo dir");
    run_git(&repo, &["init"]);
    fs::write(repo.join("README.md"), "# repo\n").expect("token repo readme");
    run_git(&repo, &["add", "."]);
    run_git(
        &repo,
        &[
            "-c",
            "user.name=Aria Test",
            "-c",
            "user.email=aria@example.com",
            "commit",
            "-m",
            "initial",
        ],
    );
    let worktree = repo
        .join(".worktrees")
        .join("aria-issues")
        .join(&lc.issue_id);
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            worktree.to_str().expect("worktree path"),
            "-b",
            "aria/issues/issue_0001",
        ],
    );
    let token = issue_evidence_token(&lc.paths, &repo, &attempt).expect("issue evidence token");

    EvidenceFixture { lc, token, runner }
}

impl EvidenceFixture {
    fn app(&self) -> axum::Router {
        self.lc.evidence_app(self.runner.clone())
    }

    async fn query(&self, app: axum::Router, query_symbol: &str) -> (StatusCode, Value) {
        request_json(
            app,
            Method::POST,
            "/api/evidence-query",
            json!({"token": self.token, "role": "coder", "query": query_symbol}),
        )
        .await
    }

    fn first_audit_line(&self) -> Value {
        let content = fs::read_to_string(
            self.lc
                .paths
                .issue_lifecycle_root(PROJECT_ID, &self.lc.issue_id)
                .join("coding-attempts")
                .join("coding_attempt_0001")
                .join("evidence-audit.jsonl"),
        )
        .expect("read audit file");
        serde_json::from_str(content.lines().next().expect("first audit line")).expect("audit JSON")
    }
}

#[tokio::test]
async fn evidence_query_for_skipped_target_returns_not_searched() {
    let fixture = seed_evidence_fixture("config", Vec::new()).await;
    let app = fixture.app();

    let (status, body) = fixture.query(app, "limits").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["not_searched"]["code"], "aggregate_index_member_source_less",
        "{body}"
    );
    assert!(
        body["text"]
            .as_str()
            .expect("text")
            .contains("未检索聚合索引"),
        "{body}"
    );
    // 零 CodeGraph 调用：跳过支路不建查询。
    assert_eq!(fixture.runner.query_dispatches.load(Ordering::SeqCst), 0);
    // 审计：零命中 + 未检索原因非空。
    let audit = fixture.first_audit_line();
    assert_eq!(audit["hit_count"], 0, "{audit}");
    assert!(
        audit["not_searched_reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()),
        "{audit}"
    );
}

#[tokio::test]
async fn normal_target_evidence_excludes_skipped_member_paths() {
    // 目标 = app（普通源码成员）；命中一条在 svc（应保留）、一条在 config
    //（被跳过，不得出现）。必须有一条应保留命中，否则「无 config」是假绿。
    // 注：目标成员自身目录的命中按跨仓证据语义本就过滤（evidence_index 的
    // is_cross_member_hit），故保留命中放在另一源码成员 svc。
    let fixture = seed_evidence_fixture(
        "app",
        vec![
            "svc/src/lib.rs".to_string(),
            "config/settings.json".to_string(),
        ],
    )
    .await;
    let app = fixture.app();

    let (status, body) = fixture.query(app, "cross_repo_symbol").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let text = body["text"].as_str().expect("text");
    assert!(text.contains("svc/src/lib.rs"), "应保留命中缺失：{text}");
    assert!(!text.contains("config/"), "跳过成员路径不得出现：{text}");
    // 查询照旧派发（普通支路）；无「未检索」事实。
    assert_eq!(fixture.runner.query_dispatches.load(Ordering::SeqCst), 1);
    assert!(body["not_searched"].is_null(), "{body}");
}

// —— 用例 8：Story 前刷新——干净复用不触发同步；持续脏 ⇒ 同步（世代 id 变化）——

#[tokio::test]
async fn story_generation_syncs_when_member_stays_dirty() {
    let fixture = seed_lc(&[SOURCE_MEMBER, CONFIG_MEMBER]).await;
    let app = fixture.app();

    // 首次生成：无索引 ⇒ 分支 1 首建（冷启动）。
    let (status, first) = fixture.generate_story(&app, &fixture.issue_id, None).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let first_id = fixture
        .active_generation_id()
        .expect("first cold-start generation");

    // 干净且未变 ⇒ 复用：同世代、不新增 durable 世代（未触发同步）。
    let (status, reused) = fixture.generate_story(&app, &fixture.issue_id, None).await;
    assert_eq!(status, StatusCode::OK, "{reused}");
    assert_eq!(
        fixture.active_generation_id().as_deref(),
        Some(first_id.as_str()),
        "干净未变必须复用同世代"
    );
    assert_eq!(fixture.durable_records().len(), 1, "复用不得新增世代");

    // 改 source 一个函数体且不提交 ⇒ 脏 ⇒ 生成前同步 ⇒ 新世代。
    let source_main = fixture.aggregate_root.join("source/src/main.rs");
    fs::write(&source_main, "pub fn member_symbol() -> u32 { 2 }\n").expect("dirty edit");
    let (status, synced) = fixture.generate_story(&app, &fixture.issue_id, None).await;
    assert_eq!(status, StatusCode::OK, "{synced}");
    let second_id = fixture
        .active_generation_id()
        .expect("generation after sync");
    assert_ne!(second_id, first_id, "脏成员必须触发同步（新世代）");

    // 再改一次仍不提交 ⇒ 再次同步（两次都脏都必须同步）。
    fs::write(&source_main, "pub fn member_symbol() -> u32 { 3 }\n").expect("dirty edit again");
    let (status, synced_again) = fixture.generate_story(&app, &fixture.issue_id, None).await;
    assert_eq!(status, StatusCode::OK, "{synced_again}");
    assert_ne!(
        fixture.active_generation_id().as_deref(),
        Some(second_id.as_str()),
        "持续脏必须再次同步"
    );
}

// —— 用例 9：真走冷启动（不手动 rebuild）⇒ durable 世代存在 + 冷启动步骤就绪 ——

#[tokio::test]
async fn planning_ready_proves_cold_start_built_the_index() {
    let fixture = seed_lc(&[SOURCE_MEMBER, CONFIG_MEMBER]).await;
    let app = fixture.app();

    // 前置：无任何在册世代（证明没有走手动 rebuild 冒充冷启动）。
    let cold = fixture.get_active(&app).await;
    assert_eq!(cold["state"], "missing", "{cold}");
    assert!(fixture.active_generation_id().is_none());

    // 规划上下文解析（Story 入口）真走冷启动：分支 1 首建 + 验收激活。
    let (status, story) = fixture.generate_story(&app, &fixture.issue_id, None).await;
    assert_eq!(status, StatusCode::OK, "{story}");

    // durable active 世代存在（冷启动产物，非手动 rebuild）。
    let generation = fixture
        .active_generation_id()
        .expect("cold start built the index");
    let projection = fixture.get_active(&app).await;
    assert_eq!(projection["state"], "active", "{projection}");

    // 冷启动投影：aggregate_index_active 步骤完成（本片就绪面的权威判定）。
    let step = bootstrap_aggregate_step(&app, &fixture.bootstrap_uri()).await;
    assert_eq!(step["status"], "completed", "{step}");
    assert_eq!(step["object_id"], generation.as_str(), "{step}");
}

// —— 用例 10：Story 专用刷新失败 ⇒ AI 不启动（422 + 稳定码）且既有 Story 保留 ——

#[tokio::test]
async fn story_generation_uses_story_specific_refresh_and_stops_before_ai_on_failure() {
    let fixture = seed_lc(&[SOURCE_MEMBER, CONFIG_MEMBER]).await;
    let app = fixture.app();

    // 先成功一次（既有 Story + 冷启动索引）。
    let (status, first) = fixture.generate_story(&app, &fixture.issue_id, None).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let existing_story_id = first["story_specs"][0]["story_spec_id"]
        .as_str()
        .expect("existing story id")
        .to_string();

    // 刷新失败：先让 source 留未提交修改（复用门必失效 ⇒ 落分支 3 增量同步），
    // 再把探针翻成「盘上有源码却未收录」⇒ 覆盖失败（稳定码失败面）。
    fs::write(
        fixture.aggregate_root.join("source/src/main.rs"),
        "pub fn member_symbol() -> u32 { 9 }\n",
    )
    .expect("dirty edit");
    fixture.probe.force();
    let (status, failed) = fixture.generate_story(&app, &fixture.issue_id, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{failed}");
    assert_eq!(failed["code"], "aggregate_index_unavailable", "{failed}");
    assert_ne!(failed["code"], "product_store_error", "{failed}");
    assert_eq!(
        failed["details"]["reason_code"], "aggregate_index_member_coverage_failed",
        "{failed}"
    );
    assert!(failed["details"]["copy"].is_object(), "{failed}");
    let message = failed["message"].as_str().expect("rendered copy");
    assert!(
        message.contains("怎么做") && message.contains("机器码"),
        "三段中文成品：{message}"
    );
    // details.detail 为脱敏后的原始事实（含成员事实）。
    assert!(
        failed["details"]["detail"]
            .as_str()
            .is_some_and(|detail| !detail.is_empty()),
        "{failed}"
    );

    // AI 未启动：没有第二个 Story；既有 Story 原样保留。
    let (status, lifecycle) = request_json(
        app.clone(),
        Method::GET,
        &format!(
            "/api/issues/{}/lifecycle?project_id={PROJECT_ID}",
            fixture.issue_id
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{lifecycle}");
    let stories = lifecycle["story_specs"].as_array().expect("story specs");
    assert_eq!(stories.len(), 1, "AI 不得启动（不新增 Story）：{lifecycle}");
    assert_eq!(
        stories[0]["story_spec_id"],
        existing_story_id.as_str(),
        "{lifecycle}"
    );
}

// —— 用例 11：Design 生成不受 degraded 停机影响（仍走 LKG + 可审计告警）——

#[tokio::test]
async fn design_generation_still_uses_last_known_good_with_warning() {
    let fixture = seed_lc(&[SOURCE_MEMBER, CONFIG_MEMBER]).await;
    let app = fixture.app();

    // 既有 Confirmed Story（Design 前置）。
    let (status, story) = fixture.generate_story(&app, &fixture.issue_id, None).await;
    assert_eq!(status, StatusCode::OK, "{story}");
    let story_id = story["story_specs"][0]["story_spec_id"]
        .as_str()
        .expect("story id")
        .to_string();
    LifecycleStore::new(fixture.paths.clone())
        .update_spec_confirmation_status(
            PROJECT_ID,
            &fixture.issue_id,
            &story_id,
            cadence_aria::product::models::LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm story");

    // 造 degraded（有 LKG）：先成功、再失败一次。
    let (status, first) = fixture.rebuild(&app).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    fixture.probe.force();
    let (status, failed) = fixture.rebuild(&app).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{failed}");

    // Design 生成：不停机（LKG + 告警），上下文带可审计告警。
    let design_body = format!(
        r#"{{"title":"聚合 Design Spec","story_spec_ids":["{story_id}"],"author_provider":"fake","reviewer_provider":"codex","review_rounds":3,"superpowers_enabled":false,"openspec_enabled":false}}"#
    );
    let (status, design) = request_json(
        app.clone(),
        Method::POST,
        &format!(
            "/api/projects/{PROJECT_ID}/issues/{}/design-specs:generate",
            fixture.issue_id
        ),
        serde_json::from_str(&design_body).expect("design body"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "degraded 不得停机 Design 生成：{design}"
    );
    let content = design["workspace_session"]["messages"]
        .as_array()
        .expect("session messages")
        .iter()
        .find(|message| {
            message["role"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("Workspace 生成任务已准备"))
        })
        .expect("generation context message")["content"]
        .as_str()
        .expect("context content");
    assert!(
        content.contains("aggregate index warning:"),
        "缺可审计告警：{content}"
    );
    assert!(
        content.contains("aggregate_index_member_coverage_failed"),
        "告警须带失败封套稳定码：{content}"
    );
}

// —— 用例 12：同一失败封套两出口（HTTP failure_copy ↔ 冷启动投影）逐字同文案 ——

#[tokio::test]
async fn http_and_bootstrap_projection_share_copy_for_same_envelope() {
    let fixture = seed_lc(&[SOURCE_MEMBER, CONFIG_MEMBER]).await;
    let app = fixture.app();

    // 同一封套造一次失败：成员源码只存在于被排除的 .worktrees/ 子树（专属码），
    // 事实点名 gray-config / msha-sdk（与生产失败构造函数同句式）。
    let envelope = "aggregate_index_failed:aggregate_index_member_source_excluded: \
                    source exists only under .worktrees/ for included members [gray-config, msha-sdk]; \
                    move it into the member main checkout";
    let record = AggregateIndexRecord {
        command_id: None,
        aggregate_index_id: format!("aggregate_index_{}", uuid::Uuid::new_v4().simple()),
        project_id: PROJECT_ID.to_string(),
        membership_revision: 1,
        status: AggregateIndexStatus::Failed,
        member_snapshots: Vec::new(),
        observed_after_member_snapshots: Vec::new(),
        codegraph_version: "1.6.0".to_string(),
        codegraph_root: fixture.aggregate_root.clone(),
        config_digest: String::new(),
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
        supersedes_aggregate_index_id: None,
        warning: Some(envelope.to_string()),
    };
    AggregateIndexStore::for_lc(fixture.paths.clone(), fixture.lc_id.clone())
        .create(PROJECT_ID, record)
        .expect("create failed generation");

    // 出口①：active 投影的 failure_copy（HTTP failure_copy_from_warning）。
    let projection = fixture.get_active(&app).await;
    let http_copy = &projection["failure_copy"];
    assert!(http_copy.is_object(), "{projection}");

    // 出口②：冷启动投影的 BootstrapFailure.copy。
    let step = bootstrap_aggregate_step(&app, &fixture.bootstrap_uri()).await;
    assert_eq!(step["status"], "failed", "{step}");
    assert_eq!(
        step["failure"]["reason_code"], "aggregate_index_member_source_excluded",
        "{step}"
    );
    let bootstrap_copy = &step["failure"]["copy"];
    assert!(bootstrap_copy.is_object(), "{step}");

    // 两出口逐字相等（title/reason/action/machine_line），reason_code 同一内层码。
    for field in ["title_zh", "reason_zh", "action_zh", "machine_line"] {
        assert_eq!(
            http_copy[field], bootstrap_copy[field],
            "两出口 {field} 必须逐字一致"
        );
    }
    // 点名守卫（F2）：两边的 reason_zh 都必须含两个成员名（不得靠两边丢点名通过）。
    for copy in [http_copy, bootstrap_copy] {
        let reason = copy["reason_zh"].as_str().expect("reason_zh");
        assert!(reason.contains("gray-config"), "点名丢失：{reason}");
        assert!(reason.contains("msha-sdk"), "点名丢失：{reason}");
    }
}
