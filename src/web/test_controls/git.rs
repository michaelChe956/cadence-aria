pub(super) fn init_git_repo(
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    let has_commits = run_git_command_silent(path, &["rev-parse", "HEAD"]).is_ok();
    if has_commits {
        return Ok(());
    }
    run_git_command(path, &["init"])?;
    run_git_command(path, &["config", "user.email", "fixture@example.com"])?;
    run_git_command(path, &["config", "user.name", "Fixture"])?;
    std::fs::write(path.join("README.md"), "# fixture\n")?;
    run_git_command(path, &["add", "."])?;
    run_git_command(path, &["commit", "-m", "initial"])?;
    Ok(())
}

fn run_git_command_silent(
    cwd: &std::path::Path,
    args: &[&str],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err("git command failed".into());
    }
    Ok(())
}

fn run_git_command(
    cwd: &std::path::Path,
    args: &[&str],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {:?} failed: {stderr}", args).into());
    }
    Ok(())
}
