use cadence_aria::cli::{CliOutput, run_cli, run_cli_async};
use tempfile::tempdir;

#[test]
fn web_check_reports_workspace_and_bind_address() {
    let workspace = tempdir().expect("workspace");
    let output = run_cli([
        "web",
        "--workspace",
        workspace.path().to_str().expect("path"),
        "--host",
        "127.0.0.1",
        "--port",
        "4317",
        "--check",
    ])
    .expect("cli");

    assert_eq!(
        output,
        CliOutput::Text(format!(
            "web_check_ok:{}:127.0.0.1:4317",
            workspace.path().display()
        ))
    );
}

#[tokio::test]
async fn async_web_check_uses_same_parser() {
    let workspace = tempdir().expect("workspace");
    let output = run_cli_async([
        "web",
        "--workspace",
        workspace.path().to_str().expect("path"),
        "--check",
    ])
    .await
    .expect("cli");

    assert!(matches!(output, CliOutput::Text(text) if text.starts_with("web_check_ok:")));
}
