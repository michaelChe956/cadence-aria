// 注意：本文件与 query_hit.rs 同被 tests.rs include 进同一模块，`PathBuf`
// 由 query_hit.rs 的 `use std::path::PathBuf;` 提供，此处不重复导入。
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};
use crate::product::logical_codebase::aggregate_index::CodeGraphCli;
use crate::product::logical_codebase::evidence_index::{
    EvidenceError, EvidenceHit, EvidenceIndexQuery,
};

const AGGREGATE_ROOT: &str = "/aggregate-root";

// C-4 Task 2：`EvidenceIndexQuery` 的 fake runner 单测（不调用真实 Provider）。
// fake runner 只验证「命令构造 + 结果解析 + 成员目录首段过滤 + 200 上限」。

struct FakeCodeGraphRunner {
    results: Mutex<VecDeque<Result<BoundedCommandResult, BoundedCommandError>>>,
    requests: Mutex<Vec<BoundedCommandRequest>>,
}

impl FakeCodeGraphRunner {
    fn with_stdout(stdout: &str) -> Self {
        let result = Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: stdout.to_string(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        });
        Self {
            results: Mutex::new(VecDeque::from([result])),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<BoundedCommandRequest> {
        self.requests.lock().expect("fake runner requests").clone()
    }
}

#[async_trait::async_trait]
impl BoundedCommandRunner for FakeCodeGraphRunner {
    async fn run(
        &self,
        request: BoundedCommandRequest,
    ) -> Result<BoundedCommandResult, BoundedCommandError> {
        self.requests.lock().expect("fake runner requests").push(request);
        self.results
            .lock()
            .expect("fake runner results")
            .pop_front()
            .expect("fake runner scripted result")
    }
}

fn query_for(runner: Arc<FakeCodeGraphRunner>) -> EvidenceIndexQuery {
    EvidenceIndexQuery::new(
        CodeGraphCli::new(runner, "codegraph".to_string()),
        PathBuf::from(AGGREGATE_ROOT),
        vec!["api".to_string(), "web".to_string()],
        "api".to_string(),
    )
}

#[test]
fn query_filters_to_other_member_dirs_and_excludes_target_and_non_members() {
    // 4 条命中：api/（成员，但为目标成员目录）、web/（成员）、api/目标仓、other/（非成员）。
    let hits_json = serde_json::json!([
        {"node": {"name": "alpha", "filePath": "api/src/lib.ts", "startLine": 1}},
        {"node": {"name": "beta", "filePath": "web/src/app.ts", "startLine": 7}},
        {"node": {"name": "gamma", "filePath": "api/target.ts", "startLine": 2}},
        {"node": {"name": "delta", "filePath": "other/foo.ts", "startLine": 3}},
    ]);
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&hits_json.to_string()));
    let query = query_for(runner.clone());

    let hits = query.query("crossRepoSymbol").expect("query must succeed");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0],
        EvidenceHit {
            file_path: "web/src/app.ts".to_string(),
            start_line: 7,
            symbol: "beta".to_string(),
        }
    );

    // 封装透传：aggregate_root 作 working_dir，argv = query <symbol> --json。
    let requests = runner.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].working_dir, PathBuf::from(AGGREGATE_ROOT));
    assert_eq!(requests[0].argv, ["query", "crossRepoSymbol", "--json"]);
}

#[test]
fn query_truncates_to_two_hundred_hits_when_over_cap() {
    let entries: Vec<serde_json::Value> = (0..250)
        .map(|index| {
            serde_json::json!({
                "node": {
                    "name": format!("sym{index}"),
                    "filePath": format!("web/src/file{index}.ts"),
                    "startLine": index,
                }
            })
        })
        .collect();
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout(
        &serde_json::json!(entries).to_string(),
    ));
    let query = query_for(runner);

    let hits = query.query("sym").expect("query must succeed");
    assert_eq!(hits.len(), 200, "hits must be truncated to the 200 cap");
}

#[test]
fn query_passes_through_codegraph_invalid_json_error() {
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout("definitely not json"));
    let query = query_for(runner);

    let error = query.query("sym").expect_err("invalid JSON must fail");
    match error {
        EvidenceError::QueryFailed { code, .. } => {
            assert_eq!(code, "codegraph_query_invalid_json");
        }
        other => panic!("expected QueryFailed(codegraph_query_invalid_json), got {other:?}"),
    }
}

#[test]
fn query_rejects_empty_symbol_without_invoking_runner() {
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout("[]"));
    let query = query_for(runner.clone());

    let error = query.query("").expect_err("empty symbol must fail");
    assert!(matches!(error, EvidenceError::InvalidQuery { .. }));
    assert!(
        runner.requests().is_empty(),
        "runner must not be invoked for an invalid symbol"
    );
}

#[test]
fn query_parses_real_fixture_definition_and_cross_member_import() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/product/logical_codebase/evidence_index/tests/fixtures/query_hit.json");
    let raw = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|error| panic!("read fixture {fixture_path:?}: {error}"));
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&raw));

    // Coder 在 web/：返回 api/ 的定义点命中（另一成员仓）。
    let query = EvidenceIndexQuery::new(
        CodeGraphCli::new(runner, "codegraph".to_string()),
        PathBuf::from(AGGREGATE_ROOT),
        vec!["api".to_string(), "web".to_string()],
        "web".to_string(),
    );
    let hits = query
        .query("format_compact_duration")
        .expect("query must succeed");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0],
        EvidenceHit {
            file_path: "api/src/lib.ts".to_string(),
            start_line: 1,
            symbol: "format_compact_duration".to_string(),
        }
    );

    // Coder 在 api/：返回 web/ 的跨成员 import 命中（启发式符号引用）。
    let runner = Arc::new(FakeCodeGraphRunner::with_stdout(&raw));
    let query = EvidenceIndexQuery::new(
        CodeGraphCli::new(runner, "codegraph".to_string()),
        PathBuf::from(AGGREGATE_ROOT),
        vec!["api".to_string(), "web".to_string()],
        "api".to_string(),
    );
    let hits = query
        .query("format_compact_duration")
        .expect("query must succeed");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0],
        EvidenceHit {
            file_path: "web/src/app.ts".to_string(),
            start_line: 1,
            symbol: "../../api/src/lib".to_string(),
        }
    );
}
