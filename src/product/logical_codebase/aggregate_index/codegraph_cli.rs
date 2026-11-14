use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
};

use super::AggregateIndexBudget;
use crate::product::json_store::ProductStoreError;

/// G2（终局关闸缺口）：受支持的 CodeGraph 兼容区间——语义 `^1.6`
///（`>=1.6.0, <2.0.0`）。
///
/// 原精确钉版 `1.6.0` 使环境补丁升级（如 1.6.1）即击断 LC 冷启动
///（`codegraph_version_mismatch`）。1.x 内 CLI 契约（denylist 范围等）
/// 稳定；2.0 起可能破坏 CLI 行为，届时需重新验证后上调区间。
pub const CODEGRAPH_MIN_VERSION: (u64, u64, u64) = (1, 6, 0);
pub const CODEGRAPH_MAX_VERSION_EXCLUSIVE: (u64, u64, u64) = (2, 0, 0);

/// 解析 `codegraph --version` 输出为三元组：取最后一个空白分隔 token，
/// 要求恰为 `major.minor.patch` 三段数字；其余形态（含前后缀噪声、缺段）
/// 一律 `None`（fail-closed，不静默放行）。
pub fn parse_codegraph_version(output: &str) -> Option<(u64, u64, u64)> {
    let token = output.split_whitespace().next_back()?;
    let mut segments = token.split('.');
    let major = segments.next()?.parse().ok()?;
    let minor = segments.next()?.parse().ok()?;
    let patch = segments.next()?.parse().ok()?;
    if segments.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}
const OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;

/// `codegraph files --json` 的专用 stdout 上限。
///
/// 文件清单载荷随聚合根规模线性膨胀（实测 9k~20k 文件时为 1.3~3.9 MB），
/// 通用 1 MiB 上限会把 JSON 尾部截断并把残缺载荷误报成解析失败。
/// 该上限只用于 `files`，其余命令维持 1 MiB。
const FILES_OUTPUT_LIMIT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AggregateIndexError {
    #[error("aggregate_index_degraded:{code}: {message}")]
    Degraded { code: &'static str, message: String },
    #[error("aggregate_index_failed:{code}: {message}")]
    Failed { code: &'static str, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeGraphCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    /// stdout 是否被上限截断——载荷不完整，调用方不得把它当完整结果用
    /// （例如把截断后的 JSON 前缀当合法载荷解析）。
    pub stdout_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeGraphStatus {
    pub initialized: bool,
    pub version: String,
    #[serde(rename = "projectPath")]
    pub project_path: PathBuf,
    #[serde(rename = "indexPath")]
    pub index_path: PathBuf,
    #[serde(rename = "lastIndexed")]
    pub last_indexed: Option<String>,
    #[serde(rename = "fileCount")]
    pub file_count: u64,
    #[serde(rename = "nodeCount")]
    pub node_count: u64,
    #[serde(rename = "edgeCount")]
    pub edge_count: u64,
    #[serde(rename = "dbSizeBytes")]
    pub db_size_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct CodeGraphFile {
    path: PathBuf,
}

impl From<ProductStoreError> for AggregateIndexError {
    fn from(error: ProductStoreError) -> Self {
        Self::Failed {
            code: "aggregate_index_store_error",
            message: error.to_string(),
        }
    }
}

#[derive(Clone)]
pub struct CodeGraphCli {
    runner: Arc<dyn BoundedCommandRunner>,
    executable: String,
}

/// 聚合根上的 codegraph 库目录路径（`.codegraph`）。
///
/// 判据与诊断同源：探测（`codegraph_library_present`）与错误消息里的路径
/// 展示共用本函数，`.codegraph` 字面量全仓只此一处。
pub fn codegraph_library_dir(root: &Path) -> PathBuf {
    root.join(".codegraph")
}

/// 聚合根上的 codegraph 库目录是否在位。
///
/// 跨仓证据检索（evidence 链）要读该库目录：目录缺失时 codegraph 对该根恒返回
/// 空结果（exit 0 + `[]`），证据链必须按「索引不可用」fail-closed，不能把
/// 「索引坏了」误报成「没有证据」。库目录在册而查询无命中仍是合法空结果。
pub fn codegraph_library_present(root: &Path) -> bool {
    codegraph_library_dir(root).is_dir()
}

impl CodeGraphCli {
    pub fn new(runner: Arc<dyn BoundedCommandRunner>, executable: String) -> Self {
        Self { runner, executable }
    }

    /// 返回实测的 CLI 版本字符串（stdout 去空白），供聚合索引记录在
    /// 激活时透传；不再让调用方依赖硬编码默认值误导排障。
    pub fn verify_version(&self) -> Result<String, AggregateIndexError> {
        let output = self.run_checked(
            &["--version"],
            Path::new("."),
            AggregateIndexBudget::INCREMENTAL,
            "codegraph_version_failed",
            OUTPUT_LIMIT_BYTES,
        )?;
        let actual = output.stdout.trim();
        let in_range = parse_codegraph_version(actual).is_some_and(|version| {
            version >= CODEGRAPH_MIN_VERSION && version < CODEGRAPH_MAX_VERSION_EXCLUSIVE
        });
        if in_range {
            Ok(actual.to_string())
        } else {
            Err(AggregateIndexError::Degraded {
                code: "codegraph_version_mismatch",
                message: format!("expected >=1.6.0, <2.0.0 (compatible ^1.6 range), got {actual}"),
            })
        }
    }

    pub fn init(&self, root: &Path) -> Result<CodeGraphCommandResult, AggregateIndexError> {
        self.run_checked(
            &["init", "."],
            root,
            AggregateIndexBudget::FIFTY_MEMBER_INITIAL,
            "codegraph_init_failed",
            OUTPUT_LIMIT_BYTES,
        )
    }

    pub fn sync(&self, root: &Path) -> Result<CodeGraphCommandResult, AggregateIndexError> {
        self.run_checked(
            &["sync", "."],
            root,
            AggregateIndexBudget::INCREMENTAL,
            "codegraph_sync_failed",
            OUTPUT_LIMIT_BYTES,
        )
    }

    /// Sync 模式防漂移：durable Active 记录可能与磁盘库漂移——`.codegraph`
    /// 被清理或聚合根被重建后，对未初始化的库执行 `sync` 必然 exit 1
    /// （`✗ CodeGraph not initialized`），stale→sync 成为死路。探测到库
    /// 目录缺失，或捕获该失败时，回退 init＋sync（等价 Initialize/Rebuild
    /// 的建库序列），让增量刷新自愈而不是恒败。
    pub fn sync_or_reinit(
        &self,
        root: &Path,
    ) -> Result<CodeGraphCommandResult, AggregateIndexError> {
        if !codegraph_library_present(root) {
            self.init(root)?;
            return self.sync(root);
        }
        match self.sync(root) {
            Ok(result) => Ok(result),
            Err(error) => match &error {
                AggregateIndexError::Failed {
                    code: "codegraph_sync_failed",
                    message,
                } if message.contains("not initialized") => {
                    self.init(root)?;
                    self.sync(root)
                }
                _ => Err(error),
            },
        }
    }

    pub fn files(&self, root: &Path) -> Result<Vec<PathBuf>, AggregateIndexError> {
        let output = self.run_checked(
            &["files", "--json"],
            root,
            AggregateIndexBudget::INCREMENTAL,
            "codegraph_files_failed",
            FILES_OUTPUT_LIMIT_BYTES,
        )?;
        // 截断判定必须先于解析：stdout 被上限切掉后剩下的是 JSON 前缀，
        // 解析必然失败。若继续落到解析分支，截断会被混报成
        // `codegraph_files_invalid_json`（上游协议异常），把排障方向带偏。
        if output.stdout_truncated {
            return Err(AggregateIndexError::Failed {
                code: "codegraph_files_output_truncated",
                message: format!(
                    "codegraph files --json 输出被截断：stdout 超过 {FILES_OUTPUT_LIMIT_BYTES} 字节上限，文件清单不完整",
                ),
            });
        }
        // codegraph 1.6 实证契约缺陷：空索引（0 文件）时 `files --json`
        // 以 exit 0 输出人读提示而非 JSON 数组。先识别该形态返回专用
        // 空索引码，交给上层归并为业务语义的覆盖不足错误；
        // `codegraph_files_invalid_json` 仅保留给真正的意外输出。
        if is_empty_index_notice(&output.stdout) {
            return Err(AggregateIndexError::Failed {
                code: "codegraph_files_empty_index",
                message: format!(
                    "codegraph files --json reported an empty index: {}",
                    output.stdout.trim()
                ),
            });
        }
        let entries: Vec<CodeGraphFile> =
            serde_json::from_str(&output.stdout).map_err(|error| AggregateIndexError::Failed {
                code: "codegraph_files_invalid_json",
                message: format!("parse JSON from codegraph files: {error}"),
            })?;
        Ok(entries.into_iter().map(|entry| entry.path).collect())
    }

    pub fn query_json(
        &self,
        root: &Path,
        query: &str,
    ) -> Result<serde_json::Value, AggregateIndexError> {
        let output = self.run_checked(
            &["query", query, "--json"],
            root,
            AggregateIndexBudget::INCREMENTAL,
            "codegraph_query_failed",
            OUTPUT_LIMIT_BYTES,
        )?;
        serde_json::from_str(&output.stdout).map_err(|error| AggregateIndexError::Failed {
            code: "codegraph_query_invalid_json",
            message: format!("parse JSON from codegraph query: {error}"),
        })
    }

    pub fn status(&self, root: &Path) -> Result<CodeGraphStatus, AggregateIndexError> {
        let output = self.run_checked(
            &["status", ".", "--json"],
            root,
            AggregateIndexBudget::INCREMENTAL,
            "codegraph_status_failed",
            OUTPUT_LIMIT_BYTES,
        )?;
        serde_json::from_str(&output.stdout).map_err(|error| AggregateIndexError::Failed {
            code: "codegraph_status_invalid_json",
            message: format!("parse JSON from codegraph status: {error}"),
        })
    }

    fn run_checked(
        &self,
        argv: &[&str],
        root: &Path,
        budget: AggregateIndexBudget,
        failure_code: &'static str,
        stdout_limit: usize,
    ) -> Result<CodeGraphCommandResult, AggregateIndexError> {
        let output = self.run(argv, root, budget.fail_secs, stdout_limit)?;
        if output.exit_code != Some(0) {
            return Err(AggregateIndexError::Failed {
                code: failure_code,
                message: command_failure_message(argv, &output),
            });
        }
        Ok(command_result(output))
    }

    fn run(
        &self,
        argv: &[&str],
        root: &Path,
        timeout_secs: u64,
        stdout_limit: usize,
    ) -> Result<BoundedCommandResult, AggregateIndexError> {
        let request = BoundedCommandRequest {
            executable: self.executable.clone(),
            argv: argv.iter().map(|argument| (*argument).to_owned()).collect(),
            working_dir: root.to_path_buf(),
            timeout: Duration::from_secs(timeout_secs),
            cancellation: CancellationToken::new(),
            environment: sanitized_environment(),
            stdout_limit,
            stderr_limit: OUTPUT_LIMIT_BYTES,
        };
        let runner = self.runner.clone();
        let output = std::thread::spawn(move || -> Result<_, AggregateIndexError> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| AggregateIndexError::Failed {
                    code: "codegraph_runner_runtime",
                    message: error.to_string(),
                })?;
            runtime
                .block_on(runner.run(request))
                .map_err(map_runner_error)
        })
        .join()
        .map_err(|_| AggregateIndexError::Failed {
            code: "codegraph_runner_panic",
            message: "bounded CodeGraph runner thread panicked".to_string(),
        })??;
        if output.timed_out {
            return Err(AggregateIndexError::Failed {
                code: "codegraph_timeout",
                message: format!(
                    "codegraph {} timed out after {timeout_secs}s",
                    argv.join(" ")
                ),
            });
        }
        if output.cancelled {
            return Err(AggregateIndexError::Failed {
                code: "codegraph_cancelled",
                message: format!("codegraph {} was cancelled", argv.join(" ")),
            });
        }
        Ok(output)
    }
}

fn sanitized_environment() -> BTreeMap<String, String> {
    std::env::var("PATH")
        .map(|path| BTreeMap::from([("PATH".to_string(), path)]))
        .unwrap_or_default()
}

fn map_runner_error(error: BoundedCommandError) -> AggregateIndexError {
    match error {
        BoundedCommandError::CommandMissing {
            executable,
            details,
        } => AggregateIndexError::Degraded {
            code: "codegraph_missing",
            message: format!("{executable}: {details}"),
        },
        BoundedCommandError::Io { details } => AggregateIndexError::Failed {
            code: "codegraph_io",
            message: details,
        },
    }
}

fn command_result(output: BoundedCommandResult) -> CodeGraphCommandResult {
    CodeGraphCommandResult {
        stdout: output.stdout,
        stderr: output.stderr,
        duration_ms: output.duration_ms,
        stdout_truncated: output.stdout_truncated,
    }
}

fn command_failure_message(argv: &[&str], output: &BoundedCommandResult) -> String {
    let stderr = output.stderr.trim();
    let stdout = output.stdout.trim();
    let details = if !stderr.is_empty() { stderr } else { stdout };
    format!(
        "codegraph {} exited with {:?}: {details}",
        argv.join(" "),
        output.exit_code
    )
}

/// codegraph 1.6 空索引时以 exit 0 输出的固定人读提示（实测 1.6.2：
/// `ℹ No files indexed. Run "codegraph index" first.`，stdout）。合法的
/// files JSON 数组不可能包含该文本。
fn is_empty_index_notice(stdout: &str) -> bool {
    stdout.trim().contains("No files indexed")
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Mutex, MutexGuard};

    use super::*;

    struct CommandCapture {
        result: Result<BoundedCommandResult, BoundedCommandError>,
    }

    impl CommandCapture {
        fn success(stdout: &str) -> Self {
            Self {
                result: Ok(command_result_with(stdout, "", Some(0), false, false)),
            }
        }

        /// 构造 stdout 已被上限截断的成功结果（上游按上限保留前缀、丢弃其余）。
        fn truncated_success(stdout: &str) -> Self {
            Self {
                result: Ok(BoundedCommandResult {
                    stdout_truncated: true,
                    ..command_result_with(stdout, "", Some(0), false, false)
                }),
            }
        }

        fn failure(exit_code: i32, stderr: &str) -> Self {
            Self {
                result: Ok(command_result_with(
                    "",
                    stderr,
                    Some(exit_code),
                    false,
                    false,
                )),
            }
        }

        fn timed_out() -> Self {
            Self {
                result: Ok(command_result_with("", "", None, true, false)),
            }
        }
    }

    struct ScriptedCodeGraphRunner {
        results: Mutex<VecDeque<Result<BoundedCommandResult, BoundedCommandError>>>,
        requests: Mutex<Vec<BoundedCommandRequest>>,
    }

    impl ScriptedCodeGraphRunner {
        fn from_results(results: Vec<CommandCapture>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().map(|capture| capture.result).collect()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn missing() -> Self {
            Self {
                results: Mutex::new(VecDeque::from([Err(BoundedCommandError::CommandMissing {
                    executable: "codegraph".to_string(),
                    details: "not found".to_string(),
                })])),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> MutexGuard<'_, Vec<BoundedCommandRequest>> {
            self.requests.lock().expect("scripted requests")
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for ScriptedCodeGraphRunner {
        async fn run(
            &self,
            request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            self.requests
                .lock()
                .expect("scripted requests")
                .push(request);
            self.results
                .lock()
                .expect("scripted results")
                .pop_front()
                .expect("scripted result")
        }
    }

    fn command_result_with(
        stdout: &str,
        stderr: &str,
        exit_code: Option<i32>,
        timed_out: bool,
        cancelled: bool,
    ) -> BoundedCommandResult {
        BoundedCommandResult {
            exit_code,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            timed_out,
            cancelled,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 12,
        }
    }

    #[test]
    fn version_gate_accepts_compatible_range_and_rejects_out_of_range() {
        // G2（终局关闸缺口）：环境升级（如 1.6.0→1.6.1）不应击断 LC 冷启动。
        // 版本门接受 ^1.6 语义兼容区间（>=1.6.0，<2.0.0）；区间外与不可解析
        // 输出仍 fail-closed（codegraph_version_mismatch，不静默放行）。
        let runner = Arc::new(ScriptedCodeGraphRunner::from_results(vec![
            CommandCapture::success("1.6.0\n"),
            CommandCapture::success("Indexed 6 files\n"),
        ]));
        let cli = CodeGraphCli::new(runner.clone(), "codegraph".into());
        cli.verify_version().unwrap();
        cli.init(Path::new("/aggregate")).unwrap();

        let requests = runner.requests();
        assert_eq!(requests[0].argv, ["--version"]);
        assert_eq!(requests[0].working_dir, PathBuf::from("."));
        assert_eq!(
            requests[0].timeout,
            Duration::from_secs(AggregateIndexBudget::INCREMENTAL.fail_secs)
        );
        assert_eq!(requests[1].argv, ["init", "."]);
        assert_eq!(requests[1].working_dir, PathBuf::from("/aggregate"));
        assert_eq!(
            requests[1].timeout,
            Duration::from_secs(AggregateIndexBudget::FIFTY_MEMBER_INITIAL.fail_secs)
        );
        assert_clean_environment(&requests[1]);
        drop(requests);

        let missing = CodeGraphCli::new(
            Arc::new(ScriptedCodeGraphRunner::missing()),
            "codegraph".into(),
        );
        assert!(matches!(
            missing.verify_version(),
            Err(AggregateIndexError::Degraded { code, .. }) if code == "codegraph_missing"
        ));

        // 同区间补丁版（G2 现场：1.6.1）必须放行。
        for compatible in ["1.6.1\n", "1.7.3\n"] {
            let cli = CodeGraphCli::new(
                Arc::new(ScriptedCodeGraphRunner::from_results(vec![
                    CommandCapture::success(compatible),
                ])),
                "codegraph".into(),
            );
            cli.verify_version()
                .unwrap_or_else(|error| panic!("compatible {compatible:?} rejected: {error}"));
        }

        // 区间外与不可解析：fail-closed 拒绝。
        for rejected in ["1.5.9\n", "2.0.0\n", "2.1.0\n", "not-a-version\n", "1.6\n"] {
            let cli = CodeGraphCli::new(
                Arc::new(ScriptedCodeGraphRunner::from_results(vec![
                    CommandCapture::success(rejected),
                ])),
                "codegraph".into(),
            );
            assert!(
                matches!(
                    cli.verify_version(),
                    Err(AggregateIndexError::Degraded { code, .. })
                        if code == "codegraph_version_mismatch"
                ),
                "version {rejected:?} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn synchronous_api_is_safe_when_called_from_a_tokio_runtime() {
        let runner = Arc::new(ScriptedCodeGraphRunner::from_results(vec![
            CommandCapture::success("Indexed 6 files\n"),
        ]));
        let cli = CodeGraphCli::new(runner, "codegraph".into());

        cli.init(Path::new("/aggregate")).unwrap();
    }

    #[test]
    fn every_supported_command_uses_expected_argv_and_json_is_parsed() {
        let runner = Arc::new(ScriptedCodeGraphRunner::from_results(vec![
            CommandCapture::success("Synced 1 file\n"),
            CommandCapture::success(r#"[{"path":"api/src/lib.rs"},{"path":"web/src/app.ts"}]"#),
            CommandCapture::success(r#"[{"name":"crossRepoGreeting"}]"#),
            CommandCapture::success(
                r#"{"initialized":true,"version":"1.6.0","projectPath":"/aggregate","indexPath":"/aggregate/.codegraph","lastIndexed":"2026-08-09T00:00:00Z","fileCount":2,"nodeCount":3,"edgeCount":4,"dbSizeBytes":5}"#,
            ),
        ]));
        let cli = CodeGraphCli::new(runner.clone(), "codegraph".into());
        let root = Path::new("/aggregate");

        cli.sync(root).unwrap();
        assert_eq!(
            cli.files(root).unwrap(),
            vec![
                PathBuf::from("api/src/lib.rs"),
                PathBuf::from("web/src/app.ts")
            ]
        );
        assert_eq!(
            cli.query_json(root, "crossRepoGreeting").unwrap(),
            serde_json::json!([{"name": "crossRepoGreeting"}])
        );
        let status = cli.status(root).unwrap();
        assert_eq!(status.file_count, 2);
        assert_eq!(status.project_path, PathBuf::from("/aggregate"));

        let requests = runner.requests();
        assert_eq!(requests[0].argv, ["sync", "."]);
        assert_eq!(
            requests[0].timeout,
            Duration::from_secs(AggregateIndexBudget::INCREMENTAL.fail_secs)
        );
        assert_eq!(requests[1].argv, ["files", "--json"]);
        assert_eq!(requests[2].argv, ["query", "crossRepoGreeting", "--json"]);
        assert_eq!(requests[3].argv, ["status", ".", "--json"]);
        for request in requests.iter() {
            assert_eq!(request.working_dir, PathBuf::from("/aggregate"));
            assert_clean_environment(request);
        }
    }

    #[test]
    fn command_failures_timeouts_and_invalid_json_have_machine_readable_codes() {
        let runner = Arc::new(ScriptedCodeGraphRunner::from_results(vec![
            CommandCapture::failure(2, "index broken"),
            CommandCapture::timed_out(),
            CommandCapture::success("not files json"),
            CommandCapture::success("also not json"),
            CommandCapture::success("still not json"),
        ]));
        let cli = CodeGraphCli::new(runner, "codegraph".into());
        let root = Path::new("/aggregate");

        assert!(matches!(
            cli.init(root),
            Err(AggregateIndexError::Failed { code, .. }) if code == "codegraph_init_failed"
        ));
        assert!(matches!(
            cli.sync(root),
            Err(AggregateIndexError::Failed { code, .. }) if code == "codegraph_timeout"
        ));
        assert!(matches!(
            cli.files(root),
            Err(AggregateIndexError::Failed { code, .. }) if code == "codegraph_files_invalid_json"
        ));
        assert!(matches!(
            cli.query_json(root, "symbol"),
            Err(AggregateIndexError::Failed { code, .. }) if code == "codegraph_query_invalid_json"
        ));
        assert!(matches!(
            cli.status(root),
            Err(AggregateIndexError::Failed { code, .. }) if code == "codegraph_status_invalid_json"
        ));
    }

    /// 现场（/tmp/notify-center，成员仓仅 .git+README）：codegraph 1.6 对
    /// 0 文件库执行 `files --json` 以 exit 0 输出人读提示而非 JSON 数组
    /// （`--json` 契约被上游违反）。必须识别该形态返回专用空索引码，
    /// 交给上层归并为业务语义错误，不得把提示文本当 JSON 解析天书透出。
    #[test]
    fn files_on_empty_index_reports_dedicated_code_instead_of_json_gibberish() {
        let runner = Arc::new(ScriptedCodeGraphRunner::from_results(vec![
            CommandCapture::success("ℹ No files indexed. Run \"codegraph index\" first."),
        ]));
        let cli = CodeGraphCli::new(runner, "codegraph".into());
        let error = cli.files(Path::new("/aggregate")).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_files_empty_index"),
            "empty index must be business-semantic, got: {error}"
        );
    }

    /// 现场（聚合根 9k~20k 文件，`codegraph files --json` 载荷 1.3~3.9 MB）：
    /// stdout 超过上限后被截断成 JSON 前缀，解析必然失败。截断必须以专用码
    /// 报出（不得混同为「输出不是合法 JSON」），消息要带上上限字节数。
    #[test]
    fn files_reports_truncated_output_with_dedicated_code() {
        let runner = Arc::new(ScriptedCodeGraphRunner::from_results(vec![
            CommandCapture::truncated_success("[{\"path\":\"src/lib.rs\""),
        ]));
        let cli = CodeGraphCli::new(runner, "codegraph".into());
        let error = cli.files(Path::new("/aggregate")).unwrap_err();
        match error {
            AggregateIndexError::Failed { code, message } => {
                assert_eq!(
                    code, "codegraph_files_output_truncated",
                    "truncated output must not be reported as an invalid-JSON failure"
                );
                assert!(
                    message.contains(&FILES_OUTPUT_LIMIT_BYTES.to_string()),
                    "message must carry the stdout limit in bytes: {message}"
                );
                assert!(
                    message.contains("截断"),
                    "message must state that the output was truncated: {message}"
                );
            }
            other => panic!("expected a failed error, got {other:?}"),
        }
    }

    /// 大负载但**完整**（stdout 未被截断）时，files() 必须照常返回清单：
    /// 现场证明 9k~20k 文件的清单载荷 1.3~3.9 MB，通用 1 MiB 上限会把 JSON
    /// 尾部切掉。用例同时断言「下发的 stdout 上限能容纳整份载荷」——只有上限
    /// 在挡路，解析本身不是问题。
    #[test]
    fn files_parses_large_complete_payload() {
        let entry_count = 20_000usize;
        let mut payload = String::from("[");
        for index in 0..entry_count {
            if index > 0 {
                payload.push(',');
            }
            payload.push_str(&format!(
                "{{\"path\":\"src/aggregate/module_{index:06}/nested/directory/structure/with/a/long/path/file_{index:06}.rs\"}}"
            ));
        }
        payload.push(']');
        assert!(
            payload.len() > OUTPUT_LIMIT_BYTES,
            "载荷必须大于通用上限，用例才有意义：{}",
            payload.len()
        );

        let runner = Arc::new(ScriptedCodeGraphRunner::from_results(vec![
            CommandCapture::success(&payload),
        ]));
        let cli = CodeGraphCli::new(runner.clone(), "codegraph".into());
        let entries = cli.files(Path::new("/aggregate")).unwrap();
        assert_eq!(entries.len(), entry_count);
        assert_eq!(
            entries[0],
            PathBuf::from(format!(
                "src/aggregate/module_{:06}/nested/directory/structure/with/a/long/path/file_{:06}.rs",
                0, 0
            ))
        );
        assert_eq!(
            entries[entry_count - 1],
            PathBuf::from(format!(
                "src/aggregate/module_{:06}/nested/directory/structure/with/a/long/path/file_{:06}.rs",
                entry_count - 1,
                entry_count - 1
            ))
        );

        let requests = runner.requests();
        assert!(
            requests[0].stdout_limit >= payload.len(),
            "stdout 上限必须能容纳整份载荷：limit={} payload={}",
            requests[0].stdout_limit,
            payload.len()
        );
    }

    /// `files` 走专用抓取配额（64 MiB）；`status` / `query_json` 等其余命令
    /// 维持 1 MiB 通用上限（不得被一起放大）。
    ///
    /// owner 2026-11-13 裁决：不传 `--no-metadata`（实测该旗标在 `--json`
    /// 模式下载荷逐字节不变、零收益，却引入「未来 CLI 移除该旗标 ⇒ `files`
    /// 全线失败」的风险）。
    #[test]
    fn files_uses_dedicated_output_limit() {
        let runner = Arc::new(ScriptedCodeGraphRunner::from_results(vec![
            CommandCapture::success(r#"[{"path":"src/lib.rs"}]"#),
            CommandCapture::success(r#"[{"name":"crossRepoGreeting"}]"#),
            CommandCapture::success(
                r#"{"initialized":true,"version":"1.6.0","projectPath":"/aggregate","indexPath":"/aggregate/.codegraph","lastIndexed":"2026-08-09T00:00:00Z","fileCount":1,"nodeCount":1,"edgeCount":1,"dbSizeBytes":1}"#,
            ),
        ]));
        let cli = CodeGraphCli::new(runner.clone(), "codegraph".into());
        let root = Path::new("/aggregate");

        cli.files(root).unwrap();
        cli.query_json(root, "crossRepoGreeting").unwrap();
        cli.status(root).unwrap();

        let requests = runner.requests();
        assert_eq!(requests[0].argv, ["files", "--json"]);
        assert_eq!(requests[0].stdout_limit, FILES_OUTPUT_LIMIT_BYTES);
        assert_eq!(requests[1].stdout_limit, OUTPUT_LIMIT_BYTES);
        assert_eq!(requests[2].stdout_limit, OUTPUT_LIMIT_BYTES);
    }

    fn assert_clean_environment(request: &BoundedCommandRequest) {
        assert!(request.environment.keys().all(|key| key == "PATH"));
        if std::env::var_os("PATH").is_some() {
            assert!(request.environment.contains_key("PATH"));
        }
    }
}
