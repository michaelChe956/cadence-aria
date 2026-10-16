use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommandRecord {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitCommandError {
    #[error("git command io error: {0}")]
    Io(String),
    #[error("git command failed: {record:?}")]
    Failed { record: GitCommandRecord },
}

/// 平台受信 Git 禁桥参数(Bug45/T4):平台在 owner 上下文执行的 git,一律
/// 在子命令之前关闭「仓库执行桥」——coder 可写的 `.git/hooks`(经
/// `core.hooksPath` 定位)与 `core.fsmonitor` 探针命令不得借平台进程自动
/// 执行。命令级 `-c` 覆盖优先于仓库/全局配置且不落盘,因此既不改仓库
/// config,也不影响 coder 自己域内(终端/沙箱)git 的 hooks 语义——两个
/// 信任域不得混用:开发者提交保护 hook 只在 coder 自己的 git 里生效。
pub const TRUSTED_GIT_BRIDGE_ARGS: [&str; 4] = [
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "core.fsmonitor=false",
];

pub fn run_git(cwd: &Path, args: &[String]) -> Result<GitCommandRecord, GitCommandError> {
    // 审计记录必须反映实际执行 argv:禁桥参数固定在最前(见
    // [`TRUSTED_GIT_BRIDGE_ARGS`]),其后才是调用方子命令。
    let mut argv: Vec<String> = TRUSTED_GIT_BRIDGE_ARGS
        .iter()
        .map(|argument| (*argument).to_string())
        .collect();
    argv.extend(args.iter().cloned());
    let output = Command::new("git")
        .args(&argv)
        .current_dir(cwd)
        .output()
        .map_err(|error| GitCommandError::Io(error.to_string()))?;
    let record = GitCommandRecord {
        cwd: cwd.to_path_buf(),
        args: argv,
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    };
    if output.status.success() {
        Ok(record)
    } else {
        Err(GitCommandError::Failed { record })
    }
}

pub fn git_stdout(cwd: &Path, args: &[String]) -> Result<String, GitCommandError> {
    run_git(cwd, args).map(|record| record.stdout.trim().to_string())
}

pub fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;

    /// Bug45/T4 fixture 根:遵守批2 GC5(测试临时物在 /tmp/aria-bug45/<task>
    /// 私有子目录,不散落 owner HOME 或真实仓库)。
    fn t4_fixture_dir() -> tempfile::TempDir {
        let base = Path::new("/tmp/aria-bug45/t4");
        fs::create_dir_all(base).expect("create /tmp/aria-bug45/t4 fixture base");
        tempfile::tempdir_in(base).expect("fixture dir under /tmp/aria-bug45/t4")
    }

    /// coder 域对照执行器:无任何禁桥参数的原始 git。
    fn raw_git(cwd: &Path, argv: &[&str]) {
        let output = std::process::Command::new("git")
            .args(argv)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .output()
            .expect("raw fixture git");
        assert!(
            output.status.success(),
            "git {argv:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).expect("write executable fixture");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("chmod executable fixture");
    }

    /// Bug45 夹具:模拟 coder 植入的「仓库执行桥」——`.git/hooks/pre-commit`
    /// 与 `core.fsmonitor` 探针脚本。两者只落标记文件、恒 exit 0/1,不影响
    /// git 自身成败;平台受信 git 不得触发任何一个。
    fn repo_with_execution_bridges() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let temp = t4_fixture_dir();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        raw_git(&repo, &["init", "--quiet", "-b", "main"]);
        raw_git(&repo, &["config", "user.email", "aria-t4@example.invalid"]);
        raw_git(&repo, &["config", "user.name", "Aria T4"]);
        fs::write(repo.join("README.md"), "# bridge fixture\n").unwrap();
        raw_git(&repo, &["add", "README.md"]);
        raw_git(&repo, &["commit", "--quiet", "-m", "initial"]);

        let pre_commit_probe = repo.join(".git/aria-pre-commit-fired");
        write_executable(
            &repo.join(".git/hooks/pre-commit"),
            &format!("#!/bin/sh\n: > {}\nexit 0\n", pre_commit_probe.display()),
        );
        let fsmonitor_probe = repo.join(".git/aria-fsmonitor-fired");
        let fsmonitor_hook = repo.join(".git/aria-fsmonitor-hook.sh");
        write_executable(
            &fsmonitor_hook,
            &format!("#!/bin/sh\n: > {}\nexit 1\n", fsmonitor_probe.display()),
        );
        raw_git(
            &repo,
            &[
                "config",
                "core.fsmonitor",
                fsmonitor_hook.to_str().expect("utf8 fsmonitor hook path"),
            ],
        );
        (temp, repo, pre_commit_probe, fsmonitor_probe)
    }

    #[test]
    fn run_git_status_does_not_spawn_repo_fsmonitor_probe() {
        let (_temp, repo, _pre_commit_probe, fsmonitor_probe) = repo_with_execution_bridges();

        let record = run_git(&repo, &args(&["status", "--porcelain"]))
            .expect("platform run_git status must succeed");
        assert!(
            !fsmonitor_probe.exists(),
            "Bug45: 平台 run_git 不得执行仓库 core.fsmonitor 探针"
        );
        // 原始 args 审计必须反映实际执行的禁桥参数(Bug45/T4 Produces)。
        assert_eq!(
            record.args,
            vec![
                "-c".to_string(),
                "core.hooksPath=/dev/null".to_string(),
                "-c".to_string(),
                "core.fsmonitor=false".to_string(),
                "status".to_string(),
                "--porcelain".to_string(),
            ],
            "GitCommandRecord.args 审计须包含前置禁桥参数"
        );

        // 夹具有效性对照:同一仓库在 coder 域(无禁桥)git 下探针会触发。
        raw_git(&repo, &["status", "--porcelain"]);
        assert!(
            fsmonitor_probe.exists(),
            "对照失败:coder 域 status 应触发其自身 fsmonitor 探针,夹具失效"
        );
    }

    #[test]
    fn run_git_commit_does_not_execute_repo_pre_commit_hook() {
        let (_temp, repo, pre_commit_probe, fsmonitor_probe) = repo_with_execution_bridges();
        fs::write(repo.join("platform-change.txt"), "staged by platform\n").unwrap();
        raw_git(&repo, &["add", "platform-change.txt"]);
        // coder 域 staging 可能触发 fsmonitor 探针,清标记后再断言平台面。
        let _ = fs::remove_file(&fsmonitor_probe);
        let _ = fs::remove_file(&pre_commit_probe);

        run_git(&repo, &args(&["commit", "--quiet", "-m", "platform bridge"]))
            .expect("platform run_git commit must succeed");
        assert!(
            !pre_commit_probe.exists(),
            "Bug45: 平台 run_git commit 不得执行仓库 pre-commit hook"
        );
        assert!(
            !fsmonitor_probe.exists(),
            "Bug45: 平台 run_git commit 亦不得执行仓库 core.fsmonitor 探针"
        );
    }

    /// 信任域对照:coder 自己的 git(无禁桥)仍执行其仓库 hooks 与
    /// fsmonitor 探针——平台禁桥不得外溢成「禁 Git 能力」或改仓库语义。
    #[test]
    fn coder_domain_git_still_runs_repo_hooks_and_fsmonitor() {
        let (_temp, repo, pre_commit_probe, fsmonitor_probe) = repo_with_execution_bridges();

        raw_git(&repo, &["status", "--porcelain"]);
        assert!(
            fsmonitor_probe.exists(),
            "coder 域 status 应触发其自身 fsmonitor 探针"
        );

        fs::write(repo.join("coder-change.txt"), "coder keeps hooks\n").unwrap();
        raw_git(&repo, &["add", "coder-change.txt"]);
        raw_git(&repo, &["commit", "--quiet", "-m", "coder keeps hooks"]);
        assert!(
            pre_commit_probe.exists(),
            "coder 域 commit 应执行其自身 pre-commit hook(开发提交保护不被绕过)"
        );
    }
}
