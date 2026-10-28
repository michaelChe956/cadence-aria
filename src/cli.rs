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
use crate::product::app_paths::ProductAppPaths;
use crate::product::dangling_reference::cleanup::{
    CleanupError, CleanupGateError, CleanupRequest, run_cleanup,
};
use crate::product::dangling_reference::patrol::patrol_state_root;
use crate::product::dangling_reference::report::{
    findings_token, render_markdown_report, report_file_name, write_markdown_report,
};
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
        [command, subcommand, rest @ ..]
            if command == "inspect" && subcommand == "dangling-references" =>
        {
            run_dangling_reference_inspection(rest)
        }
        _ => Err(CliError {
            code: "invalid_cli_args".to_string(),
            message: "expected daemon status, repl, task run, web, or inspect command".to_string(),
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
            crate::web::app::serve_web(
                options.workspace,
                options.host,
                options.port,
                options.work_item_plan_single_candidate,
            )
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
    work_item_plan_single_candidate: bool,
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
        work_item_plan_single_candidate: args
            .iter()
            .any(|item| item == "--work-item-plan-single-candidate"),
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
            let workspace = Path::new(value);
            if workspace.is_absolute() {
                return Ok(workspace.to_path_buf());
            }
            return std::env::current_dir()
                .map(|current_dir| current_dir.join(workspace))
                .map_err(internal_error);
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

// ---------------------------------------------------------------------------
// 悬空引用巡检与销账（`aria inspect dangling-references`）
//
// 三段语义（REQ-DRG-10 / 13 / 22）：
// ① 默认 dry-run：只出清单，零删除零留痕；
// ② `--apply` 必须同时给出 `--confirm <token>`，且令牌必须等于入口内部重算的
//    findings 摘要——CLI 侧先做一次判定，保证拒绝路径连清单都不落盘；
// ③ 真正的销账动作交给 `run_cleanup`（三道闸在其内部强制，闸 1 会在内部
//    重跑一次巡检并比对令牌），CLI 不绕过闸 1。
// ---------------------------------------------------------------------------

/// 悬空引用巡检子命令的选项。
#[derive(Debug, Clone, PartialEq, Eq)]
struct DanglingReferenceOptions {
    /// 是否开启销账（默认 `false` = dry-run）。
    apply: bool,
    /// 确认令牌（`sha256:<64 位小写 hex>`）。
    confirm_token: Option<String>,
    /// 清单落点目录覆盖值（缺省 = `<workspace>/cadence/analysis-docs/`）。
    report_dir: Option<String>,
}

/// 解析 `aria inspect dangling-references` 的子命令参数。
///
/// `--apply` / `--confirm <token>` / `--report-dir <dir>` 三个开关；未知参数
/// 忽略（与既有 `parse_web_options` 的宽松口径一致）。
fn parse_dangling_reference_options(rest: &[String]) -> Result<DanglingReferenceOptions, CliError> {
    let mut apply = false;
    let mut confirm_token = None;
    let mut report_dir = None;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--apply" => apply = true,
            "--confirm" => {
                index += 1;
                let value = rest.get(index).ok_or_else(|| CliError {
                    code: "invalid_cli_args".to_string(),
                    message: "--confirm 需要一个令牌值".to_string(),
                })?;
                confirm_token = Some(value.clone());
            }
            "--report-dir" => {
                index += 1;
                let value = rest.get(index).ok_or_else(|| CliError {
                    code: "invalid_cli_args".to_string(),
                    message: "--report-dir 需要一个目录值".to_string(),
                })?;
                report_dir = Some(value.clone());
            }
            _ => {}
        }
        index += 1;
    }
    Ok(DanglingReferenceOptions {
        apply,
        confirm_token,
        report_dir,
    })
}

/// 巡检入口：巡检 → 令牌 → 清单落盘 →（可选）销账。
fn run_dangling_reference_inspection(rest: &[String]) -> Result<CliOutput, CliError> {
    let options = parse_dangling_reference_options(rest)?;
    let workspace = parse_workspace(rest)?;
    let aria_root = AriaStatePaths::from_workspace_root(&workspace)
        .aria_root()
        .to_path_buf();
    let paths = ProductAppPaths::new(aria_root);

    let report = patrol_state_root(&paths).map_err(|error| CliError {
        code: "dangling_reference_patrol_failed".to_string(),
        message: error.to_string(),
    })?;
    let token = findings_token(&report.findings);

    // 闸 1（CLI 侧预检）：拒绝路径必须零删除、零留痕——连清单都不落盘。
    if options.apply {
        let provided = options.confirm_token.as_deref().ok_or_else(|| CliError {
            code: "dangling_reference_confirm_token_missing".to_string(),
            message: "销账被拒绝：缺少确认令牌（--apply 必须同时给出 --confirm <token>）"
                .to_string(),
        })?;
        if provided != token.as_str() {
            return Err(CliError {
                code: "dangling_reference_confirm_token_mismatch".to_string(),
                message: format!(
                    "销账被拒绝：确认令牌与当前巡检结果不匹配（本次重算 {token}，传入 {provided}）"
                ),
            });
        }
    }

    let report_path = dangling_reference_report_path(&workspace, options.report_dir.as_deref());
    let content = render_markdown_report(&report, &token, &chrono::Utc::now().to_rfc3339());
    write_markdown_report(&report_path, &content).map_err(|error| CliError {
        code: "dangling_reference_report_write_failed".to_string(),
        message: error.to_string(),
    })?;

    let deleted_count = if options.apply {
        let outcome = run_cleanup(
            &paths,
            &CleanupRequest {
                apply: true,
                confirm_token: options.confirm_token.clone(),
            },
        )
        .map_err(dangling_reference_cleanup_error)?;
        outcome.deleted_count
    } else {
        0
    };

    Ok(CliOutput::Text(format!(
        "dangling_reference_report_path:{}\n\
         dangling_reference_token:{}\n\
         dangling_reference_dangling_count:{}\n\
         dangling_reference_undetermined_count:{}\n\
         dangling_reference_deleted_count:{}",
        report_path.to_string_lossy(),
        token,
        report.findings.len(),
        report.undetermined.len(),
        deleted_count,
    )))
}

/// 巡检清单落点：默认 `<workspace>/cadence/analysis-docs/`，`--report-dir` 可覆盖
/// （相对路径按工作区解析，绝对路径按原样使用）。
fn dangling_reference_report_path(workspace: &Path, report_dir: Option<&str>) -> PathBuf {
    let directory = match report_dir {
        Some(value) => {
            let candidate = Path::new(value);
            if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                workspace.join(candidate)
            }
        }
        None => workspace.join("cadence").join("analysis-docs"),
    };
    directory.join(report_file_name(&dangling_reference_run_day()))
}

/// 清单文件名的运行日（UTC 日期，形如 `2026-10-28`）。
fn dangling_reference_run_day() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// 销账错误 → CLI 稳定错误码（闸 1 的两种拒绝与存储失败各占一个码）。
fn dangling_reference_cleanup_error(error: CleanupError) -> CliError {
    let message = error.to_string();
    let code = match &error {
        CleanupError::Gate(CleanupGateError::TokenMissing) => {
            "dangling_reference_confirm_token_missing"
        }
        CleanupError::Gate(CleanupGateError::TokenMismatch { .. }) => {
            "dangling_reference_confirm_token_mismatch"
        }
        CleanupError::Store(_) => "dangling_reference_cleanup_failed",
    };
    CliError {
        code: code.to_string(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CliOutput, WebOptions, parse_dangling_reference_options, parse_web_options,
        parse_workspace, run_cli,
    };
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    #[test]
    fn parse_web_options_enables_single_candidate_rollout_flag() {
        let options = parse_web_options(&[
            "--workspace".to_string(),
            ".".to_string(),
            "--work-item-plan-single-candidate".to_string(),
        ])
        .expect("parse web options");

        assert_eq!(
            options,
            WebOptions {
                workspace: std::env::current_dir().expect("current directory"),
                host: "127.0.0.1".to_string(),
                port: None,
                check: false,
                work_item_plan_single_candidate: true,
            }
        );
    }

    #[test]
    fn parse_web_options_defaults_single_candidate_rollout_flag_to_disabled() {
        let options = parse_web_options(&[]).expect("parse web options");

        assert!(!options.work_item_plan_single_candidate);
    }

    #[test]
    fn parse_workspace_makes_relative_workspace_absolute() {
        let workspace = parse_workspace(&["--workspace".to_string(), ".".to_string()]).unwrap();
        let current_dir = std::env::current_dir().unwrap();

        assert!(workspace.is_absolute());
        assert_eq!(
            workspace.canonicalize().unwrap(),
            current_dir.canonicalize().unwrap()
        );
    }

    #[test]
    fn parse_workspace_preserves_absolute_workspace() {
        let workspace = parse_workspace(&["--workspace".to_string(), "/tmp".to_string()]).unwrap();

        assert_eq!(workspace, PathBuf::from("/tmp"));
    }

    #[test]
    fn parse_workspace_without_workspace_uses_current_directory() {
        let workspace = parse_workspace(&[]).unwrap();

        assert_eq!(workspace, std::env::current_dir().unwrap());
    }

    // ------------------------------------------------------------------
    // 悬空引用巡检与销账子命令（`aria inspect dangling-references`）
    // ------------------------------------------------------------------

    #[test]
    fn parse_dangling_reference_options_reads_apply_and_confirm() {
        let options = parse_dangling_reference_options(&[
            "--apply".to_string(),
            "--confirm".to_string(),
            "sha256:abc".to_string(),
        ])
        .expect("parse options");
        assert!(options.apply);
        assert_eq!(options.confirm_token.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn parse_dangling_reference_options_defaults_to_dry_run() {
        let options = parse_dangling_reference_options(&[]).expect("parse options");
        assert!(!options.apply);
        assert!(options.confirm_token.is_none());
    }

    #[test]
    fn parse_dangling_reference_options_reads_report_dir() {
        let options =
            parse_dangling_reference_options(&["--report-dir".to_string(), "reports".to_string()])
                .expect("parse options");
        assert!(!options.apply);
        assert_eq!(options.report_dir.as_deref(), Some("reports"));
    }

    #[test]
    fn parse_dangling_reference_options_rejects_flag_without_value() {
        let error = parse_dangling_reference_options(&["--confirm".to_string()])
            .expect_err("--confirm 缺值必须报错");
        assert_eq!(error.code, "invalid_cli_args");

        let error = parse_dangling_reference_options(&["--report-dir".to_string()])
            .expect_err("--report-dir 缺值必须报错");
        assert_eq!(error.code, "invalid_cli_args");
    }

    #[test]
    fn inspect_dangling_references_dry_run_emits_five_lines_writes_report_and_deletes_nothing() {
        let workspace = dangling_reference_workspace();
        let state_root = dangling_reference_state_root(workspace.path());
        let before = dangling_reference_state_files(&state_root);

        let output =
            run_cli(dangling_reference_args(workspace.path(), &[])).expect("悬空引用巡检应当成功");
        let CliOutput::Text(text) = output;

        assert_eq!(text.lines().count(), 5, "输出必须是逐行五项：{text}");
        let values = dangling_reference_output(&text);

        let expected_report = dangling_reference_report_file(workspace.path());
        assert_eq!(
            values.get("dangling_reference_report_path"),
            Some(&expected_report.to_string_lossy().to_string())
        );

        let token = values.get("dangling_reference_token").expect("令牌行");
        assert!(
            token.starts_with("sha256:"),
            "令牌必须是 sha256 前缀：{token}"
        );
        let digest = token.trim_start_matches("sha256:");
        assert_eq!(digest.len(), 64, "摘要必须是 64 位十六进制：{token}");
        assert!(
            digest
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()),
            "摘要必须是小写十六进制：{token}"
        );

        assert_eq!(
            values.get("dangling_reference_dangling_count"),
            Some(&"0".to_string())
        );
        assert_eq!(
            values.get("dangling_reference_undetermined_count"),
            Some(&"0".to_string())
        );
        assert_eq!(
            values.get("dangling_reference_deleted_count"),
            Some(&"0".to_string()),
            "默认 dry-run 必须零删除"
        );

        // 清单已落盘，且正文含令牌行。
        assert!(
            expected_report.exists(),
            "巡检清单必须落盘：{expected_report:?}"
        );
        let content = std::fs::read_to_string(&expected_report).expect("读取巡检清单");
        assert!(
            content.contains(token.as_str()),
            "清单必须附带摘要令牌：{content}"
        );

        // 零删除零留痕：状态根逐文件不变，台账不存在。
        assert_eq!(
            before,
            dangling_reference_state_files(&state_root),
            "dry-run 不得改动状态根"
        );
        assert!(
            !dangling_reference_ledger_file(workspace.path()).exists(),
            "dry-run 不得留痕"
        );
    }

    #[test]
    fn inspect_dangling_references_report_dir_overrides_default_location() {
        let workspace = dangling_reference_workspace();
        let report_dir = tempfile::tempdir().expect("报告目录");

        let output = run_cli(dangling_reference_args(
            workspace.path(),
            &["--report-dir", &report_dir.path().to_string_lossy()],
        ))
        .expect("悬空引用巡检应当成功");
        let CliOutput::Text(text) = output;
        let values = dangling_reference_output(&text);

        let overridden = report_dir
            .path()
            .join(dangling_reference_report_file_name());
        let default = dangling_reference_report_file(workspace.path());
        assert_eq!(
            values.get("dangling_reference_report_path"),
            Some(&overridden.to_string_lossy().to_string())
        );
        assert!(
            overridden.exists(),
            "清单必须落到 --report-dir：{overridden:?}"
        );
        assert!(!default.exists(), "默认落点不应被创建：{default:?}");
    }

    #[test]
    fn inspect_dangling_references_apply_without_confirm_token_is_rejected() {
        let workspace = dangling_reference_workspace();
        let state_root = dangling_reference_state_root(workspace.path());
        let before = dangling_reference_state_files(&state_root);

        let error = run_cli(dangling_reference_args(workspace.path(), &["--apply"]))
            .expect_err("--apply 缺 --confirm 必须被拒绝");
        assert_eq!(error.code, "dangling_reference_confirm_token_missing");

        assert_eq!(
            before,
            dangling_reference_state_files(&state_root),
            "闸 1 拒绝必须零删除"
        );
        assert!(
            !dangling_reference_ledger_file(workspace.path()).exists(),
            "闸 1 拒绝必须零留痕"
        );
        assert!(
            !dangling_reference_report_file(workspace.path()).exists(),
            "闸 1 拒绝不得落盘清单"
        );
    }

    #[test]
    fn inspect_dangling_references_apply_with_mismatched_token_is_rejected() {
        let workspace = dangling_reference_workspace();
        let state_root = dangling_reference_state_root(workspace.path());
        let before = dangling_reference_state_files(&state_root);

        let wrong_token = format!("sha256:{}", "0".repeat(64));
        let error = run_cli(dangling_reference_args(
            workspace.path(),
            &["--apply", "--confirm", &wrong_token],
        ))
        .expect_err("令牌不匹配必须被拒绝");
        assert_eq!(error.code, "dangling_reference_confirm_token_mismatch");

        assert_eq!(
            before,
            dangling_reference_state_files(&state_root),
            "闸 1 拒绝必须零删除"
        );
        assert!(
            !dangling_reference_ledger_file(workspace.path()).exists(),
            "闸 1 拒绝必须零留痕"
        );
        assert!(
            !dangling_reference_report_file(workspace.path()).exists(),
            "闸 1 拒绝不得落盘清单"
        );
    }

    #[test]
    fn inspect_dangling_references_apply_with_matching_token_runs_cleanup() {
        let workspace = dangling_reference_workspace();

        let CliOutput::Text(dry_run_text) =
            run_cli(dangling_reference_args(workspace.path(), &[])).expect("dry-run 应当成功");
        let token = dangling_reference_output(&dry_run_text)
            .get("dangling_reference_token")
            .expect("令牌行")
            .clone();

        let CliOutput::Text(text) = run_cli(dangling_reference_args(
            workspace.path(),
            &["--apply", "--confirm", &token],
        ))
        .expect("令牌匹配的销账应当成功");
        let values = dangling_reference_output(&text);

        assert_eq!(
            values.get("dangling_reference_token"),
            Some(&token),
            "销账入口重算的令牌必须与确认令牌一致"
        );
        assert_eq!(
            values.get("dangling_reference_deleted_count"),
            Some(&"0".to_string()),
            "空状态根没有可删条目，删除数必须为零"
        );
        assert!(
            !dangling_reference_ledger_file(workspace.path()).exists(),
            "没有实际删除时不得留痕"
        );
    }

    /// 悬空引用巡检用的临时工作区（状态根为 `<workspace>/.aria`）。
    fn dangling_reference_workspace() -> tempfile::TempDir {
        let workspace = tempfile::tempdir().expect("临时工作区");
        std::fs::create_dir_all(dangling_reference_state_root(workspace.path()))
            .expect("创建状态根");
        workspace
    }

    /// 巡检子命令的参数（`--workspace` 固定指向临时工作区）。
    fn dangling_reference_args(workspace: &Path, extra: &[&str]) -> Vec<String> {
        let mut args = vec![
            "inspect".to_string(),
            "dangling-references".to_string(),
            "--workspace".to_string(),
            workspace.to_string_lossy().to_string(),
        ];
        args.extend(extra.iter().map(|value| value.to_string()));
        args
    }

    /// 巡检子命令的输出行解析（逐行 `key:value`）。
    fn dangling_reference_output(text: &str) -> BTreeMap<String, String> {
        text.lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// 状态根（`<workspace>/.aria`）。
    fn dangling_reference_state_root(workspace: &Path) -> PathBuf {
        workspace.join(".aria")
    }

    /// 留痕台账落点（`<state_root>/state/entity-deletions/dangling-reference/cleanup.jsonl`）。
    fn dangling_reference_ledger_file(workspace: &Path) -> PathBuf {
        dangling_reference_state_root(workspace)
            .join("state")
            .join("entity-deletions")
            .join("dangling-reference")
            .join("cleanup.jsonl")
    }

    /// 巡检清单文件名（`<运行日>_分析报告_悬空引用巡检清单_v1.0.md`）。
    fn dangling_reference_report_file_name() -> String {
        crate::product::dangling_reference::report::report_file_name(
            &super::dangling_reference_run_day(),
        )
    }

    /// 巡检清单默认落点（`<workspace>/cadence/analysis-docs/<文件名>`）。
    fn dangling_reference_report_file(workspace: &Path) -> PathBuf {
        workspace
            .join("cadence")
            .join("analysis-docs")
            .join(dangling_reference_report_file_name())
    }

    /// 状态根下的全部文件（相对路径升序）——用于「逐文件不变」断言。
    fn dangling_reference_state_files(root: &Path) -> Vec<String> {
        let mut files = Vec::new();
        dangling_reference_collect_files(root, root, &mut files);
        files.sort();
        files
    }

    fn dangling_reference_collect_files(root: &Path, dir: &Path, files: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dangling_reference_collect_files(root, &path, files);
                continue;
            }
            files.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
}
