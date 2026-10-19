use std::fs;
use std::process::Command;

use crate::interactive::models::WebWorkspaceProjection;
use crate::interactive::projection::build_workspace_projection;
use crate::interactive::web_projection::build_web_projection;
use crate::task_run::types::TaskRunError;
use crate::web::types::{ArtifactContentResponse, FileContentResponse, FileDiffResponse};

use super::WebRuntime;
use super::utils::{content_type_for_path, io_error, safe_workspace_path};

impl WebRuntime {
    pub fn projection(
        &self,
        task_id: Option<&str>,
        selected_node_id: Option<&str>,
    ) -> Result<WebWorkspaceProjection, TaskRunError> {
        Self::projection_for_workspace(&self.workspace_root, task_id, selected_node_id)
    }

    pub fn projection_for_workspace(
        workspace_root: &std::path::Path,
        task_id: Option<&str>,
        selected_node_id: Option<&str>,
    ) -> Result<WebWorkspaceProjection, TaskRunError> {
        let base = build_workspace_projection(workspace_root, task_id)?;
        build_web_projection(workspace_root, base, selected_node_id)
    }

    pub fn artifact_content(
        &self,
        artifact_ref: &str,
    ) -> Result<ArtifactContentResponse, TaskRunError> {
        let projection = self.projection(None, None)?;
        let entry = projection
            .artifact_index
            .iter()
            .find(|entry| entry.artifact_ref == artifact_ref)
            .ok_or_else(|| {
                TaskRunError::new(
                    "artifact_not_found",
                    format!("artifact not found: {artifact_ref}"),
                )
            })?;
        let path = self.workspace_root.join(&entry.path);
        let content = fs::read_to_string(path).map_err(io_error)?;
        Ok(ArtifactContentResponse {
            artifact_ref: entry.artifact_ref.clone(),
            artifact_kind: entry.artifact_kind.clone(),
            producer_node: entry.producer_node.clone(),
            path: entry.path.clone(),
            content_type: format!("{:?}", entry.content_type).to_lowercase(),
            content,
        })
    }

    pub fn file_content(&self, path: &str) -> Result<FileContentResponse, TaskRunError> {
        let safe = safe_workspace_path(&self.workspace_root, path)?;
        Ok(FileContentResponse {
            path: path.to_string(),
            content_type: content_type_for_path(path),
            content: fs::read_to_string(safe).map_err(io_error)?,
        })
    }

    pub fn file_diff(
        &self,
        base_checkpoint: &str,
        path: &str,
    ) -> Result<FileDiffResponse, TaskRunError> {
        let _ = safe_workspace_path(&self.workspace_root, path)?;
        validate_base_checkpoint_shape(base_checkpoint)?;
        let resolved = resolve_base_checkpoint(&self.workspace_root, base_checkpoint)?;
        // Bug45/T4+B3 修复轮:file_diff 对 coder/reviewer 可写的 workspace 仓
        // 执行 diff,属平台受信启动面。
        // - 子命令之前注入禁桥参数(`core.hooksPath`/`core.fsmonitor`);
        // - `--no-ext-diff` 关闭 `diff.external` 外部命令;
        // - `--no-textconv` 关闭 `diff.<driver>.textconv`(`.gitattributes`
        //   与仓库 config 均可由 coder 写入);
        // - 基准先经 `rev-parse --verify --end-of-options` 解析成对象号,
        //   再作为 diff 的 revision 参数——`--` 仍是 revision/pathspec 分界,
        //   不前移,基准的选项注入在形态白名单与解析层双重封死。
        let diff = Command::new("git")
            .args(crate::cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS)
            .args([
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                &resolved,
                "--",
                path,
            ])
            .current_dir(&self.workspace_root)
            .output()
            .map_err(|error| TaskRunError::new("git_command_failed", error.to_string()))?;
        if !diff.status.success() {
            return Err(git_failure("git_diff_failed", "git diff", &diff));
        }
        Ok(FileDiffResponse {
            base_checkpoint: base_checkpoint.to_string(),
            path: path.to_string(),
            diff: String::from_utf8_lossy(&diff.stdout).to_string(),
        })
    }
}

/// base_checkpoint 形态白名单(Bug45/F4):拒空、拒超长、拒前导 `-`、
/// 拒空白/控制字符/引号/通配符/`:`/`\` 与 `..` 路径穿越;放行 40/64 hex、
/// 分支名、`HEAD~N`、`ckpt_*` 等合法命名。形态通过不等于对象存在——不存在的
/// 合法引用由后续 `rev-parse` 报真实 git 错误(F20),绝不冒充“无差异”。
fn validate_base_checkpoint_shape(value: &str) -> Result<(), TaskRunError> {
    const MAX_BASE_CHECKPOINT_BYTES: usize = 256;
    let invalid = || {
        TaskRunError::new(
            "invalid_base_checkpoint",
            format!("invalid base checkpoint: {value:?}"),
        )
    };
    if value.is_empty()
        || value.len() > MAX_BASE_CHECKPOINT_BYTES
        || value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("..")
        || value.contains("//")
    {
        return Err(invalid());
    }
    let well_formed = value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'.' | b'_' | b'/' | b'~' | b'^' | b'@' | b'{' | b'}' | b'-'
            )
    });
    if !well_formed {
        return Err(invalid());
    }
    Ok(())
}

/// 把合法形态的基准解析为真实 commit 对象号:失败即上抛 git 错误(含
/// 有限 stderr 摘要),不再返回空 diff。
fn resolve_base_checkpoint(
    workspace_root: &std::path::Path,
    base_checkpoint: &str,
) -> Result<String, TaskRunError> {
    let expression = format!("{base_checkpoint}^{{commit}}");
    let output = Command::new("git")
        .args(crate::cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS)
        .args(["rev-parse", "--verify", "--end-of-options", &expression])
        .current_dir(workspace_root)
        .output()
        .map_err(|error| TaskRunError::new("git_command_failed", error.to_string()))?;
    if !output.status.success() {
        return Err(git_failure(
            "git_rev_parse_failed",
            "git rev-parse",
            &output,
        ));
    }
    let resolved = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !(resolved.len() == 40 || resolved.len() == 64)
        || !resolved.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(TaskRunError::new(
            "git_rev_parse_failed",
            format!("git rev-parse returned no usable object id for {base_checkpoint:?}"),
        ));
    }
    Ok(resolved)
}

/// 非零退出的 git 产物 → typed 错误:保留命令名、退出码与有限 stderr 摘要,
/// 便于 REST 层暴露真实失败原因。
fn git_failure(code: &str, command: &str, output: &std::process::Output) -> TaskRunError {
    const STDERR_SUMMARY_LIMIT: usize = 512;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let summary = stderr
        .trim()
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(STDERR_SUMMARY_LIMIT)
        .collect::<String>();
    let status = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string());
    TaskRunError::new(
        code,
        format!("{command} failed (exit {status}): {}", summary.trim()),
    )
}

#[cfg(test)]
mod bridge_tests {
    use super::super::WebRuntime;
    use std::path::Path;

    /// Bug45/T4 修复轮:file_diff 是平台受信启动面,不得执行 coder 可写仓
    /// 的 core.fsmonitor 探针。
    #[test]
    fn file_diff_does_not_spawn_repo_fsmonitor_probe() {
        use std::os::unix::fs::PermissionsExt;

        let base = Path::new("/tmp/aria-bug45/t4");
        std::fs::create_dir_all(base).expect("create /tmp/aria-bug45/t4 fixture base");
        let temp = tempfile::tempdir_in(base).expect("fixture dir under /tmp/aria-bug45/t4");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let raw = |argv: &[&str]| {
            let output = std::process::Command::new("git")
                .args(argv)
                .current_dir(&repo)
                .output()
                .expect("raw fixture git");
            assert!(output.status.success());
        };
        raw(&["init", "--quiet", "-b", "main"]);
        raw(&["config", "user.email", "aria-t4@example.invalid"]);
        raw(&["config", "user.name", "Aria T4"]);
        std::fs::write(repo.join("README.md"), "x\n").unwrap();
        raw(&["add", "."]);
        raw(&["commit", "--quiet", "-m", "init"]);
        let head = {
            let output = std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&repo)
                .output()
                .expect("read head");
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        };

        let fsmonitor_probe = repo.join(".git/aria-fsmonitor-fired");
        let fsmonitor_hook = repo.join(".git/aria-fsmonitor-hook.sh");
        std::fs::write(
            &fsmonitor_hook,
            format!("#!/bin/sh\n: > {}\nexit 1\n", fsmonitor_probe.display()),
        )
        .unwrap();
        std::fs::set_permissions(&fsmonitor_hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        raw(&[
            "config",
            "core.fsmonitor",
            fsmonitor_hook.to_str().expect("utf8 hook path"),
        ]);

        let runtime = WebRuntime::new_fake(repo.clone());
        runtime.file_diff(&head, "README.md").expect("bridged diff");
        assert!(
            !fsmonitor_probe.exists(),
            "Bug45: file_diff 不得执行仓库 core.fsmonitor 探针"
        );

        raw(&["status", "--porcelain"]);
        assert!(
            fsmonitor_probe.exists(),
            "对照失败:coder 域 status 应触发其自身 fsmonitor 探针,夹具失效"
        );
    }

    /// Bug45/F4:base_checkpoint 是 GET query 直通值——任何选项形态(含
    /// `--output=`/`--ext-diff`)必须在执行 git 之前按 400 拒绝,不得产生
    /// 任何文件副作用。
    #[test]
    fn file_diff_rejects_option_shaped_base_checkpoint_before_git_runs() {
        let (_temp, repo) = diff_fixture_repo();
        let runtime = WebRuntime::new_fake(repo.clone());

        let probe = repo.join("aria-pwned.txt");
        let injected = format!("--output={}", probe.display());
        let error = runtime
            .file_diff(&injected, "README.md")
            .expect_err("--output= 形态必须拒绝");
        assert_eq!(error.code, "invalid_base_checkpoint");
        assert!(!probe.exists(), "Bug45:拒绝的 base_checkpoint 不得产生文件");

        for injected in [
            "--ext-diff",
            "--no-index",
            "-O/tmp/aria-pwned",
            "HEAD;reboot",
        ] {
            let error = runtime
                .file_diff(injected, "README.md")
                .expect_err("选项/链式形态必须拒绝");
            assert_eq!(
                error.code, "invalid_base_checkpoint",
                "注入形态 {injected} 必须按 400 拒绝"
            );
        }
    }

    /// Bug45/F4:合法基准形态(40 hex、分支名、HEAD~N)确实产生该基线到
    /// worktree 的 diff——白名单不得把正常调用一起挡掉。
    #[test]
    fn file_diff_accepts_well_formed_baselines() {
        let (_temp, repo) = diff_fixture_repo();
        let head = raw_git(&repo, &["rev-parse", "HEAD"]);
        let ancestor = raw_git(&repo, &["rev-parse", "HEAD~1"]);
        std::fs::write(repo.join("README.md"), "worktree-change\n").unwrap();
        let runtime = WebRuntime::new_fake(repo.clone());

        for base in ["HEAD", "main", "HEAD~1", head.as_str(), ancestor.as_str()] {
            let response = runtime
                .file_diff(base, "README.md")
                .unwrap_or_else(|error| panic!("合法基准 {base} 必须可用: {error:?}"));
            assert!(
                response.diff.contains("README.md") && !response.diff.trim().is_empty(),
                "合法基准 {base} 必须返回真实 diff: {:?}",
                response.diff
            );
        }
    }

    /// Bug45/F20:形态合法但对象不存在的基准必须走真实 git 错误,不得返回
    /// 空 diff 冒充“无差异”。
    #[test]
    fn file_diff_surfaces_git_error_for_unresolvable_checkpoint() {
        let (_temp, repo) = diff_fixture_repo();
        let runtime = WebRuntime::new_fake(repo.clone());

        let error = runtime
            .file_diff("ckpt_0001", "README.md")
            .expect_err("无法解析的基准必须报错");
        assert_eq!(error.code, "git_rev_parse_failed");
        assert!(
            error.message.contains("rev-parse"),
            "错误必须保留 git 失败语义: {}",
            error.message
        );
    }

    /// Bug45/F4:diff argv 固定 `--no-ext-diff --no-textconv`,coder 可写的
    /// 仓库 config(textconv)/`.gitattributes`(diff driver)不得驱动平台进程
    /// 执行仓库注入脚本。
    #[test]
    fn file_diff_disables_repository_textconv_and_external_diff() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, repo) = diff_fixture_repo();
        std::fs::write(repo.join("README.md"), "worktree-change\n").unwrap();

        let textconv_marker = repo.join(".git/aria-textconv-fired");
        let textconv_script = repo.join(".git/aria-textconv.sh");
        std::fs::write(
            &textconv_script,
            format!("#!/bin/sh\n: > {}\ncat \"$1\"\n", textconv_marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&textconv_script, std::fs::Permissions::from_mode(0o755)).unwrap();
        raw_git(
            &repo,
            &[
                "config",
                "diff.ariaprobe.textconv",
                textconv_script.to_str().expect("utf8 textconv path"),
            ],
        );
        std::fs::write(repo.join(".gitattributes"), "README.md diff=ariaprobe\n").unwrap();

        let external_marker = repo.join(".git/aria-external-diff-fired");
        let external_script = repo.join(".git/aria-external-diff.sh");
        std::fs::write(
            &external_script,
            format!("#!/bin/sh\n: > {}\nexit 1\n", external_marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&external_script, std::fs::Permissions::from_mode(0o755)).unwrap();
        raw_git(
            &repo,
            &[
                "config",
                "diff.external",
                external_script.to_str().expect("utf8 external diff path"),
            ],
        );

        let runtime = WebRuntime::new_fake(repo.clone());
        let response = runtime
            .file_diff("HEAD", "README.md")
            .expect("受信 diff 必须成功");
        assert!(!response.diff.trim().is_empty(), "diff 不得为空");
        assert!(
            !textconv_marker.exists(),
            "Bug45: file_diff 不得执行仓库 diff.<driver>.textconv"
        );
        assert!(
            !external_marker.exists(),
            "Bug45: file_diff 不得执行仓库 diff.external"
        );
    }

    fn raw_git(repo: &Path, argv: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(argv)
            .current_dir(repo)
            .output()
            .expect("fixture git");
        assert!(
            output.status.success(),
            "fixture git {argv:?} 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn diff_fixture_repo() -> (tempfile::TempDir, std::path::PathBuf) {
        let base = Path::new("/tmp/aria-bug45/t4");
        std::fs::create_dir_all(base).expect("create /tmp/aria-bug45/t4 fixture base");
        let temp = tempfile::tempdir_in(base).expect("fixture dir under /tmp/aria-bug45/t4");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        raw_git(&repo, &["init", "--quiet", "-b", "main"]);
        raw_git(&repo, &["config", "user.email", "aria-t4@example.invalid"]);
        raw_git(&repo, &["config", "user.name", "Aria T4"]);
        std::fs::write(repo.join("README.md"), "x\n").unwrap();
        raw_git(&repo, &["add", "."]);
        raw_git(&repo, &["commit", "--quiet", "-m", "init"]);
        std::fs::write(repo.join("README.md"), "y\n").unwrap();
        raw_git(&repo, &["commit", "--quiet", "-am", "second"]);
        (temp, repo)
    }
}
