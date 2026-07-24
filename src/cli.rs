use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandRunner, TokioBoundedCommandRunner,
};
use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_health::{ProviderHealthService, SystemProviderHealthClock};
use crate::daemon::discovery::{DaemonStatus, inspect_daemon};
use crate::daemon::runner::{run_daemon_serve_one, run_daemon_until_shutdown};
use crate::product::models::ProviderName;
use crate::product::work_item_draft_evaluation::{
    DEFAULT_RUNS_PER_SCENARIO, DraftEvaluationError, DraftEvaluationReport, compare_reports,
    load_scenarios_from_str, run_evaluation_with_adapter,
};
use crate::protocol::contracts::ProviderType;
use crate::repl::discovery::{DiscoveryMode, resolve_daemon_connection};
use crate::task_run::command::parse_task_run_args;
use crate::task_run::orchestrator::TaskRunOrchestrator;
use crate::task_run::provider_factory::real_routing_provider_with_host_readiness;
use crate::task_run::types::{ReportMode, TaskRunRequest, TaskRunStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliOutput {
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub code: String,
    pub message: String,
}

pub fn run_cli<I, S>(args: I) -> Result<CliOutput, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    match args.as_slice() {
        [command, subcommand, rest @ ..]
            if command == "work-item-draft-eval" && subcommand == "run" =>
        {
            if rest.iter().any(|arg| arg == "--help") {
                return Ok(CliOutput::Text(draft_eval_run_help().to_string()));
            }
            let options = parse_draft_eval_run_options(rest)?;
            if !options.real_provider {
                return Err(CliError {
                    code: "draft_eval_real_provider_required".to_string(),
                    message: "draft evaluation requires explicit --real-provider authorization"
                        .to_string(),
                });
            }
            Err(CliError {
                code: "draft_eval_requires_async".to_string(),
                message: "draft evaluation run is only available through run_cli_async".to_string(),
            })
        }
        [command, subcommand, rest @ ..]
            if command == "work-item-draft-eval" && subcommand == "compare" =>
        {
            compare_draft_eval_reports(rest)
        }
        [command, subcommand, rest @ ..] if command == "daemon" && subcommand == "status" => {
            let workspace = parse_workspace(rest)?;
            let status = inspect_daemon(&workspace).map_err(internal_error)?;
            Ok(CliOutput::Text(match status {
                DaemonStatus::NotFound => "daemon_not_found".to_string(),
                DaemonStatus::Active => "daemon_active".to_string(),
                DaemonStatus::Stale => "daemon_stale".to_string(),
            }))
        }
        [command, subcommand, rest @ ..] if command == "daemon" && subcommand == "run" => {
            let workspace = parse_workspace(rest)?;
            let serve_one = rest.iter().any(|item| item == "--serve-one");
            if serve_one {
                Ok(CliOutput::Text(format!(
                    "daemon_run_serve_one:{}",
                    workspace.to_string_lossy()
                )))
            } else {
                Ok(CliOutput::Text(format!(
                    "daemon_run:{}",
                    workspace.to_string_lossy()
                )))
            }
        }
        [command, rest @ ..] if command == "repl" => {
            let workspace = parse_workspace(rest)?;
            let mode = if rest.iter().any(|item| item == "--no-start") {
                DiscoveryMode::NoStart
            } else {
                DiscoveryMode::AutoStart
            };
            let plan = resolve_daemon_connection(&workspace, mode)?;
            Ok(CliOutput::Text(format!("{plan:?}")))
        }
        [command, subcommand, ..] if command == "task" && subcommand == "run" => Err(CliError {
            code: "task_run_requires_async".to_string(),
            message: "task run is only available through run_cli_async".to_string(),
        }),
        [command, rest @ ..] if command == "web" => {
            let options = parse_web_options(rest)?;
            if options.check {
                return Ok(CliOutput::Text(format!(
                    "web_check_ok:{}:{}:{}",
                    options.workspace.to_string_lossy(),
                    options.host,
                    options
                        .port
                        .map(|port| port.to_string())
                        .unwrap_or_else(|| "auto".to_string())
                )));
            }
            Err(CliError {
                code: "web_requires_async".to_string(),
                message: "web server is only available through run_cli_async".to_string(),
            })
        }
        _ => Err(CliError {
            code: "invalid_cli_args".to_string(),
            message: "expected daemon status, repl, task run, work-item-draft-eval, or web command"
                .to_string(),
        }),
    }
}

pub async fn run_cli_async<I, S>(args: I) -> Result<CliOutput, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    match args.as_slice() {
        [command, subcommand, rest @ ..]
            if command == "work-item-draft-eval" && subcommand == "run" =>
        {
            if rest.iter().any(|arg| arg == "--help") {
                return Ok(CliOutput::Text(draft_eval_run_help().to_string()));
            }
            let options = parse_draft_eval_run_options(rest)?;
            if !options.real_provider {
                return Err(CliError {
                    code: "draft_eval_real_provider_required".to_string(),
                    message: "draft evaluation requires explicit --real-provider authorization"
                        .to_string(),
                });
            }
            let raw_scenarios =
                std::fs::read_to_string(&options.scenario_file).map_err(|error| CliError {
                    code: "draft_eval_scenario_read_failed".to_string(),
                    message: error.to_string(),
                })?;
            let mut scenarios =
                load_scenarios_from_str(&raw_scenarios).map_err(draft_eval_error)?;
            if options.smoke && scenarios.len() > 2 {
                scenarios.truncate(2);
            }

            let command_runner: Arc<dyn BoundedCommandRunner> = Arc::new(TokioBoundedCommandRunner);
            let provider_health = Arc::new(ProviderHealthService::with_dependencies(
                AriaStatePaths::from_workspace_root(&options.workspace),
                command_runner,
                Arc::new(SystemProviderHealthClock),
                Duration::from_secs(5),
                4096,
            ));
            provider_health
                .refresh(tokio_util::sync::CancellationToken::new())
                .await
                .map_err(|error| CliError {
                    code: "provider_health_refresh_failed".to_string(),
                    message: error.to_string(),
                })?;
            let provider_gate = Arc::new(ProviderAvailabilityGate::new(provider_health));
            let provider_name = match &options.provider {
                ProviderType::ClaudeCode => ProviderName::ClaudeCode,
                ProviderType::Codex => ProviderName::Codex,
                ProviderType::Fake => unreachable!("CLI parser rejects fake providers"),
            };
            provider_gate
                .ensure_available(&provider_name)
                .map_err(|error| CliError {
                    code: "provider_unavailable".to_string(),
                    message: error.to_string(),
                })?;
            let provider = real_routing_provider_with_host_readiness(
                provider_gate,
                crate::web::provider_availability::host_real_workflow_ready,
            )
            .map_err(task_run_error)?;
            let report = run_evaluation_with_adapter(
                &provider,
                options.provider,
                &options.workspace,
                &scenarios,
                options.runs_per_scenario,
                options.smoke,
            )
            .map_err(draft_eval_error)?;
            let serialized = serde_json::to_vec_pretty(&report).map_err(internal_error)?;
            std::fs::write(&options.report, serialized).map_err(|error| CliError {
                code: "draft_eval_report_write_failed".to_string(),
                message: error.to_string(),
            })?;
            Ok(CliOutput::Text(format!(
                "draft_eval_report_written:{}",
                options.report.to_string_lossy()
            )))
        }
        [command, subcommand, rest @ ..] if command == "task" && subcommand == "run" => {
            let options = parse_task_run_args(rest).map_err(task_run_error)?;
            let change_id = options
                .change_id
                .clone()
                .unwrap_or_else(|| "aria-login-jwt".to_string());
            let command_runner: Arc<dyn BoundedCommandRunner> = Arc::new(TokioBoundedCommandRunner);
            let provider_health = Arc::new(ProviderHealthService::with_dependencies(
                AriaStatePaths::from_workspace_root(&options.workspace),
                command_runner,
                Arc::new(SystemProviderHealthClock),
                Duration::from_secs(5),
                4096,
            ));
            provider_health
                .refresh(tokio_util::sync::CancellationToken::new())
                .await
                .map_err(|error| {
                    task_run_error(crate::task_run::types::TaskRunError::new(
                        "provider_health_refresh_failed",
                        error.to_string(),
                    ))
                })?;
            let provider_gate = Arc::new(ProviderAvailabilityGate::new(provider_health));
            let provider = real_routing_provider_with_host_readiness(
                provider_gate,
                crate::web::provider_availability::host_real_workflow_ready,
            )
            .map_err(task_run_error)?;
            let outcome = TaskRunOrchestrator::run_with_provider(
                TaskRunRequest {
                    task_id: None,
                    workspace: options.workspace,
                    request_text: options.request_text,
                    change_id,
                    provider_mode: options.provider_mode,
                    non_interactive: options.non_interactive,
                    timeout_secs: options.timeout_secs,
                },
                &provider,
            )
            .map_err(task_run_error)?;
            let text = match options.report_mode {
                ReportMode::Text => format!(
                    "task_id={}\nchange_id={}\nstatus={}\nreport={}",
                    outcome.task_id,
                    outcome.change_id,
                    task_status_text(&outcome.status),
                    outcome.report_path.to_string_lossy()
                ),
                ReportMode::Json => serde_json::to_string_pretty(&serde_json::json!({
                    "task_id": outcome.task_id,
                    "change_id": outcome.change_id,
                    "status": task_status_text(&outcome.status),
                    "report_path": outcome.report_path,
                    "openspec_change_dir": outcome.openspec_change_dir,
                    "provider_run_refs": outcome.provider_run_refs,
                    "testing_report_path": outcome.testing_report_path,
                    "final_summary_path": outcome.final_summary_path,
                    "blocked_report_path": outcome.blocked_report_path,
                }))
                .map_err(internal_error)?,
            };
            Ok(CliOutput::Text(text))
        }
        [command, subcommand, rest @ ..] if command == "daemon" && subcommand == "run" => {
            let workspace = parse_workspace(rest)?;
            let socket = parse_socket(rest);
            if rest.iter().any(|item| item == "--serve-one") {
                run_daemon_serve_one(&workspace, socket)
                    .await
                    .map_err(internal_error)?;
            } else {
                let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
                run_daemon_until_shutdown(&workspace, socket, shutdown_rx)
                    .await
                    .map_err(internal_error)?;
            }
            Ok(CliOutput::Text(String::new()))
        }
        [command, rest @ ..] if command == "web" => {
            let options = parse_web_options(rest)?;
            if options.check {
                return run_cli(args);
            }
            crate::web::app::serve_web(options.workspace, options.host, options.port)
                .await
                .map_err(internal_error)?;
            Ok(CliOutput::Text(String::new()))
        }
        _ => run_cli(args),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebOptions {
    workspace: PathBuf,
    host: String,
    port: Option<u16>,
    check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DraftEvalRunOptions {
    workspace: PathBuf,
    provider: ProviderType,
    scenario_file: PathBuf,
    runs_per_scenario: usize,
    real_provider: bool,
    report: PathBuf,
    smoke: bool,
}

fn parse_draft_eval_run_options(args: &[String]) -> Result<DraftEvalRunOptions, CliError> {
    let workspace = parse_workspace(args)?;
    let provider =
        match required_value(args, "--provider", "draft_eval_provider_required")?.as_str() {
            "codex" => ProviderType::Codex,
            "claude_code" => ProviderType::ClaudeCode,
            value => {
                return Err(CliError {
                    code: "draft_eval_provider_invalid".to_string(),
                    message: format!("unsupported draft evaluation provider {value}"),
                });
            }
        };
    let scenario_file = PathBuf::from(required_value(
        args,
        "--scenario-file",
        "draft_eval_scenario_file_required",
    )?);
    let report = PathBuf::from(required_value(
        args,
        "--report",
        "draft_eval_report_required",
    )?);
    let smoke = args.iter().any(|arg| arg == "--smoke");
    let runs_per_scenario = parse_value(args, "--runs-per-scenario")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| CliError {
            code: "draft_eval_runs_invalid".to_string(),
            message: format!("--runs-per-scenario must be a positive integer: {error}"),
        })?
        .unwrap_or(if smoke { 2 } else { DEFAULT_RUNS_PER_SCENARIO });
    if runs_per_scenario == 0 {
        return Err(CliError {
            code: "draft_eval_runs_invalid".to_string(),
            message: "--runs-per-scenario must be greater than zero".to_string(),
        });
    }
    if smoke && runs_per_scenario > 2 {
        return Err(CliError {
            code: "draft_eval_smoke_limit_exceeded".to_string(),
            message: "--smoke allows at most two runs per scenario".to_string(),
        });
    }
    Ok(DraftEvalRunOptions {
        workspace,
        provider,
        scenario_file,
        runs_per_scenario,
        real_provider: args.iter().any(|arg| arg == "--real-provider"),
        report,
        smoke,
    })
}

fn compare_draft_eval_reports(args: &[String]) -> Result<CliOutput, CliError> {
    let reports = required_value(args, "--reports", "draft_eval_reports_required")?;
    let paths = reports.split(',').collect::<Vec<_>>();
    if paths.len() != 2 || paths.iter().any(|path| path.trim().is_empty()) {
        return Err(CliError {
            code: "draft_eval_reports_invalid".to_string(),
            message: "--reports requires exactly two comma-separated report paths".to_string(),
        });
    }
    let read_report = |path: &str| -> Result<DraftEvaluationReport, CliError> {
        let raw = std::fs::read_to_string(path.trim()).map_err(|error| CliError {
            code: "draft_eval_report_read_failed".to_string(),
            message: error.to_string(),
        })?;
        serde_json::from_str(&raw).map_err(|error| CliError {
            code: "draft_eval_report_parse_failed".to_string(),
            message: error.to_string(),
        })
    };
    let first = read_report(paths[0])?;
    let second = read_report(paths[1])?;
    let comparison = compare_reports(&first, &second).map_err(draft_eval_error)?;
    Ok(CliOutput::Text(
        serde_json::to_string_pretty(&comparison).map_err(internal_error)?,
    ))
}

fn required_value(args: &[String], flag: &str, code: &str) -> Result<String, CliError> {
    parse_value(args, flag).ok_or_else(|| CliError {
        code: code.to_string(),
        message: format!("{flag} is required"),
    })
}

fn draft_eval_run_help() -> &'static str {
    "aria work-item-draft-eval run \\
  --workspace <path> \\
  --provider <codex|claude_code> \\
  --scenario-file <path> \\
  --runs-per-scenario <count> \\
  --real-provider \\
  --report <path> [--smoke]\n\n\
--real-provider          explicitly authorize real provider execution\n\
--runs-per-scenario      release default: 10; smoke default/max: 2\n\
--scenario-file          sanitized evaluation scenario JSON file\n\
--report                 required audited report output path\n\
--smoke                  use at most two scenarios; report is non-release"
}

fn parse_web_options(args: &[String]) -> Result<WebOptions, CliError> {
    let workspace = parse_workspace(args)?;
    let host = parse_value(args, "--host").unwrap_or_else(|| "127.0.0.1".to_string());
    let port = parse_value(args, "--port")
        .map(|value| value.parse::<u16>())
        .transpose()
        .map_err(|error| CliError {
            code: "invalid_cli_args".to_string(),
            message: format!("--port must be a u16: {error}"),
        })?;
    Ok(WebOptions {
        workspace,
        host,
        port,
        check: args.iter().any(|item| item == "--check"),
    })
}

fn parse_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].clone())
}

fn parse_workspace(args: &[String]) -> Result<PathBuf, CliError> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--workspace" {
            let value = args.get(index + 1).ok_or_else(|| CliError {
                code: "invalid_cli_args".to_string(),
                message: "--workspace requires a path".to_string(),
            })?;
            return Ok(Path::new(value).to_path_buf());
        }
        index += 1;
    }

    std::env::current_dir().map_err(internal_error)
}

fn parse_socket(args: &[String]) -> Option<PathBuf> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--socket" {
            return args.get(index + 1).map(PathBuf::from);
        }
        index += 1;
    }
    None
}

fn internal_error(error: impl std::fmt::Display) -> CliError {
    CliError {
        code: "internal_error".to_string(),
        message: error.to_string(),
    }
}

fn draft_eval_error(error: DraftEvaluationError) -> CliError {
    CliError {
        code: error.code,
        message: error.message,
    }
}

fn task_run_error(error: crate::task_run::types::TaskRunError) -> CliError {
    CliError {
        code: error.code,
        message: error.message,
    }
}

fn task_status_text(status: &TaskRunStatus) -> &'static str {
    match status {
        TaskRunStatus::Completed => "completed",
        TaskRunStatus::Failed => "failed",
        TaskRunStatus::BlockedByGate => "blocked_by_gate",
    }
}
