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
        // Bug45/T4 修复轮:file_diff 对 coder/reviewer 可写的 workspace 仓
        // 执行 diff,属平台受信启动面;子命令之前注入禁桥参数(与
        // cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS 同源),diff
        // 不得询问仓库 core.fsmonitor 探针。
        let diff = Command::new("git")
            .args(crate::cross_cutting::git_command::TRUSTED_GIT_BRIDGE_ARGS)
            .args(["diff", base_checkpoint, "--", path])
            .current_dir(&self.workspace_root)
            .output()
            .map_err(|error| TaskRunError::new("git_command_failed", error.to_string()))?;
        Ok(FileDiffResponse {
            base_checkpoint: base_checkpoint.to_string(),
            path: path.to_string(),
            diff: String::from_utf8_lossy(&diff.stdout).to_string(),
        })
    }
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
}
