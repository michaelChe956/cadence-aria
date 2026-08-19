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

pub const CODEGRAPH_EXACT_VERSION: &str = "1.5.0";
const OUTPUT_LIMIT_BYTES: usize = 1024 * 1024;

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

impl CodeGraphCli {
    pub fn new(runner: Arc<dyn BoundedCommandRunner>, executable: String) -> Self {
        Self { runner, executable }
    }

    pub fn verify_v1_5_0(&self) -> Result<(), AggregateIndexError> {
        let output = self.run_checked(
            &["--version"],
            Path::new("."),
            AggregateIndexBudget::INCREMENTAL,
            "codegraph_version_failed",
        )?;
        let actual = output.stdout.trim();
        if actual == CODEGRAPH_EXACT_VERSION {
            Ok(())
        } else {
            Err(AggregateIndexError::Degraded {
                code: "codegraph_version_mismatch",
                message: format!("expected {CODEGRAPH_EXACT_VERSION}, got {actual}"),
            })
        }
    }

    pub fn init(&self, root: &Path) -> Result<CodeGraphCommandResult, AggregateIndexError> {
        self.run_checked(
            &["init", "."],
            root,
            AggregateIndexBudget::FIFTY_MEMBER_INITIAL,
            "codegraph_init_failed",
        )
    }

    pub fn sync(&self, root: &Path) -> Result<CodeGraphCommandResult, AggregateIndexError> {
        self.run_checked(
            &["sync", "."],
            root,
            AggregateIndexBudget::INCREMENTAL,
            "codegraph_sync_failed",
        )
    }

    pub fn files(&self, root: &Path) -> Result<Vec<PathBuf>, AggregateIndexError> {
        let output = self.run_checked(
            &["files", "--json"],
            root,
            AggregateIndexBudget::INCREMENTAL,
            "codegraph_files_failed",
        )?;
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
    ) -> Result<CodeGraphCommandResult, AggregateIndexError> {
        let output = self.run(argv, root, budget.fail_secs)?;
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
    ) -> Result<BoundedCommandResult, AggregateIndexError> {
        let request = BoundedCommandRequest {
            executable: self.executable.clone(),
            argv: argv.iter().map(|argument| (*argument).to_owned()).collect(),
            working_dir: root.to_path_buf(),
            timeout: Duration::from_secs(timeout_secs),
            cancellation: CancellationToken::new(),
            environment: sanitized_environment(),
            stdout_limit: OUTPUT_LIMIT_BYTES,
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
    fn exact_version_is_required_and_missing_binary_becomes_degraded_error() {
        let runner = Arc::new(ScriptedCodeGraphRunner::from_results(vec![
            CommandCapture::success("1.5.0\n"),
            CommandCapture::success("Indexed 6 files\n"),
        ]));
        let cli = CodeGraphCli::new(runner.clone(), "codegraph".into());
        cli.verify_v1_5_0().unwrap();
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
            missing.verify_v1_5_0(),
            Err(AggregateIndexError::Degraded { code, .. }) if code == "codegraph_missing"
        ));

        let mismatched = CodeGraphCli::new(
            Arc::new(ScriptedCodeGraphRunner::from_results(vec![
                CommandCapture::success("1.5.1\\n"),
            ])),
            "codegraph".into(),
        );
        assert!(matches!(
            mismatched.verify_v1_5_0(),
            Err(AggregateIndexError::Degraded { code, .. }) if code == "codegraph_version_mismatch"
        ));
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
                r#"{"initialized":true,"version":"1.5.0","projectPath":"/aggregate","indexPath":"/aggregate/.codegraph","lastIndexed":"2026-08-09T00:00:00Z","fileCount":2,"nodeCount":3,"edgeCount":4,"dbSizeBytes":5}"#,
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

    fn assert_clean_environment(request: &BoundedCommandRequest) {
        assert!(request.environment.keys().all(|key| key == "PATH"));
        if std::env::var_os("PATH").is_some() {
            assert!(request.environment.contains_key("PATH"));
        }
    }
}
