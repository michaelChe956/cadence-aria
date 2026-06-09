use cadence_aria::cli::{CliError, run_cli};
use cadence_aria::task_run::command::parse_task_run_args;
use cadence_aria::task_run::types::{ProviderMode, ReportMode};

#[test]
fn parses_task_run_non_interactive_options() {
    let options = parse_task_run_args(&[
        "--workspace".to_string(),
        "/tmp/naruto-aria-e2e-login".to_string(),
        "--request".to_string(),
        "做一个用户登录功能".to_string(),
        "--change-id".to_string(),
        "aria-login-jwt".to_string(),
        "--providers".to_string(),
        "real".to_string(),
        "--timeout".to_string(),
        "3600".to_string(),
        "--report".to_string(),
        "json".to_string(),
        "--non-interactive".to_string(),
    ])
    .expect("parse task run args");

    assert_eq!(
        options.workspace.to_string_lossy(),
        "/tmp/naruto-aria-e2e-login"
    );
    assert_eq!(options.request_text, "做一个用户登录功能");
    assert_eq!(options.change_id.as_deref(), Some("aria-login-jwt"));
    assert_eq!(options.provider_mode, ProviderMode::Real);
    assert_eq!(options.timeout_secs, 3600);
    assert_eq!(options.report_mode, ReportMode::Json);
    assert!(options.non_interactive);
}

#[test]
fn rejects_missing_request() {
    let error = parse_task_run_args(&[
        "--workspace".to_string(),
        "/tmp/naruto-aria-e2e-login".to_string(),
    ])
    .expect_err("missing request must fail");

    assert_eq!(error.code, "invalid_cli_args");
    assert!(error.message.contains("--request"));
}

#[test]
fn defaults_provider_timeout_report_and_non_interactive() {
    let options = parse_task_run_args(&[
        "--workspace".to_string(),
        "/tmp/naruto-aria-e2e-login".to_string(),
        "--request".to_string(),
        "做一个用户登录功能".to_string(),
    ])
    .expect("parse task run args");

    assert_eq!(options.provider_mode, ProviderMode::Real);
    assert_eq!(options.timeout_secs, 10_800);
    assert_eq!(options.report_mode, ReportMode::Text);
    assert!(!options.non_interactive);
}

#[test]
fn sync_cli_reports_task_run_requires_async_entry() {
    let error = run_cli([
        "task",
        "run",
        "--workspace",
        "/tmp/naruto-aria-e2e-login",
        "--request",
        "做一个用户登录功能",
    ])
    .expect_err("task run must use async entry");

    assert_eq!(
        error,
        CliError {
            code: "task_run_requires_async".to_string(),
            message: "task run is only available through run_cli_async".to_string(),
        }
    );
}
